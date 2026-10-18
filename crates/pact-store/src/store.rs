//! Store 门面：真相源（JSONL）+ 快照 + 派生索引的统一生命周期。
//!
//! 写入路径（引擎临界区内，设计 §7.3）：追加 JSONL（fsync）为**提交点**；
//! 索引更新为尽力而为的派生投影（失败不回滚、不失败本次提交，由启动一致性校验重建自愈）。
//! 启动路径：`recover()` 清洗日志（截断崩溃半行）→ 读取全部事件 →
//! 索引与日志末尾 seq 一致性校验（不一致则全量重建）。
//! 语义重放（解析声明、恢复 CounterState）由引擎层驱动（store 不依赖 parser）。

use crate::error::{validate_domain_name, StoreError};
use crate::event::Event;
use crate::index::{IndexStore, TokenStatus, TursoIndexStore};
use crate::log::{self, EventLog, ReadOutcome};
use crate::snapshot::{self, Snapshot};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// 单域恢复产物：清洗后的事件流（语义重放由引擎完成）。
pub struct RecoveredDomain {
    pub name: String,
    pub events: Vec<Event>,
    /// 启动时是否截断了崩溃残留的尾部半行。
    pub truncated: bool,
}

pub struct Store {
    data_dir: PathBuf,
    index: Arc<dyn IndexStore>,
    /// 每域追加句柄 + 已写末尾 seq（域内逐行 +1 不变量的写侧维护）。
    /// Mutex 仅保护句柄表获取；写入由引擎 actor 串行调用（D6 单权威）。
    logs: Mutex<BTreeMap<String, (EventLog, u64)>>,
}

impl Store {
    /// 打开/创建存储目录与共享索引。
    pub async fn open(data_dir: impl AsRef<Path>) -> Result<Self, StoreError> {
        let data_dir = data_dir.as_ref().to_owned();
        std::fs::create_dir_all(&data_dir)?;
        let index = TursoIndexStore::open(&data_dir.join("index.tursodb")).await?;
        Ok(Self {
            data_dir,
            index: Arc::new(index),
            logs: Mutex::new(BTreeMap::new()),
        })
    }

    /// 启动恢复：清洗日志、读取事件、校验/重建索引。
    /// 返回每个域的事件流（含是否截断了崩溃残留）。
    pub async fn recover(&self) -> Result<Vec<RecoveredDomain>, StoreError> {
        // 1. 扫描 *.events.jsonl → 域名
        let mut names: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(&self.data_dir)? {
            let entry = entry?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if let Some(stem) = name.strip_suffix(".events.jsonl") {
                if validate_domain_name(stem).is_ok() {
                    names.push(stem.to_owned());
                }
            }
        }
        names.sort();

        // 2. 逐域清洗 + 读取
        let mut recovered = Vec::new();
        for name in &names {
            let path = log::events_path(&self.data_dir, name);
            let ReadOutcome { events, truncated } = log::read_and_sanitize(&path)?;
            let last_seq = events.last().map(|e| e.seq()).unwrap_or(0);
            self.logs
                .lock()
                .unwrap()
                .insert(name.clone(), (EventLog::open(&path)?, last_seq));
            recovered.push(RecoveredDomain {
                name: name.clone(),
                events,
                truncated,
            });
        }

        // 3. 索引一致性：任一域末尾 seq 与索引不符 → 全量重建（设计 §7.3）
        let mut consistent = true;
        for rd in &recovered {
            let log_last = rd.events.last().map(|e| e.seq());
            let idx_last = self.index.last_event_seq(&rd.name).await?;
            if log_last != idx_last {
                consistent = false;
                break;
            }
        }
        if !consistent {
            let entries: Vec<(String, Vec<Event>)> = recovered
                .iter()
                .map(|rd| (rd.name.clone(), rd.events.clone()))
                .collect();
            self.index.rebuild(&entries).await?;
        }

        Ok(recovered)
    }

    /// 追加事件：seq 校验 → JSONL fsync（**提交点**）→ 派生索引投影（尽力而为）。
    ///
    /// 索引是纯派生物（设计 §7.3/§7.4）：日志落盘即视为已提交；索引投影失败既不失败本次
    /// 调用、也不回滚，由下次启动的一致性校验触发全量重建（自愈）。由此排除"日志已提交
    /// 但调用方收到错误、内存未 apply"的日志/内存分歧。
    pub async fn append(&self, domain: &str, ev: &Event) -> Result<(), StoreError> {
        validate_domain_name(domain)?;
        self.log_append(domain, ev).await?;
        let _ = self.index_project(domain, ev).await;
        Ok(())
    }

    /// 派生索引投影（events 行 + tokens 终态）。返回 Err 仅供诊断，调用方忽略。
    async fn index_project(&self, domain: &str, ev: &Event) -> Result<(), StoreError> {
        self.index.upsert_event(domain, ev).await?;
        match ev {
            Event::Seeded { token, .. } | Event::Produced { token, .. } => {
                self.index
                    .upsert_token(domain, token, &TokenStatus::Counter)
                    .await?;
            }
            Event::Consumed { token, by, ts, .. } => {
                self.index
                    .upsert_token(
                        domain,
                        token,
                        &TokenStatus::Archived {
                            exhausted_by: by.clone(),
                            exhausted_at: ts.clone(),
                        },
                    )
                    .await?;
            }
            _ => {}
        }
        Ok(())
    }

    /// 追加 JSONL（含 seq 逐行 +1 校验与 fsync）。
    async fn log_append(&self, domain: &str, ev: &Event) -> Result<(), StoreError> {
        let path = log::events_path(&self.data_dir, domain);
        let expected_next = {
            let mut logs = self.logs.lock().unwrap();
            let (_, last_seq) = logs
                .entry(domain.to_owned())
                .or_insert_with(|| (EventLog::open(&path).expect("日志文件已存在"), 0));
            *last_seq + 1
        };
        if ev.seq() != expected_next {
            return Err(StoreError::SeqNotMonotonic {
                path,
                expected: expected_next,
                got: ev.seq(),
            });
        }
        let mut logs = self.logs.lock().unwrap();
        let (log, last_seq) = logs.get_mut(domain).expect("句柄已建立");
        log.append(ev)?;
        *last_seq = ev.seq();
        Ok(())
    }

    /// 写快照（原子）。
    pub fn write_snapshot(&self, domain: &str, snap: &Snapshot) -> Result<(), StoreError> {
        validate_domain_name(domain)?;
        snapshot::write_snapshot(&log::snapshot_path(&self.data_dir, domain), snap)
    }

    /// 读快照（无/损坏 → None，重放兜底）。
    pub fn load_snapshot(&self, domain: &str) -> Result<Option<Snapshot>, StoreError> {
        validate_domain_name(domain)?;
        snapshot::load_snapshot(&log::snapshot_path(&self.data_dir, domain))
    }

    /// 删除快照（域替换时清理陈旧文件）。
    pub fn remove_snapshot(&self, domain: &str) -> Result<(), StoreError> {
        validate_domain_name(domain)?;
        snapshot::remove_snapshot(&log::snapshot_path(&self.data_dir, domain))
    }

    /// 已存在的域清单（按文件扫描，含尚未被引擎加载的）。
    pub fn list_domains(&self) -> Vec<String> {
        let mut names = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.data_dir) {
            for entry in rd.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if let Some(stem) = name.strip_suffix(".events.jsonl") {
                    if validate_domain_name(stem).is_ok() {
                        names.push(stem.to_owned());
                    }
                }
            }
        }
        names.sort();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pact_core::{ParticipantName, ThingRef, Token, TokenId, TypeName};

    fn token(id: u64, ty: &str) -> Token {
        Token {
            id: TokenId(format!("t{id}")),
            ty: TypeName(ty.into()),
            thing_ref: ThingRef::none(),
            produced_by: ParticipantName("p".into()),
            depends_on: vec![],
            produced_at: "T0".into(),
        }
    }

    fn domain_loaded(seq: u64) -> Event {
        Event::DomainLoaded {
            seq,
            ts: "T0".into(),
            name: "d".into(),
            declaration: "…".into(),
        }
    }

    #[tokio::test]
    async fn append_then_recover_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        store.append("d", &domain_loaded(1)).await.unwrap();
        store
            .append(
                "d",
                &Event::Seeded {
                    seq: 2,
                    ts: "T1".into(),
                    token: token(1, "req"),
                },
            )
            .await
            .unwrap();
        store
            .append(
                "d",
                &Event::Consumed {
                    seq: 3,
                    ts: "T2".into(),
                    token: token(1, "req"),
                    by: ParticipantName("arch".into()),
                    request_id: None,
                },
            )
            .await
            .unwrap();

        // 重新打开（模拟重启）→ recover 读取
        let store2 = Store::open(dir.path()).await.unwrap();
        let recovered = store2.recover().await.unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].name, "d");
        assert_eq!(recovered[0].events.len(), 3);
        assert!(!recovered[0].truncated);
        assert_eq!(store2.list_domains(), vec!["d"]);
    }

    #[tokio::test]
    async fn seq_enforced_on_append() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        store.append("d", &domain_loaded(1)).await.unwrap();
        // 跳号：seq=3
        let err = store
            .append(
                "d",
                &Event::Seeded {
                    seq: 3,
                    ts: "T1".into(),
                    token: token(1, "req"),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            StoreError::SeqNotMonotonic {
                expected: 2,
                got: 3,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn invalid_domain_name_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        for bad in ["../evil", "a/b", "", "a b"] {
            assert!(store.append(bad, &domain_loaded(1)).await.is_err());
        }
    }

    #[tokio::test]
    async fn index_mismatch_triggers_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = Store::open(dir.path()).await.unwrap();
            store.append("d", &domain_loaded(1)).await.unwrap();
            store
                .append(
                    "d",
                    &Event::Seeded {
                        seq: 2,
                        ts: "T1".into(),
                        token: token(1, "req"),
                    },
                )
                .await
                .unwrap();
        }
        // 模拟索引滞后/损坏：删除索引文件
        std::fs::remove_file(dir.path().join("index.tursodb")).unwrap();

        // 重新打开：索引为空（seq 不一致）→ recover 触发全量重建
        let store = Store::open(dir.path()).await.unwrap();
        let recovered = store.recover().await.unwrap();
        assert_eq!(recovered.len(), 1);
        // 重建后索引末尾 seq 与日志一致（经 last_event_seq 查询验证）
        let idx = TursoIndexStore::open(&dir.path().join("index.tursodb"))
            .await
            .unwrap();
        assert_eq!(idx.last_event_seq("d").await.unwrap(), Some(2));
    }

    #[tokio::test]
    async fn snapshot_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        store.append("d", &domain_loaded(1)).await.unwrap();
        let mut state = pact_core::CounterState::new();
        state.apply(pact_core::Effect::Seeded(token(9, "req")));
        store
            .write_snapshot("d", &Snapshot { seq: 1, state })
            .unwrap();
        assert!(store.load_snapshot("d").unwrap().is_some());

        let store2 = Store::open(dir.path()).await.unwrap();
        let snap = store2.load_snapshot("d").unwrap().unwrap();
        assert_eq!(snap.seq, 1);
        assert_eq!(snap.state.counter.len(), 1);
    }
}

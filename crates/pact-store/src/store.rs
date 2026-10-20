//! Store 门面：真相源（JSONL）+ 快照 + 派生索引的统一生命周期。
//!
//! 双键布局（D27 路径 β）：`<data_dir>/<domain>/<instance>.events.jsonl`。
//! 写入路径（引擎临界区内，设计 §7.3）：追加 JSONL（fsync）为**提交点**；
//! 索引更新为尽力而为的派生投影（失败不回滚、不失败本次提交，由启动一致性校验重建自愈）。
//! 启动路径：旧单层布局探测（fail-fast）→ 两层扫描实例日志 → 清洗（截断崩溃半行）→
//! 索引与日志末尾 seq 一致性校验（不一致则全量重建）。
//! 语义重放（解析声明、恢复 CounterState）由引擎层驱动（store 不依赖 parser）。

use crate::error::{validate_domain_name, validate_instance_name, StoreError};
use crate::event::Event;
use crate::index::{IndexStore, TokenStatus, TursoIndexStore};
use crate::key::InstanceKey;
use crate::log::{self, EventLog, ReadOutcome};
use crate::snapshot::{self, Snapshot};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// 单实例恢复产物：清洗后的事件流（语义重放由引擎完成）。
#[derive(Debug)]
pub struct RecoveredInstance {
    pub key: InstanceKey,
    pub events: Vec<Event>,
    /// 启动时是否截断了崩溃残留的尾部半行。
    pub truncated: bool,
}

pub struct Store {
    data_dir: PathBuf,
    index: Arc<dyn IndexStore>,
    /// 每域实例追加句柄 + 已写末尾 seq（实例内逐行 +1 不变量的写侧维护）。
    /// Mutex 仅保护句柄表获取；写入由引擎 actor 串行调用（D6 单权威）。
    logs: Mutex<BTreeMap<InstanceKey, (EventLog, u64)>>,
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

    /// 启动恢复：旧布局探测 + 两层扫描实例日志、清洗事件、校验/重建索引。
    /// 返回每个实例的事件流（含是否截断了崩溃残留）。
    pub async fn recover(&self) -> Result<Vec<RecoveredInstance>, StoreError> {
        // 0. 旧单层布局探测（fail-fast）：顶层存在 *.events.jsonl / *.snapshot.json
        for entry in std::fs::read_dir(&self.data_dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".events.jsonl") || name.ends_with(".snapshot.json") {
                return Err(StoreError::LegacyLayoutDetected { path: entry.path() });
            }
        }

        // 1. 两层扫描 → 实例键（域子目录 + 实例日志文件）
        let keys = self.scan_instance_keys();

        // 2. 逐域实例清洗 + 读取
        let mut recovered = Vec::new();
        for key in keys {
            let path = log::events_path(&self.data_dir, &key.domain, &key.instance);
            let ReadOutcome { events, truncated } = log::read_and_sanitize(&path)?;
            let last_seq = events.last().map(|e| e.seq()).unwrap_or(0);
            self.logs
                .lock()
                .unwrap()
                .insert(key.clone(), (EventLog::open(&path)?, last_seq));
            recovered.push(RecoveredInstance {
                key,
                events,
                truncated,
            });
        }

        // 3. 索引一致性：任一实例末尾 seq 与索引不符 → 全量重建（设计 §7.3）
        let mut consistent = true;
        for rd in &recovered {
            let log_last = rd.events.last().map(|e| e.seq());
            let idx_last = self.index.last_event_seq(&rd.key).await?;
            if log_last != idx_last {
                consistent = false;
                break;
            }
        }
        if !consistent {
            let entries: Vec<(InstanceKey, Vec<Event>)> = recovered
                .iter()
                .map(|rd| (rd.key.clone(), rd.events.clone()))
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
    pub async fn append(&self, key: &InstanceKey, ev: &Event) -> Result<(), StoreError> {
        key.validate()?;
        self.log_append(key, ev).await?;
        let _ = self.index_project(key, ev).await;
        Ok(())
    }

    /// 派生索引投影（events 行 + tokens 终态）。返回 Err 仅供诊断，调用方忽略。
    async fn index_project(&self, key: &InstanceKey, ev: &Event) -> Result<(), StoreError> {
        self.index.upsert_event(key, ev).await?;
        match ev {
            Event::Seeded { token, .. } | Event::Produced { token, .. } => {
                self.index
                    .upsert_token(key, token, &TokenStatus::Counter)
                    .await?;
            }
            Event::Consumed { token, by, ts, .. } => {
                self.index
                    .upsert_token(
                        key,
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

    /// 追加 JSONL（含 seq 逐行 +1 校验与 fsync）。首次写入时建域子目录。
    async fn log_append(&self, key: &InstanceKey, ev: &Event) -> Result<(), StoreError> {
        let path = log::events_path(&self.data_dir, &key.domain, &key.instance);
        let expected_next = {
            let mut logs = self.logs.lock().unwrap();
            if !logs.contains_key(key) {
                std::fs::create_dir_all(log::domain_dir(&self.data_dir, &key.domain))?;
                logs.insert(key.clone(), (EventLog::open(&path)?, 0));
            }
            logs.get(key).expect("句柄已建立").1 + 1
        };
        if ev.seq() != expected_next {
            return Err(StoreError::SeqNotMonotonic {
                path,
                expected: expected_next,
                got: ev.seq(),
            });
        }
        let mut logs = self.logs.lock().unwrap();
        let (log, last_seq) = logs.get_mut(key).expect("句柄已建立");
        log.append(ev)?;
        *last_seq = ev.seq();
        Ok(())
    }

    /// 写快照（原子）。
    pub fn write_snapshot(&self, key: &InstanceKey, snap: &Snapshot) -> Result<(), StoreError> {
        key.validate()?;
        snapshot::write_snapshot(
            &log::snapshot_path(&self.data_dir, &key.domain, &key.instance),
            snap,
        )
    }

    /// 读快照（无/损坏 → None，重放兜底）。
    pub fn load_snapshot(&self, key: &InstanceKey) -> Result<Option<Snapshot>, StoreError> {
        key.validate()?;
        snapshot::load_snapshot(&log::snapshot_path(
            &self.data_dir,
            &key.domain,
            &key.instance,
        ))
    }

    /// 删除快照（实例替换时清理陈旧文件）。
    pub fn remove_snapshot(&self, key: &InstanceKey) -> Result<(), StoreError> {
        key.validate()?;
        snapshot::remove_snapshot(&log::snapshot_path(
            &self.data_dir,
            &key.domain,
            &key.instance,
        ))
    }

    /// 写声明原文（`<domain>/declaration.pact`，定义态缓存；原子写）。
    pub fn write_declaration(&self, domain: &str, text: &str) -> Result<(), StoreError> {
        validate_domain_name(domain)?;
        let dir = log::domain_dir(&self.data_dir, domain);
        std::fs::create_dir_all(&dir)?;
        let path = log::declaration_path(&self.data_dir, domain);
        let tmp = path.with_extension("pact.tmp");
        std::fs::write(&tmp, text.as_bytes())?;
        if let Ok(f) = std::fs::File::open(&tmp) {
            let _ = f.sync_all();
        }
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    /// 读声明原文（无 → None）。
    pub fn load_declaration(&self, domain: &str) -> Result<Option<String>, StoreError> {
        validate_domain_name(domain)?;
        match std::fs::read_to_string(log::declaration_path(&self.data_dir, domain)) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StoreError::Io {
                context: "读取声明原文",
                source: e,
            }),
        }
    }

    /// 已存在的实例键清单（按文件扫描，含尚未被引擎加载的）。
    pub fn list_instances(&self) -> Vec<InstanceKey> {
        self.scan_instance_keys()
    }

    /// 存在实例的域名清单（去重；由 `list_instances` 派生）。
    pub fn list_domains(&self) -> Vec<String> {
        let mut names: Vec<String> = self
            .scan_instance_keys()
            .into_iter()
            .map(|k| k.domain)
            .collect();
        names.dedup();
        names
    }

    /// 实例日志已见末尾 seq（未跟踪 → 0）。供引擎计算实例续接点
    /// （兼容「卸载后再实例化」的 seq 连续）。
    pub fn last_seq(&self, key: &InstanceKey) -> u64 {
        self.logs
            .lock()
            .unwrap()
            .get(key)
            .map(|(_, s)| *s)
            .unwrap_or(0)
    }

    /// 两层扫描：域子目录 → 实例日志文件 → `InstanceKey`（按 `(domain, instance)` 排序）。
    fn scan_instance_keys(&self) -> Vec<InstanceKey> {
        let mut keys = Vec::new();
        if let Ok(domains) = std::fs::read_dir(&self.data_dir) {
            for dent in domains.flatten() {
                if !dent.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    continue;
                }
                let domain = dent.file_name();
                let domain = domain.to_string_lossy().to_string();
                if validate_domain_name(&domain).is_err() {
                    continue;
                }
                if let Ok(instances) = std::fs::read_dir(dent.path()) {
                    for ient in instances.flatten() {
                        let name = ient.file_name();
                        let name = name.to_string_lossy();
                        if let Some(stem) = name.strip_suffix(".events.jsonl") {
                            if validate_instance_name(stem).is_ok() {
                                keys.push(InstanceKey::new(domain.clone(), stem));
                            }
                        }
                    }
                }
            }
        }
        keys.sort();
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pact_core::{ParticipantName, ThingRef, Token, TokenId, TypeName};

    fn key() -> InstanceKey {
        InstanceKey::new("d", "default")
    }

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
        store.append(&key(), &domain_loaded(1)).await.unwrap();
        store
            .append(
                &key(),
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
                &key(),
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
        assert_eq!(recovered[0].key, key());
        assert_eq!(recovered[0].events.len(), 3);
        assert!(!recovered[0].truncated);
        assert_eq!(store2.list_instances(), vec![key()]);
        assert_eq!(store2.list_domains(), vec!["d"]);
    }

    #[tokio::test]
    async fn seq_enforced_on_append() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        store.append(&key(), &domain_loaded(1)).await.unwrap();
        // 跳号：期望 2，给 3
        let err = store
            .append(
                &key(),
                &Event::Seeded {
                    seq: 3,
                    ts: "T1".into(),
                    token: token(1, "req"),
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::SeqNotMonotonic { .. }));
    }

    #[tokio::test]
    async fn legacy_layout_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("proj.events.jsonl"), b"").unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        let err = store.recover().await.unwrap_err();
        assert!(matches!(err, StoreError::LegacyLayoutDetected { .. }));
    }

    #[tokio::test]
    async fn instances_are_isolated_by_key() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        let a = InstanceKey::new("proj", "a");
        let b = InstanceKey::new("proj", "b");
        store.append(&a, &domain_loaded(1)).await.unwrap();
        store
            .append(
                &a,
                &Event::Seeded {
                    seq: 2,
                    ts: "T1".into(),
                    token: token(1, "req"),
                },
            )
            .await
            .unwrap();
        // 另一实例独立 seq 空间
        store.append(&b, &domain_loaded(1)).await.unwrap();
        let store2 = Store::open(dir.path()).await.unwrap();
        let recovered = store2.recover().await.unwrap();
        assert_eq!(recovered.len(), 2);
        let a_rec = recovered.iter().find(|r| r.key == a).unwrap();
        let b_rec = recovered.iter().find(|r| r.key == b).unwrap();
        assert_eq!(a_rec.events.len(), 2);
        assert_eq!(b_rec.events.len(), 1);
    }

    #[tokio::test]
    async fn invalid_domain_name_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        for bad in ["../evil", "a/b", "", "a b"] {
            let key = InstanceKey::new(bad, "default");
            assert!(matches!(
                store.append(&key, &domain_loaded(1)).await.unwrap_err(),
                StoreError::InvalidDomainName { .. }
            ));
        }
    }

    #[tokio::test]
    async fn invalid_instance_name_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        for bad in ["../evil", "a/b", "", "a b"] {
            let key = InstanceKey::new("d", bad);
            assert!(matches!(
                store.append(&key, &domain_loaded(1)).await.unwrap_err(),
                StoreError::InvalidInstanceName { .. }
            ));
        }
    }

    #[tokio::test]
    async fn index_mismatch_triggers_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = Store::open(dir.path()).await.unwrap();
            store.append(&key(), &domain_loaded(1)).await.unwrap();
            store
                .append(
                    &key(),
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
        assert_eq!(idx.last_event_seq(&key()).await.unwrap(), Some(2));
    }

    #[tokio::test]
    async fn snapshot_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        store.append(&key(), &domain_loaded(1)).await.unwrap();
        let mut state = pact_core::CounterState::new();
        state.apply(pact_core::Effect::Seeded(token(9, "req")));
        store
            .write_snapshot(&key(), &Snapshot { seq: 1, state })
            .unwrap();
        assert!(store.load_snapshot(&key()).unwrap().is_some());

        let store2 = Store::open(dir.path()).await.unwrap();
        let snap = store2.load_snapshot(&key()).unwrap().unwrap();
        assert_eq!(snap.seq, 1);
        assert_eq!(snap.state.counter.len(), 1);
    }

    #[tokio::test]
    async fn declaration_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        assert!(store.load_declaration("d").unwrap().is_none());
        store.write_declaration("d", "domain d\n").unwrap();
        assert_eq!(
            store.load_declaration("d").unwrap().as_deref(),
            Some("domain d\n")
        );
    }
}

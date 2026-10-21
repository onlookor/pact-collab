//! Store 门面：真相源（JSONL）+ 快照 + 派生索引的统一生命周期。
//!
//! 双键布局（D27 路径 β）：`<data_dir>/<domain>/<instance>.events.jsonl`。
//! 写入路径（引擎临界区内，设计 §7.3）：追加 JSONL（fsync）为**提交点**；
//! 索引更新为尽力而为的派生投影（失败不回滚、不失败本次提交，由启动一致性校验重建自愈）。
//! 启动路径：旧单层布局探测（fail-fast）→ 两层扫描实例日志 → 清洗（截断崩溃半行）→
//! 索引与日志末尾 seq 一致性校验（不一致则全量重建）。
//! 语义重放（解析声明、恢复 CounterState）由引擎层驱动（store 不依赖 parser）。

use crate::audit::ReadAuditRecord;
use crate::error::{validate_domain_name, validate_instance_name, StoreError};
use crate::index::{IndexStore, TokenStatus, TursoIndexStore};
use crate::log::{self, EventLog, ReadOutcome};
use crate::snapshot;
use pact_store_model::{Event, InstanceKey, Snapshot};
use std::collections::BTreeMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// 单实例恢复产物：清洗后的事件流（语义重放由引擎完成）。
#[derive(Debug)]
pub struct RecoveredInstance {
    pub key: InstanceKey,
    pub events: Vec<Event>,
    /// 启动时是否截断了崩溃残留的尾部半行。
    pub truncated: bool,
    /// 日志前缀压缩水位（已被快照覆盖、可从日志丢弃的事件上界；0 = 未压缩）。
    pub base_seq: u64,
}

/// 单实例前缀压缩结果（设计 §7.5）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionResult {
    /// 快照已覆盖到的最后事件 seq（压缩前日志末尾 seq）。
    pub snapshot_seq: u64,
    /// 压缩后的日志保留水位（可从日志丢弃的事件上界）。
    pub base_seq: u64,
    /// 日志中保留的事件行数。
    pub retained: u64,
    /// 本次丢弃的事件行数。
    pub discarded: u64,
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

        // 2. 逐域实例清洗 + 读取（含压缩水位与快照覆盖校验）
        let mut recovered = Vec::new();
        for key in keys {
            let path = log::events_path(&self.data_dir, &key.domain, &key.instance);
            let meta_path = log::logmeta_path(&self.data_dir, &key.domain, &key.instance);
            let base_seq = log::load_logmeta(&meta_path)?
                .map(|m| m.base_seq)
                .unwrap_or(0);
            let ReadOutcome { events, truncated } = log::read_and_sanitize(&path, base_seq)?;
            let snap = snapshot::load_snapshot(&log::snapshot_path(
                &self.data_dir,
                &key.domain,
                &key.instance,
            ))?;
            let snap_seq = snap.as_ref().map(|s| s.seq).unwrap_or(0);

            // 压缩覆盖校验：日志中已无 `DomainLoaded`（前缀被压缩丢弃）时，
            // 被丢弃前缀的状态只能由快照承载；快照缺失 → 历史不可恢复（fail-fast，
            // 不静默丢数据）。允许"日志尚未截断"的崩溃中间态（超集，无害）。
            let has_domain_loaded = events
                .iter()
                .any(|e| matches!(e, Event::DomainLoaded { .. }));
            if !has_domain_loaded {
                let need = events
                    .first()
                    .map(|e| e.seq().saturating_sub(1))
                    .unwrap_or(0)
                    .max(base_seq);
                if snap_seq < need {
                    return Err(StoreError::SnapshotRequiredForCompactedLog {
                        path,
                        base_seq: need,
                    });
                }
            }

            // 写侧续接点：取「日志末尾 seq / 压缩水位 / 快照 seq」三者最大
            // （压缩后日志可能为空，续接点必须回落到快照/水位，否则会重号覆盖）。
            let last_seq = events
                .last()
                .map(|e| e.seq())
                .unwrap_or(0)
                .max(base_seq)
                .max(snap_seq);

            // 索引与压缩水位对齐（幂等自愈「日志已截断但索引未裁剪」的崩溃窗口）
            if base_seq > 0 {
                let _ = self.index.prune_events(&key, base_seq).await;
            }

            self.logs
                .lock()
                .unwrap()
                .insert(key.clone(), (EventLog::open(&path)?, last_seq));
            recovered.push(RecoveredInstance {
                key,
                events,
                truncated,
                base_seq,
            });
        }

        // 3. 索引一致性：任一实例末尾 seq 与索引不符 → 全量重建（设计 §7.3）。
        //    压缩后索引的合法上界可能回落到水位（`base_seq`），此时接受 `idx_last <= base_seq`。
        let mut consistent = true;
        for rd in &recovered {
            let log_last = rd.events.last().map(|e| e.seq()).unwrap_or(0);
            let idx_last = self.index.last_event_seq(&rd.key).await?;
            let ok = match idx_last {
                Some(s) => s == log_last || (log_last == 0 && s <= rd.base_seq),
                None => log_last == 0,
            };
            if !ok {
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

    /// 单实例日志前缀压缩（设计 §7.5）：写快照 → 记水位 → 原子重写日志 → 裁剪索引。
    ///
    /// 语义：`snap` 必须覆盖实例当前全部状态（引擎在临界区内构造，seq = 日志末尾 seq）。
    /// 日志只保留最近 `keep_last` 条（`keep_last = 0` 即清空已覆盖前缀）。
    /// 崩溃安全：快照先行（提交点），水位清单先于日志重写落盘；任一时刻中断，
    /// 恢复看到的都是「未截断（超集）」或「已截断 + 水位 + 快照」之一。
    pub async fn compact(
        &self,
        key: &InstanceKey,
        snap: &Snapshot,
        keep_last: u64,
        compacted_at: &str,
    ) -> Result<CompactionResult, StoreError> {
        key.validate()?;
        let path = log::events_path(&self.data_dir, &key.domain, &key.instance);
        let meta_path = log::logmeta_path(&self.data_dir, &key.domain, &key.instance);

        // 1. 快照（提交点）：压缩后仅此文件承载被丢弃前缀的状态与幂等表
        self.write_snapshot(key, snap)?;

        // 2. 水位清单（先于日志重写）：记录可丢弃上界（水位单调不回退）
        let prev_base = log::load_logmeta(&meta_path)?
            .map(|m| m.base_seq)
            .unwrap_or(0);
        let base_seq = snap.seq.saturating_sub(keep_last).max(prev_base);
        log::write_logmeta(
            &meta_path,
            &log::LogMeta {
                base_seq,
                compacted_at: Some(compacted_at.to_owned()),
                ..Default::default()
            },
        )?;

        // 3. 原子重写日志：仅保留 seq > base_seq 的行（失败时旧句柄仍有效）
        let (retained, discarded) = log::rewrite_retaining(&path, base_seq)?;

        // 4. 索引裁剪（尽力而为的派生投影；中断由启动一致性校验自愈）
        let _ = self.index.prune_events(key, base_seq).await;

        // 5. 释放并重建追加句柄（重写后 inode 变化，旧句柄指向被替换的旧文件；
        //    续接点 = 快照 seq，压缩后末尾 seq 不变）
        self.logs.lock().unwrap().remove(key);
        self.logs
            .lock()
            .unwrap()
            .insert(key.clone(), (EventLog::open(&path)?, snap.seq));

        Ok(CompactionResult {
            snapshot_seq: snap.seq,
            base_seq,
            retained,
            discarded,
        })
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

    /// 追加一条读审计记录（D25 子项 3）：独立审计流 append + fsync。
    ///
    /// 调用方为引擎 actor（单写者）。失败须上抛——开启读审计后 fail-closed，
    /// 不允许"读成功但无审计"。
    pub fn append_read_audit(
        &self,
        key: &InstanceKey,
        rec: &ReadAuditRecord,
    ) -> Result<(), StoreError> {
        key.validate()?;
        let path = log::read_audit_path(&self.data_dir, &key.domain, &key.instance);
        std::fs::create_dir_all(log::domain_dir(&self.data_dir, &key.domain))?;
        let mut line = serde_json::to_string(rec).map_err(|e| StoreError::Json {
            context: "读审计序列化",
            source: e,
        })?;
        line.push('\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| StoreError::Io {
                context: "打开读审计流",
                source: e,
            })?;
        file.write_all(line.as_bytes())
            .map_err(|e| StoreError::Io {
                context: "写入读审计流",
                source: e,
            })?;
        file.sync_all().map_err(|e| StoreError::Io {
            context: "读审计流 fsync",
            source: e,
        })?;
        Ok(())
    }

    /// 读取最近 `limit` 条读审计记录（按写入顺序，时间正序）；文件不存在 → 空。
    ///
    /// 从文件尾部按块反向扫描：内存与扫描量只与 `limit` 相关（扫描上限 16 MiB），
    /// 不随审计流总长度增长。崩溃可能留下末尾半行：仅末尾无换行时容忍其解析失败；
    /// 其余损坏行上抛（不静默跳过，避免掩盖篡改）。
    pub fn read_audit_tail(
        &self,
        key: &InstanceKey,
        limit: usize,
    ) -> Result<Vec<ReadAuditRecord>, StoreError> {
        key.validate()?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let path = log::read_audit_path(&self.data_dir, &key.domain, &key.instance);
        let mut file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => {
                return Err(StoreError::Io {
                    context: "读取读审计流",
                    source: e,
                })
            }
        };
        let len = file
            .metadata()
            .map_err(|e| StoreError::Io {
                context: "读取读审计流元数据",
                source: e,
            })?
            .len();
        if len == 0 {
            return Ok(Vec::new());
        }

        const CHUNK: u64 = 64 * 1024;
        const MAX_SCAN: u64 = 16 * 1024 * 1024;
        let mut pos = len;
        let mut buf: Vec<u8> = Vec::new();
        let mut newlines = 0usize;
        let mut scanned = 0u64;
        // 多读一行冗余，抵消块边界恰好落在行首时的丢弃。
        let want_newlines = limit.saturating_add(1);
        while pos > 0 && newlines <= want_newlines {
            let read_len = CHUNK.min(pos);
            pos -= read_len;
            scanned += read_len;
            if scanned > MAX_SCAN {
                return Err(StoreError::Io {
                    context: "读取读审计流",
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "审计尾部超过 16 MiB 扫描上限",
                    ),
                });
            }
            file.seek(SeekFrom::Start(pos))
                .map_err(|e| StoreError::Io {
                    context: "读取读审计流",
                    source: e,
                })?;
            let mut chunk = vec![0u8; read_len as usize];
            file.read_exact(&mut chunk).map_err(|e| StoreError::Io {
                context: "读取读审计流",
                source: e,
            })?;
            newlines += chunk.iter().filter(|b| **b == b'\n').count();
            buf.splice(0..0, chunk);
        }
        if pos > 0 {
            // 未读到文件头：首行可能被块边界截断（且可能切在多字节字符中间），
            // 丢弃到第一个换行为止，保证剩余字节是完整 UTF-8 行。
            match buf.iter().position(|b| *b == b'\n') {
                Some(idx) => {
                    buf.drain(..=idx);
                }
                None => buf.clear(),
            }
        }
        let text = String::from_utf8(buf).map_err(|e| StoreError::Io {
            context: "读审计解码",
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, e),
        })?;

        let ended_with_newline = text.ends_with('\n');
        let mut lines: Vec<&str> = text.split('\n').collect();
        if ended_with_newline {
            lines.pop(); // 末尾换行产生的空串
        }
        if !ended_with_newline {
            // 崩溃残留：末尾半行允许解析失败
            if let Some(last) = lines.last() {
                if serde_json::from_str::<ReadAuditRecord>(last).is_err() {
                    lines.pop();
                }
            }
        }

        let start = lines.len().saturating_sub(limit);
        let mut tail = Vec::with_capacity(lines.len() - start);
        for line in &lines[start..] {
            if line.trim().is_empty() {
                continue;
            }
            let rec: ReadAuditRecord =
                serde_json::from_str(line).map_err(|e| StoreError::Json {
                    context: "读审计解析",
                    source: e,
                })?;
            tail.push(rec);
        }
        Ok(tail)
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
    use pact_core::{
        ArchiveEntry, CounterState, Effect, ParticipantName, ThingRef, Token, TokenId, TypeName,
    };
    use std::io::Write;

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
            .write_snapshot(
                &key(),
                &Snapshot::new(1, 1, "domain d", state, BTreeMap::new()),
            )
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

    // ── 前缀压缩（D25 子项 1）──────────────────────────────────────────────

    fn seeded_events() -> Vec<Event> {
        vec![
            domain_loaded(1),
            Event::Seeded {
                seq: 2,
                ts: "T1".into(),
                token: token(1, "req"),
            },
            Event::Produced {
                seq: 3,
                ts: "T2".into(),
                token: token(2, "req"),
            },
        ]
    }

    fn snapshot_at(seq: u64) -> Snapshot {
        let mut state = CounterState::new();
        state.apply(Effect::Seeded(token(1, "req")));
        state.apply(Effect::Produced(token(2, "req")));
        let mut consumed = BTreeMap::new();
        consumed.insert(
            "r1".to_string(),
            (
                TokenId("t1".into()),
                ArchiveEntry {
                    token: token(1, "req"),
                    exhausted_by: ParticipantName("arch".into()),
                    exhausted_at: "T3".into(),
                },
            ),
        );
        Snapshot::new(seq, 1, "domain d", state, consumed)
    }

    #[tokio::test]
    async fn compact_truncates_prefix_and_seq_continues() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        for ev in seeded_events() {
            store.append(&key(), &ev).await.unwrap();
        }
        let res = store
            .compact(&key(), &snapshot_at(3), 0, "T9")
            .await
            .unwrap();
        assert_eq!(
            res,
            CompactionResult {
                snapshot_seq: 3,
                base_seq: 3,
                retained: 0,
                discarded: 3
            }
        );
        // 压缩后日志为空，但续接点仍为 3（不会重号覆盖）
        assert_eq!(store.last_seq(&key()), 3);
        store
            .append(
                &key(),
                &Event::Consumed {
                    seq: 4,
                    ts: "T4".into(),
                    token: token(1, "req"),
                    by: ParticipantName("arch".into()),
                    request_id: Some("r1".into()),
                },
            )
            .await
            .unwrap();

        // 重启：日志无 DomainLoaded，由快照锚恢复；seq 自 4 续接
        let store2 = Store::open(dir.path()).await.unwrap();
        let rec = store2.recover().await.unwrap();
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0].base_seq, 3);
        assert_eq!(rec[0].events.len(), 1);
        assert_eq!(rec[0].events[0].seq(), 4);
        assert_eq!(store2.last_seq(&key()), 4);
        let snap = store2.load_snapshot(&key()).unwrap().unwrap();
        assert_eq!(snap.seq, 3);
        assert_eq!(snap.consumed_requests.len(), 1);
    }

    #[tokio::test]
    async fn compact_keep_last_retains_tail() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        for ev in seeded_events() {
            store.append(&key(), &ev).await.unwrap();
        }
        store
            .append(
                &key(),
                &Event::Consumed {
                    seq: 4,
                    ts: "T4".into(),
                    token: token(1, "req"),
                    by: ParticipantName("arch".into()),
                    request_id: None,
                },
            )
            .await
            .unwrap();
        store
            .append(
                &key(),
                &Event::Terminated {
                    seq: 5,
                    ts: "T5".into(),
                },
            )
            .await
            .unwrap();

        let res = store
            .compact(&key(), &snapshot_at(5), 2, "T9")
            .await
            .unwrap();
        assert_eq!(
            res,
            CompactionResult {
                snapshot_seq: 5,
                base_seq: 3,
                retained: 2,
                discarded: 3
            }
        );
        let store2 = Store::open(dir.path()).await.unwrap();
        let rec = store2.recover().await.unwrap();
        assert_eq!(rec[0].base_seq, 3);
        let seqs: Vec<u64> = rec[0].events.iter().map(|e| e.seq()).collect();
        assert_eq!(seqs, vec![4, 5]);
    }

    #[tokio::test]
    async fn compacted_log_without_snapshot_is_fatal() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = Store::open(dir.path()).await.unwrap();
            for ev in seeded_events() {
                store.append(&key(), &ev).await.unwrap();
            }
        }
        // 手工制造"日志已截断、水位已记，但快照缺失"
        let path = log::events_path(dir.path(), "d", "default");
        log::rewrite_retaining(&path, 3).unwrap();
        log::write_logmeta(
            &log::logmeta_path(dir.path(), "d", "default"),
            &log::LogMeta {
                base_seq: 3,
                compacted_at: None,
                ..Default::default()
            },
        )
        .unwrap();

        let store = Store::open(dir.path()).await.unwrap();
        assert!(matches!(
            store.recover().await.unwrap_err(),
            StoreError::SnapshotRequiredForCompactedLog { base_seq: 3, .. }
        ));
    }

    // ── 读审计流（D25 子项 3）────────────────────────────────────────────

    fn audit_rec(ts: &str, participant: &str, token_id: &str) -> ReadAuditRecord {
        ReadAuditRecord {
            version: crate::EVENT_SCHEMA_VERSION,
            ts: ts.into(),
            domain: "d".into(),
            instance: "default".into(),
            participant: participant.into(),
            token_id: token_id.into(),
        }
    }

    #[tokio::test]
    async fn read_audit_append_and_tail() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        // 文件不存在 → 空（未开启读审计）
        assert!(store.read_audit_tail(&key(), 10).unwrap().is_empty());

        for i in 0..5 {
            let rec = audit_rec(&format!("T{i}"), "arch", &format!("t{i}"));
            store.append_read_audit(&key(), &rec).unwrap();
        }
        let all = store.read_audit_tail(&key(), 10).unwrap();
        assert_eq!(all.len(), 5);
        assert_eq!(all[0].ts, "T0");
        assert_eq!(all[0].version, crate::EVENT_SCHEMA_VERSION);

        // 只取尾部 N 条，保持时间正序
        let tail = store.read_audit_tail(&key(), 2).unwrap();
        assert_eq!(
            tail.iter().map(|r| r.ts.as_str()).collect::<Vec<_>>(),
            vec!["T3", "T4"]
        );
        // 独立审计流：未写入事件日志（不改变 seq/重放）
        assert!(!log::events_path(dir.path(), "d", "default").exists());
    }

    #[tokio::test]
    async fn read_audit_tolerates_trailing_partial_line() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        store
            .append_read_audit(&key(), &audit_rec("T0", "arch", "t0"))
            .unwrap();
        // 模拟崩溃残留：末尾无换行的半行
        let path = log::read_audit_path(dir.path(), "d", "default");
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        f.write_all(b"{\"ts\":\"T1\"").unwrap();
        let recs = store.read_audit_tail(&key(), 10).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].ts, "T0");
    }

    /// 反向块扫描：大文件只取尾部、跨多个 64 KiB 块仍保持顺序与完整性。
    #[tokio::test]
    async fn read_audit_tail_scans_multiple_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).await.unwrap();
        let path = log::read_audit_path(dir.path(), "d", "default");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = String::new();
        for i in 0..1500 {
            text.push_str(&serde_json::to_string(&audit_rec(&format!("T{i}"), "a", "t")).unwrap());
            text.push('\n');
        }
        assert!(text.len() > 64 * 1024, "需跨块以覆盖反向扫描");
        std::fs::write(&path, text).unwrap();

        let recs = store.read_audit_tail(&key(), 3).unwrap();
        assert_eq!(
            recs.iter().map(|r| r.ts.as_str()).collect::<Vec<_>>(),
            vec!["T1497", "T1498", "T1499"]
        );
        assert_eq!(store.read_audit_tail(&key(), 1000).unwrap().len(), 1000);
        assert_eq!(store.read_audit_tail(&key(), 0).unwrap().len(), 0);
    }
}

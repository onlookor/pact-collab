//! 事件日志（JSONL，真相源）：只追加、fsync、崩溃残留截断、前缀压缩（设计 §7.2/§7.4/§7.5）。
//!
//! 不变量：
//! - 每行一个 JSON 事件（顶层 `version` + 事件体），行尾 `\n`；
//! - 行 seq 在实例内单调且**连续**：首行 seq = `base_seq + 1`，其后逐行 +1。
//!   `base_seq` = 已被快照覆盖、可从日志丢弃的事件上界（未压缩 = 0），
//!   由旁路清单 `<instance>.logmeta.json` 承载（审计信息，可缺失）；
//! - 追加路径：序列化 → write_all（含换行）→ sync_all（fsync）——
//!   换行是行内最后字节，崩溃时任何前缀残留都不含完整行，
//!   恢复时截断到最后一个 `\n` 即得到一致日志；
//! - 压缩路径：写快照（提交点，原子）→ 写 logmeta（原子）→ 原子重写日志
//!   （tmp + fsync + rename，只保留 `seq > base_seq` 的行）。任一步崩溃后
//!   恢复看到的都是「未截断（超集，安全）」或「已截断（需快照覆盖）」之一。

use crate::error::StoreError;
use crate::event::{to_wire_json, Event, WireEvent};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// 追加句柄（每域实例一个，由 Store 持有；单写者——引擎 actor 串行化保证）。
pub struct EventLog {
    file: File,
}

impl EventLog {
    /// 以追加模式打开（不存在则创建）。
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self { file })
    }

    /// 追加一个事件并 fsync。事件 seq 由调用方保证逐行 +1（Store 层校验）。
    pub fn append(&mut self, ev: &Event) -> Result<(), StoreError> {
        let mut line = to_wire_json(ev)?;
        line.push('\n');
        self.file.write_all(line.as_bytes())?;
        self.file.sync_all()?; // 真相源持久化先于 ack（设计 §7.3 写入路径）
        Ok(())
    }
}

/// 日志清洗与读取结果。
pub struct ReadOutcome {
    /// 全部完整事件（清洗后）。
    pub events: Vec<Event>,
    /// 是否截断了崩溃残留的尾部半行。
    pub truncated: bool,
}

/// 读取并清洗事件日志：
/// 1. 尾部无 `\n` 的半行（崩溃残留）→ 截断到最后一个完整行；
/// 2. 逐行解析（含 `version` 兼容校验）并校验 seq **连续**（首行 = `base_seq + 1`，
///    其后逐行 +1）；完整行损坏 → CorruptLine（无法自愈）。
///
/// `base_seq` 由旁路清单给出（未压缩 = 0）。允许日志「尚未截断」的崩溃中间态：
/// 此时首行 seq = 1 < `base_seq + 1`，是安全的超集（多出的行已被快照覆盖，重放时被跳过）。
pub fn read_and_sanitize(path: &Path, base_seq: u64) -> Result<ReadOutcome, StoreError> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    if bytes.is_empty() {
        return Ok(ReadOutcome {
            events: Vec::new(),
            truncated: false,
        });
    }

    // 1. 截断尾部半行
    let mut truncated = false;
    if *bytes.last().unwrap() != b'\n' {
        let cut = bytes
            .iter()
            .rposition(|&b| b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        bytes.truncate(cut);
        truncated = true;
        let f = OpenOptions::new().write(true).open(path)?;
        f.set_len(cut as u64)?;
        f.sync_all()?;
    }

    // 2. 逐行解析 + seq 连续性校验（连续锚点 = max(base_seq, 首行 seq - 1)）
    let text = String::from_utf8_lossy(&bytes);
    let mut events = Vec::new();
    let mut anchor: Option<u64> = None; // 上一行的 seq
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(StoreError::CorruptLine {
                path: path.to_owned(),
                line: idx + 1,
                reason: "空行".into(),
            });
        }
        let wire: WireEvent = serde_json::from_str(line).map_err(|e| StoreError::CorruptLine {
            path: path.to_owned(),
            line: idx + 1,
            reason: e.to_string(),
        })?;
        // 版本过高 → UnsupportedSchemaVersion 原样上抛（fail-fast，不按 v1 重放）
        let ev = wire.into_event()?;
        let expected = match anchor {
            Some(prev) => prev + 1,
            // 首行：正常为 base_seq+1；未截断的崩溃中间态为首行 seq ≤ base_seq+1
            None => {
                if ev.seq() > base_seq + 1 {
                    base_seq + 1
                } else {
                    ev.seq()
                }
            }
        };
        if ev.seq() != expected {
            return Err(StoreError::SeqNotMonotonic {
                path: path.to_owned(),
                expected,
                got: ev.seq(),
            });
        }
        anchor = Some(ev.seq());
        events.push(ev);
    }
    Ok(ReadOutcome { events, truncated })
}

/// 域名 → 域子目录（双键布局，D27）：`<data_dir>/<domain>`。
pub fn domain_dir(data_dir: &Path, domain: &str) -> PathBuf {
    data_dir.join(domain)
}

/// 实例键 → 事件日志文件路径：`<data_dir>/<domain>/<instance>.events.jsonl`。
pub fn events_path(data_dir: &Path, domain: &str, instance: &str) -> PathBuf {
    domain_dir(data_dir, domain).join(format!("{instance}.events.jsonl"))
}

/// 实例键 → 快照文件路径：`<data_dir>/<domain>/<instance>.snapshot.json`。
pub fn snapshot_path(data_dir: &Path, domain: &str, instance: &str) -> PathBuf {
    domain_dir(data_dir, domain).join(format!("{instance}.snapshot.json"))
}

/// 域名 → 声明原文文件路径：`<data_dir>/<domain>/declaration.pact`。
/// 定义态缓存（非审计记录）：declare 时写入，recover 用它回填声明注册表。
pub fn declaration_path(data_dir: &Path, domain: &str) -> PathBuf {
    domain_dir(data_dir, domain).join("declaration.pact")
}

/// 实例键 → 压缩清单文件路径：`<data_dir>/<domain>/<instance>.logmeta.json`。
pub fn logmeta_path(data_dir: &Path, domain: &str, instance: &str) -> PathBuf {
    domain_dir(data_dir, domain).join(format!("{instance}.logmeta.json"))
}

/// 实例键 → 读审计流路径：`<data_dir>/<domain>/<instance>.read-audit.jsonl`。
/// 与事件日志分离（D25 子项 3）：不进入 seq/重放/快照/压缩。
pub fn read_audit_path(data_dir: &Path, domain: &str, instance: &str) -> PathBuf {
    domain_dir(data_dir, domain).join(format!("{instance}.read-audit.jsonl"))
}

fn default_version() -> u32 {
    crate::event::EVENT_SCHEMA_VERSION
}

/// 压缩清单（旁路，`<instance>.logmeta.json`）：日志前缀压缩的水位与审计信息。
///
/// **非载荷关键**：可缺失（按未压缩 `base_seq = 0` 处理）。它记录：
/// 已被快照覆盖、可从日志丢弃的事件上界 `base_seq` 与最近压缩时间。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogMeta {
    #[serde(default = "default_version")]
    pub version: u32,
    /// 已被快照覆盖、可丢弃的事件上界（0 = 从未压缩）。
    pub base_seq: u64,
    /// 最近一次压缩时间（ISO-8601；未压缩为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compacted_at: Option<String>,
}

impl Default for LogMeta {
    fn default() -> Self {
        Self {
            version: crate::event::EVENT_SCHEMA_VERSION,
            base_seq: 0,
            compacted_at: None,
        }
    }
}

/// 读压缩清单：缺失 → None；JSON 损坏 → None（按未压缩处理，日志自身校验兜底）；
/// 版本高于支持上限 → `UnsupportedSchemaVersion`（fail-fast）。
pub fn load_logmeta(path: &Path) -> Result<Option<LogMeta>, StoreError> {
    match std::fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(StoreError::Io {
            context: "读取压缩清单",
            source: e,
        }),
        Ok(bytes) => match serde_json::from_slice::<LogMeta>(&bytes) {
            Ok(m) if m.version > crate::event::EVENT_SCHEMA_VERSION => {
                Err(StoreError::UnsupportedSchemaVersion {
                    found: m.version,
                    supported: crate::event::EVENT_SCHEMA_VERSION,
                })
            }
            Ok(m) => Ok(Some(m)),
            Err(_) => Ok(None),
        },
    }
}

/// 原子写压缩清单（tmp + fsync + rename）。先于日志重写落盘，保证崩溃窗口可恢复。
pub fn write_logmeta(path: &Path, meta: &LogMeta) -> Result<(), StoreError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec(meta).map_err(|e| StoreError::Json {
        context: "压缩清单序列化",
        source: e,
    })?;
    {
        let mut f = File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

/// 原子重写事件日志，只保留 `seq > base_seq` 的行（前缀压缩）。
///
/// 返回 `(保留行数, 丢弃行数)`。原行字节原样保留（不改写真相源内容）；
/// 尾部无 `\n` 的崩溃残留片段丢弃（等价于启动清洗）。写 tmp + fsync + rename 原子替换。
pub fn rewrite_retaining(path: &Path, base_seq: u64) -> Result<(u64, u64), StoreError> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    let mut kept: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut retained = 0u64;
    let mut discarded = 0u64;
    for chunk in bytes.split_inclusive(|&b| b == b'\n') {
        // 尾部半行（崩溃残留，无换行）：丢弃，等同启动清洗
        if !chunk.ends_with(b"\n") {
            continue;
        }
        let text = String::from_utf8_lossy(&chunk[..chunk.len() - 1]);
        if text.trim().is_empty() {
            return Err(StoreError::CorruptLine {
                path: path.to_owned(),
                line: (retained + discarded + 1) as usize,
                reason: "空行".into(),
            });
        }
        let wire: WireEvent = serde_json::from_str(&text).map_err(|e| StoreError::CorruptLine {
            path: path.to_owned(),
            line: (retained + discarded + 1) as usize,
            reason: e.to_string(),
        })?;
        let ev = wire.into_event()?;
        if ev.seq() > base_seq {
            retained += 1;
            kept.extend_from_slice(chunk);
        } else {
            discarded += 1;
        }
    }

    let tmp = path.with_extension("jsonl.tmp");
    {
        let mut f = File::create(&tmp)?;
        f.write_all(&kept)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok((retained, discarded))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pact_core::{ParticipantName, ThingRef, Timestamp, Token, TokenId, TypeName};

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

    /// 双键布局测试路径（建域子目录）；本模块只关心日志机制，与实例无关。
    fn test_path(dir: &Path) -> PathBuf {
        std::fs::create_dir_all(domain_dir(dir, "d")).unwrap();
        events_path(dir, "d", "default")
    }

    fn sample_events() -> Vec<Event> {
        vec![
            Event::DomainLoaded {
                seq: 1,
                ts: "T0".into(),
                name: "d".into(),
                declaration: "…".into(),
            },
            Event::Seeded {
                seq: 2,
                ts: "T1".into(),
                token: token(1, "req"),
            },
        ]
    }

    #[test]
    fn append_read_roundtrip_with_fsync() {
        let dir = tempfile::tempdir().unwrap();
        let path = test_path(dir.path());
        let mut log = EventLog::open(&path).unwrap();
        for ev in sample_events() {
            log.append(&ev).unwrap();
        }
        let out = read_and_sanitize(&path, 0).unwrap();
        assert!(!out.truncated);
        assert_eq!(out.events, sample_events());
    }

    #[test]
    fn half_line_is_truncated_on_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = test_path(dir.path());
        let mut log = EventLog::open(&path).unwrap();
        for ev in sample_events() {
            log.append(&ev).unwrap();
        }
        // 模拟崩溃残留：追加半行（无换行）
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(br#"{"kind":"Produced","seq":3,"ts":"T2","token":{"id":"t3"#)
            .unwrap();
        drop(f);

        let out = read_and_sanitize(&path, 0).unwrap();
        assert!(out.truncated);
        assert_eq!(out.events, sample_events());
        // 再读一次：幂等（无残留可截）
        let out2 = read_and_sanitize(&path, 0).unwrap();
        assert!(!out2.truncated);
        assert_eq!(out2.events, sample_events());
    }

    #[test]
    fn corrupt_complete_line_is_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = test_path(dir.path());
        std::fs::write(
            &path,
            concat!(
                r#"{"kind":"DomainLoaded","seq":1,"ts":"T0","name":"d","declaration":"x"}"#,
                "\n",
                "not json at all\n"
            ),
        )
        .unwrap();
        assert!(matches!(
            read_and_sanitize(&path, 0),
            Err(StoreError::CorruptLine { line: 2, .. })
        ));
    }

    #[test]
    fn seq_gap_is_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = test_path(dir.path());
        let mut log = EventLog::open(&path).unwrap();
        log.append(&Event::DomainLoaded {
            seq: 1,
            ts: "T0".into(),
            name: "d".into(),
            declaration: "x".into(),
        })
        .unwrap();
        // 直接追加 seq=3（跳过 2）：违反逐行 +1 不变量
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        let line = to_wire_json(&Event::Seeded {
            seq: 3,
            ts: "T1".into(),
            token: token(1, "req"),
        })
        .unwrap();
        writeln!(f, "{line}").unwrap();
        drop(f);
        assert!(matches!(
            read_and_sanitize(&path, 0),
            Err(StoreError::SeqNotMonotonic {
                expected: 2,
                got: 3,
                ..
            })
        ));
    }

    #[test]
    fn wire_json_carries_version_and_legacy_line_is_v1() {
        let ev = Event::Seeded {
            seq: 1,
            ts: "T0".into(),
            token: token(1, "req"),
        };
        let line = to_wire_json(&ev).unwrap();
        assert!(line.contains(r#""version":1"#), "{line}");
        let parsed: WireEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.event, ev);

        // 历史日志行（无 version 字段）→ 按 v1 解析
        let legacy = r#"{"kind":"DomainLoaded","seq":1,"ts":"T0","name":"d","declaration":"x"}"#;
        let parsed: WireEvent = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.version, 1);
        assert!(matches!(parsed.event, Event::DomainLoaded { .. }));
    }

    #[test]
    fn future_schema_version_is_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = test_path(dir.path());
        std::fs::write(
            &path,
            concat!(
                r#"{"version":99,"kind":"DomainLoaded","seq":1,"ts":"T0","name":"d","declaration":"x"}"#,
                "\n"
            ),
        )
        .unwrap();
        assert!(matches!(
            read_and_sanitize(&path, 0),
            Err(StoreError::UnsupportedSchemaVersion {
                found: 99,
                supported: 1
            })
        ));
    }

    #[test]
    fn compacted_log_validates_from_base_seq() {
        let dir = tempfile::tempdir().unwrap();
        let path = test_path(dir.path());
        let mut log = EventLog::open(&path).unwrap();
        for ev in [
            Event::DomainLoaded {
                seq: 1,
                ts: "T0".into(),
                name: "d".into(),
                declaration: "x".into(),
            },
            Event::Seeded {
                seq: 2,
                ts: "T1".into(),
                token: token(1, "req"),
            },
            Event::Seeded {
                seq: 3,
                ts: "T2".into(),
                token: token(2, "req"),
            },
        ] {
            log.append(&ev).unwrap();
        }
        drop(log);

        let (retained, discarded) = rewrite_retaining(&path, 2).unwrap();
        assert_eq!((retained, discarded), (1, 2));

        // base_seq = 2 → 首行允许为 3
        let out = read_and_sanitize(&path, 2).unwrap();
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].seq(), 3);
        // 清单缺失（基 0）→ 首行 3 ≠ 1：损坏（fail-fast，提示需快照）
        assert!(read_and_sanitize(&path, 0).is_err());
    }

    #[test]
    fn logmeta_roundtrip_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = logmeta_path(dir.path(), "d", "default");
        assert_eq!(load_logmeta(&path).unwrap(), None);
        let meta = LogMeta {
            version: 1,
            base_seq: 7,
            compacted_at: Some("2026-10-20T00:00:00Z".into()),
        };
        write_logmeta(&path, &meta).unwrap();
        assert_eq!(load_logmeta(&path).unwrap(), Some(meta));
        assert!(!path.with_extension("json.tmp").exists());
    }

    // 静默未使用告警消解（Timestamp 仅在类型签名中出现）
    #[allow(dead_code)]
    fn _ts(_: Timestamp) {}
}

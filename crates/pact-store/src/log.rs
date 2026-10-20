//! 事件日志（JSONL，真相源）：只追加、fsync、崩溃残留截断（设计 §7.2/§7.4）。
//!
//! 不变量：
//! - 每行一个 JSON 事件，行尾 `\n`；
//! - 行 i（0 基）的事件 seq == i+1（逐行 +1 单调，读侧强制校验）；
//! - 追加路径：序列化 → write_all（含换行）→ sync_all（fsync）——
//!   换行是行内最后字节，崩溃时任何前缀残留都不含完整行，
//!   恢复时截断到最后一个 `\n` 即得到一致日志。

use crate::error::StoreError;
use crate::event::Event;
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
        let mut line = serde_json::to_string(ev).map_err(|e| StoreError::Json {
            context: "事件序列化",
            source: e,
        })?;
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
/// 2. 逐行解析并校验 seq == 行号+1；完整行损坏 → CorruptLine（无法自愈）。
pub fn read_and_sanitize(path: &Path) -> Result<ReadOutcome, StoreError> {
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

    // 2. 逐行解析 + seq 单调校验
    let text = String::from_utf8_lossy(&bytes);
    let mut events = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            return Err(StoreError::CorruptLine {
                path: path.to_owned(),
                line: idx + 1,
                reason: "空行".into(),
            });
        }
        let ev: Event = serde_json::from_str(line).map_err(|e| StoreError::CorruptLine {
            path: path.to_owned(),
            line: idx + 1,
            reason: e.to_string(),
        })?;
        let expected = (idx + 1) as u64;
        if ev.seq() != expected {
            return Err(StoreError::SeqNotMonotonic {
                path: path.to_owned(),
                expected,
                got: ev.seq(),
            });
        }
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
        let out = read_and_sanitize(&path).unwrap();
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

        let out = read_and_sanitize(&path).unwrap();
        assert!(out.truncated);
        assert_eq!(out.events, sample_events());
        // 再读一次：幂等（无残留可截）
        let out2 = read_and_sanitize(&path).unwrap();
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
            read_and_sanitize(&path),
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
        let line = serde_json::to_string(&Event::Seeded {
            seq: 3,
            ts: "T1".into(),
            token: token(1, "req"),
        })
        .unwrap();
        writeln!(f, "{line}").unwrap();
        drop(f);
        assert!(matches!(
            read_and_sanitize(&path),
            Err(StoreError::SeqNotMonotonic {
                expected: 2,
                got: 3,
                ..
            })
        ));
    }

    // 静默未使用告警消解（Timestamp 仅在类型签名中出现）
    #[allow(dead_code)]
    fn _ts(_: Timestamp) {}
}

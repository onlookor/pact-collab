//! 快照（设计 §7.2/§7.5）：`<domain>/<instance>.snapshot.json`，原子写（tmp + fsync + rename）。
//!
//! 快照自 v1 起为**实例自足锚**：除 `seq + state` 外还携带实例声明原文、最近
//! `DomainLoaded` 的 seq 与 Rule-E 幂等表——日志前缀被压缩后，快照是唯一的历史载体，
//! 重放不再要求日志中存在 `DomainLoaded` 行。
//! 快照丢失/损坏不影响正确性（重放兜底，设计 §7.4），故读取失败按"无快照"处理；
//! 唯一的例外：日志已被压缩（无 `DomainLoaded` 行）而快照缺失 → 由 store 拒绝启动。

use crate::error::StoreError;
use crate::event::EVENT_SCHEMA_VERSION;
use pact_core::{ArchiveEntry, CounterState, TokenId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

fn default_version() -> u32 {
    EVENT_SCHEMA_VERSION
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    /// 快照 schema 版本（缺省 v1；高于支持上限 → fail-fast）。
    #[serde(default = "default_version")]
    pub version: u32,
    /// 已包含进本快照的最后事件 seq（含）。
    pub seq: u64,
    /// 实例最近一次 `DomainLoaded` 的 seq（「实例非空」判据基线；压缩后由快照承载）。
    #[serde(default)]
    pub dl_seq: u64,
    /// 实例声明原文快照（实例创建/重置时的声明；压缩后日志不再有 `DomainLoaded` 行）。
    #[serde(default)]
    pub declaration: String,
    pub state: CounterState,
    /// Rule-E 幂等表快照：request_id → (目标 token id, 首次归档结果)。
    /// 使被压缩掉的 `Consumed` 行不再需要留存（幂等语义跨重启保持）。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub consumed_requests: BTreeMap<String, (TokenId, ArchiveEntry)>,
}

impl Snapshot {
    /// 构造当前 schema 版本的快照。
    pub fn new(
        seq: u64,
        dl_seq: u64,
        declaration: impl Into<String>,
        state: CounterState,
        consumed_requests: BTreeMap<String, (TokenId, ArchiveEntry)>,
    ) -> Self {
        Self {
            version: EVENT_SCHEMA_VERSION,
            seq,
            dl_seq,
            declaration: declaration.into(),
            state,
            consumed_requests,
        }
    }
}

/// 原子写快照：tmp 文件 → fsync → rename。
pub fn write_snapshot(path: &Path, snap: &Snapshot) -> Result<(), StoreError> {
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec(snap).map_err(|e| StoreError::Json {
        context: "快照序列化",
        source: e,
    })?;
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&json)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    // 目录项 fsync（best-effort：个别平台不支持）
    if let Some(parent) = path.parent() {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

/// 读快照。文件不存在 → None；损坏 → None（重放兜底，设计 §7.4）；
/// 版本高于支持上限 → `UnsupportedSchemaVersion`（fail-fast）。
pub fn load_snapshot(path: &Path) -> Result<Option<Snapshot>, StoreError> {
    match fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(StoreError::Io {
            context: "读取快照",
            source: e,
        }),
        Ok(bytes) => match serde_json::from_slice::<Snapshot>(&bytes) {
            Ok(s) if s.version > EVENT_SCHEMA_VERSION => {
                Err(StoreError::UnsupportedSchemaVersion {
                    found: s.version,
                    supported: EVENT_SCHEMA_VERSION,
                })
            }
            Ok(s) => Ok(Some(s)),
            Err(_) => Ok(None),
        },
    }
}

/// 删除快照（域被替换时清理陈旧快照）。
pub fn remove_snapshot(path: &Path) -> Result<(), StoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(StoreError::Io {
            context: "删除快照",
            source: e,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pact_core::{Effect, ParticipantName, ThingRef, Token, TokenId, TypeName};

    fn token(id: &str) -> Token {
        Token {
            id: TokenId(id.into()),
            ty: TypeName("req".into()),
            thing_ref: ThingRef::none(),
            produced_by: ParticipantName("p".into()),
            depends_on: vec![],
            produced_at: "T0".into(),
        }
    }

    #[test]
    fn snapshot_atomic_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.snapshot.json");
        let mut state = CounterState::new();
        state.apply(Effect::Seeded(token("t1")));
        let snap = Snapshot::new(7, 1, "types { req }", state, BTreeMap::new());
        write_snapshot(&path, &snap).unwrap();
        assert_eq!(load_snapshot(&path).unwrap(), Some(snap));
        // tmp 文件不残留
        assert!(!dir.path().join("d.snapshot.json.tmp").exists());
    }

    #[test]
    fn legacy_snapshot_defaults_to_v1() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.snapshot.json");
        let mut state = CounterState::new();
        state.apply(Effect::Seeded(token("t1")));
        // 旧格式：仅 { seq, state }（无 version/declaration/dl_seq/幂等表）
        let legacy = format!(
            r#"{{"seq":4,"state":{}}}"#,
            serde_json::to_string(&state).unwrap()
        );
        std::fs::write(&path, legacy).unwrap();
        let snap = load_snapshot(&path).unwrap().unwrap();
        assert_eq!(snap.version, 1);
        assert_eq!(snap.seq, 4);
        assert_eq!(snap.dl_seq, 0);
        assert!(snap.declaration.is_empty());
        assert!(snap.consumed_requests.is_empty());
    }

    #[test]
    fn future_snapshot_version_is_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.snapshot.json");
        let mut state = CounterState::new();
        state.apply(Effect::Seeded(token("t1")));
        let mut snap = Snapshot::new(3, 1, "x", state, BTreeMap::new());
        snap.version = 99;
        write_snapshot(&path, &snap).unwrap();
        assert!(matches!(
            load_snapshot(&path),
            Err(StoreError::UnsupportedSchemaVersion {
                found: 99,
                supported: 1
            })
        ));
    }

    #[test]
    fn corrupt_snapshot_falls_back_to_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("d.snapshot.json");
        std::fs::write(&path, b"{ broken").unwrap();
        assert_eq!(load_snapshot(&path).unwrap(), None);
    }

    #[test]
    fn missing_snapshot_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_snapshot(&dir.path().join("none.json")).unwrap(), None);
    }
}

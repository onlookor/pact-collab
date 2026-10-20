//! 快照（设计 §7.2）：`<domain>/<instance>.snapshot.json` = { seq, state }，
//! 原子写（tmp + fsync + rename）。快照丢失/损坏不影响正确性——
//! 重放兜底（设计 §7.4），故读取失败按"无快照"处理。

use crate::error::StoreError;
use pact_core::CounterState;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    /// 已包含进本快照的最后事件 seq（含）。
    pub seq: u64,
    pub state: CounterState,
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

/// 读快照。文件不存在 → None；损坏 → None（重放兜底，设计 §7.4）。
pub fn load_snapshot(path: &Path) -> Result<Option<Snapshot>, StoreError> {
    match fs::read(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(StoreError::Io {
            context: "读取快照",
            source: e,
        }),
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).ok()),
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
        let snap = Snapshot { seq: 7, state };
        write_snapshot(&path, &snap).unwrap();
        assert_eq!(load_snapshot(&path).unwrap(), Some(snap));
        // tmp 文件不残留
        assert!(!dir.path().join("d.snapshot.json.tmp").exists());
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

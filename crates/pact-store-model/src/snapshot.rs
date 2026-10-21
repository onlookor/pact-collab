//! 快照**数据结构**（设计 §7.2/§7.5）。
//!
//! 本模块只定义可序列化的自足锚；读写（tmp + fsync + rename、损坏兜底、
//! 版本 fail-fast 的 IO 部分）留在原生 `pact-store`。

use pact_core::{ArchiveEntry, CounterState, TokenId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn default_version() -> u32 {
    crate::event::EVENT_SCHEMA_VERSION
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
            version: crate::event::EVENT_SCHEMA_VERSION,
            seq,
            dl_seq,
            declaration: declaration.into(),
            state,
            consumed_requests,
        }
    }
}

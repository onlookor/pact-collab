//! Counter 变更通知（Rule-W 唤醒，D4；线上格式设计 §6.4）。
//! 每域实例一个 broadcast 通道，状态变更事件即时广播；
//! 轮询兜底 = QueryEnabled / QueryStatus。

use pact_core::{Timestamp, TokenId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum NotificationKind {
    Seeded,
    Produced,
    Consumed,
    Terminated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Notification {
    /// 通知 schema 版本（与事件日志同一版本域；缺省 v1，D25 子项 1）。
    #[serde(default = "default_version")]
    pub version: u32,
    /// 事件种类（§6.4 `event` 字段）。
    pub event: NotificationKind,
    pub domain: String,
    /// 实例键（默认 `default`，D27）。
    pub instance: String,
    /// 触发本通知的事件 seq（实例内单调）。
    pub seq: u64,
    pub ts: Timestamp,
    /// 关联 token（Terminated 无）。
    pub token_id: Option<TokenId>,
    pub summary: NotificationSummary,
}

fn default_version() -> u32 {
    pact_store::EVENT_SCHEMA_VERSION
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSummary {
    /// 变更后的 Counter 规模。
    pub counter_size: usize,
}

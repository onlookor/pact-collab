//! Counter 变更通知（Rule-W 唤醒，D4；线上格式设计 §6.4）。
//! 每域一个 broadcast 通道，状态变更事件即时广播；
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
    /// 事件种类（§6.4 `event` 字段）。
    pub event: NotificationKind,
    pub domain: String,
    /// 触发本通知的事件 seq（域内单调）。
    pub seq: u64,
    pub ts: Timestamp,
    /// 关联 token（Terminated 无）。
    pub token_id: Option<TokenId>,
    pub summary: NotificationSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSummary {
    /// 变更后的 Counter 规模。
    pub counter_size: usize,
}

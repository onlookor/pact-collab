//! 读审计流（D25 子项 3，FR17）：Rule-R 读动作的独立审计记录。
//!
//! 与主事件日志**分离**（`<domain>/<instance>.read-audit.jsonl`，只追加 + fsync）：
//! - 不进入事件 seq / 重放 / Rule-T / Rule-E，因此不改变核心语义；
//! - 不参与快照与压缩（审计流独立增长，运维可按需轮转）；
//! - 仅在 `pact.toml` 显式开启 `read_audit = true` 时写入；开启后写失败即拒绝读取
//!   （fail-closed，避免"读过但无审计"）。

use serde::{Deserialize, Serialize};

fn default_version() -> u32 {
    pact_store_model::EVENT_SCHEMA_VERSION
}

/// 单条读审计记录（一行 JSON）。字段自足：时间、实例键、参与者、目标 token。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadAuditRecord {
    /// 审计流 schema 版本（与事件日志同版本语义）。
    #[serde(default = "default_version")]
    pub version: u32,
    /// 读取时间（ISO-8601，由引擎 Clock 提供）。
    pub ts: String,
    pub domain: String,
    pub instance: String,
    pub participant: String,
    pub token_id: String,
}

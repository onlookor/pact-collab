//! 读审计流（D25 子项 3 / D43）：Rule-R 读取与视图读取的独立审计记录。
//!
//! 与主事件日志**分离**（`<domain>/<instance>.read-audit.jsonl`，只追加 + fsync）：
//! - 不进入事件 seq / 重放 / Rule-T / Rule-E，因此不改变核心语义；
//! - 不参与快照与压缩（审计流独立增长，运维可按需轮转）；
//! - 仅在 `pact.toml` 显式开启 `read_audit = true` 时写入；开启后写失败即拒绝本次读取
//!   （fail-closed，避免"读过但无审计"）。
//!
//! D43 扩展：同一审计流承载两类记录，以 `kind` 区分。
//! - `read`：Rule-R 单 token 读取（旧记录无 `kind`，按 `read` 解析）；
//! - `view`：Counter/Archive 视图读取（按类型聚合，只记计数，不记 `thing_ref` 值）。

use serde::{Deserialize, Serialize};

fn default_version() -> u32 {
    pact_store_model::EVENT_SCHEMA_VERSION
}

/// 审计记录类别（D43）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ReadAuditKind {
    /// Rule-R 单 token 读取；旧审计流无 `kind` 字段时缺省为此。
    #[default]
    Read,
    /// Counter / Archive 视图读取。
    View,
}

/// 视图名（D43，`kind = view` 时出现）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewName {
    Counter,
    Archive,
}

/// 单类型的视图暴露统计（D43）。
///
/// 只记录计数，不记录 `thing_ref` 值——审计流同样受"无秘密"约束。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewAuditType {
    /// 视图返回的 token 类型。
    #[serde(rename = "type")]
    pub ty: String,
    /// 该类型在本视图中的 token 总数。
    pub tokens: usize,
    /// 其中实际返回了 `thing_ref` 的 token 数。
    pub refs_visible: usize,
    /// 其中因可见性策略被清空 `thing_ref` 的 token 数。
    pub refs_filtered: usize,
}

/// 单条读审计记录（一行 JSON）。字段自足：时间、实例键、调用者与读取目标。
///
/// `read` 记录填写 `participant` + `token_id`；`view` 记录填写 `view` +
/// `type_filter` + `types`，匿名视图读取的 `participant` 为 `None`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadAuditRecord {
    /// 审计流 schema 版本（与事件日志同版本语义）。
    #[serde(default = "default_version")]
    pub version: u32,
    /// 记录类别；缺省 `read` 以兼容 D43 之前的审计流。
    #[serde(default)]
    pub kind: ReadAuditKind,
    /// 读取时间（ISO-8601，由引擎 Clock 提供）。
    pub ts: String,
    pub domain: String,
    pub instance: String,
    /// 调用者主体；信任声明模式下的匿名视图读取为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub participant: Option<String>,
    /// Rule-R 读取的目标 token；视图记录为 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_id: Option<String>,
    /// 视图名（`kind = view`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<ViewName>,
    /// 调用方的类型过滤器（`GET /counter?type=` / `GET /archive?type=`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_filter: Option<String>,
    /// 按类型聚合的暴露统计（`kind = view`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<ViewAuditType>>,
}

impl ReadAuditRecord {
    /// Rule-R 读取记录。
    pub fn rule_r(
        ts: String,
        domain: String,
        instance: String,
        participant: String,
        token_id: String,
    ) -> Self {
        Self {
            version: pact_store_model::EVENT_SCHEMA_VERSION,
            kind: ReadAuditKind::Read,
            ts,
            domain,
            instance,
            participant: Some(participant),
            token_id: Some(token_id),
            view: None,
            type_filter: None,
            types: None,
        }
    }

    /// 视图读取记录（D43）。
    pub fn view(
        ts: String,
        domain: String,
        instance: String,
        participant: Option<String>,
        view: ViewName,
        type_filter: Option<String>,
        types: Vec<ViewAuditType>,
    ) -> Self {
        Self {
            version: pact_store_model::EVENT_SCHEMA_VERSION,
            kind: ReadAuditKind::View,
            ts,
            domain,
            instance,
            participant,
            token_id: None,
            view: Some(view),
            type_filter,
            types: Some(types),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D43 兼容：D43 之前的 Rule-R 记录没有 `kind`，按 `read` 解析。
    #[test]
    fn legacy_read_record_defaults_kind() {
        let line = r#"{"version":1,"ts":"T0","domain":"proj","instance":"default","participant":"architect","token_id":"t1"}"#;
        let rec: ReadAuditRecord = serde_json::from_str(line).unwrap();
        assert_eq!(rec.kind, ReadAuditKind::Read);
        assert_eq!(rec.participant.as_deref(), Some("architect"));
        assert_eq!(rec.token_id.as_deref(), Some("t1"));
        assert!(rec.view.is_none());
        assert!(rec.types.is_none());
    }

    /// 视图记录往返：匿名主体、类型过滤器与计数完整保留。
    #[test]
    fn view_record_roundtrip() {
        let rec = ReadAuditRecord::view(
            "T1".to_owned(),
            "proj".to_owned(),
            "default".to_owned(),
            None,
            ViewName::Counter,
            Some("design".to_owned()),
            vec![ViewAuditType {
                ty: "design".to_owned(),
                tokens: 2,
                refs_visible: 1,
                refs_filtered: 1,
            }],
        );
        let line = serde_json::to_string(&rec).unwrap();
        let back: ReadAuditRecord = serde_json::from_str(&line).unwrap();
        assert_eq!(back, rec);
        assert_eq!(back.kind, ReadAuditKind::View);
        assert!(back.token_id.is_none());
        assert_eq!(back.view, Some(ViewName::Counter));
        assert_eq!(back.type_filter.as_deref(), Some("design"));
        assert_eq!(back.types.as_ref().unwrap()[0].refs_filtered, 1);
    }

    /// 视图记录不携带 `thing_ref` 值，也不序列化空字段。
    #[test]
    fn view_record_omits_absent_fields() {
        let rec = ReadAuditRecord::view(
            "T2".to_owned(),
            "proj".to_owned(),
            "default".to_owned(),
            None,
            ViewName::Archive,
            None,
            Vec::new(),
        );
        let value: serde_json::Value = serde_json::to_value(&rec).unwrap();
        assert!(value.get("participant").is_none());
        assert!(value.get("token_id").is_none());
        assert!(value.get("type_filter").is_none());
        assert_eq!(value["kind"], "view");
        assert_eq!(value["view"], "archive");
    }
}

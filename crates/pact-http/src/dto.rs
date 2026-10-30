//! §6 线上格式 DTO（三接口共享）。
//!
//! 字段名 snake_case；时间戳为 ISO-8601 UTC 字符串（由 core 类型自带语义，
//! 格式由引擎层 Clock 保证）。

use pact_core::{ArchiveEntry, ProductionOption, RuleError, Token, TokenId, TypeName};
use pact_engine::views::{DomainListing, GateClause};
use pact_parser::Problem;
use serde::de::Deserializer;
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

// ============ 请求 ============

/// 请求体中的 `thing_ref` 字段，区分三种输入：
/// 字段缺失（作者漏传，400）、显式 `null`（`∅`）、字符串（有引用）。
///
/// `serde` 的 `Option<Option<String>>` 无法区分缺失与 `null`（两者都反序列化为 `None`），
/// 因此用自定义类型保留“字段是否出现”。
#[derive(Debug, Clone, Default)]
pub struct ThingRefField(Option<Option<String>>);

impl ThingRefField {
    pub fn missing() -> Self {
        Self(None)
    }

    pub fn explicit(value: Option<String>) -> Self {
        Self(Some(value))
    }

    pub fn is_missing(&self) -> bool {
        self.0.is_none()
    }

    /// `None` = 缺失；`Some(None)` = 显式 `null`；`Some(Some(v))` = 有引用。
    pub fn as_input(&self) -> Option<Option<&str>> {
        self.0.as_ref().map(|value| value.as_deref())
    }
}

impl Serialize for ThingRefField {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0.as_ref() {
            Some(value) => value.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }
}

impl<'de> Deserialize<'de> for ThingRefField {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(Some(Option::<String>::deserialize(deserializer)?)))
    }
}

/// `POST /domains`：声明 + 实例化（不指定实例 ⇒ 默认实例 `default`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadDomainReq {
    pub name: String,
    pub declaration: String,
    /// 目标实例名（缺省 = `default`）。
    #[serde(default)]
    pub instance: Option<String>,
    /// 目标实例非空时是否允许替换（缺省 false → 409 DomainBusy）。
    #[serde(default)]
    pub force: Option<bool>,
}

/// `POST /domains/{d}/instances`：在已登记声明上创建额外实例。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstantiateReq {
    pub instance: String,
}

/// `POST /domains/{d}[/instances/{i}]/compact`：日志前缀压缩（D25 子项 1）。
///
/// 请求体可省略字段：`{}` 等价 `keep_last = 0`（丢弃全部已入快照的前缀）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactReq {
    /// 保留最近的事件条数（缺省 0）。
    #[serde(default)]
    pub keep_last: u64,
}

/// `POST /domains/{d}/handoff`：状态交接（冻结源 → 新声明目标 → 可选切别名）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandoffReq {
    pub source_instance: String,
    pub target_instance: String,
    /// 目标实例使用的完整声明原文。
    pub declaration: String,
    /// 交接成功后是否把域的活动实例指针切到目标（缺省 true）。
    #[serde(default = "default_true")]
    pub activate: bool,
}

fn default_true() -> bool {
    true
}

/// `PUT /domains/{d}/active`：切换活动实例指针。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivateReq {
    pub instance: String,
}

/// `GET /domains/{d}/active`：当前活动实例。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveResp {
    pub domain: String,
    pub instance: String,
    /// 控制面审计：指针写入时间（无指针 / 旧版文件为 `None`）。
    #[serde(default)]
    pub ts: Option<String>,
    /// 控制面审计：指针写入者（信任声明模式或旧版文件为 `None`）。
    #[serde(default)]
    pub by: Option<String>,
}

/// `PUT /domains/{d}/instances/{i}/freeze`：设置/解除实例冻结。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FreezeReq {
    pub frozen: bool,
}

/// `POST /domains/{d}:compose`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeReq {
    pub fragments: Vec<String>,
}

/// `POST /validate`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateReq {
    pub declaration: String,
}

/// `POST /validate` 成功响应：良构 + 结构概要（错误走 §6.3 4xx）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateResp {
    pub ok: bool,
    pub types: Vec<String>,
    pub participants: Vec<String>,
    pub gates: usize,
    pub seed_authority: usize,
}

/// `POST /domains/{d}/seeds`（Rule-S；无前驱）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedReq {
    pub participant: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub thing_ref: ThingRefField,
}

/// `POST /domains/{d}/tokens`（Rule-P）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProduceReq {
    pub participant: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub predecessors: Vec<String>,
    #[serde(default)]
    pub thing_ref: ThingRefField,
}

/// `POST .../tokens/{id}/read` | `.../consume`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParticipantReq {
    pub participant: String,
}

/// `POST .../tokens/{id}/consume`（Rule-E；可选幂等键）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsumeReq {
    pub participant: String,
    /// 可选幂等键：同一域实例内重复请求返回首次结果，不重复消费。
    #[serde(default)]
    pub request_id: Option<String>,
}

/// `POST /ref/reload`：控制面显式重载（D48）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefReloadReq {
    /// 只做校验与影响预检，不替换内存索引（缺省 false）。
    #[serde(rename = "dryRun", default)]
    pub dry_run: bool,
}

// ============ 响应 ============

/// `POST /ref/reload` 影响预检分组：按 `(domain, instance, type)` 统计。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedTokenGroup {
    pub domain: String,
    pub instance: String,
    #[serde(rename = "type")]
    pub ty: String,
    /// 重载后会因缺摘要而无法归档的存量 token 数（不输出票面值）。
    pub tokens: usize,
}

/// `POST /ref/reload` 成功响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefReloadResp {
    #[serde(rename = "dryRun")]
    pub dry_run: bool,
    /// `dryRun=true` 时恒为 false。
    pub applied: bool,
    pub bindings: usize,
    pub contracts: usize,
    #[serde(rename = "blockedTokens")]
    pub blocked_tokens: Vec<BlockedTokenGroup>,
    #[serde(rename = "blockedTotal")]
    pub blocked_total: usize,
}

/// §6.3 成功响应（Rule-S/P/R）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResp {
    pub token: Token,
}

/// Rule-E 成功响应（归档条目）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryResp {
    pub entry: ArchiveEntry,
}

/// `GET /domains`：定义键 + 其实例清单（D27）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainsResp {
    pub domains: Vec<DomainListing>,
}

/// `GET /domains/{d}`（含实例端点）：域信息 + 状态摘要。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainInfo {
    pub name: String,
    /// 实例键（默认 `default`）。
    pub instance: String,
    pub counter_size: usize,
    pub archive_size: usize,
    pub terminated: bool,
}

/// `GET /domains/{d}[/instances/{i}]/read-audit`：最近 Rule-R 读审计记录（时间正序）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadAuditResp {
    pub records: Vec<pact_store::ReadAuditRecord>,
}

/// `GET /domains/{d}/gates/{participant}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatesResp {
    pub gates: Vec<GateClause>,
}

/// §6.5 `enabled` 条目：线上字段名为 `type`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnabledOption {
    #[serde(rename = "type")]
    pub ty: TypeName,
    pub witness_predecessors: Vec<TokenId>,
}

impl From<ProductionOption> for EnabledOption {
    fn from(o: ProductionOption) -> Self {
        Self {
            ty: o.ty,
            witness_predecessors: o.witness_predecessors,
        }
    }
}

/// `GET /domains/{d}/enabled`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnabledResp {
    pub options: Vec<EnabledOption>,
}

// ============ 错误（§6.3） ============

/// §6.3 失败响应：`{ "error": { "code": …, …, "message": … } }`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorDetail,
}

/// `error.code` = RuleError / Problem 变体名（机器可判）；
/// `detail` 携带变体结构化字段；`message` 面向人类。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub code: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub detail: serde_json::Value,
    pub message: String,
}

/// RuleError → 线上错误明细。
pub fn rule_error_detail(e: &RuleError) -> ErrorDetail {
    let code = rule_error_code(e);
    // RuleError 为外部标签枚举：{"Variant": {..fields..}} → 取内部对象
    let detail = serde_json::to_value(e)
        .ok()
        .and_then(|v| match v {
            serde_json::Value::Object(m) => m.into_iter().next().map(|(_, inner)| inner),
            _ => None,
        })
        .unwrap_or(serde_json::Value::Null);
    ErrorDetail {
        code,
        detail,
        message: rule_error_message(e),
    }
}

fn rule_error_code(e: &RuleError) -> String {
    use RuleError::*;
    match e {
        StaticGateDenied { .. } => "StaticGateDenied",
        GateConditionFailed { .. } => "GateConditionFailed",
        PreconditionMismatch { .. } => "PreconditionMismatch",
        SourceTypeProductionDenied { .. } => "SourceTypeProductionDenied",
        PredecessorNotOnCounter { .. } => "PredecessorNotOnCounter",
        DuplicatePredecessor { .. } => "DuplicatePredecessor",
        DuplicateTokenId { .. } => "DuplicateTokenId",
        TokenNotOnCounter { .. } => "TokenNotOnCounter",
        SeedAuthorityDenied { .. } => "SeedAuthorityDenied",
    }
    .to_owned()
}

fn rule_error_message(e: &RuleError) -> String {
    use RuleError::*;
    match e {
        StaticGateDenied {
            participant,
            action,
            ty,
        } => {
            format!(
                "参与者 {participant} 无 {} {ty} 的 gate 授权",
                action.as_str()
            )
        }
        GateConditionFailed {
            participant,
            action,
            ty,
            expr,
        } => {
            format!(
                "参与者 {participant} 的 {} {ty} gate 条件不满足：{expr:?}",
                action.as_str()
            )
        }
        PreconditionMismatch {
            ty,
            expected,
            provided,
        } => {
            let exp: Vec<String> = expected.iter().map(|t| t.0.clone()).collect();
            let prov: Vec<String> = provided.iter().map(|t| t.0.clone()).collect();
            format!(
                "前驱类型集合不匹配：Precond({ty}) = {{{}}}，实际提供 {{{}}}",
                exp.join(", "),
                prov.join(", ")
            )
        }
        SourceTypeProductionDenied { participant, ty } => {
            format!("{ty} 为源类型，{participant} 只能经 Rule-S（seed）注入")
        }
        PredecessorNotOnCounter { token_id } => {
            format!("前驱 token {token_id} 不在 Counter 上（可能已被消费）")
        }
        DuplicatePredecessor { token_id } => {
            format!("前驱 token {token_id} 重复（Rule-P 要求互异）")
        }
        DuplicateTokenId { token_id } => {
            format!("token id {token_id} 已在该实例中使用（Rule-P/S 要求新 token）")
        }
        TokenNotOnCounter { token_id } => format!("目标 token {token_id} 不在 Counter 上"),
        SeedAuthorityDenied { participant, ty } => {
            format!("({participant}, {ty}) ∉ SeedAuthority：无种子注入权")
        }
    }
}

/// Problem（解析/良构）→ 线上错误明细。
pub fn problem_detail(p: &Problem) -> ErrorDetail {
    match p {
        Problem::Syntax(errs) => ErrorDetail {
            code: "Syntax".to_owned(),
            detail: serde_json::json!({
                "errors": errs.iter().map(|e| format!("{:?}", e)).collect::<Vec<_>>(),
            }),
            message: format!("域声明语法错误（{} 处）", errs.len()),
        },
        Problem::WellFormedness(errs) => ErrorDetail {
            code: "WellFormedness".to_owned(),
            detail: serde_json::to_value(errs).unwrap_or(serde_json::Value::Null),
            message: format!("域声明良构违例（{} 处）", errs.len()),
        },
    }
}

/// 组合冲突 → 线上错误明细。
pub fn conflicts_detail(conflicts: &[pact_compose::Conflict]) -> ErrorDetail {
    ErrorDetail {
        code: "ComposeConflict".to_owned(),
        detail: serde_json::json!({
            "conflicts": conflicts.iter().map(|c| format!("{:?}", c)).collect::<Vec<_>>(),
        }),
        message: format!("片段组合冲突（{} 处）", conflicts.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pact_core::{Action, ParticipantName};

    #[test]
    fn thing_ref_field_distinguishes_missing_null_and_value() {
        let missing: SeedReq =
            serde_json::from_str(r#"{"participant":"dev","type":"requirement"}"#).unwrap();
        assert!(missing.thing_ref.is_missing());
        assert_eq!(missing.thing_ref.as_input(), None);

        let null: SeedReq =
            serde_json::from_str(r#"{"participant":"dev","type":"requirement","thing_ref":null}"#)
                .unwrap();
        assert!(!null.thing_ref.is_missing());
        assert_eq!(null.thing_ref.as_input(), Some(None));

        let value: SeedReq = serde_json::from_str(
            r#"{"participant":"dev","type":"requirement","thing_ref":"https://example.com/a"}"#,
        )
        .unwrap();
        assert_eq!(
            value.thing_ref.as_input(),
            Some(Some("https://example.com/a"))
        );
    }

    #[test]
    fn client_serializes_explicit_null_thing_ref() {
        let req = SeedReq {
            participant: "dev".into(),
            ty: "requirement".into(),
            thing_ref: ThingRefField::explicit(None),
        };
        let value = serde_json::to_value(&req).unwrap();
        assert!(value.get("thing_ref").is_some());
        assert!(value["thing_ref"].is_null());
    }

    #[test]
    fn rule_error_body_shape() {
        let e = RuleError::SeedAuthorityDenied {
            participant: ParticipantName("dev".into()),
            ty: TypeName("requirement".into()),
        };
        let body = ErrorBody {
            error: rule_error_detail(&e),
        };
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["error"]["code"], "SeedAuthorityDenied");
        assert_eq!(v["error"]["detail"]["participant"], "dev");
        assert!(v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("SeedAuthority"));
    }

    #[test]
    fn duplicate_token_id_error_shape() {
        let e = RuleError::DuplicateTokenId {
            token_id: TokenId("seq-0".into()),
        };
        let body = ErrorBody {
            error: rule_error_detail(&e),
        };
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["error"]["code"], "DuplicateTokenId");
        assert_eq!(v["error"]["detail"]["token_id"], "seq-0");
        assert!(v["error"]["message"].as_str().unwrap().contains("seq-0"));
    }

    #[test]
    fn static_gate_denied_message() {
        let e = RuleError::StaticGateDenied {
            participant: ParticipantName("tester".into()),
            action: Action::Produce,
            ty: TypeName("deployment".into()),
        };
        let d = rule_error_detail(&e);
        assert_eq!(d.code, "StaticGateDenied");
        assert!(d.message.contains("produce"));
    }

    #[test]
    fn problem_detail_codes() {
        let d = problem_detail(&Problem::Syntax(vec![]));
        assert_eq!(d.code, "Syntax");
        let d = problem_detail(&Problem::WellFormedness(vec![]));
        assert_eq!(d.code, "WellFormedness");
    }

    #[test]
    fn enabled_option_renames_ty_to_type() {
        let o = EnabledOption {
            ty: TypeName("implementation".into()),
            witness_predecessors: vec![TokenId("01J".into())],
        };
        let v = serde_json::to_value(&o).unwrap();
        assert_eq!(v["type"], "implementation");
        assert!(v.get("ty").is_none());
    }
}

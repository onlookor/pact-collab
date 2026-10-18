//! §6 线上格式 DTO（三接口共享）。
//!
//! 字段名 snake_case；时间戳为 ISO-8601 UTC 字符串（由 core 类型自带语义，
//! 格式由引擎层 Clock 保证）。

use pact_core::{ArchiveEntry, ProductionOption, RuleError, Token, TokenId, TypeName};
use pact_engine::views::GateClause;
use pact_parser::Problem;
use serde::{Deserialize, Serialize};

// ============ 请求 ============

/// `POST /domains`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadDomainReq {
    pub name: String,
    pub declaration: String,
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
    pub thing_ref: Option<String>,
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
    pub thing_ref: Option<String>,
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
    /// 可选幂等键：同域内重复请求返回首次结果，不重复消费。
    #[serde(default)]
    pub request_id: Option<String>,
}

// ============ 响应 ============

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

/// `GET /domains`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainsResp {
    pub domains: Vec<String>,
}

/// `GET /domains/{d}`：域信息 + 状态摘要。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainInfo {
    pub name: String,
    pub counter_size: usize,
    pub archive_size: usize,
    pub terminated: bool,
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

//! SOS 规则（论文 §1.4）：Rule-S / Rule-P / Rule-R / Rule-E。
//! 全部为纯函数：检查以 `&CounterState` 只读完成（零隐式动作原则——
//! 本模块无法修改状态），效果经返回值交由调用方 `CounterState::apply` 应用。

use crate::domain::Domain;
use crate::gate::{gate_holds, GateExpr};
use crate::types::{
    Action, ArchiveEntry, CounterState, GuardedAction, ParticipantName, ThingRef, Timestamp, Token,
    TokenId, TypeName,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// 规则执行失败（结构化；接口层透传为线上 JSON 的 `error.code`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuleError {
    /// 静态 gate 未授权（Rule-P/R/E 的第一条前提）。
    StaticGateDenied {
        participant: ParticipantName,
        action: Action,
        ty: TypeName,
    },
    /// GateHolds 条件子句求值为假（仅 read/consume）。
    GateConditionFailed {
        participant: ParticipantName,
        action: Action,
        ty: TypeName,
        expr: GateExpr,
    },
    /// 提供的前驱类型集合与 Precond(T) 不匹配（数量或类型，§1.4.4 AND 语义）。
    PreconditionMismatch {
        ty: TypeName,
        expected: BTreeSet<TypeName>,
        provided: BTreeSet<TypeName>,
    },
    /// Rule-P 前提 Precond(T) ≠ ∅ 违反：T 是源类型，只能经 Rule-S 注入
    /// （论文 rev1 §1.4；良构域已在声明期拒绝，此处为运行时防御）。
    SourceTypeProductionDenied {
        participant: ParticipantName,
        ty: TypeName,
    },
    /// 前驱 token 不在 Counter 上。
    PredecessorNotOnCounter { token_id: TokenId },
    /// 前驱实例重复（论文 Rule-P：tᵢ ≠ tⱼ）。
    DuplicatePredecessor { token_id: TokenId },
    /// 目标 token 不在 Counter 上。
    TokenNotOnCounter { token_id: TokenId },
    /// (p, T) ∉ SeedAuthority（Rule-S 唯一前提违反）。
    SeedAuthorityDenied {
        participant: ParticipantName,
        ty: TypeName,
    },
}

/// Rule-S（种子注入，§1.4.3）：
/// 仅要求 (p, T) ∈ SeedAuthority；**豁免前置条件**（支持循环拓扑初始化）。
/// 不读 Counter（论文规则前提不含 Counter 条件），depends_on 恒为空（外部来源）。
pub fn rule_s(
    domain: &Domain,
    participant: &ParticipantName,
    ty: &TypeName,
    thing_ref: ThingRef,
    id: TokenId,
    now: Timestamp,
) -> Result<Token, RuleError> {
    if !domain
        .seed_authority
        .contains(&(participant.clone(), ty.clone()))
    {
        return Err(RuleError::SeedAuthorityDenied {
            participant: participant.clone(),
            ty: ty.clone(),
        });
    }
    Ok(Token {
        id,
        ty: ty.clone(),
        thing_ref,
        produced_by: participant.clone(),
        depends_on: Vec::new(),
        produced_at: now,
    })
}

/// Rule-P（生产，论文 rev1 §1.4）。前提逐项：
/// 1. 静态 produce gate：Gate(p) ⊇ {produce: T}；
/// 2. Precond(T) ≠ ∅（源类型排除——源类型只能经 Rule-S 注入，§1.4.3）；
/// 3. 前驱实例互异（tᵢ ≠ tⱼ）；
/// 4. 前驱均在 Counter 上（多重集包含）；
/// 5. 前驱类型集合与 Precond(T) 精确匹配（AND 语义）。
///
/// 前驱**不被消费**：本函数只读 state，Effect::Produced 只添加新 token。
/// `depends_on` 保留调用方给出的前驱顺序（审计溯源记录，§1.4 Note）。
#[allow(clippy::too_many_arguments)]
pub fn rule_p(
    domain: &Domain,
    state: &CounterState,
    participant: &ParticipantName,
    ty: &TypeName,
    predecessors: &[TokenId],
    thing_ref: ThingRef,
    id: TokenId,
    now: Timestamp,
) -> Result<Token, RuleError> {
    if !domain
        .gates
        .contains_key(&(participant.clone(), Action::Produce, ty.clone()))
    {
        return Err(RuleError::StaticGateDenied {
            participant: participant.clone(),
            action: Action::Produce,
            ty: ty.clone(),
        });
    }
    if crate::domain::is_source_type(domain, ty) {
        return Err(RuleError::SourceTypeProductionDenied {
            participant: participant.clone(),
            ty: ty.clone(),
        });
    }
    let mut seen: BTreeSet<&TokenId> = BTreeSet::new();
    for tid in predecessors {
        if !seen.insert(tid) {
            return Err(RuleError::DuplicatePredecessor {
                token_id: tid.clone(),
            });
        }
    }
    for tid in predecessors {
        if !state.counter.contains_key(tid) {
            return Err(RuleError::PredecessorNotOnCounter {
                token_id: tid.clone(),
            });
        }
    }
    let expected: BTreeSet<TypeName> = domain.preconditions.get(ty).cloned().unwrap_or_default();
    let provided: BTreeSet<TypeName> = predecessors
        .iter()
        .map(|tid| state.counter[tid].ty.clone())
        .collect();
    // provided.len() == predecessors.len() 排除"同类型多实例"；
    // 与 expected 的集合相等保证每个前置类型恰有一枚实例。
    if provided.len() != predecessors.len() || provided != expected {
        return Err(RuleError::PreconditionMismatch {
            ty: ty.clone(),
            expected,
            provided,
        });
    }
    Ok(Token {
        id,
        ty: ty.clone(),
        thing_ref,
        produced_by: participant.clone(),
        depends_on: predecessors.to_vec(),
        produced_at: now,
    })
}

/// Rule-R（读取，非破坏性，§1.4）：
/// token ∈ Counter + 静态 read gate + GateHolds；无状态变更，返回 token 克隆。
pub fn rule_r(
    domain: &Domain,
    state: &CounterState,
    participant: &ParticipantName,
    token_id: &TokenId,
) -> Result<Token, RuleError> {
    let token = state
        .counter
        .get(token_id)
        .ok_or_else(|| RuleError::TokenNotOnCounter {
            token_id: token_id.clone(),
        })?;
    check_admission(domain, state, participant, GuardedAction::Read, token)?;
    Ok(token.clone())
}

/// Rule-E（消费，显式移除，§1.4）：
/// 检查同 Rule-R（action=Consume）；效果 = Counter 移除 + Archive 追加。
/// 无级联、无反向传播（零隐式动作原则）。
pub fn rule_e(
    domain: &Domain,
    state: &CounterState,
    participant: &ParticipantName,
    token_id: &TokenId,
    now: Timestamp,
) -> Result<ArchiveEntry, RuleError> {
    let token = state
        .counter
        .get(token_id)
        .ok_or_else(|| RuleError::TokenNotOnCounter {
            token_id: token_id.clone(),
        })?;
    check_admission(domain, state, participant, GuardedAction::Consume, token)?;
    Ok(ArchiveEntry {
        token: token.clone(),
        exhausted_by: participant.clone(),
        exhausted_at: now,
    })
}

/// 静态 gate + GateHolds 检查（Rule-R / Rule-E 共用，§1.4.0）。
fn check_admission(
    domain: &Domain,
    state: &CounterState,
    participant: &ParticipantName,
    action: GuardedAction,
    token: &Token,
) -> Result<(), RuleError> {
    gate_holds(domain, state, participant, action, token).map_err(|f| {
        let action = Action::from(action);
        match f {
            crate::gate::GateFailure::StaticDenied => RuleError::StaticGateDenied {
                participant: participant.clone(),
                action,
                ty: token.ty.clone(),
            },
            crate::gate::GateFailure::ConditionFailed { expr } => RuleError::GateConditionFailed {
                participant: participant.clone(),
                action,
                ty: token.ty.clone(),
                expr,
            },
        }
    })
}

/// 便捷查询：Domain 中某类型的直接前驱集合（无规则 = 空集，源类型）。
pub fn precond_of(domain: &Domain, ty: &TypeName) -> BTreeSet<TypeName> {
    domain.preconditions.get(ty).cloned().unwrap_or_default()
}

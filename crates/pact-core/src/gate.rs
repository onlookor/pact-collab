//! GateExpr 条件语言与 GateHolds 统一准入谓词（论文 §1.4.0、§1.7）。

use crate::domain::Domain;
use crate::types::{Action, CounterState, GuardedAction, ParticipantName, Token, TypeName};
use serde::{Deserialize, Serialize};

/// 可组合条件语言（论文 §1.7.3）。
/// derive PartialEq：片段组合的"语法同一性"判定（§1.8 条件 3）直接复用深度相等。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateExpr {
    /// via T_a：溯源敏感见证检查（实例级，非类型级，§1.4.0）。
    Via(TypeName),
    Exists(TypeName),
    /// count T {>=,=} n。n 受 §1.7.3 PosNat 约束（n ≥ 1）：0 字面量在解析期拒绝，
    /// validate_domain 以 DomainError::ZeroCountLiteral 二次防御（程序化构造路径）。
    Count {
        ty: TypeName,
        op: CountOp,
        n: u64,
    },
    And(Box<GateExpr>, Box<GateExpr>),
    Or(Box<GateExpr>, Box<GateExpr>),
    Not(Box<GateExpr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CountOp {
    /// count T >= n
    Ge,
    /// count T = n
    Eq,
}

impl GateExpr {
    /// 纯函数求值。
    /// §1.7.1 约束经签名强制：搜索空间 = 当前 Counter + 目标 token 的 depends_on，
    /// 无 Archive、无外部状态、无副作用。
    pub fn evaluate(&self, target: &Token, state: &CounterState) -> bool {
        match self {
            // 溯源敏感（§1.4.0 Witness requirement）：
            // 见证 w 须同时在 Counter 上 ∧ w.id ∈ t.depends_on（实例级，非类型级）
            GateExpr::Via(ta) => state
                .counter
                .values()
                .any(|w| w.ty == *ta && target.depends_on.contains(&w.id)),
            GateExpr::Exists(t) => state.counter.values().any(|w| w.ty == *t),
            GateExpr::Count { ty, op, n } => {
                let c = state.counter.values().filter(|w| w.ty == *ty).count() as u64;
                match op {
                    CountOp::Ge => c >= *n,
                    CountOp::Eq => c == *n,
                }
            }
            GateExpr::And(a, b) => a.evaluate(target, state) && b.evaluate(target, state),
            GateExpr::Or(a, b) => a.evaluate(target, state) || b.evaluate(target, state),
            GateExpr::Not(e) => !e.evaluate(target, state),
        }
    }

    /// 表达式中引用的全部类型名（良构校验与静态分析遍历用）。
    pub fn type_refs(&self) -> Vec<&TypeName> {
        match self {
            GateExpr::Via(t) | GateExpr::Exists(t) => vec![t],
            GateExpr::Count { ty, .. } => vec![ty],
            GateExpr::And(a, b) | GateExpr::Or(a, b) => {
                let mut v = a.type_refs();
                v.extend(b.type_refs());
                v
            }
            GateExpr::Not(e) => e.type_refs(),
        }
    }

    /// count 字面量为 0 的原子所引用的类型名（§3.1 约束 5 Positive-Nat
    /// Literal 校验用）。递归覆盖 And/Or/Not 内嵌套的 Count 原子。
    pub fn zero_count_refs(&self) -> Vec<&TypeName> {
        match self {
            GateExpr::Count { ty, n: 0, .. } => vec![ty],
            GateExpr::Via(_) | GateExpr::Exists(_) | GateExpr::Count { .. } => vec![],
            GateExpr::And(a, b) | GateExpr::Or(a, b) => {
                let mut v = a.zero_count_refs();
                v.extend(b.zero_count_refs());
                v
            }
            GateExpr::Not(e) => e.zero_count_refs(),
        }
    }
}

/// 带括号的规范文本形式（可无歧义回读）。
impl std::fmt::Display for GateExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GateExpr::Via(t) => write!(f, "via {t}"),
            GateExpr::Exists(t) => write!(f, "exists {t}"),
            GateExpr::Count {
                ty,
                op: CountOp::Ge,
                n,
            } => write!(f, "count {ty} >= {n}"),
            GateExpr::Count {
                ty,
                op: CountOp::Eq,
                n,
            } => write!(f, "count {ty} = {n}"),
            GateExpr::And(a, b) => write!(f, "({a} and {b})"),
            GateExpr::Or(a, b) => write!(f, "({a} or {b})"),
            GateExpr::Not(e) => write!(f, "not {e}"),
        }
    }
}

/// GateHolds 失败原因（论文 §1.4.0 的两个合取项）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateFailure {
    /// 静态 gate 未授权：(p, action, type(t)) ∉ GateMatrix。
    StaticDenied,
    /// 条件子句求值为假。
    ConditionFailed { expr: GateExpr },
}

/// GateHolds（论文 §1.4.0 统一准入谓词）：
/// 1. 静态 gate：(p, action, type(t)) ∈ GateMatrix；AND
/// 2. 条件子句：若匹配子句携带条件则求值为真；无条件（GateCondOpt = ε）
///    或无匹配子句 → 空真成立。
///
/// 仅作用于 read/consume（经 GuardedAction 类型强制，§1.4.0 Scope）。
pub fn gate_holds(
    domain: &Domain,
    state: &CounterState,
    participant: &ParticipantName,
    action: GuardedAction,
    token: &Token,
) -> Result<(), GateFailure> {
    let key = (participant.clone(), Action::from(action), token.ty.clone());
    match domain.gates.get(&key) {
        None => Err(GateFailure::StaticDenied),
        Some(None) => Ok(()),
        Some(Some(cond)) => {
            if cond.evaluate(token, state) {
                Ok(())
            } else {
                Err(GateFailure::ConditionFailed { expr: cond.clone() })
            }
        }
    }
}

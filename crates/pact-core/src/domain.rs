//! Domain（四个注入点，论文 §2.2）与良构性校验（§1.4.4、§3.1）。

use crate::gate::GateExpr;
use crate::types::{Action, ParticipantName, TypeName};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// 良构性错误（论文 §1.4.4、§3.1 的六项良构约束）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DomainError {
    /// 同一 rhs 出现多条前置规则（§3.1 约束 1 RHS Uniqueness，§1.4.4）。
    DuplicatePrecondRule { ty: TypeName },
    /// 单一前置规则的 lhs 内类型名重复（§3.1 约束 2 LHS Uniqueness，如 `{A + A} → B`）。
    /// 规范表示用 BTreeSet 会静默去重，故本违例只能由 parser 在保留原始序列时报出。
    /// 自环 `{A} → A` 合法（lhs 内 A 只出现一次，§1.4.4 Note）。
    DuplicatePrecondLhs { ty: TypeName, lhs_ty: TypeName },
    /// 前置规则的 lhs 为空（§3.1 约束 3 Non-Empty LHS，如 `{ } → T`）。
    /// 源类型只由"规则缺席"定义，空规则不是表达源类型的合法方式。
    EmptyPrecondLhs { ty: TypeName },
    /// Pre / Gates / GateExpr / SeedAuthority 引用了未声明的类型
    /// （§3.1 约束 4 Type Closure，引用封闭性 §1.8）。
    /// referenced_by 为自有 String：错误可内嵌进 A.5 等 serde 报告。
    UndeclaredType { referenced_by: String, ty: TypeName },
    /// GateExpr 的 count 字面量为 0（§3.1 约束 5 Positive-Nat Literal，§1.7.3）。
    /// `count T = 0` / `count T >= 0` 恒等价于 `not exists T` / true，
    /// 按 §1.7.3 Normalization rule 须拒绝；缺席语义用 `not exists T` 表达。
    ZeroCountLiteral {
        participant: ParticipantName,
        action: Action,
        ty: TypeName,
        count_ty: TypeName,
    },
    /// produces 子句携带条件（违反 §3.1 Note：生产合法性只由前置图决定，§1.5.2）。
    ProduceWithCondition {
        participant: ParticipantName,
        ty: TypeName,
    },
    /// produces 子句指向源类型（§3.1 约束 6 Source-Production Constraint：
    /// 源类型只能经 SeedAuthority / Rule-S 进入；§1.4 Rule-P 前提 Precond(T) ≠ ∅
    /// 的良构性对应面）。A.5 是同一组约束的静态分析表述。
    SourceTypeProduceGate {
        participant: ParticipantName,
        ty: TypeName,
    },
    /// 同一 (p, action, T) 子句重复声明。
    DuplicateGateClause {
        participant: ParticipantName,
        action: Action,
        ty: TypeName,
    },
}

/// PACT 域实例：四个注入点的规范表示（论文 §2.2、§3.1）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Domain {
    pub types: BTreeSet<TypeName>,
    /// rhs -> 前驱集合（AND 语义）。
    /// §1.4.4 RHS Uniqueness 在表示层面结构化保证：键唯一；
    /// LHS Uniqueness 由 BTreeSet 去重结构化保证（违例在 parser 报
    /// DomainError::DuplicatePrecondLhs，见该变体注释）。
    pub preconditions: BTreeMap<TypeName, BTreeSet<TypeName>>,
    /// (p, action, T) -> GateCondOpt（Some = via/when 条件）。
    /// 子句唯一性同样由键唯一结构化保证。
    pub gates: BTreeMap<(ParticipantName, Action, TypeName), Option<GateExpr>>,
    pub seed_authority: BTreeSet<(ParticipantName, TypeName)>,
}

/// 源类型判定（论文 §1.4.3）：Precond(T) = ∅ 即源类型。
/// Precond(T) 定义为 `{ X : ∃R ∈ Pre, rhs(R) = T ∧ X ∈ lhs(R) }`，故"规则缺席"
/// 与"lhs 为空"都给出 ∅；后者是 §3.1 约束 3 的违例，由 validate_domain 拒绝。
/// 源类型只能经 SeedAuthority / Rule-S 注入（Rule-P 前提 Precond(T) ≠ ∅ 排除）。
pub fn is_source_type(domain: &Domain, ty: &TypeName) -> bool {
    domain.preconditions.get(ty).is_none_or(|s| s.is_empty())
}

/// 对规范表示做结构校验：§3.1 六项良构约束中可由此表示承载的五项
/// （Non-Empty LHS、Type Closure、Positive-Nat Literal、Source-Production
/// Constraint，以及 produces 无条件）+ 子句唯一性。
/// RHS Uniqueness / LHS Uniqueness 由 BTreeMap 键与 BTreeSet 的表示层结构化保证。
/// 返回全部错误（非首错即停）。
pub fn validate_domain(domain: &Domain) -> Result<(), Vec<DomainError>> {
    let mut errs = Vec::new();
    let declared = &domain.types;

    for (rhs, preds) in &domain.preconditions {
        check_type(declared, &mut errs, "preconditions", rhs);
        // §3.1 约束 3 Non-Empty LHS：显式空规则非法（源类型只由规则缺席定义）
        if preds.is_empty() {
            errs.push(DomainError::EmptyPrecondLhs { ty: rhs.clone() });
        }
        for p in preds {
            check_type(declared, &mut errs, "preconditions", p);
        }
    }
    for ((participant, action, ty), cond) in &domain.gates {
        check_type(declared, &mut errs, "gates", ty);
        if *action == Action::Produce {
            if cond.is_some() {
                errs.push(DomainError::ProduceWithCondition {
                    participant: participant.clone(),
                    ty: ty.clone(),
                });
            }
            if is_source_type(domain, ty) {
                errs.push(DomainError::SourceTypeProduceGate {
                    participant: participant.clone(),
                    ty: ty.clone(),
                });
            }
        }
        if let Some(expr) = cond {
            for t in expr.type_refs() {
                check_type(declared, &mut errs, "gate_condition", t);
            }
            // §3.1 约束 5 Positive-Nat Literal：count 字面量不得为 0
            for count_ty in expr.zero_count_refs() {
                errs.push(DomainError::ZeroCountLiteral {
                    participant: participant.clone(),
                    action: *action,
                    ty: ty.clone(),
                    count_ty: count_ty.clone(),
                });
            }
        }
    }
    for (_, ty) in &domain.seed_authority {
        check_type(declared, &mut errs, "seed_authority", ty);
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(errs)
    }
}

fn check_type(
    declared: &BTreeSet<TypeName>,
    errs: &mut Vec<DomainError>,
    referenced_by: &str,
    ty: &TypeName,
) {
    if !declared.contains(ty) {
        errs.push(DomainError::UndeclaredType {
            referenced_by: referenced_by.to_owned(),
            ty: ty.clone(),
        });
    }
}

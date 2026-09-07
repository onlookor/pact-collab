//! Domain（四个注入点，论文 §2.2）与良构性校验（§1.4.4、§3.1）。

use crate::gate::GateExpr;
use crate::types::{Action, ParticipantName, TypeName};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// 良构性错误。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DomainError {
    /// 同一 rhs 出现多条前置规则（违反 §1.4.4 前置唯一性）。
    DuplicatePrecondRule { ty: TypeName },
    /// Pre / Gates / GateExpr / SeedAuthority 引用了未声明的类型（引用封闭性，§1.8）。
    UndeclaredType {
        referenced_by: &'static str,
        ty: TypeName,
    },
    /// produces 子句携带条件（违反 §3.1 Note：生产合法性只由前置图决定，§1.5.2）。
    ProduceWithCondition {
        participant: ParticipantName,
        ty: TypeName,
    },
    /// produces 子句指向源类型（§3.1 WF-3 源生产约束：源类型只能经
    /// SeedAuthority / Rule-S 进入；论文 rev1 §1.4 Rule-P 前提 Precond(T) ≠ ∅
    /// 的良构性对应面）。A.5 是同一不变量的静态分析表述。
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
    /// §1.4.4 前置唯一性在表示层面结构化保证：键唯一；
    /// 违例由 parser / compose 构造 Domain 时以 DomainError::DuplicatePrecondRule 报告。
    pub preconditions: BTreeMap<TypeName, BTreeSet<TypeName>>,
    /// (p, action, T) -> GateCondOpt（Some = via/when 条件）。
    /// 子句唯一性同样由键唯一结构化保证。
    pub gates: BTreeMap<(ParticipantName, Action, TypeName), Option<GateExpr>>,
    pub seed_authority: BTreeSet<(ParticipantName, TypeName)>,
}

/// 源类型判定（论文 rev1 §1.4.3）：T 无前置规则（Precond(T) = ∅）即源类型。
/// 源类型只能经 SeedAuthority / Rule-S 注入（Rule-P 前提 Precond(T) ≠ ∅ 排除）。
pub fn is_source_type(domain: &Domain, ty: &TypeName) -> bool {
    domain.preconditions.get(ty).is_none_or(|s| s.is_empty())
}

/// 对规范表示做结构校验：引用封闭性 + produces 不携带条件 +
/// 源生产约束（§3.1 WF-3）。返回全部错误（非首错即停）。
pub fn validate_domain(domain: &Domain) -> Result<(), Vec<DomainError>> {
    let mut errs = Vec::new();
    let declared = &domain.types;

    for (rhs, preds) in &domain.preconditions {
        check_type(declared, &mut errs, "preconditions", rhs);
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
    referenced_by: &'static str,
    ty: &TypeName,
) {
    if !declared.contains(ty) {
        errs.push(DomainError::UndeclaredType {
            referenced_by,
            ty: ty.clone(),
        });
    }
}

//! RawDeclaration -> Domain 的规范化（良构判定在 pact-core，D14）。

use crate::parser::{parse_raw, Problem, RawDeclaration};
use pact_core::{validate_domain, Action, Domain, DomainError, ParticipantName, TypeName};
use std::collections::{BTreeMap, BTreeSet};

/// 规范化 + 良构校验。收集全部错误（§1.4.4 前置唯一性、子句唯一性、
/// produces 无条件、引用封闭性）。
pub fn build_domain(raw: &RawDeclaration) -> Result<Domain, Vec<DomainError>> {
    let mut errors = Vec::new();

    let types: BTreeSet<TypeName> = raw.types.iter().cloned().collect();

    // 前置规则：按 rhs 分组；同一 rhs 多条规则 → §1.4.4 违例
    let mut preconditions: BTreeMap<TypeName, BTreeSet<TypeName>> = BTreeMap::new();
    let mut seen_rhs: BTreeSet<TypeName> = BTreeSet::new();
    for (lhs, rhs) in &raw.precond_rules {
        if !seen_rhs.insert(rhs.clone()) {
            errors.push(DomainError::DuplicatePrecondRule { ty: rhs.clone() });
            continue;
        }
        preconditions.insert(rhs.clone(), lhs.iter().cloned().collect());
    }

    // gate 子句：同一参与者可多次声明（合并，§1.8 组合语义），但子句不得重复
    let mut gates: BTreeMap<(ParticipantName, Action, TypeName), Option<pact_core::GateExpr>> =
        BTreeMap::new();
    for (participant, clauses) in &raw.gate_defs {
        for c in clauses {
            let key = (participant.clone(), c.action, c.ty.clone());
            if gates.contains_key(&key) {
                errors.push(DomainError::DuplicateGateClause {
                    participant: participant.clone(),
                    action: c.action,
                    ty: c.ty.clone(),
                });
                continue;
            }
            // produces 携带条件的判定由 validate_domain 统一报告（cond 保留入表）
            gates.insert(key, c.cond.clone());
        }
    }

    let seed_authority: BTreeSet<(ParticipantName, TypeName)> =
        raw.seed_authority.iter().cloned().collect();

    let domain = Domain {
        types,
        preconditions,
        gates,
        seed_authority,
    };
    if let Err(mut es) = validate_domain(&domain) {
        errors.append(&mut es);
    }
    if errors.is_empty() {
        Ok(domain)
    } else {
        Err(errors)
    }
}

/// 一步到位：声明文本 -> 良构 Domain。
/// 语法错误（词法全收、语法首错即停）或良构错误（全收）。
pub fn load_domain(src: &str) -> Result<Domain, Problem> {
    let raw = parse_raw(src).map_err(Problem::Syntax)?;
    build_domain(&raw).map_err(Problem::WellFormedness)
}

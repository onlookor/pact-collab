//! PACT 片段并行组合（论文 rev1 §1.8）。
//!
//! 并行组合 D₁ ∥ D₂ = 四元组（Types, Pre, Gates, Seeds）按集合并。
//! 组合在**一致性四条件**下保持良构（论文 §1.8 定理）：
//! - C1 前置唯一性保持：合并后每个 rhs 至多一条前置规则；
//! - C2 共享类型前置一致：双方都为 T 声明前置规则时必须相同；
//! - C3 gate 条件语法同一：(p, action, T) 共享子句的 GateCond 必须相同；
//! - C4 源生产封闭：合并后无 `produces T` 指向合并 Types 下的源类型。
//!
//! **C2 实践语义（D18）**：单侧声明、单侧缺席视为"缺席方对 T 的前置无
//! 约束"，合并取并合法——这是论文 §1.8 示例的规范语义（D₂ 共享 design
//! 而不重复声明其前置，顺序组合正依赖此机制）；仅双侧规则**都存在且不同**
//! 时报冲突。C4 在此语义下仍安全：`produces T` 只可能来自视 T 为非源类型
//! 的良构片段（该片段必有 T 的规则），规则并集只会增不会减，合并后 T
//! 必非源。C1 的跨域情形（同 rhs 双规则且前驱不同）为 C2 冲突特例；
//! C4 经 `pact_core::validate_domain` 终检兜底，同时捕获片段自带的良构违例。
//!
//! 全部为纯函数；冲突全部收集（非首错即停）。

use pact_core::{
    validate_domain, Action, Domain, DomainError, GateExpr, ParticipantName, TypeName,
};
use std::collections::{BTreeMap, BTreeSet};

/// 组合冲突（§1.8 条件 1–4 的结构化报告）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    /// C1+C2：双方都为共享类型（或同一 rhs）声明了前置规则且集合不同。
    /// 单侧声明不构成冲突（D18：缺席方无约束，合并取并）。
    PreconditionMismatch {
        ty: TypeName,
        pre_a: BTreeSet<TypeName>,
        pre_b: BTreeSet<TypeName>,
    },
    /// C3：共享 (p, action, T) 子句的 GateCond 非语法同一
    ///（`Option<GateExpr>` 深度相等；None 与 None 同一）。
    GateConditionMismatch {
        participant: ParticipantName,
        action: Action,
        ty: TypeName,
        cond_a: Option<GateExpr>,
        cond_b: Option<GateExpr>,
    },
    /// C4 / 良构终检：合并域违反 §3.1 WF-1–WF-3
    ///（如片段自带的源类型 produce gate、引用封闭性破坏等）。
    IllFormed(DomainError),
}

/// 多片段并行组合：从空域（组合单位元）折叠二元组合链。
/// 任一步冲突即返回该步的全部冲突明细。
pub fn compose(fragments: &[Domain]) -> Result<Domain, Vec<Conflict>> {
    let mut acc = Domain {
        types: BTreeSet::new(),
        preconditions: BTreeMap::new(),
        gates: BTreeMap::new(),
        seed_authority: BTreeSet::new(),
    };
    for frag in fragments {
        acc = compose_pair(&acc, frag)?;
    }
    Ok(acc)
}

/// 二元并行组合 D₁ ∥ D₂（论文 §1.8）。
/// 一致性条件满足 → 返回集合并后的新 Domain（经良构终检）；
/// 否则返回全部冲突明细。
pub fn compose_pair(a: &Domain, b: &Domain) -> Result<Domain, Vec<Conflict>> {
    let mut conflicts = Vec::new();

    // C1+C2：双方都为共享类型（或同一 rhs）声明前置规则且集合不同 → 冲突；
    // 单侧声明合法（D18：缺席方无约束，合并取并——论文 §1.8 示例语义）
    for ty in a.types.iter().filter(|t| b.types.contains(*t)) {
        if let (Some(pa), Some(pb)) = (a.preconditions.get(ty), b.preconditions.get(ty)) {
            if pa != pb {
                conflicts.push(Conflict::PreconditionMismatch {
                    ty: ty.clone(),
                    pre_a: pa.clone(),
                    pre_b: pb.clone(),
                });
            }
        }
    }

    // C3：共享 (p, action, T) 子句的 GateCond 语法同一
    for (key, cond_a) in &a.gates {
        if let Some(cond_b) = b.gates.get(key) {
            if cond_a != cond_b {
                conflicts.push(Conflict::GateConditionMismatch {
                    participant: key.0.clone(),
                    action: key.1,
                    ty: key.2.clone(),
                    cond_a: cond_a.clone(),
                    cond_b: cond_b.clone(),
                });
            }
        }
    }

    if !conflicts.is_empty() {
        return Err(conflicts);
    }

    // 四元组按集合并（共享键的值已验证一致，保留先入值）
    let mut merged = a.clone();
    merged.types.extend(b.types.iter().cloned());
    for (k, v) in &b.preconditions {
        merged
            .preconditions
            .entry(k.clone())
            .or_insert_with(|| v.clone());
    }
    for (k, v) in &b.gates {
        merged.gates.entry(k.clone()).or_insert_with(|| v.clone());
    }
    merged
        .seed_authority
        .extend(b.seed_authority.iter().cloned());

    // C4 + 良构终检（捕获片段自带违例与合并引入的任何 §3.1 破坏）
    match validate_domain(&merged) {
        Ok(()) => Ok(merged),
        Err(es) => Err(es.into_iter().map(Conflict::IllFormed).collect()),
    }
}

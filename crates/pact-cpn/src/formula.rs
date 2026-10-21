//! GateExpr → 否定范式（NNF）→ 析取范式（DNF）。
//!
//! CPN 的条件编码以「每条迁移一个合取项」为基本单位（论文 §4.5/§B.3）：
//! `or` 必须展开为多条迁移，`not` 必须下推到原子条件（互补库所编码）。
//! 本模块只做纯语法变换与项化简，不涉及 CPN 结构。

use crate::CpnError;
use pact_core::{CountOp, GateExpr, TypeName};
use std::collections::BTreeMap;

/// 原子条件：正条件（存在/计数/溯源见证）与上界条件（互补库所编码）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Atom {
    Exists(TypeName),
    CountGe(TypeName, u64),
    /// `count T <= k`（由 `not` 下推或 `count T = n` 拆分得到）。
    AtMost(TypeName, u64),
    Via(TypeName),
}

#[derive(Debug, Clone)]
enum Nf {
    True,
    False,
    Atom(Atom),
    And(Box<Nf>, Box<Nf>),
    Or(Box<Nf>, Box<Nf>),
}

/// 一个 DNF 合取项的正条件汇总。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PosReq {
    /// `count T >= need`（`Exists`/`Via` 至少各要求 1）。
    pub need: u64,
    pub exists: bool,
    pub via: bool,
}

/// 一个已化简的 DNF 合取项。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Term {
    pub positive: BTreeMap<TypeName, PosReq>,
    /// `count T <= k`，仅在互补库所编码需要时保留（已被上界化简的项不在此表）。
    pub at_most: BTreeMap<TypeName, u64>,
}

/// NNF 变换。`negated` 表示当前位于奇数个 `not` 之下。
fn nnf(expr: &GateExpr, negated: bool) -> Result<Nf, CpnError> {
    match expr {
        GateExpr::Via(t) => {
            if negated {
                Err(CpnError::NegatedVia { ty: t.clone() })
            } else {
                Ok(Nf::Atom(Atom::Via(t.clone())))
            }
        }
        GateExpr::Exists(t) => Ok(Nf::Atom(if negated {
            Atom::AtMost(t.clone(), 0)
        } else {
            Atom::Exists(t.clone())
        })),
        GateExpr::Count { ty, op, n } => match op {
            CountOp::Ge => {
                if *n == 0 {
                    // 良构域禁止 0 字面量（§3.1 约束 5）；程序化构造路径按恒真处理。
                    Ok(if negated { Nf::False } else { Nf::True })
                } else if negated {
                    Ok(Nf::Atom(Atom::AtMost(ty.clone(), n - 1)))
                } else {
                    Ok(Nf::Atom(Atom::CountGe(ty.clone(), *n)))
                }
            }
            CountOp::Eq => {
                // count = n  ⇔  count >= n ∧ count <= n
                let ge = if *n == 0 {
                    Nf::True
                } else {
                    Nf::Atom(Atom::CountGe(ty.clone(), *n))
                };
                let le = if *n == 0 {
                    Nf::Atom(Atom::AtMost(ty.clone(), 0))
                } else {
                    Nf::Atom(Atom::AtMost(ty.clone(), *n))
                };
                let positive = Nf::And(Box::new(ge), Box::new(le));
                if !negated {
                    Ok(positive)
                } else {
                    // ¬(count = n) ⇔ count < n ∨ count > n
                    let lt = if *n == 0 {
                        Nf::False
                    } else {
                        Nf::Atom(Atom::AtMost(ty.clone(), n - 1))
                    };
                    let gt = match n.checked_add(1) {
                        Some(next) => Nf::Atom(Atom::CountGe(ty.clone(), next)),
                        None => {
                            return Err(CpnError::ConditionTooComplex {
                                reason: "count 字面量超出 u64 上界，无法构造 count > n".into(),
                            })
                        }
                    };
                    Ok(Nf::Or(Box::new(lt), Box::new(gt)))
                }
            }
        },
        GateExpr::And(a, b) => {
            if negated {
                Ok(Nf::Or(Box::new(nnf(a, true)?), Box::new(nnf(b, true)?)))
            } else {
                Ok(Nf::And(Box::new(nnf(a, false)?), Box::new(nnf(b, false)?)))
            }
        }
        GateExpr::Or(a, b) => {
            if negated {
                Ok(Nf::And(Box::new(nnf(a, true)?), Box::new(nnf(b, true)?)))
            } else {
                Ok(Nf::Or(Box::new(nnf(a, false)?), Box::new(nnf(b, false)?)))
            }
        }
        GateExpr::Not(e) => nnf(e, !negated),
    }
}

const MAX_DNF_TERMS: usize = 1024;
/// `count T >= n` 会展开为 n 条 test arc；限制展开规模以避免恶意/误用声明
/// 造成生成器内存与输出膨胀。
const MAX_COUNT_LITERAL: u64 = 1024;

/// NNF → DNF 合取项列表。
fn dnf(nf: &Nf) -> Result<Vec<Vec<Atom>>, CpnError> {
    let out = match nf {
        Nf::True => vec![Vec::new()],
        Nf::False => Vec::new(),
        Nf::Atom(a) => vec![vec![a.clone()]],
        Nf::Or(a, b) => {
            let mut v = dnf(a)?;
            v.extend(dnf(b)?);
            v
        }
        Nf::And(a, b) => {
            let left = dnf(a)?;
            let right = dnf(b)?;
            if left.len().saturating_mul(right.len()) > MAX_DNF_TERMS {
                return Err(CpnError::ConditionTooComplex {
                    reason: format!(
                        "条件展开为 {} 个合取项，超过上限 {MAX_DNF_TERMS}",
                        left.len().saturating_mul(right.len())
                    ),
                });
            }
            let mut v = Vec::with_capacity(left.len() * right.len());
            for l in &left {
                for r in &right {
                    let mut term = l.clone();
                    term.extend(r.iter().cloned());
                    v.push(term);
                }
            }
            v
        }
    };
    if out.len() > MAX_DNF_TERMS {
        return Err(CpnError::ConditionTooComplex {
            reason: format!(
                "条件展开为 {} 个合取项，超过上限 {MAX_DNF_TERMS}",
                out.len()
            ),
        });
    }
    Ok(out)
}

/// 把一个 GateExpr 化简为 DNF 合取项列表。
///
/// 空列表表示条件恒假（所有合取项均被矛盾消除）。
/// `bounds` 为互补库所编码所需的类型上界；仅当出现 `count T <= k`
/// 且 `k < 上界` 时才需要该类型的上界。
pub(crate) fn terms(
    expr: &GateExpr,
    bounds: &BTreeMap<TypeName, u64>,
) -> Result<Vec<Term>, CpnError> {
    let nf = nnf(expr, false)?;
    let raw_terms = dnf(&nf)?;
    let mut out = Vec::with_capacity(raw_terms.len());
    for atoms in raw_terms {
        if let Some(term) = simplify(&atoms, bounds)? {
            out.push(term);
        }
    }
    Ok(out)
}

/// 合并同一合取项内的原子并消除矛盾/恒真原子。
fn simplify(atoms: &[Atom], bounds: &BTreeMap<TypeName, u64>) -> Result<Option<Term>, CpnError> {
    let mut term = Term::default();
    for atom in atoms {
        match atom {
            Atom::Exists(t) => {
                let req = term.positive.entry(t.clone()).or_default();
                req.exists = true;
                req.need = req.need.max(1);
            }
            Atom::CountGe(t, n) => {
                if *n > MAX_COUNT_LITERAL {
                    return Err(CpnError::ConditionTooComplex {
                        reason: format!("count {} >= {} 超过展开上限 {MAX_COUNT_LITERAL}", t.0, n),
                    });
                }
                let req = term.positive.entry(t.clone()).or_default();
                req.need = req.need.max(*n);
            }
            Atom::Via(t) => {
                let req = term.positive.entry(t.clone()).or_default();
                req.via = true;
                req.need = req.need.max(1);
            }
            Atom::AtMost(t, k) => {
                let bound = bounds
                    .get(t)
                    .copied()
                    .ok_or_else(|| CpnError::MissingBound { ty: t.clone() })?;
                // count T <= k 恒真 ⇔ k >= 已知上界；无需互补库所，也无需约束。
                if *k >= bound {
                    continue;
                }
                let cur = term.at_most.get(t).copied().unwrap_or(u64::MAX);
                term.at_most.insert(t.clone(), cur.min(*k));
            }
        }
    }

    // 矛盾消除：正条件需求量超过上界，或 via 要求见证而 T 必须为空。
    for (t, req) in &term.positive {
        if let Some(k) = term.at_most.get(t) {
            if req.need > *k {
                return Ok(None);
            }
            if req.via && *k == 0 {
                return Ok(None);
            }
        }
        if let Some(bound) = bounds.get(t) {
            if req.need > *bound {
                return Ok(None);
            }
        }
    }
    Ok(Some(term))
}

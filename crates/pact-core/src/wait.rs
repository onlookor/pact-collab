//! Rule-W 服务与 Rule-T 终止判定（论文 rev1 §1.4、§1.6）。

use crate::domain::Domain;
use crate::types::{Action, CounterState, ParticipantName, TokenId, TypeName};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// 参与者的一项可执行生产选项（Rule-W 轮询服务 `enabled` 的返回项）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductionOption {
    pub ty: TypeName,
    /// 一组满足前置的见证前驱实例（贪心选取，不枚举全部组合）。
    pub witness_predecessors: Vec<TokenId>,
}

/// 参与者当前可满足的全部 Rule-P 组合。
/// 源类型（Precond = ∅）被 Rule-P 前提排除（论文 rev1 §1.4：Precond(T) ≠ ∅），
/// 不出现在生产选项中——源类型只能经 Rule-S 注入。
/// 输出按 (action, 类型名) 序确定排序（gates 为 BTreeMap）。
pub fn enabled_productions(
    domain: &Domain,
    state: &CounterState,
    participant: &ParticipantName,
) -> Vec<ProductionOption> {
    let mut out = Vec::new();
    for (p, action, ty) in domain.gates.keys() {
        if *p != *participant || *action != Action::Produce {
            continue;
        }
        // Rule-P 前提 Precond(T) ≠ ∅：源类型跳过
        let expected: BTreeSet<&TypeName> = domain
            .preconditions
            .get(ty)
            .map(|s| s.iter().collect())
            .unwrap_or_default();
        if expected.is_empty() {
            continue;
        }
        let mut witnesses = Vec::with_capacity(expected.len());
        let mut ok = true;
        for pred_ty in &expected {
            // 同一前驱类型在 expected 中至多出现一次（集合），见证必然互异
            match state.counter.values().find(|w| w.ty == **pred_ty) {
                Some(w) => witnesses.push(w.id.clone()),
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            out.push(ProductionOption {
                ty: ty.clone(),
                witness_predecessors: witnesses,
            });
        }
    }
    out
}

/// Rule-T 判定（论文 rev1 §1.4）：
/// Counter = ∅ 且 ¬∃ p ∈ Participant: p is Rule-P-enabled。
///
/// Rule-P-enabled（论文 rev1 定义）：∃ T ∈ TypeSchema 使
/// (1) Precond(T) ≠ ∅（源类型排除）；(2) Gate(p) ⊇ {produce: T}；
/// (3) T 的全部前驱类型在 Counter 上各有一枚满足匹配条件的实例。
/// Counter = ∅ 时条件 (3) 对所有 Precond(T) ≠ ∅ 的类型必然失败
/// （终止论证，论文 rev1 §1.4 Rule-T Termination argument），
/// 故本判定在良构域上等价于 Counter 为空；保留显式扫描以承载语义
/// 并在前提变化时保持正确。
/// 在 Counter 上保留见证、ack 等 token 会阻止 Rule-T；Archive 中的 token
/// 不计入。消费顺序见 `docs/token-consumption-timing.md` TC13。
pub fn termination_holds(domain: &Domain, state: &CounterState) -> bool {
    if !state.counter.is_empty() {
        return false;
    }
    !domain.gates.keys().any(|(_, action, ty)| {
        *action == Action::Produce && rule_p_enabled_for_type(domain, state, ty)
    })
}

/// 论文 rev1 Rule-P-enabled 的类型级判定（三条件）。
fn rule_p_enabled_for_type(domain: &Domain, state: &CounterState, ty: &TypeName) -> bool {
    match domain.preconditions.get(ty) {
        None => false, // (1) 源类型：Precond(T) = ∅，Rule-P 排除
        Some(preds) => {
            // (3) 全部前驱类型在 Counter 上各有一枚实例（Counter = ∅ 时空失败）
            preds
                .iter()
                .all(|pt| state.counter.values().any(|w| w.ty == *pt))
        }
    }
}

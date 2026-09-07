//! 属性测试（FR13 / design.md §10）：以随机动作序列锁定核心不变量。
//!
//! 不变量清单（论文语义 + 零隐式动作原则）：
//! I1 规则函数永不修改状态（Err 路径状态逐字节不变；Ok 路径由 Effect 单点应用）
//! I2 Rule-R 非破坏性（成功读取后 Counter 逐字节不变）
//! I3 Rule-P 前驱不被消费（成功生产后前驱全部仍在 Counter）
//! I4 Rule-E 显式且无级联（成功消费只移除目标 token，其余 token 逐字节不变）
//! I5 状态变更唯一路径：任意序列重放（同 Effect 序列）结果逐字节一致
//! I6 GateExpr 求值确定性（同输入两次求值同结果）

use pact_core::*;
use proptest::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

fn tyn(s: &str) -> TypeName {
    TypeName(s.into())
}
fn par(s: &str) -> ParticipantName {
    ParticipantName(s.into())
}
fn tid(s: &str) -> TokenId {
    TokenId(s.into())
}

/// 宽容测试域：4 类型、2 参与者、全 read/consume 开放、非源类型全 produce 开放
/// （T0 为源类型：Rule-P 前提 Precond ≠ ∅ 排除其生产，论文 rev1 §1.4）、
/// 部分种子权威。前置图：T0→T1，T1→T2，{T2+T0}→T3。
fn permissive_domain() -> Domain {
    let types: BTreeSet<TypeName> = ["T0", "T1", "T2", "T3"].iter().map(|s| tyn(s)).collect();
    let mut preconditions = BTreeMap::new();
    preconditions.insert(tyn("T1"), BTreeSet::from([tyn("T0")]));
    preconditions.insert(tyn("T2"), BTreeSet::from([tyn("T1")]));
    preconditions.insert(tyn("T3"), BTreeSet::from([tyn("T2"), tyn("T0")]));
    let mut gates = BTreeMap::new();
    for p in ["p0", "p1"] {
        for t in ["T1", "T2", "T3"] {
            gates.insert((par(p), Action::Produce, tyn(t)), None);
        }
        for t in ["T0", "T1", "T2", "T3"] {
            gates.insert((par(p), Action::Read, tyn(t)), None);
            gates.insert((par(p), Action::Consume, tyn(t)), None);
        }
    }
    // 个别条件 gate：不影响不变量，但让求值路径被覆盖
    gates.insert(
        (par("p1"), Action::Read, tyn("T3")),
        Some(GateExpr::Via(tyn("T2"))),
    );
    gates.insert(
        (par("p1"), Action::Consume, tyn("T2")),
        Some(GateExpr::Count {
            ty: tyn("T0"),
            op: CountOp::Ge,
            n: 1,
        }),
    );
    Domain {
        types,
        preconditions,
        gates,
        seed_authority: BTreeSet::from([(par("p0"), tyn("T0")), (par("p0"), tyn("T1"))]),
    }
}

/// 随机动作。
#[derive(Debug, Clone)]
enum Op {
    Seed { p: u8, t: u8 },
    Produce { p: u8, t: u8, preds: Vec<u16> },
    Read { p: u8, idx: u16 },
    Consume { p: u8, idx: u16 },
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        (any::<u8>(), any::<u8>()).prop_map(|(p, t)| Op::Seed { p, t }),
        (
            any::<u8>(),
            any::<u8>(),
            prop::collection::vec(any::<u16>(), 0..3),
        )
            .prop_map(|(p, t, preds)| Op::Produce { p, t, preds }),
        (any::<u8>(), any::<u16>()).prop_map(|(p, idx)| Op::Read { p, idx }),
        (any::<u8>(), any::<u16>()).prop_map(|(p, idx)| Op::Consume { p, idx }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// I1–I5：随机动作序列下全部核心不变量保持。
    #[test]
    fn invariants_hold_under_random_ops(ops in prop::collection::vec(op_strategy(), 4..48)) {
        let domain = permissive_domain();
        let participants = [par("p0"), par("p1")];
        let types = [tyn("T0"), tyn("T1"), tyn("T2"), tyn("T3")];
        let mut state = CounterState::new();
        let mut effects: Vec<Effect> = Vec::new();
        let mut next_id: u64 = 0;
        let mut tick: u64 = 0;

        // I5 重放基准：把 (Effect 序列) 在新状态上重放，应与在线状态一致
        let mut replay = CounterState::new();

        let ids: Vec<TokenId> = Vec::new(); // 当前 Counter 的 id 池（按序）
        let mut ids = ids;

        for op in &ops {
            tick += 1;
            let now = format!("2026-01-01T00:00:{tick:04}Z");
            match op {
                Op::Seed { p, t } => {
                    let p = &participants[(*p as usize) % 2];
                    let ty = &types[(*t as usize) % 4];
                    let id = { next_id += 1; tid(&format!("tok-{next_id:04}")) };
                    let before = state.clone();
                    match rule_s(&domain, p, ty, ThingRef::none(), id.clone(), now) {
                        Ok(tk) => {
                            let e = Effect::Seeded(tk);
                            state.apply(e.clone());
                            effects.push(e);
                            ids.push(id);
                        }
                        Err(_) => assert_eq!(state, before, "I1：Err 路径状态不得改变"),
                    }
                }
                Op::Produce { p, t, preds } => {
                    let p = &participants[(*p as usize) % 2];
                    let ty = &types[(*t as usize) % 4];
                    let chosen: Vec<TokenId> = if ids.is_empty() {
                        Vec::new()
                    } else {
                        preds.iter()
                            .map(|i| ids[(*i as usize) % ids.len()].clone())
                            .collect()
                    };
                    let id = { next_id += 1; tid(&format!("tok-{next_id:04}")) };
                    let before = state.clone();
                    match rule_p(&domain, &state, p, ty, &chosen, ThingRef::none(), id.clone(), now) {
                        Ok(tk) => {
                            // I3：前驱在生产后仍在 Counter
                            for d in &tk.depends_on {
                                assert!(state.counter.contains_key(d), "I3：前驱被隐式消费：{d}");
                            }
                            let e = Effect::Produced(tk);
                            state.apply(e.clone());
                            effects.push(e);
                            ids.push(id);
                        }
                        Err(_) => assert_eq!(state, before, "I1：Err 路径状态不得改变"),
                    }
                }
                Op::Read { p, idx } => {
                    let p = &participants[(*p as usize) % 2];
                    let id = ids.get((*idx as usize) % ids.len().max(1)).cloned();
                    if let Some(id) = id {
                        let before = state.clone();
                        match rule_r(&domain, &state, p, &id) {
                            Ok(tk) => {
                                // I2：读取非破坏性
                                assert_eq!(state.counter.get(&id), Some(&tk), "I2：读取须返回 Counter 中的 token");
                                assert_eq!(state, before, "I2：读取不得改变状态");
                            }
                            Err(_) => assert_eq!(state, before, "I1：Err 路径状态不得改变"),
                        }
                    }
                }
                Op::Consume { p, idx } => {
                    let p = &participants[(*p as usize) % 2];
                    let id = ids.get((*idx as usize) % ids.len().max(1)).cloned();
                    let Some(id) = id else { continue };
                    let before = state.clone();
                    match rule_e(&domain, &state, p, &id, now) {
                        Ok(entry) => {
                            // I4：只移除目标 token，其余逐字节不变
                            let others_before: BTreeMap<_, _> =
                                before.counter.iter().filter(|(k, _)| **k != id).collect();
                            let e = Effect::Consumed(entry);
                            state.apply(e.clone());
                            effects.push(e);
                            let others_after: BTreeMap<_, _> =
                                state.counter.iter().collect();
                            assert_eq!(others_before, others_after, "I4：消费不得级联影响其他 token");
                            assert!(!state.counter.contains_key(&id));
                            assert_eq!(state.archive.len(), before.archive.len() + 1);
                        }
                        Err(_) => assert_eq!(state, before, "I1：Err 路径状态不得改变"),
                    }
                }
            }
            ids.retain(|i| state.counter.contains_key(i));
        }

        // I5：重放等价
        for e in &effects {
            replay.apply(e.clone());
        }
        assert_eq!(state, replay, "I5：在线状态与 Effect 重放不一致");
    }

    /// I6：GateExpr 求值确定性 + 不改变状态。
    #[test]
    fn gate_expr_deterministic(
        seed in any::<u64>(),
    ) {
        // 用 seed 构造一个小状态（手工 LCG，避免引入 rand 依赖）
        let mut s = seed;
        let mut state = CounterState::new();
        let mut next = 0u64;
        for ty in ["T0", "T1", "T2", "T3"] {
            for _ in 0..(s % 3) {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                next += 1;
                let id = tid(&format!("tok-{next:04}"));
                state.counter.insert(
                    id.clone(),
                    Token {
                        id: id.clone(),
                        ty: tyn(ty),
                        thing_ref: ThingRef::none(),
                        produced_by: par("p0"),
                        depends_on: vec![tid("tok-0001")],
                        produced_at: "2026-01-01T00:00:00Z".into(),
                    },
                );
            }
        }
        if state.counter.is_empty() {
            state.counter.insert(
                tid("tok-0001"),
                Token { id: tid("tok-0001"), ty: tyn("T0"), thing_ref: ThingRef::none(),
                        produced_by: par("p0"), depends_on: vec![], produced_at: "t".into() },
            );
        }
        let target = Token {
            id: tid("target"), ty: tyn("T3"), thing_ref: ThingRef::none(),
            produced_by: par("p0"), depends_on: vec![tid("tok-0001")], produced_at: "t".into(),
        };
        let exprs = vec![
            GateExpr::Via(tyn("T0")),
            GateExpr::Exists(tyn("T1")),
            GateExpr::Count { ty: tyn("T2"), op: CountOp::Ge, n: 1 },
            GateExpr::Count { ty: tyn("T2"), op: CountOp::Eq, n: 0 },
            GateExpr::Not(Box::new(GateExpr::Via(tyn("T1")))),
            GateExpr::And(
                Box::new(GateExpr::Via(tyn("T0"))),
                Box::new(GateExpr::Or(
                    Box::new(GateExpr::Exists(tyn("T1"))),
                    Box::new(GateExpr::Count { ty: tyn("T2"), op: CountOp::Eq, n: 0 }),
                )),
            ),
        ];
        let before = state.clone();
        for e in &exprs {
            let r1 = e.evaluate(&target, &state);
            let r2 = e.evaluate(&target, &state);
            assert_eq!(r1, r2, "I6：求值不确定");
        }
        assert_eq!(state, before, "I6：求值不得改变状态");
    }
}

//! pact-compose 集成测试：论文 rev1 §1.8 并行组合（四条件一致性）。
//!
//! 1. §1.8 示例：D₁（设计阶段）∥ D₂（实现阶段）= 论文给出的合并域
//! 2. 顺序组合（类型共享作为组合接口）
//! 3. C2 冲突：共享类型前置不一致 / 源状态漂移
//! 4. C3 冲突：共享子句条件非同一；同条件幂等合并
//! 5. C4 终检：非良构片段（源类型 produce gate）
//! 6. Seeds 并 / 多片段折叠 / 空组合单位元

use pact_compose::{compose, compose_pair, Conflict};
use pact_core::{Action, Domain, DomainError, GateExpr, ParticipantName, TypeName};
use pact_parser::load_domain;
use std::collections::{BTreeMap, BTreeSet};

fn tyn(s: &str) -> TypeName {
    TypeName(s.into())
}
fn par(s: &str) -> ParticipantName {
    ParticipantName(s.into())
}

// 论文 §1.8 Example：设计阶段片段
const DESIGN_PHASE: &str = r#"
types { requirement; design }
preconditions { requirement → design }
gates { architect: reads requirement, produces design }
seed_authority { external_stakeholder: seeds requirement }
"#;

// 论文 §1.8 Example：实现阶段片段（共享类型 design 为组合接口）
const IMPL_PHASE: &str = r#"
types { design; implementation }
preconditions { design → implementation }
gates { developer: reads design, produces implementation }
seed_authority { }
"#;

// 论文 §1.8 Example 给出的合并结果
const EXPECTED_MERGED: &str = r#"
types { requirement; design; implementation }
preconditions { requirement → design; design → implementation }
gates { architect: reads requirement, produces design;
        developer: reads design, produces implementation }
seed_authority { external_stakeholder: seeds requirement }
"#;

#[test]
fn paper_example_parallel_composition() {
    let d1 = load_domain(DESIGN_PHASE).unwrap();
    let d2 = load_domain(IMPL_PHASE).unwrap();
    let expected = load_domain(EXPECTED_MERGED).unwrap();

    // 二元组合
    assert_eq!(compose_pair(&d1, &d2).unwrap(), expected);
    // 组合交换律（集并对称）
    assert_eq!(compose_pair(&d2, &d1).unwrap(), expected);
    // 多片段折叠
    assert_eq!(compose(&[d1.clone(), d2.clone()]).unwrap(), expected);
    // 空组合 = 单位元；单片段 = 自身
    assert_eq!(compose(&[]).unwrap(), empty_domain());
    assert_eq!(compose(&[d1.clone()]).unwrap(), d1);
}

fn empty_domain() -> Domain {
    Domain {
        types: BTreeSet::new(),
        preconditions: BTreeMap::new(),
        gates: BTreeMap::new(),
        seed_authority: BTreeSet::new(),
    }
}

/// 直接构造 Domain（绕过 parser 良构校验，用于 C4 终检用例）。
fn raw_domain(
    types: &[&str],
    pre: &[(&[&str], &str)],
    gates: &[(&str, Action, &str, Option<GateExpr>)],
    seeds: &[(&str, &str)],
) -> Domain {
    Domain {
        types: types.iter().map(|s| tyn(s)).collect(),
        preconditions: pre
            .iter()
            .map(|(lhs, rhs)| (tyn(rhs), lhs.iter().map(|s| tyn(s)).collect()))
            .collect(),
        gates: gates
            .iter()
            .map(|(p, a, t, c)| ((par(p), *a, tyn(t)), c.clone()))
            .collect(),
        seed_authority: seeds.iter().map(|(p, t)| (par(p), tyn(t))).collect(),
    }
}

#[test]
fn precondition_mismatch_conflict() {
    // C2 冲突：双方都为共享类型 x 声明了前置规则且集合不同
    let da = raw_domain(
        &["x", "y", "z"],
        &[(&["z"], "x"), (&["x"], "y")],
        &[("p", Action::Produce, "y", None)],
        &[("ext", "z")],
    );
    let db = raw_domain(
        &["x", "w", "v"],
        &[(&["w"], "x"), (&["x"], "v")],
        &[("q", Action::Produce, "v", None)],
        &[("ext", "w")],
    );
    match compose_pair(&da, &db) {
        Err(cs) => assert_eq!(
            cs,
            vec![Conflict::PreconditionMismatch {
                ty: tyn("x"),
                pre_a: BTreeSet::from([tyn("z")]),
                pre_b: BTreeSet::from([tyn("w")]),
            }]
        ),
        Ok(_) => panic!("共享类型双侧前置规则不同应报 C2 冲突"),
    }

    // D18：单侧声明合法（缺席方无约束，合并取声明侧规则）
    let dc = raw_domain(
        &["x", "w"],
        &[(&["x"], "w")],
        &[("q", Action::Produce, "w", None)],
        &[("ext", "x")],
    );
    let merged = compose_pair(&da, &dc).unwrap();
    assert_eq!(
        merged.preconditions.get(&tyn("x")),
        Some(&BTreeSet::from([tyn("z")])),
        "单侧声明时合并取声明侧规则（论文 §1.8 示例语义）"
    );
    assert_eq!(merged.preconditions.len(), 3, "双方规则按并合并（z→x、x→y、x→w）");
}

#[test]
fn same_rhs_different_preds_conflict() {
    // C1：两域对同一 rhs t 各有一条前置规则且前驱不同
    //（rhs t 必属双方 types，良构引用封闭）
    let da = raw_domain(
        &["a", "t"],
        &[(&["a"], "t")],
        &[("p", Action::Produce, "t", None)],
        &[("ext", "a")],
    );
    let db = raw_domain(
        &["b", "t"],
        &[(&["b"], "t")],
        &[("q", Action::Produce, "t", None)],
        &[("ext", "b")],
    );
    match compose_pair(&da, &db) {
        Err(cs) => assert_eq!(
            cs,
            vec![Conflict::PreconditionMismatch {
                ty: tyn("t"),
                pre_a: BTreeSet::from([tyn("a")]),
                pre_b: BTreeSet::from([tyn("b")]),
            }]
        ),
        Ok(_) => panic!("同 rhs 不同前驱应报 C1/C2 冲突"),
    }

    // 双侧同规则：冗余但一致，幂等合并为一条
    let dc = raw_domain(
        &["a", "t"],
        &[(&["a"], "t")],
        &[("q", Action::Produce, "t", None)],
        &[("ext", "a")],
    );
    let merged = compose_pair(&da, &dc).unwrap();
    assert_eq!(merged.preconditions.len(), 1, "同规则合并为一条");
}

#[test]
fn gate_condition_mismatch_conflict_and_idempotent_merge() {
    // C3：共享 (p, Read, t) 子句，一侧 via w 一侧无条件
    let da = raw_domain(
        &["w", "t"],
        &[(&["w"], "t")],
        &[
            ("p", Action::Read, "t", Some(GateExpr::Via(tyn("w")))),
            ("p", Action::Produce, "t", None),
        ],
        &[("ext", "w")],
    );
    let db = raw_domain(
        &["w", "t"],
        &[(&["w"], "t")],
        &[("p", Action::Read, "t", None), ("p", Action::Produce, "t", None)],
        &[("ext", "w")],
    );
    match compose_pair(&da, &db) {
        Err(cs) => assert_eq!(
            cs,
            vec![Conflict::GateConditionMismatch {
                participant: par("p"),
                action: Action::Read,
                ty: tyn("t"),
                cond_a: Some(GateExpr::Via(tyn("w"))),
                cond_b: None,
            }]
        ),
        Ok(_) => panic!("共享子句条件不同应报 C3 冲突"),
    }

    // 同一条件的共享子句：幂等合并（保留一份，无冲突）
    let db2 = raw_domain(
        &["w", "t"],
        &[(&["w"], "t")],
        &[
            ("p", Action::Read, "t", Some(GateExpr::Via(tyn("w")))),
            ("p", Action::Produce, "t", None),
        ],
        &[("ext", "w")],
    );
    let merged = compose_pair(&da, &db2).unwrap();
    assert_eq!(merged.gates.len(), 2, "同条件子句合并为一份");
    assert_eq!(
        merged.gates.get(&(par("p"), Action::Read, tyn("t"))),
        Some(&Some(GateExpr::Via(tyn("w"))))
    );
}

#[test]
fn ill_formed_fragment_caught_by_final_validation() {
    // C4 终检：片段自带源类型 produce gate（绕过 parser 直接构造）
    let bad = raw_domain(
        &["a", "b"],
        &[(&["a"], "b")],
        &[("p", Action::Produce, "a", None), ("p", Action::Produce, "b", None)],
        &[],
    );
    match compose_pair(&empty_domain(), &bad) {
        Err(cs) => assert_eq!(
            cs,
            vec![Conflict::IllFormed(DomainError::SourceTypeProduceGate {
                participant: par("p"),
                ty: tyn("a"),
            })]
        ),
        Ok(_) => panic!("源类型 produce gate 应被终检捕获"),
    }
}

#[test]
fn seeds_union_and_shared_gate_types() {
    // Seeds 是权限关系，重叠条目无冲突（论文 §1.8 证明）
    let da = raw_domain(
        &["s", "t"],
        &[(&["s"], "t")],
        &[("p", Action::Produce, "t", None)],
        &[("ext", "s"), ("auditor", "s")],
    );
    let db = raw_domain(
        &["s", "u"],
        &[(&["s"], "u")],
        &[("q", Action::Produce, "u", None)],
        &[("ext", "s")],
    );
    let merged = compose_pair(&da, &db).unwrap();
    assert_eq!(merged.types.len(), 3);
    assert_eq!(merged.preconditions.len(), 2);
    assert_eq!(merged.seed_authority.len(), 2, "Seeds 按并合并，重叠条目无冲突");
    // 共享类型 s 前置一致（双方都无 s 的规则——源类型）→ 通过
}

#[test]
fn multi_fragment_fold_chain() {
    // 三片段折叠：设计 ∥ 实现 ∥ 测试（顺序组合链，共享类型为接口）
    let d1 = load_domain(DESIGN_PHASE).unwrap();
    let d2 = load_domain(IMPL_PHASE).unwrap();
    let d3 = load_domain(
        r#"
types { implementation; test_report }
preconditions { implementation → test_report }
gates { tester: reads implementation, produces test_report }
seed_authority { }
"#,
    )
    .unwrap();

    let merged = compose(&[d1, d2, d3]).unwrap();
    assert_eq!(merged.types.len(), 4);
    assert_eq!(merged.preconditions.len(), 3);
    assert_eq!(merged.seed_authority.len(), 1);
    // 合并域经良构终检，且六项分析可用（analysis 依赖由测试引入验证互操作）
    assert!(pact_core::validate_domain(&merged).is_ok());
}

#[test]
fn conflict_stops_fold_with_details() {
    // 折叠链在首个冲突对返回该对的全部明细
    let d1 = load_domain(DESIGN_PHASE).unwrap();
    let d2 = load_domain(IMPL_PHASE).unwrap();
    // d1∥d2 后 design 的前置为 {requirement}（单侧声明，D18 合法）；
    // d3 也为 design 声明前置 {req2}——双侧规则不同 → C2 冲突
    let d3 = raw_domain(
        &["design", "req2", "deploy"],
        &[(&["req2"], "design"), (&["design"], "deploy")],
        &[("op", Action::Produce, "deploy", None)],
        &[("ext", "req2")],
    );
    match compose(&[d1, d2, d3]) {
        Err(cs) => {
            assert_eq!(cs.len(), 1);
            assert!(matches!(
                &cs[0],
                Conflict::PreconditionMismatch { ty, .. } if *ty == tyn("design")
            ));
        }
        Ok(_) => panic!("折叠链应在冲突处失败"),
    }
}

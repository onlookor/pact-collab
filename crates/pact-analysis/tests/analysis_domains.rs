//! pact-analysis 集成测试：论文域（Part II / Part III / §4.2）六项分析验证
//! + 构造域的正/负用例（论文 Appendix A 各算法的触发条件）。

use pact_analysis::{
    consumption_reachability, gate_coverage, is_reachable, precondition_wf, reachability,
    stranding_risk, structural_deadlock, StrandingRisk, WfWarning,
};
use pact_core::{Action, CountOp, Domain, DomainError, GateExpr, ParticipantName, TypeName};
use pact_parser::load_domain;
use std::collections::BTreeSet;

fn tyn(s: &str) -> TypeName {
    TypeName(s.into())
}
fn par(s: &str) -> ParticipantName {
    ParticipantName(s.into())
}

// 论文 rev1 Part II §1（源生产约束版：analyst 角色改为 seed authority）
const LIFECYCLE: &str = r#"
types { requirement; design; implementation; test_report; deployment }

preconditions {
    requirement → design;
    design → implementation;
    implementation → test_report;
    test_report → deployment
}

gates {
    architect: reads requirement, consumes requirement, produces design;
    developer: reads design, consumes design, produces implementation;
    tester: reads implementation, consumes implementation, produces test_report;
    operator: reads test_report, consumes test_report, produces deployment;
    stakeholder: consumes deployment
}

seed_authority { external_stakeholder: seeds requirement }
"#;

// 论文 Part II §2.1（医疗路径）
const HEALTHCARE: &str = r#"
types { admission; triage; diagnosis; lab_result; imaging_result; treatment_plan;
        treatment; discharge_summary }

preconditions {
    admission → triage;
    triage → diagnosis;
    diagnosis → lab_result;
    diagnosis → imaging_result;
    { lab_result + imaging_result } → treatment_plan;
    treatment_plan → treatment;
    treatment → discharge_summary
}

gates {
    nurse: reads admission, consumes admission, produces triage;
    physician: reads triage, consumes triage, produces diagnosis,
               reads treatment_plan, consumes treatment_plan,
               produces treatment, reads treatment, consumes treatment,
               produces discharge_summary;
    lab_tech: reads diagnosis via triage, consumes diagnosis via triage,
              produces lab_result, consumes lab_result;
    radiologist: reads diagnosis via triage, consumes diagnosis via triage,
                 produces imaging_result, consumes imaging_result;
    care_team: reads diagnosis, reads lab_result, consumes lab_result,
               reads imaging_result, consumes imaging_result,
               produces treatment_plan;
    admin: consumes discharge_summary;
    quality_officer: reads diagnosis, reads treatment_plan, reads treatment
}

seed_authority { registrar: seeds admission }
"#;

// 论文 Part III §1（ReAct 循环：全循环拓扑）
const REACT: &str = r#"
types { observation; thought; action; tool_result }

preconditions {
    observation → thought;
    thought → action;
    action → tool_result;
    tool_result → observation
}

gates {
    agent: reads observation, consumes observation, produces thought,
            reads thought, consumes thought, produces action,
            reads action, consumes action, produces tool_result,
            reads tool_result, consumes tool_result, produces observation
}

seed_authority { user: seeds observation }
"#;

// 论文 §4.2 区分性场景域
const DISCRIMINATING: &str = r#"
types { S; W; T }

preconditions { S → W; { W + S } → T }

gates {
    producer: reads S, reads W, produces W, produces T;
    p1: reads T via W;
    p2: reads T;
    w_eater: consumes W
}

seed_authority { external: seeds S }
"#;

// ============================================================
// 1. 软件工程生命周期（论文 rev1 Part II §1）：六项分析全清
// ============================================================

#[test]
fn lifecycle_all_analyses_clean() {
    let d = load_domain(LIFECYCLE).unwrap();

    // A.1：requirement 为种子，全部 5 类型可达
    let r = reachability(&d);
    assert_eq!(r.seed_types, BTreeSet::from([tyn("requirement")]));
    assert!(
        r.unreachable.is_empty(),
        "全部类型应可达：{:?}",
        r.unreachable
    );
    assert!(is_reachable(&d, &tyn("deployment")));
    assert_eq!(r.reachable.len(), 5);

    // A.2：全部非源类型有 producer
    assert!(gate_coverage(&d).is_empty(), "生产覆盖完整");
    // A.3：全部类型有 consumer
    assert!(consumption_reachability(&d).is_empty(), "消费可达完整");
    // A.4：链式无环，无死锁分量
    assert!(structural_deadlock(&d).is_empty(), "无结构死锁");
    // A.5：良构且无 Seed/Produce 重叠警告
    let wf = precondition_wf(&d);
    assert!(wf.well_formed);
    assert!(wf.warnings.is_empty(), "无警告：{:?}", wf.warnings);
    // A.6：论文 rev1 版无 via 条件，无搁浅风险
    assert!(stranding_risk(&d).is_empty(), "无搁浅风险");
}

// ============================================================
// 2. 医疗路径（论文 Part II §2）：via 搁浅风险（无环，确定性）
// ============================================================

#[test]
fn healthcare_stranding_risk_reported() {
    let d = load_domain(HEALTHCARE).unwrap();

    // A.1/A.2/A.3/A.4/A.5 全清
    let r = reachability(&d);
    assert_eq!(r.seed_types, BTreeSet::from([tyn("admission")]));
    assert!(r.unreachable.is_empty());
    assert!(gate_coverage(&d).is_empty());
    assert!(consumption_reachability(&d).is_empty());
    assert!(structural_deadlock(&d).is_empty());
    let wf = precondition_wf(&d);
    assert!(wf.well_formed);
    assert!(wf.warnings.is_empty(), "无警告：{:?}", wf.warnings);

    // A.6：lab_tech / radiologist 读 diagnosis via triage；
    // triage 是 diagnosis 的直接前驱，physician 持有 consumes triage
    // → 两条确定性搁浅风险（无环拓扑）
    let risks = stranding_risk(&d);
    assert_eq!(
        risks,
        vec![
            StrandingRisk {
                ty_b: tyn("diagnosis"),
                ty_a: tyn("triage"),
                via_holder: par("lab_tech"),
                consumer: par("physician"),
                potential: false,
            },
            StrandingRisk {
                ty_b: tyn("diagnosis"),
                ty_a: tyn("triage"),
                via_holder: par("radiologist"),
                consumer: par("physician"),
                potential: false,
            },
        ],
        "医疗域应报告两条 via triage 搁浅风险"
    );
}

// ============================================================
// 3. ReAct 循环（论文 Part III §1）：循环拓扑 + 种子入口
// ============================================================

#[test]
fn react_cyclic_topology_no_deadlock() {
    let d = load_domain(REACT).unwrap();

    // 全循环 SCC，但 user: seeds observation 提供种子入口 → 无死锁
    assert!(structural_deadlock(&d).is_empty(), "种子入口消解环死锁");
    // 全部类型可达 / 可生产 / 可消费
    assert!(reachability(&d).unreachable.is_empty());
    assert!(gate_coverage(&d).is_empty());
    assert!(consumption_reachability(&d).is_empty());
    // 无 via 条件 → 无搁浅风险
    assert!(stranding_risk(&d).is_empty());
}

// ============================================================
// 4. 区分性场景（论文 §4.2）：消费缺口 + 确定性搁浅
// ============================================================

#[test]
fn discriminating_unconsumable_and_stranding() {
    let d = load_domain(DISCRIMINATING).unwrap();

    // A.3：S 与 T 无消费者（State B 中 T₁ 搁浅在 Counter 的结构信号，
    // 论文 §II.1.6 技术债）
    assert_eq!(consumption_reachability(&d), vec![tyn("S"), tyn("T")]);

    // A.6：p1 reads T via W；W 是 T 的直接前驱（{W+S}→T）；
    // w_eater 持有 consumes W → 确定性风险（正是 State B 演绎）
    assert_eq!(
        stranding_risk(&d),
        vec![StrandingRisk {
            ty_b: tyn("T"),
            ty_a: tyn("W"),
            via_holder: par("p1"),
            consumer: par("w_eater"),
            potential: false,
        }]
    );

    // A.1/A.2/A.4/A.5 清
    assert!(reachability(&d).unreachable.is_empty());
    assert!(gate_coverage(&d).is_empty());
    assert!(structural_deadlock(&d).is_empty());
    let wf = precondition_wf(&d);
    assert!(wf.well_formed);
    assert!(wf.warnings.is_empty(), "无警告：{:?}", wf.warnings);
}

// ============================================================
// 5. 构造用例：死锁 / 外部馈入 / 不可达 / 循环保守 / 间接祖先
// ============================================================

fn tiny_domain(
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
fn deadlock_detected_and_exemptions() {
    // 环 {a,b} 无种子入口 → 死锁
    let d = tiny_domain(
        &["a", "b"],
        &[(&["a"], "b"), (&["b"], "a")],
        &[
            ("p", Action::Produce, "a", None),
            ("p", Action::Produce, "b", None),
        ],
        &[],
    );
    // 注：此域 ill-formed（produces a 指向源类型——环内互为前驱所以 a 有前置？
    // a 的前置是 b（b→a），非源类型 → 良构）
    assert_eq!(
        structural_deadlock(&d),
        vec![pact_analysis::DeadlockComponent {
            types: vec![tyn("a"), tyn("b")]
        }]
    );

    // 种子入口消解：环 {a,b} + seed a
    let d = tiny_domain(
        &["a", "b"],
        &[(&["a"], "b"), (&["b"], "a")],
        &[("p", Action::Produce, "b", None)],
        &[("ext", "a")],
    );
    assert!(structural_deadlock(&d).is_empty(), "种子入口消解环死锁");

    // 外部馈入消解：环 {a,b}，c（种子可达）是 a 的额外前驱（{b+c}→a）
    let d = tiny_domain(
        &["a", "b", "c"],
        &[(&["a"], "b"), (&["b", "c"], "a")],
        &[("p", Action::Produce, "b", None)],
        &[("ext", "c")],
    );
    assert!(structural_deadlock(&d).is_empty(), "外部可馈入的环不死锁");

    // 非种子可达的外部前驱不消解：同图，但无任何种子 → c 不可达
    let d = tiny_domain(
        &["a", "b", "c"],
        &[(&["a"], "b"), (&["b", "c"], "a")],
        &[("p", Action::Produce, "b", None)],
        &[],
    );
    assert_eq!(
        structural_deadlock(&d),
        vec![pact_analysis::DeadlockComponent {
            types: vec![tyn("a"), tyn("b")]
        }],
        "前驱自身不可达时不构成有效馈入"
    );
}

#[test]
fn unreachable_type_reported() {
    // x 孤立（无种子可达路径）
    let d = tiny_domain(
        &["a", "b", "x"],
        &[(&["a"], "b")],
        &[("p", Action::Produce, "b", None)],
        &[("ext", "a")],
    );
    let r = reachability(&d);
    assert_eq!(r.unreachable, vec![tyn("x")]);
    assert!(!is_reachable(&d, &tyn("x")));
    assert!(is_reachable(&d, &tyn("b")));
}

#[test]
fn gate_coverage_reports_uncovered_non_source() {
    // b 非源类型但无 producer；c 有 producer
    let d = tiny_domain(
        &["a", "b", "c"],
        &[(&["a"], "b"), (&["b"], "c")],
        &[
            ("p", Action::Produce, "c", None),
            ("p", Action::Consume, "b", None),
        ],
        &[("ext", "a")],
    );
    assert_eq!(gate_coverage(&d), vec![tyn("b")]);
}

#[test]
fn precondition_wf_reports_ill_formed() {
    // 非良构域（未经 validate_domain 拒绝，A.5 独立复核）：
    // p 对源类型 a 持有 produces gate（§3.1 约束 6）
    let d = tiny_domain(
        &["a", "b"],
        &[(&["a"], "b")],
        &[
            ("p", Action::Produce, "a", None),
            ("p", Action::Produce, "b", None),
        ],
        &[],
    );
    let wf = precondition_wf(&d);
    assert!(!wf.well_formed);
    assert!(matches!(
        wf.violations.as_slice(),
        [DomainError::SourceTypeProduceGate { .. }]
    ));
    assert!(wf.warnings.is_empty());
    // 同一违例在 core 良构校验下的对应错误（A.5 委托 validate_domain）
    assert!(matches!(
        pact_core::validate_domain(&d),
        Err(es) if matches!(es.as_slice(), [DomainError::SourceTypeProduceGate { .. }])
    ));
}

#[test]
fn precondition_wf_reports_empty_lhs_and_zero_count() {
    // 空 LHS（§3.1 约束 3 Non-Empty LHS）：程序化构造的 Domain 绕过解析期检查，
    // 由 validate_domain 声明期防御捕获（b 上不放 produce，避免叠加源生产违例）
    let d = tiny_domain(
        &["a", "b"],
        &[(&[], "b")],
        &[("p", Action::Read, "b", None)],
        &[],
    );
    let wf = precondition_wf(&d);
    assert!(!wf.well_formed);
    assert!(matches!(
        wf.violations.as_slice(),
        [DomainError::EmptyPrecondLhs { .. }]
    ));

    // 零计数字面量（§3.1 约束 5 Positive-Nat Literal）：count a >= 0 非法
    // （嵌套在 Not 内也应被递归捕获）
    let d = tiny_domain(
        &["a", "b"],
        &[(&["a"], "b")],
        &[
            ("p", Action::Produce, "b", None),
            (
                "q",
                Action::Read,
                "b",
                Some(GateExpr::Not(Box::new(GateExpr::Count {
                    ty: tyn("a"),
                    op: CountOp::Ge,
                    n: 0,
                }))),
            ),
        ],
        &[("ext", "a")],
    );
    let wf = precondition_wf(&d);
    assert!(!wf.well_formed);
    assert!(matches!(
        wf.violations.as_slice(),
        [DomainError::ZeroCountLiteral { .. }]
    ));
}

#[test]
fn precondition_wf_warns_seed_produce_overlap() {
    // §1.4.3 Advisory：p 对非源类型 b 既持 seed 权威又持 produces gate → 警告非违规
    let d = tiny_domain(
        &["a", "b"],
        &[(&["a"], "b")],
        &[("p", Action::Produce, "b", None)],
        &[("p", "b")],
    );
    let wf = precondition_wf(&d);
    assert!(wf.well_formed, "重叠只是警告：{:?}", wf.violations);
    assert_eq!(
        wf.warnings,
        vec![WfWarning::SeedProduceOverlap {
            participant: par("p"),
            ty: tyn("b")
        }]
    );

    // 类型级判定：seed 持有者与 produce 方不是同一参与者也告警（participant 字段 = seed 持有者）
    let d = tiny_domain(
        &["a", "b"],
        &[(&["a"], "b")],
        &[("p", Action::Produce, "b", None)],
        &[("q", "b")],
    );
    let wf = precondition_wf(&d);
    assert!(
        wf.well_formed,
        "跨参与者重叠仍只是警告：{:?}",
        wf.violations
    );
    assert_eq!(
        wf.warnings,
        vec![WfWarning::SeedProduceOverlap {
            participant: par("q"),
            ty: tyn("b")
        }]
    );

    // 源类型上的 seed 权威不触发警告（seed 本就是源类型的合法入口）
    let d = tiny_domain(
        &["a", "b"],
        &[(&["a"], "b")],
        &[("p", Action::Produce, "b", None)],
        &[("ext", "a")],
    );
    let wf = precondition_wf(&d);
    assert!(wf.well_formed);
    assert!(wf.warnings.is_empty());
}

#[test]
fn self_loop_precondition_is_well_formed() {
    // §1.4.4：自环 A → A 合法（rhs 出现在自己的 lhs 中）
    let d = tiny_domain(
        &["a", "b"],
        &[(&["a"], "b"), (&["a", "b"], "a")],
        &[("p", Action::Produce, "b", None)],
        &[("ext", "a")],
    );
    let wf = precondition_wf(&d);
    assert!(wf.well_formed, "自环合法：{:?}", wf.violations);
    assert!(wf.warnings.is_empty());
}

#[test]
fn cyclic_stranding_is_conservative() {
    // 循环拓扑 {a,b}：p reads b via a，q consumes a → POTENTIAL（保守报告）
    let d = tiny_domain(
        &["a", "b"],
        &[(&["a"], "b"), (&["b"], "a")],
        &[
            ("p", Action::Read, "b", Some(GateExpr::Via(tyn("a")))),
            ("q", Action::Consume, "a", None),
        ],
        &[("ext", "a")],
    );
    let risks = stranding_risk(&d);
    assert_eq!(risks.len(), 1);
    assert!(risks[0].potential, "循环拓扑应保守标记 potential");
    assert_eq!(risks[0].ty_b, tyn("b"));
    assert_eq!(risks[0].ty_a, tyn("a"));
}

#[test]
fn indirect_ancestor_via_not_reported() {
    // a→b→c 链：p reads c via a —— a 是间接祖先（非直接前驱），
    // via 永不可满足 → 无风险（论文 Critical semantic note）
    let d = tiny_domain(
        &["a", "b", "c"],
        &[(&["a"], "b"), (&["b"], "c")],
        &[
            ("p", Action::Read, "c", Some(GateExpr::Via(tyn("a")))),
            ("q", Action::Consume, "a", None),
        ],
        &[("ext", "a")],
    );
    assert!(
        stranding_risk(&d).is_empty(),
        "间接祖先的 via 不构成搁浅风险"
    );
}

#[test]
fn no_consumer_no_stranding() {
    // via T_a 但 T_a 无消费者 → 无风险（论文：Consumers = ∅ → continue）
    let d = tiny_domain(
        &["a", "b"],
        &[(&["a"], "b")],
        &[("p", Action::Read, "b", Some(GateExpr::Via(tyn("a"))))],
        &[("ext", "a")],
    );
    assert!(stranding_risk(&d).is_empty());
}

#[test]
fn nested_via_atoms_are_collected() {
    // 复合条件中的嵌套 Via 原子：p reads b when (via a and exists a)
    let d = tiny_domain(
        &["a", "b"],
        &[(&["a"], "b")],
        &[
            (
                "p",
                Action::Read,
                "b",
                Some(GateExpr::And(
                    Box::new(GateExpr::Via(tyn("a"))),
                    Box::new(GateExpr::Exists(tyn("a"))),
                )),
            ),
            ("q", Action::Consume, "a", None),
        ],
        &[("ext", "a")],
    );
    let risks = stranding_risk(&d);
    assert_eq!(risks.len(), 1, "复合表达式内的 via 原子应被提取");
    assert_eq!(risks[0].ty_a, tyn("a"));
}

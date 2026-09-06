//! 黄金轨迹测试（NFR1 / design.md §10）：逐事件比对论文全部示例轨迹。
//!
//! 1. 软件工程生命周期 t₀–t₁₀（Part II §1.5）
//! 2. 医疗路径 t₁–t₁₃（Part II §2.1.3，via 条件全部通过点）
//! 3. ReAct 循环（Part III §1.2，循环拓扑 + Counter O(1) 卫生）
//! 4. §4.2 区分性场景 State A/B（via 溯源敏感性回归锚点）

use pact_core::{
    enabled_productions, rule_e, rule_p, rule_r, rule_s, termination_holds, CounterState, Domain,
    Effect, ParticipantName, RuleError, ThingRef, Timestamp, TokenId, TypeName,
};
use pact_parser::load_domain;

fn par(s: &str) -> ParticipantName {
    ParticipantName(s.into())
}
fn tyn(s: &str) -> TypeName {
    TypeName(s.into())
}
fn tid(s: &str) -> TokenId {
    TokenId(s.into())
}

/// 确定性驱动器：顺序 id + 递增逻辑时钟（D8 LogicalClock 的测试形态）。
struct Driver {
    domain: Domain,
    state: CounterState,
    next_id: u64,
    tick: u64,
}

impl Driver {
    fn new(src: &str) -> Self {
        Driver {
            domain: load_domain(src).expect("黄金测试域必须良构"),
            state: CounterState::new(),
            next_id: 0,
            tick: 0,
        }
    }

    fn tid(&mut self) -> TokenId {
        self.next_id += 1;
        tid(&format!("t{:03}", self.next_id))
    }

    fn now(&mut self) -> Timestamp {
        self.tick += 1;
        format!("2026-01-01T00:00:{:02}Z", self.tick)
    }

    fn seed(&mut self, p: &str, ty: &str, thing: Option<&str>) -> TokenId {
        let id = self.tid();
        let now = self.now();
        let tok = rule_s(
            &self.domain,
            &par(p),
            &tyn(ty),
            thing.map(ThingRef::some).unwrap_or_else(ThingRef::none),
            id.clone(),
            now,
        )
        .expect("黄金轨迹中的种子注入必须成功");
        self.state.apply(Effect::Seeded(tok));
        id
    }

    fn produce(&mut self, p: &str, ty: &str, preds: &[TokenId], thing: Option<&str>) -> TokenId {
        let id = self.tid();
        let now = self.now();
        let tok = rule_p(
            &self.domain,
            &self.state,
            &par(p),
            &tyn(ty),
            preds,
            thing.map(ThingRef::some).unwrap_or_else(ThingRef::none),
            id.clone(),
            now,
        )
        .unwrap_or_else(|e| panic!("黄金轨迹中的生产必须成功：{e:?}"));
        self.state.apply(Effect::Produced(tok));
        id
    }

    fn read_ok(&mut self, p: &str, id: &TokenId) {
        rule_r(&self.domain, &self.state, &par(p), id)
            .unwrap_or_else(|e| panic!("黄金轨迹中的读取必须成功：{e:?}"));
    }

    fn consume(&mut self, p: &str, id: &TokenId) {
        let now = self.now();
        let entry = rule_e(&self.domain, &self.state, &par(p), id, now)
            .unwrap_or_else(|e| panic!("黄金轨迹中的消费必须成功：{e:?}"));
        assert_eq!(entry.exhausted_by, par(p));
        self.state.apply(Effect::Consumed(entry));
    }

    /// Counter 中类型多重集（排序后），用于逐事件比对。
    fn counter_types(&self) -> Vec<String> {
        let mut v: Vec<String> =
            self.state.counter.values().map(|t| t.ty.0.clone()).collect();
        v.sort();
        v
    }

    fn counter_len(&self) -> usize {
        self.state.counter.len()
    }

    fn archive_types(&self) -> Vec<String> {
        self.state.archive.iter().map(|e| e.token.ty.0.clone()).collect()
    }
}

// ============================================================
// 1. 软件工程生命周期（Part II §1.5，t₀–t₁₀）
// ============================================================

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

#[test]
fn golden_software_lifecycle() {
    let mut d = Driver::new(LIFECYCLE);

    // t₀：Rule-S —— external_stakeholder 注入 requirement（豁免前置条件）
    let req = d.seed("external_stakeholder", "requirement", Some("stakeholder-need-001"));
    assert_eq!(d.counter_types(), vec!["requirement"]);
    assert_eq!(d.state.counter[&req].produced_by, par("external_stakeholder"));
    assert_eq!(d.state.counter[&req].depends_on, Vec::<TokenId>::new()); // 种子无前驱
    assert_eq!(d.state.counter[&req].thing_ref, ThingRef::some("stakeholder-need-001"));

    // t₁：architect 读 requirement（非破坏性）→ 生产 design（前驱 requirement 不动）
    d.read_ok("architect", &req);
    let design = d.produce("architect", "design", &[req.clone()], Some("docs/design-001.md"));
    assert_eq!(d.counter_types(), vec!["design", "requirement"]);
    assert_eq!(d.state.counter[&design].depends_on, vec![req.clone()]);

    // t₂：developer 读 design → 生产 implementation
    d.read_ok("developer", &design);
    let impl_t = d.produce("developer", "implementation", &[design.clone()], Some("src/impl-001"));
    assert_eq!(d.counter_len(), 3);

    // t₃：tester 读 implementation → 生产 test_report
    d.read_ok("tester", &impl_t);
    let report = d.produce("tester", "test_report", &[impl_t.clone()], Some("qa/report-001"));

    // t₄：operator 读 test_report（via implementation：见证在 Counter ∧ 在 depends_on）→ 生产 deployment
    d.read_ok("operator", &report);
    let deploy = d.produce("operator", "deployment", &[report.clone()], Some("ops/deploy-001"));
    assert_eq!(d.counter_len(), 5);

    // t₅–t₉：逆序消费（deployment → requirement），无级联
    d.consume("stakeholder", &deploy); // t₅
    d.consume("operator", &report); // t₆
    d.consume("tester", &impl_t); // t₇
    d.consume("developer", &design); // t₈
    d.consume("architect", &req); // t₉
    assert_eq!(d.counter_len(), 0);
    assert_eq!(d.archive_types(), vec!["deployment", "test_report", "implementation", "design", "requirement"]);

    // t₁₀：Rule-T —— 终止
    assert!(termination_holds(&d.domain, &d.state));
    // 已归档 token 不可再读
    assert_eq!(
        rule_r(&d.domain, &d.state, &par("architect"), &req),
        Err(RuleError::TokenNotOnCounter { token_id: req })
    );
}

// ============================================================
// 2. 医疗路径（Part II §2.1.3，t₁–t₁₃）
// ============================================================

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

#[test]
fn golden_healthcare_path() {
    let mut d = Driver::new(HEALTHCARE);

    // t₁：registrar 注入 admission（种子权威；registrar 无 gate 亦可——Rule-S 不查 gate）
    let adm = d.seed("registrar", "admission", Some("ehr:patient-42"));
    assert_eq!(d.counter_len(), 1);

    // t₂：nurse 读 admission → 生产 triage
    d.read_ok("nurse", &adm);
    let triage = d.produce("nurse", "triage", &[adm.clone()], Some("ehr:triage-42"));
    assert_eq!(d.counter_len(), 2);

    // t₃：physician 读 triage → 生产 diagnosis
    d.read_ok("physician", &triage);
    let diag = d.produce("physician", "diagnosis", &[triage.clone()], Some("ehr:dx-42"));
    assert_eq!(d.counter_len(), 3);

    // t₄：lab_tech 经 via triage 读 diagnosis → 生产 lab_result；
    //      radiologist 同样 via triage 读 → 生产 imaging_result
    d.read_ok("lab_tech", &diag);
    let lab = d.produce("lab_tech", "lab_result", &[diag.clone()], Some("lab:panel-42"));
    d.read_ok("radiologist", &diag);
    let img = d.produce("radiologist", "imaging_result", &[diag.clone()], Some("rad:ct-42"));
    assert_eq!(d.counter_len(), 5);

    // t₅：care_team 生产 treatment_plan（前置 {lab_result + imaging_result}，AND 语义）
    let plan = d.produce("care_team", "treatment_plan", &[lab.clone(), img.clone()], Some("plan:42"));
    assert_eq!(d.counter_len(), 6);

    // t₆：physician 生产 treatment、再生产 discharge_summary
    let tr = d.produce("physician", "treatment", &[plan.clone()], Some("rx:42"));
    let ds = d.produce("physician", "discharge_summary", &[tr.clone()], Some("doc:dc-42"));
    assert_eq!(d.counter_len(), 8);

    // t₇–t₁₃：逆拓扑序消费
    d.consume("admin", &ds); // t₇
    assert_eq!(d.counter_len(), 7);
    d.consume("physician", &tr); // t₈
    d.consume("physician", &plan); // t₉
    d.consume("care_team", &lab); // t₁₀
    d.consume("care_team", &img);
    assert_eq!(d.counter_len(), 3);
    // t₁₁：lab_tech 经 via triage 消费 diagnosis —— 见证 triage 仍在 Counter 上
    d.consume("lab_tech", &diag);
    assert_eq!(d.counter_len(), 2);
    // t₁₂：physician 消费 triage（无条件子句）
    d.consume("physician", &triage);
    // t₁₃：nurse 消费 admission → Counter = ∅
    d.consume("nurse", &adm);
    assert_eq!(d.counter_len(), 0);
    assert!(termination_holds(&d.domain, &d.state));
    assert_eq!(
        d.archive_types(),
        vec![
            "discharge_summary", "treatment", "treatment_plan",
            "lab_result", "imaging_result", "diagnosis", "triage", "admission",
        ]
    );
    // audit 链：discharge_summary 的完整溯源链
    let ds_entry = &d.state.archive[0];
    assert_eq!(ds_entry.token.depends_on, vec![tr.clone()]);
}

// ============================================================
// 3. ReAct 循环（Part III §1.2）：循环拓扑 + Counter O(1) 卫生
// ============================================================

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

#[test]
fn golden_react_loop() {
    let mut d = Driver::new(REACT);

    // 用户注入初始观察
    let mut obs = d.seed("user", "observation", Some("q:capital-of-france"));
    assert_eq!(d.counter_len(), 1);

    // 两轮 ReAct 循环；每一步后 Counter ≤ 2（O(1) 卫生不变量，论文 §1.6）
    for k in 0..2 {
        d.read_ok("agent", &obs);
        let thought = d.produce("agent", "thought", &[obs.clone()], Some(&format!("thought-{k}")));
        d.consume("agent", &obs);
        assert!(d.counter_len() <= 2);

        d.read_ok("agent", &thought);
        let action = d.produce("agent", "action", &[thought.clone()], Some(&format!("action-{k}")));
        d.consume("agent", &thought);
        assert!(d.counter_len() <= 2);

        d.read_ok("agent", &action);
        let tool = d.produce("agent", "tool_result", &[action.clone()], Some(&format!("tool-{k}")));
        d.consume("agent", &action);
        assert!(d.counter_len() <= 2);

        if k == 0 {
            // 继续循环：tool_result → 新 observation
            d.read_ok("agent", &tool);
            let next = d.produce("agent", "observation", &[tool.clone()], Some("obs-1"));
            d.consume("agent", &tool);
            obs = next;
            // 溯源链：新观察源于上一轮工具结果
            assert_eq!(d.state.counter[&obs].depends_on, vec![tool]);
        } else {
            // 终止分支：消费最后的 tool_result → Counter = ∅
            d.consume("agent", &tool);
        }
    }
    assert_eq!(d.counter_len(), 0);
    assert!(termination_holds(&d.domain, &d.state));
    // Rule-W 服务：Counter 空、全部类型有前置 → 无可满足生产
    assert!(enabled_productions(&d.domain, &d.state, &par("agent")).is_empty());
    // Archive：两轮 × 4 token = 8 条
    assert_eq!(d.archive_types().len(), 8);
}

// ============================================================
// 4. §4.2 区分性场景 State A/B（回归锚点）
//    Domain D_gate：S → W，{ W + S } → T；
//    p1 reads T via W（实例级见证），p2 reads T（无条件）
// ============================================================

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

#[test]
fn golden_discriminating_state_a_b() {
    let mut d = Driver::new(DISCRIMINATING);

    // 步骤 1–5（论文 §4.2 State A 构造）
    let s1 = d.seed("external", "S", None);
    let w1 = d.produce("producer", "W", &[s1.clone()], None);
    let t1 = d.produce("producer", "T", &[w1.clone(), s1.clone()], None);
    let s2 = d.seed("external", "S", None);
    d.produce("producer", "W", &[s2.clone()], None);
    assert_eq!(d.counter_types(), vec!["S", "S", "T", "W", "W"]);

    // State A：p1 读 T₁ —— W₁ 在 Counter 上 ∧ W₁ ∈ T₁.depends_on → 通过
    d.read_ok("p1", &t1);
    // p2（无 via）也通过
    d.read_ok("p2", &t1);

    // 转移到 State B：W₁ 被消费（论文：by some participant with consumes W gate）
    d.consume("w_eater", &w1);

    // State B：W₂ 仍在 Counter（类型级满足）但 W₂ ∉ T₁.depends_on → p1 被拒
    match rule_r(&d.domain, &d.state, &par("p1"), &t1) {
        Err(RuleError::GateConditionFailed { expr, .. }) => {
            assert_eq!(expr, pact_core::GateExpr::Via(tyn("W")));
        }
        other => panic!("State B：p1 读取必须被拒（溯源敏感），实际 {other:?}"),
    }
    // p2 不受影响：类型级无条件的读取始终通过
    d.read_ok("p2", &t1);
    // 状态未被拒绝的读取改变
    assert_eq!(d.counter_types(), vec!["S", "S", "T", "W"]);
}

// ============================================================
// 5. 附加：论文 §2.1.4 扩展 gate 条件（not/exists/count 组合）
// ============================================================

#[test]
fn extended_gate_conditions() {
    // physician: consumes treatment when not exists imaging_result
    // quality_officer: reads treatment_plan when count lab_result >= 1 and count imaging_result >= 1
    const EXT: &str = r#"
types { lab_result; imaging_result; treatment_plan; treatment }

preconditions { }

gates {
    physician: consumes treatment when not exists imaging_result;
    quality_officer: reads treatment_plan
        when count lab_result >= 1 and count imaging_result >= 1
}

seed_authority {
    external: seeds treatment;
    external: seeds treatment_plan;
    external: seeds lab_result;
    external: seeds imaging_result
}
"#;

    // 场景 1：not exists —— imaging_result 不在场，physician 消费通过
    let mut d = Driver::new(EXT);
    let tr = d.seed("external", "treatment", None);
    d.consume("physician", &tr);

    // 场景 2：imaging_result 在场 → physician 消费被拒（not exists 求值为假）
    let mut d = Driver::new(EXT);
    let tr = d.seed("external", "treatment", None);
    d.seed("external", "imaging_result", None);
    match rule_e(&d.domain, &d.state, &par("physician"), &tr, "t".into()) {
        Err(RuleError::GateConditionFailed { expr, .. }) => {
            assert_eq!(
                expr,
                pact_core::GateExpr::Not(Box::new(pact_core::GateExpr::Exists(tyn("imaging_result"))))
            );
        }
        other => panic!("imaging_result 在场时 physician 消费应被拒，实际 {other:?}"),
    }

    // 场景 3：count 合取 —— lab_result 缺失时 quality_officer 被拒；补齐后通过
    let plan = d.seed("external", "treatment_plan", None);
    d.seed("external", "imaging_result", None);
    assert!(matches!(
        rule_r(&d.domain, &d.state, &par("quality_officer"), &plan),
        Err(RuleError::GateConditionFailed { .. })
    ));
    d.seed("external", "lab_result", None);
    d.read_ok("quality_officer", &plan);
}

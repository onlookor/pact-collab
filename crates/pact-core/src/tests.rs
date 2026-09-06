//! pact-core 单元测试：规则语义、GateExpr、良构校验、Rule-W/T、Effect。
//! 共享一个手搭小域（论文 §4.2 区分性场景骨架，S/W/T 类型）。

#[cfg(test)]
mod tests {
    use crate::*;
    use std::collections::{BTreeMap, BTreeSet};

    // —— 测试域：types {S; W; T}，S→W，{W+S}→T ——
    fn domain() -> Domain {
        let t = |s: &str| TypeName(s.into());
        let p = |s: &str| ParticipantName(s.into());
        let mut gates = BTreeMap::new();
        gates.insert((p("producer"), Action::Read, t("S")), None);
        gates.insert((p("producer"), Action::Read, t("W")), None);
        gates.insert((p("producer"), Action::Produce, t("W")), None);
        gates.insert((p("producer"), Action::Produce, t("T")), None);
        gates.insert((p("p1"), Action::Read, t("T")), Some(GateExpr::Via(t("W"))));
        gates.insert((p("p2"), Action::Read, t("T")), None);
        gates.insert((p("w_eater"), Action::Consume, t("W")), None);
        gates.insert((p("w_eater"), Action::Produce, t("W")), None);
        gates.insert((p("w_eater"), Action::Read, t("T")), Some(GateExpr::Exists(t("W"))));
        gates.insert(
            (p("counter"), Action::Read, t("T")),
            Some(GateExpr::Count { ty: t("W"), op: CountOp::Ge, n: 2 }),
        );
        Domain {
            types: BTreeSet::from([t("S"), t("W"), t("T")]),
            preconditions: BTreeMap::from([
                (t("W"), BTreeSet::from([t("S")])),
                (t("T"), BTreeSet::from([t("W"), t("S")])),
            ]),
            gates,
            seed_authority: BTreeSet::from([(p("external"), t("S"))]),
        }
    }

    fn tid(s: &str) -> TokenId {
        TokenId(s.into())
    }
    fn tyn(s: &str) -> TypeName {
        TypeName(s.into())
    }
    fn par(s: &str) -> ParticipantName {
        ParticipantName(s.into())
    }

    fn token(id: &str, ty: &str, by: &str, deps: &[&str]) -> Token {
        Token {
            id: tid(id),
            ty: tyn(ty),
            thing_ref: ThingRef::none(),
            produced_by: par(by),
            depends_on: deps.iter().map(|s| tid(s)).collect(),
            produced_at: "2026-01-01T00:00:00Z".into(),
        }
    }

    // —— §4.2 区分性场景核心：via 的实例级溯源敏感性 ——
    #[test]
    fn via_is_instance_sensitive() {
        let d = domain();
        // State A：W₁ 与 T₁（T₁.depends_on=[W₁,S₁]）都在 Counter
        let mut state = CounterState::new();
        state.counter.insert(tid("S1"), token("S1", "S", "external", &[]));
        state.counter.insert(tid("W1"), token("W1", "W", "producer", &["S1"]));
        state.counter.insert(tid("S2"), token("S2", "S", "external", &[]));
        state.counter.insert(tid("W2"), token("W2", "W", "producer", &["S2"]));
        let t1 = token("T1", "T", "producer", &["W1", "S1"]);
        state.counter.insert(tid("T1"), t1.clone());

        // State A：p1 读 T₁ 通过（见证 W₁ 在 Counter 上 ∧ W₁ ∈ T₁.depends_on）
        assert_eq!(rule_r(&d, &state, &par("p1"), &tid("T1")), Ok(t1.clone()));
        // p2（无条件）通过
        assert!(rule_r(&d, &state, &par("p2"), &tid("T1")).is_ok());

        // State B：W₁ 被消费（W₂ 仍在 Counter，但 W₂ ∉ T₁.depends_on）
        state.apply(Effect::Consumed(ArchiveEntry {
            token: token("W1", "W", "producer", &["S1"]),
            exhausted_by: par("w_eater"),
            exhausted_at: "2026-01-01T00:00:01Z".into(),
        }));
        match rule_r(&d, &state, &par("p1"), &tid("T1")) {
            Err(RuleError::GateConditionFailed { expr, .. }) => {
                assert_eq!(expr, GateExpr::Via(tyn("W")));
            }
            other => panic!("State B 下 p1 的读取应因 via 见证失效被拒，实际：{other:?}"),
        }
        // p2 不受影响（类型级存在性 ≠ 实例级见证）
        assert!(rule_r(&d, &state, &par("p2"), &tid("T1")).is_ok());
        // exists 条件（w_eater 读 T）：W₂ 在 Counter → 通过
        assert!(rule_r(&d, &state, &par("w_eater"), &tid("T1")).is_ok());
    }

    // —— GateExpr 原子语义 ——
    #[test]
    fn gate_expr_atoms() {
        let mut state = CounterState::new();
        state.counter.insert(tid("W1"), token("W1", "W", "p", &[]));
        state.counter.insert(tid("W2"), token("W2", "W", "p", &[]));
        let target = token("T1", "T", "p", &["W1"]);

        assert!(GateExpr::Via(tyn("W")).evaluate(&target, &state)); // W1 在且在 depends_on
        assert!(GateExpr::Exists(tyn("W")).evaluate(&target, &state));
        assert!(!GateExpr::Exists(tyn("S")).evaluate(&target, &state));
        assert!(GateExpr::Count { ty: tyn("W"), op: CountOp::Ge, n: 2 }.evaluate(&target, &state));
        assert!(GateExpr::Count { ty: tyn("W"), op: CountOp::Eq, n: 2 }.evaluate(&target, &state));
        assert!(!GateExpr::Count { ty: tyn("W"), op: CountOp::Eq, n: 3 }.evaluate(&target, &state));
        assert!(GateExpr::Not(Box::new(GateExpr::Exists(tyn("S")))).evaluate(&target, &state));
        let and = GateExpr::And(
            Box::new(GateExpr::Via(tyn("W"))),
            Box::new(GateExpr::Count { ty: tyn("W"), op: CountOp::Ge, n: 1 }),
        );
        assert!(and.evaluate(&target, &state));
        let or = GateExpr::Or(
            Box::new(GateExpr::Exists(tyn("S"))),
            Box::new(GateExpr::Via(tyn("W"))),
        );
        assert!(or.evaluate(&target, &state));
    }

    // —— Rule-S ——
    #[test]
    fn rule_s_semantics() {
        let d = domain();
        // 权威外参与者被拒
        assert_eq!(
            rule_s(&d, &par("producer"), &tyn("S"), ThingRef::none(), tid("x"), "now".into()),
            Err(RuleError::SeedAuthorityDenied { participant: par("producer"), ty: tyn("S") })
        );
        // 权威内通过；豁免前置条件（S 无前置无所谓，W 有前置也被豁免的等价小域在黄金测试覆盖）
        let tk = rule_s(&d, &par("external"), &tyn("S"), ThingRef::some("need-1"), tid("x"), "now".into()).unwrap();
        assert_eq!(tk.depends_on, Vec::<TokenId>::new());
        assert_eq!(tk.thing_ref, ThingRef::some("need-1"));
        assert_eq!(tk.produced_by, par("external"));
    }

    // —— Rule-P ——
    #[test]
    fn rule_p_semantics() {
        let d = domain();
        let mut state = CounterState::new();
        state.counter.insert(tid("S1"), token("S1", "S", "external", &[]));
        state.counter.insert(tid("W1"), token("W1", "W", "producer", &["S1"]));

        // 静态 gate 拒绝：p1 无 produce gate
        assert!(matches!(
            rule_p(&d, &state, &par("p1"), &tyn("W"), &[], ThingRef::none(), tid("n"), "now".into()),
            Err(RuleError::StaticGateDenied { .. })
        ));
        // 前置类型不匹配：W 需要 {S}，给了 {W}
        assert!(matches!(
            rule_p(&d, &state, &par("producer"), &tyn("W"), &[tid("W1")], ThingRef::none(), tid("n"), "now".into()),
            Err(RuleError::PreconditionMismatch { .. })
        ));
        // 前驱不在 Counter
        assert!(matches!(
            rule_p(&d, &state, &par("producer"), &tyn("W"), &[tid("S9")], ThingRef::none(), tid("n"), "now".into()),
            Err(RuleError::PredecessorNotOnCounter { .. })
        ));
        // 前驱重复
        assert!(matches!(
            rule_p(&d, &state, &par("producer"), &tyn("T"), &[tid("W1"), tid("W1")], ThingRef::none(), tid("n"), "now".into()),
            Err(RuleError::DuplicatePredecessor { .. })
        ));
        // 多前驱精确匹配：T 需要 {W, S}
        let t1 = rule_p(
            &d, &state, &par("producer"), &tyn("T"),
            &[tid("W1"), tid("S1")], ThingRef::none(), tid("T1"), "now".into(),
        ).unwrap();
        assert_eq!(t1.depends_on, vec![tid("W1"), tid("S1")]); // 保留调用方顺序
        // 前驱数量短缺（T 需要 2 个前驱，只给 1 个）
        assert!(matches!(
            rule_p(&d, &state, &par("producer"), &tyn("T"), &[tid("W1")], ThingRef::none(), tid("n"), "now".into()),
            Err(RuleError::PreconditionMismatch { .. })
        ));
        // Rule-P 不动前驱：成功后前驱仍在（Effect 应用前检查 state 不变由类型保证；
        // 这里验证 apply 之后前驱仍在）
        state.apply(Effect::Produced(t1.clone()));
        assert!(state.counter.contains_key(&tid("W1")));
        assert!(state.counter.contains_key(&tid("S1")));
        assert_eq!(state.counter.len(), 3);
    }

    // —— Rule-R / Rule-E ——
    #[test]
    fn rule_r_and_e_semantics() {
        let d = domain();
        let mut state = CounterState::new();
        state.counter.insert(tid("W1"), token("W1", "W", "producer", &[]));

        // token 不在 Counter
        assert_eq!(
            rule_r(&d, &state, &par("producer"), &tid("W9")),
            Err(RuleError::TokenNotOnCounter { token_id: tid("W9") })
        );
        // 静态 gate 拒绝：producer 无 Consume W
        assert!(matches!(
            rule_e(&d, &state, &par("producer"), &tid("W1"), "now".into()),
            Err(RuleError::StaticGateDenied { .. })
        ));
        // Rule-R 非破坏性
        let before = state.clone();
        let got = rule_r(&d, &state, &par("producer"), &tid("W1")).unwrap();
        assert_eq!(got, state.counter[&tid("W1")]);
        assert_eq!(state, before); // 逐字节不变
        // Rule-E：移除 + 归档，无级联
        let entry = rule_e(&d, &state, &par("w_eater"), &tid("W1"), "t2".into()).unwrap();
        state.apply(Effect::Consumed(entry.clone()));
        assert!(!state.counter.contains_key(&tid("W1")));
        assert_eq!(state.archive.len(), 1);
        assert_eq!(state.archive[0].exhausted_by, par("w_eater"));
        assert_eq!(state.archive[0].token.id, tid("W1"));
        // 已归档 token 不可再读
        assert_eq!(
            rule_r(&d, &state, &par("producer"), &tid("W1")),
            Err(RuleError::TokenNotOnCounter { token_id: tid("W1") })
        );
    }

    // —— 良构校验 ——
    #[test]
    fn validate_domain_catches() {
        let mut d = domain();
        // 引用未声明类型（gate 条件内）
        d.gates.insert(
            (par("p3"), Action::Read, tyn("T")),
            Some(GateExpr::Via(tyn("X"))),
        );
        let errs = validate_domain(&d).unwrap_err();
        assert!(matches!(
            errs.as_slice(),
            [DomainError::UndeclaredType { referenced_by: "gate_condition", ty }] if *ty == tyn("X")
        ));
        // produces 携带条件
        let d2 = {
            let mut d = domain();
            d.gates.insert((par("producer"), Action::Produce, tyn("T")), Some(GateExpr::Exists(tyn("W"))));
            d
        };
        assert!(matches!(
            validate_domain(&d2),
            Err(es) if matches!(es.as_slice(), [DomainError::ProduceWithCondition { .. }])
        ));
        // 良构域通过
        assert!(validate_domain(&domain()).is_ok());
    }

    // —— Rule-W：enabled_productions ——
    #[test]
    fn enabled_productions_semantics() {
        let d = domain();
        let mut state = CounterState::new();
        state.counter.insert(tid("S1"), token("S1", "S", "external", &[]));

        let mut opts = enabled_productions(&d, &state, &par("producer"));
        opts.sort_by(|a, b| a.ty.cmp(&b.ty));
        // W（前置 {S} 满足，见证 S1）+ T（需要 {W,S}，W 缺失 → 不可满足）
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].ty, tyn("W"));
        assert_eq!(opts[0].witness_predecessors, vec![tid("S1")]);

        // 补齐 W 后 T 亦可满足（排序后 T 在前）
        state.counter.insert(tid("W1"), token("W1", "W", "producer", &["S1"]));
        let mut opts = enabled_productions(&d, &state, &par("producer"));
        opts.sort_by(|a, b| a.ty.cmp(&b.ty));
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].ty, tyn("T"));
        assert_eq!(opts[0].witness_predecessors.len(), 2);
        assert_eq!(opts[1].ty, tyn("W"));
    }

    // —— Rule-T：termination_holds（论文 rev1 §1.4）——
    #[test]
    fn termination_semantics() {
        let d = domain();
        let mut state = CounterState::new();
        // Counter 非空 → 未终止
        state.counter.insert(tid("S1"), token("S1", "S", "external", &[]));
        assert!(!termination_holds(&d, &state));
        // Counter 空 → 终止：本域全部 produce gate 目标类型均有前置规则，
        // Counter = ∅ 时 Rule-P-enabled 条件 (3) 必然失败，无参与者 enabled
        state.counter.clear();
        assert!(termination_holds(&d, &state));
    }

    // —— 源类型生产约束（论文 rev1 §1.4 / §3.1 WF-3 / A.5，回滚 D15）——
    #[test]
    fn source_type_production_excluded() {
        // 含源类型 produce gate 的域（analyst: produces requirement，
        // requirement 无前置规则）：ill-formed，声明期拒绝
        let src_domain = Domain {
            types: BTreeSet::from([tyn("requirement"), tyn("design")]),
            preconditions: BTreeMap::from([(tyn("design"), BTreeSet::from([tyn("requirement")]))]),
            gates: BTreeMap::from([
                ((par("analyst"), Action::Produce, tyn("requirement")), None),
                ((par("architect"), Action::Produce, tyn("design")), None),
            ]),
            seed_authority: BTreeSet::new(),
        };
        assert!(is_source_type(&src_domain, &tyn("requirement")));
        assert!(!is_source_type(&src_domain, &tyn("design")));
        assert!(matches!(
            validate_domain(&src_domain),
            Err(es) if matches!(
                es.as_slice(),
                [DomainError::SourceTypeProduceGate { participant, ty }]
                    if *participant == par("analyst") && *ty == tyn("requirement")
            )
        ));
        // 运行时防御：即使绕过声明期校验直接构造 Domain，Rule-P 仍拒绝源类型生产
        assert_eq!(
            rule_p(
                &src_domain, &CounterState::new(), &par("analyst"), &tyn("requirement"),
                &[], ThingRef::none(), tid("n"), "now".into(),
            ),
            Err(RuleError::SourceTypeProductionDenied {
                participant: par("analyst"),
                ty: tyn("requirement"),
            })
        );
        // Rule-W 服务：源类型不出现在生产选项中（Rule-P 前提 Precond(T) ≠ ∅）
        assert!(enabled_productions(&src_domain, &CounterState::new(), &par("analyst")).is_empty());
        // Rule-T：源类型生产不构成 Rule-P-enabled，Counter 空即终止
        // （论文 rev1 软件工程域 t₁₀ 终止论证）
        assert!(termination_holds(&src_domain, &CounterState::new()));
    }

    // —— Effect 不变量 ——
    #[test]
    fn apply_invariants() {
        let mut state = CounterState::new();
        let s1 = token("S1", "S", "external", &[]);
        state.apply(Effect::Seeded(s1.clone()));
        assert_eq!(state.counter[&tid("S1")], s1);

        let w1 = token("W1", "W", "producer", &["S1"]);
        state.apply(Effect::Produced(w1.clone()));
        assert_eq!(state.counter.len(), 2);

        let entry = ArchiveEntry { token: w1.clone(), exhausted_by: par("w_eater"), exhausted_at: "t".into() };
        state.apply(Effect::Consumed(entry));
        assert_eq!(state.counter.len(), 1);
        assert_eq!(state.archive.len(), 1);
        assert_eq!(state.archive[0].token, w1);
    }

    // —— 线上 JSON 格式（design.md §6.2；serde derive 契约）——
    #[test]
    fn token_json_wire_format() {
        let tk = token("01J9ZQ5T", "design", "architect", &["01J9ZQ3M"]);
        let json = serde_json::to_string(&tk).unwrap();
        assert!(json.contains(r#""id":"01J9ZQ5T""#), "newtype 透明序列化：{json}");
        assert!(json.contains(r#""type":"design""#), "{json}");
        assert!(json.contains(r#""produced_by":"architect""#), "{json}");
        assert!(json.contains(r#""depends_on":["01J9ZQ3M"]"#), "{json}");
        assert!(json.contains(r#""thing_ref":null"#), "∅ 表示为 null：{json}");
        // 往返一致
        let back: Token = serde_json::from_str(&json).unwrap();
        assert_eq!(back, tk);
    }
}

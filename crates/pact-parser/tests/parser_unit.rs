//! pact-parser 单元测试：文法、脱糖、错误分类与位置报告。

use pact_core::{Action, CountOp, DomainError, GateExpr};
use pact_parser::{load_domain, parse_raw, Problem};
use std::collections::BTreeSet;

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

fn tyn(s: &str) -> pact_core::TypeName {
    pact_core::TypeName(s.into())
}
fn par(s: &str) -> pact_core::ParticipantName {
    pact_core::ParticipantName(s.into())
}

#[test]
fn parse_full_lifecycle_domain() {
    let d = load_domain(LIFECYCLE).expect("良构域应解析成功");
    assert_eq!(d.types.len(), 5);
    assert_eq!(d.preconditions.len(), 4);
    // 论文 rev1：operator 的 reads test_report 无条件（via 子句已移除，
    // requirement 源类型由 seed_authority 独占注入）
    assert_eq!(
        d.gates
            .get(&(par("operator"), Action::Read, tyn("test_report"))),
        Some(&None)
    );
    assert_eq!(
        d.gates
            .get(&(par("stakeholder"), Action::Consume, tyn("deployment"))),
        Some(&None)
    );
    assert!(d
        .seed_authority
        .contains(&(par("external_stakeholder"), tyn("requirement"))));
    // 前置图内容抽查
    assert_eq!(
        d.preconditions.get(&tyn("test_report")),
        Some(&BTreeSet::from([tyn("implementation")]))
    );
}

#[test]
fn via_sugar_equals_when_form() {
    let sugar = "types { b; c }
        preconditions { c → b }
        gates { a: reads b via c }
        seed_authority { }";
    let when = "types { b; c }
        preconditions { c → b }
        gates { a: reads b when via c }
        seed_authority { }";
    assert_eq!(load_domain(sugar).unwrap(), load_domain(when).unwrap());
}

#[test]
fn ascii_arrow_equivalent() {
    let uni = "types { a; c } preconditions { a → c } gates { p: reads a } seed_authority { }";
    let ascii = "types { a; c } preconditions { a -> c } gates { p: reads a } seed_authority { }";
    assert_eq!(load_domain(uni).unwrap(), load_domain(ascii).unwrap());
}

#[test]
fn multi_precondition_brace_form() {
    let src = "types { s; w; t }
        preconditions { s → w; { w + s } → t }
        gates { p: reads w, produces t }
        seed_authority { e: seeds s }";
    let d = load_domain(src).unwrap();
    assert_eq!(
        d.preconditions.get(&tyn("t")),
        Some(&BTreeSet::from([tyn("w"), tyn("s")]))
    );
}

#[test]
fn gate_expr_precedence_and_forms() {
    let src = "types { a; b; c }
        preconditions { a → c }
        gates {
            p: reads c when count a >= 1 and exists b or not count b = 2;
            q: reads c when not exists b
        }
        seed_authority { }";
    let d = load_domain(src).unwrap();
    // 优先级 not > and > or：（count a >= 1 and exists b) or (not count b = 2)
    let expected = GateExpr::Or(
        Box::new(GateExpr::And(
            Box::new(GateExpr::Count {
                ty: tyn("a"),
                op: CountOp::Ge,
                n: 1,
            }),
            Box::new(GateExpr::Exists(tyn("b"))),
        )),
        Box::new(GateExpr::Not(Box::new(GateExpr::Count {
            ty: tyn("b"),
            op: CountOp::Eq,
            n: 2,
        }))),
    );
    assert_eq!(
        d.gates.get(&(par("p"), Action::Read, tyn("c"))),
        Some(&Some(expected))
    );
    // not 只约束紧随的原子
    let expected_q = GateExpr::Not(Box::new(GateExpr::Exists(tyn("b"))));
    assert_eq!(
        d.gates.get(&(par("q"), Action::Read, tyn("c"))),
        Some(&Some(expected_q))
    );
}

#[test]
fn parenthesized_gate_expr_accepted() {
    // 括号分组（D16 扩展族）：And(Or(a,b), c) 与左结合解析不同
    let src = "types { a; b; c }
        preconditions { a → c }
        gates { p: reads c when (a or b) and exists c }
        seed_authority { }"
        .replace("a or b", "exists a or exists b");
    let d = load_domain(&src).unwrap();
    let expected = GateExpr::And(
        Box::new(GateExpr::Or(
            Box::new(GateExpr::Exists(tyn("a"))),
            Box::new(GateExpr::Exists(tyn("b"))),
        )),
        Box::new(GateExpr::Exists(tyn("c"))),
    );
    assert_eq!(
        d.gates.get(&(par("p"), Action::Read, tyn("c"))),
        Some(&Some(expected))
    );
}

#[test]
fn emit_roundtrip_identity() {
    // 回写-再解析恒等：load(emit(load(src))) == load(src)
    let cases: Vec<&str> = vec![
        LIFECYCLE,
        "types { s; w; t }
        preconditions { s → w; { w + s } → t }
        gates { p: reads w, produces t }
        seed_authority { e: seeds s }",
        "types { a; b; c }
        preconditions { a → c }
        gates {
            p: reads c when count a >= 1 and exists b or not count b = 2;
            q: reads c when (exists a or exists b) and not via c;
            r: reads c via b;
            s: consumes c when not not exists b
        }
        seed_authority { }",
    ];
    for src in cases {
        let d = load_domain(src).unwrap();
        let text = pact_parser::emit_domain(&d);
        let re =
            load_domain(&text).unwrap_or_else(|e| panic!("回写文本应可解析：{e:?}\n----\n{text}"));
        assert_eq!(d, re, "回写-再解析应恒等：\n----\n{text}");
    }
}

#[test]
fn errors_are_well_classified() {
    // §1.4.4：前置唯一性违例（同一 rhs 两条规则）
    let dup_rhs = "types { a; b; c }
        preconditions { a → c; b → c }
        gates { p: reads a } seed_authority { }";
    assert!(matches!(
        load_domain(dup_rhs),
        Err(Problem::WellFormedness(es)) if matches!(es.as_slice(), [DomainError::DuplicatePrecondRule { ty }] if *ty == tyn("c"))
    ));
    // 引用封闭性
    let undeclared = "types { a } preconditions { a → b } gates { p: reads a } seed_authority { }";
    assert!(matches!(
        load_domain(undeclared),
        Err(Problem::WellFormedness(es)) if matches!(
            es.as_slice(),
            [DomainError::UndeclaredType { referenced_by, .. }] if referenced_by == "preconditions"
        )
    ));
    // produces 携带条件（§3.1 Note）
    let produce_cond = "types { a; b }
        preconditions { a → b }
        gates { p: produces b when exists a }
        seed_authority { }";
    assert!(matches!(
        load_domain(produce_cond),
        Err(Problem::WellFormedness(es)) if matches!(es.as_slice(), [DomainError::ProduceWithCondition { .. }])
    ));
    // 源生产约束（论文 rev1 §3.1 WF-3）：produces 指向源类型（无前置规则）
    let src_produce = "types { a; b }
        preconditions { a → b }
        gates { p: produces a, produces b }
        seed_authority { }";
    assert!(matches!(
        load_domain(src_produce),
        Err(Problem::WellFormedness(es)) if matches!(es.as_slice(), [DomainError::SourceTypeProduceGate { participant, ty }] if *participant == par("p") && *ty == tyn("a"))
    ));
    // 子句重复
    let dup_clause = "types { a; b }
        preconditions { a → b }
        gates { p: reads a, reads a }
        seed_authority { }";
    assert!(matches!(
        load_domain(dup_clause),
        Err(Problem::WellFormedness(es)) if matches!(es.as_slice(), [DomainError::DuplicateGateClause { .. }])
    ));
    // 良构错误全部收集：未声明类型 ×2
    let multi =
        "types { a } preconditions { a → b } gates { p: reads c } seed_authority { q: seeds d }";
    match load_domain(multi) {
        Err(Problem::WellFormedness(es)) => assert_eq!(es.len(), 3, "应收集全部错误：{es:?}"),
        other => panic!("应报良构错误，实际 {other:?}"),
    }
}

#[test]
fn rev2_wellformedness_rejections() {
    // §1.4.4 LHS Uniqueness：单一规则内前驱类型重复（{ a + a } → c）
    let dup_lhs = "types { a; c }
        preconditions { { a + a } → c }
        gates { p: reads a } seed_authority { }";
    assert!(matches!(
        load_domain(dup_lhs),
        Err(Problem::WellFormedness(es)) if matches!(
            es.as_slice(),
            [DomainError::DuplicatePrecondLhs { ty, lhs_ty }] if *ty == tyn("c") && *lhs_ty == tyn("a")
        )
    ));
    // §1.4.4 Non-Empty LHS：空花括号 lhs 语法层拒绝（源类型由规则缺席定义）
    let empty_lhs = "types { a; b }
        preconditions { { } → b }
        gates { p: reads a } seed_authority { }";
    match load_domain(empty_lhs) {
        Err(Problem::Syntax(es)) => {
            assert!(es[0].message.contains("Non-Empty LHS"), "{}", es[0].message)
        }
        other => panic!("空 LHS 应报语法错误，实际 {other:?}"),
    }
}

#[test]
fn rev2_pos_nat_literal_enforced() {
    // §1.7.3 PosNat：count 字面量必须匹配 [1-9][0-9]*——0 非法
    let zero = "types { a; b }
        preconditions { a → b }
        gates { p: reads b when count a >= 0 }
        seed_authority { }";
    match load_domain(zero) {
        Err(Problem::Syntax(es)) => {
            assert!(es[0].message.contains("PosNat"), "{}", es[0].message)
        }
        other => panic!("count 0 应报语法错误，实际 {other:?}"),
    }
    // 前导零同样非法（01 不是规范 PosNat）
    let leading_zero = "types { a; b }
        preconditions { a → b }
        gates { p: reads b when count a = 01 }
        seed_authority { }";
    match load_domain(leading_zero) {
        Err(Problem::Syntax(es)) => {
            assert!(es[0].message.contains("前导零"), "{}", es[0].message)
        }
        other => panic!("前导零应报语法错误，实际 {other:?}"),
    }
    // 正整数（含多位）接受
    let ok = "types { a; b }
        preconditions { a → b }
        gates { p: reads b when count a >= 12 }
        seed_authority { }";
    let d = load_domain(ok).unwrap();
    assert_eq!(
        d.gates.get(&(par("p"), Action::Read, tyn("b"))),
        Some(&Some(GateExpr::Count {
            ty: tyn("a"),
            op: CountOp::Ge,
            n: 12,
        }))
    );
}

#[test]
fn rev2_self_loop_precondition_accepted() {
    // §1.4.4：自环 a → a 合法（Precond(a) = {a} 非空，a 非源类型）
    let src = "types { a }
        preconditions { a → a }
        gates { p: reads a, consumes a }
        seed_authority { ext: seeds a }";
    let d = load_domain(src).unwrap();
    assert_eq!(
        d.preconditions.get(&tyn("a")),
        Some(&BTreeSet::from([tyn("a")]))
    );
}

#[test]
fn syntax_errors_with_location() {
    // 缺段
    let missing = "types { a } preconditions { } gates { }";
    match parse_raw(missing) {
        Err(es) => {
            assert!(
                es[0].message.contains("seed_authority"),
                "{}",
                es[0].message
            );
        }
        Ok(_) => panic!("缺 seed_authority 段应报语法错误"),
    }
    // 坏箭头（第二行；"-" 单独出现是词法错误）
    let bad_arrow = "types { a; c }\npreconditions { a - c }\ngates { } seed_authority { }";
    match parse_raw(bad_arrow) {
        Err(es) => {
            assert_eq!(es[0].span.line, 2, "错误应定位在第 2 行：{:?}", es[0].span);
            assert!(es[0].message.contains("无法识别"), "{}", es[0].message);
        }
        Ok(_) => panic!("坏箭头应报错"),
    }
    // 无法识别的符号（收集全部词法错误）
    let bad_lex =
        "types { a; c } preconditions { a → c } gates { p: reads a @ } seed_authority { }";
    match parse_raw(bad_lex) {
        Err(es) => assert_eq!(es.len(), 1, "@ 应被报为词法错误"),
        Ok(_) => panic!("@ 应被拒绝"),
    }
    // types 段为空
    let empty_types = "types { } preconditions { } gates { } seed_authority { }";
    assert!(parse_raw(empty_types).is_err());
}

#[test]
fn lenient_forms_accepted() {
    // 空段（除 types）+ 段内尾分号 + 行注释 + 同参与者多段合并
    let src = "// 领域注释
        types { a; b; }
        preconditions { a → b; }
        gates {
            p: reads a;
            p: consumes b;
        }
        seed_authority { ext: seeds a; }";
    let d = load_domain(src).unwrap();
    assert_eq!(d.gates.len(), 2);
    assert!(d.gates.contains_key(&(par("p"), Action::Consume, tyn("b"))));
    // 重复类型声明被拒（语法层）
    let dup_ty = "types { a; a } preconditions { } gates { } seed_authority { }";
    assert!(parse_raw(dup_ty).is_err());
}

#[test]
fn parse_raw_exposes_intermediate_form() {
    let raw = parse_raw(LIFECYCLE).unwrap();
    assert_eq!(raw.types.len(), 5);
    assert_eq!(raw.precond_rules.len(), 4);
    // 同一参与者多个子句聚合在其 gate_def 下
    let architect = raw
        .gate_defs
        .iter()
        .find(|(p, _)| *p == par("architect"))
        .unwrap();
    assert_eq!(architect.1.len(), 3);
}

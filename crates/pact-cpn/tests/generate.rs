use pact_core::TypeName;
use pact_cpn::{generate, CpnError, CpnOptions};
use pact_parser::load_domain;
use std::collections::BTreeMap;

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

fn tyn(s: &str) -> TypeName {
    TypeName(s.into())
}

fn opts(bounds: &[(&str, u64)]) -> CpnOptions {
    CpnOptions {
        instance: "default".into(),
        bounds: bounds
            .iter()
            .map(|(t, n)| (tyn(t), *n))
            .collect::<BTreeMap<_, _>>(),
        page_name: None,
    }
}

/// 极简 XML 良构检查：标签配对、自闭合与文本合法性。
fn assert_well_formed(xml: &str) {
    let mut stack: Vec<String> = Vec::new();
    let bytes = xml.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let start = i;
        let end = xml[start..]
            .find('>')
            .map(|p| start + p)
            .expect("未闭合的标签");
        let tag = &xml[start + 1..end];
        i = end + 1;
        if tag.starts_with('?') || tag.starts_with('!') {
            continue;
        }
        if let Some(name) = tag.strip_prefix('/') {
            let name = name.trim();
            let top = stack.pop().expect("多余的闭合标签");
            assert_eq!(top, name, "标签不匹配");
        } else if tag.trim_end().ends_with('/') {
            continue;
        } else {
            let name = tag.split_whitespace().next().unwrap();
            stack.push(name.to_string());
        }
    }
    assert!(stack.is_empty(), "存在未闭合标签：{stack:?}");
}

#[test]
fn lifecycle_structure_matches_paper_encoding() {
    let domain = load_domain(LIFECYCLE).unwrap();
    let model = generate(&domain, &opts(&[])).unwrap();
    // 5 类型库所 + 1 Archive；无条件 ⇒ 无互补库所。
    assert_eq!(model.stats.places, 6);
    assert_eq!(model.stats.complementary_places, 0);
    assert_eq!(model.stats.production_transitions, 4);
    assert_eq!(model.stats.read_transitions, 4);
    assert_eq!(model.stats.consume_transitions, 5);
    assert_eq!(model.stats.seed_transitions, 1);
    assert_eq!(model.stats.transitions, 14);
    // 生产 3 弧/迁移，读 2，消费 2，种子 1。
    assert_eq!(model.stats.arcs, 12 + 8 + 10 + 1);
    assert!(model.xml.contains("p_requirement"));
    assert!(model.xml.contains("p_archive"));
    assert!(model.xml.contains("PACT_ARCHIVE"));
    assert!(model.xml.contains("pact_next_id"));
    // 源类型不得有生产迁移。
    assert!(!model.xml.contains("produces"));
    assert!(!model.xml.contains("t_architect_requirement_prod"));
    assert_well_formed(&model.xml);
}

#[test]
fn via_uses_witness_test_arc_and_guard() {
    let domain = load_domain(
        r#"
types { a; b }
preconditions { a → b }
gates { p: reads b via a; q: produces b }
seed_authority { s: seeds a }
"#,
    )
    .unwrap();
    let model = generate(&domain, &opts(&[])).unwrap();
    assert!(model.xml.contains("t_p_b_read"));
    assert!(
        model
            .xml
            .contains("List.exists (fn z =&gt; z = #1 y1) (#4 x)"),
        "via 应生成溯源见证 guard"
    );
    assert_well_formed(&model.xml);
}

#[test]
fn not_exists_requires_bound_and_builds_complement() {
    let domain = load_domain(
        r#"
types { a; b }
preconditions { a → b }
gates { p: reads b when not exists a; q: produces b }
seed_authority { s: seeds a }
"#,
    )
    .unwrap();

    assert_eq!(
        generate(&domain, &opts(&[])).unwrap_err(),
        CpnError::MissingBound { ty: tyn("a") }
    );

    let model = generate(&domain, &opts(&[("a", 2)])).unwrap();
    assert_eq!(model.stats.complementary_places, 1);
    assert!(model.xml.contains("p_a_not"));
    assert!(model.xml.contains("2`0"), "互补库所初值应为上界");
    // 种子 s 从互补库所取回 1 个 token，维持 M(p_a) + M(p_a_not) = 上界。
    assert!(model.xml.contains("1`0"), "互补库所需维持不变量");
    assert_well_formed(&model.xml);
}

#[test]
fn not_exists_on_target_is_disabled() {
    let domain = load_domain(
        r#"
types { a; b }
preconditions { a → b }
gates { p: reads b when not exists b; q: produces b }
seed_authority { s: seeds a }
"#,
    )
    .unwrap();
    let model = generate(&domain, &opts(&[("b", 1)])).unwrap();
    // 目标 token 自身存在 ⇒ not exists b 恒假；保留迁移但 guard=false。
    assert_eq!(model.stats.read_transitions, 0);
    assert_eq!(model.stats.disabled_transitions, 1);
    assert!(model.xml.contains("false"));
    assert_well_formed(&model.xml);
}

#[test]
fn or_expands_into_multiple_transitions() {
    let domain = load_domain(
        r#"
types { a; b; c }
preconditions { a → b }
gates { p: reads b when exists a or exists c; q: produces b }
seed_authority { s: seeds a }
"#,
    )
    .unwrap();
    let model = generate(&domain, &opts(&[])).unwrap();
    assert_eq!(model.stats.read_transitions, 2);
    assert!(model.xml.contains("t_p_b_read_c1"));
    assert!(model.xml.contains("t_p_b_read_c2"));
    assert_well_formed(&model.xml);
}

#[test]
fn count_ge_two_binds_distinct_witnesses() {
    let domain = load_domain(
        r#"
types { a; b }
preconditions { a → b }
gates { p: reads b when count a >= 2; q: produces b }
seed_authority { s: seeds a }
"#,
    )
    .unwrap();
    let model = generate(&domain, &opts(&[])).unwrap();
    assert_eq!(model.stats.read_transitions, 1);
    assert!(model.xml.contains("var y2: PACT_TOKEN;"));
    assert!(model.xml.contains("#1 y1 &lt;&gt; #1 y2"));
    assert_well_formed(&model.xml);
}

#[test]
fn count_eq_requires_bound_and_uses_complement() {
    let domain = load_domain(
        r#"
types { a; b }
preconditions { a → b }
gates { p: reads b when count a = 2; q: produces b }
seed_authority { s: seeds a }
"#,
    )
    .unwrap();
    assert_eq!(
        generate(&domain, &opts(&[])).unwrap_err(),
        CpnError::MissingBound { ty: tyn("a") }
    );
    let model = generate(&domain, &opts(&[("a", 3)])).unwrap();
    assert_eq!(model.stats.complementary_places, 1);
    // count = 2 且上界 3 ⇒ 互补库所保留 1 个 token 以排除第 3 个。
    assert!(model.xml.contains("1`0"));
    assert!(model.xml.contains("#1 y1 &lt;&gt; #1 y2"));
    assert_well_formed(&model.xml);
}

#[test]
fn count_on_target_does_not_over_require() {
    let domain = load_domain(
        r#"
types { a; b }
preconditions { a → b }
gates { p: reads b when count b >= 2; q: produces b }
seed_authority { s: seeds a }
"#,
    )
    .unwrap();
    let model = generate(&domain, &opts(&[])).unwrap();
    // 目标 token 自身计入 count b，只需 1 个额外见证。
    assert!(!model.xml.contains("var y2: PACT_TOKEN;"));
    assert!(model.xml.contains("var y1: PACT_TOKEN;"));
    assert_well_formed(&model.xml);
}

#[test]
fn oversized_count_literal_is_rejected() {
    // 测试弧数量与 count 字面量线性相关，须限制恶意/误用声明的展开规模。
    let domain = load_domain(
        r#"
types { a; b }
preconditions { a → b }
gates { p: reads b when count a >= 1025; q: produces b }
seed_authority { s: seeds a }
"#,
    )
    .unwrap();
    assert!(matches!(
        generate(&domain, &opts(&[])).unwrap_err(),
        CpnError::ConditionTooComplex { .. }
    ));
}

#[test]
fn negated_via_is_rejected() {
    let domain = load_domain(
        r#"
types { a; b }
preconditions { a → b }
gates { p: reads b when not via a; q: produces b }
seed_authority { s: seeds a }
"#,
    )
    .unwrap();
    assert_eq!(
        generate(&domain, &opts(&[])).unwrap_err(),
        CpnError::NegatedVia { ty: tyn("a") }
    );
}

#[test]
fn instance_is_namespaced() {
    let domain = load_domain(LIFECYCLE).unwrap();
    let alpha = generate(
        &domain,
        &CpnOptions {
            instance: "alpha".into(),
            ..opts(&[])
        },
    )
    .unwrap();
    let beta = generate(
        &domain,
        &CpnOptions {
            instance: "beta".into(),
            ..opts(&[])
        },
    )
    .unwrap();
    assert!(alpha.xml.contains("i_alpha__p_requirement"));
    assert!(beta.xml.contains("i_beta__p_requirement"));
    assert!(!beta.xml.contains("i_alpha__"));
    assert_eq!(alpha.stats.places, beta.stats.places);
    assert_well_formed(&alpha.xml);
    assert_well_formed(&beta.xml);
}

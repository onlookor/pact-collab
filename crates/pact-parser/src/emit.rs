//! Domain → `.pact` 声明文本（反向序列化）。
//!
//! 用途：组合结果（pact-compose 产出 `Domain` 结构）与引擎恢复路径需要
//! 声明原文存档（设计 §7.1 `DomainLoaded { declaration }`），此处提供
//! 结构到文法的忠实回写。保证回写-再解析恒等（round-trip 测试锁定）：
//! `load_domain(&emit_domain(&d)) == d`（对良构 d）。
//!
//! 文本形态约定（D16 扩展族）：
//! - 多前驱规则一律使用 `{ a + b } → z` 形式；
//! - 条件子句一律使用 `when` 形式（via 糖形式在解析期已脱糖）；
//! - 二元组合子（and/or）的操作数用括号显式分组（文法括号扩展）。

use pact_core::{Action, Domain, GateExpr};

/// 参与者 → 其 gate 子句列表（渲染排序用）。
type ClausesByParticipant<'a> =
    std::collections::BTreeMap<&'a str, Vec<(&'a Action, &'a str, &'a Option<GateExpr>)>>;

/// 把良构 Domain 回写为 `.pact` 声明文本。
pub fn emit_domain(d: &Domain) -> String {
    let mut out = String::new();

    // types 段（BTreeSet 序确定）
    out.push_str("types { ");
    out.push_str(
        &d.types
            .iter()
            .map(|t| t.0.as_str())
            .collect::<Vec<_>>()
            .join("; "),
    );
    out.push_str(" }\n\n");

    // preconditions 段（BTreeMap 序确定；rhs 排序）
    out.push_str("preconditions {\n");
    for (rhs, preds) in &d.preconditions {
        if preds.is_empty() {
            continue; // 防御：空前置集合不构成规则（源类型不入前置图）
        }
        match preds.len() {
            1 => {
                let lhs = preds.iter().next().map(|t| t.0.as_str()).unwrap();
                out.push_str(&format!("    {lhs} → {rhs};\n"));
            }
            _ => {
                let lhs = preds
                    .iter()
                    .map(|t| t.0.as_str())
                    .collect::<Vec<_>>()
                    .join(" + ");
                out.push_str(&format!("    {{ {lhs} }} → {rhs};\n"));
            }
        }
    }
    out.push_str("}\n\n");

    // gates 段：按参与者分组，组内按 (action, 类型名) 排序
    out.push_str("gates {\n");
    let mut by_participant = ClausesByParticipant::new();
    for ((p, action, ty), cond) in &d.gates {
        by_participant
            .entry(p.0.as_str())
            .or_default()
            .push((action, ty.0.as_str(), cond));
    }
    for (p, clauses) in by_participant {
        let rendered = clauses
            .iter()
            .map(|(action, ty, cond)| {
                let verb = match action {
                    Action::Read => "reads",
                    Action::Consume => "consumes",
                    Action::Produce => "produces",
                };
                match cond {
                    Some(expr) => format!("{verb} {ty} when {}", emit_expr(expr)),
                    None => format!("{verb} {ty}"),
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("    {p}: {rendered};\n"));
    }
    out.push_str("}\n\n");

    // seed_authority 段
    out.push_str("seed_authority {\n");
    for (p, ty) in &d.seed_authority {
        out.push_str(&format!("    {}: seeds {};\n", p.0, ty.0));
    }
    out.push_str("}\n");

    out
}

/// GateExpr 文本形式。二元操作数一律括号分组（不依赖优先级，回写无歧义）。
pub fn emit_expr(e: &GateExpr) -> String {
    match e {
        GateExpr::Via(t) => format!("via {t}"),
        GateExpr::Exists(t) => format!("exists {t}"),
        GateExpr::Count { ty, op, n } => match op {
            pact_core::CountOp::Ge => format!("count {ty} >= {n}"),
            pact_core::CountOp::Eq => format!("count {ty} = {n}"),
        },
        GateExpr::And(a, b) => format!("({} and {})", emit_expr(a), emit_expr(b)),
        GateExpr::Or(a, b) => format!("({} or {})", emit_expr(a), emit_expr(b)),
        // not 直接前缀：二元子式已自带括号（"not (x and y)"），原子无需括号
        GateExpr::Not(a) => format!("not {}", emit_expr(a)),
    }
}

//! `declarationDigest` v1：域声明语义接口的规范化与 SHA-256 摘要（FR34 / D49）。
//!
//! 只覆盖影响绑定匹配、可见性与准入判定的语义面：域名、`types`、
//! `preconditions`、`gates`（含条件表达式）与 `seed_authority`。注释、空白、
//! 字段书写顺序、文件路径、实例名与分析结果都不参与。
//!
//! 规范化形式与规范文档（`spec/ref/v1/declaration-digest.md`）一一对应：
//! UTF-8 紧凑 JSON（无缩进、无尾换行），字段顺序固定，集合按字典序展开。
//! 摘要字符串保持 `sha256:<64 小写 hex>`；v1 语义固定在规范文档中，未来变更
//! 必须引入新规范版本与新字段/新编号，禁止在同一字段静默换义。

use pact_core::{CountOp, Domain, GateExpr};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// 摘要对象版本（写入规范化 JSON 的 `v` 字段，也是规范文档的版本）。
pub const DECLARATION_DIGEST_VERSION: u32 = 1;

/// 规范化 JSON 的顶层 `kind`，避免与其他协议的哈希混淆。
pub const DECLARATION_INTERFACE_KIND: &str = "pact.declaration.interface";

#[derive(Serialize)]
struct DeclarationInterface<'a> {
    kind: &'static str,
    v: u32,
    domain: &'a str,
    types: Vec<&'a str>,
    preconditions: Vec<Precondition<'a>>,
    gates: Vec<GateEntry<'a>>,
    seed: Vec<SeedEntry<'a>>,
}

#[derive(Serialize)]
struct Precondition<'a> {
    #[serde(rename = "type")]
    ty: &'a str,
    predecessors: Vec<&'a str>,
}

#[derive(Serialize)]
struct GateEntry<'a> {
    participant: &'a str,
    action: &'a str,
    #[serde(rename = "type")]
    ty: &'a str,
    /// `null` = 无条件；字符串 = 规范化的条件表达式文本。
    condition: Option<String>,
}

#[derive(Serialize)]
struct SeedEntry<'a> {
    participant: &'a str,
    #[serde(rename = "type")]
    ty: &'a str,
}

/// 计算域声明的语义接口摘要（`sha256:<64 小写 hex>`）。
///
/// `domain` 是声明被加载时使用的域名（`load_domain` 的 `name`，CLI 取
/// `--name` 或文件 stem）；它参与摘要，防止同一正文在不同域名下复用摘要。
pub fn declaration_digest(domain: &str, ast: &Domain) -> String {
    let bytes = declaration_interface_json(domain, ast);
    let digest = Sha256::digest(bytes.as_bytes());
    let mut hex = String::with_capacity(7 + 64);
    hex.push_str("sha256:");
    for byte in digest {
        hex.push(char::from(HEX[(byte >> 4) as usize]));
        hex.push(char::from(HEX[(byte & 0x0f) as usize]));
    }
    hex
}

const HEX: [u8; 16] = *b"0123456789abcdef";

/// 规范化 JSON 文本（紧凑、无缩进、无尾换行）。用于调试与 golden 向量。
pub fn declaration_interface_json(domain: &str, ast: &Domain) -> String {
    let mut gates: Vec<GateEntry<'_>> = ast
        .gates
        .iter()
        .map(|((participant, action, ty), condition)| GateEntry {
            participant: participant.0.as_str(),
            action: action.as_str(),
            ty: ty.0.as_str(),
            condition: condition.as_ref().map(condition_text),
        })
        .collect();
    // 规范排序：按 (participant, action, type) 字典序，与 BTreeMap 的
    // (participant, Action 枚举序, type) 不同，需显式重排。
    gates.sort_by(|a, b| (a.participant, a.action, a.ty).cmp(&(b.participant, b.action, b.ty)));

    let interface = DeclarationInterface {
        kind: DECLARATION_INTERFACE_KIND,
        v: DECLARATION_DIGEST_VERSION,
        domain,
        types: ast.types.iter().map(|ty| ty.0.as_str()).collect(),
        preconditions: ast
            .preconditions
            .iter()
            .map(|(ty, predecessors)| Precondition {
                ty: ty.0.as_str(),
                predecessors: predecessors.iter().map(|p| p.0.as_str()).collect(),
            })
            .collect(),
        gates,
        seed: ast
            .seed_authority
            .iter()
            .map(|(participant, ty)| SeedEntry {
                participant: participant.0.as_str(),
                ty: ty.0.as_str(),
            })
            .collect(),
    };
    serde_json::to_string(&interface).expect("规范化 JSON 序列化不应失败")
}

/// 条件表达式的规范文本（规范文档 §3 冻结；类型名文法不含逗号/括号，故无歧义）。
fn condition_text(expr: &GateExpr) -> String {
    match expr {
        GateExpr::Via(ty) => format!("via({})", ty.0),
        GateExpr::Exists(ty) => format!("exists({})", ty.0),
        GateExpr::Count { ty, op, n } => {
            let op = match op {
                CountOp::Ge => ">=",
                CountOp::Eq => "=",
            };
            format!("count({},{op},{n})", ty.0)
        }
        GateExpr::And(left, right) => {
            format!("and({},{})", condition_text(left), condition_text(right))
        }
        GateExpr::Or(left, right) => {
            format!("or({},{})", condition_text(left), condition_text(right))
        }
        GateExpr::Not(inner) => format!("not({})", condition_text(inner)),
    }
}

#[cfg(test)]
mod tests {
    use super::{declaration_digest, declaration_interface_json, DECLARATION_DIGEST_VERSION};

    const BASE: &str = r#"
types { request; design; impl }
preconditions { design → impl }
gates {
  architect: reads design, consumes design, produces impl;
  dev: produces impl
}
seed_authority { client: seeds request }
"#;

    fn base_domain() -> pact_core::Domain {
        pact_parser::load_domain(BASE).expect("BASE 应可解析")
    }

    #[test]
    fn formatting_and_clause_order_do_not_change_digest() {
        let reordered = r#"
// 注释与空白不参与摘要
types {
  impl;
  design;
  request
}
preconditions {

  design -> impl
 }
gates {
  dev: produces impl;
  architect: produces impl, consumes design, reads design
}
seed_authority { client: seeds request }
"#;
        let a = declaration_digest("se_lifecycle", &base_domain());
        let b = declaration_digest(
            "se_lifecycle",
            &pact_parser::load_domain(reordered).expect("重排后的声明应可解析"),
        );
        assert_eq!(a, b);
        assert!(a.starts_with("sha256:"));
        assert_eq!(a.len(), 7 + 64);
        assert!(a[7..]
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn semantic_changes_change_digest() {
        let base = declaration_digest("se_lifecycle", &base_domain());
        assert_ne!(
            base,
            declaration_digest(
                "se_lifecycle",
                &pact_parser::load_domain(
                    "types { request; design; impl; extra }\n\
                     preconditions { design -> impl }\n\
                     gates { architect: reads design, consumes design, produces impl; dev: produces impl }\n\
                     seed_authority { client: seeds request }"
                )
                .unwrap()
            )
        );
        assert_ne!(
            base,
            declaration_digest(
                "se_lifecycle",
                &pact_parser::load_domain(
                    "types { request; design; impl }\n\
                     preconditions { design -> impl }\n\
                     gates { architect: reads design when exists request, consumes design, produces impl; dev: produces impl }\n\
                     seed_authority { client: seeds request }"
                )
                .unwrap()
            )
        );
        assert_ne!(
            base,
            declaration_digest(
                "se_lifecycle",
                &pact_parser::load_domain(
                    "types { request; design; impl }\n\
                     preconditions { design -> impl }\n\
                     gates { architect: reads design, consumes design, produces impl; dev: produces impl }\n\
                     seed_authority { ops: seeds request }"
                )
                .unwrap()
            )
        );
        // 同一正文换域名：摘要必须不同，防止跨域复用。
        assert_ne!(base, declaration_digest("other_domain", &base_domain()));
    }

    #[test]
    fn canonical_json_shape_is_frozen() {
        let json = declaration_interface_json("se_lifecycle", &base_domain());
        assert_eq!(
            json,
            concat!(
                r#"{"kind":"pact.declaration.interface","#,
                r#""v":1,"#,
                r#""domain":"se_lifecycle","#,
                r#""types":["design","impl","request"],"#,
                r#""preconditions":[{"type":"impl","predecessors":["design"]}],"#,
                r#""gates":["#,
                r#"{"participant":"architect","action":"consume","type":"design","condition":null},"#,
                r#"{"participant":"architect","action":"produce","type":"impl","condition":null},"#,
                r#"{"participant":"architect","action":"read","type":"design","condition":null},"#,
                r#"{"participant":"dev","action":"produce","type":"impl","condition":null}],"#,
                r#""seed":[{"participant":"client","type":"request"}]}"#,
            )
        );
        assert_eq!(DECLARATION_DIGEST_VERSION, 1);
    }

    #[test]
    fn condition_text_is_canonical() {
        let domain = pact_parser::load_domain(
            "types { a; b; c }\n\
             preconditions { a -> b }\n\
             gates {\n\
               p: produces b,\n\
                  reads b when via a and not (exists b or count c >= 2),\n\
                  consumes b when count a = 1\n\
             }\n\
             seed_authority { s: seeds a }",
        )
        .expect("条件声明应可解析");
        let json = declaration_interface_json("cond", &domain);
        assert!(
            json.contains(r#""condition":"and(via(a),not(or(exists(b),count(c,>=,2))))""#),
            "{json}"
        );
        assert!(json.contains(r#""condition":"count(a,=,1)""#), "{json}");
    }

    #[test]
    fn golden_vectors() {
        // 规范文档 `spec/ref/v1/declaration-digest.md` §5 的 golden 向量。
        assert_eq!(
            declaration_digest("se_lifecycle", &base_domain()),
            "sha256:feade2dd1b4e7b5e3fe4ffc1619decf1cae0c217e7d1c8b2629756a1569d76e3"
        );
        let minimal = pact_parser::load_domain(
            "types { t }\npreconditions { }\ngates { p: reads t }\nseed_authority { p: seeds t }",
        )
        .unwrap();
        assert_eq!(
            declaration_digest("minimal", &minimal),
            "sha256:320e4491c4bdade4d9956db27fc42127104699110028f4cac9aff0feeaf21300"
        );
    }
}

//! PACT 域声明文法解析器（论文 §3.1）：`.pact` 文本 -> 良构 `pact_core::Domain`。
//!
//! 分两层：语法（本 crate，手写递归下降 + logos 词法）与
//! 语义良构判定（pact-core 纯函数，决策 D14）。

mod convert;
mod emit;
mod lexer;
mod parser;

pub use convert::{build_domain, load_domain};
pub use emit::{emit_domain, emit_expr};
pub use lexer::{Span, SyntaxError};
pub use parser::{parse_raw, Problem, RawClause, RawDeclaration};

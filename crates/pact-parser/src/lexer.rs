//! 词法分析（logos）。符号集 = 论文 §3.1 文法 + 例文中的 `seed_authority` 段。

use logos::Logos;

#[derive(Logos, Debug, Clone, PartialEq)]
pub enum Tok {
    #[token("types")]
    Types,
    #[token("preconditions")]
    Preconditions,
    #[token("gates")]
    Gates,
    #[token("seed_authority")]
    SeedAuthority,
    #[token("seeds")]
    Seeds,
    #[token("reads")]
    Reads,
    #[token("consumes")]
    Consumes,
    #[token("produces")]
    Produces,
    #[token("via")]
    Via,
    #[token("when")]
    When,
    #[token("exists")]
    Exists,
    #[token("count")]
    Count,
    #[token("and")]
    And,
    #[token("or")]
    Or,
    #[token("not")]
    Not,
    #[token("{")]
    LBrace,
    #[token("}")]
    RBrace,
    /// GateExpr 分组括号（D16 扩展族：论文文法未规定结合性，
    /// emit 回写 And(Or(a,b), c) 类结构需要显式分组）。
    #[token("(")]
    LParen,
    #[token(")")]
    RParen,
    #[token(";")]
    Semi,
    #[token(":")]
    Colon,
    #[token(",")]
    Comma,
    #[token("+")]
    Plus,
    /// 论文文法用 "→"；同时接受 ASCII 形式 "->"（可用性扩展）。
    #[token("→")]
    #[token("->")]
    Arrow,
    #[token(">=")]
    Ge,
    #[token("=")]
    Eq,
    #[regex("[a-zA-Z_][a-zA-Z0-9_]*", |lex| lex.slice().to_owned())]
    Ident(String),
    #[regex("[0-9]+", |lex| lex.slice().parse::<u64>().map_err(|_| ()))]
    Nat(u64),
    // 空白跳过；行注释 "// ..." 为可用性扩展（论文文法未定义）
    #[regex(r"[ \t\r\n]+", logos::skip)]
    #[regex(r"//[^\r\n]*", logos::skip)]
    Error,
}

/// 源码位置（1-based 行列）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub column: usize,
}

/// 词法/语法错误。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub span: Span,
    pub message: String,
}

/// 把字节偏移换算为 (行, 列)。
pub(crate) fn line_col(src: &str, offset: usize) -> Span {
    let mut line = 1;
    let mut column = 1;
    for ch in src[..offset.min(src.len())].chars() {
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    Span { line, column }
}

/// 词法：收集全部无法识别的符号；成功时返回 (token, 起始偏移, 结束偏移) 序列。
pub(crate) fn lex(src: &str) -> Result<Vec<(Tok, usize, usize)>, Vec<SyntaxError>> {
    let mut toks = Vec::new();
    let mut errs = Vec::new();
    let mut lexer = Tok::lexer(src);
    while let Some(tok) = lexer.next() {
        let span = lexer.span();
        match tok {
            Err(()) => errs.push(SyntaxError {
                span: line_col(src, span.start),
                message: format!("无法识别的符号：{slice:?}", slice = lexer.slice()),
            }),
            Ok(t) => toks.push((t, span.start, span.end)),
        }
    }
    if errs.is_empty() {
        Ok(toks)
    } else {
        Err(errs)
    }
}

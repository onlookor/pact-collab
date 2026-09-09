//! 递归下降语法分析（论文 §3.1 文法）。
//!
//! 结构遵循文法：四段固定顺序 types → preconditions → gates → seed_authority。
//! 宽容策略：尾分号可省略/冗余；preconditions/gates/seed_authority 段可为空
//! （types 段至少一个类型名，与文法一致）。段序与段落完整性严格要求。
//!
//! GateExpr 优先级约定（论文文法未规定结合性）：not > and > or，
//! 同级左结合。分组括号为可用性扩展（D16，emit 回写需要）。
//!
//! 解析期直接拒绝的良构违例（论文 §3.1）：空 lhs（约束 3 Non-Empty LHS）、
//! count 字面量非 PosNat（约束 5）。其余约束（RHS/LHS Uniqueness、Type Closure、
//! Source-Production）在 pact-core 判定（D14、D17 三层强制）。

use crate::lexer::{lex, line_col, Span, SyntaxError, Tok};
use pact_core::{Action, CountOp, GateExpr, ParticipantName, TypeName};

/// 语法层原始声明（规范化为 Domain 之前的中间表示）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDeclaration {
    pub types: Vec<TypeName>,
    /// (前驱类型列表, rhs)。多条规则可指向同一 rhs —— 由 build_domain 判定 §1.4.4 违例。
    pub precond_rules: Vec<(Vec<TypeName>, TypeName)>,
    /// (参与者, 子句列表)。同一参与者可多次出现（合并语义，§1.8）。
    pub gate_defs: Vec<(ParticipantName, Vec<RawClause>)>,
    pub seed_authority: Vec<(ParticipantName, TypeName)>,
}

/// 单个 gate 子句（未做唯一性与良构检查）。
/// produces 携带条件在语法上可解析，由 build_domain 报 ProduceWithCondition。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawClause {
    pub action: Action,
    pub ty: TypeName,
    pub cond: Option<GateExpr>,
}

/// load_domain 的失败分类。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// 词法/语法错误（词法层收集全部，语法层首错即停）。
    Syntax(Vec<SyntaxError>),
    /// 良构性错误（全部收集；判定在 pact-core，D14）。
    WellFormedness(Vec<pact_core::DomainError>),
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<(Tok, usize, usize)>,
    pos: usize,
}

type PResult<T> = Result<T, SyntaxError>;

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos).map(|(t, _, _)| t)
    }

    fn check(&self, tok: &Tok) -> bool {
        matches!(self.peek(), Some(t) if t == tok)
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if self.check(tok) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn next_tok(&mut self) -> PResult<Tok> {
        match self.toks.get(self.pos) {
            Some((t, _, _)) => {
                self.pos += 1;
                Ok(t.clone())
            }
            None => Err(self.err_eof("期望符号，但到达文件末尾")),
        }
    }

    fn span_here(&self) -> Span {
        match self.toks.get(self.pos) {
            Some((_, s, _)) => line_col(self.src, *s),
            None => line_col(self.src, self.src.len()),
        }
    }

    fn err_here(&self, message: impl Into<String>) -> SyntaxError {
        SyntaxError {
            span: self.span_here(),
            message: message.into(),
        }
    }

    fn err_eof(&self, message: impl Into<String>) -> SyntaxError {
        SyntaxError {
            span: line_col(self.src, self.src.len()),
            message: message.into(),
        }
    }

    /// 消耗一个标识符并返回其字符串（类型名/参与者名通用）。
    fn ident(&mut self, what: &str) -> PResult<String> {
        match self.next_tok()? {
            Tok::Ident(s) => Ok(s),
            t => Err(self.err_prev(format!("期望{what}（标识符），遇到 {t:?}"))),
        }
    }

    /// 已消耗 last 后的报告辅助：位置取前一个 token。
    fn err_prev(&self, message: impl Into<String>) -> SyntaxError {
        let idx = self.pos.saturating_sub(1);
        let off = self.toks.get(idx).map(|(_, s, _)| *s).unwrap_or(0);
        SyntaxError {
            span: line_col(self.src, off),
            message: message.into(),
        }
    }

    fn expect(&mut self, tok: &Tok) -> PResult<()> {
        if self.eat(tok) {
            Ok(())
        } else {
            let found = match self.peek() {
                Some(t) => format!("{t:?}"),
                None => "文件末尾".to_string(),
            };
            Err(self.err_here(format!("期望 {:?}，遇到 {}", tok_variant(tok), found)))
        }
    }

    /// PosNat 字面量（论文 §1.7.3、§3.1 约束 5）：`[1-9][0-9]*`。
    /// 拒绝 0（Normalization rule：`count T = 0` ≡ `not exists T`，
    /// `count T >= 0` ≡ true，均须语法拒绝）与前导零（非规范写法）。
    fn pos_nat(&mut self) -> PResult<u64> {
        let (tok, start, end) = match self.toks.get(self.pos) {
            Some(t) => t.clone(),
            None => return Err(self.err_eof("期望正整数（PosNat），但到达文件末尾")),
        };
        self.pos += 1;
        match tok {
            Tok::Nat(n) => {
                let lit = &self.src[start..end];
                let span = line_col(self.src, start);
                if n == 0 {
                    return Err(SyntaxError {
                        span,
                        message: "count 字面量必须为正整数（PosNat，§1.7.3）：0 非法；\
                                  缺席语义请用 not exists T（Normalization rule）"
                            .to_owned(),
                    });
                }
                if lit.starts_with('0') {
                    return Err(SyntaxError {
                        span,
                        message: format!(
                            "count 字面量必须匹配 PosNat `[1-9][0-9]*`（§3.1）：{lit} 含前导零"
                        ),
                    });
                }
                Ok(n)
            }
            t => Err(self.err_prev(format!("期望正整数（PosNat），遇到 {t:?}"))),
        }
    }

    // —— Domain ::= TypeSchema PreconditionGraph GateMatrix SeedAuthorityDecl ——

    fn domain(&mut self) -> PResult<RawDeclaration> {
        let types = self.type_schema()?;
        let precond_rules = self.precondition_graph()?;
        let gate_defs = self.gate_matrix()?;
        let seed_authority = self.seed_authority_decl()?;
        if let Some(t) = self.peek() {
            return Err(self.err_here(format!("域名声明结束后的多余符号：{t:?}")));
        }
        // 重复类型声明检查（语法层）
        let mut seen = std::collections::BTreeSet::new();
        for ty in &types {
            if !seen.insert(ty.clone()) {
                return Err(self.err_here(format!("重复类型声明：{ty}")));
            }
        }
        Ok(RawDeclaration {
            types,
            precond_rules,
            gate_defs,
            seed_authority,
        })
    }

    // —— TypeSchema ::= "types" "{" TypeName { ";" TypeName } "}"（至少一个类型名）——

    fn type_schema(&mut self) -> PResult<Vec<TypeName>> {
        self.expect(&Tok::Types)?;
        self.expect(&Tok::LBrace)?;
        if self.check(&Tok::RBrace) {
            return Err(self.err_here("types 段至少需要一个类型名"));
        }
        let mut types = vec![TypeName(self.ident("类型名")?)];
        while self.eat(&Tok::Semi) {
            if self.check(&Tok::RBrace) {
                break; // 尾分号容忍
            }
            types.push(TypeName(self.ident("类型名")?));
        }
        self.expect(&Tok::RBrace)?;
        Ok(types)
    }

    /// 通用段体循环：`{ (item (Semi item)*)? Semi? }`。
    /// item 解析器由闭包给出；返回条目列表（可为空）。
    fn section<T>(
        &mut self,
        keyword: &Tok,
        section_name: &str,
        mut item: impl FnMut(&mut Self) -> PResult<T>,
    ) -> PResult<Vec<T>> {
        self.expect(keyword)?;
        self.expect(&Tok::LBrace)?;
        let mut items = Vec::new();
        loop {
            if self.check(&Tok::RBrace) {
                break;
            }
            if !items.is_empty() {
                self.expect(&Tok::Semi).map_err(|mut e| {
                    e.message = format!("{}条目之间需要 \";\" 分隔（{}）", e.message, section_name);
                    e
                })?;
            }
            if self.check(&Tok::RBrace) {
                break; // 尾分号容忍
            }
            items.push(item(self)?);
        }
        self.expect(&Tok::RBrace)?;
        Ok(items)
    }

    // —— PrecondRule ::= TypeName "→" TypeName | "{" TypeName { "+" TypeName } "}" "→" TypeName ——

    fn precondition_graph(&mut self) -> PResult<Vec<(Vec<TypeName>, TypeName)>> {
        self.section(&Tok::Preconditions, "preconditions", |p| {
            let lhs = if p.eat(&Tok::LBrace) {
                // §1.4.4 Non-Empty LHS：空花括号 lhs 语法层拒绝
                //（源类型仅由规则缺席定义，不由空规则定义）
                if p.check(&Tok::RBrace) {
                    return Err(p.err_here(
                        "前置规则的 lhs 不得为空（§1.4.4 Non-Empty LHS）；\
                         源类型请直接省略其前置规则",
                    ));
                }
                let mut names = vec![TypeName(p.ident("类型名")?)];
                while p.eat(&Tok::Plus) {
                    names.push(TypeName(p.ident("类型名")?));
                }
                p.expect(&Tok::RBrace)?;
                names
            } else {
                vec![TypeName(p.ident("类型名")?)]
            };
            p.expect(&Tok::Arrow)?;
            let rhs = TypeName(p.ident("类型名")?);
            Ok((lhs, rhs))
        })
    }

    // —— GateDef ::= ParticipantName ":" ActionClause { "," ActionClause } ——

    fn gate_matrix(&mut self) -> PResult<Vec<(ParticipantName, Vec<RawClause>)>> {
        self.section(&Tok::Gates, "gates", |p| {
            let participant = ParticipantName(p.ident("参与者名")?);
            p.expect(&Tok::Colon)?;
            let mut clauses = vec![p.action_clause()?];
            while p.eat(&Tok::Comma) {
                clauses.push(p.action_clause()?);
            }
            Ok((participant, clauses))
        })
    }

    // —— ActionClause ::= ("reads"|"consumes") TypeName GateCondOpt | "produces" TypeName GateCondOpt ——

    fn action_clause(&mut self) -> PResult<RawClause> {
        let action = match self.next_tok()? {
            Tok::Reads => Action::Read,
            Tok::Consumes => Action::Consume,
            Tok::Produces => Action::Produce,
            t => return Err(self.err_prev(format!("期望 reads/consumes/produces，遇到 {t:?}"))),
        };
        let ty = TypeName(self.ident("类型名")?);
        let cond = self.gate_cond_opt()?;
        Ok(RawClause { action, ty, cond })
    }

    // —— GateCondOpt ::= ε | "via" TypeName | "when" GateExpr（via 糖形式在此脱糖，§1.7.4）——

    fn gate_cond_opt(&mut self) -> PResult<Option<GateExpr>> {
        if self.eat(&Tok::Via) {
            let ta = TypeName(self.ident("via 见证类型名")?);
            Ok(Some(GateExpr::Via(ta)))
        } else if self.eat(&Tok::When) {
            self.gate_expr().map(Some)
        } else {
            Ok(None)
        }
    }

    // —— GateExpr（优先级：or < and < not；左结合；无括号，忠于 §1.7.3 文法）——

    fn gate_expr(&mut self) -> PResult<GateExpr> {
        let mut left = self.gate_term()?;
        while self.eat(&Tok::Or) {
            let right = self.gate_term()?;
            left = GateExpr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn gate_term(&mut self) -> PResult<GateExpr> {
        let mut left = self.gate_factor()?;
        while self.eat(&Tok::And) {
            let right = self.gate_factor()?;
            left = GateExpr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn gate_factor(&mut self) -> PResult<GateExpr> {
        if self.eat(&Tok::Not) {
            return self.gate_factor().map(|e| GateExpr::Not(Box::new(e)));
        }
        self.gate_atom()
    }

    fn gate_atom(&mut self) -> PResult<GateExpr> {
        // 分组括号（D16 扩展族，emit 回写需要）
        if self.eat(&Tok::LParen) {
            let e = self.gate_expr()?;
            self.expect(&Tok::RParen)?;
            return Ok(e);
        }
        if self.eat(&Tok::Via) {
            return Ok(GateExpr::Via(TypeName(self.ident("via 见证类型名")?)));
        }
        if self.eat(&Tok::Exists) {
            return Ok(GateExpr::Exists(TypeName(self.ident("exists 类型名")?)));
        }
        if self.eat(&Tok::Count) {
            let ty = TypeName(self.ident("count 类型名")?);
            let op = if self.eat(&Tok::Ge) {
                CountOp::Ge
            } else {
                self.expect(&Tok::Eq)?;
                CountOp::Eq
            };
            let n = self.pos_nat()?;
            return Ok(GateExpr::Count { ty, op, n });
        }
        Err(self.err_here("期望 via / exists / count / not 构成条件表达式"))
    }

    // —— SeedAuthorityDecl ::= "seed_authority" "{" [ ParticipantName ":" "seeds" TypeName { ";" ... } ] "}" ——

    fn seed_authority_decl(&mut self) -> PResult<Vec<(ParticipantName, TypeName)>> {
        self.section(&Tok::SeedAuthority, "seed_authority", |p| {
            let participant = ParticipantName(p.ident("参与者名")?);
            p.expect(&Tok::Colon)?;
            p.expect(&Tok::Seeds)?;
            let ty = TypeName(p.ident("类型名")?);
            Ok((participant, ty))
        })
    }
}

/// tok_variant：错误消息里显示的关键字正文。
fn tok_variant(tok: &Tok) -> String {
    match tok {
        Tok::Types => "\"types\"".into(),
        Tok::Preconditions => "\"preconditions\"".into(),
        Tok::Gates => "\"gates\"".into(),
        Tok::SeedAuthority => "\"seed_authority\"".into(),
        Tok::LBrace => "\"{\"".into(),
        Tok::RBrace => "\"}\"".into(),
        Tok::Semi => "\";\"".into(),
        Tok::Colon => "\":\"".into(),
        Tok::Arrow => "\"→\"".into(),
        Tok::Eq => "\"=\"".into(),
        _ => format!("{tok:?}"),
    }
}

/// 仅语法层：文本 -> RawDeclaration。
/// 词法错误全部收集；语法错误首错即停。
pub fn parse_raw(src: &str) -> Result<RawDeclaration, Vec<SyntaxError>> {
    let toks = lex(src)?;
    let mut p = Parser { src, toks, pos: 0 };
    match p.domain() {
        Ok(raw) => Ok(raw),
        Err(e) => Err(vec![e]),
    }
}

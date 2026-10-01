//! Recursive-descent parser for the PCM StoEx grammar (Xtext, SimuLizar 5.2.2).
//!
//! Grammar (`Stoex.xtext`, `PCMStoex.xtext`), with the precedence it really has:
//!
//! ```text
//! expression  := ifelse EOF
//! ifelse      := boolAnd ( '?' boolAnd ':' boolAnd )?          -- not nestable without ( )
//! boolAnd     := boolOr ( 'AND' boolOr )*                      -- AND binds WEAKER than OR/XOR
//! boolOr      := compare ( ('OR'|'XOR') compare )*
//! compare     := sum ( ('>'|'<'|'=='|'<>'|'>='|'<=') sum )?    -- not associative
//! sum         := prod ( ('+'|'-') prod )*
//! prod        := pow ( ('*'|'/'|'%') pow )*
//! pow         := unary ( '^' unary )?                          -- not associative
//! unary       := 'NOT' unary | '-' unary | atom
//! atom        := DECINT | DOUBLE | STRING | BOOL
//!              | ID '(' ( boolAnd (',' boolAnd)* )? ')'        -- arguments are boolAnd, no ?:
//!              | ID ('.' ID)* '.' ('BYTESIZE'|'NUMBER_OF_ELEMENTS'|'STRUCTURE'|'TYPE'|'VALUE')
//!              | '(' ifelse ')'
//!              | 'IntPMF' '[' ('(' SIGNED_INT ';' NUMBER ')')+ ']'
//!              | 'DoublePMF' '[' ('(' SIGNED_NUMBER ';' NUMBER ')')+ ']'
//!              | 'EnumPMF' ('(' 'ordered' ')')? '[' ('(' STRING ';' NUMBER ')')+ ']'
//!              | 'BoolPMF' ('(' 'ordered' ')')? '[' ('(' BOOL ';' NUMBER ')')+ ']'
//!              | 'DoublePDF' '[' ('(' SIGNED_NUMBER ';' NUMBER ')')+ ']'
//! NUMBER      := DECINT | DOUBLE                  (converted with Double.parseDouble)
//! SIGNED_INT  := '-'? DECINT                      (Integer.valueOf; '-' must be adjacent)
//! SIGNED_NUMBER := '-'? NUMBER                    ('-' must be adjacent)
//! ```
//!
//! Literal conversion errors (`2147483648`, `- 5` inside a sample) are syntax errors, as in the
//! reference. A negative literal outside a PMF is a `NegativeExpression`.

use crate::ast::*;
use crate::error::{ParseError, Span};
use crate::lexer::{Kw, P, TokKind, Token, tokenize};

/// Parses a StoEx.
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let toks = tokenize(src)?;
    if toks.len() == 1 {
        return Err(ParseError::new(
            src,
            Span::new(0, src.len()),
            "The given stoex is empty. Therefore it is no valid stoex.",
        ));
    }
    let mut p = Parser {
        src,
        toks,
        pos: 0,
        nest: 0,
        d: 0,
    };
    let e = p.ifelse()?;
    if !matches!(p.peek().kind, TokKind::Eof) {
        return Err(p.unexpected("end of input"));
    }
    Ok(e)
}

/// Maximum nesting of parentheses, function calls and unary operators. The parser is recursive
/// (about 2.5 KB of stack per parenthesis level); deeper input is a syntax error instead of a
/// stack overflow.
pub const MAX_NESTING: u32 = 200;

/// Maximum depth of the syntax tree (a chain `a + b + c ...` of n terms has depth n). Type
/// inference and evaluation walk the tree recursively.
pub const MAX_DEPTH: u32 = 1000;

struct Parser<'s> {
    src: &'s str,
    toks: Vec<Token>,
    pos: usize,
    /// Current recursion nesting (see [`MAX_NESTING`]).
    nest: u32,
    /// Tree depth of the expression returned last (see [`MAX_DEPTH`]).
    d: u32,
}

fn describe(t: &Token, src: &str) -> String {
    match t.kind {
        TokKind::Eof => "end of input".to_string(),
        _ => format!("'{}'", &src[t.span.start..t.span.end]),
    }
}

impl<'s> Parser<'s> {
    fn peek(&self) -> &Token {
        &self.toks[self.pos]
    }

    fn peek_at(&self, n: usize) -> &Token {
        &self.toks[(self.pos + n).min(self.toks.len() - 1)]
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn unexpected(&self, expected: &str) -> ParseError {
        let t = self.peek();
        ParseError::new(
            self.src,
            t.span,
            format!("expected {expected}, found {}", describe(t, self.src)),
        )
    }

    fn is_p(&self, p: P) -> bool {
        self.peek().kind == TokKind::P(p)
    }

    fn is_kw(&self, k: Kw) -> bool {
        self.peek().kind == TokKind::Kw(k)
    }

    fn expect_p(&mut self, p: P) -> Result<Token, ParseError> {
        if self.is_p(p) {
            Ok(self.bump())
        } else {
            Err(self.unexpected(&format!("'{}'", p.as_str())))
        }
    }

    /// Enters a nested construct at the current token.
    fn enter(&mut self) -> Result<(), ParseError> {
        self.nest += 1;
        if self.nest > MAX_NESTING {
            return Err(ParseError::new(
                self.src,
                self.peek().span,
                format!("expression nested too deeply (more than {MAX_NESTING} levels)"),
            ));
        }
        Ok(())
    }

    /// Records the tree depth `d` of a node built at `span`.
    fn depth(&mut self, d: u32, span: Span) -> Result<(), ParseError> {
        if d > MAX_DEPTH {
            return Err(ParseError::new(
                self.src,
                span,
                format!("expression too deep (syntax tree deeper than {MAX_DEPTH})"),
            ));
        }
        self.d = d;
        Ok(())
    }

    fn text(&self, t: &Token) -> &'s str {
        &self.src[t.span.start..t.span.end]
    }

    fn ifelse(&mut self) -> Result<Expr, ParseError> {
        let c = self.bool_and()?;
        if self.is_p(P::Question) {
            let dc = self.d;
            self.bump();
            let a = self.bool_and()?;
            let da = self.d;
            self.expect_p(P::Colon)?;
            let b = self.bool_and()?;
            let span = Span::new(c.span.start, b.span.end);
            self.depth(dc.max(da).max(self.d) + 1, span)?;
            return Ok(Expr::new(
                ExprKind::IfElse(Box::new(c), Box::new(a), Box::new(b)),
                span,
            ));
        }
        Ok(c)
    }

    fn bool_and(&mut self) -> Result<Expr, ParseError> {
        let mut l = self.bool_or()?;
        while self.is_kw(Kw::And) {
            let dl = self.d;
            self.bump();
            let r = self.bool_or()?;
            let span = Span::new(l.span.start, r.span.end);
            self.depth(dl.max(self.d) + 1, span)?;
            l = Expr::new(
                ExprKind::BoolOp(BoolOp::And, Box::new(l), Box::new(r)),
                span,
            );
        }
        Ok(l)
    }

    fn bool_or(&mut self) -> Result<Expr, ParseError> {
        let mut l = self.compare()?;
        loop {
            let op = if self.is_kw(Kw::Or) {
                BoolOp::Or
            } else if self.is_kw(Kw::Xor) {
                BoolOp::Xor
            } else {
                break;
            };
            let dl = self.d;
            self.bump();
            let r = self.compare()?;
            let span = Span::new(l.span.start, r.span.end);
            self.depth(dl.max(self.d) + 1, span)?;
            l = Expr::new(ExprKind::BoolOp(op, Box::new(l), Box::new(r)), span);
        }
        Ok(l)
    }

    fn compare(&mut self) -> Result<Expr, ParseError> {
        let l = self.sum()?;
        let op = match self.peek().kind {
            TokKind::P(P::Gt) => CmpOp::Greater,
            TokKind::P(P::Lt) => CmpOp::Less,
            TokKind::P(P::EqEq) => CmpOp::Equals,
            TokKind::P(P::NotEq) => CmpOp::NotEqual,
            TokKind::P(P::Ge) => CmpOp::GreaterEqual,
            TokKind::P(P::Le) => CmpOp::LessEqual,
            _ => return Ok(l),
        };
        let dl = self.d;
        self.bump();
        let r = self.sum()?;
        let span = Span::new(l.span.start, r.span.end);
        self.depth(dl.max(self.d) + 1, span)?;
        Ok(Expr::new(
            ExprKind::Compare(op, Box::new(l), Box::new(r)),
            span,
        ))
    }

    fn sum(&mut self) -> Result<Expr, ParseError> {
        let mut l = self.prod()?;
        loop {
            let op = match self.peek().kind {
                TokKind::P(P::Plus) => TermOp::Add,
                TokKind::P(P::Minus) => TermOp::Sub,
                _ => break,
            };
            let dl = self.d;
            self.bump();
            let r = self.prod()?;
            let span = Span::new(l.span.start, r.span.end);
            self.depth(dl.max(self.d) + 1, span)?;
            l = Expr::new(ExprKind::Term(op, Box::new(l), Box::new(r)), span);
        }
        Ok(l)
    }

    fn prod(&mut self) -> Result<Expr, ParseError> {
        let mut l = self.pow()?;
        loop {
            let op = match self.peek().kind {
                TokKind::P(P::Star) => ProdOp::Mult,
                TokKind::P(P::Slash) => ProdOp::Div,
                TokKind::P(P::Percent) => ProdOp::Mod,
                _ => break,
            };
            let dl = self.d;
            self.bump();
            let r = self.pow()?;
            let span = Span::new(l.span.start, r.span.end);
            self.depth(dl.max(self.d) + 1, span)?;
            l = Expr::new(ExprKind::Product(op, Box::new(l), Box::new(r)), span);
        }
        Ok(l)
    }

    fn pow(&mut self) -> Result<Expr, ParseError> {
        let b = self.unary()?;
        if self.is_p(P::Caret) {
            let db = self.d;
            self.bump();
            let e = self.unary()?;
            let span = Span::new(b.span.start, e.span.end);
            self.depth(db.max(self.d) + 1, span)?;
            return Ok(Expr::new(ExprKind::Power(Box::new(b), Box::new(e)), span));
        }
        Ok(b)
    }

    fn unary(&mut self) -> Result<Expr, ParseError> {
        if self.is_kw(Kw::Not) {
            self.enter()?;
            let t = self.bump();
            let inner = self.unary()?;
            self.nest -= 1;
            let span = Span::new(t.span.start, inner.span.end);
            self.depth(self.d + 1, span)?;
            return Ok(Expr::new(ExprKind::Not(Box::new(inner)), span));
        }
        if self.is_p(P::Minus) {
            self.enter()?;
            let t = self.bump();
            let inner = self.unary()?;
            self.nest -= 1;
            let span = Span::new(t.span.start, inner.span.end);
            self.depth(self.d + 1, span)?;
            return Ok(Expr::new(ExprKind::Neg(Box::new(inner)), span));
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<Expr, ParseError> {
        // leaves have depth 1; the composite branches below overwrite it
        self.d = 1;
        let t = self.peek().clone();
        match &t.kind {
            TokKind::DecInt => {
                self.bump();
                let v = self.conv_int(&t, false)?;
                Ok(Expr::new(ExprKind::Int(v), t.span))
            }
            TokKind::Double => {
                self.bump();
                let v = conv_double(self.text(&t));
                Ok(Expr::new(ExprKind::Double(v), t.span))
            }
            TokKind::Str(s) => {
                self.bump();
                Ok(Expr::new(ExprKind::Str(s.clone()), t.span))
            }
            TokKind::Bool(b) => {
                self.bump();
                Ok(Expr::new(ExprKind::Bool(*b), t.span))
            }
            TokKind::Id => match self.peek_at(1).kind {
                TokKind::P(P::LParen) => self.function(),
                TokKind::P(P::Dot) => self.variable(),
                _ => {
                    self.bump();
                    Err(self.unexpected("'(' or '.' after identifier"))
                }
            },
            TokKind::P(P::LParen) => {
                self.enter()?;
                self.bump();
                let inner = self.ifelse()?;
                let close = self.expect_p(P::RParen)?;
                self.nest -= 1;
                let span = Span::new(t.span.start, close.span.end);
                self.depth(self.d + 1, span)?;
                Ok(Expr::new(ExprKind::Paren(Box::new(inner)), span))
            }
            TokKind::Kw(
                k @ (Kw::IntPmf | Kw::DoublePmf | Kw::EnumPmf | Kw::BoolPmf | Kw::DoublePdf),
            ) => {
                let k = *k;
                self.prob_fn(k)
            }
            _ => Err(self.unexpected("an expression")),
        }
    }

    fn function(&mut self) -> Result<Expr, ParseError> {
        let name_tok = self.bump();
        let name = self.text(&name_tok).to_string();
        self.expect_p(P::LParen)?;
        self.enter()?;
        let mut args = Vec::new();
        let mut d = 0;
        if !self.is_p(P::RParen) {
            args.push(self.bool_and()?);
            d = self.d;
            while self.is_p(P::Comma) {
                self.bump();
                args.push(self.bool_and()?);
                d = d.max(self.d);
            }
        }
        let close = self.expect_p(P::RParen)?;
        self.nest -= 1;
        let span = Span::new(name_tok.span.start, close.span.end);
        self.depth(d + 1, span)?;
        Ok(Expr::new(ExprKind::Func(name, args), span))
    }

    fn variable(&mut self) -> Result<Expr, ParseError> {
        let first = self.bump();
        let mut path = vec![self.text(&first).to_string()];
        loop {
            self.expect_p(P::Dot)?;
            let t = self.peek().clone();
            let ch = match t.kind {
                TokKind::Id => {
                    self.bump();
                    path.push(self.text(&t).to_string());
                    continue;
                }
                TokKind::Kw(Kw::ByteSize) => Characterisation::ByteSize,
                TokKind::Kw(Kw::NumberOfElements) => Characterisation::NumberOfElements,
                TokKind::Kw(Kw::Structure) => Characterisation::Structure,
                TokKind::Kw(Kw::Type) => Characterisation::Type,
                TokKind::Kw(Kw::Value) => Characterisation::Value,
                _ => {
                    return Err(self.unexpected(
                        "an identifier or BYTESIZE, NUMBER_OF_ELEMENTS, STRUCTURE, TYPE, VALUE",
                    ));
                }
            };
            self.bump();
            return Ok(Expr::new(
                ExprKind::Var(VarRef {
                    path,
                    characterisation: ch,
                }),
                Span::new(first.span.start, t.span.end),
            ));
        }
    }

    fn conv_int(&self, t: &Token, negative: bool) -> Result<i32, ParseError> {
        let digits = self.text(t);
        let parsed = if negative {
            format!("-{digits}").parse::<i32>()
        } else {
            digits.parse::<i32>()
        };
        parsed.map_err(|_| {
            ParseError::new(
                self.src,
                t.span,
                format!(
                    "For input string: \"{}{digits}\"",
                    if negative { "-" } else { "" }
                ),
            )
        })
    }

    /// `'-'? DECINT` with the '-' adjacent to the digits.
    fn signed_int(&mut self) -> Result<i32, ParseError> {
        let neg = self.minus_prefix()?;
        let t = self.peek().clone();
        if t.kind != TokKind::DecInt {
            return Err(self.unexpected("an integer"));
        }
        self.bump();
        self.conv_int(&t, neg)
    }

    /// `'-'? (DECINT | DOUBLE)` with the '-' adjacent to the digits.
    fn signed_number(&mut self) -> Result<f64, ParseError> {
        let neg = self.minus_prefix()?;
        let v = self.number()?;
        Ok(if neg { -v } else { v })
    }

    /// Consumes a '-' and checks that the next token follows without hidden tokens (the
    /// reference converts the datatype rule text, whitespace included, with parseDouble /
    /// Integer.valueOf, which fails for "- 5").
    fn minus_prefix(&mut self) -> Result<bool, ParseError> {
        if !self.is_p(P::Minus) {
            return Ok(false);
        }
        let m = self.bump();
        let next = self.peek();
        if next.span.start != m.span.end && matches!(next.kind, TokKind::DecInt | TokKind::Double) {
            return Err(ParseError::new(
                self.src,
                Span::new(m.span.start, next.span.end),
                format!(
                    "For input string: \"{}\"",
                    &self.src[m.span.start..next.span.end]
                ),
            ));
        }
        Ok(true)
    }

    /// `NUMBER := DECINT | DOUBLE`, converted to double.
    fn number(&mut self) -> Result<f64, ParseError> {
        let t = self.peek().clone();
        match t.kind {
            TokKind::DecInt | TokKind::Double => {
                self.bump();
                Ok(conv_double(self.text(&t)))
            }
            _ => Err(self.unexpected("a number")),
        }
    }

    fn prob_fn(&mut self, k: Kw) -> Result<Expr, ParseError> {
        let kw = self.bump();
        let mut ordered = false;
        if matches!(k, Kw::EnumPmf | Kw::BoolPmf) && self.is_p(P::LParen) {
            self.bump();
            if !self.is_kw(Kw::Ordered) {
                return Err(self.unexpected("'ordered'"));
            }
            self.bump();
            self.expect_p(P::RParen)?;
            ordered = true;
        }
        self.expect_p(P::LBracket)?;
        macro_rules! samples {
            ($value:expr) => {{
                let mut v = Vec::new();
                loop {
                    self.expect_p(P::LParen)?;
                    let value = $value;
                    self.expect_p(P::Semi)?;
                    let p = self.number()?;
                    self.expect_p(P::RParen)?;
                    v.push((value, p));
                    if !self.is_p(P::LParen) {
                        break;
                    }
                }
                v
            }};
        }
        let lit = match k {
            Kw::IntPmf => ProbFnLit::IntPmf(samples!(self.signed_int()?)),
            Kw::DoublePmf => ProbFnLit::DoublePmf(samples!(self.signed_number()?)),
            Kw::DoublePdf => ProbFnLit::BoxedPdf(samples!(self.signed_number()?)),
            Kw::EnumPmf => ProbFnLit::EnumPmf {
                ordered,
                samples: samples!({
                    let t = self.peek().clone();
                    match t.kind {
                        TokKind::Str(s) => {
                            self.bump();
                            s
                        }
                        _ => return Err(self.unexpected("a string")),
                    }
                }),
            },
            Kw::BoolPmf => ProbFnLit::BoolPmf {
                ordered,
                samples: samples!({
                    match self.peek().kind {
                        TokKind::Bool(b) => {
                            self.bump();
                            b
                        }
                        _ => return Err(self.unexpected("true or false")),
                    }
                }),
            },
            _ => unreachable!("prob_fn called with {k:?}"),
        };
        let close = self.expect_p(P::RBracket)?;
        Ok(Expr::new(
            ExprKind::ProbFn(lit),
            Span::new(kw.span.start, close.span.end),
        ))
    }
}

/// `Double.parseDouble` of a `DECINT`/`DOUBLE` token text (correctly rounded; `1.` and `1.e5`
/// are valid; overflow gives infinity).
fn conv_double(text: &str) -> f64 {
    let mut s = String::with_capacity(text.len() + 1);
    let b = text.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        s.push(c as char);
        if c == b'.' && !b.get(i + 1).is_some_and(u8::is_ascii_digit) {
            s.push('0');
        }
    }
    s.parse::<f64>().expect("lexer guarantees a valid number")
}

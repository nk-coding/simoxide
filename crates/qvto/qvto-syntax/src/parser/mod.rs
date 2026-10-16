//! Hand-written recursive-descent parser with Pratt parsing for OCL's binary operators,
//! following Eclipse's LPG grammars (`QVTOParser.gi` → `ImperativeOCL.gi` → `EssentialOCL.gi`).
//! Comments cite the grammar rules each function implements.
//!
//! Error handling imitates LPG's diagnoses where it is cheap (`"X" expected after "Y"`,
//! `"X" expected instead of "Y"`, `"…" unexpected token(s)`, `End-of-File reached after this
//! token "…"`) and recovers at statement and element boundaries (`;`, `}`, element keywords).

mod expr;
mod module;

use crate::ast::*;
use crate::diag::{Code, Diagnostic};
use crate::escape;
use crate::generated::{TokenKind as K, groups};
use crate::lexer::{Lexed, Token, lpg_text};
use crate::span::Span;

/// Maximum nesting of expressions and statements before the parser gives up on a construct.
/// It bounds the parser's stack on adversarial input: a level takes up to about 6 KB in debug
/// builds and 3 KB in release builds, so 256 levels fit a 2 MB thread stack. The deepest nesting
/// in the corpus is 14.
const MAX_DEPTH: u32 = 256;

pub(crate) struct Parser<'s> {
    src: &'s str,
    toks: Vec<Token>,
    pos: usize,
    diags: Vec<Diagnostic>,
    depth: u32,
    /// Set once the depth limit was hit, to report it only once.
    too_deep: bool,
}

/// A position to backtrack to.
#[derive(Clone, Copy)]
struct Checkpoint {
    pos: usize,
    diags: usize,
}

pub(crate) fn parse_unit(src: &str, lexed: Lexed) -> (Unit, Vec<Diagnostic>) {
    let mut p = Parser {
        src,
        toks: lexed.tokens,
        pos: 0,
        diags: lexed.diagnostics,
        depth: 0,
        too_deep: false,
    };
    let unit = p.unit(lexed.end);
    let mut diags = p.diags;
    diags.sort_by_key(|d| (d.span.start, d.span.end));
    (unit, diags)
}

/// Parses a single OCL expression (the text of an `EOperation` body or a derivation in an
/// OCL annotation); tokens after it are an error.
pub(crate) fn parse_expression(src: &str, lexed: Lexed) -> (crate::ast::Expr, Vec<Diagnostic>) {
    let mut p = Parser {
        src,
        toks: lexed.tokens,
        pos: 0,
        diags: lexed.diagnostics,
        depth: 0,
        too_deep: false,
    };
    let e = p.expr();
    if !p.at(K::Eof) {
        let t = p.tok();
        p.error(Code::UnexpectedTokens, t.span, "unexpected tokens after the expression");
    }
    let mut diags = p.diags;
    diags.sort_by_key(|d| (d.span.start, d.span.end));
    (e, diags)
}

impl<'s> Parser<'s> {
    // ----------------------------------------------------------------------------------------
    // Token cursor

    #[inline]
    fn peek(&self) -> K {
        self.toks[self.pos].kind
    }

    #[inline]
    fn nth(&self, n: usize) -> K {
        self.toks.get(self.pos + n).map_or(K::Eof, |t| t.kind)
    }

    #[inline]
    fn tok(&self) -> Token {
        self.toks[self.pos]
    }

    #[inline]
    fn span(&self) -> Span {
        self.toks[self.pos].span
    }

    #[inline]
    fn at(&self, k: K) -> bool {
        self.peek() == k
    }

    #[inline]
    fn at_any(&self, ks: &[K]) -> bool {
        ks.contains(&self.peek())
    }

    #[inline]
    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos];
        if t.kind != K::Eof {
            self.pos += 1;
        }
        t
    }

    #[inline]
    fn eat(&mut self, k: K) -> Option<Token> {
        if self.at(k) { Some(self.bump()) } else { None }
    }

    /// Span of the previous token (or an empty span at the start).
    fn prev_span(&self) -> Span {
        if self.pos == 0 {
            Span::at(0)
        } else {
            self.toks[self.pos - 1].span
        }
    }

    /// End of the previous token.
    fn prev_end(&self) -> u32 {
        self.prev_span().end
    }

    fn text(&self, t: Token) -> &'s str {
        &self.src[t.span.range()]
    }

    fn checkpoint(&self) -> Checkpoint {
        Checkpoint {
            pos: self.pos,
            diags: self.diags.len(),
        }
    }

    fn rewind(&mut self, cp: Checkpoint) {
        self.pos = cp.pos;
        self.diags.truncate(cp.diags);
    }

    // ----------------------------------------------------------------------------------------
    // Diagnostics

    fn error(&mut self, code: Code, span: Span, msg: impl Into<String>) {
        self.diags.push(Diagnostic::error(code, span, msg));
    }

    fn warning(&mut self, code: Code, span: Span, msg: impl Into<String>) {
        self.diags.push(Diagnostic::warning(code, span, msg));
    }

    /// Text of a token as LPG quotes it in messages.
    fn quoted(&self, t: Token) -> String {
        if t.kind == K::Eof {
            "EOF_TOKEN".to_string()
        } else {
            lpg_text(self.text(t))
        }
    }

    /// Reports that `what` is missing here. At end of input: `End-of-File reached after this
    /// token`; when the current token may start what follows (`insert`): `"X" expected after
    /// "prev"`; otherwise `"X" expected instead of "cur"`.
    fn expected(&mut self, what: &str, insert: bool) {
        let cur = self.tok();
        if cur.kind == K::Eof {
            self.eof_error();
        } else if insert && self.pos > 0 {
            let prev = self.toks[self.pos - 1];
            let msg = format!("\"{what}\" expected after \"{}\"", self.quoted(prev));
            self.error(Code::Expected, prev.span, msg);
        } else {
            let msg = format!("\"{what}\" expected instead of \"{}\"", self.quoted(cur));
            self.error(Code::ExpectedInstead, cur.span, msg);
        }
    }

    fn eof_error(&mut self) {
        if self.diags.iter().any(|d| d.code == Code::UnexpectedEof) {
            return;
        }
        let prev = if self.pos > 0 {
            Some(self.toks[self.pos - 1])
        } else {
            None
        };
        match prev {
            Some(p) => {
                let msg = format!(
                    "End-of-File reached after this token \"{}\"",
                    self.quoted(p)
                );
                self.error(Code::UnexpectedEof, p.span, msg);
            }
            None => self.error(Code::UnexpectedEof, Span::at(0), "End-of-File reached"),
        }
    }

    /// `"tok" unexpected token(s)` for the current token.
    fn unexpected(&mut self) {
        let t = self.tok();
        if t.kind == K::Eof {
            self.eof_error();
            return;
        }
        let msg = format!("\"{}\" unexpected token(s)", self.quoted(t));
        self.error(Code::UnexpectedTokens, t.span, msg);
    }

    /// Expects token `k`. On a mismatch reports it and does not consume anything.
    fn expect(&mut self, k: K) -> Option<Token> {
        if let Some(t) = self.eat(k) {
            return Some(t);
        }
        if self.nth(1) == k && self.peek() != K::Eof {
            // LPG's repair deletes a single stray token when the next one fits.
            self.unexpected();
            self.bump();
            return Some(self.bump());
        }
        let what = k.spelling().unwrap_or(k.lpg_name());
        // Missing closers and separators are reported as insertions after the previous token,
        // like LPG does when the current token can continue the input.
        let insert = matches!(
            k,
            K::Semicolon | K::Rparen | K::Rbrace | K::Rbracket | K::Comma
        );
        self.expected(what, insert);
        None
    }

    /// Skips tokens until one of `sync` (balanced over brackets), without reporting.
    fn skip_to(&mut self, sync: &[K]) {
        let mut depth = 0i32;
        loop {
            let k = self.peek();
            if k == K::Eof {
                return;
            }
            if depth == 0 && sync.contains(&k) {
                return;
            }
            match k {
                K::Lbrace | K::Lparen | K::Lbracket => depth += 1,
                K::Rbrace | K::Rparen | K::Rbracket => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                }
                _ => {}
            }
            self.bump();
        }
    }

    fn enter(&mut self) -> bool {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            if !self.too_deep {
                self.too_deep = true;
                let s = self.span();
                self.error(Code::Misplaced, s, "expression nesting too deep");
            }
            return false;
        }
        true
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    // ----------------------------------------------------------------------------------------
    // Names

    /// `simpleNameCS ::= IDENTIFIER | QuotedSimpleNameCS | this | result`
    fn at_simple_name(&self) -> bool {
        groups::SIMPLE_NAME.contains(&self.peek())
    }

    /// `simpleNameCS`; quoted names absorb following string literals
    /// (`QuotedSimpleNameCS ::= QuotedSimpleNameCS STRING_LITERAL`).
    fn simple_name(&mut self) -> Option<Name> {
        let t = self.tok();
        match t.kind {
            K::Identifier => {
                self.bump();
                Some(Name {
                    text: self.text(t).to_string(),
                    kind: NameKind::Ident,
                    span: t.span,
                })
            }
            K::QuotedIdentifier => {
                self.bump();
                let mut text = self.decode_token(t);
                let mut span = t.span;
                while self.at(K::StringLiteral) {
                    let s = self.bump();
                    text.push_str(&self.decode_token(s));
                    span = span.to(s.span);
                }
                Some(Name {
                    text,
                    kind: NameKind::Quoted,
                    span,
                })
            }
            k if groups::SIMPLE_NAME.contains(&k) => {
                self.bump();
                Some(Name {
                    text: self.text(t).to_string(),
                    kind: NameKind::Keyword(k),
                    span: t.span,
                })
            }
            _ => None,
        }
    }

    fn keyword_name(&mut self) -> Name {
        let t = self.bump();
        Name {
            text: self.text(t).to_string(),
            kind: NameKind::Keyword(t.kind),
            span: t.span,
        }
    }

    /// `unreservedSimpleNameCS ::= simpleNameCS | restrictedKeywordCS` (after `::`).
    fn unreserved_simple_name(&mut self) -> Option<Name> {
        if groups::RESTRICTED_KEYWORD.contains(&self.peek()) {
            return Some(self.keyword_name());
        }
        self.simple_name()
    }

    /// A raw `IDENTIFIER` (declarators, parameter names, scoped identifiers).
    fn ident(&mut self) -> Option<Name> {
        let t = self.eat(K::Identifier)?;
        Some(Name {
            text: self.text(t).to_string(),
            kind: NameKind::Ident,
            span: t.span,
        })
    }

    fn expect_ident(&mut self) -> Name {
        match self.ident() {
            Some(n) => n,
            None => {
                self.expected("IDENTIFIER", false);
                Name {
                    text: String::new(),
                    kind: NameKind::Ident,
                    span: Span::at(self.prev_end()),
                }
            }
        }
    }

    /// `pathNameCS ::= simpleNameCS | pathNameCS '::' unreservedSimpleNameCS`
    fn path_name(&mut self) -> Option<PathName> {
        let first = self.simple_name()?;
        let mut span = first.span;
        let mut segments = vec![first];
        while self.at(K::Coloncolon) && self.nth_is_unreserved_name(1) {
            self.bump();
            let n = self.unreserved_simple_name().unwrap();
            span = span.to(n.span);
            segments.push(n);
        }
        Some(PathName { segments, span })
    }

    fn nth_is_unreserved_name(&self, n: usize) -> bool {
        let k = self.nth(n);
        groups::SIMPLE_NAME.contains(&k) || groups::RESTRICTED_KEYWORD.contains(&k)
    }

    /// `qualifiedNameCS ::= qvtIdentifierCS ('.' qvtIdentifierCS)*`
    fn qualified_name(&mut self) -> Vec<Name> {
        let mut names = Vec::new();
        match self.simple_name() {
            Some(n) => names.push(n),
            None => {
                self.expected("IDENTIFIER", false);
                return names;
            }
        }
        loop {
            if self.at(K::Dot) {
                self.bump();
                match self.simple_name() {
                    Some(n) => names.push(n),
                    None => {
                        self.expected("IDENTIFIER", false);
                        break;
                    }
                }
            } else if !self.at(K::Eof)
                && !self.at_simple_name()
                && groups::SIMPLE_NAME.contains(&self.nth(1))
                && !self.tok().kind.is_keyword()
                && self.peek() != K::Lparen
                && self.peek() != K::Semicolon
                && self.peek() != K::Lbrace
            {
                // `a-b`: LPG repairs this by substituting the expected `.`.
                let t = self.bump();
                let msg = format!("\".\" expected instead of \"{}\"", self.quoted(t));
                self.error(Code::ExpectedInstead, t.span, msg);
                // the name after the repaired token is not part of the result (Eclipse:
                // `import foo-bar.baz` imports `foo.baz`, `transformation a-b()` is `a`)
                let _ = self.simple_name();
            } else {
                break;
            }
        }
        names
    }

    // ----------------------------------------------------------------------------------------
    // Literals

    /// Decodes a string or quoted-name token, reporting invalid escape sequences.
    fn decode_token(&mut self, t: Token) -> String {
        let content = escape::strip_quotes(self.text(t));
        match escape::decode(content) {
            Ok(s) => s,
            Err(raw) => {
                self.error(Code::InvalidEscape, t.span, escape::INVALID_ESCAPE);
                raw
            }
        }
    }

    /// `StringLiteralExpCS ::= STRING_LITERAL | StringLiteralExpCS STRING_LITERAL`; adjacent
    /// literals on the same line are an error (`AbstractQVTParser.extendStringLiteralExpCS`).
    fn string_lit(&mut self) -> Option<StringLit> {
        let first = self.eat(K::StringLiteral)?;
        let mut value = self.decode_token(first);
        let mut parts = vec![first.span];
        let mut prev = first;
        while self.at(K::StringLiteral) {
            let t = self.bump();
            if !self.src.as_bytes()[prev.span.start as usize..t.span.start as usize]
                .iter()
                .any(|&c| c == b'\n' || c == b'\r')
            {
                self.error(
                    Code::StringsOnSameLine,
                    prev.span.to(t.span),
                    "Multiline string literals must be located in different lines!",
                );
            }
            value.push_str(&self.decode_token(t));
            parts.push(t.span);
            prev = t;
        }
        Some(StringLit {
            value,
            span: parts[0].to(*parts.last().unwrap()),
            parts,
        })
    }

    fn expect_string_lit(&mut self) -> StringLit {
        match self.string_lit() {
            Some(s) => s,
            None => {
                self.expected("STRING_LITERAL", false);
                StringLit {
                    value: String::new(),
                    parts: Vec::new(),
                    span: Span::at(self.prev_end()),
                }
            }
        }
    }

    /// Integer literal value as a Java `long` (`USE_LONG_INTEGERS`); overflow is an error and
    /// gives 0 (`AbstractQVTParser.createIntegerLiteralExpCS`).
    fn int_value(&mut self, t: Token) -> i64 {
        let text = self.text(t);
        match text.parse::<i64>() {
            Ok(v) => v,
            Err(_) => {
                self.error(
                    Code::IntegerOverflow,
                    t.span,
                    format!("NumberFormatException: For input string: \"{text}\""),
                );
                0
            }
        }
    }

    // ----------------------------------------------------------------------------------------
    // Types

    /// Whether the current token can start a `typeCS`.
    fn at_type_start(&self) -> bool {
        let k = self.peek();
        groups::PRIMITIVE_TYPE.contains(&k)
            || groups::COLLECTION_TYPE.contains(&k)
            || matches!(k, K::KwTuple | K::KwList | K::KwDict)
            || groups::SIMPLE_NAME.contains(&k)
    }

    /// `typeCS ::= primitiveTypeCS | pathNameCS | collectionTypeCS | tupleTypeCS | listTypeCS |
    /// dictTypeCS`
    fn type_expr(&mut self) -> Option<TypeExpr> {
        let t = self.tok();
        let k = t.kind;
        if groups::PRIMITIVE_TYPE.contains(&k) {
            self.bump();
            return Some(TypeExpr {
                kind: TypeKind::Primitive(k),
                span: t.span,
            });
        }
        if groups::COLLECTION_TYPE.contains(&k) {
            self.bump();
            return Some(self.type_args1(t, |e| TypeKind::Collection(k, Box::new(e))));
        }
        match k {
            K::KwList => {
                self.bump();
                Some(self.type_args1(t, |e| TypeKind::List(Box::new(e))))
            }
            K::KwDict => {
                self.bump();
                self.expect(K::Lparen);
                let a = self.expect_type();
                self.expect(K::Comma);
                let b = self.expect_type();
                self.expect(K::Rparen);
                Some(TypeExpr {
                    kind: TypeKind::Dict(Box::new(a), Box::new(b)),
                    span: t.span.to(Span::at(self.prev_end())),
                })
            }
            K::KwTuple => {
                self.bump();
                Some(self.tuple_type(t))
            }
            _ => {
                let p = self.path_name()?;
                Some(TypeExpr {
                    span: p.span,
                    kind: TypeKind::Path(p),
                })
            }
        }
    }

    fn type_args1(&mut self, kw: Token, make: impl FnOnce(TypeExpr) -> TypeKind) -> TypeExpr {
        self.expect(K::Lparen);
        let e = self.expect_type();
        self.expect(K::Rparen);
        TypeExpr {
            kind: make(e),
            span: kw.span.to(Span::at(self.prev_end())),
        }
    }

    /// `tupleTypeCS ::= Tuple '(' tupleTypePartsCSopt ')'`, parts `simpleNameCS ':' typeCS`.
    fn tuple_type(&mut self, kw: Token) -> TypeExpr {
        self.expect(K::Lparen);
        let mut parts = Vec::new();
        if !self.at(K::Rparen) {
            loop {
                let Some(name) = self.simple_name() else {
                    self.expected("IDENTIFIER", false);
                    break;
                };
                self.expect(K::Colon);
                let ty = self.expect_type();
                let span = name.span.to(ty.span);
                parts.push(VarDecl {
                    name,
                    ty: Some(ty),
                    init: None,
                    span,
                });
                if self.eat(K::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect(K::Rparen);
        TypeExpr {
            kind: TypeKind::Tuple(parts),
            span: kw.span.to(Span::at(self.prev_end())),
        }
    }

    fn expect_type(&mut self) -> TypeExpr {
        match self.type_expr() {
            Some(t) => t,
            None => {
                self.expected("typeCS", false);
                TypeExpr {
                    kind: TypeKind::Error,
                    span: Span::at(self.prev_end()),
                }
            }
        }
    }

    /// `typespec ::= typeCS | typeCS '@' IDENTIFIER`
    fn type_spec(&mut self) -> Option<TypeSpec> {
        let ty = self.type_expr()?;
        let mut span = ty.span;
        let mut extent = None;
        if self.eat(K::AtSign).is_some() {
            let n = self.expect_ident();
            span = span.to(n.span);
            extent = Some(n);
        }
        Some(TypeSpec { ty, extent, span })
    }

    fn expect_type_spec(&mut self) -> TypeSpec {
        match self.type_spec() {
            Some(t) => t,
            None => {
                self.expected("typespec", false);
                let span = Span::at(self.prev_end());
                TypeSpec {
                    ty: TypeExpr {
                        kind: TypeKind::Error,
                        span,
                    },
                    extent: None,
                    span,
                }
            }
        }
    }

    /// `scoped_identifier ::= typeCS2 '::' IDENTIFIER | scoped_identifier2`,
    /// `scoped_identifier2 ::= IDENTIFIER | main | scoped_identifier2 '::' IDENTIFIER`
    fn scoped_name(&mut self) -> Option<ScopedName> {
        let t = self.tok();
        let k = t.kind;
        let is_type2 = groups::PRIMITIVE_TYPE.contains(&k)
            || groups::COLLECTION_TYPE.contains(&k)
            || matches!(k, K::KwTuple | K::KwList | K::KwDict);
        if is_type2 {
            let ty = self.type_expr()?;
            self.expect(K::Coloncolon);
            let n = self.expect_ident();
            let span = ty.span.to(n.span);
            return Some(ScopedName {
                context_type: Some(ty),
                names: vec![n],
                span,
            });
        }
        let first = match k {
            K::Identifier => self.ident().unwrap(),
            K::KwMain => self.keyword_name(),
            _ => return None,
        };
        let mut span = first.span;
        let mut names = vec![first];
        while self.at(K::Coloncolon) {
            self.bump();
            let n = self.expect_ident();
            span = span.to(n.span);
            names.push(n);
        }
        Some(ScopedName {
            context_type: None,
            names,
            span,
        })
    }

    fn expect_scoped_name(&mut self) -> ScopedName {
        match self.scoped_name() {
            Some(s) => s,
            None => {
                self.expected("IDENTIFIER", false);
                ScopedName {
                    context_type: None,
                    names: Vec::new(),
                    span: Span::at(self.prev_end()),
                }
            }
        }
    }
}

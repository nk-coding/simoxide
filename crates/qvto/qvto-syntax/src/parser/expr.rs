//! Expressions and statements (`EssentialOCL.gi`, `ImperativeOCL.gi`, and the expression rules
//! of `QVTOParser.gi`).

use super::Parser;
use crate::ast::*;
use crate::diag::Code;
use crate::generated::{PREFIX_OPS, TokenKind as K, binary_level, groups};
use crate::lexer::Token;
use crate::span::Span;

/// Tokens after which a statement ends (recovery points inside blocks).
const STMT_SYNC: &[K] = &[K::Semicolon, K::Rbrace];

impl<'s> Parser<'s> {
    fn err_expr(&self) -> Expr {
        Expr {
            kind: ExprKind::Error,
            span: Span::at(self.prev_end()),
        }
    }

    fn mk(&self, start: Span, kind: ExprKind) -> Expr {
        Expr {
            kind,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// Whether the current token can start an `OclExpressionCS`.
    pub(super) fn at_expr_start(&self) -> bool {
        let k = self.peek();
        matches!(
            k,
            K::IntegerLiteral
                | K::RealLiteral
                | K::StringLiteral
                | K::KwTrue
                | K::KwFalse
                | K::KwNull
                | K::KwInvalid
                | K::Multiply
                | K::KwSelf
                | K::Lparen
                | K::KwIf
                | K::KwLet
                | K::KwTuple
                | K::KwList
                | K::KwDict
                | K::KwWhile
                | K::KwSwitch
                | K::KwDo
                | K::KwCompute
                | K::KwTry
                | K::KwRaise
                | K::KwNew
                | K::KwBreak
                | K::KwContinue
                | K::KwObject
                | K::KwMap
                | K::KwXmap
                | K::KwLate
                | K::KwReturn
                | K::KwVar
                | K::KwLog
                | K::KwAssert
        ) || PREFIX_OPS.contains(&k)
            || groups::SIMPLE_NAME.contains(&k)
            || groups::PRIMITIVE_TYPE.contains(&k)
            || groups::COLLECTION_TYPE.contains(&k)
            || groups::RESOLVE_OP.contains(&k)
            || groups::RESOLVE_IN_OP.contains(&k)
    }

    /// `OclExpressionCS`, including the imperative forms allowed at this level: `return`,
    /// `var` (one declarator), `log`, `assert` and assignments.
    pub(super) fn expr(&mut self) -> Expr {
        if !self.enter() {
            self.leave();
            self.skip_to(&[K::Semicolon, K::Rbrace]);
            return self.err_expr();
        }
        let e = self.expr_inner(false);
        self.leave();
        e
    }

    fn expr_inner(&mut self, stmt: bool) -> Expr {
        let start = self.span();
        match self.peek() {
            K::KwReturn => {
                // returnExpCS ::= return oclExpressionCSOpt
                self.bump();
                let value = if self.at_expr_start() {
                    Some(Box::new(self.expr()))
                } else {
                    None
                };
                return self.mk(start, ExprKind::Return(value));
            }
            K::KwVar => return self.var_exp(stmt),
            K::KwLog => return self.log_exp(),
            K::KwAssert => return self.assert_exp(),
            _ => {}
        }
        let mut lhs = self.binary(0, stmt);
        // A stray token followed by something that continues the expression: LPG deletes it.
        while stmt && self.is_stray() {
            let n = self.nth(1);
            if binary_level(n) == 0
                && !matches!(n, K::ResetAssign | K::AddAssign | K::Dot | K::Arrow)
            {
                break;
            }
            self.unexpected();
            self.bump();
            let e = self.postfix(lhs);
            lhs = self.binary_rest(start, e, 0);
        }
        if matches!(self.peek(), K::ResetAssign | K::AddAssign) {
            // assignStatementCS ::= primaryExpCS (':=' | '+=') OclExpressionCS
            let add = self.bump().kind == K::AddAssign;
            if !is_primary(&lhs) {
                let msg = "Left side of an assignment must be a primary expression";
                self.error(Code::Misplaced, lhs.span, msg);
            }
            let rhs = if self.at_expr_start() {
                self.expr()
            } else {
                self.expected("OclExpressionCS", true);
                self.err_expr()
            };
            return self.mk(start, ExprKind::Assign(Box::new(Assign { lhs, rhs, add })));
        }
        lhs
    }

    /// Whether the current token cannot follow a complete expression (a candidate for LPG's
    /// deletion repair).
    fn is_stray(&self) -> bool {
        let k = self.peek();
        !(k == K::Eof
            || binary_level(k) > 0
            || matches!(
                k,
                K::Semicolon
                    | K::Comma
                    | K::Rparen
                    | K::Rbrace
                    | K::Rbracket
                    | K::Bar
                    | K::Dotdot
                    | K::ResetAssign
                    | K::AddAssign
                    | K::Dot
                    | K::Arrow
                    | K::Lbracket
                    | K::ExclamationMark
                    | K::KwThen
                    | K::KwElse
                    | K::KwElif
                    | K::KwEndif
                    | K::KwIn
                    | K::KwWhen
                    | K::KwWith
                    | K::KwEnd
                    | K::Lbrace
            ))
    }

    /// A statement: `expression_semi_list_element ::= OclExpressionCS | var_init_group_exp |
    /// IfExpCS_ext`.
    fn stmt(&mut self) -> Expr {
        if !self.enter() {
            self.leave();
            self.skip_to(STMT_SYNC);
            return self.err_expr();
        }
        let e = self.expr_inner(true);
        self.leave();
        e
    }

    /// Binary operators by precedence climbing over the generated levels (all
    /// left-associative). `let` and unary operators are handled by [`Self::unary`].
    fn binary(&mut self, min: u8, stmt: bool) -> Expr {
        let start = self.span();
        let lhs = self.unary(stmt);
        self.binary_rest(start, lhs, min)
    }

    fn binary_rest(&mut self, start: Span, mut lhs: Expr, min: u8) -> Expr {
        loop {
            let op = self.peek();
            let lvl = binary_level(op);
            if lvl == 0 || lvl <= min {
                break;
            }
            self.bump();
            let rhs = if self.at_expr_start() {
                self.binary(lvl, false)
            } else {
                self.expected("OclExpressionCS", true);
                self.err_expr()
            };
            lhs = self.mk(start, ExprKind::Binary(op, Box::new(lhs), Box::new(rhs)));
        }
        lhs
    }

    /// `unaryNotLetCS ::= '-' unaryNotLetCS | not unaryNotLetCS | primaryExpCS`, and
    /// `LetExpCS` as an operand.
    fn unary(&mut self, stmt: bool) -> Expr {
        let start = self.span();
        let k = self.peek();
        if PREFIX_OPS.contains(&k) {
            self.bump();
            if !self.enter() {
                self.leave();
                return self.err_expr();
            }
            let operand = if self.at_expr_start() {
                self.unary(false)
            } else {
                self.expected("OclExpressionCS", true);
                self.err_expr()
            };
            self.leave();
            return self.mk(start, ExprKind::Unary(k, Box::new(operand)));
        }
        if k == K::KwLet {
            return self.let_exp();
        }
        let p = self.primary(stmt);
        self.postfix(p)
    }

    /// `LetExpCS ::= let letVariablesCS in OclExpressionCS` with typed variables (OCL) or
    /// untyped ones (`letExpSubCS3`, QVTo); no mixing.
    fn let_exp(&mut self) -> Expr {
        let start = self.bump().span;
        let mut vars = Vec::new();
        loop {
            let vs = self.span();
            let Some(name) = self.simple_name() else {
                self.expected("IDENTIFIER", false);
                break;
            };
            let ty = if self.eat(K::Colon).is_some() {
                Some(self.expect_type())
            } else {
                None
            };
            let init = if self.expect(K::Equal).is_some() {
                Some((K::Equal, Box::new(self.expr())))
            } else {
                None
            };
            vars.push(VarDecl {
                name,
                ty,
                init,
                span: vs.to(Span::at(self.prev_end())),
            });
            if self.eat(K::Comma).is_none() {
                break;
            }
        }
        if vars.len() > 1
            && vars.iter().any(|v| v.ty.is_some())
            && vars.iter().any(|v| v.ty.is_none())
        {
            let s = start.to(Span::at(self.prev_end()));
            self.error(
                Code::Misplaced,
                s,
                "let variables must be either all typed or all untyped",
            );
        }
        let body = if self.expect(K::KwIn).is_some() {
            self.expr()
        } else {
            self.err_expr()
        };
        self.mk(start, ExprKind::Let(vars, Box::new(body)))
    }

    // ----------------------------------------------------------------------------------------
    // Postfix: navigation, calls, iterators, shorthands

    fn postfix(&mut self, mut e: Expr) -> Expr {
        loop {
            match self.peek() {
                K::Dot => e = self.after_dot(e),
                K::Arrow => e = self.after_arrow(e),
                K::Lbracket => e = self.xselect(e, false),
                K::ExclamationMark if self.nth(1) == K::Lbracket => {
                    self.bump();
                    e = self.xselect(e, true);
                }
                _ => return e,
            }
        }
    }

    /// `primaryExpCS '.' …`
    fn after_dot(&mut self, src: Expr) -> Expr {
        let start = src.span;
        self.bump();
        let k = self.peek();
        if matches!(k, K::KwMap | K::KwXmap) {
            return self.map_call(start, Some((src, NavOp::Dot)));
        }
        if k == K::KwLate || groups::RESOLVE_OP.contains(&k) || groups::RESOLVE_IN_OP.contains(&k) {
            return self.resolve(start, Some((src, NavOp::Dot)));
        }
        if groups::CONCEPTUAL_OPERATION.contains(&k) && self.nth(1) == K::Lparen {
            // primaryExpCS '.' conceptualOperationNameCS '(' argumentsCSopt ')'
            let name = self.keyword_name();
            let args = self.args();
            return self.mk(
                start,
                ExprKind::Call(Box::new(Call {
                    source: Some((src, NavOp::Dot)),
                    qualifier: Vec::new(),
                    name,
                    args,
                })),
            );
        }
        let Some(first) = self.simple_name() else {
            self.expected("simpleNameCS", false);
            return self.mk(
                start,
                ExprKind::Nav(Box::new(Nav {
                    source: src,
                    op: NavOp::Dot,
                    qualifier: Vec::new(),
                    name: Name {
                        text: String::new(),
                        kind: NameKind::Ident,
                        span: Span::at(self.prev_end()),
                    },
                })),
            );
        };
        // primaryExpCS '.' pathNameCS '::' unreservedSimpleNameCS ['(' args ')']
        let mut qualifier = Vec::new();
        let mut name = first;
        while self.at(K::Coloncolon) && self.nth_is_unreserved_name(1) {
            self.bump();
            qualifier.push(name);
            name = self.unreserved_simple_name().unwrap();
        }
        if self.at(K::Lparen) {
            let args = self.args();
            self.mk(
                start,
                ExprKind::Call(Box::new(Call {
                    source: Some((src, NavOp::Dot)),
                    qualifier,
                    name,
                    args,
                })),
            )
        } else {
            self.mk(
                start,
                ExprKind::Nav(Box::new(Nav {
                    source: src,
                    op: NavOp::Dot,
                    qualifier,
                    name,
                })),
            )
        }
    }

    /// `primaryExpCS '->' …`
    fn after_arrow(&mut self, src: Expr) -> Expr {
        let start = src.span;
        let arrow = self.bump();
        let k = self.peek();
        if groups::FOR_OP.contains(&k) {
            return self.for_each(start, src);
        }
        if groups::IMPERATIVE_ITERATOR_12.contains(&k) || groups::IMPERATIVE_ITERATOR_3.contains(&k)
        {
            return self.x_iterate(start, src);
        }
        match k {
            K::KwObject => return self.object_iter(start, src),
            K::KwSwitch => return self.switch_iter(start, src),
            K::KwMap | K::KwXmap => return self.map_call(start, Some((src, NavOp::Arrow))),
            _ => {}
        }
        if k == K::KwLate || groups::RESOLVE_OP.contains(&k) || groups::RESOLVE_IN_OP.contains(&k) {
            return self.resolve(start, Some((src, NavOp::Arrow)));
        }
        let Some(name) = self.simple_name() else {
            self.expected("simpleNameCS", false);
            let _ = arrow;
            return self.mk(start, ExprKind::Error);
        };
        if !self.at(K::Lparen) {
            // IterateExpCS ::= primaryExpCS '->' simpleNameCS (xcollect shorthand)
            return self.mk(
                start,
                ExprKind::Nav(Box::new(Nav {
                    source: src,
                    op: NavOp::Arrow,
                    qualifier: Vec::new(),
                    name,
                })),
            );
        }
        self.bump();
        if self.eat(K::Rparen).is_some() {
            return self.mk(
                start,
                ExprKind::Call(Box::new(Call {
                    source: Some((src, NavOp::Arrow)),
                    qualifier: Vec::new(),
                    name,
                    args: Vec::new(),
                })),
            );
        }
        // IteratorExpCS / IterateExpCS: `(v [, w] | body)`, `(v; acc : T = e | body)`,
        // `(acc : T = e | body)`; otherwise OperationCallExpCS arguments.
        if let Some((vars, acc)) = self.try_iterator_decls() {
            let body = self.expr();
            self.close_paren();
            return self.mk(
                start,
                ExprKind::Iterate(Box::new(Iterate {
                    source: src,
                    name,
                    vars,
                    acc,
                    body,
                })),
            );
        }
        let args = self.arg_list();
        self.close_paren();
        self.mk(
            start,
            ExprKind::Call(Box::new(Call {
                source: Some((src, NavOp::Arrow)),
                qualifier: Vec::new(),
                name,
                args,
            })),
        )
    }

    fn close_paren(&mut self) {
        if self.expect(K::Rparen).is_none() {
            self.skip_to(&[K::Rparen, K::Semicolon]);
            self.eat(K::Rparen);
        }
    }

    /// Iterator variable declarations followed by `|`, or `None` (with the position restored)
    /// when this is not an iterator.
    fn try_iterator_decls(&mut self) -> Option<(Vec<VarDecl>, Option<VarDecl>)> {
        if !self.at_simple_name() {
            return None;
        }
        let k1 = self.nth(1);
        if !matches!(k1, K::Colon | K::Comma | K::Bar | K::Semicolon | K::Equal) {
            return None;
        }
        let cp = self.checkpoint();
        let mut vars = Vec::new();
        let mut acc = None;
        loop {
            let Some(d) = self.iter_var_decl(true) else {
                self.rewind(cp);
                return None;
            };
            vars.push(d);
            if self.eat(K::Comma).is_none() {
                break;
            }
        }
        if self.eat(K::Semicolon).is_some() {
            match self.iter_var_decl(true) {
                Some(d) => acc = Some(d),
                None => {
                    self.rewind(cp);
                    return None;
                }
            }
        }
        if !self.at(K::Bar) {
            self.rewind(cp);
            return None;
        }
        self.bump();
        // Check the shapes the grammar allows.
        if acc.is_none() && vars.len() == 1 && vars[0].init.is_some() {
            acc = vars.pop();
        }
        let ok = match &acc {
            Some(a) => {
                a.ty.is_some()
                    && a.init.is_some()
                    && vars.len() <= 1
                    && vars.iter().all(|v| v.init.is_none())
            }
            None => (1..=2).contains(&vars.len()) && vars.iter().all(|v| v.init.is_none()),
        };
        if !ok {
            let s = cp_span(self, cp);
            self.error(Code::Misplaced, s, "invalid iterator variable declarations");
        }
        Some((vars, acc))
    }

    /// `simpleNameCS [':' typeCS] ['=' OclExpressionCS]`
    fn iter_var_decl(&mut self, allow_init: bool) -> Option<VarDecl> {
        let start = self.span();
        let name = self.simple_name()?;
        let ty = if self.eat(K::Colon).is_some() {
            Some(self.type_expr()?)
        } else {
            None
        };
        let init = if allow_init && self.at(K::Equal) {
            self.bump();
            Some((K::Equal, Box::new(self.expr())))
        } else {
            None
        };
        Some(VarDecl {
            name,
            ty,
            init,
            span: start.to(Span::at(self.prev_end())),
        })
    }

    /// `'(' argumentsCSopt ')'`
    fn args(&mut self) -> Vec<Expr> {
        self.expect(K::Lparen);
        if self.eat(K::Rparen).is_some() {
            return Vec::new();
        }
        let args = self.arg_list();
        self.close_paren();
        args
    }

    /// `argumentsCS ::= OclExpressionCS (',' OclExpressionCS)*`
    fn arg_list(&mut self) -> Vec<Expr> {
        let mut args = Vec::new();
        loop {
            if self.at_expr_start() {
                args.push(self.expr());
            } else {
                self.expected("OclExpressionCS", false);
                self.skip_to(&[K::Comma, K::Rparen, K::Semicolon]);
            }
            if self.eat(K::Comma).is_none() {
                break;
            }
        }
        args
    }

    /// `IterateExpCS ::= primaryExpCS exclamationOpt '[' declarator_vsepOpt OclExpressionCS ']'`
    fn xselect(&mut self, src: Expr, one: bool) -> Expr {
        let start = src.span;
        self.bump();
        let var = if self.at(K::Identifier) && self.nth(1) == K::Bar {
            let n = self.ident();
            self.bump();
            n
        } else {
            None
        };
        let cond = self.expr();
        if self.expect(K::Rbracket).is_none() {
            self.skip_to(&[K::Rbracket, K::Semicolon]);
            self.eat(K::Rbracket);
        }
        self.mk(
            start,
            ExprKind::XSelect(Box::new(XSelect {
                source: src,
                one,
                var,
                cond,
            })),
        )
    }

    /// `forExpCS ::= forOpCode '(' forExpDeclaratorList forExpConditionOpt ')' expression_block`
    fn for_each(&mut self, start: Span, src: Expr) -> Expr {
        let kind = self.bump().kind;
        self.expect(K::Lparen);
        let mut vars = vec![self.expect_ident()];
        while self.eat(K::Comma).is_some() {
            vars.push(self.expect_ident());
        }
        let cond = if self.eat(K::Bar).is_some() {
            Some(self.expr())
        } else {
            None
        };
        self.close_paren();
        let body = self.block();
        self.mk(
            start,
            ExprKind::ForEach(Box::new(ForEach {
                source: src,
                kind,
                vars,
                cond,
                body,
            })),
        )
    }

    /// `imperativeIterateExpCS ::= imperativeIteratorExpCSToken12 '(' imperativeIterContents12
    /// ')' | imperativeIteratorExpCSToken3 '(' imperativeIterContents3 ')'`
    fn x_iterate(&mut self, start: Span, src: Expr) -> Expr {
        let kind = self.bump().kind;
        self.expect(K::Lparen);
        let mut vars = Vec::new();
        let mut target = None;
        let three = groups::IMPERATIVE_ITERATOR_3.contains(&kind);
        if three {
            // variableDeclarationListCS ';' initializedVariableCS '|' OclExpressionCS
            loop {
                match self.iter_var_decl(false) {
                    Some(d) => vars.push(d),
                    None => {
                        self.expected("IDENTIFIER", false);
                        break;
                    }
                }
                if self.eat(K::Comma).is_none() {
                    break;
                }
            }
            self.expect(K::Semicolon);
            let ts = self.span();
            match self.simple_name() {
                Some(name) => {
                    let ty = if self.eat(K::Colon).is_some() {
                        Some(self.expect_type())
                    } else {
                        None
                    };
                    self.expect(K::Equal);
                    let init = Some((K::Equal, Box::new(self.expr())));
                    target = Some(VarDecl {
                        name,
                        ty,
                        init,
                        span: ts.to(Span::at(self.prev_end())),
                    });
                }
                None => self.expected("IDENTIFIER", false),
            }
            self.expect(K::Bar);
        } else {
            // OclExpressionCS | uninitializedVariableCS '|' OclExpressionCS |
            // simpleNameCS ',' variableDeclarationListCS '|' OclExpressionCS
            if self.at_simple_name() && matches!(self.nth(1), K::Colon | K::Comma | K::Bar) {
                let cp = self.checkpoint();
                let mut ok = true;
                loop {
                    match self.iter_var_decl(false) {
                        Some(d) => vars.push(d),
                        None => {
                            ok = false;
                            break;
                        }
                    }
                    if self.eat(K::Comma).is_none() {
                        break;
                    }
                }
                if ok && self.at(K::Bar) {
                    self.bump();
                } else {
                    vars.clear();
                    self.rewind(cp);
                }
            }
        }
        let body = self.expr();
        self.close_paren();
        self.mk(
            start,
            ExprKind::XIterate(Box::new(XIterate {
                source: src,
                kind,
                vars,
                target,
                body,
            })),
        )
    }

    /// `IterateExpCS ::= primaryExpCS '->' object '(' uninitializedVariableCS ')' objectDeclCS
    /// expression_block`
    fn object_iter(&mut self, start: Span, src: Expr) -> Expr {
        self.bump();
        self.expect(K::Lparen);
        let var = match self.iter_var_decl(false) {
            Some(v) => v,
            None => {
                self.expected("IDENTIFIER", false);
                VarDecl {
                    name: Name {
                        text: String::new(),
                        kind: NameKind::Ident,
                        span: Span::at(self.prev_end()),
                    },
                    ty: None,
                    init: None,
                    span: Span::at(self.prev_end()),
                }
            }
        };
        self.close_paren();
        let decl = self.object_decl();
        let body = self.block();
        self.mk(
            start,
            ExprKind::ObjectIter(Box::new(ObjectIter {
                source: src,
                var,
                decl,
                body,
            })),
        )
    }

    /// `IterateExpCS ::= primaryExpCS '->' switch '(' switchDeclaratorCS ')' switchBodyExpCS`
    fn switch_iter(&mut self, start: Span, src: Expr) -> Expr {
        self.bump();
        self.expect(K::Lparen);
        // switchDeclaratorCS ::= declarator | IDENTIFIER | IDENTIFIER '=' OclExpressionCS
        let ds = self.span();
        let name = self.expect_ident();
        let ty = if self.eat(K::Colon).is_some() {
            Some(self.expect_type())
        } else {
            None
        };
        let init = if matches!(self.peek(), K::Equal | K::ResetAssign) {
            let op = self.bump().kind;
            Some((op, Box::new(self.expr())))
        } else {
            None
        };
        let var = VarDecl {
            name,
            ty,
            init,
            span: ds.to(Span::at(self.prev_end())),
        };
        self.close_paren();
        let body = self.switch_body();
        self.mk(
            start,
            ExprKind::SwitchIter(Box::new(SwitchIter {
                source: src,
                var,
                body,
            })),
        )
    }

    // ----------------------------------------------------------------------------------------
    // Primaries

    fn primary(&mut self, stmt: bool) -> Expr {
        let t = self.tok();
        let start = t.span;
        let k = t.kind;
        match k {
            K::IntegerLiteral => {
                self.bump();
                let v = self.int_value(t);
                return self.mk(start, ExprKind::Int(v));
            }
            K::RealLiteral => {
                self.bump();
                return self.mk(start, ExprKind::Real(self.text(t).to_string()));
            }
            K::StringLiteral => {
                let s = self.string_lit().unwrap();
                return self.mk(start, ExprKind::Str(s));
            }
            K::KwTrue | K::KwFalse => {
                self.bump();
                return self.mk(start, ExprKind::Bool(k == K::KwTrue));
            }
            K::KwNull => {
                self.bump();
                return self.mk(start, ExprKind::Null);
            }
            K::KwInvalid => {
                self.bump();
                return self.mk(start, ExprKind::Invalid);
            }
            K::Multiply => {
                self.bump();
                return self.mk(start, ExprKind::Unlimited);
            }
            K::KwSelf => {
                self.bump();
                return self.mk(start, ExprKind::SelfExp);
            }
            K::Lparen => return self.paren(start),
            _ => {}
        }
        if groups::SIMPLE_NAME.contains(&k) {
            return self.primary_name(start);
        }
        self.primary_other(t, stmt)
    }

    // The rarer primaries are kept out of `primary`'s frame: it is on the recursive path of
    // every nesting level, and debug builds give every temporary its own stack slot.
    #[inline(never)]
    fn paren(&mut self, start: Span) -> Expr {
        self.bump();
        let inner = self.expr();
        if self.expect(K::Rparen).is_none() {
            self.skip_to(&[K::Rparen, K::Semicolon]);
            self.eat(K::Rparen);
        }
        self.mk(start, ExprKind::Paren(Box::new(inner)))
    }

    /// Dispatch for the keyword-introduced primaries; each has its own function so that only
    /// the construct actually parsed puts its frame on the stack.
    #[inline(never)]
    fn primary_other(&mut self, t: Token, stmt: bool) -> Expr {
        let start = t.span;
        let k = t.kind;
        match k {
            K::KwIf => self.if_exp(stmt),
            K::KwWhile => self.while_exp(),
            K::KwSwitch => self.switch_exp(start),
            K::KwDo => self.do_exp(start),
            K::KwCompute => self.compute(),
            K::KwTry => self.try_exp(),
            K::KwRaise => self.raise(),
            K::KwNew => self.new_exp(start),
            K::KwBreak => {
                self.bump();
                self.mk(start, ExprKind::Break)
            }
            K::KwContinue => {
                self.bump();
                self.mk(start, ExprKind::Continue)
            }
            K::KwObject => self.object_exp(t),
            K::KwMap | K::KwXmap => self.map_call(start, None),
            K::KwTuple => self.tuple_exp(t),
            K::KwList | K::KwDict => self.list_or_dict(t),
            _ if groups::PRIMITIVE_TYPE.contains(&k) => {
                self.bump();
                self.mk(
                    start,
                    ExprKind::TypeLit(TypeExpr {
                        kind: TypeKind::Primitive(k),
                        span: t.span,
                    }),
                )
            }
            _ if groups::COLLECTION_TYPE.contains(&k) => self.collection_exp(t),
            _ if k == K::KwLate
                || groups::RESOLVE_OP.contains(&k)
                || groups::RESOLVE_IN_OP.contains(&k) =>
            {
                self.resolve(start, None)
            }
            _ => {
                self.expected("OclExpressionCS", false);
                self.mk(Span::at(start.start), ExprKind::Error)
            }
        }
    }

    #[inline(never)]
    fn switch_exp(&mut self, start: Span) -> Expr {
        self.bump();
        let body = self.switch_body();
        self.mk(start, ExprKind::Switch(Box::new(body)))
    }

    #[inline(never)]
    fn do_exp(&mut self, start: Span) -> Expr {
        self.bump();
        let b = self.block();
        self.mk(start, ExprKind::Do(b))
    }

    /// `newExpCS ::= new typespec '(' argumentsCSopt ')'`
    #[inline(never)]
    fn new_exp(&mut self, start: Span) -> Expr {
        self.bump();
        let ty = self.expect_type_spec();
        let args = self.args();
        self.mk(start, ExprKind::New(Box::new(New { ty, args })))
    }

    /// `outExpCS ::= object objectDeclCS expression_block`
    #[inline(never)]
    fn object_exp(&mut self, t: Token) -> Expr {
        let start = t.span;
        self.bump();
        if !self.at_simple_name() && !self.at_type_start() && !self.at(K::KwSelf) {
            let msg = format!("\"objectDeclCS\" expected after \"{}\"", self.quoted(t));
            self.error(Code::Expected, t.span, msg);
            if self.at(K::Lbrace) {
                // LPG's repair keeps no object expression: Eclipse's AST has nothing for the
                // statement (case `missingObjectType`), so the block is consumed and dropped
                let _ = self.block();
            }
            return self.mk(start, ExprKind::Error);
        }
        let decl = self.object_decl();
        let body = self.block();
        self.mk(start, ExprKind::Object(Box::new(ObjectExp { decl, body })))
    }

    /// `TupleLiteralExpCS ::= Tuple '{' TupleLiteralPartsCS '}'` or a tuple type literal.
    #[inline(never)]
    fn tuple_exp(&mut self, t: Token) -> Expr {
        let start = t.span;
        self.bump();
        if !self.at(K::Lbrace) {
            let ty = self.tuple_type(t);
            return self.mk(start, ExprKind::TypeLit(ty));
        }
        self.bump();
        let mut parts = Vec::new();
        loop {
            let ps = self.span();
            let Some(name) = self.simple_name() else {
                self.expected("IDENTIFIER", false);
                break;
            };
            let ty = if self.eat(K::Colon).is_some() {
                Some(self.expect_type())
            } else {
                None
            };
            let init = if self.expect(K::Equal).is_some() {
                Some((K::Equal, Box::new(self.expr())))
            } else {
                None
            };
            parts.push(VarDecl {
                name,
                ty,
                init,
                span: ps.to(Span::at(self.prev_end())),
            });
            if self.eat(K::Comma).is_none() {
                break;
            }
        }
        self.close_brace();
        self.mk(start, ExprKind::TupleLit(parts))
    }

    /// `listLiteralCS ::= List '{' CollectionLiteralPartsCSopt '}'`,
    /// `dictLiteralCS ::= Dict '{' dictLiteralPartListCSopt '}'`
    #[inline(never)]
    fn list_or_dict(&mut self, t: Token) -> Expr {
        let start = t.span;
        if self.nth(1) != K::Lbrace {
            self.bump();
            self.expected("{", false);
            return self.mk(start, ExprKind::Error);
        }
        self.bump();
        if t.kind == K::KwList {
            let parts = self.collection_parts();
            return self.mk(start, ExprKind::ListLit(parts));
        }
        self.bump();
        let mut parts = Vec::new();
        if !self.at(K::Rbrace) {
            loop {
                let key = self.dict_key();
                self.expect(K::Equal);
                let value = self.expr();
                parts.push(DictPart { key, value });
                if self.eat(K::Comma).is_none() {
                    break;
                }
            }
        }
        self.close_brace();
        self.mk(start, ExprKind::DictLit(parts))
    }

    /// `CollectionLiteralExpCS ::= CollectionTypeIdentifierCS '{' CollectionLiteralPartsCSopt '}'`
    /// or a collection type literal (`Set(T){…}` is not allowed by QVTo).
    #[inline(never)]
    fn collection_exp(&mut self, t: Token) -> Expr {
        let start = t.span;
        if self.nth(1) == K::Lbrace {
            self.bump();
            let parts = self.collection_parts();
            return self.mk(start, ExprKind::CollectionLit(t.kind, parts));
        }
        let ty = self.expect_type();
        self.mk(start, ExprKind::TypeLit(ty))
    }

    #[inline(never)]
    fn primary_name(&mut self, start: Span) -> Expr {
        {
            let name = self.simple_name().unwrap();
            if self.at(K::Coloncolon) && self.nth_is_unreserved_name(1) {
                // pathNameCS '::' unreservedSimpleNameCS ['(' args ')']
                let mut segments = vec![name];
                while self.at(K::Coloncolon) && self.nth_is_unreserved_name(1) {
                    self.bump();
                    segments.push(self.unreserved_simple_name().unwrap());
                }
                if self.at(K::Lparen) {
                    let name = segments.pop().unwrap();
                    let args = self.args();
                    return self.mk(
                        start,
                        ExprKind::Call(Box::new(Call {
                            source: None,
                            qualifier: segments,
                            name,
                            args,
                        })),
                    );
                }
                let span = start.to(Span::at(self.prev_end()));
                return self.mk(start, ExprKind::Path(PathName { segments, span }));
            }
            if self.at(K::Lparen) {
                let args = self.args();
                return self.mk(
                    start,
                    ExprKind::Call(Box::new(Call {
                        source: None,
                        qualifier: Vec::new(),
                        name,
                        args,
                    })),
                );
            }
            self.mk(start, ExprKind::Name(name))
        }
    }

    fn close_brace(&mut self) {
        if self.expect(K::Rbrace).is_none() {
            self.skip_to(&[K::Rbrace, K::Semicolon]);
            self.eat(K::Rbrace);
        }
    }

    /// `literalSimpleCS -> PrimitiveLiteralExpCS` (dictionary keys).
    fn dict_key(&mut self) -> Expr {
        let t = self.tok();
        match t.kind {
            K::IntegerLiteral
            | K::RealLiteral
            | K::StringLiteral
            | K::KwTrue
            | K::KwFalse
            | K::KwNull
            | K::KwInvalid
            | K::Multiply => self.primary(false),
            _ => {
                self.expected("literalSimpleCS", false);
                self.skip_to(&[K::Equal, K::Comma, K::Rbrace]);
                self.err_expr()
            }
        }
    }

    /// `'{' CollectionLiteralPartsCSopt '}'` with parts `e` or `e '..' e`.
    fn collection_parts(&mut self) -> Vec<CollectionPart> {
        self.bump();
        let mut parts = Vec::new();
        if !self.at(K::Rbrace) {
            loop {
                let a = self.expr();
                if self.eat(K::Dotdot).is_some() {
                    let b = self.expr();
                    parts.push(CollectionPart::Range(a, b));
                } else {
                    parts.push(CollectionPart::Item(a));
                }
                if self.eat(K::Comma).is_none() {
                    break;
                }
            }
        }
        self.close_brace();
        parts
    }

    /// `objectDeclCS ::= typespec | objectIdentifierCS ':' typespecOpt`
    fn object_decl(&mut self) -> ObjectDecl {
        let start = self.span();
        let mut var = None;
        if groups::OBJECT_IDENTIFIER.contains(&self.peek()) && self.nth(1) == K::Colon {
            let t = self.bump();
            let kind = if t.kind == K::Identifier {
                NameKind::Ident
            } else {
                NameKind::Keyword(t.kind)
            };
            var = Some(Name {
                text: self.text(t).to_string(),
                kind,
                span: t.span,
            });
            self.bump();
            let ty = if self.at(K::Lbrace) {
                None
            } else {
                Some(self.expect_type_spec())
            };
            return ObjectDecl {
                var,
                ty,
                span: start.to(Span::at(self.prev_end())),
            };
        }
        let ty = Some(self.expect_type_spec());
        ObjectDecl {
            var,
            ty,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `featureMappingCallExpCS ::= (map|xmap) simpleNameCS ['::' simpleNameCS] '(' args ')'`
    /// (with a source) and `mappingCallExpCS ::= (map|xmap) pathNameCS '(' args ')'`.
    fn map_call(&mut self, start: Span, source: Option<(Expr, NavOp)>) -> Expr {
        let xmap = self.bump().kind == K::KwXmap;
        let name = match self.path_name() {
            Some(p) => p,
            None => {
                self.expected("simpleNameCS", false);
                PathName {
                    segments: Vec::new(),
                    span: Span::at(self.prev_end()),
                }
            }
        };
        if source.is_some() && name.segments.len() > 2 {
            self.error(
                Code::Misplaced,
                name.span,
                "mapping call name may have at most one qualifier",
            );
        }
        let args = self.args();
        self.mk(
            start,
            ExprKind::MapCall(Box::new(MapCall {
                source,
                xmap,
                name,
                args,
            })),
        )
    }

    /// `resolveExpCS ::= lateOpt resolveOp '(' resolveOpArgsExpCSOpt ')'` and
    /// `resolveInExpCS ::= lateOpt resolveInOp '(' scoped_identifier [',' resolveOpArgsExpCS] ')'`
    fn resolve(&mut self, start: Span, source: Option<(Expr, NavOp)>) -> Expr {
        let late = self.eat(K::KwLate).is_some();
        let k = self.peek();
        let is_in = groups::RESOLVE_IN_OP.contains(&k);
        if !is_in && !groups::RESOLVE_OP.contains(&k) {
            self.expected("resolve", false);
            return self.mk(start, ExprKind::Error);
        }
        let op = self.bump().kind;
        self.expect(K::Lparen);
        let mut mapping = None;
        let mut target = None;
        if is_in {
            mapping = Some(self.expect_scoped_name());
            if self.eat(K::Comma).is_some() {
                target = self.resolve_target();
                if target.is_none() {
                    self.expected("typeCS", false);
                }
            }
        } else if !self.at(K::Rparen) {
            target = self.resolve_target();
            if target.is_none() {
                self.expected("typeCS", false);
            }
        }
        self.close_paren();
        self.mk(
            start,
            ExprKind::Resolve(Box::new(Resolve {
                source,
                late,
                op,
                mapping,
                target,
            })),
        )
    }

    /// `resolveOpArgsExpCS ::= IDENTIFIEROpt typeCS resolveConditionOpt`
    fn resolve_target(&mut self) -> Option<ResolveTarget> {
        let var = if self.at(K::Identifier) && self.nth(1) == K::Colon {
            let n = self.ident();
            self.bump();
            n
        } else {
            None
        };
        let ty = match self.type_expr() {
            Some(t) => t,
            None if var.is_some() => self.expect_type(),
            None => return None,
        };
        let cond = if self.eat(K::Bar).is_some() {
            Some(self.expr())
        } else {
            None
        };
        Some(ResolveTarget { var, ty, cond })
    }

    // ----------------------------------------------------------------------------------------
    // if

    /// `IfExpCS ::= if OclExpressionCS then ifExpBodyCS ifElif_listOpt ifElseOpt endif` and, at
    /// statement level, `IfExpCS_ext ::= if '(' OclExpressionCS ')' notUMinus_ifExpBodyCS …
    /// [endif]`. The two are told apart after the condition: a parenthesised condition not
    /// followed by `then` is the C-style form (LPG resolves this by backtracking).
    fn if_exp(&mut self, stmt: bool) -> Expr {
        let start = self.bump().span;
        let cond = self.expr();
        if self.at(K::KwThen) || !matches!(cond.kind, ExprKind::Paren(_)) {
            return self.ocl_if(start, cond);
        }
        if !stmt {
            // IfExpCS_ext is only an expression_semi_list_element (and an if branch).
            self.expected("then", true);
        }
        self.c_if(start, cond)
    }

    fn ocl_if(&mut self, start: Span, cond: Expr) -> Expr {
        if self.expect(K::KwThen).is_none() {
            self.skip_to(&[K::KwThen, K::KwEndif, K::Semicolon]);
            if self.eat(K::KwThen).is_none() {
                self.eat(K::KwEndif);
                return self.mk(
                    start,
                    ExprKind::If(Box::new(IfExp {
                        cond,
                        then: self.err_expr(),
                        elifs: Vec::new(),
                        els: None,
                        c_style: false,
                        endif: true,
                    })),
                );
            }
        }
        let then = self.if_body();
        let mut elifs = Vec::new();
        while self.eat(K::KwElif).is_some() {
            let c = self.expr();
            self.expect(K::KwThen);
            let b = self.if_body();
            elifs.push((c, b));
        }
        let els = if self.eat(K::KwElse).is_some() {
            Some(self.if_body())
        } else {
            None
        };
        self.expect(K::KwEndif);
        self.mk(
            start,
            ExprKind::If(Box::new(IfExp {
                cond,
                then,
                elifs,
                els,
                c_style: false,
                endif: true,
            })),
        )
    }

    /// `ifExpBodyCS -> OclExpressionCS | expression_block`
    fn if_body(&mut self) -> Expr {
        if self.at(K::Lbrace) {
            let s = self.span();
            let b = self.block();
            return self.mk(s, ExprKind::Block(b));
        }
        self.expr()
    }

    fn c_if(&mut self, start: Span, cond: Expr) -> Expr {
        let cond = match cond.kind {
            ExprKind::Paren(inner) => *inner,
            _ => unreachable!(),
        };
        let then = self.c_if_body();
        let mut elifs = Vec::new();
        // ifElif_ext_listElem ::= elif '(' OclExpressionCS ')' notUMinus_ifExpBodyCS_full
        while self.at(K::KwElif) {
            self.bump();
            self.expect(K::Lparen);
            let c = self.expr();
            self.close_paren();
            let b = self.c_if_body();
            elifs.push((c, b));
        }
        let els = if self.eat(K::KwElse).is_some() {
            Some(self.c_if_body())
        } else {
            None
        };
        let endif = self.eat(K::KwEndif).is_some();
        self.mk(
            start,
            ExprKind::If(Box::new(IfExp {
                cond,
                then,
                elifs,
                els,
                c_style: true,
                endif,
            })),
        )
    }

    /// `notUMinus_ifExpBodyCS -> notUMinus_statement | IfExpCS_ext`: a block, a C-style `if`,
    /// or an expression that does not start with unary minus or `*`.
    fn c_if_body(&mut self) -> Expr {
        if self.at(K::Lbrace) {
            let s = self.span();
            let b = self.block();
            return self.mk(s, ExprKind::Block(b));
        }
        if self.at(K::Minus) || self.at(K::Multiply) {
            self.unexpected();
        }
        if !self.at_expr_start() {
            self.expected("notUMinus_ifExpBodyCS", true);
            return self.err_expr();
        }
        self.stmt()
    }

    // ----------------------------------------------------------------------------------------
    // Imperative expressions

    /// `whileExpCS ::= while '(' declarator1 ';' OclExpressionCS ')' whileBodyCS |
    /// while '(' assignStatementCS ';' OclExpressionCS ')' whileBodyCS |
    /// while '(' OclExpressionCS ')' whileBodyCS`
    fn while_exp(&mut self) -> Expr {
        let start = self.bump().span;
        self.expect(K::Lparen);
        let mut init = None;
        let cond;
        if self.at(K::Identifier) && self.nth(1) == K::Colon {
            let d = self.declarator();
            self.expect(K::Semicolon);
            init = Some(WhileInit::Decl(d));
            cond = self.expr();
        } else {
            let e = self.expr();
            if self.eat(K::Semicolon).is_some() {
                if let ExprKind::Assign(a) = &e.kind {
                    // AbstractQVTParser.getVariableFromAssignment
                    if a.add {
                        self.error(
                            Code::InvalidAdditiveAssignment,
                            e.span,
                            "Invalid use of additive assignment",
                        );
                    }
                    let plain =
                        matches!(&a.lhs.kind, ExprKind::Name(n) if n.kind == NameKind::Ident);
                    if !plain {
                        self.error(
                            Code::IdentifierExpectedOnLeftSide,
                            a.lhs.span,
                            "Left side expression must be an identifier",
                        );
                    }
                } else {
                    self.error(
                        Code::Misplaced,
                        e.span,
                        "Assignment or variable declaration expected in while header",
                    );
                }
                init = Some(WhileInit::Assign(e));
                cond = self.expr();
            } else {
                cond = e;
            }
        }
        self.close_paren();
        let body = self.block();
        self.mk(start, ExprKind::While(Box::new(While { init, cond, body })))
    }

    /// `declarator1 ::= IDENTIFIER ':' typeCS [('=' | ':=') OclExpressionCS]`,
    /// `declarator2 ::= IDENTIFIER ':=' OclExpressionCS`
    fn declarator(&mut self) -> VarDecl {
        let start = self.span();
        let name = self.expect_ident();
        let ty = if self.eat(K::Colon).is_some() {
            Some(self.expect_type())
        } else {
            None
        };
        let init = if (ty.is_some() && self.at(K::Equal)) || self.at(K::ResetAssign) {
            let op = self.bump().kind;
            Some((op, Box::new(self.expr())))
        } else {
            if ty.is_none() {
                self.expected(":=", false);
            }
            None
        };
        VarDecl {
            name,
            ty,
            init,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `computeExpCS ::= compute '(' declarator ')' expression_block`
    fn compute(&mut self) -> Expr {
        let start = self.bump().span;
        self.expect(K::Lparen);
        let decl = self.declarator();
        self.close_paren();
        let body = self.block();
        self.mk(start, ExprKind::Compute(Box::new(Compute { decl, body })))
    }

    /// `TryExpCS ::= try expression_block CatchExp_list`, `CatchExp ::= except '('
    /// except_type_list_opt ')' expression_block | except '(' IDENTIFIER ':' type_list ')'
    /// expression_block`
    fn try_exp(&mut self) -> Expr {
        let start = self.bump().span;
        let body = self.block();
        let mut catches = Vec::new();
        while self.at(K::KwExcept) {
            let cs = self.bump().span;
            self.expect(K::Lparen);
            let mut var = None;
            if self.at(K::Identifier) && self.nth(1) == K::Colon {
                var = self.ident();
                self.bump();
            }
            let mut types = Vec::new();
            if !self.at(K::Rparen) {
                types.push(self.expect_type());
                while self.eat(K::Comma).is_some() {
                    types.push(self.expect_type());
                }
            }
            self.close_paren();
            let b = self.block();
            catches.push(Catch {
                var,
                types,
                body: b,
                span: cs.to(Span::at(self.prev_end())),
            });
        }
        if catches.is_empty() {
            self.expected("except", false);
        }
        self.mk(start, ExprKind::Try(Box::new(Try { body, catches })))
    }

    /// `RaiseExpCS ::= raise pathNameCS raise_arg_opt | raise StringLiteralExpCS`
    fn raise(&mut self) -> Expr {
        let start = self.bump().span;
        let exception = if self.at(K::StringLiteral) {
            let s = self.string_lit().unwrap();
            Expr {
                span: s.span,
                kind: ExprKind::Str(s),
            }
        } else if let Some(p) = self.path_name() {
            let span = p.span;
            if p.segments.len() == 1 {
                Expr {
                    kind: ExprKind::Name(p.segments.into_iter().next().unwrap()),
                    span,
                }
            } else {
                Expr {
                    kind: ExprKind::Path(p),
                    span,
                }
            }
        } else {
            let t = self.tok();
            if t.kind == K::Eof {
                self.eof_error();
            } else {
                let msg = format!("token \"{}\" is invalid", self.quoted(t));
                self.error(Code::InvalidToken, t.span, msg);
                self.skip_to(STMT_SYNC);
            }
            return self.mk(start, ExprKind::Error);
        };
        let mut arg = None;
        if !matches!(exception.kind, ExprKind::Str(_)) && self.eat(K::Lparen).is_some() {
            let a = if self.at(K::Rparen) {
                None
            } else {
                Some(self.expr())
            };
            self.close_paren();
            arg = Some(a);
        }
        self.mk(start, ExprKind::Raise(Box::new(Raise { exception, arg })))
    }

    /// `logExpCS ::= log '(' argumentsCSopt ')' logWhenExpOpt`
    fn log_exp(&mut self) -> Expr {
        let start = self.bump().span;
        let args = self.args();
        let when = if self.eat(K::KwWhen).is_some() {
            Some(self.expr())
        } else {
            None
        };
        self.mk(start, ExprKind::Log(Box::new(Log { args, when })))
    }

    /// `assertExpCS ::= assert severityKindCSOpt '(' OclExpressionCS ')' assertWithLogExpOpt`
    fn assert_exp(&mut self) -> Expr {
        let start = self.bump().span;
        let severity = if self.at_simple_name() {
            self.simple_name()
        } else {
            None
        };
        self.expect(K::Lparen);
        let cond = self.expr();
        self.close_paren();
        let log = if self.eat(K::KwWith).is_some() {
            if self.at(K::KwLog) {
                Some(self.log_exp())
            } else {
                self.expected("log", false);
                None
            }
        } else {
            None
        };
        self.mk(
            start,
            ExprKind::Assert(Box::new(Assert {
                severity,
                cond,
                log,
            })),
        )
    }

    /// `var_init_exp ::= var var_init_declarator | var '(' var_init_declarator ')'` and, at
    /// statement level, `var_init_group_exp ::= var var_init_declarator_list |
    /// var '(' var_init_declarator_list ')'`.
    fn var_exp(&mut self, stmt: bool) -> Expr {
        let start = self.bump().span;
        let parens = self.eat(K::Lparen).is_some();
        let mut decls = vec![self.var_declarator()];
        while (stmt || parens) && self.at(K::Comma) {
            self.bump();
            decls.push(self.var_declarator());
        }
        if parens {
            self.close_paren();
        }
        if decls.len() > 1 && !stmt {
            let s = start.to(Span::at(self.prev_end()));
            self.error(
                Code::Misplaced,
                s,
                "multiple variable declarations are only allowed as statements",
            );
        }
        self.mk(start, ExprKind::Var(VarExp { decls, parens }))
    }

    /// `var_init_declarator ::= IDENTIFIER [':' typeCS] [var_init_op OclExpressionCS]` (type or
    /// initialiser required).
    fn var_declarator(&mut self) -> VarDecl {
        let start = self.span();
        let name = self.expect_ident();
        let ty = if self.eat(K::Colon).is_some() {
            Some(self.expect_type())
        } else {
            None
        };
        let init = if self.at_any(groups::VAR_INIT_OP) {
            let op = self.bump().kind;
            let e = if self.at_expr_start() {
                self.expr()
            } else {
                self.expected("OclExpressionCS", true);
                self.err_expr()
            };
            Some((op, Box::new(e)))
        } else {
            if ty.is_none() {
                self.expected(":=", false);
            }
            None
        };
        VarDecl {
            name,
            ty,
            init,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `switchBodyExpCS ::= '{' switchAltExpCSList switchElseExpCSOpt '}'`
    fn switch_body(&mut self) -> SwitchBody {
        let start = self.span();
        let mut alts = Vec::new();
        let mut els = None;
        if self.expect(K::Lbrace).is_none() {
            return SwitchBody {
                alts,
                els,
                span: start,
            };
        }
        loop {
            let s = self.span();
            match self.peek() {
                K::KwCase => {
                    // switchAltExpCS ::= case '(' OclExpressionCS ')' expression_statement
                    self.bump();
                    self.expect(K::Lparen);
                    let cond = self.expr();
                    self.close_paren();
                    let body = self.expression_statement();
                    alts.push(SwitchAlt {
                        cond,
                        body,
                        deprecated: false,
                        span: s.to(Span::at(self.prev_end())),
                    });
                }
                K::Lparen => {
                    // switchAltExpCS ::= '(' OclExpressionCS ')' '?' OclExpressionCS ';'
                    self.bump();
                    let cond = self.expr();
                    self.close_paren();
                    self.expect(K::Questionmark);
                    let body = self.expr();
                    self.expect(K::Semicolon);
                    let span = s.to(Span::at(self.prev_end()));
                    // AbstractQVTParser.createSwitchAltExpCSDeprecated: condition start to body end.
                    self.warning(
                        Code::DeprecatedSwitchAlt,
                        cond.span.to(body.span),
                        "Syntax is deprecated, use \" ''case'' ''('' <expression> '')'' <expression_statement> \" instead",
                    );
                    alts.push(SwitchAlt {
                        cond,
                        body,
                        deprecated: true,
                        span,
                    });
                }
                _ => break,
            }
        }
        if alts.is_empty() {
            self.expected("case", false);
        }
        if self.at(K::KwElse) {
            let s = self.bump().span;
            if self.eat(K::Questionmark).is_some() {
                // switchElseExpCS ::= else '?' OclExpressionCS ';'
                let body = self.expr();
                self.expect(K::Semicolon);
                let span = s.to(Span::at(self.prev_end()));
                self.warning(
                    Code::DeprecatedSwitchElse,
                    span,
                    "Syntax is deprecated, use \" 'else' <expression_statement> \" instead",
                );
                els = Some(SwitchElse {
                    body,
                    deprecated: true,
                    span,
                });
            } else {
                let body = self.expression_statement();
                els = Some(SwitchElse {
                    body,
                    deprecated: false,
                    span: s.to(Span::at(self.prev_end())),
                });
            }
        }
        self.close_brace();
        SwitchBody {
            alts,
            els,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `expression_statement -> OclExpressionCS ';' | expression_block semicolonOpt`
    fn expression_statement(&mut self) -> Expr {
        if self.at(K::Lbrace) {
            let s = self.span();
            let b = self.block();
            self.eat(K::Semicolon);
            return self.mk(s, ExprKind::Block(b));
        }
        let e = self.expr();
        self.expect(K::Semicolon);
        e
    }

    // ----------------------------------------------------------------------------------------
    // Blocks

    /// `expression_block ::= '{' expression_listOpt '}'`
    pub(super) fn block(&mut self) -> Block {
        let start = self.span();
        if self.expect(K::Lbrace).is_none() {
            return Block {
                stmts: Vec::new(),
                span: Span::at(self.prev_end()),
            };
        }
        let stmts = self.stmt_list(&[]);
        if self.at(K::Rbrace) {
            self.bump();
        } else if self.at(K::Eof) {
            let msg = "missing \"}\" to complete scope";
            self.error(Code::MissingClose, start, msg);
        } else {
            self.expect(K::Rbrace);
        }
        Block {
            stmts,
            span: start.to(Span::at(self.prev_end())),
        }
    }

    /// `expression_list -> expression_semi_list semicolonOpt` up to `}` (or one of `stop`).
    pub(super) fn stmt_list(&mut self, stop: &[K]) -> Vec<Expr> {
        let mut stmts = Vec::new();
        let mut last_pos = usize::MAX;
        loop {
            let k = self.peek();
            if k == K::Rbrace || k == K::Eof || stop.contains(&k) {
                break;
            }
            if self.pos == last_pos {
                // No progress since the last round (a stray closing bracket): drop the token.
                self.bump();
                continue;
            }
            last_pos = self.pos;
            if !self.at_expr_start() {
                // Not a statement start: LPG deletes the token.
                self.unexpected();
                self.bump();
                self.skip_to(STMT_SYNC);
                self.eat(K::Semicolon);
                continue;
            }
            stmts.push(self.stmt());
            match self.peek() {
                K::Semicolon => {
                    self.bump();
                }
                k if k == K::Rbrace || k == K::Eof || stop.contains(&k) => break,
                _ => {
                    if self.at_expr_start() {
                        // A missing separator: LPG inserts `;` after the previous token.
                        self.expected(";", true);
                    } else {
                        self.unexpected();
                        self.bump();
                        self.skip_to(STMT_SYNC);
                        self.eat(K::Semicolon);
                    }
                }
            }
        }
        stmts
    }
}

fn cp_span(p: &Parser<'_>, cp: super::Checkpoint) -> Span {
    let start = p.toks[cp.pos].span;
    start.to(Span::at(p.prev_end()))
}

/// Whether an expression is a `primaryExpCS` (allowed on the left of `:=`).
fn is_primary(e: &Expr) -> bool {
    !matches!(
        e.kind,
        ExprKind::Binary(..)
            | ExprKind::Unary(..)
            | ExprKind::Let(..)
            | ExprKind::Assign(_)
            | ExprKind::Return(_)
            | ExprKind::Var(_)
            | ExprKind::Log(_)
            | ExprKind::Assert(_)
            | ExprKind::Error
    )
}

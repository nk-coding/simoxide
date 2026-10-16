//! Iterators and ImperativeOCL: OCL iterators (`iteratorExpCS`, `iterateExpCS`), imperative
//! iterators and their shorthands, `forEach`, blocks, variables, assignment, loops, `switch`,
//! `log`/`assert`, exceptions, `new`, `object`, mapping calls and the resolve family
//! (`QvtOperationalVisitorCS`).

use crate::analyzer::Sema;
use crate::env::VarRef;
use crate::expr::{Accessor, CallCs, Ex, Info, Pos, mk, path_span_of};
use crate::ir::{self, Direction, ExprKind, LoopKind};
use crate::messages::{bind, ocl as om, validation as vm};
use crate::module::{CtxType, scoped_parts};
use crate::ops::{OpRef, PropRef};
use qvto_syntax::ast::{self, NavOp};
use qvto_syntax::{Span, TokenKind};
use qvto_types::{CollKind, Type, TypeId, rel};

/// A late resolve expression, validated after the module's bodies
/// (`lateResolveNotUsedInDeferredAssignment`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LateResolve {
    pub span: Span,
    pub ok: bool,
}

impl<'r> Sema<'r> {
    // ---- OCL iterators -------------------------------------------------------------------------

    /// `iteratorExpCS` / `iterateExpCS` with QVTo's adjustments for `closure` and `List`.
    pub(crate) fn iterate_exp(&mut self, it: &ast::Iterate, cst: Span) -> Ex {
        let name = it.name.text.as_str();
        if name == "iterate" || it.acc.is_some() {
            return self.iterate_cs(it, cst);
        }
        let source = self.collection_source(&it.source);
        let st = source.ty();
        let elem = self.ts.types.element(st);
        let mut iters = Vec::new();
        let mut names = Vec::new();
        if !it.vars.is_empty() {
            for v in it.vars.iter().take(2) {
                let (l, _) = self.variable_declaration(v, ir::LocalKind::Iterator, true);
                if self.fun.locals[l.index()].ty == TypeId::NULL
                    && let Some(e) = elem
                {
                    self.fun.locals[l.index()].ty = e;
                }
                iters.push(l);
                names.push(v.name.text.clone());
            }
        } else {
            let (l, n) = self.new_temp_iterator(st);
            iters.push(l);
            names.push(n);
        }
        let body = if source.err {
            self.dummy(cst)
        } else {
            self.expr_d(&it.body)
        };
        let bt = body.ty();
        let src_kind = self.ts.types.coll_kind(st);
        let seq_like = matches!(
            self.ts.types.get(st),
            Type::Collection(CollKind::Sequence | CollKind::OrderedSet, _)
        );
        let elem_t = elem.unwrap_or(TypeId::OCL_VOID);
        let (kind, mut ty) = match name {
            "forAll" => (LoopKind::ForAll, TypeId::BOOLEAN),
            "exists" => (LoopKind::Exists, TypeId::BOOLEAN),
            "one" => (LoopKind::One, TypeId::BOOLEAN),
            "isUnique" => (LoopKind::IsUnique, TypeId::BOOLEAN),
            "select" => (LoopKind::Select, st),
            "reject" => (LoopKind::Reject, st),
            "collect" => {
                let e = if self.ts.types.is_collection_like(bt) {
                    self.flattened_element(bt)
                } else {
                    bt
                };
                let k = if seq_like {
                    CollKind::Sequence
                } else {
                    CollKind::Bag
                };
                (LoopKind::Collect, self.ts.types.collection(k, e))
            }
            "collectNested" => {
                let k = if seq_like {
                    CollKind::Sequence
                } else {
                    CollKind::Bag
                };
                (LoopKind::CollectNested, self.ts.types.collection(k, bt))
            }
            "any" => (LoopKind::Any, elem_t),
            "sortedBy" => {
                let k = if matches!(src_kind, Some(CollKind::Sequence | CollKind::Bag)) {
                    CollKind::Sequence
                } else {
                    CollKind::OrderedSet
                };
                (LoopKind::SortedBy, self.ts.types.collection(k, elem_t))
            }
            "closure" => {
                let ordered = matches!(
                    self.ts.types.get(bt),
                    Type::Collection(CollKind::Sequence | CollKind::OrderedSet, _)
                );
                let be = if self.ts.types.is_collection_like(bt) {
                    self.ts.types.element(bt).unwrap()
                } else {
                    bt
                };
                let k = if ordered {
                    CollKind::OrderedSet
                } else {
                    CollKind::Set
                };
                (LoopKind::Closure, self.ts.types.collection(k, be))
            }
            _ => (LoopKind::Select, TypeId::NULL),
        };
        // QVTo's iteratorExpCS
        if name == "closure" && seq_like {
            ty = self.ts.types.collection(CollKind::OrderedSet, elem_t);
        }
        if matches!(self.ts.types.get(st), Type::List(_)) && self.ts.types.is_collection_like(ty) {
            let e = self.ts.types.element(ty).unwrap();
            match name {
                "select" | "reject" | "collect" | "collectNested" | "sortedBy" => {
                    ty = self.ts.types.collection(CollKind::Sequence, e)
                }
                "closure" => ty = self.ts.types.collection(CollKind::OrderedSet, e),
                _ => {}
            }
        }
        for n in &names {
            self.delete_var(n);
        }
        let err = source.err;
        let lp = ir::Loop {
            kind,
            source: source.e,
            iterators: iters,
            acc: None,
            target: None,
            body: Some(body.e),
            cond: None,
            filter_type: None,
        };
        let mut x = self.ex(ExprKind::Loop(Box::new(lp)), ty, cst);
        x.err = err;
        x.info = Info::Iterator;
        x
    }

    /// `iterateExpCS`
    fn iterate_cs(&mut self, it: &ast::Iterate, cst: Span) -> Ex {
        let source = self.collection_source(&it.source);
        let st = source.ty();
        let Some(v1) = it.vars.first() else {
            return self.dummy(cst);
        };
        let (l1, init1) = self.variable_declaration(v1, ir::LocalKind::Iterator, true);
        let acc = it.acc.as_ref().map(|a| {
            (
                a,
                self.variable_declaration(a, ir::LocalKind::Accumulator, true),
            )
        });
        if self.fun.locals[l1.index()].ty == TypeId::NULL {
            self.fun.locals[l1.index()].ty = self.ts.types.element(st).unwrap_or(TypeId::OCL_VOID);
        }
        let body = if source.err {
            self.dummy(cst)
        } else {
            self.expr_d(&it.body)
        };
        let mut ty = TypeId::OCL_INVALID;
        let mut acc_ir = None;
        if let Some((a, (l2, init2))) = acc {
            let name = a.name.text.clone();
            if self.fun.locals[l2.index()].ty == TypeId::NULL {
                self.error(v1.span, bind(om::DeclarationType_ERROR_, &[&name]));
                self.fun.locals[l2.index()].ty = TypeId::OCL_INVALID;
            }
            ty = self.fun.locals[l2.index()].ty;
            if init2.is_none() {
                self.error(a.span, bind(om::DeclarationNoInitExp_ERROR_, &[&name]));
            }
            if init1.is_some() {
                self.error(v1.span, bind(om::DeclarationInitExp_ERROR_, &[&name]));
            }
            self.delete_var(&name);
            acc_ir = Some((
                l2,
                init2
                    .map(|x| x.e)
                    .unwrap_or_else(|| mk(ExprKind::Null, TypeId::OCL_VOID, a.span)),
            ));
        }
        self.delete_var(&v1.name.text);
        let err = source.err;
        let lp = ir::Loop {
            kind: LoopKind::Iterate,
            source: source.e,
            iterators: vec![l1],
            acc: acc_ir,
            target: None,
            body: Some(body.e),
            cond: None,
            filter_type: None,
        };
        let mut x = self.ex(ExprKind::Loop(Box::new(lp)), ty, cst);
        x.err = err;
        x.info = Info::Iterator;
        x
    }

    // ---- imperative iterators ------------------------------------------------------------------

    /// `src->xselect(...)` and friends.
    pub(crate) fn ximperative_iterate(&mut self, x: &ast::XIterate, cst: Span) -> Ex {
        let name = x.kind.spelling().unwrap_or("xcollect");
        let (body, cond) = if matches!(name, "xcollect" | "collectOne") {
            (Some(XBody::Expr(&x.body)), None)
        } else {
            (None, Some(&x.body))
        };
        let source = self.collection_source(&x.source);
        let vars: Vec<IterVar> = x.vars.iter().map(IterVar::Decl).collect();
        self.imperative_iterate(
            name,
            source,
            &vars,
            x.target.as_ref().map(XTarget::Decl),
            body,
            cond,
            cst,
        )
    }

    /// `src->name`: `xcollect` with an implicit iterator and the name as body.
    pub(crate) fn xcollect_shorthand(&mut self, n: &ast::Nav, cst: Span) -> Ex {
        let source = self.collection_source(&n.source);
        self.imperative_iterate(
            "xcollect",
            source,
            &[],
            None,
            Some(XBody::Name(&n.name)),
            None,
            cst,
        )
    }

    /// `src->object(v) T {...}`: `xcollect` with an object body.
    pub(crate) fn object_iter(&mut self, o: &ast::ObjectIter, cst: Span) -> Ex {
        let source = self.collection_source(&o.source);
        let vars = [IterVar::Decl(&o.var)];
        self.imperative_iterate(
            "xcollect",
            source,
            &vars,
            None,
            Some(XBody::Object(&o.decl, &o.body)),
            None,
            cst,
        )
    }

    /// `src->switch(v) {...}`: `xcollect` with a switch body.
    pub(crate) fn switch_iter(&mut self, s: &ast::SwitchIter, cst: Span) -> Ex {
        let source = self.collection_source(&s.source);
        let vars = [IterVar::Decl(&s.var)];
        self.imperative_iterate(
            "xcollect",
            source,
            &vars,
            None,
            Some(XBody::Switch(&s.body)),
            None,
            cst,
        )
    }

    /// `src[cond]`, `src![cond]`, `src[x | cond]`; on an `xcollect` source the collect becomes
    /// `collectselect(One)` with the `x` as target.
    pub(crate) fn xselect_shorthand(&mut self, x: &ast::XSelect, cst: Span) -> Ex {
        let mut src = &x.source;
        while let ast::ExprKind::Paren(inner) = &src.kind {
            src = inner;
        }
        let target = x.var.as_ref().map(XTarget::Name);
        let name = if x.one {
            "collectselectOne"
        } else {
            "collectselect"
        };
        match &src.kind {
            ast::ExprKind::XIterate(xi) if xi.kind.spelling() == Some("xcollect") => {
                let source = self.collection_source(&xi.source);
                let vars: Vec<IterVar> = xi.vars.iter().map(IterVar::Decl).collect();
                return self.imperative_iterate(
                    name,
                    source,
                    &vars,
                    target,
                    Some(XBody::Expr(&xi.body)),
                    Some(&x.cond),
                    cst,
                );
            }
            ast::ExprKind::Nav(n) if n.op == NavOp::Arrow => {
                let source = self.collection_source(&n.source);
                return self.imperative_iterate(
                    name,
                    source,
                    &[],
                    target,
                    Some(XBody::Name(&n.name)),
                    Some(&x.cond),
                    cst,
                );
            }
            ast::ExprKind::ObjectIter(o) => {
                let source = self.collection_source(&o.source);
                let vars = [IterVar::Decl(&o.var)];
                return self.imperative_iterate(
                    name,
                    source,
                    &vars,
                    target,
                    Some(XBody::Object(&o.decl, &o.body)),
                    Some(&x.cond),
                    cst,
                );
            }
            ast::ExprKind::SwitchIter(s) => {
                let source = self.collection_source(&s.source);
                let vars = [IterVar::Decl(&s.var)];
                return self.imperative_iterate(
                    name,
                    source,
                    &vars,
                    target,
                    Some(XBody::Switch(&s.body)),
                    Some(&x.cond),
                    cst,
                );
            }
            _ => {}
        }
        let name = if x.one { "selectOne" } else { "xselect" };
        let source = self.collection_source(&x.source);
        let vars: Vec<IterVar> = x.var.iter().map(IterVar::Name).collect();
        self.imperative_iterate(name, source, &vars, None, None, Some(&x.cond), cst)
    }

    /// `visitImperativeIterateExp`
    #[allow(clippy::too_many_arguments)]
    fn imperative_iterate(
        &mut self,
        name: &str,
        source: Ex,
        vars: &[IterVar<'_>],
        target: Option<XTarget<'_>>,
        body: Option<XBody<'_>>,
        cond: Option<&ast::Expr>,
        cst: Span,
    ) -> Ex {
        let st = source.ty();
        if !self.ts.types.is_collection_like(st) {
            return self.dummy(cst);
        }
        let elem = self.ts.types.element(st).unwrap();
        let mut iters = Vec::new();
        let mut names = Vec::new();
        if !vars.is_empty() {
            for v in vars.iter().take(2) {
                let l = self.iter_var(v);
                if self.fun.locals[l.index()].ty == TypeId::NULL {
                    self.fun.locals[l.index()].ty = elem;
                }
                iters.push(l);
                names.push(self.fun.locals[l.index()].name.clone());
            }
        } else {
            let (l, n) = self.new_temp_iterator(st);
            iters.push(l);
            names.push(n);
        }
        let vdcl = iters[0];
        let kind = match name {
            "xselect" => LoopKind::XSelect,
            "selectOne" => LoopKind::SelectOne,
            "collectOne" => LoopKind::CollectOne,
            "collectselect" => LoopKind::CollectSelect,
            "collectselectOne" => LoopKind::CollectSelectOne,
            _ => LoopKind::XCollect,
        };
        let mut result_elem: Option<TypeId> = None;
        let mut body_ir = None;
        let mut target_l: Option<ir::LocalId> = None;
        let mut target_name: Option<String> = None;
        if matches!(name, "xselect" | "selectOne") {
            result_elem = Some(elem);
        } else {
            let mut body_ty = None;
            if let Some(b) = body {
                let bx = self.xbody(b);
                let target_has_init = matches!(target, Some(XTarget::Decl(d)) if d.init.is_some());
                if !target_has_init && matches!(name, "collectselect" | "collectselectOne") {
                    if !innermost_iterator_related(vdcl, &bx.e) {
                        let tn = self.full_name(self.fun.locals[vdcl.index()].ty);
                        self.error(
                            b.span(),
                            bind(vm::QvtOperationalVisitorCS_FeatureNotFoundForType, &[&tn]),
                        );
                    }
                    if target.is_none() {
                        let (l, n) = self.new_temp_var(bx.ty(), ir::LocalKind::Target);
                        target_l = Some(l);
                        target_name = Some(n);
                    }
                }
                body_ty = Some(bx.ty());
                body_ir = Some(bx.e);
            }
            if let Some(t) = target {
                let (l, init) = match t {
                    XTarget::Decl(d) => self.variable_declaration(d, ir::LocalKind::Target, true),
                    XTarget::Name(n) => {
                        let l = self.fun.new_local(
                            &n.text,
                            TypeId::NULL,
                            ir::LocalKind::Target,
                            n.span,
                        );
                        self.add_var_or_error(&n.text, l, n.span);
                        (l, None)
                    }
                };
                if let Some(i) = init {
                    body_ty = Some(i.ty());
                    body_ir = Some(i.e);
                }
                target_l = Some(l);
                target_name = Some(self.fun.locals[l.index()].name.clone());
            }
            if let Some(bt) = body_ty {
                result_elem = Some(bt);
                if let Some(l) = target_l
                    && self.fun.locals[l.index()].ty == TypeId::NULL
                {
                    self.fun.locals[l.index()].ty = bt;
                }
            }
            if result_elem.is_none() {
                result_elem = Some(elem);
                if let Some(l) = target_l
                    && self.fun.locals[l.index()].ty == TypeId::NULL
                {
                    self.fun.locals[l.index()].ty = elem;
                }
            }
        }
        let mut cond_ir = None;
        let mut filter_type = None;
        if let Some(c) = cond {
            let cx = self.expr_d(c);
            if let Info::TypeExp(t) = cx.info {
                result_elem = Some(t);
                filter_type = Some(t);
            } else if cx.ty() != TypeId::BOOLEAN {
                self.error(
                    c.span,
                    vm::QvtOperationalVisitorCS_WrongImperativeIteratorConditionType,
                );
            }
            if filter_type.is_none() {
                cond_ir = Some(cx.e);
            }
        }
        if let Some(n) = &target_name {
            self.delete_var(n);
        }
        let re = result_elem.unwrap_or(elem);
        let ty = match name {
            "selectOne" | "collectOne" | "collectselectOne" => re,
            "xselect" => {
                let k = match self.ts.types.get(st) {
                    Type::Collection(k, _) => *k,
                    _ => CollKind::Sequence,
                };
                self.ts.types.collection(k, re)
            }
            _ => {
                let seq = matches!(
                    self.ts.types.get(st),
                    Type::Collection(CollKind::Sequence | CollKind::OrderedSet, _) | Type::List(_)
                );
                self.ts.types.collection(
                    if seq {
                        CollKind::Sequence
                    } else {
                        CollKind::Bag
                    },
                    re,
                )
            }
        };
        for n in &names {
            self.delete_var(n);
        }
        let err = source.err;
        let lp = ir::Loop {
            kind,
            source: source.e,
            iterators: iters,
            acc: None,
            target: target_l,
            body: body_ir,
            cond: cond_ir,
            filter_type,
        };
        let _ = err;
        let mut x = self.ex(ExprKind::Loop(Box::new(lp)), ty, cst);
        x.info = Info::ImperativeIterate;
        x
    }

    fn iter_var(&mut self, v: &IterVar<'_>) -> ir::LocalId {
        match v {
            IterVar::Decl(d) => {
                self.variable_declaration(d, ir::LocalKind::Iterator, true)
                    .0
            }
            IterVar::Name(n) => {
                let l = self
                    .fun
                    .new_local(&n.text, TypeId::NULL, ir::LocalKind::Iterator, n.span);
                self.add_var_or_error(&n.text, l, n.span);
                l
            }
        }
    }

    fn xbody(&mut self, b: XBody<'_>) -> Ex {
        match b {
            XBody::Expr(e) => self.expr_d(e),
            XBody::Name(n) => {
                let e = ast::Expr {
                    kind: ast::ExprKind::Name(n.clone()),
                    span: n.span,
                };
                self.expr_d(&e)
            }
            XBody::Object(decl, body) => {
                let span = decl.span.to(body.span);
                self.object_core(
                    decl.var.as_ref(),
                    decl.ty.as_ref(),
                    &body.stmts,
                    span,
                    true,
                    None,
                )
            }
            XBody::Switch(s) => {
                let alts: Vec<(&ast::Expr, &ast::Expr)> =
                    s.alts.iter().map(|a| (&a.cond, &a.body)).collect();
                self.switch_common(&alts, s.els.as_ref().map(|e| &e.body), s.span, Pos::Other)
            }
        }
    }

    /// `visitForExp`: `forEach`/`forOne`.
    pub(crate) fn for_exp(&mut self, f: &ast::ForEach, cst: Span) -> Ex {
        let source = self.collection_source(&f.source);
        let st = source.ty();
        if !self.ts.types.is_collection_like(st) {
            return self.dummy(cst);
        }
        let elem = self.ts.types.element(st).unwrap();
        let mut iters = Vec::new();
        for n in f.vars.iter().take(2) {
            let l = self
                .fun
                .new_local(&n.text, elem, ir::LocalKind::Iterator, n.span);
            self.add_var_or_error(&n.text, l, n.span);
            iters.push(l);
        }
        let cond = f.cond.as_ref().map(|c| self.expr_d(c));
        let body = self.block_exp(&f.body, f.body.span);
        for n in f.vars.iter().take(2) {
            self.delete_var(&n.text);
        }
        let kind = if f.kind.spelling() == Some("forOne") {
            LoopKind::ForOne
        } else {
            LoopKind::ForEach
        };
        let lp = ir::Loop {
            kind,
            source: source.e,
            iterators: iters,
            acc: None,
            target: None,
            body: Some(body.e),
            cond: cond.map(|c| c.e),
            filter_type: None,
        };
        self.ex(ExprKind::Loop(Box::new(lp)), TypeId::OCL_VOID, cst)
    }

    // ---- blocks and variables ------------------------------------------------------------------

    /// The names declared in the current environment level.
    fn level_names(&self) -> Vec<String> {
        self.fun
            .scopes
            .last()
            .map(|s| {
                s.vars
                    .iter()
                    .filter(|v| v.named)
                    .map(|v| v.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// `visitBlockExpCS`: variables declared inside are removed at the end.
    pub(crate) fn block_exp(&mut self, b: &ast::Block, cst: Span) -> Ex {
        let before = self.level_names();
        let stmts = self.statements(&b.stmts, false);
        let after = self.level_names();
        for n in after {
            if !before.contains(&n) {
                self.delete_var(&n);
            }
        }
        let det = stmts.len() == 1 && stmts[0].det;
        let mut x = self.ex(
            ExprKind::Block(stmts.into_iter().map(|s| s.e).collect()),
            TypeId::OCL_VOID,
            cst,
        );
        x.det = det;
        x.info = Info::Block;
        x
    }

    /// Statements of a block-like container (`visitOclExpressionCS` each, nulls dropped).
    /// `query`: the container is a query body (its last expression may be an equality).
    pub(crate) fn statements(&mut self, stmts: &[ast::Expr], query: bool) -> Vec<Ex> {
        let mut out = Vec::new();
        let n = stmts.len();
        for (i, s) in stmts.iter().enumerate() {
            let pos = if query && i + 1 == n {
                Pos::QueryLast
            } else {
                Pos::Block
            };
            if let ast::ExprKind::Var(v) = &s.kind {
                // `var a := 1, b := 2` are separate statements
                if v.decls.len() > 1 {
                    for d in &v.decls {
                        out.push(self.var_init(d));
                    }
                    continue;
                }
            }
            if let Some(x) = self.visit_ocl_expression_at(s, pos) {
                out.push(x);
            }
        }
        out
    }

    /// `var x ...` (one or more declarations).
    pub(crate) fn var_statement(&mut self, v: &ast::VarExp, cst: Span) -> Ex {
        if v.decls.len() == 1 {
            return self.var_init(&v.decls[0]);
        }
        let mut out = Vec::new();
        for d in &v.decls {
            out.push(self.var_init(d).e);
        }
        self.ex(ExprKind::Block(out), TypeId::OCL_VOID, cst)
    }

    /// `variableInitializationCS`
    fn var_init(&mut self, d: &ast::VarDecl) -> Ex {
        let name = d.name.text.clone();
        let declared = d.ty.as_ref().and_then(|t| self.visit_type_cs(t));
        let mut derived = None;
        let mut init = None;
        let mut with_result = false;
        if let Some((op, e)) = &d.init {
            with_result = *op == TokenKind::Coloncolonequal;
            if let Some(x) = self.visit_ocl_expression(e) {
                derived = Some(x.ty());
                if matches!(self.ts.types.get(x.ty()), Type::TypeType(_)) {
                    self.error(e.span, vm::VariableTypeTypeError);
                }
                init = Some(x);
            }
        }
        let ty = declared.or(derived).unwrap_or(TypeId::NULL);
        if let (Some(dt), Some(it)) = (declared, derived)
            && !self.ts.ck().assignable(dt, it)
        {
            let a = self.full_name(dt);
            let b = self.full_name(it);
            let span = init.as_ref().map(|x| x.e.span).unwrap_or(d.span);
            self.error(span, bind(vm::TypeMismatchError, &[&a, &b]));
        }
        let l = self.fun.new_local(&name, ty, ir::LocalKind::Var, d.span);
        if self.lookup_local(&name).is_some() {
            self.error(d.span, bind(vm::NameAlreadyDefinedError, &[&name]));
        } else {
            self.add_var(&name, VarRef::Local(l), true);
        }
        let kind = ExprKind::VarInit {
            local: l,
            init: init.map(|x| Box::new(x.e)),
            with_result,
        };
        self.ex(kind, ty, d.span)
    }

    /// `visitTypeCS`: a type, `None` (with an error) when unknown.
    pub(crate) fn visit_type_cs(&mut self, t: &ast::TypeExpr) -> Option<TypeId> {
        self.type_cs(t)
    }

    /// `visitComputeExpCS`
    pub(crate) fn compute_exp(&mut self, c: &ast::Compute, cst: Span) -> Ex {
        let (l, init) = self.variable_declaration(&c.decl, ir::LocalKind::Var, true);
        let ty = self.fun.locals[l.index()].ty;
        let body_expr = ast::Expr {
            kind: ast::ExprKind::Block(c.body.clone()),
            span: c.body.span,
        };
        let body = self.visit_ocl_expression(&body_expr);
        self.delete_var(&c.decl.name.text);
        let det = body.as_ref().is_some_and(|b| b.det);
        let body_e = body
            .map(|b| b.e)
            .unwrap_or_else(|| mk(ExprKind::Null, TypeId::OCL_VOID, c.body.span));
        let mut x = self.ex(
            ExprKind::Compute(l, init.map(|i| Box::new(i.e)), Box::new(body_e)),
            ty,
            cst,
        );
        x.det = det;
        x.info = Info::Compute;
        x
    }

    /// `visitWhileExpCS`
    pub(crate) fn while_exp(&mut self, w: &ast::While, cst: Span) -> Ex {
        let mut result_var: Option<(ir::LocalId, Option<Ex>, String)> = None;
        match &w.init {
            Some(ast::WhileInit::Decl(d)) => {
                let (l, init) = self.variable_declaration(d, ir::LocalKind::Var, true);
                result_var = Some((l, init, d.name.text.clone()));
            }
            Some(ast::WhileInit::Assign(e)) => {
                if let ast::ExprKind::Assign(a) = &e.kind {
                    let name = match &a.lhs.kind {
                        ast::ExprKind::Name(n) if n.kind == ast::NameKind::Ident => n.text.clone(),
                        _ => "error_var".to_string(),
                    };
                    let d = ast::VarDecl {
                        name: ast::Name {
                            text: name.clone(),
                            kind: ast::NameKind::Ident,
                            span: a.lhs.span,
                        },
                        ty: None,
                        init: Some((TokenKind::ResetAssign, Box::new(a.rhs.clone()))),
                        span: a.lhs.span,
                    };
                    let (l, init) = self.variable_declaration(&d, ir::LocalKind::Var, true);
                    result_var = Some((l, init, name));
                }
            }
            None => {}
        }
        let in_env = result_var
            .as_ref()
            .filter(|(l, _, n)| self.lookup_var(n) == Some(VarRef::Local(*l)))
            .map(|(_, _, n)| n.clone());
        let cond = self.visit_ocl_expression(&w.cond);
        if let Some(c) = &cond
            && c.ty() != TypeId::BOOLEAN
        {
            let ct = if c.ty() == TypeId::NULL {
                TypeId::OCL_VOID
            } else {
                c.ty()
            };
            let n = self.type_name(ct);
            self.error(
                w.cond.span,
                bind(
                    vm::QvtOperationalVisitorCS_booleanTypeExpressionExpected,
                    &[&n],
                ),
            );
        }
        let body = self.block_exp(&w.body, w.body.span);
        if let Some(n) = in_env {
            self.delete_var(&n);
        }
        let cond_e = cond
            .map(|c| c.e)
            .unwrap_or_else(|| mk(ExprKind::Bool(false), TypeId::BOOLEAN, w.cond.span));
        let wh = mk(
            ExprKind::While(Box::new(cond_e), Box::new(body.e)),
            TypeId::OCL_VOID,
            cst,
        );
        match result_var {
            Some((l, init, _)) => {
                let ty = self.fun.locals[l.index()].ty;
                let mut x = self.ex(
                    ExprKind::Compute(l, init.map(|i| Box::new(i.e)), Box::new(wh)),
                    ty,
                    cst,
                );
                x.info = Info::Compute;
                x
            }
            None => Ex {
                e: wh,
                err: false,
                info: Info::Other,
                det: false,
            },
        }
    }

    /// `switch { ... }` (statement form).
    pub(crate) fn switch_exp(&mut self, s: &ast::SwitchBody, cst: Span, pos: Pos) -> Ex {
        let alts: Vec<(&ast::Expr, &ast::Expr)> =
            s.alts.iter().map(|a| (&a.cond, &a.body)).collect();
        self.switch_common(&alts, s.els.as_ref().map(|e| &e.body), cst, pos)
    }

    /// `visitSwitchExpCS`
    pub(crate) fn switch_common(
        &mut self,
        alts: &[(&ast::Expr, &ast::Expr)],
        els: Option<&ast::Expr>,
        cst: Span,
        pos: Pos,
    ) -> Ex {
        self.switch_common_spans(alts, els.map(|e| (e, e.span)), cst, pos)
    }

    pub(crate) fn switch_common_spans(
        &mut self,
        alts: &[(&ast::Expr, &ast::Expr)],
        els: Option<(&ast::Expr, Span)>,
        cst: Span,
        pos: Pos,
    ) -> Ex {
        let inner = if pos == Pos::Block {
            Pos::Branch
        } else {
            Pos::Other
        };
        let mut types: Vec<Option<TypeId>> = Vec::new();
        let mut out = Vec::new();
        let mut det = true;
        for (c, b) in alts {
            let cx = self.visit_ocl_expression(c);
            let bx = self.visit_ocl_expression_at(b, inner);
            types.push(bx.as_ref().map(|x| x.ty()));
            det &= bx.as_ref().is_some_and(|x| x.det);
            let ce = cx
                .map(|x| x.e)
                .unwrap_or_else(|| mk(ExprKind::Bool(false), TypeId::BOOLEAN, c.span));
            let be = bx
                .map(|x| x.e)
                .unwrap_or_else(|| mk(ExprKind::Null, TypeId::OCL_VOID, b.span));
            out.push((ce, be));
        }
        let mut else_e = None;
        match els {
            Some((e, esp)) => match self.visit_ocl_expression_cst(e, inner, esp) {
                Some(x) => {
                    types.push(Some(x.ty()));
                    det &= x.det;
                    else_e = Some(Box::new(x.e));
                }
                None => det = false,
            },
            None => det = false,
        }
        let ty = match types.len() {
            0 => TypeId::OCL_VOID,
            1 => types[0].unwrap_or(TypeId::NULL),
            _ => {
                let mut t = None;
                for i in 0..types.len() - 1 {
                    t = match (types[i], types[i + 1]) {
                        (Some(a), Some(b)) => self.common_super_type_at(cst, a, b),
                        (Some(a), None) | (None, Some(a)) => Some(a),
                        (None, None) => None,
                    };
                    if t.is_none() {
                        break;
                    }
                }
                t.unwrap_or(TypeId::OCL_VOID)
            }
        };
        let mut x = self.ex(ExprKind::Switch(out, else_e), ty, cst);
        x.det = det;
        x.info = Info::Switch;
        x
    }

    /// `visitReturnExpCS`
    pub(crate) fn return_exp(&mut self, v: Option<&ast::Expr>, cst: Span) -> Ex {
        let value = v.and_then(|e| self.expr(e));
        let ty = value.as_ref().map(|x| x.ty()).unwrap_or(TypeId::OCL_VOID);
        let mut x = self.ex(ExprKind::Return(value.map(|x| Box::new(x.e))), ty, cst);
        x.det = true;
        x.info = Info::Return;
        x
    }

    // ---- assignment ----------------------------------------------------------------------------

    /// `visitAssignStatementCS`
    pub(crate) fn assign_statement(&mut self, a: &ast::Assign, cst: Span) -> Option<Ex> {
        let mut lhs = &a.lhs;
        while let ast::ExprKind::Paren(inner) = &lhs.kind {
            lhs = inner;
        }
        let lvalue_cs = matches!(
            &lhs.kind,
            ast::ExprKind::Name(_)
                | ast::ExprKind::SelfExp
                | ast::ExprKind::Path(_)
                | ast::ExprKind::Nav(_)
        ) && !matches!(&lhs.kind, ast::ExprKind::Nav(n) if n.op == NavOp::Arrow);
        if !lvalue_cs {
            self.error(a.lhs.span, vm::notAnLValueError);
            return None;
        }
        let rhs = self.visit_ocl_expression(&a.rhs)?;
        let lv = self.expr_at(&a.lhs, Pos::Other, a.lhs.span)?;
        let lty = lv.ty();
        let is_mutation = self.is_mutable_collection(lty) && a.add;

        let target = match lv.info {
            Info::Var(Some(v)) => {
                let name = self.var_name(v);
                if let Some(src) = self.implicit_source_for_property(&name) {
                    let t = self.var_type(src);
                    let tn = self.classifier_name(t);
                    self.warning(a.lhs.span, bind(vm::HidingVariable, &[&name, &tn]));
                }
                if name == "result" && !self.in_init_section() {
                    if a.add {
                        let vt = self.var_type(v);
                        if !self.is_mutable_collection(vt) {
                            self.error(
                                a.lhs.span,
                                vm::QvtOperationalVisitorCS_incrementalAssignmentResultNotAllowed,
                            );
                            return None;
                        }
                    } else {
                        self.error(
                            a.lhs.span,
                            vm::QvtOperationalVisitorCS_assignmentResultNotAllowed,
                        );
                        return None;
                    }
                }
                if name == "self" {
                    self.error(a.lhs.span, vm::CantAssignToSelf);
                    return None;
                }
                if name == "this" {
                    self.error(a.lhs.span, vm::CantAssignToThis);
                    return None;
                }
                self.validate_variable_modification(v, a.lhs.span, None, !is_mutation);
                let vt = self.var_type(v);
                self.validate_assignment(false, &name, vt, rhs.ty(), a.add, a.lhs.span, rhs.e.span);
                match v {
                    VarRef::Local(l) => ir::LValue::Local(l),
                    _ => return None,
                }
            }
            Info::Prop(p, src_var, src_ty) => {
                let pname = self.prop_name(p);
                if !self.prop_changeable(p) {
                    self.error(a.lhs.span, bind(vm::ReadOnlyProperty, &[&pname]));
                } else {
                    if matches!(self.ts.types.get(src_ty), Type::Tuple(_)) && !is_mutation {
                        let sn = match src_var {
                            Some(v) => self.var_name(v),
                            None => "null".to_string(),
                        };
                        self.error(a.lhs.span, bind(vm::tupleMutationError, &[&pname, &sn]));
                        return None;
                    }
                    if let Some(v) = src_var {
                        self.validate_variable_modification(v, a.lhs.span, Some(p), false);
                    }
                    let pt = self.prop_type(p);
                    self.validate_assignment(
                        true,
                        &pname,
                        pt,
                        rhs.ty(),
                        a.add,
                        a.lhs.span,
                        rhs.e.span,
                    );
                }
                match lv.e.kind {
                    ExprKind::Prop(src, f) => ir::LValue::Prop(*src, f),
                    ExprKind::Global(g) => ir::LValue::Global(g),
                    ExprKind::IntermProp(src, i) => ir::LValue::IntermProp(*src, i),
                    ExprKind::TupleProp(src, n) => ir::LValue::TupleProp(*src, n),
                    _ => return None,
                }
            }
            Info::Invalid => {
                // Eclipse still makes the AssignExp (with an invalid left side); keep the value
                let mut x = self.ex(ExprKind::Block(vec![rhs.e]), lty, cst);
                x.err = true;
                return Some(x);
            }
            _ => {
                self.error(a.lhs.span, vm::notAnLValueError);
                return None;
            }
        };
        let is_prop_target = matches!(
            target,
            ir::LValue::Prop(..)
                | ir::LValue::Global(_)
                | ir::LValue::IntermProp(..)
                | ir::LValue::TupleProp(..)
        );
        self.mark_late_resolve_assigned(&rhs.e, is_prop_target);
        let asg = ir::Assign {
            target,
            target_ty: lty,
            value: rhs.e,
            add: a.add,
            target_span: a.lhs.span,
        };
        Some(self.ex(ExprKind::Assign(Box::new(asg)), lty, cst))
    }

    fn is_mutable_collection(&self, t: TypeId) -> bool {
        matches!(self.ts.types.get(t), Type::List(_) | Type::Dict(..))
    }

    /// `EClassifier.getName()`
    pub(crate) fn classifier_name(&mut self, t: TypeId) -> String {
        if t == TypeId::NULL {
            return String::new();
        }
        self.type_name(t)
    }

    /// Whether code is analysed directly in a mapping's `init` section (Eclipse's flag is per
    /// environment level).
    pub(crate) fn in_init_section(&self) -> bool {
        self.fun.scopes.last().is_some_and(|s| s.in_init)
    }

    /// `QvtOperationalParserUtil.validateAssignment`
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn validate_assignment(
        &mut self,
        is_property: bool,
        left: &str,
        lt: TypeId,
        rt: TypeId,
        add: bool,
        lspan: Span,
        rspan: Span,
    ) -> bool {
        let lcoll = self.ts.types.is_collection_like(lt);
        if add || (is_property && lcoll) {
            if add && !lcoll {
                self.error(lspan, bind(vm::SemanticUtil_3, &[left]));
                return false;
            }
            let base = self.ts.types.element(lt).unwrap();
            let actual = if self.ts.types.is_collection_like(rt) {
                self.ts.types.element(rt).unwrap()
            } else {
                rt
            };
            if !self.ts.ck().assignable(base, actual) {
                let a = self.full_name(base);
                let b = self.full_name(actual);
                self.error(rspan, bind(vm::SemanticUtil_5, &[left, &a, &b]));
                return false;
            }
        } else if !self.ts.ck().assignable(lt, rt) {
            let a = self.full_name(lt);
            let b = self.full_name(rt);
            self.error(rspan, bind(vm::SemanticUtil_8, &[left, &a, &b]));
            return false;
        }
        true
    }

    /// `QvtOperationalParserUtil.validateVariableModification`
    pub(crate) fn validate_variable_modification(
        &mut self,
        v: VarRef,
        span: Span,
        prop: Option<PropRef>,
        direct: bool,
    ) -> bool {
        let (dir, name) = match v {
            VarRef::Local(l) => match self.fun.extra[l.index()].param {
                Some(p) => (p.direction, self.fun.locals[l.index()].name.clone()),
                None => return true,
            },
            VarRef::ModelParam(p) => {
                let mp = &self.modules[p.module.index()].model_params[p.index as usize];
                (mp.direction, mp.name.clone())
            }
            VarRef::This(_) => return true,
        };
        let in_mapping = self
            .fun
            .op
            .is_some_and(|o| self.ops[o.index()].ir.kind == ir::OpKind::Mapping);
        if dir == Direction::InOut && direct && in_mapping {
            self.error(
                span,
                bind(
                    vm::QvtOperationalParserUtil_inoutParamAssignmentError,
                    &[&name],
                ),
            );
            return false;
        }
        if dir == Direction::Out && direct && in_mapping && !self.in_init_section() {
            self.error(
                span,
                bind(
                    vm::QvtOperationalParserUtil_outParamAssignmentError,
                    &[&name],
                ),
            );
            return false;
        }
        let contextual = matches!(prop, Some(PropRef::Interm(_)));
        if dir == Direction::In && !contextual {
            self.error(span, bind(vm::inputParameterModificationError, &[&name]));
            return false;
        }
        true
    }

    // ---- log, assert, exceptions ---------------------------------------------------------------

    /// `logExp`
    pub(crate) fn log_exp(&mut self, l: &ast::Log, cst: Span) -> Ex {
        let args: Vec<Ex> = l.args.iter().filter_map(|a| self.expr(a)).collect();
        let when = l.when.as_ref().and_then(|c| self.expr(c));
        self.validate_log(&args, when.as_ref(), cst);
        let mut it = args.into_iter().map(|x| x.e);
        let log = ir::Log {
            message: it.next(),
            element: it.next(),
            level: it.next(),
            when: when.map(|w| w.e),
        };
        self.ex(ExprKind::Log(Box::new(log)), TypeId::OCL_VOID, cst)
    }

    /// `validateLogExp`
    fn validate_log(&mut self, args: &[Ex], when: Option<&Ex>, cst: Span) {
        if args.is_empty() {
            self.error(cst, vm::MissingMessageLogExpArgumentError);
        }
        for (i, a) in args.iter().enumerate() {
            match i + 1 {
                1 => {
                    if a.ty() != TypeId::STRING {
                        self.error(a.e.span, vm::StringTypeMessageLogArgumentError);
                    }
                }
                2 => {}
                3 => {
                    if a.ty() != TypeId::INTEGER {
                        self.error(a.e.span, vm::LogLevelNumberArgumentError);
                    }
                }
                p => self.error(
                    a.e.span,
                    bind(vm::UnsupportedLogExpArgumentError, &[&p.to_string()]),
                ),
            }
        }
        if let Some(c) = when
            && c.ty() != TypeId::BOOLEAN
        {
            self.error(c.e.span, vm::LogExpBooleanTypeConditionError);
        }
    }

    /// `assertExp`
    pub(crate) fn assert_exp(&mut self, a: &ast::Assert, cst: Span) -> Ex {
        let cond = self.expr(&a.cond);
        let mut severity = ir::AssertSeverity::Error;
        if let Some(s) = &a.severity {
            match s.text.as_str() {
                "error" => severity = ir::AssertSeverity::Error,
                "warning" => severity = ir::AssertSeverity::Warning,
                "fatal" => severity = ir::AssertSeverity::Fatal,
                other => self.error(s.span, bind(vm::UknownSeverityKindError, &[other])),
            }
        }
        let mut log = None;
        if let Some(l) = &a.log
            && let ast::ExprKind::Log(lg) = &l.kind
        {
            let x = self.log_exp(lg, l.span);
            if let ExprKind::Log(b) = x.e.kind {
                log = Some(*b);
            }
            // validateAssertExp validates the log again (same problems, deduplicated)
        }
        match &cond {
            Some(c) if c.ty() == TypeId::BOOLEAN => {}
            Some(c) => self.error(c.e.span, vm::BooleanTypeAssertConditionError),
            None => self.error(cst, vm::BooleanTypeAssertConditionError),
        }
        let cond_e = cond
            .map(|c| c.e)
            .unwrap_or_else(|| mk(ExprKind::Bool(true), TypeId::BOOLEAN, a.cond.span));
        let asr = ir::Assert {
            severity,
            cond: cond_e,
            log,
        };
        self.ex(ExprKind::Assert(Box::new(asr)), TypeId::OCL_VOID, cst)
    }

    /// The exception class `Exception` of the library.
    fn exception_type(&mut self) -> TypeId {
        let c = self.ts.std.exception;
        self.ts.types.class(c)
    }

    /// `EmfUtil.isAssignableFrom(Exception, t)`
    fn is_exception(&mut self, t: Option<TypeId>) -> bool {
        let Some(t) = t else {
            return false;
        };
        let ex = self.exception_type();
        self.is_assignable_from(ex, t)
    }

    /// `EmfUtil.isAssignableFrom(a, b)`: `b` is `a` or a subclass of it.
    fn is_assignable_from(&mut self, a: TypeId, b: TypeId) -> bool {
        match (self.ts.types.get(a).clone(), self.ts.types.get(b).clone()) {
            (Type::Class(ca), Type::Class(cb)) => ca == cb || self.ts.meta.is_subclass(cb, ca),
            _ => a == b,
        }
    }

    /// `visitRaiseExpCS`
    pub(crate) fn raise_exp(&mut self, r: &ast::Raise, cst: Span) -> Ex {
        let te = match &r.exception.kind {
            ast::ExprKind::Name(n) => Some(ast::TypeExpr {
                kind: ast::TypeKind::Path(ast::PathName {
                    segments: vec![n.clone()],
                    span: n.span,
                }),
                span: n.span,
            }),
            ast::ExprKind::Path(p) => Some(ast::TypeExpr {
                kind: ast::TypeKind::Path(p.clone()),
                span: p.span,
            }),
            ast::ExprKind::TypeLit(t) => Some(t.clone()),
            _ => None,
        };
        let Some(te) = te else {
            let mut x = self.ex(ExprKind::Raise(None, None), TypeId::OCL_VOID, cst);
            x.det = true;
            x.info = Info::Return;
            return x;
        };
        let t = self.visit_type_cs(&te);
        if !self.is_exception(t) {
            let tn = t.map(|t| self.qualified_name(t)).unwrap_or_default();
            self.error(te.span, bind(vm::CannotRaiseNonException, &[&tn]));
        }
        let mut arg = None;
        if let Some(Some(e)) = &r.arg {
            let x = self.expr_d(e);
            if x.ty() != TypeId::STRING {
                self.error(e.span, vm::StringTypeMessageExcArgumentError);
            }
            arg = Some(Box::new(x.e));
        }
        let class = t.and_then(|t| match self.ts.types.get(t) {
            Type::Class(c) => Some(*c),
            _ => None,
        });
        let mut x = self.ex(ExprKind::Raise(class, arg), TypeId::OCL_VOID, cst);
        x.det = true;
        x.info = Info::Return;
        x
    }

    /// `visitTryExpCS`
    pub(crate) fn try_exp(&mut self, t: &ast::Try, cst: Span) -> Ex {
        let body = self.block_exp(&t.body, t.body.span);
        let mut handled: Vec<TypeId> = Vec::new();
        let mut catches = Vec::new();
        let root = self.exception_type();
        for c in &t.catches {
            let mut var_ty: Option<TypeId> = None;
            let mut classes = Vec::new();
            for te in &c.types {
                let et = self.visit_type_cs(te);
                if !self.is_exception(et) {
                    let tn = et.map(|t| self.qualified_name(t)).unwrap_or_default();
                    self.error(te.span, bind(vm::CannotCatchNonException, &[&tn]));
                }
                for h in handled.clone() {
                    if let Some(e) = et
                        && self.is_assignable_from(h, e)
                    {
                        let en = self.classifier_name(e);
                        let hn = self.classifier_name(h);
                        self.error(te.span, bind(vm::UncatchableCatchBlock, &[&en, &hn]));
                    }
                }
                if let Some(e) = et {
                    handled.push(e);
                    if let Type::Class(cl) = self.ts.types.get(e) {
                        classes.push(*cl);
                    }
                }
                var_ty = Some(if var_ty.is_none() {
                    et.unwrap_or(TypeId::NULL)
                } else {
                    root
                });
            }
            if c.types.is_empty() {
                handled.push(root);
            }
            let mut var = None;
            if let Some(n) = &c.var {
                let l = self.fun.new_local(
                    &n.text,
                    var_ty.unwrap_or(TypeId::NULL),
                    ir::LocalKind::Catch,
                    n.span,
                );
                self.add_var_or_error(&n.text, l, n.span);
                var = Some(l);
            }
            let b = self.block_exp(&c.body, c.body.span);
            if let Some(n) = &c.var {
                self.delete_var(&n.text);
            }
            catches.push(ir::Catch {
                types: classes,
                var,
                body: b.e,
                span: c.span,
            });
        }
        let tr = ir::Try {
            body: body.e,
            catches,
        };
        self.ex(ExprKind::Try(Box::new(tr)), TypeId::OCL_VOID, cst)
    }

    // ---- new, object ---------------------------------------------------------------------------

    /// `instantiationExpCS`
    pub(crate) fn new_exp(&mut self, n: &ast::New, cst: Span) -> Ex {
        let (ty, extent) = self.visit_type_spec(&n.ty, Direction::Out);
        let args: Vec<Ex> = n.args.iter().filter_map(|a| self.expr(a)).collect();
        let Some(t) = ty else {
            let nw = ir::New {
                ty: TypeId::NULL,
                extent,
                constructor: None,
                args: args.into_iter().map(|x| x.e).collect(),
            };
            return self.ex(ExprKind::New(Box::new(nw)), TypeId::NULL, cst);
        };
        if let Type::Module(m) = *self.ts.types.get(t) {
            let mid = ir::ModuleId(m);
            if !self.modules[mid.index()].is_library {
                return self.ex(
                    ExprKind::NewTransformation(mid, args.into_iter().map(|x| x.e).collect()),
                    t,
                    cst,
                );
            }
        }
        let a: Vec<crate::ops::Arg> = args.iter().map(|x| self.arg_of(x)).collect();
        let cname = self.classifier_name(t);
        let mut constructor = None;
        let cands = self.lookup_constructor_operations(t, &cname, &a);
        if !cands.is_empty() {
            let refs: Vec<OpRef> = cands.iter().map(|o| OpRef::User(*o)).collect();
            constructor = match self.most_specific_operation(&refs, &a) {
                Ok(o) => Some(o),
                Err(matches) => self.handle_ambiguous_ops_pub(cst, matches),
            };
        }
        let instantiable = matches!(self.ts.types.get(t), Type::Class(c) if !self.ts.meta.class(*c).is_abstract && !self.ts.meta.class(*c).is_interface);
        if constructor.is_none() && instantiable {
            if n.args.is_empty() {
                self.warning(cst, vm::DefaultConstructorNotDefinedImplicitUsed);
            } else {
                let qn = self.format_qualified_name(t);
                self.error(cst, bind(vm::UnresolvedConstructor, &[&qn]));
            }
        }
        let ctor = match constructor {
            Some(OpRef::User(o)) => Some(o),
            _ => None,
        };
        let nw = ir::New {
            ty: t,
            extent,
            constructor: ctor,
            args: args.into_iter().map(|x| x.e).collect(),
        };
        self.ex(ExprKind::New(Box::new(nw)), t, cst)
    }

    /// `QvtOperationalEnv.lookupConstructorOperations`
    fn lookup_constructor_operations(
        &mut self,
        owner: TypeId,
        name: &str,
        args: &[crate::ops::Arg],
    ) -> Vec<ir::OpId> {
        let ops = self.get_operations(owner);
        let mut r = Vec::new();
        for o in ops {
            if let OpRef::User(id) = o
                && self.ops[id.index()].ir.name == name
                && self.ops[id.index()].ir.kind == ir::OpKind::Constructor
            {
                let params = self.op_params(o);
                if self.match_args(owner, &params, args) && self.ops[id.index()].owner == owner {
                    r.push(id);
                }
            }
        }
        r
    }

    /// `FormattingHelperImpl.formatQualifiedName`
    pub(crate) fn format_qualified_name(&mut self, t: TypeId) -> String {
        self.qualified_name(t)
    }

    /// `object [x :] [T] { ... }`
    pub(crate) fn object_exp(
        &mut self,
        o: &ast::ObjectExp,
        cst: Span,
        validate: bool,
        top: Option<ir::OpId>,
    ) -> Ex {
        self.object_core(
            o.decl.var.as_ref(),
            o.decl.ty.as_ref(),
            &o.body.stmts,
            cst,
            validate,
            top,
        )
    }

    /// `visitObjectExpCS`; `top`: the mapping whose population this object is directly in.
    pub(crate) fn object_core(
        &mut self,
        var: Option<&ast::Name>,
        ts: Option<&ast::TypeSpec>,
        stmts: &[ast::Expr],
        cst: Span,
        validate: bool,
        top: Option<ir::OpId>,
    ) -> Ex {
        let mut ty: Option<TypeId> = None;
        let mut extent: Option<ir::ModelParamRef> = None;
        let mut instantiated: Option<TypeId> = None;
        if let Some(ts) = ts {
            let (t, e) = self.visit_type_spec(ts, Direction::Out);
            ty = t;
            extent = e;
            if let Some(t) = t
                && let Type::Class(c) = *self.ts.types.get(t)
            {
                instantiated = Some(t);
                let ci = self.ts.meta.class(c);
                if var.is_none() && (ci.is_abstract || ci.is_interface) {
                    let tn = self.qualified_name(t);
                    self.error(
                        cst,
                        bind(
                            vm::QvtOperationalVisitorCS_canNotInstantiateAbstractType,
                            &[&tn],
                        ),
                    );
                }
            }
        } else if let Some(op) = top {
            let info = &self.ops[op.index()].ir;
            if info.results.is_empty()
                && info.context_direction == Direction::InOut
                && let Some(c) = info.context
            {
                ty = Some(c);
            }
        }
        let mut referred: Option<VarRef> = None;
        if let Some(n) = var {
            match self.lookup_var(&n.text) {
                None => self.error(n.span, bind(vm::unresolvedNameError, &[&n.text])),
                Some(v) => referred = Some(v),
            }
        }
        if let (Some(v), None) = (referred, ty) {
            let vt = self.var_type(v);
            if vt != TypeId::NULL {
                ty = Some(vt);
            }
        }
        if let Some(t) = ty
            && matches!(self.ts.types.get(t), Type::Class(_))
        {
            instantiated = Some(t);
        }
        if extent.is_none() {
            if let Some(VarRef::Local(l)) = referred
                && let Some(p) = self.fun.extra[l.index()].param
                && p.mapping_param
            {
                extent = p.extent;
            }
            if extent.is_none()
                && let Some(t) = ty
            {
                let m = self.fun.module.unwrap_or(self.cur_module);
                extent = self.resolve_model_parameter(m, t, Direction::Out);
            }
        }
        let m = self.fun.module.unwrap_or(self.cur_module);
        if extent.is_none() && !self.modules[m.index()].model_params.is_empty() {
            let invalid = match referred {
                None => ty.is_none_or(|t| !self.may_belong_to_extent(t)),
                Some(v) => {
                    let vt = self.var_type(v);
                    let interm = matches!(self.ts.types.get(vt), Type::Class(c) if self.interm_class_module.contains_key(c));
                    interm || !self.ts.ck().is_user_model_element(vt)
                }
            };
            if !invalid {
                let tn = ty.map(|t| self.full_name(t)).unwrap_or_default();
                let span = ts.map(|t| t.span).unwrap_or(cst);
                self.error(
                    span,
                    bind(vm::QvtOperationalVisitorCS_extentFailToInfer, &[&tn]),
                );
            }
        }
        // the body, in a child environment with the object as implicit source for properties
        self.fun.push_scope();
        self.fun.top().parent_local = true;
        let elem_local = match referred {
            Some(VarRef::Local(l)) => l,
            _ => self
                .fun
                .new_local("", ty.unwrap_or(TypeId::NULL), ir::LocalKind::Object, cst),
        };
        let var_name = match var {
            Some(n) if referred.is_some() => n.text.clone(),
            _ => {
                self.fun.name_counter += 1;
                format!("temp{}", self.fun.name_counter)
            }
        };
        if self.fun.locals[elem_local.index()].name.is_empty() {
            self.fun.locals[elem_local.index()].name = var_name.clone();
        }
        let elem_ref = match referred {
            Some(v) => v,
            None => VarRef::Local(elem_local),
        };
        self.add_implicit_property_source(&var_name, elem_ref);
        let self_obj = var.is_some_and(|n| n.text == "self");
        if self_obj {
            self.fun.object_self_depth += 1;
        }
        let body: Vec<Ex> = self.statements(stmts, false);
        if self_obj {
            self.fun.object_self_depth -= 1;
        }
        self.fun.pop_scope();
        if validate {
            self.validate_object_exp(instantiated, elem_ref, ty, var, ts, cst);
        }
        let class = instantiated
            .and_then(|t| match self.ts.types.get(t) {
                Type::Class(c) => Some(*c),
                _ => None,
            })
            .unwrap_or(self.ts.std.element);
        let ob = ir::ObjectExp {
            var: elem_local,
            class,
            extent,
            body: body.into_iter().map(|x| x.e).collect(),
        };
        let mut x = self.ex(
            ExprKind::Object(Box::new(ob)),
            ty.unwrap_or(TypeId::NULL),
            cst,
        );
        x.info = Info::Object;
        x
    }

    /// `validateObjectExp`
    fn validate_object_exp(
        &mut self,
        instantiated: Option<TypeId>,
        referred: VarRef,
        ty: Option<TypeId>,
        var: Option<&ast::Name>,
        ts: Option<&ast::TypeSpec>,
        cst: Span,
    ) {
        let rt = self.var_type(referred);
        let mut derived = instantiated;
        if derived.is_none() && self.ts.ck().is_user_model_element(rt) {
            derived = Some(rt);
        }
        if derived.is_none_or(|d| !self.ts.ck().is_user_model_element(d)) {
            let span = ts.map(|t| t.span).or(var.map(|n| n.span)).unwrap_or(cst);
            if rt != TypeId::NULL && !self.ts.types.is_collection_like(rt) {
                let tn = self.qualified_name(rt);
                self.error(span, bind(vm::nonModelTypeError, &[&tn]));
            }
        }
        if let Some(actual) = ty {
            let span = var.map(|n| n.span).or(ts.map(|t| t.span)).unwrap_or(cst);
            if rt != TypeId::NULL && self.ts.ck().relationship(actual, rt) & rel::SAME_TYPE == 0 {
                let an = self.qualified_name(actual);
                let rn = self.qualified_name(rt);
                self.error(
                    span,
                    bind(
                        vm::QvtOperationalVisitorCS_instantiatedTypeDoesNotConformToReferredType,
                        &[&an, &rn],
                    ),
                );
            }
            if let VarRef::Local(l) = referred
                && let Some(p) = self.fun.extra[l.index()].param
                && p.mapping_param
                && p.direction == Direction::In
            {
                let n = self.fun.locals[l.index()].name.clone();
                self.error(
                    span,
                    bind(vm::QvtOperationalVisitorCS_canNotModifyINParameter, &[&n]),
                );
            }
        }
    }

    // ---- mapping calls -------------------------------------------------------------------------

    /// The span of a CS node that starts after its source (`src.map f()`, `src.resolve(..)`:
    /// the grammar attaches the source without extending the node).
    pub(crate) fn span_after_source(&self, source: Option<&ast::Expr>, cst: Span) -> Span {
        match source {
            Some(s) => {
                let src = self.src();
                let b = src.as_bytes();
                let end = cst.end as usize;
                let mut i = s.span.end as usize;
                while i < end {
                    if b[i].is_ascii_whitespace()
                        || b[i] == b'.'
                        || (b[i] == b'-' && b.get(i + 1) == Some(&b'>'))
                        || b[i] == b'>'
                    {
                        i += 1;
                    } else if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                        i = src[i + 2..]
                            .find("*/")
                            .map(|p| i + 2 + p + 2)
                            .unwrap_or(end);
                    } else if (b[i] == b'/' && b.get(i + 1) == Some(&b'/'))
                        || (b[i] == b'-' && b.get(i + 1) == Some(&b'-'))
                    {
                        i = src[i..].find('\n').map(|p| i + p + 1).unwrap_or(end);
                    } else {
                        break;
                    }
                }
                Span::new(i.min(end) as u32, cst.end)
            }
            None => cst,
        }
    }

    /// `[src.|src->]map|xmap [M::]name(args)`; the "mapping expected" check is the statement
    /// context's (`visitOclExpressionCS`).
    pub(crate) fn map_call(&mut self, m: &ast::MapCall, cst: Span, pos: Pos) -> Option<Ex> {
        let cst = self.span_after_source(m.source.as_ref().map(|(s, _)| s), cst);
        let n = m.name.segments.len();
        let name = &m.name.segments[n - 1];
        let qualifier = &m.name.segments[..n - 1];
        let r = match &m.source {
            Some((s, op)) => {
                let module = qualifier.first().cloned();
                let accessor = if *op == NavOp::Arrow {
                    Accessor::Arrow
                } else {
                    Accessor::Dot
                };
                let cs = CallCs {
                    span: cst,
                    name_span: name.span,
                    source_span: Some(s.span),
                    accessor,
                    module,
                    mapping: Some(m.xmap),
                    pos,
                };
                let src = if *op == NavOp::Arrow {
                    self.collection_source(s)
                } else {
                    self.expr_d(s)
                };
                let args = self.arguments(&m.args);
                self.operation_call(&cs, &name.text, Some(src), args)
            }
            None => {
                let cs = CallCs {
                    span: cst,
                    name_span: name.span,
                    source_span: None,
                    accessor: Accessor::None,
                    module: None,
                    mapping: Some(m.xmap),
                    pos,
                };
                if !qualifier.is_empty() {
                    if let Some(x) =
                        self.non_contextual_qualified_call(&cs, qualifier, name, &m.args)
                    {
                        return Some(x);
                    }
                    // the path is analysed as the source expression (a type)
                    let names: Vec<&str> = qualifier.iter().map(|q| q.text.as_str()).collect();
                    let pspan = path_span_of(qualifier);
                    let src = match self.lookup_classifier(&names) {
                        Some(t) => self.type_exp(t, pspan),
                        None => {
                            let te = ast::TypeExpr {
                                kind: ast::TypeKind::Path(ast::PathName {
                                    segments: qualifier.to_vec(),
                                    span: pspan,
                                }),
                                span: pspan,
                            };
                            self.error(
                                pspan,
                                bind(
                                    vm::UnknownClassifierType,
                                    &[&crate::names::type_string(&te)],
                                ),
                            );
                            self.dummy(pspan)
                        }
                    };
                    let args = self.arguments(&m.args);
                    self.operation_call(&cs, &name.text, Some(src), args)
                } else {
                    let args = self.arguments(&m.args);
                    self.operation_call(&cs, &name.text, None, args)
                }
            }
        };
        Some(r)
    }

    /// `genNonContextualQualifiedOperationCall`: `map M::name(args)` on module `M`.
    fn non_contextual_qualified_call(
        &mut self,
        cs: &CallCs,
        qualifier: &[ast::Name],
        name: &ast::Name,
        args: &[ast::Expr],
    ) -> Option<Ex> {
        let names: Vec<&str> = qualifier.iter().map(|q| q.text.as_str()).collect();
        let t = self.lookup_classifier(&names)?;
        let Type::Module(m) = *self.ts.types.get(t) else {
            return None;
        };
        let mid = ir::ModuleId(m);
        let src = self.var_ex(VarRef::This(mid), path_span_of(qualifier));
        let args = self.arguments(args);
        let (mut r, op) = self.gen_operation_call(cs, &name.text, Some(src), t, args);
        if let Some(o) = op {
            let cur = self.fun.module.unwrap_or(self.cur_module);
            let valid = self.modules[mid.index()].is_library
                || cur == mid
                || self.all_extended(cur).contains(&mid);
            if !valid {
                self.error(
                    name.span,
                    bind(vm::NoImplicitSourceForQualifiedCall, &[&name.text]),
                );
            }
            if let ExprKind::Call(c) = &mut r.e.kind {
                match &mut c.callee {
                    ir::Callee::Op { virtual_, .. } | ir::Callee::Mapping { virtual_, .. } => {
                        *virtual_ = false
                    }
                    _ => {}
                }
            }
            r.info = Info::OpCall(Some(o), Some(VarRef::This(mid)));
            self.validate_operation_call(cs, &r, o, Some(VarRef::This(mid)));
        }
        Some(r)
    }

    /// The `visitOclExpressionCS` check of a mapping call statement: the call must resolve to a
    /// mapping ("Mapping operation expected").
    pub(crate) fn check_mapping_call(&mut self, x: Ex, cst: Span) -> Option<Ex> {
        let ok = match &x.e.kind {
            ExprKind::Call(c) => {
                matches!(c.callee, ir::Callee::Mapping { .. }) && x.info != Info::Invalid
            }
            ExprKind::Loop(l) => {
                matches!(&l.body, Some(b) if matches!(&b.kind, ExprKind::Call(c) if matches!(c.callee, ir::Callee::Mapping { .. })))
            }
            _ => false,
        };
        if ok {
            return Some(x);
        }
        self.error(cst, vm::mappingOperationExpected);
        None
    }

    // ---- resolve -------------------------------------------------------------------------------

    /// `visitResolveExpCS` / `visitResolveInExpCS`
    pub(crate) fn resolve_exp(&mut self, r: &ast::Resolve, cst: Span) -> Ex {
        let spelled = r.op.spelling().unwrap_or("resolve");
        let one = spelled.find("one").is_some_and(|i| i > 0);
        let inverse = spelled.starts_with("inv");
        let kind = match (inverse, one) {
            (false, false) => ir::ResolveKind::Resolve,
            (false, true) => ir::ResolveKind::ResolveOne,
            (true, false) => ir::ResolveKind::InvResolve,
            (true, true) => ir::ResolveKind::InvResolveOne,
        };
        let mut mapping: Option<ir::OpId> = None;
        let mut mapping_result: Option<TypeId> = None;
        let mut mapping_ctx: Option<TypeId> = None;
        let mut fq = String::new();
        let mut single = false;
        if let Some(sn) = &r.mapping {
            let (mname, ctx) = scoped_parts(sn);
            let mname = mname.map(|n| n.text.clone()).unwrap_or_default();
            let ctx_ty = ctx.as_ref().and_then(|c| match c {
                CtxType::Type(t) => self.type_cs_in(t, true),
                CtxType::Path(p) => {
                    let te = ast::TypeExpr {
                        kind: ast::TypeKind::Path(ast::PathName {
                            segments: p.to_vec(),
                            span: path_span_of(p),
                        }),
                        span: path_span_of(p),
                    };
                    self.type_cs_in(&te, true)
                }
            });
            let m = self.fun.module.unwrap_or(self.cur_module);
            let cls = ctx_ty.unwrap_or(self.modules[m.index()].ty);
            let raw = self.lookup_mapping_operations(cls, &mname);
            let unfiltered: Vec<ir::OpId> = raw
                .into_iter()
                .filter(|o| self.ops[o.index()].owner == cls)
                .collect();
            let ops: Vec<ir::OpId> = unfiltered
                .iter()
                .copied()
                .filter(|o| {
                    !self.ops[o.index()]
                        .overridden
                        .is_some_and(|ov| unfiltered.contains(&ov))
                })
                .collect();
            fq = format!("{}::{}", self.classifier_name(cls), mname);
            let name_span = Span::new(sn.span.end.saturating_sub(mname.len() as u32), sn.span.end);
            match ops.len() {
                1 => {
                    single = true;
                    mapping = Some(ops[0]);
                    mapping_result = Some(self.ops[ops[0].index()].ret);
                    mapping_ctx = self.ops[ops[0].index()]
                        .contextual
                        .then_some(self.ops[ops[0].index()].owner);
                }
                0 => self.error(
                    name_span,
                    bind(vm::QvtOperationalVisitorCS_ResolveInMappingNotFound, &[&fq]),
                ),
                _ => {
                    self.error(
                        name_span,
                        bind(
                            vm::QvtOperationalVisitorCS_ambiguousMappingOperationReference,
                            &[&fq],
                        ),
                    );
                    mapping = Some(ops[0]);
                }
            }
        }
        // populateResolveExp
        let source = r
            .source
            .as_ref()
            .and_then(|(s, _)| self.visit_ocl_expression(s));
        let mut target_ty = None;
        let mut cond_var = None;
        let mut cond_ir = None;
        let mut target_var_ty = None;
        let mut target_span = cst;
        let ty;
        if let Some(t) = &r.target {
            let tt = self.visit_type_cs(&t.ty);
            target_ty = tt;
            target_var_ty = tt;
            target_span = match &t.var {
                Some(v) => v.span.to(t.ty.span),
                None => t.ty.span,
            };
            let mut clash = false;
            let mut local = None;
            if let Some(v) = &t.var {
                if self.lookup_local(&v.text).is_some() {
                    clash = true;
                    self.error(target_span, bind(vm::NameAlreadyDefinedError, &[&v.text]));
                }
                local = Some(self.fun.new_local(
                    &v.text,
                    tt.unwrap_or(TypeId::NULL),
                    ir::LocalKind::ResolveVar,
                    target_span,
                ));
            }
            let base = tt.unwrap_or(TypeId::NULL);
            ty = if one {
                base
            } else {
                self.ts.types.collection(CollKind::Sequence, base)
            };
            if let Some(c) = &t.cond {
                if let (false, Some(l), Some(v)) = (clash, local, &t.var) {
                    self.add_var(&v.text, VarRef::Local(l), true);
                }
                let cx = self.visit_ocl_expression(c);
                if let (false, Some(_), Some(v)) = (clash, local, &t.var) {
                    self.delete_var(&v.text);
                }
                if let Some(cx) = cx {
                    if cx.ty() != TypeId::BOOLEAN {
                        let n = self.type_name(cx.ty());
                        self.error(
                            cx.e.span,
                            bind(
                                vm::QvtOperationalVisitorCS_booleanTypeExpressionExpected,
                                &[&n],
                            ),
                        );
                    }
                    cond_ir = Some(cx.e);
                }
            }
            cond_var = local;
        } else {
            let base = if inverse {
                mapping_ctx.unwrap_or(TypeId::OCL_ANY)
            } else {
                mapping_result.unwrap_or(TypeId::OCL_ANY)
            };
            ty = if one {
                base
            } else {
                self.ts.types.collection(CollKind::Sequence, base)
            };
        }
        // the ResolveExp's positions are its own CS node's, without the source
        let own = self.span_after_source(r.source.as_ref().map(|(s, _)| s), cst);
        if r.late {
            self.late_resolves.push(LateResolve {
                span: own,
                ok: false,
            });
        }
        if single {
            let mp = mapping.unwrap();
            let info = &self.ops[mp.index()];
            let valid = if inverse {
                info.contextual
            } else {
                !info.ir.results.is_empty()
            };
            if !valid {
                self.warning(
                    own,
                    bind(vm::QvtOperationalVisitorCS_invalidResolveInMapping, &[&fq]),
                );
            } else if let Some(tv) = target_var_ty {
                let expected = if inverse {
                    mapping_ctx.unwrap_or(TypeId::NULL)
                } else {
                    mapping_result.unwrap_or(TypeId::NULL)
                };
                if self.ts.ck().relationship(expected, tv) & rel::RELATED_TYPE == 0 {
                    let tn = self.full_name(tv);
                    self.warning(
                        target_span,
                        bind(
                            vm::QvtOperationalVisitorCS_incompatibleTargetVariableType,
                            &[&tn, &fq],
                        ),
                    );
                }
            }
        }
        let rs = ir::Resolve {
            kind,
            late: r.late,
            source: source.map(|s| s.e),
            mapping,
            target_ty,
            cond_var,
            cond: cond_ir,
        };
        let mut x = self.ex(ExprKind::Resolve(Box::new(rs)), ty, own);
        x.info = Info::Resolve;
        x
    }

    /// Marks late resolves assigned as Eclipse supports for deferred assignment.
    fn mark_late_resolve_assigned(&mut self, rhs: &ir::Expr, prop_target: bool) {
        let span = match &rhs.kind {
            ExprKind::Resolve(r) if r.late && prop_target => Some(rhs.span),
            ExprKind::Call(c) => match (&c.callee, &c.source) {
                (ir::Callee::Std(op), Some(src))
                    if matches!(op.name(), "asBag" | "asSequence" | "asSet" | "asOrderedSet")
                        && op.desc().section == qvto_types::Section::Ocl =>
                {
                    match &src.kind {
                        ExprKind::Resolve(r)
                            if r.late && self.ts.types.is_collection_like(src.ty) =>
                        {
                            Some(src.span)
                        }
                        _ => None,
                    }
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(s) = span {
            for l in self.late_resolves.iter_mut() {
                if l.span == s {
                    l.ok = true;
                }
            }
        }
    }

    /// `validate(env)`: late resolves not used in deferred assignments.
    pub(crate) fn validate_late_resolves(&mut self) {
        let list = self.late_resolves.clone();
        for l in list {
            if !l.ok {
                self.warning(l.span, vm::lateResolveNotUsedInDeferredAssignment);
            }
        }
    }

    pub(crate) fn handle_ambiguous_ops_pub(
        &mut self,
        problem: Span,
        matches: Vec<OpRef>,
    ) -> Option<OpRef> {
        if matches.is_empty() {
            return None;
        }
        let mut parts = Vec::new();
        for o in &matches {
            let s = self.format_op_signature(*o);
            parts.push(format!("'{s}'"));
        }
        self.warning(
            problem,
            bind(vm::AmbiguousOperationReference, &[&parts.join(", ")]),
        );
        Some(matches[0])
    }
}

#[derive(Clone, Copy)]
enum IterVar<'a> {
    Decl(&'a ast::VarDecl),
    Name(&'a ast::Name),
}

#[derive(Clone, Copy)]
enum XTarget<'a> {
    Decl(&'a ast::VarDecl),
    Name(&'a ast::Name),
}

#[derive(Clone, Copy)]
enum XBody<'a> {
    Expr(&'a ast::Expr),
    /// `src->name`: the name analysed as a variable expression.
    Name(&'a ast::Name),
    Object(&'a ast::ObjectDecl, &'a ast::Block),
    Switch(&'a ast::SwitchBody),
}

impl XBody<'_> {
    fn span(&self) -> Span {
        match self {
            XBody::Expr(e) => e.span,
            XBody::Name(n) => n.span,
            XBody::Object(d, b) => d.span.to(b.span),
            XBody::Switch(s) => s.span,
        }
    }
}

/// `isInnermostIteratorRelated`: a call body must be sourced on the iterator.
fn innermost_iterator_related(it: ir::LocalId, body: &ir::Expr) -> bool {
    let src = match &body.kind {
        ExprKind::Call(c) => c.source.as_ref(),
        ExprKind::Prop(s, _) | ExprKind::TupleProp(s, _) | ExprKind::IntermProp(s, _) => Some(&**s),
        ExprKind::Loop(l) => Some(&l.source),
        ExprKind::Global(_) => return false,
        ExprKind::Resolve(r) => r.source.as_ref(),
        _ => return true,
    };
    match src {
        Some(s) => match &s.kind {
            ExprKind::Local(l) => *l == it,
            _ => false,
        },
        None => false,
    }
}

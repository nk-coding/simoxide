//! Expressions: classic OCL's `AbstractOCLAnalyzer` (names, navigation, operation calls,
//! literals, `let`, `if`) with QVTo's overrides from `QvtOperationalVisitorCS` (implicit
//! `xcollect`, qualified calls, deprecation checks, `validateOperationCall`).

use crate::analyzer::Sema;
use crate::env::VarRef;
use crate::ir::{self, CollPart, ExprKind, LoopKind};
use crate::messages::{bind, ocl as om, validation as vm};
use crate::names::type_string;
use crate::ops::{Arg, OpRef, PropRef};
use qvto_syntax::ast::{self, NavOp};
use qvto_syntax::{Span, TokenKind};
use qvto_types::{CollKind, Section, StdOp, StdOwner, Type, TypeId};

/// What Eclipse's AST node is, for the checks that test node classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Info {
    #[default]
    Other,
    /// `TypeExp` referring to a type.
    TypeExp(TypeId),
    /// `VariableExp` (`None`: the variable Eclipse creates for a failed implicit source).
    Var(Option<VarRef>),
    /// `PropertyCallExp`: the property, the source variable (if the source is a `VariableExp`)
    /// and the source type.
    Prop(PropRef, Option<VarRef>, TypeId),
    /// `OperationCallExp`: the operation and the source variable.
    OpCall(Option<OpRef>, Option<VarRef>),
    /// `IteratorExp` (OCL)
    Iterator,
    /// `ImperativeIterateExp`
    ImperativeIterate,
    /// `InvalidLiteralExp`
    Invalid,
    StringLit,
    /// `ReturnExp`, `RaiseExp`
    Return,
    /// `BlockExp`
    Block,
    /// `IfExp`
    If,
    /// `SwitchExp`
    Switch,
    /// `ComputeExp`
    Compute,
    /// `ResolveExp`
    Resolve,
    /// `ObjectExp`
    Object,
}

/// An analysed expression.
#[derive(Clone, Debug)]
pub(crate) struct Ex {
    pub e: ir::Expr,
    /// `isErrorNode`
    pub err: bool,
    pub info: Info,
    /// `isOclExpDeterministic`
    pub det: bool,
}

impl Ex {
    pub fn ty(&self) -> TypeId {
        self.e.ty
    }
}

/// Where an expression stands (`isPossibleEqualityInsteadOfAssignment` looks at the container).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Pos {
    #[default]
    Other,
    /// Directly in a block-like container (object, mapping section, helper body, constructor,
    /// block).
    Block,
    /// `then`/`else` of an OCL `if`, or the body of a switch alternative, whose container is
    /// block-like.
    Branch,
    /// The last expression of a query body.
    QueryLast,
}

impl Pos {
    pub fn warns_equality(self) -> bool {
        matches!(self, Pos::Block | Pos::Branch)
    }
}

pub(crate) fn mk(kind: ExprKind, ty: TypeId, span: Span) -> ir::Expr {
    ir::Expr { kind, ty, span }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Accessor {
    Dot,
    Arrow,
    None,
}

/// The parts of an operation call's CST node the analyser reports on.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct CallCs {
    pub span: Span,
    pub name_span: Span,
    /// The source's CST node.
    pub source_span: Option<Span>,
    pub accessor: Accessor,
    /// `M::op(...)` (`ImperativeOperationCallExpCS.getModule`)
    pub module: Option<ast::Name>,
    /// `MappingCallExpCS` and its `isStrict` (`xmap`).
    pub mapping: Option<bool>,
    pub pos: Pos,
}

impl<'r> Sema<'r> {
    pub fn ex(&self, kind: ExprKind, ty: TypeId, span: Span) -> Ex {
        Ex {
            e: mk(kind, ty, span),
            err: false,
            info: Info::Other,
            det: false,
        }
    }

    /// `createDummyInvalidLiteralExp`
    pub fn dummy(&self, span: Span) -> Ex {
        Ex {
            e: mk(ExprKind::Invalid, TypeId::OCL_INVALID, span),
            err: true,
            info: Info::Invalid,
            det: false,
        }
    }

    pub fn var_ex(&self, v: VarRef, span: Span) -> Ex {
        let ty = self.var_type(v);
        let kind = match v {
            VarRef::Local(l) => ExprKind::Local(l),
            VarRef::This(m) => ExprKind::This(m),
            VarRef::ModelParam(p) => ExprKind::ModelParam(p),
        };
        Ex {
            e: mk(kind, ty, span),
            err: false,
            info: Info::Var(Some(v)),
            det: false,
        }
    }

    /// `oclExpressionCS`; `None` where Eclipse gives null.
    pub fn expr(&mut self, e: &ast::Expr) -> Option<Ex> {
        self.expr_at(e, Pos::Other, e.span)
    }

    /// `oclExpressionCS`, a dummy where Eclipse gives null.
    pub fn expr_d(&mut self, e: &ast::Expr) -> Ex {
        match self.expr(e) {
            Some(x) => x,
            None => self.dummy(e.span),
        }
    }

    /// Analyses `e` whose CST node spans `cst` (parentheses extend the inner node).
    pub fn expr_at(&mut self, e: &ast::Expr, pos: Pos, cst: Span) -> Option<Ex> {
        use ast::ExprKind as K;
        let r = match &e.kind {
            K::Paren(inner) => {
                let mut x = self.expr_at(inner, pos, cst)?;
                x.e.span = cst;
                return Some(x);
            }
            K::Int(v) => self.ex(ExprKind::Int(*v), TypeId::INTEGER, cst),
            K::Real(t) => {
                let v = parse_real(t);
                self.ex(ExprKind::Real(v), TypeId::REAL, cst)
            }
            K::Str(s) => {
                let mut x = self.ex(ExprKind::Str(s.value.clone().into()), TypeId::STRING, cst);
                x.info = Info::StringLit;
                x
            }
            K::Bool(b) => self.ex(ExprKind::Bool(*b), TypeId::BOOLEAN, cst),
            K::Null => self.ex(ExprKind::Null, TypeId::OCL_VOID, cst),
            K::Invalid => {
                let mut x = self.ex(ExprKind::Invalid, TypeId::OCL_INVALID, cst);
                x.info = Info::Invalid;
                x
            }
            K::Unlimited => self.ex(ExprKind::Unlimited, TypeId::UNLIMITED_NATURAL, cst),
            K::SelfExp => self.variable_exp("self", cst),
            K::Name(n) => self.variable_exp(&n.text, cst),
            K::Path(p) => self.path_exp(p, cst),
            K::TypeLit(t) => self.type_lit(t, cst),
            K::CollectionLit(k, parts) => self.collection_literal(*k, parts, cst),
            K::ListLit(parts) => self.list_literal(parts, cst),
            K::DictLit(parts) => self.dict_literal(parts, cst),
            K::TupleLit(parts) => self.tuple_literal(parts, cst),
            K::Unary(op, a) => self.unary(*op, a, cst, pos),
            K::Binary(op, a, b) => self.binary(*op, a, b, cst, pos),
            K::Let(decls, body) => self.let_exp(decls, body, cst),
            K::If(i) => self.if_exp(i, cst, pos),
            K::Call(c) => self.call_exp(c, cst, pos),
            K::Nav(n) => self.nav_exp(n, cst),
            K::Iterate(it) => self.iterate_exp(it, cst),
            K::XIterate(x) => self.ximperative_iterate(x, cst),
            K::XSelect(x) => self.xselect_shorthand(x, cst),
            K::ForEach(f) => self.for_exp(f, cst),
            K::ObjectIter(o) => self.object_iter(o, cst),
            K::SwitchIter(s) => self.switch_iter(s, cst),
            K::Assign(a) => return self.assign_statement(a, cst),
            K::Var(v) => self.var_statement(v, cst),
            K::Return(v) => self.return_exp(v.as_deref(), cst),
            K::Break => self.ex(ExprKind::Break, TypeId::OCL_VOID, cst),
            K::Continue => self.ex(ExprKind::Continue, TypeId::OCL_VOID, cst),
            K::While(w) => self.while_exp(w, cst),
            K::Block(b) | K::Do(b) => self.block_exp(b, cst),
            K::Switch(s) => self.switch_exp(s, cst, pos),
            K::Log(l) => self.log_exp(l, cst),
            K::Assert(a) => self.assert_exp(a, cst),
            K::Compute(c) => self.compute_exp(c, cst),
            K::New(n) => self.new_exp(n, cst),
            K::Object(o) => self.object_exp(o, cst, true, None),
            K::MapCall(m) => return self.map_call(m, cst, pos),
            K::Resolve(r) => self.resolve_exp(r, cst),
            K::Raise(r) => self.raise_exp(r, cst),
            K::Try(t) => self.try_exp(t, cst),
            K::Error => self.dummy(cst),
        };
        Some(r)
    }

    /// `visitOclExpressionCS`: statement-level analysis (a mapping call must resolve to a
    /// mapping).
    pub fn visit_ocl_expression(&mut self, e: &ast::Expr) -> Option<Ex> {
        self.visit_ocl_expression_at(e, Pos::Other)
    }

    pub fn visit_ocl_expression_at(&mut self, e: &ast::Expr, pos: Pos) -> Option<Ex> {
        self.visit_ocl_expression_cst(e, pos, e.span)
    }

    pub fn visit_ocl_expression_cst(&mut self, e: &ast::Expr, pos: Pos, cst: Span) -> Option<Ex> {
        let x = self.expr_at(e, pos, cst)?;
        let mut inner = e;
        while let ast::ExprKind::Paren(i) = &inner.kind {
            inner = i;
        }
        if let ast::ExprKind::MapCall(m) = &inner.kind {
            let sp = self.span_after_source(m.source.as_ref().map(|(s, _)| s), cst);
            return self.check_mapping_call(x, sp);
        }
        Some(x)
    }

    // ---- names ---------------------------------------------------------------------------------

    /// `variableExpCS`: a simple name without source.
    fn variable_exp(&mut self, name: &str, cst: Span) -> Ex {
        let r = self.simple_name(name, cst, None, cst);
        if let Info::Prop(p, src, _) = r.info {
            // DeprecatedImplicitSourceCallHelper.validateCallExp (cause: the VariableExpCS)
            self.validate_implicit_source(cst, src, None, Some(p));
        }
        r
    }

    /// `customSimpleNameCS`: type, variable, property, then the error. `source` is the explicit
    /// source of `src.name`.
    pub(crate) fn simple_name(
        &mut self,
        name: &str,
        name_span: Span,
        source: Option<Ex>,
        cst: Span,
    ) -> Ex {
        if let Some(s) = &source
            && s.err
        {
            // the unparseable source itself, with the CS node's positions
            let mut x = source.unwrap();
            x.e.span = cst;
            return x;
        }
        let explicit = source.is_some();
        if !explicit {
            if let Some(t) = self.lookup_classifier(&[name]) {
                return self.type_exp(t, name_span);
            }
            if let Some(v) = self.lookup_var(name) {
                return self.var_ex(v, name_span);
            }
        }
        // a property
        let (src, owner) = match source {
            Some(s) => {
                let t = s.ty();
                let o = if self.ts.types.is_collection_like(t) {
                    self.ts.types.element(t).unwrap()
                } else {
                    t
                };
                (Some(s), o)
            }
            None => match self.implicit_source_for_property(name) {
                Some(v) => {
                    let x = self.var_ex(v, name_span);
                    let t = x.ty();
                    (Some(x), t)
                }
                None => (None, TypeId::NULL),
            },
        };
        let prop = if owner == TypeId::NULL {
            None
        } else {
            self.lookup_property(owner, name)
        };
        if let (Some(p), Some(src)) = (prop, src) {
            let pty = self.prop_type(p);
            let src_var = if let Info::Var(v) = src.info { v } else { None };
            let st = src.ty();
            if explicit && self.ts.types.is_collection_like(st) {
                // implicit collect over the source collection
                let (it, it_name) = self.new_temp_iterator(st);
                let iter_ex = self.var_ex(VarRef::Local(it), name_span);
                let body = self.property_call(p, iter_ex, pty, name_span);
                self.delete_var(&it_name);
                return self.implicit_collect(src, it, body, cst);
            }
            let span = if explicit { cst } else { name_span };
            let mut r = self.property_call(p, src, pty, span);
            r.info = Info::Prop(p, src_var, st);
            return r;
        }
        // undefined
        if explicit && self.lookup_var(name).is_some() {
            self.error(name_span, bind(om::VarInNavExp_ERROR_, &[name]));
        } else {
            self.error(name_span, bind(om::UnrecognizedVar_ERROR_, &[name]));
        }
        self.dummy(name_span)
    }

    /// The IR of a property call on a single source.
    pub(crate) fn property_call(&mut self, p: PropRef, src: Ex, ty: TypeId, span: Span) -> Ex {
        let kind = match p {
            PropRef::Feature(f) => ExprKind::Prop(Box::new(src.e), f),
            PropRef::Global(g) => match src.e.kind {
                ExprKind::This(_) => ExprKind::Global(g),
                _ => ExprKind::ModuleProp(Box::new(src.e), g),
            },
            PropRef::Interm(i) => ExprKind::IntermProp(Box::new(src.e), i),
            PropRef::TuplePart(t, i) => {
                let name = match self.ts.types.get(t) {
                    Type::Tuple(parts) => parts[i as usize].0.clone(),
                    _ => "".into(),
                };
                ExprKind::TupleProp(Box::new(src.e), name)
            }
        };
        let mut x = self.ex(kind, ty, span);
        x.err = src.err;
        x
    }

    /// A generated implicit iterator variable over the elements of a collection type
    /// (`genVariableDeclaration(null, ...)`, added to the environment as an implicit variable).
    pub fn new_temp_iterator(&mut self, coll: TypeId) -> (ir::LocalId, String) {
        let elem = self.ts.types.element(coll).unwrap_or(TypeId::OCL_VOID);
        self.new_temp_var(elem, ir::LocalKind::Temp)
    }

    pub fn new_temp_var(&mut self, ty: TypeId, kind: ir::LocalKind) -> (ir::LocalId, String) {
        let l = self.fun.new_local("", ty, kind, Span::default());
        let name = self.add_var_generated(VarRef::Local(l));
        self.fun.locals[l.index()].name = name.clone();
        (l, name)
    }

    /// `createImplicitCollect` (with QVTo's List rule).
    pub fn implicit_collect(&mut self, src: Ex, it: ir::LocalId, body: Ex, cst: Span) -> Ex {
        let mut bt = body.ty();
        if self.ts.types.is_collection_like(bt) {
            bt = self.flattened_element(bt);
        }
        let st = src.ty();
        let seq = matches!(
            self.ts.types.get(st),
            Type::Collection(CollKind::Sequence | CollKind::OrderedSet, _) | Type::List(_)
        );
        let ty = self.ts.types.collection(
            if seq {
                CollKind::Sequence
            } else {
                CollKind::Bag
            },
            bt,
        );
        let err = src.err;
        let lp = ir::Loop {
            kind: LoopKind::Collect,
            source: src.e,
            iterators: vec![it],
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

    /// `createImplicitXCollect`
    pub fn implicit_xcollect(&mut self, src: Ex, it: ir::LocalId, body: Ex, cst: Span) -> Ex {
        let st = src.ty();
        let seq = matches!(
            self.ts.types.get(st),
            Type::Collection(CollKind::Sequence | CollKind::OrderedSet, _) | Type::List(_)
        );
        let ty = self.ts.types.collection(
            if seq {
                CollKind::Sequence
            } else {
                CollKind::Bag
            },
            body.ty(),
        );
        let lp = ir::Loop {
            kind: LoopKind::XCollect,
            source: src.e,
            iterators: vec![it],
            acc: None,
            target: None,
            body: Some(body.e),
            cond: None,
            filter_type: None,
        };
        let mut x = self.ex(ExprKind::Loop(Box::new(lp)), ty, cst);
        x.info = Info::ImperativeIterate;
        x
    }

    /// `a::b::c` as an expression (`modelPropertyCallExpCS` without source): an enumeration
    /// literal, a static property or a type.
    fn path_exp(&mut self, p: &ast::PathName, cst: Span) -> Ex {
        let n = p.segments.len();
        let names: Vec<&str> = p.segments.iter().map(|s| s.text.as_str()).collect();
        let last = names[n - 1];
        let path = &names[..n - 1];
        let last_span = p.segments[n - 1].span;
        match self.lookup_classifier(path) {
            None => match self.lookup_classifier(&names) {
                Some(t) => self.type_exp(t, cst),
                None => {
                    let msg = self.unrecognized_type_message(&names);
                    self.error(cst, msg);
                    self.dummy(cst)
                }
            },
            Some(owner) => {
                if let Type::Enum(e) = *self.ts.types.get(owner) {
                    let info = self.ts.meta.enumeration(e);
                    let lit = info
                        .by_name(last)
                        .or_else(|| last.strip_prefix('_').and_then(|u| info.by_name(u)));
                    if let Some(i) = lit {
                        return self.ex(ExprKind::EnumLit(e, i), owner, cst);
                    }
                }
                if let Some(p2) = self.lookup_property(owner, last) {
                    self.error(last_span, bind(om::NonStaticAttribute_ERROR_, &[last]));
                    let pty = self.prop_type(p2);
                    let src = self.type_exp(owner, path_span_of(&p.segments[..n - 1]));
                    return self.property_call(p2, src, pty, cst);
                }
                match self.lookup_classifier(&names) {
                    Some(t) => self.type_exp(t, cst),
                    None => {
                        self.error(cst, bind(om::UnrecognizedEnum_ERROR_, &[last]));
                        self.dummy(cst)
                    }
                }
            }
        }
    }

    /// `getUnrecognizedTypeMessage` bound with the name list (`[a, b]`).
    pub fn unrecognized_type_message(&self, names: &[&str]) -> String {
        bind(
            om::UnrecognizedType_ERROR_,
            &[&format!("[{}]", names.join(", "))],
        )
    }

    pub fn type_exp(&mut self, t: TypeId, span: Span) -> Ex {
        let tt = self.ts.types.type_type(t);
        let mut x = self.ex(ExprKind::Type(t), tt, span);
        x.info = Info::TypeExp(t);
        x
    }

    /// A type used as an expression (`TypeCS` in expression position).
    fn type_lit(&mut self, t: &ast::TypeExpr, cst: Span) -> Ex {
        match self.type_cs(t) {
            Some(ty) => self.type_exp(ty, cst),
            None => {
                self.error(cst, bind(vm::UnknownClassifierType, &[&type_string(t)]));
                self.dummy(cst)
            }
        }
    }

    // ---- literals ------------------------------------------------------------------------------

    fn collection_part(&mut self, p: &ast::CollectionPart) -> (CollPart, TypeId, bool) {
        match p {
            ast::CollectionPart::Item(e) => {
                let x = self.expr_d(e);
                let t = x.ty();
                (CollPart::Item(x.e), t, x.err)
            }
            ast::CollectionPart::Range(a, b) => {
                let x = self.expr_d(a);
                let y = self.expr_d(b);
                let t = x.ty();
                (CollPart::Range(x.e, y.e), t, x.err || y.err)
            }
        }
    }

    /// The parts of a collection or `List` literal and their common supertype; problems of the
    /// common supertype are reported at the first part (Eclipse's `colPart`).
    fn literal_parts(
        &mut self,
        parts: &[ast::CollectionPart],
        _cst: Span,
    ) -> (Vec<CollPart>, Option<TypeId>, bool) {
        let mut out = Vec::new();
        let mut elem: Option<TypeId> = None;
        let mut err = false;
        let first = parts.first().map(|p| match p {
            ast::CollectionPart::Item(e) => e.span,
            ast::CollectionPart::Range(a, b) => Span {
                start: a.span.start,
                end: b.span.end,
            },
        });
        for p in parts {
            let (cp, t, e) = self.collection_part(p);
            elem = Some(match elem {
                None => t,
                Some(prev) => self.common_super_type_or_void(first.unwrap(), prev, t),
            });
            err |= e;
            out.push(cp);
        }
        (out, elem, err)
    }

    /// `collectionLiteralExpCS`
    fn collection_literal(&mut self, k: TokenKind, parts: &[ast::CollectionPart], cst: Span) -> Ex {
        let kind = crate::names::coll_kind(k);
        let (out, elem, err) = self.literal_parts(parts, cst);
        let ty = self
            .ts
            .types
            .collection(kind, elem.unwrap_or(TypeId::OCL_VOID));
        let mut x = self.ex(ExprKind::Coll(kind, out), ty, cst);
        x.err = err;
        x
    }

    /// `getCommonSuperType` of the analyser: errors reported at `cst`, `OclVoid` on failure.
    pub fn common_super_type_or_void(&mut self, cst: Span, a: TypeId, b: TypeId) -> TypeId {
        self.common_super_type_at(cst, a, b)
            .unwrap_or(TypeId::OCL_VOID)
    }

    /// `TypeUtil.commonSuperType(problemObject, ...)`: `None` (with the error) when there is
    /// none.
    pub fn common_super_type_at(&mut self, cst: Span, a: TypeId, b: TypeId) -> Option<TypeId> {
        use qvto_types::SuperTypeError as E;
        let mut errors = Vec::new();
        let t = self.ts.ck().common_super_type(a, b, &mut errors);
        for e in errors {
            let msg = match e {
                E::TypeMismatch(x, y) => bind(om::TypeMismatch_ERROR_, &[&x, &y]),
                E::TupleTypeMismatch(x, y) => bind(om::TupleTypeMismatch_ERROR_, &[&x, &y]),
                E::TupleFieldNumMismatch(x, y) => bind(om::TupleFieldNumMismatch_ERROR_, &[&x, &y]),
                E::TupleFieldNotFound(x, f, y) => {
                    bind(om::TupleFieldNotFound_ERROR_, &[&x, &f, &y])
                }
            };
            self.error(cst, msg);
        }
        t
    }

    /// `listLiteralExpCS`
    fn list_literal(&mut self, parts: &[ast::CollectionPart], cst: Span) -> Ex {
        let (out, elem, err) = self.literal_parts(parts, cst);
        let ty = self.ts.types.list(elem.unwrap_or(TypeId::OCL_VOID));
        let mut x = self.ex(ExprKind::List(out), ty, cst);
        x.err = err;
        x
    }

    /// `dictionaryLiteralExp`
    fn dict_literal(&mut self, parts: &[ast::DictPart], cst: Span) -> Ex {
        let mut out = Vec::new();
        for p in parts {
            let k = self.expr_d(&p.key);
            let v = self.expr_d(&p.value);
            out.push((k.e, v.e));
        }
        let mut kt: Option<TypeId> = None;
        let mut vt: Option<TypeId> = None;
        let mut failed = false;
        for (k, v) in &out {
            let (nk, nv) = (k.ty, v.ty);
            match (kt, vt) {
                (Some(a), Some(b)) => {
                    kt = self.common_super_type_at(cst, nk, a);
                    if kt.is_some() {
                        vt = self.common_super_type_at(cst, nv, b);
                    }
                }
                _ => {
                    kt = Some(nk);
                    vt = Some(nv);
                }
            }
            if kt.is_none() || vt.is_none() {
                failed = true;
                break;
            }
        }
        let ty = match (kt, vt, failed) {
            (Some(a), Some(b), false) => self.ts.types.dict(a, b),
            (None, None, false) => self.ts.types.dict(TypeId::OCL_VOID, TypeId::OCL_VOID),
            _ => TypeId::OCL_VOID,
        };
        self.ex(ExprKind::Dict(out), ty, cst)
    }

    /// `tupleLiteralExpCS`
    fn tuple_literal(&mut self, parts: &[ast::VarDecl], cst: Span) -> Ex {
        let mut names: Vec<String> = Vec::new();
        let mut tparts: Vec<(Box<str>, TypeId)> = Vec::new();
        let mut values = Vec::new();
        let mut err = false;
        for v in parts {
            let dty = v.ty.as_ref().and_then(|t| self.type_cs(t));
            let val = v.init.as_ref().map(|(_, e)| self.expr_d(e));
            let name = v.name.text.clone();
            if names.contains(&name) {
                self.error(v.span, bind(om::DuplicateNameInTuple_ERROR_, &[&name]));
            } else {
                names.push(name.clone());
            }
            let ty = match dty {
                Some(t) => t,
                None => val.as_ref().map(|x| x.ty()).unwrap_or(TypeId::OCL_VOID),
            };
            if let Some(x) = &val {
                err |= x.err;
            }
            tparts.push((name.clone().into(), ty));
            values.push((
                name.into_boxed_str(),
                val.map(|x| x.e)
                    .unwrap_or_else(|| mk(ExprKind::Null, TypeId::OCL_VOID, v.span)),
            ));
        }
        let ty = self.tuple_type(tparts);
        let mut x = self.ex(ExprKind::Tuple(values), ty, cst);
        x.err = err;
        x
    }

    // ---- let, if -------------------------------------------------------------------------------

    /// `letExpCSRecursive`
    fn let_exp(&mut self, decls: &[ast::VarDecl], body: &ast::Expr, cst: Span) -> Ex {
        let Some((d, rest)) = decls.split_first() else {
            return self.expr_d(body);
        };
        let (l, init) = self.variable_declaration(d, ir::LocalKind::Let, true);
        let name = d.name.text.clone();
        if self.fun.locals[l.index()].ty == TypeId::NULL {
            self.error(d.span, bind(om::DeclarationType_ERROR_, &[&name]));
            self.fun.locals[l.index()].ty = TypeId::OCL_INVALID;
        }
        if init.is_none() {
            self.error(d.span, bind(om::DeclarationNoInitExp_ERROR_, &[&name]));
        }
        let sub = self.let_exp(rest, body, cst);
        self.delete_var(&name);
        let ty = sub.ty();
        let init_e = init
            .map(|x| x.e)
            .unwrap_or_else(|| mk(ExprKind::Null, TypeId::OCL_VOID, d.span));
        let mut x = self.ex(ExprKind::Let(l, Box::new(init_e), Box::new(sub.e)), ty, cst);
        x.err = sub.err;
        x
    }

    /// `variableDeclarationCS` (QVTo: the type defaults to the initialiser's): a new local,
    /// added to the environment when `add`. Returns the local and the analysed initialiser.
    pub fn variable_declaration(
        &mut self,
        d: &ast::VarDecl,
        kind: ir::LocalKind,
        add: bool,
    ) -> (ir::LocalId, Option<Ex>) {
        let ty = d.ty.as_ref().and_then(|t| self.type_cs(t));
        let init = d.init.as_ref().map(|(_, e)| self.expr_d(e));
        let ty = ty
            .or_else(|| init.as_ref().map(|x| x.ty()))
            .unwrap_or(TypeId::NULL);
        let l = self.fun.new_local(&d.name.text, ty, kind, d.span);
        if add {
            self.add_var_or_error(&d.name.text, l, d.span);
        }
        (l, init)
    }

    /// `AbstractOCLAnalyzer.genVariableDeclaration`'s `VariableExists_ERROR_` when the name is
    /// already defined locally.
    pub fn add_var_or_error(&mut self, name: &str, l: ir::LocalId, span: Span) {
        if !self.add_var(name, VarRef::Local(l), true) {
            self.error(span, bind(om::VariableUsed_ERROR_, &[name]));
        }
    }

    /// `if c then a else b endif` (OCL) or the switch Eclipse makes of the other forms.
    fn if_exp(&mut self, i: &ast::IfExp, cst: Span, pos: Pos) -> Ex {
        let ocl = !i.c_style && i.endif && i.elifs.is_empty();
        if !ocl {
            return self.if_as_switch(i, cst, pos);
        }
        let c = self.expr_d(&i.cond);
        let inner = if pos == Pos::Block {
            Pos::Branch
        } else {
            Pos::Other
        };
        let t = self.expr_at(&i.then, inner, i.then.span);
        let e = i.els.as_ref().and_then(|x| {
            let sp = self.else_span(i.then.span.end, x.span);
            self.expr_at(x, inner, sp)
        });
        if !c.err && c.ty() != TypeId::BOOLEAN {
            let s = self.src();
            let text = i.cond.span.text(&s).to_string();
            self.error(i.cond.span, bind(om::BooleanForIf_ERROR_, &[&text]));
        }
        let ty = match (&t, &e) {
            (Some(a), Some(b)) => self.common_super_type_or_void(cst, a.ty(), b.ty()),
            _ => TypeId::OCL_VOID,
        };
        let det = t.as_ref().is_some_and(|x| x.det) && e.as_ref().is_some_and(|x| x.det);
        let err = match (&t, &e) {
            (Some(a), Some(b)) => a.err || b.err,
            _ => false,
        };
        let then_e = t
            .map(|x| x.e)
            .unwrap_or_else(|| mk(ExprKind::Null, TypeId::OCL_VOID, i.then.span));
        let mut x = self.ex(
            ExprKind::If(Box::new(c.e), Box::new(then_e), e.map(|x| Box::new(x.e))),
            ty,
            cst,
        );
        x.err = err;
        x.det = det;
        x.info = Info::If;
        x
    }

    /// `createIfExpCSExt`: a switch with the condition/then as first alternative.
    fn if_as_switch(&mut self, i: &ast::IfExp, cst: Span, pos: Pos) -> Ex {
        let mut alts: Vec<(&ast::Expr, &ast::Expr)> = vec![(&i.cond, &i.then)];
        for (c, t) in &i.elifs {
            alts.push((c, t));
        }
        let else_cst = match (&i.els, i.c_style) {
            (Some(x), false) => {
                let from = i
                    .elifs
                    .last()
                    .map(|(_, t)| t.span.end)
                    .unwrap_or(i.then.span.end);
                Some(self.else_span(from, x.span))
            }
            (Some(x), true) => Some(x.span),
            _ => None,
        };
        self.switch_common_spans(&alts, i.els.as_ref().zip(else_cst), cst, pos)
    }

    /// `ifElseOpt ::= else ifExpBodyCS`: the else part's CS node starts at `else`.
    fn else_span(&self, from: u32, els: Span) -> Span {
        let src = self.src();
        let text = src.get(from as usize..els.start as usize).unwrap_or("");
        match text.rfind("else") {
            Some(i) => Span::new(from + i as u32, els.end),
            None => els,
        }
    }

    // ---- operation calls -----------------------------------------------------------------------

    /// `a op b`
    fn binary(&mut self, op: TokenKind, a: &ast::Expr, b: &ast::Expr, cst: Span, pos: Pos) -> Ex {
        let name = match op {
            TokenKind::NotEqualExeq => "<>",
            _ => op.spelling().unwrap_or(""),
        };
        let name_span = operator_span(&self.src(), a.span.end, b.span.start, name).unwrap_or(cst);
        let cs = CallCs {
            span: cst,
            name_span,
            source_span: Some(a.span),
            accessor: Accessor::None,
            module: None,
            mapping: None,
            pos,
        };
        if pos.warns_equality() && name == "=" {
            self.warning(
                cs.name_span,
                bind(
                    vm::QvtOperationalVisitorCS_possibleEqualityInsteadOfAssignment,
                    &[],
                ),
            );
        }
        let src = self.expr_d(a);
        let arg = self.expr_d(b);
        self.operation_call(&cs, name, Some(src), vec![arg])
    }

    /// `-a`, `not a`
    fn unary(&mut self, op: TokenKind, a: &ast::Expr, cst: Span, pos: Pos) -> Ex {
        let name = op.spelling().unwrap_or("");
        let name_span = Span::new(cst.start, cst.start + name.len() as u32);
        let cs = CallCs {
            span: cst,
            name_span,
            source_span: Some(a.span),
            accessor: Accessor::None,
            module: None,
            mapping: None,
            pos,
        };
        let src = self.expr_d(a);
        self.operation_call(&cs, name, Some(src), Vec::new())
    }

    /// `name(args)`, `src.name(args)`, `src->name(args)`, `M::name(args)`, `a::b::name(args)`.
    fn call_exp(&mut self, c: &ast::Call, cst: Span, pos: Pos) -> Ex {
        let accessor = match &c.source {
            Some((_, NavOp::Dot)) => Accessor::Dot,
            Some((_, NavOp::Arrow)) => Accessor::Arrow,
            None => Accessor::None,
        };
        let name = c.name.text.as_str();
        if let (Some((src, NavOp::Arrow)), [arg]) = (&c.source, c.args.as_slice()) {
            // the parser's isIterator: `src->select(cond)` is an iterator with an implicit variable
            if c.qualifier.is_empty() && OCL_ITERATORS.contains(&name) {
                let it = ast::Iterate {
                    source: src.clone(),
                    name: c.name.clone(),
                    vars: Vec::new(),
                    acc: None,
                    body: arg.clone(),
                };
                return self.iterate_exp(&it, cst);
            }
        }
        if c.qualifier.len() > 1 && c.source.is_none() {
            return self.static_operation_call(c, cst, pos);
        }
        let module = if c.qualifier.len() == 1 {
            Some(c.qualifier[0].clone())
        } else {
            None
        };
        let cs = CallCs {
            span: cst,
            name_span: c.name.span,
            source_span: c.source.as_ref().map(|(s, _)| s.span),
            accessor,
            module,
            mapping: None,
            pos,
        };
        if pos.warns_equality() && name == "=" {
            self.warning(
                cs.name_span,
                bind(
                    vm::QvtOperationalVisitorCS_possibleEqualityInsteadOfAssignment,
                    &[],
                ),
            );
        }
        let src = match &c.source {
            Some((s, NavOp::Arrow)) => Some(self.collection_source(s)),
            Some((s, NavOp::Dot)) => Some(self.expr_d(s)),
            None => None,
        };
        let args = self.arguments(&c.args);
        self.operation_call(&cs, name, src, args)
    }

    pub fn arguments(&mut self, args: &[ast::Expr]) -> Vec<Ex> {
        args.iter().map(|a| self.expr_d(a)).collect()
    }

    /// `getCollectionSourceExpression`: a non-collection source becomes `Set{source}`
    /// (classic OCL's implicit `->` conversion).
    pub fn collection_source(&mut self, s: &ast::Expr) -> Ex {
        let x = self.expr_d(s);
        self.as_collection_source(x, s.span)
    }

    pub fn as_collection_source(&mut self, x: Ex, span: Span) -> Ex {
        if self.ts.types.is_collection_like(x.ty()) {
            return x;
        }
        let ty = self.ts.types.collection(CollKind::Set, x.ty());
        let err = x.err;
        let call = ir::Call {
            callee: ir::Callee::Std(StdOp::OclAny_oclAsSet),
            source: Some(x.e),
            args: Vec::new(),
        };
        let mut r = self.ex(ExprKind::Call(Box::new(call)), ty, span);
        r.err = err;
        r
    }

    /// `staticOperationCallExpCS`: `a::b::op(args)`.
    fn static_operation_call(&mut self, c: &ast::Call, cst: Span, pos: Pos) -> Ex {
        let cs = CallCs {
            span: cst,
            name_span: c.name.span,
            source_span: None,
            accessor: Accessor::None,
            module: None,
            mapping: None,
            pos,
        };
        if pos.warns_equality() && c.name.text == "=" {
            self.warning(
                cs.name_span,
                bind(
                    vm::QvtOperationalVisitorCS_possibleEqualityInsteadOfAssignment,
                    &[],
                ),
            );
        }
        let names: Vec<&str> = c.qualifier.iter().map(|n| n.text.as_str()).collect();
        let pspan = path_span_of(&c.qualifier);
        let src = match self.lookup_classifier(&names) {
            None => {
                let msg = self.unrecognized_type_message(&names);
                self.error(cst, msg);
                None
            }
            Some(t) => Some(self.type_exp(t, pspan)),
        };
        let args = self.arguments(&c.args);
        let owner = src.as_ref().map(|s| s.ty()).unwrap_or(TypeId::NULL);
        let (mut r, op) = self.gen_operation_call(&cs, &c.name.text, src, owner, args);
        if matches!(self.ts.types.get(owner), Type::TypeType(_)) && !self.is_static_ok(op) {
            self.error(cst, bind(om::NonStaticOperation_ERROR_, &[&c.name.text]));
        }
        r.info = Info::OpCall(op, None);
        self.post_operation_call(&cs, r, op, None, true)
    }

    fn is_static_ok(&self, op: Option<OpRef>) -> bool {
        match op {
            Some(OpRef::Std(_, StdOwner::OclType)) => true,
            Some(o) => self.op_is_static(o),
            None => false,
        }
    }

    /// Classic OCL's `operationCallExpCS` (dot and arrow forms, implicit source, implicit
    /// collect) followed by QVTo's checks, for a call whose source (if any) and arguments are
    /// analysed.
    pub(crate) fn operation_call(
        &mut self,
        cs: &CallCs,
        name: &str,
        src: Option<Ex>,
        args: Vec<Ex>,
    ) -> Ex {
        let had_source = src.is_some();
        let src = match src {
            Some(s) => s,
            None => self.implicit_source_for_call(cs, name, &args),
        };
        let src_err = src.err;
        let st = src.ty();
        let src_var = if let Info::Var(v) = src.info { v } else { None };
        let implicit_collect = cs.accessor == Accessor::Dot && self.ts.types.is_collection_like(st);
        let owner = if implicit_collect {
            self.ts.types.element(st).unwrap_or(TypeId::OCL_VOID)
        } else {
            st
        };
        let is_static = matches!(self.ts.types.get(owner), Type::TypeType(_));
        if implicit_collect {
            let (it, it_name) = self.new_temp_iterator(st);
            let iter_ex = self.var_ex(VarRef::Local(it), cs.span);
            // Eclipse generates the call on the collection source, then substitutes the iterator
            self.call_msg_src = Some(st);
            let (mut call, op) = self.gen_operation_call(cs, name, Some(iter_ex), owner, args);
            if is_static && !self.is_static_ok(op) {
                self.error(cs.span, bind(om::NonStaticOperation_ERROR_, &[name]));
            }
            call.info = Info::OpCall(op, Some(VarRef::Local(it)));
            self.delete_var(&it_name);
            let probe = call.clone();
            let mut r = self.implicit_collect(src, it, call, cs.span);
            r.err |= src_err;
            // oclExpressionCS: validateOperationCall on the collect's body
            if let Some(o) = op {
                self.validate_operation_call(cs, &probe, o, Some(VarRef::Local(it)));
            }
            return r;
        }
        let (mut r, op) = self.gen_operation_call(cs, name, Some(src), owner, args);
        if is_static && !self.is_static_ok(op) {
            self.error(cs.span, bind(om::NonStaticOperation_ERROR_, &[name]));
        }
        r.info = Info::OpCall(op, src_var);
        r.err |= src_err;
        self.post_operation_call(cs, r, op, src_var, had_source)
    }

    /// QVTo's checks after classic OCL's `operationCallExpCS` returned an `OperationCallExp`,
    /// then `validateOperationCall`.
    fn post_operation_call(
        &mut self,
        cs: &CallCs,
        mut result: Ex,
        op: Option<OpRef>,
        src_var: Option<VarRef>,
        had_source: bool,
    ) -> Ex {
        if let Some(OpRef::Std(sop, _)) = op {
            let d = sop.desc();
            if d.deprecated {
                let msg = match d.deprecated_by {
                    Some(by) => bind(vm::DeprecatedElementBy, &[d.name, by]),
                    None => bind(vm::DeprecatedElement, &[d.name]),
                };
                self.warning(cs.span, msg);
            }
            if let Some(reason) = d.unsupported {
                let m = if reason.is_empty() {
                    vm::UnsupportedElement
                } else {
                    reason
                };
                self.error(cs.span, bind(m, &[d.name]));
            }
        }
        let mut rewritten = false;
        if let Some(o) = op
            && cs.accessor == Accessor::Arrow
        {
            let st = match &result.e.kind {
                ExprKind::Call(c) => c.source.as_ref().map(|s| s.ty),
                _ => None,
            };
            if let Some(st) = st
                && self.ts.types.is_collection_like(st)
            {
                let src_ops = self.get_operations(st);
                if !src_ops.contains(&o) {
                    result = self.rewrite_as_xcollect(result, cs);
                    rewritten = true;
                }
            }
        }
        if !had_source {
            self.validate_implicit_source(cs.name_span, src_var, op, None);
        }
        if !rewritten && let Some(o) = op {
            self.validate_operation_call(cs, &result, o, src_var);
        }
        result
    }

    /// Turns `src->op(args)` whose operation is not a collection operation into
    /// `src->xcollect(e | e.op(args))` (`createImplicitXCollect`).
    fn rewrite_as_xcollect(&mut self, call: Ex, cs: &CallCs) -> Ex {
        let Ex { e, err, info, .. } = call;
        let ir::Expr { kind, ty, span } = e;
        let ExprKind::Call(c) = kind else {
            return Ex {
                e: mk(kind, ty, span),
                err,
                info,
                det: false,
            };
        };
        let mut c = *c;
        let Some(src) = c.source.take() else {
            return self.ex(ExprKind::Call(Box::new(c)), ty, span);
        };
        let src_ex = Ex {
            e: src,
            err,
            info: Info::Other,
            det: false,
        };
        let (it, it_name) = self.new_temp_iterator(src_ex.ty());
        let ity = self.fun.locals[it.index()].ty;
        c.source = Some(mk(ExprKind::Local(it), ity, cs.span));
        self.delete_var(&it_name);
        let body = Ex {
            e: mk(ExprKind::Call(Box::new(c)), ty, span),
            err: false,
            info,
            det: false,
        };
        self.implicit_xcollect(src_ex, it, body, cs.span)
    }

    /// `createImplicitSource` with QVTo's `lookupImplicitSourceForOperation`: an ambiguity is a
    /// warning; no source gives a variable of no type (and the lookup then fails).
    fn implicit_source_for_call(&mut self, cs: &CallCs, name: &str, args: &[Ex]) -> Ex {
        let a: Vec<Arg> = args.iter().map(|x| self.arg_of(x)).collect();
        let v = match self.implicit_source_for_operation(name, &a) {
            Ok(v) => v,
            Err(crate::vars::AmbiguousSource(list)) => {
                let names: Vec<String> = list
                    .iter()
                    .map(|v| match v {
                        VarRef::This(m) => format!("'{}::this'", self.modules[m.index()].name),
                        other => format!("'{}'", self.var_name(*other)),
                    })
                    .collect();
                self.warning(
                    cs.name_span,
                    bind(vm::AmbiguousImplicitSourceReference, &[&names.join(", ")]),
                );
                list.first().copied()
            }
        };
        match v {
            Some(v) => self.var_ex(v, cs.span),
            None => {
                let mut x = self.ex(ExprKind::Null, TypeId::NULL, cs.span);
                x.info = Info::Var(None);
                x
            }
        }
    }

    pub fn arg_of(&self, x: &Ex) -> Arg {
        Arg {
            ty: x.ty(),
            type_exp: matches!(x.info, Info::TypeExp(_)),
            referred: if let Info::TypeExp(t) = x.info {
                Some(t)
            } else {
                None
            },
        }
    }

    /// QVTo's `lookupOperation`: module-qualified lookups, ambiguity as a warning.
    pub(crate) fn lookup_operation(
        &mut self,
        problem: Span,
        module: Option<&ast::Name>,
        owner: TypeId,
        name: &str,
        args: &[Arg],
    ) -> Option<OpRef> {
        // an unknown (null) owner: `AbstractTypeChecker.getOperations` treats it as a user
        // type without operations of its own, so OclAny's operations are found
        if let Some(mq) = module {
            return self.qualified_operation_lookup(mq, owner, name, args);
        }
        let r = match self.find_most_specific_operation(owner, name, args) {
            Ok(None) => match name.strip_prefix('_') {
                Some(u) => self.find_most_specific_operation(owner, u, args),
                None => Ok(None),
            },
            other => other,
        };
        match r {
            Ok(o) => o,
            Err(matches) => self.handle_ambiguous_operations(problem, matches),
        }
    }

    /// `handleOperationLookupException`
    fn handle_ambiguous_operations(&mut self, problem: Span, matches: Vec<OpRef>) -> Option<OpRef> {
        let filtered: Vec<OpRef> = matches
            .iter()
            .copied()
            .filter(|&o| {
                let r = self.op_ret(o);
                if self.is_generic_t2(r) {
                    return false;
                }
                !self.op_params(o).iter().any(|p| self.is_generic_t2(*p))
            })
            .collect();
        if filtered.is_empty() {
            return matches.first().copied();
        }
        if filtered.len() == 1 {
            return Some(filtered[0]);
        }
        let mut parts = Vec::new();
        for o in &filtered {
            let s = self.format_op_signature(*o);
            parts.push(format!("'{s}'"));
        }
        self.warning(
            problem,
            bind(vm::AmbiguousOperationReference, &[&parts.join(", ")]),
        );
        Some(filtered[0])
    }

    fn is_generic_t2(&self, t: TypeId) -> bool {
        match *self.ts.types.get(t) {
            Type::Collection(_, e) | Type::List(e) | Type::Dict(_, e) => self.is_generic_t2(e),
            _ => t == TypeId::T2,
        }
    }

    /// `qualifiedOperationLookup`: `M::op(...)` in the module `M` of the extension hierarchy or
    /// an accessed library.
    fn qualified_operation_lookup(
        &mut self,
        mq: &ast::Name,
        owner: TypeId,
        name: &str,
        args: &[Arg],
    ) -> Option<OpRef> {
        let cur = self.cur_module;
        let target = mq.text.as_str();
        let envs: Vec<ir::ModuleId> = if self.modules[cur.index()].name == target {
            vec![cur]
        } else {
            let mut v = self.all_extended(cur);
            for a in self.modules[cur.index()].by_access.clone() {
                if self.modules[a.index()].is_library {
                    v.push(a);
                }
            }
            v
        };
        let mut resolved = None;
        for e in envs {
            if self.modules[e.index()].name != target {
                continue;
            }
            let actual_owner = if matches!(self.ts.types.get(owner), Type::Module(_)) {
                self.modules[e.index()].ty
            } else {
                owner
            };
            let saved = self.cur_module;
            self.cur_module = e;
            let mut r = self.find_most_specific_operation(actual_owner, name, args);
            if matches!(r, Ok(None))
                && let Some(u) = name.strip_prefix('_')
            {
                r = self.find_most_specific_operation(actual_owner, u, args);
            }
            self.cur_module = saved;
            match r {
                Ok(Some(o)) => {
                    if self.op_module(o) == Some(e) {
                        resolved = Some(o);
                    }
                }
                Ok(None) => {}
                Err(matches) => {
                    let mut op: Option<OpRef> = None;
                    let mut done = false;
                    for m in matches {
                        if self.op_module(m) == Some(e) {
                            if let Some(prev) = op {
                                resolved = self.handle_ambiguous_operations(mq.span, vec![prev, m]);
                                done = true;
                                break;
                            }
                            op = Some(m);
                        }
                    }
                    if !done {
                        resolved = op;
                    }
                }
            }
            if resolved.is_some() {
                break;
            }
        }
        resolved
    }

    /// QVTo's `genOperationCallExp`: the operation, the IR call and its type.
    pub(crate) fn gen_operation_call(
        &mut self,
        cs: &CallCs,
        name: &str,
        src: Option<Ex>,
        owner: TypeId,
        args: Vec<Ex>,
    ) -> (Ex, Option<OpRef>) {
        let a: Vec<Arg> = args.iter().map(|x| self.arg_of(x)).collect();
        let mut op_owner = owner;
        let src_ty = self.call_msg_src.take().or(src.as_ref().map(|s| s.ty()));
        let arrow_coll = cs.accessor == Accessor::Arrow
            && src_ty.is_some_and(|t| self.ts.types.is_collection_like(t));
        if arrow_coll
            && self
                .lookup_operation(cs.span, None, owner, name, &a)
                .is_none()
        {
            op_owner = self.ts.types.element(owner).unwrap_or(TypeId::OCL_VOID);
        }
        let op = self.lookup_operation(cs.name_span, cs.module.as_ref(), op_owner, name, &a);
        let mut ty = TypeId::OCL_VOID;
        match op {
            None => {
                let mut s = format!("{name}(");
                for (i, x) in a.iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    let n = if x.ty == TypeId::NULL {
                        "null".to_string()
                    } else {
                        self.type_name(x.ty)
                    };
                    s.push_str(&n);
                }
                s.push(')');
                let on = self.type_name(op_owner);
                let msg = if src_ty.unwrap_or(TypeId::NULL) == op_owner {
                    bind(
                        vm::QvtOperationalVisitorCS_OperationNotFound_ERROR_,
                        &[&s, &on],
                    )
                } else {
                    let sn = src_ty
                        .map(|t| self.type_name(t))
                        .unwrap_or_else(|| "null".into());
                    bind(
                        vm::QvtOperationalVisitorCS_OperationNotFoundForCollectionAndItsElement,
                        &[&s, &sn, &on],
                    )
                };
                self.error(cs.span, msg);
            }
            Some(o) => {
                // a null result type (e.g. the element type of a collection of unknown
                // elements) falls back to the operation's own type (`genOperationCallExp`)
                ty = match self.result_type(cs.span, op_owner, o, &a) {
                    Some(t) if t != TypeId::NULL => t,
                    _ => self.op_ret(o),
                };
                ty = self.ts.ck().ocl_type(ty);
            }
        }
        let callee = match op {
            Some(OpRef::Std(s, _)) => ir::Callee::Std(s),
            Some(OpRef::User(id)) => {
                let virtual_ = cs.module.is_none();
                if self.ops[id.index()].ir.kind == ir::OpKind::Mapping {
                    ir::Callee::Mapping {
                        op: id,
                        strict: cs.mapping == Some(true),
                        virtual_,
                    }
                } else {
                    ir::Callee::Op { op: id, virtual_ }
                }
            }
            Some(OpRef::Meta(m)) => ir::Callee::Meta(m),
            None => ir::Callee::Std(StdOp::OclAny_oclIsInvalid),
        };
        let err = src.as_ref().is_some_and(|s| s.err);
        let call = ir::Call {
            callee,
            source: src.map(|s| s.e),
            args: args.into_iter().map(|x| x.e).collect(),
        };
        let mut x = self.ex(ExprKind::Call(Box::new(call)), ty, cs.span);
        x.err = err;
        (x, op)
    }

    /// `TypeCheckerImpl.getResultType`: QVTo operations through the generics resolver, classic
    /// OCL's by operation code.
    pub(crate) fn result_type(
        &mut self,
        problem: Span,
        owner: TypeId,
        o: OpRef,
        args: &[Arg],
    ) -> Option<TypeId> {
        let is_ocl = matches!(o, OpRef::Std(s, _) if s.desc().section == Section::Ocl);
        if !is_ocl {
            if self.op_module(o).is_some() {
                return Some(self.generic_return_type(problem, owner, o, args));
            }
            return None;
        }
        if !crate::ocltypes::has_opcode(self.op_name(o)) {
            return None;
        }
        let r = self.ocl_result_type(problem, owner, o, args);
        if r.is_none() {
            let actual = self.op_owner(o);
            if actual != owner {
                return self.ocl_result_type(problem, actual, o, args);
            }
        }
        r
    }

    /// `GenericsResolver.resolveOperationReturnType`
    fn generic_return_type(
        &mut self,
        problem: Span,
        source: TypeId,
        o: OpRef,
        args: &[Arg],
    ) -> TypeId {
        let mut bind_map: Vec<(TypeId, TypeId)> = Vec::new();
        let ret = self.op_ret(o);
        if is_generic_param(ret) {
            let actual = match *self.ts.types.get(source) {
                Type::TypeType(r) => r,
                _ => source,
            };
            if actual != TypeId::NULL {
                set_binding(&mut bind_map, ret, actual);
            }
        }
        let owner = self.op_owner(o);
        self.generic_resolve(owner, source, &mut bind_map);
        let params = self.op_params(o);
        for (i, p) in params.iter().enumerate() {
            if let Some(a) = args.get(i) {
                self.generic_resolve(*p, a.ty, &mut bind_map);
            }
        }
        if bind_map.is_empty() {
            return ret;
        }
        self.generic_bind(problem, ret, &bind_map)
    }

    fn generic_resolve(&mut self, formal: TypeId, actual: TypeId, m: &mut Vec<(TypeId, TypeId)>) {
        if formal == TypeId::NULL || actual == TypeId::NULL {
            return;
        }
        let fty = self.ts.types.get(formal).clone();
        let aty = self.ts.types.get(actual).clone();
        let fcoll = self.ts.types.is_collection_like(formal);
        let acoll = self.ts.types.is_collection_like(actual);
        if fcoll && acoll {
            let fe = self.ts.types.element(formal).unwrap();
            let ae = self.ts.types.element(actual).unwrap();
            self.generic_resolve(fe, ae, m);
            if let (Type::Dict(fk, _), Type::Dict(ak, _)) = (fty, aty) {
                self.generic_resolve(fk, ak, m);
            }
        } else if let (Type::TypeType(_), Type::TypeType(ar)) = (&fty, &aty) {
            if formal == TypeId::OCL_TYPE {
                self.generic_resolve(TypeId::T, *ar, m);
            }
        } else if is_generic_param(formal) {
            set_binding(m, formal, actual);
            if !m.iter().any(|(k, _)| *k == TypeId::T2) {
                let v = if acoll {
                    self.flattened_element(actual)
                } else {
                    actual
                };
                set_binding(m, TypeId::T2, v);
            }
        }
    }

    /// `CollectionUtil.getFlattenedElementType`
    pub fn flattened_element(&self, t: TypeId) -> TypeId {
        // `List` and `Dict` are collection types too (their values are flattened)
        let mut e = self.ts.types.element(t).unwrap_or(t);
        while matches!(self.ts.types.get(e), Type::Collection(..) | Type::List(_) | Type::Dict(..)) {
            let Some(x) = self.ts.types.element(e) else { break };
            e = x;
        }
        e
    }

    fn generic_bind(&mut self, problem: Span, t: TypeId, m: &[(TypeId, TypeId)]) -> TypeId {
        match self.ts.types.get(t).clone() {
            Type::Collection(k, e) => {
                let be = self.bound(problem, e, m);
                self.ts.types.collection(k, be)
            }
            Type::List(e) => {
                let be = self.bound(problem, e, m);
                self.ts.types.list(be)
            }
            Type::Dict(k, v) => {
                let bk = self.bound(problem, k, m);
                let bv = self.bound(problem, v, m);
                self.ts.types.dict(bk, bv)
            }
            Type::TypeType(r) => {
                let rt = if t == TypeId::OCL_TYPE {
                    self.bound(problem, TypeId::T, m)
                } else {
                    self.bound(problem, r, m)
                };
                self.ts.types.type_type(rt)
            }
            _ => self.bound(problem, t, m),
        }
    }

    fn bound(&mut self, problem: Span, x: TypeId, m: &[(TypeId, TypeId)]) -> TypeId {
        if x == TypeId::COMMON_T {
            let a = m.iter().find(|(k, _)| *k == TypeId::T).map(|(_, v)| *v);
            let b = m.iter().find(|(k, _)| *k == TypeId::T2).map(|(_, v)| *v);
            if let (Some(a), Some(b)) = (a, b) {
                // `GenericsResolver`'s problem object is the operation: no position
                let _ = problem;
                return self
                    .common_super_type_at(crate::diag::NO_POSITION, a, b)
                    .unwrap_or(TypeId::OCL_ANY);
            }
        }
        m.iter()
            .find(|(k, _)| *k == x)
            .map(|(_, v)| *v)
            .unwrap_or(x)
    }

    // ---- navigation ----------------------------------------------------------------------------

    /// `src.name` (`src->name` is the `xcollect` shorthand).
    fn nav_exp(&mut self, n: &ast::Nav, cst: Span) -> Ex {
        if n.op == NavOp::Arrow {
            return self.xcollect_shorthand(n, cst);
        }
        let src = self.expr_d(&n.source);
        self.simple_name(&n.name.text, n.name.span, Some(src), cst)
    }

    // ---- validation of calls -------------------------------------------------------------------

    /// `validateOperationCall`
    pub(crate) fn validate_operation_call(
        &mut self,
        cs: &CallCs,
        call: &Ex,
        o: OpRef,
        src_var: Option<VarRef>,
    ) {
        let (src_ty, arg_tys): (Option<TypeId>, Vec<TypeId>) = match &call.e.kind {
            ExprKind::Call(c) => (
                c.source.as_ref().map(|s| s.ty),
                c.args.iter().map(|a| a.ty).collect(),
            ),
            _ => (None, Vec::new()),
        };
        let name = self.op_name(o).to_string();
        if name == "oclAsType"
            && let (Some(st), [at]) = (src_ty, arg_tys.as_slice())
            && let Type::TypeType(target) = *self.ts.types.get(*at)
            && self.incorrect_cast(st, target)
        {
            self.warning(cs.span, vm::incorrectCastWarning);
        }
        let cs_name = cs.name_span.text(&self.src()).to_string();
        if (cs_name == "=" || cs_name == "<>" || cs_name == "!=")
            && let (Some(st), [at]) = (src_ty, arg_tys.as_slice())
        {
            let at = *at;
            if !self.ts.ck().assignable(st, at) && !self.ts.ck().assignable(at, st) {
                let a = self.full_name(at);
                let b = self.full_name(st);
                self.warning(cs.span, bind(vm::TypesDoNotConform, &[&a, &b]));
            }
        }
        if self.op_is_mapping(o) && cs.mapping.is_none() {
            self.warning(
                cs.span,
                bind(vm::QvtOperationalVisitorCS_mapKeywordNotUsed, &[&name]),
            );
        }
        if let OpRef::Std(s, _) = o
            && is_mutator(s)
            && let Some(v) = src_var
        {
            let span = cs.source_span.unwrap_or(cs.span);
            self.validate_variable_modification(v, span, None, false);
        }
    }

    /// `QvtOperationalParserUtil.isIncorrectCast`
    fn incorrect_cast(&self, s: TypeId, t: TypeId) -> bool {
        match (self.ts.types.get(s), self.ts.types.get(t)) {
            (Type::Real, tt) => matches!(tt, Type::Integer),
            (Type::Collection(CollKind::Set, _), tt) => {
                !matches!(tt, Type::Collection(CollKind::Set, _))
            }
            (Type::Collection(CollKind::Bag, _), tt) => {
                !matches!(tt, Type::Collection(CollKind::Bag, _))
            }
            (Type::Collection(CollKind::Sequence, _), tt) => {
                !matches!(tt, Type::Collection(CollKind::Sequence, _))
            }
            (Type::Collection(CollKind::OrderedSet, _), tt) => !matches!(
                tt,
                Type::Collection(CollKind::OrderedSet | CollKind::Set, _)
            ),
            _ => false,
        }
    }

    /// `DeprecatedImplicitSourceCallHelper.validateCallExp` for a call on an implicit source.
    pub(crate) fn validate_implicit_source(
        &mut self,
        cause: Span,
        src_var: Option<VarRef>,
        op: Option<OpRef>,
        prop: Option<PropRef>,
    ) {
        let Some(v) = src_var else {
            return;
        };
        if self.fun.ocl_annotation {
            return;
        }
        let vn = self.var_name(v);
        if vn == "self" {
            if self.fun.object_self_depth == 0 {
                self.error(
                    cause,
                    vm::DeprecatedImplicitSourceCall_contextualImplicitCall,
                );
            }
        } else if vn == "this" {
            let owner = match (op, prop) {
                (Some(o), _) => Some(self.op_owner(o)),
                (None, Some(p)) => Some(self.prop_owner(p)),
                _ => None,
            };
            if let Some(owner) = owner
                && !matches!(self.ts.types.get(owner), Type::Module(_))
            {
                self.warning(
                    cause,
                    vm::DeprecatedImplicitSourceCall_moduleScopeImplicitCall,
                );
            }
        }
    }
}

/// `AbstractOCLParser.createIteratorNames`
const OCL_ITERATORS: [&str; 11] = [
    "any",
    "collect",
    "collectNested",
    "exists",
    "forAll",
    "isUnique",
    "one",
    "reject",
    "select",
    "sortedBy",
    "closure",
];

fn is_generic_param(t: TypeId) -> bool {
    t == TypeId::T || t == TypeId::T2 || t == TypeId::KEY_T
}

fn set_binding(m: &mut Vec<(TypeId, TypeId)>, k: TypeId, v: TypeId) {
    if let Some(e) = m.iter_mut().find(|(a, _)| *a == k) {
        e.1 = v;
    } else {
        m.push((k, v));
    }
}

pub(crate) fn path_span_of(names: &[ast::Name]) -> Span {
    match (names.first(), names.last()) {
        (Some(a), Some(b)) => a.span.to(b.span),
        _ => Span::default(),
    }
}

/// The span of an infix operator between two operands.
fn operator_span(src: &str, from: u32, to: u32, name: &str) -> Option<Span> {
    let between = src.get(from as usize..to as usize)?;
    let spelled = if name == "<>" && !between.contains("<>") {
        "!="
    } else {
        name
    };
    let i = between.find(spelled)?;
    let start = from + i as u32;
    Some(Span::new(start, start + spelled.len() as u32))
}

/// Java `Double.valueOf` for a QVTo real literal.
fn parse_real(t: &str) -> f64 {
    t.trim_end_matches(['d', 'D', 'f', 'F'])
        .parse()
        .unwrap_or(0.0)
}

fn is_mutator(s: StdOp) -> bool {
    matches!(
        s,
        StdOp::QList_add
            | StdOp::QList_insertAt
            | StdOp::QList_remove
            | StdOp::QList_removeAll
            | StdOp::QList_removeAt
            | StdOp::QList_removeFirst
            | StdOp::QList_removeLast
            | StdOp::QDict_put
            | StdOp::QDict_clear
    )
}

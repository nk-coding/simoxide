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
use qvto_types::rel::*;
use qvto_types::{CollKind, Section, StdOp, StdOwner, Type, TypeId};

/// What Eclipse's AST node is, for the checks that test node classes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Info {
    #[default]
    Other,
    /// `TypeExp` referring to a type.
    TypeExp(TypeId),
    /// `VariableExp`
    Var(VarRef),
    /// `PropertyCallExp`: the property and its source variable (if the source is a
    /// `VariableExp`).
    Prop(PropRef, Option<VarRef>),
    /// `OperationCallExp`, its operation and source variable.
    OpCall(Option<OpRef>, Option<VarRef>),
    /// `IteratorExp` with an operation call body.
    Iterator,
    ImperativeIterate,
    /// `InvalidLiteralExp`
    Invalid,
    /// `ResolveExp` (`late`)
    Resolve { late: bool },
    /// `StringLiteralExp`
    StringLit,
    Other2,
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
    /// The last expression of a query body.
    QueryLast,
}

pub(crate) fn mk(kind: ExprKind, ty: TypeId, span: Span) -> ir::Expr {
    ir::Expr { kind, ty, span }
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
            info: Info::Var(v),
            det: false,
        }
    }

    /// `QvtOperationalVisitorCS.oclExpressionCS`; `None` where Eclipse gives null.
    pub fn expr(&mut self, e: &ast::Expr) -> Option<Ex> {
        self.expr_at(e, Pos::Other, e.span)
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
            K::Int(v) => Some(self.ex(ExprKind::Int(*v), TypeId::INTEGER, cst)),
            K::Real(t) => {
                let v = crate::literal::parse_real(t);
                Some(self.ex(ExprKind::Real(v), TypeId::REAL, cst))
            }
            K::Str(s) => {
                let mut x = self.ex(ExprKind::Str(s.value.clone().into()), TypeId::STRING, cst);
                x.info = Info::StringLit;
                Some(x)
            }
            K::Bool(b) => Some(self.ex(ExprKind::Bool(*b), TypeId::BOOLEAN, cst)),
            K::Null => Some(self.ex(ExprKind::Null, TypeId::OCL_VOID, cst)),
            K::Invalid => {
                let mut x = self.ex(ExprKind::Invalid, TypeId::OCL_INVALID, cst);
                x.info = Info::Invalid;
                Some(x)
            }
            K::Unlimited => Some(self.ex(ExprKind::Unlimited, TypeId::UNLIMITED_NATURAL, cst)),
            K::SelfExp => Some(self.variable_exp("self", cst, cst)),
            K::Name(n) => Some(self.variable_exp(&n.text, n.span, cst)),
            K::Path(p) => Some(self.path_exp(p, cst)),
            K::TypeLit(t) => Some(self.type_lit(t, cst)),
            K::CollectionLit(k, parts) => Some(self.collection_literal(*k, parts, cst)),
            K::ListLit(parts) => Some(self.list_literal(parts, cst)),
            K::DictLit(parts) => Some(self.dict_literal(parts, cst)),
            K::TupleLit(parts) => Some(self.tuple_literal(parts, cst)),
            K::Unary(op, a) => Some(self.unary(*op, a, cst)),
            K::Binary(op, a, b) => Some(self.binary(*op, a, b, cst, pos)),
            K::Let(decls, body) => Some(self.let_exp(decls, body, cst)),
            K::If(i) => Some(self.if_exp(i, cst, pos)),
            K::Call(c) => Some(self.call_exp(c, cst, pos)),
            K::Nav(n) => Some(self.nav_exp(n, cst)),
            K::Iterate(it) => Some(self.iterate_exp(it, cst)),
            K::XIterate(x) => Some(self.ximperative_iterate(x, cst)),
            K::XSelect(x) => Some(self.xselect_shorthand(x, cst)),
            K::ForEach(f) => Some(self.for_exp(f, cst)),
            K::ObjectIter(o) => Some(self.object_iter(o, cst)),
            K::SwitchIter(s) => Some(self.switch_iter(s, cst)),
            K::Assign(a) => return self.assign_statement(a, cst),
            K::Var(v) => Some(self.var_statement(v, cst)),
            K::Return(v) => Some(self.return_exp(v.as_deref(), cst)),
            K::Break => Some(self.ex(ExprKind::Break, TypeId::OCL_VOID, cst)),
            K::Continue => Some(self.ex(ExprKind::Continue, TypeId::OCL_VOID, cst)),
            K::While(w) => Some(self.while_exp(w, cst)),
            K::Block(b) => Some(self.block_exp(b, cst, false)),
            K::Do(b) => Some(self.block_exp(b, cst, false)),
            K::Switch(s) => Some(self.switch_exp(s, cst)),
            K::Log(l) => Some(self.log_exp(l, cst)),
            K::Assert(a) => Some(self.assert_exp(a, cst)),
            K::Compute(c) => Some(self.compute_exp(c, cst)),
            K::New(n) => Some(self.new_exp(n, cst)),
            K::Object(o) => Some(self.object_exp(o, cst, true)),
            K::MapCall(m) => Some(self.map_call(m, cst)),
            K::Resolve(r) => Some(self.resolve_exp(r, cst)),
            K::Raise(r) => Some(self.raise_exp(r, cst)),
            K::Try(t) => Some(self.try_exp(t, cst)),
            K::Error => Some(self.dummy(cst)),
        };
        r
    }

    /// `visitOclExpressionCS`: statement-level analysis.
    pub fn visit_ocl_expression(&mut self, e: &ast::Expr) -> Option<Ex> {
        self.visit_ocl_expression_at(e, Pos::Other)
    }

    pub fn visit_ocl_expression_at(&mut self, e: &ast::Expr, pos: Pos) -> Option<Ex> {
        let r = self.expr_at(e, pos, e.span);
        if let ast::ExprKind::MapCall(_) = &e.kind {
            // the analyser of map calls reports a missing mapping itself
            return r;
        }
        r
    }

    // ---- names ---------------------------------------------------------------------------------

    /// `variableExpCS` with QVTo's `customSimpleNameCS` (no source).
    fn variable_exp(&mut self, name: &str, name_span: Span, cst: Span) -> Ex {
        let r = self.simple_name(name, name_span, None, cst, false);
        if let Info::Prop(_, src) = r.info {
            self.validate_implicit_source_call(cst, None, src, ImplicitKind::Prop(r.info), true);
        }
        r
    }

    /// `customSimpleNameCS`: type, variable, property, then the error.
    fn simple_name(&mut self, name: &str, name_span: Span, source: Option<Ex>, cst: Span, arrow: bool) -> Ex {
        if let Some(s) = &source {
            if s.err {
                return source.unwrap();
            }
        }
        // a type
        if source.is_none() {
            if let Some(t) = self.lookup_classifier(&[name]) {
                let tt = self.ts.types.type_type(t);
                let mut x = self.ex(ExprKind::Type(t), tt, name_span);
                x.info = Info::TypeExp(t);
                return x;
            }
            // a variable
            if let Some(v) = self.lookup_var(name) {
                return self.var_ex(v, name_span);
            }
        }
        // a property
        let source_elem_owner = source.as_ref().map(|s| {
            let t = s.ty();
            self.ts.types.element(t).filter(|_| self.ts.ck().is_collection_like(t)).unwrap_or(t)
        });
        let (src, owner, implicit_var) = match source {
            Some(s) => {
                let o = source_elem_owner.unwrap();
                (Some(s), Some(o), None)
            }
            None => match self.implicit_source_for_property(name) {
                Some(v) => {
                    let t = self.var_type(v);
                    (Some(self.var_ex(v, name_span)), Some(t), Some(v))
                }
                None => (None, None, None),
            },
        };
        let prop = owner.and_then(|o| self.lookup_property(o, name));
        if let (Some(p), Some(o), Some(src)) = (prop, owner, src.clone()) {
            let pty = self.property_type(o, p);
            let src_var = if let Info::Var(v) = src.info { Some(v) } else { implicit_var };
            let src_is_coll = self.ts.ck().is_collection_like(src.ty());
            if src_is_coll {
                // implicit collect / xcollect over the source collection
                let it = self.new_temp_iterator(src.ty());
                let iter_ex = self.var_ex(VarRef::Local(it.0), name_span);
                let body = self.property_call(p, iter_ex, pty, name_span);
                self.delete_var(&it.1);
                let mut r = if arrow {
                    self.implicit_xcollect(src, it.0, body, cst)
                } else {
                    self.implicit_collect(src, it.0, body, cst)
                };
                r.info = Info::Iterator;
                return r;
            }
            let mut r = self.property_call(p, src, pty, if implicit_var.is_some() { name_span } else { cst });
            r.info = Info::Prop(p, src_var);
            return r;
        }
        // undefined
        if source_elem_owner.is_some() && self.lookup_var(name).is_some() {
            self.error(name_span, bind(om::VarInNavExp_ERROR_, &[name]));
        } else {
            self.error(name_span, bind(om::UnrecognizedVar_ERROR_, &[name]));
        }
        self.dummy(name_span)
    }

    /// `TypeUtil.getPropertyType`
    pub fn property_type(&mut self, _owner: TypeId, p: PropRef) -> TypeId {
        self.prop_type(p)
    }

    /// The IR of a property call on a single source.
    fn property_call(&mut self, p: PropRef, src: Ex, ty: TypeId, span: Span) -> Ex {
        let kind = match p {
            PropRef::Feature(f) => ExprKind::Prop(Box::new(src.e), f),
            PropRef::Global(g) => ExprKind::Global(g),
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

    /// A generated implicit iterator variable over the elements of a collection type.
    pub fn new_temp_iterator(&mut self, coll: TypeId) -> (ir::LocalId, String) {
        let elem = self.ts.types.element(coll).unwrap_or(TypeId::OCL_VOID);
        let l = self.fun.new_local("", elem, ir::LocalKind::Temp, Span::default());
        let name = self.add_var_generated(VarRef::Local(l));
        self.fun.locals[l.index()].name = name.clone();
        (l, name)
    }

    /// `createImplicitCollect` (with QVTo's List rule).
    pub fn implicit_collect(&mut self, src: Ex, it: ir::LocalId, body: Ex, cst: Span) -> Ex {
        let mut bt = body.ty();
        while let Type::Collection(_, e) | Type::List(e) = *self.ts.types.get(bt) {
            bt = e;
            if !matches!(self.ts.types.get(bt), Type::Collection(..)) {
                break;
            }
        }
        let st = src.ty();
        let seq = matches!(self.ts.types.get(st), Type::Collection(CollKind::Sequence | CollKind::OrderedSet, _));
        let mut ty = self.ts.types.collection(if seq { CollKind::Sequence } else { CollKind::Bag }, bt);
        if matches!(self.ts.types.get(st), Type::List(_)) {
            ty = self.ts.types.collection(CollKind::Sequence, bt);
        }
        let err = src.err || body.err;
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
        x
    }

    /// `createImplicitXCollect`
    pub fn implicit_xcollect(&mut self, src: Ex, it: ir::LocalId, body: Ex, cst: Span) -> Ex {
        let st = src.ty();
        let seq = matches!(self.ts.types.get(st), Type::Collection(CollKind::Sequence | CollKind::OrderedSet, _) | Type::List(_));
        let ty = self.ts.types.collection(if seq { CollKind::Sequence } else { CollKind::Bag }, body.ty());
        let err = src.err;
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
        x.err = err;
        x.info = Info::ImperativeIterate;
        x
    }

    /// `a::b::c` as an expression: an enumeration literal, a static property or a type.
    fn path_exp(&mut self, p: &ast::PathName, cst: Span) -> Ex {
        let n = p.segments.len();
        let names: Vec<&str> = p.segments.iter().map(|s| s.text.as_str()).collect();
        let last = names[n - 1];
        let path = &names[..n - 1];
        match self.lookup_classifier(path) {
            None => match self.lookup_classifier(&names) {
                Some(t) => self.type_exp(t, cst),
                None => {
                    self.error(cst, bind(om::UnrecognizedType_ERROR_, &[&format!("[{}]", names.join(", "))]));
                    self.dummy(cst)
                }
            },
            Some(owner) => {
                if let Type::Enum(e) = *self.ts.types.get(owner) {
                    let info = self.ts.meta.enumeration(e);
                    let lit = info.by_name(last).or_else(|| last.strip_prefix('_').and_then(|u| info.by_name(u)));
                    if let Some(i) = lit {
                        return self.ex(ExprKind::EnumLit(e, i), owner, cst);
                    }
                }
                if let Some(p2) = self.lookup_property(owner, last) {
                    // static properties: Ecore has none; Eclipse reports non-static access
                    self.error(p.segments[n - 1].span, bind(om::NonStaticAttribute_ERROR_, &[last]));
                    let pty = self.prop_type(p2);
                    let tt = self.ts.types.type_type(owner);
                    let src = self.ex(ExprKind::Type(owner), tt, path_span_of(&p.segments[..n - 1]));
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

    fn collection_part(&mut self, p: &ast::CollectionPart) -> (CollPart, TypeId, bool, Span) {
        match p {
            ast::CollectionPart::Item(e) => {
                let x = self.expr(e).unwrap_or_else(|| self.dummy(e.span));
                let t = x.ty();
                (CollPart::Item(x.e), t, x.err, e.span)
            }
            ast::CollectionPart::Range(a, b) => {
                let x = self.expr(a).unwrap_or_else(|| self.dummy(a.span));
                let y = self.expr(b).unwrap_or_else(|| self.dummy(b.span));
                let span = a.span.to(b.span);
                if x.ty() != y.ty() {
                    let s = self.src();
                    let text = span.text(&s).to_string();
                    self.error(span, bind(om::FirstLastTypeMismatch_ERROR_, &[&text]));
                }
                let t = x.ty();
                (CollPart::Range(x.e, y.e), t, x.err || y.err, span)
            }
        }
    }

    /// `collectionLiteralExpCS`
    fn collection_literal(&mut self, k: TokenKind, parts: &[ast::CollectionPart], cst: Span) -> Ex {
        let kind = crate::names::coll_kind(k);
        let mut out = Vec::new();
        let mut elem: Option<TypeId> = None;
        let mut err = false;
        let mut first_span = cst;
        for (i, p) in parts.iter().enumerate() {
            let (cp, t, e, sp) = self.collection_part(p);
            if i == 0 {
                first_span = sp;
                elem = Some(t);
            } else {
                elem = Some(self.common_super_type_or_void(first_span, elem.unwrap(), t));
            }
            err |= e;
            out.push(cp);
        }
        let ty = self.ts.types.collection(kind, elem.unwrap_or(TypeId::OCL_VOID));
        let mut x = self.ex(ExprKind::Coll(kind, out), ty, cst);
        x.err = err;
        x
    }

    /// `getCommonSuperType` of the analyser: errors reported at `cst`, `OclVoid` on failure.
    pub fn common_super_type_or_void(&mut self, cst: Span, a: TypeId, b: TypeId) -> TypeId {
        match self.common_super_type_at(cst, a, b) {
            Some(t) => t,
            None => TypeId::OCL_VOID,
        }
    }

    /// `TypeUtil.commonSuperType(problemObject, ...)`: `None` (with the error) when there is
    /// none.
    pub fn common_super_type_at(&mut self, cst: Span, a: TypeId, b: TypeId) -> Option<TypeId> {
        use qvto_types::SuperTypeError as E;
        match self.ts.ck().common_super_type(a, b) {
            Ok(t) => Some(t),
            Err(e) => {
                let msg = match e {
                    E::TypeMismatch(x, y) => bind(om::TypeMismatch_ERROR_, &[&x, &y]),
                    E::TupleTypeMismatch(x, y) => bind(om::TupleTypeMismatch_ERROR_, &[&x, &y]),
                    E::TupleFieldNumMismatch(x, y) => bind(om::TupleFieldNumMismatch_ERROR_, &[&x, &y]),
                    E::TupleFieldNotFound(x, f, y) => bind(om::TupleFieldNotFound_ERROR_, &[&x, &f, &y]),
                };
                self.error(cst, msg);
                None
            }
        }
    }

    /// `listLiteralExpCS`
    fn list_literal(&mut self, parts: &[ast::CollectionPart], cst: Span) -> Ex {
        let mut out = Vec::new();
        let mut elem: Option<TypeId> = None;
        let mut err = false;
        let mut first_span = cst;
        for (i, p) in parts.iter().enumerate() {
            let (cp, t, e, sp) = self.collection_part(p);
            if i == 0 {
                first_span = sp;
                elem = Some(t);
            } else {
                elem = Some(self.common_super_type_or_void(first_span, elem.unwrap(), t));
            }
            err |= e;
            out.push(cp);
        }
        let ty = self.ts.types.list(elem.unwrap_or(TypeId::OCL_VOID));
        let mut x = self.ex(ExprKind::List(out), ty, cst);
        x.err = err;
        x
    }

    /// `dictionaryLiteralExp`
    fn dict_literal(&mut self, parts: &[ast::DictPart], cst: Span) -> Ex {
        let mut out = Vec::new();
        let mut kt: Option<TypeId> = None;
        let mut vt: Option<TypeId> = None;
        let mut failed = false;
        for p in parts {
            let k = self.expr(&p.key).unwrap_or_else(|| self.dummy(p.key.span));
            let v = self.expr(&p.value).unwrap_or_else(|| self.dummy(p.value.span));
            out.push((k.e, v.e));
        }
        for (k, v) in &out {
            if failed {
                break;
            }
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
            failed = kt.is_none() || vt.is_none();
        }
        let ty = match (kt, vt) {
            (Some(a), Some(b)) => self.ts.types.dict(a, b),
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
            let val = v.init.as_ref().and_then(|(_, e)| self.expr(e));
            let name = v.name.text.clone();
            let mut keep = true;
            if names.contains(&name) {
                self.error(v.span, bind(om::DuplicateNameInTuple_ERROR_, &[&name]));
                keep = false;
            } else {
                names.push(name.clone());
            }
            if val.is_none() {
                let s = self.src();
                let text = cst.text(&s).to_string();
                self.error(cst, bind(om::MissingTypeInTupleLiteralPart_ERROR_, &[&name, &text]));
            }
            let ty = match dty {
                Some(t) => t,
                None => val.as_ref().map(|x| x.ty()).unwrap_or(TypeId::OCL_VOID),
            };
            if let Some(x) = &val {
                err |= x.err;
            }
            if keep {
                tparts.push((name.clone().into(), ty));
            }
            values.push((name.into_boxed_str(), val.map(|x| x.e).unwrap_or_else(|| mk(ExprKind::Invalid, TypeId::OCL_INVALID, v.span))));
        }
        let ty = self.ts.types.tuple(tparts);
        let mut x = self.ex(ExprKind::Tuple(values), ty, cst);
        x.err = err;
        x
    }

    // ---- let, if -------------------------------------------------------------------------------

    /// `letExpCSRecursive`
    fn let_exp(&mut self, decls: &[ast::VarDecl], body: &ast::Expr, cst: Span) -> Ex {
        let Some((d, rest)) = decls.split_first() else {
            return self.expr(body).unwrap_or_else(|| self.dummy(body.span));
        };
        let (l, init) = self.variable_declaration(d, ir::LocalKind::Let, true);
        let name = d.name.text.clone();
        if self.fun.locals[l.index()].ty == TypeId::NULL {
            self.error(d.span, bind(om::DeclarationType_ERROR_, &[&name]));
            self.fun.locals[l.index()].ty = TypeId::OCL_VOID;
        }
        if init.is_none() {
            self.error(d.span, bind(om::DeclarationNoInitExp_ERROR_, &[&name]));
        }
        let sub = self.let_exp(rest, body, cst);
        self.delete_var(&name);
        let ty = sub.ty();
        let init_e = init.map(|x| x.e).unwrap_or_else(|| mk(ExprKind::Null, TypeId::OCL_VOID, d.span));
        let mut x = self.ex(ExprKind::Let(l, Box::new(init_e), Box::new(sub.e)), ty, cst);
        x.err = sub.err;
        x
    }

    /// `variableDeclarationCS` (with QVTo's type from the initialiser): a new local added to the
    /// environment. Returns the local and the analysed initialiser.
    pub fn variable_declaration(&mut self, d: &ast::VarDecl, kind: ir::LocalKind, add: bool) -> (ir::LocalId, Option<Ex>) {
        let ty = d.ty.as_ref().and_then(|t| self.type_cs(t));
        let init = d.init.as_ref().and_then(|(_, e)| self.expr(e));
        let ty = ty.or_else(|| init.as_ref().map(|x| x.ty())).unwrap_or(TypeId::NULL);
        let l = self.fun.new_local(&d.name.text, ty, kind, d.span);
        if add && !self.add_var(&d.name.text, VarRef::Local(l), true) {
            self.error(d.span, bind(om::VariableUsed_ERROR_, &[&d.name.text]));
        }
        (l, init)
    }

    /// `if c then a else b endif` (OCL) or the switch Eclipse makes of the other forms.
    fn if_exp(&mut self, i: &ast::IfExp, cst: Span, pos: Pos) -> Ex {
        let ocl = !i.c_style && i.endif && i.elifs.is_empty();
        if !ocl {
            return self.if_as_switch(i, cst, pos);
        }
        let c = self.expr(&i.cond).unwrap_or_else(|| self.dummy(i.cond.span));
        let inner = if pos == Pos::Block { Pos::Block } else { Pos::Other };
        let t = self.expr_at(&i.then, inner, i.then.span);
        let e = i.els.as_ref().and_then(|x| self.expr_at(x, inner, x.span));
        if !c.err && c.ty() != TypeId::BOOLEAN {
            let s = self.src();
            let text = i.cond.span.text(&s).to_string();
            self.error(i.cond.span, bind(om::BooleanForIf_ERROR_, &[&text]));
        }
        let mut err = false;
        let ty = match (&t, &e) {
            (Some(a), Some(b)) => {
                err = a.err || b.err;
                self.common_super_type_or_void(cst, a.ty(), b.ty())
            }
            _ => TypeId::OCL_VOID,
        };
        let det = t.as_ref().is_some_and(|x| x.det) && e.as_ref().is_some_and(|x| x.det);
        let then_e = t.map(|x| x.e).unwrap_or_else(|| mk(ExprKind::Null, TypeId::OCL_VOID, i.then.span));
        let mut x = self.ex(ExprKind::If(Box::new(c.e), Box::new(then_e), e.map(|x| Box::new(x.e))), ty, cst);
        x.err = err;
        x.det = det;
        x
    }

    /// `createIfExpCSExt`: a switch with the condition/then as first alternative.
    fn if_as_switch(&mut self, i: &ast::IfExp, cst: Span, pos: Pos) -> Ex {
        let mut alts: Vec<(&ast::Expr, &ast::Expr)> = vec![(&i.cond, &i.then)];
        for (c, t) in &i.elifs {
            alts.push((c, t));
        }
        self.switch_common(&alts, i.els.as_ref(), cst, pos)
    }

    // ---- operation calls -----------------------------------------------------------------------

    /// `a op b`
    fn binary(&mut self, op: TokenKind, a: &ast::Expr, b: &ast::Expr, cst: Span, pos: Pos) -> Ex {
        let name = match op {
            TokenKind::NotEqualExeq => "<>",
            _ => op.spelling().unwrap_or(""),
        };
        let name_span = crate::literal::token_between(&self.src(), a.span.end, b.span.start).unwrap_or(cst);
        let cs = CallCs {
            span: cst,
            name_span,
            accessor: Accessor::Dot,
            has_source: true,
            module: None,
            mapping: None,
            pos,
        };
        let src = self.expr(a).unwrap_or_else(|| self.dummy(a.span));
        let arg = self.expr(b).unwrap_or_else(|| self.dummy(b.span));
        self.operation_call(&cs, name, Some(src), vec![arg])
    }

    /// `-a`, `not a`
    fn unary(&mut self, op: TokenKind, a: &ast::Expr, cst: Span) -> Ex {
        let name = op.spelling().unwrap_or("");
        let name_span = Span::new(cst.start, cst.start + name.len() as u32);
        let cs = CallCs {
            span: cst,
            name_span,
            accessor: Accessor::Dot,
            has_source: true,
            module: None,
            mapping: None,
            pos: Pos::Other,
        };
        let src = self.expr(a).unwrap_or_else(|| self.dummy(a.span));
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
        if c.qualifier.len() > 1 && c.source.is_none() {
            return self.static_operation_call(c, cst);
        }
        let module = if c.qualifier.len() == 1 { Some(c.qualifier[0].clone()) } else { None };
        let cs = CallCs {
            span: cst,
            name_span: c.name.span,
            accessor,
            has_source: c.source.is_some(),
            module,
            mapping: None,
            pos,
        };
        if accessor == Accessor::Arrow {
            let (s, _) = c.source.as_ref().unwrap();
            let src = self.collection_source(s);
            let args = self.arguments(&c.args);
            return self.operation_call(&cs, name, Some(src), args);
        }
        let src = match &c.source {
            Some((s, _)) => Some(self.expr(s).unwrap_or_else(|| self.dummy(s.span))),
            None => None,
        };
        let args = self.arguments(&c.args);
        self.operation_call(&cs, name, src, args)
    }

    pub fn arguments(&mut self, args: &[ast::Expr]) -> Vec<Ex> {
        args.iter().map(|a| self.expr(a).unwrap_or_else(|| self.dummy(a.span))).collect()
    }

    /// `getCollectionSourceExpression`: a non-collection source becomes `Set{source}`
    /// (classic OCL's implicit `->` conversion).
    pub fn collection_source(&mut self, s: &ast::Expr) -> Ex {
        let x = self.expr(s).unwrap_or_else(|| self.dummy(s.span));
        self.as_collection_source(x, s.span)
    }

    pub fn as_collection_source(&mut self, x: Ex, span: Span) -> Ex {
        if self.ts.ck().is_collection_like(x.ty()) {
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
    fn static_operation_call(&mut self, c: &ast::Call, cst: Span) -> Ex {
        let names: Vec<&str> = c.qualifier.iter().map(|n| n.text.as_str()).collect();
        let pspan = path_span_of(&c.qualifier);
        let src = match self.lookup_classifier(&names) {
            None => {
                self.error(cst, bind(om::UnrecognizedType_ERROR_, &[&format!("[{}]", names.join(", "))]));
                None
            }
            Some(t) => Some(self.type_exp(t, pspan)),
        };
        let args = self.arguments(&c.args);
        let cs = CallCs {
            span: cst,
            name_span: c.name.span,
            accessor: Accessor::None,
            has_source: true,
            module: None,
            mapping: None,
            pos: Pos::Other,
        };
        let owner = src.as_ref().map(|s| s.ty());
        let (r, op) = self.gen_operation_call(&cs, &c.name.text, src, owner.unwrap_or(TypeId::NULL), args);
        if let (Some(ow), Some(op)) = (owner, op) {
            if let Type::TypeType(_) = self.ts.types.get(ow) {
                let in_tt = qvto_types::owner_ops(Section::Ocl, StdOwner::OclType).iter().any(|x| OpRef::Std(*x, StdOwner::OclType) == op);
                if !(in_tt || self.op_is_static(op)) {
                    self.error(cst, bind(om::NonStaticOperation_ERROR_, &[&c.name.text]));
                }
            }
        }
        r
    }

    /// `AbstractOCLAnalyzer.operationCallExpCS` with QVTo's override, for a call whose source (if
    /// any) and arguments are analysed.
    pub(crate) fn operation_call(&mut self, cs: &CallCs, name: &str, src: Option<Ex>, args: Vec<Ex>) -> Ex {
        if name == "=" && self.possible_equality_instead_of_assignment(cs) {
            self.warning(cs.name_span, bind(vm::QvtOperationalVisitorCS_possibleEqualityInsteadOfAssignment, &[]));
        }
        let had_source = src.is_some();
        let src = match src {
            Some(s) => s,
            None => self.implicit_source_for_call(cs, name, &args),
        };
        let src_err = src.err;
        let st = src.ty();
        let implicit_collect = cs.accessor == Accessor::Dot && self.ts.ck().is_collection_like(st);
        let owner = if implicit_collect {
            self.ts.types.element(st).unwrap_or(TypeId::OCL_VOID)
        } else {
            st
        };
        let is_static = matches!(self.ts.types.get(owner), Type::TypeType(_));
        let src_var = if let Info::Var(v) = src.info { Some(v) } else { None };
        let mut result;
        let op;
        if implicit_collect {
            let (it, it_name) = self.new_temp_iterator(st);
            let iter_ex = self.var_ex(VarRef::Local(it), cs.span);
            let (call, o) = self.gen_operation_call(cs, name, Some(iter_ex), owner, args);
            op = o;
            self.delete_var(&it_name);
            let mut call = call;
            call.info = Info::OpCall(o, Some(VarRef::Local(it)));
            result = self.implicit_collect(src, it, call, cs.span);
            result.info = Info::Iterator;
            self.static_check(is_static, owner, op, name, cs.span);
        } else {
            let (r, o) = self.gen_operation_call(cs, name, Some(src.clone()), owner, args);
            result = r;
            op = o;
            result.info = Info::OpCall(o, src_var);
            self.static_check(is_static, owner, op, name, cs.span);
        }
        if src_err {
            result.err = true;
        }
        // QVTo's post-processing
        if let Some(o) = op {
            if let OpRef::Std(sop, _) = o {
                let d = sop.desc();
                if d.deprecated {
                    let msg = match d.deprecated_by {
                        Some(by) => bind(vm::DeprecatedElementBy, &[d.name, by]),
                        None => bind(vm::DeprecatedElement, &[d.name]),
                    };
                    self.warning(cs.span, msg);
                }
                if let Some(reason) = d.unsupported {
                    let m = if reason.is_empty() { vm::UnsupportedElement } else { reason };
                    self.error(cs.span, bind(m, &[d.name]));
                }
            }
            if cs.accessor == Accessor::Arrow && self.ts.ck().is_collection_like(st) && !implicit_collect {
                let src_ops = self.get_operations(st);
                if !src_ops.contains(&o) {
                    result = self.rewrite_as_xcollect(result, cs);
                }
            }
        }
        if !had_source || matches!(result.info, Info::OpCall(..)) {
            if let Info::OpCall(_, sv) = result.info {
                self.validate_implicit_source_call(cs.span, Some(cs.name_span).filter(|_| true), sv, ImplicitKind::Op(op), !had_source);
            }
        }
        // validateOperationCall (QVTo's oclExpressionCS)
        match result.info {
            Info::OpCall(Some(o), sv) => self.validate_operation_call(cs, &result, o, sv),
            Info::Iterator => {
                if let Some(o) = op {
                    if let ExprKind::Loop(lp) = &result.e.kind {
                        if let Some(b) = &lp.body {
                            let probe = Ex { e: b.clone(), err: false, info: Info::Other, det: false };
                            self.validate_operation_call(cs, &probe, o, None);
                        }
                    }
                }
            }
            _ => {}
        }
        result
    }

    fn static_check(&mut self, is_static: bool, owner: TypeId, op: Option<OpRef>, name: &str, span: Span) {
        if !is_static {
            return;
        }
        let in_tt = op.is_some_and(|o| matches!(o, OpRef::Std(_, StdOwner::OclType)));
        let st = op.is_some_and(|o| self.op_is_static(o));
        if !(in_tt || st) {
            self.error(span, bind(om::NonStaticOperation_ERROR_, &[name]));
        }
        let _ = owner;
    }

    /// Turns `src->op(args)` whose operation is not a collection operation into
    /// `src->xcollect(e | e.op(args))` (`createImplicitXCollect`).
    fn rewrite_as_xcollect(&mut self, call: Ex, cs: &CallCs) -> Ex {
        let ExprKind::Call(c) = call.e.kind else {
            return call;
        };
        let mut c = *c;
        let Some(src) = c.source.take() else {
            return self.ex(ExprKind::Call(Box::new(c)), call.e.ty, call.e.span);
        };
        let src_ex = Ex { e: src, err: call.err, info: Info::Other, det: false };
        let (it, it_name) = self.new_temp_iterator(src_ex.ty());
        let ity = self.fun.locals[it.index()].ty;
        c.source = Some(mk(ExprKind::Local(it), ity, cs.span));
        self.delete_var(&it_name);
        let body = Ex { e: mk(ExprKind::Call(Box::new(c)), call.e.ty, call.e.span), err: false, info: call.info, det: false };
        self.implicit_xcollect(src_ex, it, body, cs.span)
    }

    /// `createImplicitSource`
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
                self.warning(cs.name_span, bind(vm::AmbiguousImplicitSourceReference, &[&names.join(", ")]));
                list.first().copied()
            }
        };
        match v {
            Some(v) => self.var_ex(v, cs.span),
            None => {
                let mut s = format!("{name}(");
                for (i, x) in args.iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    let n = self.type_name(x.ty());
                    s.push_str(&n);
                }
                s.push(')');
                self.error(cs.span, bind(om::IllegalSignature_ERROR_, &[&s]));
                let mut x = self.ex(ExprKind::Null, TypeId::OCL_VOID, cs.span);
                x.info = Info::Var(VarRef::This(crate::analyzer::STDLIB_MODULE));
                x.info = Info::Other;
                x
            }
        }
    }

    pub fn arg_of(&self, x: &Ex) -> Arg {
        Arg {
            ty: x.ty(),
            type_exp: matches!(x.info, Info::TypeExp(_)),
            referred: if let Info::TypeExp(t) = x.info { Some(t) } else { None },
        }
    }

    /// QVTo's `lookupOperation`: module-qualified lookups, ambiguity as a warning.
    fn lookup_operation(&mut self, problem: Span, module: Option<&ast::Name>, owner: TypeId, name: &str, args: &[Arg]) -> Option<OpRef> {
        if let Some(mq) = module {
            return self.qualified_operation_lookup(problem, mq, owner, name, args);
        }
        let r = match self.find_most_specific_operation(owner, name, args) {
            Ok(None) => {
                if let Some(u) = name.strip_prefix('_') {
                    self.find_most_specific_operation(owner, u, args)
                } else {
                    Ok(None)
                }
            }
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
        self.warning(problem, bind(vm::AmbiguousOperationReference, &[&parts.join(", ")]));
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
    fn qualified_operation_lookup(&mut self, _problem: Span, mq: &ast::Name, owner: TypeId, name: &str, args: &[Arg]) -> Option<OpRef> {
        let cur = self.cur_module;
        let target = mq.text.as_str();
        let envs: Vec<crate::ir::ModuleId> = if self.modules[cur.index()].name == target {
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
            let actual_owner = if matches!(self.ts.types.get(owner), Type::Module(_)) { self.modules[e.index()].ty } else { owner };
            let saved = self.cur_module;
            self.cur_module = e;
            let r = self.find_most_specific_operation(actual_owner, name, args);
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
                    for m in matches {
                        if self.op_module(m) == Some(e) {
                            if let Some(prev) = op {
                                resolved = self.handle_ambiguous_operations(mq.span, vec![prev, m]);
                                break;
                            }
                            op = Some(m);
                        }
                    }
                    if resolved.is_none() {
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
    pub(crate) fn gen_operation_call(&mut self, cs: &CallCs, name: &str, src: Option<Ex>, owner: TypeId, args: Vec<Ex>) -> (Ex, Option<OpRef>) {
        let a: Vec<Arg> = args.iter().map(|x| self.arg_of(x)).collect();
        let mut op_owner = owner;
        let src_ty = src.as_ref().map(|s| s.ty());
        let arrow_coll = cs.accessor == Accessor::Arrow && src_ty.is_some_and(|t| matches!(self.ts.types.get(t), Type::Collection(..) | Type::List(_) | Type::Dict(..)));
        if arrow_coll && self.lookup_operation(cs.span, None, owner, name, &a).is_none() {
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
                    let n = if x.ty == TypeId::NULL { "null".to_string() } else { self.type_name(x.ty) };
                    s.push_str(&n);
                }
                s.push(')');
                let on = self.type_name(op_owner);
                let msg = if src_ty == Some(op_owner) || src_ty.is_none() {
                    bind(vm::QvtOperationalVisitorCS_OperationNotFound_ERROR_, &[&s, &on])
                } else {
                    let sn = self.type_name(src_ty.unwrap());
                    bind(vm::QvtOperationalVisitorCS_OperationNotFoundForCollectionAndItsElement, &[&s, &sn, &on])
                };
                self.error(cs.span, msg);
            }
            Some(o) => {
                ty = self.result_type(cs.span, op_owner, o, &a).unwrap_or_else(|| self.op_ret(o));
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
    pub(crate) fn result_type(&mut self, problem: Span, owner: TypeId, o: OpRef, args: &[Arg]) -> Option<TypeId> {
        if self.op_module(o).is_some() {
            return Some(self.generic_return_type(problem, owner, o, args));
        }
        let code = crate::opcode::code(self.op_name(o));
        if code.is_none() {
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
    fn generic_return_type(&mut self, problem: Span, source: TypeId, o: OpRef, args: &[Arg]) -> TypeId {
        let mut bind_map: Vec<(TypeId, TypeId)> = Vec::new();
        let ret = self.op_ret(o);
        let is_generic = |t: TypeId| t == TypeId::T || t == TypeId::T2 || t == TypeId::KEY_T;
        let set = |m: &mut Vec<(TypeId, TypeId)>, k: TypeId, v: TypeId| {
            if let Some(e) = m.iter_mut().find(|(a, _)| *a == k) {
                e.1 = v;
            } else {
                m.push((k, v));
            }
        };
        if is_generic(ret) {
            let actual = match *self.ts.types.get(source) {
                Type::TypeType(r) => r,
                _ => source,
            };
            if actual != TypeId::NULL {
                set(&mut bind_map, ret, actual);
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
        let set = |m: &mut Vec<(TypeId, TypeId)>, k: TypeId, v: TypeId| {
            if let Some(e) = m.iter_mut().find(|(a, _)| *a == k) {
                e.1 = v;
            } else {
                m.push((k, v));
            }
        };
        let fty = self.ts.types.get(formal).clone();
        let aty = self.ts.types.get(actual).clone();
        let fcoll = matches!(fty, Type::Collection(..) | Type::List(_) | Type::Dict(..));
        let acoll = matches!(aty, Type::Collection(..) | Type::List(_) | Type::Dict(..));
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
        } else if formal == TypeId::T || formal == TypeId::T2 || formal == TypeId::KEY_T {
            set(m, formal, actual);
            if !m.iter().any(|(k, _)| *k == TypeId::T2) {
                let v = if acoll { self.flattened_element(actual) } else { actual };
                set(m, TypeId::T2, v);
            }
        }
    }

    /// `CollectionUtil.getFlattenedElementType`
    pub fn flattened_element(&self, t: TypeId) -> TypeId {
        let mut e = self.ts.types.element(t).unwrap_or(t);
        while matches!(self.ts.types.get(e), Type::Collection(..)) {
            e = self.ts.types.element(e).unwrap();
        }
        e
    }

    fn generic_bind(&mut self, problem: Span, t: TypeId, m: &[(TypeId, TypeId)]) -> TypeId {
        let bound = |s: &mut Self, x: TypeId| -> TypeId {
            if x == TypeId::COMMON_T {
                let a = m.iter().find(|(k, _)| *k == TypeId::T).map(|(_, v)| *v);
                let b = m.iter().find(|(k, _)| *k == TypeId::T2).map(|(_, v)| *v);
                if let (Some(a), Some(b)) = (a, b) {
                    return s.common_super_type_at(problem, a, b).unwrap_or(TypeId::OCL_ANY);
                }
            }
            m.iter().find(|(k, _)| *k == x).map(|(_, v)| *v).unwrap_or(x)
        };
        match self.ts.types.get(t).clone() {
            Type::Collection(k, e) => {
                let be = bound(self, e);
                self.ts.types.collection(k, be)
            }
            Type::List(e) => {
                let be = bound(self, e);
                self.ts.types.list(be)
            }
            Type::Dict(k, v) => {
                let bk = bound(self, k);
                let bv = bound(self, v);
                self.ts.types.dict(bk, bv)
            }
            Type::Tuple(_) => t,
            Type::TypeType(r) => {
                let mut rt = bound(self, r);
                if t == TypeId::OCL_TYPE {
                    rt = bound(self, TypeId::T);
                }
                self.ts.types.type_type(rt)
            }
            _ => bound(self, t),
        }
    }

    // ---- navigation ----------------------------------------------------------------------------

    /// `src.name`, `src->name` (QVTo's xcollect shorthand).
    fn nav_exp(&mut self, n: &ast::Nav, cst: Span) -> Ex {
        if !n.qualifier.is_empty() {
            // `src.p::name`: qualified property names (rare); resolve the simple name
        }
        let src = self.expr(&n.source).unwrap_or_else(|| self.dummy(n.source.span));
        let arrow = n.op == NavOp::Arrow;
        self.simple_name(&n.name.text, n.name.span, Some(src), cst, arrow)
    }

    // ---- validation of calls -------------------------------------------------------------------

    /// `isPossibleEqualityInsteadOfAssignment`
    fn possible_equality_instead_of_assignment(&self, cs: &CallCs) -> bool {
        cs.pos == Pos::Block
    }

    /// `validateOperationCall`
    fn validate_operation_call(&mut self, cs: &CallCs, call: &Ex, o: OpRef, src_var: Option<VarRef>) {
        let (src_ty, arg_tys): (Option<TypeId>, Vec<TypeId>) = match &call.e.kind {
            ExprKind::Call(c) => (c.source.as_ref().map(|s| s.ty), c.args.iter().map(|a| a.ty).collect()),
            _ => (None, Vec::new()),
        };
        let name = self.op_name(o).to_string();
        if name == "oclAsType" {
            if let (Some(st), Some(at)) = (src_ty, arg_tys.first()) {
                if let Type::TypeType(target) = *self.ts.types.get(*at) {
                    if self.incorrect_cast(st, target) {
                        self.warning(cs.span, vm::incorrectCastWarning);
                    }
                }
            }
        }
        let call_name = if cs.name_is_operator { cs.op_name.as_str() } else { name.as_str() };
        if call_name == "=" || call_name == "<>" {
            if let (Some(st), Some(&at)) = (src_ty, arg_tys.first()) {
                if !self.ts.ck().assignable(st, at) && !self.ts.ck().assignable(at, st) {
                    let a = self.full_name(at);
                    let b = self.full_name(st);
                    self.warning(cs.span, bind(vm::TypesDoNotConform, &[&a, &b]));
                }
            }
        }
        if self.op_is_mapping(o) && cs.mapping.is_none() {
            self.warning(cs.span, bind(vm::QvtOperationalVisitorCS_mapKeywordNotUsed, &[&name]));
        }
        if let OpRef::Std(s, _) = o {
            if is_mutator(s) {
                if let Some(v) = src_var {
                    let span = cs.source_span.unwrap_or(cs.span);
                    self.validate_variable_modification(v, span, None, false);
                }
            }
        }
    }

    /// `QvtOperationalParserUtil.isIncorrectCast`
    fn incorrect_cast(&self, s: TypeId, t: TypeId) -> bool {
        match (self.ts.types.get(s), self.ts.types.get(t)) {
            (Type::Real, tt) => matches!(tt, Type::Integer),
            (Type::Collection(CollKind::Set, _), tt) => !matches!(tt, Type::Collection(CollKind::Set, _)),
            (Type::Collection(CollKind::Bag, _), tt) => !matches!(tt, Type::Collection(CollKind::Bag, _)),
            (Type::Collection(CollKind::Sequence, _), tt) => !matches!(tt, Type::Collection(CollKind::Sequence, _)),
            (Type::Collection(CollKind::OrderedSet, _), tt) => !matches!(tt, Type::Collection(CollKind::OrderedSet | CollKind::Set, _)),
            _ => false,
        }
    }

    /// `DeprecatedImplicitSourceCallHelper.validateCallExp`
    fn validate_implicit_source_call(&mut self, cs_span: Span, name_span: Option<Span>, src_var: Option<VarRef>, kind: ImplicitKind, implicit: bool) {
        if !implicit {
            return;
        }
        let cause = name_span.unwrap_or(cs_span);
        let Some(v) = src_var else {
            return;
        };
        let vn = self.var_name(v);
        if vn == "self" {
            if !self.in_object_with_self && true {
                self.error(cause, vm::DeprecatedImplicitSourceCall_contextualImplicitCall);
            }
        } else if vn == "this" {
            match kind {
                ImplicitKind::Op(Some(o)) => {
                    let owner = self.op_owner(o);
                    if !matches!(self.ts.types.get(owner), Type::Module(_)) {
                        self.warning(cause, vm::DeprecatedImplicitSourceCall_moduleScopeImplicitCall);
                    }
                }
                ImplicitKind::Prop(Info::Prop(p, _)) => {
                    let owner = self.prop_owner(p);
                    if !matches!(self.ts.types.get(owner), Type::Module(_)) {
                        self.warning(cause, vm::DeprecatedImplicitSourceCall_moduleScopeImplicitCall);
                    }
                }
                _ => {}
            }
        }
    }
}

#[derive(Copy, Clone)]
pub(crate) enum ImplicitKind {
    Op(Option<OpRef>),
    Prop(Info),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Accessor {
    Dot,
    Arrow,
    None,
}

/// The parts of an operation call's CST node the analyser reports on.
#[derive(Clone, Debug)]
pub(crate) struct CallCs {
    pub span: Span,
    pub name_span: Span,
    pub accessor: Accessor,
    pub has_source: bool,
    /// `M::op(...)` (`ImperativeOperationCallExpCS.getModule`)
    pub module: Option<ast::Name>,
    /// `map` (false) / `xmap` (true) calls.
    pub mapping: Option<bool>,
    pub pos: Pos,
}

impl CallCs {
    #[allow(non_upper_case_globals)]
    const name_is_operator: bool = false;
}

pub(crate) fn path_span_of(names: &[ast::Name]) -> Span {
    match (names.first(), names.last()) {
        (Some(a), Some(b)) => a.span.to(b.span),
        _ => Span::default(),
    }
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

#[allow(dead_code)]
fn _unused() -> u8 {
    SAME_TYPE
}

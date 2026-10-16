//! Eclipse's AST validation after analysis (`QvtOperationalValidationVisitor` with its
//! `CustomOclValidationVisitor` and `MappingExtensionHelper.validate`), run with duplicate error
//! suppression. It walks the typed IR, which keeps Eclipse's AST shape and positions.

use crate::analyzer::Sema;
use crate::ir::{self, Callee, Direction, ExprKind, LoopKind, ModuleId, OpId, OpKind};
use crate::messages::{bind, ocl as om, validation as vm};
use crate::module::is_disjunctive;
use crate::units::MethodCs;
use qvto_syntax::Span;
use qvto_types::{rel, CollKind, Type, TypeId};

/// An ancestor of the node being validated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Anc {
    /// An operation body (helper/constructor/object body, mapping section).
    OpBody,
    Block,
    /// OCL `IfExp`
    If,
    /// `SwitchExp` or its alternatives
    Switch,
    /// OCL iterator or `iterate`
    OclLoop,
    /// Imperative loop (`forEach`, imperative iterators) or `while`
    ImpLoop,
    VarInit,
    Assign,
    Compute,
    /// Anything else imperative (`try`, `log`, `assert`, `return`, `raise`, ...)
    Imperative,
    /// An OCL expression (call, let, literal, ...)
    Ocl,
}

/// The code being validated: its operation (if any) and variable slots.
struct Ctx<'a> {
    op: Option<OpId>,
    mapping_body: bool,
    locals: &'a [ir::Local],
    extras: Option<&'a [crate::env::LocalExtra]>,
}

impl<'r> Sema<'r> {
    /// `QvtOperationalValidationVisitor` over a module.
    pub(crate) fn validate_module(&mut self, m: ModuleId, methods: &[(MethodCs<'_>, OpId)]) {
        let ops: Vec<OpId> = self.modules[m.index()].ir.operations.clone();
        for op in ops {
            let mcs = methods.iter().find(|(_, o)| *o == op).map(|(c, _)| *c);
            self.validate_operation(op, mcs);
        }
        // module properties' initialisers
        let globals = self.modules[m.index()].ir.globals.clone();
        for g in globals {
            if let Some(b) = self.globals[g.index()].ir.init.clone() {
                let ctx = Ctx {
                    op: None,
                    mapping_body: false,
                    locals: &b.locals,
                    extras: None,
                };
                let mut anc = vec![Anc::Ocl];
                self.v_expr(&b.expr, &ctx, &mut anc);
            }
        }
        let props: Vec<usize> = (0..self.interm_props.len()).filter(|&i| self.interm_props[i].ir.module == m).collect();
        for i in props {
            if let Some(b) = self.interm_props[i].ir.init.clone() {
                let ctx = Ctx {
                    op: None,
                    mapping_body: false,
                    locals: &b.locals,
                    extras: None,
                };
                let mut anc = vec![Anc::Ocl];
                self.v_expr(&b.expr, &ctx, &mut anc);
            }
        }
    }

    fn validate_operation(&mut self, op: OpId, mcs: Option<MethodCs<'_>>) {
        let info = self.ops[op.index()].ir.clone();
        let module = info.module;
        // visitImperativeOperation
        if let Some(ct) = info.context {
            if matches!(self.ts.types.get(ct), Type::Module(_)) {
                let tn = self.qualified_name(ct);
                let src = self.src();
                let span = mcs.map(|c| crate::module::method_decl(c, &src)).and_then(|d| d.context.map(|c| c.span())).unwrap_or(info.decl_span);
                self.error(span, bind(vm::QvtOperationalValidationVisitor_invalidContextualType, &[&tn]));
            }
        }
        self.validate_unique_param_names(op);
        for p in &info.params {
            if p.name.trim().is_empty() {
                self.error(p.span, vm::QvtOperationalValidationVisitor_parameterNamedRequired);
            }
        }
        for r in &info.results {
            if r.name.trim().is_empty() {
                self.error(r.span, vm::QvtOperationalValidationVisitor_parameterNamedRequired);
            }
        }
        match info.kind {
            OpKind::Mapping => {
                self.validate_mapping_extensions(op, mcs);
                for r in &info.results {
                    self.validate_out_param_type(r.ty, r.span);
                }
                for p in &info.params {
                    if p.direction == Direction::Out {
                        self.validate_out_param_type(p.ty, p.span);
                    }
                }
            }
            OpKind::Entry => {
                if self.modules[module.index()].is_library {
                    self.error(Span::new(info.decl_span.start, info.span.end), vm::QvtOperationalValidationVisitor_MainInLibraryError);
                }
                for p in &info.params {
                    self.warning(p.span, bind(vm::EntryOp_DepricatedParamDecl, &[]));
                }
            }
            _ => {}
        }
        // visitOperationBody (non-mapping bodies)
        if info.kind != OpKind::Mapping && info.result_ty != TypeId::OCL_VOID && info.results.len() == 1 {
            if let Some(body) = &info.body {
                let n = body.len();
                let last_det = n > 0 && self.is_deterministic(&body[n - 1]);
                if (n == 0 && !info.is_blackbox) || (n > 1 && !last_det) {
                    let end = info.results.last().map(|r| r.span.end).unwrap_or(info.decl_span.start);
                    self.warning(Span::new(info.decl_span.start, end), vm::useReturnExpForOperationResult);
                }
            }
        }
        // the bodies
        let extras = self.op_extras[op.index()].clone();
        let ctx = Ctx {
            op: Some(op),
            mapping_body: info.kind == OpKind::Mapping,
            locals: &info.locals,
            extras: Some(&extras),
        };
        if let Some(body) = &info.body {
            for e in body {
                let mut anc = vec![Anc::OpBody];
                self.v_expr(e, &ctx, &mut anc);
            }
        }
        if let Some(m) = &info.mapping {
            for e in m.init.iter().chain(m.population.iter()).chain(m.end.iter()) {
                let mut anc = vec![Anc::OpBody];
                self.v_expr(e, &ctx, &mut anc);
            }
            for e in m.when.iter().chain(m.where_.iter()) {
                let mut anc = vec![Anc::Ocl];
                let c2 = Ctx {
                    op: Some(op),
                    mapping_body: false,
                    locals: ctx.locals,
                    extras: ctx.extras,
                };
                self.v_expr(e, &c2, &mut anc);
            }
        }
    }

    fn validate_unique_param_names(&mut self, op: OpId) {
        let info = &self.ops[op.index()].ir;
        let module = info.module;
        let model: Vec<String> = if self.modules[module.index()].is_library {
            Vec::new()
        } else {
            self.modules[module.index()].model_params.iter().map(|p| p.name.clone()).collect()
        };
        let params: Vec<(String, Span)> = info.params.iter().map(|p| (p.name.clone(), p.span)).collect();
        let results: Vec<(String, Span)> = info.results.iter().map(|p| (p.name.clone(), p.span)).collect();
        let check = |s: &mut Self, list: &[(String, Span)], scope: &[(String, Span)], same: bool| {
            for (i, (n, sp)) in list.iter().enumerate() {
                let found = scope.iter().enumerate().find(|(_, (sn, _))| sn == n);
                if let Some((j, _)) = found {
                    if !(same && i == j) {
                        s.error(*sp, bind(vm::NameAlreadyDefinedError, &[n]));
                    }
                }
            }
        };
        let model_list: Vec<(String, Span)> = model.into_iter().map(|n| (n, Span::default())).collect();
        check(self, &params, &model_list, false);
        check(self, &params, &params, true);
        check(self, &results, &model_list, false);
        check(self, &results, &params, false);
        check(self, &results, &results, true);
    }

    /// `validateOutParamType`
    fn validate_out_param_type(&mut self, t: TypeId, span: Span) {
        if t == TypeId::NULL {
            return;
        }
        let elem_class = self.ts.types.class(self.ts.std.element);
        let bad = matches!(self.ts.types.get(t), Type::ModelType(_) | Type::Module(_))
            || (!self.ts.ck().is_user_model_element(t) && !self.ts.types.is_collection_like(t) && t != elem_class);
        if bad {
            let tn = self.qualified_name(t);
            self.error(span, bind(vm::nonModelTypeError, &[&tn]));
        }
    }

    /// `safeGetMappingQualifiedName`
    fn mapping_qualified_name(&mut self, op: OpId) -> String {
        let info = &self.ops[op.index()];
        let ctx = info.contextual.then_some(info.owner);
        let name = info.ir.name.clone();
        match ctx {
            Some(c) => {
                let q = self.qualified_name(c);
                format!("{q}::{name}")
            }
            None => name,
        }
    }

    /// `MappingExtensionHelper.validate`
    fn validate_mapping_extensions(&mut self, op: OpId, mcs: Option<MethodCs<'_>>) {
        let Some(m) = self.ops[op.index()].ir.mapping.clone() else {
            return;
        };
        let refs = self.ext_refs.get(&op).cloned().unwrap_or_default();
        let op_span = Span::new(self.ops[op.index()].ir.decl_span.start, self.ops[op.index()].ir.span.end);
        // inherits
        self.validate_extension_list(op, &m.inherits, &refs[0], "inherits", op_span, true);
        if !m.disjuncts.is_empty() {
            // the body of a disjuncting mapping is never executed
            if let Some(MethodCs::Mapping(ma)) = mcs {
                let body_span = ma.body.as_ref().map(|b| b.span);
                let has_body = ma.body.is_some();
                if has_body {
                    let body = ma.body.as_ref().unwrap();
                    let empty_content = match &body.population {
                        None => true,
                        Some(p) => {
                            let implicit = !p.explicit && (ma.results.len() == 1 || (ma.results.is_empty() && ma.name.names.len() > 1 && ma.direction == Some(qvto_syntax::TokenKind::KwInout)));
                            if implicit {
                                Some(p.body.span) == body_span
                            } else {
                                p.body.stmts.is_empty()
                            }
                        }
                    };
                    let non_empty = !empty_content || body.init.is_some() || body.end.is_some();
                    if non_empty {
                        let qn = self.mapping_qualified_name(op);
                        self.warning(body.span, bind(vm::MappingExtension_disjunctingMappingBodyNotExecuted, &[&qn]));
                    }
                }
            }
            let use_refs = refs[1].len() == m.disjuncts.len();
            for (kind, list, krefs) in [("inherits", &m.inherits, &refs[0]), ("merges", &m.merges, &refs[2])] {
                let use_k = krefs.len() == list.len();
                for (pos, ext) in list.iter().enumerate() {
                    let span = if use_k { krefs.get(pos).copied().unwrap_or(op_span) } else { op_span };
                    let a = self.mapping_qualified_name(op);
                    let b = self.mapping_qualified_name(*ext);
                    self.error(span, bind(vm::MappingExtension_illegalExtensionKindOnDisjunctingMapping, &[&a, kind, &b]));
                }
            }
            // out parameters of the disjuncted mappings
            let params = self.ops[op.index()].ir.params.clone();
            for (pos, ext) in m.disjuncts.iter().enumerate() {
                let eparams = self.ops[ext.index()].ir.params.clone();
                let mut it = eparams.iter();
                for p in &params {
                    if p.direction != Direction::Out {
                        continue;
                    }
                    let Some(ep) = it.next() else {
                        break;
                    };
                    if ep.ty != p.ty && use_refs {
                        let span = refs[1].get(pos).copied().unwrap_or(op_span);
                        let a = self.mapping_qualified_name(*ext);
                        let b = self.mapping_qualified_name(op);
                        self.error(span, bind(vm::MappingExtension_illegalOutParamDisjunctingMapping, &[&a, &p.name, &b]));
                    }
                }
            }
            self.validate_extension_list(op, &m.disjuncts, &refs[1], "disjuncts", op_span, false);
        }
        self.validate_extension_list(op, &m.merges, &refs[2], "merges", op_span, false);
        let _ = is_disjunctive;
    }

    fn validate_extension_list(&mut self, op: OpId, list: &[OpId], refs: &[Span], kind: &str, op_span: Span, inherit: bool) {
        let use_refs = refs.len() == list.len();
        for (i, ext) in list.iter().enumerate() {
            // caller/callee: inherits(extended, extending), merges(extended=merged... ) as Eclipse
            let ok = match kind {
                "inherits" => self.caller2callee(*ext, op),
                "merges" => self.caller2callee(*ext, op),
                _ => self.caller2callee(op, *ext),
            };
            let _ = inherit;
            if !ok {
                let span = if use_refs { refs.get(i).copied().unwrap_or(op_span) } else { op_span };
                let a = self.mapping_qualified_name(*ext);
                let b = self.mapping_qualified_name(op);
                self.error(span, bind(vm::MappingExtension_nonConformantSignatureForMappingExtension, &[&a, kind, &b]));
            }
        }
    }

    /// `isCaller2CalleeCompatible`
    fn caller2callee(&mut self, extending: OpId, extended: OpId) -> bool {
        let a = self.ops[extending.index()].ir.clone();
        let b = self.ops[extended.index()].ir.clone();
        if a.params.len() != b.params.len() {
            return false;
        }
        match (a.context, b.context) {
            (None, None) => {}
            (Some(c1), Some(c2)) => {
                if self.ts.ck().relationship(c2, c1) & rel::SUBTYPE == 0 {
                    return false;
                }
            }
            _ => return false,
        }
        let lists = [(&a.params, &b.params), (&a.results, &b.results)];
        for (caller, callee) in lists {
            if caller.len() != callee.len() {
                return false;
            }
            for (cp, ce) in caller.iter().zip(callee.iter()) {
                let assignable = self.ts.ck().relationship(ce.ty, cp.ty) & rel::SUBTYPE != 0;
                let dir_ok = cp.direction == ce.direction
                    || match cp.direction {
                        Direction::In => matches!(ce.direction, Direction::InOut | Direction::In),
                        Direction::InOut => ce.direction == Direction::InOut,
                        Direction::Out => false,
                    };
                if !assignable || !dir_ok {
                    return false;
                }
            }
        }
        a.results.len() == 1 || self.ts.ck().relationship(a.result_ty, b.result_ty) & rel::SUBTYPE != 0
    }

    /// `QvtOperationalVisitorCS.isOclExpDeterministic` on the IR.
    pub(crate) fn is_deterministic(&self, e: &ir::Expr) -> bool {
        match &e.kind {
            ExprKind::Return(_) | ExprKind::Raise(..) => true,
            ExprKind::Block(b) => b.len() == 1 && self.is_deterministic(&b[0]),
            ExprKind::If(_, t, Some(el)) => self.is_deterministic(t) && self.is_deterministic(el),
            ExprKind::If(_, _, None) => false,
            ExprKind::Switch(alts, els) => els.as_ref().is_some_and(|x| self.is_deterministic(x)) && alts.iter().all(|(_, b)| self.is_deterministic(b)),
            ExprKind::Compute(_, _, b) => self.is_deterministic(b),
            _ => false,
        }
    }

    fn local_ty(&self, ctx: &Ctx<'_>, l: ir::LocalId) -> TypeId {
        ctx.locals.get(l.index()).map(|x| x.ty).unwrap_or(TypeId::NULL)
    }

    /// Validates an expression and its children. `anc`: the ancestors, innermost last.
    fn v_expr(&mut self, e: &ir::Expr, ctx: &Ctx<'_>, anc: &mut Vec<Anc>) {
        let span = e.span;
        match &e.kind {
            ExprKind::Local(l) => {
                let lt = self.local_ty(ctx, *l);
                let name = ctx.locals.get(l.index()).map(|x| x.name.clone()).unwrap_or_default();
                if lt == TypeId::NULL || e.ty == TypeId::NULL {
                    self.error(span, bind(om::IncompleteVariableExp_ERROR_, &[&name]));
                } else if name == "result" && matches!(self.ts.types.get(lt), Type::Tuple(_)) {
                    let is_param = ctx.extras.and_then(|x| x.get(l.index())).is_some_and(|x| x.param.is_some());
                    if !is_param {
                        self.warning(span, bind(vm::QvtOperationalVisitorCS_deprecatedResultTupleAccess, &[]));
                    }
                }
            }
            ExprKind::ModelParam(p) => {
                let mp = &self.modules[p.module.index()].model_params[p.index as usize];
                if mp.ty == TypeId::NULL {
                    let n = mp.name.clone();
                    self.error(span, bind(om::IncompleteVariableExp_ERROR_, &[&n]));
                }
            }
            ExprKind::If(c, t, el) => {
                if c.ty != TypeId::BOOLEAN {
                    self.error(span, bind(vm::QvtOperationalVisitorCS_ifExpNonBooleanCond, &[]));
                }
                if el.is_none() && anc.contains(&Anc::VarInit) {
                    self.warning(span, bind(vm::QvtOperationalVisitorCS_ifExpWithoutElseAssignment, &[]));
                }
                anc.push(Anc::If);
                self.v_expr(c, ctx, anc);
                self.v_expr(t, ctx, anc);
                if let Some(x) = el {
                    self.v_expr(x, ctx, anc);
                }
                anc.pop();
                return;
            }
            ExprKind::Break | ExprKind::Continue => {
                let name = if matches!(e.kind, ExprKind::Break) { "BreakExp" } else { "ContinueExp" };
                let mut found = false;
                for a in anc.iter().rev() {
                    match a {
                        Anc::ImpLoop => {
                            found = true;
                            break;
                        }
                        Anc::OclLoop => break,
                        _ => {}
                    }
                }
                if !found {
                    self.error(span, bind(vm::BreakContinue_InvalidExpressionUsage, &[name]));
                } else if matches!(anc.last(), Some(Anc::Ocl) | Some(Anc::OpBody)) {
                    self.error(span, bind(vm::BreakContinue_InvalidExpressionOwner, &[name]));
                }
            }
            ExprKind::VarInit { init, .. } => {
                let ok = matches!(anc.last(), Some(Anc::OpBody) | Some(Anc::Block) | Some(Anc::Assign) | Some(Anc::VarInit));
                if !ok {
                    self.error(span, vm::QvtOperationalValidationVisitor_CannotDeclareVariables);
                }
                if let Some(i) = init {
                    anc.push(Anc::VarInit);
                    self.v_expr(i, ctx, anc);
                    anc.pop();
                }
                return;
            }
            ExprKind::Return(v) => {
                self.validate_return(e, v.as_deref(), ctx);
            }
            ExprKind::Global(_) if e.ty == TypeId::NULL => {
                self.error(span, bind(om::NullNavigationType_ERROR_, &["this.\"<null>\""]));
            }
            ExprKind::Assign(a) => {
                if let ir::LValue::Global(g) = &a.target {
                    if self.globals[g.index()].ir.ty == TypeId::NULL {
                        self.error(a.target_span, bind(om::NullNavigationType_ERROR_, &["this.\"<null>\""]));
                    }
                }
            }
            ExprKind::Call(c) => {
                self.validate_call(e, c, ctx);
            }
            ExprKind::Loop(lp) => {
                self.validate_loop(e, lp, ctx);
                let a = if is_ocl_loop(lp.kind) { Anc::OclLoop } else { Anc::ImpLoop };
                anc.push(a);
                self.v_expr(&lp.source, ctx, anc);
                if let Some((_, i)) = &lp.acc {
                    self.v_expr(i, ctx, anc);
                }
                if let Some(b) = &lp.body {
                    self.v_expr(b, ctx, anc);
                }
                if let Some(c) = &lp.cond {
                    self.v_expr(c, ctx, anc);
                }
                anc.pop();
                return;
            }
            ExprKind::New(n) => {
                self.validate_instantiation(e, n, ctx);
            }
            ExprKind::NewTransformation(m, args) => {
                self.validate_new_transformation(e, *m, args);
            }
            _ => {}
        }
        // children
        let a = anc_of(&e.kind);
        anc.push(a);
        for_each_child(e, |c| self.v_expr(c, ctx, anc));
        anc.pop();
    }

    fn validate_return(&mut self, e: &ir::Expr, value: Option<&ir::Expr>, ctx: &Ctx<'_>) {
        let Some(op) = ctx.op else {
            self.error(e.span, vm::returnUsedOutsideOperationBody);
            return;
        };
        let declared = self.ops[op.index()].ret;
        let actual = e.ty;
        if actual != TypeId::NULL && declared != TypeId::NULL {
            if self.ts.ck().relationship(actual, declared) & rel::SUBTYPE == 0 {
                let tn = self.qualified_name(declared);
                self.error(e.span, bind(vm::typeMismatchError, &[&tn]));
            }
            if value.is_none() && !self.ops[op.index()].ir.results.is_empty() {
                self.error(e.span, vm::missingReturnValueError);
            }
        } else if !(actual != TypeId::NULL && value.is_none() && declared == TypeId::OCL_VOID) {
            let tn = if declared == TypeId::NULL { String::new() } else { self.qualified_name(declared) };
            self.error(e.span, bind(vm::typeMismatchError, &[&tn]));
        }
        if ctx.mapping_body {
            self.error(e.span, vm::returnNotAllowedInMappingYet);
        }
    }

    fn validate_call(&mut self, e: &ir::Expr, c: &ir::Call, _ctx: &Ctx<'_>) {
        match &c.callee {
            Callee::Op { op, .. } => {
                if self.ops[op.index()].ir.kind == OpKind::Constructor {
                    let name = self.ops[op.index()].ir.name.clone();
                    let mut s = format!("{name}(");
                    for (i, a) in c.args.iter().enumerate() {
                        if i > 0 {
                            s.push_str(", ");
                        }
                        let n = if a.ty == TypeId::NULL { "null".to_string() } else { self.type_name(a.ty) };
                        s.push_str(&n);
                    }
                    s.push(')');
                    let src = c.source.as_ref().map(|x| self.type_name(x.ty)).unwrap_or_else(|| "null".into());
                    self.error(e.span, bind(vm::OperationIsUndefined, &[&s, &src]));
                }
            }
            Callee::Mapping { op, .. } => {
                let info = self.ops[op.index()].ir.clone();
                let abstract_out = info.params.iter().filter(|p| p.direction == Direction::Out).map(|p| p.ty).chain(info.results.iter().map(|r| r.ty)).any(|t| matches!(self.ts.types.get(t), Type::Class(c) if { let ci = self.ts.meta.class(*c); ci.is_abstract || ci.is_interface }));
                let disjuncts_empty = info.mapping.as_ref().is_none_or(|m| m.disjuncts.is_empty());
                if abstract_out && info.is_abstract && disjuncts_empty {
                    let qn = self.mapping_qualified_name(*op);
                    self.error(e.span, bind(vm::directCallToAbstractMappingDisallowed, &[&qn]));
                }
                for (a, p) in c.args.iter().zip(info.params.iter()) {
                    if p.direction != Direction::Out {
                        continue;
                    }
                    if !matches!(a.kind, ExprKind::Local(_) | ExprKind::Prop(..) | ExprKind::Global(_) | ExprKind::IntermProp(..) | ExprKind::TupleProp(..) | ExprKind::ModelParam(_) | ExprKind::This(_)) {
                        self.error(a.span, vm::outParamNotAnLValueError);
                    }
                    if a.ty != p.ty {
                        self.error(a.span, vm::outParamNotSameTypeError);
                    }
                }
            }
            _ => {}
        }
    }

    /// `CustomOclValidationVisitor.visitIteratorExp` / OCL `visitIterateExp`
    fn validate_loop(&mut self, e: &ir::Expr, lp: &ir::Loop, ctx: &Ctx<'_>) {
        if !is_ocl_loop(lp.kind) {
            return;
        }
        let ty = e.ty;
        let st = lp.source.ty;
        let Some(body) = &lp.body else {
            self.error(e.span, bind(vm::QvtOperationalVisitorCS_iterateExpIncomplete, &[]));
            return;
        };
        if lp.kind == LoopKind::Iterate {
            return;
        }
        if ty == TypeId::NULL || lp.iterators.is_empty() {
            self.error(e.span, bind(vm::QvtOperationalVisitorCS_iterateExpIncomplete, &[]));
            return;
        }
        let predefined = self.ts.types.is_collection_like(st) || self.ts.ck().is_predefined(st);
        if !predefined {
            return;
        }
        let bool_ty = TypeId::BOOLEAN;
        if matches!(lp.kind, LoopKind::ForAll | LoopKind::Exists | LoopKind::IsUnique) && ty != bool_ty {
            self.error(e.span, bind(vm::QvtOperationalVisitorCS_TypeConformanceIteratorResult_ERROR_, &[]));
            return;
        }
        if lp.kind == LoopKind::Collect {
            let seq_src = matches!(self.ts.types.get(st), Type::Collection(CollKind::Sequence | CollKind::OrderedSet, _) | Type::List(_));
            if seq_src {
                if !matches!(self.ts.types.get(ty), Type::Collection(CollKind::Sequence, _)) {
                    self.error(e.span, bind(vm::QvtOperationalVisitorCS_TypeConformanceCollectSequence_ERROR_, &[]));
                    return;
                }
            } else if !matches!(self.ts.types.get(ty), Type::Collection(CollKind::Bag, _)) {
                self.error(e.span, bind(vm::QvtOperationalVisitorCS_TypeConformanceCollectBag_ERROR_, &[]));
                return;
            }
        }
        if matches!(lp.kind, LoopKind::Select | LoopKind::Reject) {
            let list_ok = matches!(self.ts.types.get(st), Type::List(_)) && matches!(self.ts.types.get(ty), Type::Collection(CollKind::Sequence, _));
            if !list_ok && !self.ts.ck().exact(ty, st) {
                self.error(e.span, bind(vm::QvtOperationalVisitorCS_TypeConformanceSelectReject_ERROR_, &[]));
                return;
            }
        }
        if matches!(lp.kind, LoopKind::Select | LoopKind::Reject | LoopKind::ForAll | LoopKind::Any | LoopKind::Exists | LoopKind::One) && body.ty != bool_ty {
            self.error(e.span, bind(vm::QvtOperationalVisitorCS_TypeConformanceIteratorBodyBoolean_ERROR_, &[]));
            return;
        }
        if !self.ts.types.is_collection_like(st) {
            self.error(e.span, bind(vm::QvtOperationalVisitorCS_IteratorSource_ERROR_, &[]));
            return;
        }
        if lp.kind == LoopKind::Closure && !matches!(self.ts.types.get(ty), Type::Collection(CollKind::Set | CollKind::OrderedSet, _)) {
            self.error(e.span, bind(vm::QvtOperationalVisitorCS_TypeConformanceClosure_ERROR_, &[]));
            return;
        }
        let max = if matches!(lp.kind, LoopKind::ForAll | LoopKind::Exists) { 2 } else { 1 };
        if lp.iterators.len() > max {
            return;
        }
        let elem = self.ts.types.element(st).unwrap_or(TypeId::NULL);
        for it in &lp.iterators {
            let t = self.local_ty(ctx, *it);
            if !self.ts.ck().exact(t, elem) {
                self.error(e.span, bind(vm::QvtOperationalVisitorCS_TypeConformanceIteratorExpLoopVar_ERROR_, &[]));
                return;
            }
        }
    }

    /// `visitInstantiationExp` (classes and data types)
    fn validate_instantiation(&mut self, e: &ir::Expr, n: &ir::New, ctx: &Ctx<'_>) {
        let t = n.ty;
        if t == TypeId::NULL {
            return;
        }
        let class = match self.ts.types.get(t) {
            Type::Class(c) => Some(*c),
            _ => None,
        };
        if n.constructor.is_some() {
            if let Some(c) = class {
                let ci = self.ts.meta.class(c);
                if ci.is_abstract || ci.is_interface {
                    let tn = self.qualified_name(t);
                    self.error(e.span, bind(vm::QvtOperationalVisitorCS_canNotInstantiateAbstractType, &[&tn]));
                }
            }
            return;
        }
        if let Some(c) = class {
            if n.args.is_empty() {
                let ci = self.ts.meta.class(c);
                if ci.is_abstract || ci.is_interface {
                    let tn = self.qualified_name(t);
                    self.error(e.span, bind(vm::QvtOperationalVisitorCS_canNotInstantiateAbstractType, &[&tn]));
                }
                return;
            }
        }
        if self.ts.types.is_collection_like(t) {
            // an abstract collection type (Collection(T)) cannot be instantiated
            if matches!(self.ts.types.get(t), Type::Collection(CollKind::Collection, _)) {
                let tn = self.qualified_name(t);
                self.error(e.span, bind(vm::QvtOperationalVisitorCS_canNotInstantiateAbstractType, &[&tn]));
            }
        } else if class.is_none() && self.is_data_type(t) {
            let string_arg = n.args.len() == 1 && self.is_string_type(n.args[0].ty);
            if !string_arg {
                let args: Vec<String> = n.args.iter().map(|a| self.expr_to_string(a, ctx)).collect();
                let tn = self.qualified_name(t);
                self.error(e.span, bind(vm::QvtOperationalValidationVisitor_invalidParamsForDatatypeInstantiation, &[&format!("[{}]", args.join(", ")), &tn]));
            }
        } else if class.is_none() {
            let tn = self.qualified_name(t);
            self.error(e.span, bind(vm::QvtOperationalValidationVisitor_invalidInstantiatedType, &[&tn]));
        }
    }

    fn is_data_type(&self, t: TypeId) -> bool {
        !matches!(self.ts.types.get(t), Type::Class(_) | Type::Module(_) | Type::ModelType(_) | Type::Null)
    }

    fn is_string_type(&self, t: TypeId) -> bool {
        match self.ts.types.get(t) {
            Type::String => true,
            Type::DataType(d) => self.ts.meta.data_type(*d).instance_class_name.as_deref() == Some("java.lang.String"),
            _ => false,
        }
    }

    /// A rough `toString` of an argument expression for messages.
    fn expr_to_string(&self, e: &ir::Expr, ctx: &Ctx<'_>) -> String {
        match &e.kind {
            ExprKind::Local(l) => ctx.locals.get(l.index()).map(|x| x.name.clone()).unwrap_or_default(),
            ExprKind::Real(r) => format!("{r:?}"),
            ExprKind::Str(s) => format!("'{s}'"),
            ExprKind::Int(i) => i.to_string(),
            ExprKind::Bool(b) => b.to_string(),
            ExprKind::Null => "null".into(),
            _ => String::new(),
        }
    }

    fn validate_new_transformation(&mut self, e: &ir::Expr, m: ModuleId, args: &[ir::Expr]) {
        if !self.modules[m.index()].blackbox {
            let entry = self.main_operation(m);
            let ok = entry.is_some_and(|o| self.ops[o.index()].ir.kind == OpKind::Entry && self.ops[o.index()].ir.params.is_empty());
            if !ok {
                let n = self.modules[m.index()].name.clone();
                self.error(e.span, bind(vm::QvtOperationalValidationVisitor_ParameterlessMainExpected, &[&n]));
            }
        }
        let formal: Vec<TypeId> = self.modules[m.index()].model_params.iter().map(|p| p.ty).collect();
        if args.len() == formal.len() {
            for (a, f) in args.iter().zip(formal.iter()) {
                let compatible = match (self.ts.types.get(*f).clone(), self.ts.types.get(a.ty).clone()) {
                    (Type::ModelType(x), Type::ModelType(y)) => self.compatible_model_types(x, y),
                    _ => false,
                };
                if !compatible {
                    let an = self.type_name(a.ty);
                    let fname = self.type_name(*f);
                    self.error(a.span, bind(vm::QvtOperationalValidationVisitor_incompatibleArgumentModelType, &[&an, &fname]));
                }
            }
        } else {
            let n = self.modules[m.index()].name.clone();
            let actual: Vec<String> = args.iter().map(|a| self.type_name(a.ty)).collect();
            let formal_s: Vec<String> = formal.iter().map(|t| self.type_name(*t)).collect();
            self.error(e.span, bind(vm::QvtOperationalValidationVisitor_unresolvedTransformationSignature, &[&n, &format!("({})", actual.join(", ")), &format!("({})", formal_s.join(", "))]));
        }
    }

    /// `checkCompatibleModelType` (as written in Eclipse: compares the first model type's URIs
    /// with themselves after the size check).
    fn compatible_model_types(&self, a: u32, b: u32) -> bool {
        let pa = &self.model_types[a as usize].decl.packages;
        let pb = &self.model_types[b as usize].decl.packages;
        if pa.len() < pb.len() {
            return false;
        }
        pa.iter().all(|p| self.ts.meta.package(*p).ns_uri.is_some())
    }
}

fn is_ocl_loop(k: LoopKind) -> bool {
    matches!(
        k,
        LoopKind::Select
            | LoopKind::Reject
            | LoopKind::Collect
            | LoopKind::CollectNested
            | LoopKind::ForAll
            | LoopKind::Exists
            | LoopKind::One
            | LoopKind::Any
            | LoopKind::IsUnique
            | LoopKind::SortedBy
            | LoopKind::Closure
            | LoopKind::Iterate
    )
}

fn anc_of(k: &ExprKind) -> Anc {
    match k {
        ExprKind::Block(_) => Anc::Block,
        ExprKind::If(..) => Anc::If,
        ExprKind::Switch(..) => Anc::Switch,
        ExprKind::While(..) => Anc::ImpLoop,
        ExprKind::VarInit { .. } => Anc::VarInit,
        ExprKind::Assign(_) => Anc::Assign,
        ExprKind::Compute(..) => Anc::Compute,
        ExprKind::Object(_) => Anc::OpBody,
        ExprKind::Try(_) | ExprKind::Log(_) | ExprKind::Assert(_) | ExprKind::Return(_) | ExprKind::Raise(..) | ExprKind::New(_) | ExprKind::NewTransformation(..) | ExprKind::Resolve(_) => Anc::Imperative,
        _ => Anc::Ocl,
    }
}

/// Calls `f` on the direct children of an expression.
fn for_each_child(e: &ir::Expr, mut f: impl FnMut(&ir::Expr)) {
    use crate::ir::CollPart;
    match &e.kind {
        ExprKind::Coll(_, parts) | ExprKind::List(parts) => {
            for p in parts {
                match p {
                    CollPart::Item(x) => f(x),
                    CollPart::Range(a, b) => {
                        f(a);
                        f(b);
                    }
                }
            }
        }
        ExprKind::Dict(parts) => {
            for (k, v) in parts {
                f(k);
                f(v);
            }
        }
        ExprKind::Tuple(parts) => {
            for (_, v) in parts {
                f(v);
            }
        }
        ExprKind::Prop(s, _) | ExprKind::TupleProp(s, _) | ExprKind::IntermProp(s, _) => f(s),
        ExprKind::Call(c) => {
            if let Some(s) = &c.source {
                f(s);
            }
            for a in &c.args {
                f(a);
            }
        }
        ExprKind::Loop(l) => {
            f(&l.source);
            if let Some((_, i)) = &l.acc {
                f(i);
            }
            if let Some(b) = &l.body {
                f(b);
            }
            if let Some(c) = &l.cond {
                f(c);
            }
        }
        ExprKind::Let(_, i, b) => {
            f(i);
            f(b);
        }
        ExprKind::If(c, t, el) => {
            f(c);
            f(t);
            if let Some(x) = el {
                f(x);
            }
        }
        ExprKind::Block(b) => b.iter().for_each(f),
        ExprKind::VarInit { init, .. } => {
            if let Some(i) = init {
                f(i);
            }
        }
        ExprKind::Assign(a) => {
            match &a.target {
                ir::LValue::Prop(s, _) | ir::LValue::IntermProp(s, _) | ir::LValue::TupleProp(s, _) => f(s),
                _ => {}
            }
            f(&a.value);
        }
        ExprKind::While(c, b) => {
            f(c);
            f(b);
        }
        ExprKind::Compute(_, i, b) => {
            if let Some(i) = i {
                f(i);
            }
            f(b);
        }
        ExprKind::Switch(alts, els) => {
            for (c, b) in alts {
                f(c);
                f(b);
            }
            if let Some(x) = els {
                f(x);
            }
        }
        ExprKind::Return(v) => {
            if let Some(v) = v {
                f(v);
            }
        }
        ExprKind::Log(l) => {
            for x in [&l.message, &l.element, &l.level, &l.when].into_iter().flatten() {
                f(x);
            }
        }
        ExprKind::Assert(a) => {
            f(&a.cond);
            if let Some(l) = &a.log {
                for x in [&l.message, &l.element, &l.level, &l.when].into_iter().flatten() {
                    f(x);
                }
            }
        }
        ExprKind::Raise(_, a) => {
            if let Some(a) = a {
                f(a);
            }
        }
        ExprKind::Try(t) => {
            f(&t.body);
            for c in &t.catches {
                f(&c.body);
            }
        }
        ExprKind::Object(o) => o.body.iter().for_each(f),
        ExprKind::New(n) => n.args.iter().for_each(f),
        ExprKind::NewTransformation(_, args) => args.iter().for_each(f),
        ExprKind::Resolve(r) => {
            if let Some(s) = &r.source {
                f(s);
            }
            if let Some(c) = &r.cond {
                f(c);
            }
        }
        _ => {}
    }
}

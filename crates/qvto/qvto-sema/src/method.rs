//! Operation bodies (`visitMappingMethodCS`: constructors, mappings, helpers/queries, `main`),
//! mapping extensions, intermediate classes, tags and the deprecated `rename`.

use crate::analyzer::Sema;
use crate::env::{Scope, VarEntry, VarRef};
use crate::expr::{Ex, Pos};
use crate::ir::{self, Direction, ExprKind, ModuleId, OpId};
use crate::messages::{bind, validation as vm};
use crate::module::{CtxType, has_body, is_disjunctive, method_decl, scoped_parts};
use crate::ops::PropRef;
use crate::units::{MethodCs, ModuleCs, path_span};
use qvto_model::{Classifier, FeatureKind};
use qvto_syntax::ast;
use qvto_syntax::{Span, TokenKind};
use qvto_types::{Type, TypeId};

impl<'r> Sema<'r> {
    /// Starts the operation environment: the module level, then the operation's level with the
    /// context (implicit), the parameters and, for mappings and several results, the results
    /// (`QvtOperationalEnv.defineOperationParameters`).
    fn begin_operation(&mut self, module: ModuleId, op: OpId) {
        self.begin_code(module, Some(op));
        let info = &self.ops[op.index()];
        self.fun.locals = info.ir.locals.clone();
        self.fun.extra = self.op_extras[op.index()].clone();
        let mut scope = Scope::default();
        let is_mapping = info.ir.kind == ir::OpKind::Mapping;
        if let Some(s) = info.ir.self_local {
            scope.vars.push(VarEntry {
                name: "self".into(),
                var: VarRef::Local(s),
                explicit: false,
                named: true,
            });
        }
        let add = |name: &str, l: ir::LocalId, explicit: bool, scope: &mut Scope| {
            if !scope.vars.iter().any(|v| v.named && v.name == name) {
                scope.vars.push(VarEntry {
                    name: name.to_string(),
                    var: VarRef::Local(l),
                    explicit,
                    named: true,
                });
            }
        };
        for p in &info.ir.params {
            add(&p.name, p.local, true, &mut scope);
        }
        if is_mapping || info.ir.results.len() > 1 {
            for r in &info.ir.results {
                add(&r.name, r.local, true, &mut scope);
            }
        }
        self.fun.scopes.push(scope);
    }

    /// Stores the operation's variable slots back.
    fn end_operation(&mut self, op: OpId) {
        let locals = std::mem::take(&mut self.fun.locals);
        let extra = std::mem::take(&mut self.fun.extra);
        self.ops[op.index()].ir.locals = locals;
        self.op_extras[op.index()] = extra;
        self.end_code();
    }

    /// `visitMappingMethodCS`
    pub(crate) fn visit_method(&mut self, module: ModuleId, mcs: MethodCs<'_>, op: OpId) {
        let d = method_decl(mcs, &self.src());
        self.begin_operation(module, op);
        match mcs {
            MethodCs::Constructor(c) => self.visit_constructor(c, op, &d),
            MethodCs::Mapping(m) => self.visit_mapping_rule(m, op, &d),
            MethodCs::Helper(h) => {
                let (stmts, simple) = match &h.body {
                    ast::OpBody::None => (None, false),
                    ast::OpBody::Expr(e) => (Some(std::slice::from_ref(e)), true),
                    ast::OpBody::Block(b) => (Some(&b.stmts[..]), false),
                };
                self.visit_query(op, &d, stmts, simple);
            }
            MethodCs::Entry(e) => {
                let stmts = e.body.as_ref().map(|b| &b.stmts[..]);
                self.visit_query(op, &d, stmts, false);
            }
        }
        if self.modules[module.index()].blackbox {
            self.error(
                d.header_node(),
                bind(
                    vm::QvtOperationalVisitorCS_operationDefinitionInsideBlackboxModule,
                    &[&d.repr()],
                ),
            );
        }
        if self.ops[op.index()].ir.is_blackbox && !is_disjunctive(mcs) {
            let handlers = self.blackbox_handlers(op);
            let n = handlers.len();
            if n == 1 {
                // the implementation the evaluator will call (`getBlackboxCallHandler`)
                self.ops[op.index()].ir.blackbox_impl = handlers.into_iter().next();
            }
            if n == 0 {
                self.warning(
                    d.header_node(),
                    bind(
                        vm::QvtOperationalVisitorCS_noBlackboxImplementationFound,
                        &[&d.repr()],
                    ),
                );
            } else if n > 1 {
                self.warning(
                    d.header_node(),
                    bind(
                        vm::QvtOperationalVisitorCS_ambiguousBlackboxImplementationFound,
                        &[&d.repr()],
                    ),
                );
            }
        }
        self.end_operation(op);
    }

    /// The Java implementations (their `java_method`s) of a `blackbox` operation declared in
    /// QVTo (`BlackboxRegistry.getBlackboxCallHandler`, `OperationMatcher.matchOperation`): in
    /// the blackbox units the module imports, else in all registered units.
    fn blackbox_handlers(&mut self, op: OpId) -> Vec<String> {
        let module = self.ops[op.index()].module;
        let mut native: Vec<(std::rc::Rc<crate::blackbox::BlackboxUnit>, String)> = Vec::new();
        let me = &self.modules[module.index()];
        for &m in me.by_extension.iter().chain(me.by_access.iter()) {
            let u = self.modules[m.index()].unit;
            if u == usize::MAX {
                continue;
            }
            if let Some(bb) = &self.units[u].source.blackbox {
                let name = self.modules[m.index()].name.clone();
                if !native
                    .iter()
                    .any(|(b, n)| std::rc::Rc::ptr_eq(b, bb) && *n == name)
                {
                    native.push((bb.clone(), name));
                }
            }
        }
        let candidates: Vec<(std::rc::Rc<crate::blackbox::BlackboxUnit>, Option<String>)> =
            if native.is_empty() {
                self.resolver
                    .blackbox_units()
                    .into_iter()
                    .filter_map(|u| u.blackbox.map(|b| (b, None)))
                    .collect()
            } else {
                native.into_iter().map(|(b, n)| (b, Some(n))).collect()
            };
        let mut found = Vec::new();
        for (unit, only) in candidates {
            if unit.load_error {
                continue;
            }
            for bm in &unit.modules {
                if only.as_ref().is_some_and(|n| *n != bm.name) {
                    continue;
                }
                for bop in &bm.ops {
                    if self.blackbox_matches(op, bm, bop) {
                        found.push(bop.java_method.clone());
                    }
                }
            }
        }
        found
    }

    fn blackbox_matches(
        &mut self,
        op: OpId,
        bm: &crate::blackbox::BlackboxModule,
        b: &crate::blackbox::BlackboxOp,
    ) -> bool {
        let info = self.ops[op.index()].ir.clone();
        if b.name != info.name {
            return false;
        }
        let bctx = b
            .context
            .as_deref()
            .filter(|c| *c != "-")
            .map(|c| self.parse_bb_type(c));
        match (info.context, bctx) {
            (None, None) => {
                if !bm.library {
                    return false;
                }
            }
            (Some(c), Some(bc)) => {
                if !self.ts.ck().exact(c, bc) {
                    return false;
                }
            }
            _ => return false,
        }
        if info.params.len() != b.params.len() {
            return false;
        }
        for (p, bp) in info.params.iter().zip(&b.params) {
            let t = self.parse_bb_type(&bp.ty);
            if !self.ts.ck().exact(p.ty, t) {
                return false;
            }
        }
        let bret = if b.result.is_empty() || b.result == "-" {
            TypeId::OCL_VOID
        } else {
            self.parse_bb_type(&b.result)
        };
        match info.results.first() {
            None => bret == TypeId::OCL_VOID,
            Some(r) => self.ts.ck().exact(bret, r.ty),
        }
    }

    /// `visitConstructorCS`
    fn visit_constructor(
        &mut self,
        c: &ast::Constructor,
        op: OpId,
        d: &crate::module::MethodDecl<'_>,
    ) {
        let info = &self.ops[op.index()];
        let ctx = info.ir.context;
        match ctx {
            None => self.error(c.decl_span, vm::ContextlessConstructor),
            Some(t) => {
                let tn = self.classifier_name(t);
                if tn != d.name_text {
                    self.error(c.decl_span, vm::ConstructorNameMismatch);
                }
            }
        }
        self.delete_var("self");
        let rt = ctx.unwrap_or(TypeId::NULL);
        let l = self
            .fun
            .new_local("result", rt, ir::LocalKind::Result, c.decl_span);
        self.fun.extra[l.index()].param = Some(crate::env::ParamInfo {
            direction: Direction::Out,
            mapping_param: false,
            extent: None,
        });
        self.add_var("result", VarRef::Local(l), false);
        self.ops[op.index()].ir.result_local = Some(l);
        if let Some(b) = &c.body {
            let stmts = self.statements(&b.stmts, false);
            self.ops[op.index()].ir.body = Some(stmts.into_iter().map(|x| x.e).collect());
        }
    }

    /// `visitMappingQueryCS` (helpers, queries, `main`)
    fn visit_query(
        &mut self,
        op: OpId,
        d: &crate::module::MethodDecl<'_>,
        stmts: Option<&[ast::Expr]>,
        simple: bool,
    ) {
        let info = &self.ops[op.index()];
        if info.contextual && info.ir.context_direction != Direction::In {
            let span = d.name.map(|n| n.span).unwrap_or(d.decl_span);
            self.error(span, vm::ContextParamMustBeIn);
        }
        let mut body: Option<Vec<Ex>> = None;
        if let Some(stmts) = stmts {
            body = Some(self.statements(stmts, true));
        }
        let nres = self.ops[op.index()].ir.results.len();
        if nres <= 1 {
            let ret = if nres == 0 {
                self.ops[op.index()].ret
            } else {
                self.ops[op.index()].ir.results[0].ty
            };
            let last = body.as_ref().and_then(|b| b.last());
            if ret != TypeId::OCL_VOID
                && let Some(last) = last
            {
                let lt = last.ty();
                let det = last.det;
                if lt != TypeId::NULL && !self.ts.ck().assignable(ret, lt) && !det {
                    let a = self.full_name(lt);
                    let b = self.full_name(ret);
                    self.error(
                        d.decl_span,
                        bind(vm::bodyTypeNotCompatibleWithReturnTypeError, &[&a, &b]),
                    );
                }
            }
        }
        if !simple
            && nres == 1
            && let Some(b) = &body
            && b.len() == 1
            && !b[0].det
        {
            self.warning(d.decl_span, vm::useReturnExpForOperationResult);
        }
        if let Some(b) = body {
            self.ops[op.index()].ir.body = Some(b.into_iter().map(|x| x.e).collect());
        }
    }

    /// The span of the direction keyword of a mapping declaration.
    fn direction_span(&self, d: &crate::module::MethodDecl<'_>) -> Span {
        let src = self.src();
        let end = d
            .context
            .as_ref()
            .map(|c| c.span().start)
            .or(d.name.map(|n| n.span.start))
            .unwrap_or(d.decl_span.end);
        let text = src
            .get(d.decl_span.start as usize..end as usize)
            .unwrap_or("");
        for kw in ["inout", "out", "in"] {
            if let Some(i) = find_word(text, kw) {
                let s = d.decl_span.start + i as u32;
                return Span::new(s, s + kw.len() as u32);
            }
        }
        d.decl_span
    }

    /// `visitMappingRuleCS`
    fn visit_mapping_rule(
        &mut self,
        m: &ast::Mapping,
        op: OpId,
        d: &crate::module::MethodDecl<'_>,
    ) {
        let info = &self.ops[op.index()];
        if info.contextual && info.ir.context_direction == Direction::Out {
            let span = self.direction_span(d);
            self.error(span, vm::ContextParamMustBeInOrInout);
        }
        let mut when = Vec::new();
        if let Some(b) = &m.when {
            for g in &b.stmts {
                if let Some(x) = self.visit_ocl_expression(g) {
                    if x.ty() != TypeId::BOOLEAN {
                        let tn = self.full_name(x.ty());
                        self.error(g.span, bind(vm::mappingGuardNotBooleanError, &[&tn]));
                    } else {
                        when.push(x.e);
                    }
                }
            }
        }
        let mut where_ = Vec::new();
        if let Some(b) = &m.where_ {
            for g in &b.stmts {
                if let Some(x) = self.visit_ocl_expression(g) {
                    if x.ty() != TypeId::BOOLEAN {
                        let tn = self.full_name(x.ty());
                        self.error(g.span, bind(vm::mappingWhereNotBooleanError, &[&tn]));
                    } else {
                        where_.push(x.e);
                    }
                }
            }
        }
        let mut init = Vec::new();
        if let Some(body) = &m.body
            && let Some(b) = &body.init
        {
            if let Some(s) = self.fun.scopes.last_mut() {
                s.in_init = true;
            }
            init = self
                .statements(&b.stmts, false)
                .into_iter()
                .map(|x| x.e)
                .collect();
            if let Some(s) = self.fun.scopes.last_mut() {
                s.in_init = false;
            }
        }
        if m.results.len() > 1 {
            let rt = self.ops[op.index()].ret;
            let l = self
                .fun
                .new_local("result", rt, ir::LocalKind::Result, d.decl_span);
            self.add_var("result", VarRef::Local(l), true);
            self.ops[op.index()].ir.result_local = Some(l);
        }
        let mut population = Vec::new();
        if let Some(body) = &m.body {
            // Eclipse always has a MappingBodyCS: an empty one gets its offsets from the init
            // and end sections
            let empty: &[ast::Expr] = &[];
            let (explicit, stmts) = match &body.population {
                Some(p) => (p.explicit, &p.body.stmts[..]),
                None => (false, empty),
            };
            let cs_span = match (stmts.first(), stmts.last()) {
                (Some(a), Some(b)) => a.span.to(b.span),
                _ => {
                    let start: i64 = body.init.as_ref().map(|b| b.span.end as i64).unwrap_or(-1);
                    let mut end: i64 = body.end.as_ref().map(|b| b.span.start as i64 - 1).unwrap_or(-1);
                    if start > end {
                        end = start;
                    }
                    if start < 0 {
                        Span::new(body.span.start, body.span.start)
                    } else {
                        Span::new(start as u32, end as u32 + 1)
                    }
                }
            };
            let unique_result = m.results.len() == 1;
            let implicit_ctx = m.results.is_empty()
                && d.context.is_some()
                && m.direction == Some(TokenKind::KwInout);
            if !explicit && (unique_result || implicit_ctx) {
                let vname = if unique_result { "result" } else { "self" };
                let var = ast::Name {
                    text: vname.to_string(),
                    kind: ast::NameKind::Ident,
                    span: cs_span,
                };
                let x = self.object_core(Some(&var), None, stmts, cs_span, false, Some(op));
                population.push(x.e);
            } else {
                for s in stmts {
                    let x = match &s.kind {
                        ast::ExprKind::Object(o) => {
                            Some(self.object_exp(o, s.span, true, Some(op)))
                        }
                        _ => self.visit_ocl_expression_at(s, Pos::Block),
                    };
                    if let Some(x) = x {
                        population.push(x.e);
                    }
                }
            }
        }
        let mut end = Vec::new();
        if let Some(body) = &m.body
            && let Some(b) = &body.end
        {
            end = self
                .statements(&b.stmts, false)
                .into_iter()
                .map(|x| x.e)
                .collect();
        }
        // abstract out parameters and results must be initialised in the init section
        let has_init = m.body.as_ref().is_some_and(|b| b.init.is_some());
        let check: Vec<TypeId> = {
            let info = &self.ops[op.index()].ir;
            info.params
                .iter()
                .filter(|p| p.direction == Direction::Out)
                .map(|p| p.ty)
                .chain(info.results.iter().map(|r| r.ty))
                .collect()
        };
        let is_abstract = m.qualifiers.contains(&TokenKind::KwAbstract);
        let blackbox = self.ops[op.index()].ir.is_blackbox;
        for t in check {
            if let Type::Class(c) = *self.ts.types.get(t) {
                let ci = self.ts.meta.class(c);
                if (ci.is_abstract || ci.is_interface)
                    && !has_init
                    && !blackbox
                    && !is_abstract
                    && !is_disjunctive(MethodCs::Mapping(m))
                {
                    self.error(
                        d.header_node(),
                        vm::QvtOperationalVisitorCS_AbstractTypesNotInitialized,
                    );
                }
            }
        }
        let (inherits, merges, disjuncts) = self.process_mapping_extensions(m, op, d);
        let has_body_flag = has_body(MethodCs::Mapping(m));
        let _ = has_body_flag;
        self.ops[op.index()].ir.mapping = Some(ir::MappingInfo {
            when,
            where_,
            init,
            population,
            end,
            inherits,
            merges,
            disjuncts,
        });
    }

    /// `processMappingExtensions`
    fn process_mapping_extensions(
        &mut self,
        m: &ast::Mapping,
        op: OpId,
        d: &crate::module::MethodDecl<'_>,
    ) -> (Vec<OpId>, Vec<OpId>, Vec<OpId>) {
        let mut inherits = Vec::new();
        let mut merges = Vec::new();
        let mut disjuncts = Vec::new();
        for ext in &m.extensions {
            let kind = ext.kind.spelling().unwrap_or("inherits");
            for sn in &ext.names {
                let found = self.resolve_mapping_operation_reference(sn);
                if found.len() != 1 {
                    continue;
                }
                let target = found[0];
                let repr = scoped_name_string(sn);
                let list = match kind {
                    "inherits" => {
                        if self.ops[op.index()].ir.is_blackbox {
                            self.error(sn.span, bind(vm::QvtOperationalVisitorCS_unsupportedInheritanceOnBlackboxMapping, &[&repr, &d.repr()]));
                        }
                        if self.ops[target.index()].ir.is_blackbox {
                            self.error(
                                sn.span,
                                bind(
                                    vm::QvtOperationalVisitorCS_unsupportedInheritedBlackboxMapping,
                                    &[&repr, &d.repr()],
                                ),
                            );
                        }
                        &mut inherits
                    }
                    "merges" => {
                        if self.ops[target.index()].ir.is_blackbox {
                            self.error(
                                sn.span,
                                bind(
                                    vm::QvtOperationalVisitorCS_unsupportedMergedBlackboxMapping,
                                    &[&repr, &d.repr()],
                                ),
                            );
                        }
                        &mut merges
                    }
                    _ => &mut disjuncts,
                };
                if list.contains(&target) {
                    self.error(
                        sn.span,
                        bind(
                            vm::QvtOperationalVisitorCS_duplicateMappingRereferenceInExtensionKind,
                            &[kind, &repr],
                        ),
                    );
                } else {
                    list.push(target);
                }
                let slot = match kind {
                    "inherits" => 0,
                    "disjuncts" => 1,
                    _ => 2,
                };
                self.ext_refs.entry(op).or_default()[slot].push(sn.span);
            }
        }
        (inherits, merges, disjuncts)
    }

    /// `resolveMappingOperationReference`
    pub(crate) fn resolve_mapping_operation_reference(
        &mut self,
        sn: &ast::ScopedName,
    ) -> Vec<OpId> {
        let (name, ctx) = scoped_parts(sn);
        let mut result = Vec::new();
        let mut owning: Option<TypeId> = None;
        let has_type = ctx.is_some();
        if let Some(c) = ctx {
            let t = match c {
                CtxType::Type(t) => self.type_cs_in(t, true),
                CtxType::Path(p) => {
                    let te = ast::TypeExpr {
                        kind: ast::TypeKind::Path(ast::PathName {
                            segments: p.to_vec(),
                            span: path_span(p),
                        }),
                        span: path_span(p),
                    };
                    self.type_cs_in(&te, true)
                }
            };
            owning = t;
            if let (Some(t), Some(n)) = (t, name) {
                result = self.lookup_mapping_operations(t, &n.text);
            }
        } else if let Some(n) = name {
            let m = self.fun.module.unwrap_or(self.cur_module);
            let t = self.modules[m.index()].ty;
            owning = Some(t);
            result = self.lookup_mapping_operations(t, &n.text);
        }
        if !result.is_empty() {
            let local: Vec<OpId> = result
                .iter()
                .copied()
                .filter(|o| {
                    let info = &self.ops[o.index()];
                    let owner_null = !info.contextual && false;
                    (!has_type && owner_null) || Some(info.owner) == owning
                })
                .collect();
            result = local
                .iter()
                .copied()
                .filter(|o| {
                    !self.ops[o.index()]
                        .overridden
                        .is_some_and(|ov| local.contains(&ov))
                })
                .collect();
        }
        if result.is_empty() {
            if owning.is_some() {
                self.error(
                    sn.span,
                    bind(
                        vm::QvtOperationalVisitorCS_unresolvedMappingOperationReference,
                        &[&scoped_name_string(sn)],
                    ),
                );
            }
        } else if result.len() > 1 {
            self.error(
                sn.span,
                bind(
                    vm::QvtOperationalVisitorCS_ambiguousMappingOperationReference,
                    &[&scoped_name_string(sn)],
                ),
            );
        }
        result
    }

    // ---- tags and rename -----------------------------------------------------------------------

    /// `visitTagCS`; `context`: the tag context type (the module, or an intermediate class).
    pub(crate) fn visit_tag(&mut self, t: &ast::Tag, context: Option<TypeId>) {
        let m = self.cur_module;
        let context = context.unwrap_or(self.modules[m.index()].ty);
        self.begin_code(m, None);
        let tag_id = t.id.value.clone();
        let mut value = None;
        if let Some(v) = &t.value
            && let Some(x) = self.visit_ocl_expression(v)
            && let ExprKind::Str(s) = &x.e.kind
        {
            value = Some(s.to_string());
        }
        let (name, ctx) = scoped_parts(&t.element);
        let mut element: Option<PropRef> = None;
        let mut found_classifier = false;
        if let Some(c) = ctx {
            let ty = match c {
                CtxType::Type(te) => self.type_cs_in(te, true),
                CtxType::Path(p) => {
                    let te = ast::TypeExpr {
                        kind: ast::TypeKind::Path(ast::PathName {
                            segments: p.to_vec(),
                            span: path_span(p),
                        }),
                        span: path_span(p),
                    };
                    self.type_cs_in(&te, true)
                }
            };
            if let (Some(ty), Some(n)) = (ty, name) {
                element = self.lookup_property(ty, &n.text);
            }
        } else if let Some(n) = name {
            element = self.lookup_property(context, &n.text);
            if element.is_none() {
                found_classifier = self.lookup_classifier(&[n.text.as_str()]).is_some();
            }
        }
        self.end_code();
        if element.is_none() && !found_classifier {
            self.error(
                t.element.span,
                bind(vm::unresolvedNameError, &[&scoped_name_string(&t.element)]),
            );
            return;
        }
        if let (Some(p), true, Some(v)) = (element, tag_id == "alias", value) {
            let owner = self.prop_owner(p);
            let orig = self.prop_name(p);
            self.modules[m.index()].aliases.push((v, owner, orig));
        }
    }

    /// `legacyRenameCS`
    pub(crate) fn legacy_rename(&mut self, r: &ast::Rename) {
        self.warning(r.span, vm::DeprecatedRenameStatement);
        let m = self.cur_module;
        self.begin_code(m, None);
        let Some(ty) = self.visit_type_cs(&r.ty) else {
            self.end_code();
            return;
        };
        let orig = r.new_name.value.clone();
        let new_name = r.property.text.clone();
        let Some(p) = self.lookup_property(ty, &orig) else {
            let tn = self.full_name(ty);
            self.error(
                r.new_name.span,
                bind(vm::noPropertyInTypeError, &[&orig, &tn]),
            );
            self.end_code();
            return;
        };
        if self.lookup_property(ty, &new_name).is_some() {
            let tn = self.full_name(ty);
            self.error(
                r.property.span,
                bind(vm::propertyAlreadyExistsInTypeError, &[&new_name, &tn]),
            );
            self.end_code();
            return;
        }
        self.end_code();
        let owner = self.prop_owner(p);
        let orig_name = self.prop_name(p);
        self.modules[m.index()]
            .aliases
            .push((new_name, owner, orig_name));
    }

    // ---- intermediate classes ------------------------------------------------------------------

    /// The module's package and its `_INTERMEDIATE` subpackage, created on demand; registers the
    /// `_INTERMEDIATE` model type.
    fn intermediate_package(&mut self, m: ModuleId) -> qvto_model::PackageId {
        if let Some(p) = self.modules[m.index()].interm_package {
            return p;
        }
        let name = self.modules[m.index()].name.clone();
        let mp = self.ts.meta.add_package(&name, None, None, None);
        let uri = "http://www.eclipse.org/qvt/1.0.0/Operational/Expressions/_INTERMEDIATE";
        let p =
            self.ts
                .meta
                .add_package("_INTERMEDIATE", Some(uri), Some("_intermediate"), Some(mp));
        self.modules[m.index()].interm_package = Some(p);
        let idx = self.model_types.len() as u32;
        let ty = self.ts.types.intern(Type::ModelType(idx));
        self.ts.model_type_infos.push(qvto_types::ModelTypeInfo {
            name: "_INTERMEDIATE".into(),
            module: m.0,
        });
        self.model_types.push(crate::analyzer::ModelTypeState {
            decl: ir::ModelTypeDecl {
                name: "_INTERMEDIATE".into(),
                module: m,
                packages: vec![p],
                strict: false,
                where_: None,
                ty,
            },
            span: Span::default(),
        });
        self.register_model_type(m, ir::ModelTypeId(idx));
        p
    }

    /// `visitIntermediateClassesCS`
    pub(crate) fn visit_intermediate_classes(&mut self, m: ModuleId, cs: &ModuleCs<'_>) {
        if cs.classifiers.is_empty() {
            return;
        }
        let is_transformation = !self.modules[m.index()].is_library;
        let mut created: Vec<(String, qvto_model::ClassId, &ast::ClassDef)> = Vec::new();
        for c in cs.classifiers.iter().copied() {
            if !is_transformation {
                let msg = if c.exception {
                    vm::ExceptionDefNotAllowed
                } else {
                    vm::IntermediateClassifierNotAllowed
                };
                self.error(c.span, bind(msg, &[]));
            }
            if created.iter().any(|(n, _, _)| *n == c.name.text) {
                self.error(c.name.span, bind(vm::DuplicateClassifier, &[&c.name.text]));
                continue;
            }
            if self.modules[m.index()].blackbox {
                self.error(
                    c.span,
                    bind(
                        vm::QvtOperationalVisitorCS_intermediateClassDefinitionInsideBlackboxModule,
                        &[&c.name.text],
                    ),
                );
            }
            let p = self.intermediate_package(m);
            let cid = self.ts.meta.add_class(p, &c.name.text, false, false);
            self.interm_class_module.insert(cid, m);
            created.push((c.name.text.clone(), cid, c));
        }
        if created.is_empty() {
            return;
        }
        self.begin_code(m, None);
        // supertypes
        for (_, cid, c) in &created {
            for te in &c.extends {
                if let ast::TypeKind::Path(p) = &te.kind
                    && p.segments.len() == 1
                    && let Some((_, ext, _)) =
                        created.iter().find(|(n, _, _)| *n == p.segments[0].text)
                {
                    self.ts.meta.class_mut(*cid).supers.push(*ext);
                    continue;
                }
                let Some(t) = self.visit_type_cs(te) else {
                    continue;
                };
                let exc = self.ts.types.class(self.ts.std.exception);
                if t == exc {
                    if !c.exception {
                        self.error(te.span, bind(vm::CannotExtendExceptionExplicitly, &[]));
                    }
                } else if !self.ts.ck().is_user_model_element(t) {
                    let tn = self.full_name(t);
                    self.error(te.span, bind(vm::InvalidClassifierForExtend, &[&tn]));
                } else if let Type::Class(sc) = *self.ts.types.get(t) {
                    self.ts.meta.class_mut(*cid).supers.push(sc);
                }
            }
            if c.exception {
                let e = self.ts.std.exception;
                self.ts.meta.class_mut(*cid).supers.push(e);
            }
        }
        // properties
        let mut props: Vec<(qvto_model::FeatureId, &ast::ClassProperty)> = Vec::new();
        for (_, cid, c) in &created {
            let mut names: Vec<(String, qvto_model::FeatureId, &ast::ClassProperty)> = Vec::new();
            for f in &c.features {
                match f {
                    ast::ClassFeature::Property(p) => {
                        let fid = self.visit_classifier_property(*cid, p);
                        names.push((p.name.text.clone(), fid, p));
                        props.push((fid, p));
                    }
                    ast::ClassFeature::Tag(_) => {}
                }
            }
            for (n, _, p) in &names {
                if names.iter().filter(|(x, _, _)| x == n).count() > 1 {
                    let cn = self.ts.meta.class(*cid).name.to_string();
                    self.error(
                        p.name.span,
                        bind(vm::DuplicateProperty, &[&format!("{cn}.{n}")]),
                    );
                }
            }
        }
        self.end_code();
        // break cycles before computing the derived tables
        let ids: Vec<qvto_model::ClassId> = created.iter().map(|(_, c, _)| *c).collect();
        let mut cyclic = Vec::new();
        for &c in &ids {
            if !self.check_class_cycle(c, &ids) {
                cyclic.push(c);
            }
        }
        for &c in &cyclic {
            self.ts
                .meta
                .class_mut(c)
                .supers
                .retain(|s| !ids.contains(s));
        }
        let _ = self.ts.meta.finish_classes();
        for (_, cid, c) in &created {
            for f in &c.features {
                if let ast::ClassFeature::Tag(t) = f {
                    let ct = self.ts.types.class(*cid);
                    self.visit_tag(t, Some(ct));
                }
            }
        }
        // initialisers and opposites
        let mut inits: Vec<(qvto_model::ClassId, qvto_model::FeatureId, ir::CodeBody)> = Vec::new();
        for (fid, p) in &props {
            if let Some(b) = self.init_classifier_property(*fid, p) {
                inits.push((self.ts.meta.feature(*fid).owner, *fid, b));
            }
        }
        for (_, cid, c) in &created {
            let ct = self.ts.types.class(*cid);
            if cyclic.contains(cid) {
                let tn = self.full_name(ct);
                self.error(c.name.span, bind(vm::CycleInIntermHierarchy, &[&tn]));
            }
            // own features hiding inherited ones
            let own: Vec<qvto_model::FeatureId> = self.ts.meta.class(*cid).features.clone();
            let all: Vec<qvto_model::FeatureId> = self.ts.meta.class(*cid).all_features.to_vec();
            for f in all {
                let fname = self.ts.meta.feature(f).name.clone();
                if let Some(o) = own.iter().find(|o| self.ts.meta.feature(**o).name == fname)
                    && *o != f
                {
                    self.error(c.name.span, bind(vm::HidingProperty, &[&fname]));
                }
            }
            let class_inits: Vec<(qvto_model::FeatureId, ir::CodeBody)> = inits
                .iter()
                .filter(|(o, _, _)| o == cid)
                .map(|(_, f, b)| (*f, b.clone()))
                .collect();
            let idx = self.interm_classes.len() as u32;
            let own: Vec<qvto_model::FeatureId> = self.ts.meta.class(*cid).features.clone();
            let feature_types = own.into_iter().map(|f| (f, self.feature_type(f))).collect();
            self.interm_classes.push(ir::IntermClass {
                module: m,
                class: *cid,
                exception: c.exception,
                inits: class_inits,
                feature_types,
                span: c.span,
            });
            self.modules[m.index()].ir.classes.push(idx);
        }
    }

    fn check_class_cycle(&self, c: qvto_model::ClassId, ids: &[qvto_model::ClassId]) -> bool {
        let mut visited = vec![c];
        self.cycle_impl(c, ids, &mut visited)
    }

    fn cycle_impl(
        &self,
        c: qvto_model::ClassId,
        ids: &[qvto_model::ClassId],
        visited: &mut Vec<qvto_model::ClassId>,
    ) -> bool {
        for s in self.ts.meta.class(c).supers.clone() {
            if !ids.contains(&s) {
                continue;
            }
            if visited.contains(&s) {
                return false;
            }
            visited.push(s);
            if !self.cycle_impl(s, ids, visited) {
                return false;
            }
        }
        true
    }

    /// A registry classifier standing for an OCL type (the feature's `eType`); types the
    /// registry cannot represent get a placeholder and the declared type is kept aside.
    fn registry_classifier(&mut self, t: TypeId) -> Classifier {
        match *self.ts.types.get(t) {
            Type::Class(c) => Classifier::Class(c),
            Type::DataType(d) => Classifier::DataType(d),
            Type::Enum(e) => Classifier::Enum(e),
            _ => {
                let name = self.type_name(t);
                let p = self.placeholder_package();
                let existing = self.ts.meta.classifier_by_name(p, &name);
                match existing {
                    Some(c) => c,
                    None => Classifier::DataType(self.ts.meta.add_data_type(p, &name, None, false)),
                }
            }
        }
    }

    fn placeholder_package(&mut self) -> qvto_model::PackageId {
        if let Some(p) = self.placeholder_pkg {
            return p;
        }
        let p = self.ts.meta.add_package(
            "_qvto_types",
            Some("urn:qvto-rs:types"),
            Some("_qvto_types"),
            None,
        );
        self.placeholder_pkg = Some(p);
        p
    }

    /// `visitClassifierPropertyCS`
    fn visit_classifier_property(
        &mut self,
        c: qvto_model::ClassId,
        p: &ast::ClassProperty,
    ) -> qvto_model::FeatureId {
        let ty = self.visit_type_cs(&p.ty);
        let is_ref = ty.is_some_and(|t| matches!(self.ts.types.get(t), Type::Class(_)));
        let cl = match ty {
            Some(t) => self.registry_classifier(t),
            None => {
                let p0 = self.placeholder_package();
                match self.ts.meta.classifier_by_name(p0, "OclVoid") {
                    Some(c) => c,
                    None => {
                        Classifier::DataType(self.ts.meta.add_data_type(p0, "OclVoid", None, false))
                    }
                }
            }
        };
        let kind = if is_ref {
            FeatureKind::Reference
        } else {
            FeatureKind::Attribute
        };
        let f = self.ts.meta.add_feature(c, &p.name.text, kind, cl);
        if let Some(t) = ty {
            self.interm_feature_types.insert(f, t);
        }
        let mut seen: Vec<String> = Vec::new();
        for st in &p.stereotypes {
            let q = st.text.as_str();
            if q == "id" {
                if !is_ref {
                    self.ts.meta.feature_mut(f).id = true;
                } else {
                    self.error(
                        st.span,
                        bind(vm::IntermClassifier_unapplicableStereotypeQualifier, &[q]),
                    );
                }
            } else {
                self.warning(
                    st.span,
                    bind(vm::IntermClassifier_unknownStereotypeQualifier, &[q]),
                );
            }
            if seen.iter().any(|s| s == q) {
                self.warning(
                    st.span,
                    bind(vm::IntermClassifier_duplicatedStereotypeQualifier, &[q]),
                );
            }
            seen.push(q.to_string());
        }
        let mut keys: Vec<TokenKind> = Vec::new();
        let mut composes: Option<Span> = None;
        let mut static_: Option<Span> = None;
        let src = self.src();
        for k in &p.keys {
            let key = k.spelling().unwrap_or("");
            let span = find_key_span(&src, p.span, key).unwrap_or(p.span);
            match key {
                "composes" => {
                    if is_ref {
                        self.ts.meta.feature_mut(f).containment = true;
                    } else {
                        self.error(
                            span,
                            bind(vm::IntermClassifier_referenceOnlyFeatureKey, &[key]),
                        );
                    }
                    composes = Some(span);
                }
                "references" => {
                    if is_ref {
                        self.ts.meta.feature_mut(f).containment = false;
                    } else {
                        self.error(
                            span,
                            bind(vm::IntermClassifier_referenceOnlyFeatureKey, &[key]),
                        );
                    }
                }
                "readonly" => self.ts.meta.feature_mut(f).changeable = false,
                "derived" => self.warning(
                    span,
                    bind(vm::IntermClassifier_unsupportedFeatureKey, &[key]),
                ),
                "static" => static_ = Some(span),
                _ => {}
            }
            if keys.contains(k) {
                self.warning(
                    span,
                    bind(vm::IntermClassifier_duplicatedFeatureKey, &[key]),
                );
            }
            keys.push(*k);
        }
        if let (Some(a), Some(b)) = (composes, static_) {
            self.error(
                a,
                bind(
                    vm::IntermClassifier_incompatibleFeatureKeys,
                    &["composes", "static"],
                ),
            );
            self.error(
                b,
                bind(
                    vm::IntermClassifier_incompatibleFeatureKeys,
                    &["composes", "static"],
                ),
            );
        }
        if let Some(mult) = &p.multiplicity {
            let (l, u) = self.visit_multiplicity(mult);
            let fi = self.ts.meta.feature_mut(f);
            fi.lower = l;
            fi.upper = u;
        }
        self.ts.meta.feature_mut(f).ordered = p.ordered;
        f
    }

    /// `visitMultiplicityDefCS`
    fn visit_multiplicity(&mut self, m: &ast::Multiplicity) -> (i32, i32) {
        let to_i = |b: ast::MultBound| -> i64 {
            match b {
                ast::MultBound::Int(i) => i,
                ast::MultBound::Many => -1,
            }
        };
        let lower = to_i(m.lower);
        let upper = match m.upper {
            Some(u) => to_i(u),
            None => to_i(m.lower),
        };
        let lower_is_star = matches!(m.lower, ast::MultBound::Many) && m.upper.is_some();
        let err = if lower_is_star || lower > i32::MAX as i64 || upper > i32::MAX as i64 {
            Some("For input string: \"*\"")
        } else if lower < 0 && m.upper.is_some() {
            Some(vm::IntermClassifier_multiplicityInvalidLowerBound)
        } else if upper >= 0 && lower > upper {
            Some(vm::IntermClassifier_multiplicityInvalidRange)
        } else if upper == 0 && lower == 0 {
            Some(vm::IntermClassifier_multiplicityEmptyRange)
        } else {
            None
        };
        if let Some(e) = err {
            self.error(m.span, e);
            return (0, 1);
        }
        if m.upper.is_none() && matches!(m.lower, ast::MultBound::Many) {
            return (0, -1);
        }
        (lower as i32, upper as i32)
    }

    /// `initClassifierPropertyCS`: the initialiser (in the module environment) and the opposite.
    fn init_classifier_property(
        &mut self,
        f: qvto_model::FeatureId,
        p: &ast::ClassProperty,
    ) -> Option<ir::CodeBody> {
        let m = self.cur_module;
        let mut body = None;
        if let Some(e) = &p.init {
            self.begin_code(m, None);
            let x = self.visit_ocl_expression(e);
            if let Some(x) = &x {
                let declared = self.interm_feature_types.get(&f).copied();
                if declared.is_none() {
                    self.interm_feature_types.insert(f, x.ty());
                    let cl = self.registry_classifier(x.ty());
                    self.ts.meta.feature_mut(f).ty = cl;
                }
                let dt = self.feature_type(f);
                if !self.ts.ck().assignable(dt, x.ty()) {
                    let a = self.full_name(dt);
                    let b = self.full_name(x.ty());
                    self.error(p.span, bind(vm::TypeMismatchError, &[&a, &b]));
                }
            }
            let locals = std::mem::take(&mut self.fun.locals);
            self.end_code();
            if let Some(x) = x {
                body = Some(ir::CodeBody {
                    source: ir::SourceId(self.cur_unit as u32 + 1),
                    locals,
                    self_local: None,
                    expr: x.e,
                });
            }
        }
        if let Some(o) = &p.opposite {
            let is_ref = self.ts.meta.feature(f).kind == FeatureKind::Reference;
            let ospan = o.name.span;
            if is_ref {
                if let Some(mult) = &o.multiplicity {
                    self.visit_multiplicity(mult);
                }
                let fty = self.interm_feature_types.get(&f).copied();
                match fty.map(|t| self.ts.types.get(t).clone()) {
                    Some(Type::Class(c)) => {
                        let all = self.ts.meta.class(c).all_features.to_vec();
                        let opp = all.into_iter().find(|x| {
                            self.ts.meta.feature(*x).name.as_ref() == o.name.text
                                && self.ts.meta.feature(*x).kind == FeatureKind::Reference
                        });
                        match opp {
                            Some(x) => self.ts.meta.feature_mut(f).opposite = Some(x),
                            None => {
                                let tn = self.full_name(fty.unwrap());
                                self.error(
                                    ospan,
                                    bind(
                                        vm::IntermClassifier_invalideOppositeName,
                                        &[&o.name.text, &tn],
                                    ),
                                );
                            }
                        }
                    }
                    _ => {
                        let tn = fty.map(|t| self.full_name(t)).unwrap_or_default();
                        self.error(
                            ospan,
                            bind(vm::IntermClassifier_invalideOppositeType, &[&tn]),
                        );
                    }
                }
            } else {
                self.error(
                    ospan,
                    bind(vm::IntermClassifier_oppositeOnlyForReferences, &[]),
                );
            }
        }
        body
    }
}

/// `getStringRepresentation(ScopedNameCS)`
pub(crate) fn scoped_name_string(sn: &ast::ScopedName) -> String {
    let (name, ctx) = scoped_parts(sn);
    let mut s = String::new();
    if let Some(c) = ctx {
        s.push_str(&c.string());
        s.push_str("::");
    }
    if let Some(n) = name {
        s.push_str(&n.text);
    }
    s
}

fn find_word(text: &str, w: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut from = 0;
    while let Some(i) = text[from..].find(w) {
        let s = from + i;
        let e = s + w.len();
        let before = s == 0 || !(b[s - 1].is_ascii_alphanumeric() || b[s - 1] == b'_');
        let after = e >= b.len() || !(b[e].is_ascii_alphanumeric() || b[e] == b'_');
        if before && after {
            return Some(s);
        }
        from = s + 1;
    }
    None
}

fn find_key_span(src: &str, within: Span, key: &str) -> Option<Span> {
    let text = src.get(within.start as usize..within.end as usize)?;
    let i = find_word(text, key)?;
    let s = within.start + i as u32;
    Some(Span::new(s, s + key.len() as u32))
}

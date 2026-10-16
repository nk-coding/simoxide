//! Module structure: Eclipse's `QvtOperationalVisitorCS.visitUnitCS` passes (module headers,
//! model types, imports and module usages, intermediate classes, operation headers with
//! collision checks and virtual tables, module properties, operation bodies).

use crate::analyzer::{ModelTypeState, Sema, STDLIB_MODULE};
use crate::env::{GlobalInfo, IntermPropInfo, ModelParamInfo, ModuleEnv, ModuleFeature, ParamInfo};
use crate::ir::{self, Direction, ModelParamRef, ModelTypeId, ModuleId, OpId};
use crate::messages::{bind, compiler as cm, validation as vm};
use crate::names::type_string;
use crate::ops::{OpInfo, OpRef};
use crate::units::{path_span, path_string, MethodCs, ModuleCs};
use qvto_model::PackageId;
use qvto_syntax::ast;
use qvto_syntax::{Span, TokenKind};
use qvto_types::check::{ModelTypeInfo, ModuleTypeInfo};
use qvto_types::rel::*;
use qvto_types::{StdOp, StdOwner, Type, TypeId};

/// Kinds of operation collisions (`CollisionKind`).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Collision {
    AlreadyDefined(OpRef),
    Overrides(OpRef),
    VirtualSubtype(OpRef),
    VirtualSupertype(OpRef),
}

fn dir_of(k: Option<TokenKind>) -> Direction {
    match k {
        Some(TokenKind::KwOut) => Direction::Out,
        Some(TokenKind::KwInout) => Direction::InOut,
        _ => Direction::In,
    }
}

/// Information about a method's declaration in uniform form.
pub(crate) struct MethodDecl<'u> {
    pub name: Option<&'u ast::Name>,
    pub name_text: String,
    /// The context type (`Ctx::name`): a type expression or a path.
    pub context: Option<CtxType<'u>>,
    pub direction: Option<TokenKind>,
    pub params: &'u [ast::Param],
    pub results: &'u [ast::Param],
    pub qualifiers: &'u [TokenKind],
    pub decl_span: Span,
    pub span: Span,
    pub is_query: bool,
}

#[derive(Clone)]
pub(crate) enum CtxType<'u> {
    Type(&'u ast::TypeExpr),
    Path(&'u [ast::Name]),
}

impl CtxType<'_> {
    pub fn span(&self) -> Span {
        match self {
            CtxType::Type(t) => t.span,
            CtxType::Path(p) => path_span(p),
        }
    }
    pub fn string(&self) -> String {
        match self {
            CtxType::Type(t) => type_string(t),
            CtxType::Path(p) => path_string(p, "::"),
        }
    }
}

pub(crate) fn scoped_parts(s: &ast::ScopedName) -> (Option<&ast::Name>, Option<CtxType<'_>>) {
    let name = s.names.last();
    let ctx = if let Some(t) = &s.context_type {
        Some(CtxType::Type(t))
    } else if s.names.len() > 1 {
        Some(CtxType::Path(&s.names[..s.names.len() - 1]))
    } else {
        None
    };
    (name, ctx)
}

pub(crate) fn method_decl<'u>(m: MethodCs<'u>, src: &str) -> MethodDecl<'u> {
    let mut d = method_decl_raw(m);
    d.decl_span = eclipse_decl_span(m, &d, src);
    d
}

/// The span of Eclipse's `MappingDeclarationCS`: from the `helper`/`query`/`constructor`
/// keyword (mappings: the direction keyword, else `mapping`) to the end of the signature.
fn eclipse_decl_span(m: MethodCs<'_>, d: &MethodDecl<'_>, src: &str) -> Span {
    let q1 = d.decl_span;
    let name_start = d.context.as_ref().map(|c| c.span().start).or(d.name.map(|n| n.span.start));
    let Some(ns) = name_start else {
        return q1;
    };
    let kw: Option<&str> = match m {
        MethodCs::Helper(h) => h.kind.spelling(),
        MethodCs::Constructor(_) => Some("constructor"),
        MethodCs::Mapping(mp) => match mp.direction {
            Some(k) => k.spelling(),
            None => Some("mapping"),
        },
        MethodCs::Entry(_) => None,
    };
    let Some(kw) = kw else {
        return q1;
    };
    let text = src.get(q1.start as usize..ns as usize).unwrap_or("");
    let start = match rfind_word(text, kw) {
        Some(i) => q1.start + i as u32,
        None => q1.start,
    };
    let end = match d.results.last() {
        Some(r) => r.span.end,
        None => {
            let name_end = d.name.map(|n| n.span.end).unwrap_or(ns);
            match_paren_end(src, name_end, q1.end).unwrap_or(q1.end)
        }
    };
    Span::new(start, end.max(start))
}

fn rfind_word(text: &str, w: &str) -> Option<usize> {
    let b = text.as_bytes();
    let mut end = text.len();
    while let Some(i) = text[..end].rfind(w) {
        let e = i + w.len();
        let before = i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_');
        let after = e >= b.len() || !(b[e].is_ascii_alphanumeric() || b[e] == b'_');
        if before && after {
            return Some(i);
        }
        end = i;
    }
    None
}

/// The end of the parenthesised list starting at the first `(` after `from`.
fn match_paren_end(src: &str, from: u32, limit: u32) -> Option<u32> {
    let b = src.as_bytes();
    let mut i = from as usize;
    let lim = (limit as usize).min(b.len());
    while i < lim && b[i] != b'(' {
        i += 1;
    }
    let mut depth = 0;
    while i < lim {
        match b[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i as u32 + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn method_decl_raw<'u>(m: MethodCs<'u>) -> MethodDecl<'u> {
    match m {
        MethodCs::Helper(h) => {
            let (name, context) = scoped_parts(&h.name);
            MethodDecl {
                name,
                name_text: name.map(|n| n.text.clone()).unwrap_or_default(),
                context,
                direction: None,
                params: &h.params,
                results: &h.results,
                qualifiers: &h.qualifiers,
                decl_span: h.decl_span,
                span: h.span,
                is_query: h.kind == TokenKind::KwQuery,
            }
        }
        MethodCs::Entry(e) => MethodDecl {
            name: None,
            name_text: "main".into(),
            context: None,
            direction: None,
            params: &e.params,
            results: &[],
            qualifiers: &[],
            decl_span: e.decl_span,
            span: e.span,
            is_query: false,
        },
        MethodCs::Mapping(m) => {
            let (name, context) = scoped_parts(&m.name);
            MethodDecl {
                name,
                name_text: name.map(|n| n.text.clone()).unwrap_or_default(),
                context,
                direction: m.direction,
                params: &m.params,
                results: &m.results,
                qualifiers: &m.qualifiers,
                decl_span: m.decl_span,
                span: m.span,
                is_query: false,
            }
        }
        MethodCs::Constructor(c) => {
            let (name, context) = scoped_parts(&c.name);
            MethodDecl {
                name,
                name_text: name.map(|n| n.text.clone()).unwrap_or_default(),
                context,
                direction: None,
                params: &c.params,
                results: &[],
                qualifiers: &c.qualifiers,
                decl_span: c.decl_span,
                span: c.span,
                is_query: false,
            }
        }
    }
}

impl<'u> MethodDecl<'u> {
    /// `getMethodNameProblemNodeCS`
    pub fn name_node(&self) -> Span {
        let mut s = self.decl_span;
        if let Some(c) = &self.context {
            s = c.span();
        }
        if let Some(n) = self.name {
            s.end = n.span.end;
        }
        s
    }

    /// `getMethodHeaderProblemNodeCS`
    pub fn header_node(&self) -> Span {
        let mut s = self.decl_span;
        if let Some(c) = &self.context {
            s.end = c.span().end;
        }
        if let Some(n) = self.name {
            s.end = n.span.end;
        }
        if let Some(p) = self.params.last() {
            s.end = p.span.end;
        }
        if let Some(r) = self.results.last() {
            s.end = r.span.end;
        }
        s
    }

    /// `getMappingStringRepresentation`
    pub fn repr(&self) -> String {
        let mut s = String::new();
        if let Some(c) = &self.context {
            s.push_str(&c.string());
            s.push_str("::");
        }
        s.push_str(&self.name_text);
        s
    }
}

impl<'r> Sema<'r> {
    /// QVTo's library module `Stdlib` (`ModuleId(0)`) with its additional operations.
    pub(crate) fn create_stdlib_module(&mut self) {
        let ty = self.ts.types.intern(Type::Module(0));
        self.ts.module_infos.push(ModuleTypeInfo {
            name: "Stdlib".into(),
            transformation: false,
        });
        let mut env = new_module_env("Stdlib", true, usize::MAX, ty, Span::default());
        env.by_extension.clear();
        // QvtTypeResolverImpl of the library environment: owners in definition order
        for owner in qvto_types::stdlib::owners(qvto_types::Section::Qvto) {
            let t = self.stdlib.owner_type(&mut self.ts.types, owner);
            let ops: Vec<StdOp> = qvto_types::owner_ops(qvto_types::Section::Qvto, owner).to_vec();
            env.additional_types.push(t);
            env.std_additional.push((t, ops));
        }
        env.defines_ocl_any_features = true;
        for &op in qvto_types::owner_ops(qvto_types::Section::Class, StdOwner::Stdlib) {
            env.eoperations.push(OpRef::Std(op, StdOwner::Stdlib));
        }
        self.modules.push(env);
        self.sources.push(ir::Source {
            name: "Stdlib".into(),
            qualified_name: "Stdlib".into(),
            path: None,
            lines: qvto_syntax::LineIndex::new(""),
        });
    }

    pub(crate) fn new_module_pub(&mut self, name: &str, library: bool, unit: usize, header: Span) -> ModuleId {
        self.new_module(name, library, unit, header)
    }

    fn new_module(&mut self, name: &str, library: bool, unit: usize, header: Span) -> ModuleId {
        let id = ModuleId(self.modules.len() as u32);
        let ty = self.ts.types.intern(Type::Module(id.0));
        self.ts.module_infos.push(ModuleTypeInfo {
            name: name.to_string(),
            transformation: !library,
        });
        let mut env = new_module_env(name, library, unit, ty, header);
        env.ir.source = ir::SourceId(unit as u32 + 1);
        self.modules.push(env);
        id
    }

    fn rename_module(&mut self, m: ModuleId, name: &str) {
        self.modules[m.index()].name = name.to_string();
        self.modules[m.index()].ir.name = name.to_string();
        self.ts.module_infos[m.index()].name = name.to_string();
    }

    /// `visitUnitCS`
    pub(crate) fn analyze_unit(&mut self, u: usize, modules: &[ModuleCs<'_>], _text: &str) {
        let src = &self.units[u].source;
        self.sources.push(ir::Source {
            name: src.file_name.clone(),
            qualified_name: src.qualified_name.clone(),
            path: src.path.clone(),
            lines: self.units[u].lines.clone(),
        });
        // keep SourceId = unit + 1 (0 is Stdlib)
        while self.sources.len() < u + 2 {
            self.sources.push(ir::Source {
                name: String::new(),
                qualified_name: String::new(),
                path: None,
                lines: qvto_syntax::LineIndex::new(""),
            });
        }
        let mut ids = Vec::new();
        let mut names: Vec<String> = Vec::new();
        // pass 1: headers
        for cs in modules {
            let m = cs.module;
            let first = m.name.first().map(|n| n.text.clone()).unwrap_or_default();
            let library = m.kind == TokenKind::KwLibrary;
            let id = self.new_module(&first, library, u, m.header_span);
            self.units[u].modules.push(id);
            ids.push(id);
            self.cur_module = id;
            self.visit_module_headers(id, cs);
            let name = self.modules[id.index()].name.clone();
            if names.contains(&name) {
                let span = if m.name.is_empty() { m.span } else { path_span(&m.name) };
                self.error(span, bind(vm::QvtOperationalVisitorCS_transformationNameDuplicates, &[&name]));
            }
            names.push(name);
        }
        // pass 2: imports and usages
        for (cs, &id) in modules.iter().zip(&ids) {
            self.cur_module = id;
            self.imports_cs(u, id, cs);
        }
        let sorted = self.check_module_loops(&ids, modules);
        // pass 3: intermediate classes, tags, renamings
        for &i in &sorted {
            let (cs, id) = (&modules[i], ids[i]);
            self.cur_module = id;
            self.visit_intermediate_classes(id, cs);
            for t in &cs.tags {
                self.visit_tag(t, None);
            }
            for r in &cs.renamings {
                self.legacy_rename(r);
            }
        }
        // pass 4: method headers
        let mut method_maps = vec![Vec::new(); modules.len()];
        for &i in &sorted {
            self.cur_module = ids[i];
            method_maps[i] = self.visit_method_headers(ids[i], &modules[i]);
        }
        // pass 5: properties
        for &i in &sorted {
            self.cur_module = ids[i];
            self.create_module_properties(ids[i], &modules[i]);
        }
        // pass 6: bodies
        for &i in &sorted {
            self.cur_module = ids[i];
            self.visit_method_bodies(ids[i], &modules[i], &method_maps[i]);
        }
        // AST validation
        for (i, &id) in ids.iter().enumerate() {
            self.cur_module = id;
            self.problems().check_duplicate_errors = true;
            self.validate_module(id, &method_maps[i]);
            self.problems().check_duplicate_errors = false;
        }
    }

    // ---- pass 1 --------------------------------------------------------------------------------

    fn visit_module_headers(&mut self, id: ModuleId, cs: &ModuleCs<'_>) {
        for mt in &cs.metamodels {
            let Some(mtid) = self.visit_model_type(id, mt) else {
                continue;
            };
            let name = self.model_types[mtid.index()].decl.name.clone();
            self.modules[id.index()].ir.model_types.push(mtid);
            if !name.is_empty() {
                if self.modules[id.index()].model_types.iter().any(|(n, _)| *n == name) {
                    self.error(mt.name.span, bind(vm::QvtOperationalVisitorCS_modeltypeAlreadyDefined, &[&name]));
                } else {
                    self.modules[id.index()].model_types.push((name, mtid));
                }
            } else {
                self.register_model_type(id, mtid);
                self.warning(mt.span, bind(vm::QvtOperationalVisitorCS_modeltypeDeprecatedSyntax, &[]));
            }
            self.check_model_type_constraints(id, mtid, mt);
        }
        self.visit_transformation_header(id, cs.module);
    }

    pub(crate) fn register_model_type(&mut self, m: ModuleId, mt: ModelTypeId) {
        let name = self.model_types[mt.index()].decl.name.clone();
        let reg = &mut self.modules[m.index()].model_types;
        if let Some(e) = reg.iter_mut().find(|(n, _)| *n == name) {
            e.1 = mt;
        } else {
            reg.push((name, mt));
        }
    }

    /// `visitModelTypeCS`
    fn visit_model_type(&mut self, module: ModuleId, mt: &ast::ModelType) -> Option<ModelTypeId> {
        let name = mt.name.text.clone();
        if let Some(c) = &mt.compliance {
            if !c.value.is_empty() && c.value != "strict" {
                self.warning(c.span, bind(vm::QvtOperationalVisitorCS_unsupportedMetamodelComplianceKind, &[&c.value]));
            }
        }
        let mut packages: Vec<PackageId> = Vec::new();
        let n = mt.packages.len();
        let mut i = 0;
        while i < n {
            let pr = &mt.packages[i];
            let mut resolved: Option<PackageId> = None;
            if let Some(uri) = &pr.uri {
                resolved = self.resolve_metamodel(Some(&uri.value), &[], uri.span);
                if resolved.is_none() {
                    i += 1;
                    continue;
                }
            }
            if let Some(path) = &pr.path {
                if !path.segments.is_empty() {
                    let mname = path_string(&path.segments, "::");
                    let segs: Vec<&str> = path.segments.iter().map(|s| s.text.as_str()).collect();
                    resolved = match resolved {
                        None => self.resolve_metamodel(None, &segs, path.span),
                        Some(r) => self.check_metamodel_path(r, &segs, &mname, path.span),
                    };
                }
            }
            if let Some(r) = resolved {
                if packages.contains(&r) {
                    let pi = self.ts.meta.package(r);
                    let mname = pi.ns_uri.as_deref().unwrap_or(&pi.name).to_string();
                    self.warning(pr.span, bind(vm::DuplicateMetamodelImport, &[&mname]));
                }
                packages.push(r);
                for rest in &mt.packages[i + 1..] {
                    self.warning(rest.span, bind(vm::QvtOperationalVisitorCS_metamodelSinglePackageRefSupported, &[]));
                }
                break;
            }
            i += 1;
        }
        if packages.is_empty() {
            return None;
        }
        let idx = self.model_types.len() as u32;
        let ty = self.ts.types.intern(Type::ModelType(idx));
        self.ts.model_type_infos.push(ModelTypeInfo {
            name: name.clone(),
            module: module.0,
        });
        self.model_types.push(ModelTypeState {
            decl: ir::ModelTypeDecl {
                name,
                module,
                packages,
                strict: mt.compliance.as_ref().is_some_and(|c| c.value == "strict"),
                where_: None,
                ty,
            },
            span: mt.span,
        });
        Some(ModelTypeId(idx))
    }

    /// `resolveMetamodel` (`MetamodelResolutionHelper.registerMetamodel`)
    fn resolve_metamodel(&mut self, uri: Option<&str>, path: &[&str], span: Span) -> Option<PackageId> {
        let name = if path.is_empty() {
            uri.unwrap_or("").to_string()
        } else {
            path.join("::")
        };
        let mut found: Vec<PackageId> = Vec::new();
        if let (Some(u), true) = (uri, path.is_empty()) {
            if let Some(p) = self.ts.meta.package_by_ns_uri(u) {
                found.push(p);
            } else if let Some(p) = self.try_lookup_empty_root_package(u) {
                found.push(p);
            }
        } else {
            found = self.resolve_metamodels_by_name(path);
        }
        let mut out = Vec::new();
        for mut p in found {
            if self.ts.meta.package(p).ns_uri.is_none() {
                p = self.root_package(p);
            }
            out.push(p);
            if let Some(u) = uri {
                let reg = &mut self.units[self.cur_unit].registry;
                if let Some(e) = reg.iter_mut().find(|(k, _)| k == u) {
                    e.1 = p;
                } else {
                    reg.push((u.to_string(), p));
                }
            }
        }
        if out.is_empty() {
            self.error(span, bind(vm::failedToResolveMetamodelError, &[&name]));
            return None;
        }
        if out.len() > 1 {
            let uris: Vec<String> = out
                .iter()
                .map(|p| self.ts.meta.package(*p).ns_uri.as_deref().unwrap_or("null").to_string())
                .collect();
            self.warning(span, bind(vm::QvtOperationalVisitorCS_metamodelNameAmbiguous, &[&name, &format!("[{}]", uris.join(", "))]));
        }
        Some(out[0])
    }

    fn root_package(&self, mut p: PackageId) -> PackageId {
        while let Some(s) = self.ts.meta.package(p).super_package {
            p = s;
        }
        p
    }

    /// `MetamodelRegistry.tryLookupEmptyRootPackage`
    fn try_lookup_empty_root_package(&self, uri: &str) -> Option<PackageId> {
        let (scheme_auth, rest) = split_uri(uri);
        let first = rest.split('/').find(|s| !s.is_empty())?;
        let base = format!("{scheme_auth}/{first}").to_lowercase();
        for p in self.ts.meta.packages() {
            let Some(pu) = self.ts.meta.package(p).ns_uri.as_deref() else {
                continue;
            };
            if self.ts.meta.package_by_ns_uri(pu) != Some(p) {
                continue;
            }
            if pu.to_lowercase().starts_with(&base) {
                let r = self.root_package(p);
                if self.ts.meta.package(r).ns_uri.as_deref() == Some(uri) {
                    return Some(r);
                }
            }
        }
        None
    }

    /// `MetamodelRegistry.resolveMetamodels(registry, path)`: registered root packages by name.
    fn resolve_metamodels_by_name(&self, path: &[&str]) -> Vec<PackageId> {
        let mut out = Vec::new();
        if path.is_empty() {
            return out;
        }
        for p in self.ts.meta.packages() {
            let pi = self.ts.meta.package(p);
            if pi.super_package.is_some() {
                continue;
            }
            let Some(u) = pi.ns_uri.as_deref() else {
                continue;
            };
            if self.ts.meta.package_by_ns_uri(u) != Some(p) || p == self.ts.std.package {
                continue;
            }
            if *pi.name != *path[0] {
                continue;
            }
            let mut r = Some(p);
            for n in &path[1..] {
                r = r.and_then(|x| {
                    self.ts
                        .meta
                        .package(x)
                        .subpackages
                        .iter()
                        .copied()
                        .find(|&s| *self.ts.meta.package(s).name == **n)
                });
            }
            if let Some(r) = r {
                if !out.contains(&r) {
                    out.push(r);
                }
            }
        }
        out
    }

    /// `checkMetamodelPath`
    fn check_metamodel_path(&mut self, resolved: PackageId, path: &[&str], name: &str, span: Span) -> Option<PackageId> {
        let lookup = |s: &Sema, root: PackageId| -> Option<PackageId> {
            let pi = s.ts.meta.package(root);
            if pi.super_package.is_some() || *pi.name != *path[0] {
                return None;
            }
            let mut r = Some(root);
            for n in &path[1..] {
                r = r.and_then(|x| s.ts.meta.package(x).subpackages.iter().copied().find(|&c| *s.ts.meta.package(c).name == **n));
            }
            r
        };
        if let Some(p) = lookup(self, resolved) {
            return Some(p);
        }
        let root = self.root_package(resolved);
        if root != resolved {
            if let Some(local) = lookup(self, root) {
                let mut cur = Some(local);
                while let Some(c) = cur {
                    if c == resolved {
                        return Some(local);
                    }
                    cur = self.ts.meta.package(c).super_package;
                }
            }
        }
        self.error(span, bind(vm::QvtOperationalVisitorCS_metamodelPackageRefInconsistent, &[name]));
        None
    }

    /// `checkModelTypeConstraints`
    fn check_model_type_constraints(&mut self, module: ModuleId, mt: ModelTypeId, cs: &ast::ModelType) {
        let Some(block) = &cs.where_ else {
            return;
        };
        let ty = self.model_types[mt.index()].decl.ty;
        let mut conds = Vec::new();
        self.begin_code(module, None);
        let selfl = self.fun.new_local("self", ty, ir::LocalKind::SelfVar, cs.span);
        self.fun.push_scope();
        self.add_var("self", crate::env::VarRef::Local(selfl), false);
        for c in &block.stmts {
            let e = self.visit_ocl_expression(c);
            if let Some(e) = e {
                if e.ty() != TypeId::BOOLEAN {
                    let n = self.full_name(e.ty());
                    self.error(c.span, bind(vm::ModelTypeWhereNotBooleanError, &[&n]));
                } else {
                    conds.push(e.e);
                }
            }
        }
        let locals = std::mem::take(&mut self.fun.locals);
        self.end_code();
        if !conds.is_empty() {
            let source = ir::SourceId(self.cur_unit as u32 + 1);
            let span = block.span;
            self.model_types[mt.index()].decl.where_ = Some(ir::CodeBody {
                source,
                locals,
                self_local: Some(selfl),
                expr: ir::Expr {
                    kind: ir::ExprKind::Block(conds),
                    ty: TypeId::OCL_VOID,
                    span,
                },
            });
        }
    }

    /// The header's CS span: from the `transformation`/`library` keyword (qualifiers excluded).
    pub(crate) fn eclipse_header(&self, m: &ast::Module) -> Span {
        let src = self.src();
        let end = m.name.first().map(|n| n.span.start).unwrap_or(m.header_span.end);
        let kw = m.kind.spelling().unwrap_or("transformation");
        let text = src.get(m.header_span.start as usize..end as usize).unwrap_or("");
        match rfind_word(text, kw) {
            Some(i) => Span::new(m.header_span.start + i as u32, m.header_span.end),
            None => m.header_span,
        }
    }

    /// `visitTransformationHeaderCS`
    fn visit_transformation_header(&mut self, id: ModuleId, m: &ast::Module) {
        let qname = path_string(&m.name, ".");
        let head_to_path = Span::new(self.eclipse_header(m).start, path_span(&m.name).end);
        for q in &m.qualifiers {
            match q {
                TokenKind::KwBlackbox => {
                    self.modules[id.index()].blackbox = true;
                    self.modules[id.index()].ir.blackbox = true;
                }
                TokenKind::KwAbstract | TokenKind::KwStatic => {
                    let qn = q.spelling().unwrap_or("");
                    self.warning(head_to_path, bind(vm::QvtOperationalVisitorCS_unsupportedQualifierOnTransformation, &[qn, &qname]));
                }
                _ => {}
            }
        }
        for d in duplicates(&m.qualifiers) {
            let qn = d.spelling().unwrap_or("");
            self.warning(head_to_path, bind(vm::QvtOperationalVisitorCS_duplicateQualifierOnTransformation, &[qn, &qname]));
        }
        if m.name.len() > 1 {
            self.error(path_span(&m.name), bind(vm::moduleNameMustBeSimpleIdentifierError, &[&qname]));
        }
        let simple = m.name.last().map(|n| n.text.clone()).unwrap_or_default();
        self.rename_module(id, &simple);
        if m.kind == TokenKind::KwTransformation {
            self.visit_transformation_signature(id, m);
        } else {
            self.visit_library_signature(id, m);
        }
        if let Some(r) = &m.refines {
            self.warning(r.span, bind(vm::QvtOperationalVisitorCS_transfRefineNotSupported, &[]));
        }
        // setContextModule: model parameters as variables (done when scopes are built)
    }

    fn visit_transformation_signature(&mut self, id: ModuleId, m: &ast::Module) {
        let mut names: Vec<String> = Vec::new();
        for (i, p) in m.params.iter().flatten().enumerate() {
            let tcs = &p.ty.ty;
            let mut mt: Option<ModelTypeId> = None;
            let mut simple = false;
            if let ast::TypeKind::Path(path) = &tcs.kind {
                simple = path.segments.len() == 1;
                if let Some(first) = path.segments.first() {
                    mt = self.modules[id.index()].model_types.iter().find(|(n, _)| *n == first.text).map(|(_, t)| *t);
                }
            }
            if mt.is_none() || !simple {
                self.error(tcs.span, bind(vm::QvtOperationalVisitorCS_transfParamWrongType, &[]));
            }
            let name = match &p.name {
                Some(n) => n.text.clone(),
                None => format!("${i}"),
            };
            let dir = dir_of(p.direction);
            if names.contains(&name) {
                let span = match &p.name {
                    Some(n) if !n.text.is_empty() => n.span,
                    _ => p.span,
                };
                self.error(span, bind(vm::SameParamName, &[&name]));
            }
            names.push(name.clone());
            let ty = match mt {
                Some(t) => self.model_types[t.index()].decl.ty,
                None => TypeId::NULL,
            };
            self.modules[id.index()].model_params.push(ModelParamInfo {
                name: name.clone(),
                direction: dir,
                model_type: mt,
                ty,
                span: p.span,
            });
        }
    }

    fn visit_library_signature(&mut self, id: ModuleId, m: &ast::Module) {
        let mut used: Vec<ModelTypeId> = Vec::new();
        for p in m.params.iter().flatten() {
            if let Some(e) = &p.ty.extent {
                self.error(e.span, vm::QvtOperationalVisitorCS_LibrarySignatureErrorModelExtentSpecified);
            }
            if p.direction.is_some() {
                self.error(p.span, vm::QvtOperationalVisitorCS_LibrarySignatureErrorDirectionKindSpecified);
            }
            if p.name.is_some() {
                self.error(p.span, vm::QvtOperationalVisitorCS_LibrarySignatureErrorParameterNameSpecified);
            }
            let mut mt = None;
            if let ast::TypeKind::Path(path) = &p.ty.ty.kind {
                if path.segments.len() == 1 {
                    let first = &path.segments[0].text;
                    mt = self.modules[id.index()].model_types.iter().find(|(n, _)| n == first).map(|(_, t)| *t);
                    if let Some(t) = mt {
                        if used.contains(&t) {
                            self.error(p.span, bind(vm::QvtOperationalVisitorCS_LibrarySignatureErrorDuplicateModelType, &[first]));
                        } else {
                            used.push(t);
                        }
                    }
                }
            }
            if mt.is_none() {
                self.error(p.span, bind(vm::QvtOperationalVisitorCS_transfParamWrongType, &[]));
            }
        }
    }

    // ---- pass 2 --------------------------------------------------------------------------------

    /// `importsCS`
    fn imports_cs(&mut self, u: usize, id: ModuleId, cs: &ModuleCs<'_>) {
        // qualified unit name -> modules of the imported unit
        let mut import_map: Vec<(String, Vec<ModuleId>)> = Vec::new();
        for imp in &cs.imports {
            let q = path_string(&imp.unit, ".");
            let segs: Vec<String> = imp.unit.iter().map(|n| n.text.clone()).collect();
            let envs = self.imported_modules(u, &segs);
            if envs.is_empty() {
                continue;
            }
            if let Some(e) = import_map.iter_mut().find(|(k, _)| *k == q) {
                e.1 = envs;
            } else {
                import_map.push((q.clone(), envs));
            }
            if imp.library {
                self.warning(path_span(&imp.unit), bind(vm::DeprecatedLibraryImportStatement, &[&q]));
            }
        }
        // usages: the header's, then implicit extensions for the other imports
        let mut usages: Vec<Usage> = Vec::new();
        for mu in &cs.module.usages {
            for r in &mu.refs {
                usages.push(Usage {
                    access: mu.kind == TokenKind::KwAccess,
                    path: r.path.segments.iter().map(|n| n.text.clone()).collect(),
                    params: r.params.clone(),
                    ref_span: r.span,
                    usage_span: mu.span,
                    path_span: r.path.span,
                    in_header: true,
                });
            }
        }
        let mut used: Vec<String> = Vec::new();
        for u2 in &usages {
            let mname = u2.path.join(".");
            if import_map.iter().any(|(k, _)| *k == mname) {
                used.push(mname);
            } else if u2.path.len() == 1 {
                'done: for (k, envs) in &import_map {
                    for &e in envs {
                        if self.modules[e.index()].name == u2.path[0] {
                            used.push(k.clone());
                            break 'done;
                        }
                    }
                }
            }
        }
        for imp in &cs.imports {
            let q = path_string(&imp.unit, ".");
            let segs: Vec<String> = imp.unit.iter().map(|n| n.text.clone()).collect();
            let envs = self.imported_modules(u, &segs);
            if envs.is_empty() {
                continue;
            }
            if used.contains(&q) {
                continue;
            }
            used.push(q.clone());
            let envs = import_map.iter().find(|(k, _)| *k == q).map(|(_, e)| e.clone()).unwrap_or_default();
            for e in envs {
                let mut path = segs.clone();
                path.push(self.modules[e.index()].name.clone());
                usages.push(Usage {
                    access: false,
                    path,
                    params: None,
                    ref_span: path_span(&imp.unit),
                    usage_span: imp.span,
                    path_span: path_span(&imp.unit),
                    in_header: false,
                });
            }
        }
        for usage in &usages {
            self.module_usage(u, id, usage, &import_map);
        }
    }

    fn imported_modules(&self, u: usize, segs: &[String]) -> Vec<ModuleId> {
        self.units[u]
            .imported
            .iter()
            .find(|(s, _)| s == segs)
            .map(|(_, i)| self.units[*i].modules.clone())
            .unwrap_or_default()
    }

    /// `moduleUsageCS` for one module reference.
    fn module_usage(&mut self, u: usize, id: ModuleId, usage: &Usage, import_map: &[(String, Vec<ModuleId>)]) {
        let mut module_envs: Vec<ModuleId> = Vec::new();
        let mut legacy: Vec<ModuleId> = Vec::new();
        let n = usage.path.len();
        let unit_fq = usage.path[..n.saturating_sub(1)].join(".");
        let module_fq = usage.path.join(".");
        let short = usage.path[n - 1].clone();
        if unit_fq == self.units[u].source.qualified_name || unit_fq.is_empty() {
            module_envs.extend(self.units[u].modules.iter().copied());
        }
        let fq = import_map.iter().find(|(k, _)| *k == module_fq).map(|(_, e)| e.clone());
        if let Some(e) = &fq {
            module_envs.extend(e.iter().copied());
            legacy.extend(e.iter().copied());
        }
        if n > 1 {
            if let Some((_, e)) = import_map.iter().find(|(k, _)| *k == unit_fq) {
                module_envs.extend(e.iter().copied());
                legacy.extend(e.iter().copied());
            }
        }
        if fq.is_none() && n == 1 {
            for (_, e) in import_map {
                module_envs.extend(e.iter().copied());
            }
        }
        let mut candidates: Vec<ModuleId> = module_envs.iter().copied().filter(|&e| self.modules[e.index()].name == short).collect();
        if candidates.is_empty() {
            candidates = legacy.clone();
        }
        let mut resolved: Vec<ModuleId> = Vec::new();
        for c in candidates {
            let Some(params) = &usage.params else {
                if !resolved.contains(&c) {
                    resolved.push(c);
                }
                continue;
            };
            if params.is_empty() {
                if !resolved.contains(&c) {
                    resolved.push(c);
                }
                continue;
            }
            let mut refs: Vec<ModelTypeId> = Vec::new();
            for p in params {
                if let ast::TypeKind::Path(path) = &p.ty.ty.kind {
                    if let Some(first) = path.segments.first() {
                        if let Some((_, t)) = self.modules[id.index()].model_types.iter().find(|(n, _)| *n == first.text) {
                            refs.push(*t);
                        }
                    }
                }
            }
            let imported: Vec<ModelTypeId> = if self.modules[c.index()].is_library {
                self.modules[c.index()].ir.model_types.clone()
            } else {
                self.modules[c.index()].model_params.iter().filter_map(|p| p.model_type).collect()
            };
            if refs.len() == imported.len() {
                let ok = refs.iter().zip(&imported).all(|(a, b)| self.compatible_model_type(*a, *b));
                if ok && !resolved.contains(&c) {
                    resolved.push(c);
                }
            }
        }
        let path_str = usage.path.join("::");
        match resolved.len() {
            1 => {
                let imported = resolved[0];
                if imported != id {
                    let me = &mut self.modules[id.index()];
                    let list = if usage.access { &mut me.by_access } else { &mut me.by_extension };
                    if !list.contains(&imported) {
                        list.push(imported);
                    }
                }
                if !usage.access {
                    if self.modules[imported.index()].blackbox {
                        self.error(usage.ref_span, bind(vm::QvtOperationalVisitorCS_extendsOfBlackboxModule, &[&path_str]));
                    }
                    if !self.modules[id.index()].is_library && !self.modules[imported.index()].is_library {
                        self.validate_imported_signature(id, imported, usage.usage_span);
                    }
                    if self.modules[id.index()].is_library && !self.modules[imported.index()].is_library && usage.in_header {
                        self.error(usage.ref_span, bind(vm::QvtOperationalVisitorCS_libraryCantExtendNonLibrary, &[&path_str]));
                    }
                }
                self.modules[id.index()].module_imports.push((imported, !usage.access));
                let kind = if usage.access { ir::UsageKind::Access } else { ir::UsageKind::Extends };
                self.modules[id.index()].ir.usages.push(ir::ModuleUsage {
                    kind,
                    module: imported,
                    span: usage.usage_span,
                });
            }
            0 => self.error(usage.ref_span, bind(vm::UnresolvedModuleReference, &[&path_str])),
            _ => self.error(usage.ref_span, bind(vm::AmbiguousModuleReference, &[&path_str])),
        }
        let _ = usage.path_span;
    }

    fn compatible_model_type(&self, a: ModelTypeId, b: ModelTypeId) -> bool {
        if a == b {
            return true;
        }
        let pa = &self.model_types[a.index()].decl.packages;
        let pb = &self.model_types[b.index()].decl.packages;
        pb.iter().all(|p| pa.contains(p))
    }

    /// `validateImportedSignature`
    fn validate_imported_signature(&mut self, id: ModuleId, imported: ModuleId, span: Span) {
        let mine = self.modules[id.index()].model_params.clone();
        let theirs = self.modules[imported.index()].model_params.clone();
        let mut processed = vec![false; theirs.len()];
        let mut considered = vec![false; mine.len()];
        for (ti, ip) in theirs.iter().enumerate() {
            for (mi, p) in mine.iter().enumerate() {
                if considered[mi] {
                    continue;
                }
                if self.model_param_equal(p, ip, true) {
                    considered[mi] = true;
                    processed[ti] = true;
                    break;
                }
            }
        }
        for (ti, ip) in theirs.iter().enumerate() {
            if processed[ti] {
                continue;
            }
            let mut found = false;
            for (mi, p) in mine.iter().enumerate() {
                if considered[mi] {
                    continue;
                }
                if self.model_param_equal(p, ip, false) {
                    considered[mi] = true;
                    found = true;
                    break;
                }
            }
            if !found {
                let a = self.modules[imported.index()].name.clone();
                let b = self.modules[id.index()].name.clone();
                self.warning(span, bind(vm::QvtOperationalVisitorCS_incompatibleTransfSignature, &[&a, &b]));
                return;
            }
        }
    }

    fn model_param_equal(&self, p: &ModelParamInfo, ip: &ModelParamInfo, strict: bool) -> bool {
        if p.direction == Direction::In && ip.direction != Direction::In {
            return false;
        }
        if !strict {
            return true;
        }
        match (p.model_type, ip.model_type) {
            (Some(a), Some(b)) => self.compatible_model_type(a, b),
            _ => false,
        }
    }

    /// `checkModuleLoops`: reports import cycles among the unit's modules, removes the cyclic
    /// imports, and returns an order in which imported modules come first.
    fn check_module_loops(&mut self, ids: &[ModuleId], modules: &[ModuleCs<'_>]) -> Vec<usize> {
        let mut sorted: Vec<usize> = (0..ids.len()).collect();
        for (i, &id) in ids.iter().enumerate() {
            let mut to_visit = vec![id];
            let mut visited: Vec<ModuleId> = Vec::new();
            let mut top_import: Vec<(ModuleId, (ModuleId, bool))> = Vec::new();
            let mut loops: Vec<(ModuleId, bool)> = Vec::new();
            while let Some(v) = to_visit.pop() {
                if visited.contains(&v) {
                    continue;
                }
                visited.push(v);
                for &imp in &self.modules[v.index()].module_imports.clone() {
                    let top = if v != id {
                        top_import.iter().find(|(k, _)| *k == v).map(|(_, t)| *t).unwrap_or(imp)
                    } else {
                        imp
                    };
                    if imp.0 == id {
                        let a = self.modules[id.index()].name.clone();
                        let b = self.modules[v.index()].name.clone();
                        let span = self.eclipse_header(modules[i].module);
                        self.error(span, bind(cm::cyclicImportError, &[&a, &b]));
                        loops.push(top);
                    }
                    if !visited.contains(&imp.0) {
                        to_visit.push(imp.0);
                        if !top_import.iter().any(|(k, _)| *k == imp.0) {
                            top_import.push((imp.0, top));
                        }
                    }
                }
            }
            for l in loops {
                let me = &mut self.modules[id.index()];
                if let Some(pos) = me.module_imports.iter().position(|x| *x == l) {
                    me.module_imports.remove(pos);
                }
            }
            let mut head = Vec::new();
            let mut tail = Vec::new();
            for &s in &sorted {
                if s == i {
                    continue;
                }
                if visited.contains(&ids[s]) {
                    head.push(s);
                } else {
                    tail.push(s);
                }
            }
            sorted = head;
            sorted.push(i);
            sorted.extend(tail);
        }
        sorted
    }

    // ---- pass 4 --------------------------------------------------------------------------------

    /// `visitMethodHeaders`: declares the operations; returns them with their syntax.
    fn visit_method_headers<'u>(&mut self, id: ModuleId, cs: &ModuleCs<'u>) -> Vec<(MethodCs<'u>, OpId)> {
        let mut map: Vec<(MethodCs<'u>, OpId)> = Vec::new();
        for &mcs in &cs.methods {
            let d = method_decl(mcs, &self.src());
            let kind = match mcs {
                MethodCs::Constructor(_) => ir::OpKind::Constructor,
                MethodCs::Mapping(_) => {
                    if d.name_text == "main" {
                        let end = d.decl_span.end;
                        self.warning(Span::new(mcs.span().start, end), bind(vm::EntryOp_DisallowedDeclAsMapping, &[]));
                    }
                    ir::OpKind::Mapping
                }
                _ if d.name_text == "main" => ir::OpKind::Entry,
                MethodCs::Helper(h) if h.kind == TokenKind::KwQuery => ir::OpKind::Query,
                _ => ir::OpKind::Helper,
            };
            let op = self.visit_mapping_declaration(id, &d, kind, mcs);
            self.define_imperative_operation(op);
            map.push((mcs, op));
        }
        self.patch_signatureless(id);
        let mut entry_found = false;
        for &(mcs, op) in &map {
            let d = method_decl(mcs, &self.src());
            let qname = d.repr();
            let is_ctor = kind_of(self, op) == ir::OpKind::Constructor;
            for q in d.qualifiers {
                let qn = q.spelling().unwrap_or("");
                match q {
                    TokenKind::KwAbstract => {
                        if kind_of(self, op) == ir::OpKind::Mapping {
                            self.ops[op.index()].ir.is_abstract = true;
                        }
                        if is_ctor {
                            self.warning(d.name_node(), bind(vm::QvtOperationalVisitorCS_unsupportedQualifierOnConstructor, &[qn, &qname]));
                        }
                    }
                    TokenKind::KwBlackbox => {
                        if has_body(mcs) {
                            let msg = if is_ctor {
                                bind(vm::QvtOperationalVisitorCS_blackboxConstructorWithBodyNotAllowed, &[&qname])
                            } else {
                                bind(vm::QvtOperationalVisitorCS_blackboxOperationWithBodyNotAllowed, &[&qname])
                            };
                            self.error(d.header_node(), msg);
                        }
                        if is_disjunctive(mcs) {
                            self.error(d.name_node(), bind(vm::QvtOperationalVisitorCS_unsupportedBlackboxQualifierOnDisjunctiveMapping, &[&qname]));
                        }
                        self.ops[op.index()].ir.is_blackbox = true;
                    }
                    TokenKind::KwStatic => {
                        let msg = if is_ctor {
                            vm::QvtOperationalVisitorCS_unsupportedQualifierOnConstructor
                        } else {
                            vm::QvtOperationalVisitorCS_unsupportedQualifierOnOperation
                        };
                        self.warning(d.name_node(), bind(msg, &[qn, &qname]));
                    }
                    _ => {}
                }
            }
            for dup in duplicates(d.qualifiers) {
                let qn = dup.spelling().unwrap_or("");
                self.warning(d.name_node(), bind(vm::QvtOperationalVisitorCS_duplicateQualifierOnOperation, &[qn, &qname]));
            }
            let name = self.ops[op.index()].ir.name.clone();
            if name == "main" && !self.modules[id.index()].model_params.is_empty() {
                self.check_main_mapping_conformance(op);
            }
            if !self.modules[id.index()].is_library && name == "main" {
                if entry_found {
                    self.error(mcs.span(), bind(vm::QvtOperationalVisitorCS_entryPointShouldBeDeclOnce, &["main"]));
                }
                entry_found = true;
            }
        }
        map
    }

    /// `DeprecatedSignaturelessTransf.patchModule`
    fn patch_signatureless(&mut self, id: ModuleId) {
        if self.modules[id.index()].is_library || !self.modules[id.index()].model_params.is_empty() {
            return;
        }
        let Some(main) = self.main_operation(id) else {
            return;
        };
        let params: Vec<(String, TypeId, Direction, bool)> = {
            let op = &self.ops[main.index()];
            op.ir
                .params
                .iter()
                .chain(op.ir.results.iter())
                .map(|p| (p.name.clone(), p.ty, p.direction, true))
                .collect()
        };
        for (i, (name, ty, dir, _)) in params.into_iter().enumerate() {
            let pkg = match *self.ts.types.get(ty) {
                Type::Class(c) => Some(self.ts.meta.class(c).package),
                Type::DataType(d) => Some(self.ts.meta.data_type(d).package),
                Type::Enum(e) => Some(self.ts.meta.enumeration(e).package),
                _ => None,
            };
            let mt_name = match pkg {
                Some(p) => self.ts.meta.package(p).name.to_string(),
                None => "<modeltype>".into(),
            };
            let idx = self.model_types.len() as u32;
            let mty = self.ts.types.intern(Type::ModelType(idx));
            self.ts.model_type_infos.push(ModelTypeInfo {
                name: mt_name.clone(),
                module: id.0,
            });
            self.model_types.push(ModelTypeState {
                decl: ir::ModelTypeDecl {
                    name: mt_name,
                    module: id,
                    packages: pkg.into_iter().collect(),
                    strict: false,
                    where_: None,
                    ty: mty,
                },
                span: Span::default(),
            });
            let mp_index = self.modules[id.index()].model_params.len() as u32;
            self.modules[id.index()].model_params.push(ModelParamInfo {
                name: format!("${name}"),
                direction: dir,
                model_type: Some(ModelTypeId(idx)),
                ty: mty,
                span: Span::default(),
            });
            self.modules[id.index()].ir.model_types.push(ModelTypeId(idx));
            // the parameter's extent
            let op = &mut self.ops[main.index()];
            let nparams = op.ir.params.len();
            let p = if i < nparams { &mut op.ir.params[i] } else { &mut op.ir.results[i - nparams] };
            if p.extent.is_none() {
                p.extent = Some(ModelParamRef { module: id, index: mp_index });
            }
        }
    }

    pub(crate) fn main_operation(&self, id: ModuleId) -> Option<OpId> {
        if let Some(e) = self.modules[id.index()].ir.entry {
            return Some(e);
        }
        for o in &self.modules[id.index()].eoperations {
            if let OpRef::User(op) = o {
                if self.ops[op.index()].ir.name == "main" {
                    return Some(*op);
                }
            }
        }
        None
    }

    /// `visitMappingDeclarationCS`: creates the operation with its parameters and results.
    fn visit_mapping_declaration(&mut self, module: ModuleId, d: &MethodDecl<'_>, kind: ir::OpKind, mcs: MethodCs<'_>) -> OpId {
        let module_ty = self.modules[module.index()].ty;
        let ctx_dir = dir_of(d.direction);
        let context = match &d.context {
            Some(CtxType::Type(t)) => Some(self.type_cs_in(t, true).unwrap_or(module_ty)),
            Some(CtxType::Path(p)) => {
                let te = ast::TypeExpr {
                    kind: ast::TypeKind::Path(ast::PathName {
                        segments: p.to_vec(),
                        span: path_span(p),
                    }),
                    span: path_span(p),
                };
                Some(self.type_cs_in(&te, true).unwrap_or(module_ty))
            }
            None => None,
        };
        let is_entry = d.name_text == "main";
        let mapping_params = is_entry || kind == ir::OpKind::Mapping;
        let mut fun = crate::env::FnState::default();
        let mut params = Vec::new();
        let mut sig = Vec::new();
        let mut extras: Vec<(ir::LocalId, ParamInfo)> = Vec::new();
        for p in d.params {
            let dir = dir_of(p.direction);
            let (ty, extent) = self.visit_type_spec(&p.ty, dir);
            let ty = ty.unwrap_or(TypeId::OCL_VOID);
            let name = p.name.as_ref().map(|n| n.text.clone()).unwrap_or_default();
            let mut ext = extent;
            if mapping_params && ext.is_none() {
                ext = if is_entry {
                    self.resolve_model_parameter_deprecated(module, ty, dir)
                } else {
                    self.resolve_model_parameter(module, ty, dir)
                };
            }
            let local = fun.new_local(&name, ty, ir::LocalKind::Param, p.span);
            extras.push((local, ParamInfo { direction: dir, mapping_param: mapping_params, extent: if mapping_params { ext } else { None } }));
            sig.push((name.clone(), ty, dir));
            params.push(ir::Param {
                name,
                direction: dir,
                ty,
                extent: if mapping_params { ext } else { None },
                local,
                span: p.span,
            });
        }
        let mut self_local = None;
        if let (Some(ct), Some(cspan)) = (context, d.context.as_ref().map(|c| c.span())) {
            let l = fun.new_local("self", ct, ir::LocalKind::SelfVar, cspan);
            let ext = if mapping_params { self.resolve_model_parameter(module, ct, ctx_dir) } else { None };
            extras.push((l, ParamInfo { direction: ctx_dir, mapping_param: mapping_params, extent: ext }));
            fun.extra[l.index()].is_context = true;
            self_local = Some(l);
        }
        let mut results = Vec::new();
        let nres = d.results.len();
        for r in d.results {
            let (ty, extent) = self.visit_type_spec(&r.ty, Direction::Out);
            let ty = ty.unwrap_or(TypeId::OCL_VOID);
            let mut name = r.name.as_ref().map(|n| n.text.clone()).unwrap_or_default();
            if nres == 1 {
                name = "result".into();
                if let Some(n) = &r.name {
                    self.error(n.span, bind(vm::QvtOperationalVisitorCS_resultParamNameNotAllowed, &[&n.text]));
                }
                if r.direction.is_some() {
                    self.error(r.span, vm::QvtOperationalVisitorCS_resultParamNameDirectionNotAllowed);
                }
            }
            let mut ext = extent;
            if mapping_params && ext.is_none() {
                ext = self.resolve_model_parameter(module, ty, Direction::Out);
            }
            let local = fun.new_local(&name, ty, ir::LocalKind::Result, r.span);
            extras.push((local, ParamInfo { direction: Direction::Out, mapping_param: mapping_params, extent: if mapping_params { ext } else { None } }));
            results.push(ir::Param {
                name,
                direction: Direction::Out,
                ty,
                extent: if mapping_params { ext } else { None },
                local,
                span: r.span,
            });
        }
        for (l, info) in extras {
            fun.extra[l.index()].param = Some(info);
        }
        let ret = match results.len() {
            0 => TypeId::OCL_VOID,
            1 => results[0].ty,
            _ => {
                let parts = results.iter().map(|r| (r.name.clone().into_boxed_str(), r.ty)).collect();
                self.ts.types.tuple(parts)
            }
        };
        let result_local = if results.len() == 1 { Some(results[0].local) } else { None };
        let id = OpId(self.ops.len() as u32);
        let source = ir::SourceId(self.cur_unit as u32 + 1);
        let mut info = OpInfo {
            ir: ir::Operation {
                module,
                kind,
                name: d.name_text.clone(),
                context,
                context_direction: ctx_dir,
                params,
                results,
                result_ty: ret,
                is_abstract: false,
                is_blackbox: has_blackbox_qualifier(d.qualifiers),
                is_static: false,
                locals: fun.locals,
                self_local,
                result_local,
                body: None,
                mapping: None,
                vtable: Vec::new(),
                overrides: None,
                source,
                decl_span: d.decl_span,
                span: d.span,
                blackbox_impl: None,
            },
            owner: context.unwrap_or(module_ty),
            contextual: context.is_some(),
            params: sig,
            ret,
            module,
            overridden: None,
            hidden: false,
            is_query: d.is_query,
            name_node: d.name_node(),
            header_node: d.header_node(),
            decl_span: d.decl_span,
            vtable: Vec::new(),
        };
        info.ir.locals.iter_mut().for_each(|_| {});
        self.ops.push(info);
        self.op_extras.push(fun.extra);
        let _ = mcs;
        id
    }

    /// `visitTypeSpecCS`: the type and the extent (`T @ m`).
    pub(crate) fn visit_type_spec(&mut self, ts: &ast::TypeSpec, dir: Direction) -> (Option<TypeId>, Option<ModelParamRef>) {
        let mut extent = None;
        if let Some(e) = &ts.extent {
            extent = self.lookup_model_parameter(&e.text, dir);
            if extent.is_none() {
                let m = self.cur_module;
                let names = self.extent_names(m, dir);
                self.error(e.span, bind(vm::QvtOperationalVisitorCS_extentWrongName, &[&e.text, &format!("[{}]", names.join(", "))]));
            }
        }
        let ty = self.type_cs(&ts.ty);
        (ty, extent)
    }

    fn lookup_model_parameter(&self, name: &str, dir: Direction) -> Option<ModelParamRef> {
        let m = self.cur_module;
        for (i, p) in self.modules[m.index()].model_params.iter().enumerate() {
            if dir != Direction::In && p.direction == Direction::In {
                continue;
            }
            if p.name == name {
                return Some(ModelParamRef { module: m, index: i as u32 });
            }
        }
        None
    }

    fn extent_names(&self, m: ModuleId, dir: Direction) -> Vec<String> {
        self.modules[m.index()]
            .model_params
            .iter()
            .filter(|p| !(dir != Direction::In && p.direction == Direction::In))
            .filter(|p| !p.name.is_empty())
            .map(|p| p.name.clone())
            .collect()
    }

    /// `isMayBelongToExtent`
    pub(crate) fn may_belong_to_extent(&self, t: TypeId) -> bool {
        if t == TypeId::OCL_VOID || t == TypeId::OCL_INVALID || t == TypeId::NULL {
            return false;
        }
        if let Type::Class(c) = *self.ts.types.get(t) {
            if self.interm_class_module.contains_key(&c) {
                return false;
            }
        }
        true
    }

    fn type_root_package(&self, t: TypeId) -> Option<PackageId> {
        let p = match *self.ts.types.get(t) {
            Type::Class(c) => self.ts.meta.class(c).package,
            Type::DataType(d) => self.ts.meta.data_type(d).package,
            Type::Enum(e) => self.ts.meta.enumeration(e).package,
            _ => return None,
        };
        Some(self.root_package(p))
    }

    /// `resolveModelParameter` (`findModelParameter`)
    pub(crate) fn resolve_model_parameter(&self, m: ModuleId, t: TypeId, dir: Direction) -> Option<ModelParamRef> {
        if !self.may_belong_to_extent(t) {
            return None;
        }
        self.find_model_parameter(m, t, |pd| dir != Direction::In && pd == Direction::In)
    }

    fn resolve_model_parameter_deprecated(&self, m: ModuleId, t: TypeId, dir: Direction) -> Option<ModelParamRef> {
        if !self.may_belong_to_extent(t) {
            return None;
        }
        self.find_model_parameter(m, t, |pd| dir == Direction::Out && pd == Direction::In)
    }

    fn find_model_parameter(&self, m: ModuleId, t: TypeId, skip: impl Fn(Direction) -> bool) -> Option<ModelParamRef> {
        let root = self.type_root_package(t);
        let params = &self.modules[m.index()].model_params;
        for (i, p) in params.iter().enumerate() {
            if skip(p.direction) {
                continue;
            }
            if let Some(mt) = p.model_type {
                let pk = &self.model_types[mt.index()].decl.packages;
                if !pk.is_empty() && root == Some(pk[0]) {
                    return Some(ModelParamRef { module: m, index: i as u32 });
                }
            }
        }
        for (i, p) in params.iter().enumerate() {
            if skip(p.direction) {
                continue;
            }
            return Some(ModelParamRef { module: m, index: i as u32 });
        }
        None
    }

    /// `checkMainMappingConformance`
    fn check_main_mapping_conformance(&mut self, op: OpId) {
        let params = self.ops[op.index()].ir.params.clone();
        let mut used: Vec<Option<ModelParamRef>> = Vec::new();
        for p in params {
            if used.contains(&p.extent) {
                self.error(p.span, bind(vm::QvtOperationalVisitorCS_extentDuplicateUse, &[]));
            }
            if p.direction != Direction::Out {
                used.push(p.extent);
            }
            match p.extent {
                None => {
                    if p.direction != Direction::In {
                        let n = self.full_name(p.ty);
                        self.error(p.span, bind(vm::QvtOperationalVisitorCS_extentFailToInfer, &[&n]));
                    }
                }
                Some(e) => {
                    let ed = self.modules[e.module.index()].model_params[e.index as usize].direction;
                    if (ed == Direction::In && p.direction != Direction::In) || (ed == Direction::Out && p.direction != Direction::Out) {
                        self.error(p.span, bind(vm::QvtOperationalVisitorCS_extentDirectionMismatch, &[]));
                    }
                }
            }
        }
    }

    /// `QvtOperationalEnv.defineImperativeOperation`
    fn define_imperative_operation(&mut self, op: OpId) {
        let module = self.ops[op.index()].module;
        let owner = self.ops[op.index()].owner;
        let contextual = self.ops[op.index()].contextual;
        let mut is_error = false;
        if let Some(c) = self.find_colliding_operation(op, owner, contextual) {
            let name = self.ops[op.index()].ir.name.clone();
            let span = self.ops[op.index()].decl_span;
            match c {
                Collision::AlreadyDefined(_) => {
                    is_error = true;
                    self.ops[op.index()].hidden = true;
                    let on = self.type_name(owner);
                    self.error(span, bind(vm::SemanticUtil_0, &[&name, &on]));
                }
                Collision::Overrides(other) => {
                    if let OpRef::User(oid) = other {
                        let oret = self.ops[oid.index()].ret;
                        let nret = self.ops[op.index()].ret;
                        if self.ts.ck().compatible(nret, oret) {
                            self.ops[op.index()].overridden = Some(oid);
                            self.ops[op.index()].ir.overrides = Some(oid);
                        } else {
                            is_error = true;
                            let om = self.ops[oid.index()].module;
                            let mn = self.modules[om.index()].name.clone();
                            self.error(span, bind(vm::OperationOverrideWithInvalidReturnType, &[&name, &mn]));
                        }
                    }
                    if !is_error {
                        let m = self.op_module(other);
                        if m.is_none() || m == Some(STDLIB_MODULE) {
                            self.warning(span, bind(vm::HidingStdlibOperationDiscouraged, &[&name]));
                        }
                    }
                }
                Collision::VirtualSubtype(other) => {
                    is_error = true;
                    self.ops[op.index()].hidden = true;
                    let r = self.op_ret(other);
                    let n = self.full_name(r);
                    self.error(span, bind(vm::ReturnSubtypeMismatch, &[&name, &n]));
                }
                Collision::VirtualSupertype(other) => {
                    is_error = true;
                    self.ops[op.index()].hidden = true;
                    let r = self.op_ret(other);
                    let n = self.full_name(r);
                    self.error(span, bind(vm::ReturnSupertypeMismatch, &[&name, &n]));
                }
            }
        }
        if contextual && !is_error {
            self.resolve_additional_operation(module, owner, op);
        }
        let me = &mut self.modules[module.index()];
        me.eoperations.push(OpRef::User(op));
        me.ir.operations.push(op);
    }

    /// `QvtTypeResolverImpl.resolveAdditionalOperation`
    fn resolve_additional_operation(&mut self, module: ModuleId, owner: TypeId, op: OpId) {
        let me = &mut self.modules[module.index()];
        if owner == TypeId::OCL_ANY {
            me.defines_ocl_any_features = true;
        }
        if let Some(e) = me.ctx_ops.iter_mut().find(|(t, _)| *t == owner) {
            e.1.push(op);
        } else {
            me.ctx_ops.push((owner, vec![op]));
        }
        if !me.additional_types.contains(&owner) {
            me.additional_types.push(owner);
        }
    }

    /// `doFindCollidingOperation`
    fn find_colliding_operation(&mut self, op: OpId, owner: TypeId, contextual: bool) -> Option<Collision> {
        let module = self.ops[op.index()].module;
        let name = self.ops[op.index()].ir.name.clone();
        let mut operations = self.get_operations(owner);
        if !contextual {
            if let Some(o) = self.find_matching_from_extended(module, op, 0) {
                operations.push(o);
            }
        } else {
            self.collect_additional_ops_in_hierarchy(module, owner, &mut operations, 0);
        }
        let mut seen = std::collections::HashSet::new();
        operations.retain(|o| seen.insert(*o));
        let overridden: Vec<OpRef> = operations.iter().filter_map(|o| self.op_overridden(*o)).collect();
        let mut result = None;
        let me = OpRef::User(op);
        for next in operations {
            if next == me || self.op_name(next) != name || !self.match_parameters(next, me) {
                continue;
            }
            let next_owner = self.op_owner(next);
            if contextual {
                let orel = self.ts.ck().relationship(owner, next_owner);
                if orel != SAME_TYPE && orel & RELATED_TYPE != 0 {
                    let nret = self.op_ret(next);
                    let oret = self.ops[op.index()].ret;
                    let rrel = self.ts.ck().relationship(oret, nret);
                    if orel & SUPERTYPE != 0 {
                        if rrel & SUPERTYPE == 0 {
                            if name == "main" {
                                return None;
                            }
                            return Some(Collision::VirtualSupertype(next));
                        }
                    } else if orel & SUBTYPE != 0 && rrel & SUBTYPE == 0 {
                        if name == "main" {
                            return None;
                        }
                        return Some(Collision::VirtualSubtype(next));
                    }
                    if let OpRef::User(nid) = next {
                        if !self.ops[op.index()].vtable.contains(&nid) {
                            self.ops[op.index()].vtable.push(nid);
                        }
                        if !self.is_imported_by_access(module, next) && !self.ops[nid.index()].vtable.contains(&op) {
                            self.ops[nid.index()].vtable.push(op);
                        }
                    }
                }
            }
            if owner == next_owner || !contextual {
                let defined_here = matches!(next, OpRef::User(nid) if self.ops[nid.index()].module == module);
                if !defined_here {
                    if self.is_imported_by_extends(module, next) && !overridden.contains(&next) {
                        result = Some(Collision::Overrides(next));
                    }
                } else {
                    return Some(Collision::AlreadyDefined(next));
                }
            }
        }
        result
    }

    fn is_imported_by_access(&self, module: ModuleId, o: OpRef) -> bool {
        let Some(m) = self.op_module(o) else {
            return false;
        };
        self.modules[module.index()].by_access.contains(&m)
    }

    fn is_imported_by_extends(&self, module: ModuleId, o: OpRef) -> bool {
        let Some(m) = self.op_module(o) else {
            return false;
        };
        self.all_extended(module).contains(&m)
    }

    /// `matchParameters`: same parameter count and exactly matching OCL types.
    fn match_parameters(&mut self, a: OpRef, b: OpRef) -> bool {
        let pa = self.op_params(a);
        let pb = self.op_params(b);
        if pa.len() != pb.len() {
            return false;
        }
        for (x, y) in pa.iter().zip(&pb) {
            let x = self.ts.ck().ocl_type(*x);
            let y = self.ts.ck().ocl_type(*y);
            if !self.ts.ck().exact(x, y) {
                return false;
            }
        }
        true
    }

    /// `findMatchingFromExtended`
    fn find_matching_from_extended(&mut self, module: ModuleId, op: OpId, depth: usize) -> Option<OpRef> {
        if depth > 32 {
            return None;
        }
        let name = self.ops[op.index()].ir.name.clone();
        for (imp, _) in self.modules[module.index()].module_imports.clone() {
            for o in self.modules[imp.index()].eoperations.clone() {
                if self.op_name(o) == name && self.match_parameters(o, OpRef::User(op)) {
                    if let OpRef::User(_) = o {
                        let a = self.op_context(o);
                        let b = self.op_context(OpRef::User(op));
                        let same = match (a, b) {
                            (None, None) => true,
                            (Some(x), Some(y)) => {
                                let x = self.ts.ck().ocl_type(x);
                                let y = self.ts.ck().ocl_type(y);
                                self.ts.ck().exact(x, y)
                            }
                            _ => false,
                        };
                        if !same {
                            continue;
                        }
                    }
                    return Some(o);
                }
            }
            if let Some(r) = self.find_matching_from_extended(imp, op, depth + 1) {
                return Some(r);
            }
        }
        None
    }

    /// `collectAdditionalOperationsInTypeHierarchy(type, true, result)`
    fn collect_additional_ops_in_hierarchy(&mut self, env: ModuleId, t: TypeId, out: &mut Vec<OpRef>, depth: usize) {
        if depth > 32 {
            return;
        }
        for next in self.modules[env.index()].additional_types.clone() {
            if t != next && self.ts.ck().relationship(t, next) == STRICT_SUPERTYPE {
                if let Some((_, l)) = self.modules[env.index()].ctx_ops.iter().find(|(k, _)| *k == next) {
                    out.extend(l.iter().map(|&o| OpRef::User(o)));
                }
            }
        }
        for e in self.modules[env.index()].by_extension.clone() {
            self.collect_additional_ops_in_hierarchy(e, t, out, depth + 1);
        }
        for e in self.modules[env.index()].by_access.clone() {
            self.collect_additional_ops_in_hierarchy(e, t, out, depth + 1);
        }
    }

    // ---- pass 5 --------------------------------------------------------------------------------

    /// `createModuleProperties`
    fn create_module_properties(&mut self, id: ModuleId, cs: &ModuleCs<'_>) {
        let blackbox = self.modules[id.index()].blackbox;
        let is_transf = !self.modules[id.index()].is_library;
        for p in &cs.properties {
            match p.kind {
                ast::PropertyKind::Intermediate => {
                    let Some(ip) = self.visit_contextual_property(id, p) else {
                        continue;
                    };
                    if blackbox {
                        let n = self.interm_props[ip.index()].ir.name.clone();
                        self.error(p.span, bind(vm::QvtOperationalVisitorCS_intermediatePropertyDefinitionInsideBlackboxModule, &[&n]));
                    }
                    if let Some(ct) = self.interm_props[ip.index()].context {
                        let n = self.interm_props[ip.index()].ir.name.clone();
                        if self.lookup_property(ct, &n).is_some() {
                            self.interm_props[ip.index()].hidden = true;
                        }
                    }
                    if !is_transf {
                        self.error(p.span, bind(vm::IntermediatePropertyNotAllowed, &[]));
                    }
                    self.modules[id.index()].features.push(ModuleFeature::Interm(ip));
                }
                _ => {
                    let g = self.visit_module_property(id, p);
                    if p.kind == ast::PropertyKind::Plain && blackbox {
                        let n = self.globals[g.index()].ir.name.clone();
                        self.error(p.span, bind(vm::QvtOperationalVisitorCS_localPropertyDefinitionInsideBlackboxModule, &[&n]));
                    }
                    let name = self.globals[g.index()].ir.name.clone();
                    let mty = self.modules[id.index()].ty;
                    if let Some(existing) = self.lookup_property(mty, &name) {
                        let owner = self.prop_owner(existing);
                        if self.ts.ck().compatible(mty, owner) {
                            self.globals[g.index()].hidden = true;
                            let span = self.globals[g.index()].name_span;
                            self.error(span, bind(vm::ModulePropertyAlreadyDefined, &[&name]));
                        }
                    }
                    self.modules[id.index()].features.push(ModuleFeature::Global(g));
                    let me = &mut self.modules[id.index()];
                    // initialisation order: configuration properties first
                    if self.globals[g.index()].is_config {
                        let pos = me.ir.globals.iter().position(|x| !self.globals[x.index()].is_config).unwrap_or(me.ir.globals.len());
                        me.ir.globals.insert(pos, g);
                    } else {
                        me.ir.globals.push(g);
                    }
                }
            }
        }
        if is_transf {
            self.check_intermediate_property_clashes(id);
        }
    }

    fn visit_module_property(&mut self, id: ModuleId, p: &ast::Property) -> ir::GlobalId {
        let name_n = p.name.name();
        let name = name_n.map(|n| n.text.clone()).unwrap_or_default();
        let name_span = name_n.map(|n| n.span).unwrap_or(p.span);
        let gid = ir::GlobalId(self.globals.len() as u32);
        let is_config = p.kind == ast::PropertyKind::Configuration;
        let mut ty = None;
        let mut init = None;
        if is_config {
            if let Some(t) = &p.ty {
                ty = self.type_cs(t);
                if let Some(tt) = ty {
                    if !self.ts.ck().is_primitive(tt) && !self.ts.types.is_collection_like(tt) && !matches!(self.ts.types.get(tt), Type::DataType(_) | Type::Enum(_) | Type::Tuple(_)) {
                        let n = self.type_name(tt);
                        self.error(t.span, bind(vm::ConfigPropertyTypeUnsupported, &[&n]));
                    }
                }
            } else {
                self.error(name_span, bind(vm::ConfigPropertyMustHaveType, &[&name]));
            }
        } else {
            if let Some(t) = &p.ty {
                ty = self.type_cs(t);
            }
            if let Some(e) = &p.init {
                self.begin_code(id, None);
                let ex = self.visit_ocl_expression(e);
                let locals = std::mem::take(&mut self.fun.locals);
                self.end_code();
                if let Some(ex) = ex {
                    if matches!(self.ts.types.get(ex.ty()), Type::TypeType(_)) {
                        self.error(e.span, vm::PropertyTypeTypeError);
                    }
                    if ty.is_none() {
                        ty = Some(ex.ty());
                    }
                    let real = ex.ty();
                    let declared = ty.unwrap_or(TypeId::NULL);
                    if !self.ts.ck().assignable(declared, real) {
                        let a = self.full_name(declared);
                        let b = self.full_name(real);
                        self.error(p.span, bind(vm::TypeMismatchError, &[&a, &b]));
                    }
                    init = Some(ir::CodeBody {
                        source: ir::SourceId(self.cur_unit as u32 + 1),
                        locals,
                        self_local: None,
                        expr: ex.e,
                    });
                }
            }
        }
        self.globals.push(GlobalInfo {
            ir: ir::Global {
                module: id,
                name,
                kind: if is_config { ir::GlobalKind::Config } else { ir::GlobalKind::Property },
                ty: ty.unwrap_or(TypeId::NULL),
                init,
                span: p.span,
            },
            hidden: false,
            name_span,
            is_config,
            changeable: true,
        });
        gid
    }

    /// `visitContextualPropertyCS`
    fn visit_contextual_property(&mut self, id: ModuleId, p: &ast::Property) -> Option<ir::IntermPropId> {
        let (name_n, ctx) = scoped_parts(&p.name);
        let name = name_n.map(|n| n.text.clone()).unwrap_or_default();
        let name_span = name_n.map(|n| n.span).unwrap_or(p.span);
        let mut ty = None;
        if let Some(t) = &p.ty {
            ty = self.type_cs(t);
            if ty.is_none() {
                return Some(self.push_interm_prop(id, name, TypeId::NULL, None, None, p.span, name_span));
            }
        }
        let mut init = None;
        let mut init_ty = None;
        if let Some(e) = &p.init {
            self.warning(e.span, bind(vm::IntermediatePropertiesInitNotSupported, &[]));
            self.begin_code(id, None);
            let ex = self.visit_ocl_expression(e);
            let locals = std::mem::take(&mut self.fun.locals);
            self.end_code();
            if let Some(ex) = ex {
                init_ty = Some(ex.ty());
                init = Some(ir::CodeBody {
                    source: ir::SourceId(self.cur_unit as u32 + 1),
                    locals,
                    self_local: None,
                    expr: ex.e,
                });
            }
        }
        if ty.is_none() {
            ty = init_ty;
        }
        let ty = ty?;
        if let Some(r) = init_ty {
            if !self.ts.ck().assignable(ty, r) {
                let a = self.full_name(ty);
                let b = self.full_name(r);
                self.error(p.span, bind(vm::TypeMismatchError, &[&a, &b]));
            }
        }
        let mut context = None;
        let ctx_ty = match &ctx {
            Some(CtxType::Type(t)) => self.type_cs_in(t, true),
            Some(CtxType::Path(path)) => {
                let te = ast::TypeExpr {
                    kind: ast::TypeKind::Path(ast::PathName { segments: path.to_vec(), span: path_span(path) }),
                    span: path_span(path),
                };
                self.type_cs_in(&te, true)
            }
            None => None,
        };
        match ctx_ty {
            Some(ct) => {
                if matches!(self.ts.types.get(ct), Type::Class(_) | Type::Module(_) | Type::ModelType(_) | Type::Tuple(_)) {
                    context = Some(ct);
                } else {
                    let span = ctx.as_ref().map(|c| c.span()).unwrap_or(p.span);
                    self.error(span, bind(vm::QvtOperationalVisitorCS_ContextualPropertyTypeIsNotClass, &[&name]));
                }
            }
            None => self.error(p.name.span, bind(vm::QvtOperationalVisitorCS_ContextualPropertyTypeIsMissed, &[])),
        }
        Some(self.push_interm_prop(id, name, ty, context, init, p.span, name_span))
    }

    #[allow(clippy::too_many_arguments)]
    fn push_interm_prop(&mut self, id: ModuleId, name: String, ty: TypeId, context: Option<TypeId>, init: Option<ir::CodeBody>, span: Span, name_span: Span) -> ir::IntermPropId {
        let pid = ir::IntermPropId(self.interm_props.len() as u32);
        let cty = context.unwrap_or(TypeId::NULL);
        self.interm_props.push(IntermPropInfo {
            ir: ir::IntermProp { module: id, context: cty, name, ty, init, span },
            context,
            hidden: false,
            name_span,
        });
        pid
    }

    /// The duplicate checks of intermediate properties after all are defined
    /// (`IntermediatePropertyAlreadyDefined`).
    fn check_intermediate_property_clashes(&mut self, id: ModuleId) {
        let props: Vec<ir::IntermPropId> = self.modules[id.index()]
            .features
            .iter()
            .filter_map(|f| if let ModuleFeature::Interm(i) = f { Some(*i) } else { None })
            .collect();
        for &ip in &props {
            let Some(ct) = self.interm_props[ip.index()].context else {
                continue;
            };
            let name = self.interm_props[ip.index()].ir.name.clone();
            let found = self.lookup_property(ct, &name);
            let already = (found.is_some() && found != Some(crate::ops::PropRef::Interm(ip))) | self.interm_props[ip.index()].hidden;
            let clash = self.hierarchy_clash(id, ip, &props);
            if already || clash {
                self.interm_props[ip.index()].hidden = true;
                let span = self.interm_props[ip.index()].name_span;
                self.error(span, bind(vm::IntermediatePropertyAlreadyDefined, &[&name]));
            }
        }
    }

    /// `IntermediatePropertyHierarchy.hasHierarchyClashes`: another property of the same name on
    /// a sub- or supertype of the context.
    fn hierarchy_clash(&mut self, _id: ModuleId, ip: ir::IntermPropId, props: &[ir::IntermPropId]) -> bool {
        let Some(Type::Class(c)) = self.interm_props[ip.index()].context.map(|t| self.ts.types.get(t).clone()) else {
            return false;
        };
        let name = self.interm_props[ip.index()].ir.name.clone();
        for &o in props {
            if o == ip || self.interm_props[o.index()].ir.name != name {
                continue;
            }
            if let Some(Type::Class(oc)) = self.interm_props[o.index()].context.map(|t| self.ts.types.get(t).clone()) {
                if oc != c && (self.ts.meta.class(oc).all_supers.contains(&c) || self.ts.meta.class(c).all_supers.contains(&oc)) {
                    return true;
                }
            }
        }
        false
    }

    // ---- pass 6 --------------------------------------------------------------------------------

    fn visit_method_bodies(&mut self, id: ModuleId, _cs: &ModuleCs<'_>, map: &[(MethodCs<'_>, OpId)]) {
        for &(mcs, op) in map {
            self.visit_method(id, mcs, op);
        }
        if let Some(m) = self.main_operation(id) {
            if self.ops[m.index()].ir.kind == ir::OpKind::Entry {
                self.modules[id.index()].ir.entry = Some(m);
            }
        }
        if self.modules[id.index()].ir.entry.is_none() {
            // legacy `mapping main(...)`
            if let Some(m) = self.main_operation(id) {
                self.modules[id.index()].ir.entry = Some(m);
            }
        }
        self.validate_late_resolves();
    }
}

pub(crate) struct Usage {
    pub access: bool,
    pub path: Vec<String>,
    pub params: Option<Vec<ast::Param>>,
    pub ref_span: Span,
    pub usage_span: Span,
    pub path_span: Span,
    pub in_header: bool,
}

fn new_module_env(name: &str, library: bool, unit: usize, ty: TypeId, header: Span) -> ModuleEnv {
    ModuleEnv {
        ir: ir::Module {
            name: name.to_string(),
            kind: if library { ir::ModuleKind::Library } else { ir::ModuleKind::Transformation },
            blackbox: false,
            source: ir::SourceId(0),
            span: header,
            ty,
            model_params: Vec::new(),
            model_types: Vec::new(),
            usages: Vec::new(),
            globals: Vec::new(),
            operations: Vec::new(),
            entry: None,
            classes: Vec::new(),
        },
        name: name.to_string(),
        is_library: library,
        unit,
        by_access: Vec::new(),
        by_extension: vec![STDLIB_MODULE],
        ctx_ops: Vec::new(),
        additional_types: Vec::new(),
        defines_ocl_any_features: false,
        std_additional: Vec::new(),
        model_types: Vec::new(),
        eoperations: Vec::new(),
        features: Vec::new(),
        model_params: Vec::new(),
        mappings_map: Vec::new(),
        aliases: Vec::new(),
        module_imports: Vec::new(),
        interm_package: None,
        ty,
        header_span: header,
        blackbox: false,
    }
}

fn duplicates(q: &[TokenKind]) -> Vec<TokenKind> {
    let mut out = Vec::new();
    for k in q {
        if q.iter().filter(|x| *x == k).count() > 1 && !out.contains(k) {
            out.push(*k);
        }
    }
    out
}

fn has_blackbox_qualifier(q: &[TokenKind]) -> bool {
    q.contains(&TokenKind::KwBlackbox)
}

fn kind_of(s: &Sema, op: OpId) -> ir::OpKind {
    s.ops[op.index()].ir.kind
}

pub(crate) fn has_body(m: MethodCs<'_>) -> bool {
    match m {
        MethodCs::Mapping(x) => x.body.is_some(),
        MethodCs::Constructor(c) => c.body.is_some(),
        MethodCs::Helper(h) => !matches!(h.body, ast::OpBody::None),
        MethodCs::Entry(e) => e.body.is_some(),
    }
}

pub(crate) fn is_disjunctive(m: MethodCs<'_>) -> bool {
    match m {
        MethodCs::Mapping(x) => x.extensions.iter().any(|e| e.kind == TokenKind::KwDisjuncts),
        _ => false,
    }
}

fn split_uri(uri: &str) -> (String, &str) {
    // scheme://authority/rest or scheme:rest
    if let Some(i) = uri.find("://") {
        let after = &uri[i + 3..];
        let j = after.find('/').unwrap_or(after.len());
        (uri[..i + 3 + j].to_string(), &after[j..])
    } else if let Some(i) = uri.find(':') {
        (uri[..i + 1].to_string(), &uri[i + 1..])
    } else {
        (String::new(), uri)
    }
}

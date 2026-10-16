//! Classifier lookup (`QvtOperationalEnv.lookupClassifier`, Ecore's package lookup, the QVTo
//! library's classifiers) and type expressions (`typeCS`).

use crate::analyzer::{STDLIB_MODULE, Sema};
use crate::messages::{bind, ocl as om, validation as vm};
use qvto_model::{Classifier, PackageId};
use qvto_syntax::ast::{self, TypeKind};
use qvto_syntax::{Span, TokenKind};
use qvto_types::{CollKind, Type, TypeId};

/// The result of a classifier lookup.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    Type(TypeId),
}

impl<'r> Sema<'r> {
    /// `QvtOperationalEnv.lookupClassifier(names)` in the current module environment.
    pub fn lookup_classifier(&mut self, names: &[&str]) -> Option<TypeId> {
        if names.is_empty() {
            return None;
        }
        let m = self.cur_module;
        let first = names[0];
        if names.len() == 1 {
            if self.modules[m.index()].name == first {
                return Some(self.modules[m.index()].ty);
            }
            for e in self.modules[m.index()].by_extension.clone() {
                if e != STDLIB_MODULE && self.modules[e.index()].name == first {
                    return Some(self.modules[e.index()].ty);
                }
            }
            for e in self.modules[m.index()].by_access.clone() {
                if self.modules[e.index()].name == first {
                    return Some(self.modules[e.index()].ty);
                }
            }
            let mts = self.modules[m.index()].model_types.clone();
            if let Some((_, mt)) = mts.iter().find(|(n, _)| n == first) {
                return Some(self.model_types[mt.index()].decl.ty);
            }
            for (_, mt) in &mts {
                if let Some(t) = self.lookup_in_model_type(*mt, names) {
                    return Some(t);
                }
            }
        }
        if names.len() > 1 {
            if names.len() == 2 {
                if let Some(t) = self.stdlib_classifier(names) {
                    return Some(t);
                }
            }
            let mts = self.modules[m.index()].model_types.clone();
            if let Some((_, mt)) = mts.iter().find(|(n, _)| n == first) {
                if let Some(t) = self.lookup_in_model_type(*mt, &names[1..]) {
                    return Some(t);
                }
            }
            // EcoreEnvironment.lookupClassifier without context package: packages registered
            // in this file's registry by name
            if let Some(p) = self.find_package(&names[..names.len() - 1]) {
                return self.classifier_in(p, names[names.len() - 1]);
            }
            return self.stdlib_classifier(names);
        }
        self.stdlib_classifier(names)
    }

    /// `QvtOperationalStdLibrary.lookupClassifier`
    fn stdlib_classifier(&mut self, names: &[&str]) -> Option<TypeId> {
        let n = names.len();
        if n == 0 || n > 2 {
            return None;
        }
        let tn = names[n - 1];
        if n == 1 || names[0] == "Stdlib" {
            match tn {
                "Any" => return Some(TypeId::OCL_ANY),
                "Void" => return Some(TypeId::OCL_VOID),
                _ => {}
            }
            let std = self.ts.std;
            if let Some(c) = std.by_name(tn) {
                return Some(self.ts.types.class(c));
            }
            return match tn {
                // the library's typedefs and template parameters
                "KeyT" => Some(TypeId::KEY_T),
                "CommonT" => Some(TypeId::COMMON_T),
                _ => None,
            };
        }
        if names[0] == "oclstdlib" {
            return Some(match tn {
                "OclAny" => TypeId::OCL_ANY,
                "OclVoid" => TypeId::OCL_VOID,
                "OclInvalid" => TypeId::OCL_INVALID,
                "Boolean" => TypeId::BOOLEAN,
                "Integer" => TypeId::INTEGER,
                "Real" => TypeId::REAL,
                "String" => TypeId::STRING,
                "UnlimitedNatural" => TypeId::UNLIMITED_NATURAL,
                "OclType" => TypeId::OCL_TYPE,
                "OclElement" => TypeId::OCL_ELEMENT,
                "T" => TypeId::T,
                "T2" => TypeId::T2,
                "OclMessage" => self.ts.types.intern(Type::OclMessage),
                "State" => self.ts.types.intern(Type::State),
                _ => return None,
            });
        }
        None
    }

    /// A classifier of a model type by (relative) path, contained in one of its metamodels
    /// (`doLookupModeltypeClassifier`).
    fn lookup_in_model_type(
        &mut self,
        mt: crate::ir::ModelTypeId,
        path: &[&str],
    ) -> Option<TypeId> {
        let pkgs = self.model_types[mt.index()].decl.packages.clone();
        for pkg in pkgs {
            let r = self.lookup_classifier_in_context(pkg, path);
            if let Some((c, cpkg)) = r {
                // the result's container chain of the path's length must reach the package
                let mut p = Some(cpkg);
                for _ in 1..path.len() {
                    p = p.and_then(|x| self.ts.meta.package(x).super_package);
                }
                if p == Some(pkg) {
                    return Some(self.classifier_type(c));
                }
            }
        }
        None
    }

    /// `EcoreEnvironment.lookupClassifier` with a context package.
    fn lookup_classifier_in_context(
        &mut self,
        ctx: PackageId,
        names: &[&str],
    ) -> Option<(Classifier, PackageId)> {
        if names.len() > 1 {
            let mut lookup: Vec<&str> = names.to_vec();
            let mut cur = Some(ctx);
            let mut first_level = true;
            while let Some(c) = cur {
                let mut pkg = Some(c);
                for n in &lookup[..lookup.len() - 1] {
                    pkg = pkg.and_then(|p| self.subpackage(p, n));
                    if pkg.is_none() {
                        break;
                    }
                }
                if let Some(p) = pkg {
                    let cl = self.ts.meta.classifier_by_name(p, lookup[lookup.len() - 1]);
                    return cl.map(|x| (x, p));
                }
                if first_level && lookup.len() > 1 && *self.ts.meta.package(c).name == *lookup[0] {
                    lookup.remove(0);
                    first_level = false;
                } else {
                    lookup = names.to_vec();
                    cur = self.ts.meta.package(c).super_package;
                    first_level = false;
                }
            }
            let p = self.find_package(&names[..names.len() - 1])?;
            let cl = self.ts.meta.classifier_by_name(p, names[names.len() - 1])?;
            Some((cl, p))
        } else {
            let mut cur = Some(ctx);
            while let Some(c) = cur {
                if let Some(cl) = self.ts.meta.classifier_by_name(c, names[0]) {
                    return Some((cl, c));
                }
                cur = self.ts.meta.package(c).super_package;
            }
            None
        }
    }

    fn subpackage(&self, p: PackageId, name: &str) -> Option<PackageId> {
        self.ts
            .meta
            .package(p)
            .subpackages
            .iter()
            .copied()
            .find(|&s| *self.ts.meta.package(s).name == *name)
    }

    fn classifier_in(&mut self, p: PackageId, name: &str) -> Option<TypeId> {
        let c = self.ts.meta.classifier_by_name(p, name)?;
        Some(self.classifier_type(c))
    }

    /// `EcoreEnvironment.findPackage` over this file's registry: a root package with the first
    /// name, then nested packages; then by namespace prefix.
    pub fn find_package(&self, names: &[&str]) -> Option<PackageId> {
        if names.is_empty() {
            return None;
        }
        let reg = &self.units[self.cur_unit].registry;
        for &(_, p) in reg {
            let pi = self.ts.meta.package(p);
            if pi.super_package.is_none() && *pi.name == *names[0] {
                let mut r = Some(p);
                for n in &names[1..] {
                    r = r.and_then(|x| self.subpackage(x, n));
                }
                if r.is_some() {
                    return r;
                }
            }
        }
        let prefix = names.join(".");
        for &(_, p) in reg {
            if self.ts.meta.package(p).ns_prefix.as_deref() == Some(prefix.as_str()) {
                return Some(p);
            }
        }
        None
    }

    // ---- type expressions ----------------------------------------------------------------------

    /// `QvtOperationalVisitorCS.typeCS`: the type of a type expression, `None` when it does not
    /// resolve (the problem is reported).
    pub fn type_cs(&mut self, t: &ast::TypeExpr) -> Option<TypeId> {
        self.type_cs_in(t, false)
    }

    /// `typeCS`; `allow_library` for type expressions inside scoped names and `resolveIn`.
    pub fn type_cs_in(&mut self, t: &ast::TypeExpr, allow_library: bool) -> Option<TypeId> {
        let ty = match &t.kind {
            TypeKind::Primitive(k) => Some(primitive(*k)),
            TypeKind::Path(p) => {
                let names: Vec<&str> = p.segments.iter().map(|n| n.text.as_str()).collect();
                match self.lookup_classifier(&names) {
                    Some(x) => Some(x),
                    None => {
                        let msg = bind(om::UnrecognizedType_ERROR_, &[&names.join("::")]);
                        self.error(t.span, msg);
                        Some(TypeId::OCL_VOID)
                    }
                }
            }
            TypeKind::Collection(k, elem) => {
                let e = self.type_cs_in(elem, false);
                let kind = coll_kind(*k);
                let ct = self.ts.types.collection(kind, e.unwrap_or(TypeId::NULL));
                if e.is_none() {
                    let msg = bind(vm::UnknownClassifierType, &[&type_string(elem)]);
                    self.error(elem.span, msg);
                }
                Some(ct)
            }
            TypeKind::Tuple(parts) => Some(self.tuple_type_cs(parts)),
            TypeKind::List(elem) => {
                let e = self.type_cs_in(elem, false);
                match e {
                    Some(e) => Some(self.ts.types.list(e)),
                    None => Some(TypeId::OCL_VOID),
                }
            }
            TypeKind::Dict(k, v) => {
                let kt = self.type_cs_in(k, false);
                let vt = self.type_cs_in(v, false);
                match (kt, vt) {
                    (Some(a), Some(b)) => Some(self.ts.types.dict(a, b)),
                    _ => Some(TypeId::OCL_VOID),
                }
            }
            TypeKind::Error => None,
        };
        let ty = ty?;
        if ty == TypeId::OCL_VOID && !matches!(t.kind, TypeKind::Primitive(_)) {
            if let TypeKind::Path(p) = &t.kind {
                let names: Vec<&str> = p.segments.iter().map(|n| n.text.as_str()).collect();
                if self.stdlib_classifier(&names) == Some(TypeId::OCL_VOID) {
                    return Some(ty);
                }
            }
            return None;
        }
        if let Type::Module(m) = *self.ts.types.get(ty) {
            if self.modules[m as usize].is_library && !allow_library {
                let msg = bind(
                    vm::QvtOperationalVisitorCS_cantUseLibraryAsType,
                    &[&self.module_name(crate::ir::ModuleId(m)).to_string()],
                );
                self.error(t.span, msg);
                return None;
            }
        }
        Some(ty)
    }

    /// `AbstractOCLAnalyzer.tupleTypeCS`
    fn tuple_type_cs(&mut self, parts: &[ast::VarDecl]) -> TypeId {
        let mut out: Vec<(Box<str>, TypeId)> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        for v in parts {
            let ty = v.ty.as_ref().and_then(|t| self.type_cs(t));
            let name = v.name.text.clone();
            let mut keep_name = true;
            if names.contains(&name) {
                let msg = bind(om::DuplicateNameInTuple_ERROR_, &[&name]);
                self.error(v.span, msg);
                keep_name = false;
            } else {
                names.push(name.clone());
            }
            if v.init.is_some() {
                let msg = bind(om::InitExpNotAllowed_ERROR_, &[&name]);
                self.error(v.span, msg);
            }
            let ty = match ty {
                Some(t) => t,
                None => {
                    let msg = bind(om::DeclarationType_ERROR_, &[&name]);
                    self.error(v.span, msg);
                    TypeId::OCL_VOID
                }
            };
            if keep_name {
                out.push((name.into(), ty));
            }
        }
        self.tuple_type(out)
    }
}

pub(crate) fn primitive(k: TokenKind) -> TypeId {
    match k {
        TokenKind::KwBoolean => TypeId::BOOLEAN,
        TokenKind::KwInteger => TypeId::INTEGER,
        TokenKind::KwReal => TypeId::REAL,
        TokenKind::KwString => TypeId::STRING,
        TokenKind::KwUnlimitedNatural => TypeId::UNLIMITED_NATURAL,
        TokenKind::KwOclAny => TypeId::OCL_ANY,
        TokenKind::KwOclInvalid => TypeId::OCL_INVALID,
        _ => TypeId::OCL_VOID,
    }
}

pub(crate) fn coll_kind(k: TokenKind) -> CollKind {
    match k {
        TokenKind::KwSet => CollKind::Set,
        TokenKind::KwBag => CollKind::Bag,
        TokenKind::KwSequence => CollKind::Sequence,
        TokenKind::KwOrderedSet => CollKind::OrderedSet,
        _ => CollKind::Collection,
    }
}

/// `QvtOperationalParserUtil.getStringRepresentation(TypeCS)`.
pub(crate) fn type_string(t: &ast::TypeExpr) -> String {
    match &t.kind {
        TypeKind::Primitive(k) => k.spelling().unwrap_or("").to_string(),
        TypeKind::Path(p) => p
            .segments
            .iter()
            .map(|n| n.text.as_str())
            .collect::<Vec<_>>()
            .join("::"),
        TypeKind::Collection(k, e) => format!("{}({})", k.spelling().unwrap_or(""), type_string(e)),
        TypeKind::Tuple(parts) => {
            let s: Vec<String> = parts
                .iter()
                .map(|v| {
                    format!(
                        "{}:{}",
                        v.name.text,
                        v.ty.as_ref().map(type_string).unwrap_or_default()
                    )
                })
                .collect();
            format!("Tuple({})", s.join(","))
        }
        _ => String::new(),
    }
}

#[allow(dead_code)]
pub(crate) fn span_of(t: &ast::TypeExpr) -> Span {
    t.span
}

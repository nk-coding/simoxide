//! Operation and property lookup as Eclipse does it: `TypeCheckerImpl.getOperations` /
//! `findMostSpecificOperationMatching` / `getMostSpecificOperation`, classic OCL's
//! `matchArgsWithExactitude` and `resolveGenericType`, the additional (contextual) operations of
//! `QvtEnvironmentBase` and `QvtTypeResolverImpl`, and `findAttribute`/`getAttributes`.

use crate::analyzer::{Sema, STDLIB_MODULE};
use crate::env::ModuleFeature;
use crate::ir::{ModuleId, OpId};
use crate::ops::{Arg, OpRef, PropRef};
use qvto_types::rel::*;
use qvto_types::{owner_ops, CollKind, Section, StdOwner, TParam, Type, TypeId};

/// Relation of `TypeCheckerImpl.getMoreSpecificByParameters`.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum Relation {
    More,
    Less,
    Identical,
    Ambiguous,
    Unrelated,
}

const RELATIONAL: [&str; 4] = ["<", "<=", ">", ">="];

impl<'r> Sema<'r> {
    // ---- the standard library's lists ----------------------------------------------------------

    fn std_ops(&self, section: Section, owner: StdOwner) -> impl Iterator<Item = OpRef> + '_ {
        owner_ops(section, owner)
            .iter()
            .map(move |&o| OpRef::Std(o, owner))
    }

    /// `PredefinedType.oclOperations()` of a predefined type.
    fn ocl_operations(&self, t: TypeId) -> Vec<OpRef> {
        let owner = match self.ts.types.get(t) {
            Type::OclAny | Type::Param(TParam::T | TParam::T2) | Type::Tuple(_) => StdOwner::OclAny,
            Type::OclVoid => StdOwner::OclVoid,
            Type::OclInvalid => StdOwner::OclInvalid,
            Type::Boolean => StdOwner::Boolean,
            Type::Integer => StdOwner::Integer,
            Type::Real => StdOwner::Real,
            Type::String => StdOwner::String,
            Type::UnlimitedNatural => StdOwner::UnlimitedNatural,
            Type::OclMessage => StdOwner::OclMessage,
            Type::TypeType(_) => StdOwner::OclType,
            Type::Collection(k, _) => match k {
                CollKind::Collection => StdOwner::Collection,
                CollKind::Set => StdOwner::Set,
                CollKind::OrderedSet => StdOwner::OrderedSet,
                CollKind::Bag => StdOwner::Bag,
                CollKind::Sequence => StdOwner::Sequence,
            },
            Type::List(_) => StdOwner::List,
            Type::Dict(..) => StdOwner::Dict,
            _ => return Vec::new(),
        };
        self.std_ops(Section::Ocl, owner).collect()
    }

    fn is_any_type(&self, t: TypeId) -> bool {
        matches!(
            self.ts.types.get(t),
            Type::OclAny | Type::Param(TParam::T | TParam::T2)
        )
    }

    // ---- getOperations -------------------------------------------------------------------------

    /// `TypeCheckerImpl.getOperations(owner)` in the current module's environment.
    pub fn get_operations(&mut self, owner: TypeId) -> Vec<OpRef> {
        if let Type::TypeType(referred) = *self.ts.types.get(owner) {
            let mut r: Vec<OpRef> = self.std_ops(Section::Ocl, StdOwner::OclType).collect();
            for o in self.get_operations(referred) {
                if self.op_is_static(o) {
                    r.push(o);
                }
            }
            return r;
        }
        let mut owner = owner;
        let mut result: Vec<OpRef>;
        if self.ts.ck().is_predefined(owner) {
            result = self.ocl_operations(owner);
            if self.is_any_type(owner) {
                result.retain(|o| !RELATIONAL.contains(&self.op_name(*o)));
            }
        } else {
            owner = self.ts.ck().ocl_type(owner);
            if self.ts.ck().is_predefined(owner) {
                result = self.ocl_operations(owner);
            } else {
                result = self.uml_operations(owner);
                let any = self.get_operations(TypeId::OCL_ANY);
                result.extend(any);
            }
        }
        let add = self.additional_operations(self.cur_module, owner);
        result.extend(add);
        result
    }

    /// `QVTUMLReflection.getOperations`: a module's non-contextual operations (with its
    /// supertypes'), a class's `eAllOperations` plus `Element`'s for model elements.
    fn uml_operations(&mut self, owner: TypeId) -> Vec<OpRef> {
        let std = self.ts.std;
        match self.ts.types.get(owner).clone() {
            Type::Module(m) => {
                let mut r = Vec::new();
                let me = &self.modules[m as usize];
                if !me.is_library {
                    r.extend(self.std_ops(Section::Class, StdOwner::Transformation));
                }
                for &o in &me.eoperations {
                    if let OpRef::User(id) = o {
                        if self.ops[id.index()].contextual {
                            continue;
                        }
                    }
                    r.push(o);
                }
                r
            }
            Type::ModelType(_) => self.std_ops(Section::Class, StdOwner::Model).collect(),
            Type::Class(c) => {
                let mut r = Vec::new();
                let all: Vec<_> = self
                    .ts
                    .meta
                    .class(c)
                    .all_supers
                    .iter()
                    .copied()
                    .chain(std::iter::once(c))
                    .collect();
                for k in all {
                    if let Some(owner) = std_class_owner(&std, k) {
                        r.extend(self.std_ops(Section::Class, owner));
                    }
                    if let Some(ops) = self.class_ops.get(&k) {
                        r.extend(ops.iter().map(|m| OpRef::Meta(*m)));
                    }
                }
                if self.ts.meta.class(c).package != std.package {
                    r.extend(self.std_ops(Section::Class, StdOwner::Element));
                }
                r
            }
            _ => Vec::new(),
        }
    }

    /// All modules extended by `m`, transitively (`getAllExtendedModules`, a linked set).
    pub fn all_extended(&self, m: ModuleId) -> Vec<ModuleId> {
        let mut out: Vec<ModuleId> = Vec::new();
        fn walk(s: &Sema, m: ModuleId, out: &mut Vec<ModuleId>, depth: usize) {
            if depth > 64 {
                return;
            }
            for &e in &s.modules[m.index()].by_extension {
                if !out.contains(&e) {
                    out.push(e);
                }
                walk(s, e, out, depth + 1);
            }
        }
        walk(self, m, &mut out, 0);
        out.retain(|&x| x != m);
        out
    }

    /// `QvtEnvironmentBase.getAdditionalOperations(owner)` of the root environment of `env`.
    pub fn additional_operations(&mut self, env: ModuleId, owner: TypeId) -> Vec<OpRef> {
        let mut result = Vec::new();
        if owner == TypeId::OCL_VOID {
            self.all_contextual_operations(env, &mut result);
            for e in self.modules[env.index()].by_extension.clone() {
                self.all_contextual_operations(e, &mut result);
            }
            return result;
        }
        if self.ts.ck().is_collection_like(owner) {
            self.local_collection_ops(env, owner, true, &mut result);
            for e in self.modules[env.index()].by_extension.clone() {
                self.local_collection_ops(e, owner, true, &mut result);
            }
            for e in self.modules[env.index()].by_access.clone() {
                self.local_collection_ops(e, owner, true, &mut result);
            }
            return result;
        }
        // AbstractEnvironment.getAdditionalOperations
        self.resolver_additional_operations(env, owner, &mut result);
        for general in self.all_supertypes(owner) {
            self.resolver_additional_operations(env, general, &mut result);
        }
        result
    }

    fn all_contextual_operations(&self, env: ModuleId, out: &mut Vec<OpRef>) {
        for &o in &self.modules[env.index()].eoperations {
            if let OpRef::User(id) = o {
                if self.ops[id.index()].contextual {
                    out.push(o);
                }
            }
        }
    }

    /// `QvtTypeResolverImpl.getAdditionalOperations`: local, then the extended modules', then
    /// the access-imported libraries' local ones.
    fn resolver_additional_operations(&mut self, env: ModuleId, owner: TypeId, out: &mut Vec<OpRef>) {
        self.local_additional_operations(env, owner, out);
        for e in self.all_extended(env) {
            self.local_additional_operations(e, owner, out);
        }
        for e in self.modules[env.index()].by_access.clone() {
            if self.modules[e.index()].is_library {
                self.local_additional_operations(e, owner, out);
            }
        }
    }

    /// `extractContextualOperations`
    fn extract_ctx_ops(&mut self, env: ModuleId, owner: TypeId, out: &mut Vec<OpRef>) {
        let me = &self.modules[env.index()];
        if let Some((_, l)) = me.ctx_ops.iter().find(|(t, _)| *t == owner) {
            out.extend(l.iter().map(|&o| OpRef::User(o)));
            return;
        }
        if matches!(self.ts.types.get(owner), Type::Tuple(_)) {
            let keys: Vec<(TypeId, Vec<OpId>)> = me.ctx_ops.clone();
            for (k, l) in keys {
                if self.ts.ck().compatible(owner, k) {
                    out.extend(l.iter().map(|&o| OpRef::User(o)));
                    return;
                }
            }
        }
    }

    /// The library's non-imperative operations of an owner (`BasicTypeResolverImpl` shadow
    /// classes; exact owner, tuples by exact type match).
    fn delegate_ops(&mut self, env: ModuleId, owner: TypeId, out: &mut Vec<OpRef>) {
        let me = &self.modules[env.index()];
        if let Some((_, l)) = me.std_additional.iter().find(|(t, _)| *t == owner) {
            let o = std_owner_of(self, owner);
            out.extend(l.iter().map(|&op| OpRef::Std(op, o)));
        }
    }

    /// `QvtTypeResolverImpl.getLocalAdditionalOperations`
    fn local_additional_operations(&mut self, env: ModuleId, owner: TypeId, out: &mut Vec<OpRef>) {
        self.extract_ctx_ops(env, owner, out);
        self.delegate_ops(env, owner, out);
        let is_coll = self.ts.ck().is_collection_like(owner);
        let is_tuple = matches!(self.ts.types.get(owner), Type::Tuple(_));
        if self.modules[env.index()].defines_ocl_any_features && !is_coll && !is_tuple && owner != TypeId::OCL_ANY {
            self.extract_ctx_ops(env, TypeId::OCL_ANY, out);
            self.delegate_ops(env, TypeId::OCL_ANY, out);
        }
        if is_coll {
            self.local_collection_ops(env, owner, false, out);
        }
    }

    /// `getLocalCollectionAdditionalOperations`
    fn local_collection_ops(&mut self, env: ModuleId, coll: TypeId, include_super: bool, out: &mut Vec<OpRef>) {
        let types = self.modules[env.index()].additional_types.clone();
        for next in types {
            if !self.ts.ck().is_collection_like(next) {
                continue;
            }
            let matching = if include_super {
                self.ts.ck().compatible(coll, next)
            } else {
                self.ts.ck().exact(coll, next)
            };
            if matching {
                if let Some((_, l)) = self.modules[env.index()].ctx_ops.iter().find(|(t, _)| *t == next) {
                    out.extend(l.iter().map(|&o| OpRef::User(o)));
                }
                self.delegate_ops(env, next, out);
            }
        }
    }

    /// Supertypes of a type for additional features (`OCLStandardLibraryUtil.getAllSupertypes`
    /// for library types, `QVTUMLReflection.getAllSupertypes` for classes).
    pub fn all_supertypes(&mut self, t: TypeId) -> Vec<TypeId> {
        let ty = self.ts.types.get(t).clone();
        if self.ts.ck().is_predefined(t) {
            return match ty {
                Type::Collection(k, _) => match k {
                    CollKind::OrderedSet => {
                        let s = self.ts.types.collection(CollKind::Set, TypeId::T);
                        let c = self.ts.types.collection(CollKind::Collection, TypeId::T);
                        vec![s, c]
                    }
                    CollKind::Collection => Vec::new(),
                    _ => vec![self.ts.types.collection(CollKind::Collection, TypeId::T)],
                },
                Type::List(_) | Type::Dict(..) => Vec::new(),
                Type::Integer => vec![TypeId::REAL, TypeId::OCL_ANY],
                Type::OclAny | Type::Param(TParam::T | TParam::T2) => Vec::new(),
                _ => vec![TypeId::OCL_ANY],
            };
        }
        match ty {
            Type::Class(c) => {
                let mut v: Vec<TypeId> = Vec::new();
                for &s in self.ts.meta.class(c).all_supers.clone().iter() {
                    v.push(self.ts.types.class(s));
                }
                if self.ts.meta.class(c).package != self.ts.std.package {
                    v.push(self.ts.types.class(self.ts.std.element));
                }
                v
            }
            Type::Module(m) => {
                if self.modules[m as usize].is_library {
                    Vec::new()
                } else {
                    vec![self.ts.types.class(self.ts.std.transformation)]
                }
            }
            Type::ModelType(_) => vec![self.ts.types.class(self.ts.std.model)],
            _ => Vec::new(),
        }
    }

    // ---- matching ------------------------------------------------------------------------------

    /// `TypeCheckerImpl.resolveGenericType`
    fn resolve_generic_type(&mut self, owner: TypeId, param: TypeId, arg: TypeId) -> TypeId {
        // template parameter types of QVTo (KeyT, CommonT)
        if param == TypeId::KEY_T || param == TypeId::COMMON_T {
            if param == TypeId::KEY_T {
                if let Type::Dict(k, _) = *self.ts.types.get(owner) {
                    return k;
                }
            }
            return arg;
        }
        let owner_elem = |s: &Sema, owner: TypeId| -> TypeId {
            match *s.ts.types.get(owner) {
                Type::Collection(_, e) | Type::List(e) | Type::Dict(_, e) => {
                    if e == TypeId::OCL_VOID {
                        TypeId::T
                    } else {
                        e
                    }
                }
                _ => owner,
            }
        };
        let mut result = param;
        if param == TypeId::T {
            result = owner_elem(self, owner);
        } else {
            let pk = match *self.ts.types.get(param) {
                Type::Collection(k, e) => Some((k, e)),
                // ListType/DictionaryType are CollectionTypes of kind Collection
                Type::List(e) => Some((CollKind::Collection, e)),
                Type::Dict(_, e) => Some((CollKind::Collection, e)),
                _ => None,
            };
            if let Some((k, e)) = pk {
                if e == TypeId::T {
                    let m = owner_elem(self, owner);
                    result = self.ts.types.collection(k, m);
                } else if e == TypeId::T2 {
                    let am = match *self.ts.types.get(arg) {
                        Type::Collection(_, ae) | Type::List(ae) | Type::Dict(_, ae) => {
                            if ae == TypeId::OCL_VOID {
                                TypeId::T
                            } else {
                                ae
                            }
                        }
                        _ => arg,
                    };
                    result = self.ts.types.collection(k, am);
                }
            }
        }
        if let Type::List(e) = *self.ts.types.get(owner) {
            if result == TypeId::T && e == TypeId::OCL_VOID {
                return TypeId::OCL_VOID;
            }
        }
        result
    }

    /// `AbstractTypeChecker.matchArgsWithExactitude`: -1 when the arguments do not match.
    fn match_args_exactitude(&mut self, owner: TypeId, params: &[TypeId], args: &[Arg]) -> i32 {
        if params.len() != args.len() {
            return -1;
        }
        let mut exact = 0;
        for (p, a) in params.iter().zip(args) {
            let pop = self.ts.ck().ocl_type(*p);
            if matches!(self.ts.types.get(pop), Type::TypeType(_)) {
                if a.type_exp {
                    continue;
                }
                return -1;
            }
            let pop = self.resolve_generic_type(owner, pop, a.ty);
            if pop == TypeId::T {
                continue;
            }
            let r = self.ts.ck().relationship(a.ty, pop);
            if r & SUBTYPE == 0 {
                return -1;
            }
            if r == SAME_TYPE {
                exact += 1;
            }
        }
        exact
    }

    pub fn match_args(&mut self, owner: TypeId, params: &[TypeId], args: &[Arg]) -> bool {
        self.match_args_exactitude(owner, params, args) >= 0
    }

    /// `TypeCheckerImpl.findMostSpecificOperationMatching`. `Err` carries the ambiguous
    /// matches (`AmbiguousLookupException`).
    pub fn find_most_specific_operation(&mut self, owner: TypeId, name: &str, args: &[Arg]) -> Result<Option<OpRef>, Vec<OpRef>> {
        let mut ops = self.get_operations(owner);
        dedup_keep_first(&mut ops);
        let mut matches: Vec<OpRef> = Vec::new();
        for o in ops {
            if owner == TypeId::OCL_VOID {
                if let OpRef::User(id) = o {
                    let ctx = self.op_context(OpRef::User(id));
                    if ctx != Some(owner) && ctx.is_some() || ctx.is_none() {
                        if ctx != Some(owner) {
                            continue;
                        }
                    }
                }
            }
            if self.op_name(o) != name {
                continue;
            }
            let params = self.op_params(o);
            if self.match_args(owner, &params, args) && !matches.contains(&o) {
                matches.push(o);
            }
        }
        if matches.len() == 1 {
            return Ok(Some(matches[0]));
        } else if !matches.is_empty() {
            return self.most_specific_operation(&matches, args).map(Some);
        }
        if owner == TypeId::OCL_VOID || owner == TypeId::OCL_INVALID {
            if args.len() == 1 {
                let at = args[0].ty;
                if at != owner {
                    return self.find_most_specific_operation(at, name, args);
                }
            }
        }
        Ok(None)
    }

    /// `TypeCheckerImpl.getMostSpecificOperation`
    pub fn most_specific_operation(&mut self, matching: &[OpRef], args: &[Arg]) -> Result<OpRef, Vec<OpRef>> {
        let mut ambiguous: Vec<OpRef> = Vec::new();
        let mut most = matching[0];
        for &next in &matching[1..] {
            let mut rel = match self.more_specific_by_params(most, next, args) {
                None => continue,
                Some(r) => r,
            };
            if rel == Relation::Identical {
                let a = self.op_owner(most);
                let b = self.op_owner(next);
                match self.more_specific_type(a, b) {
                    Some(Relation::Identical) => {
                        if let Some(over) = self.select_overriding(most, next) {
                            most = over;
                        } else {
                            let resolving = self.cur_module;
                            let m1 = self.op_module(most);
                            let m2 = self.op_module(next);
                            if m1.is_none() || m2.is_none() {
                                if m1.is_some() {
                                    most = next;
                                }
                                rel = Relation::Ambiguous;
                            } else if m1 != Some(resolving) && m2 != Some(resolving) {
                                rel = Relation::Ambiguous;
                            } else if m2 == Some(resolving) {
                                most = next;
                            }
                        }
                    }
                    Some(Relation::Less) => most = next,
                    None => continue,
                    _ => {}
                }
            } else if rel == Relation::Less {
                most = next;
            }
            if rel == Relation::Ambiguous {
                for o in [most, next] {
                    if !ambiguous.contains(&o) {
                        ambiguous.push(o);
                    }
                }
            }
        }
        if !ambiguous.is_empty() {
            return Err(ambiguous);
        }
        Ok(most)
    }

    fn select_overriding(&self, a: OpRef, b: OpRef) -> Option<OpRef> {
        if self.op_overridden(a) == Some(b) {
            return Some(a);
        }
        if self.op_overridden(b) == Some(a) {
            return Some(b);
        }
        None
    }

    fn more_specific_by_params(&mut self, a: OpRef, b: OpRef, args: &[Arg]) -> Option<Relation> {
        if a == b {
            return Some(Relation::Identical);
        }
        let p1 = self.op_params(a);
        let p2 = self.op_params(b);
        if p1.is_empty() {
            return Some(Relation::Identical);
        }
        let mut result: Option<Relation> = None;
        for (i, (x, y)) in p1.iter().zip(p2.iter()).enumerate() {
            let r = self.more_specific_type(*x, *y)?;
            if r == Relation::Unrelated {
                let at = args[i].ty;
                if at == TypeId::OCL_VOID || at == TypeId::OCL_INVALID {
                    return Some(Relation::Ambiguous);
                }
                return None;
            }
            if i == 0 {
                result = Some(r);
                continue;
            }
            if result != Some(r)
                && result != Some(Relation::Identical)
                && r != Relation::Identical
            {
                return Some(Relation::Ambiguous);
            }
        }
        result
    }

    fn more_specific_type(&mut self, a: TypeId, b: TypeId) -> Option<Relation> {
        Some(match self.ts.ck().relationship(a, b) {
            STRICT_SUBTYPE => Relation::More,
            STRICT_SUPERTYPE => Relation::Less,
            SAME_TYPE => Relation::Identical,
            _ => Relation::Unrelated,
        })
    }

    // ---- properties ----------------------------------------------------------------------------

    /// `TypeCheckerImpl.findAttribute`
    pub fn find_attribute(&mut self, owner: TypeId, name: &str) -> Option<PropRef> {
        // owner instanceof EClass: getEStructuralFeature(name)
        let direct: Option<PropRef> = match self.ts.types.get(owner).clone() {
            Type::Class(c) => self.ts.meta.feature_by_name(c, name).map(PropRef::Feature),
            Type::Module(m) => {
                let me = &self.modules[m as usize];
                me.features.iter().copied().find(|f| self.feature_name(*f) == name).map(PropRef::from)
            }
            Type::Tuple(parts) => parts
                .iter()
                .position(|(n, _)| &**n == name)
                .map(|i| PropRef::TuplePart(owner, i as u32)),
            _ => None,
        };
        if let Some(p) = direct {
            let po = self.prop_owner(p);
            if self.ts.ck().compatible(owner, po) {
                return Some(p);
            }
        }
        let attrs = self.get_attributes(owner);
        let mut matches: Vec<PropRef> = Vec::new();
        for a in attrs {
            if self.prop_name(a) == name {
                if self.prop_owner(a) == owner {
                    return Some(a);
                }
                matches.push(a);
            }
        }
        if matches.len() == 1 {
            return Some(matches[0]);
        } else if !matches.is_empty() {
            return Some(self.most_specific_redefinition(&matches));
        }
        // renamed property
        let m = self.cur_module;
        let aliases = self.modules[m.index()].aliases.clone();
        for (new_name, alias_owner, orig) in aliases {
            if new_name == name && self.ts.ck().compatible(owner, alias_owner) {
                return self.find_attribute_plain(owner, &orig);
            }
        }
        None
    }

    fn find_attribute_plain(&mut self, owner: TypeId, name: &str) -> Option<PropRef> {
        let attrs = self.get_attributes(owner);
        let mut matches = Vec::new();
        for a in attrs {
            if self.prop_name(a) == name {
                if self.prop_owner(a) == owner {
                    return Some(a);
                }
                matches.push(a);
            }
        }
        match matches.len() {
            0 => None,
            1 => Some(matches[0]),
            _ => Some(self.most_specific_redefinition(&matches)),
        }
    }

    fn feature_name(&self, f: ModuleFeature) -> &str {
        match f {
            ModuleFeature::Global(g) => &self.globals[g.index()].ir.name,
            ModuleFeature::Interm(i) => &self.interm_props[i.index()].ir.name,
        }
    }

    /// `mostSpecificRedefinition`: the feature of the most specific owner.
    fn most_specific_redefinition(&mut self, feats: &[PropRef]) -> PropRef {
        let mut owners: Vec<(TypeId, PropRef)> = Vec::new();
        for &f in feats {
            let o = self.prop_owner(f);
            if let Some(e) = owners.iter_mut().find(|(t, _)| *t == o) {
                e.1 = f;
            } else {
                owners.push((o, f));
            }
        }
        let mut classifiers: Vec<TypeId> = owners.iter().map(|(t, _)| *t).collect();
        'outer: loop {
            for i in 0..classifiers.len() {
                let sups = self.all_supertypes(classifiers[i]);
                let before = classifiers.len();
                classifiers.retain(|c| !sups.contains(c));
                if classifiers.len() != before {
                    continue 'outer;
                }
            }
            break;
        }
        let first = classifiers[0];
        owners.iter().find(|(t, _)| *t == first).unwrap().1
    }

    /// `AbstractTypeChecker.getAttributes(owner)` with QVTo's reflection.
    pub fn get_attributes(&mut self, owner: TypeId) -> Vec<PropRef> {
        let mut result: Vec<PropRef> = Vec::new();
        match self.ts.types.get(owner).clone() {
            Type::TypeType(_) => return result,
            Type::Tuple(parts) => {
                for i in 0..parts.len() {
                    result.push(PropRef::TuplePart(owner, i as u32));
                }
            }
            t if self.ts.ck().is_predefined(owner) => {
                let _ = t;
            }
            _ => {
                let o = self.ts.ck().ocl_type(owner);
                if !self.ts.ck().is_predefined(o) {
                    result.extend(self.uml_attributes(o));
                }
            }
        }
        let add = self.additional_attributes(owner);
        result.extend(add);
        result
    }

    /// `QVTUMLReflection.getAttributes`: a class's all features; a module's properties (with
    /// those of extended modules), without contextual properties and hidden ones.
    fn uml_attributes(&mut self, owner: TypeId) -> Vec<PropRef> {
        match self.ts.types.get(owner).clone() {
            Type::Class(c) => self
                .ts
                .meta
                .class(c)
                .all_features
                .iter()
                .map(|&f| PropRef::Feature(f))
                .collect(),
            Type::Module(m) => {
                let mut out = Vec::new();
                self.module_attributes(ModuleId(m), &mut out, 0);
                out
            }
            _ => Vec::new(),
        }
    }

    fn module_attributes(&self, m: ModuleId, out: &mut Vec<PropRef>, depth: usize) {
        if depth > 64 {
            return;
        }
        let me = &self.modules[m.index()];
        for f in &me.features {
            if let ModuleFeature::Global(g) = f {
                if !self.globals[g.index()].hidden {
                    let p = PropRef::Global(*g);
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
        }
        for &(e, extension) in &me.module_imports {
            if extension {
                let mut sub = Vec::new();
                self.module_attributes(e, &mut sub, depth + 1);
                for p in sub {
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
        }
    }

    /// `AbstractEnvironment.getAdditionalAttributes` with `QvtTypeResolverImpl`'s.
    fn additional_attributes(&mut self, owner: TypeId) -> Vec<PropRef> {
        let env = self.cur_module;
        let mut out = Vec::new();
        self.resolver_additional_attributes(env, owner, &mut out);
        for g in self.all_supertypes(owner) {
            self.resolver_additional_attributes(env, g, &mut out);
        }
        out
    }

    fn resolver_additional_attributes(&mut self, env: ModuleId, owner: TypeId, out: &mut Vec<PropRef>) {
        self.local_additional_attributes(env, owner, out);
        for e in self.all_extended(env) {
            self.local_additional_attributes(e, owner, out);
        }
        for e in self.modules[env.index()].by_access.clone() {
            if self.modules[e.index()].is_library {
                self.local_additional_attributes(e, owner, out);
            }
        }
    }

    fn local_additional_attributes(&mut self, env: ModuleId, owner: TypeId, out: &mut Vec<PropRef>) {
        for f in &self.modules[env.index()].features {
            if let ModuleFeature::Interm(i) = f {
                let p = &self.interm_props[i.index()];
                if !p.hidden && p.context == Some(owner) {
                    out.push(PropRef::Interm(*i));
                }
            }
        }
    }

    /// `lookupProperty(owner, name)` with escaped names.
    pub fn lookup_property(&mut self, owner: TypeId, name: &str) -> Option<PropRef> {
        let r = self.find_attribute(owner, name);
        if r.is_none() {
            if let Some(un) = name.strip_prefix('_') {
                return self.find_attribute(owner, un);
            }
        }
        r
    }

    /// `QvtOperationalEnv.lookupMappingOperations`
    pub fn lookup_mapping_operations(&mut self, owner: TypeId, name: &str) -> Vec<OpId> {
        let ops = self.get_operations(owner);
        let mut r = Vec::new();
        for o in ops {
            if let OpRef::User(id) = o {
                if self.ops[id.index()].ir.name == name && self.op_is_mapping(o) {
                    r.push(id);
                }
            }
        }
        r
    }
}

fn dedup_keep_first(v: &mut Vec<OpRef>) {
    let mut seen = std::collections::HashSet::new();
    v.retain(|o| seen.insert(*o));
}

fn std_class_owner(std: &qvto_types::StdClasses, c: qvto_model::ClassId) -> Option<StdOwner> {
    if c == std.element {
        Some(StdOwner::Element)
    } else if c == std.exception {
        Some(StdOwner::Exception)
    } else if c == std.model {
        Some(StdOwner::Model)
    } else if c == std.status {
        Some(StdOwner::Status)
    } else if c == std.transformation {
        Some(StdOwner::Transformation)
    } else {
        None
    }
}

/// The library owner standing for a type key of the library's additional operations.
fn std_owner_of(s: &Sema, t: TypeId) -> StdOwner {
    match s.ts.types.get(t) {
        Type::String => StdOwner::String,
        Type::Integer => StdOwner::Integer,
        Type::OclAny => StdOwner::OclAny,
        Type::List(_) => StdOwner::List,
        Type::Dict(..) => StdOwner::Dict,
        Type::Collection(k, _) => match k {
            CollKind::Collection => StdOwner::Collection,
            CollKind::Set => StdOwner::Set,
            CollKind::OrderedSet => StdOwner::OrderedSet,
            CollKind::Bag => StdOwner::Bag,
            CollKind::Sequence => StdOwner::Sequence,
        },
        _ => StdOwner::OclAny,
    }
}

#[allow(dead_code)]
fn _unused(_: ModuleId) -> ModuleId {
    STDLIB_MODULE
}

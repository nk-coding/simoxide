//! Variables and implicit sources: Eclipse's `AbstractEnvironment` variable lists per
//! environment level, `QvtEnvironmentBase`/`QvtOperationalEnv` implicit-source lookup (the
//! module's `this` takes precedence over `self`, extended modules and accessed libraries at the
//! root).

use crate::analyzer::Sema;
use crate::env::{FnState, Scope, VarEntry, VarRef};
use crate::ir::{ModelParamRef, ModuleId, OpId};
use crate::ops::Arg;
use qvto_types::{Type, TypeId};

/// An ambiguous implicit source (`AmbiguousLookupException`).
pub(crate) struct AmbiguousSource(pub Vec<VarRef>);

impl<'r> Sema<'r> {
    /// Starts analysing code of a module: the module's environment level (`this`, the model
    /// parameters).
    pub fn begin_code(&mut self, module: ModuleId, op: Option<OpId>) {
        let mut fun = FnState {
            op,
            module: Some(module),
            ..Default::default()
        };
        let mut scope = Scope::default();
        scope.vars.push(VarEntry {
            name: "this".into(),
            var: VarRef::This(module),
            explicit: false,
            named: true,
        });
        for (i, p) in self.modules[module.index()].model_params.iter().enumerate() {
            if scope.vars.iter().any(|v| v.name == p.name) {
                continue;
            }
            scope.vars.push(VarEntry {
                name: p.name.clone(),
                var: VarRef::ModelParam(ModelParamRef {
                    module,
                    index: i as u32,
                }),
                explicit: true,
                named: true,
            });
        }
        fun.scopes.push(scope);
        self.fun = fun;
    }

    pub fn end_code(&mut self) {
        self.fun = FnState::default();
    }

    pub fn var_type(&self, v: VarRef) -> TypeId {
        match v {
            VarRef::Local(l) => self.fun.locals[l.index()].ty,
            VarRef::This(m) => self.modules[m.index()].ty,
            VarRef::ModelParam(p) => {
                self.modules[p.module.index()].model_params[p.index as usize].ty
            }
        }
    }

    pub fn var_name(&self, v: VarRef) -> String {
        match v {
            VarRef::Local(l) => self.fun.locals[l.index()].name.clone(),
            VarRef::This(_) => "this".into(),
            VarRef::ModelParam(p) => self.modules[p.module.index()].model_params[p.index as usize]
                .name
                .clone(),
        }
    }

    /// `addElement`: false when the name is already defined in the current level.
    pub fn add_var(&mut self, name: &str, v: VarRef, explicit: bool) -> bool {
        if self.lookup_local(name).is_some() {
            return false;
        }
        self.fun.top().vars.push(VarEntry {
            name: name.to_string(),
            var: v,
            explicit,
            named: true,
        });
        true
    }

    /// `addImplicitVariableForProperties`: an implicit source for properties that is not a
    /// named element of the environment.
    pub fn add_implicit_property_source(&mut self, name: &str, v: VarRef) {
        self.fun.top().vars.push(VarEntry {
            name: name.to_string(),
            var: v,
            explicit: false,
            named: false,
        });
    }

    /// `addElement(null, ...)`: a generated name (`temp<n>`).
    pub fn add_var_generated(&mut self, v: VarRef) -> String {
        loop {
            self.fun.name_counter += 1;
            let n = format!("temp{}", self.fun.name_counter);
            if self.lookup_var(&n).is_none() {
                self.fun.top().vars.push(VarEntry {
                    name: n.clone(),
                    var: v,
                    explicit: false,
                    named: true,
                });
                return n;
            }
        }
    }

    /// `deleteElement`: removes every variable of that name from the current level.
    pub fn delete_var(&mut self, name: &str) {
        self.fun.top().vars.retain(|v| !(v.named && v.name == name));
    }

    fn lookup_local_at(&self, level: usize, name: &str) -> Option<VarRef> {
        let s = &self.fun.scopes[level];
        let r = s
            .vars
            .iter()
            .find(|v| v.named && v.name == name)
            .map(|v| v.var);
        if r.is_some() {
            return r;
        }
        if s.parent_local && level > 0 {
            return self.lookup_local_at(level - 1, name);
        }
        None
    }

    /// `lookupLocal`
    pub fn lookup_local(&self, name: &str) -> Option<VarRef> {
        let level = self.fun.scopes.len().checked_sub(1)?;
        let r = self.lookup_local_at(level, name);
        if r.is_none() {
            if let Some(u) = name.strip_prefix('_') {
                return self.lookup_local_at(level, u);
            }
        }
        r
    }

    /// `lookup`: innermost level first.
    pub fn lookup_var(&self, name: &str) -> Option<VarRef> {
        for s in self.fun.scopes.iter().rev() {
            let r = s
                .vars
                .iter()
                .find(|v| v.named && v.name == name)
                .or_else(|| {
                    name.strip_prefix('_')
                        .and_then(|u| s.vars.iter().find(|v| v.named && v.name == u))
                });
            if let Some(v) = r {
                return Some(v.var);
            }
        }
        None
    }

    /// The variables visible in the current level and its parents (`getVariables`, explicit
    /// ones), for block scoping.
    pub fn scope_mark(&self) -> usize {
        self.fun.top_len()
    }

    // ---- implicit sources ----------------------------------------------------------------------

    /// Whether an operation applies to `owner` (`AbstractEnvironment.lookupOperation`, used to
    /// find implicit sources).
    fn has_operation(&mut self, owner: TypeId, name: &str, args: &[Arg]) -> bool {
        match self.find_most_specific_operation(owner, name, args) {
            Ok(Some(_)) | Err(_) => true,
            Ok(None) => {
                if let Some(u) = name.strip_prefix('_') {
                    if self.has_operation(owner, u, args) {
                        return true;
                    }
                }
                // classic OCL's fall-backs
                if owner == TypeId::UNLIMITED_NATURAL {
                    return self.has_operation(TypeId::INTEGER, name, args);
                }
                if let Type::Collection(k, _) = *self.ts.types.get(owner) {
                    if k != qvto_types::CollKind::Collection {
                        let c = self
                            .ts
                            .types
                            .collection(qvto_types::CollKind::Collection, TypeId::T);
                        return self.has_operation(c, name, args);
                    }
                }
                false
            }
        }
    }

    fn abstract_implicit_op(&mut self, level: usize, name: &str, args: &[Arg]) -> Option<VarRef> {
        let vars: Vec<VarEntry> = self.fun.scopes[level].vars.clone();
        for v in vars.iter().rev() {
            if v.explicit || !v.named {
                continue;
            }
            let t = self.var_type(v.var);
            if t == TypeId::NULL {
                continue;
            }
            if self.has_operation(t, name, args) {
                return Some(v.var);
            }
        }
        None
    }

    /// `QvtOperationalEnv.lookupImplicitSourceForOperation`
    pub fn implicit_source_for_operation(
        &mut self,
        name: &str,
        args: &[Arg],
    ) -> Result<Option<VarRef>, AmbiguousSource> {
        let top = self.fun.scopes.len() - 1;
        self.implicit_op_at(top, name, args)
    }

    fn implicit_op_at(
        &mut self,
        level: usize,
        name: &str,
        args: &[Arg],
    ) -> Result<Option<VarRef>, AmbiguousSource> {
        let mut r = self.abstract_implicit_op(level, name, args);
        if r.is_none() {
            if level > 0 {
                return self.implicit_op_at(level - 1, name, args);
            }
            r = self.root_implicit_op(name, args)?;
        }
        let tentative = r;
        if (r.is_none() || r.is_some_and(|v| self.var_name(v) == "self")) && level > 0 {
            r = self.implicit_op_at(level - 1, name, args)?;
            if tentative.is_some() && r.is_none() {
                r = tentative;
            }
        }
        Ok(r)
    }

    /// The root environment's search in extended modules and accessed libraries.
    fn root_implicit_op(
        &mut self,
        name: &str,
        args: &[Arg],
    ) -> Result<Option<VarRef>, AmbiguousSource> {
        let Some(m) = self.fun.module else {
            return Ok(None);
        };
        let mut found = Vec::new();
        let saved = self.cur_module;
        for e in self.all_extended(m) {
            self.cur_module = e;
            let t = self.modules[e.index()].ty;
            if self.has_operation(t, name, args) {
                found.push(VarRef::This(e));
            }
        }
        for e in self.modules[m.index()].by_access.clone() {
            if self.modules[e.index()].is_library {
                self.cur_module = e;
                let t = self.modules[e.index()].ty;
                if self.has_operation(t, name, args) {
                    found.push(VarRef::This(e));
                }
            }
        }
        self.cur_module = saved;
        if found.len() > 1 {
            return Err(AmbiguousSource(found));
        }
        Ok(found.first().copied())
    }

    /// Whether a property applies to a variable's type for implicit sources
    /// (`lookupImplicitSourceForPropertyInternal`).
    fn property_fits(&mut self, owner: TypeId, name: &str) -> bool {
        match self.lookup_property(owner, name) {
            Some(p) => {
                let po = self.prop_owner(p);
                self.ts.ck().compatible(owner, po)
            }
            None => false,
        }
    }

    fn abstract_implicit_prop(&mut self, level: usize, name: &str) -> Option<VarRef> {
        let vars: Vec<VarEntry> = self.fun.scopes[level].vars.clone();
        for v in vars.iter().rev() {
            if v.explicit {
                continue;
            }
            let t = self.var_type(v.var);
            if t == TypeId::NULL {
                continue;
            }
            if self.property_fits(t, name) {
                return Some(v.var);
            }
        }
        None
    }

    /// `QvtOperationalEnv.lookupImplicitSourceForProperty`
    pub fn implicit_source_for_property(&mut self, name: &str) -> Option<VarRef> {
        let top = self.fun.scopes.len() - 1;
        self.implicit_prop_at(top, name)
    }

    fn implicit_prop_at(&mut self, level: usize, name: &str) -> Option<VarRef> {
        let mut r = self.abstract_implicit_prop(level, name);
        if r.is_none() {
            if level > 0 {
                return self.implicit_prop_at(level - 1, name);
            }
            r = self.root_implicit_prop(name);
        }
        let tentative = r;
        if (r.is_none() || r.is_some_and(|v| self.var_name(v) == "self")) && level > 0 {
            r = self.implicit_prop_at(level - 1, name);
            if tentative.is_some() && r.is_none() {
                r = tentative;
            }
        }
        r
    }

    fn root_implicit_prop(&mut self, name: &str) -> Option<VarRef> {
        let m = self.fun.module?;
        self.module_implicit_prop(m, name, 0)
    }

    /// `QvtEnvironmentBase.lookupImplicitSourceForProperty` of a module environment: its own
    /// `this`, then the extended modules (recursively), then the accessed libraries.
    fn module_implicit_prop(&mut self, m: ModuleId, name: &str, depth: usize) -> Option<VarRef> {
        if depth > 32 {
            return None;
        }
        let saved = self.cur_module;
        self.cur_module = m;
        let t = self.modules[m.index()].ty;
        let own = if m != crate::analyzer::STDLIB_MODULE && self.property_fits(t, name) {
            Some(VarRef::This(m))
        } else {
            None
        };
        self.cur_module = saved;
        if own.is_some() {
            return own;
        }
        let ext: Vec<ModuleId> = self.modules[m.index()]
            .by_extension
            .iter()
            .copied()
            .filter(|x| *x != crate::analyzer::STDLIB_MODULE)
            .collect();
        for e in ext {
            if let Some(r) = self.module_implicit_prop(e, name, depth + 1) {
                return Some(r);
            }
        }
        for e in self.modules[m.index()].by_access.clone() {
            if self.modules[e.index()].is_library {
                if let Some(r) = self.module_implicit_prop(e, name, depth + 1) {
                    return Some(r);
                }
            }
        }
        None
    }
}

impl FnState {
    pub fn top_len(&self) -> usize {
        self.scopes.last().map(|s| s.vars.len()).unwrap_or(0)
    }
}

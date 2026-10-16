//! Model extents (`ModelParameterExtent`): the objects of a model parameter.
//!
//! An extent holds its initial objects (the roots of the bound resources) and the objects
//! created into it, in creation order. Created objects without a resource go into an in-memory
//! resource `extent:/N` of the store, as Eclipse's `ExtentResource`. Contained objects are
//! purged from the created list every `max(150, size)` additions and on every query, and then
//! stay purged (an object detached again later is no root any more, compat row 25).

use crate::vm::ObjHandle;
use qvto_model::{ModelStore, ResourceId};
use qvto_runtime::ObjRef;
use qvto_sema::ir::{Direction, ModuleId};
use std::collections::HashMap;

const INITIAL_EXTENT_SIZE: usize = 150;

#[derive(Clone, Debug)]
pub struct Extent {
    /// The model parameter (`None`: the unbound or the intermediate extent).
    pub param: Option<(ModuleId, u32)>,
    pub direction: Direction,
    /// `name : Module::ModelType` for messages.
    pub label: String,
    pub initial: Vec<ObjRef>,
    /// Initial objects that had a container when bound, with that container.
    pub initial_container: HashMap<ObjRef, ObjRef>,
    pub additional: Vec<ObjRef>,
    count_after_purge: usize,
    purge_limit: usize,
    /// The in-memory resource (`extent:/N`), created on demand.
    pub resource: Option<ResourceId>,
    /// Holds engine-local objects only (intermediate data).
    pub local_only: bool,
    /// The model type of an extent without parameter (`createEmptyModel`).
    pub model_type: Option<qvto_sema::ir::ModelTypeId>,
}

impl Extent {
    pub fn new(param: Option<(ModuleId, u32)>, direction: Direction, label: String) -> Extent {
        Extent {
            param,
            direction,
            label,
            initial: Vec::new(),
            initial_container: HashMap::new(),
            additional: Vec::new(),
            count_after_purge: 0,
            purge_limit: INITIAL_EXTENT_SIZE,
            resource: None,
            local_only: false,
            model_type: None,
        }
    }

    pub fn is_readonly(&self) -> bool {
        self.param.is_some() && self.direction == Direction::In
    }
}

/// Container of an object as an `ObjRef` (local objects have none).
fn container<S: ModelStore>(s: &S, o: ObjRef) -> Option<ObjRef>
where
    S::Obj: ObjHandle,
{
    if o.is_local() {
        return None;
    }
    s.container(S::Obj::from_u32(o.index()))
        .map(|(p, _)| ObjRef::host(p.to_u32()))
}

impl Extent {
    /// Binds the initial objects (the constructor of `ModelParameterExtent`).
    pub fn bind_initial<S: ModelStore>(&mut self, s: &mut S, objs: Vec<ObjRef>, next_uri: &mut u32)
    where
        S::Obj: ObjHandle,
    {
        for &o in &objs {
            if let Some(c) = container(s, o) {
                self.initial_container.insert(o, c);
            }
        }
        if !self.is_readonly() {
            for &o in &objs {
                if o.is_local() {
                    continue;
                }
                let h = S::Obj::from_u32(o.index());
                if s.resource_of(h).is_none() {
                    let r = self.in_memory(s, next_uri);
                    let _ = s.add_root(r, h, None);
                }
            }
        }
        self.initial = objs;
    }

    fn in_memory<S: ModelStore>(&mut self, s: &mut S, next_uri: &mut u32) -> ResourceId {
        if let Some(r) = self.resource {
            return r;
        }
        *next_uri += 1;
        let r = s.create_resource(&format!("extent:/{}", *next_uri));
        self.resource = Some(r);
        r
    }

    /// `addObject`.
    pub fn add_object<S: ModelStore>(&mut self, s: &mut S, o: ObjRef, next_uri: &mut u32)
    where
        S::Obj: ObjHandle,
    {
        self.additional.push(o);
        if !o.is_local() && !self.local_only {
            let h = S::Obj::from_u32(o.index());
            if s.resource_of(h).is_none() {
                let r = self.in_memory(s, next_uri);
                let _ = s.add_root(r, h, None);
            }
        }
        self.count_after_purge += 1;
        if self.count_after_purge >= self.purge_limit {
            self.purge_additional(s);
            self.count_after_purge = 0;
            self.purge_limit = self.additional.len().max(INITIAL_EXTENT_SIZE);
        }
    }

    fn purge_additional<S: ModelStore>(&mut self, s: &mut S)
    where
        S::Obj: ObjHandle,
    {
        let res = self.resource;
        let mut keep = Vec::with_capacity(self.additional.len());
        for &o in &self.additional {
            match container(s, o) {
                None => keep.push(o),
                Some(_) => {
                    // contained elsewhere: drop it from the extent resource
                    if let Some(r) = res {
                        let h = S::Obj::from_u32(o.index());
                        if s.direct_resource(h) == Some(r) {
                            s.remove_root(r, h);
                        }
                    }
                }
            }
        }
        self.additional = keep;
    }

    fn purge_initial<S: ModelStore>(&mut self, s: &S)
    where
        S::Obj: ObjHandle,
    {
        let ic = &self.initial_container;
        self.initial.retain(|&o| match container(s, o) {
            None => true,
            Some(c) => ic.get(&o) == Some(&c),
        });
    }

    /// `purgeContents()`.
    pub fn purge<S: ModelStore>(&mut self, s: &mut S)
    where
        S::Obj: ObjHandle,
    {
        self.purge_initial(s);
        self.purge_additional(s);
    }

    /// `getRootObjects()`.
    pub fn root_objects<S: ModelStore>(&self, s: &S) -> Vec<ObjRef>
    where
        S::Obj: ObjHandle,
    {
        let mut out = Vec::new();
        for &o in &self.initial {
            match container(s, o) {
                None => out.push(o),
                Some(c) if self.initial_container.get(&o) == Some(&c) => out.push(o),
                _ => {}
            }
        }
        for &o in &self.additional {
            if container(s, o).is_none() {
                out.push(o);
            }
        }
        out
    }

    /// `getAllObjects()`: each root followed by its contents (pre-order).
    pub fn all_objects<S: ModelStore>(&mut self, s: &mut S) -> Vec<ObjRef>
    where
        S::Obj: ObjHandle,
    {
        self.purge(s);
        let mut out = Vec::new();
        for &o in self.initial.iter().chain(self.additional.iter()) {
            out.push(o);
            if !o.is_local() {
                for c in s.all_contents(S::Obj::from_u32(o.index())) {
                    out.push(ObjRef::host(c.to_u32()));
                }
            }
        }
        out
    }

    /// `getContents()` at the end of a run: the root objects (initial first), detached from the
    /// in-memory resource, which is removed.
    pub fn contents<S: ModelStore>(&mut self, s: &mut S) -> Vec<ObjRef>
    where
        S::Obj: ObjHandle,
    {
        self.purge(s);
        let mut seen = std::collections::HashSet::new();
        let mut roots = Vec::new();
        for &o in self.initial.iter().chain(self.additional.iter()) {
            if seen.insert(o) {
                roots.push(o);
            }
        }
        if let Some(r) = self.resource {
            for &o in &roots {
                if o.is_local() {
                    continue;
                }
                let h = S::Obj::from_u32(o.index());
                if s.direct_resource(h) == Some(r) {
                    s.remove_root(r, h);
                }
            }
        }
        roots
    }
}

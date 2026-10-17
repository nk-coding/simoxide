//! The store: [`Session`] (what lives as long as the host's QVTo set-up: the registry with
//! SimOxide's tables and the metamodels loaded later, their metamodel objects, the journal
//! switch, the id generator) and [`SimStore`], the `ModelStore` view of a session over a
//! SimOxide model graph.
//!
//! Since merge step M5 the model graph *is* the shared EMF graph (`simoxide_emf::Graph`): every
//! object lives in it (one home), PCM objects as loaded by SimOxide, models of other metamodels
//! (the PRM, SPD models) loaded next to them, and the session's metamodel objects, copied in
//! when a store is first made over the graph. The store's handles are the graph's object ids
//! (`ObjId.0`), its resources the graph's resources; the graph's own `ModelStore` gives EMF's
//! semantics and records the journal. This crate is the bridge until M6 moves its callers onto
//! the graph directly.

use crate::journal::{Change, ChangeKind, Journal};
use crate::registry;
use qvto_emf::ids::UuidGen;
use qvto_emf::{IdPolicy, ObjId, ResourceSet};
use qvto_model::{
    ClassId, Classifier, Datum, FeatureId, MetaOperation, MetaRegistry, ModelResult, ModelStore,
    OwnedDatum, PackageId, ResourceId,
};
use simoxide_model::raw::Graph;
use std::path::Path;

/// No handle reaches this any more (one home since M5); kept for the callers' checks.
pub const EXT_BASE: u32 = 1 << 30;

/// Everything that lives as long as the host's set-up: a prototype graph with the registry
/// (SimOxide's tables, Ecore, metamodels loaded later) and the metamodel objects, the journal
/// switch and buffer, the id policy and generator. Plain data (`Clone`: a forked run's state).
#[derive(Clone)]
pub struct Session {
    pub(crate) rs: ResourceSet,
    /// Classes `0..graph_classes` are SimOxide's (the static tables).
    graph_classes: u32,
    journal: Option<Journal>,
    ids: (IdPolicy, Option<UuidGen>),
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// A session for SimOxide's metamodels: the registry of its generated tables (ids equal to
    /// SimOxide's), Ecore, and the metamodel objects of every SimOxide package, created under
    /// the packages' `platform:/plugin` locations so that `.ecore` files referring to them load.
    pub fn new() -> Session {
        let mut rs = ResourceSet::with_graph(simoxide_model::raw::pcm_graph());
        let meta = rs.graph.meta();
        let ops = meta.static_operations(rs.graph.ecore().package);
        let tops = meta.static_top_packages();
        rs.materialize(&tops, &ops)
            .expect("metamodel objects of SimOxide's packages");
        Session::from_parts(rs, registry::SimTables::get().classes)
    }

    /// A session whose graph classes are those of `meta` (built in code; Ecore is added after
    /// them): for tests and hosts with other metamodels. `materialize` names the top-level
    /// packages that get metamodel objects, with locations.
    pub fn with_registry(meta: MetaRegistry, materialize: &[(PackageId, &str)]) -> Session {
        let graph_classes = meta.class_count() as u32;
        let mut rs = ResourceSet::with_graph(qvto_emf::Graph::with_registry(meta));
        rs.ecore_resource();
        if !materialize.is_empty() {
            rs.materialize(materialize, &[])
                .expect("metamodel objects of the given packages");
        }
        Session::from_parts(rs, graph_classes)
    }

    fn from_parts(rs: ResourceSet, graph_classes: u32) -> Session {
        Session {
            rs,
            graph_classes,
            journal: None,
            ids: (IdPolicy::None, None),
        }
    }

    /// An empty model graph for this session (its registry, no objects).
    pub fn new_graph(&self) -> Graph {
        self.rs.graph.empty_like()
    }

    /// The registry: compile transformations against a clone of it after all metamodels are
    /// loaded (`qvto::compile_with_operations(.., session.meta().clone(), &session.operations())`).
    pub fn meta(&self) -> &MetaRegistry {
        self.rs.graph.meta()
    }

    /// The `EOperation`s of all registered metamodels, for the analyser.
    pub fn operations(&self) -> Vec<MetaOperation> {
        qvto_emf::operations::meta_operations(&self.rs.graph)
    }

    /// The OCL derivations of the registered metamodels' features, for the analyser
    /// (`qvto::compile_with_metamodel`).
    pub fn derivations(&self) -> Vec<qvto_model::MetaDerivation> {
        qvto_emf::operations::meta_derivations(&self.rs.graph)
    }

    /// Whether `c` is a class of SimOxide's tables.
    pub fn is_graph_class(&self, c: ClassId) -> bool {
        c.0 < self.graph_classes
    }

    /// The prototype: the registry and the metamodel objects (no models).
    pub fn resource_set(&self) -> &ResourceSet {
        &self.rs
    }

    /// The prototype, for URI maps, metamodel locations, loading metamodels.
    pub fn resource_set_mut(&mut self) -> &mut ResourceSet {
        &mut self.rs
    }

    /// Applies a `qvto_emf` configuration (URI maps, metamodel index by nsURI, aliases).
    pub fn configure(&mut self, c: &qvto_emf::config::Config) {
        for (from, to) in &c.maps {
            self.rs.map_uri(from, to);
        }
        for (key, ns) in &c.aliases {
            self.rs.add_alias(key, ns);
        }
        for (ns, loc) in &c.ns_index {
            if self.rs.ns_location(ns).is_none() {
                self.rs.add_ns_location(ns, loc);
            }
        }
    }

    /// Loads and registers the metamodel of a namespace URI (through the configured index) or
    /// at a location, with the metamodels it refers to; SimOxide's packages are not loaded
    /// again (`.ecore` files refer to their metamodel objects).
    pub fn load_metamodel(&mut self, uri: &str) -> Result<Vec<PackageId>, String> {
        if self.meta().package_by_ns_uri(uri).is_some() {
            return Ok(Vec::new());
        }
        if let Some(loc) = self.rs.ns_location(uri).map(String::from) {
            self.rs.load_metamodel(&loc)
        } else {
            let packages = self.rs.load_metamodel(uri)?;
            // a metamodel by file location: the registry key is the location
            if self.meta().package_by_ns_uri(uri).is_none()
                && let Some(&p) = packages.first()
            {
                self.rs.graph.meta_mut().register_alias(uri, p);
            }
            Ok(packages)
        }
    }

    /// Registers a metamodel given as `.ecore` text at `uri` (e.g. embedded in the host).
    pub fn load_metamodel_text(&mut self, uri: &str, text: &str) -> Result<Vec<PackageId>, String> {
        let r = self.rs.load_text(uri, text)?;
        self.rs.register(r)
    }

    /// Starts recording changes (a new, empty journal).
    pub fn start_journal(&mut self) {
        self.journal = Some(Journal::default());
    }

    /// The recorded changes; recording continues with an empty journal.
    pub fn take_journal(&mut self) -> Journal {
        match self.journal.as_mut() {
            Some(j) => std::mem::take(j),
            None => Journal::default(),
        }
    }

    /// Stops recording.
    pub fn stop_journal(&mut self) -> Option<Journal> {
        self.journal.take()
    }

    pub fn set_id_policy(&mut self, p: IdPolicy) {
        self.ids = (
            p,
            match p {
                IdPolicy::None => None,
                IdPolicy::Uuid { seed } => Some(UuidGen::new(seed)),
            },
        );
    }

    /// The store over `g` (a model graph of this session's registry).
    pub fn store<'a>(&'a mut self, g: &'a mut Graph) -> SimStore<'a> {
        SimStore::new(g, self)
    }
}

fn h(o: ObjId) -> u32 {
    o.0
}

fn map_datum<A: Copy, B>(d: Datum<'_, A>, f: impl Fn(A) -> B) -> Datum<'_, B> {
    match d {
        Datum::Obj(x) => Datum::Obj(f(x)),
        Datum::Null => Datum::Null,
        Datum::Bool(b) => Datum::Bool(b),
        Datum::Int(i) => Datum::Int(i),
        Datum::Long(i) => Datum::Long(i),
        Datum::Short(i) => Datum::Short(i),
        Datum::Byte(i) => Datum::Byte(i),
        Datum::Char(c) => Datum::Char(c),
        Datum::Double(x) => Datum::Double(x),
        Datum::Float(x) => Datum::Float(x),
        Datum::Str(s) => Datum::Str(s),
        Datum::Enum(e, i) => Datum::Enum(e, i),
        Datum::Other(s) => Datum::Other(s),
    }
}

fn owned(d: OwnedDatum<ObjId>) -> OwnedDatum<u32> {
    match d {
        OwnedDatum::Obj(x) => OwnedDatum::Obj(x.0),
        OwnedDatum::Null => OwnedDatum::Null,
        OwnedDatum::Bool(b) => OwnedDatum::Bool(b),
        OwnedDatum::Int(i) => OwnedDatum::Int(i),
        OwnedDatum::Long(i) => OwnedDatum::Long(i),
        OwnedDatum::Short(i) => OwnedDatum::Short(i),
        OwnedDatum::Byte(i) => OwnedDatum::Byte(i),
        OwnedDatum::Char(c) => OwnedDatum::Char(c),
        OwnedDatum::Double(x) => OwnedDatum::Double(x),
        OwnedDatum::Float(x) => OwnedDatum::Float(x),
        OwnedDatum::Str(s) => OwnedDatum::Str(s),
        OwnedDatum::Enum(e, i) => OwnedDatum::Enum(e, i),
        OwnedDatum::Other(s) => OwnedDatum::Other(s),
    }
}

/// The graph's journal in the store's handles.
fn convert(j: qvto_emf::Journal) -> Vec<Change> {
    use qvto_emf::ChangeKind as G;
    j.changes
        .into_iter()
        .map(|c| Change {
            obj: c.obj.0,
            kind: match c.kind {
                G::Created(class) => ChangeKind::Created(class),
                G::Set { feature, old, new } => ChangeKind::Set {
                    feature,
                    old: owned(old),
                    new: owned(new),
                },
                G::Unset { feature, old } => ChangeKind::Unset {
                    feature,
                    old: owned(old),
                },
                G::Added {
                    feature,
                    index,
                    value,
                } => ChangeKind::Added {
                    feature,
                    index,
                    value: owned(value),
                },
                G::Removed {
                    feature,
                    index,
                    value,
                } => ChangeKind::Removed {
                    feature,
                    index,
                    value: owned(value),
                },
                G::Replaced {
                    feature,
                    index,
                    old,
                    new,
                } => ChangeKind::Replaced {
                    feature,
                    index,
                    old: owned(old),
                    new: owned(new),
                },
                G::Moved { feature, from, to } => ChangeKind::Moved { feature, from, to },
                G::Container { old, new } => ChangeKind::Container {
                    old: old.map(|(p, f)| (p.0, f)),
                    new: new.map(|(p, f)| (p.0, f)),
                },
                G::RootAdded { resource, index } => ChangeKind::RootAdded { resource, index },
                G::RootRemoved { resource } => ChangeKind::RootRemoved { resource },
                G::Deleted => ChangeKind::Deleted,
            },
        })
        .collect()
}

/// The model store over a SimOxide graph and its session (see the module documentation).
pub struct SimStore<'a> {
    pub(crate) g: &'a mut Graph,
    pub(crate) s: &'a mut Session,
}

impl<'a> SimStore<'a> {
    /// The store over `g`; the first store over a graph copies the session's metamodel objects
    /// into it (and the graph adopts the session's registry).
    pub fn new(g: &'a mut Graph, s: &'a mut Session) -> SimStore<'a> {
        if g.links().package_obj.is_empty() {
            g.append_graph(&s.rs.graph);
        } else if !std::sync::Arc::ptr_eq(g.meta_arc(), s.rs.graph.meta_arc()) {
            g.adopt_meta(s.rs.graph.meta_arc().clone());
        }
        g.set_id_state(s.ids.0, s.ids.1.clone());
        let mut st = SimStore { g, s };
        st.sync_journal();
        st
    }

    /// The graph.
    pub fn graph(&self) -> &Graph {
        self.g
    }

    /// The session (the journal recorded so far is moved into it).
    pub fn session(&mut self) -> &mut Session {
        self.flush();
        self.s
    }

    /// The store resource of a resource of the graph (the same).
    pub fn graph_resource(&self, r: ResourceId) -> Option<ResourceId> {
        ((r.0 as usize) < self.g.resource_count()).then_some(r)
    }

    /// The store resource with this URI.
    pub fn resource_by_uri(&self, uri: &str) -> Option<ResourceId> {
        self.g.resource_by_uri(uri)
    }

    /// Whether the object is of one of SimOxide's classes (what its syncers see).
    pub fn is_graph_object(&self, h: u32) -> bool {
        self.s.is_graph_class(self.g.class_of(ObjId(h)))
    }

    /// Moves the graph's journal into the session and follows the session's switch.
    fn flush(&mut self) {
        if self.g.is_recording() {
            let j = self.g.take_journal();
            if let Some(sj) = self.s.journal.as_mut() {
                sj.changes.extend(convert(j));
            }
        }
        self.sync_journal();
    }

    fn sync_journal(&mut self) {
        match (self.s.journal.is_some(), self.g.is_recording()) {
            (true, false) => self.g.start_journal(),
            (false, true) => {
                self.g.stop_journal();
            }
            _ => {}
        }
    }

    /// Before a change: the graph records exactly when the session does.
    fn w(&mut self) -> &mut Graph {
        self.sync_journal();
        self.g
    }

    // ---- loading models of other metamodels ---------------------------------------------------

    /// Loads a model of another metamodel (a PRM, an SPD model, ...) into the graph, resolving
    /// its references (`href`s to the graph's PCM documents become references to their
    /// objects).
    pub fn load_model(&mut self, path: &Path) -> Result<ResourceId, String> {
        let abs = std::path::absolute(path).map_err(|e| e.to_string())?;
        let uri = qvto_emf::uri::Uri::file(&abs.to_string_lossy()).to_string();
        let empty = self.g.empty_like();
        let g = std::mem::replace(&mut *self.g, empty);
        let mut rs = self.s.rs.rebind(g);
        let r = match rs.get_resource(&uri, false) {
            Some(r) => Ok(r),
            None => rs.load(&uri),
        };
        if let Ok(r) = r {
            rs.resolve_resource(r);
        }
        *self.g = rs.graph;
        r
    }

    /// A new, empty resource for objects of other metamodels (e.g. the PRM model the host
    /// fills with runtime measurements).
    pub fn create_model_resource(&mut self, uri: &str) -> ResourceId {
        self.g.create_resource(uri)
    }

    /// Nothing to link since one graph holds every model (kept for callers); returns 0.
    pub fn link_proxies(&mut self) -> usize {
        0
    }
}

impl Drop for SimStore<'_> {
    fn drop(&mut self) {
        self.flush();
        if self.g.is_recording() {
            self.g.stop_journal();
        }
        self.s.ids = self.g.id_state();
    }
}

impl ModelStore for SimStore<'_> {
    type Obj = u32;

    fn extend_meta(&mut self, meta: &MetaRegistry) -> bool {
        self.g.extend_meta(meta)
    }
    fn meta(&self) -> &MetaRegistry {
        self.g.meta()
    }
    fn class_of(&self, o: u32) -> ClassId {
        self.g.class_of(ObjId(o))
    }
    fn is_proxy(&self, o: u32) -> bool {
        self.g.is_proxy(ObjId(o))
    }
    fn proxy_uri(&self, o: u32) -> Option<&str> {
        self.g.proxy_uri(ObjId(o))
    }
    fn get(&self, o: u32, f: FeatureId) -> Datum<'_, u32> {
        map_datum(self.g.get(ObjId(o), f), h)
    }
    fn len(&self, o: u32, f: FeatureId) -> usize {
        self.g.len(ObjId(o), f)
    }
    fn get_at(&self, o: u32, f: FeatureId, i: usize) -> Datum<'_, u32> {
        map_datum(self.g.get_at(ObjId(o), f, i), h)
    }
    fn get_list<'b>(&'b self, o: u32, f: FeatureId, out: &mut Vec<Datum<'b, u32>>) {
        let mut v = Vec::new();
        self.g.get_list(ObjId(o), f, &mut v);
        out.extend(v.into_iter().map(|d| map_datum(d, h)));
    }
    fn is_set(&self, o: u32, f: FeatureId) -> bool {
        self.g.is_set(ObjId(o), f)
    }
    fn container(&self, o: u32) -> Option<(u32, FeatureId)> {
        self.g.container(ObjId(o)).map(|(p, f)| (p.0, f))
    }
    fn contents(&self, o: u32) -> Vec<u32> {
        self.g.contents(ObjId(o)).into_iter().map(h).collect()
    }
    fn all_contents(&self, o: u32) -> Vec<u32> {
        self.g.all_contents(ObjId(o)).into_iter().map(h).collect()
    }
    fn resource_of(&self, o: u32) -> Option<ResourceId> {
        self.g.resource_of(ObjId(o))
    }
    fn direct_resource(&self, o: u32) -> Option<ResourceId> {
        self.g.direct_resource(ObjId(o))
    }
    fn resource_count(&self) -> usize {
        self.g.resource_count()
    }
    fn resource_uri(&self, r: ResourceId) -> &str {
        self.g.resource_uri(r)
    }
    fn roots(&self, r: ResourceId) -> &[u32] {
        qvto_emf::graph::ids_as_u32(self.g.roots(r))
    }
    fn create(&mut self, c: ClassId) -> ModelResult<u32> {
        self.w().create(c).map(h)
    }
    fn set(&mut self, o: u32, f: FeatureId, v: Datum<'_, u32>) -> ModelResult<()> {
        self.w().set(ObjId(o), f, map_datum(v, ObjId))
    }
    fn unset(&mut self, o: u32, f: FeatureId) -> ModelResult<()> {
        self.w().unset(ObjId(o), f)
    }
    fn add(
        &mut self,
        o: u32,
        f: FeatureId,
        v: Datum<'_, u32>,
        at: Option<usize>,
    ) -> ModelResult<bool> {
        self.w().add(ObjId(o), f, map_datum(v, ObjId), at)
    }
    fn remove(&mut self, o: u32, f: FeatureId, v: Datum<'_, u32>) -> ModelResult<bool> {
        self.w().remove(ObjId(o), f, map_datum(v, ObjId))
    }
    fn remove_at(&mut self, o: u32, f: FeatureId, i: usize) -> ModelResult<()> {
        self.w().remove_at(ObjId(o), f, i)
    }
    fn set_at(&mut self, o: u32, f: FeatureId, i: usize, v: Datum<'_, u32>) -> ModelResult<()> {
        self.w().set_at(ObjId(o), f, i, map_datum(v, ObjId))
    }
    fn clear(&mut self, o: u32, f: FeatureId) -> ModelResult<()> {
        self.w().clear(ObjId(o), f)
    }
    fn move_to(&mut self, o: u32, f: FeatureId, to: usize, from: usize) -> ModelResult<()> {
        self.w().move_to(ObjId(o), f, to, from)
    }
    fn create_resource(&mut self, uri: &str) -> ResourceId {
        self.g.create_resource(uri)
    }
    fn add_root(&mut self, r: ResourceId, o: u32, at: Option<usize>) -> ModelResult<bool> {
        self.w().add_root(r, ObjId(o), at)
    }
    fn remove_root(&mut self, r: ResourceId, o: u32) -> bool {
        self.w().remove_root(r, ObjId(o))
    }
    fn delete(&mut self, o: u32) {
        self.w().delete(ObjId(o))
    }
    fn classifier_object(&self, c: Classifier) -> Option<u32> {
        self.g.classifier_object(c).map(h)
    }
    fn object_classifier(&self, o: u32) -> Option<Classifier> {
        self.g.object_classifier(ObjId(o))
    }
    fn feature_object(&self, f: FeatureId) -> Option<u32> {
        self.g.feature_object(f).map(h)
    }
    fn object_feature(&self, o: u32) -> Option<FeatureId> {
        self.g.object_feature(ObjId(o))
    }
    fn package_object(&self, p: PackageId) -> Option<u32> {
        self.g.package_object(p).map(h)
    }
    fn object_package(&self, o: u32) -> Option<PackageId> {
        self.g.object_package(ObjId(o))
    }
}

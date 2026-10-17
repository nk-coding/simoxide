//! The store: [`Session`] (what lives as long as the host's model: registry, metamodel objects,
//! models of other metamodels, resource table, journal) and [`SimStore`], the `ModelStore` view
//! of a session over SimOxide's `raw::Graph` that the engine runs on.
//!
//! Objects have two homes. Objects of SimOxide's classes live in the `raw::Graph` itself; their
//! handle is their `raw::ObjId`. Objects of other metamodels (the PRM model of SimuLizar's
//! runtime measurements, SPD models, metamodel objects for `eClass()`, ...) live in a
//! `qvto_emf` graph of the session; their handle is [`EXT_BASE`] + their id there. A reference
//! from such an object to a graph object goes through a *mirror* (a proxy object in the
//! `qvto_emf` graph standing for the graph object); the other way round, and containment or
//! opposites across the two, are not supported (no metamodel in the corpus needs them).

use crate::journal::{ChangeKind, Journal};
use crate::raw_part::{Raw, RawView};
use crate::registry::{self, SimTables};
use crate::uuid::{IdPolicy, UuidGen};
use qvto_emf::{ObjId, ResourceKind, ResourceSet};
use qvto_model::{
    ClassId, Classifier, Datum, FeatureId, MetaOperation, MetaRegistry, ModelError, ModelResult,
    ModelStore, OwnedDatum, PackageId, ResourceId,
};
use simoxide_model::raw;
use std::collections::HashMap;
use std::path::Path;

/// Handles at or above this are objects of the session's `qvto_emf` graph.
pub const EXT_BASE: u32 = 1 << 30;

/// A resource of the store's table.
#[derive(Clone, Debug)]
pub(crate) enum Res {
    /// A resource of the `raw::Graph` (index).
    Raw(u32),
    /// A resource of the session's `qvto_emf` graph.
    Ext(ResourceId),
    /// A resource created through the store (`create_resource`: the engine's `extent:/N`); its
    /// roots may be objects of both homes.
    Mem(String),
}

/// The session's bookkeeping besides the `qvto_emf` resource set.
#[derive(Clone, Default)]
pub(crate) struct State {
    pub res: Vec<Res>,
    /// Root handles per store resource (the storage of `Mem` resources, a copy for the others).
    pub roots: Vec<Vec<u32>>,
    /// Store resource of each `raw::Graph` resource.
    pub raw_res: Vec<Option<ResourceId>>,
    pub ext_res: HashMap<ResourceId, ResourceId>,
    /// Objects that are roots of `Mem` resources.
    pub mem_root: HashMap<u32, ResourceId>,
    pub mirror: HashMap<u32, ObjId>,
    pub mirrored: HashMap<ObjId, u32>,
    /// The `raw::Graph` resource new graph objects get as their home.
    pub created_res: Option<u32>,
    pub journal: Option<Journal>,
    /// Classes `0..raw_classes` live in the `raw::Graph`.
    pub raw_classes: u32,
    pub id_policy: IdPolicy,
    pub uuid: Option<UuidGen>,
    /// `identifier::Identifier` and its `id` (for [`IdPolicy::Uuid`]).
    pub identifier: Option<(ClassId, FeatureId)>,
}

/// Everything that lives as long as the host's model: the metamodel registry (SimOxide's tables,
/// Ecore, metamodels loaded later), metamodel objects, models of other metamodels, the resource
/// table, mirrors, the journal. One session belongs to one `raw::Graph`.
///
/// A clone is an independent session with the same state (the id generator included): the
/// session of a copy of the graph, e.g. of a forked simulation run.
#[derive(Clone)]
pub struct Session {
    pub(crate) rs: ResourceSet,
    pub(crate) st: State,
    pub(crate) ecore: PackageId,
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
        let mut rs =
            ResourceSet::with_graph(qvto_emf::Graph::with_registry(registry::sim_registry()));
        let meta = rs.graph.meta();
        let ops = meta.static_operations(rs.graph.ecore().package);
        let tops = meta.static_top_packages();
        rs.materialize(&tops, &ops)
            .expect("metamodel objects of SimOxide's packages");
        Session::from_parts(rs, SimTables::get().classes)
    }

    /// A session whose graph classes are those of `meta` (built in code, the graph's classes
    /// first; Ecore is added after them): for tests and hosts with other generated metamodels.
    /// `materialize` names the top-level packages that get metamodel objects, with locations.
    pub fn with_registry(meta: MetaRegistry, materialize: &[(PackageId, &str)]) -> Session {
        let raw_classes = meta.class_count() as u32;
        let mut rs = ResourceSet::with_graph(qvto_emf::Graph::with_registry(meta));
        if !materialize.is_empty() {
            rs.materialize(materialize, &[])
                .expect("metamodel objects of the given packages");
        }
        Session::from_parts(rs, raw_classes)
    }

    fn from_parts(rs: ResourceSet, raw_classes: u32) -> Session {
        let ecore = rs.graph.ecore().package;
        let meta = rs.graph.meta();
        let identifier = meta
            .package_by_ns_uri("http://sdq.ipd.uka.de/Identifier/2.1")
            .and_then(|p| meta.class_by_name(p, "Identifier"))
            .and_then(|c| meta.feature_by_name(c, "id").map(|f| (c, f)));
        let mut s = Session {
            rs,
            st: State {
                raw_classes,
                identifier,
                ..State::default()
            },
            ecore,
        };
        s.sync_ext();
        s
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

    /// Whether objects of class `c` live in SimOxide's graph.
    pub fn is_graph_class(&self, c: ClassId) -> bool {
        c.0 < self.st.raw_classes
    }

    /// The resource set of the objects of other metamodels and of the metamodel objects.
    pub fn resource_set(&self) -> &ResourceSet {
        &self.rs
    }

    /// The resource set, for URI maps, metamodel locations, loading (call
    /// [`SimStore::link_proxies`] after loading models that refer to the graph).
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
        let r = if let Some(loc) = self.rs.ns_location(uri).map(String::from) {
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
        };
        self.sync_ext();
        r
    }

    /// Registers a metamodel given as `.ecore` text at `uri` (e.g. embedded in the host).
    pub fn load_metamodel_text(&mut self, uri: &str, text: &str) -> Result<Vec<PackageId>, String> {
        let r = self.rs.load_text(uri, text)?;
        let p = self.rs.register(r);
        self.sync_ext();
        p
    }

    /// Starts recording changes (a new, empty journal).
    pub fn start_journal(&mut self) {
        self.st.journal = Some(Journal::default());
    }

    /// The recorded changes; recording continues with an empty journal.
    pub fn take_journal(&mut self) -> Journal {
        match self.st.journal.as_mut() {
            Some(j) => std::mem::take(j),
            None => Journal::default(),
        }
    }

    /// Stops recording.
    pub fn stop_journal(&mut self) -> Option<Journal> {
        self.st.journal.take()
    }

    pub fn set_id_policy(&mut self, p: IdPolicy) {
        self.st.id_policy = p;
        self.st.uuid = match p {
            IdPolicy::None => None,
            IdPolicy::Uuid { seed } => Some(UuidGen::new(seed)),
        };
    }

    /// The store over `g` (the graph this session belongs to).
    pub fn store<'a>(&'a mut self, g: &'a mut raw::Graph) -> SimStore<'a> {
        SimStore::new(g, self)
    }

    /// Registers resources of the `qvto_emf` graph that are not in the table yet and refreshes
    /// the copies of their root lists.
    pub(crate) fn sync_ext(&mut self) {
        let st = &mut self.st;
        for er in self.rs.graph.resource_ids() {
            if !st.ext_res.contains_key(&er) {
                let r = ResourceId(st.res.len() as u32);
                st.res.push(Res::Ext(er));
                st.roots.push(Vec::new());
                st.ext_res.insert(er, r);
            }
        }
        for (&er, &r) in &st.ext_res {
            let roots: Vec<u32> = self
                .rs
                .graph
                .roots(er)
                .iter()
                .map(|x| from_ext(&st.mirrored, *x))
                .collect();
            st.roots[r.index()] = roots;
        }
    }
}

/// The handle of an object of the `qvto_emf` graph.
#[inline]
fn from_ext(mirrored: &HashMap<ObjId, u32>, x: ObjId) -> u32 {
    match mirrored.get(&x) {
        Some(&h) => h,
        None => EXT_BASE + x.0,
    }
}

#[inline]
fn is_ext(h: u32) -> bool {
    h >= EXT_BASE
}

#[inline]
fn ext_id(h: u32) -> ObjId {
    ObjId(h - EXT_BASE)
}

fn map_datum<'a>(mirrored: &HashMap<ObjId, u32>, d: Datum<'a, ObjId>) -> Datum<'a, u32> {
    match d {
        Datum::Obj(x) => Datum::Obj(from_ext(mirrored, x)),
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

fn with_obj<'a, A, B>(d: Datum<'a, A>, x: B) -> Datum<'a, B> {
    match d {
        Datum::Obj(_) => Datum::Obj(x),
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

/// The model store over a SimOxide graph and its session (see the module documentation).
pub struct SimStore<'a> {
    pub(crate) g: &'a mut raw::Graph,
    pub(crate) s: &'a mut Session,
}

impl<'a> SimStore<'a> {
    pub fn new(g: &'a mut raw::Graph, s: &'a mut Session) -> SimStore<'a> {
        let mut st = SimStore { g, s };
        st.sync_raw();
        st.s.sync_ext();
        st
    }

    /// The SimOxide graph.
    pub fn graph(&self) -> &raw::Graph {
        self.g
    }

    pub fn session(&mut self) -> &mut Session {
        self.s
    }

    /// The store resource of a resource of the graph.
    pub fn graph_resource(&self, r: raw::ResId) -> Option<ResourceId> {
        self.s.st.raw_res.get(r.0 as usize).copied().flatten()
    }

    /// The store resource with this URI.
    pub fn resource_by_uri(&self, uri: &str) -> Option<ResourceId> {
        (0..self.s.st.res.len() as u32)
            .map(ResourceId)
            .find(|r| self.resource_uri(*r) == uri)
    }

    /// Whether the object lives in SimOxide's graph (its handle is its `raw::ObjId`).
    pub fn is_graph_object(&self, h: u32) -> bool {
        !is_ext(h)
    }

    fn sync_raw(&mut self) {
        let st = &mut self.s.st;
        for i in st.raw_res.len()..self.g.resources.len() {
            let r = ResourceId(st.res.len() as u32);
            st.res.push(Res::Raw(i as u32));
            st.roots.push(Vec::new());
            st.raw_res.push(Some(r));
        }
        for (i, r) in st.raw_res.iter().enumerate() {
            if let Some(r) = r {
                st.roots[r.index()] = self.g.resources[i].roots.iter().map(|x| x.0).collect();
            }
        }
    }

    pub(crate) fn raw(&mut self) -> Raw<'_> {
        Raw {
            g: self.g,
            meta: self.s.rs.graph.meta(),
            ext: &self.s.rs.graph,
            st: &mut self.s.st,
            ecore: self.s.ecore,
        }
    }

    pub(crate) fn view(&self) -> RawView<'_> {
        RawView {
            g: self.g,
            meta: self.s.rs.graph.meta(),
            ext: &self.s.rs.graph,
            st: &self.s.st,
            ecore: self.s.ecore,
        }
    }

    fn record(&mut self, o: u32, k: ChangeKind) {
        if let Some(j) = self.s.st.journal.as_mut() {
            j.push(o, k);
        }
    }

    fn recording(&self) -> bool {
        self.s.st.journal.is_some()
    }

    // ---- the other home ---------------------------------------------------------------------------

    /// The mirror of graph object `h` in the `qvto_emf` graph (created on demand).
    fn mirror(&mut self, h: u32) -> ObjId {
        if let Some(&m) = self.s.st.mirror.get(&h) {
            return m;
        }
        let class = self.view().class(h);
        let m = self
            .s
            .rs
            .graph
            .create_proxy(class, &format!("simoxide:/graph#{h}"));
        self.s.st.mirror.insert(h, m);
        self.s.st.mirrored.insert(m, h);
        m
    }

    /// A value for the `qvto_emf` graph: graph objects as their mirrors. `None` when the value is
    /// a graph object without a mirror and `create` is false (it cannot be in any list there).
    fn ext_value<'d>(&mut self, d: Datum<'d, u32>, create: bool) -> Option<Datum<'d, ObjId>> {
        match d {
            Datum::Obj(h) if is_ext(h) => Some(with_obj(d, ext_id(h))),
            Datum::Obj(h) => {
                let m = if create {
                    self.mirror(h)
                } else {
                    *self.s.st.mirror.get(&h)?
                };
                Some(with_obj(d, m))
            }
            d => Some(with_obj(d, ObjId(0))),
        }
    }

    /// Rejects a graph object as the value of a containment or bidirectional feature of an
    /// object of another metamodel (the mirror cannot carry that state).
    fn check_cross(&self, f: FeatureId, v: &Datum<'_, u32>) -> ModelResult<()> {
        let fi = self.meta().feature(f);
        if let Datum::Obj(h) = v
            && !is_ext(*h)
            && (fi.containment || fi.opposite.is_some())
        {
            return Err(ModelError::Unsupported(format!(
                "containment or opposite '{}' between an object of another metamodel and SimOxide's graph",
                fi.name
            )));
        }
        Ok(())
    }

    fn ext_get(&self, x: ObjId, f: FeatureId) -> Datum<'_, u32> {
        map_datum(&self.s.st.mirrored, self.s.rs.graph.get(x, f))
    }

    fn ext_get_at(&self, x: ObjId, f: FeatureId, i: usize) -> Datum<'_, u32> {
        map_datum(&self.s.st.mirrored, self.s.rs.graph.get_at(x, f, i))
    }

    /// After a change in the `qvto_emf` graph: root copies, and objects that became contained
    /// leave the store's own resources (`eBasicSetContainer`).
    fn after_ext(&mut self, contained: Option<u32>) {
        self.s.sync_ext();
        if let Some(h) = contained
            && let Some(&r) = self.s.st.mem_root.get(&h)
            && let Some((_, f)) = self.container(h)
            && !self.meta().feature(f).resolve_proxies
        {
            self.mem_root_remove(r, h);
        }
    }

    fn mem_root_remove(&mut self, r: ResourceId, h: u32) -> bool {
        let roots = &mut self.s.st.roots[r.index()];
        let Some(i) = roots.iter().position(|x| *x == h) else {
            return false;
        };
        roots.remove(i);
        self.s.st.mem_root.remove(&h);
        self.record(h, ChangeKind::RootRemoved { resource: r });
        true
    }

    /// The object a containment change may have contained (for [`Self::after_ext`]).
    fn contained_by(&self, o: u32, f: FeatureId, v: &Datum<'_, u32>) -> Option<u32> {
        let fi = self.meta().feature(f);
        if fi.containment {
            v.as_obj()
        } else if self.meta().is_container(f) {
            Some(o)
        } else {
            None
        }
    }

    // ---- loading models of other metamodels ---------------------------------------------------

    /// Loads a model of another metamodel (a PRM, an SPD model, ...) into the session, linking
    /// its references into the graph (`href`s to resources of the graph become references to
    /// the graph's objects) and resolving the others. The documents of the graph's metamodels
    /// it refers to must be in the graph already (SimOxide's loader).
    pub fn load_model(&mut self, path: &Path) -> Result<ResourceId, String> {
        let abs = std::path::absolute(path).map_err(|e| e.to_string())?;
        let uri = qvto_emf::uri::Uri::file(&abs.to_string_lossy()).to_string();
        let er = match self.s.rs.get_resource(&uri, false) {
            Some(r) => r,
            None => self.s.rs.load(&uri)?,
        };
        self.link_proxies();
        self.s.rs.resolve_all();
        self.link_proxies();
        self.s.sync_ext();
        Ok(self.s.st.ext_res[&er])
    }

    /// A new, empty resource for objects of other metamodels (e.g. the PRM model the host
    /// fills with runtime measurements).
    pub fn create_model_resource(&mut self, uri: &str) -> ResourceId {
        let er = self.s.rs.graph.create_resource(uri);
        self.s.sync_ext();
        self.s.st.ext_res[&er]
    }

    /// Replaces proxies of the `qvto_emf` graph that designate objects of the graph by mirrors.
    /// Returns how many were linked.
    pub fn link_proxies(&mut self) -> usize {
        let mut by_uri: HashMap<String, usize> = HashMap::new();
        for (i, r) in self.g.resources.iter().enumerate() {
            by_uri.insert(r.uri.clone(), i);
            by_uri.insert(self.s.rs.normalize(&r.uri), i);
        }
        let mut work = Vec::new();
        {
            let g = &self.s.rs.graph;
            let meta = g.meta();
            for er in g.resource_ids() {
                if g.resource(er).kind == ResourceKind::Ecore {
                    continue;
                }
                for o in g.resource_contents(er) {
                    for &f in meta.class(g.class_of(o)).cross_references.iter() {
                        let n = if meta.feature(f).is_many() {
                            g.len(o, f)
                        } else {
                            1
                        };
                        for i in 0..n {
                            let t = if meta.feature(f).is_many() {
                                g.get_at(o, f, i)
                            } else {
                                g.get(o, f)
                            };
                            if let Datum::Obj(t) = t
                                && g.is_proxy(t)
                                && !self.s.st.mirrored.contains_key(&t)
                            {
                                work.push((o, f, t, g.proxy_uri(t).unwrap_or("").to_string()));
                            }
                        }
                    }
                }
            }
        }
        let mut linked = 0;
        for (o, f, t, uri) in work {
            let (doc, frag) = uri.split_once('#').unwrap_or((&uri, ""));
            let rr = by_uri
                .get(doc)
                .or_else(|| by_uri.get(&self.s.rs.normalize(doc)))
                .copied();
            let Some(rr) = rr else { continue };
            let Some(h) = self.resolve_in_graph(rr, frag) else {
                continue;
            };
            let m = self.mirror(h);
            self.s.rs.graph.replace_proxy(o, f, t, m);
            linked += 1;
        }
        linked
    }

    /// `Resource.getEObject(fragment)` in a resource of the graph: `//@f.i/@g` paths or IDs.
    pub fn resolve_in_graph(&self, rr: usize, frag: &str) -> Option<u32> {
        let v = self.view();
        let meta = self.meta();
        if let Some(path) = frag.strip_prefix('/') {
            let mut segs = path.split('/');
            let first = segs.next()?;
            let idx: usize = if first.is_empty() {
                0
            } else {
                first.parse().ok()?
            };
            let mut cur = self.g.resources.get(rr)?.roots.get(idx)?.0;
            for seg in segs {
                let seg = seg.strip_prefix('@')?;
                let (name, i) = match seg.rsplit_once('.') {
                    Some((n, i)) if i.parse::<usize>().is_ok() => (n, i.parse::<usize>().ok()),
                    _ => (seg, None),
                };
                let f = meta.feature_by_name(v.class(cur), name)?;
                cur = match i {
                    Some(i) => v.get_at(cur, f, i).as_obj()?,
                    None => v.get(cur, f).as_obj()?,
                };
            }
            return Some(cur);
        }
        let r = self.s.st.raw_res.get(rr).copied().flatten()?;
        self.resource_contents(r).into_iter().find(|&o| {
            let obj = &self.g.objs[o as usize];
            obj.xmi_id.as_deref() == Some(frag)
                || meta
                    .class(v.class(o))
                    .id_attribute
                    .is_some_and(|f| v.get(o, f).as_str() == Some(frag))
        })
    }

    // ---- id policy ------------------------------------------------------------------------------

    fn assign_id(&mut self, h: u32, c: ClassId) {
        let Some((ident, id)) = self.s.st.identifier else {
            return;
        };
        if !self.meta().is_subclass(c, ident) {
            return;
        }
        let Some(u) = self.s.st.uuid.as_mut().map(|g| g.next()) else {
            return;
        };
        let _ = self.set(h, id, Datum::Str(&u));
    }

    /// The root container of `o`.
    fn root_container(&self, o: u32) -> u32 {
        let mut cur = o;
        let mut steps = 0;
        while let Some((p, _)) = self.container(cur) {
            if p == o || steps > 1_000_000 {
                break;
            }
            cur = p;
            steps += 1;
        }
        cur
    }
}

impl ModelStore for SimStore<'_> {
    type Obj = u32;

    /// The program's classes (intermediate classes) are added after every metamodel class, so
    /// their instances live in the `qvto_emf` graph, with EMF's semantics.
    fn extend_meta(&mut self, meta: &MetaRegistry) -> bool {
        let ok = self.s.rs.graph.extend_meta(meta);
        if ok {
            self.s.sync_ext();
        }
        ok
    }

    fn meta(&self) -> &MetaRegistry {
        self.s.rs.graph.meta()
    }

    fn class_of(&self, o: u32) -> ClassId {
        if is_ext(o) {
            self.s.rs.graph.class_of(ext_id(o))
        } else {
            self.view().class(o)
        }
    }

    fn is_proxy(&self, o: u32) -> bool {
        if is_ext(o) {
            self.s.rs.graph.is_proxy(ext_id(o))
        } else {
            self.g.objs[o as usize].proxy.is_some()
        }
    }

    fn proxy_uri(&self, o: u32) -> Option<&str> {
        if is_ext(o) {
            self.s.rs.graph.proxy_uri(ext_id(o))
        } else {
            self.g.objs[o as usize].proxy.as_deref()
        }
    }

    fn get(&self, o: u32, f: FeatureId) -> Datum<'_, u32> {
        if is_ext(o) {
            self.ext_get(ext_id(o), f)
        } else {
            self.view().get(o, f)
        }
    }

    fn len(&self, o: u32, f: FeatureId) -> usize {
        if is_ext(o) {
            self.s.rs.graph.len(ext_id(o), f)
        } else {
            self.view().len(o, f)
        }
    }

    fn get_at(&self, o: u32, f: FeatureId, i: usize) -> Datum<'_, u32> {
        if is_ext(o) {
            self.ext_get_at(ext_id(o), f, i)
        } else {
            self.view().get_at(o, f, i)
        }
    }

    fn is_set(&self, o: u32, f: FeatureId) -> bool {
        if is_ext(o) {
            self.s.rs.graph.is_set(ext_id(o), f)
        } else {
            self.view().is_set(o, f)
        }
    }

    fn container(&self, o: u32) -> Option<(u32, FeatureId)> {
        if is_ext(o) {
            self.s
                .rs
                .graph
                .container(ext_id(o))
                .map(|(p, f)| (from_ext(&self.s.st.mirrored, p), f))
        } else {
            self.view().container(o)
        }
    }

    fn resource_of(&self, o: u32) -> Option<ResourceId> {
        let mut cur = o;
        let mut steps = 0;
        loop {
            if let Some(r) = self.direct_resource(cur) {
                return Some(r);
            }
            match self.container(cur) {
                Some((p, _)) => cur = p,
                None => return None,
            }
            steps += 1;
            if steps > 1_000_000 {
                return None;
            }
        }
    }

    fn direct_resource(&self, o: u32) -> Option<ResourceId> {
        if is_ext(o) {
            if let Some(&r) = self.s.st.mem_root.get(&o) {
                return Some(r);
            }
            self.s
                .rs
                .graph
                .direct_resource(ext_id(o))
                .and_then(|er| self.s.st.ext_res.get(&er).copied())
        } else {
            self.view().direct_resource(o)
        }
    }

    fn resource_count(&self) -> usize {
        self.s.st.res.len()
    }

    fn resource_uri(&self, r: ResourceId) -> &str {
        match &self.s.st.res[r.index()] {
            Res::Raw(i) => &self.g.resources[*i as usize].uri,
            Res::Ext(er) => self.s.rs.graph.resource_uri(*er),
            Res::Mem(uri) => uri,
        }
    }

    fn roots(&self, r: ResourceId) -> &[u32] {
        &self.s.st.roots[r.index()]
    }

    fn create(&mut self, c: ClassId) -> ModelResult<u32> {
        let ci = self.meta().class(c);
        if ci.is_abstract || ci.is_interface {
            return Err(ModelError::Abstract(c));
        }
        let h = if c.0 < self.s.st.raw_classes {
            let home = match self.s.st.created_res {
                Some(r) => r,
                None => {
                    let r = self.g.resources.len() as u32;
                    self.g.resources.push(raw::Resource {
                        uri: "qvto:/created".into(),
                        name: "created".into(),
                        ..raw::Resource::default()
                    });
                    self.s.st.created_res = Some(r);
                    self.sync_raw();
                    r
                }
            };
            self.raw().create(c, home)
        } else {
            let x = self.s.rs.graph.create(c)?;
            let h = EXT_BASE + x.0;
            self.record(h, ChangeKind::Created(c));
            h
        };
        if self.s.st.uuid.is_some() {
            self.assign_id(h, c);
        }
        Ok(h)
    }

    fn set(&mut self, o: u32, f: FeatureId, v: Datum<'_, u32>) -> ModelResult<()> {
        if !is_ext(o) {
            return self.raw().set(o, f, v);
        }
        self.check_cross(f, &v)?;
        let old = self.recording().then(|| self.get(o, f).to_owned());
        let d = self.ext_value(v, true).expect("created");
        self.s.rs.graph.set(ext_id(o), f, d)?;
        self.after_ext(self.contained_by(o, f, &v));
        if let Some(old) = old {
            let new = self.get(o, f).to_owned();
            self.record(
                o,
                ChangeKind::Set {
                    feature: f,
                    old,
                    new,
                },
            );
        }
        Ok(())
    }

    fn unset(&mut self, o: u32, f: FeatureId) -> ModelResult<()> {
        if !is_ext(o) {
            return self.raw().unset(o, f);
        }
        let old = self.recording().then(|| self.get(o, f).to_owned());
        self.s.rs.graph.unset(ext_id(o), f)?;
        self.after_ext(None);
        if let Some(old) = old {
            self.record(o, ChangeKind::Unset { feature: f, old });
        }
        Ok(())
    }

    fn add(
        &mut self,
        o: u32,
        f: FeatureId,
        v: Datum<'_, u32>,
        at: Option<usize>,
    ) -> ModelResult<bool> {
        if !is_ext(o) {
            return self.raw().add(o, f, v, at);
        }
        self.check_cross(f, &v)?;
        let len = self.len(o, f);
        let d = self.ext_value(v, true).expect("created");
        let added = self.s.rs.graph.add(ext_id(o), f, d, at)?;
        self.after_ext(self.contained_by(o, f, &v));
        if added && self.recording() {
            let value = v.to_owned();
            self.record(
                o,
                ChangeKind::Added {
                    feature: f,
                    index: at.unwrap_or(len),
                    value,
                },
            );
        }
        Ok(added)
    }

    fn remove(&mut self, o: u32, f: FeatureId, v: Datum<'_, u32>) -> ModelResult<bool> {
        if !is_ext(o) {
            return self.raw().remove(o, f, v);
        }
        let Some(d) = self.ext_value(v, false) else {
            return Ok(false);
        };
        let index = (0..self.len(o, f)).find(|&i| self.get_at(o, f, i).java_equals(&v));
        let removed = self.s.rs.graph.remove(ext_id(o), f, d)?;
        self.after_ext(None);
        if removed
            && let Some(index) = index
            && self.recording()
        {
            let value = v.to_owned();
            self.record(
                o,
                ChangeKind::Removed {
                    feature: f,
                    index,
                    value,
                },
            );
        }
        Ok(removed)
    }

    fn remove_at(&mut self, o: u32, f: FeatureId, i: usize) -> ModelResult<()> {
        if !is_ext(o) {
            return self.raw().remove_at(o, f, i);
        }
        let value = self.get_at(o, f, i).to_owned();
        self.s.rs.graph.remove_at(ext_id(o), f, i)?;
        self.after_ext(None);
        self.record(
            o,
            ChangeKind::Removed {
                feature: f,
                index: i,
                value,
            },
        );
        Ok(())
    }

    fn set_at(&mut self, o: u32, f: FeatureId, i: usize, v: Datum<'_, u32>) -> ModelResult<()> {
        if !is_ext(o) {
            return self.raw().set_at(o, f, i, v);
        }
        self.check_cross(f, &v)?;
        let old = self.get_at(o, f, i).to_owned();
        let d = self.ext_value(v, true).expect("created");
        self.s.rs.graph.set_at(ext_id(o), f, i, d)?;
        self.after_ext(self.contained_by(o, f, &v));
        if self.recording() {
            let new = v.to_owned();
            self.record(
                o,
                ChangeKind::Replaced {
                    feature: f,
                    index: i,
                    old,
                    new,
                },
            );
        }
        Ok(())
    }

    fn clear(&mut self, o: u32, f: FeatureId) -> ModelResult<()> {
        if !is_ext(o) {
            return self.raw().clear(o, f);
        }
        let old: Vec<OwnedDatum<u32>> = (0..self.len(o, f))
            .map(|i| self.get_at(o, f, i).to_owned())
            .collect();
        self.s.rs.graph.clear(ext_id(o), f)?;
        self.after_ext(None);
        for (index, value) in old.into_iter().enumerate().rev() {
            self.record(
                o,
                ChangeKind::Removed {
                    feature: f,
                    index,
                    value,
                },
            );
        }
        Ok(())
    }

    fn move_to(&mut self, o: u32, f: FeatureId, to: usize, from: usize) -> ModelResult<()> {
        if !is_ext(o) {
            return self.raw().move_to(o, f, to, from);
        }
        self.s.rs.graph.move_to(ext_id(o), f, to, from)?;
        self.record(
            o,
            ChangeKind::Moved {
                feature: f,
                from,
                to,
            },
        );
        Ok(())
    }

    fn create_resource(&mut self, uri: &str) -> ResourceId {
        let r = ResourceId(self.s.st.res.len() as u32);
        self.s.st.res.push(Res::Mem(uri.to_string()));
        self.s.st.roots.push(Vec::new());
        r
    }

    fn add_root(&mut self, r: ResourceId, o: u32, at: Option<usize>) -> ModelResult<bool> {
        let res = self.s.st.res[r.index()].clone();
        match (res, is_ext(o)) {
            (Res::Raw(_) | Res::Mem(_), false) => self.raw().root_add(r, o, at),
            (Res::Ext(er), true) => {
                if let Some(&m) = self.s.st.mem_root.get(&o) {
                    self.mem_root_remove(m, o);
                }
                let added = self.s.rs.graph.add_root(er, ext_id(o), at)?;
                self.s.sync_ext();
                if added {
                    let index = self.s.st.roots[r.index()]
                        .iter()
                        .position(|x| *x == o)
                        .unwrap_or(0);
                    self.record(o, ChangeKind::RootAdded { resource: r, index });
                }
                Ok(added)
            }
            (Res::Mem(_), true) => {
                let len = self.s.st.roots[r.index()].len();
                if let Some(i) = at
                    && i > len
                {
                    return Err(ModelError::IndexOutOfBounds { index: i, size: len });
                }
                if self.s.st.roots[r.index()].contains(&o) {
                    return match at {
                        None => Ok(false),
                        Some(_) => Err(ModelError::Unsupported(
                            "The 'no duplicates' constraint is violated".into(),
                        )),
                    };
                }
                // eSetResource: out of the old root list, out of a non-resolving container
                if let Some(old) = self.direct_resource(o) {
                    self.remove_root(old, o);
                }
                if let Some((p, f)) = self.container(o)
                    && !self.meta().feature(f).resolve_proxies
                {
                    if self.meta().feature(f).is_many() {
                        let _ = self.remove(p, f, Datum::Obj(o));
                    } else {
                        let _ = self.unset(p, f);
                    }
                }
                let i = at.unwrap_or(len).min(self.s.st.roots[r.index()].len());
                self.s.st.roots[r.index()].insert(i, o);
                self.s.st.mem_root.insert(o, r);
                self.record(o, ChangeKind::RootAdded { resource: r, index: i });
                Ok(true)
            }
            _ => Err(ModelError::Unsupported(
                "an object cannot be a root of a resource of the other home (SimOxide's graph / other metamodels)".into(),
            )),
        }
    }

    fn remove_root(&mut self, r: ResourceId, o: u32) -> bool {
        match self.s.st.res[r.index()].clone() {
            Res::Raw(_) if !is_ext(o) => self.raw().root_remove(r, o),
            Res::Mem(_) if !is_ext(o) => self.raw().root_remove(r, o),
            Res::Mem(_) => self.mem_root_remove(r, o),
            Res::Ext(er) if is_ext(o) => {
                let done = self.s.rs.graph.remove_root(er, ext_id(o));
                self.s.sync_ext();
                if done {
                    self.record(o, ChangeKind::RootRemoved { resource: r });
                }
                done
            }
            _ => false,
        }
    }

    /// `EcoreUtil.delete(o)` over both homes: usages in every resource of the store (or in the
    /// object's tree when it is in none) are removed, then the object leaves its container and
    /// its resource. Resources of metamodels (`.ecore`) are not searched.
    fn delete(&mut self, o: u32) {
        let root = self.root_container(o);
        let scope: Vec<u32> = if self.resource_of(root).is_some() {
            (0..self.s.st.res.len() as u32)
                .map(ResourceId)
                .filter(|r| match &self.s.st.res[r.index()] {
                    Res::Ext(er) => self.s.rs.graph.resource(*er).kind != ResourceKind::Ecore,
                    _ => true,
                })
                .flat_map(|r| self.resource_contents(r))
                .collect()
        } else {
            let mut v = vec![root];
            v.extend(self.all_contents(root));
            v
        };
        let mut usages: Vec<(u32, FeatureId)> = Vec::new();
        for x in scope {
            let class = self.class_of(x);
            for &f in self.meta().class(class).cross_references.iter() {
                let fi = self.meta().feature(f);
                if fi.derived || !fi.changeable {
                    continue;
                }
                if fi.is_many() {
                    for i in 0..self.len(x, f) {
                        if self.get_at(x, f, i) == Datum::Obj(o) {
                            usages.push((x, f));
                        }
                    }
                } else if self.get(x, f) == Datum::Obj(o) {
                    usages.push((x, f));
                }
            }
        }
        for (x, f) in usages {
            if self.meta().feature(f).is_many() {
                let _ = self.remove(x, f, Datum::Obj(o));
            } else {
                let _ = self.unset(x, f);
            }
        }
        if let Some((p, f)) = self.container(o) {
            if self.meta().feature(f).is_many() {
                let _ = self.remove(p, f, Datum::Obj(o));
            } else {
                let _ = self.unset(p, f);
            }
        }
        if let Some(r) = self.direct_resource(o) {
            self.remove_root(r, o);
        }
        self.record(o, ChangeKind::Deleted);
    }

    // ---- metamodel objects ----

    fn classifier_object(&self, c: Classifier) -> Option<u32> {
        self.s.rs.graph.classifier_object(c).map(|x| EXT_BASE + x.0)
    }

    fn object_classifier(&self, o: u32) -> Option<Classifier> {
        is_ext(o)
            .then(|| self.s.rs.graph.object_classifier(ext_id(o)))
            .flatten()
    }

    fn feature_object(&self, f: FeatureId) -> Option<u32> {
        self.s.rs.graph.feature_object(f).map(|x| EXT_BASE + x.0)
    }

    fn object_feature(&self, o: u32) -> Option<FeatureId> {
        is_ext(o)
            .then(|| self.s.rs.graph.object_feature(ext_id(o)))
            .flatten()
    }

    fn package_object(&self, p: PackageId) -> Option<u32> {
        self.s.rs.graph.package_object(p).map(|x| EXT_BASE + x.0)
    }

    fn object_package(&self, o: u32) -> Option<PackageId> {
        is_ext(o)
            .then(|| self.s.rs.graph.object_package(ext_id(o)))
            .flatten()
    }
}

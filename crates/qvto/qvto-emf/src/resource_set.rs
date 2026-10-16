//! A resource set: a [`Graph`] plus EMF's `ResourceSetImpl` and `URIConverter` services:
//! loading resources by URI (with URI prefix maps such as `platform:/plugin/...`), demand loading
//! for proxy resolution, a package registry (nsURI -> metamodel location) and registering the
//! packages of loaded `.ecore` resources.

use crate::ecore::{ECORE_ECORE, ECORE_NS_URI};
use crate::fragment::resolve_fragment;
use crate::graph::{Graph, ObjId, ResourceKind, Slot};
use crate::register::{link_ecore, register_resources};
use crate::uri::Uri;
use crate::xmi_load::{self, kind_for_uri};
use qvto_model::{ModelStore, PackageId, ResourceId};
use std::collections::HashMap;
use std::path::Path;

pub const ECORE_PLATFORM_URI: &str = "platform:/plugin/org.eclipse.emf.ecore/model/Ecore.ecore";

#[derive(Clone, Debug)]
pub struct ResourceSet {
    pub graph: Graph,
    uri_map: Vec<(String, String)>,
    ns_locations: HashMap<String, String>,
    /// Extra package-registry keys: URI -> nsURI of a registered package.
    aliases: HashMap<String, String>,
    ecore_resource: Option<ResourceId>,
    /// Resources whose load failed (missing file, XML error), by URI.
    pub failed: HashMap<String, String>,
}

impl Default for ResourceSet {
    fn default() -> Self {
        Self::new()
    }
}

impl ResourceSet {
    pub fn new() -> ResourceSet {
        ResourceSet {
            graph: Graph::new(),
            uri_map: Vec::new(),
            ns_locations: HashMap::new(),
            aliases: HashMap::new(),
            ecore_resource: None,
            failed: HashMap::new(),
        }
    }

    /// `URIConverter.URI_MAP.put(from, to)`: a prefix mapping (both ending with `/`) or an exact
    /// mapping.
    pub fn map_uri(&mut self, from: &str, to: &str) {
        self.uri_map.push((from.to_string(), to.to_string()));
    }

    /// Registers the location of the metamodel defining a namespace URI (like a package
    /// registry descriptor); it is loaded and registered when a document uses the namespace.
    pub fn add_ns_location(&mut self, ns_uri: &str, location: &str) {
        self.ns_locations.insert(ns_uri.to_string(), location.to_string());
    }

    /// Registers a package-registry alias: documents at `key` are the package with `ns_uri`
    /// (e.g. `platform:/plugin/org.eclipse.emf.ecore/model/Ecore.ecore` for Ecore).
    pub fn add_alias(&mut self, key: &str, ns_uri: &str) {
        self.aliases.insert(key.to_string(), ns_uri.to_string());
    }

    /// `URIConverter.normalize`: the longest matching prefix mapping.
    pub fn normalize(&self, uri: &str) -> String {
        let mut best: Option<&(String, String)> = None;
        for m in &self.uri_map {
            let hit = if m.0.ends_with('/') { uri.starts_with(&m.0) } else { uri == m.0 };
            if hit && best.is_none_or(|b| m.0.len() > b.0.len()) {
                best = Some(m);
            }
        }
        match best {
            Some((from, to)) => format!("{to}{}", &uri[from.len()..]),
            None => uri.to_string(),
        }
    }

    /// The resource holding Ecore's own objects (`EString`, `EClass`, ...), created on demand
    /// with the URI `http://www.eclipse.org/emf/2002/Ecore` and linked to the built-in entries.
    pub fn ecore_resource(&mut self) -> ResourceId {
        if let Some(r) = self.ecore_resource {
            return r;
        }
        let r = self.graph.create_resource_kind(ECORE_NS_URI, ResourceKind::Ecore);
        xmi_load::load(&mut self.graph, r, ECORE_ECORE).expect("bundled Ecore.ecore");
        link_ecore(&mut self.graph, r);
        self.ecore_resource = Some(r);
        r
    }

    fn read(&self, uri: &str) -> Result<String, String> {
        let n = self.normalize(uri);
        let u = Uri::parse(&n);
        let path = u.to_file_path().ok_or_else(|| format!("cannot open {uri} (normalized {n})"))?;
        std::fs::read_to_string(&path).map_err(|e| format!("{path}: {e}"))
    }

    /// `ResourceSet.getResource(uri, loadOnDemand)`.
    pub fn get_resource(&mut self, uri: &str, load: bool) -> Option<ResourceId> {
        let uri = Uri::parse(uri).trim_fragment().to_string();
        if let Some(r) = self.graph.resource_by_uri(&uri) {
            return Some(r);
        }
        let norm = self.normalize(&uri);
        if let Some(r) = self.graph.resource_ids().find(|r| self.normalize(&self.graph.resource(*r).uri) == norm) {
            return Some(r);
        }
        let key = self.aliases.get(&uri).cloned().unwrap_or_else(|| uri.clone());
        if key == ECORE_NS_URI {
            return Some(self.ecore_resource());
        }
        if let Some(p) = self.graph.meta().package_by_ns_uri(&key)
            && let Some(&o) = self.graph.links().package_obj.get(&p)
        {
            return self.graph.resource_of(o);
        }
        if !load || self.failed.contains_key(&uri) {
            return None;
        }
        match self.load(&uri) {
            Ok(r) => Some(r),
            Err(_) => None,
        }
    }

    /// Loads the document at `uri` into a new resource (the namespaces it uses are registered
    /// first, from the registry, the namespace locations or `xsi:schemaLocation`).
    pub fn load(&mut self, uri: &str) -> Result<ResourceId, String> {
        let text = match self.read(uri) {
            Ok(t) => t,
            Err(e) => {
                self.failed.insert(uri.to_string(), e.clone());
                return Err(e);
            }
        };
        self.load_text(uri, &text)
    }

    pub fn load_file(&mut self, path: &Path) -> Result<ResourceId, String> {
        let abs = std::path::absolute(path).map_err(|e| e.to_string())?;
        let uri = Uri::file(&abs.to_string_lossy()).to_string();
        self.load(&uri)
    }

    pub fn load_text(&mut self, uri: &str, text: &str) -> Result<ResourceId, String> {
        let pre = xmi_load::prescan(text);
        for ns in &pre.ns_uris {
            self.ensure_ns(ns, uri, &pre.schema_locations);
        }
        let parsed = Uri::parse(uri);
        let r = self.graph.create_resource_kind(uri, kind_for_uri(&parsed));
        if let Err(e) = xmi_load::load(&mut self.graph, r, text) {
            self.failed.insert(uri.to_string(), e.clone());
            return Err(e);
        }
        Ok(r)
    }

    fn ensure_ns(&mut self, ns: &str, doc_uri: &str, locations: &[(String, String)]) {
        if self.graph.meta().package_by_ns_uri(ns).is_some() || ns.is_empty() {
            return;
        }
        if ns == "http://www.omg.org/XMI" || ns.starts_with("http://schema.omg.org/") || ns == crate::xmi_load::XSI_NS {
            return;
        }
        if let Some(l) = self.ns_locations.get(ns).cloned() {
            let _ = self.load_metamodel(&l);
            return;
        }
        // `xsi:schemaLocation`: the package at the location serves the namespace, whatever its
        // own nsURI (XMLHandler.getPackageForURI)
        let Some(loc) = locations
            .iter()
            .find(|(n, _)| n == ns)
            .map(|(_, l)| Uri::parse(l).resolve(&Uri::parse(doc_uri)))
        else {
            return;
        };
        if self.load_metamodel(&loc.trim_fragment().to_string()).is_err() && self.graph.meta().package_by_ns_uri(ns).is_none() {
            return;
        }
        let Some(r) = self.get_resource(&loc.trim_fragment().to_string(), false) else { return };
        let target = match loc.fragment() {
            Some(f) => resolve_fragment(&self.graph, r, f),
            None => self.graph.roots(r).iter().copied().find(|o| self.graph.links().obj_package.contains_key(o)),
        };
        if let Some(o) = target
            && let Some(&p) = self.graph.links().obj_package.get(&o)
            && self.graph.meta().package_by_ns_uri(ns).is_none()
        {
            self.graph.meta_mut().register_alias(ns, p);
        }
    }

    /// Loads (if needed) and registers the packages of a metamodel resource and of the
    /// metamodel resources it refers to.
    pub fn load_metamodel(&mut self, uri: &str) -> Result<Vec<PackageId>, String> {
        let r = self
            .get_resource(uri, true)
            .ok_or_else(|| self.failed.get(uri).cloned().unwrap_or_else(|| format!("cannot load {uri}")))?;
        self.register(r)
    }

    fn is_registered(&self, r: ResourceId) -> bool {
        self.graph
            .roots(r)
            .iter()
            .any(|o| self.graph.links().obj_package.contains_key(o))
    }

    fn is_ecore_model(&self, r: ResourceId) -> bool {
        let e = self.graph.ecore();
        self.graph
            .roots(r)
            .iter()
            .any(|o| self.graph.meta().is_subclass(self.graph.class_of(*o), e.epackage))
    }

    /// Registers the packages of resource `r` (with the metamodels it depends on).
    pub fn register(&mut self, r: ResourceId) -> Result<Vec<PackageId>, String> {
        let mut todo = vec![r];
        let mut batch = Vec::new();
        while let Some(x) = todo.pop() {
            if batch.contains(&x) || self.is_registered(x) || !self.is_ecore_model(x) {
                continue;
            }
            self.resolve_resource(x);
            batch.push(x);
            for o in self.graph.resource_contents(x) {
                let class = self.graph.class_of(o);
                let feats = self.graph.meta().class(class).cross_references.to_vec();
                for f in feats {
                    if self.graph.meta().feature(f).derived {
                        continue;
                    }
                    for t in self.targets(o, f) {
                        if let Some(tr) = self.graph.resource_of(t)
                            && tr != x
                        {
                            todo.push(tr);
                        }
                    }
                }
            }
        }
        if batch.is_empty() {
            return Ok(Vec::new());
        }
        register_resources(&mut self.graph, &batch)
    }

    fn targets(&self, o: ObjId, f: qvto_model::FeatureId) -> Vec<ObjId> {
        match self.graph.slot(o, f) {
            Some(Slot::Ref(Some(t))) => vec![*t],
            Some(Slot::Refs(v, _)) => v.clone(),
            _ => Vec::new(),
        }
    }

    /// `EcoreUtil.resolve(proxy, resourceSet)`: the object the proxy URI designates, loading its
    /// resource on demand; `None` if it cannot be resolved.
    pub fn resolve(&mut self, proxy: ObjId) -> Option<ObjId> {
        let uri = self.graph.proxy_uri(proxy)?.to_string();
        let u = Uri::parse(&uri);
        let frag = u.fragment().unwrap_or("").to_string();
        let r = self.get_resource(&u.trim_fragment().to_string(), true)?;
        let t = resolve_fragment(&self.graph, r, &frag)?;
        if self.graph.is_proxy(t) { None } else { Some(t) }
    }

    /// Resolves the proxies referenced by objects of `r` (`EcoreUtil.resolveAll(resource)`).
    pub fn resolve_resource(&mut self, r: ResourceId) {
        let objs = self.graph.resource_contents(r);
        for o in objs {
            let class = self.graph.class_of(o);
            let feats: Vec<_> = self.graph.meta().class(class).all_features.to_vec();
            for f in feats {
                let fi = self.graph.meta().feature(f);
                if !fi.is_reference() || !fi.resolve_proxies || self.graph.meta().is_container(f) {
                    continue;
                }
                for t in self.targets(o, f) {
                    if self.graph.is_proxy(t)
                        && let Some(n) = self.resolve(t)
                    {
                        self.graph.replace_proxy(o, f, t, n);
                    }
                }
            }
        }
    }

    /// `EcoreUtil.resolveAll(resourceSet)`: every resource, including those loaded meanwhile.
    pub fn resolve_all(&mut self) {
        let mut i = 0;
        while i < self.graph.resource_count() {
            self.resolve_resource(ResourceId(i as u32));
            i += 1;
        }
    }
}

impl Graph {
    /// Replaces proxy `p` by `t` in `holder.f` (proxy resolution: no inverse update, except that
    /// a resolved containment proxy without container becomes contained).
    pub(crate) fn replace_proxy(&mut self, holder: ObjId, f: qvto_model::FeatureId, p: ObjId, t: ObjId) {
        let Ok(i) = self.slot_index(holder, f) else { return };
        match &mut self.objs[holder.index()].slots[i] {
            Slot::Ref(r) if *r == Some(p) => *r = Some(t),
            Slot::Refs(v, _) => {
                for x in v.iter_mut() {
                    if *x == p {
                        *x = t;
                    }
                }
            }
            _ => return,
        }
        if self.meta.feature(f).containment && self.objs[t.index()].container.is_none() {
            self.objs[t.index()].container = Some((holder, f));
            self.objs[p.index()].container = None;
        }
    }
}

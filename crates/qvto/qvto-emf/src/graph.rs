//! The object graph: every object of every resource of a resource set, with EMF's dynamic-object
//! semantics (`DynamicEObjectImpl`, the setting delegates of `EStructuralFeatureImpl`,
//! `EcoreEList.Dynamic`, `BasicEObjectImpl.eBasicSetContainer`, `EcoreUtil.delete`), plus the
//! behaviour of Ecore's generated classes where it differs (see `ecore_hooks`).
//!
//! Objects have one slot per feature of their class, in `eAllStructuralFeatures` order. A slot is
//! `Unset` until written, which is how EMF tells "set" from "unset" for unsettable features.

use crate::datatypes;
use crate::ecore::{EcoreIds, register_ecore};
use crate::ecore_hooks::{self, Hook};
use crate::uri::Uri;
use qvto_model::{
    ClassId, Classifier, DataKind, Datum, FeatureId, FeatureKind, MetaRegistry, ModelError,
    ModelResult, ModelStore, OwnedDatum, PackageId, ResourceId,
};
use std::collections::HashMap;

/// An object of a [`Graph`].
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ObjId(pub u32);

impl ObjId {
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A stored data value (never `Obj`).
pub type Value = OwnedDatum<ObjId>;

#[derive(Clone, Debug)]
pub(crate) enum Slot {
    Unset,
    /// Single-valued data; `Value::Null` is EMF's `NIL` (an explicit null).
    Data(Value),
    /// Single-valued reference; `None` is an explicit null of an unsettable reference.
    Ref(Option<ObjId>),
    /// Many-valued data, with the "set" flag of unsettable lists.
    Datas(Vec<Value>, bool),
    Refs(Vec<ObjId>, bool),
}

#[derive(Clone, Debug)]
pub struct Object {
    pub(crate) class: ClassId,
    pub(crate) container: Option<(ObjId, FeatureId)>,
    /// `eDirectResource()`: the resource whose root list holds the object.
    pub(crate) resource: Option<ResourceId>,
    pub(crate) proxy: Option<Box<str>>,
    pub(crate) slots: Vec<Slot>,
}

/// How a resource is read and written (`XMIResourceFactoryImpl` or `EcoreResourceFactoryImpl`).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ResourceKind {
    /// `XMIResourceImpl` with default options (ASCII encoding unless loaded from a document).
    Xmi,
    /// `.ecore` files: UTF-8, line width 80, encoded attribute style, platform-aware URIs.
    Ecore,
}

/// `xmi:id`s of a resource (`XMLResourceImpl.eObjectToIDMap` / `idToEObjectMap`).
#[derive(Clone, Debug, Default)]
pub(crate) struct IdMaps {
    pub to_id: HashMap<ObjId, String>,
    pub to_obj: HashMap<String, ObjId>,
}

#[derive(Clone, Debug)]
pub struct Resource {
    pub uri: String,
    pub(crate) parsed: Uri,
    pub roots: Vec<ObjId>,
    pub kind: ResourceKind,
    pub encoding: String,
    pub xml_version: String,
    pub xmi_version: String,
    pub(crate) ids: Option<IdMaps>,
    /// Load errors (EMF's `getErrors()`): the reference would reject the document.
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub loaded: bool,
    /// The loaded document had `xsi:schemaLocation`: EMF then adds `OPTION_SCHEMA_LOCATION` to
    /// the resource's default save options.
    pub save_schema_location: bool,
}

impl Resource {
    pub fn parsed_uri(&self) -> &Uri {
        &self.parsed
    }
    /// The `xmi:id` of an object of this resource.
    pub fn xmi_id(&self, o: ObjId) -> Option<&str> {
        self.ids.as_ref()?.to_id.get(&o).map(|s| s.as_str())
    }
    pub fn has_id_maps(&self) -> bool {
        self.ids.is_some()
    }
}

/// Links between registry entries and the Ecore objects (`EClass`, `EAttribute`, ...) that
/// define them, for metamodels registered from loaded `.ecore` resources.
#[derive(Clone, Debug, Default)]
pub struct MetaLinks {
    pub classifier_obj: HashMap<Classifier, ObjId>,
    pub obj_classifier: HashMap<ObjId, Classifier>,
    pub feature_obj: HashMap<FeatureId, ObjId>,
    pub obj_feature: HashMap<ObjId, FeatureId>,
    pub package_obj: HashMap<PackageId, ObjId>,
    pub obj_package: HashMap<ObjId, PackageId>,
}

/// A resource set and all its objects; implements [`ModelStore`].
#[derive(Clone, Debug)]
pub struct Graph {
    pub(crate) meta: MetaRegistry,
    pub(crate) ecore: EcoreIds,
    pub(crate) hooks: Vec<Hook>,
    pub(crate) objs: Vec<Object>,
    pub(crate) resources: Vec<Resource>,
    pub(crate) links: MetaLinks,
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

impl Graph {
    /// An empty graph whose registry holds Ecore.
    pub fn new() -> Graph {
        let mut meta = MetaRegistry::new();
        let ecore = register_ecore(&mut meta);
        let hooks = ecore_hooks::hook_table(&meta, &ecore);
        Graph {
            meta,
            ecore,
            hooks,
            objs: Vec::new(),
            resources: Vec::new(),
            links: MetaLinks::default(),
        }
    }

    /// The registry, for adding metamodels. Ecore's entries must not be changed.
    pub fn meta_mut(&mut self) -> &mut MetaRegistry {
        &mut self.meta
    }
    pub fn ecore(&self) -> &EcoreIds {
        &self.ecore
    }
    pub fn links(&self) -> &MetaLinks {
        &self.links
    }
    pub fn links_mut(&mut self) -> &mut MetaLinks {
        &mut self.links
    }
    pub fn object(&self, o: ObjId) -> &Object {
        &self.objs[o.index()]
    }
    pub fn object_count(&self) -> usize {
        self.objs.len()
    }
    pub fn resource(&self, r: ResourceId) -> &Resource {
        &self.resources[r.index()]
    }
    pub fn resource_mut(&mut self, r: ResourceId) -> &mut Resource {
        &mut self.resources[r.index()]
    }
    pub fn resource_ids(&self) -> impl Iterator<Item = ResourceId> + use<> {
        (0..self.resources.len() as u32).map(ResourceId)
    }
    /// The resource with this URI (exact string match).
    pub fn resource_by_uri(&self, uri: &str) -> Option<ResourceId> {
        self.resources
            .iter()
            .position(|r| r.uri == uri)
            .map(|i| ResourceId(i as u32))
    }

    /// Creates a resource of the given kind (`ResourceSet.createResource`).
    pub fn create_resource_kind(&mut self, uri: &str, kind: ResourceKind) -> ResourceId {
        let id = ResourceId(self.resources.len() as u32);
        self.resources.push(Resource {
            uri: uri.to_string(),
            parsed: Uri::parse(uri),
            roots: Vec::new(),
            kind,
            encoding: match kind {
                ResourceKind::Xmi => "ASCII".into(),
                ResourceKind::Ecore => "UTF-8".into(),
            },
            xml_version: "1.0".into(),
            xmi_version: "2.0".into(),
            ids: None,
            errors: Vec::new(),
            warnings: Vec::new(),
            loaded: false,
            save_schema_location: false,
        });
        id
    }

    /// The `xmi:id` of `o` in its resource.
    pub fn xmi_id(&self, o: ObjId) -> Option<&str> {
        let r = self.resource_of_obj(o)?;
        self.resources[r.index()].xmi_id(o)
    }

    /// Assigns an `xmi:id` (`XMLResource.setID`) in the object's current resource.
    pub fn set_xmi_id(&mut self, r: ResourceId, o: ObjId, id: Option<&str>) {
        let maps = self.resources[r.index()]
            .ids
            .get_or_insert_with(IdMaps::default);
        if let Some(old) = maps.to_id.remove(&o) {
            maps.to_obj.remove(&old);
        }
        if let Some(id) = id {
            maps.to_id.insert(o, id.to_string());
            maps.to_obj.insert(id.to_string(), o);
        }
    }

    // ---- objects ----------------------------------------------------------------------------

    pub(crate) fn new_object(&mut self, class: ClassId) -> ObjId {
        let n = self.meta.class(class).all_features.len();
        let id = ObjId(self.objs.len() as u32);
        self.objs.push(Object {
            class,
            container: None,
            resource: None,
            proxy: None,
            slots: vec![Slot::Unset; n],
        });
        id
    }

    /// Creates an unresolved proxy of class `class` for `uri` (any class, even abstract).
    pub fn create_proxy(&mut self, class: ClassId, uri: &str) -> ObjId {
        let o = self.new_object(class);
        self.objs[o.index()].proxy = Some(uri.into());
        o
    }

    pub(crate) fn set_proxy_uri(&mut self, o: ObjId, uri: Option<&str>) {
        self.objs[o.index()].proxy = uri.map(Into::into);
    }

    /// `eInternalResource()`.
    pub(crate) fn resource_of_obj(&self, o: ObjId) -> Option<ResourceId> {
        let mut cur = o;
        let mut steps = 0;
        loop {
            let obj = &self.objs[cur.index()];
            if let Some(r) = obj.resource {
                return Some(r);
            }
            match obj.container {
                Some((p, _)) => cur = p,
                None => return None,
            }
            steps += 1;
            if steps > 1_000_000 {
                return None;
            }
        }
    }

    pub fn root_container(&self, o: ObjId) -> ObjId {
        let mut cur = o;
        while let Some((p, _)) = self.objs[cur.index()].container {
            if p == o {
                break;
            }
            cur = p;
        }
        cur
    }

    /// `EcoreUtil.isAncestor(ancestor, o)`: `ancestor` is `o` or contains it.
    pub fn is_ancestor(&self, ancestor: ObjId, o: ObjId) -> bool {
        let mut cur = Some(o);
        let mut steps = 0;
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.objs[c.index()].container.map(|(p, _)| p);
            steps += 1;
            if steps > 1_000_000 {
                return false;
            }
        }
        false
    }

    // ---- features ---------------------------------------------------------------------------

    #[inline]
    pub(crate) fn slot_index(&self, o: ObjId, f: FeatureId) -> ModelResult<usize> {
        let class = self.objs[o.index()].class;
        self.meta
            .slot(class, f)
            .ok_or(ModelError::NoSuchFeature { class, feature: f })
    }

    #[inline]
    pub(crate) fn slot(&self, o: ObjId, f: FeatureId) -> Option<&Slot> {
        let i = self.meta.slot(self.objs[o.index()].class, f)?;
        Some(&self.objs[o.index()].slots[i])
    }

    #[inline]
    pub(crate) fn hook(&self, f: FeatureId) -> Hook {
        self.hooks.get(f.index()).copied().unwrap_or(Hook::None)
    }

    /// The default of a single-valued attribute: `defaultValueLiteral` parsed, else the
    /// intrinsic default of its type.
    pub(crate) fn default_datum(&self, f: FeatureId) -> Datum<'_, ObjId> {
        let fi = self.meta.feature(f);
        if fi.kind != FeatureKind::Attribute || fi.is_many() {
            return Datum::Null;
        }
        if let Some(lit) = fi.default_literal.as_deref() {
            return match datatypes::parse_value(&self.meta, fi.ty, lit, Some(self.ecore.package)) {
                Ok(Value::Str(_)) => Datum::Str(lit),
                Ok(Value::Other(_)) => Datum::Other(lit),
                Ok(v) => owned_scalar(&v),
                Err(_) => Datum::Null,
            };
        }
        owned_scalar(&datatypes::intrinsic_default(&self.meta, fi.ty))
    }

    /// Checks that `v` may be stored in `f` (EMF's `ClassCastException`).
    pub(crate) fn check_value(&self, f: FeatureId, v: &Datum<'_, ObjId>) -> ModelResult<()> {
        let fi = self.meta.feature(f);
        let ok = match (fi.kind, v) {
            (_, Datum::Null) => true,
            (FeatureKind::Reference, Datum::Obj(x)) => match fi.ty {
                Classifier::Class(c) => self.meta.is_subclass(self.objs[x.index()].class, c),
                _ => false,
            },
            (FeatureKind::Reference, _) => false,
            (FeatureKind::Attribute, Datum::Obj(_)) => false,
            (FeatureKind::Attribute, d) => match fi.ty {
                Classifier::Enum(e) => {
                    matches!(d, Datum::Enum(e2, i) if *e2 == e && (*i as usize) < self.meta.enumeration(e).literals.len())
                }
                Classifier::DataType(dt) => {
                    let info = self.meta.data_type(dt);
                    kind_accepts(info.kind, d)
                        || java_supertype_accepts(info.instance_class_name.as_deref(), d)
                }
                Classifier::Class(_) => false,
            },
        };
        if ok {
            Ok(())
        } else {
            Err(ModelError::TypeMismatch {
                feature: f,
                found: self.describe_datum(v),
            })
        }
    }

    fn describe_datum(&self, v: &Datum<'_, ObjId>) -> String {
        match v {
            Datum::Obj(x) => self.meta.qualified_name(self.objs[x.index()].class),
            Datum::Enum(e, _) => self.meta.enumeration(*e).name.to_string(),
            d => format!("{d:?}")
                .split('(')
                .next()
                .unwrap_or("?")
                .to_string(),
        }
    }

    fn require_many(&self, f: FeatureId) -> ModelResult<()> {
        if self.meta.feature(f).is_many() {
            Ok(())
        } else {
            Err(ModelError::WrongMultiplicity(f))
        }
    }

    // ---- raw list access --------------------------------------------------------------------

    pub(crate) fn refs(&self, o: ObjId, f: FeatureId) -> &[ObjId] {
        match self.slot(o, f) {
            Some(Slot::Refs(v, _)) => v,
            _ => &[],
        }
    }

    pub(crate) fn single_ref(&self, o: ObjId, f: FeatureId) -> Option<ObjId> {
        match self.slot(o, f) {
            Some(Slot::Ref(r)) => *r,
            _ => None,
        }
    }

    fn refs_mut(&mut self, o: ObjId, f: FeatureId) -> &mut Vec<ObjId> {
        let i = self.slot_index(o, f).expect("feature of class");
        let s = &mut self.objs[o.index()].slots[i];
        if !matches!(s, Slot::Refs(..)) {
            *s = Slot::Refs(Vec::new(), false);
        }
        match s {
            Slot::Refs(v, _) => v,
            _ => unreachable!(),
        }
    }

    fn datas_mut(&mut self, o: ObjId, f: FeatureId) -> &mut Vec<Value> {
        let i = self.slot_index(o, f).expect("feature of class");
        let s = &mut self.objs[o.index()].slots[i];
        if !matches!(s, Slot::Datas(..)) {
            *s = Slot::Datas(Vec::new(), false);
        }
        match s {
            Slot::Datas(v, _) => v,
            _ => unreachable!(),
        }
    }

    /// Marks an unsettable list as set (any change sets it; only `unset` clears it).
    fn mark_set(&mut self, o: ObjId, f: FeatureId) {
        if let Ok(i) = self.slot_index(o, f) {
            match &mut self.objs[o.index()].slots[i] {
                Slot::Refs(_, b) | Slot::Datas(_, b) => *b = true,
                s @ Slot::Unset => {
                    *s = if self.meta.feature(f).kind == FeatureKind::Reference {
                        Slot::Refs(Vec::new(), true)
                    } else {
                        Slot::Datas(Vec::new(), true)
                    }
                }
                _ => {}
            }
        }
    }

    fn set_slot(&mut self, o: ObjId, f: FeatureId, s: Slot) {
        let i = self.slot_index(o, f).expect("feature of class");
        self.objs[o.index()].slots[i] = s;
    }

    // ---- EMF internals ----------------------------------------------------------------------

    /// `eBasicSetContainer`: sets the container, moving the object out of its resource's root
    /// list when it becomes contained through a non-resolving containment.
    pub(crate) fn basic_set_container(&mut self, child: ObjId, new: Option<(ObjId, FeatureId)>) {
        let old_direct = self.objs[child.index()].resource;
        let (old_res, new_res);
        if let Some(r) = old_direct {
            match new {
                Some((p, f)) if !self.meta.feature(f).resolve_proxies => {
                    let roots = &mut self.resources[r.index()].roots;
                    if let Some(i) = roots.iter().position(|x| *x == child) {
                        roots.remove(i);
                    }
                    self.objs[child.index()].resource = None;
                    old_res = Some(r);
                    new_res = self.resource_of_obj(p);
                }
                _ => {
                    old_res = None;
                    new_res = None;
                }
            }
        } else {
            old_res = self.objs[child.index()]
                .container
                .and_then(|(p, _)| self.resource_of_obj(p));
            new_res = new.and_then(|(p, _)| self.resource_of_obj(p));
        }
        if old_res != new_res
            && let Some(r) = old_res
        {
            self.detached(r, child);
        }
        self.objs[child.index()].container = new;
    }

    /// `ResourceImpl.detached`: drops the `xmi:id`s of the object and its contents.
    fn detached(&mut self, r: ResourceId, o: ObjId) {
        if self.resources[r.index()].ids.is_none() {
            return;
        }
        let mut all = vec![o];
        all.extend(self.all_contents(o));
        let maps = self.resources[r.index()].ids.as_mut().unwrap();
        for x in all {
            if let Some(id) = maps.to_id.remove(&x) {
                maps.to_obj.remove(&id);
            }
        }
    }

    /// `eBasicRemoveFromContainer`: removes the object from its container's feature (no
    /// inverse update; the caller sets the new container).
    pub(crate) fn remove_from_container(&mut self, child: ObjId) {
        let Some((p, f)) = self.objs[child.index()].container else {
            return;
        };
        if self.meta.feature(f).is_many() {
            let l = self.refs_mut(p, f);
            if let Some(i) = l.iter().position(|x| *x == child) {
                l.remove(i);
            }
            self.mark_set(p, f);
        } else {
            self.set_slot(p, f, Slot::Unset);
        }
    }

    /// The link `source.f -> target` was made: update `target`'s side (`eInverseAdd`).
    pub(crate) fn inverse_add(&mut self, target: ObjId, source: ObjId, f: FeatureId) {
        let fi = self.meta.feature(f);
        if fi.containment && !self.containment_with_plain_inverse(f) {
            if self.objs[target.index()].container.is_some() {
                self.remove_from_container(target);
            }
            self.basic_set_container(target, Some((source, f)));
        } else if let Some(g) = fi.opposite {
            if self.meta.feature(g).is_many() {
                self.refs_mut(target, g).push(source);
                self.mark_set(target, g);
            } else {
                let old = self.single_ref(target, g);
                self.set_slot(target, g, Slot::Ref(Some(source)));
                if let Some(old) = old
                    && old != source
                {
                    self.inverse_remove(old, target, g);
                }
            }
        }
    }

    /// The link `source.f -> target` was cut: update `target`'s side (`eInverseRemove`).
    pub(crate) fn inverse_remove(&mut self, target: ObjId, source: ObjId, f: FeatureId) {
        let fi = self.meta.feature(f);
        if fi.containment && !self.containment_with_plain_inverse(f) {
            self.basic_set_container(target, None);
        } else if let Some(g) = fi.opposite {
            if self.meta.is_container(g) {
                return;
            }
            if self.meta.feature(g).is_many() {
                let l = self.refs_mut(target, g);
                if let Some(i) = l.iter().position(|x| *x == source) {
                    l.remove(i);
                }
                self.mark_set(target, g);
            } else {
                self.set_slot(target, g, Slot::Unset);
            }
        }
    }

    /// A containment whose `eOpposite` is no container reference (the opposite was set on one
    /// side only, as QVTo does for `composes x : T opposites y` of intermediate classes): EMF's
    /// `EObjectContainmentWithInverseEList` then only updates the opposite's value on the
    /// target (`eInverseAdd` with a plain reference delegate); the target gets no container.
    fn containment_with_plain_inverse(&self, f: FeatureId) -> bool {
        self.meta
            .feature(f)
            .opposite
            .is_some_and(|g| !self.meta.is_container(g))
    }

    fn has_inverse(&self, f: FeatureId) -> bool {
        let fi = self.meta.feature(f);
        fi.containment || fi.opposite.is_some()
    }

    /// `eSet` of a single-valued reference (not a container reference).
    pub(crate) fn set_ref_single(&mut self, o: ObjId, f: FeatureId, new: Option<ObjId>) {
        let old = self.single_ref(o, f);
        if old != new && self.has_inverse(f) {
            if let Some(old) = old {
                self.inverse_remove(old, o, f);
            }
            if let Some(n) = new {
                self.inverse_add(n, o, f);
            }
        }
        let slot = match new {
            Some(x) => Slot::Ref(Some(x)),
            None if self.meta.feature(f).unsettable => Slot::Ref(None),
            None => Slot::Unset,
        };
        self.set_slot(o, f, slot);
    }

    /// `eSet` of a container reference (`InternalSettingDelegateSingleContainer.dynamicSet`).
    fn set_container_feature(
        &mut self,
        o: ObjId,
        f: FeatureId,
        new: Option<ObjId>,
    ) -> ModelResult<()> {
        let g = self.meta.feature(f).opposite.expect("container reference");
        let cur = self.objs[o.index()].container;
        let same = match (cur, new) {
            (Some((p, cf)), Some(n)) => p == n && cf == g,
            (None, None) => true,
            _ => false,
        };
        if same {
            return Ok(());
        }
        if let Some(n) = new
            && self.is_ancestor(o, n)
        {
            return Err(ModelError::RecursiveContainment);
        }
        if cur.is_some() {
            self.remove_from_container(o);
        }
        if let Some(n) = new {
            if self.meta.feature(g).is_many() {
                self.refs_mut(n, g).push(o);
                self.mark_set(n, g);
            } else {
                let old = self.single_ref(n, g);
                self.set_slot(n, g, Slot::Ref(Some(o)));
                if let Some(old) = old
                    && old != o
                {
                    self.basic_set_container(old, None);
                }
            }
        }
        self.basic_set_container(o, new.map(|n| (n, g)));
        Ok(())
    }

    // ---- operations (the ModelStore semantics) ------------------------------------------------

    pub(crate) fn op_get(&self, o: ObjId, f: FeatureId) -> Datum<'_, ObjId> {
        let h = self.hook(f);
        if h != Hook::None
            && let Some(d) = ecore_hooks::get(self, o, f, h)
        {
            return d;
        }
        let fi = self.meta.feature(f);
        if fi.is_many() {
            return Datum::Null;
        }
        if self.meta.is_container(f) {
            return match self.objs[o.index()].container {
                Some((p, cf)) if Some(cf) == fi.opposite => Datum::Obj(p),
                _ => Datum::Null,
            };
        }
        match self.slot(o, f) {
            None => Datum::Null,
            Some(Slot::Unset) => self.default_datum(f),
            Some(Slot::Data(v)) => v.as_datum(),
            Some(Slot::Ref(Some(x))) => Datum::Obj(*x),
            Some(_) => Datum::Null,
        }
    }

    pub(crate) fn op_len(&self, o: ObjId, f: FeatureId) -> usize {
        let h = self.hook(f);
        if h != Hook::None
            && let Some(n) = ecore_hooks::len(self, o, f, h)
        {
            return n;
        }
        match self.slot(o, f) {
            Some(Slot::Refs(v, _)) => v.len(),
            Some(Slot::Datas(v, _)) => v.len(),
            _ => 0,
        }
    }

    pub(crate) fn op_get_at(&self, o: ObjId, f: FeatureId, i: usize) -> Datum<'_, ObjId> {
        let h = self.hook(f);
        if h != Hook::None
            && let Some(d) = ecore_hooks::get_at(self, o, f, h, i)
        {
            return d;
        }
        match self.slot(o, f) {
            Some(Slot::Refs(v, _)) => v.get(i).map_or(Datum::Null, |x| Datum::Obj(*x)),
            Some(Slot::Datas(v, _)) => v.get(i).map_or(Datum::Null, |x| x.as_datum()),
            _ => Datum::Null,
        }
    }

    pub(crate) fn op_is_set(&self, o: ObjId, f: FeatureId) -> bool {
        let h = self.hook(f);
        if h != Hook::None
            && let Some(b) = ecore_hooks::is_set(self, o, f, h)
        {
            return b;
        }
        let fi = self.meta.feature(f);
        if self.meta.is_container(f) {
            return matches!(self.objs[o.index()].container, Some((_, cf)) if Some(cf) == fi.opposite);
        }
        match self.slot(o, f) {
            None | Some(Slot::Unset) => false,
            Some(Slot::Refs(v, b)) => {
                if fi.unsettable {
                    *b
                } else {
                    !v.is_empty()
                }
            }
            Some(Slot::Datas(v, b)) => {
                if fi.unsettable {
                    *b
                } else {
                    !v.is_empty()
                }
            }
            Some(Slot::Ref(_)) => true,
            Some(Slot::Data(Value::Null)) => true,
            Some(Slot::Data(v)) => {
                fi.unsettable || !v.as_datum().java_equals(&self.default_datum(f))
            }
        }
    }

    pub(crate) fn op_set(
        &mut self,
        o: ObjId,
        f: FeatureId,
        v: Datum<'_, ObjId>,
    ) -> ModelResult<()> {
        self.slot_index(o, f)?;
        if !self.meta.feature(f).changeable {
            return Err(ModelError::NotChangeable(f));
        }
        let h = self.hook(f);
        if h != Hook::None
            && let Some(r) = ecore_hooks::set(self, o, f, h, v)
        {
            return r;
        }
        let fi = self.meta.feature(f);
        if fi.is_many() {
            return Err(ModelError::WrongMultiplicity(f));
        }
        self.check_value(f, &v)?;
        let kind = fi.kind;
        if kind == FeatureKind::Reference {
            let new = v.as_obj();
            if self.meta.is_container(f) {
                return self.set_container_feature(o, f, new);
            }
            self.set_ref_single(o, f, new);
            return Ok(());
        }
        let slot = if v.is_null() {
            let intrinsic = datatypes::intrinsic_default(&self.meta, fi.ty);
            if !intrinsic.as_datum().is_null() {
                Slot::Unset
            } else if fi.unsettable || !self.default_datum(f).is_null() {
                Slot::Data(Value::Null)
            } else {
                Slot::Unset
            }
        } else {
            Slot::Data(v.to_owned())
        };
        self.set_slot(o, f, slot);
        Ok(())
    }

    pub(crate) fn op_unset(&mut self, o: ObjId, f: FeatureId) -> ModelResult<()> {
        self.slot_index(o, f)?;
        if !self.meta.feature(f).changeable {
            return Err(ModelError::NotChangeable(f));
        }
        let h = self.hook(f);
        if h != Hook::None
            && let Some(r) = ecore_hooks::unset(self, o, f, h)
        {
            return r;
        }
        let fi = self.meta.feature(f);
        if fi.is_many() {
            self.op_clear(o, f)?;
            let i = self.slot_index(o, f)?;
            match &mut self.objs[o.index()].slots[i] {
                Slot::Refs(_, b) | Slot::Datas(_, b) => *b = false,
                _ => {}
            }
            return Ok(());
        }
        if fi.kind == FeatureKind::Reference {
            if self.meta.is_container(f) {
                // InternalSettingDelegateSingleContainer.dynamicUnset: detaches from any container
                if self.objs[o.index()].container.is_some() {
                    self.remove_from_container(o);
                    self.basic_set_container(o, None);
                }
                return Ok(());
            }
            if let Some(old) = self.single_ref(o, f)
                && self.has_inverse(f)
            {
                self.inverse_remove(old, o, f);
            }
        }
        self.set_slot(o, f, Slot::Unset);
        Ok(())
    }

    /// Index of `v` in the list (`indexOf`: identity for objects, `equals` for data).
    pub(crate) fn index_of(&self, o: ObjId, f: FeatureId, v: &Datum<'_, ObjId>) -> Option<usize> {
        (0..self.op_len(o, f)).find(|&i| self.op_get_at(o, f, i).java_equals(v))
    }

    pub(crate) fn op_add(
        &mut self,
        o: ObjId,
        f: FeatureId,
        v: Datum<'_, ObjId>,
        at: Option<usize>,
    ) -> ModelResult<bool> {
        self.slot_index(o, f)?;
        self.require_many(f)?;
        // EMF's order: bounds, uniqueness, then validation (null, type)
        let fi = self.meta.feature(f);
        let (unique, is_ref) = (fi.unique, fi.kind == FeatureKind::Reference);
        let len = self.op_len(o, f);
        if let Some(i) = at
            && i > len
        {
            return Err(ModelError::IndexOutOfBounds {
                index: i,
                size: len,
            });
        }
        if unique && self.index_of(o, f, &v).is_some() {
            return match at {
                None => Ok(false),
                Some(_) => Err(ModelError::Duplicate(f)),
            };
        }
        if is_ref && v.is_null() {
            return Err(ModelError::NullNotAllowed(f));
        }
        self.check_value(f, &v)?;
        self.add_unique(o, f, v, at);
        Ok(true)
    }

    /// `InternalEList.addUnique`: inserts without the uniqueness check, with inverse updates.
    pub(crate) fn add_unique(
        &mut self,
        o: ObjId,
        f: FeatureId,
        v: Datum<'_, ObjId>,
        at: Option<usize>,
    ) {
        let h = self.hook(f);
        if h != Hook::None && ecore_hooks::add_unique(self, o, f, h, v, at) {
            return;
        }
        match v {
            Datum::Obj(x) => {
                let l = self.refs_mut(o, f);
                let i = at.unwrap_or(l.len()).min(l.len());
                l.insert(i, x);
                self.mark_set(o, f);
                if self.has_inverse(f) {
                    self.inverse_add(x, o, f);
                }
            }
            d => {
                let val = d.to_owned();
                let l = self.datas_mut(o, f);
                let i = at.unwrap_or(l.len()).min(l.len());
                l.insert(i, val);
                self.mark_set(o, f);
            }
        }
    }

    pub(crate) fn op_remove(
        &mut self,
        o: ObjId,
        f: FeatureId,
        v: Datum<'_, ObjId>,
    ) -> ModelResult<bool> {
        self.slot_index(o, f)?;
        self.require_many(f)?;
        match self.index_of(o, f, &v) {
            Some(i) => {
                self.op_remove_at(o, f, i)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub(crate) fn op_remove_at(&mut self, o: ObjId, f: FeatureId, i: usize) -> ModelResult<()> {
        self.slot_index(o, f)?;
        self.require_many(f)?;
        let len = self.op_len(o, f);
        if i >= len {
            return Err(ModelError::IndexOutOfBounds {
                index: i,
                size: len,
            });
        }
        let h = self.hook(f);
        if h != Hook::None
            && let Some(r) = ecore_hooks::remove_at(self, o, f, h, i)
        {
            return r;
        }
        if self.meta.feature(f).kind == FeatureKind::Reference {
            let x = self.refs_mut(o, f).remove(i);
            self.mark_set(o, f);
            if self.has_inverse(f) {
                self.inverse_remove(x, o, f);
            }
        } else {
            self.datas_mut(o, f).remove(i);
            self.mark_set(o, f);
        }
        Ok(())
    }

    pub(crate) fn op_set_at(
        &mut self,
        o: ObjId,
        f: FeatureId,
        i: usize,
        v: Datum<'_, ObjId>,
    ) -> ModelResult<()> {
        self.slot_index(o, f)?;
        self.require_many(f)?;
        let fi = self.meta.feature(f);
        let (unique, is_ref) = (fi.unique, fi.kind == FeatureKind::Reference);
        let len = self.op_len(o, f);
        if i >= len {
            return Err(ModelError::IndexOutOfBounds {
                index: i,
                size: len,
            });
        }
        if unique
            && let Some(j) = self.index_of(o, f, &v)
            && j != i
        {
            return Err(ModelError::Duplicate(f));
        }
        if is_ref && v.is_null() {
            return Err(ModelError::NullNotAllowed(f));
        }
        self.check_value(f, &v)?;
        let h = self.hook(f);
        if h != Hook::None
            && let Some(r) = ecore_hooks::set_at(self, o, f, h, i, v)
        {
            return r;
        }
        match v {
            Datum::Obj(x) => {
                let old = std::mem::replace(&mut self.refs_mut(o, f)[i], x);
                self.mark_set(o, f);
                if old != x && self.has_inverse(f) {
                    self.inverse_remove(old, o, f);
                    self.inverse_add(x, o, f);
                }
            }
            d => {
                let val = d.to_owned();
                self.datas_mut(o, f)[i] = val;
                self.mark_set(o, f);
            }
        }
        Ok(())
    }

    pub(crate) fn op_clear(&mut self, o: ObjId, f: FeatureId) -> ModelResult<()> {
        self.slot_index(o, f)?;
        self.require_many(f)?;
        let h = self.hook(f);
        if h != Hook::None
            && let Some(r) = ecore_hooks::clear(self, o, f, h)
        {
            return r;
        }
        if self.meta.feature(f).kind == FeatureKind::Reference {
            let old = std::mem::take(self.refs_mut(o, f));
            self.mark_set(o, f);
            if self.has_inverse(f) {
                for x in old {
                    self.inverse_remove(x, o, f);
                }
            }
        } else {
            self.datas_mut(o, f).clear();
            self.mark_set(o, f);
        }
        Ok(())
    }

    pub(crate) fn op_move(
        &mut self,
        o: ObjId,
        f: FeatureId,
        to: usize,
        from: usize,
    ) -> ModelResult<()> {
        self.slot_index(o, f)?;
        self.require_many(f)?;
        let len = self.op_len(o, f);
        if to >= len {
            return Err(ModelError::IndexOutOfBounds {
                index: to,
                size: len,
            });
        }
        if from >= len {
            return Err(ModelError::IndexOutOfBounds {
                index: from,
                size: len,
            });
        }
        let h = self.hook(f);
        if h != Hook::None
            && let Some(r) = ecore_hooks::move_to(self, o, f, h, to, from)
        {
            return r;
        }
        if self.meta.feature(f).kind == FeatureKind::Reference {
            let l = self.refs_mut(o, f);
            let x = l.remove(from);
            l.insert(to, x);
        } else {
            let l = self.datas_mut(o, f);
            let x = l.remove(from);
            l.insert(to, x);
        }
        self.mark_set(o, f);
        Ok(())
    }

    /// `ResourceImpl.getContents().add` (`eSetResource`).
    pub(crate) fn op_add_root(
        &mut self,
        r: ResourceId,
        o: ObjId,
        at: Option<usize>,
    ) -> ModelResult<bool> {
        let len = self.resources[r.index()].roots.len();
        if let Some(i) = at
            && i > len
        {
            return Err(ModelError::IndexOutOfBounds {
                index: i,
                size: len,
            });
        }
        if self.resources[r.index()].roots.contains(&o) {
            return match at {
                None => Ok(false),
                Some(_) => Err(ModelError::Unsupported(
                    "The 'no duplicates' constraint is violated".into(),
                )),
            };
        }
        self.resources[r.index()].roots.insert(at.unwrap_or(len), o);
        self.set_resource(o, Some(r));
        Ok(true)
    }

    /// `eSetResource(resource)` after the object was added to / removed from a root list.
    fn set_resource(&mut self, o: ObjId, r: Option<ResourceId>) {
        let old = self.objs[o.index()].resource;
        if let (Some(old), Some(_)) = (old, r) {
            let roots = &mut self.resources[old.index()].roots;
            if let Some(i) = roots.iter().position(|x| *x == o) {
                roots.remove(i);
            }
            self.detached(old, o);
        }
        if let Some((_, f)) = self.objs[o.index()].container {
            if self.meta.feature(f).resolve_proxies {
                // cross-resource containment: stays contained
            } else {
                let cr = self.resource_of_obj(o);
                self.remove_from_container(o);
                self.objs[o.index()].container = None;
                if let Some(cr) = cr
                    && Some(cr) != r
                {
                    self.detached(cr, o);
                }
            }
        }
        if let (Some(old), None) = (old, r) {
            self.detached(old, o);
        }
        self.objs[o.index()].resource = r;
    }

    pub(crate) fn op_remove_root(&mut self, r: ResourceId, o: ObjId) -> bool {
        let roots = &mut self.resources[r.index()].roots;
        match roots.iter().position(|x| *x == o) {
            Some(i) => {
                roots.remove(i);
                if self.objs[o.index()].resource == Some(r) {
                    self.detached(r, o);
                    self.objs[o.index()].resource = None;
                }
                true
            }
            None => false,
        }
    }

    /// `EcoreUtil.delete(o)`.
    pub(crate) fn op_delete(&mut self, o: ObjId) {
        let root = self.root_container(o);
        let scope: Vec<ObjId> = match self.resource_of_obj(root) {
            Some(_) => self
                .resource_ids()
                .flat_map(|r| self.resource_contents(r))
                .collect(),
            None => {
                let mut v = vec![root];
                v.extend(self.all_contents(root));
                v
            }
        };
        let mut usages: Vec<(ObjId, FeatureId)> = Vec::new();
        for x in scope {
            let class = self.objs[x.index()].class;
            for &f in self.meta.class(class).cross_references.iter() {
                let fi = self.meta.feature(f);
                if fi.derived || !fi.changeable {
                    continue;
                }
                if fi.is_many() {
                    for &y in self.refs(x, f) {
                        if y == o {
                            usages.push((x, f));
                        }
                    }
                } else if self.single_ref(x, f) == Some(o) {
                    usages.push((x, f));
                }
            }
        }
        for (x, f) in usages {
            if self.meta.feature(f).is_many() {
                let _ = self.op_remove(x, f, Datum::Obj(o));
            } else {
                let _ = self.op_unset(x, f);
            }
        }
        if let Some((p, f)) = self.objs[o.index()].container {
            if self.meta.feature(f).is_many() {
                let _ = self.op_remove(p, f, Datum::Obj(o));
            } else {
                let _ = self.op_unset(p, f);
            }
        }
        if let Some(r) = self.objs[o.index()].resource {
            self.op_remove_root(r, o);
        }
    }
}

fn owned_scalar(v: &Value) -> Datum<'static, ObjId> {
    match *v {
        Value::Null => Datum::Null,
        Value::Bool(b) => Datum::Bool(b),
        Value::Int(i) => Datum::Int(i),
        Value::Long(i) => Datum::Long(i),
        Value::Short(i) => Datum::Short(i),
        Value::Byte(i) => Datum::Byte(i),
        Value::Char(c) => Datum::Char(c),
        Value::Double(d) => Datum::Double(d),
        Value::Float(f) => Datum::Float(f),
        Value::Enum(e, i) => Datum::Enum(e, i),
        Value::Obj(x) => Datum::Obj(x),
        Value::Str(_) | Value::Other(_) => Datum::Null,
    }
}

/// Data types whose Java instance class is a supertype of the boxes (`EJavaObject`,
/// `java.lang.Number`, e.g. the erasure of a type parameter bounded by a `Number` data type)
/// accept those values (EMF checks `eType.isInstance(value)`).
fn java_supertype_accepts(instance_class: Option<&str>, d: &Datum<'_, ObjId>) -> bool {
    let number = matches!(
        d,
        Datum::Int(_)
            | Datum::Long(_)
            | Datum::Short(_)
            | Datum::Byte(_)
            | Datum::Double(_)
            | Datum::Float(_)
    );
    match instance_class {
        Some("java.lang.Number") => number,
        Some("java.lang.Object") => number || matches!(d, Datum::Bool(_) | Datum::Char(_)),
        _ => false,
    }
}

fn kind_accepts(kind: DataKind, d: &Datum<'_, ObjId>) -> bool {
    matches!(
        (kind, d),
        (DataKind::String, Datum::Str(_))
            | (DataKind::Boolean, Datum::Bool(_))
            | (DataKind::Int, Datum::Int(_))
            | (DataKind::Long, Datum::Long(_))
            | (DataKind::Short, Datum::Short(_))
            | (DataKind::Byte, Datum::Byte(_))
            | (DataKind::Char, Datum::Char(_))
            | (DataKind::Double, Datum::Double(_))
            | (DataKind::Float, Datum::Float(_))
            | (
                DataKind::BigInteger | DataKind::BigDecimal | DataKind::Date | DataKind::Other,
                Datum::Other(_) | Datum::Str(_)
            )
            | (DataKind::Unloadable, _)
            // an `Enumerator`/`Object` data type (EMF's nullable enumerations) holds literals
            | (DataKind::Other, Datum::Enum(..))
    )
}

impl ModelStore for Graph {
    type Obj = ObjId;

    fn extend_meta(&mut self, meta: &MetaRegistry) -> bool {
        if !meta.extends(&self.meta) {
            return false;
        }
        // the registry of a dynamic resource set only grows: the new entries need no links
        // (they have no `EClass` objects) and no hooks (they are not Ecore's)
        self.meta = meta.clone();
        true
    }

    fn meta(&self) -> &MetaRegistry {
        &self.meta
    }

    fn classifier_object(&self, c: Classifier) -> Option<ObjId> {
        self.links.classifier_obj.get(&c).copied()
    }

    fn object_classifier(&self, o: ObjId) -> Option<Classifier> {
        self.links.obj_classifier.get(&o).copied()
    }

    fn feature_object(&self, f: FeatureId) -> Option<ObjId> {
        self.links.feature_obj.get(&f).copied()
    }

    fn object_feature(&self, o: ObjId) -> Option<FeatureId> {
        self.links.obj_feature.get(&o).copied()
    }

    fn package_object(&self, p: PackageId) -> Option<ObjId> {
        self.links.package_obj.get(&p).copied()
    }

    fn object_package(&self, o: ObjId) -> Option<PackageId> {
        self.links.obj_package.get(&o).copied()
    }
    fn class_of(&self, o: ObjId) -> ClassId {
        self.objs[o.index()].class
    }
    fn is_proxy(&self, o: ObjId) -> bool {
        self.objs[o.index()].proxy.is_some()
    }
    fn proxy_uri(&self, o: ObjId) -> Option<&str> {
        self.objs[o.index()].proxy.as_deref()
    }
    fn get(&self, o: ObjId, f: FeatureId) -> Datum<'_, ObjId> {
        self.op_get(o, f)
    }
    fn len(&self, o: ObjId, f: FeatureId) -> usize {
        self.op_len(o, f)
    }
    fn get_at(&self, o: ObjId, f: FeatureId, i: usize) -> Datum<'_, ObjId> {
        self.op_get_at(o, f, i)
    }
    fn is_set(&self, o: ObjId, f: FeatureId) -> bool {
        self.op_is_set(o, f)
    }
    fn container(&self, o: ObjId) -> Option<(ObjId, FeatureId)> {
        self.objs[o.index()].container
    }
    fn resource_of(&self, o: ObjId) -> Option<ResourceId> {
        self.resource_of_obj(o)
    }
    fn direct_resource(&self, o: ObjId) -> Option<ResourceId> {
        self.objs[o.index()].resource
    }
    fn resource_count(&self) -> usize {
        self.resources.len()
    }
    fn resource_uri(&self, r: ResourceId) -> &str {
        &self.resources[r.index()].uri
    }
    fn roots(&self, r: ResourceId) -> &[ObjId] {
        &self.resources[r.index()].roots
    }
    fn create(&mut self, c: ClassId) -> ModelResult<ObjId> {
        let ci = self.meta.class(c);
        if ci.is_abstract || ci.is_interface {
            return Err(ModelError::Abstract(c));
        }
        Ok(self.new_object(c))
    }
    fn set(&mut self, o: ObjId, f: FeatureId, v: Datum<'_, ObjId>) -> ModelResult<()> {
        self.op_set(o, f, v)
    }
    fn unset(&mut self, o: ObjId, f: FeatureId) -> ModelResult<()> {
        self.op_unset(o, f)
    }
    fn add(
        &mut self,
        o: ObjId,
        f: FeatureId,
        v: Datum<'_, ObjId>,
        at: Option<usize>,
    ) -> ModelResult<bool> {
        self.op_add(o, f, v, at)
    }
    fn remove(&mut self, o: ObjId, f: FeatureId, v: Datum<'_, ObjId>) -> ModelResult<bool> {
        self.op_remove(o, f, v)
    }
    fn remove_at(&mut self, o: ObjId, f: FeatureId, i: usize) -> ModelResult<()> {
        self.op_remove_at(o, f, i)
    }
    fn set_at(&mut self, o: ObjId, f: FeatureId, i: usize, v: Datum<'_, ObjId>) -> ModelResult<()> {
        self.op_set_at(o, f, i, v)
    }
    fn clear(&mut self, o: ObjId, f: FeatureId) -> ModelResult<()> {
        self.op_clear(o, f)
    }
    fn move_to(&mut self, o: ObjId, f: FeatureId, to: usize, from: usize) -> ModelResult<()> {
        self.op_move(o, f, to, from)
    }
    fn create_resource(&mut self, uri: &str) -> ResourceId {
        let kind = if Uri::parse(uri).file_extension() == Some("ecore") {
            ResourceKind::Ecore
        } else {
            ResourceKind::Xmi
        };
        self.create_resource_kind(uri, kind)
    }
    fn add_root(&mut self, r: ResourceId, o: ObjId, at: Option<usize>) -> ModelResult<bool> {
        self.op_add_root(r, o, at)
    }
    fn remove_root(&mut self, r: ResourceId, o: ObjId) -> bool {
        self.op_remove_root(r, o)
    }
    fn delete(&mut self, o: ObjId) {
        self.op_delete(o)
    }
}

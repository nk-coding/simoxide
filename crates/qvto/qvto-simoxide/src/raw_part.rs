//! EMF's semantics on SimOxide's `raw::Graph`: the operations of `qvto_emf::Graph` (dynamic EMF
//! objects, see `qvto_model::ModelStore`'s contract) ported to the graph's storage, driven by the
//! registry alone (SimOxide's `ClassId::def()` and the graph's own helpers are not used, so the
//! same code runs on any registry). Every change is recorded in the journal when one is kept.
//!
//! Storage (`raw::Obj::slots`, a feature-keyed list): an absent slot is an unset feature; a
//! single-valued attribute is `Slot::One` (`Value::Null` is an explicit null, kept only where EMF
//! tells it from unset), a single-valued reference `Slot::Ref` (`None`: explicit null of an
//! unsettable reference), lists are `Slot::Many` / `Slot::Refs`. Writes keep the slot present
//! exactly when EMF's `eIsSet` holds, so SimOxide's own reading (`Graph::is_set`, `attr`)
//! agrees.

use crate::journal::ChangeKind;
use crate::store::{EXT_BASE, Res, State};
use qvto_emf::datatypes::{intrinsic_default, parse_value};
use qvto_model::{
    ClassId, Classifier, DataKind, Datum, FeatureId, FeatureKind, MetaRegistry, ModelError,
    ModelResult, ModelStore, OwnedDatum, PackageId, ResourceId,
};
use simoxide_model::meta as sm;
use simoxide_model::raw::{self, ObjId as RawId, Slot, Value};

/// The graph with what the reading operations need besides it.
#[derive(Copy, Clone)]
pub(crate) struct RawView<'x> {
    pub g: &'x raw::Graph,
    pub meta: &'x MetaRegistry,
    pub ext: &'x qvto_emf::Graph,
    pub st: &'x State,
    pub ecore: PackageId,
}

/// The graph with what the operations need besides it.
pub(crate) struct Raw<'x> {
    pub g: &'x mut raw::Graph,
    pub meta: &'x MetaRegistry,
    /// The other part of the store (classes of objects of other metamodels).
    pub ext: &'x qvto_emf::Graph,
    pub st: &'x mut State,
    pub ecore: PackageId,
}

#[inline]
fn sf(f: FeatureId) -> sm::FeatureId {
    sm::FeatureId(f.0)
}

#[inline]
fn rid(o: u32) -> RawId {
    RawId(o)
}

/// The data kind of an attribute's values (`None` for enumerations).
fn kind(meta: &MetaRegistry, f: FeatureId) -> Option<DataKind> {
    match meta.feature(f).ty {
        Classifier::DataType(d) => Some(meta.data_type(d).kind),
        _ => None,
    }
}

/// A stored value as the API sees it.
pub(crate) fn to_datum<'a>(meta: &MetaRegistry, f: FeatureId, v: &'a Value) -> Datum<'a, u32> {
    let k = kind(meta, f);
    match v {
        Value::Null => Datum::Null,
        Value::Bool(b) => Datum::Bool(*b),
        Value::Str(s) => Datum::Str(s),
        Value::Other(s) => match k {
            Some(DataKind::String) => Datum::Str(s),
            Some(DataKind::Char) => Datum::Char(s.encode_utf16().next().unwrap_or(0)),
            _ => Datum::Other(s),
        },
        Value::Int(i) => match k {
            Some(DataKind::Long) => Datum::Long(*i),
            Some(DataKind::Short) => Datum::Short(*i as i16),
            Some(DataKind::Byte) => Datum::Byte(*i as i8),
            Some(DataKind::Double) => Datum::Double(*i as f64),
            Some(DataKind::Float) => Datum::Float(*i as f32),
            _ => Datum::Int(*i as i32),
        },
        Value::Double(d) => match k {
            Some(DataKind::Float) => Datum::Float(*d as f32),
            _ => Datum::Double(*d),
        },
        Value::Enum(i) => match meta.feature(f).ty {
            Classifier::Enum(e) => Datum::Enum(e, *i),
            _ => Datum::Int(*i as i32),
        },
    }
}

/// A value as SimOxide stores it (`simoxide_model::raw::parse_value`'s representation).
pub(crate) fn to_value(meta: &MetaRegistry, f: FeatureId, d: &Datum<'_, u32>) -> Value {
    match *d {
        Datum::Null | Datum::Obj(_) => Value::Null,
        Datum::Bool(b) => Value::Bool(b),
        Datum::Int(i) => Value::Int(i as i64),
        Datum::Long(i) => Value::Int(i),
        Datum::Short(i) => Value::Int(i as i64),
        Datum::Byte(i) => Value::Int(i as i64),
        Datum::Char(c) => Value::Other(String::from_utf16_lossy(&[c]).into()),
        Datum::Double(x) => Value::Double(x),
        Datum::Float(x) => Value::Double(x as f64),
        // kept as given, like `qvto_emf::Graph` (a String stored in an `Object`-like attribute
        // reads back as a String)
        Datum::Str(s) => Value::Str(s.into()),
        Datum::Other(s) => Value::Other(s.into()),
        Datum::Enum(_, i) => Value::Enum(i),
    }
}

fn scalar(v: &qvto_emf::Value) -> Datum<'static, u32> {
    use qvto_model::OwnedDatum as V;
    match *v {
        V::Bool(b) => Datum::Bool(b),
        V::Int(i) => Datum::Int(i),
        V::Long(i) => Datum::Long(i),
        V::Short(i) => Datum::Short(i),
        V::Byte(i) => Datum::Byte(i),
        V::Char(c) => Datum::Char(c),
        V::Double(d) => Datum::Double(d),
        V::Float(f) => Datum::Float(f),
        V::Enum(e, i) => Datum::Enum(e, i),
        _ => Datum::Null,
    }
}

/// The default of a single-valued attribute: `defaultValueLiteral`, else the type's intrinsic
/// default (as `qvto_emf::Graph` computes it).
pub(crate) fn default_datum<'m>(
    meta: &'m MetaRegistry,
    ecore: PackageId,
    f: FeatureId,
) -> Datum<'m, u32> {
    let fi = meta.feature(f);
    if fi.kind != FeatureKind::Attribute || fi.is_many() {
        return Datum::Null;
    }
    if let Some(lit) = fi.default_literal.as_deref() {
        return match parse_value(meta, fi.ty, lit, Some(ecore)) {
            Ok(qvto_emf::Value::Str(_)) => Datum::Str(lit),
            Ok(qvto_emf::Value::Other(_)) => Datum::Other(lit),
            Ok(v) => scalar(&v),
            Err(_) => Datum::Null,
        };
    }
    scalar(&intrinsic_default(meta, fi.ty))
}

/// `EStructuralFeature` value check (`ClassCastException`), as `qvto_emf::Graph::check_value`.
pub(crate) fn accepts(
    meta: &MetaRegistry,
    f: FeatureId,
    v: &Datum<'_, u32>,
    class_of: impl Fn(u32) -> ClassId,
) -> bool {
    let fi = meta.feature(f);
    match (fi.kind, v) {
        (_, Datum::Null) => true,
        (FeatureKind::Reference, Datum::Obj(x)) => match fi.ty {
            Classifier::Class(c) => meta.is_subclass(class_of(*x), c),
            _ => false,
        },
        (FeatureKind::Reference, _) => false,
        (FeatureKind::Attribute, Datum::Obj(_)) => false,
        (FeatureKind::Attribute, d) => match fi.ty {
            Classifier::Enum(e) => {
                matches!(d, Datum::Enum(e2, i) if *e2 == e && (*i as usize) < meta.enumeration(e).literals.len())
            }
            Classifier::DataType(dt) => {
                let info = meta.data_type(dt);
                qvto_emf::graph::data_accepts(info.kind, info.instance_class_name.as_deref(), d)
            }
            Classifier::Class(_) => false,
        },
    }
}

fn describe(meta: &MetaRegistry, v: &Datum<'_, u32>, class_of: impl Fn(u32) -> ClassId) -> String {
    match v {
        Datum::Obj(x) => meta.qualified_name(class_of(*x)),
        Datum::Enum(e, _) => meta.enumeration(*e).name.to_string(),
        d => format!("{d:?}")
            .split('(')
            .next()
            .unwrap_or("?")
            .to_string(),
    }
}

impl<'x> RawView<'x> {
    // ---- reading ------------------------------------------------------------------------------

    pub fn class(&self, o: u32) -> ClassId {
        ClassId(self.g.objs[o as usize].class.0)
    }

    /// The class of any handle of the store.
    pub fn class_of_handle(&self, h: u32) -> ClassId {
        if h >= EXT_BASE {
            self.ext.class_of(qvto_emf::ObjId(h - EXT_BASE))
        } else {
            self.class(h)
        }
    }

    pub fn has_feature(&self, o: u32, f: FeatureId) -> ModelResult<()> {
        let class = self.class(o);
        match self.meta.slot(class, f) {
            Some(_) => Ok(()),
            None => Err(ModelError::NoSuchFeature { class, feature: f }),
        }
    }

    pub fn slot(&self, o: u32, f: FeatureId) -> Option<&'x Slot> {
        let g: &'x raw::Graph = self.g;
        g.objs[o as usize]
            .slots
            .iter()
            .find(|(g, _)| g.0 == f.0)
            .map(|(_, s)| s)
    }

    pub fn refs(&self, o: u32, f: FeatureId) -> &'x [RawId] {
        match self.slot(o, f) {
            Some(Slot::Refs(v)) => v,
            Some(Slot::Ref(Some(x))) if self.meta.feature(f).is_many() => std::slice::from_ref(x),
            _ => &[],
        }
    }

    pub fn single_ref(&self, o: u32, f: FeatureId) -> Option<u32> {
        match self.slot(o, f) {
            Some(Slot::Ref(r)) => r.map(|x| x.0),
            Some(Slot::Refs(v)) => v.first().map(|x| x.0),
            _ => None,
        }
    }

    pub fn container(&self, o: u32) -> Option<(u32, FeatureId)> {
        self.g.objs[o as usize]
            .container
            .map(|(p, f)| (p.0, FeatureId(f.0)))
    }

    pub fn default_datum(&self, f: FeatureId) -> Datum<'x, u32> {
        default_datum(self.meta, self.ecore, f)
    }

    pub fn get(&self, o: u32, f: FeatureId) -> Datum<'x, u32> {
        let fi = self.meta.feature(f);
        if fi.is_many() || self.meta.slot(self.class(o), f).is_none() {
            return Datum::Null;
        }
        if self.meta.is_container(f) {
            return match self.container(o) {
                Some((p, cf)) if Some(cf) == fi.opposite => Datum::Obj(p),
                _ => Datum::Null,
            };
        }
        match self.slot(o, f) {
            None => self.default_datum(f),
            Some(Slot::One(v)) => to_datum(self.meta, f, v),
            Some(Slot::Ref(Some(x))) => Datum::Obj(x.0),
            Some(Slot::Refs(v)) => v.first().map_or(Datum::Null, |x| Datum::Obj(x.0)),
            Some(_) => Datum::Null,
        }
    }

    pub fn len(&self, o: u32, f: FeatureId) -> usize {
        match self.slot(o, f) {
            Some(Slot::Refs(v)) => v.len(),
            Some(Slot::Many(v)) => v.len(),
            Some(Slot::Ref(Some(_))) if self.meta.feature(f).is_many() => 1,
            _ => 0,
        }
    }

    pub fn get_at(&self, o: u32, f: FeatureId, i: usize) -> Datum<'x, u32> {
        match self.slot(o, f) {
            Some(Slot::Refs(v)) => v.get(i).map_or(Datum::Null, |x| Datum::Obj(x.0)),
            Some(Slot::Many(v)) => v.get(i).map_or(Datum::Null, |x| to_datum(self.meta, f, x)),
            Some(Slot::Ref(Some(x))) if i == 0 && self.meta.feature(f).is_many() => {
                Datum::Obj(x.0)
            }
            _ => Datum::Null,
        }
    }

    pub fn is_set(&self, o: u32, f: FeatureId) -> bool {
        let fi = self.meta.feature(f);
        if self.meta.is_container(f) {
            return matches!(self.container(o), Some((_, cf)) if Some(cf) == fi.opposite);
        }
        match self.slot(o, f) {
            None => false,
            Some(Slot::Refs(v)) => fi.unsettable || !v.is_empty(),
            Some(Slot::Many(v)) => fi.unsettable || !v.is_empty(),
            Some(Slot::Ref(r)) => r.is_some() || fi.unsettable,
            Some(Slot::One(Value::Null)) => true,
            Some(Slot::One(v)) => {
                fi.unsettable
                    || !to_datum(self.meta, f, v).java_equals(&self.default_datum(f))
            }
        }
    }

    pub fn index_of(&self, o: u32, f: FeatureId, v: &Datum<'_, u32>) -> Option<usize> {
        (0..self.len(o, f)).find(|&i| self.get_at(o, f, i).java_equals(v))
    }

    /// `EcoreUtil.isAncestor(ancestor, o)`.
    pub fn is_ancestor(&self, ancestor: u32, o: u32) -> bool {
        let mut cur = Some(o);
        let mut steps = 0;
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.container(c).map(|(p, _)| p);
            steps += 1;
            if steps > 1_000_000 {
                return false;
            }
        }
        false
    }

    /// The resource whose root list holds the object (a resource of the graph or one created
    /// through the store).
    pub fn direct_resource(&self, o: u32) -> Option<ResourceId> {
        if let Some(&r) = self.st.mem_root.get(&o) {
            return Some(r);
        }
        let home = self.g.objs[o as usize].resource.0 as usize;
        let res = self.g.resources.get(home)?;
        if res.roots.contains(&rid(o)) {
            self.st.raw_res.get(home).copied().flatten()
        } else {
            None
        }
    }

}

impl Raw<'_> {
    /// The reading view of the graph.
    pub fn v(&self) -> RawView<'_> {
        RawView {
            g: self.g,
            meta: self.meta,
            ext: self.ext,
            st: self.st,
            ecore: self.ecore,
        }
    }

    // ---- checks -------------------------------------------------------------------------------

    fn check_value(&self, f: FeatureId, v: &Datum<'_, u32>) -> ModelResult<()> {
        if !accepts(self.meta, f, v, |x| self.v().class_of_handle(x)) {
            return Err(ModelError::TypeMismatch {
                feature: f,
                found: describe(self.meta, v, |x| self.v().class_of_handle(x)),
            });
        }
        if let Datum::Obj(x) = v
            && *x >= EXT_BASE
        {
            return Err(ModelError::Unsupported(format!(
                "an object of another metamodel ({}) cannot be stored in SimOxide's graph",
                self.meta.qualified_name(self.v().class_of_handle(*x))
            )));
        }
        Ok(())
    }

    fn require_many(&self, f: FeatureId) -> ModelResult<()> {
        if self.meta.feature(f).is_many() {
            Ok(())
        } else {
            Err(ModelError::WrongMultiplicity(f))
        }
    }

    // ---- recorded storage primitives ------------------------------------------------------------

    fn record(&mut self, o: u32, k: ChangeKind) {
        if let Some(j) = self.st.journal.as_mut() {
            j.push(o, k);
        }
    }

    fn recording(&self) -> bool {
        self.st.journal.is_some()
    }

    fn slot_pos(&self, o: u32, f: FeatureId) -> Option<usize> {
        self.g.objs[o as usize]
            .slots
            .iter()
            .position(|(g, _)| g.0 == f.0)
    }

    /// Writes a single-valued slot (`None`: unset) and records the change.
    fn write_single(&mut self, o: u32, f: FeatureId, s: Option<Slot>, unset: bool) {
        let old = self.recording().then(|| self.v().get(o, f).to_owned());
        let pos = self.slot_pos(o, f);
        let slots = &mut self.g.objs[o as usize].slots;
        match (pos, s) {
            (Some(i), Some(s)) => slots[i].1 = s,
            (None, Some(s)) => slots.push((sf(f), s)),
            (Some(i), None) => {
                slots.remove(i);
            }
            (None, None) => {}
        }
        if let Some(old) = old {
            let k = if unset {
                ChangeKind::Unset { feature: f, old }
            } else {
                ChangeKind::Set {
                    feature: f,
                    old,
                    new: self.v().get(o, f).to_owned(),
                }
            };
            self.record(o, k);
        }
    }

    /// The list of a many-valued reference, created when absent.
    fn refs_mut(&mut self, o: u32, f: FeatureId) -> &mut Vec<RawId> {
        let i = match self.slot_pos(o, f) {
            Some(i) => i,
            None => {
                self.g.objs[o as usize]
                    .slots
                    .push((sf(f), Slot::Refs(Vec::new())));
                self.g.objs[o as usize].slots.len() - 1
            }
        };
        let s = &mut self.g.objs[o as usize].slots[i].1;
        if !matches!(s, Slot::Refs(_)) {
            let v = match s {
                Slot::Ref(Some(x)) => vec![*x],
                _ => Vec::new(),
            };
            *s = Slot::Refs(v);
        }
        match s {
            Slot::Refs(v) => v,
            _ => unreachable!(),
        }
    }

    fn datas_mut(&mut self, o: u32, f: FeatureId) -> &mut Vec<Value> {
        let i = match self.slot_pos(o, f) {
            Some(i) => i,
            None => {
                self.g.objs[o as usize]
                    .slots
                    .push((sf(f), Slot::Many(Vec::new())));
                self.g.objs[o as usize].slots.len() - 1
            }
        };
        let s = &mut self.g.objs[o as usize].slots[i].1;
        if !matches!(s, Slot::Many(_)) {
            *s = Slot::Many(Vec::new());
        }
        match s {
            Slot::Many(v) => v,
            _ => unreachable!(),
        }
    }

    /// After a list change: unsettable lists stay present (set) until unset, others are
    /// present only while not empty (EMF's `eIsSet`).
    fn mark_set(&mut self, o: u32, f: FeatureId) {
        if self.meta.feature(f).unsettable {
            if self.slot_pos(o, f).is_none() {
                let s = if self.meta.feature(f).kind == FeatureKind::Reference {
                    Slot::Refs(Vec::new())
                } else {
                    Slot::Many(Vec::new())
                };
                self.g.objs[o as usize].slots.push((sf(f), s));
            }
        } else if self.v().len(o, f) == 0
            && let Some(i) = self.slot_pos(o, f)
        {
            self.g.objs[o as usize].slots.remove(i);
        }
    }

    fn list_insert(&mut self, o: u32, f: FeatureId, at: Option<usize>, v: &Datum<'_, u32>) -> usize {
        let i = match v {
            Datum::Obj(x) => {
                let l = self.refs_mut(o, f);
                let i = at.unwrap_or(l.len()).min(l.len());
                l.insert(i, rid(*x));
                i
            }
            d => {
                let val = to_value(self.meta, f, d);
                let l = self.datas_mut(o, f);
                let i = at.unwrap_or(l.len()).min(l.len());
                l.insert(i, val);
                i
            }
        };
        self.mark_set(o, f);
        if self.recording() {
            let value = v.to_owned();
            self.record(
                o,
                ChangeKind::Added {
                    feature: f,
                    index: i,
                    value,
                },
            );
        }
        i
    }

    fn list_remove(&mut self, o: u32, f: FeatureId, i: usize) -> OwnedDatum<u32> {
        let value = self.v().get_at(o, f, i).to_owned();
        if self.meta.feature(f).kind == FeatureKind::Reference {
            self.refs_mut(o, f).remove(i);
        } else {
            self.datas_mut(o, f).remove(i);
        }
        self.mark_set(o, f);
        if self.recording() {
            self.record(
                o,
                ChangeKind::Removed {
                    feature: f,
                    index: i,
                    value: value.clone(),
                },
            );
        }
        value
    }

    /// Removes the first occurrence of object `x` from `o.f` (an inverse update).
    fn list_remove_obj(&mut self, o: u32, f: FeatureId, x: u32) {
        if let Some(i) = self.v().refs(o, f).iter().position(|y| y.0 == x) {
            self.list_remove(o, f, i);
        }
    }

    fn set_container_raw(&mut self, o: u32, new: Option<(u32, FeatureId)>) {
        let old = self.v().container(o);
        self.g.objs[o as usize].container = new.map(|(p, f)| (rid(p), sf(f)));
        if old != new {
            self.record(o, ChangeKind::Container { old, new });
        }
    }

    // ---- roots ----------------------------------------------------------------------------------

    /// Removes `o` from the root list of the store resource `r` (a resource of the graph or one
    /// created through the store); `false` if it is no root there.
    pub fn root_remove(&mut self, r: ResourceId, o: u32) -> bool {
        let Some(i) = self.st.roots[r.index()].iter().position(|x| *x == o) else {
            return false;
        };
        self.st.roots[r.index()].remove(i);
        match self.st.res[r.index()] {
            Res::Raw(rr) => {
                let roots = &mut self.g.resources[rr as usize].roots;
                if let Some(j) = roots.iter().position(|x| x.0 == o) {
                    roots.remove(j);
                }
            }
            Res::Mem(_) => {
                self.st.mem_root.remove(&o);
            }
            Res::Ext(_) => unreachable!("graph objects are no roots of other resources"),
        }
        self.record(o, ChangeKind::RootRemoved { resource: r });
        true
    }

    /// `Resource.getContents().add(index, o)` with `eSetResource`'s effects.
    pub fn root_add(&mut self, r: ResourceId, o: u32, at: Option<usize>) -> ModelResult<bool> {
        let len = self.st.roots[r.index()].len();
        if let Some(i) = at
            && i > len
        {
            return Err(ModelError::IndexOutOfBounds { index: i, size: len });
        }
        if self.st.roots[r.index()].contains(&o) {
            return match at {
                None => Ok(false),
                Some(_) => Err(ModelError::Unsupported(
                    "The 'no duplicates' constraint is violated".into(),
                )),
            };
        }
        // eSetResource: out of the old root list, out of a non-resolving container
        if let Some(old) = self.v().direct_resource(o) {
            self.root_remove(old, o);
        }
        let i = at.unwrap_or(len);
        self.st.roots[r.index()].insert(i, o);
        match self.st.res[r.index()] {
            Res::Raw(rr) => {
                let roots = &mut self.g.resources[rr as usize].roots;
                // the graph's list has the same order (it holds only the graph's objects)
                roots.insert(i.min(roots.len()), rid(o));
                self.g.objs[o as usize].resource = raw::ResId(rr);
            }
            Res::Mem(_) => {
                self.st.mem_root.insert(o, r);
            }
            Res::Ext(_) => unreachable!(),
        }
        self.record(o, ChangeKind::RootAdded { resource: r, index: i });
        if let Some((_, f)) = self.v().container(o)
            && !self.meta.feature(f).resolve_proxies
        {
            self.remove_from_container(o);
            self.set_container_raw(o, None);
        }
        Ok(true)
    }

    // ---- EMF internals --------------------------------------------------------------------------

    /// `eBasicSetContainer`: a root that becomes contained through a non-resolving containment
    /// leaves its root list.
    fn basic_set_container(&mut self, child: u32, new: Option<(u32, FeatureId)>) {
        if let Some((_, f)) = new
            && !self.meta.feature(f).resolve_proxies
            && let Some(r) = self.v().direct_resource(child)
        {
            self.root_remove(r, child);
        }
        self.set_container_raw(child, new);
    }

    /// `eBasicRemoveFromContainer` (no inverse update).
    fn remove_from_container(&mut self, child: u32) {
        let Some((p, f)) = self.v().container(child) else {
            return;
        };
        if self.meta.feature(f).is_many() {
            self.list_remove_obj(p, f, child);
        } else {
            self.write_single(p, f, None, false);
        }
    }

    /// A containment whose opposite is no container reference (set on one side only).
    fn containment_with_plain_inverse(&self, f: FeatureId) -> bool {
        self.meta
            .feature(f)
            .opposite
            .is_some_and(|g| !self.meta.is_container(g))
    }

    /// The link `source.f -> target` was made: update `target`'s side (`eInverseAdd`).
    fn inverse_add(&mut self, target: u32, source: u32, f: FeatureId) {
        let fi = self.meta.feature(f);
        if fi.containment && !self.containment_with_plain_inverse(f) {
            if self.v().container(target).is_some() {
                self.remove_from_container(target);
            }
            self.basic_set_container(target, Some((source, f)));
        } else if let Some(g) = fi.opposite {
            if self.meta.feature(g).is_many() {
                self.list_insert(target, g, None, &Datum::Obj(source));
            } else {
                let old = self.v().single_ref(target, g);
                self.write_single(target, g, Some(Slot::Ref(Some(rid(source)))), false);
                if let Some(old) = old
                    && old != source
                {
                    self.inverse_remove(old, target, g);
                }
            }
        }
    }

    /// The link `source.f -> target` was cut: update `target`'s side (`eInverseRemove`).
    fn inverse_remove(&mut self, target: u32, source: u32, f: FeatureId) {
        let fi = self.meta.feature(f);
        if fi.containment && !self.containment_with_plain_inverse(f) {
            self.basic_set_container(target, None);
        } else if let Some(g) = fi.opposite {
            if self.meta.is_container(g) {
                return;
            }
            if self.meta.feature(g).is_many() {
                self.list_remove_obj(target, g, source);
            } else {
                self.write_single(target, g, None, false);
            }
        }
    }

    fn has_inverse(&self, f: FeatureId) -> bool {
        let fi = self.meta.feature(f);
        fi.containment || fi.opposite.is_some()
    }

    fn set_ref_single(&mut self, o: u32, f: FeatureId, new: Option<u32>) {
        let old = self.v().single_ref(o, f);
        if old != new && self.has_inverse(f) {
            if let Some(old) = old {
                self.inverse_remove(old, o, f);
            }
            if let Some(n) = new {
                self.inverse_add(n, o, f);
            }
        }
        let slot = match new {
            Some(x) => Some(Slot::Ref(Some(rid(x)))),
            None if self.meta.feature(f).unsettable => Some(Slot::Ref(None)),
            None => None,
        };
        self.write_single(o, f, slot, false);
    }

    fn set_container_feature(&mut self, o: u32, f: FeatureId, new: Option<u32>) -> ModelResult<()> {
        let g = self.meta.feature(f).opposite.expect("container reference");
        let cur = self.v().container(o);
        let same = match (cur, new) {
            (Some((p, cf)), Some(n)) => p == n && cf == g,
            (None, None) => true,
            _ => false,
        };
        if same {
            return Ok(());
        }
        if let Some(n) = new
            && self.v().is_ancestor(o, n)
        {
            return Err(ModelError::RecursiveContainment);
        }
        if cur.is_some() {
            self.remove_from_container(o);
        }
        if let Some(n) = new {
            if self.meta.feature(g).is_many() {
                self.list_insert(n, g, None, &Datum::Obj(o));
            } else {
                let old = self.v().single_ref(n, g);
                self.write_single(n, g, Some(Slot::Ref(Some(rid(o)))), false);
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

    // ---- operations -----------------------------------------------------------------------------

    pub fn create(&mut self, c: ClassId, home: u32) -> u32 {
        let o = self.g.add_object(sm::ClassId(c.0), raw::ResId(home)).0;
        assert!(o < EXT_BASE, "SimOxide graph too large for the store's handles");
        self.record(o, ChangeKind::Created(c));
        o
    }

    pub fn set(&mut self, o: u32, f: FeatureId, v: Datum<'_, u32>) -> ModelResult<()> {
        self.v().has_feature(o, f)?;
        let fi = self.meta.feature(f);
        if !fi.changeable {
            return Err(ModelError::NotChangeable(f));
        }
        if fi.is_many() {
            return Err(ModelError::WrongMultiplicity(f));
        }
        self.check_value(f, &v)?;
        if fi.kind == FeatureKind::Reference {
            let new = v.as_obj();
            if self.meta.is_container(f) {
                return self.set_container_feature(o, f, new);
            }
            self.set_ref_single(o, f, new);
            return Ok(());
        }
        let slot = if v.is_null() {
            let intrinsic = intrinsic_default(self.meta, fi.ty);
            if !intrinsic.as_datum().is_null() {
                None
            } else if fi.unsettable || !self.v().default_datum(f).is_null() {
                Some(Slot::One(Value::Null))
            } else {
                None
            }
        } else if !fi.unsettable && v.java_equals(&self.v().default_datum(f)) {
            // equal to the default: not set (EMF's `eIsSet`)
            None
        } else {
            Some(Slot::One(to_value(self.meta, f, &v)))
        };
        self.write_single(o, f, slot, false);
        Ok(())
    }

    pub fn unset(&mut self, o: u32, f: FeatureId) -> ModelResult<()> {
        self.v().has_feature(o, f)?;
        let fi = self.meta.feature(f);
        if !fi.changeable {
            return Err(ModelError::NotChangeable(f));
        }
        if fi.is_many() {
            self.clear(o, f)?;
            if let Some(i) = self.slot_pos(o, f) {
                self.g.objs[o as usize].slots.remove(i);
            }
            return Ok(());
        }
        if fi.kind == FeatureKind::Reference {
            if self.meta.is_container(f) {
                if self.v().container(o).is_some() {
                    self.remove_from_container(o);
                    self.basic_set_container(o, None);
                }
                return Ok(());
            }
            if let Some(old) = self.v().single_ref(o, f)
                && self.has_inverse(f)
            {
                self.inverse_remove(old, o, f);
            }
        }
        self.write_single(o, f, None, true);
        Ok(())
    }

    pub fn add(
        &mut self,
        o: u32,
        f: FeatureId,
        v: Datum<'_, u32>,
        at: Option<usize>,
    ) -> ModelResult<bool> {
        self.v().has_feature(o, f)?;
        self.require_many(f)?;
        let fi = self.meta.feature(f);
        let (unique, is_ref) = (fi.unique, fi.kind == FeatureKind::Reference);
        let len = self.v().len(o, f);
        if let Some(i) = at
            && i > len
        {
            return Err(ModelError::IndexOutOfBounds { index: i, size: len });
        }
        if unique && self.v().index_of(o, f, &v).is_some() {
            return match at {
                None => Ok(false),
                Some(_) => Err(ModelError::Duplicate(f)),
            };
        }
        if is_ref && v.is_null() {
            return Err(ModelError::NullNotAllowed(f));
        }
        self.check_value(f, &v)?;
        self.list_insert(o, f, at, &v);
        if let Datum::Obj(x) = v
            && self.has_inverse(f)
        {
            self.inverse_add(x, o, f);
        }
        Ok(true)
    }

    pub fn remove(&mut self, o: u32, f: FeatureId, v: Datum<'_, u32>) -> ModelResult<bool> {
        self.v().has_feature(o, f)?;
        self.require_many(f)?;
        match self.v().index_of(o, f, &v) {
            Some(i) => {
                self.remove_at(o, f, i)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn remove_at(&mut self, o: u32, f: FeatureId, i: usize) -> ModelResult<()> {
        self.v().has_feature(o, f)?;
        self.require_many(f)?;
        let len = self.v().len(o, f);
        if i >= len {
            return Err(ModelError::IndexOutOfBounds { index: i, size: len });
        }
        let x = self.list_remove(o, f, i);
        if let OwnedDatum::Obj(x) = x
            && self.has_inverse(f)
        {
            self.inverse_remove(x, o, f);
        }
        Ok(())
    }

    pub fn set_at(&mut self, o: u32, f: FeatureId, i: usize, v: Datum<'_, u32>) -> ModelResult<()> {
        self.v().has_feature(o, f)?;
        self.require_many(f)?;
        let fi = self.meta.feature(f);
        let (unique, is_ref) = (fi.unique, fi.kind == FeatureKind::Reference);
        let len = self.v().len(o, f);
        if i >= len {
            return Err(ModelError::IndexOutOfBounds { index: i, size: len });
        }
        if unique
            && let Some(j) = self.v().index_of(o, f, &v)
            && j != i
        {
            return Err(ModelError::Duplicate(f));
        }
        if is_ref && v.is_null() {
            return Err(ModelError::NullNotAllowed(f));
        }
        self.check_value(f, &v)?;
        let old = self.v().get_at(o, f, i).to_owned();
        match v {
            Datum::Obj(x) => self.refs_mut(o, f)[i] = rid(x),
            ref d => {
                let val = to_value(self.meta, f, d);
                self.datas_mut(o, f)[i] = val;
            }
        }
        self.mark_set(o, f);
        if self.recording() {
            let new = v.to_owned();
            self.record(
                o,
                ChangeKind::Replaced {
                    feature: f,
                    index: i,
                    old: old.clone(),
                    new,
                },
            );
        }
        if let (OwnedDatum::Obj(old), Datum::Obj(x)) = (old, v)
            && old != x
            && self.has_inverse(f)
        {
            self.inverse_remove(old, o, f);
            self.inverse_add(x, o, f);
        }
        Ok(())
    }

    pub fn clear(&mut self, o: u32, f: FeatureId) -> ModelResult<()> {
        self.v().has_feature(o, f)?;
        self.require_many(f)?;
        let inverse = self.has_inverse(f);
        let mut n = self.v().len(o, f);
        // from the end, so that the recorded indexes stay valid when replayed
        let mut removed = Vec::new();
        while n > 0 {
            n -= 1;
            removed.push(self.list_remove(o, f, n));
        }
        self.mark_set(o, f);
        if inverse {
            for x in removed.into_iter().rev() {
                if let OwnedDatum::Obj(x) = x {
                    self.inverse_remove(x, o, f);
                }
            }
        }
        Ok(())
    }

    pub fn move_to(&mut self, o: u32, f: FeatureId, to: usize, from: usize) -> ModelResult<()> {
        self.v().has_feature(o, f)?;
        self.require_many(f)?;
        let len = self.v().len(o, f);
        if to >= len {
            return Err(ModelError::IndexOutOfBounds { index: to, size: len });
        }
        if from >= len {
            return Err(ModelError::IndexOutOfBounds { index: from, size: len });
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
        self.record(o, ChangeKind::Moved { feature: f, from, to });
        Ok(())
    }
}

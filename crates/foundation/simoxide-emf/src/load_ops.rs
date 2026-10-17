//! Loader-level writes and plain reads of a [`Graph`], for readers with their own XMI loading
//! rules (SimOxide's PCM loader, `simoxide-model`, which reproduces EMF 2.33's `XMIResourceImpl`
//! and has its own goldens) and for hosts that read models without the `ModelStore` protocol.
//!
//! The writes are what `XMLHelperImpl` does while a document is read: they keep containment and
//! opposites consistent (as SimOxide's former `raw::Graph` did) but check nothing, never touch
//! root lists of other resources, and are not recorded in the journal. A feature written by them
//! counts as *stored* ([`Graph::is_stored`]), also when the value equals the default or a list is
//! empty (EMF's `eIsSet` is [`ModelStore::is_set`]).

use crate::graph::{Graph, ObjId, Slot, Value};
use qvto_model::{ClassId, FeatureId, ResourceId};

impl Graph {
    // ---- reading ------------------------------------------------------------------------------

    /// The objects (index = `ObjId`).
    pub fn objects(&self) -> &[crate::graph::Object] {
        &self.objs
    }

    /// The resources (index = `ResourceId`).
    pub fn resources(&self) -> &[crate::graph::Resource] {
        &self.resources
    }

    /// Whether `f` holds a written value or list (not unset); see the module documentation.
    #[inline]
    pub fn is_stored(&self, o: ObjId, f: FeatureId) -> bool {
        !matches!(self.slot(o, f), None | Some(Slot::Unset))
    }

    /// The stored value of a single-valued attribute (`None`: unset, the default applies).
    #[inline]
    pub fn stored(&self, o: ObjId, f: FeatureId) -> Option<&Value> {
        match self.slot(o, f) {
            Some(Slot::Data(v)) => Some(v),
            _ => None,
        }
    }

    /// The values of a many-valued attribute.
    #[inline]
    pub fn stored_list(&self, o: ObjId, f: FeatureId) -> &[Value] {
        match self.slot(o, f) {
            Some(Slot::Datas(v, _)) => v,
            _ => &[],
        }
    }

    /// The target of a single-valued reference (container references come from the tree; the
    /// first element of a many-valued one).
    #[inline]
    pub fn get_ref(&self, o: ObjId, f: FeatureId) -> Option<ObjId> {
        if self.meta.is_container(f) {
            let opp = self.meta.feature(f).opposite;
            return self.objs[o.index()]
                .container
                .filter(|(_, cf)| Some(*cf) == opp)
                .map(|(c, _)| c);
        }
        match self.slot(o, f) {
            Some(Slot::Ref(r)) => *r,
            Some(Slot::Refs(v, _)) => v.first().copied(),
            _ => None,
        }
    }

    /// The targets of a many-valued reference in list order (a single-valued one as a slice of
    /// at most one).
    #[inline]
    pub fn get_refs(&self, o: ObjId, f: FeatureId) -> &[ObjId] {
        match self.slot(o, f) {
            Some(Slot::Refs(v, _)) => v,
            Some(Slot::Ref(Some(r))) => std::slice::from_ref(r),
            _ => &[],
        }
    }

    /// The children of `o` in `eContents` order, with their containment feature.
    pub fn children(&self, o: ObjId) -> impl Iterator<Item = (FeatureId, ObjId)> + '_ {
        self.meta
            .class(self.objs[o.index()].class)
            .containments
            .iter()
            .flat_map(move |&f| self.get_refs(o, f).iter().map(move |&c| (f, c)))
    }

    /// All objects of resource `r` in tree order (`getAllContents`) appended to `out`, with a
    /// caller-provided stack.
    pub fn resource_contents_into(
        &self,
        r: ResourceId,
        out: &mut Vec<ObjId>,
        stack: &mut Vec<ObjId>,
    ) {
        stack.clear();
        stack.extend(self.resources[r.index()].roots.iter().rev().copied());
        while let Some(o) = stack.pop() {
            out.push(o);
            let class = self.objs[o.index()].class;
            for &f in self.meta.class(class).containments.iter().rev() {
                stack.extend(self.get_refs(o, f).iter().rev().copied());
            }
        }
    }

    /// The value of the object's ID attribute (`eIDAttribute`), if it is set: a string, or an
    /// enumeration literal's literal (e.g. a unit).
    pub fn id(&self, o: ObjId) -> Option<&str> {
        let f = self.meta.class(self.objs[o.index()].class).id_attribute?;
        match self.stored(o, f)? {
            Value::Str(s) | Value::Other(s) => Some(s),
            Value::Enum(e, i) => Some(&self.meta.enumeration(*e).literals[*i as usize].literal),
            _ => None,
        }
    }

    /// The EMF path fragment of `o` (`//@feature.i/@other`, `/i` for the `i`-th root).
    pub fn path_fragment(&self, o: ObjId) -> String {
        let mut segs = Vec::new();
        let mut cur = o;
        while let Some((p, f)) = self.objs[cur.index()].container {
            let fi = self.meta.feature(f);
            if fi.is_many() {
                let idx = self
                    .get_refs(p, f)
                    .iter()
                    .position(|c| *c == cur)
                    .unwrap_or(0);
                segs.push(format!("@{}.{}", fi.name, idx));
            } else {
                segs.push(format!("@{}", fi.name));
            }
            cur = p;
            if segs.len() > 1_000_000 {
                break;
            }
        }
        let ri = self.objs[cur.index()]
            .resource
            .and_then(|r| {
                self.resources[r.index()]
                    .roots
                    .iter()
                    .position(|x| *x == cur)
            })
            .unwrap_or(0);
        let mut s = String::from("/");
        if ri != 0 {
            s.push_str(&ri.to_string());
        }
        for seg in segs.iter().rev() {
            s.push('/');
            s.push_str(seg);
        }
        s
    }

    // ---- loader-level writes --------------------------------------------------------------------

    /// A new object of class `c` (abstract classes too: a proxy may have any class), without
    /// values, container or resource; not recorded.
    pub fn load_object(&mut self, c: ClassId) -> ObjId {
        self.new_object(c)
    }

    /// Sets the source line of an object.
    pub fn set_line(&mut self, o: ObjId, line: u32) {
        self.objs[o.index()].line = line;
    }

    /// Appends `o` to the roots of `r` (a document's top-level element).
    pub fn load_add_root(&mut self, r: ResourceId, o: ObjId) {
        self.resources[r.index()].roots.push(o);
        self.objs[o.index()].resource = Some(r);
    }

    /// Stores the value of a single-valued attribute.
    pub fn load_set_attr(&mut self, o: ObjId, f: FeatureId, v: Value) {
        self.load_slot(o, f, Slot::Data(v));
    }

    /// Appends a value to a many-valued attribute.
    pub fn load_add_attr(&mut self, o: ObjId, f: FeatureId, v: Value) {
        match self.load_slot_mut(o, f) {
            Slot::Datas(l, _) => l.push(v),
            s => *s = Slot::Datas(vec![v], true),
        }
    }

    /// Stores an empty list in a many-valued attribute.
    pub fn load_clear_attrs(&mut self, o: ObjId, f: FeatureId) {
        self.load_slot(o, f, Slot::Datas(Vec::new(), true));
    }

    /// The list of a many-valued reference, stored (empty) if it was not (no inverse updates).
    pub fn load_refs_mut(&mut self, o: ObjId, f: FeatureId) -> &mut Vec<ObjId> {
        let s = self.load_slot_mut(o, f);
        if !matches!(s, Slot::Refs(..)) {
            *s = Slot::Refs(Vec::new(), true);
        }
        match s {
            Slot::Refs(v, _) => v,
            _ => unreachable!(),
        }
    }

    /// `eSet` of a single-valued reference while loading, maintaining the opposite (or the
    /// container of a contained object). Container references are not written.
    pub fn load_set_ref(&mut self, o: ObjId, f: FeatureId, v: Option<ObjId>) {
        let old = self.get_ref(o, f);
        if self.meta.is_container(f) {
            return;
        }
        if old == v && self.is_stored(o, f) {
            return;
        }
        let fi = self.meta.feature(f);
        if let Some(opp) = fi.opposite {
            if old != v {
                if let Some(old) = old {
                    self.load_inverse_remove(old, opp, o);
                }
                if let Some(nv) = v {
                    self.load_inverse_add(nv, opp, o);
                }
            }
        } else if fi.containment {
            if let Some(old) = old
                && Some(old) != v
            {
                self.objs[old.index()].container = None;
            }
            if let Some(nv) = v {
                self.load_detach(nv);
                self.objs[nv.index()].container = Some((o, f));
            }
        }
        self.load_slot(o, f, Slot::Ref(v));
    }

    /// `InternalEList.addUnique(index, v)` on a many-valued reference while loading, maintaining
    /// the opposite. `None` appends; `false` if the index is out of bounds.
    pub fn load_add_ref(&mut self, o: ObjId, f: FeatureId, v: ObjId, index: Option<usize>) -> bool {
        let len = self.get_refs(o, f).len();
        let idx = index.unwrap_or(len);
        if idx > len {
            return false;
        }
        let fi = self.meta.feature(f);
        if fi.containment {
            self.load_detach(v);
            self.objs[v.index()].container = Some((o, f));
        } else if let Some(opp) = fi.opposite {
            self.load_inverse_add(v, opp, o);
        }
        self.load_refs_mut(o, f).insert(idx, v);
        true
    }

    /// `EList.move(to, from)` of a reference list while loading.
    pub fn load_move_ref(&mut self, o: ObjId, f: FeatureId, to: usize, from: usize) -> bool {
        let l = self.load_refs_mut(o, f);
        if to >= l.len() || from >= l.len() {
            return false;
        }
        let x = l.remove(from);
        l.insert(to, x);
        true
    }

    /// `list.clear()` of a reference list while loading, maintaining opposites.
    pub fn load_clear_refs(&mut self, o: ObjId, f: FeatureId) {
        let old = std::mem::take(self.load_refs_mut(o, f));
        let fi = self.meta.feature(f);
        let (containment, opp) = (fi.containment, fi.opposite);
        for x in old {
            if containment {
                self.objs[x.index()].container = None;
            } else if let Some(opp) = opp {
                self.load_inverse_remove(x, opp, o);
            }
        }
    }

    /// Replaces proxy `p` by `t` wherever `holder.f` refers to it (proxy resolution: no inverse
    /// update, like EMF's resolving lists).
    pub fn replace_target(&mut self, holder: ObjId, f: FeatureId, p: ObjId, t: ObjId) {
        let Some(i) = self.meta.slot(self.objs[holder.index()].class, f) else {
            return;
        };
        match &mut self.objs[holder.index()].slots[i] {
            Slot::Ref(r) if *r == Some(p) => *r = Some(t),
            Slot::Refs(v, _) => {
                for x in v.iter_mut() {
                    if *x == p {
                        *x = t;
                    }
                }
            }
            _ => {}
        }
    }

    // ---- helpers --------------------------------------------------------------------------------

    fn load_slot(&mut self, o: ObjId, f: FeatureId, s: Slot) {
        *self.load_slot_mut(o, f) = s;
    }

    fn load_slot_mut(&mut self, o: ObjId, f: FeatureId) -> &mut Slot {
        let i = self
            .meta
            .slot(self.objs[o.index()].class, f)
            .expect("feature of the object's class");
        &mut self.objs[o.index()].slots[i]
    }

    /// `eInverseRemove` while loading: `o.f` drops `other`.
    fn load_inverse_remove(&mut self, o: ObjId, f: FeatureId, other: ObjId) {
        if self.meta.is_container(f) {
            if self.objs[o.index()].container.map(|c| c.0) == Some(other) {
                self.objs[o.index()].container = None;
            }
            return;
        }
        if self.meta.feature(f).is_many() {
            let l = self.load_refs_mut(o, f);
            if let Some(i) = l.iter().position(|x| *x == other) {
                l.remove(i);
            }
        } else if let Some(i) = self.meta.slot(self.objs[o.index()].class, f)
            && let Slot::Ref(r) = &mut self.objs[o.index()].slots[i]
            && *r == Some(other)
        {
            *r = None;
        }
    }

    /// `eInverseAdd` while loading: `o.f` now points back to `other` (displacing a previous
    /// single value).
    fn load_inverse_add(&mut self, o: ObjId, f: FeatureId, other: ObjId) {
        let back = self
            .meta
            .feature(f)
            .opposite
            .expect("inverse add on a reference without opposite");
        if self.meta.is_container(f) {
            // containment: `o` becomes a child of `other`
            if let Some((old, of)) = self.objs[o.index()].container
                && (old != other || of != back)
            {
                self.load_inverse_remove(old, of, o);
            }
            self.objs[o.index()].container = Some((other, back));
            return;
        }
        if self.meta.feature(f).is_many() {
            self.load_refs_mut(o, f).push(other);
        } else {
            if let Some(old) = self.get_ref(o, f)
                && old != other
            {
                self.load_inverse_remove(old, back, o);
            }
            self.load_slot(o, f, Slot::Ref(Some(other)));
        }
    }

    /// Takes `c` out of its container's feature (the loader's `detach`).
    fn load_detach(&mut self, c: ObjId) {
        if let Some((p, pf)) = self.objs[c.index()].container.take() {
            if self.meta.feature(pf).is_many() {
                let l = self.load_refs_mut(p, pf);
                if let Some(i) = l.iter().position(|x| *x == c) {
                    l.remove(i);
                }
            } else {
                self.load_slot(p, pf, Slot::Ref(None));
            }
        }
    }
}

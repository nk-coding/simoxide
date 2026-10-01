//! Generic EMF-like object graph: every loaded object with its class, persisted attribute values
//! and references, exactly as EMF would hold them after loading. The typed model
//! ([`crate::model`]) is built from this graph; the canonical dump ([`crate::canon`]) is produced
//! from it and compared with a dump made by real EMF.

use crate::meta::{ClassId, DataKind, FeatureId, FeatureKind};
use std::fmt;
use std::path::PathBuf;

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct ObjId(pub u32);

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct ResId(pub u32);

/// An attribute value.
#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    Null,
    Str(Box<str>),
    Int(i64),
    Double(f64),
    Bool(bool),
    /// Index into the enum's literal list.
    Enum(u32),
    /// Lexical value of a data type the loader does not interpret.
    Other(Box<str>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) | Value::Other(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Double(d) => Some(*d),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_enum(&self) -> Option<u32> {
        match self {
            Value::Enum(e) => Some(*e),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum Slot {
    One(Value),
    Many(Vec<Value>),
    Ref(Option<ObjId>),
    Refs(Vec<ObjId>),
}

#[derive(Clone, Debug)]
pub struct Obj {
    pub class: ClassId,
    /// Resource the object was read from (for a proxy: the resource of the referencing object).
    pub resource: ResId,
    pub container: Option<(ObjId, FeatureId)>,
    /// Set for unresolved proxies: the proxy URI (`<resource key>#<fragment>`).
    pub proxy: Option<Box<str>>,
    pub xmi_id: Option<Box<str>>,
    /// Set feature values, in the order they were first set.
    pub slots: Vec<(FeatureId, Slot)>,
    /// 1-based source line of the start tag.
    pub line: u32,
    pub(crate) attached: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Resource {
    /// Normalised URI key (`file:/abs/path`, `bundled:<name>`).
    pub uri: String,
    /// Short name used in dumps and diagnostics.
    pub name: String,
    pub path: Option<PathBuf>,
    pub roots: Vec<ObjId>,
    /// Problems EMF would report as load errors (the reference would refuse the file).
    pub errors: Vec<String>,
    /// The resource could not be read or parsed at all.
    pub failed: bool,
}

/// The loaded object graph over all resources.
#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub objs: Vec<Obj>,
    pub resources: Vec<Resource>,
}

impl std::ops::Index<ObjId> for Graph {
    type Output = Obj;
    fn index(&self, i: ObjId) -> &Obj {
        &self.objs[i.0 as usize]
    }
}
impl std::ops::IndexMut<ObjId> for Graph {
    fn index_mut(&mut self, i: ObjId) -> &mut Obj {
        &mut self.objs[i.0 as usize]
    }
}

/// Effective default value of an attribute (EMF `defaultValueLiteral` or the type's default).
pub fn default_value(f: FeatureId) -> Value {
    default_value_ref(f).clone()
}

/// [`default_value`], computed once per feature.
fn default_value_ref(f: FeatureId) -> &'static Value {
    static D: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    &D.get_or_init(|| {
        (0..crate::meta::FEATURES.len())
            .map(|i| compute_default_value(FeatureId(i as u32)))
            .collect()
    })[f.0 as usize]
}

fn compute_default_value(f: FeatureId) -> Value {
    let d = f.def();
    let FeatureKind::Attribute { data } = d.kind else {
        return Value::Null;
    };
    if let Some(lit) = d.default.filter(|_| data != DataKind::Other)
        && let Some(v) = parse_value(data, lit)
    {
        return v;
    }
    match data {
        DataKind::Str | DataKind::Other => Value::Null,
        DataKind::Int | DataKind::Long => Value::Int(0),
        DataKind::Double => Value::Double(0.0),
        DataKind::Bool => Value::Bool(false),
        DataKind::Enum(_) => Value::Enum(0),
    }
}

/// Parses a lexical value like EMF's `EFactory.createFromString` does. `None` on failure.
pub fn parse_value(data: DataKind, s: &str) -> Option<Value> {
    Some(match data {
        DataKind::Str => Value::Str(s.into()),
        DataKind::Other => Value::Other(s.into()),
        DataKind::Int => Value::Int(parse_java_int(s, i32::MIN as i64, i32::MAX as i64)?),
        DataKind::Long => Value::Int(parse_java_int(s, i64::MIN, i64::MAX)?),
        DataKind::Double => Value::Double(parse_java_double(s)?),
        DataKind::Bool => {
            // EcoreFactoryImpl.booleanValueOf: case-insensitive "true"/"false", otherwise an error
            if s.eq_ignore_ascii_case("true") {
                Value::Bool(true)
            } else if s.eq_ignore_ascii_case("false") {
                Value::Bool(false)
            } else {
                return None;
            }
        }
        DataKind::Enum(e) => Value::Enum(e.literal_by_name(s)?),
    })
}

/// `Integer.valueOf` / `Long.valueOf`: optional sign, ASCII digits, no whitespace.
fn parse_java_int(s: &str, min: i64, max: i64) -> Option<i64> {
    let b = s.as_bytes();
    let (neg, digits) = match b.first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let v: i128 = digits.parse().ok()?;
    let v = if neg { -v } else { v };
    (v >= min as i128 && v <= max as i128).then_some(v as i64)
}

/// `Double.valueOf`: trims ASCII whitespace/control chars, accepts a trailing `d`/`f` type suffix,
/// `NaN`, `Infinity` and hexadecimal is not supported (never seen in PCM files).
pub fn parse_java_double(s: &str) -> Option<f64> {
    let t = s.trim_matches(|c: char| c <= ' ');
    let t = t.strip_suffix(['d', 'D', 'f', 'F']).unwrap_or(t);
    let (sign, body) = match t.as_bytes().first()? {
        b'-' => (-1.0, &t[1..]),
        b'+' => (1.0, &t[1..]),
        _ => (1.0, t),
    };
    match body {
        "NaN" => return Some(f64::NAN),
        "Infinity" => return Some(sign * f64::INFINITY),
        _ => {}
    }
    // Rust's grammar is a superset for the remaining forms except for words like "inf"
    if body.is_empty()
        || !body
            .bytes()
            .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        return None;
    }
    let v: f64 = body.parse().ok()?;
    Some(sign * v)
}

impl Graph {
    pub fn obj(&self, o: ObjId) -> &Obj {
        &self[o]
    }

    pub fn slot(&self, o: ObjId, f: FeatureId) -> Option<&Slot> {
        self[o].slots.iter().find(|(g, _)| *g == f).map(|(_, s)| s)
    }

    fn slot_mut(&mut self, o: ObjId, f: FeatureId) -> &mut Slot {
        let obj = &mut self[o];
        if let Some(i) = obj.slots.iter().position(|(g, _)| *g == f) {
            return &mut obj.slots[i].1;
        }
        let d = f.def();
        let s = match (d.kind, d.many) {
            (FeatureKind::Attribute { .. }, false) => Slot::One(default_value(f)),
            (FeatureKind::Attribute { .. }, true) => Slot::Many(Vec::new()),
            (FeatureKind::Reference { .. }, false) => Slot::Ref(None),
            (FeatureKind::Reference { .. }, true) => Slot::Refs(Vec::new()),
        };
        obj.slots.push((f, s));
        &mut obj.slots.last_mut().unwrap().1
    }

    /// Effective value of a single-valued attribute (default if unset).
    pub fn attr(&self, o: ObjId, f: FeatureId) -> Value {
        match self.slot(o, f) {
            Some(Slot::One(v)) => v.clone(),
            _ => default_value(f),
        }
    }

    /// Values of a many-valued attribute.
    pub fn attrs(&self, o: ObjId, f: FeatureId) -> &[Value] {
        match self.slot(o, f) {
            Some(Slot::Many(v)) => v,
            _ => &[],
        }
    }

    pub fn is_set(&self, o: ObjId, f: FeatureId) -> bool {
        self.slot(o, f).is_some()
    }

    pub fn str_attr(&self, o: ObjId, f: FeatureId) -> Option<Box<str>> {
        match self.attr(o, f) {
            Value::Str(s) | Value::Other(s) => Some(s),
            _ => None,
        }
    }

    /// Target of a single-valued reference (container references are derived from the tree).
    pub fn get_ref(&self, o: ObjId, f: FeatureId) -> Option<ObjId> {
        if f.is_container() {
            return self[o]
                .container
                .filter(|(_, cf)| Some(*cf) == f.opposite())
                .map(|(c, _)| c);
        }
        match self.slot(o, f) {
            Some(Slot::Ref(r)) => *r,
            Some(Slot::Refs(v)) => v.first().copied(),
            _ => None,
        }
    }

    /// Targets of a many-valued reference, in list order.
    pub fn get_refs(&self, o: ObjId, f: FeatureId) -> &[ObjId] {
        match self.slot(o, f) {
            Some(Slot::Refs(v)) => v,
            Some(Slot::Ref(Some(r))) => std::slice::from_ref(r),
            _ => &[],
        }
    }

    /// Children in EMF `eContents` order.
    pub fn children(&self, o: ObjId) -> impl Iterator<Item = (FeatureId, ObjId)> + '_ {
        self[o]
            .class
            .containments()
            .flat_map(move |f| self.get_refs(o, f).iter().map(move |c| (f, *c)))
    }

    /// All objects of a resource in EMF tree order (`getAllContents`).
    pub fn all_contents(&self, r: ResId) -> Vec<ObjId> {
        let mut out = Vec::new();
        self.all_contents_into(r, &mut out, &mut Vec::new());
        out
    }

    /// [`Graph::all_contents`] appended to `out`, with a caller-provided stack buffer.
    pub(crate) fn all_contents_into(&self, r: ResId, out: &mut Vec<ObjId>, stack: &mut Vec<ObjId>) {
        stack.clear();
        stack.extend(self.resources[r.0 as usize].roots.iter().rev().copied());
        while let Some(o) = stack.pop() {
            out.push(o);
            let obj = &self[o];
            if obj.slots.is_empty() {
                continue;
            }
            for &f in obj.class.containment_features().iter().rev() {
                stack.extend(self.get_refs(o, f).iter().rev().copied());
            }
        }
    }

    /// EMF ID (value of the class's ID attribute), if set.
    pub fn id(&self, o: ObjId) -> Option<&str> {
        let f = self[o].class.def().id_attribute?;
        match self.slot(o, f)? {
            Slot::One(Value::Str(s)) | Slot::One(Value::Other(s)) => Some(s),
            Slot::One(Value::Enum(e)) => {
                // e.g. units BaseUnit: the enum literal is the id
                let crate::meta::DataKind::Enum(en) = f.data_kind()? else {
                    return None;
                };
                Some(en.def().literals[*e as usize].literal)
            }
            _ => None,
        }
    }

    /// `true` for an object of a PCM `Identifier` class without an `id` in the file: PCM's
    /// `IdentifierImpl` then invents a random UUID-based ID at load time.
    pub fn has_generated_id(&self, o: ObjId) -> bool {
        self[o].class.def().id_attribute == Some(crate::meta::feat::identifier_Identifier_id)
            && self.id(o).is_none()
            && self[o].proxy.is_none()
    }

    /// Fragment path `//@feature.i/@feature2` (or `/i` roots) as EMF computes it.
    pub fn path_fragment(&self, o: ObjId) -> String {
        let mut segs = Vec::new();
        let mut cur = o;
        while let Some((p, f)) = self[cur].container {
            if f.def().many {
                let idx = self
                    .get_refs(p, f)
                    .iter()
                    .position(|c| *c == cur)
                    .unwrap_or(0);
                segs.push(format!("@{}.{}", f.name(), idx));
            } else {
                segs.push(format!("@{}", f.name()));
            }
            cur = p;
        }
        let roots = &self.resources[self[cur].resource.0 as usize].roots;
        let ri = roots.iter().position(|r| *r == cur).unwrap_or(0);
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

    /// EMF `getURIFragment`: the ID if the object has one, else the path.
    pub fn fragment(&self, o: ObjId) -> String {
        if let Some(x) = &self[o].xmi_id {
            return x.to_string();
        }
        match self.id(o) {
            Some(id) => id.to_string(),
            None => self.path_fragment(o),
        }
    }

    /// `resource-name#fragment`, or `?proxy-uri` for an unresolved proxy.
    pub fn describe_ref(&self, o: ObjId) -> String {
        if let Some(p) = &self[o].proxy {
            return format!("?{p}");
        }
        format!(
            "{}#{}",
            self.resources[self[o].resource.0 as usize].name,
            self.fragment(o)
        )
    }

    /// Human-readable description for diagnostics: `name#fragment (pkg:Class "entityName")`.
    pub fn describe(&self, o: ObjId) -> String {
        let obj = &self[o];
        let mut s = format!("{} ({}", self.describe_ref(o), obj.class.qualified_name());
        if let Some(f) = obj.class.feature("entityName")
            && let Some(n) = self.str_attr(o, f)
        {
            s.push_str(&format!(" {n:?}"));
        }
        s.push(')');
        s
    }

    // ----- mutation with EMF semantics -------------------------------------------------------

    pub(crate) fn set_attr(&mut self, o: ObjId, f: FeatureId, v: Value) {
        let obj = &mut self[o];
        match obj.slots.iter_mut().find(|(g, _)| *g == f) {
            Some((_, s)) => *s = Slot::One(v),
            None => obj.slots.push((f, Slot::One(v))),
        }
    }

    pub(crate) fn add_attr(&mut self, o: ObjId, f: FeatureId, v: Value) {
        if let Slot::Many(l) = self.slot_mut(o, f) {
            l.push(v);
        }
    }

    pub(crate) fn clear_attrs(&mut self, o: ObjId, f: FeatureId) {
        *self.slot_mut(o, f) = Slot::Many(Vec::new());
    }

    /// Raw list access without inverse handling.
    fn refs_mut(&mut self, o: ObjId, f: FeatureId) -> &mut Vec<ObjId> {
        match self.slot_mut(o, f) {
            Slot::Refs(v) => v,
            s => {
                *s = Slot::Refs(Vec::new());
                let Slot::Refs(v) = s else { unreachable!() };
                v
            }
        }
    }

    /// `eInverseRemove`: drop `other` from `o.f` (the opposite end of a link being cut).
    fn inverse_remove(&mut self, o: ObjId, f: FeatureId, other: ObjId) {
        if f.is_container() {
            if self[o].container.map(|c| c.0) == Some(other) {
                self[o].container = None;
            }
            return;
        }
        if f.def().many {
            let l = self.refs_mut(o, f);
            if let Some(i) = l.iter().position(|x| *x == other) {
                l.remove(i);
            }
        } else if let Some(Slot::Ref(r)) = self[o]
            .slots
            .iter_mut()
            .find(|(g, _)| *g == f)
            .map(|(_, s)| s)
            && *r == Some(other)
        {
            *r = None;
        }
    }

    /// `eInverseAdd`: `o.f` now points back to `other` (displacing a previous single value).
    fn inverse_add(&mut self, o: ObjId, f: FeatureId, other: ObjId) {
        let back = f
            .opposite()
            .expect("inverse_add on reference without opposite");
        if f.is_container() {
            // containment: `o` becomes a child of `other`
            if let Some((old, of)) = self[o].container
                && (old != other || of != back)
            {
                self.inverse_remove(old, of, o);
            }
            self[o].container = Some((other, back));
            return;
        }
        if f.def().many {
            self.refs_mut(o, f).push(other);
        } else {
            let old = self.get_ref(o, f);
            if let Some(old) = old
                && old != other
            {
                self.inverse_remove(old, back, o);
            }
            *self.slot_mut(o, f) = Slot::Ref(Some(other));
        }
    }

    /// `eSet` of a single-valued reference, maintaining the opposite.
    pub(crate) fn set_ref(&mut self, o: ObjId, f: FeatureId, v: Option<ObjId>) {
        let old = self.get_ref(o, f);
        if f.is_container() {
            // setting the container side is not done by the loader
            return;
        }
        if old == v && self.is_set(o, f) {
            return;
        }
        if let Some(opp) = f.opposite() {
            if old != v {
                if let Some(old) = old {
                    self.inverse_remove(old, opp, o);
                }
                if let Some(nv) = v {
                    self.inverse_add(nv, opp, o);
                }
            }
        } else if f.is_containment() {
            if let Some(old) = old
                && old != v.unwrap_or(ObjId(u32::MAX))
            {
                self[old].container = None;
            }
            if let Some(nv) = v {
                self.detach(nv);
                self[nv].container = Some((o, f));
            }
        }
        *self.slot_mut(o, f) = Slot::Ref(v);
    }

    fn detach(&mut self, c: ObjId) {
        if let Some((p, pf)) = self[c].container.take() {
            if pf.def().many {
                let l = self.refs_mut(p, pf);
                if let Some(i) = l.iter().position(|x| *x == c) {
                    l.remove(i);
                }
            } else {
                *self.slot_mut(p, pf) = Slot::Ref(None);
            }
        }
    }

    /// `InternalEList.addUnique(index, value)` on a many-valued reference, maintaining the
    /// opposite. `index == None` appends. Returns `false` if the index is out of bounds.
    pub(crate) fn add_ref_unique(
        &mut self,
        o: ObjId,
        f: FeatureId,
        v: ObjId,
        index: Option<usize>,
    ) -> bool {
        let len = self.get_refs(o, f).len();
        let idx = index.unwrap_or(len);
        if idx > len {
            return false;
        }
        if f.is_containment() {
            self.detach(v);
            self[v].container = Some((o, f));
        } else if let Some(opp) = f.opposite() {
            self.inverse_add(v, opp, o);
        }
        self.refs_mut(o, f).insert(idx, v);
        true
    }

    /// `EList.move(to, from)`.
    pub(crate) fn move_ref(&mut self, o: ObjId, f: FeatureId, to: usize, from: usize) -> bool {
        let l = self.refs_mut(o, f);
        if to >= l.len() || from >= l.len() {
            return false;
        }
        let x = l.remove(from);
        l.insert(to, x);
        true
    }

    /// `list.clear()` of a many-valued reference, maintaining opposites.
    pub(crate) fn clear_refs(&mut self, o: ObjId, f: FeatureId) {
        let old: Vec<ObjId> = std::mem::take(self.refs_mut(o, f));
        for x in old {
            if f.is_containment() {
                self[x].container = None;
            } else if let Some(opp) = f.opposite() {
                self.inverse_remove(x, opp, o);
            }
        }
    }

    /// Replaces proxy `p` by `t` wherever `holder.f` refers to it (proxy resolution: no inverse
    /// update, like EMF's resolving lists).
    pub(crate) fn replace_target(&mut self, holder: ObjId, f: FeatureId, p: ObjId, t: ObjId) {
        if let Some((_, s)) = self[holder].slots.iter_mut().find(|(g, _)| *g == f) {
            match s {
                Slot::Ref(r) if *r == Some(p) => *r = Some(t),
                Slot::Refs(v) => {
                    for x in v.iter_mut() {
                        if *x == p {
                            *x = t;
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

impl fmt::Display for ObjId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

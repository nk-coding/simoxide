//! The object graph of a loaded model: the EMF graph shared with the QVTo engine
//! ([`simoxide_emf::Graph`], one graph for PCM, PRM, SPD and metamodel objects), with every loaded
//! object, its class, persisted attribute values and references exactly as EMF would hold them
//! after loading. The typed model ([`crate::model`]) is built from it; the canonical dump
//! ([`crate::canon`]) is produced from it and compared with a dump made by real EMF.
//!
//! The loader stores attribute values as the graph's values with the Java box of the
//! attribute's data type (`Int`, `Long`, `Double`, ...: what the QVTo engine reads). SimOxide's
//! own readers see them as [`Value`] through [`PcmGraph`] (integral kinds widened to `i64`,
//! unset attributes with the defaults of the static tables).

use crate::meta::prelude::*;
use crate::meta::{self, ClassId, DataKind, FeatureId, FeatureKind};
use simoxide_emf_core::{Classifier, DataKind as Kind, MetaRegistry, ModelStore};
use std::sync::OnceLock;

pub use simoxide_emf::{Graph, ObjId, Object as Obj, Resource};
/// A resource of the graph.
pub use simoxide_emf_core::ResourceId as ResId;

/// An empty graph whose registry holds SimOxide's metamodels (the static tables, with their
/// ids) and Ecore: where every load starts. Cheap (the registry is shared).
pub fn pcm_graph() -> Graph {
    template().empty_like()
}

fn template() -> &'static Graph {
    static T: OnceLock<Graph> = OnceLock::new();
    T.get_or_init(|| Graph::with_registry(MetaRegistry::from_static(&meta::SEED)))
}

/// An attribute value as SimOxide's readers see it.
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

/// The Java kind of the values of each feature of the static tables (`None`: no data type).
fn kinds() -> &'static [Option<Kind>] {
    static K: OnceLock<Vec<Option<Kind>>> = OnceLock::new();
    K.get_or_init(|| {
        let m = template().meta();
        (0..meta::FEATURES.len())
            .map(|i| match m.feature(FeatureId(i as u32)).ty {
                Classifier::DataType(d) => Some(m.data_type(d).kind),
                _ => None,
            })
            .collect()
    })
}

/// The graph value of a loaded attribute value of feature `f` (the box of its data type).
pub fn to_stored(f: FeatureId, v: Value) -> simoxide_emf::Value {
    use simoxide_emf::Value as V;
    let k = kinds().get(f.0 as usize).copied().flatten();
    match v {
        Value::Null => V::Null,
        Value::Bool(b) => V::Bool(b),
        Value::Str(s) => V::Str(s),
        Value::Other(s) => match k {
            Some(Kind::String) => V::Str(s),
            Some(Kind::Char) => V::Char(s.encode_utf16().next().unwrap_or(0)),
            _ => V::Other(s),
        },
        Value::Int(i) => match k {
            Some(Kind::Long) => V::Long(i),
            Some(Kind::Short) => V::Short(i as i16),
            Some(Kind::Byte) => V::Byte(i as i8),
            Some(Kind::Double) => V::Double(i as f64),
            Some(Kind::Float) => V::Float(i as f32),
            _ => V::Int(i as i32),
        },
        Value::Double(d) => match k {
            Some(Kind::Float) => V::Float(d as f32),
            _ => V::Double(d),
        },
        Value::Enum(i) => match f.def().ty {
            meta::TypeRef::Enum(e) => V::Enum(e, i),
            _ => V::Int(i as i32),
        },
    }
}

/// A graph value as SimOxide's readers see it.
pub fn from_stored(v: &simoxide_emf::Value) -> Value {
    use simoxide_emf::Value as V;
    match v {
        V::Null | V::Obj(_) => Value::Null,
        V::Bool(b) => Value::Bool(*b),
        V::Int(i) => Value::Int(*i as i64),
        V::Long(i) => Value::Int(*i),
        V::Short(i) => Value::Int(*i as i64),
        V::Byte(i) => Value::Int(*i as i64),
        V::Char(c) => Value::Other(String::from_utf16_lossy(&[*c]).into()),
        V::Double(x) => Value::Double(*x),
        V::Float(x) => Value::Double(*x as f64),
        V::Str(s) => Value::Str(s.clone()),
        V::Other(s) => Value::Other(s.clone()),
        V::Enum(_, i) => Value::Enum(*i),
    }
}

/// Effective default value of an attribute (EMF `defaultValueLiteral` or the type's default).
pub fn default_value(f: FeatureId) -> Value {
    default_value_ref(f).clone()
}

/// [`default_value`], computed once per feature.
fn default_value_ref(f: FeatureId) -> &'static Value {
    static D: OnceLock<Vec<Value>> = OnceLock::new();
    static NULL: Value = Value::Null;
    D.get_or_init(|| {
        (0..meta::FEATURES.len())
            .map(|i| compute_default_value(FeatureId(i as u32)))
            .collect()
    })
    .get(f.0 as usize)
    .unwrap_or(&NULL)
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
        DataKind::Str | DataKind::Other | DataKind::JavaObject => Value::Null,
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
        // EcoreFactoryImpl.createEJavaObjectFromString: hex-encoded Java serialisation only
        DataKind::JavaObject => return None,
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

/// `Integer.valueOf` / `Long.valueOf`: optional sign, decimal digits, no whitespace
/// (`simoxide-jcompat`).
fn parse_java_int(s: &str, min: i64, max: i64) -> Option<i64> {
    simoxide_jcompat::parse::parse_integral_str(s, min, max)
}

/// `Double.valueOf`: trims characters `<= ' '`, accepts a trailing `d`/`f` type suffix, `NaN`,
/// `Infinity` and hexadecimal literals (`simoxide-jcompat`).
pub fn parse_java_double(s: &str) -> Option<f64> {
    simoxide_jcompat::parse::parse_double_str(s)
}

/// Whether `c` is a class of SimOxide's static tables (objects of other metamodels, e.g. PRM or
/// Ecore's metamodel objects, live in the same graph).
#[inline]
pub fn is_pcm_class(c: ClassId) -> bool {
    (c.0 as usize) < meta::CLASSES.len()
}

/// SimOxide's reads of a graph: attribute values as [`Value`]s with the static tables' defaults,
/// fragments and descriptions for diagnostics.
pub trait PcmGraph {
    /// Effective value of a single-valued attribute (default if unset).
    fn attr(&self, o: ObjId, f: FeatureId) -> Value;
    /// Values of a many-valued attribute.
    fn attrs(&self, o: ObjId, f: FeatureId) -> Vec<Value>;
    fn str_attr(&self, o: ObjId, f: FeatureId) -> Option<Box<str>>;
    /// `true` for an object of a PCM `Identifier` class without an `id` in the file: PCM's
    /// `IdentifierImpl` then invents a random UUID-based ID at load time.
    fn has_generated_id(&self, o: ObjId) -> bool;
    /// EMF `getURIFragment`: the `xmi:id`, else the ID, else the path.
    fn fragment(&self, o: ObjId) -> String;
    /// `resource-name#fragment`, or `?proxy-uri` for an unresolved proxy.
    fn describe_ref(&self, o: ObjId) -> String;
    /// Human-readable description for diagnostics: `name#fragment (pkg:Class "entityName")`.
    fn describe(&self, o: ObjId) -> String;
}

impl PcmGraph for Graph {
    fn attr(&self, o: ObjId, f: FeatureId) -> Value {
        match self.stored(o, f) {
            Some(v) => from_stored(v),
            None => default_value(f),
        }
    }

    fn attrs(&self, o: ObjId, f: FeatureId) -> Vec<Value> {
        self.stored_list(o, f).iter().map(from_stored).collect()
    }

    fn str_attr(&self, o: ObjId, f: FeatureId) -> Option<Box<str>> {
        match self.stored(o, f) {
            Some(simoxide_emf::Value::Str(s) | simoxide_emf::Value::Other(s)) => Some(s.clone()),
            Some(_) => None,
            None => match default_value_ref(f) {
                Value::Str(s) | Value::Other(s) => Some(s.clone()),
                _ => None,
            },
        }
    }

    fn has_generated_id(&self, o: ObjId) -> bool {
        let c = self[o].class;
        is_pcm_class(c)
            && c.def().id_attribute == Some(meta::feat::identifier_Identifier_id)
            && self.id(o).is_none()
            && self[o].proxy.is_none()
    }

    fn fragment(&self, o: ObjId) -> String {
        if let Some(x) = self.xmi_id(o) {
            return x.to_string();
        }
        match self.id(o) {
            Some(id) => id.to_string(),
            None => self.path_fragment(o),
        }
    }

    fn describe_ref(&self, o: ObjId) -> String {
        if let Some(p) = &self[o].proxy {
            return format!("?{p}");
        }
        let name = self
            .resource_of(o)
            .map_or("", |r| self.resource(r).name.as_str());
        format!("{name}#{}", self.fragment(o))
    }

    fn describe(&self, o: ObjId) -> String {
        let class = self[o].class;
        if !is_pcm_class(class) {
            return format!(
                "{} ({})",
                self.describe_ref(o),
                self.meta().qualified_name(class)
            );
        }
        let mut s = format!("{} ({}", self.describe_ref(o), class.qualified_name());
        if let Some(f) = class.feature("entityName")
            && let Some(n) = self.str_attr(o, f)
        {
            s.push_str(&format!(" {n:?}"));
        }
        s.push(')');
        s
    }
}

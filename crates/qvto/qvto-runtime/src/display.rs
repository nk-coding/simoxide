//! String forms of values: Java's `toString()` (OCL `toString()`, QVTo `repr()`, `log` data,
//! `joinfields`, string concatenation in Java code) and classic OCL's `CollectionUtil.toString`
//! (inside tuples).

use crate::coll::{Coll, CollKind};
use crate::identity::{HashEnv, IdentityKey};
use crate::java::fmt::{push_double, push_float, push_long};
use crate::value::{EnumLit, IntBox, ObjRef, TypeRef, Value};
use std::fmt::Write;

/// How model objects, enumeration literals and types print. The engine implements it with
/// the model store; [`DefaultObjects`] is a store-less fallback.
pub trait ObjectFormat {
    /// `EObject.toString()`; EMF prints `<Java class>@<hex identity hash>` followed by
    /// ` (eClass: ...)` (dynamic objects) or ` (attr: value, ...)` (generated classes).
    fn object(&self, o: ObjRef, env: &dyn HashEnv, out: &mut String);
    /// `Enumerator.toString()`: the literal's name.
    fn enum_literal(&self, e: EnumLit, out: &mut String);
    /// A type's `toString()`.
    fn type_name(&self, t: TypeRef, out: &mut String);
}

/// Fallback formatting without a model store: `DynamicEObjectImpl@<hex hash>`.
pub struct DefaultObjects;

impl ObjectFormat for DefaultObjects {
    fn object(&self, o: ObjRef, env: &dyn HashEnv, out: &mut String) {
        let key = IdentityKey::Obj(o);
        let _ = write!(out, "{}@{:x}", env.class_name(key), env.identity_hash(key));
    }
    fn enum_literal(&self, e: EnumLit, out: &mut String) {
        let _ = write!(out, "literal{}#{}", e.enum_id, e.literal);
    }
    fn type_name(&self, t: TypeRef, out: &mut String) {
        let _ = write!(out, "type#{}", t.0);
    }
}

/// Appends Java's `String.valueOf(v)`.
pub fn java_to_string(v: &Value, env: &dyn HashEnv, objs: &dyn ObjectFormat, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Invalid => out.push_str("invalid"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => push_long(out, *i),
        Value::Boxed(IntBox::Char, c) => out.push_str(&crate::str::Str::from_utf16(&[*c as u16]).to_str_lossy()),
        Value::Boxed(_, i) => push_long(out, *i as i64),
        Value::Real(d) => push_double(out, *d),
        Value::Float(f) => push_float(out, *f),
        Value::Str(s) => out.push_str(&s.to_str_lossy()),
        Value::Obj(o) => objs.object(*o, env, out),
        Value::Enum(e) => objs.enum_literal(*e, out),
        Value::Type(t) => objs.type_name(*t, out),
        Value::Coll(c) => match &**c {
            Coll::Bag(b) => {
                // BagImpl.toString() is its HashMap's: {element=count, ...}
                out.push('{');
                for (i, (e, cell)) in b.index.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    java_to_string(e, env, objs, out);
                    let _ = write!(out, "={}", cell.count);
                }
                out.push('}');
            }
            _ => list_to_string(c.iter(), env, objs, out),
        },
        Value::List(l) => list_to_string(l.borrow().items.iter(), env, objs, out),
        Value::Dict(d) => {
            out.push_str("Dictionary{");
            for (i, (k, v)) in d.borrow().iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                java_to_string(k, env, objs, out);
                out.push('=');
                java_to_string(v, env, objs, out);
            }
            out.push('}');
        }
        Value::Tuple(t) => {
            out.push_str("Tuple{");
            for (i, (n, v)) in t.names.iter().zip(t.values.iter()).enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&n.to_str_lossy());
                out.push_str(" = ");
                tuple_part(v, env, objs, out);
            }
            out.push('}');
        }
        Value::Model(m) => {
            let _ = write!(out, "{}@{:x}", env.class_name(IdentityKey::Model(*m)), env.identity_hash(IdentityKey::Model(*m)));
        }
        Value::Instance(m) => {
            let k = IdentityKey::Instance(*m);
            let _ = write!(out, "{}@{:x}", env.class_name(k), env.identity_hash(k));
        }
    }
}

/// `AbstractCollection.toString`: `[a, b]`.
fn list_to_string<'a>(it: impl Iterator<Item = &'a Value>, env: &dyn HashEnv, objs: &dyn ObjectFormat, out: &mut String) {
    out.push('[');
    for (i, e) in it.enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        java_to_string(e, env, objs, out);
    }
    out.push(']');
}

/// `AbstractTuple.toString(Object)`: strings quoted, collections as `CollectionUtil.toString`.
fn tuple_part(v: &Value, env: &dyn HashEnv, objs: &dyn ObjectFormat, out: &mut String) {
    match v {
        Value::Str(s) => {
            out.push('\'');
            out.push_str(&s.to_str_lossy());
            out.push('\'');
        }
        Value::Coll(_) | Value::List(_) | Value::Dict(_) => ocl_collection_to_string(v, env, objs, out),
        _ => java_to_string(v, env, objs, out),
    }
}

/// `CollectionUtil.toString`: `Set{1, 'a', Sequence{2}}`.
pub fn ocl_collection_to_string(v: &Value, env: &dyn HashEnv, objs: &dyn ObjectFormat, out: &mut String) {
    let (name, items): (&str, Vec<Value>) = match v {
        Value::Coll(c) => (
            match c.kind() {
                CollKind::Sequence => "Sequence",
                CollKind::OrderedSet => "OrderedSet",
                CollKind::Set => "Set",
                CollKind::Bag => "Bag",
            },
            c.to_vec(),
        ),
        Value::List(l) => ("Sequence", l.borrow().items.clone()),
        Value::Dict(d) => ("Collection", d.borrow().values()),
        _ => return java_to_string(v, env, objs, out),
    };
    out.push_str(name);
    out.push('{');
    for (i, e) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        match e {
            Value::Coll(_) | Value::List(_) | Value::Dict(_) => ocl_collection_to_string(e, env, objs, out),
            Value::Str(s) => {
                out.push('\'');
                out.push_str(&s.to_str_lossy());
                out.push('\'');
            }
            _ => java_to_string(e, env, objs, out),
        }
    }
    out.push('}');
}

/// OCL `toString()`: like Java's, but `*` for the unlimited value of static type
/// UnlimitedNatural.
pub fn ocl_to_string(v: &Value, unlimited: bool, env: &dyn HashEnv, objs: &dyn ObjectFormat) -> String {
    let mut s = String::new();
    if unlimited && matches!(v, Value::Int(-1)) {
        s.push('*');
    } else {
        java_to_string(v, env, objs, &mut s);
    }
    s
}

/// Convenience: `String.valueOf(v)` with [`DefaultObjects`].
pub fn to_java_string(v: &Value, env: &dyn HashEnv) -> String {
    let mut s = String::new();
    java_to_string(v, env, &DefaultObjects, &mut s);
    s
}

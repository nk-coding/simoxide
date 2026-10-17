//! Equality and hashing of values.
//!
//! Two notions, as in Eclipse (`reference/notes/eclipse-semantics.md` §2, §A.2, §B.2):
//!
//! - [`java_equals`] / [`java_hash`]: Java's `equals`/`hashCode` of the runtime objects. Hash
//!   collections, `includes`, `excludes`, `Dict` keys and trace keys use them, so
//!   `Set{1}->includes(1.0)` is false and `Set{1, 1.0}` has two elements.
//! - [`ocl_equal`]: `ObjectUtil.equal`, behind OCL's `=` and `<>`, `count` and `indexOf`:
//!   numbers compare after promotion (`1 = 1.0`), an OrderedSet only equals an OrderedSet and
//!   then in order.

use crate::coll::{Coll, CollKind};
use crate::identity::{HashEnv, IdentityKey};
use crate::java::hash::{
    bool_hash, double_hash, double_to_long_bits, float_hash, float_to_int_bits, long_hash,
};
use crate::java::hashmap::JavaKey;
use crate::str::Str;
use crate::value::{IntBox, JNum, Value};
use std::cmp::Ordering;

/// The identity key of an identity-hashed value.
pub fn identity_key(v: &Value) -> Option<IdentityKey> {
    Some(match v {
        Value::Invalid => IdentityKey::Invalid,
        Value::Obj(o) => IdentityKey::Obj(*o),
        Value::Enum(e) => IdentityKey::Enum(*e),
        Value::Type(t) => IdentityKey::Type(*t),
        Value::Model(m) => IdentityKey::Model(*m),
        Value::Instance(m) => IdentityKey::Instance(*m),
        _ => return None,
    })
}

/// Java `hashCode()` (0 for `null`).
pub fn java_hash(v: &Value, env: &dyn HashEnv) -> i32 {
    match v {
        Value::Null => 0,
        Value::Bool(b) => bool_hash(*b),
        Value::Int(i) => {
            if *i as i32 as i64 == *i {
                *i as i32
            } else {
                long_hash(*i)
            }
        }
        Value::Boxed(IntBox::Long, i) => long_hash(*i as i64),
        // Short, Byte, Character: the value
        Value::Boxed(_, i) => *i,
        Value::Real(d) => double_hash(*d),
        Value::Float(f) => float_hash(*f),
        Value::Str(s) => s.java_hash(),
        Value::Coll(c) => c.java_hash(env),
        Value::Tuple(t) => t.java_hash(env),
        Value::Dict(d) => d.borrow().java_hash(env),
        Value::List(l) => crate::coll::list_hash(l.borrow().items.iter(), env),
        Value::Invalid
        | Value::Obj(_)
        | Value::Enum(_)
        | Value::Type(_)
        | Value::Model(_)
        | Value::Instance(_) => env.identity_hash(identity_key(v).expect("identity-hashed")),
    }
}

/// `String.compareTo`: UTF-16 code unit order.
pub fn compare_str(a: &Str, b: &Str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        a.as_bytes().cmp(b.as_bytes())
    } else {
        a.units().cmp(b.units())
    }
}

/// Java `equals`. `null` equals only `null`; `invalid` is a singleton.
pub fn java_equals(a: &Value, b: &Value, env: &dyn HashEnv) -> bool {
    match (a, b) {
        (Value::Null, Value::Null) => true,
        (Value::Invalid, Value::Invalid) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::Boxed(k, x), Value::Boxed(l, y)) => k == l && x == y,
        (Value::Real(x), Value::Real(y)) => double_to_long_bits(*x) == double_to_long_bits(*y),
        (Value::Float(x), Value::Float(y)) => float_to_int_bits(*x) == float_to_int_bits(*y),
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Obj(x), Value::Obj(y)) => x == y,
        (Value::Enum(x), Value::Enum(y)) => x == y,
        (Value::Type(x), Value::Type(y)) => x == y,
        (Value::Model(x), Value::Model(y)) => x == y,
        (Value::Instance(x), Value::Instance(y)) => x == y,
        (Value::Coll(x), Value::Coll(y)) => {
            std::rc::Rc::ptr_eq(x, y) || coll_java_equals(x, y, env)
        }
        (Value::Coll(x), Value::List(l)) | (Value::List(l), Value::Coll(x)) => {
            x.kind() == CollKind::Sequence
                && list_equals(
                    x.iter(),
                    x.len(),
                    l.borrow().items.iter(),
                    l.borrow().items.len(),
                    env,
                )
        }
        (Value::List(x), Value::List(y)) => {
            std::rc::Rc::ptr_eq(x, y) || {
                let (x, y) = (x.borrow(), y.borrow());
                list_equals(
                    x.items.iter(),
                    x.items.len(),
                    y.items.iter(),
                    y.items.len(),
                    env,
                )
            }
        }
        (Value::Tuple(x), Value::Tuple(y)) => std::rc::Rc::ptr_eq(x, y) || x.java_equals(y, env),
        (Value::Dict(x), Value::Dict(y)) => {
            std::rc::Rc::ptr_eq(x, y) || x.borrow().java_equals(&y.borrow(), env)
        }
        _ => false,
    }
}

fn list_equals<'a>(
    a: impl Iterator<Item = &'a Value>,
    la: usize,
    b: impl Iterator<Item = &'a Value>,
    lb: usize,
    env: &dyn HashEnv,
) -> bool {
    la == lb && a.zip(b).all(|(x, y)| java_equals(x, y, env))
}

/// Java `equals` between two collections (`AbstractSet`, `AbstractList`, `BagImpl`).
pub fn coll_java_equals(a: &Coll, b: &Coll, env: &dyn HashEnv) -> bool {
    match (a.kind(), b.kind()) {
        (x, y) if x.is_set() && y.is_set() => {
            a.len() == b.len() && b.iter().all(|e| a.contains(e, env))
        }
        (CollKind::Sequence, CollKind::Sequence) => {
            list_equals(a.iter(), a.len(), b.iter(), b.len(), env)
        }
        (CollKind::Bag, CollKind::Bag) => {
            let (Coll::Bag(x), Coll::Bag(y)) = (a, b) else {
                unreachable!()
            };
            x.size == y.size
                && x.index
                    .iter()
                    .all(|(e, c)| y.index.get(e, env).is_some_and(|d| d.count == c.count))
        }
        _ => false,
    }
}

/// `ObjectUtil.isPrimitive`: Integer, Long, String, Boolean, Double.
fn is_ocl_primitive(v: &Value) -> bool {
    matches!(
        v,
        Value::Int(_)
            | Value::Boxed(IntBox::Long, _)
            | Value::Str(_)
            | Value::Bool(_)
            | Value::Real(_)
    )
}

/// `ObjectUtil.equal` (OCL `=`).
pub fn ocl_equal(a: &Value, b: &Value, env: &dyn HashEnv) -> bool {
    if a.is_null() || b.is_null() {
        return a.is_null() && b.is_null();
    }
    if is_ocl_primitive(a) || is_ocl_primitive(b) {
        // Integer promoted to Long; then Long/Double cross comparisons
        let num = |v: &Value| match v {
            Value::Int(i) => Some(JNum::Long(*i)),
            Value::Boxed(IntBox::Long, i) => Some(JNum::Long(*i as i64)),
            Value::Real(d) => Some(JNum::Double(*d)),
            _ => None,
        };
        return match (num(a), num(b)) {
            (Some(JNum::Long(x)), Some(JNum::Long(y))) => x == y,
            (Some(JNum::Long(x)), Some(JNum::Double(y))) => x as f64 == y,
            (Some(JNum::Double(x)), Some(JNum::Long(y))) => x == y as f64,
            (Some(JNum::Double(x)), Some(JNum::Double(y))) => x == y,
            _ => match (a, b) {
                (Value::Str(x), Value::Str(y)) => x == y,
                (Value::Bool(x), Value::Bool(y)) => x == y,
                _ => false,
            },
        };
    }
    if let (Value::Coll(x), Value::Coll(y)) = (a, b) {
        let xo = x.kind() == CollKind::OrderedSet;
        let yo = y.kind() == CollKind::OrderedSet;
        if xo != yo {
            return false;
        }
        if xo {
            return std::rc::Rc::ptr_eq(x, y)
                || (x.len() == y.len()
                    && x.iter().zip(y.iter()).all(|(p, q)| ocl_equal(p, q, env)));
        }
    } else if matches!(a, Value::Coll(c) if c.kind() == CollKind::OrderedSet)
        || matches!(b, Value::Coll(c) if c.kind() == CollKind::OrderedSet)
    {
        return false;
    }
    java_equals(a, b, env)
}

/// `ObjectUtil.hashCode`: hash consistent with [`ocl_equal`] for numbers.
pub fn ocl_hash(v: &Value, env: &dyn HashEnv) -> i32 {
    match v {
        Value::Null => 0,
        Value::Int(i) => 37i32.wrapping_mul(*i as i32),
        Value::Boxed(IntBox::Long, i) => 37i32.wrapping_mul(*i),
        Value::Real(d) => 37i32.wrapping_mul(*d as i32),
        _ => java_hash(v, env),
    }
}

// Comparable class ids for HashMap tree bins.
const CC_STRING: u8 = 1;
const CC_INTEGER: u8 = 2;
const CC_LONG: u8 = 3;
const CC_SHORT: u8 = 4;
const CC_BYTE: u8 = 5;
const CC_CHAR: u8 = 6;
const CC_DOUBLE: u8 = 7;
const CC_FLOAT: u8 = 8;
const CC_BOOLEAN: u8 = 9;

/// Java class name of a value (`getClass().getName()`), `None` for `null`.
pub fn java_class_name<'a>(v: &'a Value, env: &'a dyn HashEnv) -> Option<&'a str> {
    Some(match v {
        Value::Null => return None,
        Value::Bool(_) => "java.lang.Boolean",
        Value::Int(i) => {
            if *i as i32 as i64 == *i {
                "java.lang.Integer"
            } else {
                "java.lang.Long"
            }
        }
        Value::Boxed(IntBox::Long, _) => "java.lang.Long",
        Value::Boxed(IntBox::Short, _) => "java.lang.Short",
        Value::Boxed(IntBox::Byte, _) => "java.lang.Byte",
        Value::Boxed(IntBox::Char, _) => "java.lang.Character",
        Value::Real(_) => "java.lang.Double",
        Value::Float(_) => "java.lang.Float",
        Value::Str(_) => "java.lang.String",
        Value::Coll(c) => c.kind().java_class(),
        Value::Tuple(_) => "org.eclipse.ocl.ecore.internal.evaluation.TupleImpl",
        Value::Dict(_) => "org.eclipse.m2m.internal.qvt.oml.stdlib.DictionaryImpl",
        Value::List(_) => "org.eclipse.m2m.internal.qvt.oml.stdlib.MutableListImpl",
        _ => env.class_name(identity_key(v).expect("identity-hashed")),
    })
}

impl<'e> JavaKey<dyn HashEnv + 'e> for Value {
    #[inline]
    fn java_hash(&self, env: &(dyn HashEnv + 'e)) -> i32 {
        java_hash(self, env)
    }
    #[inline]
    fn java_equals(&self, other: &Self, env: &(dyn HashEnv + 'e)) -> bool {
        java_equals(self, other, env)
    }
    fn comparable_class(&self) -> Option<u8> {
        Some(match self {
            Value::Str(_) => CC_STRING,
            Value::Int(i) => {
                if *i as i32 as i64 == *i {
                    CC_INTEGER
                } else {
                    CC_LONG
                }
            }
            Value::Boxed(IntBox::Long, _) => CC_LONG,
            Value::Boxed(IntBox::Short, _) => CC_SHORT,
            Value::Boxed(IntBox::Byte, _) => CC_BYTE,
            Value::Boxed(IntBox::Char, _) => CC_CHAR,
            Value::Real(_) => CC_DOUBLE,
            Value::Float(_) => CC_FLOAT,
            Value::Bool(_) => CC_BOOLEAN,
            _ => return None,
        })
    }
    fn compare_to(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Value::Str(a), Value::Str(b)) => compare_str(a, b),
            (Value::Int(a), Value::Int(b)) => a.cmp(b),
            (Value::Int(a), Value::Boxed(_, b)) => a.cmp(&(*b as i64)),
            (Value::Boxed(_, a), Value::Int(b)) => (*a as i64).cmp(b),
            (Value::Boxed(_, a), Value::Boxed(_, b)) => a.cmp(b),
            (Value::Real(a), Value::Real(b)) => crate::java::hash::double_compare(*a, *b),
            (Value::Float(a), Value::Float(b)) => crate::java::hash::float_compare(*a, *b),
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
            _ => Ordering::Equal,
        }
    }
    fn class_name<'a>(&'a self, env: &'a (dyn HashEnv + 'e)) -> Option<&'a str> {
        java_class_name(self, env)
    }
    fn identity_hash(&self, env: &(dyn HashEnv + 'e)) -> i32 {
        match identity_key(self) {
            Some(k) => env.identity_hash(k),
            None if self.is_null() => 0,
            // Boxes, strings and collections have identities of their own in Java, which a
            // value cannot reproduce. HashMap consults them only to order a key against `null`
            // (Java identity hashes are positive, so any positive value orders correctly) or
            // two equal-hash keys of one non-comparable class (collections, tuples) in a tree
            // bin, where Java's order is not reproducible (DESIGN §12).
            None => 1,
        }
    }
}

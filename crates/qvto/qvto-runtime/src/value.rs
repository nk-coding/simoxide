//! Runtime values.
//!
//! [`Value`] is 16 bytes: a tag and an 8-byte payload. Scalars are inline; strings are thin
//! reference-counted pointers ([`Str`]); collections and tuples are immutable `Rc`s, copied on
//! write; `Dict` and `List` are mutable and shared by reference, as in Eclipse.
//!
//! Numbers follow classic Eclipse OCL with QVTo's long integers (`reference/notes/
//! eclipse-semantics.md` §B): an OCL Integer is a Java `Integer` when it fits in 32 bits and a
//! `Long` otherwise ([`Value::Int`] holds both, normalised like `NumberUtil.coerceNumber`).
//! EMF attributes of type `ELong`, `EShort`, `EByte`, `EChar` deliver uncoerced Java boxes,
//! which Java's `equals` tells apart from an `Integer` of the same value; those are kept as
//! [`Value::Boxed`] (only when the value is in `int` range: a `Long` outside it is
//! [`Value::Int`]). Reals are `Double`s; `EFloat` attributes give [`Value::Float`].

use crate::coll::Coll;
use crate::dict::{Dict, List};
use crate::str::Str;
use crate::tuple::Tuple;
use std::cell::RefCell;
use std::rc::Rc;

/// A model object handle: a host store object, or an engine-local object (intermediate class
/// instances, exceptions, `Status`), told apart by the top bit (`DESIGN.md` §4.2).
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ObjRef(pub u32);

impl ObjRef {
    pub const LOCAL_BIT: u32 = 1 << 31;
    pub fn host(i: u32) -> ObjRef {
        debug_assert!(i & Self::LOCAL_BIT == 0);
        ObjRef(i)
    }
    pub fn local(i: u32) -> ObjRef {
        debug_assert!(i & Self::LOCAL_BIT == 0);
        ObjRef(i | Self::LOCAL_BIT)
    }
    pub fn is_local(self) -> bool {
        self.0 & Self::LOCAL_BIT != 0
    }
    /// Index within its store.
    pub fn index(self) -> u32 {
        self.0 & !Self::LOCAL_BIT
    }
}

/// An enumeration literal: the enumeration's id (`qvto_model::EnumId`) and the literal's index.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct EnumLit {
    pub enum_id: u32,
    pub literal: u32,
}

/// A type used as a value (argument of `oclAsType`, `allInstances`, `objectsOfType`, ...).
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct TypeRef(pub u32);

/// The Java class of an uncoerced integral box (see the module docs).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum IntBox {
    /// `java.lang.Long` with a value in `int` range.
    Long,
    Short,
    Byte,
    /// `java.lang.Character` (a UTF-16 code unit).
    Char,
}

/// A runtime value. See the module docs.
#[derive(Clone, Debug, Default)]
pub enum Value {
    #[default]
    Null,
    Invalid,
    Bool(bool),
    /// OCL Integer: Java `Integer` if it fits in 32 bits, else `Long`.
    Int(i64),
    /// An uncoerced Java box from an EMF attribute.
    Boxed(IntBox, i32),
    /// OCL Real (`java.lang.Double`).
    Real(f64),
    /// `java.lang.Float` from an `EFloat` attribute.
    Float(f32),
    Str(Str),
    Obj(ObjRef),
    Enum(EnumLit),
    Type(TypeRef),
    /// Set, OrderedSet, Bag or Sequence; immutable.
    Coll(Rc<Coll>),
    Tuple(Rc<Tuple>),
    /// QVTo `Dict`: mutable, shared by reference.
    Dict(Rc<RefCell<Dict>>),
    /// QVTo `List`: mutable, shared by reference.
    List(Rc<RefCell<List>>),
    /// A model extent (`ModelInstance`).
    Model(u32),
    /// A transformation or library instance (`this`).
    Instance(u32),
}

const _: () = assert!(std::mem::size_of::<Value>() == 16);

/// The number classes of Java that OCL arithmetic distinguishes.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum JNum {
    Integer(i32),
    Long(i64),
    Short(i16),
    Byte(i8),
    Double(f64),
    Float(f32),
}

impl JNum {
    /// `NumberUtil.higherPrecisionNumber`: integral boxes to `Long`, `Float` to `Double`.
    pub fn higher(self) -> JNum {
        match self {
            JNum::Integer(i) => JNum::Long(i as i64),
            JNum::Short(i) => JNum::Long(i as i64),
            JNum::Byte(i) => JNum::Long(i as i64),
            JNum::Float(f) => JNum::Double(f as f64),
            x => x,
        }
    }
    /// `NumberUtil.coerceNumber`: `Integer` when an integral value fits, `Float` to `Double`.
    pub fn coerce(self) -> JNum {
        match self {
            JNum::Short(i) => JNum::Integer(i as i32),
            JNum::Byte(i) => JNum::Integer(i as i32),
            JNum::Long(l) if l as i32 as i64 == l => JNum::Integer(l as i32),
            JNum::Float(f) => JNum::Double(f as f64),
            x => x,
        }
    }
    /// `doubleValue()`.
    pub fn as_f64(self) -> f64 {
        match self {
            JNum::Integer(i) => i as f64,
            JNum::Long(i) => i as f64,
            JNum::Short(i) => i as f64,
            JNum::Byte(i) => i as f64,
            JNum::Double(d) => d,
            JNum::Float(f) => f as f64,
        }
    }
    /// `longValue()`.
    pub fn as_i64(self) -> i64 {
        match self {
            JNum::Integer(i) => i as i64,
            JNum::Long(i) => i,
            JNum::Short(i) => i as i64,
            JNum::Byte(i) => i as i64,
            JNum::Double(d) => java_d2l(d),
            JNum::Float(f) => java_d2l(f as f64),
        }
    }
    /// `intValue()`.
    pub fn as_i32(self) -> i32 {
        match self {
            JNum::Double(d) => java_d2i(d),
            JNum::Float(f) => java_d2i(f as f64),
            x => x.as_i64() as i32,
        }
    }
    pub fn into_value(self) -> Value {
        match self {
            JNum::Integer(i) => Value::Int(i as i64),
            JNum::Long(l) => Value::long(l),
            JNum::Short(s) => Value::Boxed(IntBox::Short, s as i32),
            JNum::Byte(b) => Value::Boxed(IntBox::Byte, b as i32),
            JNum::Double(d) => Value::Real(d),
            JNum::Float(f) => Value::Float(f),
        }
    }
}

/// Java's `(long) d`: saturating, NaN to 0 (Rust's `as` does the same).
#[inline]
pub fn java_d2l(d: f64) -> i64 {
    d as i64
}

/// Java's `(int) d`: saturating, NaN to 0.
#[inline]
pub fn java_d2i(d: f64) -> i32 {
    d as i32
}

impl Value {
    pub fn int(i: i32) -> Value {
        Value::Int(i as i64)
    }
    /// A `java.lang.Long` box: [`Value::Int`] outside `int` range, else a [`Value::Boxed`].
    pub fn long(l: i64) -> Value {
        if l as i32 as i64 == l {
            Value::Boxed(IntBox::Long, l as i32)
        } else {
            Value::Int(l)
        }
    }
    /// An integer result of OCL arithmetic (`coerceNumber` of a `Long`).
    pub fn integer(l: i64) -> Value {
        Value::Int(l)
    }
    pub fn str(s: &str) -> Value {
        Value::Str(Str::new(s))
    }
    pub fn real(d: f64) -> Value {
        Value::Real(d)
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    pub fn is_invalid(&self) -> bool {
        matches!(self, Value::Invalid)
    }
    /// `null` or `invalid`.
    pub fn is_undefined(&self) -> bool {
        matches!(self, Value::Null | Value::Invalid)
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    /// True only for `Boolean.TRUE` (`when`, `where`, `while`, `assert`).
    pub fn is_true(&self) -> bool {
        matches!(self, Value::Bool(true))
    }
    pub fn as_str(&self) -> Option<&Str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_coll(&self) -> Option<&Rc<Coll>> {
        match self {
            Value::Coll(c) => Some(c),
            _ => None,
        }
    }

    /// The Java number class of a numeric value.
    pub fn as_jnum(&self) -> Option<JNum> {
        Some(match *self {
            Value::Int(i) => {
                if i as i32 as i64 == i {
                    JNum::Integer(i as i32)
                } else {
                    JNum::Long(i)
                }
            }
            Value::Boxed(IntBox::Long, i) => JNum::Long(i as i64),
            Value::Boxed(IntBox::Short, i) => JNum::Short(i as i16),
            Value::Boxed(IntBox::Byte, i) => JNum::Byte(i as i8),
            Value::Real(d) => JNum::Double(d),
            Value::Float(f) => JNum::Float(f),
            _ => return None,
        })
    }

    /// Java `instanceof Number` (a `Character` is not a number).
    pub fn is_number(&self) -> bool {
        self.as_jnum().is_some()
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Value {
        Value::Bool(b)
    }
}
impl From<i32> for Value {
    fn from(i: i32) -> Value {
        Value::Int(i as i64)
    }
}
impl From<f64> for Value {
    fn from(d: f64) -> Value {
        Value::Real(d)
    }
}
impl From<Str> for Value {
    fn from(s: Str) -> Value {
        Value::Str(s)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Value {
        Value::Str(Str::new(s))
    }
}
impl From<Coll> for Value {
    fn from(c: Coll) -> Value {
        Value::Coll(Rc::new(c))
    }
}

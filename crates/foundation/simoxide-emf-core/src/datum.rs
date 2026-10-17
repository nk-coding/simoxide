//! Feature values as seen through the model API.

use crate::ids::EnumId;

/// A feature value. Numbers keep their Java box class (`Integer`, `Long`, `Short`, ...), because
/// Java's `equals` tells them apart and QVTo collections and trace keys depend on it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Datum<'a, O> {
    Null,
    Bool(bool),
    Int(i32),
    Long(i64),
    Short(i16),
    Byte(i8),
    /// A Java `char` (UTF-16 code unit).
    Char(u16),
    Double(f64),
    Float(f32),
    Str(&'a str),
    /// Enumeration literal: the enum and the literal's index in `EnumInfo::literals`.
    Enum(EnumId, u32),
    Obj(O),
    /// Lexical value of a data type kept as text (`BigInteger`, `BigDecimal`, `Date`, ...).
    Other(&'a str),
}

impl<'a, O: Copy> Datum<'a, O> {
    pub fn is_null(&self) -> bool {
        matches!(self, Datum::Null)
    }
    pub fn as_obj(&self) -> Option<O> {
        match self {
            Datum::Obj(o) => Some(*o),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&'a str> {
        match self {
            Datum::Str(s) | Datum::Other(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Datum::Bool(b) => Some(*b),
            _ => None,
        }
    }
    /// Any integral value widened to `i64`.
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Datum::Int(i) => Some(i as i64),
            Datum::Long(i) => Some(i),
            Datum::Short(i) => Some(i as i64),
            Datum::Byte(i) => Some(i as i64),
            Datum::Char(c) => Some(c as i64),
            _ => None,
        }
    }
    pub fn to_owned(&self) -> OwnedDatum<O> {
        match *self {
            Datum::Null => OwnedDatum::Null,
            Datum::Bool(b) => OwnedDatum::Bool(b),
            Datum::Int(i) => OwnedDatum::Int(i),
            Datum::Long(i) => OwnedDatum::Long(i),
            Datum::Short(i) => OwnedDatum::Short(i),
            Datum::Byte(i) => OwnedDatum::Byte(i),
            Datum::Char(c) => OwnedDatum::Char(c),
            Datum::Double(d) => OwnedDatum::Double(d),
            Datum::Float(f) => OwnedDatum::Float(f),
            Datum::Str(s) => OwnedDatum::Str(s.into()),
            Datum::Enum(e, i) => OwnedDatum::Enum(e, i),
            Datum::Obj(o) => OwnedDatum::Obj(o),
            Datum::Other(s) => OwnedDatum::Other(s.into()),
        }
    }
    /// Java `equals` of the boxed values (references by identity; `NaN` equals `NaN`, `0.0`
    /// differs from `-0.0`, as `Double.equals` does).
    pub fn java_equals(&self, other: &Datum<'_, O>) -> bool
    where
        O: PartialEq,
    {
        match (self, other) {
            (Datum::Double(a), Datum::Double(b)) => a.to_bits() == b.to_bits(),
            (Datum::Float(a), Datum::Float(b)) => a.to_bits() == b.to_bits(),
            (Datum::Str(a), Datum::Str(b)) | (Datum::Other(a), Datum::Other(b)) => a == b,
            (Datum::Null, Datum::Null) => true,
            (Datum::Bool(a), Datum::Bool(b)) => a == b,
            (Datum::Int(a), Datum::Int(b)) => a == b,
            (Datum::Long(a), Datum::Long(b)) => a == b,
            (Datum::Short(a), Datum::Short(b)) => a == b,
            (Datum::Byte(a), Datum::Byte(b)) => a == b,
            (Datum::Char(a), Datum::Char(b)) => a == b,
            (Datum::Enum(e, a), Datum::Enum(f, b)) => e == f && a == b,
            (Datum::Obj(a), Datum::Obj(b)) => a == b,
            _ => false,
        }
    }
}

/// An owned [`Datum`].
#[derive(Clone, Debug, PartialEq)]
pub enum OwnedDatum<O> {
    Null,
    Bool(bool),
    Int(i32),
    Long(i64),
    Short(i16),
    Byte(i8),
    Char(u16),
    Double(f64),
    Float(f32),
    Str(Box<str>),
    Enum(EnumId, u32),
    Obj(O),
    Other(Box<str>),
}

impl<O: Copy> OwnedDatum<O> {
    pub fn as_datum(&self) -> Datum<'_, O> {
        match self {
            OwnedDatum::Null => Datum::Null,
            OwnedDatum::Bool(b) => Datum::Bool(*b),
            OwnedDatum::Int(i) => Datum::Int(*i),
            OwnedDatum::Long(i) => Datum::Long(*i),
            OwnedDatum::Short(i) => Datum::Short(*i),
            OwnedDatum::Byte(i) => Datum::Byte(*i),
            OwnedDatum::Char(c) => Datum::Char(*c),
            OwnedDatum::Double(d) => Datum::Double(*d),
            OwnedDatum::Float(f) => Datum::Float(*f),
            OwnedDatum::Str(s) => Datum::Str(s),
            OwnedDatum::Enum(e, i) => Datum::Enum(*e, *i),
            OwnedDatum::Obj(o) => Datum::Obj(*o),
            OwnedDatum::Other(s) => Datum::Other(s),
        }
    }
}

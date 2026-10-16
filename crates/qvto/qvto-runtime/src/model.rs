//! Values to and from the model API ([`qvto_model::Datum`]).
//!
//! A store's object handle (`ModelStore::Obj`, e.g. `qvto_emf::ObjId(u32)`) becomes an
//! [`ObjRef`] through a caller-supplied mapping; for stores with dense `u32` handles it is
//! `ObjRef::host(id)`, and [`IdentityKey::Obj`](crate::IdentityKey) of that `ObjRef` keys the
//! object's identity hash (memoised by the run's [`HashEnv`](crate::HashEnv), drawn from a
//! tape or the HotSpot surrogate).
//!
//! The mapping keeps Java's box classes: `Datum::Int` is an OCL Integer, `Long`, `Short`,
//! `Byte` and `Char` stay uncoerced boxes ([`Value::Boxed`], or [`Value::Int`] for a `Long`
//! outside `int` range), `Float` stays a `Float` (classic OCL does not coerce navigation
//! results, `reference/notes/eclipse-semantics.md` §A.6).

use crate::str::Str;
use crate::value::{EnumLit, IntBox, ObjRef, Value};
use qvto_model::{Datum, EnumId, OwnedDatum};

/// A feature value as a runtime value.
pub fn datum_to_value<O: Copy>(d: Datum<'_, O>, obj: impl FnOnce(O) -> ObjRef) -> Value {
    match d {
        Datum::Null => Value::Null,
        Datum::Bool(b) => Value::Bool(b),
        Datum::Int(i) => Value::Int(i as i64),
        Datum::Long(l) => Value::long(l),
        Datum::Short(s) => Value::Boxed(IntBox::Short, s as i32),
        Datum::Byte(b) => Value::Boxed(IntBox::Byte, b as i32),
        Datum::Char(c) => Value::Boxed(IntBox::Char, c as i32),
        Datum::Double(x) => Value::Real(x),
        Datum::Float(x) => Value::Float(x),
        Datum::Str(s) => Value::Str(Str::new(s)),
        Datum::Enum(e, i) => Value::Enum(EnumLit {
            enum_id: e.0,
            literal: i,
        }),
        Datum::Obj(o) => Value::Obj(obj(o)),
        // BigInteger, BigDecimal, Date, ...: kept as their lexical form (DESIGN §12, Q1c-8)
        Datum::Other(s) => Value::Str(Str::new(s)),
    }
}

/// A runtime value as a feature value, if it is one: `None` for collections, tuples, `Dict`,
/// `List`, `invalid`, types, models and module instances, and for strings with lone surrogates
/// (see [`value_to_owned_datum`]).
pub fn value_to_datum<O>(v: &Value, obj: impl FnOnce(ObjRef) -> O) -> Option<Datum<'_, O>> {
    Some(match v {
        Value::Null => Datum::Null,
        Value::Bool(b) => Datum::Bool(*b),
        Value::Int(i) => {
            if *i as i32 as i64 == *i {
                Datum::Int(*i as i32)
            } else {
                Datum::Long(*i)
            }
        }
        Value::Boxed(IntBox::Long, i) => Datum::Long(*i as i64),
        Value::Boxed(IntBox::Short, i) => Datum::Short(*i as i16),
        Value::Boxed(IntBox::Byte, i) => Datum::Byte(*i as i8),
        Value::Boxed(IntBox::Char, i) => Datum::Char(*i as u16),
        Value::Real(x) => Datum::Double(*x),
        Value::Float(x) => Datum::Float(*x),
        Value::Str(s) => Datum::Str(s.as_str()?),
        Value::Enum(e) => Datum::Enum(EnumId(e.enum_id), e.literal),
        Value::Obj(o) => Datum::Obj(obj(*o)),
        _ => return None,
    })
}

/// Like [`value_to_datum`], owned; lone surrogates become U+FFFD (as Java's UTF-8 encoders
/// write them as `?`, such strings cannot round-trip through XMI anyway).
pub fn value_to_owned_datum<O: Copy>(
    v: &Value,
    obj: impl FnOnce(ObjRef) -> O,
) -> Option<OwnedDatum<O>> {
    match v {
        Value::Str(s) => Some(OwnedDatum::Str(s.to_str_lossy().into())),
        _ => value_to_datum(v, obj).map(|d| d.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let id = |o: u32| ObjRef::host(o);
        for d in [
            Datum::<u32>::Null,
            Datum::Int(5),
            Datum::Long(5),
            Datum::Long(1 << 40),
            Datum::Short(-3),
            Datum::Byte(7),
            Datum::Char(0x41),
            Datum::Double(1.5),
            Datum::Float(2.5),
            Datum::Str("x"),
            Datum::Enum(EnumId(3), 1),
            Datum::Obj(9),
        ] {
            let v = datum_to_value(d, id);
            let back = value_to_datum(&v, |o: ObjRef| o.index()).unwrap();
            assert!(back.java_equals(&d), "{d:?} -> {v:?} -> {back:?}");
        }
        // Long(5) and Int(5) stay apart
        let env = crate::FastEnv;
        assert!(!crate::eq::java_equals(
            &datum_to_value(Datum::<u32>::Long(5), id),
            &Value::Int(5),
            &env
        ));
        assert!(crate::eq::ocl_equal(
            &datum_to_value(Datum::<u32>::Long(5), id),
            &Value::Int(5),
            &env
        ));
    }
}

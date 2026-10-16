//! Data values: EMF's `EFactory.createFromString` and `convertToString` for every data type
//! kind, Java's number formatting (`Double.toString`, `Float.toString`) and the intrinsic
//! defaults of data types.
//!
//! The rules follow `EcoreFactoryImpl` for Ecore's own data types and the reflective
//! `EFactoryImpl` for dynamic data types (EPL-2.0, see `LICENSES`).

use crate::graph::Value;
use qvto_model::{Classifier, DataKind, MetaRegistry, PackageId};

/// `createFromString` for a value of type `ty`. `ecore` tells whether the data type belongs to
/// Ecore's own package (its factory parses booleans strictly).
pub fn parse_value(meta: &MetaRegistry, ty: Classifier, s: &str, ecore: Option<PackageId>) -> Result<Value, String> {
    match ty {
        Classifier::Enum(e) => meta
            .enumeration(e)
            .by_literal(s)
            .map(|i| Value::Enum(e, i))
            .ok_or_else(|| {
                format!(
                    "The value '{s}' is not a valid enumerator of '{}'",
                    meta.enumeration(e).name
                )
            }),
        Classifier::DataType(d) => {
            let dt = meta.data_type(d);
            let is_ecore = Some(dt.package) == ecore;
            parse_kind(dt.kind, s, is_ecore)
        }
        Classifier::Class(_) => Err("not a data type".into()),
    }
}

/// Parses `s` as a value of kind `kind` (`Integer.valueOf`, `Double.valueOf`, ...).
pub fn parse_kind(kind: DataKind, s: &str, ecore_factory: bool) -> Result<Value, String> {
    let bad = || format!("The value '{s}' is invalid.");
    Ok(match kind {
        DataKind::String => Value::Str(s.into()),
        DataKind::Boolean => {
            if s.eq_ignore_ascii_case("true") {
                Value::Bool(true)
            } else if s.eq_ignore_ascii_case("false") || !ecore_factory {
                // `new Boolean(s)` (dynamic data types) never fails
                Value::Bool(false)
            } else {
                return Err("Expecting true or false".into());
            }
        }
        DataKind::Int => Value::Int(parse_java_int(s, i32::MIN as i64, i32::MAX as i64).ok_or_else(bad)? as i32),
        DataKind::Long => Value::Long(parse_java_int(s, i64::MIN, i64::MAX).ok_or_else(bad)?),
        DataKind::Short => Value::Short(parse_java_int(s, i16::MIN as i64, i16::MAX as i64).ok_or_else(bad)? as i16),
        DataKind::Byte => Value::Byte(parse_java_int(s, i8::MIN as i64, i8::MAX as i64).ok_or_else(bad)? as i8),
        DataKind::Char => match parse_java_int(s, i32::MIN as i64, i32::MAX as i64) {
            Some(v) => Value::Char(v as u16),
            None => Value::Char(*s.encode_utf16().collect::<Vec<_>>().first().ok_or_else(bad)?),
        },
        DataKind::Double => Value::Double(parse_java_double(s).ok_or_else(bad)?),
        DataKind::Float => Value::Float(parse_java_float(s).ok_or_else(bad)?),
        DataKind::BigInteger => Value::Other(normalize_big_integer(s).ok_or_else(bad)?.into()),
        DataKind::BigDecimal | DataKind::Date | DataKind::Other => Value::Other(s.into()),
        DataKind::Unloadable => Value::Null,
    })
}

/// `convertToString`; `None` for null.
pub fn value_to_string(meta: &MetaRegistry, v: &Value) -> Option<String> {
    Some(match v {
        Value::Null => return None,
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Long(i) => i.to_string(),
        Value::Short(i) => i.to_string(),
        Value::Byte(i) => i.to_string(),
        Value::Char(c) => (*c as i32).to_string(),
        Value::Double(d) => java_double_to_string(*d),
        Value::Float(f) => java_float_to_string(*f),
        Value::Str(s) | Value::Other(s) => s.to_string(),
        Value::Enum(e, i) => meta.enumeration(*e).literals[*i as usize].literal.to_string(),
        Value::Obj(_) => return None,
    })
}

/// The intrinsic default of a type (`EClassifier.getDefaultValue()`): zero values of Java
/// primitives, the first literal of an enumeration, otherwise null.
pub fn intrinsic_default(meta: &MetaRegistry, ty: Classifier) -> Value {
    match ty {
        Classifier::Enum(e) => {
            if meta.enumeration(e).literals.is_empty() {
                Value::Null
            } else {
                Value::Enum(e, 0)
            }
        }
        Classifier::DataType(d) => {
            let dt = meta.data_type(d);
            if !dt.primitive {
                return Value::Null;
            }
            match dt.kind {
                DataKind::Boolean => Value::Bool(false),
                DataKind::Int => Value::Int(0),
                DataKind::Long => Value::Long(0),
                DataKind::Short => Value::Short(0),
                DataKind::Byte => Value::Byte(0),
                DataKind::Char => Value::Char(0),
                DataKind::Double => Value::Double(0.0),
                DataKind::Float => Value::Float(0.0),
                _ => Value::Null,
            }
        }
        Classifier::Class(_) => Value::Null,
    }
}

/// `Integer.valueOf` / `Long.valueOf` (`parseInt`): optional sign, ASCII digits, in range.
pub fn parse_java_int(s: &str, min: i64, max: i64) -> Option<i64> {
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

/// `Double.valueOf`: trims characters `<= ' '`, accepts a `d`/`D`/`f`/`F` suffix, `NaN`,
/// `Infinity` with a sign, and the usual decimal forms. Hexadecimal floats are not supported.
pub fn parse_java_double(s: &str) -> Option<f64> {
    let (sign, body) = java_float_body(s)?;
    match body {
        "NaN" => return Some(f64::NAN),
        "Infinity" => return Some(sign * f64::INFINITY),
        _ => {}
    }
    let v: f64 = body.parse().ok()?;
    Some(sign * v)
}

/// `Float.valueOf` (correctly rounded to `float`).
pub fn parse_java_float(s: &str) -> Option<f32> {
    let (sign, body) = java_float_body(s)?;
    match body {
        "NaN" => return Some(f32::NAN),
        "Infinity" => return Some(sign as f32 * f32::INFINITY),
        _ => {}
    }
    let v: f32 = body.parse().ok()?;
    Some(sign as f32 * v)
}

fn java_float_body(s: &str) -> Option<(f64, &str)> {
    let t = s.trim_matches(|c: char| c <= ' ');
    let (sign, body) = match t.as_bytes().first()? {
        b'-' => (-1.0, &t[1..]),
        b'+' => (1.0, &t[1..]),
        _ => (1.0, t),
    };
    if body == "NaN" || body == "Infinity" {
        return Some((sign, body));
    }
    let body = body.strip_suffix(['d', 'D', 'f', 'F']).unwrap_or(body);
    // Java's grammar: digits with optional point and exponent; Rust accepts "inf" etc.
    if body.is_empty()
        || !body
            .bytes()
            .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-'))
        || !body.bytes().any(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some((sign, body))
}

fn normalize_big_integer(s: &str) -> Option<String> {
    let (neg, digits) = match s.as_bytes().first()? {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let d = digits.trim_start_matches('0');
    if d.is_empty() {
        return Some("0".into());
    }
    Some(if neg { format!("-{d}") } else { d.to_string() })
}

/// Java `Double.toString` (JDK 19+: shortest digits that round-trip).
pub fn java_double_to_string(d: f64) -> String {
    if d.is_nan() {
        return "NaN".into();
    }
    if d.is_infinite() {
        return if d > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if d == 0.0 {
        return if d.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    java_layout(&format!("{:e}", d.abs()), d < 0.0, d.abs())
}

/// Java `Float.toString` (shortest digits that round-trip as `float`).
pub fn java_float_to_string(f: f32) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0" } else { "0.0" }.into();
    }
    java_layout(&format!("{:e}", f.abs()), f < 0.0, f.abs() as f64)
}

fn java_layout(sci: &str, negative: bool, a: f64) -> String {
    let (mant, exp) = sci.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let sign = if negative { "-" } else { "" };
    if (1e-3..1e7).contains(&a) {
        let point = exp + 1;
        let s = if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), digits)
        } else if point as usize >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(point as usize - digits.len()))
        } else {
            format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
        };
        format!("{sign}{s}")
    } else {
        let frac = if digits.len() > 1 { &digits[1..] } else { "0" };
        format!("{sign}{}.{}E{}", &digits[..1], frac, exp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_number_formatting() {
        assert_eq!(java_double_to_string(1.0), "1.0");
        assert_eq!(java_double_to_string(0.1), "0.1");
        assert_eq!(java_double_to_string(1e7), "1.0E7");
        assert_eq!(java_double_to_string(1234567.0), "1234567.0");
        assert_eq!(java_double_to_string(0.001), "0.001");
        assert_eq!(java_double_to_string(0.0001), "1.0E-4");
        assert_eq!(java_double_to_string(-2.5e-10), "-2.5E-10");
        assert_eq!(java_float_to_string(0.1), "0.1");
        assert_eq!(java_float_to_string(1.5e10), "1.5E10");
        assert_eq!(parse_java_double(" 1.5d "), Some(1.5));
        assert_eq!(parse_java_double("1e3"), Some(1000.0));
        assert_eq!(parse_java_double("inf"), None);
        assert_eq!(parse_java_int("+12", -100, 100), Some(12));
        assert_eq!(parse_java_int(" 12", -100, 100), None);
        assert_eq!(normalize_big_integer("-007").as_deref(), Some("-7"));
    }
}

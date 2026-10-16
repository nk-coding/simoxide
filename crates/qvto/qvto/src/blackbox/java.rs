//! Helpers for native implementations ported from Java: reading arguments as the Java types
//! Eclipse's reflection passes them, and making Java results.

use qvto_runtime::{IntBox, Value};

/// A `java.lang.String` argument (`None` for `null`).
pub fn string(v: &Value) -> Option<String> {
    v.as_str().map(|s| s.to_str_lossy().into_owned())
}

/// The UTF-16 code units of a string argument.
pub fn utf16(v: &Value) -> Option<Vec<u16>> {
    v.as_str().map(|s| s.to_utf16())
}

/// A string result from UTF-16 code units.
pub fn from_utf16(units: &[u16]) -> Value {
    Value::Str(qvto_runtime::Str::from_utf16(units))
}

/// An integral argument (`Integer`, `Long`, `Short`, ...).
pub fn int(v: &Value) -> Option<i64> {
    match v {
        Value::Int(i) => Some(*i),
        Value::Boxed(_, i) => Some(*i as i64),
        Value::Real(r) => Some(*r as i64),
        Value::Float(f) => Some(*f as i64),
        _ => None,
    }
}

/// A floating-point argument (`Double`, `Float`, or an integer converted as
/// `NumberConversions.convertNumber` does).
pub fn real(v: &Value) -> Option<f64> {
    match v {
        Value::Real(r) => Some(*r),
        Value::Float(f) => Some(*f as f64),
        Value::Int(i) => Some(*i as f64),
        Value::Boxed(_, i) => Some(*i as f64),
        _ => None,
    }
}

/// A `java.lang.Boolean` argument.
pub fn boolean(v: &Value) -> Option<bool> {
    v.as_bool()
}

/// A `java.lang.Long` result (`Long` box in `int` range, else the OCL Integer).
pub fn long(i: i64) -> Value {
    if i32::try_from(i).is_ok() {
        Value::Boxed(IntBox::Long, i as i32)
    } else {
        Value::Int(i)
    }
}

/// `String.indexOf(str, fromIndex)` on UTF-16 code units.
pub fn index_of(h: &[u16], n: &[u16], from: i64) -> i64 {
    let from = from.max(0) as usize;
    if from >= h.len() {
        return if n.is_empty() { h.len() as i64 } else { -1 };
    }
    if n.len() > h.len() {
        return -1;
    }
    (from..=h.len() - n.len())
        .find(|&i| &h[i..i + n.len()] == n)
        .map_or(-1, |i| i as i64)
}

/// `String.lastIndexOf(str, fromIndex)` on UTF-16 code units.
pub fn last_index_of(h: &[u16], n: &[u16], from: i64) -> i64 {
    if from < 0 || n.len() > h.len() {
        return -1;
    }
    let right = h.len() - n.len();
    let start = (from as usize).min(right);
    (0..=start)
        .rev()
        .find(|&i| &h[i..i + n.len()] == n)
        .map_or(-1, |i| i as i64)
}

/// `String.startsWith(prefix, toffset)` on UTF-16 code units.
pub fn starts_with(h: &[u16], p: &[u16], offset: i64) -> bool {
    if offset < 0 || offset as usize > h.len().saturating_sub(p.len()) || p.len() > h.len() {
        return false;
    }
    let o = offset as usize;
    &h[o..o + p.len()] == p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn java_string_search() {
        assert_eq!(index_of(&u("abcabc"), &u("bc"), 0), 1);
        assert_eq!(index_of(&u("abcabc"), &u("bc"), 2), 4);
        assert_eq!(index_of(&u("abc"), &u(""), 10), 3);
        assert_eq!(index_of(&u("abc"), &u("x"), -5), -1);
        assert_eq!(last_index_of(&u("abcabc"), &u("bc"), 10), 4);
        assert_eq!(last_index_of(&u("abcabc"), &u("bc"), 3), 1);
        assert_eq!(last_index_of(&u("abc"), &u("a"), -1), -1);
        assert!(starts_with(&u("abc"), &u("bc"), 1));
        assert!(!starts_with(&u("abc"), &u("bc"), 2));
        assert!(!starts_with(&u("abc"), &u("a"), -1));
    }
}

//! OCL and QVTo string operations on values (classic OCL `EvaluationVisitorImpl` and QVTo's
//! `StringOperations`), with Eclipse's results for out-of-range indices, absent substrings and
//! unparsable numbers. Arguments are the evaluated operands; null and invalid sources are the
//! caller's business (they give invalid before any of these is reached).

use crate::coll::Coll;
use crate::java::{chars, parse, string as js};
use crate::str::Str;
use crate::value::Value;

fn s(v: Str) -> Value {
    Value::Str(v)
}

/// `size()` / QVTo `length()`: UTF-16 code units.
pub fn size(a: &Str) -> Value {
    Value::Int(a.len16() as i64)
}

/// `concat` and `+`.
pub fn concat(a: &Str, b: &Str) -> Value {
    s(a.concat(b))
}

/// `substring(lower, upper)`: 1-based, both inclusive; invalid unless `1 <= lower <= upper <= size`.
pub fn substring(a: &Str, lower: i64, upper: i64) -> Value {
    if !(1 <= lower && lower <= upper && upper <= a.len16() as i64) {
        return Value::Invalid;
    }
    js::substring(a, (lower - 1) as usize, upper as usize).map_or(Value::Invalid, s)
}

/// `at(i)`: the 1-based `i`-th code unit as a string.
pub fn at(a: &Str, i: i64) -> Value {
    if i < 1 || i > a.len16() as i64 {
        return Value::Invalid;
    }
    js::substring(a, (i - 1) as usize, i as usize).map_or(Value::Invalid, s)
}

/// `characters()`: one string per UTF-16 code unit (surrogate pairs split).
pub fn characters(a: &Str) -> Value {
    Coll::Sequence(
        a.units()
            .map(|u| Value::Str(Str::from_utf16(&[u])))
            .collect(),
    )
    .into()
}

pub fn to_upper(a: &Str) -> Value {
    s(js::to_upper_case(a))
}

pub fn to_lower(a: &Str) -> Value {
    s(js::to_lower_case(a))
}

/// `indexOf`: 1-based, 0 if absent (`indexOf('')` is 1).
pub fn index_of(a: &Str, b: &Str) -> Value {
    Value::Int(1 + js::index_of(a, b) as i64)
}

/// `lastIndexOf`: 1-based, 0 if absent.
pub fn last_index_of(a: &Str, b: &Str) -> Value {
    Value::Int(1 + js::last_index_of(a, b) as i64)
}

pub fn starts_with(a: &Str, b: &Str) -> Value {
    Value::Bool(js::starts_with(a, b))
}

pub fn ends_with(a: &Str, b: &Str) -> Value {
    Value::Bool(js::ends_with(a, b))
}

pub fn equals_ignore_case(a: &Str, b: &Str) -> Value {
    Value::Bool(js::equals_ignore_case(a, b))
}

pub fn trim(a: &Str) -> Value {
    s(js::trim(a))
}

/// `tokenize()`, `tokenize(delims)`, `tokenize(delims, returnDelims)`: a Sequence.
pub fn tokenize(a: &Str, delims: Option<&Str>, return_delims: bool) -> Value {
    let d = delims
        .cloned()
        .unwrap_or_else(|| Str::new(js::DEFAULT_DELIMS));
    Coll::Sequence(
        js::tokenize(a, &d, return_delims)
            .into_iter()
            .map(Value::Str)
            .collect(),
    )
    .into()
}

/// `substituteAll` (literal replace) and QVTo's `replace`.
pub fn substitute_all(a: &Str, old: &Str, new: &Str) -> Value {
    s(js::replace(a, old, new))
}

/// `substituteFirst`: invalid when `old` does not occur.
pub fn substitute_first(a: &Str, old: &Str, new: &Str) -> Value {
    let i = js::index_of(a, old);
    if i < 0 {
        return Value::Invalid;
    }
    let i = i as usize;
    let head = js::substring(a, 0, i).expect("in range");
    let tail = js::substring(a, i + old.len16(), a.len16()).expect("in range");
    s(head.concat(new).concat(&tail))
}

/// `toBoolean()`: `"true".equals(self)`.
pub fn to_boolean(a: &Str) -> Value {
    Value::Bool(a.as_bytes() == b"true")
}

/// `toInteger()`: `Integer.valueOf` (32 bits even with long literals); invalid on failure.
pub fn to_integer(a: &Str) -> Value {
    parse::parse_int(&a.to_utf16()).map_or(Value::Invalid, |i| Value::Int(i as i64))
}

/// `toReal()`: `Double.valueOf`; invalid on failure.
pub fn to_real(a: &Str) -> Value {
    parse::parse_double(&a.to_utf16()).map_or(Value::Invalid, Value::Real)
}

/// `<`, `<=`, `>`, `>=` by `compareTo`.
pub fn compare(a: &Str, b: &Str) -> std::cmp::Ordering {
    crate::eq::compare_str(a, b)
}

// ---------------------------------------------------------------- QVTo (StringOperations)

/// `substringBefore(m)`: null if `m` does not occur.
pub fn substring_before(a: &Str, m: &Str) -> Value {
    let i = js::index_of(a, m);
    if i < 0 {
        return Value::Null;
    }
    js::substring(a, 0, i as usize).map_or(Value::Invalid, s)
}

/// `substringAfter(m)`: null if `m` does not occur.
pub fn substring_after(a: &Str, m: &Str) -> Value {
    let i = js::index_of(a, m);
    if i < 0 {
        return Value::Null;
    }
    js::substring(a, i as usize + m.len16(), a.len16()).map_or(Value::Invalid, s)
}

fn map_unit(a: &Str, pos: usize) -> Value {
    if a.is_empty() {
        return s(a.clone());
    }
    let mut u = a.to_utf16();
    u[pos] = chars::to_upper_unit(u[pos]);
    s(Str::from_utf16(&u))
}

/// `firstToUpper()`: `Character.toUpperCase(char)` of the first code unit.
pub fn first_to_upper(a: &Str) -> Value {
    map_unit(a, 0)
}

/// `lastToUpper()`.
pub fn last_to_upper(a: &Str) -> Value {
    if a.is_empty() {
        return s(a.clone());
    }
    map_unit(a, a.len16() - 1)
}

/// `normalizeSpace()`: `trim()`, then each run of `Character.isWhitespace` code units is
/// replaced by its first unit.
pub fn normalize_space(a: &Str) -> Value {
    let t = js::trim(a);
    let mut out = Vec::with_capacity(t.len16());
    let mut white = false;
    for u in t.units() {
        if chars::is_whitespace(u) {
            if white {
                continue;
            }
            white = true;
        } else {
            white = false;
        }
        out.push(u);
    }
    s(Str::from_utf16(&out))
}

/// `find(m)`: 1-based index, 0 if absent.
pub fn find(a: &Str, m: &Str) -> Value {
    index_of(a, m)
}

/// `rfind(m)`.
pub fn rfind(a: &Str, m: &Str) -> Value {
    last_index_of(a, m)
}

/// `isQuoted(q)`.
pub fn is_quoted(a: &Str, q: &Str) -> Value {
    Value::Bool(a.len16() >= 2 * q.len16() && js::starts_with(a, q) && js::ends_with(a, q))
}

/// `quotify(q)`.
pub fn quotify(a: &Str, q: &Str) -> Value {
    s(q.concat(a).concat(q))
}

/// `unquotify(q)`.
pub fn unquotify(a: &Str, q: &Str) -> Value {
    if !is_quoted(a, q).is_true() {
        return s(a.clone());
    }
    js::substring(a, q.len16(), a.len16() - q.len16()).map_or(Value::Invalid, s)
}

/// `asBoolean()`: `"1"` or `true` (any case) is true, `"0"` or `false` false, else null.
pub fn as_boolean(a: &Str) -> Value {
    if a.as_bytes() == b"1" || js::equals_ignore_case(a, &Str::new("true")) {
        Value::Bool(true)
    } else if a.as_bytes() == b"0" || js::equals_ignore_case(a, &Str::new("false")) {
        Value::Bool(false)
    } else {
        Value::Null
    }
}

/// `asInteger()`: `Integer.valueOf`, null on failure.
pub fn as_integer(a: &Str) -> Value {
    parse::parse_int(&a.to_utf16()).map_or(Value::Null, |i| Value::Int(i as i64))
}

/// `asFloat()`: `Double.valueOf`, null on failure.
pub fn as_float(a: &Str) -> Value {
    parse::parse_double(&a.to_utf16()).map_or(Value::Null, Value::Real)
}

/// `matchIdentifier(_)`: the argument is ignored; true iff the first code unit is a letter and
/// all are letters or digits (`""` is true).
pub fn match_identifier(a: &Str) -> Value {
    Value::Bool(
        a.units()
            .enumerate()
            .all(|(i, u)| (i > 0 || chars::is_letter(u)) && chars::is_letter_or_digit(u)),
    )
}

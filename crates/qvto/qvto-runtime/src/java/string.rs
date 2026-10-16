//! `java.lang.String` operations with Java's semantics: UTF-16 code unit indices, `compareTo`
//! by code units, case mapping with the root locale (Eclipse's runner and QVTo's tests run
//! with `en_US`, which maps like the root locale), `trim` of characters `<= U+0020`, literal
//! `replace`, `StringTokenizer`. Ported behaviour of the OCL and QVTo string operations built
//! on them is in [`crate::strings`].
//!
//! Indices are `usize` code unit offsets; functions that throw in Java return `None`.

use super::chars;
use crate::str::Str;

/// The code units, without copying for ASCII strings' callers that only need indices.
fn u16s(s: &Str) -> Vec<u16> {
    s.to_utf16()
}

/// `charAt(i)`.
pub fn char_at(s: &Str, i: usize) -> Option<u16> {
    if s.is_ascii() {
        return s.as_bytes().get(i).map(|&b| b as u16);
    }
    s.units().nth(i)
}

/// `substring(begin, end)` (0-based, end exclusive).
pub fn substring(s: &Str, begin: usize, end: usize) -> Option<Str> {
    if begin > end || end > s.len16() {
        return None;
    }
    if begin == 0 && end == s.len16() {
        return Some(s.clone());
    }
    if s.is_ascii() {
        return Some(Str::new(&s.as_str().expect("ascii")[begin..end]));
    }
    let u = u16s(s);
    Some(Str::from_utf16(&u[begin..end]))
}

fn find(h: &[u16], n: &[u16], from: usize) -> Option<usize> {
    if n.is_empty() {
        return if from <= h.len() { Some(from) } else { None };
    }
    if n.len() > h.len() {
        return None;
    }
    (from..=h.len() - n.len()).find(|&i| &h[i..i + n.len()] == n)
}

/// `indexOf(t)`: -1 if absent; 0 for the empty string.
pub fn index_of(s: &Str, t: &Str) -> i32 {
    if s.is_ascii() && t.is_ascii() {
        let (h, n) = (s.as_bytes(), t.as_bytes());
        if n.is_empty() {
            return 0;
        }
        return h
            .windows(n.len())
            .position(|w| w == n)
            .map_or(-1, |i| i as i32);
    }
    find(&u16s(s), &u16s(t), 0).map_or(-1, |i| i as i32)
}

/// `lastIndexOf(t)`: -1 if absent; the length for the empty string.
pub fn last_index_of(s: &Str, t: &Str) -> i32 {
    let (h, n) = (u16s(s), u16s(t));
    if n.len() > h.len() {
        return -1;
    }
    (0..=h.len() - n.len())
        .rev()
        .find(|&i| h[i..i + n.len()] == n[..])
        .map_or(-1, |i| i as i32)
}

pub fn starts_with(s: &Str, t: &Str) -> bool {
    // well-formed text: a UTF-8 byte prefix is exactly a UTF-16 prefix (it ends on a character)
    if !t.has_lone_surrogates() && !s.has_lone_surrogates() {
        return s.as_bytes().starts_with(t.as_bytes());
    }
    let (h, n) = (u16s(s), u16s(t));
    h.starts_with(&n)
}

pub fn ends_with(s: &Str, t: &Str) -> bool {
    if !t.has_lone_surrogates() && !s.has_lone_surrogates() {
        return s.as_bytes().ends_with(t.as_bytes());
    }
    let (h, n) = (u16s(s), u16s(t));
    h.ends_with(&n)
}

/// `trim()`: strips code units `<= U+0020` at both ends.
pub fn trim(s: &Str) -> Str {
    let b = s.as_bytes();
    // all trimmed characters are ASCII, so byte trimming is exact
    let mut a = 0;
    let mut e = b.len();
    while a < e && b[a] <= 0x20 {
        a += 1;
    }
    while e > a && b[e - 1] <= 0x20 {
        e -= 1;
    }
    if a == 0 && e == b.len() {
        return s.clone();
    }
    let u: Vec<u16> = s.units().collect();
    let lead = a; // ASCII bytes == units
    let trail = b.len() - e;
    Str::from_utf16(&u[lead..u.len() - trail])
}

/// `String.toUpperCase(Locale.ROOT)`.
pub fn to_upper_case(s: &Str) -> Str {
    if s.is_ascii() {
        return Str::new(&s.as_str().expect("ascii").to_ascii_uppercase());
    }
    let mut out: Vec<u16> = Vec::with_capacity(s.len16());
    for cp in s.code_points() {
        if let Some(sp) = chars::upper_special(cp) {
            out.extend_from_slice(sp);
        } else {
            push_cp(&mut out, chars::to_upper_cp(cp));
        }
    }
    Str::from_utf16(&out)
}

/// `String.toLowerCase(Locale.ROOT)`: simple mappings, `İ` to `i̇`, and the final-sigma rule
/// for `Σ` (Java decides it with word boundaries; this uses Unicode's Final_Sigma context, see
/// the differential tests for the agreement).
pub fn to_lower_case(s: &Str) -> Str {
    if s.is_ascii() {
        return Str::new(&s.as_str().expect("ascii").to_ascii_lowercase());
    }
    let cps: Vec<u32> = s.code_points().collect();
    let mut out: Vec<u16> = Vec::with_capacity(s.len16());
    for (i, &cp) in cps.iter().enumerate() {
        if cp == 0x3a3 {
            push_cp(
                &mut out,
                if is_final_sigma(&cps, i) {
                    0x3c2
                } else {
                    0x3c3
                },
            );
        } else if let Some(sp) = chars::lower_special(cp) {
            out.extend_from_slice(sp);
        } else {
            push_cp(&mut out, chars::to_lower_cp(cp));
        }
    }
    Str::from_utf16(&out)
}

fn is_final_sigma(cps: &[u32], i: usize) -> bool {
    let before = cps[..i]
        .iter()
        .rev()
        .find(|&&c| !chars::is_case_ignorable(c))
        .is_some_and(|&c| chars::is_cased(c));
    let after = cps[i + 1..]
        .iter()
        .find(|&&c| !chars::is_case_ignorable(c))
        .is_some_and(|&c| chars::is_cased(c));
    before && !after
}

fn push_cp(out: &mut Vec<u16>, cp: u32) {
    if cp >= 0x10000 {
        let c = cp - 0x10000;
        out.push(0xd800 | (c >> 10) as u16);
        out.push(0xdc00 | (c & 0x3ff) as u16);
    } else {
        out.push(cp as u16);
    }
}

/// `equalsIgnoreCase`: same length in code units and, code point by code point, equal, or equal
/// after `Character.toUpperCase`, or after `toLowerCase(toUpperCase(..))`.
pub fn equals_ignore_case(a: &Str, b: &Str) -> bool {
    if a.len16() != b.len16() {
        return false;
    }
    let mut x = a.code_points();
    let mut y = b.code_points();
    loop {
        match (x.next(), y.next()) {
            (None, None) => return true,
            (Some(c1), Some(c2)) => {
                if c1 == c2 {
                    continue;
                }
                let (u1, u2) = (chars::to_upper_cp(c1), chars::to_upper_cp(c2));
                if u1 == u2 || chars::to_lower_cp(u1) == chars::to_lower_cp(u2) {
                    continue;
                }
                return false;
            }
            _ => return false,
        }
    }
}

/// `replace(CharSequence target, CharSequence replacement)`: literal, every occurrence left to
/// right; an empty target inserts the replacement around every code unit.
pub fn replace(s: &Str, target: &Str, repl: &Str) -> Str {
    let (h, t, r) = (u16s(s), u16s(target), u16s(repl));
    let mut out = Vec::with_capacity(h.len());
    if t.is_empty() {
        out.extend_from_slice(&r);
        for &u in &h {
            out.push(u);
            out.extend_from_slice(&r);
        }
        return Str::from_utf16(&out);
    }
    let mut i = 0;
    while let Some(j) = find(&h, &t, i) {
        out.extend_from_slice(&h[i..j]);
        out.extend_from_slice(&r);
        i = j + t.len();
    }
    if i == 0 {
        return s.clone();
    }
    out.extend_from_slice(&h[i..]);
    Str::from_utf16(&out)
}

/// `new StringTokenizer(s, delims, returnDelims)` tokens (delimiters are a set of code points).
pub fn tokenize(s: &Str, delims: &Str, return_delims: bool) -> Vec<Str> {
    let d: Vec<u32> = delims.code_points().collect();
    let cps: Vec<u32> = s.code_points().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let to_str = |c: &[u32]| {
        let mut u = Vec::new();
        for &cp in c {
            push_cp(&mut u, cp);
        }
        Str::from_utf16(&u)
    };
    while i < cps.len() {
        if d.contains(&cps[i]) {
            if return_delims {
                out.push(to_str(&cps[i..i + 1]));
            }
            i += 1;
            continue;
        }
        let start = i;
        while i < cps.len() && !d.contains(&cps[i]) {
            i += 1;
        }
        out.push(to_str(&cps[start..i]));
    }
    out
}

/// The default delimiters of OCL's `tokenize()`: `" \t\n\r\f"`.
pub const DEFAULT_DELIMS: &str = " \t\n\r\u{c}";

#[cfg(test)]
mod tests {
    use super::*;

    fn s(x: &str) -> Str {
        Str::new(x)
    }

    #[test]
    fn basics() {
        assert_eq!(index_of(&s("hello"), &s("l")), 2);
        assert_eq!(index_of(&s("hello"), &s("")), 0);
        assert_eq!(last_index_of(&s("hello"), &s("")), 5);
        assert_eq!(index_of(&s("a\u{1F600}b"), &s("b")), 3);
        assert_eq!(
            substring(&s("a\u{1F600}b"), 0, 2).unwrap().to_utf16(),
            vec![0x61, 0xd83d]
        );
        assert_eq!(trim(&s(" \u{0}x y\t")).as_str(), Some("x y"));
        assert_eq!(to_upper_case(&s("straße")).as_str(), Some("STRASSE"));
        assert_eq!(to_lower_case(&s("ΟΔΟΣ")).as_str(), Some("οδος"));
        assert_eq!(to_lower_case(&s("ΣΑ")).as_str(), Some("σα"));
        assert_eq!(
            replace(&s("abc"), &s(""), &s("-")).as_str(),
            Some("-a-b-c-")
        );
        assert_eq!(replace(&s("aaa"), &s("aa"), &s("b")).as_str(), Some("ba"));
        assert!(equals_ignore_case(&s("Straße"), &s("STRAßE")));
        let t: Vec<String> = tokenize(&s(" a  b,c"), &s(" ,"), false)
            .iter()
            .map(|x| x.to_string())
            .collect();
        assert_eq!(t, vec!["a", "b", "c"]);
        let t: Vec<String> = tokenize(&s("a,b"), &s(","), true)
            .iter()
            .map(|x| x.to_string())
            .collect();
        assert_eq!(t, vec!["a", ",", "b"]);
    }
}

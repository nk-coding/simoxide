//! Canonical dump: the strict model comparison of qvto-rs. Two sets of resources are equal when
//! their dumps are equal: same objects in the same containment order, same classes, every
//! persistent attribute value (defaults included), every persistent cross reference pointing
//! at the object with the same canonical path, the same "set" state of unsettable features.
//!
//! The Java side (`oracle/src/CanonDump.java`, and later the reference runner) writes the same
//! format from real EMF. Format: one JSON line per resource and per object:
//!
//! ```text
//! {"resource":"<label>","roots":N}
//! {"path":"/0/@f.1","type":"<nsURI>#<Class>","attrs":{...},"refs":{...},"set":[...]}
//! ```
//!
//! Generic over [`ModelStore`], so any store can be dumped.

use crate::uri::{decode, Uri};
use qvto_model::{Datum, FeatureKind, ModelStore, ResourceId};
use std::fmt::Write;

/// The label of a resource: the decoded last segment of a `file:` URI, else the URI.
pub fn label(uri: &str) -> String {
    let u = Uri::parse(uri);
    if u.is_file()
        && let Some(l) = u.last_segment()
    {
        return decode(l);
    }
    uri.to_string()
}

/// The canonical path of an object: `/<root index>` then `@feature.index` / `@feature`.
pub fn path<S: ModelStore>(s: &S, o: S::Obj) -> String {
    let mut segs = Vec::new();
    let mut cur = o;
    while s.direct_resource(cur).is_none() {
        let Some((p, f)) = s.container(cur) else { break };
        let fi = s.meta().feature(f);
        if fi.is_many() {
            let i = (0..s.len(p, f)).position(|i| s.get_at(p, f, i) == Datum::Obj(cur));
            segs.push(format!("@{}.{}", fi.name, i.map_or(-1, |i| i as i64)));
        } else {
            segs.push(format!("@{}", fi.name));
        }
        cur = p;
    }
    let mut out = String::from("/");
    if let Some(r) = s.direct_resource(cur) {
        let i = s.roots(r).iter().position(|x| *x == cur).map_or(-1, |i| i as i64);
        let _ = write!(out, "{i}");
    }
    for seg in segs.iter().rev() {
        out.push('/');
        out.push_str(seg);
    }
    out
}

/// How a reference target appears in the dump.
pub fn target<S: ModelStore>(s: &S, t: S::Obj) -> String {
    if let Some(p) = s.proxy_uri(t) {
        let u = Uri::parse(p);
        let trimmed = u.trim_fragment();
        let last = trimmed.last_segment().map(decode).unwrap_or_else(|| trimmed.to_string());
        return format!("?{last}#{}", u.fragment().unwrap_or("null"));
    }
    match s.resource_of(t) {
        None => format!("!{}", path(s, t)),
        Some(r) => format!("{}#{}", label(s.resource_uri(r)), path(s, t)),
    }
}

fn type_name<S: ModelStore>(s: &S, o: S::Obj) -> String {
    let meta = s.meta();
    let ci = meta.class(s.class_of(o));
    let p = meta.package(ci.package);
    let ns = p.ns_uri.as_deref().unwrap_or(&p.name);
    format!("{ns}#{}", ci.name)
}

pub fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn value<O: Copy>(out: &mut String, meta: &qvto_model::MetaRegistry, d: Datum<'_, O>) {
    match crate::datatypes::datum_to_string(meta, d) {
        None => out.push_str("null"),
        Some(s) => json_str(out, &s),
    }
}

/// One object's line (without the newline).
pub fn object_line<S: ModelStore>(s: &S, o: S::Obj) -> String {
    let meta = s.meta();
    let class = meta.class(s.class_of(o));
    let mut out = String::from("{\"path\":");
    json_str(&mut out, &path(s, o));
    out.push_str(",\"type\":");
    json_str(&mut out, &type_name(s, o));
    out.push_str(",\"attrs\":{");
    let mut first = true;
    let mut set = Vec::new();
    for &f in class.all_features.iter() {
        let fi = meta.feature(f);
        if fi.transient || fi.derived {
            continue;
        }
        if fi.unsettable && s.is_set(o, f) {
            set.push(fi.name.to_string());
        }
        if fi.kind != FeatureKind::Attribute {
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        json_str(&mut out, &fi.name);
        out.push(':');
        if fi.is_many() {
            out.push('[');
            for i in 0..s.len(o, f) {
                if i > 0 {
                    out.push(',');
                }
                value(&mut out, meta, s.get_at(o, f, i));
            }
            out.push(']');
        } else {
            value(&mut out, meta, s.get(o, f));
        }
    }
    out.push_str("},\"refs\":{");
    first = true;
    for &f in class.all_features.iter() {
        let fi = meta.feature(f);
        if fi.kind != FeatureKind::Reference || fi.containment || meta.is_container(f) || fi.transient || fi.derived {
            continue;
        }
        if fi.is_many() {
            let n = s.len(o, f);
            if n == 0 {
                continue;
            }
            if !first {
                out.push(',');
            }
            first = false;
            json_str(&mut out, &fi.name);
            out.push_str(":[");
            for i in 0..n {
                if i > 0 {
                    out.push(',');
                }
                match s.get_at(o, f, i) {
                    Datum::Obj(t) => json_str(&mut out, &target(s, t)),
                    _ => out.push_str("null"),
                }
            }
            out.push(']');
        } else if let Datum::Obj(t) = s.get(o, f) {
            if !first {
                out.push(',');
            }
            first = false;
            json_str(&mut out, &fi.name);
            out.push(':');
            json_str(&mut out, &target(s, t));
        }
    }
    out.push('}');
    if !set.is_empty() {
        out.push_str(",\"set\":[");
        for (i, n) in set.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            json_str(&mut out, n);
        }
        out.push(']');
    }
    out.push('}');
    out
}

/// The canonical dump of the given resources (in the given order).
pub fn dump<S: ModelStore>(s: &S, resources: &[ResourceId]) -> String {
    let mut out = String::new();
    for &r in resources {
        out.push_str("{\"resource\":");
        json_str(&mut out, &label(s.resource_uri(r)));
        let _ = writeln!(out, ",\"roots\":{}}}", s.roots(r).len());
        for &root in s.roots(r) {
            out.push_str(&object_line(s, root));
            out.push('\n');
            for o in s.all_contents(root) {
                out.push_str(&object_line(s, o));
                out.push('\n');
            }
        }
    }
    out
}

/// The first difference between two dumps: `None` if equal, else the line number and both
/// lines (the object path and the differing feature can be read from them).
pub fn first_difference(expected: &str, actual: &str) -> Option<(usize, String, String)> {
    let mut a = expected.lines();
    let mut b = actual.lines();
    let mut n = 1;
    loop {
        match (a.next(), b.next()) {
            (None, None) => return None,
            (x, y) if x == y => {}
            (x, y) => return Some((n, x.unwrap_or("<end>").to_string(), y.unwrap_or("<end>").to_string())),
        }
        n += 1;
    }
}

/// FNV-1a 64 of a text (the oracle's hash of saves and dumps).
pub fn fnv64(b: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for &x in b {
        h ^= x as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// The object at a canonical path (`/0/@f.1/@g`) of resource `r`.
pub fn object_at<S: ModelStore>(s: &S, r: ResourceId, path: &str) -> Option<S::Obj> {
    let mut segs = path.split('/').skip(1);
    let mut cur = *s.roots(r).get(segs.next()?.parse::<usize>().ok()?)?;
    for seg in segs {
        let body = seg.strip_prefix('@')?;
        let (name, idx) = match body.rsplit_once('.') {
            Some((n, i)) if !i.is_empty() && i.bytes().all(|c| c.is_ascii_digit()) => (n, Some(i.parse::<usize>().ok()?)),
            _ => (body, None),
        };
        let f = s.meta().feature_by_name(s.class_of(cur), name)?;
        cur = match idx {
            Some(i) => s.get_at(cur, f, i).as_obj()?,
            None => s.get(cur, f).as_obj()?,
        };
    }
    Some(cur)
}

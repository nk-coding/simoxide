//! URI fragments: how an object is addressed within its resource (`Resource.getURIFragment`,
//! `Resource.getEObject`). IDs win over paths; paths use `@feature.index` segments, `eKeys`
//! predicates, and for the contents of Ecore model elements the named segments of
//! `EModelElementImpl` (`//Class/feature`, `%source%` for annotations).
//!
//! Ported from EMF (`ResourceImpl`, `XMLResourceImpl`, `BasicEObjectImpl`, `EModelElementImpl`;
//! EPL-2.0, see `LICENSES`).

use crate::datatypes::value_to_string;
use crate::graph::{Graph, ObjId};
use crate::uri::decode;
use qvto_model::{Datum, FeatureId, ModelStore, ResourceId};

/// `EcoreUtil.getID`: the value of the class's ID attribute if it is set.
pub fn id_of(g: &Graph, o: ObjId) -> Option<String> {
    let f = g.meta().class(g.class_of(o)).id_attribute?;
    if !g.is_set(o, f) {
        return None;
    }
    value_to_string(g.meta(), &g.get(o, f).to_owned())
}

/// `XMLResourceImpl.getURIFragment(o)` for the resource `r` (normally `o`'s resource).
pub fn uri_fragment(g: &Graph, r: ResourceId, o: ObjId) -> String {
    if let Some(id) = g.resource(r).xmi_id(o) {
        return id.to_string();
    }
    if let Some(id) = id_of(g, o) {
        return id;
    }
    path_fragment(g, r, o)
}

/// The path form of the fragment (`/`, `/1`, `//@a.0/@b`, `//Name/feature`), or `/-1` when the
/// object is not in `r`.
pub fn path_fragment(g: &Graph, r: ResourceId, o: ObjId) -> String {
    let root_segment = |x: ObjId| {
        let roots = &g.resource(r).roots;
        if roots.len() > 1 {
            roots.iter().position(|y| *y == x).map_or("-1".to_string(), |i| i.to_string())
        } else {
            String::new()
        }
    };
    if g.direct_resource(o) == Some(r) {
        return format!("/{}", root_segment(o));
    }
    let mut segs = Vec::new();
    let mut cur = o;
    loop {
        let Some((p, f)) = g.container(cur) else {
            return "/-1".to_string();
        };
        segs.push(segment(g, p, f, cur));
        cur = p;
        if g.direct_resource(p) == Some(r) {
            break;
        }
    }
    segs.push(root_segment(cur));
    segs.reverse();
    format!("/{}", segs.join("/"))
}

const ESCAPED: &str = " \"#%&',/:<>";

/// `EModelElementImpl.eEncodeValue`.
fn encode_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if (ch as u32) < 32 || ESCAPED.contains(ch) {
            out.push_str(&format!("%{:02X}", ch as u32));
        } else {
            out.push(ch);
        }
    }
    out
}

fn str_attr(g: &Graph, o: ObjId, f: FeatureId) -> Option<String> {
    g.get(o, f).as_str().map(|s| s.to_string())
}

/// `eURIFragmentSegment(feature, child)` of `parent`.
pub fn segment(g: &Graph, parent: ObjId, f: FeatureId, child: ObjId) -> String {
    let e = g.ecore();
    let meta = g.meta();
    if meta.is_subclass(g.class_of(parent), e.emodel_element) {
        let child_class = g.class_of(child);
        if meta.is_subclass(child_class, e.enamed_element) {
            let name = str_attr(g, child, e.named_element_name);
            let mut count = 0;
            for x in g.contents(parent) {
                if x == child {
                    break;
                }
                if meta.is_subclass(g.class_of(x), e.enamed_element) && str_attr(g, x, e.named_element_name) == name {
                    count += 1;
                }
            }
            let n = name.map_or("%".to_string(), |n| encode_name(&n));
            return if count > 0 { format!("{n}.{count}") } else { n };
        }
        if meta.is_subclass(child_class, e.eannotation) {
            let source = str_attr(g, child, e.annotation_source);
            let mut count = 0;
            for x in g.contents(parent) {
                if x == child {
                    break;
                }
                if meta.is_subclass(g.class_of(x), e.eannotation) && str_attr(g, x, e.annotation_source) == source {
                    count += 1;
                }
            }
            let s = source.map_or("%".to_string(), |s| encode_segment(&s));
            return if count > 0 { format!("%{s}%.{count}") } else { format!("%{s}%") };
        }
    }
    let fi = meta.feature(f);
    let mut s = format!("@{}", fi.name);
    if fi.is_many() {
        if fi.keys.is_empty() {
            let i = (0..g.len(parent, f)).position(|i| g.get_at(parent, f, i) == Datum::Obj(child));
            s.push_str(&format!(".{}", i.map_or(-1, |i| i as i64)));
        } else {
            s.push('[');
            for (k, &a) in fi.keys.iter().enumerate() {
                if k > 0 {
                    s.push(',');
                }
                s.push_str(&meta.feature(a).name);
                s.push('=');
                if meta.feature(a).is_many() {
                    s.push('[');
                    for i in 0..g.len(child, a) {
                        if i > 0 {
                            s.push(',');
                        }
                        encode_value(&mut s, value_to_string(meta, &g.get_at(child, a, i).to_owned()));
                    }
                    s.push(']');
                } else {
                    encode_value(&mut s, value_to_string(meta, &g.get(child, a).to_owned()));
                }
            }
            s.push(']');
        }
    }
    s
}

fn encode_value(out: &mut String, v: Option<String>) {
    match v {
        None => out.push_str("null"),
        Some(v) => {
            out.push('\'');
            out.push_str(&encode_name(&v));
            out.push('\'');
        }
    }
}

/// `URI.encodeSegment(s, false)`.
pub fn encode_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let keep = ch.is_ascii_alphanumeric()
            || matches!(ch, '-' | '_' | '.' | '!' | '~' | '*' | '\'' | '(' | ')' | ';' | ':' | '@' | '&' | '=' | '+' | '$' | ',');
        if keep {
            out.push(ch);
        } else {
            let mut buf = [0u8; 4];
            for b in ch.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// `Resource.getEObject(fragment)`.
pub fn resolve_fragment(g: &Graph, r: ResourceId, frag: &str) -> Option<ObjId> {
    if let Some(path) = frag.strip_prefix('/') {
        let mut segs = path.split('/');
        let root = segs.next().unwrap_or("");
        let roots = &g.resource(r).roots;
        let mut cur = if let Some(id) = root.strip_prefix('?') {
            by_id(g, r, id)?
        } else {
            let i: usize = if root.is_empty() { 0 } else { root.parse().ok()? };
            *roots.get(i)?
        };
        for s in segs {
            cur = child_by_segment(g, cur, s)?;
        }
        return Some(cur);
    }
    let mut id = frag;
    if id.ends_with('?')
        && let Some(i) = id[..id.len() - 1].rfind('?')
        && i > 0
    {
        id = &id[..i];
    }
    by_id(g, r, id)
}

/// `getEObjectByID`: the `xmi:id` map, then the first object in tree order with that ID.
pub fn by_id(g: &Graph, r: ResourceId, id: &str) -> Option<ObjId> {
    if let Some(maps) = &g.resource(r).ids
        && let Some(o) = maps.to_obj.get(id)
    {
        return Some(*o);
    }
    let class_has_id = |o: ObjId| g.meta().class(g.class_of(o)).id_attribute.is_some();
    g.resource_contents(r)
        .into_iter()
        .find(|&o| class_has_id(o) && id_of(g, o).as_deref() == Some(id))
}

/// `eObjectForURIFragmentSegment`.
pub fn child_by_segment(g: &Graph, o: ObjId, s: &str) -> Option<ObjId> {
    let e = g.ecore();
    let meta = g.meta();
    if !s.is_empty() && !s.starts_with('@') && meta.is_subclass(g.class_of(o), e.emodel_element) {
        if s.starts_with('%') {
            let idx = s.rfind('%')?;
            let rest = &s[idx + 1..];
            if idx != 0 && (rest.is_empty() || rest.starts_with('.')) {
                let enc = &s[1..idx];
                let source = if enc == "%" { None } else { Some(decode(enc)) };
                let mut count: usize = if rest.is_empty() { 0 } else { rest[1..].parse().ok()? };
                for x in g.contents(o) {
                    if meta.is_subclass(g.class_of(x), e.eannotation) && str_attr(g, x, e.annotation_source) == source {
                        if count == 0 {
                            return Some(x);
                        }
                        count -= 1;
                    }
                }
                return None;
            }
        }
        let (name, mut count) = match s.rfind('.') {
            Some(i) => match s[i + 1..].parse::<usize>() {
                Ok(c) => (&s[..i], c),
                Err(_) => (s, 0),
            },
            None => (s, 0),
        };
        let name = if name == "%" { None } else { Some(decode(name)) };
        for x in g.contents(o) {
            if meta.is_subclass(g.class_of(x), e.enamed_element) && str_attr(g, x, e.named_element_name) == name {
                if count == 0 {
                    return Some(x);
                }
                count -= 1;
            }
        }
        return None;
    }
    let body = s.strip_prefix('@')?;
    if body.ends_with(']') {
        let i = body.find('[')?;
        let f = meta.feature_by_name(g.class_of(o), &body[..i])?;
        let pred = &body[i + 1..body.len() - 1];
        return (0..g.len(o, f)).filter_map(|k| g.get_at(o, f, k).as_obj()).find(|&x| matches_predicate(g, x, pred));
    }
    if let Some(dot) = body.rfind('.')
        && body[dot + 1..].bytes().all(|c| c.is_ascii_digit())
        && dot + 1 < body.len()
    {
        let f = meta.feature_by_name(g.class_of(o), &body[..dot])?;
        let i: usize = body[dot + 1..].parse().ok()?;
        return g.get_at(o, f, i).as_obj();
    }
    let f = meta.feature_by_name(g.class_of(o), body)?;
    if meta.feature(f).is_many() {
        g.get_at(o, f, 0).as_obj()
    } else {
        g.get(o, f).as_obj()
    }
}

fn matches_predicate(g: &Graph, x: ObjId, pred: &str) -> bool {
    // name='value',other='v2' (single-valued keys only)
    let mut rest = pred;
    while !rest.is_empty() {
        let Some(eq) = rest.find('=') else { return false };
        let name = &rest[..eq];
        let after = &rest[eq + 1..];
        let (val, next) = if let Some(q) = after.strip_prefix('\'').or_else(|| after.strip_prefix('"')) {
            let quote = after.as_bytes()[0] as char;
            let Some(end) = q.find(quote) else { return false };
            (Some(decode(&q[..end])), &q[end + 1..])
        } else if let Some(n) = after.strip_prefix("null") {
            (None, n)
        } else {
            return false;
        };
        let Some(f) = g.meta().feature_by_name(g.class_of(x), name) else { return false };
        if value_to_string(g.meta(), &g.get(x, f).to_owned()) != val {
            return false;
        }
        rest = next.strip_prefix(',').unwrap_or(next);
    }
    true
}

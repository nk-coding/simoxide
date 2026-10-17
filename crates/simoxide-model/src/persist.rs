//! Writing a model graph as XMI, and restoring a graph from its XMI with every object id kept
//! (snapshots of runs whose model changed, `DESIGN-SPD.md` N1).
//!
//! * [`write_xmi`]: one resource as an XMI document in EMF's form (the root element, or an
//!   `xmi:XMI` wrapper for several roots; persistent features that are set, attributes as XML
//!   attributes, containment as nested elements with `xsi:type`, references within the document
//!   as IDREFs, references to other documents as `href` elements). SimOxide's loader reads it
//!   back to the same objects and values.
//! * [`GraphXmi`]: a whole graph as one XMI document per resource, plus a document of the objects
//!   no resource contains (objects a model change removed or never attached, kept because the
//!   typed model keeps them as tombstones), plus the *object table* that maps the documents'
//!   elements back to their object ids, with what XMI cannot carry (object homes, source lines,
//!   unresolved proxies, explicitly unset values). [`GraphXmi::restore`] loads the documents with
//!   the ordinary loader and renumbers the objects, so the restored graph equals the saved one
//!   object for object ([`same_graph`]); a typed model built from it in the saved build order
//!   (`build_from_graph_after`) has the same typed ids.
//! * [`fingerprint`]: a content hash of a graph, independent of where the files were loaded
//!   from, to recognise the model a snapshot belongs to.
//!
//! The merge's step M5 replaces the graph with the shared EMF graph (`simoxide-emf`, which has
//! EMF's own XMI writer); the document form stays, the object table then maps to that graph's
//! object ids.

use crate::diag::Level;
use crate::load::{LoadOptions, Loader};
use crate::meta::{ClassId, DataKind, FeatureId, FeatureKind, PackageId};
use crate::raw::{Graph, Obj, ObjId, ResId, Resource, Slot, Value};
use std::collections::HashMap;
use std::fmt::Write as _;

const XMI_NS: &str = "http://www.omg.org/XMI";
const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// Where each object sits in the documents: per document, its objects in tree order
/// (`all_contents`), and per object its fragment.
struct Layout {
    /// Per document: roots.
    roots: Vec<Vec<ObjId>>,
    /// Per document: objects in tree order.
    elements: Vec<Vec<ObjId>>,
    /// Per object: (document, fragment); `u32::MAX` for objects in no document.
    doc: Vec<u32>,
    frag: Vec<String>,
}

impl Layout {
    /// The resources' documents (in resource order), then the document of the detached objects:
    /// objects reached from no resource root that are no proxy (or a contained proxy) and whose
    /// container does not hold them.
    fn new(g: &Graph) -> Layout {
        let n = g.objs.len();
        let mut l = Layout {
            roots: Vec::new(),
            elements: Vec::new(),
            doc: vec![u32::MAX; n],
            frag: vec![String::new(); n],
        };
        for r in &g.resources {
            l.add_doc(g, r.roots.clone());
        }
        // detached objects: tops are those whose container does not hold them or is not
        // detached itself; what a cycle leaves over becomes a top as well
        let cand = |l: &Layout, o: usize| l.doc[o] == u32::MAX && g.objs[o].proxy.is_none();
        let mut tops: Vec<ObjId> = (0..n)
            .filter(|&o| cand(&l, o))
            .filter(|&o| match g.objs[o].container {
                None => true,
                Some((p, f)) => {
                    !g.get_refs(p, f).contains(&ObjId(o as u32)) || !cand(&l, p.0 as usize)
                }
            })
            .map(|o| ObjId(o as u32))
            .collect();
        let mut probe = Layout {
            roots: Vec::new(),
            elements: Vec::new(),
            doc: l.doc.clone(),
            frag: Vec::new(),
        };
        probe.frag = vec![String::new(); n];
        probe.add_doc(g, tops.clone());
        tops.extend((0..n).filter(|&o| cand(&probe, o)).map(|o| ObjId(o as u32)));
        l.add_doc(g, tops);
        l
    }

    fn add_doc(&mut self, g: &Graph, roots: Vec<ObjId>) {
        let d = self.elements.len() as u32;
        let mut order = Vec::new();
        // (object, fragment) depth first, children in containment order
        let mut stack: Vec<(ObjId, String)> = roots
            .iter()
            .enumerate()
            .rev()
            .map(|(i, &o)| {
                let f = if i == 0 {
                    "/".to_string()
                } else {
                    format!("/{i}")
                };
                (o, f)
            })
            .collect();
        while let Some((o, frag)) = stack.pop() {
            if self.doc[o.0 as usize] != u32::MAX {
                // a cycle or a second container: written once
                continue;
            }
            self.doc[o.0 as usize] = d;
            order.push(o);
            let obj = &g[o];
            let mut kids = Vec::new();
            for &f in obj.class.containment_features() {
                let refs = g.get_refs(o, f);
                for (i, &c) in refs.iter().enumerate() {
                    let seg = if f.def().many {
                        format!("{frag}/@{}.{i}", f.name())
                    } else {
                        format!("{frag}/@{}", f.name())
                    };
                    kids.push((c, seg));
                }
            }
            stack.extend(kids.into_iter().rev());
            self.frag[o.0 as usize] = frag;
        }
        self.roots.push(roots);
        self.elements.push(order);
    }
}

/// The fragment tokens of a document: an object's `xmi:id` or EMF id where it is unique among
/// the document's ids and usable as an IDREF token, else its path.
fn tokens(g: &Graph, l: &Layout, d: usize) -> HashMap<ObjId, String> {
    let mut count: HashMap<&str, u32> = HashMap::new();
    for &o in &l.elements[d] {
        if let Some(x) = g[o].xmi_id.as_deref() {
            *count.entry(x).or_default() += 1;
        }
        if g[o].proxy.is_none()
            && let Some(id) = g.id(o)
        {
            *count.entry(id).or_default() += 1;
        }
    }
    let usable = |s: &str| {
        !s.is_empty()
            && !s.starts_with('/')
            && !s.contains(|c: char| c.is_whitespace() || c == '#' || c == ':')
            && count.get(s) == Some(&1)
    };
    let mut out = HashMap::new();
    for &o in &l.elements[d] {
        let t = match (g[o].xmi_id.as_deref(), g.id(o)) {
            (Some(x), _) if usable(x) => x.to_string(),
            (None, Some(id)) if g[o].proxy.is_none() && usable(id) => id.to_string(),
            _ => l.frag[o.0 as usize].clone(),
        };
        out.insert(o, t);
    }
    out
}

/// XML attribute value escaping (line breaks and tabs as character references, so that
/// attribute-value normalisation keeps them).
fn escape_attr(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            '\t' => out.push_str("&#x9;"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "&#x{:X};", c as u32);
            }
            c => out.push(c),
        }
    }
}

fn escape_text(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => {
                let _ = write!(out, "&#x{:X};", c as u32);
            }
            c => out.push(c),
        }
    }
}

/// The lexical form of an attribute value that the loader parses back to the same value
/// (`None`: not representable, e.g. `Null`).
fn lexical(f: FeatureId, v: &Value) -> Option<String> {
    let data = f.data_kind()?;
    Some(match v {
        Value::Null => return None,
        Value::Str(s) | Value::Other(s) => s.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Double(d) => {
            if d.is_nan() {
                "NaN".to_string()
            } else if d.is_infinite() {
                if *d > 0.0 { "Infinity" } else { "-Infinity" }.to_string()
            } else {
                format!("{d:?}")
            }
        }
        Value::Bool(b) => b.to_string(),
        Value::Enum(e) => match data {
            DataKind::Enum(en) => en.def().literals.get(*e as usize)?.literal.to_string(),
            _ => return None,
        },
    })
}

/// A value the loader reads back as `v` (checked, so that the object table carries the others).
fn round_trips(f: FeatureId, v: &Value) -> bool {
    let (Some(data), Some(s)) = (f.data_kind(), lexical(f, v)) else {
        return false;
    };
    if f.def().many && s.contains(['\n', '\r']) {
        // element text: the loader normalises line ends
        return false;
    }
    match crate::raw::parse_value(data, &s) {
        Some(Value::Double(a)) => matches!(v, Value::Double(b) if a.to_bits() == b.to_bits()),
        Some(p) => &p == v,
        None => false,
    }
}

/// Namespace prefixes of the packages a document uses (unique).
struct Prefixes {
    by_pkg: Vec<(PackageId, String)>,
}

impl Prefixes {
    fn get(&mut self, p: PackageId) -> String {
        if let Some((_, x)) = self.by_pkg.iter().find(|(q, _)| *q == p) {
            return x.clone();
        }
        let base = match p.def().prefix {
            "" => "p",
            x => x,
        };
        let mut x = base.to_string();
        let mut i = 1;
        while x == "xmi" || x == "xsi" || self.by_pkg.iter().any(|(_, y)| *y == x) {
            x = format!("{base}{i}");
            i += 1;
        }
        self.by_pkg.push((p, x.clone()));
        x
    }

    fn qname(&mut self, c: ClassId) -> String {
        format!("{}:{}", self.get(c.def().package), c.name())
    }
}

/// What the writer needs to know about the other documents.
struct Ctx<'a> {
    g: &'a Graph,
    l: &'a Layout,
    tokens: Vec<HashMap<ObjId, String>>,
    /// The `href` location of each document.
    locs: Vec<String>,
}

impl Ctx<'_> {
    fn is_element(&self, o: ObjId) -> bool {
        self.l.doc[o.0 as usize] != u32::MAX
    }

    fn href(&self, from: usize, t: ObjId) -> String {
        let d = self.l.doc[t.0 as usize] as usize;
        let tok = &self.tokens[d][&t];
        if d == from {
            format!("#{tok}")
        } else {
            format!("{}#{tok}", self.locs[d])
        }
    }
}

/// Writes document `d`.
fn write_doc(cx: &Ctx<'_>, d: usize) -> String {
    let roots = &cx.l.roots[d];
    let mut px = Prefixes { by_pkg: Vec::new() };
    let mut body = String::new();
    let wrapper = roots.len() != 1;
    for &r in roots {
        write_obj(cx, d, r, None, &mut px, wrapper as usize, &mut body);
    }
    let mut ns = String::new();
    for (p, x) in &px.by_pkg {
        let _ = write!(ns, " xmlns:{x}=\"");
        escape_attr(p.def().ns_uri, &mut ns);
        ns.push('"');
    }
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    if wrapper {
        let _ = write!(
            out,
            "<xmi:XMI xmi:version=\"2.0\" xmlns:xmi=\"{XMI_NS}\" xmlns:xsi=\"{XSI_NS}\"{ns}"
        );
        if roots.is_empty() {
            out.push_str("/>\n");
        } else {
            out.push_str(">\n");
            out.push_str(&body);
            out.push_str("</xmi:XMI>\n");
        }
    } else {
        // the root's start tag: `<prefix:Class`, the namespaces, the rest of the document
        let end = body[1..]
            .find([' ', '>', '/'])
            .map_or(body.len(), |i| i + 1);
        let _ = write!(
            out,
            "{} xmi:version=\"2.0\" xmlns:xmi=\"{XMI_NS}\" xmlns:xsi=\"{XSI_NS}\"{ns}{}",
            &body[..end],
            &body[end..]
        );
    }
    out
}

/// Writes object `o` as the element `tag` (`None`: a root, tagged with its class), indented.
fn write_obj(
    cx: &Ctx<'_>,
    d: usize,
    o: ObjId,
    tag: Option<FeatureId>,
    px: &mut Prefixes,
    depth: usize,
    out: &mut String,
) {
    let g = cx.g;
    let obj = &g[o];
    let indent = "  ".repeat(depth);
    out.push_str(&indent);
    out.push('<');
    match tag {
        None => out.push_str(&px.qname(obj.class)),
        Some(f) => {
            out.push_str(f.name());
            out.push_str(" xsi:type=\"");
            out.push_str(&px.qname(obj.class));
            out.push('"');
        }
    }
    if let Some(x) = &obj.xmi_id {
        out.push_str(" xmi:id=\"");
        escape_attr(x, out);
        out.push('"');
    }
    if let Some(p) = &obj.proxy {
        // a contained proxy (its URI is restored from the object table)
        out.push_str(" href=\"");
        escape_attr(p, out);
        out.push_str("\"/>\n");
        return;
    }
    // XML attributes, then child elements, each in feature order
    let mut children: Vec<String> = Vec::new();
    for f in obj.class.features() {
        if !f.is_persistent() {
            continue;
        }
        let Some(slot) = g.slot(o, f) else { continue };
        match (f.def().kind, slot) {
            (FeatureKind::Attribute { .. }, Slot::One(v)) if !f.def().many => {
                if round_trips(f, v)
                    && let Some(s) = lexical(f, v)
                {
                    let _ = write!(out, " {}=\"", f.name());
                    escape_attr(&s, out);
                    out.push('"');
                }
            }
            (FeatureKind::Attribute { .. }, Slot::Many(vs)) => {
                if vs.iter().all(|v| round_trips(f, v)) {
                    for v in vs {
                        let mut e = format!("{indent}  <{}>", f.name());
                        escape_text(&lexical(f, v).unwrap_or_default(), &mut e);
                        let _ = writeln!(e, "</{}>", f.name());
                        children.push(e);
                    }
                }
            }
            (
                FeatureKind::Reference {
                    containment: true, ..
                },
                _,
            ) => {
                for &c in g.get_refs(o, f) {
                    if cx.l.doc[c.0 as usize] == d as u32 {
                        let mut e = String::new();
                        write_obj(cx, d, c, Some(f), px, depth + 1, &mut e);
                        children.push(e);
                    }
                }
            }
            (FeatureKind::Reference { .. }, _) => {
                let targets: Vec<ObjId> = g
                    .get_refs(o, f)
                    .iter()
                    .copied()
                    .filter(|&t| g[t].proxy.is_none() && cx.is_element(t))
                    .collect();
                if targets.is_empty() {
                    continue;
                }
                if targets.iter().all(|&t| cx.l.doc[t.0 as usize] == d as u32) {
                    let toks: Vec<&str> =
                        targets.iter().map(|t| cx.tokens[d][t].as_str()).collect();
                    let _ = write!(out, " {}=\"", f.name());
                    escape_attr(&toks.join(" "), out);
                    out.push('"');
                } else {
                    for t in targets {
                        let mut e = format!(
                            "{indent}  <{} xsi:type=\"{}\" href=\"",
                            f.name(),
                            px.qname(g[t].class)
                        );
                        escape_attr(&cx.href(d, t), &mut e);
                        e.push_str("\"/>\n");
                        children.push(e);
                    }
                }
            }
            _ => {}
        }
    }
    if children.is_empty() {
        out.push_str("/>\n");
        return;
    }
    out.push_str(">\n");
    for c in children {
        out.push_str(&c);
    }
    out.push_str(&indent);
    out.push_str("</");
    match tag {
        None => out.push_str(&px.qname(obj.class)),
        Some(f) => out.push_str(f.name()),
    }
    out.push_str(">\n");
}

/// File name of a resource's document: the last segment of its name, made unique.
fn doc_names(g: &Graph) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (i, r) in g.resources.iter().enumerate() {
        let base = r
            .name
            .rsplit(['/', '\\'])
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("resource")
            .replace(|c: char| c.is_whitespace() || c == '#' || c == '%', "_");
        let mut n = base.clone();
        if names.contains(&n) || n == DETACHED_DOC {
            n = format!("{i}_{base}");
        }
        names.push(n);
    }
    names
}

/// Document name of the detached objects.
pub const DETACHED_DOC: &str = "detached-objects.xmi";

/// Resource `r` of `g` as an XMI document. References into other resources are `href`s to the
/// other resources' file names (the last segment of their names: the files of one model
/// directory) or, for bundled models, their URIs; unresolved proxies are left out.
pub fn write_xmi(g: &Graph, r: ResId) -> String {
    let l = Layout::new(g);
    // other files by their names, bundled models by their URIs
    let locs = doc_names(g)
        .into_iter()
        .zip(&g.resources)
        .map(|(n, r)| {
            if r.uri.starts_with("file:") {
                n
            } else {
                r.uri.clone()
            }
        })
        .collect();
    let cx = Ctx {
        g,
        tokens: (0..l.elements.len()).map(|d| tokens(g, &l, d)).collect(),
        l: &l,
        locs,
    };
    write_doc(&cx, r.0 as usize)
}

/// A value of the object table that XMI does not carry.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub enum TableSlot {
    /// A single-valued attribute whose value XMI does not carry (`null`, a value of another type
    /// than the feature's, ...).
    One(TableValue),
    /// A many-valued attribute (empty but set, or values XMI does not carry).
    Many(Vec<TableValue>),
    /// A single-valued reference (set to `None`, or to an unresolved proxy).
    Ref(Option<u32>),
    /// A many-valued reference (empty but set, or holding unresolved proxies).
    Refs(Vec<u32>),
}

/// An attribute value of the object table (a copy of [`Value`]; doubles by their bits).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub enum TableValue {
    Null,
    Str(String),
    Int(i64),
    Double(u64),
    Bool(bool),
    Enum(u32),
    Other(String),
}

impl From<&Value> for TableValue {
    fn from(v: &Value) -> TableValue {
        match v {
            Value::Null => TableValue::Null,
            Value::Str(s) => TableValue::Str(s.to_string()),
            Value::Int(i) => TableValue::Int(*i),
            Value::Double(d) => TableValue::Double(d.to_bits()),
            Value::Bool(b) => TableValue::Bool(*b),
            Value::Enum(e) => TableValue::Enum(*e),
            Value::Other(s) => TableValue::Other(s.to_string()),
        }
    }
}

impl From<&TableValue> for Value {
    fn from(v: &TableValue) -> Value {
        match v {
            TableValue::Null => Value::Null,
            TableValue::Str(s) => Value::Str(s.as_str().into()),
            TableValue::Int(i) => Value::Int(*i),
            TableValue::Double(d) => Value::Double(f64::from_bits(*d)),
            TableValue::Bool(b) => Value::Bool(*b),
            TableValue::Enum(e) => Value::Enum(*e),
            TableValue::Other(s) => Value::Other(s.as_str().into()),
        }
    }
}

/// An unresolved proxy that is no element of any document.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub struct TableProxy {
    pub id: u32,
    pub class: u32,
    pub uri: String,
    pub xmi_id: Option<String>,
    /// Container and containment feature.
    pub container: Option<(u32, u32)>,
}

/// A resource of a [`GraphXmi`].
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub struct XmiResource {
    pub uri: String,
    pub name: String,
    pub path: Option<String>,
    pub errors: Vec<String>,
    pub failed: bool,
    /// Document file name (unique within the graph; `href`s between the documents use it).
    pub file: String,
    pub xmi: String,
}

/// A graph as XMI documents plus the object table (see the module documentation).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub struct GraphXmi {
    pub resources: Vec<XmiResource>,
    /// The objects no resource contains, as the roots of one more document.
    pub detached_xmi: String,
    /// Number of objects of the graph.
    pub objects: u32,
    /// Per document (resources, then the detached document): the object id of each element in
    /// tree order.
    pub elements: Vec<Vec<u32>>,
    /// Per detached root: its container and containment feature, if it has one that does not
    /// hold it.
    pub detached_containers: Vec<Option<(u32, u32)>>,
    /// Per object: its resource (`Obj::resource`) and source line.
    pub homes: Vec<u32>,
    pub lines: Vec<u32>,
    /// Proxy URIs of the elements that are proxies (contained proxies).
    pub proxy_uris: Vec<(u32, String)>,
    /// Proxies that are no element.
    pub proxies: Vec<TableProxy>,
    /// Values XMI does not carry: (object, feature, value).
    pub slots: Vec<(u32, u32, TableSlot)>,
}

impl GraphXmi {
    /// Saves `g` (see the module documentation).
    pub fn save(g: &Graph) -> GraphXmi {
        let l = Layout::new(g);
        let mut locs = doc_names(g);
        locs.push(DETACHED_DOC.to_string());
        let cx = Ctx {
            g,
            tokens: (0..l.elements.len()).map(|d| tokens(g, &l, d)).collect(),
            l: &l,
            locs,
        };
        let docs: Vec<String> = (0..l.elements.len()).map(|d| write_doc(&cx, d)).collect();
        let resources = g
            .resources
            .iter()
            .enumerate()
            .map(|(i, r)| XmiResource {
                uri: r.uri.clone(),
                name: r.name.clone(),
                path: r.path.as_ref().map(|p| p.display().to_string()),
                errors: r.errors.clone(),
                failed: r.failed,
                file: cx.locs[i].clone(),
                xmi: docs[i].clone(),
            })
            .collect();
        let detached = l.elements.len() - 1;
        let detached_containers = l.roots[detached]
            .iter()
            .map(|&o| g[o].container.map(|(p, f)| (p.0, f.0)))
            .collect();
        let mut proxy_uris = Vec::new();
        let mut proxies = Vec::new();
        for (i, o) in g.objs.iter().enumerate() {
            let Some(uri) = &o.proxy else { continue };
            if l.doc[i] != u32::MAX {
                proxy_uris.push((i as u32, uri.to_string()));
            } else {
                proxies.push(TableProxy {
                    id: i as u32,
                    class: o.class.0,
                    uri: uri.to_string(),
                    xmi_id: o.xmi_id.as_deref().map(str::to_string),
                    container: o.container.map(|(p, f)| (p.0, f.0)),
                });
            }
        }
        let mut slots = Vec::new();
        for (i, o) in g.objs.iter().enumerate() {
            if o.proxy.is_some() {
                // a proxy's values (the opposite ends the loader set on it): all in the table
                for (f, s) in &o.slots {
                    if let Some(t) = table_slot_of(s) {
                        slots.push((i as u32, f.0, t));
                    }
                }
                continue;
            }
            if l.doc[i] == u32::MAX {
                continue;
            }
            let id = ObjId(i as u32);
            for (f, s) in &o.slots {
                if !f.is_persistent() {
                    // the loader never sets these; a host may have
                    if let Some(t) = table_slot_of(s) {
                        slots.push((i as u32, f.0, t));
                    }
                    continue;
                }
                let table = match s {
                    Slot::One(v) if !round_trips(*f, v) => Some(TableSlot::One(v.into())),
                    Slot::Many(vs) if vs.is_empty() || !vs.iter().all(|v| round_trips(*f, v)) => {
                        Some(TableSlot::Many(vs.iter().map(Into::into).collect()))
                    }
                    Slot::Ref(None) => Some(TableSlot::Ref(None)),
                    Slot::Refs(v) if v.is_empty() => Some(TableSlot::Refs(Vec::new())),
                    Slot::Ref(Some(t)) if !f.is_containment() && !written_target(g, &l, *t) => {
                        Some(TableSlot::Ref(Some(t.0)))
                    }
                    Slot::Refs(v)
                        if !f.is_containment() && v.iter().any(|t| !written_target(g, &l, *t)) =>
                    {
                        Some(TableSlot::Refs(v.iter().map(|t| t.0).collect()))
                    }
                    Slot::Refs(v)
                        if f.is_containment()
                            && v.iter().any(|t| l.doc[t.0 as usize] != l.doc[i]) =>
                    {
                        Some(TableSlot::Refs(v.iter().map(|t| t.0).collect()))
                    }
                    Slot::Ref(Some(t)) if f.is_containment() && l.doc[t.0 as usize] != l.doc[i] => {
                        Some(TableSlot::Ref(Some(t.0)))
                    }
                    _ => None,
                };
                if let Some(t) = table {
                    slots.push((id.0, f.0, t));
                }
            }
        }
        GraphXmi {
            resources,
            detached_xmi: docs[detached].clone(),
            objects: g.objs.len() as u32,
            elements: l
                .elements
                .iter()
                .map(|e| e.iter().map(|o| o.0).collect())
                .collect(),
            detached_containers,
            homes: g.objs.iter().map(|o| o.resource.0).collect(),
            lines: g.objs.iter().map(|o| o.line).collect(),
            proxy_uris,
            proxies,
            slots,
        }
    }

    /// The graph saved by [`GraphXmi::save`]: the documents loaded with SimOxide's loader and
    /// renumbered to the saved object ids, with the object table applied.
    pub fn restore(&self) -> Result<Graph, String> {
        let n = self.objects as usize;
        let dir = std::path::Path::new(crate::load::MEMORY_DIR).join("snapshot");
        let mut loader = Loader::new(LoadOptions {
            follow_references: true,
            tolerant_references: false,
            tolerant_namespaces: true,
        });
        loader.set_read_files(false);
        let mut files: Vec<(&str, &str)> = self
            .resources
            .iter()
            .map(|r| (r.file.as_str(), r.xmi.as_str()))
            .collect();
        files.push((DETACHED_DOC, &self.detached_xmi));
        for (f, x) in &files {
            loader.add_file(dir.join(f), x.as_bytes().to_vec());
        }
        let docs: Vec<ResId> = files
            .iter()
            .map(|(f, _)| loader.load_file(dir.join(f)))
            .collect();
        let (g2, diags) = loader.finish();
        if let Some(d) = diags.iter().find(|d| d.level == Level::Error) {
            return Err(format!("the snapshot's model does not load: {d}"));
        }
        if self.elements.len() != docs.len() {
            return Err("the snapshot's object table does not match its documents".into());
        }
        // loaded object -> saved id
        let mut map: Vec<u32> = vec![u32::MAX; g2.objs.len()];
        for (d, &r) in docs.iter().enumerate() {
            let got = g2.all_contents(r);
            if got.len() != self.elements[d].len() {
                return Err(format!(
                    "document {} has {} objects, the object table {}",
                    files[d].0,
                    got.len(),
                    self.elements[d].len()
                ));
            }
            for (o2, &o) in got.iter().zip(&self.elements[d]) {
                if o as usize >= n {
                    return Err("object id out of range in the object table".into());
                }
                map[o2.0 as usize] = o;
            }
        }
        let m = |o2: ObjId| -> Result<ObjId, String> {
            match map[o2.0 as usize] {
                u32::MAX => Err(format!(
                    "the snapshot's model refers to an object outside its documents ({})",
                    g2.describe(o2)
                )),
                o => Ok(ObjId(o)),
            }
        };
        let home = |o: u32| ResId(self.homes.get(o as usize).copied().unwrap_or(0));
        let line = |o: u32| self.lines.get(o as usize).copied().unwrap_or(0);
        let mut objs: Vec<Option<Obj>> = vec![None; n];
        for (d, &r) in docs.iter().enumerate() {
            for o2 in g2.all_contents(r) {
                let id = map[o2.0 as usize];
                let src = &g2[o2];
                let mut slots = Vec::with_capacity(src.slots.len());
                for (f, s) in &src.slots {
                    let s = match s {
                        Slot::Ref(t) => Slot::Ref(t.map(m).transpose()?),
                        Slot::Refs(v) => {
                            Slot::Refs(v.iter().map(|&t| m(t)).collect::<Result<_, _>>()?)
                        }
                        s => s.clone(),
                    };
                    slots.push((*f, s));
                }
                let mut obj = src.clone();
                obj.slots = slots;
                obj.container = src
                    .container
                    .map(|(p, f)| m(p).map(|p| (p, f)))
                    .transpose()?;
                obj.resource = home(id);
                obj.line = line(id);
                objs[id as usize] = Some(obj);
                let _ = d;
            }
        }
        // detached roots keep the container that does not hold them
        let det = docs.len() - 1;
        let det_roots = &g2.resources[docs[det].0 as usize].roots;
        for (i, r2) in det_roots.iter().enumerate() {
            let id = map[r2.0 as usize] as usize;
            if let Some(o) = objs[id].as_mut() {
                o.container = self
                    .detached_containers
                    .get(i)
                    .copied()
                    .flatten()
                    .map(|(p, f)| (ObjId(p), FeatureId(f)));
            }
        }
        for (id, uri) in &self.proxy_uris {
            if let Some(o) = objs.get_mut(*id as usize).and_then(Option::as_mut) {
                o.proxy = Some(uri.as_str().into());
                o.slots.clear();
            }
        }
        for p in &self.proxies {
            let i = p.id as usize;
            if i >= n {
                return Err("proxy id out of range in the object table".into());
            }
            objs[i] = Some(Obj {
                class: ClassId(p.class),
                resource: home(p.id),
                container: p.container.map(|(c, f)| (ObjId(c), FeatureId(f))),
                proxy: Some(p.uri.as_str().into()),
                xmi_id: p.xmi_id.as_deref().map(Into::into),
                slots: Vec::new(),
                line: line(p.id),
                attached: false,
            });
        }
        for (o, f, t) in &self.slots {
            let Some(obj) = objs.get_mut(*o as usize).and_then(Option::as_mut) else {
                return Err("object table slot of a missing object".into());
            };
            let f = FeatureId(*f);
            let s = match t {
                TableSlot::One(v) => Slot::One(v.into()),
                TableSlot::Many(v) => Slot::Many(v.iter().map(Into::into).collect()),
                TableSlot::Ref(r) => Slot::Ref(r.map(ObjId)),
                TableSlot::Refs(v) => Slot::Refs(v.iter().map(|&x| ObjId(x)).collect()),
            };
            match obj.slots.iter_mut().find(|(g, _)| *g == f) {
                Some((_, x)) => *x = s,
                None => obj.slots.push((f, s)),
            }
        }
        let mut out = Graph {
            objs: Vec::with_capacity(n),
            resources: Vec::with_capacity(self.resources.len()),
        };
        for (i, o) in objs.into_iter().enumerate() {
            out.objs.push(
                o.ok_or_else(|| format!("object #{i} is missing from the snapshot's model"))?,
            );
        }
        for (i, r) in self.resources.iter().enumerate() {
            let r2 = &g2.resources[docs[i].0 as usize];
            out.resources.push(Resource {
                uri: r.uri.clone(),
                name: r.name.clone(),
                path: r.path.as_ref().map(Into::into),
                roots: r2.roots.iter().map(|&o| m(o)).collect::<Result<_, _>>()?,
                errors: r.errors.clone(),
                failed: r.failed,
            });
        }
        Ok(out)
    }
}

/// A slot of a non-persistent feature as a table entry.
fn table_slot_of(s: &Slot) -> Option<TableSlot> {
    Some(match s {
        Slot::One(v) => TableSlot::One(v.into()),
        Slot::Many(vs) => TableSlot::Many(vs.iter().map(Into::into).collect()),
        Slot::Ref(r) => TableSlot::Ref(r.map(|o| o.0)),
        Slot::Refs(v) => TableSlot::Refs(v.iter().map(|o| o.0).collect()),
    })
}

/// A reference target the documents carry (an element that is no proxy).
fn written_target(g: &Graph, l: &Layout, t: ObjId) -> bool {
    g[t].proxy.is_none() && l.doc[t.0 as usize] != u32::MAX
}

/// Compares two graphs object for object (class, home, container, proxy, `xmi:id`, slots in any
/// order, line) and resource for resource: `Err` names the first difference.
pub fn same_graph(a: &Graph, b: &Graph) -> Result<(), String> {
    if a.objs.len() != b.objs.len() {
        return Err(format!("{} vs {} objects", a.objs.len(), b.objs.len()));
    }
    if a.resources.len() != b.resources.len() {
        return Err(format!(
            "{} vs {} resources",
            a.resources.len(),
            b.resources.len()
        ));
    }
    for (i, (x, y)) in a.resources.iter().zip(&b.resources).enumerate() {
        if x.uri != y.uri
            || x.name != y.name
            || x.path != y.path
            || x.roots != y.roots
            || x.errors != y.errors
            || x.failed != y.failed
        {
            return Err(format!("resource {i} ({}) differs", x.name));
        }
    }
    for (i, (x, y)) in a.objs.iter().zip(&b.objs).enumerate() {
        let what = if x.class != y.class {
            "class"
        } else if x.resource != y.resource {
            "resource"
        } else if x.container != y.container {
            "container"
        } else if x.proxy != y.proxy {
            "proxy"
        } else if x.xmi_id != y.xmi_id {
            "xmi:id"
        } else if x.line != y.line {
            "line"
        } else if x.slots.len() != y.slots.len() {
            "set features"
        } else {
            let mut d = "";
            for (f, s) in &x.slots {
                let Some((_, t)) = y.slots.iter().find(|(g, _)| g == f) else {
                    d = "set features";
                    break;
                };
                if !same_slot(s, t) {
                    d = f.name();
                    break;
                }
            }
            d
        };
        if !what.is_empty() {
            return Err(format!(
                "object #{i} ({}): {what} differs",
                a.describe(ObjId(i as u32))
            ));
        }
    }
    Ok(())
}

fn same_slot(a: &Slot, b: &Slot) -> bool {
    let same = |x: &Value, y: &Value| match (x, y) {
        (Value::Double(p), Value::Double(q)) => p.to_bits() == q.to_bits(),
        _ => x == y,
    };
    match (a, b) {
        (Slot::One(x), Slot::One(y)) => same(x, y),
        (Slot::Many(x), Slot::Many(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q))
        }
        (Slot::Ref(x), Slot::Ref(y)) => x == y,
        (Slot::Refs(x), Slot::Refs(y)) => x == y,
        _ => false,
    }
}

/// A content hash of `g` (two 64-bit FNV-1a lanes): classes, ids, values and references of
/// every object in id order and the resources' names and roots, not the places the files were
/// read from (absolute paths, source lines). Equal models loaded from different directories
/// have equal fingerprints.
pub fn fingerprint(g: &Graph) -> [u64; 2] {
    let mut h = Fnv::new();
    h.u64(g.resources.len() as u64);
    for r in &g.resources {
        h.str(&r.name);
        h.u64(r.failed as u64);
        h.u64(r.roots.len() as u64);
        for o in &r.roots {
            h.u64(o.0 as u64);
        }
    }
    h.u64(g.objs.len() as u64);
    for o in &g.objs {
        h.u64(o.class.0 as u64);
        h.u64(o.resource.0 as u64);
        match o.container {
            Some((p, f)) => {
                h.u64(1 + p.0 as u64);
                h.u64(f.0 as u64);
            }
            None => h.u64(0),
        }
        match &o.proxy {
            // the file name and fragment, not the directory
            Some(p) => {
                let (loc, frag) = p.split_once('#').unwrap_or((p, ""));
                h.str(loc.rsplit(['/', '\\']).next().unwrap_or(loc));
                h.str(frag);
            }
            None => h.u64(0),
        }
        h.str(o.xmi_id.as_deref().unwrap_or(""));
        let mut slots: Vec<&(FeatureId, Slot)> = o.slots.iter().collect();
        slots.sort_by_key(|(f, _)| *f);
        h.u64(slots.len() as u64);
        for (f, s) in slots {
            h.u64(f.0 as u64);
            match s {
                Slot::One(v) => h.value(v),
                Slot::Many(vs) => {
                    h.u64(vs.len() as u64);
                    vs.iter().for_each(|v| h.value(v));
                }
                Slot::Ref(r) => h.u64(r.map_or(0, |o| 1 + o.0 as u64)),
                Slot::Refs(v) => {
                    h.u64(v.len() as u64);
                    v.iter().for_each(|o| h.u64(o.0 as u64));
                }
            }
        }
    }
    [h.a, h.b]
}

struct Fnv {
    a: u64,
    b: u64,
}

impl Fnv {
    fn new() -> Fnv {
        Fnv {
            a: 0xcbf2_9ce4_8422_2325,
            b: 0x6c62_272e_07bb_0142,
        }
    }
    fn byte(&mut self, x: u8) {
        self.a = (self.a ^ x as u64).wrapping_mul(0x0000_0100_0000_01b3);
        self.b = (self.b ^ x as u64)
            .wrapping_mul(0x0000_0100_0000_01b3)
            .rotate_left(5);
    }
    fn u64(&mut self, x: u64) {
        for b in x.to_le_bytes() {
            self.byte(b);
        }
    }
    fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        for b in s.bytes() {
            self.byte(b);
        }
    }
    fn value(&mut self, v: &Value) {
        match v {
            Value::Null => self.u64(0),
            Value::Str(s) => {
                self.u64(1);
                self.str(s);
            }
            Value::Int(i) => {
                self.u64(2);
                self.u64(*i as u64);
            }
            Value::Double(d) => {
                self.u64(3);
                self.u64(d.to_bits());
            }
            Value::Bool(b) => {
                self.u64(4);
                self.u64(*b as u64);
            }
            Value::Enum(e) => {
                self.u64(5);
                self.u64(*e as u64);
            }
            Value::Other(s) => {
                self.u64(6);
                self.str(s);
            }
        }
    }
}

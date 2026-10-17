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
//! The graph is the shared EMF graph (`simoxide-emf`). Objects of other metamodels than
//! SimOxide's (a QVTo host's metamodel objects, runtime measurement models) and the resources
//! that hold only such objects are not written: a restored graph has inert placeholders in their
//! place (objects of Ecore's `EObject` without values, resources without roots), so that every
//! id stays; their owners rebuild them (`simoxide-reconf`). The fingerprint and the build order
//! ignore them.

use crate::diag::Level;
use crate::load::{LoadOptions, Loader};
use crate::meta::prelude::*;
use crate::meta::{ClassId, DataKind, FeatureId, FeatureKind, PackageId};
use crate::raw::{Graph, ObjId, PcmGraph, ResId, Value, from_stored, is_pcm_class};
use simoxide_emf::ResourceKind;
use simoxide_emf::load_ops::{OwnedSlot, StoredSlot};
use simoxide_emf_core::ModelStore;
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
        let n = g.object_count();
        let mut l = Layout {
            roots: Vec::new(),
            elements: Vec::new(),
            doc: vec![u32::MAX; n],
            frag: vec![String::new(); n],
        };
        for r in g.resources() {
            l.add_doc(g, r.roots.iter().copied().filter(|&o| pcm(g, o)).collect());
        }
        // detached objects: tops are those whose container does not hold them or is not
        // detached itself; what a cycle leaves over becomes a top as well
        let cand = |l: &Layout, o: usize| {
            l.doc[o] == u32::MAX && g[ObjId(o as u32)].proxy.is_none() && pcm(g, ObjId(o as u32))
        };
        let mut tops: Vec<ObjId> = (0..n)
            .filter(|&o| cand(&l, o))
            .filter(|&o| match g[ObjId(o as u32)].container {
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
        if let Some(x) = g.xmi_id(o) {
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
        let t = match (g.xmi_id(o), g.id(o)) {
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
    if let Some(x) = g.xmi_id(o) {
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
        let slot = g.stored_slot(o, f);
        if slot == StoredSlot::Unset {
            continue;
        }
        match (f.def().kind, slot) {
            (FeatureKind::Attribute { .. }, StoredSlot::Data(v)) if !f.def().many => {
                let v = &from_stored(v);
                if round_trips(f, v)
                    && let Some(s) = lexical(f, v)
                {
                    let _ = write!(out, " {}=\"", f.name());
                    escape_attr(&s, out);
                    out.push('"');
                }
            }
            (FeatureKind::Attribute { .. }, StoredSlot::Datas(vs, _)) => {
                let vs: Vec<Value> = vs.iter().map(from_stored).collect();
                if vs.iter().all(|v| round_trips(f, v)) {
                    for v in &vs {
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
                    .filter(|&t| g[t].proxy.is_none() && pcm(g, t) && cx.is_element(t))
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
    for (i, r) in g.resources().iter().enumerate() {
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
        .zip(g.resources())
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

/// Whether `o` is an object of SimOxide's metamodels (the others are not written).
fn pcm(g: &Graph, o: ObjId) -> bool {
    is_pcm_class(g[o].class)
}

/// A value of the object table that XMI does not carry.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub enum TableSlot {
    /// A single-valued attribute whose value XMI does not carry (`null`, a value of another type
    /// than the feature's, ...).
    One(TableValue),
    /// A many-valued attribute (empty but set, or values XMI does not carry), with the "set"
    /// flag of unsettable features.
    Many(Vec<TableValue>, bool),
    /// A single-valued reference (set to `None`, or to an unresolved proxy).
    Ref(Option<u32>),
    /// A many-valued reference (empty but set, or holding unresolved proxies), with the flag.
    Refs(Vec<u32>, bool),
}

/// An attribute value of the object table: a value of the graph (its Java box kept), doubles
/// and floats by their bits.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub enum TableValue {
    Null,
    Bool(bool),
    Int(i32),
    Long(i64),
    Short(i16),
    Byte(i8),
    Char(u16),
    Double(u64),
    Float(u32),
    Str(String),
    Enum(u32, u32),
    Other(String),
}

impl From<&simoxide_emf::Value> for TableValue {
    fn from(v: &simoxide_emf::Value) -> TableValue {
        use simoxide_emf::Value as V;
        match v {
            V::Null | V::Obj(_) => TableValue::Null,
            V::Bool(b) => TableValue::Bool(*b),
            V::Int(i) => TableValue::Int(*i),
            V::Long(i) => TableValue::Long(*i),
            V::Short(i) => TableValue::Short(*i),
            V::Byte(i) => TableValue::Byte(*i),
            V::Char(c) => TableValue::Char(*c),
            V::Double(d) => TableValue::Double(d.to_bits()),
            V::Float(f) => TableValue::Float(f.to_bits()),
            V::Str(s) => TableValue::Str(s.to_string()),
            V::Enum(e, i) => TableValue::Enum(e.0, *i),
            V::Other(s) => TableValue::Other(s.to_string()),
        }
    }
}

impl From<&TableValue> for simoxide_emf::Value {
    fn from(v: &TableValue) -> simoxide_emf::Value {
        use simoxide_emf::Value as V;
        match v {
            TableValue::Null => V::Null,
            TableValue::Bool(b) => V::Bool(*b),
            TableValue::Int(i) => V::Int(*i),
            TableValue::Long(i) => V::Long(*i),
            TableValue::Short(i) => V::Short(*i),
            TableValue::Byte(i) => V::Byte(*i),
            TableValue::Char(c) => V::Char(*c),
            TableValue::Double(d) => V::Double(f64::from_bits(*d)),
            TableValue::Float(f) => V::Float(f32::from_bits(*f)),
            TableValue::Str(s) => V::Str(s.as_str().into()),
            TableValue::Enum(e, i) => V::Enum(simoxide_emf_core::EnumId(*e), *i),
            TableValue::Other(s) => V::Other(s.as_str().into()),
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
    /// The resource holds only objects of other metamodels: restored as an empty placeholder
    /// (with another URI, so that it is not found in place of the rebuilt model).
    pub foreign: bool,
}

/// The URI prefix of a restored placeholder resource: `simoxide:/snapshot-placeholder/{i}?{uri}`
/// (`uri`: the URI of the resource it stands for; another URI, so that it is not found in place
/// of the rebuilt model).
const PLACEHOLDER: &str = "simoxide:/snapshot-placeholder/";

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
    /// Per object: its source line.
    pub lines: Vec<u32>,
    /// Proxy URIs of the elements that are proxies (contained proxies).
    pub proxy_uris: Vec<(u32, String)>,
    /// Proxies that are no element.
    pub proxies: Vec<TableProxy>,
    /// Values XMI does not carry: (object, feature, value).
    pub slots: Vec<(u32, u32, TableSlot)>,
    /// Objects of other metamodels (restored as placeholders).
    pub foreign: Vec<u32>,
}

/// The table form of a stored slot (lists with their flag; `None` when unset).
pub fn table_slot_of(s: StoredSlot<'_>) -> Option<TableSlot> {
    Some(match s {
        StoredSlot::Unset => return None,
        StoredSlot::Data(v) => TableSlot::One(v.into()),
        StoredSlot::Datas(vs, set) => TableSlot::Many(vs.iter().map(Into::into).collect(), set),
        StoredSlot::Ref(r) => TableSlot::Ref(r.map(|o| o.0)),
        StoredSlot::Refs(v, set) => TableSlot::Refs(v.iter().map(|o| o.0).collect(), set),
    })
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
            .resources()
            .iter()
            .enumerate()
            .map(|(i, r)| match r.uri.strip_prefix(PLACEHOLDER) {
                // a placeholder of a restored graph: written as the resource it stands for, so
                // that a restored graph saves the bytes it was restored from
                Some(rest) => XmiResource {
                    uri: rest.split_once('?').map_or(rest, |(_, u)| u).to_string(),
                    name: r.name.clone(),
                    path: r.path.as_ref().map(|p| p.display().to_string()),
                    errors: r.errors.clone(),
                    failed: r.failed,
                    file: cx.locs[i].clone(),
                    xmi: docs[i].clone(),
                    foreign: true,
                },
                None => XmiResource {
                    uri: r.uri.clone(),
                    name: r.name.clone(),
                    path: r.path.as_ref().map(|p| p.display().to_string()),
                    errors: r.errors.clone(),
                    failed: r.failed,
                    file: cx.locs[i].clone(),
                    xmi: docs[i].clone(),
                    foreign: !r.roots.is_empty() && r.roots.iter().all(|&o| !pcm(g, o)),
                },
            })
            .collect();
        let detached = l.elements.len() - 1;
        let detached_containers = l.roots[detached]
            .iter()
            .map(|&o| g[o].container.map(|(p, f)| (p.0, f.0)))
            .collect();
        let mut proxy_uris = Vec::new();
        let mut proxies = Vec::new();
        let mut foreign = Vec::new();
        for (i, o) in g.objects().enumerate() {
            if !is_pcm_class(o.class) {
                foreign.push(i as u32);
                continue;
            }
            let Some(uri) = &o.proxy else { continue };
            if l.doc[i] != u32::MAX {
                proxy_uris.push((i as u32, uri.to_string()));
            } else {
                proxies.push(TableProxy {
                    id: i as u32,
                    class: o.class.0,
                    uri: uri.to_string(),
                    container: o.container.map(|(p, f)| (p.0, f.0)),
                });
            }
        }
        let mut slots = Vec::new();
        for (i, o) in g.objects().enumerate() {
            if !is_pcm_class(o.class) {
                continue;
            }
            let id = ObjId(i as u32);
            if o.proxy.is_some() {
                // a proxy's values (the opposite ends the loader set on it): all in the table
                for f in o.class.features() {
                    if let Some(t) = table_slot_of(g.stored_slot(id, f)) {
                        slots.push((id.0, f.0, t));
                    }
                }
                continue;
            }
            if l.doc[i] == u32::MAX {
                continue;
            }
            for f in o.class.features() {
                let s = g.stored_slot(id, f);
                if s == StoredSlot::Unset {
                    continue;
                }
                if !f.is_persistent() {
                    // the loader never sets these; a host may have
                    if let Some(t) = table_slot_of(s) {
                        slots.push((id.0, f.0, t));
                    }
                    continue;
                }
                let refs_kept = |v: &[ObjId]| v.iter().all(|t| written_target(g, &l, *t));
                let table = match s {
                    StoredSlot::Data(v) if !round_trips(f, &from_stored(v)) => table_slot_of(s),
                    StoredSlot::Datas(vs, set)
                        if vs.is_empty()
                            || !set
                            || !vs.iter().all(|v| round_trips(f, &from_stored(v))) =>
                    {
                        table_slot_of(s)
                    }
                    StoredSlot::Ref(None) => table_slot_of(s),
                    StoredSlot::Refs(v, set) if v.is_empty() || !set => table_slot_of(s),
                    StoredSlot::Ref(Some(t))
                        if !f.is_containment() && !written_target(g, &l, t) =>
                    {
                        table_slot_of(s)
                    }
                    StoredSlot::Refs(v, _) if !f.is_containment() && !refs_kept(v) => {
                        table_slot_of(s)
                    }
                    StoredSlot::Refs(v, _)
                        if f.is_containment()
                            && v.iter().any(|t| l.doc[t.0 as usize] != l.doc[i]) =>
                    {
                        table_slot_of(s)
                    }
                    StoredSlot::Ref(Some(t))
                        if f.is_containment() && l.doc[t.0 as usize] != l.doc[i] =>
                    {
                        table_slot_of(s)
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
            objects: g.object_count() as u32,
            elements: l
                .elements
                .iter()
                .map(|e| e.iter().map(|o| o.0).collect())
                .collect(),
            detached_containers,
            lines: g.objects().map(|o| o.line).collect(),
            proxy_uris,
            proxies,
            slots,
            foreign,
        }
    }

    /// The graph saved by [`GraphXmi::save`]: the documents loaded with SimOxide's loader and
    /// renumbered to the saved object ids, with the object table applied; placeholders for the
    /// objects of other metamodels.
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
        let mut map: Vec<u32> = vec![u32::MAX; g2.object_count()];
        let mut class: Vec<Option<ClassId>> = vec![None; n];
        for (d, &r) in docs.iter().enumerate() {
            let got = g2.tree_contents(r);
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
                class[o as usize] = Some(g2[*o2].class);
            }
        }
        for p in &self.proxies {
            if p.id as usize >= n {
                return Err("proxy id out of range in the object table".into());
            }
            class[p.id as usize] = Some(ClassId(p.class));
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
        let mut out = crate::raw::pcm_graph();
        let placeholder = out.ecore().eobject;
        for &i in &self.foreign {
            if i as usize >= n {
                return Err("object id out of range in the object table".into());
            }
            class[i as usize] = Some(placeholder);
        }
        for (i, c) in class.iter().enumerate() {
            let c = c.ok_or_else(|| format!("object #{i} is missing from the snapshot's model"))?;
            out.load_object(c);
            out.set_line(ObjId(i as u32), self.lines.get(i).copied().unwrap_or(0));
        }
        // the documents' objects: values, references, containers
        for &r in &docs {
            for o2 in g2.tree_contents(r) {
                let id = ObjId(map[o2.0 as usize]);
                for f in g2[o2].class.features() {
                    let s = match g2.stored_slot(o2, f) {
                        StoredSlot::Unset => continue,
                        StoredSlot::Data(v) => OwnedSlot::Data(v.clone()),
                        StoredSlot::Datas(v, set) => OwnedSlot::Datas(v.to_vec(), set),
                        StoredSlot::Ref(t) => OwnedSlot::Ref(t.map(m).transpose()?),
                        StoredSlot::Refs(v, set) => {
                            OwnedSlot::Refs(v.iter().map(|&t| m(t)).collect::<Result<_, _>>()?, set)
                        }
                    };
                    out.set_stored_slot(id, f, s);
                }
                let c = g2[o2]
                    .container
                    .map(|(p, f)| m(p).map(|p| (p, f)))
                    .transpose()?;
                out.set_container_raw(id, c);
            }
        }
        // detached roots keep the container that does not hold them
        let det = docs.len() - 1;
        for (i, r2) in g2.resource(docs[det]).roots.iter().enumerate() {
            let id = ObjId(map[r2.0 as usize]);
            out.set_container_raw(
                id,
                self.detached_containers
                    .get(i)
                    .copied()
                    .flatten()
                    .map(|(p, f)| (ObjId(p), FeatureId(f))),
            );
        }
        for (id, uri) in &self.proxy_uris {
            let o = ObjId(*id);
            out.set_proxy_uri(o, Some(uri));
            for f in out[o].class.features() {
                out.set_stored_slot(o, f, OwnedSlot::Unset);
            }
        }
        for p in &self.proxies {
            let o = ObjId(p.id);
            out.set_proxy_uri(o, Some(&p.uri));
            out.set_container_raw(o, p.container.map(|(c, f)| (ObjId(c), FeatureId(f))));
        }
        for (o, f, t) in &self.slots {
            if *o as usize >= n {
                return Err("object table slot of a missing object".into());
            }
            let (o, f) = (ObjId(*o), FeatureId(*f));
            let s = match t {
                TableSlot::One(v) => OwnedSlot::Data(v.into()),
                TableSlot::Many(v, set) => {
                    OwnedSlot::Datas(v.iter().map(Into::into).collect(), *set)
                }
                TableSlot::Ref(r) => OwnedSlot::Ref(r.map(ObjId)),
                TableSlot::Refs(v, set) => {
                    OwnedSlot::Refs(v.iter().map(|&x| ObjId(x)).collect(), *set)
                }
            };
            out.set_stored_slot(o, f, s);
        }
        for (i, r) in self.resources.iter().enumerate() {
            let uri = if r.foreign {
                format!("{PLACEHOLDER}{i}?{}", r.uri)
            } else {
                r.uri.clone()
            };
            let nr = out.create_resource_kind(&uri, ResourceKind::Xmi);
            let res = out.resource_mut(nr);
            res.name = r.name.clone();
            res.path = r.path.as_ref().map(Into::into);
            res.errors = r.errors.clone();
            res.failed = r.failed;
            let r2 = docs[i];
            for &o2 in &g2.resource(r2).roots {
                out.load_add_root(nr, m(o2)?);
            }
            for o2 in g2.tree_contents(r2) {
                if let Some(x) = g2.xmi_id(o2) {
                    let x = x.to_string();
                    out.set_xmi_id(nr, m(o2)?, Some(&x));
                }
            }
        }
        Ok(out)
    }
}

/// The stored slot of a table entry (see [`table_slot_of`]).
pub fn owned_slot(t: &TableSlot) -> OwnedSlot {
    match t {
        TableSlot::One(v) => OwnedSlot::Data(v.into()),
        TableSlot::Many(v, set) => OwnedSlot::Datas(v.iter().map(Into::into).collect(), *set),
        TableSlot::Ref(r) => OwnedSlot::Ref(r.map(ObjId)),
        TableSlot::Refs(v, set) => OwnedSlot::Refs(v.iter().map(|&x| ObjId(x)).collect(), *set),
    }
}

/// A reference target the documents carry (an element that is no proxy).
fn written_target(g: &Graph, l: &Layout, t: ObjId) -> bool {
    g[t].proxy.is_none() && l.doc[t.0 as usize] != u32::MAX
}

/// Compares two graphs object for object (class, root resource, container, proxy, `xmi:id`,
/// stored features, line) and resource for resource: `Err` names the first difference. Objects
/// of other metamodels are compared by their number only.
pub fn same_graph(a: &Graph, b: &Graph) -> Result<(), String> {
    if a.object_count() != b.object_count() {
        return Err(format!(
            "{} vs {} objects",
            a.object_count(),
            b.object_count()
        ));
    }
    if a.resources().len() != b.resources().len() {
        return Err(format!(
            "{} vs {} resources",
            a.resources().len(),
            b.resources().len()
        ));
    }
    let pcm_roots = |g: &Graph, r: &simoxide_emf::Resource| -> Vec<ObjId> {
        r.roots.iter().copied().filter(|&o| pcm(g, o)).collect()
    };
    for (i, (x, y)) in a.resources().iter().zip(b.resources()).enumerate() {
        let foreign = |g: &Graph, r: &simoxide_emf::Resource| {
            !r.roots.is_empty() && r.roots.iter().all(|&o| !pcm(g, o))
        };
        if (x.uri != y.uri && !foreign(a, x))
            || x.name != y.name
            || x.path != y.path
            || pcm_roots(a, x) != pcm_roots(b, y)
            || x.errors != y.errors
            || x.failed != y.failed
        {
            return Err(format!("resource {i} ({}) differs", x.name));
        }
    }
    for i in 0..a.object_count() {
        let o = ObjId(i as u32);
        let (x, y) = (&a[o], &b[o]);
        if !is_pcm_class(x.class) {
            if is_pcm_class(y.class) {
                return Err(format!("object #{i}: class differs"));
            }
            continue;
        }
        let what = if x.class != y.class {
            "class"
        } else if a.direct_resource(o) != b.direct_resource(o) {
            "resource"
        } else if x.container != y.container {
            "container"
        } else if x.proxy != y.proxy {
            "proxy"
        } else if a.xmi_id(o) != b.xmi_id(o) {
            "xmi:id"
        } else if x.line != y.line {
            "line"
        } else {
            let mut d = "";
            for f in x.class.features() {
                if !same_slot(a.stored_slot(o, f), b.stored_slot(o, f)) {
                    d = f.name();
                    break;
                }
            }
            d
        };
        if !what.is_empty() {
            return Err(format!("object #{i} ({}): {what} differs", a.describe(o)));
        }
    }
    Ok(())
}

fn same_slot(a: StoredSlot<'_>, b: StoredSlot<'_>) -> bool {
    let same =
        |x: &simoxide_emf::Value, y: &simoxide_emf::Value| x.as_datum().java_equals(&y.as_datum());
    match (a, b) {
        (StoredSlot::Unset, StoredSlot::Unset) => true,
        (StoredSlot::Data(x), StoredSlot::Data(y)) => same(x, y),
        (StoredSlot::Datas(x, p), StoredSlot::Datas(y, q)) => {
            p == q && x.len() == y.len() && x.iter().zip(y).all(|(u, v)| same(u, v))
        }
        (StoredSlot::Ref(x), StoredSlot::Ref(y)) => x == y,
        (StoredSlot::Refs(x, p), StoredSlot::Refs(y, q)) => p == q && x == y,
        _ => false,
    }
}

/// A content hash of `g` (two 64-bit FNV-1a lanes): classes, ids, values and references of
/// every object of SimOxide's metamodels in id order and the names and roots of the resources
/// that hold any, not the places the files were read from (absolute paths, source lines).
/// Equal models loaded from different directories have equal fingerprints.
pub fn fingerprint(g: &Graph) -> [u64; 2] {
    let mut h = Fnv::new();
    for r in g.resources() {
        let roots: Vec<ObjId> = r.roots.iter().copied().filter(|&o| pcm(g, o)).collect();
        if roots.is_empty() && !r.failed {
            continue;
        }
        h.str(&r.name);
        h.u64(r.failed as u64);
        h.u64(roots.len() as u64);
        for o in &roots {
            h.u64(o.0 as u64);
        }
    }
    h.u64(g.object_count() as u64);
    for (i, o) in g.objects().enumerate() {
        let id = ObjId(i as u32);
        if !is_pcm_class(o.class) {
            continue;
        }
        h.u64(i as u64);
        h.u64(o.class.0 as u64);
        h.u64(g.direct_resource(id).map_or(0, |r| 1 + r.0 as u64));
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
        h.str(g.xmi_id(id).unwrap_or(""));
        for f in o.class.features() {
            let s = g.stored_slot(id, f);
            if s == StoredSlot::Unset {
                continue;
            }
            h.u64(f.0 as u64);
            match s {
                StoredSlot::Unset => {}
                StoredSlot::Data(v) => h.value(&from_stored(v)),
                StoredSlot::Datas(vs, set) => {
                    h.u64(vs.len() as u64 + ((set as u64) << 40));
                    vs.iter().for_each(|v| h.value(&from_stored(v)));
                }
                StoredSlot::Ref(r) => h.u64(r.map_or(0, |o| 1 + o.0 as u64)),
                StoredSlot::Refs(v, set) => {
                    h.u64(v.len() as u64 + ((set as u64) << 40));
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

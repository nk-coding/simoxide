//! XMI reader reproducing EMF's `XMIResourceImpl` loading with default load options
//! (`XMLHandler`, `XMIHandler`, `SAXXMIHandler`, `XMLHelperImpl`).
//!
//! Adapted from SimOxide's `simoxide-model/src/xmi.rs` (same author, EPL-2.0), which was
//! validated against EMF on the PCM corpus, generalised from generated tables to a dynamic
//! [`MetaRegistry`](qvto_model::MetaRegistry). Reproduced details:
//! - an object's XML attributes are processed before it is attached to its parent, so an IDREF
//!   resolves immediately only against objects whose start tag came earlier;
//! - IDREFs of a single-valued reference with a non-transient opposite that cannot be resolved
//!   immediately are dropped (the opposite end sets them: EMF's `mustAdd` logic);
//! - the remaining unresolved IDREFs are resolved at the end of the document in encounter order,
//!   inserted at their original list position;
//! - many-valued references keep duplicates (`addUnique`); opposites are maintained;
//! - IDs are looked up in containment-tree order (first match wins for duplicate IDs);
//! - `href`s become proxies with URIs resolved against the resource's URI.

use crate::datatypes::parse_value;
use crate::fragment::{id_of, resolve_fragment};
use crate::graph::{Graph, ObjId, ResourceKind, Value};
use crate::uri::Uri;
use qvto_model::{ClassId, Classifier, Datum, FeatureId, FeatureKind, ModelStore, PackageId, ResourceId};
use quick_xml::events::{BytesStart, Event};
use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

pub(crate) const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

struct ForwardRef<'a> {
    obj: ObjId,
    feature: FeatureId,
    ids: Vec<(Cow<'a, str>, i32)>,
    many: bool,
    pos: usize,
}

struct NsDecl<'a> {
    prefix: Cow<'a, str>,
    uri: Cow<'a, str>,
    is_xsi: bool,
}

type Attr<'a> = (Cow<'a, str>, Cow<'a, str>);

enum Frame {
    Wrapper,
    Object(ObjId),
    AttrText {
        obj: ObjId,
        feature: FeatureId,
        text: String,
        nil: bool,
    },
    Skip,
}

/// Namespace URIs and `xsi:schemaLocation` pairs used by a document, found before loading so
/// the caller can register the packages it needs.
#[derive(Debug, Default)]
pub struct Prescan {
    pub ns_uris: Vec<String>,
    pub schema_locations: Vec<(String, String)>,
}

/// Finds the namespace declarations and schema locations of a document.
pub fn prescan(text: &str) -> Prescan {
    let mut out = Prescan::default();
    let mut reader = quick_xml::Reader::from_str(text);
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                for a in e.attributes().with_checks(false).flatten() {
                    let k = String::from_utf8_lossy(a.key.as_ref()).into_owned();
                    let v = a.unescape_value().map(|v| v.into_owned()).unwrap_or_default();
                    if k == "xmlns" || k.starts_with("xmlns:") {
                        if !out.ns_uris.contains(&v) {
                            out.ns_uris.push(v);
                        }
                    } else if k.ends_with(":schemaLocation") {
                        let toks: Vec<&str> = v.split_whitespace().collect();
                        for p in toks.chunks(2) {
                            if p.len() == 2 {
                                out.schema_locations.push((p[0].to_string(), p[1].to_string()));
                            }
                        }
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    out
}

pub(crate) struct Parser<'a> {
    g: &'a mut Graph,
    res: ResourceId,
    res_uri: Uri,
    text: &'a str,
    ns: Vec<NsDecl<'a>>,
    ns_marks: Vec<usize>,
    stack: Vec<Frame>,
    attrs: Vec<Attr<'a>>,
    attr_spans: Vec<(Range<usize>, Range<usize>)>,
    /// ID-attribute values seen so far -> objects, in creation order.
    ids: HashMap<String, Vec<ObjId>>,
    xmi_ids: HashMap<String, ObjId>,
    forward: Vec<ForwardRef<'a>>,
    forward_many: Vec<ForwardRef<'a>>,
    same_doc_proxies: Vec<ObjId>,
    pos: usize,
    first_obj: u32,
    attached: Vec<bool>,
    line: u32,
    line_pos: usize,
}

/// Loads an XMI document into resource `res` of `g`. XML syntax errors abort with `Err`; EMF
/// load errors (unknown classes or features, bad values, dangling IDREFs) are recorded in the
/// resource's `errors`.
pub fn load(g: &mut Graph, res: ResourceId, text: &str) -> Result<(), String> {
    let res_uri = g.resource(res).parsed_uri().trim_fragment();
    let first_obj = g.object_count() as u32;
    let mut p = Parser {
        g,
        res,
        res_uri,
        text,
        ns: Vec::new(),
        ns_marks: Vec::new(),
        stack: Vec::new(),
        attrs: Vec::new(),
        attr_spans: Vec::new(),
        ids: HashMap::new(),
        xmi_ids: HashMap::new(),
        forward: Vec::new(),
        forward_many: Vec::new(),
        same_doc_proxies: Vec::new(),
        pos: 0,
        first_obj,
        attached: Vec::new(),
        line: 1,
        line_pos: 0,
    };
    {
        let r = p.g.resource_mut(res);
        r.encoding = "UTF-8".into();
        r.xml_version = "1.0".into();
    }
    p.read_all()?;
    p.end_document();
    p.g.resource_mut(res).loaded = true;
    Ok(())
}

impl<'a> Parser<'a> {
    fn line(&mut self) -> u32 {
        if self.pos < self.line_pos {
            self.line = 1;
            self.line_pos = 0;
        }
        if self.pos > self.line_pos {
            self.line += memchr::memchr_iter(b'\n', &self.text.as_bytes()[self.line_pos..self.pos]).count() as u32;
            self.line_pos = self.pos;
        }
        self.line
    }

    fn text_of(&self, b: &[u8]) -> Cow<'a, str> {
        let base = self.text.as_ptr() as usize;
        let p = b.as_ptr() as usize;
        if p >= base
            && let Some(s) = self.text.get(p - base..p - base + b.len())
        {
            return Cow::Borrowed(s);
        }
        Cow::Owned(String::from_utf8_lossy(b).into_owned())
    }

    fn emf_error(&mut self, msg: String) {
        let line = self.line();
        self.g.resource_mut(self.res).errors.push(format!("line {line}: {msg}"));
    }

    fn is_attached(&self, o: ObjId) -> bool {
        if o.0 < self.first_obj {
            return true;
        }
        self.attached.get((o.0 - self.first_obj) as usize).copied().unwrap_or(false)
    }

    fn set_attached(&mut self, o: ObjId) {
        let i = (o.0 - self.first_obj) as usize;
        if self.attached.len() <= i {
            self.attached.resize(i + 1, false);
        }
        self.attached[i] = true;
    }

    fn read_all(&mut self) -> Result<(), String> {
        let text = self.text;
        let mut reader = quick_xml::Reader::from_str(text);
        reader.config_mut().trim_text(false);
        reader.config_mut().expand_empty_elements = false;
        loop {
            self.pos = (reader.buffer_position() as usize).min(text.len());
            let ev = match reader.read_event() {
                Ok(ev) => ev,
                Err(e) => return Err(format!("XML error at line {}: {e}", self.line())),
            };
            match ev {
                Event::Decl(d) => {
                    if let Some(Ok(enc)) = d.encoding() {
                        let enc = String::from_utf8_lossy(&enc).into_owned();
                        self.g.resource_mut(self.res).encoding = enc;
                    }
                    if let Ok(v) = d.version() {
                        let v = String::from_utf8_lossy(&v).into_owned();
                        self.g.resource_mut(self.res).xml_version = v;
                    }
                }
                Event::Start(e) => self.start(&e, false)?,
                Event::Empty(e) => self.start(&e, true)?,
                Event::End(_) => self.end(),
                Event::Text(t) => {
                    if let Some(Frame::AttrText { text, .. }) = self.stack.last_mut() {
                        let raw = String::from_utf8_lossy(&t);
                        text.push_str(&raw.replace("\r\n", "\n").replace('\r', "\n"));
                    }
                }
                Event::CData(t) => {
                    if let Some(Frame::AttrText { text, .. }) = self.stack.last_mut() {
                        text.push_str(&String::from_utf8_lossy(&t));
                    }
                }
                Event::GeneralRef(r) => {
                    if let Some(Frame::AttrText { text, .. }) = self.stack.last_mut() {
                        if let Ok(Some(c)) = r.resolve_char_ref() {
                            text.push(c);
                        } else {
                            let name = String::from_utf8_lossy(&r).into_owned();
                            if let Some(s) = quick_xml::escape::resolve_predefined_entity(&name) {
                                text.push_str(s);
                            }
                        }
                    }
                }
                Event::Eof => break,
                _ => {}
            }
        }
        Ok(())
    }

    fn ns_decl(&self, prefix: &str) -> Option<usize> {
        self.ns.iter().rposition(|d| d.prefix == prefix)
    }
    fn resolve_prefix(&self, prefix: &str) -> Option<&str> {
        self.ns_decl(prefix).map(|i| &*self.ns[i].uri)
    }
    /// EMF parses without namespace awareness: `xsi:` attributes are recognised by their
    /// literal prefix.
    fn prefix_is_xsi(&self, prefix: &str) -> bool {
        prefix == "xsi"
    }

    fn package_for(&self, ns_uri: &str) -> Option<PackageId> {
        if let Some(p) = self.g.meta().package_by_ns_uri(ns_uri) {
            return Some(p);
        }
        // old-style: a scheme-less URI matching the last segments of a registered nsURI
        if Uri::parse(ns_uri).scheme().is_none() && !ns_uri.is_empty() {
            for p in self.g.meta().packages() {
                if let Some(u) = self.g.meta().package(p).ns_uri.as_deref()
                    && u.len() > ns_uri.len()
                    && u.ends_with(ns_uri)
                    && u.as_bytes()[u.len() - ns_uri.len() - 1] == b'/'
                {
                    return Some(p);
                }
            }
        }
        None
    }

    /// `prefix:Name` -> class, reporting EMF's errors.
    fn class_for_qname(&mut self, qname: &str) -> Option<ClassId> {
        let (prefix, name) = qname.split_once(':').unwrap_or(("", qname));
        let uri = self.resolve_prefix(prefix).map(|u| u.to_string());
        self.class_in(uri.as_deref(), prefix, name)
    }

    fn class_in(&mut self, uri: Option<&str>, prefix: &str, name: &str) -> Option<ClassId> {
        let Some(uri) = uri else {
            let _ = prefix;
            self.emf_error(format!("Class '{name}' is not found or is abstract."));
            return None;
        };
        let Some(p) = self.package_for(uri) else {
            self.emf_error(format!("Package with uri '{uri}' not found."));
            self.emf_error(format!("Class '{name}' is not found or is abstract."));
            return None;
        };
        match self.g.meta().classifier_by_name(p, name) {
            Some(Classifier::Class(c)) if !self.is_abstract(c) => Some(c),
            _ => {
                self.emf_error(format!("Class '{name}' is not found or is abstract."));
                None
            }
        }
    }

    fn start(&mut self, e: &BytesStart, empty: bool) -> Result<(), String> {
        let mut attrs = std::mem::take(&mut self.attrs);
        attrs.clear();
        let r = self.start_with(e, empty, &mut attrs);
        self.attrs = attrs;
        r
    }

    fn start_with(&mut self, e: &BytesStart, empty: bool, attrs: &mut Vec<Attr<'a>>) -> Result<(), String> {
        self.ns_marks.push(self.ns.len());
        let mut spans = std::mem::take(&mut self.attr_spans);
        spans.clear();
        let raw = e.attributes_raw();
        if scan_attributes(raw, &mut spans) {
            for (k, v) in &spans {
                let key = self.text_of(&raw[k.clone()]);
                let val = self.text_of(&raw[v.clone()]);
                self.add_attribute(key, val, attrs);
            }
        } else {
            for a in e.attributes().with_checks(false) {
                let a = a.map_err(|x| format!("bad attribute: {x}"))?;
                let key = self.text_of(a.key.as_ref());
                let val = self.text_of(&a.value);
                self.add_attribute(key, val, attrs);
            }
        }
        self.attr_spans = spans;
        let qname = self.text_of(e.name().as_ref());
        let (prefix, local) = qname.split_once(':').unwrap_or(("", &qname));
        if self.stack.is_empty()
            && attrs.iter().any(|(k, _)| {
                k.split_once(':').is_some_and(|(p, l)| {
                    (l == "schemaLocation" || l == "noNamespaceSchemaLocation") && self.prefix_is_xsi(p)
                })
            })
        {
            self.g.resource_mut(self.res).save_schema_location = true;
        }
        let frame = match self.stack.last() {
            None | Some(Frame::Wrapper) => {
                let is_root = self.stack.is_empty();
                if let Some(u) = self.resolve_prefix("xmi")
                    && let Some(v) = u.strip_prefix("http://schema.omg.org/spec/XMI/")
                {
                    let v = v.to_string();
                    self.g.resource_mut(self.res).xmi_version = v;
                }
                if is_root && local == "XMI" {
                    Frame::Wrapper
                } else {
                    let uri = self.resolve_prefix(prefix).map(|u| u.to_string());
                    match self.class_in(uri.as_deref(), prefix, local) {
                        Some(c) => {
                            let o = self.create(c, attrs);
                            self.set_attached(o);
                            self.g.resource_mut(self.res).roots.push(o);
                            self.g.objs[o.index()].resource = Some(self.res);
                            Frame::Object(o)
                        }
                        None => Frame::Skip,
                    }
                }
            }
            Some(Frame::Object(p)) => {
                let p = *p;
                self.child_element(p, local, attrs)
            }
            Some(Frame::AttrText { .. }) | Some(Frame::Skip) => Frame::Skip,
        };
        self.stack.push(frame);
        if empty {
            self.end();
        }
        Ok(())
    }

    fn is_abstract(&self, c: ClassId) -> bool {
        let ci = self.g.meta().class(c);
        ci.is_abstract || ci.is_interface
    }

    fn add_attribute(&mut self, key: Cow<'a, str>, val: Cow<'a, str>, attrs: &mut Vec<Attr<'a>>) {
        let val = normalize_attr(val);
        if key == "xmlns" {
            self.push_ns(Cow::Borrowed(""), val);
        } else if let Some(p) = key.strip_prefix("xmlns:") {
            let p = self.text_of(p.as_bytes());
            self.push_ns(p, val);
        } else {
            attrs.push((key, val));
        }
    }

    fn push_ns(&mut self, prefix: Cow<'a, str>, uri: Cow<'a, str>) {
        let is_xsi = uri == XSI_NS;
        self.ns.push(NsDecl { prefix, uri, is_xsi });
    }

    fn is_xsi_type(&self, key: &str) -> bool {
        match key.split_once(':') {
            Some((p, "type")) => self.prefix_is_xsi(p),
            _ => false,
        }
    }

    fn child_element(&mut self, parent: ObjId, local: &str, attrs: &[Attr<'a>]) -> Frame {
        let class = self.g.class_of(parent);
        let Some(f) = self.g.meta().feature_by_name(class, local) else {
            let msg = format!(
                "Feature '{local}' not found. ({})",
                self.g.meta().qualified_name(class)
            );
            self.emf_error(msg);
            return Frame::Skip;
        };
        let fi = self.g.meta().feature(f);
        match fi.kind {
            FeatureKind::Attribute => {
                let nil = attrs
                    .iter()
                    .any(|(k, v)| k.split_once(':').is_some_and(|(p, n)| n == "nil" && self.prefix_is_xsi(p)) && v == "true");
                Frame::AttrText {
                    obj: parent,
                    feature: f,
                    text: String::new(),
                    nil,
                }
            }
            FeatureKind::Reference => {
                let target = fi.class_type();
                let xsi_type = attrs
                    .iter()
                    .find(|(k, _)| self.is_xsi_type(k) || k == "xmi:type")
                    .map(|(_, v)| v.clone());
                let class = match xsi_type {
                    Some(t) => match self.class_for_qname(&t) {
                        Some(c) => c,
                        None => return Frame::Skip,
                    },
                    None => match target {
                        Some(c) => c,
                        None => {
                            self.emf_error(format!("no type for feature {local}"));
                            return Frame::Skip;
                        }
                    },
                };
                if self.is_abstract(class) {
                    let n = self.g.meta().class(class).name.to_string();
                    self.emf_error(format!("Class '{n}' is not found or is abstract."));
                    return Frame::Skip;
                }
                let o = self.create(class, attrs);
                self.set_feature_value(parent, f, Some(o), -1);
                self.set_attached(o);
                Frame::Object(o)
            }
        }
    }

    /// Creates an object and processes its XML attributes (before it is attached, like EMF).
    fn create(&mut self, class: ClassId, attrs: &[Attr<'a>]) -> ObjId {
        let o = self.g.new_object(class);
        for (k, v) in attrs {
            let k = &**k;
            if k == "xmi:id" {
                self.xmi_ids.insert(v.to_string(), o);
                self.g.set_xmi_id(self.res, o, Some(v));
                continue;
            }
            if k == "href" {
                self.handle_proxy(o, v);
                continue;
            }
            let (prefix, local) = k.split_once(':').unwrap_or(("", k));
            if !prefix.is_empty() && self.prefix_is_xsi(prefix) {
                if local == "schemaLocation" || local == "noNamespaceSchemaLocation" {
                    self.g.resource_mut(self.res).save_schema_location = true;
                }
                continue;
            }
            if matches!(k, "xmi:version" | "xmi:type" | "xmi:uuid") {
                continue;
            }
            let Some(f) = self.g.meta().feature_by_name(class, local) else {
                let msg = format!(
                    "Feature '{local}' not found. ({})",
                    self.g.meta().qualified_name(class)
                );
                self.emf_error(msg);
                continue;
            };
            match self.g.meta().feature(f).kind {
                FeatureKind::Attribute => self.set_attr_from_string(o, f, v),
                FeatureKind::Reference => self.set_value_from_id(o, f, v.clone()),
            }
        }
        o
    }

    fn parse(&self, f: FeatureId, s: &str) -> Result<Value, String> {
        let ty = self.g.meta().feature(f).ty;
        parse_value(self.g.meta(), ty, s, Some(self.g.ecore().package))
    }

    fn set_attr_from_string(&mut self, o: ObjId, f: FeatureId, v: &str) {
        if self.g.meta().feature(f).is_many() {
            // XMLHelperImpl: space-separated tokens, each added
            for tok in v.split(' ').filter(|t| !t.is_empty()) {
                match self.parse(f, tok) {
                    Ok(val) => self.g.add_unique(o, f, val.as_datum(), None),
                    Err(m) => self.bad_value(f, tok, &m),
                }
            }
            if self.g.len(o, f) == 0 {
                let _ = self.g.clear(o, f);
            }
            return;
        }
        match self.parse(f, v) {
            Ok(val) => {
                if self.g.meta().class(self.g.class_of(o)).id_attribute == Some(f) {
                    self.ids.entry(v.to_string()).or_default().push(o);
                }
                if let Err(e) = self.g.set(o, f, val.as_datum()) {
                    self.bad_value(f, v, &e.to_string());
                }
            }
            Err(m) => self.bad_value(f, v, &m),
        }
    }

    fn bad_value(&mut self, f: FeatureId, v: &str, why: &str) {
        let n = self.g.meta().feature(f).name.to_string();
        self.emf_error(format!("Value '{v}' is not legal for feature '{n}': {why}"));
    }

    fn handle_proxy(&mut self, o: ObjId, href: &str) {
        let u = Uri::parse(href);
        let resolved = if u.is_relative() && u.has_relative_path() {
            u.resolve(&self.res_uri)
        } else {
            u
        };
        if resolved.trim_fragment() == self.res_uri {
            self.same_doc_proxies.push(o);
        }
        let s = resolved.to_string();
        self.g.set_proxy_uri(o, Some(&s));
    }

    /// `XMLResource.getEObject(fragment)` against the tree built so far.
    fn lookup(&self, frag: &str) -> Option<ObjId> {
        if frag.starts_with('/') {
            return resolve_fragment(self.g, self.res, frag);
        }
        if let Some(o) = self.xmi_ids.get(frag) {
            return Some(*o);
        }
        let cands = self.ids.get(frag)?;
        let live: Vec<ObjId> = cands
            .iter()
            .copied()
            .filter(|o| self.is_attached(*o) && self.g.resource_of(*o) == Some(self.res) && id_of(self.g, *o).as_deref() == Some(frag))
            .collect();
        match live.len() {
            0 => None,
            1 => Some(live[0]),
            _ => self.g.resource_contents(self.res).into_iter().find(|o| live.contains(o)),
        }
    }

    /// `XMLHandler.setValueFromId`.
    fn set_value_from_id(&mut self, o: ObjId, f: FeatureId, ids: Cow<'a, str>) {
        let mut is_first = true;
        let mut must_add = false;
        let mut must_add_or_not_opposite_many = false;
        let mut pending: Vec<(Cow<'a, str>, i32)> = Vec::new();
        let mut position: i32 = 0;
        let mut qname: Option<String> = None;
        let fi = self.g.meta().feature(f).clone();
        for tok in id_tokens(&ids) {
            let mut id = tok;
            if let Some(i) = tok.find('#') {
                if i == 0 {
                    id = &tok[1..];
                } else {
                    let class = match qname.take() {
                        Some(q) => self.class_for_qname(&q),
                        None => match fi.class_type() {
                            Some(c) if !self.is_abstract(c) => Some(c),
                            Some(c) => {
                                let n = self.g.meta().class(c).name.to_string();
                                self.emf_error(format!("Class '{n}' is not found or is abstract."));
                                None
                            }
                            None => None,
                        },
                    };
                    if let Some(c) = class {
                        let p = self.g.new_object(c);
                        self.handle_proxy(p, tok);
                        self.set_feature_value(o, f, Some(p), -1);
                    }
                    position += 1;
                    continue;
                }
            } else if tok.contains(':') {
                qname = Some(tok.to_string());
                continue;
            }
            if is_first {
                match fi.opposite {
                    None => {
                        must_add = true;
                        must_add_or_not_opposite_many = true;
                    }
                    Some(opp) => {
                        let oi = self.g.meta().feature(opp);
                        must_add = oi.transient || fi.is_many();
                        must_add_or_not_opposite_many = must_add || !oi.is_many();
                    }
                }
                is_first = false;
            }
            if must_add_or_not_opposite_many && let Some(t) = self.lookup(id) {
                self.set_feature_value(o, f, Some(t), -1);
                qname = None;
                position += 1;
                continue;
            }
            if must_add {
                let id = self.text_of(id.as_bytes());
                pending.push((id, position));
            }
            qname = None;
            position += 1;
        }
        if position == 0 {
            self.set_feature_value(o, f, None, -2);
            return;
        }
        if pending.is_empty() {
            return;
        }
        let pos = self.pos;
        if pending.len() > 5 {
            self.forward_many.push(ForwardRef { obj: o, feature: f, ids: pending, many: true, pos });
        } else {
            for p in pending {
                self.forward.push(ForwardRef { obj: o, feature: f, ids: vec![p], many: false, pos });
            }
        }
    }

    /// `XMLHelperImpl.setValue` for references. `false` on an error EMF would report.
    fn set_feature_value(&mut self, o: ObjId, f: FeatureId, v: Option<ObjId>, position: i32) -> bool {
        let fi = self.g.meta().feature(f).clone();
        if let Some(t) = v
            && let Err(e) = self.g.check_value(f, &Datum::Obj(t))
        {
            let n = fi.name.to_string();
            self.emf_error(format!("Value is not legal for feature '{n}': {e}"));
            return true;
        }
        if !fi.is_many() {
            if let Err(e) = self.g.set(o, f, v.map_or(Datum::Null, Datum::Obj)) {
                self.emf_error(e.to_string());
            }
            return true;
        }
        let move_kind = fi.opposite.is_some_and(|opp| {
            let oi = self.g.meta().feature(opp);
            !oi.transient && oi.is_many()
        });
        let index_of = |g: &Graph, x: ObjId| (0..g.len(o, f)).position(|i| g.get_at(o, f, i) == Datum::Obj(x));
        match (position, v) {
            (-2, _) => {
                let _ = self.g.clear(o, f);
                true
            }
            (_, None) => true,
            (-1, Some(v)) => {
                if o == v {
                    let _ = self.g.add(o, f, Datum::Obj(v), None);
                } else {
                    self.g.add_unique(o, f, Datum::Obj(v), None);
                }
                true
            }
            (p, Some(v)) => {
                let p = p as usize;
                if o == v || move_kind {
                    match index_of(self.g, v) {
                        Some(i) => self.g.move_to(o, f, p, i).is_ok(),
                        None if o == v => {
                            if p > self.g.len(o, f) {
                                return false;
                            }
                            self.g.add_unique(o, f, Datum::Obj(v), Some(p));
                            true
                        }
                        None => false,
                    }
                } else {
                    if p > self.g.len(o, f) {
                        return false;
                    }
                    self.g.add_unique(o, f, Datum::Obj(v), Some(p));
                    true
                }
            }
        }
    }

    fn end(&mut self) {
        if let Some(mark) = self.ns_marks.pop() {
            self.ns.truncate(mark);
        }
        if let Some(Frame::AttrText { obj, feature, text, nil }) = self.stack.pop() {
            if self.g.meta().feature(feature).is_many() {
                if nil {
                    self.g.add_unique(obj, feature, Datum::Null, None);
                } else {
                    match self.parse(feature, &text) {
                        Ok(v) => self.g.add_unique(obj, feature, v.as_datum(), None),
                        Err(m) => self.bad_value(feature, &text, &m),
                    }
                }
            } else if nil {
                let _ = self.g.set(obj, feature, Datum::Null);
            } else {
                self.set_attr_from_string(obj, feature, &text);
            }
        }
    }

    /// EMF `handleForwardReferences(true)`.
    fn end_document(&mut self) {
        // same-document proxies of bidirectional references: connect the real object
        let proxies = std::mem::take(&mut self.same_doc_proxies);
        for p in proxies {
            let frag = self
                .g
                .proxy_uri(p)
                .and_then(|u| u.split_once('#'))
                .map(|(_, f)| f.to_string());
            let Some(frag) = frag else { continue };
            let Some(t) = self.lookup(&frag) else { continue };
            let class = self.g.class_of(p);
            let feats: Vec<FeatureId> = self.g.meta().class(class).all_features.to_vec();
            for r in feats {
                let ri = self.g.meta().feature(r);
                let Some(opp) = ri.opposite else { continue };
                if ri.kind == FeatureKind::Attribute || ri.is_many() || !self.g.is_set(p, r) {
                    continue;
                }
                let Some(holder) = self.g.get(p, r).as_obj() else { continue };
                if self.g.meta().feature(opp).is_many() {
                    let n = self.g.len(holder, opp);
                    if let Some(pi) = (0..n).position(|i| self.g.get_at(holder, opp, i) == Datum::Obj(p)) {
                        let has_t = (0..n).any(|i| self.g.get_at(holder, opp, i) == Datum::Obj(t));
                        let _ = self.g.remove_at(holder, opp, pi);
                        if !has_t {
                            self.g.add_unique(holder, opp, Datum::Obj(t), Some(pi));
                        }
                    }
                } else {
                    let _ = self.g.set(holder, opp, Datum::Obj(t));
                }
                break;
            }
        }
        let fwd = std::mem::take(&mut self.forward);
        for r in fwd {
            let (id, pos) = &r.ids[0];
            match self.lookup(id) {
                Some(t) => {
                    if !self.set_feature_value(r.obj, r.feature, Some(t), *pos) {
                        self.pos = r.pos;
                        let n = self.g.meta().feature(r.feature).name.to_string();
                        self.emf_error(format!("cannot set {n} to {id}"));
                    }
                }
                None => {
                    self.pos = r.pos;
                    self.emf_error(format!("Unresolved reference '{id}'."));
                }
            }
        }
        let fwd = std::mem::take(&mut self.forward_many);
        for r in fwd {
            debug_assert!(r.many);
            let resolved: Vec<(Option<ObjId>, i32)> = r.ids.iter().map(|(id, p)| (self.lookup(id), *p)).collect();
            for ((id, _), (t, _)) in r.ids.iter().zip(&resolved) {
                if t.is_none() {
                    self.pos = r.pos;
                    self.emf_error(format!("Unresolved reference '{id}'."));
                }
            }
            for (t, p) in resolved {
                let Some(t) = t else { continue };
                let p = p as usize;
                let n = self.g.len(r.obj, r.feature);
                let found = (0..n).position(|i| self.g.get_at(r.obj, r.feature, i) == Datum::Obj(t));
                let many_opp = self.g.meta().feature(r.feature).opposite.is_some_and(|o| self.g.meta().feature(o).is_many());
                let ok = match found {
                    Some(i) if r.obj == t || many_opp => self.g.move_to(r.obj, r.feature, p, i).is_ok(),
                    _ => {
                        if p > n {
                            false
                        } else {
                            self.g.add_unique(r.obj, r.feature, Datum::Obj(t), Some(p));
                            true
                        }
                    }
                };
                if !ok {
                    let n = self.g.meta().feature(r.feature).name.to_string();
                    self.emf_error(format!("cannot insert into {n}"));
                }
            }
        }
    }
}

/// The resource kind for a URI (by file extension, as the test suite registers factories).
pub fn kind_for_uri(uri: &Uri) -> ResourceKind {
    match uri.file_extension() {
        Some("ecore") => ResourceKind::Ecore,
        _ => ResourceKind::Xmi,
    }
}

/// The non-empty tokens of an IDREF(S) value separated by XML whitespace.
fn id_tokens(s: &str) -> impl Iterator<Item = &str> {
    s.split([' ', '\t', '\n', '\r', '\x0c']).filter(|t| !t.is_empty())
}

/// Splits the attribute part of a start tag into key and value spans the way quick-xml does.
/// `false` if the syntax is anything else; the caller then uses quick-xml's parser.
fn scan_attributes(raw: &[u8], out: &mut Vec<(Range<usize>, Range<usize>)>) -> bool {
    let ws = |b: u8| matches!(b, b' ' | b'\r' | b'\n' | b'\t');
    let n = raw.len();
    let mut i = 0;
    loop {
        while i < n && ws(raw[i]) {
            i += 1;
        }
        if i == n {
            return true;
        }
        let ks = i;
        if raw[i] == b'=' {
            return false;
        }
        i += 1;
        while i < n && raw[i] != b'=' && !ws(raw[i]) {
            i += 1;
        }
        let ke = i;
        while i < n && ws(raw[i]) {
            i += 1;
        }
        if i == n || raw[i] != b'=' {
            return false;
        }
        i += 1;
        while i < n && ws(raw[i]) {
            i += 1;
        }
        if i == n || !matches!(raw[i], b'"' | b'\'') {
            return false;
        }
        let q = raw[i];
        i += 1;
        let Some(len) = memchr::memchr(q, &raw[i..]) else {
            return false;
        };
        out.push((ks..ke, i..i + len));
        i += len + 1;
    }
}

/// XML attribute-value normalisation (literal whitespace -> space) followed by unescaping.
fn normalize_attr(s: Cow<'_, str>) -> Cow<'_, str> {
    let b = s.as_bytes();
    if !b.iter().fold(false, |acc, &c| acc | (c < 0x20) | (c == b'&')) {
        return s;
    }
    let ws = memchr::memchr3(b'\n', b'\r', b'\t', b).is_some();
    let amp = memchr::memchr(b'&', b).is_some();
    if !ws && !amp {
        return s;
    }
    let s: String = if ws {
        s.replace("\r\n", " ").replace(['\n', '\r', '\t'], " ")
    } else {
        s.into_owned()
    };
    if amp {
        match quick_xml::escape::unescape(&s) {
            Ok(u) => Cow::Owned(u.into_owned()),
            Err(_) => Cow::Owned(s),
        }
    } else {
        Cow::Owned(s)
    }
}

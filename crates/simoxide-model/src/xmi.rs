//! XMI reader reproducing EMF's `XMIResourceImpl` loading (EMF 2.33, default load options).
//!
//! Faithfully reproduced details (see `XMLHandler`, `XMLHelperImpl`, `SAXXMIHandler`):
//! - an object's XML attributes are processed *before* the object is attached to its parent, so
//!   an IDREF can only be resolved immediately against objects whose start tag came earlier;
//! - IDREFs of a single-valued reference with a non-transient opposite (e.g. `successor` /
//!   `predecessor`) that cannot be resolved immediately are dropped; the opposite end is expected
//!   to set them (EMF's `mustAdd` logic);
//! - the remaining unresolved IDREFs are resolved at the end of the document in encounter order,
//!   inserted at their original list position;
//! - many-valued references keep duplicates (`addUnique`); bidirectional references maintain
//!   their opposite (last write wins);
//! - IDs are looked up in containment-tree order (first match wins for duplicate IDs).

use crate::diag::{Diagnostics, Level};
use crate::fxhash::{FxHashMap, StrMap};
use crate::meta::{self, ClassId, DataKind, FeatureId, FeatureKind};
use crate::raw::{Graph, Obj, ObjId, ResId, Slot, parse_value};
use quick_xml::events::{BytesStart, Event};
use std::borrow::Cow;
use std::ops::Range;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

pub(crate) const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// Options affecting leniency where EMF would fail.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ParseOptions {
    /// Map namespace URIs of other PCM versions (5.0, 5.1, `sdq.ipd.uka.de`) to 5.2 packages.
    pub tolerant_namespaces: bool,
}

struct ForwardRef<'a> {
    obj: ObjId,
    feature: FeatureId,
    ids: Vec<(Cow<'a, str>, i32)>,
    many: bool,
    /// Byte position of the element (for the line number of diagnostics).
    pos: usize,
}

/// Objects with a given ID-attribute value, in creation order (almost always one).
enum IdObjs {
    One(ObjId),
    Many(Vec<ObjId>),
}

impl IdObjs {
    fn as_slice(&self) -> &[ObjId] {
        match self {
            IdObjs::One(o) => std::slice::from_ref(o),
            IdObjs::Many(v) => v,
        }
    }
}

/// A namespace declaration in scope.
struct NsDecl<'a> {
    prefix: Cow<'a, str>,
    uri: Cow<'a, str>,
    is_xsi: bool,
    /// Memo of `package_for(uri)`.
    package: Option<Option<meta::PackageId>>,
}

/// An XML attribute: qualified name and normalised value, borrowed from the text where possible.
type Attr<'a> = (Cow<'a, str>, Cow<'a, str>);

enum Frame {
    Wrapper,
    Object(ObjId),
    AttrText {
        obj: ObjId,
        feature: FeatureId,
        text: String,
    },
    Skip,
}

pub(crate) struct Parser<'a> {
    g: &'a mut Graph,
    res: ResId,
    res_key: &'a str,
    resolve_href: &'a dyn Fn(&str) -> String,
    diags: &'a mut Diagnostics,
    opts: ParseOptions,
    text: &'a str,
    ns: Vec<NsDecl<'a>>,
    ns_marks: Vec<usize>,
    stack: Vec<Frame>,
    /// Reused buffer for the attributes of the current start tag.
    attrs: Vec<Attr<'a>>,
    attr_spans: Vec<(Range<usize>, Range<usize>)>,
    /// ID-attribute values seen so far -> objects, in creation order.
    ids: FxHashMap<Cow<'a, str>, IdObjs>,
    xmi_ids: FxHashMap<Cow<'a, str>, ObjId>,
    /// `href` locations (without fragment) -> resolved resource key.
    href_keys: FxHashMap<&'a str, Box<str>>,
    forward: Vec<ForwardRef<'a>>,
    forward_many: Vec<ForwardRef<'a>>,
    same_doc_proxies: Vec<ObjId>,
    /// Byte position of the current event (its line is computed only when needed).
    pos: usize,
    /// First object created by this parse, and the byte positions of the created objects.
    first_obj: usize,
    obj_pos: Vec<usize>,
    /// Line number cache: `line` is the 1-based line of byte position `line_pos`.
    line: u32,
    line_pos: usize,
    warned_ns: Vec<String>,
}

/// Result of parsing one resource: the final id index (first object in tree order per id).
pub(crate) struct Parsed {
    pub id_index: StrMap,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(
        g: &'a mut Graph,
        res: ResId,
        res_key: &'a str,
        resolve_href: &'a dyn Fn(&str) -> String,
        diags: &'a mut Diagnostics,
        opts: ParseOptions,
    ) -> Self {
        Parser {
            g,
            res,
            res_key,
            resolve_href,
            diags,
            opts,
            text: "",
            ns: Vec::new(),
            ns_marks: Vec::new(),
            stack: Vec::new(),
            attrs: Vec::new(),
            attr_spans: Vec::new(),
            ids: FxHashMap::default(),
            xmi_ids: FxHashMap::default(),
            href_keys: FxHashMap::default(),
            forward: Vec::new(),
            forward_many: Vec::new(),
            same_doc_proxies: Vec::new(),
            pos: 0,
            first_obj: 0,
            obj_pos: Vec::new(),
            line: 1,
            line_pos: 0,
            warned_ns: Vec::new(),
        }
    }

    /// 1-based line of the current position (lines before it counted on demand).
    fn line(&mut self) -> u32 {
        if self.pos < self.line_pos {
            self.line = 1;
            self.line_pos = 0;
        }
        if self.pos > self.line_pos {
            self.line += count_newlines(&self.text.as_bytes()[self.line_pos..self.pos]);
            self.line_pos = self.pos;
        }
        self.line
    }

    fn loc(&mut self) -> String {
        let line = self.line();
        format!("{}:{}", self.g.resources[self.res.0 as usize].name, line)
    }

    /// The text of `b` if it is a slice of the document (O(1), no copy), else a lossy copy.
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

    /// An EMF load error (the reference would reject the resource).
    fn emf_error(&mut self, kind: &'static str, msg: String) {
        let loc = self.loc();
        self.g.resources[self.res.0 as usize]
            .errors
            .push(format!("{loc}: {msg}"));
        self.diags.push(Level::Error, kind, loc, msg);
    }

    pub(crate) fn parse(mut self, text: &'a str) -> Result<Parsed, String> {
        self.text = text;
        self.first_obj = self.g.objs.len();
        // about one object per 100 bytes of XMI
        self.g.objs.reserve(text.len() / 100);
        self.obj_pos.reserve(text.len() / 100);
        let r = self.read_all();
        self.set_lines();
        r?;
        self.end_document();
        Ok(Parsed {
            id_index: self.final_index(),
        })
    }

    /// Source lines of the objects created by this parse (from their byte positions, in one
    /// pass over the text; object creation positions are non-decreasing).
    fn set_lines(&mut self) {
        let mut nl = memchr::memchr_iter(b'\n', self.text.as_bytes()).peekable();
        let mut line = 1;
        let objs = &mut self.g.objs[self.first_obj..];
        debug_assert_eq!(objs.len(), self.obj_pos.len());
        let hints = slot_hints();
        for (o, &p) in objs.iter_mut().zip(&self.obj_pos) {
            while nl.next_if(|&x| x < p).is_some() {
                line += 1;
            }
            o.line = line;
            let h = &hints[o.class.0 as usize];
            if o.proxy.is_none() && o.slots.len() > h.load(Relaxed) as usize {
                h.store(o.slots.len().min(255) as u8, Relaxed);
            }
        }
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
                Event::Start(e) => self.start(&e, false)?,
                Event::Empty(e) => {
                    self.start(&e, true)?;
                }
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

    fn prefix_is_xsi(&self, prefix: &str) -> bool {
        self.ns_decl(prefix).is_some_and(|i| self.ns[i].is_xsi)
    }

    /// `package_for` of the namespace declared for `prefix` (`Err` if undeclared).
    fn package_of_prefix(&mut self, prefix: &str) -> Result<Option<meta::PackageId>, ()> {
        let i = self.ns_decl(prefix).ok_or(())?;
        if let Some(p) = self.ns[i].package {
            return Ok(p);
        }
        let uri = self.ns[i].uri.clone();
        let p = self.package_for(&uri);
        self.ns[i].package = Some(p);
        Ok(p)
    }

    fn package_for(&mut self, ns_uri: &str) -> Option<meta::PackageId> {
        if let Some(p) = meta::package_by_ns(ns_uri) {
            return Some(p);
        }
        if self.opts.tolerant_namespaces
            && let Some(p) = meta::package_by_ns_tolerant(ns_uri)
        {
            if !self.warned_ns.iter().any(|w| w == ns_uri) {
                self.warned_ns.push(ns_uri.to_string());
                let msg = format!(
                    "namespace {ns_uri} is not PCM 5.2; read as {}",
                    p.def().ns_uri
                );
                // EMF 5.2.2 cannot load this at all
                self.emf_error("foreign-namespace", msg);
            }
            return Some(p);
        }
        None
    }

    /// `prefix:Name` (or `Name` in the default namespace) -> class.
    fn class_for_qname(&mut self, qname: &str) -> Result<ClassId, String> {
        let (prefix, name) = qname.split_once(':').unwrap_or(("", qname));
        let Ok(pkg) = self.package_of_prefix(prefix) else {
            return Err(format!(
                "undeclared namespace prefix '{prefix}' in type '{qname}'"
            ));
        };
        let Some(pkg) = pkg else {
            let uri = self.resolve_prefix(prefix).unwrap_or("");
            return Err(format!("package with uri '{uri}' not found"));
        };
        meta::class_by_name(pkg, name)
            .ok_or_else(|| format!("class '{name}' is not found or is abstract"))
    }

    fn start(&mut self, e: &BytesStart, empty: bool) -> Result<(), String> {
        let mut attrs = std::mem::take(&mut self.attrs);
        attrs.clear();
        let r = self.start_with(e, empty, &mut attrs);
        self.attrs = attrs;
        r
    }

    fn start_with(
        &mut self,
        e: &BytesStart,
        empty: bool,
        attrs: &mut Vec<Attr<'a>>,
    ) -> Result<(), String> {
        // namespace declarations
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
            // unusual syntax: quick-xml's parser (and its errors)
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
        let frame = match self.stack.last() {
            None | Some(Frame::Wrapper) => {
                let is_root = self.stack.is_empty();
                let uri = self.resolve_prefix(prefix).unwrap_or("").to_string();
                if is_root && local == "XMI" && uri.starts_with("http://www.omg.org") {
                    Frame::Wrapper
                } else {
                    let pkg = match self.package_of_prefix(prefix) {
                        Ok(p) => p,
                        Err(()) => self.package_for(""),
                    };
                    let class = match pkg {
                        Some(p) => meta::class_by_name(p, local)
                            .ok_or_else(|| format!("class '{local}' not found")),
                        None => Err(format!("package with uri '{uri}' not found")),
                    };
                    match class {
                        Ok(c) if !c.def().is_abstract => {
                            let o = self.create(c, attrs);
                            self.g[o].attached = true;
                            self.g.resources[self.res.0 as usize].roots.push(o);
                            Frame::Object(o)
                        }
                        Ok(c) => {
                            self.emf_error(
                                "abstract-class",
                                format!("cannot instantiate abstract class {}", c.name()),
                            );
                            Frame::Skip
                        }
                        Err(m) => {
                            self.emf_error("unknown-type", m);
                            Frame::Skip
                        }
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

    /// A namespace declaration or an attribute of the current start tag.
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
        self.ns.push(NsDecl {
            prefix,
            uri,
            is_xsi,
            package: None,
        });
    }

    fn child_element(&mut self, parent: ObjId, local: &str, attrs: &[Attr<'a>]) -> Frame {
        let class = self.g[parent].class;
        let Some(f) = class.feature(local) else {
            let msg = format!(
                "feature '{local}' not found in class {}",
                class.qualified_name()
            );
            self.emf_error("unknown-feature", msg);
            return Frame::Skip;
        };
        match f.def().kind {
            FeatureKind::Attribute { .. } => {
                // xsi:nil not supported (never used by PCM tooling)
                Frame::AttrText {
                    obj: parent,
                    feature: f,
                    text: String::new(),
                }
            }
            FeatureKind::Reference { target, .. } => {
                let xsi_type = attrs
                    .iter()
                    .find(|(k, _)| self.is_xsi_type(k) || k == "xmi:type")
                    .map(|(_, v)| v.clone());
                let class = match xsi_type {
                    Some(t) => match self.class_for_qname(&t) {
                        Ok(c) => c,
                        Err(m) => {
                            self.emf_error("unknown-type", m);
                            return Frame::Skip;
                        }
                    },
                    None if target.0 != u32::MAX => target,
                    None => {
                        self.emf_error("unknown-type", format!("no type for feature {}", f.name()));
                        return Frame::Skip;
                    }
                };
                if class.def().is_abstract {
                    let msg = format!(
                        "cannot instantiate abstract class {} (missing xsi:type?)",
                        class.name()
                    );
                    self.emf_error("abstract-class", msg);
                    return Frame::Skip;
                }
                let o = self.create(class, attrs);
                self.set_feature_value(parent, f, Some(o), -1);
                self.g[o].attached = true;
                Frame::Object(o)
            }
        }
    }

    fn is_xsi_type(&self, key: &str) -> bool {
        match key.split_once(':') {
            Some((p, "type")) => self.prefix_is_xsi(p),
            _ => false,
        }
    }

    /// A new object; `presize`: reserve the usual number of slots of the class (not for proxies).
    fn new_obj(&mut self, class: ClassId, presize: bool) -> ObjId {
        let id = ObjId(self.g.objs.len() as u32);
        // the line is set at the end (`set_lines`)
        self.obj_pos.push(self.pos);
        let line = 0;
        let cap = if presize {
            slot_hints()[class.0 as usize].load(Relaxed) as usize
        } else {
            0
        };
        self.g.objs.push(Obj {
            class,
            resource: self.res,
            container: None,
            proxy: None,
            xmi_id: None,
            slots: Vec::with_capacity(cap),
            line,
            attached: false,
        });
        id
    }

    /// Creates an object and processes its XML attributes (before it is attached, like EMF).
    fn create(&mut self, class: ClassId, attrs: &[Attr<'a>]) -> ObjId {
        let proxy = attrs.iter().any(|(k, _)| k == "href");
        let o = self.new_obj(class, !proxy);
        for (k, v) in attrs {
            let k = &**k;
            if k == "xmi:id" {
                self.g[o].xmi_id = Some((**v).into());
                self.xmi_ids.insert(v.clone(), o);
                continue;
            }
            if k == "href" {
                self.handle_proxy(o, v);
                continue;
            }
            let (prefix, local) = k.split_once(':').unwrap_or(("", k));
            if !prefix.is_empty() && self.prefix_is_xsi(prefix) {
                continue;
            }
            if matches!(k, "xmi:version" | "xmi:type" | "xmi:uuid") {
                continue;
            }
            let Some(f) = class.feature(local) else {
                let msg = format!(
                    "feature '{local}' not found in class {}",
                    class.qualified_name()
                );
                self.emf_error("unknown-feature", msg);
                continue;
            };
            match f.def().kind {
                FeatureKind::Attribute { data } => self.set_attr_from_string(o, f, data, v.clone()),
                FeatureKind::Reference { .. } => self.set_value_from_id(o, f, v.clone()),
            }
        }
        o
    }

    fn set_attr_from_string(&mut self, o: ObjId, f: FeatureId, data: DataKind, v: Cow<'a, str>) {
        if f.def().many {
            // XMLHelperImpl: space separated tokens, each added
            for tok in v.split(' ').filter(|t| !t.is_empty()) {
                match parse_value(data, tok) {
                    Some(val) => self.g.add_attr(o, f, val),
                    None => self.bad_value(f, tok),
                }
            }
            if self.g.attrs(o, f).is_empty() {
                self.g.clear_attrs(o, f);
            }
        } else {
            match parse_value(data, &v) {
                Some(val) => {
                    if f.def().is_id {
                        match self.ids.get_mut(&*v) {
                            Some(IdObjs::Many(l)) => l.push(o),
                            Some(e @ IdObjs::One(_)) => {
                                let IdObjs::One(first) = *e else {
                                    unreachable!()
                                };
                                *e = IdObjs::Many(vec![first, o]);
                            }
                            None => {
                                self.ids.insert(v, IdObjs::One(o));
                            }
                        }
                    }
                    self.g.set_attr(o, f, val)
                }
                None => self.bad_value(f, &v),
            }
        }
    }

    fn bad_value(&mut self, f: FeatureId, v: &str) {
        let msg = format!("illegal value {v:?} for {}", f.name());
        self.emf_error("illegal-value", msg);
    }

    fn handle_proxy(&mut self, o: ObjId, href: &str) {
        let (loc, frag) = href.split_once('#').unwrap_or((href, ""));
        let uncached: Box<str>;
        let key: &str = if loc.is_empty() {
            self.res_key
        } else {
            // cache locations borrowed from the document (all but normalised values)
            match self.text_of(loc.as_bytes()) {
                Cow::Borrowed(l) => {
                    if !self.href_keys.contains_key(l) {
                        let k = (self.resolve_href)(l).into();
                        self.href_keys.insert(l, k);
                    }
                    &self.href_keys[l]
                }
                Cow::Owned(_) => {
                    uncached = (self.resolve_href)(loc).into();
                    &uncached
                }
            }
        };
        let same_doc = key == self.res_key;
        let mut uri = String::with_capacity(key.len() + 1 + frag.len());
        uri.push_str(key);
        uri.push('#');
        uri.push_str(frag);
        if same_doc {
            self.same_doc_proxies.push(o);
        }
        self.g[o].proxy = Some(uri.into());
    }

    /// `XMLResource.getEObject(fragment)` against the tree as built so far.
    pub(crate) fn lookup(&self, frag: &str) -> Option<ObjId> {
        if frag.starts_with('/') {
            return navigate_path(self.g, &self.g.resources[self.res.0 as usize].roots, frag);
        }
        if let Some(o) = self.xmi_ids.get(frag) {
            return Some(*o);
        }
        let cands = self.ids.get(frag)?.as_slice();
        let mut live = cands
            .iter()
            .copied()
            .filter(|o| self.g[*o].attached && self.g.id(*o) == Some(frag));
        let first = live.next()?;
        let rest: Vec<ObjId> = live.collect();
        if rest.is_empty() {
            return Some(first);
        }
        // duplicate ids: first in tree order
        let order = self.g.all_contents(self.res);
        order.into_iter().find(|o| *o == first || rest.contains(o))
    }

    /// EMF `XMLHandler.setValueFromId`.
    fn set_value_from_id(&mut self, o: ObjId, f: FeatureId, ids: Cow<'a, str>) {
        let mut is_first = true;
        let mut must_add = false;
        let mut must_add_or_not_opposite_many = false;
        let mut pending: Vec<(Cow<'a, str>, i32)> = Vec::new();
        let mut position: i32 = 0;
        let mut qname: Option<String> = None;
        for tok in id_tokens(&ids) {
            let mut id = tok;
            if let Some(i) = tok.find('#') {
                if i == 0 {
                    id = &tok[1..];
                } else {
                    let class = match qname.take() {
                        Some(q) => self.class_for_qname(&q).ok(),
                        None => f.target(),
                    };
                    match class.filter(|c| !c.def().is_abstract) {
                        Some(c) => {
                            let p = self.new_obj(c, false);
                            self.handle_proxy(p, tok);
                            self.set_feature_value(o, f, Some(p), -1);
                        }
                        None => self
                            .emf_error("abstract-class", format!("cannot create proxy for {tok}")),
                    }
                    position += 1;
                    continue;
                }
            } else if tok.contains(':') {
                qname = Some(tok.to_string());
                continue;
            }
            if is_first {
                match f.opposite() {
                    None => {
                        must_add = true;
                        must_add_or_not_opposite_many = true;
                    }
                    Some(opp) => {
                        must_add = opp.def().transient || f.def().many;
                        must_add_or_not_opposite_many = must_add || !opp.def().many;
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
            self.forward_many.push(ForwardRef {
                obj: o,
                feature: f,
                ids: pending,
                many: true,
                pos,
            });
        } else {
            for p in pending {
                self.forward.push(ForwardRef {
                    obj: o,
                    feature: f,
                    ids: vec![p],
                    many: false,
                    pos,
                });
            }
        }
    }

    /// `XMLHelperImpl.setValue` for references.
    fn set_feature_value(
        &mut self,
        o: ObjId,
        f: FeatureId,
        v: Option<ObjId>,
        position: i32,
    ) -> bool {
        if let (Some(t), Some(tc)) = (v, f.target())
            && !self.g[t].class.is_a(tc)
        {
            // generated setters / typed EList arrays throw (ClassCastException, ArrayStoreException)
            let msg = format!(
                "{} cannot hold a {}",
                f.name(),
                self.g[t].class.qualified_name()
            );
            self.emf_error("illegal-value", msg);
            return true;
        }
        if !f.def().many {
            self.g.set_ref(o, f, v);
            return true;
        }
        let move_kind = f
            .opposite()
            .is_some_and(|opp| !opp.def().transient && opp.def().many);
        match (position, v) {
            (-2, _) => {
                self.g.clear_refs(o, f);
                true
            }
            (_, None) => true,
            (-1, Some(v)) => {
                if o == v && self.g.get_refs(o, f).contains(&v) {
                    return true;
                }
                self.g.add_ref_unique(o, f, v, None)
            }
            (p, Some(v)) => {
                let p = p as usize;
                if o == v || move_kind {
                    match self.g.get_refs(o, f).iter().position(|x| *x == v) {
                        Some(i) => self.g.move_ref(o, f, p, i),
                        None if o == v => self.g.add_ref_unique(o, f, v, Some(p)),
                        None => false,
                    }
                } else {
                    self.g.add_ref_unique(o, f, v, Some(p))
                }
            }
        }
    }

    fn end(&mut self) {
        if let Some(mark) = self.ns_marks.pop() {
            self.ns.truncate(mark);
        }
        if let Some(Frame::AttrText { obj, feature, text }) = self.stack.pop() {
            let data = feature.data_kind().unwrap_or(DataKind::Other);
            if feature.def().many {
                match parse_value(data, &text) {
                    Some(v) => self.g.add_attr(obj, feature, v),
                    None => self.bad_value(feature, &text),
                }
            } else {
                self.set_attr_from_string(obj, feature, data, Cow::Owned(text));
            }
        }
    }

    /// EMF `handleForwardReferences(true)`.
    fn end_document(&mut self) {
        // same-document proxies of bidirectional references: connect the real object
        let proxies = std::mem::take(&mut self.same_doc_proxies);
        for p in proxies {
            let frag = self.g[p]
                .proxy
                .as_deref()
                .and_then(|u| u.split_once('#'))
                .map(|(_, f)| f.to_string());
            let Some(frag) = frag else { continue };
            let Some(t) = self.lookup(&frag) else {
                continue;
            };
            // find the holder: an object whose reference with an opposite contains the proxy
            let class = self.g[p].class;
            for r in class.features() {
                let Some(opp) = r.opposite() else { continue };
                if r.is_attribute() || !self.g.is_set(p, r) {
                    continue;
                }
                let Some(holder) = self.g.get_ref(p, r) else {
                    continue;
                };
                if opp.def().many {
                    let l = self.g.get_refs(holder, opp);
                    if let Some(pi) = l.iter().position(|x| *x == p)
                        && let Some(Slot::Refs(v)) = self.g[holder]
                            .slots
                            .iter_mut()
                            .find(|(g, _)| *g == opp)
                            .map(|(_, s)| s)
                    {
                        if v.contains(&t) {
                            v.remove(pi);
                        } else {
                            v[pi] = t;
                        }
                    }
                } else {
                    self.g.set_ref(holder, opp, Some(t));
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
                        self.emf_error(
                            "illegal-value",
                            format!("cannot set {} to {id}", r.feature.name()),
                        );
                    }
                }
                None => {
                    self.pos = r.pos;
                    self.emf_error(
                        "unresolved-idref",
                        format!("unresolved reference '{id}' in {}", r.feature.name()),
                    );
                }
            }
        }
        let fwd = std::mem::take(&mut self.forward_many);
        for r in fwd {
            debug_assert!(r.many);
            let resolved: Vec<(Option<ObjId>, i32)> =
                r.ids.iter().map(|(id, p)| (self.lookup(id), *p)).collect();
            for ((id, _), (t, _)) in r.ids.iter().zip(&resolved) {
                if t.is_none() {
                    self.pos = r.pos;
                    self.emf_error(
                        "unresolved-idref",
                        format!("unresolved reference '{id}' in {}", r.feature.name()),
                    );
                }
            }
            for (t, p) in resolved {
                let Some(t) = t else { continue };
                let p = p as usize;
                let ok = match self
                    .g
                    .get_refs(r.obj, r.feature)
                    .iter()
                    .position(|x| *x == t)
                {
                    Some(i) if r.obj == t || r.feature.opposite().is_some_and(|o| o.def().many) => {
                        self.g.move_ref(r.obj, r.feature, p, i)
                    }
                    _ => self.g.add_ref_unique(r.obj, r.feature, t, Some(p)),
                };
                if !ok {
                    self.emf_error(
                        "illegal-value",
                        format!("cannot insert into {}", r.feature.name()),
                    );
                }
            }
        }
    }

    fn final_index(&self) -> StrMap {
        let n = self.ids.len() + self.xmi_ids.len();
        let bytes = self
            .ids
            .keys()
            .chain(self.xmi_ids.keys())
            .map(|k| k.len())
            .sum();
        let mut idx = StrMap::with_capacity(n, bytes);
        let dup = self.ids.values().any(|v| matches!(v, IdObjs::Many(_)));
        if !dup {
            for (k, v) in &self.ids {
                let IdObjs::One(o) = *v else { unreachable!() };
                if self.g[o].attached && self.g.id(o) == Some(&**k) {
                    idx.insert(k, o);
                }
            }
        } else {
            for o in self.g.all_contents(self.res) {
                if let Some(id) = self.g.id(o) {
                    idx.insert_first(id, o);
                }
            }
        }
        for (k, v) in &self.xmi_ids {
            idx.insert(k, *v);
        }
        idx
    }
}

/// The non-empty tokens of an IDREF(S) value separated by ` `, `\t`, `\n`, `\r` or `\x0c`.
fn id_tokens(s: &str) -> impl Iterator<Item = &str> {
    let sep = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c');
    let b = s.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < b.len() && sep(b[i]) {
            i += 1;
        }
        if i == b.len() {
            return None;
        }
        let start = i;
        while i < b.len() && !sep(b[i]) {
            i += 1;
        }
        // separators are ASCII, so these are char boundaries
        Some(&s[start..i])
    })
}

/// Splits the attribute part of a start tag into key and value spans the way quick-xml does
/// (`key="value"` or `key='value'`, XML whitespace around `=` and between attributes).
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

/// Per class, the largest number of slots an object of the class has had after parsing (in
/// this process): the initial capacity of `Obj::slots`, which saves reallocations.
fn slot_hints() -> &'static [AtomicU8] {
    static H: OnceLock<Vec<AtomicU8>> = OnceLock::new();
    H.get_or_init(|| (0..meta::CLASSES.len()).map(|_| AtomicU8::new(0)).collect())
}

fn count_newlines(b: &[u8]) -> u32 {
    memchr::memchr_iter(b'\n', b).count() as u32
}

/// XML attribute-value normalisation (literal whitespace -> space) followed by unescaping.
/// Borrows the value unless it has to change.
fn normalize_attr(s: Cow<'_, str>) -> Cow<'_, str> {
    let b = s.as_bytes();
    // quick check without early exit (vectorises): any control character or `&`?
    if !b
        .iter()
        .fold(false, |acc, &c| acc | (c < 0x20) | (c == b'&'))
    {
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

/// Follows a path fragment (`/`, `/1`, `//@feature.0/@other`) from the given roots.
pub(crate) fn navigate_path(g: &Graph, roots: &[ObjId], frag: &str) -> Option<ObjId> {
    let mut segs = frag.split('/').skip(1);
    let root_seg = segs.next().unwrap_or("");
    let ri: usize = if root_seg.is_empty() {
        0
    } else {
        root_seg.parse().ok()?
    };
    let mut cur = *roots.get(ri)?;
    for seg in segs {
        let seg = seg.strip_prefix('@')?;
        let (name, idx) = match seg.rsplit_once('.') {
            Some((n, i)) if i.bytes().all(|c| c.is_ascii_digit()) && !i.is_empty() => {
                (n, Some(i.parse::<usize>().ok()?))
            }
            _ => (seg, None),
        };
        let f = g[cur].class.feature(name)?;
        cur = match idx {
            Some(i) if f.def().many => *g.get_refs(cur, f).get(i)?,
            Some(_) => return None,
            None if f.def().many => *g.get_refs(cur, f).first()?,
            None => g.get_ref(cur, f)?,
        };
    }
    Some(cur)
}

#[cfg(test)]
mod tests {
    use super::*;

    type Attrs = Vec<Result<(Vec<u8>, Vec<u8>), ()>>;

    fn check_text(text: &str, fast: &mut usize, total: &mut usize) {
        let mut reader = quick_xml::Reader::from_str(text);
        reader.config_mut().trim_text(false);
        reader.config_mut().expand_empty_elements = false;
        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    *total += 1;
                    let raw = e.attributes_raw();
                    let mut spans = Vec::new();
                    let want: Attrs = e
                        .attributes()
                        .with_checks(false)
                        .map(|a| {
                            a.map(|a| (a.key.as_ref().to_vec(), a.value.to_vec()))
                                .map_err(|_| ())
                        })
                        .collect();
                    if scan_attributes(raw, &mut spans) {
                        *fast += 1;
                        let got: Attrs = spans
                            .iter()
                            .map(|(k, v)| Ok((raw[k.clone()].to_vec(), raw[v.clone()].to_vec())))
                            .collect();
                        assert_eq!(got, want, "{:?}", String::from_utf8_lossy(raw));
                    } else {
                        assert!(
                            want.iter().any(|a| a.is_err()),
                            "fallback for valid attributes {:?}",
                            String::from_utf8_lossy(raw)
                        );
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }
    }

    #[test]
    fn attribute_scanner_matches_quick_xml() {
        let (mut fast, mut total) = (0, 0);
        for (_, t) in crate::load::BUNDLED {
            check_text(t, &mut fast, &mut total);
        }
        let synthetic = r#"<a x="1" y = '2'	z
            ="3&amp;4" w="a'b" v='a"b' u=""/><b x="1"y="2"/><c x/><d x=1/><e ="1"/><f x="1/>"#;
        check_text(synthetic, &mut fast, &mut total);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        for base in ["../../corpus", "tests/xmi-cases"] {
            for d in std::fs::read_dir(root.join(base)).unwrap().flatten() {
                let Ok(files) = std::fs::read_dir(d.path()) else {
                    continue;
                };
                for f in files.flatten() {
                    if let Ok(t) = std::fs::read_to_string(f.path())
                        && t.starts_with("<?xml")
                    {
                        check_text(&t, &mut fast, &mut total);
                    }
                }
            }
        }
        assert!(total > 1000 && fast + 10 > total, "{fast} of {total}");
    }
}

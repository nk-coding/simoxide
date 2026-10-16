//! XMI writer reproducing EMF's `XMISaveImpl` / `XMLSaveImpl` byte for byte (no extended
//! metadata, no XML map): feature kinds, attribute vs element serialisation, `xsi:type` rules,
//! `href`s with deresolved URIs, IDREFs, namespace prefixes, the `XMLString` printer with its
//! line-width wrapping and the late insertion of namespace declarations, and `Escape`.
//!
//! Ported from EMF (`org.eclipse.emf.ecore.xmi.impl`, EPL-2.0, see `LICENSES`).

use crate::datatypes::value_to_string;
use crate::fragment::uri_fragment;
use crate::graph::{Graph, ObjId, ResourceKind};
use crate::uri::Uri;
use qvto_model::{
    ClassId, Classifier, Datum, FeatureId, FeatureKind, ModelStore, PackageId, ResourceId,
};
use std::collections::{BTreeMap, HashMap};

/// Save options (the subset of `XMLResource.OPTION_*` that the engine and the tests use).
#[derive(Clone, Debug, Default)]
pub struct SaveOptions {
    /// `OPTION_ENCODING`; default: the resource's encoding.
    pub encoding: Option<String>,
    /// `OPTION_LINE_WIDTH`.
    pub line_width: Option<usize>,
    /// `OPTION_USE_ENCODED_ATTRIBUTE_STYLE`.
    pub encoded_attribute_style: bool,
    /// `OPTION_URI_HANDLER = URIHandlerImpl.PlatformSchemeAware`.
    pub platform_scheme_aware: bool,
    /// `OPTION_SCHEMA_LOCATION`.
    pub schema_location: bool,
    /// `OPTION_KEEP_DEFAULT_CONTENT`.
    pub keep_default_content: bool,
}

impl SaveOptions {
    /// The default save options of a resource (`EcoreResourceFactoryImpl` for `.ecore`, plus
    /// `OPTION_SCHEMA_LOCATION` when the loaded document had a schema location).
    pub fn for_resource(r: &crate::graph::Resource) -> SaveOptions {
        let mut o = Self::defaults_for(r.kind);
        o.schema_location = r.save_schema_location;
        o
    }

    /// The default save options of a resource kind.
    pub fn defaults_for(kind: ResourceKind) -> SaveOptions {
        match kind {
            ResourceKind::Xmi => SaveOptions::default(),
            ResourceKind::Ecore => SaveOptions {
                line_width: Some(80),
                encoded_attribute_style: true,
                platform_scheme_aware: true,
                ..SaveOptions::default()
            },
        }
    }
}

const XSI_NS: &str = "http://www.w3.org/2001/XMLSchema-instance";

/// The `XMLString` printer.
struct Doc {
    out: String,
    line_width: usize,
    width: usize,
    depth: usize,
    names: Vec<String>,
    last_is_start: bool,
    mark: Option<(usize, usize)>,
}

fn jlen(s: &str) -> usize {
    if s.is_ascii() {
        s.len()
    } else {
        s.encode_utf16().count()
    }
}

impl Doc {
    fn add(&mut self, s: &str) {
        if self.line_width != usize::MAX {
            self.width += jlen(s);
        }
        self.out.push_str(s);
    }
    fn add_line(&mut self) {
        self.out.push('\n');
        self.width = 0;
    }
    fn indent(&self, n: usize) -> String {
        "  ".repeat(n)
    }
    fn close_start(&mut self) {
        self.add(">");
        self.add_line();
        self.last_is_start = false;
    }
    fn start_element(&mut self, name: &str) {
        if self.last_is_start {
            self.close_start();
        }
        self.names.push(name.to_string());
        self.depth += 1;
        let ind = self.indent(self.depth - 1);
        self.add(&ind);
        self.add("<");
        self.add(name);
        self.last_is_start = true;
    }
    fn mark(&mut self) {
        self.mark = Some((self.out.len(), self.width));
    }
    fn attr_sep(&mut self) {
        if self.width > self.line_width {
            self.add_line();
            let ind = self.indent(self.depth + 1);
            self.add(&ind);
        } else {
            self.add(" ");
        }
    }
    fn add_attribute(&mut self, name: &str, value: &str) {
        self.attr_sep();
        self.add(name);
        self.add("=\"");
        self.add(value);
        self.add("\"");
    }
    fn end_empty(&mut self) {
        self.names.pop();
        self.depth -= 1;
        self.add("/>");
        self.add_line();
        self.last_is_start = false;
    }
    fn end_element(&mut self) {
        if self.last_is_start {
            self.end_empty();
        } else {
            let name = self.names.pop().unwrap();
            self.depth -= 1;
            let ind = self.indent(self.depth);
            self.add(&ind);
            self.add("</");
            self.add(&name);
            self.add(">");
            self.add_line();
        }
    }
    fn data_value_element(&mut self, name: &str, content: &str) {
        if self.last_is_start {
            self.close_start();
        }
        self.depth += 1;
        let ind = self.indent(self.depth - 1);
        self.add(&ind);
        self.add("<");
        self.add(name);
        self.add(">");
        self.add(content);
        self.add("</");
        self.depth -= 1;
        self.add(name);
        self.add(">");
        self.add_line();
        self.last_is_start = false;
    }
    fn nil_element(&mut self, name: &str) {
        if self.last_is_start {
            self.close_start();
        }
        self.depth += 1;
        let ind = self.indent(self.depth - 1);
        self.add(&ind);
        self.add("<");
        self.add(name);
        self.attr_sep();
        self.add("xsi:nil=\"true\"/>");
        self.depth -= 1;
        self.add_line();
        self.last_is_start = false;
    }
}

/// `XMLSaveImpl.Escape`.
struct Escape {
    limit: u32,
}

impl Escape {
    fn convert(&self, s: &str, text: bool) -> Result<String, String> {
        let mut out = String::with_capacity(s.len());
        let chars: Vec<char> = s.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '"' => out.push_str("&quot;"),
                '\n' if text => out.push('\n'),
                '\n' => out.push_str("&#xA;"),
                '\r' => out.push_str("&#xD;"),
                '\t' if !text => out.push_str("&#x9;"),
                '>' if text && i >= 2 && chars[i - 1] == ']' && chars[i - 2] == ']' => {
                    out.push_str("&gt;")
                }
                c if (c as u32) < 0x20 && c != '\t' => {
                    return Err(format!(
                        "An invalid XML character (Unicode: 0x{:x}) was found in the element content:{s}",
                        c as u32
                    ));
                }
                c if (c as u32) > self.limit => out.push_str(&format!("&#x{:x};", c as u32)),
                c => out.push(c),
            }
        }
        Ok(out)
    }
}

struct Saver<'a> {
    g: &'a Graph,
    res: ResourceId,
    res_uri: Uri,
    opts: SaveOptions,
    doc: Doc,
    esc: Escape,
    declare_xsi: bool,
    xmi_uri: String,
    /// `prefixesToURIs` in insertion order, `packages` (package -> prefix).
    prefixes: Vec<(String, String)>,
    packages: HashMap<PackageId, String>,
    root: Option<ObjId>,
    dangling: Option<String>,
}

/// Saves resource `r` as XMI text. Fails like EMF on dangling references (objects in no
/// resource) and on invalid XML characters.
pub fn save(g: &Graph, r: ResourceId, opts: &SaveOptions) -> Result<String, String> {
    let res = g.resource(r);
    let encoding = opts
        .encoding
        .clone()
        .unwrap_or_else(|| res.encoding.clone());
    let limit = match encoding.to_ascii_uppercase().as_str() {
        "ASCII" | "US-ASCII" => 127,
        "ISO-8859-1" => 255,
        _ => 0x10FFFF,
    };
    let xmi_uri = match res.xmi_version.as_str() {
        "2.0" => "http://www.omg.org/XMI".to_string(),
        "2.1" => "http://schema.omg.org/spec/XMI/2.1".to_string(),
        v => format!("http://www.omg.org/spec/XMI/{v}"),
    };
    let mut s = Saver {
        g,
        res: r,
        res_uri: res.parsed_uri().trim_fragment(),
        opts: opts.clone(),
        doc: Doc {
            out: String::new(),
            line_width: opts.line_width.unwrap_or(usize::MAX),
            width: 0,
            depth: 0,
            names: Vec::new(),
            last_is_start: false,
            mark: None,
        },
        esc: Escape { limit },
        declare_xsi: false,
        prefixes: vec![("xmi".to_string(), xmi_uri.clone())],
        xmi_uri,
        packages: HashMap::new(),
        root: None,
        dangling: None,
    };
    s.doc.add(&format!(
        "<?xml version=\"{}\" encoding=\"{encoding}\"?>",
        res.xml_version
    ));
    s.doc.add_line();
    let roots = res.roots.clone();
    if roots.len() == 1 {
        let top = roots[0];
        let name = s.class_qname(g.class_of(top));
        s.doc.start_element(&name);
        s.doc.mark();
        s.root = Some(top);
        s.save_element_id(top)?;
    } else {
        s.doc.start_element("xmi:XMI");
        s.doc.mark();
        for top in roots {
            let name = s.class_qname(g.class_of(top));
            s.doc.start_element(&name);
            s.root = Some(top);
            s.save_element_id(top)?;
        }
        s.doc.end_element();
    }
    if let Some(d) = s.dangling {
        return Err(d);
    }
    // namespace declarations, inserted at the mark
    let (pos, marked_width) = s.doc.mark.expect("mark");
    let rest = s.doc.out.split_off(pos);
    s.doc.width = marked_width;
    s.doc.depth += 1;
    s.namespace_declarations();
    if s.doc.width > s.doc.line_width && rest.starts_with(' ') {
        s.doc.add_line();
        s.doc.add("   ");
    }
    s.doc.out.push_str(&rest);
    Ok(s.doc.out)
}

impl Saver<'_> {
    fn prefix_of(&mut self, p: PackageId) -> String {
        if let Some(x) = self.packages.get(&p) {
            return x.clone();
        }
        let meta = self.g.meta();
        let ns_uri = meta.package(p).ns_uri.as_deref().map(|s| s.to_string());
        let found = self
            .prefixes
            .iter()
            .find(|(_, u)| Some(u.as_str()) == ns_uri.as_deref())
            .map(|(k, _)| k.clone());
        let prefix = match found {
            Some(x) => x,
            None => {
                let mut prefix = if ns_uri.is_some() {
                    meta.package(p)
                        .ns_prefix
                        .as_deref()
                        .unwrap_or("")
                        .to_string()
                } else {
                    String::new()
                };
                let conflict = self
                    .prefixes
                    .iter()
                    .any(|(k, u)| *k == prefix && Some(u.as_str()) != ns_uri.as_deref());
                if conflict {
                    let mut i = 1;
                    while self
                        .prefixes
                        .iter()
                        .any(|(k, _)| *k == format!("{prefix}_{i}"))
                    {
                        i += 1;
                    }
                    prefix = format!("{prefix}_{i}");
                }
                self.prefixes
                    .push((prefix.clone(), ns_uri.unwrap_or_default()));
                prefix
            }
        };
        self.packages.insert(p, prefix.clone());
        prefix
    }

    fn qname(&mut self, p: PackageId, name: &str) -> String {
        let prefix = self.prefix_of(p);
        if prefix.is_empty() {
            name.to_string()
        } else if name.is_empty() {
            prefix
        } else {
            format!("{prefix}:{name}")
        }
    }

    fn class_qname(&mut self, c: ClassId) -> String {
        let ci = self.g.meta().class(c);
        let (p, n) = (ci.package, ci.name.to_string());
        self.qname(p, &n)
    }

    fn feature_name(&self, f: FeatureId) -> String {
        self.g.meta().feature(f).name.to_string()
    }

    /// `Lookup.featureKind` without extended metadata and XML map.
    fn kind(&self, f: FeatureId) -> u8 {
        let meta = self.g.meta();
        let fi = meta.feature(f);
        if fi.transient {
            return 0;
        }
        let many = fi.is_many();
        let uns = fi.unsettable;
        if fi.kind == FeatureKind::Reference {
            if fi.containment {
                return match (many, uns) {
                    (true, true) => 11,
                    (true, false) => 7,
                    (false, true) => 10,
                    (false, false) => 6,
                };
            }
            if meta.is_container(f) {
                return 0;
            }
            return match (many, uns) {
                (true, true) => 13,
                (true, false) => 9,
                (false, true) => 12,
                (false, false) => 8,
            };
        }
        let serializable = match fi.ty {
            Classifier::DataType(d) => meta.data_type(d).serializable,
            Classifier::Enum(e) => meta.enumeration(e).serializable,
            Classifier::Class(_) => true,
        };
        if !serializable {
            return 0;
        }
        if many {
            5
        } else if uns {
            4
        } else {
            1
        }
    }

    fn should_save(&self, o: ObjId, f: FeatureId) -> bool {
        self.g.is_set(o, f)
            || (self.opts.keep_default_content && self.g.meta().default_literal(f).is_some())
    }

    fn save_element_id(&mut self, o: ObjId) -> Result<(), String> {
        if let Some(id) = self.g.resource(self.res).xmi_id(o) {
            let id = id.to_string();
            self.doc.add_attribute("xmi:id", &id);
        }
        self.save_features(o)
    }

    fn value_string(&self, v: &Datum<'_, ObjId>, text: bool) -> Result<Option<String>, String> {
        match value_to_string(self.g.meta(), &v.to_owned()) {
            None => Ok(None),
            Some(s) => self.esc.convert(&s, text).map(Some),
        }
    }

    fn ref_values(&self, o: ObjId, f: FeatureId) -> Vec<ObjId> {
        if self.g.meta().feature(f).is_many() {
            (0..self.g.len(o, f))
                .filter_map(|i| self.g.get_at(o, f, i).as_obj())
                .collect()
        } else {
            self.g.get(o, f).as_obj().into_iter().collect()
        }
    }

    fn save_features(&mut self, o: ObjId) -> Result<(), String> {
        let g = self.g;
        let class = g.class_of(o);
        let features: Vec<FeatureId> = g.meta().class(class).all_features.to_vec();
        let mut elements: Vec<(FeatureId, u8)> = Vec::new();
        for f in features {
            let kind = self.kind(f);
            if kind == 0 || !self.should_save(o, f) {
                continue;
            }
            let to_element = match kind {
                1 => {
                    self.save_data_single(o, f)?;
                    false
                }
                4 => {
                    if g.get(o, f).is_null() {
                        true
                    } else {
                        self.save_data_single(o, f)?;
                        false
                    }
                }
                8 | 12 => {
                    if kind == 12 && g.get(o, f).is_null() {
                        true
                    } else if self.opts.encoded_attribute_style {
                        self.save_eobject_attr(o, f);
                        false
                    } else {
                        match self.same_doc(o, f) {
                            1 => {
                                self.save_idref_attr(o, f);
                                false
                            }
                            2 => true,
                            _ => false,
                        }
                    }
                }
                9 | 13 => {
                    if kind == 13 && g.len(o, f) == 0 {
                        let n = self.feature_name(f);
                        self.doc.add_attribute(&n, "");
                        false
                    } else if self.opts.encoded_attribute_style {
                        self.save_eobject_attr(o, f);
                        false
                    } else {
                        match self.same_doc(o, f) {
                            1 => {
                                self.save_idref_attr(o, f);
                                false
                            }
                            2 => true,
                            _ => false,
                        }
                    }
                }
                5 | 11 => {
                    if g.len(o, f) == 0 {
                        let n = self.feature_name(f);
                        self.doc.add_attribute(&n, "");
                        false
                    } else {
                        true
                    }
                }
                6 | 7 | 10 => true,
                _ => false,
            };
            if to_element {
                elements.push((f, kind));
            }
        }
        if elements.is_empty() {
            self.doc.end_empty();
            return Ok(());
        }
        for (f, kind) in elements {
            let name = self.feature_name(f);
            match kind {
                4 => self.save_nil(&name),
                5 => {
                    for i in 0..g.len(o, f) {
                        let v = g.get_at(o, f, i);
                        if v.is_null() {
                            self.doc.start_element(&name);
                            self.doc.add_attribute("xsi:nil", "true");
                            self.doc.end_empty();
                            self.declare_xsi = true;
                        } else {
                            let s = self.value_string(&v, true)?.unwrap_or_default();
                            self.doc.data_value_element(&name, &s);
                        }
                    }
                }
                10 | 6 => match g.get(o, f).as_obj() {
                    None if kind == 10 => self.save_nil(&name),
                    None => {}
                    Some(v) => self.save_contained(v, f)?,
                },
                7 | 11 => {
                    for v in self.ref_values(o, f) {
                        self.save_contained(v, f)?;
                    }
                }
                12 | 8 => match g.get(o, f).as_obj() {
                    None if kind == 12 => self.save_nil(&name),
                    None => {}
                    Some(v) => self.save_href(v, f),
                },
                9 | 13 => {
                    for v in self.ref_values(o, f) {
                        self.save_href(v, f);
                    }
                }
                _ => {}
            }
        }
        self.doc.end_element();
        Ok(())
    }

    fn save_nil(&mut self, name: &str) {
        self.declare_xsi = true;
        self.doc.nil_element(name);
    }

    fn save_data_single(&mut self, o: ObjId, f: FeatureId) -> Result<(), String> {
        let v = self.g.get(o, f);
        if let Some(s) = self.value_string(&v, false)? {
            let n = self.feature_name(f);
            self.doc.add_attribute(&n, &s);
        }
        Ok(())
    }

    fn save_contained(&mut self, v: ObjId, f: FeatureId) -> Result<(), String> {
        if self.g.direct_resource(v).is_some() || self.g.is_proxy(v) {
            self.save_href(v, f);
            return Ok(());
        }
        let class = self.g.class_of(v);
        let expected = self.g.meta().feature(f).class_type();
        let name = self.feature_name(f);
        self.doc.start_element(&name);
        if Some(class) != expected {
            self.declare_xsi = true;
            let q = self.class_qname(class);
            self.doc.add_attribute("xsi:type", &q);
        }
        self.save_element_id(v)
    }

    /// `shouldSaveType` for references written as URIs.
    fn should_save_type(&self, value: ObjId, f: FeatureId) -> bool {
        let fi = self.g.meta().feature(f);
        let class = self.g.class_of(value);
        let Some(expected) = fi.class_type() else {
            return false;
        };
        let ei = self.g.meta().class(expected);
        class != expected && (ei.is_abstract || fi.type_parameter)
    }

    fn save_href(&mut self, v: ObjId, f: FeatureId) {
        let Some(href) = self.href(v) else { return };
        let name = self.feature_name(f);
        self.doc.start_element(&name);
        if self.should_save_type(v, f) {
            self.declare_xsi = true;
            let q = self.class_qname(self.g.class_of(v));
            self.doc.add_attribute("xsi:type", &q);
        }
        self.doc.add_attribute("href", &href);
        self.doc.end_empty();
    }

    fn save_eobject_attr(&mut self, o: ObjId, f: FeatureId) {
        let values = self.ref_values(o, f);
        if values.is_empty() {
            return;
        }
        let mut parts = Vec::new();
        let mut failure = false;
        for v in values {
            match self.href(v) {
                None => failure = true,
                Some(id) => {
                    let mut s = String::new();
                    if !id.starts_with('#') && self.should_save_type(v, f) {
                        s.push_str(&self.class_qname(self.g.class_of(v)));
                        s.push(' ');
                    }
                    s.push_str(&id);
                    parts.push(s);
                }
            }
        }
        let mut s = parts.join(" ");
        if failure {
            s = s.trim().to_string();
            if s.is_empty() {
                return;
            }
        }
        let n = self.feature_name(f);
        self.doc.add_attribute(&n, &s);
    }

    fn save_idref_attr(&mut self, o: ObjId, f: FeatureId) {
        let values = self.ref_values(o, f);
        if values.is_empty() {
            return;
        }
        let mut ids = Vec::new();
        let mut failure = false;
        for v in values {
            match self.idref(v) {
                Some(id) => ids.push(id),
                None => failure = true,
            }
        }
        let mut s = ids.join(" ");
        if failure {
            s = s.trim().to_string();
            if s.is_empty() {
                return;
            }
        }
        let n = self.feature_name(f);
        self.doc.add_attribute(&n, &s);
    }

    /// `sameDocSingle` / `sameDocMany`: 0 nothing, 1 same document, 2 cross document.
    fn same_doc(&self, o: ObjId, f: FeatureId) -> u8 {
        let values = self.ref_values(o, f);
        if values.is_empty() {
            return 0;
        }
        for v in values {
            if self.g.is_proxy(v) {
                return 2;
            }
            match self.g.resource_of(v) {
                Some(r) if r != self.res => return 2,
                _ => {}
            }
        }
        1
    }

    fn dangling(&mut self, v: ObjId) {
        if self.dangling.is_none() {
            self.dangling = Some(format!(
                "DanglingHREFException: The object '{}' is not contained in a resource.",
                self.g.meta().qualified_name(self.g.class_of(v))
            ));
        }
    }

    /// `getIDREF`: the fragment within this resource.
    fn idref(&mut self, v: ObjId) -> Option<String> {
        let frag = uri_fragment(self.g, self.res, v);
        if frag == "/-1" && self.g.resource_of(v) != Some(self.res) {
            self.dangling(v);
            return None;
        }
        Some(frag)
    }

    /// `getHREF`: the deresolved URI of an object (or a proxy's URI).
    fn href(&mut self, v: ObjId) -> Option<String> {
        let uri = if let Some(p) = self.g.proxy_uri(v) {
            Uri::parse(p)
        } else {
            match self.g.resource_of(v) {
                None => {
                    self.dangling(v);
                    return None;
                }
                Some(r) => {
                    let frag = uri_fragment(self.g, r, v);
                    if frag == "/-1" {
                        self.dangling(v);
                        return None;
                    }
                    self.g
                        .resource(r)
                        .parsed_uri()
                        .trim_fragment()
                        .append_fragment(&frag)
                }
            }
        };
        Some(self.deresolve(&uri).to_string())
    }

    fn deresolve(&self, uri: &Uri) -> Uri {
        let base = &self.res_uri;
        if self.opts.platform_scheme_aware
            && uri.is_platform()
            && !(!base.segments().is_empty() && uri.segments().first() == base.segments().first())
        {
            return uri.clone();
        }
        if base.is_relative() || !base.is_hierarchical() || uri.is_relative() {
            return uri.clone();
        }
        let d = uri.deresolve(base);
        if d.has_relative_path() {
            d
        } else {
            uri.clone()
        }
    }

    fn namespace_declarations(&mut self) {
        // packages(): sorted by prefix
        let mut by_prefix: BTreeMap<String, PackageId> = BTreeMap::new();
        let pkgs: Vec<(PackageId, String)> =
            self.packages.iter().map(|(p, x)| (*p, x.clone())).collect();
        for (p, prefix) in pkgs {
            by_prefix.entry(prefix).or_insert(p);
        }
        let mut schema = String::new();
        if self.opts.schema_location {
            for &p in by_prefix.values() {
                let Some(&po) = self.g.links().package_obj.get(&p) else {
                    continue;
                };
                let Some(r) = self.g.resource_of(po) else {
                    continue;
                };
                let ns = self
                    .g
                    .meta()
                    .package(p)
                    .ns_uri
                    .as_deref()
                    .unwrap_or("")
                    .to_string();
                let ruri = self.g.resource(r).uri.clone();
                if ruri != ns {
                    self.declare_xsi = true;
                    let mut loc = self.href(po).unwrap_or_default();
                    if let Some(l) = loc.strip_suffix("#/") {
                        loc = l.to_string();
                    }
                    if !schema.is_empty() {
                        schema.push(' ');
                    }
                    schema.push_str(&ns);
                    schema.push(' ');
                    schema.push_str(&loc);
                }
            }
        }
        let version = self.g.resource(self.res).xmi_version.clone();
        let xmi_uri = self.xmi_uri.clone();
        self.doc.add_attribute("xmi:version", &version);
        self.doc.add_attribute("xmlns:xmi", &xmi_uri);
        if self.declare_xsi {
            self.doc.add_attribute("xmlns:xsi", XSI_NS);
        }
        for (prefix, &p) in by_prefix.iter() {
            let Some(ns) = self.g.meta().package(p).ns_uri.as_deref() else {
                continue;
            };
            if ns == self.xmi_uri {
                continue;
            }
            let mut all = vec![prefix.clone()];
            for (k, u) in &self.prefixes {
                if u == ns && !all.contains(k) {
                    all.push(k.clone());
                }
            }
            for x in all {
                if x.is_empty() {
                    self.doc.add_attribute("xmlns", ns);
                } else if !(self.declare_xsi && x == "xsi") {
                    self.doc.add_attribute(&format!("xmlns:{x}"), ns);
                }
            }
        }
        if !schema.is_empty() {
            self.doc.add_attribute("xsi:schemaLocation", &schema);
        }
    }
}

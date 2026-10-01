//! Resource set: loads XMI files, demand-loads referenced resources and resolves cross-resource
//! references (EMF `ResourceSet` + `EcoreUtil.resolveAll`).
//!
//! URIs are normalised to keys: `file:/abs/path` for files and `pathmap://PCM_MODELS/<name>` /
//! `pathmap://METRIC_SPEC_MODELS/<name>` for the bundled default models (also reached through
//! `platform:/plugin/...` URIs, which EMF normalises to the same resource).

use crate::diag::{Diagnostics, Level};
use crate::fxhash::{FxHashMap, StrMap};
use crate::raw::{Graph, Obj, ObjId, ResId, Resource, Slot};
use crate::xmi::{ParseOptions, Parser, navigate_path};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// File extensions of the PCM model files a model directory is made of.
pub const MODEL_EXTENSIONS: &[&str] = &[
    "repository",
    "system",
    "resourceenvironment",
    "allocation",
    "usagemodel",
    "resourcetype",
    "monitorrepository",
    "measuringpoint",
];

/// Default models bundled with the crate (extracted from the 5.2.2 product jars).
pub const BUNDLED: &[(&str, &str)] = &[
    (
        "pathmap://PCM_MODELS/Palladio.resourcetype",
        include_str!("../models/Palladio.resourcetype"),
    ),
    (
        "pathmap://PCM_MODELS/PrimitiveTypes.repository",
        include_str!("../models/PrimitiveTypes.repository"),
    ),
    (
        "pathmap://PCM_MODELS/FailureTypes.repository",
        include_str!("../models/FailureTypes.repository"),
    ),
    (
        "pathmap://PCM_MODELS/Glassfish.repository",
        include_str!("../models/Glassfish.repository"),
    ),
    (
        "pathmap://PCM_MODELS/default_event_middleware.repository",
        include_str!("../models/default_event_middleware.repository"),
    ),
    (
        "pathmap://METRIC_SPEC_MODELS/commonMetrics.metricspec",
        include_str!("../models/commonMetrics.metricspec"),
    ),
    // the jar contains the file twice; EMF treats the two URIs as different resources
    (
        "pathmap://METRIC_SPEC_MODELS/models/commonMetrics.metricspec",
        include_str!("../models/commonMetrics.metricspec"),
    ),
];

#[derive(Clone, Debug)]
pub struct LoadOptions {
    /// Demand-load resources referenced by `href`s (like EMF proxy resolution). Default `true`.
    pub follow_references: bool,
    /// If an `href` cannot be resolved the EMF way (missing file, broken absolute path), try the
    /// same file name next to the referencing file and then a unique ID over all loaded resources.
    /// Each such resolution is reported as a warning. Default `true`; `false` = strict EMF.
    pub tolerant_references: bool,
    /// Accept namespace URIs of older PCM versions (reported as errors, since EMF cannot load them).
    pub tolerant_namespaces: bool,
}

impl Default for LoadOptions {
    fn default() -> Self {
        LoadOptions {
            follow_references: true,
            tolerant_references: true,
            tolerant_namespaces: true,
        }
    }
}

impl LoadOptions {
    /// Strict EMF behaviour (used for the golden comparison with the Java oracle).
    pub fn strict() -> Self {
        LoadOptions {
            follow_references: true,
            tolerant_references: false,
            tolerant_namespaces: false,
        }
    }
}

/// Loads resources into one object graph.
pub struct Loader {
    pub(crate) g: Graph,
    pub(crate) diags: Diagnostics,
    opts: LoadOptions,
    by_key: FxHashMap<String, ResId>,
    id_index: Vec<IdIndex>,
    base_dir: Option<PathBuf>,
    /// In-memory files by normalised absolute path (see [`Loader::add_file`]).
    memory: FxHashMap<PathBuf, Arc<String>>,
    read_files: bool,
    cache: Option<Arc<ParseCache>>,
    /// Proxies left by the last `resolve_all`: (holder, cross reference, proxy).
    unresolved: Vec<(ObjId, crate::meta::FeatureId, ObjId)>,
}

/// Directory that relative file names given to [`crate::load_memory`] are placed in. It only
/// names the in-memory files; nothing is read from it.
pub const MEMORY_DIR: &str = "/pcm-memory";

/// Lexically normalised absolute path.
pub fn normalize_path(p: &Path) -> PathBuf {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    };
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c.as_os_str()),
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Normalises an absolute URI (without fragment) to a resource key.
pub fn normalize_uri(uri: &str) -> String {
    const PCM_PLUGIN: &str = "platform:/plugin/org.palladiosimulator.pcm.resources/defaultModels/";
    const METRIC_PLUGIN: &str = "platform:/plugin/org.palladiosimulator.metricspec.resources/";
    if let Some(r) = uri.strip_prefix(PCM_PLUGIN) {
        return format!("pathmap://PCM_MODELS/{r}");
    }
    if let Some(r) = uri.strip_prefix(METRIC_PLUGIN) {
        return format!("pathmap://METRIC_SPEC_MODELS/{r}");
    }
    if let Some(r) = uri.strip_prefix("file:") {
        let r = r.trim_start_matches('/');
        // EMF keeps absolute URIs as written (no `..` removal): `/a/b/../c/x` and `/a/c/x` are
        // two different resources, loaded twice. Reproduced on purpose.
        return format!("file:/{}", percent_decode(r));
    }
    uri.to_string()
}

/// `a/b/c`: non-empty segments other than `.` and `..` (what `normalize_path` keeps as is).
fn is_plain_relative(p: &str) -> bool {
    p.split('/').all(|s| !s.is_empty() && s != "." && s != "..")
}

/// `/a/b/c` with plain segments.
fn is_plain_absolute(p: &str) -> bool {
    p.strip_prefix('/').is_some_and(is_plain_relative)
}

/// Resolves `href` (without fragment) against the key of the referencing resource.
pub fn resolve_uri(base_key: &str, href: &str) -> String {
    let has_scheme = href.split_once(':').is_some_and(|(s, _)| {
        s.len() > 1
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'.' || c == b'-')
    });
    if has_scheme {
        return normalize_uri(href);
    }
    if let Some(p) = base_key.strip_prefix("file:")
        && !href.contains('%')
        && is_plain_relative(href)
        && is_plain_absolute(p)
        && let Some((dir, _)) = p.rsplit_once('/')
    {
        // fast path: nothing to decode or normalise
        return format!("file:{dir}/{href}");
    }
    resolve_uri_general(base_key, href)
}

/// [`resolve_uri`] without the fast path for plain relative file references.
fn resolve_uri_general(base_key: &str, href: &str) -> String {
    let href = percent_decode(href);
    if let Some(p) = base_key.strip_prefix("file:") {
        let dir = Path::new(p).parent().unwrap_or(Path::new("/"));
        let joined = if href.starts_with('/') {
            PathBuf::from(&href)
        } else {
            dir.join(&href)
        };
        return format!("file:{}", normalize_path(&joined).display());
    }
    // hierarchical URI (pathmap://X/a/b): replace the last segment
    match base_key.rsplit_once('/') {
        Some((dir, _)) => normalize_uri(&format!("{dir}/{href}")),
        None => href,
    }
}

/// ID index of one resource (first object in tree order per ID), possibly shared with the
/// bundled-model cache, whose object ids are shifted by `offset`.
#[derive(Default)]
struct IdIndex {
    map: Arc<StrMap>,
    offset: u32,
}

impl IdIndex {
    fn get(&self, id: &str) -> Option<ObjId> {
        self.map.get(id).map(|o| ObjId(o.0 + self.offset))
    }
}

/// A parsed resource, detached from its graph (object ids from 0): copied into a graph instead
/// of parsing the text again. Parsing a resource only creates and links objects of that resource
/// (contiguous in `objs`), and depends only on its text, URI key, display name and the
/// `tolerant_namespaces` option, so the copy with shifted ids is exactly what parsing would give.
struct Template {
    objs: Vec<Obj>,
    roots: Vec<ObjId>,
    errors: Vec<String>,
    failed: bool,
    diags: Vec<crate::diag::Diagnostic>,
    id_index: Arc<StrMap>,
}

/// Cache of parsed resources that loaders can share (also between threads), for loading many
/// models that have files in common (e.g. candidate architectures that differ only in their
/// allocation or system): a resource with the same URI, display name and text as a cached one
/// is copied instead of parsed. The model is exactly the same as without the cache.
///
/// A resource is cached when it is loaded for the second time (copying a parsed resource costs
/// time, which pays off only when it is loaded again). Keeps at most `capacity` resources (seen
/// or cached), evicting the least recently used.
pub struct ParseCache {
    capacity: usize,
    inner: std::sync::Mutex<CacheState>,
}

#[derive(Default)]
struct CacheState {
    clock: u64,
    entries: Vec<CacheEntry>,
    hits: u64,
    misses: u64,
}

struct CacheEntry {
    uri: Box<str>,
    name: Box<str>,
    tolerant_namespaces: bool,
    /// Length and [`fingerprint`] of the text.
    len: usize,
    fp: u64,
    used: u64,
    /// The text and the parsed resource; `None`: seen once, not cached yet.
    cached: Option<(Arc<str>, Arc<Template>)>,
}

enum Lookup {
    Hit(Arc<Template>),
    /// Loaded before with the same text: cache it now.
    Again,
    New,
}

impl ParseCache {
    pub fn new(capacity: usize) -> Self {
        ParseCache {
            capacity,
            inner: Default::default(),
        }
    }

    /// Number of cached resources.
    pub fn len(&self) -> usize {
        self.state().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// (hits, misses) so far.
    pub fn stats(&self) -> (u64, u64) {
        let s = self.state();
        (s.hits, s.misses)
    }

    pub fn clear(&self) {
        self.state().entries.clear();
    }

    fn state(&self) -> std::sync::MutexGuard<'_, CacheState> {
        // a panic while holding the lock cannot leave the state inconsistent
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Looks a resource up; a new one is remembered as seen.
    fn get(&self, uri: &str, name: &str, tolerant: bool, text: &str) -> Lookup {
        if self.capacity == 0 {
            return Lookup::New;
        }
        let fp = fingerprint(text);
        let found = {
            let mut s = self.state();
            s.clock += 1;
            let now = s.clock;
            let e = s.entries.iter_mut().find(|e| {
                e.fp == fp
                    && e.len == text.len()
                    && e.tolerant_namespaces == tolerant
                    && *e.uri == *uri
                    && *e.name == *name
            });
            match e {
                Some(e) => {
                    e.used = now;
                    Some(e.cached.clone())
                }
                None => {
                    if s.entries.len() >= self.capacity
                        && let Some(i) = (0..s.entries.len()).min_by_key(|&i| s.entries[i].used)
                    {
                        s.entries.swap_remove(i);
                    }
                    s.entries.push(CacheEntry {
                        uri: uri.into(),
                        name: name.into(),
                        tolerant_namespaces: tolerant,
                        len: text.len(),
                        fp,
                        used: now,
                        cached: None,
                    });
                    None
                }
            }
        };
        // the full comparison outside the lock
        let r = match found {
            Some(Some((t, tmpl))) if *t == *text => Lookup::Hit(tmpl),
            // fingerprint collision: parse (and do not cache)
            Some(Some(_)) => Lookup::New,
            Some(None) => Lookup::Again,
            None => Lookup::New,
        };
        let mut s = self.state();
        match r {
            Lookup::Hit(_) => s.hits += 1,
            _ => s.misses += 1,
        }
        r
    }

    fn put(&self, uri: &str, name: &str, tolerant: bool, text: &str, t: Arc<Template>) {
        let fp = fingerprint(text);
        let text: Arc<str> = text.into();
        let mut s = self.state();
        if let Some(e) = s.entries.iter_mut().find(|e| {
            e.fp == fp
                && e.len == text.len()
                && e.tolerant_namespaces == tolerant
                && *e.uri == *uri
                && *e.name == *name
                && e.cached.is_none()
        }) {
            e.cached = Some((text, t));
        }
    }
}

/// A fast 64-bit fingerprint of a text (four independent multiply-rotate lanes).
fn fingerprint(text: &str) -> u64 {
    const K: [u64; 4] = [
        0x9e37_79b9_7f4a_7c15,
        0xc2b2_ae3d_27d4_eb4f,
        0x1656_67b1_9e37_79f9,
        0xff51_afd7_ed55_8ccd,
    ];
    let b = text.as_bytes();
    let mut lanes = K;
    let (chunks, rest) = b.as_chunks::<32>();
    for c in chunks {
        for (i, l) in lanes.iter_mut().enumerate() {
            let w = u64::from_le_bytes(c[i * 8..i * 8 + 8].try_into().unwrap());
            *l = (*l ^ w).wrapping_mul(K[i]).rotate_left(31);
        }
    }
    let mut h = b.len() as u64;
    for x in rest {
        h = (h ^ *x as u64).wrapping_mul(K[0]);
    }
    for l in lanes {
        h = (h ^ l).wrapping_mul(K[1]).rotate_left(27);
    }
    h
}

impl Default for ParseCache {
    /// A cache of 64 resources.
    fn default() -> Self {
        Self::new(64)
    }
}

fn bundled_template(i: usize, tolerant_namespaces: bool) -> &'static Template {
    const N: usize = BUNDLED.len();
    static CACHE: [[OnceLock<Template>; N]; 2] = [const { [const { OnceLock::new() }; N] }; 2];
    CACHE[tolerant_namespaces as usize][i].get_or_init(|| {
        let (key, text) = BUNDLED[i];
        // the display name of a non-file key is the key itself
        let mut l = Loader::new(LoadOptions {
            tolerant_namespaces,
            ..LoadOptions::default()
        });
        let r = l.new_resource(key, None);
        l.parse_into(r, text);
        l.template_of(r, 0, 0)
    })
}

impl Default for Loader {
    fn default() -> Self {
        Self::new(LoadOptions::default())
    }
}

impl Loader {
    pub fn new(opts: LoadOptions) -> Self {
        Loader {
            g: Graph::default(),
            diags: Diagnostics::default(),
            opts,
            by_key: FxHashMap::default(),
            id_index: Vec::new(),
            base_dir: None,
            memory: FxHashMap::default(),
            read_files: true,
            cache: None,
            unresolved: Vec::new(),
        }
    }

    /// Makes `content` (XMI) available as the file at `path` (absolute, or relative to the
    /// current directory like [`Loader::load_file`]). In-memory files take precedence over the
    /// file system for loading and for resolving `href`s; the resource is the same as if the
    /// file had been read from `path`.
    pub fn add_file(&mut self, path: impl AsRef<Path>, content: impl Into<Vec<u8>>) {
        let text = String::from_utf8(content.into())
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
        self.memory
            .insert(normalize_path(path.as_ref()), Arc::new(text));
    }

    /// `false`: never read the file system; files not added with [`Loader::add_file`] (or
    /// bundled) cannot be loaded. Default `true`.
    pub fn set_read_files(&mut self, read_files: bool) {
        self.read_files = read_files;
    }

    /// Loads the in-memory model files (see [`MODEL_EXTENSIONS`]) of directory `dir`, sorted by
    /// file name: the in-memory counterpart of [`Loader::load_dir`].
    pub fn load_memory_dir(&mut self, dir: impl AsRef<Path>) -> Vec<ResId> {
        let dir = normalize_path(dir.as_ref());
        if self.base_dir.is_none() {
            self.base_dir = Some(dir.clone());
        }
        let mut files: Vec<PathBuf> = self
            .memory
            .keys()
            .filter(|p| {
                p.parent() == Some(&dir)
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| MODEL_EXTENSIONS.contains(&e))
            })
            .cloned()
            .collect();
        files.sort();
        files.into_iter().map(|f| self.load_file(f)).collect()
    }

    /// An in-memory file, or (if allowed) a regular file, exists at `p`.
    fn file_exists(&self, p: &Path) -> bool {
        (!self.memory.is_empty() && self.memory.contains_key(&normalize_path(p)))
            || (self.read_files && p.is_file())
    }

    /// Directory display names are relative to (defaults to the directory of the first file).
    pub fn set_base_dir(&mut self, dir: impl AsRef<Path>) {
        self.base_dir = Some(normalize_path(dir.as_ref()));
    }

    pub fn graph(&self) -> &Graph {
        &self.g
    }

    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diags
    }

    fn display_name(&self, key: &str) -> String {
        if let (Some(p), Some(base)) = (key.strip_prefix("file:"), &self.base_dir)
            && let Some(b) = base.to_str()
            && let Some(rest) = p.strip_prefix(b).and_then(|r| r.strip_prefix('/'))
            && is_plain_absolute(p)
            && is_plain_absolute(b)
        {
            // fast path: a plain path below the base directory
            return rest.to_string();
        }
        if let (Some(p), Some(base)) = (key.strip_prefix("file:"), &self.base_dir) {
            let p = normalize_path(Path::new(p));
            // relative path (with ../ as needed)
            let pc: Vec<_> = p.components().collect();
            let bc: Vec<_> = base.components().collect();
            let common = pc.iter().zip(&bc).take_while(|(a, b)| a == b).count();
            let mut rel = PathBuf::new();
            for _ in common..bc.len() {
                rel.push("..");
            }
            for c in &pc[common..] {
                rel.push(c.as_os_str());
            }
            return rel.display().to_string();
        }
        key.to_string()
    }

    /// Loads a model file (and, on `resolve`, everything it references).
    pub fn load_file(&mut self, path: impl AsRef<Path>) -> ResId {
        let p = normalize_path(path.as_ref());
        if self.base_dir.is_none() {
            self.base_dir = p.parent().map(Path::to_path_buf);
        }
        let key = format!("file:{}", p.display());
        self.get_or_load(&key)
    }

    /// Loads a resource by URI (`pathmap://PCM_MODELS/Palladio.resourcetype`, `file:/abs/path`,
    /// `platform:/plugin/...`).
    pub fn load_uri(&mut self, uri: &str) -> ResId {
        let key = normalize_uri(uri);
        self.get_or_load(&key)
    }

    /// Loads XMI text under the given URI key (e.g. `file:/tmp/x.repository`).
    pub fn load_str(&mut self, key: &str, text: &str) -> ResId {
        if let Some(r) = self.by_key.get(key) {
            return *r;
        }
        let r = self.new_resource(key, None);
        self.parse_into(r, text);
        r
    }

    /// Loads all model files (see [`MODEL_EXTENSIONS`]) of a directory, sorted by file name.
    pub fn load_dir(&mut self, dir: impl AsRef<Path>) -> std::io::Result<Vec<ResId>> {
        let dir = normalize_path(dir.as_ref());
        if self.base_dir.is_none() {
            self.base_dir = Some(dir.clone());
        }
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| MODEL_EXTENSIONS.contains(&e))
            })
            .collect();
        files.sort();
        Ok(files.into_iter().map(|f| self.load_file(f)).collect())
    }

    fn new_resource(&mut self, key: &str, path: Option<PathBuf>) -> ResId {
        let r = ResId(self.g.resources.len() as u32);
        let name = self.display_name(key);
        self.g.resources.push(Resource {
            uri: key.to_string(),
            name,
            path,
            ..Default::default()
        });
        self.id_index.push(IdIndex::default());
        self.by_key.insert(key.to_string(), r);
        r
    }

    fn get_or_load(&mut self, key: &str) -> ResId {
        if let Some(r) = self.by_key.get(key) {
            return *r;
        }
        if let Some(i) = BUNDLED.iter().position(|(k, _)| *k == key) {
            let r = self.new_resource(key, None);
            self.splice_bundled(r, i);
            return r;
        }
        let path = key.strip_prefix("file:").map(PathBuf::from);
        let r = self.new_resource(key, path.clone());
        let mem = match &path {
            Some(p) if !self.memory.is_empty() => self.memory.get(&normalize_path(p)).cloned(),
            _ => None,
        };
        if let Some(t) = mem {
            self.parse_into(r, &t);
            return r;
        }
        let text = match &path {
            Some(_) if !self.read_files => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "not an in-memory file",
            )),
            Some(p) => std::fs::read(p).map(|b| {
                String::from_utf8(b)
                    .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
            }),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "unsupported URI scheme",
            )),
        };
        match text {
            Ok(t) => self.parse_into(r, &t),
            Err(e) => {
                let res = &mut self.g.resources[r.0 as usize];
                res.failed = true;
                res.errors.push(format!("cannot read: {e}"));
            }
        }
        r
    }

    /// Adds bundled model `i` as resource `r`: a copy of the objects of the bundled model,
    /// parsed once per process. Parsing a resource only creates and links objects of that
    /// resource (contiguous in `objs`), so the copy with shifted ids is exactly what parsing the
    /// text here would produce.
    fn splice_bundled(&mut self, r: ResId, i: usize) {
        let t = bundled_template(i, self.opts.tolerant_namespaces);
        self.splice(r, t);
    }

    /// Copies a template in as (the just created, empty) resource `r`.
    fn splice(&mut self, r: ResId, t: &Template) {
        let off = self.g.objs.len() as u32;
        let shift = |o: ObjId| ObjId(o.0 + off);
        self.g.objs.reserve(t.objs.len());
        for o in &t.objs {
            let mut o = o.clone();
            o.resource = r;
            o.container = o.container.map(|(c, f)| (shift(c), f));
            for (_, s) in &mut o.slots {
                match s {
                    Slot::Ref(Some(x)) => *x = shift(*x),
                    Slot::Refs(v) => v.iter_mut().for_each(|x| *x = shift(*x)),
                    _ => {}
                }
            }
            self.g.objs.push(o);
        }
        let res = &mut self.g.resources[r.0 as usize];
        res.roots = t.roots.iter().map(|o| shift(*o)).collect();
        res.errors = t.errors.clone();
        res.failed = t.failed;
        self.diags.0.extend(t.diags.iter().cloned());
        self.id_index[r.0 as usize] = IdIndex {
            map: t.id_index.clone(),
            offset: off,
        };
    }

    /// Resource `r` (objects from `first_obj`, diagnostics from `first_diag`) as a template.
    fn template_of(&self, r: ResId, first_obj: usize, first_diag: usize) -> Template {
        let off = first_obj as u32;
        let shift = |o: ObjId| ObjId(o.0 - off);
        let objs = self.g.objs[first_obj..]
            .iter()
            .map(|o| {
                let mut o = o.clone();
                o.resource = ResId(0);
                o.container = o.container.map(|(c, f)| (shift(c), f));
                for (_, s) in &mut o.slots {
                    match s {
                        Slot::Ref(Some(x)) => *x = shift(*x),
                        Slot::Refs(v) => v.iter_mut().for_each(|x| *x = shift(*x)),
                        _ => {}
                    }
                }
                o
            })
            .collect();
        let res = &self.g.resources[r.0 as usize];
        let idx = &self.id_index[r.0 as usize];
        debug_assert_eq!(idx.offset, 0);
        // ids in the index are absolute: rebase them by re-inserting
        let map = if first_obj == 0 {
            idx.map.clone()
        } else {
            Arc::new(idx.map.rebased(off))
        };
        Template {
            objs,
            roots: res.roots.iter().map(|o| shift(*o)).collect(),
            errors: res.errors.clone(),
            failed: res.failed,
            diags: self.diags.0[first_diag..].to_vec(),
            id_index: map,
        }
    }

    /// Uses a [`ParseCache`] for the resources loaded from now on.
    pub fn set_cache(&mut self, cache: Arc<ParseCache>) {
        self.cache = Some(cache);
    }

    fn parse_into(&mut self, r: ResId, text: &str) {
        let Some(cache) = self.cache.clone() else {
            return self.parse_text(r, text);
        };
        let res = &self.g.resources[r.0 as usize];
        let tolerant = self.opts.tolerant_namespaces;
        let again = match cache.get(&res.uri, &res.name, tolerant, text) {
            Lookup::Hit(t) => return self.splice(r, &t),
            Lookup::Again => true,
            Lookup::New => false,
        };
        let (first_obj, first_diag) = (self.g.objs.len(), self.diags.0.len());
        self.parse_text(r, text);
        let res = &self.g.resources[r.0 as usize];
        if again && !res.failed {
            let t = Arc::new(self.template_of(r, first_obj, first_diag));
            cache.put(&res.uri, &res.name, tolerant, text, t);
        }
    }

    fn parse_text(&mut self, r: ResId, text: &str) {
        let key = self.g.resources[r.0 as usize].uri.clone();
        let resolver = |href: &str| resolve_uri(&key, href);
        let opts = ParseOptions {
            tolerant_namespaces: self.opts.tolerant_namespaces,
        };
        let parser = Parser::new(&mut self.g, r, &key, &resolver, &mut self.diags, opts);
        match parser.parse(text) {
            Ok(p) => {
                self.id_index[r.0 as usize] = IdIndex {
                    map: Arc::new(p.id_index),
                    offset: 0,
                }
            }
            Err(e) => {
                let res = &mut self.g.resources[r.0 as usize];
                res.failed = true;
                res.errors.push(e.clone());
                let name = res.name.clone();
                self.diags.push(Level::Error, "xml-syntax", name, e);
            }
        }
    }

    /// `Resource.getEObject(fragment)` on a fully loaded resource.
    fn get_object(&self, r: ResId, frag: &str) -> Option<ObjId> {
        if frag.starts_with('/') {
            return navigate_path(&self.g, &self.g.resources[r.0 as usize].roots, frag);
        }
        self.id_index[r.0 as usize].get(frag)
    }

    /// Resolves every proxy, demand-loading referenced resources (`EcoreUtil.resolveAll`).
    pub fn resolve_all(&mut self) {
        self.unresolved.clear();
        let mut ri = 0;
        // scratch buffers
        let (mut order, mut stack) = (Vec::new(), Vec::new());
        let (mut slots, mut targets) = (Vec::new(), Vec::new());
        while ri < self.g.resources.len() {
            let r = ResId(ri as u32);
            ri += 1;
            order.clear();
            self.g.all_contents_into(r, &mut order, &mut stack);
            for &o in &order {
                slots.clear();
                slots.extend(
                    self.g[o]
                        .slots
                        .iter()
                        .filter(|(f, s)| {
                            matches!(s, Slot::Ref(Some(_)) | Slot::Refs(_))
                                && !f.is_containment()
                                && !f.is_container()
                        })
                        .map(|(f, _)| *f),
                );
                for &f in &slots {
                    targets.clear();
                    targets.extend(
                        self.g
                            .get_refs(o, f)
                            .iter()
                            .copied()
                            .filter(|t| self.g[*t].proxy.is_some()),
                    );
                    for &t in &targets {
                        if self.g[t].proxy.is_none() {
                            continue;
                        }
                        match self.resolve_proxy(o, t) {
                            Some(res)
                                if f.target().is_some_and(|tc| !self.g[res].class.is_a(tc)) =>
                            {
                                let msg = format!(
                                    "{} resolves to incompatible {}",
                                    f.name(),
                                    self.g.describe(res)
                                );
                                let loc = self.g.describe(o);
                                self.diags.push(Level::Error, "wrong-type", loc, msg);
                            }
                            Some(res) => {
                                self.g.replace_target(o, f, t, res);
                                continue;
                            }
                            None => {}
                        }
                        // still a proxy: reported by `finish` (in this order) if persistent
                        if f.is_cross_reference() {
                            self.unresolved.push((o, f, t));
                        }
                    }
                }
            }
        }
    }

    fn resolve_proxy(&mut self, holder: ObjId, p: ObjId) -> Option<ObjId> {
        // fast path: the target resource is loaded and has the object
        {
            let uri = self.g[p].proxy.as_deref()?;
            let (key, frag) = uri.split_once('#').unwrap_or((uri, ""));
            if let Some(&r) = self.by_key.get(key)
                && let Some(t) = self.get_object(r, frag)
            {
                return Some(t);
            }
        }
        let uri = self.g[p].proxy.clone()?;
        let (key, frag) = uri.split_once('#').unwrap_or((&uri, ""));
        let known = self.by_key.get(key).copied();
        let r = match known {
            Some(r) => Some(r),
            None if self.opts.follow_references => Some(self.get_or_load(key)),
            None => None,
        };
        if let Some(r) = r
            && let Some(t) = self.get_object(r, frag)
        {
            return Some(t);
        }
        if self.opts.tolerant_references
            && let Some(t) = self.tolerant_lookup(holder, key, frag)
        {
            let msg = format!("href {uri} resolved to {}", self.g.describe_ref(t));
            let loc = self.g.describe(holder);
            self.diags.push(Level::Warning, "tolerant-ref", loc, msg);
            return Some(t);
        }
        None
    }

    fn tolerant_lookup(&mut self, holder: ObjId, key: &str, frag: &str) -> Option<ObjId> {
        // 1. same file name next to the referencing resource
        let hres = &self.g.resources[self.g[holder].resource.0 as usize];
        let fname = key.rsplit(['/', '\\']).next().unwrap_or(key).to_string();
        if let Some(hp) = hres.uri.strip_prefix("file:") {
            let cand = Path::new(hp).parent().map(|d| d.join(&fname));
            if let Some(c) = cand.filter(|c| self.file_exists(c)) {
                let ck = format!("file:{}", normalize_path(&c).display());
                if ck != key {
                    let r = self.get_or_load(&ck);
                    if let Some(t) = self.get_object(r, frag) {
                        return Some(t);
                    }
                }
            }
        }
        // 2. a unique ID over all loaded resources
        if frag.is_empty() || frag.starts_with('/') {
            return None;
        }
        let mut found = self.id_index.iter().filter_map(|m| m.get(frag));
        let first = found.next()?;
        found.next().is_none().then_some(first)
    }

    /// Resolves everything and returns the graph and diagnostics.
    pub fn finish(mut self) -> (Graph, Diagnostics) {
        self.resolve_all();
        // the same file reached through different URIs is loaded twice (EMF semantics)
        let mut by_path: HashMap<PathBuf, Vec<&str>> = HashMap::new();
        for r in &self.g.resources {
            if let (Some(p), false) = (&r.path, r.roots.is_empty()) {
                by_path.entry(normalize_path(p)).or_default().push(&r.uri);
            }
        }
        let mut dups: Vec<_> = by_path.into_iter().filter(|(_, v)| v.len() > 1).collect();
        dups.sort();
        for (p, uris) in dups {
            let msg = format!(
                "loaded {} times under different URIs ({}); objects are duplicated",
                uris.len(),
                uris.join(", ")
            );
            self.diags.push(
                Level::Warning,
                "duplicate-resource",
                p.display().to_string(),
                msg,
            );
        }
        // unresolved references: the proxies `resolve_all` could not resolve, in tree order (the
        // graph does not change after `resolve_all`)
        for (o, f, t) in std::mem::take(&mut self.unresolved) {
            let p = self.g[t].proxy.clone().unwrap_or_default();
            let loc = self.g.describe(o);
            self.diags.push(
                Level::Error,
                "unresolved-ref",
                loc,
                format!("{}: cannot resolve {p}", f.name()),
            );
        }
        (self.g, self.diags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_uri_fast_path_is_exact() {
        let bases = [
            "file:/a/b/x.repository",
            "file:/x.repository",
            "file:/a/b/../c/x.system",
            "file:/a//b/x.system",
            "file:/a/./b/x.system",
            "file:/a/b/",
            "pathmap://PCM_MODELS/Palladio.resourcetype",
        ];
        let hrefs = [
            "y.repository",
            "sub/y.repository",
            "../y.repository",
            "./y.repository",
            "sub//y.repository",
            "sub/",
            "/abs/y.repository",
            "y%20z.repository",
            "a.b/c-d_e.system",
        ];
        for b in bases {
            for h in hrefs {
                assert_eq!(resolve_uri(b, h), resolve_uri_general(b, h), "{b} {h}");
            }
        }
    }
}

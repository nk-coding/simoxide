//! Comparison harness between `qvto_sema::compile` and Eclipse QVTo's full-compile oracle
//! (`tests/fixtures/eclipse-compile-oracle.tsv`, made by `tools/oracle/make-compile-fixture.sh`
//! from `tests/data/compile.cases`). Shared by `tests/oracle.rs` and
//! `examples/oracle_report.rs`.
//!
//! Per case it sets up what the reference runner (`reference/runner`, `CaseRunner.java`,
//! `BundleUnitResolverFactory.java`, `Env.java`) gave Eclipse:
//!
//! - the package registry: the packages the `plugin.xml` files on the runner's classpath
//!   register (EMF, OCL, QVTo jars; the test bundle; the samples plugin: [`ECLIPSE_REGISTERED`]),
//!   plus every nsURI of the PCM configuration (`target/pcm/pcm.conf`; the oracle's warm-up pass
//!   applied it process-wide before the recorded pass, so every case saw it), plus the case's
//!   `mmfile`s. The `.ecore` files are found through `crates/qvto-emf/tests/data/suite.conf` and
//!   the PCM configuration;
//! - unit resolution: the main unit's resolver as `UnitResolverFactory.Registry` picks it (the
//!   runner's bundle factory: source containers of the bundles below `tests/`, or the case's
//!   `imports root`; else QVTo's `URIUnitResolverFactory`: the file's folder), then the blackbox
//!   units (registered: `bundle-blackboxes` or the case's own `blackbox` lines; then Java classes
//!   by name), then `ClassPathUnitResolver` (`.qvto` files below the classpath directories);
//!   units found there resolve their own imports against that root and blackboxes only;
//! - Java blackbox units from `crates/qvto-types/data/blackboxes.tsv`.
//!
//! Problems are reported as the runner prints them: per unit (main unit and every import, each
//! once), the unit's file name (last URI segment), line (-1 for Java units), UTF-16 offset and
//! length, message with `\\ \n \t \r` escaped.

#![allow(dead_code)]

use qvto_emf::ResourceSet;
use qvto_emf::config::Config;
use qvto_model::{MetaRegistry, ModelStore};
use qvto_sema::{
    BlackboxModule, BlackboxOp, BlackboxOpKind, BlackboxParam, BlackboxUnit, Severity,
    UnitResolver, UnitSource,
};
use qvto_testkit::case::{Case, Check};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Once, mpsc};
use std::time::{Duration, Instant};

/// nsURIs that EMF's standalone `ExtensionProcessor` registers from the `plugin.xml` files on
/// the reference runner's classpath (`generated_package` / `dynamic_package` extensions of the
/// jars in `reference/runner/lib/qvto-3.11.0`, of `tests/org.eclipse.m2m.tests.qvt.oml` and of
/// `plugins/org.eclipse.m2m.qvt.oml.samples`).
pub const ECLIPSE_REGISTERED: &[&str] = &[
    "http://www.eclipse.org/emf/2002/Ecore",
    "http://www.eclipse.org/emf/2003/XMLType",
    "http://www.w3.org/XML/1998/namespace",
    "http:///org/eclipse/emf/ecore/util/ExtendedMetaData",
    "http:///org/eclipse/emf/ecore/util/EcoreAnnotation",
    "http:///org/eclipse/emf/ecore/util/DateConversionDelegateAnnotation",
    "http://www.eclipse.org/emf/2003/Change",
    "http://www.eclipse.org/QVT2/1.0.0/Operational/cst",
    "http://www.eclipse.org/qvt/1.0/ImperativeOCL",
    "http:///www.eclipse.org/m2m/qvt/operational/trace.ecore",
    "http://www.eclipse.org/qvt/1.0.0/Operational",
    "http://www.eclipse.org/qvt/1.0.0/Operational/Expressions",
    "http://www.eclipse.org/ocl/1.1.0/Ecore",
    "http://www.eclipse.org/ocl/1.1.0/oclstdlib.ecore",
    "http://www.eclipse.org/ocl/1.1.0/OCL/Types",
    "http://www.eclipse.org/ocl/1.1.0/OCL",
    "http://www.eclipse.org/ocl/1.1.0/OCL/Expressions",
    "http://www.eclipse.org/ocl/1.1.0/OCL/Utilities",
    "http://www.eclipse.org/ocl/1.1.0/OCL/CST",
    "http://www.eclipse.org/qvt/1.0.0/Operational/examples/simpleuml",
    "http://www.eclipse.org/qvt/1.0.0/Operational/examples/rdb",
    "http://www.eclipse.org/qvt/1.0.0/Operational/examples/rdb/constraints",
    "http://www.eclipse.org/qvt/1.0.0/Operational/examples/rdb/datatypes",
    "http://www.eclipse.org/qvt/1.0.0/Operational/examples/rdb/view",
    "http://www.eclipse.org/m2m/qvt/oml/testqvt",
    "http://www.eclipse.org/m2m/qvt/oml/generics",
    "http://bug577992",
    "http://www.eclipse.org/m2m/qvt/oml/test_dynamicpkg",
    "http://www.eclipse.org/m2m/qvt/oml/test/registeredDynamic",
];

/// Registry keys that are another key's package (`QVTOperationalPackage` is registered under
/// both spellings).
pub const ECLIPSE_ALIASES: &[(&str, &str)] = &[(
    "http://www.eclipse.org/QVT/1.0.0/Operational",
    "http://www.eclipse.org/qvt/1.0.0/Operational",
)];

/// Models of the EMF jars (not in the reference checkouts), copied to
/// `tests/fixtures/emf-models`: (nsURI, platform URI).
const EMF_JAR_MODELS: &[(&str, &str)] = &[
    (
        "http://www.eclipse.org/emf/2003/XMLType",
        "platform:/plugin/org.eclipse.emf.ecore/model/XMLType.ecore",
    ),
    (
        "http://www.w3.org/XML/1998/namespace",
        "platform:/plugin/org.eclipse.emf.ecore/model/XMLNamespace.ecore",
    ),
    (
        "http:///org/eclipse/emf/ecore/util/ExtendedMetaData",
        "platform:/plugin/org.eclipse.emf.ecore/model/ExtendedMetaData.ecore",
    ),
    (
        "http:///org/eclipse/emf/ecore/util/EcoreAnnotation",
        "platform:/plugin/org.eclipse.emf.ecore/model/EcoreAnnotation.ecore",
    ),
    (
        "http:///org/eclipse/emf/ecore/util/DateConversionDelegateAnnotation",
        "platform:/plugin/org.eclipse.emf.ecore/model/DateConversionDelegateAnnotation.ecore",
    ),
    (
        "http://www.eclipse.org/emf/2003/Change",
        "platform:/plugin/org.eclipse.emf.ecore.change/model/Change.ecore",
    ),
];

const ECORE_NS: &str = "http://www.eclipse.org/emf/2002/Ecore";
const TEST_BUNDLE: &str = "reference/eclipse-qvto/tests/org.eclipse.m2m.tests.qvt.oml";
const SAMPLES_PLUGIN: &str = "reference/eclipse-qvto/plugins/org.eclipse.m2m.qvt.oml.samples";
const PCM_CONF: &str = "target/pcm/pcm.conf";

/// One compile problem in the oracle's terms.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Prob {
    pub unit: String,
    /// `E` or `W`.
    pub sev: char,
    pub line: i64,
    pub offset: i64,
    pub len: i64,
    pub msg: String,
}

/// Eclipse's result for one key.
#[derive(Clone, Debug, Default)]
pub struct OracleEntry {
    pub problems: Vec<Prob>,
    pub load: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Ok,
    Panic(String),
    Timeout,
    /// The case could not be set up (missing file, ...).
    Setup(String),
}

/// Our result for one case.
#[derive(Clone, Debug)]
pub struct CaseRun {
    pub id: String,
    pub key: String,
    pub status: RunStatus,
    pub problems: Vec<Prob>,
    /// Time of the `compile` call only.
    pub elapsed: Duration,
    /// IR consistency violations (`irck::check_program`) of an error-free compilation.
    pub ir_issues: Vec<String>,
}

impl CaseRun {
    pub fn errors(&self) -> usize {
        self.problems.iter().filter(|p| p.sev == 'E').count()
    }
}

/// Multiset comparison of two problem lists: (matched, missing = oracle only, extra = ours only).
pub fn compare(oracle: &[Prob], ours: &[Prob]) -> (Vec<Prob>, Vec<Prob>, Vec<Prob>) {
    let mut pool: HashMap<&Prob, usize> = HashMap::new();
    for p in ours {
        *pool.entry(p).or_default() += 1;
    }
    let mut matched = Vec::new();
    let mut missing = Vec::new();
    for p in oracle {
        match pool.get_mut(p) {
            Some(n) if *n > 0 => {
                *n -= 1;
                matched.push(p.clone());
            }
            _ => missing.push(p.clone()),
        }
    }
    // what is left in the pool are our unmatched problems
    let mut extra = Vec::new();
    for p in ours {
        if let Some(n) = pool.get_mut(p)
            && *n > 0
        {
            *n -= 1;
            extra.push(p.clone());
        }
    }
    (matched, missing, extra)
}

/// The case's group for reports: the oracle key's origin (`eclipse`, `simulizar`, `spd`).
pub fn group(key: &str) -> &str {
    key.split(':').next().unwrap_or("?")
}

/// `check problems E W [cst]` of a case.
pub fn problem_check(case: &Case) -> Option<(i64, i64, bool)> {
    case.checks.iter().find_map(|c| match c {
        Check::Problems {
            errors,
            warnings,
            cst_only,
        } => Some((*errors, *warnings, *cst_only)),
        _ => None,
    })
}

pub fn note<'a>(case: &'a Case, prefix: &str) -> Option<&'a str> {
    case.notes.iter().find_map(|n| n.strip_prefix(prefix))
}

pub fn case_key(case: &Case) -> String {
    note(case, "note key ").unwrap_or(&case.id).to_string()
}

pub fn case_env(case: &Case) -> String {
    note(case, "note env ")
        .unwrap_or("?")
        .split(' ')
        .next()
        .unwrap_or("?")
        .to_string()
}

/// `CaseRunner.oneLine`
pub fn one_line(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\r', "\\r")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

/// Lexically normalised path (`a/../b` is `b`), as Java's `Path.normalize`.
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir if out.file_name().is_some() => {
                out.pop();
            }
            Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

fn canonical(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

// ---- bundles and resolvers --------------------------------------------------------------------

/// A bundle of `BundleUnitResolverFactory`: canonical root and source containers.
#[derive(Clone, Debug)]
struct Bundle {
    root: PathBuf,
    containers: Vec<String>,
}

fn strip_comments(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(i) = rest.find("<!--") {
        out.push_str(&rest[..i]);
        match rest[i..].find("-->") {
            Some(j) => rest = &rest[i + j + 3..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(i) = tag[from..].find(name) {
        let at = from + i;
        let before_ok = at == 0 || tag.as_bytes()[at - 1].is_ascii_whitespace();
        let after = tag[at + name.len()..].trim_start();
        if before_ok && let Some(v) = after.strip_prefix('=') {
            let v = v.trim_start();
            let q = v.chars().next()?;
            if q == '"' || q == '\'' {
                let end = v[1..].find(q)?;
                return Some(v[1..1 + end].to_string());
            }
        }
        from = at + name.len();
    }
    None
}

/// `sourceContainer` paths of a `plugin.xml` (as `BundleUnitResolverFactory.install` reads them).
fn source_containers(plugin_xml: &str) -> Vec<String> {
    let text = strip_comments(plugin_xml);
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(i) = rest.find("<sourceContainer") {
        let tag_end = rest[i..].find('>').map_or(rest.len(), |e| i + e);
        let tag = &rest[i + "<sourceContainer".len()..tag_end];
        if let Some(p) = attr(tag, "path") {
            let p = p.trim();
            if !p.is_empty() {
                out.push(if p == "/" {
                    String::new()
                } else {
                    p.trim_matches('/').to_string()
                });
            }
        }
        rest = &rest[tag_end.min(rest.len())..];
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

/// A unit resolver of the oracle's setup (`URIUnitResolver`): base directories, and whether its
/// parent includes `ClassPathUnitResolver`.
#[derive(Clone, Debug)]
struct FileResolver {
    bases: Vec<PathBuf>,
    classpath: bool,
}

/// What the worker thread needs to resolve units (shared, immutable).
pub struct ResolveData {
    base: PathBuf,
    bundles: Vec<Bundle>,
    classpath_roots: Vec<PathBuf>,
    /// env -> unit qualified name -> (file name, unit)
    blackboxes: HashMap<String, HashMap<String, (String, BlackboxUnit)>>,
    /// case id -> env of its own blackbox registrations
    case_bb_env: HashMap<String, String>,
    /// `CaseRunner.norm`: (absolute prefix, replacement)
    norm_prefixes: Vec<(String, String)>,
}

/// The resolver for one compilation.
struct CaseResolver<'a> {
    data: &'a ResolveData,
    /// Blackbox envs, in lookup order (registered, then Java classes by name).
    bb_envs: Vec<String>,
    /// Qualified name of the main unit.
    main_qname: String,
    resolvers: RefCell<Vec<FileResolver>>,
    /// unit key -> index into `resolvers`
    unit_resolver: RefCell<HashMap<String, usize>>,
}

/// `file:` URI of a directory, with a trailing `/`.
fn file_dir_uri(p: &Path) -> String {
    let mut u = qvto_emf::uri::Uri::file(&p.to_string_lossy()).to_string();
    if !u.ends_with('/') {
        u.push('/');
    }
    u
}

fn file_key(p: &Path) -> String {
    format!("file:{}", p.display())
}

fn read_text(p: &Path) -> Option<Rc<str>> {
    let bytes = std::fs::read(p).ok()?;
    Some(Rc::from(String::from_utf8_lossy(&bytes).into_owned()))
}

impl CaseResolver<'_> {
    fn main_unit(&self, main: &Path) -> Option<UnitSource> {
        self.file_unit(main.to_path_buf(), self.main_qname.clone(), 0)
    }

    fn file_unit(
        &self,
        path: PathBuf,
        qualified_name: String,
        resolver: usize,
    ) -> Option<UnitSource> {
        let text = read_text(&path)?;
        let key = file_key(&path);
        self.unit_resolver
            .borrow_mut()
            .insert(key.clone(), resolver);
        Some(UnitSource {
            key,
            qualified_name,
            file_name: path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: Some(path),
            text,
            blackbox: None,
        })
    }

    fn try_bases(&self, r: usize, qn: &str) -> Option<UnitSource> {
        let rel = format!("{}.qvto", qn.replace('.', "/"));
        let bases = self.resolvers.borrow()[r].bases.clone();
        for b in bases {
            let p = b.join(&rel);
            if p.is_file() {
                return self.file_unit(p, qn.to_string(), r);
            }
        }
        None
    }

    fn try_blackbox(&self, qn: &str) -> Option<UnitSource> {
        for env in &self.bb_envs {
            if let Some((file_name, unit)) = self.data.blackboxes.get(env).and_then(|m| m.get(qn)) {
                return Some(UnitSource {
                    key: format!("qvto://blackbox/{qn}"),
                    qualified_name: qn.to_string(),
                    file_name: file_name.clone(),
                    path: None,
                    text: Rc::from(""),
                    blackbox: Some(Rc::new(unit.clone())),
                });
            }
        }
        None
    }

    fn try_classpath(&self, qn: &str) -> Option<UnitSource> {
        let rel = format!("{}.qvto", qn.replace('.', "/"));
        for root in &self.data.classpath_roots {
            if root.join(&rel).is_file() {
                let mut rs = self.resolvers.borrow_mut();
                rs.push(FileResolver {
                    bases: vec![root.clone()],
                    classpath: false,
                });
                let idx = rs.len() - 1;
                drop(rs);
                return self.try_bases(idx, qn);
            }
        }
        None
    }
}

impl UnitResolver for CaseResolver<'_> {
    fn blackbox_units(&self) -> Vec<UnitSource> {
        let mut out = Vec::new();
        let Some(env) = self.bb_envs.first().filter(|e| e.as_str() != "classpath") else {
            return out;
        };
        if let Some(m) = self.data.blackboxes.get(env) {
            let mut names: Vec<&String> = m.keys().collect();
            names.sort();
            for qn in names {
                if let Some(u) = self.try_blackbox(qn) {
                    out.push(u);
                }
            }
        }
        out
    }

    fn resolve(&self, importer: &UnitSource, qualified_name: &str) -> Option<UnitSource> {
        let r = self
            .unit_resolver
            .borrow()
            .get(&importer.key)
            .copied()
            .unwrap_or(0);
        if let Some(u) = self.try_bases(r, qualified_name) {
            return Some(u);
        }
        if let Some(u) = self.try_blackbox(qualified_name) {
            return Some(u);
        }
        if self.resolvers.borrow()[r].classpath {
            return self.try_classpath(qualified_name);
        }
        None
    }
}

impl ResolveData {
    /// `CaseRunner.norm`: absolute paths of the base directory (as given and resolved) become
    /// `${root}`.
    fn norm(&self, s: &str) -> String {
        let mut s = s.to_string();
        for (from, to) in &self.norm_prefixes {
            if s.contains(from.as_str()) {
                s = s.replace(from.as_str(), to);
            }
        }
        s
    }

    /// The resolver of a case's compilation.
    fn case_resolver(&self, case: &Case, main: &Path) -> CaseResolver<'_> {
        let (main_resolver, qname) = self.main_resolver(case, main);
        let mut bb_envs = Vec::new();
        if case.bundle_blackboxes {
            bb_envs.push("bundle".to_string());
        }
        if let Some(e) = self.case_bb_env.get(&case.id) {
            bb_envs.push(e.clone());
        }
        bb_envs.push("classpath".to_string());
        CaseResolver {
            data: self,
            bb_envs,
            main_qname: qname,
            resolvers: RefCell::new(vec![main_resolver]),
            unit_resolver: RefCell::new(HashMap::new()),
        }
    }

    /// The main unit's resolver and qualified name, as `UnitResolverFactory.Registry.getUnit`
    /// picks them in the runner (bundle factory first, then `URIUnitResolverFactory`).
    fn main_resolver(&self, case: &Case, main: &Path) -> (FileResolver, String) {
        let canon = canonical(main);
        let bundle: Option<Bundle> = if case.imports_from_folder {
            None
        } else if let Some(root) = &case.import_root {
            let root = canonical(&normalize(&self.base.join(root)));
            canon.starts_with(&root).then(|| Bundle {
                root,
                containers: vec![String::new()],
            })
        } else {
            self.bundles
                .iter()
                .find(|b| canon.starts_with(&b.root))
                .cloned()
        };
        if let Some(b) = bundle
            && let Ok(rel) = canon.strip_prefix(&b.root)
        {
            // the bundle root as the URI spells it
            let rel_s = rel.to_string_lossy().into_owned();
            let abs_s = main.to_string_lossy().into_owned();
            let root = match abs_s.strip_suffix(&rel_s) {
                Some(r) if r.ends_with('/') => PathBuf::from(r.trim_end_matches('/')),
                _ => b.root.clone(),
            };
            let bases = b
                .containers
                .iter()
                .map(|c| {
                    if c.is_empty() {
                        root.clone()
                    } else {
                        root.join(c)
                    }
                })
                .collect();
            let mut q = rel_s.clone();
            if let (Some(dot), slash) = (q.rfind('.'), q.rfind('/'))
                && slash.is_none_or(|s| dot > s)
            {
                q.truncate(dot);
            }
            for c in &b.containers {
                if !c.is_empty() && q.starts_with(&format!("{c}/")) {
                    q = q[c.len() + 1..].to_string();
                    break;
                }
            }
            return (
                FileResolver {
                    bases,
                    classpath: true,
                },
                q.replace('/', "."),
            );
        }
        let folder = main.parent().map(Path::to_path_buf).unwrap_or_default();
        let stem = main
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        (
            FileResolver {
                bases: vec![folder],
                classpath: true,
            },
            stem,
        )
    }
}

// ---- blackboxes.tsv ---------------------------------------------------------------------------

fn parse_direction(s: &str) -> qvto_sema::ir::Direction {
    use qvto_sema::ir::Direction;
    match s {
        "out" => Direction::Out,
        "inout" => Direction::InOut,
        _ => Direction::In,
    }
}

fn parse_kind(s: &str) -> BlackboxOpKind {
    match s {
        "query" => BlackboxOpKind::Query,
        "mapping" => BlackboxOpKind::Mapping,
        "constructor" => BlackboxOpKind::Constructor,
        _ => BlackboxOpKind::Helper,
    }
}

/// Reads `crates/qvto-types/data/blackboxes.tsv`: env -> unit -> (file name, unit), and the case
/// ids listed per env.
/// Blackbox units by env and qualified name: (file name, unit).
type BlackboxEnvs = HashMap<String, HashMap<String, (String, BlackboxUnit)>>;

fn read_blackboxes(path: &Path) -> Result<(BlackboxEnvs, HashMap<String, String>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut envs: HashMap<String, HashMap<String, (String, BlackboxUnit)>> = HashMap::new();
    let mut case_env = HashMap::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        let get = |i: usize| f.get(i).copied().unwrap_or("");
        match f[0] {
            "env" => {
                envs.entry(get(1).to_string()).or_default();
                if get(4) != "-" {
                    for id in get(4).split(',').filter(|s| !s.is_empty()) {
                        case_env.insert(id.to_string(), get(1).to_string());
                    }
                }
            }
            "unit" => {
                let uri = get(3);
                let file_name = uri.rsplit('/').next().unwrap_or(uri).to_string();
                let unit = BlackboxUnit {
                    modules: Vec::new(),
                    load_error: get(4) != "OK",
                    alt: None,
                };
                envs.entry(get(1).to_string())
                    .or_default()
                    .insert(get(2).to_string(), (file_name, unit));
            }
            "module" => {
                let m = BlackboxModule {
                    name: get(3).to_string(),
                    library: get(4) == "Library",
                    metamodels: get(6)
                        .split(',')
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect(),
                    ops: Vec::new(),
                };
                if let Some((_, u)) = envs.get_mut(get(1)).and_then(|e| e.get_mut(get(2))) {
                    u.modules.push(m);
                } else {
                    return Err(format!("blackboxes.tsv: module before its unit: {line}"));
                }
            }
            "op" => {
                let params = get(8)
                    .split(';')
                    .filter(|s| !s.is_empty())
                    .map(|p| {
                        let mut it = p.splitn(3, ':');
                        BlackboxParam {
                            name: it.next().unwrap_or("").to_string(),
                            direction: parse_direction(it.next().unwrap_or("in")),
                            ty: it.next().unwrap_or("").to_string(),
                        }
                    })
                    .collect();
                let flags: Vec<&str> = get(11)
                    .split([',', ' '])
                    .filter(|s| !s.is_empty())
                    .collect();
                let op = BlackboxOp {
                    name: get(4).to_string(),
                    kind: parse_kind(get(5)),
                    context: (get(6) != "-" && !get(6).is_empty()).then(|| get(6).to_string()),
                    params,
                    result: get(9).to_string(),
                    hidden: flags.contains(&"hidden"),
                    java_static: flags.contains(&"javaStatic"),
                    with_context: flags.contains(&"withExecutionContext"),
                    java_method: get(10).to_string(),
                };
                let m = envs
                    .get_mut(get(1))
                    .and_then(|e| e.get_mut(get(2)))
                    .and_then(|(_, u)| u.modules.iter_mut().find(|m| m.name == get(3)));
                match m {
                    Some(m) => m.ops.push(op),
                    None => return Err(format!("blackboxes.tsv: op before its module: {line}")),
                }
            }
            _ => {}
        }
    }
    for e in envs.values_mut() {
        let mut list: Vec<(String, BlackboxUnit)> = e.iter().map(|(k, (_, u))| (k.clone(), u.clone())).collect();
        qvto_sema::attach_alternatives(&mut list);
        let files: HashMap<String, String> = e.iter().map(|(k, (f, _))| (k.clone(), f.clone())).collect();
        *e = list.into_iter().map(|(k, u)| { let f = files[&k].clone(); (k, (f, u)) }).collect();
    }
    Ok((envs, case_env))
}

// ---- the oracle -------------------------------------------------------------------------------

/// Reads `eclipse-compile-oracle.tsv`.
pub fn read_oracle(path: &Path) -> Result<HashMap<String, OracleEntry>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut out: HashMap<String, OracleEntry> = HashMap::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.splitn(7, '\t').collect();
        let e = out.entry(f[0].to_string()).or_default();
        match f.get(1).copied() {
            Some("E") | Some("W") if f.len() == 7 => e.problems.push(Prob {
                unit: f[2].to_string(),
                sev: f[1].chars().next().unwrap(),
                line: f[3].parse().unwrap_or(0),
                offset: f[4].parse().unwrap_or(0),
                len: f[5].parse().unwrap_or(0),
                msg: f[6].to_string(),
            }),
            Some("LOAD") => e.load = f.get(2).unwrap_or(&"").to_string(),
            _ => {}
        }
    }
    Ok(out)
}

// ---- the harness ------------------------------------------------------------------------------

pub struct Harness {
    pub base: PathBuf,
    pub cases: Vec<Case>,
    pub oracle: HashMap<String, OracleEntry>,
    /// Setup remarks: registered nsURIs without a loadable `.ecore`, PCM availability.
    pub notes: Vec<String>,
    pub pcm_available: bool,
    /// Time to build the base registry.
    pub setup_time: Duration,
    base_rs: ResourceSet,
    base_meta: MetaRegistry,
    base_ops: Vec<qvto_sema::MetaOperation>,
    data: Arc<ResolveData>,
    /// Per-case limit; a case over it is reported as a timeout (its thread is left running).
    pub timeout: Duration,
}

/// The repository root (the worktree), from this crate's manifest directory.
pub fn repo_root() -> PathBuf {
    normalize(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let ours = std::thread::current()
                .name()
                .is_some_and(|n| n.starts_with("oracle-case"));
            if ours {
                let loc = info
                    .location()
                    .map(|l| format!("{}:{}", l.file(), l.line()))
                    .unwrap_or_default();
                let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = info.payload().downcast_ref::<String>() {
                    s.clone()
                } else {
                    "?".to_string()
                };
                LAST_PANIC.with(|p| *p.borrow_mut() = Some(format!("{msg} @ {loc}")));
            } else {
                default(info);
            }
        }));
    });
}

thread_local! {
    static LAST_PANIC: RefCell<Option<String>> = const { RefCell::new(None) };
}

impl Harness {
    pub fn new() -> Result<Harness, String> {
        let t0 = Instant::now();
        let base = repo_root();
        let sema = base.join("crates/qvto-sema");
        let cases = qvto_testkit::case::read_cases(&sema.join("tests/data/compile.cases"))
            .map_err(|e| e.to_string())?;
        let oracle = read_oracle(&sema.join("tests/fixtures/eclipse-compile-oracle.tsv"))?;
        let mut notes = Vec::new();

        // the package registry
        let mut conf = Config::read(&base.join("crates/qvto-emf/tests/data/suite.conf"), &base)
            .map_err(|e| format!("suite.conf: {e}"))?;
        let pcm_path = base.join(PCM_CONF);
        let mut pcm_ns: Vec<String> = Vec::new();
        let pcm_available = pcm_path.is_file();
        let mut pcm_aliases = Vec::new();
        if pcm_available {
            let pcm = Config::read(&pcm_path, &base).map_err(|e| format!("pcm.conf: {e}"))?;
            conf.maps.extend(pcm.maps);
            for (k, v) in pcm.ns_index {
                pcm_ns.push(k.clone());
                conf.ns_index.entry(k).or_insert(v);
            }
            pcm_aliases = pcm.aliases.clone();
            conf.aliases.extend(pcm.aliases);
        } else {
            notes.push(format!(
                "{PCM_CONF} missing: SimuLizar/SPD cases are skipped"
            ));
        }
        let emf_dir = file_dir_uri(&sema.join("tests/fixtures/emf-models"));
        conf.maps.push((
            "platform:/plugin/org.eclipse.emf.ecore/model/".into(),
            emf_dir.clone(),
        ));
        conf.maps.push((
            "platform:/plugin/org.eclipse.emf.ecore.change/model/".into(),
            emf_dir,
        ));
        for (ns, loc) in EMF_JAR_MODELS {
            conf.ns_index
                .entry(ns.to_string())
                .or_insert_with(|| loc.to_string());
        }
        let mut rs = ResourceSet::new();
        for (from, to) in &conf.maps {
            rs.map_uri(from, to);
        }
        for (key, ns) in &conf.aliases {
            rs.add_alias(key, ns);
        }
        for (ns, loc) in &conf.ns_index {
            rs.add_ns_location(ns, loc);
        }
        let mut wanted: Vec<String> = ECLIPSE_REGISTERED.iter().map(|s| s.to_string()).collect();
        wanted.extend(pcm_ns);
        for ns in &wanted {
            if ns == ECORE_NS || rs.graph.meta().package_by_ns_uri(ns).is_some() {
                continue;
            }
            match conf.ns_index.get(ns) {
                Some(loc) => {
                    if let Err(e) = rs.load_metamodel(loc) {
                        notes.push(format!("registered package {ns}: {loc} does not load: {e}"));
                    }
                    if rs.graph.meta().package_by_ns_uri(ns).is_none() {
                        notes.push(format!(
                            "registered package {ns}: not registered after loading {loc}"
                        ));
                    }
                }
                None => notes.push(format!(
                    "registered package {ns}: no .ecore file (not in the registry)"
                )),
            }
        }
        // `alias` lines of the PCM configuration are global registry keys in the oracle
        let aliases = ECLIPSE_ALIASES
            .iter()
            .map(|(k, n)| (k.to_string(), n.to_string()));
        for (key, ns) in aliases.chain(pcm_aliases) {
            match rs.graph.meta().package_by_ns_uri(&ns) {
                Some(p) => rs.graph.meta_mut().register_alias(&key, p),
                None => notes.push(format!("registry alias {key}: {ns} not registered")),
            }
        }
        let base_meta = rs.graph.meta().clone();
        let base_ops = meta_operations(&rs.graph);

        // resolvers
        let mut bundles = Vec::new();
        let tests_dir = base.join("reference/eclipse-qvto/tests");
        if let Ok(rd) = std::fs::read_dir(&tests_dir) {
            let mut dirs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            dirs.sort();
            for d in dirs {
                let px = d.join("plugin.xml");
                if let Ok(t) = std::fs::read_to_string(&px) {
                    bundles.push(Bundle {
                        root: canonical(&d),
                        containers: source_containers(&t),
                    });
                }
            }
        } else {
            return Err(format!("{} missing", tests_dir.display()));
        }
        let classpath_roots = vec![base.join(TEST_BUNDLE), base.join(SAMPLES_PLUGIN)];
        let (blackboxes, case_bb_env) =
            read_blackboxes(&base.join("crates/qvto-types/data/blackboxes.tsv"))?;
        let mut norm_prefixes = Vec::new();
        for sub in ["reference/eclipse-qvto", "reference/eclipse-ocl", ""] {
            let p = if sub.is_empty() {
                base.clone()
            } else {
                base.join(sub)
            };
            if !p.exists() {
                continue;
            }
            let to = if sub.is_empty() {
                "${root}".to_string()
            } else {
                format!("${{root}}/{sub}")
            };
            let real = canonical(&p).to_string_lossy().into_owned();
            let abs = p.to_string_lossy().into_owned();
            if real != abs {
                norm_prefixes.push((real, to.clone()));
            }
            norm_prefixes.push((abs, to));
        }
        let data = Arc::new(ResolveData {
            norm_prefixes,
            base: base.clone(),
            bundles,
            classpath_roots,
            blackboxes,
            case_bb_env,
        });
        install_panic_hook();
        Ok(Harness {
            base,
            cases,
            oracle,
            notes,
            pcm_available,
            setup_time: t0.elapsed(),
            base_rs: rs,
            base_meta,
            base_ops,
            data,
            timeout: Duration::from_secs(120),
        })
    }

    /// Metamodel-file URIs (`*.ecore`, ...) the case's units use as model types that are not
    /// registry keys: `EmfStandaloneMetamodelProvider` loads them as resources.
    fn metamodel_file_uris(&self, case: &Case, main: &Path) -> Vec<String> {
        let found = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let resolver = self.data.case_resolver(case, main);
            let source = resolver.main_unit(main)?;
            Some(qvto_sema::scan_metamodel_uris(&source, &resolver))
        }));
        let Ok(Some(uris)) = found else {
            return Vec::new();
        };
        uris.into_iter()
            .filter(|u| {
                [".ecore", ".xcore", ".emof", ".oclinecore"]
                    .iter()
                    .any(|x| u.ends_with(x))
            })
            .filter(|u| self.base_meta.package_by_ns_uri(u).is_none())
            .collect()
    }

    /// The package registry for a case: the base one, plus the case's `mmfile`s, plus the
    /// metamodel files its units refer to by location.
    fn registry_for(
        &self,
        case: &Case,
        main: &Path,
    ) -> Result<(MetaRegistry, Vec<qvto_sema::MetaOperation>), String> {
        let files = self.metamodel_file_uris(case, main);
        if case.mmfiles.is_empty() && files.is_empty() {
            return Ok((self.base_meta.clone(), self.base_ops.clone()));
        }
        let mut rs = self.base_rs.clone();
        for (from, to) in &case.urimaps {
            rs.map_uri(from, &file_dir_uri(&normalize(&self.base.join(to))));
        }
        if !case.mmfiles.is_empty() {
            // TestQvtStandaloneExecutor maps the workspace URI of Ecore.ecore to Ecore's resource
            rs.add_alias(
                "platform:/resource/org.eclipse.emf.ecore/model/Ecore.ecore",
                ECORE_NS,
            );
        }
        for mm in &case.mmfiles {
            let p = case.path(&self.base, mm);
            let r = rs.load_file(&p).map_err(|e| format!("mmfile {mm}: {e}"))?;
            rs.register(r).map_err(|e| format!("mmfile {mm}: {e}"))?;
        }
        for uri in files {
            if rs.graph.meta().package_by_ns_uri(&uri).is_some() || rs.load_metamodel(&uri).is_err()
            {
                continue;
            }
            let Some(r) = rs.get_resource(&uri, false) else {
                continue;
            };
            // EmfUtil.getFirstEPackageContent
            let first = rs
                .graph
                .roots(r)
                .iter()
                .find_map(|o| rs.graph.links().obj_package.get(o).copied());
            if let Some(p) = first {
                rs.graph.meta_mut().register_alias(&uri, p);
            }
        }
        let ops = meta_operations(&rs.graph);
        Ok((rs.graph.meta().clone(), ops))
    }

    /// Runs one case: compiles it on its own thread (large stack, panics caught, time limit).
    pub fn run_case(&self, case: &Case) -> CaseRun {
        let key = case_key(case);
        let mut run = CaseRun {
            id: case.id.clone(),
            key: key.clone(),
            status: RunStatus::Ok,
            problems: Vec::new(),
            elapsed: Duration::ZERO,
            ir_issues: Vec::new(),
        };
        let origin = group(&key);
        if !self.pcm_available && (origin == "simulizar" || origin == "spd") {
            run.status = RunStatus::Setup(format!("{PCM_CONF} missing"));
            return run;
        }
        let Some(q) = &case.qvto else {
            run.status = RunStatus::Setup("no qvto".into());
            return run;
        };
        let main = normalize(&case.path(&self.base, q));
        if !main.is_file() {
            run.status = RunStatus::Setup(format!("missing {}", main.display()));
            return run;
        }
        let (meta, ops) = match self.registry_for(case, &main) {
            Ok(m) => m,
            Err(e) => {
                run.status = RunStatus::Setup(e);
                return run;
            }
        };
        let data = Arc::clone(&self.data);
        let case_c = case.clone();
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name(format!("oracle-case {}", case.id))
            .stack_size(512 << 20)
            .spawn(move || {
                let r = compile_case(&data, &case_c, &main, meta, &ops);
                let _ = tx.send(r);
            });
        if let Err(e) = spawned {
            run.status = RunStatus::Setup(format!("cannot spawn: {e}"));
            return run;
        }
        match rx.recv_timeout(self.timeout) {
            Ok(Ok((problems, elapsed, ir_issues))) => {
                run.ir_issues = ir_issues;
                run.problems = problems;
                run.elapsed = elapsed;
            }
            Ok(Err(p)) => run.status = RunStatus::Panic(p),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                run.status = RunStatus::Timeout;
                run.elapsed = self.timeout;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                run.status = RunStatus::Panic("worker died".into())
            }
        }
        run
    }

    pub fn oracle_for(&self, key: &str) -> Option<&OracleEntry> {
        self.oracle.get(key)
    }
}

/// Compiles a case and collects its problems (on the worker thread).
fn compile_case(
    data: &ResolveData,
    case: &Case,
    main: &Path,
    meta: MetaRegistry,
    ops: &[qvto_sema::MetaOperation],
) -> Result<(Vec<Prob>, Duration, Vec<String>), String> {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let resolver = data.case_resolver(case, main);
        let source = resolver.main_unit(main).expect("main unit readable");
        let t = Instant::now();
        let comp = qvto_sema::compile_with_operations(source, &resolver, meta, ops);
        let elapsed = t.elapsed();
        let ir_issues = if comp.has_errors() {
            Vec::new()
        } else {
            qvto_sema::irck::check_program(&comp.program)
        };
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for u in &comp.units {
            if !seen.insert(u.source.key.clone()) {
                continue;
            }
            let text = &u.source.text;
            for p in &u.problems {
                let (line, o, l) = p.eclipse_position(text, &u.lines);
                let line = if u.source.blackbox.is_some() { -1 } else { line };
                out.push(Prob {
                    unit: u.source.file_name.clone(),
                    sev: if p.severity == Severity::Error {
                        'E'
                    } else {
                        'W'
                    },
                    line,
                    offset: o as i64,
                    len: l as i64,
                    msg: data.norm(&one_line(&p.message)),
                });
            }
        }
        (out, elapsed, ir_issues)
    }));
    result.map_err(|_| {
        LAST_PANIC
            .with(|p| p.borrow_mut().take())
            .unwrap_or_else(|| "panic".to_string())
    })
}

pub use qvto_emf::operations::meta_operations;

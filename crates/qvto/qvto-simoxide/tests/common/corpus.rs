//! Runs the SimuLizar/SPD corpus cases (`qvto-testkit/cases/corpus.cases`) on two stores:
//! the adapter (PCM models loaded by SimOxide's loader into a `raw::Graph`, the other models
//! into the session) and `qvto_emf` (the conformance engine's setup, as `qvto conform`), with
//! the Java run's identity hashes when cached, and returns what the Java runner reports.

#![allow(dead_code)]

use qvto::blackbox::{BlackboxResolver, Catalog, CatalogUnit};
use qvto::units::{Bundle, FsResolver};
use qvto_emf::ResourceSet;
use qvto_emf::config::Config;
use qvto_model::{ModelStore, ResourceId};
use qvto_runtime::identity::IdentityHashSource;
use qvto_runtime::{IdentityKey, Mode};
use qvto_sema::ir::Direction;
use qvto_simoxide::{EXT_BASE, Journal, Session, SimStore};
use qvto_testkit::case::Case;
use qvto_testkit::fixture::{Tape, parse_tape};
use qvto_vm::ObjHandle;
use simoxide_model::raw;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub fn repo_root() -> PathBuf {
    qvto_paths::root()
}

/// Whether the corpus can run here (PCM metamodels extracted, SimuLizar/SPD metamodels found).
pub fn available() -> bool {
    repo_root().join("target/pcm/pcm.conf").exists()
}

pub fn cases() -> Vec<Case> {
    qvto_testkit::case::read_cases(&repo_root().join("qvto-testkit/cases/corpus.cases"))
        .expect("corpus.cases")
}

/// What a run gives, in the Java runner's terms.
#[derive(Debug, Default)]
pub struct Result {
    pub compile_errors: Vec<String>,
    pub severity: i32,
    pub code: i32,
    pub message: String,
    pub log: String,
    pub dump: String,
    pub compile_time: Duration,
    pub run_time: Duration,
    /// Adapter only.
    pub journal: Option<Journal>,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Backend {
    Adapter,
    Emf,
}

fn path(root: &Path, case: &Case, p: &str) -> PathBuf {
    case.path(root, p)
}

/// The blackbox units the case sees (as the conformance engine registers them).
fn case_blackboxes(catalog: &Catalog, case: &Case) -> (Vec<CatalogUnit>, Vec<CatalogUnit>) {
    let mut envs: Vec<&str> = Vec::new();
    if case.bundle_blackboxes {
        envs.push("bundle");
    }
    if let Some(e) = catalog.case_env.get(&case.id) {
        envs.push(e);
    }
    let mut registered = Vec::new();
    let mut classpath = Vec::new();
    for (i, e) in envs.iter().enumerate() {
        if i == 0 {
            registered.extend(catalog.units(e).iter().cloned());
        } else {
            classpath.extend(catalog.units(e).iter().cloned());
        }
    }
    classpath.extend(catalog.units("classpath").iter().cloned());
    (registered, classpath)
}

pub struct Units {
    pub files: FsResolver,
    pub catalog: Catalog,
}

pub fn units(root: &Path, case: &Case) -> Units {
    let catalog =
        Catalog::parse(&std::fs::read_to_string(root.join("qvto-types/data/blackboxes.tsv")).unwrap())
            .unwrap();
    let files = FsResolver {
        bundles: match &case.import_root {
            Some(r) => vec![Bundle {
                root: root.join(r).canonicalize().unwrap(),
                containers: vec![String::new()],
            }],
            None => FsResolver::bundles_below(&root.join("reference/eclipse-qvto/tests")),
        },
        from_folder: case.imports_from_folder,
    };
    Units { files, catalog }
}

pub fn resolver<'r>(u: &'r Units, case: &Case) -> BlackboxResolver<'r> {
    let (registered, classpath) = case_blackboxes(&u.catalog, case);
    BlackboxResolver {
        inner: &u.files,
        registered,
        classpath,
    }
}

fn config(root: &Path, case: &Case) -> Config {
    Config::read(&root.join(case.conf.as_deref().unwrap()), root).unwrap()
}

/// Keys of the store's objects as the tape names them (`<label>#<path>`).
pub fn object_keys<S: ModelStore>(s: &S) -> HashMap<String, u32>
where
    S::Obj: ObjHandle,
{
    let mut out = HashMap::new();
    for r in 0..s.resource_count() {
        let r = ResourceId(r as u32);
        let label = qvto_emf::canon::label(s.resource_uri(r));
        for o in s.resource_contents(r) {
            out.insert(format!("{label}#{}", qvto_emf::canon::path(s, o)), o.to_u32());
        }
    }
    out
}

struct Recovered(qvto_testkit::idhash::XorShift);

impl IdentityHashSource for Recovered {
    fn draw(&mut self, _key: IdentityKey) -> i32 {
        self.0.next()
    }
}

fn load_tape(root: &Path, case: &Case) -> Option<Tape> {
    let p = root
        .join(qvto_testkit::paths::JAVA_CACHE)
        .join("corpus")
        .join(&case.id)
        .join("idtape.tsv");
    parse_tape(&std::fs::read_to_string(p).ok()?).ok()
}

/// The Java run's identity hashes (as the conformance engine replays them): objects that exist
/// before the run by key, created objects in the Java thread's draw order.
fn id_source<S: ModelStore>(
    s: &S,
    tape: &Tape,
    existed: impl Fn(u32) -> bool + 'static,
) -> Box<dyn IdentityHashSource>
where
    S::Obj: ObjHandle,
{
    let keys = object_keys(s);
    let mut by_obj: HashMap<u32, i32> = HashMap::new();
    let mut others: Vec<i32> = Vec::new();
    for (k, h, assigned) in &tape.objects {
        match keys.get(k) {
            Some(&o) => {
                by_obj.insert(o, *h);
            }
            None => {
                if *assigned != Some(false) {
                    others.push(*h);
                }
            }
        }
    }
    let idx = qvto_testkit::idhash::sequence_index(tape.seed, &others, 20_000)
        .unwrap_or_else(|| vec![None; others.len()]);
    let mut ordered: Vec<(usize, i32)> = others
        .iter()
        .zip(idx)
        .map(|(h, i)| (i.unwrap_or(usize::MAX), *h))
        .collect();
    ordered.sort();
    let mut queue: std::collections::VecDeque<i32> = ordered.into_iter().map(|(_, h)| h).collect();
    let mut fallback = qvto_testkit::idhash::XorShift::recover(tape.seed.0, tape.seed.1);
    if let Some(x) = fallback.as_mut() {
        x.next();
        x.next();
    }
    let lookup = Box::new(move |k: IdentityKey| match k {
        IdentityKey::Obj(r) if !r.is_local() && existed(r.index()) => by_obj.get(&r.index()).copied(),
        IdentityKey::Obj(_) => queue.pop_front(),
        _ => None,
    });
    let fb: Box<dyn IdentityHashSource> = match fallback {
        Some(x) => Box::new(Recovered(x)),
        None => Box::new(qvto_runtime::identity::HotSpotXorShift::default()),
    };
    Box::new(qvto_vm::TapeHook::new(lookup, fb))
}

fn options(case: &Case, id_source: Option<Box<dyn IdentityHashSource>>) -> qvto::RunOptions {
    qvto::RunOptions {
        mode: Mode::Compat,
        id_source,
        max_stack_depth: case.max_stack_depth.map(|d| d as usize).unwrap_or(1000),
        readonly_guard: case.readonly_guard,
        config: case.config.clone(),
        line_numbers: !case.no_line_numbers,
        ..qvto::RunOptions::default()
    }
}

/// Positional inputs of the in/inout parameters.
fn bind<S: ModelStore>(
    program: &qvto::Program,
    s: &S,
    inputs: &[ResourceId],
) -> Vec<Vec<S::Obj>> {
    let ir = &program.ir;
    let main = &ir.modules[ir.main_module.index()];
    let mut args = Vec::new();
    let mut it = inputs.iter();
    for mp in &main.model_params {
        if mp.direction == Direction::Out {
            args.push(Vec::new());
            continue;
        }
        match it.next() {
            Some(r) => args.push(s.roots(*r).to_vec()),
            None => args.push(Vec::new()),
        }
    }
    args
}

/// Each out/inout extent into a resource `extent<i>.xmi`, then the dump (with the other model
/// resources for `dump models`), as the conformance engine does.
fn outputs<S: ModelStore>(s: &mut S, out: &qvto::RunOutcome, dump_models: bool) -> String
where
    S::Obj: ObjHandle,
{
    let before = s.resource_count();
    let mut extents = Vec::new();
    for (i, e) in out.extents.iter().enumerate() {
        let Some(roots) = e else { continue };
        let r = s.create_resource(&format!("file:/qvto-rs/out/extent{i}.xmi"));
        for o in roots {
            if !o.is_local() {
                let _ = s.add_root(r, S::Obj::from_u32(o.index()), None);
            }
        }
        extents.push(r);
    }
    let mut dumped = extents.clone();
    if dump_models {
        let mut v: Vec<(String, ResourceId)> = (0..before)
            .map(|r| ResourceId(r as u32))
            .filter_map(|r| {
                let uri = s.resource_uri(r);
                if !uri.starts_with("file:") || uri.ends_with(".ecore") {
                    return None;
                }
                Some((qvto_emf::canon::label(uri), r))
            })
            .collect();
        v.sort();
        dumped.extend(v.into_iter().map(|(_, r)| r));
    }
    qvto_emf::canon::dump(s, &dumped)
}

fn compile(
    root: &Path,
    case: &Case,
    meta: qvto_model::MetaRegistry,
    ops: &[qvto_model::MetaOperation],
    derivs: &[qvto_model::MetaDerivation],
    res: &mut Result,
) -> Option<qvto::Program> {
    let u = units(root, case);
    let r = resolver(&u, case);
    let main = u.files.unit_source(&path(root, case, case.qvto.as_deref().unwrap())).unwrap();
    let t0 = Instant::now();
    let compiled = qvto::compile_with_metamodel(main, &r, meta, ops, derivs);
    res.compile_time = t0.elapsed();
    for u in &compiled.units {
        for p in &u.problems {
            if p.is_error() {
                res.compile_errors.push(format!("{}: {}", u.source.file_name, p.message));
            }
        }
    }
    if compiled.has_errors() {
        return None;
    }
    compiled.program
}

/// The metamodel namespaces the case's units use.
fn metamodel_uris(root: &Path, case: &Case) -> Vec<String> {
    let u = units(root, case);
    let r = resolver(&u, case);
    let main = u.files.unit_source(&path(root, case, case.qvto.as_deref().unwrap())).unwrap();
    let mut v = qvto_sema::scan_metamodel_uris(&main, &r);
    v.extend(case.packages.iter().cloned());
    v
}

/// Whether a model document belongs in SimOxide's graph: all its namespaces are SimOxide's.
fn is_graph_document(text: &str) -> bool {
    let pre = qvto_emf::xmi_load::prescan(text);
    let mut any = false;
    for ns in &pre.ns_uris {
        if ns == "http://www.omg.org/XMI" || ns == "http://www.w3.org/2001/XMLSchema-instance" {
            continue;
        }
        any = true;
        if simoxide_model::meta::package_by_ns(ns).is_none() {
            return false;
        }
    }
    any
}

/// The documents an XMI file refers to (`href`s), as absolute paths.
fn referenced_files(file: &Path, text: &str) -> Vec<PathBuf> {
    let dir = file.parent().unwrap();
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(i) = rest.find("href=\"") {
        rest = &rest[i + 6..];
        let end = rest.find('"').unwrap_or(0);
        let href = &rest[..end];
        let doc = href.split('#').next().unwrap_or("");
        if !doc.is_empty() && !doc.contains(':') {
            let p = dir.join(doc);
            if let Ok(p) = p.canonicalize()
                && !out.contains(&p)
            {
                out.push(p);
            }
        }
        rest = &rest[end..];
    }
    out
}

/// Prepared adapter inputs: SimOxide's graph and a session with the other models.
pub struct AdapterSetup {
    pub graph: raw::Graph,
    pub session: Session,
    pub inputs: Vec<ResourceId>,
}

pub fn adapter_setup(root: &Path, case: &Case) -> AdapterSetup {
    let conf = config(root, case);
    let mut session = Session::new();
    session.configure(&conf);
    for uri in metamodel_uris(root, case) {
        let _ = session.load_metamodel(&uri);
    }
    // SimOxide's loader: graph documents among the inputs and those other models refer to
    let mut loader = simoxide_model::Loader::default();
    let mut kinds = Vec::new();
    for p in &case.inputs {
        let file = path(root, case, p).canonicalize().unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        if is_graph_document(&text) {
            let r = loader.load_file(&file);
            kinds.push((file, Some(r)));
        } else {
            for f in referenced_files(&file, &text) {
                if let Ok(t) = std::fs::read_to_string(&f)
                    && is_graph_document(&t)
                {
                    loader.load_file(&f);
                }
            }
            kinds.push((file, None));
        }
    }
    let (mut graph, _diags) = loader.finish();
    let mut inputs = Vec::new();
    {
        let mut store = session.store(&mut graph);
        for (file, r) in &kinds {
            inputs.push(match r {
                Some(r) => store.graph_resource(*r).unwrap(),
                None => store.load_model(file).unwrap(),
            });
        }
    }
    AdapterSetup {
        graph,
        session,
        inputs,
    }
}

pub fn run_adapter(root: &Path, case: &Case) -> Result {
    let mut res = Result::default();
    let mut setup = adapter_setup(root, case);
    let ops = setup.session.operations();
    let derivs = setup.session.derivations();
    let Some(program) = compile(root, case, setup.session.meta().clone(), &ops, &derivs, &mut res)
    else {
        return res;
    };
    setup.session.start_journal();
    let mut store = SimStore::new(&mut setup.graph, &mut setup.session);
    let (graph_objs, ext_objs) = (
        store.graph().objs.len() as u32,
        store.session().resource_set().graph.object_count() as u32,
    );
    let ids = load_tape(root, case).map(|t| {
        id_source(&store, &t, move |i| {
            i < graph_objs || (i >= EXT_BASE && i - EXT_BASE < ext_objs)
        })
    });
    let args = bind(&program, &store, &setup.inputs);
    let t1 = Instant::now();
    let (out, log) = {
        let mut vm = qvto::Vm::new(&program, &mut store, options(case, ids));
        let out = vm.run_transformation(args);
        (out, std::mem::take(&mut vm.log))
    };
    res.run_time = t1.elapsed();
    let journal = store.session().take_journal();
    res.dump = outputs(&mut store, &out, case.dump_models);
    res.severity = out.severity;
    res.code = out.code;
    res.message = out.message;
    res.log = log;
    res.journal = Some(journal);
    res
}

pub fn emf_setup(root: &Path, case: &Case) -> (ResourceSet, Vec<ResourceId>) {
    let conf = config(root, case);
    let mut rs = conf.resource_set_for(&path(root, case, case.qvto.as_deref().unwrap()));
    for uri in metamodel_uris(root, case) {
        if rs.graph.meta().package_by_ns_uri(&uri).is_some() {
            continue;
        }
        if let Some(loc) = rs.ns_location(&uri).map(String::from) {
            let _ = rs.load_metamodel(&loc);
        } else {
            let _ = rs.load_metamodel(&uri);
        }
    }
    rs.ecore_resource();
    let mut inputs = Vec::new();
    for p in &case.inputs {
        let file = std::path::absolute(path(root, case, p)).unwrap();
        let uri = qvto_emf::uri::Uri::file(&file.to_string_lossy()).to_string();
        let r = match rs.get_resource(&uri, false) {
            Some(r) => r,
            None => rs.load(&uri).unwrap(),
        };
        inputs.push(r);
    }
    rs.resolve_all();
    (rs, inputs)
}

pub fn run_emf(root: &Path, case: &Case) -> Result {
    let mut res = Result::default();
    let (mut rs, inputs) = emf_setup(root, case);
    let ops = qvto_emf::operations::meta_operations(&rs.graph);
    let derivs = qvto_emf::operations::meta_derivations(&rs.graph);
    let Some(program) = compile(root, case, rs.graph.meta().clone(), &ops, &derivs, &mut res) else {
        return res;
    };
    let first = rs.graph.object_count() as u32;
    let ids = load_tape(root, case).map(|t| id_source(&rs.graph, &t, move |i| i < first));
    let args: Vec<Vec<qvto_emf::ObjId>> = bind(&program, &rs.graph, &inputs);
    let t1 = Instant::now();
    let (out, log) = {
        let mut vm = qvto::Vm::new(&program, &mut rs.graph, options(case, ids));
        let out = vm.run_transformation(args);
        (out, std::mem::take(&mut vm.log))
    };
    res.run_time = t1.elapsed();
    rs.resolve_all();
    res.dump = outputs(&mut rs.graph, &out, case.dump_models);
    res.severity = out.severity;
    res.code = out.code;
    res.message = out.message;
    res.log = log;
    res
}

/// The Java runner's results for the corpus.
pub fn java() -> std::collections::BTreeMap<String, qvto_testkit::fixture::JavaResult> {
    let text = std::fs::read_to_string(repo_root().join(qvto_testkit::paths::CORPUS_FIXTURE)).unwrap();
    qvto_testkit::fixture::read_bundle(&text).unwrap()
}

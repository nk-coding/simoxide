//! The qvto-rs engine under the conformance harness (`qvto conform --engine qvto`): parse,
//! analyse, compile, bind the case's models, run, and report what the Java runner reports.

use qvto::blackbox::{BlackboxResolver, Catalog, CatalogUnit};
use qvto::units::{Bundle, FsResolver};
use qvto_emf::ResourceSet;
use qvto_model::{ModelStore, ResourceId};
use qvto_runtime::IdentityKey;
use qvto_runtime::Mode;
use qvto_runtime::identity::IdentityHashSource;
use qvto_sema::ir::Direction;
use qvto_testkit::engine::{CaseEnv, Engine, Outcome, Outputs, Severity, Status};
use qvto_testkit::fixture::{Problem, Tape, parse_tape};
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
use std::path::Path;
use std::time::Instant;

pub struct QvtoEngine {
    pub mode: Mode,
    bundles: Vec<Bundle>,
    /// The Java blackbox signatures of the corpus (`qvto-types/data/blackboxes.tsv`; parsed on
    /// the case's thread, the parsed units are not `Send`).
    signatures: std::sync::Arc<str>,
    /// Java result directories searched for a case's identity-hash tape (`<dir>/<id>/idtape.tsv`)
    /// besides the fixture caches of the Eclipse and corpus suites.
    pub tape_dirs: Vec<std::path::PathBuf>,
    /// Every identity hash this value instead of the tape (the Java side run with
    /// `-XX:hashCode=2`, which gives 1).
    pub constant_identity: Option<i32>,
}

/// The signatures of the Java blackboxes Eclipse loads for the suite and the corpus.
pub const BLACKBOX_SIGNATURES: &str = "qvto-types/data/blackboxes.tsv";

impl QvtoEngine {
    pub fn new(root: &Path, mode: Mode) -> QvtoEngine {
        let signatures =
            std::fs::read_to_string(root.join(BLACKBOX_SIGNATURES)).unwrap_or_else(|e| {
                eprintln!("qvto: {BLACKBOX_SIGNATURES}: {e}");
                String::new()
            });
        QvtoEngine {
            mode,
            bundles: FsResolver::bundles_below(&root.join("reference/eclipse-qvto/tests")),
            signatures: signatures.into(),
            tape_dirs: Vec::new(),
            constant_identity: None,
        }
    }
}

/// The blackbox units a case sees, as the Java runner registers them: the test bundle's
/// (`bundle-blackboxes`) or the case's own registrations first (they are also the candidates
/// for `blackbox` operations declared in QVTo), then Java classes by name.
fn case_blackboxes(
    catalog: &Catalog,
    case: &qvto_testkit::case::Case,
) -> (Vec<CatalogUnit>, Vec<CatalogUnit>) {
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

fn severity(s: i32) -> Severity {
    match s {
        0 => Severity::Ok,
        1 => Severity::Info,
        2 => Severity::Warning,
        8 => Severity::Cancel,
        _ => Severity::Error,
    }
}

impl Engine for QvtoEngine {
    fn name(&self) -> &str {
        match self.mode {
            Mode::Compat => "qvto",
            Mode::Fast => "qvto-fast",
        }
    }

    fn run(&mut self, env: &CaseEnv) -> Outcome {
        // a thread with a large stack: QVTo calls nest up to the stack depth limit (1000)
        let bundles = self.bundles.clone();
        let mode = self.mode;
        let signatures = self.signatures.clone();
        let tape_dirs = self.tape_dirs.clone();
        let constant = self.constant_identity;
        let r = std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(512 << 20)
                .spawn_scoped(s, || {
                    std::panic::catch_unwind(AssertUnwindSafe(|| {
                        run_case(env, bundles, &signatures, mode, &tape_dirs, constant)
                    }))
                })
                .expect("thread")
                .join()
        });
        if std::env::var_os("QVTO_SHOW").is_some()
            && let Ok(Ok(o)) = &r
        {
            eprintln!(
                "== {} status {:?} load {:?} severity {:?} code {} message {}\n-- log\n{}-- stack\n{}\n-- problems {:?}",
                env.case.id,
                o.status,
                o.load,
                o.severity,
                o.code,
                o.message,
                o.log,
                o.stack.as_deref().unwrap_or("-"),
                o.problems
                    .iter()
                    .map(|p| format!("{}:{} {}", p.unit, p.line, p.message))
                    .collect::<Vec<_>>()
            );
        }
        match r {
            Ok(Ok(o)) => o,
            Ok(Err(p)) | Err(p) => {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into());
                Outcome::with_status(Status::Crashed(format!("panic: {msg}")))
            }
        }
    }
}

/// A case's compiled program and the metamodels its units name (loaded in this order by every
/// run, so that the store's ids are the program's).
pub struct CaseProgram {
    pub compiled: qvto::Compiled,
    pub mm_uris: Vec<String>,
    pub compile_time: std::time::Duration,
}

/// Parses, analyses and compiles a case's transformation as the Java runner loads it.
#[allow(clippy::result_large_err)]
pub fn compile_case(
    env: &CaseEnv,
    bundles: Vec<Bundle>,
    signatures: &str,
) -> Result<CaseProgram, Outcome> {
    let catalog = match Catalog::parse(signatures) {
        Ok(c) => c,
        Err(e) => {
            return Err(Outcome::with_status(Status::Crashed(format!(
                "{BLACKBOX_SIGNATURES}: {e}"
            ))));
        }
    };
    let case = env.case;
    let Some(qvto) = env.transformation() else {
        return Err(Outcome::with_status(Status::Unsupported(
            "no transformation".into(),
        )));
    };
    let mut rs = match env.resource_set() {
        Ok(rs) => rs,
        Err(e) => {
            return Err(Outcome::with_status(Status::Crashed(format!(
                "resource set: {e}"
            ))));
        }
    };
    let resolver = FsResolver {
        bundles: match &case.import_root {
            // `imports root` is relative to the QVTo root (crates/qvto), not to the case's directory
            Some(r) => vec![Bundle {
                root: env
                    .base
                    .join(r)
                    .canonicalize()
                    .unwrap_or_else(|_| env.base.join(r)),
                containers: vec![String::new()],
            }],
            None => bundles,
        },
        from_folder: case.imports_from_folder,
    };
    let (registered, classpath) = case_blackboxes(&catalog, case);
    let file_resolver = resolver;
    let resolver = BlackboxResolver {
        inner: &file_resolver,
        registered,
        classpath,
    };
    let main = match file_resolver.unit_source(&qvto) {
        Ok(m) => m,
        Err(e) => {
            return Err(Outcome::with_status(Status::Crashed(format!(
                "{}: {e}",
                qvto.display()
            ))));
        }
    };
    // metamodels the units refer to, then the case's global packages (each run loads them in
    // the same order, so that the store's ids are the program's)
    let mut mm_uris = qvto_sema::scan_metamodel_uris(&main, &resolver);
    mm_uris.extend(case.packages.iter().cloned());
    for uri in &mm_uris {
        if let Err(e) = load_metamodel(&mut rs, uri)
            && std::env::var_os("QVTO_SHOW").is_some()
        {
            eprintln!("metamodel {uri}: {e}");
        }
    }
    // Ecore's own objects (EcorePackage is always there): `eClass()`, its EOperations
    rs.ecore_resource();
    let t0 = Instant::now();
    let operations = qvto_emf::operations::meta_operations(&rs.graph);
    let compiled = qvto::compile_with_metamodel(
        main,
        &resolver,
        rs.graph.meta().clone(),
        &operations,
        &qvto_emf::operations::meta_derivations(&rs.graph),
    );
    let compile_time = t0.elapsed();
    Ok(CaseProgram {
        compiled,
        mm_uris,
        compile_time,
    })
}

fn run_case(
    env: &CaseEnv,
    bundles: Vec<Bundle>,
    signatures: &str,
    mode: Mode,
    tape_dirs: &[std::path::PathBuf],
    constant: Option<i32>,
) -> Outcome {
    let case = env.case;
    let CaseProgram {
        compiled,
        mm_uris,
        compile_time,
    } = match compile_case(env, bundles, signatures) {
        Ok(p) => p,
        Err(o) => return o,
    };
    let mut o = Outcome::with_status(Status::Completed);
    o.compile_time = Some(compile_time);
    let mut warnings = false;
    for (ui, u) in compiled.units.iter().enumerate() {
        let src: &str = &u.source.text;
        for p in &u.problems {
            let (line, offset, length) = p.eclipse_position(src, &u.lines);
            // the executor's load diagnostic has the main unit's warnings only
            if !p.is_error() && ui == 0 {
                warnings = true;
            }
            o.problems.push(Problem {
                error: p.is_error(),
                unit: u.source.file_name.clone(),
                // Java units have no lines (Eclipse prints -1)
                line: if u.source.blackbox.is_some() {
                    -1
                } else {
                    line as i32
                },
                offset: offset as i32,
                length: length as i32,
                message: p.message.clone(),
            });
        }
    }
    o.load = Some(if compiled.has_errors() {
        Severity::Error
    } else if warnings {
        Severity::Warning
    } else {
        Severity::Ok
    });
    // the executor's checks of a loaded unit (`CstTransformation.doLoad`): a transformation
    // with an entry operation
    if !compiled.has_errors()
        && let Some(p) = &compiled.program
    {
        let m = &p.ir.modules[p.ir.main_module.index()];
        let executable = m.kind == qvto_sema::ir::ModuleKind::Transformation
            && (m.blackbox
                || m.operations
                    .iter()
                    .any(|&op| p.ir.operations[op.index()].name == "main"));
        if !executable {
            o.load = Some(Severity::Error);
            if m.kind == qvto_sema::ir::ModuleKind::Transformation && !case.compile_only() {
                // `InternalTransformationExecutor.execute` returns the load diagnostic
                o.severity = Some(Severity::Error);
                o.code = 130;
                o.message = format!(
                    "Transformation '{}' is has no entry point for execution",
                    m.name
                );
                return o;
            }
        }
    }
    if case.compile_only() {
        return o;
    }
    if compiled.has_errors() {
        o.status = Status::Unsupported("compile errors (sema)".into());
        return o;
    }
    let Some(program) = compiled.program else {
        o.status = Status::Crashed(format!(
            "bytecode: {}",
            compiled.bytecode_error.unwrap_or_default()
        ));
        return o;
    };
    let tape = if constant.is_some() {
        None
    } else {
        load_tape(env, tape_dirs)
    };
    match execute(env, &program, &mm_uris, mode, tape.as_ref(), constant) {
        Ok(mut o2) => {
            o2.compile_time = o.compile_time;
            o2.load = o.load;
            o2.problems = std::mem::take(&mut o.problems);
            o2
        }
        Err(o) => o,
    }
}

/// The Java run's identity-hash tape of the case, if cached.
fn load_tape(env: &CaseEnv, tape_dirs: &[std::path::PathBuf]) -> Option<Tape> {
    let cache = env.base.join(qvto_testkit::paths::JAVA_CACHE);
    let dirs = [
        cache.join("eclipse"),
        cache.join("corpus"),
        cache.join("fuzz"),
    ];
    for dir in dirs.iter().chain(tape_dirs) {
        let p = dir.join(&env.case.id).join("idtape.tsv");
        if let Ok(t) = std::fs::read_to_string(&p) {
            return parse_tape(&t).ok();
        }
    }
    None
}

/// Keys of the store's objects as the tape names them: `<label>#<path>` for model objects,
/// `mm:<nsURI>#<fragment>` for objects of metamodel resources.
fn object_keys(rs: &ResourceSet) -> HashMap<String, u32> {
    let g = &rs.graph;
    let mut out = HashMap::new();
    for r in g.resource_ids() {
        let uri = g.resource(r).uri.clone();
        let label = qvto_emf::canon::label(&uri);
        let ns = g.roots(r).iter().find_map(|&o| {
            let c = g.class_of(o);
            (g.meta().class(c).name.as_ref() == "EPackage")
                .then(|| {
                    let f = g.meta().feature_by_name(c, "nsURI")?;
                    g.get(o, f).as_str().map(String::from)
                })
                .flatten()
        });
        for o in g.resource_contents(r) {
            out.insert(format!("{label}#{}", qvto_emf::canon::path(g, o)), o.0);
            if let Some(ns) = &ns {
                out.insert(
                    format!("mm:{ns}#{}", qvto_emf::fragment::uri_fragment(g, r, o)),
                    o.0,
                );
            }
        }
    }
    out
}

/// The models of a run: a fresh resource set with the program's metamodels (in the compile
/// order) and the case's inputs (proxies resolved).
pub struct CaseModels {
    pub rs: ResourceSet,
    /// Per model parameter: the roots bound to it.
    pub args: Vec<Vec<qvto_emf::ObjId>>,
    /// Per model parameter: the input resource (in/inout parameters with a model).
    pub inputs: Vec<Option<ResourceId>>,
    /// The input trace of an incremental update.
    pub trace: Option<ResourceId>,
}

impl CaseModels {
    /// The parameters' roots as the input resources hold them now (a series of runs on the
    /// same models).
    pub fn current_args(&self) -> Vec<Vec<qvto_emf::ObjId>> {
        self.inputs
            .iter()
            .map(|r| {
                r.map(|r| self.rs.graph.roots(r).to_vec())
                    .unwrap_or_default()
            })
            .collect()
    }
}

/// Loads a case's models for one run of `program`.
#[allow(clippy::result_large_err)]
pub fn case_models(
    env: &CaseEnv,
    program: &qvto::Program,
    mm_uris: &[String],
) -> Result<CaseModels, Outcome> {
    let case = env.case;
    let mut rs = env
        .resource_set()
        .map_err(|e| Outcome::with_status(Status::Crashed(format!("resource set: {e}"))))?;
    let ir = &program.ir;
    for uri in mm_uris {
        let _ = load_metamodel(&mut rs, uri);
    }
    rs.ecore_resource();
    // model parameters: inputs bound positionally to the in/inout parameters
    let main_mod = &ir.modules[ir.main_module.index()];
    let mut args = Vec::new();
    let mut in_res = Vec::new();
    let mut inputs = case.inputs.iter();
    for mp in &main_mod.model_params {
        if mp.direction == Direction::Out {
            args.push(Vec::new());
            in_res.push(None);
            continue;
        }
        match inputs.next() {
            // `getResource(uri, true)`: a model already in the resource set (a registered
            // metamodel file) is that resource
            Some(p) => match get_or_load(&mut rs, &env.path(p)) {
                Ok(r) => {
                    args.push(rs.graph.roots(r).to_vec());
                    in_res.push(Some(r));
                }
                Err(e) => {
                    return Err(Outcome::with_status(Status::Crashed(format!(
                        "input {p}: {e}"
                    ))));
                }
            },
            None => {
                args.push(Vec::new());
                in_res.push(None);
            }
        }
    }
    // the input trace of an incremental update (its references to the previous run's inputs
    // and outputs resolve in this resource set)
    let trace_res = match &case.trace_in {
        Some(t) => match rs.load_file(&env.path(t)) {
            Ok(r) => Some(r),
            Err(e) => {
                return Err(Outcome::with_status(Status::Crashed(format!(
                    "trace {t}: {e}"
                ))));
            }
        },
        None => None,
    };
    rs.resolve_all();
    Ok(CaseModels {
        rs,
        args,
        inputs: in_res,
        trace: trace_res,
    })
}

/// One run of the compiled program on fresh models; returns the outcome and the tape hashes of
/// the created objects found in the outputs.
#[allow(clippy::result_large_err)] // the error is the outcome of a failed case, built once
fn execute(
    env: &CaseEnv,
    program: &qvto::Program,
    mm_uris: &[String],
    mode: Mode,
    tape: Option<&Tape>,
    constant: Option<i32>,
) -> Result<Outcome, Outcome> {
    let case = env.case;
    let CaseModels {
        mut rs,
        args,
        trace: trace_res,
        ..
    } = case_models(env, program, mm_uris)?;
    let inc_trace =
        trace_res.map(|r| qvto::trace::read_incremental_trace(&rs.graph, rs.graph.roots(r)));
    let first_created = rs.graph.object_count() as u32;
    let id_source: Option<Box<dyn IdentityHashSource>> = tape.map(|t| {
        // objects that exist before the run: their recorded hashes by key; objects the run
        // creates: the hashes the Java run assigned to the other objects of the tape, in the
        // order the thread drew them (both runs ask for the hashes in program order)
        let keys = object_keys(&rs);
        let mut by_obj: HashMap<u32, i32> = HashMap::new();
        let mut others: Vec<i32> = Vec::new();
        for (k, h, assigned) in &t.objects {
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
        let idx = qvto_testkit::idhash::sequence_index(t.seed, &others, 20_000)
            .unwrap_or_else(|| vec![None; others.len()]);
        let mut ordered: Vec<(usize, i32)> = others
            .iter()
            .zip(idx)
            .map(|(h, i)| (i.unwrap_or(usize::MAX), *h))
            .collect();
        ordered.sort();
        let mut queue: std::collections::VecDeque<i32> =
            ordered.into_iter().map(|(_, h)| h).collect();
        let mut fallback = qvto_testkit::idhash::XorShift::recover(t.seed.0, t.seed.1);
        if let Some(x) = fallback.as_mut() {
            // the seed's two draws are taken
            x.next();
            x.next();
        }
        if std::env::var("QVTO_TAPE_DEBUG").is_ok() {
            let mut ks: Vec<_> = keys.iter().filter(|(k, _)| k.starts_with("mm:")).collect();
            ks.sort();
            for (k, o) in ks.iter().take(400) {
                eprintln!("key {k} -> {o} {:?}", by_obj.get(o));
            }
        }
        let lookup = Box::new(move |k: IdentityKey| match k {
            IdentityKey::Obj(r) if !r.is_local() && r.index() < first_created => {
                by_obj.get(&r.index()).copied()
            }
            IdentityKey::Obj(_) => queue.pop_front(),
            _ => None,
        });
        let fb: Box<dyn IdentityHashSource> = match fallback {
            Some(x) => Box::new(Recovered(x)),
            None => Box::new(qvto_runtime::identity::HotSpotXorShift::default()),
        };
        Box::new(qvto_vm::TapeHook::new(lookup, fb)) as Box<dyn IdentityHashSource>
    });
    let id_source = match constant {
        Some(h) => {
            Some(Box::new(qvto_runtime::identity::Constant(h)) as Box<dyn IdentityHashSource>)
        }
        None => id_source,
    };
    let opts = qvto::RunOptions {
        mode,
        id_source,
        max_stack_depth: case.max_stack_depth.map(|d| d as usize).unwrap_or(1000),
        readonly_guard: case.readonly_guard,
        config: case.config.clone(),
        line_numbers: !case.no_line_numbers,
        // the case runs on a thread with a 512 MB stack: Eclipse's depth limit applies first
        max_native_stack: Some(256 << 20),
        ..qvto::RunOptions::default()
    };
    let t1 = Instant::now();
    let (out, log) = {
        let mut vm = qvto::Vm::new(program, &mut rs.graph, opts);
        vm.set_natives(std::rc::Rc::new(crate::bbox::natives()));
        if let Some(t) = inc_trace {
            vm.set_incremental_trace(t);
        }
        let out = vm.run_transformation(args);
        (out, std::mem::take(&mut vm.log))
    };
    let mut o = Outcome::with_status(Status::Completed);
    o.run_time = Some(t1.elapsed());
    o.log = log;
    o.severity = Some(severity(out.severity));
    o.code = out.code;
    o.message = out.message.clone();
    if let Some(e) = &out.error {
        o.exception = Some(e.java_class().to_string());
        o.stack = Some(e.print_stack_trace());
    }
    o.trace_records = Some(out.trace_records);
    let outputs = outputs(rs, &out, case.dump_models);
    if std::env::var_os("QVTO_SHOW").is_some() {
        eprintln!(
            "-- dump\n{}",
            qvto_emf::canon::dump(&outputs.rs.graph, &outputs.dumped())
        );
    }
    o.outputs = Some(outputs);
    Ok(o)
}

/// The Java thread's identity-hash sequence, continued after the tape's seed.
struct Recovered(qvto_testkit::idhash::XorShift);

impl IdentityHashSource for Recovered {
    fn draw(&mut self, _key: IdentityKey) -> i32 {
        self.0.next()
    }
}

/// Each out/inout extent's roots into a resource `extent<i>.xmi`, as the Java runner dumps
/// them.
fn outputs(mut rs: ResourceSet, out: &qvto::RunOutcome, dump_models: bool) -> Outputs {
    let mut extents: Vec<ResourceId> = Vec::new();
    for (i, e) in out.extents.iter().enumerate() {
        let Some(roots) = e else { continue };
        let r = rs
            .graph
            .create_resource(&format!("file:/qvto-rs/out/extent{i}.xmi"));
        for o in roots {
            if o.is_local() {
                continue;
            }
            let _ = rs.graph.add_root(r, qvto_emf::ObjId(o.index()), None);
        }
        extents.push(r);
    }
    rs.resolve_all();
    let models = if dump_models {
        Outputs::collect_models(&rs, &extents, &[])
    } else {
        Vec::new()
    };
    Outputs {
        rs,
        extents,
        models,
    }
}

/// Loads and registers the metamodel of a namespace URI (through the configuration's index of
/// metamodel files) or at a location.
pub fn load_metamodel(rs: &mut ResourceSet, uri: &str) -> Result<(), String> {
    if rs.graph.meta().package_by_ns_uri(uri).is_some() {
        return Ok(());
    }
    // registry keys of the Eclipse environment that the configuration has no file for
    if let Some((_, target)) = REGISTRY_ALIASES.iter().find(|(k, _)| *k == uri) {
        load_metamodel(rs, target)?;
        if let Some(p) = rs.graph.meta().package_by_ns_uri(target) {
            rs.graph.meta_mut().register_alias(uri, p);
        }
        return Ok(());
    }
    if let Some((_, file)) = EMF_JAR_MODELS.iter().find(|(k, _)| *k == uri)
        && rs.ns_location(uri).is_none()
    {
        let root = std::env::current_dir()
            .ok()
            .and_then(|d| qvto_testkit::find_root(&d))
            .unwrap_or_default();
        let loc = qvto_emf::uri::Uri::file(&root.join(file).to_string_lossy()).to_string();
        rs.add_ns_location(uri, &loc);
    }
    if let Some(loc) = rs.ns_location(uri).map(String::from) {
        rs.load_metamodel(&loc)?;
        // the global registry's descriptor: the package with that namespace in the file, also
        // when the file was loaded (and its packages registered) before as another file's
        // dependency (`Env.applyConf`'s `find`)
        if rs.graph.meta().package_by_ns_uri(uri).is_none()
            && let Some(r) = rs.get_resource(&loc, false)
        {
            let found = rs.graph.resource_contents(r).into_iter().find_map(|o| {
                let p = rs.graph.links().obj_package.get(&o).copied()?;
                (rs.graph.meta().package(p).ns_uri.as_deref() == Some(uri)).then_some(p)
            });
            if let Some(p) = found {
                rs.graph.meta_mut().register_alias(uri, p);
            }
        }
        return Ok(());
    }
    let packages = rs.load_metamodel(uri)?;
    // a metamodel by file location: the registry key is the location, bound to the file's
    // first package (`EmfUtil.getFirstEPackageContent`)
    if rs.graph.meta().package_by_ns_uri(uri).is_none()
        && let Some(r) = rs.get_resource(uri, false)
        && let Some(p) = rs
            .graph
            .roots(r)
            .iter()
            .find_map(|o| rs.graph.links().obj_package.get(o).copied())
            .or_else(|| packages.first().copied())
    {
        rs.graph.meta_mut().register_alias(uri, p);
    }
    Ok(())
}

/// Registry keys that are another key's package in the Eclipse environment
/// (`QVTOperationalPackage` is registered under both spellings).
const REGISTRY_ALIASES: &[(&str, &str)] = &[(
    "http://www.eclipse.org/QVT/1.0.0/Operational",
    "http://www.eclipse.org/qvt/1.0.0/Operational",
)];

use qvto_testkit::paths::EMF_JAR_MODELS;

/// The blackbox signatures `qvto run`/`qvto disasm` know: QVTo's own libraries and the Java
/// blackboxes of the corpus.
pub fn cli_catalog(root: &Path) -> Catalog {
    let mut c = qvto::blackbox::stdlib::catalog();
    if let Ok(t) = std::fs::read_to_string(root.join(BLACKBOX_SIGNATURES))
        && let Ok(x) = Catalog::parse(&t)
    {
        c.extend(&x);
    }
    c
}

/// A file resolver with QVTo's libraries registered and the test bundle's blackboxes and the
/// other Java classes of the corpus found by name.
pub fn cli_resolver<'r>(files: &'r FsResolver, catalog: &Catalog) -> BlackboxResolver<'r> {
    cli_resolver_dyn(files, catalog)
}

/// [`cli_resolver`] over any unit resolver.
pub fn cli_resolver_dyn<'r>(
    files: &'r dyn qvto_sema::UnitResolver,
    catalog: &Catalog,
) -> BlackboxResolver<'r> {
    let mut classpath: Vec<CatalogUnit> = catalog.units("bundle").to_vec();
    classpath.extend(catalog.units("classpath").iter().cloned());
    BlackboxResolver {
        inner: files,
        registered: catalog.units("stdlib").to_vec(),
        classpath,
    }
}

/// `ResourceSet.getResource(fileURI, true)`.
pub fn get_or_load(rs: &mut ResourceSet, path: &Path) -> Result<ResourceId, String> {
    let abs = std::path::absolute(path).map_err(|e| e.to_string())?;
    let uri = qvto_emf::uri::Uri::file(&abs.to_string_lossy()).to_string();
    match rs.get_resource(&uri, false) {
        Some(r) => Ok(r),
        None => rs.load(&uri),
    }
}

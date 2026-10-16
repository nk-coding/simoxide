//! The qvto-rs engine under the conformance harness (`qvto conform --engine qvto`): parse,
//! analyse, compile, bind the case's models, run, and report what the Java runner reports.

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
}

impl QvtoEngine {
    pub fn new(root: &Path, mode: Mode) -> QvtoEngine {
        QvtoEngine {
            mode,
            bundles: FsResolver::bundles_below(&root.join("reference/eclipse-qvto/tests")),
        }
    }
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
        let r = std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(512 << 20)
                .spawn_scoped(s, || {
                    std::panic::catch_unwind(AssertUnwindSafe(|| run_case(env, bundles, mode)))
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

fn run_case(env: &CaseEnv, bundles: Vec<Bundle>, mode: Mode) -> Outcome {
    let case = env.case;
    let Some(qvto) = env.transformation() else {
        return Outcome::with_status(Status::Unsupported("no transformation".into()));
    };
    let mut rs = match env.resource_set() {
        Ok(rs) => rs,
        Err(e) => return Outcome::with_status(Status::Crashed(format!("resource set: {e}"))),
    };
    let resolver = FsResolver {
        bundles: match &case.import_root {
            // `imports root` is relative to the repository root, not to the case's directory
            Some(r) => vec![Bundle {
                root: env.base.join(r).canonicalize().unwrap_or_else(|_| env.base.join(r)),
                containers: vec![String::new()],
            }],
            None => bundles,
        },
        from_folder: case.imports_from_folder,
    };
    let main = match resolver.unit_source(&qvto) {
        Ok(m) => m,
        Err(e) => return Outcome::with_status(Status::Crashed(format!("{}: {e}", qvto.display()))),
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
    let t0 = Instant::now();
    let compiled = qvto::compile(main, &resolver, rs.graph.meta().clone());
    let compile_time = t0.elapsed();
    let mut o = Outcome::with_status(Status::Completed);
    o.compile_time = Some(compile_time);
    let mut warnings = false;
    for (ui, u) in compiled.units.iter().enumerate() {
        let src: &str = &u.source.text;
        for p in &u.problems {
            let (offset, length) = p.utf16(src);
            // the executor's load diagnostic has the main unit's warnings only
            if !p.is_error() && ui == 0 {
                warnings = true;
            }
            o.problems.push(Problem {
                error: p.is_error(),
                unit: u.source.file_name.clone(),
                line: p.line(&u.lines) as i32,
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
    let tape = load_tape(env);
    match execute(env, &program, &mm_uris, mode, tape.as_ref()) {
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
fn load_tape(env: &CaseEnv) -> Option<Tape> {
    for suite in ["eclipse", "corpus"] {
        let p = env
            .base
            .join(qvto_testkit::paths::JAVA_CACHE)
            .join(suite)
            .join(&env.case.id)
            .join("idtape.tsv");
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

/// One run of the compiled program on fresh models; returns the outcome and the tape hashes of
/// the created objects found in the outputs.
fn execute(
    env: &CaseEnv,
    program: &qvto::Program,
    mm_uris: &[String],
    mode: Mode,
    tape: Option<&Tape>,
) -> Result<Outcome, Outcome> {
    let case = env.case;
    let mut rs = env
        .resource_set()
        .map_err(|e| Outcome::with_status(Status::Crashed(format!("resource set: {e}"))))?;
    let ir = &program.ir;
    for uri in mm_uris {
        let _ = load_metamodel(&mut rs, uri);
    }
    // model parameters: inputs bound positionally to the in/inout parameters
    let main_mod = &ir.modules[ir.main_module.index()];
    let mut args = Vec::new();
    let mut inputs = case.inputs.iter();
    for mp in &main_mod.model_params {
        if mp.direction == Direction::Out {
            args.push(Vec::new());
            continue;
        }
        match inputs.next() {
            Some(p) => match rs.load_file(&env.path(p)) {
                Ok(r) => args.push(rs.graph.roots(r).to_vec()),
                Err(e) => {
                    return Err(Outcome::with_status(Status::Crashed(format!(
                        "input {p}: {e}"
                    ))));
                }
            },
            None => args.push(Vec::new()),
        }
    }
    rs.resolve_all();
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
    let opts = qvto::RunOptions {
        mode,
        id_source,
        max_stack_depth: case.max_stack_depth.map(|d| d as usize).unwrap_or(1000),
        readonly_guard: case.readonly_guard,
        config: case.config.clone(),
        ..qvto::RunOptions::default()
    };
    let t1 = Instant::now();
    let (out, log) = {
        let mut vm = qvto::Vm::new(program, &mut rs.graph, opts);
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
    if let Some(loc) = rs.ns_location(uri).map(String::from) {
        return rs.load_metamodel(&loc).map(|_| ());
    }
    rs.load_metamodel(uri).map(|_| ())
}

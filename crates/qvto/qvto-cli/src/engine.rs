//! The qvto-rs engine under the conformance harness (`qvto conform --engine qvto`): parse,
//! analyse, compile, bind the case's models, run, and report what the Java runner reports.

use qvto::units::{Bundle, FsResolver};
use qvto_emf::ResourceSet;
use qvto_model::{ModelStore, ResourceId};
use qvto_runtime::Mode;
use qvto_sema::ir::Direction;
use qvto_testkit::engine::{CaseEnv, Engine, Outcome, Outputs, Severity, Status};
use qvto_testkit::fixture::Problem;
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
            Some(r) => vec![Bundle {
                root: env.path(r).canonicalize().unwrap_or_else(|_| env.path(r)),
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
    // metamodels the units refer to, then the case's global packages
    for uri in qvto_sema::scan_metamodel_uris(&main, &resolver) {
        let _ = rs.load_metamodel(&uri);
    }
    for p in &case.packages {
        let _ = rs.load_metamodel(p);
    }
    let t0 = Instant::now();
    let compiled = qvto::compile(main, &resolver, rs.graph.meta().clone());
    let compile_time = t0.elapsed();
    let mut o = Outcome::with_status(Status::Completed);
    o.compile_time = Some(compile_time);
    let mut warnings = false;
    for u in &compiled.units {
        let src: &str = &u.source.text;
        for p in &u.problems {
            let (offset, length) = p.utf16(src);
            if !p.is_error() {
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
    // model parameters: inputs bound positionally to the in/inout parameters
    let main_mod = &program.ir.modules[program.ir.main_module.index()];
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
                    return Outcome::with_status(Status::Crashed(format!("input {p}: {e}")));
                }
            },
            None => args.push(Vec::new()),
        }
    }
    rs.resolve_all();
    let opts = qvto::RunOptions {
        mode,
        max_stack_depth: case.max_stack_depth.map(|d| d as usize).unwrap_or(1000),
        readonly_guard: case.readonly_guard,
        config: case.config.clone(),
        ..qvto::RunOptions::default()
    };
    let t1 = Instant::now();
    let (out, log) = {
        let mut vm = qvto::Vm::new(&program, &mut rs.graph, opts);
        let out = vm.run_transformation(args);
        (out, std::mem::take(&mut vm.log))
    };
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
    o.outputs = Some(outputs(rs, &out, case.dump_models));
    o
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

//! The interpreter's flat code (`simoxide_sim::code`) in its two variants: a run with a trace
//! executes the trace instructions, a run without executes none. Both must give the same
//! results; and the interpreter levels it runs without a continuation of their own must still
//! count for `Limits::max_stack_depth`.

use simoxide_sim::{CompiledModel, Limits, Mode, Outputs, RunSpec, SimConfig, SimErrorKind};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Clone, Default)]
struct Buf(Rc<RefCell<Vec<u8>>>);

impl std::io::Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Model directories with a `run.json` below `rel`.
fn dirs(rel: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root().join(rel))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("run.json").is_file())
        .collect();
    v.sort();
    v
}

/// Measurements, event count, end time and main count of a run, or its error.
fn outcome(cm: &CompiledModel, cfg: SimConfig, trace: bool) -> String {
    let out = Outputs {
        trace: trace.then(|| Box::new(Buf::default()) as Box<dyn std::io::Write>),
        tape: None,
    };
    match simoxide_sim::run(cm, cfg, out) {
        Ok(r) => format!(
            "events {} end {} main {} rows {:?}",
            r.events, r.end_ns, r.main_count, r.measurements.rows
        ),
        Err(e) => format!("error {e:?}"),
    }
}

#[test]
fn runs_with_and_without_trace_agree() {
    let mut modes = vec![Mode::Exact];
    if cfg!(feature = "fast") {
        modes.push(Mode::Fast);
    }
    let mut n = 0;
    for rel in ["corpus", "corpus-fuzz", "crates/simoxide-sim/tests/models"] {
        for dir in dirs(rel) {
            let spec = RunSpec::load(&dir.join("run.json")).unwrap();
            let Ok(model) = spec.load_model(&dir) else {
                continue;
            };
            let Ok(cm) = CompiledModel::compile(model) else {
                continue;
            };
            for &mode in &modes {
                for (seed, meas) in [(spec.seed, spec.max_measurements), (3, 400)] {
                    let mut cfg = spec.sim_config("interp");
                    cfg.mode = mode;
                    cfg.seed = seed;
                    cfg.max_measurements = meas;
                    cfg.store_measurements = true;
                    let with = outcome(&cm, cfg.clone(), true);
                    let without = outcome(&cm, cfg, false);
                    assert!(
                        with == without,
                        "{} ({mode}, seed {seed}): runs with and without trace differ",
                        dir.display()
                    );
                    n += 1;
                }
            }
        }
    }
    assert!(n > 100, "only {n} runs");
}

/// Unbounded zero-time recursion (as in `robustness.rs`): the stack-depth limit fires at the
/// same point of the run as with one continuation per interpreter level of the reference
/// (trace line counts recorded before the flat code, which runs external calls, entry-level
/// system calls, the internal action of an infrastructure call and a SEFF's exit without a
/// continuation of their own).
#[test]
fn stack_depth_limit_fires_where_it_did() {
    let dir = root().join("corpus/h32_recursion");
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let mut files: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x != "json" && x != "txt"))
        .map(|p| {
            let n = p.file_name().unwrap().to_string_lossy().into_owned();
            (n, std::fs::read_to_string(&p).unwrap())
        })
        .collect();
    files.sort();
    let repo = files
        .iter_mut()
        .find(|f| f.0.ends_with(".repository"))
        .unwrap();
    for (from, to) in [
        (r#"specification="Exp(20.0)""#, r#"specification="0""#),
        (r#"branchProbability="0.4""#, r#"branchProbability="1.0""#),
        (r#"branchProbability="0.6""#, r#"branchProbability="0.0""#),
    ] {
        assert!(repo.1.contains(from));
        repo.1 = repo.1.replace(from, to);
    }
    let cm = CompiledModel::compile(spec.load_model_memory(&files).unwrap()).unwrap();
    for (depth, lines) in [(20, 48), (51, 108), (101, 204), (500, 1008)] {
        let buf = Buf::default();
        let mut cfg = spec.sim_config("depth");
        cfg.limits = Limits {
            max_stack_depth: depth,
            ..Limits::default()
        };
        let out = Outputs {
            trace: Some(Box::new(buf.clone())),
            tape: None,
        };
        let e = simoxide_sim::run(&cm, cfg, out).expect_err("stack limit");
        assert_eq!(e.kind, SimErrorKind::Limit, "{e}");
        let n = buf.0.borrow().iter().filter(|&&b| b == b'\n').count();
        assert_eq!(n, lines, "trace lines before the limit at depth {depth}");
    }
}

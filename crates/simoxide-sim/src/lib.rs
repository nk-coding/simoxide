//! Discrete-event simulator reproducing SimuLizar 5.2.2 event for event.
//!
//! ```no_run
//! use simoxide_sim::{CompiledModel, SimConfig, Outputs, Simulation};
//! let model = simoxide_model::load_dir("corpus/h01_ps_single").unwrap();
//! let cm = CompiledModel::compile(model).unwrap();
//! let cfg = SimConfig { seed: 1, max_measurements: 100, ..Default::default() };
//! let res = Simulation::new(&cm, cfg, Outputs::default()).unwrap().run().unwrap();
//! println!("{} measurements", res.measurements.count);
//! ```
//!
//! * [`ir`]: compilation of a loaded model into a [`CompiledModel`] (immutable, `Sync`).
//! * [`sim`]: the event core and interpreter ([`Simulation`]).
//! * [`run_batch`]: many independent runs on one compiled model, in parallel.
//! * [`compat`]: exact mode (default, byte-identical to the reference) and fast mode (cargo
//!   feature `fast`: same semantics and statistics, faster; `SimConfig::mode`, [`run`]).
//! * Output formats (trace, tape, measurements CSV) follow `docs/guide/formats.md`.
//!
//! Semantics: `docs/spec/simulation.md` (SIM-*), `workloads.md` (WL-*), `actions.md` (ACT-*),
//! `measurements.md` (MEAS-*).

pub mod code;
pub mod compat;
pub mod config;
mod events;
mod frames;
pub mod fxhash;
pub mod ir;
pub mod javafmt;
pub mod javahash;
pub mod meas;
pub mod rng;
pub mod sim;
mod trace;
pub mod windows;

#[cfg(feature = "fast")]
pub use compat::Fast;
pub use compat::{Compat, Exact, Literal, Mode};
pub use config::RunSpec;
pub use ir::{CompileError, CompiledModel};
pub use meas::{Measurements, SeriesSummary};
pub use rng::{RngMode, Tape};
pub use sim::{
    DEFAULT_MAX_EVENTS_PER_INSTANT, Limits, Outputs, RunResult, SimConfig, SimError, SimErrorKind,
    Simulation,
};
pub use simoxide_sched::PsAlgorithm;

/// Runs one simulation in the mode `cfg.mode` ([`Mode::Exact`] by default): the policy is
/// chosen here, once per run; the simulation itself is monomorphized per mode.
pub fn run(cm: &CompiledModel, cfg: SimConfig, out: Outputs) -> Result<RunResult, SimError> {
    match cfg.mode {
        Mode::Exact => Simulation::<Exact>::create(cm, cfg, out)?.run(),
        #[cfg(feature = "fast")]
        Mode::Fast => Simulation::<compat::Fast>::create(cm, cfg, out)?.run(),
        #[cfg(not(feature = "fast"))]
        m => Err(SimError {
            message: format!("mode {m} needs the cargo feature `fast` of simoxide-sim"),
            at_ns: 0,
            kind: SimErrorKind::Model,
        }),
    }
}

/// Why [`simulate_memory`] failed.
#[derive(Debug, Clone)]
pub enum RunError {
    /// The model files do not load (XML/XMI errors, unresolved references, invalid structure).
    Load(String),
    /// The model loads but cannot be simulated (unsupported or inconsistent construct).
    Compile(CompileError),
    /// The run aborted: see [`SimError::kind`] (model error, limit, cancellation).
    Sim(SimError),
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::Load(e) => write!(f, "load: {e}"),
            RunError::Compile(e) => write!(f, "compile: {e}"),
            RunError::Sim(e) => write!(f, "simulation: {e}"),
        }
    }
}

impl std::error::Error for RunError {}

/// One-call entry point for embedding: loads the model from memory (`files`: file name and XMI
/// content, see [`RunSpec::load_model_memory`]; the file system is never read), compiles it and
/// runs it once with `spec`'s seed and stop conditions under `limits`, in exact mode ([`simulate_memory_mode`]
/// chooses the mode). Measurements are stored
/// (`result.measurements.summaries()` gives per-series statistics, `rows` the raw series).
///
/// To run one model many times (seeds, replications), compile it once and use [`run_batch`]
/// or [`Simulation`] directly; a [`CompiledModel`] is `Send + Sync`.
pub fn simulate_memory<N: AsRef<str>, C: AsRef<[u8]>>(
    files: &[(N, C)],
    spec: &RunSpec,
    limits: Limits,
) -> Result<RunResult, RunError> {
    simulate_memory_mode(files, spec, limits, Mode::Exact)
}

/// [`simulate_memory`] in `mode`.
pub fn simulate_memory_mode<N: AsRef<str>, C: AsRef<[u8]>>(
    files: &[(N, C)],
    spec: &RunSpec,
    limits: Limits,
    mode: Mode,
) -> Result<RunResult, RunError> {
    let model = spec.load_model_memory(files).map_err(RunError::Load)?;
    let cm = CompiledModel::compile(model).map_err(RunError::Compile)?;
    let mut cfg = spec.sim_config("");
    cfg.limits = limits;
    cfg.mode = mode;
    run(&cm, cfg, Outputs::default()).map_err(RunError::Sim)
}

/// Runs `configs` on one compiled model with up to `threads` worker threads (0 = available
/// parallelism). Results are in input order. No trace or tape is written. Each run uses its
/// config's `mode`.
pub fn run_batch(
    cm: &CompiledModel,
    configs: &[SimConfig],
    threads: usize,
) -> Vec<Result<RunResult, SimError>> {
    let n = if threads == 0 {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    } else {
        threads
    }
    .max(1)
    .min(configs.len().max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut out: Vec<Option<Result<RunResult, SimError>>> =
        (0..configs.len()).map(|_| None).collect();
    let results = std::sync::Mutex::new(&mut out);
    std::thread::scope(|sc| {
        for _ in 0..n {
            sc.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if i >= configs.len() {
                        break;
                    }
                    let r = run(cm, configs[i].clone(), Outputs::default());
                    results.lock().expect("lock")[i] = Some(r);
                }
            });
        }
    });
    out.into_iter().map(|r| r.expect("run result")).collect()
}

/// The three output files of a run as text.
#[derive(Debug, Clone)]
pub struct TextOutputs {
    pub trace: String,
    pub tape: String,
    pub measurements: String,
    pub result: RunResult,
}

#[derive(Clone, Default)]
struct SharedBuf(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

impl std::io::Write for SharedBuf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl SharedBuf {
    fn take(&self) -> String {
        String::from_utf8(std::mem::take(&mut *self.0.borrow_mut())).unwrap_or_default()
    }
}

/// Loads the model of `dir` for `spec`, runs it with `rng` and returns trace, tape and
/// measurements as text (what `simoxide run --trace --tape --measurements` writes).
pub fn run_dir_to_text(
    dir: &std::path::Path,
    spec: &RunSpec,
    run_name: &str,
    rng: RngMode,
) -> Result<TextOutputs, String> {
    let model = spec.load_model(dir)?;
    let cm = CompiledModel::compile(model).map_err(|e| e.to_string())?;
    let mut cfg = spec.sim_config(run_name);
    cfg.rng = rng;
    let (trace, tape) = (SharedBuf::default(), SharedBuf::default());
    let out = Outputs {
        trace: Some(Box::new(trace.clone())),
        tape: Some(Box::new(tape.clone())),
    };
    let result = run(&cm, cfg, out).map_err(|e| e.to_string())?;
    Ok(TextOutputs {
        trace: trace.take(),
        tape: tape.take(),
        measurements: result.measurements.to_csv(),
        result,
    })
}

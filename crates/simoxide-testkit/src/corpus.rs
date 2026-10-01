//! Corpus harness: run a [`Simulator`] over `corpus/*` in tape-replay and/or own-RNG mode and compare
//! with `expected/` exactly.
//!
//! The future `simoxide-sim` integration test is a few lines:
//!
//! ```ignore
//! use simoxide_testkit::{corpus, sim::{FnSim, RunOutput}};
//! #[test]
//! fn corpus_exact() {
//!     let mut sim = FnSim::new("simoxide-sim", |req| {
//!         let o = simoxide_sim::run_dir(&req.model_dir, &req.config, &req.rng)?; // your API
//!         Ok(RunOutput { trace: Some(o.trace), tape: Some(o.tape), measurements: o.csv, ..Default::default() })
//!     });
//!     corpus::run_corpus(&mut sim, &corpus::HarnessOptions::from_env()).assert_all_pass();
//! }
//! ```
//!
//! `TESTKIT_FILTER=h0,x_pem` selects entries by substring, `TESTKIT_MODES=replay|own|both`,
//! `TESTKIT_DUMP=dir` writes the actual outputs of failing cases.

use std::fmt::{self, Write as _};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::diff::{DiffOptions, DiffReport, diff_trace_text};
use crate::meascmp::{MeasMode, MeasReport, compare_measurements};
use crate::measurements::Measurements;
use crate::runcfg::RunConfig;
use crate::sim::{RngMode, RunOutput, RunRequest, SimError, Simulator, workspace_root};
use crate::tape::{Tape, TapeMismatch, first_mismatch};

/// Default corpus location (`simoxide/corpus`).
pub fn corpus_dir() -> PathBuf {
    workspace_root().join("corpus")
}

/// Reads `path`, or `path.gz` transparently. `Ok(None)` if neither exists.
pub fn read_maybe_gz(path: &Path) -> std::io::Result<Option<String>> {
    if path.is_file() {
        return std::fs::read_to_string(path).map(Some);
    }
    let mut gz = path.as_os_str().to_owned();
    gz.push(".gz");
    let gz = PathBuf::from(gz);
    if gz.is_file() {
        let mut s = String::new();
        flate2::read::GzDecoder::new(std::fs::File::open(gz)?).read_to_string(&mut s)?;
        return Ok(Some(s));
    }
    Ok(None)
}

/// A corpus entry.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub dir: PathBuf,
    pub config: RunConfig,
}

impl Entry {
    pub fn expected(&self) -> std::io::Result<Expected> {
        let e = self.dir.join("expected");
        Ok(Expected {
            trace: read_maybe_gz(&e.join("trace.jsonl"))?,
            tape: read_maybe_gz(&e.join("tape.jsonl"))?,
            measurements: read_maybe_gz(&e.join("measurements.csv"))?.unwrap_or_default(),
        })
    }
    pub fn request(&self, rng: RngMode) -> RunRequest {
        RunRequest {
            name: self.name.clone(),
            model_dir: self.dir.clone(),
            config: self.config.clone(),
            rng,
            trace: true,
        }
    }
}

/// Expected outputs.
#[derive(Clone, Debug, Default)]
pub struct Expected {
    pub trace: Option<String>,
    pub tape: Option<String>,
    pub measurements: String,
}

/// Does `name` match a filter (comma-separated substrings; empty = all)?
pub fn matches_filter(name: &str, filter: Option<&str>) -> bool {
    match filter {
        None => true,
        Some(f) if f.trim().is_empty() => true,
        Some(f) => f
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .any(|s| name.contains(s)),
    }
}

/// Entries of a corpus directory (subdirectories with `run.json`), sorted by name.
pub fn list(corpus: &Path, filter: Option<&str>) -> std::io::Result<Vec<Entry>> {
    let mut v = Vec::new();
    for e in std::fs::read_dir(corpus)? {
        let p = e?.path();
        let rj = p.join("run.json");
        // entries whose reference run aborts (reference/import-case.sh) have no expected outputs
        if !rj.is_file() || p.join("REFERENCE-ERROR.txt").is_file() {
            continue;
        }
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        if !matches_filter(&name, filter) {
            continue;
        }
        let config = RunConfig::load(&rj).map_err(std::io::Error::other)?;
        v.push(Entry {
            name,
            dir: p,
            config,
        });
    }
    v.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(v)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    TapeReplay,
    OwnRng,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::TapeReplay => "replay",
            Mode::OwnRng => "own-rng",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HarnessOptions {
    pub corpus: PathBuf,
    pub filter: Option<String>,
    pub modes: Vec<Mode>,
    pub trace_diff: DiffOptions,
    pub measurements: MeasMode,
    /// Write actual outputs of failing cases to `<dir>/<name>.<mode>/`.
    pub dump_dir: Option<PathBuf>,
    /// Treat `Unimplemented` / `Unsupported` as failures in [`CorpusReport::assert_all_pass`].
    pub strict: bool,
    /// Entries per [`Simulator::run_batch`] call.
    pub batch: usize,
}

impl Default for HarnessOptions {
    fn default() -> Self {
        HarnessOptions {
            corpus: corpus_dir(),
            filter: None,
            modes: vec![Mode::TapeReplay, Mode::OwnRng],
            trace_diff: DiffOptions::exact(),
            measurements: MeasMode::Exact,
            dump_dir: None,
            strict: false,
            batch: 16,
        }
    }
}

impl HarnessOptions {
    /// Defaults overridden by `TESTKIT_FILTER`, `TESTKIT_MODES` (`replay`, `own`, `both`),
    /// `TESTKIT_DUMP`, `TESTKIT_STRICT=1`.
    pub fn from_env() -> Self {
        let mut o = Self::default();
        if let Ok(f) = std::env::var("TESTKIT_FILTER") {
            o.filter = Some(f);
        }
        match std::env::var("TESTKIT_MODES").as_deref() {
            Ok("replay") => o.modes = vec![Mode::TapeReplay],
            Ok("own") => o.modes = vec![Mode::OwnRng],
            _ => {}
        }
        if let Ok(d) = std::env::var("TESTKIT_DUMP") {
            o.dump_dir = Some(d.into());
        }
        o.strict = std::env::var("TESTKIT_STRICT").as_deref() == Ok("1");
        o
    }
}

/// Outcome of one (entry, mode) case.
#[derive(Clone, Debug)]
pub enum Outcome {
    Pass,
    /// The simulator has no implementation (yet) for this.
    Skipped(String),
    Fail(Box<Failure>),
}

/// What differed (the first failing artefact is the most informative: trace, then tape, then
/// measurements).
#[derive(Clone, Debug, Default)]
pub struct Failure {
    pub error: Option<SimError>,
    pub trace: Option<DiffReport>,
    pub tape: Option<TapeMismatch>,
    pub measurements: Option<MeasReport>,
    pub note: Option<String>,
}

impl Failure {
    pub fn summary(&self) -> String {
        if let Some(e) = &self.error {
            return e.to_string().lines().next().unwrap_or("").to_string();
        }
        if let Some(n) = &self.note {
            return n.clone();
        }
        if let Some(d) = self.trace.as_ref().and_then(|t| t.divergence.as_ref()) {
            return format!(
                "trace line {} ({:?}){}",
                d.expected_line,
                d.kind,
                d.fields
                    .first()
                    .map(|f| format!(" field {}", f.key))
                    .unwrap_or_default()
            );
        }
        if let Some(t) = &self.tape {
            return format!("tape: {t}");
        }
        if let Some(m) = &self.measurements {
            let first = m
                .failures()
                .next()
                .map(|s| format!("{} / {}: {}", s.mp, s.metric, s.detail))
                .or_else(|| m.missing_in_actual.first().map(|s| format!("missing {s}")))
                .or_else(|| m.extra_in_actual.first().map(|s| format!("extra {s}")))
                .unwrap_or_default();
            return format!("measurements: {first}");
        }
        "failed".into()
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(e) = &self.error {
            writeln!(f, "error: {e}")?;
        }
        if let Some(n) = &self.note {
            writeln!(f, "{n}")?;
        }
        if let Some(t) = &self.trace {
            write!(f, "{t}")?;
        }
        if let Some(t) = &self.tape {
            writeln!(f, "tape: {t}")?;
        }
        if let Some(m) = &self.measurements {
            write!(f, "{m}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct CaseResult {
    pub name: String,
    pub mode: Mode,
    pub outcome: Outcome,
    pub wall: Duration,
}

/// Compares one run's outputs with expected ones (exact trace/tape unless `trace_diff` says
/// otherwise; measurements per `meas`).
pub fn compare_outputs(
    expected: &Expected,
    actual: &RunOutput,
    trace_diff: &DiffOptions,
    meas: &MeasMode,
) -> Outcome {
    let mut f = Failure::default();
    let mut failed = false;
    if let (Some(e), Some(a)) = (&expected.trace, &actual.trace) {
        match diff_trace_text(e, a, trace_diff) {
            Ok(r) if r.is_equal() => {}
            Ok(r) => {
                f.trace = Some(r);
                failed = true;
            }
            Err(err) => {
                f.note = Some(format!("actual trace does not parse: {err}"));
                failed = true;
            }
        }
    } else if expected.trace.is_some() {
        f.note = Some("simulator produced no trace".into());
        failed = true;
    }
    if let (Some(e), Some(a)) = (&expected.tape, &actual.tape)
        && e != a
    {
        match (Tape::parse(e), Tape::parse(a)) {
            (Ok(te), Ok(ta)) => {
                let rel = match trace_diff.mode {
                    crate::diff::DiffMode::Exact => 0.0,
                    crate::diff::DiffMode::Tolerance(t)
                    | crate::diff::DiffMode::MeasurementsOnly(t)
                    | crate::diff::DiffMode::PerProcess(t) => t.rel,
                };
                if let Some(m) = first_mismatch(&te, &ta, rel) {
                    f.tape = Some(m);
                    failed = true;
                } else if matches!(trace_diff.mode, crate::diff::DiffMode::Exact) {
                    f.note
                        .get_or_insert_with(|| "tape text differs (formatting)".into());
                    failed = true;
                }
            }
            (_, Err(err)) => {
                f.note = Some(format!("actual tape does not parse: {err}"));
                failed = true;
            }
            (Err(err), _) => {
                f.note = Some(format!("expected tape does not parse: {err}"));
                failed = true;
            }
        }
    }
    if expected.measurements != actual.measurements {
        match (
            Measurements::parse(&expected.measurements),
            Measurements::parse(&actual.measurements),
        ) {
            (Ok(me), Ok(ma)) => {
                let r = compare_measurements(&me, &ma, meas);
                if !r.is_ok() {
                    f.measurements = Some(r);
                    failed = true;
                } else if matches!(meas, MeasMode::Exact) {
                    f.note
                        .get_or_insert_with(|| "measurements text differs (formatting)".into());
                    failed = true;
                }
            }
            (_, Err(e)) => {
                f.note = Some(format!("actual measurements do not parse: {e}"));
                failed = true;
            }
            (Err(e), _) => {
                f.note = Some(format!("expected measurements do not parse: {e}"));
                failed = true;
            }
        }
    }
    if failed {
        Outcome::Fail(Box::new(f))
    } else {
        Outcome::Pass
    }
}

/// Runs all selected entries in all selected modes.
pub fn run_corpus(sim: &mut dyn Simulator, opts: &HarnessOptions) -> CorpusReport {
    let entries = list(&opts.corpus, opts.filter.as_deref()).unwrap_or_else(|e| {
        panic!("cannot list corpus {}: {e}", opts.corpus.display());
    });
    let mut cases = Vec::new();
    // entries are processed in chunks through `run_batch` (one JVM per chunk for the reference;
    // bounded memory for the outputs)
    for chunk in entries.chunks(opts.batch.max(1)) {
        let mut todo: Vec<(&Entry, Arc<Expected>, Mode, RunRequest)> = Vec::new();
        for e in chunk {
            let expected = match e.expected() {
                Ok(x) => Arc::new(x),
                Err(err) => {
                    for &m in &opts.modes {
                        cases.push(CaseResult {
                            name: e.name.clone(),
                            mode: m,
                            outcome: Outcome::Fail(Box::new(Failure {
                                note: Some(format!("cannot read expected/: {err}")),
                                ..Default::default()
                            })),
                            wall: Duration::ZERO,
                        });
                    }
                    continue;
                }
            };
            for &m in &opts.modes {
                let rng = match m {
                    Mode::OwnRng => RngMode::OwnRng,
                    Mode::TapeReplay => match expected.tape.as_deref().map(Tape::parse) {
                        Some(Ok(t)) => RngMode::TapeReplay(Arc::new(t)),
                        _ => RngMode::TapeReplay(Arc::new(Tape::default())),
                    },
                };
                let req = e.request(rng);
                todo.push((e, expected.clone(), m, req));
            }
        }
        let reqs: Vec<RunRequest> = todo.iter().map(|t| t.3.clone()).collect();
        let results = sim.run_batch(&reqs);
        for ((e, expected, m, _), res) in todo.into_iter().zip(results) {
            let (outcome, wall) = match res {
                Ok(out) => {
                    let o = compare_outputs(&expected, &out, &opts.trace_diff, &opts.measurements);
                    if let (Outcome::Fail(_), Some(d)) = (&o, &opts.dump_dir) {
                        dump(d, &e.name, m, &out);
                    }
                    (o, out.wall)
                }
                Err(SimError::Unimplemented) => {
                    (Outcome::Skipped("unimplemented".into()), Duration::ZERO)
                }
                Err(SimError::Unsupported(s)) => (
                    Outcome::Skipped(format!("unsupported: {s}")),
                    Duration::ZERO,
                ),
                Err(err) => (
                    Outcome::Fail(Box::new(Failure {
                        error: Some(err),
                        ..Default::default()
                    })),
                    Duration::ZERO,
                ),
            };
            cases.push(CaseResult {
                name: e.name.clone(),
                mode: m,
                outcome,
                wall,
            });
        }
    }
    CorpusReport {
        simulator: sim.name().to_string(),
        cases,
        strict: opts.strict,
    }
}

fn dump(dir: &Path, name: &str, m: Mode, out: &RunOutput) {
    let d = dir.join(format!("{name}.{}", m.label()));
    let _ = std::fs::create_dir_all(&d);
    if let Some(t) = &out.trace {
        let _ = std::fs::write(d.join("trace.jsonl"), t);
    }
    if let Some(t) = &out.tape {
        let _ = std::fs::write(d.join("tape.jsonl"), t);
    }
    let _ = std::fs::write(d.join("measurements.csv"), &out.measurements);
}

/// All case results plus a summary table.
#[derive(Clone, Debug)]
pub struct CorpusReport {
    pub simulator: String,
    pub cases: Vec<CaseResult>,
    pub strict: bool,
}

impl CorpusReport {
    pub fn passed(&self) -> usize {
        self.cases
            .iter()
            .filter(|c| matches!(c.outcome, Outcome::Pass))
            .count()
    }
    pub fn skipped(&self) -> usize {
        self.cases
            .iter()
            .filter(|c| matches!(c.outcome, Outcome::Skipped(_)))
            .count()
    }
    pub fn failed(&self) -> usize {
        self.cases
            .iter()
            .filter(|c| matches!(c.outcome, Outcome::Fail(_)))
            .count()
    }
    /// One row per entry, one column per mode.
    pub fn table(&self) -> String {
        let mut modes: Vec<Mode> = Vec::new();
        for c in &self.cases {
            if !modes.contains(&c.mode) {
                modes.push(c.mode);
            }
        }
        let mut names: Vec<&str> = Vec::new();
        for c in &self.cases {
            if !names.contains(&c.name.as_str()) {
                names.push(&c.name);
            }
        }
        let w = names.iter().map(|n| n.len()).max().unwrap_or(5).max(5);
        let mut s = String::new();
        let _ = write!(s, "{:w$}", "model");
        for m in &modes {
            let _ = write!(s, "  {:<10}", m.label());
        }
        s.push_str("  detail\n");
        for n in names {
            let _ = write!(s, "{n:w$}");
            let mut detail = String::new();
            for m in &modes {
                let c = self.cases.iter().find(|c| c.name == n && c.mode == *m);
                let cell = match c.map(|c| &c.outcome) {
                    Some(Outcome::Pass) => "PASS".to_string(),
                    Some(Outcome::Skipped(_)) => "skip".to_string(),
                    Some(Outcome::Fail(f)) => {
                        if detail.is_empty() {
                            detail = format!("[{}] {}", m.label(), f.summary());
                        }
                        "FAIL".to_string()
                    }
                    None => "-".to_string(),
                };
                let _ = write!(s, "  {cell:<10}");
            }
            let _ = writeln!(s, "  {detail}");
        }
        let _ = writeln!(
            s,
            "{}: {} passed, {} failed, {} skipped of {} cases",
            self.simulator,
            self.passed(),
            self.failed(),
            self.skipped(),
            self.cases.len()
        );
        s
    }
    /// Panics with the table and the first failure's full report unless everything passed
    /// (skips are allowed unless `strict`).
    pub fn assert_all_pass(&self) {
        let bad = self.failed() + if self.strict { self.skipped() } else { 0 };
        if bad == 0 {
            eprintln!("{}", self.table());
            return;
        }
        let first = self.cases.iter().find_map(|c| match &c.outcome {
            Outcome::Fail(f) => Some(format!("{} [{}]:\n{f}", c.name, c.mode.label())),
            _ => None,
        });
        panic!(
            "{}\n{}",
            self.table(),
            first.unwrap_or_else(|| "skipped cases in strict mode".into())
        );
    }
}

impl fmt::Display for CorpusReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.table())
    }
}

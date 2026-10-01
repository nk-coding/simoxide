//! Fuzz driver: generate models, run the reference, run a candidate simulator, diff; save and
//! minimize divergences. Also the reference validation (pass rate, crash categories, determinism).

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::corpus::{Expected, Failure, Mode, Outcome, compare_outputs};
use crate::diff::DiffOptions;
use crate::meascmp::MeasMode;
use crate::modelgen::{self, FEATURE_NAMES, Features, GenConfig, xmi};
use crate::sim::{RefSim, RngMode, RunOutput, RunRequest, SimError, Simulator, workspace_root};
use crate::tape::Tape;

/// Fuzzing parameters.
#[derive(Clone, Debug)]
pub struct FuzzOptions {
    pub n: usize,
    pub start_seed: u64,
    /// Inclusive size range.
    pub sizes: (u32, u32),
    /// Models per reference JVM.
    pub batch: usize,
    pub features: Features,
    /// Model name prefix.
    pub prefix: String,
    /// Divergent cases are saved here (`corpus-fuzz/`).
    pub out_dir: PathBuf,
    pub work_dir: PathBuf,
    pub modes: Vec<Mode>,
    pub trace_diff: DiffOptions,
    pub measurements: MeasMode,
    pub minimize: bool,
    pub max_minimize_rounds: usize,
    pub verbose: bool,
    /// Save models the reference rejects but the candidate runs (`REPORT.reffail.txt`).
    pub save_ref_failures: bool,
}

impl Default for FuzzOptions {
    fn default() -> Self {
        FuzzOptions {
            n: 50,
            start_seed: 1,
            sizes: (1, 8),
            batch: 50,
            features: Features::default(),
            prefix: "fz".into(),
            out_dir: workspace_root().join("corpus-fuzz"),
            work_dir: std::env::temp_dir().join("testkit-fuzz"),
            modes: vec![Mode::TapeReplay, Mode::OwnRng],
            trace_diff: DiffOptions::exact(),
            measurements: MeasMode::Exact,
            minimize: true,
            max_minimize_rounds: 12,
            verbose: true,
            save_ref_failures: false,
        }
    }
}

/// The generator config of fuzz case `i` (size spread deterministically over the range).
pub fn config_for(o: &FuzzOptions, i: usize) -> GenConfig {
    let seed = o.start_seed + i as u64;
    let span = (o.sizes.1.max(o.sizes.0) - o.sizes.0 + 1) as u64;
    let mut r = modelgen::Rng::new(seed.wrapping_mul(0x2545_F491_4F6C_DD1D));
    let size = o.sizes.0 + (r.next_u64() % span) as u32;
    let mut c = GenConfig::new(format!("{}{seed}_s{size}", o.prefix), seed, size);
    c.features = o.features.clone();
    c
}

/// Writes a generated model to `work/<name>` and returns the run request.
pub fn materialize(cfg: &GenConfig, work: &Path) -> std::io::Result<RunRequest> {
    let m = modelgen::generate(cfg);
    let dir = work.join(&cfg.name);
    xmi::write_model(&m, &dir)?;
    Ok(RunRequest {
        name: cfg.name.clone(),
        model_dir: dir,
        config: m.run.clone(),
        rng: RngMode::OwnRng,
        trace: true,
    })
}

/// Normalized crash category of a failed reference run (exception class + message with ids and
/// numbers masked).
pub fn categorize(e: &SimError) -> String {
    let msg = match e {
        SimError::Failed(m) => {
            let first = m.lines().next().unwrap_or("");
            // the root cause is the most specific part
            match m.lines().rfind(|l| l.starts_with("Caused by:")) {
                Some(c) => format!("{first} <- {}", c.trim_start_matches("Caused by: ")),
                None => first.to_string(),
            }
        }
        other => return other.to_string(),
    };
    let msg = msg.strip_prefix("ERROR ").unwrap_or(&msg);
    let mut out = String::new();
    let mut chars = msg.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '_' && chars.peek().is_some_and(|n| n.is_ascii_alphanumeric()) {
            out.push_str("<id>");
            while chars
                .peek()
                .is_some_and(|n| n.is_ascii_alphanumeric() || *n == '_' || *n == '-')
            {
                chars.next();
            }
        } else if c.is_ascii_digit() {
            out.push('N');
            while chars
                .peek()
                .is_some_and(|n| n.is_ascii_digit() || *n == '.')
            {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out.chars().take(240).collect()
}

fn feature_diff(f: &Features) -> Vec<String> {
    let d = Features::default();
    FEATURE_NAMES
        .iter()
        .filter(|n| f.get(n) != d.get(n))
        .map(|n| format!("{n}={}", f.get(n).unwrap()))
        .collect()
}

fn write_maybe_gz(path: &Path, text: &str) -> std::io::Result<()> {
    if text.len() > 256 * 1024 {
        let mut p = path.as_os_str().to_owned();
        p.push(".gz");
        let f = std::fs::File::create(PathBuf::from(p))?;
        let mut enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
        enc.write_all(text.as_bytes())?;
        enc.finish()?;
        Ok(())
    } else {
        std::fs::write(path, text)
    }
}

/// Saves a model with its reference outputs as a corpus entry `<out>/<model name>/` (model
/// files, `run.json`, `expected/`, `gen.json`) plus `REPORT.<label>.txt`. The directory name equals
/// the run name, so the entry replays with the corpus harness (`HarnessOptions::corpus`).
pub fn save_case(
    out: &Path,
    cfg: &GenConfig,
    model_dir: &Path,
    reference: &RunOutput,
    label: &str,
    report: &str,
) -> std::io::Result<PathBuf> {
    let dest = out.join(&cfg.name);
    std::fs::create_dir_all(dest.join("expected"))?;
    for e in std::fs::read_dir(model_dir)? {
        let p = e?.path();
        if p.is_file() {
            std::fs::copy(&p, dest.join(p.file_name().unwrap()))?;
        }
    }
    if let Some(t) = &reference.trace {
        write_maybe_gz(&dest.join("expected/trace.jsonl"), t)?;
    }
    if let Some(t) = &reference.tape {
        write_maybe_gz(&dest.join("expected/tape.jsonl"), t)?;
    }
    write_maybe_gz(
        &dest.join("expected/measurements.csv"),
        &reference.measurements,
    )?;
    let genj = serde_json::json!({
        "name": cfg.name,
        "seed": cfg.seed,
        "size": cfg.size,
        "features_changed": feature_diff(&cfg.features),
    });
    std::fs::write(dest.join("gen.json"), format!("{genj:#}\n"))?;
    std::fs::write(dest.join(format!("REPORT.{label}.txt")), report)?;
    Ok(dest)
}

fn expected_of(o: &RunOutput) -> Expected {
    Expected {
        trace: o.trace.clone(),
        tape: o.tape.clone(),
        measurements: o.measurements.clone(),
    }
}

/// Runs the candidate on one reference result in one mode.
fn check(
    cand: &mut dyn Simulator,
    req: &RunRequest,
    reference: &RunOutput,
    mode: Mode,
    o: &FuzzOptions,
) -> Outcome {
    let mut r = req.clone();
    r.rng = match mode {
        Mode::OwnRng => RngMode::OwnRng,
        Mode::TapeReplay => RngMode::TapeReplay(Arc::new(
            reference
                .tape
                .as_deref()
                .and_then(|t| Tape::parse(t).ok())
                .unwrap_or_default(),
        )),
    };
    match cand.run(&r) {
        Ok(out) => compare_outputs(
            &expected_of(reference),
            &out,
            &o.trace_diff,
            &o.measurements,
        ),
        Err(SimError::Unimplemented) => Outcome::Skipped("unimplemented".into()),
        Err(SimError::Unsupported(s)) => Outcome::Skipped(format!("unsupported: {s}")),
        Err(e) => Outcome::Fail(Box::new(Failure {
            error: Some(e),
            ..Default::default()
        })),
    }
}

/// A divergence found by the fuzzer.
#[derive(Clone, Debug)]
pub struct Divergent {
    pub name: String,
    pub mode: Mode,
    pub summary: String,
    pub saved: Option<PathBuf>,
    pub minimized: Option<GenConfig>,
}

#[derive(Clone, Debug, Default)]
pub struct FuzzReport {
    pub total: usize,
    pub reference_ok: usize,
    pub passed: usize,
    pub skipped: usize,
    /// Reference failures by category -> model names.
    pub reference_failures: BTreeMap<String, Vec<String>>,
    pub divergences: Vec<Divergent>,
    /// Models the reference rejected: (candidate also failed, candidate ran to the end).
    pub reference_failed_candidate: (Vec<String>, Vec<String>),
    /// Coverage: generator features present per model, and reference trace events
    /// (`ev`, `begin:<type>`, `demand:<sched>`, StoEx functions of the tape) summed over all models.
    pub feature_counts: BTreeMap<String, usize>,
    pub event_counts: BTreeMap<String, u64>,
}

/// Adds the coverage of one reference trace and tape to `counts`.
pub fn count_events(trace: &str, tape: Option<&str>, counts: &mut BTreeMap<String, u64>) {
    let field = |l: &str, k: &str| -> Option<String> {
        let pat = format!("\"{k}\":\"");
        let i = l.find(&pat)? + pat.len();
        let j = l[i..].find('"')? + i;
        Some(l[i..j].to_string())
    };
    for l in trace.lines() {
        let Some(ev) = field(l, "ev") else { continue };
        let key = match ev.as_str() {
            "begin" => format!("begin:{}", field(l, "type").unwrap_or_default()),
            "demand" => format!(
                "demand:{}{}",
                field(l, "sched").unwrap_or_default(),
                if l.contains("\"st\":0.0,") || l.contains("\"st\":-") {
                    ":skipped"
                } else {
                    ""
                }
            ),
            "branch" => {
                if l.contains("\"idx\":-1") {
                    "branch:none".into()
                } else {
                    "branch".into()
                }
            }
            "meas" => format!(
                "meas:{}",
                field(l, "metric").unwrap_or_default().replace(' ', "_")
            ),
            e => e.to_string(),
        };
        *counts.entry(key).or_default() += 1;
    }
    if let Some(tape) = tape {
        for l in tape.lines() {
            if let Some(spec) = field(l, "spec") {
                let mut name = String::new();
                for (i, _) in spec.match_indices('(') {
                    let head = &spec[..i];
                    let start = head
                        .rfind(|c: char| !c.is_ascii_alphanumeric())
                        .map(|k| k + 1)
                        .unwrap_or(0);
                    let f = &head[start..];
                    if !f.is_empty() && f.chars().next().unwrap().is_ascii_uppercase() {
                        name = format!("stoex:{f}");
                        *counts.entry(name.clone()).or_default() += 1;
                    }
                }
                for lit in ["IntPMF", "DoublePMF", "DoublePDF", "BoolPMF", "EnumPMF"] {
                    if spec.contains(lit) {
                        *counts.entry(format!("stoex:{lit}")).or_default() += 1;
                    }
                }
                let _ = name;
            }
        }
    }
}

impl FuzzReport {
    /// The coverage counters as text (one `key count` per line).
    pub fn coverage_text(&self) -> String {
        let mut s = String::new();
        for (k, v) in &self.feature_counts {
            let _ = writeln!(s, "feature {k} {v}");
        }
        for (k, v) in &self.event_counts {
            let _ = writeln!(s, "event {k} {v}");
        }
        s
    }
}

impl fmt::Display for FuzzReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "{} models: reference ok {} ({:.1}%), candidate checks passed {}, skipped {}, divergent {}",
            self.total,
            self.reference_ok,
            100.0 * self.reference_ok as f64 / self.total.max(1) as f64,
            self.passed,
            self.skipped,
            self.divergences.len()
        )?;
        for (c, v) in &self.reference_failures {
            writeln!(f, "  reference failure x{}: {c}  (e.g. {})", v.len(), v[0])?;
        }
        let (both, only_ref) = &self.reference_failed_candidate;
        if !both.is_empty() || !only_ref.is_empty() {
            writeln!(
                f,
                "  reference failed: candidate failed too on {}, candidate ran on {}{}",
                both.len(),
                only_ref.len(),
                if only_ref.is_empty() {
                    String::new()
                } else {
                    format!(" (e.g. {})", only_ref[..only_ref.len().min(5)].join(", "))
                }
            )?;
        }
        for d in &self.divergences {
            writeln!(
                f,
                "  DIVERGENT {} [{}]: {}{}",
                d.name,
                d.mode.label(),
                d.summary,
                d.saved
                    .as_ref()
                    .map(|p| format!(" -> {}", p.display()))
                    .unwrap_or_default()
            )?;
        }
        Ok(())
    }
}

/// Generates `o.n` models, runs the reference and the candidate, reports and saves divergences.
pub fn run_fuzz(refsim: &mut RefSim, cand: &mut dyn Simulator, o: &FuzzOptions) -> FuzzReport {
    let mut rep = FuzzReport {
        total: o.n,
        ..Default::default()
    };
    let _ = std::fs::create_dir_all(&o.work_dir);
    let mut i = 0;
    while i < o.n {
        let k = o.batch.max(1).min(o.n - i);
        let cfgs: Vec<GenConfig> = (i..i + k).map(|j| config_for(o, j)).collect();
        i += k;
        let reqs: Vec<RunRequest> = cfgs
            .iter()
            .map(|c| {
                for f in modelgen::generate(c).features {
                    *rep.feature_counts.entry(f.to_string()).or_default() += 1;
                }
                materialize(c, &o.work_dir).expect("write model")
            })
            .collect();
        let refs = refsim.run_batch(&reqs);
        for ((cfg, req), r) in cfgs.iter().zip(&reqs).zip(refs) {
            let reference = match r {
                Ok(x) => x,
                Err(e) => {
                    rep.reference_failures
                        .entry(categorize(&e))
                        .or_default()
                        .push(cfg.name.clone());
                    if o.verbose {
                        eprintln!("[fuzz] {}: reference failed: {}", cfg.name, categorize(&e));
                    }
                    // the port should reject the model too (spec SIM-6.6)
                    match cand.run(req) {
                        Ok(_) => {
                            rep.reference_failed_candidate.1.push(cfg.name.clone());
                            if o.save_ref_failures {
                                let _ = save_case(
                                    &o.out_dir,
                                    cfg,
                                    &req.model_dir,
                                    &RunOutput::default(),
                                    "reffail",
                                    &format!("reference failed, candidate ran\n{e}"),
                                );
                            }
                        }
                        Err(_) => rep.reference_failed_candidate.0.push(cfg.name.clone()),
                    }
                    continue;
                }
            };
            rep.reference_ok += 1;
            if let Some(t) = &reference.trace {
                count_events(t, reference.tape.as_deref(), &mut rep.event_counts);
            }
            for &mode in &o.modes {
                match check(cand, req, &reference, mode, o) {
                    Outcome::Pass => rep.passed += 1,
                    Outcome::Skipped(_) => rep.skipped += 1,
                    Outcome::Fail(f) => {
                        let summary = f.summary();
                        if o.verbose {
                            eprintln!(
                                "[fuzz] {} [{}]: DIVERGENT {summary}",
                                cfg.name,
                                mode.label()
                            );
                        }
                        let mut report = format!(
                            "fuzz divergence ({} vs {}), mode {}\nconfig: seed {} size {} {:?}\n\n{f}",
                            cand.name(),
                            "refsim",
                            mode.label(),
                            cfg.seed,
                            cfg.size,
                            feature_diff(&cfg.features)
                        );
                        let minimized = if o.minimize {
                            minimize(refsim, cand, cfg, mode, o).map(|(mc, mreq, mref, mf)| {
                                let _ = save_case(
                                    &o.out_dir,
                                    &mc,
                                    &mreq.model_dir,
                                    &mref,
                                    mode.label(),
                                    &format!(
                                        "minimized from {} (seed {} size {} {:?}), mode {}\n\n{mf}",
                                        cfg.name,
                                        mc.seed,
                                        mc.size,
                                        feature_diff(&mc.features),
                                        mode.label()
                                    ),
                                );
                                let _ = std::fs::remove_dir_all(&mreq.model_dir);
                                mc
                            })
                        } else {
                            None
                        };
                        if let Some(mc) = &minimized {
                            report = format!("minimized repro: {}\n{report}", mc.name);
                        }
                        let saved = save_case(
                            &o.out_dir,
                            cfg,
                            &req.model_dir,
                            &reference,
                            mode.label(),
                            &report,
                        )
                        .ok();
                        rep.divergences.push(Divergent {
                            name: cfg.name.clone(),
                            mode,
                            summary,
                            saved,
                            minimized,
                        });
                    }
                }
            }
        }
        for r in &reqs {
            let _ = std::fs::remove_dir_all(&r.model_dir);
        }
        if o.verbose {
            eprintln!(
                "[fuzz] progress {i}/{}: reference ok {}, passed {}, divergent {}",
                o.n,
                rep.reference_ok,
                rep.passed,
                rep.divergences.len()
            );
        }
    }
    rep
}

/// Greedy delta reduction over the generator parameters: repeatedly try smaller sizes and
/// switching off single features; keep the variant with the shortest reference trace that still
/// diverges. Every round is one reference batch. Returns the smallest failing variant found (not
/// the original).
pub fn minimize(
    refsim: &mut RefSim,
    cand: &mut dyn Simulator,
    cfg: &GenConfig,
    mode: Mode,
    o: &FuzzOptions,
) -> Option<(GenConfig, RunRequest, RunOutput, Box<Failure>)> {
    let work = o.work_dir.join(format!("min-{}", cfg.name));
    let mut current = cfg.clone();
    let mut best: Option<(GenConfig, RunRequest, RunOutput, Box<Failure>)> = None;
    for round in 0..o.max_minimize_rounds {
        let mut variants: Vec<GenConfig> = Vec::new();
        let base = &cfg.name;
        let mut push = |mut c: GenConfig| {
            c.name = format!("{base}_m{round}_{}", variants.len());
            variants.push(c);
        };
        if current.size > 1 {
            let mut c = current.clone();
            c.size -= 1;
            push(c);
            if current.size > 3 {
                let mut c = current.clone();
                c.size /= 2;
                push(c);
            }
        }
        // ddmin-style: switch off halves and quarters of the enabled features, then single ones
        let on: Vec<&str> = FEATURE_NAMES
            .iter()
            .copied()
            .filter(|f| current.features.get(f).unwrap_or(0.0) > 0.0)
            .collect();
        for parts in [2usize, 4] {
            if on.len() >= 2 * parts {
                for chunk in on.chunks(on.len().div_ceil(parts)) {
                    let mut c = current.clone();
                    for f in chunk {
                        c.features.set(f, 0.0);
                    }
                    push(c);
                }
            }
        }
        for f in &on {
            let mut c = current.clone();
            c.features.set(f, 0.0);
            push(c);
        }
        if variants.is_empty() {
            break;
        }
        let reqs: Vec<RunRequest> = variants
            .iter()
            .filter_map(|c| materialize(c, &work).ok())
            .collect();
        let refs = refsim.run_batch(&reqs);
        let mut round_best: Option<(usize, GenConfig, RunRequest, RunOutput, Box<Failure>)> = None;
        for ((c, req), r) in variants.iter().zip(&reqs).zip(refs) {
            let Ok(reference) = r else { continue };
            if let Outcome::Fail(f) = check(cand, req, &reference, mode, o) {
                let score = reference
                    .trace
                    .as_ref()
                    .map(|t| t.len())
                    .unwrap_or(usize::MAX);
                if round_best.as_ref().is_none_or(|b| score < b.0) {
                    round_best = Some((score, c.clone(), req.clone(), reference, f));
                }
            }
        }
        match round_best {
            Some((_, c, req, r, f)) => {
                if o.verbose {
                    eprintln!(
                        "[fuzz] minimize {}: round {round}: size {} {:?}",
                        cfg.name,
                        c.size,
                        feature_diff(&c.features)
                    );
                }
                current = c.clone();
                // keep the model files of the best variant
                let keep = work.join(format!("best-{round}"));
                let _ = std::fs::remove_dir_all(&keep);
                let _ = copy_dir(&req.model_dir, &keep);
                let mut req = req;
                req.model_dir = keep;
                best = Some((c, req, r, f));
            }
            None => break,
        }
    }
    let out = best.map(|(c, req, r, f)| {
        // move the files out of the work dir before it is cleaned
        let final_dir = o.work_dir.join(format!("minimized-{}", cfg.name));
        let _ = std::fs::remove_dir_all(&final_dir);
        let _ = copy_dir(&req.model_dir, &final_dir);
        let mut req = req;
        req.model_dir = final_dir;
        (c, req, r, f)
    });
    let _ = std::fs::remove_dir_all(&work);
    out
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let p = e?.path();
        if p.is_file() {
            std::fs::copy(&p, to.join(p.file_name().unwrap()))?;
        }
    }
    Ok(())
}

/// Long runs without trace and tape: every request runs in the reference (`--no-trace`, batches of
/// `batch`) and in the candidate (own RNG); the measurements must be identical. Returns
/// `(name, outcome summary)` per request, `None` = identical.
pub fn run_long(
    refsim: &mut RefSim,
    cand: &mut dyn Simulator,
    reqs: &[RunRequest],
    batch: usize,
    verbose: bool,
) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    for chunk in reqs.chunks(batch.max(1)) {
        let chunk: Vec<RunRequest> = chunk
            .iter()
            .map(|r| RunRequest {
                trace: false,
                rng: RngMode::OwnRng,
                ..r.clone()
            })
            .collect();
        let refs = refsim.run_batch(&chunk);
        for (req, r) in chunk.iter().zip(refs) {
            let res = match r {
                Err(e) => Some(format!("reference failed: {}", categorize(&e))),
                Ok(reference) => match cand.run(req) {
                    Err(e) => Some(format!("candidate failed: {e}")),
                    Ok(o) => {
                        let exp = Expected {
                            trace: None,
                            tape: None,
                            measurements: reference.measurements.clone(),
                        };
                        let o = RunOutput {
                            trace: None,
                            tape: None,
                            ..o
                        };
                        match compare_outputs(&exp, &o, &DiffOptions::exact(), &MeasMode::Exact) {
                            Outcome::Pass => None,
                            Outcome::Fail(f) => Some(format!("DIVERGENT {}", f.summary())),
                            Outcome::Skipped(x) => Some(format!("skipped {x}")),
                        }
                    }
                },
            };
            if verbose {
                eprintln!(
                    "[long] {}: {}",
                    req.name,
                    res.as_deref().unwrap_or("identical")
                );
            }
            out.push((req.name.clone(), res));
        }
    }
    out
}

/// Result of [`validate_reference`].
#[derive(Clone, Debug, Default)]
pub struct Validation {
    pub total: usize,
    pub ok: usize,
    pub failures: BTreeMap<String, Vec<String>>,
    /// Models whose two runs (two JVMs) differed.
    pub nondeterministic: Vec<String>,
    pub wall_ms: Vec<(String, u128)>,
    pub coverage: BTreeMap<&'static str, usize>,
}

impl fmt::Display for Validation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut w: Vec<u128> = self.wall_ms.iter().map(|x| x.1).collect();
        w.sort();
        let pct = |p: f64| {
            w.get(((w.len() as f64 - 1.0) * p) as usize)
                .copied()
                .unwrap_or(0)
        };
        writeln!(
            f,
            "reference validation: {}/{} ok ({:.1}%), {} nondeterministic; run time ms p50 {} p90 {} max {}",
            self.ok,
            self.total,
            100.0 * self.ok as f64 / self.total.max(1) as f64,
            self.nondeterministic.len(),
            pct(0.5),
            pct(0.9),
            pct(1.0)
        )?;
        for (c, v) in &self.failures {
            writeln!(
                f,
                "  failure x{}: {c}  (e.g. {})",
                v.len(),
                v[..v.len().min(3)].join(", ")
            )?;
        }
        let mut slow = self.wall_ms.clone();
        slow.sort_by_key(|x| std::cmp::Reverse(x.1));
        let slow: Vec<String> = slow
            .iter()
            .take(3)
            .map(|(n, ms)| format!("{n} {ms} ms"))
            .collect();
        writeln!(f, "  slowest: {}", slow.join(", "))?;
        for n in &self.nondeterministic {
            writeln!(f, "  NONDETERMINISTIC: {n}")?;
        }
        let mut s = String::new();
        for (k, v) in &self.coverage {
            let _ = write!(s, "{k}={v} ");
        }
        writeln!(f, "  feature coverage (models): {s}")
    }
}

/// Generates models and runs each through the reference (`determinism`: twice, in two JVMs, and
/// compares the outputs byte for byte).
pub fn validate_reference(refsim: &mut RefSim, o: &FuzzOptions, determinism: bool) -> Validation {
    let mut v = Validation {
        total: o.n,
        ..Default::default()
    };
    let _ = std::fs::create_dir_all(&o.work_dir);
    let mut i = 0;
    while i < o.n {
        let k = o.batch.max(1).min(o.n - i);
        let cfgs: Vec<GenConfig> = (i..i + k).map(|j| config_for(o, j)).collect();
        i += k;
        let mut reqs = Vec::new();
        for c in &cfgs {
            let m = modelgen::generate(c);
            for f in &m.features {
                *v.coverage.entry(f).or_default() += 1;
            }
            reqs.push(materialize(c, &o.work_dir).expect("write model"));
        }
        let a = refsim.run_batch(&reqs);
        let b = if determinism {
            Some(refsim.run_batch(&reqs))
        } else {
            None
        };
        for (j, r) in a.into_iter().enumerate() {
            let name = cfgs[j].name.clone();
            match r {
                Ok(x) => {
                    v.ok += 1;
                    v.wall_ms.push((name.clone(), x.wall.as_millis()));
                    if let Some(b) = &b {
                        let same = match &b[j] {
                            Ok(y) => {
                                y.trace == x.trace
                                    && y.tape == x.tape
                                    && y.measurements == x.measurements
                            }
                            Err(_) => false,
                        };
                        if !same {
                            v.nondeterministic.push(name);
                        }
                    }
                }
                Err(e) => {
                    if o.verbose {
                        eprintln!("[validate] {name}: {e}");
                    }
                    v.failures.entry(categorize(&e)).or_default().push(name);
                }
            }
        }
        if o.verbose {
            eprintln!("[validate] {i}/{}: {} ok", o.n, v.ok);
        }
        for r in &reqs {
            let _ = std::fs::remove_dir_all(&r.model_dir);
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories() {
        let e = SimError::Failed(
            "ERROR java.lang.RuntimeException: First measurement to the same context _abc_12 arrived 3.5 times\nstack".into(),
        );
        assert_eq!(
            categorize(&e),
            "java.lang.RuntimeException: First measurement to the same context <id> arrived N times"
        );
        let e = SimError::Failed("ERROR java.lang.RuntimeException: aborted\njava.lang.RuntimeException: aborted\nCaused by: a.B: c\nCaused by: java.lang.NullPointerException: x".into());
        assert_eq!(
            categorize(&e),
            "java.lang.RuntimeException: aborted <- java.lang.NullPointerException: x"
        );
    }

    #[test]
    fn configs_are_reproducible() {
        let o = FuzzOptions::default();
        assert_eq!(config_for(&o, 3), config_for(&o, 3));
        let c = config_for(&o, 3);
        assert!(c.size >= 1 && c.size <= 8);
        assert_eq!(c.seed, 4);
    }
}

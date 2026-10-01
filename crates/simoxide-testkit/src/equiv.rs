//! Statistical equivalence of two simulator modes (by default the exact and the fast mode of
//! `simoxide-sim`) on the same models.
//!
//! The fast mode cannot be compared bit for bit, so the harness runs every model with many
//! independent seeds in both modes and tests, per measuring point and metric, whether the two
//! modes produce the same distribution of results:
//!
//! * per-run statistics (independent replications): tuple count, mean, p50, p90, p99; per model
//!   the finished usage-scenario runs (`main_count`, "requests") and the end time. Each is
//!   tested with Welch's t-test and the Mann-Whitney U test; the reported p-value is
//!   `min(1, 2·min(p_welch, p_mwu))`;
//! * the pooled values (a systematic sample of each run's tuples): the two-sample KS distance,
//!   with a permutation test over whole runs (valid although the tuples of one run are
//!   autocorrelated);
//! * the fraction of aborted runs (Fisher's exact test).
//!
//! State-like series (active and passive resource state, utilisation, number of containers) are
//! step functions of time. Their tuples include zero-length intermediate states at one instant,
//! which depend on the order of simultaneous events (which the fast mode may change). For them
//! the harness tests the time-weighted mean and the quantiles and KS distance of the value at
//! evenly spaced points in time (the queue-length distribution over time), not per-tuple
//! statistics.
//!
//! Values closer than rounding level (`1e-8 + 1e-9·|x|`: the clock has 1 ns resolution and
//! processor sharing may round differently) count as equal (ties) in every test, so that a
//! deterministic value shifted by a nanosecond is not "significant".
//!
//! All p-values of a campaign form one family: [`Report::finish`] applies Holm's correction, and
//! a test fails if its adjusted p-value is below `alpha`. The report also shows how the raw
//! p-values are distributed (under equivalence about 1 % of them are below 0.01).

use crate::modelgen::{self, GenConfig};
use crate::stats;
use simoxide_sim::{CompiledModel, Limits, Mode, Outputs, RunResult, RunSpec, SimConfig};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// Campaign settings.
#[derive(Clone, Debug)]
pub struct EquivOptions {
    /// The two modes compared (group A, group B).
    pub modes: [Mode; 2],
    /// Runs per mode (seeds `first_seed ..`).
    pub seeds: usize,
    pub first_seed: i64,
    /// Added to the seeds of group B (e.g. to compare a mode with itself on other seeds, an
    /// A/A test of the harness).
    pub seed_offset_b: i64,
    /// Raise a measurement stop condition to at least this many finished scenario runs
    /// (0: `run.json` as it is).
    pub min_measurements: i64,
    /// Worker threads (0: all cores).
    pub threads: usize,
    /// Values kept per run and series for the pooled KS test (systematic sample).
    pub sample_per_run: usize,
    /// Permutations of the KS test (its smallest p-value is `1 / (permutations + 1)`).
    pub permutations: usize,
    /// Family-wise error rate for the Holm correction.
    pub alpha: f64,
    /// Wall-clock limit per run.
    pub timeout: Duration,
}

impl Default for EquivOptions {
    fn default() -> Self {
        EquivOptions {
            modes: [Mode::Exact, Mode::Fast],
            seeds: 40,
            first_seed: 1000,
            seed_offset_b: 0,
            min_measurements: 0,
            threads: 0,
            sample_per_run: 128,
            permutations: 199,
            alpha: 0.01,
            timeout: Duration::from_secs(60),
        }
    }
}

/// A compiled model and its base run configuration.
pub struct Model {
    pub name: String,
    pub cm: CompiledModel,
    pub base: SimConfig,
    /// Generator features (generated models, or `FEATURES.txt` of a directory), for reports.
    pub features: String,
}

impl Model {
    /// A model directory with its `run.json`.
    pub fn from_dir(dir: &Path) -> Result<Model, String> {
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let spec = RunSpec::load(&dir.join("run.json"))?;
        let model = spec.load_model(dir)?;
        let cm = CompiledModel::compile(model).map_err(|e| e.to_string())?;
        let base = spec.sim_config(&name);
        let features = std::fs::read_to_string(dir.join("FEATURES.txt"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        Ok(Model {
            name,
            cm,
            base,
            features,
        })
    }

    /// A generated model (in memory).
    pub fn generated(cfg: &GenConfig) -> Result<Model, String> {
        let m = modelgen::generate(cfg);
        let files = modelgen::xmi::files(&m);
        let run_json = files
            .iter()
            .find(|f| f.0 == "run.json")
            .map(|f| f.1.clone())
            .ok_or("generated model without run.json")?;
        let spec = RunSpec::parse(&run_json)?;
        let xmi: Vec<(String, String)> = files
            .into_iter()
            .filter(|f| f.0 != "run.json" && f.0 != "FEATURES.txt")
            .collect();
        let model = spec.load_model_memory(&xmi)?;
        let cm = CompiledModel::compile(model).map_err(|e| e.to_string())?;
        let base = spec.sim_config(&cfg.name);
        Ok(Model {
            name: cfg.name.clone(),
            cm,
            base,
            features: m.features.join(", "),
        })
    }
}

/// Per-run statistics of one series.
#[derive(Clone, Debug, Default)]
pub struct SeriesRun {
    pub count: u64,
    pub mean: f64,
    pub p50: f64,
    pub p90: f64,
    pub p99: f64,
    /// Mean of the value as a step function between the first and the last tuple.
    pub twmean: Option<f64>,
    /// Systematic sample of the values (in time order; for state-like series: the value at
    /// evenly spaced points in time).
    pub sample: Vec<f64>,
    /// A step function of time (state, utilisation, container count): `mean`, `p50`.. are
    /// over time, not over tuples.
    pub step: bool,
}

/// Metrics whose tuples are the change points of a step function of time.
pub fn is_step_metric(metric: &str) -> bool {
    metric.starts_with("State of")
        || metric.starts_with("Utilization")
        || metric.starts_with("Number of Resource Containers")
}

/// Rounding-level tolerance: values closer than this count as equal.
#[inline]
pub fn tolerance(x: f64) -> f64 {
    1e-8 + 1e-9 * x.abs()
}

/// `x` lies within [`tolerance`] above `start` (false for NaN and infinite differences).
#[inline]
fn within(x: f64, start: f64) -> bool {
    matches!(
        (x - start).partial_cmp(&tolerance(start)),
        Some(std::cmp::Ordering::Less | std::cmp::Ordering::Equal)
    )
}

/// Replaces values within [`tolerance`] of each other (chains, in sorted order, measured from
/// the first value of the chain) by the first value of the chain, jointly over both groups.
pub fn snap_ties(a: &mut [f64], b: &mut [f64]) {
    let mut all: Vec<(f64, bool, usize)> = a
        .iter()
        .enumerate()
        .map(|(i, &x)| (x, true, i))
        .chain(b.iter().enumerate().map(|(i, &x)| (x, false, i)))
        .collect();
    all.sort_by(|x, y| x.0.total_cmp(&y.0));
    let mut start = f64::NAN;
    for (x, in_a, i) in all {
        if !within(x, start) {
            start = x;
        }
        if in_a {
            a[i] = start;
        } else {
            b[i] = start;
        }
    }
}

/// One run, summarised.
#[derive(Clone, Debug, Default)]
pub struct RunSummary {
    /// The run aborted (message).
    pub error: Option<String>,
    pub main_count: i64,
    pub end_s: f64,
    /// Keyed by `metric @ measuring point`.
    pub series: BTreeMap<String, SeriesRun>,
}

/// Summarises a run result (needs stored measurements).
pub fn summarize(r: &RunResult, sample_per_run: usize) -> RunSummary {
    let m = &r.measurements;
    let mut series = BTreeMap::new();
    for (i, rows) in m.rows.iter().enumerate() {
        if rows.is_empty() {
            continue;
        }
        let def = &m.series[i];
        let n = rows.len();
        let step = is_step_metric(def.metric);
        let (t0, t1) = (rows[0].0, rows[n - 1].0);
        let twmean = (t1 > t0).then(|| {
            rows.windows(2)
                .map(|w| w[0].1 * (w[1].0 - w[0].0))
                .sum::<f64>()
                / (t1 - t0)
        });
        let k = sample_per_run.max(1);
        let sample: Vec<f64> = if step {
            // the step function at k evenly spaced times (the last tuple at or before t)
            if t1 > t0 {
                let mut j = 0;
                (0..k)
                    .map(|q| {
                        let t = t0 + (q as f64 + 0.5) * (t1 - t0) / k as f64;
                        while j + 1 < n && rows[j + 1].0 <= t {
                            j += 1;
                        }
                        rows[j].1
                    })
                    .collect()
            } else {
                vec![rows[n - 1].1]
            }
        } else if n <= k {
            rows.iter().map(|r| r.1).collect()
        } else {
            (0..k)
                .map(|j| rows[((2 * j + 1) * n / (2 * k)).min(n - 1)].1)
                .collect()
        };
        // quantiles over the tuples, or over time for step functions
        let mut v: Vec<f64> = if step {
            sample.clone()
        } else {
            rows.iter().map(|r| r.1).collect()
        };
        let mean = if step {
            twmean.unwrap_or(rows[n - 1].1)
        } else {
            v.iter().sum::<f64>() / n as f64
        };
        v.sort_by(f64::total_cmp);
        let nv = v.len();
        let pct = |p: f64| v[((p * nv as f64).ceil() as usize).clamp(1, nv) - 1];
        series.insert(
            format!("{} @ {}", def.metric, def.mp),
            SeriesRun {
                count: n as u64,
                mean,
                p50: pct(0.5),
                p90: pct(0.9),
                p99: pct(0.99),
                twmean,
                sample,
                step,
            },
        );
    }
    RunSummary {
        error: None,
        main_count: r.main_count,
        end_s: r.end_ns as f64 / 1e9,
        series,
    }
}

/// The run configuration of seed index `i` in `mode`.
pub fn run_config(model: &Model, o: &EquivOptions, mode: Mode, i: usize) -> SimConfig {
    let mut c = model.base.clone();
    c.seed = o.first_seed + i as i64;
    c.mode = mode;
    c.store_measurements = true;
    if c.max_measurements > 0 && c.max_measurements < o.min_measurements {
        c.max_measurements = o.min_measurements;
    }
    c.limits = Limits {
        deadline: None,
        ..Limits::default()
    };
    c
}

/// Runs `o.seeds` seeds of the model in both modes (in parallel) and summarises every run.
pub fn run_modes(model: &Model, o: &EquivOptions) -> [Vec<RunSummary>; 2] {
    let jobs: Vec<(usize, usize)> = (0..2)
        .flat_map(|g| (0..o.seeds).map(move |i| (g, i)))
        .collect();
    let out: Mutex<Vec<Option<RunSummary>>> = Mutex::new(vec![None; jobs.len()]);
    let next = AtomicUsize::new(0);
    let threads = if o.threads == 0 {
        std::thread::available_parallelism().map_or(1, |n| n.get())
    } else {
        o.threads
    }
    .clamp(1, jobs.len().max(1));
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| {
                loop {
                    let j = next.fetch_add(1, Ordering::Relaxed);
                    if j >= jobs.len() {
                        break;
                    }
                    let (g, i) = jobs[j];
                    let mut cfg = run_config(model, o, o.modes[g], i);
                    if g == 1 {
                        cfg.seed += o.seed_offset_b;
                    }
                    cfg.limits.deadline = Some(Instant::now() + o.timeout);
                    let s = match simoxide_sim::run(&model.cm, cfg, Outputs::default()) {
                        Ok(r) => summarize(&r, o.sample_per_run),
                        Err(e) => RunSummary {
                            error: Some(e.message),
                            ..RunSummary::default()
                        },
                    };
                    out.lock().expect("lock")[j] = Some(s);
                }
            });
        }
    });
    let mut all = out
        .into_inner()
        .expect("lock")
        .into_iter()
        .map(|s| s.expect("run"));
    let a: Vec<RunSummary> = all.by_ref().take(o.seeds).collect();
    let b: Vec<RunSummary> = all.collect();
    [a, b]
}

/// One statistical test.
#[derive(Clone, Debug)]
pub struct Test {
    pub model: String,
    /// `metric @ measuring point`, or `(model)` for run-level statistics.
    pub series: String,
    /// `count`, `mean`, `p50`, `p90`, `p99`, `ks`, `requests`, `end_time`, `aborts` (for state-like
    /// series `mean` is the time-weighted mean and the quantiles are over time).
    pub stat: &'static str,
    /// Raw p-value.
    pub p: f64,
    /// Holm-adjusted p-value (set by [`Report::finish`]).
    pub p_adj: f64,
    /// Mean, standard deviation and number of the per-run values in group A and B (for `ks`:
    /// the KS distance in `a.0`).
    pub a: (f64, f64, usize),
    pub b: (f64, f64, usize),
    /// Both groups are constant (the statistic does not depend on the seed): any difference is
    /// systematic and reported separately (deterministic models: rounding and the order of
    /// simultaneous events, see `docs/correctness/deviations.md`).
    pub deterministic: bool,
}

fn msd(x: &[f64]) -> (f64, f64, usize) {
    (stats::mean(x), stats::variance(x).sqrt(), x.len())
}

/// Welch and Mann-Whitney combined (Bonferroni over the two), with rounding-level ties.
fn two_sample_p(a: &[f64], b: &[f64]) -> f64 {
    let (mut a, mut b) = (a.to_vec(), b.to_vec());
    snap_ties(&mut a, &mut b);
    let pw = stats::welch_t_test(&a, &b);
    let pm = stats::mann_whitney_u(&a, &b);
    match (pw.is_nan(), pm.is_nan()) {
        (true, true) => f64::NAN,
        (false, true) => pw,
        (true, false) => pm,
        (false, false) => (2.0 * pw.min(pm)).min(1.0),
    }
}

/// KS distance of the pooled samples of the runs in `a` and `b`, and its permutation p-value
/// (whole runs are permuted between the groups).
pub fn permutation_ks(a: &[&[f64]], b: &[&[f64]], permutations: usize, seed: u64) -> (f64, f64) {
    let runs: Vec<&[f64]> = a.iter().chain(b.iter()).copied().collect();
    let na = a.len();
    let mut pooled: Vec<(f64, u32)> = runs
        .iter()
        .enumerate()
        .flat_map(|(r, s)| s.iter().map(move |&x| (x, r as u32)))
        .collect();
    if na == 0 || b.is_empty() || pooled.is_empty() {
        return (f64::NAN, f64::NAN);
    }
    pooled.sort_by(|x, y| x.0.total_cmp(&y.0));
    // rounding-level ties: a block ends where the next value is beyond the tolerance of the
    // block's first value
    let mut block_end = vec![false; pooled.len()];
    let mut start = pooled[0].0;
    for i in 0..pooled.len() {
        if i + 1 == pooled.len() || !within(pooled[i + 1].0, start) {
            block_end[i] = true;
            if i + 1 < pooled.len() {
                start = pooled[i + 1].0;
            }
        }
    }
    let sizes: Vec<f64> = runs.iter().map(|s| s.len() as f64).collect();
    let d_of = |in_a: &[bool]| -> f64 {
        let ta: f64 = sizes.iter().zip(in_a).filter(|x| *x.1).map(|x| x.0).sum();
        let tb: f64 = sizes.iter().zip(in_a).filter(|x| !*x.1).map(|x| x.0).sum();
        if ta == 0.0 || tb == 0.0 {
            return 0.0;
        }
        let (mut ca, mut cb, mut d) = (0.0f64, 0.0f64, 0.0f64);
        let n = pooled.len();
        for i in 0..n {
            if in_a[pooled[i].1 as usize] {
                ca += 1.0;
            } else {
                cb += 1.0;
            }
            if block_end[i] {
                d = d.max((ca / ta - cb / tb).abs());
            }
        }
        d
    };
    let mut labels: Vec<bool> = (0..runs.len()).map(|r| r < na).collect();
    let d0 = d_of(&labels);
    let mut rng = modelgen::Rng::new(seed);
    let mut ge = 0usize;
    for _ in 0..permutations {
        // Fisher-Yates shuffle of the labels
        for i in (1..labels.len()).rev() {
            let j = rng.below(i + 1);
            labels.swap(i, j);
        }
        if d_of(&labels) >= d0 - 1e-12 {
            ge += 1;
        }
    }
    (d0, (1 + ge) as f64 / (1 + permutations) as f64)
}

fn key_seed(s: &str) -> u64 {
    // FNV-1a
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
    })
}

/// All tests of one model.
pub fn compare(model: &str, runs: &[Vec<RunSummary>; 2], o: &EquivOptions) -> Vec<Test> {
    let mut tests = Vec::new();
    let mut ks_tests = Vec::new();
    let mut push = |series: &str, stat: &'static str, x: &[f64], y: &[f64], p: f64| {
        if p.is_nan() {
            return;
        }
        let (mut xs, mut ys) = (x.to_vec(), y.to_vec());
        snap_ties(&mut xs, &mut ys);
        let constant = |v: &[f64]| v.windows(2).all(|w| w[0] == w[1]);
        tests.push(Test {
            model: model.to_string(),
            series: series.to_string(),
            stat,
            p,
            p_adj: f64::NAN,
            a: msd(x),
            b: msd(y),
            deterministic: constant(&xs) && constant(&ys),
        });
    };
    fn samples<'a>(s: &[Option<&'a SeriesRun>]) -> Vec<&'a [f64]> {
        s.iter().flatten().map(|x| &x.sample[..]).collect()
    }
    let errs = |g: usize| runs[g].iter().filter(|r| r.error.is_some()).count() as u64;
    let (ea, eb) = (errs(0), errs(1));
    let (na, nb) = (runs[0].len() as u64, runs[1].len() as u64);
    if ea + eb > 0 {
        let x: Vec<f64> = runs[0]
            .iter()
            .map(|r| f64::from(u8::from(r.error.is_some())))
            .collect();
        let y: Vec<f64> = runs[1]
            .iter()
            .map(|r| f64::from(u8::from(r.error.is_some())))
            .collect();
        push(
            "(model)",
            "aborts",
            &x,
            &y,
            stats::fisher_exact(ea, na - ea, eb, nb - eb),
        );
    }
    let ok: [Vec<&RunSummary>; 2] = [
        runs[0].iter().filter(|r| r.error.is_none()).collect(),
        runs[1].iter().filter(|r| r.error.is_none()).collect(),
    ];
    if ok[0].len() < 3 || ok[1].len() < 3 {
        return tests;
    }
    let col = |g: usize, f: &dyn Fn(&RunSummary) -> f64| -> Vec<f64> {
        ok[g].iter().map(|r| f(r)).collect()
    };
    let (x, y) = (
        col(0, &|r| r.main_count as f64),
        col(1, &|r| r.main_count as f64),
    );
    push("(model)", "requests", &x, &y, two_sample_p(&x, &y));
    let (x, y) = (col(0, &|r| r.end_s), col(1, &|r| r.end_s));
    push("(model)", "end_time", &x, &y, two_sample_p(&x, &y));
    let keys: std::collections::BTreeSet<&String> = ok
        .iter()
        .flat_map(|g| g.iter().flat_map(|r| r.series.keys()))
        .collect();
    for k in keys {
        let per = |g: usize| -> Vec<Option<&SeriesRun>> {
            ok[g].iter().map(|r| r.series.get(k.as_str())).collect()
        };
        let (sa, sb) = (per(0), per(1));
        let step = sa.iter().chain(sb.iter()).flatten().any(|x| x.step);
        if !step {
            // (the tuple count of a step function includes zero-length intermediate states)
            let counts = |s: &[Option<&SeriesRun>]| -> Vec<f64> {
                s.iter()
                    .map(|x| x.map_or(0.0, |x| x.count as f64))
                    .collect()
            };
            let (x, y) = (counts(&sa), counts(&sb));
            push(k, "count", &x, &y, two_sample_p(&x, &y));
        }
        let vals = |s: &[Option<&SeriesRun>], f: fn(&SeriesRun) -> Option<f64>| -> Vec<f64> {
            s.iter().flatten().filter_map(|x| f(x)).collect()
        };
        type Stat = (&'static str, fn(&SeriesRun) -> Option<f64>);
        let fs: [Stat; 4] = [
            // for step functions `mean` is the time-weighted mean and the quantiles are over
            // time
            ("mean", |s| Some(s.mean)),
            ("p50", |s| Some(s.p50)),
            ("p90", |s| Some(s.p90)),
            ("p99", |s| Some(s.p99)),
        ];
        for (stat, f) in fs {
            let (x, y) = (vals(&sa, f), vals(&sb, f));
            if x.len() >= 3 && y.len() >= 3 {
                push(k, stat, &x, &y, two_sample_p(&x, &y));
            }
        }
        let (xa, xb) = (samples(&sa), samples(&sb));
        let tot = |v: &[&[f64]]| v.iter().map(|s| s.len()).sum::<usize>();
        if xa.len() >= 3 && xb.len() >= 3 && tot(&xa) >= 20 && tot(&xb) >= 20 {
            let (d, p) = permutation_ks(&xa, &xb, o.permutations, key_seed(k));
            ks_tests.push(Test {
                model: model.to_string(),
                series: k.to_string(),
                stat: "ks",
                p,
                p_adj: f64::NAN,
                a: (d, 0.0, tot(&xa)),
                b: (f64::NAN, 0.0, tot(&xb)),
                deterministic: false,
            });
        }
    }
    tests.extend(ks_tests);
    tests
}

/// One model's line of the report.
#[derive(Clone, Debug)]
pub struct ModelLine {
    pub name: String,
    pub runs: [usize; 2],
    pub errors: [usize; 2],
    /// Mean finished scenario runs per run.
    pub requests: [f64; 2],
    pub tests: usize,
    pub min_p: f64,
    /// Seconds for all runs of the model.
    pub wall: f64,
    /// First abort message per mode.
    pub error_msgs: [Option<String>; 2],
    pub features: String,
}

/// A campaign: all tests of all models.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub modes: Vec<Mode>,
    pub models: Vec<ModelLine>,
    pub tests: Vec<Test>,
    /// Models that could not be loaded or compiled.
    pub skipped: Vec<(String, String)>,
    pub alpha: f64,
}

impl Report {
    /// Runs and compares one model.
    pub fn add_model(&mut self, model: &Model, o: &EquivOptions) {
        let t0 = Instant::now();
        let runs = run_modes(model, o);
        let tests = compare(&model.name, &runs, o);
        let line = ModelLine {
            name: model.name.clone(),
            runs: [runs[0].len(), runs[1].len()],
            errors: [
                runs[0].iter().filter(|r| r.error.is_some()).count(),
                runs[1].iter().filter(|r| r.error.is_some()).count(),
            ],
            requests: [0, 1].map(|g| {
                let ok: Vec<f64> = runs[g]
                    .iter()
                    .filter(|r| r.error.is_none())
                    .map(|r| r.main_count as f64)
                    .collect();
                stats::mean(&ok)
            }),
            tests: tests.len(),
            min_p: tests.iter().map(|t| t.p).fold(1.0, f64::min),
            wall: t0.elapsed().as_secs_f64(),
            error_msgs: [0, 1].map(|g| runs[g].iter().find_map(|r| r.error.clone())),
            features: model.features.clone(),
        };
        self.models.push(line);
        self.tests.extend(tests);
    }

    /// Applies Holm's correction over all tests.
    pub fn finish(&mut self, alpha: f64) {
        self.alpha = alpha;
        let p: Vec<f64> = self.tests.iter().map(|t| t.p).collect();
        for (t, a) in self.tests.iter_mut().zip(stats::holm(&p)) {
            t.p_adj = a;
        }
    }

    /// Tests whose adjusted p-value is below `alpha`, except deterministic differences.
    pub fn failures(&self) -> Vec<&Test> {
        self.tests
            .iter()
            .filter(|t| t.p_adj < self.alpha && !t.deterministic)
            .collect()
    }

    /// Statistics that are constant in both groups but differ.
    pub fn deterministic_differences(&self) -> Vec<&Test> {
        self.tests
            .iter()
            .filter(|t| t.deterministic && t.p < self.alpha)
            .collect()
    }

    /// Fraction of raw p-values below each threshold (expected: at most the threshold).
    pub fn calibration(&self) -> Vec<(f64, f64)> {
        let n = self.tests.len().max(1) as f64;
        [0.001, 0.01, 0.05, 0.1]
            .iter()
            .map(|&q| (q, self.tests.iter().filter(|t| t.p < q).count() as f64 / n))
            .collect()
    }

    /// All tests as CSV (`model,stat,series,p,p_adj,deterministic,mean_a,sd_a,n_a,mean_b,sd_b,n_b,features`).
    pub fn tests_csv(&self) -> String {
        let feats: BTreeMap<&str, &str> = self
            .models
            .iter()
            .map(|l| (l.name.as_str(), l.features.as_str()))
            .collect();
        let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        let mut s = String::from(
            "model,stat,series,p,p_adj,deterministic,mean_a,sd_a,n_a,mean_b,sd_b,n_b,features\n",
        );
        for t in &self.tests {
            let _ = writeln!(
                s,
                "{},{},{},{:e},{:e},{},{},{},{},{},{},{},{}",
                t.model,
                t.stat,
                q(&t.series),
                t.p,
                t.p_adj,
                t.deterministic,
                t.a.0,
                t.a.1,
                t.a.2,
                t.b.0,
                t.b.1,
                t.b.2,
                q(feats.get(t.model.as_str()).copied().unwrap_or(""))
            );
        }
        s
    }

    /// The report as text.
    pub fn text(&self, top: usize) -> String {
        let mut s = String::new();
        let m = |i: usize| self.modes.get(i).map_or("?", |m| m.name());
        let _ = writeln!(
            s,
            "| model | runs {a}/{b} | aborts {a}/{b} | requests/run {a} | {b} | tests | min p | s |",
            a = m(0),
            b = m(1)
        );
        let _ = writeln!(s, "|---|---|---|---|---|---|---|---|");
        for l in &self.models {
            let _ = writeln!(
                s,
                "| {} | {}/{} | {}/{} | {:.1} | {:.1} | {} | {:.2e} | {:.1} |",
                l.name,
                l.runs[0],
                l.runs[1],
                l.errors[0],
                l.errors[1],
                l.requests[0],
                l.requests[1],
                l.tests,
                l.min_p,
                l.wall
            );
        }
        for (n, e) in &self.skipped {
            let _ = writeln!(s, "skipped {n}: {e}");
        }
        let fails = self.failures();
        let det = self.deterministic_differences();
        let mut det_models: Vec<&str> = det.iter().map(|t| t.model.as_str()).collect();
        det_models.dedup();
        let _ = writeln!(
            s,
            "\n{} models, {} tests; Holm-corrected failures at alpha {}: {}; deterministic differences: {} in {} models {:?}",
            self.models.len(),
            self.tests.len(),
            self.alpha,
            fails.len(),
            det.len(),
            det_models.len(),
            det_models
        );
        let mut by_stat: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
        for t in &self.tests {
            let e = by_stat.entry(t.stat).or_default();
            e.0 += 1;
            if t.p < 0.01 {
                e.1 += 1;
            }
        }
        let _ = write!(s, "raw p-values below q (expected <= q):");
        for (q, f) in self.calibration() {
            let _ = write!(s, "  {q}: {:.4}", f);
        }
        let _ = write!(s, "\nper statistic (tests, raw p < 0.01):");
        for (k, (n, f)) in &by_stat {
            let _ = write!(s, "  {k} {n}/{f}");
        }
        let _ = writeln!(s);
        for t in &det {
            let _ = writeln!(
                s,
                "  deterministic: {} {} [{}]: A {:.9} B {:.9}",
                t.model, t.stat, t.series, t.a.0, t.b.0
            );
        }
        let mut sorted: Vec<&Test> = self.tests.iter().filter(|t| !t.deterministic).collect();
        sorted.sort_by(|a, b| a.p.total_cmp(&b.p));
        let _ = writeln!(
            s,
            "\nsmallest p-values of the statistical tests (A = {}, B = {}; mean ± sd (n) of the per-run values; ks: distance, pooled n):",
            m(0),
            m(1)
        );
        for t in sorted.iter().take(top) {
            let _ = writeln!(
                s,
                "  p={:.2e} adj={:.2e} {} {} [{}]: A {:.6} ± {:.3} ({}) B {:.6} ± {:.3} ({})",
                t.p, t.p_adj, t.model, t.stat, t.series, t.a.0, t.a.1, t.a.2, t.b.0, t.b.1, t.b.2
            );
        }
        for l in self.models.iter().filter(|l| l.errors != [0, 0]) {
            let _ = writeln!(
                s,
                "aborts in {}: A {:?} / B {:?}",
                l.name, l.error_msgs[0], l.error_msgs[1]
            );
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permutation_ks_separates_and_accepts() {
        let mut r = modelgen::Rng::new(5);
        let mk = |r: &mut modelgen::Rng, shift: f64| -> Vec<Vec<f64>> {
            (0..20)
                .map(|_| (0..50).map(|_| r.f64() + shift).collect())
                .collect()
        };
        let (a, b, c) = (mk(&mut r, 0.0), mk(&mut r, 0.0), mk(&mut r, 0.1));
        fn refs(v: &[Vec<f64>]) -> Vec<&[f64]> {
            v.iter().map(|x| &x[..]).collect()
        }
        let (_, p_same) = permutation_ks(&refs(&a), &refs(&b), 199, 1);
        let (d, p_diff) = permutation_ks(&refs(&a), &refs(&c), 199, 1);
        assert!(p_same > 0.01, "{p_same}");
        assert!(p_diff <= 0.005 && d > 0.05, "{p_diff} {d}");
    }
}

//! `simoxide-fuzz`: differential fuzzing of a PCM simulator against the Java reference.
//!
//! ```text
//! simoxide-fuzz fuzz     [--n 50] [--seed 1] [--sizes 1..8] [--batch 50] [--sim unimplemented|refsim|cmd:'prog args']
//!                   [--modes replay,own] [--compare exact|tol:REL|meas:REL|proc:REL] [--no-minimize]
//!                   [--out corpus-fuzz] [--disable f1,f2] [--prefix fz] [--set f1=p,f2=p] [--classic]
//!                   [--coverage FILE] [--save-ref-failures]
//! simoxide-fuzz validate [--n 300] [--seed 1] [--sizes 1..8] [--batch 100] [--no-determinism] [--disable ..]
//! simoxide-fuzz gen <dir> [--seed 1] [--size 5] [--disable f1,f2]
//! simoxide-fuzz diff <expected trace.jsonl[.gz]> <actual trace.jsonl> [--compare ...] [--context 8]
//! simoxide-fuzz corpus   [--sim ...] [--filter a,b] [--modes replay,own] [--compare ...] [--corpus DIR]
//! simoxide-fuzz long     [--sim ...] [--corpus DIR | --gen N --seed S --sizes a..b --set ..]
//!                   [--max-measurements 5000] [--max-sim-time T] [--batch 8]
//! simoxide-fuzz equiv    [--corpus DIR,DIR..] [--filter a,b] [--gen N --seed S --sizes a..b --set ..]
//!                   [--modes exact,fast] [--seeds 40] [--first-seed 1000] [--seed-offset-b 0]
//!                   [--min-measurements 0]
//!                   [--threads 0] [--alpha 0.01] [--permutations 199] [--top 40] [--timeout 60]
//!                   [--tests-csv FILE]
//! ```
//! `equiv` runs every model with many seeds in two modes of simoxide-sim (in process) and tests
//! per measuring point and metric that the results have the same distribution
//! (`simoxide_testkit::equiv`); exit code 1 if a Holm-corrected test fails.
//! `cmd:` simulators get the placeholders of `simoxide_testkit::sim::CmdSim` (`{dir} {run_json} {trace} {tape}
//! {measurements} {mode} {tape_in} {seed} {name}`); exit code 3 means "unsupported model".

use std::path::PathBuf;
use std::process::ExitCode;

use simoxide_testkit::corpus::{self, HarnessOptions, Mode};
use simoxide_testkit::diff::{DiffMode, DiffOptions, Tolerance};
use simoxide_testkit::fuzz::{self, FuzzOptions};
use simoxide_testkit::meascmp::MeasMode;
use simoxide_testkit::modelgen::{self, FEATURE_NAMES, Features, GenConfig};
use simoxide_testkit::sim::{CmdSim, RefSim, Simulator, Unimplemented};

struct Args {
    pos: Vec<String>,
    opts: Vec<(String, Option<String>)>,
}

const FLAGS: &[&str] = &[
    "--no-minimize",
    "--no-determinism",
    "--quiet",
    "--classic",
    "--save-ref-failures",
];

impl Args {
    fn parse(v: &[String]) -> Args {
        let mut pos = Vec::new();
        let mut opts = Vec::new();
        let mut i = 0;
        while i < v.len() {
            if v[i].starts_with("--") {
                if FLAGS.contains(&v[i].as_str()) || i + 1 >= v.len() {
                    opts.push((v[i].clone(), None));
                } else {
                    opts.push((v[i].clone(), Some(v[i + 1].clone())));
                    i += 1;
                }
            } else {
                pos.push(v[i].clone());
            }
            i += 1;
        }
        Args { pos, opts }
    }
    fn get(&self, k: &str) -> Option<&str> {
        self.opts
            .iter()
            .rev()
            .find(|(a, _)| a == k)
            .and_then(|(_, v)| v.as_deref())
    }
    fn has(&self, k: &str) -> bool {
        self.opts.iter().any(|(a, _)| a == k)
    }
    fn num<T: std::str::FromStr>(&self, k: &str, d: T) -> T {
        self.get(k).and_then(|s| s.parse().ok()).unwrap_or(d)
    }
}

fn features(a: &Args) -> Result<Features, String> {
    let mut f = if a.has("--classic") {
        Features::classic()
    } else {
        Features::default()
    };
    if let Some(list) = a.get("--disable") {
        for n in list.split(',').filter(|s| !s.is_empty()) {
            if !f.set(n, 0.0) {
                return Err(format!(
                    "unknown feature '{n}'; known: {}",
                    FEATURE_NAMES.join(", ")
                ));
            }
        }
    }
    if let Some(list) = a.get("--only") {
        f = Features::none();
        let d = Features::default();
        for n in list.split(',').filter(|s| !s.is_empty()) {
            let v = d.get(n).ok_or(format!("unknown feature '{n}'"))?;
            f.set(n, v);
        }
    }
    // --set name=p,name=p: feature probabilities (e.g. long_run=1,heavy_load=0.5)
    if let Some(list) = a.get("--set") {
        for kv in list.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = kv
                .split_once('=')
                .ok_or(format!("--set: expected name=p, got '{kv}'"))?;
            let v: f64 = v.parse().map_err(|_| format!("--set: bad value '{v}'"))?;
            if !f.set(k, v) {
                return Err(format!("unknown feature '{k}'"));
            }
        }
    }
    Ok(f)
}

fn compare(a: &Args) -> Result<(DiffOptions, MeasMode), String> {
    let mut d = DiffOptions::exact();
    d.context = a.num("--context", 8);
    let spec = a.get("--compare").unwrap_or("exact");
    let (kind, rel) = spec.split_once(':').unwrap_or((spec, "1e-9"));
    let rel: f64 = rel.parse().map_err(|_| format!("bad tolerance '{rel}'"))?;
    let tol = Tolerance { rel, abs: 0.0 };
    let meas = match kind {
        "exact" => MeasMode::Exact,
        "tol" => {
            d.mode = DiffMode::Tolerance(tol);
            MeasMode::Tolerance(tol)
        }
        "meas" => {
            d.mode = DiffMode::MeasurementsOnly(tol);
            MeasMode::Tolerance(tol)
        }
        "proc" => {
            d.mode = DiffMode::PerProcess(tol);
            MeasMode::Tolerance(tol)
        }
        _ => return Err(format!("unknown --compare '{spec}'")),
    };
    Ok((d, meas))
}

fn modes(a: &Args) -> Vec<Mode> {
    match a.get("--modes") {
        None => vec![Mode::TapeReplay, Mode::OwnRng],
        Some(s) => s
            .split(',')
            .filter_map(|m| match m {
                "replay" | "tape-replay" => Some(Mode::TapeReplay),
                "own" | "own-rng" => Some(Mode::OwnRng),
                _ => None,
            })
            .collect(),
    }
}

fn simulator(a: &Args) -> Result<Box<dyn Simulator>, String> {
    match a.get("--sim").unwrap_or("unimplemented") {
        "unimplemented" => Ok(Box::new(Unimplemented)),
        "refsim" => Ok(Box::new(RefSim::new())),
        s if s.starts_with("cmd:") => Ok(Box::new(CmdSim::parse(&s[4..])?)),
        s => Err(format!("unknown --sim '{s}'")),
    }
}

fn fuzz_options(a: &Args) -> Result<FuzzOptions, String> {
    let mut o = FuzzOptions {
        n: a.num("--n", 50),
        start_seed: a.num("--seed", 1),
        batch: a.num("--batch", 50),
        features: features(a)?,
        minimize: !a.has("--no-minimize"),
        save_ref_failures: a.has("--save-ref-failures"),
        modes: modes(a),
        verbose: !a.has("--quiet"),
        ..Default::default()
    };
    if let Some(s) = a.get("--sizes") {
        let (lo, hi) = s.split_once("..").unwrap_or((s, s));
        o.sizes = (
            lo.parse().map_err(|_| "bad --sizes")?,
            hi.parse().map_err(|_| "bad --sizes")?,
        );
    }
    if let Some(p) = a.get("--prefix") {
        o.prefix = p.to_string();
    }
    if let Some(d) = a.get("--out") {
        o.out_dir = PathBuf::from(d);
    }
    let (d, m) = compare(a)?;
    o.trace_diff = d;
    o.measurements = m;
    Ok(o)
}

fn run() -> Result<ExitCode, String> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = argv.first().cloned() else {
        return Err(
            "usage: simoxide-fuzz fuzz|validate|gen|diff|corpus ... (see --help in the source)"
                .into(),
        );
    };
    let a = Args::parse(&argv[1..]);
    match cmd.as_str() {
        "fuzz" => {
            let o = fuzz_options(&a)?;
            let mut cand = simulator(&a)?;
            let mut refsim = RefSim::new();
            let rep = fuzz::run_fuzz(&mut refsim, cand.as_mut(), &o);
            print!("{rep}");
            if let Some(p) = a.get("--coverage") {
                std::fs::write(p, rep.coverage_text()).map_err(|e| e.to_string())?;
            }
            Ok(if rep.divergences.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "validate" => {
            let mut o = fuzz_options(&a)?;
            o.n = a.num("--n", 300);
            o.batch = a.num("--batch", 100);
            let mut refsim = RefSim::new();
            let v = fuzz::validate_reference(&mut refsim, &o, !a.has("--no-determinism"));
            print!("{v}");
            let ok = v.ok as f64 >= 0.99 * v.total as f64 && v.nondeterministic.is_empty();
            Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "gen" => {
            let dir = PathBuf::from(a.pos.first().ok_or("gen <dir>")?);
            let name = dir
                .file_name()
                .ok_or("bad dir")?
                .to_string_lossy()
                .into_owned();
            let mut c = GenConfig::new(name, a.num("--seed", 1), a.num("--size", 5));
            c.features = features(&a)?;
            let m = modelgen::generate(&c);
            modelgen::xmi::write_model(&m, &dir).map_err(|e| e.to_string())?;
            println!("{}: {}", dir.display(), m.features.join(", "));
            Ok(ExitCode::SUCCESS)
        }
        "diff" => {
            let (e, x) = (
                a.pos.first().ok_or("diff <expected> <actual>")?,
                a.pos.get(1).ok_or("diff <expected> <actual>")?,
            );
            let read = |p: &str| -> Result<String, String> {
                let p = PathBuf::from(p.strip_suffix(".gz").unwrap_or(p));
                corpus::read_maybe_gz(&p)
                    .map_err(|e| e.to_string())?
                    .ok_or(format!("{} not found", p.display()))
            };
            let (d, _) = compare(&a)?;
            let r = simoxide_testkit::diff::diff_trace_text(&read(e)?, &read(x)?, &d)
                .map_err(|e| e.to_string())?;
            print!("{r}");
            Ok(if r.is_equal() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "corpus" => {
            let mut o = HarnessOptions::from_env();
            if let Some(f) = a.get("--filter") {
                o.filter = Some(f.to_string());
            }
            if let Some(c) = a.get("--corpus") {
                o.corpus = PathBuf::from(c);
            }
            o.modes = modes(&a);
            let (d, m) = compare(&a)?;
            o.trace_diff = d;
            o.measurements = m;
            let mut sim = simulator(&a)?;
            let r = corpus::run_corpus(sim.as_mut(), &o);
            print!("{}", r.table());
            if let Some(first) = r.cases.iter().find_map(|c| match &c.outcome {
                corpus::Outcome::Fail(f) => Some(format!("{} [{}]:\n{f}", c.name, c.mode.label())),
                _ => None,
            }) {
                println!("\nfirst failure: {first}");
            }
            Ok(if r.failed() == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
        "long" => {
            // long own-RNG runs without trace: corpus entries (or --gen N generated models) with
            // --max-measurements / --max-sim-time overrides; measurements must be identical
            let max_m: i64 = a.num("--max-measurements", 5000);
            let max_t: i64 = a.num("--max-sim-time", -1);
            let mut reqs = Vec::new();
            let work = std::env::temp_dir().join("testkit-long");
            if let Some(n) = a.get("--gen") {
                let mut o = fuzz_options(&a)?;
                o.n = n.parse().map_err(|_| "bad --gen")?;
                for i in 0..o.n {
                    let c = fuzz::config_for(&o, i);
                    reqs.push(fuzz::materialize(&c, &work).map_err(|e| e.to_string())?);
                }
            } else {
                let dir = PathBuf::from(a.get("--corpus").unwrap_or("corpus"));
                for e in corpus::list(&dir, a.get("--filter")).map_err(|e| e.to_string())? {
                    reqs.push(simoxide_testkit::sim::RunRequest {
                        name: e.name.clone(),
                        model_dir: e.dir.clone(),
                        config: e.config.clone(),
                        rng: simoxide_testkit::sim::RngMode::OwnRng,
                        trace: false,
                    });
                }
            }
            for r in &mut reqs {
                r.config.max_measurements = max_m;
                r.config.max_sim_time = max_t;
            }
            let mut cand = simulator(&a)?;
            let res = fuzz::run_long(
                &mut RefSim::new(),
                cand.as_mut(),
                &reqs,
                a.num("--batch", 8),
                !a.has("--quiet"),
            );
            let bad: Vec<&(String, Option<String>)> =
                res.iter().filter(|(_, r)| r.is_some()).collect();
            println!(
                "long runs (max_measurements {max_m}, max_sim_time {max_t}): {} models, {} identical",
                res.len(),
                res.len() - bad.len()
            );
            for (n, r) in &bad {
                println!("  {n}: {}", r.as_deref().unwrap_or(""));
            }
            let _ = std::fs::remove_dir_all(&work);
            Ok(
                if bad.iter().any(|(_, r)| {
                    r.as_deref()
                        .is_some_and(|x| x.starts_with("DIVERGENT") || x.starts_with("candidate"))
                }) {
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                },
            )
        }
        "equiv" => equiv(&a),
        _ => Err(format!("unknown command '{cmd}'")),
    }
}

fn equiv(a: &Args) -> Result<ExitCode, String> {
    use simoxide_testkit::equiv::{EquivOptions, Model, Report};
    let mut o = EquivOptions {
        seeds: a.num("--seeds", 40),
        first_seed: a.num("--first-seed", 1000),
        seed_offset_b: a.num("--seed-offset-b", 0),
        min_measurements: a.num("--min-measurements", 0),
        threads: a.num("--threads", 0),
        alpha: a.num("--alpha", 0.01),
        permutations: a.num("--permutations", 199),
        timeout: std::time::Duration::from_secs_f64(a.num("--timeout", 60.0)),
        ..EquivOptions::default()
    };
    if let Some(m) = a.get("--modes") {
        let v: Vec<simoxide_sim::Mode> = m
            .split(',')
            .map(|x| simoxide_sim::Mode::parse(x).ok_or(format!("--modes: unknown mode {x}")))
            .collect::<Result<_, _>>()?;
        if v.len() != 2 {
            return Err("--modes needs two modes".into());
        }
        o.modes = [v[0], v[1]];
    }
    let mut rep = Report {
        modes: o.modes.to_vec(),
        ..Report::default()
    };
    let verbose = !a.has("--quiet");
    let add = |rep: &mut Report, m: Result<Model, String>, name: &str| match m {
        Ok(m) => {
            rep.add_model(&m, &o);
            if verbose {
                let l = rep.models.last().expect("model line");
                eprintln!(
                    "{:40} aborts {}/{} tests {:4} min p {:.2e} ({:.1} s)",
                    l.name, l.errors[0], l.errors[1], l.tests, l.min_p, l.wall
                );
            }
        }
        Err(e) => rep.skipped.push((name.to_string(), e)),
    };
    if let Some(n) = a.get("--gen") {
        let mut f = fuzz_options(a)?;
        f.n = n.parse().map_err(|_| "bad --gen")?;
        f.prefix = "eq".into();
        for i in 0..f.n {
            let c = fuzz::config_for(&f, i);
            add(&mut rep, Model::generated(&c), &c.name);
        }
    }
    if a.get("--gen").is_none() || a.get("--corpus").is_some() {
        let dirs = a.get("--corpus").unwrap_or("corpus");
        for d in dirs.split(',').filter(|d| !d.is_empty()) {
            for e in
                corpus::list(&PathBuf::from(d), a.get("--filter")).map_err(|e| e.to_string())?
            {
                add(&mut rep, Model::from_dir(&e.dir), &e.name);
            }
        }
    }
    rep.finish(o.alpha);
    print!("{}", rep.text(a.num("--top", 40)));
    if let Some(p) = a.get("--tests-csv") {
        std::fs::write(p, rep.tests_csv()).map_err(|e| format!("{p}: {e}"))?;
    }
    Ok(if rep.failures().is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("simoxide-fuzz: {e}");
            ExitCode::from(2)
        }
    }
}

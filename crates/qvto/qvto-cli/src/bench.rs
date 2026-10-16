//! `qvto bench`: the benchmark suite (DESIGN §7.1). Workloads are cases of a workload file
//! (`bench/workloads.bench`, the case-file format with `bench <key> <value>` lines) or of a
//! conformance suite (`--suite eclipse|corpus`). Each workload is compiled several times and run
//! in compat and fast mode, and with `--java` the same workloads run in one JVM of the reference
//! runner (`reference/runner/src/qvtorun/Bench.java`, the same protocol); `--java-cold` adds a
//! fresh JVM per workload (first compile, first run).
//!
//! Protocol (both sides): compile K times (the program of the last compile runs); fresh mode
//! loads the inputs before every run (not timed) and runs until N runs or B ms of run time (at
//! least 5 runs); series mode (`bench series N`) loads them once and runs N times on the same,
//! changing models (SimuLizar's rules during a simulation), in `rounds` rounds (default 2,
//! each on freshly loaded models; the last round counts). A run is the VM's (Java: the
//! executor's) `run`/`execute`. Reported: median and p10–p90 of the second half of the runs
//! (fresh) or of the last round (series), the series' total, the first run, and the compile
//! times; the load average of the machine before and after.
//!
//! Workload keys: `bench group <B2..B6>`, `bench series <n>`, `bench rounds <n>`, `bench runs
//! <n>` (fresh: at most n runs), `bench note <text>`. Generated models (`target/qvto-bench/gen`)
//! are written by [`generate`] before a run (deterministic; `qvto bench gen` forces it).

use crate::engine::{self, CaseModels, compile_case};
use qvto_model::ModelStore;
use qvto_runtime::Mode;
use qvto_testkit::case::{Case, read_cases, write_case};
use qvto_testkit::engine::CaseEnv;
use qvto_testkit::{find_root, paths};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// The committed workload file.
pub const WORKLOADS: &str = "crates/qvto-cli/bench/workloads.bench";
/// Generated models and results.
pub const OUT_DIR: &str = "target/qvto-bench";

struct Opts {
    file: Option<PathBuf>,
    suite: Option<String>,
    only: Vec<String>,
    groups: Vec<String>,
    modes: Vec<Mode>,
    java: bool,
    java_cold: bool,
    rust: bool,
    compiles: usize,
    runs: usize,
    budget_ms: f64,
    report: Option<PathBuf>,
    /// `--profile FILE.svg`: sample the engine's runs (feature `profile`).
    profile: Option<PathBuf>,
    /// `--java-results FILE`: the Java runner's results of an earlier run (`java.tsv`).
    java_results: Option<PathBuf>,
}

fn die(msg: &str) -> ! {
    eprintln!("qvto bench: {msg}");
    std::process::exit(2)
}

/// Summary statistics of a list of times (ms).
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub n: usize,
    pub first: f64,
    pub median: f64,
    pub p10: f64,
    pub p90: f64,
    pub total: f64,
}

impl Stats {
    /// Statistics of `v[from..]` (the warm part), with `first` and `total` of all of `v`.
    fn of(v: &[f64], from: usize) -> Stats {
        if v.is_empty() {
            return Stats {
                first: f64::NAN,
                median: f64::NAN,
                p10: f64::NAN,
                p90: f64::NAN,
                ..Stats::default()
            };
        }
        let mut w: Vec<f64> = v[from.min(v.len() - 1)..].to_vec();
        w.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let q = |p: f64| w[((w.len() - 1) as f64 * p).round() as usize];
        Stats {
            n: v.len(),
            first: v[0],
            median: q(0.5),
            p10: q(0.1),
            p90: q(0.9),
            total: v.iter().sum(),
        }
    }

    /// Fresh runs: the second half; series: all runs of the (last) round.
    fn runs(v: &[f64], series: bool) -> Stats {
        Stats::of(v, if series { 0 } else { v.len() / 2 })
    }
}

/// What one side measured for a workload.
#[derive(Clone, Debug, Default)]
pub struct Measured {
    pub compile: Vec<f64>,
    pub runs: Vec<f64>,
    pub severity: String,
    pub code: i32,
    pub out_objects: usize,
    pub log_lines: usize,
    pub error: Option<String>,
}

fn bench_key<'a>(c: &'a Case, k: &str) -> Option<&'a str> {
    c.bench.iter().find(|(x, _)| x == k).map(|(_, v)| v.as_str())
}

fn series_of(c: &Case) -> usize {
    bench_key(c, "series").and_then(|v| v.parse().ok()).unwrap_or(0)
}

fn rounds_of(c: &Case) -> usize {
    bench_key(c, "rounds").and_then(|v| v.parse().ok()).unwrap_or(2)
}

fn group_of(c: &Case) -> String {
    bench_key(c, "group").map(String::from).unwrap_or_else(|| c.kind.clone())
}

pub fn bench(args: &[String]) {
    let mut o = Opts {
        file: None,
        suite: None,
        only: Vec::new(),
        groups: Vec::new(),
        modes: vec![Mode::Compat, Mode::Fast],
        java: false,
        java_cold: false,
        rust: true,
        compiles: 10,
        runs: 30,
        budget_ms: 2000.0,
        report: None,
        profile: None,
        java_results: None,
    };
    let mut gen_only = false;
    let mut i = 0;
    let next = |i: &mut usize| -> String {
        *i += 1;
        args.get(*i).cloned().unwrap_or_else(|| die("missing value"))
    };
    while i < args.len() {
        match args[i].as_str() {
            "gen" => gen_only = true,
            "--file" => o.file = Some(PathBuf::from(next(&mut i))),
            "--suite" => o.suite = Some(next(&mut i)),
            "--only" => o.only.extend(next(&mut i).split(',').map(String::from)),
            "--group" => o.groups.extend(next(&mut i).split(',').map(String::from)),
            "--modes" => {
                o.modes = next(&mut i)
                    .split(',')
                    .map(|m| match m {
                        "compat" => Mode::Compat,
                        "fast" => Mode::Fast,
                        _ => die("modes: compat,fast"),
                    })
                    .collect()
            }
            "--java" => o.java = true,
            "--java-cold" => o.java_cold = true,
            "--no-rust" => o.rust = false,
            "--compiles" => o.compiles = next(&mut i).parse().unwrap_or(10),
            "--runs" | "--repeat" => o.runs = next(&mut i).parse().unwrap_or(30),
            "--budget-ms" => o.budget_ms = next(&mut i).parse().unwrap_or(2000.0),
            "--report" => o.report = Some(PathBuf::from(next(&mut i))),
            "--profile" => o.profile = Some(PathBuf::from(next(&mut i))),
            "--java-results" => o.java_results = Some(PathBuf::from(next(&mut i))),
            a => die(&format!("unknown argument {a}")),
        }
        i += 1;
    }
    let root = find_root(Path::new(".")).unwrap_or_else(|| die("not inside the qvto-rs repository"));
    generate(&root, gen_only);
    if gen_only {
        return;
    }
    let file = match (&o.suite, &o.file) {
        (Some(s), _) => root.join(match s.as_str() {
            "eclipse" => paths::ECLIPSE_CASES,
            "corpus" => paths::CORPUS_CASES,
            _ => die("unknown suite"),
        }),
        (None, Some(f)) => f.clone(),
        (None, None) => root.join(WORKLOADS),
    };
    let all = read_cases(&file).unwrap_or_else(|e| die(&format!("{}: {e}", file.display())));
    let cases: Vec<Case> = all
        .into_iter()
        .filter(|c| c.runnable() && !c.compile_only())
        .filter(|c| o.only.is_empty() || o.only.contains(&c.id))
        .filter(|c| o.groups.is_empty() || o.groups.contains(&group_of(c)))
        .collect();
    if cases.is_empty() {
        die("no workloads");
    }
    let load0 = loadavg();
    let mut rust: Vec<(String, Vec<(Mode, Measured)>)> = Vec::new();
    if o.rust {
        let prof = o.profile.as_ref().map(|_| profile::start());
        for c in &cases {
            let mut per_mode = Vec::new();
            match measure_rust(&root, c, &o) {
                Ok(m) => {
                    for (mode, m) in m {
                        print_rust(&c.id, mode, &m, series_of(c) > 0);
                        per_mode.push((mode, m));
                    }
                }
                Err(e) => println!("qvto  {:34} error: {e}", c.id),
            }
            rust.push((c.id.clone(), per_mode));
        }
        if let (Some(p), Some(svg)) = (prof, &o.profile) {
            profile::finish(p, svg);
        }
    }
    let mut java: Vec<(String, Measured)> = Vec::new();
    let mut java_cold: Vec<(String, Measured)> = Vec::new();
    if let Some(f) = &o.java_results {
        java = parse_java(&std::fs::read_to_string(f).unwrap_or_else(|e| die(&format!("{}: {e}", f.display()))));
        java.retain(|(id, _)| cases.iter().any(|c| &c.id == id));
    }
    if o.java || o.java_cold {
        let dir = root.join(OUT_DIR);
        let _ = std::fs::create_dir_all(&dir);
        let jobs = dir.join("java.bench");
        let mut text = String::new();
        for c in &cases {
            write_case(c, &mut text);
            text.push('\n');
        }
        std::fs::write(&jobs, text).unwrap_or_else(|e| die(&e.to_string()));
        if o.java {
            let out = dir.join("java.tsv");
            match run_java(&root, &jobs, &out, None, &o) {
                Ok(rows) => java = rows,
                Err(e) => println!("java: {e}"),
            }
            for (id, m) in &java {
                let c = cases.iter().find(|c| &c.id == id);
                let series = c.map(|c| series_of(c) > 0).unwrap_or(false);
                print_side("java", id, &m.compile, &m.runs, series, m);
            }
        }
        if o.java_cold {
            for c in &cases {
                let out = dir.join("java-cold.tsv");
                let cold = Opts {
                    compiles: 1,
                    runs: 1,
                    ..clone_opts(&o)
                };
                match run_java(&root, &jobs, &out, Some(&c.id), &cold) {
                    Ok(mut rows) => {
                        if let Some((id, m)) = rows.pop() {
                            println!(
                                "jcold {:34} compile {:9.3} ms  run {:9.3} ms",
                                id,
                                m.compile.first().copied().unwrap_or(f64::NAN),
                                m.runs.first().copied().unwrap_or(f64::NAN)
                            );
                            java_cold.push((id, m));
                        }
                    }
                    Err(e) => println!("java cold {}: {e}", c.id),
                }
            }
        }
    }
    let load1 = loadavg();
    let md = report(&cases, &rust, &java, &java_cold, &load0, &load1);
    println!("\n{md}");
    let dir = root.join(OUT_DIR);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("last.md"), &md);
    let _ = std::fs::write(dir.join("last.tsv"), tsv(&cases, &rust, &java, &java_cold));
    if let Some(p) = &o.report {
        std::fs::write(p, &md).unwrap_or_else(|e| die(&format!("{}: {e}", p.display())));
    }
}

fn clone_opts(o: &Opts) -> Opts {
    Opts {
        file: o.file.clone(),
        suite: o.suite.clone(),
        only: o.only.clone(),
        groups: o.groups.clone(),
        modes: o.modes.clone(),
        java: o.java,
        java_cold: o.java_cold,
        rust: o.rust,
        compiles: o.compiles,
        runs: o.runs,
        budget_ms: o.budget_ms,
        report: None,
        profile: None,
        java_results: None,
    }
}

fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg")
        .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

fn print_side(side: &str, id: &str, compile: &[f64], runs: &[f64], series: bool, m: &Measured) {
    let c = Stats::of(compile, compile.len().min(1));
    let r = Stats::runs(runs, series);
    if let Some(e) = &m.error {
        println!("{side:5} {id:34} error: {e}");
        return;
    }
    println!(
        "{side:5} {id:34} compile {:8.3} ms (first {:8.3})  run {:9.3} ms [{:.3}–{:.3}] first {:9.3}{}  n {} | {} {} objs {} log {}",
        c.median,
        c.first,
        r.median,
        r.p10,
        r.p90,
        r.first,
        if series {
            format!(" total {:9.2}", r.total)
        } else {
            String::new()
        },
        r.n,
        m.severity,
        m.code,
        m.out_objects,
        m.log_lines
    );
}

fn print_rust(id: &str, mode: Mode, m: &Measured, series: bool) {
    let side = match mode {
        Mode::Compat => "qvto",
        Mode::Fast => "fast",
    };
    print_side(side, id, &m.compile, &m.runs, series, m);
}

/// Runs a workload with the engine: compiles it `compiles` times, then runs it in each mode.
fn measure_rust(root: &Path, c: &Case, o: &Opts) -> Result<Vec<(Mode, Measured)>, String> {
    let models = qvto_emf::config::Config::read(&root.join(paths::SUITE_CONF), root)
        .map_err(|e| format!("{}: {e}", paths::SUITE_CONF))?;
    let conf_models;
    let models = match &c.conf {
        Some(cf) => {
            conf_models = qvto_emf::config::Config::read(&root.join(cf), root)
                .map_err(|e| format!("{cf}: {e}"))?;
            &conf_models
        }
        None => &models,
    };
    let signatures = std::fs::read_to_string(root.join(engine::BLACKBOX_SIGNATURES)).unwrap_or_default();
    let bundles = qvto::units::FsResolver::bundles_below(&root.join("reference/eclipse-qvto/tests"));
    let compiles = o.compiles.max(1);
    let modes = o.modes.clone();
    let (runs, budget) = (
        bench_key(c, "runs").and_then(|v| v.parse().ok()).unwrap_or(o.runs),
        o.budget_ms,
    );
    let series = series_of(c);
    let rounds = rounds_of(c).max(1);
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(512 << 20)
            .spawn_scoped(s, || {
                let env = CaseEnv {
                    case: c,
                    base: root,
                    models,
                };
                let mut compile = Vec::new();
                let mut prog = None;
                for _ in 0..compiles {
                    let p = compile_case(&env, bundles.clone(), &signatures)
                        .map_err(|o| format!("{:?}", o.status))?;
                    compile.push(p.compile_time.as_secs_f64() * 1e3);
                    prog = Some(p);
                }
                let p = prog.unwrap();
                if p.compiled.has_errors() {
                    return Err("compile errors".to_string());
                }
                let program = p.compiled.program.as_ref().ok_or("no program")?;
                // the blackbox implementations, registered once (as a host does)
                let natives = std::rc::Rc::new(crate::bbox::natives());
                let mut out = Vec::new();
                for &mode in &modes {
                    let mut m = Measured {
                        compile: compile.clone(),
                        ..Measured::default()
                    };
                    let rounds = if series > 0 { rounds } else { 1 };
                    for _ in 0..rounds {
                        m.runs.clear();
                        let mut models: Option<CaseModels> = None;
                        let n = if series > 0 { series } else { runs };
                        let mut total = 0.0;
                        for r in 0..n {
                            if series == 0 && r >= 5 && total > budget {
                                break;
                            }
                            if series == 0 || models.is_none() {
                                models = Some(
                                    engine::case_models(&env, program, &p.mm_uris)
                                        .map_err(|o| format!("{:?}", o.status))?,
                                );
                            }
                            let cm = models.as_mut().unwrap();
                            let args = if series > 0 && r > 0 { cm.current_args() } else { cm.args.clone() };
                            let opts = qvto::RunOptions {
                                mode,
                                max_stack_depth: c.max_stack_depth.map(|d| d as usize).unwrap_or(1000),
                                readonly_guard: c.readonly_guard,
                                config: c.config.clone(),
                                line_numbers: !c.no_line_numbers,
                                ..qvto::RunOptions::default()
                            };
                            let (res, log, dt) = timed_run(program, &mut cm.rs.graph, opts, args, &natives);
                            m.runs.push(dt);
                            total += dt;
                            m.severity = severity_name(res.severity).into();
                            m.code = res.code;
                            m.log_lines = log.lines().count();
                            m.out_objects = res
                                .extents
                                .iter()
                                .flatten()
                                .flatten()
                                .filter(|o| !o.is_local())
                                .map(|o| {
                                    1 + cm.rs.graph.all_contents(qvto_emf::ObjId(o.index())).len()
                                })
                                .sum();
                        }
                    }
                    out.push((mode, m));
                }
                Ok(out)
            })
            .expect("thread")
            .join()
            .unwrap_or_else(|_| Err("panic".into()))
    })
}

/// One timed run: the VM's creation, the run and its drop (the profile keeps the samples
/// below this function).
#[inline(never)]
fn timed_run(
    program: &qvto::Program,
    g: &mut qvto_emf::graph::Graph,
    opts: qvto::RunOptions,
    args: Vec<Vec<qvto_emf::ObjId>>,
    natives: &std::rc::Rc<qvto_vm::Natives<qvto_emf::graph::Graph>>,
) -> (qvto::RunOutcome, String, f64) {
    let t = Instant::now();
    let mut vm = qvto::Vm::new(program, g, opts);
    vm.set_natives(natives.clone());
    let res = vm.run_transformation(args);
    let log = std::mem::take(&mut vm.log);
    drop(vm);
    let dt = t.elapsed().as_secs_f64() * 1e3;
    (res, log.to_string(), dt)
}

fn severity_name(s: i32) -> &'static str {
    match s {
        0 => "OK",
        1 => "INFO",
        2 => "WARNING",
        8 => "CANCEL",
        _ => "ERROR",
    }
}

/// Runs the workloads in one JVM (`qvto-run --bench`) at normal priority (the caller pins the
/// process to cores with `taskset`; the JVM inherits the affinity).
fn run_java(root: &Path, jobs: &Path, out: &Path, only: Option<&str>, o: &Opts) -> Result<Vec<(String, Measured)>, String> {
    let runner = root.join("reference/runner");
    if !runner.join("build/classpath.txt").is_file() {
        let st = Command::new(runner.join("build.sh"))
            .status()
            .map_err(|e| format!("build.sh: {e}"))?;
        if !st.success() {
            return Err("reference/runner/build.sh failed".into());
        }
    }
    let _ = std::fs::remove_file(out);
    let mut cmd = Command::new(runner.join("qvto-run"));
    cmd.current_dir(root)
        .env("QVTO_NICE", "0")
        .arg("--bench")
        .arg(jobs)
        .arg("--out")
        .arg(out)
        .arg("--base")
        .arg(root)
        .arg("--compiles")
        .arg(o.compiles.to_string())
        .arg("--runs")
        .arg(o.runs.to_string())
        .arg("--budget-ms")
        .arg(format!("{}", o.budget_ms as u64));
    if let Some(id) = only {
        cmd.arg("--only").arg(id);
    } else if !o.only.is_empty() {
        cmd.arg("--only").arg(o.only.join(","));
    }
    let res = cmd.output().map_err(|e| format!("qvto-run: {e}"))?;
    let text = std::fs::read_to_string(out).map_err(|e| {
        format!(
            "{}: {e}\n{}{}",
            out.display(),
            String::from_utf8_lossy(&res.stdout),
            String::from_utf8_lossy(&res.stderr)
        )
    })?;
    Ok(parse_java(&text))
}

/// Rows of the Java runner's `RESULTS.tsv`.
fn parse_java(text: &str) -> Vec<(String, Measured)> {
    let nums = |s: &str| -> Vec<f64> { s.split(',').filter_map(|x| x.parse().ok()).collect() };
    let mut rows = Vec::new();
    for line in text.lines().skip(1) {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 7 {
            continue;
        }
        let mut m = Measured {
            compile: nums(f[1]),
            runs: nums(f[2]),
            severity: f[3].into(),
            code: f[4].parse().unwrap_or(0),
            out_objects: f[5].parse().unwrap_or(0),
            log_lines: f[6].parse().unwrap_or(0),
            error: None,
        };
        if f[3] == "error" {
            m.error = Some(f.get(7).copied().unwrap_or("error").to_string());
        }
        rows.push((f[0].to_string(), m));
    }
    rows
}

fn fmt_ms(x: f64) -> String {
    if x.is_nan() {
        "–".into()
    } else if x >= 100.0 {
        format!("{x:.0}")
    } else if x >= 10.0 {
        format!("{x:.1}")
    } else if x >= 1.0 {
        format!("{x:.2}")
    } else {
        format!("{x:.3}")
    }
}

fn ratio(a: f64, b: f64) -> String {
    if a.is_nan() || b.is_nan() || a <= 0.0 {
        "–".into()
    } else {
        let r = b / a;
        if r >= 10.0 { format!("{r:.0}×") } else { format!("{r:.1}×") }
    }
}

/// The Markdown report: per workload the run times (median [p10–p90]) of compat, fast and
/// warm Java, the ratios, compile times and the sanity columns (severity, objects, log lines
/// equal on both sides).
fn report(
    cases: &[Case],
    rust: &[(String, Vec<(Mode, Measured)>)],
    java: &[(String, Measured)],
    cold: &[(String, Measured)],
    load0: &str,
    load1: &str,
) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "Load average before {load0}, after {load1}. Times in ms; runs: median [p10–p90] of the warm runs (fresh: second half; series: per run of the last round, Σ = series total); compile: median of the compiles (first in parentheses)."
    );
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "| Group | Workload | compat run | fast run | Java run (warm) | Java cold run | compat vs Java | fast vs compat | compile | Java compile (first) | check |"
    );
    let _ = writeln!(s, "|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|");
    for c in cases {
        let series = series_of(c) > 0;
        let r = rust.iter().find(|(id, _)| id == &c.id).map(|x| &x.1);
        let get = |mode: Mode| r.and_then(|v| v.iter().find(|(m, _)| *m == mode)).map(|x| &x.1);
        let comp = get(Mode::Compat);
        let fast = get(Mode::Fast);
        let j = java.iter().find(|(id, _)| id == &c.id).map(|x| &x.1);
        let jc = cold.iter().find(|(id, _)| id == &c.id).map(|x| &x.1);
        let cell = |m: Option<&Measured>| -> (String, f64) {
            match m {
                Some(m) if m.error.is_none() && !m.runs.is_empty() => {
                    let st = Stats::runs(&m.runs, series);
                    let mut t = format!("{} [{}–{}]", fmt_ms(st.median), fmt_ms(st.p10), fmt_ms(st.p90));
                    if series {
                        let _ = write!(t, " Σ {}", fmt_ms(st.total));
                    }
                    (t, if series { st.total } else { st.median })
                }
                Some(m) if m.error.is_some() => ("error".into(), f64::NAN),
                _ => ("–".into(), f64::NAN),
            }
        };
        let (ct, cv) = cell(comp);
        let (ft, fv) = cell(fast);
        let (jt, jv) = cell(j);
        let jcold = jc
            .and_then(|m| m.runs.first().copied())
            .map(fmt_ms)
            .unwrap_or_else(|| "–".into());
        let compile = comp
            .map(|m| {
                let st = Stats::of(&m.compile, 0);
                format!("{} ({})", fmt_ms(st.median), fmt_ms(st.first))
            })
            .unwrap_or_else(|| "–".into());
        let jcompile = j
            .map(|m| {
                let st = Stats::of(&m.compile, m.compile.len() / 2);
                let first = jc.and_then(|m| m.compile.first().copied()).unwrap_or(st.first);
                format!("{} ({})", fmt_ms(st.median), fmt_ms(first))
            })
            .unwrap_or_else(|| "–".into());
        let check = match (comp, j) {
            (Some(a), Some(b)) if a.error.is_none() && b.error.is_none() => {
                if a.severity == b.severity && a.out_objects == b.out_objects && a.log_lines == b.log_lines {
                    "same".to_string()
                } else {
                    format!(
                        "differs: {} {} {} / {} {} {}",
                        a.severity, a.out_objects, a.log_lines, b.severity, b.out_objects, b.log_lines
                    )
                }
            }
            (Some(a), None) if a.error.is_none() => format!("{} objs {} log {}", a.severity, a.out_objects, a.log_lines),
            _ => "–".into(),
        };
        let _ = writeln!(
            s,
            "| {} | `{}` | {ct} | {ft} | {jt} | {jcold} | {} | {} | {compile} | {jcompile} | {check} |",
            group_of(c),
            c.id,
            ratio(cv, jv),
            ratio(fv, cv),
        );
    }
    s
}

fn tsv(
    cases: &[Case],
    rust: &[(String, Vec<(Mode, Measured)>)],
    java: &[(String, Measured)],
    cold: &[(String, Measured)],
) -> String {
    let mut s = String::from("id\tside\tcompile_ms\trun_ms\tseverity\tout_objects\tlog_lines\n");
    let join = |v: &[f64]| v.iter().map(|x| format!("{x:.4}")).collect::<Vec<_>>().join(",");
    for c in cases {
        let mut rows: Vec<(&str, &Measured)> = Vec::new();
        if let Some((_, v)) = rust.iter().find(|(id, _)| id == &c.id) {
            for (m, x) in v {
                rows.push((if *m == Mode::Compat { "compat" } else { "fast" }, x));
            }
        }
        if let Some((_, x)) = java.iter().find(|(id, _)| id == &c.id) {
            rows.push(("java", x));
        }
        if let Some((_, x)) = cold.iter().find(|(id, _)| id == &c.id) {
            rows.push(("java-cold", x));
        }
        for (side, m) in rows {
            let _ = writeln!(
                s,
                "{}\t{side}\t{}\t{}\t{}\t{}\t{}",
                c.id,
                join(&m.compile),
                join(&m.runs),
                m.severity,
                m.out_objects,
                m.log_lines
            );
        }
    }
    s
}

// --- generated models ---------------------------------------------------------------------

/// Version of the generators; a change regenerates the models.
const GEN_VERSION: &str = "2";

/// Writes the generated models of the workloads into `target/qvto-bench/gen` (unless they are
/// there with the current generator version, or `force`).
pub fn generate(root: &Path, force: bool) {
    let gdir = root.join(OUT_DIR).join("gen");
    let stamp = gdir.join("VERSION");
    if !force && std::fs::read_to_string(&stamp).ok().as_deref() == Some(GEN_VERSION) {
        return;
    }
    let _ = std::fs::remove_dir_all(&gdir);
    let res = (|| -> Result<(), String> {
        for scale in [1, 10, 100] {
            write(&gdir.join(format!("simpleuml/x{scale}.simpleuml")), &simpleuml(root, scale)?)?;
            write(&gdir.join(format!("ecore/x{scale}.ecore")), &ecore(scale))?;
        }
        let corpus = root.join("crates/qvto-testkit/corpus");
        for (name, dir) in [
            ("loadbalancer", "simulizar/loadbalancer"),
            ("runconfigtest", "simulizar/runconfigtest"),
            ("spd", "spd/minimalexample"),
        ] {
            for scale in [1, 10, 100] {
                pcm(&corpus.join(dir), &gdir.join(format!("pcm/{name}-x{scale}")), scale)?;
            }
        }
        write(&stamp, GEN_VERSION)
    })();
    if let Err(e) = res {
        die(&format!("generating models: {e}"));
    }
    eprintln!("qvto bench: generated models in {}", gdir.display());
}

fn write(p: &Path, text: &str) -> Result<(), String> {
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::write(p, text).map_err(|e| format!("{}: {e}", p.display()))
}

/// A SimpleUML model with `scale` copies of the classes of the SimpleUML→RDB sample's
/// `pim.simpleuml` (persistent classes with primary keys, relationships, generalizations,
/// enumerations, a data type), each in a package `domain<i>`, and the primitive types package.
fn simpleuml(root: &Path, scale: usize) -> Result<String, String> {
    let p = root.join("reference/eclipse-qvto/plugins/org.eclipse.m2m.qvt.oml.samples/projects/org.eclipse.m2m.qvt.oml.samples.simpleuml2rdb/pim.simpleuml");
    let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.contains("name=\"class\">"))
        .ok_or("pim.simpleuml: no package 'class'")?;
    let indent = lines[start].len() - lines[start].trim_start().len();
    let end = (start + 1..lines.len())
        .find(|&i| lines[i].trim() == "</ownedElements>" && lines[i].len() - lines[i].trim_start().len() == indent)
        .ok_or("pim.simpleuml: unterminated package")?;
    // the path of the sample's package `class` (seven levels below the model)
    let class_pkg = format!("//@ownedElements.0{}/", "/@ownedElements.0".repeat(6));
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<uml:Model xmi:version=\"2.0\" xmlns:xmi=\"http://www.omg.org/XMI\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:uml=\"http://www.eclipse.org/qvt/1.0.0/Operational/examples/simpleuml\" name=\"model\">\n",
    );
    for i in 0..scale {
        let _ = writeln!(s, "  <ownedElements xsi:type=\"uml:Package\" name=\"domain{i}\">");
        for l in &lines[start + 1..end] {
            let l = l
                .replace("//@ownedElements.1/", &format!("//@ownedElements.{scale}/"))
                .replace(&class_pkg, &format!("//@ownedElements.{i}/"));
            let _ = writeln!(s, "    {}", l.trim_start());
        }
        s.push_str("  </ownedElements>\n");
    }
    s.push_str(
        "  <ownedElements xsi:type=\"uml:Package\" name=\"primitivetypes\">\n    <ownedElements xsi:type=\"uml:PrimitiveType\" name=\"String\"/>\n    <ownedElements xsi:type=\"uml:PrimitiveType\" name=\"Integer\"/>\n    <ownedElements xsi:type=\"uml:PrimitiveType\" name=\"Boolean\"/>\n  </ownedElements>\n</uml:Model>\n",
    );
    Ok(s)
}

/// An Ecore model with `5 * scale` subpackages of 10 classes each (an abstract base, a
/// supertype chain, attributes of Ecore data types and an enumeration, containment and
/// cross references within the package, a supertype from the previous package).
fn ecore(scale: usize) -> String {
    let n = 5 * scale;
    let et = |t: &str| format!("ecore:EDataType http://www.eclipse.org/emf/2002/Ecore#//{t}");
    let mut s = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ecore:EPackage xmi:version=\"2.0\" xmlns:xmi=\"http://www.omg.org/XMI\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xmlns:ecore=\"http://www.eclipse.org/emf/2002/Ecore\" name=\"gen\" nsURI=\"http://qvto-rs/bench/gen\" nsPrefix=\"gen\">\n",
    );
    for p in 0..n {
        let _ = writeln!(
            s,
            "  <eSubpackages name=\"p{p}\" nsURI=\"http://qvto-rs/bench/gen/p{p}\" nsPrefix=\"p{p}\">"
        );
        let _ = writeln!(
            s,
            "    <eClassifiers xsi:type=\"ecore:EEnum\" name=\"Kind{p}\">\n      <eLiterals name=\"A\"/>\n      <eLiterals name=\"B\" value=\"1\"/>\n      <eLiterals name=\"C\" value=\"2\"/>\n    </eClassifiers>"
        );
        for c in 0..10 {
            let mut supers = Vec::new();
            if c > 0 {
                supers.push(format!("#//p{p}/C{}", c - 1));
            }
            if c == 5 && p > 0 {
                supers.push(format!("#//p{}/C0", p - 1));
            }
            let _ = writeln!(
                s,
                "    <eClassifiers xsi:type=\"ecore:EClass\" name=\"C{c}\"{}{}>",
                if c == 0 { " abstract=\"true\"" } else { "" },
                if supers.is_empty() {
                    String::new()
                } else {
                    format!(" eSuperTypes=\"{}\"", supers.join(" "))
                }
            );
            let _ = writeln!(
                s,
                "      <eStructuralFeatures xsi:type=\"ecore:EAttribute\" name=\"name{c}\" eType=\"{}\"/>\n      <eStructuralFeatures xsi:type=\"ecore:EAttribute\" name=\"size{c}\" eType=\"{}\"/>\n      <eStructuralFeatures xsi:type=\"ecore:EAttribute\" name=\"flag{c}\" eType=\"{}\"/>\n      <eStructuralFeatures xsi:type=\"ecore:EAttribute\" name=\"kind{c}\" eType=\"#//p{p}/Kind{p}\"/>",
                et("EString"),
                et("EInt"),
                et("EBoolean")
            );
            let _ = writeln!(
                s,
                "      <eStructuralFeatures xsi:type=\"ecore:EReference\" name=\"parts{c}\" upperBound=\"-1\" eType=\"#//p{p}/C{}\" containment=\"true\"/>\n      <eStructuralFeatures xsi:type=\"ecore:EReference\" name=\"link{c}\" eType=\"#//p{p}/C{}\"/>",
                (c + 1) % 10,
                (c + 3) % 10
            );
            s.push_str("    </eClassifiers>\n");
        }
        s.push_str("  </eSubpackages>\n");
    }
    s.push_str("</ecore:EPackage>\n");
    s
}

/// A copy of a PCM model directory with `(scale - 1) * k` more resource containers, assembly
/// contexts and allocation contexts (k: the allocation contexts of the original), inserted
/// before the closing tags of the `.resourceenvironment`, `.system` and `.allocation` roots.
fn pcm(src: &Path, dst: &Path, scale: usize) -> Result<(), String> {
    copy_dir(src, dst)?;
    let find = |ext: &str| -> Result<PathBuf, String> {
        std::fs::read_dir(dst)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| p.extension().is_some_and(|x| x == ext))
            .ok_or_else(|| format!("{}: no .{ext}", dst.display()))
    };
    let (env, sys, alloc) = (find("resourceenvironment")?, find("system")?, find("allocation")?);
    let read = |p: &Path| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
    let (env_t, sys_t, alloc_t) = (read(&env)?, read(&sys)?, read(&alloc)?);
    let k = alloc_t.matches("<allocationContexts_Allocation").count().max(1);
    let extra = (scale - 1) * k;
    if extra == 0 {
        return Ok(());
    }
    // the component of the system's first assembly context
    let comp = sys_t
        .find("<encapsulatedComponent__AssemblyContext")
        .map(|i| {
            let j = sys_t[i..].find("/>").map(|j| i + j + 2).unwrap_or(sys_t.len());
            sys_t[i..j].to_string()
        })
        .ok_or("system without assembly contexts")?;
    let env_name = env.file_name().unwrap().to_string_lossy().to_string();
    let sys_name = sys.file_name().unwrap().to_string_lossy().to_string();
    let mut e = String::new();
    let mut y = String::new();
    let mut a = String::new();
    for i in 0..extra {
        let _ = write!(
            e,
            "  <resourceContainer_ResourceEnvironment id=\"_gen_rc_{i}\" entityName=\"GenNode{i}\">\n    <activeResourceSpecifications_ResourceContainer id=\"_gen_ars_{i}\">\n      <schedulingPolicy href=\"pathmap://PCM_MODELS/Palladio.resourcetype#ProcessorSharing\"/>\n      <activeResourceType_ActiveResourceSpecification href=\"pathmap://PCM_MODELS/Palladio.resourcetype#_oro4gG3fEdy4YaaT-RYrLQ\"/>\n      <processingRate_ProcessingResourceSpecification specification=\"{}\"/>\n    </activeResourceSpecifications_ResourceContainer>\n  </resourceContainer_ResourceEnvironment>\n",
            500 + i % 7 * 100
        );
        let _ = write!(
            y,
            "  <assemblyContexts__ComposedStructure id=\"_gen_ac_{i}\" entityName=\"GenAssembly{i}\">\n    {comp}\n  </assemblyContexts__ComposedStructure>\n"
        );
        let _ = write!(
            a,
            "  <allocationContexts_Allocation id=\"_gen_alc_{i}\" entityName=\"GenAllocation{i}\">\n    <resourceContainer_AllocationContext href=\"{env_name}#_gen_rc_{i}\"/>\n    <assemblyContext_AllocationContext href=\"{sys_name}#_gen_ac_{i}\"/>\n  </allocationContexts_Allocation>\n"
        );
    }
    let insert = |text: &str, add: &str| -> Result<String, String> {
        let i = text.trim_end().rfind("</").ok_or("no closing tag")?;
        Ok(format!("{}{}{}", &text[..i], add, &text[i..]))
    };
    write(&env, &insert(&env_t, &e)?)?;
    write(&sys, &insert(&sys_t, &y)?)?;
    write(&alloc, &insert(&alloc_t, &a)?)?;
    Ok(())
}

fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
    for e in std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let p = e.path();
        let to = dst.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &to)?;
        } else {
            std::fs::copy(&p, &to).map_err(|e| format!("{}: {e}", p.display()))?;
        }
    }
    Ok(())
}

/// Sampling profiles of the engine's runs (`--profile FILE.svg`, feature `profile`): a flame
/// graph and a text summary (self and inclusive samples per function) of the samples taken
/// during runs (model loading and compiling are left out).
#[cfg(feature = "profile")]
mod profile {
    use std::collections::HashMap;
    use std::path::Path;

    pub struct Prof(pprof::ProfilerGuard<'static>);

    pub fn start() -> Prof {
        Prof(
            pprof::ProfilerGuardBuilder::default()
                .frequency(4000)
                .blocklist(&["libc", "libgcc", "pthread", "vdso"])
                .build()
                .expect("profiler"),
        )
    }

    fn short(name: &str) -> String {
        let n = match name.rfind("::h") {
            Some(i) if name.len() - i == 19 => &name[..i],
            _ => name,
        };
        n.to_string()
    }

    pub fn finish(p: Prof, svg: &Path) {
        let report = match p.0.report().build() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("profile: {e}");
                return;
            }
        };
        let skip = ["case_models", "compile_case", "bench::generate", "read_to_string"];
        let mut data = HashMap::new();
        let mut selfc: HashMap<String, isize> = HashMap::new();
        let mut incl: HashMap<String, isize> = HashMap::new();
        let mut total = 0;
        for (frames, &count) in &report.data {
            let names: Vec<String> = frames
                .frames
                .iter()
                .flat_map(|f| f.iter().map(|s| short(&s.name())))
                .collect();
            if names.iter().any(|n| skip.iter().any(|k| n.contains(k))) {
                continue;
            }
            if !names.iter().any(|n| n.contains("timed_run")) {
                continue;
            }
            total += count;
            if let Some(n) = names.first() {
                *selfc.entry(n.clone()).or_default() += count;
            }
            let mut seen = std::collections::HashSet::new();
            for n in &names {
                if seen.insert(n.clone()) {
                    *incl.entry(n.clone()).or_default() += count;
                }
            }
            data.insert(frames.clone(), count);
        }
        let filtered = pprof::Report {
            data,
            timing: report.timing.clone(),
        };
        if let Ok(f) = std::fs::File::create(svg) {
            let _ = filtered.flamegraph(f);
        }
        let pct = |c: isize| 100.0 * c as f64 / total.max(1) as f64;
        let mut v: Vec<_> = selfc.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        let mut text = format!("{total} samples in runs\n\nself\n");
        for (n, c) in v.iter().take(60) {
            text.push_str(&format!("{:6.2}%  {n}\n", pct(*c)));
        }
        let mut v: Vec<_> = incl.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1));
        text.push_str("\ninclusive\n");
        for (n, c) in v.iter().take(100) {
            text.push_str(&format!("{:6.2}%  {n}\n", pct(*c)));
        }
        let txt = svg.with_extension("txt");
        let _ = std::fs::write(&txt, &text);
        eprintln!("profile: {} and {}", svg.display(), txt.display());
    }
}

#[cfg(not(feature = "profile"))]
mod profile {
    pub struct Prof;
    pub fn start() -> Prof {
        eprintln!("qvto bench: --profile needs the feature `profile` (cargo build --features profile)");
        Prof
    }
    pub fn finish(_: Prof, _: &std::path::Path) {}
}

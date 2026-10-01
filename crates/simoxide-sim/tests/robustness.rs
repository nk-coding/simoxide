//! Robustness on untrusted input: invalid, mutated or pathological models and StoEx must end in
//! an error (never a panic, stack overflow, unbounded memory or endless run) under
//! [`simoxide_sim::Limits`]. Also the embedding API (`simulate_memory`, summaries, limits).
//!
//! Longer mutation campaigns: `ROBUST_N=100000 ROBUST_SEED=7 cargo test --release -p simoxide-sim
//! --test robustness mutated -- --nocapture`.

use simoxide_sim::{
    CompiledModel, Limits, Outputs, RunError, RunSpec, SimConfig, SimErrorKind, Simulation,
    simulate_memory,
};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn corpus(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus")
        .join(name)
}

/// The model files of a corpus directory (no `run.json`, no `expected/`).
fn files(name: &str) -> (Vec<(String, String)>, RunSpec) {
    let dir = corpus(name);
    let mut v = Vec::new();
    for e in std::fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_file() && p.extension().is_some_and(|x| x != "json" && x != "txt") {
            let n = p.file_name().unwrap().to_string_lossy().into_owned();
            v.push((n, std::fs::read_to_string(&p).unwrap()));
        }
    }
    v.sort();
    (v, RunSpec::load(&dir.join("run.json")).unwrap())
}

fn replace(files: &mut [(String, String)], ext: &str, from: &str, to: &str) {
    let f = files.iter_mut().find(|f| f.0.ends_with(ext)).unwrap();
    assert!(f.1.contains(from), "{from} not in {}", f.0);
    f.1 = f.1.replace(from, to);
}

/// SplitMix64.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Runs `f` on a thread with a 1 MiB stack (the JVM's default thread stack, relevant for
/// callers through JNI); a stack overflow would abort the whole test binary.
fn small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn deeply_nested_stoex_is_an_error_not_a_stack_overflow() {
    small_stack(|| {
        let deep = |kind: &str, n: usize| -> String {
            match kind {
                "paren" => format!("{}1{}", "(".repeat(n), ")".repeat(n)),
                "neg" => format!("{}1", "-".repeat(n)),
                "not" => format!("{}true", "NOT ".repeat(n)),
                "call" => format!("{}1.0{}", "Exp(".repeat(n), ")".repeat(n)),
                "sum" => format!("1{}", "+1".repeat(n)),
                "mixed" => format!("{}1{}", "(1+".repeat(n), ")".repeat(n)),
                _ => unreachable!(),
            }
        };
        for kind in ["paren", "neg", "not", "call", "sum", "mixed"] {
            for n in [1_000usize, 100_000] {
                let r = simoxide_stoex::prepare(&deep(kind, n));
                assert!(r.is_err(), "{kind} {n}");
            }
        }
        // within the limits: parses and evaluates
        let mut rng = simoxide_random::MersenneTwister::from_int(1);
        for (s, want) in [
            (deep("paren", 190), simoxide_stoex::Value::Int(1)),
            (deep("sum", 990), simoxide_stoex::Value::Int(991)),
            (deep("neg", 190), simoxide_stoex::Value::Int(1)),
        ] {
            let p = simoxide_stoex::Program::from_str(&s, |_| 0).unwrap();
            assert_eq!(p.eval(&simoxide_stoex::NoVars, &mut rng).unwrap(), want);
        }
    });
}

const TOKENS: &[&str] = &[
    "1",
    "0",
    "-1",
    "2147483647",
    "2147483648",
    "1.5",
    "1e308",
    "1e-320",
    "NaN",
    "\"a\"",
    "true",
    "false",
    "+",
    "-",
    "*",
    "/",
    "%",
    "^",
    "(",
    ")",
    "?",
    ":",
    ",",
    "AND",
    "OR",
    "XOR",
    "NOT",
    "<",
    ">",
    "==",
    "<>",
    ">=",
    "<=",
    "Exp(",
    "Norm(",
    "UniInt(",
    "UniDouble(",
    "Pois(",
    "Gamma(",
    "Lognorm(",
    "Trunc(",
    "Round(",
    "Min(",
    "Max(",
    "Log(",
    "Pow(",
    "IntPMF[",
    "DoublePMF[",
    "DoublePDF[",
    "BoolPMF[",
    "EnumPMF[",
    "(1;0.5)",
    "(2;0.5)",
    "(-3;2)",
    "(\"x\";1)",
    "(true;0.3)",
    "]",
    "a.VALUE",
    "a.INNER.VALUE",
    "x.BYTESIZE",
    "y.NUMBER_OF_ELEMENTS",
    ".",
    " ",
    "\u{1F600}",
    "\\",
    "\"",
];

fn random_stoex(r: &mut Rng) -> String {
    let n = 1 + r.below(12);
    (0..n).map(|_| TOKENS[r.below(TOKENS.len())]).collect()
}

#[test]
fn random_stoex_never_panics() {
    small_stack(|| {
        let mut r = Rng(42);
        let mut rng = simoxide_random::MersenneTwister::from_int(7);
        let mut env = simoxide_stoex::SimpleEnv::new();
        env.set("a.VALUE", simoxide_stoex::Value::Int(3));
        for _ in 0..20_000 {
            let s = random_stoex(&mut r);
            let res = catch_unwind(AssertUnwindSafe(|| {
                if let Ok(p) = simoxide_stoex::Program::from_str(&s, |_| 0) {
                    let _ = p.eval(&env, &mut rng);
                }
            }));
            assert!(res.is_ok(), "panic on StoEx {s:?}");
        }
    });
}

/// Loads, compiles and runs with tight limits; returns the error text, if any.
fn run_limited(files: &[(String, String)], spec: &RunSpec) -> Result<(), String> {
    let limits = Limits {
        max_steps: 300_000,
        max_events: 300_000,
        max_processes: 20_000,
        max_stack_depth: 5_000,
        deadline: Some(Instant::now() + Duration::from_secs(5)),
        cancel: None,
    };
    simulate_memory(files, spec, limits)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

const MUTATION_BASES: &[&str] = &[
    "h01_ps_single",
    "h05_open_workload",
    "h08_guarded_branch_params",
    "h09_loop",
    "h11_fork_sync",
    "h13_passive_contention",
    "h15_composite",
    "h16_component_params",
    "h19_linking_resource",
    "h27_collection_inner_multi",
    "h32_recursion",
    "x_pem_subsystem_nested",
];

/// One random mutation of the concatenated model text.
fn mutate(r: &mut Rng, fs: &mut [(String, String)]) {
    let i = r.below(fs.len());
    let text = &mut fs[i].1;
    let len = text.len();
    let at = |r: &mut Rng, t: &str| {
        let mut p = r.below(t.len() + 1);
        while !t.is_char_boundary(p) {
            p -= 1;
        }
        p
    };
    match r.below(8) {
        // truncate
        0 => {
            let p = at(r, text);
            text.truncate(p);
        }
        // overwrite a byte range with random printable garbage
        1 => {
            let p = at(r, text);
            let n = (1 + r.below(8)).min(len - p);
            let mut q = p + n;
            while !text.is_char_boundary(q) {
                q += 1;
            }
            let g: String = (0..n)
                .map(|_| b"<>\"'=/ &;#x0aZ-_.:"[r.below(18)] as char)
                .collect();
            text.replace_range(p..q, &g);
        }
        // replace a StoEx specification by random tokens
        2 | 3 => {
            let specs: Vec<usize> = text
                .match_indices("specification=\"")
                .map(|(p, s)| p + s.len())
                .collect();
            if let Some(&p) = specs.get(r.below(specs.len())) {
                let q = p + text[p..].find('"').unwrap_or(0);
                let s = random_stoex(r)
                    .replace('&', "&amp;")
                    .replace('"', "&quot;")
                    .replace('<', "&lt;");
                text.replace_range(p..q, &s);
            }
        }
        // point an attribute (ids, references, numbers) to another attribute's value
        4 | 5 => {
            let vals: Vec<(usize, usize)> = text
                .match_indices("=\"")
                .filter_map(|(p, _)| {
                    let s = p + 2;
                    text[s..].find('"').map(|e| (s, s + e))
                })
                .collect();
            if vals.len() >= 2 {
                let (a, b) = (vals[r.below(vals.len())], vals[r.below(vals.len())]);
                let v = text[b.0..b.1].to_string();
                let v = match r.below(4) {
                    0 => "-1".to_string(),
                    1 => "1e400".to_string(),
                    2 => String::new(),
                    _ => v,
                };
                text.replace_range(a.0..a.1, &v);
            }
        }
        // delete or duplicate a line
        _ => {
            let lines: Vec<&str> = text.lines().collect();
            if !lines.is_empty() {
                let k = r.below(lines.len());
                let mut out: Vec<&str> = lines.clone();
                if r.below(2) == 0 {
                    out.remove(k);
                } else {
                    out.insert(k, lines[k]);
                }
                *text = out.join("\n");
            }
        }
    }
}

#[test]
fn mutated_models_never_panic() {
    let n: usize = std::env::var("ROBUST_N")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);
    let bases: Vec<_> = MUTATION_BASES.iter().map(|b| (*b, files(b))).collect();
    let seed: u64 = std::env::var("ROBUST_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2026);
    let mut r = Rng(seed);
    let (mut ok, mut failed) = (0, 0);
    let t0 = Instant::now();
    for k in 0..n {
        let (name, (fs, spec)) = &bases[k % bases.len()];
        let mut fs = fs.clone();
        for _ in 0..1 + r.below(3) {
            mutate(&mut r, &mut fs);
        }
        let res = catch_unwind(AssertUnwindSafe(|| run_limited(&fs, spec)));
        match res {
            Ok(Ok(())) => ok += 1,
            Ok(Err(_)) => failed += 1,
            Err(_) => {
                let dir = std::env::temp_dir().join(format!("robustness-panic-{k}"));
                let _ = std::fs::create_dir_all(&dir);
                for (f, c) in &fs {
                    let _ = std::fs::write(dir.join(f), c);
                }
                panic!("panic on mutant {k} of {name}, saved to {}", dir.display());
            }
        }
    }
    eprintln!(
        "{n} mutants: {ok} ran, {failed} rejected with an error, {:.1} s",
        t0.elapsed().as_secs_f64()
    );
    assert!(ok > 0 && failed > 0);
}

fn expect_limit(files: &[(String, String)], spec: &RunSpec, limits: Limits, what: &str) {
    match simulate_memory(files, spec, limits) {
        Err(RunError::Sim(e)) => {
            assert_eq!(e.kind, SimErrorKind::Limit, "{e}");
            assert!(e.message.contains(what), "{e}");
        }
        other => panic!("expected a limit error ({what}), got {other:?}"),
    }
}

#[test]
fn pathological_models_stop_with_errors() {
    // a composite component containing itself: compile error (was: unbounded path enumeration)
    let (mut fs, spec) = files("h15_composite");
    replace(
        &mut fs,
        ".repository",
        r#"<assemblyContexts__ComposedStructure id="_h15_composite_ac2""#,
        r#"<assemblyContexts__ComposedStructure id="_self" entityName="S" encapsulatedComponent__AssemblyContext="_h15_composite_comp3"/><assemblyContexts__ComposedStructure id="_h15_composite_ac2""#,
    );
    match simulate_memory(&fs, &spec, Limits::default()) {
        Err(RunError::Compile(e)) => assert!(e.0.contains("contains itself"), "{e}"),
        other => panic!("{other:?}"),
    }

    // unbounded zero-time recursion: stack limit (was: out of memory)
    let (mut fs, spec) = files("h32_recursion");
    replace(
        &mut fs,
        ".repository",
        r#"specification="Exp(20.0)""#,
        r#"specification="0""#,
    );
    replace(
        &mut fs,
        ".repository",
        r#"branchProbability="0.4""#,
        r#"branchProbability="1.0""#,
    );
    replace(
        &mut fs,
        ".repository",
        r#"branchProbability="0.6""#,
        r#"branchProbability="0.0""#,
    );
    small_stack(move || expect_limit(&fs, &spec, Limits::default(), "Limits::max_stack_depth"));

    // 2e9 loop iterations without demands: step budget and deadline (was: runs for hours)
    let (mut fs, spec) = files("h09_loop");
    replace(
        &mut fs,
        ".repository",
        "IntPMF[(1;0.2)(2;0.5)(3;0.3)]",
        "2000000000",
    );
    replace(
        &mut fs,
        ".repository",
        r#"specification="Exp(20.0)""#,
        r#"specification="0""#,
    );
    let limits = Limits {
        max_steps: 1_000_000,
        ..Limits::default()
    };
    expect_limit(&fs, &spec, limits, "Limits::max_steps");
    let limits = Limits {
        deadline: Some(Instant::now() + Duration::from_millis(200)),
        ..Limits::default()
    };
    let t0 = Instant::now();
    match simulate_memory(&fs, &spec, limits) {
        Err(RunError::Sim(e)) => assert_eq!(e.kind, SimErrorKind::Cancelled, "{e}"),
        other => panic!("{other:?}"),
    }
    assert!(t0.elapsed() < Duration::from_secs(5));

    // an open workload with inter-arrival time 0: process limit (was: 2.8 GB before the
    // livelock guard)
    let (mut fs, spec) = files("h05_open_workload");
    replace(
        &mut fs,
        ".usagemodel",
        r#"interArrivalTime_OpenWorkload specification="Exp(4.0)""#,
        r#"interArrivalTime_OpenWorkload specification="0""#,
    );
    let limits = Limits {
        max_processes: 10_000,
        ..Limits::default()
    };
    expect_limit(&fs, &spec, limits, "Limits::max_processes");

    // cancellation flag
    let (fs, spec) = files("h01_ps_single");
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let limits = Limits {
        cancel: Some(cancel),
        ..Limits::default()
    };
    match simulate_memory(&fs, &spec, limits) {
        Err(RunError::Sim(e)) => assert_eq!(e.kind, SimErrorKind::Cancelled),
        other => panic!("{other:?}"),
    }
}

#[test]
fn limits_do_not_change_results_and_memory_equals_directory() {
    for name in [
        "h01_ps_single",
        "h27_collection_inner_multi",
        "x_sl_mediastore",
    ] {
        let dir = corpus(name);
        let spec = RunSpec::load(&dir.join("run.json")).unwrap();
        let cm = CompiledModel::compile(spec.load_model(&dir).unwrap()).unwrap();
        let run = |limits: Limits| {
            let cfg = SimConfig {
                limits,
                ..spec.sim_config("")
            };
            Simulation::new(&cm, cfg, Outputs::default())
                .unwrap()
                .run()
                .unwrap()
        };
        let unlimited = run(Limits::unlimited());
        let tight = run(Limits {
            max_events: unlimited.events,
            max_steps: u64::MAX / 2,
            deadline: Some(Instant::now() + Duration::from_secs(600)),
            cancel: Some(Default::default()),
            ..Limits::default()
        });
        assert_eq!(unlimited.measurements.to_csv(), tight.measurements.to_csv());
        let (fs, _) = files(name);
        let mem = simulate_memory(&fs, &spec, Limits::default()).unwrap();
        assert_eq!(unlimited.measurements.to_csv(), mem.measurements.to_csv());
        let s = mem.measurements.summaries();
        assert!(!s.is_empty());
        for x in &s {
            assert!(
                x.count > 0 && x.min <= x.p50 && x.p50 <= x.p99 && x.p99 <= x.max,
                "{x:?}"
            );
        }
    }
}

#[test]
fn memory_loading_never_reads_the_file_system() {
    let (mut fs, spec) = files("h01_ps_single");
    // an href to an existing file outside the given files must not be followed
    let target = corpus("h01_ps_single").join("h01_ps_single.repository");
    let f = fs.iter_mut().find(|f| f.0.ends_with(".system")).unwrap();
    f.1 = f.1.replace(
        "h01_ps_single.repository#",
        &format!("{}#", target.display()),
    );
    fs.retain(|f| !f.0.ends_with(".repository"));
    assert!(matches!(
        simulate_memory(&fs, &spec, Limits::default()),
        Err(RunError::Load(_))
    ));
}

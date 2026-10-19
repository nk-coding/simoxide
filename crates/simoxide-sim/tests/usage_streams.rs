//! Usage streams (`SimConfig::usage_streams`, `docs/spec/random.md` RND-11): common random
//! numbers across models that share a usage model.
//!
//! Checked on corpus model h22 (a usage behaviour with a delay, a branch and a loop; closed, and
//! changed in memory to open) against a second "design" with other, stochastic demands:
//! * with usage streams, every user (by its process id) draws the same delays, think times,
//!   branch choices and loop counts in both designs, and the open workload's arrivals are the
//!   same; with one stream per run they are not;
//! * a load profile keeps them the same;
//! * the difference between two designs varies much less over seeds, without bias;
//! * paused, forked and snapshotted runs continue like the uninterrupted run;
//! * the exact mode and the Slingshot-exact engine refuse usage streams.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use simoxide_sim::snapshot::{Encoding, SnapshotOptions};
use simoxide_sim::{
    CompiledModel, Fast, FlavourConfig, LoadProfile, Mode, Outputs, Pause, RunSpec, SimConfig,
    SimuLizar, Simulation, Slingshot,
};

const S: i64 = 1_000_000_000;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn files(name: &str, edits: &[(&str, &str)]) -> Vec<(String, String)> {
    let dir = root().join("corpus").join(name);
    let mut v = Vec::new();
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = e.path();
        if !p.is_file() {
            continue;
        }
        let n = p.file_name().unwrap().to_string_lossy().into_owned();
        if n.ends_with(".json") || n.ends_with(".txt") {
            continue;
        }
        let mut t = std::fs::read_to_string(&p).unwrap();
        for (a, b) in edits {
            assert!(n.contains('.'));
            t = t.replace(a, b);
        }
        v.push((n, t));
    }
    v
}

fn compile(edits: &[(&str, &str)], flavour: FlavourConfig) -> CompiledModel {
    let m = RunSpec::default()
        .load_model_memory(&files("h22_usage_behaviour", edits))
        .unwrap();
    CompiledModel::compile_flavour(m, flavour).unwrap()
}

const OPEN: (&str, &str) = (
    r#"<workload_UsageScenario xsi:type="usagemodel:ClosedWorkload" population="3">
      <thinkTime_ClosedWorkload specification="Exp(1.0)"/>"#,
    r#"<workload_UsageScenario xsi:type="usagemodel:OpenWorkload">
      <interArrivalTime_OpenWorkload specification="Exp(1.5)"/>"#,
);
/// The other design: other demands, one more draw per call.
const DESIGN_B: [(&str, &str); 2] = [
    (
        r#"specification="Exp(10.0)""#,
        r#"specification="Exp(3.0)""#,
    ),
    (r#"specification="0.02""#, r#"specification="Exp(40.0)""#),
];

fn model(open: bool, b: bool, flavour: FlavourConfig) -> CompiledModel {
    let mut e = Vec::new();
    if open {
        e.push(OPEN);
    }
    if b {
        e.extend(DESIGN_B);
    }
    compile(&e, flavour)
}

fn cfg(streams: bool, until_s: i64) -> SimConfig {
    SimConfig {
        seed: 7,
        max_sim_time: until_s,
        mode: Mode::Fast,
        usage_streams: streams,
        ..Default::default()
    }
}

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

fn trace_of<F: simoxide_sim::Flavour>(cm: &CompiledModel, c: SimConfig) -> String {
    let t = Buf::default();
    let out = Outputs {
        trace: Some(Box::new(t.clone())),
        tape: None,
    };
    Simulation::<Fast, F>::create(cm, c, out)
        .unwrap()
        .finish()
        .unwrap();
    String::from_utf8(t.0.borrow().clone()).unwrap()
}

/// Per process id: what the usage model drew, in order (holds of delays, think times and
/// inter-arrival times; branch choices; loop counts), from the trace. The system of h22 holds
/// nothing (its demands are resource demands), so every hold is the usage model's.
fn usage_draws(trace: &str) -> BTreeMap<u64, Vec<String>> {
    let mut m: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    for line in trace.lines() {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        let (Some(ev), Some(p)) = (v["ev"].as_str(), v["p"].as_u64()) else {
            continue;
        };
        let x = match ev {
            "hold" => format!("hold {}", v["d"]),
            "branch" => format!("branch {}", v["idx"]),
            "loop" => format!("loop {}", v["n"]),
            _ => continue,
        };
        m.entry(p).or_default().push(x);
    }
    m
}

/// `a` and `b` agree on every process: one list is a prefix of the other (a run that ends
/// earlier sees fewer draws). Returns the number of draws compared.
fn agree(a: &BTreeMap<u64, Vec<String>>, b: &BTreeMap<u64, Vec<String>>) -> Result<usize, String> {
    let mut n = 0;
    for (p, x) in a {
        let Some(y) = b.get(p) else { continue };
        let k = x.len().min(y.len());
        if x[..k] != y[..k] {
            let i = (0..k).find(|&i| x[i] != y[i]).unwrap();
            return Err(format!("process {p}, draw {i}: {} vs {}", x[i], y[i]));
        }
        n += k;
    }
    Ok(n)
}

fn check_common<F: simoxide_sim::Flavour>(open: bool, flavour: FlavourConfig, lp: bool) {
    let (a, b) = (model(open, false, flavour), model(open, true, flavour));
    let mut c = cfg(true, 300);
    if lp {
        c.load_profile = Some(Arc::new(LoadProfile::uniform(vec![
            (0.0, 0.5),
            (60.0, 2.0),
            (150.0, 1.0),
        ])));
    }
    let (ta, tb) = (trace_of::<F>(&a, c.clone()), trace_of::<F>(&b, c.clone()));
    assert_ne!(ta, tb, "the designs differ");
    let (da, db) = (usage_draws(&ta), usage_draws(&tb));
    let n = agree(&da, &db).unwrap();
    assert!(n > 200, "only {n} draws compared");
    // without usage streams the same seed gives other users
    c.usage_streams = false;
    let (ua, ub) = (
        usage_draws(&trace_of::<F>(&a, c.clone())),
        usage_draws(&trace_of::<F>(&b, c)),
    );
    assert!(
        agree(&ua, &ub).is_err(),
        "one stream per run desynchronises"
    );
}

#[test]
fn closed_users_draw_the_same_in_both_designs() {
    check_common::<SimuLizar>(false, FlavourConfig::SIMULIZAR, false);
}

#[test]
fn open_arrivals_and_users_are_the_same_in_both_designs() {
    check_common::<SimuLizar>(true, FlavourConfig::SIMULIZAR, false);
}

#[test]
fn with_a_load_profile_too() {
    check_common::<SimuLizar>(true, FlavourConfig::SIMULIZAR, true);
    check_common::<SimuLizar>(false, FlavourConfig::SIMULIZAR, true);
}

#[test]
fn in_the_slingshot_flavour_too() {
    check_common::<Slingshot>(true, FlavourConfig::SLINGSHOT, false);
}

#[test]
fn other_seeds_give_other_users() {
    let a = model(true, false, FlavourConfig::SIMULIZAR);
    let x = usage_draws(&trace_of::<SimuLizar>(&a, cfg(true, 100)));
    let mut c = cfg(true, 100);
    c.seed = 8;
    let y = usage_draws(&trace_of::<SimuLizar>(&a, c));
    assert!(agree(&x, &y).is_err());
}

#[test]
fn paused_forked_and_snapshotted_runs_continue_identically() {
    let cm = model(true, true, FlavourConfig::SIMULIZAR);
    let mut c = cfg(true, 120);
    c.load_profile = Some(Arc::new(LoadProfile::uniform(vec![
        (0.0, 1.0),
        (40.0, 2.5),
    ])));
    let fp = |r: simoxide_sim::RunResult| {
        format!(
            "end {} main {} draws {} events {}\n{}",
            r.end_ns,
            r.main_count,
            r.uniforms,
            r.events,
            r.measurements.to_csv()
        )
    };
    let want = fp(
        Simulation::<Fast>::create(&cm, c.clone(), Outputs::default())
            .unwrap()
            .finish()
            .unwrap(),
    );
    for t in [3 * S, 40 * S, 77 * S] {
        for how in ["direct", "fork", "postcard", "json"] {
            let mut sim = Simulation::<Fast>::create(&cm, c.clone(), Outputs::default()).unwrap();
            sim.run_until(Pause::Before(t)).unwrap();
            let cont = match how {
                "direct" => sim,
                "fork" => sim.fork(Outputs::default()).unwrap(),
                enc => {
                    let encoding = if enc == "json" {
                        Encoding::Json
                    } else {
                        Encoding::Postcard
                    };
                    let mut bytes = Vec::new();
                    sim.snapshot_with(&mut bytes, &SnapshotOptions { encoding }, Vec::new())
                        .unwrap();
                    Simulation::<Fast>::restore_from(&bytes[..], &cm, Outputs::default()).unwrap()
                }
            };
            assert_eq!(fp(cont.finish().unwrap()), want, "pause at {t} ns, {how}");
        }
    }
}

#[test]
fn exact_mode_and_slingshot_exact_refuse() {
    let cm = model(true, false, FlavourConfig::SIMULIZAR);
    let mut c = cfg(true, 10);
    c.mode = Mode::Exact;
    let e = simoxide_sim::run(&cm, c, Outputs::default()).unwrap_err();
    assert!(e.message.contains("usage streams"), "{}", e.message);
    let ss = model(true, false, FlavourConfig::slingshot_exact());
    let e = simoxide_sim::run(&ss, cfg(true, 10), Outputs::default()).unwrap_err();
    assert!(e.message.contains("usage streams"), "{}", e.message);
}

#[test]
fn off_by_default_and_unchanged() {
    let cm = model(true, false, FlavourConfig::SIMULIZAR);
    assert!(!SimConfig::default().usage_streams);
    // a run with usage streams differs from one without (the usage model draws elsewhere), and
    // two runs with them are identical
    let a = trace_of::<SimuLizar>(&cm, cfg(true, 60));
    assert_eq!(a, trace_of::<SimuLizar>(&cm, cfg(true, 60)));
    assert_ne!(a, trace_of::<SimuLizar>(&cm, cfg(false, 60)));
}

/// Mean of the usage scenario's response times of a run.
fn mean_rt(cm: &CompiledModel, c: SimConfig) -> f64 {
    let r = Simulation::<Fast>::create(cm, c, Outputs::default())
        .unwrap()
        .finish()
        .unwrap();
    let i = r
        .measurements
        .series
        .iter()
        .position(|s| s.mp.starts_with("UsageScenario") && s.metric.starts_with("Response Time"))
        .expect("a usage-scenario response-time series");
    let rows = &r.measurements.rows[i];
    rows.iter().map(|x| x.1).sum::<f64>() / rows.len() as f64
}

/// What the streams are for: the difference between two designs, estimated from one run of
/// each with the same seed, varies much less over seeds (fixed seeds: deterministic). The second
/// design has 25 % more demand on the write operation.
#[test]
fn the_difference_between_designs_varies_less() {
    let a = model(true, false, FlavourConfig::SIMULIZAR);
    let b = compile(
        &[
            OPEN,
            (
                r#"specification="Exp(10.0)""#,
                r#"specification="Exp(8.0)""#,
            ),
        ],
        FlavourConfig::SIMULIZAR,
    );
    let var_of_diff = |streams: bool| {
        let d: Vec<f64> = (1..=24)
            .map(|seed| {
                let mut c = cfg(streams, 200);
                c.seed = seed;
                mean_rt(&a, c.clone()) - mean_rt(&b, c)
            })
            .collect();
        let m = d.iter().sum::<f64>() / d.len() as f64;
        d.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (d.len() - 1) as f64
    };
    let (with, without) = (var_of_diff(true), var_of_diff(false));
    eprintln!("variance of the difference: {with:.3e} with usage streams, {without:.3e} without");
    assert!(with < 0.2 * without, "{with} vs {without}");
}

/// No bias: the mean over seeds of a run's mean response time is the same with and without
/// usage streams (within the sampling error).
#[test]
#[ignore = "statistical check, about 10 s in release"]
fn no_bias() {
    let b = model(true, true, FlavourConfig::SIMULIZAR);
    let stats = |streams: bool| {
        let v: Vec<f64> = (1..=200)
            .map(|seed| {
                let mut c = cfg(streams, 200);
                c.seed = seed;
                mean_rt(&b, c)
            })
            .collect();
        let m = v.iter().sum::<f64>() / v.len() as f64;
        let var = v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (v.len() - 1) as f64;
        (m, (var / v.len() as f64).sqrt())
    };
    let ((m1, s1), (m0, s0)) = (stats(true), stats(false));
    eprintln!("mean {m1:.4} ± {s1:.4} with usage streams, {m0:.4} ± {s0:.4} without");
    assert!((m1 - m0).abs() < 3.0 * (s1 * s1 + s0 * s0).sqrt());
}

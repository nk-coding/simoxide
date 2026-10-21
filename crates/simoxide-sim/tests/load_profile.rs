//! Load profiles (`simoxide_sim::load`, `docs/spec/workloads.md` LP-1 to LP-6): a step function
//! of a factor per usage scenario, on an open workload's arrival rate (in operational time: a
//! pending arrival moves at a step) or a closed workload's population.
//!
//! Checked on small corpus models with their workloads changed in memory:
//! * a profile of factor 1 changes no output (only the step events count);
//! * a constant factor `f` on `Exp(λ)` equals the model with `Exp(f·λ)`, byte for byte;
//! * with a constant inter-arrival time, every arrival lands where the time change puts it, at
//!   steps up and down, in both flavours;
//! * a closed population follows the profile (users start at a step, surplus users leave after
//!   their run);
//! * paused, forked and snapshotted runs continue like the uninterrupted run;
//! * the profile's errors and the Slingshot-exact engine's refusal.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use simoxide_sim::snapshot::{Encoding, SnapshotOptions};
use simoxide_sim::{
    Compat, CompiledModel, Exact, Fast, Flavour, FlavourConfig, LoadProfile, Mode, Outputs, Pause,
    RunResult, RunSpec, SimConfig, SimuLizar, Simulation, Slingshot,
};

const S: i64 = 1_000_000_000;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The files of corpus model `name`, with each `(from, to)` replaced in every file.
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
            t = t.replace(a, b);
        }
        v.push((n, t));
    }
    v
}

fn compile(name: &str, edits: &[(&str, &str)], flavour: FlavourConfig) -> CompiledModel {
    let m = RunSpec::default()
        .load_model_memory(&files(name, edits))
        .unwrap();
    CompiledModel::compile_flavour(m, flavour).unwrap()
}

fn cfg(mode: Mode, until_s: i64, profile: Option<LoadProfile>) -> SimConfig {
    SimConfig {
        seed: 3,
        max_sim_time: until_s,
        mode,
        load_profile: profile.map(Arc::new),
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

impl Buf {
    fn text(&self) -> String {
        String::from_utf8(self.0.borrow().clone()).unwrap()
    }
}

/// Result fingerprint (without the event count), trace and tape of a run.
fn traced<C: Compat, F: Flavour>(cm: &CompiledModel, cfg: SimConfig) -> (String, u64, String) {
    let (t, u) = (Buf::default(), Buf::default());
    let out = Outputs {
        trace: Some(Box::new(t.clone())),
        tape: C::MODE
            .eq(&Mode::Exact)
            .then(|| Box::new(u.clone()) as Box<dyn std::io::Write>),
    };
    let r = Simulation::<C, F>::create(cm, cfg, out)
        .unwrap()
        .finish()
        .unwrap();
    (
        format!(
            "end {} main {} draws {}\n{}",
            r.end_ns,
            r.main_count,
            r.uniforms,
            r.measurements.to_csv()
        ),
        r.events,
        format!("{}{}", t.text(), u.text()),
    )
}

fn run<C: Compat, F: Flavour>(cm: &CompiledModel, cfg: SimConfig) -> RunResult {
    Simulation::<C, F>::create(cm, cfg, Outputs::default())
        .unwrap()
        .finish()
        .unwrap()
}

/// The rows of the usage scenario's response-time series.
fn scenario_rows(r: &RunResult) -> Vec<(f64, f64)> {
    let i = r
        .measurements
        .series
        .iter()
        .position(|s| s.mp.starts_with("UsageScenario") && s.metric.starts_with("Response Time"))
        .expect("a usage-scenario response-time series");
    r.measurements.rows[i].clone()
}

/// The arrival times of the scenario's users (completion time minus response time), rounded
/// to microseconds.
fn arrivals(r: &RunResult) -> Vec<f64> {
    scenario_rows(r)
        .iter()
        .map(|(t, v)| ((t - v) * 1e6).round() / 1e6)
        .collect()
}

#[test]
fn a_unit_profile_changes_nothing() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    for (name, edits) in [
        ("h05_open_workload", &[][..]),
        ("h02_ps_ties", &[][..]),
        ("x_ss_minimalexample_open_ps", &[][..]),
    ] {
        let cm = compile(name, edits, FlavourConfig::SIMULIZAR);
        let unit = LoadProfile::uniform(vec![(0.0, 1.0), (7.5, 1.0), (20.0, 1.0)]);
        let (a, ea, ta) = traced::<Exact, SimuLizar>(&cm, cfg(Mode::Exact, 60, None));
        let (b, eb, tb) = traced::<Exact, SimuLizar>(&cm, cfg(Mode::Exact, 60, Some(unit)));
        assert_eq!(a, b, "{name}: result");
        assert_eq!(ta, tb, "{name}: trace and tape");
        // the two steps after the start are events
        assert_eq!(eb, ea + 2, "{name}: events");
    }
}

#[test]
fn a_constant_factor_equals_the_scaled_rate() {
    let base = compile("h05_open_workload", &[], FlavourConfig::SIMULIZAR);
    let fast = compile(
        "h05_open_workload",
        &[("Exp(4.0)", "Exp(8.0)")],
        FlavourConfig::SIMULIZAR,
    );
    let double = LoadProfile::uniform(vec![(0.0, 2.0)]);
    let (a, ea, _) = traced::<Exact, SimuLizar>(&fast, cfg(Mode::Exact, 100, None));
    let (b, eb, _) = traced::<Exact, SimuLizar>(&base, cfg(Mode::Exact, 100, Some(double)));
    // (the traces differ in the tape's specification strings only)
    assert_eq!(a, b);
    assert_eq!(ea, eb);
}

/// The model with a constant inter-arrival time of 100 s and no user delay.
fn constant_arrivals(flavour: FlavourConfig) -> CompiledModel {
    compile(
        "h05_open_workload",
        &[
            ("Exp(4.0)", "100.0"),
            (
                "<timeSpecification_Delay specification=\"0.5\"/>",
                "<timeSpecification_Delay specification=\"0.0\"/>",
            ),
        ],
        flavour,
    )
}

fn check_arrivals<C: Compat, F: Flavour>(cm: &CompiledModel, mode: Mode) {
    // up: at 50 s the rest of the interval (50 s) passes ten times as fast
    let up = LoadProfile::uniform(vec![(50.0, 10.0)]);
    let r = run::<C, F>(cm, cfg(mode, 100, Some(up)));
    assert_eq!(arrivals(&r), vec![0.0, 55.0, 65.0, 75.0, 85.0, 95.0]);
    // down and up again: at 50 s, half the rate (the remaining 50 s become 100 s, due at
    // 150 s); at 120 s, the rate again (the remaining 30 s at half rate are 15 s: 135 s)
    let both = LoadProfile::uniform(vec![(50.0, 0.5), (120.0, 1.0)]);
    let r = run::<C, F>(cm, cfg(mode, 300, Some(both)));
    assert_eq!(arrivals(&r), vec![0.0, 135.0, 235.0]);
    // a step at an arrival's own time: the arrival is due now and draws at the new rate
    let at = LoadProfile::uniform(vec![(100.0, 4.0)]);
    let r = run::<C, F>(cm, cfg(mode, 200, Some(at)));
    assert_eq!(arrivals(&r), vec![0.0, 100.0, 125.0, 150.0, 175.0]);
}

#[test]
fn a_step_moves_the_pending_arrival() {
    let cm = constant_arrivals(FlavourConfig::SIMULIZAR);
    check_arrivals::<Exact, SimuLizar>(&cm, Mode::Exact);
    check_arrivals::<Fast, SimuLizar>(&cm, Mode::Fast);
    let ss = constant_arrivals(FlavourConfig::slingshot(Default::default()));
    check_arrivals::<Fast, Slingshot>(&ss, Mode::Fast);
}

#[test]
fn the_rate_follows_the_profile() {
    // Exp(4.0): 4 arrivals/s; 1x for 200 s, 3x for 200 s, 0.5x for 200 s (on a server fast
    // enough that every request completes soon after its arrival)
    let cm = compile(
        "h05_open_workload",
        &[(
            "<processingRate_ProcessingResourceSpecification specification=\"1.0\"/>",
            "<processingRate_ProcessingResourceSpecification specification=\"100.0\"/>",
        )],
        FlavourConfig::SIMULIZAR,
    );
    let p = LoadProfile::uniform(vec![(200.0, 3.0), (400.0, 0.5)]);
    for mode in [Mode::Exact, Mode::Fast] {
        let r = match mode {
            Mode::Exact => run::<Exact, SimuLizar>(&cm, cfg(mode, 600, Some(p.clone()))),
            Mode::Fast => run::<Fast, SimuLizar>(&cm, cfg(mode, 600, Some(p.clone()))),
        };
        let a = arrivals(&r);
        let count = |lo: f64, hi: f64| a.iter().filter(|&&t| t >= lo && t < hi).count() as f64;
        for (lo, hi, rate) in [(0.0, 200.0, 4.0), (200.0, 400.0, 12.0), (400.0, 590.0, 2.0)] {
            let want = rate * (hi - lo);
            let got = count(lo, hi);
            // Poisson: within 4 standard deviations
            assert!(
                (got - want).abs() < 4.0 * want.sqrt(),
                "{mode:?} [{lo}, {hi}): {got} arrivals, expected about {want}"
            );
        }
    }
}

#[test]
fn a_closed_population_follows_the_profile() {
    // three users, think time 1 s; the demand (1 ms) is small next to it
    let cm = compile(
        "h02_ps_ties",
        &[(
            "<processingRate_ProcessingResourceSpecification specification=\"2.0\"/>",
            "<processingRate_ProcessingResourceSpecification specification=\"1000.0\"/>",
        )],
        FlavourConfig::SIMULIZAR,
    );
    let p = LoadProfile::uniform(vec![(0.0, 2.0), (100.0, 1.0), (200.0, 3.0)]);
    let mut sim =
        Simulation::<Fast>::create(&cm, cfg(Mode::Fast, 300, Some(p)), Outputs::default()).unwrap();
    sim.run_until(Pause::After(50 * S)).unwrap();
    assert_eq!(sim.state().load_factor(0), Some(2.0));
    // six users and no other process (the closed workload has no generator)
    assert_eq!(sim.state().live_processes(), 6);
    sim.run_until(Pause::After(150 * S)).unwrap();
    assert_eq!(sim.state().live_processes(), 3);
    sim.run_until(Pause::After(250 * S)).unwrap();
    assert_eq!(sim.state().live_processes(), 9);
    let r = sim.finish().unwrap();
    let rows = scenario_rows(&r);
    let per = |lo: f64, hi: f64| rows.iter().filter(|x| x.0 >= lo && x.0 < hi).count() as f64;
    let (a, b, c) = (per(20.0, 100.0), per(120.0, 200.0), per(220.0, 300.0));
    // completions per second scale with the users (6 : 3 : 9)
    assert!((a / b - 2.0).abs() < 0.1, "{a} {b}");
    assert!((c / b - 3.0).abs() < 0.15, "{c} {b}");
}

#[test]
fn paused_forked_and_snapshotted_runs_continue_identically() {
    let cm = compile("h05_open_workload", &[], FlavourConfig::SIMULIZAR);
    let p = LoadProfile::uniform(vec![(0.0, 0.5), (20.0, 3.0), (40.0, 1.5), (60.0, 0.25)]);
    let c = cfg(Mode::Exact, 90, Some(p));
    let (want, events, trace) = traced::<Exact, SimuLizar>(&cm, c.clone());
    // pause before, at and between steps; continue directly, as a fork and from a snapshot
    for t in [5 * S, 20 * S, 20 * S + 1, 33 * S, 59 * S, 61 * S] {
        for how in ["direct", "fork", "postcard", "json"] {
            let (t1, u1) = (Buf::default(), Buf::default());
            let mut sim = Simulation::<Exact>::create(
                &cm,
                c.clone(),
                Outputs {
                    trace: Some(Box::new(t1.clone())),
                    tape: Some(Box::new(u1.clone())),
                },
            )
            .unwrap();
            sim.run_until(Pause::Before(t)).unwrap();
            let (t2, u2) = (Buf::default(), Buf::default());
            let out2 = Outputs {
                trace: Some(Box::new(t2.clone())),
                tape: Some(Box::new(u2.clone())),
            };
            let cont = match how {
                "direct" => sim,
                "fork" => sim.fork(out2).unwrap(),
                enc => {
                    let encoding = if enc == "json" {
                        Encoding::Json
                    } else {
                        Encoding::Postcard
                    };
                    let mut bytes = Vec::new();
                    sim.snapshot_with(&mut bytes, &SnapshotOptions { encoding }, Vec::new())
                        .unwrap();
                    Simulation::<Exact>::restore_from(&bytes[..], &cm, out2).unwrap()
                }
            };
            let r = cont.finish().unwrap();
            let got = format!(
                "end {} main {} draws {}\n{}",
                r.end_ns,
                r.main_count,
                r.uniforms,
                r.measurements.to_csv()
            );
            assert_eq!(got, want, "pause at {t} ns, {how}");
            assert_eq!(r.events, events, "pause at {t} ns, {how}: events");
            if how == "direct" {
                assert_eq!(
                    format!("{}{}", t1.text(), u1.text()),
                    trace,
                    "pause at {t} ns: trace and tape"
                );
            }
            let _ = (t2, u2);
        }
    }
}

#[test]
fn profile_errors() {
    let cm = compile("h05_open_workload", &[], FlavourConfig::SIMULIZAR);
    let named = |n: &str| {
        LoadProfile::from_json(&format!(r#"{{"scenarios":["{n}"],"points":[[0,2]]}}"#)).unwrap()
    };
    // by name and by id
    for n in ["Scenario", "_h05_open_workload_us1"] {
        let s = Simulation::<Fast>::create(
            &cm,
            cfg(Mode::Fast, 10, Some(named(n))),
            Outputs::default(),
        )
        .unwrap();
        assert_eq!(s.state().load_factor(0), Some(2.0));
    }
    let e = Simulation::<Fast>::create(
        &cm,
        cfg(Mode::Fast, 10, Some(named("nope"))),
        Outputs::default(),
    )
    .err()
    .unwrap();
    assert!(e.message.contains("no usage scenario"), "{}", e.message);
    // an invalid profile built in code is refused at the start too
    let bad = LoadProfile::uniform(vec![(5.0, 1.0), (5.0, 2.0)]);
    let e = Simulation::<Fast>::create(&cm, cfg(Mode::Fast, 10, Some(bad)), Outputs::default())
        .err()
        .unwrap();
    assert!(e.message.contains("increase"), "{}", e.message);
    // the Slingshot-exact engine has no profiles
    let ssx = compile("h05_open_workload", &[], FlavourConfig::slingshot_exact());
    let e = simoxide_sim::run(
        &ssx,
        cfg(Mode::Fast, 10, Some(LoadProfile::uniform(vec![(0.0, 2.0)]))),
        Outputs::default(),
    )
    .err()
    .unwrap();
    assert!(e.message.contains("load profile"), "{}", e.message);
    // a model directory's profile file
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("lp-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert_eq!(LoadProfile::load_dir(&dir).unwrap(), None);
    std::fs::write(dir.join("a.loadprofile.json"), r#"{"points":[[0,2]]}"#).unwrap();
    std::fs::write(
        dir.join("b.loadprofile.json"),
        r#"{"scenarios":"x","points":[[1,3]]}"#,
    )
    .unwrap();
    let p = LoadProfile::load_dir(&dir).unwrap().unwrap();
    assert_eq!(p.entries.len(), 2);
    assert_eq!(p.entries[1].scenarios, vec!["x".to_string()]);
}

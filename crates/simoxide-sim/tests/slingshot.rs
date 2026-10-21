//! The Slingshot flavour (fast mode, `docs/spec/slingshot/`) against the Slingshot reference
//! runner's `det` outputs:
//!
//! * the probes of the specification (`reference/slingshot/models/probes/*/expected/`);
//! * the Slingshot corpus models `x_ss_*` (`reference/slingshot/results/s2/`, made by
//!   `reference/slingshot/run.sh -seed N` with the simulated time given there).
//!
//! With the compat flags the fast mode can reproduce (`Quirks::all_fast`, CLI `--quirks fast`)
//! and the exact RNG mode (`java.util.Random` branches, MT19937 StoEx), the measurements are
//! byte-identical wherever the same-time order of the fast mode's processes and Slingshot's
//! breadth-first event cascade agree; `x_espresso` (30 users, all ties) is compared
//! statistically instead (`docs/correctness/slingshot.md`).

use simoxide_sim::{
    CompiledModel, FlavourConfig, Mode, Outputs, Pause, Quirk, Quirks, RunSpec, SimConfig,
    Simulation, Slingshot,
};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn compile(dir: &Path, quirks: Quirks) -> CompiledModel {
    let model = RunSpec::default().load_model(dir).expect("model loads");
    CompiledModel::compile_flavour(model, FlavourConfig::slingshot(quirks)).expect("compiles")
}

fn config(seed: i64, sim_time: i64, mode: Mode) -> SimConfig {
    SimConfig {
        seed,
        max_sim_time: sim_time,
        max_measurements: -1,
        mode,
        ..Default::default()
    }
}

fn run_csv(dir: &Path, quirks: Quirks, seed: i64, sim_time: i64) -> Result<String, String> {
    let cm = compile(dir, quirks);
    simoxide_sim::run(&cm, config(seed, sim_time, Mode::Exact), Outputs::default())
        .map(|r| r.measurements.to_csv())
        .map_err(|e| e.to_string())
}

struct Probe {
    dir: PathBuf,
    name: String,
    seed: i64,
    sim_time: i64,
    /// Slingshot handler errors in the reference run.
    errors: bool,
}

fn probes() -> Vec<Probe> {
    let base = root().join("reference/slingshot/models/probes");
    let mut v: Vec<Probe> = std::fs::read_dir(&base)
        .expect("probes")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("probe.json").exists())
        .map(|dir| {
            let j: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(dir.join("probe.json")).unwrap())
                    .unwrap();
            let errors = std::fs::read_to_string(dir.join("expected/errors.txt"))
                .is_ok_and(|e| !e.trim().is_empty());
            Probe {
                name: dir.file_name().unwrap().to_string_lossy().into_owned(),
                seed: j["seed"].as_i64().unwrap(),
                sim_time: j["sim_time"].as_i64().unwrap(),
                dir,
                errors,
            }
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// Every probe without a Slingshot handler error is reproduced byte for byte with the compat
/// flags; a probe whose Slingshot handler throws stops with an error where Slingshot throws
/// (the disposed handler, Q18, is the exact mode's), except `ss_two_demands`, whose quirk Q17
/// only the exact mode reproduces (the fast mode serves the demands one after the other).
#[test]
fn probes_equal_the_det_reference() {
    let ps = probes();
    assert_eq!(ps.len(), 20);
    let mut identical = 0;
    for p in &ps {
        let r = run_csv(&p.dir, Quirks::all_fast(), p.seed, p.sim_time);
        let expected = std::fs::read_to_string(p.dir.join("expected/measurements.csv")).unwrap();
        match (&r, p.errors) {
            (Ok(csv), false) => {
                assert_eq!(csv, &expected, "{}: measurements differ", p.name);
                identical += 1;
            }
            (Err(e), true) => assert!(
                e.contains("Slingshot") || e.contains("No branch"),
                "{}: {e}",
                p.name
            ),
            (Ok(_), true) => assert_eq!(p.name, "ss_two_demands"),
            (Err(e), false) => panic!("{}: {e}", p.name),
        }
    }
    assert_eq!(identical, 16);
}

/// The quirk of `ss_two_demands` is refused, not approximated.
#[test]
fn exact_only_quirks_are_refused() {
    let dir = root().join("reference/slingshot/models/probes/ss_two_demands");
    let e = run_csv(&dir, Quirks::all_fast().with(Quirk::Q17), 1, 12).unwrap_err();
    assert!(e.contains("Q17") && e.contains("X2"), "{e}");
    let e = run_csv(&dir, Quirks::NONE.with(Quirk::Q18), 1, 12).unwrap_err();
    assert!(e.contains("Q18"), "{e}");
}

/// The x_ss corpus models: byte-identical to `det` with the compat flags (seeds 1-3).
#[test]
fn x_ss_models_equal_the_det_reference() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let res = root().join("reference/slingshot/results/s2");
    let mut n = 0;
    for (model, sim_time) in [
        ("x_ss_minimal", 1000),
        ("x_ss_delay", 1000),
        ("x_ss_minimalexample", 1000),
        ("x_ss_minimalexample_open_ps", 1000),
        ("x_ss_mediastore", 200_000),
    ] {
        for seed in 1..=3 {
            let path = res.join(format!("{model}/seed{seed}/measurements.csv"));
            let Some(expected) = simoxide_testkit::corpus::read_maybe_gz(&path).unwrap() else {
                panic!("missing {}", path.display());
            };
            let got = run_csv(
                &root().join("corpus").join(model),
                Quirks::all_fast(),
                seed,
                sim_time,
            )
            .unwrap();
            assert_eq!(got, expected, "{model} seed {seed}");
            n += 1;
        }
    }
    assert_eq!(n, 15);
}

/// The fixed behaviour differs from Slingshot exactly where a quirk is documented.
#[test]
fn fixed_behaviour_differs_where_documented() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let probe = |n: &str| root().join("reference/slingshot/models/probes").join(n);
    // Q29: entry-level call and system-operation monitors are honoured
    let fixed = run_csv(&probe("ss_monitors"), Quirks::NONE, 1, 4).unwrap();
    assert!(fixed.contains("EntryLevelSystemCallMeasuringPoint"));
    assert!(fixed.contains("SystemOperationMeasuringPoint"));
    let compat = run_csv(&probe("ss_monitors"), Quirks::NONE.with(Quirk::Q29), 1, 4).unwrap();
    assert!(!compat.contains("EntryLevelSystemCallMeasuringPoint"));
    // Q26: no samples of stale processor-sharing completions
    let rows = |s: &str| s.lines().filter(|l| l.contains("State of Active")).count();
    let fixed = run_csv(&probe("ss_fifo_ties"), Quirks::NONE, 1, 7).unwrap();
    let compat = run_csv(&probe("ss_fifo_ties"), Quirks::NONE.with(Quirk::Q26), 1, 7).unwrap();
    assert!(
        rows(&fixed) < rows(&compat),
        "{} {}",
        rows(&fixed),
        rows(&compat)
    );
    // Q34: per-core state of a 2-core resource instead of the total at every replica
    let fixed = run_csv(&probe("ss_ps_multicore"), Quirks::NONE, 1, 6).unwrap();
    assert!(
        fixed
            .lines()
            .all(|l| !l.contains("State of Active") || !l.ends_with(",3.0"))
    );
    // Q25: a delay resource works instead of aborting every request
    let fixed = run_csv(&probe("ss_delay_resource"), Quirks::NONE, 1, 4).unwrap();
    assert!(fixed.contains("Response Time"));
    let compat = run_csv(
        &probe("ss_delay_resource"),
        Quirks::NONE.with(Quirk::Q25),
        1,
        4,
    )
    .unwrap();
    assert!(!compat.contains("Response Time"));
    // Q19: a loop count of 0 skips the body instead of throwing
    assert!(run_csv(&probe("ss_loop_zero"), Quirks::NONE, 1, 8).is_ok());
    assert!(run_csv(&probe("ss_loop_zero"), Quirks::NONE.with(Quirk::Q19), 1, 8).is_err());
    // Q32: the maximum measurement count stops the run (fixed), or is ignored
    let dir = root().join("corpus/x_ss_minimal");
    let cm = compile(&dir, Quirks::NONE);
    let cfg = SimConfig {
        max_measurements: 10,
        ..config(1, 1000, Mode::Exact)
    };
    assert_eq!(
        simoxide_sim::run(&cm, cfg.clone(), Outputs::default())
            .unwrap()
            .main_count,
        10
    );
    let cm = compile(&dir, Quirks::NONE.with(Quirk::Q32));
    assert!(
        simoxide_sim::run(&cm, cfg, Outputs::default())
            .unwrap()
            .main_count
            > 400
    );
}

/// Pausing and forking a Slingshot run continue exactly like the uninterrupted run (in both
/// RNG modes): the run state includes the branch stream.
#[test]
fn slingshot_fork_equals_the_uninterrupted_run() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let dir = root().join("corpus/x_ss_minimalexample_open_ps");
    let cm = compile(&dir, Quirks::all_fast());
    for mode in [Mode::Exact, Mode::Fast] {
        if !mode.available() {
            continue;
        }
        let whole = simoxide_sim::run(&cm, config(5, 300, mode), Outputs::default())
            .unwrap()
            .measurements
            .to_csv();
        let (a, b) = match mode {
            Mode::Exact => {
                let mut sim = Simulation::<simoxide_sim::Exact, Slingshot>::create(
                    &cm,
                    config(5, 300, mode),
                    Outputs::default(),
                )
                .unwrap();
                sim.run_until(Pause::After(150_000_000_000)).unwrap();
                assert!(sim.state().now_ns() <= 150_000_000_000);
                let fork = sim.fork(Outputs::default()).unwrap();
                (sim.finish().unwrap(), fork.finish().unwrap())
            }
            #[cfg(feature = "fast")]
            Mode::Fast => {
                let mut sim = Simulation::<simoxide_sim::Fast, Slingshot>::create(
                    &cm,
                    config(5, 300, mode),
                    Outputs::default(),
                )
                .unwrap();
                sim.run_until(Pause::After(150_000_000_000)).unwrap();
                let fork = sim.fork(Outputs::default()).unwrap();
                (sim.finish().unwrap(), fork.finish().unwrap())
            }
            #[cfg(not(feature = "fast"))]
            Mode::Fast => unreachable!(),
        };
        assert_eq!(a.measurements.to_csv(), whole, "{mode}");
        assert_eq!(b.measurements.to_csv(), whole, "{mode}");
        assert_eq!(a.end_ns, 300_000_000_000);
    }
}

/// The measurement-observer hook sees exactly the recorded tuples of the observed series.
#[test]
fn observed_series_see_every_tuple() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let dir = root().join("corpus/x_ss_minimal");
    let cm = compile(&dir, Quirks::all_fast());
    let rt = cm
        .series
        .iter()
        .position(|s| s.mp.starts_with("UsageScenario"))
        .unwrap() as u32;
    let mut sim = Simulation::<simoxide_sim::Exact, Slingshot>::create(
        &cm,
        config(1, 100, Mode::Exact),
        Outputs::default(),
    )
    .unwrap();
    sim.observe_series(rt);
    let mut seen = Vec::new();
    while sim
        .run_until(Pause::Events(sim.state().events() + 50))
        .unwrap()
        == simoxide_sim::Status::Paused
    {
        seen.extend(sim.take_observed());
    }
    seen.extend(sim.take_observed());
    let r = sim.finish().unwrap();
    let rows = &r.measurements.rows[rt as usize];
    assert!(!rows.is_empty());
    assert_eq!(seen.len(), rows.len());
    assert!(
        seen.iter()
            .zip(rows)
            .all(|(o, &(t, v))| o.series == rt && o.t == t && o.v == v)
    );
}

/// Flavour and model must agree; quirk flags need the Slingshot flavour.
#[test]
fn flavour_mismatch_is_an_error() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let dir = root().join("corpus/x_ss_minimal");
    let model = RunSpec::default().load_model(&dir).unwrap();
    assert!(
        CompiledModel::compile_flavour(
            model,
            FlavourConfig {
                quirks: Quirks::NONE.with(Quirk::Q26),
                ..FlavourConfig::SIMULIZAR
            }
        )
        .is_err()
    );
    let cm = compile(&dir, Quirks::NONE);
    assert!(Simulation::new(&cm, config(1, 10, Mode::Exact), Outputs::default()).is_err());
}

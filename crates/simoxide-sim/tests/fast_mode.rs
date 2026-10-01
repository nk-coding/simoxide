//! The fast mode (`simoxide_sim::Fast`, cargo feature `fast`; enabled here through the
//! simoxide-testkit dependency):
//!
//! - API: mode dispatch, typed construction, errors for tape output / replay, determinism,
//!   traces, `run_batch` with mixed modes;
//! - the engine is not changed by the fast mode: with the same random numbers, `Fast` gives
//!   byte-identical measurements, end time, request, draw and event counts as `Literal<Fast>`
//!   (the fast generator on DESMO-J's literal event sequence, i.e. the engine of the exact mode
//!   before the result-neutral optimizations), on every model directory of the repository and
//!   on generated models; models without random draws give the exact mode's measurements byte
//!   for byte;
//! - a quick statistical equivalence run of exact vs fast (`simoxide_testkit::equiv`) over the
//!   corpus and generated models. The full campaign is `simoxide-fuzz equiv` (`docs/correctness/testing.md`).

use std::path::{Path, PathBuf};

use simoxide_sim::{
    CompiledModel, Exact, Fast, Literal, Mode, Outputs, RngMode, RunResult, SimConfig, Simulation,
    Tape, run_batch,
};
use simoxide_testkit::equiv::{EquivOptions, Model, Report};
use simoxide_testkit::modelgen::GenConfig;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every model directory with a `run.json` whose reference run does not abort.
fn model_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for sub in [
        "corpus",
        "corpus-fuzz",
        "crates/simoxide-sim/tests/models",
        "crates/simoxide-cli/bench/models",
    ] {
        let Ok(rd) = std::fs::read_dir(root().join(sub)) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.join("run.json").is_file() && !p.join("REFERENCE-ERROR.txt").is_file() {
                v.push(p);
            }
        }
    }
    v.sort();
    assert!(v.len() > 80, "{} model directories", v.len());
    v
}

fn run(cm: &CompiledModel, base: &SimConfig, mode: Mode, seed: i64) -> Result<RunResult, String> {
    let cfg = SimConfig {
        mode,
        seed,
        store_measurements: true,
        ..base.clone()
    };
    simoxide_sim::run(cm, cfg, Outputs::default()).map_err(|e| e.to_string())
}

/// The fast random numbers on the literal engine (separate hand-off notes, copied `INNER`
/// contents).
fn run_literal_fast(cm: &CompiledModel, base: &SimConfig, seed: i64) -> Result<RunResult, String> {
    let cfg = SimConfig {
        mode: Mode::Fast,
        seed,
        store_measurements: true,
        ..base.clone()
    };
    Simulation::<Literal<Fast>>::create(cm, cfg, Outputs::default())
        .and_then(|s| s.run())
        .map_err(|e| e.to_string())
}

/// What must be equal between two runs that do the same thing.
fn fingerprint(r: &Result<RunResult, String>) -> String {
    match r {
        Ok(r) => format!(
            "end {} main {} draws {} events {} tuples {}\n{}",
            r.end_ns,
            r.main_count,
            r.uniforms,
            r.events,
            r.measurements.count,
            r.measurements.to_csv()
        ),
        Err(e) => format!("error {e}"),
    }
}

fn h01() -> Model {
    Model::from_dir(&root().join("corpus/h01_ps_single")).unwrap()
}

#[test]
fn mode_names_round_trip() {
    for m in [Mode::Exact, Mode::Fast] {
        assert_eq!(Mode::parse(m.name()), Some(m));
        assert!(m.available());
    }
    assert_eq!(Mode::parse("turbo"), None);
    assert_eq!(Mode::default(), Mode::Exact);
}

#[test]
fn typed_construction_checks_the_mode() {
    let m = h01();
    let fast = SimConfig {
        mode: Mode::Fast,
        ..m.base.clone()
    };
    // Simulation::new is the exact mode
    let e = Simulation::new(&m.cm, fast.clone(), Outputs::default())
        .err()
        .expect("mode mismatch");
    assert!(e.message.contains("mode"), "{}", e.message);
    let r = Simulation::<Fast>::create(&m.cm, fast.clone(), Outputs::default())
        .unwrap()
        .run()
        .unwrap();
    assert_eq!(
        fingerprint(&Ok(r)),
        fingerprint(&simoxide_sim::run(&m.cm, fast, Outputs::default()).map_err(|e| e.message))
    );
    assert!(Simulation::<Exact>::create(&m.cm, m.base.clone(), Outputs::default()).is_ok());
}

#[test]
fn fast_mode_has_no_tape() {
    let m = h01();
    let cfg = SimConfig {
        mode: Mode::Fast,
        ..m.base.clone()
    };
    let out = Outputs {
        tape: Some(Box::new(Vec::new())),
        trace: None,
    };
    let e = simoxide_sim::run(&m.cm, cfg.clone(), out).expect_err("no tape");
    assert!(e.message.contains("tape"), "{}", e.message);
    let replay = SimConfig {
        rng: RngMode::Replay(std::sync::Arc::new(Tape::default())),
        ..cfg
    };
    let e = simoxide_sim::run(&m.cm, replay, Outputs::default()).expect_err("no replay");
    assert!(e.message.contains("replay"), "{}", e.message);
}

#[test]
fn fast_mode_is_deterministic_and_seeded() {
    let m = Model::from_dir(&root().join("corpus/x_sl_mediastore")).unwrap();
    let a = run(&m.cm, &m.base, Mode::Fast, 5);
    let b = run(&m.cm, &m.base, Mode::Fast, 5);
    let c = run(&m.cm, &m.base, Mode::Fast, 6);
    assert_eq!(fingerprint(&a), fingerprint(&b));
    assert_ne!(fingerprint(&a), fingerprint(&c));
    // measurements do not depend on storing them or on a trace
    #[derive(Clone, Default)]
    struct Buf(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);
    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let buf = Buf::default();
    let cfg = SimConfig {
        mode: Mode::Fast,
        seed: 5,
        ..m.base.clone()
    };
    let traced = simoxide_sim::run(
        &m.cm,
        cfg,
        Outputs {
            trace: Some(Box::new(buf.clone())),
            tape: None,
        },
    );
    assert_eq!(fingerprint(&traced.map_err(|e| e.message)), fingerprint(&a));
    let trace = String::from_utf8(buf.0.borrow().clone()).unwrap();
    assert!(trace.lines().count() > 100 && trace.contains("\"ev\":\"demand\""));
}

#[test]
fn run_batch_dispatches_per_config() {
    let m = h01();
    let configs: Vec<SimConfig> = [Mode::Exact, Mode::Fast, Mode::Fast, Mode::Exact]
        .iter()
        .enumerate()
        .map(|(i, &mode)| SimConfig {
            mode,
            seed: 10 + i as i64,
            store_measurements: true,
            ..m.base.clone()
        })
        .collect();
    let batch = run_batch(&m.cm, &configs, 3);
    for (c, r) in configs.iter().zip(batch) {
        let seq = simoxide_sim::run(&m.cm, c.clone(), Outputs::default());
        assert_eq!(
            fingerprint(&r.map_err(|e| e.message)),
            fingerprint(&seq.map_err(|e| e.message))
        );
    }
}

/// With the same random numbers, the engine (merged hand-offs, `INNER` without copies) gives
/// exactly the literal engine's results: `Fast` = `Literal<Fast>` byte for byte. Models without
/// random draws give the exact mode's output in both.
#[test]
fn fast_engine_equals_exact_engine_on_every_model_directory() {
    let mut deterministic = 0;
    for dir in model_dirs() {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let m = Model::from_dir(&dir).unwrap_or_else(|e| panic!("{name}: {e}"));
        for seed in [m.base.seed, 77] {
            let fast = run(&m.cm, &m.base, Mode::Fast, seed);
            let frng = run_literal_fast(&m.cm, &m.base, seed);
            assert_eq!(fingerprint(&fast), fingerprint(&frng), "{name} seed {seed}");
            let exact = run(&m.cm, &m.base, Mode::Exact, seed);
            if let Ok(e) = &exact
                && e.uniforms == 0
            {
                deterministic += 1;
                let f = fast.as_ref().unwrap();
                assert_eq!(f.uniforms, 0, "{name}");
                assert_eq!(
                    e.measurements.to_csv(),
                    f.measurements.to_csv(),
                    "{name}: a model without random draws differs in fast mode"
                );
                assert_eq!((e.end_ns, e.main_count), (f.end_ns, f.main_count), "{name}");
            }
        }
    }
    assert!(deterministic >= 20, "{deterministic} deterministic runs");
}

/// The same on generated models with every generator feature (heavy load, ties, double
/// resumes, triggers, windows, ...). `FAST_ENGINE_N` sets the number (default 60).
#[test]
fn fast_engine_equals_exact_engine_on_generated_models() {
    let n: u64 = std::env::var("FAST_ENGINE_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let mut ran = 0;
    for i in 0..n {
        let mut g = GenConfig::new(format!("fe{i}"), 3_100_000 + i, 1 + (i % 10) as u32);
        if i % 2 == 0 {
            for f in simoxide_testkit::modelgen::FEATURE_NAMES {
                if !matches!(*f, "long_run") {
                    g.features.set(f, 1.0);
                }
            }
        }
        let Ok(m) = Model::generated(&g) else {
            continue;
        };
        ran += 1;
        let fast = run(&m.cm, &m.base, Mode::Fast, 1);
        let frng = run_literal_fast(&m.cm, &m.base, 1);
        assert_eq!(fingerprint(&fast), fingerprint(&frng), "{}", g.name);
    }
    assert!(ran * 10 >= n * 9, "{ran} of {n} generated models compiled");
}

/// Exact vs fast on the corpus and on generated models, 20 seeds each: no Holm-corrected
/// failure and no systematic difference (the full campaign: `simoxide-fuzz equiv`).
#[test]
fn quick_statistical_equivalence() {
    let o = EquivOptions {
        seeds: 20,
        ..EquivOptions::default()
    };
    let mut rep = Report {
        modes: o.modes.to_vec(),
        ..Report::default()
    };
    for dir in model_dirs()
        .into_iter()
        .filter(|d| d.parent().is_some_and(|p| p.ends_with("corpus")))
    {
        rep.add_model(&Model::from_dir(&dir).unwrap(), &o);
    }
    for i in 0..40u64 {
        let g = GenConfig::new(format!("qe{i}"), 4_100_000 + i, 1 + (i % 10) as u32);
        if let Ok(m) = Model::generated(&g) {
            rep.add_model(&m, &o);
        }
    }
    rep.finish(0.001);
    let text = rep.text(10);
    assert!(rep.tests.len() > 3000, "{} tests", rep.tests.len());
    assert!(rep.failures().is_empty(), "{text}");
    assert!(rep.deterministic_differences().is_empty(), "{text}");
    // p-values are not piled up near 0
    let low = rep.calibration()[1].1;
    assert!(low < 0.02, "fraction of p < 0.01: {low}\n{text}");
}

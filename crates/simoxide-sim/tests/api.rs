//! API properties: `CompiledModel` is shareable, `run_batch` equals sequential runs.

use simoxide_sim::{CompiledModel, Outputs, RunSpec, SimConfig, Simulation, run_batch};
use std::path::Path;

fn corpus(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus")
        .join(name)
}

#[test]
fn compiled_model_is_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<CompiledModel>();
}

#[test]
fn batch_equals_sequential_runs() {
    let dir = corpus("x_sl_mediastore");
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let cm = CompiledModel::compile(spec.load_model(&dir).unwrap()).unwrap();
    let configs: Vec<SimConfig> = (0..8)
        .map(|i| SimConfig {
            seed: 100 + i,
            ..spec.sim_config("x")
        })
        .collect();
    let batch = run_batch(&cm, &configs, 4);
    for (cfg, b) in configs.iter().zip(batch) {
        let b = b.unwrap();
        let s = Simulation::new(&cm, cfg.clone(), Outputs::default())
            .unwrap()
            .run()
            .unwrap();
        assert_eq!(b.measurements.to_csv(), s.measurements.to_csv());
        assert_eq!(
            (b.uniforms, b.events, b.end_ns),
            (s.uniforms, s.events, s.end_ns)
        );
    }
}

#[test]
fn seeds_change_the_run() {
    let dir = corpus("h01_ps_single");
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let cm = CompiledModel::compile(spec.load_model(&dir).unwrap()).unwrap();
    let run = |seed| {
        Simulation::new(
            &cm,
            SimConfig {
                seed,
                ..spec.sim_config("h01")
            },
            Outputs::default(),
        )
        .unwrap()
        .run()
        .unwrap()
    };
    let (a, b) = (run(1), run(2));
    assert_ne!(a.end_ns, b.end_ns);
    assert_eq!(a.main_count, 102);
}

/// REF-7 guard: a closed workload with zero think time and zero demands never advances time.
/// With a time stop only, the run aborts after `max_events_per_instant` events at t = 0; with a
/// measurement stop, finished scenario runs are progress and the run terminates normally.
#[test]
fn zero_time_livelock_is_stopped() {
    let src = corpus("h02_ps_ties");
    let dir = std::env::temp_dir().join(format!("simoxide-sim-ref7-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for e in std::fs::read_dir(&src).unwrap() {
        let p = e.unwrap().path();
        if p.is_file() {
            let t = std::fs::read_to_string(&p).unwrap();
            let t = t.replace("specification=\"1.0\"", "specification=\"0.0\"");
            std::fs::write(dir.join(p.file_name().unwrap()), t).unwrap();
        }
    }
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let cm = CompiledModel::compile(spec.load_model(&dir).unwrap()).unwrap();
    let run = |max_measurements: i64| {
        let cfg = SimConfig {
            max_sim_time: 10,
            max_measurements,
            max_events_per_instant: 50_000,
            store_measurements: false,
            ..spec.sim_config("ref7")
        };
        Simulation::new(&cm, cfg, Outputs::default()).and_then(|s| s.run())
    };
    let e = run(-1).unwrap_err();
    assert!(e.message.contains("livelock"), "{e}");
    assert_eq!(e.at_ns, 0);
    let r = run(200_000).unwrap();
    // (the post-stop drain lets the 3 users finish their iteration too)
    assert_eq!((r.end_ns, r.main_count), (0, 200_003));
    let _ = std::fs::remove_dir_all(&dir);
}

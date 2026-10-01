//! The engine's result-neutral shortcuts (`simoxide_sim::compat`: hand-offs run inside the
//! waking event instead of a separate `Resume` note, `INNER` evaluated without copying the frame
//! contents) change no output of the exact mode: against `Literal<Exact>` (DESMO-J's literal
//! event sequence) the trace, the random tape, the measurements, the event count and every
//! error (message, time, trace prefix) are byte-identical. Checked on every model directory of
//! the repository (including the ones where the reference aborts), under tight livelock and
//! event limits, and on generated models (`LITERAL_N`, default 60).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use simoxide_sim::{
    CompiledModel, Exact, Limits, Literal, Outputs, RngMode, SimConfig, Simulation, Tape,
};
use simoxide_testkit::equiv::Model;
use simoxide_testkit::modelgen::GenConfig;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
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

/// Everything a run writes or returns.
fn outputs<const LITERAL: bool>(cm: &CompiledModel, cfg: &SimConfig) -> String {
    let (trace, tape) = (Buf::default(), Buf::default());
    let out = Outputs {
        trace: Some(Box::new(trace.clone())),
        tape: Some(Box::new(tape.clone())),
    };
    let cfg = SimConfig {
        store_measurements: true,
        ..cfg.clone()
    };
    let r = if LITERAL {
        Simulation::<Literal<Exact>>::create(cm, cfg, out).and_then(|s| s.run())
    } else {
        Simulation::<Exact>::create(cm, cfg, out).and_then(|s| s.run())
    };
    let res = match r {
        Ok(r) => format!(
            "end {} main {} uniforms {} events {} warnings {:?}\n{}",
            r.end_ns,
            r.main_count,
            r.uniforms,
            r.events,
            r.warnings,
            r.measurements.to_csv()
        ),
        Err(e) => format!("error {:?} at {}: {}", e.kind, e.at_ns, e.message),
    };
    let t = String::from_utf8(trace.0.borrow().clone()).unwrap();
    let u = String::from_utf8(tape.0.borrow().clone()).unwrap();
    format!("{res}\n--- trace\n{t}--- tape\n{u}")
}

fn check(name: &str, cm: &CompiledModel, cfg: &SimConfig) -> bool {
    let a = outputs::<false>(cm, cfg);
    let b = outputs::<true>(cm, cfg);
    if a != b {
        let line = a
            .lines()
            .zip(b.lines())
            .position(|(x, y)| x != y)
            .unwrap_or(0);
        panic!(
            "{name}: outputs differ from the literal engine at line {line}:\n  {:?}\n  {:?}",
            a.lines().nth(line),
            b.lines().nth(line)
        );
    }
    a.starts_with("error")
}

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
            if p.join("run.json").is_file() {
                v.push(p);
            }
        }
    }
    v.sort();
    assert!(v.len() > 80, "{} model directories", v.len());
    v
}

#[test]
fn engine_equals_literal_engine_on_every_model_directory() {
    let (mut runs, mut errors) = (0, 0);
    for dir in model_dirs() {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let Ok(m) = Model::from_dir(&dir) else {
            continue;
        };
        let mut cfgs = vec![
            m.base.clone(),
            SimConfig {
                seed: 77,
                ..m.base.clone()
            },
            // limits that fire in the middle of the run: at an elided note or next to it
            SimConfig {
                max_events_per_instant: 7,
                ..m.base.clone()
            },
        ];
        for max_events in [1u64, 2, 3, 50, 333] {
            cfgs.push(SimConfig {
                limits: Limits {
                    max_events,
                    ..Limits::default()
                },
                ..m.base.clone()
            });
        }
        // tape replay: the reference's tape, where there is one
        if let Ok(text) = std::fs::read_to_string(dir.join("expected/tape.jsonl"))
            && let Ok(tape) = Tape::parse(&text)
        {
            // and a truncated copy: the run ends with "random tape exhausted"
            let mut short = tape.clone();
            let n = short.uniforms.len() / 3;
            short.uniforms.truncate(n);
            short.origins.truncate(n);
            for t in [tape, short] {
                cfgs.push(SimConfig {
                    rng: RngMode::Replay(std::sync::Arc::new(t)),
                    check_tape_origins: true,
                    ..m.base.clone()
                });
            }
        }
        for cfg in &cfgs {
            runs += 1;
            errors += check(&name, &m.cm, cfg) as usize;
        }
    }
    assert!(runs > 700 && errors > 100, "{runs} runs, {errors} errors");
}

#[test]
fn engine_equals_literal_engine_on_generated_models() {
    let n: u64 = std::env::var("LITERAL_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    let mut ran = 0;
    for i in 0..n {
        let mut g = GenConfig::new(format!("le{i}"), 5_100_000 + i, 1 + (i % 10) as u32);
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
        check(&g.name, &m.cm, &m.base);
        check(
            &g.name,
            &m.cm,
            &SimConfig {
                seed: 3,
                max_events_per_instant: 5,
                ..m.base.clone()
            },
        );
    }
    assert!(ran * 10 >= n * 9, "{ran} of {n} generated models compiled");
}

//! Moving the clock of a paused run (`Simulation::advance_clock`, `SsxRun::advance_clock`;
//! `simoxide resume --actions-at boundary`, `docs/guide/snapshots.md`): a run whose clock moved
//! to a time before its next event and that changes nothing continues as the uninterrupted run
//! (trace, measurements, event and draw counts), on every corpus model in both flavours and in
//! the Slingshot-exact engine, also when the clock moves right up to the next event; the clock
//! never moves past the next event, back, or outside a running simulation; the move is part
//! of the run state, so a snapshot taken after it continues as the run that moved.

use simoxide_sim::ssx::SsxRun;
use simoxide_sim::{
    Compat, CompiledModel, Fast, Flavour, FlavourConfig, Outputs, Pause, Quirks, RunResult,
    RunSpec, SimConfig, SimuLizar, Simulation, Slingshot, Status,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A shared in-memory writer.
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
        String::from_utf8(self.0.borrow().clone()).expect("utf-8")
    }
}

fn traced() -> (Outputs, Buf) {
    let b = Buf::default();
    (
        Outputs {
            trace: Some(Box::new(b.clone())),
            tape: None,
        },
        b,
    )
}

/// Everything a run produces, as text.
fn outcome(r: Result<RunResult, simoxide_sim::SimError>, trace: &Buf) -> String {
    let r = match r {
        Ok(r) => format!(
            "events={} uniforms={} end={} main={} warnings={:?}\n{}",
            r.events,
            r.uniforms,
            r.end_ns,
            r.main_count,
            r.warnings,
            r.measurements.to_csv()
        ),
        Err(e) => format!("error: {e}"),
    };
    format!("{r}\n{}", trace.text())
}

/// The corpus models with a `run.json`, compiled for `flavour` (models the flavour does not
/// take are left out).
fn models(flavour: FlavourConfig) -> Vec<(String, CompiledModel, SimConfig)> {
    let mut v = Vec::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root().join("corpus"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("run.json").is_file())
        .collect();
    dirs.sort();
    for dir in dirs {
        let Ok(spec) = RunSpec::load(&dir.join("run.json")) else {
            continue;
        };
        let Ok(model) = spec.load_model(&dir) else {
            continue;
        };
        let Ok(cm) = CompiledModel::compile_flavour(model, flavour) else {
            continue;
        };
        let mut cfg = spec.sim_config("advance");
        cfg.mode = Fast::MODE;
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        v.push((name, cm, cfg));
    }
    assert!(v.len() > 30, "{} models", v.len());
    v
}

/// The uninterrupted run, and the run paused before `at_ns` whose clock moved to `to(next)`
/// (`next`: the next event's time) before it continued: the same outputs. `None` if the run
/// does not pause there.
fn moved<C: Compat, F: Flavour>(
    cm: &CompiledModel,
    cfg: &SimConfig,
    at_ns: i64,
    to: impl Fn(i64, i64) -> i64,
) -> Option<(String, String)> {
    let (out, t) = traced();
    let whole = outcome(
        Simulation::<C, F>::create(cm, cfg.clone(), out)
            .unwrap()
            .finish(),
        &t,
    );
    let (out, t) = traced();
    let mut sim = Simulation::<C, F>::create(cm, cfg.clone(), out).unwrap();
    match sim.run_until(Pause::Before(at_ns)) {
        Ok(Status::Paused) => {}
        _ => return None,
    }
    let now = sim.state().now_ns();
    let next = sim.state().next_event_ns()?;
    let target = to(now, next);
    let moved = sim.advance_clock(target).unwrap();
    assert_eq!(moved, target > now);
    assert!(sim.state().now_ns() <= next);
    Some((whole, outcome(sim.finish(), &t)))
}

fn moved_clock_changes_nothing<F: Flavour>(flavour: FlavourConfig) {
    let mut checked = 0;
    for (name, cm, cfg) in models(flavour) {
        // a quiet stretch: between the last event and the next one, and the next event itself
        let horizon = if cfg.max_sim_time > 0 {
            cfg.max_sim_time * 1_000_000_000
        } else {
            10_000_000_000
        };
        for frac in [3, 2] {
            let at = horizon / frac + 7_654_321;
            let mid = moved::<Fast, F>(&cm, &cfg, at, |now, next| now + (next - now) / 2);
            if let Some((whole, b)) = mid {
                assert!(whole == b, "{name} ({}): moved to the middle", flavour.kind);
                checked += 1;
            }
            let up = moved::<Fast, F>(&cm, &cfg, at, |_, next| next);
            if let Some((whole, b)) = up {
                assert!(
                    whole == b,
                    "{name} ({}): moved to the next event",
                    flavour.kind
                );
            }
        }
    }
    assert!(checked > 30, "{checked} pauses checked");
}

#[test]
fn a_moved_clock_changes_nothing_simulizar() {
    moved_clock_changes_nothing::<SimuLizar>(FlavourConfig::SIMULIZAR);
}

#[test]
fn a_moved_clock_changes_nothing_slingshot() {
    moved_clock_changes_nothing::<Slingshot>(FlavourConfig::slingshot(Quirks::NONE));
}

fn load(dir: &str, flavour: FlavourConfig) -> (CompiledModel, SimConfig) {
    let dir = root().join(dir);
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let cm = CompiledModel::compile_flavour(spec.load_model(&dir).unwrap(), flavour).unwrap();
    let mut cfg = spec.sim_config("advance");
    cfg.mode = Fast::MODE;
    (cm, cfg)
}

/// Not before the start, not past the next event (the run is unchanged), never back, not
/// after the end; a snapshot after the move holds the moved clock and continues like the run.
fn the_rules_of_the_move<F: Flavour>(flavour: FlavourConfig) {
    let (cm, cfg) = load("corpus/h06_closed_think", flavour);
    let mut sim = Simulation::<Fast, F>::create(&cm, cfg.clone(), Outputs::default()).unwrap();
    assert!(sim.advance_clock(1_000_000_000).is_err(), "not started");
    sim.run_until(Pause::Before(5_000_000_000)).unwrap();
    let now = sim.state().now_ns();
    let next = sim.state().next_event_ns().unwrap();
    assert!(next > now + 2, "a gap after {now} ns");
    let mut before = Vec::new();
    sim.snapshot_to(&mut before).unwrap();
    let err = sim.advance_clock(next + 1).unwrap_err();
    assert!(err.message.contains("pending"), "{err}");
    let mut after = Vec::new();
    sim.snapshot_to(&mut after).unwrap();
    assert_eq!(before, after, "a refused move changes nothing");
    assert!(!sim.advance_clock(now - 1).unwrap());
    assert!(!sim.advance_clock(now).unwrap());
    let to = now + (next - now) / 2;
    assert!(sim.advance_clock(to).unwrap());
    assert_eq!(sim.state().now_ns(), to);
    assert_eq!(sim.state().events(), {
        let s = Simulation::<Fast, F>::restore_from(&before[..], &cm, Outputs::default()).unwrap();
        s.state().events()
    });
    // the moved clock is in the snapshot, which continues like the run
    let mut moved = Vec::new();
    sim.snapshot_to(&mut moved).unwrap();
    assert_ne!(moved, before);
    let restored =
        Simulation::<Fast, F>::restore_from(&moved[..], &cm, Outputs::default()).unwrap();
    assert_eq!(restored.state().now_ns(), to);
    let a = restored.finish().unwrap();
    let b = sim.finish().unwrap();
    assert_eq!(a.measurements.to_csv(), b.measurements.to_csv());
    assert_eq!(
        (a.events, a.uniforms, a.end_ns),
        (b.events, b.uniforms, b.end_ns)
    );
    // a finished run
    let mut sim = Simulation::<Fast, F>::create(&cm, cfg, Outputs::default()).unwrap();
    assert_eq!(sim.run_until(Pause::Never).unwrap(), Status::Finished);
    assert!(sim.advance_clock(i64::MAX / 2).is_err(), "finished");
}

#[test]
fn the_rules_of_the_move_simulizar() {
    the_rules_of_the_move::<SimuLizar>(FlavourConfig::SIMULIZAR);
}

#[test]
fn the_rules_of_the_move_slingshot() {
    the_rules_of_the_move::<Slingshot>(FlavourConfig::slingshot(Quirks::NONE));
}

/// The Slingshot-exact engine: a moved clock changes nothing on every case of `corpus-ss/`
/// (projection, measurements, counts); the same rules as above.
#[test]
fn the_slingshot_exact_engine_moves_its_clock_alike() {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root().join("corpus-ss"))
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("case.json").is_file())
        .collect();
    dirs.sort();
    let mut checked = 0;
    for dir in dirs {
        let case: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("case.json")).unwrap()).unwrap();
        let model_dir = root().join(case["model"].as_str().unwrap());
        if !simoxide_thirdparty::skip_unless_available(&model_dir) {
            continue;
        }
        let Ok(model) = RunSpec::default().load_model(&model_dir) else {
            continue;
        };
        let cm = CompiledModel::compile_flavour(model, FlavourConfig::slingshot_exact()).unwrap();
        let cfg = SimConfig {
            seed: case["seed"].as_i64().unwrap(),
            max_sim_time: case["sim_time"].as_i64().unwrap(),
            max_measurements: -1,
            ..Default::default()
        };
        let run = |at: Option<(i64, bool)>| -> Option<String> {
            let (out, t) = traced();
            let mut run = SsxRun::new(&cm, cfg.clone(), out).ok()?;
            if let Some((at, up)) = at {
                if run.run_until(Pause::Before(at), None).ok()? != Status::Paused {
                    return None;
                }
                let now = run.now_ns();
                // the next event: the first time the run would not pause before
                let mut probe = run.fork(Outputs::default()).ok()?;
                probe
                    .run_until(Pause::Events(probe.events() + 1), None)
                    .ok()?;
                let next = probe.now_ns();
                // past the next event: refused (a fork tries; `Finished` at the end is an event)
                if next + 2 < cfg.max_sim_time * 1_000_000_000 {
                    let mut past = run.fork(Outputs::default()).ok()?;
                    assert!(past.advance_clock(next + 2).is_err());
                }
                let to = if up { next } else { now + (next - now) / 2 };
                assert_eq!(run.advance_clock(to).unwrap(), to > now);
            }
            Some(outcome(run.finish(None), &t))
        };
        // (a case whose set-up fails has no pause)
        let Some(whole) = run(None) else { continue };
        let half = cfg.max_sim_time * 500_000_000 + 7_654_321;
        for up in [false, true] {
            if let Some(b) = run(Some((half, up))) {
                assert!(
                    whole == b,
                    "{}: moved (to the next event: {up})",
                    dir.display()
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 10, "{checked} pauses checked");
}

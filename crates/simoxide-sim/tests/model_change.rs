//! Runtime model changes (`crate::change`, `DESIGN-SPD.md` F2) on every model directory of the
//! repository: a change set whose graph is unchanged runs the whole change path (typed rebuild
//! with stable ids, seeded recompilation, merge, run-state adoption) and must leave the run
//! exactly as it was: trace, tape, measurements and counts equal the uninterrupted run, also
//! after a fork, a resume of the state on the base model and a deep copy on another thread.
//! Tests with real changes are in `simoxide-reconf` (they edit through the QVTo adapter).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use simoxide_model::meta::feat;
use simoxide_model::raw::ObjId;
use simoxide_sim::{
    ChangeKind, ChangeSet, Compat, CompiledModel, Exact, Fast, Mode, Outputs, Pause, RunResult,
    SimConfig, Simulation,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
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
            if p.join("run.json").is_file() && simoxide_thirdparty::skip_unless_available(&p) {
                v.push(p);
            }
        }
    }
    v.sort();
    // fewer without the third-party submodules (third_party/README.md)
    assert!(
        v.len() > 80 || !simoxide_thirdparty::all_available(),
        "{} model directories",
        v.len()
    );
    v
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
    fn take(&self) -> String {
        String::from_utf8(std::mem::take(&mut *self.0.borrow_mut())).expect("utf-8")
    }
}

#[derive(Default)]
struct W {
    trace: Buf,
    tape: Buf,
}

impl W {
    fn outputs(&self, tape: bool) -> Outputs {
        Outputs {
            trace: Some(Box::new(self.trace.clone())),
            tape: tape.then(|| Box::new(self.tape.clone()) as Box<dyn std::io::Write>),
        }
    }
}

fn fingerprint(r: &Result<RunResult, simoxide_sim::SimError>) -> String {
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
        Err(e) => format!("error {:?} at {}: {}", e.kind, e.at_ns, e.message),
    }
}

fn first_difference(a: &str, b: &str) -> String {
    for (i, (x, y)) in a.lines().zip(b.lines()).enumerate() {
        if x != y {
            return format!("line {}:\n  want {x}\n  got  {y}", i + 1);
        }
    }
    format!("lengths {} vs {}", a.lines().count(), b.lines().count())
}

/// A change set that says "something changed" without changing the graph.
fn touch() -> ChangeSet {
    let mut cs = ChangeSet::new();
    cs.push(
        ObjId(0),
        ChangeKind::Set {
            feature: feat::NamedElement_entityName,
        },
    );
    cs
}

/// The uninterrupted run: (result, trace, tape).
fn plain<C: Compat>(cm: &CompiledModel, cfg: &SimConfig, tape: bool) -> Option<[String; 3]> {
    let w = W::default();
    let sim = Simulation::<C>::create(cm, cfg.clone(), w.outputs(tape)).ok()?;
    let r = fingerprint(&sim.finish());
    Some([r, w.trace.take(), w.tape.take()])
}

/// Paused at each of `ps` with an unchanged-graph change set applied there; at the second
/// pause the run is forked (the fork finishes on another thread after a resume of a deep copy),
/// at the third the state is taken out and resumed on the base model.
fn changed<C: Compat>(
    cm: &CompiledModel,
    cfg: &SimConfig,
    tape: bool,
    ps: &[Pause],
) -> [[String; 3]; 2] {
    let w = W::default();
    let mut sim = Simulation::<C>::create(cm, cfg.clone(), w.outputs(tape)).expect("create");
    let mut out = [String::new(), String::new(), String::new()];
    let mut fork = None;
    for (i, &p) in ps.iter().enumerate() {
        let _ = sim.run_until(p);
        let g = sim.model_graph().clone();
        let applied = sim.apply_changes(g, &touch());
        if let Err(e) = &applied {
            // finished or failed runs refuse changes
            assert!(matches!(e, simoxide_sim::ChangeError::State(_)), "{e}");
        }
        match i {
            1 => {
                let st = sim.snapshot();
                out[1].push_str(&w.trace.take());
                out[2].push_str(&w.tape.take());
                fork = Some((st.detach(), out.clone()));
            }
            2 => {
                let st = sim.into_state();
                out[1].push_str(&w.trace.take());
                out[2].push_str(&w.tape.take());
                sim = Simulation::resume(cm, st, w.outputs(tape)).expect("resume");
            }
            _ => {}
        }
    }
    assert!(
        sim.state().model().generation() >= 1,
        "no change set was applied"
    );
    let r = fingerprint(&sim.finish());
    out[0] = r;
    out[1].push_str(&w.trace.take());
    out[2].push_str(&w.tape.take());
    let (d, mut fo) = fork.expect("fork");
    let f = std::thread::scope(|sc| {
        sc.spawn(|| {
            let wf = W::default();
            let sim = Simulation::resume(cm, d.into_state(), wf.outputs(tape)).expect("resume");
            let r = fingerprint(&sim.finish());
            [r, wf.trace.take(), wf.tape.take()]
        })
        .join()
        .expect("thread")
    });
    fo[0] = f[0].clone();
    fo[1].push_str(&f[1]);
    fo[2].push_str(&f[2]);
    [out, fo]
}

fn check_all<C: Compat>(mode: Mode, tape: bool) {
    let mut n = 0;
    for d in model_dirs() {
        let Ok(m) = simoxide_testkit::equiv::Model::from_dir(&d) else {
            continue;
        };
        let cfg = SimConfig { mode, ..m.base };
        let Some(want) = plain::<C>(&m.cm, &cfg, tape) else {
            continue;
        };
        let events: u64 = want[0]
            .split_whitespace()
            .nth(7)
            .and_then(|x| x.parse().ok())
            .unwrap_or(100);
        let ps = [
            Pause::Events(0),
            Pause::Events(events / 3),
            Pause::Events(events / 2),
            Pause::Events(events * 2 / 3 + 1),
        ];
        let [a, b] = changed::<C>(&m.cm, &cfg, tape, &ps);
        for (what, got) in [("run", &a), ("fork", &b)] {
            for (k, part) in ["result", "trace", "tape"].iter().enumerate() {
                assert!(
                    want[k] == got[k],
                    "{} {what}: {part} differs: {}",
                    m.name,
                    first_difference(&want[k], &got[k])
                );
            }
        }
        n += 1;
    }
    assert!(
        n > 80 || !simoxide_thirdparty::all_available(),
        "{n} models"
    );
}

#[test]
fn unchanged_model_rebuild_keeps_the_run_exact() {
    check_all::<Exact>(Mode::Exact, true);
}

#[test]
fn unchanged_model_rebuild_keeps_the_run_fast() {
    check_all::<Fast>(Mode::Fast, false);
}

#[test]
fn empty_change_set_changes_nothing() {
    let d = root().join("corpus/h01_ps_single");
    let m = simoxide_testkit::equiv::Model::from_dir(&d).expect("model");
    let mut sim = Simulation::new(&m.cm, m.base.clone(), Outputs::default()).expect("create");
    sim.run_until(Pause::Events(10)).expect("run");
    let g = sim.model_graph().clone();
    let r = sim.apply_changes(g, &ChangeSet::new()).expect("apply");
    assert_eq!(r.generation, 0);
    assert!(sim.state().model().compiled().is_none());
    let r = sim
        .apply_changes(sim.model_graph().clone(), &touch())
        .expect("apply");
    assert_eq!(r.generation, 1);
    assert!(r.applied.is_empty() && r.ignored.is_empty(), "{r:?}");
    assert!(sim.state().model().compiled().is_some());
}

#[test]
fn a_foreign_graph_is_refused() {
    let a = simoxide_testkit::equiv::Model::from_dir(&root().join("corpus/h01_ps_single"))
        .expect("model");
    let b = simoxide_testkit::equiv::Model::from_dir(&root().join("corpus/x_sl_mediastore"))
        .expect("model");
    let mut sim = Simulation::new(&a.cm, a.base.clone(), Outputs::default()).expect("create");
    sim.run_until(Pause::Events(10)).expect("run");
    let e = sim
        .apply_changes(b.cm.model.graph.clone(), &touch())
        .expect_err("foreign graph");
    assert!(matches!(e, simoxide_sim::ChangeError::Invalid(_)), "{e}");
    assert!(sim.state().model().compiled().is_none());
}

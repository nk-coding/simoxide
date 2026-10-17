//! SimuLizar's reconfiguration process with rules (DESIGN-SPD R1): the runtime measurement
//! values the rules see (against the patched SimuLizar reference, `tests/prm/regen.sh`), runs
//! without rules unchanged, forks and snapshots with rules, and SimuLizar's reconfiguration
//! examples (`corpus-rules/`) byte for byte against the reference's outputs.

mod common;

use common::{core, root};
use simoxide_reconf::{QvtMode, RuleSet, RulesHost, RulesRun, RunConfig};
use simoxide_sim::{CompiledModel, Outputs, Pause, RunSpec, SimConfig, Simulation, Status};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const S: i64 = 1_000_000_000;

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

/// The writers of a run.
#[derive(Clone, Default)]
struct W {
    trace: Buf,
    tape: Buf,
}

impl W {
    fn outputs(&self) -> Outputs {
        Outputs {
            trace: Some(Box::new(self.trace.clone())),
            tape: Some(Box::new(self.tape.clone())),
        }
    }
}

/// Trace, tape and measurements of a run.
#[derive(PartialEq)]
struct Out {
    trace: String,
    tape: String,
    csv: String,
}

impl std::fmt::Debug for Out {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Out({} trace, {} tape, {} csv bytes)",
            self.trace.len(),
            self.tape.len(),
            self.csv.len()
        )
    }
}

/// A model directory: the compiled model and its run configuration (run name = directory name,
/// as the reference's).
fn load(dir: &Path) -> (CompiledModel, SimConfig) {
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let model = spec.load_model(dir).unwrap();
    let cm = CompiledModel::compile(model).unwrap();
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    (cm, spec.sim_config(&name))
}

fn expected(dir: &Path) -> Out {
    let read = |f: &str| {
        simoxide_testkit::corpus::read_maybe_gz(&dir.join("expected").join(f))
            .unwrap()
            .unwrap_or_else(|| panic!("{}: no expected {f}", dir.display()))
    };
    Out {
        trace: read("trace.jsonl"),
        tape: read("tape.jsonl"),
        csv: read("measurements.csv"),
    }
}

fn host(rules: &Path, mode: QvtMode) -> RulesHost {
    let set = RuleSet::load_dir(core(), rules).unwrap();
    let cfg = RunConfig {
        mode,
        ..RunConfig::default()
    };
    RulesHost::new(core(), set, cfg).unwrap().keep_records()
}

fn empty_host() -> RulesHost {
    RulesHost::new(core(), RuleSet::default(), RunConfig::default()).unwrap()
}

/// Runs `cm` with `host` to the end.
fn run_rules(cm: &CompiledModel, cfg: &SimConfig, host: RulesHost) -> (Out, RulesHost) {
    let w = W::default();
    let sim = Simulation::new(cm, cfg.clone(), w.outputs()).unwrap();
    let (r, host) = RulesRun::new(sim, host).finish().unwrap();
    let out = Out {
        trace: w.trace.text(),
        tape: w.tape.text(),
        csv: r.measurements.to_csv(),
    };
    (out, host)
}

fn plain(cm: &CompiledModel, cfg: &SimConfig) -> Out {
    let w = W::default();
    let r = Simulation::new(cm, cfg.clone(), w.outputs())
        .unwrap()
        .finish()
        .unwrap();
    Out {
        trace: w.trace.text(),
        tape: w.tape.text(),
        csv: r.measurements.to_csv(),
    }
}

/// simoxide-sim's reconfiguration test models (MEAS-7.2/7.3): fixed- and variable-size
/// aggregation, time-driven aggregation, multi-core utilisation windows, lazily created
/// passive-resource recorders, a reconfiguration after the stop, a late first one.
fn reconf_models() -> Vec<PathBuf> {
    let dir = root().join("crates/simoxide-sim/tests/models");
    let mut v: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("t_reconf_")
        })
        .collect();
    v.sort();
    assert_eq!(v.len(), 7);
    v
}

fn prm_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/prm")
}

#[test]
fn the_rules_see_the_prm_values_simulizar_writes() {
    for dir in reconf_models() {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let (cm, cfg) = load(&dir);
        let (out, mut host) = run_rules(&cm, &cfg, host(&prm_dir().join("rules"), QvtMode::Auto));
        // the dump rule always succeeds: the run equals the reference's run without rules
        assert_eq!(out, expected(&dir), "{name}");
        let records = host.take_records();
        assert_eq!(records.len() as u64, host.stats().runs);
        assert!(records.iter().all(|r| r.succeeded));
        let log: String = records.iter().map(|r| r.rules[0].log.clone()).collect();
        let want = std::fs::read_to_string(prm_dir().join(format!("{name}.expected"))).unwrap();
        assert_eq!(log, want, "{name}: the PRM as the rules see it");
        if name == "t_reconf_final_flush" {
            // the reconfiguration runs after the stop, on the values of the write
            assert!(records.iter().any(|r| r.post_stop), "{name}");
        }
    }
}

#[test]
fn runs_with_an_empty_rule_set_equal_runs_without_rules() {
    let mut models = reconf_models();
    for m in [
        "h28_triggers_default",
        "h29_triggers_late",
        "h13_passive_contention",
    ] {
        models.push(root().join("corpus").join(m));
    }
    for dir in models {
        let (cm, cfg) = load(&dir);
        let (out, mut host) = run_rules(&cm, &cfg, empty_host());
        assert_eq!(out, plain(&cm, &cfg), "{}", dir.display());
        assert_eq!(host.stats().runs, host.stats().succeeded);
        // no rule ran: the run's QVTo state was created only for the PRM mirror, if at all
        assert_eq!(host.qvto().invocations(), 0);
    }
    // a model without PRM recorders never reconfigures: no QVTo state at all
    let (cm, cfg) = load(&root().join("corpus/h01_ps_single"));
    let (out, mut host) = run_rules(&cm, &cfg, empty_host());
    assert_eq!(out, plain(&cm, &cfg));
    assert_eq!(host.stats().runs, 0);
    assert!(!host.qvto().is_started());
}

/// The reconfiguration examples of `corpus-rules/` (`reference/import-rules-examples.sh`): the
/// run with SimuLizar's rules equals the patched reference's byte for byte.
fn example(name: &str, mode: QvtMode) -> (Out, RulesHost) {
    let dir = root().join("corpus-rules").join(name);
    let (cm, cfg) = load(&dir);
    let (out, host) = run_rules(&cm, &cfg, host(&dir.join("rules"), mode));
    let want = expected(&dir);
    assert!(out.trace == want.trace, "{name}: trace");
    assert!(out.tape == want.tape, "{name}: tape");
    assert!(out.csv == want.csv, "{name}: measurements");
    (out, host)
}

fn series(csv: &str, metric: &str) -> Vec<(f64, f64)> {
    csv.lines()
        .filter(|l| l.contains(metric))
        .map(|l| {
            let mut f = l.rsplitn(3, ',');
            let v = f.next().unwrap().parse().unwrap();
            let t = f.next().unwrap().parse().unwrap();
            (t, v)
        })
        .collect::<Vec<_>>()
}

#[test]
fn simulizar_examples_load_balancer() {
    // the example's launch configuration: outsource shifts load to Server2 in steps of 0.1
    let (out, h) = example("lb_outsource", QvtMode::Auto);
    assert_eq!(h.stats().runs, 99);
    // a failed `assert error` leaves the rule applied (QVTo 3.10.7, which SimuLizar 5.2.2 ships)
    assert_eq!(h.stats().succeeded, 99);
    assert_eq!(series(&out.csv, "Reconfiguration Time").len(), 99);
    // its QVTo folder: outsource, scaleDown, scaleIn, scaleUp in file-name order
    let (_, h) = example("lb_rules_qvto", QvtMode::Auto);
    let names: Vec<&str> = h.rules().rules.iter().map(|r| r.name()).collect();
    assert_eq!(names, ["outsource", "ScaleDown", "scaleIn", "ScaleUp"]);
    assert_eq!(h.stats().changed, 99);
}

#[test]
fn a_scale_out_adds_a_container_an_allocation_and_rewires_a_connector() {
    let (out, mut h) = example("lb_scaleout", QvtMode::Auto);
    // scaleOut's `assert fatal` guards make every other run "not applied": no tuple for them
    assert_eq!(h.stats().succeeded, 1);
    assert_eq!(series(&out.csv, "Reconfiguration Time").len(), 1);
    // the container count series records the new container
    let n: Vec<f64> = series(&out.csv, "Number of Resource Containers")
        .iter()
        .map(|x| x.1)
        .collect();
    assert_eq!(n, [3.0, 4.0]);
    let moved = h
        .take_records()
        .into_iter()
        .find(|r| r.succeeded)
        .expect("the scale-out");
    assert!(moved.rules[1].changed && moved.rules[1].log.contains("moved"));
}

#[test]
fn simulizar_examples_power_consumption_and_reconfiguration_test() {
    // the power-consumption variant's PCM-only rules (fast QVTo mode: 13 000 reconfigurations)
    let (_, h) = example("power_rules_qvto", QvtMode::Fast);
    assert!(h.stats().runs > 10_000 && h.stats().changed > 100);
    // SimuLizar's ReconfigurationSimulationTest: the n-th user's response time is n
    let (out, _) = example("sl_reconfiguration", QvtMode::Auto);
    let rt: Vec<f64> = series(&out.csv, "Response Time")
        .iter()
        .map(|x| x.1)
        .collect();
    assert_eq!(rt, (1..=10).map(f64::from).collect::<Vec<_>>());
}

/// Runs `name` to `pause`, then `f` on the run, then to the end; the outputs of the run that went
/// on, and of the forks `f` returns (each with its own writers, the continuation only).
fn forked(
    name: &str,
    pause: Pause,
    f: impl for<'a, 'm> FnOnce(&'a mut RulesRun<'m>) -> Vec<(RulesRun<'m>, W)>,
) -> (Out, Vec<(String, String, String)>) {
    let dir = root().join("corpus-rules").join(name);
    let (cm, cfg) = load(&dir);
    let w = W::default();
    let sim = Simulation::new(&cm, cfg, w.outputs()).unwrap();
    let mut run = RulesRun::new(sim, host(&dir.join("rules"), QvtMode::Auto));
    assert_eq!(run.run_until(pause).unwrap(), Status::Paused);
    let forks = f(&mut run);
    let (r, _) = run.finish().unwrap();
    let main = Out {
        trace: w.trace.text(),
        tape: w.tape.text(),
        csv: r.measurements.to_csv(),
    };
    let rest = forks
        .into_iter()
        .map(|(fork, fw)| {
            let (r, _) = fork.finish().unwrap();
            (fw.trace.text(), fw.tape.text(), r.measurements.to_csv())
        })
        .collect();
    (main, rest)
}

#[test]
fn forks_and_snapshots_carry_the_prm_values_and_the_rules() {
    let dir = root().join("corpus-rules/lb_scaleout");
    let want = expected(&dir);
    // before the scale-out (t = 50 s), after it (t = 500 s), and with a reconfiguration waiting
    for pause in [Pause::After(50 * S), Pause::After(500 * S)] {
        let (main, forks) = forked("lb_scaleout", pause, |run| {
            let fw = W::default();
            vec![(run.fork(fw.outputs()).unwrap(), fw)]
        });
        assert_eq!(main, want);
        for (trace, tape, csv) in forks {
            assert!(want.trace.ends_with(&trace) && !trace.is_empty());
            assert!(want.tape.ends_with(&tape));
            assert_eq!(csv, want.csv);
        }
    }
    let (main, forks) = forked("lb_scaleout", Pause::After(95 * S), |run| {
        // run to the next reconfiguration and fork while it waits for the host
        run.sim_mut().run_while(|s| !s.reconf_pending()).unwrap();
        assert!(run.sim().reconf_due().is_some());
        let fw = W::default();
        vec![(run.fork(fw.outputs()).unwrap(), fw)]
    });
    assert_eq!(main, want);
    for (trace, _, csv) in forks {
        assert!(want.trace.ends_with(&trace));
        assert_eq!(csv, want.csv);
    }
}

#[test]
fn a_detached_snapshot_with_its_host_continues_on_another_thread() {
    let dir = root().join("corpus-rules/lb_scaleout");
    let want = expected(&dir);
    let (cm, cfg) = load(&dir);
    let w = W::default();
    let sim = Simulation::new(&cm, cfg, w.outputs()).unwrap();
    let mut run = RulesRun::new(sim, host(&dir.join("rules"), QvtMode::Auto));
    run.run_until(Pause::After(300 * S)).unwrap();
    let (sim, host) = run.into_parts();
    let detached = sim.into_state().detach();
    let prefix = (w.trace.text(), w.tape.text());
    let (trace, tape, csv) = std::thread::scope(|s| {
        s.spawn(|| {
            let fw = W::default();
            let sim = Simulation::resume(&cm, detached.into_state(), fw.outputs()).unwrap();
            let (r, host) = RulesRun::from_parts(sim, host).finish().unwrap();
            assert_eq!(host.stats().succeeded, 1);
            (fw.trace.text(), fw.tape.text(), r.measurements.to_csv())
        })
        .join()
        .unwrap()
    });
    assert_eq!(prefix.0 + &trace, want.trace);
    assert_eq!(prefix.1 + &tape, want.tape);
    assert_eq!(csv, want.csv);
}

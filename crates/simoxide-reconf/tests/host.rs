//! The QVTo host on running simulations: a rule mid-run, forks before and after it, a
//! deterministic resume (also of a detached state on another thread), aborting and failing
//! rules, rule sets, the natives, the modes, both flavours, and an SPD adjustment end to end.
//! The model is SimOxide's corpus model `h19_linking_resource` (a client and a server node on
//! a LAN); the rules are in `tests/rules/`.

mod common;

use common::*;
use qvto_simoxide::{ChangeKind, IdPolicy};
use simoxide_model::meta::feat as mf;
use simoxide_model::raw::PcmGraph;
use simoxide_reconf::{
    AbortPolicy, Bind, Edit, FailedRunChanges, Imports, Invocation, ModelRef, QvtMode, QvtoCore,
    QvtoRun, ReconfError, RuleSet, RunConfig, Session, Transformation,
};
use simoxide_sim::{
    ChangeError, CompiledModel, Fast, FlavourConfig, Mode, Outputs, Pause, SimConfig, Simulation,
    Slingshot,
};
use std::cell::RefCell;
use std::path::PathBuf;
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

/// Trace and tape of a run, and its result.
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

#[derive(PartialEq)]
struct Out {
    result: String,
    trace: String,
    tape: String,
}

impl std::fmt::Debug for Out {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} ({} trace bytes, {} tape bytes)",
            self.result.lines().next().unwrap_or(""),
            self.trace.len(),
            self.tape.len()
        )
    }
}

fn result(r: Result<simoxide_sim::RunResult, simoxide_sim::SimError>) -> String {
    match r {
        Ok(r) => format!(
            "t_end={} events={}\n{}",
            r.end_ns,
            r.events,
            r.measurements.to_csv()
        ),
        Err(e) => format!("error {e}"),
    }
}

fn out(w: &W, r: Result<simoxide_sim::RunResult, simoxide_sim::SimError>) -> Out {
    Out {
        result: result(r),
        trace: w.trace.text(),
        tape: w.tape.text(),
    }
}

/// The trace lines up to time `t_s` (seconds): the part before a change.
fn trace_until(trace: &str, t_s: f64) -> String {
    trace
        .lines()
        .take_while(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .ok()
                .and_then(|v| v.get("t").and_then(|t| t.as_f64()))
                .is_none_or(|t| t <= t_s)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn loaded() -> simoxide_model::Model {
    let dir = root().join("corpus/h19_linking_resource");
    let spec = simoxide_sim::RunSpec::load(&dir.join("run.json")).unwrap();
    spec.load_model(&dir).unwrap()
}

/// The time `clock.qvto` logged (`getSimulationTime()`).
fn logged_time(log: &str) -> f64 {
    log.strip_prefix("t=")
        .and_then(|r| r.split(' ').next())
        .and_then(|t| t.parse().ok())
        .unwrap_or_else(|| panic!("{log}"))
}

fn model() -> CompiledModel {
    CompiledModel::compile(loaded()).unwrap()
}

fn rules() -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/rules")
}

fn rule(name: &str) -> Transformation {
    core()
        .compile_file(&rules().join(name), &Imports::Folder)
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn qrun() -> QvtoRun {
    QvtoRun::new(core(), RunConfig::default()).unwrap()
}

fn uninterrupted(cm: &CompiledModel) -> Out {
    let w = W::default();
    let sim = Simulation::new(cm, sim_config(), w.outputs()).unwrap();
    let r = sim.finish();
    out(&w, r)
}

/// Runs to `t` s, then `f`, then to the end.
fn with_at(cm: &CompiledModel, t: i64, f: impl FnOnce(&mut Simulation<'_>)) -> Out {
    let w = W::default();
    let mut sim = Simulation::new(cm, sim_config(), w.outputs()).unwrap();
    sim.run_until(Pause::After(t * S)).unwrap();
    f(&mut sim);
    let r = sim.finish();
    out(&w, r)
}

/// The ids of the resource containers created in a journal.
fn new_ids(j: &qvto_simoxide::Journal) -> Vec<String> {
    j.changes
        .iter()
        .filter_map(|c| match &c.kind {
            ChangeKind::Set { feature, new, .. } if feature.0 == mf::identifier_Identifier_id.0 => {
                Some(format!("{new:?}"))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn the_state_is_created_on_first_use_and_runs_without_rules_are_unchanged() {
    let cm = model();
    let base = uninterrupted(&cm);
    let run = qrun();
    assert!(!run.is_started());
    let got = with_at(&cm, 100, |_| {});
    assert_eq!(got, base);
    // a rule that changes nothing leaves the run as it is
    let mut run = qrun();
    let t = rule("clock.qvto");
    let got = with_at(&cm, 100, |sim| {
        let t0 = std::time::Instant::now();
        let o = run.invoke(sim, &t, &Invocation::simulizar()).unwrap();
        let first = t0.elapsed();
        assert!(o.succeeded);
        assert!(o.change.is_none());
        assert_eq!(logged_time(&o.log), sim.state().now_ns() as f64 / 1e9);
        assert_eq!(o.log.trim_end().split(' ').nth(1), Some("containers=2"));
        let t0 = std::time::Instant::now();
        run.invoke(sim, &t, &Invocation::simulizar()).unwrap();
        eprintln!(
            "first invocation (state created) {first:?}, second {:?}",
            t0.elapsed()
        );
    });
    assert!(run.is_started());
    assert_eq!(run.invocations(), 2);
    assert_eq!(got, base);
}

#[test]
fn a_rule_mid_run_changes_the_simulation_like_a_direct_edit() {
    let cm = model();
    let base = uninterrupted(&cm);
    let t = rule("set/a_rate.qvto");
    let mut run = qrun();
    let by_rule = with_at(&cm, 100, |sim| {
        let o = run.invoke(sim, &t, &Invocation::simulizar()).unwrap();
        assert!(o.succeeded);
        let rep = o.change.as_ref().expect("applied");
        assert_eq!(rep.generation, 1);
        assert_eq!(sim.state().model().generation(), 1);
    });
    // the same change as a direct edit (F2's path)
    let by_edit = with_at(&cm, 100, |sim| {
        let mut session = Session::new();
        let mut e = Edit::new(sim, &mut session);
        let server = e.find("_h19_linking_resource_prs2").unwrap();
        e.set_rate(server, "4.0").unwrap();
        e.apply(sim).unwrap();
    });
    assert_eq!(by_rule, by_edit);
    assert_ne!(by_rule.trace, base.trace);
    assert_eq!(
        trace_until(&by_rule.trace, 100.0),
        trace_until(&base.trace, 100.0)
    );
}

#[test]
fn forks_before_and_after_a_rule() {
    let cm = model();
    let rate = rule("set/a_rate.qvto");
    let add = rule("add_container.qvto");
    let inv = Invocation::simulizar();

    // before: a fork and its parent both run the rule, and continue equally
    let (wa, wb) = (W::default(), W::default());
    let mut a = Simulation::new(&cm, sim_config(), wa.outputs()).unwrap();
    a.run_until(Pause::After(100 * S)).unwrap();
    let mut ra = qrun();
    ra.invoke(&mut a, &rate, &inv).unwrap();
    let mut b = a.fork(wb.outputs()).unwrap();
    let mut rb = ra.clone();
    let wtail = wa.trace.text().len();
    let oa = ra.invoke(&mut a, &add, &inv).unwrap();
    let ob = rb.invoke(&mut b, &add, &inv).unwrap();
    assert_eq!(oa.journal.changes, ob.journal.changes);
    let ids = new_ids(&oa.journal);
    assert_eq!(ids.len(), 1, "{:?}", oa.journal.changes);
    let (fa, fb) = (a.finish(), b.finish());
    let tail_a = wa.trace.text()[wtail..].to_string();
    assert_eq!(result(fa), result(fb));
    assert_eq!(tail_a, wb.trace.text());

    // after: the fork's QVTo state continues the id sequence; a fresh state would repeat it
    let w = W::default();
    let mut c = Simulation::new(&cm, sim_config(), w.outputs()).unwrap();
    c.run_until(Pause::After(100 * S)).unwrap();
    let mut rc = qrun();
    let first = rc.invoke(&mut c, &add, &inv).unwrap();
    c.run_until(Pause::After(150 * S)).unwrap();
    let mut d = c.fork(Outputs::default()).unwrap();
    let mut rd = rc.clone();
    let second_c = rc.invoke(&mut c, &add, &inv).unwrap();
    let second_d = rd.invoke(&mut d, &add, &inv).unwrap();
    assert_eq!(new_ids(&second_c.journal), new_ids(&second_d.journal));
    assert_ne!(new_ids(&first.journal), new_ids(&second_c.journal));
    assert!(second_c.log.contains("added3"), "{}", second_c.log);
    assert_eq!(c.state().model().generation(), 2);
    assert_eq!(result(c.finish()), result(d.finish()));
    let mut fresh = qrun();
    let mut e = Simulation::new(&cm, sim_config(), Outputs::default()).unwrap();
    e.run_until(Pause::After(100 * S)).unwrap();
    let o = fresh.invoke(&mut e, &add, &inv).unwrap();
    assert_eq!(new_ids(&o.journal), new_ids(&first.journal));
}

#[test]
fn resume_and_detach_continue_deterministically() {
    let cm = model();
    let rate = rule("set/a_rate.qvto");
    let add = rule("add_container.qvto");
    let inv = Invocation::simulizar();
    let w = W::default();
    let mut sim = Simulation::new(&cm, sim_config(), w.outputs()).unwrap();
    sim.run_until(Pause::After(100 * S)).unwrap();
    let mut run = qrun();
    run.invoke(&mut sim, &rate, &inv).unwrap();
    run.invoke(&mut sim, &add, &inv).unwrap();
    // a snapshot of the simulation and of the QVTo state, continued on another thread
    let state = sim.snapshot().detach();
    let qstate = run.clone();
    let mark = w.trace.text().len();
    let other = std::thread::scope(|s| {
        s.spawn(|| {
            let wr = W::default();
            let mut r = Simulation::resume(&cm, state.into_state(), wr.outputs()).unwrap();
            let mut q = qstate;
            r.run_until(Pause::After(150 * S)).unwrap();
            let o = q.invoke(&mut r, &add, &Invocation::simulizar()).unwrap();
            let res = result(r.finish());
            (res, wr.trace.text(), new_ids(&o.journal))
        })
        .join()
        .unwrap()
    });
    sim.run_until(Pause::After(150 * S)).unwrap();
    let o = run.invoke(&mut sim, &add, &inv).unwrap();
    let res = result(sim.finish());
    assert_eq!(res, other.0);
    assert_eq!(&w.trace.text()[mark..], other.1);
    assert_eq!(new_ids(&o.journal), other.2);
}

#[test]
fn an_aborting_rule_fails_the_run() {
    let cm = model();
    let t = rule("remove_allocation.qvto");
    let w = W::default();
    let mut sim = Simulation::new(&cm, sim_config(), w.outputs()).unwrap();
    sim.run_until(Pause::After(100 * S)).unwrap();
    let before = sim.fork(Outputs::default()).unwrap();
    let mut run = qrun();
    match run.invoke(&mut sim, &t, &Invocation::simulizar()) {
        Err(ReconfError::RunFailed {
            outcome,
            cause: ChangeError::Abort(_),
            error,
        }) => {
            assert!(outcome.succeeded);
            assert_eq!(
                outcome.log.trim_end(),
                "removed Allocation_Assembly_RemoteServer"
            );
            assert_eq!(error.at_ns, sim.state().now_ns());
        }
        r => panic!("{:?}", r.map(|o| o.log)),
    }
    assert_eq!(sim.state().phase(), simoxide_sim::Phase::Failed);
    let e = sim.finish().unwrap_err();
    assert!(e.message.contains("SimuLizar"), "{e}");
    // the state is unusable for changes now, a fork taken before is not
    assert!(before.finish().is_ok());
}

#[test]
fn an_aborting_rule_can_be_rejected_instead() {
    let cm = model();
    let base = uninterrupted(&cm);
    let t = rule("remove_allocation.qvto");
    let clock = rule("clock.qvto");
    let mut run = QvtoRun::new(
        core(),
        RunConfig {
            on_abort: AbortPolicy::Reject,
            ..RunConfig::default()
        },
    )
    .unwrap();
    let got = with_at(&cm, 100, |sim| {
        match run.invoke(sim, &t, &Invocation::simulizar()) {
            Err(ReconfError::Rejected {
                error: ChangeError::Abort(_),
                ..
            }) => {}
            r => panic!("{:?}", r.map(|o| o.log)),
        }
        assert_eq!(sim.state().model().generation(), 0);
        // the host goes on
        assert!(run.invoke(sim, &clock, &Invocation::simulizar()).is_ok());
    });
    assert_eq!(got, base);
}

#[test]
fn assert_fatal_means_not_applied() {
    let cm = model();
    let base = uninterrupted(&cm);
    let t = rule("fatal.qvto");
    // SimuLizar: the run reports "not applied"; what the rule changed before stays
    let mut run = qrun();
    let kept = with_at(&cm, 100, |sim| {
        let o = run.invoke(sim, &t, &Invocation::simulizar()).unwrap();
        assert!(!o.succeeded);
        assert_eq!(o.severity, 4);
        assert!(o.change.is_some());
    });
    assert_ne!(kept, base);
    // or the changes of a failed run are dropped
    let mut run = QvtoRun::new(
        core(),
        RunConfig {
            failed_runs: FailedRunChanges::Discard,
            ..RunConfig::default()
        },
    )
    .unwrap();
    let dropped = with_at(&cm, 100, |sim| {
        let o = run.invoke(sim, &t, &Invocation::simulizar()).unwrap();
        assert!(!o.succeeded);
        assert!(o.change.is_none());
    });
    assert_eq!(dropped, base);
}

#[test]
fn a_rule_set_runs_in_file_name_order() {
    let cm = model();
    let set = RuleSet::load_dir(core(), &rules().join("set")).unwrap();
    let names: Vec<&str> = set.rules.iter().map(|r| r.name()).collect();
    assert_eq!(names, ["rate", "check", "never"]);
    let mut run = qrun();
    with_at(&cm, 100, |sim| {
        let o = run
            .run_rules(sim, &set, &Invocation::simulizar())
            .map_err(|(_, e)| e)
            .unwrap();
        let ok: Vec<bool> = o.rules.iter().map(|r| r.succeeded).collect();
        assert_eq!(ok, [true, true, false]);
        assert!(!o.all_succeeded());
        assert!(o.changed());
        assert_eq!(o.rules[1].log.trim_end(), "b: rate is 4.0");
        assert!(
            o.rules[2].log.contains("c: not applied"),
            "{}",
            o.rules[2].log
        );
    });
}

#[test]
fn profiles_library_is_a_stub_that_throws() {
    let cm = model();
    let t = rule("profiles.qvto");
    let mut run = qrun();
    with_at(&cm, 100, |sim| {
        let o = run.invoke(sim, &t, &Invocation::simulizar()).unwrap();
        assert!(!o.succeeded);
        assert!(
            o.message.contains("not supported by SimOxide"),
            "{} / {}",
            o.message,
            o.log
        );
    });
}

#[test]
fn bindings_and_cores_are_checked() {
    let cm = model();
    let t = rule("clock.qvto");
    let mut run = qrun();
    with_at(&cm, 100, |sim| {
        let inv =
            Invocation::simulizar().bind("env", Bind::Models(vec![ModelRef::Uri("x".into())]));
        assert!(matches!(
            run.invoke(sim, &t, &inv),
            Err(ReconfError::Binding(_))
        ));
        let inv = Invocation::simulizar().bind(
            "env",
            Bind::Models(vec![ModelRef::Uri(
                "h19_linking_resource.resourceenvironment".into(),
            )]),
        );
        assert!(run.invoke(sim, &t, &inv).unwrap().succeeded);
        let other = QvtoCore::new().unwrap();
        let t2 = other
            .compile_file(&rules().join("clock.qvto"), &Imports::Folder)
            .unwrap();
        assert!(matches!(
            run.invoke(sim, &t2, &Invocation::simulizar()),
            Err(ReconfError::WrongCore)
        ));
    });
}

#[test]
fn fast_mode_gives_the_same_journal_here() {
    let cm = model();
    let t = rule("set/a_rate.qvto");
    let mut journals = Vec::new();
    for mode in [QvtMode::Fast, QvtMode::Compat] {
        let mut run = QvtoRun::new(
            core(),
            RunConfig {
                mode,
                ..RunConfig::default()
            },
        )
        .unwrap();
        with_at(&cm, 100, |sim| {
            journals.push(
                run.invoke(sim, &t, &Invocation::simulizar())
                    .unwrap()
                    .journal
                    .changes,
            );
        });
    }
    assert_eq!(journals[0], journals[1]);
    assert_eq!(
        QvtoRun::new(core(), RunConfig::default()).unwrap().mode(),
        qvto_runtime::Mode::Compat
    );
}

#[test]
fn the_slingshot_flavour_runs_rules_and_refuses_changes_for_now() {
    let cm = CompiledModel::compile_flavour(loaded(), FlavourConfig::SLINGSHOT).unwrap();
    let cfg = SimConfig {
        mode: Mode::Fast,
        ..sim_config()
    };
    let mut sim = Simulation::<Fast, Slingshot>::create(&cm, cfg, Outputs::default()).unwrap();
    sim.run_until(Pause::After(100 * S)).unwrap();
    let mut run = qrun();
    let o = run
        .invoke(&mut sim, &rule("clock.qvto"), &Invocation::slingshot())
        .unwrap();
    assert_eq!(logged_time(&o.log), sim.state().now_ns() as f64 / 1e9);
    match run.invoke(&mut sim, &rule("set/a_rate.qvto"), &Invocation::slingshot()) {
        Err(ReconfError::Rejected {
            error: ChangeError::Unsupported(_),
            ..
        }) => {}
        r => panic!("{:?}", r.map(|o| o.log)),
    }
    assert!(sim.finish().is_ok());
}

#[test]
fn ids_follow_the_policy() {
    let cm = model();
    let add = rule("add_container.qvto");
    let mut none = QvtoRun::new(
        core(),
        RunConfig {
            ids: IdPolicy::None,
            ..RunConfig::default()
        },
    )
    .unwrap();
    with_at(&cm, 100, |sim| {
        let o = none.invoke(sim, &add, &Invocation::simulizar()).unwrap();
        assert!(new_ids(&o.journal).is_empty());
    });
    let mut seeded = QvtoRun::new(
        core(),
        RunConfig {
            ids: IdPolicy::Uuid { seed: 7 },
            ..RunConfig::default()
        },
    )
    .unwrap();
    with_at(&cm, 100, |sim| {
        let o = seeded.invoke(sim, &add, &Invocation::simulizar()).unwrap();
        let ids = new_ids(&o.journal);
        assert_eq!(ids.len(), 1);
        assert!(ids[0].contains("\"_"), "{ids:?}");
    });
}

#[cfg(feature = "spd")]
mod spd {
    use super::*;
    use qvto_model::{Datum, ModelStore};
    use simoxide_reconf::{AdjustHooks, QvtoAdjuster, SimStore};
    use simoxide_sim::Exact;
    use simoxide_spd::log::VecLog;
    use simoxide_spd::sim::{SpdRun, spd_of};
    use simoxide_spd::{AdjustmentRequest, Quirks, Spd};

    /// P2's part, minimal: `Configuration.enactedPolicy` is the request's policy.
    struct EnactPolicy;

    impl AdjustHooks for EnactPolicy {
        fn before(&mut self, store: &mut SimStore<'_>, spd: &Spd, req: &AdjustmentRequest) {
            let cfg = spd.model().configuration.expect("semantic configuration");
            let policy = spd.model().policies[req.policy].obj;
            store
                .set(
                    cfg.0,
                    qvto_model::FeatureId(mf::Configuration_enactedPolicy.0),
                    Datum::Obj(policy.0),
                )
                .unwrap();
        }
    }

    /// Slingshot's SPD minimal example: a time-triggered policy scales the application
    /// server's elastic infrastructure out by one at t = 50 s; `MainTransformation.qvto`
    /// adds the container `AppServer_1`, connects it and allocates a copy of the assembly, as
    /// in the reference runner's `adjust` record.
    #[test]
    fn spd_adjustment_end_to_end() {
        let dir = root().join("reference/slingshot/models/spd_minimal");
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.extension().is_some_and(|x| {
                    ["usagemodel", "allocation", "spd", "semanticspd"]
                        .contains(&x.to_str().unwrap())
                })
            })
            .collect();
        files.sort();
        let model = simoxide_model::load_files(&files);
        let spd = spd_of(&model, Quirks::fixed()).unwrap().expect("an SPD");
        let cm = CompiledModel::compile(model).unwrap();
        let cfg = SimConfig {
            seed: 1,
            max_sim_time: 100,
            ..SimConfig::default()
        };
        let sim = Simulation::<Exact>::new(&cm, cfg, Outputs::default()).unwrap();
        let main = core()
            .compile_file(
                &qroot()
                    .join("qvto-testkit/corpus/spd/transformations/spd/MainTransformation.qvto"),
                &Imports::Roots(vec![
                    qroot().join("qvto-testkit/corpus/spd/transformations"),
                ]),
            )
            .unwrap();
        // fast mode: in compat mode without the Java run's identity hashes, the wiring moves a
        // container within `LinkingResource.connectedResourceContainers`, on which SimuLizar's
        // router throws (MC-2.3) and the run fails; the SimuLizar flavour's SPD needs the fixed
        // router semantics there (P2)
        let run = QvtoRun::new(
            core(),
            RunConfig {
                mode: QvtMode::Fast,
                ..RunConfig::default()
            },
        )
        .unwrap();
        let mut adjuster = QvtoAdjuster::new(run, main).with_hooks(EnactPolicy);
        let mut r = SpdRun::new(sim, &spd).unwrap();
        let mut log = VecLog::default();
        r.run(&mut adjuster, &mut log).unwrap();
        let last = adjuster.last.as_ref().expect("an adjustment ran");
        assert!(last.succeeded, "{} {}", last.message, last.log);
        assert!(
            last.log.contains("Used SPD for transformation"),
            "{}",
            last.log
        );
        let g = r.sim.model_graph();
        let containers: Vec<String> = g
            .objects()
            .iter()
            .enumerate()
            .filter(|(_, o)| o.class == simoxide_model::meta::class::ResourceContainer)
            .filter_map(|(i, _)| {
                g.str_attr(
                    simoxide_model::raw::ObjId(i as u32),
                    mf::NamedElement_entityName,
                )
                .map(String::from)
            })
            .collect();
        assert!(
            containers.contains(&"AppServer_1".to_string()),
            "{containers:?}"
        );
        assert_eq!(r.sim.state().model().generation(), 1);
        let group = &spd.model().groups[0];
        assert_eq!(r.state.membership.group_size(group), Some(2));
        let (res, _, summary) = r.finish().unwrap();
        assert_eq!(summary.adjustments_ok, 1);
        assert!(res.events > 0);
    }
}

#[test]
fn repeated_runs_do_not_grow_the_state() {
    let cm = model();
    let clock = rule("clock.qvto");
    let rate = rule("set/a_rate.qvto");
    let mut run = qrun();
    with_at(&cm, 100, |sim| {
        run.invoke(sim, &clock, &Invocation::simulizar()).unwrap();
        run.invoke(sim, &rate, &Invocation::simulizar()).unwrap();
        let count = |run: &mut QvtoRun, sim: &Simulation<'_>| {
            run.with_scratch_store(sim, |s| {
                use qvto_model::ModelStore;
                s.resource_count()
            })
        };
        let n0 = count(&mut run, sim);
        let t0 = std::time::Instant::now();
        for _ in 0..1000 {
            run.invoke(sim, &clock, &Invocation::simulizar()).unwrap();
            run.invoke(sim, &rate, &Invocation::simulizar()).unwrap();
        }
        eprintln!("2000 invocations: {:?}", t0.elapsed());
        assert_eq!(count(&mut run, sim), n0);
    });
}

/// Prints what the host costs (release build): the core, a compilation, the per-run state.
#[test]
fn costs() {
    let t0 = std::time::Instant::now();
    let core = QvtoCore::new().unwrap();
    let t_core = t0.elapsed();
    let t0 = std::time::Instant::now();
    let t = core
        .compile_file(&rules().join("set/a_rate.qvto"), &Imports::Folder)
        .unwrap();
    let t_compile = t0.elapsed();
    let cm = model();
    let mut sim = Simulation::new(&cm, sim_config(), Outputs::default()).unwrap();
    sim.run_until(Pause::After(100 * S)).unwrap();
    let mut run = QvtoRun::new(&core, RunConfig::default()).unwrap();
    let t0 = std::time::Instant::now();
    run.create_model(&sim, "simoxide:/prm");
    let t_state = t0.elapsed();
    let t0 = std::time::Instant::now();
    let _g = sim.model_graph().clone();
    let t_graph = t0.elapsed();
    let t0 = std::time::Instant::now();
    let fork = run.clone();
    let t_fork = t0.elapsed();
    let o = run.invoke(&mut sim, &t, &Invocation::simulizar()).unwrap();
    eprintln!(
        "core {t_core:?}, compile {t_compile:?}, per-run state (session clone, graph copy) \
         {t_state:?} (graph copy {t_graph:?}), fork of the state {t_fork:?}, rule run {} µs, \
         change {} µs",
        o.run_micros,
        o.change.map_or(0, |c| c.micros)
    );
    drop(fork);
}

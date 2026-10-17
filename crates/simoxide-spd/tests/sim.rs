//! The driver on simoxide-sim: measurement observers deliver the tuples of the series the
//! triggers read, time events and requests are external notes, the run's outputs stay those
//! of the plain run (P1 executes no adjustment), and a fork continues like the original.

use simoxide_model::raw::ObjId;
use simoxide_sim::{CompiledModel, Exact, Outputs, RunSpec, Simulation};
use simoxide_spd::log::{Record, VecLog};
use simoxide_spd::model::*;
use simoxide_spd::sim::{NoAdjustment, SpdRun};
use simoxide_spd::{Quirks, Spd};
use std::path::Path;

fn corpus(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus")
        .join(name)
}

fn find(m: &simoxide_model::Model, id: &str) -> ObjId {
    (0..m.graph.object_count() as u32)
        .map(ObjId)
        .find(|&o| m.graph.id(o) == Some(id))
        .unwrap()
}

fn trig(id: &str, op: RelOp, stimulus: Stimulus, expected: Expected) -> Trigger {
    Trigger {
        obj: ObjId(0),
        id: id.into(),
        kind: TriggerKind::Value {
            op,
            stimulus,
            expected,
        },
    }
}

fn pol(id: &str, t: Trigger, constraints: Vec<PolicyConstraint>) -> Policy {
    Policy {
        obj: ObjId(0),
        id: id.into(),
        name: id.into(),
        active: true,
        group: Some(0),
        adjustment: Adjustment::Step { step: 1 },
        trigger: Some(t),
        constraints,
    }
}

/// h20 (one processing-sharing container with four cores) with a CPU, a task-count, a
/// response-time and a time policy on its container.
fn setup(q: Quirks) -> (CompiledModel, Spd, ObjId) {
    let dir = corpus("h20_ps_multicore");
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let model = spec.load_model(&dir).unwrap();
    let rc = find(&model, "_h20_ps_multicore_rc1");
    let sig = find(&model, "_h20_ps_multicore_sig1");
    let cooldown = vec![PolicyConstraint::Cooldown {
        time: 5.0,
        max_operations: 0,
    }];
    let m = SpdModel {
        obj: ObjId(0),
        id: "spd".into(),
        policies: vec![
            pol(
                "cpu",
                trig(
                    "c",
                    RelOp::GreaterThan,
                    Stimulus::CpuUtilization {
                        aggregation: Aggregation::Max,
                    },
                    Expected::Percentage(50.0),
                ),
                cooldown.clone(),
            ),
            pol(
                "tasks",
                trig(
                    "n",
                    RelOp::GreaterThanOrEqualTo,
                    Stimulus::TaskCount {
                        aggregation: Aggregation::Max,
                    },
                    Expected::Count(2),
                ),
                cooldown.clone(),
            ),
            pol(
                "rt",
                trig(
                    "r",
                    RelOp::GreaterThan,
                    Stimulus::OperationResponseTime {
                        signature: Some(sig),
                        signature_name: "sig1".into(),
                    },
                    Expected::Time(0.5),
                ),
                cooldown,
            ),
            pol(
                "time",
                trig(
                    "t",
                    RelOp::EqualTo,
                    Stimulus::SimulationTime,
                    Expected::Time(3.0),
                ),
                vec![],
            ),
        ],
        groups: vec![TargetGroup {
            obj: ObjId(0),
            id: "g".into(),
            name: "g".into(),
            kind: GroupKind::ElasticInfrastructure { unit: Some(rc) },
            constraints: vec![],
        }],
        configuration: None,
    };
    let spd = Spd::new(m, q).unwrap();
    (CompiledModel::compile(model).unwrap(), spd, rc)
}

fn cfg() -> simoxide_sim::SimConfig {
    let spec = RunSpec::load(&corpus("h20_ps_multicore").join("run.json")).unwrap();
    let mut c = spec.sim_config("spd");
    c.max_measurements = 300;
    c
}

fn spd_run<'m, 's>(cm: &'m CompiledModel, spd: &'s Spd, rc: ObjId) -> SpdRun<'m, 's, Exact> {
    let sim = Simulation::<Exact>::new(cm, cfg(), Outputs::default()).unwrap();
    let mut r = SpdRun::new(sim, spd).unwrap();
    // no semantic configuration in the corpus model: the group is its unit container
    r.state
        .membership
        .cfgs
        .push(GroupCfg::ElasticInfrastructure {
            unit: Some(rc),
            elements: vec![rc],
        });
    r
}

fn requests(log: &VecLog) -> Vec<(String, f64)> {
    log.records
        .iter()
        .filter_map(|r| match r {
            Record::Eval {
                policy,
                t,
                request: true,
                ..
            } => Some((policy.to_string(), *t)),
            _ => None,
        })
        .collect()
}

#[test]
fn observers_deliver_measurements_and_outputs_are_unchanged() {
    for q in [Quirks::fixed(), Quirks::slingshot()] {
        let (cm, spd, rc) = setup(q);
        let plain = simoxide_sim::run(&cm, cfg(), Outputs::default()).unwrap();
        let mut r = spd_run(&cm, &spd, rc);
        let mut log = VecLog::default();
        r.run(&mut NoAdjustment, &mut log).unwrap();
        let (res, st, summary) = r.finish().unwrap();
        assert_eq!(res.measurements.to_csv(), plain.measurements.to_csv());
        assert_eq!(res.main_count, plain.main_count);
        // one time event plus the request notes (requests of the stopping event are never
        // dispatched)
        assert_eq!(
            res.events,
            plain.events + 1 + summary.requests - summary.pending
        );
        let fired = requests(&log);
        for p in ["cpu", "tasks", "rt", "time"] {
            assert!(fired.iter().any(|(x, _)| x == p), "{p} never fired: {q:?}");
        }
        assert!(fired.contains(&("time".to_string(), 3.0)));
        // NoAdjustment: every request reported as not executed
        assert_eq!(summary.adjustments_ok, 0);
        assert!(summary.pending < 10);
        assert!(st.evaluations > 300);
        let adj = log
            .records
            .iter()
            .filter(|r| matches!(r, Record::Adjust { ok: false, .. }))
            .count() as u64;
        assert_eq!(adj, summary.requests - summary.pending);
        if q == Quirks::fixed() {
            // fixed Q2: nothing is committed by a failed adjustment, so the cooldown never
            // starts and the policies fire on many tuples
            assert!(fired.iter().filter(|(p, _)| p == "tasks").count() > 5);
        }
    }
}

#[test]
fn fork_equals_uninterrupted_run() {
    for q in [Quirks::fixed(), Quirks::slingshot()] {
        let (cm, spd, rc) = setup(q);
        let mut whole = spd_run(&cm, &spd, rc);
        let mut wlog = VecLog::default();
        whole.run(&mut NoAdjustment, &mut wlog).unwrap();
        let (wres, wst, _) = whole.finish().unwrap();

        let mut a = spd_run(&cm, &spd, rc);
        let mut alog = VecLog::default();
        a.run_until(&mut NoAdjustment, &mut alog, Some(4_000_000_000))
            .unwrap();
        let split = alog.records.len();
        let mut b = a.fork(Outputs::default()).unwrap();
        let mut blog = VecLog::default();
        a.run(&mut NoAdjustment, &mut alog).unwrap();
        b.run(&mut NoAdjustment, &mut blog).unwrap();
        let (ares, ast, _) = a.finish().unwrap();
        let (bres, bst, _) = b.finish().unwrap();
        assert_eq!(alog.records, wlog.records);
        assert_eq!(blog.records, wlog.records[split..]);
        assert_eq!(ast, wst);
        assert_eq!(bst, wst);
        for r in [&ares, &bres] {
            assert_eq!(r.measurements.to_csv(), wres.measurements.to_csv());
            assert_eq!(r.events, wres.events);
        }
    }
}

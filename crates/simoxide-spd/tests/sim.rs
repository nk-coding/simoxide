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

/// A snapshot written at several points, restored into a fresh run each time (the same
/// interpreter), continues like the uninterrupted run: decision log, SPD state, measurements,
/// events (`DESIGN-SPD.md` N1).
#[test]
fn snapshot_equals_uninterrupted_run() {
    use simoxide_sim::snapshot::{Encoding, SnapshotOptions};
    for q in [Quirks::fixed(), Quirks::slingshot()] {
        let (cm, spd, rc) = setup(q);
        let mut whole = spd_run(&cm, &spd, rc);
        let mut wlog = VecLog::default();
        whole.run(&mut NoAdjustment, &mut wlog).unwrap();
        let (wres, wst, wsum) = whole.finish().unwrap();
        for enc in [Encoding::Postcard, Encoding::Json] {
            let mut r = spd_run(&cm, &spd, rc);
            let mut log = VecLog::default();
            for t in [
                0,
                1_500_000_000,
                3_000_000_000,
                3_000_000_001,
                4_200_000_000,
            ] {
                r.run_until(&mut NoAdjustment, &mut log, Some(t)).unwrap();
                let mut bytes = Vec::new();
                r.snapshot_with(&mut bytes, &SnapshotOptions { encoding: enc })
                    .unwrap();
                let state = r.state.clone();
                drop(r);
                r = SpdRun::restore_from(&bytes[..], &cm, &spd, Outputs::default()).unwrap();
                assert_eq!(r.state, state);
                // deterministic and lossless
                let mut again = Vec::new();
                r.snapshot_with(&mut again, &SnapshotOptions { encoding: enc })
                    .unwrap();
                assert!(again == bytes, "{q:?} {enc:?} at {t}");
            }
            r.run(&mut NoAdjustment, &mut log).unwrap();
            let (res, st, sum) = r.finish().unwrap();
            assert_eq!(log.records, wlog.records, "{q:?} {enc:?}");
            assert_eq!(st, wst);
            assert_eq!(sum, wsum);
            assert_eq!(res.measurements.to_csv(), wres.measurements.to_csv());
            assert_eq!(res.events, wres.events);
        }
        // another interpreter is refused
        let (cm2, spd2, _) = setup(q);
        let mut r = spd_run(&cm, &spd, rc);
        r.run_until(
            &mut NoAdjustment,
            &mut VecLog::default(),
            Some(1_000_000_000),
        )
        .unwrap();
        let mut bytes = Vec::new();
        r.snapshot_to(&mut bytes).unwrap();
        assert!(SpdRun::<Exact>::restore_from(&bytes[..], &cm2, &spd2, Outputs::default()).is_ok());
        let other = Spd::new(
            SpdModel {
                obj: ObjId(0),
                id: "x".into(),
                policies: vec![],
                groups: vec![],
                configuration: None,
            },
            q,
        )
        .unwrap();
        let e = SpdRun::<Exact>::restore_from(&bytes[..], &cm, &other, Outputs::default())
            .err()
            .expect("other SPD");
        assert!(e.to_string().contains("scaling policy"), "{e}");
        // a plain simulation snapshot has no SPD state
        let mut sim = Simulation::<Exact>::new(&cm, cfg(), Outputs::default()).unwrap();
        sim.run_until(simoxide_sim::Pause::Events(10)).unwrap();
        let mut plain = Vec::new();
        sim.snapshot_to(&mut plain).unwrap();
        let e = SpdRun::<Exact>::restore_from(&plain[..], &cm, &spd, Outputs::default())
            .err()
            .expect("no SPD state");
        assert!(e.to_string().contains("no SPD state"), "{e}");
    }
}

/// `SpdRun::run_while` with a pause condition (`DESIGN-SPD.md` N2): a time condition pauses
/// where `run_until` pauses (the same snapshot), a series condition after the SPD work of the
/// event whose tuple made it hold; the continuation equals the uninterrupted run.
#[test]
fn pause_conditions_in_an_spd_run() {
    use simoxide_sim::sim::watch::parse_condition;
    use simoxide_sim::{Condition, Pause, Status, Watch};
    let (cm, spd, rc) = setup(Quirks::fixed());
    let mut whole = spd_run(&cm, &spd, rc);
    let mut wlog = VecLog::default();
    whole.run(&mut NoAdjustment, &mut wlog).unwrap();
    let (wres, wst, _) = whole.finish().unwrap();
    // time
    let mut a = spd_run(&cm, &spd, rc);
    a.run_until(
        &mut NoAdjustment,
        &mut VecLog::default(),
        Some(3_000_000_001),
    )
    .unwrap();
    let mut b = spd_run(&cm, &spd, rc);
    let mut w = Watch::new(vec![Condition::At(Pause::After(3_000_000_000))]);
    w.arm(&b.sim).unwrap();
    let st = b
        .run_while(&mut NoAdjustment, &mut VecLog::default(), |s| {
            w.keep_going(s)
        })
        .unwrap();
    assert_eq!(st, Status::Paused);
    let (mut x, mut y) = (Vec::new(), Vec::new());
    a.snapshot_to(&mut x).unwrap();
    b.snapshot_to(&mut y).unwrap();
    assert!(x == y);
    // a response time above 0.5 s, then on to the end
    let mut r = spd_run(&cm, &spd, rc);
    let mut log = VecLog::default();
    let mut w = Watch::new(vec![
        parse_condition("rt[_h20_ps_multicore_sig1] > 0.5").unwrap(),
    ]);
    w.arm(&r.sim).unwrap();
    let st = r
        .run_while(&mut NoAdjustment, &mut log, |s| w.keep_going(s))
        .unwrap();
    assert_eq!(st, Status::Paused);
    let f = w.fired().unwrap();
    assert!(f.reason.contains("Response Time"), "{}", f.reason);
    assert_eq!(f.events, r.sim.state().events());
    r.run(&mut NoAdjustment, &mut log).unwrap();
    let (res, st, _) = r.finish().unwrap();
    assert_eq!(log.records, wlog.records);
    assert_eq!(st, wst);
    assert_eq!(res.measurements.to_csv(), wres.measurements.to_csv());
}

/// Enforcing policies at a resume (Slingshot's incoming policies): unknown names are refused
/// and leave the run unchanged; known ones queue one request each at the current time, which
/// the adjuster executes (here: not executed) and the log records as enforced; nothing else
/// changes (enforced requests commit no adjustor state).
#[test]
fn enforcing_policies_queues_requests() {
    use simoxide_spd::log::Record;
    use simoxide_spd::resume::EnforceError;
    let (cm, spd, rc) = setup(Quirks::fixed());
    let mut whole = spd_run(&cm, &spd, rc);
    let mut wlog = VecLog::default();
    whole.run(&mut NoAdjustment, &mut wlog).unwrap();
    let (wres, wst, wsummary) = whole.finish().unwrap();
    let mut r = spd_run(&cm, &spd, rc);
    let mut log = VecLog::default();
    r.run_until(&mut NoAdjustment, &mut log, Some(2_000_000_000))
        .unwrap();
    let before = r.state.clone();
    assert_eq!(
        r.enforce_policies(&["nope"]),
        Err(EnforceError::UnknownPolicy("nope".into()))
    );
    assert_eq!(r.state, before);
    let now = r.sim.state().now_ns() as f64 / 1e9;
    r.enforce_policies(&["cpu", "time"]).unwrap();
    r.run(&mut NoAdjustment, &mut log).unwrap();
    let (res, st, summary) = r.finish().unwrap();
    let enforced: Vec<&Record> = log
        .records
        .iter()
        .filter(|r| matches!(r, Record::Adjust { enforced: true, .. }))
        .collect();
    assert_eq!(enforced.len(), 2);
    for (rec, name) in enforced.iter().zip(["cpu", "time"]) {
        match rec {
            Record::Adjust {
                t,
                policy_name,
                ok: false,
                ..
            } => {
                assert_eq!(*t, now);
                assert_eq!(&**policy_name, name);
            }
            o => panic!("{o:?}"),
        }
    }
    let rest: Vec<Record> = log
        .records
        .iter()
        .filter(|r| !matches!(r, Record::Adjust { enforced: true, .. }))
        .cloned()
        .collect();
    assert_eq!(rest, wlog.records);
    assert_eq!(st.policies, wst.policies);
    assert_eq!(st.requests, wst.requests + 2);
    assert_eq!(summary.pending, wsummary.pending);
    assert_eq!(res.measurements.to_csv(), wres.measurements.to_csv());
}

/// Min-size scale-in under Slingshot quirks commits cooldown (Q2) without a size check (Q9),
/// so the DataProcessing scale-out on the same spike never requests. Fixed quirks drop that
/// no-op in `GroupSize` and both groups request a scale-out.
#[test]
fn rmuc_fixed_quirks_request_both_scale_outs() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus-spd/rmuc_app_metrics");
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let mut c = spec.sim_config("spd");
    c.seed = 1;
    c.max_sim_time = 30;
    for (q, data_processing_scales_out) in [(Quirks::fixed(), true), (Quirks::slingshot(), false)] {
        let model = spec.load_model(&dir).unwrap();
        let spd_model = SpdModel::from_graph(&model.graph).expect("spd");
        let cm = CompiledModel::compile_flavour(
            model,
            simoxide_sim::FlavourConfig::slingshot(simoxide_sim::flavour::Quirks::NONE),
        )
        .unwrap();
        let spd = Spd::new(spd_model, q).unwrap();
        c.mode = simoxide_sim::Mode::Fast;
        let sim = Simulation::<simoxide_sim::Fast, simoxide_sim::Slingshot>::create(
            &cm,
            c.clone(),
            Outputs::default(),
        )
        .unwrap();
        let mut r = SpdRun::new(sim, &spd).unwrap();
        let mut log = VecLog::default();
        r.run(&mut NoAdjustment, &mut log).unwrap();
        let mut names = std::collections::HashMap::<String, String>::new();
        for rec in &log.records {
            if let Record::Policy { id, name, .. } = rec {
                names.insert(id.to_string(), name.to_string());
            }
        }
        let requested: Vec<String> = log
            .records
            .iter()
            .filter_map(|rec| match rec {
                Record::Eval {
                    policy,
                    request: true,
                    ..
                } => names.get(policy.as_ref()).cloned(),
                _ => None,
            })
            .collect();
        let has = |n: &str| requested.iter().any(|p| p == n);
        let mut unique: Vec<&str> = requested.iter().map(String::as_str).collect();
        unique.sort_unstable();
        unique.dedup();
        assert!(has("scaleOutDeviceCommunication"), "{q:?} {unique:?}");
        assert_eq!(
            has("scaleOutDataProcessing"),
            data_processing_scales_out,
            "{q:?} {unique:?}"
        );
    }
}

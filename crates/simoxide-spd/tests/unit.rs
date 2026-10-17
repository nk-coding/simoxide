//! V1 (`DESIGN-SPD.md` §4.2): filters, constraints, aggregators, comparator and compound
//! triggers against hand-computed expectations from Slingshot's Java code, with a fake host
//! (no simulator). Edge cases of §1.2: `==` at `coolDownEnd`, `t % delta == 0`, the EMA emit
//! rule, the 10-value window; each quirk flag in both settings.

use simoxide_model::raw::ObjId;
use simoxide_spd::log::{Record, VecLog};
use simoxide_spd::model::*;
use simoxide_spd::{
    AdjustmentRequest, Measurement, Metric, Quirks, Spd, SpdHost, SpdState, TimerId,
};

#[derive(Default)]
struct Host {
    now: f64,
    scheduled: Vec<(f64, TimerId)>,
    requests: Vec<AdjustmentRequest>,
}

impl SpdHost for Host {
    fn now(&self) -> f64 {
        self.now
    }
    fn schedule(&mut self, at: f64, timer: TimerId) {
        self.scheduled.push((at, timer));
    }
    fn request(&mut self, req: AdjustmentRequest) {
        self.requests.push(req);
    }
}

const UNIT: ObjId = ObjId(10);
const C2: ObjId = ObjId(11);
const OUTSIDE: ObjId = ObjId(12);

fn ei_group(id: &str, constraints: Vec<TargetConstraint>) -> TargetGroup {
    TargetGroup {
        obj: ObjId(1000 + id.len() as u32),
        id: id.into(),
        name: format!("group {id}").into(),
        kind: GroupKind::ElasticInfrastructure { unit: Some(UNIT) },
        constraints,
    }
}

fn time_trigger(id: &str, op: RelOp, t: f64) -> Trigger {
    Trigger {
        obj: ObjId(0),
        id: id.into(),
        kind: TriggerKind::Value {
            op,
            stimulus: Stimulus::SimulationTime,
            expected: Expected::Time(t),
        },
    }
}

fn value_trigger(id: &str, op: RelOp, stimulus: Stimulus, expected: Expected) -> Trigger {
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

fn composed(op: LogicalOp, children: Vec<Trigger>) -> Trigger {
    Trigger {
        obj: ObjId(0),
        id: "composed".into(),
        kind: TriggerKind::Composed { op, children },
    }
}

fn policy(id: &str, group: usize, adjustment: Adjustment, trigger: Trigger) -> Policy {
    Policy {
        obj: ObjId(0),
        id: id.into(),
        name: id.into(),
        active: true,
        group: Some(group),
        adjustment,
        trigger: Some(trigger),
        constraints: Vec::new(),
    }
}

fn step(n: i64) -> Adjustment {
    Adjustment::Step { step: n }
}

fn model(policies: Vec<Policy>, groups: Vec<TargetGroup>) -> SpdModel {
    SpdModel {
        obj: ObjId(0),
        id: "spd".into(),
        policies,
        groups,
        configuration: None,
    }
}

fn membership(elements: Vec<ObjId>) -> Membership {
    Membership {
        cfgs: vec![GroupCfg::ElasticInfrastructure {
            unit: Some(UNIT),
            elements,
        }],
        allocation: vec![(ObjId(20), UNIT)],
        environment: vec![UNIT, C2, OUTSIDE],
        provided: vec![(ObjId(20), vec![ObjId(30)])],
    }
}

struct Run {
    spd: Spd,
    st: SpdState,
    host: Host,
    log: VecLog,
}

impl Run {
    fn new(m: SpdModel, q: Quirks) -> Run {
        let spd = Spd::new(m, q).unwrap();
        let st = spd.state(membership(vec![UNIT, C2]));
        let mut host = Host::default();
        spd.start(&mut host);
        Run {
            spd,
            st,
            host,
            log: VecLog::default(),
        }
    }

    /// Dispatches the scheduled time events in (time, scheduling) order.
    fn run_timers(&mut self) {
        let mut ts = self.host.scheduled.clone();
        ts.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (t, id) in ts {
            self.host.now = t;
            self.spd
                .on_timer(&mut self.st, &mut self.host, id, &mut self.log);
        }
    }

    fn timer(&mut self, id: TimerId) {
        self.host.now = self.spd.timers()[id as usize].time;
        self.spd
            .on_timer(&mut self.st, &mut self.host, id, &mut self.log);
    }

    fn measure(&mut self, t: f64, m: Measurement) {
        self.host.now = t;
        self.spd
            .on_measurement(&mut self.st, &mut self.host, &m, &mut self.log);
    }

    /// Times of the requests of `policy` (model index).
    fn fired(&self, policy: usize) -> Vec<f64> {
        self.host
            .requests
            .iter()
            .filter(|r| r.policy == policy)
            .map(|r| r.time)
            .collect()
    }

    fn evals(&self, policy: &str) -> Vec<(f64, Vec<(String, String)>, bool)> {
        self.log
            .records
            .iter()
            .filter_map(|r| match r {
                Record::Eval {
                    t,
                    policy: p,
                    filters,
                    request,
                    ..
                } if &**p == policy => Some((
                    *t,
                    filters
                        .iter()
                        .map(|f| (f.name.to_string(), f.result.to_string()))
                        .collect(),
                    *request,
                )),
                _ => None,
            })
            .collect()
    }
}

fn cpu(container: ObjId, metric: Metric, t: f64, v: f64, source: u32) -> Measurement {
    Measurement {
        point: PointRef {
            kind: PointKind::ActiveResource { container },
            class: "ActiveResourceMeasuringPoint",
        },
        metric,
        point_in_time: t,
        value: v,
        source,
    }
}

// ------------------------------------------------------------------------------- time triggers

#[test]
fn simulation_time_operators() {
    let g = vec![ei_group("g", vec![])];
    let ops = [
        (RelOp::EqualTo, true),
        (RelOp::GreaterThanOrEqualTo, true),
        (RelOp::LessThanOrEqualTo, true),
        (RelOp::GreaterThan, false),
        (RelOp::LessThan, false),
    ];
    for (op, fires) in ops {
        for q in [Quirks::fixed(), Quirks::slingshot()] {
            let m = model(
                vec![policy("p", 0, step(1), time_trigger("t", op, 50.0))],
                g.clone(),
            );
            let mut r = Run::new(m, q);
            assert_eq!(r.host.scheduled, vec![(50.0, 0)]);
            r.run_timers();
            assert_eq!(r.fired(0).len(), usize::from(fires), "{op:?}");
            let e = r.evals("p");
            assert_eq!(e.len(), 1);
            if !fires {
                assert_eq!(
                    e[0].1[1],
                    ("SimulationTimeChecker".into(), "disregard".into())
                );
            }
        }
    }
}

#[test]
fn time_event_message_like_slingshot() {
    let m = model(
        vec![
            policy("a", 0, step(1), time_trigger("ta", RelOp::EqualTo, 50.0)),
            policy("b", 0, step(-1), time_trigger("tb", RelOp::EqualTo, 80.0)),
        ],
        vec![ei_group("g", vec![])],
    );
    let mut r = Run::new(m, Quirks::slingshot());
    r.timer(0);
    let line = r.log.records[3].to_json();
    assert_eq!(
        line,
        r#"{"k":"eval","t":50.0,"ev":"SimulationTimeReached","policy":"b","filters":[["TargetGroupChecker","ok"],["SimulationTimeChecker","disregard","Simulationtime of 50.000000 is not [EqualTo] expectedValue. [[DISREGARD]]"]],"request":false}"#
    );
}

/// Q15: with the compat flag a `>=` policy also fires on another policy's time event of its
/// group; fixed: only on its own; another group never.
#[test]
fn q15_shared_time_events() {
    let m = || {
        model(
            vec![
                policy("a", 0, step(1), time_trigger("ta", RelOp::EqualTo, 50.0)),
                policy(
                    "b",
                    0,
                    step(1),
                    time_trigger("tb", RelOp::GreaterThanOrEqualTo, 40.0),
                ),
                policy(
                    "c",
                    1,
                    step(1),
                    time_trigger("tc", RelOp::GreaterThanOrEqualTo, 10.0),
                ),
            ],
            vec![ei_group("g", vec![]), ei_group("other", vec![])],
        )
    };
    let mut r = Run::new(m(), Quirks::slingshot());
    r.run_timers();
    assert_eq!(r.fired(0), vec![50.0]);
    assert_eq!(r.fired(1), vec![40.0, 50.0]);
    assert_eq!(r.fired(2), vec![10.0]);
    let mut r = Run::new(m(), Quirks::fixed());
    r.run_timers();
    assert_eq!(r.fired(0), vec![50.0]);
    assert_eq!(r.fired(1), vec![40.0]);
    assert_eq!(r.fired(2), vec![10.0]);
}

// ------------------------------------------------------------------------------- constraints

/// A policy firing on every time event of its group (`>= 1`, its own event at 1) plus helper
/// policies that schedule events at `times` and never request (an interval constraint
/// disregards them always); Q15 compat delivers the helpers' events to the other policies.
fn with_helpers(main: Policy, times: &[f64], groups: Vec<TargetGroup>) -> SpdModel {
    let mut ps = vec![main];
    for (i, &t) in times.iter().enumerate() {
        let mut h = policy(
            &format!("h{i}"),
            0,
            step(1),
            time_trigger("h", RelOp::EqualTo, t),
        );
        h.constraints.push(PolicyConstraint::Interval {
            offset: 0.0,
            duration: 1e9,
            repeat: false,
        });
        ps.push(h);
    }
    model(ps, groups)
}

#[test]
fn cooldown_edges() {
    let mut p = policy(
        "p",
        0,
        step(1),
        time_trigger("t", RelOp::GreaterThanOrEqualTo, 1.0),
    );
    p.constraints.push(PolicyConstraint::Cooldown {
        time: 10.0,
        max_operations: 1,
    });
    let m = with_helpers(
        p,
        &[5.0, 10.0, 11.0, 12.0, 21.0, 25.0],
        vec![ei_group("g", vec![])],
    );
    let mut r = Run::new(m, Quirks::slingshot());
    r.run_timers();
    // 1: t > end (0): count 0, end 11. 5: in cooldown, 0 < 1 passes, count 1.
    // 10: 1 >= 1 disregards. 11: t >= end passes; t == end only moves the end (21), count stays
    // 1. 12: disregards. 21: passes (end 31). 25: disregards.
    assert_eq!(r.fired(0), vec![1.0, 5.0, 11.0, 21.0]);
    assert_eq!(r.st.policies[0].adjustor.cooldown_end, 31.0);
    assert_eq!(r.st.policies[0].adjustor.scales_in_cooldown, 1);
    assert_eq!(r.st.policies[0].adjustor.scales, 4);
    let e = r.evals("p");
    let at10 = e.iter().find(|x| x.0 == 10.0).unwrap();
    assert_eq!(
        at10.1[2],
        ("CooldownConstraintFilter".into(), "disregard".into())
    );
}

#[test]
fn cooldown_time_zero_changes_nothing() {
    let mut p = policy(
        "p",
        0,
        step(1),
        time_trigger("t", RelOp::GreaterThanOrEqualTo, 1.0),
    );
    p.constraints.push(PolicyConstraint::Cooldown {
        time: 0.0,
        max_operations: 0,
    });
    let m = with_helpers(p, &[2.0, 3.0], vec![ei_group("g", vec![])]);
    let mut r = Run::new(m, Quirks::slingshot());
    r.run_timers();
    assert_eq!(r.fired(0), vec![1.0, 2.0, 3.0]);
}

#[test]
fn interval_constraint() {
    // repeat: disregard if t != 0 && (t % 15 == 0 || t % 15 >= 10)
    let mut p = policy(
        "p",
        0,
        step(1),
        time_trigger("t", RelOp::GreaterThanOrEqualTo, 0.0),
    );
    p.constraints.push(PolicyConstraint::Interval {
        offset: 10.0,
        duration: 5.0,
        repeat: true,
    });
    let times = [9.5, 10.0, 14.9, 15.0, 16.0, 30.0, 31.0];
    let m = with_helpers(p.clone(), &times, vec![ei_group("g", vec![])]);
    let mut r = Run::new(m, Quirks::slingshot());
    r.run_timers();
    assert_eq!(r.fired(0), vec![0.0, 9.5, 16.0, 31.0]);
    // not repeated: disregard if 10 <= t <= 15
    p.constraints[0] = PolicyConstraint::Interval {
        offset: 10.0,
        duration: 5.0,
        repeat: false,
    };
    let m = with_helpers(
        p,
        &[9.99, 10.0, 15.0, 15.01, 30.0],
        vec![ei_group("g", vec![])],
    );
    let mut r = Run::new(m, Quirks::slingshot());
    r.run_timers();
    assert_eq!(r.fired(0), vec![0.0, 9.99, 15.01, 30.0]);
    let e = r.evals("p");
    let at15 = e.iter().find(|x| x.0 == 15.0).unwrap();
    assert_eq!(
        at15.1[2],
        ("IntervalConstraintFilter".into(), "disregard".into())
    );
}

#[test]
fn thrashing_constraint() {
    let g = vec![ei_group(
        "g",
        vec![TargetConstraint::Thrashing { minimum_time: 10.0 }],
    )];
    let m = model(
        vec![
            policy("up", 0, step(1), time_trigger("a", RelOp::EqualTo, 10.0)),
            // opposite sign within 10 s of the last enactment (up2 at 15): 15 + 10 >= 20
            policy("down", 0, step(-1), time_trigger("b", RelOp::EqualTo, 20.0)),
            // same sign: passes
            policy("up2", 0, step(2), time_trigger("c", RelOp::EqualTo, 15.0)),
            // opposite sign after the minimum time (15 + 10 >= 25.5 is false): passes
            policy(
                "down2",
                0,
                step(-1),
                time_trigger("d", RelOp::EqualTo, 25.5),
            ),
            // relative with 0 % counts as negative: within 10 s of down2 is the same sign
            policy(
                "rel0",
                0,
                Adjustment::Relative {
                    percentage: 0,
                    min: 1,
                },
                time_trigger("e", RelOp::EqualTo, 26.0),
            ),
            // absolute always passes
            policy(
                "abs",
                0,
                Adjustment::Absolute { goal: 3 },
                time_trigger("f", RelOp::EqualTo, 27.0),
            ),
            // the last enacted policy is absolute: passes
            policy("up3", 0, step(1), time_trigger("h", RelOp::EqualTo, 28.0)),
        ],
        g,
    );
    let mut r = Run::new(m, Quirks::slingshot());
    r.run_timers();
    let fired: Vec<usize> = r.host.requests.iter().map(|q| q.policy).collect();
    assert_eq!(fired, vec![0, 2, 3, 4, 5, 6]);
    let e = r.evals("down");
    let own = e.iter().find(|x| x.0 == 20.0).unwrap();
    assert_eq!(
        own.1[2],
        ("ThrashingConstraintFilter".into(), "disregard".into())
    );
}

/// Q2: fixed, the constraint state is committed only by a successful adjustment.
#[test]
fn q2_commit_on_success() {
    let mk = || {
        let mut p = policy(
            "p",
            0,
            step(1),
            time_trigger("t", RelOp::GreaterThanOrEqualTo, 1.0),
        );
        p.constraints.push(PolicyConstraint::Cooldown {
            time: 10.0,
            max_operations: 0,
        });
        with_helpers(p, &[5.0, 6.0, 7.0], vec![ei_group("g", vec![])])
    };
    // Slingshot: the request at 1 starts the cooldown at once: 5, 6, 7 are disregarded
    let mut r = Run::new(mk(), Quirks::slingshot());
    r.run_timers();
    assert_eq!(r.fired(0), vec![1.0]);
    // fixed: nothing committed until the adjustment succeeds
    let mut r = Run::new(mk(), Quirks::fixed());
    let q = Quirks {
        q15_shared_time_events: true,
        ..Quirks::fixed()
    };
    let mut r2 = Run::new(mk(), q);
    r.run_timers();
    assert_eq!(
        r.fired(0),
        vec![1.0],
        "fixed Q15: the helpers' events do not reach p"
    );
    r2.timer(0);
    r2.timer(1); // 5: still no committed state: fires again
    assert_eq!(r2.fired(0), vec![1.0, 5.0]);
    let req = r2.host.requests[0].clone();
    r2.spd
        .adjusted(&mut r2.st, &req, true, 5.0, (None, None), &mut r2.log);
    assert_eq!(r2.st.policies[0].adjustor.cooldown_end, 11.0);
    r2.timer(2); // 6: in the cooldown now
    assert_eq!(r2.fired(0), vec![1.0, 5.0]);
    // a failed adjustment commits nothing
    let req = r2.host.requests[1].clone();
    r2.spd
        .adjusted(&mut r2.st, &req, false, 6.0, (None, None), &mut r2.log);
    assert_eq!(r2.st.policies[0].adjustor.scales, 1);
    assert!(matches!(
        r2.log.records.last(),
        Some(Record::Adjust { ok: false, .. })
    ));
}

/// Q9: fixed (provisional), group-size constraints are checked in the chain.
#[test]
fn q9_group_size_in_chain() {
    let g = || {
        vec![ei_group(
            "g",
            vec![TargetConstraint::Size { min: 1, max: 2 }],
        )]
    };
    let m = || {
        model(
            vec![policy(
                "p",
                0,
                step(1),
                time_trigger("t", RelOp::EqualTo, 5.0),
            )],
            g(),
        )
    };
    // two elements, max 2: +1 is clamped to 0
    let mut r = Run::new(m(), Quirks::slingshot());
    assert_eq!(
        r.spd.contexts()[0].names,
        ["TargetGroupChecker", "SimulationTimeChecker", "Adjustor"]
    );
    r.run_timers();
    assert_eq!(r.fired(0), vec![5.0]);
    let mut r = Run::new(m(), Quirks::fixed());
    assert_eq!(
        r.spd.contexts()[0].names,
        [
            "TargetGroupChecker",
            "SimulationTimeChecker",
            "TargetGroupSizeConstraintFilter",
            "Adjustor"
        ]
    );
    r.run_timers();
    assert!(r.fired(0).is_empty());
    assert_eq!(r.st.policies[0].adjustor.scales, 0);
    // one element: passes
    let mut r = Run::new(m(), Quirks::fixed());
    r.st.membership = membership(vec![UNIT]);
    r.run_timers();
    assert_eq!(r.fired(0), vec![5.0]);
    // scale-in of a single element is clamped to 0 by min 1
    let m = model(
        vec![policy(
            "p",
            0,
            step(-1),
            time_trigger("t", RelOp::EqualTo, 5.0),
        )],
        g(),
    );
    let mut r = Run::new(m, Quirks::fixed());
    r.st.membership = membership(vec![UNIT]);
    r.run_timers();
    assert!(r.fired(0).is_empty());
}

/// Q11: unsupported stimuli and triggers, wrong expectation types.
#[test]
fn q11_setup_errors() {
    let g = vec![ei_group("g", vec![])];
    let bad = [
        value_trigger(
            "n",
            RelOp::EqualTo,
            Stimulus::Unsupported("NumberOfElements"),
            Expected::Count(2),
        ),
        Trigger {
            obj: ObjId(0),
            id: "trend".into(),
            kind: TriggerKind::Trend,
        },
        value_trigger(
            "w",
            RelOp::EqualTo,
            Stimulus::SimulationTime,
            Expected::Percentage(5.0),
        ),
        value_trigger(
            "c",
            RelOp::GreaterThan,
            Stimulus::CpuUtilization {
                aggregation: Aggregation::Max,
            },
            Expected::Percentage(150.0),
        ),
    ];
    for t in bad {
        let m = model(
            vec![
                policy("ok", 0, step(1), time_trigger("t", RelOp::EqualTo, 1.0)),
                policy("bad", 0, step(1), t.clone()),
            ],
            g.clone(),
        );
        let e = Spd::new(m.clone(), Quirks::fixed()).unwrap_err();
        assert!(e.0.contains("policy bad"), "{e}");
        // Slingshot: the set-up handler throws, no policy is set up at all
        let s = Spd::new(m, Quirks::slingshot()).unwrap();
        assert!(s.dropped().is_some());
        assert!(s.contexts().is_empty() && s.timers().is_empty());
    }
    // an inactive policy is not interpreted
    let mut p = policy(
        "bad",
        0,
        step(1),
        Trigger {
            obj: ObjId(0),
            id: "trend".into(),
            kind: TriggerKind::Trend,
        },
    );
    p.active = false;
    let s = Spd::new(model(vec![p], g), Quirks::fixed()).unwrap();
    assert!(s.contexts().is_empty());
}

// ------------------------------------------------------------------------- composed triggers

/// Q12: Slingshot's XOR inverts after the first child and parks; fixed: exactly one child.
#[test]
fn q12_xor() {
    let mk = || {
        let main = policy(
            "x",
            0,
            step(1),
            composed(
                LogicalOp::Xor,
                vec![
                    time_trigger("a", RelOp::EqualTo, 50.0),
                    time_trigger("b", RelOp::EqualTo, 70.0),
                ],
            ),
        );
        with_helpers(main, &[90.0, 100.0], vec![ei_group("g", vec![])])
    };
    let mut r = Run::new(mk(), Quirks::slingshot());
    r.run_timers();
    // 50: child a passes, the XOR's own next() counts it as a second pass: disregard.
    // 70: the iterator resumes at b: passes, disregard again. 90: no child left: null, the
    // chain parks. 100: resumes at the adjustor.
    assert_eq!(r.fired(0), vec![100.0]);
    let e = r.evals("x");
    assert_eq!(
        e[2].1,
        vec![
            ("TargetGroupChecker".to_string(), "ok".to_string()),
            ("LogicalXORCompoundFilter".to_string(), "null".to_string())
        ]
    );
    assert_eq!(e[3].1, vec![("Adjustor".to_string(), "ok".to_string())]);
    let q = Quirks {
        q15_shared_time_events: true,
        ..Quirks::fixed()
    };
    let mut r = Run::new(mk(), q);
    r.run_timers();
    assert_eq!(r.fired(0), vec![50.0, 70.0]);
}

/// Q12: AND resumes after a failed child; OR parks; fixed: proper AND / OR; nested triggers.
#[test]
fn q12_and_or_nested() {
    let and = || {
        composed(
            LogicalOp::And,
            vec![
                time_trigger("a", RelOp::GreaterThanOrEqualTo, 10.0),
                time_trigger("b", RelOp::GreaterThanOrEqualTo, 30.0),
                time_trigger("c", RelOp::LessThanOrEqualTo, 60.0),
            ],
        )
    };
    let q15 = Quirks {
        q15_shared_time_events: true,
        ..Quirks::fixed()
    };
    // AND of three (one flat AND filter in Slingshot); events at 10, 30, 60 (own) and 40, 70
    let m = || {
        with_helpers(
            policy("p", 0, step(1), and()),
            &[40.0, 70.0],
            vec![ei_group("g", vec![])],
        )
    };
    let mut r = Run::new(m(), q15);
    r.run_timers();
    // fixed: 10 (b fails), 30 ok, 40 ok, 60 ok, 70 (c fails)
    assert_eq!(r.fired(0), vec![30.0, 40.0, 60.0]);
    let mut r = Run::new(m(), Quirks::slingshot());
    assert_eq!(
        r.spd.contexts()[0].nodes.len(),
        4,
        "one AND node with three leaves"
    );
    r.run_timers();
    // Slingshot: 10: a ok, b fails (iterator after b). 30: c ok, end: ok. 40: restart, all ok.
    // 60: all ok. 70: a, b ok, c fails (iterator at the end) -> disregard
    assert_eq!(r.fired(0), vec![30.0, 40.0, 60.0]);
    // OR nested in AND: AND(OR(t == 10, t == 20), t >= 15)
    let nested = composed(
        LogicalOp::And,
        vec![
            composed(
                LogicalOp::Or,
                vec![
                    time_trigger("o1", RelOp::EqualTo, 10.0),
                    time_trigger("o2", RelOp::EqualTo, 20.0),
                ],
            ),
            time_trigger("g", RelOp::GreaterThanOrEqualTo, 15.0),
        ],
    );
    let m = || {
        model(
            vec![policy("p", 0, step(1), nested.clone())],
            vec![ei_group("g", vec![])],
        )
    };
    let mut r = Run::new(m(), Quirks::fixed());
    r.run_timers();
    assert_eq!(r.fired(0), vec![20.0]);
    let mut r = Run::new(m(), Quirks::slingshot());
    r.run_timers();
    // Slingshot: the OR returns null inside the AND, which then reports null too: the outer
    // chain parks at 10 and resumes at the adjustor at 15; at 20 the AND resumes after the OR
    // at g, which passes, and the AND succeeds
    assert_eq!(r.fired(0), vec![15.0, 20.0]);
}

/// Q12: two measurement stimulus kinds run the chain twice per measurement in Slingshot.
#[test]
fn q12_two_subscribers() {
    let t = composed(
        LogicalOp::Or,
        vec![
            value_trigger(
                "c",
                RelOp::GreaterThan,
                Stimulus::CpuUtilization {
                    aggregation: Aggregation::Max,
                },
                Expected::Percentage(50.0),
            ),
            value_trigger(
                "n",
                RelOp::GreaterThan,
                Stimulus::TaskCount {
                    aggregation: Aggregation::Max,
                },
                Expected::Count(3),
            ),
        ],
    );
    let m = || {
        model(
            vec![policy("p", 0, step(1), t.clone())],
            vec![ei_group("g", vec![])],
        )
    };
    let mut r = Run::new(m(), Quirks::slingshot());
    r.measure(1.0, cpu(UNIT, Metric::Utilization, 1.0, 0.1, 0));
    assert_eq!(r.st.evaluations, 2);
    let mut r = Run::new(m(), Quirks::fixed());
    r.measure(1.0, cpu(UNIT, Metric::Utilization, 1.0, 0.1, 0));
    assert_eq!(r.st.evaluations, 1);
}

// ------------------------------------------------------------------- measurements, windows

fn cpu_policy(agg: Aggregation, pct: f64) -> SpdModel {
    model(
        vec![policy(
            "cpu",
            0,
            step(1),
            value_trigger(
                "c",
                RelOp::GreaterThan,
                Stimulus::CpuUtilization { aggregation: agg },
                Expected::Percentage(pct),
            ),
        )],
        vec![ei_group("g", vec![])],
    )
}

#[test]
fn target_group_check_of_measurements() {
    let mut r = Run::new(cpu_policy(Aggregation::Max, 50.0), Quirks::slingshot());
    // outside the group: rejected by the target-group check, not logged by default
    r.measure(1.0, cpu(OUTSIDE, Metric::Utilization, 1.0, 1.0, 9));
    assert!(r.evals("cpu").is_empty());
    assert_eq!(r.st.evaluations, 1);
    r.log.all = true;
    r.measure(2.0, cpu(OUTSIDE, Metric::Utilization, 2.0, 1.0, 9));
    assert_eq!(
        r.evals("cpu")[0].1[0],
        ("TargetGroupChecker".into(), "disregard".into())
    );
    // the window saw neither
    let simoxide_spd::NodeState::Leaf {
        window: Some(w), ..
    } = &r.st.policies[0].nodes[0]
    else {
        panic!()
    };
    assert!(!w.emittable());
    // an elastic infrastructure whose unit has no configuration: Slingshot throws
    r.st.membership.cfgs.clear();
    r.measure(3.0, cpu(UNIT, Metric::Utilization, 3.0, 1.0, 0));
    let e = r.evals("cpu");
    assert_eq!(
        e.last().unwrap().1[0],
        ("TargetGroupChecker".into(), "exception".into())
    );
}

/// The 10-value window (MAX), merged over the group's elements (Q13 compat), and one window
/// per element (fixed).
#[test]
fn q13_last_ten_merged_and_per_element() {
    let mut r = Run::new(cpu_policy(Aggregation::Max, 50.0), Quirks::slingshot());
    for i in 0..9 {
        r.measure(i as f64, cpu(UNIT, Metric::Utilization, i as f64, 0.1, 0));
    }
    assert!(r.fired(0).is_empty());
    let e = r.evals("cpu");
    assert_eq!(
        e[0].1[1],
        ("CPUUtilizationTriggerChecker".into(), "disregard".into())
    );
    // the 10th sample comes from the other container and is above 50 %: MAX of the merged
    // window fires
    r.measure(9.0, cpu(C2, Metric::Utilization, 9.0, 0.9, 1));
    assert_eq!(r.fired(0), vec![9.0]);
    // getResult also for a task-count tuple (not aggregated): fires again on the same window
    r.measure(10.0, cpu(UNIT, Metric::ActiveResourceState, 10.0, 0.0, 2));
    assert_eq!(r.fired(0), vec![9.0, 10.0]);

    let mut r = Run::new(cpu_policy(Aggregation::Max, 50.0), Quirks::fixed());
    for i in 0..9 {
        r.measure(i as f64, cpu(UNIT, Metric::Utilization, i as f64, 0.1, 0));
    }
    r.measure(9.0, cpu(C2, Metric::Utilization, 9.0, 0.9, 1));
    assert!(r.fired(0).is_empty(), "C2's window is not full");
    r.measure(10.0, cpu(UNIT, Metric::Utilization, 10.0, 0.1, 0));
    assert!(
        r.fired(0).is_empty(),
        "UNIT's window is full, its max is 0.1"
    );
    r.measure(11.0, cpu(UNIT, Metric::ActiveResourceState, 11.0, 0.0, 2));
    assert!(r.fired(0).is_empty(), "other metrics are not evaluated");
    for i in 0..9 {
        r.measure(
            12.0 + i as f64,
            cpu(C2, Metric::Utilization, 12.0 + i as f64, 0.9, 1),
        );
    }
    assert_eq!(r.fired(0), vec![20.0]);
}

/// AVERAGE: the EMA. Slingshot consumes the emit slot when the value is read, even if a later
/// constraint disregards (Q13); fixed: only when the policy fires.
#[test]
fn q13_ema_emit_slot() {
    let mk = || {
        let mut m = cpu_policy(Aggregation::Average, 10.0);
        m.policies[0].constraints.push(PolicyConstraint::Interval {
            offset: 0.0,
            duration: 65.0,
            repeat: false,
        });
        m
    };
    let mut r = Run::new(mk(), Quirks::slingshot());
    // not before t > 60
    r.measure(30.0, cpu(UNIT, Metric::Utilization, 30.0, 1.0, 0));
    r.measure(60.0, cpu(UNIT, Metric::Utilization, 60.0, 1.0, 0));
    // 61: emittable, the value is read (0.2.. > 0.1), the interval disregards (t <= 65)
    r.measure(61.0, cpu(UNIT, Metric::Utilization, 61.0, 1.0, 0));
    // 66: outside the interval, but 66 - 61 <= 10: not emittable
    r.measure(66.0, cpu(UNIT, Metric::Utilization, 66.0, 1.0, 0));
    assert!(r.fired(0).is_empty());
    r.measure(71.5, cpu(UNIT, Metric::Utilization, 71.5, 1.0, 0));
    assert_eq!(r.fired(0), vec![71.5]);

    let mut r = Run::new(mk(), Quirks::fixed());
    for t in [30.0, 60.0, 61.0] {
        r.measure(t, cpu(UNIT, Metric::Utilization, t, 1.0, 0));
    }
    // the slot was not consumed at 61: fires at 66
    r.measure(66.0, cpu(UNIT, Metric::Utilization, 66.0, 1.0, 0));
    assert_eq!(r.fired(0), vec![66.0]);
    r.measure(70.0, cpu(UNIT, Metric::Utilization, 70.0, 1.0, 0));
    assert_eq!(r.fired(0), vec![66.0], "consumed by the firing at 66");
}

#[test]
fn comparator_and_expectations() {
    // task count as Long compared as double; ExpectedPercentage / 100
    let m = model(
        vec![
            policy(
                "n",
                0,
                step(1),
                value_trigger(
                    "n",
                    RelOp::GreaterThanOrEqualTo,
                    Stimulus::TaskCount {
                        aggregation: Aggregation::Sum,
                    },
                    Expected::Count(20),
                ),
            ),
            policy(
                "c",
                0,
                step(1),
                value_trigger(
                    "c",
                    RelOp::EqualTo,
                    Stimulus::CpuUtilization {
                        aggregation: Aggregation::Min,
                    },
                    Expected::Percentage(50.0),
                ),
            ),
        ],
        vec![ei_group("g", vec![])],
    );
    let mut r = Run::new(m, Quirks::slingshot());
    for i in 0..10 {
        r.measure(
            i as f64,
            cpu(UNIT, Metric::ActiveResourceState, i as f64, 2.0, 0),
        );
        r.measure(i as f64, cpu(UNIT, Metric::Utilization, i as f64, 0.5, 1));
    }
    // SUM of ten 2s = 20 >= 20; MIN 0.5 == 50 / 100
    assert!(!r.fired(0).is_empty());
    assert!(!r.fired(1).is_empty());
}

#[test]
fn response_time_trigger() {
    let sig = ObjId(30);
    let m = model(
        vec![policy(
            "rt",
            0,
            step(1),
            value_trigger(
                "rt",
                RelOp::GreaterThan,
                Stimulus::OperationResponseTime {
                    signature: Some(sig),
                    signature_name: "op".into(),
                },
                Expected::Time(2.0),
            ),
        )],
        vec![ei_group("g", vec![])],
    );
    let mut r = Run::new(m, Quirks::fixed());
    let rt = |v: f64, s: ObjId| Measurement {
        point: PointRef {
            kind: PointKind::Operation { signature: s },
            class: "AssemblyOperationMeasuringPoint",
        },
        metric: Metric::ResponseTime,
        point_in_time: 1.0,
        value: v,
        source: 0,
    };
    r.measure(1.0, rt(1.0, sig));
    r.measure(2.0, rt(3.0, sig));
    assert_eq!(r.fired(0), vec![2.0]);
    // another signature (not provided in the group): rejected by the target-group check
    r.measure(3.0, rt(3.0, ObjId(31)));
    assert_eq!(r.fired(0), vec![2.0]);
    let e = r.evals("rt");
    assert_eq!(e.len(), 2);
}

// ---------------------------------------------------------------------- state is plain data

#[test]
fn fork_continues_like_the_original() {
    let mk = || {
        let mut p = policy(
            "p",
            0,
            step(1),
            time_trigger("t", RelOp::GreaterThanOrEqualTo, 1.0),
        );
        p.constraints.push(PolicyConstraint::Cooldown {
            time: 3.0,
            max_operations: 1,
        });
        let mut m = with_helpers(p, &[2.0, 3.0, 4.0, 5.0, 8.0], vec![ei_group("g", vec![])]);
        m.policies.push(policy(
            "x",
            0,
            step(1),
            composed(
                LogicalOp::Or,
                vec![
                    time_trigger("a", RelOp::EqualTo, 2.0),
                    time_trigger("b", RelOp::EqualTo, 4.0),
                ],
            ),
        ));
        m
    };
    for q in [Quirks::fixed(), Quirks::slingshot()] {
        let mut whole = Run::new(mk(), q);
        whole.run_timers();
        let mut first = Run::new(mk(), q);
        let mut ts = first.host.scheduled.clone();
        ts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let (a, b) = ts.split_at(3);
        for &(t, id) in a {
            first.host.now = t;
            first
                .spd
                .on_timer(&mut first.st, &mut first.host, id, &mut first.log);
        }
        let snapshot = first.st.clone();
        let mut fork = Run {
            spd: first.spd.clone(),
            st: snapshot,
            host: Host::default(),
            log: VecLog::default(),
        };
        for &(t, id) in b {
            first.host.now = t;
            first
                .spd
                .on_timer(&mut first.st, &mut first.host, id, &mut first.log);
            fork.host.now = t;
            fork.spd
                .on_timer(&mut fork.st, &mut fork.host, id, &mut fork.log);
        }
        assert_eq!(first.st, whole.st);
        assert_eq!(fork.st, whole.st);
        assert_eq!(first.log.records, whole.log.records);
        let tail = &whole.log.records[whole.log.records.len() - fork.log.records.len()..];
        assert_eq!(fork.log.records, tail);
    }
}

#[test]
fn quirk_parsing() {
    assert_eq!(Quirks::parse("slingshot").unwrap(), Quirks::slingshot());
    assert_eq!(Quirks::parse("fixed").unwrap(), Quirks::fixed());
    let q = Quirks::parse("q12, Q15").unwrap();
    assert!(q.q12_broken_composed && q.q15_shared_time_events && !q.q2_commit_on_request);
    assert!(Quirks::parse("Q99").is_err());
}

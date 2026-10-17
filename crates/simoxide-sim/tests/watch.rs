//! Pause conditions (`DESIGN-SPD.md` N2, `simoxide_sim::Watch`): time and event conditions pause
//! where `Pause` pauses (the same snapshot bytes); series conditions (a tuple's value, a mean
//! over a window, a comparison held for a while, closeness to an SLO) pause right after the
//! event whose tuple made them hold, and not before (checked against a watch that also stops one
//! event earlier); a watch changes nothing in the run, in both modes and both flavours.

use simoxide_sim::sim::watch::{Op, SeriesSel, Stat, parse_condition};
use simoxide_sim::{
    Compat, CompiledModel, Condition, Exact, Fast, Flavour, FlavourConfig, Outputs, Pause, Quirks,
    RunSpec, SimConfig, SimuLizar, Simulation, Slingshot, Status, Watch,
};
use std::path::Path;

const S: i64 = 1_000_000_000;

fn load(dir: &str, flavour: FlavourConfig) -> (CompiledModel, SimConfig) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir);
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let cm = CompiledModel::compile_flavour(spec.load_model(&dir).unwrap(), flavour).unwrap();
    let cfg = spec.sim_config("watch");
    (cm, cfg)
}

fn bytes<C: Compat, F: Flavour>(sim: &mut Simulation<'_, C, F>) -> Vec<u8> {
    let mut b = Vec::new();
    sim.snapshot_to(&mut b).unwrap();
    b
}

/// Runs with `conds` to the pause; the snapshot bytes, the event count and the reason.
fn watched<C: Compat, F: Flavour>(
    cm: &CompiledModel,
    cfg: &SimConfig,
    conds: Vec<Condition>,
) -> Option<(Vec<u8>, u64, i64, usize, String)> {
    let cfg = &mode::<C>(cfg);
    let mut sim = Simulation::<C, F>::create(cm, cfg.clone(), Outputs::default()).unwrap();
    let mut w = Watch::new(conds);
    if sim.run_watch(&mut w).unwrap() == Status::Finished {
        assert!(w.fired().is_none());
        return None;
    }
    let f = w.fired().expect("paused by a condition").clone();
    assert_eq!(f.events, sim.state().events());
    Some((bytes(&mut sim), f.events, f.at_ns, f.condition, f.reason))
}

fn mode<C: Compat>(cfg: &SimConfig) -> SimConfig {
    SimConfig {
        mode: C::MODE,
        ..cfg.clone()
    }
}

fn paused<C: Compat, F: Flavour>(cm: &CompiledModel, cfg: &SimConfig, p: Pause) -> Vec<u8> {
    let cfg = &mode::<C>(cfg);
    let mut sim = Simulation::<C, F>::create(cm, cfg.clone(), Outputs::default()).unwrap();
    assert_eq!(sim.run_until(p).unwrap(), Status::Paused);
    bytes(&mut sim)
}

/// A series condition pauses at event k exactly as `Pause::Events(k)` does, and a watch that
/// also stops at the check before (event count p < k) stops there by that condition: the series
/// condition did not hold before.
fn series_condition<C: Compat, F: Flavour>(
    what: &str,
    cm: &CompiledModel,
    cfg: &SimConfig,
    c: Condition,
) -> String {
    let (b, k, _, i, reason) =
        watched::<C, F>(cm, cfg, vec![c.clone()]).unwrap_or_else(|| panic!("{what}: never held"));
    assert_eq!(i, 0);
    assert_eq!(b, paused::<C, F>(cm, cfg, Pause::Events(k)), "{what}");
    // the event count at the check before the pause
    let mut sim = Simulation::<C, F>::create(cm, mode::<C>(cfg), Outputs::default()).unwrap();
    let mut w = Watch::new(vec![c.clone()]);
    w.arm(&sim).unwrap();
    let mut prev = 0;
    sim.run_while(|s| {
        let go = w.keep_going(s);
        if go {
            prev = s.events();
        }
        go
    })
    .unwrap();
    assert_eq!(sim.state().events(), k, "{what}");
    assert!(prev < k);
    let (_, k2, _, i2, _) =
        watched::<C, F>(cm, cfg, vec![c, Condition::At(Pause::Events(prev))]).unwrap();
    assert_eq!((k2, i2), (prev, 1), "{what}: held before event {k}");
    reason
}

fn time_conditions<C: Compat, F: Flavour>(what: &str, cm: &CompiledModel, cfg: &SimConfig) {
    for p in [
        Pause::After(3 * S),
        Pause::Before(3 * S),
        Pause::After(3 * S + 1),
        Pause::Events(77),
    ] {
        let (b, ..) = watched::<C, F>(cm, cfg, vec![Condition::At(p)]).unwrap();
        assert_eq!(b, paused::<C, F>(cm, cfg, p), "{what} {p:?}");
    }
    // relative to the arming point: 2 s / 40 events after a pause at 3 s
    let mut sim = Simulation::<C, F>::create(cm, mode::<C>(cfg), Outputs::default()).unwrap();
    sim.run_until(Pause::After(3 * S)).unwrap();
    let (now, n) = (sim.state().now_ns(), sim.state().events());
    let mut a = sim.fork(Outputs::default()).unwrap();
    let mut w = Watch::new(vec![Condition::Elapsed(2 * S)]);
    a.run_watch(&mut w).unwrap();
    let mut b = sim.fork(Outputs::default()).unwrap();
    b.run_until(Pause::After(now + 2 * S)).unwrap();
    assert_eq!(bytes(&mut a), bytes(&mut b), "{what} elapsed");
    let mut w = Watch::new(vec![Condition::ElapsedEvents(40)]);
    let mut a = sim.fork(Outputs::default()).unwrap();
    a.run_watch(&mut w).unwrap();
    sim.run_until(Pause::Events(n + 40)).unwrap();
    assert_eq!(bytes(&mut a), bytes(&mut sim), "{what} elapsed events");
}

#[test]
fn time_and_event_conditions_pause_where_pause_does() {
    let (cm, cfg) = load("corpus/h06_closed_think", FlavourConfig::SIMULIZAR);
    time_conditions::<Exact, SimuLizar>("h06 exact", &cm, &cfg);
    time_conditions::<Fast, SimuLizar>("h06 fast", &cm, &cfg);
    let (cm, cfg) = load(
        "corpus/x_ss_minimal",
        FlavourConfig::slingshot(Quirks::NONE),
    );
    let cfg = SimConfig {
        max_sim_time: 60,
        ..cfg
    };
    time_conditions::<Fast, Slingshot>("x_ss_minimal slingshot", &cm, &cfg);
}

fn sel(metric: &str, point: &str) -> SeriesSel {
    SeriesSel {
        metric: metric.into(),
        point: point.into(),
    }
}

#[test]
fn series_conditions_pause_right_after_the_tuple() {
    let (cm, cfg) = load("corpus/h06_closed_think", FlavourConfig::SIMULIZAR);
    let cfg = SimConfig {
        max_measurements: 400,
        ..cfg
    };
    // a response time above 1.5 s (the scenario, by its name)
    let rt = Condition::Series {
        series: sel("rt", "Scenario"),
        stat: Stat::Value,
        op: Op::Gt,
        value: 1.5,
        sustain_s: None,
    };
    for reason in [
        series_condition::<Exact, SimuLizar>("rt exact", &cm, &cfg, rt.clone()),
        series_condition::<Fast, SimuLizar>("rt fast", &cm, &cfg, rt.clone()),
    ] {
        assert!(
            reason.starts_with(
                "Response Time Tuple of UsageScenarioMeasuringPoint[_h06_closed_think_us1]"
            ),
            "{reason}"
        );
    }
    // the mean response time of the last 5 s above 0.9 s
    let mean = parse_condition("rt[_h06_closed_think_ua1] mean 5 > 0.9").unwrap();
    series_condition::<Exact, SimuLizar>("mean", &cm, &cfg, mean);
    // the CPU's queue length at least 2 for 1 s (the container's resource, by its name)
    let held = parse_condition("state[ServerNode] >= 2 for 1").unwrap();
    let r = series_condition::<Exact, SimuLizar>("held", &cm, &cfg, held.clone());
    assert!(r.contains("for 1 s"), "{r}");
    series_condition::<Fast, SimuLizar>("held fast", &cm, &cfg, held);
    // closeness to an SLO: rt in [0, 2], sensitivity 0.5: fires at >= 1.5
    let slo = parse_condition("slo rt[_h06_closed_think_us1] upper 2 sensitivity 0.5").unwrap();
    let r = series_condition::<Exact, SimuLizar>("slo", &cm, &cfg, slo);
    assert!(r.contains(">= 1.5"), "{r}");
    // ... with an activation delay: not before 10 s
    let late =
        parse_condition("slo rt[_h06_closed_think_us1] upper 2 sensitivity 0.5 delay 10").unwrap();
    let (_, _, at, ..) = watched::<Exact, SimuLizar>(&cm, &cfg, vec![late]).unwrap();
    assert!(at >= 10 * S, "{at}");
    // the utilisation of a window series
    let (cm, cfg) = load(
        "crates/simoxide-sim/tests/models/t_util",
        FlavourConfig::SIMULIZAR,
    );
    let util = parse_condition("util[_h01_ps_single_prs1] > 0.5").unwrap();
    series_condition::<Exact, SimuLizar>("util", &cm, &cfg, util);
    // the Slingshot flavour
    let (cm, cfg) = load(
        "corpus/x_ss_minimal",
        FlavourConfig::slingshot(Quirks::NONE),
    );
    let cfg = SimConfig {
        max_sim_time: 200,
        ..cfg
    };
    let rt = parse_condition("rt[UsageScenarioMeasuringPoint] > 0.1").unwrap();
    series_condition::<Fast, Slingshot>("slingshot rt", &cm, &cfg, rt);
}

/// A condition that never holds runs to the end, unchanged; series conditions need stored
/// measurements.
#[test]
fn a_watch_changes_nothing() {
    let (cm, cfg) = load("corpus/h06_closed_think", FlavourConfig::SIMULIZAR);
    let never = parse_condition("rt[_h06_closed_think_us1] > 1000").unwrap();
    assert!(watched::<Exact, SimuLizar>(&cm, &cfg, vec![never.clone()]).is_none());
    let mut a = Simulation::<Exact>::create(&cm, cfg.clone(), Outputs::default()).unwrap();
    a.run_watch(&mut Watch::new(vec![never.clone()])).unwrap();
    let a = a.finish().unwrap();
    let b = Simulation::<Exact>::create(&cm, cfg.clone(), Outputs::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(a.measurements.to_csv(), b.measurements.to_csv());
    assert_eq!((a.events, a.uniforms), (b.events, b.uniforms));
    let cfg = SimConfig {
        store_measurements: false,
        ..cfg
    };
    let mut sim = Simulation::<Exact>::create(&cm, cfg, Outputs::default()).unwrap();
    let e = sim.run_watch(&mut Watch::new(vec![never])).unwrap_err();
    assert!(e.message.contains("store_measurements"), "{}", e.message);
}

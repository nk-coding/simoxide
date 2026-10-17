//! F4 (`DESIGN-SPD.md` §5.3 "F4 as built"): the change kinds F2 refused, the removal policies,
//! executions in progress inside changed behaviours, the Slingshot flavour's changes, bounded
//! growth over many changes, and snapshots after each change kind.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use qvto_model::{Datum, ModelStore};
use simoxide_model::meta::{class as mc, feat as mf};
use simoxide_model::raw::ObjId;
use simoxide_reconf::Edit;
use simoxide_sim::{
    ChangeOptions, ChangeReport, Compat, CompiledModel, Exact, Fast, Flavour, FlavourConfig,
    Outputs, Pause, Removal, RunResult, RunSpec, SimConfig, Simulation,
};

const S: i64 = 1_000_000_000;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load_flavour(name: &str, flavour: FlavourConfig) -> (CompiledModel, SimConfig) {
    let dir = root().join("corpus").join(name);
    let spec = RunSpec::load(&dir.join("run.json")).expect("run.json");
    let model = spec.load_model(&dir).expect("model");
    let cm = CompiledModel::compile_flavour(model, flavour).expect("compile");
    (cm, spec.sim_config("t"))
}

fn load(name: &str) -> (CompiledModel, SimConfig) {
    load_flavour(name, FlavourConfig::SIMULIZAR)
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
        String::from_utf8(std::mem::take(&mut *self.0.borrow_mut())).unwrap()
    }
}

fn traced(b: &Buf) -> Outputs {
    Outputs {
        trace: Some(Box::new(b.clone())),
        tape: None,
    }
}

fn summary(r: &RunResult) -> String {
    format!(
        "end {} main {} events {} tuples {}\n{}",
        r.end_ns,
        r.main_count,
        r.events,
        r.measurements.count,
        r.measurements.to_csv()
    )
}

#[allow(dead_code)]
/// `(t, container, demand)` of the trace's demand lines.
fn demands(trace: &str) -> Vec<(f64, String, f64)> {
    trace
        .lines()
        .filter(|l| l.starts_with("{\"ev\":\"demand\""))
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            (
                v["t"].as_f64().unwrap(),
                v["rc"].as_str().unwrap().to_string(),
                v["d"].as_f64().unwrap(),
            )
        })
        .collect()
}

/// The usage-scenario runs that begin: `(t, process)`.
fn scenario_starts(trace: &str) -> Vec<(f64, i64)> {
    trace
        .lines()
        .filter(|l| l.starts_with("{\"ev\":\"begin\"") && l.contains("\"type\":\"UsageScenario\""))
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            (v["t"].as_f64().unwrap(), v["p"].as_i64().unwrap())
        })
        .collect()
}

/// A copy of a corpus model with `edit` applied to the text of the files with extension `ext`.
fn variant(name: &str, ext: &str, edit: impl Fn(&str) -> String) -> (CompiledModel, SimConfig) {
    let src = root().join("corpus").join(name);
    let dst = std::env::temp_dir().join(format!(
        "simoxide-f4-{}-{name}-{ext}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dst).unwrap();
    for e in std::fs::read_dir(&src).unwrap().flatten() {
        let p = e.path();
        if p.is_file() {
            let mut text = std::fs::read(&p).unwrap();
            if p.extension().and_then(|x| x.to_str()) == Some(ext) {
                let t = String::from_utf8(text).unwrap();
                let t2 = edit(&t);
                assert_ne!(t, t2, "the edit changes the file");
                text = t2.into_bytes();
            }
            std::fs::write(dst.join(e.file_name()), text).unwrap();
        }
    }
    let spec = RunSpec::load(&dst.join("run.json")).expect("run.json");
    let model = spec.load_model(&dst).expect("model");
    let _ = std::fs::remove_dir_all(&dst);
    (CompiledModel::compile(model).unwrap(), spec.sim_config("t"))
}

const H19: &str = "h19_linking_resource";
const H19_SERVER: &str = "_h19_linking_resource_rc2";

fn first_of(e: &Edit, class: simoxide_model::meta::ClassId) -> ObjId {
    let g = e.graph();
    (0..g.object_count() as u32)
        .map(ObjId)
        .find(|&o| g[o].class == class)
        .expect("an object of the class")
}

// ---------------------------------------------------------------------------- the OOM case

/// The blow-up of the first F4 draft: h19 stops after a number of usage-scenario runs only;
/// under `Drain` every request still allocated to the removed server is aborted, and aborted
/// requests did not count, so the run never stopped. An aborted request counts as a finished
/// run (SimuCom counts failed runs too), so every policy stops.
#[test]
fn every_removal_policy_stops_a_run_that_counts_runs() {
    let (cm, cfg) = load(H19);
    let mut cfg = SimConfig {
        max_measurements: 300,
        ..cfg
    };
    // a run that does not stop fails here instead of growing without bound
    cfg.limits.max_events = 5_000_000;
    for removal in [Removal::Keep, Removal::Drain, Removal::Abort] {
        let mut sim = Simulation::<Exact>::create(&cm, cfg.clone(), Outputs::default()).unwrap();
        sim.set_change_options(ChangeOptions {
            removal,
            ..ChangeOptions::default()
        });
        sim.run_until(Pause::After(12 * S)).unwrap();
        let mut e = Edit::new(&sim);
        let server = e.find(H19_SERVER).unwrap();
        e.remove_from_environment(server).unwrap();
        e.apply(&mut sim).unwrap();
        let r = sim.finish().unwrap();
        assert!(r.main_count >= 300, "{removal:?}: {}", r.main_count);
    }
}

/// Many change sets on one run (a SEFF demand changed every second, alternating, plus a rate):
/// the instruction streams, series, resources and model stay bounded (old instructions no
/// execution uses are dropped at the next change).
#[test]
fn many_changes_keep_the_run_bounded() {
    let (cm, cfg) = load(H19);
    let cfg = SimConfig {
        max_measurements: 100_000,
        ..cfg
    };
    let base_sops = cm.code.plain.sops.len();
    let base_objects = cm.model.graph.object_count();
    let mut sim = Simulation::<Fast>::create(
        &cm,
        SimConfig {
            mode: simoxide_sim::Mode::Fast,
            ..cfg
        },
        Outputs::default(),
    )
    .unwrap();
    for i in 1..=300i64 {
        sim.run_until(Pause::After(i * S)).unwrap();
        let mut e = Edit::new(&sim);
        let act = e.find("_h19_linking_resource_act5").unwrap();
        let d = e.graph().get_refs(
            act,
            mf::AbstractInternalControlFlowAction_resourceDemand_Action,
        )[0];
        let spec = if i % 2 == 0 {
            "data.BYTESIZE * 0.0001"
        } else {
            "data.BYTESIZE * 0.00002"
        };
        e.set_spec(
            d,
            mf::ParametricResourceDemand_specification_ParametericResourceDemand,
            spec,
        )
        .unwrap();
        let p = e.graph().get_refs(
            e.find(H19_SERVER).unwrap(),
            mf::ResourceContainer_activeResourceSpecifications_ResourceContainer,
        )[0];
        e.set_rate(p, if i % 3 == 0 { "2000" } else { "1000" })
            .unwrap();
        let r = e.apply(&mut sim).unwrap();
        assert_eq!(r.generation, i as u32);
        let c = sim.compiled();
        assert!(
            c.code.plain.sops.len() <= 2 * base_sops,
            "change {i}: {} instructions (base {base_sops})",
            c.code.plain.sops.len()
        );
        assert_eq!(c.series.len(), cm.series.len());
        assert_eq!(c.resources.len(), cm.resources.len());
        assert!(
            c.progs.len() <= cm.progs.len() + 4,
            "change {i}: {} programs",
            c.progs.len()
        );
        assert!(sim.model_graph().object_count() <= base_objects + 4);
    }
    let r = sim.finish().unwrap();
    assert!(r.main_count > 0);
}

// ---------------------------------------------------------------------------- usage model

fn set_population(e: &mut Edit, n: i32) {
    let w = first_of(e, mc::ClosedWorkload);
    e.store()
        .set(w, mf::ClosedWorkload_population, Datum::Int(n))
        .unwrap();
}

/// A population change starts new users at once and stops the oldest ones after their
/// current run and think time (`ClosedWorkload.setPopulation`).
#[test]
fn population_changes_start_and_stop_users() {
    let (cm, cfg) = load("h06_closed_think");
    let cfg = SimConfig {
        max_measurements: 400,
        ..cfg
    };
    let b = Buf::default();
    let mut sim = Simulation::<Exact>::create(&cm, cfg, traced(&b)).unwrap();
    sim.run_until(Pause::After(20 * S)).unwrap();
    let mut e = Edit::new(&sim);
    set_population(&mut e, 8);
    let r = e.apply(&mut sim).unwrap();
    assert!(r.applied.iter().any(|m| m.contains("4 -> 8")), "{r:?}");
    sim.run_until(Pause::After(60 * S)).unwrap();
    let mut e = Edit::new(&sim);
    set_population(&mut e, 2);
    e.apply(&mut sim).unwrap();
    sim.finish().unwrap();
    let starts = scenario_starts(&b.take());
    let users = |from: f64, to: f64| {
        let mut p: Vec<i64> = starts
            .iter()
            .filter(|s| s.0 >= from && s.0 < to)
            .map(|s| s.1)
            .collect();
        p.sort_unstable();
        p.dedup();
        p.len()
    };
    // (the change applies at the last event before t = 20 s)
    assert_eq!(users(0.0, 15.0), 4);
    assert_eq!(users(30.0, 60.0), 8);
    // after the decrease and the stopped users' last runs
    let end = starts.last().unwrap().0;
    assert!(end > 90.0, "{end}");
    assert_eq!(users(end - 25.0, end + 1.0), 2);
}

/// A population change before the start equals the model with that population.
#[test]
fn population_change_before_the_start_equals_the_changed_model() {
    let (cm, cfg) = load("h06_closed_think");
    let mut sim = Simulation::<Exact>::create(&cm, cfg.clone(), Outputs::default()).unwrap();
    let mut e = Edit::new(&sim);
    set_population(&mut e, 7);
    e.apply(&mut sim).unwrap();
    let got = summary(&sim.finish().unwrap());
    let (cm7, _) = variant("h06_closed_think", "usagemodel", |t| {
        t.replace("population=\"4\"", "population=\"7\"")
    });
    let want = summary(
        &Simulation::<Exact>::create(&cm7, cfg, Outputs::default())
            .unwrap()
            .finish()
            .unwrap(),
    );
    assert_eq!(want, got);
}

// ---------------------------------------------------------------------------- removal policies

const LB: &str = "corpus-rules/lb_outsource";
const LB_NODE2: &str = "_3uVlIAEpEeS7FKokKTKFow";
const LB_TO_SERVER1: &str = "_ridTkAEoEeS7FKokKTKFow";
const LB_TO_SERVER2: &str = "_tjFukAEoEeS7FKokKTKFow";

fn load_dir_flavour(dir: &str, flavour: FlavourConfig) -> (CompiledModel, SimConfig) {
    let dir = root().join(dir);
    let spec = RunSpec::load(&dir.join("run.json")).expect("run.json");
    let model = spec.load_model(&dir).expect("model");
    let cm = CompiledModel::compile_flavour(model, flavour).expect("compile");
    let mut cfg = spec.sim_config("t");
    cfg.max_sim_time = 400;
    cfg.max_measurements = -1;
    (cm, cfg)
}

fn set_probability(e: &mut Edit, transition: &str, p: f64) {
    let t = e.find(transition).unwrap();
    e.set_double(t, mf::ProbabilisticBranchTransition_branchProbability, p)
        .unwrap();
}

/// What a scale-in left in the trace: demands on ServerNode2 after the removal at `t`, by
/// requests that started before it and after it; completions there after `t`; finished runs.
#[derive(Debug, Default, PartialEq)]
struct ScaleIn {
    demands_before: usize,
    demands_after: usize,
    done_after: usize,
    runs: i64,
}

/// The load balancer sends half the requests to ServerNode2 from t = 100 s; at t = 200 s
/// ServerNode2 is scaled in (its branch gets probability 0, the container leaves the resource
/// environment; its allocation context stays). Runs `policy` in flavour `F`.
fn scale_in<C: Compat, F: Flavour>(
    flavour: FlavourConfig,
    mode: simoxide_sim::Mode,
    removal: Removal,
) -> ScaleIn {
    scale_in_with::<C, F>(flavour, mode, removal, true)
}

fn scale_in_with<C: Compat, F: Flavour>(
    flavour: FlavourConfig,
    mode: simoxide_sim::Mode,
    removal: Removal,
    route_away: bool,
) -> ScaleIn {
    let (cm, cfg) = load_dir_flavour(LB, flavour);
    let b = Buf::default();
    let mut sim = Simulation::<C, F>::create(&cm, SimConfig { mode, ..cfg }, traced(&b)).unwrap();
    sim.set_change_options(ChangeOptions {
        removal,
        ..sim.change_options()
    });
    sim.run_until(Pause::After(100 * S)).unwrap();
    apply(&mut sim, |e| {
        set_probability(e, LB_TO_SERVER1, 0.5);
        set_probability(e, LB_TO_SERVER2, 0.5);
        // ServerNode2 saturated: it has a queue at the scale-in
        let node = e.find(LB_NODE2).unwrap();
        for p in e
            .graph()
            .get_refs(
                node,
                mf::ResourceContainer_activeResourceSpecifications_ResourceContainer,
            )
            .to_vec()
        {
            e.set_rate(p, "450").unwrap();
        }
    });
    sim.run_until(Pause::After(200 * S)).unwrap();
    // (the state's time is in ns, the trace's in double seconds)
    let t_change = sim.state().now_ns() as f64 / 1e9 + 1e-6;
    let r = apply(&mut sim, |e| {
        if route_away {
            set_probability(e, LB_TO_SERVER1, 1.0);
            set_probability(e, LB_TO_SERVER2, 0.0);
        }
        let node = e.find(LB_NODE2).unwrap();
        e.remove_from_environment(node).unwrap();
    });
    assert!(
        r.applied
            .iter()
            .chain(&r.ignored)
            .any(|m| m.contains("left the resource environment")),
        "{r:?}"
    );
    let res = sim.finish().unwrap();
    let trace = b.take();
    // the processes in a request at the change: those whose scenario began before it and had
    // not ended
    let mut open: std::collections::HashMap<i64, bool> = Default::default();
    let mut out = ScaleIn {
        runs: res.main_count,
        ..ScaleIn::default()
    };
    for l in trace.lines() {
        let v: serde_json::Value = match serde_json::from_str(l) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let t = v["t"].as_f64().unwrap_or(0.0);
        let p = v["p"].as_i64().unwrap_or(-1);
        match v["ev"].as_str() {
            Some("begin") if v["type"] == "UsageScenario" => {
                open.insert(p, t <= t_change);
            }
            Some("demand") if v["rc"] == "ServerNode2" || v["rc"] == LB_NODE2 => {
                if t > t_change {
                    if open.get(&p).copied().unwrap_or(false) {
                        out.demands_before += 1;
                    } else {
                        out.demands_after += 1;
                    }
                }
            }
            Some("demand_done")
                if (v["rc"] == "ServerNode2" || v["rc"] == LB_NODE2) && t > t_change =>
            {
                out.done_after += 1;
            }
            _ => {}
        }
    }
    out
}

fn apply<C: Compat, F: Flavour>(
    sim: &mut Simulation<'_, C, F>,
    f: impl FnOnce(&mut Edit),
) -> ChangeReport {
    let mut e = Edit::new(sim);
    f(&mut e);
    e.apply(sim).unwrap()
}

fn ss() -> FlavourConfig {
    FlavourConfig::SLINGSHOT
}

fn ss_q5() -> FlavourConfig {
    FlavourConfig::slingshot(simoxide_sim::Quirks::NONE.with(simoxide_sim::Quirk::Q5))
}

/// Scale-in of ServerNode2 while requests still reach it (its branch keeps probability 0.5):
/// `Keep` serves them (SimuLizar), `Drain` finishes the jobs it has and aborts the new
/// requests, `Abort` aborts its jobs at once, Slingshot's Q5 aborts the pending job and loses
/// the others. Every run ends (aborted requests count as finished runs).
#[test]
fn scale_in_under_every_policy_in_both_flavours() {
    use simoxide_sim::{Mode, SimuLizar, Slingshot};
    let sim = |f, r| scale_in_keep_branch::<Exact, SimuLizar>(f, Mode::Exact, r);
    let fast = |f, r| scale_in_keep_branch::<Fast, Slingshot>(f, Mode::Fast, r);
    for (name, keep, drain, abort) in [
        (
            "simulizar",
            sim(FlavourConfig::SIMULIZAR, Removal::Keep),
            sim(FlavourConfig::SIMULIZAR, Removal::Drain),
            sim(FlavourConfig::SIMULIZAR, Removal::Abort),
        ),
        (
            "slingshot",
            fast(ss(), Removal::Keep),
            fast(ss(), Removal::Drain),
            fast(ss(), Removal::Abort),
        ),
    ] {
        // keep simulating: new requests are served there
        assert!(keep.demands_after > 0, "{name} keep {keep:?}");
        assert!(keep.done_after > 0, "{name} keep {keep:?}");
        // drain: the jobs there complete, requests that start later are not served there
        assert_eq!(drain.demands_after, 0, "{name} drain {drain:?}");
        assert!(drain.done_after > 0, "{name} drain {drain:?}");
        // abort: nothing completes there after the change
        assert_eq!(abort.demands_after, 0, "{name} abort {abort:?}");
        assert_eq!(abort.done_after, 0, "{name} abort {abort:?}");
        assert_eq!(abort.demands_before, 0, "{name} abort {abort:?}");
        assert!(keep.runs > 0 && drain.runs > 0 && abort.runs > 0);
    }
    // Slingshot's Q5: the default removal under the flag
    let q5 = fast(ss_q5(), Removal::SlingshotQ5);
    assert_eq!(q5.demands_after, 0, "{q5:?}");
    assert_eq!(q5.done_after, 0, "{q5:?}");
    let (cm, _) = load_dir_flavour(LB, ss_q5());
    let s = Simulation::<Fast, Slingshot>::create(
        &cm,
        SimConfig {
            mode: Mode::Fast,
            ..SimConfig::default()
        },
        Outputs::default(),
    )
    .unwrap();
    assert_eq!(s.change_options().removal, Removal::SlingshotQ5);
}

/// [`scale_in`] without routing away: ServerNode2 leaves the environment, its branch stays.
fn scale_in_keep_branch<C: Compat, F: Flavour>(
    flavour: FlavourConfig,
    mode: simoxide_sim::Mode,
    removal: Removal,
) -> ScaleIn {
    scale_in_with::<C, F>(flavour, mode, removal, false)
}

/// The routed scale-in (the branch to ServerNode2 gets probability 0): no request reaches it
/// after the change in any policy; under drain its jobs complete.
#[test]
fn routed_scale_in_drains_in_both_flavours() {
    use simoxide_sim::{Mode, SimuLizar, Slingshot};
    for r in [
        scale_in::<Exact, SimuLizar>(FlavourConfig::SIMULIZAR, Mode::Exact, Removal::Drain),
        scale_in::<Fast, Slingshot>(ss(), Mode::Fast, Removal::Drain),
    ] {
        assert_eq!(r.demands_after, 0, "{r:?}");
        assert!(r.done_after > 0, "{r:?}");
    }
}

// ---------------------------------------------------------------------------- monitors

const H14_NODE_C: &str = "_h14_call_chain_3_rc3";

/// h14 scaled out: NodeC copied to NodeD (on the LAN), Assembly C moved there (SimuLizar: a
/// new allocation context, later entries overwrite; Slingshot: the allocation context
/// re-pointed, read live), with or without NodeC's monitors cloned. Returns the measurements.
fn h14_scale_out<C: Compat, F: Flavour>(
    flavour: FlavourConfig,
    mode: simoxide_sim::Mode,
    clone: bool,
    monitors: simoxide_sim::MonitorSync,
) -> simoxide_sim::Measurements {
    let (cm, cfg) = load_flavour("h14_call_chain_3", flavour);
    let cfg = SimConfig {
        max_measurements: 400,
        mode,
        ..cfg
    };
    let mut sim = Simulation::<C, F>::create(&cm, cfg, Outputs::default()).unwrap();
    sim.set_change_options(ChangeOptions {
        monitors,
        ..sim.change_options()
    });
    sim.run_until(Pause::After(5 * S)).unwrap();
    let r = apply(&mut sim, |e| {
        let c = e.find(H14_NODE_C).unwrap();
        let d = e.clone_container(c, "_rc4", "NodeD").unwrap();
        for l in e.links_of(c) {
            e.connect(l, d).unwrap();
        }
        if flavour.kind.is_slingshot() {
            let a = e.find("_h14_call_chain_3_alc3").unwrap();
            e.set_ref(
                a,
                mf::AllocationContext_resourceContainer_AllocationContext,
                d,
            )
            .unwrap();
        } else {
            let ac = e.find("_h14_call_chain_3_ac3").unwrap();
            e.allocate(ac, d, "_alc4").unwrap();
        }
        if clone {
            let n = e.clone_monitors(c, d, "_rc4_mon").unwrap();
            assert_eq!(n.len(), 1);
        }
    });
    assert!(r.new_resources.len() == 1, "{r:?}");
    sim.finish().unwrap().measurements
}

fn node_d_tuples(m: &simoxide_sim::Measurements) -> usize {
    m.series
        .iter()
        .zip(&m.rows)
        .filter(|(s, _)| s.mp.contains("_rc4_0"))
        .map(|(_, r)| r.len())
        .sum()
}

/// Monitors for new containers: cloned monitors (Slingshot's `ResourceContainerMonitorCloner`,
/// [`Edit::clone_monitors`]) give the new container's CPU its state series in both flavours
/// (SimuLizar: `attachMonitors` of the new active resource; Slingshot: the new monitor is
/// interpreted); without them it has none.
#[test]
fn cloned_monitors_measure_a_new_container_in_both_flavours() {
    use simoxide_sim::{Mode, MonitorSync, SimuLizar, Slingshot};
    for (name, with, without) in [
        (
            "simulizar",
            h14_scale_out::<Exact, SimuLizar>(
                FlavourConfig::SIMULIZAR,
                Mode::Exact,
                true,
                MonitorSync::Reference,
            ),
            h14_scale_out::<Exact, SimuLizar>(
                FlavourConfig::SIMULIZAR,
                Mode::Exact,
                false,
                MonitorSync::Reference,
            ),
        ),
        (
            "slingshot",
            h14_scale_out::<Fast, Slingshot>(ss(), Mode::Fast, true, MonitorSync::Reference),
            h14_scale_out::<Fast, Slingshot>(ss(), Mode::Fast, false, MonitorSync::Reference),
        ),
    ] {
        assert!(
            node_d_tuples(&with) > 10,
            "{name}: {}",
            node_d_tuples(&with)
        );
        assert_eq!(node_d_tuples(&without), 0, "{name}");
        let extra: Vec<String> = with.series[without.series.len()..]
            .iter()
            .map(|s| format!("{} {}", s.mp, s.metric))
            .collect();
        assert!(
            !extra.is_empty() && extra.iter().all(|s| s.contains("_rc4_0")),
            "{name}: {extra:?}"
        );
    }
}

/// A monitor added for an existing element (callC's response time, unmonitored in this variant
/// of h14): SimuLizar creates response-time calculators only at the start (ignored),
/// `MonitorSync::Full` honours the monitor from the change on.
#[test]
fn a_new_monitor_of_an_existing_element_needs_full_monitor_sync() {
    use simoxide_sim::MonitorSync;
    let (cm, cfg) = variant("h14_call_chain_3", "monitorrepository", |t| {
        // drop the monitor of measuring point 4 (callC)
        let i = t.find("@measuringPoints.4").expect("callC's monitor");
        let a = t[..i].rfind("<monitors").unwrap();
        let b = i + t[i..].find("</monitors>").unwrap() + "</monitors>".len();
        format!("{}{}", &t[..a], &t[b..])
    });
    let cfg = SimConfig {
        max_measurements: 300,
        ..cfg
    };
    let run = |sync: MonitorSync| {
        let mut sim = Simulation::<Exact>::create(&cm, cfg.clone(), Outputs::default()).unwrap();
        sim.set_change_options(ChangeOptions {
            monitors: sync,
            ..ChangeOptions::default()
        });
        sim.run_until(Pause::After(5 * S)).unwrap();
        let r = apply(&mut sim, |e| {
            let g = e.graph();
            let repo = first_of(e, mc::MonitorRepository);
            // callB's monitor (the one on an external-call measuring point)
            let like = g
                .get_refs(repo, mf::MonitorRepository_monitors)
                .iter()
                .copied()
                .find(|&m| {
                    g.get_ref(m, mf::Monitor_measuringPoint)
                        .is_some_and(|mp| g[mp].class == mc::ExternalCallActionMeasuringPoint)
                })
                .unwrap();
            let like_mp = g.get_ref(like, mf::Monitor_measuringPoint).unwrap();
            let (mp_repo, mp_f) = g[like_mp].container.unwrap();
            let spec = g.get_refs(like, mf::Monitor_measurementSpecifications)[0];
            let metric = g
                .get_ref(spec, mf::MeasurementSpecification_metricDescription)
                .unwrap();
            let call = e.find("_h14_call_chain_3_act7").unwrap();
            let st = e.store();
            let mp = st.create(mc::ExternalCallActionMeasuringPoint).unwrap();
            st.set(
                mp,
                mf::ExternalCallActionReference_externalCall,
                Datum::Obj(call),
            )
            .unwrap();
            st.add(mp_repo, mp_f, Datum::Obj(mp), None).unwrap();
            let mon = st.create(mc::Monitor).unwrap();
            st.set(mon, mf::identifier_Identifier_id, Datum::Str("_callC_mon"))
                .unwrap();
            st.set(mon, mf::Monitor_activated, Datum::Bool(true))
                .unwrap();
            st.set(mon, mf::Monitor_measuringPoint, Datum::Obj(mp))
                .unwrap();
            let sp = st.create(mc::MeasurementSpecification).unwrap();
            st.set(
                sp,
                mf::MeasurementSpecification_metricDescription,
                Datum::Obj(metric),
            )
            .unwrap();
            let ft = st.create(mc::FeedThrough).unwrap();
            st.set(
                sp,
                mf::MeasurementSpecification_processingType,
                Datum::Obj(ft),
            )
            .unwrap();
            st.add(
                mon,
                mf::Monitor_measurementSpecifications,
                Datum::Obj(sp),
                None,
            )
            .unwrap();
            st.add(repo, mf::MonitorRepository_monitors, Datum::Obj(mon), None)
                .unwrap();
        });
        (r, sim.finish().unwrap().measurements)
    };
    let (r, reference) = run(MonitorSync::Reference);
    assert!(r.ignored.iter().any(|m| m.contains("MC-6.1")), "{r:?}");
    assert_eq!(reference.series.len(), cm.series.len());
    let (r, full) = run(MonitorSync::Full);
    assert!(
        r.applied
            .iter()
            .any(|m| m.contains("new measurement series")),
        "{r:?}"
    );
    let new: Vec<usize> = full.rows[cm.series.len()..]
        .iter()
        .map(|r| r.len())
        .collect();
    assert!(new.len() == 1 && new[0] > 10, "{new:?}");
}

// ---------------------------------------------------------------------------- behaviours in progress

/// Inserts an internal action (`id`, one demand `spec` of the resource type of `like`'s first
/// demand) after the action `after`.
fn insert_after(e: &mut Edit, after: &str, like: &str, id: &str, spec: &str) {
    let prev = e.find(after).unwrap();
    let like = e.find(like).unwrap();
    let g = e.graph();
    let (beh, steps) = g[prev].container.unwrap();
    let next = g
        .get_ref(prev, mf::AbstractAction_successor_AbstractAction)
        .unwrap();
    let ty = g
        .get_ref(
            g.get_refs(
                like,
                mf::AbstractInternalControlFlowAction_resourceDemand_Action,
            )[0],
            mf::ParametricResourceDemand_requiredResource_ParametricResourceDemand,
        )
        .unwrap();
    let st = e.store();
    let a = st.create(mc::InternalAction).unwrap();
    st.set(a, mf::identifier_Identifier_id, Datum::Str(id))
        .unwrap();
    st.set(a, mf::NamedElement_entityName, Datum::Str(id))
        .unwrap();
    st.add(beh, steps, Datum::Obj(a), None).unwrap();
    let d = st.create(mc::ParametricResourceDemand).unwrap();
    st.set(
        d,
        mf::ParametricResourceDemand_requiredResource_ParametricResourceDemand,
        Datum::Obj(ty),
    )
    .unwrap();
    st.add(
        a,
        mf::AbstractInternalControlFlowAction_resourceDemand_Action,
        Datum::Obj(d),
        None,
    )
    .unwrap();
    e.set_spec(
        d,
        mf::ParametricResourceDemand_specification_ParametericResourceDemand,
        spec,
    )
    .unwrap();
    e.set_ref(prev, mf::AbstractAction_successor_AbstractAction, a)
        .unwrap();
    e.set_ref(a, mf::AbstractAction_successor_AbstractAction, next)
        .unwrap();
}

/// Demands of value `d` after `t` by processes whose usage-scenario run began before `t`
/// (executions in progress at a change at `t`), and by later runs.
fn demands_of(trace: &str, t: f64, d: f64) -> (usize, usize) {
    let mut began: std::collections::HashMap<i64, f64> = Default::default();
    let (mut old, mut new) = (0, 0);
    for l in trace.lines() {
        let v: serde_json::Value = serde_json::from_str(l).unwrap();
        let ti = v["t"].as_f64().unwrap_or(0.0);
        let p = v["p"].as_i64().unwrap_or(-1);
        match v["ev"].as_str() {
            Some("begin") if v["type"] == "UsageScenario" => {
                began.insert(p, ti);
            }
            Some("demand") if v["d"].as_f64() == Some(d) && ti > t => {
                if began.get(&p).is_some_and(|&b| b < t) {
                    old += 1;
                } else {
                    new += 1;
                }
            }
            _ => {}
        }
    }
    (old, new)
}

/// A new action inserted after an action executions are in at the change: under `Follow`
/// (SimuLizar: the successor is read after the action) they run it; under `Finish` they finish
/// their old instructions. New executions run it either way.
fn h01_insert(in_progress: simoxide_sim::InProgress, structural: bool) -> (usize, usize) {
    let (cm, cfg) = load("h01_ps_single");
    let cfg = SimConfig {
        max_measurements: 400,
        ..cfg
    };
    let b = Buf::default();
    let mut sim = Simulation::<Exact>::create(&cm, cfg, traced(&b)).unwrap();
    sim.set_change_options(ChangeOptions {
        in_progress,
        ..ChangeOptions::default()
    });
    sim.run_until(Pause::After(50 * S)).unwrap();
    let t = sim.state().now_ns() as f64 / 1e9 + 1e-6;
    apply(&mut sim, |e| {
        if structural {
            // a second demand of the action executions are in (a structural change)
            let mut f =
                simoxide_reconf::ChangeFile::parse("add-demand _h01_ps_single_act1:CPU 0.5\n")
                    .unwrap();
            let _ = &mut f;
            let a = e.find("_h01_ps_single_act1").unwrap();
            let g = e.graph();
            let ty = g
                .get_ref(
                    g.get_refs(
                        a,
                        mf::AbstractInternalControlFlowAction_resourceDemand_Action,
                    )[0],
                    mf::ParametricResourceDemand_requiredResource_ParametricResourceDemand,
                )
                .unwrap();
            let st = e.store();
            let d = st.create(mc::ParametricResourceDemand).unwrap();
            st.set(
                d,
                mf::ParametricResourceDemand_requiredResource_ParametricResourceDemand,
                Datum::Obj(ty),
            )
            .unwrap();
            st.add(
                a,
                mf::AbstractInternalControlFlowAction_resourceDemand_Action,
                Datum::Obj(d),
                None,
            )
            .unwrap();
            e.set_spec(
                d,
                mf::ParametricResourceDemand_specification_ParametericResourceDemand,
                "0.5",
            )
            .unwrap();
        } else {
            insert_after(
                e,
                "_h01_ps_single_act1",
                "_h01_ps_single_act1",
                "_ins",
                "0.5",
            );
        }
    });
    sim.finish().unwrap();
    demands_of(&b.take(), t, 0.5)
}

#[test]
fn executions_in_progress_follow_a_changed_chain() {
    use simoxide_sim::InProgress;
    let (old, new) = h01_insert(InProgress::Follow, false);
    assert!(old > 0 && new > 0, "{old} {new}");
    let (old, new) = h01_insert(InProgress::Finish, false);
    assert!(old == 0 && new > 0, "{old} {new}");
}

/// A structural change of the action executions are in (a second demand): under `Follow` they
/// continue at the same place of the changed action and issue the new demand.
#[test]
fn executions_in_progress_follow_a_structural_change() {
    use simoxide_sim::InProgress;
    let (old, new) = h01_insert(InProgress::Follow, true);
    assert!(old > 0 && new > 0, "{old} {new}");
    let (old, new) = h01_insert(InProgress::Finish, true);
    assert!(old == 0 && new > 0, "{old} {new}");
}

/// The load balancer gains a third branch mid-run (new requests take it: routing changes) and
/// an action after its branch: executions in progress inside a branch run that action when they
/// return (SimuLizar's successor chain), new ones too.
#[test]
fn a_load_balancer_branch_added_mid_run_with_executions_in_progress() {
    let (cm, cfg) = load_dir_flavour(LB, FlavourConfig::SIMULIZAR);
    let b = Buf::default();
    let mut sim = Simulation::<Exact>::create(&cm, cfg, traced(&b)).unwrap();
    sim.run_until(Pause::After(100 * S)).unwrap();
    let t = sim.state().now_ns() as f64 / 1e9 + 1e-6;
    let r = apply(&mut sim, |e| {
        let branch = e.find("_m2PcEAEoEeS7FKokKTKFow").unwrap();
        let like = e.find("_7FXSsAEmEeS7FKokKTKFow").unwrap();
        let ty = {
            let g = e.graph();
            g.get_ref(
                g.get_refs(
                    like,
                    mf::AbstractInternalControlFlowAction_resourceDemand_Action,
                )[0],
                mf::ParametricResourceDemand_requiredResource_ParametricResourceDemand,
            )
            .unwrap()
        };
        // the third branch: start -> a local demand of 0.0625 -> stop
        let st = e.store();
        let beh = st.create(mc::ResourceDemandingBehaviour).unwrap();
        st.set(beh, mf::identifier_Identifier_id, Datum::Str("_b3"))
            .unwrap();
        let start = st.create(mc::StartAction).unwrap();
        st.set(start, mf::identifier_Identifier_id, Datum::Str("_b3_start"))
            .unwrap();
        let work = st.create(mc::InternalAction).unwrap();
        st.set(work, mf::identifier_Identifier_id, Datum::Str("_b3_work"))
            .unwrap();
        let stop = st.create(mc::StopAction).unwrap();
        st.set(stop, mf::identifier_Identifier_id, Datum::Str("_b3_stop"))
            .unwrap();
        for a in [start, work, stop] {
            st.add(
                beh,
                mf::ResourceDemandingBehaviour_steps_Behaviour,
                Datum::Obj(a),
                None,
            )
            .unwrap();
        }
        st.set(
            start,
            mf::AbstractAction_successor_AbstractAction,
            Datum::Obj(work),
        )
        .unwrap();
        st.set(
            work,
            mf::AbstractAction_successor_AbstractAction,
            Datum::Obj(stop),
        )
        .unwrap();
        let d = st.create(mc::ParametricResourceDemand).unwrap();
        st.set(
            d,
            mf::ParametricResourceDemand_requiredResource_ParametricResourceDemand,
            Datum::Obj(ty),
        )
        .unwrap();
        st.add(
            work,
            mf::AbstractInternalControlFlowAction_resourceDemand_Action,
            Datum::Obj(d),
            None,
        )
        .unwrap();
        let tr = st.create(mc::ProbabilisticBranchTransition).unwrap();
        st.set(tr, mf::identifier_Identifier_id, Datum::Str("_b3_tr"))
            .unwrap();
        st.set(
            tr,
            mf::ProbabilisticBranchTransition_branchProbability,
            Datum::Double(0.5),
        )
        .unwrap();
        st.set(
            tr,
            mf::AbstractBranchTransition_branchBehaviour_BranchTransition,
            Datum::Obj(beh),
        )
        .unwrap();
        st.add(
            branch,
            mf::BranchAction_branches_Branch,
            Datum::Obj(tr),
            None,
        )
        .unwrap();
        e.set_spec(
            d,
            mf::ParametricResourceDemand_specification_ParametericResourceDemand,
            "0.0625",
        )
        .unwrap();
        set_probability(e, LB_TO_SERVER1, 0.5);
        // after the branch: a demand of 0.125 on the load balancer
        insert_after(
            e,
            "_m2PcEAEoEeS7FKokKTKFow",
            "_7FXSsAEmEeS7FKokKTKFow",
            "_after",
            "0.125",
        );
    });
    assert!(r.code_appended, "{r:?}");
    sim.finish().unwrap();
    let trace = b.take();
    // routing: the new branch is taken by new requests
    let (_, new_branch) = demands_of(&trace, t, 0.0625);
    assert!(new_branch > 0);
    // executions in progress (inside a server's branch at the change) run the new successor
    let (in_progress, later) = demands_of(&trace, t, 0.125);
    assert!(in_progress > 0 && later > 0, "{in_progress} {later}");
}

// ---------------------------------------------------------------------------- add and remove in one set

/// A container (or link) added to the environment and removed again in one change set:
/// SimuLizar's syncer created and registered its resources at the addition (`Keep`: the id is
/// registered, so a later container with that id is not created; the resources are unreachable
/// and unmonitored); drained or aborted it never had any.
#[test]
fn a_container_added_and_removed_in_one_set() {
    for (removal, registered) in [
        (Removal::Keep, true),
        (Removal::Drain, false),
        (Removal::Abort, false),
    ] {
        let (cm, cfg) = load(H19);
        let mut sim = Simulation::<Exact>::create(&cm, cfg, Outputs::default()).unwrap();
        sim.set_change_options(ChangeOptions {
            removal,
            ..ChangeOptions::default()
        });
        sim.run_until(Pause::After(5 * S)).unwrap();
        let r = apply(&mut sim, |e| {
            let server = e.find(H19_SERVER).unwrap();
            let c = e.clone_container(server, "_rc9", "Temp").unwrap();
            e.remove_from_environment(c).unwrap();
        });
        assert!(r.new_resources.is_empty(), "{removal:?}: {r:?}");
        let has = sim.compiled().registry.iter().any(|e| &*e.0 == "_rc9");
        assert_eq!(has, registered, "{removal:?}: {r:?}");
        if registered {
            // SimuLizar would not create a container with that id later
            let mut e = Edit::new(&sim);
            let server = e.find(H19_SERVER).unwrap();
            e.clone_container(server, "_rc9", "Again").unwrap();
            assert!(matches!(
                e.apply(&mut sim),
                Err(simoxide_sim::ChangeError::Unsupported(_))
            ));
        }
        assert!(sim.finish().is_ok());
    }
    // a link likewise
    let (cm, cfg) = load(H19);
    let mut sim = Simulation::<Exact>::create(&cm, cfg, Outputs::default()).unwrap();
    sim.set_change_options(ChangeOptions::simulizar());
    sim.run_until(Pause::After(5 * S)).unwrap();
    let r = apply(&mut sim, |e| {
        let env = first_of(e, mc::ResourceEnvironment);
        let st = e.store();
        let l = st.create(mc::LinkingResource).unwrap();
        st.set(l, mf::identifier_Identifier_id, Datum::Str("_tmp_link"))
            .unwrap();
        st.add(
            env,
            mf::ResourceEnvironment_linkingResources__ResourceEnvironment,
            Datum::Obj(l),
            None,
        )
        .unwrap();
        st.remove(
            env,
            mf::ResourceEnvironment_linkingResources__ResourceEnvironment,
            Datum::Obj(l),
        )
        .unwrap();
    });
    assert!(
        r.applied.iter().any(|m| m.contains("removed again")),
        "{r:?}"
    );
    assert!(sim.compiled().registry.iter().any(|e| &*e.0 == "_tmp_link"));
    assert!(sim.finish().is_ok());
}

// ---------------------------------------------------------------------------- usage scenarios

/// A usage scenario removed mid-run: its workload driver stops (closed users after their run
/// and think time, the open generator at its next arrival); the other scenario goes on.
#[test]
fn a_removed_usage_scenario_stops_its_users() {
    for (scen, other) in [
        ("_h24_two_scenarios_us2", "_h24_two_scenarios_us1"),
        ("_h24_two_scenarios_us1", "_h24_two_scenarios_us2"),
    ] {
        let (cm, cfg) = load("h24_two_scenarios");
        let cfg = SimConfig {
            max_sim_time: 60,
            ..cfg
        };
        let b = Buf::default();
        let mut sim = Simulation::<Exact>::create(&cm, cfg, traced(&b)).unwrap();
        sim.run_until(Pause::After(10 * S)).unwrap();
        let r = apply(&mut sim, |e| {
            let s = e.find(scen).unwrap();
            e.delete(s);
        });
        assert!(
            r.applied.iter().any(|m| m.contains("driver stops")),
            "{r:?}"
        );
        let res = sim.finish().unwrap();
        let trace = b.take();
        assert!(res.end_ns >= 60 * S, "{scen}");
        let begins = |id: &str, from: f64| {
            trace
                .lines()
                .filter(|l| {
                    l.starts_with("{\"ev\":\"begin\"") && l.contains(&format!("\"id\":\"{id}\""))
                })
                .filter(|l| {
                    let v: serde_json::Value = serde_json::from_str(l).unwrap();
                    v["t"].as_f64().unwrap() > from
                })
                .count()
        };
        // (a closed user may start one more run within its think time of 1.5 s)
        assert_eq!(begins(scen, 15.0), 0, "{scen}");
        assert!(begins(other, 15.0) > 0, "{other}");
    }
}

// ---------------------------------------------------------------------------- snapshots

/// A snapshot written after a change of each kind, restored into a fresh simulation, continues
/// byte for byte like the run that was not interrupted (N1 after F4's changes).
fn snapshot_continues<C: Compat, F: Flavour>(
    cm: &CompiledModel,
    cfg: &SimConfig,
    opts: Option<ChangeOptions>,
    at: i64,
    change: &dyn Fn(&mut Edit),
) {
    let full = |snap: bool| -> String {
        let mut sim = Simulation::<C, F>::create(cm, cfg.clone(), Outputs::default()).unwrap();
        if let Some(o) = opts {
            sim.set_change_options(o);
        }
        sim.run_until(Pause::After(at * S)).unwrap();
        let mut e = Edit::new(&sim);
        change(&mut e);
        e.apply(&mut sim).unwrap();
        sim.run_until(Pause::After((at + 3) * S)).unwrap();
        let sim = if snap {
            let mut bytes = Vec::new();
            sim.snapshot_to(&mut bytes).unwrap();
            drop(sim);
            let mut r =
                Simulation::<C, F>::restore_from(&bytes[..], cm, Outputs::default()).unwrap();
            // a second hop: a snapshot of the restored run restores to the same bytes
            let mut again = Vec::new();
            r.snapshot_to(&mut again).unwrap();
            assert!(
                again == bytes,
                "the restored state writes the bytes it was read from"
            );
            r
        } else {
            sim
        };
        summary(&sim.finish().unwrap())
    };
    assert_eq!(full(false), full(true));
}

#[test]
fn snapshots_after_each_change_kind_continue_identically() {
    use simoxide_sim::{InProgress, Mode, MonitorSync, SimuLizar, Slingshot};
    // population
    let (cm, cfg) = load("h06_closed_think");
    let cfg = SimConfig {
        max_measurements: 300,
        ..cfg
    };
    snapshot_continues::<Exact, SimuLizar>(&cm, &cfg, None, 10, &|e| set_population(e, 9));
    snapshot_continues::<Exact, SimuLizar>(&cm, &cfg, None, 10, &|e| set_population(e, 1));
    // a usage scenario removed
    let (cm, cfg) = load("h24_two_scenarios");
    let cfg = SimConfig {
        max_sim_time: 60,
        ..cfg
    };
    snapshot_continues::<Exact, SimuLizar>(&cm, &cfg, None, 10, &|e| {
        let s = e.find("_h24_two_scenarios_us1").unwrap();
        e.delete(s);
    });
    // monitors for a new container (both flavours), full monitor sync
    let clone = |e: &mut Edit| {
        let c = e.find(H14_NODE_C).unwrap();
        let d = e.clone_container(c, "_rc4", "NodeD").unwrap();
        for l in e.links_of(c) {
            e.connect(l, d).unwrap();
        }
        let ac = e.find("_h14_call_chain_3_ac3").unwrap();
        e.allocate(ac, d, "_alc4").unwrap();
        e.clone_monitors(c, d, "_rc4_mon").unwrap();
    };
    let (cm, cfg) = load("h14_call_chain_3");
    let cfg = SimConfig {
        max_measurements: 300,
        ..cfg
    };
    snapshot_continues::<Exact, SimuLizar>(&cm, &cfg, None, 5, &clone);
    snapshot_continues::<Exact, SimuLizar>(
        &cm,
        &cfg,
        Some(ChangeOptions {
            monitors: MonitorSync::Full,
            ..ChangeOptions::default()
        }),
        5,
        &clone,
    );
    let (cm, cfg) = load_flavour("h14_call_chain_3", ss());
    let cfg = SimConfig {
        max_measurements: 300,
        mode: Mode::Fast,
        ..cfg
    };
    snapshot_continues::<Fast, Slingshot>(&cm, &cfg, None, 5, &|e| {
        let c = e.find(H14_NODE_C).unwrap();
        let d = e.clone_container(c, "_rc4", "NodeD").unwrap();
        for l in e.links_of(c) {
            e.connect(l, d).unwrap();
        }
        let a = e.find("_h14_call_chain_3_alc3").unwrap();
        e.set_ref(
            a,
            mf::AllocationContext_resourceContainer_AllocationContext,
            d,
        )
        .unwrap();
        e.clone_monitors(c, d, "_rc4_mon").unwrap();
    });
    // removal policies (the scale-in of ServerNode2, its branch kept)
    let scale_in = |e: &mut Edit| {
        let node = e.find(LB_NODE2).unwrap();
        e.remove_from_environment(node).unwrap();
    };
    let to2 = |e: &mut Edit| {
        set_probability(e, LB_TO_SERVER1, 0.5);
        set_probability(e, LB_TO_SERVER2, 0.5);
        scale_in(e);
    };
    for removal in [Removal::Keep, Removal::Drain, Removal::Abort] {
        let (cm, cfg) = load_dir_flavour(LB, FlavourConfig::SIMULIZAR);
        snapshot_continues::<Exact, SimuLizar>(
            &cm,
            &cfg,
            Some(ChangeOptions {
                removal,
                ..ChangeOptions::default()
            }),
            100,
            &to2,
        );
    }
    for (flavour, removal) in [
        (ss(), Removal::Drain),
        (ss(), Removal::Abort),
        (ss_q5(), Removal::SlingshotQ5),
    ] {
        let (cm, cfg) = load_dir_flavour(LB, flavour);
        let cfg = SimConfig {
            mode: Mode::Fast,
            ..cfg
        };
        snapshot_continues::<Fast, Slingshot>(
            &cm,
            &cfg,
            Some(ChangeOptions {
                removal,
                ..ChangeOptions::for_flavour(flavour)
            }),
            100,
            &to2,
        );
    }
    // behaviours: executions in progress follow a structural change, or keep their old
    // instructions (kept segments in the snapshot)
    let (cm, cfg) = load("h01_ps_single");
    let cfg = SimConfig {
        max_measurements: 300,
        ..cfg
    };
    for in_progress in [InProgress::Follow, InProgress::Finish] {
        snapshot_continues::<Exact, SimuLizar>(
            &cm,
            &cfg,
            Some(ChangeOptions {
                in_progress,
                ..ChangeOptions::default()
            }),
            20,
            &|e| {
                insert_after(
                    e,
                    "_h01_ps_single_act1",
                    "_h01_ps_single_act1",
                    "_ins",
                    "0.5",
                )
            },
        );
    }
    // added and removed in one set (kept resources)
    let (cm, cfg) = load(H19);
    snapshot_continues::<Exact, SimuLizar>(&cm, &cfg, Some(ChangeOptions::simulizar()), 5, &|e| {
        let server = e.find(H19_SERVER).unwrap();
        let c = e.clone_container(server, "_rc9", "Temp").unwrap();
        e.remove_from_environment(c).unwrap();
    });
}

// ---------------------------------------------------------------------------- change files

/// N2's change files (`ChangeFile`) carry F4's change kinds end to end: population, monitors
/// for new containers, removal policies, a structural behaviour change, add and remove in one
/// set; also in the Slingshot flavour.
#[test]
fn change_files_carry_f4_changes_in_both_flavours() {
    use simoxide_reconf::ChangeFile;
    use simoxide_sim::{Mode, SimuLizar, Slingshot};
    fn go<C: Compat, F: Flavour>(
        name: &str,
        flavour: FlavourConfig,
        mode: Mode,
        at: i64,
        file: &str,
    ) -> (ChangeReport, RunResult) {
        let (cm, cfg) = load_flavour(name, flavour);
        let cfg = SimConfig {
            max_measurements: 300,
            mode,
            ..cfg
        };
        let mut sim = Simulation::<C, F>::create(&cm, cfg, Outputs::default()).unwrap();
        sim.run_until(Pause::After(at * S)).unwrap();
        let r = ChangeFile::parse(file).unwrap().apply(&mut sim).unwrap();
        (r, sim.finish().unwrap())
    }
    let files: &[(&str, i64, &str, &str)] = &[
        (
            "h06_closed_think",
            10,
            "population Scenario 8\n",
            "population",
        ),
        (
            "h14_call_chain_3",
            5,
            "add-container NodeC _rc4 NodeD\nclone-monitors NodeC _rc4\nallocate Assembly_C _rc4 _alc4\n",
            "new measurement series",
        ),
        (
            H19,
            5,
            "removal abort\nremove-container ServerNode\n",
            "Abort",
        ),
        (
            "h01_ps_single",
            20,
            "in-progress follow\nadd-demand _h01_ps_single_act1:CPU 0.5\n",
            "behaviours recompiled",
        ),
        (
            H19,
            5,
            "add-container ServerNode _rc9 Temp\ndelete _rc9\n",
            "removed again",
        ),
    ];
    for &(name, at, file, expect) in files {
        let (r, res) =
            go::<Exact, SimuLizar>(name, FlavourConfig::SIMULIZAR, Mode::Exact, at, file);
        assert!(
            r.applied
                .iter()
                .chain(&r.ignored)
                .any(|m| m.contains(expect)),
            "simulizar {name}: {r:?}"
        );
        assert!(res.main_count > 0);
        if name == "h14_call_chain_3" {
            // Slingshot: the first allocation context wins, so the new container takes Assembly C
            // when its allocation context is re-pointed (`set`)
            let f = "add-container NodeC _rc4 NodeD\nclone-monitors NodeC _rc4\nset _h14_call_chain_3_alc3 resourceContainer_AllocationContext _rc4\n";
            let (r, res) = go::<Fast, Slingshot>(name, ss(), Mode::Fast, at, f);
            assert!(
                r.applied.iter().any(|m| m.contains("new resource")),
                "{r:?}"
            );
            assert!(
                res.measurements
                    .series
                    .iter()
                    .any(|s| s.mp.contains("_rc4_0"))
            );
            continue;
        }
        let (r, res) = go::<Fast, Slingshot>(name, ss(), Mode::Fast, at, file);
        assert!(
            !r.applied.is_empty() || !r.ignored.is_empty(),
            "slingshot {name}: {r:?}"
        );
        assert!(res.main_count > 0);
    }
}

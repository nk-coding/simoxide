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

// ---------------------------------------------------------------------------- helpers for later

#[allow(dead_code)]
fn apply<C: Compat, F: Flavour>(
    sim: &mut Simulation<'_, C, F>,
    f: impl FnOnce(&mut Edit),
) -> ChangeReport {
    let mut e = Edit::new(sim);
    f(&mut e);
    e.apply(sim).unwrap()
}

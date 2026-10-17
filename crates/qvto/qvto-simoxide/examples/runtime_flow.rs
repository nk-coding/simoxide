//! The intended runtime flow inside SimOxide, end to end on SimOxide's own loader and graph:
//!
//! 1. SimOxide loads the PCM model (`simoxide_model::load_dir`); the session gets SimuLizar's
//!    runtime-measurement metamodel from embedded `.ecore` text (no files at run time).
//! 2. The reconfiguration rule is compiled once; `getSimulationTime()` is bound to the clock.
//! 3. At each measurement the host writes the value into the PRM model, runs the rule on the
//!    graph with the journal on, and hands the journal to a (stub) syncer callback, which says
//!    what SimOxide would have to update (and where SimuLizar's syncers have gaps).
//! 4. At the end SimOxide's typed model is rebuilt from the changed graph and validated.
//!
//! `cargo run --example runtime_flow [-- <model dir>]` (default: SimOxide's MediaStore model).

use qvto::blackbox::{BlackboxResolver, Natives, palladio};
use qvto_model::{Datum, ModelStore, ResourceId};
use qvto_simoxide::{ChangeKind, IdPolicy, Journal, Session, SimStore};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

const SRM: &str = "http://simulizar.palladiosimulator.org/RuntimeMeasurement/1.0";
const SRM_LOCATION: &str = "platform:/plugin/org.palladiosimulator.simulizar.runtimemeasurement/model/runtimemeasurement.ecore";
const SRM_ECORE: &str = include_str!("../data/runtimemeasurement.ecore");

/// A synthetic utilisation curve: one value per measurement, every 10 time units.
const UTILISATION: &[f64] = &[0.5, 0.9, 0.95, 0.85, 0.7, 0.6, 0.2, 0.1, 0.25, 0.5];

fn main() {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| qvto_paths::workspace_root(&qvto_paths::root()).join("corpus/x_sl_mediastore"));
    // 1. SimOxide's model, the session
    let mut model = simoxide_model::load_dir(&dir).expect("model directory");
    println!(
        "loaded {} with SimOxide: {} objects, {} containers",
        dir.display(),
        model.graph.objs.len(),
        model.containers.len()
    );
    let t0 = std::time::Instant::now();
    let mut session = Session::new();
    session.set_id_policy(IdPolicy::Uuid { seed: 2026 });
    session.load_metamodel_text(SRM_LOCATION, SRM_ECORE).expect("runtime measurement metamodel");
    println!("session (registry, metamodel objects, PRM metamodel): {:.1?}", t0.elapsed());
    let t0 = std::time::Instant::now();

    // 2. the rule, compiled once; the clock
    let rule = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/rules/autoscale.qvto");
    let files = qvto::units::FsResolver::folder();
    let resolver = BlackboxResolver {
        inner: &files,
        registered: Vec::new(),
        classpath: palladio::catalog().units("simulizar").to_vec(),
    };
    let compiled = qvto::compile_with_metamodel(
        files.unit_source(&rule).unwrap(),
        &resolver,
        session.meta().clone(),
        &session.operations(),
        &session.derivations(),
    );
    for u in &compiled.units {
        for p in &u.problems {
            eprintln!("{}:{}: {}", u.source.file_name, p.line(&u.lines), p.message);
        }
    }
    let program = compiled.program.expect("the rule compiles");
    println!("rule compiled: {:.1?}", t0.elapsed());
    let clock = Rc::new(Cell::new(0.0));

    // the PRM model: one measurement of a measuring point of the model's monitors
    let (prm, measurement, inputs) = {
        let mut store = session.store(&mut model.graph);
        let m = store.meta();
        let p = m.package_by_ns_uri(SRM).unwrap();
        let rm = m.class_by_name(p, "RuntimeMeasurement").unwrap();
        let mp_feature = m.feature_by_name(rm, "measuringPoint").unwrap();
        let mp_class = m.feature(mp_feature).class_type().unwrap();
        let prm = store.create_model_resource("simoxide:/prm");
        let meas = store.create(rm).unwrap();
        store.add_root(prm, meas, None).unwrap();
        // a reference from the PRM into SimOxide's graph (through a mirror)
        let mp = (0..store.resource_count() as u32)
            .flat_map(|r| store.resource_contents(ResourceId(r)))
            .find(|&o| store.is_graph_object(o) && store.meta().is_subclass(store.class_of(o), mp_class));
        if let Some(mp) = mp {
            store.set(meas, mp_feature, Datum::Obj(mp)).unwrap();
        }
        let by_ext = |s: &SimStore, ext: &str| {
            (0..s.resource_count() as u32)
                .map(ResourceId)
                .find(|r| s.resource_uri(*r).ends_with(ext))
                .unwrap()
        };
        let inputs = [prm, by_ext(&store, ".allocation"), by_ext(&store, ".resourceenvironment")];
        (prm, meas, inputs)
    };
    let _ = prm;

    // 3. the simulation's measurements
    for (k, &u) in UTILISATION.iter().enumerate() {
        let t = 10.0 * k as f64;
        clock.set(t);
        session.stop_journal();
        let mut store = SimStore::new(&mut model.graph, &mut session);
        let rm = store.class_of(measurement);
        let f = store.meta().feature_by_name(rm, "measuringValue").unwrap();
        store.set(measurement, f, Datum::Double(u)).unwrap();
        // the rule run, recorded
        store.session().start_journal();
        let args: Vec<Vec<u32>> = inputs.iter().map(|r| store.roots(*r).to_vec()).collect();
        let started = std::time::Instant::now();
        let (out, log) = {
            // the store is a view of this step; its natives are made per run (cheap)
            let mut natives = Natives::new();
            let c = clock.clone();
            palladio::register_simulation_time(&mut natives, move || c.get());
            let mut vm = qvto::Vm::new(&program, &mut store, qvto::RunOptions::default());
            vm.set_natives(Rc::new(natives));
            let out = vm.run_transformation(args);
            (out, std::mem::take(&mut vm.log))
        };
        let took = started.elapsed();
        assert_eq!(out.severity, 0, "{}", out.message);
        print!("{log}");
        let journal = store.session().take_journal();
        println!("  rule run {took:.1?}, {} changes", journal.changes.len());
        syncer(&journal, &store);
        if !journal.is_empty() {
            // what option D would do incrementally: here the typed model is simply rebuilt
            let t = std::time::Instant::now();
            let m = simoxide_model::build_from_graph(store.graph().clone(), Default::default());
            println!(
                "  SimOxide's typed model rebuilt ({:.1?}): {} containers, {} processing resources",
                t.elapsed(),
                m.containers.len(),
                m.processing_resources.len()
            );
        }
    }

    // 4. SimOxide's typed model from the changed graph
    let rebuilt = simoxide_model::build_from_graph(model.graph.clone(), Default::default());
    let errors = rebuilt
        .diagnostics
        .iter()
        .filter(|d| d.level == simoxide_model::Level::Error)
        .count();
    println!(
        "rebuilt SimOxide's model from the changed graph: {} containers, {} processing resources, {errors} errors",
        rebuilt.containers.len(),
        rebuilt.processing_resources.len()
    );
}

/// What SimOxide would do with the changes (stub): the counterparts of SimuLizar's syncers.
fn syncer(j: &Journal, s: &SimStore) {
    let m = s.meta();
    let name = |o: u32| {
        let c = s.class_of(o);
        let n = m
            .feature_by_name(c, "entityName")
            .and_then(|f| s.get(o, f).as_str().map(String::from))
            .unwrap_or_default();
        format!("{} '{n}'", m.class(c).name)
    };
    let sum = j.summary();
    for c in &j.changes {
        if !s.is_graph_object(c.obj) || sum.created.contains_key(&c.obj) {
            continue; // the PRM model, intermediate data; new objects come with their container
        }
        match &c.kind {
            ChangeKind::Added { feature, value: qvto_model::OwnedDatum::Obj(x), .. }
                if &*m.feature(*feature).name == "resourceContainer_ResourceEnvironment" =>
            {
                println!("  syncer: new {} -> create its active and linking resources, update routes and the allocation lookup", name(*x));
            }
            ChangeKind::Removed { feature, value: qvto_model::OwnedDatum::Obj(x), .. }
                if &*m.feature(*feature).name == "resourceContainer_ResourceEnvironment" =>
            {
                println!("  syncer: removed {} -> drain and remove its resources (SimuLizar's syncer does not handle this: the container keeps simulating)", name(*x));
            }
            ChangeKind::Set { feature, .. } if &*m.feature(*feature).name == "specification" => {
                println!("  syncer: StoEx of {} changed -> recompile it", name(c.obj));
            }
            ChangeKind::Set { feature, .. }
                if &*m.feature(*feature).name == "resourceContainer_AllocationContext" =>
            {
                println!("  syncer: {} moved to another container -> re-route (SimuLizar ignores this)", name(c.obj));
            }
            _ => {}
        }
    }
    if !j.is_empty() {
        println!(
            "  journal: {} created, {} objects with changed features, {} moved, {} deleted",
            sum.created.len(),
            sum.features.len(),
            sum.moved.len(),
            sum.deleted.len()
        );
    }
}

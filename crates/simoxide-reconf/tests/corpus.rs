//! F3's exit criterion (`DESIGN-SPD.md` §7): the 24 SimuLizar/SPD corpus cases of the QVTo
//! engine run on a simulation's `RunModel` in the middle of the run, through the host
//! (`QvtoRun::invoke`), and
//!
//! - their models, log and status equal the Java runner's results (the corpus fixture), with the
//!   Java run's identity hashes when cached (`target/qvto-testkit/java`), as the adapter's
//!   corpus test does;
//! - their journals equal the journals of the same transformation on the case's models loaded
//!   on their own (the adapter's setup, outside a simulation), object by object;
//! - their changes go through the change path: applied, refused (the run unchanged) or making
//!   SimuLizar abort (the run fails), as listed per case below; a run that is not failed
//!   continues to its end.

mod common;

use common::*;
use qvto_model::{ModelStore, ResourceId};
use qvto_simoxide::{IdPolicy, Session, SimStore};
use qvto_testkit::compare::renumber_extent_resources as renumber;
use simoxide_reconf::{
    Invocation, InvokeHooks, QvtMode, QvtoRun, ReconfError, RunConfig, Transformation,
};
use simoxide_sim::{ChangeError, Outputs, Pause, Simulation};

/// What the change path does with each case's changes, mid-run: `none` (the rule changed
/// nothing: below its threshold), `applied`, or `invalid` (refused, the run unchanged: the
/// rule adds an assembly context without allocating it, which SimOxide's model validation
/// rejects).
fn expected(id: &str) -> &'static str {
    match id {
        "simulizar_runconfigtest_scaleUp_high"
        | "simulizar_runconfigtest_outsource_high"
        | "simulizar_reconfiguration_adapt"
        | "simulizar_loadbalancer_scaleUp_high"
        | "simulizar_loadbalancer_scaleDown_low"
        | "simulizar_loadbalancer_outsource_high"
        | "simulizar_powerconsumption_scaleUp_high"
        | "simulizar_powerconsumption_scaleDown_high"
        | "simulizar_powerconsumption_scaleIn_high"
        | "spd_minimalexample_main" => "applied",
        "simulizar_runconfigtest_addNewServer_high"
        | "simulizar_runconfigtest_addClonedServer_high" => "invalid",
        _ => "none",
    }
}

/// The transformation on the case's models loaded on their own (the adapter's set-up): the
/// keyed journal.
fn standalone(c: &qvto_testkit::case::Case, t: &Transformation) -> Vec<String> {
    let _ = t;
    let root = qroot();
    let mut session = Session::new();
    session.resource_set_mut().add_alias(
        "platform:/plugin/org.eclipse.emf.ecore/model/Ecore.ecore",
        "http://www.eclipse.org/emf/2002/Ecore",
    );
    for (loc, text) in simoxide_reconf::palladio_metamodels() {
        session.load_metamodel_text(loc, text).unwrap();
    }
    let mut loader = simoxide_model::Loader::default();
    let mut kinds = Vec::new();
    for p in &c.inputs {
        let file = c.path(&root, p).canonicalize().unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        if is_graph_document(&text) {
            let r = loader.load_file(&file);
            kinds.push((file, Some(r)));
        } else {
            for href in text.split("href=\"").skip(1) {
                let doc = href.split(['"', '#']).next().unwrap_or("");
                if !doc.is_empty() && !doc.contains(':') {
                    let f = file.parent().unwrap().join(doc);
                    if let Ok(t) = std::fs::read_to_string(&f)
                        && is_graph_document(&t)
                    {
                        loader.load_file(f.canonicalize().unwrap());
                    }
                }
            }
            kinds.push((file, None));
        }
    }
    let (mut graph, _) = loader.finish();
    let mut inputs = Vec::new();
    {
        let mut store = session.store(&mut graph);
        for (file, r) in &kinds {
            inputs.push(match r {
                Some(r) => store.graph_resource(*r).unwrap(),
                None => store.load_model(file).unwrap(),
            });
        }
    }
    // compile against this session's registry (equal to the core's)
    let path = c.path(&root, c.qvto.as_deref().unwrap());
    let files = match &c.import_root {
        Some(r) => qvto::units::FsResolver {
            bundles: vec![qvto::units::Bundle {
                root: root.join(r).canonicalize().unwrap(),
                containers: vec![String::new()],
            }],
            from_folder: false,
        },
        None => qvto::units::FsResolver::folder(),
    };
    let cat = qvto::blackbox::palladio::catalog();
    let mut registered = Vec::new();
    for env in ["simulizar", "architecturaltemplates"] {
        registered.extend(cat.units(env).iter().cloned());
    }
    let bb = qvto::blackbox::BlackboxResolver {
        inner: &files,
        registered,
        classpath: Vec::new(),
    };
    let compiled = qvto::compile_with_metamodel(
        files.unit_source(&path).unwrap(),
        &bb,
        session.meta().clone(),
        &session.operations(),
        &session.derivations(),
    );
    let program = compiled.program.expect("compiles");
    let mut store = SimStore::new(&mut graph, &mut session);
    let keys = object_keys(&store);
    let ids = load_tape(c).map(|t| id_source(&store, &t));
    let ir = &program.ir;
    let main = &ir.modules[ir.main_module.index()];
    let mut it = inputs.iter();
    let args: Vec<Vec<u32>> = main
        .model_params
        .iter()
        .map(|mp| {
            if mp.direction == qvto_sema::ir::Direction::Out {
                return Vec::new();
            }
            it.next()
                .map(|r: &ResourceId| store.roots(*r).to_vec())
                .unwrap_or_default()
        })
        .collect();
    store.session().start_journal();
    let opts = qvto::RunOptions {
        mode: qvto_runtime::Mode::Compat,
        id_source: ids,
        config: c.config.clone(),
        ..qvto::RunOptions::default()
    };
    let mut natives = qvto::blackbox::Natives::new();
    qvto::blackbox::palladio::register_simulation_time(&mut natives, || 0.0);
    {
        let mut vm = qvto::Vm::new(&program, &mut store, opts);
        vm.set_natives(std::rc::Rc::new(natives));
        vm.run_transformation(args);
    }
    let j = store.session().take_journal();
    keyed_journal(&keys, &j)
}

#[test]
fn corpus_rules_on_a_run_model_mid_run() {
    let java = java();
    let mut report = Vec::new();
    let mut failures = Vec::new();
    for c in cases() {
        let cm = sim_model(&c);
        let mut sim = Simulation::new(&cm, sim_config(), Outputs::default()).unwrap();
        // mid-run: half of the simulated time
        sim.run_until(Pause::After(100_000_000_000)).unwrap();
        let events_before = sim.state().events();
        let t = transformation(&c);
        let mut run = QvtoRun::new(
            core(),
            RunConfig {
                mode: QvtMode::Compat,
                ids: IdPolicy::None,
                ..RunConfig::default()
            },
        )
        .unwrap();
        let models = inputs(&c, &sim, &mut run);
        let inv = Invocation {
            binds: binds(&t, &models),
            config: c
                .config
                .iter()
                .map(|(k, v)| (k.clone(), v.clone().unwrap_or_default()))
                .collect(),
            ..Invocation::default()
        };
        let keys = run.with_scratch_store(&sim, |s| object_keys(s));
        let tape = load_tape(&c);
        let j = &java[&c.id];
        let unstable = j.unstable();
        let jd = j.dump().unwrap_or("");
        let labels = dump_labels(jd);
        // a dry run: the transformation's result, compared with the Java runner's
        let mut ids = |s: &SimStore<'_>| id_source(s, tape.as_ref().unwrap());
        let (dry, d) = run
            .dry_run(
                &sim,
                &t,
                &inv,
                InvokeHooks {
                    before: None,
                    ids: if tape.is_some() { Some(&mut ids) } else { None },
                },
                |s, o| dump(s, &o.extents, &labels),
            )
            .unwrap();
        let mut diffs = Vec::new();
        if renumber(&d) != renumber(jd) && !unstable.contains(&"dump") {
            let diff = qvto_emf::canon::first_difference(&renumber(jd), &renumber(&d));
            diffs.push(format!("dump vs java: {diff:?}"));
        }
        if dry.log != j.log() && !unstable.contains(&"log") {
            diffs.push(format!("log: {:?} vs java {:?}", dry.log, j.log()));
        }
        let sev = match dry.severity {
            0 => "OK",
            2 => "WARNING",
            8 => "CANCEL",
            _ => "ERROR",
        };
        if Some(sev) != j.severity() || Some(dry.code) != j.code() {
            diffs.push(format!(
                "status {sev} {} vs java {:?} {:?}",
                dry.code,
                j.severity(),
                j.code()
            ));
        }
        // the run itself
        let mut ids = |s: &SimStore<'_>| id_source(s, tape.as_ref().unwrap());
        let r = run.invoke_with(
            &mut sim,
            &t,
            &inv,
            InvokeHooks {
                before: None,
                ids: if tape.is_some() { Some(&mut ids) } else { None },
            },
        );
        let (outcome, applied) = match &r {
            Ok(o) => (
                o.clone(),
                if o.change.is_some() {
                    "applied"
                } else {
                    "none"
                }
                .to_string(),
            ),
            Err(ReconfError::Rejected { outcome, error }) => (
                (**outcome).clone(),
                match error {
                    ChangeError::Unsupported(_) => "unsupported".to_string(),
                    ChangeError::Invalid(v) => {
                        if std::env::var_os("RECONF_VERBOSE").is_some() {
                            eprintln!("{}: invalid: {v:?}", c.id);
                        }
                        "invalid".to_string()
                    }
                    e => format!("rejected {e}"),
                },
            ),
            Err(ReconfError::RunFailed { outcome, .. }) => {
                ((**outcome).clone(), "abort".to_string())
            }
            Err(e) => panic!("{}: {e}", c.id),
        };
        // the journal: equal to the dry run's and to the transformation's on the models
        // loaded on their own
        let mine = keyed_journal(&keys, &outcome.journal);
        if mine != keyed_journal(&keys, &dry.journal) || outcome.log != dry.log {
            diffs.push("the run differs from the dry run".into());
        }
        let theirs = standalone(&c, &t);
        if mine != theirs {
            diffs.push(format!("journal: {mine:#?}\n vs standalone {theirs:#?}"));
        }
        // the change path
        let want = expected(&c.id);
        if want != applied {
            diffs.push(format!("change path: {applied}, expected {want}"));
        }
        // the run goes on (or has failed)
        let end = sim.finish();
        match (&applied[..], &end) {
            ("abort", Err(_)) => {}
            ("abort", Ok(_)) => diffs.push("an aborting change set left the run going".into()),
            (_, Ok(res)) => assert!(res.events > events_before),
            (_, Err(e)) => diffs.push(format!("the run failed: {e}")),
        }
        report.push(format!(
            "{:45} {} journal {:3} change {:12} java-dump {} run {:>6} µs",
            c.id,
            if diffs.is_empty() { "equal" } else { "DIFFERS" },
            outcome.journal.changes.len(),
            applied,
            if unstable.contains(&"dump") {
                "unstable"
            } else {
                "compared"
            },
            outcome.run_micros
        ));
        if !diffs.is_empty() {
            failures.push(format!("{}: {}", c.id, diffs.join("\n  ")));
        }
    }
    eprintln!("{}", report.join("\n"));
    assert_eq!(report.len(), 24);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

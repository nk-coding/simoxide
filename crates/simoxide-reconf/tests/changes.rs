//! Runtime model changes applied at pause points through the QVTo adapter's store (direct
//! edits with a journal): containers and allocations added, rates, latencies and demands
//! changed, containers removed; SimuLizar's syncer semantics (compat); forks and determinism.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use simoxide_model::meta::feat as mf;
use simoxide_reconf::{Edit, Session};
use simoxide_sim::{
    ChangeError, ChangeReport, CompiledModel, Fast, Mode, Outputs, Pause, RunResult, RunSpec,
    SimConfig, Simulation,
};

const S: i64 = 1_000_000_000;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(dir: &Path) -> (CompiledModel, SimConfig) {
    let spec = RunSpec::load(&dir.join("run.json")).expect("run.json");
    let model = spec.load_model(dir).expect("model");
    let cm = CompiledModel::compile(model).expect("compile");
    (cm, spec.sim_config("t"))
}

fn corpus(name: &str) -> (CompiledModel, SimConfig) {
    load(&root().join("corpus").join(name))
}

/// A copy of a corpus model with `edit` applied to the text of the file with extension `ext`.
fn variant(name: &str, ext: &str, edit: impl Fn(&str) -> String) -> (CompiledModel, SimConfig) {
    let src = root().join("corpus").join(name);
    let dst = std::env::temp_dir().join(format!(
        "simoxide-reconf-{}-{name}-{ext}-{}",
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
    let r = load(&dst);
    let _ = std::fs::remove_dir_all(&dst);
    r
}

/// Replaces the first `from` after `anchor`.
fn replace_after(t: &str, anchor: &str, from: &str, to: &str) -> String {
    let a = t.find(anchor).expect("anchor");
    let i = a + t[a..].find(from).expect("text after anchor");
    format!("{}{}{}", &t[..i], to, &t[i + from.len()..])
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

#[derive(Default, Clone)]
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
    fn fast_outputs(&self) -> Outputs {
        Outputs {
            trace: Some(Box::new(self.trace.clone())),
            tape: None,
        }
    }
}

/// Result, trace and tape of a run.
#[derive(PartialEq)]
struct Out {
    result: String,
    trace: String,
    tape: String,
}

impl std::fmt::Debug for Out {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}", self.result.lines().next().unwrap_or(""))
    }
}

fn fingerprint(r: &Result<RunResult, simoxide_sim::SimError>) -> String {
    match r {
        Ok(r) => format!(
            "end {} main {} draws {} events {} tuples {} warnings {:?}\n{}",
            r.end_ns,
            r.main_count,
            r.uniforms,
            r.events,
            r.measurements.count,
            r.warnings,
            r.measurements.to_csv()
        ),
        Err(e) => format!("error {:?} at {}: {}", e.kind, e.at_ns, e.message),
    }
}

fn first_difference(a: &str, b: &str) -> String {
    for (i, (x, y)) in a.lines().zip(b.lines()).enumerate() {
        if x != y {
            return format!("line {}:\n  want {x}\n  got  {y}", i + 1);
        }
    }
    format!("lengths {} vs {}", a.lines().count(), b.lines().count())
}

fn assert_same(what: &str, want: &Out, got: &Out) {
    for (part, a, b) in [
        ("result", &want.result, &got.result),
        ("trace", &want.trace, &got.trace),
        ("tape", &want.tape, &got.tape),
    ] {
        assert!(a == b, "{what}: {part} differs: {}", first_difference(a, b));
    }
}

fn run(cm: &CompiledModel, cfg: &SimConfig) -> Out {
    let w = W::default();
    let sim = Simulation::new(cm, cfg.clone(), w.outputs()).unwrap();
    let r = fingerprint(&sim.finish());
    Out {
        result: r,
        trace: w.trace.take(),
        tape: w.tape.take(),
    }
}

/// An edit of a run.
type EditFn = dyn Fn(&mut Edit<'_>);

/// Runs to `pause`, applies `edit` (unless `None`), finishes: the output and the report.
fn run_with(
    cm: &CompiledModel,
    cfg: &SimConfig,
    pause: Pause,
    edit: Option<&EditFn>,
) -> (Out, Option<Result<ChangeReport, ChangeError>>) {
    let w = W::default();
    let mut sim = Simulation::new(cm, cfg.clone(), w.outputs()).unwrap();
    sim.run_until(pause).unwrap();
    let report = edit.map(|f| {
        let mut session = Session::new();
        let mut e = Edit::new(&sim, &mut session);
        f(&mut e);
        e.apply(&mut sim)
    });
    let r = fingerprint(&sim.finish());
    (
        Out {
            result: r,
            trace: w.trace.take(),
            tape: w.tape.take(),
        },
        report,
    )
}

const H19: &str = "h19_linking_resource";

/// Demand lines of the trace: (time, container, demand, concrete demand).
fn demands(trace: &str) -> Vec<(f64, String, f64, f64)> {
    trace
        .lines()
        .filter(|l| l.starts_with("{\"ev\":\"demand\""))
        .map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).unwrap();
            (
                v["t"].as_f64().unwrap(),
                v["rc"].as_str().unwrap().to_string(),
                v["d"].as_f64().unwrap(),
                v["st"].as_f64().unwrap(),
            )
        })
        .collect()
}

/// h19: a copy of the server's container, connected to the LAN, and the server allocated to it.
fn scale_out(e: &mut Edit<'_>) {
    let server = e.find("_h19_linking_resource_rc2").unwrap();
    let copy = e.clone_container(server, "_rc3", "ServerNode2").unwrap();
    for link in e.links_of(server) {
        e.connect(link, copy).unwrap();
    }
    let ac = e.find("_h19_linking_resource_ac2").unwrap();
    e.allocate(ac, copy, "_alc3").unwrap();
}

#[test]
fn added_container_and_allocation_receive_the_load() {
    let (cm, cfg) = corpus(H19);
    let cfg = SimConfig {
        max_measurements: 400,
        ..cfg
    };
    let t = 20 * S;
    let (out, report) = run_with(&cm, &cfg, Pause::After(t), Some(&scale_out));
    let report = report.unwrap().unwrap();
    assert_eq!(report.generation, 1);
    assert_eq!(report.new_resources.len(), 1, "{report:?}");
    assert!(
        report
            .applied
            .iter()
            .any(|a| a.contains("new resource container _rc3")),
        "{report:?}"
    );
    assert!(out.result.starts_with("end "), "{}", out.result);
    let d = demands(&out.trace);
    let after = |rc: &str| d.iter().filter(|x| x.0 > 20.0 && x.1 == rc).count();
    let before = |rc: &str| d.iter().filter(|x| x.0 <= 20.0 && x.1 == rc).count();
    assert!(before("_h19_linking_resource_rc2") > 10);
    assert_eq!(before("_rc3"), 0);
    assert_eq!(after("_h19_linking_resource_rc2"), 0, "the server moved");
    assert!(
        after("_rc3") > 10,
        "the new container gets the server's demands"
    );
    assert!(
        after("_h19_linking_resource_link1") > 10,
        "routed over the LAN"
    );
    assert!(after("_h19_linking_resource_rc1") > 10, "the client stays");
}

#[test]
fn rate_change_before_the_start_equals_the_changed_model() {
    let (cm, cfg) = corpus(H19);
    let set = |e: &mut Edit<'_>| {
        let prs = e.find("_h19_linking_resource_prs2").unwrap();
        e.set_rate(prs, "2.5").unwrap();
    };
    let (got, report) = run_with(&cm, &cfg, Pause::Events(0), Some(&set));
    let report = report.unwrap().unwrap();
    assert_eq!(report.applied.len(), 1, "{report:?}");
    let (cm2, cfg2) = variant(H19, "resourceenvironment", |t| {
        replace_after(
            t,
            "_h19_linking_resource_prs2",
            "specification=\"1.0\"",
            "specification=\"2.5\"",
        )
    });
    assert_same("rate 2.5 from the start", &run(&cm2, &cfg2), &got);
}

#[test]
fn rate_change_mid_run_applies_to_later_demands() {
    let (cm, cfg) = corpus(H19);
    let cfg = SimConfig {
        max_measurements: 300,
        ..cfg
    };
    let set = |e: &mut Edit<'_>| {
        let prs = e.find("_h19_linking_resource_prs2").unwrap();
        e.set_rate(prs, "4.0").unwrap();
    };
    let (out, report) = run_with(&cm, &cfg, Pause::After(15 * S), Some(&set));
    assert!(report.unwrap().is_ok());
    let d = demands(&out.trace);
    let server: Vec<_> = d
        .iter()
        .filter(|x| x.1 == "_h19_linking_resource_rc2")
        .collect();
    assert!(server.iter().any(|x| x.0 > 15.0));
    for x in server {
        let rate = if x.0 > 15.0 { 4.0 } else { 1.0 };
        assert_eq!(x.3, x.2 / rate, "{x:?}");
    }
}

#[test]
fn link_latency_and_seff_demand_changes_before_the_start_equal_the_changed_model() {
    let (cm, cfg) = corpus(H19);
    let lat = |e: &mut Edit<'_>| {
        let spec = e.find("_h19_linking_resource_clrs1").unwrap();
        e.set_spec(
            spec,
            mf::CommunicationLinkResourceSpecification_latency_CommunicationLinkResourceSpecification,
            "0.05",
        )
        .unwrap();
    };
    let (got, report) = run_with(&cm, &cfg, Pause::Events(0), Some(&lat));
    assert!(report.unwrap().unwrap().applied[0].contains("latency"));
    let (cm2, cfg2) = variant(H19, "resourceenvironment", |t| {
        t.replace("specification=\"0.005\"", "specification=\"0.05\"")
    });
    assert_same("latency 0.05 from the start", &run(&cm2, &cfg2), &got);

    let demand = |e: &mut Edit<'_>| {
        let g = e.graph();
        // the demand of the server's internal action
        let act = e.find("_h19_linking_resource_act5").unwrap();
        let d = g.get_refs(
            act,
            mf::AbstractInternalControlFlowAction_resourceDemand_Action,
        )[0];
        e.set_spec(
            d,
            mf::ParametricResourceDemand_specification_ParametericResourceDemand,
            "data.BYTESIZE * 0.00002",
        )
        .unwrap();
    };
    let (got, report) = run_with(&cm, &cfg, Pause::Events(0), Some(&demand));
    assert!(report.unwrap().unwrap().code_appended);
    let (cm2, cfg2) = variant(H19, "repository", |t| {
        t.replace("data.BYTESIZE * 0.00001", "data.BYTESIZE * 0.00002")
    });
    assert_same(
        "server demand doubled from the start",
        &run(&cm2, &cfg2),
        &got,
    );
}

#[test]
fn seff_demand_change_mid_run_applies_to_new_executions() {
    let (cm, cfg) = corpus(H19);
    let demand = |e: &mut Edit<'_>| {
        let act = e.find("_h19_linking_resource_act5").unwrap();
        let d = e.graph().get_refs(
            act,
            mf::AbstractInternalControlFlowAction_resourceDemand_Action,
        )[0];
        e.set_spec(
            d,
            mf::ParametricResourceDemand_specification_ParametericResourceDemand,
            "data.BYTESIZE * 0.0001",
        )
        .unwrap();
    };
    let (out, report) = run_with(&cm, &cfg, Pause::After(10 * S), Some(&demand));
    assert!(report.unwrap().unwrap().code_appended);
    for x in demands(&out.trace)
        .iter()
        .filter(|x| x.1 == "_h19_linking_resource_rc2")
    {
        let k = if x.0 > 10.0 { 0.0001 } else { 0.00001 };
        assert!(x.2 == 1000.0 * k || x.2 == 5000.0 * k, "{x:?}");
    }
}

#[test]
fn removed_container_keeps_simulating() {
    let (cm, cfg) = corpus(H19);
    let cfg = SimConfig {
        max_measurements: 300,
        ..cfg
    };
    let want = run(&cm, &cfg);
    let remove = |e: &mut Edit<'_>| {
        let server = e.find("_h19_linking_resource_rc2").unwrap();
        e.remove_from_environment(server).unwrap();
    };
    let (got, report) = run_with(&cm, &cfg, Pause::After(12 * S), Some(&remove));
    let report = report.unwrap().unwrap();
    assert!(!report.ignored.is_empty(), "{report:?}");
    assert!(report.new_resources.is_empty());
    // SimuLizar's syncers ignore the removal: the run is the uninterrupted one
    assert_same("container removed", &want, &got);
    // deleting it unsets the allocation context's container: SimuLizar's syncer aborts
    let delete = |e: &mut Edit<'_>| {
        let server = e.find("_h19_linking_resource_rc2").unwrap();
        e.delete(server);
    };
    let (got, report) = run_with(&cm, &cfg, Pause::After(12 * S), Some(&delete));
    assert!(
        matches!(report, Some(Err(ChangeError::Abort(_)))),
        "{report:?}"
    );
    assert_same("refused change", &want, &got);
}

#[test]
fn removed_allocation_context_aborts_and_leaves_the_run_unchanged() {
    let (cm, cfg) = corpus(H19);
    let want = run(&cm, &cfg);
    let remove = |e: &mut Edit<'_>| {
        let a = e.find("_h19_linking_resource_alc2").unwrap();
        e.delete(a);
    };
    let (got, report) = run_with(&cm, &cfg, Pause::After(3 * S), Some(&remove));
    assert!(
        matches!(report, Some(Err(ChangeError::Abort(_)))),
        "{report:?}"
    );
    assert_same("refused change", &want, &got);
}

#[test]
fn population_change_is_not_supported_yet() {
    let (cm, cfg) = corpus("h06_closed_think");
    let want = run(&cm, &cfg);
    let set = |e: &mut Edit<'_>| {
        let g = e.graph();
        let w = (0..g.objs.len() as u32)
            .map(simoxide_model::raw::ObjId)
            .find(|&o| g[o].class == simoxide_model::meta::class::ClosedWorkload)
            .unwrap();
        use qvto_model::ModelStore;
        e.store()
            .set(
                w.0,
                qvto_model::FeatureId(mf::ClosedWorkload_population.0),
                qvto_model::Datum::Int(8),
            )
            .unwrap();
    };
    let (got, report) = run_with(&cm, &cfg, Pause::After(2 * S), Some(&set));
    assert!(
        matches!(report, Some(Err(ChangeError::Unsupported(_)))),
        "{report:?}"
    );
    assert_same("refused change", &want, &got);
}

#[test]
fn fork_with_a_change_equals_the_change_then_continue() {
    let (cm, cfg) = corpus(H19);
    let cfg = SimConfig {
        max_measurements: 300,
        ..cfg
    };
    let p = Pause::After(18 * S);
    let (want, _) = run_with(&cm, &cfg, p, Some(&scale_out));
    // pause, fork, change the fork
    let w = W::default();
    let mut sim = Simulation::new(&cm, cfg.clone(), w.outputs()).unwrap();
    sim.run_until(p).unwrap();
    let wf = W::default();
    let mut fork = sim.fork(wf.outputs()).unwrap();
    let prefix = (w.trace.take(), w.tape.take());
    let mut session = Session::new();
    let mut e = Edit::new(&fork, &mut session);
    scale_out(&mut e);
    e.apply(&mut fork).unwrap();
    let got = Out {
        result: fingerprint(&fork.finish()),
        trace: prefix.0.clone() + &wf.trace.take(),
        tape: prefix.1.clone() + &wf.tape.take(),
    };
    assert_same("fork with the change", &want, &got);
    // the origin goes on unchanged
    let origin = Out {
        result: fingerprint(&sim.finish()),
        trace: prefix.0 + &w.trace.take(),
        tape: prefix.1 + &w.tape.take(),
    };
    assert_same("origin", &run(&cm, &cfg), &origin);
}

#[test]
fn fork_resume_and_detach_after_changes_equal_the_continuation() {
    let (cm, cfg) = corpus(H19);
    let cfg = SimConfig {
        max_measurements: 500,
        ..cfg
    };
    // two generations: scale out, then the new container gets a faster CPU
    let edits = |sim: &mut Simulation<'_>, k: u32| {
        let mut session = Session::new();
        let mut e = Edit::new(sim, &mut session);
        if k == 0 {
            scale_out(&mut e);
        } else {
            let p = e.find("_rc3_0").unwrap();
            e.set_rate(p, "3.0").unwrap();
        }
        e.apply(sim).unwrap()
    };
    let w = W::default();
    let mut sim = Simulation::new(&cm, cfg.clone(), w.outputs()).unwrap();
    sim.run_until(Pause::After(10 * S)).unwrap();
    edits(&mut sim, 0);
    sim.run_until(Pause::After(20 * S)).unwrap();
    let r = edits(&mut sim, 1);
    assert_eq!(r.generation, 2);
    assert!(r.applied.iter().any(|a| a.contains("rate")), "{r:?}");
    sim.run_until(Pause::After(25 * S)).unwrap();
    let wf = W::default();
    let fork = sim.fork(wf.outputs()).unwrap();
    let prefix = (w.trace.take(), w.tape.take());
    // the origin: taken out and resumed on the base model, then deep-copied to a thread
    let st = sim.into_state();
    let wr = W::default();
    let mut sim = Simulation::resume(&cm, st, wr.outputs()).unwrap();
    sim.run_until(Pause::After(30 * S)).unwrap();
    let st = sim.into_state();
    let mid = (wr.trace.take(), wr.tape.take());
    let d = st.detach();
    drop(st);
    let (r, t, u) = std::thread::scope(|sc| {
        sc.spawn(|| {
            let w = W::default();
            let sim = Simulation::resume(&cm, d.into_state(), w.outputs()).unwrap();
            (fingerprint(&sim.finish()), w.trace.take(), w.tape.take())
        })
        .join()
        .unwrap()
    });
    let origin = Out {
        result: r,
        trace: prefix.0.clone() + &mid.0 + &t,
        tape: prefix.1.clone() + &mid.1 + &u,
    };
    let forked = Out {
        result: fingerprint(&fork.finish()),
        trace: prefix.0 + &wf.trace.take(),
        tape: prefix.1 + &wf.tape.take(),
    };
    assert_same("fork after changes", &origin, &forked);
    assert!(
        demands(&origin.trace)
            .iter()
            .any(|x| x.1 == "_rc3" && x.0 > 20.0 && x.3 == x.2 / 3.0)
    );
}

#[test]
fn changes_are_deterministic_also_in_fast_mode() {
    let (cm, cfg) = corpus(H19);
    let cfg = SimConfig {
        max_measurements: 300,
        ..cfg
    };
    let a = run_with(&cm, &cfg, Pause::After(7 * S), Some(&scale_out)).0;
    let b = run_with(&cm, &cfg, Pause::After(7 * S), Some(&scale_out)).0;
    assert_same("same change twice", &a, &b);
    let fast = SimConfig {
        mode: Mode::Fast,
        ..cfg
    };
    let go = || {
        let w = W::default();
        let mut sim = Simulation::<Fast>::create(&cm, fast.clone(), w.fast_outputs()).unwrap();
        sim.run_until(Pause::After(7 * S)).unwrap();
        let mut session = Session::new();
        let mut e = Edit::new(&sim, &mut session);
        scale_out(&mut e);
        e.apply(&mut sim).unwrap();
        (fingerprint(&sim.finish()), w.trace.take())
    };
    let (x, y) = (go(), go());
    assert_eq!(x, y);
    assert!(demands(&x.1).iter().any(|d| d.1 == "_rc3"));
}

#[test]
fn successive_change_sets_add_change_and_remove_a_container() {
    let (cm, cfg) = corpus(H19);
    let cfg = SimConfig {
        max_measurements: 500,
        ..cfg
    };
    let w = W::default();
    let mut sim = Simulation::new(&cm, cfg, w.outputs()).unwrap();
    sim.run_until(Pause::After(5 * S)).unwrap();
    let mut session = Session::new();
    let mut e = Edit::new(&sim, &mut session);
    scale_out(&mut e);
    assert_eq!(e.apply(&mut sim).unwrap().generation, 1);
    sim.run_until(Pause::After(10 * S)).unwrap();
    let mut e = Edit::new(&sim, &mut session);
    let p = e.find("_rc3_0").unwrap();
    e.set_rate(p, "0.5").unwrap();
    assert_eq!(e.apply(&mut sim).unwrap().generation, 2);
    sim.run_until(Pause::After(15 * S)).unwrap();
    let mut e = Edit::new(&sim, &mut session);
    let c = e.find("_rc3").unwrap();
    e.remove_from_environment(c).unwrap();
    let r = e.apply(&mut sim).unwrap();
    assert_eq!(r.generation, 3);
    assert!(!r.ignored.is_empty());
    let res = sim.finish().unwrap();
    assert!(res.main_count >= 500);
    let d = demands(&w.trace.take());
    // the removed container keeps its allocation lookups (SimuLizar): it still serves
    assert!(
        d.iter()
            .any(|x| x.1 == "_rc3" && x.0 > 15.0 && x.3 == x.2 / 0.5)
    );
}

#[test]
fn a_new_link_gives_a_route() {
    // a container without a link to the client: no route until a link connects them
    let (cm, cfg) = corpus(H19);
    let cfg = SimConfig {
        max_measurements: 200,
        ..cfg
    };
    let w = W::default();
    let mut sim = Simulation::new(&cm, cfg, w.outputs()).unwrap();
    sim.run_until(Pause::After(5 * S)).unwrap();
    let mut session = Session::new();
    let mut e = Edit::new(&sim, &mut session);
    let server = e.find("_h19_linking_resource_rc2").unwrap();
    let copy = e.clone_container(server, "_rc3", "ServerNode2").unwrap();
    let ac = e.find("_h19_linking_resource_ac2").unwrap();
    e.allocate(ac, copy, "_alc3").unwrap();
    e.apply(&mut sim).unwrap();
    let mut probe = sim.fork(Outputs::default()).unwrap();
    let err = probe.run_until(Pause::After(20 * S)).unwrap_err();
    assert!(err.message.contains("Could not determine route"), "{err}");
    // now connect it
    let mut e = Edit::new(&sim, &mut session);
    let link = e.find("_h19_linking_resource_link1").unwrap();
    let c = e.find("_rc3").unwrap();
    e.connect(link, c).unwrap();
    let r = e.apply(&mut sim).unwrap();
    assert!(r.applied.iter().any(|a| a.contains("connected")), "{r:?}");
    sim.finish().unwrap();
}

#[test]
fn branch_probabilities_before_the_start_equal_the_changed_model() {
    let name = "h07_prob_branch";
    let (cm, cfg) = corpus(name);
    let set = |e: &mut Edit<'_>| {
        for (id, p) in [("_h07_prob_branch_bt1", 0.6), ("_h07_prob_branch_bt2", 0.1)] {
            let t = e.find(id).unwrap();
            e.set_double(t, mf::ProbabilisticBranchTransition_branchProbability, p)
                .unwrap();
        }
    };
    let (got, report) = run_with(&cm, &cfg, Pause::Events(0), Some(&set));
    // an action changed, the instructions did not
    assert!(!report.unwrap().unwrap().code_appended);
    let (cm2, cfg2) = variant(name, "repository", |t| {
        t.replace("branchProbability=\"0.2\"", "branchProbability=\"0.6\"")
            .replace("branchProbability=\"0.5\"", "branchProbability=\"0.1\"")
    });
    assert_same(
        "branch probabilities from the start",
        &run(&cm2, &cfg2),
        &got,
    );
}

#[test]
fn composite_allocated_to_a_new_container_moves_its_inner_assemblies() {
    let (cm, cfg) = corpus("h15_composite");
    let cfg = SimConfig {
        max_measurements: 300,
        ..cfg
    };
    let edit = |e: &mut Edit<'_>| {
        let node = e.find("_h15_composite_rc1").unwrap();
        let copy = e.clone_container(node, "_rc2", "Node2").unwrap();
        let ac = e.find("_h15_composite_ac3").unwrap();
        e.allocate(ac, copy, "_alc2").unwrap();
    };
    let (out, report) = run_with(&cm, &cfg, Pause::After(10 * S), Some(&edit));
    let report = report.unwrap().unwrap();
    assert!(
        report
            .applied
            .iter()
            .any(|a| a.contains("allocated to _rc2")),
        "{report:?}"
    );
    let d = demands(&out.trace);
    assert!(d.iter().filter(|x| x.1 == "_rc2").count() > 10);
    assert_eq!(
        d.iter()
            .filter(|x| x.0 > 10.0 && x.1 == "_h15_composite_rc1")
            .count(),
        0
    );
}

#[test]
fn rewired_connector_calls_a_new_assembly_context() {
    let (cm, cfg) = corpus("h14_call_chain_3");
    let cfg = SimConfig {
        max_measurements: 300,
        ..cfg
    };
    let edit = |e: &mut Edit<'_>| {
        let c = e.find("_h14_call_chain_3_ac3").unwrap();
        let c2 = e.clone_assembly(c, "_ac4", "Assembly_C2").unwrap();
        let node = e.find("_h14_call_chain_3_rc3").unwrap();
        let copy = e.clone_container(node, "_rc4", "NodeC2").unwrap();
        for l in e.links_of(node) {
            e.connect(l, copy).unwrap();
        }
        e.allocate(c2, copy, "_alc4").unwrap();
        let conn = e.find("_h14_call_chain_3_conn2").unwrap();
        e.set_ref(
            conn,
            mf::AssemblyConnector_providingAssemblyContext_AssemblyConnector,
            c2,
        )
        .unwrap();
    };
    let (out, report) = run_with(&cm, &cfg, Pause::After(10 * S), Some(&edit));
    report.unwrap().unwrap();
    assert!(out.result.starts_with("end "), "{}", out.result);
    let d = demands(&out.trace);
    let after = |rc: &str| d.iter().filter(|x| x.0 > 10.0 && x.1 == rc).count();
    assert!(after("_rc4") > 10, "C2 serves the calls");
    assert_eq!(
        after("_h14_call_chain_3_rc3"),
        0,
        "C is not called any more"
    );
    assert!(after("_h14_call_chain_3_rc2") > 10, "B still runs");
}

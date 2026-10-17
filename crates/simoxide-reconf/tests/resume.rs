//! Resume with changes (`DESIGN-SPD.md` N2): a change applied to a restored snapshot equals the
//! same change applied in the uninterrupted run at the same point (F2's path), byte for byte
//! (result, measurements, trace, tape), in exact and fast mode: change files against the direct
//! `Edit` calls they stand for, a QVTo transformation (also two, with the QVTo state carried
//! through a snapshot between them), a replaced and an added rule set. Changes the change path
//! refuses leave the run unchanged; the Slingshot flavour refuses model changes (F4).
//! Exploration (`explore`) is deterministic, whatever the number of threads.

mod common;

use common::*;
use simoxide_model::meta::feat as mf;
use simoxide_model::raw::ObjId;
use simoxide_reconf::{
    ChangeFile, ChangeFileError, Edit, Imports, Invocation, QvtoRun, ResumeChanges, RuleSet,
    RulesHost, RulesRun, RunConfig, Transformation, Variant,
};
use simoxide_sim::snapshot::Snapshot;
use simoxide_sim::{
    ChangeError, Compat, CompiledModel, Exact, Fast, FlavourConfig, Mode, Outputs, Pause,
    RunResult, RunSpec, SimConfig, SimError, Simulation, Slingshot, Status,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
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

#[derive(Clone, Default)]
struct W {
    trace: Buf,
    tape: Buf,
}

impl W {
    fn outputs(&self, tape: bool) -> Outputs {
        Outputs {
            trace: Some(Box::new(self.trace.clone())),
            tape: tape.then(|| Box::new(self.tape.clone()) as Box<dyn std::io::Write>),
        }
    }
}

fn result(r: &Result<RunResult, SimError>) -> String {
    match r {
        Ok(r) => format!(
            "end {} main {} draws {} events {} tuples {}\n{}",
            r.end_ns,
            r.main_count,
            r.uniforms,
            r.events,
            r.measurements.count,
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

fn same(what: &str, want: &[String; 3], got: &[String; 3]) {
    for (i, part) in ["result", "trace", "tape"].iter().enumerate() {
        assert!(
            want[i] == got[i],
            "{what}: {part} differs: {}",
            first_difference(&want[i], &got[i])
        );
    }
}

fn load(dir: &str, f: impl FnOnce(&mut SimConfig)) -> (CompiledModel, SimConfig) {
    let dir = root().join(dir);
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let cm = CompiledModel::compile(spec.load_model(&dir).unwrap()).unwrap();
    let mut cfg = spec.sim_config(&dir.file_name().unwrap().to_string_lossy());
    f(&mut cfg);
    (cm, cfg)
}

fn snapshot_bytes<C: Compat>(sim: &mut Simulation<'_, C>) -> Vec<u8> {
    let mut b = Vec::new();
    sim.snapshot_to(&mut b).unwrap();
    b
}

type Direct = dyn Fn(&mut Edit);

fn find(e: &Edit, id: &str) -> ObjId {
    e.find(id).unwrap_or_else(|| panic!("no {id}"))
}

/// A change file applied after a restore at `pause` equals the direct edits applied in the
/// uninterrupted run at the same point (and the change takes effect).
fn change_at_resume<C: Compat>(
    what: &str,
    cm: &CompiledModel,
    cfg: &SimConfig,
    pause: Pause,
    file: &str,
    direct: &Direct,
    tape: bool,
) {
    let file = ChangeFile::parse(file).unwrap();
    // uninterrupted, with F2's edits
    let w = W::default();
    let mut sim = Simulation::<C>::create(cm, cfg.clone(), w.outputs(tape)).unwrap();
    assert_eq!(sim.run_until(pause).unwrap(), Status::Paused, "{what}");
    let mut e = Edit::new(&sim);
    direct(&mut e);
    let rep = e.apply(&mut sim).unwrap();
    assert_eq!(rep.generation, 1, "{what}");
    assert!(!rep.applied.is_empty(), "{what}: nothing applied");
    let want = [result(&sim.finish()), w.trace.text(), w.tape.text()];
    // paused, written, restored by a fresh simulation, the change file applied
    let w = W::default();
    let mut sim = Simulation::<C>::create(cm, cfg.clone(), w.outputs(tape)).unwrap();
    sim.run_until(pause).unwrap();
    let bytes = snapshot_bytes(&mut sim);
    let (t0, u0) = (w.trace.text(), w.tape.text());
    drop(sim);
    let w2 = W::default();
    let mut sim = Simulation::<C>::restore_from(&bytes[..], cm, w2.outputs(tape)).unwrap();
    let rep2 = file.apply(&mut sim).unwrap();
    assert_eq!(rep2.applied, rep.applied, "{what}");
    assert_eq!(rep2.ignored, rep.ignored, "{what}");
    let got = [
        result(&sim.finish()),
        t0 + &w2.trace.text(),
        u0 + &w2.tape.text(),
    ];
    same(what, &want, &got);
    // and the change mattered
    let mut plain = Simulation::<C>::create(cm, cfg.clone(), Outputs::default()).unwrap();
    plain.run_until(pause).unwrap();
    assert_ne!(
        result(&plain.finish()),
        want[0],
        "{what}: the change had no effect"
    );
}

fn workload(e: &Edit, scenario: &str) -> ObjId {
    let s = find(e, scenario);
    e.graph()
        .get_ref(s, mf::UsageScenario_workload_UsageScenario)
        .unwrap()
}

fn demand(e: &Edit, action: &str) -> ObjId {
    let a = find(e, action);
    e.graph().get_refs(
        a,
        mf::AbstractInternalControlFlowAction_resourceDemand_Action,
    )[0]
}

struct Case {
    dir: &'static str,
    pause: Pause,
    cfg: fn(&mut SimConfig),
    file: &'static str,
    direct: Box<Direct>,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            dir: "corpus/h19_linking_resource",
            pause: Pause::After(20 * S),
            cfg: |c| c.max_measurements = 500,
            file: "# a second server on the LAN, the server's assembly allocated to it\n\
                   add-container ServerNode _rc3 ServerNode2\n\
                   allocate _h19_linking_resource_ac2 _rc3 _alc3\n\
                   rate _rc3 2.0\n",
            direct: Box::new(|e| {
                let server = find(e, "_h19_linking_resource_rc2");
                let copy = e.clone_container(server, "_rc3", "ServerNode2").unwrap();
                for l in e.links_of(server) {
                    e.connect(l, copy).unwrap();
                }
                let ac = find(e, "_h19_linking_resource_ac2");
                e.allocate(ac, copy, "_alc3").unwrap();
                let p = find(e, "_rc3_0");
                e.set_rate(p, "2.0").unwrap();
            }),
        },
        Case {
            dir: "corpus/h07_prob_branch",
            pause: Pause::After(15 * S),
            cfg: |c| c.max_measurements = 400,
            file: "probability branch_0 0.5\n\
                   probability _h07_prob_branch_bt2 0.2\n\
                   demand _h07_prob_branch_act7:CPU 0.4\n\
                   rate ServerNode:CPU 2.0\n",
            direct: Box::new(|e| {
                let bt1 = find(e, "_h07_prob_branch_bt1");
                e.set_double(
                    bt1,
                    mf::ProbabilisticBranchTransition_branchProbability,
                    0.5,
                )
                .unwrap();
                let bt2 = find(e, "_h07_prob_branch_bt2");
                e.set_double(
                    bt2,
                    mf::ProbabilisticBranchTransition_branchProbability,
                    0.2,
                )
                .unwrap();
                let d = demand(e, "_h07_prob_branch_act7");
                e.set_spec(
                    d,
                    mf::ParametricResourceDemand_specification_ParametericResourceDemand,
                    "0.4",
                )
                .unwrap();
                let p = find(e, "_h07_prob_branch_prs1");
                e.set_rate(p, "2.0").unwrap();
            }),
        },
        Case {
            dir: "corpus/h06_closed_think",
            pause: Pause::After(10 * S),
            cfg: |c| c.max_measurements = 300,
            file: "think-time Scenario Exp(0.5)\n\
                   demand work Lognorm(-1.0, 0.5)\n",
            direct: Box::new(|e| {
                let w = workload(e, "_h06_closed_think_us1");
                e.set_spec(w, mf::ClosedWorkload_thinkTime_ClosedWorkload, "Exp(0.5)")
                    .unwrap();
                let d = demand(e, "_h06_closed_think_act1");
                e.set_spec(
                    d,
                    mf::ParametricResourceDemand_specification_ParametericResourceDemand,
                    "Lognorm(-1.0, 0.5)",
                )
                .unwrap();
            }),
        },
        Case {
            dir: "corpus/h09_loop",
            pause: Pause::After(5 * S),
            cfg: |c| c.max_measurements = 300,
            file: "loop-count _h09_loop_act4 IntPMF[(1;0.5)(4;0.5)]\n\
                   set _h09_loop_prs1 processingRate_ProcessingResourceSpecification 1.5\n",
            direct: Box::new(|e| {
                let l = find(e, "_h09_loop_act4");
                e.set_spec(
                    l,
                    mf::LoopAction_iterationCount_LoopAction,
                    "IntPMF[(1;0.5)(4;0.5)]",
                )
                .unwrap();
                let p = find(e, "_h09_loop_prs1");
                e.set_rate(p, "1.5").unwrap();
            }),
        },
        Case {
            dir: "corpus/x_pem_screencast_ms",
            pause: Pause::After(2000 * S),
            cfg: |c| {
                c.max_measurements = -1;
                c.max_sim_time = 8000;
            },
            file: "arrival-rate RealisticUsageScenario 0.08\n\
                   rate DatabaseServer:HDD 1200 * 1000 * 1000\n",
            direct: Box::new(|e| {
                let w = workload(e, "__hpRMHD6EeSA4fySuX9I2Q");
                e.set_spec(
                    w,
                    mf::OpenWorkload_interArrivalTime_OpenWorkload,
                    "Exp(0.08)",
                )
                .unwrap();
                let p = find(e, "_T3NBUHy_EeaHhOO9CmhLjQ");
                e.set_rate(p, "1200 * 1000 * 1000").unwrap();
            }),
        },
    ]
}

#[test]
fn a_change_file_at_resume_equals_the_edit_in_process() {
    for c in cases() {
        let (cm, cfg) = load(c.dir, c.cfg);
        for mode in [Mode::Exact, Mode::Fast] {
            let cfg = SimConfig {
                mode,
                ..cfg.clone()
            };
            let what = format!("{} {mode}", c.dir);
            match mode {
                Mode::Exact => {
                    change_at_resume::<Exact>(&what, &cm, &cfg, c.pause, c.file, &*c.direct, true)
                }
                Mode::Fast => {
                    change_at_resume::<Fast>(&what, &cm, &cfg, c.pause, c.file, &*c.direct, false)
                }
            }
        }
    }
}

/// Changes the change path refuses (F4: population; a removed allocation context aborts in
/// SimuLizar) and file errors leave the restored run unchanged: it continues like the
/// uninterrupted run.
#[test]
fn refused_changes_leave_the_run_unchanged() {
    let (cm, cfg) = load("corpus/h06_closed_think", |c| c.max_measurements = 200);
    let pause = Pause::After(10 * S);
    let sim = Simulation::<Exact>::create(&cm, cfg.clone(), Outputs::default()).unwrap();
    let want = result(&sim.finish());
    let mut sim = Simulation::<Exact>::create(&cm, cfg.clone(), Outputs::default()).unwrap();
    sim.run_until(pause).unwrap();
    let bytes = snapshot_bytes(&mut sim);
    let mut sim = Simulation::<Exact>::restore_from(&bytes[..], &cm, Outputs::default()).unwrap();
    for (file, check) in [
        (
            "population Scenario 8\n",
            &(|e: &ChangeFileError| {
                matches!(e, ChangeFileError::Change(ChangeError::Unsupported(v))
                    if v.iter().any(|m| m.contains("population")))
            }) as &dyn Fn(&ChangeFileError) -> bool,
        ),
        (
            "think-time Scenario Exp(0.5)\nrate NoSuchServer 2\n",
            &|e| matches!(e, ChangeFileError::Line(2, m) if m.contains("NoSuchServer")),
        ),
        ("delete _h06_closed_think_alc1\n", &|e| {
            matches!(e, ChangeFileError::Change(ChangeError::Abort(_)))
        }),
        (
            "inter-arrival Scenario Exp(1)\n",
            &|e| matches!(e, ChangeFileError::Line(1, m) if m.contains("not a open workload")),
        ),
    ] {
        let f = ChangeFile::parse(file).unwrap();
        let e = f.apply(&mut sim).unwrap_err();
        assert!(check(&e), "{file}: {e}");
    }
    assert_eq!(sim.state().model().generation(), 0);
    assert_eq!(result(&sim.finish()), want);
}

/// The Slingshot flavour takes no model changes yet (F4): the change file is refused and the
/// run goes on unchanged.
#[test]
fn the_slingshot_flavour_refuses_changes_at_resume() {
    let dir = root().join("corpus/x_ss_minimal");
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let cm = CompiledModel::compile_flavour(
        spec.load_model(&dir).unwrap(),
        FlavourConfig::slingshot(simoxide_sim::Quirks::NONE),
    )
    .unwrap();
    let cfg = SimConfig {
        mode: Mode::Fast,
        ..spec.sim_config("x_ss_minimal")
    };
    let want = result(
        &Simulation::<Fast, Slingshot>::create(&cm, cfg.clone(), Outputs::default())
            .unwrap()
            .finish(),
    );
    let mut sim =
        Simulation::<Fast, Slingshot>::create(&cm, cfg.clone(), Outputs::default()).unwrap();
    sim.run_until(Pause::After(10 * S)).unwrap();
    let mut bytes = Vec::new();
    sim.snapshot_to(&mut bytes).unwrap();
    let mut sim =
        Simulation::<Fast, Slingshot>::restore_from(&bytes[..], &cm, Outputs::default()).unwrap();
    let f = ChangeFile::parse("rate ResourceContainer1 2.0\n").unwrap();
    match f.apply(&mut sim) {
        Err(ChangeFileError::Change(ChangeError::Unsupported(v))) => {
            assert!(v[0].contains("Slingshot"), "{v:?}")
        }
        o => panic!("{o:?}"),
    }
    assert_eq!(result(&sim.finish()), want);
}

fn rule(name: &str) -> Transformation {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/rules")
        .join(name);
    core().compile_file(&path, &Imports::Folder).unwrap()
}

/// QVTo transformations at resumes: two hops, each restored by a fresh simulation and QVTo
/// state (with the snapshot's section `qvto`), each running transformations through
/// `ResumeChanges`, equal the uninterrupted run that invokes them at the same points (ids of new
/// objects included).
#[test]
fn transformations_at_resume_equal_the_run_in_process() {
    let (cm, cfg) = load("corpus/h19_linking_resource", |c| {
        c.max_sim_time = 200;
        c.max_measurements = -1;
    });
    let add = rule("add_container.qvto");
    let rate = rule("set/a_rate.qvto");
    let inv = Invocation::simulizar();
    let qrun = || QvtoRun::new(core(), RunConfig::default()).unwrap();
    for mode in [Mode::Exact, Mode::Fast] {
        let cfg = SimConfig {
            mode,
            ..cfg.clone()
        };
        macro_rules! go {
            ($c:ty, $tape:expr) => {{
                // uninterrupted
                let w = W::default();
                let mut sim = Simulation::<$c>::create(&cm, cfg.clone(), w.outputs($tape)).unwrap();
                let mut q = qrun();
                sim.run_until(Pause::After(100 * S)).unwrap();
                q.invoke(&mut sim, &rate, &inv).unwrap();
                q.invoke(&mut sim, &add, &inv).unwrap();
                sim.run_until(Pause::After(150 * S)).unwrap();
                let second = q.invoke(&mut sim, &add, &inv).unwrap();
                let want = [result(&sim.finish()), w.trace.text(), w.tape.text()];
                // two hops
                let w = W::default();
                let mut sim = Simulation::<$c>::create(&cm, cfg.clone(), w.outputs($tape)).unwrap();
                sim.run_until(Pause::After(100 * S)).unwrap();
                let mut bytes = Vec::new();
                sim.snapshot_to(&mut bytes).unwrap();
                let (mut trace, mut tp) = (w.trace.text(), w.tape.text());
                drop(sim);
                let s = Snapshot::read(&bytes[..]).unwrap();
                let w = W::default();
                let mut sim =
                    Simulation::<$c>::restore_snapshot(&s, &cm, w.outputs($tape)).unwrap();
                let mut q = qrun();
                q.restore_section(&sim, &s).unwrap();
                let steps = ResumeChanges::new()
                    .transform(rate.clone(), inv.clone())
                    .transform(add.clone(), inv.clone())
                    .apply(&mut sim, &mut q)
                    .unwrap();
                assert_eq!(steps.len(), 2);
                assert!(steps[1].log.contains("added2"), "{}", steps[1].log);
                sim.run_until(Pause::After(150 * S)).unwrap();
                let qsec = q
                    .snapshot_section(simoxide_sim::snapshot::Encoding::Postcard)
                    .unwrap();
                let s = sim
                    .snapshot_value(simoxide_sim::snapshot::Encoding::Postcard, vec![qsec])
                    .unwrap();
                trace.push_str(&w.trace.text());
                tp.push_str(&w.tape.text());
                drop((sim, q));
                let w = W::default();
                let mut sim =
                    Simulation::<$c>::restore_snapshot(&s, &cm, w.outputs($tape)).unwrap();
                let mut q = qrun();
                q.restore_section(&sim, &s).unwrap();
                let steps = ResumeChanges::new()
                    .transform(add.clone(), inv.clone())
                    .apply(&mut sim, &mut q)
                    .unwrap();
                assert_eq!(steps[0].log, second.log);
                let got = [
                    result(&sim.finish()),
                    trace + &w.trace.text(),
                    tp + &w.tape.text(),
                ];
                same(&format!("transformations {mode}"), &want, &got);
            }};
        }
        match mode {
            Mode::Exact => go!(Exact, true),
            Mode::Fast => go!(Fast, false),
        }
    }
}

fn rules_host(dir: &Path) -> RulesHost {
    let set = RuleSet::load_dir(core(), dir).unwrap();
    // QVTo's mode `Auto`: the ordered mode of the exact rules (R2)
    RulesHost::new(core(), set, RunConfig::default())
        .unwrap()
        .keep_records()
}

/// A folder with some of the `lb_scaleout` rules.
fn rules_subset(name: &str, files: &[&str]) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("n2-rules-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let src = root().join("corpus-rules/lb_scaleout/rules");
    for f in files {
        std::fs::copy(src.join(f), d.join(f)).unwrap();
    }
    d
}

fn records(h: &mut RulesHost) -> String {
    h.take_records()
        .iter()
        .map(|r| {
            let rules: Vec<String> = r
                .rules
                .iter()
                .map(|x| format!("{}:{}:{}", x.name, x.succeeded, x.changed))
                .collect();
            format!("{} {} {}\n", r.at_ns, r.succeeded, rules.join(","))
        })
        .collect()
}

/// A rule set replaced at a resume (`lb_scaleout`: `outsource` alone until 300 s, then with
/// `scaleOut`) and one added to a run without rules equal the uninterrupted runs that replace or
/// attach the rules at the same point.
#[test]
fn rule_sets_replaced_or_added_at_resume_equal_the_run_in_process() {
    let (cm, cfg) = load("corpus-rules/lb_scaleout", |_| {});
    let only = rules_subset("outsource", &["outsource.qvto"]);
    let both = root().join("corpus-rules/lb_scaleout/rules");
    let pause = Pause::After(300 * S);
    for mode in [Mode::Exact, Mode::Fast] {
        let cfg = SimConfig {
            mode,
            ..cfg.clone()
        };
        macro_rules! go {
            ($c:ty, $tape:expr) => {{
                // replaced, in process
                let w = W::default();
                let sim = Simulation::<$c>::create(&cm, cfg.clone(), w.outputs($tape)).unwrap();
                let mut run = RulesRun::new(sim, rules_host(&only));
                run.run_until(pause).unwrap();
                let new = RuleSet::load_dir(core(), &both).unwrap();
                run.host_mut().replace_rules(new);
                let (r, mut h) = run.finish().unwrap();
                let want = [result(&Ok(r)), w.trace.text(), w.tape.text()];
                let want_rec = records(&mut h);
                assert!(want_rec.contains("scaleOut:true:true"), "{want_rec}");
                // replaced at a resume
                let w = W::default();
                let sim = Simulation::<$c>::create(&cm, cfg.clone(), w.outputs($tape)).unwrap();
                let mut run = RulesRun::new(sim, rules_host(&only));
                run.run_until(pause).unwrap();
                // the records so far belong to this hop (the snapshot keeps none)
                let mut rec = records(run.host_mut());
                let mut bytes = Vec::new();
                run.snapshot_to(&mut bytes).unwrap();
                let (t0, u0) = (w.trace.text(), w.tape.text());
                drop(run);
                let s = Snapshot::read(&bytes[..]).unwrap();
                assert!(
                    RulesRun::<$c>::restore_snapshot(
                        &s,
                        &cm,
                        rules_host(&both),
                        Outputs::default()
                    )
                    .is_err()
                );
                let w2 = W::default();
                let run = RulesRun::<$c>::restore_snapshot_replacing(
                    &s,
                    &cm,
                    rules_host(&both),
                    w2.outputs($tape),
                )
                .unwrap();
                let (r, mut h) = run.finish().unwrap();
                rec.push_str(&records(&mut h));
                let got = [result(&Ok(r)), t0 + &w2.trace.text(), u0 + &w2.tape.text()];
                same(&format!("rules replaced {mode}"), &want, &got);
                assert_eq!(rec, want_rec);

                // added to a run without rules, in process
                let w = W::default();
                let mut sim = Simulation::<$c>::create(&cm, cfg.clone(), w.outputs($tape)).unwrap();
                sim.run_until(pause).unwrap();
                let (r, _) = RulesRun::new(sim, rules_host(&both)).finish().unwrap();
                let want = [result(&Ok(r)), w.trace.text(), w.tape.text()];
                // added at a resume
                let w = W::default();
                let mut sim = Simulation::<$c>::create(&cm, cfg.clone(), w.outputs($tape)).unwrap();
                sim.run_until(pause).unwrap();
                let mut bytes = Vec::new();
                sim.snapshot_to(&mut bytes).unwrap();
                let (t0, u0) = (w.trace.text(), w.tape.text());
                drop(sim);
                let w2 = W::default();
                let sim =
                    Simulation::<$c>::restore_from(&bytes[..], &cm, w2.outputs($tape)).unwrap();
                let (r, _) = RulesRun::new(sim, rules_host(&both)).finish().unwrap();
                let got = [result(&Ok(r)), t0 + &w2.trace.text(), u0 + &w2.tape.text()];
                same(&format!("rules added {mode}"), &want, &got);
            }};
        }
        match mode {
            Mode::Exact => go!(Exact, true),
            Mode::Fast => go!(Fast, false),
        }
    }
}

const EXPLORE_VARIANTS: &[(&str, &str)] = &[
    ("base", ""),
    ("fast-db", "rate DatabaseServer:HDD 1200 * 1000 * 1000\n"),
    ("more-load", "arrival-rate RealisticUsageScenario 0.08\n"),
    (
        "new-db",
        "add-container DatabaseServer _db2 DatabaseServer2\n\
         rate _db2:HDD 2000 * 1000 * 1000\n\
         allocate FileStorage _db2 _alloc_fs2\n",
    ),
    // SimuLizar aborts on a removed allocation context (MC-3.3)
    ("bad", "delete _sj2LoLYiEeaM_9DBke4xEw\n"),
];

/// Fork-based exploration of a MediaStore snapshot: the same results on 1 and 3 threads and
/// twice in a row; each variant equals restoring the snapshot, applying its changes and running
/// it; the variant the change path refuses reports its error.
#[test]
fn exploration_is_deterministic() {
    let (cm, cfg) = load("corpus/x_pem_screencast_ms", |c| {
        c.max_measurements = -1;
        c.max_sim_time = 12000;
        c.mode = Mode::Fast;
    });
    let mut sim = Simulation::<Fast>::create(&cm, cfg, Outputs::default()).unwrap();
    sim.run_until(Pause::After(3000 * S)).unwrap();
    let mut bytes = Vec::new();
    sim.snapshot_to(&mut bytes).unwrap();
    let snap = Snapshot::read(&bytes[..]).unwrap();
    let variants: Vec<Variant> = EXPLORE_VARIANTS
        .iter()
        .map(|(n, f)| Variant {
            name: n.to_string(),
            changes: ResumeChanges::new().file(ChangeFile::parse(f).unwrap()),
        })
        .collect();
    let show = |r: &[simoxide_reconf::VariantResult]| -> String {
        r.iter()
            .map(|v| match &v.outcome {
                Ok(run) => format!(
                    "{} {} {} {} {:?}\n",
                    v.name,
                    run.end_ns,
                    run.events,
                    run.main_count,
                    run.summaries
                        .iter()
                        .map(|s| (
                            s.measuring_point.clone(),
                            s.metric,
                            s.count,
                            s.mean.to_bits()
                        ))
                        .collect::<Vec<_>>()
                ),
                Err(e) => format!("{} error {e}\n", v.name),
            })
            .collect()
    };
    let one = simoxide_reconf::explore::<Fast, simoxide_sim::SimuLizar>(
        &cm,
        &snap,
        core(),
        &variants,
        None,
        1,
    );
    let three = simoxide_reconf::explore::<Fast, simoxide_sim::SimuLizar>(
        &cm,
        &snap,
        core(),
        &variants,
        None,
        3,
    );
    let again = simoxide_reconf::explore::<Fast, simoxide_sim::SimuLizar>(
        &cm,
        &snap,
        core(),
        &variants,
        None,
        3,
    );
    assert_eq!(show(&one), show(&three));
    assert_eq!(show(&one), show(&again));
    let table = simoxide_reconf::compare(&one, &[]);
    assert_eq!(table, simoxide_reconf::compare(&three, &[]));
    // the variants differ, the refused one failed
    let ok: Vec<&simoxide_reconf::VariantRun> =
        one.iter().filter_map(|v| v.outcome.as_ref().ok()).collect();
    assert_eq!(ok.len(), 4, "{table}");
    for i in 1..4 {
        assert_ne!(
            format!("{:?}", ok[0].summaries),
            format!("{:?}", ok[i].summaries),
            "{} equals base",
            one[i].name
        );
    }
    assert!(
        matches!(&one[4].outcome, Err(e) if e.contains("aborts")),
        "{:?}",
        one[4].outcome
    );
    // a variant by hand
    let mut sim = Simulation::<Fast>::restore_snapshot(&snap, &cm, Outputs::default()).unwrap();
    let skip: Vec<usize> = sim
        .state()
        .measurements()
        .rows
        .iter()
        .map(Vec::len)
        .collect();
    ChangeFile::parse(EXPLORE_VARIANTS[3].1)
        .unwrap()
        .apply(&mut sim)
        .unwrap();
    let mut r = sim.finish().unwrap();
    for (rows, k) in r.measurements.rows.iter_mut().zip(skip) {
        rows.drain(..k);
    }
    assert_eq!(
        format!("{:?}", r.measurements.summaries()),
        format!("{:?}", ok[3].summaries)
    );
    assert_eq!(r.events, ok[3].events);
}

/// `RulesRun::run_while` with a pause condition: a time condition pauses where `run_until`
/// pauses (the same snapshot); a response-time trigger pauses after the rules of its event ran;
/// the continuation equals the uninterrupted run.
#[test]
fn pause_conditions_in_a_rules_run() {
    use simoxide_sim::{Condition, Watch};
    let (cm, cfg) = load("corpus-rules/lb_scaleout", |_| {});
    let rules = root().join("corpus-rules/lb_scaleout/rules");
    let cfg = SimConfig {
        mode: Mode::Fast,
        ..cfg
    };
    let new = |cfg: &SimConfig| {
        RulesRun::new(
            Simulation::<Fast>::create(&cm, cfg.clone(), Outputs::default()).unwrap(),
            rules_host(&rules),
        )
    };
    let (want, mut wh) = new(&cfg).finish().unwrap();
    let mut a = new(&cfg);
    a.run_until(Pause::After(250 * S)).unwrap();
    let mut b = new(&cfg);
    let mut w = Watch::new(vec![Condition::At(Pause::After(250 * S))]);
    w.arm(b.sim()).unwrap();
    b.run_while(|s| w.keep_going(s)).unwrap();
    let (mut x, mut y) = (Vec::new(), Vec::new());
    a.snapshot_to(&mut x).unwrap();
    b.snapshot_to(&mut y).unwrap();
    // the engine's state (the host's section holds wall times)
    let engine = |b: &[u8]| {
        Snapshot::read(b)
            .unwrap()
            .section("engine")
            .unwrap()
            .data
            .clone()
    };
    assert!(engine(&x) == engine(&y));
    let mut r = new(&cfg);
    let mut w = Watch::new(vec![
        simoxide_sim::sim::watch::parse_condition("rt[UsageScenarioMeasuringPoint] > 2").unwrap(),
    ]);
    w.arm(r.sim()).unwrap();
    assert_eq!(r.run_while(|s| w.keep_going(s)).unwrap(), Status::Paused);
    assert!(w.fired().is_some());
    let (got, mut h) = r.finish().unwrap();
    assert_eq!(result(&Ok(got)), result(&Ok(want)));
    assert_eq!(records(&mut h), records(&mut wh));
}

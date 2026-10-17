//! Snapshots of runs whose model changes (`DESIGN-SPD.md` N1): after direct edits (F2), with a
//! run's QVTo state after a rule ran (the id sequence continues), and with SimuLizar's
//! reconfiguration rules (R1 and R2: the eleven `corpus-rules/` examples, written and restored into
//! a fresh run and host several times, byte for byte equal to the reference in the exact build and
//! to the uninterrupted run in fast mode; the R2 examples create objects without ids, so the
//! restored id generator must continue the sequence).

mod common;

use common::*;
use qvto_emf::ChangeKind;
use simoxide_model::meta::feat as mf;
use simoxide_reconf::{
    Edit, Imports, Invocation, QvtMode, QvtoRun, RuleSet, RulesHost, RulesRun, RunConfig,
    Transformation,
};
use simoxide_sim::snapshot::{Encoding, Snapshot, SnapshotOptions};
use simoxide_sim::{
    Compat, CompiledModel, Exact, Fast, Mode, Outputs, Pause, RunResult, RunSpec, SimConfig,
    SimError, Simulation, Status,
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

fn h19() -> (CompiledModel, SimConfig) {
    let dir = root().join("corpus/h19_linking_resource");
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let cm = CompiledModel::compile(spec.load_model(&dir).unwrap()).unwrap();
    let cfg = SimConfig {
        max_measurements: 500,
        ..spec.sim_config("h19_linking_resource")
    };
    (cm, cfg)
}

/// Writes a snapshot of `sim` and restores it into a fresh simulation with new writers.
fn cycle<'m, C: Compat>(
    sim: &mut Simulation<'m, C>,
    cm: &'m CompiledModel,
    tape: bool,
) -> (Simulation<'m, C>, W, Vec<u8>) {
    let mut bytes = Vec::new();
    sim.snapshot_to(&mut bytes).unwrap();
    let w = W::default();
    let mut r = Simulation::<C>::restore_from(&bytes[..], cm, w.outputs(tape)).unwrap();
    let mut again = Vec::new();
    r.snapshot_to(&mut again).unwrap();
    assert!(again == bytes, "the restored state writes other bytes");
    (r, w, bytes)
}

/// The `k`-th edit of [`direct_edits_survive_snapshots`]; the new generation.
fn edit<C: Compat>(sim: &mut Simulation<'_, C>, k: u32) -> u32 {
    let mut e = Edit::new(sim);
    match k {
        0 => {
            let server = e.find("_h19_linking_resource_rc2").unwrap();
            let copy = e.clone_container(server, "_rc3", "ServerNode2").unwrap();
            for link in e.links_of(server) {
                e.connect(link, copy).unwrap();
            }
            let ac = e.find("_h19_linking_resource_ac2").unwrap();
            e.allocate(ac, copy, "_alc3").unwrap();
        }
        1 => {
            let p = e.find("_rc3_0").unwrap();
            e.set_rate(p, "0.5").unwrap();
        }
        _ => {
            let c = e.find("_rc3").unwrap();
            e.remove_from_environment(c).unwrap();
        }
    }
    e.apply(sim).unwrap().generation
}

/// h19: three generations of direct edits (a new container with the server allocated to it; a
/// faster CPU for it; the container removed from the environment, a tombstone), the run written
/// and restored after each and in between; the edits after a restore work on the restored
/// graph. Equal to the run without snapshots, exact and fast.
#[test]
fn direct_edits_survive_snapshots() {
    let (cm, cfg) = h19();
    for mode in [Mode::Exact, Mode::Fast] {
        let cfg = SimConfig {
            mode,
            ..cfg.clone()
        };
        let tape = mode == Mode::Exact;
        macro_rules! go {
            ($c:ty) => {{
                // uninterrupted
                let w = W::default();
                let mut sim = Simulation::<$c>::create(&cm, cfg.clone(), w.outputs(tape)).unwrap();
                for k in 0..3 {
                    sim.run_until(Pause::After((5 + 5 * k as i64) * S)).unwrap();
                    assert_eq!(edit(&mut sim, k), k + 1);
                }
                let want = [result(&sim.finish()), w.trace.text(), w.tape.text()];
                // with snapshots
                let w = W::default();
                let mut sim = Simulation::<$c>::create(&cm, cfg.clone(), w.outputs(tape)).unwrap();
                let (mut trace, mut tp) = (String::new(), String::new());
                let mut cur = w;
                for k in 0..3 {
                    sim.run_until(Pause::After((5 + 5 * k as i64) * S)).unwrap();
                    assert_eq!(edit(&mut sim, k), k + 1);
                    let (s2, w2, bytes) = cycle(&mut sim, &cm, tape);
                    let snap = Snapshot::read(&bytes[..]).unwrap();
                    assert_eq!(snap.header.run_model.as_ref().unwrap().generation, k + 1);
                    assert!(snap.section("model").is_some());
                    trace.push_str(&cur.trace.text());
                    tp.push_str(&cur.tape.text());
                    drop(sim);
                    sim = s2;
                    cur = w2;
                    // and once between changes
                    sim.run_until(Pause::After((7 + 5 * k as i64) * S)).unwrap();
                    let (s3, w3, _) = cycle(&mut sim, &cm, tape);
                    trace.push_str(&cur.trace.text());
                    tp.push_str(&cur.tape.text());
                    drop(sim);
                    sim = s3;
                    cur = w3;
                }
                let r = result(&sim.finish());
                let got = [r, trace + &cur.trace.text(), tp + &cur.tape.text()];
                same(&format!("h19 edits {mode}"), &want, &got);
            }};
        }
        match mode {
            Mode::Exact => go!(Exact),
            Mode::Fast => go!(Fast),
        }
    }
}

fn rule(name: &str) -> Transformation {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/rules")
        .join(name);
    core().compile_file(&path, &Imports::Folder).unwrap()
}

/// The ids of the objects a journal gave ids to.
fn new_ids(j: &qvto_emf::Journal) -> Vec<String> {
    j.changes
        .iter()
        .filter_map(|c| match &c.kind {
            ChangeKind::Set { feature, new, .. } if feature.0 == mf::identifier_Identifier_id.0 => {
                Some(format!("{new:?}"))
            }
            _ => None,
        })
        .collect()
}

/// A rule that adds a container (seeded UUIDs) ran; the run and its QVTo state are written
/// (section `qvto`) and restored into a fresh simulation and a fresh QVTo state: the next run
/// of the rule gives the ids the uninterrupted run gives, and the run continues equally. A
/// fresh QVTo state without the section would repeat the first ids.
#[test]
fn the_qvto_state_continues_its_id_sequence() {
    let (cm, cfg) = h19();
    let cfg = SimConfig {
        max_sim_time: 200,
        max_measurements: -1,
        ..cfg
    };
    let add = rule("add_container.qvto");
    let rate = rule("set/a_rate.qvto");
    let inv = Invocation::simulizar();
    let qrun = || QvtoRun::new(core(), RunConfig::default()).unwrap();
    // uninterrupted
    let w = W::default();
    let mut sim = Simulation::<Exact>::create(&cm, cfg.clone(), w.outputs(true)).unwrap();
    sim.run_until(Pause::After(100 * S)).unwrap();
    let mut q = qrun();
    q.invoke(&mut sim, &rate, &inv).unwrap();
    let first = q.invoke(&mut sim, &add, &inv).unwrap();
    sim.run_until(Pause::After(150 * S)).unwrap();
    let second = q.invoke(&mut sim, &add, &inv).unwrap();
    let want = [result(&sim.finish()), w.trace.text(), w.tape.text()];
    assert_ne!(new_ids(&first.journal), new_ids(&second.journal));
    // with a snapshot after the first rules
    for enc in [Encoding::Postcard, Encoding::Json] {
        let w = W::default();
        let mut sim = Simulation::<Exact>::create(&cm, cfg.clone(), w.outputs(true)).unwrap();
        sim.run_until(Pause::After(100 * S)).unwrap();
        let mut q = qrun();
        q.invoke(&mut sim, &rate, &inv).unwrap();
        q.invoke(&mut sim, &add, &inv).unwrap();
        let qsec = q.snapshot_section(enc).unwrap();
        let s = sim.snapshot_value(enc, vec![qsec]).unwrap();
        let mut bytes = Vec::new();
        s.write(&mut bytes).unwrap();
        let (t0, u0) = (w.trace.text(), w.tape.text());
        drop((sim, q));
        let s = Snapshot::read(&bytes[..]).unwrap();
        let w2 = W::default();
        let mut sim = Simulation::<Exact>::restore_snapshot(&s, &cm, w2.outputs(true)).unwrap();
        assert_eq!(sim.state().model().generation(), 2);
        let mut q = qrun();
        q.restore_section(&sim, &s).unwrap();
        assert_eq!(q.invocations(), 2);
        sim.run_until(Pause::After(150 * S)).unwrap();
        let again = q.invoke(&mut sim, &add, &inv).unwrap();
        assert_eq!(new_ids(&again.journal), new_ids(&second.journal), "{enc:?}");
        assert!(again.log.contains("added3"), "{}", again.log);
        let got = [
            result(&sim.finish()),
            t0 + &w2.trace.text(),
            u0 + &w2.tape.text(),
        ];
        same(&format!("qvto state {enc:?}"), &want, &got);
    }
    // without the QVTo state: the ids start over
    let mut sim = Simulation::<Exact>::create(&cm, cfg.clone(), Outputs::default()).unwrap();
    sim.run_until(Pause::After(100 * S)).unwrap();
    let mut q = qrun();
    let fresh = q.invoke(&mut sim, &add, &inv).unwrap();
    assert_eq!(new_ids(&fresh.journal), new_ids(&first.journal));
}

// ------------------------------------------------------------------------------------------
// SimuLizar's reconfiguration examples

fn load(dir: &Path) -> (CompiledModel, SimConfig) {
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let model = spec.load_model(dir).unwrap();
    let cm = CompiledModel::compile(model).unwrap();
    let name = dir.file_name().unwrap().to_string_lossy().into_owned();
    (cm, spec.sim_config(&name))
}

fn host(rules: &Path, mode: QvtMode) -> RulesHost {
    let set = RuleSet::load_dir(core(), rules).unwrap();
    let cfg = RunConfig {
        mode,
        ..RunConfig::default()
    };
    RulesHost::new(core(), set, cfg).unwrap().keep_records()
}

fn examples() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(root().join("corpus-rules"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("run.json").is_file())
        .collect();
    v.sort();
    assert_eq!(v.len(), 11);
    v
}

/// What the records say, without wall times.
fn records(h: &mut RulesHost) -> String {
    let mut s = String::new();
    for r in h.take_records() {
        s.push_str(&format!("{} {} {}\n", r.at_ns, r.post_stop, r.succeeded));
        for x in r.rules {
            s.push_str(&format!(
                "  {} {} {} {}\n",
                x.name, x.succeeded, x.changed, x.log
            ));
        }
    }
    let st = h.stats();
    s.push_str(&format!(
        "runs {} ok {} changed {}\n",
        st.runs, st.succeeded, st.changed
    ));
    s
}

/// Runs an example with rules: uninterrupted, or written and restored into a fresh simulation
/// and a fresh host at each of `pauses` (and once while a reconfiguration waits for the host).
fn rules_run<C: Compat>(
    dir: &Path,
    cfg: &SimConfig,
    cm: &CompiledModel,
    mode: QvtMode,
    pauses: &[Pause],
    tape: bool,
    enc: Encoding,
) -> ([String; 3], String, usize) {
    let w = W::default();
    let sim = Simulation::<C>::create(cm, cfg.clone(), w.outputs(tape)).unwrap();
    let mut run = RulesRun::new(sim, host(&dir.join("rules"), mode));
    let (mut trace, mut tp) = (String::new(), String::new());
    let mut cur = w;
    let mut n = 0;
    let mut waited = false;
    for &p in pauses {
        if run.run_until(p).unwrap() == Status::Finished {
            break;
        }
        if !waited {
            // to the next reconfiguration, which waits for the host while the run is written
            run.sim_mut().run_while(|s| !s.reconf_pending()).unwrap();
            waited = run.sim().reconf_due().is_some();
        }
        let mut bytes = Vec::new();
        run.snapshot_with(&mut bytes, &SnapshotOptions { encoding: enc })
            .unwrap();
        trace.push_str(&cur.trace.text());
        tp.push_str(&cur.tape.text());
        drop(run);
        let w = W::default();
        run = RulesRun::<C>::restore_from(
            &bytes[..],
            cm,
            host(&dir.join("rules"), mode),
            w.outputs(tape),
        )
        .unwrap();
        cur = w;
        n += 1;
    }
    let (r, mut h) = run.finish().unwrap();
    let out = [
        r.measurements.to_csv(),
        trace + &cur.trace.text(),
        tp + &cur.tape.text(),
    ];
    (out, records(&mut h), n)
}

#[test]
fn rules_runs_continue_from_snapshots() {
    let mut n = 0;
    for dir in examples() {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let (cm, cfg) = load(&dir);
        let end = cfg.max_sim_time.max(1) * S;
        let pauses: Vec<Pause> = [1, 3, 10, 30, 60, 95]
            .iter()
            .map(|k| Pause::After(end * k / 100))
            .collect();
        // exact build: the rules' default QVTo mode (Auto: ordered) for the slow examples' few
        // rules, fast for the power example's 13 000 runs, as `rules_process.rs`
        let qmode = if name == "power_rules_qvto" {
            QvtMode::Fast
        } else {
            QvtMode::Auto
        };
        let (want, wrec, _) =
            rules_run::<Exact>(&dir, &cfg, &cm, qmode, &[], true, Encoding::Postcard);
        let enc = if name == "lb_scaleout" {
            Encoding::Json
        } else {
            Encoding::Postcard
        };
        let (got, grec, k) = rules_run::<Exact>(&dir, &cfg, &cm, qmode, &pauses, true, enc);
        assert!(k >= 3, "{name}: {k} snapshots");
        same(&format!("{name} exact"), &want, &got);
        assert_eq!(wrec, grec, "{name}: rule records");
        // the reference's outputs
        let exp = |f: &str| {
            simoxide_testkit::corpus::read_maybe_gz(&dir.join("expected").join(f))
                .unwrap()
                .unwrap()
        };
        assert!(got[0] == exp("measurements.csv"), "{name}: measurements");
        assert!(got[1] == exp("trace.jsonl"), "{name}: trace");
        // fast mode (simulator and QVTo)
        let fcfg = SimConfig {
            mode: Mode::Fast,
            ..cfg.clone()
        };
        let (want, wrec, _) = rules_run::<Fast>(
            &dir,
            &fcfg,
            &cm,
            QvtMode::Fast,
            &[],
            false,
            Encoding::Postcard,
        );
        let (got, grec, k2) = rules_run::<Fast>(
            &dir,
            &fcfg,
            &cm,
            QvtMode::Fast,
            &pauses,
            false,
            Encoding::Postcard,
        );
        same(&format!("{name} fast"), &want, &got);
        assert_eq!(wrec, grec, "{name}: rule records (fast)");
        n += k + k2;
    }
    eprintln!("rules: {n} snapshots");
}

//! `SimConfig::fix_double_resume`: the opt-in fix of the reference's double resume (REF-2,
//! SIM-4.4a, ACT-9.5), on corpus model h26 (a synchronous fork whose two children finish without
//! waiting, so both resume the parent before its first resume is processed), changed in memory:
//!
//! * `h26`: the second resume wakes the parent early from the CPU demand after the join;
//! * `hold`: the demand comes before the fork, so the second resume wakes the user early from
//!   its think time and its next demand aborts the run (`activate() of a running process`, as
//!   the reference's `Tried to schedule thread which was not suspended`; the failure of a
//!   MiSArch design, where two children's demands ended in the same nanosecond);
//! * `one child`: the same model with one synchronous child, where no second resume exists.
//!
//! The reference behaviour stays the default. With the fix, both models run like their
//! one-child variants (the children draw nothing), in every mode, also paused and snapshotted;
//! models whose children never resume a parent that has left its join run byte for byte as
//! without it (trace, measurements, event and draw counts).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use simoxide_sim::snapshot::{Encoding, SnapshotOptions};
use simoxide_sim::{
    CompiledModel, Exact, Fast, Mode, Outputs, Pause, RunResult, RunSpec, SimConfig, Simulation,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The model files of `corpus/<name>`, with `edits` applied to the file whose name ends with
/// the edit's suffix (each edit must apply exactly once).
fn files(name: &str, edits: &[(&str, &str, &str)]) -> Vec<(String, String)> {
    let dir = root().join("corpus").join(name);
    let mut v = Vec::new();
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = e.path();
        let n = p.file_name().unwrap().to_string_lossy().into_owned();
        if !p.is_file() || n.ends_with(".json") || n.ends_with(".txt") {
            continue;
        }
        let mut t = std::fs::read_to_string(&p).unwrap();
        for (suffix, a, b) in edits {
            if n.ends_with(suffix) {
                assert_eq!(t.matches(a).count(), 1, "{n}: {a}");
                t = t.replace(a, b);
            }
        }
        v.push((n, t));
    }
    v
}

const P: &str = "_h26_fork_double_resume_";

fn h26(edits: &[(&str, &str, &str)]) -> CompiledModel {
    // one user: the order of the users' draws cannot depend on the children's notes
    let mut all = vec![(".usagemodel", r#"population="2""#, r#"population="1""#)];
    all.extend_from_slice(edits);
    let m = RunSpec::default()
        .load_model_memory(&files("h26_fork_double_resume", &all))
        .unwrap();
    CompiledModel::compile(m).unwrap()
}

/// Edits of h26: the demand of 0.5 before the fork, nothing after it
/// (start → afterJoin (0.5) → fork → afterJoin2 (0.0) → stop).
fn hold() -> Vec<(&'static str, String, String)> {
    let a = |n: &str| format!("{P}act{n}");
    let r = ".repository";
    vec![
        (
            r,
            format!(
                r#"id="{}" entityName="start" successor_AbstractAction="{}""#,
                a("10"),
                a("7")
            ),
            format!(
                r#"id="{}" entityName="start" successor_AbstractAction="{}""#,
                a("10"),
                a("8")
            ),
        ),
        (
            r,
            format!(
                r#"entityName="fork" predecessor_AbstractAction="{}""#,
                a("10")
            ),
            format!(
                r#"entityName="fork" predecessor_AbstractAction="{}""#,
                a("8")
            ),
        ),
        // the fork's successor (afterJoin's successor is act9, followed by a newline)
        (
            r,
            format!(r#"successor_AbstractAction="{}">"#, a("8")),
            format!(r#"successor_AbstractAction="{}">"#, a("9")),
        ),
        (
            r,
            format!(
                r#"entityName="afterJoin" predecessor_AbstractAction="{}""#,
                a("7")
            ),
            format!(
                r#"entityName="afterJoin" predecessor_AbstractAction="{}""#,
                a("10")
            ),
        ),
        (
            r,
            format!(
                "successor_AbstractAction=\"{}\">\n        <resourceDemand_Action>\n          <specification_ParametericResourceDemand specification=\"0.5\"/>",
                a("9")
            ),
            format!(
                "successor_AbstractAction=\"{}\">\n        <resourceDemand_Action>\n          <specification_ParametericResourceDemand specification=\"0.5\"/>",
                a("7")
            ),
        ),
        (
            r,
            format!(
                r#"entityName="afterJoin2" predecessor_AbstractAction="{}""#,
                a("8")
            ),
            format!(
                r#"entityName="afterJoin2" predecessor_AbstractAction="{}""#,
                a("7")
            ),
        ),
        (
            r,
            r#"specification="Exp(10.0)""#.into(),
            r#"specification="0.0""#.into(),
        ),
    ]
}

/// Removes the second synchronous child (`fb2`) of h26's fork.
fn one_child() -> (&'static str, String, String) {
    let t = std::fs::read_to_string(
        root().join("corpus/h26_fork_double_resume/h26_fork_double_resume.repository"),
    )
    .unwrap();
    let open = format!(r#"<synchronousForkedBehaviours_SynchronisationPoint id="{P}fb2">"#);
    let close = "</synchronousForkedBehaviours_SynchronisationPoint>";
    let i = t.find(&open).unwrap();
    let j = i + t[i..].find(close).unwrap() + close.len();
    (".repository", t[i..j].to_string(), String::new())
}

fn model(hold_variant: bool, one: bool) -> CompiledModel {
    let mut e: Vec<(&str, String, String)> = if hold_variant { hold() } else { Vec::new() };
    if one {
        e.push(one_child());
    }
    let e: Vec<(&str, &str, &str)> = e
        .iter()
        .map(|(f, a, b)| (*f, a.as_str(), b.as_str()))
        .collect();
    h26(&e)
}

fn cfg(mode: Mode, fix: bool) -> SimConfig {
    SimConfig {
        seed: 25,
        max_measurements: 200,
        mode,
        fix_double_resume: fix,
        ..Default::default()
    }
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

/// End time, request and draw counts and the measurements of a run, or its error.
fn outcome(r: Result<RunResult, simoxide_sim::SimError>) -> String {
    match r {
        Ok(r) => format!(
            "end {} main {} draws {}\n{}",
            r.end_ns,
            r.main_count,
            r.uniforms,
            r.measurements.to_csv()
        ),
        Err(e) => format!("error: {e}"),
    }
}

fn run(cm: &CompiledModel, c: SimConfig) -> String {
    outcome(simoxide_sim::run(cm, c, Outputs::default()))
}

/// The trace and the full outcome, with the event count.
fn traced(cm: &CompiledModel, c: SimConfig) -> (String, String) {
    let t = Buf::default();
    let out = Outputs {
        trace: Some(Box::new(t.clone())),
        tape: None,
    };
    let r = simoxide_sim::run(cm, c, out);
    let events = r.as_ref().map(|r| r.events).unwrap_or(0);
    let s = format!("events {events} {}", outcome(r));
    (String::from_utf8(t.0.borrow().clone()).unwrap(), s)
}

#[test]
fn off_by_default() {
    assert!(!SimConfig::default().fix_double_resume);
}

/// The reference behaviour: the hold variant aborts, h26 runs but differs from one child.
#[test]
fn the_reference_double_resume_is_kept_by_default() {
    for mode in [Mode::Exact, Mode::Fast] {
        let r = run(&model(true, false), cfg(mode, false));
        assert!(
            r.starts_with("error:") && r.contains("activate() of a running process"),
            "{mode:?}: {r}"
        );
        let r = run(&model(false, false), cfg(mode, false));
        assert!(!r.starts_with("error:"), "{mode:?}: {r}");
        assert_ne!(r, run(&model(false, true), cfg(mode, false)), "{mode:?}");
    }
}

#[test]
fn with_the_fix_two_children_run_like_one() {
    for mode in [Mode::Exact, Mode::Fast] {
        for hold_variant in [false, true] {
            let want = run(&model(hold_variant, true), cfg(mode, false));
            assert!(!want.starts_with("error:"), "{want}");
            let got = run(&model(hold_variant, false), cfg(mode, true));
            assert_eq!(got, want, "{mode:?}, hold variant {hold_variant}");
            // one child never resumes a parent that has left its join
            assert_eq!(run(&model(hold_variant, true), cfg(mode, true)), want);
        }
    }
}

/// Models without a late resume: the same trace, events, draws and measurements.
#[test]
fn unchanged_without_a_late_resume() {
    for name in ["h11_fork_sync", "h12_fork_async", "x_pem_fork"] {
        let dir = root().join("corpus").join(name);
        // x_pem_fork: a third-party model (third_party/README.md)
        if !simoxide_thirdparty::skip_unless_available(&dir) {
            continue;
        }
        let spec = RunSpec::load(&dir.join("run.json")).unwrap();
        let cm = CompiledModel::compile(spec.load_model(&dir).unwrap()).unwrap();
        for mode in [Mode::Exact, Mode::Fast] {
            let mut c = spec.sim_config(name);
            c.mode = mode;
            let want = traced(&cm, c.clone());
            assert!(!want.0.is_empty(), "{name}: no trace");
            c.fix_double_resume = true;
            assert!(
                traced(&cm, c) == want,
                "{name} {mode:?}: the fix changed the run"
            );
        }
    }
    let cm = model(true, true);
    for mode in [Mode::Exact, Mode::Fast] {
        let want = traced(&cm, cfg(mode, false));
        assert!(traced(&cm, cfg(mode, true)) == want, "one child, {mode:?}");
    }
}

/// Pending join resumes survive a pause, a fork and a snapshot.
#[test]
fn paused_forked_and_snapshotted_runs_continue_identically() {
    fn check<C: simoxide_sim::Compat>(mode: Mode) {
        let cm = model(true, false);
        let c = cfg(mode, true);
        let full = |s: Simulation<'_, C>| outcome(s.finish());
        let want = full(Simulation::<C>::create(&cm, c.clone(), Outputs::default()).unwrap());
        assert!(!want.starts_with("error:"), "{want}");
        for n in [1u64, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144] {
            for how in ["direct", "fork", "postcard", "json"] {
                let mut sim = Simulation::<C>::create(&cm, c.clone(), Outputs::default()).unwrap();
                sim.run_until(Pause::Events(n)).unwrap();
                let cont = match how {
                    "direct" => sim,
                    "fork" => sim.fork(Outputs::default()).unwrap(),
                    enc => {
                        let encoding = if enc == "json" {
                            Encoding::Json
                        } else {
                            Encoding::Postcard
                        };
                        let mut bytes = Vec::new();
                        sim.snapshot_with(&mut bytes, &SnapshotOptions { encoding }, Vec::new())
                            .unwrap();
                        Simulation::<C>::restore_from(&bytes[..], &cm, Outputs::default()).unwrap()
                    }
                };
                assert_eq!(full(cont), want, "{mode:?}: pause after {n} events, {how}");
            }
        }
    }
    check::<Exact>(Mode::Exact);
    check::<Fast>(Mode::Fast);
}

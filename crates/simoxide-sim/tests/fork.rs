//! Pause, fork and resume (`RunState`, `DESIGN-SPD.md` 5.7, exactness target V6): a run paused
//! between two events and continued, a fork of it, a state taken out and resumed, a deep copy
//! continued on another thread — each gives the trace, tape, measurements, event, draw and
//! request counts of the uninterrupted run, byte for byte, on every model directory of the
//! repository, in exact mode (own stream and tape replay), in fast mode and on the literal
//! engine, at many pause points (random event counts, which also split same-time instants, and
//! event times).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use simoxide_sim::{
    Compat, CompiledModel, Exact, Fast, Literal, Mode, Outputs, Pause, Phase, RngMode, RunResult,
    RunState, SimConfig, Simulation, Status, Tape,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every model directory with a `run.json` (also those whose run aborts: the error must be the
/// same too).
fn model_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for sub in [
        "corpus",
        "corpus-fuzz",
        "crates/simoxide-sim/tests/models",
        "crates/simoxide-cli/bench/models",
    ] {
        let Ok(rd) = std::fs::read_dir(root().join(sub)) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.join("run.json").is_file() && simoxide_thirdparty::skip_unless_available(&p) {
                v.push(p);
            }
        }
    }
    v.sort();
    // fewer without the third-party submodules (third_party/README.md)
    assert!(
        v.len() > 80 || !simoxide_thirdparty::all_available(),
        "{} model directories",
        v.len()
    );
    v
}

fn filter() -> Option<Vec<String>> {
    std::env::var("FORK_FILTER")
        .ok()
        .map(|f| f.split(',').map(String::from).collect())
}

/// A shared in-memory writer.
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
        String::from_utf8(self.0.borrow().clone()).expect("utf-8")
    }
    fn len(&self) -> usize {
        self.0.borrow().len()
    }
}

/// Which outputs a run writes.
#[derive(Clone, Copy, Debug)]
struct Io {
    trace: bool,
    tape: bool,
}

struct Writers {
    trace: Buf,
    tape: Buf,
}

impl Writers {
    fn new() -> Writers {
        Writers {
            trace: Buf::default(),
            tape: Buf::default(),
        }
    }
    fn outputs(&self, io: Io) -> Outputs {
        Outputs {
            trace: io
                .trace
                .then(|| Box::new(self.trace.clone()) as Box<dyn std::io::Write>),
            tape: io
                .tape
                .then(|| Box::new(self.tape.clone()) as Box<dyn std::io::Write>),
        }
    }
}

/// Everything a run produces.
#[derive(PartialEq, Eq)]
struct Out {
    result: String,
    trace: String,
    tape: String,
}

impl std::fmt::Debug for Out {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // the first differing line is what matters
        writeln!(f, "{}", self.result.lines().next().unwrap_or(""))?;
        writeln!(
            f,
            "trace {} bytes, tape {} bytes",
            self.trace.len(),
            self.tape.len()
        )
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

fn check(name: &str, want: &Out, got: &Out) {
    for (what, a, b) in [
        ("result", &want.result, &got.result),
        ("trace", &want.trace, &got.trace),
        ("tape", &want.tape, &got.tape),
    ] {
        assert!(a == b, "{name}: {what} differs: {}", first_difference(a, b));
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

fn finish<C: Compat>(sim: Simulation<'_, C>, w: &Writers, prefix: (&str, &str)) -> Out {
    let r = sim.finish();
    Out {
        result: fingerprint(&r),
        trace: format!("{}{}", prefix.0, w.trace.text()),
        tape: format!("{}{}", prefix.1, w.tape.text()),
    }
}

fn uninterrupted<C: Compat>(cm: &CompiledModel, cfg: &SimConfig, io: Io) -> Option<Out> {
    let w = Writers::new();
    let sim = Simulation::<C>::create(cm, cfg.clone(), w.outputs(io)).ok()?;
    Some(finish(sim, &w, ("", "")))
}

/// Paused at `p`: the paused run continued, and a fork of it (its output prefixed with what the
/// origin wrote up to the fork).
fn forked<C: Compat>(cm: &CompiledModel, cfg: &SimConfig, io: Io, p: Pause) -> (Out, Out) {
    let w = Writers::new();
    let mut sim = Simulation::<C>::create(cm, cfg.clone(), w.outputs(io)).expect("create");
    let _ = sim.run_until(p);
    let wf = Writers::new();
    let fork = sim.fork(wf.outputs(io)).expect("fork");
    let prefix = (w.trace.text(), w.tape.text());
    let a = finish(sim, &w, ("", ""));
    let b = finish(fork, &wf, (&prefix.0, &prefix.1));
    (a, b)
}

/// Paused several times, the state taken out and resumed each time, and finally deep-copied
/// and finished on another thread.
fn resumed<C: Compat>(cm: &CompiledModel, cfg: &SimConfig, io: Io, ps: &[Pause]) -> Out {
    let mut trace = String::new();
    let mut tape = String::new();
    let mut w = Writers::new();
    let mut sim = Simulation::<C>::create(cm, cfg.clone(), w.outputs(io)).expect("create");
    for &p in ps {
        let _ = sim.run_until(p);
        let st: RunState<C> = sim.into_state();
        trace.push_str(&w.trace.text());
        tape.push_str(&w.tape.text());
        w = Writers::new();
        sim = Simulation::resume(cm, st, w.outputs(io)).expect("resume");
    }
    let _ = sim.run_while(|s| s.events() % 97 != 13);
    let st = sim.into_state();
    trace.push_str(&w.trace.text());
    tape.push_str(&w.tape.text());
    let detached = st.detach();
    drop(st);
    let (result, t2, u2) = std::thread::scope(|sc| {
        sc.spawn(|| {
            let w = Writers::new();
            let sim = Simulation::resume(cm, detached.into_state(), w.outputs(io)).expect("resume");
            let r = sim.finish();
            (fingerprint(&r), w.trace.text(), w.tape.text())
        })
        .join()
        .expect("thread")
    });
    Out {
        result,
        trace: trace + &t2,
        tape: tape + &u2,
    }
}

/// Deterministic pause points for a run of `events` events ending at `end_ns`.
fn pauses(events: u64, end_ns: i64, salt: u64) -> Vec<Pause> {
    let mut x = 0x9e37_79b9_7f4a_7c15u64 ^ salt.wrapping_mul(0x2545_f491_4f6c_dd1d);
    let mut rnd = move |n: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        if n == 0 { 0 } else { x % n }
    };
    let e = events.max(1);
    let mut v = vec![
        Pause::Events(0),
        Pause::Events(1),
        Pause::Events(e / 2),
        Pause::Events(e.saturating_sub(1)),
        Pause::Events(e),
        Pause::Events(e + 10),
        Pause::Before(end_ns / 3),
        Pause::After(end_ns / 3),
        Pause::Before(end_ns),
        Pause::After(0),
    ];
    for _ in 0..4 {
        v.push(Pause::Events(rnd(e + 1)));
    }
    v
}

/// The checks on one model, configuration and policy.
fn check_model<C: Compat>(name: &str, cm: &CompiledModel, cfg: &SimConfig, io: Io, salt: u64) {
    let Some(want) = uninterrupted::<C>(cm, cfg, io) else {
        return;
    };
    // a run without outputs gives the same result (and the fork of a traced run without a trace)
    let plain = uninterrupted::<C>(
        cm,
        cfg,
        Io {
            trace: false,
            tape: false,
        },
    )
    .expect("plain");
    assert_eq!(
        plain.result, want.result,
        "{name}: outputs change the result"
    );
    let (events, end) = parse_counts(&want.result);
    let ps = pauses(events, end, salt);
    for &p in &ps {
        let (a, b) = forked::<C>(cm, cfg, io, p);
        check(&format!("{name} {io:?} origin after {p:?}"), &want, &a);
        check(&format!("{name} {io:?} fork at {p:?}"), &want, &b);
    }
    let multi = [ps[2], ps[ps.len() - 1], ps[ps.len() - 2], ps[3]];
    let mut sorted = multi;
    sorted.sort_by_key(|p| match *p {
        Pause::Events(n) => n,
        _ => 0,
    });
    let r = resumed::<C>(cm, cfg, io, &sorted);
    check(&format!("{name} {io:?} resumed at {sorted:?}"), &want, &r);
}

/// `events` and `end` of a fingerprint (0 for an aborted run).
fn parse_counts(fp: &str) -> (u64, i64) {
    let w: Vec<&str> = fp.split_whitespace().collect();
    if w.first() != Some(&"end") {
        return (200, 1_000_000_000);
    }
    (w[7].parse().unwrap(), w[1].parse().unwrap())
}

/// x_sl_mediastore with a longer run than its `run.json`.
fn mediastore(ms: &[(String, CompiledModel, SimConfig)]) -> (&CompiledModel, SimConfig) {
    let (_, cm, base) = ms
        .iter()
        .find(|m| m.0 == "x_sl_mediastore")
        .expect("x_sl_mediastore");
    let cfg = SimConfig {
        max_measurements: 2000,
        ..base.clone()
    };
    (cm, cfg)
}

fn models() -> Vec<(String, CompiledModel, SimConfig)> {
    let f = filter();
    model_dirs()
        .into_iter()
        .filter_map(|d| {
            let m = simoxide_testkit::equiv::Model::from_dir(&d).ok()?;
            if let Some(f) = &f
                && !f.iter().any(|x| m.name.contains(x.as_str()))
            {
                return None;
            }
            Some((m.name, m.cm, m.base))
        })
        .collect()
}

#[test]
fn fork_and_resume_equal_the_uninterrupted_run_exact() {
    let ms = models();
    let mut n = 0;
    for (i, (name, cm, base)) in ms.iter().enumerate() {
        for (j, cfg) in [
            base.clone(),
            SimConfig {
                seed: 7,
                max_measurements: 300,
                max_sim_time: 2000,
                ..base.clone()
            },
        ]
        .iter()
        .enumerate()
        {
            let salt = (i * 2 + j) as u64;
            check_model::<Exact>(
                name,
                cm,
                cfg,
                Io {
                    trace: true,
                    tape: true,
                },
                salt,
            );
            check_model::<Exact>(
                name,
                cm,
                cfg,
                Io {
                    trace: false,
                    tape: false,
                },
                salt + 1000,
            );
            n += 1;
        }
    }
    assert!(
        n > 160 || !simoxide_thirdparty::all_available(),
        "{n} cases"
    );
}

#[test]
fn fork_and_resume_equal_the_uninterrupted_run_fast() {
    for (i, (name, cm, base)) in models().iter().enumerate() {
        let cfg = SimConfig {
            mode: Mode::Fast,
            ..base.clone()
        };
        check_model::<Fast>(
            name,
            cm,
            &cfg,
            Io {
                trace: true,
                tape: false,
            },
            i as u64,
        );
        check_model::<Fast>(
            name,
            cm,
            &cfg,
            Io {
                trace: false,
                tape: false,
            },
            i as u64 + 77,
        );
    }
}

#[test]
fn fork_and_resume_equal_the_uninterrupted_run_literal_engine() {
    for (i, (name, cm, base)) in models().iter().enumerate().step_by(3) {
        check_model::<Literal<Exact>>(
            name,
            cm,
            base,
            Io {
                trace: true,
                tape: true,
            },
            i as u64,
        );
    }
}

/// Tape replay: the fork shares the replayed tape and continues at its position.
#[test]
fn fork_of_a_replaying_run() {
    for (i, (name, cm, base)) in models().iter().enumerate().step_by(2) {
        let Some(own) = uninterrupted::<Exact>(
            cm,
            base,
            Io {
                trace: false,
                tape: true,
            },
        ) else {
            continue;
        };
        let Ok(tape) = Tape::parse(&own.tape) else {
            continue;
        };
        let cfg = SimConfig {
            rng: RngMode::Replay(Arc::new(tape)),
            ..base.clone()
        };
        check_model::<Exact>(
            name,
            cm,
            &cfg,
            Io {
                trace: true,
                tape: true,
            },
            i as u64,
        );
    }
}

/// A tape attached at a pause continues the full run's tape; a trace can be dropped but not
/// added.
#[test]
fn outputs_can_change_at_a_pause() {
    for (name, cm, base) in models().iter().filter(|m| {
        [
            "h01_ps_single",
            "x_sl_mediastore",
            "h11_fork_sync",
            "t_reconf_tda",
        ]
        .iter()
        .any(|n| m.0 == *n)
    }) {
        let io = Io {
            trace: true,
            tape: true,
        };
        let want = uninterrupted::<Exact>(cm, base, io).expect("run");
        let (events, _) = parse_counts(&want.result);
        for k in [0, 1, events / 3, events / 2] {
            // with outputs up to k: the length of the tape prefix
            let w = Writers::new();
            let mut sim = Simulation::<Exact>::create(cm, base.clone(), w.outputs(io)).unwrap();
            sim.run_until(Pause::Events(k)).unwrap();
            let _ = sim.into_state();
            let (tl, ul) = (w.trace.len(), w.tape.len());
            // without a tape up to k, with one from k on
            let w = Writers::new();
            let o = Io {
                trace: true,
                tape: false,
            };
            let mut sim = Simulation::<Exact>::create(cm, base.clone(), w.outputs(o)).unwrap();
            sim.run_until(Pause::Events(k)).unwrap();
            let st = sim.into_state();
            assert_eq!(w.trace.len(), tl);
            let w2 = Writers::new();
            let sim = Simulation::resume(cm, st.clone(), w2.outputs(io)).unwrap();
            let got = finish(sim, &w2, (&want.trace[..tl], &want.tape[..ul]));
            check(&format!("{name}: tape attached at {k}"), &want, &got);
            // the trace dropped from k on
            let w3 = Writers::new();
            let sim = Simulation::resume(
                cm,
                st,
                w3.outputs(Io {
                    trace: false,
                    tape: false,
                }),
            )
            .unwrap();
            assert_eq!(fingerprint(&sim.finish()), want.result);
            assert_eq!(w3.trace.len(), 0);
        }
        // an untraced state cannot get a trace
        let mut sim = Simulation::<Exact>::create(cm, base.clone(), Outputs::default()).unwrap();
        sim.run_until(Pause::Events(5)).unwrap();
        let st = sim.into_state();
        let e = Simulation::resume(cm, st, Writers::new().outputs(io))
            .err()
            .expect("trace refused");
        assert!(e.message.contains("trace"), "{}", e.message);
    }
}

#[test]
fn pause_semantics() {
    let ms = models();
    let (cm, base) = mediastore(&ms);
    let mut sim = Simulation::<Exact>::create(cm, base, Outputs::default()).unwrap();
    assert_eq!(sim.state().phase(), Phase::NotStarted);
    assert_eq!(sim.run_until(Pause::Events(0)).unwrap(), Status::Paused);
    assert_eq!(sim.state().phase(), Phase::Running);
    assert_eq!(sim.state().events(), 0);
    // by event count: exactly n, or one more when the last event ran a process hand-off
    sim.run_until(Pause::Events(100)).unwrap();
    let n = sim.state().events();
    assert!(n == 100 || n == 101, "{n}");
    // by time: everything up to t, nothing after
    let t = sim.state().now_ns() + 1_000_000_000;
    sim.run_until(Pause::After(t)).unwrap();
    assert!(sim.state().now_ns() <= t);
    assert!(sim.state().next_event_ns().unwrap() > t);
    let t2 = sim.state().next_event_ns().unwrap();
    sim.run_until(Pause::Before(t2)).unwrap();
    assert_eq!(sim.state().next_event_ns(), Some(t2));
    sim.run_until(Pause::After(t2)).unwrap();
    assert!(sim.state().now_ns() == t2 && sim.state().next_event_ns().unwrap() > t2);
    // a pause after the end: the run finishes
    assert_eq!(sim.run_until(Pause::Never).unwrap(), Status::Finished);
    assert_eq!(sim.state().phase(), Phase::Finished);
    assert_eq!(
        sim.run_until(Pause::Events(0)).unwrap(),
        Status::Finished,
        "finished stays finished"
    );
    let r = sim.finish().unwrap();
    assert!(r.main_count > 0);
}

#[test]
fn a_failed_run_stays_failed() {
    // the livelock guard: a zero-time loop
    let ms = models();
    let failing: Vec<_> = ms
        .iter()
        .filter(|(_, cm, base)| simoxide_sim::run(cm, base.clone(), Outputs::default()).is_err())
        .collect();
    assert!(!failing.is_empty());
    for (name, cm, base) in failing {
        let want = simoxide_sim::run(cm, base.clone(), Outputs::default()).unwrap_err();
        let mut sim = Simulation::<Exact>::create(cm, base.clone(), Outputs::default()).unwrap();
        let e = sim.run_until(Pause::Never).unwrap_err();
        assert_eq!(e.message, want.message, "{name}");
        assert_eq!(sim.state().phase(), Phase::Failed);
        let mut fork = sim.fork(Outputs::default()).unwrap();
        assert_eq!(
            fork.run_until(Pause::Never).unwrap_err().message,
            want.message
        );
        assert_eq!(sim.finish().unwrap_err().message, want.message);
    }
}

#[test]
fn resume_checks_the_model() {
    let ms = models();
    let a = ms.iter().find(|m| m.0 == "h01_ps_single").unwrap();
    let b = ms.iter().find(|m| m.0 == "x_sl_mediastore").unwrap();
    let mut sim = Simulation::<Exact>::create(&a.1, a.2.clone(), Outputs::default()).unwrap();
    sim.run_until(Pause::Events(10)).unwrap();
    let e = Simulation::resume(&b.1, sim.into_state(), Outputs::default())
        .err()
        .expect("other model");
    assert!(e.message.contains("model"), "{}", e.message);
}

/// A fork shares the cancellation flag of its origin unless it gets limits of its own.
#[test]
fn forks_get_their_own_limits() {
    let ms = models();
    let (cm, mut cfg) = mediastore(&ms);
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    cfg.limits.cancel = Some(flag.clone());
    let mut sim = Simulation::<Exact>::create(cm, cfg, Outputs::default()).unwrap();
    sim.run_until(Pause::Events(50)).unwrap();
    let mut own = sim.fork(Outputs::default()).unwrap();
    own.set_limits(simoxide_sim::Limits::default());
    let mut shared = sim.fork(Outputs::default()).unwrap();
    // (the same limits again: checked at the next event instead of the next periodic check)
    let same = sim.state().config().limits.clone();
    shared.set_limits(same.clone());
    sim.set_limits(same);
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    let e = shared.finish().unwrap_err();
    assert_eq!(e.kind, simoxide_sim::SimErrorKind::Cancelled);
    assert!(own.finish().is_ok());
    assert_eq!(
        sim.finish().unwrap_err().kind,
        simoxide_sim::SimErrorKind::Cancelled
    );
}

/// Many forks of one state, continued in parallel on other threads.
#[test]
fn detached_forks_run_in_parallel() {
    let ms = models();
    let (cm, base) = mediastore(&ms);
    let want = uninterrupted::<Exact>(
        cm,
        &base,
        Io {
            trace: false,
            tape: false,
        },
    )
    .unwrap();
    let mut sim = Simulation::<Exact>::create(cm, base, Outputs::default()).unwrap();
    sim.run_until(Pause::Events(500)).unwrap();
    let states: Vec<_> = (0..8).map(|_| sim.state().detach()).collect();
    let results: Vec<String> = std::thread::scope(|sc| {
        let hs: Vec<_> = states
            .into_iter()
            .map(|d| {
                sc.spawn(move || {
                    let s = Simulation::resume(cm, d.into_state(), Outputs::default()).unwrap();
                    fingerprint(&s.finish())
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    for r in results {
        assert_eq!(r, want.result);
    }
    assert_eq!(fingerprint(&sim.finish()), want.result);
}

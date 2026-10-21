//! Snapshots on disk (`simoxide_sim::snapshot`, `DESIGN-SPD.md` N1): a run paused at several
//! points, written to bytes each time, restored into a fresh simulation and continued gives the
//! trace, tape, measurements, event, draw and request counts of the uninterrupted run, byte for
//! byte, on every model directory of the repository: exact mode (with and without outputs, own
//! stream and tape replay), fast mode, the literal engine, the Slingshot flavour (both modes,
//! wherever it runs the model), after a model change, in postcard and JSON. A restored state
//! writes the same bytes again (snapshots are deterministic and lossless), and wrong models,
//! modes, flavours and versions are refused with clear errors.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use simoxide_model::meta::feat;
use simoxide_model::raw::ObjId;
use simoxide_sim::snapshot::{Encoding, Snapshot, SnapshotError, SnapshotOptions};
use simoxide_sim::{
    ChangeKind, ChangeSet, Compat, CompiledModel, Exact, Fast, Flavour, FlavourConfig, Literal,
    Mode, Outputs, Pause, RngMode, RunResult, RunSpec, SimConfig, SimuLizar, Simulation, Slingshot,
    Tape,
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

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
    std::env::var("SNAPSHOT_FILTER")
        .ok()
        .map(|f| f.split(',').map(String::from).collect())
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
}

#[derive(Clone, Copy, Debug)]
struct Io {
    trace: bool,
    tape: bool,
}

const ALL: Io = Io {
    trace: true,
    tape: true,
};
const TRACE: Io = Io {
    trace: true,
    tape: false,
};
const NONE: Io = Io {
    trace: false,
    tape: false,
};

#[derive(Default)]
struct Writers {
    trace: Buf,
    tape: Buf,
}

impl Writers {
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

/// Result, trace and tape of a run.
type Out = [String; 3];

fn check(name: &str, want: &Out, got: &Out) {
    for (i, what) in ["result", "trace", "tape"].iter().enumerate() {
        assert!(
            want[i] == got[i],
            "{name}: {what} differs: {}",
            first_difference(&want[i], &got[i])
        );
    }
}

fn uninterrupted<C: Compat, F: Flavour>(
    cm: &CompiledModel,
    cfg: &SimConfig,
    io: Io,
) -> Option<Out> {
    let w = Writers::default();
    let sim = Simulation::<C, F>::create(cm, cfg.clone(), w.outputs(io)).ok()?;
    let r = fingerprint(&sim.finish());
    Some([r, w.trace.text(), w.tape.text()])
}

/// `events` and `end` of a fingerprint.
fn parse_counts(fp: &str) -> (u64, i64) {
    let w: Vec<&str> = fp.split_whitespace().collect();
    if w.first() != Some(&"end") {
        return (200, 1_000_000_000);
    }
    (w[7].parse().unwrap(), w[1].parse().unwrap())
}

/// Pause points in run order: event counts, some splitting same-time instants.
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
        Pause::Events(e / 3 + rnd(e / 3 + 1)),
        Pause::Events(e.saturating_sub(1)),
    ];
    v.sort_by_key(|p| match p {
        Pause::Events(n) => *n,
        _ => 0,
    });
    let _ = end_ns;
    v
}

/// Writes a snapshot, checks that it reads back and that the restored state writes the same
/// bytes, and returns the restored simulation with new writers.
fn cycle<'m, C: Compat, F: Flavour>(
    name: &str,
    sim: &mut Simulation<'m, C, F>,
    cm: &'m CompiledModel,
    io: Io,
    enc: Encoding,
) -> (Simulation<'m, C, F>, Writers) {
    let mut bytes = Vec::new();
    let opts = SnapshotOptions { encoding: enc };
    sim.snapshot_with(&mut bytes, &opts, Vec::new())
        .unwrap_or_else(|e| panic!("{name}: snapshot: {e}"));
    let w = Writers::default();
    let mut restored = Simulation::<C, F>::restore_from(&bytes[..], cm, w.outputs(io))
        .unwrap_or_else(|e| panic!("{name}: restore: {e}"));
    let mut again = Vec::new();
    restored
        .snapshot_with(&mut again, &opts, Vec::new())
        .unwrap();
    if again != bytes {
        if enc == Encoding::Json {
            let body = |x: &[u8]| {
                let s = Snapshot::read(x).unwrap();
                String::from_utf8(s.sections[0].data.clone()).unwrap()
            };
            let (a, b) = (body(&bytes), body(&again));
            let i = a
                .bytes()
                .zip(b.bytes())
                .position(|(x, y)| x != y)
                .unwrap_or(0);
            panic!(
                "{name}: the restored state writes other bytes at {i}:\n{}\n{}",
                &a[i.saturating_sub(200)..(i + 100).min(a.len())],
                &b[i.saturating_sub(200)..(i + 100).min(b.len())]
            );
        }
        panic!("{name}: the restored state writes other bytes");
    }
    (restored, w)
}

/// The run paused at every point of `ps`, written and restored into a fresh simulation each
/// time; the last restore continues on another thread.
fn snapshotted<C: Compat, F: Flavour>(
    name: &str,
    cm: &CompiledModel,
    cfg: &SimConfig,
    io: Io,
    ps: &[Pause],
    enc: Encoding,
) -> Out {
    let (mut trace, mut tape) = (String::new(), String::new());
    let w0 = Writers::default();
    let mut sim = Simulation::<C, F>::create(cm, cfg.clone(), w0.outputs(io)).expect("create");
    let mut w = w0;
    for &p in ps {
        let _ = sim.run_until(p);
        let (next, nw) = cycle(name, &mut sim, cm, io, enc);
        trace.push_str(&w.trace.text());
        tape.push_str(&w.tape.text());
        drop(sim);
        sim = next;
        w = nw;
    }
    // the last one: bytes to another thread, a fresh simulation there
    let _ = sim.run_while(|s| s.events() % 89 != 17);
    let mut bytes = Vec::new();
    sim.snapshot_with(&mut bytes, &SnapshotOptions { encoding: enc }, Vec::new())
        .expect("snapshot");
    trace.push_str(&w.trace.text());
    tape.push_str(&w.tape.text());
    drop(sim);
    let (r, t2, u2) = std::thread::scope(|sc| {
        sc.spawn(|| {
            let w = Writers::default();
            let sim = Simulation::<C, F>::restore_from(&bytes[..], cm, w.outputs(io))
                .unwrap_or_else(|e| panic!("{name}: restore on a thread: {e}"));
            (fingerprint(&sim.finish()), w.trace.text(), w.tape.text())
        })
        .join()
        .expect("thread")
    });
    [r, trace + &t2, tape + &u2]
}

/// One model, mode, flavour and outputs: the snapshotted runs equal the uninterrupted one.
/// Returns the number of snapshots taken.
fn check_model<C: Compat, F: Flavour>(
    name: &str,
    cm: &CompiledModel,
    cfg: &SimConfig,
    io: Io,
    salt: u64,
    enc: Encoding,
) -> usize {
    let Some(want) = uninterrupted::<C, F>(cm, cfg, io) else {
        return 0;
    };
    let (events, end) = parse_counts(&want[0]);
    let ps = pauses(events, end, salt);
    let got = snapshotted::<C, F>(name, cm, cfg, io, &ps, enc);
    check(&format!("{name} {io:?} {enc:?} at {ps:?}"), &want, &got);
    // by time: before and after a third of the run
    let tp = [Pause::Before(end / 3), Pause::After(end / 3)];
    let got = snapshotted::<C, F>(name, cm, cfg, io, &tp, enc);
    check(&format!("{name} {io:?} {enc:?} at {tp:?}"), &want, &got);
    ps.len() + tp.len() + 2
}

#[test]
fn snapshots_continue_like_the_uninterrupted_run_exact() {
    let mut n = 0;
    let mut models_done = 0;
    for (i, (name, cm, base)) in models().iter().enumerate() {
        let a = check_model::<Exact, SimuLizar>(name, cm, base, ALL, i as u64, Encoding::Postcard);
        let b = check_model::<Exact, SimuLizar>(
            name,
            cm,
            base,
            NONE,
            i as u64 + 500,
            Encoding::Postcard,
        );
        n += a + b;
        models_done += (a > 0) as usize;
    }
    eprintln!("exact: {models_done} models, {n} snapshots");
    if filter().is_none() {
        assert!(
            models_done > 100 || !simoxide_thirdparty::all_available(),
            "{models_done}"
        );
    }
}

#[test]
fn snapshots_continue_like_the_uninterrupted_run_fast() {
    let mut n = 0;
    let mut models_done = 0;
    for (i, (name, cm, base)) in models().iter().enumerate() {
        let cfg = SimConfig {
            mode: Mode::Fast,
            ..base.clone()
        };
        let a = check_model::<Fast, SimuLizar>(name, cm, &cfg, TRACE, i as u64, Encoding::Postcard);
        n += a;
        models_done += (a > 0) as usize;
    }
    eprintln!("fast: {models_done} models, {n} snapshots");
    if filter().is_none() {
        assert!(
            models_done > 100 || !simoxide_thirdparty::all_available(),
            "{models_done}"
        );
    }
}

/// The Slingshot flavour on every model it runs, both modes.
#[test]
fn snapshots_continue_like_the_uninterrupted_run_slingshot() {
    let mut n = 0;
    let mut models_done = 0;
    for (i, d) in model_dirs().iter().enumerate() {
        let name = d.file_name().unwrap().to_string_lossy().into_owned();
        if let Some(f) = filter()
            && !f.iter().any(|x| name.contains(x.as_str()))
        {
            continue;
        }
        let Ok(spec) = RunSpec::load(&d.join("run.json")) else {
            continue;
        };
        let Ok(model) = spec.load_model(d) else {
            continue;
        };
        let Ok(cm) = CompiledModel::compile_flavour(model, FlavourConfig::SLINGSHOT) else {
            continue;
        };
        let base = spec.sim_config(&name);
        let a =
            check_model::<Exact, Slingshot>(&name, &cm, &base, ALL, i as u64, Encoding::Postcard);
        let cfg = SimConfig {
            mode: Mode::Fast,
            ..base.clone()
        };
        let b = check_model::<Fast, Slingshot>(
            &name,
            &cm,
            &cfg,
            TRACE,
            i as u64 + 7,
            Encoding::Postcard,
        );
        n += a + b;
        models_done += (a > 0) as usize;
    }
    eprintln!("slingshot: {models_done} models, {n} snapshots");
    if filter().is_none() {
        assert!(
            models_done > 50 || !simoxide_thirdparty::all_available(),
            "{models_done}"
        );
    }
}

#[test]
fn snapshots_of_the_literal_engine_and_of_tape_replays() {
    let mut n = 0;
    for (i, (name, cm, base)) in models().iter().enumerate().step_by(4) {
        n += check_model::<Literal<Exact>, SimuLizar>(
            name,
            cm,
            base,
            ALL,
            i as u64,
            Encoding::Postcard,
        );
        let Some(own) = uninterrupted::<Exact, SimuLizar>(
            cm,
            base,
            Io {
                trace: false,
                tape: true,
            },
        ) else {
            continue;
        };
        let Ok(tape) = Tape::parse(&own[2]) else {
            continue;
        };
        let cfg = SimConfig {
            rng: RngMode::Replay(Arc::new(tape)),
            ..base.clone()
        };
        n += check_model::<Exact, SimuLizar>(name, cm, &cfg, ALL, i as u64, Encoding::Postcard);
    }
    if filter().is_none() {
        assert!(n > 100 || !simoxide_thirdparty::all_available(), "{n}");
    }
}

#[test]
fn json_snapshots_continue_too() {
    for (i, (name, cm, base)) in models().iter().enumerate().step_by(7) {
        check_model::<Exact, SimuLizar>(name, cm, base, ALL, i as u64, Encoding::Json);
    }
}

/// A change set that says "something changed" without changing the graph: the run gets a
/// compiled model of its own (generation 1), which the snapshot carries.
fn touch() -> ChangeSet {
    let mut cs = ChangeSet::new();
    cs.push(
        ObjId(0),
        ChangeKind::Set {
            feature: feat::NamedElement_entityName,
        },
    );
    cs
}

/// After model changes (the full change path: rebuild, recompile, merge, adoption), the
/// snapshot carries the run's own model; restoring it continues exactly, exact and fast.
#[test]
fn snapshots_after_model_changes() {
    let mut n = 0;
    for (i, (name, cm, base)) in models().iter().enumerate() {
        for mode in [Mode::Exact, Mode::Fast] {
            let cfg = SimConfig {
                mode,
                ..base.clone()
            };
            let io = if mode == Mode::Exact { ALL } else { TRACE };
            let want = match mode {
                Mode::Exact => uninterrupted::<Exact, SimuLizar>(cm, &cfg, io),
                Mode::Fast => uninterrupted::<Fast, SimuLizar>(cm, &cfg, io),
            };
            let Some(want) = want else { continue };
            let (events, _) = parse_counts(&want[0]);
            let got = match mode {
                Mode::Exact => changed_and_snapshotted::<Exact>(name, cm, &cfg, io, events, i),
                Mode::Fast => changed_and_snapshotted::<Fast>(name, cm, &cfg, io, events, i),
            };
            let Some(got) = got else { continue };
            check(&format!("{name} {mode} changed"), &want, &got);
            n += 1;
        }
    }
    eprintln!("model changes: {n} runs");
    if filter().is_none() {
        assert!(n > 150 || !simoxide_thirdparty::all_available(), "{n}");
    }
}

fn changed_and_snapshotted<C: Compat>(
    name: &str,
    cm: &CompiledModel,
    cfg: &SimConfig,
    io: Io,
    events: u64,
    salt: usize,
) -> Option<Out> {
    let w = Writers::default();
    let mut sim = Simulation::<C>::create(cm, cfg.clone(), w.outputs(io)).ok()?;
    let _ = sim.run_until(Pause::Events(events / 4));
    let g = sim.model_graph().clone();
    sim.apply_changes(g, &touch()).ok()?;
    let _ = sim.run_until(Pause::Events(events / 3 + (salt as u64 % 7)));
    let g = sim.model_graph().clone();
    sim.apply_changes(g, &touch()).ok()?;
    assert_eq!(sim.state().model().generation(), 2);
    let (mut sim2, w2) = cycle(name, &mut sim, cm, io, Encoding::Postcard);
    assert_eq!(sim2.state().model().generation(), 2);
    let (mut trace, mut tape) = (w.trace.text(), w.tape.text());
    drop(sim);
    // a third change after the restore, then another snapshot
    let _ = sim2.run_until(Pause::Events(events / 2));
    let g = sim2.model_graph().clone();
    sim2.apply_changes(g, &touch()).ok()?;
    let (sim3, w3) = cycle(name, &mut sim2, cm, io, Encoding::Postcard);
    trace.push_str(&w2.trace.text());
    tape.push_str(&w2.tape.text());
    drop(sim2);
    let r = fingerprint(&sim3.finish());
    Some([r, trace + &w3.trace.text(), tape + &w3.tape.text()])
}

/// The same state gives the same bytes: two runs to the same point, and a fork.
#[test]
fn snapshot_bytes_are_deterministic() {
    for (name, cm, base) in models().iter().step_by(5) {
        let bytes = |p: Pause| {
            let mut sim = Simulation::<Exact>::create(cm, base.clone(), Outputs::default()).ok()?;
            let _ = sim.run_until(p);
            let mut b = Vec::new();
            sim.snapshot_to(&mut b).expect("snapshot");
            let mut fork = sim.fork(Outputs::default()).unwrap();
            let mut c = Vec::new();
            fork.snapshot_to(&mut c).expect("snapshot");
            assert!(b == c, "{name}: a fork writes other bytes");
            Some(b)
        };
        let (Some(a), Some(b)) = (bytes(Pause::Events(77)), bytes(Pause::Events(77))) else {
            continue;
        };
        assert!(
            a == b,
            "{name}: two runs to the same point write other bytes"
        );
    }
}

#[test]
fn restoring_checks_model_mode_flavour_and_version() {
    let ms = models();
    let a = ms.iter().find(|m| m.0 == "h01_ps_single").unwrap();
    let b = ms.iter().find(|m| m.0 == "x_sl_mediastore").unwrap();
    let mut sim = Simulation::<Exact>::create(&a.1, a.2.clone(), Outputs::default()).unwrap();
    sim.run_until(Pause::Events(10)).unwrap();
    let mut bytes = Vec::new();
    let info = sim.snapshot_to(&mut bytes).unwrap();
    assert_eq!(info.bytes as usize, bytes.len());
    assert_eq!(info.header.sections.len(), 1);
    // another model
    let e = Simulation::<Exact>::restore_from(&bytes[..], &b.1, Outputs::default())
        .err()
        .expect("other model");
    assert!(matches!(e, SnapshotError::Model { .. }), "{e}");
    assert!(e.to_string().contains("another model"), "{e}");
    // the same model files compiled again: accepted
    let again =
        simoxide_testkit::equiv::Model::from_dir(&root().join("corpus/h01_ps_single")).unwrap();
    assert!(Simulation::<Exact>::restore_from(&bytes[..], &again.cm, Outputs::default()).is_ok());
    // another mode
    let e = Simulation::<Fast>::restore_from(&bytes[..], &a.1, Outputs::default())
        .err()
        .expect("other mode");
    assert!(e.to_string().contains("exact mode"), "{e}");
    // another flavour
    let e = Simulation::<Exact, Slingshot>::restore_from(&bytes[..], &a.1, Outputs::default())
        .err()
        .expect("other flavour");
    assert!(matches!(e, SnapshotError::Mismatch(_)), "{e}");
    // a trace for an untraced run
    let w = Writers::default();
    let e = Simulation::<Exact>::restore_from(&bytes[..], &a.1, w.outputs(ALL))
        .err()
        .expect("trace");
    assert!(e.to_string().contains("trace"), "{e}");
    // format and SimOxide versions, damage
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let v2 = bytes_with(&bytes, "SIMOXIDE-SNAPSHOT 1\n", "SIMOXIDE-SNAPSHOT 2\n");
    let e = Simulation::<Exact>::restore_from(&v2[..], &a.1, Outputs::default())
        .err()
        .unwrap();
    assert!(
        matches!(
            e,
            SnapshotError::Version {
                found: 2,
                supported: 1
            }
        ),
        "{e}"
    );
    let ver = format!("\"simoxide\":\"{}\"", env!("CARGO_PKG_VERSION"));
    assert!(text.contains(&ver));
    let old = bytes_with(&bytes, &ver, "\"simoxide\":\"0.0.0\"");
    let e = Simulation::<Exact>::restore_from(&old[..], &a.1, Outputs::default())
        .err()
        .unwrap();
    assert!(matches!(e, SnapshotError::SimOxideVersion { .. }), "{e}");
    let mut bad = bytes.clone();
    let k = bad.len() - 3;
    bad[k] ^= 0x55;
    let e = Simulation::<Exact>::restore_from(&bad[..], &a.1, Outputs::default())
        .err()
        .unwrap();
    assert!(e.to_string().contains("checksum"), "{e}");
    let e = Simulation::<Exact>::restore_from(&b"hello\n"[..], &a.1, Outputs::default())
        .err()
        .unwrap();
    assert!(matches!(e, SnapshotError::NotASnapshot), "{e}");
    let s = Snapshot::read(&bytes[..]).unwrap();
    assert_eq!(s.header.events, sim.state().events());
    assert_eq!(s.header.mode, "exact");
    assert_eq!(s.header.flavour, "simulizar");
}

fn bytes_with(b: &[u8], from: &str, to: &str) -> Vec<u8> {
    let i = b
        .windows(from.len())
        .position(|w| w == from.as_bytes())
        .expect("pattern");
    let mut v = b[..i].to_vec();
    v.extend_from_slice(to.as_bytes());
    v.extend_from_slice(&b[i + from.len()..]);
    v
}

/// A finished and a failed run write snapshots too, and restore to the same end.
#[test]
fn snapshots_of_finished_and_failed_runs() {
    let ms = models();
    for (name, cm, base) in &ms {
        let want = simoxide_sim::run(cm, base.clone(), Outputs::default());
        let mut sim = Simulation::<Exact>::create(cm, base.clone(), Outputs::default()).unwrap();
        let _ = sim.run_until(Pause::Never);
        let mut bytes = Vec::new();
        sim.snapshot_to(&mut bytes).unwrap();
        let r = Simulation::<Exact>::restore_from(&bytes[..], cm, Outputs::default())
            .unwrap()
            .finish();
        match (&want, &r) {
            (Ok(_), Ok(_)) => assert_eq!(fingerprint(&want), fingerprint(&r), "{name}"),
            (Err(a), Err(b)) => assert_eq!(a.message, b.message, "{name}"),
            _ => panic!("{name}: {} vs {}", fingerprint(&want), fingerprint(&r)),
        }
    }
}

//! Pause, snapshots, restore and fork of the Slingshot-exact engine (`DESIGN-SPD.md` 5.7,
//! "ssx snapshots as built"; `docs/formats/snapshot.md`, section `ssx`).
//!
//! Exit criterion: on every case of `corpus-ss/` and `corpus-spd/`, a run paused at several
//! points (event counts, times), written to a snapshot at each pause and restored into a fresh
//! run each time (one hop as JSON, the last hop on another thread), gives the level-E
//! projection, the tape projection with positions, the measurements, the handler exceptions,
//! the counts and (SPD cases) the decision log of the uninterrupted run, byte for byte; the
//! same for forks and detached runs; a restored state writes the bytes it was read from.

use simoxide_reconf::{QvtoAdjuster, QvtoCore, SpdAdjust};
use simoxide_sim::snapshot::{Encoding, Section, Snapshot};
use simoxide_sim::ssx::{SsxRun, SsxState};
use simoxide_sim::{CompiledModel, FlavourConfig, Outputs, Pause, RunSpec, SimConfig, Status};
use simoxide_spd::Spd;
use simoxide_spd::log::VecLog;
use simoxide_spd::ssx::SsxSpd;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
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
        String::from_utf8(self.0.borrow().clone()).unwrap()
    }
}

/// Outputs into two buffers.
fn outputs() -> (Outputs, Buf, Buf) {
    let (p, t) = (Buf::default(), Buf::default());
    (
        Outputs {
            trace: Some(Box::new(p.clone())),
            tape: Some(Box::new(t.clone())),
        },
        p,
        t,
    )
}

struct Case {
    name: String,
    cm: CompiledModel,
    cfg: SimConfig,
}

fn load(dir: &Path) -> Case {
    let case: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("case.json")).unwrap()).unwrap();
    let model_dir = root().join(case["model"].as_str().unwrap());
    let model = RunSpec::default()
        .load_model(&model_dir)
        .expect("model loads");
    let cm = CompiledModel::compile_flavour(model, FlavourConfig::slingshot_exact()).unwrap();
    let cfg = SimConfig {
        seed: case["seed"].as_i64().unwrap(),
        max_sim_time: case["sim_time"].as_i64().unwrap(),
        max_measurements: -1,
        ..Default::default()
    };
    Case {
        name: dir.file_name().unwrap().to_string_lossy().into_owned(),
        cm,
        cfg,
    }
}

fn spd_of(cm: &CompiledModel) -> Option<Spd> {
    simoxide_spd::sim::spd_of(&cm.model, simoxide_spd::Quirks::slingshot()).unwrap()
}

fn adjuster(core: &QvtoCore, spd: &Spd) -> QvtoAdjuster {
    QvtoAdjuster::spd(core, &SpdAdjust::new(&spd.quirks())).unwrap()
}

/// Everything a run writes.
#[derive(Debug, Default, PartialEq)]
struct Out {
    projection: String,
    tape: String,
    csv: Option<String>,
    error: Option<String>,
    warnings: Vec<String>,
    events: u64,
    uniforms: u64,
    main_count: i64,
    spd_log: String,
}

impl Out {
    fn add(&mut self, o: Out) {
        self.projection.push_str(&o.projection);
        self.tape.push_str(&o.tape);
        self.spd_log.push_str(&o.spd_log);
        self.csv = o.csv;
        self.error = o.error;
        self.warnings = o.warnings;
        self.events = o.events;
        self.uniforms = o.uniforms;
        self.main_count = o.main_count;
    }

    fn diff(&self, other: &Out) -> Option<String> {
        let first = |a: &str, b: &str| {
            a.lines()
                .zip(b.lines())
                .position(|(x, y)| x != y)
                .map_or_else(
                    || format!("lengths {} vs {}", a.len(), b.len()),
                    |i| format!("line {}", i + 1),
                )
        };
        if self.projection != other.projection {
            return Some(format!(
                "projection ({})",
                first(&self.projection, &other.projection)
            ));
        }
        if self.tape != other.tape {
            return Some(format!("tape ({})", first(&self.tape, &other.tape)));
        }
        if self.spd_log != other.spd_log {
            return Some(format!(
                "spd log ({})",
                first(&self.spd_log, &other.spd_log)
            ));
        }
        if self.csv != other.csv {
            return Some("measurements".into());
        }
        if self != other {
            return Some(format!(
                "result: {:?}/{:?} vs {:?}/{:?}, events {} vs {}",
                self.error, self.warnings, other.error, other.warnings, self.events, other.events
            ));
        }
        None
    }
}

/// The uninterrupted run (the established entry points `ssx::run`, `ssx::run_with_spd`).
fn uninterrupted(c: &Case, core: &QvtoCore) -> Out {
    let (out, p, t) = outputs();
    let spd = spd_of(&c.cm);
    let mut log = VecLog::default();
    let r = match &spd {
        Some(spd) => {
            let mut adj = adjuster(core, spd);
            let mut hooks = SsxSpd::new(spd, &c.cm.model, &mut adj, &mut log);
            simoxide_sim::ssx::run_with_spd(&c.cm, c.cfg.clone(), out, &mut hooks)
        }
        None => simoxide_sim::ssx::run(&c.cm, c.cfg.clone(), out),
    };
    let mut o = Out {
        projection: p.text(),
        tape: t.text(),
        spd_log: log.to_jsonl(),
        ..Out::default()
    };
    match r {
        Ok(r) => {
            o.csv = Some(r.measurements.to_csv());
            o.warnings = r.warnings;
            o.events = r.events;
            o.uniforms = r.uniforms;
            o.main_count = r.main_count;
        }
        Err(e) => o.error = Some(e.to_string()),
    }
    o
}

/// One hop: a run (new, or restored from `snap`) to `pause` (`None`: the end). Returns what
/// it wrote and, when paused, the snapshot (encoded with `enc`) as bytes.
fn hop(
    c: &Case,
    core: &QvtoCore,
    snap: Option<&[u8]>,
    pause: Option<Pause>,
    enc: Encoding,
) -> (Out, Option<Vec<u8>>) {
    let (out, p, t) = outputs();
    let spd = spd_of(&c.cm);
    let snap = snap.map(|b| Snapshot::read(b).unwrap());
    let mut o = Out::default();
    let mut run = match &snap {
        Some(s) => SsxRun::restore_snapshot(s, &c.cm, out).unwrap(),
        None => match SsxRun::new(&c.cm, c.cfg.clone(), out) {
            Ok(r) => r,
            Err(e) => {
                o.projection = p.text();
                o.tape = t.text();
                o.error = Some(e.to_string());
                return (o, None);
            }
        },
    };
    let mut log = VecLog::default();
    let mut adj = spd.as_ref().map(|s| adjuster(core, s));
    if let (Some(a), Some(s)) = (&mut adj, &snap) {
        a.run.restore_section(&run, s).unwrap();
    }
    let mut hooks = match (&spd, &mut adj) {
        (Some(spd), Some(a)) => Some(match &snap {
            Some(s) => SsxSpd::restore(spd, s, a, &mut log).unwrap(),
            None => SsxSpd::new(spd, &c.cm.model, a, &mut log),
        }),
        _ => None,
    };
    let status = match pause {
        Some(pz) => run.run_until(
            pz,
            hooks
                .as_mut()
                .map(|x| x as &mut dyn simoxide_sim::ssx::SpdHooks),
        ),
        None => Ok(Status::Finished),
    };
    let mut bytes = None;
    match status {
        Ok(Status::Paused) => {
            let mut extra: Vec<Section> = match &hooks {
                Some(h) => h.snapshot_sections(enc).unwrap(),
                None => Vec::new(),
            };
            drop(hooks);
            if let Some(a) = &adj
                && a.run.is_started()
            {
                extra.push(a.run.snapshot_section(enc).unwrap());
            }
            let s = run.snapshot_value(enc, extra).unwrap();
            let mut b = Vec::new();
            s.write(&mut b).unwrap();
            // a restored state writes the bytes it was read from
            let again = {
                let s2 = Snapshot::read(&b[..]).unwrap();
                let mut r2 = SsxRun::restore_snapshot(&s2, &c.cm, Outputs::default()).unwrap();
                let extra2: Vec<Section> = s2
                    .sections
                    .iter()
                    .filter(|x| !["ssx", "model", "graph"].contains(&x.name.as_str()))
                    .cloned()
                    .collect();
                let mut b2 = Vec::new();
                r2.snapshot_value(enc, extra2)
                    .unwrap()
                    .write(&mut b2)
                    .unwrap();
                b2
            };
            assert!(again == b, "{}: restored state writes other bytes", c.name);
            bytes = Some(b);
        }
        Ok(Status::Finished) => {
            let r = run.finish(
                hooks
                    .as_mut()
                    .map(|x| x as &mut dyn simoxide_sim::ssx::SpdHooks),
            );
            drop(hooks);
            fill(&mut o, r);
        }
        Err(e) => {
            drop(hooks);
            o.error = Some(e.to_string());
        }
    }
    o.projection = p.text();
    o.tape = t.text();
    o.spd_log = log.to_jsonl();
    (o, bytes)
}

/// The run paused at `pauses` in turn, each hop restored from the last snapshot; hop `json`
/// as JSON, the last hop on another thread.
fn hops(c: &Case, core: &QvtoCore, pauses: &[Pause], json: usize) -> Out {
    let mut total = Out::default();
    let mut snap: Option<Vec<u8>> = None;
    for (i, &p) in pauses.iter().enumerate() {
        let enc = if i == json {
            Encoding::Json
        } else {
            Encoding::Postcard
        };
        let (o, s) = hop(c, core, snap.as_deref(), Some(p), enc);
        total.add(o);
        match s {
            Some(s) => snap = Some(s),
            None => return total,
        }
    }
    // the last hop on another thread, with its own interpreter and QVTo core
    let bytes = snap.expect("a snapshot");
    let o = std::thread::scope(|s| {
        s.spawn(|| {
            let core = QvtoCore::new().unwrap();
            hop(c, &core, Some(&bytes), None, Encoding::Postcard).0
        })
        .join()
        .unwrap()
    });
    total.add(o);
    total
}

fn cases() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for corpus in ["corpus-ss", "corpus-spd"] {
        let mut names: Vec<PathBuf> = std::fs::read_dir(root().join(corpus))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.join("case.json").exists())
            .collect();
        names.sort();
        v.extend(names);
    }
    v
}

/// The pause points of a run with `n` events and simulation time `sim_time` (s).
fn pauses(n: u64, sim_time: i64) -> Vec<Pause> {
    let s = 1_000_000_000i64;
    vec![
        Pause::Events(1),
        Pause::Events((n / 4).max(2)),
        Pause::After(sim_time * s / 2),
        // the same instant as the last pause (or before it: a hop without events)
        Pause::Events((n / 2).max(3)),
        Pause::Before(sim_time * s * 3 / 4),
        Pause::Events(n.saturating_sub(1).max(4)),
    ]
}

#[test]
fn snapshots_continue_like_the_uninterrupted_run() {
    let core = QvtoCore::new().unwrap();
    let mut failures = Vec::new();
    let mut checked = 0;
    let mut snapshots = 0;
    for dir in cases() {
        let c = load(&dir);
        let full = uninterrupted(&c, &core);
        if full.error.is_some() && full.events == 0 && full.projection.is_empty() {
            // fails before t = 0: SsxRun::new fails the same way
            let (o, s) = hop(&c, &core, None, Some(Pause::Events(1)), Encoding::Postcard);
            assert!(s.is_none(), "{}", c.name);
            if o.error != full.error {
                failures.push(format!("{}: {:?} vs {:?}", c.name, o.error, full.error));
            }
            checked += 1;
            continue;
        }
        let n = full.events.max(8);
        let ps = pauses(n, c.cfg.max_sim_time);
        snapshots += ps.len();
        let got = hops(&c, &core, &ps, 2);
        if let Some(d) = got.diff(&full) {
            failures.push(format!("{}: {d}", c.name));
        }
        eprintln!(
            "{:40} {} events, {} hops",
            c.name,
            full.events,
            ps.len() + 1
        );
        checked += 1;
    }
    eprintln!("{checked} cases, {snapshots} snapshots");
    assert!(checked >= 94, "{checked} cases");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Forks (in memory) and detached runs (another thread) continue like the uninterrupted run,
/// and so does the run they were forked from.
#[test]
fn forks_and_detached_runs_continue_like_the_uninterrupted_run() {
    let core = QvtoCore::new().unwrap();
    let mut failures = Vec::new();
    let mut checked = 0;
    for dir in cases() {
        let c = load(&dir);
        let full = uninterrupted(&c, &core);
        let spd = spd_of(&c.cm);
        let (out, p, t) = outputs();
        let Ok(mut run) = SsxRun::new(&c.cm, c.cfg.clone(), out) else {
            continue;
        };
        let k = (full.events / 2).max(1);
        let mut log = VecLog::default();
        let mut adj = spd.as_ref().map(|s| adjuster(&core, s));
        let mut hooks = match (&spd, &mut adj) {
            (Some(spd), Some(a)) => Some(SsxSpd::new(spd, &c.cm.model, a, &mut log)),
            _ => None,
        };
        let st = run.run_until(
            Pause::Events(k),
            hooks
                .as_mut()
                .map(|x| x as &mut dyn simoxide_sim::ssx::SpdHooks),
        );
        if !matches!(st, Ok(Status::Paused)) {
            continue;
        }
        // the SPD layer of every continuation: from the run's snapshot sections
        let snap = hooks.as_ref().map(|h| {
            let extra = h.snapshot_sections(Encoding::Postcard).unwrap();
            run.snapshot_value(Encoding::Postcard, extra).unwrap()
        });
        drop(hooks);
        let snap = snap.map(|mut s| {
            if let Some(a) = &adj
                && a.run.is_started()
            {
                s.sections
                    .push(a.run.snapshot_section(Encoding::Postcard).unwrap());
            }
            s
        });
        let prefix = Out {
            projection: p.text(),
            tape: t.text(),
            spd_log: log.to_jsonl(),
            ..Out::default()
        };
        // a fork in memory, a detached copy continued on another thread, the original
        let (fout, fp, ft) = outputs();
        let fork = run.fork(fout).unwrap();
        let detached: SsxState = run.detach();
        let fork_out = continue_run(&core, fork, snap.as_ref(), &spd, fp, ft);
        let d_out = std::thread::scope(|s| {
            s.spawn(|| {
                let core = QvtoCore::new().unwrap();
                let spd = spd_of(&c.cm);
                let (out, p, t) = outputs();
                let run = SsxRun::attach(&c.cm, detached, out).unwrap();
                continue_run(&core, run, snap.as_ref(), &spd, p, t)
            })
            .join()
            .unwrap()
        });
        let p0 = prefix.projection.len();
        let t0 = prefix.tape.len();
        let mut orig = continue_run(&core, run, snap.as_ref(), &spd, p.clone(), t.clone());
        // the original's buffers hold the prefix too
        orig.projection = orig.projection[p0..].to_string();
        orig.tape = orig.tape[t0..].to_string();
        for (what, o) in [("fork", fork_out), ("detached", d_out), ("original", orig)] {
            let mut a = Out {
                projection: prefix.projection.clone(),
                tape: prefix.tape.clone(),
                spd_log: prefix.spd_log.clone(),
                ..Out::default()
            };
            a.add(o);
            if let Some(d) = a.diff(&full) {
                failures.push(format!("{} ({what}): {d}", c.name));
            }
        }
        checked += 1;
    }
    eprintln!("{checked} cases forked");
    assert!(checked >= 90, "{checked} cases");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn fill(o: &mut Out, r: Result<simoxide_sim::RunResult, simoxide_sim::SimError>) {
    match r {
        Ok(r) => {
            o.csv = Some(r.measurements.to_csv());
            o.warnings = r.warnings;
            o.events = r.events;
            o.uniforms = r.uniforms;
            o.main_count = r.main_count;
        }
        Err(e) => o.error = Some(e.to_string()),
    }
}

/// Finishes `run` (its SPD layer and adjuster restored from `snap`'s sections), with its
/// outputs `p`, `t`.
fn continue_run(
    core: &QvtoCore,
    run: SsxRun<'_>,
    snap: Option<&Snapshot>,
    spd: &Option<Spd>,
    p: Buf,
    t: Buf,
) -> Out {
    let mut log = VecLog::default();
    let mut adj = spd.as_ref().map(|s| adjuster(core, s));
    if let (Some(a), Some(s)) = (&mut adj, snap) {
        a.run.restore_section(&run, s).unwrap();
    }
    let mut hooks = match (spd, &mut adj, snap) {
        (Some(spd), Some(a), Some(s)) => Some(SsxSpd::restore(spd, s, a, &mut log).unwrap()),
        _ => None,
    };
    let r = run.finish(
        hooks
            .as_mut()
            .map(|x| x as &mut dyn simoxide_sim::ssx::SpdHooks),
    );
    drop(hooks);
    let mut o = Out {
        projection: p.text(),
        tape: t.text(),
        spd_log: log.to_jsonl(),
        ..Out::default()
    };
    fill(&mut o, r);
    o
}

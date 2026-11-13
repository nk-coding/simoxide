//! `simoxide resume --actions-at clock|boundary` (#18, `docs/guide/snapshots.md`, "Actions at
//! the boundary") in new processes: after a timed pause, `boundary` applies the resume's
//! actions (`--change`, `--enforce`, ...) at the snapshot's logical boundary (the time
//! `--pause-at` or `--pause-before` asked for) instead of its clock (the time of its last
//! event); `clock`, the default, is the behaviour before. Without actions, and after a pause
//! that is not timed, `boundary` changes nothing.
//!
//! Most checks use Slingshot's minimal example in the Slingshot flavour without its policy: an
//! arrival every 3 s served for 2 s, so the events are at `3k` and `3k + 2` s: paused at 49.5
//! s, its last event is at 48 s and its next one at 50 s.

#![cfg(feature = "reconfiguration")]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("actions-at-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn sx(args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_simoxide"))
        .args(args)
        .output()
        .expect("simoxide runs");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn ok(args: &[&str]) -> String {
    let (s, err) = sx(args);
    assert!(s, "simoxide {args:?}: {err}");
    err
}

fn fails(args: &[&str]) -> String {
    let (s, err) = sx(args);
    assert!(!s, "simoxide {args:?} succeeded: {err}");
    err
}

fn json_file(p: &Path) -> Value {
    let text = std::fs::read_to_string(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    serde_json::from_str(&text).unwrap()
}

fn bytes(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn minimal() -> String {
    root()
        .join("reference/slingshot/models/spd_minimal")
        .to_string_lossy()
        .into_owned()
}

/// A report's time `k` of its interval, in nanoseconds (`null`: `None`).
fn ns(r: &Value, k: &str) -> Option<i64> {
    r["interval"][k]["time_ns"].as_i64()
}

/// The minimal example paused by `pause` (e.g. `["--pause-at", "49.5"]`), its snapshot `p.snap`.
fn parent(d: &Path, pause: &[&str]) -> String {
    let snap = d.join("p.snap").to_string_lossy().into_owned();
    let m = minimal();
    let mut a = vec![
        "run",
        "--model",
        &m,
        "--flavour",
        "slingshot",
        "--no-spd",
        "--max-sim-time",
        "100",
        "--snapshot",
        &snap,
    ];
    a.extend(pause);
    ok(&a);
    snap
}

/// `resume snap` with `extra`, into the files `tag.*` of `d`: (stderr, report, the runtime
/// file's change times, the snapshot written's bytes).
fn resume(d: &Path, snap: &str, tag: &str, extra: &[&str]) -> (String, Value, Vec<i64>, Vec<u8>) {
    let p = |f: &str| d.join(format!("{tag}.{f}")).to_string_lossy().into_owned();
    let (report, runtime, child) = (p("report.json"), p("runtime.json"), p("snap"));
    let m = minimal();
    let mut a = vec![
        "resume",
        snap,
        "--model",
        &m,
        "--pause-at",
        "60",
        "--snapshot",
        &child,
        "--report",
        &report,
        "--runtime-metadata",
        &runtime,
    ];
    a.extend(extra);
    let err = ok(&a);
    let changes = json_file(Path::new(&runtime))["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["time_ns"].as_i64().unwrap())
        .collect();
    (
        err,
        json_file(Path::new(&report)),
        changes,
        bytes(Path::new(&child)),
    )
}

const S: i64 = 1_000_000_000;

/// After `--pause-at 49.5` (last event 48 s), a change file applies at 48 s by default and with
/// `clock`, at 49.5 s with `boundary`; the report says when and in which mode, the interval
/// still starts at 49.5 s. A path through a snapshot written after the actions continues like
/// the invocation that went on (the moved clock is run state).
#[test]
fn a_change_applies_at_the_boundary() {
    let d = tmp("change");
    let snap = parent(&d, &["--pause-at", "49.5"]);
    let changes = d.join("c.changes");
    std::fs::write(&changes, "rate AppServer:CPU 2\n").unwrap();
    let c = changes.to_string_lossy().into_owned();
    let (_, def, def_t, def_snap) = resume(&d, &snap, "default", &["--change", &c]);
    let (_, clock, clock_t, clock_snap) = resume(
        &d,
        &snap,
        "clock",
        &["--change", &c, "--actions-at", "clock"],
    );
    let (err, b, b_t, b_snap) = resume(
        &d,
        &snap,
        "boundary",
        &["--change", &c, "--actions-at", "boundary"],
    );
    assert_eq!(def_t, [48 * S]);
    assert_eq!(clock_t, [48 * S]);
    assert_eq!(b_t, [49 * S + S / 2]);
    assert!(
        err.contains("actions at t=49.5 s: the interval boundary"),
        "{err}"
    );
    assert_eq!(def_snap, clock_snap, "clock is the default");
    for r in [&def, &clock, &b] {
        assert_eq!(ns(r, "start"), Some(49 * S + S / 2));
        assert_eq!(ns(r, "end"), Some(60 * S));
    }
    assert_eq!(ns(&def, "actions"), Some(48 * S));
    assert_eq!(ns(&b, "actions"), Some(49 * S + S / 2));
    assert_eq!(def["behaviour"]["actions_at"], "clock");
    assert_eq!(b["behaviour"]["actions_at"], "boundary");

    // a two-hop path: the first hop applies the change at the boundary and pauses at 55 s, the
    // second goes on to 60 s: the same snapshot as the one hop
    let p = |f: &str| d.join(f).to_string_lossy().into_owned();
    let m = minimal();
    ok(&[
        "resume",
        &snap,
        "--model",
        &m,
        "--change",
        &c,
        "--actions-at",
        "boundary",
        "--pause-at",
        "55",
        "--snapshot",
        &p("hop1.snap"),
    ]);
    ok(&[
        "resume",
        &p("hop1.snap"),
        "--model",
        &m,
        "--pause-at",
        "60",
        "--snapshot",
        &p("hop2.snap"),
    ]);
    let state = |b: &[u8]| {
        // the run state: the headers differ in `parent` (the path) only
        let f = d.join("x.snap");
        std::fs::write(&f, b).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_simoxide"))
            .args(["snapshot-info", &f.to_string_lossy()])
            .output()
            .unwrap();
        let mut h: Value = serde_json::from_slice(&out.stdout).unwrap();
        h.as_object_mut().unwrap().remove("parent");
        h
    };
    assert_eq!(state(&bytes(&d.join("hop2.snap"))), state(&b_snap));
}

/// Without actions `boundary` changes nothing: the same snapshot, report times and no clock
/// move; a pause condition `elapsed` counts from the clock the run continued from.
#[test]
fn without_actions_nothing_changes() {
    let d = tmp("none");
    let snap = parent(&d, &["--pause-at", "49.5"]);
    let (_, a, _, a_snap) = resume(&d, &snap, "plain", &[]);
    let (err, b, _, b_snap) = resume(&d, &snap, "boundary", &["--actions-at", "boundary"]);
    assert_eq!(a_snap, b_snap);
    assert!(!err.contains("actions at"), "{err}");
    assert_eq!(a["interval"], b["interval"]);
    assert_eq!(a["interval"]["actions"], Value::Null);

    // `elapsed 10`: from the clock, 48 s, without the move; from 49.5 s after it
    let changes = d.join("c.changes");
    std::fs::write(&changes, "rate AppServer:CPU 2\n").unwrap();
    let c = changes.to_string_lossy().into_owned();
    let (_, e1, _, _) = resume(
        &d,
        &snap,
        "elapsed-clock",
        &["--change", &c, "--pause-when", "elapsed 10"],
    );
    let (_, e2, _, _) = resume(
        &d,
        &snap,
        "elapsed-boundary",
        &[
            "--change",
            &c,
            "--actions-at",
            "boundary",
            "--pause-when",
            "elapsed 10",
        ],
    );
    assert_eq!(ns(&e1, "end"), Some(58 * S));
    assert_eq!(ns(&e2, "end"), Some(59 * S + S / 2));
}

/// After a pause that is not timed (`--pause-events`) the boundary is the clock: `boundary`
/// applies the actions there, as `clock` does, and says so. After `--pause-before 49.5` the
/// boundary is 49.5 s.
#[test]
fn untimed_pauses_and_pause_before() {
    let d = tmp("untimed");
    let snap = parent(&d, &["--pause-events", "148"]);
    let changes = d.join("c.changes");
    std::fs::write(&changes, "rate AppServer:CPU 2\n").unwrap();
    let c = changes.to_string_lossy().into_owned();
    let (_, a, a_t, a_snap) = resume(&d, &snap, "clock", &["--change", &c]);
    let (err, b, b_t, b_snap) = resume(
        &d,
        &snap,
        "boundary",
        &["--change", &c, "--actions-at", "boundary"],
    );
    assert!(err.contains("ends at its clock"), "{err}");
    assert_eq!(a_snap, b_snap);
    assert_eq!(a_t, b_t);
    assert_eq!(ns(&a, "start"), ns(&a, "actions"));
    assert_eq!(ns(&b, "actions"), ns(&a, "actions"));

    let d = tmp("before");
    let snap = parent(&d, &["--pause-before", "49.5"]);
    let (_, r, t, _) = resume(
        &d,
        &snap,
        "boundary",
        &["--change", &c, "--actions-at", "boundary"],
    );
    assert_eq!(t, [49 * S + S / 2]);
    assert_eq!(ns(&r, "actions"), Some(49 * S + S / 2));
}

/// The Slingshot-exact engine moves its clock alike (a change file; no runtime file there).
#[test]
fn the_slingshot_exact_engine() {
    let d = tmp("ssx");
    let p = |f: &str| d.join(f).to_string_lossy().into_owned();
    let m = minimal();
    ok(&[
        "run",
        "--model",
        &m,
        "--flavour",
        "slingshot-exact",
        "--no-spd",
        "--max-sim-time",
        "100",
        "--pause-at",
        "49.5",
        "--snapshot",
        &p("p.snap"),
    ]);
    std::fs::write(d.join("c.changes"), "rate AppServer:CPU 2\n").unwrap();
    let go = |mode: &str| -> Value {
        ok(&[
            "resume",
            &p("p.snap"),
            "--model",
            &m,
            "--change",
            &p("c.changes"),
            "--actions-at",
            mode,
            "--pause-at",
            "60",
            "--report",
            &p(&format!("{mode}.json")),
        ]);
        json_file(Path::new(&p(&format!("{mode}.json"))))
    };
    assert_eq!(ns(&go("clock"), "actions"), Some(48 * S));
    assert_eq!(ns(&go("boundary"), "actions"), Some(49 * S + S / 2));
}

/// `--actions-at` is an option of `resume`, with the values `clock` and `boundary`.
#[test]
fn refused_uses() {
    let m = minimal();
    let err = fails(&["run", "--model", &m, "--actions-at", "boundary"]);
    assert!(err.contains("an option of resume"), "{err}");
    let err = fails(&["resume", "x.snap", "--model", &m, "--actions-at", "later"]);
    assert!(err.contains("expected clock or boundary"), "{err}");
}

/// The issue's reproduction (#18): the node-based example of `corpus-spd`, paused at 50 s after
/// its last event at 49.995 113 990 s, an enforced scale-out at the resume: at the clock by
/// default, at 50 s with `boundary`. With `boundary` no tuple of the continuation lies before
/// the interval's start.
#[cfg(feature = "spd")]
#[test]
fn enforce_at_the_boundary() {
    let d = tmp("issue");
    let p = |f: &str| d.join(f).to_string_lossy().into_owned();
    let model = root()
        .join("corpus-spd/rmuc_nodebased")
        .to_string_lossy()
        .into_owned();
    ok(&[
        "run",
        "--model",
        &model,
        "--flavour",
        "slingshot",
        "--mode",
        "fast",
        "--quirks",
        "none",
        "--spd-quirks",
        "fixed",
        "--seed",
        "42",
        "--max-sim-time",
        "60",
        "--max-measurements",
        "-1",
        "--measurements-scope",
        "interval",
        "--pause-at",
        "50",
        "--snapshot",
        &p("parent.snap"),
        "--report",
        &p("parent.json"),
    ]);
    let parent = json_file(Path::new(&p("parent.json")));
    assert_eq!(ns(&parent, "last_event"), Some(49_995_113_990));
    assert_eq!(ns(&parent, "end"), Some(50 * S));
    let parent_snap = p("parent.snap");
    let go = |tag: &str, extra: &[&str]| -> (Value, Value, String) {
        let mut a = vec![
            "resume",
            &parent_snap,
            "--model",
            &model,
            "--enforce",
            "_ZZp6II1lEe6Vt_T2z50ung",
            "--pause-at",
            "51",
            "--snapshot",
        ];
        let (snap, report, runtime, csv, log) = (
            p(&format!("{tag}.snap")),
            p(&format!("{tag}.json")),
            p(&format!("{tag}-runtime.json")),
            p(&format!("{tag}.csv")),
            p(&format!("{tag}.jsonl")),
        );
        a.extend([
            snap.as_str(),
            "--report",
            &report,
            "--runtime-metadata",
            &runtime,
            "--measurements",
            &csv,
            "--spd-log",
            &log,
        ]);
        a.extend(extra);
        ok(&a);
        let log = std::fs::read_to_string(&log).unwrap();
        let adjust: Value = log
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|r| r["enforced"] == true)
            .expect("the enforced adjustment");
        (
            json_file(Path::new(&report)),
            json_file(Path::new(&runtime)),
            std::fs::read_to_string(&csv).unwrap() + &adjust.to_string(),
        )
    };
    let earliest = |csv: &str| -> f64 {
        csv.lines()
            .skip(1)
            .filter_map(|l| l.split(',').nth(2)?.parse::<f64>().ok())
            .fold(f64::INFINITY, f64::min)
    };
    let (r, rt, out) = go("clock", &[]);
    assert_eq!(ns(&r, "start"), Some(50 * S));
    assert_eq!(rt["changes"][0]["time_ns"], 49_995_113_990i64);
    assert_eq!(ns(&r, "actions"), Some(49_995_113_990));
    assert!(earliest(&out) < 50.0, "the scale-out records before 50 s");
    let (r, rt, out) = go("boundary", &["--actions-at", "boundary"]);
    assert_eq!(ns(&r, "start"), Some(50 * S));
    assert_eq!(rt["changes"][0]["time_ns"], 50 * S);
    assert_eq!(rt["changes"][0]["cause"]["type"], "enforce");
    assert_eq!(ns(&r, "actions"), Some(50 * S));
    assert!(earliest(&out) >= 50.0, "a tuple before the boundary");
    assert!(out.contains("\"t\":50.0"), "{out}");
    assert_eq!(r["counters"]["invocation"]["adjustments"], 1);
}

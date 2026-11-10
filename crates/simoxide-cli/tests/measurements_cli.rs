//! `--measurements-scope interval` (`docs/guide/snapshots.md`, "Measurement intervals") in new
//! processes: a run continued over twenty intervals exports every tuple exactly once (the
//! exports of a path concatenate to the uninterrupted run's `measurements.csv`, series by
//! series), its snapshots hold no rows (smaller than the cumulative ones by exactly the rows'
//! bytes), a request that ends after a resume is the continuation's tuple, the snapshot headers
//! link the path (`state`, `parent`, the cursor `measurements.tuples`), and several
//! continuations of one snapshot leave it unchanged.

#![cfg(feature = "spd")]

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("meas-cli-{name}"));
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
    let (good, err) = sx(args);
    assert!(good, "simoxide {args:?}: {err}");
    err
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

/// The data lines of a `measurements.csv` per series (`measuring_point,metric`), in file order.
fn series(csv: &str) -> BTreeMap<String, Vec<String>> {
    let mut lines = csv.lines();
    assert_eq!(lines.next(), Some("measuring_point,metric,time,value"));
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for l in lines {
        let f: Vec<&str> = l.rsplitn(3, ',').collect();
        m.entry(f[2].to_string()).or_default().push(l.to_string());
    }
    m
}

/// The tuples `metric,time,value` of `series`, sorted. Resource containers that an adjustment
/// creates after a resume get other element ids than in the uninterrupted run (the same on every
/// path through the same pauses), so their measuring-point keys are left out: the tuples are the
/// same.
fn observations(series: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let mut v: Vec<String> = series
        .values()
        .flatten()
        .map(|l| l.splitn(2, "],").nth(1).unwrap().to_string())
        .collect();
    v.sort();
    v
}

fn rows(csv: &str) -> u64 {
    csv.lines().count() as u64 - 1
}

/// The snapshot's header (`simoxide snapshot-info`).
fn header(p: &str) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_simoxide"))
        .args(["snapshot-info", p])
        .output()
        .unwrap();
    assert!(out.status.success());
    serde_json::from_slice(&out.stdout).unwrap()
}

fn engine_bytes(h: &Value) -> u64 {
    h["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "engine")
        .unwrap()["bytes"]
        .as_u64()
        .unwrap()
}

/// The interval line of the CLI: (scope, from, to, rows).
fn interval(err: &str) -> (String, u64, u64, u64) {
    let l = err
        .lines()
        .find(|l| l.starts_with("measurements: "))
        .unwrap_or_else(|| panic!("no interval line: {err}"));
    let field = |k: &str| {
        l.split_whitespace()
            .find_map(|w| w.strip_prefix(k))
            .unwrap_or_else(|| panic!("{k} in {l}"))
            .to_string()
    };
    let (from, to) = field("tuples=")
        .split_once("..")
        .map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap()))
        .unwrap();
    (field("scope="), from, to, field("rows=").parse().unwrap())
}

/// `postcard`'s varint length of `n`.
fn varint_len(mut n: u64) -> u64 {
    let mut k = 1;
    while n >= 0x80 {
        n >>= 7;
        k += 1;
    }
    k
}

/// The issue's scenario: the V4 model in the Slingshot flavour, fast mode, fixed SPD semantics
/// (CPU utilisation policies scale the group out and in), paused nineteen times (every 30 s)
/// and continued to the end at 600 s: twenty intervals.
#[test]
fn twenty_intervals_export_every_tuple_once() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let d = tmp("twenty");
    let p = |n: &str| d.join(n).to_string_lossy().into_owned();
    let model = root()
        .join("reference/slingshot/models/spd_v4_cpu")
        .to_string_lossy()
        .into_owned();
    let common = [
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
        "7",
        "--max-sim-time",
        "600",
    ];
    // the uninterrupted run
    let mut a = vec!["run"];
    a.extend(common);
    let (wm, wl) = (p("whole.csv"), p("whole.jsonl"));
    a.extend(["--measurements", &wm, "--spd-log", &wl]);
    let err = ok(&a);
    let whole = read(Path::new(&wm));
    let total = rows(&whole);
    assert!(
        err.contains(&format!(" measurements={total} ")),
        "the tuple count is the number of rows: {err}"
    );
    if cfg!(feature = "reconfiguration") {
        assert!(!err.contains("adjusted=0"), "the policies adjust: {err}");
    }

    // twenty intervals in interval scope, and the same pauses in cumulative scope
    let n: usize = 20;
    let mut prev_to = 0;
    let mut prev_state: Option<String> = None;
    let mut joined: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut log = String::new();
    let (mut sum_interval, mut sum_cumulative) = (0, 0);
    let mut spanning = 0;
    for i in 0..n {
        let last = i == n - 1;
        let pause = format!("{}", 30 * (i + 1));
        for scope in ["interval", "cumulative"] {
            let (m, l, s) = (
                p(&format!("{scope}-{i}.csv")),
                p(&format!("{scope}-{i}.jsonl")),
                p(&format!("{scope}-{i}.snap")),
            );
            let prev = p(&format!("{scope}-{}.snap", i.wrapping_sub(1)));
            let mut a: Vec<&str> = if i == 0 {
                let mut a = vec!["run"];
                a.extend(common);
                a.extend(["--measurements-scope", scope]);
                a
            } else {
                // the scope comes from the snapshot
                vec!["resume", &prev, "--model", &model]
            };
            a.extend(["--measurements", &m, "--spd-log", &l]);
            if !last {
                a.extend(["--pause-at", &pause, "--snapshot", &s]);
            }
            let err = ok(&a);
            let csv = read(Path::new(&m));
            let (sc, from, to, r) = interval(&err);
            assert_eq!(sc, scope);
            assert_eq!(r, rows(&csv), "{scope} {i}: rows");
            if scope == "cumulative" {
                // the issue's observation: every export repeats the history
                assert_eq!(rows(&csv), to, "{scope} {i}");
                sum_cumulative += rows(&csv);
                if !last {
                    // the same point of the same run, with the rows
                    let hc = header(&s);
                    let hi = header(&p(&format!("interval-{i}.snap")));
                    assert_eq!(hc["measurements"]["tuples"], to);
                    assert_eq!(hc["measurements"]["rows_from"], 0);
                    assert_eq!(hi["measurements"]["tuples"], to);
                    // exactly the rows' bytes: per series 16 bytes a row, and the varint of its
                    // row count instead of the single byte of an empty list
                    let held: u64 = joined
                        .values()
                        .map(|v| 16 * v.len() as u64 + varint_len(v.len() as u64) - 1)
                        .sum();
                    assert_eq!(engine_bytes(&hc) - engine_bytes(&hi), held, "{i}");
                }
                continue;
            }
            // interval scope: tuples prev_to..to, each once
            assert_eq!(from, prev_to, "{i}: adjacent");
            assert_eq!(r, to - from, "{i}");
            sum_interval += rows(&csv);
            let cut = 30.0 * i as f64;
            for (k, v) in series(&csv) {
                if k.contains("Response Time") {
                    // a request started before the resume, ended after it
                    spanning += v
                        .iter()
                        .filter(|l| {
                            let f: Vec<f64> = l
                                .rsplitn(3, ',')
                                .take(2)
                                .map(|x| x.parse().unwrap())
                                .collect();
                            i > 0 && f[1] - f[0] < cut
                        })
                        .count();
                }
                joined.entry(k).or_default().extend(v);
            }
            log += &read(Path::new(&l));
            prev_to = to;
            if !last {
                let h = header(&s);
                assert_eq!(h["measurements"]["tuples"], to);
                assert_eq!(h["measurements"]["rows_from"], to, "no rows kept");
                assert_eq!(h["parent"].as_str(), prev_state.as_deref(), "{i}");
                prev_state = Some(h["state"].as_str().unwrap().to_string());
            }
        }
    }
    // every tuple once: the exports concatenate, series by series, to the cumulative export of
    // the same path, and hold the uninterrupted run's observations
    assert_eq!(
        joined,
        series(&read(Path::new(&p("cumulative-19.csv")))),
        "the exports concatenate to the cumulative one"
    );
    assert_eq!(observations(&joined), observations(&series(&whole)));
    assert_eq!(sum_interval, total);
    assert_eq!(prev_to, total);
    assert!(sum_cumulative > 5 * total, "{sum_cumulative} vs {total}");
    assert_eq!(
        log,
        read(Path::new(&wl)),
        "the decisions are the uninterrupted run's"
    );
    assert!(spanning > 0, "a request spans a resume");

    // a cumulative snapshot continued in interval scope: the same interval, the same state
    let err = ok(&[
        "resume",
        &p("cumulative-9.snap"),
        "--model",
        &model,
        "--measurements-scope",
        "interval",
        "--measurements",
        &p("switched.csv"),
        "--pause-at",
        "330",
        "--snapshot",
        &p("switched.snap"),
    ]);
    assert_eq!(interval(&err).0, "interval");
    assert_eq!(
        read(Path::new(&p("switched.csv"))),
        read(Path::new(&p("interval-10.csv")))
    );
    let (hs, hi) = (header(&p("switched.snap")), header(&p("interval-10.snap")));
    assert_eq!(
        hs["state"], hi["state"],
        "the state id does not depend on the path"
    );
    assert_ne!(hs["parent"], hi["parent"]);
    // an interval snapshot cannot continue cumulatively
    let (good, err) = sx(&[
        "resume",
        &p("interval-9.snap"),
        "--model",
        &model,
        "--measurements-scope",
        "cumulative",
    ]);
    assert!(
        !good && err.contains("--measurements-scope interval"),
        "{err}"
    );
    std::fs::remove_dir_all(&d).unwrap();
}

/// Two continuations of one snapshot: the snapshot is not changed, both number their tuples
/// from its cursor on, and each one's export is its own branch's.
#[test]
fn two_children_continue_one_parent() {
    let d = tmp("branches");
    let p = |n: &str| d.join(n).to_string_lossy().into_owned();
    let model = root()
        .join("corpus/h06_closed_think")
        .to_string_lossy()
        .into_owned();
    let base = [
        "--model",
        &model,
        "--max-measurements",
        "1000",
        "--seed",
        "3",
    ];
    let mut a = vec!["run"];
    a.extend(base);
    let wm = p("whole.csv");
    a.extend(["--measurements", &wm]);
    ok(&a);
    let mut a = vec!["run"];
    a.extend(base);
    let (m0, s0) = (p("m0.csv"), p("s0.snap"));
    a.extend([
        "--measurements-scope",
        "interval",
        "--measurements",
        &m0,
        "--pause-events",
        "1001",
        "--snapshot",
        &s0,
    ]);
    let (_, _, cursor, _) = interval(&ok(&a));
    let parent = std::fs::read(&s0).unwrap();
    let state = header(&s0)["state"].as_str().unwrap().to_string();
    // child a: to the end; child b: two more pauses by event count (adjacent intervals can
    // share a time), then to the end
    let ma = p("ma.csv");
    let err = ok(&["resume", &s0, "--model", &model, "--measurements", &ma]);
    let (scope, from, to, _) = interval(&err);
    assert_eq!((scope.as_str(), from), ("interval", cursor));
    assert!(err.contains(&format!("parent={state}")), "{err}");
    let mut prev = s0.clone();
    let mut b = String::new();
    for (i, ev) in ["2500", "2501"].iter().enumerate() {
        let (m, s) = (p(&format!("mb{i}.csv")), p(&format!("sb{i}.snap")));
        let err = ok(&[
            "resume",
            &prev,
            "--model",
            &model,
            "--measurements",
            &m,
            "--pause-events",
            ev,
            "--snapshot",
            &s,
        ]);
        assert_eq!(interval(&err).0, "interval");
        b += &read(Path::new(&m));
        prev = s;
    }
    let mb = p("mb.csv");
    let (_, _, to_b, _) = interval(&ok(&[
        "resume",
        &prev,
        "--model",
        &model,
        "--measurements",
        &mb,
    ]));
    assert_eq!(
        std::fs::read(&s0).unwrap(),
        parent,
        "the parent is unchanged"
    );
    assert_eq!(header(&p("sb0.snap"))["parent"].as_str(), Some(&*state));
    assert_eq!(to, to_b, "the same run, the same tuple count");
    // both branches concatenate to the whole run
    let whole = series(&read(Path::new(&wm)));
    let concat = |files: &[String]| {
        let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for f in files {
            for (k, v) in series(f) {
                m.entry(k).or_default().extend(v);
            }
        }
        m
    };
    assert_eq!(concat(&[read(Path::new(&m0)), read(Path::new(&ma))]), whole);
    let parts: Vec<String> = ["m0.csv", "mb0.csv", "mb1.csv", "mb.csv"]
        .iter()
        .map(|f| read(&d.join(f)))
        .collect();
    assert_eq!(concat(&parts), whole);
    assert!(!b.is_empty());
    std::fs::remove_dir_all(&d).unwrap();
}

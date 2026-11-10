//! `--report FILE` (`docs/guide/result-report.md`) in new processes: the interval's logical
//! end, its last event and the anchoring of continuations; every stop reason; the files a report
//! names are there and complete; a report changes no other output, and a path of reports is the
//! uninterrupted run.
//!
//! The model is Slingshot's minimal example in the Slingshot flavour: an open workload with an
//! arrival every 3 s (t = 0, 3, 6, ...), each request served for 2 s by the one server, so the
//! events are at `3k` and `3k + 2` s; deterministic, every time is known by hand.

#![cfg(feature = "spd")]

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("report-cli-{name}"));
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
        String::from_utf8_lossy(&out.stderr).into_owned() + &String::from_utf8_lossy(&out.stdout),
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

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

fn json_file(p: &Path) -> Value {
    serde_json::from_str(&read(p)).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn minimal() -> String {
    root()
        .join("reference/slingshot/models/spd_minimal")
        .to_string_lossy()
        .into_owned()
}

/// The flags of the runs without scaling policies.
const FLAGS: [&str; 8] = [
    "--flavour",
    "slingshot",
    "--mode",
    "fast",
    "--quirks",
    "none",
    "--no-spd",
    "--measurements-scope",
];

/// A time of a report.
fn t(secs: f64) -> Value {
    json!({"time": secs, "time_ns": (secs * 1e9).round() as i64})
}

/// The report's interval: (start, end, last event) in seconds.
fn times(r: &Value) -> (f64, f64, f64) {
    let i = &r["interval"];
    let s = |k: &str| i[k]["time"].as_f64().unwrap();
    (s("start"), s("end"), s("last_event"))
}

/// (kind, cause) of a report's stop.
fn stop(r: &Value) -> (&str, &str) {
    (
        r["stop"]["kind"].as_str().unwrap(),
        r["stop"]["cause"].as_str().unwrap(),
    )
}

/// The snapshot's header (`snapshot-info`).
fn header(snap: &str) -> Value {
    serde_json::from_str(&ok(&["snapshot-info", snap])).unwrap()
}

/// Every file a report names exists and is complete: its length is `bytes`; the measurements
/// have `rows` rows; JSON files parse, the trace's lines parse; the snapshot's header has the
/// report's `state`. No temporary file is left next to the report.
fn check_outputs(report: &Path) {
    let r = json_file(report);
    for (kind, f) in r["outputs"].as_object().unwrap() {
        let p = PathBuf::from(f["path"].as_str().unwrap());
        let text = std::fs::read(&p).unwrap_or_else(|e| panic!("{kind} {}: {e}", p.display()));
        assert_eq!(f["bytes"], text.len(), "{kind}: {r}");
        let text = String::from_utf8_lossy(&text);
        match kind.as_str() {
            "measurements" => {
                assert_eq!(f["rows"], text.lines().count() - 1, "{kind}");
                assert_eq!(f["rows"], r["measurements"]["rows"]);
            }
            "summary" | "decision_file" => {
                serde_json::from_str::<Value>(&text).unwrap();
            }
            "trace" | "spd_log" => {
                assert!(text.ends_with('\n'), "{kind}");
                for l in text.lines() {
                    serde_json::from_str::<Value>(l).unwrap();
                }
            }
            "snapshot" => {
                let h = header(p.to_str().unwrap());
                assert_eq!(h["state"], f["state"]);
                assert_eq!(h["state"], r["state"]);
            }
            _ => {}
        }
    }
    let dir = report.parent().unwrap();
    for e in std::fs::read_dir(dir).unwrap() {
        let n = e.unwrap().file_name().to_string_lossy().into_owned();
        assert!(!n.ends_with(".tmp"), "{n} left in {}", dir.display());
    }
}

/// Series of a `measurements.csv` with their rows, in file order.
fn series(csv: &str) -> BTreeMap<String, Vec<String>> {
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for l in csv.lines().skip(1) {
        let f: Vec<&str> = l.rsplitn(3, ',').collect();
        m.entry(f[2].to_string()).or_default().push(l.to_string());
    }
    m
}

/// The issue's case and the anchoring of continuations, hand-calculated. The run is paused at
/// 1000 s; its last event is at 999 s (an arrival, `3 * 333`; the next event is the end of that
/// request at 1001 s). Then three continuations: `--pause-before 1003` (events before 1003:
/// the last at 1002, `3 * 334`), `--pause-when 'elapsed 10'` (from the clock 1002: every event
/// at or before 1012, the last at 1011), and to the end at 1200 s. The intervals tile
/// `[0, 1200]`: 0-1000, 1000-1003, 1003-1012, 1012-1200. The chain is run with and without
/// `--report` (every other file has the same bytes) and against the uninterrupted run (the
/// joined traces and the interval exports are its trace and its measurements).
#[test]
fn intervals_tile_and_reports_change_nothing() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let d = tmp("tile");
    let p = |n: &str| d.join(n).to_string_lossy().into_owned();
    let m = minimal();
    let mut whole = vec!["run", "--model", &m];
    whole.extend(FLAGS);
    let (wt, wm) = (p("whole.trace"), p("whole.csv"));
    whole.extend(["interval", "--max-sim-time", "1200"]);
    whole.extend(["--trace", &wt, "--measurements", &wm]);
    ok(&whole);
    let steps: [&[&str]; 4] = [
        &["--pause-at", "1000"],
        &["--pause-before", "1003"],
        &["--pause-when", "elapsed 10"],
        &[],
    ];
    for tag in ["r", "n"] {
        for (k, extra) in steps.iter().enumerate() {
            let f = |n: &str| p(&format!("{tag}{k}.{n}"));
            let prev = p(&format!("{tag}{}.snap", k as i64 - 1));
            let mut a: Vec<String> = if k == 0 {
                let mut a: Vec<String> = ["run", "--model", &m].map(String::from).to_vec();
                a.extend(FLAGS.map(String::from));
                a.extend(["interval", "--max-sim-time", "1200"].map(String::from));
                a
            } else {
                ["resume", &prev, "--model", &m].map(String::from).to_vec()
            };
            a.extend(extra.iter().map(|x| x.to_string()));
            if k < 3 {
                a.extend(["--snapshot".into(), f("snap")]);
            }
            for (flag, n) in [
                ("--trace", "trace"),
                ("--measurements", "csv"),
                ("--summary", "summary"),
            ] {
                a.extend([flag.into(), f(n)]);
            }
            if tag == "r" {
                a.extend(["--report".into(), f("json")]);
            }
            ok(&a.iter().map(String::as_str).collect::<Vec<_>>());
        }
    }
    // the same bytes with and without --report
    for k in 0..4 {
        for n in ["trace", "csv", "summary", "snap"] {
            if n == "snap" && k == 3 {
                continue;
            }
            let (a, b) = (d.join(format!("r{k}.{n}")), d.join(format!("n{k}.{n}")));
            assert!(
                std::fs::read(&a).unwrap() == std::fs::read(&b).unwrap(),
                "{k}.{n}"
            );
        }
    }
    // the path is the uninterrupted run
    let joined: String = (0..4)
        .map(|k| read(&d.join(format!("r{k}.trace"))))
        .collect();
    assert!(joined == read(Path::new(&wt)), "traces");
    let mut path: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for k in 0..4 {
        for (s, rows) in series(&read(&d.join(format!("r{k}.csv")))) {
            path.entry(s).or_default().extend(rows);
        }
    }
    assert_eq!(path, series(&read(Path::new(&wm))));
    let r: Vec<Value> = (0..4)
        .map(|k| json_file(&d.join(format!("r{k}.json"))))
        .collect();
    for k in 0..4 {
        check_outputs(&d.join(format!("r{k}.json")));
    }
    // hand-calculated: (start, end, last event), the stop
    assert_eq!(times(&r[0]), (0.0, 1000.0, 999.0));
    assert_eq!(stop(&r[0]), ("pause", "pause_at"));
    assert_eq!(times(&r[1]), (1000.0, 1003.0, 1002.0));
    assert_eq!(stop(&r[1]), ("pause", "pause_before"));
    assert_eq!(times(&r[2]), (1003.0, 1012.0, 1011.0));
    assert_eq!(stop(&r[2]), ("pause", "elapsed"));
    assert_eq!(
        r[2]["stop"]["condition"],
        json!({"index": 0, "text": "elapsed 10"})
    );
    assert_eq!(times(&r[3]), (1012.0, 1200.0, 1200.0));
    assert_eq!(stop(&r[3]), ("completed", "max_sim_time"));
    assert_eq!(r[0]["interval"]["end"], t(1000.0));
    assert_eq!(r[0]["interval"]["last_event"], t(999.0));
    // the issue's numbers: the snapshot's clock is the last event, its logical end the pause
    let h = header(&p("r0.snap"));
    assert_eq!(h["time_ns"], 999_000_000_000i64);
    assert_eq!(h["interval_end"], t(1000.0));
    // they tile: each start is the previous end (seconds and nanoseconds), the cursors too
    let mut cursor = 0;
    let mut events = 0;
    for (k, x) in r.iter().enumerate() {
        assert_eq!(x["format"], "simoxide-run-report");
        assert_eq!(x["version"], 1);
        assert_eq!(x["endpoint_reached"], true, "{k}");
        if k > 0 {
            assert_eq!(x["interval"]["start"], r[k - 1]["interval"]["end"], "{k}");
            assert_eq!(x["parent"], r[k - 1]["state"], "{k}");
            assert_eq!(x["command"], "resume");
            assert!(
                x["behaviour"]["restored"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("measurements_scope"))
            );
        }
        let tu = &x["interval"]["tuples"];
        assert_eq!(tu["from"], cursor, "{k}");
        cursor = tu["to"].as_u64().unwrap();
        assert_eq!(
            x["counters"]["invocation"]["tuples"],
            cursor - tu["from"].as_u64().unwrap()
        );
        events += x["counters"]["invocation"]["events"].as_u64().unwrap();
        assert_eq!(x["counters"]["cumulative"]["events"], events, "{k}");
        assert_eq!(x["status"], if k < 3 { "paused" } else { "finished" });
        assert_eq!(x["resumable"], k < 3);
        // the interval summary has the report's interval
        let s = json_file(&d.join(format!("r{k}.summary")));
        assert_eq!(s["interval"]["tuples"], x["interval"]["tuples"], "{k}");
        assert_eq!(
            s["interval"]["time"],
            json!({"from": times(x).0, "to": times(x).1}),
            "{k}"
        );
    }
    // 400 arrivals, each with its response time and the server's state twice
    assert_eq!(
        cursor,
        r[3]["counters"]["cumulative"]["tuples"].as_u64().unwrap()
    );
    assert_eq!(r[3]["counters"]["cumulative"]["usage_scenario_runs"], 400);
    assert_eq!(r[0]["limits"]["pause"]["type"], "pause_at");
    assert_eq!(r[0]["limits"]["max_sim_time"], 1200);
    assert_eq!(r[0]["behaviour"]["measurements_scope"], "interval");
    assert_eq!(r[0]["behaviour"]["restored"], json!([]));
}

/// The stop reasons, each with its hand-calculated times.
#[test]
fn stop_reasons() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let d = tmp("stops");
    let p = |n: &str| d.join(n).to_string_lossy().into_owned();
    let m = minimal();
    let report = |name: &str, extra: &[&str], succeeds: bool| -> Value {
        let rp = p(&format!("{name}.json"));
        let mut a = vec!["run", "--model", &m];
        a.extend(FLAGS);
        a.extend(["cumulative", "--report", &rp]);
        a.extend(extra);
        if succeeds {
            ok(&a)
        } else {
            fails(&a)
        };
        check_outputs(Path::new(&rp));
        json_file(Path::new(&rp))
    };
    fn with<'a>(x: &[&'a str]) -> Vec<&'a str> {
        let mut v = vec!["--max-sim-time", "1000"];
        v.extend_from_slice(x);
        v
    }
    let (late, missing) = (p("late.snap"), p("missing/m.csv"));

    // a pause after a number of events: at the clock
    let r = report("events", &with(&["--pause-events", "10"]), true);
    assert_eq!(stop(&r), ("pause", "pause_events"));
    let (_, end, last) = times(&r);
    assert_eq!(end, last);
    assert_eq!(
        r["limits"]["pause"],
        json!({"type": "pause_events", "events": 10})
    );
    assert_eq!(r["resumable"], false, "no --snapshot");

    // a series condition: the first response time, 2 s at t = 2 s
    let r = report("series", &with(&["--pause-when", "rt[*] > 1.5"]), true);
    assert_eq!(stop(&r), ("condition", "series"));
    assert_eq!(
        r["stop"]["condition"],
        json!({"index": 0, "text": "rt[*] > 1.5"})
    );
    assert_eq!(times(&r), (0.0, 2.0, 2.0));
    assert_eq!(r["endpoint_reached"], false);

    // an SLO condition: upper 2, sensitivity 0: a response time of at least 2 s
    let r = report(
        "slo",
        &with(&[
            "--pause-at",
            "500",
            "--pause-when",
            "slo rt[*] upper 2 sensitivity 0",
        ]),
        true,
    );
    assert_eq!(stop(&r), ("condition", "slo"));
    assert_eq!(times(&r), (0.0, 2.0, 2.0));
    assert_eq!(r["endpoint_reached"], false, "before the pause at 500 s");

    // a time condition among others; --pause-at with --pause-when is the pause itself
    let r = report(
        "time",
        &with(&["--pause-when", "rt[*] > 100", "--pause-when", "time 7.5"]),
        true,
    );
    assert_eq!(stop(&r), ("pause", "pause_at"));
    assert_eq!(r["stop"]["condition"]["index"], 1);
    assert_eq!(times(&r), (0.0, 7.5, 6.0));
    let r = report(
        "at",
        &with(&["--pause-when", "rt[*] > 100", "--pause-at", "7.5"]),
        true,
    );
    assert_eq!(stop(&r), ("pause", "pause_at"));
    assert_eq!(r["stop"].get("condition"), None);
    assert_eq!(times(&r), (0.0, 7.5, 6.0));

    // the run's end: the time (Slingshot's stop is an event at 1000 s; the last request ends
    // at 998 s), the number of finished usage-scenario runs (the 10th ends at 3 * 9 + 2 = 29 s)
    let r = report("end", &with(&[]), true);
    assert_eq!(stop(&r), ("completed", "max_sim_time"));
    assert_eq!(times(&r), (0.0, 1000.0, 1000.0));
    assert_eq!(
        (r["endpoint_reached"].clone(), r["status"].clone()),
        (json!(true), json!("finished"))
    );
    let r = report("count", &with(&["--max-measurements", "10"]), true);
    assert_eq!(stop(&r), ("completed", "max_measurements"));
    assert_eq!(times(&r), (0.0, 29.0, 29.0));
    assert_eq!(r["counters"]["cumulative"]["usage_scenario_runs"], 10);

    // failures: resource limits and the timeout; nothing resumable
    let r = report("max-events", &with(&["--max-events", "100"]), false);
    assert_eq!(r["status"], "failed");
    assert_eq!(stop(&r), ("limit", "max_events"));
    assert_eq!(r["resumable"], false);
    assert_eq!(r["endpoint_reached"], false);
    let (_, end, last) = times(&r);
    assert!(end == last && last > 0.0 && last < 1000.0, "{r}");
    let r = report(
        "timeout",
        &["--max-sim-time", "1000000000", "--timeout", "0.05"],
        false,
    );
    assert_eq!(stop(&r), ("limit", "timeout"));
    assert_eq!(r["limits"]["timeout"], 0.05);
    assert_eq!(r["outputs"], json!({}));
    // an invocation error
    let r = report("bad", &with(&["--decide", "_x"]), false);
    assert_eq!(stop(&r), ("error", "invocation"));
    assert_eq!(r["interval"]["end"], Value::Null);

    // a failure after the snapshot was written: the snapshot is named and resumable
    let r = report(
        "late",
        &with(&[
            "--pause-at",
            "10",
            "--snapshot",
            &late,
            "--measurements",
            &missing,
        ]),
        false,
    );
    assert_eq!(stop(&r), ("error", "invocation"));
    assert_eq!(r["resumable"], true);
    assert_eq!(r["outputs"]["snapshot"]["path"], p("late.snap"));

    // --timeout of a resume: the deadline is not part of the snapshot, it is set again
    let snap = p("late.snap");
    let rp = p("resume-timeout.json");
    let mut a = vec!["run", "--model", &m];
    a.extend(FLAGS);
    a.extend([
        "cumulative",
        "--max-sim-time",
        "1000000000",
        "--pause-at",
        "10",
        "--snapshot",
        &snap,
    ]);
    ok(&a);
    fails(&[
        "resume",
        &snap,
        "--model",
        &m,
        "--timeout",
        "0.05",
        "--report",
        &rp,
    ]);
    let r = json_file(Path::new(&rp));
    assert_eq!(stop(&r), ("limit", "timeout"));
    assert_eq!(r["interval"]["start"], t(10.0));
}

const GROUP: &str = "_DC8fM8pgEe6u3p81um7oyQ";

/// The minimal example with `policies` (ids, step, time; all active) on its elastic
/// infrastructure (1 to 3 servers).
fn model(d: &Path, policies: &[(&str, i32, f64)]) -> String {
    let src = root().join("reference/slingshot/models/spd_minimal");
    let out = d.join("model");
    std::fs::create_dir_all(&out).unwrap();
    for e in std::fs::read_dir(&src).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "spd") {
            continue;
        }
        std::fs::copy(&p, out.join(p.file_name().unwrap())).unwrap();
    }
    let mut spd = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<spd:SPD xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:adjustments="http://palladiosimulator.org/ScalingPolicyDefinition/Adjustments/1.0" xmlns:expectations="http://palladiosimulator.org/ScalingPolicyDefinition/Triggers/Expectations/1.0" xmlns:spd="http://palladiosimulator.org/ScalingPolicyDefinition/1.0" xmlns:stimuli="http://palladiosimulator.org/ScalingPolicyDefinition/Triggers/Stimuli/1.0" xmlns:target="http://palladiosimulator.org/ScalingPolicyDefinition/Constraints/Target/1.0" xmlns:targets="http://palladiosimulator.org/ScalingPolicyDefinition/Targets/1.0" xmlns:triggers="http://palladiosimulator.org/ScalingPolicyDefinition/Triggers/1.0" id="_PKdecLwZEe6i2ICnm9lfkw">
"#,
    );
    for &(id, step, at) in policies {
        spd.push_str(&format!(
            r#"  <scalingPolicies id="{id}" entityName="policy {id}" active="true" targetGroup="{GROUP}">
    <adjustmentType xsi:type="adjustments:StepAdjustment" stepValue="{step}"/>
    <scalingTrigger xsi:type="triggers:SimpleFireOnValue" id="{id}T" relationalOperator="EqualTo">
      <stimulus xsi:type="stimuli:SimulationTime"/>
      <expectedValue xsi:type="expectations:ExpectedTime" value="{at:.1}"/>
    </scalingTrigger>
  </scalingPolicies>
"#
        ));
    }
    spd.push_str(&format!(
        r#"  <targetGroups xsi:type="targets:ElasticInfrastructure" id="{GROUP}" entityName="EI TargetGroup">
    <unit href="My.resourceenvironment#_GAeeUCpXEeKNWc2UWjbmuw"/>
    <targetConstraints xsi:type="target:TargetGroupSizeConstraint" minSize="1" maxSize="3"/>
  </targetGroups>
</spd:SPD>
"#
    ));
    std::fs::write(out.join("default.spd"), spd).unwrap();
    out.to_string_lossy().into_owned()
}

/// A scaling decision (#6) in the report: the run pauses at the scale-out recommended at
/// t = 2 s (`_out2`), the decision pending; the resume selects `[_out2, _out2]` (1 to 2, 2 to 3
/// servers), and `_out6` at 6 s finds the group at its maximum (3 to 3). The report's
/// `decisions` is the decision file's, its counters count the three adjustments, and the
/// resumed run with `--decide _out2` is the run without decisions (the same measurements and
/// decision log, with or without reports).
#[cfg(feature = "reconfiguration")]
#[test]
fn decisions_and_their_outcomes() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let d = tmp("decisions");
    let p = |n: &str| d.join(n).to_string_lossy().into_owned();
    let m = model(&d, &[("_out2", 1, 2.0), ("_out6", 1, 6.0)]);
    let flags = [
        "--flavour",
        "slingshot",
        "--mode",
        "fast",
        "--quirks",
        "none",
        "--spd-quirks",
        "fixed",
        "--max-sim-time",
        "10",
    ];
    let mut a = vec!["run", "--model", &m];
    a.extend(flags);
    let (rp, snap, df, log) = (p("a.json"), p("a.snap"), p("a.decision"), p("a.log"));
    a.extend([
        "--pause-on-decision",
        "--snapshot",
        &snap,
        "--decision-file",
        &df,
        "--spd-log",
        &log,
        "--report",
        &rp,
    ]);
    ok(&a);
    check_outputs(Path::new(&rp));
    let r = json_file(Path::new(&rp));
    assert_eq!(stop(&r), ("decision", "decision"));
    assert_eq!(r["stop"]["decision"], 1);
    assert_eq!(times(&r), (0.0, 2.0, 2.0));
    assert_eq!(r["endpoint_reached"], false);
    assert_eq!(r["resumable"], true);
    assert_eq!(r["behaviour"]["spd_quirks"], "fixed");
    assert_eq!(r["limits"]["pause_on_decision"], true);
    let file = json_file(Path::new(&df));
    assert_eq!(r["decisions"]["pending"], file["pending"]);
    assert_eq!(r["decisions"]["resolved"], Value::Null);
    assert_eq!(
        r["decisions"]["pending"]["recommended"][0]["policy"],
        "_out2"
    );
    assert_eq!(r["counters"]["invocation"]["adjustments"], 0);
    for k in ["spd_log", "decision_file", "snapshot"] {
        assert!(r["outputs"].get(k).is_some(), "{k}: {r}");
    }

    let (rp, df) = (p("b.json"), p("b.decision"));
    ok(&[
        "resume",
        &snap,
        "--model",
        &m,
        "--decide",
        "_out2,_out2",
        "--decision-file",
        &df,
        "--report",
        &rp,
    ]);
    check_outputs(Path::new(&rp));
    let r = json_file(Path::new(&rp));
    assert_eq!(stop(&r), ("completed", "max_sim_time"));
    assert_eq!(times(&r), (2.0, 10.0, 10.0));
    let file = json_file(Path::new(&df));
    assert_eq!(r["decisions"]["resolved"], file["resolved"]);
    let res = &r["decisions"]["resolved"];
    assert_eq!(res["selected"], json!(["_out2", "_out2"]));
    let sizes: Vec<(Value, Value)> = res["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| (o["size_before"].clone(), o["size_after"].clone()))
        .collect();
    assert_eq!(sizes, vec![(json!(1), json!(2)), (json!(2), json!(3))]);
    assert_eq!(r["decisions"]["pending"], Value::Null);
    // the two actions at the resume and `_out6` at 6 s (at the maximum size: no change, but
    // executed)
    assert_eq!(r["counters"]["invocation"]["adjustments"], 3);
    assert_eq!(r["counters"]["cumulative"]["adjustments"], 3);
    assert!(
        r["behaviour"]["restored"]
            .as_array()
            .unwrap()
            .contains(&json!("spd_quirks"))
    );

    // the recommendation applied is the run without decisions, reports or not
    let mut a = vec!["run", "--model", &m];
    a.extend(flags);
    let wm = p("whole.csv");
    a.extend(["--measurements", &wm]);
    ok(&a);
    for (tag, rep) in [("r", true), ("n", false)] {
        let f = |n: &str| p(&format!("{tag}.{n}"));
        let (rs, rc) = (f("resumed.json"), f("csv"));
        let mut a = vec!["resume", &snap, "--model", &m, "--decide", "_out2"];
        a.extend(["--measurements", &rc]);
        if rep {
            a.extend(["--report", &rs]);
        }
        ok(&a);
        assert_eq!(read(Path::new(&rc)), read(Path::new(&wm)), "{tag}");
    }
    let r = json_file(&d.join("r.resumed.json"));
    assert_eq!(r["counters"]["invocation"]["adjustments"], 2);
}

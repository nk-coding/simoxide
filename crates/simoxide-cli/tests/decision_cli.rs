//! Scaling decisions in new processes (issue #6): `simoxide run --pause-on-decision` pauses
//! before a policy's adjustment runs, with the recommendation pending in the snapshot and the
//! decision file; `simoxide resume --decide` continues it with an explicit, ordered action list
//! in place of the recommendation.
//!
//! The model is Slingshot's minimal example (one `AppServer` behind a load balancer) with its
//! own policies on the elastic infrastructure (size 1 to 3): `_out2` scales out by one at t = 2
//! s, `_out6` at t = 6 s, `_in` (inactive: an action only) scales in by one. Flags as in the
//! issue: `--flavour slingshot --mode fast --quirks none --spd-quirks fixed`, 10 s.

#![cfg(all(feature = "spd", feature = "reconfiguration"))]

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("decision-cli-{name}"));
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
    std::fs::read_to_string(p).unwrap_or_default()
}

const GROUP: &str = "_DC8fM8pgEe6u3p81um7oyQ";

fn policy(id: &str, active: bool, step: i32, at: f64) -> String {
    format!(
        r#"  <scalingPolicies id="{id}" entityName="policy {id}" active="{active}" targetGroup="{GROUP}">
    <adjustmentType xsi:type="adjustments:StepAdjustment" stepValue="{step}"/>
    <scalingTrigger xsi:type="triggers:SimpleFireOnValue" id="{id}T" relationalOperator="EqualTo">
      <stimulus xsi:type="stimuli:SimulationTime"/>
      <expectedValue xsi:type="expectations:ExpectedTime" value="{at:.1}"/>
    </scalingTrigger>
  </scalingPolicies>
"#
    )
}

/// The minimal example with `policies` (ids, active, step, time) instead of its own.
fn model(d: &Path, policies: &[(&str, bool, i32, f64)]) -> String {
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
    for &(id, active, step, at) in policies {
        spd.push_str(&policy(id, active, step, at));
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

const ISSUE_FLAGS: [&str; 10] = [
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

fn records(log: &str) -> Vec<Value> {
    log.lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .collect()
}

/// The `adjust` records as (t, policy, size_before, size_after).
fn adjusts(log: &str) -> Vec<(f64, String, i64, i64)> {
    records(log)
        .into_iter()
        .filter(|v| v["k"] == "adjust")
        .map(|v| {
            assert_eq!(v["ok"], true, "{v}");
            (
                v["t"].as_f64().unwrap(),
                v["policy"].as_str().unwrap().to_string(),
                v["size_before"].as_i64().unwrap(),
                v["size_after"].as_i64().unwrap(),
            )
        })
        .collect()
}

/// A decision log without what decisions add: `decision` and `select` records and the
/// `decision` field of `adjust` records.
fn without_decisions(log: &str) -> Vec<Value> {
    records(log)
        .into_iter()
        .filter(|v| v["k"] != "decision" && v["k"] != "select")
        .map(|mut v| {
            if let Some(o) = v.as_object_mut() {
                o.remove("decision");
            }
            v
        })
        .collect()
}

fn json_file(p: &Path) -> Value {
    serde_json::from_str(&read(p)).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn rec(policy: &str, size: i64, step: i64) -> Value {
    json!({
        "policy": policy,
        "policy_name": format!("policy {policy}"),
        "target_group": GROUP,
        "group_size": size,
        "adjustment": {"type": "StepAdjustment", "stepValue": step},
    })
}

fn outcome(policy: &str, before: i64, after: i64) -> Value {
    json!({
        "policy": policy,
        "policy_name": format!("policy {policy}"),
        "target_group": GROUP,
        "ok": true,
        "size_before": before,
        "size_after": after,
    })
}

/// `outcomes` without the change counts (checked separately).
fn outcomes(v: &Value) -> Vec<Value> {
    v["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            let mut o = o.clone();
            o.as_object_mut().unwrap().remove("changes");
            o
        })
        .collect()
}

/// The issue's scenario: a time trigger accepts a scale-out at t = 2; the run pauses before
/// it is applied; three continuations from the same snapshot (apply, decline, a scale-in
/// instead) and `[A, A]`, `[A, A, A]` (the third refused at the maximum size), `[A, B]`.
#[test]
fn pause_on_a_recommendation_and_resume_with_actions() {
    let d = tmp("issue");
    let m = model(
        &d,
        &[
            ("_out2", true, 1, 2.0),
            ("_out6", true, 1, 6.0),
            ("_in", false, -1, 1000.0),
        ],
    );
    let p = |n: &str| d.join(n).to_string_lossy().into_owned();
    let run = |extra: &[&str]| {
        let mut a = vec!["run", "--model", &m];
        a.extend(ISSUE_FLAGS);
        a.extend(extra);
        ok(&a)
    };
    let resume = |snap: &str, extra: &[&str]| {
        let mut a = vec!["resume", snap, "--model", &m];
        a.extend(extra);
        ok(&a)
    };

    // without the new flag: both scale-outs run (1 -> 2 at 2 s, 2 -> 3 at 6 s), no decision
    let err = run(&[
        "--spd-log",
        &p("whole.jsonl"),
        "--measurements",
        &p("whole.csv"),
    ]);
    assert!(err.contains("adjusted=2"), "{err}");
    let whole = read(&d.join("whole.jsonl"));
    assert_eq!(
        adjusts(&whole),
        [(2.0, "_out2".into(), 1, 2), (6.0, "_out6".into(), 2, 3)]
    );
    assert!(!whole.contains("\"k\":\"decision\""));

    // paused at the recommendation, before its adjustment
    let err = run(&[
        "--pause-on-decision",
        "--snapshot",
        &p("d.snap"),
        "--decision-file",
        &p("d1.json"),
        "--spd-log",
        &p("a.jsonl"),
    ]);
    assert!(
        err.contains("scaling decision 1 pending at t=2: recommended [_out2]"),
        "{err}"
    );
    assert!(err.contains("paused t=2 "), "{err}");
    let a = read(&d.join("a.jsonl"));
    assert_eq!(adjusts(&a), []);
    let last = records(&a).pop().unwrap();
    assert_eq!(
        last,
        json!({"k": "decision", "t": 2.0, "decision": 1, "recommended": ["_out2"]})
    );
    let pending = json!({
        "decision": 1,
        "time": 2.0,
        "time_ns": 2_000_000_000i64,
        "recommended": [rec("_out2", 1, 1)],
    });
    assert_eq!(
        json_file(&d.join("d1.json")),
        json!({"format": "simoxide-spd-decisions", "version": 1, "resolved": null, "pending": pending})
    );
    // the snapshot holds it (snapshot-info)
    let info: Value = serde_json::from_str(&ok(&["snapshot-info", &p("d.snap")])).unwrap();
    assert_eq!(info["time_ns"], 2_000_000_000i64);
    assert_eq!(
        info["spd_decision"],
        json!({"decisions": 1, "pending": pending})
    );
    assert!(
        info["sections"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "spd-decision")
    );

    // a pending decision does not resume without an explicit choice
    let err = fails(&["resume", &p("d.snap"), "--model", &m]);
    assert!(
        err.contains("pending scaling decision") && err.contains("--decide"),
        "{err}"
    );
    let err = fails(&["resume", &p("d.snap"), "--model", &m, "--decide", "nope"]);
    assert!(err.contains("no scaling policy named nope"), "{err}");

    // 1. apply the recommendation: the uninterrupted run (log without the decision records,
    //    measurements)
    let err = resume(
        &p("d.snap"),
        &[
            "--decide",
            "_out2",
            "--decision-file",
            &p("apply.json"),
            "--spd-log",
            &p("apply.jsonl"),
            "--measurements",
            &p("apply.csv"),
        ],
    );
    assert!(
        err.contains("decision 1 (t=2): recommended [_out2], selected [_out2]"),
        "{err}"
    );
    let apply = read(&d.join("apply.jsonl"));
    assert_eq!(
        adjusts(&apply),
        [(2.0, "_out2".into(), 1, 2), (6.0, "_out6".into(), 2, 3)]
    );
    assert_eq!(
        without_decisions(&(a.clone() + &apply)),
        without_decisions(&whole)
    );
    assert_eq!(read(&d.join("apply.csv")), read(&d.join("whole.csv")));
    let f = json_file(&d.join("apply.json"));
    assert_eq!(f["pending"], Value::Null);
    assert_eq!(f["resolved"]["decision"], 1);
    assert_eq!(f["resolved"]["recommended"], json!([rec("_out2", 1, 1)]));
    assert_eq!(f["resolved"]["selected"], json!(["_out2"]));
    assert_eq!(outcomes(&f["resolved"]), [outcome("_out2", 1, 2)]);
    assert_eq!(f["resolved"]["outcomes"][0]["changes"], 4);
    let r = records(&apply);
    assert_eq!(
        r[0],
        json!({"k": "select", "t": 2.0, "decision": 1, "selected": ["_out2"]})
    );
    assert_eq!(r[1]["k"], "adjust");
    assert_eq!(r[1]["decision"], 1);

    // 2. decline: stays at one server; the recommendation does not run later, the next
    //    policy's does (1 -> 2 at 6 s)
    resume(
        &p("d.snap"),
        &[
            "--decide",
            "[]",
            "--decision-file",
            &p("decline.json"),
            "--spd-log",
            &p("decline.jsonl"),
        ],
    );
    assert_eq!(
        adjusts(&read(&d.join("decline.jsonl"))),
        [(6.0, "_out6".into(), 1, 2)]
    );
    let f = json_file(&d.join("decline.json"));
    assert_eq!(f["resolved"]["selected"], json!([]));
    assert_eq!(f["resolved"]["outcomes"], json!([]));
    // ... also not at a later resume: declined, paused at the next decision, resumed again
    resume(
        &p("d.snap"),
        &[
            "--decide",
            "[]",
            "--pause-on-decision",
            "--snapshot",
            &p("d2.snap"),
            "--decision-file",
            &p("d2.json"),
            "--spd-log",
            &p("b1.jsonl"),
        ],
    );
    let f = json_file(&d.join("d2.json"));
    assert_eq!(f["resolved"]["decision"], 1);
    assert_eq!(f["pending"]["decision"], 2);
    assert_eq!(f["pending"]["time_ns"], 6_000_000_000i64);
    assert_eq!(f["pending"]["recommended"], json!([rec("_out6", 1, 1)]));
    assert_eq!(adjusts(&read(&d.join("b1.jsonl"))), []);
    resume(
        &p("d2.snap"),
        &["--decide", "_out6", "--spd-log", &p("b2.jsonl")],
    );
    assert_eq!(
        adjusts(&read(&d.join("b2.jsonl"))),
        [(6.0, "_out6".into(), 1, 2)]
    );

    // 3. a scale-in instead (the inactive policy as an action): the scale-out never runs, the
    //    group never has two servers before 6 s (the size constraint keeps the one)
    resume(
        &p("d.snap"),
        &[
            "--decide",
            "_in",
            "--decision-file",
            &p("in.json"),
            "--spd-log",
            &p("in.jsonl"),
        ],
    );
    assert_eq!(
        adjusts(&read(&d.join("in.jsonl"))),
        [(2.0, "_in".into(), 1, 1), (6.0, "_out6".into(), 1, 2)]
    );
    assert_eq!(
        outcomes(&json_file(&d.join("in.json"))["resolved"]),
        [outcome("_in", 1, 1)]
    );

    // [A, A]: twice, in order; [A, A, A]: the third is clamped at the maximum size (3);
    // [A, B]: out, then in
    for (actions, want, then) in [
        (
            "_out2,_out2",
            vec![outcome("_out2", 1, 2), outcome("_out2", 2, 3)],
            (3, 3),
        ),
        (
            r#"["_out2","_out2","_out2"]"#,
            vec![
                outcome("_out2", 1, 2),
                outcome("_out2", 2, 3),
                outcome("_out2", 3, 3),
            ],
            (3, 3),
        ),
        (
            "_out2,_in",
            vec![outcome("_out2", 1, 2), outcome("_in", 2, 1)],
            (1, 2),
        ),
    ] {
        resume(
            &p("d.snap"),
            &[
                "--decide",
                actions,
                "--decision-file",
                &p("x.json"),
                "--spd-log",
                &p("x.jsonl"),
            ],
        );
        assert_eq!(
            outcomes(&json_file(&d.join("x.json"))["resolved"]),
            want,
            "{actions}"
        );
        let a = adjusts(&read(&d.join("x.jsonl")));
        assert_eq!(a.len(), want.len() + 1, "{actions}");
        assert!(a[..want.len()].iter().all(|x| x.0 == 2.0));
        assert_eq!(a[want.len()], (6.0, "_out6".into(), then.0, then.1));
    }

    // --decide needs a pending decision
    ok(&[
        "run",
        "--model",
        &m,
        "--flavour",
        "slingshot",
        "--max-sim-time",
        "10",
        "--pause-at",
        "1",
        "--snapshot",
        &p("t1.snap"),
    ]);
    let err = fails(&["resume", &p("t1.snap"), "--model", &m, "--decide", "[]"]);
    assert!(err.contains("no pending scaling decision"), "{err}");
    std::fs::remove_dir_all(&d).unwrap();
}

/// Two policies fire at the same instant: one decision with both recommendations in request
/// (model) order; resuming with exactly them is the uninterrupted run.
#[test]
fn recommendations_of_one_instant_form_one_decision() {
    let d = tmp("instant");
    let m = model(&d, &[("_outA", true, 2, 2.0), ("_inB", true, -1, 2.0)]);
    let p = |n: &str| d.join(n).to_string_lossy().into_owned();
    let mut a = vec!["run", "--model", &m];
    a.extend(ISSUE_FLAGS);
    let (wl, wm) = (p("whole.jsonl"), p("whole.csv"));
    let mut whole = a.clone();
    whole.extend(["--spd-log", &wl, "--measurements", &wm]);
    ok(&whole);
    let w = read(&d.join("whole.jsonl"));
    assert_eq!(
        adjusts(&w),
        [(2.0, "_outA".into(), 1, 3), (2.0, "_inB".into(), 3, 2)]
    );
    let snap = p("d.snap");
    let f1 = p("d.json");
    let l1 = p("a.jsonl");
    a.extend([
        "--pause-on-decision",
        "--snapshot",
        &snap,
        "--decision-file",
        &f1,
        "--spd-log",
        &l1,
    ]);
    ok(&a);
    let f = json_file(&d.join("d.json"));
    assert_eq!(
        f["pending"]["recommended"],
        json!([rec("_outA", 1, 2), rec("_inB", 1, -1)])
    );
    let l2 = p("b.jsonl");
    let m2 = p("b.csv");
    ok(&[
        "resume",
        &snap,
        "--model",
        &m,
        "--decide",
        "_outA,_inB",
        "--spd-log",
        &l2,
        "--measurements",
        &m2,
    ]);
    assert_eq!(
        without_decisions(&(read(&d.join("a.jsonl")) + &read(&d.join("b.jsonl")))),
        without_decisions(&w)
    );
    assert_eq!(read(&d.join("b.csv")), read(&d.join("whole.csv")));
    std::fs::remove_dir_all(&d).unwrap();
}

/// The interval line of the CLI (`--measurements-scope`): (scope, from, to, parent, state).
fn interval(err: &str) -> (String, u64, u64, Option<String>, Option<String>) {
    let l = err
        .lines()
        .find(|l| l.starts_with("measurements: "))
        .unwrap_or_else(|| panic!("no interval line: {err}"));
    let field = |k: &str| {
        l.split_whitespace()
            .find_map(|w| w.strip_prefix(k))
            .map(String::from)
    };
    let (from, to) = field("tuples=")
        .unwrap()
        .split_once("..")
        .map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap()))
        .unwrap();
    (
        field("scope=").unwrap(),
        from,
        to,
        field("parent="),
        field("state="),
    )
}

/// The data lines of a `measurements.csv` per series, in file order.
fn series(csv: &str) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut lines = csv.lines();
    assert_eq!(lines.next(), Some("measuring_point,metric,time,value"));
    let mut m = std::collections::BTreeMap::<String, Vec<String>>::new();
    for l in lines {
        let f: Vec<&str> = l.rsplitn(3, ',').collect();
        m.entry(f[2].to_string()).or_default().push(l.to_string());
    }
    m
}

fn rows(csv: &str) -> u64 {
    csv.lines().count() as u64 - 1
}

/// Decisions with `--measurements-scope interval` (issue #7): the decision snapshot carries the
/// measurement cursor and its state id; children that decide differently each export only their
/// own tuples, from the parent's cursor, and name the parent; the child that decides the
/// recommendation continues the parent's export to the uninterrupted run's measurements.
#[test]
fn decisions_with_interval_measurements() {
    let d = tmp("interval");
    let m = model(
        &d,
        &[
            ("_out2", true, 1, 2.0),
            ("_out6", true, 1, 6.0),
            ("_in", false, -1, 1000.0),
        ],
    );
    let p = |n: &str| d.join(n).to_string_lossy().into_owned();
    let run = |extra: &[&str]| {
        let mut a = vec!["run", "--model", &m];
        a.extend(ISSUE_FLAGS);
        a.extend(extra);
        ok(&a)
    };
    let header = |f: &str| -> Value { serde_json::from_str(&ok(&["snapshot-info", f])).unwrap() };
    run(&["--measurements", &p("whole.csv")]);
    let whole = read(&d.join("whole.csv"));

    // the parent: paused at the decision, its interval exported, the snapshot without rows
    let err = run(&[
        "--pause-on-decision",
        "--measurements-scope",
        "interval",
        "--snapshot",
        &p("d.snap"),
        "--measurements",
        &p("parent.csv"),
    ]);
    let (scope, from, cursor, parent, state) = interval(&err);
    assert_eq!(
        (scope.as_str(), from, parent),
        ("interval", 0, None),
        "{err}"
    );
    assert!(cursor > 0, "{err}");
    let state = state.unwrap();
    assert_eq!(rows(&read(&d.join("parent.csv"))), cursor);
    let h = header(&p("d.snap"));
    assert_eq!(h["state"], state.as_str());
    assert_eq!(h["parent"], Value::Null);
    assert_eq!(
        h["measurements"],
        json!({"tuples": cursor, "rows_from": cursor})
    );
    assert_eq!(
        h["spd_decision"]["pending"]["recommended"][0]["policy"],
        "_out2"
    );
    let snap_bytes = std::fs::read(d.join("d.snap")).unwrap();

    // children of the one parent: each exports its own tuples from the parent's cursor
    let mut ends = Vec::new();
    for (tag, actions) in [
        ("apply", "_out2"),
        ("decline", "[]"),
        ("in", "_in"),
        ("twice", "_out2,_out2"),
    ] {
        let csv = p(&format!("{tag}.csv"));
        let err = ok(&[
            "resume",
            &p("d.snap"),
            "--model",
            &m,
            "--decide",
            actions,
            "--measurements",
            &csv,
        ]);
        let (scope, from, to, parent, _) = interval(&err);
        assert_eq!(scope, "interval", "{tag}: {err}");
        assert_eq!(from, cursor, "{tag}: {err}");
        assert_eq!(parent.as_deref(), Some(state.as_str()), "{tag}: {err}");
        assert_eq!(rows(&read(Path::new(&csv))), to - from, "{tag}");
        ends.push(to);
    }
    // the parent is unchanged by its children
    assert_eq!(std::fs::read(d.join("d.snap")).unwrap(), snap_bytes);

    // parent + the child that decided the recommendation = the uninterrupted run
    let mut joined = series(&read(&d.join("parent.csv")));
    for (k, v) in series(&read(&d.join("apply.csv"))) {
        joined.entry(k).or_default().extend(v);
    }
    assert_eq!(joined, series(&whole));
    assert_eq!(ends[0], rows(&whole));

    // a child paused at the next decision: its snapshot names the parent, its cursor continues
    let err = ok(&[
        "resume",
        &p("d.snap"),
        "--model",
        &m,
        "--decide",
        "[]",
        "--pause-on-decision",
        "--snapshot",
        &p("d2.snap"),
        "--measurements",
        &p("c.csv"),
    ]);
    let (_, from, c2, _, state2) = interval(&err);
    assert_eq!(from, cursor);
    let h = header(&p("d2.snap"));
    assert_eq!(h["parent"], state.as_str());
    assert_eq!(h["state"], state2.unwrap().as_str());
    assert_eq!(h["measurements"], json!({"tuples": c2, "rows_from": c2}));
    assert_eq!(h["spd_decision"]["pending"]["decision"], 2);
    // ... and its own child starts there; with the decline child it covers the same tuples
    let err = ok(&[
        "resume",
        &p("d2.snap"),
        "--model",
        &m,
        "--decide",
        "_out6",
        "--measurements",
        &p("g.csv"),
    ]);
    let (_, from, to, _, _) = interval(&err);
    assert_eq!((from, to), (c2, ends[1]));
    let mut joined = series(&read(&d.join("c.csv")));
    for (k, v) in series(&read(&d.join("g.csv"))) {
        joined.entry(k).or_default().extend(v);
    }
    assert_eq!(joined, series(&read(&d.join("decline.csv"))));
    std::fs::remove_dir_all(&d).unwrap();
}

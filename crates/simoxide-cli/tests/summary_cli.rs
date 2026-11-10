//! `--summary` (`docs/guide/snapshots.md`, "Interval summaries") in new processes: each
//! interval's summary equals the statistics computed independently from the same interval's
//! raw export (`--measurements-scope interval`), with the last tuple before the interval carried
//! across snapshots; adjacent intervals at the same time are split by tuple number; empty series
//! and empty intervals are explicit; the counts and time integrals of a path add up to the
//! uninterrupted run's; summaries do not depend on `--measurements` or on the scope.

#![cfg(feature = "spd")]

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("summary-cli-{name}"));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ok(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_simoxide"))
        .args(args)
        .output()
        .expect("simoxide runs");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "simoxide {args:?}: {err}");
    err
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

fn json_file(p: &Path) -> Value {
    serde_json::from_str(&read(p)).unwrap()
}

/// A series: (measuring point, metric).
type Key = (String, String);

/// The rows `(time, value)` of a `measurements.csv` per series, in file order.
fn rows(csv: &str) -> BTreeMap<Key, Vec<(f64, f64)>> {
    let mut lines = csv.lines();
    assert_eq!(lines.next(), Some("measuring_point,metric,time,value"));
    let mut m: BTreeMap<Key, Vec<(f64, f64)>> = BTreeMap::new();
    for l in lines {
        let f: Vec<&str> = l.rsplitn(4, ',').collect();
        m.entry((f[3].to_string(), f[2].to_string()))
            .or_default()
            .push((f[1].parse().unwrap(), f[0].parse().unwrap()));
    }
    m
}

fn key(s: &Value) -> Key {
    (
        s["measuring_point"].as_str().unwrap().to_string(),
        s["metric"].as_str().unwrap().to_string(),
    )
}

/// The series of a summary by key.
fn series(doc: &Value) -> BTreeMap<Key, Value> {
    doc["series"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (key(s), s.clone()))
        .collect()
}

/// The interval line of the CLI: (from, to, parent, state).
fn interval(err: &str) -> (u64, u64, Option<String>, Option<String>) {
    let l = err
        .lines()
        .find(|l| l.starts_with("measurements: "))
        .unwrap_or_else(|| panic!("no interval line: {err}"));
    let field = |k: &str| {
        l.split_whitespace()
            .find_map(|w| w.strip_prefix(k))
            .map(str::to_string)
    };
    let (from, to) = field("tuples=")
        .unwrap()
        .split_once("..")
        .map(|(a, b)| (a.parse().unwrap(), b.parse().unwrap()))
        .unwrap();
    (from, to, field("parent="), field("state="))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

/// Checks a summary (every statistic) against the interval's raw export `csv`, independently
/// computed: count, sum (in file order), mean, min, max, first, last exactly; `initial` the
/// last row of the series in an earlier interval of the path (`prior`, updated here);
/// `time_weighted` the step function from the interval's start (with the initial value) or
/// the first row, to its end. Also: every series of the export is in the summary, the counts
/// add up to the interval's tuples, and times are not before the interval's start.
fn check(doc: &Value, csv: &str, prior: &mut BTreeMap<Key, (f64, f64)>) {
    assert_eq!(doc["format"], "simoxide-interval-summary");
    assert_eq!(doc["version"], 1);
    let export = rows(csv);
    let all = series(doc);
    for k in export.keys() {
        assert!(all.contains_key(k), "{k:?} missing in {doc}");
    }
    let (from, to) = (
        doc["interval"]["time"]["from"].as_f64().unwrap(),
        doc["interval"]["time"]["to"].as_f64().unwrap(),
    );
    let mut total = 0;
    for (k, s) in &all {
        let r = export.get(k).cloned().unwrap_or_default();
        let ctx = format!("{k:?}: {s}");
        total += r.len() as u64;
        assert_eq!(s["count"], r.len(), "{ctx}");
        let sum = r.iter().fold(0.0, |a, x| a + x.1);
        assert_eq!(s["sum"].as_f64().unwrap(), sum, "{ctx}");
        let tuple =
            |x: Option<&(f64, f64)>| x.map_or(Value::Null, |x| json!({"time": x.0, "value": x.1}));
        assert_eq!(s["first"], tuple(r.first()), "{ctx}");
        assert_eq!(s["last"], tuple(r.last()), "{ctx}");
        if r.is_empty() {
            for f in ["mean", "min", "max"] {
                assert_eq!(s[f], Value::Null, "{ctx}");
            }
        } else {
            assert_eq!(s["mean"].as_f64().unwrap(), sum / r.len() as f64, "{ctx}");
            let min = r.iter().map(|x| x.1).fold(f64::INFINITY, f64::min);
            let max = r.iter().map(|x| x.1).fold(f64::NEG_INFINITY, f64::max);
            assert_eq!(s["min"].as_f64().unwrap(), min, "{ctx}");
            assert_eq!(s["max"].as_f64().unwrap(), max, "{ctx}");
            assert!(r[0].0 >= from, "{ctx}");
        }
        let init = prior.get(k).copied();
        assert_eq!(
            s["initial"],
            match init {
                Some((t, v)) => json!({"status": "tuple", "time": t, "value": v}),
                None => json!({"status": "none"}),
            },
            "{ctx}"
        );
        // the step function
        let mut held: Option<(f64, f64)> = init.map(|(_, v)| (from, v));
        let start = held.map(|h| h.0).or(r.first().map(|x| x.0));
        let mut integral = 0.0;
        for &(t, v) in &r {
            if let Some((since, hv)) = held {
                integral += hv * (t - since);
            }
            held = Some((t, v));
        }
        let tw = &s["time_weighted"];
        match (held, start) {
            (Some((since, hv)), Some(start)) => {
                integral += hv * (to - since);
                assert_eq!(tw["from"].as_f64().unwrap(), start, "{ctx}");
                assert_eq!(tw["to"].as_f64().unwrap(), to, "{ctx}");
                assert!(
                    close(tw["integral"].as_f64().unwrap(), integral),
                    "{ctx}: {integral}"
                );
                if to > start {
                    let m = integral / (to - start);
                    assert!(close(tw["mean"].as_f64().unwrap(), m), "{ctx}: {m}");
                } else {
                    assert_eq!(tw["mean"], Value::Null, "{ctx}");
                }
            }
            _ => assert_eq!(*tw, Value::Null, "{ctx}"),
        }
        if let Some(&l) = r.last() {
            prior.insert(k.clone(), l);
        }
    }
    let t = &doc["interval"]["tuples"];
    assert_eq!(
        total,
        t["to"].as_u64().unwrap() - t["from"].as_u64().unwrap(),
        "{doc}"
    );
}

/// A doc's (count, integral) per series.
fn totals(doc: &Value) -> BTreeMap<Key, (u64, f64)> {
    series(doc)
        .into_iter()
        .map(|(k, s)| {
            let i = s["time_weighted"]["integral"].as_f64().unwrap_or(0.0);
            (k, (s["count"].as_u64().unwrap(), i))
        })
        .collect()
}

/// Without `parent` and `state` (they name the snapshots, which differ between the scopes).
fn without_ids(mut doc: Value) -> Value {
    let o = doc.as_object_mut().unwrap();
    o.remove("parent");
    o.remove("state");
    doc
}

const GROUP: &str = "_DC8fM8pgEe6u3p81um7oyQ";

/// Slingshot's minimal example with two scale-outs of its elastic infrastructure (1 to 3
/// servers): at t = 2 s and at t = 6 s (as `decision_cli.rs`).
fn minimal_model(d: &Path) -> String {
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
    let policy = |id: &str, at: f64| {
        format!(
            r#"  <scalingPolicies id="{id}" entityName="policy {id}" active="true" targetGroup="{GROUP}">
    <adjustmentType xsi:type="adjustments:StepAdjustment" stepValue="1"/>
    <scalingTrigger xsi:type="triggers:SimpleFireOnValue" id="{id}T" relationalOperator="EqualTo">
      <stimulus xsi:type="stimuli:SimulationTime"/>
      <expectedValue xsi:type="expectations:ExpectedTime" value="{at:.1}"/>
    </scalingTrigger>
  </scalingPolicies>
"#
        )
    };
    let spd = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<spd:SPD xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:adjustments="http://palladiosimulator.org/ScalingPolicyDefinition/Adjustments/1.0" xmlns:expectations="http://palladiosimulator.org/ScalingPolicyDefinition/Triggers/Expectations/1.0" xmlns:spd="http://palladiosimulator.org/ScalingPolicyDefinition/1.0" xmlns:stimuli="http://palladiosimulator.org/ScalingPolicyDefinition/Triggers/Stimuli/1.0" xmlns:target="http://palladiosimulator.org/ScalingPolicyDefinition/Constraints/Target/1.0" xmlns:targets="http://palladiosimulator.org/ScalingPolicyDefinition/Targets/1.0" xmlns:triggers="http://palladiosimulator.org/ScalingPolicyDefinition/Triggers/1.0" id="_PKdecLwZEe6i2ICnm9lfkw">
{}{}  <targetGroups xsi:type="targets:ElasticInfrastructure" id="{GROUP}" entityName="EI TargetGroup">
    <unit href="My.resourceenvironment#_GAeeUCpXEeKNWc2UWjbmuw"/>
    <targetConstraints xsi:type="target:TargetGroupSizeConstraint" minSize="1" maxSize="3"/>
  </targetGroups>
</spd:SPD>
"#,
        policy("_out2", 2.0),
        policy("_out6", 6.0)
    );
    std::fs::write(out.join("default.spd"), spd).unwrap();
    out.to_string_lossy().into_owned()
}

const FLAGS: [&str; 10] = [
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

/// The server of the minimal example (the target group's first member) and the one the first
/// scale-out adds, and the usage scenario.
const SERVER: &str = "ActiveResourceMeasuringPoint[_KFRwgCpXEeKNWc2UWjbmuw|replicaID=0]";
const SCENARIO: &str = "UsageScenarioMeasuringPoint[_LPnI8CHdEd6lJo4DCALHMw]";
const STATE: &str = "State of Active Resource Tuple";
const RT: &str = "Response Time Tuple";

fn at<'a>(doc: &'a Value, mp: &str, metric: &str) -> &'a Value {
    doc["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["measuring_point"] == mp && s["metric"] == metric)
        .unwrap_or_else(|| panic!("{mp} {metric} in {doc}"))
}

/// Adjacent intervals at the same time (`--pause-events`): the run without its policies pauses
/// after 5 events at t = 2 s, after the server's state and utilisation tuples of t = 2 s and
/// before the scenario's response time of t = 2 s (tuple 4). The tuple numbers decide: the
/// first interval has tuples 0..4 and no response time, the second has it at its start time.
#[test]
fn equal_times_are_split_by_tuple_number() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let d = tmp("equal-times");
    let p = |n: &str| d.join(n).to_string_lossy().into_owned();
    let model = minimal_model(&d);
    let mut flags = FLAGS.to_vec();
    flags.push("--no-spd");
    let run = |extra: &[&str]| {
        let mut a = vec!["run", "--model", &model];
        a.extend(&flags);
        a.extend(extra);
        ok(&a)
    };
    run(&[
        "--measurements",
        &p("whole.csv"),
        "--summary",
        &p("whole.json"),
    ]);
    let e1 = run(&[
        "--pause-events",
        "5",
        "--snapshot",
        &p("a.snap"),
        "--measurements-scope",
        "interval",
        "--measurements",
        &p("a.csv"),
        "--summary",
        &p("a.json"),
    ]);
    let e2 = ok(&[
        "resume",
        &p("a.snap"),
        "--model",
        &model,
        "--measurements",
        &p("b.csv"),
        "--summary",
        &p("b.json"),
    ]);
    let (a, b, whole) = (
        json_file(&d.join("a.json")),
        json_file(&d.join("b.json")),
        json_file(&d.join("whole.json")),
    );
    // the intervals: by number 0..4 and 4..17, both ending and starting at t = 2 s
    assert_eq!(a["interval"]["tuples"], json!({"from": 0, "to": 4}));
    assert_eq!(a["interval"]["time"], json!({"from": 0.0, "to": 2.0}));
    assert_eq!(b["interval"]["tuples"], json!({"from": 4, "to": 17}));
    assert_eq!(b["interval"]["time"], json!({"from": 2.0, "to": 10.0}));
    assert_eq!(
        b["interval"]["time_ns"],
        json!({"from": 2_000_000_000i64, "to": 10_000_000_000i64})
    );
    // the ids of the interval lines
    let (_, _, _, sa) = interval(&e1);
    let (_, _, pb, _) = interval(&e2);
    assert_eq!(a["state"], json!(sa.unwrap()));
    assert_eq!(b["parent"], json!(pb.unwrap()));
    assert!(a.get("parent").is_none() && b.get("state").is_none());
    // hand-calculated: the server busy from 0 to 2 s (state 1 at 0 s, 0 at 2 s)
    assert_eq!(
        *at(&a, SERVER, STATE),
        json!({"measuring_point": SERVER, "metric": STATE, "unit": "1",
               "count": 2, "sum": 1.0, "mean": 0.5, "min": 0.0, "max": 1.0,
               "first": {"time": 0.0, "value": 1.0}, "last": {"time": 2.0, "value": 0.0},
               "initial": {"status": "none"},
               "time_weighted": {"from": 0.0, "to": 2.0, "integral": 2.0, "mean": 1.0}})
    );
    // the response time of t = 2 s: not in the first interval (an empty series), the first
    // tuple of the second, at its start time
    assert_eq!(
        *at(&a, SCENARIO, RT),
        json!({"measuring_point": SCENARIO, "metric": RT, "unit": "s",
               "count": 0, "sum": 0.0, "mean": null, "min": null, "max": null,
               "first": null, "last": null, "initial": {"status": "none"},
               "time_weighted": null})
    );
    let rt = at(&b, SCENARIO, RT);
    assert_eq!(rt["count"], 3);
    assert_eq!(rt["first"], json!({"time": 2.0, "value": 2.0}));
    assert_eq!(rt["initial"], json!({"status": "none"}));
    // the second interval holds the server's last state of the first from its start
    assert_eq!(
        at(&b, SERVER, STATE)["initial"],
        json!({"status": "tuple", "time": 2.0, "value": 0.0})
    );
    // both against their exports, and together the uninterrupted run
    let mut prior = BTreeMap::new();
    check(&a, &read(&d.join("a.csv")), &mut prior);
    check(&b, &read(&d.join("b.csv")), &mut prior);
    check(&whole, &read(&d.join("whole.csv")), &mut BTreeMap::new());
    let (ta, tb) = (totals(&a), totals(&b));
    for (k, (n, i)) in totals(&whole) {
        let (na, ia) = ta.get(&k).copied().unwrap_or_default();
        let (nb, ib) = tb.get(&k).copied().unwrap_or_default();
        assert_eq!(na + nb, n, "{k:?}");
        assert!(close(ia + ib, i), "{k:?}: {ia} + {ib} != {i}");
    }
}

/// Scaling decisions and empty intervals, hand-calculated: the minimal example pauses at the
/// scale-out recommended at t = 2 s, is resumed with it (`--decide`) to t = 6 s, then for an
/// interval without events (`--pause-at 7.9`: the next event is at 8 s, so the run pauses at
/// once, at 6 s), then to the end at 10 s.
#[test]
fn decisions_and_empty_intervals() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let d = tmp("decisions");
    let p = |n: &str| d.join(n).to_string_lossy().into_owned();
    let model = minimal_model(&d);
    let run = |extra: &[&str]| {
        let mut a = vec!["run", "--model", &model];
        a.extend(FLAGS);
        a.extend(extra);
        ok(&a)
    };
    let resume = |snap: &str, extra: &[&str]| {
        let mut a = vec!["resume", snap, "--model", &model];
        a.extend(extra);
        ok(&a)
    };
    run(&[
        "--measurements",
        &p("whole.csv"),
        "--summary",
        &p("whole.json"),
    ]);
    // the chain, with and without --measurements
    for (tag, meas) in [("", true), ("n", false)] {
        let f = |n: &str| p(&format!("{tag}{n}"));
        let with = |name: &str, mut v: Vec<String>| {
            if meas {
                v.extend(["--measurements".into(), f(&format!("{name}.csv"))]);
            }
            v.extend(["--summary".into(), f(&format!("{name}.json"))]);
            v
        };
        let s = |v: Vec<String>| v;
        let a = with(
            "a",
            s(vec![
                "--pause-on-decision".into(),
                "--snapshot".into(),
                f("a.snap"),
                "--measurements-scope".into(),
                "interval".into(),
            ]),
        );
        run(&a.iter().map(String::as_str).collect::<Vec<_>>());
        let b = with(
            "b",
            s(vec![
                "--decide".into(),
                "_out2".into(),
                "--pause-at".into(),
                "6".into(),
                "--snapshot".into(),
                f("b.snap"),
            ]),
        );
        resume(
            &f("a.snap"),
            &b.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let c = with(
            "c",
            s(vec![
                "--pause-at".into(),
                "7.9".into(),
                "--snapshot".into(),
                f("c.snap"),
            ]),
        );
        resume(
            &f("b.snap"),
            &c.iter().map(String::as_str).collect::<Vec<_>>(),
        );
        let e = with("e", Vec::new());
        resume(
            &f("c.snap"),
            &e.iter().map(String::as_str).collect::<Vec<_>>(),
        );
    }
    let docs: Vec<Value> = ["a", "b", "c", "e"]
        .iter()
        .map(|n| json_file(&d.join(format!("{n}.json"))))
        .collect();
    // the summaries do not depend on --measurements: the same bytes
    for n in ["a", "b", "c", "e"] {
        assert_eq!(
            read(&d.join(format!("{n}.json"))),
            read(&d.join(format!("n{n}.json"))),
            "{n}"
        );
    }
    let [a, b, c, e] = &docs[..] else {
        unreachable!()
    };
    let iv = |doc: &Value| {
        (
            doc["interval"]["tuples"].clone(),
            doc["interval"]["time"].clone(),
        )
    };
    assert_eq!(
        iv(a),
        (json!({"from": 0, "to": 4}), json!({"from": 0.0, "to": 2.0}))
    );
    assert_eq!(
        iv(b),
        (
            json!({"from": 4, "to": 12}),
            json!({"from": 2.0, "to": 6.0})
        )
    );
    assert_eq!(
        iv(c),
        (
            json!({"from": 12, "to": 12}),
            json!({"from": 6.0, "to": 6.0})
        )
    );
    assert_eq!(
        iv(e),
        (
            json!({"from": 12, "to": 17}),
            json!({"from": 6.0, "to": 10.0})
        )
    );
    // the path: each parent is the previous state
    for w in docs.windows(2) {
        assert_eq!(w[1]["parent"], w[0]["state"]);
    }
    let tw = |doc: &Value, mp: &str, m: &str| at(doc, mp, m)["time_weighted"].clone();
    // the server: state 1 at 0 s, 0 at 2, 1 at 3, 0 at 5, 1 at 9
    assert_eq!(at(b, SERVER, STATE)["count"], 2);
    assert_eq!(
        at(b, SERVER, STATE)["initial"],
        json!({"status": "tuple", "time": 2.0, "value": 0.0})
    );
    // 0 from 2 to 3, 1 from 3 to 5, 0 from 5 to 6
    assert_eq!(
        tw(b, SERVER, STATE),
        json!({"from": 2.0, "to": 6.0, "integral": 2.0, "mean": 0.5})
    );
    // the response time of t = 2 s is the decision child's (recorded after the resume)
    assert_eq!(at(a, SCENARIO, RT)["count"], 0);
    assert_eq!(
        at(b, SCENARIO, RT)["first"],
        json!({"time": 2.0, "value": 2.0})
    );
    // the server the decision adds: its series exist from the child on; busy from 6 to 8 s
    let added: Vec<String> = series(b)
        .keys()
        .filter(|(mp, m)| {
            m == STATE && mp != SERVER && !series(a).contains_key(&(mp.clone(), m.clone()))
        })
        .map(|k| k.0.clone())
        .collect();
    let busy = added
        .iter()
        .find(|mp| at(b, mp, STATE)["count"] == 1)
        .expect("the added server's state in the child")
        .clone();
    assert_eq!(
        at(b, &busy, STATE)["last"],
        json!({"time": 6.0, "value": 1.0})
    );
    // the tuple at the pause time 6 s belongs to the interval before: a zero-length span
    assert_eq!(
        tw(b, &busy, STATE),
        json!({"from": 6.0, "to": 6.0, "integral": 0.0, "mean": null})
    );
    // the empty interval: no tuple of any series, the last ones carried
    for s in c["series"].as_array().unwrap() {
        assert_eq!(s["count"], 0, "{s}");
        assert_eq!(s["mean"], Value::Null, "{s}");
    }
    assert_eq!(
        at(c, &busy, STATE)["initial"],
        json!({"status": "tuple", "time": 6.0, "value": 1.0})
    );
    // carried through the empty interval's snapshot: busy from 6 to 8, idle to 10
    assert_eq!(
        at(e, &busy, STATE)["initial"],
        json!({"status": "tuple", "time": 6.0, "value": 1.0})
    );
    assert_eq!(
        tw(e, &busy, STATE),
        json!({"from": 6.0, "to": 10.0, "integral": 2.0, "mean": 0.5})
    );
    // the first server: 0 from 6 to 9, 1 from 9 to 10
    assert_eq!(
        tw(e, SERVER, STATE),
        json!({"from": 6.0, "to": 10.0, "integral": 1.0, "mean": 0.25})
    );
    // sample and time weighting differ: the whole run's server state, 3 of 5 tuples busy, but
    // busy 5 of 10 s
    let whole = json_file(&d.join("whole.json"));
    assert_eq!(at(&whole, SERVER, STATE)["mean"], 0.6);
    assert_eq!(tw(&whole, SERVER, STATE)["mean"], 0.5);
    // every interval against its export; the path adds up to the uninterrupted run
    let mut prior = BTreeMap::new();
    for (doc, n) in docs.iter().zip(["a", "b", "c", "e"]) {
        check(doc, &read(&d.join(format!("{n}.csv"))), &mut prior);
    }
    let mut sum: BTreeMap<Key, (u64, f64)> = BTreeMap::new();
    for doc in &docs {
        for (k, (n, i)) in totals(doc) {
            let x = sum.entry(k).or_default();
            x.0 += n;
            x.1 += i;
        }
    }
    assert_eq!(sum, totals(&whole));
}

/// The issue's setting: the V4 model in the Slingshot flavour (CPU-utilisation policies that
/// scale out and in), paused every 30 s to 300 s: ten intervals. Every interval's summary
/// equals the statistics of its interval export; without `--measurements` and in cumulative
/// scope the summaries are the same; the counts add up to the uninterrupted run's rows.
#[test]
fn ten_intervals_agree_with_their_exports() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let d = tmp("ten");
    let p = |n: &str| d.join(n).to_string_lossy().into_owned();
    let model = root()
        .join("reference/slingshot/models/spd_v4_cpu")
        .to_string_lossy()
        .into_owned();
    let flags = [
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
        "300",
    ];
    let mut a = vec!["run", "--model", &model];
    a.extend(flags);
    let whole = (p("whole.csv"), p("whole.json"));
    a.extend(["--measurements", &whole.0, "--summary", &whole.1]);
    ok(&a);
    // three chains: interval scope with and without --measurements, cumulative scope
    for (tag, scope, meas) in [
        ("i", "interval", true),
        ("n", "interval", false),
        ("c", "cumulative", false),
    ] {
        for k in 0..10 {
            let f = |n: &str| p(&format!("{tag}{k}{n}"));
            let prev = p(&format!("{tag}{}.snap", k as i64 - 1));
            let (snap, csv, json, t) = (
                f(".snap"),
                f(".csv"),
                f(".json"),
                format!("{}", 30 * (k + 1)),
            );
            let mut a: Vec<&str> = if k == 0 {
                let mut a = vec!["run", "--model", &model];
                a.extend(flags);
                a.extend(["--measurements-scope", scope]);
                a
            } else {
                vec!["resume", &prev, "--model", &model]
            };
            if k < 9 {
                a.extend(["--pause-at", &t, "--snapshot", &snap]);
            }
            if meas {
                a.extend(["--measurements", &csv]);
            }
            a.extend(["--summary", &json]);
            let err = ok(&a);
            let (from, to, _, _) = interval(&err);
            let doc = json_file(Path::new(&json));
            assert_eq!(doc["interval"]["tuples"], json!({"from": from, "to": to}));
        }
    }
    let doc = |n: String| json_file(&d.join(n));
    let mut prior = BTreeMap::new();
    let mut sum: BTreeMap<String, u64> = BTreeMap::new();
    let mut end = 0.0;
    for k in 0..10 {
        let i = doc(format!("i{k}.json"));
        // adjacent in time
        assert_eq!(i["interval"]["time"]["from"].as_f64().unwrap(), end);
        end = i["interval"]["time"]["to"].as_f64().unwrap();
        check(&i, &read(&d.join(format!("i{k}.csv"))), &mut prior);
        assert_eq!(
            read(&d.join(format!("i{k}.json"))),
            read(&d.join(format!("n{k}.json")))
        );
        assert_eq!(
            without_ids(doc(format!("c{k}.json"))),
            without_ids(i.clone()),
            "{k}"
        );
        for s in i["series"].as_array().unwrap() {
            *sum.entry(s["metric"].as_str().unwrap().to_string())
                .or_default() += s["count"].as_u64().unwrap();
        }
    }
    assert_eq!(end, 300.0);
    // the counts per metric add up to the uninterrupted run's rows (measuring-point keys of
    // containers created after a resume can differ from the uninterrupted run's, see
    // measurements_cli.rs)
    let mut rows_per_metric: BTreeMap<String, u64> = BTreeMap::new();
    for ((_, m), r) in rows(&read(Path::new(&whole.0))) {
        *rows_per_metric.entry(m).or_default() += r.len() as u64;
    }
    assert_eq!(sum, rows_per_metric);
    check(
        &json_file(Path::new(&whole.1)),
        &read(Path::new(&whole.0)),
        &mut BTreeMap::new(),
    );
}

/// `--summary-series` and `--summary-stats` select series and statistics; `count` is always
/// written; a selection without `--summary`, or an unknown statistic, is refused.
#[test]
fn selections() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let d = tmp("select");
    let p = |n: &str| d.join(n).to_string_lossy().into_owned();
    let model = minimal_model(&d);
    let run = |extra: &[&str]| -> Vec<String> {
        let mut a = vec!["run", "--model", &model];
        a.extend(FLAGS);
        a.extend(extra);
        a.into_iter().map(String::from).collect()
    };
    fn strs(v: &[String]) -> Vec<&str> {
        v.iter().map(String::as_str).collect()
    }
    ok(&strs(&run(&[
        "--summary",
        &p("s.json"),
        "--summary-series",
        "rt",
        "--summary-series",
        &format!("state[{}]", &SERVER[29..52]),
        "--summary-stats",
        "max,mean",
    ])));
    let s = json_file(&d.join("s.json"));
    assert_eq!(s["stats"], json!(["count", "mean", "max"]));
    let keys: Vec<Key> = series(&s).into_keys().collect();
    assert!(
        keys.contains(&(SERVER.to_string(), STATE.to_string())),
        "{keys:?}"
    );
    assert!(
        keys.iter()
            .all(|(mp, m)| m == RT || (mp == SERVER && m == STATE)),
        "{keys:?}"
    );
    assert!(keys.iter().any(|(_, m)| m == RT));
    for x in s["series"].as_array().unwrap() {
        let o = x.as_object().unwrap();
        let names: Vec<&str> = o.keys().map(String::as_str).collect();
        assert_eq!(names.len(), 6, "{x}");
        assert!(o.contains_key("unit") && o.contains_key("count") && o.contains_key("max"));
    }
    for bad in [
        vec!["--summary-series", "rt"],
        vec!["--summary", &p("x.json"), "--summary-stats", "median"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_simoxide"))
            .args(run(&bad))
            .output()
            .unwrap();
        assert!(!out.status.success(), "{bad:?}");
    }
}

//! Runtime metadata (issue #9): `--model-metadata` writes the model's definitions with a content
//! hash, `--runtime-metadata` per invocation the deployment at its start, every change with its
//! time, cause and affected ids, the policy state, the retired elements and what each
//! measurement series refers to (docs/guide/runtime-metadata.md).
//!
//! The model is Slingshot's minimal example (an `AppServer` behind a load balancer) with its own
//! policies on the elastic infrastructure (size 1 to 3), as in `decision_cli.rs`: `_out` scales
//! out by one at t = 2 s, `_in` scales in by one at t = 30 s; flags as in the issues:
//! `--flavour slingshot --mode fast --quirks none --spd-quirks fixed`, 60 s.

#![cfg(all(feature = "spd", feature = "reconfiguration"))]

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("metadata-cli-{name}"));
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

fn json_of(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

const GROUP: &str = "_DC8fM8pgEe6u3p81um7oyQ";
/// The unit of the elastic infrastructure: `AppServer`, its CPU (FCFS, rate 1.0).
const APP: &str = "_GAeeUCpXEeKNWc2UWjbmuw";
const APP_CPU: &str = "_KFRwgCpXEeKNWc2UWjbmuw";
/// The load balancer's container and its CPU (FCFS, rate 100000).
const LB: &str = "_R8hGoLwqEe6VP9cIU1FUYw";
const LB_CPU: &str = "_TcRpMLwqEe6VP9cIU1FUYw";
const LINK: &str = "_jW8l4LwZEe6i2ICnm9lfkw";
const CPU_TYPE: &str = "_oro4gG3fEdy4YaaT-RYrLQ";

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
    <unit href="My.resourceenvironment#{APP}"/>
    <targetConstraints xsi:type="target:TargetGroupSizeConstraint" minSize="1" maxSize="3"/>
  </targetGroups>
</spd:SPD>
"#
    ));
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
    "60",
];

const KINDS: [&str; 5] = [
    "containers",
    "linking_resources",
    "assemblies",
    "allocations",
    "target_groups",
];

fn ids(v: &Value) -> Vec<&str> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect()
}

/// `d` with the change `c` applied (docs/guide/runtime-metadata.md, "Replaying").
fn replay(d: &Value, c: &Value) -> Value {
    let mut d = d.clone();
    for k in KINDS {
        let list = d[k].as_array_mut().unwrap();
        if let Some(r) = c["removed"][k].as_array() {
            list.retain(|e| !r.contains(&e["id"]));
        }
        if let Some(ch) = c["changed"][k].as_array() {
            for e in ch {
                let i = list.iter().position(|x| x["id"] == e["id"]).unwrap();
                list[i] = e.clone();
            }
        }
        if let Some(a) = c["added"][k].as_array() {
            list.extend(a.iter().cloned());
        }
    }
    d
}

/// Sorts the elements of every kind by id (replaying appends where the model inserts).
fn by_id(d: &Value) -> Value {
    let mut d = d.clone();
    for k in KINDS {
        d[k].as_array_mut()
            .unwrap()
            .sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    }
    d
}

/// The series keys of a `measurements.csv`.
fn csv_series(p: &Path) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = std::fs::read_to_string(p)
        .unwrap()
        .lines()
        .skip(1)
        .map(|l| {
            let mut f = l.split(',');
            (f.next().unwrap().into(), f.next().unwrap().into())
        })
        .collect();
    v.dedup();
    v
}

fn series_of<'a>(r: &'a Value, mp: &str, metric: &str) -> &'a Value {
    r["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["measuring_point"] == mp && s["metric"] == metric)
        .unwrap_or_else(|| panic!("no metadata for ({mp}, {metric})"))
}

/// A scale-out at 2 s and a scale-in at 30 s, uninterrupted: the deployment at the start is the
/// model's; each adjustment is a change at its time with the copy's ids; the copy's series stay
/// described after the scale-in removed it.
#[test]
fn scaling_changes_and_series() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let d = tmp("scaling");
    let m = model(&d, &[("_out", true, 1, 2.0), ("_in", true, -1, 30.0)]);
    let p = |f: &str| d.join(f).to_string_lossy().into_owned();
    let run: Vec<&str> = [&["run", "--model", &m][..], &FLAGS].concat();
    let model_file = p("model.json");
    let rt = p("rt.json");
    let csv = p("m.csv");
    let args = [
        &run[..],
        &[
            "--model-metadata",
            &model_file,
            "--runtime-metadata",
            &rt,
            "--measurements",
            &csv,
        ],
    ]
    .concat();
    ok(&args);
    let mf = json_of(Path::new(&model_file));
    let r = json_of(Path::new(&rt));
    assert_eq!(mf["format"], "simoxide-model");
    assert_eq!(mf["version"], 1);
    assert_eq!(r["format"], "simoxide-runtime");
    assert_eq!(r["version"], 1);
    assert_eq!(r["model"]["hash"], mf["hash"]);
    assert_eq!(
        r["run"],
        json!({"parent": null, "state": null, "status": "finished"})
    );

    // the start: the model's deployment, exactly
    let start = &r["start"];
    assert_eq!(start["time_ns"], 0);
    assert_eq!(start["generation"], 0);
    let dep = &start["deployment"];
    assert_eq!(dep, &mf["deployment"]);
    assert_eq!(ids(&dep["containers"]), [APP, LB]);
    let app = &dep["containers"][0];
    assert_eq!(app["name"], "AppServer");
    assert_eq!(app["in_model"], true);
    assert_eq!(app["unit"], APP);
    assert_eq!(
        app["resources"],
        json!([{"id": APP_CPU, "in_model": true, "type": "CPU", "type_id": CPU_TYPE,
                "scheduling": "FCFS", "rate": {"expression": "1.0", "value": 1.0},
                "replicas": 1, "mttf": 0.0, "mttr": 0.0}])
    );
    assert_eq!(dep["containers"][1]["unit"], Value::Null);
    assert_eq!(
        dep["containers"][1]["resources"][0]["rate"],
        json!({"expression": "100000", "value": 100000.0})
    );
    assert_eq!(dep["containers"][1]["resources"][0]["id"], LB_CPU);
    assert_eq!(ids(&dep["linking_resources"]), [LINK]);
    assert_eq!(dep["linking_resources"][0]["connected"], json!([APP, LB]));
    assert_eq!(dep["assemblies"].as_array().unwrap().len(), 2);
    assert_eq!(dep["allocations"].as_array().unwrap().len(), 2);
    assert_eq!(
        dep["target_groups"],
        json!([{"id": GROUP, "name": "EI TargetGroup", "kind": "elastic_infrastructure",
                "unit": APP, "configured": true, "elements": [APP]}])
    );
    let pol = &start["policies"];
    assert_eq!(pol["decisions_made"], 0);
    assert_eq!(pol["pending_decision"], Value::Null);
    assert_eq!(pol["target_groups"][0]["size"], 1);
    assert_eq!(pol["policies"][0]["state"]["scales"], 0);
    assert_eq!(
        pol["policies"][0]["state"]["latest_adjustment"],
        Value::Null
    );

    // the changes: the scale-out at 2 s, the scale-in at 30 s
    let ch = r["changes"].as_array().unwrap();
    assert_eq!(ch.len(), 2, "{}", r["changes"]);
    let (out, inn) = (&ch[0], &ch[1]);
    assert_eq!(out["time_ns"], 2_000_000_000i64);
    assert_eq!(out["time"], 2.0);
    assert_eq!(out["cause"]["type"], "policy");
    assert_eq!(out["cause"]["policy"], "_out");
    assert_eq!(out["cause"]["target_group"], GROUP);
    assert_eq!(
        out["cause"]["adjustment"],
        json!({"type": "StepAdjustment", "stepValue": 1})
    );
    assert_eq!(
        out["outcome"],
        json!({"ok": true, "size_before": 1, "size_after": 2})
    );
    let added = out["added"]["containers"].as_array().unwrap();
    assert_eq!(added.len(), 1);
    let copy = &added[0];
    let copy_id = copy["id"].as_str().unwrap();
    assert_eq!(copy["name"], "AppServer_1");
    assert_eq!(copy["in_model"], false);
    assert_eq!(copy["unit"], APP);
    let copy_cpu = &copy["resources"][0];
    assert_eq!(copy_cpu["type"], "CPU");
    assert_eq!(copy_cpu["scheduling"], "FCFS");
    assert_eq!(copy_cpu["rate"], json!({"expression": "1.0", "value": 1.0}));
    assert_eq!(copy_cpu["in_model"], false);
    let copy_cpu_id = copy_cpu["id"].as_str().unwrap();
    // the copy's assembly and allocation context, on the copy
    let asm = out["added"]["assemblies"].as_array().unwrap();
    let alloc = out["added"]["allocations"].as_array().unwrap();
    assert_eq!((asm.len(), alloc.len()), (1, 1));
    assert_eq!(alloc[0]["container"], copy_id);
    assert_eq!(alloc[0]["assembly"], asm[0]["id"]);
    assert_eq!(out["removed"], json!({}));
    assert_eq!(
        out["changed"]["target_groups"][0]["elements"],
        json!([APP, copy_id])
    );
    assert_eq!(
        out["changed"]["linking_resources"][0]["connected"],
        json!([APP, LB, copy_id])
    );
    assert_eq!(inn["time_ns"], 30_000_000_000i64);
    assert_eq!(inn["cause"]["policy"], "_in");
    assert_eq!(
        inn["outcome"],
        json!({"ok": true, "size_before": 2, "size_after": 1})
    );
    // the newest copy leaves, with its assembly and allocation context
    assert_eq!(inn["added"], json!({}));
    assert_eq!(inn["removed"]["containers"], json!([copy_id]));
    assert_eq!(inn["removed"]["assemblies"], json!([asm[0]["id"]]));
    assert_eq!(inn["removed"]["allocations"], json!([alloc[0]["id"]]));
    assert_eq!(inn["changed"]["target_groups"][0]["elements"], json!([APP]));
    assert!(inn["generation"].as_u64() > out["generation"].as_u64());

    // the end: the policies' state, the copy retired
    let end = &r["end"];
    assert_eq!(end["time_ns"], 60_000_000_000i64);
    assert_eq!(end["generation"], inn["generation"]);
    let pol = &end["policies"];
    assert_eq!(pol["target_groups"][0]["size"], 1);
    assert_eq!(pol["target_groups"][0]["enactments"], 2);
    assert_eq!(
        pol["target_groups"][0]["last_enactment"],
        json!({"time": 30.0, "policy": "_in"})
    );
    assert_eq!(pol["policies"][0]["state"]["latest_adjustment"], 2.0);
    assert_eq!(pol["policies"][1]["state"]["scales"], 1);
    assert_eq!(ids(&r["retired"]["containers"]), [copy_id]);
    assert_eq!(
        r["retired"]["containers"][0]["resources"][0]["id"],
        copy_cpu_id
    );

    // every series of measurements.csv is described; the copy's CPU series still names the
    // retired copy and its monitor clone
    let csv_keys = csv_series(Path::new(&csv));
    assert!(
        csv_keys.iter().any(|(mp, _)| mp.contains(copy_cpu_id)),
        "the copy's CPU has tuples: {csv_keys:?}"
    );
    for (mp, metric) in &csv_keys {
        let s = series_of(&r, mp, metric);
        assert!(s["unit"].is_string(), "{s}");
    }
    let key = format!("ActiveResourceMeasuringPoint[{copy_cpu_id}|replicaID=0]");
    let s = series_of(&r, &key, "State of Active Resource Tuple");
    assert_eq!(s["unit"], "1");
    assert_eq!(
        s["point"],
        json!({"class": "ActiveResourceMeasuringPoint", "processing_resource": copy_cpu_id,
               "replica": 0, "container": copy_id, "resource_type": CPU_TYPE})
    );
    assert_eq!(s["monitors"].as_array().unwrap().len(), 1, "{s}");
    assert!(s["monitors"][0]["specification"].is_string(), "{s}");
    assert_eq!(s["unavailable"], json!([]));
    let key = format!("ActiveResourceMeasuringPoint[{APP_CPU}|replicaID=0]");
    let s = series_of(&r, &key, "State of Active Resource Tuple");
    assert_eq!(s["point"]["container"], APP);
    assert_eq!(s["monitors"][0]["id"], "_hVB9RtqvEe2AxOOsE9OYvg");
    let s = series_of(
        &r,
        "UsageScenarioMeasuringPoint[_LPnI8CHdEd6lJo4DCALHMw]",
        "Response Time Tuple",
    );
    assert_eq!(s["unit"], "s");
    assert_eq!(s["point"]["usage_scenario"], "_LPnI8CHdEd6lJo4DCALHMw");

    // the model file: the scaling definitions; the same file from a resume of the model
    assert_eq!(
        mf["scaling"]["target_groups"][0]["constraints"],
        json!([{"type": "TargetGroupSizeConstraint", "min": 1, "max": 3}])
    );
    assert_eq!(ids(&mf["scaling"]["policies"]), ["_out", "_in"]);
    let snap = p("s.snap");
    let args = [&run[..], &["--pause-at", "10", "--snapshot", &snap]].concat();
    ok(&args);
    let again = p("model2.json");
    ok(&["resume", &snap, "--model", &m, "--model-metadata", &again]);
    assert_eq!(
        std::fs::read(&model_file).unwrap(),
        std::fs::read(&again).unwrap()
    );
    std::fs::remove_dir_all(&d).unwrap();
}

/// Runs `m` with `flags` uninterrupted and as a chain of invocations paused at `pauses` (one
/// process each, interval scope), each with its runtime file; checks that every interval's
/// measurements are described in its own file, that the files chain (each start is the previous
/// file's deployment replayed, its policy state, generation and time the previous end's, its
/// `parent` the previous `state`), that their changes concatenate to the uninterrupted run's
/// and that the last file's end, retired elements and series are the uninterrupted run's.
/// Returns the uninterrupted run's file and the chain's.
fn chain_equals_whole(d: &Path, m: &str, flags: &[&str], pauses: &[&str]) -> (Value, Vec<Value>) {
    let p = |f: &str| d.join(f).to_string_lossy().into_owned();
    let run: Vec<&str> = [&["run", "--model", m][..], flags].concat();
    let whole = p("whole.json");
    ok(&[&run[..], &["--runtime-metadata", &whole]].concat());
    let whole = json_of(Path::new(&whole));
    let mut files: Vec<Value> = Vec::new();
    for i in 0..=pauses.len() {
        let snap_in = p(&format!("s{}.snap", i.wrapping_sub(1)));
        let mut args: Vec<String> = match i {
            0 => run.iter().map(|s| s.to_string()).collect(),
            _ => ["resume", &snap_in, "--model", m]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        };
        args.extend([
            "--measurements-scope".to_string(),
            "interval".into(),
            "--runtime-metadata".into(),
            p(&format!("r{i}.json")),
            "--measurements".into(),
            p(&format!("m{i}.csv")),
        ]);
        if let Some(t) = pauses.get(i) {
            args.extend(["--pause-at".into(), t.to_string()]);
            args.extend(["--snapshot".into(), p(&format!("s{i}.snap"))]);
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        ok(&args);
        let r = json_of(&d.join(format!("r{i}.json")));
        for (mp, metric) in csv_series(&d.join(format!("m{i}.csv"))) {
            series_of(&r, &mp, &metric);
        }
        files.push(r);
    }
    assert_eq!(files[0]["start"], whole["start"]);
    let mut changes = Vec::new();
    for (i, r) in files.iter().enumerate() {
        changes.extend(r["changes"].as_array().unwrap().iter().cloned());
        if i + 1 < files.len() {
            let next = &files[i + 1];
            let mut dep = r["start"]["deployment"].clone();
            for c in r["changes"].as_array().unwrap() {
                dep = replay(&dep, c);
            }
            assert_eq!(
                by_id(&next["start"]["deployment"]),
                by_id(&dep),
                "start {}",
                i + 1
            );
            assert_eq!(next["start"]["policies"], r["end"]["policies"]);
            assert_eq!(next["start"]["generation"], r["end"]["generation"]);
            assert_eq!(next["start"]["time_ns"], r["end"]["time_ns"]);
            assert_eq!(r["run"]["status"], "paused");
            assert_eq!(next["run"]["parent"], r["run"]["state"]);
            assert!(r["run"]["state"].is_string());
        }
    }
    assert_eq!(Value::Array(changes), whole["changes"]);
    let last = files.last().unwrap();
    for k in ["end", "retired", "series"] {
        assert_eq!(last[k], whole[k], "{k}");
    }
    (whole, files)
}

/// The small model paused at 1 s, 2 s (right after the scale-out), 20 s and 45 s: the chain's
/// runtime files give the uninterrupted run's metadata.
#[test]
fn a_chain_of_resumes_gives_the_uninterrupted_runs_metadata() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let d = tmp("chain");
    let m = model(&d, &[("_out", true, 1, 2.0), ("_in", true, -1, 30.0)]);
    let (_, files) = chain_equals_whole(&d, &m, &FLAGS, &["1", "2", "20", "45"]);
    // the first pause (1 s) is before the scale-out, the second (2 s) right after it
    assert_eq!(files[0]["changes"], json!([]));
    assert_eq!(files[1]["changes"].as_array().unwrap().len(), 1);
    assert_eq!(files[3]["changes"].as_array().unwrap().len(), 1);
    std::fs::remove_dir_all(&d).unwrap();
}

/// The V4 model of the issues (CPU-utilisation policies that scale out and in), seed 7, 600 s,
/// paused at 90 s and 300 s: the start is the model's two containers, every adjustment of the
/// decision log is a change at its time with its policy, and the chain gives the uninterrupted
/// run's metadata, the removed copies' series included.
#[test]
fn v4_cpu_chain_equals_the_uninterrupted_run() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let d = tmp("v4");
    let m = root()
        .join("reference/slingshot/models/spd_v4_cpu")
        .to_string_lossy()
        .into_owned();
    let flags = [&FLAGS[..8], &["--seed", "7", "--max-sim-time", "600"]].concat();
    let (whole, _) = chain_equals_whole(&d, &m, &flags, &["90", "300"]);
    assert_eq!(ids(&whole["start"]["deployment"]["containers"]), [APP, LB]);
    // the decision log's adjustments, one by one
    let log = d.join("spd.jsonl").to_string_lossy().into_owned();
    let run: Vec<&str> = [&["run", "--model", &m][..], &flags, &["--spd-log", &log]].concat();
    ok(&run);
    let adjusts: Vec<Value> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|r| r["k"] == "adjust")
        .collect();
    let ch = whole["changes"].as_array().unwrap();
    assert!(ch.len() >= 10, "{}", ch.len());
    assert_eq!(ch.len(), adjusts.len());
    for (c, a) in ch.iter().zip(&adjusts) {
        assert_eq!(c["time"], a["t"], "{c}");
        assert_eq!(c["cause"]["policy"], a["policy"], "{c}");
        let n = |c: &Value, k: &str| c[k]["containers"].as_array().map_or(0, Vec::len);
        let (before, after) = (
            c["outcome"]["size_before"].as_i64().unwrap(),
            c["outcome"]["size_after"].as_i64().unwrap(),
        );
        assert_eq!(
            n(c, "added") as i64 - n(c, "removed") as i64,
            after - before,
            "{c}"
        );
    }
    // a removed copy's CPU series still names it, among the retired containers
    let retired = ids(&whole["retired"]["containers"]);
    assert!(!retired.is_empty());
    let described = whole["series"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| retired.contains(&s["point"]["container"].as_str().unwrap_or("")))
        .count();
    assert!(described > 0);
    std::fs::remove_dir_all(&d).unwrap();
}

/// `--pause-on-decision`: the pending recommendation is in the policy state, not a change; the
/// actions `--decide` selects are changes with the decision's number, at its time, after a
/// `--change` step of the same resume.
#[test]
fn decisions_and_change_steps() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let d = tmp("decisions");
    let m = model(&d, &[("_out", true, 1, 2.0), ("_in", false, -1, 0.0)]);
    let p = |f: &str| d.join(f).to_string_lossy().into_owned();
    let run: Vec<&str> = [&["run", "--model", &m][..], &FLAGS].concat();
    let (snap, r0) = (p("d.snap"), p("r0.json"));
    let args = [
        &run[..],
        &[
            "--pause-on-decision",
            "--snapshot",
            &snap,
            "--runtime-metadata",
            &r0,
        ],
    ]
    .concat();
    ok(&args);
    let r0 = json_of(Path::new(&r0));
    assert_eq!(r0["changes"], json!([]));
    assert_eq!(r0["end"]["time_ns"], 2_000_000_000i64);
    let pending = &r0["end"]["policies"]["pending_decision"];
    assert_eq!(pending["decision"], 1);
    assert_eq!(pending["recommended"][0]["policy"], "_out");
    assert_eq!(r0["end"]["policies"]["decisions_made"], 1);

    let changes = d.join("faster.changes");
    std::fs::write(&changes, format!("rate {APP}:CPU 2.5\n")).unwrap();
    let r1 = p("r1.json");
    ok(&[
        "resume",
        &snap,
        "--model",
        &m,
        "--change",
        &changes.to_string_lossy(),
        "--decide",
        "_out,_out,_out",
        "--runtime-metadata",
        &r1,
    ]);
    let r1 = json_of(Path::new(&r1));
    assert_eq!(r1["start"]["policies"], r0["end"]["policies"]);
    let ch = r1["changes"].as_array().unwrap();
    assert_eq!(ch.len(), 4, "{}", r1["changes"]);
    // the change file: the unit's rate, at the resume
    assert_eq!(ch[0]["time_ns"], 2_000_000_000i64);
    assert_eq!(
        ch[0]["cause"],
        json!({"type": "change", "file": changes.to_string_lossy()})
    );
    assert_eq!(ch[0]["outcome"], Value::Null);
    let app = &ch[0]["changed"]["containers"];
    assert_eq!(ids(app), [APP]);
    assert_eq!(
        app[0]["resources"][0]["rate"],
        json!({"expression": "2.5", "value": 2.5})
    );
    // three actions of decision 1: two scale-outs, the third at the maximum size
    let sizes: Vec<(Value, Value)> = ch[1..]
        .iter()
        .map(|c| {
            (
                c["outcome"]["size_before"].clone(),
                c["outcome"]["size_after"].clone(),
            )
        })
        .collect();
    assert_eq!(
        sizes,
        [
            (json!(1), json!(2)),
            (json!(2), json!(3)),
            (json!(3), json!(3))
        ]
    );
    for c in &ch[1..] {
        assert_eq!(c["time_ns"], 2_000_000_000i64);
        assert_eq!(c["cause"]["type"], "decision");
        assert_eq!(c["cause"]["decision"], 1);
        assert_eq!(c["cause"]["policy"], "_out");
    }
    for c in &ch[1..3] {
        let added = c["added"]["containers"].as_array().unwrap();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0]["unit"], APP);
        // a copy of the changed unit: its rate
        assert_eq!(
            added[0]["resources"][0]["rate"],
            json!({"expression": "2.5", "value": 2.5})
        );
    }
    assert_eq!(
        (&ch[3]["added"], &ch[3]["removed"], &ch[3]["changed"]),
        (&json!({}), &json!({}), &json!({}))
    );
    assert_eq!(r1["end"]["policies"]["pending_decision"], Value::Null);
    assert_eq!(r1["end"]["policies"]["target_groups"][0]["size"], 3);
    // a decision changes no policy state: the policy scaled once (its recommendation)
    assert_eq!(r1["end"]["policies"]["policies"][0]["state"]["scales"], 1);
    std::fs::remove_dir_all(&d).unwrap();
}

/// The Slingshot-exact engine does not record runtime metadata.
#[test]
fn slingshot_exact_is_refused() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let d = tmp("exact");
    let m = model(&d, &[("_out", true, 1, 2.0)]);
    let rt = d.join("rt.json").to_string_lossy().into_owned();
    let err = fails(&[
        "run",
        "--model",
        &m,
        "--flavour",
        "slingshot-exact",
        "--max-sim-time",
        "10",
        "--runtime-metadata",
        &rt,
    ]);
    assert!(err.contains("--runtime-metadata"), "{err}");
    std::fs::remove_dir_all(&d).unwrap();
}

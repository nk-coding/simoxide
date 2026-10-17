//! `simoxide run` and `simoxide resume` with an HPA autoscaler (`docs/guide/hpa.md`; features
//! `spd` and `reconfiguration`) in new processes: the `.hpa.json` next to the model is picked
//! up, its syncs go to the decision log, a run paused and resumed from its snapshot continues
//! like the uninterrupted one, parameters change at the resume (`--hpa-set`, `hpa` lines of a
//! change file), and the Slingshot-exact engine refuses the HPA.

#![cfg(all(feature = "spd", feature = "reconfiguration"))]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("hpa-cli-{name}"));
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

const SPD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<spd:SPD xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:spd="http://palladiosimulator.org/ScalingPolicyDefinition/1.0" xmlns:targets="http://palladiosimulator.org/ScalingPolicyDefinition/Targets/1.0" id="_PKdecLwZEe6i2ICnm9lfkw">
  <targetGroups xsi:type="targets:ElasticInfrastructure" id="_DC8fM8pgEe6u3p81um7oyQ" entityName="app">
    <unit href="My.resourceenvironment#_GAeeUCpXEeKNWc2UWjbmuw"/>
  </targetGroups>
</spd:SPD>
"#;

const HPA: &str = r#"{"controller": {"syncPeriod": "15s"},
 "items": [{"apiVersion": "autoscaling/v2", "kind": "HorizontalPodAutoscaler",
   "metadata": {"name": "app"},
   "spec": {"scaleTargetRef": {"kind": "ElasticInfrastructure", "name": "app"},
            "minReplicas": 1, "maxReplicas": 8,
            "metrics": [{"type": "Resource", "resource": {"name": "cpu",
                         "target": {"type": "Utilization", "averageUtilization": 50}}}]},
   "simoxide": {"startupDelay": "20s"}}]}"#;

/// The V4 model (1.6 offered cores) with a target group, no policies, monitors for the
/// SimuLizar flavour and the HPA.
fn model(d: &Path) -> String {
    const UTIL: &str = "_QIb6cikUEeSuf8LV7cHLgA";
    let src = root().join("reference/slingshot/models/spd_v4_cpu");
    let out = d.join("hpa_cpu");
    std::fs::create_dir_all(&out).unwrap();
    for e in std::fs::read_dir(&src).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let mut text = std::fs::read_to_string(&p).unwrap();
        if name.ends_with(".spd") {
            text = SPD.into();
        }
        if name.ends_with(".monitorrepository") {
            let mut util = false;
            let mut lines = Vec::new();
            for l in text.lines() {
                let mut l = l.to_string();
                if l.contains(UTIL) {
                    util = true;
                } else if util && l.contains("processingType") {
                    l = l.replace(
                        "xsi:type=\"monitorrepository:FeedThrough\"",
                        "xsi:type=\"monitorrepository:TimeDriven\" windowLength=\"10.0\" windowIncrement=\"10.0\"",
                    );
                    util = false;
                }
                lines.push(l);
            }
            text = lines.join("\n");
        }
        std::fs::write(out.join(&name), text).unwrap();
    }
    std::fs::write(out.join("app.hpa.json"), HPA).unwrap();
    out.to_string_lossy().into_owned()
}

fn records(p: &Path, k: &str) -> Vec<Value> {
    std::fs::read_to_string(p)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v["k"] == k)
        .collect()
}

#[test]
fn hpa_runs_pauses_and_resumes() {
    let d = tmp("run");
    let m = model(&d);
    let p = |n: &str| d.join(n).to_string_lossy().into_owned();
    let common = ["--seed", "3", "--max-sim-time", "600"];
    // the whole run
    let mut args = vec!["run", "--model", &m, "--spd-log"];
    let whole = p("whole.jsonl");
    args.push(&whole);
    args.extend(common);
    let wm = p("whole.csv");
    args.extend(["--measurements", &wm]);
    let err = ok(&args);
    assert!(err.contains("hpa app: syncs=40"), "{err}");
    let syncs = records(Path::new(&whole), "hpa");
    assert_eq!(syncs.len(), 40);
    let max = syncs.iter().map(|r| r["desired"].as_i64().unwrap()).max();
    assert!(max >= Some(3), "{max:?}");
    assert!(!records(Path::new(&whole), "adjust").is_empty());
    assert!(!records(Path::new(&whole), "hpa_ready").is_empty());

    // paused at 250 s with a snapshot, resumed to the end: the same log and measurements
    let snap = p("at250.snap");
    let first = p("first.jsonl");
    let mut args = vec!["run", "--model", &m, "--spd-log", &first];
    args.extend(common);
    args.extend(["--pause-at", "250", "--snapshot", &snap]);
    ok(&args);
    let rest = p("rest.jsonl");
    let rm = p("rest.csv");
    ok(&[
        "resume",
        &snap,
        "--model",
        &m,
        "--spd-log",
        &rest,
        "--measurements",
        &rm,
    ]);
    let joined =
        std::fs::read_to_string(&first).unwrap() + &std::fs::read_to_string(&rest).unwrap();
    assert_eq!(joined, std::fs::read_to_string(&whole).unwrap());

    // parameters at the resume: a change file's `hpa` line and --hpa-set
    let ch = p("cap.changes");
    std::fs::write(&ch, "# cap the replicas\nhpa app maxReplicas 2\n").unwrap();
    let capped = p("capped.jsonl");
    let err = ok(&[
        "resume",
        &snap,
        "--model",
        &m,
        "--spd-log",
        &capped,
        "--change",
        &ch,
        "--hpa-set",
        "app.averageUtilization=70",
    ]);
    assert!(err.contains("hpa app: maxReplicas = 2"), "{err}");
    let recs = records(Path::new(&capped), "hpa");
    assert!(recs.iter().all(|r| r["target"] == 70));
    assert!(
        recs.iter()
            .skip(2)
            .all(|r| r["desired"].as_i64().unwrap() <= 2),
        "{recs:?}"
    );
    // errors: an unknown autoscaler, a run without HPA, the Slingshot-exact engine
    let (good, err) = sx(&[
        "resume",
        &snap,
        "--model",
        &m,
        "--hpa-set",
        "nope.maxReplicas=3",
    ]);
    assert!(!good && err.contains("no autoscaler named"), "{err}");
    let (good, err) = sx(&[
        "run",
        "--model",
        &m,
        "--no-hpa",
        "--hpa-set",
        "app.maxReplicas=3",
    ]);
    assert!(!good && err.contains("options of `resume`"), "{err}");
    let (good, err) = sx(&["run", "--model", &m, "--flavour", "slingshot-exact"]);
    assert!(!good && err.contains("Slingshot-exact"), "{err}");
    let (good, err) = sx(&["run", "--model", &m, "--no-spd"]);
    assert!(!good && err.contains("--no-hpa"), "{err}");
    // --no-hpa: the SPD alone (no policy: nothing happens)
    let none = p("none.jsonl");
    let mut args = vec!["run", "--model", &m, "--no-hpa", "--spd-log", &none];
    args.extend(common);
    let err = ok(&args);
    assert!(!err.contains("hpa app"), "{err}");
    assert!(records(Path::new(&none), "hpa").is_empty());
}

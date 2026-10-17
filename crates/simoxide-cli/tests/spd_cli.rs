//! `simoxide run` and `simoxide resume` with scaling policies whose adjustments run
//! (`DESIGN-SPD.md` P2: Slingshot's `MainTransformation`; features `spd` and `reconfiguration`)
//! in new processes: the SPD minimal example in the SimuLizar flavour scales out at 50 s, the
//! decision log records the adjustment with Slingshot's model summaries; a run paused after the
//! adjustment and resumed from its snapshot gives the uninterrupted run's log and measurements;
//! `--enforce` adjusts at the resume.

#![cfg(all(feature = "spd", feature = "reconfiguration"))]

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("spd-cli-{name}"));
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

/// The SPD minimal example for the SimuLizar flavour: its utilisation monitor gets a
/// `TimeDriven` processing type (SimuLizar needs one), everything else as in Slingshot's.
fn model(d: &Path) -> String {
    const UTIL: &str = "_QIb6cikUEeSuf8LV7cHLgA";
    let src = root().join("reference/slingshot/models/spd_minimal");
    let out = d.join("spd_minimal");
    std::fs::create_dir_all(&out).unwrap();
    for e in std::fs::read_dir(&src).unwrap() {
        let p = e.unwrap().path();
        let mut text = std::fs::read_to_string(&p).unwrap();
        if p.extension().is_some_and(|x| x == "monitorrepository") {
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
        std::fs::write(out.join(p.file_name().unwrap()), text).unwrap();
    }
    out.to_string_lossy().into_owned()
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

fn adjusts(log: &str) -> Vec<serde_json::Value> {
    log.lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .filter(|v| v["k"] == "adjust")
        .collect()
}

#[test]
fn adjustments_run_and_resume() {
    let d = tmp("run");
    let m = model(&d);
    let p = |n: &str| d.join(n).to_string_lossy().into_owned();
    let err = ok(&[
        "run",
        "--model",
        &m,
        "--max-sim-time",
        "100",
        "--spd-log",
        &p("whole.jsonl"),
        "--measurements",
        &p("whole.csv"),
    ]);
    assert!(err.contains("adjusted=1"), "{err}");
    let a = adjusts(&read(&d.join("whole.jsonl")));
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["ok"], true);
    assert_eq!(a[0]["changes"], 4);
    assert_eq!(
        a[0]["after"]["containers"][1],
        serde_json::json!(["AppServer_1", "~ResourceContainer:AppServer_1"])
    );
    // a CPU of the new container serves load: its utilisation would need F4's cloned monitor,
    // the response times change after 50 s (two replicas behind the load balancer)

    // paused after the adjustment, resumed from the snapshot
    ok(&[
        "run",
        "--model",
        &m,
        "--max-sim-time",
        "100",
        "--pause-at",
        "60",
        "--snapshot",
        &p("60.snap"),
        "--spd-log",
        &p("a.jsonl"),
    ]);
    ok(&[
        "resume",
        &p("60.snap"),
        "--model",
        &m,
        "--spd-log",
        &p("b.jsonl"),
        "--measurements",
        &p("resumed.csv"),
    ]);
    assert_eq!(
        read(&d.join("a.jsonl")) + &read(&d.join("b.jsonl")),
        read(&d.join("whole.jsonl"))
    );
    assert_eq!(read(&d.join("resumed.csv")), read(&d.join("whole.csv")));

    // enforced at a resume before the policy's own time: two scale-outs (Slingshot's
    // transformation names both new containers AppServer_1: the name counter restarts)
    ok(&[
        "run",
        "--model",
        &m,
        "--max-sim-time",
        "100",
        "--pause-at",
        "30",
        "--snapshot",
        &p("30.snap"),
    ]);
    let err = ok(&[
        "resume",
        &p("30.snap"),
        "--model",
        &m,
        "--enforce",
        "Basic Point in Time Trigger",
        "--spd-log",
        &p("e.jsonl"),
    ]);
    assert!(err.contains("adjusted=2"), "{err}");
    let a = adjusts(&read(&d.join("e.jsonl")));
    assert_eq!(a.len(), 2);
    assert_eq!(a[0]["enforced"], true);
    assert!(a[0]["t"].as_f64().unwrap() <= 30.0, "{}", a[0]["t"]);
    assert_eq!(a[1]["t"], 50.0);
    assert_eq!(
        a[1]["after"]["target_groups"][0]["elements"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
}

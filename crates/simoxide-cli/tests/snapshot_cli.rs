//! `simoxide run --pause-at T --snapshot FILE` and `simoxide resume FILE` (`DESIGN-SPD.md` N1):
//! the snapshot is continued by another process, and the outputs of the paused run plus those of
//! the resumed one equal the uninterrupted run's (trace, tape, measurements, decision log).

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("simoxide-snap-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn simoxide(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_simoxide"))
        .args(args)
        .output()
        .expect("simoxide runs");
    let err = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(out.status.success(), "simoxide {args:?}: {err}");
    err
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap_or_default()
}

fn mode() -> &'static str {
    if cfg!(feature = "exact") {
        "exact"
    } else {
        "fast"
    }
}

/// Runs `model` (extra flags `flags`) uninterrupted, then paused at each time of `at` with a
/// snapshot each time, resumed by a new process, and compares the outputs.
fn check(name: &str, model: &Path, flags: &[&str], at: &[&str], logs: &[&str]) {
    let d = tmp(name);
    let p = |f: &str| d.join(f).to_string_lossy().into_owned();
    let model = model.to_string_lossy().into_owned();
    let tape = mode() == "exact";
    let outputs = |tag: &str| {
        let mut v = vec![
            "--trace".to_string(),
            p(&format!("trace-{tag}")),
            "--measurements".into(),
            p(&format!("m-{tag}.csv")),
        ];
        if tape {
            v.extend(["--tape".to_string(), p(&format!("tape-{tag}"))]);
        }
        for l in logs {
            v.extend([l.to_string(), p(&format!("log-{tag}"))]);
        }
        v
    };
    let mut whole = vec!["run", "--model", &model, "--mode", mode()];
    whole.extend(flags);
    let w = outputs("whole");
    let mut args: Vec<&str> = whole.clone();
    args.extend(w.iter().map(String::as_str));
    simoxide(&args);
    // paused, then resumed hop by hop
    let mut first = vec!["run", "--model", &model, "--mode", mode()];
    first.extend(flags);
    let mut prev: Option<String> = None;
    for (i, t) in at.iter().enumerate() {
        let o = outputs(&i.to_string());
        let snap = p(&format!("snap-{i}"));
        let mut args: Vec<&str> = match &prev {
            None => first.clone(),
            Some(s) => vec!["resume", s, "--model", &model],
        };
        args.extend(o.iter().map(String::as_str));
        args.extend(["--pause-at", t, "--snapshot", &snap]);
        if i == 1 {
            args.extend(["--snapshot-format", "json"]);
        }
        let err = simoxide(&args);
        assert!(err.contains("paused"), "{name}: {err}");
        prev = Some(snap);
    }
    let last = at.len().to_string();
    let o = outputs(&last);
    let mut args: Vec<&str> = vec!["resume", prev.as_deref().unwrap(), "--model", &model];
    args.extend(o.iter().map(String::as_str));
    simoxide(&args);
    let info = Command::new(env!("CARGO_BIN_EXE_simoxide"))
        .args(["snapshot-info", prev.as_deref().unwrap()])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&info.stdout).contains("\"format\": 1"));
    let joined = |kind: &str| -> String {
        (0..=at.len())
            .map(|i| read(&d.join(format!("{kind}-{i}"))))
            .collect()
    };
    assert!(
        joined("trace") == read(&d.join("trace-whole")),
        "{name}: trace"
    );
    if tape {
        assert!(
            joined("tape") == read(&d.join("tape-whole")),
            "{name}: tape"
        );
    }
    if !logs.is_empty() {
        assert!(joined("log") == read(&d.join("log-whole")), "{name}: log");
    }
    assert_eq!(
        read(&d.join(format!("m-{last}.csv"))),
        read(&d.join("m-whole.csv")),
        "{name}: measurements"
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[test]
fn a_resumed_process_continues_the_run() {
    check(
        "mediastore",
        &root().join("corpus/x_sl_mediastore"),
        &[],
        &["3.5", "40", "41"],
        &[],
    );
    check(
        "fork",
        &root().join("corpus/h11_fork_sync"),
        &["--max-measurements", "600"],
        &["1", "20"],
        &[],
    );
}

#[test]
fn the_slingshot_flavour_resumes_too() {
    check(
        "slingshot",
        &root().join("corpus/h13_passive_contention"),
        &["--flavour", "slingshot", "--quirks", "Q26,Q29"],
        &["2", "9.25"],
        &[],
    );
}

#[cfg(feature = "spd")]
#[test]
fn an_spd_run_resumes_with_its_decision_log() {
    // the SPD example without its monitors (they need the Slingshot flavour's monitor kinds)
    let src = root().join("reference/slingshot/models/spd_time_policies");
    let d = tmp("spd-model");
    for e in std::fs::read_dir(&src).unwrap().flatten() {
        let ext = e
            .path()
            .extension()
            .map(|x| x.to_string_lossy().into_owned());
        if e.path().is_file()
            && !matches!(
                ext.as_deref(),
                Some("monitorrepository" | "measuringpoint" | "json")
            )
        {
            std::fs::copy(e.path(), d.join(e.file_name())).unwrap();
        }
    }
    check(
        "spd",
        &d,
        &["--max-sim-time", "100", "--spd-quirks", "slingshot"],
        &["20", "50.5"],
        &["--spd-log"],
    );
    std::fs::remove_dir_all(&d).unwrap();
}

#[cfg(feature = "reconfiguration")]
#[test]
fn a_run_with_rules_resumes_with_its_rules() {
    check(
        "rules",
        &root().join("corpus-rules/lb_scaleout"),
        &[],
        &["95", "400"],
        &["--rules-log"],
    );
}

#[test]
fn resume_refuses_another_model() {
    let d = tmp("other");
    let snap = d.join("s").to_string_lossy().into_owned();
    let a = root().join("corpus/h01_ps_single");
    let b = root().join("corpus/h02_ps_ties");
    simoxide(&[
        "run",
        "--model",
        &a.to_string_lossy(),
        "--mode",
        mode(),
        "--pause-at",
        "1",
        "--snapshot",
        &snap,
    ]);
    let out = Command::new(env!("CARGO_BIN_EXE_simoxide"))
        .args(["resume", &snap, "--model", &b.to_string_lossy()])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("another model"), "{err}");
    std::fs::remove_dir_all(&d).unwrap();
}

/// The Slingshot-exact engine (`DESIGN-SPD.md` 5.7, "ssx snapshots as built"): paused,
/// written and resumed by new processes, a JSON hop and a pause within one instant included.
#[test]
fn the_slingshot_exact_engine_resumes_too() {
    check(
        "ssx",
        &root().join("corpus/x_ss_mediastore"),
        &[
            "--flavour",
            "slingshot-exact",
            "--seed",
            "1",
            "--max-sim-time",
            "70470",
            "--max-measurements",
            "-1",
        ],
        &["100", "20000", "20000.5"],
        &[],
    );
}

/// The Slingshot-exact engine with scaling policies: the decision log continues (with
/// `reconfiguration` the adjustments run, their QVTo state in the section `qvto`).
#[cfg(feature = "spd")]
#[test]
fn a_slingshot_exact_spd_run_resumes_with_its_decision_log() {
    check(
        "ssx-spd",
        &root().join("reference/slingshot/models/spd_scale_in"),
        &[
            "--flavour",
            "slingshot-exact",
            "--seed",
            "1",
            "--max-sim-time",
            "150",
        ],
        &["40", "80", "80.5"],
        &["--spd-log"],
    );
}

/// A model directory with a load profile (`*.loadprofile.json`, docs/guide/load-profile.md):
/// the profile is read with the model, its state is part of the snapshot, and the resumed run
/// continues it; `--no-load-profile` and `--load-profile FILE` choose another one.
#[test]
fn a_load_profile_is_read_and_resumed() {
    let d = tmp("lp-model");
    let src = root().join("corpus/h05_open_workload");
    for e in std::fs::read_dir(&src).unwrap().flatten() {
        if e.path().is_file() {
            // a server fast enough for every load of the profile
            let t = std::fs::read_to_string(e.path()).unwrap().replace(
                "processingRate_ProcessingResourceSpecification specification=\"1.0\"",
                "processingRate_ProcessingResourceSpecification specification=\"100.0\"",
            );
            std::fs::write(d.join(e.file_name()), t).unwrap();
        }
    }
    std::fs::write(
        d.join("steps.loadprofile.json"),
        r#"{"points": [[0, 0.5], [30, 2.0], [60, 0.25], [90, 1.5]]}"#,
    )
    .unwrap();
    check(
        "load-profile",
        &d,
        &["--max-sim-time", "120", "--max-measurements", "-1"],
        &["40", "75"],
        &[],
    );
    let rows = |extra: &[&str]| -> usize {
        let m = d.join("lp.csv");
        let model = d.to_string_lossy().into_owned();
        let ms = m.to_string_lossy().into_owned();
        let mut args = vec![
            "run",
            "--model",
            &model,
            "--max-sim-time",
            "120",
            "--max-measurements",
            "-1",
            "--measurements",
            &ms,
        ];
        args.extend(extra);
        simoxide(&args);
        read(&m)
            .lines()
            .filter(|l| l.starts_with("UsageScenario"))
            .count()
    };
    let with = rows(&[]);
    let without = rows(&["--no-load-profile"]);
    let other = d.join("double.json");
    std::fs::write(&other, r#"{"points": [[0, 2.0]]}"#).unwrap();
    let double = rows(&["--load-profile", &other.to_string_lossy()]);
    // 4 arrivals/s for 120 s: about 480; the profile's mean factor is 1.0625; doubled: 960
    let all = format!("with {with}, without {without}, doubled {double}");
    assert!((400..560).contains(&without), "{all}");
    assert!((420..600).contains(&with) && with != without, "{all}");
    assert!((850..1070).contains(&double), "{all}");
    std::fs::remove_dir_all(&d).unwrap();
}

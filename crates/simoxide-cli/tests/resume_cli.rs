//! `simoxide run/resume --pause-when`, `simoxide resume --change/--transform/--rules/--enforce`
//! and `simoxide explore` (`DESIGN-SPD.md` N2) in new processes.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("simoxide-n2-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Runs simoxide; (success, stdout, stderr).
fn sx(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_simoxide"))
        .args(args)
        .output()
        .expect("simoxide runs");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn ok(args: &[&str]) -> String {
    let (s, _, err) = sx(args);
    assert!(s, "simoxide {args:?}: {err}");
    err
}

fn fails(args: &[&str]) -> String {
    let (s, _, err) = sx(args);
    assert!(!s, "simoxide {args:?} succeeded: {err}");
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

const MS: &str = "corpus/x_pem_screencast_ms";

/// A MediaStore run paused at 2000 s with a snapshot; the snapshot's path.
fn mediastore_snapshot(d: &Path) -> String {
    let model = root().join(MS).to_string_lossy().into_owned();
    let snap = d.join("ms.snap").to_string_lossy().into_owned();
    ok(&[
        "run",
        "--model",
        &model,
        "--mode",
        mode(),
        "--max-measurements",
        "-1",
        "--max-sim-time",
        "8000",
        "--pause-at",
        "2000",
        "--snapshot",
        &snap,
    ]);
    snap
}

/// `--pause-when` pauses a run; the paused run plus its resumed continuation equal the
/// uninterrupted run (trace, measurements).
#[test]
fn a_pause_condition_pauses_and_the_run_resumes() {
    let d = tmp("when");
    let p = |f: &str| d.join(f).to_string_lossy().into_owned();
    let model = root()
        .join("corpus/h06_closed_think")
        .to_string_lossy()
        .into_owned();
    let base = [
        "--model",
        &model,
        "--mode",
        mode(),
        "--max-measurements",
        "300",
    ];
    let mut a: Vec<&str> = vec!["run"];
    a.extend(base);
    let (t, m) = (p("t-whole"), p("m-whole.csv"));
    a.extend(["--trace", &t, "--measurements", &m]);
    ok(&a);
    let mut a: Vec<&str> = vec!["run"];
    a.extend(base);
    let (t1, s1) = (p("t1"), p("s1"));
    a.extend([
        "--trace",
        &t1,
        "--pause-when",
        "state[ServerNode] >= 2 for 1",
        "--pause-when",
        "time 1000",
        "--snapshot",
        &s1,
    ]);
    let err = ok(&a);
    assert!(
        err.contains("pause condition: State of Active Resource Tuple"),
        "{err}"
    );
    assert!(err.contains("paused t="), "{err}");
    // resumed: on to a relative pause, then to the end
    let (t2, s2, t3, m3) = (p("t2"), p("s2"), p("t3"), p("m3.csv"));
    let err = ok(&[
        "resume",
        &s1,
        "--model",
        &model,
        "--trace",
        &t2,
        "--pause-when",
        "elapsed 2.5",
        "--snapshot",
        &s2,
    ]);
    assert!(err.contains("pause condition: 2.5 s elapsed"), "{err}");
    ok(&[
        "resume",
        &s2,
        "--model",
        &model,
        "--trace",
        &t3,
        "--measurements",
        &m3,
    ]);
    assert!(read(Path::new(&m3)) == read(Path::new(&m)));
    let joined = read(Path::new(&t1)) + &read(Path::new(&t2)) + &read(Path::new(&t3));
    assert!(joined == read(Path::new(&t)), "traces differ");
    // a bad condition is a usage error
    let err = fails(&["run", "--model", &model, "--pause-when", "rt[x] = 1"]);
    assert!(err.contains("pause condition"), "{err}");
    std::fs::remove_dir_all(&d).unwrap();
}

#[cfg(not(feature = "reconfiguration"))]
#[test]
fn changes_need_the_reconfiguration_feature() {
    let d = tmp("nofeature");
    let snap = mediastore_snapshot(&d);
    let model = root().join(MS).to_string_lossy().into_owned();
    let f = d.join("c.changes");
    std::fs::write(&f, "arrival-rate RealisticUsageScenario 0.08\n").unwrap();
    let err = fails(&[
        "resume",
        &snap,
        "--model",
        &model,
        "--change",
        &f.to_string_lossy(),
    ]);
    assert!(err.contains("reconfiguration"), "{err}");
    std::fs::remove_dir_all(&d).unwrap();
}

/// `resume --change`: deterministic (two processes write the same outputs), and the change
/// takes effect; refused changes give clear errors; `--enforce` needs scaling policies.
#[cfg(feature = "reconfiguration")]
#[test]
fn resume_with_a_change_file() {
    let d = tmp("change");
    let p = |f: &str| d.join(f).to_string_lossy().into_owned();
    let snap = mediastore_snapshot(&d);
    let model = root().join(MS).to_string_lossy().into_owned();
    std::fs::write(
        d.join("c.changes"),
        "# faster disk, twice the load\nrate DatabaseServer:HDD 1200 * 1000 * 1000\n\
         arrival-rate RealisticUsageScenario 0.08\n",
    )
    .unwrap();
    let c = p("c.changes");
    let run = |tag: &str, extra: &[&str]| -> (String, String) {
        let m = p(&format!("m-{tag}.csv"));
        let mut a = vec![
            "resume",
            snap.as_str(),
            "--model",
            &model,
            "--measurements",
            &m,
        ];
        a.extend(extra);
        let err = ok(&a);
        (read(Path::new(&m)), err)
    };
    let (x, err) = run("a", &["--change", &c]);
    assert!(err.contains("change file: generation 1"), "{err}");
    let (y, _) = run("b", &["--change", &c]);
    let (z, _) = run("plain", &[]);
    assert!(x == y, "not deterministic");
    assert!(x != z, "no effect");
    // refused: F4 (population of an open workload is a wrong target; a removed allocation
    // context aborts in SimuLizar), a model error, the Slingshot flavour
    for (text, want) in [
        (
            "population RealisticUsageScenario 3\n",
            "not a closed workload",
        ),
        // F4: deleting the allocation context leaves the assembly unallocated, which the
        // model validation refuses (SimuLizar's abort is `removal keep`)
        ("delete _sj2LoLYiEeaM_9DBke4xEw\n", "is not allocated"),
        ("rate NoSuchServer 2\n", "line 1: no element"),
    ] {
        std::fs::write(d.join("bad.changes"), text).unwrap();
        let err = fails(&[
            "resume",
            &snap,
            "--model",
            &model,
            "--change",
            &p("bad.changes"),
        ]);
        assert!(err.contains(want), "{text}: {err}");
    }
    let err = fails(&["resume", &snap, "--model", &model, "--enforce", "scaleOut"]);
    assert!(err.contains("no scaling policies"), "{err}");
    // a closed workload's population (F4: applied)
    let h06 = root()
        .join("corpus/h06_closed_think")
        .to_string_lossy()
        .into_owned();
    let s6 = p("h06.snap");
    ok(&[
        "run",
        "--model",
        &h06,
        "--mode",
        mode(),
        "--pause-at",
        "5",
        "--snapshot",
        &s6,
    ]);
    std::fs::write(d.join("pop.changes"), "population Scenario 8\n").unwrap();
    let err = ok(&[
        "resume",
        &s6,
        "--model",
        &h06,
        "--change",
        &p("pop.changes"),
    ]);
    assert!(
        err.contains("closed population") && err.contains("4 -> 8"),
        "{err}"
    );
    // the Slingshot flavour takes model changes (F4)
    let ss = root()
        .join("corpus/x_ss_minimal")
        .to_string_lossy()
        .into_owned();
    let s7 = p("ss.snap");
    ok(&[
        "run",
        "--model",
        &ss,
        "--flavour",
        "slingshot",
        "--pause-at",
        "5",
        "--snapshot",
        &s7,
    ]);
    std::fs::write(d.join("ss.changes"), "rate ResourceContainer1 2\n").unwrap();
    let err = ok(&["resume", &s7, "--model", &ss, "--change", &p("ss.changes")]);
    assert!(err.contains("generation 1"), "{err}");
    std::fs::remove_dir_all(&d).unwrap();
}

/// `resume --transform` twice through a snapshot: the QVTo state (section `qvto`) carries the
/// id sequence, so the hops equal one process that applies both transformations... checked as
/// determinism of the two-hop chain and the log of the second transformation.
#[cfg(feature = "reconfiguration")]
#[test]
fn resume_with_transformations() {
    let d = tmp("transform");
    let p = |f: &str| d.join(f).to_string_lossy().into_owned();
    let model = root()
        .join("corpus/h19_linking_resource")
        .to_string_lossy()
        .into_owned();
    let add = root()
        .join("crates/simoxide-reconf/tests/rules/add_container.qvto")
        .to_string_lossy()
        .into_owned();
    let s0 = p("s0");
    ok(&[
        "run",
        "--model",
        &model,
        "--mode",
        mode(),
        "--max-measurements",
        "-1",
        "--max-sim-time",
        "200",
        "--pause-at",
        "100",
        "--snapshot",
        &s0,
    ]);
    let chain = |tag: &str| -> String {
        let s1 = p(&format!("s1-{tag}"));
        let e1 = ok(&[
            "resume",
            &s0,
            "--model",
            &model,
            "--transform",
            &add,
            "--pause-at",
            "150",
            "--snapshot",
            &s1,
        ]);
        let m = p(&format!("m-{tag}.csv"));
        let e2 = ok(&[
            "resume",
            &s1,
            "--model",
            &model,
            "--transform",
            &add,
            "--measurements",
            &m,
        ]);
        assert!(e1.contains("change addContainer: generation 1"), "{e1}");
        assert!(e2.contains("change addContainer: generation 2"), "{e2}");
        read(Path::new(&m))
    };
    assert!(chain("a") == chain("b"));
    std::fs::remove_dir_all(&d).unwrap();
}

/// `resume --rules DIR` with other rules than the snapshot's replaces them; on a snapshot
/// without rules it adds them.
#[cfg(feature = "reconfiguration")]
#[test]
fn resume_with_other_rules() {
    let d = tmp("rules");
    let p = |f: &str| d.join(f).to_string_lossy().into_owned();
    let lb = root().join("corpus-rules/lb_scaleout");
    let model = lb.to_string_lossy().into_owned();
    let only = d.join("only");
    std::fs::create_dir_all(&only).unwrap();
    std::fs::copy(lb.join("rules/outsource.qvto"), only.join("outsource.qvto")).unwrap();
    let only = only.to_string_lossy().into_owned();
    let s = p("s");
    ok(&[
        "run",
        "--model",
        &model,
        "--mode",
        mode(),
        "--rules",
        &only,
        "--pause-at",
        "300",
        "--snapshot",
        &s,
    ]);
    let err = ok(&[
        "resume",
        &s,
        "--model",
        &model,
        "--rules",
        &lb.join("rules").to_string_lossy(),
    ]);
    assert!(err.contains("rules replaced"), "{err}");
    let s2 = p("s2");
    ok(&[
        "run",
        "--model",
        &model,
        "--mode",
        mode(),
        "--rules",
        &only,
        "--pause-at",
        "300",
        "--snapshot",
        &s2,
    ]);
    let h06 = root()
        .join("corpus/h06_closed_think")
        .to_string_lossy()
        .into_owned();
    let s3 = p("s3");
    ok(&[
        "run",
        "--model",
        &h06,
        "--mode",
        mode(),
        "--pause-at",
        "5",
        "--snapshot",
        &s3,
    ]);
    let err = ok(&["resume", &s3, "--model", &h06, "--rules", &only]);
    assert!(err.contains("rules added"), "{err}");
    std::fs::remove_dir_all(&d).unwrap();
}

/// `simoxide explore`: the comparison table is the same on 1 and 3 threads.
#[cfg(feature = "reconfiguration")]
#[test]
fn explore_is_deterministic() {
    let d = tmp("explore");
    let p = |f: &str| d.join(f).to_string_lossy().into_owned();
    let snap = mediastore_snapshot(&d);
    let model = root().join(MS).to_string_lossy().into_owned();
    std::fs::write(
        d.join("db.changes"),
        "rate DatabaseServer:HDD 1200 * 1000 * 1000\n",
    )
    .unwrap();
    std::fs::write(
        d.join("load.changes"),
        "arrival-rate RealisticUsageScenario 0.08\n",
    )
    .unwrap();
    let v1 = format!("fast-db={}", p("db.changes"));
    let v2 = format!("more-load={}", p("load.changes"));
    let table = |threads: &str| {
        let (s, out, err) = sx(&[
            "explore",
            &snap,
            "--model",
            &model,
            "--variant",
            "base",
            "--variant",
            &v1,
            "--variant",
            &v2,
            "--threads",
            threads,
            "--metrics",
            "rt",
        ]);
        assert!(s, "{err}");
        out
    };
    let a = table("1");
    assert!(a.contains("fast-db") && a.contains("more-load"), "{a}");
    assert!(a.contains("rt UsageScenarioMeasuringPoint"), "{a}");
    assert_eq!(a, table("3"));
    std::fs::remove_dir_all(&d).unwrap();
}

/// The Slingshot-exact engine (`DESIGN-SPD.md` 5.7, "ssx snapshots as built"): a pause
/// condition across three processes equals the uninterrupted run (trace, tape, measurements).
#[test]
fn the_slingshot_exact_engine_pauses_on_conditions() {
    let d = tmp("ssx-when");
    let p = |f: &str| d.join(f).to_string_lossy().into_owned();
    let model = root()
        .join("corpus/x_ss_mediastore")
        .to_string_lossy()
        .into_owned();
    let base = [
        "--model",
        model.as_str(),
        "--flavour",
        "slingshot-exact",
        "--seed",
        "1",
        "--max-sim-time",
        "20000",
        "--max-measurements",
        "-1",
    ];
    let outs = |tag: &str| -> Vec<String> {
        vec![
            "--trace".into(),
            p(&format!("t-{tag}")),
            "--tape".into(),
            p(&format!("p-{tag}")),
            "--measurements".into(),
            p(&format!("m-{tag}.csv")),
        ]
    };
    let with = |head: Vec<&str>, tag: &str, more: &[&str]| -> String {
        let o = outs(tag);
        let mut a = head;
        a.extend(o.iter().map(String::as_str));
        a.extend(more);
        ok(&a)
    };
    let mut run = vec!["run"];
    run.extend(base);
    with(run.clone(), "whole", &[]);
    let s1 = p("s1.snap");
    let err = with(
        run,
        "0",
        &["--pause-when", "state[*] >= 2", "--snapshot", &s1],
    );
    assert!(err.contains("pause condition: "), "{err}");
    let s2 = p("s2.snap");
    let err = with(
        vec!["resume", &s1, "--model", &model],
        "1",
        &["--pause-when", "elapsed-events 1000", "--snapshot", &s2],
    );
    assert!(err.contains("1000 events elapsed"), "{err}");
    with(vec!["resume", &s2, "--model", &model], "2", &[]);
    for k in ["t", "p"] {
        let joined: String = (0..3).map(|i| read(&d.join(format!("{k}-{i}")))).collect();
        assert!(joined == read(&d.join(format!("{k}-whole"))), "{k}");
    }
    assert_eq!(read(&d.join("m-2.csv")), read(&d.join("m-whole.csv")));
    std::fs::remove_dir_all(&d).unwrap();
}

/// `resume --change` on a Slingshot-exact snapshot: through the engine's `ChangeHost`,
/// deterministic and effective; option lines are refused.
#[cfg(feature = "reconfiguration")]
#[test]
fn the_slingshot_exact_engine_resumes_with_a_change_file() {
    let d = tmp("ssx-change");
    let p = |f: &str| d.join(f).to_string_lossy().into_owned();
    let model = root()
        .join("corpus/x_ss_mediastore")
        .to_string_lossy()
        .into_owned();
    let snap = p("s.snap");
    ok(&[
        "run",
        "--model",
        &model,
        "--flavour",
        "slingshot-exact",
        "--max-sim-time",
        "20000",
        "--max-measurements",
        "-1",
        "--pause-at",
        "5000",
        "--snapshot",
        &snap,
    ]);
    std::fs::write(d.join("c.changes"), "rate Server1:CPU 2 * 1000 * 1000\n").unwrap();
    let run = |tag: &str, extra: &[&str]| -> (String, String) {
        let m = p(&format!("m-{tag}.csv"));
        let mut a = vec![
            "resume",
            snap.as_str(),
            "--model",
            &model,
            "--measurements",
            &m,
        ];
        a.extend(extra);
        let err = ok(&a);
        (read(Path::new(&m)), err)
    };
    let c = p("c.changes");
    let (x, err) = run("a", &["--change", &c]);
    assert!(err.contains("generation 1"), "{err}");
    let (y, _) = run("b", &["--change", &c]);
    let (z, _) = run("plain", &[]);
    assert!(x == y, "not deterministic");
    assert!(x != z, "no effect");
    std::fs::write(d.join("o.changes"), "removal keep\nrate Server1:CPU 3\n").unwrap();
    let err = fails(&[
        "resume",
        &snap,
        "--model",
        &model,
        "--change",
        &p("o.changes"),
    ]);
    assert!(err.contains("change options are not supported"), "{err}");
    std::fs::remove_dir_all(&d).unwrap();
}

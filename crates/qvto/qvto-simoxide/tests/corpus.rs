//! SimuLizar's reconfiguration rules and SPD's `MainTransformation` on PCM models loaded by
//! SimOxide's loader into the adapter: the outputs (canonical dump of the extents and of the
//! changed models), the log and the status equal the Java runner's and the `qvto_emf` path's.
//! Needs `target/pcm` and the reference repositories (skipped otherwise).

#[path = "common/corpus.rs"]
mod corpus;

use qvto_testkit::compare::renumber_extent_resources as renumber;

#[test]
fn corpus_on_the_adapter() {
    if !corpus::available() {
        eprintln!("skipped: no target/pcm");
        return;
    }
    let root = corpus::repo_root();
    let java = corpus::java();
    let mut report = Vec::new();
    let mut failures = Vec::new();
    for case in corpus::cases() {
        let a = corpus::run_adapter(&root, &case);
        let e = corpus::run_emf(&root, &case);
        let j = &java[&case.id];
        let unstable = j.unstable();
        let mut diffs = Vec::new();
        if !a.compile_errors.is_empty() {
            diffs.push(format!("compile errors {:?}", a.compile_errors));
        }
        if renumber(&a.dump) != renumber(&e.dump) {
            let d = qvto_emf::canon::first_difference(&renumber(&e.dump), &renumber(&a.dump));
            diffs.push(format!("dump vs qvto-emf: {d:?}"));
        }
        let jd = renumber(j.dump().unwrap_or(""));
        if renumber(&a.dump) != jd && !unstable.contains(&"dump") {
            let d = qvto_emf::canon::first_difference(&jd, &renumber(&a.dump));
            diffs.push(format!("dump vs java: {d:?}"));
        }
        if a.log != e.log || (a.log != j.log() && !unstable.contains(&"log")) {
            diffs.push(format!(
                "log: {:?} | emf {:?} | java {:?}",
                a.log,
                e.log,
                j.log()
            ));
        }
        let sev = match a.severity {
            0 => "OK",
            2 => "WARNING",
            8 => "CANCEL",
            _ => "ERROR",
        };
        if Some(sev) != j.severity() || Some(a.code) != j.code() || a.severity != e.severity {
            diffs.push(format!(
                "status {sev} {} vs java {:?} {:?}",
                a.code,
                j.severity(),
                j.code()
            ));
        }
        assert!(
            a.dump.lines().count() > 10,
            "{}: dump {:?}",
            case.id,
            a.dump
        );
        let changes = a.journal.as_ref().map_or(0, |j| j.changes.len());
        report.push(format!(
            "{:45} {} {:4} dump lines (java dump {}), unstable {:?}, journal {:4} run {:>9.3?} (qvto-emf {:>9.3?})",
            case.id,
            if diffs.is_empty() { "equal" } else { "DIFFERS" },
            a.dump.lines().count(),
            if renumber(&a.dump) == jd { "equal" } else { "differs" },
            unstable,
            changes,
            a.run_time,
            e.run_time
        ));
        if !diffs.is_empty() {
            failures.push(format!("{}: {}", case.id, diffs.join("\n  ")));
        }
    }
    eprintln!("{}", report.join("\n"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

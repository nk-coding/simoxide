//! Regression gate against Eclipse QVTo's full-compile oracle: the number of keys of
//! `tests/data/compile.cases` whose problems (unit, severity, line, offset, length, message)
//! equal `tests/fixtures/eclipse-compile-oracle.tsv` must not drop below the baseline in
//! `tests/fixtures/sema-oracle-baseline.txt`. Details: `cargo run --release -p qvto-sema
//! --example oracle_report`.

mod oracle_harness;

use oracle_harness::{Harness, RunStatus, compare};

fn baseline() -> usize {
    let p = oracle_harness::repo_root()
        .join("qvto-sema/tests/fixtures/sema-oracle-baseline.txt");
    let text = std::fs::read_to_string(&p).expect("baseline file");
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .and_then(|l| l.parse().ok())
        .expect("baseline: one integer")
}

#[test]
fn eclipse_compile_oracle() {
    let h = Harness::new().expect("harness setup");
    let mut exact = 0;
    let mut failures = Vec::new();
    let mut ir_issues: Vec<String> = Vec::new();
    let t = std::time::Instant::now();
    for case in &h.cases {
        let run = h.run_case(case);
        match &run.status {
            RunStatus::Ok => {}
            other => {
                failures.push(format!("{}: {other:?}", run.key));
                continue;
            }
        }
        let Some(o) = h.oracle_for(&run.key) else {
            failures.push(format!("{}: no oracle entry", run.key));
            continue;
        };
        for i in run.ir_issues.iter().take(3) {
            ir_issues.push(format!("{}: {i}", run.key));
        }
        let (_, missing, extra) = compare(&o.problems, &run.problems);
        if missing.is_empty() && extra.is_empty() {
            exact += 1;
        }
    }
    let base = baseline();
    eprintln!(
        "oracle: {exact}/{} keys match exactly (baseline {base}), {} not run, {:.1}s",
        h.cases.len(),
        failures.len(),
        t.elapsed().as_secs_f64()
    );
    for f in failures.iter().take(20) {
        eprintln!("  {f}");
    }
    assert!(ir_issues.is_empty(), "IR consistency (irck):\n{}", ir_issues.join("\n"));
    assert!(
        exact >= base,
        "{exact} keys match Eclipse's compile oracle exactly, below the baseline {base}"
    );
    if exact > base {
        eprintln!(
            "  above the baseline: update tests/fixtures/sema-oracle-baseline.txt to {exact}"
        );
    }
}

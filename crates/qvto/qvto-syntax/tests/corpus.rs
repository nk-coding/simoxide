//! The parser corpus: every `.qvto` of the Eclipse QVTo checkout, SimuLizar and the SPD
//! transformations (see `tools/oracle/corpus-files.sh`). Corpora that are not present are
//! skipped with a note.
//!
//! - `eclipse_agreement`: our lexer and parser report exactly the problems Eclipse's parser
//!   reports (`tests/fixtures/eclipse-parse-oracle.tsv`, made by `tools/oracle/make-fixture.sh`):
//!   same severity, offset, length, line and message.
//! - `round_trip`: for every file without errors, print → parse gives no errors and the same
//!   AST (span-free dump), and printing again gives the same text.

mod common;

use common::*;

#[test]
fn eclipse_agreement() {
    let roots = roots();
    let oracle = oracle();
    let mut checked = 0;
    let mut skipped = 0;
    let mut clean = 0;
    let mut mismatches = Vec::new();
    for (key, expected) in &oracle {
        let Some(path) = resolve(&roots, key) else {
            skipped += 1;
            continue;
        };
        let src = read(&path);
        let parsed = qvto_syntax::parse(&src);
        let mut expected = expected.clone();
        expected.sort();
        let ours = problem_lines(&src, &parsed);
        checked += 1;
        if expected.is_empty() {
            clean += 1;
        }
        if ours != expected {
            mismatches.push(format!("{key}\n  eclipse: {expected:#?}\n  ours:    {ours:#?}"));
        }
    }
    eprintln!("eclipse_agreement: {checked} files checked ({clean} without problems), {skipped} skipped, {} mismatches", mismatches.len());
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[test]
fn round_trip() {
    let roots = roots();
    let oracle = oracle();
    let mut checked = 0;
    let mut failures = Vec::new();
    for key in oracle.keys() {
        let Some(path) = resolve(&roots, key) else { continue };
        let src = read(&path);
        let first = qvto_syntax::parse(&src);
        if first.has_errors() {
            continue;
        }
        checked += 1;
        let printed = qvto_syntax::print::unit(&first.unit);
        let second = qvto_syntax::parse(&printed);
        let errors: Vec<_> = second.errors().map(|d| d.to_string()).collect();
        if !errors.is_empty() {
            failures.push(format!("{key}: printed text has errors: {errors:?}\n{printed}"));
            continue;
        }
        let (d1, d2) = (qvto_syntax::dump::unit(&first.unit), qvto_syntax::dump::unit(&second.unit));
        if d1 != d2 {
            let at = d1.bytes().zip(d2.bytes()).position(|(a, b)| a != b).unwrap_or(0);
            let ctx = |s: &str| s[at.saturating_sub(200)..(at + 200).min(s.len())].to_string();
            failures.push(format!("{key}: AST differs after round trip\n--- first\n{}\n--- second\n{}", ctx(&d1), ctx(&d2)));
            continue;
        }
        let printed2 = qvto_syntax::print::unit(&second.unit);
        if printed != printed2 {
            failures.push(format!("{key}: printing is not idempotent"));
        }
    }
    eprintln!("round_trip: {checked} files, {} failures", failures.len());
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

//! Differential test against Eclipse's parser on deterministic token-level mutants of the corpus
//! (`tests/common/mutate.rs`; Eclipse's verdicts in `tests/fixtures/eclipse-mutants-oracle.tsv`,
//! made by `tools/oracle/make-mutant-fixture.sh`).
//!
//! Required: the same accept/reject verdict (syntax errors or not) on every mutant. Reported:
//! agreement on the position of the first error, and on its span and message. LPG's error
//! repair is not reproduced in general, so those are statistics, not requirements.

mod common;

use common::*;
use std::collections::BTreeMap;

#[test]
fn mutants_agree_with_eclipse() {
    let text = std::fs::read_to_string(manifest().join("tests/fixtures/eclipse-mutants-oracle.tsv")).unwrap();
    let mut expected: BTreeMap<String, (usize, i64, i64, String)> = BTreeMap::new();
    for line in text.lines() {
        let f: Vec<&str> = line.splitn(6, '\t').collect();
        expected.insert(f[0].to_string(), (f[1].parse().unwrap(), f[3].parse().unwrap(), f[4].parse().unwrap(), f[5].to_string()));
    }
    let roots = roots();
    let (mut total, mut verdict_ok, mut rejected, mut first_pos_ok, mut first_full_ok) = (0, 0, 0, 0, 0);
    let mut verdict_mismatches = Vec::new();
    for key in oracle().keys() {
        let Some(path) = resolve(&roots, key) else { continue };
        let src = read(&path);
        for m in mutate::mutants(key, &src) {
            let Some((errors, off, len, msg)) = expected.get(&m.id) else { continue };
            total += 1;
            let parsed = qvto_syntax::parse(&m.text);
            let ours: Vec<_> = parsed.errors().collect();
            if (*errors > 0) == !ours.is_empty() {
                verdict_ok += 1;
            } else {
                verdict_mismatches.push(format!(
                    "{} ({}): eclipse {} errors (first at {off}: {msg}), ours {:?}",
                    m.id,
                    m.what,
                    errors,
                    ours.first().map(|d| d.to_string())
                ));
            }
            if *errors > 0 {
                rejected += 1;
                if let Some(first) = ours.iter().min_by_key(|d| (d.span.start, d.span.end)) {
                    let o = qvto_syntax::span::byte_to_utf16(&m.text, first.span.start as usize) as i64;
                    let l = qvto_syntax::span::byte_to_utf16(&m.text, first.span.end as usize) as i64 - o;
                    if o == *off {
                        first_pos_ok += 1;
                        if l == *len && first.message == *msg {
                            first_full_ok += 1;
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "mutants: {total} checked; verdict agreement {verdict_ok}/{total}; of {rejected} rejected by Eclipse: first error at the same offset {first_pos_ok}, same span and message {first_full_ok}"
    );
    assert!(verdict_mismatches.is_empty(), "{} verdict mismatches:\n{}", verdict_mismatches.len(), verdict_mismatches.join("\n"));
}

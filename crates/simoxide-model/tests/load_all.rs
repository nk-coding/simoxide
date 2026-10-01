//! Loads every model directory known to the oracle (Palladio repos, corpus, edge cases) with the
//! default (tolerant) options: no panics, fast, and no unresolved references wherever EMF itself
//! loads the model without errors. Corpus models (made to be simulated) must be free of errors.

mod common;

use simoxide_model::{Level, LoadOptions};

/// Diagnostic kinds produced while reading and resolving (as opposed to validation).
const LOAD_KINDS: &[&str] = &[
    "xml-syntax",
    "unknown-type",
    "unknown-feature",
    "abstract-class",
    "illegal-value",
    "unresolved-idref",
    "unresolved-ref",
    "foreign-namespace",
    "wrong-type",
];

#[test]
fn load_all_models() {
    let mut n = 0;
    let mut failures = Vec::new();
    for e in common::entries() {
        let m = e.loader(LoadOptions::default()).build();
        n += 1;
        let load_errors: Vec<_> = m
            .diagnostics
            .iter()
            .filter(|d| d.level == Level::Error && LOAD_KINDS.contains(&d.kind))
            .collect();
        if e.emf_errors() == Some(0) && !load_errors.is_empty() {
            failures.push(format!("{} (EMF: no errors): {}", e.name, load_errors[0]));
        }
        if e.name.starts_with("corpus_")
            && let Some(d) = m.diagnostics.iter().find(|d| d.level == Level::Error)
        {
            failures.push(format!("{} (corpus): {d}", e.name));
        }
    }
    assert!(n >= 7);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn bundled_models_resolve() {
    let e = common::entries()
        .into_iter()
        .find(|e| e.is_bundled())
        .unwrap();
    let m = e.loader(LoadOptions::default()).build();
    let cpu = m.resource_type_by_name("CPU").expect("CPU resource type");
    assert_eq!(&*m.resource_types[cpu].id, "_oro4gG3fEdy4YaaT-RYrLQ");
    let ids: Vec<&str> = m.scheduling_policies.iter().map(|p| &*p.id).collect();
    assert_eq!(ids, ["ProcessorSharing", "FCFS", "Delay"]);
    assert!(m.metrics.len() > 30);
    assert!(!m.diagnostics.iter().any(|d| d.kind == "unresolved-ref"));
}

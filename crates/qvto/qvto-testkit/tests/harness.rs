//! The harness's own checks: the case file covers the inventory, the Java fixture has a passing
//! result for every runnable case, the stub engine runs end to end, and (when the Java cache
//! with saved outputs exists) replaying Java's results through the Rust comparisons reproduces
//! Java's verdicts and dumps.

use qvto_testkit::case::{Case, read_cases};
use qvto_testkit::engine::StubEngine;
use qvto_testkit::fixture::read_bundle;
use qvto_testkit::inventory::read_inventory;
use qvto_testkit::java::JavaReplay;
use qvto_testkit::paths;
use qvto_testkit::report::{Verdict, run_cases};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn root() -> PathBuf {
    qvto_paths::root()
}

fn cases() -> Vec<Case> {
    read_cases(&root().join(paths::ECLIPSE_CASES)).unwrap()
}

#[test]
fn cases_cover_the_inventory() {
    let inv = read_inventory(&root().join(paths::INVENTORY)).unwrap();
    let cases = cases();
    let a: Vec<&str> = inv.iter().map(|r| r.test_id.as_str()).collect();
    let b: Vec<&str> = cases.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(
        a, b,
        "case file and inventory list the same ids in the same order"
    );
    let runnable = cases.iter().filter(|c| c.runnable()).count();
    assert_eq!(runnable, 494);
    let transform = cases
        .iter()
        .filter(|c| c.driver == "transform.TestQvtInterpreter" && c.runnable())
        .count();
    assert_eq!(transform, 309, "all TransformTests cases are runnable");
    for c in cases.iter().filter(|c| c.runnable()) {
        assert!(c.qvto.is_some(), "{}: no transformation", c.id);
        assert!(!c.checks.is_empty(), "{}: no checks", c.id);
    }
}

#[test]
fn fixture_has_a_passing_java_result_per_runnable_case() {
    let text = std::fs::read_to_string(root().join(paths::ECLIPSE_FIXTURE)).unwrap();
    let java = read_bundle(&text).unwrap();
    let runnable: BTreeSet<String> = cases()
        .into_iter()
        .filter(|c| c.runnable())
        .map(|c| c.id)
        .collect();
    let recorded: BTreeSet<String> = java.keys().cloned().collect();
    assert_eq!(runnable, recorded);
    for (id, r) in &java {
        assert_eq!(r.verdict(), "pass", "{id}: Java verdict");
    }
}

#[test]
fn stub_engine_runs_end_to_end() {
    let root = root();
    let models = qvto_emf::config::Config::read(&root.join(paths::SUITE_CONF), &root).unwrap();
    let cases = cases();
    let reports = run_cases(
        &mut StubEngine,
        &cases,
        &root,
        &models,
        &BTreeMap::new(),
        &|_| true,
    );
    assert_eq!(reports.len(), cases.len());
    let ni = reports
        .iter()
        .filter(|r| r.verdict == Verdict::NotImplemented)
        .count();
    assert_eq!(ni, 494);
}

/// Needs the cache with saved outputs (`qvto conform record` or `qvto conform java --save-xmi`);
/// skipped without it.
#[test]
fn java_replay_reproduces_java_verdicts() {
    let root = root();
    let dir = root.join(paths::JAVA_CACHE).join("eclipse");
    if !dir.join("results.tsv").is_file() {
        eprintln!(
            "skipped: no Java cache at {} (run `qvto conform record`)",
            dir.display()
        );
        return;
    }
    let models = qvto_emf::config::Config::read(&root.join(paths::SUITE_CONF), &root).unwrap();
    let java =
        read_bundle(&std::fs::read_to_string(root.join(paths::ECLIPSE_FIXTURE)).unwrap()).unwrap();
    let mut engine = JavaReplay::new(&dir).unwrap();
    let reports = run_cases(&mut engine, &cases(), &root, &models, &java, &|c| {
        c.runnable()
    });
    let bad: Vec<String> = reports
        .iter()
        .filter(|r| r.verdict != Verdict::Pass)
        .map(|r| format!("{} {}: {}", r.id, r.verdict.label(), r.detail))
        .collect();
    assert!(bad.is_empty(), "{bad:#?}");
}

/// The SimuLizar and SPD cases: all recorded and passing on the Java side; the replay reproduces
/// them strictly except where QVTo leaves created objects in its internal `extent:/N` resources
/// (cross-resource containment that a saved XMI cannot hold). Needs `target/pcm` and the cache.
#[test]
fn corpus_cases_recorded_and_replayed() {
    let root = root();
    let cases = read_cases(&root.join(paths::CORPUS_CASES)).unwrap();
    let java =
        read_bundle(&std::fs::read_to_string(root.join(paths::CORPUS_FIXTURE)).unwrap()).unwrap();
    assert_eq!(cases.len(), java.len());
    for c in &cases {
        assert_eq!(java[&c.id].verdict(), "pass", "{}", c.id);
    }
    let dir = root.join(paths::JAVA_CACHE).join("corpus");
    if !dir.join("results.tsv").is_file() || !root.join("target/pcm/pcm.conf").is_file() {
        eprintln!(
            "skipped replay: needs target/pcm (pcm-setup.sh) and `qvto conform record --suite corpus`"
        );
        return;
    }
    let models = qvto_emf::config::Config::read(&root.join(paths::SUITE_CONF), &root).unwrap();
    let mut engine = JavaReplay::new(&dir).unwrap();
    let cached = engine.results.clone();
    let reports = run_cases(&mut engine, &cases, &root, &models, &cached, &|_| true);
    let lenient = [
        "simulizar_runconfigtest_addNewServer_high",
        "simulizar_runconfigtest_addClonedServer_high",
    ];
    for r in &reports {
        let ok = r.verdict == Verdict::Pass
            || (lenient.contains(&r.id.as_str()) && r.verdict == Verdict::PassLenient);
        assert!(ok, "{} {}: {}", r.id, r.verdict.label(), r.detail);
    }
}

/// Identity-hash tapes: the seed of each case's thread recovers its xor-shift state, and the
/// hashes the run assigned are found in that thread's sequence (the rest were drawn by other
/// threads, e.g. metamodel objects hashed during an earlier case). Needs the Java cache.
#[test]
fn identity_hash_tapes_follow_the_thread_sequence() {
    let dir = root().join(paths::JAVA_CACHE).join("eclipse");
    if !dir.is_dir() {
        eprintln!("skipped: no Java cache");
        return;
    }
    let (mut cases, mut recovered, mut model_assigned, mut model_found, mut max_index) =
        (0, 0, 0, 0, 0);
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let Ok(t) = std::fs::read_to_string(e.path().join("idtape.tsv")) else {
            continue;
        };
        let tape = qvto_testkit::fixture::parse_tape(&t).unwrap();
        cases += 1;
        // model objects only: metamodel objects mostly got their hashes on earlier threads
        let run: Vec<&(String, i32, Option<bool>)> = tape
            .objects
            .iter()
            .filter(|o| o.2 == Some(true) && !o.0.starts_with("mm:"))
            .collect();
        let hashes: Vec<i32> = run.iter().map(|o| o.1).collect();
        let Some(idx) = qvto_testkit::idhash::sequence_index(tape.seed, &hashes, 1 << 20) else {
            continue;
        };
        recovered += 1;
        for i in &idx {
            model_assigned += 1;
            model_found += i.is_some() as usize;
            max_index = max_index.max(i.unwrap_or(0));
        }
    }
    eprintln!(
        "{cases} tapes, {recovered} seeds recovered; hashes of model objects assigned during the run found in the case \
         thread's sequence: {model_found}/{model_assigned}, at draw <= {max_index}"
    );
    assert!(
        recovered * 100 >= cases * 95,
        "seeds recovered for {recovered} of {cases}"
    );
    assert!(
        model_found * 100 >= model_assigned * 99,
        "model-object hashes in sequence: {model_found}/{model_assigned}"
    );
}

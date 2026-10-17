//! Every model file of the Eclipse QVTo test suite: loads like EMF, saves byte for byte like
//! EMF, and has the canonical dump EMF has (after resolving proxies).
//!
//! The expected results come from the EMF oracle (`oracle/run.sh models ...`):
//! `tests/data/suite-oracle.tsv` (committed: status, error count, hashes of the save and of the
//! dump). When the oracle's full output directory exists (`QVTO_EMF_ORACLE`, default
//! `target/emf-oracle/suite` in the repository), mismatches show the first differing line.
//! Skipped when the reference checkout (`reference/eclipse-qvto`) is missing.

use simoxide_emf::canon::{dump, first_difference, fnv64};
use simoxide_emf::config::Config;
use simoxide_emf::xmi_save::{SaveOptions, save};
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    qvto_paths::root()
}

/// Models whose metamodel (the QVTo AST) has dynamic subclasses of Ecore's generated classes
/// (`Library extends EClass`): EMF stores their features in misaligned slots, so its own save and
/// dump are not meaningful. In the real QVTo these classes are generated.
const EMF_ANOMALIES: [&str; 2] = [
    "reference/eclipse-qvto/plugins/org.eclipse.m2m.qvt.oml/model/Stdlib.xmi",
    "reference/eclipse-qvto/tests/org.eclipse.m2m.tests.qvt.oml/parserTestData/models/bug449912/expected.qvtoperational",
];

#[test]
fn suite_models() {
    let root = repo();
    if !root.join("reference/eclipse-qvto").exists() {
        eprintln!("reference/eclipse-qvto missing: skipped");
        return;
    }
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    let config = Config::read(&data.join("suite.conf"), &root).unwrap();
    let expected = std::fs::read_to_string(data.join("suite-oracle.tsv")).unwrap();
    let oracle_dir = std::env::var("QVTO_EMF_ORACLE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("target/emf-oracle/suite"));
    check_models("suite", &root, &config, &expected, &oracle_dir);
}

/// The PCM corpus (SimOxide's corpus models and Palladio's default models), when set up with
/// `tools/pcm-setup.sh target/pcm` and the oracle has run on it
/// (`oracle/run.sh models target/pcm/pcm.conf target/pcm/pcm-models.txt target/emf-oracle/pcm`).
#[test]
fn pcm_models() {
    let root = repo();
    let pcm = std::env::var("QVTO_PCM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("target/pcm"));
    let oracle_dir = std::env::var("QVTO_EMF_ORACLE_PCM")
        .map(PathBuf::from)
        .unwrap_or_else(|_| root.join("target/emf-oracle/pcm"));
    let Ok(expected) = std::fs::read_to_string(oracle_dir.join("results.tsv")) else {
        eprintln!("PCM oracle results missing: skipped");
        return;
    };
    let config = Config::read(&pcm.join("pcm.conf"), &root).unwrap();
    check_models("PCM", &root, &config, &expected, &oracle_dir);
}

fn check_models(what: &str, root: &Path, config: &Config, expected: &str, oracle_dir: &Path) {
    let (mut n, mut load_ok, mut save_ok, mut dump_ok) = (0, 0, 0, 0);
    let mut failures = Vec::new();
    for (i, line) in expected.lines().enumerate() {
        let f: Vec<&str> = line.split('\t').collect();
        let (path, status, errors, save_hash, dump_hash) = (f[0], f[1], f[2], f[3], f[4]);
        if EMF_ANOMALIES.contains(&path) {
            continue;
        }
        n += 1;
        let model = root.join(path);
        let mut rs = config.resource_set_for(&model);
        let r = rs.load_file(&model);
        let (my_status, my_errors) = match r {
            Ok(r) => {
                let e = rs.graph.resource(r).errors.len();
                (if e == 0 { "ok" } else { "loaderr" }, e)
            }
            Err(_) => ("fail", 0),
        };
        let id = format!("{i:04}");
        if my_status == status && my_errors.to_string() == errors {
            load_ok += 1;
        } else {
            failures.push(format!(
                "{path}: load {my_status}/{my_errors}, EMF {status}/{errors}"
            ));
        }
        let Ok(r) = r else { continue };
        let opts = SaveOptions::for_resource(rs.graph.resource(r));
        let text = save(&rs.graph, r, &opts).unwrap_or_else(|e| format!("error: {e}"));
        let h = if text.starts_with("error: ") {
            "error".to_string()
        } else {
            fnv64(text.as_bytes())
        };
        if h == save_hash {
            save_ok += 1;
        } else {
            let mut msg = format!("{path}: save differs");
            if let Ok(exp) = std::fs::read_to_string(oracle_dir.join(format!("{id}.xmi")))
                && let Some((l, a, b)) = first_difference(&exp, &text)
            {
                msg.push_str(&format!(" at line {l}:\n  EMF:  {a}\n  ours: {b}"));
            }
            failures.push(msg);
        }
        rs.resolve_all();
        let d = dump(&rs.graph, &[r]);
        if fnv64(d.as_bytes()) == dump_hash || dump_hash == "error" {
            dump_ok += 1;
        } else {
            let mut msg = format!("{path}: dump differs");
            if let Ok(exp) = std::fs::read_to_string(oracle_dir.join(format!("{id}.jsonl")))
                && let Some((l, a, b)) = first_difference(&exp, &d)
            {
                msg.push_str(&format!(" at line {l}:\n  EMF:  {a}\n  ours: {b}"));
            }
            failures.push(msg);
        }
    }
    eprintln!("{what}: {n} models: load {load_ok}, save {save_ok}, dump {dump_ok} as EMF");
    for f in failures.iter().take(40) {
        eprintln!("{f}");
    }
    assert!(failures.is_empty(), "{} mismatches", failures.len());
}

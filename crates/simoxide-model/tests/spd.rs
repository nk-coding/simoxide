//! SPD, semanticspd and spdmeasuringpoint models (the metamodels appended to the tables) load
//! like EMF loads them with Slingshot's generated SPD packages: the canonical dump equals the
//! golden of `reference/oracles/pcm-model/run-spd.sh`.

use simoxide_model::meta::class;
use simoxide_model::meta::prelude::*;
use simoxide_model::{Level, LoadOptions, Loader, canon, load::MODEL_EXTENSIONS};
use std::path::{Path, PathBuf};

fn oracle_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/oracles/pcm-model")
}

/// `spd-models.txt`: (name, directory).
fn entries() -> Vec<(String, PathBuf)> {
    let list = std::fs::read_to_string(oracle_dir().join("spd-models.txt")).unwrap();
    list.lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (n, d) = l.split_once('\t').expect("name<TAB>dir");
            (n.to_string(), oracle_dir().join(d))
        })
        .collect()
}

/// Loads the model files of the directory (the PCM extensions plus `.spd` and `.semanticspd`),
/// sorted by name, like the oracle.
fn load(dir: &Path) -> Loader {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| MODEL_EXTENSIONS.contains(&e) || e == "spd" || e == "semanticspd")
        })
        .collect();
    files.sort();
    let mut l = Loader::new(LoadOptions::strict());
    for f in files {
        l.load_file(f);
    }
    l
}

fn golden(name: &str) -> String {
    let p = oracle_dir()
        .join("golden-spd")
        .join(format!("{name}.jsonl"));
    let g = std::fs::read_to_string(p).expect("golden (run-spd.sh)");
    g.split_once('\n').unwrap().1.to_string()
}

#[test]
fn spd_models_match_emf() {
    let es = entries();
    assert!(es.len() >= 3);
    for (name, dir) in &es {
        let (g, _) = load(dir).finish();
        let dump = canon::dump(&g, false);
        let want = golden(name);
        for (i, (a, b)) in want.lines().zip(dump.lines()).enumerate() {
            assert_eq!(a, b, "{name}: line {}", i + 2);
        }
        assert_eq!(want.lines().count(), dump.lines().count(), "{name}");
    }
}

#[test]
fn spd_basic_uses_every_package() {
    let (_, dir) = entries()
        .into_iter()
        .find(|(n, _)| n == "spd_basic")
        .unwrap();
    let (g, diags) = load(&dir).finish();
    // like EMF: `ElasticInfrastructure.PCM_ResourceEnvironment` is derived and not changeable,
    // so setting it (Slingshot's own models do) is an error and the value is dropped
    let errors: Vec<_> = diags.0.iter().filter(|d| d.level == Level::Error).collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0]
            .message
            .contains("PCM_ResourceEnvironment is not changeable")
    );
    // every appended package is used
    let classes: Vec<_> = g.objs.iter().map(|o| o.class).collect();
    for c in [
        class::SPD,
        class::ElasticInfrastructure,
        class::CooldownConstraint,
        class::ThrashingConstraint,
        class::ComposedTrigger,
        class::CPUUtilization,
        class::ExpectedTrend,
        class::StepAdjustment,
        class::Configuration,
        class::Queue,
        class::ElasticInfrastructureMeasuringPoint,
    ] {
        assert!(classes.contains(&c), "{}", c.name());
    }
}

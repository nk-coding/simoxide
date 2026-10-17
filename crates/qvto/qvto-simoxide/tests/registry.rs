//! The registry built from SimOxide's generated tables equals the registry `qvto_emf` builds from
//! the same `.ecore` files (the SimuLizar 5.2.2 product's, `qvto-emf/tools/pcm-setup.sh
//! target/pcm`), entry by entry; and SimOxide's ids are kept.

use qvto_emf::config::Config;
use qvto_emf::materialize::describe_packages;
use qvto_model::{ClassId, FeatureId, ModelStore};
use qvto_simoxide::Session;
use qvto_simoxide::registry::sim_top_packages;
use simoxide_model::meta as sm;
use std::path::PathBuf;

pub fn repo_root() -> PathBuf {
    qvto_paths::root()
}

pub fn pcm_conf() -> Option<Config> {
    let root = repo_root();
    let conf = root.join("target/pcm/pcm.conf");
    if !conf.exists() {
        eprintln!("skipped: no {} (qvto-emf/tools/pcm-setup.sh target/pcm)", conf.display());
        return None;
    }
    Some(Config::read(&conf, &root).expect("pcm.conf"))
}

#[test]
fn ids_are_simoxides() {
    let s = Session::new();
    let m = s.meta();
    for (i, c) in sm::CLASSES.iter().enumerate() {
        assert_eq!(&*m.class(ClassId(i as u32)).name, c.name);
    }
    for (i, f) in sm::FEATURES.iter().enumerate() {
        let fi = m.feature(FeatureId(i as u32));
        assert_eq!(&*fi.name, f.name);
        assert_eq!(fi.owner.0, f.owner.0);
    }
    for (i, c) in sm::CLASSES.iter().enumerate() {
        let all: Vec<u32> = m.class(ClassId(i as u32)).all_features.iter().map(|f| f.0).collect();
        let sim: Vec<u32> = c.all_features.iter().map(|f| f.0).collect();
        // SimOxide's main tables miss generic supertypes (`IntSample extends Sample<Integer>`)
        let generic = sm::CLASS_EXT[i]
            .supers
            .iter()
            .filter(|t| matches!(t, sm::TypeRef::Class(_)))
            .count()
            != c.supers.len();
        if generic {
            assert!(all.len() > sim.len(), "{}", c.name);
        } else {
            assert_eq!(all, sim, "eAllStructuralFeatures of {}", c.name);
        }
    }
}

#[test]
fn registry_equals_the_ecore_files() {
    let Some(conf) = pcm_conf() else { return };
    let s = Session::new();
    let mut rs = conf.resource_set_for(&repo_root().join("x"));
    let mut diffs = Vec::new();
    for (p, loc) in sim_top_packages() {
        let ns = s.meta().package(p).ns_uri.clone().unwrap();
        let ours = describe_packages(s.meta(), &[p]);
        rs.load_metamodel(loc).unwrap_or_else(|e| panic!("{loc}: {e}"));
        let q = rs.graph.meta().package_by_ns_uri(&ns).expect("registered");
        let theirs = describe_packages(rs.graph.meta(), &[q]);
        for (a, b) in ours.iter().zip(&theirs) {
            if a != b {
                diffs.push(format!("ours:   {a}\necore:  {b}"));
            }
        }
        assert_eq!(ours.len(), theirs.len(), "{ns}");
    }
    assert!(diffs.is_empty(), "{} differences:\n{}", diffs.len(), diffs.join("\n"));
}

#[test]
fn metamodel_objects_and_operations() {
    let s = Session::new();
    // every class and feature has its Ecore object
    let g = &s.resource_set().graph;
    for i in 0..sm::CLASSES.len() {
        assert!(g.classifier_object(qvto_model::Classifier::Class(ClassId(i as u32))).is_some());
    }
    for i in 0..sm::FEATURES.len() {
        assert!(g.feature_object(FeatureId(i as u32)).is_some());
    }
    // SimOxide's EOperations reach the analyser through the objects
    let ops = s.operations();
    for op in sm::OPERATIONS {
        assert!(
            ops.iter().any(|o| o.class.0 == op.owner.0 && o.name == op.name),
            "{}",
            op.name
        );
    }
}

//! The QVTo engine's view of SimOxide's model graph (from the former `qvto-simoxide` adapter's
//! tests): the registry seeded from SimOxide's tables equals the registry built from the same
//! `.ecore` files (the SimuLizar 5.2.2 product's, `crates/foundation/simoxide-emf/tools/
//! pcm-setup.sh target/pcm`), with SimOxide's ids; every class and feature has its metamodel
//! object on an attached graph and SimOxide's `EOperation`s reach the analyser; a program's
//! intermediate classes live in the same graph and refer to its PCM objects.

mod common;

use common::*;
use qvto_emf::IdPolicy;
use qvto_emf::config::Config;
use qvto_emf::materialize::describe_packages;
use qvto_model::{ClassId, Classifier, FeatureId, ModelStore, ResourceId};
use simoxide_model::meta as sm;

fn pcm_conf() -> Option<Config> {
    let root = qroot();
    let conf = root.join("target/pcm/pcm.conf");
    if !conf.exists() {
        eprintln!("skipped: no {}", conf.display());
        return None;
    }
    Some(Config::read(&conf, &root).expect("pcm.conf"))
}

#[test]
fn ids_are_simoxides() {
    let m = core().meta();
    for (i, c) in sm::CLASSES.iter().enumerate() {
        assert_eq!(&*m.class(ClassId(i as u32)).name, c.name);
        let all: Vec<u32> = m
            .class(ClassId(i as u32))
            .all_features
            .iter()
            .map(|f| f.0)
            .collect();
        let sim: Vec<u32> = c.all_features.iter().map(|f| f.0).collect();
        assert_eq!(all, sim, "eAllStructuralFeatures of {}", c.name);
    }
    for (i, f) in sm::FEATURES.iter().enumerate() {
        let fi = m.feature(FeatureId(i as u32));
        assert_eq!((&*fi.name, fi.owner), (f.name, f.owner));
    }
}

#[test]
fn registry_equals_the_ecore_files() {
    let Some(conf) = pcm_conf() else { return };
    let ours = simoxide_model::raw::pcm_graph();
    let tops = ours.meta().static_top_packages();
    let mut rs = conf.resource_set_for(&qroot().join("x"));
    let mut diffs = Vec::new();
    for (p, loc) in tops {
        let ns = ours.meta().package(p).ns_uri.clone().unwrap();
        let a = describe_packages(ours.meta(), &[p]);
        rs.load_metamodel(loc)
            .unwrap_or_else(|e| panic!("{loc}: {e}"));
        let q = rs.graph.meta().package_by_ns_uri(&ns).expect("registered");
        let b = describe_packages(rs.graph.meta(), &[q]);
        for (x, y) in a.iter().zip(&b) {
            if x != y {
                diffs.push(format!("ours:   {x}\necore:  {y}"));
            }
        }
        assert_eq!(a.len(), b.len(), "{ns}");
    }
    assert!(
        diffs.is_empty(),
        "{} differences:\n{}",
        diffs.len(),
        diffs.join("\n")
    );
}

#[test]
fn metamodel_objects_and_operations() {
    let mut g = simoxide_model::raw::pcm_graph();
    core().attach(&mut g, IdPolicy::None);
    for i in 0..sm::CLASSES.len() {
        assert!(
            g.classifier_object(Classifier::Class(ClassId(i as u32)))
                .is_some()
        );
    }
    for i in 0..sm::FEATURES.len() {
        assert!(g.feature_object(FeatureId(i as u32)).is_some());
    }
    let ops = core().operations();
    for op in sm::OPERATIONS {
        assert!(
            ops.iter()
                .any(|o| o.class.0 == op.owner.0 && o.name == op.name),
            "{}",
            op.name
        );
    }
}

const T: &str = "
modeltype PCM_RES_ENV uses 'http://palladiosimulator.org/PalladioComponentModel/ResourceEnvironment/5.2';
transformation T(inout env : PCM_RES_ENV);

intermediate class Info {
	server : ResourceContainer;
	servers : OrderedSet(ResourceContainer);
	label : String;
}

main() {
	var e := env.rootObjects()[ResourceEnvironment]->any(true);
	var i := object Info {
		server := e.resourceContainer_ResourceEnvironment->sortedBy(entityName)->first();
		servers := e.resourceContainer_ResourceEnvironment->sortedBy(entityName);
		label := 'info';
	};
	log(i.label + ' ' + i.server.entityName + ' ' + i.servers->size().toString());
	log(i.oclIsKindOf(Info).toString() + ' ' + i.server.oclIsKindOf(ResourceContainer).toString());
}
";

#[test]
fn intermediate_class_instances_refer_to_the_graph() {
    let c = case("simulizar_runconfigtest_scaleUp_high");
    let mut g = sim_loaded(&c).graph;
    core().attach(&mut g, IdPolicy::None);
    let main = qvto::UnitSource {
        key: "mem:t".into(),
        qualified_name: "t".into(),
        file_name: "t.qvto".into(),
        path: None,
        text: std::rc::Rc::from(T),
        blackbox: None,
    };
    let resolver = qvto::units::FsResolver::folder();
    let compiled =
        qvto::compile_with_operations(main, &resolver, core().meta().clone(), core().operations());
    let program = compiled.program.expect("compiles");
    let env = (0..g.resource_count() as u32)
        .map(ResourceId)
        .find(|r| g.resource_uri(*r).ends_with(".resourceenvironment"))
        .unwrap();
    let classes = g.meta().class_count();
    let args = vec![g.roots(env).to_vec()];
    let mut vm = qvto::Vm::new(&program, &mut g, qvto::RunOptions::default());
    let out = vm.run_transformation(args);
    assert_eq!(out.severity, 0, "{} {:?}", out.message, vm.errors);
    assert!(vm.errors.is_empty(), "{:?}", vm.errors);
    let log = std::mem::take(&mut vm.log);
    drop(vm);
    assert_eq!(log, "info client 2\ntrue true\n");
    // the program's classes are the graph's now
    assert!(g.meta().class_count() > classes);
}

//! Intermediate classes on the adapter: the store takes the program's classes
//! (`ModelStore::extend_meta`), their instances live with the other metamodels' objects and
//! refer to SimOxide's graph through mirrors.

#[path = "common/corpus.rs"]
mod corpus;

use qvto_model::ModelStore;
use qvto_simoxide::SimStore;

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
    if !corpus::available() {
        return;
    }
    let root = corpus::repo_root();
    let case = corpus::cases()
        .into_iter()
        .find(|c| c.id == "simulizar_runconfigtest_scaleUp_high")
        .unwrap();
    let mut setup = corpus::adapter_setup(&root, &case);
    let main = qvto::UnitSource {
        key: "mem:t".into(),
        qualified_name: "t".into(),
        file_name: "t.qvto".into(),
        path: None,
        text: std::rc::Rc::from(T),
        blackbox: None,
    };
    let resolver = qvto::units::FsResolver::folder();
    let c = qvto::compile_with_operations(main, &resolver, setup.session.meta().clone(), &setup.session.operations());
    for u in &c.units {
        for p in &u.problems {
            eprintln!("{}", p.message);
        }
    }
    let program = c.program.expect("compiles");
    let mut store = SimStore::new(&mut setup.graph, &mut setup.session);
    let env = (0..store.resource_count() as u32)
        .map(qvto_model::ResourceId)
        .find(|r| store.resource_uri(*r).ends_with(".resourceenvironment"))
        .unwrap();
    let args = vec![store.roots(env).to_vec()];
    let mut vm = qvto::Vm::new(&program, &mut store, qvto::RunOptions::default());
    let out = vm.run_transformation(args);
    assert_eq!(out.severity, 0, "{} {:?}", out.message, vm.errors);
    assert!(vm.errors.is_empty(), "{:?}", vm.errors);
    let log = std::mem::take(&mut vm.log);
    drop(vm);
    assert_eq!(log, "info client 2\ntrue true\n");
    // the program's classes are the store's now
    assert!(store.meta().class_count() > setup_classes());
}

fn setup_classes() -> usize {
    qvto_simoxide::Session::new().meta().class_count()
}

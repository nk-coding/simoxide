//! Typed-model checks on the hand-written edge-case models in `tests/xmi-cases` (their raw
//! loading is also compared with EMF by `golden.rs`). These document EMF behaviours the
//! simulator inherits.

use simoxide_model::*;
use std::path::PathBuf;

fn case(name: &str) -> Model {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/xmi-cases")
        .join(name);
    load_dir(dir).expect("case dir")
}

fn action(m: &Model, id: &str) -> ActionId {
    m.actions
        .iter_ids()
        .find(|(_, a)| &*a.id == id)
        .map(|(i, _)| i)
        .unwrap()
}

#[test]
fn successor_links_follow_emf() {
    let m = case("seff_links");
    let (a1, a2, a3, a4) = (
        action(&m, "_a1"),
        action(&m, "_a2"),
        action(&m, "_a3"),
        action(&m, "_a4"),
    );
    // a forward `successor` IDREF is dropped; the later `predecessor` sets both ends, and the
    // last write wins: a4.predecessor = a1 re-links a1.successor from a2 to a4
    assert_eq!(m.actions[a1].successor, Some(a4));
    assert_eq!(m.actions[a2].successor, Some(a3));
    assert_eq!(m.actions[a3].successor, None);
    assert_eq!(m.actions[a3].predecessor, Some(a2));
    let seff = m.seffs.iter().find(|s| &*s.id == "_s1").unwrap();
    assert_eq!(m.action_chain(seff.behaviour), vec![a1, a4]);
    assert!(
        m.diagnostics
            .iter()
            .any(|d| d.kind == "behaviour-structure" && d.location.contains("_a2"))
    );
    // file order of steps is kept (stop before start in the second SEFF)
    let s2 = m.seffs.iter().find(|s| &*s.id == "_s2").unwrap();
    let ids: Vec<&str> = m.behaviours[s2.behaviour]
        .steps
        .iter()
        .map(|a| &*m.actions[*a].id)
        .collect();
    assert_eq!(ids, ["_b3", "_b1", "_b2"]);
    let ActionKind::Loop { iterations, body } = &m.actions[action(&m, "_b2")].kind else {
        panic!()
    };
    assert_eq!(&*iterations.spec, "IntPMF[(1;0.5)(2;0.5)]");
    assert_eq!(
        m.behaviours[body.unwrap()].owner,
        BehaviourOwner::Loop(action(&m, "_b2"))
    );
}

#[test]
fn duplicate_ids_resolve_in_tree_order() {
    let m = case("dup_ids");
    // `_dup` is an interface (first in the file) and a component (first in containment order)
    let ac1 = m
        .assembly_contexts
        .iter()
        .find(|a| &*a.id == "_ac1")
        .unwrap();
    assert_eq!(&*m.components[ac1.component.unwrap()].name, "Late");
    // resolved while parsing: only the interface existed
    let r0 = m.roles.iter().find(|r| &*r.id == "_r0").unwrap();
    assert_eq!(&*m.interfaces[r0.interface.unwrap()].name, "I");
    // resolved at the end: the component comes first in tree order -> type error, unset
    let r1 = m.roles.iter().find(|r| &*r.id == "_r1").unwrap();
    assert_eq!(r1.interface, None);
    assert!(
        m.diagnostics
            .iter()
            .any(|d| d.kind == "unresolved-ref" && d.message.contains("_nope"))
    );
}

#[test]
fn many_valued_forward_references_keep_order_and_duplicates() {
    let m = case("forward_many");
    let names = |l: &LinkingResource| {
        l.connected
            .iter()
            .map(|c| m.containers[*c].name.to_string())
            .collect::<Vec<_>>()
    };
    let l1 = m
        .linking_resources
        .iter()
        .find(|l| &*l.id == "_l1")
        .unwrap();
    assert_eq!(
        names(l1),
        ["c1", "c0", "c2", "c1", "c3", "c4", "c5", "c0", "c6"]
    );
    let l2 = m
        .linking_resources
        .iter()
        .find(|l| &*l.id == "_l2")
        .unwrap();
    assert_eq!(names(l2), ["c2", "c0", "c1"]);
    // pathmap:// and platform:/plugin/ URIs reach the same bundled resource type
    assert_eq!(l1.resource_type, l2.resource_type);
    assert_eq!(&*m.resource_types[l1.resource_type.unwrap()].name, "LAN");
    assert_eq!(&*l1.throughput.spec, "1E6");
}

#[test]
fn defaults_and_lexical_forms() {
    let m = case("defaults");
    let p0 = m
        .processing_resources
        .iter()
        .find(|p| &*p.id == "_p0")
        .unwrap();
    assert_eq!(p0.replicas, 1); // ecore default
    assert!(p0.required_by_container); // "TRUE"
    assert_eq!(p0.mttf, 1000.0); // " 1e3 " (Double.valueOf trims)
    assert_eq!(p0.mttr, 2.5); // "2.5d"
    assert_eq!(&*m.scheduling_policies[p0.scheduling.unwrap()].id, "FCFS");
    let p1 = m
        .processing_resources
        .iter()
        .find(|p| p.id.is_empty())
        .unwrap(); // EMF invents an id
    assert_eq!(p1.replicas, 4); // "+4"
    assert_eq!(&*p1.processing_rate.spec, ""); // present but unset -> validation error
    assert!(m.diagnostics.iter().any(|d| d.kind == "empty-stoex"));
    let c0 = m.containers.iter().next().unwrap();
    // literal whitespace in attributes is normalised to spaces, character references are kept
    assert_eq!(&*c0.name, "tab\tand\nnewline plus literal whitespace <&>");
    let env = m.resource_environments.iter().next().unwrap();
    assert_eq!(&*env.name, "aName"); // NamedElement.entityName default
    let s = m.usage_scenarios.iter().next().unwrap();
    let Workload::Closed {
        population,
        think_time,
    } = &s.workload
    else {
        panic!()
    };
    assert_eq!((*population, &*think_time.spec), (0, "Exp(0.5)"));
    let br = m.user_actions.iter().find(|a| &*a.id == "_br").unwrap();
    let UserActionKind::Branch { transitions } = &br.kind else {
        panic!()
    };
    assert_eq!(
        transitions
            .iter()
            .map(|t| t.probability)
            .collect::<Vec<_>>(),
        [0.25, 0.0]
    );
    let ud = &m.usage_models.iter().next().unwrap().user_data[0];
    assert_eq!(ud.usages[0].name(), "a.b");
    assert_eq!(
        ud.usages[0].characterisations[0].kind,
        CharacterisationType::Structure
    );
}

#[test]
fn multiple_roots_and_path_fragments() {
    let m = case("multi_root");
    // two roots in one file (+ the bundled PrimitiveTypes.repository it references)
    let names: Vec<&str> = m.repositories.iter().map(|r| &*r.name).collect();
    assert_eq!(names, ["first", "second", "PrimitiveDataTypes"]);
    let ac1 = m
        .assembly_contexts
        .iter()
        .find(|a| &*a.id == "_ac1")
        .unwrap();
    assert_eq!(&*m.components[ac1.component.unwrap()].name, "NoId");
    // attribute-form cross-file reference to an abstract type is an EMF error: unset
    let ac2 = m
        .assembly_contexts
        .iter()
        .find(|a| &*a.id == "_ac2")
        .unwrap();
    assert_eq!(ac2.component, None);
    let comp = m.data_types.iter().find(|d| &*d.id == "_comp").unwrap();
    let DataTypeKind::Composite { parents, inner } = &comp.kind else {
        panic!()
    };
    assert_eq!(parents.len(), 1);
    let kinds: Vec<_> = inner
        .iter()
        .map(|d| &m.data_types[d.data_type.unwrap()].kind)
        .collect();
    assert!(matches!(kinds[0], DataTypeKind::Collection { .. }));
    let coll = m.data_types.iter().find(|d| &*d.id == "_coll").unwrap();
    let DataTypeKind::Collection { inner: Some(p) } = coll.kind else {
        panic!()
    };
    assert!(matches!(
        m.data_types[p].kind,
        DataTypeKind::Primitive(PrimitiveType::Double)
    ));
}

#[test]
fn strict_and_tolerant_resolution() {
    // a broken absolute href (as found in several example models) is resolved by file name next
    // to the referencing file in tolerant mode only
    let dir = std::env::temp_dir().join(format!("simoxide-model-tolerant-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let repo = r#"<?xml version="1.0" encoding="UTF-8"?>
<repository:Repository xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:repository="http://palladiosimulator.org/PalladioComponentModel/Repository/5.2" id="_r">
  <components__Repository xsi:type="repository:BasicComponent" id="_c" entityName="C"/>
</repository:Repository>"#;
    let sys = r#"<?xml version="1.0" encoding="UTF-8"?>
<system:System xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:repository="http://palladiosimulator.org/PalladioComponentModel/Repository/5.2" xmlns:system="http://palladiosimulator.org/PalladioComponentModel/System/5.2" id="_s">
  <assemblyContexts__ComposedStructure id="_ac">
    <encapsulatedComponent__AssemblyContext xsi:type="repository:BasicComponent" href="file:/D:/somewhere/x.repository#_c"/>
  </assemblyContexts__ComposedStructure>
</system:System>"#;
    std::fs::write(dir.join("x.repository"), repo).unwrap();
    std::fs::write(dir.join("x.system"), sys).unwrap();
    let mut strict = Loader::new(LoadOptions::strict());
    strict.load_file(dir.join("x.system"));
    let m = strict.build();
    assert_eq!(m.assembly_contexts.iter().next().unwrap().component, None);
    assert!(m.diagnostics.iter().any(|d| d.kind == "unresolved-ref"));
    let m = load_files(&[dir.join("x.system")]);
    let c = m
        .assembly_contexts
        .iter()
        .next()
        .unwrap()
        .component
        .unwrap();
    assert_eq!(&*m.components[c].name, "C");
    assert!(m.diagnostics.iter().any(|d| d.kind == "tolerant-ref"));
    assert!(!m.diagnostics.iter().any(|d| d.kind == "unresolved-ref"));
    std::fs::remove_dir_all(&dir).ok();
}

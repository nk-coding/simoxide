//! Registries built in code: Ecore registered after other packages, and metamodel objects
//! materialized from registry entries.

use qvto_emf::materialize::describe_packages;
use qvto_emf::xmi_save::{SaveOptions, save};
use qvto_emf::{Graph, ResourceSet};
use qvto_model::contract::{CONTRACT_ECORE, CONTRACT_NS_URI, build_metamodel};
use qvto_model::{
    Classifier, Datum, MetaMultiplicity, MetaOperation, MetaRegistry, ModelStore,
};

/// The contract suite on a graph whose registry has the contract package first, Ecore second.
mod ecore_second {
    use super::*;
    qvto_model::contract_tests!(|| {
        let mut meta = MetaRegistry::new();
        let roles = build_metamodel(&mut meta);
        let g = Graph::with_registry(meta);
        assert_eq!(g.ecore().class_start as usize, 3);
        (g, roles)
    });
}

fn contract_first() -> (ResourceSet, qvto_model::PackageId) {
    let mut meta = MetaRegistry::new();
    build_metamodel(&mut meta);
    let p = meta.package_by_ns_uri(CONTRACT_NS_URI).unwrap();
    (ResourceSet::with_graph(Graph::with_registry(meta)), p)
}

#[test]
fn materialized_objects_register_as_the_original() {
    let (mut rs, p) = contract_first();
    let node = rs.graph.meta().class_by_name(p, "Node").unwrap();
    let string = match rs.graph.meta().classifier_by_name(p, "String") {
        Some(Classifier::DataType(d)) => Classifier::DataType(d),
        _ => panic!(),
    };
    let one = MetaMultiplicity {
        many: false,
        ordered: true,
        unique: true,
    };
    let ops = vec![MetaOperation {
        class: node,
        name: "greet".into(),
        params: vec![("who".into(), string, one)],
        result: Some((string, MetaMultiplicity { many: true, ..one })),
    }];
    let r = rs.materialize(p, "platform:/plugin/x/model/contract.ecore", &ops).unwrap();
    // eClass() objects
    let n = rs.graph.create(node).unwrap();
    let co = rs.graph.classifier_object(Classifier::Class(rs.graph.class_of(n))).unwrap();
    let e = rs.graph.ecore().clone();
    assert_eq!(rs.graph.get(co, e.named_element_name), Datum::Str("Node"));
    assert_eq!(rs.graph.object_classifier(co), Some(Classifier::Class(node)));
    // the operations read back
    let back: Vec<_> = qvto_emf::operations::meta_operations(&rs.graph)
        .into_iter()
        .filter(|o| o.class == node)
        .collect();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].name, "greet");
    assert!(back[0].result.unwrap().1.many);
    // saved and registered again from the text: the same entries
    let text = save(&rs.graph, r, &SaveOptions::for_resource(rs.graph.resource(r))).unwrap();
    let mut fresh = ResourceSet::new();
    let r2 = fresh.load_text("file:/x/contract.ecore", &text).unwrap();
    fresh.register(r2).unwrap();
    let p2 = fresh.graph.meta().package_by_ns_uri(CONTRACT_NS_URI).unwrap();
    assert_eq!(
        describe_packages(fresh.graph.meta(), &[p2]),
        describe_packages(rs.graph.meta(), &[p])
    );
    // and as `contract.ecore` registers
    let mut orig = ResourceSet::new();
    let r3 = orig.load_text("file:/c/contract.ecore", CONTRACT_ECORE).unwrap();
    orig.register(r3).unwrap();
    let p3 = orig.graph.meta().package_by_ns_uri(CONTRACT_NS_URI).unwrap();
    assert_eq!(
        describe_packages(orig.graph.meta(), &[p3]),
        describe_packages(rs.graph.meta(), &[p])
    );
}

#[test]
fn ecore_files_can_refer_to_materialized_packages() {
    let (mut rs, p) = contract_first();
    rs.materialize(p, "platform:/plugin/x/model/contract.ecore", &[]).unwrap();
    let ext = r#"<?xml version="1.0" encoding="UTF-8"?>
<ecore:EPackage xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
    xmlns:ecore="http://www.eclipse.org/emf/2002/Ecore" name="ext" nsURI="http://ext" nsPrefix="ext">
  <eClassifiers xsi:type="ecore:EClass" name="Special" eSuperTypes="../../x/model/contract.ecore#//Node">
    <eStructuralFeatures xsi:type="ecore:EReference" name="target" eType="ecore:EClass ../../x/model/contract.ecore#//Node"/>
  </eClassifiers>
</ecore:EPackage>
"#;
    let r = rs.load_text("platform:/plugin/y/model/ext.ecore", ext).unwrap();
    let new = rs.register(r).unwrap();
    assert_eq!(new.len(), 1, "only the new package is registered");
    let meta = rs.graph.meta();
    let special = meta.class_by_name(new[0], "Special").unwrap();
    let node = meta.class_by_name(p, "Node").unwrap();
    assert!(meta.is_subclass(special, node));
    let t = meta.feature_by_name(special, "target").unwrap();
    assert_eq!(meta.feature(t).ty, Classifier::Class(node));
}

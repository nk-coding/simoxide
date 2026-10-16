//! Registering metamodels from `.ecore` resources gives the same registry entries as building
//! them directly, and the built-in Ecore entries are linked to the objects of Ecore's resource.

use qvto_emf::ResourceSet;
use qvto_model::contract::{build_metamodel, CONTRACT_ECORE, CONTRACT_NS_URI};
use qvto_model::{Classifier, Datum, MetaRegistry, ModelStore};

fn describe(reg: &MetaRegistry, p: qvto_model::PackageId) -> Vec<String> {
    let mut out = Vec::new();
    for &c in &reg.package(p).classifiers {
        out.push(format!("classifier {}", reg.classifier_name(c)));
        if let Classifier::Class(c) = c {
            let ci = reg.class(c);
            let supers: Vec<&str> = ci.all_supers.iter().map(|s| &*reg.class(*s).name).collect();
            out.push(format!("  abstract={} supers={supers:?} id={:?}", ci.is_abstract, ci.id_attribute.map(|f| reg.feature(f).name.to_string())));
            for &f in ci.all_features.iter() {
                let fi = reg.feature(f);
                out.push(format!(
                    "  {} {:?} {} [{}..{}] o={} u={} ch={} v={} t={} uns={} d={} id={} cont={} rp={} opp={:?} def={:?}",
                    fi.name,
                    fi.kind,
                    reg.classifier_name(fi.ty),
                    fi.lower,
                    fi.upper,
                    fi.ordered,
                    fi.unique,
                    fi.changeable,
                    fi.volatile,
                    fi.transient,
                    fi.unsettable,
                    fi.derived,
                    fi.id,
                    fi.containment,
                    fi.resolve_proxies,
                    fi.opposite.map(|o| reg.feature(o).name.to_string()),
                    fi.default_literal
                ));
            }
        }
    }
    out
}

#[test]
fn registered_contract_metamodel_equals_built_one() {
    let mut built = MetaRegistry::new();
    build_metamodel(&mut built);
    let mut rs = ResourceSet::new();
    let r = rs.load_text("file:/x/contract.ecore", CONTRACT_ECORE).unwrap();
    rs.register(r).unwrap();
    let reg = rs.graph.meta();
    let a = describe(&built, built.package_by_ns_uri(CONTRACT_NS_URI).unwrap());
    let b = describe(reg, reg.package_by_ns_uri(CONTRACT_NS_URI).unwrap());
    assert_eq!(a, b);
}

#[test]
fn builtin_ecore_is_linked_to_its_resource() {
    let mut rs = ResourceSet::new();
    let r = rs.ecore_resource();
    let g = &rs.graph;
    let e = g.ecore().clone();
    let meta = g.meta();
    for c in meta.classes().take(e.class_end as usize) {
        let o = *g.links().classifier_obj.get(&Classifier::Class(c)).expect("linked class");
        let names: Vec<String> = (0..g.len(o, e.class_structural_features))
            .map(|i| {
                let f = g.get_at(o, e.class_structural_features, i).as_obj().unwrap();
                g.get(f, e.named_element_name).as_str().unwrap().to_string()
            })
            .collect();
        let local: Vec<String> = meta.class(c).features.iter().map(|f| meta.feature(*f).name.to_string()).collect();
        assert_eq!(names, local, "{}", meta.class(c).name);
        // derived eAllStructuralFeatures computed on the objects agrees with the registry
        let all: Vec<String> = (0..g.len(o, e.class_all_structural_features))
            .map(|i| match g.get_at(o, e.class_all_structural_features, i) {
                Datum::Obj(f) => g.get(f, e.named_element_name).as_str().unwrap().to_string(),
                d => panic!("{d:?}"),
            })
            .collect();
        let reg_all: Vec<String> = meta.class(c).all_features.iter().map(|f| meta.feature(*f).name.to_string()).collect();
        assert_eq!(all, reg_all);
    }
    assert_eq!(g.roots(r).len(), 1);
}

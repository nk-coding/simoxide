//! The extension tables (`meta::PACKAGE_EXT` etc.) agree with the main tables.

use simoxide_model::meta::{
    CLASS_EXT, CLASSES, ClassifierRef, DATA_TYPES, FEATURE_EXT, FEATURES, FeatureKind, OPERATIONS,
    PACKAGE_EXT, PACKAGES, PackageId, TypeRef,
};
use simoxide_model::raw::{Graph, ResId};

#[test]
fn extension_tables_agree_with_main_tables() {
    assert_eq!(PACKAGE_EXT.len(), PACKAGES.len());
    assert_eq!(CLASS_EXT.len(), CLASSES.len());
    assert_eq!(FEATURE_EXT.len(), FEATURES.len());
    for (c, x) in CLASSES.iter().zip(CLASS_EXT) {
        assert_eq!(c.is_abstract, x.is_abstract || x.is_interface, "{}", c.name);
        // the main tables' supertypes, in order, among the complete ones
        let classes: Vec<_> = x
            .supers
            .iter()
            .filter_map(|t| match t {
                TypeRef::Class(s) => Some(*s),
                _ => None,
            })
            .collect();
        assert!(c.supers.iter().all(|s| classes.contains(s)), "{}", c.name);
    }
    for (f, x) in FEATURES.iter().zip(FEATURE_EXT) {
        assert_eq!(f.many, x.upper != 1, "{}", f.name);
        match (f.kind, x.ty) {
            (FeatureKind::Reference { target, .. }, TypeRef::Class(t)) => assert_eq!(target, t),
            (FeatureKind::Reference { .. }, t) => panic!("{}: reference typed {t:?}", f.name),
            (FeatureKind::Attribute { .. }, TypeRef::Class(_)) => panic!("{}", f.name),
            _ => {}
        }
    }
    // every classifier is listed once, in its own package
    let mut seen_classes = vec![0; CLASSES.len()];
    for (i, p) in PACKAGE_EXT.iter().enumerate() {
        assert_eq!(p.location.is_some(), p.super_package.is_none());
        for k in p.classifiers {
            match *k {
                ClassifierRef::Class(c) => {
                    assert_eq!(c.def().package, PackageId(i as u32));
                    seen_classes[c.0 as usize] += 1;
                }
                ClassifierRef::Enum(e) => assert_eq!(e.def().package, PackageId(i as u32)),
                ClassifierRef::DataType(d) => {
                    assert_eq!(DATA_TYPES[d as usize].package, PackageId(i as u32))
                }
            }
        }
    }
    assert!(seen_classes.iter().all(|&n| n == 1));
    assert!(OPERATIONS.iter().all(|o| (o.owner.0 as usize) < CLASSES.len()));
}

#[test]
fn add_object_appends_a_detached_object() {
    let mut g = Graph::default();
    g.resources.push(Default::default());
    let c = simoxide_model::meta::class::ResourceContainer;
    let o = g.add_object(c, ResId(0));
    assert_eq!(g[o].class, c);
    assert!(g[o].container.is_none() && g[o].slots.is_empty());
    assert!(g.resources[0].roots.is_empty());
}

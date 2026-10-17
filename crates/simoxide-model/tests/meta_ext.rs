//! The static tables (`meta::PACKAGES` etc., the EMF registry's seed) are consistent: the
//! fields derived for static readers agree with the EMF facts.

use simoxide_model::meta::prelude::*;
use simoxide_model::meta::{
    CLASSES, ClassId, Classifier, DATA_TYPES, ENUMS, FEATURES, FeatureKind, OPERATIONS, PACKAGES,
    PackageId, TypeRef, class, enums, feat, package_by_ns,
};
use simoxide_model::raw::{Graph, ResId};

#[test]
fn derived_fields_agree_with_the_emf_facts() {
    for c in CLASSES {
        // `supers` are the complete supertypes without the Ecore classes, in order
        let classes: Vec<_> = c
            .super_types
            .iter()
            .filter_map(|t| match t {
                TypeRef::Class(s) => Some(*s),
                _ => None,
            })
            .collect();
        assert_eq!(c.supers, classes, "{}", c.name);
        assert!(!c.is_interface || c.is_abstract, "{}", c.name);
    }
    for f in FEATURES {
        assert_eq!(f.many, f.upper != 1, "{}", f.name);
        match (f.kind, f.ty) {
            (FeatureKind::Reference { target, .. }, TypeRef::Class(t)) => assert_eq!(target, t),
            (FeatureKind::Reference { .. }, t) => panic!("{}: reference typed {t:?}", f.name),
            (FeatureKind::Attribute { .. }, TypeRef::Class(_)) => panic!("{}", f.name),
            _ => {}
        }
    }
    // every classifier is listed once, in its own package
    let mut seen_classes = vec![0; CLASSES.len()];
    for (i, p) in PACKAGES.iter().enumerate() {
        assert_eq!(p.location.is_some(), p.super_package.is_none());
        for k in p.classifiers {
            match *k {
                Classifier::Class(c) => {
                    assert_eq!(c.def().package, PackageId(i as u32));
                    seen_classes[c.0 as usize] += 1;
                }
                Classifier::Enum(e) => assert_eq!(e.def().package, PackageId(i as u32)),
                Classifier::DataType(d) => {
                    assert_eq!(DATA_TYPES[d.index()].package, PackageId(i as u32))
                }
            }
        }
    }
    assert!(seen_classes.iter().all(|&n| n == 1));
    assert!(
        OPERATIONS
            .iter()
            .all(|o| (o.owner.0 as usize) < CLASSES.len())
    );
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

#[test]
fn generic_supertypes_are_in_the_main_tables() {
    // `IntSample extends Sample<Integer>` etc. (eGenericSuperTypes in the .ecore)
    for c in [
        class::IntSample,
        class::BoolSample,
        class::DoubleSample,
        class::StringSample,
    ] {
        assert_eq!(c.def().supers, [class::Sample], "{}", c.name());
        assert!(c.is_a(class::Sample));
        assert_eq!(c.feature("probability"), Some(feat::Sample_probability));
        assert_eq!(c.feature("value"), Some(feat::Sample_value));
        assert_eq!(
            c.features().collect::<Vec<_>>(),
            class::Sample.features().collect::<Vec<_>>()
        );
    }
    // `value: T` has no data type of its own (erasure EJavaObject)
    assert_eq!(
        feat::Sample_value.data_kind(),
        Some(simoxide_model::meta::DataKind::JavaObject)
    );
}

#[test]
fn ecore_supertypes_are_in_the_tables() {
    // PCMBaseClass: eSuperTypes="#//PCMClass ...Ecore.ecore#//EObject"; Ecore classes have no ClassId
    assert_eq!(
        class::PCMBaseClass.def().super_types,
        [TypeRef::Class(class::PCMClass), TypeRef::Ecore("EObject")]
    );
    assert_eq!(class::PCMBaseClass.def().supers, [class::PCMClass]);
}

/// Sizes of the SimuLizar 5.2.2 part of the tables. Appended metamodels come after it, so every
/// id of this part (and every constant name) stays what it was.
const BASE: (usize, usize, usize, usize, usize, usize) = (35, 299, 520, 14, 8, 18);

#[test]
fn appended_packages_keep_the_ids_of_the_base_tables() {
    let (p, c, f, e, d, o) = BASE;
    assert_eq!(
        PACKAGES[p - 1].ns_uri,
        "http://palladiosimulator.org/simulizar/measuringpoint"
    );
    assert_eq!(CLASSES[c - 1].package, PackageId(p as u32 - 1));
    assert!(CLASSES[c..].iter().all(|x| x.package.0 as usize >= p));
    assert!(FEATURES[..f].iter().all(|x| (x.owner.0 as usize) < c));
    assert!(FEATURES[f..].iter().all(|x| x.owner.0 as usize >= c));
    assert!(ENUMS[..e].iter().all(|x| (x.package.0 as usize) < p));
    assert!(ENUMS[e..].iter().all(|x| x.package.0 as usize >= p));
    assert!(DATA_TYPES[..d].iter().all(|x| (x.package.0 as usize) < p));
    assert!(OPERATIONS[..o].iter().all(|x| (x.owner.0 as usize) < c));
    // a few pinned ids
    assert_eq!(class::IntSample, ClassId(21));
    assert_eq!(class::PCMBaseClass, ClassId(55));
    assert_eq!(feat::Sample_value.0, 15);
    // the base part never references the appended one
    for x in &CLASSES[..c] {
        assert!(
            x.all_supers.iter().all(|s| (s.0 as usize) < c),
            "{}",
            x.name
        );
        assert!(
            x.all_features.iter().all(|s| (s.0 as usize) < f),
            "{}",
            x.name
        );
    }
}

#[test]
fn spd_packages_are_appended() {
    let (p, c, ..) = BASE;
    let names: Vec<&str> = PACKAGES[p..].iter().map(|x| x.name).collect();
    assert_eq!(
        names,
        [
            "spd",
            "spd.targets",
            "spd.adjustments",
            "spd.constraints",
            "spd.constraints.policy",
            "spd.constraints.target",
            "spd.triggers",
            "spd.triggers.stimuli",
            "spd.triggers.expectations",
            "semanticspd",
            "spdmeasuringpoint",
        ]
    );
    assert_eq!(
        package_by_ns("http://palladiosimulator.org/ScalingPolicyDefinition/1.0"),
        Some(PackageId(p as u32))
    );
    assert_eq!(class::SPD.0 as usize, c + 1);
    assert_eq!(
        PACKAGES[p].location,
        Some("platform:/plugin/org.palladiosimulator.spd/model/SPD.ecore")
    );
    // cross-metamodel references and supertypes resolve into the base tables
    assert!(class::SPD.is_a(class::Entity));
    assert!(class::ElasticInfrastructureMeasuringPoint.is_a(class::MeasuringPoint));
    assert_eq!(
        class::ElasticInfrastructure
            .feature("unit")
            .and_then(|f| f.target()),
        Some(class::ResourceContainer)
    );
    assert_eq!(
        class::Configuration.feature("spd").and_then(|f| f.target()),
        Some(class::SPD)
    );
    assert_eq!(
        enums::RelationalOperator.def().literals[4].name,
        "GreaterThanOrEqualTo"
    );
}

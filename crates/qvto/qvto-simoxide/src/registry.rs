//! The metamodel registry of SimOxide's generated tables (`simoxide_model::meta`): packages,
//! classes, features and enums get the same ids as in SimOxide (`ClassId(i)` here is
//! `meta::ClassId(i)` there), so model objects are read and written without translating ids.
//! Ecore follows them (`qvto_emf::Graph::with_registry`), then any metamodel loaded later.

use qvto_model::{
    ClassId, Classifier, DataTypeId, EnumId, EnumLiteral, FeatureId, FeatureKind, MetaMultiplicity,
    MetaOperation, MetaRegistry, PackageId,
};
use simoxide_model::meta as sm;

/// Sizes of the SimOxide part of a registry: ids below these are SimOxide's.
#[derive(Copy, Clone, Debug)]
pub struct SimTables {
    pub packages: u32,
    pub classes: u32,
    pub features: u32,
    pub enums: u32,
}

impl SimTables {
    pub fn get() -> SimTables {
        SimTables {
            packages: sm::PACKAGES.len() as u32,
            classes: sm::CLASSES.len() as u32,
            features: sm::FEATURES.len() as u32,
            enums: sm::ENUMS.len() as u32,
        }
    }
}

/// Feature types and supertypes that name Ecore classifiers, resolved once Ecore is registered.
pub(crate) struct Pending {
    pub ecore_types: Vec<(FeatureId, &'static str, bool)>,
    /// Class, position among its supertypes, Ecore class name.
    pub ecore_supers: Vec<(ClassId, usize, &'static str)>,
    pub data_types: Vec<DataTypeId>,
}

/// Adds SimOxide's packages to an empty registry, with SimOxide's ids. Feature types that are
/// Ecore classifiers are patched by [`finish`] after Ecore is registered.
pub(crate) fn add_sim_packages(meta: &mut MetaRegistry) -> Pending {
    assert_eq!(meta.package_count(), 0, "SimOxide's packages come first");
    let mut data_types = Vec::new();
    let mut ecore_supers = Vec::new();
    for (i, p) in sm::PACKAGES.iter().enumerate() {
        let x = &sm::PACKAGE_EXT[i];
        let short = p.name.rsplit('.').next().unwrap_or(p.name);
        let id = meta.add_package(
            short,
            Some(p.ns_uri),
            Some(p.prefix),
            x.super_package.map(|s| PackageId(s.0)),
        );
        assert_eq!(id.0, i as u32);
        // classifiers in declaration order: the tables number classes, enums and data types in
        // this walk's order, so the ids come out equal
        for k in x.classifiers {
            match *k {
                sm::ClassifierRef::Class(c) => {
                    let d = c.def();
                    let e = &sm::CLASS_EXT[c.0 as usize];
                    let cid = meta.add_class(id, d.name, e.is_abstract, e.is_interface);
                    assert_eq!(cid.0, c.0);
                    meta.class_mut(cid).instance_class_name = e.instance_class_name.map(Into::into);
                    // the complete `eSuperTypes` (generic ones included); Ecore's after Ecore
                    // is registered
                    for (k, t) in e.supers.iter().enumerate() {
                        match *t {
                            sm::TypeRef::Class(s) => meta.class_mut(cid).supers.push(ClassId(s.0)),
                            sm::TypeRef::Ecore(n) => ecore_supers.push((cid, k, n)),
                            ref t => panic!("supertype {t:?} of {}", d.name),
                        }
                    }
                }
                sm::ClassifierRef::Enum(e) => {
                    let d = e.def();
                    let lits = d
                        .literals
                        .iter()
                        .map(|l| EnumLiteral {
                            name: l.name.into(),
                            value: l.value,
                            literal: l.literal.into(),
                        })
                        .collect();
                    let eid = meta.add_enum(id, d.name, lits);
                    assert_eq!(eid.0, e.0);
                }
                sm::ClassifierRef::DataType(d) => {
                    let dd = &sm::DATA_TYPES[d as usize];
                    let did =
                        meta.add_data_type(id, dd.name, dd.instance_class_name, dd.serializable);
                    assert_eq!(data_types.len(), d as usize);
                    data_types.push(did);
                }
            }
        }
    }
    let mut ecore_types = Vec::new();
    // features in table order: per class in declaration order, so local indexes agree
    for (i, f) in sm::FEATURES.iter().enumerate() {
        let x = &sm::FEATURE_EXT[i];
        let (kind, placeholder) = match f.kind {
            sm::FeatureKind::Attribute { .. } => (FeatureKind::Attribute, None),
            sm::FeatureKind::Reference { target, .. } => (
                FeatureKind::Reference,
                Some(Classifier::Class(ClassId(target.0))),
            ),
        };
        let ty = match x.ty {
            sm::TypeRef::Class(c) => Classifier::Class(ClassId(c.0)),
            sm::TypeRef::Enum(e) => Classifier::Enum(EnumId(e.0)),
            sm::TypeRef::DataType(d) => Classifier::DataType(data_types[d as usize]),
            sm::TypeRef::Ecore(_) | sm::TypeRef::Unknown => {
                placeholder.unwrap_or(Classifier::Class(ClassId(0)))
            }
        };
        let fid = meta.add_feature(ClassId(f.owner.0), f.name, kind, ty);
        assert_eq!(fid.0, i as u32);
        match x.ty {
            sm::TypeRef::Ecore(n) => ecore_types.push((fid, n, false)),
            // a type parameter: its erasure (`EJavaObject` / `EObject`)
            sm::TypeRef::Unknown => ecore_types.push((
                fid,
                if kind == FeatureKind::Attribute {
                    "EJavaObject"
                } else {
                    "EObject"
                },
                true,
            )),
            _ => {}
        }
        let fi = meta.feature_mut(fid);
        fi.lower = f.lower;
        fi.upper = x.upper;
        fi.ordered = x.ordered;
        fi.unique = x.unique;
        fi.changeable = x.changeable;
        fi.unsettable = x.unsettable;
        fi.volatile = f.volatile;
        fi.transient = f.transient;
        fi.derived = f.derived;
        fi.id = f.is_id;
        fi.default_literal = f.default.map(Into::into);
        if let sm::FeatureKind::Reference {
            containment,
            opposite,
            resolve_proxies,
            ..
        } = f.kind
        {
            fi.containment = containment;
            fi.resolve_proxies = resolve_proxies;
            fi.opposite = opposite.map(|o| FeatureId(o.0));
        }
    }
    meta.finish_classes().expect("SimOxide metamodel");
    for i in 0..sm::PACKAGES.len() {
        meta.register_ns_uri(PackageId(i as u32));
    }
    Pending {
        ecore_types,
        ecore_supers,
        data_types,
    }
}

/// Resolves the feature types that are Ecore classifiers (Ecore is registered now) and returns
/// SimOxide's `EOperation`s for the analyser.
pub(crate) fn finish(meta: &mut MetaRegistry, pending: &Pending) -> Vec<MetaOperation> {
    let ecore = meta
        .package_by_ns_uri(qvto_emf::ecore::ECORE_NS_URI)
        .expect("Ecore registered");
    if !pending.ecore_supers.is_empty() {
        for &(c, k, name) in &pending.ecore_supers {
            let s = meta
                .class_by_name(ecore, name)
                .unwrap_or_else(|| panic!("Ecore class {name}"));
            meta.class_mut(c).supers.insert(k, s);
        }
        // the derived tables of every SimOxide class again (subclasses included)
        for i in 0..sm::CLASSES.len() {
            meta.unfinish_class(ClassId(i as u32));
        }
        meta.finish_classes().expect("SimOxide metamodel");
    }
    for &(f, name, tparam) in &pending.ecore_types {
        let k = meta
            .classifier_by_name(ecore, name)
            .unwrap_or_else(|| panic!("Ecore classifier {name}"));
        let fi = meta.feature_mut(f);
        fi.ty = k;
        fi.type_parameter = tparam;
    }
    let ty = |meta: &MetaRegistry, t: sm::TypeRef| -> Option<Classifier> {
        Some(match t {
            sm::TypeRef::Class(c) => Classifier::Class(ClassId(c.0)),
            sm::TypeRef::Enum(e) => Classifier::Enum(EnumId(e.0)),
            sm::TypeRef::DataType(d) => Classifier::DataType(pending.data_types[d as usize]),
            sm::TypeRef::Ecore(n) => meta.classifier_by_name(ecore, n)?,
            sm::TypeRef::Unknown => return None,
        })
    };
    let typed = |meta: &MetaRegistry, t: &sm::TypedDef| {
        ty(meta, t.ty).map(|c| {
            (
                c,
                MetaMultiplicity {
                    many: !(0..=1).contains(&t.upper),
                    ordered: t.ordered,
                    unique: t.unique,
                },
            )
        })
    };
    let mut ops = Vec::new();
    'ops: for op in sm::OPERATIONS {
        let mut params = Vec::new();
        for p in op.params {
            match typed(meta, &p.ty) {
                Some((c, m)) => params.push((p.name.to_string(), c, m)),
                None => continue 'ops,
            }
        }
        ops.push(MetaOperation {
            class: ClassId(op.owner.0),
            name: op.name.to_string(),
            params,
            result: op.result.as_ref().and_then(|r| typed(meta, r)),
            // generated code implements them (no OCL bodies in SimOxide's metamodels)
            body: None,
        });
    }
    ops
}

/// The top-level SimOxide packages and the locations other metamodels use for them
/// (`platform:/plugin/<bundle>/<path>`).
pub fn sim_top_packages() -> Vec<(PackageId, &'static str)> {
    sm::PACKAGE_EXT
        .iter()
        .enumerate()
        .filter_map(|(i, x)| x.location.map(|l| (PackageId(i as u32), l)))
        .collect()
}

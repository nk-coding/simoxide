//! Static metamodel tables (generated code) and their ingestion into a [`MetaRegistry`].
//!
//! A host with generated tables (SimOxide's PCM tables, `simoxide-model/tools/gen_meta.py`)
//! describes its metamodels as [`StaticPackages`]. [`MetaRegistry::from_static`] registers them
//! first, so every package, class, feature, enumeration and data type gets the id it has in the
//! tables: the host's static ids *are* registry ids. Ecore follows (it is registered by the graph
//! crate, which then calls [`MetaRegistry::resolve_static_ecore`] for the feature types and
//! supertypes that name Ecore classifiers), then metamodels loaded at run time.
//!
//! Besides the EMF facts, the tables carry a few fields derived from them for static readers
//! (SimOxide's loader reads `all_features`, `id_attribute`, the reference target and the lexical
//! kind of attribute values without a registry); [`MetaRegistry::from_static`] recomputes the
//! registry's own derived tables from the EMF facts.

use crate::ids::{ClassId, DataTypeId, EnumId, FeatureId, PackageId};
use crate::meta::{Classifier, EnumLiteral, FeatureKind, MetaRegistry};
use crate::operation::{MetaMultiplicity, MetaOperation};

/// A complete set of generated metamodel tables; indexes are ids.
#[derive(Debug)]
pub struct StaticPackages {
    pub packages: &'static [StaticPackage],
    pub classes: &'static [StaticClass],
    pub features: &'static [StaticFeature],
    pub enums: &'static [StaticEnum],
    pub data_types: &'static [StaticDataType],
    /// `EOperation` signatures (the registry keeps none; see
    /// [`MetaRegistry::static_operations`]).
    pub operations: &'static [StaticOperation],
}

/// An `EPackage`.
#[derive(Debug)]
pub struct StaticPackage {
    /// Qualified name, e.g. `pcm.repository`.
    pub name: &'static str,
    pub ns_uri: &'static str,
    pub prefix: &'static str,
    pub super_package: Option<PackageId>,
    /// `platform:/plugin/<bundle>/<path>` of the `.ecore` file (top-level packages only).
    pub location: Option<&'static str>,
    /// `eClassifiers` in declaration order.
    pub classifiers: &'static [Classifier],
}

impl StaticPackage {
    /// Unqualified name (`EPackage.getName()`).
    pub fn short_name(&self) -> &'static str {
        self.name.rsplit('.').next().unwrap_or(self.name)
    }
}

/// An `EEnum`.
#[derive(Debug)]
pub struct StaticEnum {
    pub name: &'static str,
    pub package: PackageId,
    pub literals: &'static [StaticLiteral],
}

/// An `EEnumLiteral`.
#[derive(Debug)]
pub struct StaticLiteral {
    pub name: &'static str,
    pub value: i32,
    pub literal: &'static str,
}

/// The type of a feature, parameter or operation, or a supertype.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum StaticType {
    Class(ClassId),
    Enum(EnumId),
    DataType(DataTypeId),
    /// A classifier of Ecore itself, by name (`EString`, `EObject`, ...).
    Ecore(&'static str),
    /// No classifier: a type parameter (`Sample<T>.value`), whose erasure is `EJavaObject`
    /// (attributes) or `EObject` (references).
    TypeParameter,
}

/// An `EClass`.
#[derive(Debug)]
pub struct StaticClass {
    pub name: &'static str,
    pub package: PackageId,
    /// The `abstract` attribute.
    pub is_abstract: bool,
    pub is_interface: bool,
    pub instance_class_name: Option<&'static str>,
    /// `eSuperTypes` in order, Ecore's classes (an explicit `EObject`) and generic supertypes
    /// (`Sample<Integer>`, as its class) included.
    pub super_types: &'static [StaticType],
    // ---- derived, for static readers ----
    /// The classes of `super_types` (Ecore's left out).
    pub supers: &'static [ClassId],
    /// `eAllSuperTypes` without Ecore's classes.
    pub all_supers: &'static [ClassId],
    /// `eAllStructuralFeatures` (the slot order).
    pub all_features: &'static [FeatureId],
    /// `eIDAttribute`.
    pub id_attribute: Option<FeatureId>,
}

impl StaticClass {
    /// Whether instances can be created (neither abstract nor an interface).
    pub fn instantiable(&self) -> bool {
        !self.is_abstract && !self.is_interface
    }
}

/// How a static reader parses an attribute's lexical values (a coarse view of the data type).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum LexicalKind {
    Str,
    /// `int`/`Integer`, `short`, `byte`.
    Int,
    Long,
    /// `double`/`Double`, `float`.
    Double,
    Bool,
    Enum(EnumId),
    /// Any other data type (kept as the lexical string).
    Other,
    /// An attribute typed by a type parameter (`Sample<T>.value`), whose erasure is
    /// `EJavaObject`: EMF converts literals by Java deserialisation of a hex string only.
    JavaObject,
}

/// Attribute or reference, with the facts a static reader needs.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum StaticFeatureKind {
    Attribute {
        data: LexicalKind,
    },
    Reference {
        /// `ClassId(u32::MAX)` if the target type is not a class of the tables.
        target: ClassId,
        containment: bool,
        /// The opposite is a containment (this is the `eContainer` side).
        container: bool,
        opposite: Option<FeatureId>,
        resolve_proxies: bool,
    },
}

/// An `EStructuralFeature`. Features are listed per owner class in declaration order.
#[derive(Debug)]
pub struct StaticFeature {
    pub name: &'static str,
    pub owner: ClassId,
    pub kind: StaticFeatureKind,
    pub ty: StaticType,
    /// `upperBound != 1`.
    pub many: bool,
    pub lower: i32,
    /// `upperBound` (`-1` unbounded).
    pub upper: i32,
    pub ordered: bool,
    pub unique: bool,
    pub changeable: bool,
    pub unsettable: bool,
    pub transient: bool,
    pub derived: bool,
    pub volatile: bool,
    pub is_id: bool,
    /// `defaultValueLiteral`.
    pub default: Option<&'static str>,
}

/// An `EDataType` defined by the metamodels (Ecore's are referred to by name).
#[derive(Debug)]
pub struct StaticDataType {
    pub name: &'static str,
    pub package: PackageId,
    pub instance_class_name: Option<&'static str>,
    pub serializable: bool,
}

/// A typed element (operation result or parameter): type and multiplicity.
#[derive(Debug)]
pub struct StaticTyped {
    pub ty: StaticType,
    pub upper: i32,
    pub ordered: bool,
    pub unique: bool,
}

/// An `EParameter`.
#[derive(Debug)]
pub struct StaticParam {
    pub name: &'static str,
    pub ty: StaticTyped,
}

/// An `EOperation` (signature only).
#[derive(Debug)]
pub struct StaticOperation {
    pub owner: ClassId,
    pub name: &'static str,
    /// `None` for `void`.
    pub result: Option<StaticTyped>,
    pub params: &'static [StaticParam],
}

/// What [`MetaRegistry::from_static`] leaves for [`MetaRegistry::resolve_static_ecore`].
#[derive(Clone, Debug, Default)]
pub(crate) struct PendingEcore {
    /// Feature, Ecore classifier name, typed by a type parameter.
    pub types: Vec<(FeatureId, &'static str, bool)>,
    /// Class, position among its supertypes, Ecore class name.
    pub supers: Vec<(ClassId, usize, &'static str)>,
}

impl MetaRegistry {
    /// A registry holding the static tables' metamodels with their ids (it must be the first
    /// content of the registry). Feature types and supertypes that name Ecore classifiers are
    /// resolved by [`MetaRegistry::resolve_static_ecore`] once Ecore is registered after them
    /// (the graph crate does that when it registers Ecore).
    pub fn from_static(seed: &'static StaticPackages) -> MetaRegistry {
        let mut meta = MetaRegistry::new();
        let mut pending = PendingEcore::default();
        for (i, p) in seed.packages.iter().enumerate() {
            let id = meta.add_package(
                p.short_name(),
                Some(p.ns_uri),
                Some(p.prefix),
                p.super_package,
            );
            assert_eq!(id.0, i as u32, "package order");
            // classifiers in declaration order: the tables number classes, enums and data types
            // in this walk's order, so the ids come out equal
            for k in p.classifiers {
                match *k {
                    Classifier::Class(c) => {
                        let d = &seed.classes[c.index()];
                        let cid = meta.add_class(id, d.name, d.is_abstract, d.is_interface);
                        assert_eq!(cid, c, "class order");
                        meta.class_mut(cid).instance_class_name =
                            d.instance_class_name.map(Into::into);
                        for (k, t) in d.super_types.iter().enumerate() {
                            match *t {
                                StaticType::Class(s) => meta.class_mut(cid).supers.push(s),
                                StaticType::Ecore(n) => pending.supers.push((cid, k, n)),
                                ref t => panic!("supertype {t:?} of {}", d.name),
                            }
                        }
                    }
                    Classifier::Enum(e) => {
                        let d = &seed.enums[e.index()];
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
                        assert_eq!(eid, e, "enum order");
                    }
                    Classifier::DataType(d) => {
                        let dd = &seed.data_types[d.index()];
                        let did = meta.add_data_type(
                            id,
                            dd.name,
                            dd.instance_class_name,
                            dd.serializable,
                        );
                        assert_eq!(did, d, "data type order");
                    }
                }
            }
        }
        // features in table order: per class in declaration order, so local indexes agree
        for (i, f) in seed.features.iter().enumerate() {
            let (kind, placeholder) = match f.kind {
                StaticFeatureKind::Attribute { .. } => (FeatureKind::Attribute, None),
                StaticFeatureKind::Reference { target, .. } => {
                    (FeatureKind::Reference, Some(Classifier::Class(target)))
                }
            };
            let ty = match f.ty {
                StaticType::Class(c) => Classifier::Class(c),
                StaticType::Enum(e) => Classifier::Enum(e),
                StaticType::DataType(d) => Classifier::DataType(d),
                StaticType::Ecore(_) | StaticType::TypeParameter => {
                    placeholder.unwrap_or(Classifier::Class(ClassId(0)))
                }
            };
            let fid = meta.add_feature(f.owner, f.name, kind, ty);
            assert_eq!(fid.0, i as u32, "feature order");
            match f.ty {
                StaticType::Ecore(n) => pending.types.push((fid, n, false)),
                // a type parameter: its erasure
                StaticType::TypeParameter => pending.types.push((
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
            fi.upper = f.upper;
            fi.ordered = f.ordered;
            fi.unique = f.unique;
            fi.changeable = f.changeable;
            fi.unsettable = f.unsettable;
            fi.volatile = f.volatile;
            fi.transient = f.transient;
            fi.derived = f.derived;
            fi.id = f.is_id;
            fi.default_literal = f.default.map(Into::into);
            if let StaticFeatureKind::Reference {
                containment,
                opposite,
                resolve_proxies,
                ..
            } = f.kind
            {
                fi.containment = containment;
                fi.resolve_proxies = resolve_proxies;
                fi.opposite = opposite;
            }
        }
        meta.finish_classes().expect("static metamodel tables");
        for i in 0..seed.packages.len() {
            meta.register_ns_uri(PackageId(i as u32));
        }
        meta.set_static(seed, pending);
        meta
    }

    /// Resolves the feature types and supertypes of the static tables that are Ecore classifiers
    /// (Ecore is registered in package `ecore` now). Does nothing when nothing is pending.
    pub fn resolve_static_ecore(&mut self, ecore: PackageId) {
        let Some(pending) = self.take_static_pending() else {
            return;
        };
        if !pending.supers.is_empty() {
            for &(c, k, name) in &pending.supers {
                let s = self
                    .class_by_name(ecore, name)
                    .unwrap_or_else(|| panic!("Ecore class {name}"));
                self.class_mut(c).supers.insert(k, s);
            }
            // the derived tables of every static class again (subclasses included)
            let n = self.static_seed().map_or(0, |s| s.classes.len());
            for i in 0..n {
                self.unfinish_class(ClassId(i as u32));
            }
            self.finish_classes().expect("static metamodel tables");
        }
        for &(f, name, tparam) in &pending.types {
            let k = self
                .classifier_by_name(ecore, name)
                .unwrap_or_else(|| panic!("Ecore classifier {name}"));
            let fi = self.feature_mut(f);
            fi.ty = k;
            fi.type_parameter = tparam;
        }
    }

    /// The `EOperation`s of the static tables (signatures; the generated code implements them),
    /// with Ecore's classifiers taken from package `ecore`. Operations whose signature has a
    /// type parameter are left out.
    pub fn static_operations(&self, ecore: PackageId) -> Vec<MetaOperation> {
        let Some(seed) = self.static_seed() else {
            return Vec::new();
        };
        let ty = |t: StaticType| -> Option<Classifier> {
            Some(match t {
                StaticType::Class(c) => Classifier::Class(c),
                StaticType::Enum(e) => Classifier::Enum(e),
                StaticType::DataType(d) => Classifier::DataType(d),
                StaticType::Ecore(n) => self.classifier_by_name(ecore, n)?,
                StaticType::TypeParameter => return None,
            })
        };
        let typed = |t: &StaticTyped| {
            ty(t.ty).map(|c| {
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
        'ops: for op in seed.operations {
            let mut params = Vec::new();
            for p in op.params {
                match typed(&p.ty) {
                    Some((c, m)) => params.push((p.name.to_string(), c, m)),
                    None => continue 'ops,
                }
            }
            ops.push(MetaOperation {
                class: op.owner,
                name: op.name.to_string(),
                params,
                result: op.result.as_ref().and_then(typed),
                body: None,
            });
        }
        ops
    }

    /// The top-level packages of the static tables with the locations other metamodels use for
    /// them (`platform:/plugin/<bundle>/<path>`).
    pub fn static_top_packages(&self) -> Vec<(PackageId, &'static str)> {
        self.static_seed().map_or_else(Vec::new, |s| {
            s.packages
                .iter()
                .enumerate()
                .filter_map(|(i, p)| p.location.map(|l| (PackageId(i as u32), l)))
                .collect()
        })
    }
}

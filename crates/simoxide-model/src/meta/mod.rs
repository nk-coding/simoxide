//! Static metamodel tables (EPackages, EClasses, EStructuralFeatures, EEnums) generated from the
//! `.ecore` files shipped in the SimuLizar 5.2.2 product (see `tools/gen_meta.py`), followed by
//! the SPD, semanticspd and spdmeasuringpoint packages of Slingshot (appended, so the ids of the
//! 5.2.2 part do not depend on them).
//!
//! The tables are the static seed of the EMF registry (`simoxide_emf_core::seed`): the ids here are
//! the registry's ids. The generic XMI layer ([`crate::raw`]) is driven entirely by these tables,
//! which is what makes it reproduce EMF's behaviour (feature lookup, defaults, containment order,
//! opposites, IDs) for every class, not just the typed v1 subset. The lookups on the ids are
//! extension traits ([`prelude`]).

use crate::fxhash::FxHashMap as HashMap;
use std::sync::OnceLock;

mod generated;
pub use generated::{
    CLASSES, DATA_TYPES, ENUMS, FEATURES, OPERATIONS, PACKAGES, SEED, class, enums, feat,
};

/// The table types are the EMF core's static seed types, under the names SimOxide uses.
pub use simoxide_emf_core::seed::{
    LexicalKind as DataKind, StaticClass as ClassDef, StaticDataType as DataTypeDef,
    StaticEnum as EnumDef, StaticFeature as FeatureDef, StaticFeatureKind as FeatureKind,
    StaticLiteral as EnumLiteral, StaticOperation as OperationDef, StaticPackage as PackageDef,
    StaticPackages, StaticParam as ParamDef, StaticType as TypeRef, StaticTyped as TypedDef,
};
/// The ids are the EMF core's: a registry seeded with [`SEED`] gives every entry of the tables
/// the same id.
pub use simoxide_emf_core::{ClassId, Classifier, DataTypeId, EnumId, FeatureId, PackageId};

/// The extension traits that give the ids their static lookups (`c.def()`, `c.is_a(..)`,
/// `f.is_containment()`, ...): `use simoxide_model::meta::prelude::*;`.
pub mod prelude {
    pub use super::{PcmClass, PcmEnum, PcmFeature, PcmMeta, PcmPackage};
}

/// The static definition of an id (`def()`) and its name.
pub trait PcmMeta: Copy {
    type Def: 'static;
    fn def(self) -> &'static Self::Def;
    fn name(self) -> &'static str;
}

impl PcmMeta for PackageId {
    type Def = PackageDef;
    fn def(self) -> &'static PackageDef {
        &PACKAGES[self.0 as usize]
    }
    /// Qualified name, e.g. `pcm.repository`.
    fn name(self) -> &'static str {
        self.def().name
    }
}

impl PcmMeta for EnumId {
    type Def = EnumDef;
    fn def(self) -> &'static EnumDef {
        &ENUMS[self.0 as usize]
    }
    fn name(self) -> &'static str {
        self.def().name
    }
}

impl PcmMeta for ClassId {
    type Def = ClassDef;
    fn def(self) -> &'static ClassDef {
        &CLASSES[self.0 as usize]
    }
    fn name(self) -> &'static str {
        self.def().name
    }
}

impl PcmMeta for FeatureId {
    type Def = FeatureDef;
    fn def(self) -> &'static FeatureDef {
        &FEATURES[self.0 as usize]
    }
    fn name(self) -> &'static str {
        self.def().name
    }
}

/// Static lookups of a package.
pub trait PcmPackage {
    /// Unqualified package name (EMF `EPackage.getName()`).
    fn short_name(self) -> &'static str;
}

impl PcmPackage for PackageId {
    fn short_name(self) -> &'static str {
        self.def().short_name()
    }
}

/// Static lookups of an enumeration.
pub trait PcmEnum {
    /// EMF `EEnum.getEEnumLiteralByLiteral` first, then by name (`EFactoryImpl.createFromString`).
    fn literal_by_name(self, s: &str) -> Option<u32>;
}

impl PcmEnum for EnumId {
    fn literal_by_name(self, s: &str) -> Option<u32> {
        let d = self.def();
        d.literals
            .iter()
            .position(|l| l.literal == s)
            .or_else(|| d.literals.iter().position(|l| l.name == s))
            .map(|i| i as u32)
    }
}

/// Static lookups of a class.
pub trait PcmClass: Copy {
    /// `package:Class`, the type name used in dumps and diagnostics.
    fn qualified_name(self) -> String;
    /// `self` is `other` or a subtype of it.
    fn is_a(self, other: ClassId) -> bool;
    /// EMF `getEStructuralFeature(name)`.
    fn feature(self, name: &str) -> Option<FeatureId>;
    /// `eAllStructuralFeatures`.
    fn features(self) -> std::iter::Copied<std::slice::Iter<'static, FeatureId>>;
    /// Containment features in EMF `eContents` order.
    fn containments(self) -> std::iter::Copied<std::slice::Iter<'static, FeatureId>>;
    /// Containment features in EMF `eContents` order (precomputed).
    fn containment_features(self) -> &'static [FeatureId];
    /// Persisted references with a lower bound of at least 1, in feature order (precomputed).
    #[doc(hidden)]
    fn required_references(self) -> &'static [FeatureId];
}

impl PcmClass for ClassId {
    fn qualified_name(self) -> String {
        format!("{}:{}", self.def().package.short_name(), self.def().name)
    }
    fn is_a(self, other: ClassId) -> bool {
        self == other || self.def().all_supers.contains(&other)
    }
    fn feature(self, name: &str) -> Option<FeatureId> {
        index().features.get(&(self, name)).copied()
    }
    fn features(self) -> std::iter::Copied<std::slice::Iter<'static, FeatureId>> {
        self.def().all_features.iter().copied()
    }
    fn containments(self) -> std::iter::Copied<std::slice::Iter<'static, FeatureId>> {
        self.containment_features().iter().copied()
    }
    fn containment_features(self) -> &'static [FeatureId] {
        &index().containments[self.0 as usize]
    }
    fn required_references(self) -> &'static [FeatureId] {
        &index().required_refs[self.0 as usize]
    }
}

/// Static lookups of a feature.
pub trait PcmFeature: Copy {
    fn is_attribute(self) -> bool;
    fn is_containment(self) -> bool;
    /// The opposite is a containment (this is the `eContainer` side).
    fn is_container(self) -> bool;
    fn opposite(self) -> Option<FeatureId>;
    /// The target class of a reference (`None` if not a class of the tables).
    fn target(self) -> Option<ClassId>;
    fn data_kind(self) -> Option<DataKind>;
    /// Plain cross reference as EMF serialises and `eCrossReferences` reports it.
    fn is_cross_reference(self) -> bool;
    /// `true` if the value is persisted (non-transient, non-derived).
    fn is_persistent(self) -> bool;
}

impl PcmFeature for FeatureId {
    fn is_attribute(self) -> bool {
        matches!(self.def().kind, FeatureKind::Attribute { .. })
    }
    fn is_containment(self) -> bool {
        matches!(
            self.def().kind,
            FeatureKind::Reference {
                containment: true,
                ..
            }
        )
    }
    fn is_container(self) -> bool {
        matches!(
            self.def().kind,
            FeatureKind::Reference {
                container: true,
                ..
            }
        )
    }
    fn opposite(self) -> Option<FeatureId> {
        match self.def().kind {
            FeatureKind::Reference { opposite, .. } => opposite,
            _ => None,
        }
    }
    fn target(self) -> Option<ClassId> {
        match self.def().kind {
            FeatureKind::Reference { target, .. } if target.0 != u32::MAX => Some(target),
            _ => None,
        }
    }
    fn data_kind(self) -> Option<DataKind> {
        match self.def().kind {
            FeatureKind::Attribute { data } => Some(data),
            _ => None,
        }
    }
    fn is_cross_reference(self) -> bool {
        let d = self.def();
        matches!(
            d.kind,
            FeatureKind::Reference {
                containment: false,
                container: false,
                ..
            }
        ) && !d.transient
            && !d.derived
    }
    fn is_persistent(self) -> bool {
        let d = self.def();
        !d.transient && !d.derived && !self.is_container()
    }
}

struct Index {
    by_ns: HashMap<&'static str, PackageId>,
    classes: HashMap<(PackageId, &'static str), ClassId>,
    features: HashMap<(ClassId, &'static str), FeatureId>,
    containments: Vec<Vec<FeatureId>>,
    required_refs: Vec<Vec<FeatureId>>,
}

fn index() -> &'static Index {
    static I: OnceLock<Index> = OnceLock::new();
    I.get_or_init(|| {
        let mut by_ns = HashMap::default();
        for (i, p) in PACKAGES.iter().enumerate() {
            by_ns.insert(p.ns_uri, PackageId(i as u32));
        }
        let mut classes = HashMap::default();
        let mut features = HashMap::default();
        for (i, c) in CLASSES.iter().enumerate() {
            let cid = ClassId(i as u32);
            classes.insert((c.package, c.name), cid);
            for &f in c.all_features {
                // first match wins, like EClassImpl.getEStructuralFeature(String)
                features.entry((cid, f.def().name)).or_insert(f);
            }
        }
        let containments = CLASSES
            .iter()
            .map(|c| {
                c.all_features
                    .iter()
                    .copied()
                    .filter(|f| f.is_containment())
                    .collect()
            })
            .collect();
        let required_refs = CLASSES
            .iter()
            .map(|c| {
                c.all_features
                    .iter()
                    .copied()
                    .filter(|f| {
                        let d = f.def();
                        d.lower >= 1
                            && !d.transient
                            && !d.derived
                            && !f.is_container()
                            && matches!(d.kind, FeatureKind::Reference { .. })
                    })
                    .collect()
            })
            .collect();
        Index {
            by_ns,
            classes,
            features,
            containments,
            required_refs,
        }
    })
}

/// Package registered under exactly this namespace URI.
pub fn package_by_ns(ns_uri: &str) -> Option<PackageId> {
    index().by_ns.get(ns_uri).copied()
}

/// Tolerant package lookup for other PCM versions: `.../PalladioComponentModel/Repository/5.0`
/// (or the old `sdq.ipd.uka.de` host) maps to the 5.2 package with the same path.
pub fn package_by_ns_tolerant(ns_uri: &str) -> Option<PackageId> {
    fn key(u: &str) -> Option<&str> {
        let rest = u.split_once("://")?.1;
        let path = rest.split_once('/')?.1;
        let (p, last) = path.rsplit_once('/')?;
        if last.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            Some(p)
        } else {
            Some(path)
        }
    }
    let k = key(ns_uri)?;
    PACKAGES
        .iter()
        .position(|p| key(p.ns_uri) == Some(k))
        .map(|i| PackageId(i as u32))
}

pub fn class_by_name(pkg: PackageId, name: &str) -> Option<ClassId> {
    index().classes.get(&(pkg, name)).copied()
}

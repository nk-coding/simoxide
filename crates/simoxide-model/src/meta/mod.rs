//! Static metamodel tables (EPackages, EClasses, EStructuralFeatures, EEnums) generated from the
//! `.ecore` files shipped in the SimuLizar 5.2.2 product (see `tools/gen_meta.py`).
//!
//! The generic XMI layer ([`crate::raw`]) is driven entirely by these tables, which is what makes
//! it reproduce EMF's behaviour (feature lookup, defaults, containment order, opposites, IDs) for
//! every class, not just the typed v1 subset.

use crate::fxhash::FxHashMap as HashMap;
use std::sync::OnceLock;

mod generated;
pub use generated::{CLASSES, ENUMS, FEATURES, PACKAGES, class, enums, feat};

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct PackageId(pub u32);
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct ClassId(pub u32);
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct FeatureId(pub u32);
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct EnumId(pub u32);

pub struct PackageDef {
    /// Qualified name, e.g. `pcm.repository`.
    pub name: &'static str,
    pub ns_uri: &'static str,
    pub prefix: &'static str,
}

pub struct EnumLiteral {
    pub name: &'static str,
    pub value: i32,
    pub literal: &'static str,
}

pub struct EnumDef {
    pub name: &'static str,
    pub package: PackageId,
    pub literals: &'static [EnumLiteral],
}

pub struct ClassDef {
    pub name: &'static str,
    pub package: PackageId,
    pub is_abstract: bool,
    pub supers: &'static [ClassId],
    /// EMF `eAllSuperTypes` order.
    pub all_supers: &'static [ClassId],
    /// EMF `eAllStructuralFeatures` order (determines containment traversal order).
    pub all_features: &'static [FeatureId],
    /// EMF `eIDAttribute`.
    pub id_attribute: Option<FeatureId>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum DataKind {
    Str,
    Int,
    Long,
    Double,
    Bool,
    Enum(EnumId),
    /// Any other data type (kept as the raw lexical string).
    Other,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum FeatureKind {
    Attribute {
        data: DataKind,
    },
    Reference {
        /// `ClassId(u32::MAX)` if the target type is outside the known metamodels.
        target: ClassId,
        containment: bool,
        /// The opposite is a containment reference (i.e. this is the `eContainer` side).
        container: bool,
        opposite: Option<FeatureId>,
        resolve_proxies: bool,
    },
}

pub struct FeatureDef {
    pub name: &'static str,
    pub owner: ClassId,
    pub kind: FeatureKind,
    pub many: bool,
    pub lower: i32,
    pub transient: bool,
    pub derived: bool,
    pub volatile: bool,
    pub is_id: bool,
    pub default: Option<&'static str>,
}

impl PackageId {
    pub fn def(self) -> &'static PackageDef {
        &PACKAGES[self.0 as usize]
    }
    /// Unqualified package name (EMF `EPackage.getName()`).
    pub fn short_name(self) -> &'static str {
        let n = self.def().name;
        n.rsplit('.').next().unwrap_or(n)
    }
}

impl EnumId {
    pub fn def(self) -> &'static EnumDef {
        &ENUMS[self.0 as usize]
    }
    pub fn literal_by_name(self, s: &str) -> Option<u32> {
        let d = self.def();
        // EMF `EEnum.getEEnumLiteralByLiteral` first, then by name (EFactoryImpl.createFromString)
        d.literals
            .iter()
            .position(|l| l.literal == s)
            .or_else(|| d.literals.iter().position(|l| l.name == s))
            .map(|i| i as u32)
    }
}

impl ClassId {
    pub fn def(self) -> &'static ClassDef {
        &CLASSES[self.0 as usize]
    }
    pub fn name(self) -> &'static str {
        self.def().name
    }
    /// `package:Class`, the type name used in dumps and diagnostics.
    pub fn qualified_name(self) -> String {
        format!("{}:{}", self.def().package.short_name(), self.def().name)
    }
    /// `self` is `other` or a subtype of it.
    pub fn is_a(self, other: ClassId) -> bool {
        self == other || self.def().all_supers.contains(&other)
    }
    /// EMF `getEStructuralFeature(name)`.
    pub fn feature(self, name: &str) -> Option<FeatureId> {
        index().features.get(&(self, name)).copied()
    }
    pub fn features(self) -> impl Iterator<Item = FeatureId> {
        self.def().all_features.iter().copied()
    }
    /// Containment features in EMF `eContents` order.
    pub fn containments(self) -> impl Iterator<Item = FeatureId> {
        self.containment_features().iter().copied()
    }
    /// Containment features in EMF `eContents` order (precomputed).
    pub fn containment_features(self) -> &'static [FeatureId] {
        &index().containments[self.0 as usize]
    }
    /// Persisted references with a lower bound of at least 1, in feature order (precomputed).
    pub(crate) fn required_references(self) -> &'static [FeatureId] {
        &index().required_refs[self.0 as usize]
    }
}

impl FeatureId {
    pub fn def(self) -> &'static FeatureDef {
        &FEATURES[self.0 as usize]
    }
    pub fn name(self) -> &'static str {
        self.def().name
    }
    pub fn is_attribute(self) -> bool {
        matches!(self.def().kind, FeatureKind::Attribute { .. })
    }
    pub fn is_containment(self) -> bool {
        matches!(
            self.def().kind,
            FeatureKind::Reference {
                containment: true,
                ..
            }
        )
    }
    pub fn is_container(self) -> bool {
        matches!(
            self.def().kind,
            FeatureKind::Reference {
                container: true,
                ..
            }
        )
    }
    pub fn opposite(self) -> Option<FeatureId> {
        match self.def().kind {
            FeatureKind::Reference { opposite, .. } => opposite,
            _ => None,
        }
    }
    pub fn target(self) -> Option<ClassId> {
        match self.def().kind {
            FeatureKind::Reference { target, .. } if target.0 != u32::MAX => Some(target),
            _ => None,
        }
    }
    pub fn data_kind(self) -> Option<DataKind> {
        match self.def().kind {
            FeatureKind::Attribute { data } => Some(data),
            _ => None,
        }
    }
    /// Plain cross reference as EMF serialises and `eCrossReferences` reports it.
    pub fn is_cross_reference(self) -> bool {
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
    /// `true` if the value is persisted (non-transient, non-derived).
    pub fn is_persistent(self) -> bool {
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

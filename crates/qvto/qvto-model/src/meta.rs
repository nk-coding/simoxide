//! Metamodels: packages, classes, features, data types and enumerations, resolved to dense ids.
//!
//! A [`MetaRegistry`] is built once per set of metamodels (from `.ecore` files by `qvto-emf`, from
//! Ecore itself, or programmatically) and only grows: ids never change. The derived tables of a
//! class (all supertypes, all features, slot offsets, containments, ID attribute) follow EMF's
//! `EClassImpl` exactly, so that feature order, `eContents` order and slot indexes agree with
//! EMF's `eAllStructuralFeatures`.

use crate::ids::{ClassId, DataTypeId, EnumId, FeatureId, PackageId};
use std::collections::HashMap;
use std::fmt;

/// The type of a feature: a class, a data type or an enumeration (`EClassifier`).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Classifier {
    Class(ClassId),
    DataType(DataTypeId),
    Enum(EnumId),
}

/// The Java representation of a data type's values, which decides parsing, printing, defaults
/// and equality (Java boxes of different classes are never equal).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum DataKind {
    String,
    Boolean,
    /// `int` / `java.lang.Integer`.
    Int,
    Long,
    Short,
    Byte,
    /// `char` / `java.lang.Character` (a UTF-16 code unit).
    Char,
    Double,
    Float,
    BigInteger,
    BigDecimal,
    /// `java.util.Date`, kept in its lexical form.
    Date,
    /// A Java class whose values are kept in lexical form but which EMF cannot parse from a
    /// string (`java.lang.Object`, `EJavaClass`, `EByteArray`, ...).
    Other,
    /// No instance class EMF could load (dynamic data types without a known Java class): EMF's
    /// `createFromString` yields `null` for every literal.
    Unloadable,
}

impl DataKind {
    /// The kind for a Java instance class name, as `EcoreUtil.wrapperClassFor` sees it.
    pub fn from_instance_class(name: Option<&str>) -> (DataKind, bool) {
        let Some(name) = name else {
            return (DataKind::Unloadable, false);
        };
        match name {
            "java.lang.String" => (DataKind::String, false),
            "boolean" => (DataKind::Boolean, true),
            "java.lang.Boolean" => (DataKind::Boolean, false),
            "int" => (DataKind::Int, true),
            "java.lang.Integer" => (DataKind::Int, false),
            "long" => (DataKind::Long, true),
            "java.lang.Long" => (DataKind::Long, false),
            "short" => (DataKind::Short, true),
            "java.lang.Short" => (DataKind::Short, false),
            "byte" => (DataKind::Byte, true),
            "java.lang.Byte" => (DataKind::Byte, false),
            "char" => (DataKind::Char, true),
            "java.lang.Character" => (DataKind::Char, false),
            "double" => (DataKind::Double, true),
            "java.lang.Double" => (DataKind::Double, false),
            "float" => (DataKind::Float, true),
            "java.lang.Float" => (DataKind::Float, false),
            "java.math.BigInteger" => (DataKind::BigInteger, false),
            "java.math.BigDecimal" => (DataKind::BigDecimal, false),
            "java.util.Date" => (DataKind::Date, false),
            "java.lang.Object" | "java.lang.Class" | "byte[]" => (DataKind::Other, false),
            n if n.starts_with("java.") || n.starts_with("org.eclipse.emf.") => {
                (DataKind::Other, false)
            }
            _ => (DataKind::Unloadable, false),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PackageInfo {
    pub name: Box<str>,
    pub ns_uri: Option<Box<str>>,
    pub ns_prefix: Option<Box<str>>,
    pub super_package: Option<PackageId>,
    pub subpackages: Vec<PackageId>,
    /// `eClassifiers` in declaration order.
    pub classifiers: Vec<Classifier>,
}

#[derive(Clone, Debug)]
pub struct ClassInfo {
    pub name: Box<str>,
    pub package: PackageId,
    pub is_abstract: bool,
    pub is_interface: bool,
    pub instance_class_name: Option<Box<str>>,
    /// `eSuperTypes`.
    pub supers: Vec<ClassId>,
    /// `eStructuralFeatures` (local features) in declaration order.
    pub features: Vec<FeatureId>,
    // ---- derived by `MetaRegistry::finish_classes` ----
    /// `eAllSuperTypes` (EMF order: for each direct supertype its supertypes, then itself).
    pub all_supers: Box<[ClassId]>,
    /// `eAllStructuralFeatures`: the slot order of an object of this class.
    pub all_features: Box<[FeatureId]>,
    /// The containment features of `all_features` (the `eContents` order).
    pub containments: Box<[FeatureId]>,
    /// The non-containment, non-container references of `all_features` (`eCrossReferences`).
    pub cross_references: Box<[FeatureId]>,
    /// `eIDAttribute`: the first attribute with `iD` in `eAllAttributes` order.
    pub id_attribute: Option<FeatureId>,
    /// Offset of each owner's block of local features within `all_features`.
    blocks: Box<[(ClassId, u32)]>,
    /// Bit set of this class and all its supertypes.
    supertype_bits: Box<[u64]>,
    finished: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum FeatureKind {
    Attribute,
    Reference,
}

#[derive(Clone, Debug)]
pub struct FeatureInfo {
    pub name: Box<str>,
    pub owner: ClassId,
    /// Index in the owner's local feature list.
    pub local_index: u32,
    pub kind: FeatureKind,
    pub ty: Classifier,
    pub lower: i32,
    /// `-1` is unbounded (`*`), `-2` unspecified.
    pub upper: i32,
    pub ordered: bool,
    pub unique: bool,
    pub changeable: bool,
    pub volatile: bool,
    pub transient: bool,
    pub unsettable: bool,
    pub derived: bool,
    /// `EAttribute.iD`.
    pub id: bool,
    pub containment: bool,
    pub resolve_proxies: bool,
    pub opposite: Option<FeatureId>,
    /// `EReference.eKeys`.
    pub keys: Vec<FeatureId>,
    pub default_literal: Option<Box<str>>,
    /// The feature's generic type is a type parameter (affects `xsi:type` in XMI).
    pub type_parameter: bool,
}

impl FeatureInfo {
    pub fn is_many(&self) -> bool {
        self.upper > 1 || self.upper == -1
    }
    pub fn is_attribute(&self) -> bool {
        self.kind == FeatureKind::Attribute
    }
    pub fn is_reference(&self) -> bool {
        self.kind == FeatureKind::Reference
    }
    pub fn class_type(&self) -> Option<ClassId> {
        match self.ty {
            Classifier::Class(c) => Some(c),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DataTypeInfo {
    pub name: Box<str>,
    pub package: PackageId,
    pub instance_class_name: Option<Box<str>>,
    pub kind: DataKind,
    /// Java primitive (`int`, `boolean`, ...): has an intrinsic default and cannot hold null.
    pub primitive: bool,
    pub serializable: bool,
}

#[derive(Clone, Debug)]
pub struct EnumLiteral {
    pub name: Box<str>,
    pub value: i32,
    /// The literal string (EMF: the name when not set).
    pub literal: Box<str>,
}

#[derive(Clone, Debug)]
pub struct EnumInfo {
    pub name: Box<str>,
    pub package: PackageId,
    pub literals: Vec<EnumLiteral>,
    pub serializable: bool,
}

impl EnumInfo {
    /// `EEnum.getEEnumLiteralByLiteral`.
    pub fn by_literal(&self, s: &str) -> Option<u32> {
        self.literals
            .iter()
            .position(|l| &*l.literal == s)
            .map(|i| i as u32)
    }
    /// `EEnum.getEEnumLiteral(name)`.
    pub fn by_name(&self, s: &str) -> Option<u32> {
        self.literals
            .iter()
            .position(|l| &*l.name == s)
            .map(|i| i as u32)
    }
}

/// An error found while finishing a batch of classes.
#[derive(Clone, Debug, PartialEq)]
pub struct MetaError(pub String);

impl fmt::Display for MetaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for MetaError {}

/// All metamodels known to one engine instance.
#[derive(Clone, Debug, Default)]
pub struct MetaRegistry {
    packages: Vec<PackageInfo>,
    classes: Vec<ClassInfo>,
    features: Vec<FeatureInfo>,
    data_types: Vec<DataTypeInfo>,
    enums: Vec<EnumInfo>,
    by_ns_uri: HashMap<Box<str>, PackageId>,
    /// Ecore's `EObject`, the implicit supertype of every class, once Ecore is registered.
    eobject: Option<ClassId>,
}

impl MetaRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- reading ----------------------------------------------------------------------------

    pub fn package(&self, p: PackageId) -> &PackageInfo {
        &self.packages[p.index()]
    }
    pub fn class(&self, c: ClassId) -> &ClassInfo {
        &self.classes[c.index()]
    }
    pub fn feature(&self, f: FeatureId) -> &FeatureInfo {
        &self.features[f.index()]
    }
    pub fn data_type(&self, d: DataTypeId) -> &DataTypeInfo {
        &self.data_types[d.index()]
    }
    pub fn enumeration(&self, e: EnumId) -> &EnumInfo {
        &self.enums[e.index()]
    }
    pub fn package_count(&self) -> usize {
        self.packages.len()
    }
    pub fn class_count(&self) -> usize {
        self.classes.len()
    }
    pub fn feature_count(&self) -> usize {
        self.features.len()
    }
    pub fn data_type_count(&self) -> usize {
        self.data_types.len()
    }
    pub fn enum_count(&self) -> usize {
        self.enums.len()
    }
    pub fn packages(&self) -> impl Iterator<Item = PackageId> + '_ {
        (0..self.packages.len() as u32).map(PackageId)
    }
    pub fn classes(&self) -> impl Iterator<Item = ClassId> + '_ {
        (0..self.classes.len() as u32).map(ClassId)
    }

    /// The package registered under a namespace URI (top-level or nested).
    pub fn package_by_ns_uri(&self, ns_uri: &str) -> Option<PackageId> {
        self.by_ns_uri.get(ns_uri).copied()
    }

    /// Registers `p` under its namespace URI (the latest registration wins, as in
    /// `EPackage.Registry.put`).
    pub fn register_ns_uri(&mut self, p: PackageId) {
        if let Some(u) = self.packages[p.index()].ns_uri.clone() {
            self.by_ns_uri.insert(u, p);
        }
    }

    /// Registers `p` under an extra key (e.g. a `platform:/plugin/...` location).
    pub fn register_alias(&mut self, key: &str, p: PackageId) {
        self.by_ns_uri.insert(key.into(), p);
    }

    /// `EPackage.getEClassifier(name)`.
    pub fn classifier_by_name(&self, p: PackageId, name: &str) -> Option<Classifier> {
        self.packages[p.index()]
            .classifiers
            .iter()
            .copied()
            .find(|c| self.classifier_name(*c) == name)
    }

    pub fn class_by_name(&self, p: PackageId, name: &str) -> Option<ClassId> {
        match self.classifier_by_name(p, name)? {
            Classifier::Class(c) => Some(c),
            _ => None,
        }
    }

    pub fn classifier_name(&self, c: Classifier) -> &str {
        match c {
            Classifier::Class(c) => &self.classes[c.index()].name,
            Classifier::DataType(d) => &self.data_types[d.index()].name,
            Classifier::Enum(e) => &self.enums[e.index()].name,
        }
    }

    pub fn classifier_package(&self, c: Classifier) -> PackageId {
        match c {
            Classifier::Class(c) => self.classes[c.index()].package,
            Classifier::DataType(d) => self.data_types[d.index()].package,
            Classifier::Enum(e) => self.enums[e.index()].package,
        }
    }

    /// `EClass.getEStructuralFeature(name)`: the first feature of that name in
    /// `eAllStructuralFeatures`.
    pub fn feature_by_name(&self, c: ClassId, name: &str) -> Option<FeatureId> {
        self.classes[c.index()]
            .all_features
            .iter()
            .copied()
            .find(|f| &*self.features[f.index()].name == name)
    }

    /// Slot of `f` in objects of class `c` (its index in `c`'s `all_features`).
    #[inline]
    pub fn slot(&self, c: ClassId, f: FeatureId) -> Option<usize> {
        let fi = &self.features[f.index()];
        let ci = &self.classes[c.index()];
        let base = ci.blocks.iter().find(|(o, _)| *o == fi.owner)?.1;
        Some(base as usize + fi.local_index as usize)
    }

    /// `c` is `sup` or one of its subclasses (`sup.isSuperTypeOf(c)`); every class conforms to
    /// Ecore's `EObject`.
    #[inline]
    pub fn is_subclass(&self, c: ClassId, sup: ClassId) -> bool {
        if c == sup || Some(sup) == self.eobject {
            return true;
        }
        let bits = &self.classes[c.index()].supertype_bits;
        let i = sup.index();
        bits.get(i / 64).is_some_and(|w| w & (1u64 << (i % 64)) != 0)
    }

    /// Ecore's `EObject`, if Ecore is registered.
    pub fn eobject_class(&self) -> Option<ClassId> {
        self.eobject
    }
    pub fn set_eobject_class(&mut self, c: ClassId) {
        self.eobject = Some(c);
    }

    /// `EReference.isContainer()`: the opposite is a containment.
    pub fn is_container(&self, f: FeatureId) -> bool {
        self.features[f.index()]
            .opposite
            .is_some_and(|o| self.features[o.index()].containment)
    }

    /// Qualified class name `pkg.sub::Class` for messages.
    pub fn qualified_name(&self, c: ClassId) -> String {
        let ci = &self.classes[c.index()];
        format!("{}::{}", self.package_path(ci.package, "."), ci.name)
    }

    /// Package names from the root package, joined by `sep`.
    pub fn package_path(&self, p: PackageId, sep: &str) -> String {
        let mut names = vec![&*self.packages[p.index()].name];
        let mut cur = self.packages[p.index()].super_package;
        while let Some(q) = cur {
            names.push(&self.packages[q.index()].name);
            cur = self.packages[q.index()].super_package;
        }
        names.reverse();
        names.join(sep)
    }

    /// The default value of a single-valued attribute as its lexical form: the feature's
    /// `defaultValueLiteral` if any. The intrinsic default of the type is up to the store.
    pub fn default_literal(&self, f: FeatureId) -> Option<&str> {
        self.features[f.index()].default_literal.as_deref()
    }

    // ---- building ---------------------------------------------------------------------------

    pub fn add_package(
        &mut self,
        name: &str,
        ns_uri: Option<&str>,
        ns_prefix: Option<&str>,
        super_package: Option<PackageId>,
    ) -> PackageId {
        let id = PackageId(self.packages.len() as u32);
        self.packages.push(PackageInfo {
            name: name.into(),
            ns_uri: ns_uri.map(Into::into),
            ns_prefix: ns_prefix.map(Into::into),
            super_package,
            subpackages: Vec::new(),
            classifiers: Vec::new(),
        });
        if let Some(s) = super_package {
            self.packages[s.index()].subpackages.push(id);
        }
        id
    }

    /// Declares a class; its derived tables are computed by [`MetaRegistry::finish_classes`].
    pub fn add_class(&mut self, p: PackageId, name: &str, is_abstract: bool, is_interface: bool) -> ClassId {
        let id = ClassId(self.classes.len() as u32);
        self.classes.push(ClassInfo {
            name: name.into(),
            package: p,
            is_abstract,
            is_interface,
            instance_class_name: None,
            supers: Vec::new(),
            features: Vec::new(),
            all_supers: Box::new([]),
            all_features: Box::new([]),
            containments: Box::new([]),
            cross_references: Box::new([]),
            id_attribute: None,
            blocks: Box::new([]),
            supertype_bits: Box::new([]),
            finished: false,
        });
        self.packages[p.index()].classifiers.push(Classifier::Class(id));
        id
    }

    pub fn add_data_type(
        &mut self,
        p: PackageId,
        name: &str,
        instance_class_name: Option<&str>,
        serializable: bool,
    ) -> DataTypeId {
        let id = DataTypeId(self.data_types.len() as u32);
        let (kind, primitive) = DataKind::from_instance_class(instance_class_name);
        self.data_types.push(DataTypeInfo {
            name: name.into(),
            package: p,
            instance_class_name: instance_class_name.map(Into::into),
            kind,
            primitive,
            serializable,
        });
        self.packages[p.index()].classifiers.push(Classifier::DataType(id));
        id
    }

    pub fn add_enum(&mut self, p: PackageId, name: &str, literals: Vec<EnumLiteral>) -> EnumId {
        let id = EnumId(self.enums.len() as u32);
        self.enums.push(EnumInfo {
            name: name.into(),
            package: p,
            literals,
            serializable: true,
        });
        self.packages[p.index()].classifiers.push(Classifier::Enum(id));
        id
    }

    /// Adds a feature to class `c` with EMF's defaults (`[0..1]`, ordered, unique, changeable,
    /// resolving proxies); adjust it with [`MetaRegistry::feature_mut`].
    pub fn add_feature(&mut self, c: ClassId, name: &str, kind: FeatureKind, ty: Classifier) -> FeatureId {
        let id = FeatureId(self.features.len() as u32);
        let local_index = self.classes[c.index()].features.len() as u32;
        self.features.push(FeatureInfo {
            name: name.into(),
            owner: c,
            local_index,
            kind,
            ty,
            lower: 0,
            upper: 1,
            ordered: true,
            unique: true,
            changeable: true,
            volatile: false,
            transient: false,
            unsettable: false,
            derived: false,
            id: false,
            containment: false,
            resolve_proxies: true,
            opposite: None,
            keys: Vec::new(),
            default_literal: None,
            type_parameter: false,
        });
        self.classes[c.index()].features.push(id);
        id
    }

    pub fn class_mut(&mut self, c: ClassId) -> &mut ClassInfo {
        &mut self.classes[c.index()]
    }
    pub fn feature_mut(&mut self, f: FeatureId) -> &mut FeatureInfo {
        &mut self.features[f.index()]
    }
    pub fn data_type_mut(&mut self, d: DataTypeId) -> &mut DataTypeInfo {
        &mut self.data_types[d.index()]
    }
    pub fn enum_mut(&mut self, e: EnumId) -> &mut EnumInfo {
        &mut self.enums[e.index()]
    }

    /// Computes the derived tables of every class that is not finished yet. Classes of one batch
    /// may refer to each other in any order; supertypes must be finished or in the batch.
    pub fn finish_classes(&mut self) -> Result<(), MetaError> {
        let n = self.classes.len();
        let words = n.div_ceil(64).max(1);
        let pending: Vec<ClassId> = (0..n as u32)
            .map(ClassId)
            .filter(|c| !self.classes[c.index()].finished)
            .collect();
        let mut state = vec![0u8; n]; // 0 = todo, 1 = in progress, 2 = done
        for c in 0..n {
            if self.classes[c].finished {
                state[c] = 2;
            }
        }
        let mut errors = Vec::new();
        for &c in &pending {
            self.finish_one(c, &mut state, words, &mut errors);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(MetaError(errors.join("; ")))
        }
    }

    fn finish_one(&mut self, c: ClassId, state: &mut [u8], words: usize, errors: &mut Vec<String>) {
        match state[c.index()] {
            2 => return,
            1 => {
                errors.push(format!("cyclic supertypes at {}", self.classes[c.index()].name));
                return;
            }
            _ => {}
        }
        state[c.index()] = 1;
        let supers = self.classes[c.index()].supers.clone();
        for &s in &supers {
            self.finish_one(s, state, words, errors);
        }
        // eAllSuperTypes
        let mut all_supers: Vec<ClassId> = Vec::new();
        for &s in &supers {
            for &x in self.classes[s.index()].all_supers.iter() {
                if !all_supers.contains(&x) {
                    all_supers.push(x);
                }
            }
            if !all_supers.contains(&s) {
                all_supers.push(s);
            }
        }
        // eAllStructuralFeatures
        let mut all_features: Vec<FeatureId> = Vec::new();
        for &s in &supers {
            for &f in self.classes[s.index()].all_features.iter() {
                if !all_features.contains(&f) {
                    all_features.push(f);
                }
            }
        }
        for &f in &self.classes[c.index()].features {
            if !all_features.contains(&f) {
                all_features.push(f);
            }
        }
        let mut blocks: Vec<(ClassId, u32)> = Vec::new();
        for (i, &f) in all_features.iter().enumerate() {
            let fi = &self.features[f.index()];
            if !blocks.iter().any(|(o, _)| *o == fi.owner) {
                blocks.push((fi.owner, i as u32 - fi.local_index));
            }
        }
        let mut containments = Vec::new();
        let mut cross = Vec::new();
        let mut attrs = Vec::new();
        for &f in &all_features {
            let fi = &self.features[f.index()];
            match fi.kind {
                FeatureKind::Reference => {
                    if fi.containment {
                        containments.push(f);
                    } else if !self.is_container(f) {
                        cross.push(f);
                    }
                }
                FeatureKind::Attribute => attrs.push(f),
            }
        }
        let id_attribute = attrs.iter().copied().find(|f| self.features[f.index()].id);
        let mut bits = vec![0u64; words];
        bits[c.index() / 64] |= 1 << (c.index() % 64);
        for &s in &all_supers {
            bits[s.index() / 64] |= 1 << (s.index() % 64);
        }
        let ci = &mut self.classes[c.index()];
        ci.all_supers = all_supers.into();
        ci.all_features = all_features.into();
        ci.containments = containments.into();
        ci.cross_references = cross.into();
        ci.id_attribute = id_attribute;
        ci.blocks = blocks.into();
        ci.supertype_bits = bits.into();
        ci.finished = true;
        state[c.index()] = 2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diamond_features_and_slots() {
        let mut r = MetaRegistry::new();
        let p = r.add_package("p", Some("http://p"), Some("p"), None);
        let s = r.add_data_type(p, "S", Some("java.lang.String"), true);
        let a = r.add_class(p, "A", true, false);
        let b = r.add_class(p, "B", true, false);
        let c = r.add_class(p, "C", true, false);
        let d = r.add_class(p, "D", false, false);
        r.class_mut(b).supers.push(a);
        r.class_mut(c).supers.push(a);
        r.class_mut(d).supers.extend([b, c]);
        let fa = r.add_feature(a, "a", FeatureKind::Attribute, Classifier::DataType(s));
        let fb = r.add_feature(b, "b", FeatureKind::Attribute, Classifier::DataType(s));
        let fc1 = r.add_feature(c, "c1", FeatureKind::Attribute, Classifier::DataType(s));
        let fc2 = r.add_feature(c, "c2", FeatureKind::Attribute, Classifier::DataType(s));
        let fd = r.add_feature(d, "d", FeatureKind::Attribute, Classifier::DataType(s));
        r.finish_classes().unwrap();
        assert_eq!(&*r.class(d).all_supers, &[a, b, c]);
        assert_eq!(&*r.class(d).all_features, &[fa, fb, fc1, fc2, fd]);
        assert_eq!(r.slot(d, fc2), Some(3));
        assert_eq!(r.slot(c, fc2), Some(2));
        assert_eq!(r.slot(b, fc2), None);
        assert!(r.is_subclass(d, a) && r.is_subclass(d, c) && !r.is_subclass(a, d));
        assert_eq!(r.feature_by_name(d, "c1"), Some(fc1));
    }
}

//! Ecore itself: the built-in registry entries of the Ecore package (bootstrapped from EMF's
//! `Ecore.ecore`, bundled in `models/`), and the ids of the Ecore classes and features that the
//! store treats specially (generated-code behaviour of `EClassImpl`, `ETypedElementImpl`, ...).

use qvto_model::{
    ClassId, Classifier, EnumLiteral, FeatureId, FeatureKind, MetaRegistry, PackageId,
};
use std::collections::HashMap;

/// EMF 2.41's `Ecore.ecore` (EPL-2.0).
pub const ECORE_ECORE: &str = include_str!("../models/Ecore.ecore");
pub const ECORE_NS_URI: &str = "http://www.eclipse.org/emf/2002/Ecore";

/// A minimal element tree of an XML document (for bootstrapping only).
#[derive(Debug, Default)]
pub(crate) struct El {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<El>,
}

impl El {
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }
    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }
}

pub(crate) fn parse_tree(text: &str) -> Result<El, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(text);
    let mut stack: Vec<El> = vec![El::default()];
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) => {
                stack.push(element(&e)?);
            }
            Event::Empty(e) => {
                let el = element(&e)?;
                stack.last_mut().unwrap().children.push(el);
            }
            Event::End(_) => {
                let el = stack.pop().unwrap();
                stack.last_mut().ok_or("unbalanced")?.children.push(el);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let mut doc = stack.pop().ok_or("empty")?;
    doc.children.pop().ok_or_else(|| "no root element".into())
}

fn element(e: &quick_xml::events::BytesStart) -> Result<El, String> {
    let mut el = El {
        name: String::from_utf8_lossy(e.name().as_ref()).into_owned(),
        ..El::default()
    };
    for a in e.attributes() {
        let a = a.map_err(|x| x.to_string())?;
        let v = a.unescape_value().map_err(|x| x.to_string())?;
        el.attrs.push((String::from_utf8_lossy(a.key.as_ref()).into_owned(), v.into_owned()));
    }
    Ok(el)
}

macro_rules! ecore_ids {
    (classes { $($c:ident = $cn:literal),* $(,)? } features { $($f:ident = $fc:literal . $fname:literal),* $(,)? }) => {
        /// Ids of the Ecore package in a registry.
        #[derive(Clone, Debug)]
        pub struct EcoreIds {
            pub package: PackageId,
            /// Classes of Ecore have ids `0..class_end`, features `0..feature_end` (Ecore is
            /// always registered first).
            pub class_end: u32,
            pub feature_end: u32,
            $(pub $c: ClassId,)*
            $(pub $f: FeatureId,)*
        }

        impl EcoreIds {
            fn resolve(reg: &MetaRegistry, p: PackageId) -> EcoreIds {
                let class = |n: &str| reg.class_by_name(p, n).unwrap_or_else(|| panic!("Ecore class {n}"));
                let feature = |c: &str, n: &str| {
                    reg.feature_by_name(class(c), n).unwrap_or_else(|| panic!("Ecore feature {c}.{n}"))
                };
                EcoreIds {
                    package: p,
                    class_end: reg.class_count() as u32,
                    feature_end: reg.feature_count() as u32,
                    $($c: class($cn),)*
                    $($f: feature($fc, $fname),)*
                }
            }
        }
    };
}

ecore_ids! {
    classes {
        eobject = "EObject",
        emodel_element = "EModelElement",
        enamed_element = "ENamedElement",
        eclassifier = "EClassifier",
        eclass = "EClass",
        edata_type = "EDataType",
        eenum = "EEnum",
        eenum_literal = "EEnumLiteral",
        epackage = "EPackage",
        efactory = "EFactory",
        etyped_element = "ETypedElement",
        estructural_feature = "EStructuralFeature",
        eattribute = "EAttribute",
        ereference = "EReference",
        eoperation = "EOperation",
        eparameter = "EParameter",
        eannotation = "EAnnotation",
        string_map_entry = "EStringToStringMapEntry",
        egeneric_type = "EGenericType",
        etype_parameter = "ETypeParameter",
    }
    features {
        model_element_annotations = "EModelElement" . "eAnnotations",
        named_element_name = "ENamedElement" . "name",
        annotation_source = "EAnnotation" . "source",
        annotation_details = "EAnnotation" . "details",
        annotation_contents = "EAnnotation" . "contents",
        annotation_references = "EAnnotation" . "references",
        entry_key = "EStringToStringMapEntry" . "key",
        entry_value = "EStringToStringMapEntry" . "value",
        classifier_instance_class_name = "EClassifier" . "instanceClassName",
        classifier_instance_class = "EClassifier" . "instanceClass",
        classifier_default_value = "EClassifier" . "defaultValue",
        classifier_instance_type_name = "EClassifier" . "instanceTypeName",
        classifier_package = "EClassifier" . "ePackage",
        classifier_type_parameters = "EClassifier" . "eTypeParameters",
        class_abstract = "EClass" . "abstract",
        class_interface = "EClass" . "interface",
        class_super_types = "EClass" . "eSuperTypes",
        class_operations = "EClass" . "eOperations",
        class_all_attributes = "EClass" . "eAllAttributes",
        class_all_references = "EClass" . "eAllReferences",
        class_references = "EClass" . "eReferences",
        class_attributes = "EClass" . "eAttributes",
        class_all_containments = "EClass" . "eAllContainments",
        class_all_operations = "EClass" . "eAllOperations",
        class_all_structural_features = "EClass" . "eAllStructuralFeatures",
        class_all_super_types = "EClass" . "eAllSuperTypes",
        class_id_attribute = "EClass" . "eIDAttribute",
        class_structural_features = "EClass" . "eStructuralFeatures",
        class_generic_super_types = "EClass" . "eGenericSuperTypes",
        class_all_generic_super_types = "EClass" . "eAllGenericSuperTypes",
        data_type_serializable = "EDataType" . "serializable",
        enum_literals = "EEnum" . "eLiterals",
        literal_value = "EEnumLiteral" . "value",
        literal_instance = "EEnumLiteral" . "instance",
        literal_literal = "EEnumLiteral" . "literal",
        literal_enum = "EEnumLiteral" . "eEnum",
        package_ns_uri = "EPackage" . "nsURI",
        package_ns_prefix = "EPackage" . "nsPrefix",
        package_factory = "EPackage" . "eFactoryInstance",
        package_classifiers = "EPackage" . "eClassifiers",
        package_subpackages = "EPackage" . "eSubpackages",
        package_super_package = "EPackage" . "eSuperPackage",
        typed_ordered = "ETypedElement" . "ordered",
        typed_unique = "ETypedElement" . "unique",
        typed_lower = "ETypedElement" . "lowerBound",
        typed_upper = "ETypedElement" . "upperBound",
        typed_many = "ETypedElement" . "many",
        typed_required = "ETypedElement" . "required",
        typed_type = "ETypedElement" . "eType",
        typed_generic_type = "ETypedElement" . "eGenericType",
        feature_changeable = "EStructuralFeature" . "changeable",
        feature_volatile = "EStructuralFeature" . "volatile",
        feature_transient = "EStructuralFeature" . "transient",
        feature_default_literal = "EStructuralFeature" . "defaultValueLiteral",
        feature_default_value = "EStructuralFeature" . "defaultValue",
        feature_unsettable = "EStructuralFeature" . "unsettable",
        feature_derived = "EStructuralFeature" . "derived",
        feature_containing_class = "EStructuralFeature" . "eContainingClass",
        attribute_id = "EAttribute" . "iD",
        attribute_type = "EAttribute" . "eAttributeType",
        reference_containment = "EReference" . "containment",
        reference_container = "EReference" . "container",
        reference_resolve_proxies = "EReference" . "resolveProxies",
        reference_opposite = "EReference" . "eOpposite",
        reference_type = "EReference" . "eReferenceType",
        reference_keys = "EReference" . "eKeys",
        operation_containing_class = "EOperation" . "eContainingClass",
        operation_type_parameters = "EOperation" . "eTypeParameters",
        operation_parameters = "EOperation" . "eParameters",
        operation_exceptions = "EOperation" . "eExceptions",
        operation_generic_exceptions = "EOperation" . "eGenericExceptions",
        generic_upper_bound = "EGenericType" . "eUpperBound",
        generic_type_arguments = "EGenericType" . "eTypeArguments",
        generic_raw_type = "EGenericType" . "eRawType",
        generic_lower_bound = "EGenericType" . "eLowerBound",
        generic_type_parameter = "EGenericType" . "eTypeParameter",
        generic_classifier = "EGenericType" . "eClassifier",
        type_parameter_bounds = "ETypeParameter" . "eBounds",
    }
}

impl EcoreIds {
    #[inline]
    pub fn is_ecore_class(&self, c: ClassId) -> bool {
        c.0 < self.class_end
    }
    #[inline]
    pub fn is_ecore_feature(&self, f: FeatureId) -> bool {
        f.0 < self.feature_end
    }
}

/// Registers the Ecore package in an empty registry and returns its ids.
pub fn register_ecore(reg: &mut MetaRegistry) -> EcoreIds {
    assert_eq!(reg.class_count(), 0, "Ecore must be the first package of a registry");
    let root = parse_tree(ECORE_ECORE).expect("bundled Ecore.ecore");
    let p = reg.add_package(
        root.attr("name").unwrap(),
        root.attr("nsURI"),
        root.attr("nsPrefix"),
        None,
    );
    let mut by_name: HashMap<String, Classifier> = HashMap::new();
    for c in root.children("eClassifiers") {
        let name = c.attr("name").unwrap();
        let k = match c.attr("xsi:type").unwrap() {
            "ecore:EClass" => Classifier::Class(reg.add_class(
                p,
                name,
                c.attr("abstract") == Some("true"),
                c.attr("interface") == Some("true"),
            )),
            "ecore:EDataType" => Classifier::DataType(reg.add_data_type(
                p,
                name,
                c.attr("instanceClassName"),
                c.attr("serializable") != Some("false"),
            )),
            "ecore:EEnum" => {
                let lits = c
                    .children("eLiterals")
                    .map(|l| EnumLiteral {
                        name: l.attr("name").unwrap().into(),
                        value: l.attr("value").and_then(|v| v.parse().ok()).unwrap_or(0),
                        literal: l.attr("literal").or(l.attr("name")).unwrap().into(),
                    })
                    .collect();
                Classifier::Enum(reg.add_enum(p, name, lits))
            }
            t => panic!("unexpected classifier type {t}"),
        };
        by_name.insert(name.to_string(), k);
    }
    let lookup = |r: &str| -> Classifier {
        let n = r.strip_prefix("#//").unwrap_or(r);
        by_name[n]
    };
    let mut opposites: Vec<(FeatureId, String)> = Vec::new();
    for c in root.children("eClassifiers") {
        let Classifier::Class(cid) = by_name[c.attr("name").unwrap()] else {
            continue;
        };
        if let Some(s) = c.attr("eSuperTypes") {
            for t in s.split_whitespace() {
                if let Classifier::Class(sup) = lookup(t) {
                    reg.class_mut(cid).supers.push(sup);
                }
            }
        }
        for f in c.children("eStructuralFeatures") {
            let kind = match f.attr("xsi:type").unwrap() {
                "ecore:EAttribute" => FeatureKind::Attribute,
                _ => FeatureKind::Reference,
            };
            let ty = f
                .attr("eType")
                .or_else(|| f.children("eGenericType").next().and_then(|g| g.attr("eClassifier")))
                .map(lookup)
                .unwrap();
            let fid = reg.add_feature(cid, f.attr("name").unwrap(), kind, ty);
            let b = |k: &str, d: bool| f.attr(k).map(|v| v == "true").unwrap_or(d);
            let fi = reg.feature_mut(fid);
            fi.lower = f.attr("lowerBound").and_then(|v| v.parse().ok()).unwrap_or(0);
            fi.upper = f.attr("upperBound").and_then(|v| v.parse().ok()).unwrap_or(1);
            fi.ordered = b("ordered", true);
            fi.unique = b("unique", true);
            fi.changeable = b("changeable", true);
            fi.volatile = b("volatile", false);
            fi.transient = b("transient", false);
            fi.unsettable = b("unsettable", false);
            fi.derived = b("derived", false);
            fi.id = b("iD", false);
            fi.containment = b("containment", false);
            fi.resolve_proxies = b("resolveProxies", true);
            fi.default_literal = f.attr("defaultValueLiteral").map(Into::into);
            if let Some(o) = f.attr("eOpposite") {
                opposites.push((fid, o.trim_start_matches("#//").to_string()));
            }
        }
    }
    for (f, o) in opposites {
        let (c, n) = o.split_once('/').unwrap();
        let Classifier::Class(c) = by_name[c] else { unreachable!() };
        let g = reg.class(c).features.iter().copied().find(|g| &*reg.feature(*g).name == n);
        reg.feature_mut(f).opposite = g;
    }
    reg.finish_classes().expect("Ecore metamodel");
    reg.register_ns_uri(p);
    let ids = EcoreIds::resolve(reg, p);
    reg.set_eobject_class(ids.eobject);
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecore_registry() {
        let mut reg = MetaRegistry::new();
        let e = register_ecore(&mut reg);
        let c = reg.class(e.eclass);
        assert_eq!(&*c.name, "EClass");
        let names: Vec<&str> = c.all_supers.iter().map(|s| &*reg.class(*s).name).collect();
        assert_eq!(names, ["EModelElement", "ENamedElement", "EClassifier"]);
        assert_eq!(reg.feature(c.all_features[0]).name.as_ref(), "eAnnotations");
        assert!(reg.feature(e.class_structural_features).containment);
        assert_eq!(reg.feature(e.class_structural_features).opposite, Some(e.feature_containing_class));
        assert!(reg.is_container(e.feature_containing_class));
        assert!(reg.is_subclass(e.eattribute, e.etyped_element));
        assert_eq!(reg.package_by_ns_uri(ECORE_NS_URI), Some(e.package));
    }
}

//! Registering metamodels: converting the `EPackage`s of loaded `.ecore` resources into
//! [`MetaRegistry`](qvto_model::MetaRegistry) entries, and linking registry entries with the Ecore
//! objects that define them (both ways), as EMF's dynamic packages are their own metadata.

use crate::ecore::ECORE_NS_URI;
use crate::graph::{Graph, ObjId};
use qvto_model::{
    ClassId, Classifier, Datum, EnumLiteral, FeatureKind, ModelStore, PackageId, ResourceId,
};

fn str_of(g: &Graph, o: ObjId, f: qvto_model::FeatureId) -> Option<String> {
    g.get(o, f).as_str().map(|s| s.to_string())
}

fn bool_of(g: &Graph, o: ObjId, f: qvto_model::FeatureId) -> bool {
    matches!(g.get(o, f), Datum::Bool(true))
}

fn int_of(g: &Graph, o: ObjId, f: qvto_model::FeatureId) -> i32 {
    g.get(o, f).as_i64().unwrap_or(0) as i32
}

fn objs(g: &Graph, o: ObjId, f: qvto_model::FeatureId) -> Vec<ObjId> {
    (0..g.len(o, f))
        .filter_map(|i| g.get_at(o, f, i).as_obj())
        .collect()
}

/// Links the built-in Ecore entries to the objects of a resource holding `Ecore.ecore`.
pub fn link_ecore(g: &mut Graph, r: ResourceId) {
    let e = g.ecore().clone();
    let Some(&pkg) = g.roots(r).first() else {
        return;
    };
    g.links_mut().package_obj.insert(e.package, pkg);
    g.links_mut().obj_package.insert(pkg, e.package);
    for c in objs(g, pkg, e.package_classifiers) {
        let Some(name) = str_of(g, c, e.named_element_name) else {
            continue;
        };
        let Some(k) = g.meta().classifier_by_name(e.package, &name) else {
            continue;
        };
        g.links_mut().classifier_obj.insert(k, c);
        g.links_mut().obj_classifier.insert(c, k);
        if let Classifier::Class(cid) = k {
            for f in objs(g, c, e.class_structural_features) {
                let Some(fname) = str_of(g, f, e.named_element_name) else {
                    continue;
                };
                let fid = g
                    .meta()
                    .class(cid)
                    .features
                    .iter()
                    .copied()
                    .find(|x| *g.meta().feature(*x).name == *fname);
                if let Some(fid) = fid {
                    g.links_mut().feature_obj.insert(fid, f);
                    g.links_mut().obj_feature.insert(f, fid);
                }
            }
        }
    }
}

/// Registers every `EPackage` (with subpackages) among the roots of the given resources, in one
/// batch, so the packages may refer to each other. Classifiers referenced from other resources
/// must already be registered (or be in the batch). Returns the new packages, top-level first.
pub fn register_resources(
    g: &mut Graph,
    resources: &[ResourceId],
) -> Result<Vec<PackageId>, String> {
    let e = g.ecore().clone();
    let mut packages: Vec<(ObjId, PackageId)> = Vec::new();
    let mut new_classes: Vec<(ObjId, ClassId)> = Vec::new();
    // pass 1: packages and classifiers
    fn add_pkg(
        g: &mut Graph,
        o: ObjId,
        sup: Option<PackageId>,
        out: &mut Vec<(ObjId, PackageId)>,
        classes: &mut Vec<(ObjId, ClassId)>,
    ) {
        let e = g.ecore().clone();
        if g.links().obj_package.contains_key(&o) {
            return;
        }
        let name = str_of(g, o, e.named_element_name).unwrap_or_default();
        let ns_uri = str_of(g, o, e.package_ns_uri);
        let ns_prefix = str_of(g, o, e.package_ns_prefix);
        let p = g
            .meta_mut()
            .add_package(&name, ns_uri.as_deref(), ns_prefix.as_deref(), sup);
        g.links_mut().package_obj.insert(p, o);
        g.links_mut().obj_package.insert(o, p);
        out.push((o, p));
        for c in objs(g, o, e.package_classifiers) {
            let cname = str_of(g, c, e.named_element_name).unwrap_or_default();
            let class = g.class_of(c);
            let k = if g.meta().is_subclass(class, e.eclass) {
                let (abs, itf) = (
                    bool_of(g, c, e.class_abstract),
                    bool_of(g, c, e.class_interface),
                );
                let cid = g.meta_mut().add_class(p, &cname, abs, itf);
                let icn = str_of(g, c, e.classifier_instance_class_name);
                g.meta_mut().class_mut(cid).instance_class_name = icn.map(Into::into);
                classes.push((c, cid));
                Classifier::Class(cid)
            } else if g.meta().is_subclass(class, e.eenum) {
                let lits = objs(g, c, e.enum_literals)
                    .into_iter()
                    .map(|l| {
                        let n = str_of(g, l, e.named_element_name).unwrap_or_default();
                        EnumLiteral {
                            value: int_of(g, l, e.literal_value),
                            literal: str_of(g, l, e.literal_literal)
                                .unwrap_or_else(|| n.clone())
                                .into(),
                            name: n.into(),
                        }
                    })
                    .collect();
                let id = g.meta_mut().add_enum(p, &cname, lits);
                g.meta_mut().enum_mut(id).serializable = bool_of(g, c, e.data_type_serializable);
                Classifier::Enum(id)
            } else {
                let icn = str_of(g, c, e.classifier_instance_class_name);
                let ser = bool_of(g, c, e.data_type_serializable);
                Classifier::DataType(g.meta_mut().add_data_type(p, &cname, icn.as_deref(), ser))
            };
            g.links_mut().classifier_obj.insert(k, c);
            g.links_mut().obj_classifier.insert(c, k);
        }
        for s in objs(g, o, e.package_subpackages) {
            add_pkg(g, s, Some(p), out, classes);
        }
    }
    for &r in resources {
        let roots: Vec<ObjId> = g.roots(r).to_vec();
        for o in roots {
            if g.meta().is_subclass(g.class_of(o), e.epackage) {
                add_pkg(g, o, None, &mut packages, &mut new_classes);
            }
        }
    }
    // pass 2: supertypes and features
    let mut errors = Vec::new();
    let mut proxies: std::collections::HashMap<(String, bool), Classifier> = std::collections::HashMap::new();
    let mut proxy_pkg: Option<PackageId> = None;
    let mut opposites = Vec::new();
    let mut keys = Vec::new();
    let eobject = Classifier::Class(e.eobject);
    let ejava_object = g
        .meta()
        .classifier_by_name(e.package, "EJavaObject")
        .expect("EJavaObject");
    for &(c, cid) in &new_classes {
        for s in objs(g, c, e.class_super_types) {
            match g.links().obj_classifier.get(&s).copied() {
                Some(Classifier::Class(sid)) => g.meta_mut().class_mut(cid).supers.push(sid),
                _ => errors.push(format!(
                    "unresolved supertype {} of {}",
                    g.proxy_uri(s).unwrap_or("?"),
                    g.meta().class(cid).name
                )),
            }
        }
        for f in objs(g, c, e.class_structural_features) {
            let fname = str_of(g, f, e.named_element_name).unwrap_or_default();
            let is_ref = g.meta().is_subclass(g.class_of(f), e.ereference);
            let kind = if is_ref {
                FeatureKind::Reference
            } else {
                FeatureKind::Attribute
            };
            let ty_obj = g.get(f, e.typed_type).as_obj();
            let ty = match ty_obj.and_then(|t| g.links().obj_classifier.get(&t).copied()) {
                Some(k) => k,
                None => {
                    errors.push(format!(
                        "unresolved type of {}.{fname}",
                        g.meta().class(cid).name
                    ));
                    match ty_obj {
                        // EMF keeps the proxy: a classifier without name (`null`), package,
                        // supertypes or features, one per proxy URI
                        Some(t) => proxy_classifier(g, t, is_ref, &mut proxies, &mut proxy_pkg),
                        None if is_ref => eobject,
                        None => ejava_object,
                    }
                }
            };
            let fid = g.meta_mut().add_feature(cid, &fname, kind, ty);
            let gt = g.get(f, e.typed_generic_type).as_obj();
            let type_parameter =
                gt.is_some_and(|gt| g.get(gt, e.generic_type_parameter).as_obj().is_some());
            let lower = int_of(g, f, e.typed_lower);
            let upper = int_of(g, f, e.typed_upper);
            let flags = [
                bool_of(g, f, e.typed_ordered),
                bool_of(g, f, e.typed_unique),
                bool_of(g, f, e.feature_changeable),
                bool_of(g, f, e.feature_volatile),
                bool_of(g, f, e.feature_transient),
                bool_of(g, f, e.feature_unsettable),
                bool_of(g, f, e.feature_derived),
            ];
            let id = !is_ref && bool_of(g, f, e.attribute_id);
            let containment = is_ref && bool_of(g, f, e.reference_containment);
            let resolve = !is_ref || bool_of(g, f, e.reference_resolve_proxies);
            let default = str_of(g, f, e.feature_default_literal);
            let fi = g.meta_mut().feature_mut(fid);
            fi.lower = lower;
            fi.upper = upper;
            [
                fi.ordered,
                fi.unique,
                fi.changeable,
                fi.volatile,
                fi.transient,
                fi.unsettable,
                fi.derived,
            ] = flags;
            fi.id = id;
            fi.containment = containment;
            fi.resolve_proxies = resolve;
            fi.default_literal = default.map(Into::into);
            fi.type_parameter = type_parameter;
            g.links_mut().feature_obj.insert(fid, f);
            g.links_mut().obj_feature.insert(f, fid);
            if is_ref {
                if let Some(o) = g.get(f, e.reference_opposite).as_obj() {
                    opposites.push((fid, o));
                }
                let k = objs(g, f, e.reference_keys);
                if !k.is_empty() {
                    keys.push((fid, k));
                }
            }
        }
    }
    // unresolved types of operations and parameters: the same placeholders (the operations
    // themselves are read later, `operations::meta_operations`)
    for &(c, _) in &new_classes {
        for op in objs(g, c, e.class_operations) {
            let mut typed = vec![op];
            typed.extend(objs(g, op, e.operation_parameters));
            for x in typed {
                if let Some(t) = g.get(x, e.typed_type).as_obj()
                    && !g.links().obj_classifier.contains_key(&t)
                    && g.is_proxy(t)
                {
                    let is_class = g.meta().is_subclass(g.class_of(t), e.eclass);
                    let _ = proxy_classifier(g, t, is_class, &mut proxies, &mut proxy_pkg);
                }
            }
        }
    }
    for (fid, o) in opposites {
        match g.links().obj_feature.get(&o) {
            Some(&of) => g.meta_mut().feature_mut(fid).opposite = Some(of),
            None => errors.push(format!(
                "unresolved opposite of {}",
                g.meta().feature(fid).name
            )),
        }
    }
    for (fid, k) in keys {
        let ks: Vec<_> = k
            .iter()
            .filter_map(|o| g.links().obj_feature.get(o).copied())
            .collect();
        g.meta_mut().feature_mut(fid).keys = ks;
    }
    g.meta_mut().finish_classes().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for &(_, p) in &packages {
        if g.meta().package(p).ns_uri.as_deref() != Some(ECORE_NS_URI) {
            g.meta_mut().register_ns_uri(p);
        }
        out.push(p);
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        // the packages are registered anyway; unresolved parts were defaulted as EMF would
        // leave proxies
        Err(errors.join("; "))
    }
}

/// The placeholder classifier of an unresolved proxy (EMF keeps the proxy: a classifier without
/// name (`null`), package, supertypes or features), one per proxy URI and kind; the proxy object
/// is linked to it.
fn proxy_classifier(
    g: &mut Graph,
    t: ObjId,
    is_class: bool,
    proxies: &mut std::collections::HashMap<(String, bool), Classifier>,
    proxy_pkg: &mut Option<PackageId>,
) -> Classifier {
    let uri = g.proxy_uri(t).unwrap_or("").to_string();
    let key = (uri, is_class);
    let k = match proxies.get(&key) {
        Some(&k) => k,
        None => {
            let pp = *proxy_pkg.get_or_insert_with(|| g.meta_mut().add_package("", None, None, None));
            let k = if is_class {
                Classifier::Class(g.meta_mut().add_class(pp, "null", false, false))
            } else {
                Classifier::DataType(g.meta_mut().add_data_type(pp, "null", None, true))
            };
            proxies.insert(key, k);
            k
        }
    };
    g.links_mut().obj_classifier.insert(t, k);
    k
}

/// The registry classifier for an Ecore classifier object, if registered.
pub fn classifier_of(g: &Graph, o: ObjId) -> Option<Classifier> {
    g.links().obj_classifier.get(&o).copied()
}

/// The `EClass` object of a registry class (for `eClass()`), if linked.
pub fn class_object(g: &Graph, c: ClassId) -> Option<ObjId> {
    g.links().classifier_obj.get(&Classifier::Class(c)).copied()
}

/// The `EAttribute`/`EReference` object of a registry feature, if linked.
pub fn feature_object(g: &Graph, f: qvto_model::FeatureId) -> Option<ObjId> {
    g.links().feature_obj.get(&f).copied()
}

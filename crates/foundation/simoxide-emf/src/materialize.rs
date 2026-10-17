//! Metamodel objects for registry entries built in code: the `EPackage`, `EClass`,
//! `EAttribute`, ... objects that `.ecore` loading would have produced, created from the
//! [`MetaRegistry`](qvto_model::MetaRegistry) and linked to its entries ([`MetaLinks`]), so that
//! `eClass()`, Ecore's reflective operations and `.ecore` files that refer to these packages
//! work for programmatic registries (e.g. a host's generated metamodel tables) as they do for
//! registered `.ecore` files.

use crate::graph::{Graph, MetaLinks, ObjId, ResourceKind};
use crate::resource_set::ResourceSet;
use qvto_model::{
    ClassId, Classifier, Datum, FeatureKind, MetaOperation, MetaRegistry, ModelStore, PackageId,
    ResourceId,
};

impl ResourceSet {
    /// Creates the Ecore objects of each package `top` (with its subpackages) in a new resource
    /// with the URI `uri` (the location other metamodels use to refer to it, e.g.
    /// `platform:/plugin/<bundle>/model/<file>.ecore`), links them to the registry entries, and
    /// adds the `EOperation`s of `ops` whose class is in the package trees. Types must be
    /// registry entries with objects: Ecore's (loaded on demand), packages materialized or
    /// loaded before, or one of these package trees. Returns the resources; errors name the
    /// types without objects (their features stay untyped).
    pub fn materialize(
        &mut self,
        tops: &[(PackageId, &str)],
        ops: &[MetaOperation],
    ) -> Result<Vec<ResourceId>, String> {
        self.ecore_resource();
        let meta = self.graph.meta().clone();
        let mut packages = Vec::new();
        let mut resources = Vec::new();
        for &(top, uri) in tops {
            let r = self.graph.create_resource_kind(uri, ResourceKind::Ecore);
            let start = packages.len();
            collect_packages(&meta, top, &mut packages);
            resources.extend(std::iter::repeat_n(r, packages.len() - start));
        }
        let g = &mut self.graph;
        let e = g.ecore().clone();
        let mut errors = Vec::new();
        // pass 1: packages and classifiers
        for (&p, &r) in packages.iter().zip(&resources) {
            let pi = meta.package(p);
            let po = new_obj(g, e.epackage);
            set_str(g, po, e.named_element_name, Some(&pi.name));
            set_str(g, po, e.package_ns_uri, pi.ns_uri.as_deref());
            set_str(g, po, e.package_ns_prefix, pi.ns_prefix.as_deref());
            match pi.super_package {
                Some(s) if s != p && packages.contains(&s) => {
                    let so = g.links().package_obj[&s];
                    add(g, so, e.package_subpackages, po);
                }
                _ => {
                    let _ = g.add_root(r, po, None);
                }
            }
            link(g.links_mut(), Link::Package(p), po);
            for &k in &pi.classifiers {
                let ko = match k {
                    Classifier::Class(c) => {
                        let ci = meta.class(c);
                        let o = new_obj(g, e.eclass);
                        set_bool(g, o, e.class_abstract, ci.is_abstract);
                        set_bool(g, o, e.class_interface, ci.is_interface);
                        set_str(
                            g,
                            o,
                            e.classifier_instance_class_name,
                            ci.instance_class_name.as_deref(),
                        );
                        o
                    }
                    Classifier::DataType(d) => {
                        let di = meta.data_type(d);
                        let o = new_obj(g, e.edata_type);
                        set_str(
                            g,
                            o,
                            e.classifier_instance_class_name,
                            di.instance_class_name.as_deref(),
                        );
                        if !di.serializable {
                            let _ = g.set(o, e.data_type_serializable, Datum::Bool(false));
                        }
                        o
                    }
                    Classifier::Enum(en) => {
                        let o = new_obj(g, e.eenum);
                        for l in &meta.enumeration(en).literals {
                            let lo = new_obj(g, e.eenum_literal);
                            set_str(g, lo, e.named_element_name, Some(&l.name));
                            if l.value != 0 {
                                let _ = g.set(lo, e.literal_value, Datum::Int(l.value));
                            }
                            if l.literal != l.name {
                                set_str(g, lo, e.literal_literal, Some(&l.literal));
                            }
                            add(g, o, e.enum_literals, lo);
                        }
                        o
                    }
                };
                set_str(g, ko, e.named_element_name, Some(meta.classifier_name(k)));
                add(g, po, e.package_classifiers, ko);
                link(g.links_mut(), Link::Classifier(k), ko);
            }
        }
        // pass 2: supertypes, features, operations
        let type_obj = |g: &Graph, k: Classifier, errors: &mut Vec<String>| -> Option<ObjId> {
            let o = g.links().classifier_obj.get(&k).copied();
            if o.is_none() {
                errors.push(format!("no object for type {}", meta.classifier_name(k)));
            }
            o
        };
        let mut opposites = Vec::new();
        for &p in &packages {
            for &k in &meta.package(p).classifiers {
                let Classifier::Class(c) = k else { continue };
                let co = g.links().classifier_obj[&k];
                for &s in &meta.class(c).supers {
                    if let Some(so) = type_obj(g, Classifier::Class(s), &mut errors) {
                        add(g, co, e.class_super_types, so);
                    }
                }
                for &f in &meta.class(c).features {
                    let fi = meta.feature(f);
                    let is_ref = fi.kind == FeatureKind::Reference;
                    let fo = new_obj(g, if is_ref { e.ereference } else { e.eattribute });
                    set_str(g, fo, e.named_element_name, Some(&fi.name));
                    add(g, co, e.class_structural_features, fo);
                    if !fi.type_parameter
                        && let Some(t) = type_obj(g, fi.ty, &mut errors)
                    {
                        let _ = g.set(fo, e.typed_type, Datum::Obj(t));
                    }
                    set_int(g, fo, e.typed_lower, fi.lower, 0);
                    set_int(g, fo, e.typed_upper, fi.upper, 1);
                    set_bool_d(g, fo, e.typed_ordered, fi.ordered, true);
                    set_bool_d(g, fo, e.typed_unique, fi.unique, true);
                    set_bool_d(g, fo, e.feature_changeable, fi.changeable, true);
                    set_bool(g, fo, e.feature_volatile, fi.volatile);
                    set_bool(g, fo, e.feature_transient, fi.transient);
                    set_bool(g, fo, e.feature_unsettable, fi.unsettable);
                    set_bool(g, fo, e.feature_derived, fi.derived);
                    set_str(
                        g,
                        fo,
                        e.feature_default_literal,
                        fi.default_literal.as_deref(),
                    );
                    if is_ref {
                        set_bool(g, fo, e.reference_containment, fi.containment);
                        set_bool_d(g, fo, e.reference_resolve_proxies, fi.resolve_proxies, true);
                        if let Some(o) = fi.opposite {
                            opposites.push((fo, o));
                        }
                    } else {
                        set_bool(g, fo, e.attribute_id, fi.id);
                    }
                    link(g.links_mut(), Link::Feature(f), fo);
                }
                for op in ops.iter().filter(|op| op.class == c) {
                    let oo = new_obj(g, e.eoperation);
                    set_str(g, oo, e.named_element_name, Some(&op.name));
                    add(g, co, e.class_operations, oo);
                    if let Some((t, m)) = op.result {
                        typed(g, oo, t, m, &e, &mut errors, &type_obj);
                    }
                    for (n, t, m) in &op.params {
                        let po = new_obj(g, e.eparameter);
                        set_str(g, po, e.named_element_name, Some(n));
                        add(g, oo, e.operation_parameters, po);
                        typed(g, po, *t, *m, &e, &mut errors, &type_obj);
                    }
                }
            }
        }
        for (fo, o) in opposites {
            match g.links().feature_obj.get(&o).copied() {
                Some(oo) => {
                    let _ = g.set(fo, e.reference_opposite, Datum::Obj(oo));
                }
                None => errors.push(format!("no object for opposite {}", meta.feature(o).name)),
            }
        }
        resources.dedup();
        if errors.is_empty() {
            Ok(resources)
        } else {
            Err(errors.join("; "))
        }
    }
}

fn collect_packages(meta: &MetaRegistry, p: PackageId, out: &mut Vec<PackageId>) {
    out.push(p);
    for &s in &meta.package(p).subpackages {
        collect_packages(meta, s, out);
    }
}

enum Link {
    Package(PackageId),
    Classifier(Classifier),
    Feature(qvto_model::FeatureId),
}

fn link(l: &mut MetaLinks, what: Link, o: ObjId) {
    match what {
        Link::Package(p) => {
            l.package_obj.insert(p, o);
            l.obj_package.insert(o, p);
        }
        Link::Classifier(k) => {
            l.classifier_obj.insert(k, o);
            l.obj_classifier.insert(o, k);
        }
        Link::Feature(f) => {
            l.feature_obj.insert(f, o);
            l.obj_feature.insert(o, f);
        }
    }
}

fn new_obj(g: &mut Graph, c: ClassId) -> ObjId {
    g.create(c).expect("Ecore class")
}

fn add(g: &mut Graph, o: ObjId, f: qvto_model::FeatureId, v: ObjId) {
    let _ = g.add(o, f, Datum::Obj(v), None);
}

fn set_str(g: &mut Graph, o: ObjId, f: qvto_model::FeatureId, v: Option<&str>) {
    if let Some(v) = v {
        let _ = g.set(o, f, Datum::Str(v));
    }
}

fn set_bool(g: &mut Graph, o: ObjId, f: qvto_model::FeatureId, v: bool) {
    set_bool_d(g, o, f, v, false);
}

fn set_bool_d(g: &mut Graph, o: ObjId, f: qvto_model::FeatureId, v: bool, default: bool) {
    if v != default {
        let _ = g.set(o, f, Datum::Bool(v));
    }
}

fn set_int(g: &mut Graph, o: ObjId, f: qvto_model::FeatureId, v: i32, default: i32) {
    if v != default {
        let _ = g.set(o, f, Datum::Int(v));
    }
}

/// Looks up (or makes) the metamodel object of a classifier.
type TypeObjFn<'a> = dyn Fn(&Graph, Classifier, &mut Vec<String>) -> Option<ObjId> + 'a;

fn typed(
    g: &mut Graph,
    o: ObjId,
    t: Classifier,
    m: qvto_model::MetaMultiplicity,
    e: &crate::ecore::EcoreIds,
    errors: &mut Vec<String>,
    type_obj: &TypeObjFn<'_>,
) {
    if let Some(to) = type_obj(g, t, errors) {
        let _ = g.set(o, e.typed_type, Datum::Obj(to));
    }
    if m.many {
        let _ = g.set(o, e.typed_upper, Datum::Int(-1));
    }
    set_bool_d(g, o, e.typed_ordered, m.ordered, true);
    set_bool_d(g, o, e.typed_unique, m.unique, true);
}

/// A name-based description of packages (with subpackages) of a registry, one line per package,
/// classifier, literal and feature with all their properties, for comparing registries built in
/// different ways (ids differ, names do not).
pub fn describe_packages(meta: &MetaRegistry, tops: &[PackageId]) -> Vec<String> {
    let mut out = Vec::new();
    let mut packages = Vec::new();
    for &t in tops {
        collect_packages(meta, t, &mut packages);
    }
    let cname = |k: Classifier| {
        format!(
            "{}::{}",
            meta.package_path(meta.classifier_package(k), "."),
            meta.classifier_name(k)
        )
    };
    let fname = |f: qvto_model::FeatureId| {
        let fi = meta.feature(f);
        format!("{}.{}", cname(Classifier::Class(fi.owner)), fi.name)
    };
    for p in packages {
        let pi = meta.package(p);
        out.push(format!(
            "package {} uri={:?} prefix={:?} subpackages={:?}",
            meta.package_path(p, "."),
            pi.ns_uri,
            pi.ns_prefix,
            pi.subpackages
                .iter()
                .map(|s| &*meta.package(*s).name)
                .collect::<Vec<_>>()
        ));
        for &k in &pi.classifiers {
            match k {
                Classifier::Class(c) => {
                    let ci = meta.class(c);
                    out.push(format!(
                        "class {} abstract={} interface={} icn={:?} supers={:?} all_features={:?} id={:?}",
                        cname(k),
                        ci.is_abstract,
                        ci.is_interface,
                        ci.instance_class_name,
                        ci.supers.iter().map(|s| cname(Classifier::Class(*s))).collect::<Vec<_>>(),
                        ci.all_features.iter().map(|f| fname(*f)).collect::<Vec<_>>(),
                        ci.id_attribute.map(fname),
                    ));
                    for &f in &ci.features {
                        let fi = meta.feature(f);
                        out.push(format!(
                            "  feature {} {:?} ty={} [{}..{}] ordered={} unique={} changeable={} volatile={} transient={} unsettable={} derived={} id={} containment={} resolve={} opposite={:?} default={:?} tparam={}",
                            fname(f),
                            fi.kind,
                            cname(fi.ty),
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
                            fi.opposite.map(fname),
                            fi.default_literal,
                            fi.type_parameter,
                        ));
                    }
                }
                Classifier::DataType(d) => {
                    let di = meta.data_type(d);
                    out.push(format!(
                        "datatype {} icn={:?} kind={:?} serializable={}",
                        cname(k),
                        di.instance_class_name,
                        di.kind,
                        di.serializable
                    ));
                }
                Classifier::Enum(en) => {
                    let ei = meta.enumeration(en);
                    out.push(format!(
                        "enum {} literals={:?}",
                        cname(k),
                        ei.literals
                            .iter()
                            .map(|l| format!("{}={}/{}", l.name, l.value, l.literal))
                            .collect::<Vec<_>>()
                    ));
                }
            }
        }
    }
    out
}

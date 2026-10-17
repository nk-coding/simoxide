//! The `EOperation`s of the registered classes, read from the metamodels' `.ecore` objects (the
//! registry keeps no operations): what a host hands to the analyser
//! (`qvto_sema::compile_with_operations`) so that calls like `pkg.getEClassifier(n)` resolve.

use crate::graph::Graph;
use qvto_model::{
    ClassId, Classifier, Datum, FeatureId, MetaDerivation, MetaMultiplicity, MetaOperation,
    ModelStore,
};

/// The annotation source of OCL bodies and derivations that Eclipse QVTo evaluates
/// (`OCLAnnotationSupport.OCL_SOURCE_URI`).
pub const OCL_ANNOTATION_SOURCE: &str = "http://www.eclipse.org/2007/OCL";

/// The `key` detail of the element's first annotation with the source (`getEAnnotation`).
fn annotation_detail(g: &Graph, element: crate::ObjId, source: &str, key: &str) -> Option<String> {
    let e = g.ecore();
    for i in 0..g.len(element, e.model_element_annotations) {
        let Datum::Obj(a) = g.get_at(element, e.model_element_annotations, i) else {
            continue;
        };
        if !matches!(g.get(a, e.annotation_source), Datum::Str(s) if s == source) {
            continue;
        }
        for j in 0..g.len(a, e.annotation_details) {
            let Datum::Obj(d) = g.get_at(a, e.annotation_details, j) else {
                continue;
            };
            if matches!(g.get(d, e.entry_key), Datum::Str(k) if k == key) {
                return match g.get(d, e.entry_value) {
                    Datum::Str(v) => Some(v.to_string()),
                    _ => None,
                };
            }
        }
        return None;
    }
    None
}

/// The OCL derivations of the features of dynamic classes (no instance class).
pub fn meta_derivations(g: &Graph) -> Vec<MetaDerivation> {
    let mut out = Vec::new();
    let classes: Vec<ClassId> = g.meta().classes().collect();
    for c in classes {
        if g.meta().class(c).instance_class_name.is_some() {
            continue;
        }
        let feats: Vec<FeatureId> = g.meta().class(c).features.clone();
        for f in feats {
            let Some(fo) = crate::register::feature_object(g, f) else {
                continue;
            };
            if let Some(expr) = annotation_detail(g, fo, OCL_ANNOTATION_SOURCE, "derive") {
                out.push(MetaDerivation { feature: f, expr });
            }
        }
    }
    out
}

/// The operations of every registered class that has a linked `EClass` object, in registry
/// class order and `eOperations` order per class. Operations with a parameter of a type the
/// registry does not know are left out.
pub fn meta_operations(g: &Graph) -> Vec<MetaOperation> {
    let e = g.ecore().clone();
    let typed = |o| -> Option<(Classifier, MetaMultiplicity)> {
        let ty = match g.get(o, e.typed_type) {
            Datum::Obj(t) => crate::register::classifier_of(g, t),
            _ => None,
        };
        let upper = match g.get(o, e.typed_upper) {
            Datum::Int(i) => i,
            _ => 1,
        };
        let ordered = !matches!(g.get(o, e.typed_ordered), Datum::Bool(false));
        let unique = !matches!(g.get(o, e.typed_unique), Datum::Bool(false));
        ty.map(|c| {
            (
                c,
                MetaMultiplicity {
                    many: !(0..=1).contains(&upper),
                    ordered,
                    unique,
                },
            )
        })
    };
    let mut out = Vec::new();
    let classes: Vec<ClassId> = g.meta().classes().collect();
    for c in classes {
        let Some(co) = crate::register::class_object(g, c) else {
            continue;
        };
        for i in 0..g.len(co, e.class_operations) {
            let Datum::Obj(op) = g.get_at(co, e.class_operations, i) else {
                continue;
            };
            let name = match g.get(op, e.named_element_name) {
                Datum::Str(s) => s.to_string(),
                _ => continue,
            };
            let result = typed(op);
            let mut params = Vec::new();
            let mut ok = true;
            for j in 0..g.len(op, e.operation_parameters) {
                let Datum::Obj(p) = g.get_at(op, e.operation_parameters, j) else {
                    continue;
                };
                let pname = match g.get(p, e.named_element_name) {
                    Datum::Str(s) => s.to_string(),
                    _ => String::new(),
                };
                match typed(p) {
                    Some((c, m)) => params.push((pname, c, m)),
                    None => ok = false,
                }
            }
            if ok {
                let body = if g.meta().class(c).instance_class_name.is_none() {
                    annotation_detail(g, op, OCL_ANNOTATION_SOURCE, "body")
                } else {
                    None
                };
                out.push(MetaOperation {
                    class: c,
                    name,
                    params,
                    result,
                    body,
                });
            }
        }
    }
    out
}

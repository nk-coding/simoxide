//! The `EOperation`s of the registered classes, read from the metamodels' `.ecore` objects (the
//! registry keeps no operations): what a host hands to the analyser
//! (`qvto_sema::compile_with_operations`) so that calls like `pkg.getEClassifier(n)` resolve.

use crate::graph::Graph;
use qvto_model::{ClassId, Classifier, Datum, MetaMultiplicity, MetaOperation, ModelStore};

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
                out.push(MetaOperation {
                    class: c,
                    name,
                    params,
                    result,
                });
            }
        }
    }
    out
}

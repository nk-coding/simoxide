//! Behaviour of Ecore's generated classes that differs from dynamic objects, for instances of
//! Ecore's own classes (the objects of `.ecore` models):
//!
//! - `ETypedElement.eType` is a view of `eGenericType` (setting it creates an `EGenericType`
//!   child); `eSuperTypes` and `eExceptions` are views of `eGenericSuperTypes` and
//!   `eGenericExceptions` (`EClassImpl`, `EOperationImpl`, `ETypedElementImpl`);
//! - the `isSet` rules of these pairs, of `instanceClassName` / `instanceTypeName`
//!   (`EClassifierImpl`) and of `EEnumLiteral.literal` (which reads as the name when unset);
//! - Ecore's derived features (`eAllSuperTypes`, `eAllStructuralFeatures`, `eAttributes`,
//!   `eReferenceType`, `many`, ...), computed from the model objects.
//!
//! Ported from EMF's `org.eclipse.emf.ecore.impl` (EPL-2.0, see `LICENSES`).

use crate::ecore::EcoreIds;
use crate::graph::{Graph, ObjId, Slot, Value};
use qvto_model::{ClassId, Datum, FeatureId, MetaRegistry, ModelError, ModelResult};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub(crate) enum Hook {
    None,
    EType,
    GenericType,
    SuperTypes,
    GenericSuperTypes,
    Exceptions,
    GenericExceptions,
    InstanceClassName,
    InstanceTypeName,
    LiteralLiteral,
    Derived,
}

pub(crate) fn hook_table(meta: &MetaRegistry, e: &EcoreIds) -> Vec<Hook> {
    let mut t = vec![Hook::None; e.feature_end as usize];
    for (i, h) in t.iter_mut().enumerate().skip(e.feature_start as usize) {
        if meta.feature(FeatureId(i as u32)).derived {
            *h = Hook::Derived;
        }
    }
    let mut set = |f: FeatureId, h: Hook| t[f.index()] = h;
    set(e.typed_type, Hook::EType);
    set(e.typed_generic_type, Hook::GenericType);
    set(e.class_super_types, Hook::SuperTypes);
    set(e.class_generic_super_types, Hook::GenericSuperTypes);
    set(e.operation_exceptions, Hook::Exceptions);
    set(e.operation_generic_exceptions, Hook::GenericExceptions);
    set(e.classifier_instance_class_name, Hook::InstanceClassName);
    set(e.classifier_instance_type_name, Hook::InstanceTypeName);
    set(e.literal_literal, Hook::LiteralLiteral);
    t
}

fn is_a(g: &Graph, x: ObjId, c: ClassId) -> bool {
    g.meta.is_subclass(g.objs[x.index()].class, c)
}

/// The generic list behind a view feature.
fn generic_list(g: &Graph, h: Hook) -> FeatureId {
    match h {
        Hook::SuperTypes => g.ecore.class_generic_super_types,
        _ => g.ecore.operation_generic_exceptions,
    }
}

/// `EGenericTypeImpl.getERawType`: the classifier, else the erasure of the type parameter.
pub(crate) fn raw_type(g: &Graph, gt: ObjId) -> Option<ObjId> {
    let e = &g.ecore;
    if let Some(c) = g.single_ref(gt, e.generic_classifier) {
        return Some(c);
    }
    let tp = g.single_ref(gt, e.generic_type_parameter)?;
    let container = g.objs[gt.index()].container.map(|(p, _)| p);
    let need_class = container.is_some_and(|p| is_a(g, p, e.eclass) || is_a(g, p, e.ereference));
    let need_data = !need_class && container.is_some_and(|p| is_a(g, p, e.eattribute));
    for &b in g.refs(tp, e.type_parameter_bounds) {
        if let Some(r) = raw_type(g, b) {
            let ok = if need_class {
                is_a(g, r, e.eclass)
            } else if need_data {
                is_a(g, r, e.edata_type)
            } else {
                true
            };
            if ok {
                return Some(r);
            }
        }
    }
    // EcorePackage.Literals.EOBJECT / EJAVA_OBJECT (objects of the Ecore resource, if loaded)
    let fallback = if need_class {
        qvto_model::Classifier::Class(e.eobject)
    } else {
        g.meta.classifier_by_name(e.package, "EJavaObject")?
    };
    g.links.classifier_obj.get(&fallback).copied()
}

fn is_simple_generic(g: &Graph, gt: ObjId) -> bool {
    g.single_ref(gt, g.ecore.generic_type_parameter).is_none()
        && g.refs(gt, g.ecore.generic_type_arguments).is_empty()
}

fn e_type(g: &Graph, o: ObjId) -> Option<ObjId> {
    raw_type(g, g.single_ref(o, g.ecore.typed_generic_type)?)
}

fn view(g: &Graph, o: ObjId, h: Hook) -> Vec<ObjId> {
    g.refs(o, generic_list(g, h))
        .iter()
        .filter_map(|&gt| raw_type(g, gt))
        .collect()
}

fn str_slot(g: &Graph, o: ObjId, f: FeatureId) -> Option<&str> {
    match g.slot(o, f) {
        Some(Slot::Data(Value::Str(s))) => Some(s),
        _ => None,
    }
}

// ---- derived features of EClass ---------------------------------------------------------------

fn supers(g: &Graph, c: ObjId) -> Vec<ObjId> {
    view(g, c, Hook::SuperTypes)
        .into_iter()
        .filter(|&s| is_a(g, s, g.ecore.eclass))
        .collect()
}

/// Appends `xs` to `out` without duplicates (a set of the elements for long lists).
fn extend_unique(out: &mut Vec<ObjId>, seen: &mut std::collections::HashSet<ObjId>, xs: &[ObjId]) {
    for &x in xs {
        if out.len() < 32 {
            if !out.contains(&x) {
                out.push(x);
            }
        } else {
            if seen.len() < out.len() {
                seen.extend(out.iter().copied());
            }
            if seen.insert(x) {
                out.push(x);
            }
        }
    }
}

/// `eAllSuperTypes`: for each supertype its `eAllSuperTypes` (cached), then itself.
fn all_supers(g: &Graph, c: ObjId) -> Vec<ObjId> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for s in supers(g, c) {
        if let Some(v) = derived(g, s, g.ecore.class_all_super_types) {
            extend_unique(&mut out, &mut seen, &v);
        }
        extend_unique(&mut out, &mut seen, &[s]);
    }
    out
}

/// For each supertype its value of the derived list `f` (cached; the supertypes' local lists in
/// `eAllSuperTypes` order), then `c`'s local list, without duplicates.
fn accumulate(
    g: &Graph,
    c: ObjId,
    f: FeatureId,
    local: &dyn Fn(ObjId) -> Vec<ObjId>,
) -> Vec<ObjId> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for s in supers(g, c) {
        if let Some(v) = derived(g, s, f) {
            extend_unique(&mut out, &mut seen, &v);
        }
    }
    extend_unique(&mut out, &mut seen, &local(c));
    out
}

fn local_of(g: &Graph, c: ObjId, class: ClassId) -> Vec<ObjId> {
    g.refs(c, g.ecore.class_structural_features)
        .iter()
        .copied()
        .filter(|&f| is_a(g, f, class))
        .collect()
}

fn is_containment(g: &Graph, f: ObjId) -> bool {
    matches!(
        g.op_get(f, g.ecore.reference_containment),
        Datum::Bool(true)
    )
}

/// The value of a derived list of Ecore (`eAllStructuralFeatures`, ...), cached until an
/// object of Ecore's classes changes.
pub(crate) fn derived(g: &Graph, o: ObjId, f: FeatureId) -> Option<std::sync::Arc<[ObjId]>> {
    if let Some((at, v)) = g.derived_cache.borrow().get(&(o, f))
        && *at == g.ecore_gen
    {
        return Some(v.clone());
    }
    if !is_derived_list(g, f) {
        return None;
    }
    // an empty placeholder while computing: a cycle of supertypes contributes nothing
    g.derived_cache
        .borrow_mut()
        .insert((o, f), (g.ecore_gen, std::sync::Arc::from(Vec::new())));
    let v: std::sync::Arc<[ObjId]> = derived_list(g, o, f)?.into();
    g.derived_cache
        .borrow_mut()
        .insert((o, f), (g.ecore_gen, v.clone()));
    Some(v)
}

fn is_derived_list(g: &Graph, f: FeatureId) -> bool {
    let e = &g.ecore;
    [
        e.class_all_super_types,
        e.class_all_structural_features,
        e.class_attributes,
        e.class_references,
        e.class_all_attributes,
        e.class_all_references,
        e.class_all_containments,
        e.class_all_operations,
        e.class_all_generic_super_types,
    ]
    .contains(&f)
}

fn derived_list(g: &Graph, o: ObjId, f: FeatureId) -> Option<Vec<ObjId>> {
    let e = &g.ecore;
    Some(if f == e.class_all_super_types {
        all_supers(g, o)
    } else if f == e.class_all_structural_features {
        accumulate(g, o, f, &|x| {
            g.refs(x, e.class_structural_features).to_vec()
        })
    } else if f == e.class_attributes {
        local_of(g, o, e.eattribute)
    } else if f == e.class_references {
        local_of(g, o, e.ereference)
    } else if f == e.class_all_attributes {
        accumulate(g, o, f, &|x| local_of(g, x, e.eattribute))
    } else if f == e.class_all_references {
        accumulate(g, o, f, &|x| local_of(g, x, e.ereference))
    } else if f == e.class_all_containments {
        derived(g, o, e.class_all_references)?
            .iter()
            .copied()
            .filter(|&r| is_containment(g, r))
            .collect()
    } else if f == e.class_all_operations {
        accumulate(g, o, f, &|x| g.refs(x, e.class_operations).to_vec())
    } else if f == e.class_all_generic_super_types {
        accumulate(g, o, f, &|x| {
            g.refs(x, e.class_generic_super_types).to_vec()
        })
    } else {
        return None;
    })
}

fn derived_single(g: &Graph, o: ObjId, f: FeatureId) -> Datum<'_, ObjId> {
    let e = &g.ecore;
    let obj = |x: Option<ObjId>| x.map_or(Datum::Null, Datum::Obj);
    if f == e.typed_many {
        let upper = g.op_get(o, e.typed_upper).as_i64().unwrap_or(1);
        Datum::Bool(upper > 1 || upper == -1)
    } else if f == e.typed_required {
        Datum::Bool(g.op_get(o, e.typed_lower).as_i64().unwrap_or(0) >= 1)
    } else if f == e.reference_container {
        let opp = g.single_ref(o, e.reference_opposite);
        Datum::Bool(opp.is_some_and(|x| is_containment(g, x)))
    } else if f == e.reference_type {
        obj(e_type(g, o).filter(|&t| is_a(g, t, e.eclass)))
    } else if f == e.attribute_type {
        obj(e_type(g, o).filter(|&t| is_a(g, t, e.edata_type)))
    } else if f == e.generic_raw_type {
        obj(raw_type(g, o))
    } else if f == e.class_id_attribute {
        obj(derived(g, o, e.class_all_attributes).and_then(|v| {
            v.iter()
                .copied()
                .find(|&a| matches!(g.op_get(a, e.attribute_id), Datum::Bool(true)))
        }))
    } else {
        Datum::Null
    }
}

// ---- dispatch ---------------------------------------------------------------------------------

pub(crate) fn get(g: &Graph, o: ObjId, f: FeatureId, h: Hook) -> Option<Datum<'_, ObjId>> {
    match h {
        Hook::EType => Some(e_type(g, o).map_or(Datum::Null, Datum::Obj)),
        Hook::LiteralLiteral => Some(match str_slot(g, o, f) {
            Some(s) => Datum::Str(s),
            None => match str_slot(g, o, g.ecore.named_element_name) {
                Some(n) => Datum::Str(n),
                None => Datum::Null,
            },
        }),
        Hook::Derived if !g.meta.feature(f).is_many() => Some(derived_single(g, o, f)),
        _ => None,
    }
}

pub(crate) fn len(g: &Graph, o: ObjId, f: FeatureId, h: Hook) -> Option<usize> {
    match h {
        Hook::SuperTypes | Hook::Exceptions => Some(g.refs(o, generic_list(g, h)).len()),
        Hook::Derived => derived(g, o, f).map(|v| v.len()),
        _ => None,
    }
}

pub(crate) fn get_at(
    g: &Graph,
    o: ObjId,
    f: FeatureId,
    h: Hook,
    i: usize,
) -> Option<Datum<'_, ObjId>> {
    let from = |v: std::sync::Arc<[ObjId]>| v.get(i).map_or(Datum::Null, |x| Datum::Obj(*x));
    match h {
        Hook::SuperTypes | Hook::Exceptions => {
            let gt = *g.refs(o, generic_list(g, h)).get(i)?;
            Some(raw_type(g, gt).map_or(Datum::Null, Datum::Obj))
        }
        Hook::Derived => derived(g, o, f).map(from),
        _ => None,
    }
}

pub(crate) fn is_set(g: &Graph, o: ObjId, f: FeatureId, h: Hook) -> Option<bool> {
    let e = &g.ecore;
    let generic_set = |list: FeatureId| g.refs(o, list).iter().any(|&gt| !is_simple_generic(g, gt));
    Some(match h {
        Hook::EType => {
            let gt = g.single_ref(o, e.typed_generic_type);
            gt.is_some_and(|gt| raw_type(g, gt).is_some() && is_simple_generic(g, gt))
        }
        Hook::GenericType => {
            let gt = g.single_ref(o, e.typed_generic_type);
            gt.is_some() && !is_set(g, o, e.typed_type, Hook::EType).unwrap()
        }
        Hook::SuperTypes | Hook::Exceptions => {
            let list = generic_list(g, h);
            !g.refs(o, list).is_empty() && !generic_set(list)
        }
        Hook::GenericSuperTypes => generic_set(e.class_generic_super_types),
        Hook::GenericExceptions => generic_set(e.operation_generic_exceptions),
        Hook::InstanceClassName => {
            let c = str_slot(g, o, e.classifier_instance_class_name);
            c.is_some() && c == str_slot(g, o, e.classifier_instance_type_name)
        }
        Hook::InstanceTypeName => {
            let t = str_slot(g, o, e.classifier_instance_type_name);
            t.is_some() && t != str_slot(g, o, e.classifier_instance_class_name)
        }
        Hook::LiteralLiteral => str_slot(g, o, f).is_some(),
        Hook::Derived => match derived(g, o, f) {
            Some(v) => !v.is_empty(),
            None => !derived_single(g, o, f).is_null(),
        },
        Hook::None => return None,
    })
}

/// List operations on Ecore's derived lists (`UnmodifiableEList`).
fn read_only(f: FeatureId) -> ModelResult<()> {
    Err(ModelError::Unsupported(format!(
        "UnsupportedOperationException: derived feature {f:?}"
    )))
}

/// `EClassImpl.setInstanceTypeName`'s derivation of the class name.
fn class_name_of_type_name(t: &str) -> String {
    const PRIMS: [&str; 8] = [
        "boolean", "byte", "char", "double", "float", "int", "long", "short",
    ];
    if let Some(i) = t.find('<') {
        let mut c = t[..i].to_string();
        if !(t.contains('.') || PRIMS.contains(&c.as_str())) {
            c = "java.lang.Object".into();
        }
        if let Some(end) = t.rfind('>') {
            c.push_str(&t[end + 1..]);
        }
        c
    } else if !t.contains('.') {
        let (base, dims) = match t.find('[') {
            Some(i) => (&t[..i], &t[i..]),
            None => (t, ""),
        };
        if PRIMS.contains(&base) {
            t.to_string()
        } else {
            format!("java.lang.Object{dims}")
        }
    } else {
        t.to_string()
    }
}

/// A new `EGenericType` whose classifier is `c`.
fn wrap(g: &mut Graph, c: ObjId) -> ObjId {
    let gt = g.new_object(g.ecore.egeneric_type);
    let cf = g.ecore.generic_classifier;
    g.set_ref_single(gt, cf, Some(c));
    gt
}

pub(crate) fn set(
    g: &mut Graph,
    o: ObjId,
    f: FeatureId,
    h: Hook,
    v: Datum<'_, ObjId>,
) -> Option<ModelResult<()>> {
    let e = g.ecore.clone();
    Some(match h {
        Hook::EType => {
            if let Err(err) = g.check_value(f, &v) {
                return Some(Err(err));
            }
            let gt = v.as_obj().map(|c| wrap(g, c));
            g.set_ref_single(o, e.typed_generic_type, gt);
            Ok(())
        }
        Hook::InstanceClassName | Hook::InstanceTypeName => {
            let s = match v {
                Datum::Str(s) => Some(s.to_string()),
                Datum::Null => None,
                _ => {
                    return Some(Err(ModelError::TypeMismatch {
                        feature: f,
                        found: format!("{v:?}"),
                    }));
                }
            };
            let (cn, tn) = match (h, s) {
                (_, None) => (None, None),
                (Hook::InstanceClassName, Some(s)) => (Some(s.clone()), Some(s)),
                (_, Some(t)) => (Some(class_name_of_type_name(&t)), Some(t)),
            };
            let to_slot =
                |x: Option<String>| x.map_or(Slot::Unset, |s| Slot::Data(Value::Str(s.into())));
            let ci = g.slot_index(o, e.classifier_instance_class_name).unwrap();
            let ti = g.slot_index(o, e.classifier_instance_type_name).unwrap();
            g.touch(o);
            g.objs[o.index()].slots[ci] = to_slot(cn);
            g.objs[o.index()].slots[ti] = to_slot(tn);
            Ok(())
        }
        Hook::Derived => read_only(f),
        _ => return None,
    })
}

pub(crate) fn unset(g: &mut Graph, o: ObjId, f: FeatureId, h: Hook) -> Option<ModelResult<()>> {
    match h {
        Hook::EType | Hook::InstanceClassName | Hook::InstanceTypeName => {
            set(g, o, f, h, Datum::Null)
        }
        Hook::SuperTypes | Hook::Exceptions => Some(g.op_unset(o, generic_list(g, h))),
        Hook::Derived => Some(read_only(f)),
        _ => None,
    }
}

/// Returns `true` if handled.
pub(crate) fn add_unique(
    g: &mut Graph,
    o: ObjId,
    _f: FeatureId,
    h: Hook,
    v: Datum<'_, ObjId>,
    at: Option<usize>,
) -> bool {
    match (h, v) {
        (Hook::SuperTypes | Hook::Exceptions, Datum::Obj(c)) => {
            let gt = wrap(g, c);
            let list = generic_list(g, h);
            g.add_unique(o, list, Datum::Obj(gt), at);
            true
        }
        _ => false,
    }
}

pub(crate) fn remove_at(
    g: &mut Graph,
    o: ObjId,
    f: FeatureId,
    h: Hook,
    i: usize,
) -> Option<ModelResult<()>> {
    match h {
        Hook::SuperTypes | Hook::Exceptions => Some(g.op_remove_at(o, generic_list(g, h), i)),
        Hook::Derived => Some(read_only(f)),
        _ => None,
    }
}

pub(crate) fn set_at(
    g: &mut Graph,
    o: ObjId,
    f: FeatureId,
    h: Hook,
    i: usize,
    v: Datum<'_, ObjId>,
) -> Option<ModelResult<()>> {
    match h {
        Hook::SuperTypes | Hook::Exceptions => {
            let gt = g.refs(o, generic_list(g, h))[i];
            if raw_type(g, gt) != v.as_obj() {
                let cf = g.ecore.generic_classifier;
                g.set_ref_single(gt, cf, v.as_obj());
            }
            Some(Ok(()))
        }
        Hook::Derived => Some(read_only(f)),
        _ => None,
    }
}

pub(crate) fn clear(g: &mut Graph, o: ObjId, f: FeatureId, h: Hook) -> Option<ModelResult<()>> {
    match h {
        Hook::SuperTypes | Hook::Exceptions => Some(g.op_clear(o, generic_list(g, h))),
        Hook::Derived => Some(read_only(f)),
        _ => None,
    }
}

pub(crate) fn move_to(
    g: &mut Graph,
    o: ObjId,
    f: FeatureId,
    h: Hook,
    to: usize,
    from: usize,
) -> Option<ModelResult<()>> {
    match h {
        Hook::SuperTypes | Hook::Exceptions => Some(g.op_move(o, generic_list(g, h), to, from)),
        Hook::Derived => Some(read_only(f)),
        _ => None,
    }
}

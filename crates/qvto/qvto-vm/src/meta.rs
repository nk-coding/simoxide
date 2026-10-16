//! Operations of metamodel classes (`EOperation`s, `Callee::Meta`): Ecore's own operations
//! (`EObject.eGet`, `EClass.getEStructuralFeature`, `EPackage.getEClassifier`, ...) as EMF's
//! generated code implements them, read and written through the model store; operations of
//! other metamodels through the host's native implementations
//! (`Natives::eoperation`, the counterpart of generated Java code).
//!
//! Eclipse calls an `EOperation` without OCL body through `EcoreEvaluationEnvironment
//! .callOperation`: on a `null`/`invalid` receiver the result is `invalid`, a Java exception of
//! the implementation makes the call `invalid` (logged), and a many-valued result is coerced
//! into the OCL collection of the operation's multiplicity.

use crate::error::VmResult;
use crate::vm::{ObjHandle, Vm};
use qvto_model::{Classifier, Datum, FeatureId, ModelStore};
use qvto_runtime::coll::{Coll, CollKind};
use qvto_runtime::{ObjRef, Value};
use qvto_sema::ir::MetaOpId;
use qvto_types::Type;
use std::rc::Rc;

const ECORE_NS: &str = "http://www.eclipse.org/emf/2002/Ecore";

impl<S: ModelStore> Vm<'_, S>
where
    S::Obj: ObjHandle,
{
    /// A call of a metamodel operation; the receiver at `at`, the arguments after it.
    pub(crate) fn call_meta(&mut self, m: MetaOpId, at: usize) -> VmResult<Value> {
        let mo = &self.prog.ir.meta_ops[m.index()];
        let src = self.regs[at].clone();
        if src.is_undefined() {
            return Ok(Value::Invalid);
        }
        let args: Vec<Value> = (0..mo.params.len())
            .map(|i| self.regs[at + 1 + i].clone())
            .collect();
        let meta = &self.prog.ir.meta;
        let class = meta.class(mo.class);
        let ns = meta
            .package(class.package)
            .ns_uri
            .as_deref()
            .unwrap_or("")
            .to_string();
        let class_name = class.name.to_string();
        let name = mo.name.clone();
        let r = if ns == ECORE_NS {
            self.ecore_operation(&class_name, &name, &src, &args)
        } else {
            None
        };
        let r = match r {
            Some(r) => r,
            None => match self.native_eoperation(&ns, &class_name, &name, &src, args)? {
                Some(v) => Ok(v),
                None => {
                    return Err(self.runtime_error(format!(
                        "metamodel operation '{name}' is not supported yet"
                    )));
                }
            },
        };
        Ok(match r {
            Ok(v) => self.coerce_meta_result(m, v),
            // a Java exception of the implementation: the call is invalid
            Err(msg) => self.swallow(msg),
        })
    }

    /// `coerceValue`: a collection result in the collection kind of the operation's type.
    fn coerce_meta_result(&self, m: MetaOpId, v: Value) -> Value {
        let rt = self.prog.ir.meta_ops[m.index()].result;
        let Type::Collection(kind, _) = self.prog.ir.types.get(rt) else {
            return v;
        };
        let items = match &v {
            Value::Coll(_) | Value::List(_) => crate::vm::Iter::of(&v).items.to_vec(),
            Value::Null => return Value::Null,
            _ => return v,
        };
        let k = match kind {
            qvto_types::CollKind::Set => CollKind::Set,
            qvto_types::CollKind::OrderedSet => CollKind::OrderedSet,
            qvto_types::CollKind::Bag => CollKind::Bag,
            _ => CollKind::Sequence,
        };
        match k {
            CollKind::Sequence => Value::Coll(Rc::new(Coll::Sequence(items))),
            k => Value::Coll(Rc::new(Coll::from_values(k, items, &self.env))),
        }
    }

    fn host(&self, v: &Value) -> Option<S::Obj> {
        match v {
            Value::Obj(r) if !r.is_local() => Some(S::Obj::from_u32(r.index())),
            _ => None,
        }
    }

    fn obj(o: S::Obj) -> Value {
        Value::Obj(ObjRef::host(o.to_u32()))
    }

    fn opt_obj(o: Option<S::Obj>) -> Value {
        o.map_or(Value::Null, Self::obj)
    }

    /// A feature of the object's class by name.
    fn feature_named(&self, v: &Value, name: &str) -> Option<FeatureId> {
        let c = self.class_of(v)?;
        let meta = &self.prog.ir.meta;
        meta.class(c)
            .all_features
            .iter()
            .copied()
            .find(|&f| meta.feature(f).name.as_ref() as &str == name)
    }

    /// `v.<name>` (Ecore reflection on metamodel objects); `null` if the feature is absent.
    fn read(&mut self, v: &Value, name: &str) -> Value {
        match self.feature_named(v, name) {
            Some(f) => self.get_prop(v, f).unwrap_or(Value::Invalid),
            None => Value::Null,
        }
    }

    fn read_items(&mut self, v: &Value, name: &str) -> Vec<Value> {
        let l = self.read(v, name);
        crate::vm::Iter::of(&l).items.to_vec()
    }

    fn name_of(&mut self, v: &Value) -> Option<String> {
        self.read(v, "name")
            .as_str()
            .map(|s| s.to_str_lossy().into_owned())
    }

    /// The registry feature an `EStructuralFeature` object stands for.
    fn feature_of(&self, v: &Value) -> Option<FeatureId> {
        self.store.object_feature(self.host(v)?)
    }

    fn same(a: &Value, b: &Value) -> bool {
        matches!((a, b), (Value::Obj(x), Value::Obj(y)) if x == y)
    }

    fn int_arg(v: &Value) -> Option<i64> {
        match v {
            Value::Int(i) => Some(*i),
            Value::Boxed(_, i) => Some(*i as i64),
            _ => None,
        }
    }

    fn index_in(items: &[Value], v: &Value) -> i64 {
        items
            .iter()
            .position(|x| Self::same(x, v))
            .map_or(-1, |i| i as i64)
    }

    /// `list.get(i)` of EMF's id lookups: `null` out of range.
    fn at(items: &[Value], i: Option<i64>) -> Result<Value, String> {
        match i {
            Some(i) if i >= 0 && (i as usize) < items.len() => Ok(items[i as usize].clone()),
            Some(_) => Ok(Value::Null),
            None => Err("NullPointerException".into()),
        }
    }

    /// Ecore's operations; `None` if `name` is none of them. `Err` is a Java exception.
    fn ecore_operation(
        &mut self,
        class: &str,
        name: &str,
        src: &Value,
        args: &[Value],
    ) -> Option<Result<Value, String>> {
        let a0 = args.first().cloned().unwrap_or(Value::Null);
        Some(Ok(match (class, name, args.len()) {
            // ---- EObject ----
            ("EObject", "eClass", 0) => match self.class_of(src) {
                Some(c) => Self::opt_obj(self.store.classifier_object(Classifier::Class(c))),
                None => Value::Null,
            },
            ("EObject", "eIsProxy", 0) => {
                Value::Bool(self.host(src).is_some_and(|h| self.store.is_proxy(h)))
            }
            ("EObject", "eContainer", 0) => match self.host(src) {
                Some(h) => Self::opt_obj(self.store.container(h).map(|(p, _)| p)),
                None => Value::Null,
            },
            ("EObject", "eContainingFeature" | "eContainmentFeature", 0) => {
                match self.host(src).and_then(|h| self.store.container(h)) {
                    Some((_, f)) => Self::opt_obj(self.store.feature_object(f)),
                    None => Value::Null,
                }
            }
            ("EObject", "eContents", 0) => {
                let items = match self.host(src) {
                    Some(h) => self.store.contents(h).into_iter().map(Self::obj).collect(),
                    None => Vec::new(),
                };
                Value::Coll(Rc::new(Coll::Sequence(items)))
            }
            ("EObject", "eAllContents", 0) => {
                let items = match self.host(src) {
                    Some(h) => self.store.all_contents(h).into_iter().map(Self::obj).collect(),
                    None => Vec::new(),
                };
                Value::Coll(Rc::new(Coll::Sequence(items)))
            }
            ("EObject", "eCrossReferences", 0) => {
                let mut items = Vec::new();
                if let Some(h) = self.host(src) {
                    let meta = self.store.meta();
                    let c = self.store.class_of(h);
                    for &f in meta.class(c).all_features.iter() {
                        let fi = meta.feature(f);
                        let container = fi.opposite.is_some_and(|o| meta.feature(o).containment);
                        if !fi.is_reference() || fi.containment || container || fi.derived {
                            continue;
                        }
                        if fi.is_many() {
                            for i in 0..self.store.len(h, f) {
                                if let Datum::Obj(o) = self.store.get_at(h, f, i) {
                                    items.push(Self::obj(o));
                                }
                            }
                        } else if let Datum::Obj(o) = self.store.get(h, f) {
                            items.push(Self::obj(o));
                        }
                    }
                }
                Value::Coll(Rc::new(Coll::Sequence(items)))
            }
            ("EObject", "eResource", 0) => match self.host(src).and_then(|h| self.store.resource_of(h)) {
                // no Resource values: its URI stands for it
                Some(r) => Value::str(self.store.resource_uri(r)),
                None => Value::Null,
            },
            ("EObject", "eGet", 1 | 2) => {
                let Some(f) = self.feature_of(&a0) else {
                    return Some(Err("IllegalArgumentException: not a feature".into()));
                };
                if self.class_of(src).is_some_and(|c| {
                    !self.prog.ir.meta.class(c).all_features.contains(&f)
                }) {
                    return Some(Err(format!(
                        "IllegalArgumentException: The feature '{}' is not a valid feature",
                        self.prog.ir.meta.feature(f).name
                    )));
                }
                let v = match self.get_prop(src, f) {
                    Ok(v) => v,
                    Err(_) => Value::Invalid,
                };
                // an EList is a java.util.List: a Sequence for OCL
                match v {
                    Value::Coll(c) if self.prog.ir.meta.feature(f).is_many() => {
                        Value::Coll(Rc::new(Coll::Sequence(c.to_vec())))
                    }
                    v => v,
                }
            }
            ("EObject", "eSet", 2) => {
                let Some(f) = self.feature_of(&a0) else {
                    return Some(Err("IllegalArgumentException: not a feature".into()));
                };
                let v = args[1].clone();
                if let Err(u) = self.set_prop(src, f, v, true) {
                    return Some(Err(u.error().message.clone().unwrap_or_default()));
                }
                Value::Null
            }
            ("EObject", "eIsSet", 1) => match (self.host(src), self.feature_of(&a0)) {
                (Some(h), Some(f)) => Value::Bool(self.store.is_set(h, f)),
                _ => return Some(Err("IllegalArgumentException: not a feature".into())),
            },
            ("EObject", "eUnset", 1) => match (self.host(src), self.feature_of(&a0)) {
                (Some(h), Some(f)) => {
                    if let Err(e) = self.store.unset(h, f) {
                        return Some(Err(crate::objects::model_error_text(&e)));
                    }
                    Value::Null
                }
                _ => return Some(Err("IllegalArgumentException: not a feature".into())),
            },
            // ---- EModelElement ----
            ("EModelElement", "getEAnnotation", 1) => {
                let source = a0.as_str().map(|s| s.to_str_lossy().into_owned());
                let mut found = Value::Null;
                for a in self.read_items(src, "eAnnotations") {
                    let s = self.read(&a, "source");
                    if s.as_str().map(|s| s.to_str_lossy().into_owned()) == source {
                        found = a;
                        break;
                    }
                }
                found
            }
            // ---- EPackage ----
            ("EPackage", "getEClassifier", 1) => {
                let wanted = a0.as_str().map(|s| s.to_str_lossy().into_owned());
                let mut found = Value::Null;
                for c in self.read_items(src, "eClassifiers") {
                    if self.name_of(&c) == wanted {
                        found = c;
                        break;
                    }
                }
                found
            }
            // ---- EClassifier ----
            ("EClassifier", "isInstance", 1) => {
                let Some(c) = self.host(src).and_then(|h| self.store.object_classifier(h)) else {
                    return Some(Ok(Value::Bool(false)));
                };
                Value::Bool(match c {
                    Classifier::Class(c) => self.value_conforms_to_class(&a0, c),
                    _ => !a0.is_null(),
                })
            }
            ("EClassifier", "getClassifierID", 0) => {
                let pkg = self.read(src, "ePackage");
                let items = self.read_items(&pkg, "eClassifiers");
                Value::Int(Self::index_in(&items, src))
            }
            // ---- EClass ----
            ("EClass", "isSuperTypeOf", 1) => {
                let sups = self.read_items(&a0, "eAllSuperTypes");
                Value::Bool(Self::same(src, &a0) || sups.iter().any(|s| Self::same(s, src)))
            }
            ("EClass", "getFeatureCount", 0) => {
                Value::Int(self.read_items(src, "eAllStructuralFeatures").len() as i64)
            }
            ("EClass", "getEStructuralFeature", 1) => {
                let all = self.read_items(src, "eAllStructuralFeatures");
                match &a0 {
                    Value::Str(s) => {
                        let wanted = Some(s.to_str_lossy().into_owned());
                        let mut found = Value::Null;
                        for f in all {
                            if self.name_of(&f) == wanted {
                                found = f;
                                break;
                            }
                        }
                        found
                    }
                    v => match Self::at(&all, Self::int_arg(v)) {
                        Ok(v) => v,
                        Err(e) => return Some(Err(e)),
                    },
                }
            }
            ("EClass", "getFeatureID", 1) => {
                let all = self.read_items(src, "eAllStructuralFeatures");
                Value::Int(Self::index_in(&all, &a0))
            }
            ("EClass", "getOperationCount", 0) => {
                Value::Int(self.read_items(src, "eAllOperations").len() as i64)
            }
            ("EClass", "getEOperation", 1) => {
                let all = self.read_items(src, "eAllOperations");
                match Self::at(&all, Self::int_arg(&a0)) {
                    Ok(v) => v,
                    Err(e) => return Some(Err(e)),
                }
            }
            ("EClass", "getOperationID", 1) => {
                let all = self.read_items(src, "eAllOperations");
                Value::Int(Self::index_in(&all, &a0))
            }
            ("EClass", "getOverride", 1) => Value::Null,
            ("EClass", "getFeatureType", 1) => self.read(&a0, "eGenericType"),
            // ---- EStructuralFeature / EOperation ----
            ("EStructuralFeature", "getFeatureID", 0) => {
                let c = self.read(src, "eContainingClass");
                let all = self.read_items(&c, "eAllStructuralFeatures");
                Value::Int(Self::index_in(&all, src))
            }
            ("EStructuralFeature", "getContainerClass", 0) => Value::Null,
            ("EOperation", "getOperationID", 0) => {
                let c = self.read(src, "eContainingClass");
                let all = self.read_items(&c, "eAllOperations");
                Value::Int(Self::index_in(&all, src))
            }
            ("EOperation", "isOverrideOf", 1) => Value::Bool(false),
            // ---- EEnum ----
            ("EEnum", "getEEnumLiteral", 1) => {
                let lits = self.read_items(src, "eLiterals");
                let mut found = Value::Null;
                for l in lits {
                    let hit = match &a0 {
                        Value::Str(s) => {
                            self.name_of(&l) == Some(s.to_str_lossy().into_owned())
                        }
                        v => {
                            let lv = self.read(&l, "value");
                            Self::int_arg(&lv).is_some() && Self::int_arg(&lv) == Self::int_arg(v)
                        }
                    };
                    if hit {
                        found = l;
                        break;
                    }
                }
                found
            }
            ("EEnum", "getEEnumLiteralByLiteral", 1) => {
                let wanted = a0.as_str().map(|s| s.to_str_lossy().into_owned());
                let mut found = Value::Null;
                for l in self.read_items(src, "eLiterals") {
                    let lit = self.read(&l, "literal");
                    if lit.as_str().map(|s| s.to_str_lossy().into_owned()) == wanted {
                        found = l;
                        break;
                    }
                }
                found
            }
            // ---- EFactory ----
            ("EFactory", "create", 1) => {
                let Some(Classifier::Class(c)) =
                    self.host(&a0).and_then(|h| self.store.object_classifier(h))
                else {
                    return Some(Err("IllegalArgumentException: not a registered class".into()));
                };
                match self.store.create(c) {
                    Ok(h) => Self::obj(h),
                    Err(e) => return Some(Err(crate::objects::model_error_text(&e))),
                }
            }
            _ => return None,
        }))
    }

    /// A host implementation of a metamodel operation (`Natives::eoperation`), if registered.
    fn native_eoperation(
        &mut self,
        ns: &str,
        class: &str,
        name: &str,
        src: &Value,
        args: Vec<Value>,
    ) -> VmResult<Option<Value>> {
        let key = crate::native::eoperation_key(ns, class, name);
        let Some(n) = self
            .native
            .natives
            .as_ref()
            .and_then(|n| n.get(&key).cloned())
        else {
            return Ok(None);
        };
        let call = crate::native::NativeCall {
            op: None,
            self_: src.clone(),
            args,
        };
        self.invoke_native(&n, call).map(Some)
    }
}

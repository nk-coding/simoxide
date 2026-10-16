//! Runtime types: `oclIsKindOf`, `oclIsTypeOf`, `oclAsType` and QVTo's `isKindOf` on values.

use crate::vm::{ObjHandle, Vm};
use qvto_bytecode::Program;
use qvto_model::{ClassId, DataKind, ModelStore};
use qvto_runtime::{IntBox, Value};
use qvto_types::{CollKind, Type, TypeId};

/// The simple name of a type (stack traces: `Context::op`).
pub fn type_simple_name(p: &Program, t: TypeId) -> String {
    let ir = &p.ir;
    match ir.types.get(t) {
        Type::Class(c) => ir.meta.class(*c).name.to_string(),
        Type::DataType(d) => ir.meta.data_type(*d).name.to_string(),
        Type::Enum(e) => ir.meta.enumeration(*e).name.to_string(),
        Type::OclAny => "OclAny".into(),
        Type::OclVoid => "OclVoid".into(),
        Type::OclInvalid => "OclInvalid".into(),
        Type::Boolean => "Boolean".into(),
        Type::Integer => "Integer".into(),
        Type::Real => "Real".into(),
        Type::String => "String".into(),
        Type::UnlimitedNatural => "UnlimitedNatural".into(),
        Type::Collection(k, e) => format!("{}({})", k.name(), type_simple_name(p, *e)),
        Type::List(e) => format!("List({})", type_simple_name(p, *e)),
        Type::Dict(k, v) => format!(
            "Dict({}, {})",
            type_simple_name(p, *k),
            type_simple_name(p, *v)
        ),
        Type::Module(m) => ir.modules[*m as usize].name.clone(),
        Type::ModelType(m) => ir.model_types[*m as usize].name.clone(),
        Type::Tuple(parts) => format!(
            "Tuple({})",
            parts
                .iter()
                .map(|(n, t)| format!("{n} : {}", type_simple_name(p, *t)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Type::TypeType(_) => "OclType".into(),
        other => format!("{other:?}"),
    }
}

impl<S: ModelStore> Vm<'_, S>
where
    S::Obj: ObjHandle,
{
    pub(crate) fn class_of(&self, v: &Value) -> Option<ClassId> {
        match v {
            Value::Obj(o) => Some(if o.is_local() {
                self.local[o.index() as usize].class
            } else {
                self.store.class_of(S::Obj::from_u32(o.index()))
            }),
            _ => None,
        }
    }

    pub(crate) fn value_conforms_to_class(&self, v: &Value, c: ClassId) -> bool {
        match self.class_of(v) {
            Some(k) => self.prog.ir.meta.is_subclass(k, c),
            None => false,
        }
    }

    /// `isKindOf` of a defined value (classic OCL's `EcoreEvaluationEnvironment.isKindOf`
    /// with QVTo's overrides: OclAny excludes collections, collection types check the first
    /// element).
    pub(crate) fn is_kind_of(&self, v: &Value, t: TypeId) -> bool {
        let ir = &self.prog.ir;
        match ir.types.get(t) {
            Type::OclAny => !matches!(v, Value::Coll(_) | Value::List(_) | Value::Dict(_)),
            Type::OclVoid => v.is_null(),
            Type::OclInvalid => v.is_invalid(),
            Type::Boolean => matches!(v, Value::Bool(_)),
            Type::String => matches!(v, Value::Str(_)),
            Type::Integer | Type::UnlimitedNatural => {
                matches!(
                    v,
                    Value::Int(_) | Value::Boxed(IntBox::Long | IntBox::Short | IntBox::Byte, _)
                )
            }
            // only a java.lang.Integer is also a Real (EcoreEvaluationEnvironment)
            Type::Real => match v {
                Value::Real(_) | Value::Float(_) => true,
                Value::Int(i) => *i as i32 as i64 == *i,
                _ => false,
            },
            Type::Class(c) => {
                if *c == ir.std_classes.element {
                    return self.class_of(v).is_some();
                }
                if *c == ir.std_classes.object {
                    return self.class_of(v).is_some();
                }
                if *c == ir.std_classes.model {
                    return matches!(v, Value::Model(_));
                }
                if *c == ir.std_classes.transformation {
                    return matches!(v, Value::Instance(_));
                }
                self.value_conforms_to_class(v, *c)
            }
            Type::Enum(e) => matches!(v, Value::Enum(l) if l.enum_id == e.0),
            Type::DataType(d) => {
                let dt = ir.meta.data_type(*d);
                match dt.kind {
                    DataKind::String => matches!(v, Value::Str(_)),
                    DataKind::Boolean => matches!(v, Value::Bool(_)),
                    DataKind::Int => matches!(v, Value::Int(i) if *i as i32 as i64 == *i),
                    DataKind::Long => {
                        matches!(v, Value::Boxed(IntBox::Long, _))
                            || matches!(v, Value::Int(i) if *i as i32 as i64 != *i)
                    }
                    DataKind::Short => matches!(v, Value::Boxed(IntBox::Short, _)),
                    DataKind::Byte => matches!(v, Value::Boxed(IntBox::Byte, _)),
                    DataKind::Char => matches!(v, Value::Boxed(IntBox::Char, _)),
                    DataKind::Double => matches!(v, Value::Real(_)),
                    DataKind::Float => matches!(v, Value::Float(_)),
                    _ => !v.is_undefined(),
                }
            }
            Type::Collection(k, e) => {
                let (kind_ok, first) = match v {
                    Value::Coll(c) => {
                        let ok = match k {
                            CollKind::Collection => true,
                            CollKind::Set => c.kind() == qvto_runtime::CollKind::Set,
                            CollKind::OrderedSet => c.kind() == qvto_runtime::CollKind::OrderedSet,
                            CollKind::Bag => c.kind() == qvto_runtime::CollKind::Bag,
                            CollKind::Sequence => c.kind() == qvto_runtime::CollKind::Sequence,
                        };
                        (ok, c.iter().next().cloned())
                    }
                    Value::List(l) => (
                        matches!(k, CollKind::Collection | CollKind::Sequence),
                        l.borrow().items.first().cloned(),
                    ),
                    Value::Dict(d) => (
                        *k == CollKind::Collection,
                        d.borrow().values().first().cloned(),
                    ),
                    _ => return false,
                };
                kind_ok && first.is_none_or(|x| self.is_kind_of(&x, *e))
            }
            Type::List(e) => match v {
                Value::List(l) => l
                    .borrow()
                    .items
                    .first()
                    .is_none_or(|x| self.is_kind_of(x, *e)),
                _ => false,
            },
            Type::Dict(..) => matches!(v, Value::Dict(_)),
            Type::Tuple(_) => match v {
                Value::Tuple(tp) => tp.type_id == t.0,
                _ => false,
            },
            Type::ModelType(m) => match v {
                Value::Model(x) => {
                    let ext = &self.extents[*x as usize];
                    match ext.param {
                        Some((module, idx)) => {
                            let mp = &ir.modules[module.index()].model_params[idx as usize];
                            mp.model_type.0 == *m
                        }
                        None => false,
                    }
                }
                _ => false,
            },
            Type::Module(m) => match v {
                Value::Instance(i) => {
                    let i = if i & crate::run::NESTED != 0 {
                        self.nested[(i & !crate::run::NESTED) as usize].0.0
                    } else {
                        *i
                    };
                    i == *m || self.module_extends(i, *m)
                }
                _ => false,
            },
            Type::TypeType(_) => matches!(v, Value::Type(_)),
            _ => false,
        }
    }

    fn module_extends(&self, m: u32, sup: u32) -> bool {
        let module = &self.prog.ir.modules[m as usize];
        module.usages.iter().any(|u| {
            u.kind == qvto_sema::ir::UsageKind::Extends
                && (u.module.0 == sup || self.module_extends(u.module.0, sup))
        })
    }

    /// `oclIsKindOf` with lax null handling: `null` conforms to everything but OclInvalid,
    /// `invalid` to everything.
    pub(crate) fn ocl_is_kind_of(&self, v: &Value, t: TypeId) -> bool {
        match v {
            Value::Null => t != TypeId::OCL_INVALID,
            Value::Invalid => true,
            _ => self.is_kind_of(v, t),
        }
    }

    /// `oclIsTypeOf`.
    pub(crate) fn ocl_is_type_of(&self, v: &Value, t: TypeId) -> bool {
        let ir = &self.prog.ir;
        match v {
            Value::Null => t == TypeId::OCL_VOID,
            Value::Invalid => t == TypeId::OCL_INVALID,
            Value::Obj(_) => match ir.types.get(t) {
                Type::Class(c) => self.class_of(v) == Some(*c),
                _ => false,
            },
            Value::Int(i) => match ir.types.get(t) {
                Type::Integer => true,
                Type::Real => false,
                Type::DataType(_) => self.is_kind_of(v, t),
                _ => {
                    let _ = i;
                    false
                }
            },
            Value::Real(_) => {
                matches!(ir.types.get(t), Type::Real)
                    || matches!(ir.types.get(t), Type::DataType(_) if self.is_kind_of(v, t))
            }
            Value::Coll(c) => match ir.types.get(t) {
                Type::Collection(k, _) => {
                    *k != CollKind::Collection
                        && k.name() == c.kind().ocl_name()
                        && self.is_kind_of(v, t)
                }
                _ => false,
            },
            _ => self.is_kind_of(v, t),
        }
    }

    /// QVTo's `oclIsKindOf` of the typed `xselect` (`QvtImperativeIteratorTemplate`):
    /// undefined values only match their own types exactly.
    pub(crate) fn qvt_kind_of(&self, v: &Value, t: TypeId) -> bool {
        if v.is_undefined() {
            return self.ocl_is_type_of(v, t);
        }
        self.is_kind_of(v, t)
    }

    /// `oclAsType(T)`.
    pub(crate) fn ocl_as_type(&self, v: Value, t: TypeId) -> Value {
        if v.is_undefined() {
            return v;
        }
        let ir = &self.prog.ir;
        match (ir.types.get(t), &v) {
            // Real to Integer truncates (intValue)
            (Type::Integer, Value::Real(x)) => return Value::Int(*x as i32 as i64),
            (Type::Real, Value::Int(i)) if *i as i32 as i64 == *i => {
                return Value::Real(*i as f64);
            }
            _ => {}
        }
        if self.is_kind_of(&v, t) {
            v
        } else {
            Value::Invalid
        }
    }

    /// A readable name of the value's type for messages.
    pub(crate) fn type_of_name(&self, v: &Value) -> String {
        match self.class_of(v) {
            Some(c) => self.prog.ir.meta.class(c).name.to_string(),
            None => format!("{v:?}"),
        }
    }
}

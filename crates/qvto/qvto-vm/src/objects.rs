//! Objects: navigation, QVTo's assignment semantics (`assign`, `callSetter`), creation into
//! extents, engine-local objects (intermediate classes, library classes), contextual
//! intermediate properties, default values.

use crate::error::VmResult;
use crate::vm::{LocalObj, ObjHandle, Vm, push};
use qvto_bytecode::{Program, Site};
use qvto_model::{
    ClassId, Classifier, DataKind, Datum, FeatureId, ModelError, ModelStore, OwnedDatum,
};
use qvto_runtime::coll::Coll;
use qvto_runtime::dict::{Dict, List};
use qvto_runtime::model::datum_to_value;
use qvto_runtime::{IntBox, ObjRef, Str, Value};
use qvto_sema::ir::{Direction, IntermPropId, ModelParamRef};
use qvto_types::{CollKind, Type, TypeId};
use std::cell::RefCell;
use std::rc::Rc;

/// Which classes have engine-local instances: QVTo's library classes and the intermediate
/// classes of the program.
pub fn local_classes(p: &Program) -> Vec<bool> {
    let meta = &p.ir.meta;
    let mut v = vec![false; meta.class_count()];
    for c in meta.classes() {
        if meta.class(c).package == p.ir.std_classes.package {
            v[c.index()] = true;
        }
    }
    for ic in &p.ir.interm_classes {
        v[ic.class.index()] = true;
    }
    v
}

/// The Java number class an assigned value is converted to (`NumberConversions.convertNumber`).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum NumClass {
    Integer,
    Long,
    Short,
    Byte,
    Double,
    Float,
}

pub(crate) fn convert_number(v: Value, c: Option<NumClass>) -> Value {
    let Some(c) = c else { return v };
    let Some(n) = v.as_jnum() else { return v };
    match c {
        NumClass::Integer => Value::Int(n.as_i32() as i64),
        NumClass::Long => Value::long(n.as_i64()),
        NumClass::Short => Value::Boxed(IntBox::Short, n.as_i64() as i16 as i32),
        NumClass::Byte => Value::Boxed(IntBox::Byte, n.as_i64() as i8 as i32),
        NumClass::Double => Value::Real(n.as_f64()),
        NumClass::Float => Value::Float(n.as_f64() as f32),
    }
}

fn data_num_class(k: DataKind) -> Option<NumClass> {
    Some(match k {
        DataKind::Int => NumClass::Integer,
        DataKind::Long => NumClass::Long,
        DataKind::Short => NumClass::Short,
        DataKind::Byte => NumClass::Byte,
        DataKind::Double => NumClass::Double,
        DataKind::Float => NumClass::Float,
        _ => return None,
    })
}

impl<S: ModelStore> Vm<'_, S>
where
    S::Obj: ObjHandle,
{
    pub(crate) fn is_local_class(&self, c: ClassId) -> bool {
        self.class_local.get(c.index()).copied().unwrap_or(false)
    }

    pub(crate) fn datum(&self, d: Datum<'_, S::Obj>) -> Value {
        datum_to_value(d, |o| ObjRef::host(o.to_u32()))
    }

    /// The instance class of a type for number conversions on assignment.
    pub(crate) fn num_class_of_type(&self, t: TypeId) -> Option<NumClass> {
        match self.prog.ir.types.get(t) {
            Type::Integer | Type::UnlimitedNatural => Some(NumClass::Integer),
            Type::Real => Some(NumClass::Double),
            Type::DataType(d) => data_num_class(self.prog.ir.meta.data_type(*d).kind),
            _ => None,
        }
    }

    fn num_class_of_feature(&self, f: FeatureId) -> Option<NumClass> {
        match self.prog.ir.meta.feature(f).ty {
            Classifier::DataType(d) => data_num_class(self.prog.ir.meta.data_type(d).kind),
            _ => None,
        }
    }

    // ---- creation ---------------------------------------------------------------------------

    pub(crate) fn new_local(&mut self, class: ClassId) -> ObjRef {
        let n = self.prog.ir.meta.class(class).all_features.len();
        self.local.push(LocalObj {
            class,
            slots: vec![Value::Null; n],
            exception: None,
            status: None,
        });
        ObjRef::local((self.local.len() - 1) as u32)
    }

    /// `createInstance` of a class into an extent (explicit or default).
    pub(crate) fn create_object(
        &mut self,
        class: ClassId,
        extent: Option<ModelParamRef>,
    ) -> VmResult<Value> {
        let ci = self.prog.ir.meta.class(class);
        if ci.is_abstract || ci.is_interface {
            return Err(self.runtime_error(format!(
                "Cannot instantiate type {}",
                self.qualified_class_name(class)
            )));
        }
        if self.is_local_class(class) {
            let o = self.new_local(class);
            self.init_local_features(o, class)?;
            let x = self.default_extent(class, extent);
            self.put_to_extent(o, x)?;
            return Ok(Value::Obj(o));
        }
        let h = match self.store.create(class) {
            Ok(h) => h,
            Err(e) => {
                return Err(self.runtime_error(format!(
                    "Cannot instantiate type {}: {e}",
                    self.qualified_class_name(class)
                )));
            }
        };
        let o = ObjRef::host(h.to_u32());
        let x = self.default_extent(class, extent);
        self.put_to_extent(o, x)?;
        Ok(Value::Obj(o))
    }

    pub(crate) fn qualified_class_name(&self, c: ClassId) -> String {
        let meta = &self.prog.ir.meta;
        let ci = meta.class(c);
        format!("{}::{}", meta.package_path(ci.package, "::"), ci.name)
    }

    fn init_local_features(&mut self, o: ObjRef, class: ClassId) -> VmResult<()> {
        let meta = &self.prog.ir.meta;
        let feats: Vec<FeatureId> = meta.class(class).all_features.to_vec();
        for (slot, f) in feats.iter().enumerate() {
            let v = self.feature_default(*f);
            self.local[o.index() as usize].slots[slot] = v;
        }
        // initialisers of the class and its intermediate superclasses
        let prog = self.prog;
        for (ci, ic) in prog.ir.interm_classes.iter().enumerate() {
            if !prog.ir.meta.is_subclass(class, ic.class) {
                continue;
            }
            for (f, code) in &prog.interm_inits[ci] {
                let v = self.run_body(code, Value::Obj(o))?;
                let v = if v.is_null() {
                    self.feature_default(*f)
                } else {
                    v
                };
                // `callSetter(instance, feature, value, isUndefined, true)`
                if prog.ir.meta.slot(class, *f).is_some() {
                    self.set_prop(&Value::Obj(o), *f, v, true)?;
                }
            }
        }
        Ok(())
    }

    /// The type default of a feature of a local object.
    fn feature_default(&self, f: FeatureId) -> Value {
        let meta = &self.prog.ir.meta;
        let fi = meta.feature(f);
        // `createInitialValue` of the declared OCL type
        if let Some(&ty) = self.interm_feature_ty.get(&f)
            && !fi.is_many()
        {
            return self.default_value(ty);
        }
        if fi.is_many() {
            let kind = match CollKind::of_feature(fi.ordered, fi.unique) {
                CollKind::Set => qvto_bytecode::insn::coll_kind::SET,
                CollKind::OrderedSet => qvto_bytecode::insn::coll_kind::ORDERED_SET,
                CollKind::Bag => qvto_bytecode::insn::coll_kind::BAG,
                _ => qvto_bytecode::insn::coll_kind::SEQUENCE,
            };
            return self.new_coll(kind);
        }
        match fi.ty {
            Classifier::DataType(d) => match meta.data_type(d).kind {
                DataKind::String => Value::str(""),
                DataKind::Boolean => Value::Bool(false),
                DataKind::Int => Value::Int(0),
                DataKind::Long => Value::long(0),
                DataKind::Double => Value::Real(0.0),
                DataKind::Float => Value::Float(0.0),
                _ => Value::Null,
            },
            _ => Value::Null,
        }
    }

    /// The extent of a new object: the explicit one, else the default one (intermediate
    /// classes: the intermediate extent; model classes: the first non-`in` parameter whose
    /// model type's first metamodel is the class's root package, else the first non-`in`
    /// parameter, else the unbound extent).
    pub(crate) fn default_extent(&self, class: ClassId, extent: Option<ModelParamRef>) -> usize {
        if let Some(x) = extent {
            // a parameter of a module that is not bound in this run (an imported
            // transformation's own extent): not an output
            return self
                .param_extents
                .get(&(x.module.0, x.index))
                .copied()
                .unwrap_or(self.unbound_extent);
        }
        if self.is_local_class(class) {
            return self.unbound_extent;
        }
        let ir = &self.prog.ir;
        let cur = self.cur_transf;
        let main = &ir.modules[cur.index()];
        let root = self.root_package(ir.meta.class(class).package);
        for (i, mp) in main.model_params.iter().enumerate() {
            if mp.direction == Direction::In {
                continue;
            }
            let mt = &ir.model_types[mp.model_type.index()];
            if mt.packages.first() == Some(&root)
                && let Some(&e) = self.param_extents.get(&(cur.0, i as u32))
            {
                return e;
            }
        }
        for (i, mp) in main.model_params.iter().enumerate() {
            if mp.direction == Direction::In {
                continue;
            }
            if let Some(&e) = self.param_extents.get(&(cur.0, i as u32)) {
                return e;
            }
        }
        self.unbound_extent
    }

    fn root_package(&self, mut p: qvto_model::PackageId) -> qvto_model::PackageId {
        let meta = &self.prog.ir.meta;
        while let Some(sp) = meta.package(p).super_package {
            p = sp;
        }
        p
    }

    pub(crate) fn put_to_extent(&mut self, o: ObjRef, x: usize) -> VmResult<()> {
        if self.opts.readonly_guard && self.extents[x].is_readonly() {
            return Err(self.runtime_error(format!(
                "Input model parameter '{}' is read-only!",
                self.extents[x].label
            )));
        }
        let mut next = self.next_extent_uri;
        self.extents[x].add_object(self.store, o, &mut next);
        self.next_extent_uri = next;
        Ok(())
    }

    /// `new T(args)`.
    pub(crate) fn instantiate(&mut self, site: &Site, at: usize) -> VmResult<Value> {
        let Site::New {
            ty,
            extent,
            constructor,
            nargs,
        } = *site
        else {
            unreachable!()
        };
        let args: Vec<Value> = (0..nargs as usize)
            .map(|i| self.regs[at + i].clone())
            .collect();
        let ir = &self.prog.ir;
        match ir.types.get(ty).clone() {
            Type::Class(c) => {
                let obj = self.create_object(c, extent)?;
                match constructor {
                    Some(op) => {
                        self.call_constructor(op, obj.clone(), args)?;
                    }
                    None => {
                        if !args.is_empty() {
                            return Err(self.runtime_error("Undefined constructor is called"));
                        }
                    }
                }
                Ok(obj)
            }
            Type::DataType(d) => {
                let dt = ir.meta.data_type(d);
                match args.as_slice() {
                    [Value::Str(s)] => {
                        let text = s.to_str_lossy().into_owned();
                        match crate::stdlib::parse_data_value(dt.kind, &text) {
                            Some(v) => Ok(v),
                            None => Err(self.runtime_error(format!(
                                "Failed to instantiate datatype '{}'",
                                dt.name
                            ))),
                        }
                    }
                    _ => Err(self.runtime_error(format!(
                        "Invalid parameters for datatype instantiation of '{}'",
                        dt.name
                    ))),
                }
            }
            Type::Collection(CollKind::Collection, _) => Err(self.runtime_error(format!(
                "Cannot instantiate type {}",
                crate::types::type_simple_name(self.prog, ty)
            ))),
            Type::Collection(..) | Type::List(_) | Type::Dict(..) => {
                let mut v = self.default_value(ty);
                for a in args {
                    push(&mut v, a, &self.env);
                }
                Ok(v)
            }
            _ => Err(self.runtime_error(format!(
                "Cannot instantiate type {}",
                crate::types::type_simple_name(self.prog, ty)
            ))),
        }
    }

    // ---- defaults and assignment ------------------------------------------------------------

    /// `EvaluationUtil.createInitialValue`.
    pub(crate) fn default_value(&self, t: TypeId) -> Value {
        use qvto_bytecode::insn::coll_kind as k;
        match self.prog.ir.types.get(t) {
            Type::String => Value::str(""),
            Type::Boolean => Value::Bool(false),
            Type::Integer | Type::UnlimitedNatural => Value::Int(0),
            Type::Real => Value::Real(0.0),
            Type::Collection(kind, _) => match kind {
                CollKind::Set => self.new_coll(k::SET),
                CollKind::OrderedSet => self.new_coll(k::ORDERED_SET),
                CollKind::Bag => self.new_coll(k::BAG),
                CollKind::Sequence => self.new_coll(k::SEQUENCE),
                CollKind::Collection => Value::Null,
            },
            Type::List(_) => self.new_coll(k::LIST),
            Type::Dict(..) => self.new_coll(k::DICT),
            Type::Tuple(parts) => {
                let names: Rc<[Str]> = parts.iter().map(|(n, _)| Str::new(n)).collect();
                let values: Vec<Value> = parts
                    .iter()
                    .map(|(_, pt)| self.default_value(*pt))
                    .collect();
                Value::Tuple(Rc::new(qvto_runtime::tuple::Tuple::new(
                    t.0,
                    names,
                    values.into_boxed_slice(),
                )))
            }
            _ => Value::Null,
        }
    }

    /// `createNewCollection(type)`; `None` for `Collection(T)` and non-collection types.
    fn new_collection_of_type(&self, t: TypeId) -> Option<Value> {
        match self.prog.ir.types.get(t) {
            Type::Collection(CollKind::Collection, _) => None,
            Type::Collection(..) | Type::List(_) | Type::Dict(..) => Some(self.default_value(t)),
            _ => None,
        }
    }

    fn element_type(&self, t: TypeId) -> TypeId {
        self.prog.ir.types.element(t).unwrap_or(TypeId::OCL_ANY)
    }

    /// [`Vm::assign_value`] with fast paths that keep Eclipse's results: a fresh, unshared
    /// collection of the variable's kind whose table a copy would rebuild identically is
    /// taken as is (`:=`), and `+=` reuses an unshared old collection when the copy
    /// constructor would rebuild it identically.
    pub(crate) fn assign_fast(
        &self,
        ty: TypeId,
        old: Value,
        rhs: Value,
        reset: bool,
        same_elem: bool,
    ) -> Value {
        if reset
            && same_elem
            && let Value::Coll(c) = &rhs
            && let Type::Collection(k, _) = self.prog.ir.types.get(ty)
            && k.name() == c.kind().ocl_name()
            && Rc::strong_count(c) == 1
            && qvto_runtime::coll::ops::fresh_add_equivalent(c)
        {
            return rhs;
        }
        self.assign_value(ty, old, rhs, reset)
    }

    /// `callSetter` of an undefined value on a module property. Module properties are
    /// single-valued features (also of collection types) whose OCL types accept null: `assign`
    /// gives the new value, an invalid one unsets the feature (null) on `:=` and leaves it
    /// unchanged on `+=`.
    pub(crate) fn property_undefined(&self, ty: TypeId, old: Value, rhs: Value, reset: bool) -> Value {
        if rhs.is_invalid() {
            if reset { Value::Null } else { old }
        } else {
            self.assign_value(ty, old, rhs, reset)
        }
    }

    /// QVTo's `assign(lhsType, old, rhs, isReset)` (`QvtOperationalEvaluationEnv`).
    pub(crate) fn assign_value(&self, ty: TypeId, old: Value, rhs: Value, reset: bool) -> Value {
        let types = &self.prog.ir.types;
        if !types.is_collection_like(ty) || rhs.is_invalid() {
            return self.list_coercion(ty, rhs);
        }
        let elem = self.element_type(ty);
        let nc = self.num_class_of_type(elem);
        let new = if reset {
            if matches!(rhs, Value::List(_) | Value::Dict(_)) {
                rhs
            } else {
                let mut c = match self.new_collection_of_type(ty) {
                    Some(c) => c,
                    None => match &rhs {
                        Value::Coll(_) | Value::List(_) | Value::Dict(_) => {
                            self.new_coll_like(&rhs)
                        }
                        _ => self.new_coll(qvto_bytecode::insn::coll_kind::SET),
                    },
                };
                match &rhs {
                    Value::Coll(_) | Value::List(_) | Value::Dict(_) => {
                        for e in crate::vm::Iter::of(&rhs).items.iter() {
                            push(&mut c, convert_number(e.clone(), nc), &self.env);
                        }
                    }
                    Value::Null => {}
                    _ => push(&mut c, convert_number(rhs, nc), &self.env),
                }
                c
            }
        } else {
            let base = match &old {
                Value::Coll(_) | Value::List(_) | Value::Dict(_) => old,
                _ => self
                    .new_collection_of_type(ty)
                    .unwrap_or_else(|| self.new_coll(qvto_bytecode::insn::coll_kind::SET)),
            };
            self.append(base, rhs, elem)
        };
        self.list_coercion(ty, new)
    }

    /// QVTo's `append`: List and Dict are mutated, other collections copied
    /// (`createNewCollection(c)`, which reuses unshared storage when the copy would be
    /// identical).
    fn append(&self, coll: Value, content: Value, elem: TypeId) -> Value {
        let nc = self.num_class_of_type(elem);
        let converted = convert_number(content.clone(), nc);
        let single = converted.is_null() || self.is_kind_of(&converted, elem);
        let items: Vec<Value> = if single {
            vec![converted]
        } else if matches!(content, Value::Coll(_) | Value::List(_) | Value::Dict(_)) {
            crate::vm::Iter::of(&content)
                .items
                .iter()
                .map(|e| convert_number(e.clone(), nc))
                .collect()
        } else {
            Vec::new()
        };
        match coll {
            Value::Coll(c) => {
                // the copy, then the additions in place
                let kind = c.kind();
                let mut v = qvto_runtime::coll::ops::as_same(Value::Coll(c), kind, &self.env);
                for x in items {
                    push(&mut v, x, &self.env);
                }
                v
            }
            mut other => {
                for x in items {
                    push(&mut other, x, &self.env);
                }
                other
            }
        }
    }

    /// `EvaluationUtil.doImplicitListCoercion`.
    pub(crate) fn list_coercion(&self, ty: TypeId, v: Value) -> Value {
        match (self.prog.ir.types.get(ty), &v) {
            (Type::List(_), Value::Coll(_) | Value::Dict(_)) => {
                let items = crate::vm::Iter::of(&v).items.to_vec();
                Value::List(Rc::new(RefCell::new(List::from_vec(items))))
            }
            (Type::Collection(CollKind::Sequence, _), Value::List(l)) => {
                Value::Coll(Rc::new(Coll::Sequence(l.borrow().items.clone())))
            }
            _ => v,
        }
    }

    // ---- navigation -------------------------------------------------------------------------

    /// `obj.feature`.
    pub(crate) fn get_prop(&mut self, o: &Value, f: FeatureId) -> VmResult<Value> {
        let r = match o {
            Value::Obj(r) => *r,
            Value::Null | Value::Invalid => return Ok(Value::Invalid),
            Value::Tuple(t) => {
                let name = &self.prog.ir.meta.feature(f).name;
                return Ok(t.get(name).cloned().unwrap_or(Value::Null));
            }
            _ => return Ok(Value::Invalid),
        };
        let meta = &self.prog.ir.meta;
        let fi = meta.feature(f);
        if r.is_local() {
            let lo = &self.local[r.index() as usize];
            return match meta.slot(lo.class, f) {
                Some(s) => Ok(lo.slots[s].clone()),
                None => Err(self.runtime_error(format!("Unknown property '{}'", fi.name))),
            };
        }
        let h = S::Obj::from_u32(r.index());
        let class = self.store.class_of(h);
        if meta.slot(class, f).is_none()
            && !self.store.meta().class(class).all_features.contains(&f)
        {
            return Err(self.runtime_error(format!("Unknown property '{}'", fi.name)));
        }
        if !fi.is_many() {
            let d = self.store.get(h, f);
            return Ok(self.datum(d));
        }
        let n = self.store.len(h, f);
        let mut items = Vec::with_capacity(n);
        for i in 0..n {
            let d = self.store.get_at(h, f, i);
            items.push(self.datum(d));
        }
        let kind = CollKind::of_feature(fi.ordered, fi.unique);
        Ok(match kind {
            CollKind::Sequence => Value::Coll(Rc::new(Coll::Sequence(items))),
            k => {
                let rk = match k {
                    CollKind::Set => qvto_runtime::CollKind::Set,
                    CollKind::OrderedSet => qvto_runtime::CollKind::OrderedSet,
                    _ => qvto_runtime::CollKind::Bag,
                };
                Value::Coll(Rc::new(Coll::from_values(rk, items, &self.env)))
            }
        })
    }

    /// The OCL type of a feature (`getOCLType`): the element type, or a collection of the
    /// feature's kind for many-valued features.
    fn feature_ocl_type(&self, f: FeatureId) -> FeatureType {
        let fi = self.prog.ir.meta.feature(f);
        FeatureType {
            many: fi.is_many(),
            kind: CollKind::of_feature(fi.ordered, fi.unique),
            num: self.num_class_of_feature(f),
            primitive: matches!(fi.ty, Classifier::DataType(d)
                if matches!(self.prog.ir.meta.data_type(d).instance_class_name.as_deref(),
                    Some("int" | "long" | "short" | "byte" | "char" | "double" | "float" | "boolean"))),
            is_ref: fi.is_reference(),
            upper: fi.upper,
        }
    }

    /// `callSetter(owner, feature, value, isUndefined, isReset)`.
    pub(crate) fn set_prop(
        &mut self,
        o: &Value,
        f: FeatureId,
        v: Value,
        reset: bool,
    ) -> VmResult<()> {
        let r = match o {
            Value::Obj(r) => *r,
            Value::Invalid => return Ok(()),
            Value::Null => {
                let name = self.prog.ir.meta.feature(f).name.clone();
                self.swallow(format!("Null owner for property '{name}'"));
                return Ok(());
            }
            _ => return Ok(()),
        };
        let ft = self.feature_ocl_type(f);
        let undefined = v.is_undefined();
        if r.is_local() {
            let class = self.local[r.index() as usize].class;
            let Some(slot) = self.prog.ir.meta.slot(class, f) else {
                let name = self.prog.ir.meta.feature(f).name.clone();
                return Err(self.runtime_error(format!("Unknown property '{name}'")));
            };
            let cur = self.local[r.index() as usize].slots[slot].clone();
            let nv = if ft.many {
                if undefined {
                    if reset { self.new_coll_like(&cur) } else { cur }
                } else {
                    self.assign_coll(&ft, cur, v, reset)
                }
            } else if let Some(&ty) = self
                .interm_feature_ty
                .get(&f)
                .filter(|t| self.prog.ir.types.is_collection_like(**t))
            {
                // a single-valued feature of a collection type: `assign` on its OCL type
                let nv = self.assign_value(ty, cur.clone(), v, reset);
                if nv.is_invalid() {
                    if reset { Value::Null } else { cur }
                } else {
                    nv
                }
            } else if v.is_invalid() {
                if reset { Value::Null } else { cur }
            } else {
                convert_number(v, ft.num)
            };
            self.local[r.index() as usize].slots[slot] = nv;
            return Ok(());
        }
        let h = S::Obj::from_u32(r.index());
        if self.opts.readonly_guard {
            self.check_readonly(r, f, &v)?;
        }
        if ft.many {
            if undefined {
                if reset {
                    let _ = self.store.clear(h, f);
                }
                return Ok(());
            }
            // current EList as a Sequence (EList is a List: `createNewCollection` copies it
            // into an ArrayList)
            let n = self.store.len(h, f);
            let cur: Vec<Value> = (0..n)
                .map(|i| self.datum(self.store.get_at(h, f, i)))
                .collect();
            let cur = Value::Coll(Rc::new(Coll::Sequence(cur)));
            let nv = self.assign_coll(&ft, cur, v, reset);
            let mut items: Vec<Value> = crate::vm::Iter::of(&nv).items.to_vec();
            if ft.is_ref {
                items.retain(|x| !x.is_null());
            }
            let mut owned: Vec<OwnedDatum<S::Obj>> = Vec::with_capacity(items.len());
            for x in items {
                let x = convert_number(x, ft.num);
                match self.to_datum(&x) {
                    Some(d) => owned.push(d),
                    None => {
                        self.swallow("ClassCastException: not a feature value");
                        return Ok(());
                    }
                }
            }
            let datums: Vec<Datum<'_, S::Obj>> = owned.iter().map(|d| d.as_datum()).collect();
            if let Err(e) = self.store.set_list(h, f, &datums) {
                // Failed to merge new content to multivalued feature: whole re-assignment
                let _ = e;
                let _ = self.store.clear(h, f);
                for d in &datums {
                    if let Err(e) = self.store.add(h, f, *d, None) {
                        self.swallow(model_error_text(&e));
                        break;
                    }
                }
            }
            return Ok(());
        }
        let nv = v;
        if nv.is_invalid() || (nv.is_null() && ft.primitive) {
            if reset && let Err(e) = self.store.unset(h, f) {
                self.swallow(model_error_text(&e));
            }
            return Ok(());
        }
        let nv = match nv {
            Value::Coll(_) | Value::List(_) | Value::Dict(_) if ft.upper == -2 => {
                crate::vm::Iter::of(&nv)
                    .items
                    .iter()
                    .find(|x| !x.is_null())
                    .cloned()
                    .unwrap_or(nv)
            }
            x => x,
        };
        let nv = convert_number(nv, ft.num);
        let Some(d) = self.to_datum(&nv) else {
            self.swallow("ClassCastException: value is not a feature value");
            return Ok(());
        };
        if let Err(e) = self.store.set(h, f, d.as_datum()) {
            self.swallow(model_error_text(&e));
        }
        Ok(())
    }

    fn assign_coll(&self, ft: &FeatureType, cur: Value, v: Value, reset: bool) -> Value {
        use qvto_bytecode::insn::coll_kind as k;
        if reset {
            if matches!(v, Value::List(_) | Value::Dict(_)) {
                return v;
            }
            let mut c = self.new_coll(match ft.kind {
                CollKind::Set => k::SET,
                CollKind::OrderedSet => k::ORDERED_SET,
                CollKind::Bag => k::BAG,
                _ => k::SEQUENCE,
            });
            match &v {
                Value::Coll(_) | Value::List(_) | Value::Dict(_) => {
                    for e in crate::vm::Iter::of(&v).items.iter() {
                        push(&mut c, convert_number(e.clone(), ft.num), &self.env);
                    }
                }
                Value::Null => {}
                _ => push(&mut c, convert_number(v, ft.num), &self.env),
            }
            c
        } else {
            let mut c = match cur {
                Value::Coll(c) => Value::Coll(Rc::new(Coll::copy_of(c.kind(), &c, &self.env))),
                other => other,
            };
            match &v {
                Value::Coll(_) | Value::List(_) | Value::Dict(_) => {
                    for e in crate::vm::Iter::of(&v).items.iter() {
                        push(&mut c, convert_number(e.clone(), ft.num), &self.env);
                    }
                }
                _ => push(&mut c, convert_number(v, ft.num), &self.env),
            }
            c
        }
    }

    fn check_readonly(&self, owner: ObjRef, f: FeatureId, v: &Value) -> VmResult<()> {
        let ro = |o: ObjRef| -> Option<usize> {
            // the extent of the object's root, if read-only
            let mut cur = o;
            loop {
                for (i, x) in self.extents.iter().enumerate() {
                    if x.is_readonly() && x.initial.contains(&cur) {
                        return Some(i);
                    }
                }
                if cur.is_local() {
                    return None;
                }
                match self.store.container(S::Obj::from_u32(cur.index())) {
                    Some((p, _)) => cur = ObjRef::host(p.to_u32()),
                    None => return None,
                }
            }
        };
        let mut hit = ro(owner);
        if hit.is_none() && self.prog.ir.meta.feature(f).containment {
            if let Value::Obj(x) = v {
                hit = ro(*x);
            }
        }
        if let Some(i) = hit {
            return Err(self.runtime_error(format!(
                "Input model parameter '{}' is read-only!",
                self.extents[i].label
            )));
        }
        Ok(())
    }

    pub(crate) fn to_datum(&self, v: &Value) -> Option<OwnedDatum<S::Obj>> {
        match v {
            Value::Obj(r) if r.is_local() => None,
            _ => qvto_runtime::model::value_to_owned_datum(v, |r| S::Obj::from_u32(r.index())),
        }
    }

    // ---- contextual intermediate properties --------------------------------------------------

    pub(crate) fn get_interm(&mut self, o: &Value, p: IntermPropId) -> Value {
        let Some(r) = IntermOwner::of(o) else {
            return Value::Invalid;
        };
        if let Some(v) = self.interm.get(&(r, p)) {
            return v.clone();
        }
        let ty = self.prog.ir.interm_props[p.index()].ty;
        let v = self.default_value(ty);
        self.interm.insert((r, p), v.clone());
        v
    }

    pub(crate) fn set_interm(&mut self, o: &Value, p: IntermPropId, v: Value, reset: bool) {
        let Some(r) = IntermOwner::of(o) else { return };
        let ty = self.prog.ir.interm_props[p.index()].ty;
        let old = self.get_interm(o, p);
        let nv = if self.prog.ir.types.is_collection_like(ty) {
            if v.is_undefined() {
                if reset { self.default_value(ty) } else { old }
            } else {
                self.assign_value(ty, old, v, reset)
            }
        } else if v.is_invalid() {
            if reset { Value::Null } else { old }
        } else {
            convert_number(v, self.num_class_of_type(ty))
        };
        self.interm.insert((r, p), nv);
    }

    // ---- copies ------------------------------------------------------------------------------

    /// `Element::clone()` (no containment copied) and `deepclone()` (`EcoreUtil.Copier`:
    /// containment copied, references to copied objects redirected, other references kept),
    /// the copy added to the default extent of its class.
    pub(crate) fn copy_object(&mut self, r: ObjRef, deep: bool) -> VmResult<Value> {
        if r.is_local() {
            let lo = self.local[r.index() as usize].clone();
            self.local.push(lo);
            let c = ObjRef::local((self.local.len() - 1) as u32);
            return Ok(Value::Obj(c));
        }
        let mut map: Vec<(S::Obj, S::Obj)> = Vec::new();
        let src = S::Obj::from_u32(r.index());
        let copy = match self.copy_rec(src, deep, &mut map) {
            Ok(c) => c,
            Err(e) => return Ok(self.swallow(model_error_text(&e))),
        };
        // copyReferences
        let meta = &self.prog.ir.meta;
        for &(o, c) in &map {
            let class = self.store.class_of(o);
            let feats: Vec<FeatureId> = meta.class(class).all_features.to_vec();
            for f in feats {
                let fi = meta.feature(f);
                if !fi.is_reference() || fi.containment || !fi.changeable || fi.derived {
                    continue;
                }
                if meta.is_container(f) || !self.store.is_set(o, f) {
                    continue;
                }
                let map_target = |t: S::Obj| -> Option<S::Obj> {
                    match map.iter().find(|(x, _)| *x == t) {
                        Some((_, y)) => Some(*y),
                        None => fi.opposite.is_none().then_some(t),
                    }
                };
                if fi.is_many() {
                    let n = self.store.len(o, f);
                    let targets: Vec<S::Obj> = (0..n)
                        .filter_map(|i| self.store.get_at(o, f, i).as_obj())
                        .collect();
                    for t in targets {
                        if let Some(ct) = map_target(t) {
                            let _ = self.store.add(c, f, Datum::Obj(ct), None);
                        }
                    }
                } else if let Some(t) = self.store.get(o, f).as_obj()
                    && let Some(ct) = map_target(t)
                {
                    let _ = self.store.set(c, f, Datum::Obj(ct));
                }
            }
        }
        self.clone_interm_props(&map);
        let cr = ObjRef::host(copy.to_u32());
        let class = self.store.class_of(copy);
        let x = self.default_extent(class, None);
        if !deep {
            let contents: Vec<S::Obj> = self.store.all_contents(copy);
            for o in contents {
                self.put_to_extent(ObjRef::host(o.to_u32()), x)?;
            }
        }
        self.put_to_extent(cr, x)?;
        Ok(Value::Obj(cr))
    }

    /// `IntermediatePropertyCopier.cloneIntermediateProperties` (bug 254962): the contextual
    /// intermediate properties of every copied object, objects mapped to their copies,
    /// collections and tuples cloned, other values shared.
    fn clone_interm_props(&mut self, map: &[(S::Obj, S::Obj)]) {
        let prog = self.prog;
        for &(o, c) in map {
            let src = Value::Obj(ObjRef::host(o.to_u32()));
            let dst = Value::Obj(ObjRef::host(c.to_u32()));
            for (i, ip) in prog.ir.interm_props.iter().enumerate() {
                if !self.is_kind_of(&src, ip.context) {
                    continue;
                }
                let p = IntermPropId(i as u32);
                let v = self.get_interm(&src, p);
                let r = self.clone_ocl(&v, map);
                self.set_interm(&dst, p, r, true);
            }
        }
    }

    fn clone_ocl(&self, v: &Value, map: &[(S::Obj, S::Obj)]) -> Value {
        match v {
            Value::Obj(r) if !r.is_local() => {
                let h = S::Obj::from_u32(r.index());
                match map.iter().find(|(x, _)| *x == h) {
                    Some((_, y)) => Value::Obj(ObjRef::host(y.to_u32())),
                    None => v.clone(),
                }
            }
            Value::Coll(_) | Value::List(_) => {
                let mut out = self.new_coll_like(v);
                for e in crate::vm::Iter::of(v).items.iter() {
                    push(&mut out, self.clone_ocl(e, map), &self.env);
                }
                out
            }
            Value::Dict(d) => {
                let out = self.new_coll_like(v);
                if let Value::Dict(o) = &out {
                    for (k, x) in d.borrow().iter() {
                        o.borrow_mut().put(k.clone(), self.clone_ocl(x, map), &self.env);
                    }
                }
                out
            }
            Value::Tuple(t) => Value::Tuple(Rc::new(qvto_runtime::tuple::Tuple::new(
                t.type_id,
                t.names.clone(),
                t.values.iter().map(|x| self.clone_ocl(x, map)).collect(),
            ))),
            _ => v.clone(),
        }
    }

    fn copy_rec(
        &mut self,
        o: S::Obj,
        deep: bool,
        map: &mut Vec<(S::Obj, S::Obj)>,
    ) -> Result<S::Obj, ModelError> {
        let class = self.store.class_of(o);
        let c = self.store.create(class)?;
        map.push((o, c));
        let meta = &self.prog.ir.meta;
        let feats: Vec<FeatureId> = meta.class(class).all_features.to_vec();
        for f in feats {
            let fi = meta.feature(f);
            if !fi.changeable || fi.derived || !self.store.is_set(o, f) {
                continue;
            }
            if fi.is_attribute() {
                if fi.is_many() {
                    let n = self.store.len(o, f);
                    let vals: Vec<OwnedDatum<S::Obj>> = (0..n)
                        .map(|i| self.store.get_at(o, f, i).to_owned())
                        .collect();
                    for v in &vals {
                        self.store.add(c, f, v.as_datum(), None)?;
                    }
                } else {
                    let v = self.store.get(o, f).to_owned();
                    self.store.set(c, f, v.as_datum())?;
                }
            } else if fi.containment && deep {
                if fi.is_many() {
                    let n = self.store.len(o, f);
                    let kids: Vec<S::Obj> = (0..n)
                        .filter_map(|i| self.store.get_at(o, f, i).as_obj())
                        .collect();
                    for k in kids {
                        let kc = self.copy_rec(k, deep, map)?;
                        self.store.add(c, f, Datum::Obj(kc), None)?;
                    }
                } else if let Some(k) = self.store.get(o, f).as_obj() {
                    let kc = self.copy_rec(k, deep, map)?;
                    self.store.set(c, f, Datum::Obj(kc))?;
                }
            }
        }
        Ok(c)
    }

    // ---- metamodel operations ---------------------------------------------------------------

    /// Operations of metamodels the VM knows (Ecore's `EObject` operations).
    pub(crate) fn call_meta(&mut self, m: qvto_sema::ir::MetaOpId, at: usize) -> VmResult<Value> {
        let mo = &self.prog.ir.meta_ops[m.index()];
        let src = self.regs[at].clone();
        let name = mo.name.as_str();
        let Value::Obj(r) = src else {
            return Ok(Value::Invalid);
        };
        if r.is_local() {
            return Ok(match name {
                "eContainer" => Value::Null,
                _ => Value::Invalid,
            });
        }
        let h = S::Obj::from_u32(r.index());
        Ok(match name {
            "eContainer" => match self.store.container(h) {
                Some((p, _)) => Value::Obj(ObjRef::host(p.to_u32())),
                None => Value::Null,
            },
            "eContents" => {
                let items: Vec<Value> = self
                    .store
                    .contents(h)
                    .into_iter()
                    .map(|c| Value::Obj(ObjRef::host(c.to_u32())))
                    .collect();
                Value::Coll(Rc::new(Coll::from_values(
                    qvto_runtime::CollKind::OrderedSet,
                    items,
                    &self.env,
                )))
            }
            "eAllContents" => {
                let items: Vec<Value> = self
                    .store
                    .all_contents(h)
                    .into_iter()
                    .map(|c| Value::Obj(ObjRef::host(c.to_u32())))
                    .collect();
                Value::Coll(Rc::new(Coll::Sequence(items)))
            }
            "eIsProxy" => Value::Bool(self.store.is_proxy(h)),
            "eContainingFeature" | "eContainmentFeature" | "eClass" | "eResource" => {
                return Err(self
                    .runtime_error(format!("metamodel operation '{name}' is not supported yet")));
            }
            _ => {
                return Err(self
                    .runtime_error(format!("metamodel operation '{name}' is not supported yet")));
            }
        })
    }
}

pub(crate) struct FeatureType {
    many: bool,
    kind: CollKind,
    num: Option<NumClass>,
    primitive: bool,
    is_ref: bool,
    upper: i32,
}

pub(crate) fn model_error_text(e: &ModelError) -> String {
    match e {
        ModelError::TypeMismatch { .. } => format!("ClassCastException: {e}"),
        ModelError::IndexOutOfBounds { .. } => format!("IndexOutOfBoundsException: {e}"),
        _ => format!("IllegalArgumentException: {e}"),
    }
}

#[allow(dead_code)]
fn new_dict(env: &dyn qvto_runtime::HashEnv) -> Value {
    Value::Dict(Rc::new(RefCell::new(Dict::new(env))))
}

/// The owner of a contextual intermediate property value: an object, a module instance
/// (`intermediate property M::p` on a transformation) or a model extent (on a model type).
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub(crate) enum IntermOwner {
    Obj(ObjRef),
    Module(u32),
    Model(u32),
}

impl IntermOwner {
    fn of(v: &Value) -> Option<IntermOwner> {
        match v {
            Value::Obj(r) => Some(IntermOwner::Obj(*r)),
            Value::Instance(m) => Some(IntermOwner::Module(*m)),
            Value::Model(m) => Some(IntermOwner::Model(*m)),
            _ => None,
        }
    }
}

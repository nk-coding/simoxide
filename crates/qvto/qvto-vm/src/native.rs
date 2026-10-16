//! Native blackbox implementations: operations and transformations that the host implements in
//! Rust, as Eclipse runs Java blackbox libraries (`JavaMethodHandlerFactory`,
//! `QvtOperationalEvaluationVisitorImpl.doVisitBlackboxOperation`, the transformation handler
//! of `createTransformationHandler`).
//!
//! The analyser knows a blackbox library by its signatures (`qvto_sema::BlackboxUnit`, the
//! modules and operations Eclipse derives from a Java class); each operation carries an
//! implementation key ([`qvto_sema::ir::Operation::blackbox_impl`], for Java libraries the
//! method `pkg.Class#name(java.lang.String, ...)`). The host registers a function per key in
//! [`Natives`] and gives the registry to the run ([`Vm::set_natives`]). Blackbox
//! transformations (`blackbox transformation T(...);`) are registered by transformation name.
//!
//! A native function gets the VM (`&mut Vm`, with the `native_*` methods below for logging,
//! configuration properties, model access and per-run library state) and the call: `self`
//! (the contextual receiver, a constructor's new object, a transformation instance) and the
//! arguments, `invalid` already turned into `null` as Eclipse does before reflection.
//!
//! Eclipse's call protocol, reproduced by the VM:
//! - an operation of a Java library is called like an OCL operation (no QVT stack frame, no
//!   mapping protocol); on a `null` or `invalid` receiver the call is `invalid`;
//! - a `blackbox` operation declared in QVTo runs as an imperative operation whose body is the
//!   native function; a `blackbox mapping` keeps the mapping protocol (`when`, trace lookup, the
//!   result must be defined, trace record, merged mappings, the result's extent);
//! - an exception of the implementation becomes a `QvtRuntimeException`
//!   `Java method invocation error '<method>'` + `\nCaused by: <class>[: <message>]`;
//!   cancellation (`OperationCanceledException`, `IContext.getMonitor().cancel()`) interrupts.

use crate::error::{ErrorKind, Unwind, VmResult};
use crate::vm::{ObjHandle, Vm};
use qvto_model::{ClassId, Datum, FeatureId, ModelStore};
use qvto_runtime::coll::{Coll, CollKind};
use qvto_runtime::{ObjRef, Value};
use qvto_sema::ir::{Direction, ModuleKind, OpId};
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::rc::Rc;

/// The receiver and arguments of a native call.
#[derive(Clone, Debug, Default)]
pub struct NativeCall {
    /// The operation called (`None` for a blackbox transformation).
    pub op: Option<OpId>,
    /// The contextual receiver, a constructor's new object, or the transformation instance;
    /// `null` for module operations.
    pub self_: Value,
    /// The arguments (`invalid` passed as `null`); for a transformation: its model extents
    /// (`Value::Model`) in signature order.
    pub args: Vec<Value>,
}

/// Why a native function failed.
#[derive(Clone, Debug)]
pub enum NativeError {
    /// A (Java) exception thrown by the implementation.
    Exception {
        class: String,
        message: Option<String>,
    },
    /// The execution was cancelled (`OperationCanceledException`).
    Canceled,
    /// A QVT exception from VM calls made by the implementation.
    Unwind(Unwind),
}

impl NativeError {
    /// A `java.lang.RuntimeException` (or another class) with a message.
    pub fn exception(class: &str, message: impl Into<String>) -> NativeError {
        NativeError::Exception {
            class: class.to_string(),
            message: Some(message.into()),
        }
    }

    /// A `NullPointerException` (Java code dereferencing a `null` argument).
    pub fn npe() -> NativeError {
        NativeError::Exception {
            class: "java.lang.NullPointerException".into(),
            message: None,
        }
    }
}

impl From<Unwind> for NativeError {
    fn from(u: Unwind) -> NativeError {
        NativeError::Unwind(u)
    }
}

pub type NativeResult = Result<Value, NativeError>;

/// A native implementation.
pub type NativeFn<S> = Rc<dyn Fn(&mut Vm<'_, S>, NativeCall) -> NativeResult>;

/// A registered implementation: the text Eclipse shows for it in invocation errors (Java's
/// `Method.toString()`) and the function.
pub struct Native<S: ModelStore>
where
    S::Obj: ObjHandle,
{
    pub display: Rc<str>,
    pub f: NativeFn<S>,
}

impl<S: ModelStore> Clone for Native<S>
where
    S::Obj: ObjHandle,
{
    fn clone(&self) -> Self {
        Native {
            display: self.display.clone(),
            f: self.f.clone(),
        }
    }
}

/// The host's native blackbox implementations, by implementation key (operations) and by
/// name (transformations).
pub struct Natives<S: ModelStore>
where
    S::Obj: ObjHandle,
{
    ops: HashMap<String, Native<S>>,
    transformations: HashMap<String, Native<S>>,
}

impl<S: ModelStore> Default for Natives<S>
where
    S::Obj: ObjHandle,
{
    fn default() -> Self {
        Natives {
            ops: HashMap::new(),
            transformations: HashMap::new(),
        }
    }
}

impl<S: ModelStore> Clone for Natives<S>
where
    S::Obj: ObjHandle,
{
    fn clone(&self) -> Self {
        Natives {
            ops: self.ops.clone(),
            transformations: self.transformations.clone(),
        }
    }
}

/// The implementation key of a Java method from its `Method.toString()` text
/// (`public static java.lang.String pkg.Class.name(java.lang.String,int)` gives
/// `pkg.Class#name(java.lang.String,int)`), the form of `Operation::blackbox_impl`.
pub fn java_method_key(method: &str) -> String {
    let open = method.find('(').unwrap_or(method.len());
    let close = method[open..].find(')').map_or(method.len(), |i| open + i + 1);
    let head = &method[..open];
    let qualified = head.rsplit(' ').next().unwrap_or(head);
    let (class, name) = qualified.rsplit_once('.').unwrap_or(("", qualified));
    format!("{class}#{name}{}", &method[open..close])
}

impl<S: ModelStore> Natives<S>
where
    S::Obj: ObjHandle,
{
    pub fn new() -> Natives<S> {
        Natives::default()
    }

    /// Registers the implementation of the operations whose `blackbox_impl` is `key`.
    pub fn op(
        &mut self,
        key: &str,
        f: impl Fn(&mut Vm<'_, S>, NativeCall) -> NativeResult + 'static,
    ) -> &mut Self {
        self.ops.insert(
            key.to_string(),
            Native {
                display: Rc::from(key),
                f: Rc::new(f),
            },
        );
        self
    }

    /// Registers the implementation of a Java method given by its `Method.toString()` (the
    /// text of invocation errors); the key is derived with [`java_method_key`].
    pub fn java(
        &mut self,
        method: &str,
        f: impl Fn(&mut Vm<'_, S>, NativeCall) -> NativeResult + 'static,
    ) -> &mut Self {
        self.ops.insert(
            java_method_key(method),
            Native {
                display: Rc::from(method),
                f: Rc::new(f),
            },
        );
        self
    }

    /// Registers the implementation of `blackbox transformation <name>(...)`.
    pub fn transformation(
        &mut self,
        name: &str,
        display: &str,
        f: impl Fn(&mut Vm<'_, S>, NativeCall) -> NativeResult + 'static,
    ) -> &mut Self {
        self.transformations.insert(
            name.to_string(),
            Native {
                display: Rc::from(display),
                f: Rc::new(f),
            },
        );
        self
    }

    /// Adds every implementation of `other` (later registrations win).
    pub fn extend(&mut self, other: &Natives<S>) -> &mut Self {
        for (k, v) in &other.ops {
            self.ops.insert(k.clone(), v.clone());
        }
        for (k, v) in &other.transformations {
            self.transformations.insert(k.clone(), v.clone());
        }
        self
    }

    pub fn get(&self, key: &str) -> Option<&Native<S>> {
        self.ops.get(key)
    }

    pub fn get_transformation(&self, name: &str) -> Option<&Native<S>> {
        self.transformations.get(name)
    }

    pub fn len(&self) -> usize {
        self.ops.len() + self.transformations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The VM's native state of a run.
pub(crate) struct NativeState<S: ModelStore>
where
    S::Obj: ObjHandle,
{
    pub natives: Option<Rc<Natives<S>>>,
    /// Per-run library instances (Eclipse keeps one Java object per class and transformation).
    pub states: HashMap<TypeId, Box<dyn Any>>,
    /// `IContext.getMonitor().cancel()` was called.
    pub canceled: bool,
    /// Configuration properties set for a blackbox transformation call
    /// (`makeBlackboxTransformationArgs`).
    pub config: Vec<(String, Value)>,
}

impl<S: ModelStore> Default for NativeState<S>
where
    S::Obj: ObjHandle,
{
    fn default() -> Self {
        NativeState {
            natives: None,
            states: HashMap::new(),
            canceled: false,
            config: Vec::new(),
        }
    }
}

impl<S: ModelStore> Vm<'_, S>
where
    S::Obj: ObjHandle,
{
    /// The native blackbox implementations of this run.
    pub fn set_natives(&mut self, natives: Rc<Natives<S>>) {
        self.native.natives = Some(natives);
    }

    /// An operation of a native (Java) library module: called like an OCL operation.
    pub(crate) fn is_library_native(&self, op: OpId) -> bool {
        let o = self.prog.op(op);
        let m = &self.prog.ir.modules[o.module.index()];
        o.is_blackbox && m.blackbox && m.kind == ModuleKind::Library
    }

    fn lookup_native(&self, op: OpId) -> Option<Native<S>> {
        let key = self.prog.op(op).blackbox_impl.as_deref()?;
        self.native.natives.as_ref()?.get(key).cloned()
    }

    fn no_blackbox(&self, op: OpId) -> Unwind {
        let o = self.prog.op(op);
        let name = match o.context {
            Some(c) => format!("{}::{}", crate::types::type_simple_name(self.prog, c), o.name),
            None => o.name.clone(),
        };
        self.runtime_error(format!("No blackbox implementation found for '{name}'"))
    }

    /// Calls the native implementation of a blackbox operation (helper, query, constructor,
    /// library operation) with `self` and the arguments.
    pub(crate) fn call_native(&mut self, op: OpId, src: Value, args: Vec<Value>) -> VmResult<Value> {
        let Some(n) = self.lookup_native(op) else {
            return Err(self.no_blackbox(op));
        };
        let args = args
            .into_iter()
            .map(|a| if a.is_invalid() { Value::Null } else { a })
            .collect();
        let call = NativeCall {
            op: Some(op),
            self_: src,
            args,
        };
        self.invoke_native(&n, call)
    }

    fn invoke_native(&mut self, n: &Native<S>, call: NativeCall) -> VmResult<Value> {
        let r = (n.f)(self, call);
        if self.native.canceled {
            self.native.canceled = false;
            return Err(self.error(ErrorKind::Interrupted, None));
        }
        match r {
            Ok(v) => Ok(v),
            Err(NativeError::Unwind(u)) => Err(u),
            Err(NativeError::Canceled) => Err(self.error(ErrorKind::Interrupted, None)),
            Err(NativeError::Exception { class, message }) => {
                let cause = match message {
                    Some(m) => format!("{class}: {m}"),
                    None => class,
                };
                Err(self.runtime_error(format!(
                    "Java method invocation error '{}'\nCaused by: {cause}",
                    n.display
                )))
            }
        }
    }

    /// The registers `at..at + n` as arguments.
    pub(crate) fn native_args(&self, op: OpId, at: usize) -> Vec<Value> {
        let n = self.prog.op(op).params.len();
        (0..n).map(|i| self.regs[at + i].clone()).collect()
    }

    /// `executeMapping` of a `blackbox mapping` declared in QVTo, after the trace lookup: the
    /// native result (must be defined), `result`, the trace record, merged mappings.
    pub(crate) fn native_mapping_body(
        &mut self,
        op: OpId,
        src: &Value,
        args: &mut [Value],
        base: usize,
    ) -> VmResult<Value> {
        let o = self.prog.op(op);
        let self_ = if o.context.is_some() {
            src.clone()
        } else {
            Value::Null
        };
        let in_args: Vec<Value> = o
            .params
            .iter()
            .zip(args.iter())
            .map(|(p, a)| {
                if p.direction == Direction::Out {
                    Value::Null
                } else {
                    a.clone()
                }
            })
            .collect();
        let v = self.call_native(op, self_, in_args)?;
        if v.is_undefined() {
            return Err(self.runtime_error(format!(
                "Blackbox operation '{}' failed to assign mapping result",
                o.name
            )));
        }
        if let Some(rl) = o.result_local {
            self.regs[base + rl.index()] = v.clone();
        }
        if let Some(p) = o.results.first() {
            self.regs[base + p.local.index()] = v.clone();
        }
        Ok(v)
    }

    /// The handler of a `blackbox transformation`: the native implementation registered under
    /// the transformation's name, called with the instance and its model extents; the
    /// transformation's configuration properties become the context's.
    pub(crate) fn call_native_transformation(
        &mut self,
        module: qvto_sema::ir::ModuleId,
        instance: Value,
    ) -> VmResult<Value> {
        let m = &self.prog.ir.modules[module.index()];
        let name = m.name.clone();
        let native = self
            .native
            .natives
            .as_ref()
            .and_then(|n| n.get_transformation(&name).cloned());
        let Some(n) = native else {
            return Err(self.runtime_error(format!(
                "No blackbox implementation found for '{name}'"
            )));
        };
        let mut args = Vec::with_capacity(m.model_params.len());
        for i in 0..m.model_params.len() {
            match self.param_extents.get(&(module.0, i as u32)) {
                Some(&x) => args.push(Value::Model(x as u32)),
                None => args.push(Value::Null),
            }
        }
        let mut config = Vec::new();
        for &g in &m.globals {
            let gl = &self.prog.ir.globals[g.index()];
            if gl.kind == qvto_sema::ir::GlobalKind::Config {
                config.push((gl.name.clone(), self.globals[g.index()].clone()));
            }
        }
        let saved = std::mem::replace(&mut self.native.config, config);
        let r = self.invoke_native(
            &n,
            NativeCall {
                op: None,
                self_: instance,
                args,
            },
        );
        self.native.config = saved;
        r
    }

    // ---- the API for native implementations -------------------------------------------------

    /// The library instance of type `T` of this run (created with `Default` on first use).
    pub fn native_state<T: Default + 'static>(&mut self) -> &mut T {
        self.native
            .states
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(T::default()))
            .downcast_mut::<T>()
            .expect("native state type")
    }

    /// `IContext.getLog().log(message)`.
    pub fn native_log(&mut self, message: &str) {
        self.log_line(message);
    }

    /// `IContext.getMonitor().cancel()`: the execution is interrupted when the native call
    /// returns.
    pub fn native_cancel(&mut self) {
        self.native.canceled = true;
    }

    /// `IContext.getConfigProperty(name)`: a property set for a blackbox transformation call,
    /// else the run's configuration (the launcher's string value).
    pub fn native_config(&self, name: &str) -> Option<Value> {
        if let Some((_, v)) = self.native.config.iter().find(|(k, _)| k == name) {
            return Some(v.clone());
        }
        self.opts
            .config
            .iter()
            .find(|(k, _)| k == name)
            .and_then(|(_, v)| v.as_deref().map(Value::str))
    }

    /// `String.valueOf(v)` as Java prints values (EMF's `toString` of objects).
    pub fn native_to_string(&self, v: &Value) -> String {
        match v {
            Value::Null => "null".into(),
            v => self.to_java_string(v, false),
        }
    }

    /// The class of an object.
    pub fn native_class(&self, v: &Value) -> Option<ClassId> {
        self.class_of(v)
    }

    /// The class `name` of the package with namespace URI `ns_uri`.
    pub fn native_class_by_name(&self, ns_uri: &str, name: &str) -> Option<ClassId> {
        let meta = &self.prog.ir.meta;
        let p = meta.package_by_ns_uri(ns_uri)?;
        match meta.classifier_by_name(p, name)? {
            qvto_model::Classifier::Class(c) => Some(c),
            _ => None,
        }
    }

    /// Whether an object is an instance of the class (or a subclass).
    pub fn native_is_instance(&self, v: &Value, class: ClassId) -> bool {
        self.value_conforms_to_class(v, class)
    }

    /// A new object of a class, in no extent and no resource (`EcoreUtil.create`, a factory's
    /// `create...()`).
    pub fn native_create(&mut self, class: ClassId) -> Result<Value, NativeError> {
        if self.is_local_class(class) {
            return Ok(Value::Obj(self.new_local(class)));
        }
        match self.store.create(class) {
            Ok(h) => Ok(Value::Obj(ObjRef::host(h.to_u32()))),
            Err(e) => Err(NativeError::exception(
                "java.lang.IllegalArgumentException",
                e.to_string(),
            )),
        }
    }

    fn native_feature(&self, v: &Value, name: &str) -> Result<FeatureId, NativeError> {
        let Some(c) = self.class_of(v) else {
            return Err(NativeError::npe());
        };
        let meta = &self.prog.ir.meta;
        meta.class(c)
            .all_features
            .iter()
            .copied()
            .find(|&f| meta.feature(f).name.as_ref() as &str == name)
            .ok_or_else(|| {
                NativeError::exception(
                    "java.lang.IllegalArgumentException",
                    format!("The feature '{name}' is not a valid feature"),
                )
            })
    }

    /// `eGet` of the feature `name` (a collection for many-valued features).
    pub fn native_get(&mut self, v: &Value, name: &str) -> Result<Value, NativeError> {
        let f = self.native_feature(v, name)?;
        Ok(self.get_prop(v, f)?)
    }

    /// `eSet` of a single-valued feature (`null` unsets).
    pub fn native_set(&mut self, v: &Value, name: &str, value: Value) -> Result<(), NativeError> {
        let f = self.native_feature(v, name)?;
        let Value::Obj(r) = v else {
            return Err(NativeError::npe());
        };
        if r.is_local() {
            return Ok(self.set_prop(v, f, value, true)?);
        }
        let h = S::Obj::from_u32(r.index());
        let res = if value.is_null() {
            self.store.unset(h, f)
        } else {
            match self.to_datum(&value) {
                Some(d) => self.store.set(h, f, d.as_datum()),
                None => {
                    return Err(NativeError::exception(
                        "java.lang.ClassCastException",
                        "value is not a feature value",
                    ));
                }
            }
        };
        res.map_err(|e| {
            NativeError::exception("java.lang.IllegalArgumentException", e.to_string())
        })
    }

    /// `((EList) eGet(feature)).add(value)`.
    pub fn native_add(&mut self, v: &Value, name: &str, value: Value) -> Result<(), NativeError> {
        let f = self.native_feature(v, name)?;
        let Value::Obj(r) = v else {
            return Err(NativeError::npe());
        };
        if r.is_local() {
            return Ok(self.set_prop(v, f, value, false)?);
        }
        let h = S::Obj::from_u32(r.index());
        let Some(d) = self.to_datum(&value) else {
            return Err(NativeError::exception(
                "java.lang.ClassCastException",
                "value is not a feature value",
            ));
        };
        self.store
            .add(h, f, d.as_datum(), None)
            .map(|_| ())
            .map_err(|e| NativeError::exception("java.lang.IllegalArgumentException", e.to_string()))
    }

    /// `obj.eResource().getContents().add(root)`; `false` if `obj` has no resource.
    pub fn native_add_to_resource_of(&mut self, obj: &Value, root: &Value) -> bool {
        let (Value::Obj(o), Value::Obj(x)) = (obj, root) else {
            return false;
        };
        if o.is_local() || x.is_local() {
            return false;
        }
        let Some(r) = self.store.resource_of(S::Obj::from_u32(o.index())) else {
            return false;
        };
        let _ = self.store.add_root(r, S::Obj::from_u32(x.index()), None);
        true
    }

    /// Whether the object is in a resource (`eResource() != null`).
    pub fn native_has_resource(&self, obj: &Value) -> bool {
        match obj {
            Value::Obj(o) if !o.is_local() => self
                .store
                .resource_of(S::Obj::from_u32(o.index()))
                .is_some(),
            _ => false,
        }
    }

    /// The elements of a collection, `List` or `Dict` (its values), in iteration order.
    pub fn native_items(&self, v: &Value) -> Vec<Value> {
        crate::vm::Iter::of(v).items.to_vec()
    }

    /// A collection of the given kind (Java's `HashSet`, `LinkedHashSet`, `BagImpl`,
    /// `ArrayList`) with the items in order.
    pub fn native_collection(&self, kind: CollKind, items: Vec<Value>) -> Value {
        match kind {
            CollKind::Sequence => Value::Coll(Rc::new(Coll::Sequence(items))),
            k => Value::Coll(Rc::new(Coll::from_values(k, items, &self.env))),
        }
    }

    /// A new QVTo `List` (`MutableListImpl`) with the items.
    pub fn native_list(&self, items: Vec<Value>) -> Value {
        let l = self.new_coll(qvto_bytecode::insn::coll_kind::LIST);
        if let Value::List(lr) = &l {
            lr.borrow_mut().items.extend(items);
        }
        l
    }

    /// A new, empty QVTo `Dict` (`Utils.createDictionary()`).
    pub fn native_dict(&self) -> Value {
        self.new_coll(qvto_bytecode::insn::coll_kind::DICT)
    }

    /// The keys of a `Dict` in its iteration order.
    pub fn native_dict_keys(&self, v: &Value) -> Vec<Value> {
        match v {
            Value::Dict(d) => d.borrow().keys(),
            _ => Vec::new(),
        }
    }

    /// `dict.put(key, value)`.
    pub fn native_dict_put(&self, d: &Value, key: Value, value: Value) {
        if let Value::Dict(d) = d {
            d.borrow_mut().put(key, value, &self.env);
        }
    }

    /// `dict.get(key)`.
    pub fn native_dict_get(&self, d: &Value, key: &Value) -> Value {
        match d {
            Value::Dict(d) => d.borrow().get(key, &self.env).cloned().unwrap_or(Value::Null),
            _ => Value::Null,
        }
    }

    /// `JavaModelExtent.getRootObjects()` of a model parameter (`Value::Model`).
    pub fn native_model_roots(&self, model: &Value) -> Vec<Value> {
        match model {
            Value::Model(x) => self.extents[*x as usize]
                .root_objects(&*self.store)
                .into_iter()
                .map(Value::Obj)
                .collect(),
            _ => Vec::new(),
        }
    }

    /// `JavaModelExtent.addObject(obj)`.
    pub fn native_model_add(&mut self, model: &Value, obj: &Value) {
        if let (Value::Model(x), Value::Obj(o)) = (model, obj) {
            let mut next = self.next_extent_uri;
            self.extents[*x as usize].add_object(self.store, *o, &mut next);
            self.next_extent_uri = next;
        }
    }

    /// The value of a single-valued attribute as the store keeps it.
    pub fn native_attr(&self, v: &Value, f: FeatureId) -> Value {
        match v {
            Value::Obj(r) if !r.is_local() => {
                let d: Datum<'_, S::Obj> = self.store.get(S::Obj::from_u32(r.index()), f);
                self.datum(d)
            }
            _ => Value::Null,
        }
    }
}

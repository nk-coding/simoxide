//! Incremental update with the trace of a previous run (`QVTEvaluationOptions
//! .INCREMENTAL_UPDATE_TRACE`, `TraceUtil.getIncrementalTraceRecord`): a mapping call that finds
//! no record in the run's own trace looks for a record of the input trace with the same
//! mapping, source and in-parameters; if its parameter and result types match the mapping,
//! the mapping starts with the recorded results and `out` parameters (put into their extents)
//! instead of new objects, and its body runs as usual.
//!
//! The host reads the trace model (`qvto::trace::read_incremental_trace`) into
//! [`IncrementalTrace`]; model elements are store objects of the run.

use crate::error::VmResult;
use crate::vm::{ObjHandle, Vm, push};
use qvto_model::ModelStore;
use qvto_runtime::{ObjRef, Value};
use qvto_sema::ir::{Direction, OpId, OpKind};

/// A value of a trace record (`EValue`).
#[derive(Clone, Debug, PartialEq)]
pub enum TraceValue {
    Null,
    /// A model element (a store object of the run); unresolved proxies are `Null`.
    Obj(u32),
    Str(String),
    Bool(bool),
    Int(i64),
    Real(f64),
    Tuple(Vec<(String, TraceValue)>),
    /// `Set`, `OrderedSet`, `Bag`, `Sequence`, `List` and the elements.
    Coll(String, Vec<TraceValue>),
    Dict(Vec<(TraceValue, TraceValue)>),
}

/// `VarParameterValue`: a context, parameter or result of a record.
#[derive(Clone, Debug)]
pub struct TraceParam {
    pub name: String,
    pub kind: Direction,
    /// The type's name as Eclipse wrote it (`EPackage`, `String`, `Tuple(a : T, ...)`).
    pub type_name: String,
    pub value: TraceValue,
}

/// A `TraceRecord` of the input trace.
#[derive(Clone, Debug)]
pub struct TraceRecordIn {
    /// `EMappingOperation`: the mapping's name, its module's namespace prefix and name.
    pub mapping: String,
    pub package: String,
    pub module: String,
    pub context: Option<TraceParam>,
    pub params: Vec<TraceParam>,
    pub results: Vec<TraceParam>,
}

/// The records of an input trace (`Trace.traceRecords`).
#[derive(Clone, Debug, Default)]
pub struct IncrementalTrace {
    pub records: Vec<TraceRecordIn>,
}

/// A record bound to the program's mapping.
pub(crate) struct IncRecord {
    op: OpId,
    /// The source key (`createKey`): the context alone, or the context and the non-`out`
    /// parameters.
    key: Vec<Value>,
    rec: TraceRecordIn,
}

impl<S: ModelStore> Vm<'_, S>
where
    S::Obj: ObjHandle,
{
    /// Sets the input trace (`resolveTraces` at the start of the run): records bound to the
    /// mappings by name and module (`getMappingKey`), recorded output model elements detached
    /// from their resources ("should operate like a new object").
    pub fn set_incremental_trace(&mut self, trace: IncrementalTrace) {
        let ir = &self.prog.ir;
        let mut out = Vec::new();
        for rec in trace.records {
            let found = ir.operations.iter().enumerate().rev().find(|(_, o)| {
                o.kind == OpKind::Mapping
                    && o.name == rec.mapping
                    && ir.modules[o.module.index()].name == rec.module
                    && ir.modules[o.module.index()].name == rec.package
            });
            let Some((i, _)) = found else {
                continue;
            };
            for p in rec.params.iter().chain(rec.results.iter()) {
                if p.kind == Direction::Out {
                    self.detach_trace_objects(&p.value);
                }
            }
            let ctx = rec
                .context
                .as_ref()
                .map(|c| self.trace_value(&c.value))
                .unwrap_or(Value::Null);
            let mut key = vec![ctx];
            for p in &rec.params {
                if p.kind != Direction::Out {
                    key.push(self.trace_value(&p.value));
                }
            }
            out.push(IncRecord {
                op: OpId(i as u32),
                key,
                rec,
            });
        }
        self.inc_trace = out;
    }

    fn detach_trace_objects(&mut self, v: &TraceValue) {
        match v {
            TraceValue::Obj(o) => {
                let h = S::Obj::from_u32(*o);
                if let Some(r) = self.store.direct_resource(h) {
                    self.store.remove_root(r, h);
                }
            }
            TraceValue::Tuple(ps) => ps.iter().for_each(|(_, x)| self.detach_trace_objects(x)),
            TraceValue::Coll(_, xs) => xs.iter().for_each(|x| self.detach_trace_objects(x)),
            TraceValue::Dict(kv) => kv.iter().for_each(|(k, x)| {
                self.detach_trace_objects(k);
                self.detach_trace_objects(x);
            }),
            _ => {}
        }
    }

    /// `createOclObjectFromValue` (tuples are read part by part, see `fetch`).
    fn trace_value(&self, v: &TraceValue) -> Value {
        use qvto_bytecode::insn::coll_kind as k;
        match v {
            TraceValue::Null | TraceValue::Tuple(_) => Value::Null,
            TraceValue::Obj(o) => Value::Obj(ObjRef::host(*o)),
            TraceValue::Str(s) => Value::str(s),
            TraceValue::Bool(b) => Value::Bool(*b),
            TraceValue::Int(i) => Value::Int(*i),
            TraceValue::Real(r) => Value::Real(*r),
            TraceValue::Coll(kind, xs) => {
                let mut c = match kind.as_str() {
                    "Set" => self.new_coll(k::SET),
                    "OrderedSet" => self.new_coll(k::ORDERED_SET),
                    "Bag" => self.new_coll(k::BAG),
                    "List" => Value::List(std::rc::Rc::new(std::cell::RefCell::new(
                        qvto_runtime::dict::List::new(),
                    ))),
                    _ => self.new_coll(k::SEQUENCE),
                };
                for x in xs {
                    push(&mut c, self.trace_value(x), &self.env);
                }
                c
            }
            TraceValue::Dict(kv) => {
                let d = qvto_runtime::dict::Dict::new(&self.env);
                let d = Value::Dict(std::rc::Rc::new(std::cell::RefCell::new(d)));
                if let Value::Dict(m) = &d {
                    for (key, x) in kv {
                        m.borrow_mut()
                            .put(self.trace_value(key), self.trace_value(x), &self.env);
                    }
                }
                d
            }
        }
    }

    /// The name of a value's type (`createOclTypeFromValue(...).getName()`) for the result
    /// match of tuple parts.
    fn trace_type_name(&self, v: &TraceValue) -> String {
        match v {
            TraceValue::Obj(o) => {
                let c = self.store.class_of(S::Obj::from_u32(*o));
                self.prog.ir.meta.class(c).name.to_string()
            }
            TraceValue::Str(_) => "String".into(),
            TraceValue::Bool(_) => "Boolean".into(),
            TraceValue::Int(_) => "Integer".into(),
            TraceValue::Real(_) => "Real".into(),
            _ => String::new(),
        }
    }

    /// `getIncrementalTraceRecord` and `fetchIncrementalResultFromTrace` for a mapping call
    /// without a record in the run's trace: sets the result and `out` variables of the frame.
    pub(crate) fn incremental_fetch(
        &mut self,
        op: OpId,
        self_val: &Value,
        params: &[(Direction, Value)],
        base: usize,
    ) -> VmResult<()> {
        if self.inc_trace.is_empty() {
            return Ok(());
        }
        let o = self.prog.op(op);
        let mut key = vec![self_val.clone()];
        key.extend(
            params
                .iter()
                .filter(|(d, _)| *d != Direction::Out)
                .map(|(_, v)| v.clone()),
        );
        let env = &self.env;
        let eq = |a: &[Value], b: &[Value]| {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(x, y)| qvto_runtime::eq::java_equals(x, y, env))
        };
        // the last record of the source wins (`addRecordBySource`)
        let Some(idx) = self
            .inc_trace
            .iter()
            .rposition(|r| r.op == op && eq(&r.key, &key))
        else {
            return Ok(());
        };
        if !self.incremental_result_match(op, idx) {
            return Ok(());
        }
        let rec = self.inc_trace[idx].rec.clone();
        // out parameters
        for (p, v) in o.params.iter().zip(rec.params.iter()) {
            if v.kind == Direction::Out {
                let val = self.trace_value(&v.value);
                if !val.is_null() {
                    self.regs[base + p.local.index()] = val.clone();
                }
                self.trace_object_to_extent(&val, p.extent)?;
            }
        }
        if o.results.is_empty() {
            return Ok(());
        }
        let Some(r) = rec.results.first() else {
            return Ok(());
        };
        if o.results.len() == 1 {
            let val = self.trace_value(&r.value);
            if !val.is_null() {
                self.regs[base + o.results[0].local.index()] = val.clone();
            }
            self.trace_object_to_extent(&val, o.results[0].extent)?;
        } else if let TraceValue::Tuple(parts) = &r.value {
            for (name, pv) in parts {
                let Some(p) = o.results.iter().find(|p| p.name == *name) else {
                    continue;
                };
                let val = self.trace_value(pv);
                if !val.is_null() {
                    self.regs[base + p.local.index()] = val.clone();
                }
                self.trace_object_to_extent(&val, p.extent)?;
            }
        }
        Ok(())
    }

    fn trace_object_to_extent(
        &mut self,
        v: &Value,
        extent: Option<qvto_sema::ir::ModelParamRef>,
    ) -> VmResult<()> {
        if let Value::Obj(r) = v {
            let class = self.store.class_of(S::Obj::from_u32(r.index()));
            let x = self.default_extent(class, extent);
            self.put_to_extent(*r, x)?;
        }
        Ok(())
    }

    /// `checkIncrementalResultMatch`: the parameter types and the result types by name.
    fn incremental_result_match(&self, op: OpId, idx: usize) -> bool {
        let o = self.prog.op(op);
        let rec = &self.inc_trace[idx].rec;
        let name = |t| crate::types::type_simple_name(self.prog, t);
        let mut it = rec.params.iter();
        for p in &o.params {
            match it.next() {
                Some(r) if r.type_name == name(p.ty) => {}
                _ => return false,
            }
        }
        if o.results.is_empty() {
            return true;
        }
        if rec.results.len() != 1 {
            return false;
        }
        let r = &rec.results[0];
        if o.results.len() == 1 {
            return r.type_name == name(o.results[0].ty);
        }
        let TraceValue::Tuple(parts) = &r.value else {
            return false;
        };
        o.results.iter().all(|p| {
            parts
                .iter()
                .find(|(n, _)| *n == p.name)
                .is_some_and(|(_, v)| self.trace_type_name(v) == name(p.ty))
        })
    }
}

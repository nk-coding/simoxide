//! Calls of imperative operations: helpers, queries, constructors, entries and mappings with
//! Eclipse's protocol (`QvtOperationalEvaluationVisitorImpl.executeImperativeOperation`,
//! `visitMappingOperation`, `executeMapping`, `visitMappingBody`), virtual dispatch and
//! overriding.

use crate::error::{ErrorKind, VmResult};
use crate::vm::{Frame, IterSlot, ObjHandle, Vm};
use qvto_bytecode::{BodyCode, ChunkId};
use qvto_model::ModelStore;
use qvto_runtime::{ObjRef, Str, Value};
use qvto_sema::ir::{Direction, OpId, OpKind};
use qvto_types::{Type, TypeId};
use std::rc::Rc;

/// Outcome of a mapping call (`MappingCallResult.Status`).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MappingStatus {
    Executed,
    PreconditionFailed,
    FetchedFromTrace,
    NoDisjunctSelected,
}

impl MappingStatus {
    pub fn failed(self) -> bool {
        matches!(
            self,
            MappingStatus::PreconditionFailed | MappingStatus::NoDisjunctSelected
        )
    }
}

pub(crate) struct MappingResult {
    pub value: Value,
    pub status: MappingStatus,
    /// The argument list after the call (out arguments replaced).
    pub args: Vec<Value>,
}

/// The values a reusing call (inherits, merges) shares with its caller: result parameters,
/// `result`, out parameters.
#[derive(Clone, Debug, Default)]
pub(crate) struct Shared {
    pub results: Vec<Value>,
    pub result_var: Option<Value>,
    pub outs: Vec<Option<Value>>,
}

impl<S: ModelStore> Vm<'_, S>
where
    S::Obj: ObjHandle,
{
    /// The runtime type of a value for virtual dispatch (`getType`).
    fn dispatch_type(&self, v: &Value) -> Option<TypeId> {
        let ir = &self.prog.ir;
        let t = match v {
            Value::Obj(_) => Type::Class(self.class_of(v)?),
            Value::Str(_) => Type::String,
            Value::Int(_) => Type::Integer,
            Value::Real(_) => Type::Real,
            Value::Bool(_) => Type::Boolean,
            // `getType(null)` is OclVoid
            Value::Null => return Some(TypeId::OCL_VOID),
            _ => return None,
        };
        ir.types.find(&t)
    }

    /// `VirtualTable.lookupActualOperation`: the operation of the actual type, else of its
    /// supertypes depth-first in declaration order, else the formal one.
    pub(crate) fn virtual_target(&self, op: OpId, src: &Value) -> OpId {
        let o = self.prog.op(op);
        if self.vtable(op).is_empty() || o.context.is_none() {
            return op;
        }
        let Some(actual) = self.dispatch_type(src) else {
            return op;
        };
        let key = (op.0, actual.0);
        if let Some(&r) = self.vcache.borrow().get(&key) {
            return OpId(r);
        }
        let r = self.lookup_actual(op, actual).unwrap_or(op);
        self.vcache.borrow_mut().insert(key, r.0);
        r
    }

    /// The virtual table of an operation: the analyser's, else (while the analyser does not
    /// fill it) the contextual operations with the same name, kind and parameter types, in
    /// program order.
    fn vtable(&self, op: OpId) -> &[OpId] {
        let o = self.prog.op(op);
        if !o.vtable.is_empty() {
            return &o.vtable;
        }
        self.vtables
            .get(op.index())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    fn lookup_actual(&self, op: OpId, actual: TypeId) -> Option<OpId> {
        let ir = &self.prog.ir;
        let o = self.prog.op(op);
        if o.context == Some(actual) {
            return Some(op);
        }
        for &c in self.vtable(op) {
            if self.prog.op(c).context == Some(actual) {
                return Some(c);
            }
        }
        if let Type::Class(cl) = ir.types.get(actual) {
            for &sup in &ir.meta.class(*cl).supers {
                if let Some(st) = ir.types.find(&Type::Class(sup))
                    && let Some(r) = self.lookup_actual(op, st)
                {
                    return Some(r);
                }
            }
        }
        None
    }

    /// The operation of an extending module that overrides `op`, if any (`extends`).
    pub(crate) fn overriding(&self, op: OpId) -> OpId {
        let ir = &self.prog.ir;
        // the main module's chain: the first operation that overrides `op`
        for (i, o) in ir.operations.iter().enumerate() {
            if o.overrides == Some(op) {
                return OpId(i as u32);
            }
        }
        op
    }

    fn check_depth(&self) -> VmResult<()> {
        let depth = self.frames.len() + 1;
        if depth >= self.opts.max_stack_depth {
            return Err(self.error(
                ErrorKind::StackOverflow,
                Some(format!("Stack depth: {depth}")),
            ));
        }
        Ok(())
    }

    /// Allocates a frame for an operation; returns `(base, ibase)`.
    fn push_frame(&mut self, op: OpId, chunk: ChunkId) -> (usize, usize) {
        let code = self.prog.code(op);
        let base = self.regs.len();
        self.regs.resize(base + code.nregs as usize, Value::Null);
        let ibase = self.iters.len();
        self.iters
            .resize(ibase + code.n_iters as usize, IterSlot::Empty);
        let o = self.prog.op(op);
        self.frames.push(Frame {
            op: Some(op),
            module: o.module,
            chunk,
            pc: 0,
            fixed_ip: None,
        });
        (base, ibase)
    }

    fn pop_frame(&mut self, base: usize, ibase: usize) {
        self.regs.truncate(base);
        self.iters.truncate(ibase);
        self.frames.pop();
    }

    fn set_chunk(&mut self, c: ChunkId) {
        if let Some(f) = self.frames.last_mut() {
            f.chunk = c;
            f.pc = 0;
        }
    }

    /// Binds `self` and the non-`out` parameters.
    fn bind(&mut self, op: OpId, base: usize, src: &Value, args: &[Value]) {
        let o = self.prog.op(op);
        if let Some(sl) = o.self_local
            && o.context.is_some()
        {
            self.regs[base + sl.index()] = src.clone();
        }
        for (p, a) in o.params.iter().zip(args.iter()) {
            if p.direction != Direction::Out {
                let v = self.list_coercion(p.ty, a.clone());
                self.regs[base + p.local.index()] = v;
            }
        }
    }

    /// A call of a helper, query, constructor or entry operation; receiver at `at`, arguments
    /// after it (temporaries, moved into the callee's frame).
    pub(crate) fn call_op(&mut self, op: OpId, at: usize, virt: bool) -> VmResult<Value> {
        let mut src = std::mem::take(&mut self.regs[at]);
        let referred = op;
        let o = self.prog.op(op);
        if let Some(ct) = o.context {
            src = self.list_coercion(ct, src);
        }
        if src.is_invalid() {
            return Ok(Value::Invalid);
        }
        let mut op = if virt {
            self.virtual_target(op, &src)
        } else {
            op
        };
        let o = self.prog.op(op);
        if o.context.is_some() && src.is_null() && o.context != Some(TypeId::OCL_VOID) {
            return Ok(Value::Invalid);
        }
        if virt {
            op = self.overriding(op);
        }
        let v = self.execute_op_at(op, src, at + 1)?;
        let rt = self.prog.op(referred).result_ty;
        Ok(self.list_coercion(rt, v))
    }

    /// Runs a non-mapping operation on a new frame with the given arguments.
    pub(crate) fn execute_op(&mut self, op: OpId, src: Value, args: Vec<Value>) -> VmResult<Value> {
        let at = self.regs.len();
        self.regs.extend(args);
        let r = self.execute_op_at(op, src, at);
        self.regs.truncate(at);
        r
    }

    /// Runs an operation on a new frame; the arguments are the registers from `args_at`.
    fn execute_op_at(&mut self, op: OpId, src: Value, args_at: usize) -> VmResult<Value> {
        self.check_depth()?;
        let o = self.prog.op(op);
        if o.is_blackbox {
            return Err(
                self.runtime_error(format!("No blackbox implementation found for '{}'", o.name))
            );
        }
        if o.kind == OpKind::Mapping {
            let n = o.params.len();
            let args: Vec<Value> = (0..n).map(|i| self.regs[args_at + i].clone()).collect();
            let r = self.execute_mapping(op, src, args, None)?;
            return Ok(r.value);
        }
        let Some(body) = self.prog.code(op).body else {
            return Ok(Value::Null);
        };
        let (base, ibase) = self.push_frame(op, body);
        if let Some(sl) = o.self_local
            && o.context.is_some()
        {
            self.regs[base + sl.index()] = src.clone();
        }
        for (i, p) in o.params.iter().enumerate() {
            if p.direction != Direction::Out {
                let a = std::mem::take(&mut self.regs[args_at + i]);
                let v = self.list_coercion(p.ty, a);
                self.regs[base + p.local.index()] = v;
            }
        }
        if o.kind == OpKind::Constructor
            && let Some(rl) = o.result_local
        {
            // result := self (Eclipse moves self to result)
            self.regs[base + rl.index()] = src;
            if let Some(sl) = o.self_local {
                self.regs[base + sl.index()] = Value::Null;
            }
        }
        let r = self.exec(body, base, ibase);
        let v = match r {
            Ok(v) => self.op_result(op, base, v),
            Err(e) => {
                self.pop_frame(base, ibase);
                return Err(e);
            }
        };
        self.pop_frame(base, ibase);
        Ok(v)
    }

    /// `visitOperationBody`'s result adjustments: void operations give null, several results
    /// a tuple.
    fn op_result(&mut self, op: OpId, base: usize, v: Value) -> Value {
        let o = self.prog.op(op);
        if o.result_ty == TypeId::OCL_VOID {
            return Value::Null;
        }
        if o.results.len() > 1 && !matches!(v, Value::Tuple(_)) {
            return self.tuple_result(op, base, false).unwrap_or(Value::Invalid);
        }
        v
    }

    /// `createTupleResult`: the tuple of the result parameters (mappings create missing objects).
    fn tuple_result(&mut self, op: OpId, base: usize, create: bool) -> VmResult<Value> {
        let o = self.prog.op(op);
        let Type::Tuple(parts) = self.prog.ir.types.get(o.result_ty).clone() else {
            return Ok(Value::Invalid);
        };
        let mut values = Vec::with_capacity(parts.len());
        for (name, pty) in parts.iter() {
            let p = o
                .results
                .iter()
                .find(|r| r.name.as_ref() as &str == name.as_ref());
            let mut v = match p {
                Some(p) => self.regs[base + p.local.index()].clone(),
                None => Value::Null,
            };
            if v.is_null() && create {
                if let Type::Class(c) = self.prog.ir.types.get(*pty).clone() {
                    v = self.create_object(c, p.and_then(|p| p.extent))?;
                } else {
                    v = self.default_value(*pty);
                }
                if let Some(p) = p {
                    self.regs[base + p.local.index()] = v.clone();
                }
            }
            values.push(v);
        }
        let names: Rc<[Str]> = parts.iter().map(|(n, _)| Str::new(n)).collect();
        Ok(Value::Tuple(Rc::new(qvto_runtime::tuple::Tuple::new(
            o.result_ty.0,
            names,
            values.into_boxed_slice(),
        ))))
    }

    pub(crate) fn call_constructor(
        &mut self,
        op: OpId,
        obj: Value,
        args: Vec<Value>,
    ) -> VmResult<()> {
        let op = self.overriding(op);
        self.execute_op(op, obj, args)?;
        Ok(())
    }

    /// A mapping call expression (`map`/`xmap`): dispatch, the strict check, the out
    /// arguments written back into the argument registers.
    pub(crate) fn call_mapping_site(
        &mut self,
        op: OpId,
        at: usize,
        strict: bool,
        virt: bool,
    ) -> VmResult<Value> {
        let mut src = self.regs[at].clone();
        let referred = op;
        let o = self.prog.op(op);
        if let Some(ct) = o.context {
            src = self.list_coercion(ct, src);
        }
        let n = o.params.len();
        let args: Vec<Value> = (0..n).map(|i| self.regs[at + 1 + i].clone()).collect();
        if src.is_invalid() {
            return Ok(Value::Invalid);
        }
        let mut op = if virt {
            self.virtual_target(op, &src)
        } else {
            op
        };
        let o = self.prog.op(op);
        if o.context.is_some() && src.is_null() && o.context != Some(TypeId::OCL_VOID) {
            return Ok(Value::Invalid);
        }
        if virt {
            op = self.overriding(op);
        }
        self.check_depth()?;
        let r = self.execute_mapping(op, src, args, None)?;
        if strict && r.status.failed() {
            let name = self.prog.op(op).name.clone();
            return Err(self.error(
                ErrorKind::AssertionFailed,
                Some(format!("Precondition failed in mapping '{name}'")),
            ));
        }
        // retrieveOutArgs: the caller writes the values back (compiled after the call)
        let params = &self.prog.op(op).params;
        for (i, p) in params.iter().enumerate() {
            if p.direction == Direction::Out
                && let Some(v) = r.args.get(i)
            {
                self.regs[at + 1 + i] = v.clone();
            }
        }
        let rt = self.prog.op(referred).result_ty;
        Ok(self.list_coercion(rt, r.value))
    }

    /// `visitMappingOperation` and `executeMapping` on a new frame. `shared` carries the
    /// caller's result and out values for reusing calls (inherited and merged mappings) and
    /// receives them back.
    pub(crate) fn execute_mapping(
        &mut self,
        op: OpId,
        src: Value,
        mut args: Vec<Value>,
        shared: Option<&mut Shared>,
    ) -> VmResult<MappingResult> {
        self.check_depth()?;
        let prog = self.prog;
        let o = prog.op(op);
        let code = prog.code(op);
        let first = code
            .when
            .or(code.init)
            .or(code.population)
            .unwrap_or(ChunkId(0));
        let (base, ibase) = self.push_frame(op, first);
        let r = self.mapping_body(op, &src, &mut args, base, ibase, shared);
        self.pop_frame(base, ibase);
        let (value, status) = r?;
        let _ = o;
        Ok(MappingResult {
            value,
            status,
            args,
        })
    }

    fn mapping_body(
        &mut self,
        op: OpId,
        src: &Value,
        args: &mut [Value],
        base: usize,
        ibase: usize,
        mut shared: Option<&mut Shared>,
    ) -> VmResult<(Value, MappingStatus)> {
        let prog = self.prog;
        let o = prog.op(op);
        let code = prog.code(op);
        self.bind(op, base, src, args);
        if let Some(sh) = shared.as_deref() {
            self.share_in(op, base, sh);
        }
        // when
        if let Some(w) = code.when {
            self.set_chunk(w);
            let ok = self.exec(w, base, ibase)?;
            if !ok.is_true() {
                return Ok((Value::Null, MappingStatus::PreconditionFailed));
            }
        }
        let (value, status) = self.mapping_execute(op, src, args, base, ibase)?;
        // where (also after a trace hit)
        if let Some(w) = code.where_ {
            self.set_chunk(w);
            let ok = self.exec(w, base, ibase)?;
            if !ok.is_true() {
                return Err(self.error(
                    ErrorKind::AssertionFailed,
                    Some(format!("Postcondition failed in mapping '{}'", o.name)),
                ));
            }
        }
        if status == MappingStatus::Executed
            && let Some(sh) = shared.as_deref_mut()
        {
            self.share_out(op, base, sh);
        }
        Ok((value, status))
    }

    fn params_of(&self, op: OpId, base: usize, args: &[Value]) -> Vec<(Direction, Value)> {
        let o = self.prog.op(op);
        o.params
            .iter()
            .zip(args.iter())
            .map(|(p, a)| {
                let v = if p.direction == Direction::Out {
                    a.clone()
                } else {
                    self.regs[base + p.local.index()].clone()
                };
                (p.direction, v)
            })
            .collect()
    }

    fn result_var(&self, op: OpId, base: usize) -> Value {
        match self.prog.op(op).result_local {
            Some(l) => self.regs[base + l.index()].clone(),
            None => Value::Null,
        }
    }

    /// `executeMapping`: trace lookup, disjuncts, body.
    fn mapping_execute(
        &mut self,
        op: OpId,
        src: &Value,
        args: &mut [Value],
        base: usize,
        ibase: usize,
    ) -> VmResult<(Value, MappingStatus)> {
        let prog = self.prog;
        let o = prog.op(op);
        let code = prog.code(op);
        let ctx = o.context.map(|_| src.clone());
        let self_val = ctx.clone().unwrap_or(Value::Null);
        let params = self.params_of(op, base, args);
        let result_now = self.result_var(op, base);
        if let Some(rec) = self.trace_lookup(op, &self_val, &params, &result_now) {
            let r = &self.traces.records[rec];
            // out arguments refreshed from the record
            for (i, (d, v)) in r.params.iter().enumerate() {
                if *d == Direction::Out
                    && let Some(a) = args.get_mut(i)
                {
                    *a = v.clone();
                }
            }
            let value = if o.results.is_empty() {
                Value::Null
            } else {
                r.result.clone().unwrap_or(Value::Null)
            };
            return Ok((value, MappingStatus::FetchedFromTrace));
        }
        let m = o.mapping.as_ref();
        if let Some(m) = m
            && !m.disjuncts.is_empty()
        {
            return self.dispatch_disjuncts(op, src, args, base);
        }
        if o.is_blackbox {
            return Err(
                self.runtime_error(format!("No blackbox implementation found for '{}'", o.name))
            );
        }
        // setupInitialResultValues
        for p in &o.results {
            if self.regs[base + p.local.index()].is_null() {
                self.regs[base + p.local.index()] = self.default_value(p.ty);
            }
        }
        if let Some(c) = code.init {
            self.set_chunk(c);
            self.exec(c, base, ibase)?;
        }
        // createOrGetResult
        for p in &o.results {
            let v = &self.regs[base + p.local.index()];
            if v.is_undefined() && p.ty != TypeId::OCL_VOID {
                let nv = self.create_instance_of(p.ty, p.extent)?;
                self.regs[base + p.local.index()] = nv;
            }
        }
        let value = if o.results.len() > 1 {
            let t = self.tuple_result(op, base, true)?;
            if let Some(rl) = o.result_local {
                self.regs[base + rl.index()] = t.clone();
            }
            t
        } else {
            self.result_var(op, base)
        };
        // createOutParams
        for p in &o.params {
            if p.direction == Direction::Out && self.regs[base + p.local.index()].is_undefined() {
                let nv = self.create_instance_of(p.ty, p.extent)?;
                self.regs[base + p.local.index()] = nv;
            }
        }
        // setOutParamsValues
        for (i, p) in o.params.iter().enumerate() {
            if p.direction == Direction::Out {
                args[i] = self.regs[base + p.local.index()].clone();
            }
        }
        // the trace record, before population
        let params = self.params_of(op, base, args);
        let result = if o.results.is_empty() {
            None
        } else {
            Some(self.result_var(op, base))
        };
        self.add_trace(op, o.context.map(|_| src.clone()), params, result);
        // inherited mappings
        if let Some(m) = &o.mapping {
            for &inh in &m.inherits {
                let inh = self.overriding(inh);
                self.reusing_call(op, inh, src, args, base)?;
            }
        }
        if let Some(c) = code.population {
            self.set_chunk(c);
            self.exec(c, base, ibase)?;
        }
        if let Some(c) = code.end {
            self.set_chunk(c);
            self.exec(c, base, ibase)?;
        }
        if let Some(m) = &o.mapping {
            for &mg in &m.merges {
                let mg = self.overriding(mg);
                self.reusing_call(op, mg, src, args, base)?;
            }
        }
        // results without container and resource go to their extent
        for p in &o.results {
            if let Value::Obj(r) = self.regs[base + p.local.index()].clone() {
                let orphan = if r.is_local() {
                    false
                } else {
                    let h = S::Obj::from_u32(r.index());
                    self.store.resource_of(h).is_none() && self.store.container(h).is_none()
                };
                if orphan {
                    let class = self.class_of(&Value::Obj(r)).unwrap();
                    let x = self.default_extent(class, p.extent);
                    self.put_to_extent(r, x)?;
                }
            }
        }
        Ok((value, MappingStatus::Executed))
    }

    fn create_instance_of(
        &mut self,
        ty: TypeId,
        extent: Option<qvto_sema::ir::ModelParamRef>,
    ) -> VmResult<Value> {
        match self.prog.ir.types.get(ty).clone() {
            Type::Class(c) => self.create_object(c, extent),
            Type::Collection(..) | Type::List(_) | Type::Dict(..) => Ok(self.default_value(ty)),
            _ => Err(self.runtime_error(format!(
                "Cannot instantiate type {}",
                crate::types::type_simple_name(self.prog, ty)
            ))),
        }
    }

    /// An inherited or merged mapping, sharing results and out parameters with `caller`.
    fn reusing_call(
        &mut self,
        caller: OpId,
        callee: OpId,
        src: &Value,
        args: &[Value],
        base: usize,
    ) -> VmResult<()> {
        let mut sh = Shared::default();
        let co = self.prog.op(caller);
        sh.results = co
            .results
            .iter()
            .map(|p| self.regs[base + p.local.index()].clone())
            .collect();
        sh.result_var = co.result_local.map(|l| self.regs[base + l.index()].clone());
        sh.outs = co
            .params
            .iter()
            .map(|p| {
                (p.direction == Direction::Out).then(|| self.regs[base + p.local.index()].clone())
            })
            .collect();
        let r = self.execute_mapping(callee, src.clone(), args.to_vec(), Some(&mut sh))?;
        if r.status == MappingStatus::Executed {
            let co = self.prog.op(caller);
            for (p, v) in co.results.iter().zip(sh.results.iter()) {
                self.regs[base + p.local.index()] = v.clone();
            }
            if co.results.len() > 1
                && let (Some(l), Some(v)) = (co.result_local, sh.result_var.clone())
            {
                self.regs[base + l.index()] = v;
            }
            for (p, v) in co.params.iter().zip(sh.outs.iter()) {
                if let Some(v) = v {
                    self.regs[base + p.local.index()] = v.clone();
                }
            }
        }
        Ok(())
    }

    fn share_in(&mut self, op: OpId, base: usize, sh: &Shared) {
        let o = self.prog.op(op);
        for (p, v) in o.results.iter().zip(sh.results.iter()) {
            self.regs[base + p.local.index()] = v.clone();
        }
        if o.results.len() > 1
            && let (Some(l), Some(v)) = (o.result_local, sh.result_var.clone())
        {
            self.regs[base + l.index()] = v;
        }
        for (p, v) in o.params.iter().zip(sh.outs.iter()) {
            if let Some(v) = v {
                self.regs[base + p.local.index()] = v.clone();
            }
        }
    }

    fn share_out(&mut self, op: OpId, base: usize, sh: &mut Shared) {
        let o = self.prog.op(op);
        sh.results = o
            .results
            .iter()
            .map(|p| self.regs[base + p.local.index()].clone())
            .collect();
        if o.results.len() > 1 {
            sh.result_var = o.result_local.map(|l| self.regs[base + l.index()].clone());
        }
        for (i, p) in o.params.iter().enumerate() {
            if p.direction == Direction::Out
                && let Some(slot) = sh.outs.get_mut(i)
            {
                *slot = Some(self.regs[base + p.local.index()].clone());
            }
        }
    }

    /// `dispatchDisjunctMapping`: the first disjunct whose context and argument types match and
    /// whose call does not fail.
    fn dispatch_disjuncts(
        &mut self,
        op: OpId,
        src: &Value,
        args: &mut [Value],
        base: usize,
    ) -> VmResult<(Value, MappingStatus)> {
        let prog = self.prog;
        let o = prog.op(op);
        let disjuncts = o.mapping.as_ref().unwrap().disjuncts.clone();
        for d in disjuncts {
            let d = self.overriding(d);
            let dop = prog.op(d);
            if let Some(ct) = dop.context
                && !self.ocl_is_kind_of(src, ct)
            {
                continue;
            }
            if dop.params.len() != args.len() {
                continue;
            }
            if !dop
                .params
                .iter()
                .zip(args.iter())
                .all(|(p, a)| a.is_null() || self.ocl_is_kind_of(a, p.ty))
            {
                continue;
            }
            let r = self.execute_mapping(d, src.clone(), args.to_vec(), None)?;
            if !r.status.failed() {
                if let Some(rl) = o.result_local {
                    self.regs[base + rl.index()] = r.value.clone();
                }
                for (i, p) in o.params.iter().enumerate() {
                    if p.direction == Direction::Out {
                        self.regs[base + p.local.index()] = r.args[i].clone();
                        args[i] = r.args[i].clone();
                    }
                }
                let params = self.params_of(op, base, args);
                let result = if o.results.is_empty() {
                    None
                } else {
                    Some(self.result_var(op, base))
                };
                self.add_trace(op, o.context.map(|_| src.clone()), params, result);
                return Ok((r.value, MappingStatus::Executed));
            }
        }
        Ok((Value::Null, MappingStatus::NoDisjunctSelected))
    }

    /// Runs a code body (initialiser) on a frame of its own with `self` bound.
    pub(crate) fn run_body(&mut self, code: &BodyCode, self_val: Value) -> VmResult<Value> {
        let chunk = self.prog.chunk(code.chunk);
        let base = self.regs.len();
        self.regs.resize(base + chunk.nregs as usize, Value::Null);
        let ibase = self.iters.len();
        self.iters
            .resize(ibase + chunk.n_iters as usize, IterSlot::Empty);
        if let Some(s) = code.self_reg {
            self.regs[base + s as usize] = self_val;
        }
        let module = chunk.module;
        self.frames.push(Frame {
            op: None,
            module,
            chunk: code.chunk,
            pc: 0,
            fixed_ip: None,
        });
        let r = self.exec(code.chunk, base, ibase);
        self.frames.pop();
        self.regs.truncate(base);
        self.iters.truncate(ibase);
        r
    }

    #[allow(dead_code)]
    pub(crate) fn obj_value(o: S::Obj) -> Value {
        Value::Obj(ObjRef::host(o.to_u32()))
    }
}

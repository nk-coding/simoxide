//! Running a transformation: model parameters bound to extents, module initialisation,
//! model type conditions, the entry operation, deferred late-resolve assignments, the
//! execution diagnostic (`InternalTransformationExecutor`, `doVisitTransformation`).

use crate::error::{ErrorKind, QvtError, VmResult};
use crate::extent::Extent;
use crate::vm::{Diag, ObjHandle, Vm};
use qvto_bytecode::Site;
use qvto_model::ModelStore;
use qvto_runtime::{ObjRef, Value};
use qvto_sema::ir::{Direction, GlobalKind};
use qvto_types::Type;

/// The result of a run, in the terms of Eclipse's `ExecutionDiagnostic`.
#[derive(Debug)]
pub struct RunOutcome {
    /// EMF severity: 0 OK, 2 WARNING, 4 ERROR, 8 CANCEL.
    pub severity: i32,
    pub code: i32,
    pub message: String,
    /// The terminating QVT exception.
    pub error: Option<QvtError>,
    /// Log records and non-fatal assertions.
    pub diags: Vec<Diag>,
    /// Per model parameter of the transformation: the root objects of `out`/`inout` extents
    /// (detached from the in-memory extent resources), `None` for `in` parameters.
    pub extents: Vec<Option<Vec<ObjRef>>>,
    pub trace_records: usize,
}

impl<S: ModelStore> Vm<'_, S>
where
    S::Obj: ObjHandle,
{
    /// Runs the program's main transformation with the given root objects per model parameter
    /// (empty for `out` parameters).
    pub fn run_transformation(&mut self, args: Vec<Vec<S::Obj>>) -> RunOutcome {
        let ir = &self.prog.ir;
        let main = &ir.modules[ir.main_module.index()];
        if args.len() != main.model_params.len() {
            return RunOutcome {
                severity: 4,
                code: 140,
                message: format!(
                    "Invalid number of model parameters: expected {}, got {}",
                    main.model_params.len(),
                    args.len()
                ),
                error: None,
                diags: Vec::new(),
                extents: Vec::new(),
                trace_records: 0,
            };
        }
        // extents
        let mut given: Vec<Vec<ObjRef>> = Vec::new();
        for (i, (mp, roots)) in main.model_params.iter().zip(args).enumerate() {
            let mt = &ir.model_types[mp.model_type.index()];
            let label = format!("{} : {}::{}", mp.name, main.name, mt.name);
            let mut e = Extent::new(Some((ir.main_module, i as u32)), mp.direction, label);
            let objs: Vec<ObjRef> = if mp.direction == Direction::Out {
                Vec::new()
            } else {
                roots
                    .into_iter()
                    .map(|o| ObjRef::host(o.to_u32()))
                    .collect()
            };
            let mut next = self.next_extent_uri;
            given.push(objs.clone());
            e.bind_initial(self.store, objs, &mut next);
            self.next_extent_uri = next;
            self.extents.push(e);
            self.param_extents
                .insert((ir.main_module.0, i as u32), self.extents.len() - 1);
        }
        // modules the main transformation extends share its model parameters (by position)
        let mut stack = vec![ir.main_module];
        let mut seen = vec![ir.main_module];
        while let Some(m) = stack.pop() {
            for u in &ir.modules[m.index()].usages {
                if u.kind != qvto_sema::ir::UsageKind::Extends || seen.contains(&u.module) {
                    continue;
                }
                seen.push(u.module);
                stack.push(u.module);
                let n = ir.modules[u.module.index()].model_params.len();
                for i in 0..n.min(main.model_params.len()) {
                    if let Some(&x) = self.param_extents.get(&(ir.main_module.0, i as u32)) {
                        self.param_extents.insert((u.module.0, i as u32), x);
                    }
                }
            }
        }
        let mut unbound = Extent::new(None, Direction::Out, "unbound".into());
        unbound.local_only = false;
        self.extents.push(unbound);
        self.unbound_extent = self.extents.len() - 1;

        let r = self.run_main();
        // `createEvaluationResult`: the contents of the non-`in` extents that are not
        // generated for a legacy `main(in x : T)` signature (`$x`); none and an object result
        // of the entry: that object alone (Eclipse's compatibility extent). They fill the
        // non-`in` parameters in order.
        let n_params = main.model_params.len();
        let mut results: Vec<Vec<ObjRef>> = Vec::new();
        for i in 0..n_params {
            let mp = &main.model_params[i];
            // every extent's contents are taken (purge, detach from the in-memory resource)
            let x = self.param_extents[&(ir.main_module.0, i as u32)];
            let c = self.extents[x].contents(self.store);
            if mp.direction == Direction::In || generated_param(&mp.name) {
                continue;
            }
            results.push(c);
        }
        if results.is_empty()
            && let Ok(Value::Obj(o)) = &r
        {
            let mut e = Extent::new(None, Direction::Out, "compatibility".into());
            let mut next = self.next_extent_uri;
            e.bind_initial(self.store, vec![*o], &mut next);
            self.next_extent_uri = next;
            results.push(e.contents(self.store));
        }
        let mut results = results.into_iter();
        let mut extents = Vec::with_capacity(n_params);
        for (i, mp) in main.model_params.iter().enumerate() {
            if mp.direction == Direction::In {
                extents.push(None);
            } else if r.is_err() {
                // a failed execution writes no extent back: the caller's extents keep what
                // they were given
                extents.push(Some(given[i].clone()));
            } else {
                extents.push(Some(results.next().unwrap_or_default()));
            }
        }
        let r = r.map(|_| ());
        let trace_records = self.traces.len();
        match r {
            Ok(()) => {
                let severity = self.diags.iter().map(|d| d.severity).max().unwrap_or(0);
                RunOutcome {
                    severity,
                    code: 0,
                    message: "Transformation execution completed".into(),
                    error: None,
                    diags: std::mem::take(&mut self.diags),
                    extents,
                    trace_records,
                }
            }
            Err(u) => {
                let e = u.error().clone();
                self.log_line("Terminating execution...");
                let st = e.print_stack_trace();
                self.log_line(&st);
                RunOutcome {
                    severity: if e.kind == ErrorKind::Interrupted {
                        8
                    } else {
                        4
                    },
                    code: e.diagnostic_code(),
                    message: e.diagnostic_message(),
                    error: Some(e),
                    diags: std::mem::take(&mut self.diags),
                    extents,
                    trace_records,
                }
            }
        }
    }

    fn run_main(&mut self) -> VmResult<Value> {
        let prog = self.prog;
        let ir = &prog.ir;
        // module instances: imported modules first, each property in declaration order
        for (mi, m) in ir.modules.iter().enumerate() {
            self.init_module(mi, m)?;
        }
        // model type conditions of the non-out parameters
        let main = &ir.modules[ir.main_module.index()];
        for (i, mp) in main.model_params.iter().enumerate() {
            if mp.direction == Direction::Out {
                continue;
            }
            if let Some(code) = &prog.model_type_where[mp.model_type.index()] {
                let x = self.param_extents[&(ir.main_module.0, i as u32)];
                let ok = self.run_body(code, Value::Model(x as u32))?;
                if !ok.is_true() {
                    return Err(self.error(
                        ErrorKind::AssertionFailed,
                        Some(format!(
                            "WHERE condition failed for model parameter '{}' of transformation '{}'",
                            mp.name, main.name
                        )),
                    ));
                }
            }
        }
        let Some(entry) = main.entry else {
            return Err(self.runtime_error(format!("Module '{}' is not executable", main.name)));
        };
        // entry arguments: the first initial object of each matched model parameter
        let eop = prog.op(entry);
        let mut args = Vec::with_capacity(eop.params.len());
        let mut param_index = 0usize;
        for p in &eop.params {
            if p.direction == Direction::Out {
                args.push(Value::Null);
                continue;
            }
            let mut matched = param_index;
            if let Some(x) = p.extent {
                matched = x.index as usize;
            }
            match self.param_extents.get(&(ir.main_module.0, matched as u32)) {
                Some(&x) => {
                    if let Some(&o) = self.extents[x].initial.first() {
                        args.push(Value::Obj(o));
                    }
                }
                None => {
                    return Err(self.runtime_error(format!(
                        "entry operation arguments mismatch: no argument for '{}' parameter",
                        p.name
                    )));
                }
            }
            param_index += 1;
        }
        if args.len() != eop.params.len() {
            return Err(self.runtime_error("Unexpected runtime exception caught during execution"));
        }
        let v = self.execute_op(entry, Value::Null, args)?;
        self.process_deferred()?;
        Ok(v)
    }

    fn init_module(&mut self, mi: usize, m: &qvto_sema::ir::Module) -> VmResult<()> {
        let prog = self.prog;
        let _ = mi;
        for &g in &m.globals {
            let gl = &prog.ir.globals[g.index()];
            let v = match gl.kind {
                GlobalKind::Config => {
                    let raw = self
                        .opts
                        .config
                        .iter()
                        .find(|(k, _)| *k == gl.name)
                        .map(|(_, v)| v.clone());
                    match raw {
                        Some(Some(s)) => {
                            self.frames.push(crate::vm::Frame {
                                op: None,
                                module: gl.module,
                                chunk: qvto_bytecode::ChunkId(0),
                                pc: 0,
                                fixed_ip: Some(gl.span.start),
                            });
                            let r = self.config_value(gl.ty, &gl.name, &s);
                            self.frames.pop();
                            r?
                        }
                        _ => Value::Null,
                    }
                }
                GlobalKind::Property => match &prog.globals[g.index()] {
                    Some(code) => self.run_body(code, Value::Instance(gl.module.0))?,
                    None => self.default_value(gl.ty),
                },
            };
            self.globals[g.index()] = v;
        }
        Ok(())
    }

    /// `ConversionUtils.createFromString` of a configuration property value.
    fn config_value(&self, ty: qvto_types::TypeId, name: &str, raw: &str) -> VmResult<Value> {
        let units: Vec<u16> = raw.encode_utf16().collect();
        use qvto_runtime::java::parse;
        let v = match self.prog.ir.types.get(ty) {
            Type::String | Type::OclAny => Some(Value::str(raw)),
            Type::Integer | Type::UnlimitedNatural => {
                parse::parse_int(&units).map(|i| Value::Int(i as i64))
            }
            Type::Real => parse::parse_double(&units).map(Value::Real),
            Type::Boolean => Some(Value::Bool(raw.eq_ignore_ascii_case("true"))),
            Type::DataType(d) => {
                crate::stdlib::parse_data_value(self.prog.ir.meta.data_type(*d).kind, raw)
            }
            Type::Enum(e) => {
                let en = self.prog.ir.meta.enumeration(*e);
                en.by_literal(raw).or_else(|| en.by_name(raw)).map(|l| {
                    Value::Enum(qvto_runtime::EnumLit {
                        enum_id: e.0,
                        literal: l,
                    })
                })
            }
            _ => Some(Value::str(raw)),
        };
        v.ok_or_else(|| {
            self.runtime_error(format!(
                "Failed to get '{name}' configuration property value from '{raw}' string"
            ))
        })
    }

    /// Evaluates a late resolve's condition for one candidate on a fresh frame.
    fn late_cond(
        &mut self,
        chunk: qvto_bytecode::ChunkId,
        entry: u32,
        var: u16,
        cand: Value,
    ) -> VmResult<bool> {
        let c = self.prog.chunk(chunk);
        let base = self.regs.len();
        self.regs.resize(base + c.nregs as usize, Value::Null);
        let ibase = self.iters.len();
        self.iters
            .resize(ibase + c.n_iters as usize, crate::vm::IterSlot::Empty);
        self.regs[base + var as usize] = cand;
        self.frames.push(crate::vm::Frame {
            op: None,
            module: c.module,
            chunk,
            pc: entry,
            fixed_ip: None,
        });
        let r = self.exec_at(chunk, base, ibase, entry);
        self.frames.pop();
        self.regs.truncate(base);
        self.iters.truncate(ibase);
        Ok(r?.is_true())
    }

    /// Late resolve assignments, first in first out, after the entry operation.
    fn process_deferred(&mut self) -> VmResult<()> {
        let mut i = 0;
        while i < self.late.len() {
            let t = self.late[i].clone();
            i += 1;
            let site = self.prog.chunk(t.chunk).sites[t.site as usize].clone();
            let Site::LateResolve {
                kind,
                has_source,
                source_is_coll,
                mapping,
                target_ty,
                cond,
                feature,
                interm,
                add,
                conversion,
            } = site
            else {
                continue;
            };
            let all_kind = if kind.is_inverse() {
                qvto_sema::ir::ResolveKind::InvResolve
            } else {
                qvto_sema::ir::ResolveKind::Resolve
            };
            let rs = Site::Resolve {
                kind: if cond.is_some() { all_kind } else { kind },
                has_source,
                source_is_coll,
                mapping,
                target_ty,
            };
            let mut v = self.resolve(&rs, t.source.clone());
            if let Some((entry, var)) = cond {
                let cands = crate::vm::Iter::of(&v).items;
                let mut kept = Vec::new();
                for c in cands.iter() {
                    if self.late_cond(t.chunk, entry, var, c.clone())? {
                        kept.push(c.clone());
                        if kind.is_one() {
                            break;
                        }
                    }
                }
                v = if kind.is_one() {
                    kept.into_iter().next().unwrap_or(Value::Null)
                } else {
                    Value::Coll(std::rc::Rc::new(qvto_runtime::coll::Coll::Sequence(kept)))
                };
            }
            if let Some(op) = conversion {
                let base = self.regs.len();
                self.regs.push(v);
                v = self.call_std(op, base, false, false)?;
                self.regs.truncate(base);
            }
            if let Some(f) = feature {
                self.set_prop(&t.owner, f, v, !add)?;
            } else if let Some(p) = interm {
                self.set_interm(&t.owner, p, v, !add);
            }
        }
        Ok(())
    }
}

/// A model parameter the analyser generated for a legacy `main(in x : T)` signature: `$x`
/// (unnamed declared parameters are `$<index>`). The IR has no flag for it yet.
fn generated_param(name: &str) -> bool {
    name.strip_prefix('$')
        .is_some_and(|r| !r.is_empty() && !r.bytes().all(|b| b.is_ascii_digit()))
}

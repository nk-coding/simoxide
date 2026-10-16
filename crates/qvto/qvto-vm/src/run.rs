//! Running a transformation: model parameters bound to extents, module initialisation,
//! model type conditions, the entry operation, deferred late-resolve assignments, the
//! execution diagnostic (`InternalTransformationExecutor`, `doVisitTransformation`).

use crate::error::{ErrorKind, QvtError, VmResult};
use crate::extent::Extent;
use crate::vm::{Diag, ObjHandle, Vm};
use qvto_bytecode::Site;
use qvto_model::ModelStore;
use qvto_runtime::{ObjRef, Value};
use qvto_sema::ir::{Direction, GlobalKind, ModuleId};
use qvto_types::Type;
use qvto_types::CollKind;

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
        // the other transformations' model parameters (`ModelParameterHelper`): a main
        // parameter of compatible direction and metamodels, else of compatible direction, else
        // an extent of their own
        for (mi, m) in ir.modules.iter().enumerate() {
            if mi == ir.main_module.index() || m.kind != qvto_sema::ir::ModuleKind::Transformation {
                continue;
            }
            let mut bound: Vec<usize> = Vec::new();
            let mut unset = Vec::new();
            for (pi, mp) in m.model_params.iter().enumerate() {
                let found = main.model_params.iter().enumerate().position(|(j, q)| {
                    !bound.contains(&j)
                        && direction_compatible(q.direction, mp.direction)
                        && ir.model_types[mp.model_type.index()]
                            .packages
                            .iter()
                            .all(|p| ir.model_types[q.model_type.index()].packages.contains(p))
                });
                match found {
                    Some(j) => {
                        bound.push(j);
                        let x = self.param_extents[&(ir.main_module.0, j as u32)];
                        self.param_extents.insert((mi as u32, pi as u32), x);
                    }
                    None => unset.push(pi),
                }
            }
            for pi in unset {
                let mp = &m.model_params[pi];
                let found = main.model_params.iter().enumerate().position(|(j, q)| {
                    !bound.contains(&j) && direction_compatible(q.direction, mp.direction)
                });
                let x = match found {
                    Some(j) => {
                        bound.push(j);
                        self.param_extents[&(ir.main_module.0, j as u32)]
                    }
                    None => {
                        self.extents.push(Extent::new(
                            Some((qvto_sema::ir::ModuleId(mi as u32), pi as u32)),
                            mp.direction,
                            mp.name.clone(),
                        ));
                        self.extents.len() - 1
                    }
                };
                self.param_extents.insert((mi as u32, pi as u32), x);
            }
        }
        let mut unbound = Extent::new(None, Direction::Out, "unbound".into());
        unbound.local_only = false;
        self.extents.push(unbound);
        self.unbound_extent = self.extents.len() - 1;

        let r = self.run_main();
        // `createEvaluationResult`: the contents of the non-`in` extents that are not
        // generated for a legacy `main(in x : T)` signature; none and an object result
        // of the entry: that object alone (Eclipse's compatibility extent). They fill the
        // non-`in` parameters in order.
        let n_params = main.model_params.len();
        let mut results: Vec<Vec<ObjRef>> = Vec::new();
        for i in 0..n_params {
            let mp = &main.model_params[i];
            // every extent's contents are taken (purge, detach from the in-memory resource)
            let x = self.param_extents[&(ir.main_module.0, i as u32)];
            let c = self.extents[x].contents(self.store);
            if mp.direction == Direction::In || mp.generated {
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
        self.run_transformation_module(ir.main_module)
    }

    /// Model type conditions, the entry operation, deferred assignments of a transformation
    /// whose model parameters are bound.
    fn run_transformation_module(&mut self, module: ModuleId) -> VmResult<Value> {
        let prog = self.prog;
        let ir = &prog.ir;
        // model type conditions of the non-out parameters
        let main = &ir.modules[module.index()];
        for (i, mp) in main.model_params.iter().enumerate() {
            if mp.direction == Direction::Out {
                continue;
            }
            if let Some(code) = &prog.model_type_where[mp.model_type.index()] {
                let x = self.param_extents[&(module.0, i as u32)];
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
        if main.blackbox && main.kind == qvto_sema::ir::ModuleKind::Transformation {
            let instance = Value::Instance(module.0);
            return self.call_native_transformation(module, instance);
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
            match self.param_extents.get(&(module.0, matched as u32)) {
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

    /// `new T(m1, ...)`: a transformation instance on the given models, its properties
    /// initialised (`callTransformationImplicitConstructor`).
    pub(crate) fn new_transformation(
        &mut self,
        module: ModuleId,
        at: usize,
        nargs: usize,
    ) -> VmResult<Value> {
        let mut models = Vec::with_capacity(nargs);
        for i in 0..nargs {
            match self.regs[at + i] {
                Value::Model(x) => models.push(x as usize),
                _ => {
                    return Err(self.runtime_error("Undefined model passed to transformation"));
                }
            }
        }
        let k = self.nested.len();
        self.nested.push((module, models));
        let saved = self.bind_nested(k);
        let m = &self.prog.ir.modules[module.index()];
        let r = self.init_module(module.index(), m);
        self.unbind_nested(module, saved);
        r?;
        Ok(Value::Instance(NESTED | k as u32))
    }

    fn bind_nested(&mut self, k: usize) -> Vec<Option<usize>> {
        let (module, models) = self.nested[k].clone();
        let n = self.prog.ir.modules[module.index()].model_params.len();
        let mut saved = Vec::with_capacity(n);
        for i in 0..n {
            let key = (module.0, i as u32);
            saved.push(self.param_extents.get(&key).copied());
            match models.get(i) {
                Some(&x) => {
                    self.param_extents.insert(key, x);
                }
                None => {
                    self.param_extents.remove(&key);
                }
            }
        }
        saved
    }

    fn unbind_nested(&mut self, module: ModuleId, saved: Vec<Option<usize>>) {
        for (i, s) in saved.into_iter().enumerate() {
            let key = (module.0, i as u32);
            match s {
                Some(x) => {
                    self.param_extents.insert(key, x);
                }
                None => {
                    self.param_extents.remove(&key);
                }
            }
        }
    }

    /// `Transformation::transform()`: runs a transformation instance; the result is a `Status`
    /// (failed with the exception for QVT exceptions other than interruptions).
    pub(crate) fn transform(&mut self, v: &Value) -> VmResult<Value> {
        let k = match v {
            Value::Instance(i) if i & NESTED != 0 => (i & !NESTED) as usize,
            _ => return Ok(Value::Invalid),
        };
        let module = self.nested[k].0;
        let m = &self.prog.ir.modules[module.index()];
        if m.entry.is_none() && !m.blackbox {
            return Ok(Value::Invalid);
        }
        let saved = self.bind_nested(k);
        let saved_transf = std::mem::replace(&mut self.cur_transf, module);
        let saved_late = std::mem::take(&mut self.late);
        // the nested run has a diagnostic of its own (its log goes to the same log)
        let saved_diags = std::mem::take(&mut self.diags);
        let r = self.run_transformation_module(module);
        let nested_error = self.diags.iter().any(|d| d.severity >= 4);
        self.diags = saved_diags;
        self.late = saved_late;
        self.cur_transf = saved_transf;
        self.unbind_nested(module, saved);
        let status_class = self.prog.ir.std_classes.status;
        let (ok, exc) = match r {
            Ok(_) => (!nested_error, Value::Null),
            Err(u) => {
                let e = u.error().clone();
                if e.kind == ErrorKind::Interrupted {
                    return Err(u);
                }
                // an `Exception` with the message and the stack
                let o = self.new_local(self.prog.ir.std_classes.exception);
                self.local[o.index() as usize].exception =
                    Some((e.message.clone(), e.stack.clone()));
                (false, Value::Obj(o))
            }
        };
        let o = self.new_local(status_class);
        self.local[o.index() as usize].status = Some((ok, exc));
        Ok(Value::Obj(o))
    }

    fn init_module(&mut self, mi: usize, m: &qvto_sema::ir::Module) -> VmResult<()> {
        let prog = self.prog;
        let _ = mi;
        // configuration properties first, then the others (`doInitModule`)
        let globals = m
            .globals
            .iter()
            .filter(|g| prog.ir.globals[g.index()].kind == GlobalKind::Config)
            .chain(
                m.globals
                    .iter()
                    .filter(|g| prog.ir.globals[g.index()].kind != GlobalKind::Config),
            );
        for &g in globals {
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
            // stored with `callSetter`'s rules (a null collection becomes an empty one)
            let v = if v.is_undefined() {
                self.property_undefined(gl.ty, Value::Null, v, true)
            } else {
                self.assign_value(gl.ty, Value::Null, v, true)
            };
            self.globals[g.index()] = v;
        }
        Ok(())
    }

    /// `ConversionUtils.createFromString` of a configuration property value.
    fn config_value(&self, ty: qvto_types::TypeId, name: &str, raw: &str) -> VmResult<Value> {
        let v = match self.prog.ir.types.get(ty) {
            Type::String | Type::OclAny => Some(Value::str(raw)),
            _ => self.convert_from_string(ty, &raw.chars().collect::<Vec<_>>()),
        };
        v.ok_or_else(|| {
            self.runtime_error(format!(
                "Failed to get '{name}' configuration property value from '{raw}' string"
            ))
        })
    }

    /// `ConversionUtils.createFromString` (with `ConversionCollection` and
    /// `ConversionDictionary` for collections and dictionaries of data types); `None` where
    /// Java throws `IllegalArgumentException`.
    fn convert_from_string(&self, ty: qvto_types::TypeId, lit: &[char]) -> Option<Value> {
        use qvto_bytecode::insn::coll_kind as k;
        use qvto_runtime::java::parse;
        let types = &self.prog.ir.types;
        let text = || lit.iter().collect::<String>();
        match types.get(ty) {
            Type::Dict(kt, vt) => {
                let (kt, vt) = (*kt, *vt);
                if !self.is_data_type(kt) || !self.is_data_type(vt) {
                    return None;
                }
                let lit = brackets(java_trim(lit))?;
                let out = self.new_coll(k::DICT);
                let Value::Dict(d) = &out else { return None };
                let mut i = 0;
                while i < lit.len() {
                    let end = extract(lit, i, '=')?;
                    let key = self.convert_from_string(kt, java_trim(&unescape(&lit[i..end])))?;
                    i = end + 1;
                    let end = extract(lit, i, ',').unwrap_or(lit.len());
                    let val = self.convert_from_string(vt, java_trim(&unescape(&lit[i..end])))?;
                    i = end + 1;
                    d.borrow_mut().put(key, val, &self.env);
                }
                Some(out)
            }
            Type::Collection(CollKind::Collection, _) => None,
            Type::Collection(_, et) | Type::List(et) => {
                let et = *et;
                if !self.is_data_type(et) {
                    return None;
                }
                let lit = brackets(java_trim(lit))?;
                let mut items = Vec::new();
                let mut i = 0;
                while i < lit.len() {
                    let end = extract(lit, i, ',').unwrap_or(lit.len());
                    items.push(self.convert_from_string(et, java_trim(&unescape(&lit[i..end])))?);
                    i = end + 1;
                }
                let kc = match types.get(ty) {
                    Type::List(_) => k::LIST,
                    Type::Collection(CollKind::Set, _) => k::SET,
                    Type::Collection(CollKind::OrderedSet, _) => k::ORDERED_SET,
                    Type::Collection(CollKind::Sequence, _) => k::SEQUENCE,
                    _ => k::BAG,
                };
                let mut out = self.new_coll(kc);
                for v in items {
                    crate::vm::push(&mut out, v, &self.env);
                }
                Some(out)
            }
            Type::Integer | Type::UnlimitedNatural => {
                let units: Vec<u16> = text().encode_utf16().collect();
                parse::parse_int(&units).map(|i| Value::Int(i as i64))
            }
            Type::Real => {
                let units: Vec<u16> = text().encode_utf16().collect();
                parse::parse_double(&units).map(Value::Real)
            }
            Type::String => Some(Value::str(&text())),
            Type::Boolean => Some(Value::Bool(text().eq_ignore_ascii_case("true"))),
            Type::DataType(d) => {
                crate::stdlib::parse_data_value(self.prog.ir.meta.data_type(*d).kind, &text())
            }
            Type::Enum(e) => {
                let raw = text();
                let en = self.prog.ir.meta.enumeration(*e);
                en.by_literal(&raw).or_else(|| en.by_name(&raw)).map(|l| {
                    Value::Enum(qvto_runtime::EnumLit {
                        enum_id: e.0,
                        literal: l,
                    })
                })
            }
            _ => None,
        }
    }

    /// An `EDataType` in Eclipse: primitive types, data types, enumerations, collections,
    /// dictionaries.
    fn is_data_type(&self, t: qvto_types::TypeId) -> bool {
        matches!(
            self.prog.ir.types.get(t),
            Type::String
                | Type::Integer
                | Type::UnlimitedNatural
                | Type::Real
                | Type::Boolean
                | Type::DataType(_)
                | Type::Enum(_)
                | Type::Collection(..)
                | Type::List(_)
                | Type::Dict(..)
        )
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

/// `isModelParamEqual`'s direction rule: an `in` parameter only binds `in` parameters.
fn direction_compatible(main: Direction, other: Direction) -> bool {
    main != Direction::In || other == Direction::In
}

/// The bit of `Value::Instance` that marks instances made with `new T(...)`.
pub(crate) const NESTED: u32 = 1 << 31;

/// Java's `String.trim`.
fn java_trim(s: &[char]) -> &[char] {
    let a = s.iter().position(|&c| c > ' ').unwrap_or(s.len());
    let b = s.iter().rposition(|&c| c > ' ').map_or(a, |i| i + 1);
    &s[a..b.max(a)]
}

/// Strips the brackets of `[...]`; `None` for an unclosed bracket.
fn brackets(s: &[char]) -> Option<&[char]> {
    if s.first() == Some(&'[') {
        if s.last() != Some(&']') || s.len() < 2 {
            return None;
        }
        return Some(&s[1..s.len() - 1]);
    }
    Some(s)
}

/// `ConversionCollection.unescapeString` (a trailing backslash leaves a NUL character).
fn unescape(s: &[char]) -> Vec<char> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == '\\' {
            i += 1;
            out.push(if i < s.len() { s[i] } else { '\0' });
        } else {
            out.push(s[i]);
        }
        i += 1;
    }
    out
}

/// The end of an element (`extractCollectionElement`, `extractDictionaryKey`/`Value`): the
/// first `stop` outside brackets, skipping escaped characters.
fn extract(s: &[char], start: usize, stop: char) -> Option<usize> {
    let mut level = 0i32;
    let mut i = start;
    while i < s.len() {
        match s[i] {
            '\\' => i += 1,
            '[' => level += 1,
            ']' => level -= 1,
            c if c == stop && level == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

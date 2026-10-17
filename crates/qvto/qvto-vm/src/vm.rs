//! The interpreter: the VM state of one run and the instruction loop.

use crate::env::RunEnv;
use crate::error::{ErrorKind, QvtError, StackElem, Unwind, VmResult};
use crate::extent::Extent;
use crate::stdlib::Prim;
use crate::trace::Traces;
use qvto_bytecode::{ChunkId, Const, Insn, Program, Site};
use qvto_model::{ClassId, FeatureId, ModelStore};
use qvto_runtime::coll::{Coll, CollKind};
use qvto_runtime::dict::{Dict, List};
use qvto_runtime::fxhash::FxHashMap as HashMap;
use qvto_runtime::identity::IdentityHashSource;
use qvto_runtime::{EnumLit, IdentityKey, Mode, ObjRef, Str, TypeRef, Value};
use qvto_sema::ir::GlobalId;
use qvto_sema::ir::{IntermPropId, ModuleId, OpId};
use qvto_types::{StdOp, TypeId};
use std::cell::RefCell;
use std::rc::Rc;

/// A store's object handle as a dense `u32` (the VM's `ObjRef::host`).
pub trait ObjHandle: Copy {
    fn to_u32(self) -> u32;
    fn from_u32(i: u32) -> Self;
}

impl ObjHandle for u32 {
    fn to_u32(self) -> u32 {
        self
    }
    fn from_u32(i: u32) -> u32 {
        i
    }
}

#[cfg(feature = "emf")]
impl ObjHandle for qvto_emf::ObjId {
    fn to_u32(self) -> u32 {
        self.0
    }
    fn from_u32(i: u32) -> Self {
        qvto_emf::ObjId(i)
    }
}

/// Options of a run.
pub struct RunOptions {
    pub mode: Mode,
    /// Identity hashes (compat mode, default HotSpot's surrogate sequence; ordered mode,
    /// default derived from the key like fast mode).
    pub id_source: Option<Box<dyn IdentityHashSource>>,
    /// `QVTEvaluationOptions.EVALUATION_MAX_STACK_DEPTH` (default 1000).
    pub max_stack_depth: usize,
    /// `FLAG_READONLY_GUARD_ENABLED`.
    pub readonly_guard: bool,
    /// Configuration properties by name (string values, converted by type).
    pub config: Vec<(String, Option<String>)>,
    /// Step budget (backward jumps and calls); `None` for unlimited.
    pub step_limit: Option<u64>,
    /// Largest collection a single operation may build (ranges, `product`, the results of
    /// library operations): beyond it the execution is interrupted ("resource limit"). Eclipse
    /// has no limit of its own (it runs out of heap); `None` for none.
    pub max_collection_size: Option<usize>,
    /// Longest string (UTF-16 units) an operation may build; likewise.
    pub max_string_length: Option<usize>,
    /// Native stack (bytes) the run may use below its entry: a call beyond it fails like the
    /// depth limit (`QvtStackOverFlowError`), so that a deep recursion cannot overflow the
    /// thread's stack. A QVTo call level takes about 2–4 KB in release builds (8–16 KB in debug
    /// builds): the default (1 MB) is safe on a 2 MB thread; Eclipse's depth of 1 000 needs a
    /// larger stack and budget (the conformance harness runs on a 512 MB thread).
    pub max_native_stack: Option<usize>,
    /// Stack traces with units and lines; `false` gives `(Unknown Source)` (a program
    /// compiled without line numbers).
    pub line_numbers: bool,
}

/// 2^27 elements (2 GB of values): a little below what a JVM with a few GB of heap holds.
pub const DEFAULT_MAX_COLLECTION_SIZE: usize = 1 << 27;
/// 2^27 UTF-16 units (a quarter of a GB of text).
pub const DEFAULT_MAX_STRING_LENGTH: usize = 1 << 27;

impl Default for RunOptions {
    fn default() -> Self {
        RunOptions {
            mode: Mode::default(),
            id_source: None,
            max_stack_depth: 1000,
            readonly_guard: false,
            config: Vec::new(),
            step_limit: None,
            max_collection_size: Some(DEFAULT_MAX_COLLECTION_SIZE),
            max_string_length: Some(DEFAULT_MAX_STRING_LENGTH),
            max_native_stack: Some(1 << 20),
            line_numbers: true,
        }
    }
}

/// An engine-local object: an instance of an intermediate class or a library class.
#[derive(Clone, Debug)]
pub struct LocalObj {
    pub class: ClassId,
    pub slots: Vec<Value>,
    /// Exceptions: the message and the stack trace.
    pub exception: Option<(Option<String>, Vec<StackElem>)>,
    /// `Status`: success and the raised exception.
    pub status: Option<(bool, Value)>,
}

/// A frame of the call stack (for stack traces).
#[derive(Clone, Debug)]
pub struct Frame {
    pub op: Option<OpId>,
    pub module: ModuleId,
    pub chunk: ChunkId,
    pub pc: u32,
    /// An instruction pointer outside chunks (module initialisation: the property).
    pub fixed_ip: Option<u32>,
}

/// The iteration state of one loop slot.
#[derive(Clone, Debug, Default)]
pub enum IterSlot {
    #[default]
    Empty,
    Iter(Iter),
    /// A loop over a Sequence or OrderedSet value, read in place (the slot keeps the value;
    /// a change of the variable copies it first, as values are immutable).
    Coll(Rc<Coll>, usize),
    /// `closure`: a stack of levels; elements below the first are added to the result.
    Closure(Vec<Iter>),
}

#[derive(Clone, Debug)]
pub struct Iter {
    pub items: Rc<[Value]>,
    pub pos: usize,
}

impl Iter {
    pub fn of(v: &Value) -> Iter {
        let items: Rc<[Value]> = match v {
            Value::Coll(c) => match &**c {
                Coll::Sequence(v) => v.as_slice().into(),
                c => c.to_vec().into(),
            },
            Value::List(l) => l.borrow().items.as_slice().into(),
            Value::Dict(d) => d.borrow().values().into(),
            Value::Null | Value::Invalid => Rc::from([]),
            other => Rc::from([other.clone()]),
        };
        Iter { items, pos: 0 }
    }
}

/// One entry of the execution diagnostic (`ExecutionDiagnostic` children).
#[derive(Clone, Debug, PartialEq)]
pub struct Diag {
    /// EMF severity: 0 OK, 2 WARNING, 4 ERROR.
    pub severity: i32,
    pub code: i32,
    pub message: String,
}

/// A deferred assignment of a late resolve.
#[derive(Clone, Debug)]
pub struct LateTask {
    pub chunk: ChunkId,
    pub site: u16,
    pub owner: Value,
    pub source: Value,
}

/// The state of one run.
pub struct Vm<'a, S: ModelStore>
where
    S::Obj: ObjHandle,
{
    pub(crate) prog: &'a Program,
    pub store: &'a mut S,
    pub env: RunEnv,
    /// The program's constants as values, converted on first use.
    pub(crate) consts: Vec<Option<Value>>,
    pub(crate) regs: Vec<Value>,
    pub(crate) iters: Vec<IterSlot>,
    pub(crate) frames: Vec<Frame>,
    pub(crate) globals: Vec<Value>,
    pub(crate) local: Vec<LocalObj>,
    pub(crate) extents: Vec<Extent>,
    /// Extent of each model parameter of the main transformation, by `(module, index)`.
    pub(crate) param_extents: HashMap<(u32, u32), usize>,
    pub(crate) unbound_extent: usize,
    pub(crate) next_extent_uri: u32,
    pub(crate) traces: Traces,
    pub log: String,
    pub diags: Vec<Diag>,
    pub(crate) opts: RunOptions,
    pub(crate) vcache: RefCell<HashMap<(u32, u32), u32>>,
    pub(crate) interm: HashMap<(crate::objects::IntermOwner, IntermPropId), Value>,
    pub(crate) late: Vec<LateTask>,
    pub(crate) prims: &'static [Prim],
    /// Argument buffer of standard library calls (reused).
    pub(crate) argbuf: Vec<Value>,
    pub(crate) str_counters: HashMap<String, i64>,
    pub(crate) steps: u64,
    /// Stderr-like messages of swallowed runtime errors (`Evaluation failed at ...`).
    pub errors: Vec<String>,
    pub(crate) class_local: Vec<bool>,
    /// The declared OCL types of intermediate class features (`IntermClass::feature_types`).
    pub(crate) interm_feature_ty: HashMap<qvto_model::FeatureId, TypeId>,
    /// Where instances of intermediate classes live and how their features are kept.
    pub(crate) interm_info: crate::objects::IntermInfo,
    /// Values of the engine-held features (attributes) of intermediate objects in the store.
    pub(crate) interm_vals: HashMap<(u32, qvto_model::FeatureId), Value>,
    /// Values of `static` intermediate features (`IntermediateStaticFieldAdapter`).
    pub(crate) static_vals: HashMap<qvto_model::FeatureId, Value>,
    /// The intermediate extent of each transformation (`getIntermediateExtent`).
    pub(crate) interm_extents: HashMap<u32, usize>,
    /// The records of the input trace of an incremental update.
    pub(crate) inc_trace: Vec<crate::inctrace::IncRecord>,
    /// Transformation instances made with `new T(...)`: the module and the extents of its
    /// model parameters (`Value::Instance(NESTED | index)`).
    pub(crate) nested: Vec<(ModuleId, Vec<usize>)>,
    /// The property values of each nested instance while it is not current (in its module's
    /// property order).
    pub(crate) nested_globals: Vec<Vec<Value>>,
    /// The nested instances made current (`bind_nested`), innermost last.
    pub(crate) bound_nested: Vec<usize>,
    /// The transformation whose model parameters receive created objects by default.
    pub(crate) cur_transf: ModuleId,
    /// Static type of the receiver of the current `CallStdX`.
    pub(crate) std_src_ty: TypeId,
    /// Type literals that are classifier objects of the store, and back.
    pub(crate) type_objects: HashMap<TypeId, ObjRef>,
    pub(crate) object_types: HashMap<ObjRef, TypeId>,
    /// Address of a local of the outermost VM entry (native stack accounting).
    pub(crate) stack_base: std::cell::Cell<usize>,
    /// The static type of that call.
    pub(crate) std_res_ty: TypeId,
    pub(crate) regexes: RefCell<HashMap<(String, bool), Result<regex::Regex, String>>>,
    /// Native blackbox implementations and their per-run state (`native.rs`).
    pub(crate) native: crate::native::NativeState<S>,
}

/// A register of the current frame (shared). Registers are below the frame's size by
/// construction of the compiler (every frame is allocated with its chunk's register count);
/// debug builds check.
macro_rules! r {
    ($vm:ident, $base:ident, $r:expr) => {
        (*{
            let i = $base + $r as usize;
            debug_assert!(i < $vm.regs.len());
            // SAFETY: in bounds (see above); the reference does not outlive the instruction
            unsafe { &*$vm.regs.as_ptr().add(i) }
        })
    };
}

/// A register of the current frame (for writing).
macro_rules! w {
    ($vm:ident, $base:ident, $r:expr) => {
        (*{
            let i = $base + $r as usize;
            debug_assert!(i < $vm.regs.len());
            // SAFETY: as for `r!`; no other reference to the register is alive
            unsafe { &mut *($vm.regs.as_mut_ptr().add(i)) }
        })
    };
}

impl<'a, S: ModelStore> Vm<'a, S>
where
    S::Obj: ObjHandle,
{
    pub fn new(prog: &'a Program, store: &'a mut S, opts: RunOptions) -> Vm<'a, S> {
        let env = RunEnv::new(opts.mode, None);
        let mut vm = Vm {
            prog,
            store,
            env,
            consts: Vec::new(),
            regs: Vec::with_capacity(1024),
            iters: Vec::new(),
            frames: Vec::new(),
            globals: vec![Value::Null; prog.ir.globals.len()],
            local: Vec::new(),
            extents: Vec::new(),
            param_extents: HashMap::default(),
            unbound_extent: 0,
            next_extent_uri: 0,
            traces: Traces::default(),
            log: String::new(),
            diags: Vec::new(),
            opts,
            vcache: RefCell::new(HashMap::default()),
            interm: HashMap::default(),
            late: Vec::new(),
            prims: crate::stdlib::prims(),
            argbuf: Vec::with_capacity(8),
            str_counters: HashMap::default(),
            steps: 0,
            errors: Vec::new(),
            class_local: Vec::new(),
            interm_feature_ty: prog
                .ir
                .interm_classes
                .iter()
                .flat_map(|c| c.feature_types.iter().copied())
                .collect(),
            interm_info: crate::objects::IntermInfo::default(),
            interm_vals: HashMap::default(),
            static_vals: HashMap::default(),
            interm_extents: HashMap::default(),
            inc_trace: Vec::new(),
            nested: Vec::new(),
            nested_globals: Vec::new(),
            bound_nested: Vec::new(),
            cur_transf: prog.ir.main_module,
            regexes: RefCell::new(HashMap::default()),
            native: Default::default(),
            std_src_ty: TypeId::OCL_VOID,
            stack_base: std::cell::Cell::new(0),
            type_objects: HashMap::default(),
            object_types: HashMap::default(),
            std_res_ty: TypeId::OCL_VOID,
        };
        if let Some(src) = vm.opts.id_source.take() {
            vm.env = RunEnv::new(vm.opts.mode, Some(src));
        }
        // identity aliases (compat mode) and the classifier objects of type literals
        vm.alias_classifier_identities();
        vm.consts = vec![None; prog.consts.len()];
        vm.interm_info = crate::objects::IntermInfo::new(prog, vm.store);
        vm.class_local = crate::objects::local_classes(prog, vm.interm_info.in_store);
        vm
    }

    // ---- errors -----------------------------------------------------------------------------

    /// The QVT stack trace of the current call stack (innermost first).
    pub fn stack_trace(&self) -> Vec<StackElem> {
        let ir = &self.prog.ir;
        let mut out = Vec::with_capacity(self.frames.len());
        for f in self.frames.iter().rev() {
            let chunk = self.prog.chunk(f.chunk);
            let module = &ir.modules[f.module.index()];
            let operation = match f.op {
                Some(op) => {
                    let o = &ir.operations[op.index()];
                    match o.context {
                        Some(c) => format!(
                            "{}::{}",
                            crate::types::type_simple_name(self.prog, c),
                            o.name
                        ),
                        None => o.name.clone(),
                    }
                }
                None => module.name.clone(),
            };
            let source = &ir.sources[module.source.index()];
            let (src_id, ip) = match f.fixed_ip {
                Some(ip) => (module.source, ip),
                None => (
                    chunk.source,
                    chunk.ips.get(f.pc as usize).copied().unwrap_or(u32::MAX),
                ),
            };
            let line = if ip == u32::MAX {
                None
            } else {
                Some(ir.sources[src_id.index()].lines.line(ip))
            };
            out.push(StackElem {
                module: module.name.clone(),
                operation,
                unit: self.opts.line_numbers.then(|| source.name.clone()),
                line,
            });
        }
        out
    }

    pub(crate) fn error(&self, kind: ErrorKind, message: Option<String>) -> Unwind {
        let type_class = match &kind {
            ErrorKind::AssertionFailed => Some(self.prog.ir.std_classes.assertion_failed),
            ErrorKind::Raised(c) => *c,
            _ => None,
        };
        Unwind::from(QvtError {
            kind,
            message,
            stack: self.stack_trace(),
            type_class,
        })
    }

    pub(crate) fn runtime_error(&self, msg: impl Into<String>) -> Unwind {
        self.error(ErrorKind::Runtime, Some(msg.into()))
    }

    /// A Java runtime exception inside an expression: Eclipse logs it and the expression yields
    /// invalid.
    pub(crate) fn swallow(&mut self, msg: impl Into<String>) -> Value {
        self.errors.push(msg.into());
        Value::Invalid
    }

    // ---- values -----------------------------------------------------------------------------

    pub(crate) fn new_coll(&self, kind: u8) -> Value {
        use qvto_bytecode::insn::coll_kind as k;
        match kind {
            k::SET => Value::Coll(Rc::new(Coll::new_set(&self.env))),
            k::ORDERED_SET => Value::Coll(Rc::new(Coll::new_ordered_set())),
            k::BAG => Value::Coll(Rc::new(Coll::new_bag(&self.env))),
            k::SEQUENCE => Value::Coll(Rc::new(Coll::new_sequence())),
            k::LIST => Value::List(Rc::new(RefCell::new(List::new()))),
            _ => Value::Dict(Rc::new(RefCell::new(Dict::new(&self.env)))),
        }
    }

    /// Type values and the `eClass` that `log` prints of an object are the metamodel's
    /// `EClassifier` objects in Eclipse: their identity hashes are those of the store's
    /// classifier objects (when the store has them), so that a Java run's tape applies.
    fn alias_classifier_identities(&mut self) {
        use qvto_model::Classifier;
        use qvto_types::Type;
        let types = &self.prog.ir.types;
        for i in 0..types.len() {
            let c = match types.get(TypeId(i as u32)) {
                Type::Class(c) => Classifier::Class(*c),
                Type::DataType(d) => Classifier::DataType(*d),
                Type::Enum(e) => Classifier::Enum(*e),
                _ => continue,
            };
            if let Some(o) = self.store.classifier_object(c) {
                let r = ObjRef::host(o.to_u32());
                self.env
                    .alias(IdentityKey::Type(TypeRef(i as u32)), IdentityKey::Obj(r));
                self.type_objects.insert(TypeId(i as u32), r);
                self.object_types.insert(r, TypeId(i as u32));
            }
        }
        // the `eClass` of a printed object (`log.rs`)
        for c in 0..self.prog.ir.meta.class_count() {
            let c = ClassId(c as u32);
            if let Some(o) = self.store.classifier_object(Classifier::Class(c)) {
                let target = IdentityKey::Obj(ObjRef::host(o.to_u32()));
                self.env
                    .alias(IdentityKey::Type(TypeRef(0x4000_0000 | c.0)), target);
            }
        }
    }

    pub(crate) fn step(&mut self) -> VmResult<()> {
        self.steps += 1;
        if let Some(l) = self.opts.step_limit
            && self.steps > l
        {
            return Err(self.error(ErrorKind::Interrupted, None));
        }
        Ok(())
    }

    /// A compiled `Pattern.matches` pattern (cached).
    pub(crate) fn regex(&self, p: &Str) -> Result<regex::Regex, String> {
        self.regex_kind(p, true)
    }

    /// A compiled pattern for `replaceAll`/`replaceFirst` (cached).
    pub(crate) fn regex_find(&self, p: &Str) -> Result<regex::Regex, String> {
        self.regex_kind(p, false)
    }

    fn regex_kind(&self, p: &Str, full: bool) -> Result<regex::Regex, String> {
        let key = (p.to_str_lossy().into_owned(), full);
        if let Some(r) = self.regexes.borrow().get(&key) {
            return r.clone();
        }
        let r = if full {
            crate::jregex::compile_full(&key.0)
        } else {
            crate::jregex::compile_find(&key.0)
        };
        self.regexes.borrow_mut().insert(key, r.clone());
        r
    }

    // ---- execution --------------------------------------------------------------------------

    /// Runs a chunk on the frame at `base` (registers) and `ibase` (iteration slots) until it
    /// returns; `try` handlers of the chunk catch QVT exceptions.
    pub(crate) fn exec(&mut self, cid: ChunkId, base: usize, ibase: usize) -> VmResult<Value> {
        self.exec_at(cid, base, ibase, 0)
    }

    /// [`Vm::exec`] from an entry point inside the chunk.
    pub(crate) fn exec_at(
        &mut self,
        cid: ChunkId,
        base: usize,
        ibase: usize,
        start: u32,
    ) -> VmResult<Value> {
        let prog = self.prog;
        let chunk = prog.chunk(cid);
        let mut pc: u32 = start;
        loop {
            match self.run(cid, base, ibase, &mut pc) {
                Ok(v) => return Ok(v),
                Err(u) => {
                    let e = u.error();
                    if !e.catchable() || chunk.handlers.is_empty() {
                        return Err(u);
                    }
                    match self.find_catch(cid, pc, e) {
                        Some((target, var)) => {
                            if let Some(var) = var {
                                let exc = self.exception_object(e);
                                w!(self, base, var) = exc;
                            }
                            pc = target;
                        }
                        None => return Err(u),
                    }
                }
            }
        }
    }

    fn find_catch(&self, cid: ChunkId, pc: u32, e: &QvtError) -> Option<(u32, Option<u16>)> {
        let chunk = self.prog.chunk(cid);
        for h in &chunk.handlers {
            if pc < h.start || pc >= h.end {
                continue;
            }
            for c in &h.catches {
                if c.types.is_empty() {
                    return Some((c.target, c.var));
                }
                let exc = match &e.kind {
                    ErrorKind::Raised(t) => *t,
                    ErrorKind::AssertionFailed => e.type_class,
                    _ => None,
                };
                if let Some(et) = exc
                    && c.types
                        .iter()
                        .any(|t| self.prog.ir.meta.is_subclass(et, *t))
                {
                    return Some((c.target, c.var));
                }
            }
            // a handler that does not match lets outer handlers try
        }
        None
    }

    #[inline(never)]
    fn run(&mut self, cid: ChunkId, base: usize, ibase: usize, pcr: &mut u32) -> VmResult<Value> {
        let prog = self.prog;
        let chunk = prog.chunk(cid);
        let code = &chunk.code[..];
        let mut pc = *pcr as usize;
        macro_rules! set_pc {
            () => {
                *pcr = pc as u32;
                if let Some(f) = self.frames.last_mut() {
                    f.pc = pc as u32;
                }
            };
        }
        macro_rules! fail {
            ($e:expr) => {{
                set_pc!();
                return Err($e);
            }};
        }
        loop {
            debug_assert!(pc < code.len());
            // SAFETY: every chunk ends with `Return` or a jump, and jump targets are inside it
            let insn = unsafe { *code.get_unchecked(pc) };
            match insn {
                Insn::Nop => {}
                Insn::Move { dst, src } => {
                    let v = r!(self, base, src).clone();
                    w!(self, base, dst) = v;
                }
                Insn::Take { dst, src } => {
                    let v = std::mem::take(&mut w!(self, base, src));
                    w!(self, base, dst) = v;
                }
                Insn::Add { dst, a, b } => {
                    let v = match (&r!(self, base, a), &r!(self, base, b)) {
                        (Value::Int(x), Value::Int(y)) => Value::Int(x.wrapping_add(*y)),
                        (Value::Real(x), Value::Real(y)) => Value::Real(x + y),
                        _ => match self.slow_bin(Prim::Add, StdOp::Real_add, base, a, b) {
                            Ok(v) => v,
                            Err(e) => fail!(e),
                        },
                    };
                    w!(self, base, dst) = v;
                }
                Insn::Sub { dst, a, b } => {
                    let v = match (&r!(self, base, a), &r!(self, base, b)) {
                        (Value::Int(x), Value::Int(y)) => Value::Int(x.wrapping_sub(*y)),
                        (Value::Real(x), Value::Real(y)) => Value::Real(x - y),
                        _ => match self.slow_bin(Prim::Sub, StdOp::Real_sub, base, a, b) {
                            Ok(v) => v,
                            Err(e) => fail!(e),
                        },
                    };
                    w!(self, base, dst) = v;
                }
                Insn::Mul { dst, a, b } => {
                    let v = match (&r!(self, base, a), &r!(self, base, b)) {
                        (Value::Int(x), Value::Int(y)) => Value::Int(x.wrapping_mul(*y)),
                        (Value::Real(x), Value::Real(y)) => Value::Real(x * y),
                        _ => match self.slow_bin(Prim::Mul, StdOp::Real_mul, base, a, b) {
                            Ok(v) => v,
                            Err(e) => fail!(e),
                        },
                    };
                    w!(self, base, dst) = v;
                }
                Insn::Lt { dst, a, b }
                | Insn::Le { dst, a, b }
                | Insn::Gt { dst, a, b }
                | Insn::Ge { dst, a, b } => {
                    let fast = match (&r!(self, base, a), &r!(self, base, b)) {
                        (Value::Int(x), Value::Int(y)) => Some(match insn {
                            Insn::Lt { .. } => x < y,
                            Insn::Le { .. } => x <= y,
                            Insn::Gt { .. } => x > y,
                            _ => x >= y,
                        }),
                        (Value::Real(x), Value::Real(y)) => Some(match insn {
                            Insn::Lt { .. } => x < y,
                            Insn::Le { .. } => x <= y,
                            Insn::Gt { .. } => x > y,
                            _ => x >= y,
                        }),
                        _ => None,
                    };
                    let v = match fast {
                        Some(b) => Value::Bool(b),
                        None => {
                            let (prim, op) = match insn {
                                Insn::Lt { .. } => (Prim::Lt, StdOp::Real_lt),
                                Insn::Le { .. } => (Prim::Le, StdOp::Real_le),
                                Insn::Gt { .. } => (Prim::Gt, StdOp::Real_gt),
                                _ => (Prim::Ge, StdOp::Real_ge),
                            };
                            match self.slow_bin(prim, op, base, a, b) {
                                Ok(v) => v,
                                Err(e) => fail!(e),
                            }
                        }
                    };
                    w!(self, base, dst) = v;
                }
                Insn::EqNum { dst, a, b } | Insn::NeNum { dst, a, b } => {
                    // the operands promoted (copies: they may be variables' registers)
                    let pa = higher_precision(&r!(self, base, a));
                    let pb = higher_precision(&r!(self, base, b));
                    let va = pa.as_ref().unwrap_or(&r!(self, base, a));
                    let vb = pb.as_ref().unwrap_or(&r!(self, base, b));
                    let fast = match (va, vb) {
                        (Value::Int(x), Value::Int(y)) => Some(x == y),
                        (Value::Real(x), Value::Real(y)) => Some(x == y),
                        (Value::Int(x), Value::Real(y)) | (Value::Real(y), Value::Int(x)) => {
                            Some(*x as f64 == *y)
                        }
                        _ => None,
                    };
                    let ne = matches!(insn, Insn::NeNum { .. });
                    let v = match fast {
                        Some(e) => Value::Bool(e != ne),
                        None => {
                            let (prim, op) = if ne {
                                (Prim::Ne, StdOp::OclAny_ne)
                            } else {
                                (Prim::Eq, StdOp::OclAny_eq)
                            };
                            match self.slow_bin(prim, op, base, a, b) {
                                Ok(v) => v,
                                Err(e) => fail!(e),
                            }
                        }
                    };
                    w!(self, base, dst) = v;
                }
                Insn::Eq { dst, a, b } | Insn::Ne { dst, a, b } => {
                    let fast = match (&r!(self, base, a), &r!(self, base, b)) {
                        (Value::Int(x), Value::Int(y)) => Some(x == y),
                        (Value::Bool(x), Value::Bool(y)) => Some(x == y),
                        (Value::Str(x), Value::Str(y)) => Some(x == y),
                        (Value::Obj(x), Value::Obj(y)) => Some(x == y),
                        (Value::Enum(x), Value::Enum(y)) => Some(x == y),
                        _ => None,
                    };
                    let ne = matches!(insn, Insn::Ne { .. });
                    let v = match fast {
                        Some(e) => Value::Bool(e != ne),
                        None => {
                            let (prim, op) = if ne {
                                (Prim::Ne, StdOp::OclAny_ne)
                            } else {
                                (Prim::Eq, StdOp::OclAny_eq)
                            };
                            match self.slow_bin(prim, op, base, a, b) {
                                Ok(v) => v,
                                Err(e) => fail!(e),
                            }
                        }
                    };
                    w!(self, base, dst) = v;
                }
                Insn::Not { dst, src } => {
                    let v = match &r!(self, base, src) {
                        Value::Bool(b) => Value::Bool(!b),
                        _ => Value::Invalid,
                    };
                    w!(self, base, dst) = v;
                }
                Insn::LoadNull { dst } => w!(self, base, dst) = Value::Null,
                Insn::LoadInvalid { dst } => w!(self, base, dst) = Value::Invalid,
                Insn::LoadBool { dst, v } => w!(self, base, dst) = Value::Bool(v),
                Insn::LoadInt { dst, v } => w!(self, base, dst) = Value::Int(v as i64),
                Insn::LoadConst { dst, k } => {
                    let v = match &self.consts[k as usize] {
                        Some(v) => v.clone(),
                        None => {
                            // a type literal of a metamodel classifier is its `EClassifier`
                            // object (Ecore's OCL environment), when the store has one
                            let v = match &prog.consts[k as usize] {
                                Const::Type(t) if self.type_objects.contains_key(t) => {
                                    Value::Obj(self.type_objects[t])
                                }
                                c => const_value(c),
                            };
                            self.consts[k as usize] = Some(v.clone());
                            v
                        }
                    };
                    w!(self, base, dst) = v;
                }
                Insn::LoadDefault { dst, ty } => {
                    w!(self, base, dst) = self.default_value(TypeId(ty));
                }
                Insn::LoadThis { dst, module } => {
                    w!(self, base, dst) = Value::Instance(module as u32)
                }
                Insn::LoadModel { dst, module, index } => {
                    let v = match self.param_extents.get(&(module as u32, index as u32)) {
                        Some(&e) => Value::Model(e as u32),
                        None => Value::Null,
                    };
                    w!(self, base, dst) = v;
                }
                Insn::LoadGlobal { dst, g } => {
                    w!(self, base, dst) = self.globals[g as usize].clone();
                }
                Insn::CopyColl { r } => {
                    // a copy reorders only Java-ordered storage (compat mode)
                    if self.opts.mode == qvto_runtime::Mode::Compat
                        && let Value::Coll(c) = &r!(self, base, r)
                    {
                        let kind = c.kind();
                        let v = std::mem::take(&mut w!(self, base, r));
                        w!(self, base, r) = qvto_runtime::coll::ops::as_same(v, kind, &self.env);
                    }
                }
                Insn::StoreGlobal { src, g } => {
                    self.globals[g as usize] = r!(self, base, src).clone();
                }
                Insn::JumpIfFastMode { target } => {
                    // Eclipse's double evaluation is kept in compat and ordered mode
                    if !self.opts.mode.eclipse_semantics() {
                        pc = target as usize;
                        continue;
                    }
                }
                Insn::SetNext { coll, idx, val } => {
                    let i = match r!(self, base, idx) {
                        Value::Int(i) => i,
                        _ => 0,
                    };
                    let v = r!(self, base, val).clone();
                    if let Value::Coll(c) = &mut w!(self, base, coll)
                        && let qvto_runtime::coll::Coll::Sequence(items) = std::rc::Rc::make_mut(c)
                        && let Some(x) = items.get_mut(i as usize)
                    {
                        *x = v;
                    }
                    w!(self, base, idx) = Value::Int(i + 1);
                }
                Insn::LoadInstGlobal { dst, inst, g } => {
                    let v = self.instance_global(&r!(self, base, inst), GlobalId(g as u32));
                    w!(self, base, dst) = v;
                }
                Insn::StoreInstGlobal { src, inst, g } => {
                    let i = r!(self, base, inst).clone();
                    let v = r!(self, base, src).clone();
                    self.set_instance_global(&i, GlobalId(g as u32), v);
                }
                Insn::Jump { target } => {
                    if (target as usize) <= pc
                        && let Err(e) = self.step()
                    {
                        fail!(e)
                    }
                    pc = target as usize;
                    continue;
                }
                Insn::JumpIfTrue { r, target } => {
                    if matches!(r!(self, base, r), Value::Bool(true)) {
                        pc = target as usize;
                        continue;
                    }
                }
                Insn::JumpIfNotTrue { r, target } => {
                    if !matches!(r!(self, base, r), Value::Bool(true)) {
                        pc = target as usize;
                        continue;
                    }
                }
                Insn::JumpIfFalse { r, target } => {
                    if matches!(r!(self, base, r), Value::Bool(false)) {
                        pc = target as usize;
                        continue;
                    }
                }
                Insn::JumpIfUndef { r, target } => {
                    if r!(self, base, r).is_undefined() {
                        pc = target as usize;
                        continue;
                    }
                }
                Insn::JumpIfInvalid { r, target } => {
                    if r!(self, base, r).is_invalid() {
                        pc = target as usize;
                        continue;
                    }
                }
                Insn::JumpIfNull { r, target } => {
                    if r!(self, base, r).is_null() {
                        pc = target as usize;
                        continue;
                    }
                }
                Insn::JumpIfNotBool { r, target } => {
                    if !matches!(r!(self, base, r), Value::Bool(_)) {
                        pc = target as usize;
                        continue;
                    }
                }
                Insn::Return { src } => {
                    *pcr = pc as u32;
                    return Ok(std::mem::take(&mut w!(self, base, src)));
                }
                Insn::CallStd { dst, base: b, op } => {
                    set_pc!();
                    let op = StdOp::all()[op as usize];
                    let at = base + b as usize;
                    match self.call_std(op, at, false, false) {
                        Ok(v) => w!(self, base, dst) = v,
                        Err(e) => fail!(e),
                    }
                }
                Insn::CallStdX { dst, base: b, site } => {
                    set_pc!();
                    let Site::Std {
                        op,
                        src_unlimited,
                        arg_unlimited,
                        src_ty,
                        res_ty,
                    } = chunk.sites[site as usize]
                    else {
                        unreachable!()
                    };
                    let at = base + b as usize;
                    self.std_src_ty = src_ty;
                    self.std_res_ty = res_ty;
                    match self.call_std(op, at, src_unlimited, arg_unlimited) {
                        Ok(v) => w!(self, base, dst) = v,
                        Err(e) => fail!(e),
                    }
                }
                Insn::Call { dst, base: b, op } | Insn::CallVirtual { dst, base: b, op } => {
                    set_pc!();
                    if let Err(e) = self.step() {
                        fail!(e)
                    }
                    let virt = matches!(insn, Insn::CallVirtual { .. });
                    let at = base + b as usize;
                    match self.call_op(OpId(op as u32), at, virt) {
                        Ok(v) => w!(self, base, dst) = v,
                        Err(e) => fail!(e),
                    }
                }
                Insn::CallMapping { dst, base: b, site } => {
                    set_pc!();
                    if let Err(e) = self.step() {
                        fail!(e)
                    }
                    let Site::Mapping {
                        op,
                        strict,
                        virtual_,
                    } = chunk.sites[site as usize]
                    else {
                        unreachable!()
                    };
                    let at = base + b as usize;
                    match self.call_mapping_site(op, at, strict, virtual_) {
                        Ok(v) => w!(self, base, dst) = v,
                        Err(e) => fail!(e),
                    }
                }
                Insn::CallMeta { dst, base: b, site } => {
                    set_pc!();
                    let Site::Meta(m) = chunk.sites[site as usize] else {
                        unreachable!()
                    };
                    let at = base + b as usize;
                    match self.call_meta(m, at) {
                        Ok(v) => w!(self, base, dst) = v,
                        Err(e) => fail!(e),
                    }
                }
                Insn::New { dst, base: b, site } => {
                    set_pc!();
                    let at = base + b as usize;
                    match self.instantiate(&chunk.sites[site as usize], at) {
                        Ok(v) => w!(self, base, dst) = v,
                        Err(e) => fail!(e),
                    }
                }
                Insn::NewTransformation { dst, base: b, site } => {
                    set_pc!();
                    let Site::NewTransformation { module, nargs } = chunk.sites[site as usize]
                    else {
                        unreachable!()
                    };
                    let at = base + b as usize;
                    match self.new_transformation(module, at, nargs as usize) {
                        Ok(v) => w!(self, base, dst) = v,
                        Err(e) => fail!(e),
                    }
                }
                Insn::GetProp { dst, obj, feature } => {
                    let o = std::mem::take(&mut w!(self, base, obj));
                    let res = self.get_prop(&o, FeatureId(feature as u32));
                    w!(self, base, obj) = o;
                    match res {
                        Ok(v) => w!(self, base, dst) = v,
                        Err(e) => fail!(e),
                    }
                }
                Insn::GetPropW { dst, obj, site } => {
                    let Site::Feature(f) = chunk.sites[site as usize] else {
                        unreachable!()
                    };
                    let o = r!(self, base, obj).clone();
                    match self.get_prop(&o, f) {
                        Ok(v) => w!(self, base, dst) = v,
                        Err(e) => fail!(e),
                    }
                }
                Insn::GetTuple { dst, src, site } => {
                    let Site::Name(name) = &chunk.sites[site as usize] else {
                        unreachable!()
                    };
                    let v = match &r!(self, base, src) {
                        Value::Tuple(t) => t.get(name).cloned().unwrap_or(Value::Null),
                        _ => Value::Invalid,
                    };
                    w!(self, base, dst) = v;
                }
                Insn::GetInterm { dst, obj, site } => {
                    let Site::Interm(p) = chunk.sites[site as usize] else {
                        unreachable!()
                    };
                    let o = r!(self, base, obj).clone();
                    w!(self, base, dst) = self.get_interm(&o, p);
                }
                Insn::Assign { dst, src, site } => {
                    let Site::Assign {
                        ty,
                        add,
                        same_elem,
                        property,
                    } = chunk.sites[site as usize]
                    else {
                        unreachable!()
                    };
                    let old = std::mem::take(&mut w!(self, base, dst));
                    let rhs = std::mem::take(&mut w!(self, base, src));
                    w!(self, base, dst) = if property && rhs.is_undefined() {
                        self.property_undefined(ty, old, rhs, !add)
                    } else if property && !self.prog.ir.types.is_collection_like(ty) {
                        // `callSetter` stores the number as the property's type
                        let v = self.assign_fast(ty, old, rhs, !add, same_elem);
                        crate::objects::convert_number(v, self.num_class_of_type(ty))
                    } else {
                        self.assign_fast(ty, old, rhs, !add, same_elem)
                    };
                }
                Insn::Coerce { r, site } => {
                    let Site::Type(ty) = chunk.sites[site as usize] else {
                        unreachable!()
                    };
                    let v = std::mem::take(&mut w!(self, base, r));
                    w!(self, base, r) = self.list_coercion(ty, v);
                }
                Insn::SetProp { obj, src, site } => {
                    set_pc!();
                    let Site::SetProp { feature, add } = chunk.sites[site as usize] else {
                        unreachable!()
                    };
                    let o = r!(self, base, obj).clone();
                    let v = r!(self, base, src).clone();
                    if let Err(e) = self.set_prop(&o, feature, v, !add) {
                        fail!(e)
                    }
                }
                Insn::SetInterm { obj, src, site } => {
                    let Site::SetInterm { prop, add } = chunk.sites[site as usize] else {
                        unreachable!()
                    };
                    let o = r!(self, base, obj).clone();
                    let v = r!(self, base, src).clone();
                    self.set_interm(&o, prop, v, !add);
                }
                Insn::NewColl { dst, kind } => {
                    w!(self, base, dst) = self.new_coll(kind);
                }
                Insn::NewCollLike { dst, src } => {
                    let v = self.new_coll_like(&r!(self, base, src));
                    w!(self, base, dst) = v;
                }
                Insn::LitAdd { coll, val } => {
                    let v = r!(self, base, val).clone();
                    let c = std::mem::take(&mut w!(self, base, coll));
                    w!(self, base, coll) = self.lit_add(c, v);
                }
                Insn::LitRange { coll, lo } => {
                    let a = r!(self, base, lo).clone();
                    let b = r!(self, base, lo + 1).clone();
                    let c = std::mem::take(&mut w!(self, base, coll));
                    match self.lit_range(c, a, b) {
                        Ok(v) => w!(self, base, coll) = v,
                        Err(e) => fail!(e),
                    }
                }
                Insn::LitPut { dict, key } => {
                    let k = r!(self, base, key).clone();
                    let v = r!(self, base, key + 1).clone();
                    if !k.is_invalid()
                        && !v.is_invalid()
                        && let Value::Dict(d) = &r!(self, base, dict)
                    {
                        d.borrow_mut().put(k, v, &self.env);
                    }
                }
                Insn::NewTuple { dst, base: b, site } => {
                    let Site::Tuple { ty, order } = &chunk.sites[site as usize] else {
                        unreachable!()
                    };
                    let at = base + b as usize;
                    w!(self, base, dst) = self.new_tuple(*ty, order, at);
                }
                Insn::Push { coll, val } => {
                    let v = r!(self, base, val).clone();
                    let env = &self.env;
                    push(&mut self.regs[base + coll as usize], v, env);
                }
                Insn::PushFlat { coll, val } => {
                    let v = r!(self, base, val).clone();
                    let env = &self.env;
                    push_flat(&mut self.regs[base + coll as usize], v, env);
                }
                Insn::PushNew { dst, coll, val } => {
                    let v = r!(self, base, val).clone();
                    let env = &self.env;
                    let added = match &mut self.regs[base + coll as usize] {
                        Value::Coll(c) => Rc::make_mut(c).add(v, env),
                        _ => true,
                    };
                    w!(self, base, dst) = Value::Bool(added);
                }
                Insn::IterInit { slot, src } => {
                    self.iters[ibase + slot as usize] = match &r!(self, base, src) {
                        Value::Coll(c)
                            if matches!(&**c, Coll::Sequence(_) | Coll::OrderedSet(_)) =>
                        {
                            IterSlot::Coll(c.clone(), 0)
                        }
                        v => IterSlot::Iter(Iter::of(v)),
                    };
                }
                Insn::IterNext { lp, target } => {
                    let info = chunk.loops[lp as usize];
                    let next = match &mut self.iters[ibase + info.slot as usize] {
                        IterSlot::Iter(it) => {
                            if it.pos < it.items.len() {
                                it.pos += 1;
                                Some(it.items[it.pos - 1].clone())
                            } else {
                                None
                            }
                        }
                        IterSlot::Coll(c, pos) => match &**c {
                            Coll::Sequence(v) => {
                                let x = v.get(*pos).cloned();
                                *pos += 1;
                                x
                            }
                            Coll::OrderedSet(m) => m.next_from(pos).cloned(),
                            _ => None,
                        },
                        _ => None,
                    };
                    match next {
                        Some(v) => w!(self, base, info.var) = v,
                        None => {
                            pc = target as usize;
                            continue;
                        }
                    }
                    if let Err(e) = self.step() {
                        fail!(e)
                    }
                }
                Insn::ClosureInit { slot, src } => {
                    let it = Iter::of(&r!(self, base, src));
                    self.iters[ibase + slot as usize] = IterSlot::Closure(vec![it]);
                }
                Insn::ClosureNext { lp, target } => {
                    let info = chunk.loops[lp as usize];
                    let mut found = None;
                    while let IterSlot::Closure(levels) =
                        &mut self.iters[ibase + info.slot as usize]
                    {
                        let depth = levels.len();
                        let Some(top) = levels.last_mut() else {
                            break;
                        };
                        if top.pos >= top.items.len() {
                            levels.pop();
                            continue;
                        }
                        top.pos += 1;
                        let x = top.items[top.pos - 1].clone();
                        if depth > 1 {
                            let env = &self.env;
                            let added = match &mut self.regs[base + info.acc as usize] {
                                Value::Coll(c) => Rc::make_mut(c).add(x.clone(), env),
                                _ => false,
                            };
                            if !added {
                                continue;
                            }
                        }
                        found = Some(x);
                        break;
                    }
                    match found {
                        Some(x) => w!(self, base, info.var) = x,
                        None => {
                            pc = target as usize;
                            continue;
                        }
                    }
                    if let Err(e) = self.step() {
                        fail!(e)
                    }
                }
                Insn::ClosurePush { slot, val } => {
                    let v = r!(self, base, val).clone();
                    if !v.is_null()
                        && let IterSlot::Closure(levels) = &mut self.iters[ibase + slot as usize]
                    {
                        levels.push(Iter::of(&v));
                    }
                }
                Insn::SortBy { dst, elems, site } => {
                    let Site::Sort {
                        ordered_set,
                        unlimited,
                    } = chunk.sites[site as usize]
                    else {
                        unreachable!()
                    };
                    let es = r!(self, base, elems).clone();
                    let ks = r!(self, base, elems + 1).clone();
                    w!(self, base, dst) = self.sort_by(es, ks, ordered_set, unlimited);
                }
                Insn::KindTest { dst, src, site } => {
                    let Site::Type(t) = chunk.sites[site as usize] else {
                        unreachable!()
                    };
                    let v = r!(self, base, src).clone();
                    w!(self, base, dst) = Value::Bool(self.qvt_kind_of(&v, t));
                }
                Insn::ObjectInit { var, site } => {
                    set_pc!();
                    let Site::Object {
                        class,
                        extent,
                        fail,
                    } = chunk.sites[site as usize]
                    else {
                        unreachable!()
                    };
                    let cur = r!(self, base, var).clone();
                    // an object expression of a collection type (a mapping's `List` result)
                    // keeps the collection unchecked (`getOutOwner`); one of a tuple type (a
                    // mapping's `Tuple` result, which the IR gives no class) conforms
                    let collection = matches!(
                        cur,
                        Value::Coll(_) | Value::List(_) | Value::Dict(_) | Value::Tuple(_)
                    );
                    if collection {
                    } else if !cur.is_null() {
                        let ty_ok = self.value_conforms_to_class(&cur, class);
                        if !ty_ok {
                            self.swallow(format!(
                                "Cannot cast object of type {} to {}",
                                self.type_of_name(&cur),
                                self.prog.ir.meta.class(class).name
                            ));
                            pc = fail as usize;
                            continue;
                        }
                    } else {
                        match self.create_object(class, extent) {
                            Ok(o) => w!(self, base, var) = o,
                            Err(e) => fail!(e),
                        }
                    }
                }
                Insn::Resolve { dst, base: b, site } => {
                    let src = r!(self, base, b).clone();
                    w!(self, base, dst) = self.resolve(&chunk.sites[site as usize], src);
                }
                Insn::LateResolve { base: b, site } => {
                    let owner = r!(self, base, b).clone();
                    let source = r!(self, base, b + 1).clone();
                    self.late.push(LateTask {
                        chunk: cid,
                        site,
                        owner,
                        source,
                    });
                }
                Insn::Log { base: b, site } => {
                    let Site::Log {
                        has_level,
                        has_element,
                    } = chunk.sites[site as usize]
                    else {
                        unreachable!()
                    };
                    let at = base + b as usize;
                    self.do_log(at, has_level, has_element, None);
                }
                Insn::AssertFail { base: b, site } => {
                    set_pc!();
                    let at = base + b as usize;
                    if let Err(e) = self.assert_failed(&chunk.sites[site as usize], at) {
                        fail!(e)
                    }
                }
                Insn::Raise { src, site } => {
                    set_pc!();
                    let Site::Raise { class, .. } = chunk.sites[site as usize] else {
                        unreachable!()
                    };
                    let msg = match &r!(self, base, src) {
                        Value::Str(s) => Some(s.to_str_lossy().into_owned()),
                        _ => None,
                    };
                    fail!(self.error(ErrorKind::Raised(class), msg))
                }
                Insn::Unsupported { site } => {
                    set_pc!();
                    let Site::Unsupported(m) = &chunk.sites[site as usize] else {
                        unreachable!()
                    };
                    fail!(self.runtime_error(m.to_string()))
                }
            }
            pc += 1;
        }
    }

    /// A binary operator on operands that are not both Integers or Reals.
    fn slow_bin(&mut self, prim: Prim, op: StdOp, base: usize, a: u16, b: u16) -> VmResult<Value> {
        let x = self.regs[base + a as usize].clone();
        let y = self.regs[base + b as usize].clone();
        self.prim_call(prim, op, vec![x, y])
    }

    /// `createNewCollectionOfSameKind` (QVTo's: List and Dict keep their kind).
    pub(crate) fn new_coll_like(&self, v: &Value) -> Value {
        use qvto_bytecode::insn::coll_kind as k;
        match v {
            Value::Coll(c) => self.new_coll(match c.kind() {
                CollKind::Set => k::SET,
                CollKind::OrderedSet => k::ORDERED_SET,
                CollKind::Bag => k::BAG,
                CollKind::Sequence => k::SEQUENCE,
            }),
            Value::List(_) => self.new_coll(k::LIST),
            Value::Dict(_) => self.new_coll(k::DICT),
            _ => self.new_coll(k::SEQUENCE),
        }
    }

    fn lit_add(&self, c: Value, v: Value) -> Value {
        if c.is_invalid() || v.is_invalid() {
            return Value::Invalid;
        }
        let mut c = c;
        push(&mut c, v, &self.env);
        c
    }

    fn lit_range(&self, c: Value, a: Value, b: Value) -> VmResult<Value> {
        if c.is_invalid() {
            return Ok(c);
        }
        let (Value::Int(lo), Value::Int(hi)) = (&a, &b) else {
            // null bounds: the part is skipped; other bounds: Java casts fail
            if a.is_invalid() || b.is_invalid() {
                return Ok(Value::Invalid);
            }
            return Ok(c);
        };
        let (lo, hi) = (*lo as i32, *hi as i32);
        let mut c = c;
        if lo <= hi {
            self.check_collection_size(hi as i64 - lo as i64 + 1 + coll_len(&c) as i64)?;
            for i in lo..=hi {
                push(&mut c, Value::Int(i as i64), &self.env);
            }
        }
        Ok(c)
    }

    /// Fails with an interruption when a collection of `n` elements is beyond the limit.
    pub(crate) fn check_collection_size(&self, n: i64) -> VmResult<()> {
        match self.opts.max_collection_size {
            Some(m) if n > m as i64 => Err(self.error(
                ErrorKind::Interrupted,
                Some(format!(
                    "resource limit exceeded: a collection of {n} elements (limit {m})"
                )),
            )),
            _ => Ok(()),
        }
    }

    /// The size limits for a value an operation built (collections, strings); building it is
    /// also charged to the step budget (one step per 256 elements or characters), so that a
    /// loop that grows a value step by step cannot do quadratic work within its budget.
    pub(crate) fn check_value_size(&mut self, v: &Value) -> VmResult<()> {
        if self.opts.step_limit.is_some() {
            let n = match v {
                Value::Str(s) => s.len16(),
                _ => coll_len(v),
            };
            if n >= 256 {
                self.steps += (n / 256) as u64;
                self.step()?;
            }
        }
        match v {
            Value::Coll(_) | Value::List(_) | Value::Dict(_) => {
                self.check_collection_size(coll_len(v) as i64)
            }
            Value::Str(s) => match self.opts.max_string_length {
                Some(m) if s.len16() > m => Err(self.error(
                    ErrorKind::Interrupted,
                    Some(format!(
                        "resource limit exceeded: a string of {} characters (limit {m})",
                        s.len16()
                    )),
                )),
                _ => Ok(()),
            },
            _ => Ok(()),
        }
    }

    fn new_tuple(&self, ty: TypeId, order: &[u16], at: usize) -> Value {
        let qvto_types::Type::Tuple(parts) = self.prog.ir.types.get(ty) else {
            return Value::Invalid;
        };
        let names: Rc<[Str]> = parts.iter().map(|(n, _)| Str::new(n)).collect();
        let mut values = vec![Value::Null; parts.len()];
        for (i, &k) in order.iter().enumerate() {
            // an `invalid` part stays a part (classic OCL's `visitTupleLiteralExp`)
            let v = self.regs[at + i].clone();
            values[k as usize] = coerce_number(v);
        }
        Value::Tuple(Rc::new(qvto_runtime::tuple::Tuple::new(
            ty.0,
            names,
            values.into_boxed_slice(),
        )))
    }

    fn exception_object(&mut self, e: &QvtError) -> Value {
        let class = match e.kind {
            ErrorKind::Raised(Some(c)) => c,
            ErrorKind::AssertionFailed => self.prog.ir.std_classes.assertion_failed,
            _ => self.prog.ir.std_classes.exception,
        };
        let o = self.new_local(class);
        if let Some(idx) = o.is_local().then(|| o.index()) {
            self.local[idx as usize].exception = Some((e.message.clone(), e.stack.clone()));
        }
        Value::Obj(o)
    }
}

/// `coerceNumber` of a tuple part (`AbstractTuple.getValue`).
fn coerce_number(v: Value) -> Value {
    match v {
        Value::Boxed(_, i) => Value::Int(i as i64),
        Value::Float(f) => Value::Real(f as f64),
        v => v,
    }
}

/// Java `add` into the collection being built in `slot` (copy on write when shared).
pub(crate) fn push(slot: &mut Value, v: Value, env: &dyn qvto_runtime::HashEnv) {
    match slot {
        Value::Coll(c) => {
            Rc::make_mut(c).add(v, env);
        }
        Value::List(l) => l.borrow_mut().items.push(v),
        Value::Dict(d) => {
            // DictionaryImpl.add: the element under the key null
            d.borrow_mut().put(Value::Null, v, env);
        }
        _ => {}
    }
}

/// OCL `collect`: `addAll(CollectionUtil.flatten(body))` for collection bodies.
pub(crate) fn push_flat(slot: &mut Value, v: Value, env: &dyn qvto_runtime::HashEnv) {
    match &v {
        Value::Coll(_) | Value::List(_) | Value::Dict(_) => {
            let mut items = Vec::new();
            flatten_into(&v, &mut items);
            for x in items {
                push(slot, x, env);
            }
        }
        _ => push(slot, v, env),
    }
}

fn flatten_into(v: &Value, out: &mut Vec<Value>) {
    match v {
        Value::Coll(c) => {
            for x in c.iter() {
                flatten_into(x, out);
            }
        }
        Value::List(l) => {
            for x in l.borrow().items.iter() {
                flatten_into(x, out);
            }
        }
        Value::Dict(d) => {
            for x in d.borrow().values() {
                flatten_into(&x, out);
            }
        }
        x => out.push(x.clone()),
    }
}

pub(crate) fn const_value(c: &Const) -> Value {
    match c {
        Const::Int(i) => Value::Int(*i),
        Const::Real(x) => Value::Real(*x),
        Const::Str(s) => Value::str(s),
        Const::Enum(e, l) => Value::Enum(EnumLit {
            enum_id: e.0,
            literal: *l,
        }),
        Const::Type(t) => Value::Type(TypeRef(t.0)),
    }
}

/// `NumberUtil.higherPrecisionNumber`: a `Float` as a `Double`, `Short`/`Byte`/`Long` boxes as
/// (long) Integers; `None` for values it leaves alone.
pub(crate) fn higher_precision(v: &Value) -> Option<Value> {
    match v {
        Value::Float(f) => Some(Value::Real(*f as f64)),
        Value::Boxed(
            qvto_runtime::IntBox::Short | qvto_runtime::IntBox::Byte | qvto_runtime::IntBox::Long,
            i,
        ) => Some(Value::Int(*i as i64)),
        _ => None,
    }
}

/// The number of elements of a collection, `List` or `Dict` value (0 otherwise).
pub(crate) fn coll_len(v: &Value) -> usize {
    match v {
        Value::Coll(c) => c.len(),
        Value::List(l) => l.borrow().items.len(),
        Value::Dict(d) => d.borrow().len(),
        _ => 0,
    }
}

/// An address on the current native stack (stacks grow downwards on the supported targets).
#[inline(never)]
pub(crate) fn stack_address() -> usize {
    let x = 0u8;
    std::hint::black_box(&x) as *const u8 as usize
}

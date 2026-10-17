//! The compiler from the typed IR to bytecode.
//!
//! Every operation gets one frame layout: its variables (`LocalId` = register) followed by
//! temporaries, allocated as a stack. Helpers, queries, constructors and entries compile into
//! one chunk; mappings into one chunk per part (`when`, `where`, `init`, population, `end`),
//! all on the same frame. Code bodies outside operations (initialisers, model type `where`)
//! get chunks with frames of their own.
//!
//! OCL and imperative iterators become inline loops over `IterInit`/`IterNext` (several
//! iterator variables: nested loops, the first variable varying fastest, as classic OCL's
//! `IterationTemplate` advances them); `and`/`or`/`implies` become jumps with classic OCL's
//! lax-null rules; `break`/`continue`/`return` become jumps or `Return` (inside an `except`
//! body they leave the `try`, as Eclipse drops them there; inside a top-level statement of
//! `init`/`end` a `return` ends only that statement).
//!
//! The compiler also records Eclipse's instruction pointer per instruction: the start offset
//! of the last AST node entered in evaluation order (Eclipse's interruptible visitor sets it on
//! every node; operation calls reset it to the call after returning). Stack traces and
//! assertion locations use it.

use crate::insn::{Insn, Reg, coll_kind};
use crate::program::*;
use qvto_model::FeatureId;
use qvto_sema::ir::{
    self, Callee, CodeBody, CollPart, Direction, Expr, ExprKind, LValue, Local, LocalId, LoopKind,
    ModuleId, OpId, OpKind, SourceId,
};
use qvto_types::{CollKind, StdOp, Type, TypeId, TypeTable};
use std::collections::HashMap;
use std::fmt;

/// A compile error: the program exceeds a limit of the bytecode format, or the IR is
/// inconsistent.
#[derive(Clone, Debug)]
pub struct CompileError(pub String);

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CompileError {}

type CResult<T> = Result<T, CompileError>;

fn err<T>(m: impl Into<String>) -> CResult<T> {
    Err(CompileError(m.into()))
}

/// Compiles an analysed program.
pub fn compile(ir: ir::Program) -> CResult<Program> {
    let mut consts = ConstPool::default();
    let mut chunks = Vec::new();
    let mut ops = Vec::with_capacity(ir.operations.len());
    for (i, op) in ir.operations.iter().enumerate() {
        let code = compile_op(&ir, OpId(i as u32), op, &mut consts, &mut chunks)?;
        ops.push(code);
    }
    let mut globals = Vec::with_capacity(ir.globals.len());
    for g in &ir.globals {
        let name = format!("{}::{}", ir.modules[g.module.index()].name, g.name);
        globals.push(match &g.init {
            Some(b) => Some(compile_body(
                &ir,
                g.module,
                b,
                &name,
                &mut consts,
                &mut chunks,
            )?),
            None => None,
        });
    }
    let mut interm_inits = Vec::with_capacity(ir.interm_classes.len());
    for c in &ir.interm_classes {
        let mut v = Vec::new();
        for (f, b) in &c.inits {
            let name = format!(
                "{}::{}",
                ir.meta.class(c.class).name,
                ir.meta.feature(*f).name
            );
            v.push((
                *f,
                compile_body(&ir, c.module, b, &name, &mut consts, &mut chunks)?,
            ));
        }
        interm_inits.push(v);
    }
    let mut model_type_where = Vec::with_capacity(ir.model_types.len());
    for mt in &ir.model_types {
        model_type_where.push(match &mt.where_ {
            Some(b) => Some(compile_conditions(
                &ir,
                mt.module,
                b,
                &format!("{}::where", mt.name),
                &mut consts,
                &mut chunks,
            )?),
            None => None,
        });
    }
    // OCL bodies of metamodel operations and derivations (in the main module's environment)
    let mut meta_bodies = Vec::with_capacity(ir.meta_ops.len());
    for mo in &ir.meta_ops {
        meta_bodies.push(match &mo.body {
            Some(b) => {
                let name = format!("{}::{}", ir.meta.class(mo.class).name, mo.name);
                let code = compile_body(&ir, ir.main_module, &b.code, &name, &mut consts, &mut chunks)?;
                Some((code, b.params.iter().map(|l| l.0 as Reg).collect()))
            }
            None => None,
        });
    }
    let mut derivations = HashMap::new();
    for (f, b) in &ir.derivations {
        let fi = ir.meta.feature(*f);
        let name = format!("{}::{}", ir.meta.class(fi.owner).name, fi.name);
        derivations.insert(
            *f,
            compile_body(&ir, ir.main_module, b, &name, &mut consts, &mut chunks)?,
        );
    }
    let overriding = overriding_table(&ir);
    let library_classes = ir
        .meta
        .classes()
        .map(|c| ir.meta.class(c).package == ir.std_classes.package)
        .collect();
    Ok(Program {
        ir,
        consts: consts.consts,
        chunks,
        ops,
        globals,
        interm_inits,
        model_type_where,
        meta_bodies,
        derivations,
        overriding,
        library_classes,
    })
}

/// Per operation: the last operation of the chain of operations that override it (`extends`;
/// the first overriding operation in declaration order at each step), itself if none.
fn overriding_table(ir: &ir::Program) -> Vec<OpId> {
    let n = ir.operations.len();
    let mut first: Vec<Option<u32>> = vec![None; n];
    for (i, o) in ir.operations.iter().enumerate() {
        if let Some(t) = o.overrides
            && first[t.index()].is_none()
        {
            first[t.index()] = Some(i as u32);
        }
    }
    (0..n)
        .map(|op| {
            let mut cur = op as u32;
            for _ in 0..n {
                match first[cur as usize] {
                    Some(i) if i != cur => cur = i,
                    _ => break,
                }
            }
            OpId(cur)
        })
        .collect()
}

#[derive(Default)]
struct ConstPool {
    consts: Vec<Const>,
    strings: HashMap<Box<str>, u32>,
}

impl ConstPool {
    fn add(&mut self, c: Const) -> u32 {
        if let Const::Str(s) = &c {
            if let Some(&k) = self.strings.get(s) {
                return k;
            }
            let k = self.consts.len() as u32;
            self.strings.insert(s.clone(), k);
            self.consts.push(c);
            return k;
        }
        if let Some(k) = self.consts.iter().position(|x| *x == c) {
            return k as u32;
        }
        self.consts.push(c);
        (self.consts.len() - 1) as u32
    }
}

fn compile_op(
    ir: &ir::Program,
    id: OpId,
    op: &ir::Operation,
    consts: &mut ConstPool,
    chunks: &mut Vec<Chunk>,
) -> CResult<OpCode> {
    let module = &ir.modules[op.module.index()];
    let qname = match op.context {
        Some(c) => format!("{}::{}::{}", module.name, type_name(ir, c), op.name),
        None => format!("{}::{}", module.name, op.name),
    };
    let _ = id;
    let mut code = OpCode::default();
    let mut frame = FrameInfo {
        nregs: op.locals.len() as u32,
        n_iters: 0,
    };
    let header_ip = op.decl_span.start;
    let mut emit = |name: &str,
                    f: &mut dyn FnMut(&mut Builder) -> CResult<()>,
                    frame: &mut FrameInfo|
     -> CResult<ChunkId> {
        let mut b = Builder::new(ir, consts, op.module, op.source, &op.locals, name.into())?;
        b.ip = header_ip;
        f(&mut b)?;
        let chunk = b.finish(frame)?;
        chunks.push(chunk);
        Ok(ChunkId((chunks.len() - 1) as u32))
    };
    if let Some(m) = &op.mapping {
        if !m.when.is_empty() {
            code.when = Some(emit(
                &format!("{qname}/when"),
                &mut |b| b.conditions(&m.when),
                &mut frame,
            )?);
        }
        if !m.where_.is_empty() {
            code.where_ = Some(emit(
                &format!("{qname}/where"),
                &mut |b| b.conditions(&m.where_),
                &mut frame,
            )?);
        }
        if !m.init.is_empty() {
            code.init = Some(emit(
                &format!("{qname}/init"),
                &mut |b| b.section(&m.init),
                &mut frame,
            )?);
        }
        code.population = Some(emit(
            &format!("{qname}/population"),
            &mut |b| {
                // `return` ends the population only; the mapping's result is unaffected
                b.statements_then_null(&m.population)
            },
            &mut frame,
        )?);
        if !m.end.is_empty() {
            code.end = Some(emit(
                &format!("{qname}/end"),
                &mut |b| b.section(&m.end),
                &mut frame,
            )?);
        }
    } else if let Some(body) = &op.body {
        let is_void = op.kind == OpKind::Entry && false;
        let _ = is_void;
        code.body = Some(emit(&qname, &mut |b| b.op_body(body), &mut frame)?);
    }
    code.nregs = frame.nregs;
    code.n_iters = frame.n_iters;
    Ok(code)
}

fn compile_body(
    ir: &ir::Program,
    module: ModuleId,
    body: &CodeBody,
    name: &str,
    consts: &mut ConstPool,
    chunks: &mut Vec<Chunk>,
) -> CResult<BodyCode> {
    let mut frame = FrameInfo {
        nregs: body.locals.len() as u32,
        n_iters: 0,
    };
    let mut b = Builder::new(ir, consts, module, body.source, &body.locals, name.into())?;
    b.ip = body.expr.span.start;
    let r = b.alloc()?;
    b.expr(&body.expr, r)?;
    b.emit(Insn::Return { src: r });
    let chunk = b.finish(&mut frame)?;
    chunks.push(chunk);
    Ok(BodyCode {
        chunk: ChunkId((chunks.len() - 1) as u32),
        self_reg: body.self_local.map(|l| l.0 as Reg),
    })
}

/// An expression whose evaluation has no side effects and changes no variable.
fn simple(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Null
        | ExprKind::Invalid
        | ExprKind::Bool(_)
        | ExprKind::Int(_)
        | ExprKind::Real(_)
        | ExprKind::Str(_)
        | ExprKind::Unlimited
        | ExprKind::EnumLit(..)
        | ExprKind::Type(_)
        | ExprKind::Local(_)
        | ExprKind::This(_)
        | ExprKind::Global(_)
        | ExprKind::ModelParam(_) => true,
        ExprKind::Prop(s, _) | ExprKind::TupleProp(s, _) => simple(s),
        _ => false,
    }
}

/// Whether an expression may read or write the variable (conservative: true when unsure).
fn mentions_local(e: &Expr, l: LocalId) -> bool {
    match &e.kind {
        ExprKind::Local(x) => *x == l,
        ExprKind::Null
        | ExprKind::Invalid
        | ExprKind::Bool(_)
        | ExprKind::Int(_)
        | ExprKind::Real(_)
        | ExprKind::Str(_)
        | ExprKind::Unlimited
        | ExprKind::EnumLit(..)
        | ExprKind::Type(_)
        | ExprKind::This(_)
        | ExprKind::Global(_)
        | ExprKind::ModelParam(_) => false,
        ExprKind::Prop(s, _) | ExprKind::TupleProp(s, _) | ExprKind::IntermProp(s, _) => {
            mentions_local(s, l)
        }
        ExprKind::Call(c) => match c.callee {
            Callee::Std(_) => {
                c.source.as_ref().is_some_and(|s| mentions_local(s, l))
                    || c.args.iter().any(|a| mentions_local(a, l))
            }
            _ => true,
        },
        ExprKind::Coll(_, parts) | ExprKind::List(parts) => parts.iter().any(|p| match p {
            CollPart::Item(x) => mentions_local(x, l),
            CollPart::Range(a, b) => mentions_local(a, l) || mentions_local(b, l),
        }),
        _ => true,
    }
}

/// A model type's `where` conditions: each statement of the body must be true.
fn compile_conditions(
    ir: &ir::Program,
    module: ModuleId,
    body: &CodeBody,
    name: &str,
    consts: &mut ConstPool,
    chunks: &mut Vec<Chunk>,
) -> CResult<BodyCode> {
    let mut frame = FrameInfo {
        nregs: body.locals.len() as u32,
        n_iters: 0,
    };
    let mut b = Builder::new(ir, consts, module, body.source, &body.locals, name.into())?;
    b.ip = body.expr.span.start;
    match &body.expr.kind {
        ExprKind::Block(stmts) => b.conditions(stmts)?,
        _ => b.conditions(std::slice::from_ref(&body.expr))?,
    }
    let chunk = b.finish(&mut frame)?;
    chunks.push(chunk);
    Ok(BodyCode {
        chunk: ChunkId((chunks.len() - 1) as u32),
        self_reg: body.self_local.map(|l| l.0 as Reg),
    })
}

pub(crate) fn type_name(ir: &ir::Program, t: TypeId) -> String {
    match ir.types.get(t) {
        Type::Class(c) => ir.meta.class(*c).name.to_string(),
        Type::DataType(d) => ir.meta.data_type(*d).name.to_string(),
        Type::Enum(e) => ir.meta.enumeration(*e).name.to_string(),
        Type::OclAny => "OclAny".into(),
        Type::Boolean => "Boolean".into(),
        Type::Integer => "Integer".into(),
        Type::Real => "Real".into(),
        Type::String => "String".into(),
        Type::UnlimitedNatural => "UnlimitedNatural".into(),
        Type::OclVoid => "OclVoid".into(),
        Type::OclInvalid => "OclInvalid".into(),
        Type::Collection(k, e) => format!("{}({})", k.name(), type_name(ir, *e)),
        Type::List(e) => format!("List({})", type_name(ir, *e)),
        Type::Dict(k, v) => format!("Dict({}, {})", type_name(ir, *k), type_name(ir, *v)),
        Type::Module(m) => ir.modules[*m as usize].name.clone(),
        Type::ModelType(m) => ir.model_types[*m as usize].name.clone(),
        Type::Tuple(_) => "Tuple".into(),
        other => format!("{other:?}"),
    }
}

struct FrameInfo {
    nregs: u32,
    n_iters: u32,
}

/// Control-flow targets of `break`, `continue` and `return`.
enum Ctl {
    Loop {
        breaks: Vec<usize>,
        continue_target: u32,
    },
    /// An `except` body: `break`, `continue` and `return` that would leave it go to the end of
    /// the `try`.
    Barrier { exits: Vec<usize> },
    /// A top-level statement of `init`/`end`: `return` goes to the next statement.
    ReturnTo { exits: Vec<usize> },
}

struct Builder<'a> {
    ir: &'a ir::Program,
    types: &'a TypeTable,
    consts: &'a mut ConstPool,
    chunk: Chunk,
    nlocals: u32,
    next: u32,
    max: u32,
    iter_depth: u32,
    max_iters: u32,
    ctl: Vec<Ctl>,
    ip: u32,
    locals: &'a [Local],
    /// Names of `iterate` result variables whose source is being compiled: classic OCL adds
    /// the result variable to the evaluation environment before it evaluates the source, so a
    /// variable of the same name added meanwhile (another `iterate`'s result, an iterator, a
    /// `let`) fails (`IllegalArgumentException`, the expression is invalid).
    live_names: Vec<String>,
}

impl<'a> Builder<'a> {
    fn new(
        ir: &'a ir::Program,
        consts: &'a mut ConstPool,
        module: ModuleId,
        source: SourceId,
        locals: &'a [Local],
        name: String,
    ) -> CResult<Builder<'a>> {
        if locals.len() >= u16::MAX as usize {
            return err(format!("{name}: too many variables"));
        }
        Ok(Builder {
            ir,
            types: &ir.types,
            consts,
            chunk: Chunk {
                name,
                source,
                module,
                code: Vec::new(),
                ips: Vec::new(),
                sites: Vec::new(),
                loops: Vec::new(),
                handlers: Vec::new(),
                nregs: 0,
                n_iters: 0,
            },
            nlocals: locals.len() as u32,
            next: locals.len() as u32,
            max: locals.len() as u32,
            iter_depth: 0,
            max_iters: 0,
            ctl: Vec::new(),
            ip: u32::MAX,
            locals,
            live_names: Vec::new(),
        })
    }

    fn finish(mut self, frame: &mut FrameInfo) -> CResult<Chunk> {
        self.chunk.nregs = self.max;
        self.chunk.n_iters = self.max_iters;
        frame.nregs = frame.nregs.max(self.max);
        frame.n_iters = frame.n_iters.max(self.max_iters);
        if self.chunk.code.len() >= u32::MAX as usize / 2 {
            return err("chunk too large");
        }
        Ok(self.chunk)
    }

    // ---- emission ---------------------------------------------------------------------------

    fn pc(&self) -> u32 {
        self.chunk.code.len() as u32
    }

    fn emit(&mut self, i: Insn) -> usize {
        self.chunk.code.push(i);
        self.chunk.ips.push(self.ip);
        self.chunk.code.len() - 1
    }

    fn jump(&mut self, i: Insn) -> usize {
        self.emit(i)
    }

    fn patch(&mut self, at: usize) {
        let t = self.pc();
        self.chunk.code[at].set_target(t);
    }

    fn patch_to(&mut self, at: usize, t: u32) {
        self.chunk.code[at].set_target(t);
    }

    fn site(&mut self, s: Site) -> CResult<u16> {
        if self.chunk.sites.len() >= u16::MAX as usize {
            return err(format!("{}: too many sites", self.chunk.name));
        }
        self.chunk.sites.push(s);
        Ok((self.chunk.sites.len() - 1) as u16)
    }

    fn global16(&self, g: qvto_sema::ir::GlobalId) -> CResult<u16> {
        u16::try_from(g.0).or_else(|_| err(format!("{}: too many properties", self.chunk.name)))
    }

    fn konst(&mut self, c: Const) -> u32 {
        self.consts.add(c)
    }

    fn enter(&mut self, e: &Expr) {
        if e.span.end > e.span.start || e.span.start > 0 {
            self.ip = e.span.start;
        }
    }

    // ---- registers --------------------------------------------------------------------------

    fn alloc(&mut self) -> CResult<Reg> {
        self.alloc_n(1)
    }

    fn alloc_n(&mut self, n: u32) -> CResult<Reg> {
        let r = self.next;
        self.next += n;
        if self.next >= u16::MAX as u32 {
            return err(format!("{}: too many registers", self.chunk.name));
        }
        self.max = self.max.max(self.next);
        Ok(r as Reg)
    }

    fn mark(&self) -> u32 {
        self.next
    }

    fn release(&mut self, m: u32) {
        debug_assert!(m >= self.nlocals);
        self.next = m;
    }

    /// Set, OrderedSet, Bag or Sequence (not `Collection`, `List`, `Dict`).
    fn is_ocl_collection(&self, t: TypeId) -> bool {
        matches!(self.types.get(t), Type::Collection(k, _) if *k != CollKind::Collection)
    }

    fn local(&self, l: LocalId) -> Reg {
        l.0 as Reg
    }

    fn loop_slot(&mut self) -> u16 {
        let s = self.iter_depth;
        self.iter_depth += 1;
        self.max_iters = self.max_iters.max(self.iter_depth);
        s as u16
    }

    fn free_slot(&mut self) {
        self.iter_depth -= 1;
    }

    fn loop_info(&mut self, slot: u16, var: Reg, acc: Reg) -> CResult<u16> {
        if self.chunk.loops.len() >= u16::MAX as usize {
            return err("too many loops");
        }
        self.chunk.loops.push(LoopInfo { slot, var, acc });
        Ok((self.chunk.loops.len() - 1) as u16)
    }

    // ---- statements -------------------------------------------------------------------------

    /// `when`/`where`: `true` if every clause is `true`.
    fn conditions(&mut self, conds: &[Expr]) -> CResult<()> {
        let r = self.alloc()?;
        let mut fails = Vec::new();
        for c in conds {
            let m = self.mark();
            let t = self.expr_reg(c)?;
            fails.push(self.jump(Insn::JumpIfNotTrue { r: t, target: 0 }));
            self.release(m);
        }
        self.emit(Insn::LoadBool { dst: r, v: true });
        self.emit(Insn::Return { src: r });
        for f in fails {
            self.patch(f);
        }
        self.emit(Insn::LoadBool { dst: r, v: false });
        self.emit(Insn::Return { src: r });
        Ok(())
    }

    /// `init`/`end`: each top-level statement on its own (a `return` ends that statement).
    fn section(&mut self, stmts: &[Expr]) -> CResult<()> {
        for s in stmts {
            self.ctl.push(Ctl::ReturnTo { exits: Vec::new() });
            self.stmt(s)?;
            let Some(Ctl::ReturnTo { exits }) = self.ctl.pop() else {
                unreachable!()
            };
            for e in exits {
                self.patch(e);
            }
        }
        let r = self.alloc()?;
        self.emit(Insn::LoadNull { dst: r });
        self.emit(Insn::Return { src: r });
        Ok(())
    }

    fn statements_then_null(&mut self, stmts: &[Expr]) -> CResult<()> {
        for s in stmts {
            self.stmt(s)?;
        }
        let r = self.alloc()?;
        self.emit(Insn::LoadNull { dst: r });
        self.emit(Insn::Return { src: r });
        Ok(())
    }

    /// An operation body: the value of the last statement unless a `return` ends it.
    fn op_body(&mut self, stmts: &[Expr]) -> CResult<()> {
        let r = self.alloc()?;
        self.emit(Insn::LoadNull { dst: r });
        for (i, s) in stmts.iter().enumerate() {
            if i + 1 == stmts.len() {
                self.expr(s, r)?;
            } else {
                self.stmt(s)?;
            }
        }
        self.emit(Insn::Return { src: r });
        Ok(())
    }

    fn stmt(&mut self, e: &Expr) -> CResult<()> {
        if let ExprKind::Assign(a) = &e.kind {
            // the value of a statement-level assignment is not needed
            self.enter(e);
            return self.assign(a, None);
        }
        let m = self.mark();
        let r = self.alloc()?;
        self.expr(e, r)?;
        self.release(m);
        Ok(())
    }

    // ---- expressions ------------------------------------------------------------------------

    /// A register holding the value of `e`: a variable's own register, else a new temporary
    /// (released by the caller's mark).
    fn expr_reg(&mut self, e: &Expr) -> CResult<Reg> {
        if let ExprKind::Local(l) = e.kind {
            self.enter(e);
            return Ok(self.local(l));
        }
        let r = self.alloc()?;
        self.expr(e, r)?;
        Ok(r)
    }

    fn is_unlimited(&self, e: &Expr) -> bool {
        e.ty == TypeId::UNLIMITED_NATURAL
    }

    fn expr(&mut self, e: &Expr, dst: Reg) -> CResult<()> {
        self.enter(e);
        match &e.kind {
            ExprKind::Null => {
                self.emit(Insn::LoadNull { dst });
            }
            ExprKind::Invalid => {
                self.emit(Insn::LoadInvalid { dst });
            }
            ExprKind::Bool(v) => {
                self.emit(Insn::LoadBool { dst, v: *v });
            }
            ExprKind::Int(i) => {
                if *i as i32 as i64 == *i {
                    self.emit(Insn::LoadInt { dst, v: *i as i32 });
                } else {
                    let k = self.konst(Const::Int(*i));
                    self.emit(Insn::LoadConst { dst, k });
                }
            }
            ExprKind::Real(x) => {
                let k = self.konst(Const::Real(*x));
                self.emit(Insn::LoadConst { dst, k });
            }
            ExprKind::Str(s) => {
                let k = self.konst(Const::Str(s.clone()));
                self.emit(Insn::LoadConst { dst, k });
            }
            ExprKind::Unlimited => {
                self.emit(Insn::LoadInt { dst, v: -1 });
            }
            ExprKind::EnumLit(en, lit) => {
                let k = self.konst(Const::Enum(*en, *lit));
                self.emit(Insn::LoadConst { dst, k });
            }
            ExprKind::Type(t) => {
                let k = self.konst(Const::Type(*t));
                self.emit(Insn::LoadConst { dst, k });
            }
            ExprKind::Coll(kind, parts) => {
                let k = match kind {
                    CollKind::Set => coll_kind::SET,
                    CollKind::OrderedSet => coll_kind::ORDERED_SET,
                    CollKind::Bag => coll_kind::BAG,
                    CollKind::Sequence => coll_kind::SEQUENCE,
                    // an abstract `Collection{}` literal: classic OCL builds a Set
                    CollKind::Collection => coll_kind::SET,
                };
                self.literal_parts(dst, k, parts)?;
            }
            ExprKind::List(parts) => self.literal_parts(dst, coll_kind::LIST, parts)?,
            ExprKind::Dict(pairs) => {
                let acc = self.alloc()?;
                self.emit(Insn::NewColl {
                    dst: acc,
                    kind: coll_kind::DICT,
                });
                for (k, v) in pairs {
                    let m = self.mark();
                    let base = self.alloc_n(2)?;
                    self.expr(k, base)?;
                    self.expr(v, base + 1)?;
                    self.emit(Insn::LitPut {
                        dict: acc,
                        key: base,
                    });
                    self.release(m);
                }
                self.emit(Insn::Move { dst, src: acc });
            }
            ExprKind::Tuple(parts) => {
                let m = self.mark();
                let base = self.alloc_n(parts.len() as u32)?;
                for (i, (_, v)) in parts.iter().enumerate() {
                    self.expr(v, base + i as Reg)?;
                }
                let order = match self.types.get(e.ty) {
                    Type::Tuple(tparts) => parts
                        .iter()
                        .map(|(n, _)| tparts.iter().position(|(m, _)| m == n).map(|i| i as u16))
                        .collect::<Option<Vec<u16>>>(),
                    _ => None,
                };
                let Some(order) = order else {
                    return err("tuple literal without a matching tuple type");
                };
                let site = self.site(Site::Tuple {
                    ty: e.ty,
                    order: order.into(),
                })?;
                self.emit(Insn::NewTuple { dst, base, site });
                self.release(m);
            }
            ExprKind::Local(l) => {
                let src = self.local(*l);
                if src != dst {
                    self.emit(Insn::Move { dst, src });
                }
            }
            ExprKind::This(m) => {
                self.emit(Insn::LoadThis {
                    dst,
                    module: m.0 as u16,
                });
            }
            ExprKind::Global(g) => {
                self.emit(Insn::LoadGlobal { dst, g: g.0 });
                if self.is_ocl_collection(e.ty) {
                    self.emit(Insn::CopyColl { r: dst });
                }
            }
            ExprKind::ModuleProp(inst, g) => {
                let m = self.mark();
                let i = self.expr_reg(inst)?;
                let g16 = self.global16(*g)?;
                self.emit(Insn::LoadInstGlobal {
                    dst,
                    inst: i,
                    g: g16,
                });
                self.release(m);
            }
            ExprKind::ModelParam(p) => {
                self.emit(Insn::LoadModel {
                    dst,
                    module: p.module.0 as u16,
                    index: p.index as u16,
                });
            }
            ExprKind::Prop(src, f) => {
                let m = self.mark();
                let s = self.expr_reg(src)?;
                self.get_prop(dst, s, *f)?;
                // a single-valued feature of an OCL collection type (intermediate classes):
                // `coerceValue` copies it on every read
                if !self.ir.meta.feature(*f).is_many() && self.is_ocl_collection(e.ty) {
                    self.emit(Insn::CopyColl { r: dst });
                }
                self.release(m);
            }
            ExprKind::TupleProp(src, name) => {
                let m = self.mark();
                let s = self.expr_reg(src)?;
                let site = self.site(Site::Name(name.clone()))?;
                self.emit(Insn::GetTuple { dst, src: s, site });
                self.release(m);
            }
            ExprKind::IntermProp(src, p) => {
                let m = self.mark();
                let s = self.expr_reg(src)?;
                let site = self.site(Site::Interm(*p))?;
                self.emit(Insn::GetInterm { dst, obj: s, site });
                if self.is_ocl_collection(e.ty) {
                    self.emit(Insn::CopyColl { r: dst });
                }
                self.release(m);
            }
            ExprKind::Call(c) => self.call(e, c, dst)?,
            ExprKind::Loop(lp) => self.iterate(e, lp, dst)?,
            ExprKind::Let(l, init, body) => {
                let m = self.mark();
                let t = self.alloc()?;
                self.expr(init, t)?;
                if self.live_names.contains(&self.locals[l.0 as usize].name) {
                    self.emit(Insn::LoadInvalid { dst });
                    self.release(m);
                    return Ok(());
                }
                self.emit(Insn::Move {
                    dst: self.local(*l),
                    src: t,
                });
                self.release(m);
                self.expr(body, dst)?;
            }
            ExprKind::If(c, a, b) => {
                let m = self.mark();
                let t = self.expr_reg(c)?;
                let j_undef = self.jump(Insn::JumpIfNotBool { r: t, target: 0 });
                let j_else = self.jump(Insn::JumpIfFalse { r: t, target: 0 });
                self.release(m);
                self.expr(a, dst)?;
                let j_end = self.jump(Insn::Jump { target: 0 });
                self.patch(j_else);
                match b {
                    Some(b) => self.expr(b, dst)?,
                    None => {
                        self.emit(Insn::LoadNull { dst });
                    }
                }
                let j_end2 = self.jump(Insn::Jump { target: 0 });
                self.patch(j_undef);
                self.emit(Insn::LoadInvalid { dst });
                self.patch(j_end);
                self.patch(j_end2);
            }
            ExprKind::Block(stmts) => {
                for s in stmts {
                    self.stmt(s)?;
                }
                self.emit(Insn::LoadNull { dst });
            }
            ExprKind::VarInit {
                local,
                init,
                with_result,
            } => {
                let r = self.local(*local);
                let ty = self.locals[local.index()].ty;
                match init {
                    Some(init) => {
                        let m = self.mark();
                        let t = self.alloc()?;
                        self.expr(init, t)?;
                        if *with_result {
                            self.emit(Insn::Move { dst, src: t });
                        }
                        if self.needs_coercion(ty) {
                            let site = self.site(Site::Type(ty))?;
                            self.emit(Insn::Coerce { r: t, site });
                        }
                        self.emit(Insn::Move { dst: r, src: t });
                        self.release(m);
                    }
                    None => {
                        if ty == TypeId::OCL_INVALID {
                            // `createDefaultInitializationValue`: an `invalid` literal
                            self.emit(Insn::LoadInvalid { dst: r });
                        } else {
                            self.emit(Insn::LoadDefault { dst: r, ty: ty.0 });
                        }
                        if *with_result {
                            self.emit(Insn::Move { dst, src: r });
                        }
                    }
                }
                if !*with_result {
                    self.emit(Insn::LoadNull { dst });
                }
            }
            ExprKind::Assign(a) => self.assign(a, Some(dst))?,
            ExprKind::While(c, body) => {
                let head = self.pc();
                let m = self.mark();
                let t = self.expr_reg(c)?;
                let j_end = self.jump(Insn::JumpIfNotTrue { r: t, target: 0 });
                self.release(m);
                self.ctl.push(Ctl::Loop {
                    breaks: Vec::new(),
                    continue_target: head,
                });
                self.stmt(body)?;
                self.emit(Insn::Jump { target: head });
                let Some(Ctl::Loop { breaks, .. }) = self.ctl.pop() else {
                    unreachable!()
                };
                self.patch(j_end);
                for b in breaks {
                    self.patch(b);
                }
                self.emit(Insn::LoadNull { dst });
            }
            ExprKind::Compute(l, init, body) => {
                let r = self.local(*l);
                match init {
                    Some(init) => {
                        let m = self.mark();
                        let t = self.alloc()?;
                        self.expr(init, t)?;
                        self.emit(Insn::Move { dst: r, src: t });
                        self.release(m);
                    }
                    None => {
                        self.emit(Insn::LoadNull { dst: r });
                    }
                }
                self.stmt(body)?;
                self.emit(Insn::Move { dst, src: r });
            }
            ExprKind::Switch(alts, els) => {
                let mut ends = Vec::new();
                let mut undefs = Vec::new();
                for (c, body) in alts {
                    let m = self.mark();
                    let t = self.expr_reg(c)?;
                    undefs.push(self.jump(Insn::JumpIfUndef { r: t, target: 0 }));
                    let j_next = self.jump(Insn::JumpIfNotTrue { r: t, target: 0 });
                    self.release(m);
                    self.expr(body, dst)?;
                    ends.push(self.jump(Insn::Jump { target: 0 }));
                    self.patch(j_next);
                }
                match els {
                    Some(b) => self.expr(b, dst)?,
                    None => {
                        self.emit(Insn::LoadNull { dst });
                    }
                }
                ends.push(self.jump(Insn::Jump { target: 0 }));
                for u in undefs {
                    self.patch(u);
                }
                self.emit(Insn::LoadInvalid { dst });
                for j in ends {
                    self.patch(j);
                }
            }
            ExprKind::Return(v) => {
                let m = self.mark();
                let t = self.alloc()?;
                match v {
                    Some(v) => self.expr(v, t)?,
                    None => {
                        self.emit(Insn::LoadNull { dst: t });
                    }
                }
                let mut handled = false;
                for c in self.ctl.iter_mut().rev() {
                    match c {
                        Ctl::Barrier { exits } | Ctl::ReturnTo { exits } => {
                            let at = self.chunk.code.len();
                            exits.push(at);
                            self.chunk.code.push(Insn::Jump { target: 0 });
                            self.chunk.ips.push(self.ip);
                            handled = true;
                            break;
                        }
                        Ctl::Loop { .. } => {}
                    }
                }
                if !handled {
                    self.emit(Insn::Return { src: t });
                }
                self.release(m);
                self.emit(Insn::LoadNull { dst });
            }
            ExprKind::Break | ExprKind::Continue => {
                let is_break = matches!(e.kind, ExprKind::Break);
                let ip = self.ip;
                let at = self.chunk.code.len();
                let mut target = None;
                for c in self.ctl.iter_mut().rev() {
                    match c {
                        Ctl::Loop {
                            breaks,
                            continue_target,
                        } => {
                            if is_break {
                                breaks.push(at);
                                target = Some(0);
                            } else {
                                target = Some(*continue_target);
                            }
                            break;
                        }
                        Ctl::Barrier { exits } => {
                            exits.push(at);
                            target = Some(0);
                            break;
                        }
                        Ctl::ReturnTo { .. } => {}
                    }
                }
                if let Some(t) = target {
                    self.chunk.code.push(Insn::Jump { target: t });
                    self.chunk.ips.push(ip);
                }
                self.emit(Insn::LoadNull { dst });
            }
            ExprKind::Log(l) => {
                self.log(l, false, None)?;
                self.emit(Insn::LoadNull { dst });
            }
            ExprKind::Assert(a) => {
                let m = self.mark();
                let ip = self.ip;
                let t = self.expr_reg(&a.cond)?;
                let j_ok = self.jump(Insn::JumpIfTrue { r: t, target: 0 });
                self.release(m);
                let src = self.chunk.source;
                let location: Box<str> = match self.ir.sources.get(src.index()) {
                    Some(file) => format!("{}:{}", file.name, file.lines.line(e.span.start)).into(),
                    None => "UknownSource".into(),
                };
                let _ = ip;
                let (has_log, has_level, has_element) = match &a.log {
                    Some(l) => (true, l.level.is_some(), l.element.is_some()),
                    None => (false, false, false),
                };
                let site = self.site(Site::Assert {
                    severity: a.severity,
                    location,
                    has_log,
                    has_level,
                    has_element,
                    silent: false,
                })?;
                match &a.log {
                    Some(l) => self.log(l, true, Some(site))?,
                    None => {
                        let m = self.mark();
                        let base = self.alloc_n(3)?;
                        self.emit(Insn::AssertFail { base, site });
                        self.release(m);
                    }
                }
                self.patch(j_ok);
                self.emit(Insn::LoadNull { dst });
            }
            ExprKind::Raise(class, arg) => {
                let m = self.mark();
                let t = self.alloc()?;
                match arg {
                    Some(a) => self.expr(a, t)?,
                    None => {
                        self.emit(Insn::LoadNull { dst: t });
                    }
                }
                let site = self.site(Site::Raise {
                    class: *class,
                    has_arg: arg.is_some(),
                })?;
                self.emit(Insn::Raise { src: t, site });
                self.release(m);
                self.emit(Insn::LoadNull { dst });
            }
            ExprKind::Try(t) => {
                let start = self.pc();
                self.stmt(&t.body)?;
                let end = self.pc();
                let mut ends = vec![self.jump(Insn::Jump { target: 0 })];
                let mut catches = Vec::new();
                for c in &t.catches {
                    let target = self.pc();
                    catches.push(CatchInfo {
                        types: c.types.clone(),
                        var: c.var.map(|v| self.local(v)),
                        target,
                    });
                    self.ctl.push(Ctl::Barrier { exits: Vec::new() });
                    self.stmt(&c.body)?;
                    let Some(Ctl::Barrier { exits }) = self.ctl.pop() else {
                        unreachable!()
                    };
                    ends.extend(exits);
                    ends.push(self.jump(Insn::Jump { target: 0 }));
                }
                for j in ends {
                    self.patch(j);
                }
                self.chunk.handlers.push(Handler {
                    start,
                    end,
                    catches,
                });
                self.emit(Insn::LoadNull { dst });
            }
            ExprKind::Object(o) => {
                let var = self.local(o.var);
                if self.locals[o.var.index()].kind == ir::LocalKind::Object {
                    // the analyser's temporary for the object: Eclipse removes it after the
                    // expression, so every evaluation creates a new object
                    self.emit(Insn::LoadNull { dst: var });
                }
                let site = self.site(Site::Object {
                    class: o.class,
                    extent: o.extent,
                    fail: 0,
                })?;
                if self.types.is_collection_like(e.ty) {
                    // `object OrderedSet(T) {}`: `createInstance` of a collection type is a
                    // new empty collection (the IR's class is only a placeholder)
                    let j_create = self.jump(Insn::JumpIfNull { r: var, target: 0 });
                    let j_body = self.jump(Insn::Jump { target: 0 });
                    self.patch(j_create);
                    self.emit(Insn::LoadDefault { dst: var, ty: e.ty.0 });
                    self.patch(j_body);
                }
                self.emit(Insn::ObjectInit { var, site });
                for s in &o.body {
                    self.stmt(s)?;
                }
                self.emit(Insn::Move { dst, src: var });
                let j_end = self.jump(Insn::Jump { target: 0 });
                let fail = self.pc();
                if let Site::Object { fail: f, .. } = &mut self.chunk.sites[site as usize] {
                    *f = fail;
                }
                self.emit(Insn::LoadInvalid { dst });
                self.patch(j_end);
            }
            ExprKind::New(n) => {
                let m = self.mark();
                let base = self.alloc_n(n.args.len().max(1) as u32)?;
                for (i, a) in n.args.iter().enumerate() {
                    self.expr(a, base + i as Reg)?;
                }
                let site = self.site(Site::New {
                    ty: n.ty,
                    extent: n.extent,
                    constructor: n.constructor,
                    nargs: n.args.len() as u16,
                })?;
                self.emit(Insn::New { dst, base, site });
                self.ip = e.span.start;
                self.release(m);
            }
            ExprKind::NewTransformation(module, args) => {
                let m = self.mark();
                let base = self.alloc_n(args.len().max(1) as u32)?;
                for (i, a) in args.iter().enumerate() {
                    self.expr(a, base + i as Reg)?;
                }
                let site = self.site(Site::NewTransformation {
                    module: *module,
                    nargs: args.len() as u16,
                })?;
                self.emit(Insn::NewTransformation { dst, base, site });
                self.ip = e.span.start;
                self.release(m);
            }
            ExprKind::Resolve(r) => self.resolve(r, dst)?,
        }
        Ok(())
    }

    fn needs_coercion(&self, ty: TypeId) -> bool {
        matches!(
            self.types.get(ty),
            Type::List(_) | Type::Collection(CollKind::Sequence, _)
        )
    }

    fn get_prop(&mut self, dst: Reg, obj: Reg, f: FeatureId) -> CResult<()> {
        if f.0 <= u16::MAX as u32 {
            self.emit(Insn::GetProp {
                dst,
                obj,
                feature: f.0 as u16,
            });
        } else {
            let site = self.site(Site::Feature(f))?;
            self.emit(Insn::GetPropW { dst, obj, site });
        }
        Ok(())
    }

    fn literal_parts(&mut self, dst: Reg, kind: u8, parts: &[CollPart]) -> CResult<()> {
        let acc = self.alloc()?;
        self.emit(Insn::NewColl { dst: acc, kind });
        for p in parts {
            let m = self.mark();
            match p {
                CollPart::Item(e) => {
                    let t = self.expr_reg(e)?;
                    self.emit(Insn::LitAdd { coll: acc, val: t });
                }
                CollPart::Range(a, b) => {
                    let lo = self.alloc_n(2)?;
                    self.expr(a, lo)?;
                    self.expr(b, lo + 1)?;
                    self.emit(Insn::LitRange { coll: acc, lo });
                }
            }
            self.release(m);
        }
        self.emit(Insn::Move { dst, src: acc });
        Ok(())
    }

    /// `log` (and an assertion's log: the location prefix goes with the assertion site).
    fn log(&mut self, l: &ir::Log, assert: bool, assert_site: Option<u16>) -> CResult<()> {
        let m = self.mark();
        let mut j_skip = None;
        if let Some(w) = &l.when {
            let t = self.expr_reg(w)?;
            j_skip = Some(self.jump(Insn::JumpIfNotTrue { r: t, target: 0 }));
        }
        let base = self.alloc_n(3)?;
        if let Some(lv) = &l.level {
            self.expr(lv, base)?;
        }
        match &l.message {
            Some(msg) => self.expr(msg, base + 1)?,
            None => {
                self.emit(Insn::LoadNull { dst: base + 1 });
            }
        }
        if let Some(el) = &l.element {
            self.expr(el, base + 2)?;
        }
        if assert {
            let site = assert_site.expect("assert site");
            self.emit(Insn::AssertFail { base, site });
            if let Some(j) = j_skip.take() {
                // the log's condition is false: nothing is logged, the assertion still fails
                let j_end = self.jump(Insn::Jump { target: 0 });
                self.patch(j);
                let silent = match &self.chunk.sites[site as usize] {
                    Site::Assert {
                        severity, location, ..
                    } => Site::Assert {
                        severity: *severity,
                        location: location.clone(),
                        has_log: false,
                        has_level: false,
                        has_element: false,
                        silent: true,
                    },
                    _ => unreachable!(),
                };
                let s2 = self.site(silent)?;
                self.emit(Insn::AssertFail { base, site: s2 });
                self.patch(j_end);
            }
        } else {
            let site = self.site(Site::Log {
                has_level: l.level.is_some(),
                has_element: l.element.is_some(),
            })?;
            self.emit(Insn::Log { base, site });
        }
        if let Some(j) = j_skip {
            self.patch(j);
        }
        self.release(m);
        Ok(())
    }

    // ---- calls ------------------------------------------------------------------------------

    fn call(&mut self, e: &Expr, c: &ir::Call, dst: Reg) -> CResult<()> {
        match &c.callee {
            Callee::Std(op) => self.call_std(e, *op, c, dst),
            Callee::Op { op, virtual_ } => {
                let m = self.mark();
                let base = self.call_args(c)?;
                let op16 =
                    u16::try_from(op.0).map_err(|_| CompileError("too many operations".into()))?;
                if *virtual_ {
                    self.emit(Insn::CallVirtual {
                        dst,
                        base,
                        op: op16,
                    });
                } else {
                    self.emit(Insn::Call {
                        dst,
                        base,
                        op: op16,
                    });
                }
                self.ip = e.span.start;
                self.release(m);
                Ok(())
            }
            Callee::Mapping {
                op,
                strict,
                virtual_,
            } => {
                let m = self.mark();
                let base = self.call_args(c)?;
                let site = self.site(Site::Mapping {
                    op: *op,
                    strict: *strict,
                    virtual_: *virtual_,
                })?;
                self.emit(Insn::CallMapping { dst, base, site });
                self.ip = e.span.start;
                // out arguments: the callee's values are in the argument registers now
                let ir = self.ir;
                let params = &ir.operations[op.index()].params;
                for (i, (p, a)) in params.iter().zip(c.args.iter()).enumerate() {
                    if p.direction != Direction::Out {
                        continue;
                    }
                    let r = base + 1 + i as Reg;
                    match &a.kind {
                        ExprKind::Local(l) => {
                            let lr = self.local(*l);
                            self.emit(Insn::Move { dst: lr, src: r });
                        }
                        ExprKind::Prop(owner, f) => {
                            let mm = self.mark();
                            let o = self.expr_reg(owner)?;
                            let site = self.site(Site::SetProp {
                                feature: *f,
                                add: false,
                            })?;
                            self.ip = e.span.start;
                            self.emit(Insn::SetProp {
                                obj: o,
                                src: r,
                                site,
                            });
                            self.release(mm);
                        }
                        ExprKind::IntermProp(owner, p) => {
                            let mm = self.mark();
                            let o = self.expr_reg(owner)?;
                            let site = self.site(Site::SetInterm {
                                prop: *p,
                                add: false,
                            })?;
                            self.emit(Insn::SetInterm {
                                obj: o,
                                src: r,
                                site,
                            });
                            self.release(mm);
                        }
                        ExprKind::Global(g) => {
                            self.emit(Insn::StoreGlobal { src: r, g: g.0 });
                        }
                        ExprKind::ModuleProp(inst, g) => {
                            let mm = self.mark();
                            let i = self.expr_reg(inst)?;
                            let g16 = self.global16(*g)?;
                            self.emit(Insn::StoreInstGlobal {
                                src: r,
                                inst: i,
                                g: g16,
                            });
                            self.release(mm);
                        }
                        _ => {}
                    }
                }
                self.release(m);
                Ok(())
            }
            Callee::Meta(mo) => {
                let m = self.mark();
                let base = self.call_args(c)?;
                let site = self.site(Site::Meta(*mo))?;
                self.emit(Insn::CallMeta { dst, base, site });
                self.ip = e.span.start;
                self.release(m);
                Ok(())
            }
        }
    }

    /// Receiver (or null) and arguments into consecutive registers.
    fn call_args(&mut self, c: &ir::Call) -> CResult<Reg> {
        let base = self.alloc_n(1 + c.args.len() as u32)?;
        match &c.source {
            Some(s) => self.expr(s, base)?,
            None => {
                self.emit(Insn::LoadNull { dst: base });
            }
        }
        for (i, a) in c.args.iter().enumerate() {
            self.expr(a, base + 1 + i as Reg)?;
        }
        Ok(base)
    }

    fn call_std(&mut self, e: &Expr, op: StdOp, c: &ir::Call, dst: Reg) -> CResult<()> {
        match op {
            StdOp::Boolean_and | StdOp::Boolean_or | StdOp::Boolean_implies
                if c.source.is_some() && c.args.len() == 1 =>
            {
                return self.bool_op(op, c, dst);
            }
            _ => {}
        }
        if let Some(()) = self.fast_op(e, op, c, dst)? {
            return Ok(());
        }
        let m = self.mark();
        let base = self.call_args(c)?;
        let src_unlimited = c.source.as_ref().is_some_and(|s| self.is_unlimited(s));
        let arg_unlimited = c.args.first().is_some_and(|a| self.is_unlimited(a));
        let needs_type = matches!(op.name(), "sum" | "product" | "oclAsType");
        if src_unlimited || arg_unlimited || needs_type {
            let site = self.site(Site::Std {
                op,
                src_unlimited,
                arg_unlimited,
                src_ty: c.source.as_ref().map(|s| s.ty).unwrap_or(TypeId::OCL_VOID),
                res_ty: e.ty,
            })?;
            self.emit(Insn::CallStdX { dst, base, site });
        } else {
            self.emit(Insn::CallStd {
                dst,
                base,
                op: op as u16,
            });
        }
        self.ip = e.span.start;
        self.release(m);
        Ok(())
    }

    /// `x->op(args)` with `x`'s value moved into the call.
    fn call_std_taking(
        &mut self,
        e: &Expr,
        op: StdOp,
        c: &ir::Call,
        dst: Reg,
        x: LocalId,
    ) -> CResult<()> {
        let m = self.mark();
        let base = self.alloc_n(1 + c.args.len() as u32)?;
        self.emit(Insn::Take {
            dst: base,
            src: self.local(x),
        });
        for (i, a) in c.args.iter().enumerate() {
            self.expr(a, base + 1 + i as Reg)?;
        }
        let src_unlimited = c.source.as_ref().is_some_and(|s| self.is_unlimited(s));
        let arg_unlimited = c.args.first().is_some_and(|a| self.is_unlimited(a));
        let needs_type = matches!(op.name(), "sum" | "product" | "oclAsType");
        if src_unlimited || arg_unlimited || needs_type {
            let site = self.site(Site::Std {
                op,
                src_unlimited,
                arg_unlimited,
                src_ty: c.source.as_ref().map(|s| s.ty).unwrap_or(TypeId::OCL_VOID),
                res_ty: e.ty,
            })?;
            self.emit(Insn::CallStdX { dst, base, site });
        } else {
            self.emit(Insn::CallStd {
                dst,
                base,
                op: op as u16,
            });
        }
        self.ip = e.span.start;
        self.release(m);
        Ok(())
    }

    fn is_numeric(&self, t: TypeId) -> bool {
        t == TypeId::INTEGER || t == TypeId::REAL
    }

    /// Operators with instructions of their own: `+ - * < <= > >=` on Integer and Real
    /// operands, `=`, `<>`, `not`.
    fn fast_op(&mut self, e: &Expr, op: StdOp, c: &ir::Call, dst: Reg) -> CResult<Option<()>> {
        let d = op.desc();
        if d.section != qvto_types::Section::Ocl {
            return Ok(None);
        }
        let Some(src) = &c.source else {
            return Ok(None);
        };
        if self.is_unlimited(src) || c.args.first().is_some_and(|a| self.is_unlimited(a)) {
            return Ok(None);
        }
        if d.name == "not" && c.args.is_empty() {
            let m = self.mark();
            let s = self.expr_reg(src)?;
            self.emit(Insn::Not { dst, src: s });
            self.release(m);
            return Ok(Some(()));
        }
        if c.args.len() != 1 {
            return Ok(None);
        }
        let arg = &c.args[0];
        let numeric = self.is_numeric(src.ty) && self.is_numeric(arg.ty);
        let kind = match d.name {
            "+" if numeric => 0,
            "-" if numeric => 1,
            "*" if numeric => 2,
            "<" if numeric => 3,
            "<=" if numeric => 4,
            ">" if numeric => 5,
            ">=" if numeric => 6,
            "=" => 7,
            "<>" => 8,
            _ => return Ok(None),
        };
        let m = self.mark();
        // the left operand stays in its variable's register only if the right one cannot
        // change it
        let a = if simple(arg) {
            self.expr_reg(src)?
        } else {
            let t = self.alloc()?;
            self.expr(src, t)?;
            t
        };
        let b = self.expr_reg(arg)?;
        self.emit(match kind {
            0 => Insn::Add { dst, a, b },
            1 => Insn::Sub { dst, a, b },
            2 => Insn::Mul { dst, a, b },
            3 => Insn::Lt { dst, a, b },
            4 => Insn::Le { dst, a, b },
            5 => Insn::Gt { dst, a, b },
            6 => Insn::Ge { dst, a, b },
            7 if numeric => Insn::EqNum { dst, a, b },
            7 => Insn::Eq { dst, a, b },
            _ if numeric => Insn::NeNum { dst, a, b },
            _ => Insn::Ne { dst, a, b },
        });
        self.ip = e.span.start;
        self.release(m);
        Ok(Some(()))
    }

    /// Classic OCL's `and`, `or`, `implies` with lax null handling (`EvaluationVisitorImpl`):
    /// a deciding left operand short-circuits, a deciding right operand wins over an
    /// undefined left one, any other undefined operand gives invalid.
    fn bool_op(&mut self, op: StdOp, c: &ir::Call, dst: Reg) -> CResult<()> {
        let m = self.mark();
        let l = self.alloc()?;
        self.expr(c.source.as_ref().unwrap(), l)?;
        // left: and → false decides false; or → true decides true; implies → false decides true
        let j_left = match op {
            StdOp::Boolean_and => self.jump(Insn::JumpIfFalse { r: l, target: 0 }),
            StdOp::Boolean_or => self.jump(Insn::JumpIfTrue { r: l, target: 0 }),
            _ => self.jump(Insn::JumpIfFalse { r: l, target: 0 }),
        };
        let r = self.expr_reg(&c.args[0])?;
        let j_right = match op {
            StdOp::Boolean_and => self.jump(Insn::JumpIfFalse { r, target: 0 }),
            _ => self.jump(Insn::JumpIfTrue { r, target: 0 }),
        };
        let u1 = self.jump(Insn::JumpIfUndef { r: l, target: 0 });
        let u2 = self.jump(Insn::JumpIfUndef { r, target: 0 });
        match op {
            // both defined, neither deciding
            StdOp::Boolean_and => {
                self.emit(Insn::LoadBool { dst, v: true });
            }
            StdOp::Boolean_or => {
                self.emit(Insn::LoadBool { dst, v: false });
            }
            _ => {
                self.emit(Insn::Move { dst, src: r });
            }
        }
        let j_end1 = self.jump(Insn::Jump { target: 0 });
        self.patch(u1);
        self.patch(u2);
        self.emit(Insn::LoadInvalid { dst });
        let j_end2 = self.jump(Insn::Jump { target: 0 });
        // deciding values
        self.patch(j_left);
        self.patch(j_right);
        let v = !matches!(op, StdOp::Boolean_and);
        self.emit(Insn::LoadBool { dst, v });
        self.patch(j_end1);
        self.patch(j_end2);
        self.release(m);
        Ok(())
    }

    // ---- assignment -------------------------------------------------------------------------

    fn late_resolve_of<'e>(&self, v: &'e Expr) -> Option<(&'e ir::Resolve, Option<StdOp>)> {
        match &v.kind {
            ExprKind::Resolve(r) if r.late => Some((r, None)),
            ExprKind::Call(c) => {
                if let Callee::Std(op) = c.callee
                    && c.args.is_empty()
                    && matches!(op.name(), "asSet" | "asBag" | "asSequence" | "asOrderedSet")
                    && let Some(src) = &c.source
                    && let ExprKind::Resolve(r) = &src.kind
                    && r.late
                {
                    return Some((r, Some(op)));
                }
                None
            }
            _ => None,
        }
    }

    fn assign(&mut self, a: &ir::Assign, dst: Option<Reg>) -> CResult<()> {
        if let Some((r, conv)) = self.late_resolve_of(&a.value) {
            let m = self.mark();
            let d = match dst {
                Some(d) => d,
                None => self.alloc()?,
            };
            self.late_assign(a, r, conv, d)?;
            self.release(m);
            return Ok(());
        }
        let m = self.mark();
        let t = self.alloc()?;
        // `x := x->op(...)`: the variable's value moves into the call (its storage can be
        // reused in place), as `x` is overwritten right after
        let mut taken = false;
        if let LValue::Local(x) = &a.target
            && self.types.is_collection_like(a.target_ty)
            && let ExprKind::Call(c) = &a.value.kind
            && let Callee::Std(op) = c.callee
            && let Some(src) = &c.source
            && matches!(src.kind, ExprKind::Local(l) if l == *x)
            && !c.args.iter().any(|arg| mentions_local(arg, *x))
            && !matches!(
                op,
                StdOp::Boolean_and | StdOp::Boolean_or | StdOp::Boolean_implies
            )
        {
            self.enter(&a.value);
            self.call_std_taking(&a.value, op, c, t, *x)?;
            taken = true;
        }
        if !taken {
            self.expr(&a.value, t)?;
        }
        if let Some(d) = dst {
            self.emit(Insn::Move { dst: d, src: t });
        }
        match &a.target {
            LValue::Local(l) if !self.types.is_collection_like(a.target_ty) => {
                // a scalar variable: QVTo's assign is the value itself
                self.emit(Insn::Move {
                    dst: self.local(*l),
                    src: t,
                });
            }
            LValue::Local(l) => {
                let site = self.site(Site::Assign {
                    ty: a.target_ty,
                    add: a.add,
                    same_elem: self.same_elem(a.value.ty, a.target_ty),
                    property: false,
                })?;
                self.emit(Insn::Assign {
                    dst: self.local(*l),
                    src: t,
                    site,
                });
            }
            LValue::Global(g) => {
                let old = self.alloc()?;
                self.emit(Insn::LoadGlobal { dst: old, g: g.0 });
                let site = self.site(Site::Assign {
                    ty: a.target_ty,
                    add: a.add,
                    same_elem: self.same_elem(a.value.ty, a.target_ty),
                    property: true,
                })?;
                self.emit(Insn::Assign {
                    dst: old,
                    src: t,
                    site,
                });
                self.emit(Insn::StoreGlobal { src: old, g: g.0 });
            }
            LValue::ModuleProp(inst, g) => {
                let i = self.expr_reg(inst)?;
                let g16 = self.global16(*g)?;
                let old = self.alloc()?;
                self.emit(Insn::LoadInstGlobal {
                    dst: old,
                    inst: i,
                    g: g16,
                });
                let site = self.site(Site::Assign {
                    ty: a.target_ty,
                    add: a.add,
                    same_elem: self.same_elem(a.value.ty, a.target_ty),
                    property: true,
                })?;
                self.emit(Insn::Assign {
                    dst: old,
                    src: t,
                    site,
                });
                self.emit(Insn::StoreInstGlobal {
                    src: old,
                    inst: i,
                    g: g16,
                });
            }
            LValue::Prop(owner, f) => {
                let o = self.expr_reg(owner)?;
                let site = self.site(Site::SetProp {
                    feature: *f,
                    add: a.add,
                })?;
                self.emit(Insn::SetProp {
                    obj: o,
                    src: t,
                    site,
                });
            }
            LValue::IntermProp(owner, p) => {
                let o = self.expr_reg(owner)?;
                let site = self.site(Site::SetInterm {
                    prop: *p,
                    add: a.add,
                })?;
                self.emit(Insn::SetInterm {
                    obj: o,
                    src: t,
                    site,
                });
            }
            LValue::TupleProp(tuple, part) => {
                // `t.part += x` on a List/Dict part: the part is mutated in place
                let o = self.expr_reg(tuple)?;
                let cur = self.alloc()?;
                let ns = self.site(Site::Name(part.clone()))?;
                self.emit(Insn::GetTuple {
                    dst: cur,
                    src: o,
                    site: ns,
                });
                let site = self.site(Site::Assign {
                    ty: a.target_ty,
                    add: a.add,
                    same_elem: false,
                    property: false,
                })?;
                self.emit(Insn::Assign {
                    dst: cur,
                    src: t,
                    site,
                });
            }
        }
        self.release(m);
        Ok(())
    }

    /// The element types of a value and of the variable it is assigned to agree (so the
    /// assignment converts no element).
    fn same_elem(&self, value_ty: TypeId, target_ty: TypeId) -> bool {
        match (self.types.element(value_ty), self.types.element(target_ty)) {
            (Some(a), Some(b)) => {
                a == b
                    || !matches!(
                        self.types.get(b),
                        Type::Integer | Type::Real | Type::UnlimitedNatural | Type::DataType(_)
                    )
            }
            _ => false,
        }
    }

    /// `x.p := late resolve...`: the owner and the resolve's source are evaluated now, the
    /// assignment after the entry operation. On a variable the assignment is dropped.
    fn late_assign(
        &mut self,
        a: &ir::Assign,
        r: &ir::Resolve,
        conv: Option<StdOp>,
        dst: Reg,
    ) -> CResult<()> {
        let m = self.mark();
        let base = self.alloc_n(2)?;
        let (feature, interm) = match &a.target {
            LValue::Prop(owner, f) => {
                self.expr(owner, base)?;
                (Some(*f), None)
            }
            LValue::IntermProp(owner, p) => {
                self.expr(owner, base)?;
                (None, Some(*p))
            }
            _ => {
                self.emit(Insn::LoadNull { dst: base });
                (None, None)
            }
        };
        match &r.source {
            Some(s) => self.expr(s, base + 1)?,
            None => {
                self.emit(Insn::LoadNull { dst: base + 1 });
            }
        }
        if feature.is_some() || interm.is_some() {
            let cond = match (&r.cond, r.cond_var) {
                (Some(c), Some(v)) => Some((self.out_of_line_cond(c)?, self.local(v))),
                _ => None,
            };
            let site = self.site(Site::LateResolve {
                kind: r.kind,
                has_source: r.source.is_some(),
                source_is_coll: r
                    .source
                    .as_ref()
                    .is_some_and(|s| self.types.is_collection_like(s.ty)),
                mapping: r.mapping,
                target_ty: r.target_ty,
                cond,
                feature,
                interm,
                add: a.add,
                conversion: conv,
            })?;
            self.emit(Insn::LateResolve { base, site });
        }
        self.emit(Insn::LoadNull { dst });
        self.release(m);
        Ok(())
    }

    /// A condition compiled out of line in this chunk (late resolve conditions run after the
    /// entry operation, from the returned entry point on a fresh frame of the chunk's layout).
    fn out_of_line_cond(&mut self, c: &Expr) -> CResult<u32> {
        let j_over = self.jump(Insn::Jump { target: 0 });
        let entry = self.pc();
        let saved = std::mem::take(&mut self.ctl);
        let m = self.mark();
        let t = self.alloc()?;
        self.expr(c, t)?;
        self.emit(Insn::Return { src: t });
        self.release(m);
        self.ctl = saved;
        self.patch(j_over);
        Ok(entry)
    }

    // ---- resolve ----------------------------------------------------------------------------

    fn resolve(&mut self, r: &ir::Resolve, dst: Reg) -> CResult<()> {
        let m = self.mark();
        let base = self.alloc()?;
        match &r.source {
            Some(s) => self.expr(s, base)?,
            None => {
                self.emit(Insn::LoadNull { dst: base });
            }
        }
        if r.late {
            // not the right-hand side of a property assignment: null now; the deferred
            // evaluation's value is discarded (Eclipse)
            self.emit(Insn::LoadNull { dst });
            self.release(m);
            return Ok(());
        }
        let has_cond = r.cond.is_some() && r.cond_var.is_some();
        let kind = if has_cond {
            // all candidates; the condition filters below
            if r.kind.is_inverse() {
                ir::ResolveKind::InvResolve
            } else {
                ir::ResolveKind::Resolve
            }
        } else {
            r.kind
        };
        let site = self.site(Site::Resolve {
            kind,
            has_source: r.source.is_some(),
            source_is_coll: r
                .source
                .as_ref()
                .is_some_and(|s| self.types.is_collection_like(s.ty)),
            mapping: r.mapping,
            target_ty: r.target_ty,
        })?;
        if !has_cond {
            self.emit(Insn::Resolve { dst, base, site });
            self.release(m);
            return Ok(());
        }
        let cands = self.alloc()?;
        self.emit(Insn::Resolve {
            dst: cands,
            base,
            site,
        });
        let acc = self.alloc()?;
        let one = r.kind.is_one();
        if one {
            self.emit(Insn::LoadNull { dst: acc });
        } else {
            self.emit(Insn::NewColl {
                dst: acc,
                kind: coll_kind::SEQUENCE,
            });
        }
        let var = self.local(r.cond_var.unwrap());
        let slot = self.loop_slot();
        let lp = self.loop_info(slot, var, acc)?;
        self.emit(Insn::IterInit { slot, src: cands });
        let head = self.pc();
        let j_exit = self.jump(Insn::IterNext { lp, target: 0 });
        let mm = self.mark();
        let c = self.expr_reg(r.cond.as_ref().unwrap())?;
        self.emit(Insn::JumpIfNotTrue { r: c, target: head });
        self.release(mm);
        let mut exits = vec![j_exit];
        if one {
            self.emit(Insn::Move { dst: acc, src: var });
            exits.push(self.jump(Insn::Jump { target: 0 }));
        } else {
            self.emit(Insn::Push {
                coll: acc,
                val: var,
            });
            self.emit(Insn::Jump { target: head });
        }
        for j in exits {
            self.patch(j);
        }
        self.free_slot();
        self.emit(Insn::Move { dst, src: acc });
        self.release(m);
        Ok(())
    }

    // ---- iterators --------------------------------------------------------------------------

    /// The static collection kind of a type (List as Sequence for result-kind decisions is
    /// handled by the callers).
    fn static_kind(&self, t: TypeId) -> Option<CollKind> {
        self.types.coll_kind(t)
    }

    /// Emits nested loops over `src` binding `iters` (the first varies fastest) and calls
    /// `body`, whose `Jump`s to `next` continue; returns the patch list of the loop exits
    /// (exhausted). `exit_jumps` collects jumps the body makes to leave all loops.
    fn loops(
        &mut self,
        src: Reg,
        iters: &[LocalId],
        acc: Reg,
        body: &mut dyn FnMut(&mut Self, u32) -> CResult<()>,
    ) -> CResult<Vec<usize>> {
        // outermost: the last iterator
        let n = iters.len().max(1);
        let mut heads = Vec::with_capacity(n);
        let mut exits = Vec::with_capacity(n);
        let mut slots = Vec::with_capacity(n);
        for k in (0..n).rev() {
            let var = iters.get(k).map(|l| self.local(*l)).unwrap_or(acc);
            let slot = self.loop_slot();
            slots.push(slot);
            let lp = self.loop_info(slot, var, acc)?;
            self.emit(Insn::IterInit { slot, src });
            heads.push(self.pc());
            exits.push(self.jump(Insn::IterNext { lp, target: 0 }));
        }
        // innermost head is the continue target
        let inner_head = *heads.last().unwrap();
        self.ctl.push(Ctl::Loop {
            breaks: Vec::new(),
            continue_target: inner_head,
        });
        body(self, inner_head)?;
        self.emit(Insn::Jump { target: inner_head });
        let Some(Ctl::Loop { breaks, .. }) = self.ctl.pop() else {
            unreachable!()
        };
        // an exhausted inner loop continues the next outer one
        let mut out = breaks;
        for i in (1..exits.len()).rev() {
            let outer_head = heads[i - 1];
            self.patch_to(exits[i], outer_head);
        }
        out.push(exits[0]);
        for _ in &slots {
            self.free_slot();
        }
        Ok(out)
    }

    fn iterate(&mut self, e: &Expr, lp: &ir::Loop, dst: Reg) -> CResult<()> {
        let m = self.mark();
        // `iterate`: the result variable with its initial value first, then the source
        // (`visitIterateExp`); the iterators are added after the source
        let acc_name = lp.acc.as_ref().map(|(l, _)| self.locals[l.0 as usize].name.clone());
        if let Some((accl, init)) = &lp.acc
            && lp.kind == LoopKind::Iterate
        {
            let ar = self.local(*accl);
            let mm = self.mark();
            let t = self.alloc()?;
            self.expr(init, t)?;
            self.emit(Insn::Move { dst: ar, src: t });
            self.release(mm);
            if acc_name.as_ref().is_some_and(|n| self.live_names.contains(n)) {
                self.emit(Insn::LoadInvalid { dst });
                self.release(m);
                return Ok(());
            }
        }
        let pushed = lp.kind == LoopKind::Iterate && acc_name.is_some();
        if pushed {
            self.live_names.push(acc_name.clone().unwrap());
        }
        let src = self.expr_reg(&lp.source);
        if pushed {
            self.live_names.pop();
        }
        let src = src?;
        if lp
            .iterators
            .iter()
            .any(|l| self.live_names.contains(&self.locals[l.0 as usize].name))
        {
            self.emit(Insn::LoadInvalid { dst });
            self.release(m);
            return Ok(());
        }
        let acc = self.alloc()?;
        let src_kind = self.static_kind(lp.source.ty);
        let src_set_or_bag = matches!(src_kind, Some(CollKind::Set) | Some(CollKind::Bag));
        let iter0 = lp.iterators.first().map(|l| self.local(*l));
        let mut invalids: Vec<usize> = Vec::new();
        let mut ends: Vec<usize> = Vec::new();
        use LoopKind as K;
        match lp.kind {
            K::ForEach | K::ForOne => {
                ends.push(self.jump(Insn::JumpIfUndef { r: src, target: 0 }));
                let one = lp.kind == K::ForOne;
                let exits = self.loops(src, &lp.iterators, acc, &mut |b, next| {
                    if let Some(c) = &lp.cond {
                        let mm = b.mark();
                        let t = b.expr_reg(c)?;
                        b.emit(Insn::JumpIfNotTrue { r: t, target: next });
                        b.release(mm);
                    }
                    if let Some(body) = &lp.body {
                        b.stmt(body)?;
                    }
                    if one {
                        let j = b.jump(Insn::Jump { target: 0 });
                        b.break_patch(j);
                    }
                    Ok(())
                })?;
                ends.extend(exits);
                for j in ends.drain(..) {
                    self.patch(j);
                }
                self.emit(Insn::LoadNull { dst });
                self.release(m);
                return Ok(());
            }
            _ => {}
        }
        // every other iterator: an undefined source gives invalid
        invalids.push(self.jump(Insn::JumpIfUndef { r: src, target: 0 }));
        match lp.kind {
            K::ForAll | K::Exists => {
                let forall = lp.kind == K::ForAll;
                self.emit(Insn::LoadBool {
                    dst: acc,
                    v: forall,
                });
                let mut inv = Vec::new();
                let exits = self.loops(src, &lp.iterators, acc, &mut |b, next| {
                    let mm = b.mark();
                    let t = b.expr_reg(lp.body.as_ref().unwrap())?;
                    inv.push(b.jump(Insn::JumpIfNotBool { r: t, target: 0 }));
                    if forall {
                        b.emit(Insn::JumpIfTrue { r: t, target: next });
                    } else {
                        b.emit(Insn::JumpIfFalse { r: t, target: next });
                    }
                    b.release(mm);
                    b.emit(Insn::LoadBool {
                        dst: acc,
                        v: !forall,
                    });
                    let j = b.jump(Insn::Jump { target: 0 });
                    b.break_patch(j);
                    Ok(())
                })?;
                invalids.extend(inv);
                ends.extend(exits);
            }
            K::One => {
                self.emit(Insn::LoadBool { dst: acc, v: false });
                let found = self.alloc()?;
                self.emit(Insn::LoadBool {
                    dst: found,
                    v: false,
                });
                let mut inv = Vec::new();
                let exits = self.loops(src, &lp.iterators, acc, &mut |b, next| {
                    let mm = b.mark();
                    let t = b.expr_reg(lp.body.as_ref().unwrap())?;
                    inv.push(b.jump(Insn::JumpIfNotBool { r: t, target: 0 }));
                    b.emit(Insn::JumpIfFalse { r: t, target: next });
                    b.release(mm);
                    let j_second = b.jump(Insn::JumpIfTrue {
                        r: found,
                        target: 0,
                    });
                    b.emit(Insn::LoadBool {
                        dst: found,
                        v: true,
                    });
                    b.emit(Insn::LoadBool { dst: acc, v: true });
                    b.emit(Insn::Jump { target: next });
                    b.patch(j_second);
                    b.emit(Insn::LoadBool { dst: acc, v: false });
                    let j = b.jump(Insn::Jump { target: 0 });
                    b.break_patch(j);
                    Ok(())
                })?;
                invalids.extend(inv);
                ends.extend(exits);
            }
            K::Any => {
                self.emit(Insn::LoadNull { dst: acc });
                let mut inv = Vec::new();
                let exits = self.loops(src, &lp.iterators, acc, &mut |b, next| {
                    let mm = b.mark();
                    let t = b.expr_reg(lp.body.as_ref().unwrap())?;
                    inv.push(b.jump(Insn::JumpIfNotBool { r: t, target: 0 }));
                    b.emit(Insn::JumpIfFalse { r: t, target: next });
                    b.release(mm);
                    b.emit(Insn::Move {
                        dst: acc,
                        src: iter0.unwrap(),
                    });
                    let j = b.jump(Insn::Jump { target: 0 });
                    b.break_patch(j);
                    Ok(())
                })?;
                invalids.extend(inv);
                ends.extend(exits);
            }
            K::IsUnique => {
                self.emit(Insn::LoadBool { dst: acc, v: true });
                let set = self.alloc()?;
                self.emit(Insn::NewColl {
                    dst: set,
                    kind: coll_kind::SET,
                });
                let exits = self.loops(src, &lp.iterators, acc, &mut |b, next| {
                    let mm = b.mark();
                    let t = b.expr_reg(lp.body.as_ref().unwrap())?;
                    let f = b.alloc()?;
                    b.emit(Insn::PushNew {
                        dst: f,
                        coll: set,
                        val: t,
                    });
                    b.emit(Insn::JumpIfTrue { r: f, target: next });
                    b.release(mm);
                    b.emit(Insn::LoadBool { dst: acc, v: false });
                    let j = b.jump(Insn::Jump { target: 0 });
                    b.break_patch(j);
                    Ok(())
                })?;
                ends.extend(exits);
            }
            K::Select | K::Reject => {
                let kind = match src_kind {
                    Some(CollKind::Set) => coll_kind::SET,
                    Some(CollKind::Bag) => coll_kind::BAG,
                    Some(CollKind::Sequence) => coll_kind::SEQUENCE,
                    _ => coll_kind::ORDERED_SET,
                };
                self.emit(Insn::NewColl { dst: acc, kind });
                let select = lp.kind == K::Select;
                let mut inv = Vec::new();
                let exits = self.loops(src, &lp.iterators, acc, &mut |b, next| {
                    let mm = b.mark();
                    let t = b.expr_reg(lp.body.as_ref().unwrap())?;
                    inv.push(b.jump(Insn::JumpIfNotBool { r: t, target: 0 }));
                    if select {
                        b.emit(Insn::JumpIfFalse { r: t, target: next });
                    } else {
                        b.emit(Insn::JumpIfTrue { r: t, target: next });
                    }
                    b.release(mm);
                    b.emit(Insn::Push {
                        coll: acc,
                        val: iter0.unwrap(),
                    });
                    Ok(())
                })?;
                invalids.extend(inv);
                ends.extend(exits);
            }
            K::Collect | K::CollectNested => {
                let kind = if src_set_or_bag {
                    coll_kind::BAG
                } else {
                    coll_kind::SEQUENCE
                };
                self.emit(Insn::NewColl { dst: acc, kind });
                let flat = lp.kind == K::Collect;
                let mut inv = Vec::new();
                let exits = self.loops(src, &lp.iterators, acc, &mut |b, _next| {
                    let mm = b.mark();
                    let t = b.expr_reg(lp.body.as_ref().unwrap())?;
                    inv.push(b.jump(Insn::JumpIfInvalid { r: t, target: 0 }));
                    if flat {
                        b.emit(Insn::PushFlat { coll: acc, val: t });
                    } else {
                        b.emit(Insn::Push { coll: acc, val: t });
                    }
                    b.release(mm);
                    Ok(())
                })?;
                invalids.extend(inv);
                ends.extend(exits);
            }
            K::SortedBy => {
                let elems = self.alloc_n(2)?;
                self.emit(Insn::NewColl {
                    dst: elems,
                    kind: coll_kind::SEQUENCE,
                });
                self.emit(Insn::NewColl {
                    dst: elems + 1,
                    kind: coll_kind::SEQUENCE,
                });
                let mut inv = Vec::new();
                let body_e = lp.body.as_ref().unwrap();
                let unlimited = body_e.ty == TypeId::UNLIMITED_NATURAL;
                let exits = self.loops(src, &lp.iterators, acc, &mut |b, _next| {
                    let mm = b.mark();
                    let t = b.expr_reg(body_e)?;
                    inv.push(b.jump(Insn::JumpIfUndef { r: t, target: 0 }));
                    b.emit(Insn::Push {
                        coll: elems,
                        val: iter0.unwrap(),
                    });
                    b.emit(Insn::Push {
                        coll: elems + 1,
                        val: t,
                    });
                    b.release(mm);
                    Ok(())
                })?;
                invalids.extend(inv);
                for j in exits {
                    self.patch(j);
                }
                if has_side_effects(body_e) {
                    // classic OCL evaluates the body for every element a second time and keys
                    // by the second values (`evaluateSortedByIterator`); observable only through
                    // side effects, so fast mode evaluates once
                    let skip = self.jump(Insn::JumpIfFastMode { target: 0 });
                    let idx = self.alloc()?;
                    self.emit(Insn::LoadInt { dst: idx, v: 0 });
                    let mut stop = Vec::new();
                    let exits = self.loops(src, &lp.iterators, acc, &mut |b, _next| {
                        let mm = b.mark();
                        let t = b.expr_reg(body_e)?;
                        // an undefined value ends the pass, the first values stay
                        stop.push(b.jump(Insn::JumpIfUndef { r: t, target: 0 }));
                        b.emit(Insn::SetNext {
                            coll: elems + 1,
                            idx,
                            val: t,
                        });
                        b.release(mm);
                        Ok(())
                    })?;
                    for j in exits.into_iter().chain(stop) {
                        self.patch(j);
                    }
                    self.patch(skip);
                }
                let site = self.site(Site::Sort {
                    ordered_set: matches!(
                        src_kind,
                        Some(CollKind::Set) | Some(CollKind::OrderedSet)
                    ),
                    unlimited,
                })?;
                self.emit(Insn::SortBy {
                    dst: acc,
                    elems,
                    site,
                });
            }
            K::Closure => {
                let ordered = matches!(self.static_kind(e.ty), Some(CollKind::OrderedSet));
                self.emit(Insn::NewColl {
                    dst: acc,
                    kind: if ordered {
                        coll_kind::ORDERED_SET
                    } else {
                        coll_kind::SET
                    },
                });
                let var = iter0.unwrap();
                let slot = self.loop_slot();
                let lpi = self.loop_info(slot, var, acc)?;
                self.emit(Insn::ClosureInit { slot, src });
                let head = self.pc();
                let j_exit = self.jump(Insn::ClosureNext { lp: lpi, target: 0 });
                self.ctl.push(Ctl::Loop {
                    breaks: Vec::new(),
                    continue_target: head,
                });
                let mm = self.mark();
                let t = self.expr_reg(lp.body.as_ref().unwrap())?;
                invalids.push(self.jump(Insn::JumpIfInvalid { r: t, target: 0 }));
                self.emit(Insn::ClosurePush { slot, val: t });
                self.release(mm);
                self.emit(Insn::Jump { target: head });
                let Some(Ctl::Loop { breaks, .. }) = self.ctl.pop() else {
                    unreachable!()
                };
                self.free_slot();
                ends.push(j_exit);
                ends.extend(breaks);
            }
            K::Iterate => {
                let (accl, _) = lp.acc.as_ref().unwrap();
                let ar = self.local(*accl);
                let exits = self.loops(src, &lp.iterators, acc, &mut |b, _next| {
                    let mm = b.mark();
                    let t = b.alloc()?;
                    b.expr(lp.body.as_ref().unwrap(), t)?;
                    b.emit(Insn::Move { dst: ar, src: t });
                    b.release(mm);
                    Ok(())
                })?;
                for j in exits {
                    self.patch(j);
                }
                self.emit(Insn::Move { dst: acc, src: ar });
            }
            K::XCollect | K::CollectOne => {
                let one = lp.kind == K::CollectOne;
                if one {
                    self.emit(Insn::LoadNull { dst: acc });
                } else {
                    let kind = if src_set_or_bag {
                        coll_kind::BAG
                    } else {
                        coll_kind::SEQUENCE
                    };
                    self.emit(Insn::NewColl { dst: acc, kind });
                }
                let mut inv = Vec::new();
                let target = lp.target.map(|t| self.local(t));
                let exits = self.loops(src, &lp.iterators, acc, &mut |b, next| {
                    let mm = b.mark();
                    let t = match &lp.body {
                        Some(body) => b.expr_reg(body)?,
                        None => iter0.unwrap(),
                    };
                    if let Some(tr) = target {
                        b.emit(Insn::Move { dst: tr, src: t });
                    }
                    b.emit(Insn::JumpIfNull { r: t, target: next });
                    inv.push(b.jump(Insn::JumpIfInvalid { r: t, target: 0 }));
                    if one {
                        b.emit(Insn::Move { dst: acc, src: t });
                        let j = b.jump(Insn::Jump { target: 0 });
                        b.break_patch(j);
                    } else {
                        b.emit(Insn::Push { coll: acc, val: t });
                    }
                    b.release(mm);
                    Ok(())
                })?;
                invalids.extend(inv);
                ends.extend(exits);
            }
            K::XSelect | K::SelectOne => {
                let one = lp.kind == K::SelectOne;
                if one {
                    self.emit(Insn::LoadNull { dst: acc });
                } else {
                    self.emit(Insn::NewCollLike { dst: acc, src });
                }
                let mut inv = Vec::new();
                let target = lp.target.map(|t| self.local(t));
                let filter = match lp.filter_type {
                    Some(t) => Some(self.site(Site::Type(t))?),
                    None => None,
                };
                let exits = self.loops(src, &lp.iterators, acc, &mut |b, next| {
                    let mm = b.mark();
                    let cur = iter0.unwrap();
                    if let Some(body) = &lp.body {
                        let t = b.expr_reg(body)?;
                        if let Some(tr) = target {
                            b.emit(Insn::Move { dst: tr, src: t });
                        }
                    } else if let Some(tr) = target {
                        b.emit(Insn::Move { dst: tr, src: cur });
                    }
                    b.emit(Insn::JumpIfNull {
                        r: cur,
                        target: next,
                    });
                    let c = match (filter, &lp.cond) {
                        (Some(site), _) => {
                            let c = b.alloc()?;
                            b.emit(Insn::KindTest {
                                dst: c,
                                src: cur,
                                site,
                            });
                            c
                        }
                        (None, Some(cond)) => b.expr_reg(cond)?,
                        (None, None) => {
                            let c = b.alloc()?;
                            b.emit(Insn::LoadBool { dst: c, v: true });
                            c
                        }
                    };
                    inv.push(b.jump(Insn::JumpIfNotBool { r: c, target: 0 }));
                    b.emit(Insn::JumpIfFalse { r: c, target: next });
                    if one {
                        b.emit(Insn::Move { dst: acc, src: cur });
                        let j = b.jump(Insn::Jump { target: 0 });
                        b.break_patch(j);
                    } else {
                        b.emit(Insn::Push {
                            coll: acc,
                            val: cur,
                        });
                    }
                    b.release(mm);
                    Ok(())
                })?;
                invalids.extend(inv);
                ends.extend(exits);
            }
            K::CollectSelect | K::CollectSelectOne => {
                let one = lp.kind == K::CollectSelectOne;
                if one {
                    self.emit(Insn::LoadNull { dst: acc });
                } else {
                    let kind = if src_set_or_bag {
                        coll_kind::BAG
                    } else {
                        coll_kind::SEQUENCE
                    };
                    self.emit(Insn::NewColl { dst: acc, kind });
                }
                let mut inv = Vec::new();
                let target = lp.target.map(|t| self.local(t));
                let filter = match lp.filter_type {
                    Some(t) => Some(self.site(Site::Type(t))?),
                    None => None,
                };
                let exits = self.loops(src, &lp.iterators, acc, &mut |b, next| {
                    let mm = b.mark();
                    let t = match &lp.body {
                        Some(body) => b.expr_reg(body)?,
                        None => iter0.unwrap(),
                    };
                    if let Some(tr) = target {
                        b.emit(Insn::Move { dst: tr, src: t });
                    }
                    b.emit(Insn::JumpIfNull { r: t, target: next });
                    let c = match (filter, &lp.cond) {
                        (Some(site), _) => {
                            let c = b.alloc()?;
                            b.emit(Insn::KindTest {
                                dst: c,
                                src: t,
                                site,
                            });
                            c
                        }
                        (None, Some(cond)) => b.expr_reg(cond)?,
                        (None, None) => {
                            let c = b.alloc()?;
                            b.emit(Insn::LoadBool { dst: c, v: true });
                            c
                        }
                    };
                    inv.push(b.jump(Insn::JumpIfNotBool { r: c, target: 0 }));
                    b.emit(Insn::JumpIfFalse { r: c, target: next });
                    inv.push(b.jump(Insn::JumpIfInvalid { r: t, target: 0 }));
                    if one {
                        b.emit(Insn::Move { dst: acc, src: t });
                        let j = b.jump(Insn::Jump { target: 0 });
                        b.break_patch(j);
                    } else {
                        b.emit(Insn::Push { coll: acc, val: t });
                    }
                    b.release(mm);
                    Ok(())
                })?;
                invalids.extend(inv);
                ends.extend(exits);
            }
            K::ForEach | K::ForOne => unreachable!(),
        }
        let j_done = self.jump(Insn::Jump { target: 0 });
        for j in invalids {
            self.patch(j);
        }
        self.emit(Insn::LoadInvalid { dst: acc });
        self.patch(j_done);
        for j in ends {
            self.patch(j);
        }
        self.emit(Insn::Move { dst, src: acc });
        self.release(m);
        Ok(())
    }

    /// Registers a jump that leaves the innermost loop (like `break`).
    fn break_patch(&mut self, at: usize) {
        for c in self.ctl.iter_mut().rev() {
            if let Ctl::Loop { breaks, .. } = c {
                breaks.push(at);
                return;
            }
        }
        panic!("break_patch outside a loop");
    }
}

/// Whether evaluating the expression can have an effect beyond its value: calls of imperative
/// or metamodel operations, imperative expressions, mutating library operations.
fn has_side_effects(e: &Expr) -> bool {
    use qvto_sema::ir::Callee;
    let own = match &e.kind {
        ExprKind::Call(c) => match &c.callee {
            Callee::Std(op) => {
                let d = op.desc();
                matches!(
                    d.owner,
                    qvto_types::StdOwner::List
                        | qvto_types::StdOwner::Dict
                        | qvto_types::StdOwner::Model
                        | qvto_types::StdOwner::Transformation
                        | qvto_types::StdOwner::Stdlib
                ) && !matches!(
                    d.name,
                    "at" | "first" | "last" | "size" | "includes" | "excludes" | "get" | "hasKey"
                        | "keys" | "values" | "isEmpty" | "notEmpty" | "indexOf" | "count"
                ) || (
                    // QVTo's string counters (`StringOperations.StringCounter`, session state)
                    d.owner == qvto_types::StdOwner::String
                        && (d.name == "addSuffixNumber" || d.name.ends_with("StrCounter"))
                )
            }
            _ => true,
        },
        ExprKind::Assign(_)
        | ExprKind::Log(_)
        | ExprKind::Assert(_)
        | ExprKind::Object(_)
        | ExprKind::New(_)
        | ExprKind::NewTransformation(..)
        | ExprKind::Raise(..)
        | ExprKind::Resolve(_) => true,
        _ => false,
    };
    if own {
        return true;
    }
    let mut any = false;
    qvto_sema::irck::for_each_child(e, |c| any = any || has_side_effects(c));
    any
}

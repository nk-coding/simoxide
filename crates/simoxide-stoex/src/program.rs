//! Compilation of a prepared StoEx into a flat, index-based program, and its evaluator.
//!
//! * Variables are resolved to caller-defined slots at compile time.
//! * Static types are baked into the operator nodes (they only depend on the source tree).
//! * Parentheses disappear; deterministic subtrees (no variables, no random draws) are folded
//!   to constants, or to a stored error that is raised at the same point of the evaluation
//!   order as in the reference (e.g. `1/0`).
//! * Evaluation is a recursive walk over the node array; it does not allocate unless a string
//!   value is produced from a non-constant source or a function has more than four arguments.

use crate::ast::*;
use crate::error::{EvalError, EvalErrorKind as K, PrepareError};
use crate::funcs::{self, Func};
use crate::ops;
use crate::probfn::ProbFn;
use crate::types::{self, SType};
use crate::value::Value;
use crate::{Prepared, VariableMode};
use simoxide_random::UniformSource;
use std::sync::Arc;

/// Variable lookup during evaluation (the stack frame).
pub trait Env {
    /// The value bound to `slot`, or `Ok(None)` if the frame has no such id. An implementation
    /// that stores lazily evaluated characterisations (SimuLizar's `EvaluationProxy` for
    /// `INNER`) evaluates them here, drawing from `rng`.
    fn lookup<R: UniformSource + ?Sized>(
        &self,
        slot: u32,
        rng: &mut R,
    ) -> Result<Option<Value>, EvalError>;
}

/// An environment without variables.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoVars;

impl Env for NoVars {
    fn lookup<R: UniformSource + ?Sized>(
        &self,
        _: u32,
        _: &mut R,
    ) -> Result<Option<Value>, EvalError> {
        Ok(None)
    }
}

/// Information about a variable reference of a program.
#[derive(Debug, Clone, PartialEq)]
pub struct VarInfo {
    pub var: VarRef,
    /// Stack frame id (`a.b.VALUE`).
    pub id: Arc<str>,
    /// Slot assigned by the resolver.
    pub slot: u32,
}

#[derive(Debug, Clone)]
enum Node {
    Const(Value),
    Fail(EvalError),
    /// Variable: index into `vars`.
    Var(u32),
    /// Probability function literal: index into `probfns`.
    Sample(u32),
    Neg(u32),
    Not(u32),
    Term(TermOp, u32, u32, SType, SType),
    Product(ProdOp, u32, u32, SType, SType),
    Power(u32, u32, SType, SType),
    Compare(CmpOp, u32, u32, SType, SType),
    BoolOp(BoolOp, u32, u32),
    IfElse(u32, u32, u32),
    /// Function: arguments are `args[start..start + len]`.
    Call(Func, u32, u32),
}

/// A compiled StoEx. Cheap to share between threads (`Send + Sync`).
#[derive(Debug, Clone)]
pub struct Program {
    nodes: Vec<Node>,
    args: Vec<u32>,
    root: u32,
    probfns: Vec<ProbFn>,
    vars: Vec<VarInfo>,
}

struct NoRng;
impl UniformSource for NoRng {
    fn next_uniform(&mut self) -> f64 {
        unreachable!("constant folding never draws")
    }
}

struct Lowering<'a, F: FnMut(&VarRef) -> u32> {
    p: Program,
    resolve: F,
    probfns: std::slice::Iter<'a, ProbFn>,
}

impl<F: FnMut(&VarRef) -> u32> Lowering<'_, F> {
    fn push(&mut self, n: Node) -> u32 {
        self.p.nodes.push(n);
        (self.p.nodes.len() - 1) as u32
    }

    fn is_const(&self, i: u32) -> bool {
        matches!(self.p.nodes[i as usize], Node::Const(_) | Node::Fail(_))
    }

    /// Folds node `i` if all its inputs are constant.
    fn fold(&mut self, i: u32, inputs: &[u32]) -> u32 {
        if inputs.iter().all(|&c| self.is_const(c)) {
            let r = self
                .p
                .eval_node(i, &NoVars, &mut NoRng, VariableMode::ExceptionOnNotFound);
            self.p.nodes[i as usize] = match r {
                Ok(v) => Node::Const(v),
                Err(e) => Node::Fail(e),
            };
        }
        i
    }

    fn st(e: &Expr) -> SType {
        types::eval_type(e).expect("types checked in prepare")
    }

    fn lower(&mut self, e: &Expr) -> u32 {
        match &e.kind {
            ExprKind::Int(v) => self.push(Node::Const(Value::Int(*v))),
            ExprKind::Double(v) => self.push(Node::Const(Value::Double(*v))),
            ExprKind::Bool(v) => self.push(Node::Const(Value::Bool(*v))),
            ExprKind::Str(v) => self.push(Node::Const(Value::Str(Arc::from(v.as_str())))),
            ExprKind::Paren(i) => self.lower(i),
            ExprKind::Var(v) => {
                let slot = (self.resolve)(v);
                self.p.vars.push(VarInfo {
                    var: v.clone(),
                    id: Arc::from(v.id()),
                    slot,
                });
                let idx = (self.p.vars.len() - 1) as u32;
                self.push(Node::Var(idx))
            }
            ExprKind::ProbFn(_) => {
                let pf = self
                    .probfns
                    .next()
                    .expect("one prepared function per literal");
                self.p.probfns.push(pf.clone());
                let idx = (self.p.probfns.len() - 1) as u32;
                self.push(Node::Sample(idx))
            }
            ExprKind::Neg(i) => {
                let c = self.lower(i);
                let n = self.push(Node::Neg(c));
                self.fold(n, &[c])
            }
            ExprKind::Not(i) => {
                let c = self.lower(i);
                let n = self.push(Node::Not(c));
                self.fold(n, &[c])
            }
            ExprKind::Term(op, l, r) => {
                let (lt, rt) = (Self::st(l), Self::st(r));
                let (a, b) = (self.lower(l), self.lower(r));
                let n = self.push(Node::Term(*op, a, b, lt, rt));
                self.fold(n, &[a, b])
            }
            ExprKind::Product(op, l, r) => {
                let (lt, rt) = (Self::st(l), Self::st(r));
                let (a, b) = (self.lower(l), self.lower(r));
                let n = self.push(Node::Product(*op, a, b, lt, rt));
                self.fold(n, &[a, b])
            }
            ExprKind::Power(l, r) => {
                let (lt, rt) = (Self::st(l), Self::st(r));
                let (a, b) = (self.lower(l), self.lower(r));
                let n = self.push(Node::Power(a, b, lt, rt));
                self.fold(n, &[a, b])
            }
            ExprKind::Compare(op, l, r) => {
                let (lt, rt) = (Self::st(l), Self::st(r));
                let (a, b) = (self.lower(l), self.lower(r));
                let n = self.push(Node::Compare(*op, a, b, lt, rt));
                self.fold(n, &[a, b])
            }
            ExprKind::BoolOp(op, l, r) => {
                let (a, b) = (self.lower(l), self.lower(r));
                let n = self.push(Node::BoolOp(*op, a, b));
                self.fold(n, &[a, b])
            }
            ExprKind::IfElse(c, t, f) => {
                let ci = self.lower(c);
                let ti = self.lower(t);
                let fi = self.lower(f);
                if let Node::Const(Value::Bool(b)) = self.p.nodes[ci as usize] {
                    return if b { ti } else { fi };
                }
                if self.is_const(ci) {
                    // non-boolean constant or stored error: fails when evaluated
                    let n = self.push(Node::IfElse(ci, ti, fi));
                    let r =
                        self.p
                            .eval_node(n, &NoVars, &mut NoRng, VariableMode::ExceptionOnNotFound);
                    if let Err(e) = r {
                        self.p.nodes[n as usize] = Node::Fail(e);
                    }
                    return n;
                }
                self.push(Node::IfElse(ci, ti, fi))
            }
            ExprKind::Func(name, args) => {
                let f = Func::from_name(name).expect("function names checked in prepare");
                let lowered: Vec<u32> = args.iter().map(|a| self.lower(a)).collect();
                let start = self.p.args.len() as u32;
                self.p.args.extend_from_slice(&lowered);
                let n = self.push(Node::Call(f, start, lowered.len() as u32));
                if f.is_random() {
                    n
                } else {
                    self.fold(n, &lowered)
                }
            }
        }
    }
}

impl Program {
    /// Compiles a prepared expression. `resolve` maps each variable reference (in source order)
    /// to a slot that the [`Env`] understands.
    pub fn compile(prepared: &Prepared, resolve: impl FnMut(&VarRef) -> u32) -> Program {
        let mut l = Lowering {
            p: Program {
                nodes: Vec::new(),
                args: Vec::new(),
                root: 0,
                probfns: Vec::new(),
                vars: Vec::new(),
            },
            resolve,
            probfns: prepared.probfns.iter(),
        };
        let root = l.lower(&prepared.expr);
        let mut p = l.p;
        p.root = root;
        p.compact();
        p
    }

    /// Parses, prepares and compiles.
    pub fn from_str(
        src: &str,
        resolve: impl FnMut(&VarRef) -> u32,
    ) -> Result<Program, PrepareError> {
        Ok(Program::compile(&crate::prepare(src)?, resolve))
    }

    /// Drops nodes that are unreachable after folding and renumbers.
    fn compact(&mut self) {
        let mut map = vec![u32::MAX; self.nodes.len()];
        let mut nodes = Vec::new();
        let mut args = Vec::new();
        fn visit(
            p: &Program,
            i: u32,
            map: &mut [u32],
            nodes: &mut Vec<Node>,
            args: &mut Vec<u32>,
        ) -> u32 {
            if map[i as usize] != u32::MAX {
                return map[i as usize];
            }
            let n = match p.nodes[i as usize].clone() {
                Node::Neg(c) => Node::Neg(visit(p, c, map, nodes, args)),
                Node::Not(c) => Node::Not(visit(p, c, map, nodes, args)),
                Node::Term(o, a, b, x, y) => {
                    let a = visit(p, a, map, nodes, args);
                    Node::Term(o, a, visit(p, b, map, nodes, args), x, y)
                }
                Node::Product(o, a, b, x, y) => {
                    let a = visit(p, a, map, nodes, args);
                    Node::Product(o, a, visit(p, b, map, nodes, args), x, y)
                }
                Node::Power(a, b, x, y) => {
                    let a = visit(p, a, map, nodes, args);
                    Node::Power(a, visit(p, b, map, nodes, args), x, y)
                }
                Node::Compare(o, a, b, x, y) => {
                    let a = visit(p, a, map, nodes, args);
                    Node::Compare(o, a, visit(p, b, map, nodes, args), x, y)
                }
                Node::BoolOp(o, a, b) => {
                    let a = visit(p, a, map, nodes, args);
                    Node::BoolOp(o, a, visit(p, b, map, nodes, args))
                }
                Node::IfElse(c, a, b) => {
                    let c = visit(p, c, map, nodes, args);
                    let a = visit(p, a, map, nodes, args);
                    Node::IfElse(c, a, visit(p, b, map, nodes, args))
                }
                Node::Call(f, s, len) => {
                    let mapped: Vec<u32> = (s..s + len)
                        .map(|k| visit(p, p.args[k as usize], map, nodes, args))
                        .collect();
                    let start = args.len() as u32;
                    args.extend(mapped);
                    Node::Call(f, start, len)
                }
                other => other,
            };
            nodes.push(n);
            let idx = (nodes.len() - 1) as u32;
            map[i as usize] = idx;
            idx
        }
        let root = visit(self, self.root, &mut map, &mut nodes, &mut args);
        self.nodes = nodes;
        self.args = args;
        self.root = root;
    }

    /// The variable references of the program, in source order.
    pub fn vars(&self) -> &[VarInfo] {
        &self.vars
    }

    /// The constant result, if the whole expression folded to a value.
    pub fn constant(&self) -> Option<&Value> {
        match &self.nodes[self.root as usize] {
            Node::Const(v) => Some(v),
            _ => None,
        }
    }

    /// Number of IR nodes (after folding).
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Evaluates with `VariableMode::ExceptionOnNotFound` (what the simulator uses).
    #[inline]
    pub fn eval<E: Env, R: UniformSource + ?Sized>(
        &self,
        env: &E,
        rng: &mut R,
    ) -> Result<Value, EvalError> {
        self.eval_node(self.root, env, rng, VariableMode::ExceptionOnNotFound)
    }

    /// Evaluates with an explicit variable mode.
    #[inline]
    pub fn eval_mode<E: Env, R: UniformSource + ?Sized>(
        &self,
        env: &E,
        rng: &mut R,
        mode: VariableMode,
    ) -> Result<Value, EvalError> {
        self.eval_node(self.root, env, rng, mode)
    }

    /// `evaluateStatic(spec, Double.class, frame)`.
    #[inline]
    pub fn eval_f64<E: Env, R: UniformSource + ?Sized>(
        &self,
        env: &E,
        rng: &mut R,
    ) -> Result<f64, EvalError> {
        if let Node::Const(Value::Double(d)) = self.nodes[self.root as usize] {
            return Ok(d);
        }
        match self.eval(env, rng)? {
            Value::Double(d) => Ok(d),
            Value::Int(i) => Ok(f64::from(i)),
            v => v.to_f64(),
        }
    }

    /// `evaluateStatic(spec, Integer.class, frame)`.
    #[inline]
    pub fn eval_i32<E: Env, R: UniformSource + ?Sized>(
        &self,
        env: &E,
        rng: &mut R,
    ) -> Result<i32, EvalError> {
        self.eval(env, rng)?.to_i32()
    }

    /// `evaluateStatic(spec, Boolean.class, frame)`.
    #[inline]
    pub fn eval_bool<E: Env, R: UniformSource + ?Sized>(
        &self,
        env: &E,
        rng: &mut R,
    ) -> Result<bool, EvalError> {
        self.eval(env, rng)?.to_bool()
    }

    fn missing(&self, var: &VarInfo, mode: VariableMode) -> Result<Value, EvalError> {
        match mode {
            VariableMode::ExceptionOnNotFound => Err(EvalError::new(
                K::Runtime,
                format!(
                    "Architecture specification incomplete. Stackframe is missing id {}",
                    var.id
                ),
            )),
            VariableMode::ReturnNullOnNotFound => Ok(Value::Null),
            VariableMode::ReturnDefaultOnNotFound => match var.var.characterisation {
                Characterisation::ByteSize | Characterisation::NumberOfElements => {
                    Ok(Value::Int(0))
                }
                _ => Err(EvalError::new(
                    K::Runtime,
                    format!(
                        "Architecture specification incomplete. Stackframe is missing id {}",
                        var.id
                    ),
                )),
            },
        }
    }

    fn eval_node<E: Env, R: UniformSource + ?Sized>(
        &self,
        i: u32,
        env: &E,
        rng: &mut R,
        mode: VariableMode,
    ) -> Result<Value, EvalError> {
        match &self.nodes[i as usize] {
            Node::Const(v) => Ok(v.clone()),
            Node::Fail(e) => Err(e.clone()),
            Node::Var(k) => {
                let info = &self.vars[*k as usize];
                match env.lookup(info.slot, rng)? {
                    Some(v) => Ok(v),
                    None => self.missing(info, mode),
                }
            }
            Node::Sample(k) => self.probfns[*k as usize].sample(rng),
            Node::Neg(c) => ops::neg(&self.eval_node(*c, env, rng, mode)?),
            Node::Not(c) => ops::not(&self.eval_node(*c, env, rng, mode)?),
            Node::Term(op, a, b, lt, rt) => {
                let l = self.eval_node(*a, env, rng, mode)?;
                let r = self.eval_node(*b, env, rng, mode)?;
                ops::term(*op, *lt, *rt, &l, &r)
            }
            Node::Product(op, a, b, lt, rt) => {
                let l = self.eval_node(*a, env, rng, mode)?;
                let r = self.eval_node(*b, env, rng, mode)?;
                ops::product(*op, *lt, *rt, &l, &r)
            }
            Node::Power(a, b, lt, rt) => {
                let l = self.eval_node(*a, env, rng, mode)?;
                let r = self.eval_node(*b, env, rng, mode)?;
                ops::power(*lt, *rt, &l, &r)
            }
            Node::Compare(op, a, b, lt, rt) => {
                let l = self.eval_node(*a, env, rng, mode)?;
                let r = self.eval_node(*b, env, rng, mode)?;
                ops::compare(*op, *lt, *rt, &l, &r)
            }
            Node::BoolOp(op, a, b) => {
                let l = ops::cast_bool(&self.eval_node(*a, env, rng, mode)?)?;
                let r = ops::cast_bool(&self.eval_node(*b, env, rng, mode)?)?;
                Ok(Value::Bool(ops::bool_op(*op, l, r)))
            }
            Node::IfElse(c, a, b) => {
                if ops::cast_bool(&self.eval_node(*c, env, rng, mode)?)? {
                    self.eval_node(*a, env, rng, mode)
                } else {
                    self.eval_node(*b, env, rng, mode)
                }
            }
            Node::Call(f, start, len) => {
                let (start, len) = (*start as usize, *len as usize);
                if len <= 4 {
                    let mut buf: [Value; 4] = [Value::Null, Value::Null, Value::Null, Value::Null];
                    for (k, slot) in buf.iter_mut().enumerate().take(len) {
                        *slot = self.eval_node(self.args[start + k], env, rng, mode)?;
                    }
                    funcs::call(*f, &buf[..len], rng)
                } else {
                    let mut v = Vec::with_capacity(len);
                    for k in 0..len {
                        v.push(self.eval_node(self.args[start + k], env, rng, mode)?);
                    }
                    funcs::call(*f, &v, rng)
                }
            }
        }
    }
}

// Programs are shared between simulation threads.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Program>();
};

//! Stochastic Expressions (StoEx) as SimuLizar 5.2.2 parses and evaluates them.
//!
//! Semantics, with references to the Java sources, are in `docs/spec/stoex.md`.
//!
//! # Pipeline
//!
//! 1. [`parse`]: text -> [`Expr`] (the Xtext grammar of `PCMStoex`, including its precedence
//!    oddities; errors carry line/column).
//! 2. [`prepare`]: what `new StoExCacheEntry(spec)` does: parse, type inference (rejects
//!    unknown function names), adjustment/validation/sorting of every PMF/PDF literal.
//! 3. [`Program::compile`]: resolves variables to caller slots, bakes in the static types that
//!    drive int/double arithmetic, folds deterministic subtrees.
//! 4. [`Program::eval`]: evaluates against an [`Env`] (the stack frame), drawing uniforms from a
//!    [`simoxide_random::UniformSource`]. Every PMF/PDF literal and every distribution function
//!    draws exactly one uniform per evaluation, in left-to-right evaluation order; boolean
//!    operators evaluate both sides, `?:` only the chosen branch.
//!
//! ```
//! use simoxide_stoex::{Program, SimpleEnv, Value};
//! let mut env = SimpleEnv::new();
//! env.set("n.VALUE", Value::Int(3));
//! let prog = Program::from_str("n.VALUE * 2 + IntPMF[(1;0.5)(2;0.5)]", |v| env.slot(&v.id())).unwrap();
//! let mut rng = simoxide_random::Replay::new(&[0.7]);
//! assert_eq!(prog.eval(&env, &mut rng).unwrap(), Value::Int(8));
//! ```

pub mod ast;
pub mod error;
pub mod funcs;
pub mod interp;
pub mod jmath;
pub mod lexer;
pub mod ops;
pub mod parser;
pub mod print;
pub mod probfn;
pub mod program;
pub mod types;
pub mod value;

pub use ast::{Characterisation, Expr, ExprKind, VarRef};
pub use error::{EvalError, EvalErrorKind, ParseError, PrepareError, Span};
pub use parser::parse;
pub use probfn::ProbFn;
pub use program::{Env, NoVars, Program, VarInfo};
pub use types::{SType, TypeEnum};
pub use value::{Converted, Expected, Value};

use simoxide_random::UniformSource;
use std::collections::HashMap;
use std::sync::Arc;

/// What a variable lookup does when the stack frame has no value (`VariableMode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VariableMode {
    /// Error (the default of `StackContext` and of every call the simulator makes).
    #[default]
    ExceptionOnNotFound,
    /// 0 for `BYTESIZE`/`NUMBER_OF_ELEMENTS`, an error otherwise.
    ReturnDefaultOnNotFound,
    /// Java `null`.
    ReturnNullOnNotFound,
}

/// A parsed, type-checked expression with its prepared probability functions
/// (the `StoExCacheEntry` of the reference).
#[derive(Debug, Clone)]
pub struct Prepared {
    pub expr: Expr,
    /// Prepared PMF/PDF literals in pre-order of the tree.
    pub probfns: Vec<ProbFn>,
}

impl Prepared {
    /// Static type of the whole expression as the evaluator sees it.
    pub fn root_type(&self) -> SType {
        types::eval_type(&self.expr).ok().flatten()
    }
}

/// Parses and prepares a StoEx like `new StoExCacheEntry(spec)`.
pub fn prepare(src: &str) -> Result<Prepared, PrepareError> {
    let expr = parse(src)?;
    types::check(&expr)?;
    let mut probfns = Vec::new();
    let mut err = None;
    expr.walk(&mut |e| {
        if err.is_some() {
            return;
        }
        if let ExprKind::ProbFn(lit) = &e.kind {
            match ProbFn::prepare(lit) {
                Ok(p) => probfns.push(p),
                Err(message) => {
                    err = Some(PrepareError::InvalidProbFunction {
                        message,
                        span: e.span,
                    })
                }
            }
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    Ok(Prepared { expr, probfns })
}

/// A binding in a [`SimpleEnv`].
#[derive(Debug, Clone)]
pub enum Binding {
    Value(Value),
    /// SimuLizar's `EvaluationProxy`: the expression is evaluated on every access, against the
    /// captured frame, in `ExceptionOnNotFound` mode.
    Proxy(Arc<Program>, Arc<SimpleEnv>),
}

/// A simple stack frame: ids are assigned slots on first use.
#[derive(Debug, Clone, Default)]
pub struct SimpleEnv {
    ids: HashMap<String, u32>,
    values: Vec<Option<Binding>>,
}

impl SimpleEnv {
    pub fn new() -> Self {
        Self::default()
    }

    /// Slot of an id (allocated if new). Use as the compile-time resolver.
    pub fn slot(&mut self, id: &str) -> u32 {
        if let Some(s) = self.ids.get(id) {
            return *s;
        }
        let s = self.values.len() as u32;
        self.ids.insert(id.to_string(), s);
        self.values.push(None);
        s
    }

    /// Binds a value.
    pub fn set(&mut self, id: &str, v: Value) {
        let s = self.slot(id) as usize;
        self.values[s] = Some(Binding::Value(v));
    }

    /// Binds a lazily evaluated expression.
    pub fn set_proxy(&mut self, id: &str, program: Arc<Program>, frame: Arc<SimpleEnv>) {
        let s = self.slot(id) as usize;
        self.values[s] = Some(Binding::Proxy(program, frame));
    }
}

impl Env for SimpleEnv {
    #[inline]
    fn lookup<R: UniformSource + ?Sized>(
        &self,
        slot: u32,
        rng: &mut R,
    ) -> Result<Option<Value>, EvalError> {
        match self.values.get(slot as usize) {
            Some(Some(Binding::Value(v))) => Ok(Some(v.clone())),
            Some(Some(Binding::Proxy(p, frame))) => eval_proxy(p, frame, rng),
            _ => Ok(None),
        }
    }
}

#[inline(never)]
fn eval_proxy<R: UniformSource + ?Sized>(
    p: &Program,
    frame: &SimpleEnv,
    rng: &mut R,
) -> Result<Option<Value>, EvalError> {
    p.eval(frame, rng).map(Some)
}

//! Reference tree-walking evaluator: a direct transcription of `PCMStoExEvaluationVisitor`
//! over the AST (no folding, types computed per node). Used to cross-check [`crate::Program`].

use crate::ast::*;
use crate::error::{EvalError, EvalErrorKind as K};
use crate::funcs::{self, Func};
use crate::ops;
use crate::program::Env;
use crate::types;
use crate::value::Value;
use crate::{Prepared, VariableMode};
use simoxide_random::UniformSource;
use std::collections::HashMap;

/// Evaluates `prepared` directly on its syntax tree.
pub fn eval<E: Env, R: UniformSource + ?Sized>(
    prepared: &Prepared,
    resolve: &mut dyn FnMut(&VarRef) -> u32,
    env: &E,
    rng: &mut R,
    mode: VariableMode,
) -> Result<Value, EvalError> {
    let mut pf_index = HashMap::new();
    let mut k = 0usize;
    prepared.expr.walk(&mut |e| {
        if let ExprKind::ProbFn(_) = e.kind {
            pf_index.insert(e as *const Expr, k);
            k += 1;
        }
    });
    let mut it = Interp {
        prepared,
        pf_index,
        resolve,
        mode,
    };
    it.eval(&prepared.expr, env, rng)
}

struct Interp<'a> {
    prepared: &'a Prepared,
    pf_index: HashMap<*const Expr, usize>,
    resolve: &'a mut dyn FnMut(&VarRef) -> u32,
    mode: VariableMode,
}

impl Interp<'_> {
    fn ty(e: &Expr) -> types::SType {
        types::eval_type(e).expect("prepared")
    }

    fn eval<E: Env, R: UniformSource + ?Sized>(
        &mut self,
        e: &Expr,
        env: &E,
        rng: &mut R,
    ) -> Result<Value, EvalError> {
        match &e.kind {
            ExprKind::Var(v) => {
                let slot = (self.resolve)(v);
                match env.lookup(slot, rng)? {
                    Some(val) => Ok(val),
                    None => match self.mode {
                        VariableMode::ReturnNullOnNotFound => Ok(Value::Null),
                        VariableMode::ReturnDefaultOnNotFound
                            if Self::ty(e) == Some(types::TypeEnum::Int) =>
                        {
                            Ok(Value::Int(0))
                        }
                        _ => Err(EvalError::new(
                            K::Runtime,
                            format!(
                                "Architecture specification incomplete. Stackframe is missing id {}",
                                v.id()
                            ),
                        )),
                    },
                }
            }
            ExprKind::Compare(op, l, r) => {
                let (lt, rt) = (Self::ty(l), Self::ty(r));
                let a = self.eval(l, env, rng)?;
                let b = self.eval(r, env, rng)?;
                ops::compare(*op, lt, rt, &a, &b)
            }
            ExprKind::Double(v) => Ok(Value::Double(*v)),
            ExprKind::Int(v) => Ok(Value::Int(*v)),
            ExprKind::Str(v) => Ok(Value::from(v.as_str())),
            ExprKind::Paren(i) => self.eval(i, env, rng),
            ExprKind::ProbFn(_) => {
                let k = self.pf_index[&(e as *const Expr)];
                self.prepared.probfns[k].sample(rng)
            }
            ExprKind::Product(op, l, r) => {
                let (lt, rt) = (Self::ty(l), Self::ty(r));
                let a = self.eval(l, env, rng)?;
                let b = self.eval(r, env, rng)?;
                ops::product(*op, lt, rt, &a, &b)
            }
            ExprKind::Term(op, l, r) => {
                let (lt, rt) = (Self::ty(l), Self::ty(r));
                let a = self.eval(l, env, rng)?;
                let b = self.eval(r, env, rng)?;
                ops::term(*op, lt, rt, &a, &b)
            }
            ExprKind::BoolOp(op, l, r) => {
                let a = ops::cast_bool(&self.eval(l, env, rng)?)?;
                let b = ops::cast_bool(&self.eval(r, env, rng)?)?;
                Ok(Value::Bool(ops::bool_op(*op, a, b)))
            }
            ExprKind::Neg(i) => ops::neg(&self.eval(i, env, rng)?),
            ExprKind::Bool(b) => Ok(Value::Bool(*b)),
            ExprKind::Not(i) => ops::not(&self.eval(i, env, rng)?),
            ExprKind::Power(b, x) => {
                let (lt, rt) = (Self::ty(b), Self::ty(x));
                let a = self.eval(b, env, rng)?;
                let c = self.eval(x, env, rng)?;
                ops::power(lt, rt, &a, &c)
            }
            ExprKind::Func(name, args) => {
                let mut vals = Vec::with_capacity(args.len());
                for a in args {
                    vals.push(self.eval(a, env, rng)?);
                }
                let f = Func::from_name(name).expect("prepared");
                funcs::call(f, &vals, rng)
            }
            ExprKind::IfElse(c, a, b) => {
                if ops::cast_bool(&self.eval(c, env, rng)?)? {
                    self.eval(a, env, rng)
                } else {
                    self.eval(b, env, rng)
                }
            }
        }
    }
}

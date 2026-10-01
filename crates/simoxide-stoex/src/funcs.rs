//! The StoEx function library (`FunctionLib` of simucomframework.variables 5.2.2).
//!
//! All arguments are evaluated (left to right, with their random draws) before the function is
//! looked up and `checkParameters` runs. Distribution functions then construct the distribution
//! and draw exactly one uniform (via the sampling hooks of `simoxide_random::UniformSource`,
//! which default to `simoxide_random::dist`; a fast-mode source samples differently).

// The parameter checks are written as `!(x <= 0.0)` etc. on purpose: that is how the Java code
// rejects arguments, and it lets NaN through exactly like the reference.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::error::{EvalError, EvalErrorKind as K};
use crate::jmath;
use crate::ops::err;
use crate::value::Value;
use simoxide_random::{DistError, UniformSource};

/// Functions known to the type inference (`Binom` is known there but not to `FunctionLib`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Func {
    Norm,
    Exp,
    Pois,
    UniDouble,
    UniInt,
    Trunc,
    Round,
    Ceil,
    Log,
    Sqrt,
    Lognorm,
    LognormMoments,
    Gamma,
    GammaMoments,
    Min,
    Max,
    MinDeviation,
    MaxDeviation,
    /// Accepted by type inference, unknown at evaluation (`FunctionUnknownException`).
    Binom,
}

impl Func {
    /// Resolves a function name (case sensitive). `None` for names the type inference rejects.
    pub fn from_name(name: &str) -> Option<Func> {
        Some(match name {
            "Norm" => Func::Norm,
            "Exp" => Func::Exp,
            "Pois" => Func::Pois,
            "UniDouble" => Func::UniDouble,
            "UniInt" => Func::UniInt,
            "Trunc" => Func::Trunc,
            "Round" => Func::Round,
            "Ceil" => Func::Ceil,
            "Log" => Func::Log,
            "Sqrt" => Func::Sqrt,
            "Lognorm" => Func::Lognorm,
            "LognormMoments" => Func::LognormMoments,
            "Gamma" => Func::Gamma,
            "GammaMoments" => Func::GammaMoments,
            "Min" => Func::Min,
            "Max" => Func::Max,
            "MinDeviation" => Func::MinDeviation,
            "MaxDeviation" => Func::MaxDeviation,
            "Binom" => Func::Binom,
            _ => return None,
        })
    }

    /// The StoEx name.
    pub fn name(self) -> &'static str {
        match self {
            Func::Norm => "Norm",
            Func::Exp => "Exp",
            Func::Pois => "Pois",
            Func::UniDouble => "UniDouble",
            Func::UniInt => "UniInt",
            Func::Trunc => "Trunc",
            Func::Round => "Round",
            Func::Ceil => "Ceil",
            Func::Log => "Log",
            Func::Sqrt => "Sqrt",
            Func::Lognorm => "Lognorm",
            Func::LognormMoments => "LognormMoments",
            Func::Gamma => "Gamma",
            Func::GammaMoments => "GammaMoments",
            Func::Min => "Min",
            Func::Max => "Max",
            Func::MinDeviation => "MinDeviation",
            Func::MaxDeviation => "MaxDeviation",
            Func::Binom => "Binom",
        }
    }

    /// True if the function draws a uniform when evaluated.
    pub fn is_random(self) -> bool {
        matches!(
            self,
            Func::Norm
                | Func::Exp
                | Func::Pois
                | Func::UniDouble
                | Func::UniInt
                | Func::Lognorm
                | Func::LognormMoments
                | Func::Gamma
                | Func::GammaMoments
        )
    }
}

/// `NumberConverter.toDouble`.
fn to_double(v: &Value) -> Result<f64, EvalError> {
    match v {
        Value::Double(d) => Ok(*d),
        Value::Int(i) => Ok(f64::from(*i)),
        _ => Err(err(K::Runtime, format!("Can't case {v} to double!"))),
    }
}

#[cold]
fn not_accepted(f: Func) -> EvalError {
    err(
        K::FunctionParametersNotAccepted,
        format!(
            "Parameters passed to function {} do not match function definition!",
            f.name()
        ),
    )
}

fn dist_err(f: Func, e: DistError) -> EvalError {
    match e {
        DistError::ParametersNotAccepted { .. } => not_accepted(f),
        other => err(K::Distribution, other.to_string()),
    }
}

fn is_number(v: &Value) -> bool {
    matches!(v, Value::Int(_) | Value::Double(_))
}

fn same_class(a: &Value, b: &Value) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b) && !matches!(b, Value::Null)
}

/// `FunctionLib.evaluate(id, params)`: `checkParameters`, then `evaluate`.
pub fn call<R: UniformSource + ?Sized>(
    f: Func,
    p: &[Value],
    rng: &mut R,
) -> Result<Value, EvalError> {
    let n = p.len();
    macro_rules! check {
        ($cond:expr) => {
            if !($cond) {
                return Err(not_accepted(f));
            }
        };
    }
    let d = |r: Result<f64, DistError>| r.map(Value::Double).map_err(|e| dist_err(f, e));
    let i = |r: Result<i32, DistError>| r.map(Value::Int).map_err(|e| dist_err(f, e));
    match f {
        Func::Binom => Err(err(
            K::FunctionUnknown,
            "Function Binom is unknown! Evaluation aborted",
        )),
        Func::Norm => {
            check!(n == 2);
            let (m, s) = (to_double(&p[0])?, to_double(&p[1])?);
            d(rng.sample_norm(m, s))
        }
        Func::Exp => {
            check!(n == 1 && !(to_double(&p[0])? <= 0.0));
            d(rng.sample_exp(to_double(&p[0])?))
        }
        Func::Pois => {
            check!(n == 1 && !(to_double(&p[0])? < 0.0));
            i(rng.sample_pois(to_double(&p[0])?))
        }
        Func::UniDouble => {
            check!(n == 2 && !(to_double(&p[0])? > to_double(&p[1])?));
            d(rng.sample_unidouble(to_double(&p[0])?, to_double(&p[1])?))
        }
        Func::UniInt => {
            check!(n == 2);
            match (&p[0], &p[1]) {
                (Value::Int(a), Value::Int(b)) => i(rng.sample_uniint(*a, *b)),
                _ => Err(not_accepted(f)),
            }
        }
        Func::Trunc | Func::Round | Func::Ceil => {
            check!(n == 1);
            match &p[0] {
                Value::Int(v) => Ok(Value::Int(*v)),
                Value::Double(x) => Ok(Value::Int(match f {
                    // (int) Math.round(Math.floor(x)): long narrowed by wrapping
                    Func::Trunc => jmath::java_round(x.floor()) as i32,
                    Func::Round => jmath::java_round(*x) as i32,
                    // (int) Math.ceil(x): saturating
                    _ => x.ceil() as i32,
                })),
                _ => Err(not_accepted(f)),
            }
        }
        Func::Log => {
            check!(n == 2);
            let base = match &p[0] {
                Value::Double(b) if !(*b <= 0.0 || *b == 1.0) => *b,
                Value::Int(b) if !(*b <= 0 || *b == 1) => f64::from(*b),
                _ => return Err(not_accepted(f)),
            };
            let value = match &p[1] {
                Value::Double(v) if !(*v <= 0.0) => *v,
                Value::Int(v) if *v > 0 => f64::from(*v),
                _ => return Err(not_accepted(f)),
            };
            Ok(Value::Double(jmath::log(value) / jmath::log(base)))
        }
        Func::Sqrt => {
            check!(n == 1);
            match &p[0] {
                Value::Int(v) => Ok(Value::Double(f64::from(*v).sqrt())),
                Value::Double(v) => Ok(Value::Double(v.sqrt())),
                _ => Err(not_accepted(f)),
            }
        }
        Func::Lognorm => {
            check!(n == 2);
            check!(!(to_double(&p[1])? <= 0.0));
            to_double(&p[0])?;
            d(rng.sample_lognorm(to_double(&p[0])?, to_double(&p[1])?))
        }
        Func::LognormMoments => {
            check!(n == 2);
            check!(!(to_double(&p[0])? < 0.0));
            check!(!(to_double(&p[1])? < 0.0));
            d(rng.sample_lognorm_moments(to_double(&p[0])?, to_double(&p[1])?))
        }
        Func::Gamma => {
            check!(n == 2);
            check!(!(to_double(&p[1])? <= 0.0));
            check!(!(to_double(&p[0])? <= 0.0));
            d(rng.sample_gamma(to_double(&p[0])?, to_double(&p[1])?))
        }
        Func::GammaMoments => {
            check!(n == 2);
            check!(!(to_double(&p[0])? < 0.0));
            check!(!(to_double(&p[1])? < 0.0));
            d(rng.sample_gamma_moments(to_double(&p[0])?, to_double(&p[1])?))
        }
        Func::Min | Func::Max => {
            check!(n == 2);
            if matches!(p[0], Value::Null) {
                return Err(err(K::NullPointer, "null argument"));
            }
            check!(is_number(&p[0]) && is_number(&p[1]) && same_class(&p[0], &p[1]));
            Ok(match (&p[0], &p[1], f) {
                (Value::Double(a), Value::Double(b), Func::Max) => {
                    Value::Double(jmath::max_f64(*a, *b))
                }
                (Value::Double(a), Value::Double(b), _) => Value::Double(jmath::min_f64(*a, *b)),
                (Value::Int(a), Value::Int(b), Func::Max) => Value::Int(*a.max(b)),
                (Value::Int(a), Value::Int(b), _) => Value::Int(*a.min(b)),
                _ => unreachable!("checked above"),
            })
        }
        Func::MinDeviation | Func::MaxDeviation => {
            check!(n == 3);
            check!(is_number(&p[0]) || matches!(p[0], Value::Str(_)));
            let (Value::Double(abs), Value::Double(rel)) = (&p[1], &p[2]) else {
                return Err(not_accepted(f));
            };
            let (abs, rel) = (*abs, *rel);
            let max = f == Func::MaxDeviation;
            Ok(match &p[0] {
                Value::Str(_) => p[0].clone(),
                Value::Int(v) => {
                    let v = f64::from(*v);
                    let r = if max {
                        if abs > v * rel {
                            (v + abs).ceil()
                        } else {
                            (v + v * rel).ceil()
                        }
                    } else if abs > v * rel {
                        (v - abs).floor()
                    } else {
                        (v - v * rel).floor()
                    };
                    Value::Int(r as i32)
                }
                Value::Double(v) => {
                    let v = *v;
                    Value::Double(if max {
                        if abs > v * rel {
                            (v + abs).ceil()
                        } else {
                            (v + v * rel).ceil()
                        }
                    } else if abs > v * rel {
                        (v - abs).floor()
                    } else {
                        (v - v * rel).floor()
                    })
                }
                _ => unreachable!("checked above"),
            })
        }
    }
}

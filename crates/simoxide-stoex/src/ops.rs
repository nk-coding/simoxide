//! Operator semantics of `PCMStoExEvaluationVisitor` (simucomframework.variables 5.2.2).
//!
//! Each binary operator receives the *static* types of its operands (see [`crate::types`]),
//! resolves `ANY` from the run-time value, and then picks int or double arithmetic. The static
//! type is trusted: a static INT operand that is a Double at run time (e.g. an `IntPMF` that
//! fell through to `0.0`, or `x.BYTESIZE` bound to a Double) is an error, not a promotion.

use crate::ast::{BoolOp, CmpOp, ProdOp, TermOp};
use crate::error::{EvalError, EvalErrorKind as K};
use crate::jmath;
use crate::types::{SType, TypeEnum};
use crate::value::Value;
use std::cmp::Ordering;

#[cold]
pub(crate) fn err(kind: K, msg: impl Into<String>) -> EvalError {
    EvalError::new(kind, msg)
}

#[cold]
fn class_cast(v: &Value, to: &str) -> EvalError {
    err(
        K::ClassCast,
        format!("class {} cannot be cast to class {to}", v.java_class()),
    )
}

#[cold]
fn npe() -> EvalError {
    err(K::NullPointer, "null value")
}

/// `getDynamicType`.
#[inline]
pub fn dynamic_type(v: &Value) -> Result<TypeEnum, EvalError> {
    Ok(match v {
        Value::Int(_) => TypeEnum::Int,
        Value::Double(_) => TypeEnum::Double,
        Value::Str(_) => TypeEnum::Enum,
        Value::Bool(_) => TypeEnum::Bool,
        Value::Null => {
            return Err(err(
                K::Runtime,
                "Unknown dynamic type found! Should never happen!",
            ));
        }
    })
}

#[inline]
fn resolve(t: SType, v: &Value) -> Result<SType, EvalError> {
    if t == Some(TypeEnum::Any) {
        Ok(Some(dynamic_type(v)?))
    } else {
        Ok(t)
    }
}

/// `getDouble` of the visitor.
#[inline]
pub fn get_double(v: &Value) -> Result<f64, EvalError> {
    match v {
        Value::Double(d) => Ok(*d),
        Value::Int(i) => Ok(f64::from(*i)),
        Value::Null => Err(npe()),
        _ => Err(err(
            K::UnsupportedOperation,
            format!("Trying to cast a {} to a Double!", v.java_class()),
        )),
    }
}

/// `(Integer) v` followed by unboxing.
#[inline]
fn cast_int(v: &Value) -> Result<i32, EvalError> {
    match v {
        Value::Int(i) => Ok(*i),
        Value::Null => Err(npe()),
        _ => Err(class_cast(v, "java.lang.Integer")),
    }
}

/// `(Double) v` followed by unboxing.
#[inline]
fn cast_double(v: &Value) -> Result<f64, EvalError> {
    match v {
        Value::Double(d) => Ok(*d),
        Value::Null => Err(npe()),
        _ => Err(class_cast(v, "java.lang.Double")),
    }
}

/// `(Boolean) v` followed by unboxing.
#[inline]
pub fn cast_bool(v: &Value) -> Result<bool, EvalError> {
    match v {
        Value::Bool(b) => Ok(*b),
        Value::Null => Err(npe()),
        _ => Err(class_cast(v, "java.lang.Boolean")),
    }
}

/// `caseTermExpression`.
#[inline]
pub fn term(op: TermOp, lt: SType, rt: SType, l: &Value, r: &Value) -> Result<Value, EvalError> {
    let lt = resolve(lt, l)?;
    let rt = resolve(rt, r)?;
    if lt == Some(TypeEnum::Int) && rt == Some(TypeEnum::Int) {
        match (l, r) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(match op {
                TermOp::Add => a.wrapping_add(*b),
                TermOp::Sub => a.wrapping_sub(*b),
            })),
            _ => Err(err(
                K::TypesIncompatibleInTerm,
                "Incompatible types in term expression. Expecting Integer!",
            )),
        }
    } else {
        let a = get_double(l)?;
        let b = get_double(r)?;
        Ok(Value::Double(match op {
            TermOp::Add => a + b,
            TermOp::Sub => jmath::dsub(a, b),
        }))
    }
}

/// `caseProductExpression`.
#[inline]
pub fn product(op: ProdOp, lt: SType, rt: SType, l: &Value, r: &Value) -> Result<Value, EvalError> {
    let lt = resolve(lt, l)?;
    let rt = resolve(rt, r)?;
    if lt == Some(TypeEnum::Int) && rt == Some(TypeEnum::Int) {
        match (l, r) {
            (Value::Int(a), Value::Int(b)) => {
                let (a, b) = (*a, *b);
                Ok(Value::Int(match op {
                    ProdOp::Mult => a.wrapping_mul(b),
                    ProdOp::Div | ProdOp::Mod if b == 0 => {
                        return Err(err(K::Arithmetic, "/ by zero"));
                    }
                    ProdOp::Div => a.wrapping_div(b),
                    ProdOp::Mod => a.wrapping_rem(b),
                }))
            }
            _ => Err(err(
                K::TypesIncompatibleInProduct,
                "Incompatible types in product expression. Expecting Integer!",
            )),
        }
    } else {
        let a = get_double(l)?;
        let b = get_double(r)?;
        Ok(Value::Double(match op {
            ProdOp::Mult => a * b,
            ProdOp::Div => a / b,
            ProdOp::Mod => jmath::drem(a, b),
        }))
    }
}

/// `casePowerExpression`: always `Math.pow` on doubles.
#[inline]
pub fn power(lt: SType, rt: SType, b: &Value, x: &Value) -> Result<Value, EvalError> {
    let lt = resolve(lt, b)?;
    let rt = resolve(rt, x)?;
    let base = if lt == Some(TypeEnum::Int) {
        f64::from(cast_int(b)?)
    } else {
        cast_double(b)?
    };
    let exp = if rt == Some(TypeEnum::Int) {
        f64::from(cast_int(x)?)
    } else {
        cast_double(x)?
    };
    Ok(Value::Double(jmath::pow(base, exp)))
}

/// `caseCompareExpression`.
#[inline]
pub fn compare(op: CmpOp, lt: SType, rt: SType, l: &Value, r: &Value) -> Result<Value, EvalError> {
    let lt = resolve(lt, l)?;
    let rt = resolve(rt, r)?;
    let promote = |v: &Value| cast_int(v).map(|i| Value::Double(f64::from(i)));
    let mut lv = std::borrow::Cow::Borrowed(l);
    let mut rv = std::borrow::Cow::Borrowed(r);
    if lt == Some(TypeEnum::Int) && rt == Some(TypeEnum::Double) {
        lv = std::borrow::Cow::Owned(promote(l)?);
    }
    if rt == Some(TypeEnum::Int) && lt == Some(TypeEnum::Double) {
        rv = std::borrow::Cow::Owned(promote(r)?);
    }
    let ord = match (&*lv, &*rv) {
        (Value::Null, _) | (_, Value::Null) => return Err(npe()),
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        (Value::Double(a), Value::Double(b)) => jmath::double_compare(*a, *b),
        (Value::Str(a), Value::Str(b)) => jmath::string_compare(a, b),
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (a, b) => {
            return Err(err(
                K::TypesIncompatibleInComparison,
                format!("Can not compare {} to {}", a.java_class(), b.java_class()),
            ));
        }
    };
    Ok(Value::Bool(match op {
        CmpOp::Equals => ord == Ordering::Equal,
        CmpOp::Less => ord == Ordering::Less,
        CmpOp::LessEqual => ord != Ordering::Greater,
        CmpOp::Greater => ord == Ordering::Greater,
        CmpOp::GreaterEqual => ord != Ordering::Less,
        CmpOp::NotEqual => ord != Ordering::Equal,
    }))
}

/// `caseBooleanOperatorExpression` (both operands are always evaluated).
#[inline]
pub fn bool_op(op: BoolOp, a: bool, b: bool) -> bool {
    match op {
        BoolOp::Or => a || b,
        BoolOp::And => a && b,
        BoolOp::Xor => a ^ b,
    }
}

/// `caseNegativeExpression`.
#[inline]
pub fn neg(v: &Value) -> Result<Value, EvalError> {
    match v {
        Value::Int(i) => Ok(Value::Int(i.wrapping_neg())),
        Value::Double(d) => Ok(Value::Double(-d)),
        _ => Err(err(
            K::Runtime,
            "Type mismatch, unary minus only supported for numbers!",
        )),
    }
}

/// `caseNotExpression`.
#[inline]
pub fn not(v: &Value) -> Result<Value, EvalError> {
    Ok(Value::Bool(!cast_bool(v)?))
}

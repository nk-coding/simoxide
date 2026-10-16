//! OCL arithmetic and comparison of numbers as classic Eclipse OCL evaluates them
//! (`EvaluationVisitorImpl` operation calls; `reference/notes/eclipse-semantics.md` §B):
//! operands are promoted (`Integer`, `Short`, `Byte` to `Long`, `Float` to `Double`), integer
//! results wrap in 64 bits and are normalised back to `Integer` when they fit, `Int / Int` is a
//! Real, division and `div`/`mod` by zero are invalid, `floor` and `round` cast to `int`.
//!
//! UnlimitedNatural `*` is the Integer -1, recognised only through the static type: callers
//! pass `unlimited` flags for operands whose static type is UnlimitedNatural.

use crate::value::{IntBox, JNum, Value};

/// Binary numeric operations.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum NumOp {
    Plus,
    Minus,
    Times,
    Divide,
    Div,
    Mod,
    Max,
    Min,
    Lt,
    Gt,
    Le,
    Ge,
}

/// Java's `Math.max(double, double)`: NaN if either is NaN, `max(-0.0, 0.0) = 0.0`.
pub fn java_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() { b } else { a }
    } else if a >= b {
        a
    } else {
        b
    }
}

/// Java's `Math.min(double, double)`: NaN if either is NaN, `min(-0.0, 0.0) = -0.0`.
pub fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() { a } else { b }
    } else if a <= b {
        a
    } else {
        b
    }
}

/// `Math.round(double)`: the closest `long`, ties toward positive infinity; NaN gives 0,
/// out-of-range values saturate.
pub fn java_round(x: f64) -> i64 {
    if x.is_nan() {
        return 0;
    }
    let f = x.floor();
    let r = if x - f >= 0.5 { f + 1.0 } else { f };
    r as i64
}

fn int(v: i64) -> Value {
    Value::Int(v)
}

/// `source op arg` for a numeric source. Returns `None` when the source is not a number (the
/// caller dispatches to other operand types). An invalid or null argument gives invalid.
pub fn binary(op: NumOp, a: &Value, b: &Value, a_unlimited: bool, b_unlimited: bool) -> Option<Value> {
    let x = a.as_jnum()?.higher();
    if b.is_undefined() {
        return Some(Value::Invalid);
    }
    let Some(y) = b.as_jnum() else {
        // a non-number argument: Java's casts fail
        return Some(Value::Invalid);
    };
    let y = y.higher();
    use NumOp::*;
    Some(match (x, y) {
        (JNum::Long(p), JNum::Long(q)) => {
            let su = a_unlimited && p == -1;
            let au = b_unlimited && q == -1;
            if su && au {
                return Some(match op {
                    Lt | Gt => Value::Bool(false),
                    Le | Ge => Value::Bool(true),
                    _ => Value::Invalid,
                });
            } else if su || au {
                return Some(match op {
                    Lt | Le => Value::Bool(au),
                    Gt | Ge => Value::Bool(su),
                    _ => Value::Invalid,
                });
            }
            match op {
                Plus => int(p.wrapping_add(q)),
                Minus => int(p.wrapping_sub(q)),
                Times => int(p.wrapping_mul(q)),
                Divide => {
                    if q == 0 {
                        Value::Invalid
                    } else {
                        Value::Real(p as f64 / q as f64)
                    }
                }
                Div => {
                    if q == 0 {
                        Value::Invalid
                    } else {
                        int(p.wrapping_div(q))
                    }
                }
                Mod => {
                    if q == 0 {
                        Value::Invalid
                    } else {
                        int(p.wrapping_rem(q))
                    }
                }
                Max => int(p.max(q)),
                Min => int(p.min(q)),
                Lt => Value::Bool(p < q),
                Gt => Value::Bool(p > q),
                Le => Value::Bool(p <= q),
                Ge => Value::Bool(p >= q),
            }
        }
        (JNum::Long(p), JNum::Double(q)) => {
            if a_unlimited && p == -1 {
                return Some(match op {
                    Lt => Value::Bool(false),
                    Gt | Ge => Value::Bool(true),
                    _ => Value::Invalid,
                });
            }
            let pf = p as f64;
            match op {
                Plus => Value::Real(pf + q),
                Minus => Value::Real(pf - q),
                Times => Value::Real(pf * q),
                Divide => {
                    if q == 0.0 {
                        Value::Invalid
                    } else {
                        Value::Real(pf / q)
                    }
                }
                Max => Value::Real(java_max(pf, q)),
                Min => Value::Real(java_min(pf, q)),
                Lt => Value::Bool(pf < q),
                Gt => Value::Bool(pf > q),
                Le => Value::Bool(pf <= q),
                Ge => Value::Bool(pf >= q),
                Div | Mod => Value::Invalid,
            }
        }
        (JNum::Double(p), JNum::Long(q)) => {
            if b_unlimited && q == -1 {
                return Some(match op {
                    Lt => Value::Bool(true),
                    Gt | Ge => Value::Bool(false),
                    _ => Value::Invalid,
                });
            }
            let qf = q as f64;
            match op {
                Plus => Value::Real(p + qf),
                Minus => Value::Real(p - qf),
                Times => Value::Real(p * qf),
                Divide => {
                    if q == 0 {
                        Value::Invalid
                    } else {
                        Value::Real(p / qf)
                    }
                }
                Max => Value::Real(java_max(p, qf)),
                Min => Value::Real(java_min(p, qf)),
                Lt => Value::Bool(p < qf),
                Gt => Value::Bool(p > qf),
                Le => Value::Bool(p <= qf),
                Ge => Value::Bool(p >= qf),
                Div | Mod => Value::Invalid,
            }
        }
        (JNum::Double(p), JNum::Double(q)) => match op {
            Plus => Value::Real(p + q),
            Minus => Value::Real(p - q),
            Times => Value::Real(p * q),
            Divide => {
                if q == 0.0 {
                    Value::Invalid
                } else {
                    Value::Real(p / q)
                }
            }
            Max => Value::Real(java_max(p, q)),
            Min => Value::Real(java_min(p, q)),
            Lt => Value::Bool(p < q),
            Gt => Value::Bool(p > q),
            Le => Value::Bool(p <= q),
            Ge => Value::Bool(p >= q),
            Div | Mod => Value::Invalid,
        },
        _ => unreachable!("higher() gives Long or Double"),
    })
}

/// Unary minus. Integer `-MIN_VALUE` becomes a Long; a Long `-2^31` an Integer; other number
/// classes (Short, Byte, Float, Double) give a Real.
pub fn negate(a: &Value, unlimited: bool) -> Value {
    match *a {
        Value::Int(-1) if unlimited => Value::Invalid,
        Value::Int(v) if v as i32 as i64 == v => int(-v),
        Value::Int(v) => {
            // a Long
            if v == 2_147_483_648 { int(i32::MIN as i64) } else { Value::long(v.wrapping_neg()) }
        }
        // `UNLIMITED.equals(sourceVal)` is false for a Long box
        Value::Boxed(IntBox::Long, v) => Value::long(-(v as i64)),
        _ => match a.as_jnum() {
            Some(n) => Value::Real(-n.as_f64()),
            None => Value::Invalid,
        },
    }
}

/// `abs()`. Integer `MIN_VALUE` becomes a Long; `Long.MIN_VALUE` stays negative; a Long result
/// stays a Long box.
pub fn abs(a: &Value, unlimited: bool) -> Value {
    match *a {
        Value::Int(-1) if unlimited => Value::Invalid,
        Value::Int(v) if v as i32 as i64 == v => int(v.abs()),
        Value::Int(v) => Value::long(v.wrapping_abs()),
        Value::Boxed(IntBox::Long, v) => {
            if unlimited && v == -1 {
                Value::Invalid
            } else {
                Value::long((v as i64).abs())
            }
        }
        _ => match a.as_jnum() {
            Some(n) => Value::Real(n.as_f64().abs()),
            None => Value::Invalid,
        },
    }
}

/// `floor()`: a Real gives `(int) Math.floor(x)` (saturating, NaN 0); an Integer is returned
/// unchanged.
pub fn floor(a: &Value, unlimited: bool) -> Value {
    match a {
        Value::Real(d) => int(d.floor() as i32 as i64),
        Value::Float(f) => int((*f as f64).floor() as i32 as i64),
        v if unlimited && v.as_jnum().map(|n| n.as_i64()) == Some(-1) => Value::Invalid,
        v if v.is_number() => v.clone(),
        _ => Value::Invalid,
    }
}

/// `round()`: a Real gives `(int) Math.round(x)` (the `long` result is truncated to 32 bits).
pub fn round(a: &Value, unlimited: bool) -> Value {
    match a {
        Value::Real(d) => int(java_round(*d) as i32 as i64),
        Value::Float(f) => int(java_round(*f as f64) as i32 as i64),
        v if unlimited && v.as_jnum().map(|n| n.as_i64()) == Some(-1) => Value::Invalid,
        v if v.is_number() => v.clone(),
        _ => Value::Invalid,
    }
}

/// `JNum` of a value, for callers that need the Java class.
pub fn jnum(v: &Value) -> Option<JNum> {
    v.as_jnum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        let i = |v: i64| Value::Int(v);
        let b = |op, x: &Value, y: &Value| binary(op, x, y, false, false).unwrap();
        assert!(matches!(b(NumOp::Plus, &i(2147483647), &i(1)), Value::Int(2147483648)));
        assert!(matches!(b(NumOp::Times, &i(i64::MAX), &i(2)), Value::Int(-2)));
        assert!(matches!(b(NumOp::Divide, &i(4), &i(2)), Value::Real(x) if x == 2.0));
        assert!(b(NumOp::Divide, &i(4), &i(0)).is_invalid());
        assert!(b(NumOp::Mod, &i(4), &i(0)).is_invalid());
        assert!(matches!(b(NumOp::Mod, &i(-7), &i(2)), Value::Int(-1)));
        assert!(matches!(b(NumOp::Div, &i(-7), &i(2)), Value::Int(-3)));
        assert!(matches!(b(NumOp::Max, &i(1), &Value::Real(2.0)), Value::Real(x) if x == 2.0));
        assert!(matches!(binary(NumOp::Lt, &i(-1), &i(5), true, false), Some(Value::Bool(false))));
        assert!(matches!(negate(&i(i32::MIN as i64), false), Value::Int(2147483648)));
        assert!(matches!(round(&Value::Real(-2.5), false), Value::Int(-2)));
        assert!(matches!(round(&Value::Real(3e9), false), Value::Int(-1294967296)));
        assert!(matches!(floor(&Value::Real(1e10), false), Value::Int(2147483647)));
        assert_eq!(java_round(0.49999999999999994), 0);
    }
}

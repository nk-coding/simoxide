//! Runtime values: the Java objects the StoEx visitor passes around.

use crate::error::{EvalError, EvalErrorKind};
use std::fmt;
use std::sync::Arc;

/// A StoEx value (`Integer`, `Double`, `Boolean`, `String`; `Null` only appears for missing
/// variables in [`crate::VariableMode::ReturnNull`]).
#[derive(Clone, Debug)]
pub enum Value {
    Int(i32),
    Double(f64),
    Bool(bool),
    Str(Arc<str>),
    Null,
}

impl PartialEq for Value {
    /// Java `equals`: doubles compare by bits (so NaN equals NaN, `0.0 != -0.0`).
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Double(a), Value::Double(b)) => a.to_bits() == b.to_bits(),
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Null, Value::Null) => true,
            _ => false,
        }
    }
}

impl From<i32> for Value {
    fn from(v: i32) -> Self {
        Value::Int(v)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Double(v)
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Str(Arc::from(v))
    }
}

/// The Java class a result must have (`StackContext.evaluateStatic(spec, expectedType, ...)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expected {
    Integer,
    Long,
    Double,
    Boolean,
    String,
}

impl Expected {
    fn java_name(self) -> &'static str {
        match self {
            Expected::Integer => "java.lang.Integer",
            Expected::Long => "java.lang.Long",
            Expected::Double => "java.lang.Double",
            Expected::Boolean => "java.lang.Boolean",
            Expected::String => "java.lang.String",
        }
    }
}

/// A value converted to an expected type.
#[derive(Debug, Clone, PartialEq)]
pub enum Converted {
    Integer(i32),
    Long(i64),
    Double(f64),
    Boolean(bool),
    String(Arc<str>),
}

impl Value {
    /// Canonical Java class name of the value.
    pub fn java_class(&self) -> &'static str {
        match self {
            Value::Int(_) => "java.lang.Integer",
            Value::Double(_) => "java.lang.Double",
            Value::Bool(_) => "java.lang.Boolean",
            Value::Str(_) => "java.lang.String",
            Value::Null => "null",
        }
    }

    /// Bitwise identity (same as `==`).
    pub fn same(&self, other: &Value) -> bool {
        self == other
    }

    /// `evaluateStatic(..., expectedType, ...)`: an Integer converts to Long/Double; nothing
    /// else converts (a Double never becomes an Integer).
    pub fn convert(&self, expected: Expected) -> Result<Converted, EvalError> {
        let r = match (expected, self) {
            (Expected::Integer, Value::Int(v)) => Converted::Integer(*v),
            (Expected::Long, Value::Int(v)) => Converted::Long(i64::from(*v)),
            (Expected::Double, Value::Double(v)) => Converted::Double(*v),
            (Expected::Double, Value::Int(v)) => Converted::Double(f64::from(*v)),
            (Expected::Boolean, Value::Bool(v)) => Converted::Boolean(*v),
            (Expected::String, Value::Str(v)) => Converted::String(v.clone()),
            (_, Value::Null) => {
                return Err(EvalError::new(
                    EvalErrorKind::NullPointer,
                    "evaluation result is null",
                ));
            }
            _ => {
                return Err(EvalError::new(
                    EvalErrorKind::UnsupportedOperation,
                    format!(
                        "Evaluation result is of type {} but expected was {} and no conversion was available...",
                        self.java_class(),
                        expected.java_name()
                    ),
                ));
            }
        };
        Ok(r)
    }

    /// `evaluateStatic(spec, Double.class, ...)`.
    pub fn to_f64(&self) -> Result<f64, EvalError> {
        match self.convert(Expected::Double)? {
            Converted::Double(v) => Ok(v),
            _ => unreachable!(),
        }
    }

    /// `evaluateStatic(spec, Integer.class, ...)`.
    pub fn to_i32(&self) -> Result<i32, EvalError> {
        match self.convert(Expected::Integer)? {
            Converted::Integer(v) => Ok(v),
            _ => unreachable!(),
        }
    }

    /// `evaluateStatic(spec, Boolean.class, ...)`.
    pub fn to_bool(&self) -> Result<bool, EvalError> {
        match self.convert(Expected::Boolean)? {
            Converted::Boolean(v) => Ok(v),
            _ => unreachable!(),
        }
    }
}

impl fmt::Display for Value {
    /// Java `toString` of the boxed value (`Double.toString` formatting is approximated by the
    /// shortest round-trip representation).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(v) => write!(f, "{v}"),
            Value::Double(v) => write!(f, "{v:?}"),
            Value::Bool(v) => write!(f, "{v}"),
            Value::Str(v) => f.write_str(v),
            Value::Null => f.write_str("null"),
        }
    }
}

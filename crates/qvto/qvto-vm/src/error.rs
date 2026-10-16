//! QVT exceptions and their stack traces (`QvtRuntimeException` and subclasses of Eclipse).
//!
//! Only exceptions unwind: `raise`d exceptions and failed fatal assertions (Eclipse's
//! `QvtException`, catchable by `try`/`except`), and the fatal ones (`QvtRuntimeException`,
//! stack overflow, interruption). Java runtime exceptions inside expressions are not
//! exceptions here: the expression yields `invalid` (DESIGN §5.4, compat row 15).

use qvto_model::ClassId;
use std::fmt;

/// One frame of a QVT stack trace (`QVTStackTraceElement`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StackElem {
    pub module: String,
    /// `op` or `Context::op`; the module name for module initialisation.
    pub operation: String,
    /// The source file name, `None` for unknown sources.
    pub unit: Option<String>,
    /// 1-based, `None` if unknown.
    pub line: Option<u32>,
}

impl fmt::Display for StackElem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}", self.module, self.operation)?;
        match &self.unit {
            Some(u) => match self.line {
                Some(l) => write!(f, "({u}:{l})"),
                None => write!(f, "({u}:<Unknown>)"),
            },
            None => f.write_str("(Unknown Source)"),
        }
    }
}

/// The Java class of a QVT exception.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// `raise T(...)`: `QvtException` with the exception type (`None` for `raise 'msg'`).
    Raised(Option<ClassId>),
    /// `QvtAssertionFailed` (fatal assertion, `xmap` precondition, `where` postcondition).
    AssertionFailed,
    /// `QvtRuntimeException`.
    Runtime,
    /// `QvtStackOverFlowError`.
    StackOverflow,
    /// `QvtInterruptedExecutionException`.
    Interrupted,
}

/// A QVT exception in flight.
#[derive(Clone, Debug)]
pub struct QvtError {
    pub kind: ErrorKind,
    pub message: Option<String>,
    pub stack: Vec<StackElem>,
    /// The exception type of an `AssertionFailed` (the stdlib class), for catch matching.
    pub type_class: Option<ClassId>,
}

impl QvtError {
    pub fn java_class(&self) -> &'static str {
        match self.kind {
            ErrorKind::Raised(_) => "org.eclipse.m2m.internal.qvt.oml.evaluator.QvtException",
            ErrorKind::AssertionFailed => {
                "org.eclipse.m2m.internal.qvt.oml.evaluator.QvtAssertionFailed"
            }
            ErrorKind::Runtime => "org.eclipse.m2m.internal.qvt.oml.evaluator.QvtRuntimeException",
            ErrorKind::StackOverflow => {
                "org.eclipse.m2m.internal.qvt.oml.evaluator.QvtStackOverFlowError"
            }
            ErrorKind::Interrupted => {
                "org.eclipse.m2m.internal.qvt.oml.evaluator.QvtInterruptedExecutionException"
            }
        }
    }

    /// Caught by `try`/`except` (Eclipse catches `QvtException` only).
    pub fn catchable(&self) -> bool {
        matches!(self.kind, ErrorKind::Raised(_) | ErrorKind::AssertionFailed)
    }

    /// `Throwable.toString()`.
    pub fn header(&self) -> String {
        match &self.message {
            Some(m) => format!("{}: {m}", self.java_class()),
            None => self.java_class().to_string(),
        }
    }

    /// `QvtRuntimeException.printQvtStackTrace`: the header line, then `\tat ...` lines.
    pub fn print_stack_trace(&self) -> String {
        let mut s = self.header();
        for e in &self.stack {
            s.push('\n');
            s.push_str("\tat ");
            s.push_str(&e.to_string());
        }
        s
    }

    /// The execution diagnostic's message (`InternalTransformationExecutor.createExecutionFailure`).
    pub fn diagnostic_message(&self) -> String {
        if self.kind == ErrorKind::StackOverflow {
            return "QVT Stack overflow occured during execution".into();
        }
        match &self.message {
            Some(m) => m.clone(),
            None => format!("QVT exception caught : '{}'", self.java_class()),
        }
    }

    /// The execution diagnostic's code: 100 fatal assertion, 110 interrupted, 120 other.
    pub fn diagnostic_code(&self) -> i32 {
        match self.kind {
            ErrorKind::AssertionFailed => 100,
            ErrorKind::Interrupted => 110,
            _ => 120,
        }
    }
}

/// Why execution of a chunk stopped early.
#[derive(Clone, Debug)]
pub enum Unwind {
    Error(Box<QvtError>),
}

impl Unwind {
    pub fn error(&self) -> &QvtError {
        match self {
            Unwind::Error(e) => e,
        }
    }
}

impl From<QvtError> for Unwind {
    fn from(e: QvtError) -> Unwind {
        Unwind::Error(Box::new(e))
    }
}

pub type VmResult<T> = Result<T, Unwind>;

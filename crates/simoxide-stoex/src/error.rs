//! Error types.
//!
//! Every error of the reference aborts the evaluation (and in the simulator the run). The Rust
//! errors keep the name of the Java exception class the reference throws at that point
//! ([`EvalError::java_class`]), which the golden tests compare.

use std::fmt;

/// Byte range in the source text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }
}

/// A syntax error (the reference throws `java.text.ParseException`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
    /// 1-based line of `span.start`.
    pub line: usize,
    /// 1-based column (in characters) of `span.start`.
    pub column: usize,
}

impl ParseError {
    pub(crate) fn new(src: &str, span: Span, message: impl Into<String>) -> Self {
        let start = span.start.min(src.len());
        let before = &src[..start];
        let line = before.matches('\n').count() + 1;
        let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        ParseError {
            message: message.into(),
            span,
            line,
            column,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.column, self.message)
    }
}

impl std::error::Error for ParseError {}

/// Errors of [`crate::prepare`]: what `new StoExCacheEntry(spec)` throws.
#[derive(Debug, Clone, PartialEq)]
pub enum PrepareError {
    /// The text is not a StoEx (`ParseException`, wrapped in `RuntimeException`).
    Parse(ParseError),
    /// Type inference rejected a function name
    /// (`UnsupportedOperationException: ...: Function X not supported!`).
    UnknownFunction { name: String, span: Span },
    /// A PMF/PDF literal is invalid after the probability adjustment ("PMF not valid"/"PDF not
    /// valid": probability sum, negative probability, duplicate or zero PDF value, ...).
    InvalidProbFunction { message: String, span: Span },
}

impl PrepareError {
    /// Innermost Java exception class the reference reports.
    pub fn java_class(&self) -> &'static str {
        match self {
            PrepareError::Parse(_) => "java.text.ParseException",
            PrepareError::UnknownFunction { .. } => "java.lang.UnsupportedOperationException",
            PrepareError::InvalidProbFunction { .. } => "java.lang.RuntimeException",
        }
    }
}

impl fmt::Display for PrepareError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrepareError::Parse(e) => write!(f, "syntax error at {e}"),
            PrepareError::UnknownFunction { name, .. } => {
                write!(f, "Function {name} not supported!")
            }
            PrepareError::InvalidProbFunction { message, .. } => f.write_str(message),
        }
    }
}

impl std::error::Error for PrepareError {}

impl From<ParseError> for PrepareError {
    fn from(e: ParseError) -> Self {
        PrepareError::Parse(e)
    }
}

/// Kind of an evaluation error, named after the Java exception the reference throws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EvalErrorKind {
    /// `TypesIncompatibleInTermException` (`+`/`-` with a static INT operand that is no Integer).
    TypesIncompatibleInTerm,
    /// `TypesIncompatibleInProductException`.
    TypesIncompatibleInProduct,
    /// `TypesIncompatibleInComparisionException` (sic).
    TypesIncompatibleInComparison,
    /// `ClassCastException` (a cast in the visitor failed).
    ClassCast,
    /// `NullPointerException` (a `null` variable value reached an operator).
    NullPointer,
    /// `ArithmeticException` (integer division or modulo by zero).
    Arithmetic,
    /// `UnsupportedOperationException` (e.g. `getDouble` of a non-number).
    UnsupportedOperation,
    /// `FunctionUnknownException` (e.g. `Binom`, which type inference knows but `FunctionLib`
    /// does not).
    FunctionUnknown,
    /// `FunctionParametersNotAcceptedException`.
    FunctionParametersNotAccepted,
    /// `IllegalArgumentException`.
    IllegalArgument,
    /// Plain `RuntimeException` (missing variable, unknown dynamic type, unary minus of a
    /// non-number, NumberConverter failure, "No interval found" of a PDF, ...).
    Runtime,
    /// A distribution constructor or inversion failed (Commons Math exception).
    Distribution,
}

impl EvalErrorKind {
    /// Fully qualified name of the Java exception class.
    pub fn java_class(self) -> &'static str {
        match self {
            EvalErrorKind::TypesIncompatibleInTerm => {
                "de.uka.ipd.sdq.simucomframework.variables.exceptions.TypesIncompatibleInTermException"
            }
            EvalErrorKind::TypesIncompatibleInProduct => {
                "de.uka.ipd.sdq.simucomframework.variables.exceptions.TypesIncompatibleInProductException"
            }
            EvalErrorKind::TypesIncompatibleInComparison => {
                "de.uka.ipd.sdq.simucomframework.variables.exceptions.TypesIncompatibleInComparisionException"
            }
            EvalErrorKind::ClassCast => "java.lang.ClassCastException",
            EvalErrorKind::NullPointer => "java.lang.NullPointerException",
            EvalErrorKind::Arithmetic => "java.lang.ArithmeticException",
            EvalErrorKind::UnsupportedOperation => "java.lang.UnsupportedOperationException",
            EvalErrorKind::FunctionUnknown => {
                "de.uka.ipd.sdq.simucomframework.variables.exceptions.FunctionUnknownException"
            }
            EvalErrorKind::FunctionParametersNotAccepted => {
                "de.uka.ipd.sdq.simucomframework.variables.exceptions.FunctionParametersNotAcceptedException"
            }
            EvalErrorKind::IllegalArgument => "java.lang.IllegalArgumentException",
            EvalErrorKind::Runtime => "java.lang.RuntimeException",
            EvalErrorKind::Distribution => "org.apache.commons.math.MathException",
        }
    }
}

/// An evaluation error (the reference throws `StochasticExpressionEvaluationFailedException`
/// wrapping the exception named by [`EvalError::kind`]). Boxed so that `Result<Value,
/// EvalError>` stays as small as a [`crate::Value`].
#[derive(Debug, Clone, PartialEq)]
pub struct EvalError(Box<(EvalErrorKind, String)>);

impl EvalError {
    #[cold]
    pub fn new(kind: EvalErrorKind, message: impl Into<String>) -> Self {
        EvalError(Box::new((kind, message.into())))
    }

    /// What went wrong (named after the Java exception).
    pub fn kind(&self) -> EvalErrorKind {
        self.0.0
    }

    /// The message (like the Java exception message where it matters).
    pub fn message(&self) -> &str {
        &self.0.1
    }

    /// Java exception class of the root cause.
    pub fn java_class(&self) -> &'static str {
        self.0.0.java_class()
    }
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.java_class(), self.message())
    }
}

impl std::error::Error for EvalError {}

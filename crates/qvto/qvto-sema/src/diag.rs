//! Problems of a compilation unit, collected as Eclipse's `QvtOperationalEnv` does: errors and
//! warnings in insertion-ordered sets (an identical problem — same message and position — is
//! kept once), and, while the AST validation runs, errors at a position that already has an
//! error are dropped (`setCheckForDuplicateErrors`).

use qvto_syntax::span::byte_to_utf16;
use qvto_syntax::{LineIndex, Span};
use std::collections::HashSet;

pub use qvto_syntax::Severity;

/// A problem with Eclipse's position conventions.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Problem {
    pub severity: Severity,
    /// Byte range in the unit's source.
    pub span: Span,
    pub message: String,
}

/// The span of a problem Eclipse reports without position (its problem object is not a CST
/// node): line -1, offset -1, length 1.
pub const NO_POSITION: Span = Span {
    start: u32::MAX,
    end: u32::MAX,
};

impl Problem {
    /// Whether the problem has a position in the source ([`NO_POSITION`]).
    pub fn has_position(&self) -> bool {
        self.span != NO_POSITION
    }

    /// Line, UTF-16 offset and length as Eclipse prints them (-1, -1, 1 without position).
    pub fn eclipse_position(&self, src: &str, lines: &LineIndex) -> (i64, i64, i64) {
        if !self.has_position() {
            return (-1, -1, 1);
        }
        let (o, l) = self.utf16(src);
        (self.line(lines) as i64, o as i64, l as i64)
    }

    /// UTF-16 offset and length, as Eclipse reports them.
    pub fn utf16(&self, src: &str) -> (usize, usize) {
        let o = byte_to_utf16(src, self.span.start as usize);
        let e = byte_to_utf16(src, self.span.end as usize);
        (o, e.saturating_sub(o))
    }

    /// 1-based line of the start (Eclipse's `getLineNumberOfCharAt`).
    pub fn line(&self, lines: &LineIndex) -> u32 {
        lines.line(self.span.start)
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

#[derive(Clone, Debug, Default)]
pub struct Problems {
    errors: Vec<Problem>,
    warnings: Vec<Problem>,
    seen: HashSet<Problem>,
    /// `myCheckForDuplicateErrors`
    pub check_duplicate_errors: bool,
}

impl Problems {
    pub fn error(&mut self, span: Span, message: impl Into<String>) {
        let p = Problem {
            severity: Severity::Error,
            span,
            message: message.into(),
        };
        if self.check_duplicate_errors && self.errors.iter().any(|e| e.span == span) {
            return;
        }
        if self.seen.insert(p.clone()) {
            self.errors.push(p);
        }
    }

    pub fn warning(&mut self, span: Span, message: impl Into<String>) {
        let p = Problem {
            severity: Severity::Warning,
            span,
            message: message.into(),
        };
        if self.seen.insert(p.clone()) {
            self.warnings.push(p);
        }
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    pub fn warning_count(&self) -> usize {
        self.warnings.len()
    }

    /// Errors then warnings, each in report order (Eclipse's `getAllProblemMessages`).
    pub fn all(&self) -> impl Iterator<Item = &Problem> {
        self.errors.iter().chain(self.warnings.iter())
    }

    pub fn into_vec(self) -> Vec<Problem> {
        let mut v = self.errors;
        v.extend(self.warnings);
        v
    }
}

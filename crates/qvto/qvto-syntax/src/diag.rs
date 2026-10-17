//! Syntax diagnostics.

use crate::span::{LineIndex, Span};
use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

/// Stable diagnostic codes. Each says which Eclipse check it reproduces.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Code {
    // Lexer (LPG lexer errors).
    /// A character no token can start with (LPG: `"c" unexpected character ignored`).
    InvalidCharacter,
    /// A token the lexer could not complete: unterminated string or comment, `1e` (LPG:
    /// `token "..." is invalid`).
    InvalidToken,
    // Parser (LPG error repair).
    /// Tokens the parser had to skip (LPG: `"..." unexpected token(s)`).
    UnexpectedTokens,
    /// A missing token (LPG: `"X" expected after "Y"`).
    Expected,
    /// A wrong token (LPG: `"X" expected instead of "Y"`).
    ExpectedInstead,
    /// Input ended early (LPG: `End-of-File reached after this token "X"`).
    UnexpectedEof,
    /// A `{` or `(` without its closing token (LPG: `missing "}" to complete scope`).
    MissingClose,
    // Checks in Eclipse's parser actions (`AbstractQVTParser`, `AbstractOCLParser`).
    /// An escape sequence `decodeEscapeSequence` does not know.
    InvalidEscape,
    /// Two string literals concatenated on one line.
    StringsOnSameLine,
    /// An integer literal that does not fit in a Java `long`.
    IntegerOverflow,
    /// The left side of the assignment in a `while (x := e; c)` header is not a plain name.
    IdentifierExpectedOnLeftSide,
    /// `+=` in a `while` header.
    InvalidAdditiveAssignment,
    /// `(cond) ? e;` switch alternatives (warning).
    DeprecatedSwitchAlt,
    /// `else ? e;` in a switch (warning).
    DeprecatedSwitchElse,
    /// `import` after other unit elements (warning).
    ImportNotFirst,
    /// A module element before the module header (warning).
    ModuleDeclarationMustPrecede,
    /// A helper or query called `main` (warning).
    EntryAsHelper,
    /// A file without a module, or several modules with other elements (error).
    NoModule,
    /// More than one module and other top-level elements.
    MultipleModulesExtraElements,
    /// A construct the grammar does not allow in this position (C-style `if` as a value, etc.).
    Misplaced,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Code,
    pub span: Span,
    pub message: String,
}

impl Diagnostic {
    pub fn error(code: Code, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Error,
            code,
            span,
            message: message.into(),
        }
    }

    pub fn warning(code: Code, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            severity: Severity::Warning,
            code,
            span,
            message: message.into(),
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// `file:line:col: error: message`.
    pub fn render(&self, file: &str, src: &str, lines: &LineIndex) -> String {
        let lc = lines.line_col(src, self.span.start);
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        format!("{file}:{}:{}: {sev}: {}", lc.line, lc.col, self.message)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} {:?} at {:?}: {}",
            self.severity, self.code, self.span, self.message
        )
    }
}

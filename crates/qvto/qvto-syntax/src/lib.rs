//! Lexer, parser and AST of QVT Operational Mappings (QVT 1.3, chapter 8) and the OCL it embeds.
//!
//! The concrete syntax follows Eclipse QVTo's LPG grammars (see `DESIGN.md`, "Syntax", and the
//! crate README). Keyword, punctuation and character-class tables and the operator precedence
//! levels are generated from the `.gi` files ([`generated`]). Every node carries a byte span.
//! The parser recovers from errors and reports them as [`Diagnostic`]s.
//!
//! ```
//! let parsed = qvto_syntax::parse("transformation T(); main() { log('hi'); }");
//! assert!(parsed.diagnostics.is_empty());
//! assert_eq!(parsed.unit.elements.len(), 2);
//! ```

pub mod ast;
pub mod diag;
pub mod dump;
pub mod escape;
#[rustfmt::skip]
pub mod generated;
pub mod lexer;
mod parser;
pub mod print;
pub mod span;

pub use ast::Unit;
pub use diag::{Code, Diagnostic, Severity};
pub use generated::TokenKind;
pub use span::{LineIndex, Span};

/// Result of parsing one compilation unit.
#[derive(Clone, Debug)]
pub struct Parsed {
    pub unit: Unit,
    /// Lexer and parser diagnostics, sorted by position.
    pub diagnostics: Vec<Diagnostic>,
    /// Comment spans.
    pub comments: Vec<Span>,
}

impl Parsed {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    pub fn errors(&self) -> impl Iterator<Item = &Diagnostic> {
        self.diagnostics.iter().filter(|d| d.is_error())
    }
}

/// Parses a `.qvto` source text.
pub fn parse(src: &str) -> Parsed {
    let lexed = lexer::lex(src);
    let comments = lexed.comments.clone();
    let (unit, diagnostics) = parser::parse_unit(src, lexed);
    Parsed { unit, diagnostics, comments }
}

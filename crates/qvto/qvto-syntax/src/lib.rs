//! Lexer, parser and AST of QVT Operational Mappings (QVT 1.3, chapter 8) and the OCL it embeds.
//!
//! The concrete syntax follows Eclipse QVTo's LPG grammars (see `DESIGN.md`, "Syntax"). Every node
//! carries a span. The parser recovers from errors and reports them as diagnostics.

//! Semantic analysis of QVT Operational Mappings as Eclipse QVTo 3.11 does it: units and
//! imports (`QVTOCompiler`), module structure, model types, name and overload resolution, type
//! checking and Eclipse's desugarings (`QvtOperationalVisitorCS` on classic OCL's
//! `AbstractOCLAnalyzer`), the AST validation (`QvtOperationalValidationVisitor`), with
//! Eclipse's messages. Produces the typed IR ([`ir`]).
//!
//! ```ignore
//! let result = qvto_sema::compile(main_unit, &resolver, registry);
//! for unit in &result.units { for p in &unit.problems { /* severity, span, message */ } }
//! let program = result.program;
//! ```

pub mod diag;
pub mod ir;
pub mod messages;

mod analyzer;
mod env;
mod expr;
mod lookup;
mod module;
mod names;
mod ops;
mod units;
mod validate;

pub use diag::{Problem, Problems, Severity};
pub use units::{compile, scan_metamodel_uris, Compilation, UnitResolver, UnitResult, UnitSource};

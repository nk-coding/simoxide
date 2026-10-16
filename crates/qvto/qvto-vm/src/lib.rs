//! The bytecode interpreter (DESIGN §5): frames and registers, calls of helpers, queries,
//! constructors and mappings with Eclipse's protocol (trace records, `when`/`where`,
//! disjuncts, inherited and merged mappings), virtual dispatch, objects in model extents,
//! QVTo's assignment semantics, `resolve`, late resolve, `log`/`assert`/`raise`/`try`, the
//! standard library over `qvto-runtime`. Generic over the model store (`qvto_model::ModelStore`).
//!
//! ```ignore
//! let mut vm = Vm::new(&program, &mut store, RunOptions::default());
//! let outcome = vm.run_transformation(vec![input_roots, vec![]]);
//! print!("{}", vm.log);
//! ```

mod calls;
pub mod env;
pub mod error;
pub mod extent;
mod jregex;
mod log;
mod meta;
mod model_ops;
pub mod native;
mod objects;
mod run;
mod stdlib;
mod trace;
mod types;
pub mod vm;

pub use env::{RunEnv, TapeHook};
pub use error::{ErrorKind, QvtError, StackElem, Unwind};
pub use native::{NativeCall, NativeError, NativeFn, NativeResult, Natives, java_method_key};
pub use run::RunOutcome;
pub use stdlib::Prim;
pub use trace::{TraceRecord, Traces};
pub use vm::{Diag, ObjHandle, RunOptions, Vm};

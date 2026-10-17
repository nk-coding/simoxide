//! qvto-rs on SimOxide's model graph.
//!
//! - [`registry`]: the metamodel registry of SimOxide's generated tables, ids equal to
//!   SimOxide's (`simoxide_model::meta`), with Ecore and the metamodel objects of every package.
//! - [`Session`] / [`SimStore`]: a `qvto_model::ModelStore` over `simoxide_model::raw::Graph`
//!   (objects read and written in place, handles are SimOxide's `ObjId`s) plus a `qvto_emf`
//!   graph for objects of other metamodels (PRM, SPD, metamodel objects).
//! - [`Journal`]: every change of a run, as EMF notifications report it, for SimOxide's syncers.
//!
//! ```ignore
//! let mut model = simoxide_model::load_dir("corpus/x")?;           // SimOxide's loader
//! let mut session = qvto_simoxide::Session::new();
//! session.load_metamodel(RUNTIME_MEASUREMENT_NS)?;                // other metamodels, then compile
//! let compiled = qvto::compile_with_operations(main, &resolver, session.meta().clone(), &session.operations());
//! session.start_journal();
//! let mut store = session.store(&mut model.graph);
//! let out = qvto::Vm::new(&program, &mut store, opts).run_transformation(args);
//! let changes = session.take_journal();                            // to the syncers
//! ```
//!
//! See `docs/simoxide-integration.md` for the integration plan.

pub mod journal;
pub mod registry;
mod store;

pub use journal::{Change, ChangeKind, Journal, Summary};
pub use qvto_emf::IdPolicy;
pub use store::{EXT_BASE, Session, SimStore};

//! Own implementation of the model API: dynamic metamodels loaded from `.ecore`, resources and
//! resource sets, proxies, XMI load and save with EMF's semantics, and a canonical dump for
//! comparing models.

pub mod canon;
mod chunks;
pub mod compare;
pub mod config;
pub mod datatypes;
pub mod ecore;
mod ecore_hooks;
pub mod fragment;
pub mod graph;
pub mod ids;
pub mod journal;
pub mod load_ops;
pub mod materialize;
pub mod operations;
pub mod register;
pub mod resource_set;
pub mod uri;
pub mod xmi_load;
pub mod xmi_save;

pub use graph::{Graph, MetaLinks, ObjId, Object, Resource, ResourceKind, Value};
pub use ids::IdPolicy;
pub use journal::{Change, ChangeKind, Journal, Summary};
pub use resource_set::ResourceSet;

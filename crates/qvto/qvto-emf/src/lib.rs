//! Own implementation of the model API: dynamic metamodels loaded from `.ecore`, resources and
//! resource sets, proxies, XMI load and save with EMF's semantics, and a canonical dump for
//! comparing models.

pub mod datatypes;
pub mod ecore;
mod ecore_hooks;
pub mod fragment;
pub mod graph;
pub mod register;
pub mod resource_set;
pub mod uri;
pub mod xmi_load;

pub use graph::{Graph, MetaLinks, ObjId, Resource, ResourceKind, Value};
pub use resource_set::ResourceSet;

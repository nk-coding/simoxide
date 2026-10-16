//! Model API: metamodels resolved to dense ids ([`MetaRegistry`]) and the [`ModelStore`] trait
//! through which the engine reads and writes models with EMF's semantics (containment,
//! opposites, ordering, uniqueness, proxies, resources).
//!
//! The engine only talks to models through these types, so it runs on `qvto-emf` and, inside
//! SimOxide, directly on `simoxide_model::raw::Graph`. See `DESIGN.md`, "Model API".
//! [`contract`] is the test suite every store implementation runs.

pub mod contract;
mod datum;
mod ids;
mod meta;
mod store;

pub use datum::{Datum, OwnedDatum};
pub use ids::{ClassId, DataTypeId, EnumId, FeatureId, PackageId, ResourceId};
pub use meta::{
    ClassInfo, Classifier, DataKind, DataTypeInfo, EnumInfo, EnumLiteral, FeatureInfo, FeatureKind,
    MetaError, MetaRegistry, PackageInfo,
};
pub use store::{ModelError, ModelResult, ModelStore};

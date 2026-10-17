//! The EMF core shared by SimOxide and its QVTo engine (formerly `qvto-model`; the QVTo crates
//! still import it as `qvto_model`): metamodels resolved to dense ids ([`MetaRegistry`]), seeded
//! from generated tables ([`seed`]: SimOxide's PCM tables keep their ids) or built at run time,
//! feature values ([`Datum`]), and the [`ModelStore`] trait through which the QVTo engine reads
//! and writes models with EMF's semantics (containment, opposites, ordering, uniqueness, proxies,
//! resources). [`contract`] is the test suite every store implementation runs.

pub mod contract;
mod datum;
mod ids;
mod meta;
mod operation;
pub mod seed;
mod store;

pub use datum::{Datum, OwnedDatum};
pub use ids::{ClassId, DataTypeId, EnumId, FeatureId, PackageId, ResourceId};
pub use meta::{
    ClassInfo, Classifier, DataKind, DataTypeInfo, EnumInfo, EnumLiteral, FeatureInfo, FeatureKind,
    MetaError, MetaRegistry, PackageInfo,
};
pub use operation::{MetaDerivation, MetaMultiplicity, MetaOperation};
pub use store::{ModelError, ModelResult, ModelStore};

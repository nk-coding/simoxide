//! The metamodel registry of SimOxide's generated tables (`simoxide_model::meta::SEED`):
//! packages, classes, features and enums get the same ids as in SimOxide (they *are* the EMF
//! core's ids, `MetaRegistry::from_static`), so model objects are read and written without
//! translating ids. Ecore follows them (`qvto_emf::Graph::with_registry` resolves the tables'
//! Ecore references), then any metamodel loaded later.

use qvto_model::{MetaRegistry, PackageId};
use simoxide_model::meta as sm;

/// Sizes of the SimOxide part of a registry: ids below these are SimOxide's.
#[derive(Copy, Clone, Debug)]
pub struct SimTables {
    pub packages: u32,
    pub classes: u32,
    pub features: u32,
    pub enums: u32,
}

impl SimTables {
    pub fn get() -> SimTables {
        SimTables {
            packages: sm::PACKAGES.len() as u32,
            classes: sm::CLASSES.len() as u32,
            features: sm::FEATURES.len() as u32,
            enums: sm::ENUMS.len() as u32,
        }
    }
}

/// A registry holding SimOxide's packages with their ids (Ecore not yet registered).
pub(crate) fn sim_registry() -> MetaRegistry {
    MetaRegistry::from_static(&sm::SEED)
}

/// The top-level SimOxide packages and the locations other metamodels use for them
/// (`platform:/plugin/<bundle>/<path>`).
pub fn sim_top_packages() -> Vec<(PackageId, &'static str)> {
    sim_registry().static_top_packages()
}

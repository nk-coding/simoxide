//! The change journal: every change a run makes to the models, in order, as EMF notifications
//! would report it. It is the input of SimOxide's syncers (the counterparts of SimuLizar's
//! `ResourceEnvironmentSyncer`, `AllocationLookupSyncer`, ...): after a rule run the host walks
//! the changes and updates its compiled model.
//!
//! Objects of SimOxide's graph get the effective changes, side effects included: both ends of a
//! bidirectional reference, the old container of a moved object, a root leaving its resource.
//! Objects of other metamodels (the PRM model, SPD models, intermediate data) get the operations
//! as called (`set`, `add`, ... on the object itself).

use qvto_model::{ClassId, FeatureId, OwnedDatum, ResourceId};
use std::collections::{BTreeMap, BTreeSet};

/// One change of one object (`obj` is a store handle; for resource events the object added or
/// removed).
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub obj: u32,
    pub kind: ChangeKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChangeKind {
    /// A new object (`EFactory.create`).
    Created(ClassId),
    /// A single-valued feature got a new value (effective values before and after).
    Set {
        feature: FeatureId,
        old: OwnedDatum<u32>,
        new: OwnedDatum<u32>,
    },
    /// A single-valued feature was unset (it reads as its default now).
    Unset {
        feature: FeatureId,
        old: OwnedDatum<u32>,
    },
    /// A value was inserted into a many-valued feature at `index`.
    Added {
        feature: FeatureId,
        index: usize,
        value: OwnedDatum<u32>,
    },
    /// The value at `index` was removed from a many-valued feature.
    Removed {
        feature: FeatureId,
        index: usize,
        value: OwnedDatum<u32>,
    },
    /// The value at `index` was replaced (`EList.set`).
    Replaced {
        feature: FeatureId,
        index: usize,
        old: OwnedDatum<u32>,
        new: OwnedDatum<u32>,
    },
    /// `EList.move(to, from)`.
    Moved {
        feature: FeatureId,
        from: usize,
        to: usize,
    },
    /// The object's container changed (`eContainer` / `eContainmentFeature`).
    Container {
        old: Option<(u32, FeatureId)>,
        new: Option<(u32, FeatureId)>,
    },
    /// The object became a root of a resource, at `index`.
    RootAdded { resource: ResourceId, index: usize },
    /// The object is no longer a root of the resource.
    RootRemoved { resource: ResourceId },
    /// `EcoreUtil.delete` was called on the object (the removals it caused are recorded before).
    Deleted,
}

/// The journal of a store (see [`crate::Session::start_journal`]).
#[derive(Clone, Debug, Default)]
pub struct Journal {
    pub changes: Vec<Change>,
}

impl Journal {
    pub fn push(&mut self, obj: u32, kind: ChangeKind) {
        self.changes.push(Change { obj, kind });
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// What changed, per object: the features whose value changed, created and deleted objects,
    /// objects whose container changed. The usual input of a syncer that does not replay the
    /// changes one by one.
    pub fn summary(&self) -> Summary {
        let mut s = Summary::default();
        for c in &self.changes {
            match &c.kind {
                ChangeKind::Created(class) => {
                    s.created.insert(c.obj, *class);
                }
                ChangeKind::Set { feature, .. }
                | ChangeKind::Unset { feature, .. }
                | ChangeKind::Added { feature, .. }
                | ChangeKind::Removed { feature, .. }
                | ChangeKind::Replaced { feature, .. }
                | ChangeKind::Moved { feature, .. } => {
                    s.features.entry(c.obj).or_default().insert(*feature);
                }
                ChangeKind::Container { .. } => {
                    s.moved.insert(c.obj);
                }
                ChangeKind::RootAdded { .. } | ChangeKind::RootRemoved { .. } => {
                    s.moved.insert(c.obj);
                }
                ChangeKind::Deleted => {
                    s.deleted.insert(c.obj);
                }
            }
        }
        s
    }
}

/// See [`Journal::summary`].
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub created: BTreeMap<u32, ClassId>,
    pub features: BTreeMap<u32, BTreeSet<FeatureId>>,
    /// Objects whose container or resource changed.
    pub moved: BTreeSet<u32>,
    pub deleted: BTreeSet<u32>,
}

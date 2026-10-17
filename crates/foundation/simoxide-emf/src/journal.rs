//! The change journal of a [`Graph`](crate::Graph): every effective change, in order, as EMF
//! notifications would report it, side effects included (both ends of a bidirectional reference,
//! the old container of a moved object, a root leaving its resource). It is the input of
//! SimOxide's syncers: after a model change (a QVTo run, a direct edit) the host turns the changes
//! into a change set and updates its compiled model (`DESIGN-SPD.md` 5.3).
//!
//! Recording is off by default (one branch per write); [`Graph::start_journal`](crate::Graph::start_journal)
//! turns it on. Loading (the XMI readers and the loader-level writes) is never recorded.

use crate::graph::ObjId;
use qvto_model::{ClassId, FeatureId, OwnedDatum, ResourceId};
use std::collections::{BTreeMap, BTreeSet};

/// One change of one object (for resource events: the object added or removed).
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub obj: ObjId,
    pub kind: ChangeKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChangeKind {
    /// A new object (`EFactory.create`).
    Created(ClassId),
    /// A single-valued feature got a new value (effective values before and after).
    Set {
        feature: FeatureId,
        old: OwnedDatum<ObjId>,
        new: OwnedDatum<ObjId>,
    },
    /// A single-valued feature was unset (it reads as its default now).
    Unset {
        feature: FeatureId,
        old: OwnedDatum<ObjId>,
    },
    /// A value was inserted into a many-valued feature at `index`.
    Added {
        feature: FeatureId,
        index: usize,
        value: OwnedDatum<ObjId>,
    },
    /// The value at `index` was removed from a many-valued feature.
    Removed {
        feature: FeatureId,
        index: usize,
        value: OwnedDatum<ObjId>,
    },
    /// The value at `index` was replaced (`EList.set`).
    Replaced {
        feature: FeatureId,
        index: usize,
        old: OwnedDatum<ObjId>,
        new: OwnedDatum<ObjId>,
    },
    /// `EList.move(to, from)`.
    Moved {
        feature: FeatureId,
        from: usize,
        to: usize,
    },
    /// The object's container changed (`eContainer` / `eContainmentFeature`).
    Container {
        old: Option<(ObjId, FeatureId)>,
        new: Option<(ObjId, FeatureId)>,
    },
    /// The object became a root of a resource, at `index`.
    RootAdded { resource: ResourceId, index: usize },
    /// The object is no longer a root of the resource.
    RootRemoved { resource: ResourceId },
    /// `EcoreUtil.delete` was called on the object (the removals it caused are recorded before).
    Deleted,
}

/// The changes recorded by a graph.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Journal {
    pub changes: Vec<Change>,
}

impl Journal {
    pub fn push(&mut self, obj: ObjId, kind: ChangeKind) {
        self.changes.push(Change { obj, kind });
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }

    /// What changed, per object: the features whose value changed, created and deleted objects,
    /// objects whose container or resource changed. The usual input of a syncer that does not
    /// replay the changes one by one.
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
                ChangeKind::Container { .. }
                | ChangeKind::RootAdded { .. }
                | ChangeKind::RootRemoved { .. } => {
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
    pub created: BTreeMap<ObjId, ClassId>,
    pub features: BTreeMap<ObjId, BTreeSet<FeatureId>>,
    /// Objects whose container or resource changed.
    pub moved: BTreeSet<ObjId>,
    pub deleted: BTreeSet<ObjId>,
}

//! Operations of metamodel classes (`EOperation`s). The registry keeps none; hosts that know
//! them (`qvto_emf::operations::meta_operations` reads them from loaded `.ecore` files) hand
//! them to the analyser, which resolves calls such as `cls.getEStructuralFeature(n)` to them.

use crate::{ClassId, Classifier};

/// An operation of a metamodel class.
#[derive(Clone, Debug)]
pub struct MetaOperation {
    pub class: ClassId,
    pub name: String,
    /// Name, type and multiplicity (`upperBound > 1 || < 0`, ordered, unique) of each parameter.
    pub params: Vec<(String, Classifier, MetaMultiplicity)>,
    /// The result type; `None` for `void`.
    pub result: Option<(Classifier, MetaMultiplicity)>,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct MetaMultiplicity {
    pub many: bool,
    pub ordered: bool,
    pub unique: bool,
}

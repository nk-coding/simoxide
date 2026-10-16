//! The model store trait: how the engine reads and writes models.

use crate::datum::{Datum, OwnedDatum};
use crate::ids::{ClassId, FeatureId, PackageId, ResourceId};
use crate::meta::{Classifier, MetaRegistry};
use std::fmt;
use std::hash::Hash;

/// A failed model operation. Each variant corresponds to the Java exception EMF throws in the
/// same situation; the engine turns them into `invalid` plus a diagnostic, as Eclipse does.
#[derive(Clone, Debug, PartialEq)]
pub enum ModelError {
    /// The feature is not a feature of the object's class.
    NoSuchFeature { class: ClassId, feature: FeatureId },
    /// A single-valued operation on a many-valued feature or the reverse.
    WrongMultiplicity(FeatureId),
    /// `ClassCastException`: the value does not conform to the feature's type.
    TypeMismatch { feature: FeatureId, found: String },
    /// `IndexOutOfBoundsException`.
    IndexOutOfBounds { index: usize, size: usize },
    /// `IllegalArgumentException("The 'no duplicates' constraint is violated")`.
    Duplicate(FeatureId),
    /// `IllegalArgumentException("The 'no null' constraint is violated")`.
    NullNotAllowed(FeatureId),
    /// Instantiating an abstract class or an interface.
    Abstract(ClassId),
    /// `IllegalArgumentException("Recursive containment not allowed for ...")`.
    RecursiveContainment,
    /// `IllegalArgumentException("The feature '...' is not a valid changeable feature")`: `set`
    /// or `unset` of a feature that is not changeable (list operations do not check).
    NotChangeable(FeatureId),
    /// The operation is not supported by this store or on this feature.
    Unsupported(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::NoSuchFeature { class, feature } => {
                write!(f, "feature {feature:?} is not a feature of class {class:?}")
            }
            ModelError::WrongMultiplicity(x) => write!(f, "wrong multiplicity for feature {x:?}"),
            ModelError::TypeMismatch { feature, found } => {
                write!(
                    f,
                    "value of type {found} does not conform to feature {feature:?}"
                )
            }
            ModelError::IndexOutOfBounds { index, size } => {
                write!(f, "index={index}, size={size}")
            }
            ModelError::Duplicate(_) => f.write_str("The 'no duplicates' constraint is violated"),
            ModelError::NullNotAllowed(_) => f.write_str("The 'no null' constraint is violated"),
            ModelError::Abstract(c) => write!(f, "class {c:?} is abstract"),
            ModelError::RecursiveContainment => f.write_str("Recursive containment not allowed"),
            ModelError::NotChangeable(x) => {
                write!(f, "The feature {x:?} is not a valid changeable feature")
            }
            ModelError::Unsupported(m) => f.write_str(m),
        }
    }
}
impl std::error::Error for ModelError {}

pub type ModelResult<T> = Result<T, ModelError>;

/// A store of model objects: all resources (models) of one run, with EMF's semantics.
///
/// # Contract
///
/// The semantics are those of dynamic EMF objects (`DynamicEObjectImpl` with the setting
/// delegates of `EStructuralFeatureImpl` and the lists of `EcoreEList.Dynamic`). Precisely:
///
/// - **Reading.** [`get`](ModelStore::get) of a single-valued feature returns the effective
///   value: the stored value, else the feature's default (`defaultValueLiteral`, else the type's
///   intrinsic default: `0`, `false`, `0.0`, the first enum literal for enumerations, `null` for
///   other types). A container reference (one whose opposite is a containment) reads the
///   container. Many-valued features are read with [`len`](ModelStore::len) and
///   [`get_at`](ModelStore::get_at); `get` of a many-valued feature returns `Null`.
/// - **`is_set`.** Many-valued: the list is not empty (unsettable features: it was modified
///   since the last unset). Single-valued data, not unsettable: the value differs from the
///   default. Unsettable: set and not unset since. References: not null (unsettable: set).
/// - **Uniqueness.** `add` (append) to a unique list that already contains the value is a no-op
///   returning `Ok(false)`; inserting at an index, or `set_at`, with a value that is elsewhere
///   in the list fails with [`ModelError::Duplicate`]. Reference lists cannot contain `null`.
///   Uniqueness compares objects by identity and data values with Java `equals`.
/// - **Containment.** Adding an object to a containment feature removes it from its previous
///   container (and from the root list of its resource, unless the containment feature resolves
///   proxies: then it stays a root, as EMF's cross-resource containment). Removing it from a
///   containment feature (remove, clear, unset, set to another value) detaches it.
/// - **Opposites.** Changing one end of a bidirectional reference updates the other end; a
///   single-valued end that gets a new value drops the old link on both sides.
/// - **Roots.** [`add_root`](ModelStore::add_root) detaches the object from a container (unless
///   that containment resolves proxies) and from another resource.
/// - **Changeability.** `set` and `unset` of a feature that is not changeable fail (EMF's
///   reflective `eSet`/`eUnset`); list operations do not check.
/// - **Delete** follows `EcoreUtil.delete`: remove every changeable reference to the object from
///   objects of the resources (or of its containment tree if it has none), then remove it from its
///   container and from its resource.
///
/// `qvto_model::contract` holds a test suite for these rules that every implementation runs.
pub trait ModelStore {
    /// Object handle.
    type Obj: Copy + Eq + Hash + Ord + fmt::Debug;

    fn meta(&self) -> &MetaRegistry;
    fn class_of(&self, o: Self::Obj) -> ClassId;
    /// An unresolved proxy (an object only known by its URI).
    fn is_proxy(&self, o: Self::Obj) -> bool;
    fn proxy_uri(&self, o: Self::Obj) -> Option<&str>;

    // ---- reading ----
    fn get(&self, o: Self::Obj, f: FeatureId) -> Datum<'_, Self::Obj>;
    fn len(&self, o: Self::Obj, f: FeatureId) -> usize;
    fn get_at(&self, o: Self::Obj, f: FeatureId, i: usize) -> Datum<'_, Self::Obj>;
    fn is_set(&self, o: Self::Obj, f: FeatureId) -> bool;
    /// Container and containment feature.
    fn container(&self, o: Self::Obj) -> Option<(Self::Obj, FeatureId)>;
    /// The resource of the object's root container (`eResource()`).
    fn resource_of(&self, o: Self::Obj) -> Option<ResourceId>;
    /// The resource whose root list holds the object (`eDirectResource()`).
    fn direct_resource(&self, o: Self::Obj) -> Option<ResourceId>;
    fn resource_count(&self) -> usize;
    fn resource_uri(&self, r: ResourceId) -> &str;
    fn roots(&self, r: ResourceId) -> &[Self::Obj];

    // ---- writing ----
    fn create(&mut self, c: ClassId) -> ModelResult<Self::Obj>;
    fn set(&mut self, o: Self::Obj, f: FeatureId, v: Datum<'_, Self::Obj>) -> ModelResult<()>;
    fn unset(&mut self, o: Self::Obj, f: FeatureId) -> ModelResult<()>;
    /// Appends (`at == None`, `EList.add`) or inserts (`EList.add(index, v)`).
    fn add(
        &mut self,
        o: Self::Obj,
        f: FeatureId,
        v: Datum<'_, Self::Obj>,
        at: Option<usize>,
    ) -> ModelResult<bool>;
    /// `EList.remove(Object)`: the first occurrence.
    fn remove(&mut self, o: Self::Obj, f: FeatureId, v: Datum<'_, Self::Obj>) -> ModelResult<bool>;
    /// `EList.remove(int)`.
    fn remove_at(&mut self, o: Self::Obj, f: FeatureId, i: usize) -> ModelResult<()>;
    /// `EList.set(int, Object)`.
    fn set_at(
        &mut self,
        o: Self::Obj,
        f: FeatureId,
        i: usize,
        v: Datum<'_, Self::Obj>,
    ) -> ModelResult<()>;
    fn clear(&mut self, o: Self::Obj, f: FeatureId) -> ModelResult<()>;
    /// `EList.move(to, from)`.
    fn move_to(&mut self, o: Self::Obj, f: FeatureId, to: usize, from: usize) -> ModelResult<()>;
    fn create_resource(&mut self, uri: &str) -> ResourceId;
    /// Adds to the resource's root list (`getContents().add`); `Ok(false)` if already a root.
    fn add_root(&mut self, r: ResourceId, o: Self::Obj, at: Option<usize>) -> ModelResult<bool>;
    fn remove_root(&mut self, r: ResourceId, o: Self::Obj) -> bool;
    /// `EcoreUtil.delete(o)`.
    fn delete(&mut self, o: Self::Obj);

    // ---- metamodel objects (optional) ----

    /// The object of a classifier (an `EClass`, `EDataType` or `EEnum` instance) if the store
    /// keeps its metamodels as objects, for `eClass()` and reflective Ecore navigation.
    fn classifier_object(&self, c: Classifier) -> Option<Self::Obj> {
        let _ = c;
        None
    }
    /// The classifier an object represents, if it is a metamodel object of the registry.
    fn object_classifier(&self, o: Self::Obj) -> Option<Classifier> {
        let _ = o;
        None
    }
    /// The object of a structural feature (`EAttribute`/`EReference` instance).
    fn feature_object(&self, f: FeatureId) -> Option<Self::Obj> {
        let _ = f;
        None
    }
    /// The feature an object represents.
    fn object_feature(&self, o: Self::Obj) -> Option<FeatureId> {
        let _ = o;
        None
    }
    /// The object of a package (`EPackage` instance).
    fn package_object(&self, p: PackageId) -> Option<Self::Obj> {
        let _ = p;
        None
    }
    /// The package an object represents.
    fn object_package(&self, o: Self::Obj) -> Option<PackageId> {
        let _ = o;
        None
    }

    /// Grows the store's registry to `meta`, which must [extend](MetaRegistry::extends) it:
    /// the classes a program adds to the metamodels it was compiled against (QVTo's intermediate
    /// classes, whose instances Eclipse keeps as dynamic EMF objects). `false` if the store
    /// cannot (a store with fixed metamodels) or `meta` does not extend its registry; the engine
    /// then keeps such objects itself, without EMF semantics.
    fn extend_meta(&mut self, meta: &MetaRegistry) -> bool {
        let _ = meta;
        false
    }

    // ---- provided ----

    /// `ECollections.setEList(list, values)`: makes the list equal to `values` with EMF's
    /// sequence of moves, inserts and removals (so containment and opposites see the same
    /// changes as in EMF). On an error the list may be partially updated; QVTo then falls back to
    /// clear and add all.
    fn set_list(
        &mut self,
        o: Self::Obj,
        f: FeatureId,
        values: &[Datum<'_, Self::Obj>],
    ) -> ModelResult<()> {
        let protos: Vec<OwnedDatum<Self::Obj>> = values.iter().map(|v| v.to_owned()).collect();
        let index_in_list = |s: &Self, x: &OwnedDatum<Self::Obj>, from: usize| -> Option<usize> {
            (from..s.len(o, f)).find(|&i| s.get_at(o, f, i).java_equals(&x.as_datum()))
        };
        let index_in_protos = |x: &OwnedDatum<Self::Obj>, from: usize| -> Option<usize> {
            protos
                .iter()
                .enumerate()
                .skip(from)
                .find(|(_, p)| p.as_datum().java_equals(&x.as_datum()))
                .map(|(i, _)| i)
        };
        let mut index = 0;
        for proto in &protos {
            if self.len(o, f) <= index {
                self.add(o, f, proto.as_datum(), None)?;
            } else {
                loop {
                    let mut done = true;
                    let target = self.get_at(o, f, index).to_owned();
                    if !target.as_datum().java_equals(&proto.as_datum()) {
                        match index_in_list(self, proto, index) {
                            Some(position) => match index_in_protos(&target, index) {
                                None => {
                                    self.remove_at(o, f, index)?;
                                    done = false;
                                }
                                Some(mut t) if t > position => {
                                    let len = self.len(o, f);
                                    if len <= t {
                                        t = len - 1;
                                    }
                                    self.move_to(o, f, t, index)?;
                                    done = false;
                                }
                                Some(_) => self.move_to(o, f, index, position)?,
                            },
                            None => {
                                self.add(o, f, proto.as_datum(), Some(index))?;
                            }
                        }
                    }
                    if done {
                        break;
                    }
                }
            }
            index += 1;
        }
        let mut i = self.len(o, f);
        while i > index {
            i -= 1;
            self.remove_at(o, f, i)?;
        }
        Ok(())
    }

    /// `eContents()`: the objects contained by `o`, in containment feature order.
    fn contents(&self, o: Self::Obj) -> Vec<Self::Obj> {
        let meta = self.meta();
        let mut out = Vec::new();
        for &f in meta.class(self.class_of(o)).containments.iter() {
            if meta.feature(f).is_many() {
                for i in 0..self.len(o, f) {
                    if let Datum::Obj(c) = self.get_at(o, f, i) {
                        out.push(c);
                    }
                }
            } else if let Datum::Obj(c) = self.get(o, f) {
                out.push(c);
            }
        }
        out
    }

    /// `eAllContents()` (pre-order, without `o` itself).
    fn all_contents(&self, o: Self::Obj) -> Vec<Self::Obj> {
        let mut out = Vec::new();
        let mut stack: Vec<Self::Obj> = self.contents(o).into_iter().rev().collect();
        while let Some(x) = stack.pop() {
            out.push(x);
            stack.extend(self.contents(x).into_iter().rev());
        }
        out
    }

    /// The objects of a resource in `getAllContents` order (roots, each followed by its tree).
    fn resource_contents(&self, r: ResourceId) -> Vec<Self::Obj> {
        let mut out = Vec::new();
        for &root in self.roots(r) {
            out.push(root);
            out.extend(self.all_contents(root));
        }
        out
    }
}

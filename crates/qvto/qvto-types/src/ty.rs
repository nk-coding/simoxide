//! Types: an interning table of OCL and QVTo types.
//!
//! Every type the analyser or the program uses is a [`TypeId`] into one [`TypeTable`]. Equal
//! structures intern to the same id (`Set(String)` is one id), so type equality is id equality,
//! as Eclipse's type caches make `Set(String) == Set(String)` an identity in classic OCL.
//!
//! Model classifiers are referenced by their `qvto_model` ids (`ClassId`, `DataTypeId`,
//! `EnumId`) in the program's `MetaRegistry`. QVTo's standard library classes (`Element`,
//! `Object`, `Exception`, `AssertionFailed`, `Model`, `Status`, `Transformation`) and the
//! intermediate classes and exceptions of the program are classes of that registry too (see
//! [`crate::StdClasses`]), so instances of all of them are ordinary model objects.

use qvto_model::{ClassId, DataTypeId, EnumId};
use std::collections::HashMap;
use std::fmt;

/// An interned type.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct TypeId(pub u32);

impl TypeId {
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// The OCL collection kinds. `Collection` is the abstract kind (no runtime collection has it).
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum CollKind {
    Collection,
    Set,
    OrderedSet,
    Bag,
    Sequence,
}

impl CollKind {
    pub fn name(self) -> &'static str {
        match self {
            CollKind::Collection => "Collection",
            CollKind::Set => "Set",
            CollKind::OrderedSet => "OrderedSet",
            CollKind::Bag => "Bag",
            CollKind::Sequence => "Sequence",
        }
    }
    pub fn is_unique(self) -> bool {
        matches!(self, CollKind::Set | CollKind::OrderedSet)
    }
    pub fn is_ordered(self) -> bool {
        matches!(self, CollKind::OrderedSet | CollKind::Sequence)
    }
    /// The kind of a feature's values by its `ordered` and `unique` flags (classic OCL's
    /// `UMLReflectionImpl.getOCLCollectionKind`).
    pub fn of_feature(ordered: bool, unique: bool) -> CollKind {
        match (ordered, unique) {
            (true, true) => CollKind::OrderedSet,
            (true, false) => CollKind::Sequence,
            (false, true) => CollKind::Set,
            (false, false) => CollKind::Bag,
        }
    }
}

/// Template parameters of standard library signatures. They never appear in the types of
/// analysed expressions.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum TParam {
    /// `T`: the element type of the receiver collection (or the receiver type for `OclAny`
    /// operations such as `oclAsSet`).
    T,
    /// `T2`: the argument's type parameter (`product`, `oclAsType`, ...).
    T2,
    /// `KeyT` of `Dict(KeyT, T)`.
    KeyT,
    /// QVTo's `CommonT`.
    CommonT,
}

/// The structure of a type.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum Type {
    // ---- OCL standard library ---------------------------------------------------------------
    OclAny,
    /// The type of `null`; conforms to everything.
    OclVoid,
    /// The type of `invalid`; conforms to everything.
    OclInvalid,
    Boolean,
    Integer,
    Real,
    String,
    UnlimitedNatural,
    /// Classic OCL's `OclType` (the metatype of type expressions in some signatures).
    OclType,
    /// `OclElement` of classic OCL (unused by Ecore models; kept for completeness).
    OclElement,
    /// `OclMessage` and `State` of classic OCL (signatures only; Ecore models have none).
    OclMessage,
    State,
    /// `Set(T)`, `OrderedSet(T)`, `Bag(T)`, `Sequence(T)`, `Collection(T)`.
    Collection(CollKind, TypeId),
    /// `Tuple(a : A, b : B)`; parts in declaration order (classic OCL's tuple types are found
    /// by part names and types regardless of order; [`TypeTable::tuple`] sorts nothing, but
    /// [`TypeTable::find_tuple`] looks up order-insensitively).
    Tuple(Box<[(Box<str>, TypeId)]>),
    /// The type of a type literal expression (`TypeExp`): classic OCL's `TypeType` referring to
    /// the given type.
    TypeType(TypeId),
    // ---- QVTo -------------------------------------------------------------------------------
    /// `List(T)`
    List(TypeId),
    /// `Dict(K, V)`
    Dict(TypeId, TypeId),
    /// A module (transformation or library) used as a type: the type of `this` and of
    /// `new T(...)` for a transformation `T`. The index is the program's module index
    /// (`qvto_sema::ir::ModuleId`).
    Module(u32),
    /// A model type declared by `modeltype M uses ...`: the type of model parameters. The index
    /// is the program's model type index (`qvto_sema::ir::ModelTypeId`). Conforms to the stdlib
    /// class `Model`.
    ModelType(u32),
    // ---- model classifiers ------------------------------------------------------------------
    Class(ClassId),
    DataType(DataTypeId),
    Enum(EnumId),
    // ---- signatures only --------------------------------------------------------------------
    Param(TParam),
}

/// The interning table.
#[derive(Clone, Debug)]
pub struct TypeTable {
    types: Vec<Type>,
    index: HashMap<Type, TypeId>,
}

macro_rules! fixed {
    ($($name:ident = $idx:expr => $ty:expr;)*) => {
        impl TypeId {
            $(pub const $name: TypeId = TypeId($idx);)*
        }
        const FIXED: &[Type] = &[$($ty),*];
    };
}

fixed! {
    OCL_ANY = 0 => Type::OclAny;
    OCL_VOID = 1 => Type::OclVoid;
    OCL_INVALID = 2 => Type::OclInvalid;
    BOOLEAN = 3 => Type::Boolean;
    INTEGER = 4 => Type::Integer;
    REAL = 5 => Type::Real;
    STRING = 6 => Type::String;
    UNLIMITED_NATURAL = 7 => Type::UnlimitedNatural;
    OCL_TYPE = 8 => Type::OclType;
    OCL_ELEMENT = 9 => Type::OclElement;
    T = 10 => Type::Param(TParam::T);
    T2 = 11 => Type::Param(TParam::T2);
    KEY_T = 12 => Type::Param(TParam::KeyT);
    COMMON_T = 13 => Type::Param(TParam::CommonT);
}

impl Default for TypeTable {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeTable {
    pub fn new() -> TypeTable {
        let mut t = TypeTable {
            types: Vec::new(),
            index: HashMap::new(),
        };
        for ty in FIXED {
            t.intern(ty.clone());
        }
        t
    }

    pub fn intern(&mut self, ty: Type) -> TypeId {
        if let Some(&id) = self.index.get(&ty) {
            return id;
        }
        let id = TypeId(self.types.len() as u32);
        self.types.push(ty.clone());
        self.index.insert(ty, id);
        id
    }

    /// The id of a type if it is interned.
    pub fn find(&self, ty: &Type) -> Option<TypeId> {
        self.index.get(ty).copied()
    }

    #[inline]
    pub fn get(&self, id: TypeId) -> &Type {
        &self.types[id.index()]
    }

    pub fn len(&self) -> usize {
        self.types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    pub fn collection(&mut self, kind: CollKind, elem: TypeId) -> TypeId {
        self.intern(Type::Collection(kind, elem))
    }
    pub fn list(&mut self, elem: TypeId) -> TypeId {
        self.intern(Type::List(elem))
    }
    pub fn dict(&mut self, key: TypeId, value: TypeId) -> TypeId {
        self.intern(Type::Dict(key, value))
    }
    pub fn class(&mut self, c: ClassId) -> TypeId {
        self.intern(Type::Class(c))
    }
    pub fn type_type(&mut self, t: TypeId) -> TypeId {
        self.intern(Type::TypeType(t))
    }

    /// A tuple type with the given parts. An existing tuple type with the same parts in any
    /// order is reused (classic OCL's `AbstractTypeResolver.findTupleType`).
    pub fn tuple(&mut self, parts: Vec<(Box<str>, TypeId)>) -> TypeId {
        if let Some(id) = self.find_tuple(&parts) {
            return id;
        }
        self.intern(Type::Tuple(parts.into_boxed_slice()))
    }

    /// The tuple type with these parts in any order, if one exists.
    pub fn find_tuple(&self, parts: &[(Box<str>, TypeId)]) -> Option<TypeId> {
        if let Some(id) = self.find(&Type::Tuple(parts.to_vec().into_boxed_slice())) {
            return Some(id);
        }
        self.types.iter().enumerate().find_map(|(i, t)| match t {
            Type::Tuple(ps)
                if ps.len() == parts.len() && parts.iter().all(|p| ps.contains(p)) =>
            {
                Some(TypeId(i as u32))
            }
            _ => None,
        })
    }

    /// Element type of a collection or `List` type (`Dict(K, V)`: `V`).
    pub fn element(&self, id: TypeId) -> Option<TypeId> {
        match *self.get(id) {
            Type::Collection(_, e) | Type::List(e) => Some(e),
            Type::Dict(_, v) => Some(v),
            _ => None,
        }
    }

    /// The collection kind of an OCL collection type.
    pub fn coll_kind(&self, id: TypeId) -> Option<CollKind> {
        match *self.get(id) {
            Type::Collection(k, _) => Some(k),
            _ => None,
        }
    }

    pub fn is_collection(&self, id: TypeId) -> bool {
        matches!(self.get(id), Type::Collection(..))
    }

    /// OCL collection, `List` or `Dict`.
    pub fn is_collection_like(&self, id: TypeId) -> bool {
        matches!(
            self.get(id),
            Type::Collection(..) | Type::List(_) | Type::Dict(..)
        )
    }
}

impl fmt::Display for TypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "t{}", self.0)
    }
}

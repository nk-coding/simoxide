//! Type relationships, conformance, common supertypes and type names, as Eclipse QVTo computes
//! them: a port of QVTo's `TypeCheckerImpl.getRelationship` (List, Dict and tuple rules) on top
//! of classic OCL's `AbstractTypeChecker` (`getRelationship`, `commonSuperType`), with
//! `QVTUMLReflection` (model classes conform to the library class `Element`) and Ecore's
//! `UMLReflectionImpl` (class hierarchy, data types mapped to OCL types).
//!
//! Template parameters `T` and `T2` behave like `OclAny` (they are `AnyType` instances in
//! classic OCL).

use crate::stdlib::StdClasses;
use crate::ty::{CollKind, TParam, Type, TypeId, TypeTable};
use qvto_model::{ClassId, DataKind, DataTypeId, MetaRegistry};

/// `UMLReflection.SAME_TYPE` etc.
pub mod rel {
    pub const SAME_TYPE: u8 = 1;
    pub const STRICT_SUBTYPE: u8 = 2;
    pub const SUBTYPE: u8 = SAME_TYPE | STRICT_SUBTYPE;
    pub const STRICT_SUPERTYPE: u8 = 4;
    pub const SUPERTYPE: u8 = SAME_TYPE | STRICT_SUPERTYPE;
    pub const RELATED_TYPE: u8 = SAME_TYPE | STRICT_SUBTYPE | STRICT_SUPERTYPE;
    pub const UNRELATED_TYPE: u8 = 8;
}
use rel::*;

/// What the type checker needs to know about modules and model types (QVTo `Module` and
/// `ModelType` are classes in Eclipse).
#[derive(Clone, Debug, Default)]
pub struct ModuleTypeInfo {
    pub name: String,
    /// A transformation (subtype of the library class `Transformation`) rather than a library.
    pub transformation: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ModelTypeInfo {
    pub name: String,
    /// The module whose package contains the model type (for qualified names).
    pub module: u32,
}

/// A failed `commonSuperType` (classic OCL reports these through the problem handler).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuperTypeError {
    /// `TypeMismatch_ERROR_`
    TypeMismatch(String, String),
    /// `TupleTypeMismatch_ERROR_`
    TupleTypeMismatch(String, String),
    /// `TupleFieldNumMismatch_ERROR_`
    TupleFieldNumMismatch(String, String),
    /// `TupleFieldNotFound_ERROR_` (type1, field, type2)
    TupleFieldNotFound(String, String, String),
}

/// A view of the types of one compilation for checking: the table, the registry and the
/// module/model-type facts.
pub struct Checker<'a> {
    pub types: &'a mut TypeTable,
    pub meta: &'a MetaRegistry,
    pub std: &'a StdClasses,
    pub modules: &'a [ModuleTypeInfo],
    pub model_types: &'a [ModelTypeInfo],
}

impl<'a> Checker<'a> {
    #[inline]
    fn ty(&self, t: TypeId) -> &Type {
        self.types.get(t)
    }

    fn is_any(&self, t: TypeId) -> bool {
        matches!(
            self.ty(t),
            Type::OclAny | Type::Param(TParam::T) | Type::Param(TParam::T2)
        )
    }

    fn is_void_or_invalid(&self, t: TypeId) -> bool {
        t == TypeId::OCL_VOID || t == TypeId::OCL_INVALID
    }

    /// `instanceof PrimitiveType`
    pub fn is_primitive(&self, t: TypeId) -> bool {
        matches!(
            self.ty(t),
            Type::Boolean | Type::Integer | Type::Real | Type::String | Type::UnlimitedNatural
        )
    }

    /// `instanceof PredefinedType` (the classic OCL library types).
    pub fn is_predefined(&self, t: TypeId) -> bool {
        matches!(
            self.ty(t),
            Type::OclAny
                | Type::OclVoid
                | Type::OclInvalid
                | Type::Boolean
                | Type::Integer
                | Type::Real
                | Type::String
                | Type::UnlimitedNatural
                | Type::OclElement
                | Type::OclMessage
                | Type::State
                | Type::Collection(..)
                | Type::Tuple(_)
                | Type::TypeType(_)
                | Type::List(_)
                | Type::Dict(..)
                | Type::Param(TParam::T | TParam::T2)
        )
    }

    /// OCL collection, `List` or `Dict` (Eclipse's `CollectionType`).
    pub fn is_collection_like(&self, t: TypeId) -> bool {
        matches!(
            self.ty(t),
            Type::Collection(..) | Type::List(_) | Type::Dict(..)
        )
    }

    /// The OCL type of a data type (`UMLReflectionImpl.getOCLTypeFor`): primitive Java classes
    /// map to the OCL primitive types, `java.lang.Object` to `OclAny`, Java collections to the
    /// generic collection types; others (enums, `EChar`, `EDate`, data types without a
    /// loadable class) stay themselves.
    pub fn data_type_ocl(&mut self, d: DataTypeId) -> TypeId {
        let dt = self.meta.data_type(d);
        let t = match dt.kind {
            DataKind::Boolean => Some(TypeId::BOOLEAN),
            DataKind::Double | DataKind::Float | DataKind::BigDecimal => Some(TypeId::REAL),
            DataKind::String => Some(TypeId::STRING),
            DataKind::Int
            | DataKind::Long
            | DataKind::Short
            | DataKind::Byte
            | DataKind::BigInteger => Some(TypeId::INTEGER),
            _ => match dt.instance_class_name.as_deref() {
                Some("java.lang.Object") => Some(TypeId::OCL_ANY),
                Some(
                    "java.util.List"
                    | "java.util.ArrayList"
                    | "java.util.LinkedList"
                    | "org.eclipse.emf.common.util.EList"
                    | "org.eclipse.emf.common.util.BasicEList",
                ) => Some(self.types.collection(CollKind::Sequence, TypeId::T)),
                Some("java.util.Set" | "java.util.HashSet" | "java.util.LinkedHashSet") => {
                    Some(self.types.collection(CollKind::Set, TypeId::T))
                }
                Some("java.util.Collection") => {
                    Some(self.types.collection(CollKind::Collection, TypeId::T))
                }
                _ => None,
            },
        };
        t.unwrap_or_else(|| self.types.intern(Type::DataType(d)))
    }

    /// `uml.getOCLType(type)` for a type: data types become OCL types.
    pub fn ocl_type(&mut self, t: TypeId) -> TypeId {
        match *self.ty(t) {
            Type::DataType(d) => self.data_type_ocl(d),
            _ => t,
        }
    }

    /// Whether a class is a user model element (`QVTUMLReflection.isUserModelElement`): a
    /// metamodel class or an intermediate class, not a QVTo library class.
    pub fn is_user_model_element(&self, t: TypeId) -> bool {
        match *self.ty(t) {
            // intermediate classes live in their module's package, so they count too
            Type::Class(c) => self.meta.class(c).package != self.std.package,
            // ocl.ecore's TupleType is an EClass of a non-library metaclass
            Type::Tuple(_) => true,
            _ => false,
        }
    }

    /// The class of a class-like type for the class hierarchy (modules and model types are
    /// classes in Eclipse).
    fn class_supers(&self, t: TypeId) -> Option<ClassLike> {
        match *self.ty(t) {
            Type::Class(c) => Some(ClassLike::Class(c)),
            Type::Module(m) => Some(ClassLike::Module(m)),
            Type::ModelType(m) => Some(ClassLike::ModelType(m)),
            _ => None,
        }
    }

    /// `EClass.isSuperTypeOf` between class-like types.
    fn is_super_type_of(&self, sup: ClassLike, sub: ClassLike) -> bool {
        if sup == sub {
            return true;
        }
        match (sup, sub) {
            (ClassLike::Class(a), ClassLike::Class(b)) => {
                // EMF: EObject is not a supertype unless declared; MetaRegistry::is_subclass
                // treats every class as a subclass of EObject, which EMF's isSuperTypeOf does
                // not.
                b == a || self.meta.class(b).all_supers.contains(&a)
            }
            (ClassLike::Class(a), ClassLike::Module(m)) => {
                self.modules
                    .get(m as usize)
                    .is_some_and(|mi| mi.transformation)
                    && a == self.std.transformation
            }
            (ClassLike::Class(a), ClassLike::ModelType(_)) => a == self.std.model,
            _ => false,
        }
    }

    /// `UMLReflectionImpl.getRelationship` with `QVTUMLReflection`'s `Element` rule.
    fn uml_relationship(&self, t1: TypeId, t2: TypeId) -> u8 {
        if t1 == t2 {
            return SAME_TYPE;
        }
        let mut result = UNRELATED_TYPE;
        if let (Some(a), Some(b)) = (self.class_supers(t1), self.class_supers(t2)) {
            if self.is_super_type_of(a, b) {
                result = STRICT_SUPERTYPE;
            } else if self.is_super_type_of(b, a) {
                result = STRICT_SUBTYPE;
            }
        }
        let element = Type::Class(self.std.element);
        if *self.ty(t1) == element && self.is_user_model_element(t2) {
            return STRICT_SUPERTYPE;
        }
        if *self.ty(t2) == element && self.is_user_model_element(t1) {
            return STRICT_SUBTYPE;
        }
        result
    }

    /// `TypeCheckerImpl.getRelationship`.
    pub fn relationship(&mut self, t1: TypeId, t2: TypeId) -> u8 {
        if t1 == t2 {
            return SAME_TYPE;
        }
        if t1 == TypeId::NULL {
            return UNRELATED_TYPE;
        }
        if t2 == TypeId::NULL {
            // AbstractTypeChecker: OclVoid and OclInvalid conform to anything, even to no type
            let o1 = self.ocl_type(t1);
            return if o1 == TypeId::OCL_VOID || o1 == TypeId::OCL_INVALID {
                STRICT_SUBTYPE
            } else {
                UNRELATED_TYPE
            };
        }
        if self.is_any(t1) {
            return if self.is_any(t2) {
                SAME_TYPE
            } else {
                STRICT_SUPERTYPE
            };
        } else if self.is_any(t2) {
            return STRICT_SUBTYPE;
        }
        let list1 = matches!(self.ty(t1), Type::List(_));
        let list2 = matches!(self.ty(t2), Type::List(_));
        if list1 || list2 {
            if let (Type::List(e1), Type::List(e2)) = (self.ty(t1).clone(), self.ty(t2).clone()) {
                return self.relationship(e1, e2);
            }
            if !list1 {
                if let Type::Collection(k, ce) = *self.ty(t1) {
                    let Type::List(le) = *self.ty(t2) else {
                        unreachable!()
                    };
                    if k == CollKind::Collection && self.compatible(le, ce) {
                        return STRICT_SUPERTYPE;
                    }
                    if k == CollKind::Sequence && self.compatible(le, ce) {
                        return SUPERTYPE;
                    }
                }
            } else if !list2 && let Type::Collection(k, ce) = *self.ty(t2) {
                let Type::List(le) = *self.ty(t1) else {
                    unreachable!()
                };
                if k == CollKind::Collection && self.compatible(le, ce) {
                    return STRICT_SUBTYPE;
                }
                if k == CollKind::Sequence && self.compatible(le, ce) {
                    return SUBTYPE;
                }
            }
            if self.is_void_or_invalid(t1) || self.is_void_or_invalid(t2) {
                return self.ocl_relationship(t1, t2);
            }
            return UNRELATED_TYPE;
        }
        let dict1 = matches!(self.ty(t1), Type::Dict(..));
        let dict2 = matches!(self.ty(t2), Type::Dict(..));
        if dict1 || dict2 {
            if let (Type::Dict(k1, v1), Type::Dict(k2, v2)) =
                (self.ty(t1).clone(), self.ty(t2).clone())
            {
                let elem = self.relationship(v1, v2);
                let key = self.relationship(k1, k2);
                if elem != key {
                    if key & SUBTYPE != 0 && elem & SUBTYPE != 0 {
                        return STRICT_SUBTYPE;
                    } else if key & SUPERTYPE != 0 && elem & SUPERTYPE != 0 {
                        return STRICT_SUPERTYPE;
                    }
                    return elem;
                }
                return elem;
            } else if !dict1 {
                if let Type::Collection(CollKind::Collection, ce) = *self.ty(t1) {
                    let Type::Dict(_, v) = *self.ty(t2) else {
                        unreachable!()
                    };
                    if self.compatible(v, ce) {
                        return STRICT_SUPERTYPE;
                    }
                }
                if self.is_void_or_invalid(t1) {
                    return self.ocl_relationship(t1, t2);
                }
                return UNRELATED_TYPE;
            } else {
                // Eclipse casts type1 here (a bug): `col2` is the dictionary itself, so the
                // test compares the dictionary's value type with its own element type.
                if let Type::Collection(CollKind::Collection, _) = *self.ty(t2) {
                    let Type::Dict(_, v) = *self.ty(t1) else {
                        unreachable!()
                    };
                    if self.compatible(v, v) {
                        return STRICT_SUBTYPE;
                    }
                }
                if self.is_void_or_invalid(t2) {
                    return self.ocl_relationship(t1, t2);
                }
                return UNRELATED_TYPE;
            }
        }
        if let (Type::Tuple(p1), Type::Tuple(p2)) = (self.ty(t1).clone(), self.ty(t2).clone()) {
            let mut current = 0u8;
            let mut matching = 0usize;
            for (name, ty1) in p1.iter() {
                let Some((_, ty2)) = p2.iter().find(|(n, _)| n == name) else {
                    return UNRELATED_TYPE;
                };
                let part = self.relationship(*ty1, *ty2);
                if part == SAME_TYPE {
                } else if part == RELATED_TYPE {
                    current = RELATED_TYPE;
                } else if part & STRICT_SUBTYPE != 0 {
                    if current & STRICT_SUPERTYPE != 0 {
                        current = RELATED_TYPE;
                    } else {
                        current |= part;
                    }
                } else if part & STRICT_SUPERTYPE != 0 {
                    if current & STRICT_SUBTYPE != 0 {
                        current = RELATED_TYPE;
                    } else {
                        current |= part;
                    }
                } else {
                    return UNRELATED_TYPE;
                }
                matching += 1;
            }
            if matching < p1.len() || p1.len() != p2.len() {
                return UNRELATED_TYPE;
            }
            return if current == 0 { SAME_TYPE } else { current };
        }
        self.ocl_relationship(t1, t2)
    }

    /// Classic OCL's `AbstractTypeChecker.getRelationship` (after QVTo's special cases).
    fn ocl_relationship(&mut self, t1: TypeId, t2: TypeId) -> u8 {
        let t1 = self.ocl_type(t1);
        let t2 = self.ocl_type(t2);
        if t1 == t2 {
            return SAME_TYPE;
        }
        if t1 == TypeId::OCL_VOID && t2 != TypeId::OCL_INVALID {
            return STRICT_SUBTYPE;
        } else if t2 == TypeId::OCL_VOID && t1 != TypeId::OCL_INVALID {
            return STRICT_SUPERTYPE;
        }
        if t1 == TypeId::OCL_INVALID {
            return STRICT_SUBTYPE;
        } else if t2 == TypeId::OCL_INVALID {
            return STRICT_SUPERTYPE;
        }
        if self.is_any(t1) {
            return if self.is_any(t2) {
                SAME_TYPE
            } else if self.is_collection_like(t2) {
                UNRELATED_TYPE
            } else {
                STRICT_SUPERTYPE
            };
        } else if self.is_any(t2) {
            return if self.is_collection_like(t1) {
                UNRELATED_TYPE
            } else {
                STRICT_SUBTYPE
            };
        }
        if self.is_primitive(t1) {
            let order = |t: TypeId| match t {
                TypeId::UNLIMITED_NATURAL => 0,
                TypeId::INTEGER => 1,
                TypeId::REAL => 2,
                _ => -1,
            };
            let (o1, o2) = (order(t1), order(t2));
            if o1 >= 0 && o2 >= 0 {
                if o1 < o2 {
                    return STRICT_SUBTYPE;
                } else if o1 > o2 {
                    return STRICT_SUPERTYPE;
                }
            }
            return UNRELATED_TYPE;
        } else if self.is_primitive(t2) {
            return UNRELATED_TYPE;
        }
        if self.is_collection_like(t1) {
            if !self.is_collection_like(t2) {
                return UNRELATED_TYPE;
            }
            let (k1, e1) = self.coll_parts(t1);
            let (k2, e2) = self.coll_parts(t2);
            let kind = kind_relationship(k1, k2);
            if kind == UNRELATED_TYPE {
                return kind;
            }
            let elem = self.relationship(e1, e2);
            return match kind {
                SAME_TYPE => elem,
                STRICT_SUBTYPE => match elem {
                    SAME_TYPE | STRICT_SUBTYPE => STRICT_SUBTYPE,
                    _ => UNRELATED_TYPE,
                },
                STRICT_SUPERTYPE => match elem {
                    SAME_TYPE | STRICT_SUPERTYPE => STRICT_SUPERTYPE,
                    _ => UNRELATED_TYPE,
                },
                _ => UNRELATED_TYPE,
            };
        }
        if matches!(self.ty(t1), Type::TypeType(_)) && t2 == TypeId::OCL_TYPE {
            return STRICT_SUBTYPE;
        } else if matches!(self.ty(t2), Type::TypeType(_)) && t1 == TypeId::OCL_TYPE {
            return STRICT_SUPERTYPE;
        }
        let tup1 = matches!(self.ty(t1), Type::Tuple(_));
        let tup2 = matches!(self.ty(t2), Type::Tuple(_));
        if tup1 || tup2 {
            if !(tup1 && tup2) {
                return UNRELATED_TYPE;
            }
            // both tuples: handled by QVTo's rule before; classic OCL compares names only
            let (Type::Tuple(p1), Type::Tuple(p2)) = (self.ty(t1).clone(), self.ty(t2).clone())
            else {
                unreachable!()
            };
            if p1.len() != p2.len() {
                return UNRELATED_TYPE;
            }
            for (n, _) in p1.iter() {
                if !p2.iter().any(|(m, _)| m == n) {
                    return UNRELATED_TYPE;
                }
            }
            return SAME_TYPE;
        }
        if self.is_predefined(t1) || self.is_predefined(t2) {
            return UNRELATED_TYPE;
        }
        self.uml_relationship(t1, t2)
    }

    /// Kind and element type of an OCL collection, `List` or `Dict`. Eclipse's `ListType` and
    /// `DictionaryType` extend the generic `CollectionTypeImpl`, whose kind is `Collection`.
    fn coll_parts(&self, t: TypeId) -> (CollKind, TypeId) {
        match *self.ty(t) {
            Type::Collection(k, e) => (k, e),
            Type::List(e) => (CollKind::Collection, e),
            Type::Dict(_, v) => (CollKind::Collection, v),
            _ => (CollKind::Collection, TypeId::OCL_VOID),
        }
    }

    /// `compatibleTypeMatch` (QVTo's: same, strict subtype or subtype).
    pub fn compatible(&mut self, t1: TypeId, t2: TypeId) -> bool {
        matches!(
            self.relationship(t1, t2),
            SAME_TYPE | STRICT_SUBTYPE | SUBTYPE
        )
    }

    /// `exactTypeMatch`.
    pub fn exact(&mut self, t1: TypeId, t2: TypeId) -> bool {
        self.relationship(t1, t2) == SAME_TYPE
    }

    /// The Java instance class of an `EDataType`-like type (OCL primitives are `EDataType`s with
    /// a Java class too).
    fn instance_class(&self, t: TypeId) -> Option<&str> {
        match self.ty(t) {
            Type::Boolean => Some("java.lang.Boolean"),
            Type::Integer | Type::UnlimitedNatural => Some("java.lang.Integer"),
            Type::Real => Some("java.lang.Double"),
            Type::String => Some("java.lang.String"),
            Type::DataType(d) => self.meta.data_type(*d).instance_class_name.as_deref(),
            _ => None,
        }
    }

    fn is_edatatype(&self, t: TypeId) -> bool {
        self.is_primitive(t) || matches!(self.ty(t), Type::DataType(_) | Type::Enum(_))
    }

    /// `QvtOperationalParserUtil.isAssignableToFrom`.
    pub fn assignable(&mut self, var: TypeId, init: TypeId) -> bool {
        if var == TypeId::UNLIMITED_NATURAL && init == TypeId::INTEGER {
            return true;
        }
        if self.is_edatatype(var) && self.is_edatatype(init) && !self.is_predefined(var) {
            let ia = self.instance_class(var);
            if ia.is_some() && ia == self.instance_class(init) {
                return true;
            }
            // `initialiserType == ExtendedMetaData.getBaseType(variableType)`: the registry keeps
            // no annotations; EMF's XSD convention names the nullable wrapper of enum `E` `EObject`
            if let (Type::DataType(d), Type::Enum(e)) =
                (self.ty(var).clone(), self.ty(init).clone())
            {
                let di = self.meta.data_type(d);
                let ei = self.meta.enumeration(e);
                if di.package == ei.package && *di.name == *format!("{}Object", ei.name) {
                    return true;
                }
            }
        }
        self.relationship(var, init) & SUPERTYPE != 0
    }

    /// `AbstractTypeChecker.commonSuperType`.
    pub fn common_super_type(&mut self, t1: TypeId, t2: TypeId) -> Result<TypeId, SuperTypeError> {
        let t1 = self.ocl_type(t1);
        let t2 = self.ocl_type(t2);
        if t1 == t2 {
            return Ok(t2);
        }
        if t1 == TypeId::T {
            return Ok(t2);
        } else if t2 == TypeId::T {
            return Ok(t1);
        }
        if t1 == TypeId::OCL_VOID || t1 == TypeId::OCL_INVALID {
            return Ok(t2);
        }
        if t2 == TypeId::OCL_VOID || t2 == TypeId::OCL_INVALID {
            return Ok(t1);
        }
        if t1 == TypeId::OCL_ANY && !self.is_collection_like(t2) {
            return Ok(t1);
        }
        if t2 == TypeId::OCL_ANY && !self.is_collection_like(t1) {
            return Ok(t2);
        }
        if (t1 == TypeId::INTEGER || t1 == TypeId::UNLIMITED_NATURAL) && t2 == TypeId::REAL {
            return Ok(t2);
        }
        if (t2 == TypeId::INTEGER || t2 == TypeId::UNLIMITED_NATURAL) && t1 == TypeId::REAL {
            return Ok(t1);
        }
        if self.is_collection_like(t1) && self.is_collection_like(t2) {
            let (k1, e1) = self.coll_parts(t1);
            let (k2, e2) = self.coll_parts(t2);
            let kind = common_kind(k1, k2);
            let elem = self.common_super_type(e1, e2)?;
            return Ok(self.types.collection(kind, elem));
        }
        if matches!(self.ty(t1), Type::TypeType(_)) && matches!(self.ty(t2), Type::TypeType(_)) {
            return Ok(TypeId::OCL_TYPE);
        }
        let tup1 = matches!(self.ty(t1), Type::Tuple(_));
        let tup2 = matches!(self.ty(t2), Type::Tuple(_));
        if tup1 || tup2 {
            if !(tup1 && tup2) {
                return Err(SuperTypeError::TupleTypeMismatch(
                    self.name(t1),
                    self.name(t2),
                ));
            }
            let (Type::Tuple(p1), Type::Tuple(p2)) = (self.ty(t1).clone(), self.ty(t2).clone())
            else {
                unreachable!()
            };
            if p1.len() != p2.len() {
                return Err(SuperTypeError::TupleFieldNumMismatch(
                    self.name(t1),
                    self.name(t2),
                ));
            }
            let mut parts = Vec::with_capacity(p1.len());
            for (n, a) in p1.iter() {
                let Some((_, b)) = p2.iter().find(|(m, _)| m == n) else {
                    return Err(SuperTypeError::TupleFieldNotFound(
                        self.name(t1),
                        n.to_string(),
                        self.name(t2),
                    ));
                };
                let c = self.common_super_type(*a, *b)?;
                parts.push((n.clone(), c));
            }
            return Ok(self.types.tuple(parts));
        }
        if self.is_collection_like(t1) || self.is_collection_like(t2) {
            return Err(SuperTypeError::TypeMismatch(self.name(t1), self.name(t2)));
        }
        Ok(self
            .uml_common_super_type(t1, t2)
            .unwrap_or(TypeId::OCL_ANY))
    }

    /// `QVTUMLReflection.getCommonSuperType` over `UMLReflectionImpl`'s.
    fn uml_common_super_type(&mut self, t1: TypeId, t2: TypeId) -> Option<TypeId> {
        if t1 == t2 {
            return Some(t2);
        }
        if let (Type::Class(a), Type::Class(b)) = (self.ty(t1).clone(), self.ty(t2).clone()) {
            let ca = self.meta.class(a);
            let cb = self.meta.class(b);
            if cb.all_supers.contains(&a) {
                return Some(t1);
            }
            if ca.all_supers.contains(&b) {
                return Some(t2);
            }
            let mut s1: Vec<ClassId> = ca.all_supers.to_vec();
            s1.push(a);
            let mut s2: Vec<ClassId> = cb.all_supers.to_vec();
            s2.push(b);
            s1.retain(|x| s2.contains(x));
            if let Some(&last) = s1.last() {
                return Some(self.types.class(last));
            }
        }
        let element = Type::Class(self.std.element);
        if *self.ty(t1) == element && self.is_user_model_element(t2) {
            return Some(t1);
        }
        if *self.ty(t2) == element && self.is_user_model_element(t1) {
            return Some(t2);
        }
        None
    }

    // ---- names ------------------------------------------------------------------------------

    /// `uml.getName(type)`: the classifier's own name (`Set(String)`, `List(Integer)`,
    /// `Dictionary(String, Integer)`, `Tuple(a : Integer)`, `OclType`, `EClass`).
    pub fn name(&self, t: TypeId) -> String {
        match self.ty(t) {
            Type::OclAny => "OclAny".into(),
            Type::OclVoid => "OclVoid".into(),
            Type::OclInvalid => "OclInvalid".into(),
            Type::Boolean => "Boolean".into(),
            Type::Integer => "Integer".into(),
            Type::Real => "Real".into(),
            Type::String => "String".into(),
            Type::UnlimitedNatural => "UnlimitedNatural".into(),
            Type::OclElement => "OclElement".into(),
            Type::OclMessage => "OclMessage".into(),
            Type::State => "State".into(),
            // an unknown (null) element type prints as nothing (`CollectionTypeImpl.getName`)
            Type::Collection(k, e) if *e == TypeId::NULL => format!("{}()", k.name()),
            Type::Collection(k, e) => format!("{}({})", k.name(), self.name(*e)),
            Type::Tuple(parts) => {
                let mut s = String::from("Tuple(");
                for (i, (n, pt)) in parts.iter().enumerate() {
                    if i > 0 {
                        s.push_str(", ");
                    }
                    s.push_str(n);
                    s.push_str(" : ");
                    s.push_str(&self.name(*pt));
                }
                s.push(')');
                s
            }
            Type::TypeType(r) => {
                if t == TypeId::OCL_TYPE {
                    "OclType".into()
                } else {
                    self.name(*r)
                }
            }
            Type::List(e) => format!("List({})", self.name(*e)),
            Type::Dict(k, v) => format!("Dictionary({}, {})", self.name(*k), self.name(*v)),
            Type::Module(m) => self
                .modules
                .get(*m as usize)
                .map(|x| x.name.clone())
                .unwrap_or_default(),
            Type::ModelType(m) => self
                .model_types
                .get(*m as usize)
                .map(|x| x.name.clone())
                .unwrap_or_default(),
            Type::Class(c) => self.meta.class(*c).name.to_string(),
            Type::DataType(d) => self.meta.data_type(*d).name.to_string(),
            Type::Enum(e) => self.meta.enumeration(*e).name.to_string(),
            Type::Null => "<null>".into(),
            Type::Param(p) => match p {
                TParam::T => "T".into(),
                TParam::T2 => "T2".into(),
                TParam::KeyT => "KeyT".into(),
                TParam::CommonT => "CommonT".into(),
            },
        }
    }

    /// The container path of a type, as `getTypeFullName`/`getQualifiedName` print it:
    /// `oclstdlib` for OCL library types, the package path for model classifiers, the module
    /// for types the module's environment creates (collections, tuples, model types) and for
    /// the module's intermediate classes.
    fn container_path(&self, t: TypeId, module: &str) -> Option<String> {
        match self.ty(t) {
            Type::OclAny
            | Type::OclVoid
            | Type::OclInvalid
            | Type::Boolean
            | Type::Integer
            | Type::Real
            | Type::String
            | Type::UnlimitedNatural
            | Type::OclElement
            | Type::OclMessage
            | Type::State
            | Type::Param(TParam::T | TParam::T2) => Some("oclstdlib".into()),
            Type::Param(_) => Some("Stdlib".into()),
            Type::Collection(_, e) if *e == TypeId::T => Some("oclstdlib".into()),
            Type::List(e) if *e == TypeId::T => Some("Stdlib".into()),
            Type::Dict(k, _) if *k == TypeId::KEY_T => Some("Stdlib".into()),
            Type::Collection(..) | Type::Tuple(_) | Type::List(_) | Type::Dict(..) => {
                Some(module.to_string())
            }
            Type::TypeType(_) => Some(module.to_string()),
            Type::ModelType(m) => self
                .model_types
                .get(*m as usize)
                .and_then(|x| self.modules.get(x.module as usize))
                .map(|x| x.name.clone()),
            Type::Module(_) | Type::Null => None,
            Type::Class(c) => Some(self.meta.package_path(self.meta.class(*c).package, "::")),
            Type::DataType(d) => Some(
                self.meta
                    .package_path(self.meta.data_type(*d).package, "::"),
            ),
            Type::Enum(e) => Some(
                self.meta
                    .package_path(self.meta.enumeration(*e).package, "::"),
            ),
        }
    }

    /// `QvtOperationalTypesUtil.getTypeFullName`: `pkg::Class`, `module::Set(String)`
    /// (element `OclVoid` printed as `T`), `OclType(module::...)` for type types.
    pub fn full_name(&self, t: TypeId, module: &str) -> String {
        let base = self.type_name_qvt(t);
        let full = match self.container_path(t, module) {
            Some(p) if !p.is_empty() => format!("{p}::{base}"),
            _ => base,
        };
        if matches!(self.ty(t), Type::TypeType(_)) {
            format!("OclType({full})")
        } else {
            full
        }
    }

    /// `QvtOperationalTypesUtil.getTypeName`.
    fn type_name_qvt(&self, t: TypeId) -> String {
        match self.ty(t) {
            Type::Null => "unknown".into(),
            Type::Collection(k, e) if *k != CollKind::Collection => {
                let en = if *e == TypeId::NULL {
                    "unknown".to_string()
                } else if *e == TypeId::OCL_VOID {
                    "T".to_string()
                } else {
                    self.type_name_qvt(*e)
                };
                format!("{}({en})", k.name())
            }
            _ => self.name(t),
        }
    }

    /// `uml.getQualifiedName(type)` (`safeGetQualifiedName`): container names joined by `::`.
    pub fn qualified_name(&self, t: TypeId, module: &str) -> String {
        let base = self.name(t);
        match self.container_path(t, module) {
            Some(p) if !p.is_empty() => format!("{p}::{base}"),
            _ => base,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum ClassLike {
    Class(ClassId),
    Module(u32),
    ModelType(u32),
}

/// `AbstractTypeChecker.getRelationship(CollectionKind, CollectionKind)`.
fn kind_relationship(k1: CollKind, k2: CollKind) -> u8 {
    use CollKind::*;
    match k1 {
        Bag => match k2 {
            Bag => SAME_TYPE,
            Collection => STRICT_SUBTYPE,
            _ => UNRELATED_TYPE,
        },
        Set => match k2 {
            Set => SAME_TYPE,
            OrderedSet => STRICT_SUPERTYPE,
            Collection => STRICT_SUBTYPE,
            _ => UNRELATED_TYPE,
        },
        OrderedSet => match k2 {
            OrderedSet => SAME_TYPE,
            Set | Collection => STRICT_SUBTYPE,
            _ => UNRELATED_TYPE,
        },
        Sequence => match k2 {
            Sequence => SAME_TYPE,
            Collection => STRICT_SUBTYPE,
            _ => UNRELATED_TYPE,
        },
        Collection => match k2 {
            Collection => SAME_TYPE,
            _ => STRICT_SUPERTYPE,
        },
    }
}

/// `AbstractTypeChecker.commonSuperType(CollectionKind, CollectionKind)`.
fn common_kind(k1: CollKind, k2: CollKind) -> CollKind {
    use CollKind::*;
    match k1 {
        Bag | Sequence => {
            if k2 == k1 {
                k1
            } else {
                Collection
            }
        }
        Set => match k2 {
            OrderedSet | Set => Set,
            _ => Collection,
        },
        OrderedSet => match k2 {
            OrderedSet | Set => k2,
            _ => Collection,
        },
        Collection => Collection,
    }
}

//! Result types of classic OCL's standard library operations
//! (`OCLStandardLibraryUtil.getResultTypeOf`), by operation name (Eclipse's operation codes).
//!
//! Errors Eclipse reports under the context `collectionTypeResultTypeOf` are filtered by QVTo
//! (`QvtOperationalEnv.isMDTOCLCompatibilityFalseProblem`) and therefore not reported here.

use crate::analyzer::Sema;
use crate::messages::{bind, ocl as om};
use crate::ops::{Arg, OpRef};
use qvto_syntax::Span;
use qvto_types::{CollKind, Type, TypeId};

/// `OCLStandardLibraryUtil.getOperationCode(name) > 0`
pub(crate) fn has_opcode(name: &str) -> bool {
    matches!(
        name,
        "+" | "-"
            | "*"
            | "/"
            | "and"
            | "not"
            | "or"
            | "implies"
            | "abs"
            | "div"
            | "mod"
            | "max"
            | "min"
            | "size"
            | "concat"
            | "substring"
            | "toInteger"
            | "toReal"
            | "xor"
            | "floor"
            | "round"
            | "toLower"
            | "toUpper"
            | "allInstances"
            | "="
            | "<>"
            | "oclAsType"
            | "oclIsKindOf"
            | "oclIsTypeOf"
            | "oclIsUndefined"
            | "oclIsInvalid"
            | "<"
            | ">"
            | "<="
            | ">="
            | "oclIsNew"
            | "oclIsInState"
            | "hasReturned"
            | "result"
            | "isSignalSent"
            | "isOperationCall"
            | "count"
            | "excludes"
            | "excludesAll"
            | "includes"
            | "includesAll"
            | "isEmpty"
            | "notEmpty"
            | "product"
            | "sum"
            | "asBag"
            | "asOrderedSet"
            | "asSequence"
            | "asSet"
            | "excluding"
            | "flatten"
            | "including"
            | "intersection"
            | "union"
            | "at"
            | "first"
            | "indexOf"
            | "insertAt"
            | "last"
            | "prepend"
            | "subSequence"
            | "append"
            | "subOrderedSet"
            | "symmetricDifference"
            | "exists"
            | "forAll"
            | "isUnique"
            | "one"
            | "any"
            | "collect"
            | "collectNested"
            | "closure"
            | "select"
            | "reject"
            | "sortedBy"
            | "toBoolean"
            | "toString"
            | "characters"
            | "endsWith"
            | "equalsIgnoreCase"
            | "lastIndexOf"
            | "matches"
            | "replaceAll"
            | "replaceFirst"
            | "startsWith"
            | "substituteAll"
            | "substituteFirst"
            | "tokenize"
            | "trim"
            | "toLowerCase"
            | "toUpperCase"
            | "selectByKind"
            | "selectByType"
            | "oclAsSet"
    )
}

fn arg_type(a: &Arg) -> TypeId {
    match a.referred {
        Some(t) if a.type_exp => t,
        _ => a.ty,
    }
}

impl<'r> Sema<'r> {
    /// `OCLStandardLibraryUtil.getResultTypeOf`
    pub(crate) fn ocl_result_type(
        &mut self,
        problem: Span,
        source: TypeId,
        o: OpRef,
        args: &[Arg],
    ) -> Option<TypeId> {
        let name = self.op_name(o).to_string();
        let name = name.as_str();
        match self.ts.types.get(source).clone() {
            Type::Boolean | Type::Integer | Type::Real | Type::String | Type::UnlimitedNatural => {
                self.primitive_result(problem, source, name, args)
            }
            Type::Collection(k, e) => match k {
                CollKind::Bag => self.bag_result(problem, source, e, name, args),
                CollKind::Set => self.set_result(problem, source, e, name, args),
                CollKind::OrderedSet => self.ordered_set_result(problem, source, e, name, args),
                CollKind::Sequence => self.sequence_result(problem, source, e, name, args),
                CollKind::Collection => self.collection_result(source, e, name, args),
            },
            Type::List(e) => self.collection_result(source, e, name, args),
            Type::Dict(_, v) => self.collection_result(source, v, name, args),
            Type::TypeType(r) => {
                if name == "allInstances" {
                    return Some(self.ts.types.collection(CollKind::Set, r));
                }
                self.any_result(problem, source, name, args)
            }
            _ => self.any_result(problem, source, name, args),
        }
    }

    fn cst(&mut self, problem: Span, a: TypeId, b: TypeId) -> TypeId {
        // TypeUtil.commonSuperType(problemObject, ...): null on failure (reported)
        self.common_super_type_at(problem, a, b)
            .unwrap_or(TypeId::NULL)
    }

    fn any_result(
        &mut self,
        problem: Span,
        source: TypeId,
        name: &str,
        args: &[Arg],
    ) -> Option<TypeId> {
        match name {
            "=" | "<>" | "<" | ">" | "<=" | ">=" | "oclIsKindOf" | "oclIsTypeOf" | "oclIsNew"
            | "oclIsInState" | "oclIsUndefined" | "oclIsInvalid" => Some(TypeId::BOOLEAN),
            "oclAsType" => {
                let a = args.first()?;
                if self.ts.types.is_collection_like(source) {
                    let sn = self.type_name(source);
                    self.error(problem, bind(om::Noncomforming_ERROR_, &[&sn, "oclAsType"]));
                    return None;
                }
                Some(arg_type(a))
            }
            "oclAsSet" => Some(self.ts.types.collection(CollKind::Set, source)),
            _ => None,
        }
    }

    fn primitive_result(
        &mut self,
        problem: Span,
        source: TypeId,
        name: &str,
        args: &[Arg],
    ) -> Option<TypeId> {
        match name {
            "+" | "*" => {
                let a = args.first()?.ty;
                Some(self.cst(problem, a, source))
            }
            "/" => {
                let a = args.first()?.ty;
                self.cst(problem, a, source);
                Some(TypeId::REAL)
            }
            "-" => match args.first() {
                None => Some(source),
                Some(a) => Some(self.cst(problem, a.ty, source)),
            },
            ">" | "<" | ">=" | "<=" | "implies" | "xor" | "not" | "and" | "or" | "endsWith"
            | "equalsIgnoreCase" | "matches" | "startsWith" | "toBoolean" => Some(TypeId::BOOLEAN),
            "min" | "max" | "abs" | "div" | "mod" | "substring" | "concat" => Some(source),
            "floor" | "toInteger" | "size" | "round" | "indexOf" | "lastIndexOf" => {
                Some(TypeId::INTEGER)
            }
            "toReal" => Some(TypeId::REAL),
            "at" | "replaceAll" | "replaceFirst" | "substituteAll" | "substituteFirst"
            | "toLower" | "toLowerCase" | "toString" | "toUpper" | "toUpperCase" | "trim" => {
                Some(TypeId::STRING)
            }
            "characters" | "tokenize" => {
                Some(self.ts.types.collection(CollKind::Sequence, TypeId::STRING))
            }
            _ => self.any_result(problem, source, name, args),
        }
    }

    fn elem_of(&self, t: TypeId) -> TypeId {
        if self.ts.types.is_collection_like(t) {
            self.ts.types.element(t).unwrap_or(TypeId::NULL)
        } else {
            TypeId::NULL
        }
    }

    fn coll(&mut self, k: CollKind, e: TypeId) -> TypeId {
        self.ts.types.collection(k, e)
    }

    fn bag_result(
        &mut self,
        problem: Span,
        source: TypeId,
        elem: TypeId,
        name: &str,
        args: &[Arg],
    ) -> Option<TypeId> {
        Some(match name {
            "=" | "<>" => TypeId::BOOLEAN,
            "union" => {
                let ae = self.elem_of(args.first()?.ty);
                let c = self.cst(problem, elem, ae);
                self.coll(CollKind::Bag, c)
            }
            "including" => {
                let c = self.cst(problem, elem, args.first()?.ty);
                self.coll(CollKind::Bag, c)
            }
            "intersection" => {
                let at = args.first()?.ty;
                let ae = self.elem_of(at);
                let c = self.cst(problem, elem, ae);
                if matches!(self.ts.types.get(at), Type::Collection(CollKind::Set, _)) {
                    self.coll(CollKind::Set, c)
                } else {
                    self.coll(CollKind::Bag, c)
                }
            }
            "excluding" | "asBag" | "select" | "reject" => source,
            "count" => TypeId::INTEGER,
            "flatten" => {
                if !self.ts.types.is_collection_like(elem) {
                    source
                } else {
                    let f = self.flattened_element(source);
                    self.coll(CollKind::Bag, f)
                }
            }
            "asSequence" | "sortedBy" => self.coll(CollKind::Sequence, elem),
            "asSet" => self.coll(CollKind::Set, elem),
            "asOrderedSet" => self.coll(CollKind::OrderedSet, elem),
            "selectByKind" | "selectByType" => {
                let t = arg_type(args.first()?);
                self.coll(CollKind::Bag, t)
            }
            "collectNested" => self.coll(CollKind::Bag, TypeId::T2),
            _ => return self.collection_result(source, elem, name, args),
        })
    }

    fn set_result(
        &mut self,
        problem: Span,
        source: TypeId,
        elem: TypeId,
        name: &str,
        args: &[Arg],
    ) -> Option<TypeId> {
        Some(match name {
            "=" | "<>" => TypeId::BOOLEAN,
            "union" => {
                let at = args.first()?.ty;
                let ae = self.elem_of(at);
                let c = self.cst(problem, elem, ae);
                if matches!(self.ts.types.get(at), Type::Collection(CollKind::Bag, _)) {
                    self.coll(CollKind::Bag, c)
                } else {
                    self.coll(CollKind::Set, c)
                }
            }
            "-" | "symmetricDifference" => {
                let ae = self.elem_of(args.first()?.ty);
                let c = self.cst(problem, elem, ae);
                self.coll(CollKind::Set, c)
            }
            "including" => {
                let c = self.cst(problem, elem, args.first()?.ty);
                self.coll(CollKind::Set, c)
            }
            "intersection" => {
                let ae = self.elem_of(args.first()?.ty);
                self.cst(problem, elem, ae);
                source
            }
            "excluding" | "asSet" | "select" | "reject" => source,
            "count" => TypeId::INTEGER,
            "flatten" => {
                if !self.ts.types.is_collection_like(elem) {
                    source
                } else {
                    let f = self.flattened_element(source);
                    self.coll(CollKind::Set, f)
                }
            }
            "asBag" => self.coll(CollKind::Bag, elem),
            "asSequence" => self.coll(CollKind::Sequence, elem),
            "asOrderedSet" | "sortedBy" => self.coll(CollKind::OrderedSet, elem),
            "selectByKind" | "selectByType" => {
                let t = arg_type(args.first()?);
                self.coll(CollKind::Set, t)
            }
            "collectNested" => self.coll(CollKind::Bag, TypeId::T2),
            _ => return self.collection_result(source, elem, name, args),
        })
    }

    fn ordered_set_result(
        &mut self,
        problem: Span,
        source: TypeId,
        elem: TypeId,
        name: &str,
        args: &[Arg],
    ) -> Option<TypeId> {
        Some(match name {
            "=" | "<>" => TypeId::BOOLEAN,
            "indexOf" => TypeId::INTEGER,
            "append" | "prepend" => {
                let c = self.cst(problem, elem, args.first()?.ty);
                self.coll(CollKind::OrderedSet, c)
            }
            "insertAt" => {
                let c = self.cst(problem, elem, args.get(1)?.ty);
                self.coll(CollKind::OrderedSet, c)
            }
            "subOrderedSet" => source,
            "selectByKind" | "selectByType" => {
                let t = arg_type(args.first()?);
                self.coll(CollKind::OrderedSet, t)
            }
            "at" | "first" | "last" => elem,
            "asSet" => self.coll(CollKind::Set, elem),
            "asBag" => self.coll(CollKind::Bag, elem),
            "asSequence" => self.coll(CollKind::Sequence, elem),
            _ => {
                let set = self.coll(CollKind::Set, elem);
                return self.set_result(problem, set, elem, name, args);
            }
        })
    }

    fn sequence_result(
        &mut self,
        problem: Span,
        source: TypeId,
        elem: TypeId,
        name: &str,
        args: &[Arg],
    ) -> Option<TypeId> {
        Some(match name {
            "count" | "indexOf" => TypeId::INTEGER,
            "=" | "<>" => TypeId::BOOLEAN,
            "union" => {
                let ae = self.elem_of(args.first()?.ty);
                let c = self.cst(problem, elem, ae);
                self.coll(CollKind::Sequence, c)
            }
            "including" | "append" | "prepend" => {
                let c = self.cst(problem, elem, args.first()?.ty);
                self.coll(CollKind::Sequence, c)
            }
            "insertAt" => {
                let c = self.cst(problem, elem, args.get(1)?.ty);
                self.coll(CollKind::Sequence, c)
            }
            "excluding" | "asSequence" | "subSequence" | "sortedBy" => source,
            "flatten" => {
                if !self.ts.types.is_collection_like(elem) {
                    source
                } else {
                    let f = self.flattened_element(source);
                    self.coll(CollKind::Sequence, f)
                }
            }
            "at" | "first" | "last" => elem,
            "asBag" => self.coll(CollKind::Bag, elem),
            "asSet" => self.coll(CollKind::Set, elem),
            "asOrderedSet" => self.coll(CollKind::OrderedSet, elem),
            "select" | "reject" | "selectByKind" | "selectByType" => {
                let t = arg_type(args.first()?);
                self.coll(CollKind::Sequence, t)
            }
            "collectNested" => self.coll(CollKind::Sequence, TypeId::T2),
            _ => return self.collection_result(source, elem, name, args),
        })
    }

    fn collection_result(
        &mut self,
        source: TypeId,
        elem: TypeId,
        name: &str,
        args: &[Arg],
    ) -> Option<TypeId> {
        Some(match name {
            "size" | "count" => TypeId::INTEGER,
            "includes" | "excludes" | "includesAll" | "excludesAll" | "isEmpty" | "notEmpty"
            | "=" | "<>" | "oclIsUndefined" | "oclIsInvalid" => TypeId::BOOLEAN,
            "max" | "min" => {
                if elem != TypeId::REAL && elem != TypeId::INTEGER {
                    return None;
                }
                elem
            }
            "sum" => {
                if elem != TypeId::REAL
                    && elem != TypeId::INTEGER
                    && elem != TypeId::UNLIMITED_NATURAL
                {
                    return None;
                }
                elem
            }
            "product" => {
                let t2 = self.elem_of(args.first()?.ty);
                let tuple = self.tuple_type(vec![("first".into(), elem), ("second".into(), t2)]);
                self.coll(CollKind::Set, tuple)
            }
            "exists" | "forAll" | "isUnique" | "one" => TypeId::BOOLEAN,
            "any" => elem,
            "collect" => self.coll(CollKind::Collection, TypeId::T2),
            "closure" => self.coll(CollKind::Set, TypeId::T2),
            "selectByKind" | "selectByType" => {
                let t = arg_type(args.first()?);
                self.coll(CollKind::Collection, t)
            }
            "oclAsSet" => self.coll(CollKind::Set, source),
            _ => return None,
        })
    }
}

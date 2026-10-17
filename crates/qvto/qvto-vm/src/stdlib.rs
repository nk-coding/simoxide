//! The standard library at run time: every `StdOp` of the generated table (`qvto-types`) maps
//! to a primitive ([`Prim`]) by section, owner and name. Classic OCL operations are evaluated by
//! name on the runtime values, as classic OCL's evaluator does (one `StdOp` can stand for the
//! same signature on several owners); QVTo's handlers (`Q*`, library classes) have their own
//! primitives. A table test checks that every operation has one.

use crate::error::VmResult;
use crate::vm::{Iter, ObjHandle, Vm, push};
use qvto_model::{DataKind, ModelStore};
use qvto_runtime::IntBox;
use qvto_runtime::coll::{Coll, CollKind, ops};
use qvto_runtime::dict::List;
use qvto_runtime::eq::{java_equals, ocl_equal};
use qvto_runtime::num::{self, NumOp};
use qvto_runtime::{ObjRef, Str, TypeRef, Value, strings};
use qvto_types::{Section, StdOp, StdOwner, TypeId};
use std::cell::RefCell;
use std::rc::Rc;

/// What an operation does at run time.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Prim {
    // OclAny
    Eq,
    Ne,
    OclAsType,
    OclIsKindOf,
    OclIsTypeOf,
    OclIsUndefined,
    OclIsInvalid,
    OclIsNew,
    OclIsInState,
    ToString,
    OclAsSet,
    Lt,
    Gt,
    Le,
    Ge,
    // Boolean
    Not,
    And,
    Or,
    Implies,
    Xor,
    // numbers
    Add,
    Sub,
    Neg,
    Mul,
    Divide,
    IntDiv,
    Mod,
    Min,
    Max,
    Abs,
    Floor,
    Round,
    // strings
    Size,
    Concat,
    Substring,
    ToInteger,
    ToReal,
    ToLower,
    ToUpper,
    At,
    Characters,
    EndsWith,
    EqualsIgnoreCase,
    IndexOf,
    LastIndexOf,
    Matches,
    ReplaceAll,
    ReplaceFirst,
    StartsWith,
    SubstituteAll,
    SubstituteFirst,
    ToBoolean,
    Tokenize,
    Trim,
    // types
    AllInstances,
    // collections (classic OCL)
    Count,
    Excludes,
    ExcludesAll,
    Includes,
    IncludesAll,
    IsEmpty,
    NotEmpty,
    Product,
    Sum,
    SelectByKind,
    SelectByType,
    Union,
    Intersection,
    Including,
    Excluding,
    Minus,
    SymmetricDifference,
    Flatten,
    AsBag,
    AsSet,
    AsSequence,
    AsOrderedSet,
    Append,
    Prepend,
    InsertAt,
    SubOrderedSet,
    SubSequence,
    First,
    Last,
    // QVTo collection handlers
    QIncluding,
    QExcluding,
    QUnion,
    QIntersection,
    QMinus,
    QSymDiff,
    QFlatten,
    QAsSet,
    QAsOrderedSet,
    QAsSequence,
    QAsBag,
    QAsList,
    QRepr,
    QDump,
    QIncludes,
    QExcludes,
    QCount,
    QIncludesAll,
    QExcludesAll,
    QEq,
    QNe,
    QAppend,
    QPrepend,
    QInsertAt,
    QIndexOf,
    QReverse,
    // QVTo strings
    SAddSuffixNumber,
    SAsBoolean,
    SAsFloat,
    SAsInteger,
    SFind,
    SFirstToUpper,
    SGetStrCounter,
    SIncrStrCounter,
    SIsQuoted,
    SLastToUpper,
    SLength,
    SMatch,
    SMatchBoolean,
    SMatchFloat,
    SMatchIdentifier,
    SMatchInteger,
    SNormalizeSpace,
    SReplace,
    SRestartAllStrCounter,
    SRfind,
    SStartStrCounter,
    SSubstringAfter,
    SSubstringBefore,
    SQuotify,
    SUnquotify,
    // List
    LAdd,
    LInsertAt,
    LRemove,
    LRemoveAll,
    LRemoveAt,
    LRemoveFirst,
    LRemoveLast,
    LJoinfields,
    LFirst,
    LLast,
    LAt,
    LClone,
    LSelectByType,
    LSelectByKind,
    LSubSequence,
    LFlatten,
    LIncluding,
    LExcluding,
    LUnion,
    LReverse,
    LAppend,
    LPrepend,
    LInsertAtIndex,
    LIndexOf,
    // Dict
    DGet,
    DPut,
    DHasKey,
    DClear,
    DKeys,
    DValues,
    DDefaultGet,
    // Element
    EAllSubobjects,
    EAllSubobjectsOfType,
    EAllSubobjectsOfKind,
    EContainer,
    EMetaClassName,
    ESubobjects,
    ESubobjectsOfType,
    ESubobjectsOfKind,
    EClone,
    EDeepclone,
    // Exception
    ExGetArgument,
    // Model
    MObjects,
    MObjectsOfType,
    MObjectsOfKind,
    MRootObjects,
    MRemoveElement,
    MAllInstances,
    MCopy,
    MCreateEmptyModel,
    // Transformation, Status
    TTransform,
    StSucceeded,
    StFailed,
    StRaisedException,
    // module
    Dump,
    /// Declared by Eclipse but not implemented there (or not here yet).
    Unsupported,
}

fn prim_of(op: StdOp) -> Prim {
    use Prim::*;
    let d = op.desc();
    if d.unsupported.is_some() {
        return Unsupported;
    }
    let n = d.params.len();
    match d.section {
        Section::Ocl => match d.name {
            "=" => Eq,
            "<>" => Ne,
            "oclAsType" => OclAsType,
            "oclIsKindOf" => OclIsKindOf,
            "oclIsTypeOf" => OclIsTypeOf,
            "oclIsUndefined" => OclIsUndefined,
            "oclIsInvalid" => OclIsInvalid,
            "oclIsNew" => OclIsNew,
            "oclIsInState" => OclIsInState,
            "toString" => ToString,
            "oclAsSet" => OclAsSet,
            "<" => Lt,
            ">" => Gt,
            "<=" => Le,
            ">=" => Ge,
            "not" => Not,
            "and" => And,
            "or" => Or,
            "implies" => Implies,
            "xor" => Xor,
            "+" => Add,
            "-" if n == 0 => Neg,
            "-" if d.owner == StdOwner::Set => Minus,
            "-" => Sub,
            "*" => Mul,
            "/" => Divide,
            "div" => IntDiv,
            "mod" => Mod,
            "min" if d.owner == StdOwner::Collection => Min,
            "max" if d.owner == StdOwner::Collection => Max,
            "min" => Min,
            "max" => Max,
            "abs" => Abs,
            "floor" => Floor,
            "round" => Round,
            "size" => Size,
            "concat" => Concat,
            "substring" => Substring,
            "toInteger" => ToInteger,
            "toReal" => ToReal,
            "toLower" | "toLowerCase" => ToLower,
            "toUpper" | "toUpperCase" => ToUpper,
            "at" => At,
            "characters" => Characters,
            "endsWith" => EndsWith,
            "equalsIgnoreCase" => EqualsIgnoreCase,
            "indexOf" => IndexOf,
            "lastIndexOf" => LastIndexOf,
            "matches" => Matches,
            "replaceAll" => ReplaceAll,
            "replaceFirst" => ReplaceFirst,
            "startsWith" => StartsWith,
            "substituteAll" => SubstituteAll,
            "substituteFirst" => SubstituteFirst,
            "toBoolean" => ToBoolean,
            "tokenize" => Tokenize,
            "trim" => Trim,
            "allInstances" => AllInstances,
            "count" => Count,
            "excludes" => Excludes,
            "excludesAll" => ExcludesAll,
            "includes" => Includes,
            "includesAll" => IncludesAll,
            "isEmpty" => IsEmpty,
            "notEmpty" => NotEmpty,
            "product" => Product,
            "sum" => Sum,
            "selectByKind" => SelectByKind,
            "selectByType" => SelectByType,
            "union" => Union,
            "intersection" => Intersection,
            "including" => Including,
            "excluding" => Excluding,
            "symmetricDifference" => SymmetricDifference,
            "flatten" => Flatten,
            "asBag" => AsBag,
            "asSet" => AsSet,
            "asSequence" => AsSequence,
            "asOrderedSet" => AsOrderedSet,
            "append" => Append,
            "prepend" => Prepend,
            "insertAt" => InsertAt,
            "subOrderedSet" => SubOrderedSet,
            "subSequence" => SubSequence,
            "first" => First,
            "last" => Last,
            _ => Unsupported,
        },
        Section::Qvto => match d.owner {
            StdOwner::String => match d.name {
                "addSuffixNumber" => SAddSuffixNumber,
                "asBoolean" => SAsBoolean,
                "asFloat" => SAsFloat,
                "asInteger" => SAsInteger,
                "find" => SFind,
                "firstToUpper" => SFirstToUpper,
                "getStrCounter" => SGetStrCounter,
                "incrStrCounter" => SIncrStrCounter,
                "isQuoted" => SIsQuoted,
                "lastToUpper" => SLastToUpper,
                "length" => SLength,
                "match" => SMatch,
                "matchBoolean" => SMatchBoolean,
                "matchFloat" => SMatchFloat,
                "matchIdentifier" => SMatchIdentifier,
                "matchInteger" => SMatchInteger,
                "normalizeSpace" => SNormalizeSpace,
                "replace" => SReplace,
                "restartAllStrCounter" => SRestartAllStrCounter,
                "rfind" => SRfind,
                "startStrCounter" => SStartStrCounter,
                "substringAfter" => SSubstringAfter,
                "substringBefore" => SSubstringBefore,
                "quotify" => SQuotify,
                "unquotify" => SUnquotify,
                _ => Unsupported,
            },
            StdOwner::OclAny => match d.name {
                "repr" => QRepr,
                "dump" => QDump,
                "allInstances" => EAllSubobjectsOfKind,
                _ => Unsupported,
            },
            StdOwner::List => match d.name {
                "add" => LAdd,
                "insertAt" if d.params.first().map(|p| p.0) == Some("index") => LInsertAtIndex,
                "insertAt" => LInsertAt,
                "remove" => LRemove,
                "removeAll" => LRemoveAll,
                "removeAt" => LRemoveAt,
                "removeFirst" => LRemoveFirst,
                "removeLast" => LRemoveLast,
                "joinfields" => LJoinfields,
                "first" => LFirst,
                "last" => LLast,
                "at" => LAt,
                "clone" => LClone,
                "selectByType" => LSelectByType,
                "selectByKind" => LSelectByKind,
                "subSequence" => LSubSequence,
                "flatten" => LFlatten,
                "including" => LIncluding,
                "excluding" => LExcluding,
                "union" => LUnion,
                "reverse" => LReverse,
                "append" => LAppend,
                "prepend" => LPrepend,
                "indexOf" => LIndexOf,
                _ => Unsupported,
            },
            StdOwner::Dict => match d.name {
                "get" => DGet,
                "put" => DPut,
                "hasKey" => DHasKey,
                "clear" => DClear,
                "keys" => DKeys,
                "values" => DValues,
                "defaultget" => DDefaultGet,
                _ => Unsupported,
            },
            _ => match d.name {
                "asSet" => QAsSet,
                "asOrderedSet" => QAsOrderedSet,
                "asSequence" => QAsSequence,
                "asBag" => QAsBag,
                "asList" => QAsList,
                "flatten" => QFlatten,
                "repr" => QRepr,
                "dump" => QDump,
                "includes" => QIncludes,
                "excludes" => QExcludes,
                "count" => QCount,
                "includesAll" => QIncludesAll,
                "excludesAll" => QExcludesAll,
                "=" => QEq,
                "<>" => QNe,
                "union" => QUnion,
                "intersection" => QIntersection,
                "-" => QMinus,
                "including" => QIncluding,
                "excluding" => QExcluding,
                "symmetricDifference" => QSymDiff,
                "append" => QAppend,
                "prepend" => QPrepend,
                "insertAt" => QInsertAt,
                "indexOf" => QIndexOf,
                "reverse" => QReverse,
                _ => Unsupported,
            },
        },
        Section::Class => match (d.owner, d.name) {
            (StdOwner::Element, "allSubobjects") => EAllSubobjects,
            (StdOwner::Element, "allSubobjectsOfType") => EAllSubobjectsOfType,
            (StdOwner::Element, "allSubobjectsOfKind") => EAllSubobjectsOfKind,
            (StdOwner::Element, "container") => EContainer,
            (StdOwner::Element, "metaClassName") => EMetaClassName,
            (StdOwner::Element, "subobjects") => ESubobjects,
            (StdOwner::Element, "subobjectsOfType") => ESubobjectsOfType,
            (StdOwner::Element, "subobjectsOfKind") => ESubobjectsOfKind,
            (StdOwner::Element, "clone") => EClone,
            (StdOwner::Element, "deepclone") => EDeepclone,
            (StdOwner::Exception, "getArgument") => ExGetArgument,
            (StdOwner::Model, "objects") => MObjects,
            (StdOwner::Model, "objectsOfType") => MObjectsOfType,
            (StdOwner::Model, "objectsOfKind") => MObjectsOfKind,
            (StdOwner::Model, "rootObjects") => MRootObjects,
            (StdOwner::Model, "removeElement") => MRemoveElement,
            (StdOwner::Model, "allInstances") => MAllInstances,
            (StdOwner::Model, "copy") => MCopy,
            (StdOwner::Model, "createEmptyModel") => MCreateEmptyModel,
            (StdOwner::Stdlib, "dump") => Dump,
            (StdOwner::Transformation, "transform") => TTransform,
            (StdOwner::Status, "succeeded") => StSucceeded,
            (StdOwner::Status, "failed") => StFailed,
            (StdOwner::Status, "raisedException") => StRaisedException,
            _ => Unsupported,
        },
    }
}

/// The primitive of every operation, indexed by `StdOp as usize`.
pub fn prim_table() -> Vec<Prim> {
    StdOp::all().iter().map(|&op| prim_of(op)).collect()
}

/// [`prim_table`], computed once.
pub(crate) fn prims() -> &'static [Prim] {
    static PRIMS: std::sync::OnceLock<Vec<Prim>> = std::sync::OnceLock::new();
    PRIMS.get_or_init(prim_table)
}

/// A `java.lang.Integer` index (a Long index fails the cast in Eclipse).
fn index_arg(v: &Value) -> Option<i32> {
    match v {
        Value::Int(i) if *i as i32 as i64 == *i => Some(*i as i32),
        _ => None,
    }
}

fn type_arg_of(
    v: &Value,
    objs: &qvto_runtime::fxhash::FxHashMap<ObjRef, TypeId>,
) -> Option<TypeId> {
    match v {
        Value::Type(TypeRef(t)) => Some(TypeId(*t)),
        // a type literal of a metamodel classifier is its `EClassifier` object
        Value::Obj(o) => objs.get(o).copied(),
        _ => None,
    }
}

fn is_coll(v: &Value) -> bool {
    matches!(v, Value::Coll(_) | Value::List(_) | Value::Dict(_))
}

/// Parses a data type literal (`new T('lit')`, configuration properties).
pub(crate) fn parse_data_value(k: DataKind, s: &str) -> Option<Value> {
    let units: Vec<u16> = s.encode_utf16().collect();
    use qvto_runtime::java::parse;
    Some(match k {
        DataKind::String => Value::str(s),
        DataKind::Boolean => Value::Bool(s.eq_ignore_ascii_case("true")),
        DataKind::Int => Value::Int(parse::parse_int(&units)? as i64),
        DataKind::Long => Value::long(parse::parse_long(&units)?),
        DataKind::Short => {
            let i = parse::parse_int(&units)?;
            if i as i16 as i32 != i {
                return None;
            }
            Value::Boxed(qvto_runtime::IntBox::Short, i)
        }
        DataKind::Byte => {
            let i = parse::parse_int(&units)?;
            if i as i8 as i32 != i {
                return None;
            }
            Value::Boxed(qvto_runtime::IntBox::Byte, i)
        }
        DataKind::Double => Value::Real(parse::parse_double(&units)?),
        DataKind::Float => Value::Float(parse::parse_double(&units)? as f32),
        DataKind::Char => Value::Boxed(qvto_runtime::IntBox::Char, *units.first()? as i32),
        _ => Value::str(s),
    })
}

impl<S: ModelStore> Vm<'_, S>
where
    S::Obj: ObjHandle,
{
    /// Calls a standard library operation; receiver at `at`, arguments after it.
    pub(crate) fn call_std(
        &mut self,
        op: StdOp,
        at: usize,
        src_unl: bool,
        arg_unl: bool,
    ) -> VmResult<Value> {
        let n = op.desc().params.len();
        // the arguments in a buffer the VM keeps (a nested call takes a fresh one)
        let mut args = std::mem::take(&mut self.argbuf);
        args.clear();
        for i in 0..=n {
            args.push(std::mem::take(&mut self.regs[at + i]));
        }
        let prim = self.prims[op as usize];
        let r = self.prim(prim, op, &mut args, src_unl, arg_unl);
        args.clear();
        self.argbuf = args;
        let v = r?;
        self.check_value_size(&v)?;
        Ok(v)
    }

    pub(crate) fn prim_call(
        &mut self,
        prim: Prim,
        op: StdOp,
        mut args: Vec<Value>,
    ) -> VmResult<Value> {
        let v = self.prim(prim, op, &mut args, false, false)?;
        self.check_value_size(&v)?;
        Ok(v)
    }

    fn prim(
        &mut self,
        prim: Prim,
        op: StdOp,
        args: &mut [Value],
        su: bool,
        au: bool,
    ) -> VmResult<Value> {
        use Prim::*;
        let env = &self.env;
        let a1 = args.get(1).cloned().unwrap_or(Value::Null);
        let s = std::mem::take(&mut args[0]);
        // QVTo handlers (and library class operations) give invalid for an undefined source
        let qvto_handler = matches!(op.desc().section, Section::Qvto | Section::Class);
        if qvto_handler
            && s.is_undefined()
            && !matches!(
                prim,
                SStartStrCounter | SGetStrCounter | SIncrStrCounter | SRestartAllStrCounter | Dump
            )
            && op.desc().owner != StdOwner::Stdlib
            && !op.desc().is_static
        {
            return Ok(Value::Invalid);
        }
        // classic OCL: an invalid argument of a one-argument operation (other than the boolean
        // ones) makes the call invalid
        if op.desc().section == Section::Ocl
            && args.len() == 2
            && a1.is_invalid()
            && !matches!(prim, And | Or | Implies | Xor | Not)
        {
            return Ok(Value::Invalid);
        }
        // and of a two-argument one (classic OCL's "ternary operations": an undefined source
        // or an invalid argument)
        if op.desc().section == Section::Ocl
            && args.len() == 3
            && (s.is_undefined() || a1.is_invalid() || args[2].is_invalid())
        {
            return Ok(Value::Invalid);
        }
        // so do QVTo's collection handlers (`SetTypeOperations`, `OrderedSetTypeOperations`,
        // `CollectionTypeOperations`, shared by List)
        if matches!(
            prim,
            QIncluding
                | LIncluding
                | QExcluding
                | LExcluding
                | QAppend
                | LAppend
                | QPrepend
                | LPrepend
                | QIndexOf
                | LIndexOf
                | QIncludes
                | QExcludes
                | QCount
                | QUnion
                | LUnion
                | QIntersection
                | QMinus
                | QSymDiff
        ) && a1.is_invalid()
            || matches!(prim, QInsertAt | LInsertAtIndex)
                && args.get(2).is_some_and(|v| v.is_invalid())
        {
            return Ok(Value::Invalid);
        }
        Ok(match prim {
            Eq | Ne | QEq | QNe => {
                if s.is_invalid() || a1.is_invalid() || (matches!(prim, QEq | QNe) && s.is_null()) {
                    Value::Invalid
                } else {
                    let eq = ocl_equal(&s, &a1, env);
                    Value::Bool(if matches!(prim, Eq | QEq) { eq } else { !eq })
                }
            }
            OclAsType => match type_arg_of(&a1, &self.object_types) {
                Some(t) => self.ocl_as_type(s, t, self.std_src_ty),
                None => Value::Invalid,
            },
            OclIsKindOf => match type_arg_of(&a1, &self.object_types) {
                Some(t) => Value::Bool(self.ocl_is_kind_of(&s, t)),
                None => Value::Invalid,
            },
            OclIsTypeOf => match type_arg_of(&a1, &self.object_types) {
                Some(t) => Value::Bool(self.ocl_is_type_of(&s, t)),
                None => Value::Invalid,
            },
            OclIsUndefined => Value::Bool(s.is_undefined()),
            OclIsInvalid => Value::Bool(s.is_invalid()),
            OclIsNew | OclIsInState => Value::Invalid,
            ToString => {
                if s.is_invalid() {
                    Value::str("invalid")
                } else {
                    Value::str(&self.to_java_string(&s, su))
                }
            }
            QRepr => Value::str(&self.to_java_string(&s, false)),
            OclAsSet => match s {
                Value::Invalid => Value::Invalid,
                Value::Null => Value::Coll(Rc::new(Coll::new_set(env))),
                v => {
                    let mut c = Coll::new_set(env);
                    c.add(v, env);
                    Value::Coll(Rc::new(c))
                }
            },
            Lt | Gt | Le | Ge => {
                if s.is_undefined() {
                    return Ok(Value::Invalid);
                }
                if let (Value::Str(x), Value::Str(y)) = (&s, &a1) {
                    let o = strings::compare(x, y);
                    use std::cmp::Ordering::*;
                    return Ok(Value::Bool(match prim {
                        Lt => o == Less,
                        Gt => o == Greater,
                        Le => o != Greater,
                        _ => o != Less,
                    }));
                }
                let nop = match prim {
                    Lt => NumOp::Lt,
                    Gt => NumOp::Gt,
                    Le => NumOp::Le,
                    _ => NumOp::Ge,
                };
                num::binary(nop, &s, &a1, su, au).unwrap_or(Value::Invalid)
            }
            Not => match s {
                Value::Bool(b) => Value::Bool(!b),
                _ => Value::Invalid,
            },
            And | Or | Implies | Xor => {
                let (Value::Bool(x), Value::Bool(y)) = (&s, &a1) else {
                    return Ok(match (prim, &s, &a1) {
                        (And, Value::Bool(false), _) | (And, _, Value::Bool(false)) => {
                            Value::Bool(false)
                        }
                        (Or, Value::Bool(true), _) | (Or, _, Value::Bool(true)) => {
                            Value::Bool(true)
                        }
                        (Implies, Value::Bool(false), _) | (Implies, _, Value::Bool(true)) => {
                            Value::Bool(true)
                        }
                        _ => Value::Invalid,
                    });
                };
                Value::Bool(match prim {
                    And => *x && *y,
                    Or => *x || *y,
                    Implies => !*x || *y,
                    _ => *x ^ *y,
                })
            }
            Add => {
                if let Value::Str(x) = &s {
                    match &a1 {
                        Value::Str(y) => strings::concat(x, y),
                        _ => Value::Invalid,
                    }
                } else if s.is_undefined() {
                    Value::Invalid
                } else {
                    num::binary(NumOp::Plus, &s, &a1, su, au).unwrap_or(Value::Invalid)
                }
            }
            Sub | Mul | Divide | IntDiv | Mod | Min | Max if !is_coll(&s) => {
                if s.is_undefined() {
                    return Ok(Value::Invalid);
                }
                let nop = match prim {
                    Sub => NumOp::Minus,
                    Mul => NumOp::Times,
                    Divide => NumOp::Divide,
                    IntDiv => NumOp::Div,
                    Mod => NumOp::Mod,
                    Min => NumOp::Min,
                    _ => NumOp::Max,
                };
                num::binary(nop, &s, &a1, su, au).unwrap_or(Value::Invalid)
            }
            Sub => ops::minus(s, a1, env),
            Mul | Divide | IntDiv | Mod => Value::Invalid,
            Min => match &s {
                Value::Coll(c) => ops::min(c),
                _ => self.coll_op(&s, ops::min),
            },
            Max => match &s {
                Value::Coll(c) => ops::max(c),
                _ => self.coll_op(&s, ops::max),
            },
            Neg => num::negate(&s, su),
            Abs => num::abs(&s, su),
            Floor => num::floor(&s, su),
            Round => num::round(&s, su),
            Size => match &s {
                Value::Str(x) => strings::size(x),
                Value::Coll(c) => Value::Int(c.len() as i64),
                Value::List(l) => Value::Int(l.borrow().items.len() as i64),
                Value::Dict(d) => Value::Int(d.borrow().len() as i64),
                _ => Value::Invalid,
            },
            SLength => match &s {
                Value::Str(x) => strings::size(x),
                _ => Value::Invalid,
            },
            Concat => match (&s, &a1) {
                (Value::Str(x), Value::Str(y)) => strings::concat(x, y),
                _ => Value::Invalid,
            },
            // `(Integer) arg`: a Long (or Short, Byte) argument fails the cast
            Substring => match (&s, index_arg(&a1), args.get(2).and_then(index_arg)) {
                (Value::Str(x), Some(l), Some(u)) => strings::substring(x, l as i64, u as i64),
                _ => Value::Invalid,
            },
            ToInteger => str_fn(&s, strings::to_integer),
            ToReal => str_fn(&s, strings::to_real),
            ToLower => str_fn(&s, strings::to_lower),
            ToUpper => str_fn(&s, strings::to_upper),
            Characters => str_fn(&s, strings::characters),
            Trim => str_fn(&s, strings::trim),
            ToBoolean => str_fn(&s, strings::to_boolean),
            At => match &s {
                Value::Str(x) => match index_arg(&a1) {
                    Some(i) => strings::at(x, i as i64),
                    None => Value::Invalid,
                },
                Value::Coll(c) => match index_arg(&a1) {
                    Some(i) => ops::at(c, i),
                    None => Value::Invalid,
                },
                Value::List(_) | Value::Dict(_) => match index_arg(&a1) {
                    Some(i) => self.coll_op(&s, |c| ops::at(c, i)),
                    None => Value::Invalid,
                },
                _ => Value::Invalid,
            },
            EndsWith => str2(&s, &a1, strings::ends_with),
            StartsWith => str2(&s, &a1, strings::starts_with),
            EqualsIgnoreCase => str2(&s, &a1, strings::equals_ignore_case),
            IndexOf => match &s {
                Value::Str(x) => match &a1 {
                    Value::Str(y) => strings::index_of(x, y),
                    _ => Value::Invalid,
                },
                Value::Coll(c) => ops::index_of(c, &a1, env),
                _ => self.coll_op(&s, |c| ops::index_of(c, &a1, env)),
            },
            LastIndexOf => str2(&s, &a1, strings::last_index_of),
            SubstituteAll | SubstituteFirst => {
                let a2 = args.get(2).cloned().unwrap_or(Value::Null);
                match (&s, &a1, &a2) {
                    (Value::Str(x), Value::Str(y), Value::Str(z)) => {
                        if prim == SubstituteAll {
                            strings::substitute_all(x, y, z)
                        } else {
                            strings::substitute_first(x, y, z)
                        }
                    }
                    _ => Value::Invalid,
                }
            }
            Tokenize => {
                let Value::Str(x) = &s else {
                    return Ok(Value::Invalid);
                };
                match args.len() {
                    1 => strings::tokenize(x, None, false),
                    2 => match &a1 {
                        Value::Str(d) => strings::tokenize(x, Some(d), false),
                        _ => Value::Invalid,
                    },
                    _ => match (&a1, args.get(2)) {
                        (Value::Str(d), Some(Value::Bool(b))) => strings::tokenize(x, Some(d), *b),
                        _ => Value::Invalid,
                    },
                }
            }
            Matches | SMatch => match (&s, &a1) {
                (Value::Str(x), Value::Str(p)) => match self.regex(p) {
                    Ok(re) => Value::Bool(re.is_match(&x.to_str_lossy())),
                    Err(m) => self.swallow(m),
                },
                _ => Value::Invalid,
            },
            ReplaceAll | ReplaceFirst => {
                let a2 = args.get(2).cloned().unwrap_or(Value::Null);
                match (&s, &a1, &a2) {
                    (Value::Str(x), Value::Str(p), Value::Str(r)) => {
                        match self.regex_find(p).and_then(|re| {
                            crate::jregex::replace(
                                &re,
                                &x.to_str_lossy(),
                                &r.to_str_lossy(),
                                prim == ReplaceAll,
                            )
                        }) {
                            Ok(out) => Value::str(&out),
                            Err(m) => self.swallow(m),
                        }
                    }
                    _ => Value::Invalid,
                }
            }
            AllInstances => match type_arg_of(&s, &self.object_types) {
                Some(t) => self.all_instances(t),
                None => Value::Invalid,
            },
            // ---- collections ----
            Count | QCount => {
                if prim == QCount && a1.is_invalid() {
                    return Ok(Value::Invalid);
                }
                self.coll_op(&s, |c| Value::Int(ops::count(c, &a1, env) as i64))
            }
            Includes | QIncludes | Excludes | QExcludes => {
                if matches!(prim, QIncludes | QExcludes) && a1.is_invalid() {
                    return Ok(Value::Invalid);
                }
                let inc = matches!(prim, Includes | QIncludes);
                self.coll_op(&s, |c| Value::Bool(ops::includes(c, &a1, env) == inc))
            }
            IncludesAll | QIncludesAll | ExcludesAll | QExcludesAll => {
                let Some(d) = self.as_coll(&a1) else {
                    return Ok(Value::Invalid);
                };
                let inc = matches!(prim, IncludesAll | QIncludesAll);
                self.coll_op(&s, |c| {
                    Value::Bool(if inc {
                        ops::includes_all(c, &d, env)
                    } else {
                        ops::excludes_all(c, &d, env)
                    })
                })
            }
            IsEmpty | NotEmpty => {
                let e = prim == IsEmpty;
                match &s {
                    Value::Coll(c) => Value::Bool(c.is_empty() == e),
                    Value::List(l) => Value::Bool(l.borrow().items.is_empty() == e),
                    Value::Dict(d) => Value::Bool(d.borrow().is_empty() == e),
                    Value::Str(x) => Value::Bool((x.len16() == 0) == e),
                    _ => Value::Invalid,
                }
            }
            Sum => match self.as_coll(&s) {
                Some(c) => match ops::sum(&c) {
                    Some(v) => v,
                    None => {
                        // empty: by the static element type
                        let el = self.prog.ir.types.element(self.std_src_ty);
                        match el {
                            Some(TypeId::INTEGER) | Some(TypeId::UNLIMITED_NATURAL) => {
                                Value::Int(0)
                            }
                            Some(TypeId::REAL) => Value::Real(0.0),
                            _ => Value::Null,
                        }
                    }
                },
                None => Value::Invalid,
            },
            Product => {
                // CollectionUtil.product: a Set of Tuple(first, second) of the call's element type
                let (Some(x), Some(y)) = (self.as_coll(&s), self.as_coll(&a1)) else {
                    return Ok(Value::Invalid);
                };
                self.check_collection_size(x.len() as i64 * y.len() as i64)?;
                let tt = self
                    .prog
                    .ir
                    .types
                    .element(self.std_res_ty)
                    .unwrap_or(TypeId::OCL_VOID);
                let names: Rc<[Str]> = match self.prog.ir.types.get(tt) {
                    qvto_types::Type::Tuple(parts) => {
                        parts.iter().map(|(n, _)| Str::new(n)).collect()
                    }
                    _ => [Str::new("first"), Str::new("second")]
                        .into_iter()
                        .collect(),
                };
                let pos =
                    |w: &str, d: usize| names.iter().position(|n| n.to_string() == w).unwrap_or(d);
                let (first, second) = (pos("first", 0), pos("second", 1));
                let xs = Iter::of(&s).items;
                let ys = Iter::of(&a1).items;
                let mut out = self.new_coll(qvto_bytecode::insn::coll_kind::SET);
                for x in xs.iter() {
                    for y in ys.iter() {
                        let mut vals = vec![Value::Null; names.len()];
                        vals[first] = x.clone();
                        if second < vals.len() {
                            vals[second] = y.clone();
                        }
                        let t = qvto_runtime::tuple::Tuple::new(
                            tt.0,
                            names.clone(),
                            vals.into_boxed_slice(),
                        );
                        push(&mut out, Value::Tuple(Rc::new(t)), &self.env);
                    }
                }
                out
            }
            SelectByKind | SelectByType | LSelectByKind | LSelectByType => {
                let Some(t) = type_arg_of(&a1, &self.object_types) else {
                    return Ok(Value::Invalid);
                };
                let kind = matches!(prim, SelectByKind | LSelectByKind);
                let items = Iter::of(&s).items;
                let mut out = self.new_coll_like(&s);
                for x in items.iter() {
                    let ok = if kind {
                        !x.is_null() && self.is_kind_of(x, t)
                    } else {
                        self.ocl_is_type_of(x, t)
                    };
                    if ok {
                        push(&mut out, x.clone(), &self.env);
                    }
                }
                out
            }
            Union => ops::union(s, a1, env),
            QUnion | LUnion => ops::qvto_union(s, a1, env),
            Intersection | QIntersection => ops::intersection(s, a1, env),
            Including => {
                if a1.is_invalid() {
                    Value::Invalid
                } else {
                    ops::including(s, a1, env)
                }
            }
            QIncluding | LIncluding => ops::qvto_including(s, a1, env),
            Excluding => ops::excluding(s, &a1, env),
            QExcluding | LExcluding => ops::qvto_excluding(s, &a1, env),
            Minus => ops::minus(s, a1, env),
            QMinus => ops::qvto_minus(s, a1, env),
            SymmetricDifference => ops::symmetric_difference(s, a1, env),
            QSymDiff => ops::qvto_symmetric_difference(s, a1, env),
            Flatten => ops::flatten(s, env),
            QFlatten | LFlatten => ops::qvto_flatten(s, env),
            AsBag | QAsBag => ops::as_bag(s, env),
            AsSet | QAsSet => ops::as_set(s, env),
            AsSequence => ops::as_sequence(s, env),
            QAsSequence => ops::qvto_as_sequence(s, env),
            AsOrderedSet | QAsOrderedSet => ops::as_ordered_set(s, env),
            QAsList => ops::qvto_as_list(s),
            Append => ops::append(s, a1, env),
            QAppend | LAppend => ops::qvto_append(s, a1, env),
            Prepend => ops::prepend(s, a1, env),
            QPrepend | LPrepend => ops::qvto_prepend(s, a1, env),
            InsertAt => match index_arg(&a1) {
                Some(i) => ops::insert_at(s, i, args.get(2).cloned().unwrap_or(Value::Null), env),
                None => Value::Invalid,
            },
            QInsertAt | LInsertAtIndex => match index_arg(&a1) {
                Some(i) => {
                    ops::qvto_insert_at(s, i, args.get(2).cloned().unwrap_or(Value::Null), env)
                }
                None => Value::Invalid,
            },
            SubOrderedSet => match (index_arg(&a1), args.get(2).and_then(index_arg)) {
                (Some(l), Some(u)) => ops::sub_ordered_set(s, l, u, env),
                _ => Value::Invalid,
            },
            SubSequence | LSubSequence => match (index_arg(&a1), args.get(2).and_then(index_arg)) {
                (Some(l), Some(u)) => {
                    let r = ops::sub_sequence(s.clone(), l, u);
                    if matches!(s, Value::List(_)) {
                        ops::qvto_as_list(r)
                    } else {
                        r
                    }
                }
                _ => Value::Invalid,
            },
            First | LFirst => self.coll_op(&s, ops::first),
            Last | LLast => self.coll_op(&s, ops::last),
            QIndexOf | LIndexOf => {
                if a1.is_invalid() {
                    Value::Invalid
                } else {
                    self.coll_op(&s, |c| ops::index_of(c, &a1, env))
                }
            }
            QReverse | LReverse => ops::qvto_reverse(s, env),
            QDump => {
                let text = self.to_java_string(&s, false);
                self.log_line(&text);
                Value::Null
            }
            Dump => {
                let text = self.to_java_string(&a1, false);
                self.log_line(&text);
                Value::Null
            }
            // ---- QVTo strings ----
            SFirstToUpper => str_fn(&s, strings::first_to_upper),
            SLastToUpper => str_fn(&s, strings::last_to_upper),
            SNormalizeSpace => str_fn(&s, strings::normalize_space),
            SAsBoolean => str_fn(&s, strings::as_boolean),
            SAsInteger => str_fn(&s, strings::as_integer),
            SAsFloat => str_fn(&s, strings::as_float),
            SMatchIdentifier => str_fn(&s, strings::match_identifier),
            SSubstringAfter => str2(&s, &a1, strings::substring_after),
            SSubstringBefore => str2(&s, &a1, strings::substring_before),
            SFind => str2(&s, &a1, strings::find),
            SRfind => str2(&s, &a1, strings::rfind),
            SIsQuoted => str2(&s, &a1, strings::is_quoted),
            SQuotify => str2(&s, &a1, strings::quotify),
            SUnquotify => str2(&s, &a1, strings::unquotify),
            SReplace => {
                let a2 = args.get(2).cloned().unwrap_or(Value::Null);
                match (&s, &a1, &a2) {
                    (Value::Str(x), Value::Str(y), Value::Str(z)) => {
                        Value::Str(qvto_runtime::java::string::replace(x, y, z))
                    }
                    _ => Value::Invalid,
                }
            }
            SMatchBoolean => match (&s, &a1) {
                (Value::Str(x), Value::Bool(b)) => match strings::as_boolean(x) {
                    Value::Bool(c) => Value::Bool(c == *b),
                    _ => Value::Bool(false),
                },
                _ => Value::Invalid,
            },
            SMatchInteger => match (&s, &a1) {
                (Value::Str(x), Value::Int(_)) => {
                    let v = strings::to_integer(x);
                    Value::Bool(!v.is_invalid() && java_equals(&v, &a1, env))
                }
                _ => Value::Invalid,
            },
            SMatchFloat => match &s {
                Value::Str(x) => match (strings::to_real(x), a1.as_jnum()) {
                    (Value::Real(r), Some(n)) => Value::Bool(r == n.as_f64()),
                    (_, Some(_)) => Value::Bool(false),
                    _ => Value::Invalid,
                },
                _ => Value::Invalid,
            },
            SStartStrCounter => {
                if let Value::Str(x) = &a1.clone().or(&s) {
                    self.str_counters.insert(x.to_str_lossy().into_owned(), 0);
                }
                Value::Null
            }
            SGetStrCounter => match &a1.clone().or(&s) {
                Value::Str(x) => match self.str_counters.get(&*x.to_str_lossy()) {
                    Some(v) => Value::Int(*v),
                    None => Value::Null,
                },
                _ => Value::Invalid,
            },
            SIncrStrCounter => match &a1.clone().or(&s) {
                Value::Str(x) => {
                    let c = self
                        .str_counters
                        .entry(x.to_str_lossy().into_owned())
                        .or_insert(0);
                    *c += 1;
                    Value::Int(*c)
                }
                _ => Value::Invalid,
            },
            SRestartAllStrCounter => {
                for v in self.str_counters.values_mut() {
                    *v = 0;
                }
                Value::Null
            }
            SAddSuffixNumber => match &s {
                Value::Str(x) => {
                    let k = x.to_str_lossy().into_owned();
                    match self.str_counters.get_mut(&k) {
                        None => {
                            self.str_counters.insert(k, 0);
                            s.clone()
                        }
                        Some(c) => {
                            let before = *c;
                            *c += 1;
                            Value::str(&format!("{k}{before}"))
                        }
                    }
                }
                _ => Value::Invalid,
            },
            // ---- List ----
            // `ListOperations`' mutators are typed `OclVoid` but return the list itself (or
            // invalid for a source that is no list)
            LAdd => match &s {
                Value::List(l) => {
                    if !a1.is_invalid() {
                        l.borrow_mut().items.push(a1);
                    }
                    s.clone()
                }
                _ => Value::Invalid,
            },
            LInsertAt => {
                // insertAt(element, index): ArrayList.add(index - 1, element)
                let idx = args.get(2).and_then(index_arg);
                match (&s, idx) {
                    (Value::List(l), Some(i)) => {
                        if a1.is_invalid() {
                            s.clone()
                        } else {
                            let mut lb = l.borrow_mut();
                            if i >= 1 && (i as usize) <= lb.items.len() + 1 {
                                lb.items.insert(i as usize - 1, a1);
                                drop(lb);
                                s.clone()
                            } else {
                                // IndexOutOfBoundsException
                                Value::Invalid
                            }
                        }
                    }
                    _ => Value::Invalid,
                }
            }
            LRemove => match &s {
                Value::List(l) => {
                    if !a1.is_invalid() {
                        l.borrow_mut().items.retain(|x| !java_equals(x, &a1, env));
                    }
                    s.clone()
                }
                _ => Value::Invalid,
            },
            LRemoveAll => match (&s, self.as_coll(&a1)) {
                (Value::List(l), Some(d)) => {
                    l.borrow_mut().items.retain(|x| !d.contains(x, env));
                    s.clone()
                }
                _ => Value::Invalid,
            },
            LRemoveAt => match (&s, index_arg(&a1)) {
                (Value::List(l), Some(i)) => {
                    let mut lb = l.borrow_mut();
                    if i >= 1 && (i as usize) <= lb.items.len() {
                        lb.items.remove(i as usize - 1)
                    } else {
                        Value::Invalid
                    }
                }
                _ => Value::Invalid,
            },
            LRemoveFirst | LRemoveLast => match &s {
                Value::List(l) => {
                    let mut lb = l.borrow_mut();
                    if lb.items.is_empty() {
                        Value::Invalid
                    } else if prim == LRemoveFirst {
                        lb.items.remove(0)
                    } else {
                        lb.items.pop().unwrap()
                    }
                }
                _ => Value::Invalid,
            },
            LJoinfields => {
                let items = Iter::of(&s).items;
                let sep = args.get(1).cloned().unwrap_or(Value::Null);
                let begin = args.get(2).cloned().unwrap_or(Value::Null);
                let end = args.get(3).cloned().unwrap_or(Value::Null);
                let mut out = String::new();
                out.push_str(&self.to_java_string(&begin, false));
                for (i, x) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(&self.to_java_string(&sep, false));
                    }
                    out.push_str(&self.to_java_string(x, false));
                }
                out.push_str(&self.to_java_string(&end, false));
                Value::str(&out)
            }
            LAt => match index_arg(&a1) {
                Some(i) => self.coll_op(&s, |c| ops::at(c, i)),
                None => Value::Invalid,
            },
            LClone => match &s {
                Value::List(l) => Value::List(Rc::new(RefCell::new(l.borrow().clone()))),
                _ => Value::Invalid,
            },
            // ---- Dict ----
            DGet => match &s {
                Value::Dict(d) => d.borrow().get(&a1, env).cloned().unwrap_or(Value::Null),
                _ => Value::Invalid,
            },
            DDefaultGet => match &s {
                Value::Dict(d) => d
                    .borrow()
                    .get(&a1, env)
                    .cloned()
                    .unwrap_or_else(|| args.get(2).cloned().unwrap_or(Value::Null)),
                _ => Value::Invalid,
            },
            DPut => {
                let v = args.get(2).cloned().unwrap_or(Value::Null);
                if let Value::Dict(d) = &s
                    && !v.is_invalid()
                {
                    d.borrow_mut().put(a1, v, env);
                }
                Value::Null
            }
            DHasKey => match &s {
                Value::Dict(d) => Value::Bool(d.borrow().has_key(&a1, env)),
                _ => Value::Invalid,
            },
            DClear => {
                if let Value::Dict(d) = &s {
                    d.borrow_mut().clear();
                }
                Value::Null
            }
            DKeys | DValues => match &s {
                Value::Dict(d) => {
                    let items = if prim == DKeys {
                        d.borrow().keys()
                    } else {
                        d.borrow().values()
                    };
                    Value::List(Rc::new(RefCell::new(List::from_vec(items))))
                }
                _ => Value::Invalid,
            },
            // ---- Element ----
            EContainer => match &s {
                Value::Obj(r) if !r.is_local() => {
                    match self.store.container(S::Obj::from_u32(r.index())) {
                        Some((p, _)) => Value::Obj(ObjRef::host(p.to_u32())),
                        None => Value::Null,
                    }
                }
                Value::Obj(_) => Value::Null,
                _ => Value::Invalid,
            },
            EMetaClassName => match self.class_of(&s) {
                Some(c) => Value::str(&self.prog.ir.meta.class(c).name),
                None => Value::Invalid,
            },
            ESubobjects | ESubobjectsOfType | ESubobjectsOfKind | EAllSubobjects
            | EAllSubobjectsOfType | EAllSubobjectsOfKind => {
                let all = matches!(
                    prim,
                    EAllSubobjects | EAllSubobjectsOfType | EAllSubobjectsOfKind
                );
                let filter = match prim {
                    ESubobjects | EAllSubobjects => None,
                    _ => match type_arg_of(&a1, &self.object_types) {
                        Some(t) => {
                            Some((t, matches!(prim, ESubobjectsOfKind | EAllSubobjectsOfKind)))
                        }
                        None => return Ok(Value::Invalid),
                    },
                };
                let Value::Obj(r) = s else {
                    return Ok(Value::Invalid);
                };
                let objs: Vec<ObjRef> = if r.is_local() {
                    Vec::new()
                } else {
                    let h = S::Obj::from_u32(r.index());
                    let v = if all {
                        self.store.all_contents(h)
                    } else {
                        self.store.contents(h)
                    };
                    v.into_iter().map(|o| ObjRef::host(o.to_u32())).collect()
                };
                let mut set = Coll::new_set(&self.env);
                for o in objs {
                    let v = Value::Obj(o);
                    let ok = match filter {
                        None => true,
                        Some((t, true)) => self.is_kind_of(&v, t),
                        Some((t, false)) => self.ocl_is_type_of(&v, t),
                    };
                    if ok {
                        set.add(v, &self.env);
                    }
                }
                Value::Coll(Rc::new(set))
            }
            EClone | EDeepclone => match &s {
                Value::Obj(r) => self.copy_object(*r, prim == EDeepclone)?,
                _ => Value::Invalid,
            },
            ExGetArgument => match &s {
                Value::Obj(r) if r.is_local() => match &self.local[r.index() as usize].exception {
                    Some((Some(m), _)) => Value::str(m),
                    _ => Value::Null,
                },
                _ => Value::Null,
            },
            // ---- Model ----
            MObjects | MObjectsOfType | MObjectsOfKind | MAllInstances => {
                let Value::Model(x) = s else {
                    return Ok(Value::Invalid);
                };
                let filter = match prim {
                    MObjects => None,
                    _ => match type_arg_of(&a1, &self.object_types) {
                        Some(t) => Some((t, prim != MObjectsOfType)),
                        None => return Ok(Value::Invalid),
                    },
                };
                let objs = self.extents[x as usize].all_objects(self.store);
                let mut c = Coll::new_ordered_set();
                for o in objs {
                    let v = Value::Obj(o);
                    let ok = match filter {
                        None => true,
                        Some((t, true)) => self.is_kind_of(&v, t),
                        Some((t, false)) => self.ocl_is_type_of(&v, t),
                    };
                    if ok {
                        c.add(v, &self.env);
                    }
                }
                Value::Coll(Rc::new(c))
            }
            MRootObjects => {
                let Value::Model(x) = s else {
                    return Ok(Value::Invalid);
                };
                self.extents[x as usize].purge(self.store);
                let roots = self.extents[x as usize].root_objects(self.store);
                let mut c = Coll::new_ordered_set();
                for o in roots {
                    c.add(Value::Obj(o), &self.env);
                }
                Value::Coll(Rc::new(c))
            }
            MRemoveElement => {
                let (Value::Model(x), Value::Obj(r)) = (&s, &a1) else {
                    return Ok(Value::Invalid);
                };
                self.remove_element(*x as usize, *r);
                Value::Null
            }
            TTransform => return self.transform(&s),
            StSucceeded | StFailed | StRaisedException => match &s {
                Value::Obj(r) if r.is_local() => match &self.local[r.index() as usize].status {
                    Some((ok, exc)) => match prim {
                        StSucceeded => Value::Bool(*ok),
                        StFailed => Value::Bool(!*ok),
                        _ => exc.clone(),
                    },
                    None => Value::Invalid,
                },
                _ => Value::Invalid,
            },
            MCopy => return self.model_copy(&s),
            MCreateEmptyModel => return self.create_empty_model(&s),
            Unsupported => {
                return Err(
                    self.runtime_error(format!("operation '{}' is not supported", op.desc().name))
                );
            }
        })
    }

    /// Applies a function to a collection value (`List` and `Dict` viewed as collections).
    fn coll_op(&self, v: &Value, f: impl FnOnce(&Coll) -> Value) -> Value {
        match self.as_coll(v) {
            Some(c) => f(&c),
            None => Value::Invalid,
        }
    }

    fn as_coll(&self, v: &Value) -> Option<Rc<Coll>> {
        match v {
            Value::Coll(c) => Some(c.clone()),
            Value::List(l) => Some(Rc::new(Coll::Sequence(l.borrow().items.clone()))),
            Value::Dict(d) => Some(Rc::new(Coll::Sequence(d.borrow().values()))),
            _ => None,
        }
    }

    /// `Classifier::allInstances()` in a transformation: every object of every model
    /// parameter extent and the unbound extent that is of the kind, in a `HashSet`.
    /// `QVTExtentMap.collectInstances`: the extents of the current transformation's model
    /// parameters and the unbound extent (not those of `Model::copy()`/`createEmptyModel()`).
    fn all_instances(&mut self, t: TypeId) -> Value {
        let mut set = Coll::new_set(&self.env);
        let m = self.cur_transf;
        let n = self.prog.ir.modules[m.index()].model_params.len();
        let mut xs: Vec<usize> = (0..n)
            .filter_map(|i| self.param_extents.get(&(m.0, i as u32)).copied())
            .collect();
        xs.push(self.unbound_extent);
        for i in xs {
            let objs = self.extents[i].all_objects(self.store);
            for o in objs {
                let v = Value::Obj(o);
                if self.is_kind_of(&v, t) {
                    set.add(v, &self.env);
                }
            }
        }
        Value::Coll(Rc::new(set))
    }

    /// `Model::removeElement`: `EcoreUtil`-style delete within the extent.
    fn remove_element(&mut self, x: usize, r: ObjRef) {
        self.extents[x].purge(self.store);
        if !r.is_local() {
            self.store.delete(S::Obj::from_u32(r.index()));
        }
        let e = &mut self.extents[x];
        if let Some(i) = e.initial.iter().position(|o| *o == r) {
            e.initial.remove(i);
        } else if let Some(i) = e.additional.iter().position(|o| *o == r) {
            e.additional.remove(i);
        }
        e.forget_resource_addition(r);
    }

    /// `sortedBy`: elements sorted by their keys (stable; the last key of equal elements
    /// wins, as Eclipse keys a `HashMap` by element); keys of different classes or not
    /// comparable give invalid.
    pub(crate) fn sort_by(
        &self,
        elems: Value,
        keys: Value,
        ordered_set: bool,
        unlimited: bool,
    ) -> Value {
        let es = Iter::of(&elems).items;
        let ks = Iter::of(&keys).items;
        // last key per equal element (Eclipse keys a HashMap by element); elements of a set
        // are unique
        let mut eff: Vec<Value> = ks.to_vec();
        if !matches!(&elems, Value::Coll(c) if c.kind().is_set()) {
            match es.iter().map(atom_hash).collect::<Option<Vec<u64>>>() {
                Some(hs) => {
                    // from the end: the first element of each class of equal elements is its
                    // last occurrence; classes with the same hash are chained
                    let n = es.len();
                    let mut head: qvto_runtime::fxhash::FxHashMap<u64, u32> =
                        qvto_runtime::fxhash::FxHashMap::with_capacity_and_hasher(
                            n,
                            Default::default(),
                        );
                    let mut next: Vec<u32> = vec![u32::MAX; n];
                    let mut last: Vec<u32> = vec![0; n];
                    for j in (0..n).rev() {
                        let mut r = head.get(&hs[j]).copied().unwrap_or(u32::MAX);
                        while r != u32::MAX && !java_equals(&es[r as usize], &es[j], &self.env) {
                            r = next[r as usize];
                        }
                        if r == u32::MAX {
                            last[j] = j as u32;
                            next[j] = head.insert(hs[j], j as u32).unwrap_or(u32::MAX);
                        } else {
                            last[j] = last[r as usize];
                        }
                    }
                    for i in 0..n {
                        let j = last[i] as usize;
                        if j != i {
                            eff[i] = ks[j].clone();
                        }
                    }
                }
                None => {
                    for i in 0..es.len() {
                        for j in (i + 1..es.len()).rev() {
                            if java_equals(&es[i], &es[j], &self.env) {
                                eff[i] = ks[j].clone();
                                break;
                            }
                        }
                    }
                }
            }
        }
        // the keys' Java classes: `Comparable.compareTo` fails across classes
        #[derive(PartialEq)]
        enum K {
            Int,
            Long,
            Short,
            Byte,
            Real,
            Float,
            Str,
            Bool,
            Enum,
        }
        let class = |v: &Value| -> Option<K> {
            Some(match v {
                Value::Int(i) => {
                    if *i as i32 as i64 == *i {
                        K::Int
                    } else {
                        K::Long
                    }
                }
                Value::Boxed(IntBox::Long, _) => K::Long,
                Value::Boxed(IntBox::Short, _) => K::Short,
                Value::Boxed(IntBox::Byte, _) => K::Byte,
                Value::Real(_) => K::Real,
                Value::Float(_) => K::Float,
                Value::Str(_) => K::Str,
                Value::Bool(_) => K::Bool,
                Value::Enum(_) => K::Enum,
                _ => return None,
            })
        };
        let Some(first) = eff.first() else {
            return self.sorted_result(Vec::new(), ordered_set);
        };
        let c0 = class(first);
        if (c0.is_none() || !eff.iter().all(|k| class(k) == c0))
            && !(unlimited
                && eff
                    .iter()
                    .all(|k| matches!(class(k), Some(K::Int) | Some(K::Long))))
        {
            return Value::Invalid;
        }
        let mut idx: Vec<usize> = (0..es.len()).collect();
        let cmp = |a: &Value, b: &Value| -> std::cmp::Ordering {
            use std::cmp::Ordering::*;
            if unlimited {
                let ua = matches!(a, Value::Int(-1));
                let ub = matches!(b, Value::Int(-1));
                match (ua, ub) {
                    (true, true) => return Equal,
                    (true, false) => return Greater,
                    (false, true) => return Less,
                    _ => {}
                }
            }
            let num = |v: &Value| match v {
                Value::Int(i) => Some(*i),
                Value::Boxed(IntBox::Long | IntBox::Short | IntBox::Byte, i) => Some(*i as i64),
                _ => None,
            };
            if let (Some(x), Some(y)) = (num(a), num(b)) {
                return x.cmp(&y);
            }
            match (a, b) {
                (Value::Float(x), Value::Float(y)) => {
                    qvto_runtime::java::hash::double_compare(*x as f64, *y as f64)
                }
                (Value::Real(x), Value::Real(y)) => {
                    qvto_runtime::java::hash::double_compare(*x, *y)
                }
                (Value::Str(x), Value::Str(y)) => strings::compare(x, y),
                (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
                (Value::Enum(x), Value::Enum(y)) => x.literal.cmp(&y.literal),
                _ => Equal,
            }
        };
        idx.sort_by(|&i, &j| cmp(&eff[i], &eff[j]));
        let sorted: Vec<Value> = idx.into_iter().map(|i| es[i].clone()).collect();
        self.sorted_result(sorted, ordered_set)
    }

    fn sorted_result(&self, v: Vec<Value>, ordered_set: bool) -> Value {
        if ordered_set {
            Value::Coll(Rc::new(Coll::from_values(
                CollKind::OrderedSet,
                v,
                &self.env,
            )))
        } else {
            Value::Coll(Rc::new(Coll::Sequence(v)))
        }
    }
}

/// A hash consistent with `java_equals` for atoms, without drawing identity hashes (identity-
/// hashed values hash by their identity key); `None` for collections, tuples, dictionaries.
fn atom_hash(v: &Value) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let mut h = qvto_runtime::fxhash::FxHasher::default();
    std::mem::discriminant(v).hash(&mut h);
    match v {
        Value::Null | Value::Invalid => {}
        Value::Bool(b) => b.hash(&mut h),
        Value::Int(i) => i.hash(&mut h),
        Value::Boxed(_, i) => i.hash(&mut h),
        Value::Real(d) => qvto_runtime::java::hash::double_to_long_bits(*d).hash(&mut h),
        Value::Float(f) => qvto_runtime::java::hash::float_to_int_bits(*f).hash(&mut h),
        Value::Str(s) => s.java_hash().hash(&mut h),
        Value::Obj(_) | Value::Enum(_) | Value::Type(_) | Value::Model(_) | Value::Instance(_) => {
            qvto_runtime::eq::identity_key(v)?.hash(&mut h)
        }
        _ => return None,
    }
    Some(h.finish())
}

trait OrValue {
    fn or<'a>(&'a self, other: &'a Value) -> &'a Value;
}

impl OrValue for Value {
    /// The argument of a static operation called with or without a receiver.
    fn or<'a>(&'a self, other: &'a Value) -> &'a Value {
        if self.is_null() { other } else { self }
    }
}

fn str_fn(s: &Value, f: impl FnOnce(&Str) -> Value) -> Value {
    match s {
        Value::Str(x) => f(x),
        _ => Value::Invalid,
    }
}

fn str2(s: &Value, a: &Value, f: impl FnOnce(&Str, &Str) -> Value) -> Value {
    match (s, a) {
        (Value::Str(x), Value::Str(y)) => f(x, y),
        _ => Value::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operation_has_a_primitive() {
        let mut missing = Vec::new();
        for &op in StdOp::all() {
            let d = op.desc();
            if prim_of(op) == Prim::Unsupported && d.unsupported.is_none() {
                missing.push(format!("{op:?}"));
            }
        }
        // not implemented by this engine version (and not by Eclipse's OclMessage either)
        missing.retain(|m| {
            !m.starts_with("OclMessage_")
                && !m.starts_with("Status_")
                && !m.starts_with("Transformation_")
        });
        assert!(missing.is_empty(), "no primitive: {missing:?}");
    }
}

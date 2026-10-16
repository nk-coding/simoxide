//! The standard library: classic OCL's predefined operations and QVTo's library, as Eclipse
//! QVTo 3.11 defines them. The table is generated from a dump of Eclipse's own library objects
//! (`data/stdlib.tsv`, `tools/oracle/StdlibDump.java`, `examples/gen_stdlib.rs`); nothing in it is
//! written by hand.
//!
//! - [`StdOp`] names one operation; the VM dispatches on it. Classic OCL operations
//!   ([`Section::Ocl`]) are shared between owners when their signature is identical (classic
//!   OCL evaluates them by name on the source value); QVTo's operations ([`Section::Qvto`],
//!   [`Section::Class`]) each have their own Java handler in Eclipse and their own variant here.
//! - [`StdOwner`] is the generic owner type an operation is listed under; [`owner_ops`] gives the
//!   operations of an owner in Eclipse's order, which decides overload resolution ties.
//! - Signature types are kept as Eclipse prints them (`Set(oclstdlib::T)`,
//!   `TypeType(oclstdlib::T)`, `Stdlib::Element`) and interned by [`StdLib::sig_type`].

use crate::ty::{CollKind, Type, TypeId, TypeTable};
use qvto_model::{ClassId, MetaRegistry, PackageId};

#[path = "stdlib_data.rs"]
#[rustfmt::skip]
mod data;

pub use data::StdOp;

/// Where Eclipse defines an operation.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Section {
    /// `PredefinedType.oclOperations()` of a classic OCL type.
    Ocl,
    /// An additional operation of the QVTo library environment on an OCL type (a Java handler).
    Qvto,
    /// An operation owned by a QVTo library class (`Element`, `Model`, ...) or by the library
    /// module itself.
    Class,
}

/// The owner an operation is listed under.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum StdOwner {
    OclAny,
    OclVoid,
    OclInvalid,
    Boolean,
    Integer,
    Real,
    String,
    UnlimitedNatural,
    OclMessage,
    /// Classic OCL's `TypeType` (operations on type expressions, e.g. `allInstances`).
    OclType,
    Collection,
    Set,
    OrderedSet,
    Bag,
    Sequence,
    List,
    Dict,
    Element,
    Exception,
    Model,
    Status,
    Transformation,
    /// The QVTo library module `Stdlib` (non-contextual operations such as `dump(x)`).
    Stdlib,
}

/// One operation of the table.
#[derive(Clone, Debug)]
pub struct StdOpDesc {
    pub op: StdOp,
    pub section: Section,
    pub owner: StdOwner,
    pub name: &'static str,
    /// Parameter names and signature types.
    pub params: &'static [(&'static str, &'static str)],
    pub ret: &'static str,
    pub is_static: bool,
    pub deprecated: bool,
    pub deprecated_by: Option<&'static str>,
    /// Eclipse declares the operation but does not implement it; the text is Eclipse's reason.
    pub unsupported: Option<&'static str>,
}

impl StdOp {
    #[inline]
    pub fn desc(self) -> &'static StdOpDesc {
        &data::DESCS[self as usize]
    }
    #[inline]
    pub fn name(self) -> &'static str {
        self.desc().name
    }
    pub fn all() -> &'static [StdOp] {
        data::ALL
    }
    pub const COUNT: usize = data::COUNT;
}

/// The operations listed under an owner in a section, in Eclipse's order.
pub fn owner_ops(section: Section, owner: StdOwner) -> &'static [StdOp] {
    data::OWNER_OPS
        .iter()
        .find(|(s, o, _)| *s == section && *o == owner)
        .map(|(_, _, l)| *l)
        .unwrap_or(&[])
}

/// The owners of a section in Eclipse's definition order.
pub fn owners(section: Section) -> impl Iterator<Item = StdOwner> {
    data::OWNER_OPS
        .iter()
        .filter(move |(s, _, _)| *s == section)
        .map(|(_, o, _)| *o)
}

/// QVTo's library classes, registered as classes of the program's `MetaRegistry` in the
/// package `Stdlib` (nsURI `http://www.eclipse.org/m2m/qvt/oml/1.0.0/Stdlib`), in Eclipse's
/// order.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StdClasses {
    pub package: PackageId,
    pub element: ClassId,
    pub exception: ClassId,
    pub assertion_failed: ClassId,
    pub model: ClassId,
    pub object: ClassId,
    pub status: ClassId,
    pub transformation: ClassId,
}

pub const STDLIB_NS_URI: &str = "http://www.eclipse.org/m2m/qvt/oml/1.0.0/Stdlib";

impl StdClasses {
    /// Registers the library classes (once per registry: an existing `Stdlib` package is reused).
    pub fn register(meta: &mut MetaRegistry) -> StdClasses {
        if let Some(p) = meta.package_by_ns_uri(STDLIB_NS_URI) {
            let c = |n: &str| meta.class_by_name(p, n).expect("Stdlib class");
            return StdClasses {
                package: p,
                element: c("Element"),
                exception: c("Exception"),
                assertion_failed: c("AssertionFailed"),
                model: c("Model"),
                object: c("Object"),
                status: c("Status"),
                transformation: c("Transformation"),
            };
        }
        let p = meta.add_package("Stdlib", Some(STDLIB_NS_URI), Some("Stdlib"), None);
        let element = meta.add_class(p, "Element", true, false);
        let exception = meta.add_class(p, "Exception", false, false);
        let assertion_failed = meta.add_class(p, "AssertionFailed", false, false);
        meta.class_mut(assertion_failed).supers.push(exception);
        let model = meta.add_class(p, "Model", false, false);
        let object = meta.add_class(p, "Object", true, false);
        let status = meta.add_class(p, "Status", false, false);
        let transformation = meta.add_class(p, "Transformation", true, false);
        meta.finish_classes().expect("Stdlib classes");
        meta.register_ns_uri(p);
        StdClasses {
            package: p,
            element,
            exception,
            assertion_failed,
            model,
            object,
            status,
            transformation,
        }
    }

    pub fn by_name(&self, name: &str) -> Option<ClassId> {
        Some(match name {
            "Element" => self.element,
            "Exception" => self.exception,
            "AssertionFailed" => self.assertion_failed,
            "Model" => self.model,
            "Object" => self.object,
            "Status" => self.status,
            "Transformation" => self.transformation,
            _ => return None,
        })
    }
}

/// The standard library bound to one type table: signature types interned.
#[derive(Clone, Debug)]
pub struct StdLib {
    pub classes: StdClasses,
    params: Vec<Box<[TypeId]>>,
    rets: Vec<TypeId>,
}

impl StdLib {
    /// Interns every signature type of the table into `types`.
    pub fn new(types: &mut TypeTable, classes: StdClasses) -> StdLib {
        let mut params = Vec::with_capacity(StdOp::COUNT);
        let mut rets = Vec::with_capacity(StdOp::COUNT);
        for &op in StdOp::all() {
            let d = op.desc();
            let ps: Box<[TypeId]> = d
                .params
                .iter()
                .map(|(_, t)| parse_sig_type(types, &classes, t))
                .collect();
            params.push(ps);
            rets.push(parse_sig_type(types, &classes, d.ret));
        }
        StdLib {
            classes,
            params,
            rets,
        }
    }

    /// Parameter types of an operation (with template parameters).
    #[inline]
    pub fn params(&self, op: StdOp) -> &[TypeId] {
        &self.params[op as usize]
    }

    /// Declared return type of an operation (with template parameters).
    #[inline]
    pub fn ret(&self, op: StdOp) -> TypeId {
        self.rets[op as usize]
    }

    /// The type an owner stands for, with template parameters (`Set(T)`).
    pub fn owner_type(&self, types: &mut TypeTable, owner: StdOwner) -> TypeId {
        let t = TypeId::T;
        match owner {
            StdOwner::OclAny => TypeId::OCL_ANY,
            StdOwner::OclVoid => TypeId::OCL_VOID,
            StdOwner::OclInvalid => TypeId::OCL_INVALID,
            StdOwner::Boolean => TypeId::BOOLEAN,
            StdOwner::Integer => TypeId::INTEGER,
            StdOwner::Real => TypeId::REAL,
            StdOwner::String => TypeId::STRING,
            StdOwner::UnlimitedNatural => TypeId::UNLIMITED_NATURAL,
            StdOwner::OclMessage => types.intern(Type::OclMessage),
            StdOwner::OclType => types.type_type(t),
            StdOwner::Collection => types.collection(CollKind::Collection, t),
            StdOwner::Set => types.collection(CollKind::Set, t),
            StdOwner::OrderedSet => types.collection(CollKind::OrderedSet, t),
            StdOwner::Bag => types.collection(CollKind::Bag, t),
            StdOwner::Sequence => types.collection(CollKind::Sequence, t),
            StdOwner::List => types.list(t),
            StdOwner::Dict => types.dict(TypeId::KEY_T, t),
            StdOwner::Element => types.class(self.classes.element),
            StdOwner::Exception => types.class(self.classes.exception),
            StdOwner::Model => types.class(self.classes.model),
            StdOwner::Status => types.class(self.classes.status),
            StdOwner::Transformation => types.class(self.classes.transformation),
            StdOwner::Stdlib => TypeId::OCL_VOID,
        }
    }
}

/// Parses a signature type as printed by the dump.
pub fn parse_sig_type(types: &mut TypeTable, classes: &StdClasses, s: &str) -> TypeId {
    let s = s.trim();
    if let Some((head, rest)) = s.split_once('(') {
        let inner = rest.strip_suffix(')').expect("signature type");
        let args = split_args(inner);
        let kind = match head {
            "Collection" => Some(CollKind::Collection),
            "Set" => Some(CollKind::Set),
            "OrderedSet" => Some(CollKind::OrderedSet),
            "Bag" => Some(CollKind::Bag),
            "Sequence" => Some(CollKind::Sequence),
            _ => None,
        };
        if let Some(k) = kind {
            let e = parse_sig_type(types, classes, args[0]);
            return types.collection(k, e);
        }
        return match head {
            "List" => {
                let e = parse_sig_type(types, classes, args[0]);
                types.list(e)
            }
            "Dict" => {
                let k = parse_sig_type(types, classes, args[0]);
                let v = parse_sig_type(types, classes, args[1]);
                types.dict(k, v)
            }
            "TypeType" => {
                let e = parse_sig_type(types, classes, args[0]);
                types.type_type(e)
            }
            "Tuple" => {
                let parts = args
                    .iter()
                    .map(|a| {
                        let (n, t) = a.split_once(':').expect("tuple part");
                        (n.into(), parse_sig_type(types, classes, t))
                    })
                    .collect();
                types.tuple(parts)
            }
            _ => panic!("unknown signature type {s}"),
        };
    }
    match s {
        "oclstdlib::OclAny" => TypeId::OCL_ANY,
        "oclstdlib::OclVoid" => TypeId::OCL_VOID,
        "oclstdlib::OclInvalid" => TypeId::OCL_INVALID,
        "oclstdlib::Boolean" => TypeId::BOOLEAN,
        "oclstdlib::Integer" => TypeId::INTEGER,
        "oclstdlib::Real" => TypeId::REAL,
        "oclstdlib::String" => TypeId::STRING,
        "oclstdlib::UnlimitedNatural" => TypeId::UNLIMITED_NATURAL,
        "oclstdlib::OclType" => TypeId::OCL_TYPE,
        "oclstdlib::OclElement" => TypeId::OCL_ELEMENT,
        "oclstdlib::State" => types.intern(Type::State),
        "oclstdlib::OclMessage" => types.intern(Type::OclMessage),
        "oclstdlib::T" => TypeId::T,
        "oclstdlib::T2" => TypeId::T2,
        "Stdlib::KeyT" => TypeId::KEY_T,
        "Stdlib::CommonT" => TypeId::COMMON_T,
        _ => {
            let name = s.strip_prefix("Stdlib::").expect("signature type");
            let c = classes.by_name(name).expect("Stdlib class");
            types.class(c)
        }
    }
}

fn split_args(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                out.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_parses() {
        let mut meta = MetaRegistry::new();
        let classes = StdClasses::register(&mut meta);
        let mut types = TypeTable::new();
        let lib = StdLib::new(&mut types, classes);
        let inc = owner_ops(Section::Qvto, StdOwner::Set)
            .iter()
            .copied()
            .find(|o| o.name() == "including")
            .unwrap();
        assert_eq!(lib.params(inc), &[TypeId::T2]);
        assert!(!owner_ops(Section::Ocl, StdOwner::String).is_empty());
        assert_eq!(StdClasses::register(&mut meta), classes);
    }
}

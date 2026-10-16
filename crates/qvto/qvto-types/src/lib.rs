//! The OCL and QVTo type system, the standard library signatures (one table shared by type
//! checking and the VM) and blackbox signature descriptors.
//!
//! - [`ty`]: interned types ([`TypeId`], [`Type`], [`TypeTable`]).
//! - [`stdlib`]: the standard library table ([`StdOp`]), generated from Eclipse's library, and
//!   QVTo's library classes ([`StdClasses`]).

pub mod check;
pub mod stdlib;
pub mod ty;

pub use check::{Checker, ModelTypeInfo, ModuleTypeInfo, SuperTypeError, rel};
pub use stdlib::{Section, StdClasses, StdLib, StdOp, StdOpDesc, StdOwner};
pub use ty::{CollKind, TParam, Type, TypeId, TypeTable};

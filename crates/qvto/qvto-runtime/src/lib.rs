//! Runtime values (OCL values incl. null and invalid, collections, tuples, Dict and List),
//! Java-compatible hashing and formatting for compat mode, the standard library
//! implementations, and the blackbox trait.

pub mod coll;
pub mod dict;
pub mod eq;
pub mod identity;
pub mod java;
pub mod str;
pub mod tuple;
pub mod value;

pub use coll::{Coll, CollKind};
pub use identity::{CompatEnv, FastEnv, HashEnv, IdentityKey, Mode};
pub use str::Str;
pub use value::{EnumLit, IntBox, ObjRef, TypeRef, Value};

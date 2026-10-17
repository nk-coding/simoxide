//! Runtime values (OCL values incl. null and invalid, collections, tuples, Dict and List),
//! Java-compatible hashing and formatting for compat mode, the standard library
//! implementations, and the blackbox trait. See the crate README.
//!
//! ```
//! use qvto_runtime::{coll::ops, CompatEnv, Coll, Value};
//! let env = CompatEnv::default();
//! let mut s = Coll::new_set(&env); // java.util.HashSet
//! for i in [17, 1, 33] {
//!     s.add(Value::int(i), &env);
//! }
//! let v = ops::qvto_including(s.into(), Value::str("x"), &env);
//! assert_eq!(qvto_runtime::display::to_java_string(&v, &env), "[17, 1, 33, x]");
//! ```

pub mod coll;
pub mod dict;
pub mod display;
pub mod eq;
pub mod fxhash;
pub mod identity;
pub mod java;
pub mod model;
pub mod num;
pub mod str;
pub mod strings;
pub mod tuple;
pub mod value;

pub use coll::{Coll, CollKind};
#[cfg(feature = "compat")]
pub use identity::CompatEnv;
pub use identity::{FastEnv, HashEnv, IdentityKey, Mode};
pub use str::Str;
pub use value::{EnumLit, IntBox, ObjRef, TypeRef, Value};

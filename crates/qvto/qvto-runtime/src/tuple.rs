//! OCL tuples (`AbstractTuple` of classic OCL).

use crate::eq::{java_equals, java_hash};
use crate::identity::{HashEnv, IdentityKey};
use crate::str::Str;
use crate::value::Value;
use std::rc::Rc;

/// A tuple: its tuple type and the part values in the type's property order.
///
/// Tuple types are deduplicated by part names and types (`TypeResolver.findTupleType`), so two
/// tuples with the same parts in any order have the same `type_id`. Java equality is "same
/// type and equal parts"; the hash is `37 * identityHash(type) + 17 * parts.hashCode()`, where
/// `parts` is a `HashMap<String, Object>`.
#[derive(Clone, Debug)]
pub struct Tuple {
    pub type_id: u32,
    /// Part names in the type's `oclProperties()` order (also the `toString` order).
    pub names: Rc<[Str]>,
    pub values: Box<[Value]>,
}

impl Tuple {
    pub fn new(type_id: u32, names: Rc<[Str]>, values: Box<[Value]>) -> Tuple {
        assert_eq!(names.len(), values.len());
        Tuple { type_id, names, values }
    }

    /// The value of a part, by name.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.names.iter().position(|n| n.as_str() == Some(name)).map(|i| &self.values[i])
    }

    pub fn java_hash(&self, env: &dyn HashEnv) -> i32 {
        let ty = env.identity_hash(IdentityKey::TupleType(self.type_id));
        let parts = self
            .names
            .iter()
            .zip(self.values.iter())
            .fold(0i32, |h, (n, v)| h.wrapping_add(n.java_hash() ^ java_hash(v, env)));
        37i32.wrapping_mul(ty).wrapping_add(17i32.wrapping_mul(parts))
    }

    pub fn java_equals(&self, other: &Tuple, env: &dyn HashEnv) -> bool {
        self.type_id == other.type_id
            && self.values.len() == other.values.len()
            && self.values.iter().zip(other.values.iter()).all(|(a, b)| java_equals(a, b, env))
    }
}

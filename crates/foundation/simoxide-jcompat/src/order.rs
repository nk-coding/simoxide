//! Iteration orders of Java hash-based collections, as far as a mode needs them.
//!
//! A [`MapOrder`] gives the iteration order of a map whose keys (with their `hashCode`s) were
//! inserted in slice order and never removed. [`InsertionOrder`] is the order of a
//! `LinkedHashMap` (javadoc) and the order the fast modes use for every map; the order of a plain
//! `java.util.HashMap` is `JdkHashOrder` in `simoxide-jcompat-exact` (exact and compat modes).

/// The iteration order of an insert-only map, as indices into the keys' hashes.
pub trait MapOrder {
    fn order(hashes: &[i32]) -> Vec<usize>;
}

/// First-insertion order (`LinkedHashMap`; the fast modes).
#[derive(Clone, Copy, Debug, Default)]
pub struct InsertionOrder;

impl MapOrder for InsertionOrder {
    #[inline]
    fn order(hashes: &[i32]) -> Vec<usize> {
        (0..hashes.len()).collect()
    }
}

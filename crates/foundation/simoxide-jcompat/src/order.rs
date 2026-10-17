//! Iteration orders of Java hash-based collections, as far as a mode needs them.
//!
//! A [`MapOrder`] gives the iteration order of a map whose keys (with their `hashCode`s) were
//! inserted in slice order and never removed. [`InsertionOrder`] is the order of a
//! `LinkedHashMap` (javadoc) and the order the fast modes use for every map; the order of a plain
//! `java.util.HashMap` is `JdkHashOrder` in `simoxide-jcompat-exact` (exact and compat modes).

/// The iteration order of an insert-only map, as indices into the keys' hashes.
pub trait MapOrder {
    /// The indices of the keys in iteration order.
    fn order(hashes: &[i32]) -> Vec<usize>;
    /// The sort key of a key with `hash` in a table with index mask `mask` (`capacity - 1`):
    /// keys iterate by ascending sort key, equal keys in first-insertion order. For callers that
    /// sort small key sets themselves (no allocation); [`MapOrder::order`] agrees with it.
    fn bucket(hash: i32, mask: u32) -> u32;
}

/// First-insertion order (`LinkedHashMap`; the fast modes).
#[derive(Clone, Copy, Debug, Default)]
pub struct InsertionOrder;

impl MapOrder for InsertionOrder {
    #[inline]
    fn order(hashes: &[i32]) -> Vec<usize> {
        (0..hashes.len()).collect()
    }
    #[inline(always)]
    fn bucket(_hash: i32, _mask: u32) -> u32 {
        0
    }
}

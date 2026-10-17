//! `java.util.HashMap`'s iteration order for maps whose keys are never removed, in closed form
//! (SimOxide's SEFF `INNER` characterisations and resource finalisation, spec ACT-5.7, ND-12,
//! ND-13).
//!
//! The order is a pure function of the key set, the first-insertion order of the keys and the
//! final size: buckets `spread(hash) & (C - 1)` in ascending order, keys of one bucket in
//! first-insertion order (resizes split buckets without reordering). The capacity `C` starts at
//! 16 and doubles whenever the size exceeds `0.75 * C`. Treeified bins (more than 8 keys in one
//! bucket at capacity >= 64) are not emulated; [`crate::hashmap::JavaHashMap`] has them.

use crate::hashmap::spread;
use simoxide_jcompat::order::MapOrder;

/// Table capacity of a default `HashMap` after inserting `n` distinct keys.
#[inline]
pub fn capacity_for(n: usize) -> usize {
    let mut cap = 16usize;
    // resize happens when ++size > threshold (0.75 * cap)
    while n > cap / 4 * 3 {
        cap *= 2;
    }
    cap
}

/// Iteration order of a `HashMap` whose keys (with precomputed `hashCode()`s) were first
/// inserted in slice order: the indices into `hashes` in iteration order.
pub fn iteration_order(hashes: &[i32]) -> Vec<usize> {
    let cap = capacity_for(hashes.len());
    let mask = (cap - 1) as u32;
    let mut idx: Vec<usize> = (0..hashes.len()).collect();
    // stable sort keeps first-insertion order inside a bucket
    idx.sort_by_key(|&i| spread(hashes[i]) as u32 & mask);
    idx
}

/// The order of a default `java.util.HashMap` (insert-only, no tree bins).
#[derive(Clone, Copy, Debug, Default)]
pub struct JdkHashOrder;

impl MapOrder for JdkHashOrder {
    #[inline]
    fn order(hashes: &[i32]) -> Vec<usize> {
        iteration_order(hashes)
    }
    #[inline(always)]
    fn bucket(hash: i32, mask: u32) -> u32 {
        let h = hash as u32;
        (h ^ (h >> 16)) & mask
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use simoxide_jcompat::hash::str_hash;

    #[test]
    fn capacity() {
        assert_eq!(capacity_for(0), 16);
        assert_eq!(capacity_for(12), 16);
        assert_eq!(capacity_for(13), 32);
        assert_eq!(capacity_for(24), 32);
        assert_eq!(capacity_for(25), 64);
    }

    #[test]
    fn order() {
        // "Aa" and "BB" collide (2112): insertion order inside the bucket
        let keys = ["BB", "Aa", "a"];
        let h: Vec<i32> = keys.iter().map(|k| str_hash(k)).collect();
        // bucket of "a" = 97 & 15 = 1; bucket of 2112 = (2112 ^ 0) & 15 = 0
        assert_eq!(iteration_order(&h), vec![0, 1, 2]);
    }
}

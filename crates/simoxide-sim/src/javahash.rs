//! Emulation of `java.util.HashMap<String, _>` iteration order (spec ACT-5.7, ND-12, ND-13).
//!
//! The order of a map whose keys are never removed is a pure function of the key set, the
//! first-insertion order of the keys and the final size: buckets `spread(hash) & (C - 1)` in
//! ascending order, keys of one bucket in first-insertion order (resizes split buckets without
//! reordering). The capacity `C` starts at 16 and doubles whenever the size exceeds `0.75 * C`.
//! Treeified bins (more than 8 keys in one bucket at capacity ≥ 64) are not emulated.

/// `String.hashCode()` over UTF-16 code units, with Java's wrapping arithmetic.
pub fn string_hash(s: &str) -> i32 {
    let mut h: i32 = 0;
    for u in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(i32::from(u));
    }
    h
}

/// `HashMap.hash(key)`: `h ^ (h >>> 16)`.
#[inline]
pub fn spread(h: i32) -> u32 {
    let h = h as u32;
    h ^ (h >> 16)
}

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

/// Iteration order of a `HashMap` whose keys (with precomputed `String.hashCode()`s) were first
/// inserted in slice order: returns the indices into `hashes` in iteration order.
pub fn iteration_order(hashes: &[i32]) -> Vec<usize> {
    let cap = capacity_for(hashes.len());
    let mask = (cap - 1) as u32;
    let mut idx: Vec<usize> = (0..hashes.len()).collect();
    // stable sort keeps first-insertion order inside a bucket
    idx.sort_by_key(|&i| spread(hashes[i]) & mask);
    idx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_codes() {
        assert_eq!(string_hash(""), 0);
        assert_eq!(string_hash("a"), 97);
        assert_eq!(string_hash("hello"), 99162322);
        assert_eq!(
            string_hash("a.VALUE"),
            "a.VALUE"
                .bytes()
                .fold(0i32, |h, b| h.wrapping_mul(31).wrapping_add(b as i32))
        );
        // wrapping
        assert_eq!(string_hash("polygenelubricants"), i32::MIN);
    }

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
        let h: Vec<i32> = keys.iter().map(|k| string_hash(k)).collect();
        let o = iteration_order(&h);
        // bucket of "a" = 97 & 15 = 1; bucket of 2112 = (2112 ^ 0) & 15 = 0
        assert_eq!(o, vec![0, 1, 2]);
    }
}

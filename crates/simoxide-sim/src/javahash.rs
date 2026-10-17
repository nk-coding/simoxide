//! Emulation of `java.util.HashMap<String, _>` iteration order (spec ACT-5.7, ND-12, ND-13):
//! `String.hashCode` from `simoxide-jcompat`, the bucket order of insert-only maps from
//! `simoxide-jcompat-exact` (`order`: buckets `spread(hash) & (C - 1)` ascending, first-insertion
//! order inside a bucket, capacity 16 doubling past `0.75 * C`; tree bins not emulated).

pub use simoxide_jcompat::hash::str_hash as string_hash;
pub use simoxide_jcompat_exact::order::{capacity_for, iteration_order};

/// `HashMap.hash(key)`: `h ^ (h >>> 16)`.
#[inline]
pub fn spread(h: i32) -> u32 {
    simoxide_jcompat_exact::hashmap::spread(h) as u32
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
        assert_eq!(spread(0x12345678), 0x12345678 ^ 0x1234);
        assert_eq!(spread(-1), 0xffff_0000);
    }
}

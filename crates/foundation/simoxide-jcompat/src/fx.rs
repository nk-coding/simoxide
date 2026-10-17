//! A fast, deterministic hasher for internal maps keyed by small integers and short strings
//! (object handles, feature ids, identity keys, IDs, URIs): the multiply-rotate hash of rustc's
//! `FxHasher`.
//! Not DoS-resistant, which does not matter for keys the engine makes itself; deterministic,
//! which DESIGN §5.5 asks for anyway.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

#[derive(Default, Clone, Copy)]
pub struct FxHasher {
    hash: u64,
}

impl FxHasher {
    #[inline]
    fn add(&mut self, w: u64) {
        self.hash = (self.hash.rotate_left(5) ^ w).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, mut b: &[u8]) {
        // 8-byte words, then one 4-byte word, then single bytes (the loader's variant: the
        // fastest on its ID and URI keys)
        while b.len() >= 8 {
            self.add(u64::from_le_bytes(b[..8].try_into().unwrap()));
            b = &b[8..];
        }
        if b.len() >= 4 {
            self.add(u32::from_le_bytes(b[..4].try_into().unwrap()) as u64);
            b = &b[4..];
        }
        for &x in b {
            self.add(x as u64);
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn write_i32(&mut self, i: i32) {
        self.add(i as u32 as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

pub type FxBuildHasher = BuildHasherDefault<FxHasher>;
pub type FxHashMap<K, V> = HashMap<K, V, FxBuildHasher>;
pub type FxHashSet<K> = HashSet<K, FxBuildHasher>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_spread() {
        let mut m: FxHashMap<u32, u32> = FxHashMap::default();
        for i in 0..10_000 {
            m.insert(i, i * 2);
        }
        assert_eq!(m.len(), 10_000);
        assert!((0..10_000).all(|i| m[&i] == i * 2));
        let h = |x: u64| {
            let mut s = FxHasher::default();
            s.write_u64(x);
            s.finish()
        };
        assert_eq!(h(1), h(1));
        assert_ne!(h(1), h(2));
    }
}

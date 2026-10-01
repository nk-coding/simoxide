//! A small, fast, deterministic hasher for the loader's internal maps (the `FxHasher` of rustc).
//! Keys are IDs, URIs and metamodel names from model files; hash flooding is not a concern here.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

#[derive(Default, Clone, Copy)]
pub(crate) struct FxHasher {
    hash: u64,
}

const SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    #[inline]
    fn add(&mut self, w: u64) {
        self.hash = (self.hash.rotate_left(5) ^ w).wrapping_mul(SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, mut b: &[u8]) {
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
    fn finish(&self) -> u64 {
        self.hash
    }
}

pub(crate) type FxBuild = BuildHasherDefault<FxHasher>;
pub(crate) type FxHashMap<K, V> = HashMap<K, V, FxBuild>;

/// Hash of a string key (the same for equal strings in every process).
#[inline]
pub(crate) fn hash_str(s: &str) -> u64 {
    let mut h = FxHasher::default();
    h.write(s.as_bytes());
    // final avalanche: hashbrown takes bucket bits from the low end
    let x = h.finish();
    (x ^ (x >> 29)).wrapping_mul(0xbf58_476d_1ce4_e5b9) ^ (x >> 32)
}

/// Passes a precomputed `u64` hash through.
#[derive(Default, Clone, Copy)]
pub(crate) struct IdentityHasher(u64);

impl Hasher for IdentityHasher {
    fn write(&mut self, b: &[u8]) {
        // not used (keys are u64)
        for &x in b {
            self.0 = self.0.rotate_left(8) ^ x as u64;
        }
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.0 = i;
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

/// A string -> [`ObjId`] map whose keys are stored in one string (no allocation per key).
#[derive(Default, Clone, Debug)]
pub(crate) struct StrMap {
    keys: String,
    /// key hash -> first entry with that hash
    heads: HashMap<u64, u32, BuildHasherDefault<IdentityHasher>>,
    /// (key start, key end, value, next entry with the same hash or u32::MAX)
    entries: Vec<(u32, u32, crate::raw::ObjId, u32)>,
}

impl StrMap {
    pub(crate) fn with_capacity(n: usize, key_bytes: usize) -> Self {
        StrMap {
            keys: String::with_capacity(key_bytes),
            heads: HashMap::with_capacity_and_hasher(n, Default::default()),
            entries: Vec::with_capacity(n),
        }
    }

    fn find(&self, h: u64, k: &str) -> Option<usize> {
        let mut i = *self.heads.get(&h)?;
        while i != u32::MAX {
            let (s, e, _, next) = self.entries[i as usize];
            if &self.keys[s as usize..e as usize] == k {
                return Some(i as usize);
            }
            i = next;
        }
        None
    }

    pub(crate) fn get(&self, k: &str) -> Option<crate::raw::ObjId> {
        self.find(hash_str(k), k).map(|i| self.entries[i].2)
    }

    /// Inserts or replaces.
    pub(crate) fn insert(&mut self, k: &str, v: crate::raw::ObjId) {
        let h = hash_str(k);
        if let Some(i) = self.find(h, k) {
            self.entries[i].2 = v;
            return;
        }
        let s = self.keys.len() as u32;
        self.keys.push_str(k);
        let e = self.keys.len() as u32;
        let i = self.entries.len() as u32;
        let next = self.heads.insert(h, i).unwrap_or(u32::MAX);
        self.entries.push((s, e, v, next));
    }

    /// A copy with every value `v` replaced by `v - off`.
    pub(crate) fn rebased(&self, off: u32) -> StrMap {
        let mut m = self.clone();
        for e in &mut m.entries {
            e.2 = crate::raw::ObjId(e.2.0 - off);
        }
        m
    }

    /// Inserts unless present.
    pub(crate) fn insert_first(&mut self, k: &str, v: crate::raw::ObjId) {
        if self.find(hash_str(k), k).is_none() {
            self.insert(k, v);
        }
    }
}

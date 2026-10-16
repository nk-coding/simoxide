//! Identity hash codes and the hashing environment.
//!
//! Java hashes EObjects, enum literals, tuple types, `invalid`, model and module instances and
//! a few internal objects by identity (`System.identityHashCode`). On HotSpot these values come
//! from a thread-local xor-shift generator that every identity hash request on the thread
//! advances, so no transformation can predict them. They decide the iteration order of every
//! `HashSet`, `BagImpl` and `HashMap` holding such values (`DESIGN.md` §6.1, compat rows 2 and 27).
//!
//! The engine asks a [`HashEnv`] for them. Compat mode memoises one value per [`IdentityKey`],
//! drawn on first use from an [`IdentityHashSource`]:
//!
//! - [`HotSpotXorShift`]: HotSpot's generator (`-XX:hashCode=5`), the default surrogate.
//!   Java-like, deterministic, but not equal to any particular JVM run.
//! - [`Tape`]: values recorded from a Java run (keyed by the engine's identity keys, which the
//!   harness maps from stable object keys), with a fallback source for keys not on the tape.
//! - [`Sequential`]: 1, 2, 3, ...; small hash sets then iterate in creation order.
//!
//! Fast mode does not depend on these values for order; [`FastEnv`] derives them from the key.

use crate::value::{EnumLit, ObjRef, TypeRef};
use std::cell::RefCell;
use std::collections::HashMap;

/// Compat or fast mode (`DESIGN.md` §6).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Java's observable behaviour: hash-based collections iterate in `HashMap` bucket order.
    Compat,
    /// Same values, insertion-ordered hash-based collections.
    Fast,
}

/// Something Java hashes by identity.
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub enum IdentityKey {
    /// A model object (host store or engine-local).
    Obj(ObjRef),
    /// An enumeration literal (`EEnumLiteral` or generated `Enumerator`).
    Enum(EnumLit),
    /// A type used as a value (`EClassifier`).
    Type(TypeRef),
    /// A tuple type (tuples hash as `37 * identityHash(type) + 17 * parts.hashCode()`).
    TupleType(u32),
    /// The `invalid` singleton.
    Invalid,
    /// A model extent (`ModelInstance`).
    Model(u32),
    /// A transformation or library instance (`ModuleInstance`).
    Instance(u32),
    /// Engine-internal Java objects without a value of their own, such as the
    /// `MutableInteger` count cells of a `BagImpl` (whose hash code includes them).
    Cell(u64),
}

/// Where identity hash codes come from. Called once per key, when the key's identity hash is
/// first requested; the result is memoised by [`IdentityHashes`].
pub trait IdentityHashSource {
    fn draw(&mut self, key: IdentityKey) -> i32;
}

/// HotSpot's identity hash generator (`-XX:hashCode=5`, the default since JDK 8): Marsaglia's
/// xor-shift with thread-local state `(X, Y, Z, W)`, initialised to `(os::random(), 842502087,
/// 0x8767, 273326509)`; each draw is masked to 31 bits, and 0 is replaced by `0xBAD`.
#[derive(Clone, Debug)]
pub struct HotSpotXorShift {
    x: u32,
    y: u32,
    z: u32,
    w: u32,
}

impl HotSpotXorShift {
    /// Thread state with the given `X` seed.
    pub fn new(seed: u32) -> Self {
        HotSpotXorShift {
            x: seed,
            y: 842_502_087,
            z: 0x8767,
            w: 273_326_509,
        }
    }

    /// Seed of the thread that received the `n`-th (1-based) value of HotSpot's `os::random()`
    /// (Park–Miller, `seed' = 16807 * seed mod (2^31 - 1)`, initial seed 1234567).
    pub fn nth_os_random_thread(n: u32) -> Self {
        let mut s: u64 = 1_234_567;
        for _ in 0..n.max(1) {
            s = (16807 * s) % 2_147_483_647;
        }
        Self::new(s as u32)
    }

    /// The next identity hash.
    pub fn next_hash(&mut self) -> i32 {
        let mut t = self.x;
        t ^= t << 11;
        self.x = self.y;
        self.y = self.z;
        self.z = self.w;
        let mut v = self.w;
        v = (v ^ (v >> 19)) ^ (t ^ (t >> 8));
        self.w = v;
        let v = v & 0x7fff_ffff;
        if v == 0 { 0xBAD } else { v as i32 }
    }
}

impl Default for HotSpotXorShift {
    fn default() -> Self {
        Self::nth_os_random_thread(1)
    }
}

impl IdentityHashSource for HotSpotXorShift {
    fn draw(&mut self, _key: IdentityKey) -> i32 {
        self.next_hash()
    }
}

/// Identity hashes recorded from a Java run.
pub struct Tape {
    values: HashMap<IdentityKey, i32>,
    fallback: Box<dyn IdentityHashSource>,
    misses: usize,
}

impl Tape {
    pub fn new(fallback: Box<dyn IdentityHashSource>) -> Self {
        Tape {
            values: HashMap::new(),
            fallback,
            misses: 0,
        }
    }
    pub fn insert(&mut self, key: IdentityKey, hash: i32) {
        self.values.insert(key, hash);
    }
    /// Keys that were not on the tape and got a value from the fallback.
    pub fn misses(&self) -> usize {
        self.misses
    }
}

impl IdentityHashSource for Tape {
    fn draw(&mut self, key: IdentityKey) -> i32 {
        match self.values.get(&key) {
            Some(&h) => h,
            None => {
                self.misses += 1;
                self.fallback.draw(key)
            }
        }
    }
}

/// 1, 2, 3, ... in order of first use.
#[derive(Default, Clone, Debug)]
pub struct Sequential {
    last: i32,
}

impl IdentityHashSource for Sequential {
    fn draw(&mut self, _key: IdentityKey) -> i32 {
        self.last = self.last.wrapping_add(1);
        self.last
    }
}

/// Memoised identity hashes: one value per key for the whole run, as in Java.
pub struct IdentityHashes {
    memo: HashMap<IdentityKey, i32>,
    source: Box<dyn IdentityHashSource>,
}

impl IdentityHashes {
    pub fn new(source: Box<dyn IdentityHashSource>) -> Self {
        IdentityHashes {
            memo: HashMap::new(),
            source,
        }
    }
    pub fn get(&mut self, key: IdentityKey) -> i32 {
        if let Some(&h) = self.memo.get(&key) {
            return h;
        }
        let h = self.source.draw(key);
        self.memo.insert(key, h);
        h
    }
    /// The value if it was already drawn.
    pub fn peek(&self, key: IdentityKey) -> Option<i32> {
        self.memo.get(&key).copied()
    }
}

impl Default for IdentityHashes {
    fn default() -> Self {
        Self::new(Box::new(HotSpotXorShift::default()))
    }
}

/// What hashing and equality of runtime values need from the run: the mode, identity hashes
/// and Java class names of identity-hashed values (the class name breaks ties in treeified
/// `HashMap` bins).
pub trait HashEnv {
    fn mode(&self) -> Mode;
    fn identity_hash(&self, key: IdentityKey) -> i32;
    /// `getClass().getName()` of an identity-hashed value.
    fn class_name(&self, key: IdentityKey) -> &str {
        default_class_name(key)
    }
    #[inline]
    fn compat(&self) -> bool {
        self.mode() == Mode::Compat
    }
}

/// Java class names Eclipse uses for identity-hashed values of a dynamic-EMF run.
pub fn default_class_name(key: IdentityKey) -> &'static str {
    match key {
        IdentityKey::Obj(_) => "org.eclipse.emf.ecore.impl.DynamicEObjectImpl",
        IdentityKey::Enum(_) => "org.eclipse.emf.ecore.impl.EEnumLiteralImpl",
        IdentityKey::Type(_) => "org.eclipse.emf.ecore.impl.EClassImpl",
        IdentityKey::TupleType(_) => "org.eclipse.ocl.ecore.impl.TupleTypeImpl",
        IdentityKey::Invalid => "org.eclipse.ocl.ecore.internal.OCLStandardLibraryImpl$1",
        IdentityKey::Model(_) => "org.eclipse.m2m.internal.qvt.oml.evaluator.ModelInstanceImpl",
        IdentityKey::Instance(_) => {
            "org.eclipse.m2m.internal.qvt.oml.evaluator.TransformationInstanceImpl"
        }
        IdentityKey::Cell(_) => "org.eclipse.ocl.util.BagImpl$MutableInteger",
    }
}

/// Compat-mode environment with memoised identity hashes.
pub struct CompatEnv {
    pub ids: RefCell<IdentityHashes>,
}

impl CompatEnv {
    pub fn new(source: Box<dyn IdentityHashSource>) -> Self {
        CompatEnv {
            ids: RefCell::new(IdentityHashes::new(source)),
        }
    }
}

impl Default for CompatEnv {
    fn default() -> Self {
        CompatEnv {
            ids: RefCell::new(IdentityHashes::default()),
        }
    }
}

impl HashEnv for CompatEnv {
    fn mode(&self) -> Mode {
        Mode::Compat
    }
    fn identity_hash(&self, key: IdentityKey) -> i32 {
        self.ids.borrow_mut().get(key)
    }
}

/// Fast-mode environment: identity hashes are a fixed mix of the key (no state, no memo).
#[derive(Default, Clone, Copy, Debug)]
pub struct FastEnv;

impl HashEnv for FastEnv {
    fn mode(&self) -> Mode {
        Mode::Fast
    }
    fn identity_hash(&self, key: IdentityKey) -> i32 {
        let (tag, v): (u64, u64) = match key {
            IdentityKey::Obj(o) => (1, o.0 as u64),
            IdentityKey::Enum(e) => (2, ((e.enum_id as u64) << 32) | e.literal as u64),
            IdentityKey::Type(t) => (3, t.0 as u64),
            IdentityKey::TupleType(t) => (4, t as u64),
            IdentityKey::Invalid => (5, 0),
            IdentityKey::Model(m) => (6, m as u64),
            IdentityKey::Instance(m) => (7, m as u64),
            IdentityKey::Cell(c) => (8, c),
        };
        // splitmix64 finaliser
        let mut z = v.wrapping_add(tag.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z as u32 & 0x7fff_ffff) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xorshift_is_31_bit_and_deterministic() {
        let mut a = HotSpotXorShift::default();
        let mut b = HotSpotXorShift::default();
        for _ in 0..1000 {
            let h = a.next_hash();
            assert!(h > 0);
            assert_eq!(h, b.next_hash());
        }
        assert_eq!(HotSpotXorShift::nth_os_random_thread(1).x, 1_422_014_746);
    }

    #[test]
    fn memo_draws_once() {
        let mut ids = IdentityHashes::new(Box::new(Sequential::default()));
        let a = IdentityKey::Obj(ObjRef(7));
        let b = IdentityKey::Obj(ObjRef(3));
        assert_eq!(ids.get(a), 1);
        assert_eq!(ids.get(b), 2);
        assert_eq!(ids.get(a), 1);
    }

    #[test]
    fn tape_with_fallback() {
        let mut t = Tape::new(Box::new(Sequential::default()));
        t.insert(IdentityKey::Obj(ObjRef(1)), 42);
        let mut ids = IdentityHashes::new(Box::new(t));
        assert_eq!(ids.get(IdentityKey::Obj(ObjRef(1))), 42);
        assert_eq!(ids.get(IdentityKey::Obj(ObjRef(2))), 1);
    }
}

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
#[cfg(feature = "compat")]
use std::cell::RefCell;
use std::collections::HashMap;

/// Compat or fast mode (`DESIGN.md` §6).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Java's observable behaviour: hash-based collections iterate in `HashMap` bucket order
    /// (needs the cargo feature `compat`; the default with it).
    Compat,
    /// Same values, insertion-ordered hash-based collections (always available; the default
    /// without `compat`).
    Fast,
}

impl Default for Mode {
    /// [`Mode::Compat`] in builds with the feature `compat`, else [`Mode::Fast`].
    fn default() -> Self {
        if cfg!(feature = "compat") {
            Mode::Compat
        } else {
            Mode::Fast
        }
    }
}

impl Mode {
    /// Whether this build can run the mode (compat mode needs the cargo feature `compat`).
    pub fn available(self) -> bool {
        self == Mode::Fast || cfg!(feature = "compat")
    }

    /// Why this build cannot run the mode (`None` if it can).
    pub fn unavailable_reason(self) -> Option<String> {
        (!self.available()).then(|| {
            "compat mode is not compiled in: build with the cargo feature `compat` \
             (qvto-cli: `--features compat`; see LICENSES)"
                .to_string()
        })
    }
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

/// HotSpot's identity hash generator (`-XX:hashCode=5`), from `simoxide-jcompat-exact`.
#[cfg(feature = "compat")]
pub use simoxide_jcompat_exact::hotspot::HotSpotXorShift;

#[cfg(feature = "compat")]
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

/// The same value for every key: HotSpot's `-XX:hashCode=2` (every identity hash is 1), which
/// makes identity-hash orders reproducible across runs (differential fuzzing).
#[derive(Clone, Debug)]
pub struct Constant(pub i32);

impl IdentityHashSource for Constant {
    fn draw(&mut self, _key: IdentityKey) -> i32 {
        self.0
    }
}

/// Memoised identity hashes: one value per key for the whole run, as in Java.
pub struct IdentityHashes {
    /// Host objects by index, in pages of 512 allocated on first use (handles are dense but
    /// can be large: a run touches few of a big store's objects).
    pages: Vec<Option<Box<[Option<i32>; PAGE]>>>,
    memo: crate::fxhash::FxHashMap<IdentityKey, i32>,
    source: Box<dyn IdentityHashSource>,
}

const PAGE: usize = 512;

impl IdentityHashes {
    pub fn new(source: Box<dyn IdentityHashSource>) -> Self {
        IdentityHashes {
            pages: Vec::new(),
            memo: Default::default(),
            source,
        }
    }
    pub fn get(&mut self, key: IdentityKey) -> i32 {
        if let IdentityKey::Obj(o) = key
            && !o.is_local()
        {
            let i = o.index() as usize;
            let (p, j) = (i / PAGE, i % PAGE);
            if let Some(Some(page)) = self.pages.get(p)
                && let Some(h) = page[j]
            {
                return h;
            }
            let h = self.source.draw(key);
            if self.pages.len() <= p {
                self.pages.resize_with(p + 1, || None);
            }
            self.pages[p].get_or_insert_with(|| Box::new([None; PAGE]))[j] = Some(h);
            return h;
        }
        if let Some(&h) = self.memo.get(&key) {
            return h;
        }
        let h = self.source.draw(key);
        self.memo.insert(key, h);
        h
    }
    /// The value if it was already drawn.
    pub fn peek(&self, key: IdentityKey) -> Option<i32> {
        if let IdentityKey::Obj(o) = key
            && !o.is_local()
        {
            let i = o.index() as usize;
            return self.pages.get(i / PAGE)?.as_ref()?[i % PAGE];
        }
        self.memo.get(&key).copied()
    }
}

#[cfg(feature = "compat")]
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
    /// Java-ordered storage: compat mode, in builds with the feature `compat` (never otherwise).
    #[inline]
    fn compat(&self) -> bool {
        cfg!(feature = "compat") && self.mode() == Mode::Compat
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

/// Compat-mode environment with memoised identity hashes (feature `compat`).
#[cfg(feature = "compat")]
pub struct CompatEnv {
    pub ids: RefCell<IdentityHashes>,
}

#[cfg(feature = "compat")]
impl CompatEnv {
    pub fn new(source: Box<dyn IdentityHashSource>) -> Self {
        CompatEnv {
            ids: RefCell::new(IdentityHashes::new(source)),
        }
    }
}

#[cfg(feature = "compat")]
impl Default for CompatEnv {
    fn default() -> Self {
        CompatEnv {
            ids: RefCell::new(IdentityHashes::default()),
        }
    }
}

#[cfg(feature = "compat")]
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

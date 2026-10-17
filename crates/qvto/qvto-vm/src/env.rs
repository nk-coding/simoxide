//! The run's hashing environment (`qvto_runtime::HashEnv`): the mode and the identity hashes of
//! objects, enum literals, types, tuple types, models and module instances (DESIGN §6.1,
//! Q1c-7).
//!
//! Compat mode memoises one identity hash per key, drawn on first use from an
//! [`IdentityHashSource`]: HotSpot's xor-shift surrogate by default, or a tape of a Java run
//! ([`TapeHook`]) that maps keys of the run to the hashes the Java run recorded. Fast mode
//! derives the hash from the key.

use qvto_runtime::identity::{HotSpotXorShift, IdentityHashSource, IdentityHashes};
use qvto_runtime::{FastEnv, HashEnv, IdentityKey, Mode};
use std::cell::RefCell;

/// The `HashEnv` of a run.
pub struct RunEnv {
    mode: Mode,
    ids: RefCell<IdentityHashes>,
    /// Keys that are the same Java object as another key: type values and the `eClass` of
    /// printed objects are the metamodel's `EClassifier` objects of the store.
    aliases: std::collections::HashMap<IdentityKey, IdentityKey>,
}

impl RunEnv {
    pub fn new(mode: Mode, source: Option<Box<dyn IdentityHashSource>>) -> RunEnv {
        RunEnv {
            mode,
            ids: RefCell::new(IdentityHashes::new(
                source.unwrap_or_else(|| Box::new(HotSpotXorShift::default())),
            )),
            aliases: std::collections::HashMap::new(),
        }
    }

    /// Makes `key` the same object as `target` for identity hashes.
    pub fn alias(&mut self, key: IdentityKey, target: IdentityKey) {
        self.aliases.insert(key, target);
    }

    /// The identity hash of a key if it has been drawn.
    pub fn peek(&self, key: IdentityKey) -> Option<i32> {
        self.ids.borrow().peek(key)
    }
}

impl HashEnv for RunEnv {
    fn mode(&self) -> Mode {
        self.mode
    }
    fn identity_hash(&self, key: IdentityKey) -> i32 {
        let key = self.aliases.get(&key).copied().unwrap_or(key);
        match self.mode {
            Mode::Compat => self.ids.borrow_mut().get(key),
            Mode::Fast => FastEnv.identity_hash(key),
        }
    }
}

/// An identity-hash source that answers keys a hook knows (a Java run's tape, mapped to this
/// run's keys) and draws the others from a fallback.
pub struct TapeHook {
    lookup: Box<dyn FnMut(IdentityKey) -> Option<i32>>,
    fallback: Box<dyn IdentityHashSource>,
    pub hits: usize,
    pub misses: usize,
}

impl TapeHook {
    pub fn new(
        lookup: Box<dyn FnMut(IdentityKey) -> Option<i32>>,
        fallback: Box<dyn IdentityHashSource>,
    ) -> TapeHook {
        TapeHook {
            lookup,
            fallback,
            hits: 0,
            misses: 0,
        }
    }
}

impl IdentityHashSource for TapeHook {
    fn draw(&mut self, key: IdentityKey) -> i32 {
        match (self.lookup)(key) {
            Some(h) => {
                self.hits += 1;
                h
            }
            None => {
                self.misses += 1;
                self.fallback.draw(key)
            }
        }
    }
}

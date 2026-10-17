//! The run's hashing environment (`qvto_runtime::HashEnv`): the mode and the identity hashes of
//! objects, enum literals, types, tuple types, models and module instances (DESIGN §6.1,
//! Q1c-7).
//!
//! Compat mode memoises one identity hash per key, drawn on first use from an
//! [`IdentityHashSource`]: HotSpot's xor-shift surrogate by default, or a tape of a Java run
//! ([`TapeHook`]) that maps keys of the run to the hashes the Java run recorded. Fast mode
//! derives the hash from the key. Ordered mode does too, unless the run is given a source:
//! its collections do not depend on the values, which only appear in `Class@hash` texts.

use qvto_runtime::identity::{IdentityHashSource, IdentityHashes};
use qvto_runtime::{FastEnv, HashEnv, IdentityKey, Mode};
use std::cell::RefCell;

/// The `HashEnv` of a run.
pub struct RunEnv {
    mode: Mode,
    /// Identity hashes come from the memo (compat mode, or ordered mode with a given source).
    memo: bool,
    ids: RefCell<IdentityHashes>,
    /// Keys that are the same Java object as another key: type values and the `eClass` of
    /// printed objects are the metamodel's `EClassifier` objects of the store.
    aliases: std::collections::HashMap<IdentityKey, IdentityKey>,
}

impl RunEnv {
    pub fn new(mode: Mode, source: Option<Box<dyn IdentityHashSource>>) -> RunEnv {
        RunEnv {
            mode,
            memo: mode == Mode::Compat || (mode == Mode::Ordered && source.is_some()),
            ids: RefCell::new(IdentityHashes::new(source.unwrap_or_else(default_source))),
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
        if self.memo {
            self.ids.borrow_mut().get(key)
        } else {
            FastEnv.identity_hash(key)
        }
    }
}

/// Compat mode's default identity-hash source: HotSpot's xor-shift surrogate.
#[cfg(feature = "compat")]
fn default_source() -> Box<dyn IdentityHashSource> {
    Box::new(qvto_runtime::identity::HotSpotXorShift::default())
}

/// Without compat mode the memo is asked only with a source (fast and ordered mode derive
/// hashes from the keys).
#[cfg(not(feature = "compat"))]
fn default_source() -> Box<dyn IdentityHashSource> {
    Box::new(qvto_runtime::identity::Sequential::default())
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

//! The Fx hasher (rustc's, `simoxide-jcompat`) for the interpreter's lookup tables; keys are
//! small integers and id paths.

pub use simoxide_jcompat::fx::FxHasher;

/// `HashMap` with [`FxHasher`].
pub type FxMap<K, V> = simoxide_jcompat::fx::FxHashMap<K, V>;

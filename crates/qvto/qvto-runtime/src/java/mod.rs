//! Java-compatible building blocks: hash codes, `HashMap`, number formatting and parsing,
//! character data and `String` operations with Java's semantics. The first ones are the
//! foundation crates `simoxide-jcompat` (what the Java documentation specifies) and
//! `simoxide-jcompat-exact` (`HashMap` order; with the feature `compat`, else an uninhabited
//! stand-in); `string` works on [`crate::str::Str`].

pub use simoxide_jcompat::{chars, chars_data, fmt, hash, parse};
#[cfg(feature = "compat")]
pub use simoxide_jcompat_exact::hashmap;
#[cfg(not(feature = "compat"))]
#[path = "nohashmap.rs"]
pub mod hashmap;
pub mod string;

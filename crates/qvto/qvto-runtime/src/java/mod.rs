//! Java-compatible building blocks: hash codes, `HashMap`, number formatting and parsing,
//! character data and `String` operations with Java's semantics. The first ones are the
//! foundation crates `simoxide-jcompat` (what the Java documentation specifies) and
//! `simoxide-jcompat-exact` (`HashMap` order, for compat mode); `string` works on [`crate::str::Str`].

pub use simoxide_jcompat::{chars, chars_data, fmt, hash, parse};
pub use simoxide_jcompat_exact::hashmap;
pub mod string;

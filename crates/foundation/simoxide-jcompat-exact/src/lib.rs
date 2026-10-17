//! Java behaviour that only OpenJDK's and HotSpot's sources define, for SimOxide's exact modes
//! and the QVTo engine's compat mode:
//!
//! - [`hashmap`]: `java.util.HashMap` of JDK 21 with its observable iteration order (power-of-two
//!   table, `spread`, resize split, constructor sizing, tree bins, removal);
//! - [`order`]: the closed-form `HashMap` order of insert-only maps ([`order::JdkHashOrder`]);
//! - [`hotspot`]: HotSpot's identity-hash generator and the recovery of a thread's sequence.
//!
//! **No licence is granted for this crate** (see `LICENSE-NOTICE` and the repository's
//! `LICENSES`): it is an independent reimplementation, but its details come only from
//! GPL-licensed OpenJDK and HotSpot sources, while the code that uses it is EPL-2.0, and no
//! licence covers the combination. Build it for local use only; builds that include it must not
//! be distributed. SimOxide's and the QVTo engine's default builds do not include it: their users
//! opt in with the cargo features `jdk-order` (through `exact` or `compat`).

pub mod hashmap;
pub mod hotspot;
pub mod order;

//! HotSpot's identity-hash sequence, for replaying identity-hash tapes (`DESIGN.md` 6.1): the
//! generator and the recovery of a thread's sequence from its first two draws, in
//! `simoxide-jcompat-exact`.

pub use simoxide_jcompat_exact::hotspot::{HotSpotXorShift, XorShift, sequence_index};

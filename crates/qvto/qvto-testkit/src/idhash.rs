//! HotSpot's identity-hash sequence, for replaying identity-hash tapes (DESIGN.md 6.1).
//!
//! With the default `-XX:hashCode=5`, HotSpot gives each thread a Marsaglia xor-shift state
//! (`X = os::random()`, `Y = 842502087`, `Z = 0x8767`, `W = 273326509` at thread creation) and
//! draws the next value whenever an object of that thread gets its identity hash; the hash is
//! the value's low 31 bits (0 becomes `0xBAD`). The Java runner runs each case on a fresh thread
//! and records the first two hashes the thread draws (the tape's `seed` line), which determine
//! `X`: the whole sequence of the case's thread is then known, and each recorded object hash can
//! be placed in it ([`sequence_index`]).

/// The xor-shift state of one HotSpot thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XorShift {
    x: u32,
    y: u32,
    z: u32,
    w: u32,
}

const Y0: u32 = 842_502_087;
const Z0: u32 = 0x8767;
const W0: u32 = 273_326_509;
const HASH_MASK: u32 = 0x7fff_ffff;

impl XorShift {
    /// A thread whose `os::random()` value was `x`.
    pub fn new(x: u32) -> XorShift {
        XorShift {
            x,
            y: Y0,
            z: Z0,
            w: W0,
        }
    }

    /// The next identity hash (`ObjectSynchronizer::get_next_hash`, hashCode mode 5).
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> i32 {
        let mut t = self.x;
        t ^= t << 11;
        self.x = self.y;
        self.y = self.z;
        self.z = self.w;
        let mut v = self.w;
        v = (v ^ (v >> 19)) ^ (t ^ (t >> 8));
        self.w = v;
        let h = v & HASH_MASK;
        if h == 0 { 0xBAD } else { h as i32 }
    }

    /// The thread state whose first two draws are `h1`, `h2` (the tape's seed), if any.
    pub fn recover(h1: i32, h2: i32) -> Option<XorShift> {
        // h1 = low 31 bits of v1 = (W0 ^ W0 >> 19) ^ (t ^ t >> 8), t = x ^ x << 11
        for top in [0u32, 0x8000_0000] {
            let v1 = (h1 as u32) | top;
            let u = v1 ^ (W0 ^ (W0 >> 19));
            let t = u ^ (u >> 8) ^ (u >> 16) ^ (u >> 24);
            let x = t ^ (t << 11) ^ (t << 22);
            let mut s = XorShift::new(x);
            if s.next() == h1 && s.next() == h2 {
                return Some(XorShift::new(x));
            }
        }
        None
    }
}

/// Positions of the given hashes in the sequence of the thread with the given seed, searching
/// the first `limit` draws (`None` for hashes not found: drawn by another thread).
pub fn sequence_index(
    seed: (i32, i32),
    hashes: &[i32],
    limit: usize,
) -> Option<Vec<Option<usize>>> {
    let mut s = XorShift::recover(seed.0, seed.1)?;
    let mut want: std::collections::HashMap<i32, Option<usize>> =
        hashes.iter().map(|h| (*h, None)).collect();
    let mut missing = want.len();
    for i in 0..limit {
        if missing == 0 {
            break;
        }
        if let Some(slot) = want.get_mut(&s.next())
            && slot.is_none()
        {
            *slot = Some(i);
            missing -= 1;
        }
    }
    Some(hashes.iter().map(|h| want[h]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recover_round_trip() {
        for x in [1u32, 12345, 0x7fff_ffff, 987_654_321] {
            let mut s = XorShift::new(x);
            let (a, b) = (s.next(), s.next());
            let mut r = XorShift::recover(a, b).unwrap();
            assert_eq!(r.next(), a);
            assert_eq!(r.next(), b);
        }
    }
}

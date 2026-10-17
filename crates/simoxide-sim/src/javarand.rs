//! `java.util.Random`'s linear congruential generator, written from its API specification (the
//! algorithm and constants are part of the Java SE documentation of `java.util.Random`; no
//! OpenJDK code, decision U12). Slingshot draws probabilistic branches and link failures from
//! `Math.random()`, which is `nextDouble()` of one such generator; the reference runner seeds
//! it with `N + 1000003` for seed `N` (`docs/spec/slingshot/random.md`, SS-RNG-1.2).

/// The `java.util.Random` LCG.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct JavaRandom {
    seed: u64,
    /// Doubles drawn so far.
    pub count: u64,
}

const MULTIPLIER: u64 = 0x5_DEEC_E66D;
const ADDEND: u64 = 0xB;
const MASK: u64 = (1 << 48) - 1;

impl JavaRandom {
    /// `new Random(seed)`: `(seed ^ 0x5DEECE66D) & (2^48 - 1)`.
    pub fn new(seed: i64) -> JavaRandom {
        JavaRandom {
            seed: (seed as u64 ^ MULTIPLIER) & MASK,
            count: 0,
        }
    }

    /// The branch stream of Slingshot for run seed `seed` (the runner's `Math.random()` seed).
    pub fn slingshot_math(seed: i64) -> JavaRandom {
        JavaRandom::new(seed.wrapping_add(1_000_003))
    }

    /// `next(bits)`.
    #[inline]
    fn next(&mut self, bits: u32) -> u64 {
        self.seed = (self.seed.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND)) & MASK;
        self.seed >> (48 - bits)
    }

    /// `nextDouble()`: `((next(26) << 27) + next(27)) * 2^-53`.
    #[inline]
    pub fn next_double(&mut self) -> f64 {
        self.count += 1;
        let hi = self.next(26);
        let lo = self.next(27);
        ((hi << 27) + lo) as f64 * (1.0 / (1u64 << 53) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_java_util_random() {
        // new java.util.Random(42).nextDouble() x 3 (values printed by a JVM)
        let mut r = JavaRandom::new(42);
        assert_eq!(r.next_double(), 0.7275636800328681);
        assert_eq!(r.next_double(), 0.6832234717598454);
        assert_eq!(r.next_double(), 0.30871945533265976);
        assert_eq!(r.count, 3);
        // new java.util.Random(0).nextDouble()
        assert_eq!(JavaRandom::new(0).next_double(), 0.730967787376657);
        // the reference runner's Math.random() for seed 1 (x_ss_minimalexample tape, stream math)
        let mut m = JavaRandom::slingshot_math(1);
        assert_eq!(m.next_double(), 0.3740892462829557);
        assert_eq!(m.next_double(), 0.7456648079325994);
        assert_eq!(m.next_double(), 0.2692023415646321);
    }
}

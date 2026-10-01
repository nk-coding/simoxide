//! Port of Apache Commons Math 3.6.1 `org.apache.commons.math3.random.MersenneTwister`
//! (MT19937) together with `BitsStreamGenerator.nextDouble()`.

const N: usize = 624;
const M: usize = 397;
const MAG01: [u32; 2] = [0x0, 0x9908_b0df];

/// MT19937 with the seeding and double conversion of Commons Math 3.6.1.
#[derive(Clone)]
pub struct MersenneTwister {
    mt: [u32; N],
    mti: usize,
}

impl std::fmt::Debug for MersenneTwister {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MersenneTwister")
            .field("mti", &self.mti)
            .finish_non_exhaustive()
    }
}

impl MersenneTwister {
    /// `new MersenneTwister(int seed)` / `setSeed(int)`.
    pub fn from_int(seed: i32) -> Self {
        let mut mt = MersenneTwister { mt: [0; N], mti: N };
        mt.set_seed_int(seed);
        mt
    }

    /// `new MersenneTwister(int[] seed)` / `setSeed(int[])` (init_by_array).
    pub fn from_int_array(seed: &[i32]) -> Self {
        let mut mt = MersenneTwister { mt: [0; N], mti: N };
        mt.set_seed_int_array(seed);
        mt
    }

    /// `new MersenneTwister(long seed)` / `setSeed(long)`.
    pub fn from_long(seed: i64) -> Self {
        let s = seed as u64;
        Self::from_int_array(&[(s >> 32) as u32 as i32, (s & 0xffff_ffff) as u32 as i32])
    }

    fn set_seed_int(&mut self, seed: i32) {
        // Java: long longMT = seed (sign-extended); subsequent values are masked to 32 bits.
        let mut long_mt = seed as i64;
        self.mt[0] = long_mt as u32;
        for i in 1..N {
            long_mt = (1_812_433_253i64
                .wrapping_mul(long_mt ^ (long_mt >> 30))
                .wrapping_add(i as i64))
                & 0xffff_ffff;
            self.mt[i] = long_mt as u32;
        }
        self.mti = N;
    }

    fn set_seed_int_array(&mut self, seed: &[i32]) {
        assert!(
            !seed.is_empty(),
            "MersenneTwister seed array must not be empty"
        );
        self.set_seed_int(19_650_218);
        let mut i = 1usize;
        let mut j = 0usize;
        let mut k = N.max(seed.len());
        while k != 0 {
            let l0 = self.mt[i] as i64;
            let l1 = self.mt[i - 1] as i64;
            let l = (l0 ^ ((l1 ^ (l1 >> 30)).wrapping_mul(1_664_525)))
                .wrapping_add(seed[j] as i64)
                .wrapping_add(j as i64);
            self.mt[i] = (l & 0xffff_ffff) as u32;
            i += 1;
            j += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
            if j >= seed.len() {
                j = 0;
            }
            k -= 1;
        }
        k = N - 1;
        while k != 0 {
            let l0 = self.mt[i] as i64;
            let l1 = self.mt[i - 1] as i64;
            let l = (l0 ^ ((l1 ^ (l1 >> 30)).wrapping_mul(1_566_083_941))).wrapping_sub(i as i64);
            self.mt[i] = (l & 0xffff_ffff) as u32;
            i += 1;
            if i >= N {
                self.mt[0] = self.mt[N - 1];
                i = 1;
            }
            k -= 1;
        }
        self.mt[0] = 0x8000_0000;
        self.mti = N;
    }

    #[cold]
    #[inline(never)]
    fn twist(&mut self) {
        let mt = &mut self.mt;
        let mut mt_next = mt[0];
        for k in 0..N - M {
            let mt_curr = mt_next;
            mt_next = mt[k + 1];
            let y = (mt_curr & 0x8000_0000) | (mt_next & 0x7fff_ffff);
            mt[k] = mt[k + M] ^ (y >> 1) ^ MAG01[(y & 1) as usize];
        }
        for k in N - M..N - 1 {
            let mt_curr = mt_next;
            mt_next = mt[k + 1];
            let y = (mt_curr & 0x8000_0000) | (mt_next & 0x7fff_ffff);
            mt[k] = mt[k + M - N] ^ (y >> 1) ^ MAG01[(y & 1) as usize];
        }
        let y = (mt_next & 0x8000_0000) | (mt[0] & 0x7fff_ffff);
        mt[N - 1] = mt[M - 1] ^ (y >> 1) ^ MAG01[(y & 1) as usize];
        self.mti = 0;
    }

    /// Next tempered 32-bit output (`next(32)` as unsigned).
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        if self.mti >= N {
            self.twist();
        }
        let mut y = self.mt[self.mti];
        self.mti += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9d2c_5680;
        y ^= (y << 15) & 0xefc6_0000;
        y ^= y >> 18;
        y
    }

    /// `next(bits)`: the top `bits` bits of the next output (1 <= bits <= 32).
    #[inline]
    pub fn next_bits(&mut self, bits: u32) -> u32 {
        self.next_u32() >> (32 - bits)
    }

    /// `nextInt()`.
    #[inline]
    pub fn next_int(&mut self) -> i32 {
        self.next_u32() as i32
    }

    /// `BitsStreamGenerator.nextDouble()`: 52 random bits, uniform in `[0, 1)`
    /// (two 32-bit outputs per double).
    #[inline]
    pub fn next_double(&mut self) -> f64 {
        let high = (self.next_bits(26) as u64) << 26;
        let low = self.next_bits(26) as u64;
        (high | low) as f64 * f64::from_bits(0x3cb0_0000_0000_0000) // 0x1.0p-52
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_mt19937_init_by_array() {
        // Reference output of mt19937ar.c with init_by_array({0x123, 0x234, 0x345, 0x456}):
        // the first genrand_int32() values.
        let mut mt = MersenneTwister::from_int_array(&[0x123, 0x234, 0x345, 0x456]);
        let expected = [
            1_067_595_299u32,
            955_945_823,
            477_289_528,
            4_107_218_783,
            4_228_976_476,
        ];
        for e in expected {
            assert_eq!(mt.next_u32(), e);
        }
    }

    #[test]
    fn scale_constant() {
        assert_eq!(f64::from_bits(0x3cb0_0000_0000_0000), 2f64.powi(-52));
    }
}

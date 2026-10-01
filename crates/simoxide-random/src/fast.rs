//! Fast-mode random numbers (feature `fast`): a different generator and different sampling
//! algorithms for the same distributions, **not** the reference's bits.
//!
//! SimOxide's exact mode reproduces SimuLizar's random numbers bit for bit: MT19937 with two
//! 32-bit outputs per double, and every distribution inverted numerically (Commons Math 2.1
//! bracketing plus Brent, bisection over the regularized gamma function for `Pois`). That costs
//! 0.5 to 20 µs per sample. The fast mode (`simoxide_sim::compat::Fast`) samples the same
//! distributions with standard algorithms:
//!
//! | StoEx | Fast algorithm |
//! |---|---|
//! | uniform stream | xoshiro256++ (seeded by SplitMix64), 53-bit doubles in `[0, 1)` |
//! | `Exp(rate)` | ziggurat (256 layers, Marsaglia–Tsang), times the mean |
//! | `Norm(m, s)` | ziggurat standard normal, `m + s·z` |
//! | `Lognorm(mu, s)`, `LognormMoments` | `exp(mu + s·z)` (parameters as the reference computes them) |
//! | `Gamma(a, t)`, `GammaMoments` | Marsaglia–Tsang (2000); `a < 1` via `G(a+1)·U^(1/a)` |
//! | `Pois(m)` | inversion by sequential search for `m < 10`, PTRS (Hörmann 1993) otherwise; **minus 1**, as the reference (REF-4) |
//! | `UniDouble(a, b)` | `a + u·(b − a)` (the reference's Brent result is only 1e-6 accurate) |
//! | `UniInt(a, b)` | Lemire's unbiased multiply-shift |
//!
//! **Parameters and errors are the reference's.** Every hook runs the reference's parameter
//! check and distribution constructor first ([`crate::dist`]), so invalid parameters give the same
//! error without a draw. The fast algorithm is used only where the parameters are finite (and in
//! range for `Pois`/`UniInt`); anything else (NaN or infinite parameters produced by
//! `GammaMoments(0, cv)`, `UniDouble(a, a)` (REF-3), an `int` overflow of `UniInt`'s count, ...)
//! falls back to the reference algorithm with one uniform, so it fails or succeeds exactly like
//! the reference. What differs: the values (another stream and other algorithms), the number of
//! uniforms per sample, and parameter sets where the reference's numerical inversion itself fails
//! or runs for hours (e.g. scales beyond 2^53, where its bracketing in steps of 1.0 stalls): the
//! fast algorithms sample those normally.

// Negated comparisons (`!(x > 0.0)`) are deliberate: NaN parameters take the reference path.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::dist::{self, DistError, Exponential, Gamma, LogNormal, Normal, Poisson, UniformDouble};
use crate::source::UniformSource;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------------------------
// generator

/// SplitMix64 (Steele, Lea, Flood 2014): seeds [`Xoshiro256pp`].
#[derive(Clone, Debug)]
pub struct SplitMix64(pub u64);

impl SplitMix64 {
    /// Next output.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
}

/// xoshiro256++ 1.0 (Blackman, Vigna 2019): 256-bit state, period 2^256 − 1, passes BigCrush
/// and PractRand.
#[derive(Clone, Debug)]
pub struct Xoshiro256pp {
    s: [u64; 4],
}

impl Xoshiro256pp {
    /// Seeds the state with four SplitMix64 outputs of `seed` (the authors' recommendation);
    /// the state is never all zero.
    pub fn seed_from_u64(seed: u64) -> Self {
        let mut sm = SplitMix64(seed);
        let s = [sm.next_u64(), sm.next_u64(), sm.next_u64(), sm.next_u64()];
        Xoshiro256pp { s }
    }

    /// Next 64-bit output.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.s;
        let result = s[0].wrapping_add(s[3]).rotate_left(23).wrapping_add(s[0]);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        result
    }
}

// ---------------------------------------------------------------------------------------------
// tables

const ZIG_LAYERS: usize = 256;
/// Start of the tail of the 256-layer normal ziggurat (Marsaglia, Tsang 2000).
const ZIG_NORM_R: f64 = 3.654_152_885_361_009;
const ZIG_NORM_V: f64 = 0.004_928_673_233_99;
/// Start of the tail of the 256-layer exponential ziggurat.
const ZIG_EXP_R: f64 = 7.697_117_470_131_05;
const ZIG_EXP_V: f64 = 0.003_949_659_822_581_557;
/// `ln k!` from the table below this, from Stirling's series above.
const LN_FACT_TABLE: usize = 256;

/// Ziggurat layer tables and `ln k!` for small `k`, computed once per process.
pub struct Tables {
    norm_x: [f64; ZIG_LAYERS + 1],
    norm_f: [f64; ZIG_LAYERS + 1],
    exp_x: [f64; ZIG_LAYERS + 1],
    exp_f: [f64; ZIG_LAYERS + 1],
    ln_fact: [f64; LN_FACT_TABLE],
}

impl std::fmt::Debug for Tables {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Tables")
    }
}

/// Layer boundaries `x[0] = v/f(r) > x[1] = r > ... > x[256] = 0` and `f(x[i])`.
fn zig_tables(
    r: f64,
    v: f64,
    f: impl Fn(f64) -> f64,
    f_inv: impl Fn(f64) -> f64,
) -> ([f64; ZIG_LAYERS + 1], [f64; ZIG_LAYERS + 1]) {
    let mut x = [0.0; ZIG_LAYERS + 1];
    x[0] = v / f(r);
    x[1] = r;
    for i in 2..ZIG_LAYERS {
        x[i] = f_inv(v / x[i - 1] + f(x[i - 1]));
    }
    x[ZIG_LAYERS] = 0.0;
    let mut fx = [0.0; ZIG_LAYERS + 1];
    for i in 0..=ZIG_LAYERS {
        fx[i] = f(x[i]);
    }
    (x, fx)
}

impl Tables {
    fn build() -> Tables {
        let (norm_x, norm_f) = zig_tables(
            ZIG_NORM_R,
            ZIG_NORM_V,
            |x| (-0.5 * x * x).exp(),
            |y| (-2.0 * y.ln()).sqrt(),
        );
        let (exp_x, exp_f) = zig_tables(ZIG_EXP_R, ZIG_EXP_V, |x| (-x).exp(), |y| -y.ln());
        let mut ln_fact = [0.0; LN_FACT_TABLE];
        for k in 1..LN_FACT_TABLE {
            ln_fact[k] = ln_fact[k - 1] + (k as f64).ln();
        }
        Tables {
            norm_x,
            norm_f,
            exp_x,
            exp_f,
            ln_fact,
        }
    }

    /// The process-wide tables.
    pub fn get() -> &'static Tables {
        static T: OnceLock<Tables> = OnceLock::new();
        T.get_or_init(Tables::build)
    }

    /// `ln k!` for `k >= 0` (table up to 255, Stirling's series with three correction terms
    /// above: relative error < 1e-16).
    #[inline]
    pub fn ln_factorial(&self, k: f64) -> f64 {
        if k < LN_FACT_TABLE as f64 {
            return self.ln_fact[k as usize];
        }
        let x = k + 1.0;
        let x2 = x * x;
        (x - 0.5) * x.ln() - x
            + 0.918_938_533_204_672_8 // 0.5 * ln(2π)
            + (1.0 / 12.0 - (1.0 / 360.0 - 1.0 / (1260.0 * x2)) / x2) / x
    }
}

// ---------------------------------------------------------------------------------------------
// the source

/// The fast mode's uniform source and samplers (see the module docs).
#[derive(Clone)]
pub struct FastSource {
    rng: Xoshiro256pp,
    t: &'static Tables,
}

impl std::fmt::Debug for FastSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FastSource").finish_non_exhaustive()
    }
}

const TWO_POW_M53: f64 = 1.0 / (1u64 << 53) as f64;

impl FastSource {
    /// A stream for `seed` (different seeds give independent-looking streams).
    pub fn new(seed: u64) -> Self {
        FastSource {
            rng: Xoshiro256pp::seed_from_u64(seed),
            t: Tables::get(),
        }
    }

    /// Next raw 64-bit output.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        self.rng.next_u64()
    }

    /// Uniform in `[0, 1)` with 53 random bits.
    #[inline]
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * TWO_POW_M53
    }

    /// Uniform in the open interval `(0, 1)`.
    #[inline]
    pub fn open01(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 0.5) * TWO_POW_M53
    }

    /// Unbiased uniform integer in `[0, n)` (Lemire 2019), `n > 0`.
    #[inline]
    pub fn below(&mut self, n: u64) -> u64 {
        debug_assert!(n > 0);
        let mut m = u128::from(self.next_u64()) * u128::from(n);
        if (m as u64) < n {
            let t = n.wrapping_neg() % n;
            while (m as u64) < t {
                m = u128::from(self.next_u64()) * u128::from(n);
            }
        }
        (m >> 64) as u64
    }

    /// Standard normal (ziggurat).
    #[inline]
    pub fn std_normal(&mut self) -> f64 {
        let t = self.t;
        loop {
            let bits = self.next_u64();
            let i = (bits & 0xff) as usize;
            // [1, 2) from the top 52 bits, then [-1, 1)
            let u = 2.0 * f64::from_bits((bits >> 12) | 0x3ff0_0000_0000_0000) - 3.0;
            let x = u * t.norm_x[i];
            if x.abs() < t.norm_x[i + 1] {
                return x;
            }
            if i == 0 {
                return self.normal_tail(u < 0.0);
            }
            let y = t.norm_f[i + 1] + (t.norm_f[i] - t.norm_f[i + 1]) * self.uniform();
            if y < (-0.5 * x * x).exp() {
                return x;
            }
        }
    }

    /// Tail beyond `ZIG_NORM_R` (Marsaglia 1964).
    #[cold]
    fn normal_tail(&mut self, negative: bool) -> f64 {
        loop {
            let x = self.open01().ln() / ZIG_NORM_R;
            let y = self.open01().ln();
            if -2.0 * y >= x * x {
                return if negative {
                    x - ZIG_NORM_R
                } else {
                    ZIG_NORM_R - x
                };
            }
        }
    }

    /// Standard exponential (ziggurat).
    #[inline]
    pub fn std_exp(&mut self) -> f64 {
        let t = self.t;
        loop {
            let bits = self.next_u64();
            let i = (bits & 0xff) as usize;
            let u = f64::from_bits((bits >> 12) | 0x3ff0_0000_0000_0000) - 1.0;
            let x = u * t.exp_x[i];
            if x < t.exp_x[i + 1] {
                return x;
            }
            if i == 0 {
                return ZIG_EXP_R - self.open01().ln();
            }
            let y = t.exp_f[i + 1] + (t.exp_f[i] - t.exp_f[i + 1]) * self.uniform();
            if y < (-x).exp() {
                return x;
            }
        }
    }

    /// Gamma with shape `alpha > 0` and scale 1 (Marsaglia, Tsang 2000).
    #[inline]
    pub fn std_gamma(&mut self, alpha: f64) -> f64 {
        if alpha < 1.0 {
            // G(a) = G(a + 1) * U^(1/a)
            let g = self.gamma_ge1(alpha + 1.0);
            return g * self.open01().powf(1.0 / alpha);
        }
        self.gamma_ge1(alpha)
    }

    fn gamma_ge1(&mut self, alpha: f64) -> f64 {
        let d = alpha - 1.0 / 3.0;
        let c = 1.0 / (9.0 * d).sqrt();
        loop {
            let (x, v) = loop {
                let x = self.std_normal();
                let v = 1.0 + c * x;
                if v > 0.0 {
                    break (x, v);
                }
            };
            let v = v * v * v;
            let u = self.open01();
            let x2 = x * x;
            if u < 1.0 - 0.0331 * x2 * x2 || u.ln() < 0.5 * x2 + d * (1.0 - v + v.ln()) {
                return d * v;
            }
        }
    }

    /// Poisson with mean `m` (`0 < m`, finite; below about 2^31): sequential-search inversion
    /// for `m < 10`, PTRS (Hörmann 1993, transformed rejection with squeeze) otherwise.
    pub fn poisson(&mut self, m: f64) -> f64 {
        if m < 10.0 {
            let mut p = (-m).exp();
            let mut s = p;
            let u = self.uniform();
            let mut k = 0.0;
            // the cap only matters where rounding keeps `s` below `u` near 1
            while u >= s && k < 1000.0 {
                k += 1.0;
                p *= m / k;
                s += p;
            }
            return k;
        }
        let log_m = m.ln();
        let b = 0.931 + 2.53 * m.sqrt();
        let a = -0.059 + 0.02483 * b;
        let inv_alpha = 1.1239 + 1.1328 / (b - 3.4);
        let v_r = 0.9277 - 3.6224 / (b - 2.0);
        loop {
            let u = self.uniform() - 0.5;
            let v = self.uniform();
            let us = 0.5 - u.abs();
            let k = ((2.0 * a / us + b) * u + m + 0.43).floor();
            if us >= 0.07 && v <= v_r {
                return k;
            }
            if k < 0.0 || (us < 0.013 && v > us) {
                continue;
            }
            if (v * inv_alpha / (a / (us * us) + b)).ln() <= -m + k * log_m - self.t.ln_factorial(k)
            {
                return k;
            }
        }
    }
}

impl UniformSource for FastSource {
    #[inline]
    fn next_uniform(&mut self) -> f64 {
        self.uniform()
    }

    #[inline]
    fn sample_exp(&mut self, rate: f64) -> Result<f64, DistError> {
        if !(rate > 0.0) {
            // NaN passes the reference's check `!(rate <= 0)`: the reference path handles it
            return dist::sample_exp(rate, self);
        }
        let d = Exponential::new(rate)?;
        let mean = d.mean();
        if !mean.is_finite() {
            return d.sample(self);
        }
        Ok(mean * self.std_exp())
    }

    #[inline]
    fn sample_norm(&mut self, mean: f64, sd: f64) -> Result<f64, DistError> {
        let d = Normal::new(mean, sd)?;
        if !(mean.is_finite() && sd.is_finite()) {
            return d.sample(self);
        }
        Ok(mean + sd * self.std_normal())
    }

    #[inline]
    fn sample_lognorm(&mut self, mu: f64, sigma: f64) -> Result<f64, DistError> {
        if !(sigma > 0.0) {
            return dist::sample_lognorm(mu, sigma, self);
        }
        let d = LogNormal::new(mu, sigma)?;
        fast_lognorm(self, d)
    }

    #[inline]
    fn sample_lognorm_moments(&mut self, mean: f64, stdev: f64) -> Result<f64, DistError> {
        if !(mean >= 0.0 && stdev >= 0.0) {
            return dist::sample_lognorm_moments(mean, stdev, self);
        }
        let d = LogNormal::from_moments(mean, stdev * stdev)?;
        fast_lognorm(self, d)
    }

    #[inline]
    fn sample_gamma(&mut self, alpha: f64, theta: f64) -> Result<f64, DistError> {
        if !(alpha > 0.0 && theta > 0.0) {
            return dist::sample_gamma(alpha, theta, self);
        }
        let d = Gamma::new(alpha, theta)?;
        fast_gamma(self, d)
    }

    #[inline]
    fn sample_gamma_moments(&mut self, mean: f64, coeff_var: f64) -> Result<f64, DistError> {
        if !(mean >= 0.0 && coeff_var >= 0.0) {
            return dist::sample_gamma_moments(mean, coeff_var, self);
        }
        let d = Gamma::from_moments(mean, coeff_var)?;
        fast_gamma(self, d)
    }

    #[inline]
    fn sample_pois(&mut self, mean: f64) -> Result<i32, DistError> {
        if !(mean > 0.0 && mean <= 1e9) {
            return dist::sample_pois(mean, self);
        }
        Poisson::new(mean)?;
        // the reference's integer inversion returns the largest k with F(k) <= u: Poisson − 1
        Ok(self.poisson(mean) as i32 - 1)
    }

    #[inline]
    fn sample_unidouble(&mut self, a: f64, b: f64) -> Result<f64, DistError> {
        if !(a.is_finite() && b.is_finite() && a < b) {
            // includes UniDouble(a, a), which fails in the reference's inversion (REF-3)
            return dist::sample_unidouble(a, b, self);
        }
        UniformDouble::new(a, b)?;
        Ok(a + self.uniform() * (b - a))
    }

    #[inline]
    fn sample_uniint(&mut self, a: i32, b: i32) -> Result<i32, DistError> {
        let n = i64::from(b) - i64::from(a) + 1;
        if !(1..=i64::from(i32::MAX)).contains(&n) {
            // b < a, or the reference's `int` count overflows
            return dist::sample_uniint(a, b, self);
        }
        Ok((i64::from(a) + self.below(n as u64) as i64) as i32)
    }
}

#[inline]
fn fast_lognorm(s: &mut FastSource, d: LogNormal) -> Result<f64, DistError> {
    let (mu, sigma) = d.params();
    if !(mu.is_finite() && sigma.is_finite()) {
        return d.sample(s);
    }
    Ok((mu + sigma * s.std_normal()).exp())
}

#[inline]
fn fast_gamma(s: &mut FastSource, d: Gamma) -> Result<f64, DistError> {
    let (alpha, beta) = d.params();
    if !(alpha.is_finite() && beta.is_finite()) {
        return d.sample(s);
    }
    Ok(beta * s.std_gamma(alpha))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xoshiro_reference_output() {
        // reference implementation (xoshiro256plusplus.c) with s = {1, 2, 3, 4}
        let mut x = Xoshiro256pp { s: [1, 2, 3, 4] };
        let want = [
            41_943_041u64,
            58_720_359,
            3_588_806_011_781_223,
            3_591_011_842_654_386,
            9_228_616_714_210_784_205,
        ];
        for w in want {
            assert_eq!(x.next_u64(), w);
        }
    }

    #[test]
    fn splitmix_reference_output() {
        // SplitMix64 with seed 1234567 (reference values of splitmix64.c)
        let mut s = SplitMix64(1_234_567);
        assert_eq!(s.next_u64(), 6_457_827_717_110_365_317);
        assert_eq!(s.next_u64(), 3_203_168_211_198_807_973);
    }

    #[test]
    fn ziggurat_tables_are_consistent() {
        let t = Tables::get();
        for (x, f) in [(&t.norm_x, &t.norm_f), (&t.exp_x, &t.exp_f)] {
            assert!(x.windows(2).all(|w| w[0] > w[1]), "decreasing layers");
            assert_eq!(x[256], 0.0);
            assert!(f.windows(2).all(|w| w[0] < w[1]));
            assert!((f[256] - 1.0).abs() < 1e-15);
        }
        assert!((t.ln_factorial(10.0) - 3_628_800f64.ln()).abs() < 1e-13);
        // table/series boundary
        let (lo, hi) = (t.ln_factorial(255.0), t.ln_factorial(256.0));
        assert!((hi - lo - 256f64.ln()).abs() < 1e-11, "{lo} {hi}");
    }

    #[test]
    fn lemire_below_covers_the_range() {
        let mut s = FastSource::new(3);
        let mut seen = [0u32; 7];
        for _ in 0..70_000 {
            seen[s.below(7) as usize] += 1;
        }
        assert!(
            seen.iter().all(|&c| (9_500..10_500).contains(&c)),
            "{seen:?}"
        );
    }
}

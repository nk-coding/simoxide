//! Correctly rounded natural logarithm and exponential, SimOxide's own implementation.
//!
//! [`crate::jmath`] uses these for `Math.log` and `Math.exp` unless the `hotspot-math` feature
//! is enabled. HotSpot's x86-64 intrinsics are not always correctly rounded: on uniformly spread
//! inputs `Math.exp` differs from the correctly rounded result in about 0.25 % of the cases and
//! `Math.log` in about 1e-5 (only near 1), always by one ulp. These are the only differences;
//! special cases (NaN, infinities, zeros, negative arguments) give the same values as HotSpot.
//!
//! Method: table-driven argument reduction (64 entries for `exp`, 128 for `log`), a polynomial
//! with about 2^-67 relative error, and a rounding test; the inputs whose rounding the fast path
//! cannot decide (about 0.1 %) are recomputed in double-double arithmetic (about 2^-100). Only
//! IEEE basic operations are used, so the results are the same on every platform. Exact products
//! use fused multiply-add where the CPU has it (detected at run time on x86-64) and Dekker's
//! algorithm otherwise; both give the same bits.

use crate::crmath_tables::{
    EXP2_64, INV_LN2_64, LN2_1, LN2_2, LN2_3, LN2_64_1, LN2_64_2, LN2_64_3, LOG_INV,
};

/// Relative error bound of the fast paths (about 8 times the analysed bound).
const FAST_ERR: f64 = 5.421_010_862_427_522e-20; // 2^-64
/// Relative error bound of the double-double paths.
const SLOW_ERR: f64 = 7.888_609_052_210_118e-31; // 2^-100

// ---------------------------------------------------------------------------------------------
// error-free transformations

/// `a + b = s + e` exactly.
#[inline(always)]
fn two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    let bb = s - a;
    (s, (a - (s - bb)) + (b - bb))
}

/// `a + b = s + e` exactly, for `|a| >= |b|` (or `a == 0`).
#[inline(always)]
fn fast_two_sum(a: f64, b: f64) -> (f64, f64) {
    let s = a + b;
    (s, b - (s - a))
}

/// Exact product `a * b = p + e` (no overflow or underflow in the arguments used here).
trait TwoProd {
    fn two_prod(a: f64, b: f64) -> (f64, f64);
}

/// Dekker's product with Veltkamp splitting into 26-bit halves.
struct Dekker;

impl TwoProd for Dekker {
    #[inline(always)]
    fn two_prod(a: f64, b: f64) -> (f64, f64) {
        let p = a * b;
        let split = |v: f64| {
            let t = 134_217_729.0 * v; // 2^27 + 1
            let h = t - (t - v);
            (h, v - h)
        };
        let (ah, al) = split(a);
        let (bh, bl) = split(b);
        (p, ((ah * bh - p) + ah * bl + al * bh) + al * bl)
    }
}

/// Fused multiply-add; only inside functions compiled with the `fma` target feature (otherwise
/// `mul_add` is a slow library call).
#[cfg(target_arch = "x86_64")]
struct Fma;

#[cfg(target_arch = "x86_64")]
impl TwoProd for Fma {
    #[inline(always)]
    fn two_prod(a: f64, b: f64) -> (f64, f64) {
        let p = a * b;
        (p, a.mul_add(b, -p))
    }
}

/// Exact product for the slow paths.
#[inline(always)]
fn two_prod(a: f64, b: f64) -> (f64, f64) {
    #[cfg(target_feature = "fma")]
    {
        let p = a * b;
        (p, a.mul_add(b, -p))
    }
    #[cfg(not(target_feature = "fma"))]
    Dekker::two_prod(a, b)
}

/// Calls `$generic::<Fma>` (compiled with FMA) if the CPU has FMA and the build does not assume
/// it already, else `$generic::<Dekker>` or, with FMA enabled at compile time, the default.
macro_rules! dispatch {
    ($x:expr, $generic:ident, $fma:ident) => {{
        #[cfg(all(target_arch = "x86_64", not(target_feature = "fma")))]
        {
            if std::arch::is_x86_feature_detected!("fma") {
                // SAFETY: the CPU supports FMA (checked above).
                return unsafe { $fma($x) };
            }
            $generic::<Dekker>($x)
        }
        #[cfg(target_feature = "fma")]
        {
            $generic::<Fma>($x)
        }
        #[cfg(all(not(target_arch = "x86_64"), not(target_feature = "fma")))]
        {
            $generic::<Dekker>($x)
        }
    }};
}

#[inline(always)]
fn pow2(k: i32) -> f64 {
    debug_assert!((-1022..=1023).contains(&k));
    f64::from_bits(((1023 + k) as u64) << 52)
}

/// The double nearest to `hi + lo` (`|lo| <= ulp(hi) / 2`) if every value within `err` of it
/// rounds to the same double; `None` if the rounding cannot be decided.
#[inline(always)]
fn round_checked(hi: f64, lo: f64, err: f64) -> Option<f64> {
    let a = hi + (lo - err);
    let b = hi + (lo + err);
    if a == b { Some(a) } else { None }
}

/// As [`round_checked`] for `(hi + lo) * 2^m` below the normal range: rounds to a multiple of
/// 2^-1074 (`hi` in `(0, 2.01)`, `m <= -1022`).
fn round_checked_subnormal(hi: f64, lo: f64, err: f64, m: i32) -> Option<f64> {
    // in units of 2^-1074: t = th + tl < 2^53 (all scalings exact)
    let s = pow2(1074 + m);
    let (th, tl, te) = (hi * s, lo * s, err * s);
    let n = th.round();
    let f = th - n; // exact, |f| <= 1/2
    let up = (f - 0.5) + tl; // t - (n + 1/2)
    let down = (f + 0.5) + tl; // t - (n - 1/2)
    let n = if up > te {
        n + 1.0
    } else if down < -te {
        n - 1.0
    } else if up < -te && down > te {
        n
    } else {
        return None;
    };
    // n * 2^-1074 for n < 2^53 (n >= 2^52 is the encoding of a normal number with exponent 1)
    Some(f64::from_bits(n as u64))
}

// ---------------------------------------------------------------------------------------------
// double-double arithmetic (slow paths only)

#[derive(Clone, Copy)]
struct Dd(f64, f64);

impl Dd {
    fn add(self, o: Dd) -> Dd {
        let (s, e) = two_sum(self.0, o.0);
        let (t, f) = two_sum(self.1, o.1);
        let (s, e) = fast_two_sum(s, e + t);
        let (s, e) = fast_two_sum(s, e + f);
        Dd(s, e)
    }
    fn mul(self, o: Dd) -> Dd {
        let (p, e) = two_prod(self.0, o.0);
        let (p, e) = fast_two_sum(p, e + (self.0 * o.1 + self.1 * o.0));
        Dd(p, e)
    }
    fn mul_f(self, b: f64) -> Dd {
        let (p, e) = two_prod(self.0, b);
        let (p, e) = fast_two_sum(p, e + self.1 * b);
        Dd(p, e)
    }
    fn div_f(self, b: f64) -> Dd {
        let q = self.0 / b;
        let (p, e) = two_prod(q, b);
        let r = ((self.0 - p) - e + self.1) / b;
        let (q, r) = fast_two_sum(q, r);
        Dd(q, r)
    }
}

// ---------------------------------------------------------------------------------------------
// log

/// Natural logarithm, correctly rounded. Special cases as `Math.log`: NaN for NaN and negative
/// arguments (including -inf), -inf for ±0, +inf for +inf.
pub fn log(x: f64) -> f64 {
    dispatch!(x, log_impl, log_fma)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "fma")]
unsafe fn log_fma(x: f64) -> f64 {
    log_impl::<Fma>(x)
}

#[inline(always)]
fn log_impl<P: TwoProd>(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    if x <= 0.0 {
        return if x == 0.0 {
            f64::NEG_INFINITY
        } else {
            f64::NAN
        };
    }
    if x == f64::INFINITY {
        return x;
    }
    if x == 1.0 {
        return 0.0;
    }
    let (k, m, i) = log_reduce(x);
    let (c, t_hi, t_lo) = LOG_INV[i];
    // r = m c - 1 is exact (|r| < 2^-7 and m c a multiple of 2^-60)
    let (p, pe) = P::two_prod(m, c);
    let r = (p - 1.0) + pe;
    // log x = k ln2 - ln c + log1p(r); log1p(r) = r - r^2/2 + r^3 (1/3 - r/4 + ... + r^8/11)
    let (r2, r2e) = P::two_prod(r, r);
    let tail = r
        * r2
        * (1.0 / 3.0
            + r * (-0.25
                + r * (0.2
                    + r * (-1.0 / 6.0
                        + r * (1.0 / 7.0
                            + r * (-0.125 + r * (1.0 / 9.0 + r * (-0.1 + r * (1.0 / 11.0)))))))));
    let kf = f64::from(k);
    // high parts: k LN2_1 (exact), -ln c, r, -r^2/2 (no cancellation beyond a factor of ~2:
    // k = 0 and c = 1 near x = 1)
    let (s, e1) = two_sum(kf * LN2_1, t_hi);
    let (s, e2) = two_sum(s, r);
    let (s, e3) = two_sum(s, -0.5 * r2);
    let lo = (e1 + e2 + e3) + (kf * LN2_2 + t_lo - 0.5 * r2e + tail);
    let (hi, lo) = fast_two_sum(s, lo);
    if let Some(v) = round_checked(hi, lo, hi.abs() * FAST_ERR) {
        return v;
    }
    log_slow(k, r, i)
}

/// `x = 2^k m`, with `m` in `[1, 1.40625)` or `[0.703125, 1)`, and the table index `i`.
#[inline(always)]
fn log_reduce(x: f64) -> (i32, f64, usize) {
    let mut bits = x.to_bits();
    let mut k = ((bits >> 52) as i32) - 1023;
    if k == -1023 {
        // subnormal
        bits = (x * pow2(54)).to_bits();
        k = ((bits >> 52) as i32) - 1023 - 54;
    }
    let i = ((bits >> 45) & 127) as usize;
    let mut m = f64::from_bits((bits & 0x000f_ffff_ffff_ffff) | 0x3ff0_0000_0000_0000);
    if i >= 52 {
        m *= 0.5;
        k += 1;
    }
    (k, m, i)
}

#[cold]
fn log_slow(k: i32, r: f64, i: usize) -> f64 {
    let (_, t_hi, t_lo) = LOG_INV[i];
    // log1p(r) = r (1 - r (1/2 - r (1/3 - ...))) up to r^16/16 (|r| < 2^-7)
    let mut acc = Dd(1.0, 0.0).div_f(16.0);
    for n in (1..16).rev() {
        acc = Dd(1.0, 0.0).div_f(f64::from(n)).add(acc.mul_f(-r));
    }
    let l = acc.mul_f(r);
    let kf = f64::from(k);
    let (p2, p2e) = two_prod(kf, LN2_2);
    let kl = Dd(kf * LN2_1, 0.0)
        .add(Dd(p2, p2e))
        .add(Dd(kf * LN2_3, 0.0));
    let Dd(hi, lo) = kl.add(Dd(t_hi, t_lo)).add(l);
    round_checked(hi, lo, hi.abs() * SLOW_ERR).unwrap_or(hi)
}

// ---------------------------------------------------------------------------------------------
// exp

/// Exponential, correctly rounded (also for subnormal results). Special cases as `Math.exp`:
/// NaN for NaN, +inf for +inf and overflow, 0 for -inf and underflow.
pub fn exp(x: f64) -> f64 {
    dispatch!(x, exp_impl, exp_fma)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "fma")]
unsafe fn exp_fma(x: f64) -> f64 {
    exp_impl::<Fma>(x)
}

#[inline(always)]
fn exp_impl<P: TwoProd>(x: f64) -> f64 {
    if x.is_nan() {
        return x + x;
    }
    if x > 709.79 {
        return f64::INFINITY; // ln(f64::MAX) = 709.7827...
    }
    if x < -745.2 {
        return 0.0; // exp(x) < 2^-1075
    }
    if x.abs() < 5.551_115_123_125_783e-17 {
        // |x| < 2^-54: rounds to 1
        return 1.0 + x;
    }
    // x = (64 m + j) ln2/64 + r, |r| <= ln2/128; exp(x) = 2^m 2^(j/64) exp(r)
    let nf = (x * INV_LN2_64).round();
    let n = nf as i32;
    let (j, m) = ((n & 63) as usize, n >> 6);
    let t = x - nf * LN2_64_1; // both exact (|n| < 2^17)
    let (p, pe) = P::two_prod(nf, LN2_64_2);
    let (r, re) = two_sum(t, -p);
    let r_lo = re - pe; // dropped: n LN2_64_3 (< 2^-78)
    // exp(r) - 1 = r + r^2/2 + r^3 (1/6 + r/24 + r^2/120 + r^3/720 + r^4/5040) + r_lo (1 + r)
    let (r2, r2e) = P::two_prod(r, r);
    let (u, ue) = fast_two_sum(r, 0.5 * r2);
    let u_lo = ue
        + (0.5 * r2e
            + r_lo
            + r * r_lo
            + r * r2
                * (1.0 / 6.0
                    + r * (1.0 / 24.0
                        + r * (1.0 / 120.0 + r * (1.0 / 720.0 + r * (1.0 / 5040.0))))));
    let (t_hi, t_lo) = EXP2_64[j];
    // 2^(j/64) (1 + u)
    let (p, pe) = P::two_prod(t_hi, u);
    let (s, se) = fast_two_sum(t_hi, p);
    let lo = se + pe + (t_lo + t_hi * u_lo + t_lo * u);
    let (hi, lo) = fast_two_sum(s, lo);
    let err = hi * FAST_ERR;
    let v = if m > -1022 || (m == -1022 && hi >= 1.0) {
        round_checked(hi, lo, err).map(|v| scale(v, m))
    } else {
        round_checked_subnormal(hi, lo, err, m)
    };
    v.unwrap_or_else(|| exp_slow(x, nf, j, m))
}

/// `v 2^m` for a result in the normal range or overflowing (`m <= 1024`).
#[inline(always)]
fn scale(v: f64, m: i32) -> f64 {
    if m > 1023 {
        v * pow2(1023) * 2.0
    } else {
        v * pow2(m)
    }
}

#[cold]
fn exp_slow(x: f64, nf: f64, j: usize, m: i32) -> f64 {
    let (p2, p2e) = two_prod(nf, LN2_64_2);
    let r = Dd(x - nf * LN2_64_1, 0.0)
        .add(Dd(-p2, -p2e))
        .add(Dd(-nf * LN2_64_3, 0.0));
    // exp(r) - 1 = sum r^k / k!, k = 1..13 (|r| < 2^-7)
    let mut term = r;
    let mut sum = r;
    for k in 2..=13 {
        term = term.mul(r).div_f(f64::from(k));
        sum = sum.add(term);
    }
    let (t_hi, t_lo) = EXP2_64[j];
    let t = Dd(t_hi, t_lo);
    let Dd(hi, lo) = t.add(t.mul(sum));
    let err = hi * SLOW_ERR;
    if m > -1022 || (m == -1022 && hi >= 1.0) {
        scale(round_checked(hi, lo, err).unwrap_or(hi), m)
    } else {
        round_checked_subnormal(hi, lo, err, m).unwrap_or_else(|| {
            round_checked_subnormal(hi, lo, 0.0, m).expect("exact rounding without error bound")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn special_cases() {
        assert!(log(f64::NAN).is_nan() && log(-1.0).is_nan() && log(f64::NEG_INFINITY).is_nan());
        assert_eq!(log(0.0), f64::NEG_INFINITY);
        assert_eq!(log(-0.0), f64::NEG_INFINITY);
        assert_eq!(log(f64::INFINITY), f64::INFINITY);
        assert_eq!(log(1.0).to_bits(), 0);
        assert!(exp(f64::NAN).is_nan());
        assert_eq!(exp(f64::INFINITY), f64::INFINITY);
        assert_eq!(exp(f64::NEG_INFINITY).to_bits(), 0);
        assert_eq!(exp(0.0), 1.0);
        assert_eq!(exp(-0.0), 1.0);
        assert_eq!(exp(709.78), 1.7928227943945155e308);
        assert_eq!(exp(709.783), f64::INFINITY);
        assert_eq!(exp(-745.13), 5e-324);
        assert_eq!(exp(-745.14).to_bits(), 0);
    }

    /// Both product implementations give the same results.
    #[test]
    #[cfg(target_arch = "x86_64")]
    fn fma_and_dekker_agree() {
        if !std::arch::is_x86_feature_detected!("fma") {
            return;
        }
        let mut s = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..200_000 {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            let x = f64::from_bits(s & 0x7fef_ffff_ffff_ffff);
            let y = (s >> 11) as f64 / (1u64 << 53) as f64 * 1455.0 - 745.2;
            // SAFETY: FMA support checked above.
            unsafe {
                assert_eq!(
                    log_fma(x).to_bits(),
                    log_impl::<Dekker>(x).to_bits(),
                    "log({x:e})"
                );
                assert_eq!(
                    exp_fma(y).to_bits(),
                    exp_impl::<Dekker>(y).to_bits(),
                    "exp({y:e})"
                );
            }
        }
    }

    /// The fast paths agree with the double-double paths wherever they decide the rounding.
    #[test]
    fn fast_paths_agree_with_slow_paths() {
        let mut s = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let n = if cfg!(debug_assertions) {
            20_000
        } else {
            1_000_000
        };
        for _ in 0..n {
            let x = f64::from_bits(next() & 0x7fef_ffff_ffff_ffff);
            if x != 1.0 {
                let (k, m, i) = log_reduce(x);
                let (p, pe) = two_prod(m, LOG_INV[i].0);
                let slow = log_slow(k, (p - 1.0) + pe, i);
                assert_eq!(log(x).to_bits(), slow.to_bits(), "log({x:e})");
            }
            let u = (next() >> 11) as f64 / (1u64 << 53) as f64;
            for y in [
                -745.2 + 1455.0 * u,
                2.0 * u - 1.0,
                -745.2 + 37.0 * u,
                1.0 - u,
                0.5 + u,
            ] {
                if y.abs() >= 5.6e-17 && y <= 709.79 {
                    let nf = (y * INV_LN2_64).round();
                    let n = nf as i32;
                    let slow = exp_slow(y, nf, (n & 63) as usize, n >> 6);
                    assert_eq!(exp(y).to_bits(), slow.to_bits(), "exp({y:e})");
                }
                if y > 0.0 {
                    let (k, m, i) = log_reduce(y);
                    let (p, pe) = two_prod(m, LOG_INV[i].0);
                    if y != 1.0 {
                        assert_eq!(
                            log(y).to_bits(),
                            log_slow(k, (p - 1.0) + pe, i).to_bits(),
                            "log({y:e})"
                        );
                    }
                }
            }
        }
    }
}

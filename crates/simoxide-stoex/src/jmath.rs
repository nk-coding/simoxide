//! Java arithmetic semantics needed by the StoEx evaluator.
//!
//! * integer arithmetic wraps (`int` overflow), `/` and `%` by zero are errors;
//! * `(int) double` saturates and maps NaN to 0 (Rust `as` does the same);
//! * `Math.round(double)` returns a `long` (ties towards +inf), which `Round`/`Trunc` then
//!   narrow with `(int)`, i.e. *wrapping* to 32 bits;
//! * `Double.compareTo` is a total order (`-0.0 < 0.0`, NaN equal to itself and largest);
//! * `String.compareTo` compares UTF-16 code units;
//! * `Math.log`, `Math.pow`: HotSpot on x86-64 uses Intel LIBM intrinsics, not fdlibm
//!   (`StrictMath`) and not the C library. Measured against Java 17 and 21 (same results) on
//!   300 000 inputs each: `Math.log` was correctly rounded in every case, `Math.pow` in 99.95 %
//!   (glibc: 0.16 % mismatches, fdlibm/`libm` crate: 6.7 %). This module therefore implements
//!   correctly rounded `log` and `pow` (double-double evaluation, exact handling of integer
//!   powers, Java's special cases). Known difference: the ~0.05 % of `pow` inputs where the
//!   Intel routine is 1 ulp off (see `docs/spec/stoex.md`), and possible double rounding for
//!   results in the subnormal range.

use std::cmp::Ordering;

/// `Math.round(double)` (Java 7+): closest long, ties towards positive infinity, NaN -> 0,
/// saturating.
pub fn java_round(x: f64) -> i64 {
    if x.is_nan() {
        return 0;
    }
    let f = x.floor();
    // x - floor(x) is exact for |x| < 2^52; for larger |x|, x is an integer.
    let r = if x - f >= 0.5 { f + 1.0 } else { f };
    r as i64
}

/// `Double.compare(a, b)`.
pub fn double_compare(a: f64, b: f64) -> Ordering {
    // Same as Java: compare as numbers, then by the raw bits of doubleToLongBits
    // (canonical NaN).
    if a < b {
        return Ordering::Less;
    }
    if a > b {
        return Ordering::Greater;
    }
    let ab = if a.is_nan() {
        0x7ff8_0000_0000_0000u64 as i64
    } else {
        a.to_bits() as i64
    };
    let bb = if b.is_nan() {
        0x7ff8_0000_0000_0000u64 as i64
    } else {
        b.to_bits() as i64
    };
    ab.cmp(&bb)
}

/// `String.compareTo` (UTF-16 code unit order).
pub fn string_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// `Math.max(double, double)`.
pub fn max_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.is_sign_negative() && !b.is_nan() {
        return b;
    }
    if a >= b { a } else { b }
}

/// `Math.min(double, double)`.
pub fn min_f64(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.is_sign_negative() {
        return b;
    }
    if a <= b { a } else { b }
}

// ---------------------------------------------------------------------------------------------
// double-double arithmetic

#[derive(Clone, Copy, Debug)]
struct Dd(f64, f64);

#[inline]
fn two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    let bb = s - a;
    let e = (a - (s - bb)) + (b - bb);
    Dd(s, e)
}

#[inline]
fn quick_two_sum(a: f64, b: f64) -> Dd {
    let s = a + b;
    Dd(s, b - (s - a))
}

#[inline]
fn two_prod(a: f64, b: f64) -> Dd {
    let p = a * b;
    Dd(p, a.mul_add(b, -p))
}

impl Dd {
    fn add(self, o: Dd) -> Dd {
        let s = two_sum(self.0, o.0);
        let t = two_sum(self.1, o.1);
        let s = quick_two_sum(s.0, s.1 + t.0);
        quick_two_sum(s.0, s.1 + t.1)
    }
    fn neg(self) -> Dd {
        Dd(-self.0, -self.1)
    }
    fn mul(self, o: Dd) -> Dd {
        let p = two_prod(self.0, o.0);
        quick_two_sum(p.0, p.1 + (self.0 * o.1 + self.1 * o.0))
    }
    fn mul_f(self, b: f64) -> Dd {
        let p = two_prod(self.0, b);
        quick_two_sum(p.0, p.1 + self.1 * b)
    }
    fn div(self, o: Dd) -> Dd {
        let q1 = self.0 / o.0;
        let r = self.add(o.mul_f(q1).neg());
        let q2 = r.0 / o.0;
        let r = r.add(o.mul_f(q2).neg());
        let q3 = r.0 / o.0;
        quick_two_sum(q1, q2).add(Dd(q3, 0.0))
    }
    fn div_f(self, b: f64) -> Dd {
        self.div(Dd(b, 0.0))
    }
}

const LN2: Dd = Dd(
    f64::from_bits(0x3FE6_2E42_FEFA_39EF),
    f64::from_bits(0x3C7A_BC9E_3B39_803F),
);

/// Splits a positive finite double into `m * 2^e` with `m` in `[sqrt(1/2), sqrt(2))`.
fn frexp_centered(x: f64) -> (f64, i32) {
    let (mut m, mut e) = frexp(x); // m in [0.5, 1)
    if m < std::f64::consts::FRAC_1_SQRT_2 {
        // [0.5, 0.707) -> [1, 1.414)
        m *= 2.0;
        e -= 1;
    }
    (m, e)
}

/// `x = m * 2^e`, `m` in `[0.5, 1)`, for positive finite `x`.
fn frexp(x: f64) -> (f64, i32) {
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        // subnormal: normalise
        let (m, e) = frexp(x * f64::from_bits(0x4350_0000_0000_0000)); // 2^54
        return (m, e - 54);
    }
    let m = f64::from_bits((bits & !(0x7ffu64 << 52)) | (1022u64 << 52));
    (m, exp - 1022)
}

/// Natural logarithm of a positive finite double in double-double precision (~2^-104).
fn dd_log(x: f64) -> Dd {
    let (m, e) = frexp_centered(x);
    // log(m) = 2 atanh(s), s = (m - 1) / (m + 1); m - 1 is exact.
    let num = Dd(m - 1.0, 0.0);
    let den = two_sum(m, 1.0);
    let s = num.div(den);
    let s2 = s.mul(s);
    let mut term = s;
    let mut sum = s;
    let mut k = 3.0;
    loop {
        term = term.mul(s2);
        let t = term.div_f(k);
        if t.0.abs() < 1e-36 * sum.0.abs().max(1e-300) || t.0 == 0.0 {
            break;
        }
        sum = sum.add(t);
        k += 2.0;
    }
    let lm = sum.mul_f(2.0);
    // e * ln2 (e is exact in a double; the products are exact with two_prod)
    let ef = f64::from(e);
    let el = two_prod(ef, LN2.0).add(two_prod(ef, LN2.1));
    el.add(lm)
}

/// `exp` of a double-double argument; returns the correctly rounded double (normal range).
fn dd_exp_round(t: Dd) -> f64 {
    if t.0.is_nan() {
        return f64::NAN;
    }
    if t.0 > 710.0 {
        return f64::INFINITY;
    }
    if t.0 < -746.0 {
        return 0.0;
    }
    let k = (t.0 / LN2.0).round();
    // r = t - k ln2
    let r = t
        .add(two_prod(k, LN2.0).neg())
        .add(two_prod(k, LN2.1).neg());
    // expm1(r / 1024) by Taylor, then square 10 times: e' = 2e + e^2
    let rs = Dd(r.0 / 1024.0, r.1 / 1024.0);
    let mut term = rs;
    let mut e = rs;
    let mut n = 2.0;
    loop {
        term = term.mul(rs).div_f(n);
        if term.0.abs() < 1e-40 {
            break;
        }
        e = e.add(term);
        n += 1.0;
        if n > 30.0 {
            break;
        }
    }
    for _ in 0..10 {
        e = e.mul_f(2.0).add(e.mul(e));
    }
    let v = Dd(1.0, 0.0).add(e);
    let v = quick_two_sum(v.0, v.1);
    scale(v.0, k as i32)
}

/// `v * 2^k` with a single rounding in the normal range.
fn scale(v: f64, k: i32) -> f64 {
    let mut v = v;
    let mut k = k;
    while k > 1000 {
        v *= f64::from_bits(((1023 + 1000) as u64) << 52);
        k -= 1000;
    }
    while k < -1000 {
        v *= f64::from_bits(((1023 - 1000) as u64) << 52);
        k += 1000;
    }
    v * f64::from_bits(((1023 + k) as u64) << 52)
}

/// The NaN x86-64 SSE produces for invalid operations ("real indefinite", sign bit set); this
/// is what HotSpot's `Math.log(-1)` or `Math.pow(-2, 0.5)` return.
pub const INDEFINITE_NAN: f64 = f64::from_bits(0xfff8_0000_0000_0000);

/// Java's `double % double` (`drem`) as executed by HotSpot 21 on x86-64: the exact IEEE
/// `fmod` value, with these NaN bit patterns (Java 17 instead returns glibc's, i.e. a NaN `x`
/// propagated unchanged; only NaN payloads differ):
/// `NaN % y` -> `0x7ff8..`; `±inf % NaN` -> `0x7ff8..`; `finite % NaN` -> that NaN (quieted);
/// `x % ±0` and `±inf % y` -> `0xfff8..`.
pub fn drem(x: f64, y: f64) -> f64 {
    if x.is_nan() {
        return f64::from_bits(0x7ff8_0000_0000_0000);
    }
    if y.is_nan() {
        return if x.is_infinite() {
            f64::from_bits(0x7ff8_0000_0000_0000)
        } else {
            quiet(y)
        };
    }
    if y == 0.0 || x.is_infinite() {
        return INDEFINITE_NAN;
    }
    x % y
}

/// `a - b` exactly as an SSE `subsd` (a NaN `b` keeps its sign). Kept out of line: LLVM may
/// otherwise merge `a + b` / `a - b` into `a + select(-b, b)`, which flips the sign of a NaN
/// `b` (only NaN payloads are affected, but the golden files compare bits).
#[inline(never)]
pub fn dsub(a: f64, b: f64) -> f64 {
    a - b
}

/// A NaN operand, quieted (what SSE arithmetic returns for a NaN input).
fn quiet(x: f64) -> f64 {
    f64::from_bits(x.to_bits() | 0x0008_0000_0000_0000)
}

/// Correctly rounded natural logarithm with `Math.log` special cases (NaN bit patterns as on
/// HotSpot x86-64).
pub fn log(x: f64) -> f64 {
    if x.is_nan() {
        return quiet(x);
    }
    if x < 0.0 {
        return INDEFINITE_NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() {
        return f64::INFINITY;
    }
    if x == 1.0 {
        return 0.0;
    }
    let d = dd_log(x);
    let r = quick_two_sum(d.0, d.1);
    r.0
}

fn is_integer(y: f64) -> bool {
    y.is_finite() && y == y.trunc()
}

fn is_odd_integer(y: f64) -> bool {
    // doubles >= 2^53 are even
    is_integer(y) && y.abs() < 9_007_199_254_740_992.0 && (y as i64) % 2 != 0
}

/// `Math.pow(x, y)` (correctly rounded, Java special cases).
pub fn pow(x: f64, y: f64) -> f64 {
    // Java special cases (java.lang.Math.pow javadoc)
    if y == 0.0 {
        return 1.0;
    }
    if y == 1.0 {
        return x;
    }
    if x.is_nan() {
        return quiet(x);
    }
    if y.is_nan() {
        return quiet(y);
    }
    let ax = x.abs();
    if y.is_infinite() {
        if ax == 1.0 {
            return f64::from_bits(0x7ff8_0000_0000_0000);
        }
        return if (ax > 1.0) == (y > 0.0) {
            f64::INFINITY
        } else {
            0.0
        };
    }
    if x == 0.0 || x.is_infinite() {
        let neg = x.is_sign_negative();
        let odd = is_odd_integer(y);
        // |x| = 0 or inf: result is 0 or inf; sign negative only for x<0 and odd integer y.
        let big = (x == 0.0) == (y < 0.0);
        let mag = if big { f64::INFINITY } else { 0.0 };
        return if neg && odd { -mag } else { mag };
    }
    if x < 0.0 {
        if !is_integer(y) {
            return INDEFINITE_NAN;
        }
        let r = pow_pos(ax, y);
        return if is_odd_integer(y) { -r } else { r };
    }
    pow_pos(x, y)
}

fn pow_pos(x: f64, y: f64) -> f64 {
    if x == 1.0 {
        return 1.0;
    }
    if y == 0.5 {
        // correctly rounded like pow; also what HotSpot does for this exponent
        return x.sqrt();
    }
    if let Some(r) = exact_int_pow(x, y) {
        return r;
    }
    let l = dd_log(x);
    let t0 = l.0 * y;
    if t0 > 800.0 {
        return f64::INFINITY;
    }
    if t0 < -800.0 {
        return 0.0;
    }
    dd_exp_round(l.mul_f(y))
}

/// Exact result for `x^n` with a positive integer `n` when the odd part of `x` raised to `n`
/// fits in 128 bits, rounded once (ties to even). Returns None if not applicable or the result
/// is outside the normal range.
fn exact_int_pow(x: f64, y: f64) -> Option<f64> {
    if !(2.0..=128.0).contains(&y) || !is_integer(y) {
        return None;
    }
    let n = y as u32;
    let bits = x.to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32;
    if exp == 0 {
        return None;
    }
    let mant = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
    let tz = mant.trailing_zeros();
    let m = mant >> tz; // odd
    let e = exp - 1075 + tz as i32; // x = m * 2^e
    let mbits = 64 - m.leading_zeros();
    if u64::from(mbits) * u64::from(n) > 127 {
        return None;
    }
    let mut p: u128 = 1;
    for _ in 0..n {
        p *= u128::from(m);
    }
    let pe = i64::from(e) * i64::from(n);
    // round p to 53 bits, ties to even
    let pb = 128 - p.leading_zeros();
    let (q, shift) = if pb > 53 {
        let sh = pb - 53;
        let q = p >> sh;
        let rem = p & ((1u128 << sh) - 1);
        let half = 1u128 << (sh - 1);
        let q = if rem > half || (rem == half && q & 1 == 1) {
            q + 1
        } else {
            q
        };
        (q, sh as i64)
    } else {
        (p, 0)
    };
    let total_e = pe + shift;
    // q < 2^54; value = q * 2^total_e; require normal range
    let qf = q as f64; // exact (q <= 2^53)
    let top = total_e + (64 - (q as u64).leading_zeros()) as i64 - 1;
    if !(-1021..=1023).contains(&top) {
        return None;
    }
    Some(scale(qf, total_e as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_like_java() {
        assert_eq!(java_round(0.5), 1);
        assert_eq!(java_round(-0.5), 0);
        assert_eq!(java_round(-1.5), -1);
        assert_eq!(java_round(0.49999999999999994), 0);
        assert_eq!(java_round(f64::NAN), 0);
        assert_eq!(java_round(1e300), i64::MAX);
        assert_eq!(java_round(-1e300), i64::MIN);
        assert_eq!(java_round(4503599627370497.0), 4503599627370497);
    }

    #[test]
    fn compare_like_java() {
        assert_eq!(double_compare(-0.0, 0.0), Ordering::Less);
        assert_eq!(double_compare(f64::NAN, f64::NAN), Ordering::Equal);
        assert_eq!(double_compare(f64::NAN, f64::INFINITY), Ordering::Greater);
        assert_eq!(string_compare("\u{ffff}", "\u{10000}"), Ordering::Greater);
    }

    #[test]
    fn pow_special_cases() {
        assert!(pow(1.0, f64::NAN).is_nan());
        assert!(pow(1.0, f64::INFINITY).is_nan());
        assert!(pow(-1.0, f64::NEG_INFINITY).is_nan());
        assert_eq!(pow(f64::NAN, 0.0), 1.0);
        assert_eq!(pow(-0.0, 3.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(pow(-0.0, 2.0).to_bits(), 0.0f64.to_bits());
        assert_eq!(pow(-0.0, -3.0), f64::NEG_INFINITY);
        assert_eq!(pow(f64::NEG_INFINITY, 3.0), f64::NEG_INFINITY);
        assert_eq!(pow(f64::NEG_INFINITY, -3.0).to_bits(), (-0.0f64).to_bits());
        assert!(pow(-2.0, 0.5).is_nan());
        assert_eq!(pow(-2.0, 3.0), -8.0);
        assert_eq!(pow(2.0, 10.0), 1024.0);
        assert_eq!(pow(82.0, 10.0), 13744803133596057600.0); // exact tie, even
        assert_eq!(pow(2.0, -1074.0), f64::from_bits(1));
    }

    #[test]
    fn log_values() {
        assert_eq!(log(1.0), 0.0);
        assert_eq!(log(std::f64::consts::E), 1.0);
        assert_eq!(log(1.5452940029559548), 0.43521418542163437);
        assert_eq!(log(f64::MIN_POSITIVE / 4.0), -709.782712893384); // subnormal input
    }
}

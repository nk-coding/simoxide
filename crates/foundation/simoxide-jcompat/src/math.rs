//! `java.lang.Math` and `Double` basics whose results the javadoc specifies: `Math.round`,
//! `Math.max`, `Math.min`, `Double.compare`. (`Math.log`/`exp`/`pow` are in `simoxide-random`
//! and `simoxide-stoex`; HotSpot's bit-exact intrinsics in `simoxide-hotspot-math`.)

pub use crate::hash::double_compare;

/// `Math.round(double)` (Java 7+): the closest `long`, ties toward positive infinity; NaN gives
/// 0, out-of-range values saturate.
#[inline]
pub fn round(x: f64) -> i64 {
    if x.is_nan() {
        return 0;
    }
    let f = x.floor();
    // x - floor(x) is exact for |x| < 2^52; for larger |x|, x is an integer.
    let r = if x - f >= 0.5 { f + 1.0 } else { f };
    r as i64
}

/// `Math.max(double, double)`: NaN if either is NaN (the NaN argument itself),
/// `max(-0.0, 0.0) = 0.0`.
#[inline]
pub fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && a.is_sign_negative() {
        return b;
    }
    if a >= b { a } else { b }
}

/// `Math.min(double, double)`: NaN if either is NaN (the NaN argument itself),
/// `min(-0.0, 0.0) = -0.0`.
#[inline]
pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return a;
    }
    if a == 0.0 && b == 0.0 && b.is_sign_negative() {
        return b;
    }
    if a <= b { a } else { b }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn javadoc_cases() {
        assert_eq!(round(2.5), 3);
        assert_eq!(round(-2.5), -2);
        assert_eq!(round(f64::NAN), 0);
        assert_eq!(round(1e300), i64::MAX);
        assert_eq!(round(-1e300), i64::MIN);
        assert!(max(f64::NAN, 1.0).is_nan() && max(1.0, f64::NAN).is_nan());
        assert!(min(f64::NAN, 1.0).is_nan() && min(1.0, f64::NAN).is_nan());
        assert!(max(-0.0, 0.0).is_sign_positive() && max(0.0, -0.0).is_sign_positive());
        assert!(min(-0.0, 0.0).is_sign_negative() && min(0.0, -0.0).is_sign_negative());
        assert_eq!(max(1.0, 2.0), 2.0);
        assert_eq!(min(1.0, 2.0), 1.0);
    }
}

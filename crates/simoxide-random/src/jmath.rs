//! `java.lang.Math` semantics as executed by HotSpot on x86-64.
//!
//! `Math.log` and `Math.exp` are intrinsics backed by the Intel LIBM stubs, neither fdlibm
//! (`StrictMath`) nor the C library, and not always correctly rounded. With the `hotspot-math`
//! feature, [`log`] and [`exp`] are the bit-exact port of those stubs in the
//! `simoxide-hotspot-math` crate, which is GPL-2.0-only (see its documentation and `LICENSES`).
//! Without it they are SimOxide's correctly rounded [`crate::crmath`] functions, which differ
//! from HotSpot by one ulp on about 0.25 % of `exp` and 1e-5 of `log` arguments.

/// `Math.log(x)`.
#[inline]
pub fn log(x: f64) -> f64 {
    #[cfg(feature = "hotspot-math")]
    return simoxide_hotspot_math::log(x);
    #[cfg(not(feature = "hotspot-math"))]
    return crate::crmath::log(x);
}

/// `Math.exp(x)`.
#[inline]
pub fn exp(x: f64) -> f64 {
    #[cfg(feature = "hotspot-math")]
    return simoxide_hotspot_math::exp(x);
    #[cfg(not(feature = "hotspot-math"))]
    return crate::crmath::exp(x);
}

/// `Math.max(a, b)`: NaN if either is NaN; `max(-0.0, 0.0) = 0.0`.
#[inline]
pub fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() { b } else { a }
    } else if a >= b {
        a
    } else {
        b
    }
}

/// `Math.min(a, b)`: NaN if either is NaN; `min(-0.0, 0.0) = -0.0`.
#[inline]
pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else if a == 0.0 && b == 0.0 {
        if a.is_sign_negative() { a } else { b }
    } else if a <= b {
        a
    } else {
        b
    }
}

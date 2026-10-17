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

pub use simoxide_jcompat::math::{max, min};

//! Simulation time as the reference engine represents it.
//!
//! SimuLizar 5.2.2 runs on DESMO-J 2.3.3, configured by `DesmoJExperiment` with an epsilon of
//! one nanosecond and a reference unit of one second. Every point in time is therefore an
//! integer number of nanoseconds (`TimeInstant._timeInEpsilon`, a Java `long`), and every delay
//! handed to the engine as a `double` number of seconds is converted by `new TimeSpan(double)`:
//!
//! ```java
//! this._durationInEpsilon = (long)(d * (double)TimeOperations.getEpsilon().convert(1L, timeUnit));
//! ```
//!
//! i.e. `(long)(d * 1e9)`: **truncation toward zero**, not rounding. The current time seen by
//! model code (`getCurrentSimulationTime()`) is `(double) nanos / 1e9`.
//!
//! The resource models in this crate take and return [`SimTime`] values and perform exactly
//! these conversions, so the event core must use integer nanoseconds as its clock.

/// A point in simulation time in nanoseconds (DESMO-J `TimeInstant` with epsilon = 1 ns).
pub type SimTime = i64;

/// Nanoseconds per second as a double, `(double) NANOSECONDS.convert(1, SECONDS)`.
pub const NANOS_PER_SECOND: f64 = 1e9;

/// Converts a delay in seconds to nanoseconds like DESMO-J's `new TimeSpan(double)`:
/// `(long)(d * 1e9)`, truncating toward zero.
///
/// Rust's saturating `as` cast has the same semantics as Java's `(long)` cast (NaN becomes 0,
/// out-of-range values saturate). DESMO-J additionally aborts the simulation for negative results
/// and for `Long.MAX_VALUE`; callers that can produce such delays must check with
/// [`checked_span`].
#[inline]
pub fn span(seconds: f64) -> SimTime {
    (seconds * NANOS_PER_SECOND) as i64
}

/// Like [`span`], but returns `None` where DESMO-J would abort the simulation
/// (a negative span or a span of `Long.MAX_VALUE`).
#[inline]
pub fn checked_span(seconds: f64) -> Option<SimTime> {
    let s = span(seconds);
    (s >= 0 && s != i64::MAX).then_some(s)
}

/// The double value model code sees for a point in time:
/// `TimeInstant.getTimeAsDouble()` = `(double) nanos / 1e9`.
#[inline]
pub fn seconds(t: SimTime) -> f64 {
    t as f64 / NANOS_PER_SECOND
}

/// How a simulator encodes points in time as [`SimTime`] values: the conversions the resource
/// models (and an event core) need. Implementations are zero-sized and monomorphized, so the
/// choice costs nothing per call.
///
/// * [`Nanos`] (the default everywhere): DESMO-J's integer nanoseconds, delays truncated by
///   [`span`] (SimuLizar).
/// * [`F64Secs`]: a `double` number of seconds stored as its IEEE bit pattern, delays added with
///   one double addition as SSJ does (Slingshot). For non-negative times the bit pattern orders
///   like the value, so an event list keyed by `(SimTime, seq)` keeps working unchanged.
pub trait Clock: Copy + Default + std::fmt::Debug + Send + Sync + 'static {
    /// The double value of a point in time, in seconds.
    fn secs(t: SimTime) -> f64;
    /// `t + d` for a delay of `d` seconds (`d` finite and not negative, see
    /// [`Clock::checked_after`]).
    fn after(t: SimTime, d: f64) -> SimTime;
    /// `t + d`, or `None` where the engine aborts (a negative or non-finite delay; for
    /// [`Nanos`], DESMO-J's checks of [`checked_span`]).
    fn checked_after(t: SimTime, d: f64) -> Option<SimTime>;
    /// The point in time of `s` seconds after the start.
    fn at_secs(s: f64) -> SimTime;
    /// The point in time in nanoseconds (truncated), for APIs that speak nanoseconds.
    fn to_ns(t: SimTime) -> i64;
    /// The first point in time not before `ns` nanoseconds.
    fn from_ns(ns: i64) -> SimTime;
}

/// Integer nanoseconds (DESMO-J, SimuLizar): see the module documentation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Nanos;

impl Clock for Nanos {
    #[inline(always)]
    fn secs(t: SimTime) -> f64 {
        seconds(t)
    }
    #[inline(always)]
    fn after(t: SimTime, d: f64) -> SimTime {
        t + span(d)
    }
    #[inline(always)]
    fn checked_after(t: SimTime, d: f64) -> Option<SimTime> {
        checked_span(d).map(|s| t + s)
    }
    #[inline(always)]
    fn at_secs(s: f64) -> SimTime {
        span(s)
    }
    #[inline(always)]
    fn to_ns(t: SimTime) -> i64 {
        t
    }
    #[inline(always)]
    fn from_ns(ns: i64) -> SimTime {
        ns
    }
}

/// A `double` number of seconds as its bit pattern (SSJ, Slingshot: `t = now + delay`, one
/// IEEE addition, no quantisation). Only non-negative, finite times occur.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct F64Secs;

impl Clock for F64Secs {
    #[inline(always)]
    fn secs(t: SimTime) -> f64 {
        f64::from_bits(t as u64)
    }
    #[inline(always)]
    fn after(t: SimTime, d: f64) -> SimTime {
        // `+ 0.0` turns a -0.0 sum into 0.0 (the bit pattern must stay non-negative)
        ((Self::secs(t) + d) + 0.0).to_bits() as SimTime
    }
    #[inline(always)]
    fn checked_after(t: SimTime, d: f64) -> Option<SimTime> {
        let s = Self::secs(t) + d;
        (d >= 0.0 && s.is_finite()).then(|| (s + 0.0).to_bits() as SimTime)
    }
    #[inline(always)]
    fn at_secs(s: f64) -> SimTime {
        (s + 0.0).to_bits() as SimTime
    }
    #[inline(always)]
    fn to_ns(t: SimTime) -> i64 {
        span(Self::secs(t))
    }
    #[inline(always)]
    fn from_ns(ns: i64) -> SimTime {
        Self::at_secs(ns as f64 / NANOS_PER_SECOND)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_like_java() {
        assert_eq!(span(1.0), 1_000_000_000);
        // 1.7499999999999998 * 1e9 = 1749999999.9999998 -> truncated, not rounded
        assert_eq!(span(1.749_999_999_999_999_8), 1_749_999_999);
        assert_eq!(span(0.3), 300_000_000);
        assert_eq!(span(1.75), 1_750_000_000);
        assert_eq!(span(0.0), 0);
        assert_eq!(span(-0.0), 0);
        assert_eq!(span(0.9e-9), 0);
        assert_eq!(span(f64::NAN), 0);
        assert_eq!(span(1e300), i64::MAX);
        assert_eq!(checked_span(1e300), None);
        assert_eq!(checked_span(-1.0), None);
        assert_eq!(checked_span(-1e-10), Some(0));
    }

    #[test]
    fn f64_clock_orders_like_its_values() {
        let t0 = F64Secs::at_secs(0.0);
        assert_eq!(t0, 0);
        let a = F64Secs::after(t0, 0.1);
        let b = F64Secs::after(a, 0.2);
        assert_eq!(F64Secs::secs(b), 0.1 + 0.2);
        assert!(t0 < a && a < b);
        assert!(F64Secs::after(b, 1e-300) >= b);
        assert_eq!(F64Secs::checked_after(a, -1e-12), None);
        assert_eq!(F64Secs::checked_after(a, f64::INFINITY), None);
        assert_eq!(F64Secs::checked_after(t0, -0.0), Some(0));
        assert_eq!(F64Secs::to_ns(F64Secs::at_secs(1.5)), 1_500_000_000);
        assert_eq!(Nanos::after(5, 1e-9), 6);
        assert_eq!(Nanos::checked_after(5, -1.0), None);
    }

    #[test]
    fn seconds_is_plain_division() {
        assert_eq!(seconds(1_749_999_999), 1_749_999_999_f64 / 1e9);
        assert_eq!(seconds(0), 0.0);
    }
}

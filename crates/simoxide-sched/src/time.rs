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
    fn seconds_is_plain_division() {
        assert_eq!(seconds(1_749_999_999), 1_749_999_999_f64 / 1e9);
        assert_eq!(seconds(0), 0.0);
    }
}

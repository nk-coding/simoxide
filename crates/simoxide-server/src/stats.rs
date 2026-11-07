//! Incremental statistics of measurement series, for live progress and the confidence stop.
//!
//! A [`SeriesTracker`] consumes the rows a series gained since the last call (rows are only ever
//! appended during a run), so a progress update costs time proportional to the new rows, not to
//! the whole series. It keeps:
//!
//! * the running mean and variance (shifted sums), minimum and maximum of the values after the
//!   warm-up;
//! * the time-weighted mean of the value as a step function (meaningful for state and
//!   utilisation series);
//! * batch means for a confidence interval of the mean that tolerates autocorrelated simulation
//!   output ([`BatchMeans`]).
//!
//! Student's t quantile is computed from the regularised incomplete beta function (the same
//! method as `simoxide-testkit`'s `stats` module, which is test infrastructure and not a
//! dependency of the server).

use serde::{Deserialize, Serialize};

/// An interval needs this many batches: the sample variance of fewer is undefined.
pub const MIN_BATCHES_FOR_INTERVAL: usize = 2;

/// The `g` of the Lanczos approximation of [`ln_gamma`].
const LANCZOS_G: f64 = 7.0;
/// The coefficients of the Lanczos approximation of [`ln_gamma`] (g = 7, n = 9).
const LANCZOS_COEFFICIENTS: [f64; 9] = [
    0.999_999_999_999_809_9,
    676.520_368_121_885_1,
    -1_259.139_216_722_402_8,
    771.323_428_777_653_1,
    -176.615_029_162_140_6,
    12.507_343_278_686_905,
    -0.138_571_095_265_720_12,
    9.984_369_578_019_572e-6,
    1.505_632_735_149_311_6e-7,
];
/// Stands in for a zero in the modified Lentz method of [`inc_beta`].
const LENTZ_TINY: f64 = 1e-300;
/// The continued fraction of [`inc_beta`] stops at a factor this close to 1 ...
const LENTZ_TOLERANCE: f64 = 1e-15;
/// ... or after this many terms.
const LENTZ_MAX_TERMS: u32 = 1000;
/// [`t_quantile`] first brackets the quantile in `[0, T_INITIAL_UPPER_BOUND]` and doubles the
/// upper bound until the bracket contains it (a level close to 1 with few degrees of freedom
/// needs a large quantile: 63 662 at 99.999 % and one degree of freedom).
const T_INITIAL_UPPER_BOUND: f64 = 16.0;
/// Bisection steps of [`t_quantile`] (the bracket shrinks to far below `f64` resolution).
const T_BISECTION_STEPS: u32 = 200;
/// [`t_quantile_cached`] starts over beyond this many entries.
const T_CACHE_CAPACITY: usize = 4096;

/// `ln Γ(x)` (Lanczos approximation, g = 7, n = 9; reflection for `x < 0.5`).
pub fn ln_gamma(x: f64) -> f64 {
    use std::f64::consts::PI;
    if x < 0.5 {
        return (PI / (PI * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = LANCZOS_COEFFICIENTS[0];
    let t = x + LANCZOS_G + 0.5;
    for (i, c) in LANCZOS_COEFFICIENTS.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    0.5 * (2.0 * PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// Regularised incomplete beta function `I_x(a, b)`.
pub fn inc_beta(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    if x * (a + b + 2.0) < a + 1.0 {
        inc_beta_cf(a, b, x)
    } else {
        1.0 - inc_beta_cf(b, a, 1.0 - x)
    }
}

/// The continued fraction of `I_x(a, b)` (DLMF 8.17.22), modified Lentz method.
fn inc_beta_cf(a: f64, b: f64, x: f64) -> f64 {
    let prefactor =
        (a * x.ln() + b * (1.0 - x).ln() + ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b)).exp() / a;
    let numerator = |j: u32| -> f64 {
        if j == 1 {
            return 1.0;
        }
        let n = j - 1;
        let m = f64::from(n / 2);
        if n % 2 == 1 {
            -(a + m) * (a + b + m) * x / ((a + 2.0 * m) * (a + 2.0 * m + 1.0))
        } else {
            m * (b - m) * x / ((a + 2.0 * m - 1.0) * (a + 2.0 * m))
        }
    };
    let nonzero = |v: f64| if v == 0.0 { LENTZ_TINY } else { v };
    let mut f = LENTZ_TINY;
    let (mut c, mut d) = (f, 0.0);
    for j in 1..LENTZ_MAX_TERMS {
        let aj = numerator(j);
        d = 1.0 / nonzero(1.0 + aj * d);
        c = nonzero(1.0 + aj / c);
        let delta = c * d;
        f *= delta;
        if (delta - 1.0).abs() <= LENTZ_TOLERANCE {
            break;
        }
    }
    prefactor * f
}

/// Student t CDF with `df` degrees of freedom.
pub fn t_cdf(t: f64, df: f64) -> f64 {
    let x = df / (df + t * t);
    let tail = 0.5 * inc_beta(df / 2.0, 0.5, x);
    if t >= 0.0 { 1.0 - tail } else { tail }
}

/// Student t quantile: `t` with `t_cdf(t, df) = p` (bisection); infinite for `p >= 1` and for
/// quantiles beyond what `f64` resolves.
pub fn t_quantile(p: f64, df: f64) -> f64 {
    if p < 0.5 {
        // the distribution is symmetric
        return -t_quantile(1.0 - p, df);
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    let (mut lo, mut hi) = (0.0, T_INITIAL_UPPER_BOUND);
    while t_cdf(hi, df) < p {
        lo = hi;
        hi *= 2.0;
        if !hi.is_finite() {
            return f64::INFINITY;
        }
    }
    for _ in 0..T_BISECTION_STEPS {
        let mid = 0.5 * (lo + hi);
        if t_cdf(mid, df) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// [`t_quantile`] memoised per thread (a confidence check asks for the same few quantiles
/// again and again).
pub fn t_quantile_cached(p: f64, df: f64) -> f64 {
    use std::cell::RefCell;
    use std::collections::HashMap;
    thread_local! {
        static CACHE: RefCell<HashMap<(u64, u64), f64>> = RefCell::new(HashMap::new());
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if c.len() > T_CACHE_CAPACITY {
            c.clear();
        }
        *c.entry((p.to_bits(), df.to_bits()))
            .or_insert_with(|| t_quantile(p, df))
    })
}

/// How the batch means of a confidence interval are formed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Batching {
    /// Batches of a fixed size; the interval needs at least `min_batches` complete batches
    /// (SimuCom's static batch-means algorithm).
    Static {
        /// Values per batch.
        size: u64,
        /// Complete batches needed for an interval.
        min_batches: usize,
    },
    /// A fixed number of batches whose size doubles as the series grows: between `min` and
    /// `2 * min` batches once there are `min` values (batch size 1 at first). The batch size
    /// grows with the series, so the batch means become nearly independent on long runs.
    Doubling {
        /// The fewest batches, once there are this many values.
        min: usize,
    },
}

/// Batch means of a series, updated value by value.
#[derive(Clone, Debug)]
pub struct BatchMeans {
    /// How the batches are formed.
    pub batching: Batching,
    /// Current batch size.
    pub size: u64,
    /// Means of the complete batches.
    pub means: Vec<f64>,
    /// Sum of the values of the batch being filled.
    cur_sum: f64,
    /// Values of the batch being filled.
    cur_n: u64,
}

impl BatchMeans {
    /// No values yet.
    pub fn new(batching: Batching) -> BatchMeans {
        let size = match batching {
            Batching::Static { size, .. } => size.max(1),
            Batching::Doubling { .. } => 1,
        };
        BatchMeans {
            batching,
            size,
            means: Vec::new(),
            cur_sum: 0.0,
            cur_n: 0,
        }
    }

    /// Adds a value; completes a batch, and merges batches when [`Batching::Doubling`] has
    /// reached twice its minimum.
    pub fn push(&mut self, v: f64) {
        self.cur_sum += v;
        self.cur_n += 1;
        if self.cur_n < self.size {
            return;
        }
        self.means.push(self.cur_sum / self.cur_n as f64);
        self.cur_sum = 0.0;
        self.cur_n = 0;
        if let Batching::Doubling { min } = self.batching {
            let min = min.max(MIN_BATCHES_FOR_INTERVAL);
            if self.means.len() >= min.saturating_mul(2) {
                // merge neighbours: half as many batches of twice the size
                let merged: Vec<f64> = self
                    .means
                    .chunks_exact(2)
                    .map(|c| 0.5 * (c[0] + c[1]))
                    .collect();
                self.means = merged;
                self.size *= 2;
            }
        }
    }

    /// The batches needed before an interval is computed.
    pub fn min_batches(&self) -> usize {
        match self.batching {
            Batching::Static { min_batches, .. } => min_batches.max(MIN_BATCHES_FOR_INTERVAL),
            Batching::Doubling { min } => min.max(MIN_BATCHES_FOR_INTERVAL),
        }
    }

    /// The confidence interval of the mean at `level` (e.g. 0.95), or `None` with too few
    /// batches.
    pub fn interval(&self, level: f64) -> Option<Interval> {
        let k = self.means.len();
        if k < self.min_batches() {
            return None;
        }
        let kf = k as f64;
        let mean = self.means.iter().sum::<f64>() / kf;
        let var = self
            .means
            .iter()
            .map(|m| (m - mean) * (m - mean))
            .sum::<f64>()
            / (kf - 1.0);
        let t = t_quantile_cached(0.5 + level / 2.0, kf - 1.0);
        let half_width = t * (var / kf).sqrt();
        // lag-1 autocorrelation of the batch means: near 0 when the batches are long enough
        let lag1 = if var > 0.0 {
            self.means
                .windows(2)
                .map(|w| (w[0] - mean) * (w[1] - mean))
                .sum::<f64>()
                / ((kf - 1.0) * var)
        } else {
            0.0
        };
        Some(Interval {
            level,
            mean,
            half_width,
            relative_half_width: if mean != 0.0 {
                half_width / mean.abs()
            } else {
                f64::INFINITY
            },
            batches: k,
            batch_size: self.size,
            lag1_autocorrelation: lag1,
        })
    }
}

/// A confidence interval of a series' mean from batch means.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    /// Confidence level (e.g. 0.95).
    pub level: f64,
    /// Mean of the batch means (the values of complete batches).
    pub mean: f64,
    /// Half width: the interval is `mean ± half_width`.
    pub half_width: f64,
    /// `half_width / |mean|` (infinite for a zero mean).
    #[serde(serialize_with = "finite_or_null")]
    pub relative_half_width: f64,
    /// Number of batches the interval is computed from.
    pub batches: usize,
    /// Values per batch.
    pub batch_size: u64,
    /// Lag-1 autocorrelation of the batch means: close to 0 when the batches are long enough
    /// to be nearly independent; well above 0 means the interval is too narrow.
    pub lag1_autocorrelation: f64,
}

/// JSON has no infinity or NaN: such values become `null`.
pub fn finite_or_null<S: serde::Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
    if v.is_finite() {
        s.serialize_f64(*v)
    } else {
        s.serialize_none()
    }
}

/// `Option<f64>` with non-finite values as `null`.
pub fn opt_finite_or_null<S: serde::Serializer>(v: &Option<f64>, s: S) -> Result<S::Ok, S::Error> {
    match v {
        Some(x) if x.is_finite() => s.serialize_f64(*x),
        _ => s.serialize_none(),
    }
}

/// Running statistics of one series, fed with the rows it gained since the last update.
#[derive(Clone, Debug)]
pub struct SeriesTracker {
    /// Rows of the series consumed so far (warm-up included).
    pub seen: usize,
    /// Rows after the warm-up.
    pub n: u64,
    /// Shift (the first value) and the sums of the shifted values and their squares: as
    /// stable as Welford's update for the variance when the shift is near the mean, without its
    /// division per value.
    shift: f64,
    sum: f64,
    sum2: f64,
    /// Smallest value after the warm-up.
    pub min: f64,
    /// Largest value after the warm-up.
    pub max: f64,
    /// Time and value of the first row after the warm-up.
    pub first: Option<(f64, f64)>,
    /// Time and value of the last row after the warm-up.
    pub last: Option<(f64, f64)>,
    /// Integral of the value over time (step function) after the warm-up.
    area: f64,
}

/// Rows to skip at the start of every series.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Warmup {
    /// Rows earlier than this simulation time (s) are not counted.
    pub time: f64,
    /// The first rows of each series are not counted.
    pub samples: u64,
}

impl Default for SeriesTracker {
    fn default() -> Self {
        SeriesTracker {
            seen: 0,
            n: 0,
            shift: 0.0,
            sum: 0.0,
            sum2: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            first: None,
            last: None,
            area: 0.0,
        }
    }
}

impl SeriesTracker {
    /// Consumes `rows[self.seen..]`, passing every value after the warm-up to `each` as well.
    /// Rows never disappear during a run: a series shorter than what was seen is a bug of the
    /// caller, and the tracker starts over.
    pub fn update(&mut self, rows: &[(f64, f64)], warmup: Warmup, each: &mut dyn FnMut(f64)) {
        if rows.len() < self.seen {
            *self = SeriesTracker::default();
        }
        for (i, &(t, v)) in rows.iter().enumerate().skip(self.seen) {
            if (i as u64) < warmup.samples || t < warmup.time {
                continue;
            }
            if self.n == 0 {
                self.shift = v;
            }
            self.n += 1;
            let d = v - self.shift;
            self.sum += d;
            self.sum2 += d * d;
            self.min = self.min.min(v);
            self.max = self.max.max(v);
            if let Some((t0, v0)) = self.last {
                self.area += v0 * (t - t0);
            }
            if self.first.is_none() {
                self.first = Some((t, v));
            }
            self.last = Some((t, v));
            each(v);
        }
        self.seen = rows.len();
    }

    /// Mean of the values after the warm-up.
    pub fn mean(&self) -> Option<f64> {
        (self.n > 0).then(|| self.shift + self.sum / self.n as f64)
    }

    /// Sample standard deviation (0 for fewer than 2 values).
    pub fn std_dev(&self) -> Option<f64> {
        match self.n {
            0 => None,
            1 => Some(0.0),
            n => {
                let n = n as f64;
                let var = (self.sum2 - self.sum * self.sum / n) / (n - 1.0);
                Some(var.max(0.0).sqrt())
            }
        }
    }

    /// Mean of the value as a step function of time between the first and the last row.
    pub fn time_weighted_mean(&self) -> Option<f64> {
        let ((t0, _), (t1, _)) = (self.first?, self.last?);
        (t1 > t0).then(|| self.area / (t1 - t0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t_quantiles() {
        assert!((t_quantile(0.975, 10.0) - 2.228_138_851_986).abs() < 1e-6);
        assert!((t_quantile(0.975, 1.0) - 12.706_204_736_174_7).abs() < 1e-5);
        assert!((t_quantile(0.995, 30.0) - 2.749_995_653_567).abs() < 1e-6);
        assert!((t_quantile(0.025, 10.0) + 2.228_138_851_986).abs() < 1e-6);
        assert_eq!(t_quantile(1.0, 3.0), f64::INFINITY);
    }

    /// Far tails with few degrees of freedom, against the closed forms for one and two degrees
    /// of freedom (the bracket used to end at 10 000).
    #[test]
    fn t_quantiles_in_the_far_tail() {
        use std::f64::consts::PI;
        for p in [0.999_995, 0.999_999_5, 0.999_999_999] {
            let cauchy = (PI * (p - 0.5)).tan();
            let rel = (t_quantile(p, 1.0) - cauchy).abs() / cauchy;
            assert!(
                rel < 1e-6,
                "df 1, p {p}: {} vs {cauchy}",
                t_quantile(p, 1.0)
            );
            let a = 4.0 * p * (1.0 - p);
            let two = (2.0 * p - 1.0) * (2.0 / a).sqrt();
            let rel = (t_quantile(p, 2.0) - two).abs() / two;
            assert!(rel < 1e-6, "df 2, p {p}: {} vs {two}", t_quantile(p, 2.0));
        }
    }

    #[test]
    fn tracker_matches_direct_statistics() {
        let rows: Vec<(f64, f64)> = (0..1000).map(|i| (i as f64, (i % 7) as f64)).collect();
        let mut t = SeriesTracker::default();
        t.update(&rows[..10], Warmup::default(), &mut |_| {});
        t.update(&rows, Warmup::default(), &mut |_| {});
        let mean = rows.iter().map(|r| r.1).sum::<f64>() / 1000.0;
        assert_eq!(t.n, 1000);
        assert!((t.mean().unwrap() - mean).abs() < 1e-12);
        assert_eq!(t.min, 0.0);
        assert_eq!(t.max, 6.0);
    }

    #[test]
    fn warmup_skips_rows() {
        let rows: Vec<(f64, f64)> = (0..100).map(|i| (i as f64, i as f64)).collect();
        let mut t = SeriesTracker::default();
        t.update(
            &rows,
            Warmup {
                time: 10.0,
                samples: 20,
            },
            &mut |_| {},
        );
        assert_eq!(t.n, 80);
        assert_eq!(t.first, Some((20.0, 20.0)));
    }

    #[test]
    fn doubling_batches_stay_between_min_and_twice_min() {
        let mut b = BatchMeans::new(Batching::Doubling { min: 30 });
        for i in 0..100_000 {
            b.push((i % 13) as f64);
            if i >= 30 {
                assert!(
                    b.means.len() >= 30 && b.means.len() < 60,
                    "{}",
                    b.means.len()
                );
            }
        }
        let ci = b.interval(0.95).unwrap();
        assert!((ci.mean - 6.0).abs() < 0.1);
        assert!(ci.half_width < 0.2);
    }

    #[test]
    fn static_batches_need_min_batches() {
        let mut b = BatchMeans::new(Batching::Static {
            size: 10,
            min_batches: 5,
        });
        for i in 0..49 {
            b.push(i as f64);
        }
        assert!(b.interval(0.95).is_none());
        b.push(49.0);
        let ci = b.interval(0.95).unwrap();
        assert_eq!(ci.batches, 5);
        assert!((ci.mean - 24.5).abs() < 1e-12);
    }
}

//! Measurements comparator: exact, tolerance and statistical modes over `measurements.csv` series.

use std::fmt;

use crate::diff::Tolerance;
use crate::measurements::{Measurements, Series};
use crate::stats;

/// Statistical comparison settings.
#[derive(Clone, Debug)]
pub struct StatOptions {
    /// KS significance level: a series fails only if KS rejects at `alpha` AND the batch-means
    /// confidence intervals of the means do not overlap.
    pub alpha: f64,
    /// Number of batches for the batch-means CI.
    pub batches: usize,
    /// CI confidence level.
    pub conf: f64,
    /// Series with fewer samples (either side) are only reported, not judged.
    pub min_samples: usize,
    /// Relative tolerance for time-weighted means of state/utilisation series.
    pub state_rel: f64,
    /// Absolute tolerance for time-weighted means of state/utilisation series.
    pub state_abs: f64,
    /// Percentiles reported (not judged).
    pub percentiles: Vec<f64>,
}

impl Default for StatOptions {
    fn default() -> Self {
        StatOptions {
            alpha: 0.001,
            batches: 20,
            conf: 0.99,
            min_samples: 40,
            state_rel: 0.1,
            state_abs: 0.05,
            percentiles: vec![0.5, 0.9, 0.99],
        }
    }
}

#[derive(Clone, Debug)]
pub enum MeasMode {
    Exact,
    Tolerance(Tolerance),
    Statistical(StatOptions),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail,
    /// Too few samples to judge.
    Skipped,
}

/// Statistics of one series pair (statistical mode).
#[derive(Clone, Debug, Default)]
pub struct SeriesStats {
    pub mean_expected: f64,
    pub mean_actual: f64,
    /// Time-weighted means for state-like metrics.
    pub twm: Option<(f64, f64)>,
    pub ks_d: f64,
    pub ks_p: f64,
    pub ci_expected: Option<(f64, f64)>,
    pub ci_actual: Option<(f64, f64)>,
    pub ci_overlap: Option<bool>,
    /// `(p, expected quantile, actual quantile)`.
    pub percentiles: Vec<(f64, f64, f64)>,
}

#[derive(Clone, Debug)]
pub struct SeriesCmp {
    pub mp: String,
    pub metric: String,
    pub n_expected: usize,
    pub n_actual: usize,
    pub verdict: Verdict,
    pub detail: String,
    pub stats: Option<SeriesStats>,
}

#[derive(Clone, Debug, Default)]
pub struct MeasReport {
    pub series: Vec<SeriesCmp>,
    pub missing_in_actual: Vec<String>,
    pub extra_in_actual: Vec<String>,
}

impl MeasReport {
    pub fn is_ok(&self) -> bool {
        self.missing_in_actual.is_empty()
            && self.extra_in_actual.is_empty()
            && self.series.iter().all(|s| s.verdict != Verdict::Fail)
    }
    pub fn failures(&self) -> impl Iterator<Item = &SeriesCmp> {
        self.series.iter().filter(|s| s.verdict == Verdict::Fail)
    }
}

/// State-like metrics are step functions: compared by time-weighted mean.
pub fn is_state_metric(metric: &str) -> bool {
    metric.starts_with("State of") || metric.starts_with("Utilization")
}

fn cmp_rows(e: &Series, a: &Series, tol: Option<Tolerance>) -> (Verdict, String) {
    let eq = |x: f64, y: f64| match tol {
        None => x.to_bits() == y.to_bits() || (x.is_nan() && y.is_nan()),
        Some(t) => t.eq(x, y),
    };
    for i in 0..e.len().min(a.len()) {
        if !eq(e.times[i], a.times[i]) || !eq(e.values[i], a.values[i]) {
            return (
                Verdict::Fail,
                format!(
                    "row {i}: expected ({}, {}) actual ({}, {})",
                    crate::javafmt::to_string(e.times[i]),
                    crate::javafmt::to_string(e.values[i]),
                    crate::javafmt::to_string(a.times[i]),
                    crate::javafmt::to_string(a.values[i])
                ),
            );
        }
    }
    if e.len() != a.len() {
        return (
            Verdict::Fail,
            format!("{} rows, expected {}", a.len(), e.len()),
        );
    }
    (Verdict::Pass, String::new())
}

fn cmp_stat(e: &Series, a: &Series, o: &StatOptions) -> (Verdict, String, SeriesStats) {
    let mut st = SeriesStats {
        mean_expected: stats::mean(&e.values),
        mean_actual: stats::mean(&a.values),
        ..Default::default()
    };
    if is_state_metric(&e.metric) {
        let end = e
            .times
            .last()
            .copied()
            .unwrap_or(0.0)
            .max(a.times.last().copied().unwrap_or(0.0));
        let (x, y) = (
            stats::time_weighted_mean(&e.times, &e.values, Some(end)),
            stats::time_weighted_mean(&a.times, &a.values, Some(end)),
        );
        st.twm = Some((x, y));
        let ok = crate::diff::float_eq(x, y, o.state_rel, o.state_abs);
        let v = if e.len().min(a.len()) < 2 {
            Verdict::Skipped
        } else if ok {
            Verdict::Pass
        } else {
            Verdict::Fail
        };
        return (v, format!("time-weighted mean {x:.6} vs {y:.6}"), st);
    }
    let (d, p) = stats::ks_two_sample(&e.values, &a.values);
    st.ks_d = d;
    st.ks_p = p;
    st.ci_expected = stats::batch_means_ci(&e.values, o.batches, o.conf);
    st.ci_actual = stats::batch_means_ci(&a.values, o.batches, o.conf);
    st.ci_overlap = match (st.ci_expected, st.ci_actual) {
        (Some((m1, h1)), Some((m2, h2))) => Some((m1 - m2).abs() <= h1 + h2),
        _ => None,
    };
    let mut se = e.values.clone();
    let mut sa = a.values.clone();
    se.sort_by(f64::total_cmp);
    sa.sort_by(f64::total_cmp);
    st.percentiles = o
        .percentiles
        .iter()
        .map(|&p| {
            (
                p,
                stats::quantile_sorted(&se, p),
                stats::quantile_sorted(&sa, p),
            )
        })
        .collect();
    let detail = format!(
        "mean {:.6} vs {:.6}, KS D={:.4} p={:.3e}, CI overlap {:?}",
        st.mean_expected, st.mean_actual, d, p, st.ci_overlap
    );
    let v = if e.len() < o.min_samples || a.len() < o.min_samples {
        Verdict::Skipped
    } else if p < o.alpha && st.ci_overlap != Some(true) {
        Verdict::Fail
    } else {
        Verdict::Pass
    };
    (v, detail, st)
}

/// Compares all series; series present on one side only are listed separately.
pub fn compare_measurements(
    expected: &Measurements,
    actual: &Measurements,
    mode: &MeasMode,
) -> MeasReport {
    let mut r = MeasReport::default();
    for e in &expected.series {
        let Some(a) = actual.get(&e.mp, &e.metric) else {
            r.missing_in_actual.push(e.key());
            continue;
        };
        let (verdict, detail, stats) = match mode {
            MeasMode::Exact => {
                let (v, d) = cmp_rows(e, a, None);
                (v, d, None)
            }
            MeasMode::Tolerance(t) => {
                let (v, d) = cmp_rows(e, a, Some(*t));
                (v, d, None)
            }
            MeasMode::Statistical(o) => {
                let (v, d, s) = cmp_stat(e, a, o);
                (v, d, Some(s))
            }
        };
        r.series.push(SeriesCmp {
            mp: e.mp.clone(),
            metric: e.metric.clone(),
            n_expected: e.len(),
            n_actual: a.len(),
            verdict,
            detail,
            stats,
        });
    }
    for a in &actual.series {
        if expected.get(&a.mp, &a.metric).is_none() {
            r.extra_in_actual.push(a.key());
        }
    }
    r
}

impl fmt::Display for MeasReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for s in &self.series {
            if s.verdict == Verdict::Pass && s.detail.is_empty() {
                continue;
            }
            writeln!(
                f,
                "{:?} {} / {} ({} vs {} rows) {}",
                s.verdict, s.mp, s.metric, s.n_expected, s.n_actual, s.detail
            )?;
        }
        for m in &self.missing_in_actual {
            writeln!(f, "missing in actual: {m}")?;
        }
        for m in &self.extra_in_actual {
            writeln!(f, "extra in actual: {m}")?;
        }
        if self.is_ok() {
            writeln!(f, "measurements OK ({} series)", self.series.len())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn series(mp: &str, metric: &str, v: Vec<f64>) -> Series {
        Series {
            mp: mp.into(),
            metric: metric.into(),
            times: (0..v.len()).map(|i| i as f64).collect(),
            values: v,
        }
    }

    #[test]
    fn exact_tolerance() {
        let e = Measurements {
            series: vec![series("A", "Response Time Tuple", vec![1.0, 2.0])],
        };
        let mut a = e.clone();
        assert!(compare_measurements(&e, &a, &MeasMode::Exact).is_ok());
        a.series[0].values[1] = 2.0 + 1e-12;
        let r = compare_measurements(&e, &a, &MeasMode::Exact);
        assert!(!r.is_ok());
        assert!(r.series[0].detail.starts_with("row 1"));
        assert!(
            compare_measurements(
                &e,
                &a,
                &MeasMode::Tolerance(Tolerance {
                    rel: 1e-9,
                    abs: 0.0
                })
            )
            .is_ok()
        );
        a.series.push(series("B", "x", vec![1.0]));
        assert_eq!(
            compare_measurements(&e, &a, &MeasMode::Exact)
                .extra_in_actual
                .len(),
            1
        );
    }

    #[test]
    fn statistical() {
        // two deterministic "exponential-like" samples with the same distribution
        let q = |n: usize, off: f64| -> Vec<f64> {
            (0..n)
                .map(|i| -(1.0 - ((i as f64 * 0.618_033_988_75 + off) % 1.0)).ln())
                .collect()
        };
        let e = Measurements {
            series: vec![series("A", "Response Time Tuple", q(2000, 0.1))],
        };
        let a = Measurements {
            series: vec![series("A", "Response Time Tuple", q(3000, 0.37))],
        };
        let r = compare_measurements(&e, &a, &MeasMode::Statistical(StatOptions::default()));
        assert!(r.is_ok(), "{r}");
        let b = Measurements {
            series: vec![series(
                "A",
                "Response Time Tuple",
                q(3000, 0.37).iter().map(|x| x * 1.5).collect(),
            )],
        };
        let r = compare_measurements(&e, &b, &MeasMode::Statistical(StatOptions::default()));
        assert!(!r.is_ok(), "{r}");
    }
}

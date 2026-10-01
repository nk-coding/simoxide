//! Probability function literals as the simulator prepares and samples them
//! (`ProbFunctionCache`, `ProbabilityMassFunctionImpl`, `BoxedPDFImpl`, `MathTools`,
//! probfunction.math 5.2.2).
//!
//! Preparation (once per expression, at "parse" time):
//! 1. *Adjustment*: `sum` of the probabilities in literal order; if `|sum - 1| > 1e-9`
//!    (`10e-10` in the source), `delta = (1 - sum) / count(p > 0)` is added to every positive
//!    probability.
//! 2. The samples are stably sorted by value (`Integer`/`Double`/`String`/`Boolean`
//!    `compareTo`).
//! 3. Validation: the sorted sum must be within `1e-5` of 1 and every probability in `[0, 1]`
//!    (PDF: no duplicate values, no value `< 0`, and the first value must not be 0 because an
//!    implicit sample at 0 is assumed).
//! 4. Cumulative sums in sorted order.
//!
//! Sampling draws exactly one uniform `u`. PMF: the value of the first sample with
//! `u < cum[i]`; if there is none (sum slightly below 1) the result is the Double `0.0`, even
//! for an `IntPMF`. Boxed PDF: linear interpolation on the segment of the first `i` with
//! `u < cum[i]`, i.e. `(u - b) / a` with `a = (cum[i] - cum[i-1]) / (v[i] - v[i-1])`,
//! `b = cum[i-1] - a * v[i-1]` (and `(0, 0)` as the point before the first sample); no segment
//! is an error.

use crate::ast::ProbFnLit;
use crate::error::{EvalError, EvalErrorKind};
use crate::jmath::{double_compare, string_compare};
use crate::value::Value;
use simoxide_random::UniformSource;
use std::cmp::Ordering;
use std::sync::Arc;

/// Sorted sample values of a PMF.
#[derive(Debug, Clone, PartialEq)]
pub enum PmfValues {
    Int(Vec<i32>),
    Double(Vec<f64>),
    Str(Vec<Arc<str>>),
    Bool(Vec<bool>),
}

/// A prepared PMF.
#[derive(Debug, Clone, PartialEq)]
pub struct Pmf {
    pub values: PmfValues,
    /// Adjusted probabilities in sorted order.
    pub probs: Vec<f64>,
    /// Running sums of `probs`.
    pub cum: Vec<f64>,
    /// `cum` is non-decreasing and NaN-free (always true after validation): samples may be
    /// found by binary search.
    pub monotone: bool,
}

/// A prepared boxed PDF.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxedPdf {
    /// Sorted sample values.
    pub values: Vec<f64>,
    /// Adjusted probabilities in sorted order.
    pub probs: Vec<f64>,
    pub cum: Vec<f64>,
    /// `(a, b)` of the segment ending at each sample.
    pub lines: Vec<(f64, f64)>,
    /// `cum` is non-decreasing and NaN-free (see [`Pmf::monotone`]).
    pub monotone: bool,
}

/// A prepared probability function literal.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbFn {
    Pmf(Pmf),
    Pdf(BoxedPdf),
}

/// `ProbFunctionCache.adjustPMF` / `adjustPDF`, in literal order.
pub fn adjust(probs: &mut [f64]) {
    let mut sum = 0.0;
    for p in probs.iter() {
        sum += *p;
    }
    if (sum - 1.0).abs() > 10e-10 {
        let count = probs.iter().filter(|p| **p > 0.0).count() as f64;
        let delta = (1.0 - sum) / count;
        for p in probs.iter_mut() {
            if *p > 0.0 {
                *p += delta;
            }
        }
    }
}

/// `MathTools.equalsDouble(d1, d2)` (the NaN check in the source never fires).
fn equals_double(d1: f64, d2: f64) -> bool {
    (d1 - d2).abs() < 1e-5
}

fn cumulative(probs: &[f64]) -> Vec<f64> {
    let mut acc = 0.0;
    probs
        .iter()
        .map(|p| {
            acc += *p;
            acc
        })
        .collect()
}

/// Non-decreasing and without NaN. Running sums of the validated probabilities (all in
/// `[0, 1]`) always are: `fl(s + p) >= s` for `p >= 0`.
fn is_monotone(cum: &[f64]) -> bool {
    cum.iter().all(|c| !c.is_nan()) && cum.windows(2).all(|w| w[0] <= w[1])
}

/// Index of the first `c` in `cum` with `u < c` (`Iterator::position`). The first entries are
/// scanned linearly (typical PMFs put most mass there, and the scan branches predictably); the
/// rest of a monotone `cum` is binary searched: there the predicate is false on a prefix and true
/// on the rest, so the search finds the same index (`u = NaN` gives `None` either way).
#[inline]
#[allow(clippy::neg_cmp_op_on_partial_ord)] // `!(u < c)` is true for NaN `u`, like the scan
fn first_above(cum: &[f64], monotone: bool, u: f64) -> Option<usize> {
    const SCAN: usize = 16;
    if !monotone || cum.len() <= SCAN {
        return cum.iter().position(|c| u < *c);
    }
    if let Some(i) = cum[..SCAN].iter().position(|c| u < *c) {
        return Some(i);
    }
    let i = SCAN + cum[SCAN..].partition_point(|c| !(u < *c));
    (i < cum.len()).then_some(i)
}

fn sum(probs: &[f64]) -> f64 {
    let mut s = 0.0;
    for p in probs {
        s += *p;
    }
    s
}

/// Stable sort permutation.
fn sort_perm<T>(v: &[T], cmp: impl Fn(&T, &T) -> Ordering) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&a, &b| cmp(&v[a], &v[b])); // stable (merge sort), like Collections.sort
    idx
}

fn prepare_pmf<T: Clone>(
    samples: &[(T, f64)],
    cmp: impl Fn(&T, &T) -> Ordering,
    wrap: impl FnOnce(Vec<T>) -> PmfValues,
) -> Result<Pmf, String> {
    let mut probs: Vec<f64> = samples.iter().map(|s| s.1).collect();
    adjust(&mut probs);
    let vals: Vec<T> = samples.iter().map(|s| s.0.clone()).collect();
    let perm = if vals.len() > 1 {
        sort_perm(&vals, cmp)
    } else {
        (0..vals.len()).collect()
    };
    let sorted_vals: Vec<T> = perm.iter().map(|&i| vals[i].clone()).collect();
    let sorted_probs: Vec<f64> = perm.iter().map(|&i| probs[i]).collect();
    // checkConstrains
    if !equals_double(sum(&sorted_probs), 1.0) {
        return Err("PMF not valid: probabilities do not sum up to 1".into());
    }
    if sorted_probs.iter().any(|p| *p < 0.0 || *p > 1.0) {
        return Err("PMF not valid: invalid sample probability".into());
    }
    let cum = cumulative(&sorted_probs);
    let monotone = is_monotone(&cum);
    Ok(Pmf {
        values: wrap(sorted_vals),
        probs: sorted_probs,
        cum,
        monotone,
    })
}

fn prepare_pdf(samples: &[(f64, f64)]) -> Result<BoxedPdf, String> {
    let mut probs: Vec<f64> = samples.iter().map(|s| s.1).collect();
    adjust(&mut probs);
    // setSamples: duplicate values (Double.equals, i.e. bitwise) are rejected
    for i in 0..samples.len() {
        for j in 0..i {
            if samples[i].0.to_bits() == samples[j].0.to_bits() {
                return Err(
                    "PDF not valid: found duplicate sample values (not probabilities)".into(),
                );
            }
        }
    }
    let vals: Vec<f64> = samples.iter().map(|s| s.0).collect();
    let perm = sort_perm(&vals, |a, b| double_compare(*a, *b));
    let values: Vec<f64> = perm.iter().map(|&i| vals[i]).collect();
    let probs: Vec<f64> = perm.iter().map(|&i| probs[i]).collect();
    let cum = cumulative(&probs);
    // MathTools.computeLines
    let line = |x1: f64, y1: f64, x2: f64, y2: f64| -> Result<(f64, f64), String> {
        if x2 - x1 == 0.0 {
            return Err(format!(
                "PDF not valid: Two samples of the PDF have the same value. Note that an initial sample with value 0 is assumed, so you must not specify another one with value 0. Values: {x1:?} and {x2:?}"
            ));
        }
        let a = (y2 - y1) / (x2 - x1);
        let b = y1 - (a * x1);
        Ok((a, b))
    };
    let mut lines = Vec::with_capacity(values.len());
    lines.push(line(0.0, 0.0, values[0], probs[0])?);
    for i in 1..values.len() {
        let (y1, y2) = (cum[i - 1], cum[i]);
        if y1 != y2 {
            lines.push(line(values[i - 1], y1, values[i], y2)?);
        } else {
            // Never used: a uniform below cum[i] is already below cum[i-1].
            lines.push((f64::NAN, f64::NAN));
        }
    }
    // checkConstrains
    if !equals_double(sum(&probs), 1.0) {
        return Err("PDF not valid: probabilities do not sum up to 1".into());
    }
    let mut prev = 0.0;
    for (v, p) in values.iter().zip(&probs) {
        if *v < 0.0 || *p < 0.0 || *p > 1.0 || *v < prev {
            return Err("PDF not valid: invalid sample".into());
        }
        prev = *v;
    }
    let monotone = is_monotone(&cum);
    Ok(BoxedPdf {
        values,
        probs,
        cum,
        lines,
        monotone,
    })
}

impl ProbFn {
    /// Prepares a literal like `ProbFunctionCache` does. The error message is the reason the
    /// reference rejects it.
    pub fn prepare(lit: &ProbFnLit) -> Result<ProbFn, String> {
        Ok(match lit {
            ProbFnLit::IntPmf(s) => ProbFn::Pmf(prepare_pmf(s, |a, b| a.cmp(b), PmfValues::Int)?),
            ProbFnLit::DoublePmf(s) => ProbFn::Pmf(prepare_pmf(
                s,
                |a, b| double_compare(*a, *b),
                PmfValues::Double,
            )?),
            ProbFnLit::EnumPmf { samples, .. } => {
                let s: Vec<(Arc<str>, f64)> = samples
                    .iter()
                    .map(|(v, p)| (Arc::from(v.as_str()), *p))
                    .collect();
                ProbFn::Pmf(prepare_pmf(
                    &s,
                    |a, b| string_compare(a, b),
                    PmfValues::Str,
                )?)
            }
            ProbFnLit::BoolPmf { samples, .. } => {
                ProbFn::Pmf(prepare_pmf(samples, |a, b| a.cmp(b), PmfValues::Bool)?)
            }
            ProbFnLit::BoxedPdf(s) => ProbFn::Pdf(prepare_pdf(s)?),
        })
    }

    /// Draws one sample (one uniform).
    #[inline]
    pub fn sample<R: UniformSource + ?Sized>(&self, rng: &mut R) -> Result<Value, EvalError> {
        let u = rng.next_uniform();
        match self {
            ProbFn::Pmf(p) => Ok(p.value_for(u)),
            ProbFn::Pdf(p) => p.value_for(u).map(Value::Double),
        }
    }
}

impl Pmf {
    /// The sample for uniform `u` (`drawSample`).
    #[inline(always)]
    pub fn value_for(&self, u: f64) -> Value {
        match first_above(&self.cum, self.monotone, u) {
            Some(j) => match &self.values {
                PmfValues::Int(v) => Value::Int(v[j]),
                PmfValues::Double(v) => Value::Double(v[j]),
                PmfValues::Str(v) => Value::Str(v[j].clone()),
                PmfValues::Bool(v) => Value::Bool(v[j]),
            },
            None => Value::Double(0.0),
        }
    }
}

impl BoxedPdf {
    /// The sample for uniform `u` (`drawSample`).
    #[inline(always)]
    pub fn value_for(&self, u: f64) -> Result<f64, EvalError> {
        match first_above(&self.cum, self.monotone, u) {
            Some(i) => {
                let (a, b) = self.lines[i];
                Ok((u - b) / a)
            }
            None => Err(EvalError::new(
                EvalErrorKind::Runtime,
                "No interval found for probability. This should never happen!",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_search_equals_linear_scan() {
        let mut x: u64 = 0x1234_5678_9abc_def1;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 11) as f64 / (1u64 << 53) as f64
        };
        for n in [1usize, 5, 9, 17, 100, 400] {
            let mut probs: Vec<f64> = (0..n)
                .map(|i| if i % 3 == 1 { 0.0 } else { next() })
                .collect();
            let total: f64 = probs.iter().sum();
            probs.iter_mut().for_each(|p| *p /= total);
            let cum = cumulative(&probs);
            assert!(is_monotone(&cum));
            let mut us: Vec<f64> = (0..2000).map(|_| next()).collect();
            us.extend(cum.iter().copied());
            us.extend([0.0, 1.0, f64::NAN, -0.0, 2.0, cum[n - 1]]);
            for u in us {
                assert_eq!(first_above(&cum, true, u), cum.iter().position(|c| u < *c));
            }
        }
        assert!(!is_monotone(&[0.1, 0.05]));
        assert!(!is_monotone(&[0.1, f64::NAN]));
    }
}

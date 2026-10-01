//! Sampling of probability-function literals (PMFs, boxed PDFs) and of probabilistic branches,
//! as done by SimuCom/SimuLizar 5.2.2.
//!
//! Life cycle of a PMF/PDF literal in the reference (per distinct StoEx string, once):
//! 1. `ProbFunctionCache.adjustPMF/adjustPDF` ([`adjust_probabilities`]) on the samples in model
//!    order; this mutates the model.
//! 2. `transformToPMF` / `transformToBoxedPDF`: the samples are stably sorted by value
//!    (`Comparable.compareTo`; see [`java_double_compare`], [`java_string_compare`]).
//! 3. `checkConstrains` ([`validate_pmf`], [`BoxedPdfSampler::new`]).
//! 4. Per evaluation: one uniform `u`; first index `j` with `u < cum[j]` where `cum` are the
//!    running sums of the sorted probabilities (starting from `0.0`, summed left to right).

use std::cmp::Ordering;

use crate::source::UniformSource;

/// Error of a probability function literal.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbFnError {
    /// `ProbabilitySumNotOneException`: `|sum - 1| >= 1e-5` (`MathTools.equalsDouble`).
    SumNotOne(f64),
    /// `InvalidSampleValueException`: a probability outside `[0, 1]`, or a negative PDF value.
    InvalidSample(usize),
    /// `DoubleSampleException`: two PDF samples with the same value (`Double.equals`).
    DuplicateSample(f64),
    /// `Line` constructor: two consecutive interpolation points with equal x (for example a
    /// first PDF sample at `0`).
    SameValue(f64),
    /// Empty sample list (`computeCumulativeProbabilities` throws).
    Empty,
    /// `BoxedPDFImpl.drawSample`: `u >= cum[last]` ("No interval found for probability").
    NoInterval(f64),
}

impl std::fmt::Display for ProbFnError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbFnError::SumNotOne(s) => write!(f, "probability sum {s} is not 1"),
            ProbFnError::InvalidSample(i) => write!(f, "invalid sample at index {i}"),
            ProbFnError::DuplicateSample(v) => {
                write!(f, "found duplicate sample values (not probabilities): {v}")
            }
            ProbFnError::SameValue(v) => write!(
                f,
                "Two samples of the PDF have the same value. Note that an initial sample with value 0 is assumed, so you must not specify another one with value 0. Value: {v}"
            ),
            ProbFnError::Empty => write!(f, "ProbabilityList is empty or null!"),
            ProbFnError::NoInterval(u) => write!(
                f,
                "No interval found for probability {u}. This should never happen!"
            ),
        }
    }
}

impl std::error::Error for ProbFnError {}

/// `ProbFunctionCache.adjustPMF` / `adjustPDF`: if the probabilities (summed in model order) are
/// off from 1 by more than `1e-9`, spread the difference evenly over all samples with positive
/// probability. Returns `true` if it adjusted. Must be applied in model order, before sorting.
pub fn adjust_probabilities(probs: &mut [f64]) -> bool {
    let mut sum = 0.0;
    for &p in probs.iter() {
        sum += p;
    }
    if (sum - 1.0).abs() > 1.0E-9 {
        let count = probs.iter().filter(|&&p| p > 0.0).count() as i32 as f64;
        let delta = (1.0 - sum) / count;
        for p in probs.iter_mut() {
            if *p > 0.0 {
                *p += delta;
            }
        }
        true
    } else {
        false
    }
}

/// `MathTools.equalsDouble(a, b)`: `|a - b| < 1e-5` (the NaN branch never fires in Java).
#[inline]
pub fn java_equals_double(a: f64, b: f64) -> bool {
    (a - b).abs() < 1.0E-5
}

/// Running sums as `MathTools.computeCumulativeProbabilities`: `prob = 0; prob += p_i`.
pub fn cumulative(probs: &[f64]) -> Vec<f64> {
    let mut acc = 0.0;
    probs
        .iter()
        .map(|&p| {
            acc += p;
            acc
        })
        .collect()
}

/// `ProbabilityMassFunctionImpl.checkConstrains` on the **sorted** probabilities.
pub fn validate_pmf(sorted_probs: &[f64]) -> Result<(), ProbFnError> {
    let mut sum = 0.0;
    for &p in sorted_probs {
        sum += p;
    }
    if !java_equals_double(sum, 1.0) {
        return Err(ProbFnError::SumNotOne(sum));
    }
    for (i, &p) in sorted_probs.iter().enumerate() {
        if p < 0.0 || p > 1.0 {
            return Err(ProbFnError::InvalidSample(i));
        }
    }
    Ok(())
}

/// `Double.compare(a, b)` (the order of `Double.compareTo`, used to sort PMF/PDF samples):
/// numeric order, `-0.0 < 0.0`, all NaNs equal and greater than everything.
#[inline]
pub fn java_double_compare(a: f64, b: f64) -> Ordering {
    if a < b {
        Ordering::Less
    } else if a > b {
        Ordering::Greater
    } else {
        java_double_to_long_bits(a).cmp(&java_double_to_long_bits(b))
    }
}

/// `Double.doubleToLongBits` (canonical NaN).
#[inline]
pub fn java_double_to_long_bits(a: f64) -> i64 {
    if a.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        a.to_bits() as i64
    }
}

/// `String.compareTo`: lexicographic over UTF-16 code units.
pub fn java_string_compare(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// Stable sort permutation (`Collections.sort` is stable): `order[k]` is the original index of
/// the `k`-th smallest element.
pub fn sort_order<T>(values: &[T], mut cmp: impl FnMut(&T, &T) -> Ordering) -> Vec<usize> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&i, &j| cmp(&values[i], &values[j]));
    order
}

/// Finds the first `j` with `x < cum[j]` (linear-scan semantics). Uses binary search when `cum`
/// is non-decreasing (then both give the same index).
#[inline]
fn first_above(cum: &[f64], monotone: bool, x: f64) -> Option<usize> {
    if monotone && cum.len() > 8 {
        // Non-decreasing and NaN-free: `x < cum[j]` is false on a prefix and true on the rest.
        #[allow(clippy::neg_cmp_op_on_partial_ord)] // same predicate as the scan, incl. NaN `x`
        let j = cum.partition_point(|&c| !(x < c));
        (j < cum.len()).then_some(j)
    } else {
        cum.iter().position(|&c| x < c)
    }
}

fn is_monotone(cum: &[f64]) -> bool {
    cum.iter().all(|c| !c.is_nan()) && cum.windows(2).all(|w| w[0] <= w[1])
}

/// Sampler for a PMF literal (`ProbabilityMassFunctionImpl.drawSample`).
///
/// Built from the probabilities **after** adjustment and sorting by value. `sample_index`
/// returns the index into that sorted list, or `None` when `u >= cum[last]`; in that case the
/// reference returns `Double 0.0` instead of a sample value.
#[derive(Debug, Clone, PartialEq)]
pub struct PmfSampler {
    cum: Box<[f64]>,
    monotone: bool,
}

impl PmfSampler {
    /// Precomputes the running sums (no validation; see [`validate_pmf`]).
    pub fn new(sorted_probs: &[f64]) -> Self {
        let cum = cumulative(sorted_probs).into_boxed_slice();
        let monotone = is_monotone(&cum);
        PmfSampler { cum, monotone }
    }
    /// The running sums.
    pub fn cumulative(&self) -> &[f64] {
        &self.cum
    }
    /// Index of the sample chosen by uniform `u`.
    #[inline]
    pub fn sample_index(&self, u: f64) -> Option<usize> {
        first_above(&self.cum, self.monotone, u)
    }
    /// Draws one uniform and returns the chosen index.
    #[inline]
    pub fn sample(&self, rng: &mut impl UniformSource) -> Option<usize> {
        self.sample_index(rng.next_uniform())
    }
}

/// Sampler for a boxed PDF literal (`BoxedPDFImpl.drawSample`): linear interpolation of the CDF
/// through `(0, 0)` and `(value_i, cum_i)`.
#[derive(Debug, Clone, PartialEq)]
pub struct BoxedPdfSampler {
    /// Running sums of the sorted probabilities.
    cum: Box<[f64]>,
    /// Per index: the line `(a, b)` (`y = a*x + b`) stored under key `cum[j]` in Java's
    /// `HashMap<Double, Line>`.
    lines: Box<[(f64, f64)]>,
    monotone: bool,
}

fn java_line(x1: f64, y1: f64, x2: f64, y2: f64) -> Result<(f64, f64), ProbFnError> {
    if x2 - x1 == 0.0 {
        return Err(ProbFnError::SameValue(x2));
    }
    let a = (y2 - y1) / (x2 - x1);
    let b = y1 - a * x1;
    Ok((a, b))
}

impl BoxedPdfSampler {
    /// `transformToBoxedPDF` + `checkConstrains` from `(value, probability)` pairs in model order
    /// (after [`adjust_probabilities`]).
    pub fn new(samples: &[(f64, f64)]) -> Result<Self, ProbFnError> {
        if samples.is_empty() {
            return Err(ProbFnError::Empty);
        }
        // containsDuplicateSamples: HashSet<Double> (Double.equals = doubleToLongBits equality).
        let mut keys: Vec<i64> = samples
            .iter()
            .map(|s| java_double_to_long_bits(s.0))
            .collect();
        keys.sort_unstable();
        if let Some(w) = keys.windows(2).find(|w| w[0] == w[1]) {
            return Err(ProbFnError::DuplicateSample(f64::from_bits(w[0] as u64)));
        }
        let order = sort_order(samples, |a, b| java_double_compare(a.0, b.0));
        let values: Vec<f64> = order.iter().map(|&i| samples[i].0).collect();
        let probs: Vec<f64> = order.iter().map(|&i| samples[i].1).collect();
        let cum = cumulative(&probs);

        // MathTools.computeLines: put(cum[0], Line(0,0,v0,p0)); then for i >= 1 with
        // cum[i-1] != cum[i]: put(cum[i], Line(v[i-1], cum[i-1], v[i], cum[i])).
        let mut puts: Vec<(i64, (f64, f64))> = Vec::with_capacity(cum.len());
        puts.push((
            java_double_to_long_bits(cum[0]),
            java_line(0.0, 0.0, values[0], probs[0])?,
        ));
        for i in 1..cum.len() {
            let (x1, y1, x2, y2) = (values[i - 1], cum[i - 1], values[i], cum[i]);
            if y1 != y2 {
                puts.push((java_double_to_long_bits(y2), java_line(x1, y1, x2, y2)?));
            }
        }
        // Resolve HashMap semantics: the line stored under key k is the last put with key k.
        let lines: Vec<(f64, f64)> = cum
            .iter()
            .map(|&c| {
                let k = java_double_to_long_bits(c);
                puts.iter()
                    .rev()
                    .find(|p| p.0 == k)
                    .map(|p| p.1)
                    // Key never put (only possible for a later equal-run element): unreachable,
                    // because the scan stops at the first element of a run.
                    .unwrap_or((f64::NAN, f64::NAN))
            })
            .collect();

        // checkConstrains (on the sorted samples).
        let mut sum = 0.0;
        for &p in &probs {
            sum += p;
        }
        if !java_equals_double(sum, 1.0) {
            return Err(ProbFnError::SumNotOne(sum));
        }
        let mut prev = 0.0;
        for (i, (&v, &p)) in values.iter().zip(&probs).enumerate() {
            if v < 0.0 || p < 0.0 || p > 1.0 || v < prev {
                return Err(ProbFnError::InvalidSample(i));
            }
            prev = v;
        }

        let monotone = is_monotone(&cum);
        Ok(BoxedPdfSampler {
            cum: cum.into_boxed_slice(),
            lines: lines.into_boxed_slice(),
            monotone,
        })
    }

    /// The running sums of the sorted probabilities.
    pub fn cumulative(&self) -> &[f64] {
        &self.cum
    }

    /// Value for uniform `u`: `Line.getX(u) = (u - b) / a` of the first box with `u < cum[j]`.
    #[inline]
    pub fn inverse_cdf(&self, u: f64) -> Result<f64, ProbFnError> {
        match first_above(&self.cum, self.monotone, u) {
            Some(j) => {
                let (a, b) = self.lines[j];
                Ok((u - b) / a)
            }
            None => Err(ProbFnError::NoInterval(u)),
        }
    }

    /// Draws one uniform and interpolates.
    #[inline]
    pub fn sample(&self, rng: &mut impl UniformSource) -> Result<f64, ProbFnError> {
        self.inverse_cdf(rng.next_uniform())
    }
}

/// `TransitionDeterminer.createSummedProbabilityList`: running sums of branch probabilities.
pub fn summed_probabilities(probs: &[f64]) -> Vec<f64> {
    cumulative(probs)
}

/// `TransitionDeterminer.getRandomIndex`: first `i` with `last_sum * u < summed[i]`
/// (`None` = Java's `-1`, which then fails with an `IndexOutOfBoundsException`).
/// Consumes no uniform when `summed` is empty (Java returns before drawing).
#[inline]
pub fn branch_index(summed: &[f64], u: f64) -> Option<usize> {
    let last = *summed.last()?;
    let x = last * u;
    summed.iter().position(|&s| x < s)
}

/// Draws one uniform (unless `summed` is empty) and selects a branch like
/// `TransitionDeterminer.getRandomIndex`.
#[inline]
pub fn sample_branch(summed: &[f64], rng: &mut impl UniformSource) -> Option<usize> {
    if summed.is_empty() {
        return None;
    }
    branch_index(summed, rng.next_uniform())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pmf_scan_semantics() {
        let s = PmfSampler::new(&[0.25, 0.25, 0.5]);
        assert_eq!(s.sample_index(0.0), Some(0));
        assert_eq!(s.sample_index(0.2499), Some(0));
        assert_eq!(s.sample_index(0.25), Some(1)); // strict <
        assert_eq!(s.sample_index(0.5), Some(2));
        assert_eq!(s.sample_index(0.9999), Some(2));
        assert_eq!(s.sample_index(1.0), None);
    }

    #[test]
    fn pmf_binary_search_matches_scan() {
        let probs: Vec<f64> = (0..40)
            .map(|i| if i % 3 == 0 { 0.0 } else { 0.037 })
            .collect();
        let s = PmfSampler::new(&probs);
        assert!(s.monotone);
        let cum = s.cumulative().to_vec();
        for k in 0..=4000 {
            let u = k as f64 / 4000.0;
            assert_eq!(s.sample_index(u), cum.iter().position(|&c| u < c));
        }
        for &c in &cum {
            for u in [
                c,
                f64::from_bits(c.to_bits().saturating_sub(1)),
                f64::from_bits(c.to_bits() + 1),
            ] {
                assert_eq!(s.sample_index(u), cum.iter().position(|&c| u < c));
            }
        }
    }

    #[test]
    fn adjust_like_java() {
        let mut p = [0.5, 0.0, 0.3];
        assert!(adjust_probabilities(&mut p));
        assert_eq!(p, [0.5 + 0.1, 0.0, 0.3 + (1.0 - 0.8) / 2.0]);
        let mut q = [0.5, 0.5];
        assert!(!adjust_probabilities(&mut q));
    }

    #[test]
    fn boxed_pdf_interpolates() {
        let s = BoxedPdfSampler::new(&[(2.0, 0.5), (1.0, 0.5)]).unwrap();
        assert_eq!(s.inverse_cdf(0.0).unwrap(), 0.0);
        assert_eq!(s.inverse_cdf(0.25).unwrap(), 0.5);
        assert_eq!(s.inverse_cdf(0.75).unwrap(), 1.5);
        assert!(s.inverse_cdf(1.0).is_err());
        assert!(matches!(
            BoxedPdfSampler::new(&[(0.0, 1.0)]),
            Err(ProbFnError::SameValue(_))
        ));
        assert!(matches!(
            BoxedPdfSampler::new(&[(1.0, 0.5), (1.0, 0.5)]),
            Err(ProbFnError::DuplicateSample(_))
        ));
    }

    #[test]
    fn branch_semantics() {
        let s = summed_probabilities(&[0.2, 0.3, 0.5]);
        assert_eq!(branch_index(&s, 0.0), Some(0));
        assert_eq!(branch_index(&s, 0.2), Some(1));
        assert_eq!(branch_index(&s, 0.99), Some(2));
        assert_eq!(branch_index(&[], 0.5), None);
        // Not normalised: scaled by the last sum.
        let s = summed_probabilities(&[1.0, 3.0]);
        assert_eq!(branch_index(&s, 0.24), Some(0));
        assert_eq!(branch_index(&s, 0.25), Some(1));
    }

    #[test]
    fn java_orders() {
        assert_eq!(java_double_compare(-0.0, 0.0), Ordering::Less);
        assert_eq!(
            java_double_compare(f64::NAN, f64::INFINITY),
            Ordering::Greater
        );
        assert_eq!(java_double_compare(-f64::NAN, f64::NAN), Ordering::Equal);
        assert_eq!(java_string_compare("B", "a"), Ordering::Less);
        // U+FF5E (one UTF-16 unit) sorts after U+1F600 (surrogates 0xD83D..) in Java.
        assert_eq!(
            java_string_compare("\u{ff5e}", "\u{1f600}"),
            Ordering::Greater
        );
    }
}

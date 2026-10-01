//! Named distributions of the StoEx function library (`FunctionLib` in SimuCom 5.2.2).
//!
//! Each distribution type mirrors the Commons Math 2.1 object that SimuCom creates per call;
//! `inverse_cdf` is `inverseF(u)` of the Palladio wrapper. The `sample_*` free functions are the
//! StoEx functions: they check the parameters like `checkParameters`, construct, draw exactly one
//! uniform and invert.

// Negated comparisons (`!(x <= 0.0)`) are deliberate: they reproduce Java's NaN behaviour.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

use crate::jmath;
use crate::source::UniformSource;
use crate::special::{
    self, ContinuousDistribution, IntegerDistribution, MathError,
    inverse_cumulative_continuous_with, inverse_cumulative_int,
};
use std::cell::RefCell;

/// Why a sample could not be drawn. In the reference each of these aborts the simulation run.
#[derive(Debug, Clone, PartialEq)]
pub enum DistError {
    /// `checkParameters` rejected the arguments (`FunctionParametersNotAcceptedException`).
    /// No uniform was consumed.
    ParametersNotAccepted {
        /// StoEx function name, e.g. `"Exp"`.
        function: &'static str,
    },
    /// The distribution constructor rejected the parameters (`IllegalArgumentException` or
    /// `MathException` in Commons Math). No uniform was consumed.
    InvalidParameter {
        /// Distribution name.
        dist: &'static str,
        /// Message of the Java exception.
        message: &'static str,
        /// Offending value.
        value: f64,
    },
    /// The numerical inversion failed (`MathException` / `FunctionEvaluationException` /
    /// `ConvergenceException` / `IllegalArgumentException` inside `inverseCumulativeProbability`).
    /// The uniform was consumed.
    Numerical {
        /// Distribution name.
        dist: &'static str,
        /// What failed.
        message: &'static str,
    },
}

impl std::fmt::Display for DistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DistError::ParametersNotAccepted { function } => write!(
                f,
                "Parameters passed to function {function} do not match function definition!"
            ),
            DistError::InvalidParameter {
                dist,
                message,
                value,
            } => write!(f, "{dist}: {message} ({value})"),
            DistError::Numerical { dist, message } => write!(f, "{dist}: {message}"),
        }
    }
}

impl std::error::Error for DistError {}

type Result<T> = std::result::Result<T, DistError>;

pub use crate::special::BracketSearch;

fn numerical(dist: &'static str, e: MathError) -> DistError {
    DistError::Numerical {
        dist,
        message: e.message(),
    }
}

fn invalid(dist: &'static str, message: &'static str, value: f64) -> DistError {
    DistError::InvalidParameter {
        dist,
        message,
        value,
    }
}

// ---------------------------------------------------------------------------------------------

/// `ExponentialDistributionImpl(mean = 1/rate)`: `-mean * Math.log(1 - u)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Exponential {
    mean: f64,
}

impl Exponential {
    /// Palladio `ExponentialDistribution(rate)`: `mean = 1.0 / rate`, which must be `> 0`.
    pub fn new(rate: f64) -> Result<Self> {
        let mean = 1.0 / rate;
        if mean <= 0.0 {
            return Err(invalid("Exponential", "mean must be positive", mean));
        }
        Ok(Exponential { mean })
    }
    /// The mean `1.0 / rate`.
    pub fn mean(&self) -> f64 {
        self.mean
    }
    /// `inverseF(u)`.
    #[inline]
    pub fn inverse_cdf(&self, u: f64) -> Result<f64> {
        #[allow(clippy::manual_range_contains)]
        if u < 0.0 || u > 1.0 {
            Err(numerical(
                "Exponential",
                MathError::IllegalArgument("probability out of [0, 1] range"),
            ))
        } else if u == 1.0 {
            Ok(f64::INFINITY)
        } else {
            Ok(-self.mean * jmath::log(1.0 - u))
        }
    }
    /// One uniform, inverted.
    #[inline]
    pub fn sample<R: UniformSource + ?Sized>(&self, rng: &mut R) -> Result<f64> {
        self.inverse_cdf(rng.next_uniform())
    }
}

// ---------------------------------------------------------------------------------------------

/// `NormalDistributionImpl(mean, sd)` (inverse CDF solved with Brent, accuracy 1e-9).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Normal {
    mean: f64,
    sd: f64,
}

const SQRT2: f64 = std::f64::consts::SQRT_2; // Math.sqrt(2.0), exact

impl Normal {
    /// Palladio `NormalDistribution(mean, sigma)`; `sd` must be `> 0`.
    pub fn new(mean: f64, sd: f64) -> Result<Self> {
        if sd <= 0.0 {
            return Err(invalid("Normal", "standard deviation must be positive", sd));
        }
        Ok(Normal { mean, sd })
    }
    /// `(mean, sd)`.
    pub fn params(&self) -> (f64, f64) {
        (self.mean, self.sd)
    }
    /// `NormalDistributionImpl.cumulativeProbability(x)`.
    pub fn cdf(&self, x: f64) -> std::result::Result<f64, MathError> {
        match special::erf((x - self.mean) / (self.sd * SQRT2)) {
            Ok(e) => Ok(0.5 * (1.0 + e)),
            Err(MathError::MaxIterations) => {
                if x < (self.mean - 20.0 * self.sd) {
                    Ok(0.0)
                } else if x > (self.mean + 20.0 * self.sd) {
                    Ok(1.0)
                } else {
                    Err(MathError::MaxIterations)
                }
            }
            Err(e) => Err(e),
        }
    }
    /// `inverseF(u)` (bracket search: `BracketSearch::default()`).
    pub fn inverse_cdf(&self, u: f64) -> Result<f64> {
        self.inverse_cdf_with(u, BracketSearch::default())
    }
    /// `inverseF(u)` with an explicit bracket search.
    pub fn inverse_cdf_with(&self, u: f64, search: BracketSearch) -> Result<f64> {
        if u == 0.0 {
            return Ok(f64::NEG_INFINITY);
        }
        if u == 1.0 {
            return Ok(f64::INFINITY);
        }
        inverse_cumulative_continuous_with(self, u, search).map_err(|e| numerical("Normal", e))
    }
    /// One uniform, inverted.
    #[inline]
    pub fn sample<R: UniformSource + ?Sized>(&self, rng: &mut R) -> Result<f64> {
        self.inverse_cdf(rng.next_uniform())
    }
}

impl ContinuousDistribution for Normal {
    fn cdf(&self, x: f64) -> std::result::Result<f64, MathError> {
        Normal::cdf(self, x)
    }
    fn initial_domain(&self, p: f64) -> f64 {
        if p < 0.5 {
            self.mean - self.sd
        } else if p > 0.5 {
            self.mean + self.sd
        } else {
            self.mean
        }
    }
    fn domain_lower_bound(&self, p: f64) -> f64 {
        if p < 0.5 { -f64::MAX } else { self.mean }
    }
    fn domain_upper_bound(&self, p: f64) -> f64 {
        if p < 0.5 { self.mean } else { f64::MAX }
    }
    fn solver_absolute_accuracy(&self) -> f64 {
        1e-9
    }
}

// ---------------------------------------------------------------------------------------------

/// Palladio `LognormalDistributionImpl(mu, sigma)`: a `NormalDistributionImpl(mu, sigma)` whose
/// CDF is evaluated at `Math.log(x)` and whose solver bounds are `Math.exp` of the normal ones.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogNormal {
    normal: Normal,
}

impl LogNormal {
    /// `LognormalDistributionImpl(mu, sigma)`; `sigma` must be `> 0`.
    pub fn new(mu: f64, sigma: f64) -> Result<Self> {
        let normal = Normal::new(mu, sigma)
            .map_err(|_| invalid("Lognormal", "standard deviation must be positive", sigma))?;
        Ok(LogNormal { normal })
    }
    /// `LognormalDistributionFromMomentsImpl(mean, variance)`.
    pub fn from_moments(mean: f64, variance: f64) -> Result<Self> {
        if mean <= 0.0 {
            return Err(invalid(
                "LognormalFromMoments",
                "Mean has to be positive",
                mean,
            ));
        }
        if variance <= 0.0 {
            return Err(invalid(
                "LognormalFromMoments",
                "Variance has to be positive",
                variance,
            ));
        }
        let sigma2 = jmath::log(variance / (mean * mean) + 1.0);
        let mu = jmath::log(mean) - sigma2 / 2.0;
        let sigma = sigma2.sqrt();
        Self::new(mu, sigma)
    }
    /// `(mu, sigma)`.
    pub fn params(&self) -> (f64, f64) {
        self.normal.params()
    }
    /// `LognormalDistributionImpl.cumulativeProbability(x)`.
    pub fn cdf(&self, x: f64) -> std::result::Result<f64, MathError> {
        if x == 0.0 {
            return Ok(0.0);
        }
        self.normal.cdf(jmath::log(x))
    }
    /// `inverseF(u)` of the Palladio wrapper (0 for `u == 0`).
    pub fn inverse_cdf(&self, u: f64) -> Result<f64> {
        self.inverse_cdf_with(u, BracketSearch::default())
    }
    /// `inverseF(u)` with an explicit bracket search.
    pub fn inverse_cdf_with(&self, u: f64, search: BracketSearch) -> Result<f64> {
        if u == 0.0 {
            return Ok(0.0);
        }
        if u == 1.0 {
            return Ok(f64::INFINITY);
        }
        inverse_cumulative_continuous_with(self, u, search).map_err(|e| numerical("Lognormal", e))
    }
    /// One uniform, inverted.
    #[inline]
    pub fn sample<R: UniformSource + ?Sized>(&self, rng: &mut R) -> Result<f64> {
        self.inverse_cdf(rng.next_uniform())
    }
}

impl ContinuousDistribution for LogNormal {
    fn cdf(&self, x: f64) -> std::result::Result<f64, MathError> {
        LogNormal::cdf(self, x)
    }
    fn initial_domain(&self, p: f64) -> f64 {
        jmath::exp(self.normal.initial_domain(p))
    }
    fn domain_lower_bound(&self, p: f64) -> f64 {
        jmath::exp(ContinuousDistribution::domain_lower_bound(&self.normal, p))
    }
    fn domain_upper_bound(&self, p: f64) -> f64 {
        jmath::exp(ContinuousDistribution::domain_upper_bound(&self.normal, p))
    }
    fn solver_absolute_accuracy(&self) -> f64 {
        1e-9
    }
}

// ---------------------------------------------------------------------------------------------

/// `GammaDistributionImpl(alpha, beta = theta)` (Brent, accuracy 1e-9).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gamma {
    alpha: f64,
    beta: f64,
}

impl Gamma {
    /// `GammaDistributionImpl(alpha, theta)`: rejects `alpha <= 0`, then `theta <= 0`
    /// (NaN passes both checks, as in Java).
    pub fn new(alpha: f64, theta: f64) -> Result<Self> {
        if alpha <= 0.0 {
            return Err(invalid("Gamma", "alpha must be positive", alpha));
        }
        if theta <= 0.0 {
            return Err(invalid("Gamma", "beta must be positive", theta));
        }
        Ok(Gamma { alpha, beta: theta })
    }
    /// Palladio `GammaDistributionFromMoments(mean, coefficientOfVariance)`:
    /// `variance = cv*cv*mean*mean`, `theta = variance/mean`, `alpha = mean/theta`.
    pub fn from_moments(mean: f64, coeff_var: f64) -> Result<Self> {
        let variance = coeff_var * coeff_var * mean * mean;
        let theta = variance / mean;
        let alpha = mean / (variance / mean);
        Self::new(alpha, theta)
    }
    /// `(alpha, beta)`.
    pub fn params(&self) -> (f64, f64) {
        (self.alpha, self.beta)
    }
    /// `GammaDistributionImpl.cumulativeProbability(x)`.
    pub fn cdf(&self, x: f64) -> std::result::Result<f64, MathError> {
        if x <= 0.0 {
            Ok(0.0)
        } else {
            special::regularized_gamma_p(
                self.alpha,
                x / self.beta,
                special::GAMMA_DEFAULT_EPSILON,
                i32::MAX,
            )
        }
    }
    /// `inverseF(u)`.
    pub fn inverse_cdf(&self, u: f64) -> Result<f64> {
        self.inverse_cdf_with(u, BracketSearch::default())
    }
    /// `inverseF(u)` with an explicit bracket search.
    pub fn inverse_cdf_with(&self, u: f64, search: BracketSearch) -> Result<f64> {
        if u == 0.0 {
            return Ok(0.0);
        }
        if u == 1.0 {
            return Ok(f64::INFINITY);
        }
        inverse_cumulative_continuous_with(self, u, search).map_err(|e| numerical("Gamma", e))
    }
    /// One uniform, inverted.
    #[inline]
    pub fn sample<R: UniformSource + ?Sized>(&self, rng: &mut R) -> Result<f64> {
        self.inverse_cdf(rng.next_uniform())
    }
}

impl ContinuousDistribution for Gamma {
    fn cdf(&self, x: f64) -> std::result::Result<f64, MathError> {
        Gamma::cdf(self, x)
    }
    fn initial_domain(&self, p: f64) -> f64 {
        if p < 0.5 {
            self.alpha * self.beta * 0.5
        } else {
            self.alpha * self.beta
        }
    }
    fn domain_lower_bound(&self, _p: f64) -> f64 {
        f64::from_bits(1) // Double.MIN_VALUE
    }
    fn domain_upper_bound(&self, p: f64) -> f64 {
        if p < 0.5 {
            self.alpha * self.beta
        } else {
            f64::MAX
        }
    }
    fn solver_absolute_accuracy(&self) -> f64 {
        1e-9
    }
}

// ---------------------------------------------------------------------------------------------

/// Palladio `UniformDistributionImpl(a, b)` (inverted with bracketing + Brent at the default
/// accuracy 1e-6, so samples are not exactly `a + u*(b-a)`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UniformDouble {
    a: f64,
    b: f64,
}

impl UniformDouble {
    /// `UniformDistributionImpl(a, b)`: rejects `b < a`.
    pub fn new(a: f64, b: f64) -> Result<Self> {
        if b < a {
            return Err(invalid(
                "Uniform",
                "Second value has to be greater than first value of interval",
                b,
            ));
        }
        Ok(UniformDouble { a, b })
    }
    /// `(a, b)`.
    pub fn params(&self) -> (f64, f64) {
        (self.a, self.b)
    }
    /// `inverseF(u)`.
    pub fn inverse_cdf(&self, u: f64) -> Result<f64> {
        self.inverse_cdf_with(u, BracketSearch::default())
    }
    /// `inverseF(u)` with an explicit bracket search.
    pub fn inverse_cdf_with(&self, u: f64, search: BracketSearch) -> Result<f64> {
        inverse_cumulative_continuous_with(self, u, search).map_err(|e| numerical("Uniform", e))
    }
    /// One uniform, inverted.
    #[inline]
    pub fn sample<R: UniformSource + ?Sized>(&self, rng: &mut R) -> Result<f64> {
        self.inverse_cdf(rng.next_uniform())
    }
}

impl ContinuousDistribution for UniformDouble {
    fn cdf(&self, x: f64) -> std::result::Result<f64, MathError> {
        if x < self.a {
            Ok(0.0)
        } else if x > self.b {
            Ok(1.0)
        } else {
            Ok((x - self.a) / (self.b - self.a))
        }
    }
    fn initial_domain(&self, _p: f64) -> f64 {
        (self.a + self.b) / 2.0
    }
    fn domain_lower_bound(&self, _p: f64) -> f64 {
        self.a
    }
    fn domain_upper_bound(&self, _p: f64) -> f64 {
        self.b
    }
    fn solver_absolute_accuracy(&self) -> f64 {
        1e-6 // BrentSolver.DEFAULT_ABSOLUTE_ACCURACY (AbstractContinuousDistribution default)
    }
}

// ---------------------------------------------------------------------------------------------

/// Palladio `UniformIntDistributionImpl(a, b)` with the `+1` of `UniformIntDistribution.inverseF`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UniformInt {
    a: i32,
    b: i32,
    int_count: i32,
}

impl UniformInt {
    /// `UniformIntDistributionImpl(a, b)`: rejects `b < a`.
    pub fn new(a: i32, b: i32) -> Result<Self> {
        if b < a {
            return Err(invalid(
                "UniformInt",
                "Second value has to be greater than first value of interval",
                b as f64,
            ));
        }
        Ok(UniformInt {
            a,
            b,
            int_count: b.wrapping_sub(a).wrapping_add(1),
        })
    }
    /// `(a, b)`.
    pub fn params(&self) -> (i32, i32) {
        (self.a, self.b)
    }
    /// `inverseF(u)`: `value = super.inverseF(u); if (++value > b) value = b`.
    pub fn inverse_cdf(&self, u: f64) -> Result<i32> {
        let v = inverse_cumulative_int(self, u).map_err(|e| numerical("UniformInt", e))?;
        let v = v.wrapping_add(1);
        Ok(if v > self.b { self.b } else { v })
    }
    /// One uniform, inverted.
    #[inline]
    pub fn sample<R: UniformSource + ?Sized>(&self, rng: &mut R) -> Result<i32> {
        self.inverse_cdf(rng.next_uniform())
    }
}

impl IntegerDistribution for UniformInt {
    fn cdf_int(&self, x: i32) -> std::result::Result<f64, MathError> {
        if x < self.a {
            Ok(0.0)
        } else if x > self.b {
            Ok(1.0)
        } else {
            Ok(x.wrapping_sub(self.a).wrapping_add(1) as f64 / self.int_count as f64)
        }
    }
    fn domain_lower_bound(&self, _p: f64) -> i32 {
        self.a
    }
    fn domain_upper_bound(&self, _p: f64) -> i32 {
        self.b
    }
}

// ---------------------------------------------------------------------------------------------

/// `PoissonDistributionImpl(mean)` (bisection over `[0, i32::MAX]` on
/// `regularizedGammaQ(x + 1, mean, 1e-12, 10^7)`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Poisson {
    mean: f64,
}

impl Poisson {
    /// `PoissonDistributionImpl(mean)`: rejects `mean <= 0`.
    pub fn new(mean: f64) -> Result<Self> {
        if mean <= 0.0 {
            return Err(invalid(
                "Poisson",
                "the Poisson mean must be positive",
                mean,
            ));
        }
        Ok(Poisson { mean })
    }
    /// The mean.
    pub fn mean(&self) -> f64 {
        self.mean
    }
    /// `inverseF(u)`: the reference's bisection, with the CDF values memoized per thread (see
    /// [`PoissonCdfMemo`]; bit-identical to [`Poisson::inverse_cdf_unmemoized`]).
    pub fn inverse_cdf(&self, u: f64) -> Result<i32> {
        POISSON_MEMO
            .with(|m| {
                let mut m = m.borrow_mut();
                let memo = Memoized {
                    mean: self.mean,
                    table: RefCell::new(m.table(self.mean)),
                };
                inverse_cumulative_int(&memo, u)
            })
            .map_err(|e| numerical("Poisson", e))
    }
    /// `inverseF(u)`, every CDF value computed.
    pub fn inverse_cdf_unmemoized(&self, u: f64) -> Result<i32> {
        inverse_cumulative_int(self, u).map_err(|e| numerical("Poisson", e))
    }
    /// One uniform, inverted.
    #[inline]
    pub fn sample<R: UniformSource + ?Sized>(&self, rng: &mut R) -> Result<i32> {
        self.inverse_cdf(rng.next_uniform())
    }
}

/// Memo of Poisson CDF values `regularizedGammaQ(x + 1, mean)` for a few means (per thread).
///
/// The reference inverts by bisection over `[0, 2^31 - 1]`: about 31 CDF evaluations per sample,
/// most of them at the same points for every sample of a mean (the top of the bisection tree,
/// where the CDF is 1.0, and the few points around the likely results). The CDF is a pure
/// function of `(mean, x)`, so a memoized value is the value the bisection would compute; the
/// bisection itself, its comparisons and its result are unchanged. Errors and NaN are not
/// stored (they are recomputed, and fail the same way).
struct PoissonCdfMemo {
    /// Most recently used first.
    tables: Vec<PoissonTable>,
}

/// The memoized CDF values of one mean.
struct PoissonTable {
    mean_bits: u64,
    /// `x < SMALL`: `f64::to_bits`, `UNKNOWN` if not computed yet.
    small: Vec<u64>,
    /// `x >= SMALL`: open addressing, key `x` (`EMPTY` = free), at most half full.
    large: Vec<(i32, f64)>,
    large_len: usize,
}

const POISSON_MEANS: usize = 4;
const SMALL: usize = 256;
const LARGE_CAP: usize = 1 << 10;
const UNKNOWN: u64 = u64::MAX; // a NaN pattern; NaN values are never stored
const EMPTY: i32 = -1;

thread_local! {
    static POISSON_MEMO: RefCell<PoissonCdfMemo> =
        const { RefCell::new(PoissonCdfMemo { tables: Vec::new() }) };
}

impl PoissonCdfMemo {
    fn table(&mut self, mean: f64) -> &mut PoissonTable {
        let bits = mean.to_bits();
        match self.tables.iter().position(|t| t.mean_bits == bits) {
            Some(0) => {}
            Some(i) => self.tables[..=i].rotate_right(1),
            None => {
                if self.tables.len() == POISSON_MEANS {
                    self.tables.pop();
                }
                self.tables.insert(
                    0,
                    PoissonTable {
                        mean_bits: bits,
                        small: Vec::new(),
                        large: Vec::new(),
                        large_len: 0,
                    },
                );
            }
        }
        &mut self.tables[0]
    }
}

impl PoissonTable {
    #[inline]
    fn slot(&self, x: i32) -> usize {
        ((x as u32).wrapping_mul(0x9E37_79B9) >> 20) as usize & (self.large.len() - 1)
    }

    fn get(&self, x: i32) -> Option<f64> {
        if (x as usize) < SMALL {
            return match self.small.get(x as usize) {
                Some(&b) if b != UNKNOWN => Some(f64::from_bits(b)),
                _ => None,
            };
        }
        if self.large.is_empty() {
            return None;
        }
        let mut i = self.slot(x);
        loop {
            let (k, v) = self.large[i];
            if k == x {
                return Some(v);
            }
            if k == EMPTY {
                return None;
            }
            i = (i + 1) & (self.large.len() - 1);
        }
    }

    fn put(&mut self, x: i32, v: f64) {
        if (x as usize) < SMALL {
            if self.small.is_empty() {
                self.small = vec![UNKNOWN; SMALL];
            }
            self.small[x as usize] = v.to_bits();
            return;
        }
        if self.large.is_empty() || 2 * (self.large_len + 1) > LARGE_CAP {
            // first use, or full: start over (the values are recomputed on demand)
            self.large = vec![(EMPTY, 0.0); LARGE_CAP];
            self.large_len = 0;
        }
        let mut i = self.slot(x);
        while self.large[i].0 != EMPTY {
            i = (i + 1) & (self.large.len() - 1);
        }
        self.large[i] = (x, v);
        self.large_len += 1;
    }
}

/// A [`Poisson`] whose CDF values go through a [`PoissonTable`].
struct Memoized<'a> {
    mean: f64,
    table: RefCell<&'a mut PoissonTable>,
}

impl IntegerDistribution for Memoized<'_> {
    fn cdf_int(&self, x: i32) -> std::result::Result<f64, MathError> {
        if x < 0 || x == i32::MAX {
            return Poisson { mean: self.mean }.cdf_int(x);
        }
        if let Some(v) = self.table.borrow().get(x) {
            return Ok(v);
        }
        let v = Poisson { mean: self.mean }.cdf_int(x)?;
        if !v.is_nan() {
            self.table.borrow_mut().put(x, v);
        }
        Ok(v)
    }
    fn domain_lower_bound(&self, _p: f64) -> i32 {
        0
    }
    fn domain_upper_bound(&self, _p: f64) -> i32 {
        i32::MAX
    }
}

impl IntegerDistribution for Poisson {
    fn cdf_int(&self, x: i32) -> std::result::Result<f64, MathError> {
        if x < 0 {
            return Ok(0.0);
        }
        if x == i32::MAX {
            return Ok(1.0);
        }
        special::regularized_gamma_q(x as f64 + 1.0, self.mean, 1e-12, 10_000_000)
    }
    fn domain_lower_bound(&self, _p: f64) -> i32 {
        0
    }
    fn domain_upper_bound(&self, _p: f64) -> i32 {
        i32::MAX
    }
}

// ---------------------------------------------------------------------------------------------
// StoEx functions (SimuCom `FunctionLib`, 5.2.2). Each: parameter check (no uniform), construction
// (no uniform), one uniform, inversion.

fn check(ok: bool, function: &'static str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(DistError::ParametersNotAccepted { function })
    }
}

/// `Exp(rate)`: requires `rate > 0` (`!(rate <= 0)`, so NaN passes the check).
pub fn sample_exp<R: UniformSource + ?Sized>(rate: f64, rng: &mut R) -> Result<f64> {
    check(!(rate <= 0.0), "Exp")?;
    Exponential::new(rate)?.sample(rng)
}

/// `Norm(mean, sd)` (no parameter check beyond arity; `sd <= 0` fails in the constructor).
pub fn sample_norm<R: UniformSource + ?Sized>(mean: f64, sd: f64, rng: &mut R) -> Result<f64> {
    sample_norm_with(mean, sd, BracketSearch::default(), rng)
}

/// [`sample_norm`] with an explicit bracket search.
pub fn sample_norm_with<R: UniformSource + ?Sized>(
    mean: f64,
    sd: f64,
    search: BracketSearch,
    rng: &mut R,
) -> Result<f64> {
    let d = Normal::new(mean, sd)?;
    d.inverse_cdf_with(rng.next_uniform(), search)
}

/// `Lognorm(mu, sigma)`: requires `sigma > 0`.
pub fn sample_lognorm<R: UniformSource + ?Sized>(mu: f64, sigma: f64, rng: &mut R) -> Result<f64> {
    sample_lognorm_with(mu, sigma, BracketSearch::default(), rng)
}

/// [`sample_lognorm`] with an explicit bracket search.
pub fn sample_lognorm_with<R: UniformSource + ?Sized>(
    mu: f64,
    sigma: f64,
    search: BracketSearch,
    rng: &mut R,
) -> Result<f64> {
    check(!(sigma <= 0.0), "Lognorm")?;
    let d = LogNormal::new(mu, sigma)?;
    d.inverse_cdf_with(rng.next_uniform(), search)
}

/// `LognormMoments(mean, stdev)`: requires `mean >= 0` and `stdev >= 0`;
/// then `LognormalDistributionFromMoments(mean, stdev * stdev)`.
pub fn sample_lognorm_moments<R: UniformSource + ?Sized>(
    mean: f64,
    stdev: f64,
    rng: &mut R,
) -> Result<f64> {
    sample_lognorm_moments_with(mean, stdev, BracketSearch::default(), rng)
}

/// [`sample_lognorm_moments`] with an explicit bracket search.
pub fn sample_lognorm_moments_with<R: UniformSource + ?Sized>(
    mean: f64,
    stdev: f64,
    search: BracketSearch,
    rng: &mut R,
) -> Result<f64> {
    check(!(mean < 0.0) && !(stdev < 0.0), "LognormMoments")?;
    let d = LogNormal::from_moments(mean, stdev * stdev)?;
    d.inverse_cdf_with(rng.next_uniform(), search)
}

/// `Gamma(alpha, theta)`: requires `theta > 0` and `alpha > 0`.
pub fn sample_gamma<R: UniformSource + ?Sized>(alpha: f64, theta: f64, rng: &mut R) -> Result<f64> {
    sample_gamma_with(alpha, theta, BracketSearch::default(), rng)
}

/// [`sample_gamma`] with an explicit bracket search.
pub fn sample_gamma_with<R: UniformSource + ?Sized>(
    alpha: f64,
    theta: f64,
    search: BracketSearch,
    rng: &mut R,
) -> Result<f64> {
    check(!(theta <= 0.0) && !(alpha <= 0.0), "Gamma")?;
    let d = Gamma::new(alpha, theta)?;
    d.inverse_cdf_with(rng.next_uniform(), search)
}

/// `GammaMoments(mean, coeffVar)`: requires both `>= 0`.
pub fn sample_gamma_moments<R: UniformSource + ?Sized>(
    mean: f64,
    coeff_var: f64,
    rng: &mut R,
) -> Result<f64> {
    sample_gamma_moments_with(mean, coeff_var, BracketSearch::default(), rng)
}

/// [`sample_gamma_moments`] with an explicit bracket search.
pub fn sample_gamma_moments_with<R: UniformSource + ?Sized>(
    mean: f64,
    coeff_var: f64,
    search: BracketSearch,
    rng: &mut R,
) -> Result<f64> {
    check(!(mean < 0.0) && !(coeff_var < 0.0), "GammaMoments")?;
    let d = Gamma::from_moments(mean, coeff_var)?;
    d.inverse_cdf_with(rng.next_uniform(), search)
}

/// `Pois(mean)`: requires `mean >= 0` (`mean == 0` then fails in the constructor).
pub fn sample_pois<R: UniformSource + ?Sized>(mean: f64, rng: &mut R) -> Result<i32> {
    check(!(mean < 0.0), "Pois")?;
    Poisson::new(mean)?.sample(rng)
}

/// `UniDouble(a, b)`: requires `!(a > b)` (`a == b` then fails in the inversion).
pub fn sample_unidouble<R: UniformSource + ?Sized>(a: f64, b: f64, rng: &mut R) -> Result<f64> {
    sample_unidouble_with(a, b, BracketSearch::default(), rng)
}

/// [`sample_unidouble`] with an explicit bracket search.
pub fn sample_unidouble_with<R: UniformSource + ?Sized>(
    a: f64,
    b: f64,
    search: BracketSearch,
    rng: &mut R,
) -> Result<f64> {
    check(!(a > b), "UniDouble")?;
    let d = UniformDouble::new(a, b)?;
    d.inverse_cdf_with(rng.next_uniform(), search)
}

/// `UniInt(a, b)`: both arguments must be StoEx integers (the caller checks the types;
/// otherwise `ParametersNotAccepted`).
pub fn sample_uniint<R: UniformSource + ?Sized>(a: i32, b: i32, rng: &mut R) -> Result<i32> {
    UniformInt::new(a, b)?.sample(rng)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MersenneTwister;

    /// The memoized bisection gives the unmemoized result for every uniform, also when the
    /// means alternate (more means than memo tables), for tiny and large means and for the
    /// edges of `[0, 1)`.
    #[test]
    fn poisson_memo_is_exact() {
        let mut r = MersenneTwister::from_seed(&[7, 8, 9, 10, 11, 12]).unwrap();
        let means = [
            0.3, 1.5, 2.0, 2.5, 4.0, 1e-9, 17.25, 250.0, 3000.0, 1.5, 2.0, 1e6, 2.0,
        ];
        let edges = [
            0.0,
            1e-300,
            1e-17,
            0.5,
            1.0 - 1e-16,
            1.0 - f64::EPSILON / 2.0,
        ];
        for round in 0..3 {
            for &m in &means {
                let p = Poisson::new(m).unwrap();
                let n = if m > 1000.0 { 40 } else { 400 };
                for i in 0..n + edges.len() {
                    let u = if i < n {
                        r.next_uniform()
                    } else {
                        edges[i - n]
                    };
                    assert_eq!(
                        p.inverse_cdf(u),
                        p.inverse_cdf_unmemoized(u),
                        "round {round} mean {m} u {u}"
                    );
                }
            }
        }
    }
}

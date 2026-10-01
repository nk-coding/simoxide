//! Uniform sources: the trait, the reference stream, and tape recording / replay.

use crate::dist::{self, DistError};
use crate::mt::MersenneTwister;

/// A source of uniform random numbers in `[0, 1)` (the reference's `IRandomGenerator.random()`),
/// plus the StoEx distribution functions sampled from it.
///
/// The `sample_*` methods are hooks: by default they are exactly the reference's functions of
/// [`crate::dist`] (parameter check, construction, one uniform, numerical inversion). A source
/// may override them with other algorithms for the same distributions (the fast mode's
/// [`crate::fast::FastSource`], feature `fast`); evaluators call the hooks, so the choice is
/// made at compile time by the source type.
pub trait UniformSource {
    /// Next uniform in `[0, 1)`.
    fn next_uniform(&mut self) -> f64;

    /// StoEx `Exp(rate)` ([`dist::sample_exp`]).
    #[inline]
    fn sample_exp(&mut self, rate: f64) -> Result<f64, DistError> {
        dist::sample_exp(rate, self)
    }
    /// StoEx `Norm(mean, sd)` ([`dist::sample_norm`]).
    #[inline]
    fn sample_norm(&mut self, mean: f64, sd: f64) -> Result<f64, DistError> {
        dist::sample_norm(mean, sd, self)
    }
    /// StoEx `Lognorm(mu, sigma)` ([`dist::sample_lognorm`]).
    #[inline]
    fn sample_lognorm(&mut self, mu: f64, sigma: f64) -> Result<f64, DistError> {
        dist::sample_lognorm(mu, sigma, self)
    }
    /// StoEx `LognormMoments(mean, stdev)` ([`dist::sample_lognorm_moments`]).
    #[inline]
    fn sample_lognorm_moments(&mut self, mean: f64, stdev: f64) -> Result<f64, DistError> {
        dist::sample_lognorm_moments(mean, stdev, self)
    }
    /// StoEx `Gamma(alpha, theta)` ([`dist::sample_gamma`]).
    #[inline]
    fn sample_gamma(&mut self, alpha: f64, theta: f64) -> Result<f64, DistError> {
        dist::sample_gamma(alpha, theta, self)
    }
    /// StoEx `GammaMoments(mean, coeffVar)` ([`dist::sample_gamma_moments`]).
    #[inline]
    fn sample_gamma_moments(&mut self, mean: f64, coeff_var: f64) -> Result<f64, DistError> {
        dist::sample_gamma_moments(mean, coeff_var, self)
    }
    /// StoEx `Pois(mean)` ([`dist::sample_pois`]; Poisson − 1, REF-4).
    #[inline]
    fn sample_pois(&mut self, mean: f64) -> Result<i32, DistError> {
        dist::sample_pois(mean, self)
    }
    /// StoEx `UniDouble(a, b)` ([`dist::sample_unidouble`]).
    #[inline]
    fn sample_unidouble(&mut self, a: f64, b: f64) -> Result<f64, DistError> {
        dist::sample_unidouble(a, b, self)
    }
    /// StoEx `UniInt(a, b)` ([`dist::sample_uniint`]).
    #[inline]
    fn sample_uniint(&mut self, a: i32, b: i32) -> Result<i32, DistError> {
        dist::sample_uniint(a, b, self)
    }
}

/// Forwards `next_uniform` and every sampling hook to the wrapped source.
macro_rules! forward_uniform_source {
    () => {
        #[inline]
        fn next_uniform(&mut self) -> f64 {
            (**self).next_uniform()
        }
        #[inline]
        fn sample_exp(&mut self, rate: f64) -> Result<f64, DistError> {
            (**self).sample_exp(rate)
        }
        #[inline]
        fn sample_norm(&mut self, mean: f64, sd: f64) -> Result<f64, DistError> {
            (**self).sample_norm(mean, sd)
        }
        #[inline]
        fn sample_lognorm(&mut self, mu: f64, sigma: f64) -> Result<f64, DistError> {
            (**self).sample_lognorm(mu, sigma)
        }
        #[inline]
        fn sample_lognorm_moments(&mut self, mean: f64, stdev: f64) -> Result<f64, DistError> {
            (**self).sample_lognorm_moments(mean, stdev)
        }
        #[inline]
        fn sample_gamma(&mut self, alpha: f64, theta: f64) -> Result<f64, DistError> {
            (**self).sample_gamma(alpha, theta)
        }
        #[inline]
        fn sample_gamma_moments(&mut self, mean: f64, coeff_var: f64) -> Result<f64, DistError> {
            (**self).sample_gamma_moments(mean, coeff_var)
        }
        #[inline]
        fn sample_pois(&mut self, mean: f64) -> Result<i32, DistError> {
            (**self).sample_pois(mean)
        }
        #[inline]
        fn sample_unidouble(&mut self, a: f64, b: f64) -> Result<f64, DistError> {
            (**self).sample_unidouble(a, b)
        }
        #[inline]
        fn sample_uniint(&mut self, a: i32, b: i32) -> Result<i32, DistError> {
            (**self).sample_uniint(a, b)
        }
    };
}

impl<S: UniformSource + ?Sized> UniformSource for &mut S {
    forward_uniform_source!();
}

impl<S: UniformSource + ?Sized> UniformSource for Box<S> {
    forward_uniform_source!();
}

/// The reference stream: `SimuComDefaultRandomNumberGenerator` = Commons Math `MersenneTwister`.
pub type SimuComStream = MersenneTwister;

/// Error for seeds that the reference rejects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedError {
    /// A seed long does not fit in a Java `int` (`ApacheMathRandomGenerator.setSeed(long[])`).
    NotAnInt(i64),
    /// The seed array does not have exactly six entries.
    WrongLength(usize),
}

impl std::fmt::Display for SeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SeedError::NotAnInt(s) => {
                write!(f, "{s} cannot be cast to int without changing its value.")
            }
            SeedError::WrongLength(n) => write!(
                f,
                "Seed array must have length of six longs for initialising random number generator (got {n})"
            ),
        }
    }
}

impl std::error::Error for SeedError {}

impl MersenneTwister {
    /// The reference stream for a run configured with `fixedSeed0..5`
    /// (`SimuComDefaultRandomNumberGenerator(long[])`).
    pub fn from_seed(seed: &[i64]) -> Result<Self, SeedError> {
        if seed.len() != 6 {
            return Err(SeedError::WrongLength(seed.len()));
        }
        let mut ints = [0i32; 6];
        for (d, &s) in ints.iter_mut().zip(seed) {
            *d = i32::try_from(s).map_err(|_| SeedError::NotAnInt(s))?;
        }
        Ok(MersenneTwister::from_int_array(&ints))
    }
}

impl UniformSource for MersenneTwister {
    #[inline]
    fn next_uniform(&mut self) -> f64 {
        self.next_double()
    }
}

/// Wraps a source and records every uniform drawn through it.
#[derive(Debug, Clone)]
pub struct Recorder<S> {
    inner: S,
    tape: Vec<f64>,
}

impl<S: UniformSource> Recorder<S> {
    /// Starts recording with an empty tape.
    pub fn new(inner: S) -> Self {
        Recorder {
            inner,
            tape: Vec::new(),
        }
    }
    /// The uniforms drawn so far.
    pub fn tape(&self) -> &[f64] {
        &self.tape
    }
    /// Number of uniforms drawn so far.
    pub fn count(&self) -> usize {
        self.tape.len()
    }
    /// Returns the wrapped source and the tape.
    pub fn into_parts(self) -> (S, Vec<f64>) {
        (self.inner, self.tape)
    }
}

impl<S: UniformSource> UniformSource for Recorder<S> {
    #[inline]
    fn next_uniform(&mut self) -> f64 {
        let u = self.inner.next_uniform();
        self.tape.push(u);
        u
    }
}

/// Replays a recorded tape of uniforms. Panics when the tape is exhausted (a divergence between
/// the replaying and the recording run); use [`Replay::try_next`] to check instead.
#[derive(Debug, Clone)]
pub struct Replay<'a> {
    tape: &'a [f64],
    pos: usize,
}

impl<'a> Replay<'a> {
    /// Replays `tape` from the start.
    pub fn new(tape: &'a [f64]) -> Self {
        Replay { tape, pos: 0 }
    }
    /// Uniforms consumed so far.
    pub fn position(&self) -> usize {
        self.pos
    }
    /// Uniforms left on the tape.
    pub fn remaining(&self) -> usize {
        self.tape.len() - self.pos
    }
    /// Next uniform, or `None` when the tape is exhausted.
    #[inline]
    pub fn try_next(&mut self) -> Option<f64> {
        let u = *self.tape.get(self.pos)?;
        self.pos += 1;
        Some(u)
    }
}

impl UniformSource for Replay<'_> {
    #[inline]
    fn next_uniform(&mut self) -> f64 {
        match self.try_next() {
            Some(u) => u,
            None => panic!(
                "random tape exhausted after {} uniforms (replay diverged from recording)",
                self.pos
            ),
        }
    }
}

/// Replays a fixed sequence cyclically (tests, benchmarks).
#[derive(Debug, Clone)]
pub struct Cycle<'a> {
    values: &'a [f64],
    pos: usize,
}

impl<'a> Cycle<'a> {
    /// Cycles through `values` (must be non-empty).
    pub fn new(values: &'a [f64]) -> Self {
        assert!(!values.is_empty(), "Cycle needs at least one value");
        Cycle { values, pos: 0 }
    }
}

impl UniformSource for Cycle<'_> {
    #[inline]
    fn next_uniform(&mut self) -> f64 {
        let u = self.values[self.pos];
        self.pos += 1;
        if self.pos == self.values.len() {
            self.pos = 0;
        }
        u
    }
}

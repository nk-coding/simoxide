//! Bit-exact port of the random number generation and distribution sampling used by
//! SimuLizar 5.2.2 (see `docs/spec/random.md` for the Java sources this follows).
//!
//! # What the reference does
//!
//! * One uniform stream per simulation run: `SimuComDefaultRandomNumberGenerator` wrapping
//!   `MT19937RandomGenerator`, i.e. Apache Commons Math **2.1** `MersenneTwister`, seeded with the
//!   six `fixedSeed` longs of the run configuration (each must fit in an `int`; they are passed to
//!   `MersenneTwister.setSeed(int[])`). Every random decision draws `nextDouble()` from it.
//! * Every named distribution in StoEx (`Exp`, `Norm`, `Lognorm`, `LognormMoments`, `Gamma`,
//!   `GammaMoments`, `Pois`, `UniDouble`, `UniInt`) is sampled by **inversion**: exactly one
//!   uniform per sample, passed to the Commons Math 2.1 `inverseCumulativeProbability` (or a
//!   Palladio subclass of it). Most of these invert the CDF numerically (bracketing + Brent
//!   solver, or bisection for the discrete ones); this crate ports those algorithms step by step.
//! * PMF literals: linear scan over the running sums of the (value-sorted) probabilities, first
//!   index with `u < cum[i]`. Boxed PDFs: the same scan, then linear interpolation inside the box.
//! * Probabilistic branches (SimuLizar `TransitionDeterminer`): first index with
//!   `last_sum * u < cum[i]`.
//! * `java.lang.Math.log`/`exp` are HotSpot intrinsics on x86-64 (Intel LIBM stubs), not fdlibm.
//!   With the `hotspot-math` feature (off by default), [`jmath`] uses a port of those stubs so that
//!   results match bit for bit; the port is GPL-2.0-only (see `LICENSES`). Without it, [`jmath`]
//!   uses the correctly rounded [`crmath`] functions, which differ from HotSpot by one ulp on
//!   about 0.25 % of `exp` and 1e-5 of `log` arguments, so a run can eventually diverge from the
//!   reference.
//!
//! # API overview
//!
//! * [`UniformSource`]: anything that yields uniforms in `[0, 1)`. Implemented by
//!   [`MersenneTwister`] (the reference generator), [`Recorder`] (records every uniform it passes
//!   through), [`Replay`] (replays a recorded tape), [`source::Cycle`] and `&mut S`.
//! * [`SimuComStream::from_seed`] builds the reference stream from the six configuration seeds.
//! * [`dist`]: one type per distribution with `new(params) -> Result<Self, DistError>`,
//!   `inverse_cdf(u)` and `sample(&mut impl UniformSource)`, plus free functions
//!   [`dist::sample_exp`] etc. that do exactly what the StoEx function of the same name does per
//!   call (parameter check, construction, one uniform, inversion). A parameter or construction
//!   error never consumes a uniform (as in Java, where the distribution is created before
//!   `random()` is called).
//! * [`probfn`]: [`probfn::PmfSampler`], [`probfn::BoxedPdfSampler`], the probability adjustment
//!   and validation of `ProbFunctionCache`, Java's sort order for PMF sample values and
//!   [`probfn::branch_index`].
//! * [`jmath`] and [`special`]: `java.lang.Math` semantics (log, exp, max, min) and the Commons
//!   Math 2.1 special functions (`Gamma`, `Erf`, continued fraction, Brent solver), exposed
//!   because other crates may need the same bit-exact functions (e.g. StoEx `Log`).
//! * [`dist::BracketSearch`]: the numerically inverted distributions bracket the root in steps
//!   of 1.0 like the reference (`Linear`, default). `Galloping` finds the same bracket with
//!   O(log) CDF evaluations (identical on all oracle cases; default with feature
//!   `fast-bracket`); use the `*_with` functions to choose per call.
//!
//! Reference quirks that are reproduced on purpose: `Pois(m)` returns Poisson(m) − 1 (Commons
//! Math 2.1 integer inversion); `UniDouble` is a Brent approximation (accuracy 1e-6); a PMF draw
//! beyond the last cumulative sum yields `Double 0.0` (see `docs/spec/random.md`).
//!
//! All sampling functions are allocation-free; samplers precompute their cumulative arrays once.
//!
//! Features: `hotspot-math` (bit-exact `Math.log`/`exp`, GPL-2.0-only, see above),
//! `rcp-table` (with `hotspot-math`: use the recorded AMD Zen 5 `rcpss` table in `jmath::log`
//! instead of the instruction), `fast-bracket` (default `BracketSearch::Galloping`), `fast` (the
//! [`fast`] module: xoshiro256++ and standard sampling algorithms for SimOxide's fast mode, not
//! bit-exact).
//!
//! The StoEx distribution functions are hooks of [`UniformSource`] (`sample_norm`, ...) whose
//! default implementations are the reference's functions of [`dist`]; a source type can replace
//! them (the fast mode's `fast::FastSource` does).

pub mod crmath;
mod crmath_tables;
pub mod dist;
#[cfg(feature = "fast")]
pub mod fast;
pub mod jmath;
pub mod mt;
pub mod probfn;
pub mod source;
pub mod special;

pub use dist::DistError;
pub use mt::MersenneTwister;
pub use source::{Recorder, Replay, SimuComStream, UniformSource};

//! Exact and fast mode: what the simulator does only to reproduce SimuLizar's random numbers.
//!
//! A [`Simulation`](crate::Simulation) is generic over a [`Compat`] policy, fixed at compile
//! time and monomorphized, so the choice costs nothing per event. The policies differ only in
//! what changes values, the random numbers; everything that is result-neutral (merged hand-off
//! events, `INNER` without copies, see below) is done in every mode.
//!
//! * `Exact` (SimuLizar-exact; cargo feature `exact`): the reference's random stream (MT19937),
//!   its numerical inverse CDFs and HotSpot's `Math.log` (bit-exact with `hotspot-math`), with
//!   the random tape, and Java `HashMap` iteration order where the reference iterates one
//!   (`INNER` characterisations, resource finalisation). Traces, tapes and measurements are
//!   byte-identical to the reference. The `HashMap` order comes from `simoxide-jcompat-exact`,
//!   for which no licence is granted (`LICENSES`): builds with `exact` must not be distributed.
//! * [`Fast`] (always compiled; the default without `exact`): xoshiro256++ and standard samplers
//!   (ziggurat, Marsaglia–Tsang, PTRS, `a + u·(b − a)`) instead of MT19937 and numerical
//!   inversion ([`simoxide_random::fast`]); parameter checks and errors are the reference's; no
//!   random tape, no tape replay, no per-draw bookkeeping; maps the reference iterates in
//!   `HashMap` order are iterated in insertion order (see `docs/correctness/deviations.md`,
//!   "Fast mode").
//!
//! The engine is the same in both modes. It deviates from DESMO-J's literal event sequence in
//! two places, without changing any output (trace, tape, measurements, event count):
//!
//! - a process woken by an engine-level event (end of a `hold`, of a DELAY demand, of a PS/FCFS
//!   demand) runs inside that event instead of in a separate `Resume` note at the same time,
//!   when that note would be the next one anyway (no other note pending at that instant, the
//!   run does not stop after the event, no error pending). The elided note is still counted as
//!   an event and passes the per-event checks (livelock guard, `Limits`); the trace has no line
//!   for it;
//! - `INNER` characterisations are evaluated while visiting the frame chain (no
//!   `getContents()` copy), in the same order ([`Compat::Order`]) and with the same tape origin.
//!
//! [`Literal`] runs a policy on the literal engine (separate `Resume` notes, copied frame
//! contents). It exists to test that both engines give the same outputs (`tests/fast_mode.rs`,
//! `tests/literal_engine.rs`); it is not a run-time [`Mode`].
//!
//! [`Mode`] names the policy at run time; [`crate::run`] and [`crate::run_batch`] dispatch on
//! [`SimConfig::mode`](crate::SimConfig::mode) once per run.

use crate::ir::{CompiledModel, ResIdx};
use crate::rng::{Origin, RngMode};
use simoxide_jcompat::order::{InsertionOrder, MapOrder};
use simoxide_random::UniformSource;
use simoxide_stoex::Value;

/// Which simulator variant a run uses (see the module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// Byte-identical to the reference (needs the cargo feature `exact`; the default with it).
    Exact,
    /// Same semantics and statistics, not the reference's random numbers (always compiled; the
    /// default without `exact`).
    Fast,
}

impl Default for Mode {
    /// [`Mode::Exact`] in builds with the feature `exact`, else [`Mode::Fast`].
    fn default() -> Self {
        if cfg!(feature = "exact") {
            Mode::Exact
        } else {
            Mode::Fast
        }
    }
}

impl Mode {
    /// `exact`, `fast`.
    pub fn name(self) -> &'static str {
        match self {
            Mode::Exact => "exact",
            Mode::Fast => "fast",
        }
    }

    /// Parses [`Mode::name`].
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "exact" => Some(Mode::Exact),
            "fast" => Some(Mode::Fast),
            _ => None,
        }
    }

    /// Whether this build can run the mode (the exact mode needs the cargo feature `exact`).
    pub fn available(self) -> bool {
        self == Mode::Fast || cfg!(feature = "exact")
    }

    /// Why this build cannot run the mode (`None` if it can).
    pub fn unavailable_reason(self) -> Option<String> {
        (!self.available()).then(|| {
            format!(
                "mode {self} is not compiled in: build with the cargo feature `exact` \
                 (simoxide-cli: `--features exact`, with `hotspot-math` for bit-exact log/exp)"
            )
        })
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A compile-time simulator policy (implemented by `Exact`, [`Fast`] and [`Literal`]).
pub trait Compat: 'static {
    /// The run's uniform source and distribution samplers.
    #[doc(hidden)]
    type Rng: SimRng;
    /// The iteration order of the maps the reference keeps in a Java `HashMap` (`INNER`
    /// characterisations, ND-12/13): Java's (`exact`) or insertion order (fast).
    #[doc(hidden)]
    type Order: MapOrder;
    /// The run-time name of this policy.
    const MODE: Mode;
    /// Run DESMO-J's event sequence literally: a separate `Resume` note for every hand-off and
    /// a copy of the frame contents for `INNER` (only [`Literal`], for equivalence tests).
    #[doc(hidden)]
    const LITERAL_ENGINE: bool = false;
    /// `deactivateAllActiveResources` order of the model's resources ([`Self::Order`] over the
    /// resource registry).
    #[doc(hidden)]
    fn finalise_order(cm: &CompiledModel) -> &[ResIdx] {
        &cm.finalise_order_fast
    }
}

/// Byte-identical to the reference (SimuLizar-exact; cargo feature `exact`).
#[cfg(feature = "exact")]
#[derive(Debug, Clone, Copy)]
pub struct Exact;

#[cfg(feature = "exact")]
impl Compat for Exact {
    type Rng = crate::rng::Rng;
    type Order = simoxide_jcompat_exact::order::JdkHashOrder;
    const MODE: Mode = Mode::Exact;
    #[inline]
    fn finalise_order(cm: &CompiledModel) -> &[ResIdx] {
        &cm.finalise_order
    }
}

/// Fast mode: same semantics and statistics, not the reference's random numbers.
#[derive(Debug, Clone, Copy)]
pub struct Fast;

impl Compat for Fast {
    type Rng = FastSourceRng;
    type Order = InsertionOrder;
    const MODE: Mode = Mode::Fast;
}

/// The policy that type parameters default to: `Exact` with the feature `exact`, else [`Fast`].
#[cfg(feature = "exact")]
pub type DefaultCompat = Exact;
/// The policy that type parameters default to: `Exact` with the feature `exact`, else [`Fast`].
#[cfg(not(feature = "exact"))]
pub type DefaultCompat = Fast;

/// Policy `P` on the literal engine (see the module docs): the same outputs as `P`, more
/// notes processed. For tests: `Simulation::<Literal<Fast>>::create(..)` with
/// `cfg.mode = P::MODE`.
#[derive(Debug, Clone, Copy)]
pub struct Literal<P>(std::marker::PhantomData<P>);

impl<P: Compat> Compat for Literal<P> {
    type Rng = P::Rng;
    type Order = P::Order;
    const MODE: Mode = P::MODE;
    const LITERAL_ENGINE: bool = true;
    #[inline]
    fn finalise_order(cm: &CompiledModel) -> &[ResIdx] {
        P::finalise_order(cm)
    }
}

/// What the simulator needs from its random source besides uniforms and samples: the exact
/// mode's tape bookkeeping (origin tags, `s` records, replayed samples), no-ops in fast mode.
/// Part of the run state ([`crate::RunState`]): `Clone` continues the same stream; it holds no
/// writer (the tape records are buffered, see [`SimRng::tape_buf`]).
#[doc(hidden)]
pub trait SimRng: UniformSource + Sized + Clone + Send + Sync {
    fn create(
        mode: &RngMode,
        seed: i64,
        tape: bool,
        check_origins: bool,
        use_samples: bool,
    ) -> Result<Self, String>;
    /// Switches the tape records on or off (start of a run, or a pause).
    fn attach_tape(&mut self, on: bool) -> Result<(), String>;
    /// The buffered tape records, written out and cleared by the simulation.
    fn tape_buf(&mut self) -> Option<&mut String>;
    /// Evaluations need no bookkeeping (no tape output, no replay).
    fn plain(&self) -> bool;
    /// Uniforms (exact) or samples (fast) drawn so far.
    fn count(&self) -> u64;
    fn set_origin(&mut self, o: Origin<'_>) -> Option<String>;
    fn restore_origin(&mut self, prev: Option<String>);
    fn eval_begin(&mut self);
    fn recorded_sample(&self) -> Option<Value>;
    fn eval_end(&mut self, spec: &str, v: Option<&Value>);
    /// The tape ran out (replay): the run must stop.
    fn exhausted(&self) -> Option<&str>;
    fn take_problem(&mut self) -> Option<String>;
}

impl SimRng for crate::rng::Rng {
    fn create(
        mode: &RngMode,
        seed: i64,
        tape: bool,
        check_origins: bool,
        use_samples: bool,
    ) -> Result<Self, String> {
        crate::rng::Rng::new(mode, seed, tape, check_origins, use_samples)
    }
    fn attach_tape(&mut self, on: bool) -> Result<(), String> {
        crate::rng::Rng::attach_tape(self, on);
        Ok(())
    }
    #[inline]
    fn tape_buf(&mut self) -> Option<&mut String> {
        crate::rng::Rng::tape_buf(self)
    }
    #[inline]
    fn plain(&self) -> bool {
        self.plain
    }
    #[inline]
    fn count(&self) -> u64 {
        self.count
    }
    #[inline]
    fn set_origin(&mut self, o: Origin<'_>) -> Option<String> {
        crate::rng::Rng::set_origin(self, o)
    }
    #[inline]
    fn restore_origin(&mut self, prev: Option<String>) {
        crate::rng::Rng::restore_origin(self, prev)
    }
    #[inline]
    fn eval_begin(&mut self) {
        crate::rng::Rng::eval_begin(self)
    }
    #[inline]
    fn recorded_sample(&self) -> Option<Value> {
        crate::rng::Rng::recorded_sample(self)
    }
    #[inline]
    fn eval_end(&mut self, spec: &str, v: Option<&Value>) {
        crate::rng::Rng::eval_end(self, spec, v)
    }
    #[inline]
    fn exhausted(&self) -> Option<&str> {
        self.problem
            .as_deref()
            .filter(|p| p.starts_with("random tape exhausted"))
    }
    fn take_problem(&mut self) -> Option<String> {
        self.problem.take()
    }
}

/// The fast mode's source: [`simoxide_random::fast::FastSource`] plus a sample counter.
#[doc(hidden)]
#[derive(Clone)]
pub struct FastSourceRng {
    src: simoxide_random::fast::FastSource,
    count: u64,
}

macro_rules! counted {
    ($($name:ident($($a:ident: $t:ty),*) -> $r:ty;)*) => {
        $(
            #[inline]
            fn $name(&mut self, $($a: $t),*) -> Result<$r, simoxide_random::DistError> {
                self.count += 1;
                self.src.$name($($a),*)
            }
        )*
    };
}

impl UniformSource for FastSourceRng {
    #[inline]
    fn next_uniform(&mut self) -> f64 {
        self.count += 1;
        self.src.uniform()
    }
    counted! {
        sample_exp(rate: f64) -> f64;
        sample_norm(mean: f64, sd: f64) -> f64;
        sample_lognorm(mu: f64, sigma: f64) -> f64;
        sample_lognorm_moments(mean: f64, stdev: f64) -> f64;
        sample_gamma(alpha: f64, theta: f64) -> f64;
        sample_gamma_moments(mean: f64, coeff_var: f64) -> f64;
        sample_pois(mean: f64) -> i32;
        sample_unidouble(a: f64, b: f64) -> f64;
        sample_uniint(a: i32, b: i32) -> i32;
    }
}

impl SimRng for FastSourceRng {
    fn create(
        mode: &RngMode,
        seed: i64,
        tape: bool,
        _check_origins: bool,
        _use_samples: bool,
    ) -> Result<Self, String> {
        if matches!(mode, RngMode::Replay(_)) {
            return Err("fast mode cannot replay a random tape (use --mode exact)".into());
        }
        let mut r = FastSourceRng {
            src: simoxide_random::fast::FastSource::new(seed as u64),
            count: 0,
        };
        r.attach_tape(tape)?;
        Ok(r)
    }
    fn attach_tape(&mut self, on: bool) -> Result<(), String> {
        match on {
            true => Err("fast mode writes no random tape (use --mode exact)".into()),
            false => Ok(()),
        }
    }
    #[inline(always)]
    fn tape_buf(&mut self) -> Option<&mut String> {
        None
    }
    #[inline]
    fn plain(&self) -> bool {
        true
    }
    #[inline]
    fn count(&self) -> u64 {
        self.count
    }
    #[inline]
    fn set_origin(&mut self, _: Origin<'_>) -> Option<String> {
        None
    }
    #[inline]
    fn restore_origin(&mut self, _: Option<String>) {}
    #[inline]
    fn eval_begin(&mut self) {}
    #[inline]
    fn recorded_sample(&self) -> Option<Value> {
        None
    }
    #[inline]
    fn eval_end(&mut self, _: &str, _: Option<&Value>) {}
    #[inline]
    fn exhausted(&self) -> Option<&str> {
        None
    }
    fn take_problem(&mut self) -> Option<String> {
        None
    }
}

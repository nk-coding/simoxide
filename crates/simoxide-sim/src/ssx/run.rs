//! A pausable Slingshot-exact run (`DESIGN-SPD.md` 5.7, "ssx snapshots as built"):
//! [`SsxRun`] owns the engine and its event list; it runs until a [`Pause`] or a [`Watch`]
//! holds between two events, forks, detaches its state ([`SsxState`], `Send`) and takes model
//! changes between events ([`ChangeHost`]).
//!
//! The SPD layer's hooks ([`SpdHooks`]) are passed per call, so that a caller keeps them (and
//! their decision log) across pauses; a run with SPD passes the same layer to every call.

use super::*;
use crate::Status;
use crate::change::{ChangeError, ChangeHost, ChangeReport, ChangeSet};
use crate::sim::Pause;
use crate::sim::watch::{Watch, Watched};
use simoxide_model::raw::Graph;
use simoxide_sched::{Clock, F64Secs};
use std::sync::Arc;

/// When the engine stops between two events ([`Engine::advance`]).
pub(crate) trait SsxPause {
    /// Never pauses: the check compiles away (an uninterrupted run).
    const NEVER: bool = false;
    /// Pause before the next event, due at `next` (s)?
    fn pause(&mut self, e: &Engine<'_>, next: f64) -> bool;
}

/// Runs to the end.
pub(crate) struct NoPause;

impl SsxPause for NoPause {
    const NEVER: bool = true;
    #[inline(always)]
    fn pause(&mut self, _: &Engine<'_>, _: f64) -> bool {
        false
    }
}

/// The time of `ns` on Slingshot's double clock.
fn secs(ns: i64) -> f64 {
    F64Secs::secs(F64Secs::from_ns(ns))
}

impl SsxPause for Pause {
    #[inline]
    fn pause(&mut self, e: &Engine<'_>, next: f64) -> bool {
        match *self {
            Pause::After(t) => next > secs(t),
            Pause::Before(t) => next >= secs(t),
            Pause::Events(n) => e.events >= n,
            Pause::Never => false,
        }
    }
}

/// A predicate (`true`: go on).
struct While<G>(G);

impl<G: FnMut(&SsxView<'_>) -> bool> SsxPause for While<G> {
    fn pause(&mut self, e: &Engine<'_>, next: f64) -> bool {
        !(self.0)(&SsxView {
            e,
            next: Some(next),
        })
    }
}

/// A [`Watch`]: time conditions on the double clock, as [`Pause`] compares.
struct Watching<'w>(&'w mut Watch);

impl SsxPause for Watching<'_> {
    fn pause(&mut self, e: &Engine<'_>, next: f64) -> bool {
        self.0.check(e, |mut p| SsxPause::pause(&mut p, e, next))
    }
}

impl Watched for Engine<'_> {
    fn now_ns(&self) -> i64 {
        simoxide_sched::time::span(self.now)
    }
    fn events(&self) -> u64 {
        self.events
    }
    fn measurements(&self) -> &crate::Measurements {
        &self.meas
    }
    fn store_measurements(&self) -> bool {
        // the engine keeps every row (EDP2)
        true
    }
}

/// What [`SsxRun::run_while`]'s predicate sees between two events.
pub struct SsxView<'a> {
    e: &'a Engine<'a>,
    next: Option<f64>,
}

impl SsxView<'_> {
    /// Simulation time (s) of the last event.
    pub fn now(&self) -> f64 {
        self.e.now
    }
    /// Simulation time (ns) of the last event.
    pub fn now_ns(&self) -> i64 {
        simoxide_sched::time::span(self.e.now)
    }
    /// Time (s) of the next event.
    pub fn next_event(&self) -> Option<f64> {
        self.next
    }
    /// Events dispatched so far.
    pub fn events(&self) -> u64 {
        self.e.events
    }
    /// The measurements so far.
    pub fn measurements(&self) -> &crate::Measurements {
        &self.e.meas
    }
}

/// A Slingshot-exact run that can pause between two events, fork, detach and continue
/// (see the module documentation). [`super::run`] is `SsxRun::new(..)?.finish(None)`.
pub struct SsxRun<'m> {
    pub(super) e: Engine<'m>,
    pub(super) cfg: SimConfig,
    pub(super) out: Outputs,
    /// The error that ended the run (returned again by later calls).
    pub(super) error: Option<SimError>,
}

/// Writes `s` (and empties it) to `w`; without a writer the text is dropped.
fn drain(w: &mut Option<Box<dyn std::io::Write>>, s: &mut Option<String>) {
    let Some(s) = s.as_mut() else {
        return;
    };
    if let Some(w) = w.as_mut()
        && !s.is_empty()
    {
        let _ = w.write_all(s.as_bytes());
        let _ = w.flush();
    }
    s.clear();
}

impl<'m> SsxRun<'m> {
    /// A run of `cm` (compiled for [`FlavourKind::SlingshotExact`]) with `cfg` (see
    /// [`super::run`]), set up to t = 0: `ResourceSimulation`'s constructor has run (passive
    /// capacities drawn), the first events are queued. An error of the set-up is the run's
    /// error (the outputs get what was written before it).
    pub fn new(cm: &'m CompiledModel, cfg: SimConfig, out: Outputs) -> Result<Self, SimError> {
        if cm.flavour.kind != FlavourKind::SlingshotExact {
            return Err(SimError {
                message: format!(
                    "the Slingshot-exact engine needs a model compiled for slingshot-exact, not {}",
                    cm.flavour.kind
                ),
                at_ns: 0,
                kind: SimErrorKind::Model,
            });
        }
        if cfg.load_profile.is_some() {
            return Err(SimError {
                message: "the Slingshot-exact engine has no load profiles (LP-6): run the slingshot or simulizar flavour".into(),
                at_ns: 0,
                kind: SimErrorKind::Model,
            });
        }
        if cfg.usage_streams {
            return Err(SimError {
                message: "the Slingshot-exact engine has no usage streams (RND-11): run the slingshot or simulizar flavour in fast mode".into(),
                at_ns: 0,
                kind: SimErrorKind::Model,
            });
        }
        if cfg.sync_paths || cfg.call_counts {
            return Err(SimError {
                message: "the Slingshot-exact engine has no path monitoring (MEAS-13): run the slingshot or simulizar flavour".into(),
                at_ns: 0,
                kind: SimErrorKind::Model,
            });
        }
        if cfg.max_sim_time <= 0 {
            return Err(SimError {
                message: "the Slingshot-exact mode needs a simulation time (Slingshot's simTime is its only stop condition, SS-RUN-5)".into(),
                at_ns: 0,
                kind: SimErrorKind::Model,
            });
        }
        let e = Engine::new(cm, &cfg, &out)?;
        let mut r = SsxRun {
            e,
            cfg,
            out,
            error: None,
        };
        if let Err(err) = r.e.start(&r.cfg) {
            r.e.done = true;
            r.flush();
            return Err(err);
        }
        Ok(r)
    }

    /// Writes the projection and tape lines so far to the outputs.
    pub(super) fn flush(&mut self) {
        drain(&mut self.out.trace, &mut self.e.proj);
        drain(&mut self.out.tape, &mut self.e.rng.tape);
    }

    fn go<P: SsxPause>(
        &mut self,
        hooks: Option<&mut dyn SpdHooks>,
        p: &mut P,
    ) -> Result<Status, SimError> {
        if let Some(e) = &self.error {
            return Err(e.clone());
        }
        let r = self.e.advance(&self.cfg, hooks, p);
        if let Err(e) = &r {
            self.error = Some(e.clone());
        }
        self.flush();
        r
    }

    /// Runs until `p` holds between two events (checked before each event, also the first)
    /// or the run is over. `hooks`: the run's SPD layer, if it has one (the same on every
    /// call).
    pub fn run_until(
        &mut self,
        mut p: Pause,
        hooks: Option<&mut dyn SpdHooks>,
    ) -> Result<Status, SimError> {
        self.go(hooks, &mut p)
    }

    /// Runs while `keep_going` returns true, asked before each event, or until the run is
    /// over.
    pub fn run_while(
        &mut self,
        hooks: Option<&mut dyn SpdHooks>,
        keep_going: impl FnMut(&SsxView<'_>) -> bool,
    ) -> Result<Status, SimError> {
        self.go(hooks, &mut While(keep_going))
    }

    /// Runs until a condition of `watch` holds (N2's pause conditions; time conditions on
    /// Slingshot's double clock) or the run is over; arms the watch first if it is not armed.
    /// A watched pause equals [`Pause::Events`] at the same count.
    pub fn run_watch(
        &mut self,
        watch: &mut Watch,
        hooks: Option<&mut dyn SpdHooks>,
    ) -> Result<Status, SimError> {
        if !watch.armed {
            watch
                .arm_on(&self.e, self.model_graph())
                .map_err(|message| SimError {
                    message,
                    at_ns: self.now_ns(),
                    kind: SimErrorKind::Model,
                })?;
        }
        if watch.fired.is_some() {
            return Ok(if self.e.done {
                Status::Finished
            } else {
                Status::Paused
            });
        }
        self.go(hooks, &mut Watching(watch))
    }

    /// Runs to the end and returns the result (the outputs get the rest).
    /// Registers interpreter listener `l` (SIM-14; `None`: none): before the first event to see
    /// every callback; forks and restored runs have none until one is set.
    pub fn set_listener(&mut self, l: Option<Box<dyn crate::listener::Listener>>) {
        self.e.lis = Default::default();
        self.e.lis.l = l;
    }

    /// [`SsxRun::set_listener`] as a builder.
    pub fn with_listener(mut self, l: Box<dyn crate::listener::Listener>) -> Self {
        self.set_listener(Some(l));
        self
    }

    /// Takes the listener back.
    pub fn take_listener(&mut self) -> Option<Box<dyn crate::listener::Listener>> {
        let l = self.e.lis.l.take();
        self.e.lis = Default::default();
        l
    }

    pub fn finish(mut self, hooks: Option<&mut dyn SpdHooks>) -> Result<RunResult, SimError> {
        self.go(hooks, &mut NoPause)?;
        Ok(self.result())
    }

    /// The result of a finished run.
    fn result(&mut self) -> RunResult {
        RunResult {
            measurements: std::mem::take(&mut self.e.meas),
            uniforms: self.e.rng.stoex,
            events: self.e.events,
            end_ns: simoxide_sched::time::span(self.cfg.max_sim_time as f64),
            main_count: self.e.finished,
            warnings: std::mem::take(&mut self.e.errors),
        }
    }

    /// Simulation time (ns) of the last event.
    pub fn now_ns(&self) -> i64 {
        simoxide_sched::time::span(self.e.now)
    }

    /// Simulation time (s) of the last event.
    pub fn now(&self) -> f64 {
        self.e.now
    }

    /// Events dispatched so far.
    pub fn events(&self) -> u64 {
        self.e.events
    }

    /// The run is over (finished or failed).
    pub fn is_over(&self) -> bool {
        self.e.done
    }

    /// The measurements so far.
    pub fn measurements(&self) -> &crate::Measurements {
        &self.e.meas
    }

    /// Finished usage-scenario runs so far.
    pub fn main_count(&self) -> i64 {
        self.e.finished
    }

    /// The handler exceptions so far ([`RunResult::warnings`]).
    pub fn warnings(&self) -> &[String] {
        &self.e.errors
    }

    /// The run's configuration.
    pub fn config(&self) -> &SimConfig {
        &self.cfg
    }

    /// The compiled model the run was created on.
    pub fn base(&self) -> &'m CompiledModel {
        self.e.base
    }

    /// The compiled model the run simulates now (after model changes its own).
    pub fn compiled(&self) -> &CompiledModel {
        self.e.cm
    }

    /// The run's model graph (with a QVTo host's state, see [`ChangeHost::set_model_graph`]).
    pub fn model_graph(&self) -> &Graph {
        ChangeHost::model_graph(&self.e)
    }

    /// The model's generation (0: the base model; one more per change set).
    pub fn generation(&self) -> u32 {
        self.e.own.generation
    }

    /// The run is written to a trace (the level-E projection).
    pub fn traced(&self) -> bool {
        self.e.proj.is_some()
    }

    /// Forks the run: a new run that continues from the current state with the outputs `out`
    /// (which get the fork's continuation only); this one goes on unchanged. The engine's
    /// objects are copied through [`SsxState`] (a deep copy that keeps their sharing).
    pub fn fork(&mut self, out: Outputs) -> Result<SsxRun<'m>, SimError> {
        let st = self.detach();
        SsxRun::attach(self.e.base, st, out)
    }

    /// The run's state as a value that can move to another thread ([`SsxRun::attach`]
    /// continues it), outputs flushed first. This run goes on unchanged.
    pub fn detach(&mut self) -> SsxState {
        self.flush();
        SsxState::of(self)
    }

    /// Continues `state` on `base` (the compiled model the run was created on) with the outputs
    /// `out`, which receive the continuation only. A traced state continues with a trace
    /// ([`Outputs::trace`] `None` discards it); a state without one cannot write one. A tape can
    /// be attached or left out at any pause.
    pub fn attach(
        base: &'m CompiledModel,
        state: SsxState,
        out: Outputs,
    ) -> Result<SsxRun<'m>, SimError> {
        state.into_run(base, out)
    }
}

impl ChangeHost for SsxRun<'_> {
    fn now_s(&self) -> f64 {
        self.e.now
    }

    fn now_ns(&self) -> i64 {
        simoxide_sched::time::span(self.e.now)
    }

    fn base_compiled(&self) -> &CompiledModel {
        self.e.base
    }

    fn compiled_now(&self) -> &CompiledModel {
        self.e.cm
    }

    fn model_generation(&self) -> u32 {
        self.e.own.generation
    }

    fn model_graph(&self) -> &Graph {
        ChangeHost::model_graph(&self.e)
    }

    fn set_model_graph(&mut self, graph: Arc<Graph>) -> Result<(), ChangeError> {
        self.e.set_model_graph(graph)
    }

    /// A change set between two events (e.g. at a resume): the engine's model change as inside
    /// an SPD adjustment, then what `ModelAdjusted`'s subscribers do for it (active resources
    /// of new containers, the deleted containers' resources leave the table, passive resources
    /// of new allocation contexts, calculators of new monitors), without the event.
    fn apply_changes(
        &mut self,
        graph: Graph,
        changes: &ChangeSet,
    ) -> Result<ChangeReport, ChangeError> {
        if self.e.done {
            return Err(ChangeError::State("the run has ended".into()));
        }
        let before = self.e.cm;
        let r = self.e.apply_changes(graph, changes)?;
        if !std::ptr::eq(before, self.e.cm) {
            self.e
                .sync_after_change(before)
                .map_err(|t| ChangeError::Invalid(vec![format!("{}: {}", t.class, t.message)]))?;
        }
        Ok(r)
    }

    fn fail_run(&mut self, message: String) -> SimError {
        self.e.fail_run(message)
    }
}

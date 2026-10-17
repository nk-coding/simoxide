//! The mutable state of one run ([`RunState`]) and the API that pauses, forks and resumes a run
//! (`DESIGN-SPD.md` 5.7).

use super::*;

/// Where a run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Created; the initialisation (workload drivers, initial tuples, sliding windows) runs with
    /// the first [`Simulation::run_until`] or [`Simulation::finish`].
    NotStarted,
    /// Between two events.
    Running,
    /// A stop condition held (or no event was left) and the post-stop drain has run
    /// ([`Simulation::finish`] returns the result).
    Finished,
    /// The run aborted with an error ([`Simulation::finish`] returns it).
    Failed,
}

/// When [`Simulation::run_until`] returns control. Checked between events only, before each
/// event (also before the first one); a run that reaches a stop condition first finishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pause {
    /// Before the first event later than this time (ns): every event at or before it has run.
    After(i64),
    /// Before the first event at or after this time (ns).
    Before(i64),
    /// As soon as at least this many events were processed ([`RunResult::events`]). A
    /// process hand-off that runs inside the event that caused it counts as an event of its
    /// own (see `Simulation::handoff`), so the count at the pause can exceed the bound by one.
    Events(u64),
    /// Never: run to the end.
    Never,
}

/// What [`Simulation::run_until`] and [`Simulation::run_while`] reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Paused between two events; the run can continue.
    Paused,
    /// The run is over ([`Phase::Finished`]); [`Simulation::finish`] returns the result.
    Finished,
}

/// The sizes of the compiled tables a state indexes into: a state continues only on a
/// [`CompiledModel`] of the same shape (in practice, the one it was created on).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Shape([usize; 10]);

impl Shape {
    pub(crate) fn of(cm: &CompiledModel) -> Shape {
        Shape([
            cm.resources.len(),
            cm.series.len(),
            cm.scenarios.len(),
            cm.windows.len(),
            cm.progs.len(),
            cm.uact.len(),
            cm.act.len(),
            cm.ac_paths.len(),
            cm.code.traced.sops.len(),
            cm.code.plain.sops.len(),
        ])
    }
}

/// All mutable state of one simulation run: clock and event list, processes with their
/// continuation stacks and frames, resource queues and processor-sharing tables, passive
/// resources, the random stream and tape position, measurements, sliding windows, PRM
/// recorders, the reconfiguration process, resolution caches, limits and counters.
///
/// A [`Simulation`] is the engine: the immutable [`CompiledModel`] it borrows, the output
/// writers (trace, random tape) and one `RunState`. So
///
/// * **fork** = clone the state ([`Simulation::fork`], [`Simulation::snapshot`]). Both copies
///   continue exactly like the uninterrupted run: the same trace lines, tape records,
///   measurements and event count, in exact and in fast mode;
/// * **pause** = return between two events ([`Simulation::run_until`],
///   [`Simulation::run_while`]) and continue later, from the same `Simulation` or from a state
///   taken out of it ([`Simulation::into_state`], [`Simulation::resume`]).
///
/// Outputs are not state: a state taken out of a simulation has no writers, and the writers
/// given to [`Simulation::resume`] or [`Simulation::fork`] receive the continuation only. The
/// writers of a paused simulation are flushed whenever a state is taken, so the output written
/// up to the pause plus the continuation of a fork is byte for byte the output of the
/// uninterrupted run.
///
/// A clone shares the parameter frames with its origin (`Rc`, copy on write: the interpreter
/// only writes into unshared frames) and copies everything else, so a fork costs
/// O(processes + jobs + stored measurement rows). Because of the `Rc`s a `RunState` stays on
/// its thread; [`RunState::detach`] makes a deep copy that shares no frame ([`Detached`],
/// `Send`) to continue on another thread. (`Arc` frames were measured: 7 % slower on the
/// benchmark suite, 13 % on MediaStore.)
///
/// ```no_run
/// use simoxide_sim::{CompiledModel, Outputs, Pause, SimConfig, Simulation};
/// let cm = CompiledModel::compile(simoxide_model::load_dir("corpus/h01_ps_single").unwrap()).unwrap();
/// let cfg = SimConfig { max_measurements: 1000, ..Default::default() };
/// let mut sim = Simulation::new(&cm, cfg, Outputs::default()).unwrap();
/// sim.run_until(Pause::After(5_000_000_000)).unwrap(); // every event up to t = 5 s
/// let fork = sim.fork(Outputs::default()).unwrap();
/// let (a, b) = (sim.finish().unwrap(), fork.finish().unwrap());
/// assert_eq!(a.measurements.to_csv(), b.measurements.to_csv());
/// ```
pub struct RunState<C: Compat = Exact> {
    /// The run configuration (stop conditions, seed, limits).
    pub(super) cfg: SimConfig,
    pub(super) phase: Phase,
    /// The error of a failed run.
    pub(super) failure: Option<SimError>,
    /// The instruction streams with the trace instructions: the program counters of the
    /// processes point into this variant (`CompiledModel::code`).
    pub(super) traced: bool,
    pub(super) shape: Shape,
    // ------------------------------------------------------------------ clock and events
    pub(super) now: i64,
    /// The experiment has not stopped yet (`false` during the post-stop drain).
    pub(super) running: bool,
    pub(super) n_events: u64,
    /// Livelock guard: consecutive events at `now`, and `main_count` when last reset.
    pub(super) same_instant: u64,
    pub(super) count_seen: i64,
    /// `mainMeasurementsCount` (SIM-6.4).
    pub(super) main_count: i64,
    // ------------------------------------------------------------------ processes
    pub(super) procs: Vec<Proc>,
    pub(super) free: Vec<u32>,
    pub(super) table_seq: u64,
    /// The shared empty frame (parameterless calls, fresh result frames).
    pub(super) empty: FrameRef,
    /// Synchronous children of pending fork joins (slots reused through `free_joins`).
    pub(super) joins: Vec<Vec<u64>>,
    pub(super) free_joins: Vec<u32>,
    // ------------------------------------------------------------------ resources
    pub(super) res: Vec<ResRt>,
    pub(super) passives: Vec<PassiveInst>,
    /// Basic component instances by path id.
    pub(super) comps: Vec<Option<CompInst>>,
    /// Paths missing from the compiled trie (created on first use).
    pub(super) dyn_paths: Vec<AcPath>,
    pub(super) dyn_path_child: FxMap<(u32, AcRef), u32>,
    // ------------------------------------------------------------------ random numbers
    pub(super) rng: C::Rng,
    // ------------------------------------------------------------------ measurements
    /// Recorded tuples, sliding windows, PRM recorders, reconfiguration process, event list.
    pub(super) meas: Sink,
    /// Series created during the run (lazily created passive-resource calculators), by
    /// measuring-point key (see `CompiledModel::series_by_mp`).
    pub(super) run_series: HashMap<Box<str>, Vec<(&'static str, SeriesId)>>,
    pub(super) warnings: Vec<String>,
    // ------------------------------------------------------------------ resolution caches
    /// Provided-role calls, by id (see `Simulation::prov_id`).
    pub(super) provs: Vec<ProvRes>,
    pub(super) prov_ids: FxMap<(AcRef, RoleId, SignatureId), u32>,
    /// Per external-call site ([`crate::code::SOp::Call`]): its last resolution.
    pub(super) call_cache: Vec<CallCache>,
    /// Per user action: the provided-role call of the entry-level system call (`u32::MAX`:
    /// not resolved yet).
    pub(super) elsc_prov: Vec<u32>,
    // ------------------------------------------------------------------ limits
    /// Interpreter steps so far, and the step / event count of the next [`Limits`] check.
    pub(super) steps: u64,
    pub(super) next_step_check: u64,
    pub(super) next_event_check: u64,
    /// `Limits::max_stack_depth` (`usize::MAX` if unlimited).
    pub(super) max_depth: usize,
    // ------------------------------------------------------------------ within an event
    // (zero between events; a fork never sees other values)
    /// Nesting of `run_process` (processes run synchronously after the stop).
    pub(super) nesting: u32,
    /// Nesting of direct continuation-handler calls (`Simulation::enter`).
    pub(super) direct: u32,
    // ------------------------------------------------------------------ allocation reuse
    // (no influence on results; a clone starts with empty ones)
    pub(super) fork_scratch: Vec<u64>,
    /// Recycled frames (see [`FramePool`]).
    pub(super) pool: FramePool,
    /// Empty stacks that stand in for a running process's (see `run_process`); boxed because
    /// they are swapped with `Proc::data`.
    #[allow(clippy::vec_box)]
    pub(super) spare_data: Vec<Box<ProcData>>,
}

impl<C: Compat> Clone for RunState<C> {
    /// A fork: an independent copy that continues exactly like this state. Frames are shared
    /// (copy on write); the allocation-reuse pools are not copied.
    fn clone(&self) -> Self {
        RunState {
            cfg: self.cfg.clone(),
            phase: self.phase,
            failure: self.failure.clone(),
            traced: self.traced,
            shape: self.shape,
            now: self.now,
            running: self.running,
            n_events: self.n_events,
            same_instant: self.same_instant,
            count_seen: self.count_seen,
            main_count: self.main_count,
            procs: self.procs.clone(),
            free: self.free.clone(),
            table_seq: self.table_seq,
            empty: self.empty.clone(),
            joins: self.joins.clone(),
            free_joins: self.free_joins.clone(),
            res: self.res.clone(),
            passives: self.passives.clone(),
            comps: self.comps.clone(),
            dyn_paths: self.dyn_paths.clone(),
            dyn_path_child: self.dyn_path_child.clone(),
            rng: self.rng.clone(),
            meas: self.meas.clone(),
            run_series: self.run_series.clone(),
            warnings: self.warnings.clone(),
            provs: self.provs.clone(),
            prov_ids: self.prov_ids.clone(),
            call_cache: self.call_cache.clone(),
            elsc_prov: self.elsc_prov.clone(),
            steps: self.steps,
            next_step_check: self.next_step_check,
            next_event_check: self.next_event_check,
            max_depth: self.max_depth,
            nesting: self.nesting,
            direct: self.direct,
            fork_scratch: Vec::new(),
            pool: FramePool::default(),
            spare_data: Vec::new(),
        }
    }
}

impl<C: Compat> RunState<C> {
    /// A new state for `cm` (the run has not started).
    pub(crate) fn new(cm: &CompiledModel, cfg: SimConfig, traced: bool, rng: C::Rng) -> Self {
        let ps = cfg.ps_algorithm;
        let res = cm
            .resources
            .iter()
            .map(|r| ResRt {
                sched: match r.policy {
                    SchedulingPolicy::Delay => None,
                    SchedulingPolicy::ProcessorSharing => {
                        Some(ActiveResource::new(r.policy, r.instances, ps))
                    }
                    SchedulingPolicy::Fcfs => Some(ActiveResource::new(r.policy, 1, ps)),
                },
                delay_set: Vec::new(),
                shadow: vec![0; r.instances.max(1) as usize],
            })
            .collect();
        let meas = Sink {
            m: Measurements {
                series: cm.series.clone(),
                rows: vec![Vec::new(); cm.series.len()],
                count: 0,
            },
            windows: Vec::new(),
            series_windows: Vec::new(),
            next_pid: 0,
            cur_pid: 0,
            reconf_pid: 0,
            error: None,
            prm_armed: false,
            prm: Prm::default(),
            reconf: Reconf {
                time_series: cm.reconf_time_series.clone(),
                store: cfg.store_measurements,
                ..Default::default()
            },
            events: EventList::default(),
        };
        let max_depth = max_depth(&cfg.limits);
        RunState {
            cfg,
            phase: Phase::NotStarted,
            failure: None,
            traced,
            shape: Shape::of(cm),
            now: 0,
            running: true,
            n_events: 0,
            same_instant: 0,
            count_seen: 0,
            main_count: 0,
            procs: Vec::new(),
            free: Vec::new(),
            table_seq: 0,
            empty: FrameRef::new(Frame::default()),
            joins: Vec::new(),
            free_joins: Vec::new(),
            res,
            passives: Vec::new(),
            comps: Vec::new(),
            dyn_paths: Vec::new(),
            dyn_path_child: FxMap::default(),
            rng,
            meas,
            run_series: HashMap::new(),
            warnings: Vec::new(),
            provs: Vec::new(),
            prov_ids: FxMap::default(),
            call_cache: vec![CallCache::default(); cm.code.traced.call_sites as usize],
            elsc_prov: vec![u32::MAX; cm.uact.len()],
            steps: 0,
            next_step_check: 0,
            next_event_check: 0,
            max_depth,
            nesting: 0,
            direct: 0,
            fork_scratch: Vec::new(),
            pool: FramePool::default(),
            spare_data: Vec::new(),
        }
    }

    /// Where the run is.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The run configuration.
    pub fn config(&self) -> &SimConfig {
        &self.cfg
    }

    /// Current simulation time (ns).
    pub fn now_ns(&self) -> i64 {
        self.now
    }

    /// Time of the next pending event (ns), if any.
    pub fn next_event_ns(&self) -> Option<i64> {
        self.meas.events.next_time()
    }

    /// Events processed so far ([`RunResult::events`]).
    pub fn events(&self) -> u64 {
        self.n_events
    }

    /// Finished usage-scenario runs so far ([`RunResult::main_count`]).
    pub fn main_count(&self) -> i64 {
        self.main_count
    }

    /// Uniforms (exact mode) or samples (fast mode) drawn so far ([`RunResult::uniforms`]).
    pub fn uniforms(&self) -> u64 {
        self.rng.count()
    }

    /// The measurements recorded so far.
    pub fn measurements(&self) -> &Measurements {
        &self.meas.m
    }

    /// Live processes (users, workload drivers, forked behaviours).
    pub fn live_processes(&self) -> usize {
        self.procs.len() - self.free.len()
    }

    /// Whether the state was created with a trace (see [`Simulation::resume`]).
    pub fn traced(&self) -> bool {
        self.traced
    }
}

impl<C: Compat> RunState<C> {
    /// A deep copy that shares no frame with this state (frames shared within the state stay
    /// shared in the copy), to continue on another thread ([`Detached::into_state`]).
    pub fn detach(&self) -> Detached<C> {
        let mut st = self.clone();
        let mut c = crate::frames::FrameCopier::default();
        st.empty = c.frame(&st.empty);
        for p in &mut st.procs {
            c.all(&mut p.data.frames);
            c.all(&mut p.data.results);
        }
        Detached(st)
    }
}

/// A [`RunState`] that shares no frame with any other value ([`RunState::detach`]), so it can
/// be sent to another thread.
pub struct Detached<C: Compat = Exact>(RunState<C>);

// SAFETY: a `RunState` is not `Send` only because of the `Rc`s of its frames and proxies (every
// other field is `Send`: the random source by the `SimRng` bound, the rest plain data and
// `Arc`s). A `Detached` is only made by `RunState::detach`, which replaces every frame
// reference of the state by a fresh copy: the frame pool and the spare stacks of a clone are
// empty, and the frames are reachable only through `empty` and the processes' frame and
// result stacks (and from there through parents and proxies), which are all copied. So every
// reference count of the wrapped frames is owned by the wrapped state alone, and moving it to
// another thread as a whole cannot race with anything. `tests/fork.rs` checks the threads.
unsafe impl<C: Compat> Send for Detached<C> {}

impl<C: Compat> Detached<C> {
    /// The state, on this thread.
    pub fn into_state(self) -> RunState<C> {
        self.0
    }
}

/// `Limits::max_stack_depth` as a bound (`usize::MAX` if unlimited).
pub(crate) fn max_depth(l: &Limits) -> usize {
    match l.max_stack_depth {
        0 => usize::MAX,
        n => n as usize,
    }
}

/// A pause condition of [`Simulation::advance`], checked before every event.
pub(crate) trait PauseWhen<C: Compat> {
    /// Never pauses: the check compiles away ([`Simulation::finish`]).
    const NEVER: bool = false;
    /// Pause before the event at `next` (ns)?
    fn pause(&mut self, st: &RunState<C>, next: i64) -> bool;
}

impl<C: Compat> PauseWhen<C> for Pause {
    const NEVER: bool = false;
    #[inline]
    fn pause(&mut self, st: &RunState<C>, next: i64) -> bool {
        match *self {
            Pause::After(t) => next > t,
            Pause::Before(t) => next >= t,
            Pause::Events(n) => st.n_events >= n,
            Pause::Never => false,
        }
    }
}

/// [`PauseWhen`] that never pauses.
pub(crate) struct NoPause;

impl<C: Compat> PauseWhen<C> for NoPause {
    const NEVER: bool = true;
    #[inline(always)]
    fn pause(&mut self, _: &RunState<C>, _: i64) -> bool {
        false
    }
}

/// [`PauseWhen`] from a predicate (`true`: go on).
pub(crate) struct While<F>(pub F);

impl<C: Compat, F: FnMut(&RunState<C>) -> bool> PauseWhen<C> for While<F> {
    #[inline]
    fn pause(&mut self, st: &RunState<C>, _: i64) -> bool {
        !(self.0)(st)
    }
}

/// Discards a trace: a traced state continued without a trace output (see
/// [`Simulation::resume`]).
struct Discard;

impl Write for Discard {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'m, C: Compat> Simulation<'m, C> {
    /// Continues `state` on `cm` (the model it was created on) with the outputs `out`, which
    /// receive the continuation only.
    ///
    /// The process positions of a state point into one of the two instruction-stream variants:
    /// a state created with a trace continues with one ([`Outputs::trace`] `None` discards it),
    /// a state created without a trace cannot write one. A tape can be attached or left out at
    /// any pause (exact mode only).
    pub fn resume(
        cm: &'m CompiledModel,
        mut state: RunState<C>,
        out: Outputs,
    ) -> Result<Self, SimError> {
        let fail = |message: String| SimError {
            message,
            at_ns: state.now,
            kind: SimErrorKind::Model,
        };
        if state.shape != Shape::of(cm) {
            return Err(fail(
                "the run state belongs to another compiled model".to_string(),
            ));
        }
        let trace = match (out.trace, state.traced) {
            (Some(w), true) => Some(w),
            (None, true) => Some(Box::new(Discard) as Box<dyn Write>),
            (None, false) => None,
            (Some(_), false) => {
                return Err(fail(
                    "the run state was created without a trace: its continuation cannot write one"
                        .to_string(),
                ));
            }
        };
        state.rng.attach_tape(out.tape.is_some()).map_err(fail)?;
        Ok(Simulation {
            cm,
            code: cm.code.get(state.traced),
            trace: trace.map(TraceOut::new),
            tape: out.tape,
            st: state,
        })
    }

    /// The state (read only).
    pub fn state(&self) -> &RunState<C> {
        &self.st
    }

    /// A copy of the current state (a fork), to continue with [`Simulation::resume`]. Flushes
    /// the outputs first, so that what they hold is exactly the output up to this point.
    pub fn snapshot(&mut self) -> RunState<C> {
        self.flush_outputs();
        let mut st = self.st.clone();
        // the copy has no outputs
        let _ = st.rng.attach_tape(false);
        st
    }

    /// Forks the run: a new simulation that continues from the current state with the outputs
    /// `out` (see [`Simulation::resume`]); this one goes on unchanged.
    pub fn fork(&mut self, out: Outputs) -> Result<Simulation<'m, C>, SimError> {
        let st = self.snapshot();
        Simulation::resume(self.cm, st, out)
    }

    /// Ends this simulation and returns its state (outputs flushed and detached).
    pub fn into_state(mut self) -> RunState<C> {
        self.flush_outputs();
        let _ = self.st.rng.attach_tape(false);
        self.st
    }

    /// Replaces the [`Limits`] of the run (e.g. a fork's own deadline or cancellation flag:
    /// a fork shares the flag of its origin otherwise). Checked from the next event or step on.
    pub fn set_limits(&mut self, limits: Limits) {
        self.st.max_depth = max_depth(&limits);
        self.st.cfg.limits = limits;
        self.st.next_step_check = self.st.steps;
        self.st.next_event_check = self.st.n_events;
    }

    /// Runs until `p` holds between two events or the run is over.
    pub fn run_until(&mut self, mut p: Pause) -> Result<Status, SimError> {
        let r = self.advance(&mut p);
        self.settle(r)
    }

    /// Runs while `keep_going` returns true, asked between events (before each event,
    /// including the first), or until the run is over.
    pub fn run_while(
        &mut self,
        keep_going: impl FnMut(&RunState<C>) -> bool,
    ) -> Result<Status, SimError> {
        let r = self.advance(&mut While(keep_going));
        self.settle(r)
    }

    /// The outcome of an [`Simulation::advance`]: an error is stored in the state (the run is
    /// over) and the outputs are flushed.
    fn settle(&mut self, r: R<Status>) -> Result<Status, SimError> {
        match r {
            Ok(s) => Ok(s),
            Err(e) => {
                self.flush_outputs();
                Err(*e)
            }
        }
    }
}

//! The event core and the interpreter.
//!
//! * **Event list:** a binary heap keyed by `(time_ns, seq)`; `seq` is a global insertion counter,
//!   so same-time events run in insertion (FIFO) order as in DESMO-J's `EventTreeList` (SIM-3).
//! * **Processes** (users, the open-workload generator, forked behaviours) are explicit state
//!   machines: a stack of continuations ([`Cont`]) plus the variable-frame, result-frame and
//!   assembly-context stacks of the reference's interpreter context. A `Resume` event continues a
//!   process from whatever wait point it is at (SIM-4.4a), exactly like the reference's coroutine
//!   hand-off.
//! * **Waits** are the reference's suspension points: `hold` (two events: delay end, then
//!   resume), active-resource demands (scheduler completion, then resume), passive-resource
//!   acquires and fork joins.
//! * **Stop** conditions are checked after every event (SIM-6.2); then the resources are
//!   deactivated (final state tuples) and the remaining processes are drained synchronously in
//!   resource-table order (SIM-6.5, patch P3).

use crate::compat::{Compat, Exact, Mode, SimRng};
use crate::events::{Ev, EventList};
use crate::frames::{Binding, Frame, FrameEnv, FramePool, Proxy, eval_proxy, frame_mut};
use crate::fxhash::FxMap;
use crate::ir::*;
use crate::meas::Measurements;
use crate::rng::{Origin, RngMode};
use crate::trace::TraceOut;
use crate::windows::Window;
use simoxide_model::*;
use simoxide_random::UniformSource;
use simoxide_sched::time::{checked_span, seconds};
use simoxide_sched::{
    ActiveResource, PassiveListener, PassiveResource, PsAlgorithm, ResourceListener,
    SchedulingPolicy,
};
use simoxide_stoex::Value;
use std::collections::HashMap;
use std::fmt;
use std::io::Write;
use std::rc::Rc;

mod calls;
mod flat;

/// Run configuration (the reference's `run.json` plus port options).
#[derive(Clone, Debug)]
pub struct SimConfig {
    /// Name written into the trace header.
    pub run_name: String,
    pub seed: i64,
    /// Whole seconds; `<= 0` disables.
    pub max_sim_time: i64,
    /// `<= 0` disables.
    pub max_measurements: i64,
    pub simulate_linking_resources: bool,
    pub simulate_throughput_of_linking_resources: bool,
    pub rng: RngMode,
    /// In replay mode, compare the origin tag of every uniform with the tape (debug aid).
    pub check_tape_origins: bool,
    /// In replay mode, use the tape's recorded evaluation results (`s` records) instead of the
    /// locally sampled values (isolates the simulator from distribution sampling).
    pub replay_samples: bool,
    /// Keep the measurement rows (else only count them).
    pub store_measurements: bool,
    pub ps_algorithm: PsAlgorithm,
    /// Livelock guard (REF-7, `docs/correctness/deviations.md`): abort after this many consecutive events at
    /// one simulation time without progress towards a stop condition (with `max_measurements`
    /// enabled, a finished usage-scenario run is progress). `0` disables the guard; the
    /// reference then loops forever, like a model that never advances time.
    pub max_events_per_instant: u64,
    /// Resource limits for untrusted or generated models (see [`Limits`]).
    pub limits: Limits,
    /// Exact (default) or fast mode ([`crate::compat`]). [`crate::run`] and [`crate::run_batch`]
    /// dispatch on it; [`Simulation::create`] checks it against its policy type.
    pub mode: Mode,
}

/// Resource limits of one run. A run that exceeds one stops with a [`SimError`] of kind
/// [`SimErrorKind::Limit`] (or [`SimErrorKind::Cancelled`]); the limits never change the result
/// of a run that stays within them. The defaults only bound what the reference cannot run either
/// (it deadlocks at a call depth of 300, `docs/correctness/reference-bugs.md` REF-14, and needs one Java thread per
/// process); the other limits are off by default. See `docs/correctness/deviations.md`.
#[derive(Clone, Debug)]
pub struct Limits {
    /// Maximum depth of the continuation stack of one process (nested calls, behaviours, loops;
    /// about 5 entries per nested call), checked when a component's SEFF is entered. `0` =
    /// unlimited. Default 10 000.
    pub max_stack_depth: u32,
    /// Maximum number of simultaneously live processes (users, forked behaviours). `0` =
    /// unlimited. Default 1 000 000.
    pub max_processes: u32,
    /// Maximum number of processed events. `0` = unlimited (default).
    pub max_events: u64,
    /// Maximum number of interpreter steps, a deterministic work budget that also bounds work
    /// that does not advance time (e.g. a loop with 10^9 iterations and no demands). `0` =
    /// unlimited (default). What counts as a step is an implementation detail (continuation
    /// handler invocations); the count of a run can change between versions.
    pub max_steps: u64,
    /// Wall-clock deadline, checked every few thousand steps and events.
    pub deadline: Option<std::time::Instant>,
    /// Cooperative cancellation: set the flag to `true` to stop the run (checked like
    /// `deadline`).
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_stack_depth: 10_000,
            max_processes: 1_000_000,
            max_events: 0,
            max_steps: 0,
            deadline: None,
            cancel: None,
        }
    }
}

impl Limits {
    /// No limits at all (the livelock guard is separate: [`SimConfig::max_events_per_instant`]).
    pub fn unlimited() -> Self {
        Limits {
            max_stack_depth: 0,
            max_processes: 0,
            ..Limits::default()
        }
    }
}

/// How often (in steps and in events) the deadline and the cancellation flag are checked.
const CHECK_INTERVAL: u64 = 4096;

/// Maximum nesting of processes that run synchronously inside each other (after the stop, a
/// fork runs its children synchronously): bounds the native stack.
const MAX_NESTING: u32 = 200;

/// Default of [`SimConfig::max_events_per_instant`]: far above what a terminating model does at
/// one instant, low enough to stop a zero-time livelock within seconds and about 1 GB.
pub const DEFAULT_MAX_EVENTS_PER_INSTANT: u64 = 20_000_000;

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig {
            run_name: String::new(),
            seed: 0,
            max_sim_time: -1,
            max_measurements: -1,
            simulate_linking_resources: false,
            simulate_throughput_of_linking_resources: true,
            rng: RngMode::Own,
            check_tape_origins: false,
            replay_samples: true,
            store_measurements: true,
            ps_algorithm: PsAlgorithm::Exact,
            max_events_per_instant: DEFAULT_MAX_EVENTS_PER_INSTANT,
            limits: Limits::default(),
            mode: Mode::Exact,
        }
    }
}

/// Where the optional trace and tape go.
#[derive(Default)]
pub struct Outputs {
    pub trace: Option<Box<dyn Write>>,
    pub tape: Option<Box<dyn Write>>,
}

/// Error that aborts a run (the reference sets status ERROR and stops).
#[derive(Debug, Clone)]
pub struct SimError {
    pub message: String,
    /// Simulation time of the error (ns).
    pub at_ns: i64,
    pub kind: SimErrorKind,
}

/// Why a run stopped with an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimErrorKind {
    /// The model or the run configuration: the reference aborts too (at the same event, for the
    /// errors reproduced from it), or the model is outside what the port supports.
    Model,
    /// A resource limit of [`Limits`] or the livelock guard
    /// ([`SimConfig::max_events_per_instant`]).
    Limit,
    /// [`Limits::cancel`] was set or [`Limits::deadline`] passed.
    Cancelled,
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at t={}: {}", seconds(self.at_ns), self.message)
    }
}
impl std::error::Error for SimError {}

/// Result of a completed run.
#[derive(Debug, Clone)]
pub struct RunResult {
    pub measurements: Measurements,
    pub uniforms: u64,
    /// Processed event notes.
    pub events: u64,
    /// Final simulation time (ns).
    pub end_ns: i64,
    /// `mainMeasurementsCount` (finished usage-scenario runs, SIM-6.4).
    pub main_count: i64,
    /// Non-fatal problems (e.g. tape origin mismatches).
    pub warnings: Vec<String>,
}

// ------------------------------------------------------------------------------------------
// processes

#[inline]
fn key(slot: u32, generation: u32) -> u64 {
    (u64::from(slot) << 32) | u64::from(generation)
}
#[inline]
fn unkey(k: u64) -> (u32, u32) {
    ((k >> 32) as u32, k as u32)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PState {
    Suspended,
    Running,
    Terminated,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LastRes {
    None,
    Think,
    Active(ResIdx),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PKind {
    OpenGen,
    OpenUser,
    ClosedUser,
    Forked { sync: bool, parent: u64 },
}

/// Interpreter continuation (one level of the reference's recursive interpreter).
#[derive(Clone, Copy, Debug)]
enum Cont {
    OpenGen {
        scen: u32,
        st: u8,
    },
    OpenUser {
        scen: u32,
        st: u8,
    },
    ClosedUser {
        scen: u32,
        st: u8,
    },
    Forked {
        beh: BehaviourId,
        st: u8,
    },
    Scenario {
        scen: u32,
        t0: i64,
        st: u8,
    },
    /// A usage-scenario behaviour at instruction `pc` of `CompiledModel::code` (`t0`: start of
    /// the measured entry-level system call, `t1`: of its system operation; -1: none).
    UBeh {
        pc: u32,
        t0: i64,
        t1: i64,
    },
    ULoop {
        body: ScenarioBehaviourId,
        left: i32,
    },
    /// A call of `Simulation::provs[prov]`.
    ProvRole {
        prov: u32,
        t0: i64,
        st: u8,
    },
    ReqDeleg {
        ac: (AcRef, u32),
    },
    AsmConn {
        src: ContainerId,
        dst: ContainerId,
        /// The provided-role call at the other end.
        prov: u32,
        st: u8,
    },
    Infra {
        a: ActionId,
        ic: u32,
        left: i32,
        caller: (AcRef, u32),
        st: u8,
    },
    /// A SEFF behaviour at instruction `pc` of `CompiledModel::code` (`t0`: start of the
    /// measured external call, -1: none; `caller`: the assembly context popped by the current
    /// external call).
    SBeh {
        pc: u32,
        t0: i64,
        caller: (AcRef, u32),
    },
    SLoop {
        body: BehaviourId,
        left: i32,
    },
    Coll {
        a: ActionId,
        left: i32,
        st: u8,
    },
    ForkJoin {
        a: ActionId,
        /// Index into `Simulation::joins`: the synchronous children.
        join: u32,
    },
    ConsumeDone {
        res: ResIdx,
    },
}

#[derive(Default)]
struct ProcData {
    conts: Vec<Cont>,
    frames: Vec<Rc<Frame>>,
    results: Vec<Rc<Frame>>,
    /// Assembly-context stack: the context and the trie id of the path `acs[1..=i]`.
    acs: Vec<(AcRef, u32)>,
    /// Interpreter levels of the reference that the flat code runs inside the behaviour's own
    /// continuation but that are on the stack (an external or entry-level system call in
    /// progress, the internal action of an infrastructure call, a basic component's SEFF
    /// exit): counted for `Limits::max_stack_depth`.
    elided: u32,
}

struct Proc {
    generation: u32,
    pid: u64,
    state: PState,
    kind: PKind,
    /// The stacks (boxed: `run_process` swaps them out and back as one pointer).
    data: Box<ProcData>,
    /// In the running set of its own think-time delay resource.
    in_think: bool,
    last_res: LastRes,
    /// Resource-table insertion sequence (`None`: not in the table).
    table_seq: Option<u64>,
    /// `ForkedBehaviourProcess.isTerminated` flag.
    fork_done: bool,
    /// Response-time series of the open external-call / assembly-operation starts (MEAS-1.5).
    rt: Vec<SeriesId>,
}

/// The last resolution of an external-call site (`call_required` from the caller's assembly
/// context `ac`, with `parent` the path id of the context below it): the continuation it
/// pushed. Only calls that resolve without a required delegation are kept.
#[derive(Clone, Copy, Default)]
struct CallCache {
    ac: AcRef,
    parent: u32,
    cont: Option<Cont>,
}

/// A resolved provided-role call (`Simulation::provs`).
#[derive(Clone, Copy)]
struct ProvRes {
    ac: AcRef,
    role: RoleId,
    sig: SignatureId,
    /// Assembly-operation response-time series.
    aser: Option<SeriesId>,
    target: ProvTarget,
}

#[derive(Clone, Copy)]
enum ProvTarget {
    /// The providing structure delegates to `ir` of `ia` (call id `inner`, `u32::MAX` until
    /// first used).
    Deleg {
        ia: AcRef,
        ir: RoleId,
        inner: u32,
    },
    /// A basic component and its SEFF behaviour (`None`: not exactly one SEFF).
    Basic {
        comp: ComponentId,
        seff: Option<BehaviourId>,
    },
    Fail(&'static str),
}

enum Flow {
    Continue,
    Wait,
    Finished,
}

// ------------------------------------------------------------------------------------------
// resources at run time

struct ResRt {
    sched: Option<ActiveResource<u64>>,
    /// DELAY-type resources: `running_processes` (process keys).
    delay_set: Vec<u64>,
    /// Queue length per instance as the state probes read it.
    shadow: Vec<u64>,
}

struct PassiveInst {
    pr: PassiveResourceId,
    ac: AcRef,
    res: PassiveResource<u64>,
    capacity: i64,
    state: Option<SeriesId>,
    waiting: Option<SeriesId>,
    holding: Option<SeriesId>,
    wait_start: Vec<(u64, i64)>,
    hold_start: Vec<(u64, i64)>,
}

struct CompInst {
    passives: Vec<(PassiveResourceId, u32)>,
}

// ------------------------------------------------------------------------------------------
// measurement sink: recorded tuples plus the sliding windows fed by some series

pub(crate) struct Sink {
    m: Measurements,
    windows: Vec<Window>,
    /// Series id -> windows accepting its tuples.
    series_windows: Vec<Vec<u32>>,
    /// Last process id handed out (process ids count from 1 in creation order).
    next_pid: u64,
    /// pid of the process currently executing (0 = engine).
    cur_pid: u64,
    /// pid of the reconfiguration process (once created).
    reconf_pid: u64,
    /// An error raised inside a measurement emission (reported after the current event).
    error: Option<&'static str>,
    /// PRM recorders exist (MEAS-7.2).
    prm_armed: bool,
    prm: Prm,
    reconf: Reconf,
    /// The event list: runtime-measurement writes schedule the reconfiguration process from
    /// inside the measurement emission, at the point of the write (FIFO order of same-time notes).
    events: EventList,
}

/// `Reconfigurator` / `ReconfigurationProcess` state (MEAS-7.2).
#[derive(Default)]
struct Reconf {
    created: bool,
    /// Its `Resume` note is pending (`isScheduled()`).
    scheduled: bool,
    /// `lastReconfigurationTime` (ns).
    last_ns: i64,
    /// The experiment has stopped: scheduling resumes the process synchronously.
    stopped: bool,
    /// `Reconfiguration Time Tuple` series (one tuple per run).
    time_series: Vec<SeriesId>,
    store: bool,
    /// A synchronous run (after the stop) is in progress.
    running: bool,
    /// The process died: a PRM write during its own synchronous run scheduled it again.
    error: Option<&'static str>,
}

/// PRM recorders (`triggersSelfAdaptations`, MEAS-7.2). The `Reconfigurator` observes the
/// runtime measurement model: a change at a time after the last scheduling, while the process
/// is not scheduled, creates the `ReconfigurationProcess` (once: a process id and a `spawn`
/// line) and schedules its `Resume` at now. Each run reconfigures (successfully, without rules)
/// and passivates; only a `Reconfiguration Time` monitor makes the runs visible.
#[derive(Default)]
struct Prm {
    /// Series id -> indices into `recs`.
    series: Vec<Vec<u32>>,
    recs: Vec<PrmState>,
}

struct PrmState {
    rec: PrmRec,
    /// Tuples until the next aggregation check (`measurementsUntilNextAggregation`).
    until: i64,
    /// Buffered tuples (fixed size: the count, saturating at the buffer size).
    buffered: i64,
    /// Variable size: points in time of the buffered tuples.
    times: std::collections::VecDeque<f64>,
    /// Aggregates `Long` values (passive-resource state): the aggregation fails.
    long_values: bool,
}

impl PrmState {
    fn new(rec: PrmRec) -> PrmState {
        let until = match rec {
            PrmRec::FeedThrough => 0,
            PrmRec::Fixed { freq, .. } | PrmRec::Variable { freq, .. } => freq,
        };
        PrmState {
            rec,
            until,
            buffered: 0,
            times: Default::default(),
            long_values: false,
        }
    }

    /// A tuple reaches the recorder: does it write into the PRM?
    fn tuple(&mut self, t: f64) -> bool {
        match self.rec {
            PrmRec::FeedThrough => true,
            PrmRec::Fixed { freq, n } => {
                self.buffered = (self.buffered + 1).min(n);
                self.until -= 1;
                if self.until == 0 {
                    self.until = freq;
                    self.buffered == n
                } else {
                    false
                }
            }
            PrmRec::Variable {
                freq,
                retro,
                continuous,
            } => {
                use crate::windows::Amount;
                self.times.push_back(t);
                self.until -= 1;
                if self.until != 0 {
                    return false;
                }
                self.until = freq;
                let r = Amount::of(retro);
                let (first, last) = (self.times[0], self.times[self.times.len() - 1]);
                // aggregationRequired: !last.minus(R).isLessThan(first)
                if Amount::of(last).minus(&r).is_less_than(&Amount::of(first)) {
                    return false;
                }
                // onPreAggregate: evict what lies more than R before the newest tuple
                let old = |x: f64| Amount::of(last).minus(&Amount::of(x)).is_greater_than(&r);
                if continuous {
                    let mut polled = None;
                    while self.times.front().is_some_and(|&x| old(x)) {
                        polled = self.times.pop_front();
                    }
                    if let Some(x) = polled {
                        self.times.push_front(x);
                    }
                } else {
                    while self.times.front().is_some_and(|&x| old(x)) {
                        self.times.pop_front();
                    }
                }
                true
            }
        }
    }
}

/// A PRM write at `now` (`Reconfigurator.checkAndExecuteReconfigurations`).
#[cold]
#[inline(never)]
fn prm_write(meas: &mut Sink, trace: &mut Option<TraceOut>, now: i64) {
    let r = &mut meas.reconf;
    if !meas.prm_armed || now <= r.last_ns || r.scheduled {
        return;
    }
    if !r.created {
        r.created = true;
        meas.next_pid += 1;
        meas.reconf_pid = meas.next_pid;
        if let Some(tr) = trace.as_mut() {
            tr.spawn(
                now,
                meas.next_pid,
                "ReconfigurationProcess",
                "Reconfiguration Process",
                meas.cur_pid,
            );
        }
    }
    if r.running {
        // a write by the process's own run (a triggering Reconfiguration Time spec) before
        // lastReconfigurationTime is updated: scheduleAt of a running process aborts the run
        r.error = Some(
            "IllegalStateException: Tried to schedule thread which was not suspended [Reconfiguration Process]",
        );
        meas.prm_armed = false;
        if let Some(tr) = trace.as_mut() {
            tr.pend(now, meas.reconf_pid);
        }
        return;
    }
    if r.stopped {
        // scheduleAt(0) of a stopped experiment resumes the process right away (and
        // lastReconfigurationTime is set after it returns)
        r.running = true;
        reconf_run(meas, trace, now);
        meas.reconf.running = false;
        meas.reconf.last_ns = now;
    } else {
        r.last_ns = now;
        r.scheduled = true;
        meas.events.insert(now, Ev::Reconf);
    }
}

/// A run of the reconfiguration process: begin/end reconfiguration, which succeeds without
/// rules; the reconfiguration time is 0.
#[cold]
#[inline(never)]
fn reconf_run(meas: &mut Sink, trace: &mut Option<TraceOut>, now: i64) {
    for i in 0..meas.reconf.time_series.len() {
        let s = meas.reconf.time_series[i];
        let store = meas.reconf.store;
        emit(meas, trace, store, now, s, seconds(now), 0.0);
    }
}

/// A tuple of series `s` reaches its PRM recorders.
#[cold]
#[inline(never)]
fn prm_tuple(meas: &mut Sink, trace: &mut Option<TraceOut>, now: i64, s: SeriesId, t: f64) {
    let Some(recs) = meas.prm.series.get(s as usize) else {
        return;
    };
    let mut write = false;
    for &r in recs {
        let rec = &mut meas.prm.recs[r as usize];
        if rec.tuple(t) {
            if rec.long_values {
                meas.error.get_or_insert(LONG_AGGREGATION_ERROR);
            }
            write = true;
        }
    }
    if write {
        prm_write(meas, trace, now);
    }
}

// ------------------------------------------------------------------------------------------
// measurement emission helpers (free functions so that disjoint fields can be borrowed)

#[inline]
fn emit(
    meas: &mut Sink,
    trace: &mut Option<TraceOut>,
    store: bool,
    now: i64,
    s: SeriesId,
    t: f64,
    v: f64,
) {
    if store {
        meas.m.rows[s as usize].push((t, v));
    }
    meas.m.count += 1;
    if let Some(tr) = trace.as_mut() {
        let def = &meas.m.series[s as usize];
        tr.meas(now, &def.mp, def.metric, t, v);
    }
    if let Some(ws) = meas.series_windows.get(s as usize) {
        for &w in ws {
            meas.windows[w as usize].add(t, v);
        }
    }
    if meas.prm_armed {
        prm_tuple(meas, trace, now, s, t);
    }
}

/// Emission of a window's aggregated tuple (written to its recorder, not fed back to windows).
fn emit_out(
    meas: &mut Sink,
    trace: &mut Option<TraceOut>,
    store: bool,
    now: i64,
    s: SeriesId,
    t: f64,
    v: f64,
) {
    if store {
        meas.m.rows[s as usize].push((t, v));
    }
    meas.m.count += 1;
    if let Some(tr) = trace.as_mut() {
        let def = &meas.m.series[s as usize];
        tr.meas(now, &def.mp, def.metric, t, v);
    }
}

struct ResListener<'a> {
    meas: &'a mut Sink,
    trace: &'a mut Option<TraceOut>,
    store: bool,
    now: i64,
    def: &'a CRes,
    shadow: &'a mut Vec<u64>,
    per_core: bool,
}

impl ResourceListener<u64> for ResListener<'_> {
    fn state_changed(&mut self, core: u32, state: u64) {
        let t = seconds(self.now);
        let c = core as usize;
        if c < self.shadow.len() {
            self.shadow[c] = state;
        }
        if let Some(ss) = self.def.state_series.get(c) {
            for &s in ss {
                emit(
                    self.meas,
                    self.trace,
                    self.store,
                    self.now,
                    s,
                    t,
                    state as f64,
                );
            }
        }
        if let Some(s) = self.def.overall_series {
            let f = busy_fraction(self.shadow, self.def.instances, self.per_core);
            emit(self.meas, self.trace, self.store, self.now, s, t, f);
        }
    }
}

fn busy_fraction(shadow: &[u64], instances: u32, per_core: bool) -> f64 {
    let busy = (0..instances as usize)
        .filter(|&i| {
            let q = if per_core {
                shadow.get(i).copied().unwrap_or(0)
            } else {
                shadow[0]
            };
            q > 0
        })
        .count();
    busy as f64 / f64::from(instances)
}

// ------------------------------------------------------------------------------------------

/// One simulation run over a [`CompiledModel`], with the simulator policy `C` ([`Exact`] by
/// default; see [`crate::compat`]).
pub struct Simulation<'m, C: Compat = Exact> {
    cm: &'m CompiledModel,
    /// The behaviours' instructions for this run (with or without the trace instructions).
    code: &'m crate::code::Flat,
    cfg: SimConfig,
    now: i64,
    running: bool,
    procs: Vec<Proc>,
    free: Vec<u32>,
    res: Vec<ResRt>,
    passives: Vec<PassiveInst>,
    /// Basic component instances by path id.
    comps: Vec<Option<CompInst>>,
    /// Paths missing from the compiled trie (created on first use).
    dyn_paths: Vec<AcPath>,
    dyn_path_child: FxMap<(u32, AcRef), u32>,
    rng: C::Rng,
    meas: Sink,
    /// Series created during the run (lazily created passive-resource calculators), by
    /// measuring-point key (see `CompiledModel::series_by_mp`).
    run_series: HashMap<Box<str>, Vec<(&'static str, SeriesId)>>,
    trace: Option<TraceOut>,
    main_count: i64,
    table_seq: u64,
    n_events: u64,
    warnings: Vec<String>,
    /// The shared empty frame (parameterless calls, fresh result frames).
    empty: Rc<Frame>,
    /// Synchronous children of pending fork joins (slots reused through `free_joins`).
    joins: Vec<Vec<u64>>,
    free_joins: Vec<u32>,
    fork_scratch: Vec<u64>,
    /// Provided-role calls, by id (see `prov_id`).
    provs: Vec<ProvRes>,
    prov_ids: FxMap<(AcRef, RoleId, SignatureId), u32>,
    /// Interpreter steps so far, and the step / event count of the next [`Limits`] check.
    steps: u64,
    next_step_check: u64,
    next_event_check: u64,
    /// `Limits::max_stack_depth` (`usize::MAX` if unlimited).
    max_depth: usize,
    /// Nesting of `run_process` (processes run synchronously after the stop).
    nesting: u32,
    /// Livelock guard: consecutive events at `now`, and `main_count` when last reset.
    same_instant: u64,
    count_seen: i64,
    /// Recycled frames (see [`FramePool`]).
    pool: FramePool,
    /// Nesting of direct continuation-handler calls (`Simulation::enter`).
    direct: u32,
    /// Per external-call site ([`crate::code::SOp::Call`]): its last resolution.
    call_cache: Vec<CallCache>,
    /// Per user action: the provided-role call of the entry-level system call (`u32::MAX`:
    /// not resolved yet).
    elsc_prov: Vec<u32>,
    /// Empty stacks that stand in for a running process's (see `run_process`); boxed because
    /// they are swapped with `Proc::data`.
    #[allow(clippy::vec_box)]
    spare_data: Vec<Box<ProcData>>,
}

/// Proxy-depth errors of the evaluator are limits (see `frames::MAX_PROXY_DEPTH`).
fn eval_error_kind(e: &simoxide_stoex::EvalError) -> SimErrorKind {
    if e.message().starts_with("limit exceeded:") {
        SimErrorKind::Limit
    } else {
        SimErrorKind::Model
    }
}

/// `Calculator` start/stop matching (MEAS-1.5): a second start in the same request context.
const RT_SAME_CONTEXT: &str = "IllegalStateException: First measurement to the same context arrived while previous series of the same context did not complete.";

/// Internal result: the error is boxed so that the hot paths pass small `Result`s around.
type R<T> = Result<T, Box<SimError>>;

impl<'m> Simulation<'m, Exact> {
    /// An exact-mode simulation (`cfg.mode` must be [`Mode::Exact`]; [`crate::run`] dispatches on
    /// the mode, [`Simulation::create`] builds any policy).
    pub fn new(cm: &'m CompiledModel, cfg: SimConfig, out: Outputs) -> Result<Self, SimError> {
        Self::create(cm, cfg, out)
    }
}

impl<'m, C: Compat> Simulation<'m, C> {
    /// A simulation with policy `C`, e.g. `Simulation::<Fast>::create(..)`; `cfg.mode` must be
    /// `C::MODE`.
    pub fn create(cm: &'m CompiledModel, cfg: SimConfig, out: Outputs) -> Result<Self, SimError> {
        if cfg.mode != C::MODE {
            return Err(SimError {
                message: format!(
                    "SimConfig::mode is {} but the simulation is built for mode {} (use simoxide_sim::run to dispatch on the mode)",
                    cfg.mode,
                    C::MODE
                ),
                at_ns: 0,
                kind: SimErrorKind::Model,
            });
        }
        let rng = C::Rng::create(
            &cfg.rng,
            cfg.seed,
            out.tape,
            cfg.check_tape_origins,
            cfg.replay_samples,
        )
        .map_err(|m| SimError {
            message: m,
            at_ns: 0,
            kind: SimErrorKind::Model,
        })?;
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
        let max_depth = match cfg.limits.max_stack_depth {
            0 => usize::MAX,
            n => n as usize,
        };
        Ok(Simulation {
            cm,
            code: cm.code.get(out.trace.is_some()),
            cfg,
            now: 0,
            running: true,
            procs: Vec::new(),
            free: Vec::new(),
            res,
            passives: Vec::new(),
            comps: Vec::new(),
            dyn_paths: Vec::new(),
            dyn_path_child: FxMap::default(),
            rng,
            meas,
            run_series: HashMap::new(),
            trace: out.trace.map(TraceOut::new),
            main_count: 0,
            table_seq: 0,
            n_events: 0,
            warnings: Vec::new(),
            empty: Rc::new(Frame::default()),
            joins: Vec::new(),
            free_joins: Vec::new(),
            fork_scratch: Vec::new(),
            provs: Vec::new(),
            prov_ids: FxMap::default(),
            steps: 0,
            next_step_check: 0,
            next_event_check: 0,
            max_depth,
            nesting: 0,
            same_instant: 0,
            count_seen: 0,
            pool: FramePool::default(),
            direct: 0,
            call_cache: vec![CallCache::default(); cm.code.traced.call_sites as usize],
            elsc_prov: vec![u32::MAX; cm.uact.len()],
            spare_data: Vec::new(),
        })
    }

    #[cold]
    #[inline(never)]
    fn err<T>(&self, msg: impl Into<String>) -> R<T> {
        Err(Box::new(SimError {
            message: msg.into(),
            at_ns: self.now,
            kind: SimErrorKind::Model,
        }))
    }

    #[cold]
    #[inline(never)]
    fn limit_err<T>(&self, kind: SimErrorKind, msg: String) -> R<T> {
        Err(Box::new(SimError {
            message: msg,
            at_ns: self.now,
            kind,
        }))
    }

    /// Deadline and cancellation flag.
    #[cold]
    #[inline(never)]
    fn check_cancel(&self) -> R<()> {
        let l = &self.cfg.limits;
        if let Some(c) = &l.cancel
            && c.load(std::sync::atomic::Ordering::Relaxed)
        {
            return self.limit_err(SimErrorKind::Cancelled, "cancelled".into());
        }
        if let Some(d) = l.deadline
            && std::time::Instant::now() >= d
        {
            return self.limit_err(SimErrorKind::Cancelled, "deadline passed".into());
        }
        Ok(())
    }

    /// `Limits::max_steps`, deadline and cancellation, every `CHECK_INTERVAL` steps.
    #[cold]
    #[inline(never)]
    fn check_steps(&mut self) -> R<()> {
        let max = self.cfg.limits.max_steps;
        if max > 0 && self.steps > max {
            return self.limit_err(
                SimErrorKind::Limit,
                format!("limit exceeded: more than {max} interpreter steps (Limits::max_steps)"),
            );
        }
        self.check_cancel()?;
        self.next_step_check = self.steps + CHECK_INTERVAL;
        if max > 0 {
            self.next_step_check = self.next_step_check.min(max + 1);
        }
        Ok(())
    }

    /// `Limits::max_events`, deadline and cancellation, every `CHECK_INTERVAL` events.
    #[cold]
    #[inline(never)]
    fn check_events(&mut self) -> R<()> {
        let max = self.cfg.limits.max_events;
        if max > 0 && self.n_events > max {
            return self.limit_err(
                SimErrorKind::Limit,
                format!("limit exceeded: more than {max} events (Limits::max_events)"),
            );
        }
        self.check_cancel()?;
        self.next_event_check = self.n_events + CHECK_INTERVAL;
        if max > 0 {
            self.next_event_check = self.next_event_check.min(max + 1);
        }
        Ok(())
    }

    /// Runs the simulation to its end.
    #[inline(never)]
    pub fn run(mut self) -> Result<RunResult, SimError> {
        let r = self.run_inner();
        if let Some(t) = self.trace.as_mut() {
            t.finish();
        }
        self.rng.finish();
        r.map_err(|e| *e)?;
        if let Some(p) = self.rng.take_problem() {
            self.warnings.push(p);
        }
        Ok(RunResult {
            measurements: std::mem::take(&mut self.meas.m),
            uniforms: self.rng.count(),
            events: self.n_events,
            end_ns: self.now,
            main_count: self.main_count,
            warnings: std::mem::take(&mut self.warnings),
        })
    }

    fn run_inner(&mut self) -> R<()> {
        let cm = self.cm;
        if self.cfg.max_sim_time <= 0 && self.cfg.max_measurements <= 0 {
            return self.err("no stop condition enabled");
        }
        if let Some(t) = self.trace.as_mut() {
            t.header(
                &self.cfg.run_name,
                self.cfg.seed,
                self.cfg.max_sim_time,
                self.cfg.max_measurements,
            );
        }
        // resource environment: initial state tuples
        for r in &cm.resources {
            for &(s, _inst) in &r.initial_states {
                emit(
                    &mut self.meas,
                    &mut self.trace,
                    self.cfg.store_measurements,
                    0,
                    s,
                    0.0,
                    0.0,
                );
            }
        }
        // usage model: open workload generators are processes created at init
        let mut gens = Vec::new();
        for (i, s) in cm.scenarios.iter().enumerate() {
            if let CWorkload::Open { .. } = s.workload {
                let k = self.spawn(
                    PKind::OpenGen,
                    Cont::OpenGen {
                        scen: i as u32,
                        st: 0,
                    },
                )?;
                gens.push((i, k));
            }
        }
        // NumberOfResourceContainerTrackingListener: initial count (MEAS-7.3), after the
        // usage model syncer created the open-workload generators
        if let Some((s, n)) = cm.container_count {
            emit(
                &mut self.meas,
                &mut self.trace,
                self.cfg.store_measurements,
                0,
                s,
                0.0,
                n as f64,
            );
        }
        // probe-framework decorators attach the PRM recorders after the initial state and
        // container-count tuples
        if cm.prm_any {
            let prm = &mut self.meas.prm;
            prm.series = vec![Vec::new(); cm.series.len()];
            for &(s, rec) in &cm.prm_series {
                prm.series[s as usize].push(prm.recs.len() as u32);
                prm.recs.push(PrmState::new(rec));
            }
            self.meas.prm_armed = true;
        }
        // probe-framework decorators: sliding windows, first events at span(length) (MEAS-6.3)
        self.meas.series_windows = vec![Vec::new(); self.meas.m.series.len()];
        for (i, w) in cm.windows.iter().enumerate() {
            self.meas.windows.push(Window::new(w.len, w.inc, true));
            if let Some(inp) = w.input {
                self.meas.series_windows[inp as usize].push(i as u32);
            }
            let Some(span) = checked_span(w.len) else {
                return self.err("invalid sliding window length");
            };
            self.meas.events.insert(span, Ev::Window(i as u32));
        }
        // SimuComModel.init at t = 0: run the drivers in scenario order
        for (i, s) in cm.scenarios.iter().enumerate() {
            match s.workload {
                CWorkload::Open { .. } => {
                    let k = gens.iter().find(|g| g.0 == i).map(|g| g.1).unwrap_or(0);
                    self.activate(k)?;
                }
                CWorkload::Closed { population, .. } => {
                    for _ in 0..population.max(0) {
                        let k = self.spawn(
                            PKind::ClosedUser,
                            Cont::ClosedUser {
                                scen: i as u32,
                                st: 0,
                            },
                        )?;
                        self.activate(k)?;
                    }
                }
            }
        }
        // main loop
        self.count_seen = self.main_count;
        while let Some((t, ev)) = self.meas.events.pop() {
            self.count_event(t)?;
            self.dispatch(ev)?;
            if let Some(e) = self.meas.error {
                return self.err(e);
            }
            if let Some(p) = self.rng.exhausted() {
                return self.err(p.to_string());
            }
            if self.stop_condition() {
                break;
            }
        }
        self.finalise()
    }

    /// Per-event bookkeeping of an event at `t` (also for an elided hand-off note, see
    /// [`Self::handoff`]): the livelock guard, the event count and the `Limits` checks.
    #[inline]
    fn count_event(&mut self, t: i64) -> R<()> {
        debug_assert!(t >= self.now);
        self.same_instant = if t == self.now {
            self.same_instant + 1
        } else {
            0
        };
        let limit = match self.cfg.max_events_per_instant {
            0 => u64::MAX,
            n => n,
        };
        if self.same_instant >= limit {
            // a finished scenario run brings a max-measurements stop closer
            if self.cfg.max_measurements > 0 && self.main_count != self.count_seen {
                self.count_seen = self.main_count;
                self.same_instant = 0;
            } else {
                return self.livelock(self.same_instant, t);
            }
        }
        self.now = t;
        self.n_events += 1;
        if self.n_events >= self.next_event_check {
            self.check_events()?;
        }
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn livelock(&self, n: u64, t: i64) -> R<()> {
        self.limit_err(SimErrorKind::Limit, format!(
            "livelock: {n} events at simulation time {} s without progress towards a stop condition (REF-7; see --max-events-per-instant)",
            seconds(t)
        ))
    }

    #[inline]
    fn stop_condition(&self) -> bool {
        (self.cfg.max_sim_time > 0 && seconds(self.now) >= self.cfg.max_sim_time as f64)
            || (self.cfg.max_measurements > 0 && self.main_count >= self.cfg.max_measurements)
    }

    fn finalise(&mut self) -> R<()> {
        let cm = self.cm;
        self.running = false;
        self.meas.reconf.stopped = true;
        let now = self.now;
        if let Some(t) = self.trace.as_mut() {
            t.stop(now);
        }
        // simulationStop listeners: final partial windows
        for i in 0..self.meas.windows.len() {
            let eff = self.meas.windows[i].effective_length(seconds(now));
            if eff != 0.0 || eff.is_sign_negative() {
                self.window_full(i);
            }
        }
        // deactivateAllActiveResources: current queue lengths, then overall utilisation
        for &ri in &cm.finalise_order {
            let def = &cm.resources[ri as usize];
            let rt = &mut self.res[ri as usize];
            let per_core = def.policy == SchedulingPolicy::ProcessorSharing;
            for inst in 0..def.instances as usize {
                let q = if per_core {
                    rt.shadow[inst]
                } else {
                    rt.shadow[0]
                };
                for &s in &def.state_series[inst] {
                    emit(
                        &mut self.meas,
                        &mut self.trace,
                        self.cfg.store_measurements,
                        now,
                        s,
                        seconds(now),
                        q as f64,
                    );
                }
            }
            if let Some(s) = def.overall_series {
                let f = busy_fraction(&rt.shadow, def.instances, per_core);
                emit(
                    &mut self.meas,
                    &mut self.trace,
                    self.cfg.store_measurements,
                    now,
                    s,
                    seconds(now),
                    f,
                );
            }
            if let Some(s) = rt.sched.as_mut() {
                s.stop();
                if s.pending_wakeup().is_none() {
                    self.meas.events.clear_timer(ri);
                }
            }
        }
        // ResourceTableManager.waitForProcesses: drain in insertion order
        let mut table: Vec<(u64, u64)> = self
            .procs
            .iter()
            .enumerate()
            .filter(|(_, p)| p.state != PState::Terminated)
            .filter_map(|(i, p)| p.table_seq.map(|s| (s, key(i as u32, p.generation))))
            .collect();
        table.sort_unstable();
        for (_, k) in table {
            self.activate(k)?;
        }
        // SimulatedBasicComponentInstance.cleanUp: processes still waiting at passive resources
        let mut waiting = Vec::new();
        for p in &self.passives {
            waiting.extend(p.res.waiting().map(|(j, _)| j));
        }
        for k in waiting {
            self.activate(k)?;
        }
        let (u, n) = (self.rng.count(), self.meas.m.count);
        if let Some(t) = self.trace.as_mut() {
            t.finish_line(now, u, n);
        }
        // the reconfiguration process died after the stop; the run ends with its error
        if let Some(e) = self.meas.reconf.error.or(self.meas.error) {
            return self.err(e);
        }
        Ok(())
    }

    // --------------------------------------------------------------------------- processes

    /// Creates a suspended process running `body`, with empty frame and assembly-context stacks
    /// (a freed slot's stacks are reused, keeping their capacity).
    fn spawn(&mut self, kind: PKind, body: Cont) -> R<u64> {
        let max = self.cfg.limits.max_processes as usize;
        if max > 0 && self.procs.len() - self.free.len() >= max {
            return self.limit_err(
                SimErrorKind::Limit,
                format!("limit exceeded: more than {max} live processes (Limits::max_processes)"),
            );
        }
        self.meas.next_pid += 1;
        let pid = self.meas.next_pid;
        let slot = match self.free.pop() {
            Some(s) => {
                let p = &mut self.procs[s as usize];
                p.pid = pid;
                p.state = PState::Suspended;
                p.kind = kind;
                debug_assert!(p.data.frames.is_empty() && p.data.acs.is_empty());
                p.data.conts.push(body);
                p.in_think = false;
                p.last_res = LastRes::None;
                p.table_seq = None;
                p.fork_done = false;
                s
            }
            None => {
                let mut data = Box::<ProcData>::default();
                data.conts.push(body);
                self.procs.push(Proc {
                    generation: 0,
                    pid,
                    state: PState::Suspended,
                    kind,
                    data,
                    in_think: false,
                    last_res: LastRes::None,
                    table_seq: None,
                    fork_done: false,
                    rt: Vec::new(),
                });
                (self.procs.len() - 1) as u32
            }
        };
        if let Some(t) = self.trace.as_mut() {
            let (k, n) = match kind {
                PKind::OpenGen => ("OpenWorkload", "OpenWorkloadUserMaturationChamber"),
                PKind::OpenUser => ("OpenWorkloadUser", "OpenUser"),
                PKind::ClosedUser => ("ClosedWorkloadUser", "ClosedUser"),
                PKind::Forked { .. } => ("ForkedBehaviourProcess", "Forked Behaviour"),
            };
            t.spawn(self.now, pid, k, n, self.meas.cur_pid);
        }
        Ok(key(slot, self.procs[slot as usize].generation))
    }

    /// Is the process (still) this incarnation and not terminated?
    #[inline]
    fn alive(&self, k: u64) -> bool {
        let (s, g) = unkey(k);
        self.procs
            .get(s as usize)
            .is_some_and(|p| p.generation == g && p.state != PState::Terminated)
    }

    /// `p.activate()` = `scheduleAt(0)` (SIM-4.3).
    fn activate(&mut self, k: u64) -> R<()> {
        let (s, g) = unkey(k);
        let Some(p) = self.procs.get(s as usize) else {
            return Ok(());
        };
        if p.generation != g || p.state == PState::Terminated {
            return Ok(());
        }
        if p.state == PState::Running {
            return self.err("IllegalStateException: activate() of a running process");
        }
        if !self.running {
            // stopped experiment: the process is resumed synchronously
            return self.run_process(s);
        }
        self.meas.events.insert(self.now, Ev::Resume(k));
        Ok(())
    }

    /// `activate()` from an engine-level event (the end of a think time, of a DELAY demand or of
    /// a PS/FCFS demand), the last thing the event does: DESMO-J's thread hand-off is a `Resume`
    /// note at `now` (SIM-4.6/4.7). When that note would be the very next one anyway, the
    /// process runs right away, inside this event: no other note is pending at `now`, the run
    /// does not stop after this event (a stopping event leaves the process to the post-stop
    /// drain) and no error is pending (which the main loop would raise between the notes). The
    /// elided note still counts as an event and passes the per-event checks
    /// ([`Self::count_event`]), and it writes no trace line, so every output is unchanged; only
    /// the event-list round trip is saved. [`Compat::LITERAL_ENGINE`] always schedules the note.
    #[inline]
    fn handoff(&mut self, k: u64) -> R<()> {
        if !C::LITERAL_ENGINE
            && self.running
            && self.meas.events.next_time() != Some(self.now)
            && self.meas.error.is_none()
            && self.rng.exhausted().is_none()
            && !self.stop_condition()
        {
            // the checks of `activate` when it schedules the note
            let (s, g) = unkey(k);
            let Some(p) = self.procs.get(s as usize) else {
                return Ok(());
            };
            if p.generation != g || p.state == PState::Terminated {
                return Ok(());
            }
            if p.state == PState::Running {
                return self.err("IllegalStateException: activate() of a running process");
            }
            // the `Resume` note's event
            self.count_event(self.now)?;
            return self.run_process(s);
        }
        self.activate(k)
    }

    fn dispatch(&mut self, ev: Ev) -> R<()> {
        match ev {
            Ev::Reconf => {
                self.meas.reconf.scheduled = false;
                reconf_run(&mut self.meas, &mut self.trace, self.now);
            }
            Ev::Resume(k) => {
                let (s, g) = unkey(k);
                let p = &self.procs[s as usize];
                if p.generation == g && p.state == PState::Suspended {
                    self.run_process(s)?;
                }
            }
            Ev::Think(k) => {
                let (s, g) = unkey(k);
                let p = &mut self.procs[s as usize];
                if p.generation == g && p.state != PState::Terminated && p.in_think {
                    p.in_think = false;
                    self.handoff(k)?;
                }
            }
            Ev::DelayRes(ri, k) => {
                let rt = &mut self.res[ri as usize];
                if let Some(pos) = rt.delay_set.iter().position(|&x| x == k) {
                    rt.delay_set.swap_remove(pos);
                    let n = rt.delay_set.len() as u64;
                    self.delay_state_changed(ri, n);
                    self.handoff(k)?;
                }
            }
            Ev::Window(i) => {
                self.window_full(i as usize);
                let inc = self.cm.windows[i as usize].inc;
                let Some(span) = checked_span(inc) else {
                    return self.err("invalid sliding window increment");
                };
                self.meas.events.insert(self.now + span, Ev::Window(i));
            }
            Ev::Wake(ri) => {
                let cm = self.cm;
                let def = &cm.resources[ri as usize];
                let rt = &mut self.res[ri as usize];
                let per_core = def.policy == SchedulingPolicy::ProcessorSharing;
                let mut l = ResListener {
                    meas: &mut self.meas,
                    trace: &mut self.trace,
                    store: self.cfg.store_measurements,
                    now: self.now,
                    def,
                    shadow: &mut rt.shadow,
                    per_core,
                };
                let sched = rt.sched.as_mut().expect("scheduler");
                // the timer slot always holds the resource's current (pending) wake-up
                let w = sched.pending_wakeup().expect("pending wake-up");
                debug_assert_eq!(w.at, self.now);
                let r = sched.on_wakeup(self.now, &w, &mut l);
                let c = match r {
                    Ok(c) => c,
                    Err(e) => return self.err(e.0),
                };
                if let Some(c) = c {
                    if let Some(n) = c.next {
                        self.meas.events.set_timer(ri, n.at);
                    }
                    self.handoff(c.job)?;
                }
            }
        }
        Ok(())
    }

    /// `onWindowFullEvent`: aggregate and record, then move the window on.
    fn window_full(&mut self, i: usize) {
        let now_s = seconds(self.now);
        let w = &mut self.meas.windows[i];
        let eff = w.effective_length(now_s);
        let res = crate::windows::utilization(&w.data, w.lower, eff);
        w.move_on();
        if let Some(out) = self.cm.windows[i].out {
            emit_out(
                &mut self.meas,
                &mut self.trace,
                self.cfg.store_measurements,
                self.now,
                out,
                res.0,
                res.1,
            );
        }
        if self.cm.windows[i].prm {
            prm_write(&mut self.meas, &mut self.trace, self.now);
        }
    }

    fn delay_state_changed(&mut self, ri: ResIdx, n: u64) {
        let def = &self.cm.resources[ri as usize];
        let rt = &mut self.res[ri as usize];
        let mut l = ResListener {
            meas: &mut self.meas,
            trace: &mut self.trace,
            store: self.cfg.store_measurements,
            now: self.now,
            def,
            shadow: &mut rt.shadow,
            per_core: false,
        };
        l.state_changed(0, n);
    }

    /// Runs process `s` until it waits or ends.
    fn run_process(&mut self, s: u32) -> R<()> {
        // after the stop, processes (and forked children) run synchronously, nested here
        if self.nesting >= MAX_NESTING {
            return self.limit_err(
                SimErrorKind::Limit,
                format!("limit exceeded: more than {MAX_NESTING} processes run synchronously inside each other after the stop"),
            );
        }
        self.nesting += 1;
        let prev_pid = self.meas.cur_pid;
        let spare = self.spare_data.pop().unwrap_or_default();
        let mut data = {
            let p = &mut self.procs[s as usize];
            p.state = PState::Running;
            self.meas.cur_pid = p.pid;
            std::mem::replace(&mut p.data, spare)
        };
        let r = loop {
            self.steps += 1;
            if self.steps >= self.next_step_check
                && let Err(e) = self.check_steps()
            {
                break Err(e);
            }
            match self.step(s, &mut data) {
                Ok(Flow::Continue) => {}
                Ok(Flow::Wait) => {
                    let p = &mut self.procs[s as usize];
                    p.state = PState::Suspended;
                    let spare = std::mem::replace(&mut p.data, data);
                    self.spare_data.push(spare);
                    break Ok(());
                }
                Ok(Flow::Finished) => {
                    self.terminate(s, data);
                    break Ok(());
                }
                Err(e) => break Err(e),
            }
        };
        self.meas.cur_pid = prev_pid;
        self.nesting -= 1;
        r
    }

    fn terminate(&mut self, s: u32, mut data: Box<ProcData>) {
        let p = &mut self.procs[s as usize];
        p.state = PState::Terminated;
        p.table_seq = None;
        let pid = p.pid;
        p.generation = p.generation.wrapping_add(1);
        // keep the stacks' capacity for the next process in this slot
        data.conts.clear();
        self.pool.recycle_all(&mut data.frames);
        self.pool.recycle_all(&mut data.results);
        data.acs.clear();
        data.elided = 0;
        p.rt.clear();
        let spare = std::mem::replace(&mut p.data, data);
        self.spare_data.push(spare);
        self.free.push(s);
        if let Some(t) = self.trace.as_mut() {
            t.pend(self.now, pid);
        }
    }

    #[inline]
    fn pkey(&self, s: u32) -> u64 {
        key(s, self.procs[s as usize].generation)
    }

    // --------------------------------------------------------------------------- evaluation

    /// `StackContext.evaluateStatic(spec, frame)`: the raw result.
    fn eval(&mut self, prog: ProgId, frame: Option<&Frame>, origin: Origin<'_>) -> R<Value> {
        let cm = self.cm;
        let p = &cm.progs[prog as usize];
        if let Some(v) = &p.konst {
            // a constant draws nothing: no origin, no tape record, no recorded sample
            return Ok(v.clone());
        }
        if self.rng.plain() {
            let r = match &p.prog {
                Ok(pr) => pr.eval(
                    &FrameEnv {
                        frame,
                        cm,
                        depth: 0,
                    },
                    &mut self.rng,
                ),
                Err(e) => Err(simoxide_stoex::EvalError::new(
                    simoxide_stoex::EvalErrorKind::Runtime,
                    e.clone(),
                )),
            };
            return r.map_err(|e| self.eval_error(prog, e));
        }
        let prev = self.rng.set_origin(origin);
        self.rng.eval_begin();
        let r = match &p.prog {
            Ok(pr) => pr.eval(
                &FrameEnv {
                    frame,
                    cm,
                    depth: 0,
                },
                &mut self.rng,
            ),
            Err(e) => Err(simoxide_stoex::EvalError::new(
                simoxide_stoex::EvalErrorKind::Runtime,
                e.clone(),
            )),
        };
        let r = match self.rng.recorded_sample() {
            Some(v) => Ok(v),
            None => r,
        };
        self.rng.eval_end(&p.spec, r.as_ref().ok());
        self.rng.restore_origin(prev);
        r.map_err(|e| self.eval_error(prog, e))
    }

    #[cold]
    #[inline(never)]
    fn eval_error(&self, prog: ProgId, e: simoxide_stoex::EvalError) -> Box<SimError> {
        Box::new(SimError {
            message: format!(
                "Evaluation of expression {} failed: {:?}",
                self.cm.progs[prog as usize].spec, e
            ),
            at_ns: self.now,
            kind: eval_error_kind(&e),
        })
    }

    #[inline]
    fn eval_f64(&mut self, prog: ProgId, frame: Option<&Frame>, origin: Origin<'_>) -> R<f64> {
        // a constant draws nothing: no origin, no tape record (as in `eval`)
        if let Some(v) = self.cm.progs[prog as usize].konst_f64 {
            return Ok(v);
        }
        let v = self.eval(prog, frame, origin)?;
        v.to_f64().map_err(|e| {
            Box::new(SimError {
                message: format!("{:?}", e),
                at_ns: self.now,
                kind: SimErrorKind::Model,
            })
        })
    }

    fn eval_i32(&mut self, prog: ProgId, frame: Option<&Frame>, origin: Origin<'_>) -> R<i32> {
        let v = self.eval(prog, frame, origin)?;
        v.to_i32().map_err(|e| {
            Box::new(SimError {
                message: format!("{:?}", e),
                at_ns: self.now,
                kind: SimErrorKind::Model,
            })
        })
    }

    fn eval_bool(&mut self, prog: ProgId, frame: Option<&Frame>, origin: Origin<'_>) -> R<bool> {
        let v = self.eval(prog, frame, origin)?;
        v.to_bool().map_err(|e| {
            Box::new(SimError {
                message: format!("{:?}", e),
                at_ns: self.now,
                kind: SimErrorKind::Model,
            })
        })
    }

    /// A proxy evaluated as an outermost `evaluateStatic` (tape `s` record).
    fn eval_proxy_top(&mut self, p: &Proxy) -> R<Value> {
        let cm = self.cm;
        self.rng.eval_begin();
        let r = eval_proxy(cm, p, &mut self.rng);
        let r = match self.rng.recorded_sample() {
            Some(v) => Ok(v),
            None => r,
        };
        self.rng
            .eval_end(&cm.progs[p.prog as usize].spec, r.as_ref().ok());
        r.map_err(|e| {
            Box::new(SimError {
                message: format!("{:?}", e),
                at_ns: self.now,
                kind: eval_error_kind(&e),
            })
        })
    }

    /// `addParameterToStackFrame(context, usages, target)` (ACT-5.2).
    fn fill(
        &mut self,
        usages: &[CChar],
        ctx: Option<&Rc<Frame>>,
        target: &mut Frame,
        origin: Origin<'_>,
    ) -> R<()> {
        for c in usages {
            if c.inner {
                target.put(
                    c.key,
                    Binding::Proxy(Rc::new(Proxy {
                        prog: c.prog,
                        frame: ctx.cloned(),
                    })),
                );
            } else {
                let v = self.eval(c.prog, ctx.map(|f| &**f), origin)?;
                target.put(c.key, Binding::Val(v));
            }
        }
        Ok(())
    }

    /// A new parentless frame with `usages` evaluated in `ctx` (the shared empty frame if there
    /// are none: writes into a shared frame copy it first, see [`Frame`]).
    fn input_frame(
        &mut self,
        usages: &[CChar],
        ctx: Option<&Rc<Frame>>,
        origin: Origin<'_>,
    ) -> R<Rc<Frame>> {
        if usages.is_empty() {
            return Ok(self.empty.clone());
        }
        let mut f = self.pool.frame(None);
        self.fill(usages, ctx, frame_mut(&mut f), origin)?;
        Ok(f)
    }

    /// `NumberConverter.toDouble` of a frame binding.
    fn binding_to_f64(&mut self, b: &Binding) -> R<f64> {
        let v = match b {
            Binding::Val(v) => v.clone(),
            Binding::Proxy(p) => self.eval_proxy_top(p)?,
        };
        match v {
            Value::Int(i) => Ok(f64::from(i)),
            Value::Double(d) => Ok(d),
            other => self.err(format!("NumberConverter: cannot convert {other:?}")),
        }
    }

    // --------------------------------------------------------------------------- resources

    /// `hold(d)` of the running process: returns true if the process now waits.
    fn hold(&mut self, s: u32, d: f64) -> R<bool> {
        let pid = self.procs[s as usize].pid;
        if let Some(t) = self.trace.as_mut() {
            t.hold(self.now, pid, d);
        }
        if !self.running {
            return Ok(false);
        }
        let k = self.pkey(s);
        let last = self.procs[s as usize].last_res;
        if last != LastRes::Think {
            if let LastRes::Active(r) = last {
                self.dequeue(r, k)?;
            }
            self.procs[s as usize].in_think = true;
            self.set_last(s, LastRes::Think);
        }
        let p = &mut self.procs[s as usize];
        if !p.in_think {
            p.in_think = true;
        }
        let Some(span) = checked_span(d) else {
            return self.err(format!("SimAbortedException: invalid delay {d}"));
        };
        self.meas.events.insert(self.now + span, Ev::Think(k));
        Ok(true)
    }

    fn set_last(&mut self, s: u32, r: LastRes) {
        let p = &mut self.procs[s as usize];
        if p.table_seq.is_none() {
            p.table_seq = Some(self.table_seq);
            self.table_seq += 1;
        }
        p.last_res = r;
    }

    /// `dequeue` of the previously used resource (only DELAY-type resources act).
    fn dequeue(&mut self, r: ResIdx, k: u64) -> R<()> {
        let rt = &mut self.res[r as usize];
        if rt.sched.is_none()
            && let Some(pos) = rt.delay_set.iter().position(|&x| x == k)
        {
            rt.delay_set.swap_remove(pos);
            let n = rt.delay_set.len() as u64;
            self.delay_state_changed(r, n);
            self.activate(k)?;
        }
        Ok(())
    }

    /// `AbstractScheduledResource.consumeResource`: returns true if the process now waits
    /// (a `ConsumeDone` continuation must then be pushed by the caller).
    fn consume(&mut self, s: u32, ri: ResIdx, demand: f64, service: i64) -> R<bool> {
        let cm = self.cm;
        let def = &cm.resources[ri as usize];
        // HDDResource.consumeResource: bytes / read or write rate first
        let demand = match def.hdd {
            None => demand,
            Some((read, write)) => {
                let rate = match service {
                    1 => read,
                    2 => write,
                    _ => return self.err("HDD Resource called without explicit read/write call"),
                };
                demand / self.eval_f64(rate, None, Origin::Plain("?"))?
            }
        };
        let origin = Origin::Plain(&def.origin);
        let concrete = match def.kind {
            ResKind::Processing(_) => {
                let rate = self.eval_f64(def.rate, None, origin)?;
                demand / rate + 0.0
            }
            ResKind::Link(_) => {
                let tp = self.eval_f64(def.rate, None, origin)?;
                if tp <= 0.0 {
                    return self.err("ThroughputZeroOrNegativeException");
                }
                let latency = match def.latency {
                    Some(l) => self.eval_f64(l, None, origin)?,
                    None => 0.0,
                };
                let mut c = demand / tp;
                c /= 1.0;
                let add = 0.0 + latency;
                c + add
            }
        };
        let pid = self.procs[s as usize].pid;
        if let Some(t) = self.trace.as_mut() {
            t.demand(
                self.now,
                pid,
                &def.type_id,
                &def.rc,
                &def.spec_name,
                &def.sched_name,
                demand,
                concrete,
            );
        }
        if concrete <= 0.0 {
            if let Some(t) = self.trace.as_mut() {
                t.demand_done(self.now, pid, &def.type_id, &def.rc);
            }
            return Ok(false);
        }
        for &sid in &def.demand_series {
            emit(
                &mut self.meas,
                &mut self.trace,
                self.cfg.store_measurements,
                self.now,
                sid,
                seconds(self.now),
                concrete,
            );
        }
        if !self.running {
            if let Some(t) = self.trace.as_mut() {
                t.demand_done(self.now, pid, &def.type_id, &def.rc);
            }
            return Ok(false);
        }
        // AbstractActiveResource.process
        let k = self.pkey(s);
        let last = self.procs[s as usize].last_res;
        let is_delay = self.res[ri as usize].sched.is_none();
        if last != LastRes::Active(ri) {
            match last {
                LastRes::Active(r) => self.dequeue(r, k)?,
                LastRes::Think => {
                    // the process's own think-time delay resource
                    if self.procs[s as usize].in_think {
                        self.procs[s as usize].in_think = false;
                        self.activate(k)?;
                    }
                }
                LastRes::None => {}
            }
            if is_delay {
                self.delay_enqueue(ri, k);
            }
            self.set_last(s, LastRes::Active(ri));
        }
        if is_delay {
            if !self.res[ri as usize].delay_set.contains(&k) {
                self.delay_enqueue(ri, k);
            }
            let Some(span) = checked_span(concrete) else {
                return self.err(format!("SimAbortedException: invalid delay {concrete}"));
            };
            self.meas
                .events
                .insert(self.now + span, Ev::DelayRes(ri, k));
        } else {
            let rt = &mut self.res[ri as usize];
            let per_core = def.policy == SchedulingPolicy::ProcessorSharing;
            let mut l = ResListener {
                meas: &mut self.meas,
                trace: &mut self.trace,
                store: self.cfg.store_measurements,
                now: self.now,
                def,
                shadow: &mut rt.shadow,
                per_core,
            };
            let w = rt
                .sched
                .as_mut()
                .expect("scheduler")
                .process(self.now, k, concrete, &mut l);
            match w {
                Ok(w) => self.meas.events.set_timer(ri, w.at),
                Err(e) => return self.err(e.0),
            }
        }
        Ok(true)
    }

    fn delay_enqueue(&mut self, ri: ResIdx, k: u64) {
        let rt = &mut self.res[ri as usize];
        if !rt.delay_set.contains(&k) {
            rt.delay_set.push(k);
        }
        let n = rt.delay_set.len() as u64;
        self.delay_state_changed(ri, n);
    }

    fn path_node(&self, id: u32) -> &AcPath {
        let n = self.cm.ac_paths.len();
        match self.cm.ac_paths.get(id as usize) {
            Some(p) => p,
            None => &self.dyn_paths[id as usize - n],
        }
    }

    /// The FQ assembly-context path of trie node `id`.
    fn path_of(&self, mut id: u32) -> Vec<u32> {
        let mut v = Vec::new();
        while id != ROOT_PATH {
            let p = self.path_node(id);
            v.push(p.ac);
            id = p.parent;
        }
        v.reverse();
        v
    }

    /// Trie id of path `parent` extended by `ac`.
    #[inline]
    fn path_child(&mut self, parent: u32, ac: AcRef) -> u32 {
        if let Some(c) = self.cm.path_child(parent, ac) {
            return c;
        }
        self.dyn_path_child_slow(parent, ac)
    }

    #[cold]
    fn dyn_path_child_slow(&mut self, parent: u32, ac: AcRef) -> u32 {
        if let Some(&c) = self.dyn_path_child.get(&(parent, ac)) {
            return c;
        }
        let mut path = self.path_of(parent);
        path.push(ac);
        let container = self.cm.alloc.get(&path[..]).copied();
        let id = (self.cm.ac_paths.len() + self.dyn_paths.len()) as u32;
        self.dyn_paths.push(AcPath {
            parent,
            ac,
            container,
        });
        self.dyn_path_child.insert((parent, ac), id);
        id
    }

    /// Path id of the assembly-context stack `acs` after pushing `ac` (`acs[0]` is not part of
    /// the path).
    #[inline]
    fn path_after_push(&mut self, acs: &[(AcRef, u32)], ac: AcRef) -> u32 {
        match acs.last() {
            None => ROOT_PATH,
            Some(&(_, p)) => self.path_child(p, ac),
        }
    }

    #[inline]
    fn ac_push(&mut self, acs: &mut Vec<(AcRef, u32)>, ac: AcRef) {
        let p = self.path_after_push(acs, ac);
        acs.push((ac, p));
    }

    /// Container of an assembly-context path.
    #[inline]
    fn container_of(&self, path: u32) -> R<ContainerId> {
        match self.path_node(path).container {
            Some(c) => Ok(c),
            None => self.err(format!(
                "No allocation registered for assembly context {}",
                self.path_of(path)
                    .iter()
                    .map(|&a| self.cm.ac_id(a).to_string())
                    .collect::<Vec<_>>()
                    .join("::")
            )),
        }
    }

    #[inline]
    fn resource_of(&self, c: ContainerId, rt: Option<ResourceTypeId>) -> R<ResIdx> {
        let Some(rt) = rt else {
            return self.err("resource demand without resource type");
        };
        match self.cm.resource_in(c, rt) {
            Some(r) => Ok(r),
            // nested resource containers are not simulated: the container lookup returns null
            None if !self.cm.container_simulated[c.index()] => self.err(format!(
                "NullPointerException: resource container {} is not simulated (nested resource containers are ignored by SimuLizar 5.2.2)",
                self.cm.model.containers[c].id
            )),
            None => self.err(format!(
                "ResourceContainerIsMissingRequiredResourceType {}",
                self.cm.model.resource_types[rt].id
            )),
        }
    }

    /// Network transmission between two containers (ACT-11): returns true if waiting.
    fn transmit(
        &mut self,
        s: u32,
        src: ContainerId,
        dst: ContainerId,
        payload: Option<&Frame>,
    ) -> R<Option<ResIdx>> {
        let cm = self.cm;
        let demand = if self.cfg.simulate_linking_resources {
            // MiddlewareCompletionAwareDemandCalculator: the middleware completion is expected to
            // have put the marshalled size on the frame as `stream.BYTESIZE`
            self.eval_f64(cm.stream_bytesize, payload, Origin::Plain("?"))?
        } else if self.cfg.simulate_throughput_of_linking_resources {
            let mut d = 0.0f64;
            if let Some(f) = payload
                && f.any_key(|k| cm.keys.bytesize[k as usize])
            {
                f.visit_contents(&cm.keys, |k, b| {
                    if cm.keys.bytesize[k as usize] {
                        d += self.binding_to_f64(b)?;
                    }
                    Ok::<(), Box<SimError>>(())
                })?;
            }
            d
        } else {
            0.0
        };
        if src == dst {
            return Ok(None);
        }
        let n = cm.model.containers.len();
        let ri = match cm.route_res.get(src.index() * n + dst.index()) {
            Some(&r) => (r != u32::MAX).then_some(r),
            None => {
                let m = &cm.model;
                let link = cm.links.iter().copied().find(|&l| {
                    let c = &m.linking_resources[l].connected;
                    c.contains(&src) && c.contains(&dst)
                });
                link.map(|l| cm.link_res[&l])
            }
        };
        let Some(ri) = ri else {
            return self.err(
                "Could not determine route between nodes. This should be turned into a simulation feature.",
            );
        };
        Ok(self.consume(s, ri, demand, 0)?.then_some(ri))
    }

    // --------------------------------------------------------------------------- passive

    fn component_instance(&mut self, d: &ProcData) -> R<()> {
        let path = cur_path(d);
        if self.comps.get(path as usize).is_some_and(|c| c.is_some()) {
            return Ok(());
        }
        let cm = self.cm;
        let m = &cm.model;
        if path == ROOT_PATH {
            return self.err("basic component without assembly context");
        }
        let ac = self.path_node(path).ac;
        let Some(comp) = m.assembly_contexts[AssemblyContextId(ac)].component else {
            return self.err("assembly context without component");
        };
        let mut passives = Vec::new();
        let cur = d.frames.last().map(|f| &**f);
        for &pr in m.components[comp].passive_resources() {
            let prd = &m.passive_resources[pr];
            // capacity: (long) evaluateStatic(spec, Long.class, current frame)
            let v = self.eval(cm.pr_capacity[pr.index()], cur, Origin::Plain("?"))?;
            let capacity = match v.convert(simoxide_stoex::Expected::Long) {
                Ok(simoxide_stoex::Converted::Long(l)) => l,
                _ => return self.err(format!("capacity {v:?} is no Long")),
            };
            let mon = cm.passive_mon[pr.index()];
            let mp = format!(
                "ResourceURIMeasuringPoint[{}|Passive Resource: {}.{}]",
                cm.pr_ids[pr.index()],
                m.assembly_contexts[AssemblyContextId(ac)].name,
                prd.name
            );
            // calculators in the order state (+ initial tuple), waiting, holding; each
            // registration attaches the PRM recorders deferred for its MP and metric
            let rep = &mp[mp.find('|').map_or(0, |i| i + 1)..mp.len() - 1];
            let state = match mon.state {
                true => {
                    Some(self.passive_series(&mp, rep, metric_names::STATE_OF_PASSIVE_RESOURCE)?)
                }
                false => None,
            };
            if let Some(st) = state {
                emit(
                    &mut self.meas,
                    &mut self.trace,
                    self.cfg.store_measurements,
                    self.now,
                    st,
                    seconds(self.now),
                    capacity as f64,
                );
            }
            let waiting = match mon.waiting {
                true => Some(self.passive_series(&mp, rep, metric_names::WAITING_TIME)?),
                false => None,
            };
            let holding = match mon.holding {
                true => Some(self.passive_series(&mp, rep, metric_names::HOLDING_TIME)?),
                false => None,
            };
            let idx = self.passives.len() as u32;
            self.passives.push(PassiveInst {
                pr,
                ac,
                res: PassiveResource::new(capacity.max(0) as u64),
                capacity,
                state,
                waiting,
                holding,
                wait_start: Vec::new(),
                hold_start: Vec::new(),
            });
            passives.push((pr, idx));
        }
        if self.comps.len() <= path as usize {
            self.comps.resize_with(path as usize + 1, || None);
        }
        self.comps[path as usize] = Some(CompInst { passives });
        Ok(())
    }

    /// Series of a lazily created passive-resource calculator. A new calculator gets the PRM
    /// recorders deferred for its measuring point (`rep`, the string representation) and metric.
    fn passive_series(&mut self, mp: &str, rep: &str, metric: &'static str) -> R<SeriesId> {
        let cm = self.cm;
        if cm.prm_passive.is_empty() || self.find_series(mp, metric).is_some() {
            return Ok(self.new_series(mp, metric));
        }
        let s = self.new_series(mp, metric);
        let mut attached = false;
        for p in cm
            .prm_passive
            .iter()
            .filter(|p| p.metric == metric && &*p.rep == rep)
        {
            if let Some(e) = &p.invalid {
                return self.err(e.to_string());
            }
            let prm = &mut self.meas.prm;
            if prm.series.len() <= s as usize {
                prm.series.resize(s as usize + 1, Vec::new());
            }
            prm.series[s as usize].push(prm.recs.len() as u32);
            let mut st = PrmState::new(p.rec);
            st.long_values = metric == metric_names::STATE_OF_PASSIVE_RESOURCE
                && !matches!(p.rec, PrmRec::FeedThrough);
            prm.recs.push(st);
            attached = true;
        }
        if attached {
            // PRMRecorder constructor: the new RuntimeMeasurement is added to the PRM
            prm_write(&mut self.meas, &mut self.trace, self.now);
        }
        Ok(s)
    }

    /// The series of measuring point `mp` and `metric`, if it exists.
    fn find_series(&self, mp: &str, metric: &'static str) -> Option<SeriesId> {
        [self.cm.series_by_mp.get(mp), self.run_series.get(mp)]
            .into_iter()
            .flatten()
            .flatten()
            .find(|e| e.0 == metric)
            .map(|e| e.1)
    }

    fn new_series(&mut self, mp: &str, metric: &'static str) -> SeriesId {
        if let Some(s) = self.find_series(mp, metric) {
            return s;
        }
        let s = self.meas.m.series.len() as SeriesId;
        self.meas.m.series.push(SeriesDef {
            mp: mp.into(),
            metric,
        });
        self.meas.m.rows.push(Vec::new());
        self.run_series
            .entry(mp.into())
            .or_default()
            .push((metric, s));
        s
    }

    fn passive_of(&self, d: &ProcData, pr: Option<PassiveResourceId>) -> R<u32> {
        let inst = self
            .comps
            .get(cur_path(d) as usize)
            .and_then(|c| c.as_ref());
        match (inst, pr) {
            (Some(i), Some(pr)) => match i.passives.iter().find(|x| x.0 == pr) {
                Some(x) => Ok(x.1),
                None => {
                    self.err("Illegal passive resource for this basic component instance passed")
                }
            },
            _ => self.err("Illegal passive resource for this basic component instance passed"),
        }
    }

    /// Acquire (ACT-8.2): returns true if the process now waits.
    fn acquire(&mut self, s: u32, d: &ProcData, pr: Option<PassiveResourceId>) -> R<bool> {
        if !self.running {
            return Ok(false);
        }
        let pi = self.passive_of(d, pr)? as usize;
        let k = self.pkey(s);
        let pid = self.procs[s as usize].pid;
        let cm = self.cm;
        let now = self.now;
        let inst = &mut self.passives[pi];
        let avail = inst.res.available();
        let queue = inst.res.waiting().count();
        let mut l = PassiveL {
            meas: &mut self.meas,
            trace: &mut self.trace,
            store: self.cfg.store_measurements,
            now,
            procs: &self.procs,
            pr_id: &cm.pr_ids[inst.pr.index()],
            ac_id: cm.ac_id(inst.ac),
            state: inst.state,
            waiting: inst.waiting,
            holding: inst.holding,
            capacity: inst.capacity,
            wait_start: &mut inst.wait_start,
            hold_start: &mut inst.hold_start,
            woken: Vec::new(),
            pre: Some((pid, avail, queue)),
            error: None,
        };
        let granted = inst.res.acquire(k, 1, &mut l);
        if let Some(e) = l.error {
            return self.err(e);
        }
        Ok(!granted)
    }

    fn release(&mut self, s: u32, d: &ProcData, pr: Option<PassiveResourceId>) -> R<()> {
        if !self.running {
            return Ok(());
        }
        let pi = self.passive_of(d, pr)? as usize;
        let k = self.pkey(s);
        let cm = self.cm;
        let now = self.now;
        let inst = &mut self.passives[pi];
        let mut l = PassiveL {
            meas: &mut self.meas,
            trace: &mut self.trace,
            store: self.cfg.store_measurements,
            now,
            procs: &self.procs,
            pr_id: &cm.pr_ids[inst.pr.index()],
            ac_id: cm.ac_id(inst.ac),
            state: inst.state,
            waiting: inst.waiting,
            holding: inst.holding,
            capacity: inst.capacity,
            wait_start: &mut inst.wait_start,
            hold_start: &mut inst.hold_start,
            woken: Vec::new(),
            pre: None,
            error: None,
        };
        inst.res.release(k, 1, &mut l);
        let woken = std::mem::take(&mut l.woken);
        if let Some(e) = l.error {
            return self.err(e);
        }
        for w in woken {
            self.activate(w)?;
        }
        Ok(())
    }

    // --------------------------------------------------------------------------- interpreter

    fn pid(&self, s: u32) -> u64 {
        self.procs[s as usize].pid
    }

    /// Resolves a required role from assembly context `ac` (ACT-2.4..2.7) by pushing the
    /// continuations that perform the call.
    fn call_required(
        &mut self,
        d: &mut ProcData,
        ac: AcRef,
        role: Option<RoleId>,
        sig: SignatureId,
    ) -> R<()> {
        let cm = self.cm;
        let m = &cm.model;
        let Some(role) = role else {
            return self.err("Required role must not be null");
        };
        if ac == SYSTEM_AC {
            return self.err("Required delegation of the system cannot be simulated");
        }
        let acid = AssemblyContextId(ac);
        if m.assembly_contexts[acid].parent.is_none() {
            return self.err("Required delegation of the system cannot be simulated");
        }
        let Some(&c) = cm.req_conn.get(&(acid, role)) else {
            return self.err("Found unbound provided role. PCM model is invalid.");
        };
        match &m.connectors[c].kind {
            ConnectorKind::Assembly {
                requiring: Some(req),
                providing: Some(prov),
                provided_role: Some(prole),
                ..
            } => {
                let sp = self.path_after_push(&d.acs, req.0);
                let src = self.container_of(sp);
                let dp = self.path_after_push(&d.acs, prov.0);
                let dst = self.container_of(dp);
                let (src, dst) = (src?, dst?);
                let prov = self.prov_id(prov.0, *prole, sig);
                d.conts.push(Cont::AsmConn {
                    src,
                    dst,
                    prov,
                    st: 0,
                });
            }
            ConnectorKind::AssemblyInfrastructure {
                providing: Some(prov),
                provided_role: Some(prole),
                ..
            } => {
                let prov = self.prov_id(prov.0, *prole, sig);
                d.conts.push(Cont::ProvRole {
                    prov,
                    t0: -1,
                    st: 0,
                });
            }
            ConnectorKind::RequiredDelegation { outer_role, .. }
            | ConnectorKind::RequiredInfrastructureDelegation { outer_role, .. } => {
                let parent = d.acs.pop().unwrap_or((SYSTEM_AC, ROOT_PATH));
                d.conts.push(Cont::ReqDeleg { ac: parent });
                return self.call_required(d, parent.0, *outer_role, sig);
            }
            _ => return self.err("unsupported connector"),
        }
        Ok(())
    }

    fn step(&mut self, s: u32, d: &mut ProcData) -> R<Flow> {
        let cm = self.cm;
        let Some(top) = d.conts.last_mut() else {
            return Ok(Flow::Finished);
        };
        match top {
            // ------------------------------------------------------------------ bodies
            Cont::OpenGen { scen, st } => {
                if *st == 0 {
                    if !self.running {
                        d.conts.pop();
                        return Ok(Flow::Continue);
                    }
                    *st = 1;
                    let scen = *scen;
                    let u = self.spawn(PKind::OpenUser, Cont::OpenUser { scen, st: 0 })?;
                    self.activate(u)?;
                    let CWorkload::Open { inter_arrival } = cm.scenarios[scen as usize].workload
                    else {
                        unreachable!()
                    };
                    let ia = self.eval_f64(inter_arrival, None, Origin::Plain("interarrival"))?;
                    if self.hold(s, ia)? {
                        return Ok(Flow::Wait);
                    }
                } else {
                    *st = 0;
                }
            }
            Cont::OpenUser { scen, st } => {
                if *st == 0 {
                    *st = 1;
                    let scen = *scen;
                    d.conts.push(Cont::Scenario {
                        scen,
                        t0: -1,
                        st: 0,
                    });
                } else {
                    self.main_count += 1;
                    d.conts.pop();
                }
            }
            Cont::ClosedUser { scen, st } => match *st {
                0 => {
                    if !self.running {
                        d.conts.pop();
                        return Ok(Flow::Continue);
                    }
                    *st = 1;
                    let scen = *scen;
                    d.conts.push(Cont::Scenario {
                        scen,
                        t0: -1,
                        st: 0,
                    });
                }
                1 => {
                    *st = 2;
                    let CWorkload::Closed { think_time, .. } =
                        cm.scenarios[*scen as usize].workload
                    else {
                        unreachable!()
                    };
                    let tt = self.eval_f64(think_time, None, Origin::Plain("think"))?;
                    if self.hold(s, tt)? {
                        return Ok(Flow::Wait);
                    }
                }
                _ => {
                    *st = 0;
                    self.main_count += 1;
                }
            },
            Cont::Forked { beh, st } => {
                if *st == 0 {
                    *st = 1;
                    let beh = *beh;
                    d.conts.push(self.sbeh(beh));
                } else {
                    d.conts.pop();
                    self.procs[s as usize].fork_done = true;
                    if let PKind::Forked { sync: true, parent } = self.procs[s as usize].kind
                        && self.alive(parent)
                        && !self.fork_done_of(parent)
                        && self.running
                    {
                        self.activate(parent)?;
                    }
                }
            }
            // ------------------------------------------------------------------ usage model
            Cont::Scenario { .. } => return self.step_scenario(s, d),
            Cont::UBeh { .. } => return self.step_ubeh(s, d),
            Cont::ULoop { body, left } => {
                if *left <= 0 {
                    d.conts.pop();
                } else {
                    *left -= 1;
                    let body = *body;
                    d.conts.push(self.ubeh(body));
                }
            }
            // ------------------------------------------------------------------ composition
            Cont::ProvRole { .. } => return self.step_prov_role(s, d),
            Cont::ReqDeleg { ac } => {
                let ac = *ac;
                d.conts.pop();
                d.acs.push(ac);
            }
            Cont::AsmConn { .. } => return self.step_asm_conn(s, d),
            Cont::Infra { .. } => return self.step_infra(d),
            // ------------------------------------------------------------------ SEFF
            Cont::SBeh { .. } => return self.step_sbeh(s, d),
            Cont::SLoop { body, left } => {
                if *left <= 0 {
                    d.conts.pop();
                } else {
                    *left -= 1;
                    let body = *body;
                    d.conts.push(self.sbeh(body));
                }
            }
            Cont::Coll { .. } => return self.step_coll(d),
            Cont::ForkJoin { a, join } => {
                let waiting = self.joins[*join as usize]
                    .iter()
                    .any(|&c| self.alive(c) && !self.fork_done_of(c));
                if waiting {
                    return Ok(Flow::Wait);
                }
                let (a, join) = (*a, *join);
                d.conts.pop();
                self.joins[join as usize].clear();
                self.free_joins.push(join);
                let pid = self.pid(s);
                if let Some(t) = self.trace.as_mut() {
                    t.join(self.now, pid, &cm.action_ids[a.index()]);
                }
            }
            Cont::ConsumeDone { res } => {
                let def = &cm.resources[*res as usize];
                d.conts.pop();
                let pid = self.pid(s);
                if let Some(t) = self.trace.as_mut() {
                    t.demand_done(self.now, pid, &def.type_id, &def.rc);
                }
            }
        }
        Ok(Flow::Continue)
    }

    #[inline(never)]
    fn step_scenario(&mut self, s: u32, d: &mut ProcData) -> R<Flow> {
        let cm = self.cm;
        let Some(Cont::Scenario { scen, t0, st }) = d.conts.last_mut() else {
            unreachable!()
        };
        let sc = &cm.scenarios[*scen as usize];
        let sid = &cm.scenario_ids[sc.id.index()];
        if *st == 0 {
            *st = 1;
            let pid = self.pid(s);
            if let Some(t) = self.trace.as_mut() {
                t.element(self.now, true, pid, "UsageScenario", sid, None);
            }
            if self.running && sc.series.is_some() {
                *t0 = self.now;
            }
            // fresh interpreter context: one empty frame (WL-1.2)
            while let Some(f) = d.frames.pop() {
                self.pool.recycle(Some(f));
            }
            d.frames.push(self.pool.frame(None));
            d.results.clear();
            d.acs.clear();
            let Some(b) = sc.behaviour else {
                return self.err("usage scenario without behaviour");
            };
            d.conts.push(self.ubeh(b));
        } else {
            let t0 = *t0;
            let pid = self.pid(s);
            if let Some(t) = self.trace.as_mut() {
                t.element(self.now, false, pid, "UsageScenario", sid, None);
            }
            if let Some(se) = sc.series
                && self.running
                && t0 >= 0
            {
                let now = seconds(self.now);
                emit(
                    &mut self.meas,
                    &mut self.trace,
                    self.cfg.store_measurements,
                    self.now,
                    se,
                    now,
                    now - seconds(t0),
                );
            }
            d.conts.pop();
        }
        Ok(Flow::Continue)
    }

    /// Id of the provided-role call `(ac, role, sig)` in `provs` (resolved on first use).
    #[inline]
    fn prov_id(&mut self, ac: AcRef, role: RoleId, sig: SignatureId) -> u32 {
        if let Some(&id) = self.prov_ids.get(&(ac, role, sig)) {
            return id;
        }
        let r = self.resolve_prov(ac, role, sig);
        let id = self.provs.len() as u32;
        self.provs.push(r);
        self.prov_ids.insert((ac, role, sig), id);
        id
    }

    /// What a call of `role.sig` on assembly context `ac` runs (ACT-2.2..2.3): a provided
    /// delegation of the providing structure, or the component's SEFF. No side effects: errors
    /// are raised when the call happens.
    #[cold]
    fn resolve_prov(&self, ac: AcRef, role: RoleId, sig: SignatureId) -> ProvRes {
        let cm = self.cm;
        let m = &cm.model;
        // (the synthetic system assembly context has a random id: never monitored)
        let aser = if ac == SYSTEM_AC || cm.asmop_series.is_empty() {
            None
        } else {
            cm.asmop_series
                .get(&op_key(
                    &m.assembly_contexts[AssemblyContextId(ac)].id,
                    &m.roles[role].id,
                    &m.signatures[sig].id,
                ))
                .copied()
        };
        let res = |target| ProvRes {
            ac,
            role,
            sig,
            aser,
            target,
        };
        let structure = if ac == SYSTEM_AC {
            Some(m.systems[cm.system].structure)
        } else {
            let Some(comp) = m.assembly_contexts[AssemblyContextId(ac)].component else {
                return res(ProvTarget::Fail("assembly context without component"));
            };
            match &m.components[comp].kind {
                ComponentKind::Basic { .. } => None,
                ComponentKind::Composite { structure } | ComponentKind::SubSystem { structure } => {
                    Some(*structure)
                }
                _ => return res(ProvTarget::Fail("unsupported component type")),
            }
        };
        match structure {
            Some(stc) => {
                let Some(&c) = cm.prov_deleg.get(&(stc, role)) else {
                    return res(ProvTarget::Fail(
                        "Found unbound provided role. PCM model is invalid.",
                    ));
                };
                match &m.connectors[c].kind {
                    ConnectorKind::ProvidedDelegation {
                        inner_role: Some(ir),
                        assembly: Some(ia),
                        ..
                    } => res(ProvTarget::Deleg {
                        ia: ia.0,
                        ir: *ir,
                        inner: u32::MAX,
                    }),
                    _ => res(ProvTarget::Fail("invalid provided delegation connector")),
                }
            }
            None => {
                let comp = m.assembly_contexts[AssemblyContextId(ac)]
                    .component
                    .expect("component");
                let seffs = cm
                    .seff_for
                    .get(&(comp, cm.sig_key[sig.index()]))
                    .map_or(&[][..], |v| &v[..]);
                let seff = (seffs.len() == 1).then(|| m.seffs[seffs[0]].behaviour);
                res(ProvTarget::Basic { comp, seff })
            }
        }
    }

    #[inline(never)]
    fn step_infra(&mut self, d: &mut ProcData) -> R<Flow> {
        let cm = self.cm;
        let Some(Cont::Infra {
            a,
            ic,
            left,
            caller,
            st,
        }) = d.conts.last_mut()
        else {
            unreachable!()
        };
        let CAct::Internal { infra, .. } = &cm.act[a.index()] else {
            unreachable!()
        };
        let call = &infra[*ic as usize];
        if *st == 0 {
            if *left <= 0 {
                d.conts.pop();
                d.elided -= 1;
                return Ok(Flow::Continue);
            }
            *left -= 1;
            *st = 1;
            let Some(sig) = call.signature else {
                return self.err("infrastructure call without signature");
            };
            let fin = self.input_frame(
                &call.inputs,
                d.frames.last(),
                Origin::Elem("param", &call.id),
            )?;
            d.frames.push(fin);
            let c = d.acs.pop().unwrap_or((SYSTEM_AC, ROOT_PATH));
            *caller = c;
            self.call_required(d, c.0, call.role, sig)?;
        } else {
            *st = 0;
            let c = *caller;
            d.acs.push(c);
            self.pool.recycle(d.frames.pop());
        }
        Ok(Flow::Continue)
    }

    /// The process waits for a demand on `ri`: the `ConsumeDone` continuation only writes the
    /// trace line of the completed demand, so it is only needed with a trace.
    #[inline]
    fn wait_consumed(&self, d: &mut ProcData, ri: ResIdx) -> Flow {
        if self.trace.is_some() {
            d.conts.push(Cont::ConsumeDone { res: ri });
        }
        Flow::Wait
    }

    /// `evaluateInner`: a new frame over `cur` with the value of every `INNER` proxy whose key
    /// starts with `prefix`, in the Java `HashMap` order of `getContents()` of the new (empty)
    /// frame's chain. Visited in place (the literal engine copies the contents first; the
    /// proxies do not see the new frame, so both give the same values in the same order).
    fn eval_inner(&mut self, cur: Option<Rc<Frame>>, prefix: &str) -> R<Rc<Frame>> {
        let cm = self.cm;
        let mut rc = self.pool.frame(cur);
        let fi = frame_mut(&mut rc);
        if C::LITERAL_ENGINE {
            for (k, b) in fi.contents(&cm.keys) {
                if cm.keys.names[k as usize].starts_with(prefix)
                    && let Binding::Proxy(p) = &b
                {
                    let v = self.eval_proxy_top(p)?;
                    fi.put(k, Binding::Val(v));
                }
            }
        } else if let Some(c) = fi.parent.clone() {
            c.visit_contents(&cm.keys, |k, b| {
                if let Binding::Proxy(p) = b
                    && cm.keys.names[k as usize].starts_with(prefix)
                {
                    let v = self.eval_proxy_top(p)?;
                    fi.put(k, Binding::Val(v));
                }
                Ok::<(), Box<SimError>>(())
            })?;
        }
        Ok(rc)
    }

    #[inline(never)]
    fn step_coll(&mut self, d: &mut ProcData) -> R<Flow> {
        let cm = self.cm;
        let Some(Cont::Coll { a, left, st }) = d.conts.last_mut() else {
            unreachable!()
        };
        let a = *a;
        let CAct::Collection { prefix, body, .. } = &cm.act[a.index()] else {
            unreachable!()
        };
        if *st == 0 {
            if *left <= 0 {
                d.conts.pop();
                return Ok(Flow::Continue);
            }
            *left -= 1;
            *st = 1;
            let cur = d.frames.last().cloned();
            let aid = &cm.action_ids[a.index()];
            let prev = self.rng.set_origin(Origin::Elem("inner", aid));
            let fi = self.eval_inner(cur, prefix);
            self.rng.restore_origin(prev);
            d.frames.push(fi?);
            let Some(body) = *body else {
                return self.err("collection iterator without body");
            };
            d.conts.push(self.sbeh(body));
        } else {
            *st = 0;
            self.pool.recycle(d.frames.pop());
        }
        Ok(Flow::Continue)
    }

    #[inline]
    fn fork_done_of(&self, k: u64) -> bool {
        let (s, _) = unkey(k);
        self.procs[s as usize].fork_done
    }
}

/// Trie id of the current assembly-context path (`acs[1..]`).
#[inline]
fn cur_path(d: &ProcData) -> u32 {
    d.acs.last().map_or(ROOT_PATH, |e| e.1)
}

/// Listener for passive-resource events: trace lines, state/waiting/holding tuples, wake-ups.
struct PassiveL<'a> {
    meas: &'a mut Sink,
    trace: &'a mut Option<TraceOut>,
    store: bool,
    now: i64,
    procs: &'a [Proc],
    pr_id: &'a str,
    ac_id: &'a str,
    state: Option<SeriesId>,
    waiting: Option<SeriesId>,
    holding: Option<SeriesId>,
    capacity: i64,
    wait_start: &'a mut Vec<(u64, i64)>,
    hold_start: &'a mut Vec<(u64, i64)>,
    woken: Vec<u64>,
    /// acquire: (pid, available before, queue length before)
    pre: Option<(u64, i64, usize)>,
    error: Option<String>,
}

impl PassiveL<'_> {
    fn pid_of(&self, k: u64) -> u64 {
        let (s, _) = unkey(k);
        self.procs[s as usize].pid
    }
    fn hold_key(&self, k: u64) -> u64 {
        if self.capacity == 1 { u64::MAX } else { k }
    }
}

impl PassiveListener<u64> for PassiveL<'_> {
    fn requested(&mut self, job: u64, _num: u64) {
        if self.waiting.is_some() {
            if self.wait_start.iter().any(|x| x.0 == job) {
                self.error = Some("IllegalStateException: duplicate waiting-time start".into());
            }
            self.wait_start.push((job, self.now));
        }
        if let (Some(t), Some((pid, avail, queue))) = (self.trace.as_mut(), self.pre) {
            t.passive(
                self.now,
                "acquire",
                pid,
                self.pr_id,
                self.ac_id,
                avail,
                Some(queue),
            );
        }
    }

    fn acquired(&mut self, job: u64, _num: u64, available: i64) {
        let pid = self.pid_of(job);
        let now = self.now;
        let t = seconds(now);
        if let Some(tr) = self.trace.as_mut() {
            tr.passive(now, "grant", pid, self.pr_id, self.ac_id, available, None);
        }
        if let Some(s) = self.state {
            emit(
                self.meas,
                self.trace,
                self.store,
                now,
                s,
                t,
                available as f64,
            );
        }
        if let Some(s) = self.waiting
            && let Some(pos) = self.wait_start.iter().position(|x| x.0 == job)
        {
            let (_, t0) = self.wait_start.remove(pos);
            emit(
                self.meas,
                self.trace,
                self.store,
                now,
                s,
                t,
                t - seconds(t0),
            );
        }
        if self.holding.is_some() {
            let hk = self.hold_key(job);
            if self.hold_start.iter().any(|x| x.0 == hk) {
                self.error = Some("IllegalStateException: duplicate holding-time start".into());
            }
            self.hold_start.push((hk, now));
        }
    }

    fn released(&mut self, job: u64, _num: u64, available: i64) {
        let pid = self.pid_of(job);
        let now = self.now;
        let t = seconds(now);
        if let Some(tr) = self.trace.as_mut() {
            tr.passive(now, "release", pid, self.pr_id, self.ac_id, available, None);
        }
        if let Some(s) = self.state {
            emit(
                self.meas,
                self.trace,
                self.store,
                now,
                s,
                t,
                available as f64,
            );
        }
        if let Some(s) = self.holding {
            let hk = self.hold_key(job);
            if let Some(pos) = self.hold_start.iter().position(|x| x.0 == hk) {
                let (_, t0) = self.hold_start.remove(pos);
                emit(
                    self.meas,
                    self.trace,
                    self.store,
                    now,
                    s,
                    t,
                    t - seconds(t0),
                );
            }
        }
    }

    fn wake(&mut self, job: u64) {
        self.woken.push(job);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_keys_roundtrip() {
        let k = key(123_456, 7);
        assert_eq!(unkey(k), (123_456, 7));
    }

    #[test]
    fn busy_fraction_counts_busy_instances() {
        assert_eq!(busy_fraction(&[1, 0, 2, 0], 4, true), 0.5);
        assert_eq!(busy_fraction(&[3], 2, false), 1.0);
        assert_eq!(busy_fraction(&[0], 2, false), 0.0);
    }
}

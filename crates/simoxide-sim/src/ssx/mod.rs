//! The **Slingshot-exact mode** (`DESIGN-SPD.md` X2, decision U14; `docs/spec/slingshot/`,
//! rules `SS-*`; `docs/guide/slingshot.md`).
//!
//! Slingshot has no processes: every step of the model is a bus event, a handler reacts to it
//! and returns the events that follow, which are queued at `now + delay` behind the events
//! already queued for that instant (SS-ENG, SS-BUS). This engine reproduces that cascade
//! handler by handler, so that every projected step (the level-E projection, SS-X-2), every
//! draw and every measurement happens at the same position as in the reference runner's `det`
//! mode:
//!
//! * a FIFO event list keyed by `(double time, insertion)`; a handler's results are queued in
//!   insertion order after it returns, `MeasurementMade` directly when a probe fires
//!   (SS-BUS-4/5);
//! * one event kind per Slingshot event that has a subscriber or a projected line; bookkeeping
//!   events without either (`UserSlept`, `ProbeTaken`, `UsageScenarioStarted`, …) are left out,
//!   which changes no order;
//! * one handler function per Slingshot subscriber method, in subscription order (SS-BUS-2),
//!   written statement by statement after the Java code, including its partial side effects
//!   (draws, frame pushes, state changes, measurements) up to a throw;
//! * a **disposed bit per handler** (Q18, SS-BUS-7): a handler that throws never runs again,
//!   the run goes on;
//! * every quirk on (U5): this mode is Slingshot as the `det` runner runs it.
//!
//! Shared with the other flavours: the compiled model ([`CompiledModel`], compiled for
//! [`FlavourKind::SlingshotExact`] like the Slingshot flavour), the StoEx programs and their
//! evaluator, the frame bindings and evaluation proxies, the random streams (MT19937,
//! `java.util.Random`), the processor-sharing and FCFS schedulers of `simoxide-sched` on the
//! same `F64Secs` clock, the measurement sink and the CSV writer. What is Slingshot-only lives
//! in this module.
//!
//! Outputs: the run's measurements ([`RunResult`]); [`Outputs::trace`] receives the level-E
//! projection (`projection.jsonl`), [`Outputs::tape`] the tape projection
//! (`tape-projection.jsonl`), both in the format of `reference/slingshot/tools/project.py`.
//! The handler exceptions are in [`RunResult::warnings`].

mod frame;
mod monitor;
mod pdist;
mod resource;
mod rng;
mod run;
pub mod spd;
mod state;
mod system;
mod usage;

use crate::flavour::FlavourKind;
use crate::frames::Binding;
use crate::ir::*;
use crate::javafmt::{push_json_double, push_json_str};
use crate::meas::Measurements;
use crate::sim::{Outputs, RunResult, SimConfig, SimError, SimErrorKind};
use frame::{SEnv, SFrame};
use simoxide_model::*;
use simoxide_sched::Wakeup;
use simoxide_stoex::Value;
use std::cell::{Cell, RefCell};
use std::collections::{BinaryHeap, HashMap};
use std::rc::Rc;

pub(crate) use rng::SsRng;
pub use run::{SsxRun, SsxView};
pub use spd::{
    HandlerError, SpdAdjusted, SpdHooks, SpdMeasurement, SpdRequest, SpdStart, SpdTimeEvent,
    Stimulus,
};
#[cfg(feature = "snapshot")]
pub use state::SSX;
pub use state::SsxState;

// ------------------------------------------------------------------------------------------
// entities (Slingshot's objects, with Java's reference semantics where Slingshot shares them)

/// `User`: a SimuCom `StackContext` with an identity.
pub(crate) struct User {
    pub id: u64,
    pub stack: RefCell<Vec<SFrame>>,
}
pub(crate) type URef = Rc<User>;

/// `UsageScenarioBehaviorContext` (root, loop, branch).
pub(crate) struct UBeh {
    pub kind: UBehKind,
    pub behaviour: ScenarioBehaviourId,
    /// `nextAction`: `None` = `Optional.empty()`.
    pub next: Option<Option<UserActionId>>,
    pub parent: Option<Rc<UBeh>>,
    /// `RootScenarioContext.startOnce`.
    pub start_once: Cell<bool>,
    /// `LoopScenarioBehaviorContext`: maximal count and progression.
    pub max: i32,
    pub progression: Cell<i32>,
}

#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum UBehKind {
    Root,
    Loop,
    Branch,
}

/// `UserInterpretationContext` (closed or open workload).
pub(crate) struct UCtx {
    /// Index into `cm.scenarios`.
    pub scenario: usize,
    pub action: Option<UserActionId>,
    pub user: URef,
    pub beh: Rc<UBeh>,
    pub result: Option<SFrame>,
    pub closed: bool,
}

/// `UserRequest` of an entry-level system call.
pub(crate) struct UserRequest {
    pub user: URef,
    pub uact: UserActionId,
    pub role: Option<RoleId>,
    pub sig: Option<SignatureId>,
}

/// `SeffBehaviorWrapper`: the cursor over one behaviour.
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone)]
pub(crate) struct Wrapper {
    pub beh: BehaviourId,
    pub current: Option<ActionId>,
    pub finished: bool,
}

/// The kind of a `SeffBehaviorContextHolder`.
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone)]
pub(crate) enum HKind {
    Root,
    Branch,
    Loop {
        max: i32,
        progression: i32,
    },
    /// `ForkBehaviorContextHolder`: `unfinishedBehaviors` (indices into the wrappers), its
    /// `ArrayList` iterator (`cursor`, `last_ret`), `currentSeff`, `processingMarker`.
    Fork {
        unfinished: Vec<usize>,
        cursor: usize,
        last_ret: Option<usize>,
        current: usize,
        processed: bool,
    },
    /// `InfrastructureCallsContextHolder`: the expanded calls (action, call index).
    Infra {
        calls: Vec<(ActionId, usize)>,
        pos: usize,
    },
}

pub(crate) struct Holder {
    pub kind: HKind,
    pub wrappers: Vec<Wrapper>,
    /// `parent.isPresent()` (`isChild`).
    pub child: bool,
}
pub(crate) type HRef = Rc<RefCell<Holder>>;

/// `RequestProcessingContext`.
pub(crate) struct Rpc {
    pub role: Option<RoleId>,
    pub user: URef,
    pub uctx: Option<Rc<UCtx>>,
    pub request: Option<Rc<UserRequest>>,
}

/// `SEFFInterpretationContext`.
pub(crate) struct SeffCtx {
    /// Identity (`hashCode()` of the caller in request contexts).
    pub id: u64,
    pub holder: HRef,
    pub rpc: Rc<Rpc>,
    pub ac: AssemblyContextId,
    pub caller: Option<Rc<SeffCtx>>,
    pub cow: Option<Rc<Cow>>,
    pub parent: Option<Rc<SeffCtx>>,
    pub result: Option<SFrame>,
}

/// What a `GeneralEntryRequest` calls.
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy)]
pub(crate) enum GerKind {
    /// An `ExternalCallAction`.
    External(ActionId),
    /// The `n`-th `InfrastructureCall` of an internal action.
    Infra(ActionId, usize),
}

/// `GeneralEntryRequest`.
pub(crate) struct Ger {
    pub kind: GerKind,
    pub role: Option<RoleId>,
    pub sig: Option<SignatureId>,
    pub user: URef,
    pub from: Rc<SeffCtx>,
}

/// `CallOverWireRequest` (request or reply).
pub(crate) struct Cow {
    pub from: AssemblyContextId,
    pub to: AssemblyContextId,
    pub user: URef,
    pub entry: Rc<Ger>,
    pub vars: Option<SFrame>,
    pub reply: bool,
}

/// `ResourceDemandRequest`.
pub(crate) struct Rdr {
    pub ac: AssemblyContextId,
    pub ctx: Rc<SeffCtx>,
    pub passive: Option<PassiveResourceId>,
    pub active: bool,
    pub action: ActionId,
    /// Index of the `ParametricResourceDemand` in the action's list.
    pub demand: usize,
}

/// A `Job` (active or linking).
pub(crate) struct Job {
    pub id: u64,
    pub demand: Cell<f64>,
    pub kind: JobKind,
}

pub(crate) enum JobKind {
    Active {
        rtype: Option<ResourceTypeId>,
        req: Rc<Rdr>,
        /// The container of the allocation context (`None`: the assembly is not allocated).
        container: Option<ContainerId>,
    },
    Link {
        link: LinkingResourceId,
        cow: Rc<Cow>,
    },
}

// ------------------------------------------------------------------------------------------
// events

/// The Slingshot events the engine dispatches.
pub(crate) enum Ev {
    PreSim,
    SimStarted,
    Finished,
    // usage
    UPassed(Rc<UCtx>, UserActionId),
    UserStarted(Rc<UCtx>),
    InterArrival(Rc<UCtx>),
    WokeUp(Rc<UCtx>),
    ReqFinished(Rc<UserRequest>, Rc<UCtx>),
    UserFinished(Rc<UCtx>),
    UserAborted(Rc<UCtx>),
    ClosedInit(Rc<UCtx>),
    InnerInit(Rc<UCtx>),
    UProgressed(Rc<UCtx>),
    EntryReq(Rc<UserRequest>, Rc<UCtx>),
    // system
    SProgressed(Rc<SeffCtx>),
    SChild(Rc<SeffCtx>),
    SFinished(Rc<SeffCtx>),
    SPassed(Rc<SeffCtx>, ActionId),
    ExtCalled(Rc<Ger>),
    InfraCalled(Rc<Ger>),
    CowReq(Rc<Cow>),
    CowOk(Rc<Cow>),
    CowAbort(Rc<Cow>),
    RdReq(Rc<Rdr>),
    RdAbort(Rc<Rdr>),
    ArFinished(Rc<Rdr>),
    // resources
    JobInit(Rc<Job>),
    JobProg(Rc<Job>, Wakeup<u64>),
    JobFin(Rc<Job>),
    JobAbort(Rc<Job>),
    ArState(ResIdx, u64, f64),
    ArDemand(ResIdx, f64),
    PAcq(Rc<Rdr>),
    PRel(Rc<Rdr>),
    PState(Rc<Rdr>, i64),
    // monitoring
    Meas(SeriesId, f64, f64),
    // scaling policies (SS-SPD)
    /// `SimulationTimeReached` (index of the set-up's time event).
    SpdTime(u32),
    /// `ModelAdjustmentRequested` (the hook's request, the policy id).
    SpdReq(u32, Rc<str>),
    ModelAdjusted(Rc<spd::Adjusted>),
}

/// A handler's result: events with their delays, in insertion order.
pub(crate) type Out = Vec<(Ev, f64)>;

/// An exception thrown by a handler (SS-BUS-7).
#[derive(Debug, Clone)]
pub(crate) struct Throw {
    pub class: &'static str,
    pub message: String,
}

pub(crate) type HR = Result<Out, Throw>;

pub(crate) fn throw<T>(class: &'static str, message: impl Into<String>) -> Result<T, Throw> {
    Err(Throw {
        class,
        message: message.into(),
    })
}

/// `AbstractSimulationEvent(delay)`: `Preconditions.checkArgument(delay >= 0)`.
pub(crate) fn delay(d: f64) -> Result<f64, Throw> {
    if d >= 0.0 {
        Ok(d)
    } else {
        throw(
            "IllegalArgumentException",
            format!(
                "Delay must be at least 0, but is {}",
                crate::javafmt::double(d)
            ),
        )
    }
}

/// The subscriber methods (SS-BUS-2), the units of disposal.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum H {
    UsageStart,
    UserStarted,
    InterArrival,
    WokeUp,
    ReqFinished,
    UserFinished,
    UserAborted,
    ClosedInit,
    InnerInit,
    UProgressed,
    SysEntry,
    SysExternal,
    SysCowOk,
    SysCowAbort,
    SysRdAbort,
    SysInfra,
    SysArFinished,
    SeffProgressed,
    SeffPassiveAcq,
    SeffChild,
    SeffFinished,
    RsRdReq,
    RsJobInit,
    RsPassiveRel,
    RsJobProg,
    RsJobFin,
    RsJobAbort,
    RsCowReq,
    MonActiveState,
    MonActiveDemand,
    MonPassiveReq,
    MonPassiveAcq,
    MonPassiveRel,
    MonPassiveState,
    MonUsageStart,
    MonUsageStop,
    MonOpStart,
    MonOpStop,
    SpdStart,
    SpdAdjust,
    NoeResource,
    RsModelAdjusted,
    NoeSystem,
    MonModelAdjusted,
}

const H_COUNT: usize = H::MonModelAdjusted as usize + 1;

impl H {
    /// `Class.method` of the subscriber.
    pub fn name(self) -> &'static str {
        match self {
            H::UsageStart => "UsageSimulationBehavior.onSimulationStart",
            H::UserStarted => "UsageSimulationBehavior.onUserStarted",
            H::InterArrival => "UsageSimulationBehavior.onInterArrivalUserInitiated",
            H::WokeUp => "UsageSimulationBehavior.onWakeUpUserEvent",
            H::ReqFinished => "UsageSimulationBehavior.onUserRequestFinished",
            H::UserFinished => "UsageSimulationBehavior.onUserFinished",
            H::UserAborted => "UsageSimulationBehavior.onUserAborted",
            H::ClosedInit => "UsageSimulationBehavior.onClosedWorkloadUserInitiated",
            H::InnerInit => "UsageSimulationBehavior.onInnerScenarioBehaviorInitiated",
            H::UProgressed => "UsageSimulationBehavior.onUserInterpretationProgressed",
            H::SysEntry => "SystemSimulationBehavior.onUserEntryRequested",
            H::SysExternal => "SystemSimulationBehavior.onRequestInitiated",
            H::SysCowOk => "SystemSimulationBehavior.onCallOverWireSucceeded",
            H::SysCowAbort => "SystemSimulationBehavior.onCallOverWireAborted",
            H::SysRdAbort => "SystemSimulationBehavior.onResourceDemandRequestAborted",
            H::SysInfra => "SystemSimulationBehavior.onSEFFInfrastructureCalled",
            H::SysArFinished => "SystemSimulationBehavior.onActiveResourceFinished",
            H::SeffProgressed => "SeffSimulationBehavior.onSeffInterpretationProgressed",
            H::SeffPassiveAcq => "SeffSimulationBehavior.onPassiveResourceAcquired",
            H::SeffChild => "SeffSimulationBehavior.onSEFFChildInterpretationStarted",
            H::SeffFinished => "SeffSimulationBehavior.onSEFFInterpretationFinished",
            H::RsRdReq => "ResourceSimulation.onResourceDemandRequested",
            H::RsJobInit => "ResourceSimulation.onJobInitiated",
            H::RsPassiveRel => "ResourceSimulation.onPassiveResourceReleased",
            H::RsJobProg => "ResourceSimulation.onJobProgressed",
            H::RsJobFin => "ResourceSimulation.onJobFinished",
            H::RsJobAbort => "ResourceSimulation.onJobAborted",
            H::RsCowReq => "ResourceSimulation.onCallOverWireRequested",
            H::MonActiveState => "ActiveResourceMonitorBehavior.onActiveResourceStateUpdated",
            H::MonActiveDemand => "ActiveResourceMonitorBehavior.onResourceDemandCalculated",
            H::MonPassiveReq => "PassiveResourceMonitoringBehavior.onResourceDemandRequest",
            H::MonPassiveAcq => "PassiveResourceMonitoringBehavior.onPassiveResourceAcquired",
            H::MonPassiveRel => "PassiveResourceMonitoringBehavior.onPassiveResourceReleased",
            H::MonPassiveState => "PassiveResourceMonitoringBehavior.onPassiveResourceStateUpdated",
            H::MonUsageStart => {
                "UsageScenarioResponseTimeMonitoringBehavior.onUsageScenarioStarted"
            }
            H::MonUsageStop => {
                "UsageScenarioResponseTimeMonitoringBehavior.onUsageScenarioFinished"
            }
            H::MonOpStart => {
                "OperationCallActionResponseTimeMonitoringBehavior.onOperationCallStarted"
            }
            H::MonOpStop => {
                "OperationCallActionResponseTimeMonitoringBehavior.onOperationCallStopped"
            }
            H::SpdStart => "SpdBehavior.onPreSimulationConfigurationStarted",
            H::SpdAdjust => "SpdAdjustmentBehavior.onModelAdjustmentRequested",
            H::NoeResource => "NumberOfElementsMonitorBehavior.onModelAdjusted",
            H::RsModelAdjusted => "ResourceSimulation.onModelAdjusted",
            H::NoeSystem => "NumberOfElementsMonitorBehavior.onModelAdjusted",
            H::MonModelAdjusted => "MonitorRepositoryInterpreterBehavior.onModelAdjusted",
        }
    }
}

// ------------------------------------------------------------------------------------------
// event list (SSJ's SplayTree: (time, insertion) order, SS-ENG-3)

struct Entry {
    t: f64,
    seq: u64,
    ev: Ev,
}

impl PartialEq for Entry {
    fn eq(&self, o: &Self) -> bool {
        self.seq == o.seq
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Entry {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        // a max-heap: the earliest (time, seq) is the greatest
        o.t.total_cmp(&self.t).then(o.seq.cmp(&self.seq))
    }
}

// ------------------------------------------------------------------------------------------
// the engine

/// (assembly context, provided role, signature) ids (`AssemblyOperationCompoundKey`).
pub(crate) type OpKey = (Box<str>, Box<str>, Box<str>);

/// Calculators of the monitors (built from the monitor repository at the start, SS-MEAS).
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Default, Clone)]
pub(crate) struct Monitors {
    /// Per `cm.scenarios` index: the response-time calculator (the last monitor of a scenario
    /// wins, `userProbesMap.put`).
    pub scenario: Vec<Option<usize>>,
    /// Assembly-operation calculators by (assembly context, provided role, signature) ids.
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::sorted")
    )]
    pub asmop: HashMap<OpKey, usize>,
    /// Per resource: state, utilisation and demand calculators (series), registration order.
    pub state: Vec<Vec<SeriesId>>,
    pub util: Vec<Vec<SeriesId>>,
    pub demand: Vec<Vec<SeriesId>>,
    /// Per passive resource id: the calculators observing each probe, registration order.
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::sorted")
    )]
    pub passive: HashMap<Box<str>, PassiveProbes>,
    /// Start/stop calculators: series and open starts per request context.
    pub calcs: Vec<StartStop>,
    /// The series by (measuring-point key, metric). Two series of created elements can show the
    /// same key (equal aliases): the reference's EDP2 measurements are per measuring point, so
    /// they stay two series, written one after the other (`measurements.csv` sorts stably).
    #[cfg_attr(feature = "snapshot", serde(with = "crate::ssx::state::keys_map"))]
    pub keys: HashMap<(String, &'static str), SeriesId>,
    /// The key with the model's ids of a series whose shown key has aliases.
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::sorted")
    )]
    pub raw_keys: HashMap<SeriesId, Box<str>>,
}

#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Default, Clone)]
pub(crate) struct PassiveProbes {
    /// Waiting-time calculators (start: requested, stop: acquired), holding-time calculators
    /// (start: acquired, stop: released), in registration order.
    pub waiting: Vec<usize>,
    pub holding: Vec<usize>,
    /// Calculators on the acquired probe, in registration order (`true`: waiting, stops).
    pub acquired: Vec<(usize, bool)>,
    pub state: Vec<SeriesId>,
}

/// A start-stop calculator (`Calculator` with two probes).
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone)]
pub(crate) struct StartStop {
    pub series: SeriesId,
    pub open: Vec<(RequestCtx, f64)>,
}

/// `RequestContext`: the user id, plus the caller context's identity for SEFF calls.
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct RequestCtx {
    pub user: u64,
    pub caller: Option<u64>,
}

pub(crate) struct Engine<'m> {
    /// The compiled model the run simulates now (the base model, or the last of
    /// `own.models` after SPD adjustments, `spd.rs`).
    pub cm: &'m CompiledModel,
    pub m: &'m Model,
    /// The compiled model the run was created on.
    base: &'m CompiledModel,
    own: spd::OwnModels,
    spd: spd::SpdSide,
    /// Ids of created elements -> the reference runner's aliases (empty without changes).
    alias: HashMap<Box<str>, Box<str>>,
    /// A run error raised by an SPD adjustment.
    failure: Option<SimError>,
    pub now: f64,
    heap: BinaryHeap<Entry>,
    seq: u64,
    pub rng: SsRng,
    disposed: [bool; H_COUNT],
    pub errors: Vec<String>,
    pub meas: Measurements,
    pub mon: Monitors,
    // ids
    next_user: u64,
    next_ctx: u64,
    next_job: u64,
    // projection
    proj: Option<String>,
    lines: i64,
    users: HashMap<u64, u64>,
    jobs: HashMap<u64, u64>,
    // resources
    pub res: Vec<resource::ResRt>,
    /// Per resource: in the active-resource table (resources of containers an SPD adjustment
    /// added join it at `ModelAdjusted`, those of deleted containers leave it).
    pub res_live: Vec<bool>,
    /// `LinkingResourceTable` (built at the start, never updated).
    pub links: Vec<LinkingResourceId>,
    pub passives: Vec<resource::Passive>,
    pub passive_idx: HashMap<(PassiveResourceId, AssemblyContextId), usize>,
    /// Jobs at a resource, by id.
    pub live: HashMap<u64, Rc<Job>>,
    /// The first allocation context of each assembly context (SS-ACT-9).
    pub alloc: Vec<Option<ContainerId>>,
    pub sys: StructureId,
    pub system: SystemId,
    stopped: bool,
    /// The run is over (`SimulationFinished` dispatched, the event list empty, or an error).
    done: bool,
    events: u64,
    /// Finished usage-scenario runs (`UserFinished` events).
    finished: i64,
    same_instant: u64,
    last_t: f64,
    max_per_instant: u64,
}

impl<'m> Engine<'m> {
    fn new(cm: &'m CompiledModel, cfg: &SimConfig, out: &Outputs) -> Result<Self, SimError> {
        let m = &cm.model;
        let fail = |message: String| SimError {
            message,
            at_ns: 0,
            kind: SimErrorKind::Model,
        };
        let system = m
            .allocations
            .iter()
            .next()
            .and_then(|a| a.system)
            .or_else(|| m.systems.iter_ids().next().map(|(i, _)| i))
            .ok_or_else(|| fail("no system".into()))?;
        let sys = m.systems[system].structure;
        let alloc = spd::first_match(m);
        let rng = SsRng::new(cfg.seed, out.tape.is_some()).map_err(fail)?;
        let mut meas = Measurements {
            series: Vec::new(),
            rows: Vec::new(),
            count: 0,
        };
        let mon = monitor::build(cm, &mut meas);
        Ok(Engine {
            cm,
            m,
            base: cm,
            own: spd::OwnModels::default(),
            spd: spd::SpdSide::default(),
            alias: HashMap::new(),
            failure: None,
            now: 0.0,
            heap: BinaryHeap::new(),
            seq: 0,
            rng,
            disposed: [false; H_COUNT],
            errors: Vec::new(),
            meas,
            mon,
            next_user: 0,
            next_ctx: 0,
            next_job: 0,
            proj: out.trace.is_some().then(|| String::with_capacity(1 << 16)),
            lines: 0,
            users: HashMap::new(),
            jobs: HashMap::new(),
            res: Vec::new(),
            res_live: Vec::new(),
            links: cm.links.clone(),
            passives: Vec::new(),
            passive_idx: HashMap::new(),
            live: HashMap::new(),
            alloc,
            sys,
            system,
            stopped: false,
            done: false,
            events: 0,
            finished: 0,
            same_instant: 0,
            last_t: 0.0,
            max_per_instant: cfg.max_events_per_instant,
        })
    }

    pub fn new_user(&mut self) -> URef {
        self.next_user += 1;
        Rc::new(User {
            id: self.next_user,
            stack: RefCell::new(Vec::new()),
        })
    }

    pub fn ctx_id(&mut self) -> u64 {
        self.next_ctx += 1;
        self.next_ctx
    }

    pub fn job_id(&mut self) -> u64 {
        self.next_job += 1;
        self.next_job
    }

    /// Queues `ev` at `now + delay` (SS-ENG-2).
    #[inline]
    pub fn schedule(&mut self, ev: Ev, delay: f64) {
        self.schedule_at(ev, self.now + delay);
    }

    #[inline]
    pub fn schedule_at(&mut self, ev: Ev, t: f64) {
        let seq = self.seq;
        self.seq += 1;
        self.heap.push(Entry {
            t: t + 0.0,
            seq,
            ev,
        });
    }

    /// Runs subscriber `h` (unless disposed) and queues its results; a throw disposes it.
    #[inline]
    fn call(&mut self, h: H, f: impl FnOnce(&mut Self) -> HR) {
        if self.disposed[h as usize] {
            return;
        }
        match f(self) {
            Ok(out) => {
                for (ev, d) in out {
                    match &ev {
                        // a completion is due at the time its resource computed (`now + delay`)
                        Ev::JobProg(_, w) => {
                            let t = <simoxide_sched::F64Secs as simoxide_sched::Clock>::secs(w.at);
                            self.schedule_at(ev, t);
                        }
                        _ => self.schedule(ev, d),
                    }
                }
            }
            Err(t) => {
                self.disposed[h as usize] = true;
                self.errors.push(format!(
                    "t={}: {} in {}: {}",
                    crate::javafmt::double(self.now),
                    t.class,
                    h.name(),
                    t.message
                ));
            }
        }
    }

    /// `ResourceSimulation`'s constructor and the first events (SS-RUN-3): the run before
    /// t = 0.
    fn start(&mut self, cfg: &SimConfig) -> Result<(), SimError> {
        self.resources_init()
            .map_err(|t| self.sim_error(format!("{}: {}", t.class, t.message)))?;
        self.schedule(Ev::PreSim, 0.0);
        self.schedule(Ev::SimStarted, 0.0);
        self.schedule_at(Ev::Finished, cfg.max_sim_time as f64);
        Ok(())
    }

    /// Dispatches events until `p` pauses before the next one (between events) or the run is
    /// over. An error ends the run.
    fn advance<P: run::SsxPause>(
        &mut self,
        cfg: &SimConfig,
        mut hooks: Option<&mut dyn SpdHooks>,
        p: &mut P,
    ) -> Result<crate::Status, SimError> {
        if self.done {
            return Ok(crate::Status::Finished);
        }
        if self.stopped {
            // `ChangeHost::fail_run` between events
            self.done = true;
            return match self.failure.take() {
                Some(f) => Err(f),
                None => Ok(crate::Status::Finished),
            };
        }
        loop {
            if !P::NEVER
                && let Some(top) = self.heap.peek()
                && p.pause(self, top.t)
            {
                return Ok(crate::Status::Paused);
            }
            let e = self.heap.pop();
            let Some(e) = e else { break };
            self.now = e.t;
            if self.now == self.last_t {
                self.same_instant += 1;
                if self.max_per_instant > 0 && self.same_instant > self.max_per_instant {
                    self.done = true;
                    return Err(SimError {
                        message: format!(
                            "livelock guard: more than {} events at t = {}",
                            self.max_per_instant,
                            crate::javafmt::double(self.now)
                        ),
                        at_ns: simoxide_sched::time::span(self.now),
                        kind: SimErrorKind::Limit,
                    });
                }
            } else {
                self.last_t = self.now;
                self.same_instant = 0;
            }
            self.events += 1;
            self.dispatch(e.ev, &mut hooks);
            if self.stopped {
                self.done = true;
                if let Some(f) = self.failure.take() {
                    return Err(f);
                }
                break;
            }
            if cfg.limits.max_events > 0 && self.events >= cfg.limits.max_events {
                self.done = true;
                return Err(SimError {
                    message: format!("event limit {} reached", cfg.limits.max_events),
                    at_ns: simoxide_sched::time::span(self.now),
                    kind: SimErrorKind::Limit,
                });
            }
        }
        self.done = true;
        Ok(crate::Status::Finished)
    }

    fn sim_error(&self, message: String) -> SimError {
        SimError {
            message,
            at_ns: simoxide_sched::time::span(self.now),
            kind: SimErrorKind::Model,
        }
    }

    /// Dispatches one event: its projected line, then its subscribers in subscription order.
    fn dispatch(&mut self, ev: Ev, hooks: &mut Option<&mut dyn SpdHooks>) {
        self.project(&ev);
        self.rng.pos = self.lines - 1;
        match ev {
            Ev::PreSim => {
                // SpdBehavior is subscribed before the monitor interpreter (SS-SPD-1.2)
                if let Some(h) = hooks.as_deref_mut() {
                    self.call(H::SpdStart, |e| e.spd_start(h));
                }
            }
            Ev::SimStarted => self.call(H::UsageStart, |e| e.on_simulation_start()),
            Ev::Finished => {
                // ResourceSimulation.onSimulationFinished clears the queues, CoreBehavior stops
                self.stopped = true;
            }
            Ev::UPassed(c, a) => {
                self.call(H::MonUsageStart, |e| e.mon_usage_started(&c, a));
                self.call(H::MonUsageStop, |e| e.mon_usage_finished(&c, a));
            }
            Ev::UserStarted(c) => self.call(H::UserStarted, |e| e.on_user_started(&c)),
            Ev::InterArrival(c) => self.call(H::InterArrival, |e| e.do_switch_user(&c, c.action)),
            Ev::WokeUp(c) => self.call(H::WokeUp, |e| e.do_switch_user(&c, c.action)),
            Ev::ReqFinished(r, c) => {
                self.call(H::ReqFinished, |e| e.on_user_request_finished(&r, &c))
            }
            Ev::UserFinished(c) => {
                self.finished += 1;
                self.call(H::UserFinished, |e| e.on_user_finished(&c))
            }
            Ev::UserAborted(c) => self.call(H::UserAborted, |e| e.on_user_aborted(&c)),
            Ev::ClosedInit(c) => self.call(H::ClosedInit, |e| e.on_closed_initiated(&c)),
            Ev::InnerInit(c) => self.call(H::InnerInit, |e| e.do_switch_user(&c, c.action)),
            Ev::UProgressed(c) => self.call(H::UProgressed, |e| e.do_switch_user(&c, c.action)),
            Ev::EntryReq(r, c) => self.call(H::SysEntry, |e| e.on_user_entry_requested(&r, &c)),
            Ev::SProgressed(c) => self.call(H::SeffProgressed, |e| e.on_seff_progressed(&c)),
            Ev::SChild(c) => self.call(H::SeffChild, |e| e.on_seff_child_started(&c)),
            Ev::SFinished(c) => self.call(H::SeffFinished, |e| e.on_seff_finished(&c)),
            Ev::SPassed(c, a) => {
                self.call(H::MonOpStart, |e| e.mon_op_started(&c, a));
                self.call(H::MonOpStop, |e| e.mon_op_stopped(&c, a));
            }
            Ev::ExtCalled(g) => self.call(H::SysExternal, |e| e.request_call_over_wire(&g)),
            Ev::InfraCalled(g) => self.call(H::SysInfra, |e| e.on_infra_called(&g)),
            Ev::CowReq(c) => self.call(H::RsCowReq, |e| e.on_cow_requested(&c)),
            Ev::CowOk(c) => self.call(H::SysCowOk, |e| e.on_cow_succeeded(&c)),
            Ev::CowAbort(c) => self.call(H::SysCowAbort, |e| e.on_cow_aborted(&c)),
            Ev::RdReq(r) => {
                self.call(H::MonPassiveReq, |e| e.mon_passive_requested(&r));
                self.call(H::RsRdReq, |e| e.on_demand_requested(&r));
            }
            Ev::RdAbort(r) => self.call(H::SysRdAbort, |e| e.on_demand_aborted(&r)),
            Ev::ArFinished(r) => self.call(H::SysArFinished, |e| e.on_active_finished(&r)),
            Ev::JobInit(j) => self.call(H::RsJobInit, |e| e.on_job_initiated(&j)),
            Ev::JobProg(j, w) => self.call(H::RsJobProg, |e| e.on_job_progressed(&j, &w)),
            Ev::JobFin(j) => self.call(H::RsJobFin, |e| Ok(e.on_job_finished(&j))),
            Ev::JobAbort(j) => self.call(H::RsJobAbort, |e| Ok(e.on_job_aborted(&j))),
            Ev::ArState(r, n, u) => self.call(H::MonActiveState, |e| e.mon_active_state(r, n, u)),
            Ev::ArDemand(r, v) => self.call(H::MonActiveDemand, |e| e.mon_active_demand(r, v)),
            Ev::PAcq(r) => {
                self.call(H::MonPassiveAcq, |e| e.mon_passive_acquired(&r));
                self.call(H::SeffPassiveAcq, |e| e.on_passive_acquired(&r));
            }
            Ev::PRel(r) => {
                self.call(H::MonPassiveRel, |e| e.mon_passive_released(&r));
                self.call(H::RsPassiveRel, |e| e.on_passive_released(&r));
            }
            Ev::PState(r, a) => self.call(H::MonPassiveState, |e| e.mon_passive_state(&r, a)),
            Ev::Meas(s, t, v) => {
                if self.spd.measured
                    && let Some(h) = hooks.as_deref_mut()
                {
                    self.spd_measurement(h, s, t, v);
                }
            }
            Ev::SpdTime(i) => {
                if let Some(h) = hooks.as_deref_mut() {
                    let now = self.now;
                    self.spd_subscribers(h, spd::Stimulus::Time, |_, h, k| {
                        h.on_time(k, i as usize, now)
                    });
                }
            }
            Ev::SpdReq(r, _) => {
                if let Some(h) = hooks.as_deref_mut() {
                    self.call(H::SpdAdjust, |e| e.spd_adjust(h, r));
                }
            }
            Ev::ModelAdjusted(a) => {
                self.call(H::NoeResource, |_| Ok(Out::new()));
                self.call(H::RsModelAdjusted, |e| e.rs_model_adjusted(&a));
                self.call(H::NoeSystem, |_| Ok(Out::new()));
                self.call(H::MonModelAdjusted, |e| e.mon_model_adjusted(&a));
            }
        }
    }

    // --------------------------------------------------------------------------- evaluation

    /// `StackContext.evaluateStatic(spec, frame)`: the raw value, drawing at `site`.
    pub fn eval(
        &mut self,
        prog: ProgId,
        frame: Option<&SFrame>,
        site: &'static str,
    ) -> Result<Value, Throw> {
        let cm = self.cm;
        let p = &cm.progs[prog as usize];
        if let Some(v) = &p.konst {
            return Ok(v.clone());
        }
        let prev = std::mem::replace(&mut self.rng.site, site);
        let r = match &p.prog {
            Ok(pr) => pr.eval(&SEnv { frame, cm }, &mut self.rng),
            Err(e) => Err(simoxide_stoex::EvalError::new(
                simoxide_stoex::EvalErrorKind::Runtime,
                e.clone(),
            )),
        };
        self.rng.site = prev;
        r.map_err(|e| Throw {
            class: "StochasticExpressionEvaluationFailedException",
            message: format!("Evaluation of expression {} failed: {:?}", p.spec, e),
        })
    }

    pub fn eval_f64(
        &mut self,
        prog: ProgId,
        frame: Option<&SFrame>,
        site: &'static str,
    ) -> Result<f64, Throw> {
        let v = self.eval(prog, frame, site)?;
        v.to_f64().map_err(conv_throw)
    }

    pub fn eval_i32(
        &mut self,
        prog: ProgId,
        frame: Option<&SFrame>,
        site: &'static str,
    ) -> Result<i32, Throw> {
        let v = self.eval(prog, frame, site)?;
        v.to_i32().map_err(conv_throw)
    }

    pub fn eval_i64(
        &mut self,
        prog: ProgId,
        frame: Option<&SFrame>,
        site: &'static str,
    ) -> Result<i64, Throw> {
        let v = self.eval(prog, frame, site)?;
        match v.convert(simoxide_stoex::Expected::Long) {
            Ok(simoxide_stoex::Converted::Long(l)) => Ok(l),
            Ok(_) => unreachable!(),
            Err(e) => Err(conv_throw(e)),
        }
    }

    pub fn eval_bool(
        &mut self,
        prog: ProgId,
        frame: Option<&SFrame>,
        site: &'static str,
    ) -> Result<bool, Throw> {
        let v = self.eval(prog, frame, site)?;
        v.to_bool().map_err(conv_throw)
    }

    /// `SimulatedStackHelper.addParameterToStackFrame(context, usages, target)` (ACT-5.2).
    pub fn add_parameters(
        &mut self,
        context: Option<&SFrame>,
        usages: &[CChar],
        target: &SFrame,
        site: &'static str,
    ) -> Result<(), Throw> {
        for c in usages {
            let b = if c.inner {
                Binding::Proxy(crate::frames::ProxyRef::new(crate::frames::Proxy {
                    prog: c.prog,
                    // `contextStackFrame.copyFrame()` (a null context throws in Java)
                    frame: match context {
                        Some(f) => Some(frame::snapshot(f)),
                        None => {
                            return throw(
                                "NullPointerException",
                                "addParameterToStackFrame: no context frame for an INNER usage",
                            );
                        }
                    },
                }))
            } else {
                Binding::Val(self.eval(c.prog, context, site)?)
            };
            target.borrow_mut().put(c.key, b);
        }
        Ok(())
    }

    /// `SimulatedStackHelper.createAndPushNewStackFrame(stack, usages, parent)`.
    pub fn push_new_frame(
        &mut self,
        user: &URef,
        usages: &[CChar],
        parent: Option<SFrame>,
        site: &'static str,
    ) -> Result<SFrame, Throw> {
        let f = frame::new_frame(parent);
        let ctx = user.stack.borrow().last().cloned();
        self.add_parameters(ctx.as_ref(), usages, &f, site)?;
        user.stack.borrow_mut().push(f.clone());
        Ok(f)
    }

    /// `stack.currentStackFrame()` (`Stack.peek`).
    pub fn top(&self, user: &URef) -> Result<SFrame, Throw> {
        match user.stack.borrow().last() {
            Some(f) => Ok(f.clone()),
            None => throw("EmptyStackException", "null"),
        }
    }

    /// `stack.removeStackFrame()` (`Stack.pop`).
    pub fn pop(&self, user: &URef) -> Result<(), Throw> {
        match user.stack.borrow_mut().pop() {
            Some(_) => Ok(()),
            None => throw("EmptyStackException", "null"),
        }
    }

    /// `NumberConverter.toDouble` of a frame value.
    pub fn binding_to_f64(&mut self, b: &Binding) -> Result<f64, Throw> {
        let v = match b {
            Binding::Val(v) => v.clone(),
            Binding::Proxy(p) => {
                crate::frames::eval_proxy(self.cm, p, &mut self.rng).map_err(|e| Throw {
                    class: "StochasticExpressionEvaluationFailedException",
                    message: format!("{e:?}"),
                })?
            }
        };
        match v {
            Value::Int(i) => Ok(f64::from(i)),
            Value::Double(d) => Ok(d),
            other => throw(
                "NumberFormatException",
                format!("NumberConverter: cannot convert {other:?}"),
            ),
        }
    }

    // --------------------------------------------------------------------------- projection

    fn user_no(&mut self, u: &User) -> u64 {
        let n = self.users.len() as u64 + 1;
        *self.users.entry(u.id).or_insert(n)
    }

    fn job_no(&mut self, id: u64) -> u64 {
        let n = self.jobs.len() as u64 + 1;
        *self.jobs.entry(id).or_insert(n)
    }

    /// The level-E projection line of an event at its dispatch (SS-X-2).
    fn project(&mut self, ev: &Ev) {
        if self.proj.is_none() {
            if is_projected(ev) {
                self.lines += 1;
            }
            return;
        }
        let m = self.m;
        let mut l = Line::new();
        match ev {
            Ev::UPassed(c, a) => {
                l.head("pass", self.now);
                let u = self.user_no(&c.user);
                l.int("u", u);
                l.str("type", uaction_type(&m.user_actions[*a].kind));
                l.str("el", self.key(&m.user_actions[*a].id));
            }
            Ev::SPassed(c, a) => {
                l.head("pass", self.now);
                let u = self.user_no(&c.rpc.user);
                l.int("u", u);
                l.str("ac", self.key(&m.assembly_contexts[c.ac].id));
                l.str("type", action_type(&m.actions[*a].kind));
                l.str("el", self.key(&m.actions[*a].id));
            }
            Ev::EntryReq(r, _) | Ev::ReqFinished(r, _) => {
                l.head(
                    if matches!(ev, Ev::EntryReq(..)) {
                        "call"
                    } else {
                        "call_end"
                    },
                    self.now,
                );
                let u = self.user_no(&r.user);
                l.int("u", u);
                if let Some(x) = r.role {
                    l.str("role", self.key(&m.roles[x].id));
                }
                if let Some(x) = r.sig {
                    l.str("sig", self.key(&m.signatures[x].id));
                }
            }
            Ev::ExtCalled(g) | Ev::InfraCalled(g) => {
                l.head(
                    if matches!(ev, Ev::ExtCalled(..)) {
                        "ext"
                    } else {
                        "infra"
                    },
                    self.now,
                );
                let u = self.user_no(&g.from.rpc.user);
                l.int("u", u);
                l.str("ac", self.key(&m.assembly_contexts[g.from.ac].id));
                if let Some(x) = g.role {
                    l.str("role", self.key(&m.roles[x].id));
                }
                if let Some(x) = g.sig {
                    l.str("sig", self.key(&m.signatures[x].id));
                }
            }
            Ev::RdReq(r) => {
                l.head("request", self.now);
                let u = self.user_no(&r.ctx.rpc.user);
                l.int("u", u);
                l.str("ac", self.key(&m.assembly_contexts[r.ac].id));
                l.str("kind", if r.active { "ACTIVE" } else { "PASSIVE" });
                if let Some(d) = m.actions[r.action].resource_demands.get(r.demand) {
                    if let Some(t) = d.resource_type {
                        l.str("res", self.key(&m.resource_types[t].id));
                    }
                    l.str("action", self.key(&m.actions[r.action].id));
                }
                if let Some(p) = r.passive {
                    l.str("passive", self.key(&m.passive_resources[p].id));
                }
            }
            Ev::JobInit(j) => {
                l.head("job", self.now);
                let n = self.job_no(j.id);
                l.int("job", n);
                match &j.kind {
                    JobKind::Active {
                        rtype,
                        req,
                        container,
                    } => {
                        let u = self.user_no(&req.ctx.rpc.user);
                        l.int("u", u);
                        if let Some(c) = container {
                            l.str("rc", self.key(&m.containers[*c].id));
                        }
                        if let Some(t) = rtype {
                            l.str("res", self.key(&m.resource_types[*t].id));
                        }
                    }
                    JobKind::Link { link, .. } => {
                        l.str("link", self.key(&m.linking_resources[*link].id));
                    }
                }
                l.num("d", j.demand.get());
            }
            Ev::JobFin(j) | Ev::JobAbort(j) => {
                l.head(
                    if matches!(ev, Ev::JobFin(..)) {
                        "job_end"
                    } else {
                        "job_abort"
                    },
                    self.now,
                );
                let n = self.job_no(j.id);
                l.int("job", n);
            }
            Ev::PAcq(r) | Ev::PRel(r) => {
                l.head(
                    if matches!(ev, Ev::PAcq(..)) {
                        "grant"
                    } else {
                        "release"
                    },
                    self.now,
                );
                let u = self.user_no(&r.ctx.rpc.user);
                l.int("u", u);
                l.str("ac", self.key(&m.assembly_contexts[r.ac].id));
                if let Some(p) = r.passive {
                    l.str("passive", self.key(&m.passive_resources[p].id));
                }
            }
            Ev::UserFinished(c) | Ev::UserAborted(c) => {
                l.head(
                    if matches!(ev, Ev::UserFinished(..)) {
                        "user_end"
                    } else {
                        "user_abort"
                    },
                    self.now,
                );
                let u = self.user_no(&c.user);
                l.int("u", u);
            }
            Ev::Meas(s, t, v) => {
                l.head("meas", self.now);
                let def = &self.meas.series[*s as usize];
                l.str("mp", &def.mp);
                l.str("metric", def.metric);
                l.s.push_str(",\"v\":[");
                push_json_double(&mut l.s, *t);
                l.s.push(',');
                push_json_double(&mut l.s, *v);
                l.s.push(']');
            }
            Ev::SpdTime(i) => {
                l.head("spd_time", self.now);
                l.str("tg", self.spd_tg(*i));
            }
            Ev::SpdReq(_, policy) => {
                l.head("spd_request", self.now);
                l.str("policy", policy);
            }
            Ev::ModelAdjusted(a) => {
                l.head("spd_adjusted", self.now);
                Self::project_adjusted(a, &mut l);
            }
            Ev::Finished => l.head("stop", self.now),
            _ => return,
        }
        l.s.push_str("}\n");
        self.lines += 1;
        if let Some(p) = self.proj.as_mut() {
            p.push_str(&l.s);
        }
    }

    /// Records a measurement (EDP2 row, synchronously) and queues its `MeasurementMade`
    /// (SS-MEAS-1.2, SS-BUS-5).
    pub fn measure(&mut self, s: SeriesId, t: f64, v: f64) {
        self.meas.rows[s as usize].push((t, v));
        self.meas.count += 1;
        self.schedule(Ev::Meas(s, t, v), 0.0);
    }
}

fn conv_throw(e: simoxide_stoex::EvalError) -> Throw {
    Throw {
        class: "ClassCastException",
        message: format!("{e:?}"),
    }
}

fn is_projected(ev: &Ev) -> bool {
    matches!(
        ev,
        Ev::UPassed(..)
            | Ev::SPassed(..)
            | Ev::EntryReq(..)
            | Ev::ReqFinished(..)
            | Ev::ExtCalled(..)
            | Ev::InfraCalled(..)
            | Ev::RdReq(..)
            | Ev::JobInit(..)
            | Ev::JobFin(..)
            | Ev::JobAbort(..)
            | Ev::PAcq(..)
            | Ev::PRel(..)
            | Ev::UserFinished(..)
            | Ev::UserAborted(..)
            | Ev::Meas(..)
            | Ev::SpdTime(..)
            | Ev::SpdReq(..)
            | Ev::ModelAdjusted(..)
            | Ev::Finished
    )
}

/// A projection line under construction.
struct Line {
    s: String,
}

impl Line {
    fn new() -> Line {
        Line {
            s: String::with_capacity(160),
        }
    }
    fn head(&mut self, ev: &str, t: f64) {
        self.s.push_str("{\"ev\":\"");
        self.s.push_str(ev);
        self.s.push_str("\",\"t\":");
        push_json_double(&mut self.s, t);
    }
    fn str(&mut self, k: &str, v: &str) {
        self.s.push_str(",\"");
        self.s.push_str(k);
        self.s.push_str("\":");
        push_json_str(&mut self.s, v);
    }
    fn int(&mut self, k: &str, v: u64) {
        self.s.push_str(",\"");
        self.s.push_str(k);
        self.s.push_str("\":");
        self.s.push_str(&v.to_string());
    }
    fn num(&mut self, k: &str, v: f64) {
        self.s.push_str(",\"");
        self.s.push_str(k);
        self.s.push_str("\":");
        push_json_double(&mut self.s, v);
    }
}

/// The projected lines of level M (SS-X-2): the subset of a level-E projection.
pub fn level_m(projection: &str) -> String {
    projection
        .lines()
        .filter(|l| {
            [
                "{\"ev\":\"job_abort\"",
                "{\"ev\":\"user_abort\"",
                "{\"ev\":\"meas\"",
                "{\"ev\":\"spd_",
                "{\"ev\":\"stop\"",
            ]
            .iter()
            .any(|p| l.starts_with(p))
        })
        .map(|l| format!("{l}\n"))
        .collect()
}

/// Runs a model compiled for [`FlavourKind::SlingshotExact`] (see the module docs). Uses
/// `cfg.seed` and `cfg.max_sim_time` (Slingshot's `simTime`, whole seconds, required); the
/// mode, the maximum measurement count (Q32) and the linking-resource options (SS-ACT-7.2) are
/// not read.
pub fn run(cm: &CompiledModel, cfg: SimConfig, out: Outputs) -> Result<RunResult, SimError> {
    run_impl(cm, cfg, out, None)
}

/// [`run`] with an SPD layer (`DESIGN-SPD.md` X2d, [`spd`]): its time events, policy
/// subscribers and adjustments run on Slingshot's bus.
pub fn run_with_spd(
    cm: &CompiledModel,
    cfg: SimConfig,
    out: Outputs,
    hooks: &mut dyn SpdHooks,
) -> Result<RunResult, SimError> {
    run_impl(cm, cfg, out, Some(hooks))
}

fn run_impl(
    cm: &CompiledModel,
    cfg: SimConfig,
    out: Outputs,
    hooks: Option<&mut dyn SpdHooks>,
) -> Result<RunResult, SimError> {
    SsxRun::new(cm, cfg, out)?.finish(hooks)
}

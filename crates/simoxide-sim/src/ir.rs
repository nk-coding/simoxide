//! Compilation of a loaded PCM model into the flat, index-based form the interpreter runs on.
//!
//! Everything that does not depend on the run-time state is resolved here: behaviours become
//! action chains, StoEx strings become [`simoxide_stoex::Program`]s whose variables are resolved to frame
//! key ids ([`Keys`]), connectors are indexed by the lookups the interpreter performs
//! (`ComposedStructureInnerSwitch`, `RepositoryComponentSwitch`), the allocation becomes a map from
//! assembly-context paths to containers, and monitors become measurement series.
//!
//! A [`CompiledModel`] is immutable and `Sync`; many simulations can run on it in parallel.

use crate::fxhash::FxMap;
use crate::stats::{Stat, StatKind};
use simoxide_jcompat::hash::str_hash as string_hash;
use simoxide_jcompat::order::{InsertionOrder, MapOrder};
#[cfg(feature = "exact")]
use simoxide_jcompat_exact::order::JdkHashOrder;
use simoxide_model::raw::ObjId;
use simoxide_model::*;
use simoxide_sched::SchedulingPolicy;
use simoxide_stoex::Program;
use std::collections::HashMap;
use std::fmt;

/// A name or message the engine holds as a constant (metric names, action kinds, errors).
/// (A type alias: snapshots write the text and intern it when they read it back.)
pub type StaticStr = &'static str;

/// Index of a compiled StoEx program.
pub type ProgId = u32;
/// Index of a stack-frame key (`a.VALUE`, ...).
pub type KeyId = u32;
/// Index of an active resource (processing resource or linking resource).
pub type ResIdx = u32;
/// Index of a measurement series.
pub type SeriesId = u32;
/// Assembly context on the interpreter's assembly-context stack: an [`AssemblyContextId`]
/// index, or [`SYSTEM_AC`] for SimuLizar's synthetic system assembly context.
pub type AcRef = u32;
/// The synthetic system assembly context (`_SYSTEM_ASSEMBLY_CONTEXT_`).
pub const SYSTEM_AC: AcRef = u32::MAX;
/// Its id in traces (`docs/reference-simulator/patches.md` P7).
pub const SYSTEM_AC_ID: &str = "_SYSTEM_ASSEMBLY_CONTEXT_";

/// Metric description ids (`commonMetrics.metricspec`).
pub mod metric_ids {
    pub const RESPONSE_TIME: &str = "_6rYmYs7nEeOX_4BzImuHbA";
    pub const HOLDING_TIME: &str = "_zETOUs7pEeOX_4BzImuHbA";
    pub const WAITING_TIME: &str = "_QWjAYs7qEeOX_4BzImuHbA";
    pub const RESOURCE_DEMAND: &str = "_eg_F0s7qEeOX_4BzImuHbA";
    pub const STATE_OF_ACTIVE_RESOURCE: &str = "_paDhIs7qEeOX_4BzImuHbA";
    pub const STATE_OF_PASSIVE_RESOURCE: &str = "_x0-pks7rEeOX_4BzImuHbA";
    pub const UTILIZATION_TUPLE: &str = "_mhws4SkUEeSuf8LV7cHLgA";
    pub const UTILIZATION: &str = "_QIb6cikUEeSuf8LV7cHLgA";
    pub const RECONFIGURATION_TIME: &str = "_VYg6MujFEeSB6OBq2SKZxQ";
    pub const NUMBER_OF_RESOURCE_CONTAINERS: &str = "_e7x3gq-eEeSgL6DrxYuwZg";
    /// `Execution Result Type over Time` (reliability extension, MEAS-7.4).
    pub const EXECUTION_RESULT_TYPE_TUPLE: &str = "_-TkoURX7Eey-ibmvVnJ8rg";
}

/// Names of the tuple metrics as written to `measurements.csv`.
pub mod metric_names {
    pub const RESPONSE_TIME: &str = "Response Time Tuple";
    pub const RESOURCE_DEMAND: &str = "Resource Demand Tuple";
    pub const STATE_OF_ACTIVE_RESOURCE: &str = "State of Active Resource Tuple";
    pub const UTILIZATION: &str = "Utilization of Active Resource Tuple";
    pub const WAITING_TIME: &str = "Waiting Time Tuple";
    pub const HOLDING_TIME: &str = "Holding Time Tuple";
    pub const STATE_OF_PASSIVE_RESOURCE: &str = "State of Passive Resource Tuple";
    pub const RECONFIGURATION_TIME: &str = "Reconfiguration Time Tuple";
    pub const NUMBER_OF_RESOURCE_CONTAINERS: &str = "Number of Resource Containers over Time";
}

/// Error while compiling a model (the reference would fail before or at the start of the run).
#[derive(Debug, Clone)]
pub struct CompileError(pub String);

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for CompileError {}

/// Interned stack-frame keys (`serialise(reference) + "." + characterisation`).
#[derive(Debug, Default, Clone)]
pub struct Keys {
    pub names: Vec<Box<str>>,
    /// `String.hashCode()` of each key (HashMap order emulation).
    pub hash: Vec<i32>,
    /// Key ends with `BYTESIZE` (network payload).
    pub bytesize: Vec<bool>,
    map: HashMap<Box<str>, KeyId>,
}

impl Keys {
    pub fn intern(&mut self, s: &str) -> KeyId {
        if let Some(&k) = self.map.get(s) {
            return k;
        }
        let k = self.names.len() as KeyId;
        self.names.push(s.into());
        self.hash.push(string_hash(s));
        self.bytesize.push(s.ends_with("BYTESIZE"));
        self.map.insert(s.into(), k);
        k
    }
    pub fn get(&self, s: &str) -> Option<KeyId> {
        self.map.get(s).copied()
    }
}

/// A compiled StoEx (one per distinct specification string, like `StoExCache`).
#[derive(Debug, Clone)]
pub struct Prog {
    pub spec: Box<str>,
    /// Parse/type errors are raised when the expression is evaluated (as `StoExCache.getEntry`).
    pub prog: Result<Program, String>,
    /// The value, if the whole expression folded to a constant (evaluation draws nothing).
    pub konst: Option<simoxide_stoex::Value>,
    /// The constant as a `Double` result (`evaluateStatic(spec, Double.class)`), if it converts.
    pub konst_f64: Option<f64>,
}

/// One characterisation of a variable usage, flattened (usage order, then characterisation
/// order).
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct CChar {
    pub key: KeyId,
    pub prog: ProgId,
    /// The reference contains `INNER`: stored as a lazily evaluated proxy (ACT-5.2).
    pub inner: bool,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct CInfraCall {
    pub id: Box<str>,
    pub count: ProgId,
    pub role: Option<RoleId>,
    pub signature: Option<SignatureId>,
    pub inputs: Vec<CChar>,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct CResCall {
    pub id: Box<str>,
    pub count: ProgId,
    /// Resource type (the last type providing the called interface), if found.
    pub resource_type: Option<ResourceTypeId>,
    /// `resourceServiceId` of the called resource signature.
    pub service_id: i64,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub enum CAct {
    Start,
    Stop,
    Internal {
        demands: Vec<(ProgId, Option<ResourceTypeId>)>,
        infra: Vec<CInfraCall>,
        rescalls: Vec<CResCall>,
    },
    External {
        role: Option<RoleId>,
        signature: Option<SignatureId>,
        inputs: Vec<CChar>,
        returns: Vec<CChar>,
        series: Option<SeriesId>,
    },
    ProbBranch {
        cum: Vec<f64>,
        behaviours: Vec<Option<BehaviourId>>,
        sels: Vec<Box<str>>,
    },
    GuardBranch {
        guards: Vec<ProgId>,
        guard_ids: Vec<Box<str>>,
        behaviours: Vec<Option<BehaviourId>>,
        sels: Vec<Box<str>>,
    },
    EmptyBranch,
    Loop {
        count: ProgId,
        body: Option<BehaviourId>,
    },
    Collection {
        count: ProgId,
        /// `<param>.` (INNER lookup prefix)
        prefix: Box<str>,
        body: Option<BehaviourId>,
    },
    Fork {
        asynchronous: Vec<BehaviourId>,
        synchronous: Vec<BehaviourId>,
    },
    Acquire(Option<PassiveResourceId>),
    Release(Option<PassiveResourceId>),
    SetVariable(Vec<CChar>),
    /// `RecoveryAction` without failure simulation: its primary behaviour
    /// (`NOPReliabilityInterpreter`).
    Recovery(Option<BehaviourId>),
    Unsupported(String),
}

/// A `ResourceDemandingBehaviour` as the interpreter walks it.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct CBeh {
    /// First `StartAction` of the steps.
    pub start: Option<ActionId>,
    /// Actions after the start, following `successor` up to (excluding) the `StopAction`.
    pub chain: Vec<ActionId>,
    /// The chain ended at a `StopAction` (else the reference throws after the last action).
    pub ends_at_stop: bool,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub enum CUAct {
    Start,
    Stop,
    Elsc {
        role: Option<RoleId>,
        signature: Option<SignatureId>,
        inputs: Vec<CChar>,
        outputs: Vec<CChar>,
        series: Option<SeriesId>,
        sysop_series: Option<SeriesId>,
    },
    Delay(ProgId),
    Branch {
        cum: Vec<f64>,
        behaviours: Vec<Option<ScenarioBehaviourId>>,
        sels: Vec<Box<str>>,
    },
    Loop {
        count: ProgId,
        body: Option<ScenarioBehaviourId>,
    },
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct CUBeh {
    pub start: Option<UserActionId>,
    pub chain: Vec<UserActionId>,
    pub ends_at_stop: bool,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub enum CWorkload {
    Open { inter_arrival: ProgId },
    Closed { population: i64, think_time: ProgId },
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct CScenario {
    pub id: UsageScenarioId,
    pub behaviour: Option<ScenarioBehaviourId>,
    pub workload: CWorkload,
    pub series: Option<SeriesId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub enum ResKind {
    Processing(ProcessingResourceId),
    Link(LinkingResourceId),
}

/// An active resource of the simulation (`ScheduledResource` / `SimulatedLinkingResource`).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct CRes {
    pub kind: ResKind,
    pub policy: SchedulingPolicy,
    /// `numberOfInstances` (the model's number of replicas; 1 for links).
    pub instances: u32,
    /// Processing rate (processing resources) or throughput (links).
    pub rate: ProgId,
    /// Latency (links).
    pub latency: Option<ProgId>,
    /// HDD resources (`HDDResource`): read and write processing rates.
    pub hdd: Option<(ProgId, ProgId)>,
    /// Trace fields.
    pub type_id: Box<str>,
    pub rc: Box<str>,
    pub spec_name: Box<str>,
    pub sched_name: Box<str>,
    /// Tape origin of rate/throughput/latency draws: `rate:<rc>`.
    pub origin: Box<str>,
    /// Monitoring: state series per instance.
    /// State-of-active-resource series listening on each instance (in registration order; the
    /// monitors of all replicas of FCFS/DELAY resources listen on instance 0).
    pub state_series: Vec<Vec<SeriesId>>,
    pub overall_series: Option<SeriesId>,
    pub demand_series: Vec<SeriesId>,
    /// Init-time measurement order: (series, instance) for state, in creation order.
    pub initial_states: Vec<(SeriesId, u32)>,
}

/// A node of the assembly-context path trie: the FQ path (without the system AC) that ends with
/// `ac` below the path `parent`. Node 0 is the empty path (the system level).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct AcPath {
    pub parent: u32,
    pub ac: AcRef,
    /// Allocation of the path (`alloc`), if any.
    pub container: Option<ContainerId>,
}

/// Id of the empty assembly-context path.
pub const ROOT_PATH: u32 = 0;

/// An entry of the resource registry ([`CompiledModel::registry`]): a container or linking
/// resource id with its resources by resource type id.
pub type RegistryEntry = (Box<str>, Vec<(Box<str>, ResIdx)>);

/// Measurement series: measuring point key and tuple metric name.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct SeriesDef {
    pub mp: Box<str>,
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::interned")
    )]
    pub metric: StaticStr,
}

/// A sliding window (utilisation or time-driven aggregation, MEAS-6/MEAS-7.1), in creation order.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct CWindow {
    pub len: f64,
    pub inc: f64,
    /// Series whose tuples the window accepts (`None`: no EDP2-visible data, e.g.
    /// `TimeDrivenAggregation`, which only adds its periodic events).
    pub input: Option<SeriesId>,
    /// Series the aggregated utilisation tuples are written to.
    pub out: Option<SeriesId>,
    /// A PRM recorder (`triggersSelfAdaptations`) receives every aggregated value (MEAS-7.2).
    pub prm: bool,
    /// The runtime measurement it writes ([`CompiledModel::prm_meas`]; `u32::MAX`: none).
    pub prm_meas: u32,
    /// `TimeDrivenAggregation`: the statistical characterisation of the window's data (the
    /// tuples of `input`, the calculator of the specification's metric), MEAS-7.5; `None`: a
    /// utilisation window.
    pub stat: Option<Stat>,
}

/// A runtime measurement of the runtime measurement model (PRM, SimuLizar's
/// `RuntimeMeasurement`, D2): one per PRM recorder, in the order SimuLizar creates them
/// (MEAS-7.5). The run state holds its value.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct PrmMeas {
    /// The measuring point of the specification's monitor (graph object).
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::opt_objid")
    )]
    pub mp: Option<ObjId>,
    /// The measurement specification (graph object).
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::objid")
    )]
    pub spec: ObjId,
    /// The measuring point's series key and the metric (informational).
    pub key: Box<str>,
}

/// A recorder writing into the runtime measurement model (PRM): a measurement specification
/// with `triggersSelfAdaptations = true` (MEAS-7.2). Its writes only matter for the
/// `Reconfigurator`, which creates the reconfiguration process at the first write after t = 0.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub enum PrmRec {
    /// `FeedThroughRecorder`: every tuple.
    FeedThrough,
    /// `FixedSizeMeasurementsAggregator`: every `freq`-th tuple once `n` are buffered; the
    /// value is `stat` of the buffer.
    Fixed { freq: i64, n: i64, stat: Stat },
    /// `VariableSizeMeasurementAggregator`: every `freq`-th tuple once the buffer spans
    /// `retro` seconds; `continuous`: scope of validity of the metric (eviction keeps the last
    /// tuple before the interval); the value is `stat` of the buffer.
    Variable {
        freq: i64,
        retro: f64,
        continuous: bool,
        stat: Stat,
    },
}

/// PRM recorders of lazily created passive-resource calculators, matched by the measuring
/// point's string representation (`DeferredMeasurementInitialization`).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct PassivePrm {
    /// `Passive Resource: <assembly context name>.<passive resource name>`.
    pub rep: Box<str>,
    /// Tuple metric name (state, waiting or holding time).
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::interned")
    )]
    pub metric: StaticStr,
    pub rec: PrmRec,
    /// The aggregator's constructor fails with this error when the calculator appears.
    pub invalid: Option<Box<str>>,
    /// The runtime measurement the recorder adds to the PRM when it is created.
    pub meas: PrmMeas,
}

/// Aggregating the `Long` values of the passive-resource state fails (`ClassCastException`).
pub const LONG_AGGREGATION_ERROR: &str =
    "ClassCastException: class java.lang.Long cannot be cast to class java.lang.Double";

/// Passive-resource monitoring flags (lazily created calculators, MEAS-5.1).
#[derive(Debug, Clone, Copy, Default)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct PassiveMon {
    pub state: bool,
    pub waiting: bool,
    pub holding: bool,
}

/// The compiled, immutable model.
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct CompiledModel {
    /// (A snapshot rebuilds it from the model graph and the build order.)
    #[cfg_attr(feature = "snapshot", serde(skip))]
    pub model: Model,
    pub keys: Keys,
    pub progs: Vec<Prog>,
    pub beh: Vec<CBeh>,
    pub act: Vec<CAct>,
    pub ubeh: Vec<CUBeh>,
    pub uact: Vec<CUAct>,
    pub scenarios: Vec<CScenario>,
    pub system: SystemId,
    /// `(structure, outer provided role)` -> first ProvidedDelegationConnector.
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::sorted")
    )]
    pub prov_deleg: FxMap<(StructureId, RoleId), ConnectorId>,
    /// `(assembly context, required role)` -> first matching connector of its parent structure.
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::sorted")
    )]
    pub req_conn: FxMap<(AssemblyContextId, RoleId), ConnectorId>,
    /// `(component, signature id key)` -> SEFFs describing that signature (by id string).
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::sorted")
    )]
    pub seff_for: FxMap<(ComponentId, u32), Vec<SeffId>>,
    /// Signature -> key of its id string (signatures are matched by id).
    pub sig_key: Vec<u32>,
    /// FQ assembly-context path (without the system AC) -> container.
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::sorted")
    )]
    pub alloc: FxMap<Box<[u32]>, ContainerId>,
    /// Every FQ assembly-context path reachable from the system (nesting of composite
    /// structures), as a trie; see [`CompiledModel::path_child`].
    pub ac_paths: Vec<AcPath>,
    /// Per assembly context: `(parent path, child path)` pairs of the trie.
    pub ac_path_children: Vec<Vec<(u32, u32)>>,
    pub resources: Vec<CRes>,
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::sorted")
    )]
    pub container_res: FxMap<(ContainerId, ResourceTypeId), ResIdx>,
    /// Dense form of `container_res`: `[container * resource_types + type]`, `u32::MAX` = none.
    pub container_res_dense: Vec<ResIdx>,
    /// Per container: simulated (a top-level container of the resource environment). SimuLizar
    /// 5.2.2 creates no simulated container for nested resource containers.
    pub container_simulated: Vec<bool>,
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::sorted")
    )]
    pub link_res: FxMap<LinkingResourceId, ResIdx>,
    /// Route table `[src * containers + dst]`: resource of the first linking resource (in
    /// `links` order) connecting both, `u32::MAX` = none. Empty for very large environments.
    pub route_res: Vec<ResIdx>,
    /// `deactivateAllActiveResources` order (ResourceRegistry / container HashMap order; the
    /// exact mode).
    #[cfg(feature = "exact")]
    #[cfg_attr(feature = "snapshot", serde(skip))]
    pub finalise_order: Vec<ResIdx>,
    /// The same in the registry's insertion order (the fast mode).
    /// (Both orders are functions of `registry`; a snapshot recomputes them.)
    #[cfg_attr(feature = "snapshot", serde(skip))]
    pub finalise_order_fast: Vec<ResIdx>,
    /// Linking resources in resource-environment order (routing; after model changes the
    /// router's order, and the typed model's `connected` lists are the router's view,
    /// `crate::change`).
    pub links: Vec<LinkingResourceId>,
    /// The resource registry (`ResourceRegistry`) in insertion order: container or linking
    /// resource id, with its resources by resource type id (`finalise_order` is its Java
    /// `HashMap` iteration order).
    pub registry: Vec<RegistryEntry>,
    pub series: Vec<SeriesDef>,
    /// The series by measuring-point key: `(metric, series)` pairs.
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::series_map")
    )]
    pub series_by_mp: HashMap<Box<str>, Vec<(&'static str, SeriesId)>>,
    pub windows: Vec<CWindow>,
    /// Assembly-operation response-time series by `op_key(assembly id, role id, signature id)`.
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::sorted")
    )]
    pub asmop_series: HashMap<String, SeriesId>,
    pub passive_mon: Vec<PassiveMon>,
    /// PRM recorders on series created at initialisation, in creation order, with the index
    /// of their runtime measurement ([`CompiledModel::prm_meas`]).
    pub prm_series: Vec<(SeriesId, PrmRec, u32)>,
    /// The runtime measurements of the PRM recorders created at initialisation, in SimuLizar's
    /// order (MEAS-7.5): fixed-size, variable-size, feed-through, time-driven aggregation,
    /// utilisation windows. Recorders of lazily created calculators append theirs at run time.
    pub prm_meas: Vec<PrmMeas>,
    /// PRM recorders waiting for passive-resource calculators.
    pub prm_passive: Vec<PassivePrm>,
    /// Any PRM recorder or PRM window: the run may create the reconfiguration process.
    pub prm_any: bool,
    /// `Reconfiguration Time Tuple` series: one `(t, 0.0)` per run of the reconfiguration
    /// process (its empty reconfiguration succeeds), MEAS-7.3.
    pub reconf_time_series: Vec<SeriesId>,
    /// `Number of Resource Containers over Time`: series and the initial count (MEAS-7.3).
    pub container_count: Option<(SeriesId, usize)>,
    /// Component parameter defaults per component (ACT-2.3 step 1).
    pub comp_params: Vec<Vec<CChar>>,
    /// Configuration parameters per assembly context (ACT-2.3 step 2).
    pub ac_params: Vec<Vec<CChar>>,
    /// Capacity StoEx per passive resource.
    pub pr_capacity: Vec<ProgId>,
    /// `stream.BYTESIZE`: the payload demand of `simulateLinkingResources` (ACT-11.3).
    pub stream_bytesize: ProgId,
    /// Trace/tape strings.
    pub action_ids: Vec<Box<str>>,
    pub uaction_ids: Vec<Box<str>>,
    pub ac_ids: Vec<Box<str>>,
    pub role_ids: Vec<Box<str>>,
    pub sig_ids: Vec<Box<str>>,
    pub scenario_ids: Vec<Box<str>>,
    pub pr_ids: Vec<Box<str>>,
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::interned_vec")
    )]
    pub action_type: Vec<StaticStr>,
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::interned_vec")
    )]
    pub uaction_type: Vec<StaticStr>,
    /// The behaviours as flat instruction streams (what the interpreter executes).
    pub code: crate::code::Code,
    /// Warnings (unsupported features that only matter if reached).
    pub warnings: Vec<String>,
    /// The flavour the model is compiled for (its runs follow it, [`crate::run`]).
    pub flavour: crate::flavour::FlavourConfig,
    /// The Slingshot flavour's tables (empty for SimuLizar).
    pub ss: SsTables,
}

impl fmt::Debug for CompiledModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompiledModel")
            .field("progs", &self.progs.len())
            .field("actions", &self.act.len())
            .field("resources", &self.resources.len())
            .field("series", &self.series.len())
            .finish()
    }
}

/// Trace id of an element: its `id` attribute, else the EMF fragment.
fn trace_id(m: &Model, id: &str, obj: simoxide_model::raw::ObjId) -> Box<str> {
    if let Some(x) = m.graph.id(obj) {
        return x.into();
    }
    if id.is_empty() {
        m.graph.fragment(obj).into()
    } else {
        id.into()
    }
}

/// Compiles a StoEx specification, interning its variables' frame keys in `keys`.
pub(crate) fn compile_prog(spec: &str, keys: &mut Keys) -> Prog {
    let prog = match simoxide_stoex::prepare(spec) {
        Ok(prep) => Ok(Program::compile(&prep, |v| keys.intern(&v.id()))),
        Err(e) => Err(format!("{e:?}")),
    };
    let konst = prog.as_ref().ok().and_then(|p| p.constant().cloned());
    let konst_f64 = konst.as_ref().and_then(|v| v.to_f64().ok());
    Prog {
        spec: spec.into(),
        prog,
        konst,
        konst_f64,
    }
}

struct Builder {
    keys: Keys,
    progs: Vec<Prog>,
    prog_map: HashMap<Box<str>, ProgId>,
}

impl Builder {
    fn prog(&mut self, spec: &str) -> ProgId {
        if let Some(&p) = self.prog_map.get(spec) {
            return p;
        }
        let id = self.progs.len() as ProgId;
        self.progs.push(compile_prog(spec, &mut self.keys));
        self.prog_map.insert(spec.into(), id);
        id
    }

    fn usages(&mut self, us: &[VariableUsage]) -> Vec<CChar> {
        let mut out = Vec::new();
        for u in us {
            let name = u.name();
            let inner = u.reference.iter().any(|s| &**s == "INNER");
            for c in &u.characterisations {
                let key = self.keys.intern(&format!("{}.{}", name, c.kind.name()));
                let prog = self.prog(&c.spec.spec);
                out.push(CChar { key, prog, inner });
            }
        }
        out
    }
}

fn action_type(k: &ActionKind) -> &'static str {
    match k {
        ActionKind::Start => "StartAction",
        ActionKind::Stop => "StopAction",
        ActionKind::Internal => "InternalAction",
        ActionKind::ExternalCall { .. } => "ExternalCallAction",
        ActionKind::Branch { .. } => "BranchAction",
        ActionKind::Loop { .. } => "LoopAction",
        ActionKind::CollectionIterator { .. } => "CollectionIteratorAction",
        ActionKind::Fork { .. } => "ForkAction",
        ActionKind::Acquire { .. } => "AcquireAction",
        ActionKind::Release { .. } => "ReleaseAction",
        ActionKind::SetVariable { .. } => "SetVariableAction",
        ActionKind::InternalCall { .. } => "InternalCallAction",
        ActionKind::EmitEvent { .. } => "EmitEventAction",
        ActionKind::Unsupported(n) => n,
    }
}

fn uaction_type(k: &UserActionKind) -> &'static str {
    match k {
        UserActionKind::Start => "Start",
        UserActionKind::Stop => "Stop",
        UserActionKind::EntryLevelSystemCall { .. } => "EntryLevelSystemCall",
        UserActionKind::Delay { .. } => "Delay",
        UserActionKind::Branch { .. } => "Branch",
        UserActionKind::Loop { .. } => "Loop",
    }
}

/// Measuring-point key string (`docs/guide/formats.md` §4).
fn mp_key(m: &Model, mp: &MeasuringPoint) -> Option<String> {
    Some(match &mp.kind {
        MeasuringPointKind::UsageScenario { scenario } => format!(
            "UsageScenarioMeasuringPoint[{}]",
            m.usage_scenarios[(*scenario)?].id
        ),
        MeasuringPointKind::EntryLevelSystemCall { call } => format!(
            "EntryLevelSystemCallMeasuringPoint[{}]",
            m.user_actions[(*call)?].id
        ),
        MeasuringPointKind::SystemOperation {
            system,
            role,
            signature,
        } => format!(
            "SystemOperationMeasuringPoint[{}|{}|{}]",
            m.roles[(*role)?].id,
            m.signatures[(*signature)?].id,
            m.systems[(*system)?].id
        ),
        MeasuringPointKind::AssemblyOperation {
            assembly,
            role,
            signature,
        } => format!(
            "AssemblyOperationMeasuringPoint[{}|{}|{}]",
            m.roles[(*role)?].id,
            m.signatures[(*signature)?].id,
            m.assembly_contexts[(*assembly)?].id
        ),
        MeasuringPointKind::ExternalCallAction { action } => format!(
            "ExternalCallActionMeasuringPoint[{}]",
            m.actions[(*action)?].id
        ),
        MeasuringPointKind::ActiveResource {
            resource,
            replica_id,
        } => format!(
            "ActiveResourceMeasuringPoint[{}|replicaID={}]",
            m.processing_resources[(*resource)?].id,
            replica_id
        ),
        MeasuringPointKind::ResourceEnvironment { environment } => {
            let o = m.resource_environments[(*environment)?].obj;
            let id = m
                .graph
                .id(o)
                .map_or_else(|| m.graph.fragment(o), |x| x.to_string());
            format!("ResourceEnvironmentMeasuringPoint[{id}]")
        }
        MeasuringPointKind::Other {
            measuring_point, ..
        } if m.graph[mp.obj]
            .class
            .is_a(simoxide_model::meta::class::ResourceURIMeasuringPoint) =>
        {
            // refsim: the fragment of the resource URI (or "null") and the measuringPoint string
            let uri = m.graph.attr(
                mp.obj,
                simoxide_model::meta::feat::ResourceURIMeasuringPoint_resourceURI,
            );
            let frag = uri
                .as_str()
                .and_then(|u| u.split_once('#').map(|x| x.1.to_string()))
                .unwrap_or_else(|| "null".into());
            format!(
                "ResourceURIMeasuringPoint[{frag}|{}]",
                measuring_point.as_deref().unwrap_or("null")
            )
        }
        _ => return None,
    })
}

/// Checks of `AbstractMeasurementAggregator`'s constructor: the error it throws, if any.
fn prm_aggregation_invalid(
    spec: &MeasurementSpecification,
    frequency: i64,
    statistic: StatisticalCharacterization,
) -> Option<String> {
    if statistic == StatisticalCharacterization::Missing {
        return Some(format!(
            "NullPointerException: aggregation of specification {} has no statistical characterization",
            spec.id
        ));
    }
    (frequency < 1).then(|| {
        format!(
            "IllegalStateException: Value of 'frequency' attribute of the aggregation of specification {} must be positive!",
            spec.id
        )
    })
}

/// `MonitorRepositoryUtil` measurement identifier of an operation measuring point.
pub fn op_key(owner: &str, role: &str, sig: &str) -> String {
    format!("{owner}::{role}::{sig}")
}

fn metric_id<'a>(m: &'a Model, spec: &MeasurementSpecification) -> Option<&'a str> {
    spec.metric.map(|x| &*m.metrics[x].id)
}

/// The scope of validity of the specification's metric is `CONTINUOUS`.
fn metric_continuous(m: &Model, spec: &MeasurementSpecification) -> bool {
    spec.metric.is_some_and(|x| {
        matches!(
            m.metrics[x].kind,
            MetricKind::Base {
                scope_of_validity: "Continuous",
                ..
            }
        )
    })
}

/// The tuple metric of the calculator that measures a base metric (by metric id).
fn tuple_metric_name(id: &str) -> Option<&'static str> {
    Some(match id {
        metric_ids::RESPONSE_TIME => metric_names::RESPONSE_TIME,
        metric_ids::RESOURCE_DEMAND => metric_names::RESOURCE_DEMAND,
        metric_ids::STATE_OF_ACTIVE_RESOURCE => metric_names::STATE_OF_ACTIVE_RESOURCE,
        metric_ids::UTILIZATION => metric_names::UTILIZATION,
        metric_ids::WAITING_TIME => metric_names::WAITING_TIME,
        metric_ids::HOLDING_TIME => metric_names::HOLDING_TIME,
        metric_ids::STATE_OF_PASSIVE_RESOURCE => metric_names::STATE_OF_PASSIVE_RESOURCE,
        metric_ids::RECONFIGURATION_TIME => metric_names::RECONFIGURATION_TIME,
        metric_ids::NUMBER_OF_RESOURCE_CONTAINERS => metric_names::NUMBER_OF_RESOURCE_CONTAINERS,
        _ => return None,
    })
}

/// Where a sliding window comes from: its specification and measuring point.
struct WinSrc {
    mp: Option<ObjId>,
    spec: ObjId,
    /// Series key of the measuring point.
    key: Option<String>,
    /// Tuple metric of the calculator a time-driven aggregation reads.
    metric: Option<&'static str>,
    label: Box<str>,
}

impl WinSrc {
    fn of(m: &Model, mon: &Monitor, spec: &MeasurementSpecification) -> WinSrc {
        let mp = mon.measuring_point.map(|x| &m.measuring_points[x]);
        let key = mp.and_then(|mp| mp_key(m, mp));
        let metric = metric_id(m, spec).and_then(tuple_metric_name);
        WinSrc {
            mp: mp.map(|x| x.obj),
            spec: spec.obj,
            label: format!(
                "{}|{}",
                key.as_deref().unwrap_or_default(),
                metric.unwrap_or(metric_names::UTILIZATION)
            )
            .into(),
            key,
            metric,
        }
    }
}

/// The interned tables a compilation starts from ([`CompiledModel::compile_seeded`]): frame
/// keys, StoEx programs and measurement series keep their ids, new ones are appended.
#[derive(Clone, Debug, Default)]
pub struct Seed {
    /// The flavour to compile for (SimuLizar by default).
    pub flavour: crate::flavour::FlavourConfig,
    pub keys: Keys,
    pub progs: Vec<Prog>,
    pub series: Vec<SeriesDef>,
}

impl CompiledModel {
    /// Compiles a loaded model (the first usage model is simulated).
    pub fn compile(model: Model) -> Result<CompiledModel, CompileError> {
        Self::compile_seeded(model, Seed::default())
    }

    /// Compiles a loaded model for `flavour` (SimuLizar or Slingshot with its compat flags).
    pub fn compile_flavour(
        model: Model,
        flavour: crate::flavour::FlavourConfig,
    ) -> Result<CompiledModel, CompileError> {
        Self::compile_seeded(
            model,
            Seed {
                flavour,
                ..Seed::default()
            },
        )
    }

    /// [`CompiledModel::compile`] starting from the keys, programs and series of `seed`: equal
    /// keys, StoEx strings and (measuring point, metric) pairs get the seed's ids. Used to
    /// recompile a changed model for a running simulation (`crate::change`).
    pub fn compile_seeded(model: Model, seed: Seed) -> Result<CompiledModel, CompileError> {
        let m = &model;
        let flavour = seed.flavour;
        if flavour.kind == crate::flavour::FlavourKind::SimuLizar && !flavour.quirks.is_empty() {
            return Err(CompileError(
                "Slingshot quirk flags need the Slingshot flavour".into(),
            ));
        }
        let prog_map = seed
            .progs
            .iter()
            .enumerate()
            .map(|(i, p)| (p.spec.clone(), i as ProgId))
            .collect();
        let mut b = Builder {
            keys: seed.keys,
            progs: seed.progs,
            prog_map,
        };
        let mut warnings = Vec::new();

        // ---------------------------------------------------------------- series registry
        let mut series: Vec<SeriesDef> = seed.series;
        let mut series_map: HashMap<(String, &'static str), SeriesId> = series
            .iter()
            .enumerate()
            .map(|(i, d)| ((d.mp.to_string(), d.metric), i as SeriesId))
            .collect();
        fn new_series(
            series: &mut Vec<SeriesDef>,
            series_map: &mut HashMap<(String, &'static str), SeriesId>,
            mp: String,
            metric: &'static str,
        ) -> SeriesId {
            *series_map.entry((mp.clone(), metric)).or_insert_with(|| {
                series.push(SeriesDef {
                    mp: mp.into(),
                    metric,
                });
                (series.len() - 1) as SeriesId
            })
        }
        let repo = m.monitor_repositories.iter().next();
        let monitors: Vec<&Monitor> = repo
            .map(|r| r.monitors.iter().map(|&x| &m.monitors[x]).collect())
            .unwrap_or_default();
        let ss = flavour.kind == crate::flavour::FlavourKind::Slingshot;
        let quirks = flavour.quirks;
        // monitors of SimuLizar's probe framework (sliding windows, PRM, reconfiguration time,
        // container count, passive resources): none in the Slingshot flavour, which has its own
        // (`SsTables`, `docs/spec/slingshot/monitoring.md`)
        let sim_monitors: Vec<&Monitor> = if ss { Vec::new() } else { monitors.clone() };

        // response time calculators: key(MP) -> series (later specs overwrite the probes)
        // (probes are looked up by `MonitorRepositoryUtil` measurement identifiers, i.e. by id
        // strings: a measuring point may reference an equal-id element of another loaded model)
        let mut scen_series: HashMap<&str, SeriesId> = HashMap::new();
        let mut elsc_series: HashMap<&str, SeriesId> = HashMap::new();
        let mut ext_series: HashMap<&str, SeriesId> = HashMap::new();
        let mut sysop_series: HashMap<String, SeriesId> = HashMap::new();
        let mut asmop_series: HashMap<String, SeriesId> = HashMap::new();
        for mon in &monitors {
            if !mon.activated {
                continue;
            }
            let Some(mpid) = mon.measuring_point else {
                continue;
            };
            let mp = &m.measuring_points[mpid];
            // Q29: Slingshot ignores entry-level call, system-operation and external-call points
            if ss
                && quirks.has(crate::flavour::Quirk::Q29)
                && matches!(
                    mp.kind,
                    MeasuringPointKind::EntryLevelSystemCall { .. }
                        | MeasuringPointKind::SystemOperation { .. }
                        | MeasuringPointKind::ExternalCallAction { .. }
                )
            {
                continue;
            }
            for spec in &mon.specifications {
                if metric_id(m, spec) != Some(metric_ids::RESPONSE_TIME) {
                    continue;
                }
                let Some(key) = mp_key(m, mp) else { continue };
                let s = new_series(
                    &mut series,
                    &mut series_map,
                    key,
                    metric_names::RESPONSE_TIME,
                );
                match &mp.kind {
                    MeasuringPointKind::UsageScenario { scenario: Some(x) } => {
                        scen_series.insert(&m.usage_scenarios[*x].id, s);
                    }
                    MeasuringPointKind::EntryLevelSystemCall { call: Some(x) } => {
                        elsc_series.insert(&m.user_actions[*x].id, s);
                    }
                    MeasuringPointKind::ExternalCallAction { action: Some(x) } => {
                        ext_series.insert(&m.actions[*x].id, s);
                    }
                    MeasuringPointKind::SystemOperation {
                        system: Some(y),
                        role: Some(r),
                        signature: Some(sg),
                    } => {
                        sysop_series.insert(
                            op_key(&m.systems[*y].id, &m.roles[*r].id, &m.signatures[*sg].id),
                            s,
                        );
                    }
                    MeasuringPointKind::AssemblyOperation {
                        assembly: Some(a),
                        role: Some(r),
                        signature: Some(sg),
                    } => {
                        asmop_series.insert(
                            op_key(
                                &m.assembly_contexts[*a].id,
                                &m.roles[*r].id,
                                &m.signatures[*sg].id,
                            ),
                            s,
                        );
                    }
                    _ => {}
                }
            }
        }

        // ---------------------------------------------------------------- SEFF behaviours
        let mut beh = Vec::with_capacity(m.behaviours.len());
        for bh in m.behaviours.iter() {
            let start = bh
                .steps
                .iter()
                .copied()
                .find(|a| matches!(m.actions[*a].kind, ActionKind::Start));
            let mut chain = Vec::new();
            let mut ends_at_stop = false;
            if let Some(s) = start {
                let mut cur = m.actions[s].successor;
                while let Some(a) = cur {
                    if matches!(m.actions[a].kind, ActionKind::Stop) {
                        ends_at_stop = true;
                        break;
                    }
                    if chain.len() > m.actions.len() {
                        break; // cycle
                    }
                    chain.push(a);
                    cur = m.actions[a].successor;
                }
            }
            beh.push(CBeh {
                start,
                chain,
                ends_at_stop,
            });
        }

        // ---------------------------------------------------------------- SEFF actions
        let beh_by_obj: HashMap<simoxide_model::raw::ObjId, BehaviourId> =
            m.behaviours.iter_ids().map(|(i, bh)| (bh.obj, i)).collect();
        let mut act = Vec::with_capacity(m.actions.len());
        let mut action_ids = Vec::with_capacity(m.actions.len());
        let mut action_type_v = Vec::with_capacity(m.actions.len());
        for (aid, a) in m.actions.iter_ids() {
            action_ids.push(trace_id(m, &a.id, a.obj));
            action_type_v.push(action_type(&a.kind));
            let c = match &a.kind {
                ActionKind::Start => CAct::Start,
                ActionKind::Stop => CAct::Stop,
                ActionKind::Internal => {
                    let demands = a
                        .resource_demands
                        .iter()
                        .map(|d| (b.prog(&d.spec.spec), d.resource_type))
                        .collect();
                    let infra = a
                        .infrastructure_calls
                        .iter()
                        .map(|ic| CInfraCall {
                            id: trace_id(m, "", ic.obj),
                            count: b.prog(&ic.number_of_calls.spec),
                            role: ic.role,
                            signature: ic.signature,
                            inputs: b.usages(&ic.inputs),
                        })
                        .collect();
                    let rescalls = a
                        .resource_calls
                        .iter()
                        .map(|rc| {
                            // last resource type (repository order) providing the interface
                            // of the called signature
                            let iface = rc
                                .signature
                                .and_then(|x| m.resource_signatures[x].interface);
                            let mut rt = None;
                            for (tid, t) in m.resource_types.iter_ids() {
                                for &pr in &t.provided_roles {
                                    if iface.is_some()
                                        && m.roles[pr]
                                            .resource_interface
                                            .map(|i| &m.resource_interfaces[i].id)
                                            == iface.map(|i| &m.resource_interfaces[i].id)
                                    {
                                        rt = Some(tid);
                                        break;
                                    }
                                }
                            }
                            CResCall {
                                id: trace_id(m, "", rc.obj),
                                count: b.prog(&rc.number_of_calls.spec),
                                resource_type: rt,
                                service_id: rc
                                    .signature
                                    .map(|x| m.resource_signatures[x].service_id)
                                    .unwrap_or(0),
                            }
                        })
                        .collect();
                    CAct::Internal {
                        demands,
                        infra,
                        rescalls,
                    }
                }
                ActionKind::ExternalCall {
                    signature,
                    role,
                    inputs,
                    returns,
                    ..
                } => CAct::External {
                    role: *role,
                    signature: *signature,
                    inputs: b.usages(inputs),
                    returns: b.usages(returns),
                    series: ext_series.get(&*m.actions[aid].id).copied(),
                },
                ActionKind::Branch { transitions } => {
                    if transitions.is_empty() {
                        CAct::EmptyBranch
                    } else {
                        let sels = transitions
                            .iter()
                            .map(|t| trace_id(m, &t.id, t.obj))
                            .collect();
                        let behaviours = transitions.iter().map(|t| t.behaviour).collect();
                        match transitions[0].condition {
                            BranchCondition::Probability(_) => {
                                let mut cum = Vec::new();
                                let mut s = 0.0f64;
                                for t in transitions {
                                    let p = match t.condition {
                                        BranchCondition::Probability(p) => p,
                                        _ => f64::NAN,
                                    };
                                    s += p;
                                    cum.push(s);
                                }
                                CAct::ProbBranch {
                                    cum,
                                    behaviours,
                                    sels,
                                }
                            }
                            BranchCondition::Guard(_) => {
                                let guards = transitions
                                    .iter()
                                    .map(|t| match &t.condition {
                                        BranchCondition::Guard(g) => b.prog(&g.spec),
                                        _ => b.prog("false"),
                                    })
                                    .collect();
                                let guard_ids = transitions
                                    .iter()
                                    .map(|t| trace_id(m, &t.id, t.obj))
                                    .collect();
                                CAct::GuardBranch {
                                    guards,
                                    guard_ids,
                                    behaviours,
                                    sels,
                                }
                            }
                        }
                    }
                }
                ActionKind::Loop { iterations, body } => CAct::Loop {
                    count: b.prog(&iterations.spec),
                    body: *body,
                },
                ActionKind::CollectionIterator { parameter, body } => {
                    let name = parameter
                        .map(|p| m.parameters[p].name.to_string())
                        .unwrap_or_default();
                    CAct::Collection {
                        count: b.prog(&format!("{name}.NUMBER_OF_ELEMENTS")),
                        prefix: format!("{name}.").into(),
                        body: *body,
                    }
                }
                ActionKind::Fork {
                    asynchronous,
                    synchronisation,
                } => CAct::Fork {
                    asynchronous: asynchronous.clone(),
                    synchronous: synchronisation
                        .as_ref()
                        .map(|s| s.synchronous.clone())
                        .unwrap_or_default(),
                },
                ActionKind::Acquire { resource, .. } => CAct::Acquire(*resource),
                ActionKind::Release { resource } => CAct::Release(*resource),
                ActionKind::SetVariable { usages } => CAct::SetVariable(b.usages(usages)),
                ActionKind::InternalCall { .. } => {
                    CAct::Unsupported("InternalCallAction".to_string())
                }
                ActionKind::EmitEvent { .. } => CAct::Unsupported("EmitEventAction".to_string()),
                ActionKind::Unsupported("RecoveryAction") => CAct::Recovery(
                    m.graph
                        .get_ref(
                            a.obj,
                            simoxide_model::meta::feat::RecoveryAction_primaryBehaviour__RecoveryAction,
                        )
                        .and_then(|o| beh_by_obj.get(&o).copied()),
                ),
                ActionKind::Unsupported(n) => CAct::Unsupported(n.to_string()),
            };
            act.push(c);
        }

        // ---------------------------------------------------------------- usage model
        let mut ubeh = Vec::with_capacity(m.scenario_behaviours.len());
        for sb in m.scenario_behaviours.iter() {
            let start = sb
                .actions
                .iter()
                .copied()
                .find(|a| matches!(m.user_actions[*a].kind, UserActionKind::Start));
            let mut chain = Vec::new();
            let mut ends_at_stop = false;
            if let Some(s) = start {
                let mut cur = m.user_actions[s].successor;
                while let Some(a) = cur {
                    if matches!(m.user_actions[a].kind, UserActionKind::Stop) {
                        ends_at_stop = true;
                        break;
                    }
                    if chain.len() > m.user_actions.len() {
                        break;
                    }
                    chain.push(a);
                    cur = m.user_actions[a].successor;
                }
            }
            ubeh.push(CUBeh {
                start,
                chain,
                ends_at_stop,
            });
        }
        let system = m
            .allocations
            .iter()
            .find_map(|a| a.system)
            .or_else(|| m.systems.ids().next())
            .ok_or_else(|| CompileError("no system".into()))?;
        let mut uact = Vec::with_capacity(m.user_actions.len());
        let mut uaction_ids = Vec::new();
        let mut uaction_type_v = Vec::new();
        for ua in m.user_actions.iter() {
            uaction_ids.push(trace_id(m, &ua.id, ua.obj));
            uaction_type_v.push(uaction_type(&ua.kind));
            let c = match &ua.kind {
                UserActionKind::Start => CUAct::Start,
                UserActionKind::Stop => CUAct::Stop,
                UserActionKind::EntryLevelSystemCall {
                    role,
                    signature,
                    inputs,
                    outputs,
                    ..
                } => CUAct::Elsc {
                    role: *role,
                    signature: *signature,
                    inputs: b.usages(inputs),
                    outputs: b.usages(outputs),
                    series: elsc_series.get(&*ua.id).copied(),
                    sysop_series: match (role, signature) {
                        (Some(r), Some(s)) => sysop_series
                            .get(&op_key(
                                &m.systems[system].id,
                                &m.roles[*r].id,
                                &m.signatures[*s].id,
                            ))
                            .copied(),
                        _ => None,
                    },
                },
                UserActionKind::Delay { time } => CUAct::Delay(b.prog(&time.spec)),
                UserActionKind::Branch { transitions } => {
                    let mut cum = Vec::new();
                    let mut s = 0.0f64;
                    for t in transitions {
                        s += t.probability;
                        cum.push(s);
                    }
                    CUAct::Branch {
                        cum,
                        behaviours: transitions.iter().map(|t| t.behaviour).collect(),
                        sels: transitions
                            .iter()
                            .map(|t| m.graph.fragment(t.obj).into())
                            .collect(),
                    }
                }
                UserActionKind::Loop { iterations, body } => CUAct::Loop {
                    count: b.prog(&iterations.spec),
                    body: *body,
                },
            };
            uact.push(c);
        }
        let usage_model = m
            .usage_models
            .iter()
            .next()
            .ok_or_else(|| CompileError("no usage model".into()))?;
        let mut scenarios = Vec::new();
        for &sid in &usage_model.scenarios {
            let s = &m.usage_scenarios[sid];
            let workload = match &s.workload {
                Workload::Open { inter_arrival_time } => CWorkload::Open {
                    inter_arrival: b.prog(&inter_arrival_time.spec),
                },
                Workload::Closed {
                    population,
                    think_time,
                } => CWorkload::Closed {
                    population: *population,
                    think_time: b.prog(&think_time.spec),
                },
                Workload::Missing => {
                    return Err(CompileError(format!(
                        "usage scenario {} has no workload",
                        s.id
                    )));
                }
            };
            scenarios.push(CScenario {
                id: sid,
                behaviour: s.behaviour,
                workload,
                series: scen_series.get(&*s.id).copied(),
            });
        }

        // ---------------------------------------------------------------- composition
        let mut prov_deleg = FxMap::default();
        let mut req_conn = FxMap::default();
        for (sid, st) in m.structures.iter_ids() {
            for &c in &st.connectors {
                match &m.connectors[c].kind {
                    ConnectorKind::ProvidedDelegation {
                        outer_role: Some(r),
                        ..
                    } => {
                        prov_deleg.entry((sid, *r)).or_insert(c);
                    }
                    ConnectorKind::RequiredDelegation {
                        assembly: Some(a),
                        inner_role: Some(r),
                        ..
                    }
                    | ConnectorKind::RequiredInfrastructureDelegation {
                        assembly: Some(a),
                        inner_role: Some(r),
                        ..
                    }
                    | ConnectorKind::Assembly {
                        requiring: Some(a),
                        required_role: Some(r),
                        ..
                    }
                    | ConnectorKind::AssemblyInfrastructure {
                        requiring: Some(a),
                        required_role: Some(r),
                        ..
                    } if m.assembly_contexts[*a].parent == Some(sid) => {
                        // only connectors of the AC's parent structure count
                        req_conn.entry((*a, *r)).or_insert(c);
                    }
                    _ => {}
                }
            }
        }
        check_composition(m, m.systems[system].structure)?;
        // allocation: FQ path -> container (later entries overwrite; AllocationLookupSyncer)
        let mut alloc: FxMap<Box<[u32]>, ContainerId> = FxMap::default();
        let sys_structure = m.systems[system].structure;
        // Slingshot: the first allocation context of an assembly context wins (SS-ACT-9.1);
        // with Q30 the inner assembly contexts of a composite are not allocated (SS-ACT-1.5)
        let alloc_mode = match (ss, quirks.has(crate::flavour::Quirk::Q30)) {
            (false, _) => AllocMode::LastNested,
            (true, false) => AllocMode::FirstNested,
            (true, true) => AllocMode::FirstFlat,
        };
        for al in m.allocations.iter() {
            for &ctx in &al.contexts {
                allocate_context_mode(m, sys_structure, ctx, &mut alloc, alloc_mode)?;
            }
        }

        // path trie of all nested assembly contexts (same depth bound as `add_nested`)
        let mut ac_paths = vec![AcPath {
            parent: u32::MAX,
            ac: SYSTEM_AC,
            container: alloc.get(&[][..]).copied(),
        }];
        let mut ac_path_children: Vec<Vec<(u32, u32)>> =
            vec![Vec::new(); m.assembly_contexts.len()];
        fn add_paths(
            m: &Model,
            alloc: &FxMap<Box<[u32]>, ContainerId>,
            st: StructureId,
            parent: u32,
            base: &mut Vec<u32>,
            paths: &mut Vec<AcPath>,
            children: &mut Vec<Vec<(u32, u32)>>,
        ) {
            for &ac in &m.structures[st].assembly_contexts {
                if children[ac.index()].iter().any(|e| e.0 == parent) {
                    continue;
                }
                base.push(ac.0);
                let id = paths.len() as u32;
                paths.push(AcPath {
                    parent,
                    ac: ac.0,
                    container: alloc.get(&base[..]).copied(),
                });
                children[ac.index()].push((parent, id));
                if base.len() <= 32
                    && let Some(comp) = m.assembly_contexts[ac].component
                    && let Some(inner) = m.components[comp].structure()
                {
                    add_paths(m, alloc, inner, id, base, paths, children);
                }
                base.pop();
            }
        }
        add_paths(
            m,
            &alloc,
            sys_structure,
            ROOT_PATH,
            &mut Vec::new(),
            &mut ac_paths,
            &mut ac_path_children,
        );

        // ---------------------------------------------------------------- resources
        let env = m
            .allocations
            .iter()
            .find_map(|a| a.environment)
            .or_else(|| m.resource_environments.ids().next());
        let mut resources: Vec<CRes> = Vec::new();
        let mut container_res = FxMap::default();
        let mut link_res = FxMap::default();
        let mut links = Vec::new();
        // registry keys in insertion order (containers, then links) and per-container type ids
        let mut registry: Vec<(String, Vec<(String, ResIdx)>)> = Vec::new();
        let mut container_simulated = vec![false; m.containers.len()];
        let mut ss_tables = SsTables::default();
        if ss {
            ss_resources(
                m,
                env,
                quirks,
                &monitors,
                &mut b,
                &mut series,
                &mut series_map,
                &mut resources,
                &mut container_res,
                &mut link_res,
                &mut links,
                &mut registry,
                &mut container_simulated,
                &mut ss_tables,
            )?;
        } else if let Some(env) = env {
            let e = &m.resource_environments[env];
            for &cid in &e.containers {
                container_simulated[cid.index()] = true;
                let cont = &m.containers[cid];
                let mut types: Vec<(String, ResIdx)> = Vec::new();
                for &prid in &cont.processing_resources {
                    let pr = &m.processing_resources[prid];
                    let Some(rt) = pr.resource_type else {
                        warnings.push(format!("processing resource {} has no type", pr.id));
                        continue;
                    };
                    let rate = b.prog(&pr.processing_rate.spec);
                    if let Some(&existing) = container_res.get(&(cid, rt)) {
                        // syncProcessingResource: only the rate changes
                        let r: &mut CRes = &mut resources[existing as usize];
                        r.rate = rate;
                        continue;
                    }
                    let pol_id = pr
                        .scheduling
                        .map(|s| m.scheduling_policies[s].id.to_string())
                        .unwrap_or_default();
                    let policy = SchedulingPolicy::from_pcm_id(&pol_id).ok_or_else(|| {
                        CompileError(format!("unsupported scheduling policy '{pol_id}'"))
                    })?;
                    let sched_name: Box<str> = match pol_id.as_str() {
                        "ProcessorSharing" => "PROCESSOR_SHARING".into(),
                        "Delay" => "DELAY".into(),
                        o => o.into(),
                    };
                    if pr.replicas > MAX_REPLICAS {
                        return Err(CompileError(format!(
                            "numberOfReplicas {} of {} exceeds the supported maximum {MAX_REPLICAS}",
                            pr.replicas, pr.id
                        )));
                    }
                    let instances = pr.replicas.max(1) as u32;
                    let idx = resources.len() as ResIdx;
                    let mut r = CRes {
                        kind: ResKind::Processing(prid),
                        policy,
                        instances,
                        rate,
                        latency: None,
                        hdd: pr
                            .hdd
                            .as_ref()
                            .map(|h| (b.prog(&h.read.spec), b.prog(&h.write.spec))),
                        type_id: m.resource_types[rt].id.clone(),
                        rc: cont.id.clone(),
                        spec_name: pr.id.clone(),
                        sched_name,
                        origin: format!("rate:{}", cont.id).into(),
                        state_series: vec![Vec::new(); instances as usize],
                        overall_series: None,
                        demand_series: Vec::new(),
                        initial_states: Vec::new(),
                    };
                    // monitors (ResourceEnvironmentSyncer.attachMonitors)
                    for mon in &monitors {
                        if !mon.activated {
                            continue;
                        }
                        let Some(mpid) = mon.measuring_point else {
                            continue;
                        };
                        let mp = &m.measuring_points[mpid];
                        let MeasuringPointKind::ActiveResource {
                            resource: Some(mres),
                            replica_id,
                        } = &mp.kind
                        else {
                            continue;
                        };
                        if m.processing_resources[*mres].id != pr.id {
                            continue;
                        }
                        let key = mp_key(m, mp).unwrap_or_default();
                        for spec in &mon.specifications {
                            match metric_id(m, spec) {
                                Some(metric_ids::STATE_OF_ACTIVE_RESOURCE) => {
                                    if !matches!(spec.processing, ProcessingType::FeedThrough) {
                                        return Err(CompileError(
                                            "state of active resource needs FeedThrough".into(),
                                        ));
                                    }
                                    if *replica_id == 0 && instances > 1 {
                                        let omp = format!(
                                            "ActiveResourceMeasuringPoint[{}|replicaID={}]",
                                            pr.id, instances
                                        );
                                        r.overall_series = Some(new_series(
                                            &mut series,
                                            &mut series_map,
                                            omp,
                                            metric_names::UTILIZATION,
                                        ));
                                    }
                                    let inst = if matches!(
                                        policy,
                                        SchedulingPolicy::Delay | SchedulingPolicy::Fcfs
                                    ) {
                                        0
                                    } else {
                                        *replica_id
                                    };
                                    let s = new_series(
                                        &mut series,
                                        &mut series_map,
                                        key.clone(),
                                        metric_names::STATE_OF_ACTIVE_RESOURCE,
                                    );
                                    if inst >= 0 && (inst as usize) < r.state_series.len() {
                                        r.state_series[inst as usize].push(s);
                                        r.initial_states.push((s, inst as u32));
                                    }
                                }
                                Some(metric_ids::RESOURCE_DEMAND) => {
                                    let s = new_series(
                                        &mut series,
                                        &mut series_map,
                                        key.clone(),
                                        metric_names::RESOURCE_DEMAND,
                                    );
                                    r.demand_series.push(s);
                                }
                                _ => {}
                            }
                        }
                    }
                    resources.push(r);
                    container_res.insert((cid, rt), idx);
                    types.push((m.resource_types[rt].id.to_string(), idx));
                }
                registry.push((cont.id.to_string(), types));
            }
            for &lid in &e.linking_resources {
                let l = &m.linking_resources[lid];
                links.push(lid);
                let idx = resources.len() as ResIdx;
                let (type_id, type_name) = l
                    .resource_type
                    .map(|t| {
                        (
                            m.resource_types[t].id.clone(),
                            m.resource_types[t].name.clone(),
                        )
                    })
                    .unwrap_or_default();
                resources.push(CRes {
                    kind: ResKind::Link(lid),
                    policy: SchedulingPolicy::Fcfs,
                    instances: 1,
                    rate: b.prog(&l.throughput.spec),
                    latency: Some(b.prog(&l.latency.spec)),
                    hdd: None,
                    type_id: type_id.clone(),
                    rc: l.id.clone(),
                    spec_name: type_name,
                    sched_name: "FCFS".into(),
                    origin: format!("rate:{}", l.id).into(),
                    state_series: vec![Vec::new()],
                    overall_series: None,
                    demand_series: Vec::new(),
                    initial_states: Vec::new(),
                });
                link_res.insert(lid, idx);
                registry.push((l.id.to_string(), vec![(type_id.to_string(), idx)]));
            }
        }
        let registry: Vec<RegistryEntry> = registry
            .into_iter()
            .map(|(k, t)| {
                (
                    k.into(),
                    t.into_iter().map(|(a, b)| (a.into(), b)).collect(),
                )
            })
            .collect();
        let finalise_order_fast = finalise_order::<InsertionOrder>(&registry);
        #[cfg(feature = "exact")]
        let finalise_order = finalise_order::<JdkHashOrder>(&registry);

        // sliding windows (probe-framework decorators in extension-registry order:
        // slidingwindow = TimeDrivenAggregation, then utilization = TimeDriven utilisation)
        let mut windows = Vec::new();
        // per window: its specification, measuring point and (time-driven aggregation) the
        // metric of the calculator it reads
        let mut window_src: Vec<WinSrc> = Vec::new();
        for mon in sim_monitors.iter().filter(|x| x.activated) {
            for spec in &mon.specifications {
                if let ProcessingType::TimeDrivenAggregation {
                    window_length,
                    window_increment,
                    statistic,
                } = spec.processing
                {
                    // the input (the calculator of the spec's metric) is found after all
                    // calculators exist, below
                    windows.push(CWindow {
                        len: window_length,
                        inc: window_increment,
                        input: None,
                        out: None,
                        prm: spec.triggers_self_adaptations,
                        prm_meas: u32::MAX,
                        stat: Some(Stat {
                            kind: StatKind::of(statistic).unwrap_or(StatKind::ArithmeticMean),
                            continuous: metric_continuous(m, spec),
                        }),
                    });
                    window_src.push(WinSrc::of(m, mon, spec));
                }
            }
        }
        for wanted in [metric_ids::UTILIZATION_TUPLE, metric_ids::UTILIZATION] {
            for mon in sim_monitors.iter().filter(|x| x.activated) {
                for spec in &mon.specifications {
                    if metric_id(m, spec) != Some(wanted) {
                        continue;
                    }
                    let (len, inc) = match spec.processing {
                        ProcessingType::TimeDriven {
                            window_length,
                            window_increment,
                        }
                        | ProcessingType::TimeDrivenAggregation {
                            window_length,
                            window_increment,
                            ..
                        } => (window_length, window_increment),
                        _ => {
                            return Err(CompileError(
                                "utilization monitor must provide a TimeDriven processing type"
                                    .into(),
                            ));
                        }
                    };
                    let mp = mon.measuring_point.map(|x| &m.measuring_points[x]);
                    let key = mp.and_then(|mp| mp_key(m, mp)).unwrap_or_default();
                    let Some(&state) =
                        series_map.get(&(key.clone(), metric_names::STATE_OF_ACTIVE_RESOURCE))
                    else {
                        return Err(CompileError(format!(
                            "Utilization measurements (sliding window based) cannot be initialized: no state of active resource calculator for {key}"
                        )));
                    };
                    let out =
                        new_series(&mut series, &mut series_map, key, metric_names::UTILIZATION);
                    windows.push(CWindow {
                        len,
                        inc,
                        input: Some(state),
                        out: Some(out),
                        prm: spec.triggers_self_adaptations,
                        prm_meas: u32::MAX,
                        stat: None,
                    });
                    window_src.push(WinSrc::of(m, mon, spec));
                    // replica 0 of a multi-core resource: also a window on the overall utilisation
                    if let Some(MeasuringPointKind::ActiveResource {
                        resource: Some(prid),
                        replica_id: 0,
                    }) = mp.map(|x| &x.kind)
                        && m.processing_resources[*prid].replicas > 1
                        && let Some(r) = resources.iter().find(|r| {
                            matches!(r.kind, ResKind::Processing(p)
                                if m.processing_resources[p].id == m.processing_resources[*prid].id)
                        })
                        && let Some(ov) = r.overall_series
                    {
                        windows.push(CWindow {
                            len,
                            inc,
                            input: Some(ov),
                            out: Some(ov),
                            prm: spec.triggers_self_adaptations,
                            prm_meas: u32::MAX,
                            stat: None,
                        });
                        window_src.push(WinSrc::of(m, mon, spec));
                    }
                }
            }
        }

        // passive resource monitoring flags (isMonitored over all monitors, MEAS-5.1)
        let mut passive_mon = vec![PassiveMon::default(); m.passive_resources.len()];
        for (pid, pr) in m.passive_resources.iter_ids() {
            let mut pm = PassiveMon::default();
            for mon in &sim_monitors {
                let Some(mpid) = mon.measuring_point else {
                    continue;
                };
                let conforms = match &m.measuring_points[mpid].kind {
                    MeasuringPointKind::AssemblyPassiveResource {
                        passive_resource: Some(x),
                        ..
                    } => m.passive_resources[*x].id == pr.id,
                    MeasuringPointKind::Other { .. } => true,
                    _ => false,
                };
                if !conforms {
                    continue;
                }
                for spec in &mon.specifications {
                    match metric_id(m, spec) {
                        Some(metric_ids::STATE_OF_PASSIVE_RESOURCE) => pm.state = true,
                        Some(metric_ids::WAITING_TIME) => pm.waiting = true,
                        Some(metric_ids::HOLDING_TIME) => pm.holding = true,
                        _ => {}
                    }
                }
            }
            passive_mon[pid.index()] = pm;
        }

        // reconfiguration time (ProbeFrameworkListener.initReconfigurationTimeMeasurement) and
        // number of resource containers (NumberOfResourceContainerTrackingListener), MEAS-7.3
        let mut reconf_time_series = Vec::new();
        let mut container_count = None;
        for mon in sim_monitors.iter().filter(|x| x.activated) {
            let Some(mp) = mon.measuring_point.map(|x| &m.measuring_points[x]) else {
                continue;
            };
            for spec in &mon.specifications {
                match metric_id(m, spec) {
                    Some(metric_ids::RECONFIGURATION_TIME) => {
                        if let Some(key) = mp_key(m, mp) {
                            reconf_time_series.push(new_series(
                                &mut series,
                                &mut series_map,
                                key,
                                metric_names::RECONFIGURATION_TIME,
                            ));
                        }
                    }
                    Some(metric_ids::NUMBER_OF_RESOURCE_CONTAINERS)
                        if container_count.is_none()
                            && matches!(
                                mp.kind,
                                MeasuringPointKind::ResourceEnvironment { .. }
                            ) =>
                    {
                        if let Some(key) = mp_key(m, mp) {
                            let n = env.map_or(0, |e| m.resource_environments[e].containers.len());
                            let s = new_series(
                                &mut series,
                                &mut series_map,
                                key,
                                metric_names::NUMBER_OF_RESOURCE_CONTAINERS,
                            );
                            container_count = Some((s, n));
                        }
                    }
                    _ => {}
                }
            }
        }
        // time-driven aggregation windows read the calculator of their spec's metric at their
        // measuring point (SlidingWindowProbeFrameWorkListenerDecorator), MEAS-7.5
        for (w, src) in windows.iter_mut().zip(&window_src) {
            if w.stat.is_some()
                && let (Some(key), Some(name)) = (&src.key, src.metric)
            {
                w.input = series_map.get(&(key.clone(), name)).copied();
            }
        }
        // PRM recorders of specs with triggersSelfAdaptations (MEAS-7.2): FeedThrough and the
        // measurement-driven aggregations attach to the calculator of their MP and base metric.
        // The decorators run in extension-registry order (aggregation: fixed-size, then
        // variable-size specs; feed-through), each over the specs in monitor order; every
        // recorder adds its runtime measurement to the PRM when it is created (MEAS-7.5).
        let mut prm_series = Vec::new();
        let mut prm_passive = Vec::new();
        let mut prm_meas: Vec<PrmMeas> = Vec::new();
        let mut cands: Vec<(u8, &Monitor, &MeasurementSpecification)> = Vec::new();
        for mon in sim_monitors.iter().filter(|x| x.activated) {
            if mon.measuring_point.is_none() {
                continue;
            }
            for spec in &mon.specifications {
                if !spec.triggers_self_adaptations {
                    continue;
                }
                let group = match &spec.processing {
                    ProcessingType::FixedSizeAggregation { .. } => 0,
                    ProcessingType::VariableSizeAggregation { .. } => 1,
                    ProcessingType::FeedThrough => 2,
                    ProcessingType::Unsupported(_) => 3,
                    _ => continue,
                };
                cands.push((group, mon, spec));
            }
        }
        cands.sort_by_key(|c| c.0);
        for (_, mon, spec) in cands {
            let mp = &m.measuring_points[mon.measuring_point.expect("measuring point")];
            let (rec, invalid) = match &spec.processing {
                ProcessingType::FeedThrough => (PrmRec::FeedThrough, None),
                ProcessingType::FixedSizeAggregation {
                    frequency,
                    number_of_measurements,
                    statistic,
                } => (
                    PrmRec::Fixed {
                        freq: *frequency,
                        n: *number_of_measurements,
                        stat: Stat {
                            kind: StatKind::of(*statistic).unwrap_or(StatKind::ArithmeticMean),
                            continuous: metric_continuous(m, spec),
                        },
                    },
                    prm_aggregation_invalid(spec, *frequency, *statistic).or_else(|| {
                        (*number_of_measurements < 1).then(|| format!(
                            "IllegalStateException: Value of 'numberOfMeasurements' attribute of 'FixedSizeAggregation' of specification {} must be positive!",
                            spec.id
                        ))
                    }),
                ),
                ProcessingType::VariableSizeAggregation {
                    frequency,
                    retrospection_length,
                    statistic,
                } => (
                    PrmRec::Variable {
                        freq: *frequency,
                        retro: *retrospection_length,
                        continuous: metric_continuous(m, spec),
                        stat: Stat {
                            kind: StatKind::of(*statistic).unwrap_or(StatKind::ArithmeticMean),
                            continuous: metric_continuous(m, spec),
                        },
                    },
                    prm_aggregation_invalid(spec, *frequency, *statistic).or_else(|| {
                        (*retrospection_length <= 0.0 || retrospection_length.is_nan()).then(|| format!(
                            "IllegalStateException: Value of 'retrospectionLength' attribute of 'VariableSizeAggregation' of specification {} must be positive!",
                            spec.id
                        ))
                    }),
                ),
                ProcessingType::Unsupported(n) => {
                    warnings.push(format!(
                        "measurement specification {} ({n}) triggers self-adaptations: its runtime measurements are not modelled",
                        spec.id
                    ));
                    continue;
                }
                _ => continue,
            };
            let Some(metric) = spec.metric.map(|x| &m.metrics[x]) else {
                continue;
            };
            if !matches!(
                metric.kind,
                MetricKind::Base {
                    numerical: true,
                    ..
                }
            ) {
                return Err(CompileError(format!(
                    "IllegalStateException: Cannot initialize measurements aggregation defined by MeasurementSpecification with id '{}': So far, only NumericalBaseMetricDescriptions are supported for fixed and variable size aggregation!",
                    spec.id
                )));
            }
            let Some(name) = tuple_metric_name(&metric.id) else {
                continue;
            };
            let meas = PrmMeas {
                mp: Some(mp.obj),
                spec: spec.obj,
                key: format!("{}|{name}", mp_key(m, mp).unwrap_or_default()).into(),
            };
            if let MeasuringPointKind::AssemblyPassiveResource {
                assembly: Some(a),
                passive_resource: Some(p),
            } = &mp.kind
            {
                let ac_name = &m.assembly_contexts[*a].name;
                prm_passive.push(PassivePrm {
                    rep: format!(
                        "Passive Resource: {}.{}",
                        ac_name, m.passive_resources[*p].name
                    )
                    .into(),
                    metric: name,
                    rec,
                    invalid: invalid.map(Into::into),
                    meas,
                });
                continue;
            }
            let Some(key) = mp_key(m, mp) else { continue };
            let Some(&s) = series_map.get(&(key, name)) else {
                continue;
            };
            // utilisation: only the overall-utilisation calculator, not a window's output
            if name == metric_names::UTILIZATION
                && !resources.iter().any(|r| r.overall_series == Some(s))
            {
                continue;
            }
            // the aggregator is created (and checked) when its calculator exists
            if let Some(e) = invalid {
                return Err(CompileError(e));
            }
            prm_series.push((s, rec, prm_meas.len() as u32));
            prm_meas.push(meas);
        }
        // then the sliding-window decorators: time-driven aggregation, utilisation
        for (w, src) in windows.iter_mut().zip(&window_src) {
            if w.prm {
                w.prm_meas = prm_meas.len() as u32;
                prm_meas.push(PrmMeas {
                    mp: src.mp,
                    spec: src.spec,
                    key: src.label.clone(),
                });
            }
        }
        // execution-result tuples (MEAS-7.4) need an explicit monitor with this metric; they are
        // textual (SUCCESS/FAILURE) and not produced here
        if sim_monitors.iter().any(|mon| {
            mon.specifications
                .iter()
                .any(|s| metric_id(m, s) == Some(metric_ids::EXECUTION_RESULT_TYPE_TUPLE))
        }) {
            warnings.push(
                "'Execution Result Type over Time' measurements are not produced (MEAS-7.4)".into(),
            );
        }
        let stream_bytesize = b.prog("stream.BYTESIZE");
        if ss {
            ss_actions(m, sys_structure, &mut b, &mut ss_tables);
        }
        let prm_any =
            !prm_series.is_empty() || !prm_passive.is_empty() || windows.iter().any(|w| w.prm);

        let mut sig_key_map: HashMap<&str, u32> = HashMap::new();
        let sig_key: Vec<u32> = m
            .signatures
            .iter()
            .map(|sg| {
                let n = sig_key_map.len() as u32;
                *sig_key_map.entry(&*sg.id).or_insert(n)
            })
            .collect();
        let mut seff_for: FxMap<(ComponentId, u32), Vec<SeffId>> = FxMap::default();
        for (cid, c) in m.components.iter_ids() {
            for &sf in c.seffs() {
                if let Some(sg) = m.seffs[sf].signature {
                    seff_for
                        .entry((cid, sig_key[sg.index()]))
                        .or_default()
                        .push(sf);
                }
            }
        }
        let comp_params = m
            .components
            .iter()
            .map(|c| b.usages(&c.parameter_usages))
            .collect();
        let ac_params = m
            .assembly_contexts
            .iter()
            .map(|a| b.usages(&a.config_parameters))
            .collect();
        let pr_capacity = m
            .passive_resources
            .iter()
            .map(|p| b.prog(&p.capacity.spec))
            .collect();
        let ac_ids = m
            .assembly_contexts
            .iter()
            .map(|a| trace_id(m, &a.id, a.obj))
            .collect();
        let role_ids = m.roles.iter().map(|r| trace_id(m, &r.id, r.obj)).collect();
        let sig_ids = m
            .signatures
            .iter()
            .map(|r| trace_id(m, &r.id, r.obj))
            .collect();
        let scenario_ids = m
            .usage_scenarios
            .iter()
            .map(|r| trace_id(m, &r.id, r.obj))
            .collect();
        let pr_ids = m
            .passive_resources
            .iter()
            .map(|r| trace_id(m, &r.id, r.obj))
            .collect();
        let route_res = route_table(
            m.containers.len(),
            links
                .iter()
                .map(|l| (link_res[l], &m.linking_resources[*l].connected[..])),
        );
        let container_res_dense =
            dense_container_res(&container_res, m.containers.len(), m.resource_types.len());
        let Builder { keys, progs, .. } = b;
        let code = crate::code::Code::build(&beh, &act, &ubeh, &uact);
        let mut series_by_mp: HashMap<Box<str>, Vec<(&'static str, SeriesId)>> = HashMap::new();
        for (i, sd) in series.iter().enumerate() {
            series_by_mp
                .entry(sd.mp.clone())
                .or_default()
                .push((sd.metric, i as SeriesId));
        }
        Ok(CompiledModel {
            keys,
            progs,
            beh,
            act,
            ubeh,
            uact,
            scenarios,
            system,
            prov_deleg,
            req_conn,
            seff_for,
            sig_key,
            alloc,
            ac_paths,
            ac_path_children,
            resources,
            container_res_dense,
            container_simulated,
            container_res,
            link_res,
            route_res,
            #[cfg(feature = "exact")]
            finalise_order,
            finalise_order_fast,
            links,
            registry,
            series,
            series_by_mp,
            windows,
            asmop_series,
            passive_mon,
            prm_series,
            prm_meas,
            prm_passive,
            prm_any,
            reconf_time_series,
            container_count,
            comp_params,
            ac_params,
            pr_capacity,
            stream_bytesize,
            action_ids,
            uaction_ids,
            ac_ids,
            role_ids,
            sig_ids,
            scenario_ids,
            pr_ids,
            action_type: action_type_v,
            uaction_type: uaction_type_v,
            code,
            warnings,
            model,
            flavour,
            ss: ss_tables,
        })
    }

    /// The trie child of path `parent` for assembly context `ac` (`None`: not a reachable path).
    #[inline]
    pub fn path_child(&self, parent: u32, ac: AcRef) -> Option<u32> {
        self.ac_path_children
            .get(ac as usize)?
            .iter()
            .find(|e| e.0 == parent)
            .map(|e| e.1)
    }

    /// The resource of type `rt` in container `c`.
    #[inline]
    pub fn resource_in(&self, c: ContainerId, rt: ResourceTypeId) -> Option<ResIdx> {
        let n = self.model.resource_types.len();
        match self.container_res_dense.get(c.index() * n + rt.index()) {
            Some(&r) if r != u32::MAX => Some(r),
            _ => None,
        }
    }

    /// Trace id of an assembly context reference.
    pub fn ac_id(&self, ac: AcRef) -> &str {
        if ac == SYSTEM_AC {
            SYSTEM_AC_ID
        } else {
            &self.ac_ids[ac as usize]
        }
    }
}

/// Route table `[src * n_cont + dst]` (see [`CompiledModel::route_res`]): for each pair of
/// containers the resource of the first link (in `links` order) connecting both; empty for more
/// than 2 048 containers. O(sum of |connected|^2) instead of O(n_cont^2 * links).
pub(crate) fn route_table<'a>(
    n_cont: usize,
    links: impl Iterator<Item = (ResIdx, &'a [ContainerId])>,
) -> Vec<ResIdx> {
    if n_cont > 2048 {
        return Vec::new();
    }
    let mut route_res = vec![u32::MAX; n_cont * n_cont];
    for (r, connected) in links {
        let mut c: Vec<usize> = connected.iter().map(|x| x.index()).collect();
        c.sort_unstable();
        c.dedup();
        for &a in &c {
            for &b in &c {
                let e = &mut route_res[a * n_cont + b];
                if *e == u32::MAX {
                    *e = r;
                }
            }
        }
    }
    route_res
}

/// Dense form of `container_res` (see [`CompiledModel::container_res_dense`]).
pub(crate) fn dense_container_res(
    container_res: &FxMap<(ContainerId, ResourceTypeId), ResIdx>,
    n_cont: usize,
    n_types: usize,
) -> Vec<ResIdx> {
    let mut dense = vec![u32::MAX; n_cont * n_types];
    for (&(c, rt), &r) in container_res {
        dense[c.index() * n_types + rt.index()] = r;
    }
    dense
}

/// `deactivateAllActiveResources` order: the `HashMap` order `O` (Java's in the exact mode) of
/// the registry's container ids, then of each container's resource type ids.
pub(crate) fn finalise_order<O: MapOrder>(registry: &[RegistryEntry]) -> Vec<ResIdx> {
    let mut order = Vec::new();
    let hashes: Vec<i32> = registry.iter().map(|(k, _)| string_hash(k)).collect();
    for i in O::order(&hashes) {
        let types = &registry[i].1;
        let th: Vec<i32> = types.iter().map(|(k, _)| string_hash(k)).collect();
        for j in O::order(&th) {
            order.push(types[j].1);
        }
    }
    order
}

/// `AllocationLookupSyncer.doAddAllocationContext`: the lookup entries of allocation context
/// `ctx` (its assembly context's FQ path and every path nested below it -> its container;
/// later entries overwrite). Nothing without an assembly context or container.
pub(crate) fn allocate_context(
    m: &Model,
    sys_structure: StructureId,
    ctx: AllocationContextId,
    alloc: &mut FxMap<Box<[u32]>, ContainerId>,
) -> Result<(), CompileError> {
    allocate_context_mode(m, sys_structure, ctx, alloc, AllocMode::LastNested)
}

/// How allocation contexts map assembly-context paths to containers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AllocMode {
    /// SimuLizar: a later allocation context of the same path overwrites an earlier one; the
    /// inner assembly contexts of a composite are allocated with it (ACT-2.8).
    LastNested,
    /// Slingshot: the first allocation context wins (SS-ACT-9.1); inner contexts as SimuLizar
    /// (the fixed behaviour of Q30).
    FirstNested,
    /// Slingshot with Q30: the first wins; inner assembly contexts stay unallocated (SS-ACT-1.5).
    FirstFlat,
}

pub(crate) fn allocate_context_mode(
    m: &Model,
    sys_structure: StructureId,
    ctx: AllocationContextId,
    alloc: &mut FxMap<Box<[u32]>, ContainerId>,
    mode: AllocMode,
) -> Result<(), CompileError> {
    let c = &m.allocation_contexts[ctx];
    if let (Some(ac), Some(cont)) = (c.assembly, c.container) {
        let mut base = if m.assembly_contexts[ac].parent == Some(sys_structure) {
            vec![ac.0]
        } else {
            let mut out = Vec::new();
            paths_to(
                m,
                &m.structures[sys_structure].assembly_contexts,
                ac,
                &mut Vec::new(),
                &mut out,
            );
            if out.len() != 1 {
                return Err(CompileError(format!(
                    "Cannot determine unique path to nested assembly context {}",
                    m.assembly_contexts[ac].id
                )));
            }
            out.pop().expect("one path")
        };
        add_nested(m, &mut base, cont, alloc, 0, mode);
    }
    Ok(())
}

fn add_nested(
    m: &Model,
    base: &mut Vec<u32>,
    cont: ContainerId,
    alloc: &mut FxMap<Box<[u32]>, ContainerId>,
    depth: usize,
    mode: AllocMode,
) {
    match mode {
        AllocMode::LastNested => {
            alloc.insert(base.clone().into_boxed_slice(), cont);
        }
        AllocMode::FirstNested | AllocMode::FirstFlat => {
            alloc.entry(base.clone().into_boxed_slice()).or_insert(cont);
        }
    }
    let ac = AssemblyContextId(*base.last().expect("path"));
    if depth < 32
        && mode != AllocMode::FirstFlat
        && let Some(comp) = m.assembly_contexts[ac].component
        && let Some(st) = m.components[comp].structure()
    {
        for &inner in &m.structures[st].assembly_contexts {
            base.push(inner.0);
            add_nested(m, base, cont, alloc, depth + 1, mode);
            base.pop();
        }
    }
}

/// All paths from the system to a nested assembly context (`determineBaseAssemblyPath`).
fn paths_to(
    m: &Model,
    from: &[AssemblyContextId],
    target: AssemblyContextId,
    cur: &mut Vec<u32>,
    out: &mut Vec<Vec<u32>>,
) {
    for &ac in from {
        if out.len() > 100 || cur.len() > 32 {
            return;
        }
        cur.push(ac.0);
        if ac == target {
            out.push(cur.clone());
        } else if let Some(comp) = m.assembly_contexts[ac].component
            && let Some(st) = m.components[comp].structure()
        {
            paths_to(m, &m.structures[st].assembly_contexts, target, cur, out);
        }
        cur.pop();
    }
}

/// Upper bound of `numberOfReplicas` of a processing resource (per-replica state is
/// allocated up front).
const MAX_REPLICAS: i64 = 100_000;

/// Upper bound of assembly-context paths (nested assembly contexts, counted once per path from
/// the system): the compiler enumerates them all.
const MAX_AC_PATHS: u64 = 1_000_000;

/// Rejects composite components that contain themselves (directly or indirectly) and
/// compositions with more than [`MAX_AC_PATHS`] nested assembly-context paths. Both would make
/// the path enumeration below explode (the reference recurses without end, too).
fn check_composition(m: &Model, sys: StructureId) -> Result<(), CompileError> {
    // paths[st]: number of paths below structure `st` (None = not computed yet); iterative DFS
    // with an explicit stack, `on_stack` marks structures being expanded (cycle detection)
    let n = m.structures.len();
    let mut paths: Vec<Option<u64>> = vec![None; n];
    let mut on_stack = vec![false; n];
    let inner = |ac: AssemblyContextId| {
        m.assembly_contexts[ac]
            .component
            .and_then(|c| m.components[c].structure())
    };
    let mut stack: Vec<StructureId> = vec![sys];
    while let Some(&st) = stack.last() {
        if paths[st.index()].is_some() {
            stack.pop();
            continue;
        }
        if !on_stack[st.index()] {
            on_stack[st.index()] = true;
            for &ac in &m.structures[st].assembly_contexts {
                if let Some(i) = inner(ac) {
                    if on_stack[i.index()] {
                        return Err(CompileError(format!(
                            "composite component contains itself (assembly context {})",
                            m.assembly_contexts[ac].id
                        )));
                    }
                    if paths[i.index()].is_none() {
                        stack.push(i);
                    }
                }
            }
            continue;
        }
        // all children computed
        let mut total: u64 = 0;
        for &ac in &m.structures[st].assembly_contexts {
            let below = inner(ac).map_or(0, |i| paths[i.index()].unwrap_or(0));
            total = total.saturating_add(1).saturating_add(below);
        }
        if total > MAX_AC_PATHS {
            return Err(CompileError(format!(
                "composition too large: more than {MAX_AC_PATHS} nested assembly-context paths"
            )));
        }
        paths[st.index()] = Some(total);
        on_stack[st.index()] = false;
        stack.pop();
    }
    Ok(())
}

// ------------------------------------------------------------------------------------------
// Slingshot flavour tables (`docs/spec/slingshot/`)

/// What a Slingshot active-resource monitor samples at every resource event (SS-MEAS-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub enum SsProbe {
    /// `requestsAtResource`: all jobs at the resource (SS-MEAS-3.2).
    State,
    /// Busy cores / cores (SS-MEAS-3.3).
    Util,
    /// The jobs `reportCoreUsage` attributes to core `k` (fixed Q34, a multi-core PS resource).
    CoreState(u32),
    /// Core `k` busy: 1 or 0 (fixed Q34).
    CoreUtil(u32),
}

/// A passive-resource metric of Slingshot's monitor (SS-MEAS-4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub enum SsPassiveMetric {
    Waiting,
    Holding,
    State,
}

/// A Slingshot passive-resource monitor.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct SsPassiveMon {
    pub pr: PassiveResourceId,
    /// The assembly context of the measuring point; `None`: every instance of the passive
    /// resource (Q34: probes per passive resource).
    pub ac: Option<AssemblyContextId>,
    pub metric: SsPassiveMetric,
    pub series: SeriesId,
}

/// The tables of the Slingshot flavour (empty for SimuLizar).
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct SsTables {
    /// Per resource (`ResIdx`): the active-resource monitors, in registration order.
    pub res_mon: Vec<Vec<(SeriesId, SsProbe)>>,
    /// Per resource (`ResIdx`): the failure probability of a linking resource (0 otherwise).
    pub link_failure: Vec<f64>,
    /// Per SEFF action: the amount of an `AcquireAction`/`ReleaseAction`, its first
    /// `ParametricResourceDemand` (SS-ACT-8.1).
    pub passive_amount: Vec<Option<ProgId>>,
    /// Per SEFF action: a `ForkAction` with a `SynchronisationPoint`.
    pub fork_sync_point: Vec<bool>,
    /// Passive-resource monitors.
    pub passive_mon: Vec<SsPassiveMon>,
    /// Q20: the first assembly connector of the system per required-role id.
    #[cfg_attr(
        feature = "snapshot",
        serde(with = "crate::snapshot::serde_util::sorted")
    )]
    pub sys_req_conn: HashMap<Box<str>, ConnectorId>,
    /// An `AssemblyOperationMeasuringPoint` response-time monitor exists (Q31).
    pub asmop_monitored: bool,
}

/// Measuring-point key of the Slingshot flavour: [`mp_key`] plus the points only Slingshot
/// records (`AssemblyPassiveResourceMeasuringPoint[assembly|passive resource]`, the reference
/// runner's `Keys.mpKey`).
fn ss_mp_key(m: &Model, mp: &MeasuringPoint) -> Option<String> {
    match &mp.kind {
        MeasuringPointKind::AssemblyPassiveResource {
            assembly,
            passive_resource,
        } => Some(format!(
            "AssemblyPassiveResourceMeasuringPoint[{}|{}]",
            m.assembly_contexts[(*assembly)?].id,
            m.passive_resources[(*passive_resource)?].id
        )),
        _ => mp_key(m, mp),
    }
}

/// Resources and resource monitors of the Slingshot flavour (SS-RES-1, SS-MEAS-3, SS-MEAS-4).
#[allow(clippy::too_many_arguments)]
fn ss_resources(
    m: &Model,
    env: Option<ResourceEnvironmentId>,
    quirks: crate::flavour::Quirks,
    monitors: &[&Monitor],
    b: &mut Builder,
    series: &mut Vec<SeriesDef>,
    series_map: &mut HashMap<(String, &'static str), SeriesId>,
    resources: &mut Vec<CRes>,
    container_res: &mut FxMap<(ContainerId, ResourceTypeId), ResIdx>,
    link_res: &mut FxMap<LinkingResourceId, ResIdx>,
    links: &mut Vec<LinkingResourceId>,
    registry: &mut Vec<(String, Vec<(String, ResIdx)>)>,
    container_simulated: &mut [bool],
    t: &mut SsTables,
) -> Result<(), CompileError> {
    use crate::flavour::Quirk;
    let mut new_series = |mp: String, metric: &'static str| -> SeriesId {
        *series_map.entry((mp.clone(), metric)).or_insert_with(|| {
            series.push(SeriesDef {
                mp: mp.into(),
                metric,
            });
            (series.len() - 1) as SeriesId
        })
    };
    // the containers of the allocation contexts, in allocation order (ActiveResourceTable)
    let mut conts: Vec<ContainerId> = Vec::new();
    for al in m.allocations.iter() {
        for &ctx in &al.contexts {
            if let Some(c) = m.allocation_contexts[ctx].container
                && !conts.contains(&c)
            {
                conts.push(c);
            }
        }
    }
    for cid in conts {
        let cont = &m.containers[cid];
        let mut types: Vec<(String, ResIdx)> = Vec::new();
        for &prid in &cont.processing_resources {
            let pr = &m.processing_resources[prid];
            let Some(rt) = pr.resource_type else {
                continue;
            };
            let pol_id = pr
                .scheduling
                .map(|s| m.scheduling_policies[s].id.to_string())
                .unwrap_or_default();
            // FCFS and processor sharing; Delay only with the fixed Q25; anything else: no
            // resource (a demand on it aborts the user, SS-RES-1.2)
            let policy = match SchedulingPolicy::from_pcm_id(&pol_id) {
                Some(SchedulingPolicy::Delay) if quirks.has(Quirk::Q25) => continue,
                Some(p) => p,
                None => continue,
            };
            if pr.replicas > MAX_REPLICAS {
                return Err(CompileError(format!(
                    "numberOfReplicas {} of {} exceeds the supported maximum {MAX_REPLICAS}",
                    pr.replicas, pr.id
                )));
            }
            let instances = pr.replicas.max(1) as u32;
            let r = CRes {
                kind: ResKind::Processing(prid),
                policy,
                instances,
                rate: b.prog(&pr.processing_rate.spec),
                latency: None,
                // Slingshot ignores the HDD read and write rates (SS-RES-0.2)
                hdd: None,
                type_id: m.resource_types[rt].id.clone(),
                rc: cont.id.clone(),
                spec_name: pr.id.clone(),
                sched_name: pol_id.as_str().into(),
                origin: format!("rate:{}", cont.id).into(),
                state_series: vec![Vec::new(); instances as usize],
                overall_series: None,
                demand_series: Vec::new(),
                initial_states: Vec::new(),
            };
            container_simulated[cid.index()] = true;
            match container_res.get(&(cid, rt)) {
                // a later specification of the same type replaces the earlier one
                Some(&existing) => resources[existing as usize] = r,
                None => {
                    let idx = resources.len() as ResIdx;
                    resources.push(r);
                    container_res.insert((cid, rt), idx);
                    types.push((m.resource_types[rt].id.to_string(), idx));
                }
            }
        }
        registry.push((cont.id.to_string(), types));
    }
    t.link_failure = vec![0.0; resources.len()];
    if let Some(env) = env {
        for &lid in &m.resource_environments[env].linking_resources {
            let l = &m.linking_resources[lid];
            links.push(lid);
            let idx = resources.len() as ResIdx;
            let (type_id, type_name) = l
                .resource_type
                .map(|t| {
                    (
                        m.resource_types[t].id.clone(),
                        m.resource_types[t].name.clone(),
                    )
                })
                .unwrap_or_default();
            resources.push(CRes {
                kind: ResKind::Link(lid),
                policy: SchedulingPolicy::Fcfs,
                instances: 1,
                rate: b.prog(&l.throughput.spec),
                latency: Some(b.prog(&l.latency.spec)),
                hdd: None,
                type_id: type_id.clone(),
                rc: l.id.clone(),
                spec_name: type_name,
                sched_name: "FCFS".into(),
                origin: format!("rate:{}", l.id).into(),
                state_series: vec![Vec::new()],
                overall_series: None,
                demand_series: Vec::new(),
                initial_states: Vec::new(),
            });
            t.link_failure.push(l.failure_probability);
            link_res.insert(lid, idx);
            registry.push((l.id.to_string(), vec![(type_id.to_string(), idx)]));
        }
    }
    // monitors, in repository order (MonitorRepositoryInterpreterBehavior)
    t.res_mon = vec![Vec::new(); resources.len()];
    for mon in monitors.iter().filter(|x| x.activated) {
        let Some(mp) = mon.measuring_point.map(|x| &m.measuring_points[x]) else {
            continue;
        };
        match &mp.kind {
            MeasuringPointKind::ActiveResource {
                resource: Some(mres),
                replica_id,
            } => {
                let Some(key) = ss_mp_key(m, mp) else {
                    continue;
                };
                // probes per ProcessingResourceSpecification (ActiveResourceProbeTable)
                let idx = resources.iter().position(|r| {
                    matches!(r.kind, ResKind::Processing(p)
                        if m.processing_resources[p].id == m.processing_resources[*mres].id)
                });
                for spec in &mon.specifications {
                    let (metric, util) = match metric_id(m, spec) {
                        Some(metric_ids::STATE_OF_ACTIVE_RESOURCE) => {
                            (metric_names::STATE_OF_ACTIVE_RESOURCE, false)
                        }
                        Some(metric_ids::UTILIZATION) => (metric_names::UTILIZATION, true),
                        Some(metric_ids::RESOURCE_DEMAND) => {
                            let s = new_series(key.clone(), metric_names::RESOURCE_DEMAND);
                            if let Some(i) = idx {
                                resources[i].demand_series.push(s);
                            }
                            continue;
                        }
                        _ => continue,
                    };
                    let s = new_series(key.clone(), metric);
                    let Some(i) = idx else { continue };
                    let r = &resources[i];
                    let probe = if quirks.has(Quirk::Q34) {
                        Some(if util { SsProbe::Util } else { SsProbe::State })
                    } else if *replica_id < 0 || *replica_id >= i64::from(r.instances) {
                        None
                    } else if r.policy == SchedulingPolicy::ProcessorSharing && r.instances > 1 {
                        let k = *replica_id as u32;
                        Some(if util {
                            SsProbe::CoreUtil(k)
                        } else {
                            SsProbe::CoreState(k)
                        })
                    } else {
                        Some(if util { SsProbe::Util } else { SsProbe::State })
                    };
                    if let Some(p) = probe {
                        t.res_mon[i].push((s, p));
                    }
                }
            }
            MeasuringPointKind::AssemblyPassiveResource {
                assembly: Some(a),
                passive_resource: Some(p),
            } => {
                let Some(key) = ss_mp_key(m, mp) else {
                    continue;
                };
                for spec in &mon.specifications {
                    let (metric, name) = match metric_id(m, spec) {
                        Some(metric_ids::WAITING_TIME) => {
                            (SsPassiveMetric::Waiting, metric_names::WAITING_TIME)
                        }
                        Some(metric_ids::HOLDING_TIME) => {
                            (SsPassiveMetric::Holding, metric_names::HOLDING_TIME)
                        }
                        Some(metric_ids::STATE_OF_PASSIVE_RESOURCE) => (
                            SsPassiveMetric::State,
                            metric_names::STATE_OF_PASSIVE_RESOURCE,
                        ),
                        _ => continue,
                    };
                    let s = new_series(key.clone(), name);
                    t.passive_mon.push(SsPassiveMon {
                        pr: *p,
                        ac: (!quirks.has(Quirk::Q34)).then_some(*a),
                        metric,
                        series: s,
                    });
                }
            }
            MeasuringPointKind::AssemblyOperation { .. } => {
                t.asmop_monitored |= mon
                    .specifications
                    .iter()
                    .any(|s| metric_id(m, s) == Some(metric_ids::RESPONSE_TIME));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Per-action tables of the Slingshot flavour.
fn ss_actions(m: &Model, sys: StructureId, b: &mut Builder, t: &mut SsTables) {
    t.passive_amount = m
        .actions
        .iter()
        .map(|a| match a.kind {
            ActionKind::Acquire { .. } | ActionKind::Release { .. } => {
                a.resource_demands.first().map(|d| b.prog(&d.spec.spec))
            }
            _ => None,
        })
        .collect();
    t.fork_sync_point = m
        .actions
        .iter()
        .map(|a| {
            matches!(
                &a.kind,
                ActionKind::Fork {
                    synchronisation: Some(_),
                    ..
                }
            )
        })
        .collect();
    for &c in &m.structures[sys].connectors {
        if let ConnectorKind::Assembly {
            required_role: Some(r),
            ..
        } = &m.connectors[c].kind
        {
            t.sys_req_conn.entry(m.roles[*r].id.clone()).or_insert(c);
        }
    }
}

//! Intermediate representation of a generated PCM model (written to XMI by [`super::xmi`]).

use crate::runcfg::RunConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResType {
    Cpu,
    Hdd,
    Delay,
}

impl ResType {
    /// Id in `Palladio.resourcetype`.
    pub fn type_id(self) -> &'static str {
        match self {
            ResType::Cpu => "_oro4gG3fEdy4YaaT-RYrLQ",
            ResType::Hdd => "_BIjHoQ3KEdyouMqirZIhzQ",
            ResType::Delay => "_nvHX4KkREdyEA_b89s7q9w",
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            ResType::Cpu => "CPU",
            ResType::Hdd => "HDD",
            ResType::Delay => "DELAY",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sched {
    Ps,
    Fcfs,
    Delay,
}

impl Sched {
    /// Fragment in `Palladio.resourcetype`.
    pub fn policy(self) -> &'static str {
        match self {
            Sched::Ps => "ProcessorSharing",
            Sched::Fcfs => "FCFS",
            Sched::Delay => "Delay",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProcRes {
    pub id: String,
    pub ty: ResType,
    pub sched: Sched,
    pub rate: String,
    pub rate_mean: f64,
    pub replicas: u32,
    /// `HDDProcessingResourceSpecification` read and write processing rates (HDD only).
    pub hdd_rates: Option<(String, String)>,
}

#[derive(Clone, Debug)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub res: Vec<ProcRes>,
}

#[derive(Clone, Debug)]
pub struct Link {
    pub id: String,
    pub spec_id: String,
    pub name: String,
    pub latency: String,
    pub latency_mean: f64,
    pub throughput: String,
    pub throughput_mean: f64,
    pub containers: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamKind {
    Int,
    Bytes,
    Coll,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub kind: ParamKind,
}

#[derive(Clone, Debug)]
pub struct Signature {
    pub id: String,
    pub name: String,
    pub params: Vec<Param>,
    pub returns: bool,
}

#[derive(Clone, Debug)]
pub struct Interface {
    pub id: String,
    pub name: String,
    pub infra: bool,
    pub sigs: Vec<Signature>,
}

/// Characterisation type of a variable usage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Char {
    Value,
    Bytesize,
    NumberOfElements,
}

impl Char {
    pub fn name(self) -> &'static str {
        match self {
            Char::Value => "VALUE",
            Char::Bytesize => "BYTESIZE",
            Char::NumberOfElements => "NUMBER_OF_ELEMENTS",
        }
    }
}

/// `name` may be dotted (`items.INNER`); one usage can carry several characterisations.
#[derive(Clone, Debug)]
pub struct VarUsage {
    pub name: String,
    pub chars: Vec<(Char, String)>,
}

#[derive(Clone, Debug)]
pub struct Role {
    pub id: String,
    pub name: String,
    pub iface: usize,
}

#[derive(Clone, Debug)]
pub struct Passive {
    pub id: String,
    pub name: String,
    pub capacity: String,
}

#[derive(Clone, Debug)]
pub struct Behaviour {
    pub id: String,
    pub start_id: String,
    pub stop_id: String,
    pub actions: Vec<Action>,
}

#[derive(Clone, Debug)]
pub struct InfraCall {
    pub id: String,
    pub role: String,
    pub sig: String,
    pub count: String,
    pub inputs: Vec<VarUsage>,
}

#[derive(Clone, Debug)]
pub struct ResCall {
    pub id: String,
    /// Resource required role of the component.
    pub role: String,
    pub sig: ResSig,
    pub count: String,
}

/// Resource signature of a resource call (`Palladio.resourcetype`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResSig {
    CpuProcess,
    HddRead,
    HddWrite,
}

impl ResSig {
    pub fn id(self) -> &'static str {
        match self {
            ResSig::CpuProcess => "_wF22kE5CEeCUKeckjJ_n-w",
            ResSig::HddRead => "_ygMyEE5CEeCUKeckjJ_n-w",
            ResSig::HddWrite => "_zUFtIE5CEeCUKeckjJ_n-w",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    Internal {
        id: String,
        name: String,
        demands: Vec<(ResType, String)>,
        infra: Vec<InfraCall>,
        rescalls: Vec<ResCall>,
    },
    External {
        id: String,
        name: String,
        role: String,
        sig: String,
        inputs: Vec<VarUsage>,
        returns: Vec<VarUsage>,
    },
    ProbBranch {
        id: String,
        name: String,
        /// (transition id, probability, body)
        trans: Vec<(String, String, Behaviour)>,
    },
    GuardedBranch {
        id: String,
        name: String,
        /// (transition id, condition, body)
        trans: Vec<(String, String, Behaviour)>,
    },
    Loop {
        id: String,
        name: String,
        count: String,
        body: Behaviour,
    },
    Iterate {
        id: String,
        name: String,
        /// XMI path of the iterated parameter (`//@interfaces__Repository.i/@signatures.../@parameters...`).
        param_path: String,
        body: Behaviour,
    },
    Fork {
        id: String,
        name: String,
        asyncs: Vec<Behaviour>,
        /// (synchronisation point id, behaviours)
        sync: Option<(String, Vec<Behaviour>)>,
    },
    Acquire {
        id: String,
        name: String,
        pr: String,
    },
    Release {
        id: String,
        name: String,
        pr: String,
    },
    SetVar {
        id: String,
        name: String,
        usages: Vec<VarUsage>,
    },
}

impl Action {
    pub fn id(&self) -> &str {
        match self {
            Action::Internal { id, .. }
            | Action::External { id, .. }
            | Action::ProbBranch { id, .. }
            | Action::GuardedBranch { id, .. }
            | Action::Loop { id, .. }
            | Action::Iterate { id, .. }
            | Action::Fork { id, .. }
            | Action::Acquire { id, .. }
            | Action::Release { id, .. }
            | Action::SetVar { id, .. } => id,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Seff {
    pub id: String,
    pub sig: String,
    pub body: Behaviour,
}

#[derive(Clone, Debug)]
pub struct BasicComp {
    pub id: String,
    pub name: String,
    pub provides: Vec<Role>,
    pub requires: Vec<Role>,
    /// Resource required role (CPU interface) for resource calls.
    pub rreq: Option<String>,
    /// Resource required role (HDD interface) for HDD read/write resource calls.
    pub rreq_hdd: Option<String>,
    pub passive: Vec<Passive>,
    pub comp_params: Vec<VarUsage>,
    pub seffs: Vec<Seff>,
}

#[derive(Clone, Debug)]
pub enum Conn {
    Assembly {
        id: String,
        req_ac: String,
        req_role: String,
        prov_ac: String,
        prov_role: String,
    },
    Infra {
        id: String,
        req_ac: String,
        req_role: String,
        prov_ac: String,
        prov_role: String,
    },
    ProvDeleg {
        id: String,
        outer_role: String,
        ac: String,
        inner_role: String,
    },
    ReqDeleg {
        id: String,
        inner_role: String,
        ac: String,
        outer_role: String,
    },
}

#[derive(Clone, Debug)]
pub struct Assembly {
    pub id: String,
    pub name: String,
    pub comp: usize,
    pub config: Vec<VarUsage>,
}

#[derive(Clone, Debug)]
pub struct CompositeComp {
    pub id: String,
    pub name: String,
    pub provides: Vec<Role>,
    pub requires: Vec<Role>,
    pub inner: Vec<Assembly>,
    pub conns: Vec<Conn>,
}

#[derive(Clone, Debug)]
pub enum Component {
    Basic(BasicComp),
    Composite(CompositeComp),
}

impl Component {
    pub fn id(&self) -> &str {
        match self {
            Component::Basic(b) => &b.id,
            Component::Composite(c) => &c.id,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Component::Basic(b) => &b.name,
            Component::Composite(c) => &c.name,
        }
    }
}

#[derive(Clone, Debug)]
pub enum UAction {
    Call {
        id: String,
        name: String,
        sys_role: String,
        sig: String,
        inputs: Vec<VarUsage>,
    },
    Delay {
        id: String,
        name: String,
        spec: String,
    },
    Branch {
        id: String,
        name: String,
        trans: Vec<(String, UBehaviour)>,
    },
    Loop {
        id: String,
        name: String,
        count: String,
        body: UBehaviour,
    },
}

#[derive(Clone, Debug)]
pub struct UBehaviour {
    pub id: String,
    pub start_id: String,
    pub stop_id: String,
    pub actions: Vec<UAction>,
}

#[derive(Clone, Debug)]
pub enum Workload {
    Open { interarrival: String },
    Closed { population: u32, think: String },
}

#[derive(Clone, Debug)]
pub struct Scenario {
    pub id: String,
    pub name: String,
    pub workload: Workload,
    pub body: UBehaviour,
}

#[derive(Clone, Debug)]
pub struct SysRole {
    pub id: String,
    pub name: String,
    pub iface: usize,
}

/// A complete generated model.
#[derive(Clone, Debug)]
pub struct GenModel {
    pub name: String,
    pub containers: Vec<Container>,
    pub links: Vec<Link>,
    pub interfaces: Vec<Interface>,
    /// Id of the collection data type, if used.
    pub coll_type: Option<String>,
    pub components: Vec<Component>,
    pub assemblies: Vec<Assembly>,
    pub connectors: Vec<Conn>,
    pub sys_roles: Vec<SysRole>,
    /// (allocation context id, assembly index, container index)
    pub allocation: Vec<(String, usize, usize)>,
    pub scenarios: Vec<Scenario>,
    pub run: RunConfig,
    /// Extra sliding-window monitors.
    pub windows: Vec<Window>,
    /// Response-time monitors on assembly operations: (assembly context id, context defined in the
    /// system (else in a composite of the repository), provided role id, signature id).
    pub asm_op_monitors: Vec<(String, bool, String, String)>,
    /// `triggersSelfAdaptations` of the measurement specifications.
    pub triggers: Triggers,
    /// PRM-only aggregations of scenario response times (replacing the FeedThrough).
    pub aggregations: Vec<Aggregation>,
    /// A `Reconfiguration Time` monitor on a ReconfigurationMeasuringPoint with this resource URI
    /// (required: SimuLizar's measuring-point checks dereference it): one tuple per run of the
    /// reconfiguration process.
    pub reconf_monitor: Option<String>,
    /// A `Number of Resource Containers` monitor on the resource environment.
    pub container_count_monitor: bool,
    /// A nested resource container: (parent container index, container, index into
    /// `allocation` of the allocation context moved to it).
    pub nested: Option<(usize, Container, Option<usize>)>,
    /// Features actually present (for coverage statistics).
    pub features: Vec<&'static str>,
}

/// A sliding-window monitor: utilisation (`TimeDriven`) of an active resource replica, or a
/// `TimeDrivenAggregation` (arithmetic mean) of a scenario's response time.
#[derive(Clone, Debug)]
pub struct Window {
    pub target: WindowTarget,
    pub len: f64,
    pub inc: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowTarget {
    /// (container, resource index, replica)
    Utilisation(usize, usize, u32),
    Scenario(usize),
}

/// `triggersSelfAdaptations` of the generated measurement specifications.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Triggers {
    /// All `false` (refsim's default monitors).
    Off,
    /// Attribute omitted: the EMF default `true` everywhere.
    All,
    /// `true` for the specifications whose id hashes (with this salt) below `percent`.
    Some { salt: u64, percent: u64 },
}

impl Triggers {
    /// The attribute value for the specification `id` (`None`: omit the attribute).
    pub fn value(&self, id: &str) -> Option<bool> {
        match *self {
            Triggers::Off => Some(false),
            Triggers::All => None,
            Triggers::Some { salt, percent } => {
                // FNV-1a over the id, salted
                let mut h = 0xcbf2_9ce4_8422_2325u64 ^ salt;
                for b in id.bytes() {
                    h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
                }
                Some(h % 100 < percent)
            }
        }
    }
}

/// A measurement-driven aggregation on a scenario's response time.
#[derive(Clone, Debug)]
pub struct Aggregation {
    pub scenario: usize,
    /// `FixedSizeAggregation` (`number_of_measurements`) or `VariableSizeAggregation`
    /// (`retrospection_length`).
    pub fixed: bool,
    pub frequency: u32,
    pub number_of_measurements: u32,
    pub retrospection_length: f64,
}

//! Typed, arena/index-based representation of the PCM subset needed for simulation
//! (`docs/guide/scope.md`) plus monitoring (monitor repository, measuring points, metric descriptions).
//!
//! Every element keeps `obj`, its [`ObjId`] in the generic graph ([`Model::graph`]), for tracing
//! and diagnostics, and (for PCM `Identifier`s) the original XMI `id` (empty if the file has none;
//! EMF then invents a random one). Lists are in file order; arenas are in load order (resources)
//! and containment-tree order (objects).
//! References that could not be resolved are `None` (see [`Model::diagnostics`] and
//! [`crate::validate`]). StoEx expressions are kept as raw strings ([`RandomVar::spec`]).

use crate::diag::Diagnostics;
use crate::raw::{Graph, ObjId};
use std::fmt;
use std::marker::PhantomData;

macro_rules! ids {
    ($($(#[$m:meta])* $name:ident),* $(,)?) => {$(
        $(#[$m])*
        #[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
        pub struct $name(pub u32);
        impl $name {
            #[inline]
            pub fn index(self) -> usize { self.0 as usize }
        }
        impl From<usize> for $name {
            #[inline]
            fn from(i: usize) -> Self { $name(i as u32) }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }
    )*};
}

ids!(
    RepositoryId,
    ComponentId,
    InterfaceId,
    SignatureId,
    ParameterId,
    DataTypeId,
    RoleId,
    PassiveResourceId,
    SeffId,
    BehaviourId,
    ActionId,
    StructureId,
    AssemblyContextId,
    ConnectorId,
    SystemId,
    ResourceEnvironmentId,
    ContainerId,
    ProcessingResourceId,
    LinkingResourceId,
    ResourceRepositoryId,
    ResourceTypeId,
    SchedulingPolicyId,
    ResourceInterfaceId,
    ResourceSignatureId,
    AllocationId,
    AllocationContextId,
    UsageModelId,
    UsageScenarioId,
    ScenarioBehaviourId,
    UserActionId,
    MonitorRepositoryId,
    MonitorId,
    MeasuringPointId,
    MetricId,
);

/// A typed vector indexed by `I`.
#[derive(Clone)]
pub struct Arena<I, T> {
    items: Vec<T>,
    _p: PhantomData<fn(I) -> I>,
}

impl<I, T> Default for Arena<I, T> {
    fn default() -> Self {
        Arena {
            items: Vec::new(),
            _p: PhantomData,
        }
    }
}

impl<I: From<usize> + Copy, T> Arena<I, T> {
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }
    /// `(id, element)` pairs.
    pub fn iter_ids(&self) -> impl Iterator<Item = (I, &T)> {
        self.items.iter().enumerate().map(|(i, t)| (I::from(i), t))
    }
    pub fn ids(&self) -> impl Iterator<Item = I> {
        (0..self.items.len()).map(I::from)
    }
    pub(crate) fn push(&mut self, t: T) -> I {
        self.items.push(t);
        I::from(self.items.len() - 1)
    }
    pub fn as_slice(&self) -> &[T] {
        &self.items
    }
}

macro_rules! arena_index {
    ($($id:ident => $t:ident),* $(,)?) => {$(
        impl std::ops::Index<$id> for Arena<$id, $t> {
            type Output = $t;
            #[inline]
            fn index(&self, i: $id) -> &$t { &self.items[i.index()] }
        }
        impl std::ops::IndexMut<$id> for Arena<$id, $t> {
            #[inline]
            fn index_mut(&mut self, i: $id) -> &mut $t { &mut self.items[i.index()] }
        }
        impl<'a> IntoIterator for &'a Arena<$id, $t> {
            type Item = &'a $t;
            type IntoIter = std::slice::Iter<'a, $t>;
            fn into_iter(self) -> Self::IntoIter { self.items.iter() }
        }
        impl fmt::Debug for Arena<$id, $t> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_list().entries(self.items.iter()).finish()
            }
        }
    )*};
}

arena_index!(
    RepositoryId => Repository,
    ComponentId => Component,
    InterfaceId => Interface,
    SignatureId => Signature,
    ParameterId => Parameter,
    DataTypeId => DataType,
    RoleId => Role,
    PassiveResourceId => PassiveResource,
    SeffId => Seff,
    BehaviourId => Behaviour,
    ActionId => Action,
    StructureId => Structure,
    AssemblyContextId => AssemblyContext,
    ConnectorId => Connector,
    SystemId => System,
    ResourceEnvironmentId => ResourceEnvironment,
    ContainerId => ResourceContainer,
    ProcessingResourceId => ProcessingResource,
    LinkingResourceId => LinkingResource,
    ResourceRepositoryId => ResourceRepository,
    ResourceTypeId => ResourceType,
    SchedulingPolicyId => SchedulingPolicy,
    ResourceInterfaceId => ResourceInterface,
    ResourceSignatureId => ResourceSignature,
    AllocationId => Allocation,
    AllocationContextId => AllocationContext,
    UsageModelId => UsageModel,
    UsageScenarioId => UsageScenario,
    ScenarioBehaviourId => ScenarioBehaviour,
    UserActionId => UserAction,
    MonitorRepositoryId => MonitorRepository,
    MonitorId => Monitor,
    MeasuringPointId => MeasuringPoint,
    MetricId => Metric,
);

// ---------------------------------------------------------------------------------------------
// shared value types

/// A `PCMRandomVariable`: the raw StoEx `specification` string (empty if the attribute is unset).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct RandomVar {
    pub spec: Box<str>,
    /// The `PCMRandomVariable` object; `None` if the containing reference is empty.
    pub obj: Option<ObjId>,
}

impl RandomVar {
    pub fn is_missing(&self) -> bool {
        self.obj.is_none()
    }
}

/// `VariableCharacterisationType`.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum CharacterisationType {
    Structure,
    NumberOfElements,
    Value,
    ByteSize,
    Type,
}

impl CharacterisationType {
    /// Name as used in StoEx (`a.BYTESIZE`).
    pub fn name(self) -> &'static str {
        match self {
            CharacterisationType::Structure => "STRUCTURE",
            CharacterisationType::NumberOfElements => "NUMBER_OF_ELEMENTS",
            CharacterisationType::Value => "VALUE",
            CharacterisationType::ByteSize => "BYTESIZE",
            CharacterisationType::Type => "TYPE",
        }
    }
    pub(crate) fn from_index(i: u32) -> Self {
        match i {
            0 => CharacterisationType::Structure,
            1 => CharacterisationType::NumberOfElements,
            2 => CharacterisationType::Value,
            3 => CharacterisationType::ByteSize,
            _ => CharacterisationType::Type,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VariableCharacterisation {
    pub kind: CharacterisationType,
    pub spec: RandomVar,
    pub obj: ObjId,
}

/// A `VariableUsage`: characterisations of a (possibly namespaced) variable.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct VariableUsage {
    /// Name segments of the `AbstractNamedReference` chain (`a.b.c` -> `["a","b","c"]`); empty if
    /// the reference is missing.
    pub reference: Vec<Box<str>>,
    pub characterisations: Vec<VariableCharacterisation>,
    pub obj: ObjId,
}

impl VariableUsage {
    /// Dotted reference name (`a.b.c`).
    pub fn name(&self) -> String {
        self.reference.join(".")
    }
}

// ---------------------------------------------------------------------------------------------
// repository

#[derive(Clone, Debug)]
pub struct Repository {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub description: Option<Box<str>>,
    pub components: Vec<ComponentId>,
    pub interfaces: Vec<InterfaceId>,
    pub data_types: Vec<DataTypeId>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ComponentType {
    Business,
    Infrastructure,
}

#[derive(Clone, Debug)]
pub enum ComponentKind {
    Basic {
        seffs: Vec<SeffId>,
        passive_resources: Vec<PassiveResourceId>,
    },
    Composite {
        structure: StructureId,
    },
    SubSystem {
        structure: StructureId,
    },
    CompleteType,
    ProvidesType,
}

#[derive(Clone, Debug)]
pub struct Component {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub repository: Option<RepositoryId>,
    pub kind: ComponentKind,
    pub component_type: ComponentType,
    pub provided_roles: Vec<RoleId>,
    pub required_roles: Vec<RoleId>,
    pub resource_required_roles: Vec<RoleId>,
    /// `componentParameterUsage_ImplementationComponentType` (defaults of component parameters).
    pub parameter_usages: Vec<VariableUsage>,
}

impl Component {
    pub fn seffs(&self) -> &[SeffId] {
        match &self.kind {
            ComponentKind::Basic { seffs, .. } => seffs,
            _ => &[],
        }
    }
    pub fn passive_resources(&self) -> &[PassiveResourceId] {
        match &self.kind {
            ComponentKind::Basic {
                passive_resources, ..
            } => passive_resources,
            _ => &[],
        }
    }
    pub fn structure(&self) -> Option<StructureId> {
        match &self.kind {
            ComponentKind::Composite { structure } | ComponentKind::SubSystem { structure } => {
                Some(*structure)
            }
            _ => None,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum InterfaceKind {
    Operation,
    Infrastructure,
    EventGroup,
}

#[derive(Clone, Debug)]
pub struct Interface {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub kind: InterfaceKind,
    pub repository: Option<RepositoryId>,
    pub parents: Vec<InterfaceId>,
    /// Operation, infrastructure or event-type signatures.
    pub signatures: Vec<SignatureId>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SignatureKind {
    Operation,
    Infrastructure,
    EventType,
}

#[derive(Clone, Debug)]
pub struct Signature {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub kind: SignatureKind,
    pub interface: Option<InterfaceId>,
    pub parameters: Vec<ParameterId>,
    /// Operation signatures only.
    pub return_type: Option<DataTypeId>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ParameterModifier {
    None,
    In,
    Out,
    InOut,
}

#[derive(Clone, Debug)]
pub struct Parameter {
    pub name: Box<str>,
    pub obj: ObjId,
    pub data_type: Option<DataTypeId>,
    pub modifier: ParameterModifier,
    /// Owning operation/infrastructure/event signature (None for resource signatures).
    pub signature: Option<SignatureId>,
    pub resource_signature: Option<ResourceSignatureId>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum PrimitiveType {
    Int,
    String,
    Bool,
    Double,
    Char,
    Byte,
    Long,
}

#[derive(Clone, Debug)]
pub struct InnerDeclaration {
    pub name: Box<str>,
    pub data_type: Option<DataTypeId>,
    pub obj: ObjId,
}

#[derive(Clone, Debug)]
pub enum DataTypeKind {
    Primitive(PrimitiveType),
    Collection {
        inner: Option<DataTypeId>,
    },
    Composite {
        parents: Vec<DataTypeId>,
        inner: Vec<InnerDeclaration>,
    },
}

#[derive(Clone, Debug)]
pub struct DataType {
    /// Empty for primitive types (not `Identifier`s; referenced by path).
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub repository: Option<RepositoryId>,
    pub kind: DataTypeKind,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum RoleKind {
    OperationProvided,
    OperationRequired,
    InfrastructureProvided,
    InfrastructureRequired,
    SinkProvided,
    SourceRequired,
    ResourceProvided,
    ResourceRequired,
}

impl RoleKind {
    pub fn is_provided(self) -> bool {
        matches!(
            self,
            RoleKind::OperationProvided
                | RoleKind::InfrastructureProvided
                | RoleKind::SinkProvided
                | RoleKind::ResourceProvided
        )
    }
}

/// Owner of a role (an `InterfaceProvidingRequiringEntity` or a resource type).
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum RoleOwner {
    Component(ComponentId),
    System(SystemId),
    ResourceType(ResourceTypeId),
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Role {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub kind: RoleKind,
    pub owner: RoleOwner,
    /// Operation/infrastructure interface or event group of the role.
    pub interface: Option<InterfaceId>,
    /// Resource roles only.
    pub resource_interface: Option<ResourceInterfaceId>,
}

#[derive(Clone, Debug)]
pub struct PassiveResource {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub component: Option<ComponentId>,
    pub capacity: RandomVar,
}

// ---------------------------------------------------------------------------------------------
// SEFF

#[derive(Clone, Debug)]
pub struct Seff {
    pub id: Box<str>,
    pub obj: ObjId,
    pub component: Option<ComponentId>,
    pub signature: Option<SignatureId>,
    pub seff_type_id: Box<str>,
    /// The SEFF's own step list (a `ResourceDemandingSEFF` is a `ResourceDemandingBehaviour`).
    pub behaviour: BehaviourId,
    pub internal_behaviours: Vec<BehaviourId>,
}

/// What a `ResourceDemandingBehaviour` belongs to.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BehaviourOwner {
    Seff(SeffId),
    /// `ResourceDemandingInternalBehaviour` of a SEFF.
    Internal(SeffId),
    /// Body of a loop or collection iterator action.
    Loop(ActionId),
    /// Behaviour of the `n`-th transition of a branch action.
    BranchTransition(ActionId, u32),
    /// Asynchronous forked behaviour of a fork action.
    ForkedAsync(ActionId),
    /// Synchronous forked behaviour (in the fork's synchronisation point).
    ForkedSync(ActionId),
    /// Anything else (e.g. recovery behaviours, which are out of scope).
    Other,
}

#[derive(Clone, Debug)]
pub struct Behaviour {
    /// Empty if the behaviour has no ID in the file.
    pub id: Box<str>,
    pub obj: ObjId,
    pub owner: BehaviourOwner,
    /// `steps_Behaviour` in file order.
    pub steps: Vec<ActionId>,
    /// First `StartAction` among the steps.
    pub start: Option<ActionId>,
}

#[derive(Clone, Debug)]
pub struct ResourceDemand {
    pub spec: RandomVar,
    pub resource_type: Option<ResourceTypeId>,
    pub obj: ObjId,
}

#[derive(Clone, Debug)]
pub struct InfrastructureCall {
    pub signature: Option<SignatureId>,
    pub role: Option<RoleId>,
    pub number_of_calls: RandomVar,
    pub inputs: Vec<VariableUsage>,
    pub obj: ObjId,
}

#[derive(Clone, Debug)]
pub struct ResourceCall {
    pub signature: Option<ResourceSignatureId>,
    pub role: Option<RoleId>,
    pub number_of_calls: RandomVar,
    pub inputs: Vec<VariableUsage>,
    pub obj: ObjId,
}

#[derive(Clone, Debug)]
pub enum BranchCondition {
    Probability(f64),
    Guard(RandomVar),
}

#[derive(Clone, Debug)]
pub struct BranchTransition {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub condition: BranchCondition,
    pub behaviour: Option<BehaviourId>,
}

#[derive(Clone, Debug)]
pub struct SynchronisationPoint {
    pub obj: ObjId,
    pub synchronous: Vec<BehaviourId>,
    pub outputs: Vec<VariableUsage>,
}

#[derive(Clone, Debug)]
pub enum ActionKind {
    Start,
    Stop,
    Internal,
    ExternalCall {
        signature: Option<SignatureId>,
        role: Option<RoleId>,
        inputs: Vec<VariableUsage>,
        returns: Vec<VariableUsage>,
        retry_count: i64,
    },
    Branch {
        transitions: Vec<BranchTransition>,
    },
    Loop {
        iterations: RandomVar,
        body: Option<BehaviourId>,
    },
    CollectionIterator {
        parameter: Option<ParameterId>,
        body: Option<BehaviourId>,
    },
    Fork {
        asynchronous: Vec<BehaviourId>,
        synchronisation: Option<SynchronisationPoint>,
    },
    Acquire {
        resource: Option<PassiveResourceId>,
        timeout: bool,
        timeout_value: f64,
    },
    Release {
        resource: Option<PassiveResourceId>,
    },
    SetVariable {
        usages: Vec<VariableUsage>,
    },
    InternalCall {
        behaviour: Option<BehaviourId>,
        inputs: Vec<VariableUsage>,
    },
    EmitEvent {
        event_type: Option<SignatureId>,
        role: Option<RoleId>,
        inputs: Vec<VariableUsage>,
    },
    /// An action class outside the v1 scope (e.g. `RecoveryAction`); the name of its class.
    Unsupported(&'static str),
}

#[derive(Clone, Debug)]
pub struct Action {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub behaviour: Option<BehaviourId>,
    pub predecessor: Option<ActionId>,
    pub successor: Option<ActionId>,
    pub kind: ActionKind,
    /// `AbstractInternalControlFlowAction` parts (empty for other actions).
    pub resource_demands: Vec<ResourceDemand>,
    pub infrastructure_calls: Vec<InfrastructureCall>,
    pub resource_calls: Vec<ResourceCall>,
}

// ---------------------------------------------------------------------------------------------
// composition / system

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum StructureOwner {
    System(SystemId),
    Component(ComponentId),
}

/// A `ComposedStructure` (system, composite component or subsystem).
#[derive(Clone, Debug)]
pub struct Structure {
    pub obj: ObjId,
    pub owner: StructureOwner,
    pub assembly_contexts: Vec<AssemblyContextId>,
    pub connectors: Vec<ConnectorId>,
}

#[derive(Clone, Debug)]
pub struct AssemblyContext {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub parent: Option<StructureId>,
    pub component: Option<ComponentId>,
    pub config_parameters: Vec<VariableUsage>,
}

#[derive(Clone, Debug)]
pub enum ConnectorKind {
    Assembly {
        requiring: Option<AssemblyContextId>,
        providing: Option<AssemblyContextId>,
        required_role: Option<RoleId>,
        provided_role: Option<RoleId>,
    },
    ProvidedDelegation {
        inner_role: Option<RoleId>,
        outer_role: Option<RoleId>,
        assembly: Option<AssemblyContextId>,
    },
    RequiredDelegation {
        inner_role: Option<RoleId>,
        outer_role: Option<RoleId>,
        assembly: Option<AssemblyContextId>,
    },
    AssemblyInfrastructure {
        requiring: Option<AssemblyContextId>,
        providing: Option<AssemblyContextId>,
        required_role: Option<RoleId>,
        provided_role: Option<RoleId>,
    },
    ProvidedInfrastructureDelegation {
        inner_role: Option<RoleId>,
        outer_role: Option<RoleId>,
        assembly: Option<AssemblyContextId>,
    },
    RequiredInfrastructureDelegation {
        inner_role: Option<RoleId>,
        outer_role: Option<RoleId>,
        assembly: Option<AssemblyContextId>,
    },
    RequiredResourceDelegation {
        inner_role: Option<RoleId>,
        outer_role: Option<RoleId>,
        assembly: Option<AssemblyContextId>,
    },
    /// Event connectors and other connector classes outside the v1 scope.
    Unsupported(&'static str),
}

#[derive(Clone, Debug)]
pub struct Connector {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub parent: Option<StructureId>,
    pub kind: ConnectorKind,
}

#[derive(Clone, Debug)]
pub struct System {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub structure: StructureId,
    pub provided_roles: Vec<RoleId>,
    pub required_roles: Vec<RoleId>,
    pub resource_required_roles: Vec<RoleId>,
}

// ---------------------------------------------------------------------------------------------
// resource environment / resource types

#[derive(Clone, Debug)]
pub struct ResourceEnvironment {
    pub name: Box<str>,
    pub obj: ObjId,
    /// Top-level containers.
    pub containers: Vec<ContainerId>,
    pub linking_resources: Vec<LinkingResourceId>,
}

#[derive(Clone, Debug)]
pub struct ResourceContainer {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub environment: Option<ResourceEnvironmentId>,
    pub parent: Option<ContainerId>,
    pub nested: Vec<ContainerId>,
    /// `activeResourceSpecifications_ResourceContainer` (includes HDD specifications).
    pub processing_resources: Vec<ProcessingResourceId>,
}

#[derive(Clone, Debug)]
pub struct HddRates {
    pub read: RandomVar,
    pub write: RandomVar,
}

#[derive(Clone, Debug)]
pub struct ProcessingResource {
    pub id: Box<str>,
    pub obj: ObjId,
    pub container: Option<ContainerId>,
    pub resource_type: Option<ResourceTypeId>,
    pub scheduling: Option<SchedulingPolicyId>,
    pub processing_rate: RandomVar,
    pub replicas: i64,
    pub mttf: f64,
    pub mttr: f64,
    pub required_by_container: bool,
    /// Set for `HDDProcessingResourceSpecification`.
    pub hdd: Option<HddRates>,
}

#[derive(Clone, Debug)]
pub struct LinkingResource {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub environment: Option<ResourceEnvironmentId>,
    /// In file order, duplicates kept (as EMF does).
    pub connected: Vec<ContainerId>,
    /// ID of the `CommunicationLinkResourceSpecification`.
    pub spec_id: Box<str>,
    pub resource_type: Option<ResourceTypeId>,
    pub latency: RandomVar,
    pub throughput: RandomVar,
    pub failure_probability: f64,
}

#[derive(Clone, Debug)]
pub struct ResourceRepository {
    pub obj: ObjId,
    pub resource_types: Vec<ResourceTypeId>,
    pub scheduling_policies: Vec<SchedulingPolicyId>,
    pub interfaces: Vec<ResourceInterfaceId>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ResourceTypeKind {
    Processing,
    CommunicationLink,
}

#[derive(Clone, Debug)]
pub struct ResourceType {
    /// e.g. `_oro4gG3fEdy4YaaT-RYrLQ` (CPU) in `Palladio.resourcetype`.
    pub id: Box<str>,
    /// e.g. `CPU`, `HDD`, `DELAY`, `LAN`.
    pub name: Box<str>,
    pub obj: ObjId,
    pub kind: ResourceTypeKind,
    pub provided_roles: Vec<RoleId>,
}

#[derive(Clone, Debug)]
pub struct SchedulingPolicy {
    /// e.g. `ProcessorSharing`, `FCFS`, `Delay`.
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
}

#[derive(Clone, Debug)]
pub struct ResourceInterface {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub signatures: Vec<ResourceSignatureId>,
}

#[derive(Clone, Debug)]
pub struct ResourceSignature {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub interface: Option<ResourceInterfaceId>,
    pub service_id: i64,
    pub parameters: Vec<ParameterId>,
}

// ---------------------------------------------------------------------------------------------
// allocation

#[derive(Clone, Debug)]
pub struct Allocation {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub system: Option<SystemId>,
    pub environment: Option<ResourceEnvironmentId>,
    pub contexts: Vec<AllocationContextId>,
}

#[derive(Clone, Debug)]
pub struct AllocationContext {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub allocation: Option<AllocationId>,
    pub container: Option<ContainerId>,
    pub assembly: Option<AssemblyContextId>,
}

// ---------------------------------------------------------------------------------------------
// usage model

#[derive(Clone, Debug)]
pub struct UserData {
    pub obj: ObjId,
    pub assembly: Option<AssemblyContextId>,
    pub usages: Vec<VariableUsage>,
}

#[derive(Clone, Debug)]
pub struct UsageModel {
    pub obj: ObjId,
    pub scenarios: Vec<UsageScenarioId>,
    pub user_data: Vec<UserData>,
}

#[derive(Clone, Debug)]
pub enum Workload {
    Closed {
        population: i64,
        think_time: RandomVar,
    },
    Open {
        inter_arrival_time: RandomVar,
    },
    Missing,
}

#[derive(Clone, Debug)]
pub struct UsageScenario {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub usage_model: Option<UsageModelId>,
    pub workload: Workload,
    pub behaviour: Option<ScenarioBehaviourId>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ScenarioBehaviourOwner {
    Scenario(UsageScenarioId),
    Loop(UserActionId),
    /// Behaviour of the `n`-th transition of a usage branch.
    Branch(UserActionId, u32),
    Other,
}

#[derive(Clone, Debug)]
pub struct ScenarioBehaviour {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub owner: ScenarioBehaviourOwner,
    /// `actions_ScenarioBehaviour` in file order.
    pub actions: Vec<UserActionId>,
    /// First `Start` among the actions.
    pub start: Option<UserActionId>,
}

#[derive(Clone, Debug)]
pub struct UsageBranchTransition {
    pub obj: ObjId,
    pub probability: f64,
    pub behaviour: Option<ScenarioBehaviourId>,
}

#[derive(Clone, Debug)]
pub enum UserActionKind {
    Start,
    Stop,
    EntryLevelSystemCall {
        role: Option<RoleId>,
        signature: Option<SignatureId>,
        inputs: Vec<VariableUsage>,
        outputs: Vec<VariableUsage>,
        priority: i64,
    },
    Delay {
        time: RandomVar,
    },
    Branch {
        transitions: Vec<UsageBranchTransition>,
    },
    Loop {
        iterations: RandomVar,
        body: Option<ScenarioBehaviourId>,
    },
}

#[derive(Clone, Debug)]
pub struct UserAction {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub behaviour: Option<ScenarioBehaviourId>,
    pub predecessor: Option<UserActionId>,
    pub successor: Option<UserActionId>,
    pub kind: UserActionKind,
}

// ---------------------------------------------------------------------------------------------
// monitoring

#[derive(Clone, Debug)]
pub struct MonitorRepository {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub monitors: Vec<MonitorId>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum StatisticalCharacterization {
    ArithmeticMean,
    HarmonicMean,
    GeometricMean,
    Median,
    Missing,
}

#[derive(Clone, Debug)]
pub enum ProcessingType {
    FeedThrough,
    TimeDriven {
        window_length: f64,
        window_increment: f64,
    },
    TimeDrivenAggregation {
        window_length: f64,
        window_increment: f64,
        statistic: StatisticalCharacterization,
    },
    FixedSizeAggregation {
        frequency: i64,
        number_of_measurements: i64,
        statistic: StatisticalCharacterization,
    },
    VariableSizeAggregation {
        frequency: i64,
        retrospection_length: f64,
        statistic: StatisticalCharacterization,
    },
    /// Other processing types (e.g. `map:Map`); the class name.
    Unsupported(&'static str),
    Missing,
}

#[derive(Clone, Debug)]
pub struct MeasurementSpecification {
    pub id: Box<str>,
    pub obj: ObjId,
    pub metric: Option<MetricId>,
    pub processing: ProcessingType,
    pub triggers_self_adaptations: bool,
}

#[derive(Clone, Debug)]
pub struct Monitor {
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub activated: bool,
    pub measuring_point: Option<MeasuringPointId>,
    pub specifications: Vec<MeasurementSpecification>,
}

#[derive(Clone, Debug)]
pub enum MeasuringPointKind {
    AssemblyOperation {
        assembly: Option<AssemblyContextId>,
        role: Option<RoleId>,
        signature: Option<SignatureId>,
    },
    AssemblyPassiveResource {
        assembly: Option<AssemblyContextId>,
        passive_resource: Option<PassiveResourceId>,
    },
    ActiveResource {
        resource: Option<ProcessingResourceId>,
        replica_id: i64,
    },
    SystemOperation {
        system: Option<SystemId>,
        role: Option<RoleId>,
        signature: Option<SignatureId>,
    },
    SubSystemOperation {
        subsystem: Option<ComponentId>,
        role: Option<RoleId>,
        signature: Option<SignatureId>,
    },
    LinkingResource {
        resource: Option<LinkingResourceId>,
    },
    UsageScenario {
        scenario: Option<UsageScenarioId>,
    },
    EntryLevelSystemCall {
        call: Option<UserActionId>,
    },
    ExternalCallAction {
        action: Option<ActionId>,
    },
    ResourceEnvironment {
        environment: Option<ResourceEnvironmentId>,
    },
    ResourceContainer {
        container: Option<ContainerId>,
    },
    /// `StringMeasuringPoint` / `ResourceURIMeasuringPoint` and other classes: class name and the
    /// `measuringPoint` string.
    Other {
        class: &'static str,
        measuring_point: Option<Box<str>>,
    },
}

#[derive(Clone, Debug)]
pub struct MeasuringPoint {
    pub obj: ObjId,
    pub kind: MeasuringPointKind,
    /// Stored `stringRepresentation` (EMF's getter recomputes it; informational only).
    pub stored_string: Option<Box<str>>,
}

#[derive(Clone, Debug)]
pub enum MetricKind {
    /// `NumericalBaseMetricDescription` / `TextualBaseMetricDescription`.
    Base {
        numerical: bool,
        capture_type: &'static str,
        data_type: &'static str,
        scale: &'static str,
        scope_of_validity: &'static str,
        default_unit: Option<Box<str>>,
    },
    /// `MetricSetDescription`.
    Set {
        subsumed: Vec<MetricId>,
    },
    Other(&'static str),
}

#[derive(Clone, Debug)]
pub struct Metric {
    /// e.g. `_mZb3MdoLEeO-WvSDaR6unQ` (response time).
    pub id: Box<str>,
    pub name: Box<str>,
    pub obj: ObjId,
    pub textual_description: Option<Box<str>>,
    pub kind: MetricKind,
}

// ---------------------------------------------------------------------------------------------

/// A loaded PCM model: all resources of one resource set.
#[derive(Clone, Default)]
pub struct Model {
    pub repositories: Arena<RepositoryId, Repository>,
    pub components: Arena<ComponentId, Component>,
    pub interfaces: Arena<InterfaceId, Interface>,
    pub signatures: Arena<SignatureId, Signature>,
    pub parameters: Arena<ParameterId, Parameter>,
    pub data_types: Arena<DataTypeId, DataType>,
    pub roles: Arena<RoleId, Role>,
    pub passive_resources: Arena<PassiveResourceId, PassiveResource>,
    pub seffs: Arena<SeffId, Seff>,
    pub behaviours: Arena<BehaviourId, Behaviour>,
    pub actions: Arena<ActionId, Action>,
    pub structures: Arena<StructureId, Structure>,
    pub assembly_contexts: Arena<AssemblyContextId, AssemblyContext>,
    pub connectors: Arena<ConnectorId, Connector>,
    pub systems: Arena<SystemId, System>,
    pub resource_environments: Arena<ResourceEnvironmentId, ResourceEnvironment>,
    pub containers: Arena<ContainerId, ResourceContainer>,
    pub processing_resources: Arena<ProcessingResourceId, ProcessingResource>,
    pub linking_resources: Arena<LinkingResourceId, LinkingResource>,
    pub resource_repositories: Arena<ResourceRepositoryId, ResourceRepository>,
    pub resource_types: Arena<ResourceTypeId, ResourceType>,
    pub scheduling_policies: Arena<SchedulingPolicyId, SchedulingPolicy>,
    pub resource_interfaces: Arena<ResourceInterfaceId, ResourceInterface>,
    pub resource_signatures: Arena<ResourceSignatureId, ResourceSignature>,
    pub allocations: Arena<AllocationId, Allocation>,
    pub allocation_contexts: Arena<AllocationContextId, AllocationContext>,
    pub usage_models: Arena<UsageModelId, UsageModel>,
    pub usage_scenarios: Arena<UsageScenarioId, UsageScenario>,
    pub scenario_behaviours: Arena<ScenarioBehaviourId, ScenarioBehaviour>,
    pub user_actions: Arena<UserActionId, UserAction>,
    pub monitor_repositories: Arena<MonitorRepositoryId, MonitorRepository>,
    pub monitors: Arena<MonitorId, Monitor>,
    pub measuring_points: Arena<MeasuringPointId, MeasuringPoint>,
    pub metrics: Arena<MetricId, Metric>,
    /// The generic object graph everything was built from.
    pub graph: Graph,
    /// Load, resolution, build and validation diagnostics.
    pub diagnostics: Diagnostics,
}

impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Model")
            .field("repositories", &self.repositories.len())
            .field("components", &self.components.len())
            .field("seffs", &self.seffs.len())
            .field("actions", &self.actions.len())
            .field("systems", &self.systems.len())
            .field("containers", &self.containers.len())
            .field("allocations", &self.allocations.len())
            .field("usage_scenarios", &self.usage_scenarios.len())
            .field("monitors", &self.monitors.len())
            .field("diagnostics", &self.diagnostics.0.len())
            .finish()
    }
}

impl Model {
    /// `resource#fragment (pkg:Class "name")` of a graph object, for diagnostics.
    pub fn describe(&self, obj: ObjId) -> String {
        self.graph.describe(obj)
    }

    /// The SEFF of `component` describing `signature`, if any.
    pub fn seff_for(&self, component: ComponentId, signature: SignatureId) -> Option<SeffId> {
        self.components[component]
            .seffs()
            .iter()
            .copied()
            .find(|s| self.seffs[*s].signature == Some(signature))
    }

    /// Resource type by name (`CPU`, `HDD`, `DELAY`, `LAN` in `Palladio.resourcetype`).
    pub fn resource_type_by_name(&self, name: &str) -> Option<ResourceTypeId> {
        self.resource_types
            .iter_ids()
            .find(|(_, t)| &*t.name == name)
            .map(|(i, _)| i)
    }

    /// Graph objects whose XMI id is `id` (over all resources).
    pub fn objects_with_id(&self, id: &str) -> Vec<ObjId> {
        (0..self.graph.objs.len() as u32)
            .map(ObjId)
            .filter(|o| self.graph.id(*o) == Some(id))
            .collect()
    }

    /// Actions of a behaviour following the `successor` chain from its start action.
    pub fn action_chain(&self, b: BehaviourId) -> Vec<ActionId> {
        let mut out = Vec::new();
        let mut cur = self.behaviours[b].start;
        while let Some(a) = cur {
            if out.contains(&a) {
                break;
            }
            out.push(a);
            cur = self.actions[a].successor;
        }
        out
    }

    /// User actions of a scenario behaviour following the `successor` chain from its start.
    pub fn user_action_chain(&self, b: ScenarioBehaviourId) -> Vec<UserActionId> {
        let mut out = Vec::new();
        let mut cur = self.scenario_behaviours[b].start;
        while let Some(a) = cur {
            if out.contains(&a) {
                break;
            }
            out.push(a);
            cur = self.user_actions[a].successor;
        }
        out
    }
}

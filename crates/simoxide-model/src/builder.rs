//! Builds the typed [`Model`] from the generic object graph.

use crate::diag::{Diagnostics, Level};
use crate::fxhash::FxHashMap;
use crate::meta::{ClassId, FeatureId, class as c, feat as f};
use crate::model::*;
use crate::raw::{Graph, ObjId, ResId, Value};

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
enum K {
    None,
    Repository,
    Component,
    Interface,
    Signature,
    Parameter,
    DataType,
    Role,
    PassiveResource,
    Seff,
    Behaviour,
    Action,
    AssemblyContext,
    Connector,
    System,
    ResEnv,
    Container,
    ProcRes,
    LinkRes,
    ResRepo,
    ResType,
    SchedPol,
    ResIface,
    ResSig,
    Allocation,
    AllocCtx,
    UsageModel,
    Scenario,
    ScenBeh,
    UserAction,
    MonRepo,
    Monitor,
    MeasuringPoint,
    Metric,
}

/// [`kind_of`] of every class, computed once.
fn kind_of_cached(cl: ClassId) -> K {
    static KINDS: std::sync::OnceLock<Vec<K>> = std::sync::OnceLock::new();
    KINDS.get_or_init(|| {
        (0..crate::meta::CLASSES.len())
            .map(|i| kind_of(ClassId(i as u32)))
            .collect()
    })[cl.0 as usize]
}

fn kind_of(cl: ClassId) -> K {
    let is = |x: ClassId| cl.is_a(x);
    if is(c::repository_Repository) {
        K::Repository
    } else if is(c::RepositoryComponent) {
        K::Component
    } else if is(c::Interface) {
        K::Interface
    } else if is(c::Signature) {
        K::Signature
    } else if is(c::Parameter) {
        K::Parameter
    } else if is(c::DataType) {
        K::DataType
    } else if is(c::Role) {
        K::Role
    } else if is(c::PassiveResource) {
        K::PassiveResource
    } else if is(c::ResourceDemandingSEFF) {
        K::Seff
    } else if is(c::ResourceDemandingBehaviour) {
        K::Behaviour
    } else if is(c::AbstractAction) {
        K::Action
    } else if is(c::AssemblyContext) {
        K::AssemblyContext
    } else if is(c::Connector) {
        K::Connector
    } else if is(c::System) {
        K::System
    } else if is(c::ResourceEnvironment) {
        K::ResEnv
    } else if is(c::ResourceContainer) {
        K::Container
    } else if is(c::ProcessingResourceSpecification) {
        K::ProcRes
    } else if is(c::LinkingResource) {
        K::LinkRes
    } else if is(c::ResourceRepository) {
        K::ResRepo
    } else if is(c::ResourceType) {
        K::ResType
    } else if is(c::SchedulingPolicy) {
        K::SchedPol
    } else if is(c::ResourceInterface) {
        K::ResIface
    } else if is(c::ResourceSignature) {
        K::ResSig
    } else if is(c::Allocation) {
        K::Allocation
    } else if is(c::AllocationContext) {
        K::AllocCtx
    } else if is(c::UsageModel) {
        K::UsageModel
    } else if is(c::UsageScenario) {
        K::Scenario
    } else if is(c::ScenarioBehaviour) {
        K::ScenBeh
    } else if is(c::AbstractUserAction) {
        K::UserAction
    } else if is(c::MonitorRepository) {
        K::MonRepo
    } else if is(c::Monitor) {
        K::Monitor
    } else if is(c::MeasuringPoint) {
        K::MeasuringPoint
    } else if is(c::MetricDescription) {
        K::Metric
    } else {
        K::None
    }
}

struct B<'a> {
    g: &'a Graph,
    kind: Vec<K>,
    idx: Vec<u32>,
    /// ComposedStructure objects (systems, composites, subsystems) -> structure index.
    structure: FxHashMap<ObjId, u32>,
    /// SEFF objects -> their behaviour index.
    seff_behaviour: FxHashMap<ObjId, u32>,
    diags: Diagnostics,
    m: Model,
}

impl<'a> B<'a> {
    fn cls(&self, o: ObjId) -> ClassId {
        self.g[o].class
    }

    /// Resolved (non-proxy) target of a single-valued reference.
    fn r(&self, o: ObjId, feat: FeatureId) -> Option<ObjId> {
        self.g
            .get_ref(o, feat)
            .filter(|t| self.g[*t].proxy.is_none())
    }

    fn typed<T: From<usize>>(
        &mut self,
        t: Option<ObjId>,
        k: K,
        from: ObjId,
        feat: FeatureId,
    ) -> Option<T> {
        let t = t?;
        if self.kind[t.0 as usize] == k {
            return Some(T::from(self.idx[t.0 as usize] as usize));
        }
        let msg = format!(
            "{} refers to {} (expected {k:?})",
            feat.name(),
            self.g.describe(t)
        );
        let loc = self.g.describe(from);
        self.diags.push(Level::Error, "wrong-type", loc, msg);
        None
    }

    fn one<T: From<usize>>(&mut self, o: ObjId, feat: FeatureId, k: K) -> Option<T> {
        let t = self.r(o, feat);
        self.typed(t, k, o, feat)
    }

    fn many<T: From<usize>>(&mut self, o: ObjId, feat: FeatureId, k: K) -> Vec<T> {
        let g = self.g;
        g.get_refs(o, feat)
            .iter()
            .filter(|t| g[**t].proxy.is_none())
            .filter_map(|&t| self.typed(Some(t), k, o, feat))
            .collect()
    }

    fn id(&self, o: ObjId) -> Box<str> {
        self.g.id(o).unwrap_or("").into()
    }

    fn name(&self, o: ObjId) -> Box<str> {
        self.g
            .str_attr(o, f::NamedElement_entityName)
            .unwrap_or_default()
    }

    fn string(&self, o: ObjId, feat: FeatureId) -> Option<Box<str>> {
        self.g.str_attr(o, feat)
    }

    fn int(&self, o: ObjId, feat: FeatureId) -> i64 {
        self.g.attr(o, feat).as_int().unwrap_or(0)
    }

    fn dbl(&self, o: ObjId, feat: FeatureId) -> f64 {
        self.g.attr(o, feat).as_f64().unwrap_or(0.0)
    }

    fn boolean(&self, o: ObjId, feat: FeatureId) -> bool {
        self.g.attr(o, feat).as_bool().unwrap_or(false)
    }

    fn enum_idx(&self, o: ObjId, feat: FeatureId) -> u32 {
        self.g.attr(o, feat).as_enum().unwrap_or(0)
    }

    fn enum_name(&self, o: ObjId, feat: FeatureId) -> &'static str {
        match (self.g.attr(o, feat), feat.data_kind()) {
            (Value::Enum(i), Some(crate::meta::DataKind::Enum(e))) => {
                e.def().literals[i as usize].name
            }
            _ => "",
        }
    }

    /// Contained `PCMRandomVariable` of a containment feature.
    fn rv(&self, o: ObjId, feat: FeatureId) -> RandomVar {
        match self.g.get_ref(o, feat) {
            Some(v) => RandomVar {
                spec: self
                    .g
                    .str_attr(v, f::RandomVariable_specification)
                    .unwrap_or_default(),
                obj: Some(v),
            },
            None => RandomVar::default(),
        }
    }

    fn usage(&self, u: ObjId) -> VariableUsage {
        let mut reference = Vec::new();
        let mut cur = self
            .g
            .get_ref(u, f::VariableUsage_namedReference__VariableUsage);
        while let Some(r) = cur {
            reference.push(
                self.g
                    .str_attr(r, f::AbstractNamedReference_referenceName)
                    .unwrap_or_default(),
            );
            cur = if self.cls(r).is_a(c::NamespaceReference) {
                self.g
                    .get_ref(r, f::NamespaceReference_innerReference_NamespaceReference)
            } else {
                None
            };
        }
        let characterisations = self
            .g
            .get_refs(u, f::VariableUsage_variableCharacterisation_VariableUsage)
            .iter()
            .map(|&vc| VariableCharacterisation {
                kind: CharacterisationType::from_index(
                    self.enum_idx(vc, f::VariableCharacterisation_type),
                ),
                spec: self.rv(
                    vc,
                    f::VariableCharacterisation_specification_VariableCharacterisation,
                ),
                obj: vc,
            })
            .collect();
        VariableUsage {
            reference,
            characterisations,
            obj: u,
        }
    }

    fn usages(&self, o: ObjId, feat: FeatureId) -> Vec<VariableUsage> {
        self.g
            .get_refs(o, feat)
            .iter()
            .map(|&u| self.usage(u))
            .collect()
    }

    fn container_of(&self, o: ObjId) -> Option<ObjId> {
        self.g[o].container.map(|(p, _)| p)
    }

    fn container_typed<T: From<usize>>(&self, o: ObjId, k: K) -> Option<T> {
        let p = self.container_of(o)?;
        (self.kind[p.0 as usize] == k).then(|| T::from(self.idx[p.0 as usize] as usize))
    }

    fn structure_of(&self, o: ObjId) -> Option<StructureId> {
        self.structure.get(&o).map(|i| StructureId(*i))
    }

    fn behaviour_of(&self, o: ObjId) -> Option<BehaviourId> {
        match self.kind[o.0 as usize] {
            K::Behaviour => Some(BehaviourId(self.idx[o.0 as usize])),
            K::Seff => self.seff_behaviour.get(&o).map(|i| BehaviourId(*i)),
            _ => None,
        }
    }

    fn behaviour_ref(&mut self, o: ObjId, feat: FeatureId) -> Option<BehaviourId> {
        let t = self.r(o, feat)?;
        let b = self.behaviour_of(t);
        if b.is_none() {
            let msg = format!(
                "{} refers to {} (expected a behaviour)",
                feat.name(),
                self.g.describe(t)
            );
            let loc = self.g.describe(o);
            self.diags.push(Level::Error, "wrong-type", loc, msg);
        }
        b
    }

    // ---------------------------------------------------------------------------------------

    fn build_obj(&mut self, o: ObjId) {
        let k = self.kind[o.0 as usize];
        let cl = self.cls(o);
        match k {
            K::None => {}
            K::Repository => {
                let x = Repository {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    description: self.string(o, f::repository_Repository_repositoryDescription),
                    components: self.many(
                        o,
                        f::repository_Repository_components__Repository,
                        K::Component,
                    ),
                    interfaces: self.many(
                        o,
                        f::repository_Repository_interfaces__Repository,
                        K::Interface,
                    ),
                    data_types: self.many(
                        o,
                        f::repository_Repository_dataTypes__Repository,
                        K::DataType,
                    ),
                };
                self.m.repositories.push(x);
            }
            K::Component => {
                let kind = if cl.is_a(c::BasicComponent) {
                    ComponentKind::Basic {
                        seffs: self.many(
                            o,
                            f::BasicComponent_serviceEffectSpecifications__BasicComponent,
                            K::Seff,
                        ),
                        passive_resources: self.many(
                            o,
                            f::BasicComponent_passiveResource_BasicComponent,
                            K::PassiveResource,
                        ),
                    }
                } else if cl.is_a(c::CompositeComponent) {
                    ComponentKind::Composite {
                        structure: self.structure_of(o).unwrap(),
                    }
                } else if cl.is_a(c::SubSystem) {
                    ComponentKind::SubSystem {
                        structure: self.structure_of(o).unwrap(),
                    }
                } else if cl.is_a(c::CompleteComponentType) {
                    ComponentKind::CompleteType
                } else {
                    ComponentKind::ProvidesType
                };
                let impl_type = cl.is_a(c::ImplementationComponentType);
                let x = Component {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    repository: self.container_typed(o, K::Repository),
                    kind,
                    component_type: if impl_type && self.enum_idx(o, f::ImplementationComponentType_componentType) == 1 {
                        ComponentType::Infrastructure
                    } else {
                        ComponentType::Business
                    },
                    provided_roles: self.many(o, f::InterfaceProvidingEntity_providedRoles_InterfaceProvidingEntity, K::Role),
                    required_roles: self.many(o, f::InterfaceRequiringEntity_requiredRoles_InterfaceRequiringEntity, K::Role),
                    resource_required_roles: self.many(
                        o,
                        f::ResourceInterfaceRequiringEntity_resourceRequiredRoles__ResourceInterfaceRequiringEntity,
                        K::Role,
                    ),
                    parameter_usages: if impl_type {
                        self.usages(o, f::ImplementationComponentType_componentParameterUsage_ImplementationComponentType)
                    } else {
                        Vec::new()
                    },
                };
                self.m.components.push(x);
            }
            K::Interface => {
                let (kind, sigs) = if cl.is_a(c::OperationInterface) {
                    (
                        InterfaceKind::Operation,
                        self.many(
                            o,
                            f::OperationInterface_signatures__OperationInterface,
                            K::Signature,
                        ),
                    )
                } else if cl.is_a(c::InfrastructureInterface) {
                    (
                        InterfaceKind::Infrastructure,
                        self.many(o, f::InfrastructureInterface_infrastructureSignatures__InfrastructureInterface, K::Signature),
                    )
                } else {
                    (
                        InterfaceKind::EventGroup,
                        self.many(o, f::EventGroup_eventTypes__EventGroup, K::Signature),
                    )
                };
                let x = Interface {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    kind,
                    repository: self.container_typed(o, K::Repository),
                    parents: self.many(o, f::Interface_parentInterfaces__Interface, K::Interface),
                    signatures: sigs,
                };
                self.m.interfaces.push(x);
            }
            K::Signature => {
                let (kind, params, ret) = if cl.is_a(c::OperationSignature) {
                    let ret = self.one(
                        o,
                        f::OperationSignature_returnType__OperationSignature,
                        K::DataType,
                    );
                    (
                        SignatureKind::Operation,
                        self.many(
                            o,
                            f::OperationSignature_parameters__OperationSignature,
                            K::Parameter,
                        ),
                        ret,
                    )
                } else if cl.is_a(c::InfrastructureSignature) {
                    let p = self.many(
                        o,
                        f::InfrastructureSignature_parameters__InfrastructureSignature,
                        K::Parameter,
                    );
                    (SignatureKind::Infrastructure, p, None)
                } else {
                    (
                        SignatureKind::EventType,
                        self.many(o, f::EventType_parameter__EventType, K::Parameter),
                        None,
                    )
                };
                let x = Signature {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    kind,
                    interface: self.container_typed(o, K::Interface),
                    parameters: params,
                    return_type: ret,
                };
                self.m.signatures.push(x);
            }
            K::Parameter => {
                let modifier = match self.enum_idx(o, f::Parameter_modifier__Parameter) {
                    1 => ParameterModifier::In,
                    2 => ParameterModifier::Out,
                    3 => ParameterModifier::InOut,
                    _ => ParameterModifier::None,
                };
                let x = Parameter {
                    name: self
                        .string(o, f::Parameter_parameterName)
                        .unwrap_or_default(),
                    obj: o,
                    data_type: self.one(o, f::Parameter_dataType__Parameter, K::DataType),
                    modifier,
                    signature: self.container_typed(o, K::Signature),
                    resource_signature: self.container_typed(o, K::ResSig),
                };
                self.m.parameters.push(x);
            }
            K::DataType => {
                let kind = if cl.is_a(c::PrimitiveDataType) {
                    DataTypeKind::Primitive(match self.enum_idx(o, f::PrimitiveDataType_type) {
                        0 => PrimitiveType::Int,
                        1 => PrimitiveType::String,
                        2 => PrimitiveType::Bool,
                        3 => PrimitiveType::Double,
                        4 => PrimitiveType::Char,
                        5 => PrimitiveType::Byte,
                        _ => PrimitiveType::Long,
                    })
                } else if cl.is_a(c::CollectionDataType) {
                    DataTypeKind::Collection {
                        inner: self.one(
                            o,
                            f::CollectionDataType_innerType_CollectionDataType,
                            K::DataType,
                        ),
                    }
                } else {
                    let decls = self
                        .g
                        .get_refs(o, f::CompositeDataType_innerDeclaration_CompositeDataType)
                        .to_vec();
                    let inner = decls
                        .into_iter()
                        .map(|d| InnerDeclaration {
                            name: self.name(d),
                            data_type: self.one(
                                d,
                                f::InnerDeclaration_datatype_InnerDeclaration,
                                K::DataType,
                            ),
                            obj: d,
                        })
                        .collect();
                    DataTypeKind::Composite {
                        parents: self.many(
                            o,
                            f::CompositeDataType_parentType_CompositeDataType,
                            K::DataType,
                        ),
                        inner,
                    }
                };
                let name = if cl.is_a(c::Entity) {
                    self.name(o)
                } else {
                    "".into()
                };
                let x = DataType {
                    id: self.id(o),
                    name,
                    obj: o,
                    repository: self.container_typed(o, K::Repository),
                    kind,
                };
                self.m.data_types.push(x);
            }
            K::Role => self.build_role(o, cl),
            K::PassiveResource => {
                let x = PassiveResource {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    component: self.container_typed(o, K::Component),
                    capacity: self.rv(o, f::PassiveResource_capacity_PassiveResource),
                };
                self.m.passive_resources.push(x);
            }
            K::Seff => {
                let x = Seff {
                    id: self.id(o),
                    obj: o,
                    component: self.container_typed(o, K::Component),
                    signature: self.one(
                        o,
                        f::ServiceEffectSpecification_describedService__SEFF,
                        K::Signature,
                    ),
                    seff_type_id: self
                        .string(o, f::ServiceEffectSpecification_seffTypeID)
                        .unwrap_or_default(),
                    behaviour: self.behaviour_of(o).unwrap(),
                    internal_behaviours: self.many(
                        o,
                        f::ResourceDemandingSEFF_resourceDemandingInternalBehaviours,
                        K::Behaviour,
                    ),
                };
                self.m.seffs.push(x);
                self.build_behaviour(o);
            }
            K::Behaviour => self.build_behaviour(o),
            K::Action => self.build_action(o, cl),
            K::AssemblyContext => {
                let x = AssemblyContext {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    parent: self.container_of(o).and_then(|p| self.structure_of(p)),
                    component: self.one(
                        o,
                        f::AssemblyContext_encapsulatedComponent__AssemblyContext,
                        K::Component,
                    ),
                    config_parameters: self
                        .usages(o, f::AssemblyContext_configParameterUsages__AssemblyContext),
                };
                self.m.assembly_contexts.push(x);
            }
            K::Connector => self.build_connector(o, cl),
            K::System => {
                let x = System {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    structure: self.structure_of(o).unwrap(),
                    provided_roles: self.many(o, f::InterfaceProvidingEntity_providedRoles_InterfaceProvidingEntity, K::Role),
                    required_roles: self.many(o, f::InterfaceRequiringEntity_requiredRoles_InterfaceRequiringEntity, K::Role),
                    resource_required_roles: self.many(
                        o,
                        f::ResourceInterfaceRequiringEntity_resourceRequiredRoles__ResourceInterfaceRequiringEntity,
                        K::Role,
                    ),
                };
                self.m.systems.push(x);
            }
            K::ResEnv => {
                let x = ResourceEnvironment {
                    name: self.name(o),
                    obj: o,
                    containers: self.many(
                        o,
                        f::ResourceEnvironment_resourceContainer_ResourceEnvironment,
                        K::Container,
                    ),
                    linking_resources: self.many(
                        o,
                        f::ResourceEnvironment_linkingResources__ResourceEnvironment,
                        K::LinkRes,
                    ),
                };
                self.m.resource_environments.push(x);
            }
            K::Container => {
                let x = ResourceContainer {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    environment: self.container_typed(o, K::ResEnv),
                    parent: self.container_typed(o, K::Container),
                    nested: self.many(
                        o,
                        f::ResourceContainer_nestedResourceContainers__ResourceContainer,
                        K::Container,
                    ),
                    processing_resources: self.many(
                        o,
                        f::ResourceContainer_activeResourceSpecifications_ResourceContainer,
                        K::ProcRes,
                    ),
                };
                self.m.containers.push(x);
            }
            K::ProcRes => {
                let hdd = cl
                    .is_a(c::HDDProcessingResourceSpecification)
                    .then(|| HddRates {
                        read: self.rv(o, f::HDDProcessingResourceSpecification_readProcessingRate),
                        write: self
                            .rv(o, f::HDDProcessingResourceSpecification_writeProcessingRate),
                    });
                let x = ProcessingResource {
                    id: self.id(o),
                    obj: o,
                    container: self.container_typed(o, K::Container),
                    resource_type: self.one(o, f::ProcessingResourceSpecification_activeResourceType_ActiveResourceSpecification, K::ResType),
                    scheduling: self.one(o, f::ProcessingResourceSpecification_schedulingPolicy, K::SchedPol),
                    processing_rate: self.rv(o, f::ProcessingResourceSpecification_processingRate_ProcessingResourceSpecification),
                    replicas: self.int(o, f::ProcessingResourceSpecification_numberOfReplicas),
                    mttf: self.dbl(o, f::ProcessingResourceSpecification_MTTF),
                    mttr: self.dbl(o, f::ProcessingResourceSpecification_MTTR),
                    required_by_container: self.boolean(o, f::ProcessingResourceSpecification_requiredByContainer),
                    hdd,
                };
                self.m.processing_resources.push(x);
            }
            K::LinkRes => {
                let spec = self.g.get_ref(
                    o,
                    f::LinkingResource_communicationLinkResourceSpecifications_LinkingResource,
                );
                let (spec_id, rt, lat, thr, fp) = match spec {
                    Some(s) => (
                        self.id(s),
                        self.one(
                            s,
                            f::CommunicationLinkResourceSpecification_communicationLinkResourceType_CommunicationLinkResourceSpecification,
                            K::ResType,
                        ),
                        self.rv(s, f::CommunicationLinkResourceSpecification_latency_CommunicationLinkResourceSpecification),
                        self.rv(s, f::CommunicationLinkResourceSpecification_throughput_CommunicationLinkResourceSpecification),
                        self.dbl(s, f::CommunicationLinkResourceSpecification_failureProbability),
                    ),
                    None => ("".into(), None, RandomVar::default(), RandomVar::default(), 0.0),
                };
                let x = LinkingResource {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    environment: self.container_typed(o, K::ResEnv),
                    connected: self.many(
                        o,
                        f::LinkingResource_connectedResourceContainers_LinkingResource,
                        K::Container,
                    ),
                    spec_id,
                    resource_type: rt,
                    latency: lat,
                    throughput: thr,
                    failure_probability: fp,
                };
                self.m.linking_resources.push(x);
            }
            K::ResRepo => {
                let x = ResourceRepository {
                    obj: o,
                    resource_types: self.many(
                        o,
                        f::ResourceRepository_availableResourceTypes_ResourceRepository,
                        K::ResType,
                    ),
                    scheduling_policies: self.many(
                        o,
                        f::ResourceRepository_schedulingPolicies__ResourceRepository,
                        K::SchedPol,
                    ),
                    interfaces: self.many(
                        o,
                        f::ResourceRepository_resourceInterfaces__ResourceRepository,
                        K::ResIface,
                    ),
                };
                self.m.resource_repositories.push(x);
            }
            K::ResType => {
                let x = ResourceType {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    kind: if cl.is_a(c::CommunicationLinkResourceType) {
                        ResourceTypeKind::CommunicationLink
                    } else {
                        ResourceTypeKind::Processing
                    },
                    provided_roles: self.many(
                        o,
                        f::ResourceInterfaceProvidingEntity_resourceProvidedRoles__ResourceInterfaceProvidingEntity,
                        K::Role,
                    ),
                };
                self.m.resource_types.push(x);
            }
            K::SchedPol => {
                let x = SchedulingPolicy {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                };
                self.m.scheduling_policies.push(x);
            }
            K::ResIface => {
                let x = ResourceInterface {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    signatures: self.many(
                        o,
                        f::ResourceInterface_resourceSignatures__ResourceInterface,
                        K::ResSig,
                    ),
                };
                self.m.resource_interfaces.push(x);
            }
            K::ResSig => {
                let x = ResourceSignature {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    interface: self.container_typed(o, K::ResIface),
                    service_id: self.int(o, f::ResourceSignature_resourceServiceId),
                    parameters: self.many(
                        o,
                        f::ResourceSignature_parameter__ResourceSignature,
                        K::Parameter,
                    ),
                };
                self.m.resource_signatures.push(x);
            }
            K::Allocation => {
                let x = Allocation {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    system: self.one(o, f::Allocation_system_Allocation, K::System),
                    environment: self.one(
                        o,
                        f::Allocation_targetResourceEnvironment_Allocation,
                        K::ResEnv,
                    ),
                    contexts: self.many(
                        o,
                        f::Allocation_allocationContexts_Allocation,
                        K::AllocCtx,
                    ),
                };
                self.m.allocations.push(x);
            }
            K::AllocCtx => {
                let x = AllocationContext {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    allocation: self.container_typed(o, K::Allocation),
                    container: self.one(
                        o,
                        f::AllocationContext_resourceContainer_AllocationContext,
                        K::Container,
                    ),
                    assembly: self.one(
                        o,
                        f::AllocationContext_assemblyContext_AllocationContext,
                        K::AssemblyContext,
                    ),
                };
                self.m.allocation_contexts.push(x);
            }
            K::UsageModel => {
                let uds = self
                    .g
                    .get_refs(o, f::UsageModel_userData_UsageModel)
                    .to_vec();
                let user_data = uds
                    .into_iter()
                    .map(|u| UserData {
                        obj: u,
                        assembly: self.one(
                            u,
                            f::UserData_assemblyContext_userData,
                            K::AssemblyContext,
                        ),
                        usages: self.usages(u, f::UserData_userDataParameterUsages_UserData),
                    })
                    .collect();
                let x = UsageModel {
                    obj: o,
                    scenarios: self.many(o, f::UsageModel_usageScenario_UsageModel, K::Scenario),
                    user_data,
                };
                self.m.usage_models.push(x);
            }
            K::Scenario => {
                let workload = match self.g.get_ref(o, f::UsageScenario_workload_UsageScenario) {
                    Some(w) if self.cls(w).is_a(c::ClosedWorkload) => Workload::Closed {
                        population: self.int(w, f::ClosedWorkload_population),
                        think_time: self.rv(w, f::ClosedWorkload_thinkTime_ClosedWorkload),
                    },
                    Some(w) if self.cls(w).is_a(c::OpenWorkload) => Workload::Open {
                        inter_arrival_time: self
                            .rv(w, f::OpenWorkload_interArrivalTime_OpenWorkload),
                    },
                    _ => Workload::Missing,
                };
                let x = UsageScenario {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    usage_model: self.container_typed(o, K::UsageModel),
                    workload,
                    behaviour: self.one(
                        o,
                        f::UsageScenario_scenarioBehaviour_UsageScenario,
                        K::ScenBeh,
                    ),
                };
                self.m.usage_scenarios.push(x);
            }
            K::ScenBeh => {
                let owner = match self.g[o].container {
                    Some((p, _)) if self.kind[p.0 as usize] == K::Scenario => {
                        ScenarioBehaviourOwner::Scenario(UsageScenarioId(self.idx[p.0 as usize]))
                    }
                    Some((p, _)) if self.kind[p.0 as usize] == K::UserAction => {
                        ScenarioBehaviourOwner::Loop(UserActionId(self.idx[p.0 as usize]))
                    }
                    Some((t, _)) if self.cls(t).is_a(c::BranchTransition) => {
                        match self.container_of(t) {
                            Some(br) if self.kind[br.0 as usize] == K::UserAction => {
                                let n = self
                                    .g
                                    .get_refs(br, f::Branch_branchTransitions_Branch)
                                    .iter()
                                    .position(|x| *x == t);
                                ScenarioBehaviourOwner::Branch(
                                    UserActionId(self.idx[br.0 as usize]),
                                    n.unwrap_or(0) as u32,
                                )
                            }
                            _ => ScenarioBehaviourOwner::Other,
                        }
                    }
                    _ => ScenarioBehaviourOwner::Other,
                };
                let actions: Vec<UserActionId> = self.many(
                    o,
                    f::ScenarioBehaviour_actions_ScenarioBehaviour,
                    K::UserAction,
                );
                let start = self
                    .g
                    .get_refs(o, f::ScenarioBehaviour_actions_ScenarioBehaviour)
                    .iter()
                    .position(|a| self.cls(*a).is_a(c::Start))
                    .map(|i| actions[i]);
                let x = ScenarioBehaviour {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    owner,
                    actions,
                    start,
                };
                self.m.scenario_behaviours.push(x);
            }
            K::UserAction => {
                let kind = if cl.is_a(c::Start) {
                    UserActionKind::Start
                } else if cl.is_a(c::Stop) {
                    UserActionKind::Stop
                } else if cl.is_a(c::EntryLevelSystemCall) {
                    UserActionKind::EntryLevelSystemCall {
                        role: self.one(
                            o,
                            f::EntryLevelSystemCall_providedRole_EntryLevelSystemCall,
                            K::Role,
                        ),
                        signature: self.one(
                            o,
                            f::EntryLevelSystemCall_operationSignature__EntryLevelSystemCall,
                            K::Signature,
                        ),
                        inputs: self.usages(
                            o,
                            f::EntryLevelSystemCall_inputParameterUsages_EntryLevelSystemCall,
                        ),
                        outputs: self.usages(
                            o,
                            f::EntryLevelSystemCall_outputParameterUsages_EntryLevelSystemCall,
                        ),
                        priority: self.int(o, f::EntryLevelSystemCall_priority),
                    }
                } else if cl.is_a(c::Delay) {
                    UserActionKind::Delay {
                        time: self.rv(o, f::Delay_timeSpecification_Delay),
                    }
                } else if cl.is_a(c::Branch) {
                    let ts = self
                        .g
                        .get_refs(o, f::Branch_branchTransitions_Branch)
                        .to_vec();
                    let transitions = ts
                        .into_iter()
                        .map(|t| UsageBranchTransition {
                            obj: t,
                            probability: self.dbl(t, f::BranchTransition_branchProbability),
                            behaviour: self.one(
                                t,
                                f::BranchTransition_branchedBehaviour_BranchTransition,
                                K::ScenBeh,
                            ),
                        })
                        .collect();
                    UserActionKind::Branch { transitions }
                } else {
                    UserActionKind::Loop {
                        iterations: self.rv(o, f::Loop_loopIteration_Loop),
                        body: self.one(o, f::Loop_bodyBehaviour_Loop, K::ScenBeh),
                    }
                };
                let x = UserAction {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    behaviour: self.container_typed(o, K::ScenBeh),
                    predecessor: self.one(o, f::AbstractUserAction_predecessor, K::UserAction),
                    successor: self.one(o, f::AbstractUserAction_successor, K::UserAction),
                    kind,
                };
                self.m.user_actions.push(x);
            }
            K::MonRepo => {
                let x = MonitorRepository {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    monitors: self.many(o, f::MonitorRepository_monitors, K::Monitor),
                };
                self.m.monitor_repositories.push(x);
            }
            K::Monitor => {
                let specs = self
                    .g
                    .get_refs(o, f::Monitor_measurementSpecifications)
                    .to_vec();
                let specifications = specs
                    .into_iter()
                    .map(|s| self.measurement_spec(s))
                    .collect();
                let x = Monitor {
                    id: self.id(o),
                    name: self.name(o),
                    obj: o,
                    activated: self.boolean(o, f::Monitor_activated),
                    measuring_point: self.one(o, f::Monitor_measuringPoint, K::MeasuringPoint),
                    specifications,
                };
                self.m.monitors.push(x);
            }
            K::MeasuringPoint => self.build_measuring_point(o, cl),
            K::Metric => {
                let kind = if cl.is_a(c::BaseMetricDescription) {
                    MetricKind::Base {
                        numerical: cl.is_a(c::NumericalBaseMetricDescription),
                        capture_type: self.enum_name(o, f::BaseMetricDescription_captureType),
                        data_type: self.enum_name(o, f::BaseMetricDescription_dataType),
                        scale: self.enum_name(o, f::BaseMetricDescription_scale),
                        scope_of_validity: self
                            .enum_name(o, f::BaseMetricDescription_scopeOfValidity),
                        default_unit: if cl.is_a(c::NumericalBaseMetricDescription) {
                            self.string(o, f::NumericalBaseMetricDescription_defaultUnit)
                        } else {
                            None
                        },
                    }
                } else if cl.is_a(c::MetricSetDescription) {
                    MetricKind::Set {
                        subsumed: self.many(o, f::MetricSetDescription_subsumedMetrics, K::Metric),
                    }
                } else {
                    MetricKind::Other(cl.name())
                };
                let x = Metric {
                    id: self.id(o),
                    name: self.string(o, f::Description_name).unwrap_or_default(),
                    obj: o,
                    textual_description: self.string(o, f::Description_textualDescription),
                    kind,
                };
                self.m.metrics.push(x);
            }
        }
    }

    fn build_role(&mut self, o: ObjId, cl: ClassId) {
        let (kind, iface, riface) = if cl.is_a(c::OperationProvidedRole) {
            (
                RoleKind::OperationProvided,
                self.one(
                    o,
                    f::OperationProvidedRole_providedInterface__OperationProvidedRole,
                    K::Interface,
                ),
                None,
            )
        } else if cl.is_a(c::OperationRequiredRole) {
            (
                RoleKind::OperationRequired,
                self.one(
                    o,
                    f::OperationRequiredRole_requiredInterface__OperationRequiredRole,
                    K::Interface,
                ),
                None,
            )
        } else if cl.is_a(c::InfrastructureProvidedRole) {
            (
                RoleKind::InfrastructureProvided,
                self.one(
                    o,
                    f::InfrastructureProvidedRole_providedInterface__InfrastructureProvidedRole,
                    K::Interface,
                ),
                None,
            )
        } else if cl.is_a(c::InfrastructureRequiredRole) {
            (
                RoleKind::InfrastructureRequired,
                self.one(
                    o,
                    f::InfrastructureRequiredRole_requiredInterface__InfrastructureRequiredRole,
                    K::Interface,
                ),
                None,
            )
        } else if cl.is_a(c::SinkRole) {
            (
                RoleKind::SinkProvided,
                self.one(o, f::SinkRole_eventGroup__SinkRole, K::Interface),
                None,
            )
        } else if cl.is_a(c::SourceRole) {
            (
                RoleKind::SourceRequired,
                self.one(o, f::SourceRole_eventGroup__SourceRole, K::Interface),
                None,
            )
        } else if cl.is_a(c::ResourceProvidedRole) {
            (
                RoleKind::ResourceProvided,
                None,
                self.one(
                    o,
                    f::ResourceProvidedRole_providedResourceInterface__ResourceProvidedRole,
                    K::ResIface,
                ),
            )
        } else {
            (
                RoleKind::ResourceRequired,
                None,
                self.one(
                    o,
                    f::ResourceRequiredRole_requiredResourceInterface__ResourceRequiredRole,
                    K::ResIface,
                ),
            )
        };
        let owner = match self.container_of(o) {
            Some(p) => match self.kind[p.0 as usize] {
                K::Component => RoleOwner::Component(ComponentId(self.idx[p.0 as usize])),
                K::System => RoleOwner::System(SystemId(self.idx[p.0 as usize])),
                K::ResType => RoleOwner::ResourceType(ResourceTypeId(self.idx[p.0 as usize])),
                _ => RoleOwner::Unknown,
            },
            None => RoleOwner::Unknown,
        };
        let x = Role {
            id: self.id(o),
            name: self.name(o),
            obj: o,
            kind,
            owner,
            interface: iface,
            resource_interface: riface,
        };
        self.m.roles.push(x);
    }

    fn build_behaviour(&mut self, o: ObjId) {
        let owner = match self.g[o].container {
            None => BehaviourOwner::Other,
            Some((p, pf)) => match self.kind[p.0 as usize] {
                K::Seff if pf == f::ResourceDemandingSEFF_resourceDemandingInternalBehaviours => {
                    BehaviourOwner::Internal(SeffId(self.idx[p.0 as usize]))
                }
                K::Action if pf == f::AbstractLoopAction_bodyBehaviour_Loop => {
                    BehaviourOwner::Loop(ActionId(self.idx[p.0 as usize]))
                }
                K::Action if pf == f::ForkAction_asynchronousForkedBehaviours_ForkAction => {
                    BehaviourOwner::ForkedAsync(ActionId(self.idx[p.0 as usize]))
                }
                _ if self.cls(p).is_a(c::AbstractBranchTransition) => match self.container_of(p) {
                    Some(br) if self.kind[br.0 as usize] == K::Action => {
                        let n = self
                            .g
                            .get_refs(br, f::BranchAction_branches_Branch)
                            .iter()
                            .position(|x| *x == p);
                        BehaviourOwner::BranchTransition(
                            ActionId(self.idx[br.0 as usize]),
                            n.unwrap_or(0) as u32,
                        )
                    }
                    _ => BehaviourOwner::Other,
                },
                _ if self.cls(p).is_a(c::SynchronisationPoint) => match self.container_of(p) {
                    Some(fk) if self.kind[fk.0 as usize] == K::Action => {
                        BehaviourOwner::ForkedSync(ActionId(self.idx[fk.0 as usize]))
                    }
                    _ => BehaviourOwner::Other,
                },
                _ => BehaviourOwner::Other,
            },
        };
        let owner = if self.kind[o.0 as usize] == K::Seff {
            BehaviourOwner::Seff(SeffId(self.idx[o.0 as usize]))
        } else {
            owner
        };
        let steps: Vec<ActionId> =
            self.many(o, f::ResourceDemandingBehaviour_steps_Behaviour, K::Action);
        let start = self
            .g
            .get_refs(o, f::ResourceDemandingBehaviour_steps_Behaviour)
            .iter()
            .position(|a| self.cls(*a).is_a(c::StartAction))
            .map(|i| steps[i]);
        let x = Behaviour {
            id: self.id(o),
            obj: o,
            owner,
            steps,
            start,
        };
        self.m.behaviours.push(x);
    }

    fn build_action(&mut self, o: ObjId, cl: ClassId) {
        let kind = if cl.is_a(c::StartAction) {
            ActionKind::Start
        } else if cl.is_a(c::StopAction) {
            ActionKind::Stop
        } else if cl.is_a(c::InternalAction) {
            ActionKind::Internal
        } else if cl.is_a(c::ExternalCallAction) {
            ActionKind::ExternalCall {
                signature: self.one(
                    o,
                    f::ExternalCallAction_calledService_ExternalService,
                    K::Signature,
                ),
                role: self.one(o, f::ExternalCallAction_role_ExternalService, K::Role),
                inputs: self.usages(o, f::CallAction_inputVariableUsages__CallAction),
                returns: self.usages(o, f::CallReturnAction_returnVariableUsage__CallReturnAction),
                retry_count: self.int(o, f::ExternalCallAction_retryCount),
            }
        } else if cl.is_a(c::BranchAction) {
            let ts = self.g.get_refs(o, f::BranchAction_branches_Branch).to_vec();
            let transitions = ts
                .into_iter()
                .map(|t| {
                    let condition = if self.cls(t).is_a(c::GuardedBranchTransition) {
                        BranchCondition::Guard(self.rv(
                            t,
                            f::GuardedBranchTransition_branchCondition_GuardedBranchTransition,
                        ))
                    } else {
                        BranchCondition::Probability(
                            self.dbl(t, f::ProbabilisticBranchTransition_branchProbability),
                        )
                    };
                    BranchTransition {
                        id: self.id(t),
                        name: self.name(t),
                        obj: t,
                        condition,
                        behaviour: self.behaviour_ref(
                            t,
                            f::AbstractBranchTransition_branchBehaviour_BranchTransition,
                        ),
                    }
                })
                .collect();
            ActionKind::Branch { transitions }
        } else if cl.is_a(c::LoopAction) {
            ActionKind::Loop {
                iterations: self.rv(o, f::LoopAction_iterationCount_LoopAction),
                body: self.behaviour_ref(o, f::AbstractLoopAction_bodyBehaviour_Loop),
            }
        } else if cl.is_a(c::CollectionIteratorAction) {
            ActionKind::CollectionIterator {
                parameter: self.one(
                    o,
                    f::CollectionIteratorAction_parameter_CollectionIteratorAction,
                    K::Parameter,
                ),
                body: self.behaviour_ref(o, f::AbstractLoopAction_bodyBehaviour_Loop),
            }
        } else if cl.is_a(c::ForkAction) {
            let asynchronous = self.many(
                o,
                f::ForkAction_asynchronousForkedBehaviours_ForkAction,
                K::Behaviour,
            );
            let synchronisation = self
                .g
                .get_ref(o, f::ForkAction_synchronisingBehaviours_ForkAction)
                .map(|sp| SynchronisationPoint {
                    obj: sp,
                    synchronous: self.many(
                        sp,
                        f::SynchronisationPoint_synchronousForkedBehaviours_SynchronisationPoint,
                        K::Behaviour,
                    ),
                    outputs: self.usages(
                        sp,
                        f::SynchronisationPoint_outputParameterUsage_SynchronisationPoint,
                    ),
                });
            ActionKind::Fork {
                asynchronous,
                synchronisation,
            }
        } else if cl.is_a(c::AcquireAction) {
            ActionKind::Acquire {
                resource: self.one(
                    o,
                    f::AcquireAction_passiveresource_AcquireAction,
                    K::PassiveResource,
                ),
                timeout: self.boolean(o, f::AcquireAction_timeout),
                timeout_value: self.dbl(o, f::AcquireAction_timeoutValue),
            }
        } else if cl.is_a(c::ReleaseAction) {
            ActionKind::Release {
                resource: self.one(
                    o,
                    f::ReleaseAction_passiveResource_ReleaseAction,
                    K::PassiveResource,
                ),
            }
        } else if cl.is_a(c::SetVariableAction) {
            ActionKind::SetVariable {
                usages: self.usages(
                    o,
                    f::SetVariableAction_localVariableUsages_SetVariableAction,
                ),
            }
        } else if cl.is_a(c::InternalCallAction) {
            ActionKind::InternalCall {
                behaviour: self.behaviour_ref(
                    o,
                    f::InternalCallAction_calledResourceDemandingInternalBehaviour,
                ),
                inputs: self.usages(o, f::CallAction_inputVariableUsages__CallAction),
            }
        } else if cl.is_a(c::EmitEventAction) {
            ActionKind::EmitEvent {
                event_type: self.one(
                    o,
                    f::EmitEventAction_eventType__EmitEventAction,
                    K::Signature,
                ),
                role: self.one(o, f::EmitEventAction_sourceRole__EmitEventAction, K::Role),
                inputs: self.usages(o, f::CallAction_inputVariableUsages__CallAction),
            }
        } else {
            ActionKind::Unsupported(cl.name())
        };
        let (demands, icalls, rcalls) = if cl.is_a(c::AbstractInternalControlFlowAction) {
            let ds = self
                .g
                .get_refs(
                    o,
                    f::AbstractInternalControlFlowAction_resourceDemand_Action,
                )
                .to_vec();
            let demands = ds
                .into_iter()
                .map(|d| ResourceDemand {
                    spec: self.rv(
                        d,
                        f::ParametricResourceDemand_specification_ParametericResourceDemand,
                    ),
                    resource_type: self.one(
                        d,
                        f::ParametricResourceDemand_requiredResource_ParametricResourceDemand,
                        K::ResType,
                    ),
                    obj: d,
                })
                .collect();
            let ics = self
                .g
                .get_refs(
                    o,
                    f::AbstractInternalControlFlowAction_infrastructureCall__Action,
                )
                .to_vec();
            let icalls = ics
                .into_iter()
                .map(|x| InfrastructureCall {
                    signature: self.one(
                        x,
                        f::InfrastructureCall_signature__InfrastructureCall,
                        K::Signature,
                    ),
                    role: self.one(
                        x,
                        f::InfrastructureCall_requiredRole__InfrastructureCall,
                        K::Role,
                    ),
                    number_of_calls: self
                        .rv(x, f::InfrastructureCall_numberOfCalls__InfrastructureCall),
                    inputs: self.usages(x, f::CallAction_inputVariableUsages__CallAction),
                    obj: x,
                })
                .collect();
            let rcs = self
                .g
                .get_refs(o, f::AbstractInternalControlFlowAction_resourceCall__Action)
                .to_vec();
            let rcalls = rcs
                .into_iter()
                .map(|x| ResourceCall {
                    signature: self.one(x, f::ResourceCall_signature__ResourceCall, K::ResSig),
                    role: self.one(
                        x,
                        f::ResourceCall_resourceRequiredRole__ResourceCall,
                        K::Role,
                    ),
                    number_of_calls: self.rv(x, f::ResourceCall_numberOfCalls__ResourceCall),
                    inputs: self.usages(x, f::CallAction_inputVariableUsages__CallAction),
                    obj: x,
                })
                .collect();
            (demands, icalls, rcalls)
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };
        let x = Action {
            id: self.id(o),
            name: self.name(o),
            obj: o,
            behaviour: self.container_of(o).and_then(|p| self.behaviour_of(p)),
            predecessor: self.one(o, f::AbstractAction_predecessor_AbstractAction, K::Action),
            successor: self.one(o, f::AbstractAction_successor_AbstractAction, K::Action),
            kind,
            resource_demands: demands,
            infrastructure_calls: icalls,
            resource_calls: rcalls,
        };
        self.m.actions.push(x);
    }

    fn build_connector(&mut self, o: ObjId, cl: ClassId) {
        type Fs = (FeatureId, FeatureId, FeatureId);
        let deleg = |b: &mut Self, (inner, outer, ac): Fs| {
            (
                b.one(o, inner, K::Role),
                b.one(o, outer, K::Role),
                b.one(o, ac, K::AssemblyContext),
            )
        };
        let kind = if cl.is_a(c::AssemblyConnector) {
            ConnectorKind::Assembly {
                requiring: self.one(
                    o,
                    f::AssemblyConnector_requiringAssemblyContext_AssemblyConnector,
                    K::AssemblyContext,
                ),
                providing: self.one(
                    o,
                    f::AssemblyConnector_providingAssemblyContext_AssemblyConnector,
                    K::AssemblyContext,
                ),
                required_role: self.one(
                    o,
                    f::AssemblyConnector_requiredRole_AssemblyConnector,
                    K::Role,
                ),
                provided_role: self.one(
                    o,
                    f::AssemblyConnector_providedRole_AssemblyConnector,
                    K::Role,
                ),
            }
        } else if cl.is_a(c::ProvidedDelegationConnector) {
            let (inner_role, outer_role, assembly) = deleg(
                self,
                (
                    f::ProvidedDelegationConnector_innerProvidedRole_ProvidedDelegationConnector,
                    f::ProvidedDelegationConnector_outerProvidedRole_ProvidedDelegationConnector,
                    f::ProvidedDelegationConnector_assemblyContext_ProvidedDelegationConnector,
                ),
            );
            ConnectorKind::ProvidedDelegation {
                inner_role,
                outer_role,
                assembly,
            }
        } else if cl.is_a(c::RequiredDelegationConnector) {
            let (inner_role, outer_role, assembly) = deleg(
                self,
                (
                    f::RequiredDelegationConnector_innerRequiredRole_RequiredDelegationConnector,
                    f::RequiredDelegationConnector_outerRequiredRole_RequiredDelegationConnector,
                    f::RequiredDelegationConnector_assemblyContext_RequiredDelegationConnector,
                ),
            );
            ConnectorKind::RequiredDelegation {
                inner_role,
                outer_role,
                assembly,
            }
        } else if cl.is_a(c::AssemblyInfrastructureConnector) {
            ConnectorKind::AssemblyInfrastructure {
                requiring: self.one(
                    o,
                    f::AssemblyInfrastructureConnector_requiringAssemblyContext__AssemblyInfrastructureConnector,
                    K::AssemblyContext,
                ),
                providing: self.one(
                    o,
                    f::AssemblyInfrastructureConnector_providingAssemblyContext__AssemblyInfrastructureConnector,
                    K::AssemblyContext,
                ),
                required_role: self.one(o, f::AssemblyInfrastructureConnector_requiredRole__AssemblyInfrastructureConnector, K::Role),
                provided_role: self.one(o, f::AssemblyInfrastructureConnector_providedRole__AssemblyInfrastructureConnector, K::Role),
            }
        } else if cl.is_a(c::ProvidedInfrastructureDelegationConnector) {
            let (inner_role, outer_role, assembly) = deleg(
                self,
                (
                    f::ProvidedInfrastructureDelegationConnector_innerProvidedRole__ProvidedInfrastructureDelegationConnector,
                    f::ProvidedInfrastructureDelegationConnector_outerProvidedRole__ProvidedInfrastructureDelegationConnector,
                    f::ProvidedInfrastructureDelegationConnector_assemblyContext__ProvidedInfrastructureDelegationConnector,
                ),
            );
            ConnectorKind::ProvidedInfrastructureDelegation {
                inner_role,
                outer_role,
                assembly,
            }
        } else if cl.is_a(c::RequiredInfrastructureDelegationConnector) {
            let (inner_role, outer_role, assembly) = deleg(
                self,
                (
                    f::RequiredInfrastructureDelegationConnector_innerRequiredRole__RequiredInfrastructureDelegationConnector,
                    f::RequiredInfrastructureDelegationConnector_outerRequiredRole__RequiredInfrastructureDelegationConnector,
                    f::RequiredInfrastructureDelegationConnector_assemblyContext__RequiredInfrastructureDelegationConnector,
                ),
            );
            ConnectorKind::RequiredInfrastructureDelegation {
                inner_role,
                outer_role,
                assembly,
            }
        } else if cl.is_a(c::RequiredResourceDelegationConnector) {
            let (inner_role, outer_role, assembly) = deleg(
                self,
                (
                    f::RequiredResourceDelegationConnector_innerRequiredRole__RequiredResourceDelegationConnector,
                    f::RequiredResourceDelegationConnector_outerRequiredRole__RequiredResourceDelegationConnector,
                    f::RequiredResourceDelegationConnector_assemblyContext__RequiredResourceDelegationConnector,
                ),
            );
            ConnectorKind::RequiredResourceDelegation {
                inner_role,
                outer_role,
                assembly,
            }
        } else {
            ConnectorKind::Unsupported(cl.name())
        };
        let x = Connector {
            id: self.id(o),
            name: self.name(o),
            obj: o,
            parent: self.container_of(o).and_then(|p| self.structure_of(p)),
            kind,
        };
        self.m.connectors.push(x);
    }

    fn measurement_spec(&mut self, s: ObjId) -> MeasurementSpecification {
        let processing = match self
            .g
            .get_ref(s, f::MeasurementSpecification_processingType)
        {
            None => ProcessingType::Missing,
            Some(p) => {
                let pc = self.cls(p);
                let stat = |b: &Self| match b
                    .g
                    .get_ref(p, f::Aggregation_statisticalCharacterization)
                    .map(|x| b.cls(x))
                {
                    Some(x) if x.is_a(c::ArithmeticMean) => {
                        StatisticalCharacterization::ArithmeticMean
                    }
                    Some(x) if x.is_a(c::HarmonicMean) => StatisticalCharacterization::HarmonicMean,
                    Some(x) if x.is_a(c::GeometricMean) => {
                        StatisticalCharacterization::GeometricMean
                    }
                    Some(x) if x.is_a(c::Median) => StatisticalCharacterization::Median,
                    _ => StatisticalCharacterization::Missing,
                };
                if pc.is_a(c::FeedThrough) {
                    ProcessingType::FeedThrough
                } else if pc.is_a(c::TimeDrivenAggregation) {
                    ProcessingType::TimeDrivenAggregation {
                        window_length: self.dbl(p, f::TimeDriven_windowLength),
                        window_increment: self.dbl(p, f::TimeDriven_windowIncrement),
                        statistic: stat(self),
                    }
                } else if pc.is_a(c::TimeDriven) {
                    ProcessingType::TimeDriven {
                        window_length: self.dbl(p, f::TimeDriven_windowLength),
                        window_increment: self.dbl(p, f::TimeDriven_windowIncrement),
                    }
                } else if pc.is_a(c::FixedSizeAggregation) {
                    ProcessingType::FixedSizeAggregation {
                        frequency: self.int(p, f::MeasurementDrivenAggregation_frequency),
                        number_of_measurements: self
                            .int(p, f::FixedSizeAggregation_numberOfMeasurements),
                        statistic: stat(self),
                    }
                } else if pc.is_a(c::VariableSizeAggregation) {
                    ProcessingType::VariableSizeAggregation {
                        frequency: self.int(p, f::MeasurementDrivenAggregation_frequency),
                        retrospection_length: self
                            .dbl(p, f::VariableSizeAggregation_retrospectionLength),
                        statistic: stat(self),
                    }
                } else {
                    ProcessingType::Unsupported(pc.name())
                }
            }
        };
        MeasurementSpecification {
            id: self.id(s),
            obj: s,
            metric: self.one(s, f::MeasurementSpecification_metricDescription, K::Metric),
            processing,
            triggers_self_adaptations: self
                .boolean(s, f::MeasurementSpecification_triggersSelfAdaptations),
        }
    }

    fn build_measuring_point(&mut self, o: ObjId, cl: ClassId) {
        let op = |b: &mut Self| {
            (
                b.one(o, f::OperationReference_role, K::Role),
                b.one(o, f::OperationReference_operationSignature, K::Signature),
            )
        };
        let kind = if cl.is_a(c::AssemblyOperationMeasuringPoint) {
            let (role, signature) = op(self);
            MeasuringPointKind::AssemblyOperation {
                assembly: self.one(o, f::AssemblyReference_assembly, K::AssemblyContext),
                role,
                signature,
            }
        } else if cl.is_a(c::AssemblyPassiveResourceMeasuringPoint) {
            MeasuringPointKind::AssemblyPassiveResource {
                assembly: self.one(o, f::AssemblyReference_assembly, K::AssemblyContext),
                passive_resource: self.one(
                    o,
                    f::PassiveResourceReference_passiveResource,
                    K::PassiveResource,
                ),
            }
        } else if cl.is_a(c::ActiveResourceMeasuringPoint) {
            MeasuringPointKind::ActiveResource {
                resource: self.one(o, f::ActiveResourceReference_activeResource, K::ProcRes),
                replica_id: self.int(o, f::ActiveResourceReference_replicaID),
            }
        } else if cl.is_a(c::SystemOperationMeasuringPoint) {
            let (role, signature) = op(self);
            MeasuringPointKind::SystemOperation {
                system: self.one(o, f::SystemReference_system, K::System),
                role,
                signature,
            }
        } else if cl.is_a(c::SubSystemOperationMeasuringPoint) {
            let (role, signature) = op(self);
            MeasuringPointKind::SubSystemOperation {
                subsystem: self.one(o, f::SubSystemReference_subsystem, K::Component),
                role,
                signature,
            }
        } else if cl.is_a(c::LinkingResourceMeasuringPoint) {
            MeasuringPointKind::LinkingResource {
                resource: self.one(o, f::LinkingResourceReference_linkingResource, K::LinkRes),
            }
        } else if cl.is_a(c::UsageScenarioMeasuringPoint) {
            MeasuringPointKind::UsageScenario {
                scenario: self.one(o, f::UsageScenarioReference_usageScenario, K::Scenario),
            }
        } else if cl.is_a(c::EntryLevelSystemCallMeasuringPoint) {
            MeasuringPointKind::EntryLevelSystemCall {
                call: self.one(
                    o,
                    f::EntryLevelSystemCallReference_entryLevelSystemCall,
                    K::UserAction,
                ),
            }
        } else if cl.is_a(c::ExternalCallActionMeasuringPoint) {
            MeasuringPointKind::ExternalCallAction {
                action: self.one(o, f::ExternalCallActionReference_externalCall, K::Action),
            }
        } else if cl.is_a(c::ResourceEnvironmentMeasuringPoint) {
            MeasuringPointKind::ResourceEnvironment {
                environment: self.one(
                    o,
                    f::ResourceEnvironmentReference_resourceEnvironment,
                    K::ResEnv,
                ),
            }
        } else if cl.is_a(c::ResourceContainerMeasuringPoint) {
            MeasuringPointKind::ResourceContainer {
                container: self.one(
                    o,
                    f::ResourceContainerReference_resourceContainer,
                    K::Container,
                ),
            }
        } else {
            MeasuringPointKind::Other {
                class: cl.name(),
                measuring_point: if cl.is_a(c::StringMeasuringPoint) {
                    self.string(o, f::StringMeasuringPoint_measuringPoint)
                } else {
                    None
                },
            }
        };
        let x = MeasuringPoint {
            obj: o,
            kind,
            stored_string: self.string(o, f::MeasuringPoint_stringRepresentation),
        };
        self.m.measuring_points.push(x);
    }
}

/// Builds the typed model from a resolved graph (resources in load order, objects in tree order).
pub fn build(g: Graph, diags: Diagnostics) -> Model {
    let n = g.objs.len();
    let mut order = Vec::with_capacity(n);
    let mut stack = Vec::new();
    for r in 0..g.resources.len() {
        g.all_contents_into(ResId(r as u32), &mut order, &mut stack);
    }
    let mut kind = vec![K::None; n];
    let mut idx = vec![u32::MAX; n];
    let mut counts = [0u32; K::Metric as usize + 1];
    let mut structure = FxHashMap::default();
    let mut seff_behaviour = FxHashMap::default();
    let mut n_struct = 0u32;
    let next = |k: K, counts: &mut [u32]| {
        let c = &mut counts[k as usize];
        *c += 1;
        *c - 1
    };
    for &o in &order {
        let cl = g[o].class;
        let k = kind_of_cached(cl);
        kind[o.0 as usize] = k;
        if k == K::None {
            continue;
        }
        idx[o.0 as usize] = next(k, &mut counts);
        if k == K::Seff {
            seff_behaviour.insert(o, next(K::Behaviour, &mut counts));
        }
        if cl.is_a(c::ComposedStructure) {
            structure.insert(o, n_struct);
            n_struct += 1;
        }
    }
    let mut b = B {
        g: &g,
        kind,
        idx,
        structure,
        seff_behaviour,
        diags,
        m: Model::default(),
    };
    for &o in &order {
        b.build_obj(o);
        if let Some(&s) = b.structure.get(&o) {
            debug_assert_eq!(b.m.structures.len() as u32, s);
            let owner = match b.kind[o.0 as usize] {
                K::System => StructureOwner::System(SystemId(b.idx[o.0 as usize])),
                _ => StructureOwner::Component(ComponentId(b.idx[o.0 as usize])),
            };
            let x = Structure {
                obj: o,
                owner,
                assembly_contexts: b.many(
                    o,
                    f::ComposedStructure_assemblyContexts__ComposedStructure,
                    K::AssemblyContext,
                ),
                connectors: b.many(
                    o,
                    f::ComposedStructure_connectors__ComposedStructure,
                    K::Connector,
                ),
            };
            b.m.structures.push(x);
        }
    }
    let mut m = b.m;
    m.diagnostics = b.diags;
    m.graph = g;
    m
}

//! The typed view of a scaling policy definition ([`SpdModel`]), built once from the model graph.
//!
//! It holds what the interpreter reads and never changes during a run: policies, triggers,
//! constraints and target groups. The semantic configuration (the current elements of each
//! target group), which the adjustment changes, is read separately into a [`Membership`]
//! (part of the run state).
//!
//! Metamodel: `SPD.ecore` and `semantic.ecore` of Palladio-Addons-SPD-Metamodel
//! (`DESIGN-SPD.md` §1.1), in the generated tables of `simoxide-model`.

use simoxide_model::meta::{ClassId, FeatureId, class, feat};
use simoxide_model::raw::{Graph, ObjId, Value};

/// A configuration error of the SPD model (fixed behaviour of Q11; with the compat flag,
/// Slingshot drops the whole SPD instead, see [`crate::Quirks::q11_setup_error_drops_spd`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpdError(pub String);

impl std::fmt::Display for SpdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SPD: {}", self.0)
    }
}

impl std::error::Error for SpdError {}

/// `RelationalOperator` (literal order of the metamodel).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelOp {
    LessThan,
    GreaterThan,
    EqualTo,
    LessThanOrEqualTo,
    GreaterThanOrEqualTo,
}

impl RelOp {
    /// The EMF literal (what `toString()` of the enum gives in Slingshot's messages).
    pub fn literal(self) -> &'static str {
        match self {
            RelOp::LessThan => "LessThan",
            RelOp::GreaterThan => "GreaterThan",
            RelOp::EqualTo => "EqualTo",
            RelOp::LessThanOrEqualTo => "LessThanOrEqualTo",
            RelOp::GreaterThanOrEqualTo => "GreaterThanOrEqualTo",
        }
    }

    /// `SimpleFireOnValueComparator`: plain `double` comparison.
    pub fn holds(self, actual: f64, expected: f64) -> bool {
        match self {
            RelOp::EqualTo => actual == expected,
            RelOp::GreaterThanOrEqualTo => actual >= expected,
            RelOp::GreaterThan => actual > expected,
            RelOp::LessThanOrEqualTo => actual <= expected,
            RelOp::LessThan => actual < expected,
        }
    }

    fn from_index(i: u32) -> RelOp {
        match i {
            0 => RelOp::LessThan,
            1 => RelOp::GreaterThan,
            2 => RelOp::EqualTo,
            3 => RelOp::LessThanOrEqualTo,
            _ => RelOp::GreaterThanOrEqualTo,
        }
    }
}

/// `LogicalOperator` of a composed trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogicalOp {
    And,
    Or,
    Xor,
}

/// `AGGREGATIONMETHOD` of a managed-element stimulus (`aggregationOverElements`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Aggregation {
    Average,
    Max,
    Min,
    Median,
    Sum,
}

impl Aggregation {
    fn from_index(i: u32) -> Aggregation {
        match i {
            0 => Aggregation::Average,
            1 => Aggregation::Max,
            2 => Aggregation::Min,
            3 => Aggregation::Median,
            _ => Aggregation::Sum,
        }
    }
}

/// The stimulus of a base trigger.
#[derive(Clone, Debug, PartialEq)]
pub enum Stimulus {
    SimulationTime,
    /// Response times of an operation (signature, if set; its name for messages).
    OperationResponseTime {
        signature: Option<ObjId>,
        signature_name: Box<str>,
    },
    CpuUtilization {
        aggregation: Aggregation,
    },
    TaskCount {
        aggregation: Aggregation,
    },
    QueueLength {
        passive_resource: Option<ObjId>,
    },
    /// A stimulus Slingshot has no checker for (`NumberOfElements`, memory, HDD, network): the
    /// metamodel class name.
    Unsupported(&'static str),
    /// No stimulus set.
    Missing,
}

impl Stimulus {
    /// The stimulus is driven by measurements (`MeasurementMade`), not by time events.
    pub fn is_measurement(&self) -> bool {
        matches!(
            self,
            Stimulus::OperationResponseTime { .. }
                | Stimulus::CpuUtilization { .. }
                | Stimulus::TaskCount { .. }
                | Stimulus::QueueLength { .. }
        )
    }
}

/// The expected value of a base trigger.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Expected {
    /// `NoExpectation`: always in accordance.
    Nothing,
    /// `ExpectedPercentage` (0-100; compared as `value / 100`).
    Percentage(f64),
    Count(i64),
    Time(f64),
    Trend,
    Missing,
}

impl Expected {
    /// The metamodel class name (for messages).
    pub fn class_name(&self) -> &'static str {
        match self {
            Expected::Nothing => "NoExpectation",
            Expected::Percentage(_) => "ExpectedPercentage",
            Expected::Count(_) => "ExpectedCount",
            Expected::Time(_) => "ExpectedTime",
            Expected::Trend => "ExpectedTrend",
            Expected::Missing => "null",
        }
    }
}

/// A scaling trigger.
#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub obj: ObjId,
    pub id: Box<str>,
    pub kind: TriggerKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TriggerKind {
    /// `SimpleFireOnValue`.
    Value {
        op: RelOp,
        stimulus: Stimulus,
        expected: Expected,
    },
    /// `SimpleFireOnTrend` (Slingshot has no checker for it, Q11).
    Trend,
    /// `ComposedTrigger` with its children in model order.
    Composed {
        op: LogicalOp,
        children: Vec<Trigger>,
    },
}

/// The adjustment of a policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Adjustment {
    Step { step: i64 },
    Relative { percentage: i64, min: i64 },
    Absolute { goal: i64 },
    Missing,
}

impl Adjustment {
    /// The metamodel class name (`adjustment` of the decision log's `policy` record).
    pub fn class_name(&self) -> Option<&'static str> {
        match self {
            Adjustment::Step { .. } => Some("StepAdjustment"),
            Adjustment::Relative { .. } => Some("RelativeAdjustment"),
            Adjustment::Absolute { .. } => Some("AbsoluteAdjustment"),
            Adjustment::Missing => None,
        }
    }
}

/// A policy constraint (Slingshot supports only these two; any other throws at set-up).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PolicyConstraint {
    Cooldown {
        time: f64,
        max_operations: i64,
    },
    Interval {
        offset: f64,
        duration: f64,
        repeat: bool,
    },
    Unsupported(&'static str),
}

/// A target constraint of a target group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TargetConstraint {
    Thrashing { minimum_time: f64 },
    Size { min: i64, max: i64 },
}

/// What a target group scales.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupKind {
    /// Resource containers (copies of `unit`).
    ElasticInfrastructure {
        unit: Option<ObjId>,
    },
    /// Assembly contexts (copies of `unit`).
    ServiceGroup {
        unit: Option<ObjId>,
    },
    CompetingConsumers {
        unit: Option<ObjId>,
    },
}

impl GroupKind {
    pub fn unit(&self) -> Option<ObjId> {
        match *self {
            GroupKind::ElasticInfrastructure { unit }
            | GroupKind::ServiceGroup { unit }
            | GroupKind::CompetingConsumers { unit } => unit,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TargetGroup {
    pub obj: ObjId,
    pub id: Box<str>,
    pub name: Box<str>,
    pub kind: GroupKind,
    pub constraints: Vec<TargetConstraint>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Policy {
    pub obj: ObjId,
    pub id: Box<str>,
    pub name: Box<str>,
    pub active: bool,
    /// Index into [`SpdModel::groups`] (`None`: unset or not a group of this SPD).
    pub group: Option<usize>,
    pub adjustment: Adjustment,
    pub trigger: Option<Trigger>,
    pub constraints: Vec<PolicyConstraint>,
}

/// The scaling policy definition of a run (the first `SPD` object of the model graph, as
/// Slingshot's model provider takes one), and its semantic configuration object.
#[derive(Clone, Debug, PartialEq)]
pub struct SpdModel {
    pub obj: ObjId,
    pub id: Box<str>,
    pub policies: Vec<Policy>,
    pub groups: Vec<TargetGroup>,
    /// The `semanticspd:Configuration` object (the first one of the graph), if loaded.
    pub configuration: Option<ObjId>,
}

fn id_of(g: &Graph, o: ObjId) -> Box<str> {
    g.id(o).unwrap_or_default().into()
}

fn name_of(g: &Graph, o: ObjId) -> Box<str> {
    g.str_attr(o, feat::NamedElement_entityName)
        .unwrap_or_else(|| "".into())
}

fn int(g: &Graph, o: ObjId, f: FeatureId) -> i64 {
    match g.attr(o, f) {
        Value::Int(i) => i,
        Value::Double(d) => d as i64,
        _ => 0,
    }
}

fn double(g: &Graph, o: ObjId, f: FeatureId) -> f64 {
    g.attr(o, f).as_f64().unwrap_or(0.0)
}

fn is(g: &Graph, o: ObjId, c: ClassId) -> bool {
    g[o].class.is_a(c)
}

impl SpdModel {
    /// The SPD of `g`: `None` when the graph has no `SPD` object.
    pub fn from_graph(g: &Graph) -> Option<SpdModel> {
        let spd = (0..g.objs.len() as u32)
            .map(ObjId)
            .find(|&o| g[o].proxy.is_none() && g[o].class == class::SPD)?;
        let configuration = (0..g.objs.len() as u32)
            .map(ObjId)
            .find(|&o| g[o].proxy.is_none() && g[o].class == class::Configuration);
        let groups: Vec<TargetGroup> = g
            .get_refs(spd, feat::SPD_targetGroups)
            .iter()
            .map(|&t| target_group(g, t))
            .collect();
        let policies = g
            .get_refs(spd, feat::SPD_scalingPolicies)
            .iter()
            .map(|&p| policy(g, p, &groups))
            .collect();
        Some(SpdModel {
            obj: spd,
            id: id_of(g, spd),
            policies,
            groups,
            configuration,
        })
    }
}

fn target_group(g: &Graph, t: ObjId) -> TargetGroup {
    let kind = if is(g, t, class::ElasticInfrastructure) {
        GroupKind::ElasticInfrastructure {
            unit: g.get_ref(t, feat::ElasticInfrastructure_unit),
        }
    } else if is(g, t, class::ServiceGroup) {
        GroupKind::ServiceGroup {
            unit: g.get_ref(t, feat::ServiceGroup_unitAssembly),
        }
    } else {
        GroupKind::CompetingConsumers {
            unit: g.get_ref(t, feat::CompetingConsumersGroup_unitAssembly),
        }
    };
    let constraints = g
        .get_refs(t, feat::TargetGroup_targetConstraints)
        .iter()
        .filter_map(|&c| {
            if is(g, c, class::ThrashingConstraint) {
                Some(TargetConstraint::Thrashing {
                    minimum_time: double(g, c, feat::ThrashingConstraint_minimumTimeNoThrashing),
                })
            } else if is(g, c, class::TargetGroupSizeConstraint) {
                Some(TargetConstraint::Size {
                    min: int(g, c, feat::TargetGroupSizeConstraint_minSize),
                    max: int(g, c, feat::TargetGroupSizeConstraint_maxSize),
                })
            } else {
                None
            }
        })
        .collect();
    TargetGroup {
        obj: t,
        id: id_of(g, t),
        name: name_of(g, t),
        kind,
        constraints,
    }
}

fn policy(g: &Graph, p: ObjId, groups: &[TargetGroup]) -> Policy {
    let adjustment = match g.get_ref(p, feat::ScalingPolicy_adjustmentType) {
        Some(a) if is(g, a, class::StepAdjustment) => Adjustment::Step {
            step: int(g, a, feat::StepAdjustment_stepValue),
        },
        Some(a) if is(g, a, class::RelativeAdjustment) => Adjustment::Relative {
            percentage: int(g, a, feat::RelativeAdjustment_percentageGrowthValue),
            min: int(g, a, feat::RelativeAdjustment_minAdjustmentValue),
        },
        Some(a) if is(g, a, class::AbsoluteAdjustment) => Adjustment::Absolute {
            goal: int(g, a, feat::AbsoluteAdjustment_goalValue),
        },
        _ => Adjustment::Missing,
    };
    let group = g
        .get_ref(p, feat::ScalingPolicy_targetGroup)
        .and_then(|t| groups.iter().position(|x| x.obj == t));
    let constraints = g
        .get_refs(p, feat::ScalingPolicy_policyConstraints)
        .iter()
        .map(|&c| {
            if is(g, c, class::CooldownConstraint) {
                PolicyConstraint::Cooldown {
                    time: double(g, c, feat::CooldownConstraint_cooldownTime),
                    max_operations: int(g, c, feat::CooldownConstraint_maxScalingOperations),
                }
            } else if is(g, c, class::IntervalConstraint) {
                PolicyConstraint::Interval {
                    offset: double(g, c, feat::IntervalConstraint_offset),
                    duration: double(g, c, feat::IntervalConstraint_intervalDuration),
                    repeat: g
                        .attr(c, feat::IntervalConstraint_repeat)
                        .as_bool()
                        .unwrap_or(false),
                }
            } else {
                PolicyConstraint::Unsupported(g[c].class.name())
            }
        })
        .collect();
    Policy {
        obj: p,
        id: id_of(g, p),
        name: name_of(g, p),
        active: g
            .attr(p, feat::ScalingPolicy_active)
            .as_bool()
            .unwrap_or(false),
        group,
        adjustment,
        trigger: g
            .get_ref(p, feat::ScalingPolicy_scalingTrigger)
            .map(|t| trigger(g, t)),
        constraints,
    }
}

fn trigger(g: &Graph, t: ObjId) -> Trigger {
    let kind = if is(g, t, class::ComposedTrigger) {
        let op = match g
            .attr(t, feat::ComposedTrigger_logicalOperator)
            .as_enum()
            .unwrap_or(0)
        {
            0 => LogicalOp::And,
            1 => LogicalOp::Or,
            _ => LogicalOp::Xor,
        };
        TriggerKind::Composed {
            op,
            children: g
                .get_refs(t, feat::ComposedTrigger_scalingtrigger)
                .iter()
                .map(|&c| trigger(g, c))
                .collect(),
        }
    } else if is(g, t, class::SimpleFireOnValue) {
        TriggerKind::Value {
            op: RelOp::from_index(
                g.attr(t, feat::SimpleFireOnValue_relationalOperator)
                    .as_enum()
                    .unwrap_or(0),
            ),
            stimulus: stimulus(g, g.get_ref(t, feat::BaseTrigger_stimulus)),
            expected: expected(g, g.get_ref(t, feat::BaseTrigger_expectedValue)),
        }
    } else {
        TriggerKind::Trend
    };
    Trigger {
        obj: t,
        id: id_of(g, t),
        kind,
    }
}

fn aggregation(g: &Graph, s: ObjId) -> Aggregation {
    Aggregation::from_index(
        g.attr(
            s,
            feat::ManagedElementsStateStimulus_aggregationOverElements,
        )
        .as_enum()
        .unwrap_or(0),
    )
}

fn stimulus(g: &Graph, s: Option<ObjId>) -> Stimulus {
    let Some(s) = s else {
        return Stimulus::Missing;
    };
    let c = g[s].class;
    if c.is_a(class::SimulationTime) {
        Stimulus::SimulationTime
    } else if c.is_a(class::OperationResponseTime) {
        let signature = g.get_ref(s, feat::OperationResponseTime_operationSignature);
        Stimulus::OperationResponseTime {
            signature,
            signature_name: signature.map(|x| name_of(g, x)).unwrap_or_default(),
        }
    } else if c.is_a(class::CPUUtilization) {
        Stimulus::CpuUtilization {
            aggregation: aggregation(g, s),
        }
    } else if c.is_a(class::TaskCount) {
        Stimulus::TaskCount {
            aggregation: aggregation(g, s),
        }
    } else if c.is_a(class::QueueLength) {
        Stimulus::QueueLength {
            passive_resource: g.get_ref(s, feat::QueueLength_passiveResource),
        }
    } else {
        Stimulus::Unsupported(c.name())
    }
}

fn expected(g: &Graph, e: Option<ObjId>) -> Expected {
    let Some(e) = e else {
        return Expected::Missing;
    };
    let c = g[e].class;
    if c.is_a(class::NoExpectation) {
        Expected::Nothing
    } else if c.is_a(class::ExpectedPercentage) {
        Expected::Percentage(double(g, e, feat::ExpectedPercentage_value))
    } else if c.is_a(class::ExpectedCount) {
        Expected::Count(int(g, e, feat::ExpectedCount_count))
    } else if c.is_a(class::ExpectedTime) {
        Expected::Time(double(g, e, feat::ExpectedTime_value))
    } else {
        Expected::Trend
    }
}

// ---------------------------------------------------------------------------------------------
// membership: the semantic configuration and the allocation, as plain data

/// The current elements of one target-group configuration (`TargetGroupCfg`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupCfg {
    /// `ElasticInfrastructureCfg`: the containers.
    ElasticInfrastructure {
        unit: Option<ObjId>,
        elements: Vec<ObjId>,
    },
    /// `ServiceGroupCfg`: the assembly contexts.
    ServiceGroup {
        unit: Option<ObjId>,
        elements: Vec<ObjId>,
    },
    /// `CompetingConsumersGroupCfg`: the assembly contexts and the broker assembly.
    CompetingConsumers {
        unit: Option<ObjId>,
        broker: Option<ObjId>,
        elements: Vec<ObjId>,
    },
}

/// What target-group checks read from the models that adjustments change: the semantic
/// configuration's target-group configurations, the allocation, the resource environment and
/// the provided signatures of allocated assemblies. Plain data; the host refreshes it after an
/// adjustment ([`Membership::from_graph`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Membership {
    pub cfgs: Vec<GroupCfg>,
    /// Allocation contexts: (assembly context, container), in allocation order.
    pub allocation: Vec<(ObjId, ObjId)>,
    /// Containers of the resource environment.
    pub environment: Vec<ObjId>,
    /// Per allocated assembly context: the signatures of its component's operation provided
    /// roles.
    pub provided: Vec<(ObjId, Vec<ObjId>)>,
}

impl Membership {
    /// Reads the membership from the graph: the configuration object of `spd` (if any), the
    /// configuration's allocation (or the graph's first allocation) and its environment.
    pub fn from_graph(g: &Graph, spd: &SpdModel) -> Membership {
        let mut m = Membership::default();
        let cfg = spd.configuration;
        if let Some(c) = cfg {
            for &t in g.get_refs(c, feat::Configuration_targetCfgs) {
                let cl = g[t].class;
                let refs = |f| g.get_refs(t, f).to_vec();
                m.cfgs.push(if cl.is_a(class::ElasticInfrastructureCfg) {
                    GroupCfg::ElasticInfrastructure {
                        unit: g.get_ref(t, feat::ElasticInfrastructureCfg_unit),
                        elements: refs(feat::ElasticInfrastructureCfg_elements),
                    }
                } else if cl.is_a(class::ServiceGroupCfg) {
                    GroupCfg::ServiceGroup {
                        unit: g.get_ref(t, feat::ServiceGroupCfg_unit),
                        elements: refs(feat::ServiceGroupCfg_elements),
                    }
                } else {
                    GroupCfg::CompetingConsumers {
                        unit: g.get_ref(t, feat::CompetingConsumersGroupCfg_unit),
                        broker: g.get_ref(t, feat::CompetingConsumersGroupCfg_brokerAssembly),
                        elements: refs(feat::CompetingConsumersGroupCfg_elements),
                    }
                });
            }
        }
        let alloc = cfg
            .and_then(|c| g.get_ref(c, feat::Configuration_allocation))
            .filter(|&a| g[a].proxy.is_none())
            .or_else(|| {
                (0..g.objs.len() as u32)
                    .map(ObjId)
                    .find(|&o| g[o].proxy.is_none() && g[o].class == class::Allocation)
            });
        if let Some(a) = alloc {
            for &ac in g.get_refs(a, feat::Allocation_allocationContexts_Allocation) {
                let asm = g.get_ref(
                    ac,
                    feat::AllocationContext_assemblyContext_AllocationContext,
                );
                let rc = g.get_ref(
                    ac,
                    feat::AllocationContext_resourceContainer_AllocationContext,
                );
                if let (Some(asm), Some(rc)) = (asm, rc) {
                    m.allocation.push((asm, rc));
                }
            }
            if let Some(env) = g.get_ref(a, feat::Allocation_targetResourceEnvironment_Allocation) {
                m.environment = g
                    .get_refs(
                        env,
                        feat::ResourceEnvironment_resourceContainer_ResourceEnvironment,
                    )
                    .to_vec();
            }
        }
        let mut seen = Vec::new();
        for &(asm, _) in &m.allocation {
            if seen.contains(&asm) {
                continue;
            }
            seen.push(asm);
            let mut sigs = Vec::new();
            if let Some(comp) = g.get_ref(
                asm,
                feat::AssemblyContext_encapsulatedComponent__AssemblyContext,
            ) {
                for &role in g.get_refs(
                    comp,
                    feat::InterfaceProvidingEntity_providedRoles_InterfaceProvidingEntity,
                ) {
                    if !g[role].class.is_a(class::OperationProvidedRole) {
                        continue;
                    }
                    if let Some(itf) = g.get_ref(
                        role,
                        feat::OperationProvidedRole_providedInterface__OperationProvidedRole,
                    ) {
                        sigs.extend_from_slice(g.get_refs(
                            itf,
                            feat::OperationInterface_signatures__OperationInterface,
                        ));
                    }
                }
            }
            m.provided.push((asm, sigs));
        }
        m
    }

    /// The current number of elements of `group` (its configuration's `elements`), `None` if
    /// the configuration has no entry for the group's unit.
    pub fn group_size(&self, group: &TargetGroup) -> Option<usize> {
        self.cfg_of(group).map(|c| match c {
            GroupCfg::ElasticInfrastructure { elements, .. }
            | GroupCfg::ServiceGroup { elements, .. }
            | GroupCfg::CompetingConsumers { elements, .. } => elements.len(),
        })
    }

    /// The configuration of `group`: the first one of the matching kind with the group's unit.
    pub fn cfg_of(&self, group: &TargetGroup) -> Option<&GroupCfg> {
        self.cfg_index(group).map(|i| &self.cfgs[i])
    }

    fn cfg_index(&self, group: &TargetGroup) -> Option<usize> {
        let unit = group.kind.unit();
        self.cfgs.iter().position(|c| match (c, group.kind) {
            (
                GroupCfg::ElasticInfrastructure { unit: u, .. },
                GroupKind::ElasticInfrastructure { .. },
            )
            | (GroupCfg::ServiceGroup { unit: u, .. }, GroupKind::ServiceGroup { .. })
            | (
                GroupCfg::CompetingConsumers { unit: u, .. },
                GroupKind::CompetingConsumers { .. },
            ) => *u == unit && unit.is_some(),
            _ => false,
        })
    }

    /// Sets the element list of `group`'s configuration (a host or test double feeding the
    /// outcome of an adjustment). Returns `false` if the group has no configuration.
    pub fn set_elements(&mut self, group: &TargetGroup, new: Vec<ObjId>) -> bool {
        let Some(i) = self.cfg_index(group) else {
            return false;
        };
        match &mut self.cfgs[i] {
            GroupCfg::ElasticInfrastructure { elements, .. }
            | GroupCfg::ServiceGroup { elements, .. }
            | GroupCfg::CompetingConsumers { elements, .. } => *elements = new,
        }
        true
    }

    fn assemblies_on(&self, container: ObjId) -> impl Iterator<Item = ObjId> + '_ {
        self.allocation
            .iter()
            .filter(move |(_, c)| *c == container)
            .map(|(a, _)| *a)
    }

    fn provides(&self, asm: ObjId, sig: ObjId) -> bool {
        self.provided
            .iter()
            .any(|(a, s)| *a == asm && s.contains(&sig))
    }
}

/// Is `container` in `group` (`TargetGroupUtils.isContainerInTargetGroup`)? `Err`: Slingshot
/// throws (an elastic infrastructure without a configuration for its unit:
/// `elasticInfraCfgs.get(0)` of an empty list).
pub(crate) fn container_in_group(
    m: &Membership,
    group: &TargetGroup,
    container: ObjId,
) -> Result<bool, &'static str> {
    match group.kind {
        GroupKind::ElasticInfrastructure { .. } => match m.cfg_of(group) {
            Some(GroupCfg::ElasticInfrastructure { elements, .. }) => {
                Ok(elements.contains(&container))
            }
            _ => Err("java.lang.IndexOutOfBoundsException: Index 0 out of bounds for length 0"),
        },
        GroupKind::ServiceGroup { .. } | GroupKind::CompetingConsumers { .. } => {
            let contexts: Vec<ObjId> = group_contexts(m, group);
            Ok(m.assemblies_on(container).any(|a| contexts.contains(&a)))
        }
    }
}

/// The assembly contexts of a service group or competing-consumers group (with the broker),
/// over all matching configurations.
fn group_contexts(m: &Membership, group: &TargetGroup) -> Vec<ObjId> {
    let unit = group.kind.unit();
    let mut v = Vec::new();
    for c in &m.cfgs {
        match (c, group.kind) {
            (GroupCfg::ServiceGroup { unit: u, elements }, GroupKind::ServiceGroup { .. })
                if *u == unit && unit.is_some() =>
            {
                v.extend_from_slice(elements)
            }
            (
                GroupCfg::CompetingConsumers {
                    unit: u,
                    broker,
                    elements,
                },
                GroupKind::CompetingConsumers { .. },
            ) if *u == unit && unit.is_some() => {
                v.extend(broker.iter().copied());
                v.extend_from_slice(elements)
            }
            _ => {}
        }
    }
    v
}

/// `TargetGroupUtils.isAssemblyInTargetGroup`. For an elastic infrastructure Slingshot checks
/// whether the assembly is allocated to *any* container of the environment.
pub(crate) fn assembly_in_group(m: &Membership, group: &TargetGroup, asm: ObjId) -> bool {
    match group.kind {
        GroupKind::ElasticInfrastructure { .. } => m
            .allocation
            .iter()
            .any(|(a, c)| *a == asm && m.environment.contains(c)),
        _ => group_contexts(m, group).contains(&asm),
    }
}

/// `TargetGroupUtils.isOperationSinatureRelatedToTargetGroup`. For an elastic infrastructure:
/// any assembly allocated in the environment provides the signature.
pub(crate) fn signature_in_group(m: &Membership, group: &TargetGroup, sig: ObjId) -> bool {
    match group.kind {
        GroupKind::ElasticInfrastructure { .. } => m
            .environment
            .iter()
            .flat_map(|&c| m.assemblies_on(c))
            .any(|a| m.provides(a, sig)),
        _ => group_contexts(m, group)
            .into_iter()
            .any(|a| m.provides(a, sig)),
    }
}

/// The measuring point of a measurement, resolved to what target-group checks and trigger
/// checkers read (the `PcmmeasuringpointSwitch` cases of `MeasuringPointInsideTargetGroup`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointKind {
    /// `ActiveResourceMeasuringPoint`: the container of the processing resource.
    ActiveResource { container: ObjId },
    /// An `OperationReference` (system, assembly or subsystem operation measuring point).
    Operation { signature: ObjId },
    /// `AssemblyPassiveResourceMeasuringPoint` (an `AssemblyReference`).
    AssemblyPassiveResource { assembly: ObjId, passive: ObjId },
    /// `ResourceContainerMeasuringPoint`.
    Container { container: ObjId },
    /// `ResourceEnvironmentMeasuringPoint`.
    Environment,
    /// Any other measuring point (never inside a target group).
    Other,
}

/// A resolved measuring point: its kind and its metamodel class name (messages).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointRef {
    pub kind: PointKind,
    pub class: &'static str,
}

impl PointRef {
    /// Resolves measuring point `mp` of `g`.
    pub fn from_graph(g: &Graph, mp: ObjId) -> PointRef {
        let c = g[mp].class;
        let kind = if c.is_a(class::ActiveResourceMeasuringPoint) {
            g.get_ref(mp, feat::ActiveResourceReference_activeResource)
                .and_then(|r| {
                    g.get_ref(
                        r,
                        feat::ProcessingResourceSpecification_resourceContainer_ProcessingResourceSpecification,
                    )
                })
                .map_or(PointKind::Other, |container| PointKind::ActiveResource {
                    container,
                })
        } else if c.is_a(class::OperationReference) {
            g.get_ref(mp, feat::OperationReference_operationSignature)
                .map_or(PointKind::Other, |signature| PointKind::Operation {
                    signature,
                })
        } else if c.is_a(class::AssemblyReference) && c.is_a(class::PassiveResourceReference) {
            match (
                g.get_ref(mp, feat::AssemblyReference_assembly),
                g.get_ref(mp, feat::PassiveResourceReference_passiveResource),
            ) {
                (Some(assembly), Some(passive)) => {
                    PointKind::AssemblyPassiveResource { assembly, passive }
                }
                _ => PointKind::Other,
            }
        } else if c.is_a(class::ResourceContainerReference) {
            g.get_ref(mp, feat::ResourceContainerReference_resourceContainer)
                .map_or(PointKind::Other, |container| PointKind::Container {
                    container,
                })
        } else if c.is_a(class::ResourceEnvironmentReference) {
            PointKind::Environment
        } else {
            PointKind::Other
        };
        PointRef {
            kind,
            class: c.name(),
        }
    }
}

/// `MeasuringPointInsideTargetGroup`: is the measuring point inside the group?
pub(crate) fn point_in_group(
    m: &Membership,
    group: &TargetGroup,
    p: &PointRef,
) -> Result<bool, &'static str> {
    match p.kind {
        PointKind::ActiveResource { container } | PointKind::Container { container } => {
            container_in_group(m, group, container)
        }
        PointKind::Environment => {
            for &c in &m.environment {
                if container_in_group(m, group, c)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        PointKind::AssemblyPassiveResource { assembly, .. } => {
            Ok(assembly_in_group(m, group, assembly))
        }
        PointKind::Operation { signature } => Ok(signature_in_group(m, group, signature)),
        PointKind::Other => Ok(false),
    }
}

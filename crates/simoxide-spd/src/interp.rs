//! The interpreter: one [`PolicyCtx`] per active policy with its filter chain, evaluated per
//! event against a plain-data [`SpdState`] (`DESIGN-SPD.md` §1.2, §5.4; `docs/spec/slingshot/
//! spd.md`, SS-SPD-1 to SS-SPD-3).
//!
//! The chain, in Slingshot's order (`SPDAdjustorContext.java:80-94`):
//! target-group check, trigger, the group's thrashing constraints (fixed Q9: and its size
//! constraints, in model order), the policy constraints in model order, the adjustor.
//!
//! The chain runs like Slingshot's `FilterChain`: from the start, unless a filter returned no
//! result (`null`, only Slingshot's broken compound filters do, Q12), which parks the chain
//! after that filter until the next event. With the Q12 compat flag the compound filters are
//! ports of `LogicalANDComboundFilter`, `LogicalORCompoundFilter` and
//! `LogicalXORCompoundFilter` including their iterator state; without it they are plain
//! AND / OR / XOR of the children's results.

use crate::adjust;
use crate::aggregate::{PerElement, Window};
use crate::jfmt::f6;
use crate::log::{FilterRecord, Log, Record};
use crate::model::*;
use crate::quirks::Quirks;
use simoxide_model::raw::ObjId;

/// Index of a time event of [`Spd::timers`].
pub type TimerId = u32;

/// A `SimulationTimeReached` the set-up schedules: one per `SimulationTime` trigger (also
/// inside composed triggers) of an active policy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeEvent {
    /// The policy context that scheduled it.
    pub ctx: usize,
    /// Its target group (index into [`SpdModel::groups`]).
    pub group: usize,
    /// The expected time (s).
    pub time: f64,
}

/// The metric of a measurement tuple, as far as the triggers distinguish them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    /// `Response Time Tuple`.
    ResponseTime,
    /// `Utilization of Active Resource Tuple` (CPU utilisation).
    Utilization,
    /// `State of Active Resource Tuple` (task count).
    ActiveResourceState,
    /// `State of Passive Resource Tuple` (queue length).
    PassiveResourceState,
    Other,
}

/// A measurement tuple of a series the host observes (Slingshot: `MeasurementMade`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measurement {
    pub point: PointRef,
    pub metric: Metric,
    /// The tuple's point in time (s).
    pub point_in_time: f64,
    pub value: f64,
    /// The host's key of the measuring point (series); the fixed Q13 keeps one window per key.
    pub source: u32,
}

/// An adjustment the interpreter requests: the output of a policy whose chain passed
/// (Slingshot: `ModelAdjustmentRequested`). The host queues it at the current time, behind the
/// events already queued for that time, executes it (P2: `MainTransformation.qvto`) and reports
/// the outcome with [`Spd::adjusted`].
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct AdjustmentRequest {
    pub id: u64,
    /// Index into [`SpdModel::policies`].
    pub policy: usize,
    /// Index into [`Spd::contexts`].
    pub ctx: usize,
    /// Index into [`SpdModel::groups`].
    pub group: usize,
    pub adjustment: Adjustment,
    /// When the request was made (s).
    pub time: f64,
}

/// What the interpreter needs from the simulation it runs in.
pub trait SpdHost {
    /// The current simulation time (s).
    fn now(&self) -> f64;
    /// Schedules time event `timer` at `at` (s): dispatched at `at`, behind the events already
    /// scheduled for that time; the host then calls [`Spd::on_timer`].
    fn schedule(&mut self, at: f64, timer: TimerId);
    /// Queues an adjustment request at the current time (FIFO behind the events already queued
    /// for now).
    fn request(&mut self, req: AdjustmentRequest);
}

// ---------------------------------------------------------------------------------------------
// the compiled chain

/// What a trigger leaf checks.
#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    /// `SimulationTimeChecker`.
    Time,
    /// `OperationResponseTimeTriggerChecker`.
    ResponseTime {
        signature: Option<ObjId>,
        signature_name: Box<str>,
    },
    /// `CPUUtilizationTriggerChecker`.
    Cpu { aggregation: Aggregation },
    /// `TaskCountTriggerChecker`.
    TaskCount { aggregation: Aggregation },
    /// `QueueLengthTriggerChecker`.
    Queue { passive_resource: Option<ObjId> },
}

impl Check {
    fn class_name(&self) -> &'static str {
        match self {
            Check::Time => "SimulationTimeChecker",
            Check::ResponseTime { .. } => "OperationResponseTimeTriggerChecker",
            Check::Cpu { .. } => "CPUUtilizationTriggerChecker",
            Check::TaskCount { .. } => "TaskCountTriggerChecker",
            Check::Queue { .. } => "QueueLengthTriggerChecker",
        }
    }

    /// The subscriber name of the stimulus (`StimuliInterpreter`): two leaves of one policy
    /// with the same kind share a subscriber.
    fn subscriber(&self) -> u8 {
        match self {
            Check::Time => 0,
            Check::ResponseTime { .. } => 1,
            Check::Cpu { .. } => 2,
            Check::TaskCount { .. } => 3,
            Check::Queue { .. } => 4,
        }
    }

    /// `allowedExpectedPrimitives` of the checker.
    fn allows(&self, e: &Expected) -> bool {
        matches!(
            (self, e),
            (Check::Time | Check::ResponseTime { .. }, Expected::Time(_))
                | (Check::Cpu { .. }, Expected::Percentage(_))
                | (
                    Check::TaskCount { .. } | Check::Queue { .. },
                    Expected::Count(_)
                )
        )
    }
}

/// A node of a policy's trigger tree.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Leaf {
        trigger_id: Box<str>,
        op: RelOp,
        check: Check,
        expected: Expected,
    },
    Compound {
        op: LogicalOp,
        children: Vec<usize>,
    },
}

impl Node {
    fn class_name(&self) -> &'static str {
        match self {
            Node::Leaf { check, .. } => check.class_name(),
            Node::Compound { op, .. } => match op {
                LogicalOp::And => "LogicalANDComboundFilter",
                LogicalOp::Or => "LogicalORCompoundFilter",
                LogicalOp::Xor => "LogicalXORCompoundFilter",
            },
        }
    }
}

/// A filter of the chain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Filter {
    TargetGroup,
    Trigger,
    Thrashing {
        minimum_time: f64,
    },
    /// Fixed Q9 only.
    GroupSize {
        min: i64,
        max: i64,
    },
    Cooldown {
        time: f64,
        max_operations: i64,
    },
    Interval {
        offset: f64,
        duration: f64,
        repeat: bool,
    },
    Adjustor,
}

/// The compiled context of one active policy (`SPDAdjustorContext`).
#[derive(Clone, Debug, PartialEq)]
pub struct PolicyCtx {
    /// Index into [`SpdModel::policies`].
    pub policy: usize,
    pub group: usize,
    pub chain: Vec<Filter>,
    /// The trigger tree; `root` is its root.
    pub nodes: Vec<Node>,
    pub root: usize,
    /// Subscribed to time events.
    pub time: bool,
    /// Number of measurement subscribers (distinct measurement stimulus kinds of the trigger).
    pub measurement: u8,
    /// The class names of `chain` (the `policy` record).
    pub names: Vec<&'static str>,
}

// ---------------------------------------------------------------------------------------------
// state

/// `SPDAdjustorState` of one policy (the scaling counters and the cooldown).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct AdjustorState {
    pub latest_adjustment: f64,
    pub scales: u64,
    pub cooldown_end: f64,
    pub scales_in_cooldown: i64,
}

/// `TargetGroupState`: the last enacted policy of the group and its time.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct GroupState {
    /// (time, policy index) of the last enactment.
    pub last: Option<(f64, usize)>,
    pub enactments: u64,
}

/// Per trigger node: aggregation windows (leaves) or the iterator position of a compound
/// filter (Q12 compat).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub enum NodeState {
    Leaf {
        /// Slingshot's single window (Q13 compat).
        window: Option<Window>,
        /// One window per element (fixed Q13).
        per_element: Option<PerElement>,
        /// Fixed Q13: the evaluation read the value; committed if the policy fires.
        read: bool,
    },
    Compound {
        /// `FilterChain.iterator`: `None` = not in use (restart from the first child).
        iter: Option<u16>,
    },
}

/// The state of one policy context.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct PolicyState {
    pub adjustor: AdjustorState,
    /// The outer chain's iterator: `Some(i)` = parked before filter `i` (Q12 compat).
    pub chain_at: Option<u16>,
    pub nodes: Vec<NodeState>,
    /// The handler has been invoked (the `policy` record is written).
    pub seen: bool,
}

/// All mutable SPD state of a run: plain data, cloned with the run state (forks, snapshots).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpdState {
    /// Per policy context.
    pub policies: Vec<PolicyState>,
    /// Per target group.
    pub groups: Vec<GroupState>,
    pub membership: Membership,
    pub next_request: u64,
    pub evaluations: u64,
    pub requests: u64,
}

// ---------------------------------------------------------------------------------------------
// filter results

/// `ComparatorResult`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmp {
    InAccordance,
    Disregard,
    WrongExpectedValue,
}

impl Cmp {
    fn name(self) -> &'static str {
        match self {
            Cmp::InAccordance => "IN_ACCORDANCE",
            Cmp::Disregard => "DISREGARD",
            Cmp::WrongExpectedValue => "WRONG_EXPECTED_VALUE",
        }
    }
}

/// Why a filter disregarded (formatted only for the log, as Slingshot's messages).
#[derive(Clone, Debug, PartialEq)]
pub enum Reason {
    NotInGroup,
    GroupMismatch,
    WrongEventForGroup,
    TimeWrongEvent,
    TimeNot {
        time: f64,
        op: RelOp,
        cmp: Cmp,
    },
    NotMeasurement,
    NotStimulusMetric,
    NotEmittable,
    NotInAccordance,
    OrtNotMet {
        trigger: Box<str>,
    },
    OrtWrongSignature {
        expected: Box<str>,
        got: Box<str>,
    },
    OrtWrongPoint {
        class: &'static str,
    },
    OrtNotMeasurement,
    QueueNot {
        cmp: Cmp,
    },
    Empty,
    Cooldown {
        count: i64,
        max: i64,
    },
    Interval {
        repeat: bool,
        value: f64,
        offset: f64,
        delta: f64,
    },
    Thrashing,
    GroupSizeZero {
        size: usize,
    },
    GroupSizeInvalid(String),
    XorMore,
    XorNone(Box<Reason>),
    OrNone(Box<Reason>),
    Exception(String),
}

impl Reason {
    /// Slingshot's message (`FilterResult.disregard(...)` reason text).
    pub fn message(&self) -> String {
        match self {
            Reason::NotInGroup => "The measurement is not inside this target group.".into(),
            Reason::GroupMismatch => "The target group does not match the event.".into(),
            Reason::WrongEventForGroup => "The event can only be checked if it is a MeasurementMade OR SimulationTimeReached at the moment.".into(),
            Reason::TimeWrongEvent => {
                "Expected simulation time reached event, but receivedMeasurementMade".into()
            }
            Reason::TimeNot { time, op, cmp } => format!(
                "Simulationtime of {} is not [{}] expectedValue. [[{}]]",
                f6(*time),
                op.literal(),
                cmp.name()
            ),
            Reason::NotMeasurement => "Not a measurement made event.".into(),
            Reason::NotStimulusMetric => "Not a measurement of the stimulus' metric.".into(),
            Reason::NotEmittable => "Values not emittable.".into(),
            Reason::NotInAccordance => "Value and Expectation not in accordance.".into(),
            Reason::OrtNotMet { trigger } => {
                format!("Conditions for trigger {trigger} are not met.")
            }
            Reason::OrtWrongSignature { expected, got } => {
                format!("Expected signature {expected} but received {got}")
            }
            Reason::OrtWrongPoint { class } => format!(
                "Expected measuringpoint of type OperationReference, but received measuringpoint of type {class}Impl"
            ),
            Reason::OrtNotMeasurement => {
                "Expected measurement made event, but received SimulationTimeReached".into()
            }
            Reason::QueueNot { cmp } => format!(
                "Cannot trigger, since the result is not in accordance: {}",
                cmp.name()
            ),
            Reason::Empty => String::new(),
            Reason::Cooldown { count, max } => {
                format!("Max number scales reached: {count} >= {max}")
            }
            Reason::Interval {
                repeat,
                value,
                offset,
                delta,
            } => {
                if *repeat {
                    format!(
                        "The simulation time is inside of the interval or zero: {} in [{}, {}]",
                        f6(*value),
                        f6(*offset),
                        f6(*delta)
                    )
                } else {
                    format!(
                        "The simulation time is inside of the interval: {} in [{}, {}]",
                        f6(*value),
                        f6(*offset),
                        f6(*delta)
                    )
                }
            }
            Reason::Thrashing => "Thrashing Constraint prevents this scaling Operation.".into(),
            Reason::GroupSizeZero { size } => format!(
                "Target group size constraint: the change of a group of {size} elements is clamped to 0."
            ),
            Reason::GroupSizeInvalid(m) => m.clone(),
            Reason::XorMore => "More than one filter through XOR".into(),
            Reason::XorNone(m) => format!("No filter went through. Last message: {}", m.message()),
            Reason::OrNone(m) => format!("No filter went through. Last message: {}", m.message()),
            Reason::Exception(m) => m.clone(),
        }
    }
}

/// `FilterResult` (plus Slingshot's `null` and a thrown exception).
#[derive(Clone, Debug, PartialEq)]
pub enum FRes {
    Ok,
    Disregard(Reason),
    /// No result (Slingshot's OR filter, an XOR filter at the end of its children).
    Null,
    /// The filter threw: the chain disregards.
    Exception(String),
}

impl FRes {
    fn record(&self, name: &'static str) -> FilterRecord {
        match self {
            FRes::Ok => FilterRecord {
                name,
                result: "ok",
                reason: None,
            },
            FRes::Disregard(r) => FilterRecord {
                name,
                result: "disregard",
                reason: Some(r.message()),
            },
            FRes::Null => FilterRecord {
                name,
                result: "null",
                reason: None,
            },
            FRes::Exception(m) => FilterRecord {
                name,
                result: "exception",
                reason: Some(m.clone()),
            },
        }
    }

    /// The disregard message a compound filter passes on (`reason().toString()`).
    fn as_reason(&self) -> Reason {
        match self {
            FRes::Disregard(r) => r.clone(),
            FRes::Exception(m) => Reason::Exception(m.clone()),
            FRes::Ok | FRes::Null => Reason::Empty,
        }
    }
}

/// The event a chain evaluates.
#[derive(Clone, Copy, Debug)]
enum Ev<'a> {
    Time { ev: TimeEvent },
    Measurement { m: &'a Measurement, time: f64 },
}

impl Ev<'_> {
    fn time(&self) -> f64 {
        match self {
            Ev::Time { ev, .. } => ev.time,
            Ev::Measurement { time, .. } => *time,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Ev::Time { .. } => "SimulationTimeReached",
            Ev::Measurement { .. } => "MeasurementMade",
        }
    }
}

const NPE: &str = "java.lang.NullPointerException";

// ---------------------------------------------------------------------------------------------

/// The SPD interpreter of one model: immutable, shared by runs and forks (`Send + Sync`); the
/// mutable part is the [`SpdState`].
#[derive(Clone, Debug)]
pub struct Spd {
    model: SpdModel,
    quirks: Quirks,
    ctxs: Vec<PolicyCtx>,
    timers: Vec<TimeEvent>,
    /// Q11 compat: the set-up error that made Slingshot drop the SPD.
    dropped: Option<String>,
    /// Signature names (messages of the response-time checker).
    signature_names: Vec<(ObjId, Box<str>)>,
}

impl Spd {
    /// Interprets `model` once (`SpdInterpreter`): one context per active policy, the time
    /// events to schedule. A set-up error is a configuration error (fixed Q11); with the Q11
    /// compat flag the interpreter is empty instead and [`Spd::dropped`] names the error.
    pub fn new(model: SpdModel, quirks: Quirks) -> Result<Spd, SpdError> {
        let mut spd = Spd {
            model,
            quirks,
            ctxs: Vec::new(),
            timers: Vec::new(),
            dropped: None,
            signature_names: Vec::new(),
        };
        match spd.setup() {
            Ok(()) => Ok(spd),
            Err(e) if quirks.q11_setup_error_drops_spd => {
                spd.ctxs.clear();
                spd.timers.clear();
                spd.dropped = Some(e.0);
                Ok(spd)
            }
            Err(e) => Err(e),
        }
    }

    /// Names for the response-time checker's messages (`Expected signature X but received Y`).
    pub fn with_signature_names(mut self, names: Vec<(ObjId, Box<str>)>) -> Spd {
        self.signature_names = names;
        self
    }

    pub fn model(&self) -> &SpdModel {
        &self.model
    }

    pub fn quirks(&self) -> Quirks {
        self.quirks
    }

    /// The policy contexts (active policies, model order).
    pub fn contexts(&self) -> &[PolicyCtx] {
        &self.ctxs
    }

    /// The time events of the set-up.
    pub fn timers(&self) -> &[TimeEvent] {
        &self.timers
    }

    /// Q11 compat: the set-up error after which Slingshot runs without SPD.
    pub fn dropped(&self) -> Option<&str> {
        self.dropped.as_deref()
    }

    /// Some policy is driven by measurements: the host must deliver measurements.
    pub fn uses_measurements(&self) -> bool {
        self.ctxs.iter().any(|c| c.measurement > 0)
    }

    /// The metrics measurement-driven policies read.
    pub fn metrics(&self) -> Vec<Metric> {
        let mut v = Vec::new();
        for c in &self.ctxs {
            for n in &c.nodes {
                if let Node::Leaf { check, .. } = n {
                    let m = match check {
                        Check::Time => continue,
                        Check::ResponseTime { .. } => Metric::ResponseTime,
                        Check::Cpu { .. } => Metric::Utilization,
                        Check::TaskCount { .. } => Metric::ActiveResourceState,
                        Check::Queue { .. } => Metric::PassiveResourceState,
                    };
                    if !v.contains(&m) {
                        v.push(m);
                    }
                }
            }
        }
        v
    }

    fn setup(&mut self) -> Result<(), SpdError> {
        let q = self.quirks;
        for (pi, p) in self.model.policies.iter().enumerate() {
            if !p.active {
                continue;
            }
            let err = |m: String| SpdError(format!("policy {} ({}): {m}", p.name, p.id));
            let group = p.group.ok_or_else(|| err("no target group".into()))?;
            let g = &self.model.groups[group];
            let trigger = p
                .trigger
                .as_ref()
                .ok_or_else(|| err("no scaling trigger".into()))?;
            let mut nodes = Vec::new();
            let mut times = Vec::new();
            let root = build_trigger(trigger, g, q.q12_broken_composed, &mut nodes, &mut times)
                .map_err(err)?;
            let mut chain = vec![Filter::TargetGroup, Filter::Trigger];
            for c in &g.constraints {
                match *c {
                    TargetConstraint::Thrashing { minimum_time } => {
                        chain.push(Filter::Thrashing { minimum_time })
                    }
                    TargetConstraint::Size { min, max } if !q.q9_no_size_check => {
                        chain.push(Filter::GroupSize { min, max })
                    }
                    TargetConstraint::Size { .. } => {}
                }
            }
            for c in &p.constraints {
                chain.push(match *c {
                    PolicyConstraint::Cooldown {
                        time,
                        max_operations,
                    } => Filter::Cooldown {
                        time,
                        max_operations,
                    },
                    PolicyConstraint::Interval {
                        offset,
                        duration,
                        repeat,
                    } => Filter::Interval {
                        offset,
                        duration,
                        repeat,
                    },
                    PolicyConstraint::Unsupported(c) => {
                        return Err(err(format!(
                            "constraint {c}: only cooldown, intervall and thrashing constraints are supported"
                        )));
                    }
                });
            }
            chain.push(Filter::Adjustor);
            let mut subs: Vec<u8> = nodes
                .iter()
                .filter_map(|n| match n {
                    Node::Leaf { check, .. } => Some(check.subscriber()),
                    _ => None,
                })
                .collect();
            subs.sort_unstable();
            subs.dedup();
            let time = subs.contains(&0);
            let measurement = subs.iter().filter(|&&s| s != 0).count() as u8;
            let names = chain
                .iter()
                .map(|f| match f {
                    Filter::TargetGroup => "TargetGroupChecker",
                    Filter::Trigger => nodes[root].class_name(),
                    Filter::Thrashing { .. } => "ThrashingConstraintFilter",
                    Filter::GroupSize { .. } => "TargetGroupSizeConstraintFilter",
                    Filter::Cooldown { .. } => "CooldownConstraintFilter",
                    Filter::Interval { .. } => "IntervalConstraintFilter",
                    Filter::Adjustor => "Adjustor",
                })
                .collect();
            let ctx = self.ctxs.len();
            for t in times {
                self.timers.push(TimeEvent {
                    ctx,
                    group,
                    time: t,
                });
            }
            self.ctxs.push(PolicyCtx {
                policy: pi,
                group,
                chain,
                nodes,
                root,
                time,
                measurement,
                names,
            });
        }
        Ok(())
    }

    /// The initial state, with the target groups' current elements `membership`
    /// ([`Membership::from_graph`]).
    pub fn state(&self, membership: Membership) -> SpdState {
        let q13 = self.quirks.q13_merged_samples;
        SpdState {
            policies: self
                .ctxs
                .iter()
                .map(|c| {
                    PolicyState {
                        adjustor: AdjustorState::default(),
                        chain_at: None,
                        nodes:
                            c.nodes
                                .iter()
                                .map(|n| match n {
                                    Node::Leaf {
                                        check:
                                            Check::Cpu { aggregation }
                                            | Check::TaskCount { aggregation },
                                        ..
                                    } => NodeState::Leaf {
                                        window: q13.then(|| Window::for_aggregation(*aggregation)),
                                        per_element: (!q13).then(|| PerElement::new(*aggregation)),
                                        read: false,
                                    },
                                    Node::Leaf { .. } => NodeState::Leaf {
                                        window: None,
                                        per_element: None,
                                        read: false,
                                    },
                                    Node::Compound { .. } => NodeState::Compound { iter: None },
                                })
                                .collect(),
                        seen: false,
                    }
                })
                .collect(),
            groups: vec![GroupState::default(); self.model.groups.len()],
            membership,
            next_request: 0,
            evaluations: 0,
            requests: 0,
        }
    }

    /// Schedules the set-up's time events (`SpdBehavior` at `PreSimulationConfigurationStarted`):
    /// in policy order, within a policy in trigger order.
    pub fn start(&self, host: &mut dyn SpdHost) {
        for (i, t) in self.timers.iter().enumerate() {
            host.schedule(t.time, i as TimerId);
        }
    }

    /// A time event is dispatched: every policy subscribed to time events evaluates it.
    pub fn on_timer(
        &self,
        st: &mut SpdState,
        host: &mut dyn SpdHost,
        timer: TimerId,
        log: &mut dyn Log,
    ) {
        let ev = Ev::Time {
            ev: self.timers[timer as usize],
        };
        for c in 0..self.ctxs.len() {
            if self.ctxs[c].time {
                self.evaluate(c, st, host, &ev, log);
            }
        }
    }

    /// A measurement of an observed series: every measurement-driven policy evaluates it (with
    /// the Q12 compat flag once per measurement subscriber).
    pub fn on_measurement(
        &self,
        st: &mut SpdState,
        host: &mut dyn SpdHost,
        m: &Measurement,
        log: &mut dyn Log,
    ) {
        let ev = Ev::Measurement {
            m,
            time: host.now(),
        };
        for c in 0..self.ctxs.len() {
            let n = match self.ctxs[c].measurement {
                0 => 0,
                k if self.quirks.q12_broken_composed => k,
                _ => 1,
            };
            for _ in 0..n {
                self.evaluate(c, st, host, &ev, log);
            }
        }
    }

    /// The executor of `req` reports its outcome (`ModelAdjusted`): with the fixed Q2 a
    /// successful adjustment commits the adjustor's state change; `size_before`/`size_after`
    /// (the target group's element counts, if known) go into the log. The host updates
    /// [`SpdState::membership`] itself.
    pub fn adjusted(
        &self,
        st: &mut SpdState,
        req: &AdjustmentRequest,
        ok: bool,
        now: f64,
        sizes: (Option<usize>, Option<usize>),
        log: &mut dyn Log,
    ) {
        if ok && !self.quirks.q2_commit_on_request {
            self.commit(st, req.ctx, req.time);
        }
        if log.enabled() {
            let p = &self.model.policies[req.policy];
            log.record(Record::Adjust {
                t: now,
                policy: p.id.clone(),
                policy_name: p.name.clone(),
                ok,
                size_before: sizes.0,
                size_after: sizes.1,
            });
        }
    }

    /// One handler invocation of policy context `c` for `ev`.
    fn evaluate(
        &self,
        c: usize,
        st: &mut SpdState,
        host: &mut dyn SpdHost,
        ev: &Ev,
        log: &mut dyn Log,
    ) {
        let ctx = &self.ctxs[c];
        let logging = log.enabled();
        if logging && !st.policies[c].seen {
            let p = &self.model.policies[ctx.policy];
            log.record(Record::Policy {
                id: p.id.clone(),
                name: p.name.clone(),
                active: p.active,
                target_group: Some(self.model.groups[ctx.group].id.clone()),
                adjustment: p.adjustment.class_name(),
                trigger: p.trigger.as_ref().map(|t| match t.kind {
                    TriggerKind::Value { .. } => "SimpleFireOnValue",
                    TriggerKind::Trend => "SimpleFireOnTrend",
                    TriggerKind::Composed { .. } => "ComposedTrigger",
                }),
                chain: ctx.names.clone(),
            });
        }
        st.policies[c].seen = true;
        st.evaluations += 1;
        let mut recs: Vec<FilterRecord> = Vec::new();
        // FilterChain.next: from the start unless parked
        let mut pos = st.policies[c].chain_at.take().map_or(0, usize::from);
        let mut request = false;
        let mut group_rejected = false;
        while pos < ctx.chain.len() {
            let f = ctx.chain[pos];
            pos += 1;
            let r = self.filter(c, f, st, ev);
            if logging {
                recs.push(r.record(ctx.names[pos - 1]));
            }
            match r {
                FRes::Ok => {
                    if f == Filter::Adjustor {
                        request = true;
                    }
                }
                FRes::Disregard(_) | FRes::Exception(_) => {
                    if f == Filter::TargetGroup && matches!(r, FRes::Disregard(_)) {
                        group_rejected = true;
                    }
                    break;
                }
                FRes::Null => {
                    st.policies[c].chain_at = Some(pos as u16);
                    break;
                }
            }
        }
        if !self.quirks.q13_merged_samples {
            for n in st.policies[c].nodes.iter_mut() {
                if let NodeState::Leaf {
                    per_element: Some(p),
                    read,
                    ..
                } = n
                {
                    if *read && request {
                        p.commit();
                    }
                    *read = false;
                }
            }
        }
        if logging && (request || !group_rejected || log.all()) {
            log.record(Record::Eval {
                t: ev.time(),
                ev: ev.name(),
                policy: self.model.policies[ctx.policy].id.clone(),
                filters: recs,
                request,
            });
        }
        if request {
            st.requests += 1;
            let req = AdjustmentRequest {
                id: st.next_request,
                policy: ctx.policy,
                ctx: c,
                group: ctx.group,
                adjustment: self.model.policies[ctx.policy].adjustment,
                time: ev.time(),
            };
            st.next_request += 1;
            host.request(req);
        }
    }

    fn filter(&self, c: usize, f: Filter, st: &mut SpdState, ev: &Ev) -> FRes {
        let ctx = &self.ctxs[c];
        let t = ev.time();
        match f {
            Filter::TargetGroup => match ev {
                Ev::Time { ev: te, .. } => {
                    let ok = if self.quirks.q15_shared_time_events {
                        self.model.groups[te.group].id == self.model.groups[ctx.group].id
                    } else {
                        te.ctx == c
                    };
                    if ok {
                        FRes::Ok
                    } else {
                        FRes::Disregard(Reason::GroupMismatch)
                    }
                }
                Ev::Measurement { m, .. } => {
                    match point_in_group(&st.membership, &self.model.groups[ctx.group], &m.point) {
                        Ok(true) => FRes::Ok,
                        Ok(false) => FRes::Disregard(Reason::NotInGroup),
                        Err(e) => FRes::Exception(e.into()),
                    }
                }
            },
            Filter::Trigger => {
                let PolicyState { nodes, .. } = &mut st.policies[c];
                if self.quirks.q12_broken_composed {
                    self.node_compat(ctx, nodes, ctx.root, Some(ev))
                } else {
                    self.node_fixed(ctx, nodes, ctx.root, ev)
                }
            }
            Filter::Thrashing { minimum_time } => {
                let gs = &st.groups[ctx.group];
                let Some((last_t, last_p)) = gs.last else {
                    return FRes::Ok;
                };
                let cur = &self.model.policies[ctx.policy].adjustment;
                let last = &self.model.policies[last_p].adjustment;
                if matches!(cur, Adjustment::Absolute { .. })
                    || matches!(last, Adjustment::Absolute { .. })
                {
                    return FRes::Ok;
                }
                if sign(cur) != sign(last) && last_t + minimum_time >= t {
                    FRes::Disregard(Reason::Thrashing)
                } else {
                    FRes::Ok
                }
            }
            Filter::GroupSize { min, max } => {
                let g = &self.model.groups[ctx.group];
                let Some(size) = st.membership.group_size(g) else {
                    return FRes::Ok;
                };
                let adj = &self.model.policies[ctx.policy].adjustment;
                let r = adjust::desired_change(adj, size as i64, self.quirks.q4_relative_zero)
                    .and_then(|ch| adjust::clamp(ch, size as i64, min, max));
                match r {
                    Err(m) => FRes::Disregard(Reason::GroupSizeInvalid(m)),
                    Ok(ch) if adjust::effective_change(ch, size as i64) == 0 => {
                        FRes::Disregard(Reason::GroupSizeZero { size })
                    }
                    Ok(_) => FRes::Ok,
                }
            }
            Filter::Cooldown { max_operations, .. } => {
                let a = &st.policies[c].adjustor;
                if t >= a.cooldown_end || a.scales_in_cooldown < max_operations {
                    FRes::Ok
                } else {
                    FRes::Disregard(Reason::Cooldown {
                        count: a.scales_in_cooldown,
                        max: max_operations,
                    })
                }
            }
            Filter::Interval {
                offset,
                duration,
                repeat,
            } => {
                let delta = offset + duration;
                let within = (repeat && t != 0.0 && (t % delta == 0.0 || t % delta >= offset))
                    || (!repeat && t >= offset && t <= delta);
                if within {
                    FRes::Disregard(Reason::Interval {
                        repeat,
                        value: if repeat { t % delta } else { t },
                        offset,
                        delta,
                    })
                } else {
                    FRes::Ok
                }
            }
            Filter::Adjustor => {
                if self.quirks.q2_commit_on_request {
                    self.commit(st, c, t);
                }
                FRes::Ok
            }
        }
    }

    /// `Adjustor.doProcess`'s state change at time `t`.
    fn commit(&self, st: &mut SpdState, c: usize, t: f64) {
        let ctx = &self.ctxs[c];
        let a = &mut st.policies[c].adjustor;
        a.latest_adjustment = t;
        a.scales += 1;
        let g = &mut st.groups[ctx.group];
        g.last = Some((t, ctx.policy));
        g.enactments += 1;
        let cooldown = self.model.policies[ctx.policy]
            .constraints
            .iter()
            .find_map(|c| match c {
                PolicyConstraint::Cooldown { time, .. } => Some(*time),
                _ => None,
            });
        if let Some(time) = cooldown
            && time != 0.0
        {
            if t > a.cooldown_end {
                a.scales_in_cooldown = 0;
                a.cooldown_end = t + time;
            } else if t == a.cooldown_end {
                a.cooldown_end = t + time;
            } else {
                a.scales_in_cooldown += 1;
            }
        }
    }

    // ---------------------------------------------------------------------------- trigger leaves

    /// `TriggerChecker.compareToTrigger`.
    fn compare(&self, check: &Check, op: RelOp, expected: &Expected, value: f64) -> Cmp {
        if *expected == Expected::Nothing {
            return Cmp::InAccordance;
        }
        if !check.allows(expected) {
            return Cmp::WrongExpectedValue;
        }
        let e = match *expected {
            Expected::Time(v) => v,
            Expected::Count(c) => c as f64,
            Expected::Percentage(p) => p / 100.0,
            _ => return Cmp::WrongExpectedValue,
        };
        if op.holds(value, e) {
            Cmp::InAccordance
        } else {
            Cmp::Disregard
        }
    }

    fn leaf(&self, node: &Node, ns: &mut NodeState, ev: Option<&Ev>) -> FRes {
        let Node::Leaf {
            trigger_id,
            op,
            check,
            expected,
        } = node
        else {
            unreachable!("leaf")
        };
        let Some(ev) = ev else {
            return FRes::Exception(NPE.into());
        };
        match check {
            Check::Time => match ev {
                Ev::Time { ev: te, .. } => match self.compare(check, *op, expected, te.time) {
                    Cmp::InAccordance => FRes::Ok,
                    cmp => FRes::Disregard(Reason::TimeNot {
                        time: te.time,
                        op: *op,
                        cmp,
                    }),
                },
                Ev::Measurement { .. } => FRes::Disregard(Reason::TimeWrongEvent),
            },
            Check::ResponseTime {
                signature,
                signature_name,
            } => {
                let Ev::Measurement { m, .. } = ev else {
                    return FRes::Disregard(Reason::OrtNotMeasurement);
                };
                let PointKind::Operation { signature: got } = m.point.kind else {
                    return FRes::Disregard(Reason::OrtWrongPoint {
                        class: m.point.class,
                    });
                };
                if *signature != Some(got) {
                    let got_name = self
                        .signature_names
                        .iter()
                        .find(|(o, _)| *o == got)
                        .map_or_else(|| "".into(), |(_, n)| n.clone());
                    return FRes::Disregard(Reason::OrtWrongSignature {
                        expected: signature_name.clone(),
                        got: got_name,
                    });
                }
                if m.metric != Metric::ResponseTime {
                    return FRes::Exception(
                        "java.lang.IllegalArgumentException: no response time in the tuple".into(),
                    );
                }
                match self.compare(check, *op, expected, m.value) {
                    Cmp::InAccordance => FRes::Ok,
                    _ => FRes::Disregard(Reason::OrtNotMet {
                        trigger: trigger_id.clone(),
                    }),
                }
            }
            Check::Cpu { .. } | Check::TaskCount { .. } => {
                let Ev::Measurement { m, .. } = ev else {
                    return FRes::Disregard(Reason::NotMeasurement);
                };
                let metric = if matches!(check, Check::Cpu { .. }) {
                    Metric::Utilization
                } else {
                    Metric::ActiveResourceState
                };
                let NodeState::Leaf {
                    window,
                    per_element,
                    read,
                } = ns
                else {
                    unreachable!("leaf state")
                };
                if let Some(w) = window {
                    // Q13 compat: one window; getResult for every event reaching the checker
                    if m.metric == metric {
                        w.aggregate(m.point_in_time, m.value);
                    }
                    if !w.emittable() {
                        return FRes::Disregard(Reason::NotEmittable);
                    }
                    let v = w.take();
                    match self.compare(check, *op, expected, v) {
                        Cmp::InAccordance => FRes::Ok,
                        _ => FRes::Disregard(Reason::NotInAccordance),
                    }
                } else {
                    let p = per_element.as_mut().expect("per-element windows");
                    if m.metric != metric {
                        return FRes::Disregard(Reason::NotStimulusMetric);
                    }
                    p.aggregate(m.source, m.point_in_time, m.value);
                    if !p.emittable() {
                        return FRes::Disregard(Reason::NotEmittable);
                    }
                    *read = true;
                    match self.compare(check, *op, expected, p.value()) {
                        Cmp::InAccordance => FRes::Ok,
                        _ => FRes::Disregard(Reason::NotInAccordance),
                    }
                }
            }
            Check::Queue { passive_resource } => {
                let Ev::Measurement { m, .. } = ev else {
                    return FRes::Disregard(Reason::Empty);
                };
                let PointKind::AssemblyPassiveResource { passive, .. } = m.point.kind else {
                    return FRes::Disregard(Reason::Empty);
                };
                if Some(passive) != *passive_resource || m.metric != Metric::PassiveResourceState {
                    return FRes::Disregard(Reason::Empty);
                }
                match self.compare(check, *op, expected, m.value) {
                    Cmp::InAccordance => FRes::Ok,
                    cmp => FRes::Disregard(Reason::QueueNot { cmp }),
                }
            }
        }
    }

    // ---------------------------------------------------------------------- fixed compounds (Q12)

    fn node_fixed(&self, ctx: &PolicyCtx, ns: &mut [NodeState], n: usize, ev: &Ev) -> FRes {
        match &ctx.nodes[n] {
            leaf @ Node::Leaf { .. } => self.leaf(leaf, &mut ns[n], Some(ev)),
            Node::Compound { op, children } => {
                // every child sees every event (stateful children aggregate independently)
                let rs: Vec<FRes> = children
                    .iter()
                    .map(|&ch| self.node_fixed(ctx, ns, ch, ev))
                    .collect();
                let oks = rs.iter().filter(|r| **r == FRes::Ok).count();
                let last_fail = || {
                    rs.iter()
                        .rev()
                        .find(|r| **r != FRes::Ok)
                        .map(FRes::as_reason)
                        .unwrap_or(Reason::Empty)
                };
                match op {
                    LogicalOp::And => match rs.iter().find(|r| **r != FRes::Ok) {
                        None => FRes::Ok,
                        Some(r) => FRes::Disregard(r.as_reason()),
                    },
                    LogicalOp::Or if oks > 0 => FRes::Ok,
                    LogicalOp::Or => FRes::Disregard(Reason::OrNone(Box::new(last_fail()))),
                    LogicalOp::Xor if oks == 1 => FRes::Ok,
                    LogicalOp::Xor if oks > 1 => FRes::Disregard(Reason::XorMore),
                    LogicalOp::Xor => FRes::Disregard(Reason::XorNone(Box::new(last_fail()))),
                }
            }
        }
    }

    // ----------------------------------------------------------------- Slingshot compounds (Q12)

    /// `filter.doProcess` of node `n` (Slingshot's compound filters).
    fn node_compat(
        &self,
        ctx: &PolicyCtx,
        ns: &mut [NodeState],
        n: usize,
        ev: Option<&Ev>,
    ) -> FRes {
        match &ctx.nodes[n] {
            leaf @ Node::Leaf { .. } => self.leaf(leaf, &mut ns[n], ev),
            Node::Compound { op, children } => match op {
                LogicalOp::And => {
                    // doProcess: result = null; next(event); return result
                    let mut result = None;
                    self.and_next(ctx, ns, n, children, ev, &mut result);
                    result.unwrap_or(FRes::Null)
                }
                // next() re-initialises without evaluating any child
                LogicalOp::Or => FRes::Null,
                LogicalOp::Xor => {
                    let mut x = Xor {
                        result: None,
                        continued: 0,
                        disregarded: 0,
                        event: ev.copied(),
                    };
                    self.xor_next(ctx, ns, n, children, &mut x);
                    x.result.unwrap_or(FRes::Null)
                }
            },
        }
    }

    fn iter_of(ns: &mut [NodeState], n: usize) -> &mut Option<u16> {
        match &mut ns[n] {
            NodeState::Compound { iter } => iter,
            NodeState::Leaf { .. } => unreachable!("compound state"),
        }
    }

    /// `FilterChain.next` of a compound's own chain: the next child, with the compound's
    /// callbacks for success (`next`) and disregard. Returns the child's result.
    fn chain_step(
        &self,
        ctx: &PolicyCtx,
        ns: &mut [NodeState],
        n: usize,
        children: &[usize],
        ev: Option<&Ev>,
    ) -> Option<FRes> {
        let it = Self::iter_of(ns, n);
        let i = usize::from(it.unwrap_or(0));
        if i < children.len() {
            *it = Some(i as u16 + 1);
            Some(self.node_compat(ctx, ns, children[i], ev))
        } else {
            *it = None;
            None
        }
    }

    /// `LogicalANDComboundFilter.next`: the chain step; then success if the chain is no longer
    /// in use. Its `disregard` sets the result and leaves the iterator where it is.
    fn and_next(
        &self,
        ctx: &PolicyCtx,
        ns: &mut [NodeState],
        n: usize,
        children: &[usize],
        ev: Option<&Ev>,
        result: &mut Option<FRes>,
    ) {
        match self.chain_step(ctx, ns, n, children, ev) {
            Some(FRes::Ok) => self.and_next(ctx, ns, n, children, ev, result),
            Some(r @ (FRes::Disregard(_) | FRes::Exception(_))) => {
                *result = Some(FRes::Disregard(r.as_reason()))
            }
            Some(FRes::Null) | None => {}
        }
        if Self::iter_of(ns, n).is_none() {
            *result = Some(FRes::Ok);
        }
    }

    /// `LogicalXORCompoundFilter.next`.
    fn xor_next(
        &self,
        ctx: &PolicyCtx,
        ns: &mut [NodeState],
        n: usize,
        children: &[usize],
        x: &mut Xor,
    ) {
        if x.continued == 0 {
            x.continued += 1;
            let e = x.event;
            self.xor_step(ctx, ns, n, children, e.as_ref(), x);
        } else {
            x.result = Some(FRes::Disregard(Reason::XorMore));
            x.init();
        }
    }

    /// `super.next(eventToProcess)` of the XOR filter, with its callbacks.
    fn xor_step(
        &self,
        ctx: &PolicyCtx,
        ns: &mut [NodeState],
        n: usize,
        children: &[usize],
        ev: Option<&Ev>,
        x: &mut Xor,
    ) {
        match self.chain_step(ctx, ns, n, children, ev) {
            Some(FRes::Ok) => self.xor_next(ctx, ns, n, children, x),
            Some(r @ (FRes::Disregard(_) | FRes::Exception(_))) => {
                self.xor_disregard(ctx, ns, n, children, r.as_reason(), x)
            }
            Some(FRes::Null) | None => {}
        }
    }

    /// `LogicalXORCompoundFilter.disregard`.
    fn xor_disregard(
        &self,
        ctx: &PolicyCtx,
        ns: &mut [NodeState],
        n: usize,
        children: &[usize],
        msg: Reason,
        x: &mut Xor,
    ) {
        x.disregarded += 1;
        if x.disregarded + x.continued < children.len() {
            let e = x.event;
            self.xor_step(ctx, ns, n, children, e.as_ref(), x);
        } else if x.continued == 1 {
            x.result = Some(FRes::Ok);
        } else {
            x.result = Some(FRes::Disregard(Reason::XorNone(Box::new(msg))));
            x.init();
        }
    }
}

/// The transient fields of `LogicalXORCompoundFilter` (reset by each `doProcess`).
struct Xor<'a> {
    result: Option<FRes>,
    continued: usize,
    disregarded: usize,
    /// `eventToProcess` (`null` after `initialize`).
    event: Option<Ev<'a>>,
}

impl Xor<'_> {
    fn init(&mut self) {
        self.event = None;
        self.continued = 0;
        self.disregarded = 0;
    }
}

/// `ThrashingConstraintFilter.retrieveSign`: positive iff the step or percentage is > 0.
fn sign(a: &Adjustment) -> bool {
    match *a {
        Adjustment::Relative { percentage, .. } => percentage > 0,
        Adjustment::Step { step } => step > 0,
        _ => true,
    }
}

/// Builds the trigger tree (`ScalingTriggerInterpreter`, `StimuliInterpreter`): leaves for the
/// supported stimuli; composed triggers as Slingshot combines them (`addFrom`: an AND joins an
/// existing AND filter of its first child, OR and XOR nest pairwise) with `compat`, as one
/// n-ary node otherwise. Collects the time events of `SimulationTime` leaves.
fn build_trigger(
    t: &Trigger,
    group: &TargetGroup,
    compat: bool,
    nodes: &mut Vec<Node>,
    times: &mut Vec<f64>,
) -> Result<usize, String> {
    match &t.kind {
        TriggerKind::Trend => Err(format!(
            "trigger {}: SimpleFireOnTrend is not supported",
            t.id
        )),
        TriggerKind::Value {
            op,
            stimulus,
            expected,
        } => {
            let want = |cls: &str, ok: bool| {
                if ok {
                    Ok(())
                } else {
                    Err(format!(
                        "trigger {}: It is only possible that the trigger is of type {cls}, but the given type was {}",
                        t.id,
                        expected.class_name()
                    ))
                }
            };
            let check = match stimulus {
                Stimulus::SimulationTime => {
                    let Expected::Time(v) = expected else {
                        return want("ExpectedTime", false).map(|_| 0);
                    };
                    times.push(*v);
                    Check::Time
                }
                Stimulus::OperationResponseTime {
                    signature,
                    signature_name,
                } => {
                    want("ExpectedTime", matches!(expected, Expected::Time(_)))?;
                    Check::ResponseTime {
                        signature: *signature,
                        signature_name: signature_name.clone(),
                    }
                }
                Stimulus::CpuUtilization { aggregation } => {
                    let Expected::Percentage(p) = expected else {
                        return want("ExpectedPercentage", false).map(|_| 0);
                    };
                    if !(0.0..=100.0).contains(p) {
                        return Err(format!(
                            "trigger {}: The expected percentage must be between 0 and 100",
                            t.id
                        ));
                    }
                    Check::Cpu {
                        aggregation: *aggregation,
                    }
                }
                Stimulus::TaskCount { aggregation } => {
                    want("ExpectedCount", matches!(expected, Expected::Count(_)))?;
                    Check::TaskCount {
                        aggregation: *aggregation,
                    }
                }
                Stimulus::QueueLength { passive_resource } => {
                    if !matches!(group.kind, GroupKind::CompetingConsumers { .. }) {
                        return Err(format!(
                            "trigger {}: The QueueLength trigger is only for CompetingConsumersGroup",
                            t.id
                        ));
                    }
                    Check::Queue {
                        passive_resource: *passive_resource,
                    }
                }
                Stimulus::Unsupported(c) => {
                    return Err(format!("trigger {}: stimulus {c} is not supported", t.id));
                }
                Stimulus::Missing => {
                    return Err(format!("trigger {}: no stimulus", t.id));
                }
            };
            nodes.push(Node::Leaf {
                trigger_id: t.id.clone(),
                op: *op,
                check,
                expected: *expected,
            });
            Ok(nodes.len() - 1)
        }
        TriggerKind::Composed { op, children } => {
            if children.is_empty() {
                return Err(format!(
                    "trigger {}: composed trigger without children",
                    t.id
                ));
            }
            if !compat {
                let mut ids = Vec::new();
                for c in children {
                    ids.push(build_trigger(c, group, compat, nodes, times)?);
                }
                if ids.len() == 1 {
                    return Ok(ids[0]);
                }
                nodes.push(Node::Compound {
                    op: *op,
                    children: ids,
                });
                return Ok(nodes.len() - 1);
            }
            let mut cur: Option<usize> = None;
            for c in children {
                let r = build_trigger(c, group, compat, nodes, times)?;
                cur = Some(match cur {
                    None => r,
                    Some(prev) => {
                        if *op == LogicalOp::And
                            && let Node::Compound {
                                op: LogicalOp::And,
                                children,
                            } = &mut nodes[prev]
                        {
                            children.push(r);
                            prev
                        } else {
                            nodes.push(Node::Compound {
                                op: *op,
                                children: vec![prev, r],
                            });
                            nodes.len() - 1
                        }
                    }
                });
            }
            Ok(cur.expect("children"))
        }
    }
}

//! Scaling decisions: a run that pauses before a policy's adjustment is executed, and resumes
//! with an explicit, ordered list of actions in its place (`docs/guide/spd.md`, "Decisions").
//!
//! With [`SpdRun::set_pause_on_decision`] the driver intercepts a request of a policy's chain
//! when it is dispatched, before its adjustment runs (where Slingshot's MENTOR extension,
//! `SnapshotTriggeringBehavior`, pre-intercepts `ModelAdjustmentRequested`). The request and
//! every other recommendation queued for the same instant form one [`PendingDecision`], in
//! request order; none of them is executed, and the run pauses ([`SpdRun::run_while`] returns
//! `Paused`) between two events. A snapshot holds the decision (section `spd-decision`).
//!
//! [`SpdRun::decide`] resolves it once: the selected actions (policies, by id or name; any
//! number, repeated or mixed, `[]` declines) are executed at once, in order, in place of the
//! recommendations, at the decision's time, each like an enforced policy (past trigger and
//! constraints). The recommendations are dropped: their queued notes dispatch as no-ops. The
//! [`Resolution`] reports the recommendations, the selection and each action's outcome.
//!
//! Policy state changes on the recommendation, as without decisions (Q2, semantics decision of
//! 2026-11-04): the chain that made the request committed the adjustor state (cooldown, scaling
//! counters), the target group's last enactment (thrashing) and the triggers' aggregation slots
//! when it fired, before the pause. Neither the selection nor the outcome of an action changes
//! policy state (as Slingshot's incoming policies, [`crate::Spd::enforce`]); they change the
//! model and with it the target groups' membership. So resuming with exactly the recommended
//! policies continues as the run without decisions does.
//!
//! The HPA controller's rescales (`crate::hpa`) and enforced requests are not intercepted.

use crate::interp::{AdjustOutcome, AdjustmentRequest};
use crate::log::{Log, Record};
use crate::model::Adjustment;
use crate::resume::EnforceError;
use crate::sim::{Adjuster, SpdRun};
use serde_json::{Value, json};
use simoxide_sim::compat::Compat;
use simoxide_sim::flavour::Flavour;
use simoxide_sim::{SimError, SimErrorKind};
use std::fmt;

/// `format` of the decision file ([`report`]).
pub const FORMAT: &str = "simoxide-spd-decisions";
/// `version` of the decision file ([`report`]).
pub const VERSION: u32 = 1;

/// One recommendation of a decision: an intercepted request of a policy's chain, with the
/// policy's and target group's ids and the group's size at the decision.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct Recommendation {
    pub request: AdjustmentRequest,
    /// The policy's id (`.spd`) and name.
    pub policy: Box<str>,
    pub policy_name: Box<str>,
    /// The target group's id (`.spd`).
    pub target_group: Box<str>,
    /// The target group's element count at the decision, if known.
    pub group_size: Option<usize>,
}

/// A decision waiting for its actions ([`SpdRun::decide`]).
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct PendingDecision {
    /// Its number in the run (1, 2, ...; counted across snapshots and resumes).
    pub decision: u64,
    /// The simulation time of the decision (ns): the requests' time.
    pub time_ns: i64,
    /// The recommendations, in request order.
    pub recommended: Vec<Recommendation>,
    /// Tags of external notes dispatched with the intercepted one and not handled yet (handled
    /// after the actions; empty in practice: the driver handles one note per event).
    pub(crate) deferred: Vec<u32>,
}

/// What an executed action did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActionOutcome {
    /// The policy's id and name, its target group's id.
    pub policy: Box<str>,
    pub policy_name: Box<str>,
    pub target_group: Box<str>,
    /// The adjuster's outcome: success, the group's sizes before and after (a step the size
    /// constraint clamps to nothing leaves them equal), Slingshot's change count.
    pub ok: bool,
    pub size_before: Option<usize>,
    pub size_after: Option<usize>,
    pub changes: Option<usize>,
}

impl ActionOutcome {
    fn new(r: &Recommendation, out: &AdjustOutcome) -> ActionOutcome {
        ActionOutcome {
            policy: r.policy.clone(),
            policy_name: r.policy_name.clone(),
            target_group: r.target_group.clone(),
            ok: out.ok,
            size_before: out.size_before,
            size_after: out.size_after,
            changes: out.changes,
        }
    }
}

/// A resolved decision: the recommendations, the selected actions (policy ids, in order) and
/// what each action did.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolution {
    pub decision: PendingDecision,
    pub selected: Vec<Box<str>>,
    pub outcomes: Vec<ActionOutcome>,
}

/// The decision state of a run: the decisions made so far, the pending one.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Decisions {
    pub made: u64,
    pub pending: Option<PendingDecision>,
    /// Request notes still queued whose request a decision took (they dispatch as no-ops; the
    /// request table is not reset before they have).
    pub cancelled: u32,
}

/// Why a decision could not be resolved. The run is unchanged, except for [`DecisionError::Sim`]
/// (an action's adjustment failed the run).
#[derive(Clone, Debug)]
pub enum DecisionError {
    /// No decision is pending.
    NoPendingDecision,
    /// An action names no policy of the SPD, or one without a target group.
    Policy(EnforceError),
    /// The simulation failed.
    Sim(SimError),
}

impl fmt::Display for DecisionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecisionError::NoPendingDecision => f.write_str("no scaling decision is pending"),
            DecisionError::Policy(e) => e.fmt(f),
            DecisionError::Sim(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for DecisionError {}

fn secs(ns: i64) -> f64 {
    ns as f64 / 1e9
}

/// An adjustment as JSON: its metamodel class and attributes.
pub(crate) fn adjustment_json(a: &Adjustment) -> Value {
    match *a {
        Adjustment::Step { step } => json!({"type": "StepAdjustment", "stepValue": step}),
        Adjustment::Relative { percentage, min } => json!({
            "type": "RelativeAdjustment",
            "percentageGrowthValue": percentage,
            "minAdjustmentValue": min,
        }),
        Adjustment::Absolute { goal } => json!({"type": "AbsoluteAdjustment", "goalValue": goal}),
        Adjustment::Missing => Value::Null,
    }
}

impl Recommendation {
    pub fn to_json(&self) -> Value {
        json!({
            "policy": &*self.policy,
            "policy_name": &*self.policy_name,
            "target_group": &*self.target_group,
            "group_size": self.group_size,
            "adjustment": adjustment_json(&self.request.adjustment),
        })
    }
}

impl PendingDecision {
    /// The decision as JSON (`docs/guide/spd.md`, "Decision file").
    pub fn to_json(&self) -> Value {
        json!({
            "decision": self.decision,
            "time": secs(self.time_ns),
            "time_ns": self.time_ns,
            "recommended": self.recommended.iter().map(Recommendation::to_json).collect::<Vec<_>>(),
        })
    }

    /// The recommended policies' ids, in order.
    pub fn policies(&self) -> Vec<&str> {
        self.recommended.iter().map(|r| &*r.policy).collect()
    }
}

impl ActionOutcome {
    pub fn to_json(&self) -> Value {
        json!({
            "policy": &*self.policy,
            "policy_name": &*self.policy_name,
            "target_group": &*self.target_group,
            "ok": self.ok,
            "size_before": self.size_before,
            "size_after": self.size_after,
            "changes": self.changes,
        })
    }
}

impl Resolution {
    /// The resolution as JSON: the decision's fields, `selected` and `outcomes`.
    pub fn to_json(&self) -> Value {
        let mut v = self.decision.to_json();
        v["selected"] = json!(self.selected.iter().map(|s| &**s).collect::<Vec<_>>());
        v["outcomes"] = Value::Array(self.outcomes.iter().map(ActionOutcome::to_json).collect());
        v
    }
}

/// The decision file of one invocation: the decision it resolved and the one it paused at.
pub fn report(resolved: Option<&Resolution>, pending: Option<&PendingDecision>) -> Value {
    json!({
        "format": FORMAT,
        "version": VERSION,
        "resolved": resolved.map(Resolution::to_json),
        "pending": pending.map(PendingDecision::to_json),
    })
}

/// Parses an action list: a JSON array of strings (`[]`, `["A","A"]`) or policies separated by
/// commas (`A,A`; an empty string or `[]` is the empty list).
pub fn parse_actions(s: &str) -> Result<Vec<String>, String> {
    let t = s.trim();
    if t.starts_with('[') {
        let v: Vec<String> = serde_json::from_str(t)
            .map_err(|e| format!("expected a JSON array of policy ids or names: {e}"))?;
        return Ok(v);
    }
    let v: Vec<String> = t.split(',').map(|x| x.trim().to_string()).collect();
    if v.iter().all(String::is_empty) {
        return Ok(Vec::new());
    }
    if v.iter().any(String::is_empty) {
        return Err(format!("an empty policy name in {s:?}"));
    }
    Ok(v)
}

impl<C: Compat, F: Flavour> SpdRun<'_, '_, C, F> {
    /// From now on, pause when a policy's request is about to be executed (see the module
    /// documentation). Not part of the run state: a forked or restored run does not pause
    /// unless this is set again.
    pub fn set_pause_on_decision(&mut self, on: bool) {
        self.pause_on_decision = on;
    }

    /// The decision the run paused at, if it has not been resolved.
    pub fn pending_decision(&self) -> Option<&PendingDecision> {
        self.decisions.pending.as_ref()
    }

    /// The number of decisions the run has made (pending or resolved).
    pub fn decisions_made(&self) -> u64 {
        self.decisions.made
    }

    /// Takes the request about to be executed (`first`, just dispatched) and the other queued
    /// recommendations as the pending decision.
    pub(crate) fn intercept(
        &mut self,
        first: AdjustmentRequest,
        deferred: Vec<u32>,
        log: &mut dyn Log,
    ) {
        let mut reqs = vec![first];
        for slot in &mut self.requests {
            if slot
                .as_ref()
                .is_some_and(AdjustmentRequest::is_recommendation)
            {
                reqs.extend(slot.take());
                self.decisions.cancelled += 1;
            }
        }
        let model = self.spd.model();
        let recommended: Vec<Recommendation> = reqs
            .into_iter()
            .map(|request| {
                let p = &model.policies[request.policy];
                let g = model.groups.get(request.group);
                Recommendation {
                    policy: p.id.clone(),
                    policy_name: p.name.clone(),
                    target_group: g.map(|g| g.id.clone()).unwrap_or_default(),
                    group_size: g.and_then(|g| self.state.membership.group_size(g)),
                    request,
                }
            })
            .collect();
        self.decisions.made += 1;
        let d = PendingDecision {
            decision: self.decisions.made,
            time_ns: self.sim.state().now_ns(),
            recommended,
            deferred,
        };
        if log.enabled() {
            log.record(Record::Decision {
                t: secs(d.time_ns),
                decision: d.decision,
                recommended: d.recommended.iter().map(|r| r.policy.clone()).collect(),
            });
        }
        self.decisions.pending = Some(d);
        self.compact_requests();
    }

    /// Resolves the pending decision with `actions` (policies by id or name, in order; `[]`
    /// declines): each action's adjustment is executed now, in place of the recommendations,
    /// by `adjuster` (see the module documentation). On an error before the first action
    /// (no decision pending, an unknown policy) nothing changes.
    pub fn decide(
        &mut self,
        adjuster: &mut dyn Adjuster<C, F>,
        log: &mut dyn Log,
        actions: &[&str],
    ) -> Result<Resolution, DecisionError> {
        if self.decisions.pending.is_none() {
            return Err(DecisionError::NoPendingDecision);
        }
        let idx = self
            .policy_indices(actions)
            .map_err(DecisionError::Policy)?;
        let d = self.decisions.pending.take().expect("pending");
        let model = self.spd.model();
        let selected: Vec<Box<str>> = idx.iter().map(|&i| model.policies[i].id.clone()).collect();
        let now_ns = self.sim.state().now_ns();
        if log.enabled() {
            log.record(Record::Select {
                t: secs(now_ns),
                decision: d.decision,
                selected: selected.clone(),
            });
        }
        let id = d.recommended.first().map_or(0, |r| r.request.id);
        let mut outcomes = Vec::with_capacity(idx.len());
        for &i in &idx {
            let req = self.spd.selected(i, secs(now_ns), d.decision, id);
            let p = &self.spd.model().policies[i];
            let g = self.spd.model().groups.get(req.group);
            let r = Recommendation {
                policy: p.id.clone(),
                policy_name: p.name.clone(),
                target_group: g.map(|g| g.id.clone()).unwrap_or_default(),
                group_size: None,
                request: req.clone(),
            };
            let out = self
                .execute(adjuster, log, &req)
                .map_err(DecisionError::Sim)?;
            outcomes.push(ActionOutcome::new(&r, &out));
        }
        if !d.deferred.is_empty() {
            self.handle_tags(d.deferred.clone(), adjuster, log, false)
                .map_err(DecisionError::Sim)?;
        }
        Ok(Resolution {
            decision: d,
            selected,
            outcomes,
        })
    }

    /// The error of a run continued with its decision pending.
    pub(crate) fn pending_error(&self) -> SimError {
        let d = self.decisions.pending.as_ref().expect("pending");
        SimError {
            message: format!(
                "scaling decision {} is pending (recommended: {}): resolve it before the run continues",
                d.decision,
                d.policies().join(", ")
            ),
            at_ns: self.sim.state().now_ns(),
            kind: SimErrorKind::Model,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_actions;

    #[test]
    fn action_lists() {
        assert_eq!(parse_actions("[]").unwrap(), Vec::<String>::new());
        assert_eq!(parse_actions("").unwrap(), Vec::<String>::new());
        assert_eq!(parse_actions(" a , a ").unwrap(), ["a", "a"]);
        assert_eq!(parse_actions(r#"["_a", "x, y"]"#).unwrap(), ["_a", "x, y"]);
        assert!(parse_actions("a,,b").is_err());
        assert!(parse_actions("[1]").is_err());
    }
}

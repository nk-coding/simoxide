//! The HPA controller inside an [`SpdRun`] (`docs/spec/hpa.md` HPA-1, HPA-4, HPA-6, HPA-7).
//!
//! Per autoscaler three kinds of external notes: the **sync** (every sync period, from the first
//! sync on), the **window start** one metric window before each sync (it reads the busy-time
//! integrals of the pods' CPUs), and **ready** notes when starting pods become ready. A rescale
//! queues an adjustment request (an absolute adjustment of the group to the new ready count) at
//! the current time, as SPD policies do; the run's adjuster executes it.

use super::controller::{self, Pod, PodState};
use super::{Autoscaler, Hpa};
use crate::interp::{AdjustOutcome, AdjustmentRequest};
use crate::log::{HpaRecord, Log, Record};
use crate::model::{Adjustment, GroupCfg};
use crate::sim::{REQUEST, SpdRun};
use simoxide_model::raw::ObjId;
use simoxide_sim::compat::Compat;
use simoxide_sim::flavour::Flavour;
use simoxide_sim::ir::ResKind;
use simoxide_sim::{CompiledModel, SimError};

/// Tags of the HPA's notes: `HPA | kind << 16 | autoscaler`.
pub(crate) const HPA: u32 = 1 << 30;
const SYNC: u32 = 0;
const READY: u32 = 1;
const WINDOW: u32 = 2;

fn tag(kind: u32, i: usize) -> u32 {
    HPA | (kind << 16) | i as u32
}

/// A ready pod: an element of the target group (a container, or an assembly).
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PodInfo {
    pub element: u32,
    /// Created (`status.startTime`) and ready (the Ready condition's transition), ns.
    pub start_ns: i64,
    pub ready_ns: i64,
}

/// The run state of one autoscaler: plain data, part of forks and snapshots.
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, PartialEq)]
pub struct AutoscalerState {
    /// The controller's view of the spec (a change at a resume may change it).
    pub spec: controller::Spec,
    pub startup_delay_ns: i64,
    pub memory: controller::HpaMemory,
    /// Starting pods.
    pub replicas: controller::Replicas,
    /// Ready pods (the group's elements, in the configuration's order).
    pub pods: Vec<PodInfo>,
    /// Busy-time integrals at the window start: (resource, integral).
    pub readings: Vec<(u32, f64)>,
    /// The ready count of the last queued, not yet executed adjustment, and how many are queued.
    pub queued_goal: Option<i32>,
    pub queued: u32,
    pub syncs: u64,
    pub rescales: u64,
    pub adjustments: u64,
    pub failed: u64,
    /// Pods created by adjustments (new elements of the group).
    pub created: u64,
    /// The most replicas (ready and starting) so far.
    pub max_replicas: i32,
    /// Replica-seconds of ready pods and of all pods (ready and starting) so far, accounted up
    /// to `account_ns` with the counts valid since then.
    pub ready_seconds: f64,
    pub replica_seconds: f64,
    pub account_ns: i64,
    pub account_ready: i32,
    pub account_total: i32,
}

/// The run state of the HPA (`SpdRun::hpa_state`).
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HpaState {
    pub autoscalers: Vec<AutoscalerState>,
}

/// What an autoscaler did in a run (for objectives: cost as replica-seconds, scaling activity).
#[derive(Clone, Debug, PartialEq)]
pub struct HpaSummary {
    pub name: String,
    pub syncs: u64,
    pub rescales: u64,
    pub adjustments: u64,
    pub failed: u64,
    pub created: u64,
    /// Ready and starting pods now.
    pub ready: i32,
    pub starting: i32,
    pub max_replicas: i32,
    /// ∫ ready pods dt and ∫ (ready + starting) pods dt, s.
    pub ready_replica_seconds: f64,
    pub replica_seconds: f64,
}

impl HpaSummary {
    /// One line for the command line's summary.
    pub fn line(&self) -> String {
        format!(
            "hpa {}: syncs={} rescales={} adjusted={} failed={} created={} replicas={} starting={} max={} replica-seconds={:.1} ready-replica-seconds={:.1}",
            self.name,
            self.syncs,
            self.rescales,
            self.adjustments,
            self.failed,
            self.created,
            self.ready,
            self.starting,
            self.max_replicas,
            self.replica_seconds,
            self.ready_replica_seconds
        )
    }
}

/// The CPU resources of container `c` in `cm`: (resource index, instances).
pub(crate) fn cpu_resources(cm: &CompiledModel, c: ObjId) -> Vec<(u32, u32)> {
    let m = &cm.model;
    let mut v = Vec::new();
    for (ri, r) in cm.resources.iter().enumerate() {
        let ResKind::Processing(pid) = r.kind else {
            continue;
        };
        let pr = &m.processing_resources[pid];
        if pr.container.map(|k| m.containers[k].obj) != Some(c) {
            continue;
        }
        if pr
            .resource_type
            .is_some_and(|t| &*m.resource_types[t].name == "CPU")
        {
            v.push((ri as u32, r.instances.max(1)));
        }
    }
    v
}

fn container_of(st: &crate::SpdState, a: &Autoscaler, element: ObjId) -> Option<ObjId> {
    if a.containers {
        Some(element)
    } else {
        st.membership
            .allocation
            .iter()
            .find(|(ac, _)| *ac == element)
            .map(|&(_, c)| c)
    }
}

fn account(s: &mut AutoscalerState, now: i64) {
    let dt = (now - s.account_ns).max(0) as f64 / 1e9;
    s.ready_seconds += f64::from(s.account_ready) * dt;
    s.replica_seconds += f64::from(s.account_total) * dt;
    s.account_ns = now;
    s.account_ready = s.pods.len() as i32;
    s.account_total = s.account_ready + s.replicas.pending.len() as i32;
    s.max_replicas = s.max_replicas.max(s.account_total);
}

fn sim_err(message: String, at_ns: i64) -> SimError {
    SimError {
        message,
        at_ns,
        kind: simoxide_sim::SimErrorKind::Model,
    }
}

impl<'m, 's, C: Compat, F: Flavour> SpdRun<'m, 's, C, F> {
    /// Runs the autoscalers of `hpa` with this run (built on a model [`super::attach`]ed with
    /// their configuration), from now on: the first syncs and window starts are scheduled, the
    /// group's current elements are ready pods (started long ago), their CPUs tracked.
    pub fn with_hpa(mut self, hpa: &'s Hpa) -> Result<Self, SimError> {
        let now = self.sim.state().now_ns();
        self.hpa = Some(hpa);
        self.hpa_state = HpaState {
            autoscalers: hpa
                .autoscalers
                .iter()
                .map(|a| AutoscalerState {
                    spec: a.cfg.spec.clone(),
                    startup_delay_ns: a.cfg.startup_delay_ns,
                    memory: Default::default(),
                    replicas: Default::default(),
                    pods: Vec::new(),
                    readings: Vec::new(),
                    queued_goal: None,
                    queued: 0,
                    syncs: 0,
                    rescales: 0,
                    adjustments: 0,
                    failed: 0,
                    created: 0,
                    max_replicas: 0,
                    ready_seconds: 0.0,
                    replica_seconds: 0.0,
                    account_ns: now,
                    account_ready: 0,
                    account_total: 0,
                })
                .collect(),
        };
        for i in 0..hpa.autoscalers.len() {
            // the initial pods: ready since long before the start
            self.hpa_refresh(i, i64::MIN / 4, i64::MIN / 4);
            let a = &hpa.autoscalers[i].cfg;
            let first = now + a.first_sync_ns;
            self.sim
                .schedule_external(first - a.metric_window_ns, tag(WINDOW, i))?;
            self.sim.schedule_external(first, tag(SYNC, i))?;
        }
        Ok(self)
    }

    /// The autoscalers' run state.
    pub fn hpa_state(&self) -> &HpaState {
        &self.hpa_state
    }

    /// The autoscalers' activity so far (replica-seconds up to now).
    pub fn hpa_summary(&self) -> Vec<HpaSummary> {
        let Some(hpa) = self.hpa else {
            return Vec::new();
        };
        let now = self.sim.state().now_ns();
        hpa.autoscalers
            .iter()
            .zip(&self.hpa_state.autoscalers)
            .map(|(a, s)| {
                let mut s = s.clone();
                account(&mut s, now);
                HpaSummary {
                    name: a.cfg.name.clone(),
                    syncs: s.syncs,
                    rescales: s.rescales,
                    adjustments: s.adjustments,
                    failed: s.failed,
                    created: s.created,
                    ready: s.pods.len() as i32,
                    starting: s.replicas.pending.len() as i32,
                    max_replicas: s.max_replicas,
                    ready_replica_seconds: s.ready_seconds,
                    replica_seconds: s.replica_seconds,
                }
            })
            .collect()
    }

    /// Changes an autoscaler's parameters (at a resume: `simoxide resume --hpa-set`):
    /// `minReplicas`, `maxReplicas`, `averageUtilization`, `tolerance`, `startupDelay` (s), the
    /// stabilisation windows `scaleUpStabilization`, `scaleDownStabilization` (s; with a
    /// `behavior`), `downscaleStabilization` (s; without one).
    pub fn hpa_set(&mut self, name: &str, key: &str, value: f64) -> Result<(), String> {
        let hpa = self.hpa.ok_or("the run has no HPA")?;
        let i = hpa
            .index_of(name)
            .ok_or_else(|| format!("no autoscaler named {name:?}"))?;
        let s = &mut self.hpa_state.autoscalers[i];
        let int = || -> Result<i32, String> {
            if value.fract() != 0.0 || !(1.0..=1e9).contains(&value) {
                return Err(format!("{key}: a positive integer expected, got {value}"));
            }
            Ok(value as i32)
        };
        let secs = || -> Result<i64, String> {
            if !(value >= 0.0 && value.is_finite()) {
                return Err(format!(
                    "{key}: a duration in seconds expected, got {value}"
                ));
            }
            Ok((value * 1e9).round() as i64)
        };
        let mut spec = s.spec.clone();
        match key {
            "minReplicas" => spec.min_replicas = int()?,
            "maxReplicas" => spec.max_replicas = int()?,
            "averageUtilization" => spec.target_utilization = int()?,
            "tolerance" if value >= 0.0 => spec.tolerance = value,
            "startupDelay" => {
                s.startup_delay_ns = secs()?;
                return Ok(());
            }
            "scaleUpStabilization" | "scaleDownStabilization" => {
                let b = spec
                    .behavior
                    .as_mut()
                    .ok_or_else(|| format!("{key}: the autoscaler has no behavior"))?;
                let r = if key == "scaleUpStabilization" {
                    &mut b.up
                } else {
                    &mut b.down
                };
                r.stabilization_ns = secs()?;
            }
            "downscaleStabilization" => spec.downscale_stabilization_ns = secs()?,
            _ => return Err(format!("unknown or invalid HPA parameter {key}={value}")),
        }
        if spec.max_replicas < spec.min_replicas {
            return Err(format!("{name}: maxReplicas below minReplicas"));
        }
        s.spec = spec;
        Ok(())
    }

    /// A note of the HPA was dispatched.
    pub(crate) fn hpa_note(&mut self, t: u32, log: &mut dyn Log) -> Result<(), SimError> {
        let i = (t & 0xffff) as usize;
        match (t & !HPA) >> 16 {
            SYNC => self.hpa_sync(i, log),
            READY => self.hpa_ready(i, log),
            _ => {
                self.hpa_window(i);
                Ok(())
            }
        }
    }

    /// Re-reads autoscaler `i`'s pods from the membership: elements that joined are ready pods
    /// created at `start` and ready at `ready`; their CPUs are tracked.
    fn hpa_refresh(&mut self, i: usize, start: i64, ready: i64) {
        let Some(hpa) = self.hpa else {
            return;
        };
        let a = &hpa.autoscalers[i];
        let group = &self.spd.model().groups[a.group];
        let elements: Vec<ObjId> = match self.state.membership.cfg_of(group) {
            Some(
                GroupCfg::ElasticInfrastructure { elements, .. }
                | GroupCfg::ServiceGroup { elements, .. }
                | GroupCfg::CompetingConsumers { elements, .. },
            ) => elements.clone(),
            None => Vec::new(),
        };
        let now = self.sim.state().now_ns();
        let s = &mut self.hpa_state.autoscalers[i];
        let pods: Vec<PodInfo> = elements
            .iter()
            .map(|e| {
                s.pods
                    .iter()
                    .find(|p| p.element == e.0)
                    .copied()
                    .unwrap_or(PodInfo {
                        element: e.0,
                        start_ns: start,
                        ready_ns: ready,
                    })
            })
            .collect();
        if pods != s.pods {
            s.pods = pods;
        }
        account(s, now);
        let mut track = Vec::new();
        let cm = self.sim.compiled();
        for e in &elements {
            if let Some(c) = container_of(&self.state, a, *e) {
                track.extend(cpu_resources(cm, c).into_iter().map(|(r, _)| r));
            }
        }
        for r in track {
            self.sim.track_busy(r);
        }
    }

    /// The window start: reads the pods' busy-time integrals.
    fn hpa_window(&mut self, i: usize) {
        let Some(hpa) = self.hpa else {
            return;
        };
        let a = &hpa.autoscalers[i];
        let cm = self.sim.compiled();
        let mut readings = Vec::new();
        for p in &self.hpa_state.autoscalers[i].pods {
            if let Some(c) = container_of(&self.state, a, ObjId(p.element)) {
                for (r, _) in cpu_resources(cm, c) {
                    if let Some((_, v)) = self.sim.busy_integral(r) {
                        readings.push((r, v));
                    }
                }
            }
        }
        self.hpa_state.autoscalers[i].readings = readings;
    }

    /// The pods' samples at a sync (HPA-4): ready pods with their CPU usage over the metric
    /// window, unready ones, starting ones.
    fn hpa_samples(&self, i: usize, now: i64) -> Vec<Pod> {
        let hpa = self.hpa.expect("hpa");
        let a = &hpa.autoscalers[i];
        let s = &self.hpa_state.autoscalers[i];
        let cm = self.sim.compiled();
        let w = a.cfg.metric_window_ns;
        let now_s = now as f64 / 1e9;
        let mut pods = Vec::with_capacity(s.pods.len() + s.replicas.pending.len());
        let mut default_request = None;
        for p in &s.pods {
            let res = container_of(&self.state, a, ObjId(p.element))
                .map(|c| cpu_resources(cm, c))
                .unwrap_or_default();
            let cores: u32 = res.iter().map(|r| r.1).sum();
            let request = a.cfg.cpu_request.unwrap_or(f64::from(cores.max(1)));
            let request_milli = (request * 1000.0).round() as i64;
            default_request.get_or_insert(request_milli);
            // groupPods: inside the CPU initialisation period a pod whose metric window started
            // before it became ready is unready
            let in_init = now
                < p.start_ns
                    .saturating_add(a.cfg.cpu_initialization_period_ns);
            let state = if in_init && p.ready_ns > now - w {
                PodState::Unready
            } else if res.is_empty() {
                PodState::Missing
            } else {
                let mut cores_used = 0.0;
                let mut ok = true;
                for (r, inst) in &res {
                    let Some((since, v)) = self.sim.busy_integral(*r) else {
                        ok = false;
                        break;
                    };
                    let (from, span) = match s.readings.iter().find(|x| x.0 == *r) {
                        Some(&(_, x)) => (x, w as f64 / 1e9),
                        None => (0.0, now_s - since),
                    };
                    if span <= 0.0 {
                        ok = false;
                        break;
                    }
                    cores_used += f64::from(*inst) * (v - from).max(0.0) / span;
                }
                if ok {
                    // metrics-server: nanocores; the HPA reads `MilliValue()` (rounded up)
                    let nano = (cores_used * 1e9).round() as i64;
                    PodState::Ready((nano + 999_999).div_euclid(1_000_000))
                } else {
                    PodState::Missing
                }
            };
            pods.push(Pod {
                request_milli,
                state,
            });
        }
        let request_milli = default_request
            .unwrap_or_else(|| (a.cfg.cpu_request.unwrap_or(1.0) * 1000.0).round() as i64);
        for _ in &s.replicas.pending {
            pods.push(Pod {
                request_milli,
                state: PodState::Unready,
            });
        }
        pods
    }

    fn hpa_sync(&mut self, i: usize, log: &mut dyn Log) -> Result<(), SimError> {
        let hpa = self.hpa.expect("hpa");
        let a = &hpa.autoscalers[i];
        let now = self.sim.state().now_ns();
        self.hpa_refresh(i, now, now);
        let pods = self.hpa_samples(i, now);
        let s = &mut self.hpa_state.autoscalers[i];
        s.syncs += 1;
        let ready = s.pods.len() as i32;
        let base = s.queued_goal.unwrap_or(ready);
        let starting = s.replicas.pending.len() as i32;
        let current = base + starting;
        let d = controller::reconcile(&s.spec, &mut s.memory, now, current, &pods);
        let mut plan = controller::Plan {
            started: 0,
            cancelled: 0,
            goal: None,
        };
        if d.rescale() {
            s.rescales += 1;
            plan = s.replicas.rescale(base, d.desired, now, s.startup_delay_ns);
            account(s, now);
        }
        let delay = s.startup_delay_ns;
        if log.enabled() {
            let c = d.calc;
            log.record(Record::Hpa(Box::new(HpaRecord {
                t: now as f64 / 1e9,
                hpa: a.cfg.name.as_str().into(),
                current,
                ready,
                starting,
                unready: c.map_or(0, |c| c.unready as i32),
                missing: c.map_or(0, |c| c.missing as i32),
                utilization: c.map(|c| c.utilization),
                target: s.spec.target_utilization,
                ratio: c.map(|c| c.ratio),
                recommended: c.map(|c| c.replicas),
                stabilized: d.stabilized,
                desired: d.desired,
                reason: d.reason,
                error: d.error.clone(),
                started: plan.started,
                cancelled: plan.cancelled,
                goal: plan.goal,
            })));
        }
        if let Some(goal) = plan.goal {
            self.hpa_request(i, goal)?;
        }
        if plan.started > 0 && delay > 0 {
            self.sim.schedule_external(now + delay, tag(READY, i))?;
        }
        let period = a.cfg.sync_period_ns;
        let next = now + period;
        self.sim
            .schedule_external(next - a.cfg.metric_window_ns, tag(WINDOW, i))?;
        self.sim.schedule_external(next, tag(SYNC, i))?;
        Ok(())
    }

    fn hpa_ready(&mut self, i: usize, log: &mut dyn Log) -> Result<(), SimError> {
        let hpa = self.hpa.expect("hpa");
        let now = self.sim.state().now_ns();
        let s = &mut self.hpa_state.autoscalers[i];
        let n = s.replicas.due(now);
        if n == 0 {
            return Ok(());
        }
        let goal = s.queued_goal.unwrap_or(s.pods.len() as i32) + n;
        account(s, now);
        if log.enabled() {
            log.record(Record::HpaReady {
                t: now as f64 / 1e9,
                hpa: hpa.autoscalers[i].cfg.name.as_str().into(),
                ready: n,
                goal,
            });
        }
        self.hpa_request(i, goal)
    }

    /// Queues the absolute adjustment of autoscaler `i`'s group to `goal` ready pods.
    fn hpa_request(&mut self, i: usize, goal: i32) -> Result<(), SimError> {
        let hpa = self.hpa.expect("hpa");
        let a = &hpa.autoscalers[i];
        let now = self.sim.state().now_ns();
        let req = AdjustmentRequest {
            id: self.state.next_request,
            policy: a.policy,
            ctx: usize::MAX,
            group: a.group,
            adjustment: Adjustment::Absolute { goal: goal.into() },
            time: now as f64 / 1e9,
            enforced: false,
            hpa: Some(i as u32),
        };
        self.state.next_request += 1;
        let s = &mut self.hpa_state.autoscalers[i];
        s.queued_goal = Some(goal);
        s.queued += 1;
        let t = REQUEST | self.requests.len() as u32;
        self.requests.push(Some(req));
        self.sim.schedule_external(now, t)
    }

    /// An HPA adjustment was executed: the new elements are ready pods (created a start-up
    /// delay ago).
    pub(crate) fn hpa_adjusted(&mut self, i: usize, out: &AdjustOutcome) -> Result<(), SimError> {
        let now = self.sim.state().now_ns();
        let s = &mut self.hpa_state.autoscalers[i];
        s.queued = s.queued.saturating_sub(1);
        if s.queued == 0 {
            s.queued_goal = None;
        }
        if out.ok {
            s.adjustments += 1;
        } else {
            s.failed += 1;
        }
        let delay = s.startup_delay_ns;
        let known: Vec<u32> = s.pods.iter().map(|p| p.element).collect();
        self.hpa_refresh(i, now - delay, now);
        let s = &mut self.hpa_state.autoscalers[i];
        let new = s
            .pods
            .iter()
            .filter(|p| !known.contains(&p.element))
            .count();
        s.created += new as u64;
        let hpa = self.hpa.expect("hpa");
        let created: u64 = self.hpa_state.autoscalers.iter().map(|s| s.created).sum();
        let limit = hpa.autoscalers[i].cfg.max_created_pods;
        if created > limit {
            return Err(sim_err(
                format!(
                    "HPA: {created} pods created, more than controller.maxCreatedPods = {limit} (each stays in the run's model; raise the limit if the run needs more)"
                ),
                now,
            ));
        }
        if self.hpa_state.autoscalers[i].pods.is_empty() {
            return Err(sim_err(
                format!(
                    "HPA {}: the target group has no element after an adjustment",
                    self.hpa.expect("hpa").autoscalers[i].cfg.name
                ),
                now,
            ));
        }
        Ok(())
    }

    /// After a model change from outside (a change file at a resume): re-reads the pods.
    pub(crate) fn hpa_model_changed(&mut self) {
        let now = self.sim.state().now_ns();
        for i in 0..self.hpa_state.autoscalers.len() {
            self.hpa_refresh(i, now, now);
        }
    }
}

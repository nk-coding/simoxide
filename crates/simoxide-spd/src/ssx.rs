//! The SPD interpreter in a Slingshot-exact run (`DESIGN-SPD.md` X2d): an implementation of
//! the engine's hook trait [`simoxide_sim::ssx::SpdHooks`].
//!
//! The engine emulates Slingshot's bus (the time events at `PreSimulationConfigurationStarted`,
//! one subscriber per policy and stimulus with its own disposed bit, `ModelAdjustmentRequested`
//! running the adjustment, the `ModelAdjusted` subscribers); this module answers it with the
//! interpreter ([`Spd::subscribers`], [`Spd::on_timer_of`], [`Spd::on_measurement_of`]) and an
//! [`SsxAdjuster`] that changes the run's model through the engine's
//! [`simoxide_sim::ChangeHost`] (`simoxide-reconf`'s `QvtoAdjuster`: Slingshot's
//! `MainTransformation`, QVTo's ordered mode).
//!
//! ```ignore
//! let spd = spd_of(&cm.model, Quirks::slingshot())?.unwrap();
//! let mut adjuster = QvtoAdjuster::spd(&core, &SpdAdjust::new(&spd.quirks()))?;
//! let mut log = VecLog::default();
//! let mut hooks = SsxSpd::new(&spd, &cm.model, &mut adjuster, &mut log);
//! let r = simoxide_sim::ssx::run_with_spd(&cm, cfg, out, &mut hooks)?;
//! ```

use crate::interp::{
    AdjustOutcome, AdjustmentRequest, Measurement, Metric, Spd, SpdHost, SpdState,
};
use crate::log::Log;
use crate::model::{Membership, PointRef};
use crate::sim::{NoAdjustment, point_of_key};
use simoxide_model::Model;
use simoxide_sim::ir::metric_names;
use simoxide_sim::ssx::{
    HandlerError, SpdAdjusted, SpdHooks, SpdMeasurement, SpdRequest, SpdStart, SpdTimeEvent,
    Stimulus,
};
use simoxide_sim::{ChangeHost, SimError};

/// Executes adjustment requests of a Slingshot-exact run on the engine's model (`host`): the
/// counterpart of [`crate::sim::Adjuster`] for [`ChangeHost`]s. It may change the model and
/// must then refresh `state.membership`; its outcome's [`AdjustOutcome::adjusted`] is the
/// `ModelAdjusted` the engine applies.
pub trait SsxAdjuster {
    fn adjust(
        &mut self,
        host: &mut dyn ChangeHost,
        spd: &Spd,
        state: &mut SpdState,
        req: &AdjustmentRequest,
    ) -> Result<AdjustOutcome, SimError>;
}

impl SsxAdjuster for NoAdjustment {
    fn adjust(
        &mut self,
        _: &mut dyn ChangeHost,
        spd: &Spd,
        state: &mut SpdState,
        req: &AdjustmentRequest,
    ) -> Result<AdjustOutcome, SimError> {
        let size = state.membership.group_size(&spd.model().groups[req.group]);
        Ok(AdjustOutcome {
            ok: false,
            size_before: size,
            size_after: size,
            ..AdjustOutcome::default()
        })
    }
}

/// The SPD layer of a Slingshot-exact run (see the module documentation).
pub struct SsxSpd<'s, 'a> {
    spd: &'s Spd,
    /// The interpreter's state (after the run: its final state).
    pub state: SpdState,
    adjuster: &'a mut dyn SsxAdjuster,
    log: &'a mut dyn Log,
    /// Per engine subscriber: the policy context.
    subs: Vec<usize>,
    /// Requests by handle; taken when executed.
    requests: Vec<Option<AdjustmentRequest>>,
    /// Per engine series: its measuring point and metric, once resolved.
    points: Vec<Option<Option<(PointRef, Metric)>>>,
    /// Successful adjustments.
    pub adjusted_ok: u64,
}

/// Collects the requests of one subscriber call.
struct Host {
    now: f64,
    requests: Vec<AdjustmentRequest>,
}

impl SpdHost for Host {
    fn now(&self) -> f64 {
        self.now
    }
    fn schedule(&mut self, _: f64, _: crate::TimerId) {
        // only the set-up schedules time events
    }
    fn request(&mut self, req: AdjustmentRequest) {
        self.requests.push(req);
    }
}

impl<'s, 'a> SsxSpd<'s, 'a> {
    /// The SPD layer for a run of `model` (the target groups' members are read from it).
    pub fn new(
        spd: &'s Spd,
        model: &Model,
        adjuster: &'a mut dyn SsxAdjuster,
        log: &'a mut dyn Log,
    ) -> SsxSpd<'s, 'a> {
        let membership = Membership::from_graph(&model.graph, spd.model());
        SsxSpd {
            spd,
            state: spd.state(membership),
            adjuster,
            log,
            subs: Vec::new(),
            requests: Vec::new(),
            points: Vec::new(),
            adjusted_ok: 0,
        }
    }

    /// Requests not executed when the run ended.
    pub fn pending(&self) -> usize {
        self.requests.iter().filter(|r| r.is_some()).count()
    }

    fn take(&mut self, h: Host) -> Option<SpdRequest> {
        // a policy subscriber returns at most one `ModelAdjustmentRequested`
        let req = h.requests.into_iter().next()?;
        let id = self.requests.len() as u32;
        let policy = self.spd.model().policies[req.policy].id.clone();
        self.requests.push(Some(req));
        Some(SpdRequest { id, policy })
    }

    fn point(&mut self, m: &SpdMeasurement<'_>) -> Option<(PointRef, Metric)> {
        let s = m.series as usize;
        if self.points.len() <= s {
            self.points.resize(s + 1, None);
        }
        *self.points[s].get_or_insert_with(|| {
            let metric = match m.metric {
                metric_names::RESPONSE_TIME => Metric::ResponseTime,
                metric_names::UTILIZATION => Metric::Utilization,
                metric_names::STATE_OF_ACTIVE_RESOURCE => Metric::ActiveResourceState,
                metric_names::STATE_OF_PASSIVE_RESOURCE => Metric::PassiveResourceState,
                _ => return None,
            };
            Some((point_of_key(m.model, m.key), metric))
        })
    }
}

impl SpdHooks for SsxSpd<'_, '_> {
    fn start(&mut self, _: &Model) -> Result<SpdStart, HandlerError> {
        let model = self.spd.model();
        let time_events = self
            .spd
            .timers()
            .iter()
            .map(|t| SpdTimeEvent {
                delay: t.time,
                target_group: model.groups[t.group].id.clone(),
            })
            .collect();
        let subs = self.spd.subscribers();
        let subscribers = subs
            .iter()
            .map(|&(_, time)| {
                if time {
                    Stimulus::Time
                } else {
                    Stimulus::Measurement
                }
            })
            .collect();
        self.subs = subs.into_iter().map(|(c, _)| c).collect();
        Ok(SpdStart {
            time_events,
            subscribers,
        })
    }

    fn on_time(
        &mut self,
        subscriber: usize,
        event: usize,
        now: f64,
    ) -> Result<Option<SpdRequest>, HandlerError> {
        let mut h = Host {
            now,
            requests: Vec::new(),
        };
        let c = self.subs[subscriber];
        self.spd
            .on_timer_of(c, &mut self.state, &mut h, event as u32, self.log);
        Ok(self.take(h))
    }

    fn on_measurement(
        &mut self,
        subscriber: usize,
        m: &SpdMeasurement<'_>,
        now: f64,
    ) -> Result<Option<SpdRequest>, HandlerError> {
        let Some((point, metric)) = self.point(m) else {
            return Ok(None);
        };
        let mm = Measurement {
            point,
            metric,
            point_in_time: m.point_in_time,
            value: m.value,
            source: m.series,
        };
        let mut h = Host {
            now,
            requests: Vec::new(),
        };
        let c = self.subs[subscriber];
        self.spd
            .on_measurement_of(c, &mut self.state, &mut h, &mm, self.log);
        Ok(self.take(h))
    }

    fn adjust(&mut self, request: u32, host: &mut dyn ChangeHost) -> Result<SpdAdjusted, SimError> {
        let Some(req) = self
            .requests
            .get_mut(request as usize)
            .and_then(Option::take)
        else {
            return Ok(SpdAdjusted::default());
        };
        let out = self
            .adjuster
            .adjust(host, self.spd, &mut self.state, &req)?;
        if out.ok {
            self.adjusted_ok += 1;
        }
        let now = host.now_s();
        self.spd
            .adjusted_with(&mut self.state, &req, &out, now, self.log);
        Ok(match &out.adjusted {
            Some(a) => SpdAdjusted {
                ok: a.ok,
                changes: a.changes(),
                new_containers: a.new_containers.clone(),
                deleted_containers: a.deleted_containers.clone(),
                new_allocation_contexts: a.new_allocation_contexts.clone(),
            },
            None => SpdAdjusted {
                ok: out.ok,
                changes: out.changes.unwrap_or(if out.ok { 2 } else { 0 }),
                ..SpdAdjusted::default()
            },
        })
    }
}

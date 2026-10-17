//! Runs the SPD interpreter inside a `simoxide-sim` run (`DESIGN-SPD.md` §5.4).
//!
//! The engine knows nothing about SPD; it offers two generic hooks (`simoxide_sim::sim::hooks`):
//!
//! * **time events** are external notes of the event list: scheduled at set-up (before the
//!   workload's first notes, as Slingshot queues `SimulationTimeReached` at
//!   `PreSimulationConfigurationStarted`), dispatched FIFO among same-time notes. The driver
//!   gets control right after the note's event and calls [`Spd::on_timer`];
//! * **measurements** are observed series: the series whose metric some trigger reads; the
//!   driver gets control after each event that emitted such a tuple and calls
//!   [`Spd::on_measurement`] per tuple, in emission order.
//!
//! An adjustment request is queued as an external note at the current time (behind the notes
//! already queued for now, like `ModelAdjustmentRequested`), and an [`Adjuster`] executes it
//! when it is dispatched. P1 has no adjustment: [`NoAdjustment`] reports every request as not
//! executed.
//!
//! In the SimuLizar flavour the tuples are SimuLizar's (window-aggregated utilisation, not
//! Slingshot's instantaneous samples): a documented deviation without a reference.

use crate::interp::{AdjustmentRequest, Measurement, Metric, Spd, SpdHost, SpdState, TimerId};
use crate::log::Log;
use crate::model::{Membership, PointKind, PointRef, SpdError, SpdModel};
use crate::quirks::Quirks;
use simoxide_model::Model;
use simoxide_model::raw::ObjId;
use simoxide_sim::compat::Compat;
use simoxide_sim::flavour::{Flavour, SimuLizar};
use simoxide_sim::ir::metric_names;
use simoxide_sim::{CompiledModel, Outputs, RunResult, SimError, SimErrorKind, Simulation, Status};

/// Tags of request notes (time events use their timer index).
const REQUEST: u32 = 1 << 31;

/// Builds the interpreter of the SPD in `model` (`None`: the model has no SPD).
pub fn spd_of(model: &Model, quirks: Quirks) -> Result<Option<Spd>, SpdError> {
    let Some(m) = SpdModel::from_graph(&model.graph) else {
        return Ok(None);
    };
    let names = model
        .signatures
        .iter()
        .map(|s| (s.obj, s.name.clone()))
        .collect();
    Ok(Some(Spd::new(m, quirks)?.with_signature_names(names)))
}

/// The outcome of an executed adjustment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdjustOutcome {
    pub ok: bool,
    /// Element counts of the target group before and after, if known.
    pub size_before: Option<usize>,
    pub size_after: Option<usize>,
}

/// Executes adjustment requests (P2: Slingshot's `MainTransformation.qvto` on the run's
/// model, then the syncers). It may change the simulation's model and must then refresh
/// `state.membership`. Generic over the flavour (`F`); [`SpdRun`] drives the SimuLizar
/// flavour. `simoxide-reconf`'s `QvtoAdjuster` (feature `spd`) runs a QVTo transformation.
pub trait Adjuster<C: Compat, F: Flavour = SimuLizar> {
    fn adjust(
        &mut self,
        sim: &mut Simulation<'_, C, F>,
        spd: &Spd,
        state: &mut SpdState,
        req: &AdjustmentRequest,
    ) -> Result<AdjustOutcome, SimError>;
}

/// P1: no adjustment is executed; every request is reported as failed (`ok: false`), so the
/// fixed Q2 commits no constraint state.
pub struct NoAdjustment;

impl<C: Compat, F: Flavour> Adjuster<C, F> for NoAdjustment {
    fn adjust(
        &mut self,
        _: &mut Simulation<'_, C, F>,
        spd: &Spd,
        state: &mut SpdState,
        req: &AdjustmentRequest,
    ) -> Result<AdjustOutcome, SimError> {
        let size = state.membership.group_size(&spd.model().groups[req.group]);
        Ok(AdjustOutcome {
            ok: false,
            size_before: size,
            size_after: size,
        })
    }
}

/// The measuring point and metric of each compiled series, as far as SPD reads them.
pub(crate) fn series_points(cm: &CompiledModel) -> Vec<Option<(PointRef, Metric)>> {
    let m = &cm.model;
    cm.series
        .iter()
        .map(|s| {
            let metric = match s.metric {
                metric_names::RESPONSE_TIME => Metric::ResponseTime,
                metric_names::UTILIZATION => Metric::Utilization,
                metric_names::STATE_OF_ACTIVE_RESOURCE => Metric::ActiveResourceState,
                metric_names::STATE_OF_PASSIVE_RESOURCE => Metric::PassiveResourceState,
                _ => return None,
            };
            Some((point_of_key(m, &s.mp), metric))
        })
        .collect()
}

/// Resolves a measuring-point key of `measurements.csv` (`docs/guide/formats.md` §4).
pub fn point_of_key(m: &Model, key: &str) -> PointRef {
    let (class, args) = match key.split_once('[') {
        Some((c, rest)) => (c, rest.strip_suffix(']').unwrap_or(rest)),
        None => (key, ""),
    };
    let parts: Vec<&str> = args.split('|').collect();
    let other = |class: &'static str| PointRef {
        kind: PointKind::Other,
        class,
    };
    let sig = |id: &str| m.signatures.iter().find(|s| &*s.id == id).map(|s| s.obj);
    match class {
        "ActiveResourceMeasuringPoint" => {
            let container = m
                .processing_resources
                .iter()
                .find(|r| &*r.id == parts[0])
                .and_then(|r| r.container)
                .map(|c| m.containers[c].obj);
            match container {
                Some(container) => PointRef {
                    kind: PointKind::ActiveResource { container },
                    class: "ActiveResourceMeasuringPoint",
                },
                None => other("ActiveResourceMeasuringPoint"),
            }
        }
        "AssemblyOperationMeasuringPoint" | "SystemOperationMeasuringPoint" => {
            let class = if class.starts_with("Assembly") {
                "AssemblyOperationMeasuringPoint"
            } else {
                "SystemOperationMeasuringPoint"
            };
            match parts.get(1).and_then(|id| sig(id)) {
                Some(signature) => PointRef {
                    kind: PointKind::Operation { signature },
                    class,
                },
                None => other(class),
            }
        }
        "ResourceEnvironmentMeasuringPoint" => PointRef {
            kind: PointKind::Environment,
            class: "ResourceEnvironmentMeasuringPoint",
        },
        "UsageScenarioMeasuringPoint" => other("UsageScenarioMeasuringPoint"),
        "EntryLevelSystemCallMeasuringPoint" => other("EntryLevelSystemCallMeasuringPoint"),
        "ExternalCallActionMeasuringPoint" => other("ExternalCallActionMeasuringPoint"),
        _ => other("MeasuringPoint"),
    }
}

fn span(seconds: f64) -> i64 {
    (seconds * 1e9) as i64
}

/// Counters of an SPD run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpdSummary {
    pub evaluations: u64,
    pub requests: u64,
    pub adjustments_ok: u64,
    /// Requests still queued when the run ended.
    pub pending: u64,
}

/// A simulation with an SPD interpreter.
pub struct SpdRun<'m, 's, C: Compat> {
    pub sim: Simulation<'m, C>,
    pub(crate) spd: &'s Spd,
    pub state: SpdState,
    pub(crate) series: Vec<Option<(PointRef, Metric)>>,
    /// Requests by note tag (`REQUEST | index`); taken when dispatched.
    pub(crate) requests: Vec<Option<AdjustmentRequest>>,
    pub(crate) adjusted_ok: u64,
}

struct Host<'a, 'm, C: Compat> {
    sim: &'a mut Simulation<'m, C>,
    requests: &'a mut Vec<Option<AdjustmentRequest>>,
    error: Option<SimError>,
}

impl<C: Compat> SpdHost for Host<'_, '_, C> {
    fn now(&self) -> f64 {
        self.sim.state().now_ns() as f64 / 1e9
    }

    fn schedule(&mut self, at: f64, timer: TimerId) {
        let now = self.sim.state().now_ns();
        let at = span(at).max(now);
        if let Err(e) = self.sim.schedule_external(at, timer) {
            self.error.get_or_insert(e);
        }
    }

    fn request(&mut self, req: AdjustmentRequest) {
        let tag = REQUEST | self.requests.len() as u32;
        self.requests.push(Some(req));
        let now = self.sim.state().now_ns();
        if let Err(e) = self.sim.schedule_external(now, tag) {
            self.error.get_or_insert(e);
        }
    }
}

impl<'m, 's, C: Compat> SpdRun<'m, 's, C> {
    /// Attaches `spd` to `sim` (not started): observes the series its triggers read and
    /// schedules the set-up's time events.
    pub fn new(mut sim: Simulation<'m, C>, spd: &'s Spd) -> Result<Self, SimError> {
        let cm = sim.compiled();
        let membership = Membership::from_graph(&cm.model.graph, spd.model());
        let series = series_points(cm);
        let metrics = spd.metrics();
        for (i, s) in series.iter().enumerate() {
            if let Some((_, m)) = s
                && metrics.contains(m)
            {
                sim.observe_series(i as u32);
            }
        }
        let mut run = SpdRun {
            sim,
            spd,
            state: spd.state(membership),
            series,
            requests: Vec::new(),
            adjusted_ok: 0,
        };
        let mut host = Host {
            sim: &mut run.sim,
            requests: &mut run.requests,
            error: None,
        };
        spd.start(&mut host);
        if let Some(e) = host.error {
            return Err(e);
        }
        Ok(run)
    }

    /// Runs to the end, handling time events, measurements and requests (executed by
    /// `adjuster`).
    pub fn run(
        &mut self,
        adjuster: &mut dyn Adjuster<C>,
        log: &mut dyn Log,
    ) -> Result<(), SimError> {
        self.run_until(adjuster, log, None).map(|_| ())
    }

    /// Runs like [`SpdRun::run`] until the next event is at or after `before_ns` (if given):
    /// every event before it has run, and so has the SPD work they caused. Then the run can be
    /// forked ([`SpdRun::fork`]) or continued.
    pub fn run_until(
        &mut self,
        adjuster: &mut dyn Adjuster<C>,
        log: &mut dyn Log,
        before_ns: Option<i64>,
    ) -> Result<Status, SimError> {
        let bound = before_ns.unwrap_or(i64::MAX);
        loop {
            let status = self
                .sim
                .run_while(|s| !s.hooks_pending() && s.next_event_ns().is_none_or(|t| t < bound))?;
            let pending = self.sim.state().hooks_pending();
            self.handle(adjuster, log)?;
            if status == Status::Finished {
                return Ok(Status::Finished);
            }
            if !pending {
                return Ok(Status::Paused);
            }
        }
    }

    /// A fork between events: the simulation's fork (outputs `out`) with a copy of the SPD
    /// state and the queued requests. Both continue like the uninterrupted run.
    pub fn fork(&mut self, out: Outputs) -> Result<SpdRun<'m, 's, C>, SimError> {
        Ok(SpdRun {
            sim: self.sim.fork(out)?,
            spd: self.spd,
            state: self.state.clone(),
            series: self.series.clone(),
            requests: self.requests.clone(),
            adjusted_ok: self.adjusted_ok,
        })
    }

    /// The run's result and the SPD counters.
    pub fn finish(self) -> Result<(RunResult, SpdState, SpdSummary), SimError> {
        let summary = SpdSummary {
            evaluations: self.state.evaluations,
            requests: self.state.requests,
            adjustments_ok: self.adjusted_ok,
            pending: self.requests.iter().filter(|r| r.is_some()).count() as u64,
        };
        let r = self.sim.finish()?;
        Ok((r, self.state, summary))
    }

    fn handle(
        &mut self,
        adjuster: &mut dyn Adjuster<C>,
        log: &mut dyn Log,
    ) -> Result<(), SimError> {
        for tag in self.sim.take_external() {
            if tag & REQUEST != 0 {
                let Some(req) = self.requests[(tag & !REQUEST) as usize].take() else {
                    continue;
                };
                let out = adjuster.adjust(&mut self.sim, self.spd, &mut self.state, &req)?;
                if out.ok {
                    self.adjusted_ok += 1;
                }
                let now = self.sim.state().now_ns() as f64 / 1e9;
                self.spd.adjusted(
                    &mut self.state,
                    &req,
                    out.ok,
                    now,
                    (out.size_before, out.size_after),
                    log,
                );
            } else {
                let mut host = Host {
                    sim: &mut self.sim,
                    requests: &mut self.requests,
                    error: None,
                };
                self.spd.on_timer(&mut self.state, &mut host, tag, log);
                if let Some(e) = host.error {
                    return Err(e);
                }
            }
        }
        let tuples = self.sim.take_observed();
        for o in tuples {
            let Some((point, metric)) = self.series[o.series as usize] else {
                continue;
            };
            let m = Measurement {
                point,
                metric,
                point_in_time: o.t,
                value: o.v,
                source: o.series,
            };
            let mut host = Host {
                sim: &mut self.sim,
                requests: &mut self.requests,
                error: None,
            };
            self.spd.on_measurement(&mut self.state, &mut host, &m, log);
            if let Some(e) = host.error {
                return Err(e);
            }
        }
        Ok(())
    }
}

/// An error of the SPD set-up as a simulation error (model error at t = 0).
pub fn setup_error(e: SpdError) -> SimError {
    SimError {
        message: e.to_string(),
        at_ns: 0,
        kind: SimErrorKind::Model,
    }
}

/// Elements of a target group as object ids, for test doubles that feed an element count.
pub fn placeholder_elements(n: usize, keep: &[ObjId]) -> Vec<ObjId> {
    let mut v: Vec<ObjId> = keep.iter().copied().take(n).collect();
    let mut k = 0u32;
    while v.len() < n {
        v.push(ObjId(u32::MAX - k));
        k += 1;
    }
    v
}

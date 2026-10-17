//! Scaling policies (SPD) in the Slingshot-exact engine (`DESIGN-SPD.md` X2d,
//! `docs/spec/slingshot/spd.md`, SS-SPD-*).
//!
//! The engine knows nothing about SPD models: an SPD layer (`simoxide-spd`) plugs in through
//! [`SpdHooks`]. What the engine emulates is Slingshot's bus around it:
//!
//! * `SpdBehavior.onPreSimulationConfigurationStarted` ([`SpdHooks::start`]): the
//!   `SimulationTimeReached` events, queued at t = 0 with their expected time as delay before
//!   every other result of the set-up (SS-SPD-1.2), and the policy subscribers, registered
//!   during the run and so called after every behaviour on `SimulationTimeReached` and
//!   `MeasurementMade` (SS-SPD-1.3);
//! * one subscriber per policy and stimulus ([`SpdHooks::on_time`],
//!   [`SpdHooks::on_measurement`]), each with its own disposed bit; a request is a
//!   `ModelAdjustmentRequested` at delay 0;
//! * `SpdAdjustmentBehavior.onModelAdjustmentRequested` ([`SpdHooks::adjust`]): the adjustment
//!   runs inside the handler on the engine's model through [`ChangeHost`]; the system, the
//!   SEFFs and the allocation are read live, so the change takes effect for the next event
//!   (SS-SPD-5.2). Its result is `ModelAdjusted`;
//! * the four subscribers of `ModelAdjusted` in their order (SS-SPD-5.1):
//!   `NumberOfElementsMonitorBehavior` (resources; no probe in this engine),
//!   `ResourceSimulation.onModelAdjusted` (active resources of new containers, removal of the
//!   deleted ones' resources: Q5, passive resources of new allocation contexts),
//!   `NumberOfElementsMonitorBehavior` (system; no probe), and
//!   `MonitorRepositoryInterpreterBehavior.onModelAdjusted` (the cloned monitors' calculators);
//! * the projection lines `spd_time`, `spd_request` and `spd_adjusted`, and the reference
//!   runner's aliases for the ids of elements created during the run (`Keys.alias`:
//!   `~EClass:entityName`, else the container's key plus `/feature.index`).
//!
//! After a change the engine continues on the recompiled model (`crate::change::recompile`:
//! stable typed ids, resources and programs keep their indices); the model reference is swapped
//! inside the adjustment handler, and every earlier compiled model is kept alive until the run
//! ends, so no reference into one can dangle.

use super::*;
use crate::change::{ChangeError, ChangeHost, ChangeOptions, ChangeReport, ChangeSet, RunSeed};
use simoxide_model::meta::PcmClass;
use simoxide_model::raw::{Graph, ObjId, PcmGraph};
use std::collections::HashSet;
use std::sync::Arc;

/// The stimulus a policy subscriber is registered for.
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stimulus {
    /// `SimulationTimeReached`.
    Time,
    /// `MeasurementMade`.
    Measurement,
}

/// A `SimulationTimeReached` of the set-up.
#[derive(Clone, Debug, PartialEq)]
pub struct SpdTimeEvent {
    /// Its delay at t = 0: the expected time (s).
    pub delay: f64,
    /// The target group's id (projection).
    pub target_group: Box<str>,
}

/// What `SpdBehavior.onPreSimulationConfigurationStarted` returns and registers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpdStart {
    /// The time events, in the order the interpreter returns them.
    pub time_events: Vec<SpdTimeEvent>,
    /// The policy subscribers, in registration order (policies in model order).
    pub subscribers: Vec<Stimulus>,
}

/// A `ModelAdjustmentRequested`.
#[derive(Clone, Debug, PartialEq)]
pub struct SpdRequest {
    /// The hook's handle of the request ([`SpdHooks::adjust`]).
    pub id: u32,
    /// The scaling policy's id (projection).
    pub policy: Box<str>,
}

/// A `MeasurementMade` as a policy subscriber sees it.
pub struct SpdMeasurement<'a> {
    /// The model the run simulates now.
    pub model: &'a Model,
    /// The engine's series (index into `RunResult::measurements`).
    pub series: u32,
    /// The measuring-point key with the model's ids (no aliases).
    pub key: &'a str,
    /// The metric's name (`crate::ir::metric_names`).
    pub metric: &'static str,
    pub point_in_time: f64,
    pub value: f64,
}

/// The `ModelAdjusted` of an adjustment (`SpdAdjustmentBehavior`): objects of the run's model
/// graph after it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpdAdjusted {
    pub ok: bool,
    /// The number of changes (resource environment, allocation, one per cloned monitor).
    pub changes: usize,
    pub new_containers: Vec<ObjId>,
    pub deleted_containers: Vec<ObjId>,
    pub new_allocation_contexts: Vec<ObjId>,
}

/// An exception of a policy subscriber or of `SpdBehavior` (SS-BUS-7: the subscriber is
/// disposed).
#[derive(Clone, Debug, PartialEq)]
pub struct HandlerError {
    pub class: &'static str,
    pub message: String,
}

/// An SPD layer of a Slingshot-exact run (see the module documentation).
pub trait SpdHooks {
    /// `SpdBehavior.onPreSimulationConfigurationStarted` on the run's model.
    fn start(&mut self, model: &Model) -> Result<SpdStart, HandlerError>;
    /// Subscriber `subscriber` (a [`Stimulus::Time`] one) on time event `event` (index into
    /// [`SpdStart::time_events`]) at `now`.
    fn on_time(
        &mut self,
        subscriber: usize,
        event: usize,
        now: f64,
    ) -> Result<Option<SpdRequest>, HandlerError>;
    /// Subscriber `subscriber` (a [`Stimulus::Measurement`] one) on a `MeasurementMade`.
    fn on_measurement(
        &mut self,
        subscriber: usize,
        m: &SpdMeasurement<'_>,
        now: f64,
    ) -> Result<Option<SpdRequest>, HandlerError>;
    /// Runs the adjustment of request `request` on the run's model (`host`). An error ends the
    /// run.
    fn adjust(&mut self, request: u32, host: &mut dyn ChangeHost) -> Result<SpdAdjusted, SimError>;
}

/// The engine's side of the SPD layer.
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Default, Clone)]
pub(crate) struct SpdSide {
    /// Per time event: the target group's id.
    pub(super) tg: Vec<Box<str>>,
    pub(super) subs: Vec<Stimulus>,
    pub(super) disposed: Vec<bool>,
    /// Some subscriber listens to measurements.
    pub measured: bool,
}

/// A `ModelAdjusted` in the engine's typed ids.
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
#[derive(Clone)]
pub(crate) struct Adjusted {
    pub(super) ok: bool,
    pub(super) changes: usize,
    pub(super) new: Vec<ContainerId>,
    pub(super) deleted: Vec<ContainerId>,
    pub(super) new_contexts: Vec<AllocationContextId>,
    pub(super) monitors: Vec<MonitorId>,
    pub(super) names_new: Vec<Box<str>>,
    pub(super) names_deleted: Vec<Box<str>>,
}

/// The model the engine runs after changes (owned; `Engine::cm` points into the last one).
#[derive(Default)]
pub(crate) struct OwnModels {
    /// Every compiled model of the run after a change, kept until the run ends.
    pub(super) models: Vec<Arc<CompiledModel>>,
    /// Build order of the current typed model.
    pub(super) order: Option<Arc<[ObjId]>>,
    pub(super) generation: u32,
    /// The model graph when it differs from the compiled model's outside SimOxide's
    /// metamodels (`ChangeHost::set_model_graph`).
    pub(super) graph: Option<Arc<Graph>>,
    /// The ids of the elements present at the start (`Keys.inputIds`).
    pub(super) input: Option<HashSet<Box<str>>>,
}

/// The first allocation context of each assembly context, compared by id (SS-ACT-9).
pub(super) fn first_match(m: &Model) -> Vec<Option<ContainerId>> {
    let mut alloc = vec![None; m.assembly_contexts.len()];
    let mut seen = vec![false; m.assembly_contexts.len()];
    if let Some(a) = m.allocations.iter().next() {
        for &ctx in &a.contexts {
            let c = &m.allocation_contexts[ctx];
            if let Some(ac) = c.assembly {
                // Slingshot compares ids: the first context of an equal-id assembly
                for (i, x) in m.assembly_contexts.iter_ids() {
                    if !seen[i.index()] && x.id == m.assembly_contexts[ac].id {
                        seen[i.index()] = true;
                        alloc[i.index()] = c.container;
                    }
                }
            }
        }
    }
    alloc
}

/// The ids of every object of `g`.
pub(super) fn ids_of(g: &Graph) -> HashSet<Box<str>> {
    (0..g.object_count() as u32)
        .map(ObjId)
        .filter_map(|o| g.id(o).map(Box::from))
        .collect()
}

/// The `EClass` name of `o`.
fn class_name(g: &Graph, o: ObjId) -> String {
    let c = g[o].class;
    if simoxide_model::raw::is_pcm_class(c) {
        let q = c.qualified_name();
        q.rsplit(':').next().unwrap_or(&q).to_string()
    } else {
        let q = g.meta_arc().qualified_name(c);
        q.rsplit([':', '.']).next().unwrap_or(&q).to_string()
    }
}

/// `entityName`, if the class has one.
fn entity_name(g: &Graph, o: ObjId) -> Option<Box<str>> {
    let c = g[o].class;
    if !simoxide_model::raw::is_pcm_class(c) {
        return None;
    }
    let f = c.feature("entityName")?;
    g.str_attr(o, f)
}

/// `Keys.id`: the id of an input element, the alias of a created one, else the fragment.
fn key_of(g: &Graph, o: ObjId, input: &HashSet<Box<str>>, depth: u32) -> String {
    match g.id(o) {
        Some(id) if input.contains(id) => id.to_string(),
        Some(_) if depth < 64 => alias_of(g, o, input, depth),
        Some(id) => id.to_string(),
        None => g.fragment(o),
    }
}

/// `Keys.alias`: `~EClass:entityName`, else the key of the container plus `/feature.index`.
fn alias_of(g: &Graph, o: ObjId, input: &HashSet<Box<str>>, depth: u32) -> String {
    if let Some(n) = entity_name(g, o) {
        return format!("~{}:{n}", class_name(g, o));
    }
    let Some((c, f)) = g[o].container else {
        return format!("~{}", class_name(g, o));
    };
    let info = g.meta_arc().feature(f);
    let mut seg = info.name.to_string();
    if info.is_many() {
        let i = g
            .get_refs(c, f)
            .iter()
            .position(|&x| x == o)
            .map_or(-1, |i| i as i64);
        seg.push_str(&format!(".{i}"));
    }
    let pid = key_of(g, c, input, depth + 1);
    if pid.starts_with('~') {
        format!("{pid}/{seg}")
    } else {
        format!("~{pid}/{seg}")
    }
}

/// The aliases of every element of `g` that is not an input element.
pub(super) fn aliases(g: &Graph, input: &HashSet<Box<str>>) -> HashMap<Box<str>, Box<str>> {
    let mut a = HashMap::new();
    for o in (0..g.object_count() as u32).map(ObjId) {
        if let Some(id) = g.id(o)
            && !input.contains(id)
        {
            a.insert(Box::from(id), alias_of(g, o, input, 0).into());
        }
    }
    a
}

impl<'m> Engine<'m> {
    /// The id as the reference runner writes it (`Keys.id`): aliased if created during the run.
    #[inline]
    pub(super) fn key<'a>(&'a self, id: &'a str) -> &'a str {
        if self.alias.is_empty() {
            return id;
        }
        self.alias.get(id).map_or(id, |a| a)
    }

    /// `SpdBehavior.onPreSimulationConfigurationStarted`.
    pub(super) fn spd_start(&mut self, h: &mut dyn SpdHooks) -> HR {
        let s = h.start(self.m).map_err(|e| Throw {
            class: e.class,
            message: e.message,
        })?;
        let mut out = Out::new();
        for t in &s.time_events {
            // `new SimulationTimeReached(..., delay)`: the interpreter builds the events before
            // the subscribers are registered
            out.push((Ev::SpdTime(out.len() as u32), delay(t.delay)?));
        }
        self.spd.tg = s.time_events.into_iter().map(|t| t.target_group).collect();
        self.spd.disposed = vec![false; s.subscribers.len()];
        self.spd.measured = s.subscribers.contains(&Stimulus::Measurement);
        self.spd.subs = s.subscribers;
        Ok(out)
    }

    /// The policy subscribers of `kind` in registration order, each with its disposed bit.
    pub(super) fn spd_subscribers(
        &mut self,
        h: &mut dyn SpdHooks,
        kind: Stimulus,
        mut f: impl FnMut(
            &mut Self,
            &mut dyn SpdHooks,
            usize,
        ) -> Result<Option<SpdRequest>, HandlerError>,
    ) {
        for k in 0..self.spd.subs.len() {
            if self.spd.subs[k] != kind || self.spd.disposed[k] {
                continue;
            }
            match f(self, h, k) {
                Ok(Some(r)) => self.schedule(Ev::SpdReq(r.id, r.policy.into()), 0.0),
                Ok(None) => {}
                Err(t) => {
                    self.spd.disposed[k] = true;
                    self.errors.push(format!(
                        "t={}: {} in SPDAdjustorContext subscriber {k}: {}",
                        crate::javafmt::double(self.now),
                        t.class,
                        t.message
                    ));
                }
            }
        }
    }

    /// The measurement of `MeasurementMade(s, t, v)` for the policy subscribers.
    pub(super) fn spd_measurement(&mut self, h: &mut dyn SpdHooks, s: SeriesId, t: f64, v: f64) {
        self.spd_subscribers(h, Stimulus::Measurement, |e, h, k| {
            let def = &e.meas.series[s as usize];
            let key = e.mon.raw_keys.get(&s).unwrap_or(&def.mp);
            let m = SpdMeasurement {
                model: e.m,
                series: s,
                key,
                metric: def.metric,
                point_in_time: t,
                value: v,
            };
            h.on_measurement(k, &m, e.now)
        });
    }

    /// `SpdAdjustmentBehavior.onModelAdjustmentRequested`: the adjustment on the run's model.
    pub(super) fn spd_adjust(&mut self, h: &mut dyn SpdHooks, request: u32) -> HR {
        let monitors = |m: &Model| -> Vec<MonitorId> {
            m.monitor_repositories
                .iter()
                .next()
                .map(|r| r.monitors.clone())
                .unwrap_or_default()
        };
        let before: HashSet<MonitorId> = monitors(self.m).into_iter().collect();
        let a = match h.adjust(request, self) {
            Ok(a) => a,
            Err(e) => {
                self.failure.get_or_insert(e);
                self.stopped = true;
                return Ok(Out::new());
            }
        };
        let m = self.m;
        let container = |o: ObjId| {
            m.containers
                .iter_ids()
                .find(|(_, c)| c.obj == o)
                .map(|x| x.0)
        };
        let new: Vec<ContainerId> = a
            .new_containers
            .iter()
            .filter_map(|&o| container(o))
            .collect();
        let deleted: Vec<ContainerId> = a
            .deleted_containers
            .iter()
            .filter_map(|&o| container(o))
            .collect();
        let new_contexts = a
            .new_allocation_contexts
            .iter()
            .filter_map(|&o| {
                m.allocation_contexts
                    .iter_ids()
                    .find(|(_, c)| c.obj == o)
                    .map(|x| x.0)
            })
            .collect();
        let adj = Adjusted {
            ok: a.ok,
            changes: a.changes,
            names_new: new.iter().map(|&c| m.containers[c].name.clone()).collect(),
            names_deleted: deleted
                .iter()
                .map(|&c| m.containers[c].name.clone())
                .collect(),
            new,
            deleted,
            new_contexts,
            monitors: if a.ok {
                monitors(m)
                    .into_iter()
                    .filter(|x| !before.contains(x))
                    .collect()
            } else {
                Vec::new()
            },
        };
        Ok(vec![(Ev::ModelAdjusted(Rc::new(adj)), 0.0)])
    }

    /// `ResourceSimulation.onModelAdjusted` (SS-RES-6.1): the active resources of new
    /// containers, the deleted containers' resources leave the table (their jobs: Q5), the
    /// passive resources of new allocation contexts.
    pub(super) fn rs_model_adjusted(&mut self, a: &Adjusted) -> HR {
        let cm = self.cm;
        let m = self.m;
        while self.res.len() < cm.resources.len() {
            let r = &cm.resources[self.res.len()];
            self.res.push(resource::new_res(r));
            self.res_live.push(false);
        }
        let res_of = |c: ContainerId| -> Vec<ResIdx> {
            m.containers[c]
                .processing_resources
                .iter()
                .filter_map(|&p| {
                    let rt = m.processing_resources[p].resource_type?;
                    cm.container_res.get(&(c, rt)).copied()
                })
                .collect()
        };
        for &c in &a.new {
            for ri in res_of(c) {
                self.res[ri as usize] = resource::new_res(&cm.resources[ri as usize]);
                self.res_live[ri as usize] = true;
            }
        }
        for &c in &a.deleted {
            for ri in res_of(c) {
                self.res_live[ri as usize] = false;
            }
        }
        self.passives_init(&a.new_contexts)?;
        Ok(Out::new())
    }

    /// `MonitorRepositoryInterpreterBehavior.onModelAdjusted`: the calculators of the monitors
    /// the adjustment cloned (SS-MEAS-1.1). Registered at once: the reference registers them a
    /// few same-instant generations later, which no projected step can observe unless a request
    /// reaches a new container's resource within the adjustment's instant.
    pub(super) fn mon_model_adjusted(&mut self, a: &Adjusted) -> HR {
        let cm = self.cm;
        for &mid in &a.monitors {
            self.mon.add(cm, &mut self.meas, mid, Some(&self.alias));
        }
        Ok(Out::new())
    }

    /// The projection line of `ModelAdjusted`.
    pub(super) fn project_adjusted(a: &Adjusted, l: &mut Line) {
        l.s.push_str(",\"ok\":");
        l.s.push_str(if a.ok { "true" } else { "false" });
        l.int("changes", a.changes as u64);
        if a.ok {
            for (k, names) in [("new", &a.names_new), ("deleted", &a.names_deleted)] {
                l.s.push_str(",\"");
                l.s.push_str(k);
                l.s.push_str("\":[");
                for (i, n) in names.iter().enumerate() {
                    if i > 0 {
                        l.s.push(',');
                    }
                    push_json_str(&mut l.s, n);
                }
                l.s.push(']');
            }
        }
    }

    /// What `ModelAdjusted`'s subscribers do for a change set made between two events
    /// (`SsxRun`'s [`ChangeHost::apply_changes`], e.g. at a resume), without the event: the
    /// containers, allocation contexts and monitors of the model now that `before` did not
    /// have, the containers it had that are gone.
    pub(super) fn sync_after_change(&mut self, before: &CompiledModel) -> Result<(), Throw> {
        let m = self.m;
        let m0 = &before.model;
        let env = |m: &Model| -> Vec<ContainerId> {
            m.resource_environments
                .iter()
                .flat_map(|r| r.containers.iter().copied())
                .collect()
        };
        let ctxs = |m: &Model| -> Vec<AllocationContextId> {
            m.allocations
                .iter()
                .next()
                .map(|a| a.contexts.clone())
                .unwrap_or_default()
        };
        let monitors = |m: &Model| -> Vec<MonitorId> {
            m.monitor_repositories
                .iter()
                .next()
                .map(|r| r.monitors.clone())
                .unwrap_or_default()
        };
        let (c0, c1) = (env(m0), env(m));
        let (a0, m_0) = (ctxs(m0), monitors(m0));
        let adj = Adjusted {
            ok: true,
            changes: 0,
            new: c1.iter().copied().filter(|c| !c0.contains(c)).collect(),
            deleted: c0.iter().copied().filter(|c| !c1.contains(c)).collect(),
            new_contexts: ctxs(m).into_iter().filter(|c| !a0.contains(c)).collect(),
            monitors: monitors(m)
                .into_iter()
                .filter(|x| !m_0.contains(x))
                .collect(),
            names_new: Vec::new(),
            names_deleted: Vec::new(),
        };
        self.rs_model_adjusted(&adj)?;
        self.mon_model_adjusted(&adj)?;
        Ok(())
    }

    /// The target group of time event `i`.
    pub(super) fn spd_tg(&self, i: u32) -> &str {
        &self.spd.tg[i as usize]
    }
}

impl ChangeHost for Engine<'_> {
    fn now_s(&self) -> f64 {
        self.now
    }

    fn now_ns(&self) -> i64 {
        simoxide_sched::time::span(self.now)
    }

    fn base_compiled(&self) -> &CompiledModel {
        self.base
    }

    fn compiled_now(&self) -> &CompiledModel {
        self.cm
    }

    fn model_generation(&self) -> u32 {
        self.own.generation
    }

    fn model_graph(&self) -> &Graph {
        self.own.graph.as_deref().unwrap_or(&self.cm.model.graph)
    }

    fn set_model_graph(&mut self, graph: Arc<Graph>) -> Result<(), ChangeError> {
        let old = self.model_graph();
        let same = graph.object_count() >= old.object_count()
            && old.objects().zip(graph.objects()).all(|(a, b)| {
                a.class == b.class
                    || !(simoxide_model::raw::is_pcm_class(a.class)
                        || simoxide_model::raw::is_pcm_class(b.class))
            });
        if !same {
            return Err(ChangeError::Invalid(vec![
                "the graph is not a changed copy of the run's model graph".into(),
            ]));
        }
        self.own.graph = Some(graph);
        Ok(())
    }

    fn apply_changes(
        &mut self,
        graph: Graph,
        changes: &ChangeSet,
    ) -> Result<ChangeReport, ChangeError> {
        if self.stopped {
            return Err(ChangeError::State("the run has ended".into()));
        }
        if changes.is_empty() {
            return Ok(ChangeReport {
                generation: self.own.generation,
                ..ChangeReport::default()
            });
        }
        let order: Arc<[ObjId]> = match &self.own.order {
            Some(o) => o.clone(),
            None => simoxide_model::build_order(&self.cm.model.graph).into(),
        };
        let rec = crate::change::recompile(
            self.cm,
            &order,
            RunSeed {
                series: &self.cm.series,
                dyn_paths: &[],
                opts: ChangeOptions::for_flavour(self.cm.flavour),
            },
            graph,
            changes,
        )?;
        let mut report = rec.report;
        let cm = Arc::new(rec.cm);
        self.own.generation += 1;
        report.generation = self.own.generation;
        self.own.order = Some(rec.order.into());
        self.own.graph = None;
        // SAFETY: the `Arc` is kept in `self.own.models` until the engine is dropped, so the
        // pointee lives as long as the engine and never moves; earlier models are kept too, so
        // a reference to one held by a caller of this handler stays valid. The engine's
        // lifetime `'m` is only ever used through the engine.
        self.cm = unsafe { &*Arc::as_ptr(&cm) };
        self.own.models.push(cm);
        self.m = &self.cm.model;
        // the allocation is read live (SS-ACT-9); the active-resource table changes at
        // `ModelAdjusted`
        self.alloc = first_match(self.m);
        let input = self
            .own
            .input
            .get_or_insert_with(|| ids_of(&self.base.model.graph));
        self.alias = aliases(&self.cm.model.graph, input);
        Ok(report)
    }

    fn fail_run(&mut self, message: String) -> SimError {
        let e = self.sim_error(message);
        self.failure.get_or_insert(e.clone());
        self.stopped = true;
        e
    }
}

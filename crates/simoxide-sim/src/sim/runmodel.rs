//! The run's own model after runtime model changes ([`RunModel`]) and the change path of a
//! running simulation ([`Simulation::apply_changes`]); the recompilation is `crate::change`.

use super::state::Shape;
use super::*;
use crate::change::{ChangeError, ChangeReport, ChangeSet, RunSeed, StateOps, WorkloadOp};
use crate::code::{Flat, SOp, UOp};
use simoxide_model::raw::{Graph, ObjId};
use std::sync::Arc;

/// The model a run simulates, as part of its [`RunState`]: the shared base [`CompiledModel`]
/// until the first model change, then a compiled model of the run's own (`Arc`, shared by the
/// forks of the run until one of them changes its model again).
///
/// The run's compiled model keeps every index the run state holds: resources, assembly-context
/// paths, instruction positions, measurement series, StoEx programs and frame keys keep their
/// meaning; new entities are appended; removed ones stay (tombstones). Its typed model
/// (`CompiledModel::model`) holds the run's model graph.
///
/// A run without model changes never reads or writes this (one `None` per state); its clone is
/// a reference count.
///
/// The run's model graph can also differ from its compiled model's outside SimOxide's
/// metamodels only ([`Simulation::set_model_graph`]: models of other metamodels such as a
/// runtime measurement model, a QVTo host's metamodel objects and id generator); the compiled
/// model is the same then.
#[derive(Clone, Default)]
pub struct RunModel(Option<Box<Changed>>, Option<Arc<Graph>>);

/// A run's own model (behind one pointer, so that runs without changes carry 8 bytes).
#[derive(Clone)]
pub(super) struct Changed {
    /// The compiled model after the changes.
    pub(super) cm: Arc<CompiledModel>,
    /// Build order of `cm`'s typed model (the typed ids of its elements).
    pub(super) order: Arc<[ObjId]>,
    /// The shape of the base model.
    pub(super) base_shape: Shape,
    pub(super) generation: u32,
}

impl RunModel {
    /// Number of applied change sets that changed the model (0: the base model).
    pub fn generation(&self) -> u32 {
        self.0.as_ref().map_or(0, |c| c.generation)
    }

    /// The run's own compiled model, if its model changed.
    pub fn compiled(&self) -> Option<&Arc<CompiledModel>> {
        self.0.as_ref().map(|c| &c.cm)
    }

    pub(super) fn base_shape(&self) -> Option<Shape> {
        self.0.as_ref().map(|c| c.base_shape)
    }

    /// The build order of the run's own model (its typed ids).
    #[cfg(feature = "snapshot")]
    pub(super) fn order(&self) -> Option<&Arc<[ObjId]>> {
        self.0.as_ref().map(|c| &c.order)
    }

    /// The run's model graph when it differs from the compiled model's (see
    /// [`Simulation::set_model_graph`]).
    #[cfg(feature = "snapshot")]
    pub(super) fn graph_override(&self) -> Option<&Graph> {
        self.1.as_deref()
    }

    #[cfg(feature = "snapshot")]
    pub(super) fn set_graph_override(&mut self, g: Arc<Graph>) {
        self.1 = Some(g);
    }

    /// A run's own model (restored from a snapshot).
    #[cfg(feature = "snapshot")]
    pub(super) fn changed(
        cm: Arc<CompiledModel>,
        order: Arc<[ObjId]>,
        base_shape: Shape,
        generation: u32,
    ) -> RunModel {
        RunModel(
            Some(Box::new(Changed {
                cm,
                order,
                base_shape,
                generation,
            })),
            None,
        )
    }

    /// The compiled model in `a` with the simulation's lifetime.
    ///
    /// # Safety
    ///
    /// The caller keeps `a` (the `Arc` in the run state of the simulation that holds the
    /// reference) alive while the reference is used: a [`Simulation`] stores it as its `cm`
    /// next to the state that owns the `Arc`, and replaces both together
    /// ([`Simulation::apply_changes`]); a model change happens between events, when no other
    /// copy of the reference is live. The pointee is immutable and never moves (it is the `Arc`'s
    /// allocation).
    pub(super) unsafe fn compiled_ref<'m>(a: &Arc<CompiledModel>) -> &'m CompiledModel {
        // SAFETY: see above; the pointer comes from a live `Arc`
        unsafe { &*Arc::as_ptr(a) }
    }
}

impl<C: Compat, F: Flavour> RunState<C, F> {
    /// The run's model (base or changed).
    pub fn model(&self) -> &RunModel {
        &self.model
    }
}

impl<'m, C: Compat, F: Flavour> Simulation<'m, C, F> {
    /// The compiled model the run was created on (before any model change).
    pub fn base_model(&self) -> &'m CompiledModel {
        self.base
    }

    /// The compiled model the run simulates now: the one it was created on, or its own after
    /// model changes ([`Simulation::apply_changes`]).
    pub fn compiled(&self) -> &CompiledModel {
        self.cm
    }

    /// The model graph the run simulates now (to copy, change and pass to
    /// [`Simulation::apply_changes`]).
    pub fn model_graph(&self) -> &Graph {
        self.st.model.1.as_deref().unwrap_or(&self.cm.model.graph)
    }

    /// Replaces the run's model graph by `graph`, a copy of [`Simulation::model_graph`] whose
    /// objects of SimOxide's metamodels are unchanged: only objects and resources of other
    /// metamodels (a runtime measurement model, metamodel objects, ...) changed or were added.
    /// The compiled model and the simulation do not change; later copies of the model graph
    /// (model changes, forks) carry the new content. For hosts that keep such models with the
    /// run's model (`simoxide-reconf`). Nothing is checked beyond the object classes.
    pub fn set_model_graph(&mut self, graph: Arc<Graph>) -> Result<(), ChangeError> {
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
        self.st.model.1 = Some(graph);
        Ok(())
    }

    /// The run's change options (see [`crate::ChangeOptions`]; the flavour's defaults unless
    /// set with [`Simulation::set_change_options`]).
    pub fn change_options(&self) -> crate::ChangeOptions {
        self.st.changes
    }

    /// Sets how the run takes model changes from now on (part of the run state: forks and
    /// snapshots keep it).
    pub fn set_change_options(&mut self, opts: crate::ChangeOptions) {
        self.st.changes = opts;
    }

    /// Applies a model change between two events (paused, or not started): `graph` is a copy of
    /// [`Simulation::model_graph`] with the changes, `changes` the change journal of the edits
    /// (an empty change set changes nothing). The run continues on the changed model as the
    /// syncers of SimuLizar 5.2.2 (or Slingshot's, for a Slingshot-flavour run) would leave it,
    /// with the run's [`crate::ChangeOptions`] (rebuild and diff, see `crate::change`). On an
    /// error nothing changes.
    ///
    /// Deterministic: the same state and the same change give the same continuation, so a fork
    /// that applies a change equals a run that applies it at the same point.
    pub fn apply_changes(
        &mut self,
        graph: Graph,
        changes: &ChangeSet,
    ) -> Result<ChangeReport, ChangeError> {
        match self.st.phase {
            Phase::NotStarted | Phase::Running => {}
            Phase::Finished => return Err(ChangeError::State("the run has finished".into())),
            Phase::Failed => return Err(ChangeError::State("the run has failed".into())),
        }
        if changes.is_empty() {
            return Ok(ChangeReport {
                generation: self.st.model.generation(),
                ..ChangeReport::default()
            });
        }
        let order: Arc<[ObjId]> = match &self.st.model.0 {
            Some(c) => c.order.clone(),
            None => simoxide_model::build_order(&self.cm.model.graph).into(),
        };
        let rec = crate::change::recompile(
            self.cm,
            &order,
            RunSeed {
                series: &self.st.meas.m.series,
                dyn_paths: &self.st.dyn_paths,
                opts: self.st.changes,
            },
            graph,
            changes,
        )?;
        let mut report = rec.report;
        let mut cm = rec.cm;
        // executions in progress on the new instructions (or their old ones), unused old
        // instructions dropped; computed before the state changes
        let pcs = if rec.ops.code_changed {
            let (flat, map) = self.plan_code(&cm)?;
            if self.st.traced {
                cm.code.traced = flat;
            } else {
                cm.code.plain = flat;
            }
            if map.retained > 0 {
                report.applied.push(format!(
                    "{} executions in progress keep their old instructions ({} instructions kept)",
                    map.kept_execs, map.retained
                ));
            }
            Some(map)
        } else {
            None
        };
        let cm = Arc::new(cm);
        let gen_after = self.st.model.generation() + 1;
        if let Some(map) = &pcs {
            self.remap_pcs(map);
        }
        self.adopt(&cm, &rec.path_extra);
        let changed = Changed {
            base_shape: self.st.model.base_shape().unwrap_or(self.st.shape),
            generation: gen_after,
            order: rec.order.into(),
            cm,
        };
        self.st.shape = Shape::of(&changed.cm);
        report.generation = changed.generation;
        // the changed graph is the compiled model's now
        self.st.model = RunModel(Some(Box::new(changed)), None);
        // SAFETY: the `Arc` just stored in the state (see `RunModel::compiled_ref`); the
        // previous run model's `Arc` may be dropped here, and no reference into it is live
        // (this runs between events, outside the interpreter).
        self.cm = unsafe { RunModel::compiled_ref(self.st.model.compiled().expect("model")) };
        self.code = self.cm.code.get(self.st.traced);
        self.adopt_ops(&rec.ops, gen_after).map_err(|e| {
            ChangeError::State(format!("the run failed applying the change: {}", e.message))
        })?;
        Ok(report)
    }

    /// Ends the run with an error between two events, for a host whose model change makes the
    /// reference abort (a reconfiguration whose change set fails with [`ChangeError::Abort`]:
    /// SimuLizar's syncer throws inside the reconfiguration process). The run is
    /// [`Phase::Failed`] at the current time; its outputs are flushed, and every later
    /// [`Simulation::run_until`] or [`Simulation::finish`] returns the error (also in forks).
    /// Returns the error.
    pub fn fail_run(&mut self, message: impl Into<String>) -> SimError {
        let e = SimError {
            message: message.into(),
            at_ns: F::Clock::to_ns(self.st.now),
            kind: SimErrorKind::Model,
        };
        self.st.phase = Phase::Failed;
        self.st.failure = Some(e.clone());
        self.flush_outputs();
        e
    }

    /// Fits the run state to the recompiled model `cm` (which keeps every index of the current
    /// one): runtime state for new resources, the resolution caches dropped (calls resolve again
    /// on the changed system), the assembly-context paths created during the run moved into
    /// `cm` (same ids; those outside its per-assembly-context table stay in the state's map).
    fn adopt(&mut self, cm: &CompiledModel, path_extra: &[((u32, AcRef), u32)]) {
        let st = &mut self.st;
        let ps = st.cfg.ps_algorithm;
        for r in &cm.resources[st.res.len()..] {
            st.res.push(ResRt::new(r, ps, F::SLINGSHOT));
        }
        // new series (their tuples start now; the run's own series are a prefix of `cm`'s)
        debug_assert!(cm.series.len() >= st.meas.m.series.len());
        for d in &cm.series[st.meas.m.series.len()..] {
            st.meas.m.series.push(d.clone());
            st.meas.m.rows.push(Vec::new());
        }
        if !st.meas.series_windows.is_empty() {
            st.meas.series_windows.resize(cm.series.len(), Vec::new());
        }
        st.dyn_paths.clear();
        st.dyn_path_child.clear();
        st.dyn_path_child.extend(path_extra.iter().copied());
        st.prov_ids.clear();
        st.call_cache.clear();
        st.call_cache.resize(
            cm.code.traced.call_sites.max(cm.code.plain.call_sites) as usize,
            CallCache::default(),
        );
        st.elsc_prov.clear();
        st.elsc_prov.resize(cm.uact.len(), u32::MAX);
    }
}

/// Where executions in progress continue after the instructions changed
/// ([`Simulation::plan_code`]): old program counter -> new one.
#[derive(Default)]
pub(super) struct PcMap {
    s: FxMap<u32, u32>,
    u: FxMap<u32, u32>,
    /// Old instructions kept for executions in progress.
    pub(super) retained: usize,
    /// Executions on kept instructions.
    pub(super) kept_execs: usize,
}

/// The action of a SEFF instruction.
fn sop_action(op: SOp) -> Option<ActionId> {
    match op {
        SOp::Start(a)
        | SOp::Begin(a)
        | SOp::BeginRt(a, _)
        | SOp::End(a)
        | SOp::EndRt(a, _)
        | SOp::Demand { a, .. }
        | SOp::Infra(a, _)
        | SOp::ResCall(a, _)
        | SOp::Call { a, .. }
        | SOp::Return { a, .. }
        | SOp::Exec(a) => Some(a),
        SOp::NoStart | SOp::Ret | SOp::NoStop => None,
    }
}

/// The place of a SEFF instruction within its action (`Begin` < demands < infrastructure
/// calls < resource calls < the call or effect < its return < `End`).
fn sop_rank(op: SOp) -> (u8, u32) {
    match op {
        SOp::Start(_) | SOp::Begin(_) | SOp::BeginRt(..) => (0, 0),
        SOp::Demand { .. } => (1, 0),
        SOp::Infra(_, i) => (2, i),
        SOp::ResCall(_, i) => (3, i),
        SOp::Call { .. } | SOp::Exec(_) => (4, 0),
        SOp::Return { .. } => (5, 0),
        _ => (6, 0),
    }
}

/// The same instruction (by kind and action, the `k`-th demand of the action).
fn same_sop(a: SOp, b: SOp) -> bool {
    match (a, b) {
        (SOp::Start(x), SOp::Start(y)) => x == y,
        (SOp::Begin(x) | SOp::BeginRt(x, _), SOp::Begin(y) | SOp::BeginRt(y, _)) => x == y,
        (SOp::End(x) | SOp::EndRt(x, _), SOp::End(y) | SOp::EndRt(y, _)) => x == y,
        (SOp::Infra(x, i), SOp::Infra(y, j)) | (SOp::ResCall(x, i), SOp::ResCall(y, j)) => {
            x == y && i == j
        }
        (SOp::Call { a: x, .. }, SOp::Call { a: y, .. })
        | (SOp::Return { a: x, .. }, SOp::Return { a: y, .. }) => x == y,
        (SOp::Exec(x), SOp::Exec(y)) => x == y,
        _ => false,
    }
}

/// Where an execution of a SEFF behaviour that ran `done` (its old instructions up to the
/// program counter) continues in `new` (the behaviour's new instructions), following the
/// changed chain: after the same instruction, or at the same place of its changed action.
/// `None`: the action left the behaviour, or a call in progress lost its return.
fn follow_sop(done: &[SOp], new: &[SOp]) -> Option<usize> {
    let Some(&e) = done.last() else {
        return Some(0);
    };
    let a = sop_action(e)?;
    let hit = if let SOp::Demand { .. } = e {
        let k = done
            .iter()
            .filter(|o| matches!(o, SOp::Demand { a: x, .. } if *x == a))
            .count();
        new.iter()
            .enumerate()
            .filter(|(_, o)| matches!(o, SOp::Demand { a: x, .. } if *x == a))
            .nth(k - 1)
            .map(|(q, _)| q)
    } else {
        new.iter().position(|&o| same_sop(e, o))
    };
    if let Some(q) = hit {
        return Some(q + 1);
    }
    if let SOp::Call { .. } = e {
        // the call's continuation returns into `Return`
        return None;
    }
    let mine: Vec<usize> = (0..new.len())
        .filter(|&q| sop_action(new[q]) == Some(a))
        .collect();
    let first = *mine.first()?;
    let r = sop_rank(e);
    Some(
        mine.iter()
            .rev()
            .find(|&&q| sop_rank(new[q]) <= r)
            .map_or(first, |&q| q + 1),
    )
}

/// The action of a usage-scenario instruction.
fn uop_action(op: UOp) -> Option<UserActionId> {
    match op {
        UOp::Start(a)
        | UOp::Begin(a)
        | UOp::BeginRt(a)
        | UOp::End(a)
        | UOp::EndRt(a, _)
        | UOp::Delay(a, _)
        | UOp::Call(a)
        | UOp::Return(a)
        | UOp::Exec(a) => Some(a),
        UOp::NoStart | UOp::Ret | UOp::NoStop => None,
    }
}

fn same_uop(a: UOp, b: UOp) -> bool {
    match (a, b) {
        (UOp::Start(x), UOp::Start(y)) => x == y,
        (UOp::Begin(x) | UOp::BeginRt(x), UOp::Begin(y) | UOp::BeginRt(y)) => x == y,
        (UOp::End(x) | UOp::EndRt(x, _), UOp::End(y) | UOp::EndRt(y, _)) => x == y,
        (UOp::Delay(x, _), UOp::Delay(y, _)) => x == y,
        (UOp::Call(x), UOp::Call(y)) | (UOp::Return(x), UOp::Return(y)) => x == y,
        (UOp::Exec(x), UOp::Exec(y)) => x == y,
        _ => false,
    }
}

fn uop_rank(op: UOp) -> u8 {
    match op {
        UOp::Start(_) | UOp::Begin(_) | UOp::BeginRt(_) => 0,
        UOp::Delay(..) | UOp::Call(_) | UOp::Exec(_) => 1,
        UOp::Return(_) => 2,
        _ => 3,
    }
}

/// [`follow_sop`] for usage-scenario behaviours.
fn follow_uop(done: &[UOp], new: &[UOp]) -> Option<usize> {
    let Some(&e) = done.last() else {
        return Some(0);
    };
    let a = uop_action(e)?;
    if let Some(q) = new.iter().position(|&o| same_uop(e, o)) {
        return Some(q + 1);
    }
    if let UOp::Call(_) = e {
        return None;
    }
    let mine: Vec<usize> = (0..new.len())
        .filter(|&q| uop_action(new[q]) == Some(a))
        .collect();
    let first = *mine.first()?;
    let r = uop_rank(e);
    Some(
        mine.iter()
            .rev()
            .find(|&&q| uop_rank(new[q]) <= r)
            .map_or(first, |&q| q + 1),
    )
}

/// Why kept instructions cannot run on the changed actions (`None`: they can).
fn sop_conflict(op: SOp, act: &[CAct]) -> Option<ActionId> {
    let ok = match op {
        SOp::Infra(a, i) => {
            matches!(&act[a.index()], CAct::Internal { infra, .. } if infra.len() > i as usize)
        }
        SOp::ResCall(a, i) => {
            matches!(&act[a.index()], CAct::Internal { rescalls, .. } if rescalls.len() > i as usize)
        }
        SOp::Call { a, .. } | SOp::Return { a, .. } => {
            matches!(act[a.index()], CAct::External { .. })
        }
        SOp::Exec(a) => !matches!(
            act[a.index()],
            CAct::Start | CAct::Stop | CAct::Internal { .. } | CAct::External { .. }
        ),
        _ => true,
    };
    (!ok).then(|| sop_action(op).expect("an action"))
}

fn uop_conflict(op: UOp, uact: &[CUAct]) -> Option<UserActionId> {
    let ok = match op {
        UOp::Call(a) | UOp::Return(a) => matches!(uact[a.index()], CUAct::Elsc { .. }),
        UOp::Exec(a) => matches!(uact[a.index()], CUAct::Branch { .. } | CUAct::Loop { .. }),
        _ => true,
    };
    (!ok).then(|| uop_action(op).expect("an action"))
}

impl<C: Compat, F: Flavour> Simulation<'_, C, F> {
    /// The run's instruction streams for the recompiled model `cm` (whose streams are new):
    /// where each execution in progress continues ([`crate::InProgress`]: on the changed
    /// behaviour, or on its old instructions, which are then kept after the new ones). Old
    /// instructions no execution uses are dropped (the streams are rebuilt at every change).
    /// Fails (nothing changed) when kept instructions or a call, infrastructure call or
    /// collection iterator in progress no longer fit their changed action.
    pub(super) fn plan_code(&self, cm: &CompiledModel) -> Result<(Flat, PcMap), ChangeError> {
        let follow = self.st.changes.in_progress == crate::InProgress::Follow;
        let old = self.code;
        let mut flat = cm.code.get(self.st.traced).clone();
        let fresh_s = flat.sops.len();
        let fresh_u = flat.uops.len();
        let mut map = PcMap::default();
        let mut kept_s: FxMap<u32, u32> = FxMap::default();
        let mut kept_u: FxMap<u32, u32> = FxMap::default();
        let mut errors: Vec<String> = Vec::new();
        for p in self
            .st
            .procs
            .iter()
            .filter(|p| p.state != PState::Terminated)
        {
            for c in &p.data.conts {
                match *c {
                    Cont::SBeh { pc, .. } if !map.s.contains_key(&pc) => {
                        let (start, end, b) = old.sseg_of(pc);
                        let done = &old.sops[start as usize..pc as usize];
                        let ns = flat.sbeh[b as usize] as usize;
                        let ne = flat.sseg_of(ns as u32).1 as usize;
                        let to = match follow
                            .then(|| follow_sop(done, &flat.sops[ns..ne.min(fresh_s)]))
                            .flatten()
                        {
                            Some(q) => (ns + q) as u32,
                            None => {
                                for &op in &old.sops[pc as usize..end as usize] {
                                    if let Some(a) = sop_conflict(op, &cm.act) {
                                        errors.push(format!(
                                            "an execution in progress runs action {} whose structure changed (its kind, or its number of calls)",
                                            cm.action_ids[a.index()]
                                        ));
                                        break;
                                    }
                                }
                                map.kept_execs += 1;
                                let k = *kept_s.entry(start).or_insert_with(|| {
                                    let at = flat.sops.len() as u32;
                                    flat.sseg.push((at, b));
                                    for &op in &old.sops[start as usize..end as usize] {
                                        flat.sops.push(match op {
                                            SOp::Call { a, .. } => {
                                                flat.call_sites += 1;
                                                SOp::Call {
                                                    a,
                                                    site: flat.call_sites - 1,
                                                }
                                            }
                                            op => op,
                                        });
                                    }
                                    at
                                });
                                k + (pc - start)
                            }
                        };
                        map.s.insert(pc, to);
                    }
                    Cont::UBeh { pc, .. } if !map.u.contains_key(&pc) => {
                        let (start, end, b) = old.useg_of(pc);
                        let done = &old.uops[start as usize..pc as usize];
                        let ns = flat.ubeh[b as usize] as usize;
                        let ne = flat.useg_of(ns as u32).1 as usize;
                        let to = match follow
                            .then(|| follow_uop(done, &flat.uops[ns..ne.min(fresh_u)]))
                            .flatten()
                        {
                            Some(q) => (ns + q) as u32,
                            None => {
                                for &op in &old.uops[pc as usize..end as usize] {
                                    if let Some(a) = uop_conflict(op, &cm.uact) {
                                        errors.push(format!(
                                            "an execution in progress runs user action {} whose structure changed",
                                            cm.uaction_ids[a.index()]
                                        ));
                                        break;
                                    }
                                }
                                map.kept_execs += 1;
                                let k = *kept_u.entry(start).or_insert_with(|| {
                                    let at = flat.uops.len() as u32;
                                    flat.useg.push((at, b));
                                    flat.uops
                                        .extend_from_slice(&old.uops[start as usize..end as usize]);
                                    at
                                });
                                k + (pc - start)
                            }
                        };
                        map.u.insert(pc, to);
                    }
                    Cont::Infra { a, ic, .. } if !matches!(&cm.act[a.index()], CAct::Internal { infra, .. } if infra.len() > ic as usize) =>
                    {
                        errors.push(format!(
                            "an infrastructure call of action {} is in progress and the action lost it",
                            cm.action_ids[a.index()]
                        ));
                    }
                    Cont::Coll { a, .. }
                        if !matches!(cm.act[a.index()], CAct::Collection { .. }) =>
                    {
                        errors.push(format!(
                            "the collection iterator {} is in progress and is no collection iterator any more",
                            cm.action_ids[a.index()]
                        ));
                    }
                    _ => {}
                }
            }
        }
        if !errors.is_empty() {
            errors.sort();
            errors.dedup();
            return Err(ChangeError::Unsupported(errors));
        }
        map.retained = (flat.sops.len() - fresh_s) + (flat.uops.len() - fresh_u);
        Ok((flat, map))
    }

    /// Moves the executions in progress as planned ([`Simulation::plan_code`]).
    pub(super) fn remap_pcs(&mut self, map: &PcMap) {
        for p in self.st.procs.iter_mut() {
            if p.state == PState::Terminated {
                continue;
            }
            for c in p.data.conts.iter_mut() {
                match c {
                    Cont::SBeh { pc, .. } => *pc = map.s[pc],
                    Cont::UBeh { pc, .. } => *pc = map.u[pc],
                    _ => {}
                }
            }
        }
    }

    /// The run state's part of a model change, after the merge (the removal policy, workload
    /// drivers, new windows and PRM recorders, the container count, Slingshot's passive
    /// resources). An error here (a limit, an evaluation error) fails the run.
    fn adopt_ops(&mut self, ops: &StateOps, gen_after: u32) -> Result<(), SimError> {
        let r = self.adopt_ops_inner(ops, gen_after);
        r.map_err(|e| self.fail_run(e.message))
    }

    fn adopt_ops_inner(&mut self, ops: &StateOps, gen_after: u32) -> R<()> {
        let cm = self.cm;
        let policy = self.st.changes.removal;
        self.remove_resources(&ops.removed, policy, gen_after)?;
        if self.st.phase != Phase::Running {
            // the start creates everything from the changed model
            return Ok(());
        }
        // usage model
        for &w in &ops.workloads {
            match w {
                WorkloadOp::Start(i) => match cm.scenarios[i as usize].workload {
                    CWorkload::Open { .. } => {
                        let k = self.spawn(PKind::OpenGen, Cont::OpenGen { scen: i, st: 0 })?;
                        self.activate(k)?;
                    }
                    CWorkload::Closed { population, .. } => {
                        for _ in 0..population.max(0) {
                            let k =
                                self.spawn(PKind::ClosedUser, Cont::ClosedUser { scen: i, st: 0 })?;
                            self.activate(k)?;
                        }
                    }
                },
                WorkloadOp::Stop(i) => {
                    for k in self.closed_users(i) {
                        self.st.procs[unkey(k).0 as usize].ss_flags |= STOP_REQUESTED;
                    }
                }
                WorkloadOp::Population { scen, to } => {
                    let users = self.closed_users(scen);
                    let n = users.len() as i64;
                    let to = to.max(0);
                    if n > to {
                        // `ClosedWorkload.stopUsers`: the oldest users leave after their run
                        for &k in &users[..(n - to) as usize] {
                            self.st.procs[unkey(k).0 as usize].ss_flags |= STOP_REQUESTED;
                        }
                    }
                    for _ in n..to {
                        let k = self.spawn(PKind::ClosedUser, Cont::ClosedUser { scen, st: 0 })?;
                        self.activate(k)?;
                    }
                }
            }
        }
        // new sliding windows start at the change
        if ops.first_new_window < cm.windows.len() {
            let n = self.st.meas.m.series.len();
            self.st.meas.series_windows.resize(n, Vec::new());
            for i in ops.first_new_window..cm.windows.len() {
                let w = &cm.windows[i];
                let keep_last = w.stat.is_none_or(|x| x.continuous);
                self.st.meas.windows.push(crate::windows::Window::new_at(
                    w.len,
                    w.inc,
                    keep_last,
                    F::Clock::secs(self.st.now),
                ));
                if let Some(inp) = w.input {
                    self.st.meas.series_windows[inp as usize].push(i as u32);
                }
                let Some(at) = F::Clock::checked_after(self.st.now, w.len) else {
                    return self.err("invalid sliding window length");
                };
                self.st.meas.events.insert(at, Ev::Window(i as u32));
            }
        }
        // new PRM recorders
        if ops.first_new_prm.0 < cm.prm_series.len() || ops.first_new_prm.1 < cm.prm_meas.len() {
            self.st
                .meas
                .prm
                .attach_new(cm, ops.first_new_prm.0, ops.first_new_prm.1);
            self.st.meas.prm_armed |= cm.prm_any;
        }
        self.st.meas.reconf.time_series = cm.reconf_time_series.clone();
        // NumberOfResourceContainerTrackingListener outside a reconfiguration process (the
        // process reports its count itself, `reconf_done`)
        if let Some((s, n)) = cm.container_count
            && self.st.meas.reconf.due.is_none()
            && n != self.st.meas.reconf.containers
        {
            self.st.meas.reconf.containers = n;
            let now = self.st.now;
            emit(
                &mut self.st.meas,
                &mut self.trace,
                self.st.cfg.store_measurements,
                now,
                s,
                F::Clock::secs(now),
                n as f64,
            );
        }
        if F::SLINGSHOT {
            // the passive resources of new allocation contexts (`buildPassiveResources`), and
            // the passive monitors of every instance (cloned monitors)
            for p in self.st.passives.iter_mut() {
                p.ss_mon = cm
                    .ss
                    .passive_mon
                    .iter()
                    .filter(|m| m.pr == p.pr && m.ac.is_none_or(|a| a == AssemblyContextId(p.ac)))
                    .map(|m| (m.series, m.metric))
                    .collect();
            }
            self.ss_create_passives()?;
        }
        Ok(())
    }

    /// The closed-workload users of scenario `scen` that have not been asked to stop, oldest
    /// first.
    fn closed_users(&self, scen: u32) -> Vec<u64> {
        let mut v: Vec<(u64, u64)> = self
            .st
            .procs
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.state != PState::Terminated
                    && p.kind == PKind::ClosedUser
                    && p.ss_flags & STOP_REQUESTED == 0
                    && matches!(p.data.conts.first(), Some(Cont::ClosedUser { scen: x, .. }) if *x == scen)
            })
            .map(|(i, p)| (p.pid, key(i as u32, p.generation)))
            .collect();
        v.sort_unstable();
        v.into_iter().map(|e| e.1).collect()
    }
}

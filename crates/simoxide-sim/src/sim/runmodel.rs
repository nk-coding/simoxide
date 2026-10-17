//! The run's own model after runtime model changes ([`RunModel`]) and the change path of a
//! running simulation ([`Simulation::apply_changes`]); the recompilation is `crate::change`.

use super::state::Shape;
use super::*;
use crate::change::{ChangeError, ChangeReport, ChangeSet, RunSeed};
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
            && old
                .objects()
                .zip(graph.objects())
                .all(|(a, b)| a.class == b.class);
        if !same {
            return Err(ChangeError::Invalid(vec![
                "the graph is not a changed copy of the run's model graph".into(),
            ]));
        }
        self.st.model.1 = Some(graph);
        Ok(())
    }

    /// Applies a model change between two events (paused, or not started): `graph` is a copy of
    /// [`Simulation::model_graph`] with the changes, `changes` the change journal of the edits
    /// (an empty change set changes nothing). The run continues on the changed model as the
    /// syncers of SimuLizar 5.2.2 would leave it (rebuild and diff, see `crate::change`);
    /// executions in progress keep their state. On an error nothing changes.
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
            },
            graph,
            changes,
        )?;
        let mut report = rec.report;
        let cm = Arc::new(rec.cm);
        self.adopt(&cm, &rec.path_extra);
        let changed = Changed {
            base_shape: self.st.model.base_shape().unwrap_or(self.st.shape),
            generation: self.st.model.generation() + 1,
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
        debug_assert_eq!(cm.series.len(), st.meas.m.series.len());
        st.dyn_paths.clear();
        st.dyn_path_child.clear();
        st.dyn_path_child.extend(path_extra.iter().copied());
        st.prov_ids.clear();
        st.call_cache.clear();
        st.call_cache
            .resize(cm.code.traced.call_sites as usize, CallCache::default());
        st.elsc_prov.clear();
        st.elsc_prov.resize(cm.uact.len(), u32::MAX);
    }
}

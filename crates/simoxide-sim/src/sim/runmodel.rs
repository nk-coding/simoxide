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
#[derive(Clone, Default)]
pub struct RunModel {
    /// The compiled model after changes (`None`: the base model).
    pub(super) cur: Option<Arc<CompiledModel>>,
    /// Build order of `cur`'s typed model (the typed ids of its elements).
    pub(super) order: Option<Arc<[ObjId]>>,
    /// The shape of the base model (set with `cur`).
    pub(super) base_shape: Option<Shape>,
    pub(super) generation: u32,
}

impl RunModel {
    /// Number of applied change sets that changed the model (0: the base model).
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// The run's own compiled model, if its model changed.
    pub fn compiled(&self) -> Option<&Arc<CompiledModel>> {
        self.cur.as_ref()
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

impl<C: Compat> RunState<C> {
    /// The run's model (base or changed).
    pub fn model(&self) -> &RunModel {
        &self.model
    }
}

impl<'m, C: Compat> Simulation<'m, C> {
    /// The compiled model the run simulates now: the one it was created on, or its own after
    /// model changes ([`Simulation::apply_changes`]).
    pub fn compiled(&self) -> &CompiledModel {
        self.cm
    }

    /// The model graph the run simulates now (to copy, change and pass to
    /// [`Simulation::apply_changes`]).
    pub fn model_graph(&self) -> &Graph {
        &self.cm.model.graph
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
                generation: self.st.model.generation,
                ..ChangeReport::default()
            });
        }
        let order: Arc<[ObjId]> = match &self.st.model.order {
            Some(o) => o.clone(),
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
        self.adopt(&cm);
        let m = &mut self.st.model;
        if m.base_shape.is_none() {
            m.base_shape = Some(self.st.shape);
        }
        m.generation += 1;
        m.order = Some(rec.order.into());
        m.cur = Some(cm.clone());
        self.st.shape = Shape::of(&cm);
        // SAFETY: `cm` is the `Arc` just stored in the state (see `RunModel::compiled_ref`);
        // the previous run model's `Arc` may be dropped here, and no reference into it is live
        // (this runs between events, outside the interpreter).
        self.cm = unsafe { RunModel::compiled_ref(self.st.model.cur.as_ref().expect("model")) };
        self.code = self.cm.code.get(self.st.traced);
        report.generation = self.st.model.generation;
        Ok(report)
    }

    /// Fits the run state to the recompiled model `cm` (which keeps every index of the current
    /// one): runtime state for new resources, the resolution caches dropped (calls resolve again
    /// on the changed system), the assembly-context paths created during the run moved into
    /// `cm` (same ids).
    fn adopt(&mut self, cm: &CompiledModel) {
        let st = &mut self.st;
        let ps = st.cfg.ps_algorithm;
        for r in &cm.resources[st.res.len()..] {
            st.res.push(ResRt::new(r, ps));
        }
        debug_assert_eq!(cm.series.len(), st.meas.m.series.len());
        st.dyn_paths.clear();
        st.dyn_path_child.clear();
        st.prov_ids.clear();
        st.call_cache.clear();
        st.call_cache
            .resize(cm.code.traced.call_sites as usize, CallCache::default());
        st.elsc_prov.clear();
        st.elsc_prov.resize(cm.uact.len(), u32::MAX);
    }
}

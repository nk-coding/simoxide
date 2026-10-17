//! Runtime model changes of SimOxide runs, from the QVTo side (`DESIGN-SPD.md` 5.3 and 5.5).
//!
//! The QVTo engine runs on SimOxide's model graph itself (`simoxide_model::raw::Graph`, the
//! shared EMF graph `simoxide_emf::Graph`): the graph is the `qvto_model::ModelStore`, and it
//! records every change in its [`Journal`]. Both kinds of change source go through it:
//!
//! - QVTo transformations: the host ([`QvtoCore`] shared by all runs, [`QvtoRun`] per run,
//!   created on first use), SimuLizar's rule folders ([`RuleSet`], [`QvtoRun::run_rules`]) and
//!   SPD adjustments ([`QvtoAdjuster`]; Slingshot's embedded `MainTransformation`:
//!   [`QvtoAdjuster::spd`], module [`spd`]; feature `spd`);
//! - direct edits ([`Edit`]): a small API over the same graph, for tests, snapshots and tools.
//!
//! [`change_set`] turns a journal into the simulator's [`ChangeSet`]; [`apply_journal`] and
//! [`Edit::apply`] hand the changed graph and its change set to
//! [`Simulation::apply_changes`], which recompiles the run's model (rebuild and diff).
//!
//! ```no_run
//! use simoxide_reconf::Edit;
//! use simoxide_sim::{CompiledModel, Outputs, Pause, SimConfig, Simulation};
//! let cm = CompiledModel::compile(simoxide_model::load_dir("corpus/h19_linking_resource").unwrap()).unwrap();
//! let cfg = SimConfig { max_measurements: 1000, ..Default::default() };
//! let mut sim = Simulation::new(&cm, cfg, Outputs::default()).unwrap();
//! sim.run_until(Pause::After(50_000_000_000)).unwrap();
//! let mut e = Edit::new(&sim);
//! let server = e.find("_h19_linking_resource_rc2").unwrap();
//! let copy = e.clone_container(server, "_rc3", "ServerNode2").unwrap();
//! for link in e.links_of(server) {
//!     e.connect(link, copy).unwrap();
//! }
//! let ac = e.find("_h19_linking_resource_ac2").unwrap();
//! e.allocate(ac, copy, "_alc3").unwrap();
//! let report = e.apply(&mut sim).unwrap();
//! println!("{:?}", report.applied);
//! let result = sim.finish().unwrap();
//! ```

use qvto_model::{Datum, ModelError, ModelStore, OwnedDatum};
use simoxide_model::meta::prelude::*;
use simoxide_model::meta::{ClassId, FeatureId, class as mc, feat as mf};
use simoxide_model::raw::{Graph, ObjId, PcmGraph, Value, is_pcm_class};
use simoxide_sim::{
    Change, ChangeError, ChangeKind, ChangeReport, ChangeSet, Compat, Flavour, Simulation,
};

pub use qvto_emf::{IdPolicy, Journal};

#[cfg(feature = "spd")]
mod adjust;
pub mod edits;
mod host;
mod process;
mod qvt;
pub mod resume;
mod rules;
#[cfg(feature = "snapshot")]
mod snapshot;
#[cfg(feature = "spd")]
pub mod spd;

#[cfg(feature = "spd")]
pub use adjust::{AdjustHooks, DefaultAdjustHooks, QvtoAdjuster};
pub use edits::{ChangeFile, ChangeFileError, EditOp, OptionOp};
pub use host::{
    AbortPolicy, Bind, CoreBuilder, FailedRunChanges, IdSourceFn, Imports, Invocation, InvokeHooks,
    ModelRef, NsBinding, Outcome, ParamDir, ParamInfo, QvtMode, QvtoCore, QvtoRun, ReconfError,
    RunConfig, Transformation, palladio_metamodels, resolve_mode,
};
pub use process::{PRM_URI, ReconfRecord, RuleRecord, RulesHost, RulesRun, RulesStats};
#[cfg(feature = "snapshot")]
pub use resume::explore;
pub use resume::{
    ChangeStep, ResumeChanges, ResumeError, StepReport, Variant, VariantResult, VariantRun, compare,
};
pub use rules::{RuleSet, RulesOutcome};
#[cfg(feature = "snapshot")]
pub use snapshot::{QVTO, QvtoState, RULES};
#[cfg(feature = "spd")]
pub use spd::{SlingshotHooks, SpdAdjust, SpdVariant, main_transformation};

/// The simulator's change set of a journal recorded on `g`: the changes of objects of
/// SimOxide's metamodels, in order, with the object values of list changes (objects of other
/// metamodels in the same graph, such as the PRM, are not the simulator's).
pub fn change_set(g: &Graph, j: &Journal) -> ChangeSet {
    use qvto_emf::ChangeKind as J;
    let pcm = |o: ObjId| (o.index() < g.object_count()) && is_pcm_class(g[o].class);
    let obj = |d: &OwnedDatum<ObjId>| match d {
        OwnedDatum::Obj(x) if pcm(*x) => Some(*x),
        _ => None,
    };
    let mut cs = ChangeSet::new();
    for c in &j.changes {
        if !pcm(c.obj) {
            continue;
        }
        let kind = match &c.kind {
            J::Created(class) => ChangeKind::Created(*class),
            J::Set { feature, .. } => ChangeKind::Set { feature: *feature },
            J::Unset { feature, .. } => ChangeKind::Unset { feature: *feature },
            J::Added { feature, value, .. } => ChangeKind::Added {
                feature: *feature,
                value: obj(value),
            },
            J::Removed { feature, value, .. } => ChangeKind::Removed {
                feature: *feature,
                value: obj(value),
            },
            J::Replaced { feature, .. } => ChangeKind::Replaced { feature: *feature },
            J::Moved { feature, .. } => ChangeKind::Moved { feature: *feature },
            J::Container { .. } => ChangeKind::Container,
            J::RootAdded { .. } | J::RootRemoved { .. } => ChangeKind::Root,
            J::Deleted => ChangeKind::Deleted,
        };
        cs.changes.push(Change { obj: c.obj, kind });
    }
    cs
}

/// Applies the changes recorded in `journal` (made on `graph`, a copy of the run's
/// [`Simulation::model_graph`]) to the run.
pub fn apply_journal<C: Compat, F: Flavour>(
    sim: &mut Simulation<'_, C, F>,
    graph: Graph,
    journal: &Journal,
) -> Result<ChangeReport, ChangeError> {
    let cs = change_set(&graph, journal);
    sim.apply_changes(graph, &cs)
}

/// An error of an edit (the store's).
#[derive(Debug, Clone, PartialEq)]
pub struct EditError(pub String);

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EditError {}

impl From<ModelError> for EditError {
    fn from(e: ModelError) -> Self {
        EditError(format!("{e:?}"))
    }
}

type R<T> = Result<T, EditError>;

/// Direct edits of a run's model, recorded in the graph's journal: a copy of the run's graph,
/// changed through [`Edit::store`] (any `ModelStore` operation) or the helpers, then applied
/// with [`Edit::apply`]. Ids of new objects are given by the caller (they appear in traces and
/// decide the resource registry's order).
pub struct Edit {
    graph: Graph,
}

impl Edit {
    /// Starts an edit of `sim`'s current model.
    pub fn new<H: simoxide_sim::ChangeHost + ?Sized>(sim: &H) -> Edit {
        let mut graph = sim.model_graph().clone();
        graph.start_journal();
        Edit { graph }
    }

    /// The graph as edited so far.
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// The edited graph as a model store (every change is recorded).
    pub fn store(&mut self) -> &mut Graph {
        &mut self.graph
    }

    /// The graph and the change set of the edits.
    pub fn finish(mut self) -> (Graph, ChangeSet) {
        let j = self.graph.stop_journal().unwrap_or_default();
        let cs = change_set(&self.graph, &j);
        (self.graph, cs)
    }

    /// Applies the edits to `sim` ([`Simulation::apply_changes`]).
    pub fn apply<H: simoxide_sim::ChangeHost + ?Sized>(
        self,
        sim: &mut H,
    ) -> Result<ChangeReport, ChangeError> {
        let (g, cs) = self.finish();
        sim.apply_changes(g, &cs)
    }

    /// The object with this id (`Identifier.id` or `xmi:id`) contained in a resource.
    pub fn find(&self, id: &str) -> Option<ObjId> {
        (0..self.graph.resources().len() as u32)
            .flat_map(|r| self.graph.tree_contents(simoxide_model::raw::ResId(r)))
            .find(|&o| self.graph.id(o) == Some(id))
    }

    /// Sets the specification of the random variable that `owner` contains in `feature` (e.g.
    /// `ProcessingResourceSpecification.processingRate`, a link's latency, a demand), creating
    /// the variable if needed.
    pub fn set_spec(&mut self, owner: ObjId, feature: FeatureId, spec: &str) -> R<()> {
        let st = &mut self.graph;
        let rv = match st.get(owner, feature) {
            Datum::Obj(o) => o,
            _ => {
                let rv = st.create(mc::PCMRandomVariable)?;
                st.set(owner, feature, Datum::Obj(rv))?;
                rv
            }
        };
        st.set(rv, mf::RandomVariable_specification, Datum::Str(spec))?;
        Ok(())
    }

    /// Sets the processing rate of a processing resource specification.
    pub fn set_rate(&mut self, spec: ObjId, rate: &str) -> R<()> {
        self.set_spec(
            spec,
            mf::ProcessingResourceSpecification_processingRate_ProcessingResourceSpecification,
            rate,
        )
    }

    /// A new resource container in `container`'s resource environment with copies of its
    /// processing resource specifications (ids `<id>_<n>`); not connected to any linking
    /// resource ([`Edit::connect`], [`Edit::links_of`]). Returns the new container.
    pub fn clone_container(&mut self, container: ObjId, id: &str, name: &str) -> R<ObjId> {
        let g = &self.graph;
        let Some((env, _)) = g[container].container else {
            return Err(EditError(
                "the container is not in a resource environment".into(),
            ));
        };
        struct Spec {
            class: ClassId,
            rate: Option<Box<str>>,
            values: Vec<(FeatureId, Value)>,
            refs: Vec<(FeatureId, ObjId)>,
        }
        let rv_spec =
            |o: Option<ObjId>| o.and_then(|o| g.str_attr(o, mf::RandomVariable_specification));
        let specs: Vec<Spec> = g
            .get_refs(
                container,
                mf::ResourceContainer_activeResourceSpecifications_ResourceContainer,
            )
            .iter()
            .map(|&p| Spec {
                class: g[p].class,
                rate: rv_spec(g.get_ref(
                    p,
                    mf::ProcessingResourceSpecification_processingRate_ProcessingResourceSpecification,
                )),
                values: [
                    mf::ProcessingResourceSpecification_numberOfReplicas,
                    mf::ProcessingResourceSpecification_MTTF,
                    mf::ProcessingResourceSpecification_MTTR,
                    mf::ProcessingResourceSpecification_requiredByContainer,
                ]
                .into_iter()
                .filter(|&f| g.is_stored(p, f))
                .map(|f| (f, g.attr(p, f)))
                .collect(),
                refs: [
                    mf::ProcessingResourceSpecification_schedulingPolicy,
                    mf::ProcessingResourceSpecification_activeResourceType_ActiveResourceSpecification,
                ]
                .into_iter()
                .filter_map(|f| g.get_ref(p, f).map(|x| (f, x)))
                .collect(),
            })
            .collect();
        let st = &mut self.graph;
        let c = st.create(mc::ResourceContainer)?;
        st.set(c, mf::identifier_Identifier_id, Datum::Str(id))?;
        st.set(c, mf::NamedElement_entityName, Datum::Str(name))?;
        for (n, s) in specs.iter().enumerate() {
            let p = st.create(s.class)?;
            st.set(
                p,
                mf::identifier_Identifier_id,
                Datum::Str(&format!("{id}_{n}")),
            )?;
            for (f, v) in &s.values {
                let d = match v {
                    Value::Int(i) => Datum::Int(*i as i32),
                    Value::Double(x) => Datum::Double(*x),
                    Value::Bool(b) => Datum::Bool(*b),
                    _ => continue,
                };
                st.set(p, *f, d)?;
            }
            for (f, x) in &s.refs {
                st.set(p, *f, Datum::Obj(*x))?;
            }
            if let Some(rate) = &s.rate {
                let rv = st.create(mc::PCMRandomVariable)?;
                st.set(rv, mf::RandomVariable_specification, Datum::Str(rate))?;
                st.set(
                    p,
                    mf::ProcessingResourceSpecification_processingRate_ProcessingResourceSpecification,
                    Datum::Obj(rv),
                )?;
            }
            st.add(
                c,
                mf::ResourceContainer_activeResourceSpecifications_ResourceContainer,
                Datum::Obj(p),
                None,
            )?;
        }
        st.add(
            env,
            mf::ResourceEnvironment_resourceContainer_ResourceEnvironment,
            Datum::Obj(c),
            None,
        )?;
        Ok(c)
    }

    /// The linking resources of `container`'s resource environment connected to it.
    pub fn links_of(&self, container: ObjId) -> Vec<ObjId> {
        let g = &self.graph;
        let Some((env, _)) = g[container].container else {
            return Vec::new();
        };
        g.get_refs(
            env,
            mf::ResourceEnvironment_linkingResources__ResourceEnvironment,
        )
        .iter()
        .copied()
        .filter(|&l| {
            g.get_refs(
                l,
                mf::LinkingResource_connectedResourceContainers_LinkingResource,
            )
            .contains(&container)
        })
        .collect()
    }

    /// A new assembly context in the structure of `assembly` that encapsulates the same
    /// component (without configuration parameter usages). Returns it.
    pub fn clone_assembly(&mut self, assembly: ObjId, id: &str, name: &str) -> R<ObjId> {
        let g = &self.graph;
        let Some((structure, f)) = g[assembly].container else {
            return Err(EditError(
                "the assembly context is not in a structure".into(),
            ));
        };
        let component = g.get_ref(
            assembly,
            mf::AssemblyContext_encapsulatedComponent__AssemblyContext,
        );
        let st = &mut self.graph;
        let a = st.create(mc::AssemblyContext)?;
        st.set(a, mf::identifier_Identifier_id, Datum::Str(id))?;
        st.set(a, mf::NamedElement_entityName, Datum::Str(name))?;
        if let Some(c) = component {
            st.set(
                a,
                mf::AssemblyContext_encapsulatedComponent__AssemblyContext,
                Datum::Obj(c),
            )?;
        }
        st.add(structure, f, Datum::Obj(a), None)?;
        Ok(a)
    }

    /// Sets a single-valued reference (e.g. an assembly connector's providing assembly context).
    pub fn set_ref(&mut self, o: ObjId, feature: FeatureId, target: ObjId) -> R<()> {
        self.graph.set(o, feature, Datum::Obj(target))?;
        Ok(())
    }

    /// Sets a `double` attribute (e.g. a branch probability).
    pub fn set_double(&mut self, o: ObjId, feature: FeatureId, v: f64) -> R<()> {
        self.graph.set(o, feature, Datum::Double(v))?;
        Ok(())
    }

    /// Connects `container` to the linking resource `link`.
    pub fn connect(&mut self, link: ObjId, container: ObjId) -> R<()> {
        self.graph.add(
            link,
            mf::LinkingResource_connectedResourceContainers_LinkingResource,
            Datum::Obj(container),
            None,
        )?;
        Ok(())
    }

    /// A new allocation context (id `id`) in the allocation that allocates `assembly` to
    /// `container`; its references are set before it is added to the allocation, as a
    /// transformation would do it. Returns the allocation context.
    pub fn allocate(&mut self, assembly: ObjId, container: ObjId, id: &str) -> R<ObjId> {
        let g = &self.graph;
        let allocation = (0..g.object_count() as u32)
            .map(ObjId)
            .find(|&o| {
                g[o].class.is_a(mc::Allocation) && g[o].container.is_none() && g[o].proxy.is_none()
            })
            .ok_or_else(|| EditError("no allocation".into()))?;
        let st = &mut self.graph;
        let a = st.create(mc::AllocationContext)?;
        st.set(a, mf::identifier_Identifier_id, Datum::Str(id))?;
        st.set(a, mf::NamedElement_entityName, Datum::Str(id))?;
        st.set(
            a,
            mf::AllocationContext_resourceContainer_AllocationContext,
            Datum::Obj(container),
        )?;
        st.set(
            a,
            mf::AllocationContext_assemblyContext_AllocationContext,
            Datum::Obj(assembly),
        )?;
        st.add(
            allocation,
            mf::Allocation_allocationContexts_Allocation,
            Datum::Obj(a),
            None,
        )?;
        Ok(a)
    }

    /// `EcoreUtil.delete` of an object: it leaves its container, and every reference to it is
    /// removed (or unset: for a resource container referenced by an allocation context that is
    /// an `UNSET` notification, on which SimuLizar's allocation syncer aborts).
    pub fn delete(&mut self, o: ObjId) {
        self.graph.delete(o);
    }

    /// Monitor cloning as Slingshot's `ResourceContainerMonitorCloner` does it after a
    /// scale-out: for every monitor of the monitor repository whose measuring point is a
    /// resource-container measuring point of `template`, and for every monitor of an
    /// active-resource measuring point of one of `template`'s processing resources (per
    /// specification, in specification order), a copy for `container` (for an active resource:
    /// `container`'s first specification of the same resource type, the replica id kept) with a
    /// new measuring point in the original's measuring-point repository. Copies keep
    /// `activated`, the metric descriptions, `triggersSelfAdaptations` and the processing types
    /// (attributes copied); names get `-copy`; ids are `<id_prefix>_<n>` (Slingshot draws
    /// UUIDs). Returns the new monitors. The change path then attaches them: in the Slingshot
    /// flavour every new monitor probes; in the SimuLizar flavour those of new active
    /// resources (`ResourceEnvironmentSyncer.attachMonitors`), all with `MonitorSync::Full`.
    pub fn clone_monitors(
        &mut self,
        template: ObjId,
        container: ObjId,
        id_prefix: &str,
    ) -> R<Vec<ObjId>> {
        let g = &self.graph;
        let Some(repo) = (0..g.object_count() as u32)
            .map(ObjId)
            .find(|&o| g[o].class.is_a(mc::MonitorRepository) && g[o].proxy.is_none())
        else {
            return Err(EditError("no monitor repository".into()));
        };
        let specs_of = |c: ObjId| {
            g.get_refs(
                c,
                mf::ResourceContainer_activeResourceSpecifications_ResourceContainer,
            )
            .to_vec()
        };
        let rtype = |p: ObjId| {
            g.get_ref(
                p,
                mf::ProcessingResourceSpecification_activeResourceType_ActiveResourceSpecification,
            )
            .and_then(|t| g.id(t).map(str::to_string))
        };
        let monitors = g.get_refs(repo, mf::MonitorRepository_monitors).to_vec();
        let mp_of = |m: ObjId| g.get_ref(m, mf::Monitor_measuringPoint);
        // (original monitor, new measuring point's class, its target, replica id)
        let mut plan: Vec<(ObjId, ObjId, ClassId, ObjId, Option<i32>)> = Vec::new();
        for &m in &monitors {
            if let Some(mp) = mp_of(m)
                && g[mp].class.is_a(mc::ResourceContainerMeasuringPoint)
                && g.get_ref(mp, mf::ResourceContainerReference_resourceContainer) == Some(template)
            {
                plan.push((m, mp, mc::ResourceContainerMeasuringPoint, container, None));
            }
        }
        let new_specs = specs_of(container);
        for spec in specs_of(template) {
            for &m in &monitors {
                let Some(mp) = mp_of(m) else { continue };
                if !g[mp].class.is_a(mc::ActiveResourceMeasuringPoint)
                    || g.get_ref(mp, mf::ActiveResourceReference_activeResource) != Some(spec)
                {
                    continue;
                }
                let Some(target) = new_specs
                    .iter()
                    .copied()
                    .find(|&p| rtype(p).is_some() && rtype(p) == rtype(spec))
                else {
                    continue;
                };
                let replica = match g.get(mp, mf::ActiveResourceReference_replicaID) {
                    Datum::Int(i) => Some(i),
                    _ => Some(0),
                };
                plan.push((m, mp, mc::ActiveResourceMeasuringPoint, target, replica));
            }
        }
        let mut out = Vec::new();
        for (n, &(m, _, _, target, _)) in plan.iter().enumerate() {
            out.push(self.clone_monitor(m, target, &format!("{id_prefix}_{}", n + 1))?);
        }
        Ok(out)
    }

    /// One monitor copy as `ResourceContainerMonitorCloner.createMonitor` makes it: `monitor`
    /// (whose measuring point is a resource-container or active-resource measuring point) for
    /// `target` (a container, or a processing resource specification; the replica id is
    /// kept), with a new measuring point in the original's measuring-point repository, the
    /// copy added to the monitor repository. Ids `<id>`, `<id>_<n>` for the specifications and
    /// processing types. Returns the new monitor.
    pub fn clone_monitor(&mut self, monitor: ObjId, target: ObjId, id: &str) -> R<ObjId> {
        /// A processing type: its class and attributes.
        type Processing = (ClassId, Vec<(FeatureId, OwnedDatum<ObjId>)>);
        let g = &self.graph;
        let Some((repo, repo_f)) = g[monitor].container else {
            return Err(EditError(
                "the monitor is not in a monitor repository".into(),
            ));
        };
        let Some(mp) = g.get_ref(monitor, mf::Monitor_measuringPoint) else {
            return Err(EditError("the monitor has no measuring point".into()));
        };
        let class = if g[target].class.is_a(mc::ResourceContainer) {
            mc::ResourceContainerMeasuringPoint
        } else if g[target].class.is_a(mc::ProcessingResourceSpecification) {
            mc::ActiveResourceMeasuringPoint
        } else {
            return Err(EditError(format!(
                "{} is no container or processing resource",
                g.describe(target)
            )));
        };
        let replica = match g.get(mp, mf::ActiveResourceReference_replicaID) {
            Datum::Int(i) => i,
            _ => 0,
        };
        let mp_repo = g[mp].container;
        let activated = matches!(g.get(monitor, mf::Monitor_activated), Datum::Bool(true));
        let name = g
            .str_attr(monitor, mf::NamedElement_entityName)
            .unwrap_or_default()
            .to_string();
        let specs: Vec<(Option<ObjId>, bool, Option<Processing>)> = g
            .get_refs(monitor, mf::Monitor_measurementSpecifications)
            .iter()
            .map(|&sp| {
                let pt = g
                    .get_ref(sp, mf::MeasurementSpecification_processingType)
                    .map(|pt| {
                        let class = g[pt].class;
                        let values = class
                            .features()
                            .filter(|&f| f != mf::identifier_Identifier_id && g.is_stored(pt, f))
                            .filter_map(|f| match g.get(pt, f) {
                                Datum::Obj(_) | Datum::Null => None,
                                d => Some((f, d.to_owned())),
                            })
                            .collect();
                        (class, values)
                    });
                (
                    g.get_ref(sp, mf::MeasurementSpecification_metricDescription),
                    matches!(
                        g.get(sp, mf::MeasurementSpecification_triggersSelfAdaptations),
                        Datum::Bool(true)
                    ),
                    pt,
                )
            })
            .collect();
        let st = &mut self.graph;
        let mut n = 0;
        let mut sub = || {
            n += 1;
            format!("{id}_{n}")
        };
        let mon = st.create(mc::Monitor)?;
        st.set(mon, mf::identifier_Identifier_id, Datum::Str(id))?;
        st.set(
            mon,
            mf::NamedElement_entityName,
            Datum::Str(&format!("{name}-copy")),
        )?;
        st.set(mon, mf::Monitor_activated, Datum::Bool(activated))?;
        for (metric, triggers, pt) in &specs {
            let sp = st.create(mc::MeasurementSpecification)?;
            st.set(sp, mf::identifier_Identifier_id, Datum::Str(&sub()))?;
            if let Some(md) = metric {
                st.set(
                    sp,
                    mf::MeasurementSpecification_metricDescription,
                    Datum::Obj(*md),
                )?;
            }
            st.set(
                sp,
                mf::MeasurementSpecification_triggersSelfAdaptations,
                Datum::Bool(*triggers),
            )?;
            if let Some((pclass, values)) = pt {
                let p = st.create(*pclass)?;
                st.set(p, mf::identifier_Identifier_id, Datum::Str(&sub()))?;
                for (f, v) in values {
                    st.set(p, *f, v.as_datum())?;
                }
                st.set(
                    sp,
                    mf::MeasurementSpecification_processingType,
                    Datum::Obj(p),
                )?;
            }
            st.add(
                mon,
                mf::Monitor_measurementSpecifications,
                Datum::Obj(sp),
                None,
            )?;
        }
        let new_mp = st.create(class)?;
        if class == mc::ResourceContainerMeasuringPoint {
            st.set(
                new_mp,
                mf::ResourceContainerReference_resourceContainer,
                Datum::Obj(target),
            )?;
        } else {
            st.set(
                new_mp,
                mf::ActiveResourceReference_activeResource,
                Datum::Obj(target),
            )?;
            st.set(
                new_mp,
                mf::ActiveResourceReference_replicaID,
                Datum::Int(replica),
            )?;
        }
        if let Some((r, f)) = mp_repo {
            st.add(r, f, Datum::Obj(new_mp), None)?;
        }
        st.set(mon, mf::Monitor_measuringPoint, Datum::Obj(new_mp))?;
        st.add(repo, repo_f, Datum::Obj(mon), None)?;
        Ok(mon)
    }

    /// Removes a resource container from its resource environment (the references to it stay).
    pub fn remove_from_environment(&mut self, container: ObjId) -> R<()> {
        let Some((env, f)) = self.graph[container].container else {
            return Err(EditError(
                "the container is not in a resource environment".into(),
            ));
        };
        self.graph.remove(env, f, Datum::Obj(container))?;
        Ok(())
    }
}

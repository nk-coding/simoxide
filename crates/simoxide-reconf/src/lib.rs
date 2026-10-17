//! Runtime model changes of SimOxide runs, from the QVTo side (`DESIGN-SPD.md` 5.3 and 5.5).
//!
//! The boundary to QVTo is the API of the `qvto-simoxide` adapter: a [`Session`] whose
//! [`SimStore`] (a `qvto_model::ModelStore` over SimOxide's `raw::Graph`) records every change
//! in a [`Journal`]. Both kinds of change source go through it:
//!
//! - QVTo transformations: the host ([`QvtoCore`] shared by all runs, [`QvtoRun`] per run,
//!   created on first use), SimuLizar's rule folders ([`RuleSet`], [`QvtoRun::run_rules`]) and
//!   SPD adjustments ([`QvtoAdjuster`], feature `spd`);
//! - direct edits ([`Edit`]): a small API over the same store, for tests, snapshots and tools.
//!
//! [`change_set`] turns a journal into the simulator's [`ChangeSet`]; [`apply_journal`] and
//! [`Edit::apply`] hand the changed graph and its change set to
//! [`Simulation::apply_changes`], which recompiles the run's model (rebuild and diff).
//!
//! ```no_run
//! use simoxide_reconf::{Edit, Session};
//! use simoxide_sim::{CompiledModel, Outputs, Pause, SimConfig, Simulation};
//! let cm = CompiledModel::compile(simoxide_model::load_dir("corpus/h19_linking_resource").unwrap()).unwrap();
//! let cfg = SimConfig { max_measurements: 1000, ..Default::default() };
//! let mut sim = Simulation::new(&cm, cfg, Outputs::default()).unwrap();
//! sim.run_until(Pause::After(50_000_000_000)).unwrap();
//! let mut session = Session::new();
//! let mut e = Edit::new(&sim, &mut session);
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
//!
//! It uses the QVTo adapter `qvto-simoxide` (crates/qvto) until the merge's steps M5/M6 move the
//! journal and the store into the shared EMF graph; the API seen here stays.

use qvto_model::{
    ClassId as QClass, Datum, FeatureId as QFeature, ModelError, ModelStore, OwnedDatum,
};
use simoxide_model::meta::prelude::*;
use simoxide_model::meta::{ClassId, FeatureId, class as mc, feat as mf};
use simoxide_model::raw::{Graph, ObjId, PcmGraph, Value};
use simoxide_sim::{
    Change, ChangeError, ChangeKind, ChangeReport, ChangeSet, Compat, Flavour, Simulation,
};

pub use qvto_simoxide::{EXT_BASE, IdPolicy, Journal, Session, SimStore};

#[cfg(feature = "spd")]
mod adjust;
mod host;
mod qvt;
mod rules;

#[cfg(feature = "spd")]
pub use adjust::{AdjustHooks, DefaultAdjustHooks, QvtoAdjuster};
pub use host::{
    AbortPolicy, Bind, CoreBuilder, FailedRunChanges, IdSourceFn, Imports, Invocation, InvokeHooks,
    ModelRef, NsBinding, Outcome, ParamDir, ParamInfo, QvtMode, QvtoCore, QvtoRun, ReconfError,
    RunConfig, Transformation, palladio_metamodels, resolve_mode,
};
pub use rules::{RuleSet, RulesOutcome};

/// The simulator's change set of a journal: the changes of graph objects (handles below
/// [`EXT_BASE`]), in order, with the object values of list changes.
pub fn change_set(j: &Journal) -> ChangeSet {
    use qvto_simoxide::ChangeKind as J;
    let obj = |d: &OwnedDatum<u32>| match d {
        OwnedDatum::Obj(x) if *x < EXT_BASE => Some(ObjId(*x)),
        _ => None,
    };
    let f = |f: &QFeature| FeatureId(f.0);
    let mut cs = ChangeSet::new();
    for c in &j.changes {
        if c.obj >= EXT_BASE {
            continue;
        }
        let kind = match &c.kind {
            J::Created(class) => ChangeKind::Created(ClassId(class.0)),
            J::Set { feature, .. } => ChangeKind::Set {
                feature: f(feature),
            },
            J::Unset { feature, .. } => ChangeKind::Unset {
                feature: f(feature),
            },
            J::Added { feature, value, .. } => ChangeKind::Added {
                feature: f(feature),
                value: obj(value),
            },
            J::Removed { feature, value, .. } => ChangeKind::Removed {
                feature: f(feature),
                value: obj(value),
            },
            J::Replaced { feature, .. } => ChangeKind::Replaced {
                feature: f(feature),
            },
            J::Moved { feature, .. } => ChangeKind::Moved {
                feature: f(feature),
            },
            J::Container { .. } => ChangeKind::Container,
            J::RootAdded { .. } | J::RootRemoved { .. } => ChangeKind::Root,
            J::Deleted => ChangeKind::Deleted,
        };
        cs.changes.push(Change {
            obj: ObjId(c.obj),
            kind,
        });
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
    sim.apply_changes(graph, &change_set(journal))
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

/// Direct edits of a run's model through the adapter's store, recorded in the session's
/// journal: a copy of the run's graph, changed by [`Edit::store`] (any `ModelStore` operation)
/// or the helpers, then applied with [`Edit::apply`]. Ids of new objects are given by the
/// caller (they appear in traces and decide the resource registry's order).
pub struct Edit<'s> {
    graph: Graph,
    session: &'s mut Session,
}

fn qc(c: ClassId) -> QClass {
    QClass(c.0)
}

fn qf(f: FeatureId) -> QFeature {
    QFeature(f.0)
}

impl<'s> Edit<'s> {
    /// Starts an edit of `sim`'s current model; `session` records the journal (a session
    /// belongs to the run's graph: one per run, or a new one per edit).
    pub fn new<C: Compat, F: Flavour>(
        sim: &Simulation<'_, C, F>,
        session: &'s mut Session,
    ) -> Edit<'s> {
        session.start_journal();
        Edit {
            graph: sim.model_graph().clone(),
            session,
        }
    }

    /// The graph as edited so far.
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    /// The store over the edited graph (every change is recorded).
    pub fn store(&mut self) -> SimStore<'_> {
        self.session.store(&mut self.graph)
    }

    /// The graph and the change set of the edits.
    pub fn finish(self) -> (Graph, ChangeSet) {
        let j = self.session.stop_journal().unwrap_or_default();
        (self.graph, change_set(&j))
    }

    /// Applies the edits to `sim` ([`Simulation::apply_changes`]).
    pub fn apply<C: Compat, F: Flavour>(
        self,
        sim: &mut Simulation<'_, C, F>,
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
        let mut st = self.store();
        let rv = match st.get(owner.0, qf(feature)) {
            Datum::Obj(o) => o,
            _ => {
                let rv = st.create(qc(mc::PCMRandomVariable))?;
                st.set(owner.0, qf(feature), Datum::Obj(rv))?;
                rv
            }
        };
        st.set(rv, qf(mf::RandomVariable_specification), Datum::Str(spec))?;
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
                .filter(|&f| g.is_set(p, f))
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
        let mut st = self.store();
        let c = st.create(qc(mc::ResourceContainer))?;
        st.set(c, qf(mf::identifier_Identifier_id), Datum::Str(id))?;
        st.set(c, qf(mf::NamedElement_entityName), Datum::Str(name))?;
        for (n, s) in specs.iter().enumerate() {
            let p = st.create(qc(s.class))?;
            st.set(
                p,
                qf(mf::identifier_Identifier_id),
                Datum::Str(&format!("{id}_{n}")),
            )?;
            for (f, v) in &s.values {
                let d = match v {
                    Value::Int(i) => Datum::Int(*i as i32),
                    Value::Double(x) => Datum::Double(*x),
                    Value::Bool(b) => Datum::Bool(*b),
                    _ => continue,
                };
                st.set(p, qf(*f), d)?;
            }
            for (f, x) in &s.refs {
                st.set(p, qf(*f), Datum::Obj(x.0))?;
            }
            if let Some(rate) = &s.rate {
                let rv = st.create(qc(mc::PCMRandomVariable))?;
                st.set(rv, qf(mf::RandomVariable_specification), Datum::Str(rate))?;
                st.set(
                    p,
                    qf(mf::ProcessingResourceSpecification_processingRate_ProcessingResourceSpecification),
                    Datum::Obj(rv),
                )?;
            }
            st.add(
                c,
                qf(mf::ResourceContainer_activeResourceSpecifications_ResourceContainer),
                Datum::Obj(p),
                None,
            )?;
        }
        st.add(
            env.0,
            qf(mf::ResourceEnvironment_resourceContainer_ResourceEnvironment),
            Datum::Obj(c),
            None,
        )?;
        Ok(ObjId(c))
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
        let mut st = self.store();
        let a = st.create(qc(mc::AssemblyContext))?;
        st.set(a, qf(mf::identifier_Identifier_id), Datum::Str(id))?;
        st.set(a, qf(mf::NamedElement_entityName), Datum::Str(name))?;
        if let Some(c) = component {
            st.set(
                a,
                qf(mf::AssemblyContext_encapsulatedComponent__AssemblyContext),
                Datum::Obj(c.0),
            )?;
        }
        st.add(structure.0, qf(f), Datum::Obj(a), None)?;
        Ok(ObjId(a))
    }

    /// Sets a single-valued reference (e.g. an assembly connector's providing assembly context).
    pub fn set_ref(&mut self, o: ObjId, feature: FeatureId, target: ObjId) -> R<()> {
        self.store().set(o.0, qf(feature), Datum::Obj(target.0))?;
        Ok(())
    }

    /// Sets a `double` attribute (e.g. a branch probability).
    pub fn set_double(&mut self, o: ObjId, feature: FeatureId, v: f64) -> R<()> {
        self.store().set(o.0, qf(feature), Datum::Double(v))?;
        Ok(())
    }

    /// Connects `container` to the linking resource `link`.
    pub fn connect(&mut self, link: ObjId, container: ObjId) -> R<()> {
        self.store().add(
            link.0,
            qf(mf::LinkingResource_connectedResourceContainers_LinkingResource),
            Datum::Obj(container.0),
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
        let mut st = self.store();
        let a = st.create(qc(mc::AllocationContext))?;
        st.set(a, qf(mf::identifier_Identifier_id), Datum::Str(id))?;
        st.set(a, qf(mf::NamedElement_entityName), Datum::Str(id))?;
        st.set(
            a,
            qf(mf::AllocationContext_resourceContainer_AllocationContext),
            Datum::Obj(container.0),
        )?;
        st.set(
            a,
            qf(mf::AllocationContext_assemblyContext_AllocationContext),
            Datum::Obj(assembly.0),
        )?;
        st.add(
            allocation.0,
            qf(mf::Allocation_allocationContexts_Allocation),
            Datum::Obj(a),
            None,
        )?;
        Ok(ObjId(a))
    }

    /// `EcoreUtil.delete` of an object: it leaves its container, and every reference to it is
    /// removed (or unset: for a resource container referenced by an allocation context that is
    /// an `UNSET` notification, on which SimuLizar's allocation syncer aborts).
    pub fn delete(&mut self, o: ObjId) {
        self.store().delete(o.0);
    }

    /// Removes a resource container from its resource environment (the references to it stay).
    pub fn remove_from_environment(&mut self, container: ObjId) -> R<()> {
        let Some((env, f)) = self.graph[container].container else {
            return Err(EditError(
                "the container is not in a resource environment".into(),
            ));
        };
        self.store().remove(env.0, qf(f), Datum::Obj(container.0))?;
        Ok(())
    }
}

//! Runtime model changes (`DESIGN-SPD.md` 5.3, phase F2): change sets and the first syncer,
//! *rebuild and diff*.
//!
//! A running simulation reads only its [`CompiledModel`]. To change the model of a run between
//! two events (`Simulation::apply_changes`), the run's model graph is edited (by a QVTo
//! transformation on the model graph (the shared EMF graph), or by direct edits through the same
//! adapter), the edits are described by a [`ChangeSet`] (the adapter's change journal), and
//! [`recompile`] turns the changed graph into the run's next compiled model:
//!
//! 1. the typed model is rebuilt so that every element keeps its typed id
//!    (`simoxide_model::build_from_graph_after`): new elements are appended, removed ones stay as
//!    tombstones;
//! 2. it is compiled again, starting from the old model's frame keys, StoEx programs and the
//!    run's measurement series, which keep their ids ([`CompiledModel::compile_seeded`]);
//! 3. the fresh compilation is merged into the old one ("diff"): every index the run state
//!    holds (resources, assembly-context paths, instruction positions, series, programs, keys)
//!    keeps its meaning, new entities are appended, removed ones keep their entries.
//!
//! What a change does follows SimuLizar 5.2.2's syncers (`ResourceEnvironmentSyncer`,
//! `AllocationLookupSyncer`, `ResourceEnvironmentObservingLegacyRouter`, `UsageModelSyncer`) and
//! its interpreter, which reads the system, the repository and the allocation lookup at every
//! call, including their gaps (compat behaviour). The rules are listed on [`recompile`] and in
//! `docs/spec/model-changes.md` (MC-*).

use crate::fxhash::FxMap;
use crate::ir::*;
use simoxide_model::meta::prelude::*;
use simoxide_model::meta::{ClassId, FeatureId, class as mc, feat as mf};
use simoxide_model::raw::{Graph, ObjId, PcmGraph};
use simoxide_model::*;
use std::collections::HashSet;
use std::fmt;

/// The changes of a change set, in the order they were made (the change journal of the
/// model graph's journal, or of direct edits). Objects are graph objects; changes
/// of objects outside the graph (the adapter's other home: runtime measurements, SPD models)
/// are not part of it.
///
/// [`recompile`] works from the changed graph (rebuild and diff); the change set tells whether
/// anything changed at all, and carries what only the notifications show (SimuLizar's syncers
/// react to notification kinds, e.g. they abort on some).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChangeSet {
    pub changes: Vec<Change>,
}

/// One change of one graph object.
#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    pub obj: ObjId,
    pub kind: ChangeKind,
}

/// The kinds of the adapter's journal (EMF notifications), without the values except the
/// object values of list changes.
#[derive(Clone, Debug, PartialEq)]
pub enum ChangeKind {
    /// A new object.
    Created(ClassId),
    /// A single-valued feature got a new value.
    Set { feature: FeatureId },
    /// A single-valued feature was unset.
    Unset { feature: FeatureId },
    /// A value was added to a many-valued feature (`value`: the object, for references).
    Added {
        feature: FeatureId,
        value: Option<ObjId>,
    },
    /// A value was removed from a many-valued feature.
    Removed {
        feature: FeatureId,
        value: Option<ObjId>,
    },
    /// A value of a many-valued feature was replaced (`EList.set`).
    Replaced { feature: FeatureId },
    /// `EList.move`.
    Moved { feature: FeatureId },
    /// The object's container changed.
    Container,
    /// The object became, or stopped being, a root of a resource.
    Root,
    /// `EcoreUtil.delete` of the object.
    Deleted,
}

impl ChangeSet {
    pub fn new() -> ChangeSet {
        ChangeSet::default()
    }
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
    pub fn push(&mut self, obj: ObjId, kind: ChangeKind) {
        self.changes.push(Change { obj, kind });
    }
}

/// What applying a change set did.
#[derive(Clone, Debug, Default)]
pub struct ChangeReport {
    /// The model generation of the run afterwards (0: the base model; +1 per applied change set
    /// that changed anything).
    pub generation: u32,
    /// What the syncers did, one line each.
    pub applied: Vec<String>,
    /// Changes without effect, because SimuLizar's syncers do not handle them (compat).
    pub ignored: Vec<String>,
    /// Resources created (indices into `CompiledModel::resources`).
    pub new_resources: Vec<ResIdx>,
    /// The behaviours changed: new executions run new instructions (appended), executions in
    /// progress continue on the old ones.
    pub code_appended: bool,
    /// Wall time of the rebuild, recompilation and merge (µs).
    pub micros: u64,
}

/// Why a change set was not applied. The run state is unchanged in every case.
#[derive(Clone, Debug, PartialEq)]
pub enum ChangeError {
    /// The run cannot take changes (finished or failed).
    State(String),
    /// The changed model is not valid (validation or compilation errors), or the graph is not a
    /// changed copy of the run's model graph.
    Invalid(Vec<String>),
    /// Changes the rebuild-and-diff syncer does not handle yet (monitors, workloads, structural
    /// behaviour changes in progress; `DESIGN-SPD.md` F4).
    Unsupported(Vec<String>),
    /// SimuLizar aborts the simulation on this change (one of its syncers throws); the message
    /// names the exception. A reconfiguration process (R1) turns this into a failed run.
    Abort(String),
}

impl fmt::Display for ChangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChangeError::State(m) => write!(f, "cannot change the model: {m}"),
            ChangeError::Invalid(v) => write!(f, "invalid model change: {}", v.join("; ")),
            ChangeError::Unsupported(v) => {
                write!(f, "unsupported model change: {}", v.join("; "))
            }
            ChangeError::Abort(m) => write!(f, "SimuLizar aborts on this model change: {m}"),
        }
    }
}

impl std::error::Error for ChangeError {}

/// How a run takes model changes (`Simulation::set_change_options`; part of the run state, so
/// forks and snapshots keep it). The defaults ([`ChangeOptions::for_flavour`]) are the fixed
/// behaviours of `DESIGN-SPD.md` §5.3 and U13; each option names the reference behaviour it
/// reproduces (`docs/spec/model-changes.md`, MC-*).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct ChangeOptions {
    /// What happens to the resources of a container (or a processing resource) that leaves
    /// the resource environment (MC-1.5).
    pub removal: Removal,
    /// How executions inside a changed behaviour continue (MC-4.3).
    pub in_progress: InProgress,
    /// Whether a changed allocation context (another container or assembly context) moves its
    /// lookup entries (MC-3.2).
    pub reallocation: Reallocation,
    /// Which new monitors get measurement series (MC-6).
    pub monitors: MonitorSync,
}

/// The removal policy (MC-1.5, MC-8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub enum Removal {
    /// SimuLizar 5.2.2: the resources keep simulating, allocation lookups that lead to them
    /// still work, they take part in the final deactivation. The SimuLizar flavour also aborts
    /// on the changes its syncers throw on (MC-2.3, MC-3.3), as SimuLizar does.
    Keep,
    /// The default: the resources finish the jobs they have and serve the requests that were in
    /// progress at the change (their later demands too); a request that starts after the change
    /// and still reaches them is aborted (MC-8.3). Once idle with no such request left they are
    /// gone (no final deactivation tuples). A removed allocation context keeps its lookup
    /// entries for the requests in progress.
    Drain,
    /// The jobs at the resources are aborted at the change (their requests end: a closed
    /// user thinks and starts again, an open user is dropped, a forked behaviour ends), every
    /// later demand there aborts its request; the resources are gone.
    Abort,
    /// Slingshot (quirk Q5, SS-SPD-5.4): the job whose completion is pending is aborted at that
    /// completion, the other jobs at the resource are lost (their requests never continue);
    /// later demands abort their requests; a removed allocation context's lookups end at once.
    SlingshotQ5,
}

/// Executions in progress inside a changed behaviour (MC-4.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub enum InProgress {
    /// SimuLizar (the default): the interpreter reads an action's successor after the action,
    /// so an execution follows the changed chain at once: it finishes the action it is in (at
    /// the same position of the changed action) and continues with the action's successor in
    /// the changed behaviour.
    Follow,
    /// An execution in progress finishes the instructions it started with; new executions run
    /// the changed behaviour (F2's behaviour, an option).
    Finish,
}

/// A changed allocation context (MC-3.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub enum Reallocation {
    /// SimuLizar 5.2.2's `AllocationLookupSyncer`: ignored (the default in the SimuLizar
    /// flavour).
    Ignore,
    /// The context's lookup entries are recomputed (later entries overwrite, MC-3.1): requests
    /// use the new container from their next demand on (the default in the Slingshot flavour,
    /// whose lookup reads the allocation at every demand).
    Apply,
}

/// New monitors (MC-6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub enum MonitorSync {
    /// The flavour's reference (the default). SimuLizar: `ResourceEnvironmentSyncer.
    /// attachMonitors` gives a *new* active resource the state, overall-utilisation and demand
    /// calculators of the monitors that refer to its specification; every other new monitor is
    /// ignored (no sliding windows, PRM recorders or response-time calculators after the start).
    /// Slingshot: every new monitor of the monitor repository is interpreted (its calculators
    /// probe from then on, `MonitorRepositoryInterpreterBehavior.onModelAdjusted`).
    Reference,
    /// Every new measurement specification gets what it would get at the start: series,
    /// sliding windows (from the change on), PRM recorders, reconfiguration-time and
    /// container-count series (SimuLizar flavour; an extension).
    Full,
}

impl ChangeOptions {
    /// The defaults of a flavour: drain (Slingshot with Q5: Slingshot's removal), follow,
    /// SimuLizar ignores reallocations and Slingshot applies them, the reference's monitors.
    pub fn for_flavour(f: crate::flavour::FlavourConfig) -> ChangeOptions {
        let ss = f.kind.is_slingshot();
        ChangeOptions {
            removal: if ss && f.has(crate::flavour::Quirk::Q5) {
                Removal::SlingshotQ5
            } else {
                Removal::Drain
            },
            in_progress: InProgress::Follow,
            reallocation: if ss {
                Reallocation::Apply
            } else {
                Reallocation::Ignore
            },
            monitors: MonitorSync::Reference,
        }
    }

    /// SimuLizar 5.2.2's syncers in every respect (keep simulating, ignored reallocations, the
    /// reference's monitors, executions follow the changed chain).
    pub fn simulizar() -> ChangeOptions {
        ChangeOptions {
            removal: Removal::Keep,
            in_progress: InProgress::Follow,
            reallocation: Reallocation::Ignore,
            monitors: MonitorSync::Reference,
        }
    }
}

impl Default for ChangeOptions {
    fn default() -> Self {
        ChangeOptions::for_flavour(crate::flavour::FlavourConfig::SIMULIZAR)
    }
}

/// What the run state contributes to a recompilation: its series (the compiled ones and those
/// created during the run, in id order), the assembly-context paths it created and its change
/// options.
pub(crate) struct RunSeed<'a> {
    pub series: &'a [SeriesDef],
    pub dyn_paths: &'a [AcPath],
    pub opts: ChangeOptions,
}

/// A change of a workload driver, for the run state (`Simulation::adopt`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WorkloadOp {
    /// A new usage scenario (index into `CompiledModel::scenarios`): its driver starts.
    Start(u32),
    /// A removed usage scenario: its driver stops.
    Stop(u32),
    /// A closed workload's population changed.
    Population { scen: u32, to: i64 },
}

/// What the run state does after the merge (`Simulation::adopt`).
#[derive(Default)]
pub(crate) struct StateOps {
    /// Resources that left the resource environment with this change set (the removal
    /// policy decides what happens to them).
    pub removed: Vec<ResIdx>,
    /// Workload driver changes, in order.
    pub workloads: Vec<WorkloadOp>,
    /// The windows `cm.windows[first_new_window..]` are new (they start at the change).
    pub first_new_window: usize,
    /// The PRM recorders `cm.prm_series[first_new_prm.0..]` and runtime measurements
    /// `cm.prm_meas[first_new_prm.1..]` are new.
    pub first_new_prm: (usize, usize),
    /// Top-level containers of the resource environment after the change.
    pub containers: usize,
    /// The behaviours' instructions changed (new streams; executions in progress are moved
    /// onto them or keep theirs, `crate::sim::runmodel`).
    pub code_changed: bool,
}

/// The result of [`recompile`].
pub(crate) struct Recompiled {
    pub cm: CompiledModel,
    /// Build order of `cm.model` (for the next change set).
    pub order: Vec<ObjId>,
    /// Trie nodes of `cm.ac_paths` outside `ac_path_children` (references to the system
    /// assembly context): the run state looks them up with its own paths.
    pub path_extra: Vec<((u32, AcRef), u32)>,
    pub report: ChangeReport,
    pub ops: StateOps,
}

/// New measurement series of a fresh compilation that the run takes (ids from `base` on are
/// new; accepted ones are renumbered densely after the run's series).
struct SeriesMap {
    base: usize,
    to: Vec<Option<SeriesId>>,
}

impl SeriesMap {
    fn get(&self, s: SeriesId) -> Option<SeriesId> {
        if (s as usize) < self.base {
            Some(s)
        } else {
            self.to.get(s as usize - self.base).copied().flatten()
        }
    }
    fn opt(&self, s: Option<SeriesId>) -> Option<SeriesId> {
        s.and_then(|s| self.get(s))
    }
    fn vec(&self, v: &[SeriesId]) -> Vec<SeriesId> {
        v.iter().filter_map(|&s| self.get(s)).collect()
    }
}

/// The top-level containers of the allocation's resource environment.
fn top_level_containers(m: &Model) -> usize {
    let env = m
        .allocations
        .iter()
        .find_map(|a| a.environment)
        .or_else(|| m.resource_environments.ids().next());
    env.map_or(0, |e| m.resource_environments[e].containers.len())
}

/// The compiled model of a run after the change set `changes`: `graph` is the run's model graph
/// (`old.model.graph`, built in the order `old_order`) after the changes. The rules are
/// SimuLizar 5.2.2's syncers and interpreter (MC-1 to MC-7) with the run's
/// [`ChangeOptions`] (MC-8), or Slingshot's for a Slingshot-flavour run (SS-SPD-5):
///
/// * **Resource environment** (`ResourceEnvironmentSyncer`). A container added to the
///   environment gets its active resources (idle; registry order appended); a processing
///   resource added to a container gets one, unless the container has one of that type already,
///   whose rate then takes the new specification. A changed processing rate, link throughput or
///   latency takes effect for demands issued from then on (jobs in a queue keep their demand).
///   Changes of replicas, scheduling policy, resource type and HDD rates are ignored. Removed
///   containers and processing resources follow the removal policy ([`Removal`]); removed links
///   leave the routing. A new container whose id is registered already is not supported
///   (SimuLizar would not create it). *Slingshot*: resources exist for allocated containers
///   (SS-RES-1); new ones get theirs, with their monitors' probes.
/// * **Routing** (`ResourceEnvironmentObservingLegacyRouter`): links added to the environment are
///   appended to the routing order; removed links leave it (transmissions in progress complete);
///   containers newly connected to a link are routed over it; removed connections stay (the
///   router's removal is a no-op). `MOVE` and `set` of the link list or of a link's connected
///   containers abort under [`Removal::Keep`] and change nothing otherwise. *Slingshot*: the
///   links' connected containers are read live.
/// * **Allocation** (`AllocationLookupSyncer`): an added allocation context adds lookup entries for
///   its assembly context's path and every path nested below it (later entries overwrite);
///   changes of an existing allocation context's container or assembly context are ignored
///   ([`Reallocation::Ignore`]) or move its entries ([`Reallocation::Apply`]). Under
///   [`Removal::Keep`], removing an allocation context, `unset` or `move` in the allocation abort
///   (as SimuLizar's `NullPointerException` / `UnsupportedOperationException`); otherwise a
///   removed context's entries stay for the requests in progress. Lookups happen at every
///   demand, so requests in progress use the new entries from their next demand on. *Slingshot*:
///   the first allocation context of an assembly context wins, read live (removed contexts'
///   entries end at once only under [`Removal::SlingshotQ5`]).
/// * **System and repository** (read live by the interpreter): calls made after the change
///   resolve over the new connectors and assembly contexts; calls in progress continue where
///   they were resolved to. Changed actions take effect at their next execution; executions in
///   progress follow [`InProgress`].
/// * **Usage model** (`UsageModelSyncer`): changed think and inter-arrival times take effect at
///   the next draw; a population change starts new users or stops the oldest ones after their
///   current run (`ClosedWorkload.setPopulation`); a new usage scenario's workload driver starts,
///   a removed one's stops; a changed workload kind is ignored.
/// * **Monitors**: [`MonitorSync`].
pub(crate) fn recompile(
    old: &CompiledModel,
    old_order: &[ObjId],
    run: RunSeed<'_>,
    graph: Graph,
    changes: &ChangeSet,
) -> Result<Recompiled, ChangeError> {
    use crate::flavour::Quirk;
    let t0 = std::time::Instant::now();
    let opts = run.opts;
    let ss = old.flavour.kind.is_slingshot();
    // SimuLizar's syncers throw on some notifications (reproduced under `Keep`)
    let simulizar_aborts = !ss && opts.removal == Removal::Keep;
    check_graph(&old.model.graph, &graph)?;
    journal_checks(&graph, changes, simulizar_aborts)?;
    let mut report = ChangeReport::default();
    let mut ops = StateOps::default();

    // ------------------------------------------------------------ typed model and compile
    let (m2, order) =
        simoxide_model::build_from_graph_after(graph, Diagnostics::default(), old_order);
    let known: HashSet<(&str, &str, &str)> = old
        .model
        .diagnostics
        .iter()
        .filter(|d| d.level == Level::Error)
        .map(|d| (d.kind, &*d.location, &*d.message))
        .collect();
    let errors: Vec<String> = m2
        .diagnostics
        .iter()
        .filter(|d| {
            d.level == Level::Error && !known.contains(&(d.kind, &*d.location, &*d.message))
        })
        .map(|d| d.to_string())
        .collect();
    if !errors.is_empty() {
        return Err(ChangeError::Invalid(errors));
    }
    let seed = Seed {
        flavour: old.flavour,
        keys: old.keys.clone(),
        progs: old.progs.clone(),
        series: run.series.to_vec(),
    };
    let fresh =
        CompiledModel::compile_seeded(m2, seed).map_err(|e| ChangeError::Invalid(vec![e.0]))?;
    let m2 = &fresh.model;
    let m1 = &old.model;
    let base = run.series.len();
    // new monitors are honoured (Slingshot: every new monitor; SimuLizar: `MonitorSync::Full`)
    let honour = ss || opts.monitors == MonitorSync::Full;
    let mut unsupported = Vec::new();

    // ------------------------------------------------------------ usage model
    let fresh_scen: FxMap<UsageScenarioId, usize> = fresh
        .scenarios
        .iter()
        .enumerate()
        .map(|(j, s)| (s.id, j))
        .collect();
    let mut scenarios: Vec<CScenario> = Vec::with_capacity(fresh.scenarios.len());
    for (i, a) in old.scenarios.iter().enumerate() {
        let name = &fresh.scenario_ids[a.id.index()];
        let Some(&j) = fresh_scen.get(&a.id) else {
            let mut s = a.clone();
            if !s.removed {
                s.removed = true;
                ops.workloads.push(WorkloadOp::Stop(i as u32));
                report.applied.push(format!(
                    "usage scenario {name} removed: its workload driver stops"
                ));
            }
            scenarios.push(s);
            continue;
        };
        let b = &fresh.scenarios[j];
        let mut s = b.clone();
        s.removed = a.removed;
        if a.removed {
            report.ignored.push(format!(
                "usage scenario {name} was removed before: its workload driver stays stopped"
            ));
        }
        match (&a.workload, &b.workload) {
            (CWorkload::Open { inter_arrival: x }, CWorkload::Open { inter_arrival: y }) => {
                if x != y {
                    report.applied.push(format!(
                        "inter-arrival time of {name}: {} -> {}",
                        old.progs[*x as usize].spec, fresh.progs[*y as usize].spec
                    ));
                }
            }
            (
                CWorkload::Closed {
                    population: p,
                    think_time: x,
                },
                CWorkload::Closed {
                    population: q,
                    think_time: y,
                },
            ) => {
                if p != q {
                    if ss && old.flavour.has(Quirk::Q14) {
                        report.ignored.push(format!(
                            "closed population of {name} ({p} -> {q}): ignored (Slingshot Q14)"
                        ));
                        s.workload = CWorkload::Closed {
                            population: *p,
                            think_time: *y,
                        };
                    } else if !a.removed {
                        ops.workloads.push(WorkloadOp::Population {
                            scen: i as u32,
                            to: *q,
                        });
                        report
                            .applied
                            .push(format!("closed population of {name}: {p} -> {q}"));
                    }
                }
                if x != y {
                    report.applied.push(format!(
                        "think time of {name}: {} -> {}",
                        old.progs[*x as usize].spec, fresh.progs[*y as usize].spec
                    ));
                }
            }
            _ => {
                report.ignored.push(format!(
                    "workload kind of {name} changed (UsageModelSyncer has no resync strategy)"
                ));
                s.workload = a.workload.clone();
            }
        }
        scenarios.push(s);
    }
    for b in &fresh.scenarios {
        if old.scenarios.iter().any(|a| a.id == b.id) {
            continue;
        }
        ops.workloads
            .push(WorkloadOp::Start(scenarios.len() as u32));
        report.applied.push(format!(
            "new usage scenario {}: its workload driver starts",
            fresh.scenario_ids[b.id.index()]
        ));
        scenarios.push(b.clone());
    }

    // ------------------------------------------------------------ resources and registry
    let env2 = m2
        .allocations
        .iter()
        .find_map(|a| a.environment)
        .or_else(|| m2.resource_environments.ids().next());
    let n_cont = m2.containers.len();
    let mut resources = old.resources.clone();
    let mut container_res = old.container_res.clone();
    let mut container_simulated = old.container_simulated.clone();
    container_simulated.resize(n_cont, false);
    let mut link_res = old.link_res.clone();
    let mut registry = old.registry.clone();
    let mut reg_index: FxMap<&str, usize> = FxMap::default();
    for (i, (k, _)) in old.registry.iter().enumerate() {
        reg_index.entry(&**k).or_insert(i);
    }
    let by_kind: FxMap<ResKind, ResIdx> = old
        .resources
        .iter()
        .enumerate()
        .map(|(i, r)| (r.kind, i as ResIdx))
        .collect();
    // per merged resource: its index in the fresh compilation (monitors and Slingshot tables)
    let mut fresh_of: Vec<Option<usize>> = vec![None; old.resources.len()];
    let name = |r: &CRes| format!("{} of {}", r.type_id, r.rc);
    // a fresh resource for an existing one: rates follow, everything else is ignored
    let sync = |r: &mut CRes, f: &CRes, report: &mut ChangeReport| {
        if r.rate != f.rate {
            report.applied.push(format!(
                "rate of {}: {} -> {}",
                name(r),
                fresh.progs[r.rate as usize].spec,
                fresh.progs[f.rate as usize].spec
            ));
            r.rate = f.rate;
        }
        if matches!(r.kind, ResKind::Link(_)) && r.latency != f.latency {
            report.applied.push(format!("latency of {}", name(r)));
            r.latency = f.latency;
        }
        if r.instances != f.instances || r.policy != f.policy || r.hdd != f.hdd {
            report.ignored.push(format!(
                "{}: changed replicas, scheduling or HDD rates (ResourceEnvironmentSyncer syncs the processing rate only)",
                name(r)
            ));
        }
    };
    let mut new_entries: Vec<RegistryEntry> = Vec::new();
    // containers and links added and removed again in the change set: SimuLizar's syncer
    // created their resources at the addition (and never removes them)
    let journal_added = |feature: FeatureId| -> Vec<ObjId> {
        changes
            .changes
            .iter()
            .filter_map(|c| match c.kind {
                ChangeKind::Added {
                    feature: f,
                    value: Some(v),
                } if f == feature => Some(v),
                _ => None,
            })
            .collect()
    };
    let mut transient: Vec<ObjId> = Vec::new();
    if let Some(env) = env2 {
        let e = &m2.resource_environments[env];
        let live: HashSet<ObjId> = e
            .containers
            .iter()
            .map(|&c| m2.containers[c].obj)
            .chain(
                e.linking_resources
                    .iter()
                    .map(|&l| m2.linking_resources[l].obj),
            )
            .collect();
        for o in journal_added(mf::ResourceEnvironment_resourceContainer_ResourceEnvironment)
            .into_iter()
            .chain(journal_added(
                mf::ResourceEnvironment_linkingResources__ResourceEnvironment,
            ))
        {
            if !live.contains(&o) && !transient.contains(&o) {
                transient.push(o);
            }
        }
    }
    // the processing resource specifications in the environment after the change
    let live_specs: HashSet<ProcessingResourceId> = env2
        .map(|e| {
            m2.resource_environments[e]
                .containers
                .iter()
                .flat_map(|&c| m2.containers[c].processing_resources.iter().copied())
                .collect()
        })
        .unwrap_or_default();
    if ss {
        // Slingshot: the resources of the allocated containers (`ActiveResourceTable`); new
        // containers' resources are created, existing ones keep theirs (rates are read live)
        for (j, f) in fresh.resources.iter().enumerate() {
            if let ResKind::Link(_) = f.kind {
                continue;
            }
            if let Some(&i) = by_kind.get(&f.kind) {
                fresh_of[i as usize] = Some(j);
                sync(&mut resources[i as usize], f, &mut report);
                continue;
            }
            let ResKind::Processing(prid) = f.kind else {
                unreachable!()
            };
            let pr = &m2.processing_resources[prid];
            let (Some(cid), Some(rt)) = (pr.container, pr.resource_type) else {
                continue;
            };
            if let Some(&i) = container_res.get(&(cid, rt)) {
                // a later specification of the type replaces the earlier one: rates follow
                fresh_of[i as usize] = Some(j);
                sync(&mut resources[i as usize], f, &mut report);
                continue;
            }
            let i = resources.len() as ResIdx;
            resources.push(f.clone());
            fresh_of.push(Some(j));
            container_res.insert((cid, rt), i);
            report.new_resources.push(i);
            report.applied.push(format!("new resource {}", name(f)));
            let cont = &m2.containers[cid];
            if !container_simulated[cid.index()] {
                container_simulated[cid.index()] = true;
                report
                    .applied
                    .push(format!("new resource container {}", cont.id));
            }
            match registry.iter_mut().find(|e| *e.0 == *cont.id) {
                Some(e) => e.1.push((f.type_id.clone(), i)),
                None => registry.push((cont.id.clone(), vec![(f.type_id.clone(), i)])),
            }
        }
    } else if let Some(env) = env2 {
        let env_containers = m2.resource_environments[env].containers.clone();
        for &cid in &env_containers {
            let cont = &m2.containers[cid];
            let existing = old.container_simulated.get(cid.index()).copied() == Some(true);
            if !existing && reg_index.contains_key(&*cont.id) {
                unsupported.push(format!(
                    "new resource container {} has the id of a registered container or link (SimuLizar would not create it)",
                    cont.id
                ));
                continue;
            }
            container_simulated[cid.index()] = true;
            let mut types: Vec<(Box<str>, ResIdx)> = Vec::new();
            let mut seen: Vec<ResourceTypeId> = Vec::new();
            for &prid in &cont.processing_resources {
                let Some(rt) = m2.processing_resources[prid].resource_type else {
                    continue;
                };
                if seen.contains(&rt) {
                    continue;
                }
                seen.push(rt);
                let Some(&j) = fresh.container_res.get(&(cid, rt)) else {
                    continue;
                };
                let f = &fresh.resources[j as usize];
                if let Some(&i) = by_kind.get(&f.kind) {
                    // the same processing resource specification
                    fresh_of[i as usize] = Some(j as usize);
                    sync(&mut resources[i as usize], f, &mut report);
                    if container_res.get(&(cid, rt)) != Some(&i) {
                        report.ignored.push(format!(
                            "{}: changed resource type (not synced by ResourceEnvironmentSyncer)",
                            name(&resources[i as usize])
                        ));
                    }
                } else if let Some(&i) = old.container_res.get(&(cid, rt)) {
                    // createSimulatedActiveResource on a type the container has: syncProcessingResource
                    sync(&mut resources[i as usize], f, &mut report);
                } else {
                    let i = resources.len() as ResIdx;
                    resources.push(f.clone());
                    fresh_of.push(Some(j as usize));
                    container_res.insert((cid, rt), i);
                    types.push((f.type_id.clone(), i));
                    report.new_resources.push(i);
                    report.applied.push(format!("new resource {}", name(f)));
                }
            }
            if existing {
                if !types.is_empty() {
                    let e = reg_index[&*cont.id];
                    registry[e].1.extend(types);
                }
            } else {
                report
                    .applied
                    .push(format!("new resource container {}", cont.id));
                new_entries.push((cont.id.clone(), types));
            }
        }
    }
    if let Some(env) = env2 {
        for &lid in &m2.resource_environments[env].linking_resources.clone() {
            let j = fresh.link_res[&lid];
            let f = &fresh.resources[j as usize];
            match old.link_res.get(&lid) {
                Some(&i) => {
                    fresh_of[i as usize] = Some(j as usize);
                    sync(&mut resources[i as usize], f, &mut report);
                }
                None => {
                    let l = &m2.linking_resources[lid];
                    if reg_index.contains_key(&*l.id) {
                        unsupported.push(format!(
                            "new linking resource {} has the id of a registered container or link",
                            l.id
                        ));
                        continue;
                    }
                    let i = resources.len() as ResIdx;
                    resources.push(f.clone());
                    fresh_of.push(Some(j as usize));
                    link_res.insert(lid, i);
                    report.new_resources.push(i);
                    report
                        .applied
                        .push(format!("new linking resource {}", l.id));
                    new_entries.push((l.id.clone(), vec![(f.type_id.clone(), i)]));
                }
            }
        }
    }
    // added and removed again in this change set: SimuLizar created the resources at the
    // addition (`Keep` creates them: they keep simulating, unreachable); drained or aborted they
    // are gone before they served anything
    for &o in &transient {
        let what = m2.describe(o);
        if opts.removal == Removal::Keep && !ss {
            let created = transient_resources(
                old,
                &fresh,
                o,
                &mut resources,
                &mut fresh_of,
                &mut new_entries,
                &reg_index,
            );
            match created {
                Ok(n) => report.applied.push(format!(
                    "{what} was added and removed again in one change set: its {n} resources keep simulating (MC-1.5)"
                )),
                Err(e) => unsupported.push(e),
            }
        } else {
            report.ignored.push(format!(
                "{what} was added to the resource environment and removed again in one change set: no resources"
            ));
        }
    }
    registry.extend(new_entries);
    // removed: processing resources whose specification left the environment
    for (i, r) in old.resources.iter().enumerate() {
        match r.kind {
            ResKind::Processing(p) if !live_specs.contains(&p) => {
                ops.removed.push(i as ResIdx);
                if opts.removal == Removal::Keep {
                    report.ignored.push(format!(
                        "{} left the resource environment: it keeps simulating (ResourceEnvironmentSyncer does not handle removals)",
                        name(r)
                    ));
                } else {
                    report.applied.push(format!(
                        "{} left the resource environment ({:?})",
                        name(r),
                        opts.removal
                    ));
                }
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ measurement series
    // the new series the run takes, renumbered after the run's
    let mut take = vec![honour; fresh.series.len().saturating_sub(base)];
    if !honour {
        // SimuLizar: `attachMonitors` of new active resources
        for r in &resources[old.resources.len()..] {
            for &s in r
                .state_series
                .iter()
                .flatten()
                .chain(&r.demand_series)
                .chain(&r.overall_series)
            {
                if (s as usize) >= base {
                    take[s as usize - base] = true;
                }
            }
        }
    }
    let mut series: Vec<SeriesDef> = run.series.to_vec();
    let mut to = Vec::with_capacity(take.len());
    for (k, &t) in take.iter().enumerate() {
        if t {
            to.push(Some(series.len() as SeriesId));
            series.push(fresh.series[base + k].clone());
        } else {
            to.push(None);
        }
    }
    let smap = SeriesMap { base, to };
    if series.len() > base {
        let names: Vec<String> = series[base..]
            .iter()
            .map(|s| format!("{} {}", s.mp, s.metric))
            .collect();
        report
            .applied
            .push(format!("new measurement series: {}", names.join(", ")));
    }
    if fresh.series.len() - base > series.len() - base {
        report.ignored.push(format!(
            "{} new measurement series of monitors SimuLizar attaches only at the start (MC-6.1)",
            fresh.series.len() - series.len()
        ));
    }
    // a monitoring reference of an element: old elements keep theirs (new monitors honoured
    // add theirs; calculators are never removed), new elements get the fresh one if honoured
    let pick = |old_s: Option<SeriesId>, fresh_s: Option<SeriesId>| -> Option<SeriesId> {
        if honour {
            smap.opt(fresh_s).or(old_s)
        } else {
            old_s
        }
    };
    for (i, r) in resources.iter_mut().enumerate() {
        let new = i >= old.resources.len();
        if !(new || honour) {
            continue;
        }
        let Some(j) = fresh_of[i] else { continue };
        let f = &fresh.resources[j];
        if new {
            r.state_series = f.state_series.iter().map(|v| smap.vec(v)).collect();
            r.overall_series = smap.opt(f.overall_series);
            r.demand_series = smap.vec(&f.demand_series);
            r.initial_states = f
                .initial_states
                .iter()
                .filter_map(|&(s, k)| smap.get(s).map(|s| (s, k)))
                .collect();
        } else if r.instances == f.instances {
            for (a, b) in r.state_series.iter_mut().zip(&f.state_series) {
                for s in smap.vec(b) {
                    if !a.contains(&s) {
                        a.push(s);
                    }
                }
            }
            r.overall_series = r.overall_series.or(smap.opt(f.overall_series));
            for s in smap.vec(&f.demand_series) {
                if !r.demand_series.contains(&s) {
                    r.demand_series.push(s);
                }
            }
        }
    }
    let mut act = fresh.act.clone();
    for (i, a) in act.iter_mut().enumerate() {
        if let CAct::External { series, .. } = a {
            let old_s = match old.act.get(i) {
                Some(CAct::External { series, .. }) => *series,
                _ => None,
            };
            // new actions use existing calculators found by id (SimuLizar looks probes up
            // by id strings: a copy keeps its original's id), new series only if taken
            *series = if i < old.act.len() {
                pick(old_s, *series)
            } else {
                smap.opt(*series)
            };
        }
    }
    let mut uact = fresh.uact.clone();
    for (i, a) in uact.iter_mut().enumerate() {
        if let CUAct::Elsc {
            series,
            sysop_series,
            ..
        } = a
        {
            let (o1, o2) = match old.uact.get(i) {
                Some(CUAct::Elsc {
                    series,
                    sysop_series,
                    ..
                }) => (*series, *sysop_series),
                _ => (None, None),
            };
            if i < old.uact.len() {
                *series = pick(o1, *series);
                *sysop_series = pick(o2, *sysop_series);
            } else {
                *series = smap.opt(*series);
                *sysop_series = smap.opt(*sysop_series);
            }
        }
    }
    for (i, s) in scenarios.iter_mut().enumerate() {
        let fresh_s = fresh_scen
            .get(&s.id)
            .and_then(|&j| fresh.scenarios[j].series);
        s.series = match old.scenarios.get(i) {
            Some(a) => pick(a.series, fresh_s),
            None => smap.opt(fresh_s),
        };
    }
    let mut asmop_series = old.asmop_series.clone();
    if honour {
        for (k, &s) in &fresh.asmop_series {
            if let Some(s) = smap.get(s) {
                asmop_series.entry(k.clone()).or_insert(s);
            }
        }
    }
    let mut series_by_mp = old.series_by_mp.clone();
    for (i, d) in series.iter().enumerate().skip(base) {
        series_by_mp
            .entry(d.mp.clone())
            .or_default()
            .push((d.metric, i as SeriesId));
    }

    // sliding windows, PRM recorders, reconfiguration monitors: `MonitorSync::Full` only
    let mut windows = old.windows.clone();
    let mut prm_meas = old.prm_meas.clone();
    let mut prm_series = old.prm_series.clone();
    let mut reconf_time_series = old.reconf_time_series.clone();
    let mut container_count = old.container_count;
    let mut prm_any = old.prm_any;
    ops.first_new_window = windows.len();
    ops.first_new_prm = (prm_series.len(), prm_meas.len());
    if honour && !ss {
        let meas_map: Vec<u32> = fresh
            .prm_meas
            .iter()
            .map(|p| {
                match prm_meas
                    .iter()
                    .position(|q| q.spec == p.spec && q.mp == p.mp)
                {
                    Some(i) => i as u32,
                    None => {
                        prm_meas.push(p.clone());
                        (prm_meas.len() - 1) as u32
                    }
                }
            })
            .collect();
        let mmap = |i: u32| meas_map.get(i as usize).copied().unwrap_or(u32::MAX);
        for &(s, rec, mi) in &fresh.prm_series {
            if let Some(s) = smap.get(s) {
                let e = (s, rec, mmap(mi));
                if !prm_series.contains(&e) {
                    prm_series.push(e);
                }
            }
        }
        let mut matched = vec![false; old.windows.len()];
        for w in &fresh.windows {
            let w2 = CWindow {
                input: smap.opt(w.input),
                out: smap.opt(w.out),
                prm_meas: if w.prm_meas == u32::MAX {
                    u32::MAX
                } else {
                    mmap(w.prm_meas)
                },
                ..w.clone()
            };
            let same = |a: &CWindow| {
                a.len.to_bits() == w2.len.to_bits()
                    && a.inc.to_bits() == w2.inc.to_bits()
                    && a.input == w2.input
                    && a.out == w2.out
                    && a.prm == w2.prm
                    && a.stat == w2.stat
            };
            if let Some(i) = (0..old.windows.len()).find(|&i| !matched[i] && same(&old.windows[i]))
            {
                matched[i] = true;
            } else {
                report.applied.push(format!(
                    "new sliding window ({} s, every {} s)",
                    w2.len, w2.inc
                ));
                windows.push(w2);
            }
        }
        for &s in &fresh.reconf_time_series {
            if let Some(s) = smap.get(s)
                && !reconf_time_series.contains(&s)
            {
                reconf_time_series.push(s);
            }
        }
        if container_count.is_none() {
            container_count = fresh
                .container_count
                .and_then(|(s, n)| smap.get(s).map(|s| (s, n)));
        }
        prm_any |= fresh.prm_any;
    } else if !ss
        && (fresh.windows.len() != old.windows.len()
            || fresh.prm_meas.len() != old.prm_meas.len()
            || fresh.reconf_time_series.len() != old.reconf_time_series.len())
    {
        report.ignored.push(
            "new sliding windows, PRM recorders or reconfiguration monitors: SimuLizar creates them at the start only (MC-6.1)".into(),
        );
    }
    ops.containers = top_level_containers(m2);
    if let Some((s, _)) = container_count {
        container_count = Some((s, ops.containers));
    }

    // ------------------------------------------------------------ routing
    // (containers connected in the change set stay connected even if disconnected again:
    // the router's removal is a no-op)
    let container_of: FxMap<ObjId, ContainerId> =
        m2.containers.iter_ids().map(|(i, c)| (c.obj, i)).collect();
    // the old model's `connected` lists are the router's view (see below)
    let old_conn =
        |l: LinkingResourceId| -> Vec<ContainerId> { m1.linking_resources[l].connected.clone() };
    let env2_links: Vec<LinkingResourceId> = env2
        .map(|e| m2.resource_environments[e].linking_resources.clone())
        .unwrap_or_default();
    let mut links = Vec::new();
    let mut route_links: Vec<Box<[ContainerId]>> = Vec::new();
    for &l in &old.links {
        if !env2_links.contains(&l) {
            report.applied.push(format!(
                "linking resource {} left the routing",
                m1.linking_resources[l].id
            ));
            continue;
        }
        if ss {
            // Slingshot reads the connected containers live
            links.push(l);
            route_links.push(m2.linking_resources[l].connected.clone().into_boxed_slice());
            continue;
        }
        let mut c = old_conn(l);
        let lobj = m2.linking_resources[l].obj;
        let added_now: Vec<ContainerId> = changes
            .changes
            .iter()
            .filter_map(|ch| match ch.kind {
                ChangeKind::Added {
                    feature,
                    value: Some(v),
                } if ch.obj == lobj
                    && feature
                        == mf::LinkingResource_connectedResourceContainers_LinkingResource =>
                {
                    container_of.get(&v).copied()
                }
                _ => None,
            })
            .collect();
        for &x in m2.linking_resources[l].connected.iter().chain(&added_now) {
            if !c.contains(&x) {
                report.applied.push(format!(
                    "{} connected to {}",
                    m2.containers[x].id, m2.linking_resources[l].id
                ));
                c.push(x);
            }
        }
        if c.iter()
            .any(|x| !m2.linking_resources[l].connected.contains(x))
        {
            report.ignored.push(format!(
                "containers disconnected from {} stay routed (the router's removal is a no-op)",
                m2.linking_resources[l].id
            ));
        }
        links.push(l);
        route_links.push(c.into_boxed_slice());
    }
    for &l in &env2_links {
        if !links.contains(&l) && link_res.contains_key(&l) {
            let lobj = m2.linking_resources[l].obj;
            let mut c = m2.linking_resources[l].connected.clone();
            if !ss {
                for ch in &changes.changes {
                    if let ChangeKind::Added {
                        feature,
                        value: Some(v),
                    } = ch.kind
                        && ch.obj == lobj
                        && feature
                            == mf::LinkingResource_connectedResourceContainers_LinkingResource
                        && let Some(&x) = container_of.get(&v)
                        && !c.contains(&x)
                    {
                        c.push(x);
                    }
                }
            }
            links.push(l);
            route_links.push(c.into_boxed_slice());
        }
    }
    let route_res = route_table(
        n_cont,
        links
            .iter()
            .zip(&route_links)
            .map(|(l, c)| (link_res[l], &c[..])),
    );

    // ------------------------------------------------------------ allocation
    let ctxs = |m: &Model| -> Vec<AllocationContextId> {
        m.allocations
            .iter()
            .flat_map(|a| a.contexts.iter().copied())
            .collect()
    };
    let (c1, c2) = (ctxs(m1), ctxs(m2));
    for &gone in c1.iter().filter(|c| !c2.contains(c)) {
        if simulizar_aborts {
            return Err(ChangeError::Abort(format!(
                "NullPointerException: AllocationLookupSyncer cannot remove allocation context {}",
                m1.allocation_contexts[gone].id
            )));
        }
        report.applied.push(format!(
            "allocation context {} removed: {}",
            m1.allocation_contexts[gone].id,
            if opts.removal == Removal::SlingshotQ5 {
                "its lookups end"
            } else {
                "its lookup entries stay for the requests in progress"
            }
        ));
    }
    let sys_structure = m2.systems[fresh.system].structure;
    let alloc = if ss {
        // Slingshot: the first allocation context of an assembly context, read live
        let mut alloc = fresh.alloc.clone();
        for (path, &c) in &old.alloc {
            match alloc.get(path) {
                None if opts.removal != Removal::SlingshotQ5 => {
                    alloc.insert(path.clone(), c);
                }
                Some(&now) if now != c && opts.reallocation == Reallocation::Ignore => {
                    alloc.insert(path.clone(), c);
                }
                _ => {}
            }
        }
        for &ctx in &c2 {
            if !c1.contains(&ctx)
                && let (Some(ac), Some(c)) = (
                    m2.allocation_contexts[ctx].assembly,
                    m2.allocation_contexts[ctx].container,
                )
            {
                report.applied.push(format!(
                    "{} allocated to {}",
                    m2.assembly_contexts[ac].id, m2.containers[c].id
                ));
            }
        }
        alloc
    } else {
        let mut alloc = old.alloc.clone();
        for &ctx in &c2 {
            let b = &m2.allocation_contexts[ctx];
            if c1.contains(&ctx) {
                let a = &m1.allocation_contexts[ctx];
                if a.container != b.container || a.assembly != b.assembly {
                    match opts.reallocation {
                        Reallocation::Ignore => report.ignored.push(format!(
                            "allocation context {}: changed container or assembly context (AllocationLookupSyncer ignores it)",
                            b.id
                        )),
                        Reallocation::Apply => {
                            allocate_context(m2, sys_structure, ctx, &mut alloc).map_err(|e| {
                                ChangeError::Invalid(vec![format!(
                                    "allocation context {}: {}",
                                    b.id, e.0
                                )])
                            })?;
                            report.applied.push(format!(
                                "allocation context {} re-pointed{}",
                                b.id,
                                b.container
                                    .map(|c| format!(" to {}", m2.containers[c].id))
                                    .unwrap_or_default()
                            ));
                        }
                    }
                }
                continue;
            }
            if b.assembly.is_some() && b.container.is_none() {
                return Err(ChangeError::Invalid(vec![format!(
                    "new allocation context {} has no resource container",
                    b.id
                )]));
            }
            allocate_context(m2, sys_structure, ctx, &mut alloc)
                .map_err(|e| ChangeError::Abort(format!("IllegalStateException: {}", e.0)))?;
            if let (Some(ac), Some(c)) = (b.assembly, b.container) {
                report.applied.push(format!(
                    "{} allocated to {}",
                    m2.assembly_contexts[ac].id, m2.containers[c].id
                ));
            }
        }
        alloc
    };

    // ------------------------------------------------------------ assembly-context paths
    let mut ac_paths: Vec<AcPath> = old.ac_paths.clone();
    ac_paths.extend(run.dyn_paths.iter().cloned());
    let mut node_of: FxMap<(u32, AcRef), u32> = FxMap::default();
    for (i, p) in ac_paths.iter().enumerate().skip(1) {
        node_of.entry((p.parent, p.ac)).or_insert(i as u32);
    }
    let mut map = vec![ROOT_PATH; fresh.ac_paths.len()];
    for (j, p) in fresh.ac_paths.iter().enumerate().skip(1) {
        let parent = map[p.parent as usize];
        let id = *node_of.entry((parent, p.ac)).or_insert_with(|| {
            ac_paths.push(AcPath {
                parent,
                ac: p.ac,
                container: None,
            });
            (ac_paths.len() - 1) as u32
        });
        map[j] = id;
    }
    let mut path = Vec::new();
    for i in 0..ac_paths.len() {
        path.clear();
        let mut id = i as u32;
        while id != ROOT_PATH {
            path.push(ac_paths[id as usize].ac);
            id = ac_paths[id as usize].parent;
        }
        path.reverse();
        ac_paths[i].container = alloc.get(&path[..]).copied();
    }
    let mut ac_path_children: Vec<Vec<(u32, u32)>> = vec![Vec::new(); m2.assembly_contexts.len()];
    let mut path_extra = Vec::new();
    for (i, p) in ac_paths.iter().enumerate().skip(1) {
        match ac_path_children.get_mut(p.ac as usize) {
            Some(c) if !c.iter().any(|e| e.0 == p.parent) => c.push((p.parent, i as u32)),
            Some(_) => {}
            None => path_extra.push(((p.parent, p.ac), i as u32)),
        }
    }

    // ------------------------------------------------------------ Slingshot tables
    let ss_tables = if ss {
        let mut t = fresh.ss.clone();
        t.res_mon = (0..resources.len())
            .map(|i| match fresh_of[i] {
                Some(j) => {
                    let mut v: Vec<(SeriesId, SsProbe)> =
                        old.ss.res_mon.get(i).cloned().unwrap_or_default();
                    for &(s, p) in &fresh.ss.res_mon[j] {
                        if let Some(s) = smap.get(s)
                            && !v.contains(&(s, p))
                        {
                            v.push((s, p));
                        }
                    }
                    v
                }
                None => old.ss.res_mon.get(i).cloned().unwrap_or_default(),
            })
            .collect();
        t.link_failure = (0..resources.len())
            .map(|i| match (old.ss.link_failure.get(i), fresh_of[i]) {
                (Some(&p), _) => p,
                (None, Some(j)) => fresh.ss.link_failure.get(j).copied().unwrap_or(0.0),
                (None, None) => 0.0,
            })
            .collect();
        let mut pm = old.ss.passive_mon.clone();
        for p in &fresh.ss.passive_mon {
            if let Some(s) = smap.get(p.series)
                && !pm
                    .iter()
                    .any(|q| q.series == s && q.pr == p.pr && q.ac == p.ac)
            {
                pm.push(SsPassiveMon { series: s, ..*p });
            }
        }
        t.passive_mon = pm;
        t
    } else {
        fresh.ss.clone()
    };

    // ------------------------------------------------------------ code
    // (rebuilt from the merged tables: their series references; executions in progress are
    // moved onto the new streams by the run state, `crate::sim::runmodel`)
    let code = crate::code::Code::build(&fresh.beh, &act, &fresh.ubeh, &uact);
    ops.code_changed = code != old.code;
    if ops.code_changed {
        report.code_appended = true;
        report
            .applied
            .push("behaviours recompiled (new instructions)".into());
    }
    if !unsupported.is_empty() {
        return Err(ChangeError::Unsupported(unsupported));
    }

    // ------------------------------------------------------------ the merged model
    let mut warnings = old.warnings.clone();
    for w in &fresh.warnings {
        if !warnings.contains(w) {
            warnings.push(w.clone());
        }
    }
    let container_res_dense = dense_container_res(&container_res, n_cont, m2.resource_types.len());
    let finalise_order_fast = finalise_order::<simoxide_jcompat::order::InsertionOrder>(&registry);
    #[cfg(feature = "exact")]
    let finalise_order = finalise_order::<simoxide_jcompat_exact::order::JdkHashOrder>(&registry);
    // the router's view goes into the typed model (`transmit` without a route table reads it)
    let mut fresh = fresh;
    for (&l, c) in links.iter().zip(&route_links) {
        fresh.model.linking_resources[l].connected = c.to_vec();
    }
    let CompiledModel {
        model,
        keys,
        progs,
        beh,
        ubeh,
        system,
        prov_deleg,
        req_conn,
        seff_for,
        sig_key,
        passive_mon,
        comp_params,
        ac_params,
        pr_capacity,
        stream_bytesize,
        action_ids,
        uaction_ids,
        ac_ids,
        role_ids,
        sig_ids,
        scenario_ids,
        pr_ids,
        action_type,
        uaction_type,
        ..
    } = fresh;
    let cm = CompiledModel {
        model,
        keys,
        progs,
        beh,
        act,
        ubeh,
        uact,
        scenarios,
        system,
        prov_deleg,
        req_conn,
        seff_for,
        sig_key,
        alloc,
        ac_paths,
        ac_path_children,
        resources,
        container_res,
        container_res_dense,
        container_simulated,
        link_res,
        route_res,
        #[cfg(feature = "exact")]
        finalise_order,
        finalise_order_fast,
        links,
        registry,
        series,
        series_by_mp,
        windows,
        asmop_series,
        passive_mon,
        prm_series,
        prm_meas,
        prm_passive: old.prm_passive.clone(),
        prm_any,
        reconf_time_series,
        container_count,
        comp_params,
        ac_params,
        pr_capacity,
        stream_bytesize,
        action_ids,
        uaction_ids,
        ac_ids,
        role_ids,
        sig_ids,
        scenario_ids,
        pr_ids,
        action_type,
        uaction_type,
        code,
        warnings,
        flavour: old.flavour,
        ss: ss_tables,
    };
    report.micros = t0.elapsed().as_micros() as u64;
    Ok(Recompiled {
        cm,
        order,
        path_extra,
        report,
        ops,
    })
}

/// The resources SimuLizar's syncer created for `o`, a container or link added to the resource
/// environment and removed again in one change set (they keep simulating under
/// [`Removal::Keep`], MC-1.5): built by compiling the model with `o` put back into the
/// environment. Returns the number of resources.
fn transient_resources(
    _old: &CompiledModel,
    _fresh: &CompiledModel,
    o: ObjId,
    _resources: &mut Vec<CRes>,
    _fresh_of: &mut Vec<Option<usize>>,
    _new_entries: &mut Vec<RegistryEntry>,
    _reg_index: &FxMap<&str, usize>,
) -> Result<usize, String> {
    Err(format!(
        "object {} was added to the resource environment and removed again in one change set (Removal::Keep: not supported yet)",
        o.0
    ))
}

/// The new graph must be the run's graph with changes: objects are only ever appended and keep
/// their class.
fn check_graph(old: &Graph, new: &Graph) -> Result<(), ChangeError> {
    let bad = new.object_count() < old.object_count()
        || new.resources().len() < old.resources().len()
        || old.objects().zip(new.objects()).any(|(a, b)| {
            // objects of other metamodels (a QVTo host's state) may be placeholders on one side
            a.class != b.class
                && (simoxide_model::raw::is_pcm_class(a.class)
                    || simoxide_model::raw::is_pcm_class(b.class))
        });
    if bad {
        return Err(ChangeError::Invalid(vec![
            "the graph is not a changed copy of the run's model graph".into(),
        ]));
    }
    Ok(())
}

/// The aborts that only the notifications show.
fn journal_checks(g: &Graph, changes: &ChangeSet, aborts: bool) -> Result<(), ChangeError> {
    for c in &changes.changes {
        let Some(obj) = (c.obj.index() < g.object_count()).then(|| &g[c.obj]) else {
            return Err(ChangeError::Invalid(vec![format!(
                "change of object {} which is not in the graph",
                c.obj.0
            )]));
        };
        if !aborts {
            continue;
        }
        let in_allocation = obj.class.is_a(mc::Allocation) || obj.class.is_a(mc::AllocationContext);
        match c.kind {
            ChangeKind::Removed { feature, .. }
                if feature == mf::Allocation_allocationContexts_Allocation =>
            {
                return Err(ChangeError::Abort(format!(
                    "NullPointerException: AllocationLookupSyncer cannot remove an allocation context ({})",
                    g.describe(c.obj)
                )));
            }
            ChangeKind::Unset { .. } | ChangeKind::Moved { .. } if in_allocation => {
                return Err(ChangeError::Abort(format!(
                    "UnsupportedOperationException: AllocationLookupSyncer does not support this event type ({:?} of {})",
                    c.kind,
                    g.describe(c.obj)
                )));
            }
            ChangeKind::Moved { feature } | ChangeKind::Replaced { feature }
                if feature == mf::ResourceEnvironment_linkingResources__ResourceEnvironment
                    || feature
                        == mf::LinkingResource_connectedResourceContainers_LinkingResource =>
            {
                return Err(ChangeError::Abort(format!(
                    "UnsupportedOperationException: ResourceEnvironmentObservingLegacyRouter does not support this event type ({:?} of {})",
                    c.kind,
                    g.describe(c.obj)
                )));
            }
            _ => {}
        }
    }
    Ok(())
}

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

/// What the run state contributes to a recompilation: its series (the compiled ones and those
/// created during the run, in id order) and the assembly-context paths it created.
pub(crate) struct RunSeed<'a> {
    pub series: &'a [SeriesDef],
    pub dyn_paths: &'a [AcPath],
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
}

/// The compiled model of a run after the change set `changes`: `graph` is the run's model graph
/// (`old.model.graph`, built in the order `old_order`) after the changes.
///
/// Compat rules (SimuLizar 5.2.2's syncers and interpreter):
///
/// * **Resource environment** (`ResourceEnvironmentSyncer`). A container added to the
///   environment gets its active resources (idle; registry order appended); a processing
///   resource added to a container gets one, unless the container has one of that type already,
///   whose rate then takes the new specification. A changed processing rate, link throughput or
///   latency takes effect for demands issued from then on (jobs in a queue keep their demand).
///   Changes of replicas, scheduling policy, resource type and HDD rates are ignored. Removed
///   containers, processing and linking resources keep simulating: their resources stay, their
///   jobs complete, and allocation lookups that lead to them still work. A new container whose
///   id is registered already is not supported (SimuLizar would not create it).
/// * **Routing** (`ResourceEnvironmentObservingLegacyRouter`): links added to the environment are
///   appended to the routing order; removed links leave it (transmissions in progress complete);
///   containers newly connected to a link are routed over it; removed connections stay (the
///   router's removal is a no-op). `MOVE` and `set` of the link list or of a link's connected
///   containers abort.
/// * **Allocation** (`AllocationLookupSyncer`): an added allocation context adds lookup entries for
///   its assembly context's path and every path nested below it (later entries overwrite);
///   changes of an existing allocation context's container or assembly context are ignored;
///   removing an allocation context, `unset` or `move` in the allocation abort (as SimuLizar's
///   `NullPointerException` / `UnsupportedOperationException`). Lookups happen at every demand,
///   so requests in progress use the new entries from their next demand on.
/// * **System and repository** (read live by the interpreter): calls made after the change
///   resolve over the new connectors and assembly contexts; calls in progress continue where
///   they were resolved to. Changed actions take effect at their next execution; new executions
///   of a changed behaviour run its new instructions, an execution in progress finishes the
///   instructions it started with (a deviation: SimuLizar follows a changed successor chain at
///   once). A structural change of an action (its kind, or the number of infrastructure or
///   resource calls of an internal action) is not supported.
/// * **Usage model** (`UsageModelSyncer`): changed think and inter-arrival times take effect at
///   the next draw; population changes, new and removed scenarios are not supported.
/// * **Monitors**: changes that create or remove measurement series, sliding windows or PRM
///   recorders are not supported (F4: monitor syncing).
pub(crate) fn recompile(
    old: &CompiledModel,
    old_order: &[ObjId],
    run: RunSeed<'_>,
    graph: Graph,
    changes: &ChangeSet,
) -> Result<Recompiled, ChangeError> {
    let t0 = std::time::Instant::now();
    if old.flavour.kind != crate::flavour::FlavourKind::SimuLizar {
        // the Slingshot flavour's syncers (scale-in removal policies, cloned monitors) are F4/P3
        return Err(ChangeError::Unsupported(vec![
            "model changes of a Slingshot-flavour run (DESIGN-SPD.md F4, P3)".into(),
        ]));
    }
    check_graph(&old.model.graph, &graph)?;
    journal_checks(&graph, changes)?;
    let mut report = ChangeReport::default();

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

    // ------------------------------------------------------------ unsupported changes
    let mut unsupported = Vec::new();
    if fresh.series.len() != run.series.len() {
        let names: Vec<String> = fresh.series[run.series.len()..]
            .iter()
            .map(|s| format!("{} {}", s.mp, s.metric))
            .collect();
        unsupported.push(format!(
            "new measurement series ({}): monitor changes need F4's monitor syncer",
            names.join(", ")
        ));
    }
    let same_windows = old.windows.len() == fresh.windows.len()
        && old.windows.iter().zip(&fresh.windows).all(|(a, b)| {
            a.len.to_bits() == b.len.to_bits()
                && a.inc.to_bits() == b.inc.to_bits()
                && a.input == b.input
                && a.out == b.out
                && a.prm == b.prm
        });
    if !same_windows {
        unsupported.push("changed sliding windows (monitors): F4".to_string());
    }
    if old.prm_series != fresh.prm_series
        || old.prm_meas != fresh.prm_meas
        || old.prm_passive.len() != fresh.prm_passive.len()
        || old.prm_any != fresh.prm_any
        || old.reconf_time_series != fresh.reconf_time_series
        || old.container_count.map(|c| c.0) != fresh.container_count.map(|c| c.0)
    {
        unsupported
            .push("changed runtime-measurement (PRM) or reconfiguration monitors: F4".into());
    }
    if old.scenarios.len() != fresh.scenarios.len()
        || old
            .scenarios
            .iter()
            .zip(&fresh.scenarios)
            .any(|(a, b)| a.id != b.id)
    {
        unsupported.push("usage scenarios added or removed: F4".into());
    }
    for (a, b) in old.scenarios.iter().zip(&fresh.scenarios) {
        match (&a.workload, &b.workload) {
            (CWorkload::Open { inter_arrival: x }, CWorkload::Open { inter_arrival: y }) => {
                if x != y {
                    report.applied.push(format!(
                        "inter-arrival time of {}: {} -> {}",
                        old.scenario_ids[a.id.index()],
                        old.progs[*x as usize].spec,
                        fresh.progs[*y as usize].spec
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
                    unsupported.push(format!(
                        "closed population of {} changed ({p} -> {q}): F4",
                        old.scenario_ids[a.id.index()]
                    ));
                }
                if x != y {
                    report.applied.push(format!(
                        "think time of {}: {} -> {}",
                        old.scenario_ids[a.id.index()],
                        old.progs[*x as usize].spec,
                        fresh.progs[*y as usize].spec
                    ));
                }
            }
            _ => unsupported.push(format!(
                "workload kind of {} changed: F4",
                old.scenario_ids[a.id.index()]
            )),
        }
    }
    // actions that old instructions may still execute must keep their structure
    let code_changed = fresh.code.traced != old.code.traced || fresh.code.plain != old.code.plain;
    if code_changed {
        for (i, a) in old.act.iter().enumerate() {
            let b = &fresh.act[i];
            let same = match (a, b) {
                (
                    CAct::Internal {
                        infra: i1,
                        rescalls: r1,
                        ..
                    },
                    CAct::Internal {
                        infra: i2,
                        rescalls: r2,
                        ..
                    },
                ) => i1.len() == i2.len() && r1.len() == r2.len(),
                _ => std::mem::discriminant(a) == std::mem::discriminant(b),
            };
            if !same {
                unsupported.push(format!(
                    "structural change of action {} (its kind or number of calls): F4",
                    old.action_ids[i]
                ));
            }
        }
        for (i, a) in old.uact.iter().enumerate() {
            if std::mem::discriminant(a) != std::mem::discriminant(&fresh.uact[i]) {
                unsupported.push(format!(
                    "structural change of user action {}: F4",
                    old.uaction_ids[i]
                ));
            }
        }
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
    let mut reached = vec![false; old.resources.len()];
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
            if !live.contains(&o) {
                unsupported.push(format!(
                    "{} was added to the resource environment and removed again in one change set (SimuLizar would create its resources)",
                    m2.describe(o)
                ));
            }
        }
    }
    if let Some(env) = env2 {
        let env_containers = m2.resource_environments[env].containers.clone();
        let env_links = m2.resource_environments[env].linking_resources.clone();
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
                    reached[i as usize] = true;
                    sync(&mut resources[i as usize], f, &mut report);
                    if container_res.get(&(cid, rt)) != Some(&i) {
                        report.ignored.push(format!(
                            "{}: changed resource type (not synced by ResourceEnvironmentSyncer)",
                            name(&resources[i as usize])
                        ));
                    }
                } else if let Some(&i) = old.container_res.get(&(cid, rt)) {
                    // createSimulatedActiveResource on a type the container has: syncProcessingResource
                    reached[i as usize] = true;
                    sync(&mut resources[i as usize], f, &mut report);
                } else {
                    let i = resources.len() as ResIdx;
                    resources.push(f.clone());
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
        for &lid in &env_links {
            let j = fresh.link_res[&lid];
            let f = &fresh.resources[j as usize];
            match old.link_res.get(&lid) {
                Some(&i) => {
                    reached[i as usize] = true;
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
    registry.extend(new_entries);
    for (i, r) in old.resources.iter().enumerate() {
        if !reached[i] {
            report.ignored.push(format!(
                "{} left the resource environment: it keeps simulating (ResourceEnvironmentSyncer does not handle removals)",
                name(r)
            ));
        }
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
            for ch in &changes.changes {
                if let ChangeKind::Added {
                    feature,
                    value: Some(v),
                } = ch.kind
                    && ch.obj == lobj
                    && feature == mf::LinkingResource_connectedResourceContainers_LinkingResource
                    && let Some(&x) = container_of.get(&v)
                    && !c.contains(&x)
                {
                    c.push(x);
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
    let mut alloc = old.alloc.clone();
    let ctxs = |m: &Model| -> Vec<AllocationContextId> {
        m.allocations
            .iter()
            .flat_map(|a| a.contexts.iter().copied())
            .collect()
    };
    let (c1, c2) = (ctxs(m1), ctxs(m2));
    if let Some(&gone) = c1.iter().find(|c| !c2.contains(c)) {
        return Err(ChangeError::Abort(format!(
            "NullPointerException: AllocationLookupSyncer cannot remove allocation context {}",
            m1.allocation_contexts[gone].id
        )));
    }
    let sys_structure = m2.systems[fresh.system].structure;
    for &ctx in &c2 {
        let b = &m2.allocation_contexts[ctx];
        if c1.contains(&ctx) {
            let a = &m1.allocation_contexts[ctx];
            if a.container != b.container || a.assembly != b.assembly {
                report.ignored.push(format!(
                    "allocation context {}: changed container or assembly context (AllocationLookupSyncer ignores it)",
                    b.id
                ));
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

    // ------------------------------------------------------------ code
    let code = if code_changed {
        report.code_appended = true;
        report
            .applied
            .push("behaviours recompiled (new executions run the new instructions)".into());
        crate::code::Code {
            traced: old.code.traced.append(&fresh.code.traced),
            plain: old.code.plain.append(&fresh.code.plain),
        }
    } else {
        old.code.clone()
    };
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
        act,
        ubeh,
        uact,
        scenarios,
        system,
        prov_deleg,
        req_conn,
        seff_for,
        sig_key,
        series,
        series_by_mp,
        asmop_series,
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
        windows: old.windows.clone(),
        asmop_series,
        passive_mon,
        prm_series: old.prm_series.clone(),
        prm_meas: old.prm_meas.clone(),
        prm_passive: old.prm_passive.clone(),
        prm_any: old.prm_any,
        reconf_time_series: old.reconf_time_series.clone(),
        container_count: old.container_count,
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
        ss: old.ss.clone(),
    };
    report.micros = t0.elapsed().as_micros() as u64;
    Ok(Recompiled {
        cm,
        order,
        path_extra,
        report,
    })
}

/// The new graph must be the run's graph with changes: objects are only ever appended and keep
/// their class.
fn check_graph(old: &Graph, new: &Graph) -> Result<(), ChangeError> {
    let bad = new.object_count() < old.object_count()
        || new.resources().len() < old.resources().len()
        || old
            .objects()
            .iter()
            .zip(new.objects())
            .any(|(a, b)| a.class != b.class);
    if bad {
        return Err(ChangeError::Invalid(vec![
            "the graph is not a changed copy of the run's model graph".into(),
        ]));
    }
    Ok(())
}

/// The aborts that only the notifications show.
fn journal_checks(g: &Graph, changes: &ChangeSet) -> Result<(), ChangeError> {
    for c in &changes.changes {
        let Some(obj) = g.objects().get(c.obj.0 as usize) else {
            return Err(ChangeError::Invalid(vec![format!(
                "change of object {} which is not in the graph",
                c.obj.0
            )]));
        };
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

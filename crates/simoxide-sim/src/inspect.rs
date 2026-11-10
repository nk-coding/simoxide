//! Runtime model and measurement metadata for external analysis
//! (`docs/guide/runtime-metadata.md`): the *model file* of a compiled model (its definitions and
//! its deployment, with a content hash), the *deployment* of a run's current model, the
//! differences between two deployments, the *retired* elements (those a run's model still holds
//! after they left the deployment) and what each measurement series refers to.
//!
//! Everything here is a function of a model and a series list, both part of the run state, so a
//! paused and resumed run gives the metadata of the uninterrupted run. Element identities are
//! the model's ids (`id` of the PCM elements; the ids SPD adjustments and change files give the
//! elements they create).
//!
//! JSON objects are written with sorted keys; arrays keep model order.

use crate::ir::{CompiledModel, SeriesDef, metric_names, ss_mp_key, tuple_metric_name};
use serde_json::{Map, Value, json};
use simoxide_model::*;
use std::collections::{HashMap, HashSet};

/// `format` of the model file.
pub const MODEL_FORMAT: &str = "simoxide-model";
/// `format` of the runtime file.
pub const RUNTIME_FORMAT: &str = "simoxide-runtime";
/// `version` of both formats.
pub const VERSION: u32 = 1;

/// The element kinds of a deployment, in output order. `target_groups` are filled by the SPD
/// layer (`simoxide_spd::inspect`); without scaling policies the list is empty.
pub const KINDS: [&str; 5] = [
    "containers",
    "linking_resources",
    "assemblies",
    "allocations",
    "target_groups",
];

/// The deployment of a model: per kind of [`KINDS`] its elements (JSON objects with an `id`),
/// in model order.
#[derive(Clone, Debug, PartialEq)]
pub struct Deployment {
    pub kinds: Vec<(&'static str, Vec<Value>)>,
}

impl Default for Deployment {
    fn default() -> Self {
        Deployment {
            kinds: KINDS.iter().map(|k| (*k, Vec::new())).collect(),
        }
    }
}

impl Deployment {
    /// The elements of `kind` (one of [`KINDS`]).
    pub fn get(&self, kind: &str) -> &[Value] {
        self.kinds
            .iter()
            .find(|(k, _)| *k == kind)
            .map_or(&[], |(_, v)| v)
    }

    /// The elements of `kind`, to change or extend.
    pub fn get_mut(&mut self, kind: &str) -> &mut Vec<Value> {
        let i = self
            .kinds
            .iter()
            .position(|(k, _)| *k == kind)
            .expect("a deployment kind");
        &mut self.kinds[i].1
    }

    /// The element of `kind` with `id`.
    pub fn find_mut(&mut self, kind: &str, id: &str) -> Option<&mut Value> {
        self.get_mut(kind)
            .iter_mut()
            .find(|e| e["id"].as_str() == Some(id))
    }

    /// The deployment as one JSON object, a member per kind.
    pub fn to_json(&self) -> Value {
        let mut o = Map::new();
        for (k, v) in &self.kinds {
            o.insert((*k).into(), Value::Array(v.clone()));
        }
        Value::Object(o)
    }

    /// What changed from `self` to `new`: `added` and `changed` elements (as they are in
    /// `new`), `removed` ids, each per kind; kinds without entries are left out.
    pub fn diff(&self, new: &Deployment) -> Value {
        let (mut added, mut removed, mut changed) = (Map::new(), Map::new(), Map::new());
        for (kind, new_elems) in &new.kinds {
            let old_elems = self.get(kind);
            let old: HashMap<&str, &Value> = old_elems
                .iter()
                .map(|e| (e["id"].as_str().unwrap_or_default(), e))
                .collect();
            let ids: HashSet<&str> = new_elems
                .iter()
                .map(|e| e["id"].as_str().unwrap_or_default())
                .collect();
            let (mut a, mut c) = (Vec::new(), Vec::new());
            for e in new_elems {
                match old.get(e["id"].as_str().unwrap_or_default()) {
                    None => a.push(e.clone()),
                    Some(o) if *o != e => c.push(e.clone()),
                    Some(_) => {}
                }
            }
            let r: Vec<Value> = old_elems
                .iter()
                .filter(|e| !ids.contains(e["id"].as_str().unwrap_or_default()))
                .map(|e| e["id"].clone())
                .collect();
            for (map, list) in [(&mut added, a), (&mut removed, r), (&mut changed, c)] {
                if !list.is_empty() {
                    map.insert((*kind).into(), Value::Array(list));
                }
            }
        }
        json!({"added": added, "removed": removed, "changed": changed})
    }
}

/// FNV-1a 64 of `bytes`, as 16 hex digits (the hash of snapshot state ids).
pub fn fnv64_hex(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

// ---------------------------------------------------------------------------------------------
// the deployment

/// Constant values of StoEx specifications, from a compiled model's programs.
struct Konst<'a>(HashMap<&'a str, f64>);

impl<'a> Konst<'a> {
    fn of(cm: &'a CompiledModel) -> Konst<'a> {
        Konst(
            cm.progs
                .iter()
                .filter_map(|p| Some((&*p.spec, p.konst_f64?)))
                .collect(),
        )
    }

    /// `{"expression": spec, "value": the constant or null}`.
    fn var(&self, v: &RandomVar) -> Value {
        if v.is_missing() {
            return Value::Null;
        }
        let value = self.0.get(&*v.spec).copied().or_else(|| {
            // a specification no program of the run evaluates (e.g. of a removed resource)
            v.spec.trim().parse::<f64>().ok()
        });
        json!({"expression": &*v.spec, "value": value.filter(|x| x.is_finite())})
    }
}

/// The containers of the model's resource environments, depth first (top-level containers in
/// environment order, each followed by its nested ones).
fn present_containers(m: &Model) -> Vec<ContainerId> {
    fn walk(m: &Model, c: ContainerId, out: &mut Vec<ContainerId>) {
        if out.contains(&c) {
            return;
        }
        out.push(c);
        for &n in &m.containers[c].nested {
            walk(m, n, out);
        }
    }
    let mut out = Vec::new();
    for env in m.resource_environments.iter() {
        for &c in &env.containers {
            walk(m, c, &mut out);
        }
    }
    out
}

/// The assembly contexts of the systems' structures (the deployable assemblies; those inside
/// composite components belong to the components).
fn present_assemblies(m: &Model) -> Vec<AssemblyContextId> {
    let mut out = Vec::new();
    for s in m.systems.iter() {
        for &a in &m.structures[s.structure].assembly_contexts {
            if !out.contains(&a) {
                out.push(a);
            }
        }
    }
    out
}

fn present_links(m: &Model) -> Vec<LinkingResourceId> {
    let mut out = Vec::new();
    for env in m.resource_environments.iter() {
        for &l in &env.linking_resources {
            if !out.contains(&l) {
                out.push(l);
            }
        }
    }
    out
}

fn present_allocations(m: &Model) -> Vec<AllocationContextId> {
    let mut out = Vec::new();
    for al in m.allocations.iter() {
        for &c in &al.contexts {
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

/// What an element of `m` looks like in the deployment.
struct Elements<'a> {
    m: &'a Model,
    konst: Konst<'a>,
    /// The ids of the model the run started from.
    base: HashSet<&'a str>,
}

impl<'a> Elements<'a> {
    fn new(cm: &'a CompiledModel, base: &'a Model) -> Elements<'a> {
        let mut ids = HashSet::new();
        for c in base.containers.iter() {
            ids.insert(&*c.id);
        }
        for r in base.processing_resources.iter() {
            ids.insert(&*r.id);
        }
        for l in base.linking_resources.iter() {
            ids.insert(&*l.id);
        }
        for a in base.assembly_contexts.iter() {
            ids.insert(&*a.id);
        }
        for a in base.allocation_contexts.iter() {
            ids.insert(&*a.id);
        }
        Elements {
            m: &cm.model,
            konst: Konst::of(cm),
            base: ids,
        }
    }

    fn resource(&self, r: ProcessingResourceId) -> Value {
        let m = self.m;
        let pr = &m.processing_resources[r];
        let ty = pr.resource_type.map(|t| &m.resource_types[t]);
        let mut v = json!({
            "id": &*pr.id,
            "in_model": self.base.contains(&*pr.id),
            "type": ty.map(|t| &*t.name),
            "type_id": ty.map(|t| &*t.id),
            "scheduling": pr.scheduling.map(|s| &*m.scheduling_policies[s].id),
            "rate": self.konst.var(&pr.processing_rate),
            "replicas": pr.replicas,
            "mttf": pr.mttf,
            "mttr": pr.mttr,
        });
        if let Some(h) = &pr.hdd {
            v["hdd"] = json!({
                "read_rate": self.konst.var(&h.read),
                "write_rate": self.konst.var(&h.write),
            });
        }
        v
    }

    fn container(&self, c: ContainerId) -> Value {
        let m = self.m;
        let rc = &m.containers[c];
        json!({
            "id": &*rc.id,
            "name": &*rc.name,
            "in_model": self.base.contains(&*rc.id),
            "parent": rc.parent.map(|p| &*m.containers[p].id),
            "unit": Value::Null,
            "resources": rc.processing_resources.iter().map(|&r| self.resource(r)).collect::<Vec<_>>(),
        })
    }

    fn link(&self, l: LinkingResourceId) -> Value {
        let m = self.m;
        let lr = &m.linking_resources[l];
        let ty = lr.resource_type.map(|t| &m.resource_types[t]);
        json!({
            "id": &*lr.id,
            "name": &*lr.name,
            "in_model": self.base.contains(&*lr.id),
            "connected": lr.connected.iter().map(|&c| &*m.containers[c].id).collect::<Vec<_>>(),
            "type": ty.map(|t| &*t.name),
            "type_id": ty.map(|t| &*t.id),
            "latency": self.konst.var(&lr.latency),
            "throughput": self.konst.var(&lr.throughput),
            "failure_probability": lr.failure_probability,
        })
    }

    fn assembly(&self, a: AssemblyContextId) -> Value {
        let m = self.m;
        let ac = &m.assembly_contexts[a];
        let system = ac.parent.and_then(|s| match m.structures[s].owner {
            StructureOwner::System(s) => Some(&*m.systems[s].id),
            StructureOwner::Component(_) => None,
        });
        json!({
            "id": &*ac.id,
            "name": &*ac.name,
            "in_model": self.base.contains(&*ac.id),
            "component": ac.component.map(|c| &*m.components[c].id),
            "system": system,
            "unit": Value::Null,
        })
    }

    fn allocation(&self, a: AllocationContextId) -> Value {
        let m = self.m;
        let ac = &m.allocation_contexts[a];
        json!({
            "id": &*ac.id,
            "name": &*ac.name,
            "in_model": self.base.contains(&*ac.id),
            "allocation": ac.allocation.map(|x| &*m.allocations[x].id),
            "assembly": ac.assembly.map(|x| &*m.assembly_contexts[x].id),
            "container": ac.container.map(|x| &*m.containers[x].id),
        })
    }
}

/// The deployment of `cm`'s model: the containers of its resource environments (with their
/// processing resources), the linking resources, the systems' assembly contexts and the
/// allocation contexts, as they are configured now. `base` is the model the run started from
/// (`in_model`: an element with the same id is defined there). Target groups are left empty.
pub fn deployment(cm: &CompiledModel, base: &Model) -> Deployment {
    let m = &cm.model;
    let e = Elements::new(cm, base);
    let mut d = Deployment::default();
    *d.get_mut("containers") = present_containers(m)
        .into_iter()
        .map(|c| e.container(c))
        .collect();
    *d.get_mut("linking_resources") = present_links(m).into_iter().map(|l| e.link(l)).collect();
    *d.get_mut("assemblies") = present_assemblies(m)
        .into_iter()
        .map(|a| e.assembly(a))
        .collect();
    *d.get_mut("allocations") = present_allocations(m)
        .into_iter()
        .map(|a| e.allocation(a))
        .collect();
    d
}

/// The elements `cm`'s model still holds that are not in its deployment: containers (with
/// their processing resources), processing resources removed from a container that stayed,
/// linking resources, assembly contexts and allocation contexts that left the model, with their
/// properties as they were when they left. A run keeps them (tombstones, `crate::change`), so
/// that measurements and requests in progress that refer to them stay resolvable.
pub fn retired(cm: &CompiledModel, base: &Model) -> Value {
    let m = &cm.model;
    let e = Elements::new(cm, base);
    let containers: HashSet<ContainerId> = present_containers(m).into_iter().collect();
    let links: HashSet<LinkingResourceId> = present_links(m).into_iter().collect();
    let assemblies: HashSet<AssemblyContextId> = present_assemblies(m).into_iter().collect();
    let allocations: HashSet<AllocationContextId> = present_allocations(m).into_iter().collect();
    // only the systems' assemblies are deployed: a retired assembly had a system's structure
    // (a tombstone has no parent), a composite component's never leave with the deployment
    let composite: HashSet<AssemblyContextId> = m
        .structures
        .iter()
        .filter(|s| matches!(s.owner, StructureOwner::Component(_)))
        .flat_map(|s| s.assembly_contexts.iter().copied())
        .collect();
    let retired_containers: Vec<ContainerId> = m
        .containers
        .ids()
        .filter(|c| !containers.contains(c))
        .collect();
    let resources: Vec<Value> = m
        .processing_resources
        .iter_ids()
        .filter(|(r, pr)| {
            pr.container.is_none_or(|c| {
                containers.contains(&c) && !m.containers[c].processing_resources.contains(r)
            })
        })
        .map(|(r, pr)| {
            let mut v = e.resource(r);
            v["container"] = json!(pr.container.map(|c| &*m.containers[c].id));
            v
        })
        .collect();
    json!({
        "containers": retired_containers.iter().map(|&c| e.container(c)).collect::<Vec<_>>(),
        "processing_resources": resources,
        "linking_resources": m.linking_resources.ids().filter(|l| !links.contains(l)).map(|l| e.link(l)).collect::<Vec<_>>(),
        "assemblies": m.assembly_contexts.ids()
            .filter(|a| !assemblies.contains(a) && !composite.contains(a))
            .map(|a| e.assembly(a)).collect::<Vec<_>>(),
        "allocations": m.allocation_contexts.ids().filter(|a| !allocations.contains(a)).map(|a| e.allocation(a)).collect::<Vec<_>>(),
    })
}

// ---------------------------------------------------------------------------------------------
// the model file

fn role_json(m: &Model, r: RoleId) -> Value {
    let role = &m.roles[r];
    json!({
        "id": &*role.id,
        "name": &*role.name,
        "interface": role.interface.map(|i| &*m.interfaces[i].id),
    })
}

fn roles_json(m: &Model, roles: &[RoleId]) -> Vec<Value> {
    roles.iter().map(|&r| role_json(m, r)).collect()
}

fn processing_json(p: &ProcessingType) -> Value {
    let stat = |s: &StatisticalCharacterization| format!("{s:?}");
    match p {
        ProcessingType::FeedThrough => json!({"type": "FeedThrough"}),
        ProcessingType::TimeDriven {
            window_length,
            window_increment,
        } => {
            json!({"type": "TimeDriven", "window_length": window_length, "window_increment": window_increment})
        }
        ProcessingType::TimeDrivenAggregation {
            window_length,
            window_increment,
            statistic,
        } => json!({"type": "TimeDrivenAggregation", "window_length": window_length,
                    "window_increment": window_increment, "statistic": stat(statistic)}),
        ProcessingType::FixedSizeAggregation {
            frequency,
            number_of_measurements,
            statistic,
        } => json!({"type": "FixedSizeAggregation", "frequency": frequency,
                    "number_of_measurements": number_of_measurements, "statistic": stat(statistic)}),
        ProcessingType::VariableSizeAggregation {
            frequency,
            retrospection_length,
            statistic,
        } => json!({"type": "VariableSizeAggregation", "frequency": frequency,
                    "retrospection_length": retrospection_length, "statistic": stat(statistic)}),
        ProcessingType::Unsupported(c) => json!({"type": c}),
        ProcessingType::Missing => Value::Null,
    }
}

/// A measuring point of the model file: its `class`, its `key` as in `measurements.csv` (`null`
/// for points SimOxide records no series at) and the model elements it references, by id.
fn measuring_point_json(m: &Model, mp: &MeasuringPoint) -> Value {
    let o = |x: Option<&str>| x.map_or(Value::Null, |x| json!(x));
    let (class, refs) = match &mp.kind {
        MeasuringPointKind::AssemblyOperation {
            assembly,
            role,
            signature,
        } => (
            "AssemblyOperationMeasuringPoint",
            json!({"assembly": o(assembly.map(|x| &*m.assembly_contexts[x].id)),
                   "role": o(role.map(|x| &*m.roles[x].id)),
                   "signature": o(signature.map(|x| &*m.signatures[x].id))}),
        ),
        MeasuringPointKind::AssemblyPassiveResource {
            assembly,
            passive_resource,
        } => (
            "AssemblyPassiveResourceMeasuringPoint",
            json!({"assembly": o(assembly.map(|x| &*m.assembly_contexts[x].id)),
                   "passive_resource": o(passive_resource.map(|x| &*m.passive_resources[x].id))}),
        ),
        MeasuringPointKind::ActiveResource {
            resource,
            replica_id,
        } => (
            "ActiveResourceMeasuringPoint",
            json!({"processing_resource": o(resource.map(|x| &*m.processing_resources[x].id)),
                   "replica": replica_id}),
        ),
        MeasuringPointKind::SystemOperation {
            system,
            role,
            signature,
        } => (
            "SystemOperationMeasuringPoint",
            json!({"system": o(system.map(|x| &*m.systems[x].id)),
                   "role": o(role.map(|x| &*m.roles[x].id)),
                   "signature": o(signature.map(|x| &*m.signatures[x].id))}),
        ),
        MeasuringPointKind::SubSystemOperation {
            subsystem,
            role,
            signature,
        } => (
            "SubSystemOperationMeasuringPoint",
            json!({"subsystem": o(subsystem.map(|x| &*m.components[x].id)),
                   "role": o(role.map(|x| &*m.roles[x].id)),
                   "signature": o(signature.map(|x| &*m.signatures[x].id))}),
        ),
        MeasuringPointKind::LinkingResource { resource } => (
            "LinkingResourceMeasuringPoint",
            json!({"linking_resource": o(resource.map(|x| &*m.linking_resources[x].id))}),
        ),
        MeasuringPointKind::UsageScenario { scenario } => (
            "UsageScenarioMeasuringPoint",
            json!({"usage_scenario": o(scenario.map(|x| &*m.usage_scenarios[x].id))}),
        ),
        MeasuringPointKind::EntryLevelSystemCall { call } => (
            "EntryLevelSystemCallMeasuringPoint",
            json!({"entry_level_system_call": o(call.map(|x| &*m.user_actions[x].id))}),
        ),
        MeasuringPointKind::ExternalCallAction { action } => (
            "ExternalCallActionMeasuringPoint",
            json!({"external_call": o(action.map(|x| &*m.actions[x].id))}),
        ),
        MeasuringPointKind::ResourceEnvironment { .. } => {
            ("ResourceEnvironmentMeasuringPoint", json!({}))
        }
        MeasuringPointKind::ResourceContainer { container } => (
            "ResourceContainerMeasuringPoint",
            json!({"container": o(container.map(|x| &*m.containers[x].id))}),
        ),
        MeasuringPointKind::Other {
            class,
            measuring_point,
        } => (*class, json!({"text": o(measuring_point.as_deref())})),
    };
    let mut v = json!({"class": class, "key": ss_mp_key(m, mp)});
    if let (Value::Object(v), Value::Object(r)) = (&mut v, refs) {
        v.extend(r);
    }
    v
}

/// The definitions of `cm`'s model for the model file (`docs/guide/runtime-metadata.md`, "Model
/// file"): resource types, interfaces, components, systems, usage scenarios, external calls,
/// monitors, and `deployment`, the model's deployment (with target groups, from the caller).
/// [`seal`] adds the format and the content hash.
pub fn definitions(cm: &CompiledModel, deployment: &Deployment) -> Map<String, Value> {
    let m = &cm.model;
    let ids = Ids::new(m);
    let konst = Konst::of(cm);
    let mut o = Map::new();
    o.insert(
        "resource_types".into(),
        m.resource_types
            .iter()
            .map(|t| {
                json!({"id": &*t.id, "name": &*t.name, "kind": match t.kind {
                    ResourceTypeKind::Processing => "processing",
                    ResourceTypeKind::CommunicationLink => "communication_link",
                }})
            })
            .collect(),
    );
    o.insert(
        "interfaces".into(),
        m.interfaces
            .iter()
            .map(|i| {
                json!({
                    "id": &*i.id,
                    "name": &*i.name,
                    "kind": format!("{:?}", i.kind),
                    "signatures": i.signatures.iter().map(|&s| json!({
                        "id": &*m.signatures[s].id, "name": &*m.signatures[s].name,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    );
    o.insert(
        "components".into(),
        m.components
            .iter()
            .map(|c| {
                json!({
                    "id": &*c.id,
                    "name": &*c.name,
                    "kind": match c.kind {
                        ComponentKind::Basic { .. } => "basic",
                        ComponentKind::Composite { .. } => "composite",
                        ComponentKind::SubSystem { .. } => "subsystem",
                        ComponentKind::CompleteType => "complete_type",
                        ComponentKind::ProvidesType => "provides_type",
                    },
                    "repository": c.repository.map(|r| &*m.repositories[r].id),
                    "provided_roles": roles_json(m, &c.provided_roles),
                    "required_roles": roles_json(m, &c.required_roles),
                    "assemblies": c.structure().map(|s| m.structures[s].assembly_contexts.iter()
                        .map(|&a| json!({
                            "id": &*m.assembly_contexts[a].id,
                            "name": &*m.assembly_contexts[a].name,
                            "component": m.assembly_contexts[a].component.map(|x| &*m.components[x].id),
                        })).collect::<Vec<_>>()).unwrap_or_default(),
                    "passive_resources": c.passive_resources().iter().map(|&p| json!({
                        "id": &*m.passive_resources[p].id,
                        "name": &*m.passive_resources[p].name,
                        "capacity": konst.var(&m.passive_resources[p].capacity),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    );
    o.insert(
        "systems".into(),
        m.systems
            .iter()
            .map(|s| {
                json!({
                    "id": &*s.id,
                    "name": &*s.name,
                    "provided_roles": roles_json(m, &s.provided_roles),
                    "required_roles": roles_json(m, &s.required_roles),
                })
            })
            .collect(),
    );
    o.insert(
        "usage_scenarios".into(),
        m.usage_scenarios
            .iter()
            .map(|sc| {
                let workload = match &sc.workload {
                    Workload::Closed {
                        population,
                        think_time,
                    } => json!({"type": "closed", "population": population,
                                "think_time": konst.var(think_time)}),
                    Workload::Open { inter_arrival_time } => {
                        json!({"type": "open", "inter_arrival_time": konst.var(inter_arrival_time)})
                    }
                    Workload::Missing => Value::Null,
                };
                let calls: Vec<Value> = m
                    .user_actions
                    .iter_ids()
                    .filter_map(|(u, ua)| match &ua.kind {
                        UserActionKind::EntryLevelSystemCall {
                            role, signature, ..
                        } if ids.user_action_scenario(u) == Some(&*sc.id) => Some(json!({
                            "id": &*ua.id,
                            "name": &*ua.name,
                            "role": role.map(|r| &*m.roles[r].id),
                            "signature": signature.map(|s| &*m.signatures[s].id),
                        })),
                        _ => None,
                    })
                    .collect();
                json!({"id": &*sc.id, "name": &*sc.name, "workload": workload, "calls": calls})
            })
            .collect(),
    );
    o.insert(
        "external_calls".into(),
        m.actions
            .iter_ids()
            .filter_map(|(a, act)| match &act.kind {
                ActionKind::ExternalCall {
                    role, signature, ..
                } => Some(json!({
                    "id": &*act.id,
                    "name": &*act.name,
                    "component": ids.action_component(a),
                    "role": role.map(|r| &*m.roles[r].id),
                    "signature": signature.map(|s| &*m.signatures[s].id),
                })),
                _ => None,
            })
            .collect(),
    );
    o.insert(
        "monitors".into(),
        m.monitors
            .iter()
            .map(|mo| {
                json!({
                    "id": &*mo.id,
                    "name": &*mo.name,
                    "activated": mo.activated,
                    "measuring_point": mo.measuring_point.map(|p| measuring_point_json(m, &m.measuring_points[p])),
                    "specifications": mo.specifications.iter().map(|s| json!({
                        "id": &*s.id,
                        "metric": s.metric.map(|x| json!({"id": &*m.metrics[x].id, "name": &*m.metrics[x].name})),
                        "processing": processing_json(&s.processing),
                        "triggers_self_adaptations": s.triggers_self_adaptations,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    );
    o.insert("deployment".into(), deployment.to_json());
    o
}

/// The model file of `cm` with deployment `d` and the scaling definitions `scaling` (`null`
/// without scaling policies; `simoxide_spd::inspect::model_file` gives both).
pub fn model_file(cm: &CompiledModel, d: &Deployment, scaling: Value) -> Value {
    let mut o = definitions(cm, d);
    o.insert("scaling".into(), scaling);
    seal(o)
}

/// The model file: `definitions` (with whatever the caller added, e.g. `scaling`) with `format`,
/// `version` and `hash`, the FNV-1a 64 of the compact JSON of everything else (keys sorted):
/// equal hashes, equal content.
pub fn seal(mut definitions: Map<String, Value>) -> Value {
    definitions.remove("hash");
    definitions.insert("format".into(), json!(MODEL_FORMAT));
    definitions.insert("version".into(), json!(VERSION));
    let text = serde_json::to_string(&definitions).expect("JSON");
    definitions.insert("hash".into(), json!(fnv64_hex(text.as_bytes())));
    Value::Object(definitions)
}

// ---------------------------------------------------------------------------------------------
// measurement series

/// The unit of a metric's values (`docs/guide/runtime-metadata.md`, "Series"): `s` for times,
/// `1` for counts and fractions.
pub fn metric_unit(metric: &str) -> Option<&'static str> {
    Some(match metric {
        metric_names::RESPONSE_TIME
        | metric_names::RESOURCE_DEMAND
        | metric_names::WAITING_TIME
        | metric_names::HOLDING_TIME
        | metric_names::RECONFIGURATION_TIME => "s",
        metric_names::STATE_OF_ACTIVE_RESOURCE
        | metric_names::UTILIZATION
        | metric_names::STATE_OF_PASSIVE_RESOURCE
        | metric_names::NUMBER_OF_RESOURCE_CONTAINERS => "1",
        _ => return None,
    })
}

/// Id lookups over every element of a model, tombstones included.
struct Ids<'a> {
    m: &'a Model,
    resources: HashMap<&'a str, ProcessingResourceId>,
    assemblies: HashMap<&'a str, AssemblyContextId>,
    passives: HashMap<&'a str, PassiveResourceId>,
    actions: HashMap<&'a str, ActionId>,
    user_actions: HashMap<&'a str, UserActionId>,
    /// Measuring-point key -> the monitors at that point, in repository order.
    monitors: HashMap<String, Vec<MonitorId>>,
}

fn index<'a, I: From<usize> + Copy, T>(
    a: &'a Arena<I, T>,
    id: impl Fn(&'a T) -> &'a str,
) -> HashMap<&'a str, I> {
    let mut h = HashMap::new();
    for (i, t) in a.iter_ids() {
        h.entry(id(t)).or_insert(i);
    }
    h
}

impl<'a> Ids<'a> {
    fn new(m: &'a Model) -> Ids<'a> {
        let mut monitors: HashMap<String, Vec<MonitorId>> = HashMap::new();
        for (i, mo) in m.monitors.iter_ids() {
            if let Some(k) = mo
                .measuring_point
                .and_then(|p| ss_mp_key(m, &m.measuring_points[p]))
            {
                monitors.entry(k).or_default().push(i);
            }
        }
        Ids {
            m,
            resources: index(&m.processing_resources, |x| &x.id),
            assemblies: index(&m.assembly_contexts, |x| &x.id),
            passives: index(&m.passive_resources, |x| &x.id),
            actions: index(&m.actions, |x| &x.id),
            user_actions: index(&m.user_actions, |x| &x.id),
            monitors,
        }
    }

    /// The component whose SEFF holds `a` (through nested behaviours).
    fn action_component(&self, a: ActionId) -> Option<&'a str> {
        let m = self.m;
        let mut cur = a;
        for _ in 0..1024 {
            let b = m.actions[cur].behaviour?;
            match m.behaviours[b].owner {
                BehaviourOwner::Seff(s) | BehaviourOwner::Internal(s) => {
                    return m.seffs[s].component.map(|c| &*m.components[c].id);
                }
                BehaviourOwner::Loop(x)
                | BehaviourOwner::BranchTransition(x, _)
                | BehaviourOwner::ForkedAsync(x)
                | BehaviourOwner::ForkedSync(x) => cur = x,
                BehaviourOwner::Other => return None,
            }
        }
        None
    }

    /// The usage scenario whose behaviour holds `u` (through loops and branches).
    fn user_action_scenario(&self, u: UserActionId) -> Option<&'a str> {
        let m = self.m;
        let mut cur = u;
        for _ in 0..1024 {
            let b = m.user_actions[cur].behaviour?;
            match m.scenario_behaviours[b].owner {
                ScenarioBehaviourOwner::Scenario(s) => return Some(&m.usage_scenarios[s].id),
                ScenarioBehaviourOwner::Loop(x) | ScenarioBehaviourOwner::Branch(x, _) => cur = x,
                ScenarioBehaviourOwner::Other => return None,
            }
        }
        None
    }
}

/// The parts of a measuring-point key: its class and the `|`-separated parts in the brackets.
fn key_parts(key: &str) -> (&str, Vec<&str>) {
    match key.split_once('[') {
        Some((c, rest)) => {
            let args = rest.strip_suffix(']').unwrap_or(rest);
            (c, args.split('|').collect())
        }
        None => (key, Vec::new()),
    }
}

/// The model elements of a measuring point (ids), and the associations not available.
struct Point {
    p: Map<String, Value>,
    unavailable: Vec<&'static str>,
}

impl Point {
    /// An id the key names.
    fn named(&mut self, k: &str, v: &str) {
        self.p.insert(k.into(), json!(v));
    }

    /// An id found through the model: `k`, else `k` is unavailable.
    fn found(&mut self, k: &'static str, v: Option<&str>) {
        match v {
            Some(v) => self.named(k, v),
            None => self.unavailable.push(k),
        }
    }
}

/// What a measuring-point key refers to (`docs/guide/formats.md` §4 defines the keys from the
/// model's ids): the point's model elements by id, and the names of the associations neither the
/// key nor the model gives.
fn point_of(ids: &Ids, key: &str) -> (Value, Vec<&'static str>) {
    let m = ids.m;
    let (class, parts) = key_parts(key);
    let part = |i: usize| parts.get(i).copied().unwrap_or_default();
    let mut p = Point {
        p: Map::new(),
        unavailable: Vec::new(),
    };
    p.named("class", class);
    let component_of_assembly = |a: &str| {
        ids.assemblies
            .get(a)
            .and_then(|&x| m.assembly_contexts[x].component)
            .map(|c| &*m.components[c].id)
    };
    match class {
        "UsageScenarioMeasuringPoint" => p.named("usage_scenario", part(0)),
        "EntryLevelSystemCallMeasuringPoint" => {
            p.named("entry_level_system_call", part(0));
            let ua = ids.user_actions.get(part(0)).copied();
            let (role, sig) = match ua.map(|u| &m.user_actions[u].kind) {
                Some(UserActionKind::EntryLevelSystemCall {
                    role, signature, ..
                }) => (
                    role.map(|r| &*m.roles[r].id),
                    signature.map(|s| &*m.signatures[s].id),
                ),
                _ => (None, None),
            };
            p.found(
                "usage_scenario",
                ua.and_then(|u| ids.user_action_scenario(u)),
            );
            p.found("role", role);
            p.found("signature", sig);
        }
        "SystemOperationMeasuringPoint" => {
            p.named("role", part(0));
            p.named("signature", part(1));
            p.named("system", part(2));
        }
        "AssemblyOperationMeasuringPoint" => {
            p.named("role", part(0));
            p.named("signature", part(1));
            p.named("assembly", part(2));
            p.found("component", component_of_assembly(part(2)));
        }
        "SubSystemOperationMeasuringPoint" => {
            p.named("subsystem", part(0));
            p.named("role", part(1));
            p.named("signature", part(2));
        }
        "ExternalCallActionMeasuringPoint" => {
            p.named("external_call", part(0));
            let a = ids.actions.get(part(0)).copied();
            let (role, sig) = match a.map(|a| &m.actions[a].kind) {
                Some(ActionKind::ExternalCall {
                    role, signature, ..
                }) => (
                    role.map(|r| &*m.roles[r].id),
                    signature.map(|s| &*m.signatures[s].id),
                ),
                _ => (None, None),
            };
            p.found("role", role);
            p.found("signature", sig);
            p.found("component", a.and_then(|a| ids.action_component(a)));
        }
        "ActiveResourceMeasuringPoint" => {
            p.named("processing_resource", part(0));
            let replica = part(1)
                .strip_prefix("replicaID=")
                .and_then(|x| x.parse::<i64>().ok());
            p.p.insert("replica".into(), json!(replica));
            let pr = ids
                .resources
                .get(part(0))
                .map(|&r| &m.processing_resources[r]);
            p.found(
                "container",
                pr.and_then(|pr| pr.container).map(|c| &*m.containers[c].id),
            );
            p.found(
                "resource_type",
                pr.and_then(|pr| pr.resource_type)
                    .map(|t| &*m.resource_types[t].id),
            );
        }
        "LinkingResourceMeasuringPoint" => p.named("linking_resource", part(0)),
        "ResourceContainerMeasuringPoint" => p.named("container", part(0)),
        "ResourceEnvironmentMeasuringPoint" => p.named("resource_environment", part(0)),
        "AssemblyPassiveResourceMeasuringPoint" => {
            p.named("assembly", part(0));
            p.named("passive_resource", part(1));
            p.found("component", component_of_assembly(part(0)));
        }
        "ResourceURIMeasuringPoint" => match ids.passives.get(part(0)) {
            // SimuLizar's passive-resource points: the passive resource by id, the assembly
            // context by its name only
            Some(&pr) => {
                p.named("passive_resource", part(0));
                p.found(
                    "component",
                    m.passive_resources[pr]
                        .component
                        .map(|c| &*m.components[c].id),
                );
                p.unavailable.push("assembly");
            }
            // reconfiguration points: a URI fragment and the point's text
            None => {
                let text = key
                    .split_once('[')
                    .and_then(|x| x.1.split_once('|'))
                    .map_or("", |x| x.1);
                p.named("resource_uri_fragment", part(0));
                p.named("text", text.strip_suffix(']').unwrap_or(text));
            }
        },
        _ => p.unavailable.push("point"),
    }
    (Value::Object(p.p), p.unavailable)
}

/// The metadata of measurement series `defs` (a run's `Measurements::series`) against the run's
/// current model `cm` (which keeps the elements that left it): one object per series, in the
/// order of `measurements.csv` (`Measurements::sorted_series`). Each has the series' identity
/// (`measuring_point`, `metric`), the `unit` of its values, the model elements its point refers
/// to (`point`), the monitors of the model at that point (`monitors`, each with the
/// specification that gives the series' metric, if any) and the associations that are not
/// available (`unavailable`; `monitor` when no monitor of the model has the point).
pub fn series(cm: &CompiledModel, defs: &[SeriesDef]) -> Vec<Value> {
    let m = &cm.model;
    let ids = Ids::new(m);
    let mut order: Vec<usize> = (0..defs.len()).collect();
    order.sort_by(|&a, &b| {
        let (sa, sb) = (&defs[a], &defs[b]);
        crate::meas::java_cmp(&sa.mp, &sb.mp)
            .then_with(|| crate::meas::java_cmp(sa.metric, sb.metric))
    });
    order
        .into_iter()
        .map(|i| {
            let d = &defs[i];
            let (point, mut unavailable) = point_of(&ids, &d.mp);
            let monitors: Vec<Value> = ids
                .monitors
                .get(&*d.mp)
                .into_iter()
                .flatten()
                .map(|&mo| {
                    let mo = &m.monitors[mo];
                    let spec = mo.specifications.iter().find(|s| {
                        s.metric.is_some_and(|x| {
                            let me = &m.metrics[x];
                            tuple_metric_name(&me.id) == Some(d.metric) || &*me.name == d.metric
                        })
                    });
                    json!({
                        "id": &*mo.id,
                        "name": &*mo.name,
                        "activated": mo.activated,
                        "specification": spec.map(|s| &*s.id),
                    })
                })
                .collect();
            if monitors.is_empty() {
                unavailable.push("monitor");
            }
            json!({
                "measuring_point": &*d.mp,
                "metric": d.metric,
                "unit": metric_unit(d.metric),
                "point": point,
                "monitors": monitors,
                "unavailable": unavailable,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// the runtime file

fn secs(ns: i64) -> f64 {
    ns as f64 / 1e9
}

/// The deployment of an invocation's run as it changes: the deployment at the start (the
/// initial projection) and one change record per model change or adjustment
/// (`docs/guide/runtime-metadata.md`, "Runtime file").
#[derive(Clone, Debug)]
pub struct Recorder {
    current: Deployment,
    start: Value,
    changes: Vec<Value>,
}

impl Recorder {
    /// Starts with deployment `d` at `time_ns`, model generation `generation`, with the policy
    /// state `policies` (`null` without scaling policies).
    pub fn start(d: Deployment, time_ns: i64, generation: u32, policies: Value) -> Recorder {
        let start = json!({
            "time": secs(time_ns),
            "time_ns": time_ns,
            "generation": generation,
            "deployment": d.to_json(),
            "policies": policies,
        });
        Recorder {
            current: d,
            start,
            changes: Vec::new(),
        }
    }

    /// The deployment now.
    pub fn current(&self) -> &Deployment {
        &self.current
    }

    /// Records a change at `time_ns` (model generation `generation` after it) that left
    /// deployment `d`: its `cause`, `outcome` (`null` if none) and the difference from the
    /// deployment before ([`Deployment::diff`]; empty if the change left it as it was).
    pub fn record(
        &mut self,
        d: Deployment,
        time_ns: i64,
        generation: u32,
        cause: Value,
        outcome: Value,
    ) {
        let mut c = self.current.diff(&d);
        c["time"] = json!(secs(time_ns));
        c["time_ns"] = json!(time_ns);
        c["generation"] = json!(generation);
        c["cause"] = cause;
        c["outcome"] = outcome;
        self.changes.push(c);
        self.current = d;
    }

    /// The changes recorded so far.
    pub fn changes(&self) -> &[Value] {
        &self.changes
    }

    /// The runtime file of an invocation that ended (paused, finished or stopped) at `end_ns`
    /// on model `cm` (the run's model now, with the elements that left it, [`retired`]; `base`
    /// the model the run started from), with the measurement series `series_defs`, model
    /// generation `generation` and the policy state `policies`. `model_hash` names the model
    /// file, `run` the invocation (`parent`, `state`, `status`).
    #[allow(clippy::too_many_arguments)]
    pub fn into_json(
        self,
        model_hash: &str,
        run: Value,
        cm: &CompiledModel,
        base: &CompiledModel,
        series_defs: &[SeriesDef],
        end_ns: i64,
        generation: u32,
        policies: Value,
    ) -> Value {
        json!({
            "format": RUNTIME_FORMAT,
            "version": VERSION,
            "model": {"format": MODEL_FORMAT, "hash": model_hash},
            "run": run,
            "start": self.start,
            "changes": self.changes,
            "end": {
                "time": secs(end_ns),
                "time_ns": end_ns,
                "generation": generation,
                "policies": policies,
            },
            "retired": retired(cm, &base.model),
            "series": series(cm, series_defs),
        })
    }
}

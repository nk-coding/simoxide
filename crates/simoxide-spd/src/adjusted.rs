//! What an executed adjustment changed, as Slingshot reports it (`DESIGN-SPD.md` §1.3, P2):
//! `SpdAdjustmentBehavior.onModelAdjustmentRequested` builds `ModelAdjusted(true, changes)`
//! from the container and allocation lists before and after the transformation
//! ([`ModelAdjusted`]), and the reference runner's `SpdObserver` records a model summary before
//! and after each adjustment ([`summary`]). Both read the model graph only; the adjuster
//! (`simoxide-reconf`'s QVTo adjuster, or another host) calls [`Before::capture`] before the
//! transformation and [`ModelAdjusted::compute`] after it.

use crate::model::SpdModel;
use simoxide_model::meta::prelude::*;
use simoxide_model::meta::{ClassId, class, feat};
use simoxide_model::raw::{Graph, ObjId, PcmGraph};
use std::collections::HashSet;

/// The lists Slingshot copies before the transformation runs: the containers of the
/// allocation's environment and the allocation contexts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Before {
    pub containers: Vec<ObjId>,
    pub allocation_contexts: Vec<ObjId>,
}

/// A monitor Slingshot's `ResourceContainerMonitorCloner` creates for a new container: a copy
/// of `monitor` (a monitor of the unit container, or of one of its processing resources) whose
/// measuring point refers to `target` (the new container, or its processing resource of the
/// same resource type).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MonitorClone {
    pub monitor: ObjId,
    pub target: ObjId,
}

/// Slingshot's `ModelAdjusted` (`SpdAdjustmentBehavior.java:90-122`). On success its change
/// list holds a `ResourceEnvironmentChange` (old, new and deleted containers), an
/// `AllocationChange` (the new allocation contexts) and one `MonitorChange` per cloned monitor;
/// on failure it is empty. Deleted allocation contexts are not reported (quirk Q14).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelAdjusted {
    pub ok: bool,
    pub old_containers: Vec<ObjId>,
    pub new_containers: Vec<ObjId>,
    pub deleted_containers: Vec<ObjId>,
    pub new_allocation_contexts: Vec<ObjId>,
    /// The monitors Slingshot clones for the new containers, in its order. Creating them is the
    /// syncers' part (phase F4); this is what Slingshot's change list reports.
    pub monitor_clones: Vec<MonitorClone>,
}

impl ModelAdjusted {
    /// A failed adjustment: `ModelAdjusted(false, [])`.
    pub fn failed() -> ModelAdjusted {
        ModelAdjusted::default()
    }

    /// The number of entries of Slingshot's change list (the reference runner's `changes`).
    pub fn changes(&self) -> usize {
        if self.ok {
            2 + self.monitor_clones.len()
        } else {
            0
        }
    }

    /// The successful adjustment's report: the lists of `before` against the graph `g` after
    /// the transformation, and the monitors to clone (those of the graph's first monitor
    /// repository, the one Slingshot injects; none without one).
    pub fn compute(before: &Before, g: &Graph, spd: &SpdModel) -> ModelAdjusted {
        let after = Before::capture(g, spd);
        let new_containers: Vec<ObjId> = after
            .containers
            .iter()
            .copied()
            .filter(|c| !before.containers.contains(c))
            .collect();
        let deleted_containers = before
            .containers
            .iter()
            .copied()
            .filter(|c| !after.containers.contains(c))
            .collect();
        let new_allocation_contexts = after
            .allocation_contexts
            .iter()
            .copied()
            .filter(|a| !before.allocation_contexts.contains(a))
            .collect();
        let monitor_clones = monitor_clones(g, spd, &new_containers);
        ModelAdjusted {
            ok: true,
            old_containers: before.containers.clone(),
            new_containers,
            deleted_containers,
            new_allocation_contexts,
            monitor_clones,
        }
    }
}

impl Before {
    /// The lists of the run's allocation (the semantic configuration's, else the graph's first)
    /// and its target environment.
    pub fn capture(g: &Graph, spd: &SpdModel) -> Before {
        let mut b = Before::default();
        if let Some(a) = run_allocation(g, spd) {
            b.allocation_contexts = g
                .get_refs(a, feat::Allocation_allocationContexts_Allocation)
                .to_vec();
            if let Some(env) = g.get_ref(a, feat::Allocation_targetResourceEnvironment_Allocation) {
                b.containers = g
                    .get_refs(
                        env,
                        feat::ResourceEnvironment_resourceContainer_ResourceEnvironment,
                    )
                    .to_vec();
            }
        }
        b
    }
}

/// The run's allocation: the semantic configuration's (if resolved), else the first one of the
/// graph.
pub fn run_allocation(g: &Graph, spd: &SpdModel) -> Option<ObjId> {
    spd.configuration
        .and_then(|c| g.get_ref(c, feat::Configuration_allocation))
        .filter(|&a| g[a].proxy.is_none())
        .or_else(|| first_root(g, class::Allocation))
}

/// The first root of class `c` in resource order (how the reference runner's summary picks the
/// environment, allocation and configuration).
fn first_root(g: &Graph, c: ClassId) -> Option<ObjId> {
    g.resource_ids()
        .flat_map(|r| g.resource(r).roots.clone())
        .find(|&o| g[o].proxy.is_none() && g[o].class.is_a(c))
}

/// `createMonitors`: for the unit container of the elastic infrastructure that holds the first
/// new container, its monitors (resource-container measuring points, then the monitors of its
/// processing resources), cloned for each new container; a processing-resource monitor only
/// if the new container has a processing resource of the same type (the first one).
fn monitor_clones(g: &Graph, spd: &SpdModel, new: &[ObjId]) -> Vec<MonitorClone> {
    let Some(&first) = new.first() else {
        return Vec::new();
    };
    let Some(cfg) = spd.configuration else {
        return Vec::new();
    };
    let unit = g
        .get_refs(cfg, feat::Configuration_targetCfgs)
        .iter()
        .copied()
        .filter(|&t| g[t].class.is_a(class::ElasticInfrastructureCfg))
        .find(|&t| {
            g.get_refs(t, feat::ElasticInfrastructureCfg_elements)
                .contains(&first)
        })
        .and_then(|t| g.get_ref(t, feat::ElasticInfrastructureCfg_unit));
    let (Some(unit), Some(repo)) = (unit, first_root(g, class::MonitorRepository)) else {
        return Vec::new();
    };
    let monitors = g.get_refs(repo, feat::MonitorRepository_monitors);
    let mp_of = |m: ObjId| {
        g.get_ref(m, feat::Monitor_measuringPoint)
            .filter(|&p| g[p].proxy.is_none())
    };
    let id = |o: Option<ObjId>| o.and_then(|o| g.id(o).map(String::from));
    // ResourceContainerMonitorFinder: by id of the referenced element
    let unit_id = id(Some(unit));
    let container_monitors: Vec<ObjId> = monitors
        .iter()
        .copied()
        .filter(|&m| {
            mp_of(m).is_some_and(|p| {
                g[p].class.is_a(class::ResourceContainerMeasuringPoint)
                    && id(g.get_ref(p, feat::ResourceContainerReference_resourceContainer))
                        == unit_id
            })
        })
        .collect();
    let specs = |c: ObjId| {
        g.get_refs(
            c,
            feat::ResourceContainer_activeResourceSpecifications_ResourceContainer,
        )
        .to_vec()
    };
    let resource_monitors: Vec<(ObjId, ObjId)> = specs(unit)
        .into_iter()
        .flat_map(|spec| {
            let spec_id = id(Some(spec));
            monitors
                .iter()
                .copied()
                .filter(move |&m| {
                    mp_of(m).is_some_and(|p| {
                        g[p].class.is_a(class::ActiveResourceMeasuringPoint)
                            && id(g.get_ref(p, feat::ActiveResourceReference_activeResource))
                                == spec_id
                    })
                })
                .map(move |m| (m, spec))
                .collect::<Vec<_>>()
        })
        .collect();
    let type_of = |spec: ObjId| {
        id(g.get_ref(
            spec,
            feat::ProcessingResourceSpecification_activeResourceType_ActiveResourceSpecification,
        ))
    };
    let mut out = Vec::new();
    for &c in new {
        for &m in &container_monitors {
            out.push(MonitorClone {
                monitor: m,
                target: c,
            });
        }
        for &(m, unit_spec) in &resource_monitors {
            let t = type_of(unit_spec);
            if let Some(spec) = specs(c).into_iter().find(|&s| type_of(s) == t) {
                out.push(MonitorClone {
                    monitor: m,
                    target: spec,
                });
            }
        }
    }
    out
}

/// The ids of every object of the graph: the elements present when a run starts. The summary
/// shows other elements (created by adjustments) by their class and name, as the reference
/// runner's `Keys.alias` does.
pub fn input_ids(g: &Graph) -> HashSet<Box<str>> {
    (0..g.object_count() as u32)
        .map(ObjId)
        .filter_map(|o| g.id(o).map(Box::from))
        .collect()
}

/// The class's `EClass` name.
fn class_name(c: ClassId) -> String {
    let q = c.qualified_name();
    q.rsplit(':').next().unwrap_or(&q).to_string()
}

fn quote(s: &str, out: &mut String) {
    out.push_str(&serde_json::to_string(s).expect("string"));
}

/// `Keys.id`: the id of an input element, `~Class:name` for elements created later.
fn key_id(g: &Graph, o: ObjId, input: &HashSet<Box<str>>) -> String {
    match g.id(o) {
        Some(id) if input.contains(id) => id.to_string(),
        _ => {
            let class = class_name(g[o].class);
            match g.str_attr(o, feat::NamedElement_entityName) {
                Some(n) => format!("~{class}:{n}"),
                None => format!("~{class}"),
            }
        }
    }
}

/// The reference runner's model summary (`SpdObserver.summary`): the containers of the first
/// resource environment (name and key), its links with their containers, the first
/// allocation's contexts (assembly and container names) and the target-group configurations
/// of the first semantic configuration with their elements, each in model order. `input`: the
/// ids of the elements present at the start ([`input_ids`]).
pub fn summary(g: &Graph, input: &HashSet<Box<str>>) -> String {
    let name = |o: ObjId| -> String {
        g.str_attr(o, feat::NamedElement_entityName)
            .map(String::from)
            .unwrap_or_default()
    };
    let mut s = String::from("{\"containers\":[");
    let env = first_root(g, class::ResourceEnvironment);
    if let Some(env) = env {
        for (i, &c) in g
            .get_refs(
                env,
                feat::ResourceEnvironment_resourceContainer_ResourceEnvironment,
            )
            .iter()
            .enumerate()
        {
            s.push_str(if i > 0 { ",[" } else { "[" });
            quote(&name(c), &mut s);
            s.push(',');
            quote(&key_id(g, c, input), &mut s);
            s.push(']');
        }
    }
    s.push_str("],\"links\":[");
    if let Some(env) = env {
        for (i, &l) in g
            .get_refs(
                env,
                feat::ResourceEnvironment_linkingResources__ResourceEnvironment,
            )
            .iter()
            .enumerate()
        {
            s.push_str(if i > 0 { ",[" } else { "[" });
            quote(&name(l), &mut s);
            for &c in g.get_refs(
                l,
                feat::LinkingResource_connectedResourceContainers_LinkingResource,
            ) {
                s.push(',');
                quote(&name(c), &mut s);
            }
            s.push(']');
        }
    }
    s.push_str("],\"allocation\":[");
    if let Some(a) = first_root(g, class::Allocation) {
        for (i, &ac) in g
            .get_refs(a, feat::Allocation_allocationContexts_Allocation)
            .iter()
            .enumerate()
        {
            s.push_str(if i > 0 { ",[" } else { "[" });
            let asm = g.get_ref(
                ac,
                feat::AllocationContext_assemblyContext_AllocationContext,
            );
            let rc = g.get_ref(
                ac,
                feat::AllocationContext_resourceContainer_AllocationContext,
            );
            quote(&asm.map(name).unwrap_or_default(), &mut s);
            s.push(',');
            quote(&rc.map(name).unwrap_or_default(), &mut s);
            s.push(']');
        }
    }
    s.push_str("],\"target_groups\":[");
    if let Some(cfg) = first_root(g, class::Configuration) {
        for (i, &t) in g
            .get_refs(cfg, feat::Configuration_targetCfgs)
            .iter()
            .enumerate()
        {
            let cl = g[t].class;
            s.push_str(if i > 0 { ",{\"type\":" } else { "{\"type\":" });
            quote(&class_name(cl), &mut s);
            s.push_str(",\"elements\":[");
            let els: &[ObjId] = if cl.is_a(class::ElasticInfrastructureCfg) {
                g.get_refs(t, feat::ElasticInfrastructureCfg_elements)
            } else if cl.is_a(class::ServiceGroupCfg) {
                g.get_refs(t, feat::ServiceGroupCfg_elements)
            } else {
                &[]
            };
            for (j, &e) in els.iter().enumerate() {
                if j > 0 {
                    s.push(',');
                }
                match g.str_attr(e, feat::NamedElement_entityName) {
                    Some(n) => quote(&n, &mut s),
                    None => quote(&key_id(g, e, input), &mut s),
                }
            }
            s.push_str("]}");
        }
    }
    s.push_str("]}");
    s
}

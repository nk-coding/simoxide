//! The SPD part of the runtime metadata (`simoxide_sim::inspect`,
//! `docs/guide/runtime-metadata.md`): target-group membership in the deployment, the scaling
//! definitions of the model file, the policy state of a run, and [`Recording`], an adjuster that
//! records every adjustment's change of the deployment with its cause.

use crate::decision::adjustment_json;
use crate::model::{GroupCfg, GroupKind, Membership, PolicyConstraint, SpdModel, TargetConstraint};
use crate::sim::{Adjuster, SpdRun};
use crate::{AdjustOutcome, AdjustmentRequest, Spd, SpdState};
use serde_json::{Map, Value, json};
use simoxide_model::raw::{Graph, ObjId};
use simoxide_sim::compat::Compat;
use simoxide_sim::inspect::{Deployment, Recorder};
use simoxide_sim::{ChangeHost, Flavour, SimError, Simulation};

fn id_of(g: &Graph, o: ObjId) -> Value {
    g.id(o).map_or(Value::Null, |s| json!(s))
}

fn kind_name(k: &GroupKind) -> &'static str {
    match k {
        GroupKind::ElasticInfrastructure { .. } => "elastic_infrastructure",
        GroupKind::ServiceGroup { .. } => "service_group",
        GroupKind::CompetingConsumers { .. } => "competing_consumers",
    }
}

/// Adds the target groups of `spd` to `d` (their configurations' current elements, read from the
/// model graph `g`), and sets `unit` on the containers and assemblies that are elements of a
/// group: the group's unit, the element it scales by copying.
pub fn add_target_groups(g: &Graph, spd: &SpdModel, d: &mut Deployment) {
    let m = Membership::from_graph(g, spd);
    let mut groups = Vec::new();
    for tg in &spd.groups {
        let cfg = m.cfg_of(tg);
        let (elements, broker): (&[ObjId], Option<ObjId>) = match cfg {
            Some(GroupCfg::ElasticInfrastructure { elements, .. })
            | Some(GroupCfg::ServiceGroup { elements, .. }) => (elements, None),
            Some(GroupCfg::CompetingConsumers {
                elements, broker, ..
            }) => (elements, *broker),
            None => (&[], None),
        };
        let unit = tg.kind.unit();
        let kind = match tg.kind {
            GroupKind::ElasticInfrastructure { .. } => "containers",
            _ => "assemblies",
        };
        for &e in elements {
            if let (Some(id), Some(u)) = (g.id(e), unit)
                && let Some(el) = d.find_mut(kind, id)
            {
                el["unit"] = id_of(g, u);
            }
        }
        let mut v = json!({
            "id": &*tg.id,
            "name": &*tg.name,
            "kind": kind_name(&tg.kind),
            "unit": unit.map_or(Value::Null, |u| id_of(g, u)),
            "configured": cfg.is_some(),
            "elements": elements.iter().map(|&e| id_of(g, e)).collect::<Vec<_>>(),
        });
        if matches!(tg.kind, GroupKind::CompetingConsumers { .. }) {
            v["broker"] = broker.map_or(Value::Null, |b| id_of(g, b));
        }
        groups.push(v);
    }
    *d.get_mut("target_groups") = groups;
}

/// The deployment of `host`'s current model (`simoxide_sim::inspect::deployment`) with the
/// target groups of `spd`, if any.
pub fn deployment<H: ChangeHost + ?Sized>(host: &H, spd: Option<&SpdModel>) -> Deployment {
    let mut d = simoxide_sim::inspect::deployment(host.compiled_now(), &host.base_compiled().model);
    if let Some(s) = spd {
        add_target_groups(host.model_graph(), s, &mut d);
    }
    d
}

/// The scaling definitions of the model file: target groups with their constraints, policies
/// with their adjustment and constraints, the HPA autoscalers (`hpa`: name, target group,
/// policy). Triggers are not described (version 1).
pub fn scaling_definitions(spd: &Spd, hpa: Option<&crate::hpa::Hpa>) -> Value {
    let m = spd.model();
    let groups: Vec<Value> = m
        .groups
        .iter()
        .map(|g| {
            json!({
                "id": &*g.id,
                "name": &*g.name,
                "kind": kind_name(&g.kind),
                "constraints": g.constraints.iter().map(|c| match *c {
                    TargetConstraint::Thrashing { minimum_time } =>
                        json!({"type": "ThrashingConstraint", "minimum_time": minimum_time}),
                    TargetConstraint::Size { min, max } =>
                        json!({"type": "TargetGroupSizeConstraint", "min": min, "max": max}),
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    let policies: Vec<Value> = m
        .policies
        .iter()
        .map(|p| {
            json!({
                "id": &*p.id,
                "name": &*p.name,
                "active": p.active,
                "target_group": p.group.map(|g| &*m.groups[g].id),
                "adjustment": adjustment_json(&p.adjustment),
                "constraints": p.constraints.iter().map(|c| match *c {
                    PolicyConstraint::Cooldown { time, max_operations } =>
                        json!({"type": "CooldownConstraint", "cooldown_time": time,
                               "max_scaling_operations": max_operations}),
                    PolicyConstraint::Interval { offset, duration, repeat } =>
                        json!({"type": "IntervalConstraint", "offset": offset,
                               "duration": duration, "repeat": repeat}),
                    PolicyConstraint::Unsupported(c) => json!({"type": c}),
                }).collect::<Vec<_>>(),
            })
        })
        .collect();
    let autoscalers: Vec<Value> = hpa
        .map(|h| {
            h.autoscalers
                .iter()
                .map(|a| {
                    json!({
                        "name": a.cfg.name,
                        "target_group": &*m.groups[a.group].id,
                        "policy": &*m.policies[a.policy].id,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "spd": &*m.id,
        "target_groups": groups,
        "policies": policies,
        "autoscalers": autoscalers,
    })
}

/// The model file of `cm` (the model a run starts from): its definitions and deployment, with
/// the target groups and scaling definitions of `spd` and the autoscalers of `hpa`.
pub fn model_file(
    cm: &simoxide_sim::CompiledModel,
    spd: Option<&Spd>,
    hpa: Option<&crate::hpa::Hpa>,
) -> Value {
    let mut d = simoxide_sim::inspect::deployment(cm, &cm.model);
    if let Some(s) = spd {
        add_target_groups(&cm.model.graph, s.model(), &mut d);
    }
    let scaling = spd.map_or(Value::Null, |s| scaling_definitions(s, hpa));
    simoxide_sim::inspect::model_file(cm, &d, scaling)
}

/// The policy state of a run (`docs/guide/runtime-metadata.md`, "Policy state"): per policy its
/// adjustor state (active policies; inactive ones have none), per target group its size and last
/// enactment, the decisions made and the pending one (`crate::decision`), per HPA autoscaler its
/// replicas and bounds.
pub fn policy_state<C: Compat, F: Flavour>(r: &SpdRun<'_, '_, C, F>) -> Value {
    let spd = r.spd;
    let m = spd.model();
    let st: &SpdState = &r.state;
    let policies: Vec<Value> = m
        .policies
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let ctx = spd.contexts().iter().position(|c| c.policy == i);
            let state = ctx.and_then(|c| st.policies.get(c)).map(|s| {
                let a = &s.adjustor;
                json!({
                    "scales": a.scales,
                    "latest_adjustment": (a.scales > 0).then_some(a.latest_adjustment),
                    "cooldown_end": a.cooldown_end,
                    "scales_in_cooldown": a.scales_in_cooldown,
                })
            });
            json!({
                "id": &*p.id,
                "active": p.active,
                "state": state,
            })
        })
        .collect();
    let groups: Vec<Value> = m
        .groups
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let gs = st.groups.get(i);
            json!({
                "id": &*g.id,
                "size": st.membership.group_size(g),
                "enactments": gs.map_or(0, |s| s.enactments),
                "last_enactment": gs.and_then(|s| s.last).map(|(t, p)| json!({
                    "time": t, "policy": &*m.policies[p].id,
                })),
            })
        })
        .collect();
    let autoscalers: Vec<Value> = match r.hpa {
        Some(h) => h
            .autoscalers
            .iter()
            .zip(&r.hpa_state.autoscalers)
            .map(|(a, s)| {
                json!({
                    "name": a.cfg.name,
                    "min_replicas": s.spec.min_replicas,
                    "max_replicas": s.spec.max_replicas,
                    "target_utilization": s.spec.target_utilization,
                    "ready": s.pods.len(),
                    "starting": s.replicas.pending.len(),
                    "rescales": s.rescales,
                })
            })
            .collect(),
        None => Vec::new(),
    };
    json!({
        "policies": policies,
        "target_groups": groups,
        "decisions_made": r.decisions_made(),
        "pending_decision": r.pending_decision().map(|d| d.to_json()),
        "autoscalers": autoscalers,
    })
}

/// What made an adjustment (`docs/guide/runtime-metadata.md`, "Changes"): a policy's own
/// recommendation (`policy`), an action selected for a decision (`decision`), an enforced policy
/// (`enforce`) or an HPA rescale (`hpa`).
pub fn cause(spd: &Spd, req: &AdjustmentRequest) -> Value {
    let m = spd.model();
    let mut c = Map::new();
    let kind = if let Some(i) = req.hpa {
        c.insert("autoscaler".into(), json!(i));
        "hpa"
    } else if let Some(d) = req.decision {
        c.insert("decision".into(), json!(d));
        "decision"
    } else if req.enforced {
        "enforce"
    } else {
        "policy"
    };
    c.insert("type".into(), json!(kind));
    c.insert("policy".into(), json!(&*m.policies[req.policy].id));
    c.insert("target_group".into(), json!(&*m.groups[req.group].id));
    c.insert("request".into(), json!(req.id));
    c.insert("adjustment".into(), adjustment_json(&req.adjustment));
    Value::Object(c)
}

/// An adjustment's outcome: success and the group's sizes before and after.
pub fn outcome(out: &AdjustOutcome) -> Value {
    json!({"ok": out.ok, "size_before": out.size_before, "size_after": out.size_after})
}

/// An adjuster that runs `inner` and records each adjustment in `rec`: the deployment after it,
/// at the simulation time of the adjustment, with its cause and outcome.
pub struct Recording<'a, C: Compat, F: Flavour> {
    pub inner: &'a mut dyn Adjuster<C, F>,
    pub rec: &'a mut Recorder,
}

impl<C: Compat, F: Flavour> Adjuster<C, F> for Recording<'_, C, F> {
    fn adjust(
        &mut self,
        sim: &mut Simulation<'_, C, F>,
        spd: &Spd,
        state: &mut SpdState,
        req: &AdjustmentRequest,
    ) -> Result<AdjustOutcome, SimError> {
        let out = self.inner.adjust(sim, spd, state, req)?;
        let d = deployment(&*sim, Some(spd.model()));
        self.rec.record(
            d,
            sim.now_ns(),
            sim.model_generation(),
            cause(spd, req),
            outcome(&out),
        );
        Ok(out)
    }
}

//! Kubernetes-style horizontal pod autoscaling (HPA) of SPD target groups
//! (`docs/spec/hpa.md`, `docs/guide/hpa.md`, `DESIGN-SPD.md` §5.10).
//!
//! An autoscaler ([`config::AutoscalerConfig`], from a Kubernetes `autoscaling/v2`
//! `HorizontalPodAutoscaler` with a CPU utilisation target) scales one target group of the
//! model's SPD (an elastic infrastructure: the containers are the pods; a service group: the
//! assemblies, on their containers). A native, plain-data controller ([`controller`]) runs at
//! its sync period: it reads each pod's CPU utilisation over the metric window from the engine's
//! busy-time integrals (no monitor needed), computes Kubernetes' desired replica count
//! (proportional rule, tolerance, unready pods, stabilisation, scaling policies, min/max), and
//! executes a rescale as an SPD **absolute adjustment** of the group through the run's adjuster
//! (Slingshot's `MainTransformation.qvto`): the structural change is the one SPD policies make.
//! New pods become ready after a start-up delay (the adjustment then runs).
//!
//! [`attach`] adds one inactive SPD policy per autoscaler to the model graph (an
//! `AbsoluteAdjustment` of its group, `id` [`policy_id`]): the policy the transformation enacts.
//! [`Hpa::new`] resolves the configuration against the SPD interpreter; `SpdRun::with_hpa`
//! (feature `sim`) runs it.

pub mod config;
pub mod controller;
#[cfg(feature = "sim")]
pub(crate) mod run;

pub use config::{AutoscalerConfig, ControllerConfig, HpaConfig, TargetRef};
pub use controller::{Decision, HpaMemory, Pod, PodState, Replicas, Spec};
#[cfg(feature = "sim")]
pub use run::{AutoscalerState, HpaState, HpaSummary, PodInfo};

use crate::interp::Spd;
use crate::model::{GroupKind, SpdModel};
use simoxide_model::Model;
use simoxide_model::meta::{class, feat};
use simoxide_model::raw::{Graph, ObjId};

/// The `id` of the inactive policy [`attach`] adds for the autoscaler `name`.
pub fn policy_id(name: &str) -> String {
    format!("_simoxide_hpa_{name}")
}

/// Adds the autoscalers' policies to `model`'s SPD (see the module documentation); before the
/// model is compiled. Policies already there (attached before) are kept.
pub fn attach(model: &mut Model, cfg: &HpaConfig) -> Result<(), String> {
    attach_graph(&mut model.graph, cfg)
}

/// [`attach`] on a model graph.
pub fn attach_graph(g: &mut Graph, cfg: &HpaConfig) -> Result<(), String> {
    use simoxide_emf_core::{Datum, ModelStore};
    if cfg.autoscalers.is_empty() {
        return Ok(());
    }
    let spd = SpdModel::from_graph(g).ok_or_else(|| {
        "the HPA scales target groups of an SPD: the model has no .spd (and .semanticspd)"
            .to_string()
    })?;
    if spd.configuration.is_none() {
        return Err(
            "the HPA's adjustments need the SPD's semantic configuration (.semanticspd)".into(),
        );
    }
    let mut groups_seen = Vec::new();
    for a in &cfg.autoscalers {
        let gi = resolve_group(&spd, &a.target, &a.name)?;
        if groups_seen.contains(&gi) {
            return Err(format!(
                "HorizontalPodAutoscaler {:?}: two autoscalers scale the target group {}",
                a.name, spd.groups[gi].name
            ));
        }
        groups_seen.push(gi);
        let id = policy_id(&a.name);
        if spd.policies.iter().any(|p| *p.id == *id) {
            continue;
        }
        let e = |e: simoxide_emf_core::ModelError| format!("attaching the HPA policy: {e:?}");
        let adj = g.create(class::AbsoluteAdjustment).map_err(e)?;
        g.set(adj, feat::AbsoluteAdjustment_goalValue, Datum::Int(1))
            .map_err(e)?;
        let p = g.create(class::ScalingPolicy).map_err(e)?;
        g.set(p, feat::identifier_Identifier_id, Datum::Str(&id))
            .map_err(e)?;
        let name = format!("HPA {}", a.name);
        g.set(p, feat::NamedElement_entityName, Datum::Str(&name))
            .map_err(e)?;
        g.set(p, feat::ScalingPolicy_active, Datum::Bool(false))
            .map_err(e)?;
        g.set(
            p,
            feat::ScalingPolicy_targetGroup,
            Datum::Obj(spd.groups[gi].obj),
        )
        .map_err(e)?;
        g.set(p, feat::ScalingPolicy_adjustmentType, Datum::Obj(adj))
            .map_err(e)?;
        g.add(spd.obj, feat::SPD_scalingPolicies, Datum::Obj(p), None)
            .map_err(e)?;
    }
    Ok(())
}

fn resolve_group(spd: &SpdModel, t: &TargetRef, hpa: &str) -> Result<usize, String> {
    let at = format!("HorizontalPodAutoscaler {hpa:?}: scaleTargetRef");
    let found: Vec<usize> = spd
        .groups
        .iter()
        .enumerate()
        .filter(|(_, g)| *g.name == *t.name || *g.id == *t.name)
        .map(|(i, _)| i)
        .collect();
    let gi = match found.as_slice() {
        [i] => *i,
        [] => {
            let names: Vec<&str> = spd.groups.iter().map(|g| &*g.name).collect();
            return Err(format!(
                "{at}: no target group named {:?} (the SPD has: {})",
                t.name,
                names.join(", ")
            ));
        }
        _ => {
            return Err(format!(
                "{at}: several target groups are named {:?}",
                t.name
            ));
        }
    };
    let g = &spd.groups[gi];
    let kind = match g.kind {
        GroupKind::ElasticInfrastructure { .. } => "ElasticInfrastructure",
        GroupKind::ServiceGroup { .. } => "ServiceGroup",
        _ => {
            return Err(format!(
                "{at}: {} is not an elastic infrastructure or a service group",
                g.name
            ));
        }
    };
    if let Some(k) = &t.kind
        && k != kind
        && k != "TargetGroup"
    {
        return Err(format!("{at}: {} is a {kind}, not a {k}", g.name));
    }
    Ok(gi)
}

/// One resolved autoscaler.
#[derive(Clone, Debug)]
pub struct Autoscaler {
    pub cfg: AutoscalerConfig,
    /// Its target group (index into [`SpdModel::groups`]) and policy (into
    /// [`SpdModel::policies`]).
    pub group: usize,
    pub policy: usize,
    /// The group's elements are containers (elastic infrastructure), not assemblies.
    pub containers: bool,
}

/// The autoscalers of a run, resolved against its SPD interpreter. Immutable and shared, as
/// [`Spd`]; the run state is `SpdRun::hpa_state`.
#[derive(Clone, Debug)]
pub struct Hpa {
    pub autoscalers: Vec<Autoscaler>,
}

impl Hpa {
    /// Resolves `cfg` against `spd`, which must be built from a model [`attach`]ed with it.
    pub fn new(cfg: &HpaConfig, spd: &Spd) -> Result<Hpa, String> {
        let m = spd.model();
        let mut v = Vec::with_capacity(cfg.autoscalers.len());
        for a in &cfg.autoscalers {
            let group = resolve_group(m, &a.target, &a.name)?;
            let id = policy_id(&a.name);
            let policy = m.policies.iter().position(|p| *p.id == *id).ok_or_else(|| {
                format!(
                    "HorizontalPodAutoscaler {:?}: its policy is not in the model (call hpa::attach before compiling)",
                    a.name
                )
            })?;
            if v.len() >= 1 << 16 {
                return Err("too many autoscalers".into());
            }
            v.push(Autoscaler {
                cfg: a.clone(),
                group,
                policy,
                containers: matches!(
                    m.groups[group].kind,
                    GroupKind::ElasticInfrastructure { .. }
                ),
            });
        }
        Ok(Hpa { autoscalers: v })
    }

    pub fn is_empty(&self) -> bool {
        self.autoscalers.is_empty()
    }

    /// The autoscaler named `name`.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.autoscalers.iter().position(|a| a.cfg.name == name)
    }
}

/// The adjustment object of an HPA policy (for writing the goal before the transformation).
pub fn adjustment_object(g: &Graph, policy: ObjId) -> Option<ObjId> {
    g.get_ref(policy, feat::ScalingPolicy_adjustmentType)
}

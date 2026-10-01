//! Validation of structural properties the simulator relies on. EMF performs none of these checks
//! on load; the reference would fail (or behave oddly) at simulation time instead.
//!
//! Kinds reported (all as [`Diagnostic`](crate::Diagnostic)s on the model):
//! - `missing-required`: a reference with lower bound 1 is unset (generic, all classes);
//! - `empty-stoex`: a required `PCMRandomVariable` has no specification;
//! - `behaviour-structure`: start/stop/successor chain problems in SEFF and usage behaviours;
//! - `role-mismatch`, `signature-mismatch`: calls through roles/interfaces that do not fit;
//! - `unconnected-role`: required role of an assembly context without connector;
//! - `unallocated`: system assembly context without allocation context;
//! - `branch-probabilities`: probabilities not summing to 1;
//! - `workload`: usage scenario without (valid) workload;
//! - `duplicate-id`: an ID used twice in one resource;
//! - `scheduling-policy`: a policy outside ProcessorSharing/FCFS/Delay.

use crate::diag::Level;
use crate::fxhash::FxHashMap;
use crate::model::*;
use crate::raw::{ObjId, ResId};
use std::collections::HashSet;

struct V<'a> {
    m: &'a Model,
    out: Vec<(Level, &'static str, ObjId, String)>,
}

impl V<'_> {
    fn push(&mut self, l: Level, kind: &'static str, o: ObjId, msg: impl Into<String>) {
        self.out.push((l, kind, o, msg.into()));
    }

    fn rv(&mut self, o: ObjId, what: &str, r: &RandomVar) {
        if r.is_missing() {
            // reported by missing-required
            return;
        }
        if r.spec.trim().is_empty() {
            self.push(
                Level::Error,
                "empty-stoex",
                o,
                format!("{what} has an empty specification"),
            );
        }
    }

    fn generic(&mut self) {
        let g = &self.m.graph;
        let (mut order, mut stack) = (Vec::new(), Vec::new());
        let mut ids: FxHashMap<&str, ObjId> = FxHashMap::default();
        for r in 0..g.resources.len() {
            ids.clear();
            order.clear();
            g.all_contents_into(ResId(r as u32), &mut order, &mut stack);
            for &o in &order {
                if let Some(id) = g.id(o)
                    && let Some(prev) = ids.insert(id, o)
                {
                    let msg = format!("ID {id:?} also used by {}", g.describe(prev));
                    self.push(Level::Warning, "duplicate-id", o, msg);
                }
                for &f in g[o].class.required_references() {
                    if g.get_refs(o, f).is_empty() {
                        self.push(
                            Level::Warning,
                            "missing-required",
                            o,
                            format!("required reference {} is not set", f.name()),
                        );
                    }
                }
            }
        }
    }

    fn behaviours(&mut self) {
        let m = self.m;
        for (bid, b) in m.behaviours.iter_ids() {
            let starts = b
                .steps
                .iter()
                .filter(|a| matches!(m.actions[**a].kind, ActionKind::Start))
                .count();
            let stops = b
                .steps
                .iter()
                .filter(|a| matches!(m.actions[**a].kind, ActionKind::Stop))
                .count();
            if b.steps.is_empty() {
                // empty behaviours (e.g. unused branch bodies) are legal but cannot be executed
                self.push(
                    Level::Warning,
                    "behaviour-structure",
                    b.obj,
                    "behaviour has no actions",
                );
                continue;
            }
            if starts != 1 || stops != 1 {
                self.push(
                    Level::Error,
                    "behaviour-structure",
                    b.obj,
                    format!(
                        "expected exactly one start and one stop action, found {starts} and {stops}"
                    ),
                );
            }
            let chain = m.action_chain(bid);
            if let Some(last) = chain.last() {
                let a = &m.actions[*last];
                if a.successor.is_some() {
                    self.push(
                        Level::Error,
                        "behaviour-structure",
                        a.obj,
                        "successor chain contains a cycle",
                    );
                } else if !matches!(a.kind, ActionKind::Stop) {
                    self.push(
                        Level::Error,
                        "behaviour-structure",
                        a.obj,
                        "successor chain ends before a stop action",
                    );
                }
            }
            for a in &chain {
                if m.actions[*a].behaviour != Some(bid) {
                    self.push(
                        Level::Error,
                        "behaviour-structure",
                        m.actions[*a].obj,
                        "successor leaves its behaviour",
                    );
                }
            }
            if chain.len() < b.steps.len() {
                // in step order (deterministic), each action once
                let mut seen = HashSet::new();
                for a in b
                    .steps
                    .iter()
                    .filter(|a| !chain.contains(a) && seen.insert(**a))
                {
                    self.push(
                        Level::Warning,
                        "behaviour-structure",
                        m.actions[*a].obj,
                        "action not reachable from start",
                    );
                }
            }
        }
        for (bid, b) in m.scenario_behaviours.iter_ids() {
            let starts = b
                .actions
                .iter()
                .filter(|a| matches!(m.user_actions[**a].kind, UserActionKind::Start))
                .count();
            let stops = b
                .actions
                .iter()
                .filter(|a| matches!(m.user_actions[**a].kind, UserActionKind::Stop))
                .count();
            if starts != 1 || stops != 1 {
                self.push(
                    Level::Error,
                    "behaviour-structure",
                    b.obj,
                    format!(
                        "expected exactly one start and one stop action, found {starts} and {stops}"
                    ),
                );
            }
            let chain = m.user_action_chain(bid);
            if let Some(last) = chain.last() {
                let a = &m.user_actions[*last];
                if a.successor.is_some() {
                    self.push(
                        Level::Error,
                        "behaviour-structure",
                        a.obj,
                        "successor chain contains a cycle",
                    );
                } else if !matches!(a.kind, UserActionKind::Stop) {
                    self.push(
                        Level::Error,
                        "behaviour-structure",
                        a.obj,
                        "successor chain ends before a stop action",
                    );
                }
            }
            if chain.len() < b.actions.len() {
                for a in b.actions.iter().filter(|a| !chain.contains(a)) {
                    self.push(
                        Level::Warning,
                        "behaviour-structure",
                        m.user_actions[*a].obj,
                        "action not reachable from start",
                    );
                }
            }
        }
    }

    /// Signatures of an interface including inherited ones.
    fn interface_signatures(
        &self,
        i: InterfaceId,
        out: &mut Vec<SignatureId>,
        seen: &mut Vec<InterfaceId>,
    ) {
        if seen.contains(&i) {
            return;
        }
        seen.push(i);
        let it = &self.m.interfaces[i];
        out.extend(&it.signatures);
        for p in &it.parents {
            self.interface_signatures(*p, out, seen);
        }
    }

    fn check_call(
        &mut self,
        o: ObjId,
        role: Option<RoleId>,
        sig: Option<SignatureId>,
        want: RoleKind,
    ) {
        let (Some(r), Some(s)) = (role, sig) else {
            return;
        };
        let role = &self.m.roles[r];
        if role.kind != want {
            self.push(
                Level::Error,
                "role-mismatch",
                o,
                format!("role {} is {:?}, expected {want:?}", role.name, role.kind),
            );
            return;
        }
        if let Some(i) = role.interface {
            let mut sigs = Vec::new();
            self.interface_signatures(i, &mut sigs, &mut Vec::new());
            if !sigs.contains(&s) {
                let msg = format!(
                    "signature {} is not in interface {} of role {}",
                    self.m.signatures[s].name, self.m.interfaces[i].name, role.name
                );
                self.push(Level::Error, "signature-mismatch", o, msg);
            }
        }
    }

    fn actions(&mut self) {
        let m = self.m;
        for a in m.actions.iter() {
            for d in &a.resource_demands {
                self.rv(d.obj, "resource demand", &d.spec);
            }
            match &a.kind {
                ActionKind::ExternalCall {
                    signature, role, ..
                } => {
                    self.check_call(a.obj, *role, *signature, RoleKind::OperationRequired);
                    // the role must belong to the component owning the SEFF
                    if let (Some(r), Some(b)) = (role, a.behaviour) {
                        let comp = self.seff_component(b);
                        if let (Some(cmp), RoleOwner::Component(owner)) = (comp, m.roles[*r].owner)
                            && cmp != owner
                        {
                            self.push(
                                Level::Error,
                                "role-mismatch",
                                a.obj,
                                "required role belongs to another component",
                            );
                        }
                    }
                }
                ActionKind::Branch { transitions } => {
                    let probs: Vec<f64> = transitions
                        .iter()
                        .filter_map(|t| match t.condition {
                            BranchCondition::Probability(p) => Some(p),
                            _ => None,
                        })
                        .collect();
                    if !probs.is_empty() && probs.len() == transitions.len() {
                        let s: f64 = probs.iter().sum();
                        if (s - 1.0).abs() > 1e-9 {
                            self.push(
                                Level::Warning,
                                "branch-probabilities",
                                a.obj,
                                format!("branch probabilities sum to {s}"),
                            );
                        }
                    }
                    for t in transitions {
                        if let BranchCondition::Guard(g) = &t.condition {
                            self.rv(t.obj, "branch condition", g);
                        }
                    }
                }
                ActionKind::Loop { iterations, .. } => {
                    self.rv(a.obj, "iteration count", iterations)
                }
                _ => {}
            }
        }
    }

    fn seff_component(&self, mut b: BehaviourId) -> Option<ComponentId> {
        for _ in 0..1000 {
            match self.m.behaviours[b].owner {
                BehaviourOwner::Seff(s) | BehaviourOwner::Internal(s) => {
                    return self.m.seffs[s].component;
                }
                BehaviourOwner::Loop(a)
                | BehaviourOwner::BranchTransition(a, _)
                | BehaviourOwner::ForkedAsync(a)
                | BehaviourOwner::ForkedSync(a) => b = self.m.actions[a].behaviour?,
                BehaviourOwner::Other => return None,
            }
        }
        None
    }

    fn structures(&mut self) {
        let m = self.m;
        for s in m.structures.iter() {
            // required operation roles of every assembly context must be connected
            let mut connected: HashSet<(AssemblyContextId, RoleId)> = HashSet::new();
            for c in &s.connectors {
                match &m.connectors[*c].kind {
                    ConnectorKind::Assembly {
                        requiring: Some(ac),
                        required_role: Some(r),
                        ..
                    }
                    | ConnectorKind::AssemblyInfrastructure {
                        requiring: Some(ac),
                        required_role: Some(r),
                        ..
                    }
                    | ConnectorKind::RequiredDelegation {
                        assembly: Some(ac),
                        inner_role: Some(r),
                        ..
                    }
                    | ConnectorKind::RequiredInfrastructureDelegation {
                        assembly: Some(ac),
                        inner_role: Some(r),
                        ..
                    } => {
                        connected.insert((*ac, *r));
                    }
                    _ => {}
                }
            }
            for ac in &s.assembly_contexts {
                let Some(comp) = m.assembly_contexts[*ac].component else {
                    continue;
                };
                for r in &m.components[comp].required_roles {
                    if m.roles[*r].kind == RoleKind::OperationRequired
                        && !connected.contains(&(*ac, *r))
                    {
                        let msg = format!("required role {} is not connected", m.roles[*r].name);
                        self.push(
                            Level::Warning,
                            "unconnected-role",
                            m.assembly_contexts[*ac].obj,
                            msg,
                        );
                    }
                }
            }
        }
    }

    fn allocations(&mut self) {
        let m = self.m;
        for al in m.allocations.iter() {
            let Some(sys) = al.system else { continue };
            let allocated: HashSet<AssemblyContextId> = al
                .contexts
                .iter()
                .filter_map(|c| m.allocation_contexts[*c].assembly)
                .collect();
            let mut missing = Vec::new();
            for ac in &m.structures[m.systems[sys].structure].assembly_contexts {
                Self::unallocated(m, *ac, &allocated, &mut missing, 0);
            }
            for ac in missing {
                let msg = format!(
                    "assembly context {} is not allocated in {}",
                    m.assembly_contexts[ac].name, al.name
                );
                self.push(
                    Level::Error,
                    "unallocated",
                    m.assembly_contexts[ac].obj,
                    msg,
                );
            }
        }
    }

    /// An assembly context is allocated directly or, for a subsystem, through all of its inner
    /// assembly contexts.
    fn unallocated(
        m: &Model,
        ac: AssemblyContextId,
        allocated: &HashSet<AssemblyContextId>,
        out: &mut Vec<AssemblyContextId>,
        depth: u32,
    ) {
        if allocated.contains(&ac) || depth > 64 {
            return;
        }
        match m.assembly_contexts[ac]
            .component
            .map(|c| &m.components[c].kind)
        {
            Some(ComponentKind::SubSystem { structure }) => {
                for inner in &m.structures[*structure].assembly_contexts {
                    Self::unallocated(m, *inner, allocated, out, depth + 1);
                }
            }
            _ => out.push(ac),
        }
    }

    fn usage(&mut self) {
        let m = self.m;
        for s in m.usage_scenarios.iter() {
            match &s.workload {
                Workload::Missing => self.push(
                    Level::Error,
                    "workload",
                    s.obj,
                    "usage scenario has no workload",
                ),
                Workload::Closed {
                    population,
                    think_time,
                } => {
                    if *population < 1 {
                        self.push(
                            Level::Warning,
                            "workload",
                            s.obj,
                            format!("closed workload population {population}"),
                        );
                    }
                    self.rv(s.obj, "think time", think_time);
                }
                Workload::Open { inter_arrival_time } => {
                    self.rv(s.obj, "inter-arrival time", inter_arrival_time)
                }
            }
        }
        for a in m.user_actions.iter() {
            match &a.kind {
                UserActionKind::EntryLevelSystemCall {
                    role, signature, ..
                } => {
                    self.check_call(a.obj, *role, *signature, RoleKind::OperationProvided);
                    if let Some(r) = role
                        && !matches!(m.roles[*r].owner, RoleOwner::System(_))
                    {
                        self.push(
                            Level::Warning,
                            "role-mismatch",
                            a.obj,
                            "entry-level call role is not a system role",
                        );
                    }
                }
                UserActionKind::Branch { transitions } => {
                    let s: f64 = transitions.iter().map(|t| t.probability).sum();
                    if !transitions.is_empty() && (s - 1.0).abs() > 1e-9 {
                        self.push(
                            Level::Warning,
                            "branch-probabilities",
                            a.obj,
                            format!("branch probabilities sum to {s}"),
                        );
                    }
                }
                UserActionKind::Delay { time } => self.rv(a.obj, "delay", time),
                UserActionKind::Loop { iterations, .. } => {
                    self.rv(a.obj, "loop iterations", iterations)
                }
                _ => {}
            }
        }
    }

    fn resources(&mut self) {
        let m = self.m;
        for p in m.processing_resources.iter() {
            self.rv(p.obj, "processing rate", &p.processing_rate);
            if let Some(s) = p.scheduling {
                let id = &*m.scheduling_policies[s].id;
                if !matches!(id, "ProcessorSharing" | "FCFS" | "Delay") {
                    self.push(
                        Level::Warning,
                        "scheduling-policy",
                        p.obj,
                        format!("scheduling policy {id} is not simulated in v1"),
                    );
                }
            }
            if p.replicas < 1 {
                self.push(
                    Level::Error,
                    "resource",
                    p.obj,
                    format!("numberOfReplicas is {}", p.replicas),
                );
            }
        }
        for l in m.linking_resources.iter() {
            self.rv(l.obj, "latency", &l.latency);
            self.rv(l.obj, "throughput", &l.throughput);
        }
        for pr in m.passive_resources.iter() {
            self.rv(pr.obj, "passive resource capacity", &pr.capacity);
        }
    }
}

/// Runs all checks and appends the findings to `m.diagnostics`.
pub fn validate(m: &mut Model) {
    let mut v = V { m, out: Vec::new() };
    v.generic();
    v.behaviours();
    v.actions();
    v.structures();
    v.allocations();
    v.usage();
    v.resources();
    let out = v.out;
    for (l, k, o, msg) in out {
        let loc = m.graph.describe(o);
        m.diagnostics.push(l, k, loc, msg);
    }
}

//! A resumed SPD run (`DESIGN-SPD.md` N2): changes between the restore and the continuation.
//!
//! * [`SpdRun::model_changed`]: after a model change at the resume (a change file, a
//!   transformation), the target groups' membership is read from the changed model, as an
//!   adjustment does.
//! * [`SpdRun::enforce_policies`]: Slingshot's "incoming policies" (the MENTOR initialisation
//!   extension's `.config`, `SnapshotInitFromBehaviour`): adjustment requests of the named
//!   policies queued at the resume time, past their triggers and constraints; the run's
//!   adjuster executes them like the chain's requests (P2: `MainTransformation` on the run's
//!   model). Their outcome changes no adjustor or group state (Slingshot's adjustor never sees
//!   them).

use crate::model::Membership;
use crate::sim::{REQUEST, SpdRun};
use simoxide_sim::compat::Compat;
use simoxide_sim::flavour::Flavour;
use std::fmt;

/// Why policies could not be enforced. The run is unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnforceError {
    /// No policy of the SPD has this name or id.
    UnknownPolicy(String),
    /// The policy has no target group of this SPD (nothing to adjust).
    NoTargetGroup(String),
    /// The request could not be queued (the simulation refused the note).
    Schedule(String),
}

impl fmt::Display for EnforceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnforceError::UnknownPolicy(p) => write!(f, "no scaling policy named {p}"),
            EnforceError::NoTargetGroup(p) => {
                write!(f, "scaling policy {p} has no target group to adjust")
            }
            EnforceError::Schedule(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for EnforceError {}

impl<C: Compat, F: Flavour> SpdRun<'_, '_, C, F> {
    /// Re-reads the target groups' membership from the run's (changed) model.
    pub fn model_changed(&mut self) {
        self.state.membership =
            Membership::from_graph(&self.sim.compiled().model.graph, self.spd.model());
        self.observe_new_series();
    }

    /// Enforces the policies `names` (name or id, in this order) at the current time: their
    /// adjustments are requested now ([`crate::Spd::enforce`]), past triggers and constraints,
    /// as Slingshot's initialisation extension does with a state's incoming policies. The
    /// requests are queued behind the events already queued for now and executed by the
    /// adjuster of the run's next [`SpdRun::run_while`]; they appear in the decision log as
    /// `adjust` records with `"enforced":true`. On an error nothing is queued.
    pub fn enforce_policies(&mut self, names: &[&str]) -> Result<(), EnforceError> {
        let policies = &self.spd.model().policies;
        let mut idx = Vec::with_capacity(names.len());
        for n in names {
            let Some(i) = policies.iter().position(|p| &*p.name == *n || &*p.id == *n) else {
                return Err(EnforceError::UnknownPolicy(n.to_string()));
            };
            if policies[i].group.is_none() {
                return Err(EnforceError::NoTargetGroup(n.to_string()));
            }
            idx.push(i);
        }
        let now_ns = self.sim.state().now_ns();
        let now = now_ns as f64 / 1e9;
        for i in idx {
            let req = self.spd.enforce(&mut self.state, i, now);
            let tag = REQUEST | self.requests.len() as u32;
            self.requests.push(Some(req));
            self.sim
                .schedule_external(now_ns, tag)
                .map_err(|e| EnforceError::Schedule(e.message))?;
        }
        Ok(())
    }
}

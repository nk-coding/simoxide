//! A resumed SPD run (`DESIGN-SPD.md` N2): changes between the restore and the continuation.
//!
//! * [`SpdRun::model_changed`]: after a model change at the resume (a change file, a
//!   transformation), the target groups' membership is read from the changed model, as an
//!   adjustment does.
//! * [`SpdRun::enforce_policies`]: Slingshot's "incoming policies" (the MENTOR initialisation
//!   extension's `.config`, `SnapshotInitFromBehaviour`): adjustment requests of the named
//!   policies queued at the resume time, past their triggers and constraints. Executing an
//!   adjustment is phase P2 (`MainTransformation` on the run's model): until then the hook
//!   checks the names and returns [`EnforceError::Unsupported`], and the run is unchanged.

use crate::model::Membership;
use crate::sim::SpdRun;
use simoxide_sim::compat::Compat;
use std::fmt;

/// Why policies could not be enforced. The run is unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnforceError {
    /// No policy of the SPD has this name or id.
    UnknownPolicy(String),
    /// Enforcing needs the adjustment phase (P2).
    Unsupported(String),
}

impl fmt::Display for EnforceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnforceError::UnknownPolicy(p) => write!(f, "no scaling policy named {p}"),
            EnforceError::Unsupported(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for EnforceError {}

impl<C: Compat> SpdRun<'_, '_, C> {
    /// Re-reads the target groups' membership from the run's (changed) model.
    pub fn model_changed(&mut self) {
        self.state.membership =
            Membership::from_graph(&self.sim.compiled().model.graph, self.spd.model());
    }

    /// Enforces the policies `names` (name or id) at the current time: their adjustments are
    /// requested now, bypassing triggers and constraints, as Slingshot's initialisation
    /// extension does with a state's incoming policies. Not available before P2 (adjustments
    /// are not executed yet): [`EnforceError::Unsupported`] after the names are checked.
    pub fn enforce_policies(&mut self, names: &[&str]) -> Result<(), EnforceError> {
        let policies = &self.spd.model().policies;
        for n in names {
            if !policies.iter().any(|p| &*p.name == *n || &*p.id == *n) {
                return Err(EnforceError::UnknownPolicy(n.to_string()));
            }
        }
        Err(EnforceError::Unsupported(format!(
            "enforcing scaling policies at a resume ({}) needs the SPD adjustment phase \
             (DESIGN-SPD.md P2): adjustments are not executed yet",
            names.join(", ")
        )))
    }
}

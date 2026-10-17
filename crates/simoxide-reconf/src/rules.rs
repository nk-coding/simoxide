//! SimuLizar's reconfiguration rules (`QVTOReconfigurator`): a folder of `.qvto` rules, run one
//! after the other on a trigger (R1: the reconfiguration process, after a runtime measurement
//! write). This module runs them; when they run, the PRM and the reconfiguration process's
//! timing are R1's.

use crate::host::{Imports, Invocation, Outcome, QvtoCore, QvtoRun, ReconfError, Transformation};
use simoxide_sim::{Compat, Flavour, Simulation};
use std::path::Path;

/// The rules of a rule folder, compiled once (share it between runs).
#[derive(Clone, Debug, Default)]
pub struct RuleSet {
    pub rules: Vec<Transformation>,
}

impl RuleSet {
    /// The `.qvto` files directly in `dir`, in file-name order (SimuLizar loads the folder in
    /// file-system order; SimOxide sorts, `QVTOReconfigurationLoader`), compiled with imports
    /// next to the rule.
    pub fn load_dir(core: &QvtoCore, dir: &Path) -> Result<RuleSet, ReconfError> {
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .map_err(|e| ReconfError::Load(format!("{}: {e}", dir.display())))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "qvto"))
            .collect();
        files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
        let rules = files
            .iter()
            .map(|f| core.compile_file(f, &Imports::Folder))
            .collect::<Result<_, _>>()?;
        Ok(RuleSet { rules })
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

/// What a run of a rule set did.
#[derive(Clone, Debug, Default)]
pub struct RulesOutcome {
    /// One outcome per rule, in order.
    pub rules: Vec<Outcome>,
}

impl RulesOutcome {
    /// SimuLizar's result (`QVTOExecutor.executeTransformations`: `result &= ...`): every rule
    /// succeeded ("Reconfigured system by a matching rule"). A rule that fails an
    /// `assert fatal` is "not applied" and makes this false.
    pub fn all_succeeded(&self) -> bool {
        self.rules.iter().all(|o| o.succeeded)
    }

    /// Whether some rule changed the model.
    pub fn changed(&self) -> bool {
        self.rules.iter().any(|o| o.change.is_some())
    }
}

impl QvtoRun {
    /// Runs every rule of `rules` in order on the run's model, as SimuLizar's
    /// `QVTOReconfigurator.runExecute` does: each rule sees the changes of the rules before it
    /// (its changes are applied before the next rule runs); a failing rule does not stop the
    /// others. Stops at the first error ([`ReconfError`]: binding, rejected changes, a failed
    /// run); the outcomes of the rules before are in `done`.
    pub fn run_rules<C: Compat, F: Flavour>(
        &mut self,
        sim: &mut Simulation<'_, C, F>,
        rules: &RuleSet,
        inv: &Invocation,
    ) -> Result<RulesOutcome, (RulesOutcome, ReconfError)> {
        let mut done = RulesOutcome::default();
        for r in &rules.rules {
            match self.invoke(sim, r, inv) {
                Ok(o) => done.rules.push(o),
                Err(e) => return Err((done, e)),
            }
        }
        Ok(done)
    }
}

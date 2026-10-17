//! SPD adjustments by a QVTo transformation (`DESIGN-SPD.md` 5.4, P2): an implementation of
//! `simoxide_spd::Adjuster` that runs a transformation (Slingshot's `MainTransformation.qvto`)
//! on the run's model with Slingshot's bindings (by namespace URI: allocation, system, resource
//! environment, repository, semantic SPD and usage model `inout`, SPD `in`) and configuration
//! properties (`keepModeling = true`), and applies its changes.
//!
//! What is specific to `MainTransformation` goes into [`AdjustHooks`]: setting
//! `Configuration.enactedPolicy` before the run ([`AdjustHooks::before`], recorded in the run's
//! journal), Slingshot's `ModelAdjusted` diff, the model summaries and the outcome
//! ([`AdjustHooks::after`]): [`crate::spd::SlingshotHooks`], set up by
//! [`QvtoAdjuster::spd`]. The default hooks set nothing before and report the
//! transformation's success, refreshing the SPD membership from the changed model.

use crate::host::{Invocation, InvokeHooks, Outcome, QvtoRun, ReconfError, Transformation};
use simoxide_model::raw::Graph;
use simoxide_sim::{ChangeHost, Compat, Flavour, SimError, SimErrorKind, Simulation};
use simoxide_spd::sim::{AdjustOutcome, Adjuster};
use simoxide_spd::ssx::SsxAdjuster;
use simoxide_spd::{AdjustmentRequest, Membership, Spd, SpdState};

/// The parts of an adjustment that depend on the transformation (P2).
pub trait AdjustHooks {
    /// Called first for each request with the run's base model graph (the model the run
    /// started with, unchanged by adjustments; e.g. which elements are inputs).
    fn prepare(&mut self, _base: &Graph) {}

    /// Edits before the transformation runs, in its journal (P2: `enactedPolicy`).
    fn before(&mut self, _store: &mut Graph, _spd: &Spd, _req: &AdjustmentRequest) {}

    /// The adjustment's outcome, given the transformation's (`Err`: it was not applied, the run
    /// is unchanged, or the run failed). `graph` is the run's model graph afterwards. The
    /// default: `ok` when the transformation succeeded; the membership is refreshed from the
    /// graph; the target group's sizes before and after. An error ends the SPD run.
    fn after(
        &mut self,
        graph: &Graph,
        spd: &Spd,
        state: &mut SpdState,
        req: &AdjustmentRequest,
        outcome: Result<&Outcome, &ReconfError>,
    ) -> Result<AdjustOutcome, String> {
        let group = &spd.model().groups[req.group];
        let size_before = state.membership.group_size(group);
        match outcome {
            Ok(o) => {
                if o.change.is_some() {
                    state.membership = Membership::from_graph(graph, spd.model());
                }
                Ok(AdjustOutcome {
                    ok: o.succeeded,
                    size_before,
                    size_after: state.membership.group_size(group),
                    ..AdjustOutcome::default()
                })
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

/// [`AdjustHooks`] with the default behaviour.
pub struct DefaultAdjustHooks;

impl AdjustHooks for DefaultAdjustHooks {}

/// Runs a QVTo transformation for each SPD adjustment request (see the module documentation).
pub struct QvtoAdjuster {
    /// The run's QVTo state (fork it with the run: [`QvtoAdjuster::fork`]).
    pub run: QvtoRun,
    pub transformation: Transformation,
    pub invocation: Invocation,
    pub hooks: Box<dyn AdjustHooks + Send>,
    /// The outcome of the last transformation run.
    pub last: Option<Outcome>,
    /// Create the monitors the adjustment's `ModelAdjusted` lists (Slingshot's
    /// `ResourceContainerMonitorCloner`, [`crate::Edit::clone_monitor`]) as a change set right
    /// after the transformation's (default: on).
    pub clone_monitors: bool,
}

impl QvtoAdjuster {
    /// Slingshot's bindings (every parameter by namespace URI, all roots) and
    /// `keepModeling = true`, default hooks.
    pub fn new(run: QvtoRun, transformation: Transformation) -> QvtoAdjuster {
        QvtoAdjuster {
            run,
            transformation,
            invocation: Invocation::slingshot().config("keepModeling", "true"),
            hooks: Box::new(DefaultAdjustHooks),
            last: None,
            clone_monitors: true,
        }
    }

    pub fn with_hooks(mut self, hooks: impl AdjustHooks + Send + 'static) -> Self {
        self.hooks = Box::new(hooks);
        self
    }

    /// The adjuster of a fork of the run, with a copy of the run's QVTo state and the given
    /// hooks.
    pub fn fork(&self, hooks: impl AdjustHooks + Send + 'static) -> QvtoAdjuster {
        QvtoAdjuster {
            run: self.run.clone(),
            transformation: self.transformation.clone(),
            invocation: self.invocation.clone(),
            hooks: Box::new(hooks),
            last: self.last.clone(),
            clone_monitors: self.clone_monitors,
        }
    }
}

impl<C: Compat, F: Flavour> Adjuster<C, F> for QvtoAdjuster {
    fn adjust(
        &mut self,
        sim: &mut Simulation<'_, C, F>,
        spd: &Spd,
        state: &mut SpdState,
        req: &AdjustmentRequest,
    ) -> Result<AdjustOutcome, SimError> {
        self.adjust_on(sim, spd, state, req)
    }
}

/// The Slingshot-exact engine's adjustments (`DESIGN-SPD.md` X2d): the same transformation run
/// on the engine's model inside its `ModelAdjustmentRequested` handler.
impl SsxAdjuster for QvtoAdjuster {
    fn adjust(
        &mut self,
        host: &mut dyn ChangeHost,
        spd: &Spd,
        state: &mut SpdState,
        req: &AdjustmentRequest,
    ) -> Result<AdjustOutcome, SimError> {
        self.adjust_on(host, spd, state, req)
    }
}

impl QvtoAdjuster {
    /// Runs the transformation for `req` on the run `sim` (a simulation or the Slingshot-exact
    /// engine) and applies its changes, then the cloned monitors.
    pub fn adjust_on<H: ChangeHost + ?Sized>(
        &mut self,
        sim: &mut H,
        spd: &Spd,
        state: &mut SpdState,
        req: &AdjustmentRequest,
    ) -> Result<AdjustOutcome, SimError> {
        self.hooks.prepare(&sim.base_compiled().model.graph);
        let hooks = &mut self.hooks;
        let mut before = |s: &mut Graph| hooks.before(s, spd, req);
        let r = self.run.invoke_with(
            sim,
            &self.transformation,
            &self.invocation,
            InvokeHooks {
                before: Some(&mut before),
                ids: None,
            },
        );
        let at_ns = sim.now_ns();
        if let Err(ReconfError::RunFailed { error, .. }) = &r {
            return Err(error.clone());
        }
        let out = self
            .hooks
            .after(sim.model_graph(), spd, state, req, r.as_ref())
            .map_err(|message| SimError {
                message: format!(
                    "SPD adjustment by {}: {message}",
                    self.transformation.name()
                ),
                at_ns,
                kind: SimErrorKind::Model,
            });
        // the monitors Slingshot clones for new containers (F4: the change path attaches them)
        if self.clone_monitors
            && let Ok(o) = &out
            && let Some(adj) = &o.adjusted
            && !adj.monitor_clones.is_empty()
        {
            let generation = sim.model_generation();
            let mut e = crate::Edit::new(sim);
            let mut made = Ok(());
            for (k, c) in adj.monitor_clones.iter().enumerate() {
                if let Err(err) =
                    e.clone_monitor(c.monitor, c.target, &format!("_spd_g{generation}_mon{k}"))
                {
                    made = Err(err.0);
                    break;
                }
            }
            made.and_then(|()| e.apply(sim).map(|_| ()).map_err(|e| e.to_string()))
                .map_err(|message| SimError {
                    message: format!("SPD adjustment: cloning monitors: {message}"),
                    at_ns,
                    kind: SimErrorKind::Model,
                })?;
        }
        self.last = match r {
            Ok(o) => Some(o),
            Err(ReconfError::Rejected { outcome, .. }) => Some(*outcome),
            Err(_) => None,
        };
        out
    }
}

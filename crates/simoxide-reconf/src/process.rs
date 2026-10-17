//! SimuLizar's reconfiguration process with rules (DESIGN-SPD R1; spec MEAS-7.2, 7.3, 7.5 and
//! `docs/spec/model-changes.md`).
//!
//! The simulator keeps the runtime measurement values (PRM) as plain run state and pauses the
//! run, between two events, whenever its reconfiguration process runs
//! ([`Simulation::attach_reconfiguration`]). [`RulesHost`] is that process's host: it mirrors the
//! PRM values into a runtime measurement model of the run's QVTo state (one `RuntimeMeasurement`
//! per PRM recorder, SimuLizar's order, referring to the monitor's measuring point and the
//! measurement specification), runs the rules in file-name order ([`QvtoRun::run_rules`]: each
//! rule sees the changes of the rules before it, which are applied to the simulation right
//! away), and reports SimuLizar's result (`QVTOExecutor.executeTransformations`: `result &=` over
//! every rule; a rule that terminates, through `assert fatal` or an exception, is "not
//! applied", its changes so far stay) with [`Simulation::reconf_done`]. [`RulesRun`] drives a
//! simulation with a host.
//!
//! What SimuLizar would abort on aborts the run as well: a rule whose parameters cannot be
//! bound, a change the syncers throw on ([`crate::AbortPolicy::FailRun`]). Changes SimOxide does
//! not support yet (monitor, population and structural behaviour changes: F4) or that its
//! model validation refuses also end the run with an error that names the rule, instead of
//! continuing with a model that differs from SimuLizar's.

use crate::host::{Invocation, ModelRef, QvtoCore, QvtoRun, ReconfError, RunConfig};
use crate::qvt::PRM_NS;
use crate::rules::{RuleSet, RulesOutcome};
use qvto_emf::ObjId;
use qvto_model::{Datum, ModelStore};
use simoxide_sim::{
    Compat, DefaultCompat, Flavour, Outputs, Pause, RunResult, RunState, SimError, SimuLizar,
    Simulation, Status,
};
use std::time::Instant;

/// One run of the reconfiguration process with rules.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct ReconfRecord {
    /// Simulation time (ns).
    pub at_ns: i64,
    /// A run after the stop (inside the finalisation).
    pub post_stop: bool,
    /// SimuLizar's result: every rule applied.
    pub succeeded: bool,
    /// Per rule, in order: name, applied, changed the model, its QVTo log.
    pub rules: Vec<RuleRecord>,
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct RuleRecord {
    pub name: String,
    pub succeeded: bool,
    pub changed: bool,
    pub log: String,
}

/// Counters of a run's reconfigurations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct RulesStats {
    /// Runs of the reconfiguration process.
    pub runs: u64,
    /// Successful ones (every rule applied).
    pub succeeded: u64,
    /// Runs in which some rule changed the model.
    pub changed: u64,
    /// Wall time in the host: PRM mirror and rules (µs).
    pub micros: u64,
    /// Of which the PRM mirror (µs).
    pub prm_micros: u64,
}

/// The PRM mirror in the run's QVTo state: the model's root and its measurements, in PRM
/// order (objects of the run's model graph).
#[derive(Clone, Debug, Default)]
pub(crate) struct PrmMirror {
    pub(crate) model: Option<(ModelRef, ObjId)>,
    pub(crate) meas: Vec<ObjId>,
}

/// The host of a run's reconfiguration process: the run's QVTo state, the rules and the PRM
/// mirror. `Clone` is the host of a fork.
#[derive(Clone)]
pub struct RulesHost {
    pub(crate) run: QvtoRun,
    pub(crate) rules: RuleSet,
    inv: Invocation,
    pub(crate) prm: PrmMirror,
    pub(crate) stats: RulesStats,
    /// Records of the reconfigurations, when kept ([`RulesHost::keep_records`]).
    pub(crate) records: Option<Vec<ReconfRecord>>,
}

/// The URI of the PRM model in the run's QVTo state.
pub const PRM_URI: &str = "simoxide:/runtime.runtimemeasurement";

impl RulesHost {
    /// A host running `rules` (bound SimuLizar's way, [`Invocation::simulizar`]) with a run's
    /// QVTo state configured by `cfg`.
    pub fn new(core: &QvtoCore, rules: RuleSet, cfg: RunConfig) -> Result<RulesHost, ReconfError> {
        Ok(RulesHost {
            run: QvtoRun::new(core, cfg)?,
            rules,
            inv: Invocation::simulizar(),
            prm: PrmMirror::default(),
            stats: RulesStats::default(),
            records: None,
        })
    }

    /// Keeps a [`ReconfRecord`] of every run of the reconfiguration process (with the rules'
    /// logs).
    pub fn keep_records(mut self) -> Self {
        self.records = Some(Vec::new());
        self
    }

    /// The records kept so far (oldest first), and empties them.
    pub fn take_records(&mut self) -> Vec<ReconfRecord> {
        self.records
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    }

    /// The records kept so far (`None`: not kept), e.g. to put taken ones back.
    pub fn records_mut(&mut self) -> Option<&mut Vec<ReconfRecord>> {
        self.records.as_mut()
    }

    pub fn rules(&self) -> &RuleSet {
        &self.rules
    }

    pub fn stats(&self) -> &RulesStats {
        &self.stats
    }

    /// The run's QVTo state (to inspect the models, e.g. [`QvtoRun::with_scratch_store`]).
    pub fn qvto(&mut self) -> &mut QvtoRun {
        &mut self.run
    }

    /// The PRM model in the run's QVTo state, once a reconfiguration created it.
    pub fn prm_model(&self) -> Option<&ModelRef> {
        self.prm.model.as_ref().map(|(m, _)| m)
    }

    /// Writes the run's PRM values into the PRM model (creating the model and the measurements
    /// the run has added since the last time).
    pub fn sync_prm<C: Compat, F: Flavour>(
        &mut self,
        sim: &mut Simulation<'_, C, F>,
    ) -> Result<(), ReconfError> {
        let ms: Vec<(Option<ObjId>, ObjId, f64)> = sim
            .prm_measurements()
            .iter()
            .map(|m| (m.def.mp, m.def.spec, m.value))
            .collect();
        self.mirror_prm(sim, &ms)
    }

    /// Creates the PRM model (once) and the runtime measurements of `ms` it lacks, then writes
    /// the values of `ms`.
    pub(crate) fn mirror_prm<C: Compat, F: Flavour>(
        &mut self,
        sim: &mut Simulation<'_, C, F>,
        ms: &[(Option<ObjId>, ObjId, f64)],
    ) -> Result<(), ReconfError> {
        if self.prm.model.is_none() {
            let r = self.run.create_model(sim, PRM_URI);
            let ModelRef::Loaded(i) = r else {
                unreachable!("a created model is a loaded one")
            };
            let ((), _) = self.run.with_store(sim, |st| {
                let (model, _) = prm_classes(st);
                let root = st.create(model).expect("RuntimeMeasurementModel");
                st.add_root(i, root, None).expect("root");
                self.prm.model = Some((r, root));
            })?;
        }
        let (_, root) = self.prm.model.clone().expect("PRM model");
        let known = self.prm.meas.len();
        let mut created = Vec::new();
        let ((), _) = self.run.with_store(sim, |st| {
            let (model_class, meas) = prm_classes(st);
            let (f_meas, f_mp, f_spec, f_value) = {
                let meta = st.meta();
                let f = |c, n| meta.feature_by_name(c, n).expect("PRM feature");
                (
                    f(model_class, "measurements"),
                    f(meas, "measuringPoint"),
                    f(meas, "measurementSpecification"),
                    f(meas, "measuringValue"),
                )
            };
            for &(mp, spec, _) in &ms[known.min(ms.len())..] {
                let m = st.create(meas).expect("RuntimeMeasurement");
                if let Some(mp) = mp {
                    st.set(m, f_mp, Datum::Obj(mp)).expect("measuringPoint");
                }
                st.set(m, f_spec, Datum::Obj(spec))
                    .expect("measurementSpecification");
                st.add(root, f_meas, Datum::Obj(m), None)
                    .expect("measurements");
                created.push(m);
            }
            let handles: Vec<ObjId> = self.prm.meas.iter().chain(&created).copied().collect();
            for (&h, &(_, _, v)) in handles.iter().zip(ms) {
                st.set(h, f_value, Datum::Double(v))
                    .expect("measuringValue");
            }
        })?;
        self.prm.meas.extend(created);
        Ok(())
    }

    /// Runs the waiting run of `sim`'s reconfiguration process, if any: mirrors the PRM, runs
    /// the rules, reports the result. Returns whether a run was handled. An error ends the run
    /// ([`Simulation::fail_run`]) and is returned.
    pub fn handle<C: Compat, F: Flavour>(
        &mut self,
        sim: &mut Simulation<'_, C, F>,
    ) -> Result<bool, SimError> {
        let Some(due) = sim.reconf_due() else {
            return Ok(false);
        };
        let t0 = Instant::now();
        let r = self.sync_prm(sim).map_err(|e| (RulesOutcome::default(), e));
        self.stats.prm_micros += t0.elapsed().as_micros() as u64;
        let r = r.and_then(|()| self.run.run_rules(sim, &self.rules, &self.inv));
        self.stats.micros += t0.elapsed().as_micros() as u64;
        self.stats.runs += 1;
        let (out, error) = match r {
            Ok(out) => (out, None),
            Err((out, e)) => (out, Some(e)),
        };
        let succeeded = error.is_none() && out.rules.iter().all(applied);
        if succeeded {
            self.stats.succeeded += 1;
        }
        if out.changed() {
            self.stats.changed += 1;
        }
        if let Some(recs) = self.records.as_mut() {
            recs.push(ReconfRecord {
                at_ns: due.at_ns,
                post_stop: due.post_stop,
                succeeded,
                rules: out
                    .rules
                    .iter()
                    .map(|o| RuleRecord {
                        name: o.transformation.clone(),
                        succeeded: applied(o),
                        changed: o.change.is_some(),
                        log: o.log.clone(),
                    })
                    .collect(),
            });
        }
        if let Some(e) = error {
            let rule = self
                .rules
                .rules
                .get(out.rules.len())
                .map_or("?", |r| r.name());
            return Err(match e {
                ReconfError::RunFailed { error, .. } => error,
                e => {
                    let e = e.to_string();
                    match e.strip_prefix(rule).and_then(|r| r.strip_prefix(": ")) {
                        Some(r) => sim.fail_run(format!("reconfiguration rule {rule}: {r}")),
                        None => sim.fail_run(format!("reconfiguration rule {rule}: {e}")),
                    }
                }
            });
        }
        sim.reconf_done(succeeded)?;
        Ok(true)
    }
}

/// SimuLizar 5.2.2's "applied" (`AbstractQVTOExecutor.handleExecutionResult`: severity OK or
/// INFO) with the QVTo it ships (3.10.7): a run that completes reports OK, also when a
/// non-fatal `assert` (`warning`, `error`) failed; a run that terminates (`assert fatal`, an
/// exception) reports ERROR. qvto-rs follows QVTo 3.11, which raises the severity of a
/// completed run to that of its failed non-fatal assertions, so "applied" is "completed"
/// (diagnostic code 0).
fn applied(o: &crate::host::Outcome) -> bool {
    o.code == 0 && o.severity != 8
}

/// The PRM metamodel's classes: `RuntimeMeasurementModel`, `RuntimeMeasurement`.
fn prm_classes(st: &impl ModelStore) -> (qvto_model::ClassId, qvto_model::ClassId) {
    let meta = st.meta();
    let p = meta
        .package_by_ns_uri(PRM_NS)
        .expect("the core registers the PRM metamodel");
    (
        meta.class_by_name(p, "RuntimeMeasurementModel")
            .expect("RuntimeMeasurementModel"),
        meta.class_by_name(p, "RuntimeMeasurement")
            .expect("RuntimeMeasurement"),
    )
}

/// The run reached `p` (before the next event; see [`Pause`]).
fn reached<C: Compat, F: Flavour>(p: Pause, s: &RunState<C, F>) -> bool {
    match p {
        Pause::Never => false,
        Pause::After(t) => s.next_event_ns().is_some_and(|n| n > t),
        Pause::Before(t) => s.next_event_ns().is_some_and(|n| n >= t),
        Pause::Events(n) => s.events() >= n,
    }
}

/// A simulation run with SimuLizar's reconfiguration rules: the simulation and its
/// reconfiguration host. Pauses, forks and snapshots carry the PRM values (run state) and the
/// QVTo state (host) together.
pub struct RulesRun<'m, C: Compat = DefaultCompat, F: Flavour = SimuLizar> {
    sim: Simulation<'m, C, F>,
    host: RulesHost,
}

impl<'m, C: Compat, F: Flavour> RulesRun<'m, C, F> {
    /// Attaches `host` to `sim` ([`Simulation::attach_reconfiguration`]).
    pub fn new(mut sim: Simulation<'m, C, F>, host: RulesHost) -> Self {
        sim.attach_reconfiguration();
        RulesRun { sim, host }
    }

    /// A run from its parts (e.g. a resumed state and the host saved with it).
    pub fn from_parts(sim: Simulation<'m, C, F>, host: RulesHost) -> Self {
        Self::new(sim, host)
    }

    pub fn into_parts(self) -> (Simulation<'m, C, F>, RulesHost) {
        (self.sim, self.host)
    }

    /// The simulation and the host, both mutable.
    pub(crate) fn parts_mut(&mut self) -> (&mut Simulation<'m, C, F>, &mut RulesHost) {
        (&mut self.sim, &mut self.host)
    }

    pub fn sim(&self) -> &Simulation<'m, C, F> {
        &self.sim
    }

    pub fn sim_mut(&mut self) -> &mut Simulation<'m, C, F> {
        &mut self.sim
    }

    pub fn host(&self) -> &RulesHost {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut RulesHost {
        &mut self.host
    }

    /// Runs until `p` (between events, as [`Simulation::run_until`]) or the end of the run,
    /// running the reconfiguration rules whenever the reconfiguration process runs.
    pub fn run_until(&mut self, p: Pause) -> Result<Status, SimError> {
        loop {
            let st = self
                .sim
                .run_while(|s| !s.reconf_pending() && !reached(p, s))?;
            if self.host.handle(&mut self.sim)? {
                continue;
            }
            return Ok(st);
        }
    }

    /// Runs to the end and returns the result.
    pub fn finish(mut self) -> Result<(RunResult, RulesHost), SimError> {
        self.run_until(Pause::Never)?;
        let r = self.sim.finish()?;
        Ok((r, self.host))
    }

    /// A fork: the simulation's ([`Simulation::fork`]) with a copy of the host (the QVTo state
    /// is shared copy on write).
    pub fn fork(&mut self, out: Outputs) -> Result<RulesRun<'m, C, F>, SimError> {
        Ok(RulesRun {
            sim: self.sim.fork(out)?,
            host: self.host.clone(),
        })
    }
}

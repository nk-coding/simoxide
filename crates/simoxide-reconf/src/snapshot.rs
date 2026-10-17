//! Snapshots of runs with a QVTo state (`DESIGN-SPD.md` N1): a run's QVTo state
//! ([`QvtoRun::save_state`], section `qvto` for hosts of their own, e.g. an SPD run with a
//! `QvtoAdjuster`) and runs with SimuLizar's reconfiguration rules ([`RulesRun::snapshot_to`],
//! section `rules`).
//!
//! The QVTo state is not written as it is: it lives in the run's model graph, whose objects of
//! SimOxide's metamodels the engine's snapshot writes (XMI, placeholders for the others); the
//! core's metamodel objects are attached again, the models of other metamodels are rebuilt by
//! their owners (the PRM mirror of the rules host, with the same objects in the same order), and
//! the id generator continues where it was (`qvt::SessionState`). The transformations, rule sets and
//! the QVTo core are not part of a snapshot either: the caller passes them again, like the
//! compiled model.

use crate::host::{QvtoRun, ReconfError};
use crate::process::{ReconfRecord, RulesHost, RulesRun, RulesStats};
use crate::qvt::SessionState;
use serde::{Deserialize, Serialize};
use simoxide_sim::snapshot::{
    Encoding, Section, Snapshot, SnapshotError, SnapshotInfo, SnapshotOptions,
};
use simoxide_sim::{Compat, CompiledModel, Flavour, Outputs, Simulation};

/// Name of the section of a [`QvtoRun`] (hosts that keep one of their own).
pub const QVTO: &str = "qvto";
/// Name of the section of a [`RulesRun`]'s host.
pub const RULES: &str = "rules";

/// A run's QVTo state as a snapshot keeps it ([`QvtoRun::save_state`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QvtoState {
    /// The per-run state existed (a transformation ran or a model was created).
    started: bool,
    invocations: u64,
    session: Option<SessionState>,
}

impl QvtoRun {
    /// The run's QVTo state for a snapshot (see the module documentation).
    pub fn save_state(&self) -> QvtoState {
        match &self.state {
            None => QvtoState {
                started: false,
                invocations: 0,
                session: None,
            },
            Some(st) => QvtoState {
                started: true,
                invocations: st.invocations,
                session: self.session_state(),
            },
        }
    }

    /// Continues this (new, unused) QVTo state from `s`, on the restored simulation `sim`: the
    /// per-run state is created again if it existed; owners of models of other metamodels
    /// rebuild them before (see [`RulesRun::restore_snapshot`]), then the id generator is set.
    pub fn restore_state<H: simoxide_sim::ChangeHost + ?Sized>(
        &mut self,
        sim: &H,
        s: &QvtoState,
    ) -> Result<(), ReconfError> {
        if !s.started {
            return Ok(());
        }
        self.restore_session(sim, s.invocations, s.session.as_ref())
            .map_err(ReconfError::Load)
    }

    /// [`QvtoRun::save_state`] as the section `qvto`.
    pub fn snapshot_section(&self, enc: Encoding) -> Result<Section, SnapshotError> {
        Section::encode(QVTO, &self.save_state(), enc)
    }

    /// [`QvtoRun::restore_state`] from the section `qvto` of `s` (nothing if it has none).
    pub fn restore_section<H: simoxide_sim::ChangeHost + ?Sized>(
        &mut self,
        sim: &H,
        s: &Snapshot,
    ) -> Result<(), SnapshotError> {
        if let Some(sec) = s.section(QVTO) {
            let st: QvtoState = sec.decode(s.header.encoding)?;
            self.restore_state(sim, &st)
                .map_err(|e| SnapshotError::Restore(e.to_string()))?;
        }
        Ok(())
    }
}

/// The section `rules`: the host's state.
#[derive(Serialize, Deserialize)]
struct RulesImage {
    /// The rules' names (checked on restore).
    rules: Vec<String>,
    qvto: QvtoState,
    /// The PRM mirror (`None`: not created yet): the model's resource and root, and its
    /// runtime measurements (objects of the run's model graph).
    prm: Option<(u32, u32, Vec<u32>)>,
    stats: RulesStats,
    records: Option<Vec<ReconfRecord>>,
}

impl<'m, C: Compat, F: Flavour> RulesRun<'m, C, F> {
    /// The run's snapshot (the simulation's sections and `rules`), between events.
    pub fn snapshot_value(&mut self, enc: Encoding) -> Result<Snapshot, SnapshotError> {
        let (sim, host) = self.parts_mut();
        let img = RulesImage {
            rules: host
                .rules
                .rules
                .iter()
                .map(|r| r.name().to_string())
                .collect(),
            qvto: host.run.save_state(),
            prm: host.prm.model.as_ref().map(|(m, root)| {
                let r = match m {
                    crate::host::ModelRef::Loaded(r) | crate::host::ModelRef::Graph(r) => r.0,
                    crate::host::ModelRef::Uri(_) => u32::MAX,
                };
                (r, root.0, host.prm.meas.iter().map(|o| o.0).collect())
            }),
            stats: host.stats.clone(),
            records: host.records.clone(),
        };
        let sec = Section::encode(RULES, &img, enc)?;
        sim.snapshot_value(enc, vec![sec])
    }

    /// Writes the run's snapshot to `w` (postcard).
    pub fn snapshot_to(&mut self, w: impl std::io::Write) -> Result<SnapshotInfo, SnapshotError> {
        self.snapshot_with(w, &SnapshotOptions::default())
    }

    /// [`RulesRun::snapshot_to`] with options.
    pub fn snapshot_with(
        &mut self,
        mut w: impl std::io::Write,
        opts: &SnapshotOptions,
    ) -> Result<SnapshotInfo, SnapshotError> {
        let s = self.snapshot_value(opts.encoding)?;
        let mut bytes = Vec::new();
        let header = s.write(&mut bytes)?;
        w.write_all(&bytes)?;
        w.flush()?;
        Ok(SnapshotInfo {
            header,
            bytes: bytes.len() as u64,
        })
    }

    /// Restores a run written by [`RulesRun::snapshot_to`] on `base` (the compiled model it was
    /// created on) with `host`, a new host with the same rules and configuration
    /// ([`RulesHost::new`]; `keep_records` as wanted), which continues the saved host's state.
    pub fn restore_from(
        r: impl std::io::Read,
        base: &'m CompiledModel,
        host: RulesHost,
        out: Outputs,
    ) -> Result<Self, SnapshotError> {
        let s = Snapshot::read(r)?;
        Self::restore_snapshot(&s, base, host, out)
    }

    /// [`RulesRun::restore_from`] of a snapshot already read.
    pub fn restore_snapshot(
        s: &Snapshot,
        base: &'m CompiledModel,
        host: RulesHost,
        out: Outputs,
    ) -> Result<Self, SnapshotError> {
        Self::restore_inner(s, base, host, out, true)
    }

    /// [`RulesRun::restore_snapshot`] with other rules than the snapshot's (`DESIGN-SPD.md` N2:
    /// a rule set replaced at the resume): the host's rules run from now on, the saved QVTo
    /// state, PRM mirror and statistics continue. Equal to the uninterrupted run that calls
    /// [`RulesHost::replace_rules`] at the snapshot's pause.
    pub fn restore_snapshot_replacing(
        s: &Snapshot,
        base: &'m CompiledModel,
        host: RulesHost,
        out: Outputs,
    ) -> Result<Self, SnapshotError> {
        Self::restore_inner(s, base, host, out, false)
    }

    /// The rules' names in the section `rules` of `s`.
    pub fn snapshot_rules(s: &Snapshot) -> Result<Vec<String>, SnapshotError> {
        let sec = s.section(RULES).ok_or_else(|| {
            SnapshotError::Mismatch("the snapshot has no reconfiguration host".into())
        })?;
        let img: RulesImage = sec.decode(s.header.encoding)?;
        Ok(img.rules)
    }

    fn restore_inner(
        s: &Snapshot,
        base: &'m CompiledModel,
        mut host: RulesHost,
        out: Outputs,
        same_rules: bool,
    ) -> Result<Self, SnapshotError> {
        let sec = s.section(RULES).ok_or_else(|| {
            SnapshotError::Mismatch("the snapshot has no reconfiguration host".into())
        })?;
        let img: RulesImage = sec.decode(s.header.encoding)?;
        let names: Vec<String> = host
            .rules
            .rules
            .iter()
            .map(|r| r.name().to_string())
            .collect();
        if same_rules && names != img.rules {
            return Err(SnapshotError::Mismatch(format!(
                "the snapshot's rules are {:?}, the host's {:?}",
                img.rules, names
            )));
        }
        let sim = Simulation::<C, F>::restore_snapshot(s, base, out)?;
        let err = |e: ReconfError| SnapshotError::Restore(e.to_string());
        // the QVTo state with every id (the PRM mirror's objects included)
        host.run.restore_state(&sim, &img.qvto).map_err(err)?;
        if let Some((r, root, meas)) = img.prm {
            host.prm.model = Some((
                crate::host::ModelRef::Loaded(qvto_model::ResourceId(r)),
                qvto_emf::ObjId(root),
            ));
            host.prm.meas = meas.into_iter().map(qvto_emf::ObjId).collect();
        }
        host.stats = img.stats;
        if img.records.is_some() {
            host.records = img.records;
        }
        Ok(RulesRun::from_parts(sim, host))
    }
}

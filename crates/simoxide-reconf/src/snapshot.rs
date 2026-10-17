//! Snapshots of runs with a QVTo state (`DESIGN-SPD.md` N1): a run's QVTo state
//! ([`QvtoRun::save_state`], section `qvto` for hosts of their own, e.g. an SPD run with a
//! `QvtoAdjuster`) and runs with SimuLizar's reconfiguration rules ([`RulesRun::snapshot_to`],
//! section `rules`).
//!
//! The QVTo state is not written as it is: the run's model graph is the simulation's (written as
//! XMI by the engine's snapshot), the session's models of other metamodels are rebuilt by their
//! owners (the PRM mirror of the rules host, with the same objects in the same order), and the
//! id generator continues where it was (`qvt::SessionState`). The transformations, rule sets and
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
            Some(st) => {
                let session = crate::host::lock_session(st).save();
                QvtoState {
                    started: true,
                    invocations: st.invocations,
                    session: Some(session),
                }
            }
        }
    }

    /// Continues this (new, unused) QVTo state from `s`, on the restored simulation `sim`: the
    /// per-run state is created again if it existed; owners of models of other metamodels
    /// rebuild them before (see [`RulesRun::restore_snapshot`]), then the id generator is set.
    pub fn restore_state<C: Compat, F: Flavour>(
        &mut self,
        sim: &Simulation<'_, C, F>,
        s: &QvtoState,
    ) {
        if !s.started {
            return;
        }
        let st = self.prepare(sim);
        st.invocations = s.invocations;
        if let Some(ss) = &s.session {
            st.session().restore(ss);
        }
    }

    /// [`QvtoRun::save_state`] as the section `qvto`.
    pub fn snapshot_section(&self, enc: Encoding) -> Result<Section, SnapshotError> {
        Section::encode(QVTO, &self.save_state(), enc)
    }

    /// [`QvtoRun::restore_state`] from the section `qvto` of `s` (nothing if it has none).
    pub fn restore_section<C: Compat, F: Flavour>(
        &mut self,
        sim: &Simulation<'_, C, F>,
        s: &Snapshot,
    ) -> Result<(), SnapshotError> {
        if let Some(sec) = s.section(QVTO) {
            let st: QvtoState = sec.decode(s.header.encoding)?;
            self.restore_state(sim, &st);
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
    /// Runtime measurements of the PRM mirror (`None`: not created yet).
    prm: Option<u32>,
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
            prm: host.prm.model.as_ref().map(|_| host.prm.meas.len() as u32),
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
        mut host: RulesHost,
        out: Outputs,
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
        if names != img.rules {
            return Err(SnapshotError::Mismatch(format!(
                "the snapshot's rules are {:?}, the host's {:?}",
                img.rules, names
            )));
        }
        let mut sim = Simulation::<C, F>::restore_snapshot(s, base, out)?;
        let err = |e: ReconfError| SnapshotError::Restore(e.to_string());
        if let Some(n) = img.prm {
            host.rebuild_prm(&mut sim, n as usize).map_err(err)?;
        }
        host.run.restore_state(&sim, &img.qvto);
        host.stats = img.stats;
        if img.records.is_some() {
            host.records = img.records;
        }
        Ok(RulesRun::from_parts(sim, host))
    }
}

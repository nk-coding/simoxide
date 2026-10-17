//! Snapshots of SPD runs (`DESIGN-SPD.md` N1): the simulation's snapshot (`simoxide_sim::snapshot`)
//! plus the section `spd`: the interpreter's state ([`SpdState`]), the queued adjustment requests
//! and the counters. The interpreter itself ([`Spd`]) is immutable and built from the model; the
//! caller passes it again, as the compiled model.

use crate::interp::{AdjustmentRequest, PolicyState, Spd, SpdState};
use crate::model::{GroupCfg, Membership};
use crate::quirks::Quirks;
use crate::sim::{SpdRun, series_points};
use serde::{Deserialize, Serialize};
use simoxide_model::raw::ObjId;
use simoxide_sim::snapshot::{
    Encoding, Section, Snapshot, SnapshotError, SnapshotInfo, SnapshotOptions,
};
use simoxide_sim::{Compat, CompiledModel, Outputs, Simulation};

/// Name of the SPD section.
pub const SPD: &str = "spd";

/// [`GroupCfg`] with object ids as numbers.
#[derive(Serialize, Deserialize)]
enum GroupCfgImage {
    ElasticInfrastructure {
        unit: Option<u32>,
        elements: Vec<u32>,
    },
    ServiceGroup {
        unit: Option<u32>,
        elements: Vec<u32>,
    },
    CompetingConsumers {
        unit: Option<u32>,
        broker: Option<u32>,
        elements: Vec<u32>,
    },
}

fn ids(v: &[ObjId]) -> Vec<u32> {
    v.iter().map(|o| o.0).collect()
}

fn objs(v: Vec<u32>) -> Vec<ObjId> {
    v.into_iter().map(ObjId).collect()
}

#[derive(Serialize, Deserialize)]
struct MembershipImage {
    cfgs: Vec<GroupCfgImage>,
    allocation: Vec<(u32, u32)>,
    environment: Vec<u32>,
    provided: Vec<(u32, Vec<u32>)>,
}

impl From<&Membership> for MembershipImage {
    fn from(m: &Membership) -> Self {
        MembershipImage {
            cfgs: m
                .cfgs
                .iter()
                .map(|c| match c {
                    GroupCfg::ElasticInfrastructure { unit, elements } => {
                        GroupCfgImage::ElasticInfrastructure {
                            unit: unit.map(|o| o.0),
                            elements: ids(elements),
                        }
                    }
                    GroupCfg::ServiceGroup { unit, elements } => GroupCfgImage::ServiceGroup {
                        unit: unit.map(|o| o.0),
                        elements: ids(elements),
                    },
                    GroupCfg::CompetingConsumers {
                        unit,
                        broker,
                        elements,
                    } => GroupCfgImage::CompetingConsumers {
                        unit: unit.map(|o| o.0),
                        broker: broker.map(|o| o.0),
                        elements: ids(elements),
                    },
                })
                .collect(),
            allocation: m.allocation.iter().map(|(a, c)| (a.0, c.0)).collect(),
            environment: ids(&m.environment),
            provided: m.provided.iter().map(|(a, v)| (a.0, ids(v))).collect(),
        }
    }
}

impl From<MembershipImage> for Membership {
    fn from(m: MembershipImage) -> Self {
        Membership {
            cfgs: m
                .cfgs
                .into_iter()
                .map(|c| match c {
                    GroupCfgImage::ElasticInfrastructure { unit, elements } => {
                        GroupCfg::ElasticInfrastructure {
                            unit: unit.map(ObjId),
                            elements: objs(elements),
                        }
                    }
                    GroupCfgImage::ServiceGroup { unit, elements } => GroupCfg::ServiceGroup {
                        unit: unit.map(ObjId),
                        elements: objs(elements),
                    },
                    GroupCfgImage::CompetingConsumers {
                        unit,
                        broker,
                        elements,
                    } => GroupCfg::CompetingConsumers {
                        unit: unit.map(ObjId),
                        broker: broker.map(ObjId),
                        elements: objs(elements),
                    },
                })
                .collect(),
            allocation: m
                .allocation
                .into_iter()
                .map(|(a, c)| (ObjId(a), ObjId(c)))
                .collect(),
            environment: objs(m.environment),
            provided: m
                .provided
                .into_iter()
                .map(|(a, v)| (ObjId(a), objs(v)))
                .collect(),
        }
    }
}

/// The section `spd`.
#[derive(Serialize, Deserialize)]
struct SpdImage {
    /// The interpreter's compat flags (checked on restore).
    quirks: Quirks,
    /// Shape of the interpreter: policy contexts and time events (checked on restore).
    contexts: u32,
    timers: u32,
    policies: Vec<PolicyState>,
    groups: Vec<crate::interp::GroupState>,
    membership: MembershipImage,
    next_request: u64,
    evaluations: u64,
    requests: u64,
    queued: Vec<Option<AdjustmentRequest>>,
    adjusted_ok: u64,
}

/// The compat flags of the SPD interpreter of snapshot `s` (`None`: no SPD state), to build
/// the interpreter for [`SpdRun::restore_from`] the same way.
pub fn quirks_of(s: &Snapshot) -> Result<Option<Quirks>, SnapshotError> {
    match s.section(SPD) {
        None => Ok(None),
        Some(sec) => Ok(Some(sec.decode::<SpdImage>(s.header.encoding)?.quirks)),
    }
}

impl<'m, 's, C: Compat> SpdRun<'m, 's, C> {
    /// The run's snapshot (the simulation's sections and `spd`), between events: after
    /// [`SpdRun::run_until`] returned, when the SPD work of the last event is done.
    pub fn snapshot_value(&mut self, enc: Encoding) -> Result<Snapshot, SnapshotError> {
        let st = &self.state;
        let img = SpdImage {
            quirks: self.spd.quirks(),
            contexts: self.spd.contexts().len() as u32,
            timers: self.spd.timers().len() as u32,
            policies: st.policies.clone(),
            groups: st.groups.clone(),
            membership: (&st.membership).into(),
            next_request: st.next_request,
            evaluations: st.evaluations,
            requests: st.requests,
            queued: self.requests.clone(),
            adjusted_ok: self.adjusted_ok,
        };
        let sec = Section::encode(SPD, &img, enc)?;
        self.sim.snapshot_value(enc, vec![sec])
    }

    /// Writes the run's snapshot to `w` (postcard).
    pub fn snapshot_to(&mut self, w: impl std::io::Write) -> Result<SnapshotInfo, SnapshotError> {
        self.snapshot_with(w, &SnapshotOptions::default())
    }

    /// [`SpdRun::snapshot_to`] with options.
    pub fn snapshot_with(
        &mut self,
        w: impl std::io::Write,
        opts: &SnapshotOptions,
    ) -> Result<SnapshotInfo, SnapshotError> {
        let s = self.snapshot_value(opts.encoding)?;
        let bytes = {
            let mut v = Vec::new();
            s.write(&mut v)?;
            v
        };
        let mut w = w;
        w.write_all(&bytes)?;
        w.flush()?;
        let header = Snapshot::read(&bytes[..])?.header;
        Ok(SnapshotInfo {
            header,
            bytes: bytes.len() as u64,
        })
    }

    /// Restores an SPD run written by [`SpdRun::snapshot_to`]: the simulation on `base` (see
    /// [`Simulation::restore_from`]) with the interpreter `spd` (built from the same model).
    pub fn restore_from(
        r: impl std::io::Read,
        base: &'m CompiledModel,
        spd: &'s Spd,
        out: Outputs,
    ) -> Result<Self, SnapshotError> {
        let s = Snapshot::read(r)?;
        Self::restore_snapshot(&s, base, spd, out)
    }

    /// [`SpdRun::restore_from`] of a snapshot already read.
    pub fn restore_snapshot(
        s: &Snapshot,
        base: &'m CompiledModel,
        spd: &'s Spd,
        out: Outputs,
    ) -> Result<Self, SnapshotError> {
        let sec = s
            .section(SPD)
            .ok_or_else(|| SnapshotError::Mismatch("the snapshot has no SPD state".into()))?;
        let img: SpdImage = sec.decode(s.header.encoding)?;
        if img.quirks != spd.quirks() {
            return Err(SnapshotError::Mismatch(format!(
                "the SPD state is of an interpreter with the compat flags {:?}, this one has {:?}",
                img.quirks,
                spd.quirks()
            )));
        }
        if img.contexts as usize != spd.contexts().len()
            || img.timers as usize != spd.timers().len()
            || img.policies.len() != spd.contexts().len()
        {
            return Err(SnapshotError::Mismatch(
                "the SPD state belongs to another scaling policy definition".into(),
            ));
        }
        let sim = Simulation::<C>::restore_snapshot(s, base, out)?;
        Ok(SpdRun {
            series: series_points(sim.compiled()),
            sim,
            spd,
            state: SpdState {
                policies: img.policies,
                groups: img.groups,
                membership: img.membership.into(),
                next_request: img.next_request,
                evaluations: img.evaluations,
                requests: img.requests,
            },
            requests: img.queued,
            adjusted_ok: img.adjusted_ok,
        })
    }
}

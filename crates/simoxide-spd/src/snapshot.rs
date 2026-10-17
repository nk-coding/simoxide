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
use simoxide_sim::{Compat, CompiledModel, Flavour, Outputs, Simulation};

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

/// The section `spd` of an interpreter's state (`queued`: the requests by handle; SpdRun's
/// queue or the Slingshot-exact layer's request table).
pub(crate) fn spd_section(
    spd: &Spd,
    st: &SpdState,
    queued: &[Option<AdjustmentRequest>],
    adjusted_ok: u64,
    enc: Encoding,
) -> Result<Section, SnapshotError> {
    let img = SpdImage {
        quirks: spd.quirks(),
        contexts: spd.contexts().len() as u32,
        timers: spd.timers().len() as u32,
        policies: st.policies.clone(),
        groups: st.groups.clone(),
        membership: (&st.membership).into(),
        next_request: st.next_request,
        evaluations: st.evaluations,
        requests: st.requests,
        queued: queued.to_vec(),
        adjusted_ok,
    };
    Section::encode(SPD, &img, enc)
}

/// The interpreter's state of the section `spd` of `s`, checked against `spd`: the state,
/// the requests by handle, the successful adjustments.
pub(crate) fn read_spd_section(
    s: &Snapshot,
    spd: &Spd,
) -> Result<(SpdState, Vec<Option<AdjustmentRequest>>, u64), SnapshotError> {
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
    Ok((
        SpdState {
            policies: img.policies,
            groups: img.groups,
            membership: img.membership.into(),
            next_request: img.next_request,
            evaluations: img.evaluations,
            requests: img.requests,
        },
        img.queued,
        img.adjusted_ok,
    ))
}

impl<'m, 's, C: Compat, F: Flavour> SpdRun<'m, 's, C, F> {
    /// The run's snapshot (the simulation's sections and `spd`), between events: after
    /// [`SpdRun::run_until`] returned, when the SPD work of the last event is done.
    pub fn snapshot_value(&mut self, enc: Encoding) -> Result<Snapshot, SnapshotError> {
        let sec = spd_section(self.spd, &self.state, &self.requests, self.adjusted_ok, enc)?;
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
        let (state, requests, adjusted_ok) = read_spd_section(s, spd)?;
        let sim = Simulation::<C, F>::restore_snapshot(s, base, out)?;
        Ok(SpdRun {
            series: series_points(sim.compiled()),
            sim,
            spd,
            state,
            requests,
            adjusted_ok,
        })
    }
}

// ------------------------------------------------------------------------------------------
// the SPD layer of a Slingshot-exact run (`crate::ssx::SsxSpd`)

/// Name of the section with the Slingshot-exact layer's own state (next to `spd`).
pub const SPD_SSX: &str = "spd-ssx";

/// A resolved measuring point with object ids as numbers.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum PointImage {
    ActiveResource(u32),
    Operation(u32),
    AssemblyPassiveResource(u32, u32),
    Container(u32),
    Environment,
    Other,
}

impl From<crate::model::PointKind> for PointImage {
    fn from(k: crate::model::PointKind) -> Self {
        use crate::model::PointKind as K;
        match k {
            K::ActiveResource { container } => PointImage::ActiveResource(container.0),
            K::Operation { signature } => PointImage::Operation(signature.0),
            K::AssemblyPassiveResource { assembly, passive } => {
                PointImage::AssemblyPassiveResource(assembly.0, passive.0)
            }
            K::Container { container } => PointImage::Container(container.0),
            K::Environment => PointImage::Environment,
            K::Other => PointImage::Other,
        }
    }
}

impl From<PointImage> for crate::model::PointKind {
    fn from(k: PointImage) -> Self {
        use crate::model::PointKind as K;
        match k {
            PointImage::ActiveResource(c) => K::ActiveResource {
                container: ObjId(c),
            },
            PointImage::Operation(s) => K::Operation {
                signature: ObjId(s),
            },
            PointImage::AssemblyPassiveResource(a, p) => K::AssemblyPassiveResource {
                assembly: ObjId(a),
                passive: ObjId(p),
            },
            PointImage::Container(c) => K::Container {
                container: ObjId(c),
            },
            PointImage::Environment => K::Environment,
            PointImage::Other => K::Other,
        }
    }
}

/// The section `spd-ssx`: the series' resolved measuring points and metrics (resolved on the
/// model of their first measurement, so kept rather than resolved again).
#[derive(Serialize, Deserialize)]
pub(crate) struct SsxSpdImage {
    /// Per engine series: unresolved, not a policy's, or (point, class, metric).
    pub points: Vec<Option<Option<(PointImage, String, crate::interp::Metric)>>>,
}

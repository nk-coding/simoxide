//! Snapshots of runs on disk (`DESIGN-SPD.md` 5.7, phase N1; format: `docs/formats/snapshot.md`).
//!
//! A snapshot is the run state ([`crate::RunState`]) of a simulation paused between two events,
//! written to a byte stream, plus — when the run changed its model — the run's own model (the
//! model graph as XMI documents with the object table of [`simoxide_model::persist`], the build
//! order, and the run's compiled tables). Restoring it gives a simulation that continues exactly
//! like the uninterrupted run: the same trace lines, tape records, measurements and counts.
//!
//! The file:
//!
//! ```text
//! SIMOXIDE-SNAPSHOT 1\n           magic line with the format version
//! {"format":1,"simoxide":"0.1.0",...}\n   header (one JSON line, see `Header`)
//! <section bytes> ...              the sections named in the header, in order
//! ```
//!
//! Each section is one serialised value, in the encoding the header names: `postcard` (compact,
//! the default) or `json` (for reading and debugging; non-finite numbers do not survive it). The
//! engine writes the section `engine` (the run state) and, after model changes, `model`; layers
//! above the engine add their own (`spd`, `rules`, `qvto`: the SPD interpreter's state, the
//! reconfiguration host's). Every section has a checksum in the header.
//!
//! An unchanged model is not stored: the header carries its fingerprint
//! ([`model_fingerprint`]), and restoring needs the same compiled model, as
//! [`crate::Simulation::resume`] does. Snapshots are deterministic: the same state gives the same
//! bytes (maps are written in key order).

use crate::ir::CompiledModel;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io::{Read, Write};

/// The format version this build writes and reads.
pub const FORMAT: u32 = 1;

/// The first line of a snapshot file (without the version).
const MAGIC: &str = "SIMOXIDE-SNAPSHOT";

/// How sections are encoded.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Encoding {
    /// `postcard` (compact binary; the default).
    #[default]
    Postcard,
    /// JSON text (for reading; non-finite numbers are written as `null` and cannot be read back).
    Json,
}

/// The header of a snapshot (the second line of the file, JSON).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Header {
    /// [`FORMAT`].
    pub format: u32,
    /// The SimOxide version that wrote it (`CARGO_PKG_VERSION`). The sections are SimOxide's
    /// internal run state, so a snapshot is read by the same version only.
    pub simoxide: String,
    /// Simulator mode: `exact` or `fast`.
    pub mode: String,
    /// The literal engine (tests only).
    #[serde(default)]
    pub literal: bool,
    /// `simulizar` or `slingshot`, with the Slingshot compat flags (`Q2,Q26`, empty: none).
    pub flavour: String,
    pub quirks: String,
    /// Fingerprint of the compiled model the run was created on ([`model_fingerprint`]).
    pub model: String,
    /// The run's own model after model changes: its generation and fingerprint.
    pub run_model: Option<RunModelInfo>,
    /// Where the run is: simulation time (ns), events processed, phase.
    pub time_ns: i64,
    pub events: u64,
    pub phase: String,
    /// Run with a trace (a traced run continues with a trace, which may be discarded).
    pub traced: bool,
    pub encoding: Encoding,
    pub sections: Vec<SectionInfo>,
}

/// The run's own model in a [`Header`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunModelInfo {
    pub generation: u32,
    pub fingerprint: String,
}

/// A section in a [`Header`]: its name, length in bytes and checksum (FNV-1a 64, hex).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionInfo {
    pub name: String,
    pub bytes: u64,
    pub checksum: String,
}

/// A section of a snapshot: a name and the encoded value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub data: Vec<u8>,
}

impl Section {
    /// `value` encoded with `enc`.
    pub fn encode<T: Serialize>(
        name: &str,
        value: &T,
        enc: Encoding,
    ) -> Result<Section, SnapshotError> {
        let data = match enc {
            Encoding::Postcard => postcard::to_stdvec(value)
                .map_err(|e| SnapshotError::Encode(format!("{name}: {e}")))?,
            Encoding::Json => serde_json::to_vec(value)
                .map_err(|e| SnapshotError::Encode(format!("{name}: {e}")))?,
        };
        Ok(Section {
            name: name.to_string(),
            data,
        })
    }

    /// The value, decoded with `enc` (the whole section must be consumed).
    pub fn decode<T: for<'de> Deserialize<'de>>(&self, enc: Encoding) -> Result<T, SnapshotError> {
        let bad = |e: String| SnapshotError::Corrupt(format!("section {}: {e}", self.name));
        match enc {
            Encoding::Postcard => {
                let (v, rest) =
                    postcard::take_from_bytes(&self.data).map_err(|e| bad(e.to_string()))?;
                if !rest.is_empty() {
                    return Err(bad(format!("{} bytes left over", rest.len())));
                }
                Ok(v)
            }
            Encoding::Json => serde_json::from_slice(&self.data).map_err(|e| bad(e.to_string())),
        }
    }
}

/// A snapshot read from a file: header and sections.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub header: Header,
    pub sections: Vec<Section>,
}

impl Snapshot {
    /// The section `name`.
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    /// Writes the snapshot (the header's section list is filled in from `sections`) and returns
    /// the header written.
    pub fn write(&self, mut w: impl Write) -> Result<Header, SnapshotError> {
        let mut h = self.header.clone();
        h.sections = self
            .sections
            .iter()
            .map(|s| SectionInfo {
                name: s.name.clone(),
                bytes: s.data.len() as u64,
                checksum: format!("{:016x}", fnv64(&s.data)),
            })
            .collect();
        let line = serde_json::to_string(&h).map_err(|e| SnapshotError::Encode(e.to_string()))?;
        w.write_all(format!("{MAGIC} {FORMAT}\n{line}\n").as_bytes())?;
        for s in &self.sections {
            w.write_all(&s.data)?;
        }
        w.flush()?;
        Ok(h)
    }

    /// Reads a snapshot: checks the magic line, the format version and the checksums.
    pub fn read(mut r: impl Read) -> Result<Snapshot, SnapshotError> {
        let mut bytes = Vec::new();
        r.read_to_end(&mut bytes)?;
        let line = |b: &[u8]| -> Option<usize> { b.iter().position(|&c| c == b'\n') };
        let n1 = line(&bytes).ok_or(SnapshotError::NotASnapshot)?;
        let first = std::str::from_utf8(&bytes[..n1]).map_err(|_| SnapshotError::NotASnapshot)?;
        let version = first
            .strip_prefix(MAGIC)
            .and_then(|v| v.trim().parse::<u32>().ok())
            .ok_or(SnapshotError::NotASnapshot)?;
        if version != FORMAT {
            return Err(SnapshotError::Version {
                found: version,
                supported: FORMAT,
            });
        }
        let rest = &bytes[n1 + 1..];
        let n2 = line(rest).ok_or_else(|| SnapshotError::Corrupt("no header".into()))?;
        let header: Header = serde_json::from_slice(&rest[..n2])
            .map_err(|e| SnapshotError::Corrupt(format!("header: {e}")))?;
        if header.format != FORMAT {
            return Err(SnapshotError::Version {
                found: header.format,
                supported: FORMAT,
            });
        }
        let mut body = &rest[n2 + 1..];
        let mut sections = Vec::with_capacity(header.sections.len());
        for s in &header.sections {
            let n = usize::try_from(s.bytes).unwrap_or(usize::MAX);
            if body.len() < n {
                return Err(SnapshotError::Corrupt(format!(
                    "section {} is truncated",
                    s.name
                )));
            }
            let (data, more) = body.split_at(n);
            body = more;
            if format!("{:016x}", fnv64(data)) != s.checksum {
                return Err(SnapshotError::Corrupt(format!(
                    "section {}: checksum mismatch",
                    s.name
                )));
            }
            sections.push(Section {
                name: s.name.clone(),
                data: data.to_vec(),
            });
        }
        if !body.is_empty() {
            return Err(SnapshotError::Corrupt(format!(
                "{} bytes after the last section",
                body.len()
            )));
        }
        Ok(Snapshot { header, sections })
    }
}

/// Why a snapshot could not be written or restored.
#[derive(Debug)]
pub enum SnapshotError {
    Io(std::io::Error),
    /// Not a SimOxide snapshot (no magic line).
    NotASnapshot,
    /// Written in another format version.
    Version {
        found: u32,
        supported: u32,
    },
    /// Written by another SimOxide version (the run state is internal: same version only).
    SimOxideVersion {
        found: String,
        this: String,
    },
    /// The compiled model given to restore it is not the one the run was created on.
    Model {
        expected: String,
        found: String,
    },
    /// The snapshot is for another simulator mode, flavour or engine than the one restoring it.
    Mismatch(String),
    /// Damaged or inconsistent content.
    Corrupt(String),
    /// The state cannot be written (e.g. a run in the middle of an event).
    Encode(String),
    /// The restored state does not fit (e.g. the snapshot's model does not compile).
    Restore(String),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnapshotError::Io(e) => write!(f, "snapshot I/O: {e}"),
            SnapshotError::NotASnapshot => f.write_str("not a SimOxide snapshot"),
            SnapshotError::Version { found, supported } => write!(
                f,
                "snapshot format version {found} is not supported (this build reads version {supported})"
            ),
            SnapshotError::SimOxideVersion { found, this } => write!(
                f,
                "the snapshot was written by SimOxide {found}; this is SimOxide {this} (a snapshot holds the internal run state and is read by the same version)"
            ),
            SnapshotError::Model { expected, found } => write!(
                f,
                "the snapshot belongs to another model (fingerprint {expected}; the given model has {found})"
            ),
            SnapshotError::Mismatch(m) => write!(f, "snapshot mismatch: {m}"),
            SnapshotError::Corrupt(m) => write!(f, "damaged snapshot: {m}"),
            SnapshotError::Encode(m) => write!(f, "cannot write the snapshot: {m}"),
            SnapshotError::Restore(m) => write!(f, "cannot restore the snapshot: {m}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<std::io::Error> for SnapshotError {
    fn from(e: std::io::Error) -> Self {
        SnapshotError::Io(e)
    }
}

/// FNV-1a, 64 bit.
pub(crate) fn fnv64(b: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &x in b {
        h = (h ^ x as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The fingerprint of a compiled model: its model graph's content
/// ([`simoxide_model::persist::fingerprint`], independent of the directory it was loaded from)
/// and the flavour it is compiled for. Equal for the same model files compiled the same way.
pub fn model_fingerprint(cm: &CompiledModel) -> String {
    let [a, b] = simoxide_model::persist::fingerprint(&cm.model.graph);
    let fl = cm.flavour;
    let mut s = format!("{a:016x}{b:016x}-{}", fl.kind.name());
    if !fl.quirks.is_empty() {
        s.push_str(&format!("-{}", fl.quirks));
    }
    s
}

/// Options of [`crate::Simulation::snapshot_with`].
#[derive(Clone, Debug, Default)]
pub struct SnapshotOptions {
    pub encoding: Encoding,
}

/// What a written snapshot holds (returned by [`crate::Simulation::snapshot_with`]).
#[derive(Clone, Debug)]
pub struct SnapshotInfo {
    pub header: Header,
    /// Total size in bytes.
    pub bytes: u64,
}

/// The JSON form of a snapshot's header and sections, for reading (`simoxide snapshot-info`).
pub fn describe(s: &Snapshot) -> String {
    let mut out = serde_json::to_string_pretty(&s.header).unwrap_or_default();
    out.push('\n');
    out
}

/// The random source of a run in a snapshot.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum RngImage {
    /// The exact mode's stream: MT19937 (words and position) or the position in the replayed
    /// tape (the tape is part of the run configuration), with the tape bookkeeping.
    Exact {
        mt: Option<(Vec<u32>, u32)>,
        replay: Option<u64>,
        count: u64,
        origin: String,
        check_origins: bool,
        problem: Option<String>,
        depth: u32,
        eval_start: u64,
        use_samples: bool,
    },
    /// The fast mode's xoshiro256++ state and sample count.
    Fast { state: [u64; 4], count: u64 },
}

/// `serde(with)` helpers for the run state's types (`&'static str` fields, maps in key order,
/// graph object ids).
pub mod serde_util {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use simoxide_model::raw::ObjId;
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};

    /// `s` as a `&'static str`: the run state holds metric names, action kinds and error
    /// messages as static strings. A string that is no constant of the engine is kept once per
    /// process.
    pub fn intern(s: &str) -> &'static str {
        static SET: OnceLock<Mutex<HashSet<&'static str>>> = OnceLock::new();
        let set = SET.get_or_init(|| Mutex::new(HashSet::new()));
        let mut g = set.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(x) = g.get(s) {
            return x;
        }
        let x: &'static str = Box::leak(s.to_string().into_boxed_str());
        g.insert(x);
        x
    }

    pub mod interned {
        use super::*;
        pub fn serialize<S: Serializer>(v: &&'static str, s: S) -> Result<S::Ok, S::Error> {
            v.serialize(s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<&'static str, D::Error> {
            String::deserialize(d).map(|x| intern(&x))
        }
    }

    pub mod opt_interned {
        use super::*;
        pub fn serialize<S: Serializer>(v: &Option<&'static str>, s: S) -> Result<S::Ok, S::Error> {
            v.serialize(s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(
            d: D,
        ) -> Result<Option<&'static str>, D::Error> {
            Option::<String>::deserialize(d).map(|x| x.map(|x| intern(&x)))
        }
    }

    pub mod interned_vec {
        use super::*;
        pub fn serialize<S: Serializer>(v: &[&'static str], s: S) -> Result<S::Ok, S::Error> {
            v.serialize(s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<&'static str>, D::Error> {
            Vec::<String>::deserialize(d).map(|v| v.iter().map(|x| intern(x)).collect())
        }
    }

    /// A map as a list of pairs in key order (deterministic bytes; the maps are only looked up).
    pub mod sorted {
        use super::*;
        pub fn serialize<'a, M, K, V, S>(m: &'a M, s: S) -> Result<S::Ok, S::Error>
        where
            &'a M: IntoIterator<Item = (&'a K, &'a V)>,
            K: Ord + Serialize + 'a,
            V: Serialize + 'a,
            S: Serializer,
        {
            let mut v: Vec<(&K, &V)> = m.into_iter().collect();
            v.sort_by(|a, b| a.0.cmp(b.0));
            v.serialize(s)
        }
        pub fn deserialize<'de, M, K, V, D>(d: D) -> Result<M, D::Error>
        where
            M: FromIterator<(K, V)>,
            K: Deserialize<'de>,
            V: Deserialize<'de>,
            D: Deserializer<'de>,
        {
            Vec::<(K, V)>::deserialize(d).map(|v| v.into_iter().collect())
        }
    }

    /// The series of a measuring point by metric (`CompiledModel::series_by_mp` and the run's
    /// lazily created series), in key order.
    pub mod series_map {
        use super::*;
        use std::collections::HashMap;
        type M = HashMap<Box<str>, Vec<(&'static str, u32)>>;
        pub fn serialize<S: Serializer>(m: &M, s: S) -> Result<S::Ok, S::Error> {
            let mut v: Vec<(&str, &Vec<(&'static str, u32)>)> =
                m.iter().map(|(k, v)| (&**k, v)).collect();
            v.sort_by(|a, b| a.0.cmp(b.0));
            v.serialize(s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<M, D::Error> {
            let v = Vec::<(String, Vec<(String, u32)>)>::deserialize(d)?;
            Ok(v.into_iter()
                .map(|(k, l)| {
                    (
                        k.into_boxed_str(),
                        l.into_iter().map(|(m, s)| (intern(&m), s)).collect(),
                    )
                })
                .collect())
        }
    }

    pub mod objid {
        use super::*;
        pub fn serialize<S: Serializer>(v: &ObjId, s: S) -> Result<S::Ok, S::Error> {
            v.0.serialize(s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<ObjId, D::Error> {
            u32::deserialize(d).map(ObjId)
        }
    }

    pub mod opt_objid {
        use super::*;
        pub fn serialize<S: Serializer>(v: &Option<ObjId>, s: S) -> Result<S::Ok, S::Error> {
            v.map(|o| o.0).serialize(s)
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<ObjId>, D::Error> {
            Option::<u32>::deserialize(d).map(|o| o.map(ObjId))
        }
    }

    pub mod objids {
        use super::*;
        pub fn serialize<S: Serializer>(v: &[ObjId], s: S) -> Result<S::Ok, S::Error> {
            s.collect_seq(v.iter().map(|o| o.0))
        }
        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ObjId>, D::Error> {
            Vec::<u32>::deserialize(d).map(|v| v.into_iter().map(ObjId).collect())
        }
    }
}

// ------------------------------------------------------------------------------------------
// the compiled model's tables that are not plain data

impl Serialize for crate::ir::Keys {
    /// The key names in id order (the hashes and the lookup map follow from them).
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.names.serialize(s)
    }
}

impl<'de> Deserialize<'de> for crate::ir::Keys {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let names = Vec::<String>::deserialize(d)?;
        let mut k = crate::ir::Keys::default();
        for n in &names {
            k.intern(n);
        }
        if k.names.len() != names.len() {
            return Err(serde::de::Error::custom("duplicate frame key"));
        }
        Ok(k)
    }
}

impl Serialize for crate::ir::Prog {
    /// The specification only: it is compiled again when the model is restored
    /// ([`restore_compiled`]).
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.spec.serialize(s)
    }
}

impl<'de> Deserialize<'de> for crate::ir::Prog {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let spec = String::deserialize(d)?;
        Ok(crate::ir::Prog {
            spec: spec.into_boxed_str(),
            prog: Err("not compiled (snapshot)".into()),
            konst: None,
            konst_f64: None,
        })
    }
}

/// Completes a compiled model read from a snapshot: its typed model, the StoEx programs
/// (compiled again from their specifications against the restored keys, as the compiler does),
/// and the finalisation orders (functions of the resource registry).
pub(crate) fn restore_compiled(
    cm: &mut CompiledModel,
    model: simoxide_model::Model,
) -> Result<(), String> {
    cm.model = model;
    let keys = &mut cm.keys;
    let n_keys = keys.names.len();
    for p in &mut cm.progs {
        let fresh = crate::ir::compile_prog(&p.spec, keys);
        *p = fresh;
    }
    if keys.names.len() != n_keys {
        return Err("a program of the snapshot's model refers to an unknown frame key".into());
    }
    #[cfg(feature = "exact")]
    {
        cm.finalise_order =
            crate::ir::finalise_order::<simoxide_jcompat_exact::order::JdkHashOrder>(&cm.registry);
    }
    cm.finalise_order_fast =
        crate::ir::finalise_order::<simoxide_jcompat::order::InsertionOrder>(&cm.registry);
    Ok(())
}

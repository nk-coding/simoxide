//! Shared helpers: the model list and goldens of the EMF oracle (reference/oracles/pcm-model).
#![allow(dead_code)]

use simoxide_model::{LoadOptions, Loader};
use std::path::{Path, PathBuf};

pub const BUNDLED: &[&str] = &[
    "pathmap://PCM_MODELS/Palladio.resourcetype",
    "pathmap://PCM_MODELS/PrimitiveTypes.repository",
    "pathmap://PCM_MODELS/FailureTypes.repository",
    "pathmap://PCM_MODELS/Glassfish.repository",
    "pathmap://PCM_MODELS/default_event_middleware.repository",
    "pathmap://METRIC_SPEC_MODELS/commonMetrics.metricspec",
    "pathmap://METRIC_SPEC_MODELS/models/commonMetrics.metricspec",
];

pub fn oracle_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/oracles/pcm-model")
}

pub struct Entry {
    pub name: String,
    /// `-` for the bundled default models.
    pub dir: String,
    pub golden: Option<String>,
}

impl Entry {
    pub fn is_bundled(&self) -> bool {
        self.dir == "-"
    }
    pub fn emf_errors(&self) -> Option<u64> {
        let first = self.golden.as_ref()?.lines().next()?;
        let v: serde_json::Value = serde_json::from_str(first).ok()?;
        v["emf_errors"].as_u64()
    }
    /// Golden dump without the header line.
    pub fn golden_body(&self) -> Option<&str> {
        let g = self.golden.as_deref()?;
        Some(g.split_once('\n').map(|(_, b)| b).unwrap_or(""))
    }
    pub fn loader(&self, opts: LoadOptions) -> Loader {
        let mut l = Loader::new(opts);
        if self.is_bundled() {
            for u in BUNDLED {
                l.load_uri(u);
            }
        } else {
            l.load_dir(&self.dir).expect("read model dir");
        }
        l
    }
}

/// Where an entry's directory is in this checkout: `models.txt` records absolute paths of the
/// checkout that generated it, so the in-tree directories (`corpus_*`, `case_*`) are taken from
/// this checkout (a worktree tests its own files).
fn local_dir(name: &str, dir: &str) -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for (prefix, marker, base) in [
        ("corpus_", "/corpus/", root.join("../../corpus")),
        ("case_", "/tests/xmi-cases/", root.join("tests/xmi-cases")),
        // repositories that are git submodules here (third_party/README.md), same commits
        (
            "Palladio-Example-Models_",
            "/Palladio-Example-Models/",
            root.join("../../third_party/palladio-example-models"),
        ),
        (
            "Palladio-Analyzer-Slingshot-E2E-Tests_",
            "/Palladio-Analyzer-Slingshot-E2E-Tests/",
            root.join("../../third_party/slingshot-e2e-tests"),
        ),
        (
            "Palladio-Documentation-Slingshot_",
            "/Palladio-Documentation-Slingshot/",
            root.join("../../third_party/slingshot-docs"),
        ),
    ] {
        if name.starts_with(prefix)
            && let Some((_, rest)) = dir.rsplit_once(marker)
        {
            return base.join(rest).to_string_lossy().into_owned();
        }
    }
    dir.to_string()
}

/// Entries of `models.txt` whose directory exists (the Palladio repos may be absent) and whose
/// third-party models are checked out.
pub fn entries() -> Vec<Entry> {
    let list = std::fs::read_to_string(oracle_dir().join("models.txt"))
        .expect("models.txt of the simoxide-model oracle");
    let mut out = Vec::new();
    for line in list
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        let (name, dir) = line.split_once('\t').expect("name<TAB>dir");
        let dir = &if dir == "-" {
            dir.to_string()
        } else {
            local_dir(name, dir)
        };
        if dir != "-" && !Path::new(dir).is_dir() {
            continue;
        }
        // links into third-party submodules that are not checked out
        if dir != "-" && !simoxide_thirdparty::skip_unless_available(Path::new(dir)) {
            continue;
        }
        let golden =
            std::fs::read_to_string(oracle_dir().join("golden").join(format!("{name}.jsonl"))).ok();
        out.push(Entry {
            name: name.to_string(),
            dir: dir.to_string(),
            golden,
        });
    }
    out
}

//! Shared helpers of the corpus tests: corpus roots, the Eclipse oracle fixture, and problem
//! lines in the oracle's format.

#![allow(dead_code)]

pub mod mutate;

use qvto_syntax::{LineIndex, Parsed, Severity, span};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn manifest() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn dir_from_env(var: &str, default: PathBuf) -> Option<PathBuf> {
    let p = std::env::var(var).map(PathBuf::from).unwrap_or(default);
    p.is_dir().then_some(p)
}

/// Corpus roots by fixture prefix (`eclipse`, `simulizar`, `spd`); missing ones are absent.
pub fn roots() -> BTreeMap<&'static str, PathBuf> {
    let repo = qvto_paths::root();
    // the palladio-research work directory, as reference/tools/palladio-research.sh resolves it:
    // $PALLADIO_RESEARCH, else reference/.work/palladio-research of the repository
    let pr = std::env::var("PALLADIO_RESEARCH")
        .ok()
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            qvto_paths::workspace_root(&repo).join("reference/.work/palladio-research")
        });
    let repos = pr.join("repos");
    let mut m = BTreeMap::new();
    if let Some(p) = dir_from_env("QVTO_ECLIPSE_DIR", repo.join("reference/eclipse-qvto")) {
        m.insert("eclipse", p);
    }
    if let Some(p) = dir_from_env(
        "QVTO_SIMULIZAR_DIR",
        repos.join("Palladio-Analyzer-SimuLizar"),
    ) {
        m.insert("simulizar", p);
    }
    if let Some(p) = dir_from_env(
        "QVTO_SPD_DIR",
        repos.join("Palladio-Addons-SPD-Metamodel/bundles/org.palladiosimulator.spd.semantic.transformations"),
    ) {
        m.insert("spd", p);
    }
    m
}

/// The oracle fixture: file key (`eclipse:tests/…`) to its problem lines (without the key),
/// empty for files Eclipse parses without problems. Keys in file order.
pub fn oracle() -> BTreeMap<String, Vec<String>> {
    let text = std::fs::read_to_string(manifest().join("tests/fixtures/eclipse-parse-oracle.tsv"))
        .unwrap();
    let mut m: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in text.lines() {
        let (key, rest) = line.split_once('\t').unwrap();
        let e = m.entry(key.to_string()).or_default();
        if rest != "OK" {
            e.push(rest.to_string());
        }
    }
    m
}

/// Resolves a fixture key to a path, if its corpus is present.
pub fn resolve(roots: &BTreeMap<&'static str, PathBuf>, key: &str) -> Option<PathBuf> {
    let (prefix, rel) = key.split_once(':')?;
    let p = roots.get(prefix)?.join(rel);
    p.is_file().then_some(p)
}

pub fn read(p: &Path) -> String {
    String::from_utf8_lossy(&std::fs::read(p).unwrap()).into_owned()
}

/// Problem lines in the oracle's format: `E|W  utf16-offset  utf16-length  line  message`.
pub fn problem_lines(src: &str, parsed: &Parsed) -> Vec<String> {
    let lines = LineIndex::new(src);
    let mut out: Vec<String> = parsed
        .diagnostics
        .iter()
        .map(|d| {
            let off = span::byte_to_utf16(src, d.span.start as usize);
            let len = span::byte_to_utf16(src, d.span.end as usize) - off;
            let sev = if d.severity == Severity::Error {
                "E"
            } else {
                "W"
            };
            format!(
                "{sev}\t{off}\t{len}\t{}\t{}",
                lines.line(d.span.start),
                d.message
            )
        })
        .collect();
    out.sort();
    out
}

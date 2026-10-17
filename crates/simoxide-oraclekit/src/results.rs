//! Result directories and comparisons.
//!
//! The harnesses lay out their results the same way: one subdirectory per case, named by the
//! case, holding that case's output files (`refsim batch --out DIR`: `DIR/<model>/trace.jsonl`,
//! `tape.jsonl`, `measurements.csv`; `qvto-run --batch --out DIR`: `DIR/<case id>/result.txt`,
//! `out.jsonl`, ...; the Slingshot corpus: `build/corpus-ss/<case>/...`). Committed expectations
//! (`corpus/<model>/expected/`, `corpus-ss/<case>/expected/`) store files larger than
//! [`GZIP_THRESHOLD`] as `<file>.gz`; readers here look for both.
//!
//! [`ResultSet`] is the in-memory form (case → file name → text); [`differing_files`] and
//! [`differences`] compare two of them, as the `--check` modes and the stability reruns do.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The files of one case: file name → content.
pub type Files = BTreeMap<String, String>;

/// The results of a batch: case name → its files.
pub type ResultSet = BTreeMap<String, Files>;

/// A case's result that holds its files (so that a harness's own result type can be compared).
pub trait CaseFiles {
    fn files(&self) -> &Files;
}

impl CaseFiles for Files {
    fn files(&self) -> &Files {
        self
    }
}

/// Expected files larger than this are stored gzip'ed (`<file>.gz`), as `reference/regen-expected.sh`
/// and the fuzzer's saved cases do.
pub const GZIP_THRESHOLD: usize = 256 * 1024;

fn gz_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(".gz");
    PathBuf::from(p)
}

/// Reads `path`, or `path.gz` transparently. `Ok(None)` if neither exists.
pub fn read_maybe_gz(path: &Path) -> std::io::Result<Option<String>> {
    if path.is_file() {
        return std::fs::read_to_string(path).map(Some);
    }
    let gz = gz_path(path);
    if gz.is_file() {
        let mut s = String::new();
        flate2::read::GzDecoder::new(std::fs::File::open(gz)?).read_to_string(&mut s)?;
        return Ok(Some(s));
    }
    Ok(None)
}

/// Writes `text` to `path`, or gzip'ed to `path.gz` when it is larger than [`GZIP_THRESHOLD`].
pub fn write_maybe_gz(path: &Path, text: &str) -> std::io::Result<()> {
    if text.len() > GZIP_THRESHOLD {
        let f = std::fs::File::create(gz_path(path))?;
        let mut enc = flate2::write::GzEncoder::new(f, flate2::Compression::default());
        enc.write_all(text.as_bytes())?;
        enc.finish()?;
        Ok(())
    } else {
        std::fs::write(path, text)
    }
}

/// The files of `names` that exist in `dir` (plain text; no `.gz`).
pub fn read_case(dir: &Path, names: &[&str]) -> Files {
    names
        .iter()
        .filter_map(|n| {
            std::fs::read_to_string(dir.join(n))
                .ok()
                .map(|t| (n.to_string(), t))
        })
        .collect()
}

/// Reads a result directory: every subdirectory that holds `marker` is a case (named after the
/// subdirectory) with the files of `names` it has.
pub fn read_dir(dir: &Path, marker: &str, names: &[&str]) -> std::io::Result<ResultSet> {
    let mut out = ResultSet::new();
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if !p.join(marker).is_file() {
            continue;
        }
        let id = p.file_name().unwrap().to_string_lossy().to_string();
        out.insert(id, read_case(&p, names));
    }
    Ok(out)
}

/// For each case of both sets, the files of `names` whose contents differ (a file missing on one
/// side differs from one that is present); cases without a difference are left out.
pub fn differing_files<'n, A: CaseFiles, B: CaseFiles>(
    a: &BTreeMap<String, A>,
    b: &BTreeMap<String, B>,
    names: &[&'n str],
) -> BTreeMap<String, Vec<&'n str>> {
    let mut out = BTreeMap::new();
    for (id, fa) in a {
        let Some(fb) = b.get(id) else { continue };
        let (fa, fb) = (fa.files(), fb.files());
        let d: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| fa.get(*n) != fb.get(*n))
            .collect();
        if !d.is_empty() {
            out.insert(id.clone(), d);
        }
    }
    out
}

/// One difference between two result sets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Difference {
    /// The case is only in the first set.
    OnlyFirst(String),
    /// The case is only in the second set.
    OnlySecond(String),
    /// The case's file differs (or is missing on one side).
    File { case: String, file: String },
}

impl std::fmt::Display for Difference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Difference::OnlyFirst(c) => write!(f, "{c}: only in the first"),
            Difference::OnlySecond(c) => write!(f, "{c}: only in the second"),
            Difference::File { case, file } => write!(f, "{case}: {file} differs"),
        }
    }
}

/// Every difference between two result sets over all their files, in case order.
pub fn differences<A: CaseFiles, B: CaseFiles>(
    a: &BTreeMap<String, A>,
    b: &BTreeMap<String, B>,
) -> Vec<Difference> {
    let mut out = Vec::new();
    let mut ids: Vec<&String> = a.keys().chain(b.keys()).collect();
    ids.sort();
    ids.dedup();
    for id in ids {
        match (a.get(id), b.get(id)) {
            (Some(_), None) => out.push(Difference::OnlyFirst(id.clone())),
            (None, Some(_)) => out.push(Difference::OnlySecond(id.clone())),
            (Some(fa), Some(fb)) => {
                let (fa, fb) = (fa.files(), fb.files());
                let mut names: Vec<&String> = fa.keys().chain(fb.keys()).collect();
                names.sort();
                names.dedup();
                for n in names {
                    if fa.get(n) != fb.get(n) {
                        out.push(Difference::File {
                            case: id.clone(),
                            file: n.clone(),
                        });
                    }
                }
            }
            (None, None) => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(cases: &[(&str, &[(&str, &str)])]) -> ResultSet {
        cases
            .iter()
            .map(|(c, fs)| {
                (
                    c.to_string(),
                    fs.iter()
                        .map(|(n, t)| (n.to_string(), t.to_string()))
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn compare_sets() {
        let a = set(&[
            ("a", &[("x", "1"), ("y", "2")]),
            ("b", &[("x", "1")]),
            ("c", &[]),
        ]);
        let b = set(&[("a", &[("x", "1"), ("y", "3")]), ("b", &[]), ("d", &[])]);
        let d = differing_files(&a, &b, &["x", "y"]);
        assert_eq!(d.len(), 2);
        assert_eq!(d["a"], ["y"]);
        assert_eq!(d["b"], ["x"]);
        let all: Vec<String> = differences(&a, &b).iter().map(|d| d.to_string()).collect();
        assert_eq!(
            all,
            [
                "a: y differs",
                "b: x differs",
                "c: only in the first",
                "d: only in the second"
            ]
        );
        assert!(differences(&a, &a).is_empty());
    }

    #[test]
    fn dirs_and_gz() {
        let d = std::env::temp_dir().join(format!("oraclekit-results-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("one")).unwrap();
        std::fs::create_dir_all(d.join("two")).unwrap();
        std::fs::create_dir_all(d.join("not-a-case")).unwrap();
        std::fs::write(d.join("one/result.txt"), "r1").unwrap();
        std::fs::write(d.join("one/log.txt"), "l1").unwrap();
        std::fs::write(d.join("two/result.txt"), "r2").unwrap();
        std::fs::write(d.join("not-a-case/log.txt"), "x").unwrap();
        let r = read_dir(&d, "result.txt", &["result.txt", "log.txt"]).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r["one"]["log.txt"], "l1");
        assert!(!r["two"].contains_key("log.txt"));

        let big = "x".repeat(GZIP_THRESHOLD + 1);
        write_maybe_gz(&d.join("big.txt"), &big).unwrap();
        write_maybe_gz(&d.join("small.txt"), "s").unwrap();
        assert!(d.join("big.txt.gz").is_file() && !d.join("big.txt").exists());
        assert_eq!(read_maybe_gz(&d.join("big.txt")).unwrap().unwrap(), big);
        assert_eq!(read_maybe_gz(&d.join("small.txt")).unwrap().unwrap(), "s");
        assert_eq!(read_maybe_gz(&d.join("none.txt")).unwrap(), None);
        let _ = std::fs::remove_dir_all(&d);
    }
}

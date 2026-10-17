//! Results of the Java reference runner: per-case result directories (`qvto-run --batch`) and
//! the committed fixture bundles made from them.
//!
//! A result directory has one subdirectory per case with `result.txt` (tab-separated key/value
//! lines), `out.jsonl` (canonical dump of the out/inout extents), `log.txt`, `stack.txt`,
//! `diag.txt` and `idtape.tsv` (identity-hash tape). A fixture bundle is one text file holding
//! the small files of every case:
//!
//! ```text
//! qvto-testkit java fixture 1
//! @case <id>
//! @file <name> <byte length>
//! <content>
//! ```
//!
//! Each content is followed by a newline that is not part of it.

use std::collections::BTreeMap;
use std::path::Path;

/// The files of a case that go into a fixture bundle (the tape stays in the cache).
pub const FIXTURE_FILES: [&str; 5] = [
    "result.txt",
    "out.jsonl",
    "log.txt",
    "stack.txt",
    "diag.txt",
];

/// A compile problem as the Java runner reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub error: bool,
    pub unit: String,
    pub line: i32,
    pub offset: i32,
    pub length: i32,
    pub message: String,
}

/// The Java runner's result for one case.
#[derive(Clone, Debug, Default)]
pub struct JavaResult {
    pub id: String,
    pub files: BTreeMap<String, String>,
}

impl JavaResult {
    pub fn file(&self, name: &str) -> Option<&str> {
        self.files.get(name).map(String::as_str)
    }

    /// A field of `result.txt`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.file("result.txt")?.lines().find_map(|l| {
            let (k, v) = l.split_once('\t')?;
            (k == key).then_some(v)
        })
    }

    pub fn verdict(&self) -> &str {
        self.get("verdict").unwrap_or("missing")
    }

    /// Severity of the execution diagnostic (`OK`, `WARNING`, `ERROR`, `CANCEL`).
    pub fn severity(&self) -> Option<&str> {
        self.get("severity")
    }

    pub fn code(&self) -> Option<i32> {
        self.get("code").and_then(|c| c.parse().ok())
    }

    pub fn exception(&self) -> Option<&str> {
        self.get("exception").filter(|e| *e != "-")
    }

    pub fn dump(&self) -> Option<&str> {
        self.file("out.jsonl")
    }

    pub fn log(&self) -> &str {
        self.file("log.txt").unwrap_or("")
    }

    pub fn stack(&self) -> Option<&str> {
        self.file("stack.txt")
    }

    /// Outputs that differed between two Java runs (`dump`, `log`): identity-hash order or time.
    pub fn unstable(&self) -> Vec<&str> {
        self.get("unstable")
            .map(|s| s.split(',').filter(|x| !x.is_empty()).collect())
            .unwrap_or_default()
    }

    pub fn problems(&self) -> Vec<Problem> {
        let Some(d) = self.file("diag.txt") else {
            return Vec::new();
        };
        d.lines()
            .filter_map(|l| {
                let f: Vec<&str> = l.splitn(7, '\t').collect();
                if f.len() < 7 || f[0] != "problem" {
                    return None;
                }
                Some(Problem {
                    error: f[1] == "error",
                    unit: f[2].into(),
                    line: f[3].parse().ok()?,
                    offset: f[4].parse().ok()?,
                    length: f[5].parse().ok()?,
                    message: unescape(&f[6..].join("\t")),
                })
            })
            .collect()
    }
}

/// Undoes the runner's `oneLine` (`\\`, `\r`, `\n`, `\t` escapes of a problem message).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(x) => out.push(x),
            None => out.push('\\'),
        }
    }
    out
}

/// Reads the result directory of a batch run (`<dir>/<id>/...`).
pub fn read_result_dir(dir: &Path) -> std::io::Result<BTreeMap<String, JavaResult>> {
    let mut out = BTreeMap::new();
    for e in std::fs::read_dir(dir)? {
        let p = e?.path();
        if !p.join("result.txt").is_file() {
            continue;
        }
        let id = p.file_name().unwrap().to_string_lossy().to_string();
        let mut r = JavaResult {
            id: id.clone(),
            files: BTreeMap::new(),
        };
        for f in FIXTURE_FILES.iter().chain(["idtape.tsv"].iter()) {
            if let Ok(t) = std::fs::read_to_string(p.join(f)) {
                r.files.insert(f.to_string(), t);
            }
        }
        out.insert(id, r);
    }
    Ok(out)
}

const HEADER: &str = "qvto-testkit java fixture 1\n";

/// Writes a fixture bundle: the [`FIXTURE_FILES`] of every case, empty files and timings left
/// out.
pub fn write_bundle(results: &BTreeMap<String, JavaResult>) -> String {
    let mut s = String::from(HEADER);
    for (id, r) in results {
        s.push_str("@case ");
        s.push_str(id);
        s.push('\n');
        for f in FIXTURE_FILES {
            let text = r.files.get(f).map(|t| {
                if f == "result.txt" {
                    // timings vary from run to run; they stay in the cache
                    t.lines()
                        .filter(|l| !l.starts_with("run-ms\t"))
                        .map(|l| format!("{l}\n"))
                        .collect()
                } else {
                    t.clone()
                }
            });
            if let Some(t) = text.as_ref().filter(|t| !t.is_empty()) {
                s.push_str(&format!("@file {f} {}\n", t.len()));
                s.push_str(t);
                s.push('\n');
            }
        }
    }
    s
}

/// Reads a fixture bundle.
pub fn read_bundle(text: &str) -> Result<BTreeMap<String, JavaResult>, String> {
    let rest = text
        .strip_prefix(HEADER)
        .ok_or("not a java fixture bundle")?;
    let mut out = BTreeMap::new();
    let mut cur: Option<JavaResult> = None;
    let mut pos = 0;
    let b = rest.as_bytes();
    while pos < b.len() {
        let eol = rest[pos..]
            .find('\n')
            .map(|i| pos + i)
            .ok_or("truncated bundle")?;
        let line = &rest[pos..eol];
        pos = eol + 1;
        if let Some(id) = line.strip_prefix("@case ") {
            if let Some(r) = cur.take() {
                out.insert(r.id.clone(), r);
            }
            cur = Some(JavaResult {
                id: id.into(),
                files: BTreeMap::new(),
            });
        } else if let Some(f) = line.strip_prefix("@file ") {
            let (name, len) = f.rsplit_once(' ').ok_or("bad @file line")?;
            let len: usize = len.parse().map_err(|_| "bad @file length")?;
            let content = rest.get(pos..pos + len).ok_or("truncated content")?;
            pos += len + 1;
            cur.as_mut()
                .ok_or("@file before @case")?
                .files
                .insert(name.into(), content.into());
        } else {
            return Err(format!("unexpected line in bundle: {line}"));
        }
    }
    if let Some(r) = cur.take() {
        out.insert(r.id.clone(), r);
    }
    Ok(out)
}

/// Identity-hash tape of a Java run (`idtape.tsv`, see `reference/runner/src/qvtorun/Tape.java`).
#[derive(Clone, Debug, Default)]
pub struct Tape {
    /// The first two identity hashes of the case's fresh thread.
    pub seed: (i32, i32),
    /// Object key, identity hash, assigned during the run (`Some(true)`), by the tape
    /// (`Some(false)`) or unknown (`None`).
    pub objects: Vec<(String, i32, Option<bool>)>,
    /// The objects of the input resources (`pre` lines), keyed by their path before the run,
    /// likewise. Tapes recorded before these lines existed have none (their input objects are
    /// among [`Tape::objects`], keyed by their path after the run).
    pub pre: Vec<(String, i32, Option<bool>)>,
}

pub fn parse_tape(text: &str) -> Result<Tape, String> {
    let mut t = Tape::default();
    for l in text.lines() {
        let f: Vec<&str> = l.split('\t').collect();
        match f.first() {
            Some(&"seed") if f.len() == 3 => {
                t.seed = (
                    f[1].parse().map_err(|_| "bad seed")?,
                    f[2].parse().map_err(|_| "bad seed")?,
                )
            }
            Some(&k @ ("obj" | "pre")) if f.len() == 4 => {
                let e = (
                    f[1].into(),
                    f[2].parse().map_err(|_| "bad hash")?,
                    match f[3] {
                        "1" => Some(true),
                        "0" => Some(false),
                        _ => None,
                    },
                );
                if k == "pre" {
                    t.pre.push(e)
                } else {
                    t.objects.push(e)
                }
            }
            _ => return Err(format!("bad tape line: {l}")),
        }
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_round_trip() {
        let mut m = BTreeMap::new();
        let mut files = BTreeMap::new();
        files.insert(
            "result.txt".to_string(),
            "id\ta\nverdict\tpass\n".to_string(),
        );
        files.insert("log.txt".to_string(), "@file x 3\nline\n".to_string());
        files.insert(
            "diag.txt".to_string(),
            "problem\terror\ta.qvto\t3\t10\t4\tmsg\\twith tab \\\\ and\\nline\n".to_string(),
        );
        m.insert(
            "a".to_string(),
            JavaResult {
                id: "a".into(),
                files,
            },
        );
        m.insert(
            "b".to_string(),
            JavaResult {
                id: "b".into(),
                files: BTreeMap::new(),
            },
        );
        let b = write_bundle(&m);
        let r = read_bundle(&b).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r["a"].files, m["a"].files);
        assert_eq!(r["a"].verdict(), "pass");
        assert_eq!(r["a"].problems()[0].message, "msg\twith tab \\ and\nline");
    }
}

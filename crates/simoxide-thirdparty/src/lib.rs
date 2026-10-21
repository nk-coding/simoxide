//! Third-party test models referenced through git submodules (`third_party/README.md`).
//!
//! SimOxide does not distribute the models of the Palladio example repositories whose licence is
//! undeclared or pending (LICENSES section 5). They are git submodules under `third_party/`,
//! pinned to the commits the former copies came from, and the test material refers to them in two
//! ways:
//!
//! - **Unmodified files** are relative symbolic links committed in place of the copies (for example
//!   `corpus/x_pem_acquire/AcquireExample.repository`). Without the submodule they dangle.
//! - **Modified files** (SimOxide changed an upstream file, e.g. a demand or an added comment) are
//!   derived: `third_party/derived.txt` names the destination and the upstream source, and
//!   `third_party/derived/<destination>.ed` holds the change as an `ed` script (`diff -e`: line
//!   numbers and the new lines only). [`ensure`] writes the destinations (they are git-ignored).
//!
//! Tests that read such material call [`check`] (or [`skip_unless_available`]) first: without the
//! submodules they skip with a message naming the setup command, and with
//! `SIMOXIDE_REQUIRE_THIRD_PARTY=1` they fail instead (for CI and before regenerating outputs).
//!
//! Outputs that reproduce such a model in full (canonical model dumps, transformation output
//! dumps) are not committed either, only their digests ([`digest`]).

pub mod digest;

pub use digest::Digest;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The command that makes the material available.
pub const SETUP_HINT: &str = "git submodule update --init (see third_party/README.md)";

/// Environment variable: when set to `1`, a missing submodule fails the test instead of skipping it.
pub const REQUIRE_ENV: &str = "SIMOXIDE_REQUIRE_THIRD_PARTY";

/// A submodule under `third_party/`.
#[derive(Debug, Clone, Copy)]
pub struct Submodule {
    /// Path relative to the workspace root.
    pub path: &'static str,
    pub url: &'static str,
    /// The pinned commit (also recorded in the superproject's index).
    pub commit: &'static str,
}

/// The submodules (keep in sync with `.gitmodules` and LICENSES section 5).
pub const SUBMODULES: &[Submodule] = &[
    Submodule {
        path: "third_party/palladio-example-models",
        url: "https://github.com/PalladioSimulator/Palladio-Example-Models.git",
        commit: "4a8dc455216774435fefd42965b848851f7658ee",
    },
    Submodule {
        path: "third_party/slingshot-e2e-tests",
        url: "https://github.com/PalladioSimulator/Palladio-Analyzer-Slingshot-E2E-Tests.git",
        commit: "fd3e916d7cb522059efb29e1565068b99cb2bf64",
    },
    Submodule {
        path: "third_party/slingshot-docs",
        url: "https://github.com/PalladioSimulator/Palladio-Documentation-Slingshot.git",
        commit: "94448159e75f33edfb98e960ad244018b97e37d4",
    },
];

/// The workspace root (the directory holding `third_party/`).
pub fn workspace_root() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    here.parent()
        .and_then(Path::parent)
        .unwrap_or(here)
        .to_path_buf()
}

/// Whether the submodule at `path` (relative to the root) is checked out.
pub fn submodule_available(path: &str) -> bool {
    let dir = workspace_root().join(path);
    // an uninitialised submodule is an empty directory
    fs::read_dir(&dir)
        .map(|mut it| it.any(|e| e.map(|e| e.file_name() != ".git").unwrap_or(false)))
        .unwrap_or(false)
}

/// The submodule a path (relative to the root, or absolute below it) lies in.
pub fn submodule_of(path: &Path) -> Option<&'static Submodule> {
    let root = workspace_root();
    let rel = path.strip_prefix(&root).unwrap_or(path);
    SUBMODULES.iter().find(|s| rel.starts_with(s.path))
}

/// Whether `path` (a file or directory, absolute or relative to the root) is or holds
/// third-party material: it lies in a submodule, is a derived file or a link into a submodule,
/// or (a directory) directly contains one. Decides which outputs are kept as digests only.
pub fn uses_third_party(path: &Path) -> bool {
    let root = workspace_root();
    let abs = normalize(&if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    });
    let is_tp = |p: &Path| -> bool {
        let rel = p.strip_prefix(&root).unwrap_or(p);
        if submodule_of(rel).is_some() {
            return true;
        }
        if let Ok(m) = manifest()
            && m.iter().any(|d| d.dest == rel)
        {
            return true;
        }
        if fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()) {
            let target = fs::read_link(p).unwrap_or_default();
            let resolved = normalize(&p.parent().map(|d| d.join(&target)).unwrap_or(target));
            return SUBMODULES
                .iter()
                .any(|s| resolved.starts_with(root.join(s.path)));
        }
        false
    };
    if is_tp(&abs) {
        return true;
    }
    if !abs.is_dir() {
        return false;
    }
    if let Ok(m) = manifest() {
        let rel = abs.strip_prefix(&root).unwrap_or(&abs);
        if m.iter().any(|d| d.dest.parent() == Some(rel)) {
            return true;
        }
    }
    fs::read_dir(&abs)
        .map(|it| it.flatten().any(|e| is_tp(&e.path())))
        .unwrap_or(false)
}

/// A derived file: an upstream file with a SimOxide change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    /// Destination, relative to the root.
    pub dest: PathBuf,
    /// Upstream source, relative to the root (inside a submodule).
    pub source: PathBuf,
}

impl Derived {
    /// The `ed` script, relative to the root.
    pub fn script(&self) -> PathBuf {
        let mut p = PathBuf::from("third_party/derived")
            .join(&self.dest)
            .into_os_string();
        p.push(".ed");
        PathBuf::from(p)
    }
}

/// The manifest file, relative to the root.
pub const MANIFEST: &str = "third_party/derived.txt";

/// Reads `third_party/derived.txt` (`destination<TAB>source` per line, `#` comments).
pub fn manifest() -> io::Result<Vec<Derived>> {
    let text = fs::read_to_string(workspace_root().join(MANIFEST))?;
    parse_manifest(&text)
}

pub fn parse_manifest(text: &str) -> io::Result<Vec<Derived>> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (d, s) = line.split_once('\t').ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{MANIFEST}:{}: expected `dest<TAB>source`", i + 1),
            )
        })?;
        out.push(Derived {
            dest: d.into(),
            source: s.into(),
        });
    }
    Ok(out)
}

/// Applies an `ed` script as `diff -e` writes it (commands `Na`, `N[,M]c`, `N[,M]d`, bottom-up)
/// to `original`. Both texts consist of `\n`-terminated lines.
pub fn apply_ed(original: &str, script: &str) -> Result<String, String> {
    let mut lines: Vec<&str> = split_lines(original)?;
    let mut cmds = script.split_inclusive('\n');
    while let Some(cmd) = cmds.next() {
        let cmd = cmd
            .strip_suffix('\n')
            .ok_or("ed script: unterminated command")?;
        let (range, op) = cmd.split_at(cmd.len().saturating_sub(1));
        let (a, b) = match range.split_once(',') {
            Some((a, b)) => (num(a)?, num(b)?),
            None => (num(range)?, num(range)?),
        };
        let mut text = Vec::new();
        if op == "a" || op == "c" {
            loop {
                let l = cmds.next().ok_or("ed script: text not terminated by `.`")?;
                if l == ".\n" {
                    break;
                }
                text.push(l);
            }
        }
        if b < a || b > lines.len() || (op != "a" && a == 0) {
            return Err(format!(
                "ed script: line range {a},{b} outside 1..{}",
                lines.len()
            ));
        }
        match op {
            "a" => {
                lines.splice(a..a, text);
            }
            "c" => {
                lines.splice(a - 1..b, text);
            }
            "d" => {
                lines.drain(a - 1..b);
            }
            _ => return Err(format!("ed script: unknown command `{cmd}`")),
        }
    }
    Ok(lines.concat())
}

fn num(s: &str) -> Result<usize, String> {
    s.parse()
        .map_err(|_| format!("ed script: bad line number `{s}`"))
}

fn split_lines(text: &str) -> Result<Vec<&str>, String> {
    if !text.is_empty() && !text.ends_with('\n') {
        return Err("ed script: the original does not end with a newline".into());
    }
    Ok(text.split_inclusive('\n').collect())
}

/// Missing material: the message names the submodule and the setup command.
fn missing(what: &Path, sub: Option<&Submodule>) -> String {
    match sub {
        Some(s) => format!(
            "{} needs the submodule {} (not checked out): {SETUP_HINT}",
            what.display(),
            s.path
        ),
        None => format!("{} is missing: {SETUP_HINT}", what.display()),
    }
}

/// Writes `path` only if its content differs (atomically: temp file + rename).
fn write_if_changed(path: &Path, content: &str) -> io::Result<()> {
    if fs::read_to_string(path)
        .map(|c| c == content)
        .unwrap_or(false)
    {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("thirdparty-{}.tmp", std::process::id()));
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)
}

/// Derives one file from its source and script.
pub fn derive(d: &Derived) -> Result<(), String> {
    let root = workspace_root();
    let src = root.join(&d.source);
    let original = match fs::read_to_string(&src) {
        Ok(t) => t,
        Err(_) => {
            // no stale derived file without its source
            let _ = fs::remove_file(root.join(&d.dest));
            return Err(missing(&d.dest, submodule_of(&d.source)));
        }
    };
    let script_path = root.join(d.script());
    let script =
        fs::read_to_string(&script_path).map_err(|e| format!("{}: {e}", script_path.display()))?;
    let text =
        apply_ed(&original, &script).map_err(|e| format!("{}: {e}", script_path.display()))?;
    write_if_changed(&root.join(&d.dest), &text).map_err(|e| format!("{}: {e}", d.dest.display()))
}

/// Derives every file of the manifest whose submodule is available (once per process). Returns
/// the destinations that could not be derived, with the reason.
pub fn ensure() -> &'static [(PathBuf, String)] {
    static DONE: OnceLock<Vec<(PathBuf, String)>> = OnceLock::new();
    DONE.get_or_init(|| {
        let list = match manifest() {
            Ok(l) => l,
            Err(e) => return vec![(PathBuf::from(MANIFEST), e.to_string())],
        };
        list.iter()
            .filter_map(|d| derive(d).err().map(|e| (d.dest.clone(), e)))
            .collect()
    })
}

/// Checks that the third-party material under `path` (a file or directory, absolute or relative
/// to the root) is available: derives the manifest's files, then looks for derived files that
/// could not be written and for dangling symbolic links. `Err` carries the message for the skip.
pub fn check(path: &Path) -> Result<(), String> {
    let root = workspace_root();
    let abs = normalize(&if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    });
    let rel = abs.strip_prefix(&root).unwrap_or(&abs).to_path_buf();
    for (dest, err) in ensure() {
        if dest.starts_with(&rel) || rel.starts_with(dest) {
            return Err(err.clone());
        }
    }
    dangling(&abs, &rel)
}

fn dangling(abs: &Path, rel: &Path) -> Result<(), String> {
    let meta = match fs::symlink_metadata(abs) {
        Ok(m) => m,
        Err(_) => return Ok(()), // not ours to report
    };
    if meta.file_type().is_symlink() {
        if fs::metadata(abs).is_err() {
            let target = fs::read_link(abs).unwrap_or_default();
            let resolved = abs.parent().map(|p| p.join(&target)).unwrap_or(target);
            let sub = SUBMODULES
                .iter()
                .find(|s| normalize(&resolved).starts_with(workspace_root().join(s.path)));
            return Err(missing(rel, sub));
        }
        return Ok(());
    }
    if meta.is_dir() {
        // the submodules themselves are not scanned (their own links are not ours)
        let root = workspace_root();
        if submodule_of(abs).is_some()
            || abs.file_name().is_some_and(|n| n == ".git")
            || abs == root.join("target")
        {
            return Ok(());
        }
        let mut entries: Vec<_> = fs::read_dir(abs)
            .map_err(|e| e.to_string())?
            .flatten()
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            dangling(&e.path(), &rel.join(e.file_name()))?;
        }
    }
    Ok(())
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// Whether every submodule is checked out and every derived file could be written.
pub fn all_available() -> bool {
    SUBMODULES.iter().all(|s| submodule_available(s.path)) && ensure().is_empty()
}

/// For tests that use third-party material in many places: `true` if all of it is available
/// ([`all_available`]); otherwise prints `skipped (third-party): ...` and returns `false` (or
/// panics with `SIMOXIDE_REQUIRE_THIRD_PARTY=1`).
pub fn skip_unless_all_available() -> bool {
    if all_available() {
        return true;
    }
    let missing: Vec<&str> = SUBMODULES
        .iter()
        .filter(|s| !submodule_available(s.path))
        .map(|s| s.path)
        .collect();
    let m = if missing.is_empty() {
        format!(
            "derived files unavailable ({}): {SETUP_HINT}",
            ensure()[0].1
        )
    } else {
        format!(
            "needs the submodules {} (not checked out): {SETUP_HINT}",
            missing.join(", ")
        )
    };
    if required() {
        panic!("third-party material missing ({REQUIRE_ENV}=1): {m}");
    }
    eprintln!("skipped (third-party): {m}");
    false
}

/// Whether missing material fails tests (`SIMOXIDE_REQUIRE_THIRD_PARTY=1`).
pub fn required() -> bool {
    std::env::var(REQUIRE_ENV)
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// For tests: `true` if the material under `path` is available. Otherwise prints
/// `skipped (third-party): <reason>` to stderr and returns `false`, or panics with the reason when
/// `SIMOXIDE_REQUIRE_THIRD_PARTY=1`.
pub fn skip_unless_available(path: &Path) -> bool {
    match check(path) {
        Ok(()) => true,
        Err(m) => {
            if required() {
                panic!("third-party material missing ({REQUIRE_ENV}=1): {m}");
            }
            eprintln!("skipped (third-party): {m}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ed_scripts_apply_bottom_up() {
        let orig = "a\nb\nc\nd\n";
        // diff -e of orig -> "a\nB\nc\nx\ny\n" (d deleted, x y added, b changed)
        let script = "4c\nx\ny\n.\n2c\nB\n.\n";
        assert_eq!(apply_ed(orig, script).unwrap(), "a\nB\nc\nx\ny\n");
        assert_eq!(apply_ed(orig, "0a\nh\n.\n").unwrap(), "h\na\nb\nc\nd\n");
        assert_eq!(apply_ed(orig, "2,3d\n").unwrap(), "a\nd\n");
        assert!(apply_ed(orig, "9d\n").is_err());
        assert!(apply_ed("x", "1d\n").is_err());
    }

    #[test]
    fn manifest_parses() {
        let m = parse_manifest("# c\n\na/b\tthird_party/x/b\n").unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].script(), PathBuf::from("third_party/derived/a/b.ed"));
    }
}

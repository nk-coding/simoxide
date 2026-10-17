//! Where the QVTo crates find their data: the QVTo root (`crates/qvto/` of the SimOxide
//! repository) and the workspace's target directory.
//!
//! Tests, examples, tools and case files name their data relative to the QVTo root:
//! `reference/...` (specifications, Eclipse sources, the Java runner), `qvto-testkit/cases/...`,
//! and `target/...` for generated material (`target/pcm`, `target/qvto-testkit/java`,
//! `target/qvto-bench`, ...). `target/` there is a link to the workspace's target directory
//! (`CARGO_TARGET_DIR`, else `<repository>/target`), kept up to date by [`root`] and
//! [`find_root`] (and by `tools/qvto-env.sh` for the shell scripts), so the Java runner, the
//! case files and the scripts see the same files as the Rust code.

use std::path::{Path, PathBuf};

/// The QVTo root of this source tree (`crates/qvto`), from this crate's manifest directory, with
/// its `target` link in place. For tests, examples and tools built from the tree.
pub fn root() -> PathBuf {
    let r = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/qvto")
        .to_path_buf();
    ensure_target_link(&r);
    r
}

/// The repository (cargo workspace) root above a QVTo root: `crates/qvto/../..`.
pub fn workspace_root(qvto_root: &Path) -> PathBuf {
    let mut p = qvto_root.to_path_buf();
    p.pop();
    p.pop();
    p
}

/// Finds the QVTo root from a directory inside the repository (the command line tools work from
/// the current directory): a directory with `reference/inventory` and `qvto-testkit`, or one
/// with `crates/qvto/reference/inventory` (the repository root or a directory below it).
pub fn find_root(start: &Path) -> Option<PathBuf> {
    let mut d = std::path::absolute(start).ok()?;
    loop {
        if d.join("reference/inventory").is_dir() && d.join("qvto-testkit").is_dir() {
            ensure_target_link(&d);
            return Some(d);
        }
        let nested = d.join("crates/qvto");
        if nested.join("reference/inventory").is_dir() {
            ensure_target_link(&nested);
            return Some(nested);
        }
        if !d.pop() {
            return None;
        }
    }
}

/// The workspace's target directory: `$CARGO_TARGET_DIR`, else `<repository>/target`.
pub fn target_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("CARGO_TARGET_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    workspace_root(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates/qvto"),
    )
    .join("target")
}

/// Makes `<qvto_root>/target` a link to [`target_dir`] (created or re-pointed; a real directory
/// there is left alone). Errors are ignored: concurrent callers race to the same result.
pub fn ensure_target_link(qvto_root: &Path) {
    let link = qvto_root.join("target");
    let want = target_dir();
    let want = if want.is_absolute() {
        want
    } else {
        workspace_root(qvto_root).join(want)
    };
    match std::fs::symlink_metadata(&link) {
        Ok(m) if !m.file_type().is_symlink() => return,
        Ok(_) if std::fs::read_link(&link).ok().as_deref() == Some(want.as_path()) => return,
        _ => {}
    }
    let _ = std::fs::create_dir_all(&want);
    #[cfg(unix)]
    {
        // atomic: a fresh link under a unique name, renamed over the old one
        let tmp = qvto_root.join(format!(".target-link-{}", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        if std::os::unix::fs::symlink(&want, &tmp).is_ok() && std::fs::rename(&tmp, &link).is_err()
        {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_has_reference_and_target() {
        let r = root();
        assert!(r.join("qvto-paths/Cargo.toml").is_file());
        assert!(r.join("target").exists());
        assert_eq!(find_root(&r.join("qvto-paths/src")).unwrap(), r);
        assert_eq!(find_root(&workspace_root(&r)).unwrap(), r);
    }
}

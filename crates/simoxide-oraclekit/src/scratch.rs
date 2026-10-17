//! Scratch directories for staged inputs and results of one batch.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A fresh directory `<work_dir>/<prefix><pid>-<n>` (unique within the process and among
/// concurrent processes), removed when dropped unless `keep` is set.
#[derive(Debug)]
pub struct Scratch {
    path: PathBuf,
    /// Leave the directory in place when dropped (for debugging a harness).
    pub keep: bool,
}

impl Scratch {
    /// Creates the directory (a stale one of the same name is removed first).
    pub fn new(work_dir: &Path, prefix: &str, keep: bool) -> std::io::Result<Scratch> {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = work_dir.join(format!("{prefix}{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        Ok(Scratch { path, keep })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `self.path().join(p)`.
    pub fn join(&self, p: impl AsRef<Path>) -> PathBuf {
        self.path.join(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_and_removed() {
        let w = std::env::temp_dir().join("oraclekit-scratch-test");
        let a = Scratch::new(&w, "t", false).unwrap();
        let b = Scratch::new(&w, "t", false).unwrap();
        assert_ne!(a.path(), b.path());
        assert!(a.path().is_dir());
        std::fs::write(a.join("f"), "x").unwrap();
        let pa = a.path().to_path_buf();
        drop(a);
        assert!(!pa.exists());
        let mut k = b;
        k.keep = true;
        let pk = k.path().to_path_buf();
        drop(k);
        assert!(pk.is_dir());
        let _ = std::fs::remove_dir_all(&pk);
    }
}

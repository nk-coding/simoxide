//! Test harness: the conformance runner over the Eclipse QVTo test suite and differential
//! testing against the Java reference runner (`reference/runner`).
//!
//! - [`case`]: case files, the job lists both sides read (`cases/eclipse.cases`: the 523 cases
//!   of the inventory, runnable or classified; `cases/corpus.cases`: SimuLizar and SPD).
//! - [`fixture`]: the Java runner's results (result directories, committed fixture bundles,
//!   identity-hash tapes).
//! - [`java`]: running the Java runner in batch mode, recording fixtures, replaying them.
//! - [`engine`]: the [`engine::Engine`] trait the qvto-rs engine implements, and a stub.
//! - [`compare`]: the Eclipse drivers' checks and the strict differential comparison.
//! - [`report`]: the scoreboard.
//!
//! See `README.md` for the formats and the workflow.

pub mod case;
pub mod compare;
pub mod engine;
pub mod fixture;
pub mod idhash;
pub mod inventory;
pub mod java;
pub mod report;

use std::path::{Path, PathBuf};

/// The repository root: the nearest ancestor of `start` with `reference/inventory`.
pub fn find_root(start: &Path) -> Option<PathBuf> {
    let mut d = std::path::absolute(start).ok()?;
    loop {
        if d.join("reference/inventory").is_dir() && d.join("crates").is_dir() {
            return Some(d);
        }
        if !d.pop() {
            return None;
        }
    }
}

/// Paths of the harness's data, relative to the repository root.
pub mod paths {
    /// The Eclipse suite's cases.
    pub const ECLIPSE_CASES: &str = "crates/qvto-testkit/cases/eclipse.cases";
    /// SimuLizar and SPD cases.
    pub const CORPUS_CASES: &str = "crates/qvto-testkit/cases/corpus.cases";
    /// Committed Java results of the Eclipse cases.
    pub const ECLIPSE_FIXTURE: &str = "crates/qvto-testkit/fixtures/java-eclipse.txt";
    /// Committed Java results of the corpus cases.
    pub const CORPUS_FIXTURE: &str = "crates/qvto-testkit/fixtures/java-corpus.txt";
    /// Metamodel configuration of the Eclipse suite (shared with qvto-emf and its oracle).
    pub const SUITE_CONF: &str = "crates/qvto-emf/tests/data/suite.conf";
    /// The inventory.
    pub const INVENTORY: &str = "reference/inventory/eclipse-tests.tsv";
    /// Cache of full Java results (with tapes and XMI), regenerable, not committed.
    pub const JAVA_CACHE: &str = "target/qvto-testkit/java";
}

//! The pieces that SimOxide's three Java reference harnesses share on the Rust side:
//!
//! - refsim, the patched SimuLizar reference (`reference/refsim`), driven by `simoxide-testkit`'s
//!   `RefSim` and `simoxide-fuzz`;
//! - the Eclipse QVTo runner (`crates/qvto/reference/runner/qvto-run`), driven by `qvto-testkit`
//!   (`qvto conform record`, `qvto conform java`, `qvto fuzz`, `qvto bench --java`);
//! - the Slingshot reference runner (`reference/slingshot`), whose scripts share the Java-side
//!   helpers in `reference/oraclekit/` (the seed agent, the JVM launch, the result comparison).
//!
//! Modules:
//!
//! - [`batch`]: a warm-JVM batch process: spawn it, read its result lines with a start-up and a
//!   per-result deadline, kill it when one passes, keep its standard error;
//! - [`java`]: JVM conventions: the shared seed agent and its `-javaagent` option, building a
//!   runner on demand, stack trace excerpts, signs of a JVM in trouble;
//! - [`results`]: result directories (`<dir>/<case>/<file>`), expected files stored as `.gz`, and
//!   comparing two sets of results case by case;
//! - [`scratch`]: unique scratch directories per batch, removed unless kept.
//!
//! The crate knows nothing about the harnesses' formats (traces, fixtures, verdicts): those stay
//! with their harnesses. See `README.md` and the documentation site
//! (`docs/correctness/testing.md`, "Reference harnesses").

pub mod batch;
pub mod java;
pub mod results;
pub mod scratch;

use std::path::{Path, PathBuf};

/// The repository root (two levels above this crate), canonical when possible.
pub fn repo_root() -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    p.canonicalize().unwrap_or(p)
}

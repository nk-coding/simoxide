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
pub mod fuzz;
#[cfg(feature = "compat")]
pub mod idhash;
pub mod inventory;
pub mod java;
pub mod report;

/// The QVTo root (`crates/qvto`) from a directory inside the repository (see [`qvto_paths`]).
pub use qvto_paths::find_root;
pub use qvto_paths::target_dir;

/// Paths of the harness's data, relative to the QVTo root.
pub mod paths {
    /// The Eclipse suite's cases.
    pub const ECLIPSE_CASES: &str = "qvto-testkit/cases/eclipse.cases";
    /// SimuLizar and SPD cases.
    pub const CORPUS_CASES: &str = "qvto-testkit/cases/corpus.cases";
    /// Committed Java results of the Eclipse cases.
    pub const ECLIPSE_FIXTURE: &str = "qvto-testkit/fixtures/java-eclipse.txt";
    /// Committed Java results of the corpus cases.
    pub const CORPUS_FIXTURE: &str = "qvto-testkit/fixtures/java-corpus.txt";
    /// Regression cases of the fuzzing campaigns and the external corpora (Q7).
    pub const FUZZ_CASES: &str = "qvto-testkit/cases/fuzz.cases";
    /// Committed Java results of the fuzz regression cases.
    pub const FUZZ_FIXTURE: &str = "qvto-testkit/fixtures/java-fuzz.txt";
    /// The SPD adjustment corpus (`MainTransformation` on every path; DESIGN-SPD Q1).
    pub const SPD_CASES: &str = "qvto-testkit/cases/spd.cases";
    /// Committed Java results of the SPD cases.
    pub const SPD_FIXTURE: &str = "qvto-testkit/fixtures/java-spd.txt";
    /// Metamodel configuration of the Eclipse suite (shared with qvto-emf and its oracle).
    pub const SUITE_CONF: &str = "qvto-emf/tests/data/suite.conf";
    /// The inventory.
    pub const INVENTORY: &str = "reference/inventory/eclipse-tests.tsv";
    /// Metamodels of the EMF jars registered in the Eclipse environment (copies of their
    /// `.ecore` files in the analyser's fixtures): nsURI and file.
    pub const EMF_JAR_MODELS: &[(&str, &str)] = &[
        (
            "http://www.eclipse.org/emf/2003/XMLType",
            "qvto-sema/tests/fixtures/emf-models/XMLType.ecore",
        ),
        (
            "http://www.w3.org/XML/1998/namespace",
            "qvto-sema/tests/fixtures/emf-models/XMLNamespace.ecore",
        ),
        (
            "http://www.eclipse.org/emf/2003/Change",
            "qvto-sema/tests/fixtures/emf-models/Change.ecore",
        ),
    ];
    /// Cache of full Java results (with tapes and XMI), regenerable, not committed.
    pub const JAVA_CACHE: &str = "target/qvto-testkit/java";
    /// The same for runs of the `det` Java runner (insertion-ordered QVTo/OCL collections).
    pub const JAVA_DET_CACHE: &str = "target/qvto-testkit/java-det";

    /// The fixture of the `det` Java runner next to a fixture: `java-x.txt` -> `java-x-det.txt`.
    pub fn det_fixture(fixture: &str) -> String {
        match fixture.strip_suffix(".txt") {
            Some(stem) => format!("{stem}-det.txt"),
            None => format!("{fixture}-det"),
        }
    }
}

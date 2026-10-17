//! Records the adapter's results on the SimuLizar/SPD corpus (journal as text, output dump, log,
//! status) for the comparison after the adapter is removed (merge step M6). Writes the files when
//! `RECORD_ADAPTER_CORPUS` names a directory; otherwise it only runs.

#[path = "common/corpus.rs"]
mod corpus;

use qvto_testkit::compare::renumber_extent_resources as renumber;

#[test]
fn record_adapter_corpus() {
    if !corpus::available() {
        eprintln!("skipped: no target/pcm");
        return;
    }
    let root = corpus::repo_root();
    let dir = std::env::var_os("RECORD_ADAPTER_CORPUS").map(std::path::PathBuf::from);
    for case in corpus::cases() {
        let a = corpus::run_adapter(&root, &case);
        assert!(a.compile_errors.is_empty(), "{}", case.id);
        if let Some(d) = &dir {
            std::fs::create_dir_all(d).unwrap();
            std::fs::write(d.join(format!("{}.journal", case.id)), &a.journal_text).unwrap();
            std::fs::write(d.join(format!("{}.dump", case.id)), renumber(&a.dump)).unwrap();
            std::fs::write(
                d.join(format!("{}.status", case.id)),
                format!(
                    "{} {}\n{}\n--log--\n{}",
                    a.severity, a.code, a.message, a.log
                ),
            )
            .unwrap();
        }
    }
}

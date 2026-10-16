//! Replays the committed Java answers in `tests/golden/*.tsv` (samples of the differential
//! tests, written with `QVTO_WRITE_GOLDEN=1`; see `tests/differential.rs`). Needs no JDK.

mod common;

use common::{crate_dir, eval, read_golden};

#[test]
fn golden_files_agree() {
    let dir = crate_dir().join("tests/golden");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("tests/golden")
        .map(|e| e.unwrap().path())
        .collect();
    files.sort();
    assert!(!files.is_empty());
    let mut total = 0;
    let mut bad = Vec::new();
    for f in &files {
        for (req, want) in read_golden(f) {
            total += 1;
            let got = eval(&req);
            if got != want {
                bad.push(format!(
                    "{}: {req}\n  java: {want}\n  rust: {got}",
                    f.file_name().unwrap().to_string_lossy()
                ));
            }
        }
    }
    for b in bad.iter().take(10) {
        eprintln!("{b}");
    }
    assert!(
        bad.is_empty(),
        "{} of {total} golden cases differ",
        bad.len()
    );
    eprintln!("{total} golden cases agree");
}

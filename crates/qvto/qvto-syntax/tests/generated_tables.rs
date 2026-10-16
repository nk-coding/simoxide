//! Checks that `src/generated.rs` equals what the generator derives from the `.gi` files.
//! Skipped (with a note) when the Eclipse sources are not present.

#[path = "../gen/lpg.rs"]
mod lpg;

use std::path::Path;

#[test]
fn generated_tables_match_grammars() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let Some(dir) = lpg::eclipse_dir(manifest) else {
        eprintln!("skipped: reference/eclipse-qvto not present");
        return;
    };
    let tables = lpg::derive(&lpg::Paths::new(&dir))
        .unwrap_or_else(|e| panic!("grammar check failed:\n{e}"));
    let text = lpg::render(&tables);
    let committed = std::fs::read_to_string(manifest.join("src/generated.rs")).unwrap();
    assert!(
        committed == text,
        "src/generated.rs is out of date: run `cargo run -p qvto-syntax --example gen_tables`"
    );
}

//! Regenerates `src/generated.rs` from Eclipse QVTo's LPG grammars.
//!
//! ```sh
//! cargo run -p qvto-syntax --example gen_tables            # write src/generated.rs
//! cargo run -p qvto-syntax --example gen_tables -- --check # fail if it is out of date
//! ```
//!
//! The grammars are found at `reference/eclipse-qvto` (see `reference/README.md`) or at
//! `$QVTO_ECLIPSE_DIR`.

#[path = "../gen/lpg.rs"]
mod lpg;

use std::path::Path;

fn main() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let check = std::env::args().any(|a| a == "--check");
    let dir = lpg::eclipse_dir(manifest).expect("Eclipse QVTo sources not found: see reference/README.md or set QVTO_ECLIPSE_DIR");
    let tables = lpg::derive(&lpg::Paths::new(&dir)).unwrap_or_else(|e| {
        eprintln!("grammar check failed:\n{e}");
        std::process::exit(1)
    });
    let text = lpg::render(&tables);
    let out = manifest.join("src/generated.rs");
    let old = std::fs::read_to_string(&out).unwrap_or_default();
    if check {
        if old != text {
            eprintln!("{} is out of date; run the generator", out.display());
            std::process::exit(1);
        }
        println!("{} is up to date", out.display());
        return;
    }
    std::fs::write(&out, &text).expect("write generated.rs");
    println!(
        "wrote {}: {} terminals ({} punctuation), {} keywords, {} groups, {} operator levels",
        out.display(),
        tables.terminals.len(),
        tables.punct.len(),
        tables.keywords.len(),
        tables.groups.len(),
        tables.binary_levels.len()
    );
}

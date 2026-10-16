//! Writes the mutants of the corpus (`tests/common/mutate.rs`) to a directory, one file per
//! mutant, and prints `path<TAB>id` lines; `tools/oracle/make-mutant-fixture.sh` runs Eclipse's
//! parser on them.
//!
//! ```sh
//! cargo run -p qvto-syntax --example mutants -- OUTDIR > index.tsv
//! ```

#[path = "../tests/common/mod.rs"]
mod common;

fn main() {
    let arg = std::env::args().nth(1).expect("output directory or --parse");
    if arg == "--parse" {
        // Parse every mutant in process, printing each id first (to find hangs and crashes).
        let roots = common::roots();
        for key in common::oracle().keys() {
            let Some(path) = common::resolve(&roots, key) else { continue };
            let src = common::read(&path);
            for m in common::mutate::mutants(key, &src) {
                eprintln!("{}", m.id);
                std::hint::black_box(qvto_syntax::parse(&m.text));
            }
        }
        return;
    }
    let out = std::path::PathBuf::from(arg);
    std::fs::create_dir_all(&out).unwrap();
    let roots = common::roots();
    let mut n = 0;
    for key in common::oracle().keys() {
        let Some(path) = common::resolve(&roots, key) else { continue };
        let src = common::read(&path);
        for m in common::mutate::mutants(key, &src) {
            let p = out.join(format!("m{n:05}.qvto"));
            std::fs::write(&p, &m.text).unwrap();
            println!("{}\t{}", p.display(), m.id);
            n += 1;
        }
    }
    eprintln!("{n} mutants");
}

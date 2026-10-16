//! Prints the span-free AST dump and the diagnostics of a file (or of stdin with `-`).
//! `--print` prints the unparsed source instead of the dump.
use std::io::Read;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "-".into());
    let mut src = String::new();
    if path == "-" {
        std::io::stdin().read_to_string(&mut src).unwrap();
    } else {
        src = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
    }
    let parsed = qvto_syntax::parse(&src);
    if args.iter().any(|a| a == "--print") {
        print!("{}", qvto_syntax::print::unit(&parsed.unit));
    } else {
        println!("{}", qvto_syntax::dump::unit(&parsed.unit));
    }
    let lines = qvto_syntax::LineIndex::new(&src);
    for d in &parsed.diagnostics {
        eprintln!("{}", d.render(&path, &src, &lines));
    }
}

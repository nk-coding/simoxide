//! Parses the files listed on stdin (one path per line) and prints their problems in the
//! format of the Eclipse parse oracle (`tools/oracle/ParseOracle.java`): one line per problem,
//! `path<TAB>E|W<TAB>offset<TAB>length<TAB>line<TAB>message` with UTF-16 offsets, or
//! `path<TAB>OK`. With `--time N`, parses everything N times and reports the best time.
//!
//! ```sh
//! find … -name '*.qvto' | cargo run --release -p qvto-syntax --example parse_files
//! ```

use qvto_syntax::{LineIndex, Severity, span};
use std::io::BufRead;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let repeat: usize = args
        .iter()
        .position(|a| a == "--time")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let paths: Vec<String> = std::io::stdin()
        .lock()
        .lines()
        .map_while(Result::ok)
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect();
    let texts: Vec<String> = paths
        .iter()
        .map(|p| String::from_utf8_lossy(&std::fs::read(p).expect(p)).into_owned())
        .collect();
    for (p, src) in paths.iter().zip(&texts) {
        let parsed = qvto_syntax::parse(src);
        if parsed.diagnostics.is_empty() {
            println!("{p}\tOK");
            continue;
        }
        let lines = LineIndex::new(src);
        for d in &parsed.diagnostics {
            let off = span::byte_to_utf16(src, d.span.start as usize);
            let len = span::byte_to_utf16(src, d.span.end as usize) - off;
            let sev = if d.severity == Severity::Error {
                "E"
            } else {
                "W"
            };
            println!(
                "{p}\t{sev}\t{off}\t{len}\t{}\t{}",
                lines.line(d.span.start),
                d.message
            );
        }
    }
    if repeat > 0 {
        let bytes: usize = texts.iter().map(String::len).sum();
        let mut best = f64::MAX;
        for _ in 0..repeat {
            let t0 = Instant::now();
            let mut n = 0usize;
            for src in &texts {
                n += qvto_syntax::parse(src).unit.elements.len();
            }
            std::hint::black_box(n);
            best = best.min(t0.elapsed().as_secs_f64());
        }
        eprintln!(
            "files={} bytes={bytes} best_ms={:.2} MB/s={:.1}",
            texts.len(),
            best * 1e3,
            bytes as f64 / best / 1e6
        );
    }
}

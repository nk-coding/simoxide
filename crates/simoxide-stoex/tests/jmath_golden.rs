//! `Math.pow` / `Math.log` of HotSpot (Java 17/21, x86-64) vs `simoxide_stoex::jmath`.
//! Golden file: `reference/oracles/stoex/golden/javamath.txt` (from `oracle.JavaMathDump`);
//! `STOEX_MATH_DUMP=<file>` checks another (larger) dump.

use std::io::BufRead;

fn hex(s: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(s, 16).unwrap())
}

fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits()
}

#[test]
fn pow_and_log_match_hotspot() {
    let path = std::env::var("STOEX_MATH_DUMP").unwrap_or_else(|_| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../reference/oracles/stoex/golden/javamath.txt"
        )
        .to_string()
    });
    let f = std::fs::File::open(&path).expect("math dump");
    let (mut n_pow, mut n_log, mut bad_pow, mut bad_log) = (0usize, 0usize, 0usize, 0usize);
    let mut examples = Vec::new();
    for line in std::io::BufReader::new(f).lines() {
        let line = line.unwrap();
        let t: Vec<&str> = line.split(' ').collect();
        let (x, y, java) = (hex(t[1]), hex(t[2]), hex(t[3]));
        if t[0] == "pow" {
            n_pow += 1;
            let r = simoxide_stoex::jmath::pow(x, y);
            if !same(r, java) {
                bad_pow += 1;
                let ulps = (r.to_bits() as i64 - java.to_bits() as i64).abs();
                assert!(
                    ulps <= 1 && !java.is_nan(),
                    "pow({x:e}, {y:e}) = {r:e}, java {java:e}"
                );
                if examples.len() < 5 {
                    examples.push(format!("pow({x:e},{y:e}): {r:e} vs {java:e}"));
                }
            }
        } else if t[0] == "rem" {
            let r = simoxide_stoex::jmath::drem(x, y);
            assert!(
                same(r, java),
                "{x:e} % {y:e} = {r:e} ({:016x}), java {java:e} ({:016x})",
                r.to_bits(),
                java.to_bits()
            );
        } else {
            n_log += 1;
            let r = simoxide_stoex::jmath::log(x);
            if !same(r, java) {
                bad_log += 1;
                if examples.len() < 10 {
                    examples.push(format!("log({x:e}): {r:e} vs {java:e}"));
                }
            }
        }
    }
    eprintln!("pow: {bad_pow}/{n_pow} differ by 1 ulp; log: {bad_log}/{n_log}; {examples:?}");
    assert_eq!(bad_log, 0, "{examples:?}");
    // Intel's pow is not correctly rounded in ~0.05% of cases.
    assert!(bad_pow * 1000 <= n_pow, "{bad_pow}/{n_pow} {examples:?}");
}

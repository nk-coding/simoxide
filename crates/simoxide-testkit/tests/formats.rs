//! Byte-exact round trips of every corpus expected file through the parsers and writers, number
//! formatting against the reference JVM, and consistency between trace, tape and measurements.

use std::io::Read;

use proptest::prelude::*;
use simoxide_testkit::corpus::{self, corpus_dir};
use simoxide_testkit::json::Value;
use simoxide_testkit::measurements::Measurements;
use simoxide_testkit::tape::Tape;
use simoxide_testkit::trace::{EventKind, Trace};
use simoxide_testkit::{javafmt, json};

fn entries() -> Vec<corpus::Entry> {
    corpus::list(&corpus_dir(), None).expect("corpus")
}

#[test]
fn corpus_files_roundtrip_byte_identical() {
    let es = entries();
    assert!(es.len() >= 61, "corpus has {} entries", es.len());
    let mut lines = 0usize;
    for e in &es {
        let x = e.expected().unwrap();
        let t = x.trace.as_deref().expect("trace");
        let tr = Trace::parse(t).unwrap_or_else(|err| panic!("{}: {err}", e.name));
        assert_eq!(tr.to_text(), t, "{}: trace round trip", e.name);
        lines += tr.len();
        let p = x.tape.as_deref().expect("tape");
        let tp = Tape::parse(p).unwrap_or_else(|err| panic!("{}: {err}", e.name));
        assert_eq!(tp.to_text(), p, "{}: tape round trip", e.name);
        let m =
            Measurements::parse(&x.measurements).unwrap_or_else(|err| panic!("{}: {err}", e.name));
        assert_eq!(m.to_csv(), x.measurements, "{}: csv round trip", e.name);
        // re-sorting must be a no-op (the file is in Java String order)
        let mut ms = m.clone();
        ms.sort();
        assert_eq!(ms, m, "{}: series order", e.name);

        // consistency: header/finish, meas events == csv rows (per series, emission order)
        let h = tr.header().expect("header");
        assert_eq!(h.get_str("run"), Some(e.name.as_str()));
        assert_eq!(h.get_i64("seed"), Some(e.config.seed));
        let f = tr.finish().expect("finish");
        assert_eq!(f.get_i64("uniforms"), Some(tp.len() as i64), "{}", e.name);
        assert_eq!(
            f.get_i64("measurements"),
            Some(m.total() as i64),
            "{}",
            e.name
        );
        let mut from_trace = Measurements::default();
        let mut rec = simoxide_testkit::measurements::Recorder::new();
        for r in &tr.records {
            if EventKind::of_record(r) == EventKind::Meas {
                let id = rec.series_id(r.get_str("mp").unwrap(), r.get_str("metric").unwrap());
                let Some(Value::Nums(v)) = r.get("v") else {
                    panic!("v")
                };
                rec.record(id, v[0], v[1]);
            }
        }
        from_trace.series = rec.into_measurements().series;
        assert_eq!(
            from_trace.to_csv(),
            x.measurements,
            "{}: trace meas vs csv",
            e.name
        );
        // every sample record's uniforms carry its origin
        for s in &tp.samples {
            for i in s.first_uniform..s.first_uniform + s.n {
                assert_eq!(
                    tp.origins[i as usize], s.origin,
                    "{}: sample origin",
                    e.name
                );
            }
        }
    }
    eprintln!("{} corpus entries, {lines} trace lines", es.len());
}

/// Every number token of every expected file re-formats to itself.
#[test]
fn corpus_numbers_format_identically() {
    let mut n = 0usize;
    for e in entries() {
        let x = e.expected().unwrap();
        for text in [x.trace.unwrap(), x.tape.unwrap()] {
            for l in text.lines() {
                for (_, v) in json::parse_line(l).unwrap().fields {
                    match v {
                        Value::Num(d) => {
                            n += 1;
                            let s = javafmt::to_string(d);
                            assert!(l.contains(&s), "{}: {d:?} -> {s} not in {l}", e.name);
                            if n.is_multiple_of(64) {
                                assert_eq!(javafmt::exact::to_string(d), s);
                            }
                        }
                        Value::Nums(v) => {
                            for d in v {
                                n += 1;
                                let s = javafmt::to_string(d);
                                assert!(l.contains(&s), "{}: {d:?} -> {s} not in {l}", e.name);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        for l in x.measurements.lines().skip(1) {
            let f: Vec<&str> = l.rsplitn(3, ',').collect();
            for tok in &f[..2] {
                let d: f64 = tok.trim_matches('"').parse().unwrap();
                n += 1;
                assert_eq!(
                    &javafmt::to_string(d),
                    tok.trim_matches('"'),
                    "{}: {l}",
                    e.name
                );
            }
        }
    }
    eprintln!("{n} numbers checked");
}

/// Golden `Double.toString` output of the reference JVM (Java 21): edge cases, all small
/// subnormals, powers of 2 and 10 with neighbours, random bit patterns
/// (`tests/data/DoubleGolden.java 10000 20260929`).
#[test]
fn java21_golden() {
    let p = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/data/java-double-tostring.txt.gz"
    );
    let mut s = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(p).unwrap())
        .read_to_string(&mut s)
        .unwrap();
    let mut n = 0;
    for l in s.lines() {
        let (h, want) = l.split_once(' ').unwrap();
        let d = f64::from_bits(u64::from_str_radix(h, 16).unwrap());
        assert_eq!(javafmt::to_string(d), want, "bits {h}");
        n += 1;
    }
    assert!(n > 20000);
}

/// Runs the Java generator live for 10^6 random doubles (needs `java` on PATH; ~10 s).
#[test]
#[ignore]
fn java21_live_million() {
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/DoubleGolden.java");
    let out = std::process::Command::new("java")
        .args([src, "1000000", "7"])
        .output()
        .expect("java");
    let s = String::from_utf8(out.stdout).unwrap();
    let mut n = 0;
    for l in s.lines() {
        let (h, want) = l.split_once(' ').unwrap();
        let d = f64::from_bits(u64::from_str_radix(h, 16).unwrap());
        assert_eq!(javafmt::to_string(d), want, "bits {h}");
        n += 1;
    }
    assert!(n > 1_000_000);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4000))]
    #[test]
    fn fast_equals_exact_any_bits(bits in any::<u64>()) {
        let d = f64::from_bits(bits);
        prop_assume!(d.is_finite());
        prop_assert_eq!(javafmt::to_string(d), javafmt::exact::to_string(d));
        // round trip
        prop_assert_eq!(javafmt::to_string(d).parse::<f64>().unwrap().to_bits(), bits);
    }

    #[test]
    fn fast_equals_exact_subnormal(m in 1u64..(1u64 << 52)) {
        let d = f64::from_bits(m);
        prop_assert_eq!(javafmt::to_string(d), javafmt::exact::to_string(d));
    }

    #[test]
    fn fast_equals_exact_short_decimals(c in 1u64..100_000, e in -330i32..310) {
        let d: f64 = format!("{c}e{e}").parse().unwrap();
        prop_assume!(d.is_finite() && d != 0.0);
        prop_assert_eq!(javafmt::to_string(d), javafmt::exact::to_string(d));
    }
}

//! Throughput of the trace writer and number formatter:
//! `cargo run --release -p simoxide-testkit --example bench-writer`.
use std::time::Instant;

use simoxide_testkit::trace::TraceWriter;

fn main() {
    let n = 2_000_000u64;
    let mut x = 0x1234_5678_9abc_def0u64;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x >> 11) as f64 / (1u64 << 53) as f64
    };
    let vals: Vec<f64> = (0..n).map(|_| -next().ln() * 0.05).collect();
    let t0 = Instant::now();
    let mut buf = Vec::with_capacity(32);
    let mut total = 0usize;
    for &v in &vals {
        buf.clear();
        simoxide_testkit::javafmt::write(&mut buf, v);
        total += buf.len();
    }
    let dt = t0.elapsed();
    println!(
        "javafmt: {:.1} ns/double ({total} bytes)",
        dt.as_nanos() as f64 / n as f64
    );
    let t0 = Instant::now();
    let mut w = TraceWriter::new(std::io::sink());
    let mut t = 0.0;
    for (i, &v) in vals.iter().enumerate() {
        t += v;
        let t = simoxide_testkit::trace::t_from_ns((t * 1e9) as i64);
        if i % 2 == 0 {
            w.demand(
                t,
                7,
                "_oro4gG3fEdy4YaaT-RYrLQ",
                "_rc1",
                "_prs1",
                "PROCESSOR_SHARING",
                v,
                v,
            );
        } else {
            w.element(true, t, 7, "InternalAction", "_act12", Some("_ac3"));
        }
    }
    w.finish().unwrap();
    let dt = t0.elapsed();
    println!(
        "TraceWriter: {:.1} ns/event",
        dt.as_nanos() as f64 / n as f64
    );
}

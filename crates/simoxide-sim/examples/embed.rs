//! Embedding simoxide-sim: model files in memory -> run with limits -> summary statistics.
//!
//! `cargo run --release -p simoxide-sim --example embed -- corpus/x_sl_mediastore`
use simoxide_sim::{Limits, RunError, RunSpec, SimErrorKind, simulate_memory};
use std::time::{Duration, Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).ok_or("usage: embed <dir>")?);
    // (file name, XMI bytes), e.g. received over the network; run.json is optional
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for e in std::fs::read_dir(&dir)? {
        let p = e?.path();
        if p.is_file() && p.extension().is_some_and(|x| x != "json") {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            files.push((name, std::fs::read(&p)?));
        }
    }
    let spec = match std::fs::read_to_string(dir.join("run.json")) {
        Ok(t) => RunSpec::parse(&t)?,
        Err(_) => RunSpec {
            seed: 1,
            max_measurements: 1000,
            ..RunSpec::default()
        },
    };
    let limits = Limits {
        deadline: Some(Instant::now() + Duration::from_secs(30)),
        max_events: 100_000_000,
        ..Limits::default()
    };
    match simulate_memory(&files, &spec, limits) {
        Ok(r) => {
            println!(
                "{} events, end at {} s, {} tuples",
                r.events,
                r.end_ns as f64 / 1e9,
                r.measurements.count
            );
            for s in r.measurements.summaries() {
                println!(
                    "{:<60} {:<32} n={:<7} mean={:.6} p95={:.6} tw_mean={:?}",
                    s.measuring_point, s.metric, s.count, s.mean, s.p95, s.time_weighted_mean
                );
            }
        }
        Err(RunError::Sim(e)) if e.kind == SimErrorKind::Model => {
            eprintln!("the model aborts (as in SimuLizar): {e}")
        }
        Err(e) => eprintln!("{e}"),
    }
    Ok(())
}

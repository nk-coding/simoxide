//! Quantifies how far the O(log n) virtual-time PS is from the bit-exact PS, on all oracle PS
//! scripts. Completion times are compared per (job, step): the virtual-time variant stays within
//! a few nanoseconds (it reorders float operations, see `src/ps_vt.rs`), except where that flips
//! a lost-time decision.

mod common;

use std::collections::HashMap;
use std::fs;

use simoxide_sched::PsAlgorithm;

/// Per (job, occurrence) completion times of a trace.
fn completion_map(trace: &str) -> HashMap<(String, usize), i64> {
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut m = HashMap::new();
    for (job, t) in common::completions(trace) {
        let k = seen.entry(job.clone()).or_default();
        m.insert((job, *k), t);
        *k += 1;
    }
    m
}

#[test]
fn virtual_time_ps_is_close_to_exact() {
    let dir = common::oracle_dir().join("scripts");
    let mut names: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.starts_with("ps") || n.starts_with("rnd_ps"))
        .collect();
    names.sort();
    let (mut total, mut differing, mut identical_traces, mut max_abs) =
        (0usize, 0usize, 0usize, 0i64);
    let mut report = String::new();
    for n in &names {
        let sc = common::parse(&fs::read_to_string(dir.join(n)).unwrap());
        let exact = common::run(&sc, PsAlgorithm::Exact);
        let vt = common::run(&sc, PsAlgorithm::VirtualTime);
        // R lines print remaining demands whose bit patterns legitimately differ
        let strip = |s: &str| {
            s.lines()
                .filter(|l| !l.starts_with("R "))
                .collect::<Vec<_>>()
                .join("\n")
        };
        if strip(&exact) == strip(&vt) {
            identical_traces += 1;
        }
        let (e, v) = (completion_map(&exact), completion_map(&vt));
        assert_eq!(e.len(), v.len(), "{n}: number of completions differs");
        let mut file_max = 0;
        for (k, te) in &e {
            let tv = v[k];
            total += 1;
            if tv != *te {
                differing += 1;
            }
            file_max = file_max.max((tv - te).abs());
        }
        max_abs = max_abs.max(file_max);
        report.push_str(&format!("{n}: max |dt| = {file_max} ns\n"));
    }
    eprintln!(
        "{report}virtual-time vs exact PS: {identical_traces}/{} traces identical (ignoring R lines), \
         {differing}/{total} completion times differ, max |dt| = {max_abs} ns",
        names.len()
    );
    // Observed: max 10 ns (docs/spec/scheduler.md). No hard guarantee in general: a rounding
    // difference can flip the 1e-5 s lost-time decision of an interval (<= 10 us each).
    assert!(
        max_abs <= 10_000,
        "virtual-time PS deviates by {max_abs} ns"
    );
    assert!(
        differing > 0,
        "expected some rounding differences (is the comparison working?)"
    );
}

/// Throughput under heavy contention (run with `--release --ignored --nocapture`).
#[test]
#[ignore]
fn perf_heavy_contention() {
    use simoxide_sched::{ActiveResource, SchedulingPolicy};
    let mut rng = common::SplitMix(99);
    let w: Vec<(f64, f64)> = (0..300_000)
        .map(|_| (rng.exp(0.998), rng.exp(1.0)))
        .collect();
    for alg in [PsAlgorithm::Exact, PsAlgorithm::VirtualTime] {
        let mut res = ActiveResource::new(SchedulingPolicy::ProcessorSharing, 1, alg);
        let (mut max_n, mut sum_n, mut calls) = (0u64, 0u64, 0u64);
        let t = std::time::Instant::now();
        common::run_open(
            &mut res,
            w.iter().copied(),
            |r| {
                let n = r.queue_length(0);
                max_n = max_n.max(n);
                sum_n += n;
                calls += 1;
            },
            |_, _, _| {},
        );
        eprintln!(
            "{alg:?}: {:.3} s for {} jobs, mean jobs in system at events {:.0}, max {max_n}",
            t.elapsed().as_secs_f64(),
            w.len(),
            sum_n as f64 / calls as f64
        );
    }
}

/// A job that demands again while it is still served (early resume, SIM-4.4a) keeps one entry
/// and its position and gets the new demand, in both implementations.
#[test]
fn requeued_job_replaces_its_demand_in_both_algorithms() {
    use simoxide_sched::{ActiveResource, SchedulingPolicy};
    let mut out = Vec::new();
    for alg in [PsAlgorithm::Exact, PsAlgorithm::VirtualTime] {
        let mut r = ActiveResource::new(SchedulingPolicy::ProcessorSharing, 1, alg);
        let mut done = Vec::new();
        r.process(0, 1u64, 2.0, &mut ()).unwrap();
        r.process(0, 2u64, 3.0, &mut ()).unwrap();
        // job 1 again at t = 1 s with a new demand of 0.5 s
        let mut w = r.process(1_000_000_000, 1u64, 0.5, &mut ()).unwrap();
        loop {
            let c = r.on_wakeup(w.at, &w, &mut ()).unwrap().expect("current");
            done.push((c.job, w.at));
            match c.next {
                Some(n) => w = n,
                None => break,
            }
        }
        assert_eq!(r.queue_length(0), 0);
        out.push(done);
    }
    // job 1: 0.5 s left at t = 1, shared by two -> t = 2; job 2: 2.5 s left at t = 1, 2 at t = 2
    // -> alone until t = 4
    assert_eq!(out[0], vec![(1, 2_000_000_000), (2, 4_000_000_000)]);
    assert_eq!(out[0], out[1]);
}

//! Long analytical checks (run with `cargo test -p simoxide-sched --release -- --ignored`):
//! mean response times of M/M/1 and M/M/c queues against the closed forms, with batch means.

mod common;

use common::{SplitMix, run_open};
use simoxide_sched::time::seconds;
use simoxide_sched::{ActiveResource, PsAlgorithm, SchedulingPolicy};

/// Mean response time and the half-width of its ~99.7% confidence interval (batch means).
fn mean_response(
    res: &mut ActiveResource<u32>,
    lambda: f64,
    mu: f64,
    jobs: usize,
    seed: u64,
) -> (f64, f64) {
    let mut rng = SplitMix(seed);
    let arrivals: Vec<(f64, f64)> = (0..jobs).map(|_| (rng.exp(lambda), rng.exp(mu))).collect();
    let mut resp = vec![0.0; jobs];
    run_open(
        res,
        arrivals,
        |_| {},
        |j, a, c| resp[j as usize] = seconds(c - a),
    );
    let warmup = jobs / 20;
    let batches = 30;
    let per = (jobs - warmup) / batches;
    let means: Vec<f64> = (0..batches)
        .map(|b| {
            resp[warmup + b * per..warmup + (b + 1) * per]
                .iter()
                .sum::<f64>()
                / per as f64
        })
        .collect();
    let m = means.iter().sum::<f64>() / batches as f64;
    let var = means.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (batches - 1) as f64;
    (m, 3.0 * (var / batches as f64).sqrt())
}

/// Erlang C mean response time of M/M/c.
fn mmc_response(lambda: f64, mu: f64, c: u32) -> f64 {
    let a = lambda / mu;
    let mut sum = 0.0;
    let mut term = 1.0;
    for k in 0..c {
        if k > 0 {
            term *= a / k as f64;
        }
        sum += term;
    }
    let last = term * a / c as f64 * c as f64 / (c as f64 - a);
    let p_wait = last / (sum + last);
    1.0 / mu + p_wait / (c as f64 * mu - lambda)
}

fn check(policy: SchedulingPolicy, cores: u32, alg: PsAlgorithm, lambda: f64, expected: f64) {
    let mut res = ActiveResource::new(policy, cores, alg);
    let (m, hw) = mean_response(&mut res, lambda, 1.0, 2_000_000, 7 + cores as u64);
    eprintln!(
        "{policy:?} c={cores} {alg:?}: mean response {m:.4} +- {hw:.4}, expected {expected:.4}"
    );
    assert!(
        (m - expected).abs() <= hw.max(0.01 * expected),
        "{m} vs {expected} (+- {hw})"
    );
}

#[test]
#[ignore]
fn mm1_ps() {
    check(
        SchedulingPolicy::ProcessorSharing,
        1,
        PsAlgorithm::Exact,
        0.7,
        1.0 / (1.0 - 0.7),
    );
}

#[test]
#[ignore]
fn mm1_ps_virtual_time() {
    check(
        SchedulingPolicy::ProcessorSharing,
        1,
        PsAlgorithm::VirtualTime,
        0.7,
        1.0 / (1.0 - 0.7),
    );
}

/// Multi-core PS with exponential service has the M/M/c number-in-system distribution.
#[test]
#[ignore]
fn mmc_ps() {
    for c in [2u32, 4] {
        let lambda = 0.75 * c as f64;
        check(
            SchedulingPolicy::ProcessorSharing,
            c,
            PsAlgorithm::Exact,
            lambda,
            mmc_response(lambda, 1.0, c),
        );
    }
}

#[test]
#[ignore]
fn mm1_fcfs() {
    check(
        SchedulingPolicy::Fcfs,
        1,
        PsAlgorithm::Exact,
        0.7,
        1.0 / (1.0 - 0.7),
    );
}

#[test]
#[ignore]
fn m_m_inf_delay() {
    check(SchedulingPolicy::Delay, 1, PsAlgorithm::Exact, 5.0, 1.0);
}

#[test]
fn erlang_c_sanity() {
    assert!((mmc_response(0.7, 1.0, 1) - 1.0 / 0.3).abs() < 1e-12);
    assert!((mmc_response(1.6, 1.0, 2) - (1.0 + (6.4 / 9.0) / 0.4)).abs() < 1e-12);
}

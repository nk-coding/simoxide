//! Property tests: invariants every resource model must satisfy on arbitrary workloads.

mod common;

use proptest::prelude::*;

use common::run_open;
use simoxide_sched::time::span;
use simoxide_sched::{
    ActiveResource, PassiveListener, PassiveResource, PsAlgorithm, SchedulingPolicy, SimTime,
};

/// (inter-arrival, demand) pairs on a 1 ms grid (gaps are 0 or >= 1 ms, so the 1e-5 s lost-time
/// rule never applies) with demands >= 1 ms.
fn grid_workload(max: usize) -> impl Strategy<Value = Vec<(f64, f64)>> {
    prop::collection::vec((0u32..40, 1u32..3000), 1..max).prop_map(|v| {
        v.into_iter()
            .map(|(ia, d)| (ia as f64 * 1e-3, d as f64 * 1e-3))
            .collect()
    })
}

/// Arbitrary (inter-arrival, demand) pairs, including sub-epsilon gaps and sub-JIFFY demands.
fn wild_workload(max: usize) -> impl Strategy<Value = Vec<(f64, f64)>> {
    let gap = prop_oneof![Just(0.0), 0.0..2e-5, 0.0..3.0];
    let demand = prop_oneof![1e-12..1e-8, 1e-6..1e-4, 1e-3..5.0];
    prop::collection::vec((gap, demand), 1..max)
}

struct Outcome {
    arrival: Vec<SimTime>,
    completion: Vec<SimTime>,
    order: Vec<u32>,
    stale: u64,
}

fn simulate(res: &mut ActiveResource<u32>, w: &[(f64, f64)]) -> Outcome {
    let n = w.len();
    let mut o = Outcome {
        arrival: vec![-1; n],
        completion: vec![-1; n],
        order: vec![],
        stale: 0,
    };
    let cores = res.cores();
    o.stale = run_open(
        res,
        w.iter().copied(),
        |r| {
            // per-core states: non-increasing, differ by at most one
            let q: Vec<u64> = (0..cores).map(|c| r.queue_length(c)).collect();
            assert!(
                q.windows(2).all(|p| p[0] >= p[1] && p[0] - p[1] <= 1),
                "{q:?}"
            );
        },
        |job, a, c| {
            assert_eq!(o.completion[job as usize], -1, "job completed twice");
            o.arrival[job as usize] = a;
            o.completion[job as usize] = c;
            o.order.push(job);
        },
    );
    assert!(
        o.completion.iter().all(|&c| c >= 0),
        "a job never completed"
    );
    o
}

fn ps(cores: u32, alg: PsAlgorithm) -> ActiveResource<u32> {
    ActiveResource::new(SchedulingPolicy::ProcessorSharing, cores, alg)
}

proptest! {
    // 256 cases by default; more with PROPTEST_CASES=n

    /// Every job completes once, never earlier than its demand allows (at most one core per job).
    #[test]
    fn ps_completes_every_job(w in wild_workload(60), cores in 1u32..5) {
        for alg in [PsAlgorithm::Exact, PsAlgorithm::VirtualTime] {
            let o = simulate(&mut ps(cores, alg), &w);
            for (i, &(_, d)) in w.iter().enumerate() {
                let service = o.completion[i] - o.arrival[i];
                prop_assert!(service >= span(d.max(1e-9)) - 2, "job {i}: {service} ns for demand {d}");
            }
        }
    }

    /// Single-core PS and FCFS are both work conserving: same busy periods, same makespan --
    /// up to the reference's lost-time rule: whenever two consecutive resource events are less
    /// than 1e-5 s apart (e.g. an arrival right after a completion), that interval is not
    /// served. Bound: 2 events per job, each losing < 1e-5 s, plus 1 ns truncation.
    #[test]
    fn work_conservation_ps_vs_fcfs(w in grid_workload(80)) {
        let p = simulate(&mut ps(1, PsAlgorithm::Exact), &w);
        let f = simulate(&mut ActiveResource::new(SchedulingPolicy::Fcfs, 1, PsAlgorithm::Exact), &w);
        let (mp, mf) = (*p.completion.iter().max().unwrap(), *f.completion.iter().max().unwrap());
        prop_assert!((mp - mf).abs() <= 2 * w.len() as i64 * 10_001, "PS {mp} vs FCFS {mf}");
        // total demand is a lower bound on the busy time
        let total: f64 = w.iter().map(|x| x.1).sum();
        prop_assert!(mf - p.arrival[0] >= span(total) - w.len() as i64);
    }

    /// FCFS serves in arrival order and each job starts when its predecessor leaves.
    #[test]
    fn fcfs_order(w in wild_workload(60)) {
        let o = simulate(&mut ActiveResource::new(SchedulingPolicy::Fcfs, 1, PsAlgorithm::Exact), &w);
        prop_assert_eq!(o.order.clone(), (0..w.len() as u32).collect::<Vec<_>>());
    }

    /// PS fairness: a job that arrives no later and demands no more finishes no later.
    #[test]
    fn ps_fairness(w in grid_workload(60), cores in 1u32..5) {
        let o = simulate(&mut ps(cores, PsAlgorithm::Exact), &w);
        for i in 0..w.len() {
            for j in 0..w.len() {
                if o.arrival[i] <= o.arrival[j] && w[i].1 <= w[j].1 {
                    prop_assert!(o.completion[i] <= o.completion[j] + 1,
                        "job {i} ({}, {}) finished after job {j} ({}, {})",
                        o.arrival[i], w[i].1, o.arrival[j], w[j].1);
                }
            }
        }
    }

    /// With all jobs arriving together on one core, PS completion times follow the closed form
    /// c_k = c_{k-1} + (n - k) (d_k - d_{k-1}) over the sorted demands.
    #[test]
    fn ps_batch_closed_form(mut d in prop::collection::vec(1u32..5000, 1..40)) {
        let w: Vec<(f64, f64)> = d.iter().map(|&x| (0.0, x as f64 * 1e-3)).collect();
        let o = simulate(&mut ps(1, PsAlgorithm::Exact), &w);
        d.sort();
        let mut c = 0.0;
        let mut prev = 0.0;
        let n = d.len();
        let mut done: Vec<SimTime> = o.completion.clone();
        done.sort();
        for (k, &x) in d.iter().enumerate() {
            let x = x as f64 * 1e-3;
            c += (n - k) as f64 * (x - prev);
            prev = x;
            prop_assert!((done[k] - span(c)).abs() <= 2 + k as i64, "k={k}: {} vs {}", done[k], span(c));
        }
    }

    /// Delay: every job leaves exactly span(demand) after it arrived, never stale.
    #[test]
    fn delay_exact(w in wild_workload(60)) {
        let o = simulate(&mut ActiveResource::new(SchedulingPolicy::Delay, 1, PsAlgorithm::Exact), &w);
        for (i, &(_, d)) in w.iter().enumerate() {
            prop_assert_eq!(o.completion[i] - o.arrival[i], span(d));
        }
        prop_assert_eq!(o.stale, 0);
    }

    /// Virtual-time PS stays close to the exact PS. Usually within a few ns; a rounding
    /// difference can flip the 1e-5 s lost-time decision of an interval, so the bound is 10 us.
    #[test]
    fn virtual_time_close_to_exact(w in grid_workload(80), cores in 1u32..5) {
        let e = simulate(&mut ps(cores, PsAlgorithm::Exact), &w);
        let v = simulate(&mut ps(cores, PsAlgorithm::VirtualTime), &w);
        for i in 0..w.len() {
            prop_assert!((e.completion[i] - v.completion[i]).abs() <= 10_000,
                "job {i}: exact {} vs vt {}", e.completion[i], v.completion[i]);
        }
    }

    /// Passive resource: units are conserved and waiting requests are granted in FIFO order.
    #[test]
    fn passive_fifo_and_conservation(
        cap in 1u64..5,
        ops in prop::collection::vec((0u32..6, 1u64..5, any::<bool>()), 1..200),
    ) {
        #[derive(Default)]
        struct L { granted_after_wait: Vec<u32> }
        impl PassiveListener<u32> for L {
            fn wake(&mut self, job: u32) { self.granted_after_wait.push(job); }
        }
        let mut r = PassiveResource::<u32>::new(cap);
        let mut held = [0u64; 6];
        let mut waiting: std::collections::VecDeque<(u32, u64)> = Default::default();
        let mut l = L::default();
        for (job, n, acquire) in ops {
            let n = n.min(cap);
            let is_waiting = waiting.iter().any(|w| w.0 == job);
            if is_waiting { continue; } // a waiting job is passivated: it cannot act
            if acquire || held[job as usize] == 0 {
                if r.acquire(job, n, &mut l) { held[job as usize] += n; } else { waiting.push_back((job, n)); }
            } else {
                let k = held[job as usize];
                held[job as usize] = 0;
                l.granted_after_wait.clear();
                r.release(job, k, &mut l);
                for g in l.granted_after_wait.drain(..) {
                    let (wj, wn) = waiting.pop_front().unwrap();
                    prop_assert_eq!(wj, g, "grant order is not FIFO");
                    held[wj as usize] += wn;
                }
            }
            let used: u64 = held.iter().sum();
            prop_assert_eq!(r.available(), cap as i64 - used as i64);
            prop_assert_eq!(r.waiting().collect::<Vec<_>>(), waiting.iter().copied().collect::<Vec<_>>());
        }
    }
}

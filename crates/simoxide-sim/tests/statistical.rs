//! Whole-simulation statistical validation, independent of the reference: PCM models of classic
//! queueing systems, run long, compared with closed-form results (mean response time with a
//! batch-means confidence interval, utilisation, throughput).
//!
//! Slow in debug builds, so every test is `#[ignore]`. Run them with
//! `cargo test --release -p simoxide-sim --test statistical -- --ignored --nocapture`
//! (or `crates/simoxide-testkit/scripts/heavy-tests.sh`). About 20 s in release.
//!
//! Every test checks the exact mode and the fast mode (`simoxide_sim::Mode::Fast`) with the
//! same bounds.
//!
//! Models are built with the testkit generator's model types (`simoxide_testkit::modelgen::model`) and
//! written with its XMI writer, so they get the refsim default monitors.

mod common;

use simoxide_sim::{CompiledModel, Mode, Outputs, RunSpec, SimConfig};
use simoxide_testkit::modelgen::model::*;
use simoxide_testkit::modelgen::xmi;
use simoxide_testkit::runcfg::RunConfig;
use simoxide_testkit::stats;
use std::cell::Cell;

// ------------------------------------------------------------------------------ model building

struct Station {
    ty: ResType,
    sched: Sched,
    replicas: u32,
}

enum Step {
    /// Demand on the station of this resource type.
    Demand(ResType, &'static str),
    Acquire,
    Release,
    /// Synchronous fork: one behaviour per inner list.
    Fork(Vec<Vec<Step>>),
}

struct Spec {
    stations: Vec<Station>,
    steps: Vec<Step>,
    workload: Workload,
    passive_capacity: Option<&'static str>,
    max_measurements: i64,
    seed: i64,
}

struct Ids {
    name: String,
    n: usize,
}

impl Ids {
    fn next(&mut self, k: &str) -> String {
        self.n += 1;
        format!("_{}_{k}{}", self.name, self.n)
    }
}

fn behaviour(ids: &mut Ids, steps: &[Step], pr: &str) -> Behaviour {
    let mut actions = Vec::new();
    for s in steps {
        actions.push(match s {
            Step::Demand(t, spec) => Action::Internal {
                id: ids.next("act"),
                name: "work".into(),
                demands: vec![(*t, spec.to_string())],
                infra: vec![],
                rescalls: vec![],
            },
            Step::Acquire => Action::Acquire {
                id: ids.next("act"),
                name: "acquire".into(),
                pr: pr.to_string(),
            },
            Step::Release => Action::Release {
                id: ids.next("act"),
                name: "release".into(),
                pr: pr.to_string(),
            },
            Step::Fork(children) => {
                let bs = children.iter().map(|c| behaviour(ids, c, pr)).collect();
                Action::Fork {
                    id: ids.next("act"),
                    name: "fork".into(),
                    asyncs: vec![],
                    sync: Some((ids.next("sp"), bs)),
                }
            }
        });
    }
    Behaviour {
        id: ids.next("rdb"),
        start_id: ids.next("act"),
        stop_id: ids.next("act"),
        actions,
    }
}

fn build(name: &str, s: &Spec) -> GenModel {
    let mut ids = Ids {
        name: name.to_string(),
        n: 0,
    };
    let res: Vec<ProcRes> = s
        .stations
        .iter()
        .map(|st| ProcRes {
            id: ids.next("prs"),
            ty: st.ty,
            sched: st.sched,
            rate: "1.0".into(),
            rate_mean: 1.0,
            replicas: st.replicas,
            hdd_rates: None,
        })
        .collect();
    let sig = Signature {
        id: ids.next("sig"),
        name: "serve".into(),
        params: vec![],
        returns: false,
    };
    let iface = Interface {
        id: ids.next("if"),
        name: "IService".into(),
        infra: false,
        sigs: vec![sig.clone()],
    };
    let pr_id = ids.next("pr");
    let body = behaviour(&mut ids, &s.steps, &pr_id);
    let prov = Role {
        id: ids.next("prov"),
        name: "Provided_IService".into(),
        iface: 0,
    };
    let comp = BasicComp {
        id: ids.next("comp"),
        name: "Server".into(),
        provides: vec![prov.clone()],
        requires: vec![],
        rreq: None,
        rreq_hdd: None,
        passive: s
            .passive_capacity
            .map(|c| Passive {
                id: pr_id.clone(),
                name: "pool".into(),
                capacity: c.into(),
            })
            .into_iter()
            .collect(),
        comp_params: vec![],
        seffs: vec![Seff {
            id: ids.next("seff"),
            sig: sig.id.clone(),
            body,
        }],
    };
    let asm = Assembly {
        id: ids.next("ac"),
        name: "Assembly_Server".into(),
        comp: 0,
        config: vec![],
    };
    let sys_role = SysRole {
        id: ids.next("prov"),
        name: "Provided_IService".into(),
        iface: 0,
    };
    let conn = Conn::ProvDeleg {
        id: ids.next("conn"),
        outer_role: sys_role.id.clone(),
        ac: asm.id.clone(),
        inner_role: prov.id.clone(),
    };
    let call = UAction::Call {
        id: ids.next("ua"),
        name: "call_serve".into(),
        sys_role: sys_role.id.clone(),
        sig: sig.id.clone(),
        inputs: vec![],
    };
    let scenario = Scenario {
        id: ids.next("us"),
        name: "Scenario0".into(),
        workload: s.workload.clone(),
        body: UBehaviour {
            id: ids.next("sb"),
            start_id: ids.next("ua"),
            stop_id: ids.next("ua"),
            actions: vec![call],
        },
    };
    GenModel {
        name: name.into(),
        containers: vec![Container {
            id: ids.next("rc"),
            name: "Node0".into(),
            res,
        }],
        links: vec![],
        interfaces: vec![iface],
        coll_type: None,
        components: vec![Component::Basic(comp)],
        assemblies: vec![asm.clone()],
        connectors: vec![conn],
        sys_roles: vec![sys_role],
        allocation: vec![(ids.next("alc"), 0, 0)],
        scenarios: vec![scenario],
        run: RunConfig {
            seed: s.seed,
            max_measurements: s.max_measurements,
            max_sim_time: -1,
            ..Default::default()
        },
        windows: vec![],
        asm_op_monitors: vec![],
        triggers: Triggers::Off,
        aggregations: vec![],
        reconf_monitor: None,
        container_count_monitor: false,
        nested: None,
        features: vec![],
    }
}

// ------------------------------------------------------------------------------ running

thread_local! {
    /// The mode `run` uses (set by `both`).
    static MODE: Cell<Mode> = const { Cell::new(Mode::Exact) };
}

/// Runs a test body in the exact and in the fast mode.
fn both(f: impl Fn()) {
    for m in [Mode::Exact, Mode::Fast] {
        MODE.set(m);
        f();
    }
}

struct Result {
    /// Scenario response times (warm-up removed).
    rt: Vec<f64>,
    /// First/last completion time of the kept response times.
    span: (f64, f64),
    /// Per station: (time-weighted mean number of jobs, busy fraction) over replica 0.
    stations: Vec<(f64, f64)>,
}

fn run(name: &str, s: &Spec) -> Result {
    let m = build(name, s);
    let dir = common::scratch(name);
    xmi::write_model(&m, &dir).unwrap();
    let spec = RunSpec::load(&dir.join("run.json")).unwrap();
    let cm = CompiledModel::compile(spec.load_model(&dir).unwrap()).unwrap();
    let cfg = SimConfig {
        store_measurements: true,
        mode: MODE.get(),
        ..spec.sim_config(name)
    };
    let r = simoxide_sim::run(&cm, cfg, Outputs::default()).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let meas = &r.measurements;
    let series = |mp_part: &str, metric: &str| -> Vec<(f64, f64)> {
        meas.series
            .iter()
            .zip(&meas.rows)
            .filter(|(d, _)| d.mp.contains(mp_part) && d.metric == metric)
            .flat_map(|(_, rows)| rows.iter().copied())
            .collect()
    };
    let rt_rows = series("UsageScenarioMeasuringPoint", "Response Time Tuple");
    let warm = rt_rows.len() / 50;
    let kept = &rt_rows[warm..];
    let t0 = kept.first().map(|x| x.0).unwrap_or(0.0);
    let t1 = kept.last().map(|x| x.0).unwrap_or(0.0);
    let stations = m.containers[0]
        .res
        .iter()
        .map(|p| {
            let st = series(
                &format!("{}|replicaID=0", p.id),
                "State of Active Resource Tuple",
            );
            let (ts, vs): (Vec<f64>, Vec<f64>) = st.iter().filter(|x| x.0 >= t0).copied().unzip();
            let busy: Vec<f64> = vs
                .iter()
                .map(|&v| if v > 0.0 { 1.0 } else { 0.0 })
                .collect();
            (
                stats::time_weighted_mean(&ts, &vs, Some(t1)),
                stats::time_weighted_mean(&ts, &busy, Some(t1)),
            )
        })
        .collect();
    Result {
        rt: kept.iter().map(|x| x.1).collect(),
        span: (t0, t1),
        stations,
    }
}

/// Mean response time within a 99.9 % batch-means CI (40 batches) of `expected`, allowing 0.5 %
/// extra for the initial transient.
fn check_rt(label: &str, r: &Result, expected: f64) {
    let (m, hw) = stats::batch_means_ci(&r.rt, 40, 0.999).unwrap();
    let label = format!("[{}] {label}", MODE.get());
    eprintln!(
        "{label}: n={} mean response time {m:.5} ± {hw:.5} (99.9 %), expected {expected:.5} (rel. error {:+.3} %)",
        r.rt.len(),
        100.0 * (m - expected) / expected
    );
    assert!(
        (m - expected).abs() <= hw + 0.005 * expected,
        "{label}: mean {m} ± {hw}, expected {expected}"
    );
}

fn check_close(label: &str, what: &str, got: f64, expected: f64, tol: f64) {
    let label = format!("[{}] {label}", MODE.get());
    eprintln!("{label}: {what} {got:.5}, expected {expected:.5}");
    assert!(
        (got - expected).abs() <= tol,
        "{label}: {what} {got} vs {expected} (tol {tol})"
    );
}

fn throughput(r: &Result) -> f64 {
    r.rt.len() as f64 / (r.span.1 - r.span.0)
}

fn open(rate: f64) -> Workload {
    Workload::Open {
        interarrival: format!("Exp({})", simoxide_testkit::javafmt::to_string(rate)),
    }
}

fn single(
    ty: ResType,
    sched: Sched,
    replicas: u32,
    demand: &'static str,
) -> (Vec<Station>, Vec<Step>) {
    (
        vec![Station {
            ty,
            sched,
            replicas,
        }],
        vec![Step::Demand(ty, demand)],
    )
}

/// Erlang C: probability of waiting in M/M/c with offered load a = λ/μ.
fn erlang_c(c: u32, a: f64) -> f64 {
    let mut sum = 0.0;
    let mut term = 1.0;
    for k in 0..c {
        if k > 0 {
            term *= a / k as f64;
        }
        sum += term;
    }
    let last = term * a / c as f64;
    let rho = a / c as f64;
    let top = last / (1.0 - rho);
    top / (sum + top)
}

/// M/M/c mean response time.
fn mmc_rt(c: u32, lambda: f64, mu: f64) -> f64 {
    let a = lambda / mu;
    erlang_c(c, a) / (c as f64 * mu - lambda) + 1.0 / mu
}

/// Exact MVA for a closed network of queueing stations with think time `z`: returns
/// (response time, throughput).
fn mva(n: u32, demands: &[f64], z: f64) -> (f64, f64) {
    let mut q = vec![0.0; demands.len()];
    let (mut r, mut x) = (0.0, 0.0);
    for k in 1..=n {
        let rk: Vec<f64> = demands.iter().zip(&q).map(|(d, q)| d * (1.0 + q)).collect();
        r = rk.iter().sum();
        x = k as f64 / (z + r);
        q = rk.iter().map(|r| x * r).collect();
    }
    (r, x)
}

// ------------------------------------------------------------------------------ tests

/// Completed users per run (`STAT_N` overrides).
static N: std::sync::LazyLock<i64> = std::sync::LazyLock::new(|| {
    std::env::var("STAT_N")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1_500_000)
});

#[test]
#[ignore = "slow: statistical validation"]
fn mm1_processor_sharing() {
    both(|| {
        let (stations, steps) = single(ResType::Cpu, Sched::Ps, 1, "Exp(1.0)");
        let r = run(
            "st_mm1_ps",
            &Spec {
                stations,
                steps,
                workload: open(0.7),
                passive_capacity: None,
                max_measurements: *N,
                seed: 11,
            },
        );
        check_rt("M/M/1-PS", &r, 1.0 / (1.0 - 0.7));
        check_close("M/M/1-PS", "utilisation", r.stations[0].1, 0.7, 0.01);
        check_close("M/M/1-PS", "mean jobs", r.stations[0].0, 0.7 / 0.3, 0.12);
        check_close("M/M/1-PS", "throughput", throughput(&r), 0.7, 0.01);
    });
}

#[test]
#[ignore = "slow: statistical validation"]
fn mm1_fcfs() {
    both(|| {
        let (stations, steps) = single(ResType::Cpu, Sched::Fcfs, 1, "Exp(1.0)");
        let r = run(
            "st_mm1_fcfs",
            &Spec {
                stations,
                steps,
                workload: open(0.7),
                passive_capacity: None,
                max_measurements: *N,
                seed: 12,
            },
        );
        check_rt("M/M/1-FCFS", &r, 1.0 / (1.0 - 0.7));
        check_close("M/M/1-FCFS", "utilisation", r.stations[0].1, 0.7, 0.01);
        check_close("M/M/1-FCFS", "throughput", throughput(&r), 0.7, 0.01);
    });
}

#[test]
#[ignore = "slow: statistical validation"]
fn md1_fcfs() {
    both(|| {
        // Pollaczek-Khinchine with deterministic service: W = rho / (2 mu (1 - rho))
        let (stations, steps) = single(ResType::Cpu, Sched::Fcfs, 1, "1.0");
        let r = run(
            "st_md1",
            &Spec {
                stations,
                steps,
                workload: open(0.7),
                passive_capacity: None,
                max_measurements: *N,
                seed: 13,
            },
        );
        check_rt("M/D/1-FCFS", &r, 1.0 + 0.7 / (2.0 * 0.3));
    });
}

#[test]
#[ignore = "slow: statistical validation"]
fn mg1_ps_insensitivity() {
    both(|| {
        // M/G/1-PS: E[R] = E[S] / (1 - rho) for any service distribution
        for (k, demand) in ["UniDouble(0.0, 2.0)", "LognormMoments(1.0, 2.0)", "1.0"]
            .into_iter()
            .enumerate()
        {
            let (stations, steps) = single(ResType::Cpu, Sched::Ps, 1, demand);
            let r = run(
                &format!("st_mg1_ps{k}"),
                &Spec {
                    stations,
                    steps,
                    workload: open(0.6),
                    passive_capacity: None,
                    max_measurements: *N,
                    seed: 14 + k as i64,
                },
            );
            check_rt(&format!("M/G/1-PS {demand}"), &r, 1.0 / (1.0 - 0.6));
        }
    });
}

#[test]
#[ignore = "slow: statistical validation"]
fn mmc_ps() {
    both(|| {
        // PS with 3 cores (rate min(1, 3/n) per job) has the birth-death chain of M/M/3, hence its
        // mean response time
        let (stations, steps) = single(ResType::Cpu, Sched::Ps, 3, "Exp(1.0)");
        let r = run(
            "st_mm3_ps",
            &Spec {
                stations,
                steps,
                workload: open(2.4),
                passive_capacity: None,
                max_measurements: *N,
                seed: 21,
            },
        );
        check_rt("M/M/3-PS", &r, mmc_rt(3, 2.4, 1.0));
        check_close("M/M/3-PS", "throughput", throughput(&r), 2.4, 0.03);
    });
}

#[test]
#[ignore = "slow: statistical validation"]
fn fcfs_ignores_replicas() {
    both(|| {
        // Reference quirk (SimFCFSResource serves only the head of one queue; `capacity` is unused):
        // an FCFS resource with 3 replicas is a single server. M/M/1 at rho = 0.7.
        let (stations, steps) = single(ResType::Cpu, Sched::Fcfs, 3, "Exp(1.0)");
        let r = run(
            "st_mm3_fcfs",
            &Spec {
                stations,
                steps,
                workload: open(0.7),
                passive_capacity: None,
                max_measurements: *N,
                seed: 22,
            },
        );
        check_rt("FCFS x3 replicas (= M/M/1)", &r, 1.0 / (1.0 - 0.7));
        check_close(
            "FCFS x3 replicas",
            "utilisation",
            r.stations[0].1,
            0.7,
            0.01,
        );
    });
}

#[test]
#[ignore = "slow: statistical validation"]
fn open_tandem_jackson() {
    both(|| {
        // CPU (PS, mean 0.5) then HDD (FCFS, exponential mean 0.8), lambda = 1:
        // R = 0.5 / (1 - 0.5) + 0.8 / (1 - 0.8) = 5
        let r = run(
            "st_tandem",
            &Spec {
                stations: vec![
                    Station {
                        ty: ResType::Cpu,
                        sched: Sched::Ps,
                        replicas: 1,
                    },
                    Station {
                        ty: ResType::Hdd,
                        sched: Sched::Fcfs,
                        replicas: 1,
                    },
                ],
                steps: vec![
                    Step::Demand(ResType::Cpu, "Exp(2.0)"),
                    Step::Demand(ResType::Hdd, "Exp(1.25)"),
                ],
                workload: open(1.0),
                passive_capacity: None,
                max_measurements: *N,
                seed: 31,
            },
        );
        check_rt("Jackson tandem PS->FCFS", &r, 5.0);
        check_close(
            "Jackson tandem",
            "CPU utilisation",
            r.stations[0].1,
            0.5,
            0.01,
        );
        check_close(
            "Jackson tandem",
            "HDD utilisation",
            r.stations[1].1,
            0.8,
            0.012,
        );
        check_close("Jackson tandem", "HDD mean jobs", r.stations[1].0, 4.0, 0.4);
    });
}

#[test]
#[ignore = "slow: statistical validation"]
fn closed_network_mva() {
    both(|| {
        // N users, exponential think time Z; CPU (PS) and HDD (FCFS, exponential) per cycle
        for (users, z) in [(1u32, 1.0), (5, 1.0), (12, 0.5)] {
            let r = run(
                &format!("st_mva{users}"),
                &Spec {
                    stations: vec![
                        Station {
                            ty: ResType::Cpu,
                            sched: Sched::Ps,
                            replicas: 1,
                        },
                        Station {
                            ty: ResType::Hdd,
                            sched: Sched::Fcfs,
                            replicas: 1,
                        },
                    ],
                    steps: vec![
                        Step::Demand(ResType::Cpu, "Exp(5.0)"),
                        Step::Demand(ResType::Hdd, "Exp(8.0)"),
                    ],
                    workload: Workload::Closed {
                        population: users,
                        think: format!("Exp({})", simoxide_testkit::javafmt::to_string(1.0 / z)),
                    },
                    passive_capacity: None,
                    max_measurements: *N / 2,
                    seed: 40 + users as i64,
                },
            );
            let (rt, x) = mva(users, &[0.2, 0.125], z);
            let label = format!("closed MVA N={users} Z={z}");
            check_rt(&label, &r, rt);
            check_close(&label, "throughput", throughput(&r), x, 0.01 * x);
            check_close(&label, "CPU utilisation", r.stations[0].1, x * 0.2, 0.01);
        }
    });
}

#[test]
#[ignore = "slow: statistical validation"]
fn passive_resource_semaphore_is_mmc() {
    both(|| {
        // acquire (capacity 3) -> delay-resource service Exp(1) -> release: an M/M/3 queue whose
        // waiting happens at the semaphore
        let r = run(
            "st_semaphore",
            &Spec {
                stations: vec![Station {
                    ty: ResType::Delay,
                    sched: Sched::Delay,
                    replicas: 1,
                }],
                steps: vec![
                    Step::Acquire,
                    Step::Demand(ResType::Delay, "Exp(1.0)"),
                    Step::Release,
                ],
                workload: open(2.4),
                passive_capacity: Some("3"),
                max_measurements: *N,
                seed: 51,
            },
        );
        check_rt("semaphore(3) + delay = M/M/3", &r, mmc_rt(3, 2.4, 1.0));
    });
}

#[test]
#[ignore = "slow: statistical validation"]
fn fork_join() {
    both(|| {
        // two synchronous children on an infinite-server (delay) resource: E[max(X1, X2)] = 1.5 / mu
        let delay = Station {
            ty: ResType::Delay,
            sched: Sched::Delay,
            replicas: 1,
        };
        let r = run(
            "st_forkjoin_delay",
            &Spec {
                stations: vec![delay],
                steps: vec![Step::Fork(vec![
                    vec![Step::Demand(ResType::Delay, "Exp(1.0)")],
                    vec![Step::Demand(ResType::Delay, "Exp(1.0)")],
                ])],
                workload: open(0.5),
                passive_capacity: None,
                max_measurements: *N,
                seed: 61,
            },
        );
        check_rt("fork-join on delay (E[max of 2 Exp(1)])", &r, 1.5);

        // two FCFS M/M/1 stations (CPU and HDD, exponential mean 1) forked in parallel, lambda = 0.5:
        // Nelson & Tantawi (1988), exact for two homogeneous servers: R = (12 - rho) / 8 / (mu - lambda)
        let fcfs = |ty| Station {
            ty,
            sched: Sched::Fcfs,
            replicas: 1,
        };
        let r = run(
            "st_forkjoin_fcfs",
            &Spec {
                stations: vec![fcfs(ResType::Cpu), fcfs(ResType::Hdd)],
                steps: vec![Step::Fork(vec![
                    vec![Step::Demand(ResType::Cpu, "Exp(1.0)")],
                    vec![Step::Demand(ResType::Hdd, "Exp(1.0)")],
                ])],
                workload: open(0.5),
                passive_capacity: None,
                max_measurements: *N,
                seed: 62,
            },
        );
        check_rt(
            "fork-join 2 x M/M/1 (Nelson-Tantawi)",
            &r,
            (12.0 - 0.5) / 8.0 / 0.5,
        );
        check_close(
            "fork-join 2 x M/M/1",
            "CPU utilisation",
            r.stations[0].1,
            0.5,
            0.01,
        );
        check_close(
            "fork-join 2 x M/M/1",
            "HDD utilisation",
            r.stations[1].1,
            0.5,
            0.01,
        );
    });
}

//! SimOxide driver of bench/compare: the same `key=value` arguments and `RESULT` / `PAR` lines as
//! the JVM drivers (`bench/compare/java/common/cmp/Harness.java`).
//!
//! `model=DIR simTime=T maxMeas=M seed=S warmup=W runs=R threads=T [mode=reload|batch]`
//!
//! * `mode=reload` (default): every run loads the XMI files from disk, compiles the model,
//!   simulates with stored measurements and computes the summaries, like a JVM driver's run
//!   (which reloads the EMF models every run). With `threads > 1`, worker threads take runs from
//!   a shared counter.
//! * `mode=batch`: load and compile once, then `simoxide_sim::run_batch` (the embedding API's way).

use simoxide_sim::{CompiledModel, RunSpec, SimConfig, Simulation};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

#[derive(Default, Clone)]
struct Res {
    wall_ms: f64,
    sim_ms: f64,
    requests: i64,
    mean_rt: f64,
    sim_end: f64,
    events: i64,
    error: Option<String>,
    /// completion, ms since the start of the measured phase
    t_end_ms: f64,
}

fn load(dir: &Path, a: &HashMap<String, String>, seed_offset: i64) -> Result<(CompiledModel, SimConfig), String> {
    let spec = RunSpec::default();
    let model = spec.load_model(dir)?;
    let cm = CompiledModel::compile(model).map_err(|e| e.to_string())?;
    let mut cfg = spec.sim_config("cmp");
    cfg.seed = a.get("seed").map_or(Ok(1), |s| s.parse()).map_err(|e| format!("seed: {e}"))?;
    cfg.max_sim_time = a.get("simTime").map_or(Ok(-1), |s| s.parse()).map_err(|e| format!("simTime: {e}"))?;
    cfg.max_measurements = a.get("maxMeas").map_or(Ok(-1), |s| s.parse()).map_err(|e| format!("maxMeas: {e}"))?;
    cfg.store_measurements = true;
    if a.get("varySeed").is_some_and(|v| v == "true") {
        cfg.seed += seed_offset;
    }
    Ok((cm, cfg))
}

fn simulate(cm: &CompiledModel, cfg: SimConfig, r: &mut Res) {
    let t = Instant::now();
    match Simulation::new(cm, cfg, Default::default()).and_then(|s| s.run()) {
        Ok(out) => {
            r.sim_ms = t.elapsed().as_secs_f64() * 1e3;
            r.events = out.events as i64;
            r.sim_end = out.end_ns as f64 / 1e9;
            r.requests = 0;
            r.mean_rt = f64::NAN;
            if let Some(s) = out.measurements.summaries().into_iter().find(|s| {
                s.measuring_point.starts_with("UsageScenarioMeasuringPoint")
                    && s.metric.starts_with("Response Time")
            }) {
                r.requests = s.count as i64;
                r.mean_rt = s.mean;
            }
        }
        Err(e) => r.error = Some(e.to_string()),
    }
}

fn one_run(dir: &Path, a: &HashMap<String, String>, id: usize) -> Res {
    let t = Instant::now();
    let mut r = Res::default();
    match load(dir, a, id as i64) {
        Ok((cm, cfg)) => simulate(&cm, cfg, &mut r),
        Err(e) => r.error = Some(e),
    }
    r.wall_ms = t.elapsed().as_secs_f64() * 1e3;
    r
}

fn epoch_ms() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis()
}

fn print(model: &str, phase: &str, i: usize, r: &Res) {
    println!(
        "RESULT sim=simoxide model={model} phase={phase} run={i} wall_ms={:.3} sim_ms={:.3} requests={} mean_rt={:.6} sim_end={:.3} events={} t_end_ms={:.1}{}",
        r.wall_ms,
        r.sim_ms,
        r.requests,
        r.mean_rt,
        r.sim_end,
        r.events,
        r.t_end_ms,
        r.error
            .as_ref()
            .map_or(String::new(), |e| format!(" error={}", e.replace(' ', "_")))
    );
}

fn main() {
    let a: HashMap<String, String> = std::env::args()
        .skip(1)
        .filter_map(|s| s.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
        .collect();
    let num = |k: &str, d: usize| a.get(k).map_or(d, |v| v.parse().expect(k));
    let dir = PathBuf::from(a.get("model").expect("model=DIR"));
    let model = dir.file_name().unwrap().to_string_lossy().into_owned();
    let (warmup, runs, threads) = (num("warmup", 0), num("runs", 1), num("threads", 1));
    let batch = a.get("mode").is_some_and(|m| m == "batch");

    let duration: f64 = a.get("duration").map_or(0.0, |v| v.parse().expect("duration"));
    let sim_name = if batch { "simoxide-batch" } else { "simoxide" };
    for w in 0..warmup {
        print(&model, "warm", w, &one_run(&dir, &a, w));
    }
    let t0 = Instant::now();
    let epoch0 = epoch_ms();
    let deadline = t0 + std::time::Duration::from_secs_f64(duration);
    let mut v: Vec<(usize, Res)> = Vec::new();
    if batch {
        // compile once; with duration=S, repeated run_batch calls of 4 runs per thread until S passed
        let (cm, cfg) = load(&dir, &a, 0).expect("load");
        let vary = a.get("varySeed").is_some_and(|x| x == "true");
        let chunk = if duration > 0.0 { threads.max(1) * 4 } else { runs };
        let mut next = 0usize;
        loop {
            let configs: Vec<SimConfig> = (0..chunk)
                .map(|k| SimConfig { seed: cfg.seed + if vary { (next + k) as i64 } else { 0 }, ..cfg.clone() })
                .collect();
            let res = simoxide_sim::run_batch(&cm, &configs, threads);
            let t_end = t0.elapsed().as_secs_f64() * 1e3;
            for r in res {
                let mut x = Res { wall_ms: -1.0, sim_ms: -1.0, t_end_ms: t_end, ..Res::default() };
                match r {
                    Ok(o) => {
                        x.events = o.events as i64;
                        x.sim_end = o.end_ns as f64 / 1e9;
                        x.mean_rt = f64::NAN;
                        if let Some(s) = o.measurements.summaries().into_iter().find(|s| {
                            s.measuring_point.starts_with("UsageScenarioMeasuringPoint")
                                && s.metric.starts_with("Response Time")
                        }) {
                            x.requests = s.count as i64;
                            x.mean_rt = s.mean;
                        }
                    }
                    Err(e) => x.error = Some(e.to_string()),
                }
                v.push((next, x));
                next += 1;
            }
            if duration <= 0.0 || Instant::now() >= deadline {
                break;
            }
        }
    } else {
        let next = AtomicUsize::new(0);
        let out: Mutex<Vec<(usize, Res)>> = Mutex::new(Vec::new());
        std::thread::scope(|sc| {
            for _ in 0..threads.max(1) {
                sc.spawn(|| {
                    loop {
                        if duration > 0.0 && Instant::now() >= deadline {
                            break;
                        }
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if duration <= 0.0 && i >= runs {
                            break;
                        }
                        let mut r = one_run(&dir, &a, warmup + i);
                        r.t_end_ms = t0.elapsed().as_secs_f64() * 1e3;
                        out.lock().unwrap().push((i, r));
                    }
                });
            }
        });
        v = out.into_inner().unwrap();
    }
    let wall = t0.elapsed().as_secs_f64() * 1e3;
    v.sort_by_key(|x| x.0);
    for (i, r) in &v {
        print(&model, "run", *i, r);
    }
    let n = v.len();
    println!(
        "PAR sim={sim_name} model={model} threads={threads} runs={n} wall_ms={wall:.1} runs_per_s={:.3} end_epoch_ms={} start_epoch_ms={epoch0}",
        n as f64 / (wall / 1e3),
        epoch_ms()
    );
}

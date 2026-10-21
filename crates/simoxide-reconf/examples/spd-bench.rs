//! Benchmark of SPD runs (`DESIGN-SPD.md` §6, phase B1): warm runs of one model with its scaling
//! policies in one process, the time spent in adjustments, and the costs of a fork, a snapshot
//! (serialize, deserialize, restore) and the continuation of a restored run.
//!
//! ```sh
//! cargo run --release -p simoxide-reconf --example spd-bench -- MODEL_DIR \
//!     [flavour=simulizar|slingshot] [mode=fast|exact] [simTime=S] [seed=N] [warmup=W] [runs=R] \
//!     [snapshotAt=S] [quirks=LIST] [spdQuirks=fixed|slingshot|LIST]
//! ```
//!
//! `quirks` are the Slingshot flavour's compat flags (`simoxide run --quirks`, e.g. `fast`), `spdQuirks`
//! the SPD interpreter's (`--spd-quirks`; default `fixed`); `quirks=fast spdQuirks=slingshot` is what
//! stock Slingshot does (`reference/slingshot/spd-stat.sh`).
//!
//! Prints one `SETUP` line (QVTo core and `MainTransformation`, once per process), one `RUN` line
//! per run (`wall_ms` from the simulation's creation to its result, `adjust_ms` inside the
//! adjuster, `adjustments`, `events`, `requests`, `sim_end`) and, with `snapshotAt`, one `SNAP`
//! line per measured run (fork of the run and its QVTo state, snapshot to bytes, bytes to a
//! `Snapshot`, restore, continuation) with a check that the restored run ends like the
//! uninterrupted one.

use simoxide_reconf::{QvtoAdjuster, QvtoCore, SpdAdjust};
use simoxide_sim::snapshot::{Encoding, Snapshot};
use simoxide_sim::{
    Compat, CompiledModel, Exact, Fast, Flavour, FlavourConfig, Mode, Outputs, RunSpec, SimConfig,
    SimError, SimuLizar, Simulation, Slingshot,
};
use simoxide_spd::log::NoLog;
use simoxide_spd::sim::{SpdRun, spd_of};
use simoxide_spd::{Quirks, Spd};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Times the adjustments of the inner adjuster.
struct Timed {
    inner: QvtoAdjuster,
    total: Duration,
    n: u32,
}

impl<C: Compat, F: Flavour> simoxide_spd::sim::Adjuster<C, F> for Timed {
    fn adjust(
        &mut self,
        sim: &mut Simulation<'_, C, F>,
        spd: &Spd,
        state: &mut simoxide_spd::SpdState,
        req: &simoxide_spd::AdjustmentRequest,
    ) -> Result<simoxide_spd::AdjustOutcome, SimError> {
        let t0 = Instant::now();
        let r = self.inner.adjust_on(sim, spd, state, req);
        self.total += t0.elapsed();
        self.n += 1;
        r
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// The mean response time of the first usage scenario and its count.
fn rt(m: &simoxide_sim::Measurements) -> (u64, f64) {
    m.summaries()
        .into_iter()
        .find(|s| {
            s.measuring_point.starts_with("UsageScenarioMeasuringPoint")
                && s.metric.starts_with("Response Time")
        })
        .map_or((0, f64::NAN), |s| (s.count as u64, s.mean))
}

struct Ctx<'a> {
    cm: &'a CompiledModel,
    spd: &'a Spd,
    core: &'a QvtoCore,
    opts: SpdAdjust,
    cfg: SimConfig,
    snapshot_at: Option<i64>,
}

fn adjuster(c: &Ctx) -> Timed {
    Timed {
        inner: QvtoAdjuster::spd(c.core, &c.opts).expect("adjuster"),
        total: Duration::ZERO,
        n: 0,
    }
}

fn one<C: Compat, F: Flavour>(c: &Ctx, phase: &str, i: usize) -> Result<(), String> {
    let e = |e: SimError| e.to_string();
    let t0 = Instant::now();
    let mut adj = adjuster(c);
    let t_adj = t0.elapsed();
    let sim = Simulation::<C, F>::create(c.cm, c.cfg.clone(), Outputs::default()).map_err(e)?;
    let mut r = SpdRun::new(sim, c.spd).map_err(e)?;
    let mut snap_line = None;
    if let Some(at) = c.snapshot_at {
        r.run_until(&mut adj, &mut NoLog, Some(at)).map_err(e)?;
        // fork: the run and its QVTo state, in memory
        let f0 = Instant::now();
        let fork = r.fork(Outputs::default()).map_err(e)?;
        let fadj = adj.inner.fork_spd(c.opts.summaries);
        let t_fork = f0.elapsed();
        drop((fork, fadj));
        // snapshot: the run's sections plus the QVTo state, to bytes
        let s0 = Instant::now();
        let mut s = r.snapshot_value(Encoding::Postcard).map_err(|e| e.to_string())?;
        s.sections.push(
            adj.inner
                .run
                .snapshot_section(Encoding::Postcard)
                .map_err(|e| e.to_string())?,
        );
        let mut bytes = Vec::new();
        s.write(&mut bytes).map_err(|e| e.to_string())?;
        let t_ser = s0.elapsed();
        let d0 = Instant::now();
        let s2 = Snapshot::read(&bytes[..]).map_err(|e| e.to_string())?;
        let t_de = d0.elapsed();
        let r0 = Instant::now();
        let mut r2 = SpdRun::<C, F>::restore_snapshot(&s2, c.cm, c.spd, Outputs::default())
            .map_err(|e| e.to_string())?;
        let mut adj2 = adjuster(c);
        adj2.inner
            .run
            .restore_section(&r2.sim, &s2)
            .map_err(|e| e.to_string())?;
        let t_res = r0.elapsed();
        let c0 = Instant::now();
        r2.run(&mut adj2, &mut NoLog).map_err(e)?;
        let (res2, _, _) = r2.finish().map_err(e)?;
        let t_cont = c0.elapsed();
        snap_line = Some((t_fork, t_ser, bytes.len(), t_de, t_res, t_cont, res2, adj2.n));
    }
    r.run(&mut adj, &mut NoLog).map_err(e)?;
    let (res, _, sum) = r.finish().map_err(e)?;
    let wall = t0.elapsed();
    let (req, mean) = rt(&res.measurements);
    println!(
        "RUN phase={phase} run={i} wall_ms={:.3} adjuster_new_ms={:.3} adjust_ms={:.3} adjustments={} adjustments_ok={} events={} requests={req} mean_rt={mean:.6} sim_end={:.3}",
        ms(wall),
        ms(t_adj),
        ms(adj.total),
        adj.n,
        sum.adjustments_ok,
        res.events,
        res.end_ns as f64 / 1e9
    );
    if let Some((t_fork, t_ser, n, t_de, t_res, t_cont, res2, n2)) = snap_line {
        let (req2, mean2) = rt(&res2.measurements);
        let same = res2.events == res.events
            && res2.end_ns == res.end_ns
            && req2 == req
            && (mean2.to_bits() == mean.to_bits() || (mean.is_nan() && mean2.is_nan()));
        println!(
            "SNAP phase={phase} run={i} fork_us={:.1} serialize_us={:.1} bytes={n} deserialize_us={:.1} restore_us={:.1} continue_ms={:.3} adjustments_after={n2} same_end={same}",
            ms(t_fork) * 1e3,
            ms(t_ser) * 1e3,
            ms(t_de) * 1e3,
            ms(t_res) * 1e3,
            ms(t_cont)
        );
    }
    Ok(())
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = PathBuf::from(args.first().ok_or("usage: spd-bench MODEL_DIR [key=value ...]")?);
    let a: HashMap<&str, &str> = args[1..].iter().filter_map(|s| s.split_once('=')).collect();
    let get = |k: &str, d: &str| a.get(k).copied().unwrap_or(d).to_string();
    let num = |k: &str, d: i64| get(k, &d.to_string()).parse::<i64>().map_err(|e| format!("{k}: {e}"));
    let slingshot = get("flavour", "slingshot") == "slingshot";
    let mode = match get("mode", "fast").as_str() {
        "fast" => Mode::Fast,
        "exact" => Mode::Exact,
        m => return Err(format!("mode {m}")),
    };
    let (warmup, runs) = (num("warmup", 3)? as usize, num("runs", 10)? as usize);
    let snapshot_at = match a.get("snapshotAt") {
        Some(s) => Some((s.parse::<f64>().map_err(|e| e.to_string())? * 1e9) as i64),
        None => None,
    };

    let t0 = Instant::now();
    let model = RunSpec::default().load_model(&dir)?;
    let spd_quirks = match a.get("spdQuirks") {
        Some(q) => Quirks::parse(q)?,
        None => Quirks::fixed(),
    };
    let spd = spd_of(&model, spd_quirks)
        .map_err(|e| e.to_string())?
        .ok_or("the model has no SPD")?;
    let flavour = if slingshot {
        FlavourConfig::slingshot(match a.get("quirks") {
            Some(q) => simoxide_sim::Quirks::parse(q)?,
            None => Default::default(),
        })
    } else {
        FlavourConfig::SIMULIZAR
    };
    let cm = CompiledModel::compile_flavour(model, flavour).map_err(|e| e.to_string())?;
    let t_load = t0.elapsed();
    let t1 = Instant::now();
    let core = QvtoCore::new().map_err(|e| e.to_string())?;
    let opts = SpdAdjust::new(&spd.quirks());
    let first = QvtoAdjuster::spd(&core, &opts).map_err(|e| e.to_string())?;
    let t_qvto = t1.elapsed();
    drop(first);
    let mut cfg = RunSpec::default().sim_config("spd-bench");
    cfg.mode = mode;
    cfg.seed = num("seed", 1)?;
    cfg.max_sim_time = num("simTime", 100)?;
    cfg.max_measurements = -1;
    cfg.store_measurements = true;
    println!(
        "SETUP model={} flavour={} mode={mode} load_compile_ms={:.3} qvto_setup_ms={:.3}",
        dir.file_name().unwrap().to_string_lossy(),
        if slingshot { "slingshot" } else { "simulizar" },
        ms(t_load),
        ms(t_qvto)
    );
    let c = Ctx {
        cm: &cm,
        spd: &spd,
        core: &core,
        opts,
        cfg,
        snapshot_at,
    };
    let go = |phase: &str, i: usize| match (mode, slingshot) {
        (Mode::Fast, false) => one::<Fast, SimuLizar>(&c, phase, i),
        (Mode::Fast, true) => one::<Fast, Slingshot>(&c, phase, i),
        (Mode::Exact, false) => one::<Exact, SimuLizar>(&c, phase, i),
        (Mode::Exact, true) => one::<Exact, Slingshot>(&c, phase, i),
    };
    for i in 0..warmup {
        go("warm", i)?;
    }
    for i in 0..runs {
        go("run", i)?;
    }
    Ok(())
}

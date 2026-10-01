//! `simoxide-bench`: the simoxide-sim benchmark suite (`docs/performance/engine.md`).
//!
//! ```text
//! simoxide-bench suite  [--filter a,b] [--reps N] [--short-secs S] [--mode exact|fast]
//! simoxide-bench batch  [--model DIR] [--max-measurements M] [--runs N] [--threads 1,2,4,8,16]
//!                       [--mode exact|fast]
//! ```
//!
//! The modes draw different random numbers, so their runs do different amounts of work: compare
//! them by runs/s and simulated requests/s (finished usage-scenario runs,
//! `RunResult::main_count`) as well as events/s.
//!
//! `suite` measures, for each benchmark model: XMI load and IR compile time; a short run with the
//! corpus `run.json` (µs per `Simulation::new` + `run`, allocations); a long run (events/s,
//! allocations per event, peak live heap bytes of one simulation) with measurements counted only
//! and with measurements stored. `batch` measures scaling of `run_batch` over threads.
//!
//! The binary counts allocations with a thread-local counting wrapper around the system
//! allocator (or mimalloc with the `mimalloc` feature).

use simoxide_sim::{CompiledModel, Mode, Outputs, RunSpec, SimConfig};
use std::alloc::{GlobalAlloc, Layout};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[cfg(feature = "mimalloc")]
type Inner = mimalloc::MiMalloc;
#[cfg(not(feature = "mimalloc"))]
type Inner = std::alloc::System;

#[cfg(feature = "mimalloc")]
const INNER: Inner = mimalloc::MiMalloc;
#[cfg(not(feature = "mimalloc"))]
const INNER: Inner = std::alloc::System;

thread_local! {
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
    static LIVE: Cell<i64> = const { Cell::new(0) };
    static PEAK: Cell<i64> = const { Cell::new(0) };
}

struct Counting;

#[inline]
fn on_alloc(size: usize) {
    // `try_with`: the thread-local may already be destroyed during thread exit.
    let _ = ALLOCS.try_with(|a| a.set(a.get() + 1));
    let _ = LIVE.try_with(|l| {
        let v = l.get() + size as i64;
        l.set(v);
        let _ = PEAK.try_with(|p| {
            if v > p.get() {
                p.set(v)
            }
        });
    });
}

#[inline]
fn on_free(size: usize) {
    let _ = LIVE.try_with(|l| l.set(l.get() - size as i64));
}

// SAFETY: every method forwards to the inner allocator with the caller's arguments unchanged;
// the wrapper only updates thread-local counters (const-initialised `Cell`s without destructors,
// so accessing them never allocates or recurses into the allocator).
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        on_alloc(l.size());
        // SAFETY: forwarded unchanged (see above).
        unsafe { INNER.alloc(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        on_free(l.size());
        // SAFETY: forwarded unchanged (see above).
        unsafe { INNER.dealloc(p, l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        on_alloc(l.size());
        // SAFETY: forwarded unchanged (see above).
        unsafe { INNER.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        on_free(l.size());
        on_alloc(new);
        // SAFETY: forwarded unchanged (see above).
        unsafe { INNER.realloc(p, l, new) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn allocs() -> u64 {
    ALLOCS.with(|a| a.get())
}
fn live() -> i64 {
    LIVE.with(|a| a.get())
}
fn reset_peak() {
    let l = live();
    PEAK.with(|p| p.set(l));
}
fn peak() -> i64 {
    PEAK.with(|p| p.get())
}

struct Bench {
    name: &'static str,
    dir: &'static str,
    /// Long run: max measurements (max sim time disabled).
    long_meas: i64,
}

const SUITE: &[Bench] = &[
    Bench {
        name: "mediastore",
        dir: "corpus/x_sl_mediastore",
        long_meas: 20_000,
    },
    Bench {
        name: "espresso",
        dir: "corpus/x_espresso",
        long_meas: 20_000,
    },
    Bench {
        name: "h13 passive",
        dir: "corpus/h13_passive_contention",
        long_meas: 20_000,
    },
    Bench {
        name: "fork (x_pem_fork)",
        dir: "corpus/x_pem_fork",
        long_meas: 20_000,
    },
    Bench {
        name: "fork sync (h11)",
        dir: "corpus/h11_fork_sync",
        long_meas: 20_000,
    },
    Bench {
        name: "call chain (h14)",
        dir: "corpus/h14_call_chain_3",
        long_meas: 20_000,
    },
    Bench {
        name: "nested subsystem",
        dir: "corpus/x_pem_subsystem_nested",
        long_meas: 20_000,
    },
    Bench {
        name: "StoEx dists (h21)",
        dir: "corpus/h21_stoex_distributions",
        long_meas: 5_000,
    },
    Bench {
        name: "INNER coll. (h27)",
        dir: "corpus/h27_collection_inner_multi",
        long_meas: 5_000,
    },
    Bench {
        name: "PS 4 cores, ~20 jobs",
        dir: "crates/simoxide-cli/bench/models/ps_many",
        long_meas: 20_000,
    },
    Bench {
        name: "generated s10 (#5)",
        dir: "crates/simoxide-cli/bench/models/gen_s5",
        long_meas: 2_000,
    },
    Bench {
        name: "generated s10 (#6)",
        dir: "crates/simoxide-cli/bench/models/gen_s6",
        long_meas: 100,
    },
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn load(dir: &Path) -> Result<(RunSpec, CompiledModel, f64, f64), String> {
    let spec = RunSpec::load(&dir.join("run.json"))?;
    let t0 = Instant::now();
    let model = spec.load_model(dir)?;
    let t1 = Instant::now();
    let cm = CompiledModel::compile(model).map_err(|e| e.to_string())?;
    let t2 = Instant::now();
    Ok((
        spec,
        cm,
        (t1 - t0).as_secs_f64() * 1e6,
        (t2 - t1).as_secs_f64() * 1e6,
    ))
}

/// Best (minimum) of the repetitions: the least disturbed by other load on the machine.
fn best(v: &mut [f64]) -> f64 {
    v.iter().copied().fold(f64::INFINITY, f64::min)
}

struct Args {
    cmd: String,
    filter: Option<Vec<String>>,
    reps: usize,
    short_secs: f64,
    model: String,
    max_meas: i64,
    runs: usize,
    threads: Vec<usize>,
    mode: Mode,
}

fn parse() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut a = Args {
        cmd: it.next().unwrap_or_default(),
        filter: None,
        reps: 5,
        short_secs: 0.3,
        model: "corpus/x_sl_mediastore".into(),
        max_meas: 20_000,
        runs: 64,
        threads: vec![1, 2, 4, 8, 16],
        mode: Mode::Exact,
    };
    while let Some(f) = it.next() {
        let mut val = || it.next().ok_or(format!("{f} needs a value"));
        match f.as_str() {
            "--filter" => a.filter = Some(val()?.split(',').map(String::from).collect()),
            "--reps" => a.reps = val()?.parse().map_err(|e| format!("{e}"))?,
            "--short-secs" => a.short_secs = val()?.parse().map_err(|e| format!("{e}"))?,
            "--model" => a.model = val()?,
            "--max-measurements" => a.max_meas = val()?.parse().map_err(|e| format!("{e}"))?,
            "--runs" => a.runs = val()?.parse().map_err(|e| format!("{e}"))?,
            "--threads" => {
                a.threads = val()?
                    .split(',')
                    .map(|x| x.parse().map_err(|e| format!("{e}")))
                    .collect::<Result<_, _>>()?
            }
            "--mode" => {
                let v = val()?;
                a.mode = Mode::parse(&v).ok_or(format!("--mode: unknown mode {v}"))?;
                if !a.mode.available() {
                    return Err(format!("--mode {v}: built without the `fast` feature"));
                }
            }
            o => return Err(format!("unknown flag {o}")),
        }
    }
    Ok(a)
}

/// One run: (events, seconds, finished usage-scenario runs).
fn run_once(cm: &CompiledModel, cfg: &SimConfig) -> Result<(u64, f64, i64), String> {
    let t = Instant::now();
    let r = simoxide_sim::run(cm, cfg.clone(), Outputs::default()).map_err(|e| e.to_string())?;
    Ok((r.events, t.elapsed().as_secs_f64(), r.main_count))
}

fn suite(a: &Args) -> Result<(), String> {
    println!("mode {}", a.mode);
    println!(
        "| benchmark | load µs | compile µs | short: events | short µs/run | short allocs | long: events | long ms/run | Mev/s (count) | Mev/s (store) | M requests/s | allocs/event | peak heap KiB |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for b in SUITE {
        if let Some(f) = &a.filter
            && !f
                .iter()
                .any(|x| b.name.contains(x.as_str()) || b.dir.contains(x.as_str()))
        {
            continue;
        }
        let dir = root().join(b.dir);
        // load/compile: best of 5
        let mut lt = Vec::new();
        let mut ct = Vec::new();
        let mut loaded = None;
        for _ in 0..5 {
            let (spec, cm, l, c) = load(&dir)?;
            lt.push(l);
            ct.push(c);
            loaded = Some((spec, cm));
        }
        let (spec, cm) = loaded.expect("loaded");
        let short = SimConfig {
            store_measurements: true,
            mode: a.mode,
            ..spec.sim_config(b.name)
        };
        // short run: repeat for short_secs
        let (short_events, _, _) = run_once(&cm, &short)?;
        let a0 = allocs();
        run_once(&cm, &short)?;
        let short_allocs = allocs() - a0;
        let mut st = Vec::new();
        let t0 = Instant::now();
        while t0.elapsed().as_secs_f64() < a.short_secs || st.len() < 5 {
            st.push(run_once(&cm, &short)?.1 * 1e6);
        }
        // long run
        let long = SimConfig {
            max_sim_time: -1,
            max_measurements: b.long_meas,
            store_measurements: false,
            ..short.clone()
        };
        let long_store = SimConfig {
            store_measurements: true,
            ..long.clone()
        };
        let mut lt_count = Vec::new();
        let mut lt_store = Vec::new();
        let mut events = 0;
        let mut requests = 0;
        for _ in 0..a.reps {
            let (e, t, n) = run_once(&cm, &long)?;
            events = e;
            requests = n;
            lt_count.push(t);
            lt_store.push(run_once(&cm, &long_store)?.1);
        }
        let a0 = allocs();
        reset_peak();
        let base = live();
        run_once(&cm, &long)?;
        let long_allocs = allocs() - a0;
        let pk = peak() - base;
        let tl = best(&mut lt_count);
        let ts = best(&mut lt_store);
        println!(
            "| {} | {:.0} | {:.0} | {} | {:.1} | {} | {} | {:.1} | {:.2} | {:.2} | {:.3} | {:.3} | {:.0} |",
            b.name,
            best(&mut lt),
            best(&mut ct),
            short_events,
            best(&mut st),
            short_allocs,
            events,
            tl * 1e3,
            events as f64 / tl / 1e6,
            events as f64 / ts / 1e6,
            requests as f64 / tl / 1e6,
            long_allocs as f64 / events.max(1) as f64,
            pk as f64 / 1024.0
        );
    }
    Ok(())
}

fn batch(a: &Args) -> Result<(), String> {
    let dir = root().join(&a.model);
    let (spec, cm, _, _) = load(&dir)?;
    let base = SimConfig {
        max_sim_time: -1,
        max_measurements: a.max_meas,
        store_measurements: false,
        mode: a.mode,
        ..spec.sim_config("batch")
    };
    let configs: Vec<SimConfig> = (0..a.runs)
        .map(|i| SimConfig {
            seed: base.seed + i as i64,
            ..base.clone()
        })
        .collect();
    println!(
        "model {} ({} runs, {} measurements, mode {})",
        a.model, a.runs, a.max_meas, a.mode
    );
    println!("| threads | wall s | runs/s | Mev/s | M requests/s | speed-up | efficiency |");
    println!("|---|---|---|---|---|---|---|");
    let mut t1 = None;
    for &t in &a.threads {
        let t0 = Instant::now();
        let res = simoxide_sim::run_batch(&cm, &configs, t);
        let dt = t0.elapsed().as_secs_f64();
        let mut ev = 0u64;
        let mut req = 0i64;
        for r in res {
            let r = r.map_err(|e| e.to_string())?;
            ev += r.events;
            req += r.main_count;
        }
        let base = *t1.get_or_insert(dt);
        println!(
            "| {t} | {dt:.3} | {:.1} | {:.1} | {:.3} | {:.2} | {:.0}% |",
            a.runs as f64 / dt,
            ev as f64 / dt / 1e6,
            req as f64 / dt / 1e6,
            base / dt,
            100.0 * base / dt / t as f64
        );
    }
    Ok(())
}

fn main() -> std::process::ExitCode {
    let r = parse().and_then(|a| match a.cmd.as_str() {
        "suite" => suite(&a),
        "batch" => batch(&a),
        _ => Err("usage: simoxide-bench suite|batch [flags] (see the source header)".into()),
    });
    match r {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

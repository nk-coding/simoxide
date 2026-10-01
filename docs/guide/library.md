# Library API

`simoxide-sim` is the simulator as a library. It has no global mutable state and needs no file
system access, so it can be embedded in a service, for example behind a JVM.

By default the library contains no GPL code: its exact mode uses correctly rounded `Math.log` and
`Math.exp`, which can make it drift from the reference
([Deviations](../correctness/deviations.md#math-log-and-math-exp-without-hotspot-math)). The
feature `hotspot-math` makes it bit-exact but links GPL-2.0-only code, and a program built with it
must not be distributed (see `LICENSES`).

## One run from memory

```rust
use simoxide_sim::{Limits, RunError, RunSpec, simulate_memory};

// (file name, XMI bytes): no file system access; hrefs between the files resolve by name
let files: Vec<(String, Vec<u8>)> = receive_model_files();
let spec = RunSpec { seed: 1, max_measurements: 10_000, ..RunSpec::default() };
let limits = Limits {
    deadline: Some(std::time::Instant::now() + std::time::Duration::from_secs(30)),
    ..Limits::default()
};
match simulate_memory(&files, &spec, limits) {
    Ok(result) => {
        for s in result.measurements.summaries() {
            // measuring point, metric, count, mean, std_dev, min/max, p50/p90/p95/p99,
            // time_weighted_mean (state and utilisation series)
            println!("{} {} n={} mean={}", s.measuring_point, s.metric, s.count, s.mean);
        }
        let _raw = &result.measurements.rows; // raw (time, value) series, per result.measurements.series
        let _csv = result.measurements.to_csv(); // measurements.csv
    }
    Err(RunError::Load(e)) => eprintln!("invalid model: {e}"),
    Err(RunError::Compile(e)) => eprintln!("unsupported model: {e}"),
    Err(RunError::Sim(e)) => eprintln!("{:?}: {e}", e.kind), // Model | Limit | Cancelled
}
```

A runnable version is `crates/simoxide-sim/examples/embed.rs`. `simulate_memory_mode` takes a
[mode](./fast-mode.md) as well.

## Many runs of one model

1. Load the model once: `RunSpec::load_model_memory` or `RunSpec::load_model` (from a directory).
2. Compile it once: `CompiledModel::compile`. A `CompiledModel` is immutable and `Send + Sync`.
3. Run it as often as needed:
   - `run_batch(&cm, &configs, threads)` runs a list of configurations in parallel;
   - `simoxide_sim::run(&cm, cfg, outputs)` runs one configuration in the mode `cfg.mode`;
   - `Simulation::new(&cm, cfg, outputs)?.run()` runs one configuration in exact mode.

Every `SimConfig` carries its own seed, stop conditions, mode and `Limits`. Cancellation is an
`Arc<AtomicBool>` in `Limits::cancel`. The only per-thread state is a set of memos of pure
functions in `simoxide-random` (`log_gamma`, Poisson CDF values), which cannot change results.

`simoxide_model::ParseCache` shares parsed files between loads and threads. It pays off when the
same files are loaded repeatedly, for example variants of one model that differ in one file.

## Outputs

`Outputs` takes optional writers for the trace and the random tape; the measurements are always
collected in `RunResult::measurements`:

- `rows` and `series`: the raw tuples, grouped by measuring point and metric;
- `summaries()`: count, mean, standard deviation, min, max, percentiles and, for state and
  utilisation series, the time-weighted mean;
- `to_csv()`, `write_csv(..)`: `measurements.csv` as the reference writes it.

`SimConfig::store_measurements = false` counts tuples without keeping them.

## Embedding notes

- **Panics.** The workspace release profile sets `panic = "abort"`. For a JNI or FFI library,
  build with `panic = "unwind"` and wrap calls in `catch_unwind`. No panic is known on any input
  (300 000 mutated models and 20 000 random StoEx strings); the wrapper is a second line of
  defence.
- **Stack.** Run simulations on a worker thread with a few MB of stack. The deepest recursion,
  StoEx evaluation, is capped and fits in 1 MB.
- **Memory.** Stored measurements take 16 bytes per tuple and are not bounded by a default limit.
  Bound long runs with `Limits::max_events` or a deadline, or set `store_measurements = false`.
  Everything else a run allocates is small (6 to 130 KiB of live heap on the benchmark models).
- **Loading and compilation** have no deadline. Both are linear in the input size, except the
  linking-resource route table, which is capped at 2 048 containers. Cap the size of accepted
  XMI at the service boundary.
- **File access.** `simoxide_model::load_dir` and `RunSpec::load_model` follow `href`s to any
  readable file. For untrusted input use the memory API, which never reads the file system.
- **Process isolation.** The simplest isolation for a service is to run `simoxide run` as a
  subprocess or container, with `--timeout` and a memory cgroup.

## Crates

| Crate | Contents |
|---|---|
| `simoxide-model` | XMI loader with EMF semantics, typed PCM subset, validation, in-memory loading, `ParseCache` |
| `simoxide-stoex` | StoEx lexer, parser, type inference, PMF/PDF, compiler to a flat program, Java math |
| `simoxide-random` | bit-exact MT19937 and Commons Math distributions, Java `Math.log`/`exp` (correctly rounded, or HotSpot's with feature `hotspot-math`); fast-mode generator and samplers (feature `fast`) |
| `simoxide-hotspot-math` | port of HotSpot's x86-64 `Math.log`/`exp` intrinsics; **GPL-2.0-only**, linked only with the feature `hotspot-math` |
| `simoxide-sched` | processor sharing, FCFS, delay and passive resources on DESMO-J nanosecond time |
| `simoxide-sim` | IR compiler, event core, interpreter, measurements, batch and embedding API, exact and fast mode |
| `simoxide-cli` | the `simoxide` and `simoxide-bench` binaries |
| `simoxide-testkit` | differential testing: formats, diff, reference driver, model generator, `simoxide-fuzz`, exact-vs-fast statistics |

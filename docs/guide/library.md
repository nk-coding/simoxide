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

## Pause, fork and resume

All mutable state of a run is one value, `RunState`; a `Simulation` is that state plus the
compiled model it borrows and the output writers. A run can stop between two events and go on:

```rust
use simoxide_sim::{Outputs, Pause, Simulation};

let mut sim = Simulation::new(&cm, cfg, Outputs::default())?;
sim.run_until(Pause::After(60_000_000_000))?;   // every event up to t = 60 s (ns)
let fork = sim.fork(Outputs::default())?;       // a copy that goes on independently
let state = fork.into_state();                  // or keep the state and resume it later
let resumed = Simulation::resume(&cm, state, Outputs::default())?;
let (a, b) = (sim.finish()?, resumed.finish()?);
```

- `run_until` takes `Pause::After(t)`, `Pause::Before(t)` or `Pause::Events(n)`; `run_while`
  takes a predicate on the `RunState`, asked before each event; `run_watch` takes a `Watch`, a
  set of pause conditions (a time, a measurement trigger, closeness to an SLO;
  [Snapshots](./snapshots.md#pause-conditions)). They return `Status::Paused` or
  `Status::Finished`; `finish` runs to the end and returns the `RunResult`.
- A fork, a paused and resumed run and the uninterrupted run are identical: the same trace,
  tape, measurements and counts, in exact and in fast mode. Nothing is rebased: times stay
  absolute and the random stream, windows and recorders continue where they were.
- The writers given to `fork` and `resume` receive the continuation only. The writers of the
  paused run are flushed when a state is taken, so its output so far plus the continuation is
  the output of the uninterrupted run. A tape can be attached or dropped at any pause; a state
  created with a trace keeps the trace instructions (without a trace writer the lines are
  discarded), and one created without cannot get a trace.
- A fork shares the parameter frames with its origin (copy on write) and copies the rest:
  processes, jobs and the stored measurement rows. `RunState` stays on its thread;
  `RunState::detach()` makes a deep copy (`Detached`, `Send`) for another thread.
  `simoxide-bench fork` measures both.
- A fork has the `Limits` of its origin, including the same cancellation flag;
  `Simulation::set_limits` gives it its own.
- The same state can be written to a file and continued in another process:
  `Simulation::snapshot_to` and `Simulation::restore_from` (feature `snapshot`), see
  [Snapshots](./snapshots.md).

## Changing the model of a run

A paused run (or one not started yet) can continue on a changed model. The run's model graph is
edited, and `Simulation::apply_changes` takes the changed graph with a `ChangeSet` that lists the
changes (the change journal of the edits):

```rust
use simoxide_reconf::Edit;

sim.run_until(Pause::After(60_000_000_000))?;
let mut e = Edit::new(&sim);                    // a copy of the run's graph, journal on
let server = e.find("_rc2").unwrap();
let copy = e.clone_container(server, "_rc3", "Server 2")?;
for link in e.links_of(server) {
    e.connect(link, copy)?;
}
e.allocate(e.find("_ac2").unwrap(), copy, "_alc3")?;
let report = e.apply(&mut sim)?;                // ChangeReport: applied, ignored, new resources
let result = sim.finish()?;
```

- The model graph is the EMF graph SimOxide shares with its QVTo engine (`simoxide_emf::Graph`):
  it is a `qvto_model::ModelStore` with EMF's semantics and records every change, side effects
  included, in its journal. `Edit::store()` gives the graph with the full `ModelStore` API; a QVTo
  transformation run on the graph produces the same kind of journal, and
  `simoxide_reconf::apply_journal` applies it.
- The run continues as SimuLizar 5.2.2 continues after such a change: its syncers' rules, gaps
  included, are on [Model changes](../spec/model-changes.md). Requests in progress keep their
  state. Changes SimuLizar aborts on, and changes not supported yet, return an error and leave the
  run unchanged.
- The changed model belongs to the run (`RunState::model()`): forks share it, and a fork that
  applies a change equals a run that applies it at the same point. `Simulation::resume` takes the
  compiled model the run was created on, also after changes.
- A run without changes does not pay for this: its state holds one empty field.
  `simoxide-bench change` measures the cost of a change: a copy of the graph, a rebuild of the
  typed model, a recompilation and a merge, about 0.3 ms for MediaStore (1 220 objects).

## QVTo transformations on a run

`simoxide-reconf` runs QVTo transformations (SimuLizar's reconfiguration rules, Slingshot's SPD
transformations, your own) on a paused run's model and applies their changes through the same
path:

```rust
use simoxide_reconf::{Imports, Invocation, QvtoCore, QvtoRun, RuleSet, RunConfig};

let core = QvtoCore::new()?;                         // once per process (Send + Sync)
let rule = core.compile_file(Path::new("rules/scaleOut.qvto"), &Imports::Folder)?;
let mut qvto = QvtoRun::new(&core, RunConfig::default())?;   // per run, empty until used

sim.run_until(Pause::After(60_000_000_000))?;
let out = qvto.invoke(&mut sim, &rule, &Invocation::simulizar())?;
println!("{} {}: {}", out.transformation, out.succeeded, out.log);
if let Some(report) = &out.change {               // the ChangeReport of the applied changes
    println!("{:?}", report.applied);
}

let rules = RuleSet::load_dir(&core, Path::new("rules"))?;   // SimuLizar: file-name order
let all = qvto.run_rules(&mut sim, &rules, &Invocation::simulizar()).map_err(|(_, e)| e)?;
```

- **Core and run state.** `QvtoCore` holds what never changes: the metamodel registry,
  metamodel objects, and the metamodels SimuLizar and Slingshot register besides SimOxide's
  (runtime measurements, feature model and configuration, completions); compile each
  transformation once with it and share both between runs and threads. `QvtoRun` is a run's QVTo
  state, created on first use: the run's model graph gets the core's metamodel objects (a copy
  of the core's prototype, about 1 ms) and its id generator. Models of other metamodels
  (`QvtoRun::load_model`, `create_model`: a runtime measurement model, ...) are resources of the
  same graph and live with the run's model (`Simulation::set_model_graph` when only they
  changed). Clone `QvtoRun` with the run's fork (copy on write): new objects then get the same
  ids in both.
- **Bindings.** By default every model parameter is bound as SimuLizar does: the models whose
  metamodel is the parameter's (`Bind::ByNsUri`); `Invocation::slingshot()` binds as Slingshot
  does; `Invocation::bind(name, Bind::Models(..))` names models (graph resources, or models of
  other metamodels loaded with `QvtoRun::load_model`/`create_model`). Configuration properties:
  `Invocation::config("keepModeling", "true")`.
- **Outcome.** A transformation that fails (`assert fatal`, an exception) is not an error:
  `Outcome::succeeded` is false ("not applied"), and what it changed before it failed is
  applied, as SimuLizar and Slingshot do (`RunConfig::failed_runs` drops it instead). Changes the
  run cannot take return `ReconfError::Rejected` with the run unchanged; changes SimuLizar aborts
  on fail the run (`ReconfError::RunFailed`; `RunConfig::on_abort = AbortPolicy::Reject` keeps
  the run going instead).
- **Natives.** `getSimulationTime()` is the run's clock. Palladio's `ProfilesLibrary`
  (stereotypes) is a stub that throws.
- **Modes.** The default build runs QVTo's fast mode: Eclipse's semantics, with sets and
  `->any` in a deterministic insertion order instead of Java's hash order. A build with the
  feature `compat` (or `exact`) runs Eclipse's compat mode, whose models equal Eclipse QVTo's;
  `QvtMode::Compat` in other builds is `ReconfError::ModeUnavailable`.
- **SPD.** `QvtoAdjuster` (feature `spd`) executes the SPD interpreter's adjustment requests
  with a transformation such as Slingshot's `MainTransformation.qvto` ([SPD](spd.md)).
- **Flavours.** The host works with both flavours; the Slingshot flavour does not take model
  changes yet (`ChangeError::Unsupported`).
- `dry_run` runs a transformation on a scratch copy, `with_scratch_store` reads one, and
  `with_store` edits the run's models with any `ModelStore` operation (all on the graph:
  `&mut simoxide_model::raw::Graph`, objects as `ObjId`). Outside a simulation,
  `QvtoCore::attach` gives a loaded model graph the core's QVTo state and
  `QvtoCore::load_model` loads other models into it.
- **SimuLizar's reconfiguration process.** `RulesRun` runs a simulation whose reconfiguration
  process executes a `RuleSet` with SimuLizar's timing, runtime measurement values and result
  ([Reconfiguration rules](reconfiguration.md#library)).

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
| `simoxide-spd` | scaling policy definitions: Slingshot's filter-chain interpreter and its driver |
| `simoxide-reconf` | runtime model changes: direct edits, the QVTo host (transformations, SimuLizar rule sets, SPD adjustments); QVTo directly on the model graph |
| `simoxide-cli` | the `simoxide` and `simoxide-bench` binaries |
| `simoxide-testkit` | differential testing: formats, diff, reference driver, model generator, `simoxide-fuzz`, exact-vs-fast statistics |
| `simoxide-oraclekit` | the reference harnesses' shared driver: warm-JVM batch processes, the seed agent, scratch and result directories, comparisons (test infrastructure, not published) |
| `simoxide-jcompat`, `simoxide-jcompat-exact` | Java behaviour (hash codes, `Double.toString`, parsing, insertion order); `-exact` (opt-in, no licence granted): `HashMap` order and HotSpot's identity hashes ([Foundation crates](../development/foundation.md)) |
| `simoxide-emf-core`, `simoxide-emf` | EMF's metamodel registry and the model graph shared by SimOxide and QVTo: XMI, resource sets, change journal ([Foundation crates](../development/foundation.md)) |
| `qvto`, `qvto-syntax`, `qvto-types`, `qvto-sema`, `qvto-bytecode`, `qvto-runtime`, `qvto-vm` | the QVTo engine: the facade `qvto` and its pipeline ([QVTo architecture](../qvto/architecture.md)) |
| `qvto-testkit`, `qvto-cli`, `qvto-paths` | QVTo's conformance harness, the `qvto` command, data paths |

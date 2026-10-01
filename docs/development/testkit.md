# Test kit

`crates/simoxide-testkit` holds everything needed to compare a simulator with the Java reference. Only
the statistical exact-vs-fast harness (`equiv`) depends on `simoxide-sim` (with its `fast`
feature); everything else drives simulators through the `Simulator` trait.

## Writing the three output files (simulator side)

| Need | API |
|---|---|
| `Double.toString` of Java ≥ 19 | `javafmt::write(&mut Vec<u8>, f64)`, `javafmt::to_string`; `write_json` quotes NaN/±Infinity |
| `trace.jsonl` | `trace::TraceWriter<W: Write>`: one typed method per event (`header`, `spawn`, `element`, `system_op`, `assembly_op`, `hold`, `branch`, `loop_`, `infra`, `fork`, `join`, `demand`, `demand_done`, `acquire`, `grant`, `release`, `meas`, `stop`, `finish_event`), plus `event(ev, t)` for anything else. Internal 64 KiB buffer, no allocation per event |
| `t` | `trace::t_from_ns(ns)` = `ns as f64 / 1e9` |
| `tape.jsonl` | `tape::TapeWriter`: `uniform(u, origin)`, `sample(n, origin, spec, &SampleValue)` (truncates `spec` like the reference) |
| `measurements.csv` | `measurements::Recorder`: `series_id(mp, metric)` once, `record(id, time, value)`, then `into_measurements().to_csv()` (Java `String` order of `mp + "\0" + metric`) |

All writers reproduce every line of every `corpus/*/expected` file byte for byte
(`tests/formats.rs`).

## Tape replay

`tape::Tape::parse(text)` gives `uniforms`, per-uniform `origins` (interned), and `samples` (derived
values with the index of their first uniform). `tape.replay()` is a cursor:

- `next_uniform()` has no check.
- `next_checked(origin)` and `next_checked_parts(purpose, Some(id))` fail with
  `ReplayError::OriginMismatch { index, expected, actual }` at the first draw the port makes in a
  different order or place.
- `last_sample()` is the `s` record that ends at the cursor, for checking a derived value.

`tape::first_origin_mismatch(expected, actual)` and `tape::first_mismatch(..)` compare whole tapes.

## Comparing

- **Traces.** `diff::diff_traces` and `diff_trace_text` report the first divergence: preceding
  lines, differing fields, time, process, the process's spawn, and its open `begin` elements. Two
  consecutive extra or missing lines are classified as `Extra`/`Missing` after re-synchronising.
  Modes:
  - `Exact`;
  - `Tolerance{rel, abs}`;
  - `MeasurementsOnly`, which compares only `meas` events;
  - `PerProcess`, which compares each process's event sequence on its own and so tolerates a
    different interleaving.
- **Measurements.** `meascmp::compare_measurements` has three modes: `Exact`, `Tolerance`, and
  `Statistical(StatOptions)`.
  - In the statistical mode a sample series fails only if the KS test rejects at `alpha` *and* the
    batch-means CIs do not overlap. Percentiles are reported.
  - State and utilisation series are compared by their time-weighted mean.
- **CLI.** `simoxide-fuzz diff expected.jsonl[.gz] actual.jsonl [--compare exact|tol:1e-9|meas:0|proc:0]`

## Driving a simulator

`sim::Simulator` has `run(&RunRequest) -> Result<RunOutput, SimError>` and an optional
`run_batch`.

- `RunRequest` holds `model_dir`, the parsed `run.json` (`RunConfig`), and `rng`
  (`RngMode::OwnRng` or `RngMode::TapeReplay(Arc<Tape>)`).
- Implementations:
  - `RefSim` runs `reference/refsim batch` in one warm JVM per batch. It stages the models with
    symlinks, applies a timeout per model, and reruns what is left after a hang.
    A batch whose stderr shows a failed thread creation or an `OutOfMemoryError` counts as
    failed: SimuLizar can then end a run early and still report `ok`.
  - `CmdSim` wraps any CLI through placeholders (`{dir} {run_json} {trace} {tape} {measurements}
    {mode} {tape_in} {seed} {name}`). `{replay:FLAG}` expands to `FLAG <tape>` in replay mode
    and to nothing otherwise. Exit code 3 means unsupported.
  - `FnSim` wraps a closure and catches panics.
  - `Unimplemented` is the stub.

## Corpus harness (for `simoxide-sim` tests)

```rust
let mut sim = simoxide_testkit::sim::FnSim::new("simoxide-sim", |req| { /* run, return RunOutput */ });
simoxide_testkit::corpus::run_corpus(&mut sim, &simoxide_testkit::corpus::HarnessOptions::from_env()).assert_all_pass();
```

- Every `corpus/*` entry runs in `replay` and `own-rng` mode and is compared exactly with
  `expected/`. The files may be `.gz`.
- The result prints as a table with one row per model and a column per mode, followed by the first
  failure in full.
- Environment:
  - `TESTKIT_FILTER=h0,x_pem`
  - `TESTKIT_MODES=replay|own`
  - `TESTKIT_DUMP=dir`: write the actual outputs of failing cases
  - `TESTKIT_STRICT=1`: fail on `Unimplemented`
- CLI: `simoxide-fuzz corpus --sim cmd:'...'`.

## Model generator and fuzzer

`modelgen::generate(&GenConfig { name, seed, size 1..=10, features })` is a pure function.
`modelgen::xmi::write_model(&m, dir)` writes:
- `.repository`, `.system`, `.resourceenvironment`, `.allocation`, `.usagemodel`;
- `.measuringpoint` and `.monitorrepository`, the refsim default monitors, like `Monitors.java`;
- `run.json` and `FEATURES.txt`.

`Features` has one probability per feature, named in `FEATURE_NAMES`. The fuzzer switches features
off with `--disable`, and the minimizer does the same.

```
simoxide-fuzz validate --n 300            # reference pass rate, crash categories, determinism (2 JVMs)
simoxide-fuzz fuzz --n 200 --modes replay,own \
  --sim "cmd:$PWD/target/release/simoxide run --model {dir} --run-json {run_json} --name {name} \
         --trace {trace} --tape {tape} --measurements {measurements} {replay:--replay-tape}"
simoxide-fuzz corpus --filter h0 --sim "cmd:..."   # same harness as the simoxide-sim tests, from the shell
simoxide-fuzz gen /tmp/m1 --seed 7 --size 6
```

When the candidate diverges, the fuzzer saves the case as a corpus entry `corpus-fuzz/<name>/`. It
holds the model, `run.json`, `expected/` (the reference outputs), `gen.json` and
`REPORT.<mode>.txt`.

The smallest diverging variant is saved next to it as `corpus-fuzz/<name>_m<round>_<k>/`. The
minimizer is greedy: it lowers `size` and switches off halves, quarters and single features, with
one reference batch per round.

`simoxide-fuzz corpus --corpus corpus-fuzz` replays saved cases.

## Statistical equivalence of two modes (`equiv`)

`equiv` runs models in two `simoxide_sim::Mode`s (default exact and fast) with many seeds, in
process and in parallel, and tests per measuring point and metric whether the results have the
same distribution ([Testing](../correctness/testing.md#fast-mode) describes the tests).

```rust
use simoxide_testkit::equiv::{EquivOptions, Model, Report};
let o = EquivOptions { seeds: 40, ..EquivOptions::default() };
let mut rep = Report { modes: o.modes.to_vec(), ..Report::default() };
rep.add_model(&Model::from_dir(dir)?, &o);                 // or Model::generated(&gen_config)?
rep.finish(0.01);                                           // Holm over all tests
assert!(rep.failures().is_empty() && rep.deterministic_differences().is_empty(), "{}", rep.text(20));
```

- `summarize(&RunResult, k)` reduces a run to per-series statistics (count, mean, p50/p90/p99,
  time-weighted mean, a systematic sample of `k` values; state-like series over time).
- `compare` gives the tests of one model; `Report::tests_csv` writes all of them (with the
  generator features of each model).
- `stats`: `welch_t_test`, `mann_whitney_u`, `fisher_exact`, `holm`, `normal_two_sided_p`.
- CLI: `simoxide-fuzz equiv [--corpus DIRS] [--gen N --seed S --sizes a..b --set ..] [--modes a,b]
  [--seeds 40] [--seed-offset-b N] [--min-measurements M] [--alpha 0.01] [--tests-csv FILE]`;
  exit code 1 on a Holm-corrected failure. `--modes exact,exact --seed-offset-b 100000` is the
  control that shows how the tests behave without any difference.

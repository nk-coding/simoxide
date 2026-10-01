# Command line

The workspace builds three binaries:

| Binary | Crate | Purpose |
|---|---|---|
| `simoxide` | `simoxide-cli` | run a model, benchmark it |
| `simoxide-bench` | `simoxide-cli` | the benchmark suite |
| `simoxide-fuzz` | `simoxide-testkit` | differential testing against the reference |

## `simoxide run`

Runs one simulation. The flags mirror `reference/refsim run`.

```text
simoxide run --model <dir> [flags]
```

| Flag | Meaning |
|---|---|
| `--model DIR` | model directory |
| `--run-json FILE` | run configuration (default: `DIR/run.json` if present) |
| `--seed N` | seed |
| `--max-sim-time T` | stop at simulated time `T` seconds (`-1` = off) |
| `--max-measurements M` | stop after `M` finished usage-scenario runs (`-1` = off) |
| `--no-link-throughput` | do not simulate the throughput of linking resources |
| `--measurements FILE` | write `measurements.csv` |
| `--trace FILE` | write the event trace |
| `--tape FILE` | write the random tape (exact mode only) |
| `--replay-tape FILE` | take the uniforms from a recorded tape instead of the generator |
| `--check-origins` | with `--replay-tape`: fail at the first draw made at a different place |
| `--name NAME` | run name in the trace header |
| `--mode exact\|fast` | simulator mode, default `exact` ([Fast mode](./fast-mode.md)) |
| `--ps-algorithm exact\|virtual-time` | processor-sharing algorithm, default `exact` ([Deviations](../correctness/deviations.md#virtual-time-processor-sharing)) |

Limits (see [Deviations](../correctness/deviations.md#resource-limits-for-untrusted-models)):

| Flag | Default |
|---|---|
| `--max-events-per-instant N` | 20 000 000 (livelock guard, `0` = off) |
| `--max-stack-depth N` | 10 000 continuations (about 2 000 nested calls) |
| `--max-processes N` | 1 000 000 |
| `--max-events N`, `--max-steps N` | off |
| `--timeout SECONDS` | off |

On success, `run` prints a summary line on stderr (`t_end`, `events`, `uniforms`,
`measurements`, `main_count`, wall time) and exits with 0. A load, compile or simulation error
exits with 1 (`error: ...`), a usage error with 2. Compile warnings, for example about monitors
that cannot be recorded, are printed as `warning: ...`.

## `simoxide bench`

```text
simoxide bench --model <dir> [--runs N] [--threads T] [run flags]
```

Runs the model `N` times (default 20) on `T` threads and prints events/s and runs/s. With the
`profile` cargo feature, `--profile out.svg` or `--profile out.folded` records a sampling profile.

## `simoxide load-bench`

```text
simoxide load-bench --model <dir> [--runs N]
```

Measures XMI loading plus compilation only.

## `simoxide-bench`

```text
simoxide-bench suite [--filter a,b] [--reps N] [--mode exact|fast]
simoxide-bench batch [--model DIR] [--max-measurements M] [--runs N] [--threads 1,2,4,8] [--mode exact|fast]
```

`suite` runs the benchmark models (load, compile, a short and a long run, allocations, peak heap);
`batch` measures thread scaling. See [Benchmarking](../development/benchmarking.md).

## `simoxide-fuzz`

| Command | Purpose |
|---|---|
| `fuzz` | generate models, run them in the reference and in a candidate, compare exactly |
| `validate` | reference pass rate and determinism on generated models |
| `corpus` | run a corpus directory against a candidate |
| `long` | long runs, measurements only |
| `equiv` | exact vs fast mode, statistically |
| `gen` | write one generated model |
| `diff` | first divergence of two traces |

See [Testing](../correctness/testing.md) and [Test kit](../development/testkit.md).

## Cargo features

| Crate | Feature | Effect |
|---|---|---|
| `simoxide-cli` | `fast` (default) | include the fast mode; `--no-default-features` builds an exact-only binary |
| `simoxide-cli` | `profile` | `--profile` on `bench` and `load-bench` |
| `simoxide-cli` | `mimalloc` | mimalloc as the global allocator |
| `simoxide-sim`, `simoxide-random` | `fast` (off) | the fast mode in the libraries |
| `simoxide-random` | `rcp-table` | a recorded AMD Zen 5 `rcpss` table instead of the instruction ([Exactness](../correctness/#caveats)) |
| `simoxide-random` | `fast-bracket` | galloping bracket search for inverse CDFs (not proven exact) |

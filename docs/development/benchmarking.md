# Benchmarking

## Tools

| Command | What it measures |
|---|---|
| `cargo run --release -p simoxide-cli --bin simoxide-bench -- suite [--filter a,b] [--reps N] [--mode exact\|fast]` | per benchmark model: XMI load and compile time, a short run (µs per run, allocations), a long run (events/s with measurements counted and stored, requests/s, allocations per event, peak live heap). Best of `--reps` |
| `simoxide-bench batch --model DIR --threads 1,2,4,8 --runs N [--mode exact\|fast]` | `run_batch` scaling over threads |
| `simoxide bench --model DIR --max-measurements M --runs N [--threads T]` | events/s of one model with any run flags |
| `simoxide load-bench --model DIR --runs N` | XMI load plus compile |
| `cargo run --release -p simoxide-model --example load-bench -- bench [--reps N] [--phases] [--memory] [--cache] [dirs]` | loading only, per phase, from disk, memory or cache |
| `cargo run --release -p simoxide-random --example bench` | per-sample cost of each distribution |
| `--profile out.svg` or `--profile out.folded` on `bench` and `load-bench` | in-process sampling profiler (pprof at 5 kHz); needs `--features profile`. `.folded` writes one line per stack, frames as `name@file:line` |
| `bench/compare/run-all.sh` | the comparison with the other Palladio simulators ([Comparison in detail](../performance/comparison.md#reproducing)) |

The benchmark models are listed in `crates/simoxide-cli/src/bin/simoxide-bench.rs`. Three of them
are in `crates/simoxide-cli/bench/models/` (`ps_many`, `gen_s5`, `gen_s6`), the rest in `corpus/`.
`simoxide-bench` counts allocations with a wrapper around the system allocator;
`--features mimalloc` switches `simoxide` and `simoxide-bench` to mimalloc.

The two modes draw different random numbers, so their runs do slightly different work: compare
them by runs/s or requests/s, not events/s.

## A/B comparisons

- Keep the old binary and run old and new alternately, several times, pinned to one core
  (`taskset -c N`), and take medians.
- Check that the machine is quiet (`uptime`). Other jobs easily add 10 % noise; treat differences
  under about 3 % as noise.
- Check that outputs are unchanged ([guardrails](./contributing.md#guardrails)) before measuring
  speed.

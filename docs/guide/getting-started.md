# Getting started

## Build

SimOxide needs Rust stable 1.88 or later (edition 2024). It is tested with 1.98.1.

```sh
cargo build --release
```

This builds the `simoxide` command line tool into `target/release/`, with both the exact and the
[fast mode](./fast-mode.md). The release profile uses fat LTO and `panic = "abort"`.

This is the **exact build**. Its feature `hotspot-math` makes `Math.log` and `Math.exp`
bit-identical to HotSpot by linking a port of OpenJDK code licensed under GPL-2.0-only. That
licence is incompatible with the EPL-2.0 and Apache-2.0 code in the rest of SimOxide, so the
binary is for your own use and must not be distributed.

The **EPL build** leaves the feature out and can be distributed:

```sh
cargo build --release -p simoxide-cli --no-default-features --features fast
```

Use it in [fast mode](./fast-mode.md). Its exact mode uses SimOxide's own correctly rounded `log`
and `exp`, which differ from HotSpot by one ulp on about 0.25 % of `exp` and 1e-5 of `log`
arguments, so it matches the reference only until the first such draw (see
[Deviations](../correctness/deviations.md#math-log-and-math-exp-without-hotspot-math)).

## Run a model

A model is a directory with the PCM files of one system: repository, system, resource
environment, allocation, usage model and, optionally, a monitor repository. An optional
`run.json` holds the seed and the stop conditions.

```sh
# a corpus model, with its run.json
./target/release/simoxide run --model corpus/x_sl_mediastore --measurements m.csv

# explicit seed and stop condition, all three output files
./target/release/simoxide run --model DIR --seed 7 --max-measurements 10000 \
    --trace trace.jsonl --tape tape.jsonl --measurements m.csv
```

The run prints a summary line on stderr (end time, events, draws, measurements). The outputs are:

| File | Content |
|---|---|
| `measurements.csv` | every recorded measurement tuple, one row per tuple |
| `trace.jsonl` | the event trace, one JSON object per event |
| `tape.jsonl` | the random tape: every uniform draw and its origin |

All three are described in [Output formats](./formats.md). In exact mode they are byte-identical
to what the patched reference writes for the same model and seed.

## `run.json`

```json
{ "seed": 1, "max_measurements": 1000, "max_sim_time": -1,
  "simulate_linking_resources": false, "simulate_throughput_of_linking_resources": true }
```

- `max_measurements` counts finished usage-scenario runs; `max_sim_time` is in simulated
  seconds. `-1` turns a condition off.
- The two network flags are the SimuLizar configuration keys, with the SimuLizar UI defaults.
- Optional keys `usagemodel`, `allocation` and `monitorrepository` name the entry files.
  Without them the directory must contain exactly one `.usagemodel`, at least one
  `.allocation` and at most one `.monitorrepository`.

Command line flags override the values from `run.json`. See
[Output formats §5](./formats.md#_5-run-configuration-run-json) for details.

## Untrusted models

SimOxide is meant to run inside services on generated or user-supplied models. Every input ends
in a result or an error, never in a crash. Bound the run time and size explicitly:

```sh
./target/release/simoxide run --model DIR --timeout 10 --max-events 50000000
```

The limits are listed in [Deviations](../correctness/deviations.md#resource-limits-for-untrusted-models).

## Next steps

- [Command line](./cli.md): all commands and flags.
- [Library API](./library.md): embed SimOxide in another program.
- [Supported models](./scope.md): what SimOxide simulates and what it rejects.

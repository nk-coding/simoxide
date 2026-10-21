# Getting started

## Get the source

```sh
git clone https://github.com/nk-coding/simoxide
cd simoxide
```

Building needs nothing else. The tests, the Slingshot example models and the SPD examples also
need three git submodules with third-party models that SimOxide does not distribute:

```sh
git submodule update --init                       # or clone with --recurse-submodules
cargo run -q -p simoxide-thirdparty -- setup      # derive SimOxide's modified files from them
```

See [Licensing](./licensing.md#third-party-test-models). Without them, `corpus/x_pem_*`,
`corpus/x_ss_*` and the SPD examples under `reference/slingshot/models/` are dangling links; every
other model directory works.

## Build

SimOxide needs Rust stable 1.88 or later (edition 2024). It is tested with 1.98.1. The release
profile uses fat LTO and `panic = "abort"`.

```sh
# the default, distributable EPL build: fast mode, Slingshot flavours, SPD/HPA decisions
cargo build --release

# plus the QVTo engine: reconfiguration rules, SPD and HPA adjustments, resume with changes
cargo build --release -p simoxide-cli --features reconfiguration

# the exact build (local use only, see Licensing): adds --mode exact
cargo build --release -p simoxide-cli --features exact,hotspot-math,reconfiguration
```

`cargo build-exact` is an alias for the exact build without `reconfiguration` (it also builds
`simoxide-fuzz`). What each build contains and why the exact build must not be distributed:
[Modes and features](./modes.md#builds) and [Licensing](./licensing.md). A build without `exact`
answers `--mode exact` with an error.

With `exact` but without `hotspot-math`, the exact mode uses correctly rounded `log` and `exp`,
which differ from HotSpot by one ulp on about 0.25 % of `exp` and 1e-5 of `log` arguments, so it
matches the reference only until the first such draw
([Deviations](../correctness/deviations.md#math-log-and-math-exp-without-hotspot-math)).

## Run a model

A model is a directory with the PCM files of one system: repository, system, resource
environment, allocation, usage model and, optionally, a monitor repository. An optional
`run.json` holds the seed and the stop conditions.

```sh
# SimuLizar semantics, fast mode (the default); the model's run.json sets seed and stop
./target/release/simoxide run --model corpus/x_sl_mediastore --measurements m.csv

# exact mode (exact build): byte-identical to SimuLizar 5.2.2, with trace and random tape
./target/release/simoxide run --mode exact --model corpus/x_sl_mediastore --seed 7 \
    --max-measurements 10000 --trace trace.jsonl --tape tape.jsonl --measurements m.csv

# Slingshot semantics; Slingshot's only stop condition is the simulated time
./target/release/simoxide run --model corpus/x_sl_loadbalancer --flavour slingshot \
    --max-sim-time 1000 --max-measurements -1 --measurements m.csv

# Slingshot's event bus, event for event (every build)
./target/release/simoxide run --model corpus/x_sl_loadbalancer --flavour slingshot-exact \
    --seed 1 --max-sim-time 1000 --measurements m.csv
```

The run prints a summary line on stderr (end time, events, draws, measurements). The outputs are:

| File | Content |
|---|---|
| `measurements.csv` | every recorded measurement tuple, one row per tuple |
| `trace.jsonl` | the event trace, one JSON object per event (`slingshot-exact`: the level-E event projection) |
| `tape.jsonl` | the random tape: every uniform draw and its origin (exact modes only) |

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
  `.allocation` and at most one `.monitorrepository`. `reconfiguration_rules` names a folder of
  SimuLizar reconfiguration rules.

Command line flags override the values from `run.json`. See
[Output formats §5](./formats.md#_5-run-configuration-run-json) for details.

## Files SimOxide picks up next to the model

| File | Effect | Off with |
|---|---|---|
| `*.spd`, `*.semanticspd` | Slingshot scaling policies ([Scaling policies](./spd.md)) | `--no-spd` |
| `*.hpa.json`, `*.hpa` | Kubernetes HPA autoscalers ([Autoscaling (HPA)](./hpa.md)) | `--no-hpa` |
| `*.loadprofile.json` | a time-varying load ([Load profiles](./load-profile.md)) | `--no-load-profile` |

## A few more things to try

```sh
# SimuLizar's load-balancer example with its reconfiguration rule (build with reconfiguration)
./target/release/simoxide run --model corpus-rules/lb_outsource --rules-log rules.jsonl \
    --measurements m.csv

# pause at t = 120 s, write a snapshot, continue it later
./target/release/simoxide run --model corpus/x_sl_mediastore --max-measurements -1 \
    --max-sim-time 600 --pause-at 120 --snapshot ms.snap
./target/release/simoxide resume ms.snap --model corpus/x_sl_mediastore --measurements m.csv

# many runs in parallel
./target/release/simoxide bench --model corpus/x_sl_mediastore --runs 20 --threads 8
```

## Untrusted models

SimOxide is meant to run inside services on generated or user-supplied models. Every input ends
in a result or an error, never in a crash. Bound the run time and size explicitly:

```sh
./target/release/simoxide run --model DIR --timeout 10 --max-events 50000000
```

The limits are listed in [Deviations](../correctness/deviations.md#resource-limits-for-untrusted-models).

## Next steps

- [Modes and features](./modes.md): which mode for which purpose, what works where.
- [Command line](./cli.md): all commands and flags.
- [Library API](./library.md): embed SimOxide in another program.
- [Supported models](./scope.md): what SimOxide simulates and what it rejects.

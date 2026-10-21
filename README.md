# SimOxide

A performance simulator for Palladio Component Models (PCM), written in Rust. It simulates a model
the way **SimuLizar 5.2.2** or **Slingshot** does, exactly or fast, and changes the model while it
runs: SimuLizar's reconfiguration rules, Slingshot's scaling policies (SPD) and Kubernetes-style
autoscaling, on its own QVTo engine.

- **Exact.** For a supported model and a seed, SimOxide reproduces the reference byte for byte:
  the same event trace, the same random draws and the same `measurements.csv`, SimuLizar's bugs
  included. Checked on 66 corpus models, about 12 000 generated models, component oracles and
  queueing theory. The Slingshot-exact mode equals the deterministic Slingshot event for event on
  its 90-case corpus.
- **Fast.** A warm run is 250 to 2 600 times faster than stock SimuLizar 5.2.2 on its default
  DESMO-J engine, 240 to 3 300 times faster on its optional SSJ engine, and 230 to 1 300
  times faster than Slingshot; a one-shot run takes milliseconds instead of seconds. On 22 cores
  SimOxide completes over 5 000 MediaStore evaluations per second at 64 MB; the best JVM
  configuration completes 17 at 19 GB.
- **Two simulators' semantics.** `--flavour simulizar` (default), `slingshot` (Slingshot's
  semantics, its bugs fixed unless compat flags ask for them) and `slingshot-exact` (Slingshot's
  event bus handler by handler). `--mode fast` (default) or `exact` chooses the random numbers.
- **Runtime changes.** SimuLizar's QVTo reconfiguration rules; Slingshot's SPD with its own
  adjustment transformations; HPA autoscalers (`autoscaling/v2`); load profiles; change files.
- **Snapshots.** Pause between two events, fork, write a snapshot, change the model, resume in
  another process: the continuation equals the uninterrupted run byte for byte.
- **Embeddable.** A library with no global state that loads models from memory, runs batches in
  parallel and bounds untrusted models with limits.

## Quick start

Rust stable 1.88 or later:

```sh
cargo build --release                                       # EPL build, fast mode
./target/release/simoxide run --model corpus/x_sl_mediastore --measurements m.csv
./target/release/simoxide run --model corpus/x_sl_loadbalancer --flavour slingshot \
    --max-sim-time 1000 --max-measurements -1 --measurements m.csv
./target/release/simoxide bench --model corpus/x_sl_mediastore --runs 20 --threads 8

# with the QVTo engine: reconfiguration rules, SPD/HPA adjustments, resume with changes
cargo build --release -p simoxide-cli --features reconfiguration
./target/release/simoxide run --model corpus-rules/lb_outsource --rules-log rules.jsonl

# the exact build (local use only, see Licence): --mode exact
cargo build --release -p simoxide-cli --features exact,hotspot-math   # or: cargo build-exact
./target/release/simoxide run --mode exact --model DIR --seed 7 --max-measurements 10000 \
    --trace trace.jsonl --tape tape.jsonl --measurements m.csv
```

The tests and some example models need three git submodules with third-party models:
`git submodule update --init`, then `cargo run -q -p simoxide-thirdparty -- setup`
(`third_party/README.md`).

SimOxide comes in two kinds of build:

- **EPL build** (the default): the fast mode in both flavours, the Slingshot-exact mode, SPD, HPA,
  load profiles, snapshots, and with `--features reconfiguration` the QVTo engine. It contains
  only EPL-2.0 code (and permissively licensed dependencies) and may be distributed.
- **Exact build** (features `exact` and `hotspot-math`): adds `--mode exact`, which reproduces
  SimuLizar bit for bit. It needs Java `HashMap` iteration order (crate
  `simoxide-jcompat-exact`, for which no licence is granted) and HotSpot's `Math.log`/`Math.exp`
  (crate `simoxide-hotspot-math`, GPL-2.0-only). **Its binaries must not be distributed**; build
  it from source for your own use. Without `hotspot-math` the exact mode uses correctly rounded
  `log`/`exp`, which differ from HotSpot by one ulp on rare inputs
  (`docs/correctness/deviations.md`).

The libraries leave all of these features off by default.

## Documentation

The documentation is a VitePress site in `docs/`:

```sh
cd docs && npm install && npm run dev
```

| Section | Contents |
|---|---|
| [Guide](docs/guide/introduction.md) | getting started, [modes and features](docs/guide/modes.md), Slingshot-compatible mode, fast mode, reconfiguration rules, SPD, HPA, load profiles, snapshots, command line, library API, formats, [licensing](docs/guide/licensing.md) |
| [Performance](docs/performance/index.md) | comparison with SimuLizar, Slingshot, SimuCom and EventSim; engine benchmarks |
| [Correctness](docs/correctness/index.md) | the exactness claim and its evidence, testing, deviations, reference bugs |
| [Semantics](docs/spec/index.md) | SimuLizar 5.2.2's and [Slingshot's](docs/spec/slingshot/index.md) semantics as numbered rules with source references |
| [Reference simulators](docs/reference-simulator/refsim.md) | `refsim`, its patches, the Slingshot runner, the model corpus |
| [QVTo](docs/qvto/index.md) | the QVTo engine: usage, architecture, conformance, performance, its use in SimOxide, licensing |
| [Development](docs/development/contributing.md) | conventions, guardrails, benchmarking, test kit, foundation crates |

`DESIGN-SPD.md` is the design of the Slingshot mode, SPD, HPA, model changes and snapshots;
`history/` keeps the development log and the original plans.

## Repository layout

| Path | Contents |
|---|---|
| `crates/simoxide-model` | XMI loader with EMF semantics, typed PCM subset |
| `crates/simoxide-stoex` | stochastic expressions |
| `crates/simoxide-random` | the reference's random numbers and distributions |
| `crates/simoxide-sched` | schedulers and passive resources |
| `crates/simoxide-sim` | the simulator: compiler, event core, interpreter, measurements, both flavours, the Slingshot-exact engine (`ssx`), snapshots, API |
| `crates/simoxide-spd` | scaling policies (Slingshot's SPD interpreter) and HPA autoscalers |
| `crates/simoxide-reconf` | runtime model changes: direct edits, change files, the QVTo host (SimuLizar's rules, SPD adjustments), exploration |
| `crates/simoxide-cli` | `simoxide` and `simoxide-bench` |
| `crates/simoxide-testkit` | differential testing, model generator, `simoxide-fuzz` |
| `crates/simoxide-thirdparty` | the third-party test models behind git submodules: checks, derived files, digests |
| `crates/simoxide-oraclekit` | what the Java reference harnesses share: warm-JVM batch driver, seed agent, result directories, comparisons (`reference/oraclekit/` on the Java side) |
| `crates/foundation/` | shared with the QVTo engine: Java compatibility (`simoxide-jcompat`, opt-in `simoxide-jcompat-exact`), the EMF layer (`simoxide-emf-core`, `simoxide-emf`), `simoxide-hotspot-math` |
| `crates/qvto/` | the QVTo engine (`crates/qvto/README.md`, [docs/qvto](docs/qvto/index.md)) |
| `reference/` | `refsim`: deterministic SimuLizar 5.2.2, the oracle; `reference/slingshot/`: the Slingshot reference runner |
| `corpus/`, `corpus-fuzz/` | models with the reference's expected outputs |
| `corpus-ss/`, `corpus-spd/`, `corpus-rules/` | Slingshot-exact cases, SPD cases, reconfiguration-rule cases, with reference outputs |
| `third_party/` | git submodules with third-party example models (no licence declared upstream), SimOxide's changes to them as `ed` scripts |
| `bench/compare/` | the comparison with the other Palladio simulators |

## Licence

Eclipse Public License 2.0 (`LICENSE`), except two crates that only the exact build links:
`crates/foundation/simoxide-hotspot-math` (GPL-2.0-only, feature `hotspot-math`) and
`crates/foundation/simoxide-jcompat-exact` (no licence granted, features `exact`/`compat`).
SimOxide contains code and data derived from Palladio (SimuLizar, Slingshot, the SPD metamodel),
Apache Commons Math, JScience, Eclipse QVTo/OCL and OpenJDK; `LICENSES` lists them with their
licences and the open licensing issues. Example models whose repositories declare no licence
are not distributed: they are git submodules (`third_party/README.md`). Summary:
[Licensing](docs/guide/licensing.md).

# SimOxide

A Rust port of the Palladio performance simulator **SimuLizar 5.2.2**: the DESMO-J engine, the
default schedulers and the default measurements.

- **Exact.** For a supported model and a seed, SimOxide reproduces the reference byte for byte:
  the same event trace, the same random draws and the same `measurements.csv`, SimuLizar's bugs
  included. Checked on 66 corpus models, about 12 000 generated models, component oracles and
  queueing theory.
- **Fast.** A warm run is 250 to 2 400 times faster than stock SimuLizar 5.2.2 and 180 to 1 000
  times faster than Slingshot; a one-shot run takes milliseconds instead of seconds. On 22 cores
  SimOxide completes over 5 000 MediaStore evaluations per second at 64 MB; the best JVM
  configuration completes 17 at 19 GB.
- **Embeddable.** A library with no global state that loads models from memory, runs batches in
  parallel and bounds untrusted models with limits. An opt-in fast mode trades the reference's
  exact random numbers for statistically equivalent, faster ones.

## Quick start

Rust stable 1.88 or later:

```sh
cargo build --release
./target/release/simoxide run --model corpus/x_sl_mediastore --measurements m.csv   # fast mode
./target/release/simoxide run --mode exact --model DIR --seed 7 --max-measurements 10000 \   # exact build
    --trace trace.jsonl --tape tape.jsonl --measurements m.csv
./target/release/simoxide bench --model DIR --runs 20 --threads 8
```

SimOxide comes in two builds:

- **EPL build** (the default, `cargo build --release`): the fast mode (`--mode fast`, the
  default): the reference's semantics and statistics, other random numbers. It contains only
  EPL-2.0 (and Apache-2.0) code and may be distributed.
- **Exact build** (`cargo build --release -p simoxide-cli --features exact,hotspot-math`, or
  `cargo build-exact`): adds `--mode exact`, which reproduces the reference bit for bit. It needs
  Java `HashMap` iteration order (crate `simoxide-jcompat-exact`, for which no licence is granted)
  and HotSpot's `Math.log`/`Math.exp` (crate `simoxide-hotspot-math`, GPL-2.0-only). **Its
  binaries must not be distributed**; build it from source for your own use. Without
  `hotspot-math` the exact mode uses correctly rounded `log`/`exp`, which differ from HotSpot by
  one ulp on rare inputs (see `docs/correctness/deviations.md`).

The libraries leave all of these features off by default.

## Documentation

The documentation is a VitePress site in `docs/`:

```sh
cd docs && npm install && npm run dev
```

| Section | Contents |
|---|---|
| [Guide](docs/guide/introduction.md) | getting started, command line, library API, fast mode, supported models, output formats |
| [Performance](docs/performance/index.md) | comparison with SimuLizar, Slingshot, SimuCom and EventSim; engine benchmarks |
| [Correctness](docs/correctness/index.md) | the exactness claim and its evidence, testing, deviations, reference bugs |
| [Semantics](docs/spec/index.md) | SimuLizar 5.2.2's semantics as numbered rules with source references |
| [Reference simulator](docs/reference-simulator/refsim.md) | `refsim`, its patches, the model corpus |
| [QVTo](docs/qvto/index.md) | the QVTo engine: usage, architecture, conformance, performance, its use in SimOxide, licensing |
| [Development](docs/development/contributing.md) | conventions, guardrails, benchmarking, test kit, foundation crates |

`history/` keeps the development log and the original plan.

## Repository layout

| Path | Contents |
|---|---|
| `crates/simoxide-model` | XMI loader with EMF semantics, typed PCM subset |
| `crates/simoxide-stoex` | stochastic expressions |
| `crates/simoxide-random` | the reference's random numbers and distributions |
| `crates/simoxide-sched` | schedulers and passive resources |
| `crates/simoxide-sim` | the simulator: compiler, event core, interpreter, measurements, API |
| `crates/simoxide-cli` | `simoxide` and `simoxide-bench` |
| `crates/simoxide-testkit` | differential testing, model generator, `simoxide-fuzz` |
| `crates/foundation/` | shared with the QVTo engine: Java compatibility (`simoxide-jcompat`, opt-in `simoxide-jcompat-exact`), the EMF layer (`simoxide-emf-core`, `simoxide-emf`), `simoxide-hotspot-math` |
| `crates/qvto/` | the QVTo engine (`crates/qvto/README.md`, [docs/qvto](docs/qvto/index.md)) |
| `reference/` | `refsim`: deterministic SimuLizar 5.2.2, the oracle |
| `corpus/`, `corpus-fuzz/` | models with the reference's expected outputs |
| `bench/compare/` | the comparison with the other Palladio simulators |

## Licence

Eclipse Public License 2.0 (`LICENSE`), except two crates that only the exact build links:
`crates/foundation/simoxide-hotspot-math` (GPL-2.0-only, feature `hotspot-math`) and
`crates/foundation/simoxide-jcompat-exact` (no licence granted, features `exact`/`compat`). SimOxide contains code and data derived from Palladio, Apache Commons Math, JScience and
OpenJDK; `LICENSES` lists them with their licences and the open licensing issues.

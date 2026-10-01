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
./target/release/simoxide run --model corpus/x_sl_mediastore --measurements m.csv
./target/release/simoxide run --model DIR --seed 7 --max-measurements 10000 \
    --trace trace.jsonl --tape tape.jsonl --measurements m.csv
./target/release/simoxide bench --model DIR --runs 20 --threads 8
```

SimOxide comes in two builds:

- **Exact build** (the default of `simoxide-cli`): reproduces the reference bit for bit. This needs
  HotSpot's `Math.log` and `Math.exp`, which SimOxide ports from GPL-2.0-only OpenJDK code (crate
  `simoxide-hotspot-math`, cargo feature `hotspot-math`). The GPL-2.0 is incompatible with the
  EPL-2.0 and Apache-2.0 code in SimOxide, so build it from source for your own use. **Its
  binaries must not be distributed.**
- **EPL build** (`cargo build --release -p simoxide-cli --no-default-features --features fast`):
  contains no GPL code and can be distributed; use its fast mode. Its exact mode uses correctly
  rounded `log` and `exp`, which differ from HotSpot by one ulp on rare inputs, so it can drift
  from the reference (see `docs/correctness/deviations.md`).

The libraries (`simoxide-sim`, `simoxide-random`, `simoxide-stoex`) leave `hotspot-math` off by
default, so a program that embeds SimOxide contains no GPL code unless it enables the feature.

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
| [Development](docs/development/contributing.md) | conventions, guardrails, benchmarking, test kit |

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
| `reference/` | `refsim`: deterministic SimuLizar 5.2.2, the oracle |
| `corpus/`, `corpus-fuzz/` | models with the reference's expected outputs |
| `bench/compare/` | the comparison with the other Palladio simulators |

## Licence

Eclipse Public License 2.0 (`LICENSE`), except `crates/simoxide-hotspot-math`, which is
GPL-2.0-only and linked only through the feature `hotspot-math` (the exact build, see Quick
start). SimOxide contains code and data derived from Palladio, Apache Commons Math, JScience and
OpenJDK; `LICENSES` lists them with their licences and the open licensing issues.

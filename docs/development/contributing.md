# Contributing

## Principles

- **The reference defines "correct".** The behaviour of the patched SimuLizar 5.2.2 in
  `reference/` is the specification, not a paper or intuition. Where semantics matter, check the
  5.2.2 sources (the `releases/5.2.2` tags, or the decompiled product jars), not master.
- **Reference bugs are reproduced.** A fix goes behind an explicit option and onto
  [Deviations](../correctness/deviations.md).
- **Keep the reference's floating-point operation order** wherever it affects results. Faster
  algorithms that change rounding go behind a flag.
- **Performance work never changes an output** of the exact mode (see the guardrails below).

## Repository layout

| Path | Contents |
|---|---|
| `crates/foundation/` | shared by SimOxide and the QVTo engine ([Foundation crates](./foundation.md)): `simoxide-jcompat` (Java behaviour the javadoc specifies), `simoxide-emf-core` (EMF ids, metamodel registry seeded from the PCM tables, `ModelStore`), `simoxide-emf` (the EMF graph: SimOxide's model graph and QVTo's store), opt-in `simoxide-jcompat-exact` and `simoxide-hotspot-math` |
| `crates/simoxide-model` | XMI loader with EMF semantics, typed PCM subset, validation; the generated PCM tables (`src/meta`, the registry's static seed) |
| `crates/simoxide-stoex` | StoEx parser, types, evaluator, PMF/PDF, compiler, Java math |
| `crates/simoxide-random` | the reference's random numbers and distributions; fast-mode samplers |
| `crates/simoxide-sched` | processor sharing, FCFS, delay and passive resources |
| `crates/simoxide-sim` | IR compiler, event core, interpreter, measurements, batch and embedding API |
| `crates/simoxide-cli` | `simoxide`, `simoxide-bench`, `bench/golden.sh` |
| `crates/simoxide-reconf` | runtime model changes: the QVTo host (transformations, SimuLizar's rules, SPD adjustments), journal to change set, direct edits |
| `crates/simoxide-testkit` | differential testing, model generator, `simoxide-fuzz` |
| `crates/simoxide-oraclekit` | what the Java reference harnesses share: warm-JVM batch driver, seed agent, result directories, comparisons (`reference/oraclekit/` on the Java side) |
| `crates/qvto/` | the QVTo engine (`qvto-*` crates, its reference runner and oracles; [QVTo engine](../qvto/), [Development](../qvto/development.md)) |
| `reference/` | [refsim](../reference-simulator/refsim.md), patches, component oracles |
| `corpus/`, `corpus-fuzz/` | models with the reference's expected outputs; saved fuzz cases |
| `bench/compare/` | the comparison with the other Palladio simulators |
| `docs/` | this site (VitePress); `docs/spec/` is the semantics specification |
| `history/` | the development log and plans, also qvto-rs's milestone log; not maintained |

## Conventions

- Rust stable, edition 2024, one cargo workspace.
- `cargo fmt`, and `cargo clippy --all-targets -- -D warnings` must pass, for the workspace and
  for `simoxide-sim` and `simoxide-random` with and without the `fast` feature.
- No `unsafe` without a comment explaining why and a test.
- Tests are quick by default. Anything long goes behind `#[ignore]`, an environment variable or a
  feature flag.
- Java: Java 21 at `/usr/bin/java`; reference jars are referenced in place, never copied.
- Documentation describes the current state, short and factual. Semantics rules keep their
  identifiers (`SIM-4.4a`, `ACT-7.3`, `REF-7`, ...): code comments cite them.

## Guardrails

Run after every change to the simulator:

1. `cargo test --release --workspace` (includes the corpus in both modes, `tests/bugs.rs` and
   `tests/literal_engine.rs`).
2. `cargo build-exact`, then `crates/simoxide-cli/bench/golden.sh > after.txt` (exact mode; the
   default build has only the fast mode): a hash of the trace, tape, measurements
   and event count of every model directory under three run configurations. Compare with the
   list from before the change; the hashes must not change.
3. `simoxide-fuzz corpus --corpus corpus --sim "cmd:…simoxide run --mode exact …"` (both RNG modes, against the
   reference's expected outputs), and the same with `--corpus corpus-fuzz`.
4. For work on aborting runs: `reference/reffail-cmp.sh`.
5. For optimisations: hashes of the outputs of generated models (for example 1 000 with default
   features and 400 with all optional features, three run configurations each, both modes),
   against a binary built before the change.

Cargo aliases (`.cargo/config.toml`) run the test gates per area:

| Alias | Runs |
|---|---|
| `cargo test-sim` | SimOxide's crates, release |
| `cargo test-fdn` | the [foundation crates](./foundation.md), release |
| `cargo test-qvto` | the QVTo crates (debug build, as in qvto-rs) |
| `cargo build-exact` | the exact build of `simoxide` and `simoxide-fuzz` (`exact`, `hotspot-math`) for `golden.sh` and the corpus |
| `cargo conform` | the QVTo conformance scoreboard in compat mode (`--suite all`; `--engine qvto-fast`, `--engine qvto-ordered`) |
| `cargo qvto` | the `qvto` tool with compat mode (`cargo qvto bench`, `cargo qvto spd-replay`, `cargo qvto conform record`) |

The tests turn the opt-in features `exact`/`compat` on through their dev-dependencies.
`conform` and `qvto` build with the profile `release-unwind`: they catch panics per case, and
the `release` profile aborts.

- **QVTo crates**: `cargo test-qvto`, then the scoreboard in the three modes (compat 589 of 589
  strictly equal to Eclipse QVTo, fast 589/584, ordered 583 plus 6 `java-fails-equal`), the
  analyser oracle (`oracle_report`, 1 102 of 1 102) and, for speed, `qvto bench` against a
  binary built before the change. [QVTo development](../qvto/development.md#test-gates) lists
  the commands.
- **Foundation crates**: `cargo test-fdn`, the QVTo gates, and SimOxide's guardrails above,
  because SimOxide loads its models into the same graph (`load-bench` for changes to the graph
  or the loader).
- **`simoxide-reconf`**: `cargo test-sim` covers it (the rules corpus byte for byte, the QVTo
  corpus on simulation models, model changes, snapshots).

The oracle, conformance and corpus tests of the QVTo and foundation crates need material that is
not in git; without it they skip ([Reference material](../qvto/development.md#reference-material)):

- the reference bulk under `crates/qvto/reference/` (the Eclipse QVTo and OCL checkouts, the
  runner's jars and classes, the specifications, the external corpora; about 1 GB, ignored by
  `crates/qvto/.gitignore`). Copy it from another checkout or fetch it as
  `crates/qvto/reference/README.md` says;
- the PCM metamodels in `target/pcm`, written by
  `crates/foundation/simoxide-emf/tools/pcm-setup.sh target/pcm` (run in `crates/qvto`). The
  generated `pcm.conf` holds absolute paths into its target directory, so generate it for every
  worktree and every `CARGO_TARGET_DIR`; do not copy it.

## Releases

Only the EPL build may be distributed as a binary: release binaries, container images and packages
are built with the default features (`cargo build --release -p simoxide-cli`, `-p qvto-cli`).
`cargo tree -e normal -p simoxide-cli` and `-p qvto-cli` must list neither
`simoxide-hotspot-math` nor `simoxide-jcompat-exact` (no licence granted, `LICENSES` section 7),
which only the opt-in features `exact`, `compat` and `hotspot-math` pull in. Source releases may contain everything; every crate package includes
its licence texts (`LICENSE`, and `LICENSES` for the EPL crates).

## Documentation site

```sh
cd docs
npm ci
npm run dev       # local preview
npm run build     # static site in docs/.vitepress/dist
```

`docs/performance/comparison.md` includes the generated tables of `bench/compare/results/`, and
`docs/reference-simulator/corpus.md` includes `corpus/INDEX.md`.

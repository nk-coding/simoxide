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
| `crates/simoxide-model` | XMI loader with EMF semantics, typed PCM subset, validation |
| `crates/simoxide-stoex` | StoEx parser, types, evaluator, PMF/PDF, compiler, Java math |
| `crates/simoxide-random` | the reference's random numbers and distributions; fast-mode samplers |
| `crates/simoxide-sched` | processor sharing, FCFS, delay and passive resources |
| `crates/simoxide-sim` | IR compiler, event core, interpreter, measurements, batch and embedding API |
| `crates/simoxide-cli` | `simoxide`, `simoxide-bench`, `bench/golden.sh` |
| `crates/simoxide-testkit` | differential testing, model generator, `simoxide-fuzz` |
| `reference/` | [refsim](../reference-simulator/refsim.md), patches, component oracles |
| `corpus/`, `corpus-fuzz/` | models with the reference's expected outputs; saved fuzz cases |
| `bench/compare/` | the comparison with the other Palladio simulators |
| `docs/` | this site (VitePress); `docs/spec/` is the semantics specification |
| `history/` | the development log and plans; not maintained |

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
2. `crates/simoxide-cli/bench/golden.sh > after.txt`: a hash of the trace, tape, measurements
   and event count of every model directory under three run configurations. Compare with the
   list from before the change; the hashes must not change.
3. `simoxide-fuzz corpus --corpus corpus --sim "cmd:…simoxide run …"` (both RNG modes, against the
   reference's expected outputs), and the same with `--corpus corpus-fuzz`.
4. For work on aborting runs: `reference/reffail-cmp.sh`.
5. For optimisations: hashes of the outputs of generated models (for example 1 000 with default
   features and 400 with all optional features, three run configurations each, both modes),
   against a binary built before the change.

## Releases

Only the EPL build may be distributed as a binary: release binaries, container images and packages
are built with `cargo build --release -p simoxide-cli --no-default-features --features fast`
(`cargo tree -p simoxide-cli -e normal --no-default-features --features fast` must not list
`simoxide-hotspot-math`). Source releases may contain everything; every crate package includes
its licence texts (`LICENSE`, and `LICENSES` for the EPL crates).

## Documentation site

```sh
cd docs
npm install
npm run dev       # local preview
npm run build     # static site in docs/.vitepress/dist
```

`docs/performance/comparison.md` includes the generated tables of `bench/compare/results/`, and
`docs/reference-simulator/corpus.md` includes `corpus/INDEX.md`.

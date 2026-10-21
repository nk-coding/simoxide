# QVTo engine (`crates/qvto`, formerly qvto-rs)

A QVT Operational Mappings (QVTo) engine in Rust, part of the SimOxide workspace. It runs
transformations as Eclipse QVTo 3.11 on classic Eclipse OCL does, compiled to a register
bytecode, in three modes: *fast* and *ordered* (EPL, every build) and *compat* (Eclipse's Java
`HashMap` orders and identity hashes; cargo feature `compat`, builds not for distribution).
SimOxide runs SimuLizar's reconfiguration rules and Slingshot's SPD adjustments on it.

It was developed as the separate repository qvto-rs and imported with its history
(`git log -- crates/qvto`).

## Documentation

The QVTo section of the documentation site (`docs/qvto/`):

| Page | Contents |
|---|---|
| `docs/qvto/index.md` | what the engine covers, the modes, the current conformance numbers |
| `docs/qvto/usage.md` | the `qvto` command, the library API, `RunOptions`, blackboxes |
| `docs/qvto/architecture.md` | the pipeline, the crates, the foundation crates they use |
| `docs/qvto/conformance.md` | the oracles, the results, the known differences from Eclipse |
| `docs/qvto/performance.md` | the benchmark suite and its results |
| `docs/qvto/simoxide.md` | how SimOxide uses the engine |
| `docs/qvto/licensing.md` | which modes and builds are EPL-only |
| `docs/qvto/development.md` | paths, reference material, test gates, recording fixtures |

Here, for developers:

| File | Contents |
|---|---|
| `DESIGN.md` | the engine's design and decision log (§12), from qvto-rs |
| `docs/merge-into-simoxide.md` | the merge into SimOxide: inventory, foundation crates, licensing split, steps M0-M8 and their status |
| `qvto-cli/README.md` | the `qvto` command; the benchmark suite and its results per workload |
| `qvto-testkit/README.md` | the conformance harness: cases, the Java side, verdicts, tapes, corpora, fuzzing |
| `reference/README.md` | the specifications, Eclipse sources and the Java reference runner, and how to get them |
| `reference/inventory/` | the inventory of the Eclipse QVTo test suite |
| `reference/notes/` | notes on Eclipse QVTo's and OCL's semantics, with source references |
| crate `README.md`s | each crate's files and verification |

qvto-rs's milestone log is `history/qvto-plan.md` at the repository root (not maintained).

## Layout

| Crate | Contents |
|---|---|
| `qvto-syntax` | lexer, parser, AST, spans, error recovery |
| `qvto-types` | type system, standard library table, blackbox signatures |
| `qvto-sema` | semantic analysis and type checking; the typed IR |
| `qvto-bytecode` | instruction set, compiler, disassembler |
| `qvto-runtime` | values, collections, Java-compatible behaviour of the modes |
| `qvto-vm` | the interpreter |
| `qvto` | the engine's facade, blackbox libraries |
| `qvto-testkit` | conformance runner, differential testing, fuzz generators and regression cases |
| `qvto-cli` | the `qvto` command |
| `qvto-paths` | where tests, examples and tools find the QVTo root and the target directory |

The model API and the EMF layer are foundation crates shared with SimOxide:
`crates/foundation/simoxide-emf-core` (imported as `qvto_model`) and
`crates/foundation/simoxide-emf` (imported as `qvto_emf`).

Paths in the QVTo documents, case files and tools are relative to this directory, the *QVTo
root*: `reference/...`, `qvto-testkit/cases/...`, and `target/...` for generated material.
`target` here is a link to the workspace's target directory, which `qvto-paths` keeps up to date
(`docs/qvto/development.md`, "Paths").

## Building and testing

From the repository root, niced on a shared machine
(`CARGO_BUILD_JOBS=4 nice -n 10 cargo ...`). The reference material that is not in git (the
`reference/` bulk, and the PCM metamodels in `target/pcm`, generated per target directory) is
described in `docs/qvto/development.md`, "Reference material"; without it the oracle,
conformance and corpus tests skip.

| Command | Gate |
|---|---|
| `cargo test-qvto` | all tests of the QVTo crates pass |
| `cargo conform --suite all` | compat: 589 of 589 runnable cases strictly equal to the Java runner |
| `cargo conform --suite all --engine qvto-fast` | fast: 589 pass the Eclipse checks, 584 strictly |
| `cargo conform --suite all --engine qvto-ordered` | ordered vs the `det` Java runner: 583 strictly plus 6 `java-fails-equal` |
| `cargo run --profile release-unwind -p qvto-sema --example oracle_report` | analyser: 1 102 of 1 102 units equal to Eclipse |
| `cargo qvto spd-replay` | ordered mode vs the Slingshot reference runner: 13 of 13 |
| `taskset -c 2,3 cargo qvto bench` | the benchmark suite, against a binary built before the change |

## Licence

Eclipse Public License 2.0, the licence of Eclipse QVTo and of SimOxide, except that builds with
the feature `compat` include `simoxide-jcompat-exact`, for which no licence is granted. See
`docs/qvto/licensing.md` and the repository's `LICENSES`.

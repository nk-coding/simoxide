# QVTo engine (qvto-rs)

A QVT Operational Mappings (QVTo) engine in Rust, part of the SimOxide workspace. It was developed
as the separate repository qvto-rs and imported with its history (`git log -- crates/qvto`).

- **Complete.** Covers the QVTo language of the OMG QVT 1.3 specification and the OCL it embeds,
  as implemented by Eclipse QVTo, the reference implementation.
- **Three modes.** *Compat mode* behaves like Eclipse QVTo, including its quirks: collection
  iteration order, number and string formatting, null and invalid handling, trace and resolve
  semantics. *Fast mode* keeps the semantics but drops the Java emulation that costs time.
  *Ordered mode* behaves like Eclipse QVTo in a JVM whose QVTo and OCL hash collections iterate in
  insertion order (the Slingshot reference runner's `det` mode), for SimOxide's Slingshot-exact
  mode.
- **Bytecode.** Transformations compile to a register bytecode that a VM interprets.
- **Embeddable.** The engine reads and writes models through a trait, so it runs on its own
  Ecore/XMI layer and directly on SimOxide's model graph (`qvto-simoxide`).

Status: `PLAN.md` (the qvto-rs milestone log, Q0-Q7, all done). The merge into SimOxide follows
`docs/merge-into-simoxide.md` (steps M0-M8; status at its top).

Paths in the QVTo documents, case files and tools are relative to this directory
(`crates/qvto/`, the *QVTo root*): `reference/...`, `qvto-testkit/cases/...`, and `target/...`
for generated material. `target` here is a link to the workspace's target directory
(`$CARGO_TARGET_DIR` at run time, else `<repository>/target`); the Rust tools create and re-point
it (`qvto-paths`), so make it by hand only before running a Java tool first:
`ln -s ../../target target`. Generated material (`target/pcm`, `target/qvto-testkit/java`,
`target/emf-oracle`) therefore belongs to one target directory.

## Documents

| File | Contents |
|---|---|
| `DESIGN.md` | scope, architecture, model API, bytecode and VM, compat vs fast, performance, testing, milestones, licensing |
| `PLAN.md` | milestones and their status (qvto-rs history) |
| `docs/merge-into-simoxide.md` | the merge into SimOxide: duplication inventory, foundation crates, licensing split, steps |
| `docs/simoxide-integration.md` | running the engine inside SimOxide (superseded by `DESIGN-SPD.md` and the merge design) |
| `qvto-cli/README.md` | the `qvto` command; the benchmark suite (`qvto bench`) and its results against Eclipse QVTo |
| `reference/README.md` | specifications, Eclipse sources, the Java reference runner |
| `reference/external/` | external QVTo corpora (not committed; `SOURCES.tsv`: repositories, revisions, licences) |
| `reference/inventory/` | inventory of the Eclipse QVTo test suite |
| `reference/notes/` | notes on Eclipse QVTo and OCL semantics |

## Layout

| Crate | Contents |
|---|---|
| `qvto-syntax` | lexer, parser, AST, spans, error recovery |
| `qvto-model` | the model API is `crates/foundation/simoxide-emf-core` (metamodel registry, static seed, `ModelStore` trait, contract tests), imported as `qvto_model` |
| `qvto-emf` | own Ecore/XMI store: EMF semantics, `.ecore` metamodels, XMI load/save as EMF, canonical dump, the suite's comparer |
| `qvto-types` | type system, standard library signatures |
| `qvto-sema` | semantic analysis and type checking |
| `qvto-bytecode` | instruction set, compiler, disassembler |
| `qvto-runtime` | values, collections, Java-compatible hashing and formatting, standard library |
| `qvto-vm` | the interpreter |
| `qvto` | engine facade |
| `qvto-testkit` | conformance runner, differential testing, fuzz generators and regression cases |
| `qvto-cli` | the `qvto` command |
| `qvto-paths` | where tests, examples and tools find the QVTo root and the target directory |
| `qvto-simoxide` | the store over SimOxide's model graph (`simoxide-model`), its metamodel registry, the change journal; the bridge until the shared EMF graph (merge steps M5/M6) |

## Building and testing

From the repository root. This machine is shared: build niced with few jobs
(`CARGO_BUILD_JOBS=4 nice -n 19 cargo ...`, tests with `-- --test-threads=4`).

The ignored reference material (`reference/specs`, `eclipse-qvto`, `eclipse-ocl`, `runner/lib`,
`runner/build`, `external`; `reference/README.md` says how to get it) and the generated PCM
metamodels (`../foundation/simoxide-emf/tools/pcm-setup.sh target/pcm`, run here) are needed by the oracle,
conformance and corpus tests; without them those tests skip.

| Command | Gate |
|---|---|
| `cargo test-qvto` | all tests of the QVTo crates pass (alias in `.cargo/config.toml`; the tests turn `compat` on) |
| `cargo conform --suite all` | compat: 589 of 589 runnable cases equal the Java runner strictly (Eclipse 494, corpus 24, fuzz 24, SPD 47); `--suite spd --compare-unstable` 47 of 47 also on what varies between Java runs |
| `cargo conform --suite all --engine qvto-fast` | fast: 589 pass the Eclipse checks, 584 strictly (the order-insensitive verdict) |
| `cargo conform --suite all --engine qvto-ordered` | ordered vs the `det` Java runner: 583 strictly plus 6 `java-fails-equal` (the `det` rule breaks those cases' own assertions in Java too) |
| `cargo qvto spd-replay` | ordered mode vs the Slingshot reference runner's `det` runs: 13 of 13 recorded adjustments equal (needs `reference/slingshot`) |
| `cargo run --profile release-unwind -p qvto-cli -- conform --suite all --engine qvto-fast` | the same in the EPL build (no `compat`): identical scoreboard |
| `cargo run --profile release-unwind -p qvto-sema --example oracle_report` | analyser oracle: 1 102 of 1 102 keys match |
| `taskset -c 2,3 cargo qvto bench` | the benchmark suite (`qvto-cli/README.md`), against a binary built before the change |

Modes and builds: the default build is EPL-only and has the fast and ordered modes
(`Mode::default()` is `Fast`). Compat mode (Eclipse's `HashSet`/`HashMap` orders, HotSpot identity hashes) needs the
cargo feature `compat` (qvto-runtime, qvto-vm, qvto, qvto-testkit, qvto-cli, qvto-simoxide),
which links `simoxide-jcompat-exact`; no licence is granted for it, so such builds must not be
distributed (the repository's `LICENSES`, section 7). A run in compat mode without the feature
fails with `RunOutcome::code == MODE_UNAVAILABLE`; `qvto conform --engine qvto` and
`qvto fuzz` refuse with a message.

Choosing a mode (`qvto::Mode`, the same as `qvto_runtime::Mode`):

```rust
use qvto::Mode;
// Slingshot-exact (EPL, every build): Eclipse's semantics, Set/Bag/Dict in insertion order
let opts = qvto::RunOptions { mode: Mode::Ordered, ..qvto::RunOptions::default() };
// SimuLizar-exact needs Mode::Compat (feature `compat`); Mode::Fast for throughput
assert!(Mode::Ordered.available());
let m = Mode::from_name("ordered"); // "compat" | "fast" | "ordered", for configuration files
```

| Mode | Hash collections | Other Eclipse emulation | Identity hashes (`Class@hash`) | Build |
|---|---|---|---|---|
| `Compat` | Java `HashMap` bucket order | all | HotSpot sequence or a tape | feature `compat` (no licence, LICENSES §7) |
| `Ordered` | insertion order; a `Set` is a `LinkedHashSet` (OCL sees an OrderedSet, as in the `det` JVM) | all | from the key, or `RunOptions::id_source` | every build (EPL) |
| `Fast` | insertion order | all but the double evaluation of an effectful `sortedBy` | from the key | every build (EPL) |

`qvto conform`, `qvto fuzz` and the oracle report catch panics per case, so they are built with
the `release-unwind` profile (the workspace's `release` profile aborts on panic).

## Licence

Eclipse Public License 2.0, the licence of Eclipse QVTo and of SimOxide. See `DESIGN.md`,
"Licensing", and the repository's `LICENSES`.

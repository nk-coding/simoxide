# Development

The repository's [Contributing](../development/contributing.md) rules apply. This page adds
what is specific to the QVTo crates.

## Layout

| Path | Contents |
|---|---|
| `crates/qvto/qvto-*` | the engine's crates ([Architecture](./architecture.md)), each with a `README.md` on its files and its verification |
| `crates/qvto/reference/` | the Java reference runner (`runner/`), the inventory of the Eclipse test suite (`inventory/`), notes on Eclipse's semantics (`notes/`); the bulk is not committed (below) |
| `crates/qvto/qvto-testkit/cases`, `fixtures`, `corpus` | the conformance cases, the Java runner's committed results, the SimuLizar and SPD material |
| `crates/qvto/DESIGN.md` | the engine's design and decision log (developer document from qvto-rs; code comments cite its §12 entries as `DESIGN §12 Q5b-3`) |
| `crates/qvto/docs/merge-into-simoxide.md` | the merge of qvto-rs into SimOxide: steps and their status |
| `crates/foundation/` | the crates shared with SimOxide ([Foundation crates](../development/foundation.md)) |
| `history/qvto-plan.md` | qvto-rs's milestone log (Q0-Q7, SPD-Q1, SPD-Q2), not maintained |

## Paths

Paths in the QVTo case files, tools and documents are relative to `crates/qvto/`, the *QVTo
root*: `reference/...`, `qvto-testkit/cases/...`, and `target/...` for generated material
(`target/pcm`, `target/qvto-testkit/java`, `target/qvto-bench`, `target/emf-oracle`).
`crates/qvto/target` is a symbolic link to the workspace's target directory (`CARGO_TARGET_DIR`,
else `<repository>/target`). The `qvto-paths` crate finds the QVTo root, and it creates the link
or points it at the current target directory whenever a Rust test, example or tool starts. So the
Java runner, the case files and the scripts see the same files as the Rust code. Make the link
by hand only when a Java tool runs before any Rust tool in a fresh checkout:
`ln -s ../../target crates/qvto/target`. All generated material therefore belongs to one target
directory.

## Reference material

Two kinds of material are not in git. Without them the oracle, conformance and corpus tests skip,
and `qvto conform` cannot run.

**The reference bulk** under `crates/qvto/reference/` (ignored by `crates/qvto/.gitignore`):

| Directory | Contents | Size |
|---|---|---|
| `specs/` | the OMG QVT 1.3 and OCL 2.4 specifications, as PDF and text | 5 MB |
| `eclipse-qvto/` | sparse checkout of Eclipse QVTo at a fixed commit: grammars, compiler, evaluator, the test suite (the conformance cases point into it) | 32 MB |
| `eclipse-ocl/` | sparse checkout of classic Eclipse OCL | 14 MB |
| `runner/lib/` | the QVTo 3.11.0, OCL, EMF and Equinox jars the runner is built against | 10 MB |
| `runner/build/` | the runner's class files (`reference/runner/build.sh`) | |
| `external/` | the external corpora with `SOURCES.tsv` | 1 GB |

`crates/qvto/reference/README.md` gives the source, revision and commands for each. In a new
worktree or clone, copy the directories from a checkout that has them, as M1 of the merge did:

```sh
for d in specs eclipse-qvto eclipse-ocl runner/lib runner/build external; do
  mkdir -p "crates/qvto/reference/$(dirname $d)"
  cp -a "$OTHER/crates/qvto/reference/$d" "crates/qvto/reference/$d"
done
```

**The PCM metamodels** in `target/pcm`, for the SimuLizar and SPD corpus, the SPD suite and the
EMF oracle's PCM part. Generate them from the QVTo root:

```sh
cd crates/qvto
../foundation/simoxide-emf/tools/pcm-setup.sh target/pcm [plugins-dir] [corpus-dir] [repos-dir]
```

The script extracts the PCM 5.2.2 metamodels and default models from the Palladio product jars
(`plugins-dir`), and takes the SimuLizar and SPD metamodels and the Slingshot examples from the
Palladio source repositories (`repos-dir`). It writes `target/pcm/pcm.conf` and
`pcm-models.txt`. **`pcm.conf` holds absolute paths** into the target directory and the
repositories, so it is generated per target directory: every worktree and every
`CARGO_TARGET_DIR` needs its own run, and a copied `pcm.conf` points into the other tree. The
defaults of the optional arguments are absolute paths on the development machine; pass them
elsewhere.

Java 21 (`/usr/bin/java`) runs the reference runner and the oracles. The `det` runner also needs
the seed agent of the Slingshot runner's `det` mode, which `simoxide-oraclekit` builds on demand
(`reference/oraclekit/seed-agent.sh` at the repository root, into `reference/oraclekit/build/`).

## Test gates

From the repository root. On a shared machine, build niced with few jobs
(`CARGO_BUILD_JOBS=4 nice -n 10 cargo ...`, tests with `-- --test-threads=4`). The aliases are in
`.cargo/config.toml`.

| Command | Gate |
|---|---|
| `cargo test-qvto` | every test of the QVTo crates passes (debug build; the tests turn `compat` on) |
| `cargo test-fdn` | every test of the foundation crates passes (release) |
| `cargo conform --suite all` | compat mode: 589 of 589 runnable cases strictly equal to the Java runner |
| `cargo conform --suite all --engine qvto-fast` | fast mode: 589 pass the drivers' checks, 584 strictly |
| `cargo conform --suite all --engine qvto-ordered` | ordered mode against the `det` runner: 583 strictly, plus 6 `java-fails-equal` |
| `cargo run --profile release-unwind -p qvto-cli -- conform --suite all --engine qvto-fast` | the same scoreboard in the EPL build (no `compat`) |
| `cargo run --profile release-unwind -p qvto-sema --example oracle_report` | the analyser: 1 102 of 1 102 units equal to Eclipse |
| `cargo qvto spd-replay` | ordered mode against the Slingshot reference runner: 13 of 13 (needs `reference/slingshot`) |
| `taskset -c 2,3 cargo qvto bench` | the benchmark suite, against a binary built before the change ([Performance](./performance.md)) |
| `cargo tree -e normal -p qvto-cli` | the release check: neither `simoxide-jcompat-exact` nor `simoxide-hotspot-math` ([Licensing](./licensing.md)) |

`cargo conform` and `cargo qvto` build `qvto-cli` with `--features compat` and the
`release-unwind` profile: the scoreboard, the fuzzers and the oracle report catch panics per case,
and the workspace's `release` profile aborts on a panic. Changes to the foundation crates also
run SimOxide's guardrails ([Contributing](../development/contributing.md#guardrails)), because
SimOxide loads its models into the same graph.

## Recording fixtures

The Java side's results are committed in `qvto-testkit/fixtures/`. Record them again after a
change of the cases or of the runner:

```sh
cargo qvto conform record --suite all          # 5 Java runs per case; marks unstable outputs
cargo qvto conform record --suite all --det    # the det runner's fixtures (ordered mode)
cargo qvto conform record --suite all --check  # record again, compare with the committed fixtures (outputs marked unstable excepted); writes nothing
cargo qvto conform java --suite eclipse --out DIR --save-xmi   # one Java run, full results
```

The full results (tapes, XMI) stay in `target/qvto-testkit/java/<suite>`. Compat mode replays the
identity hashes of that cache when it is present, so keep it with the fixtures it belongs to. A
fresh recording may draw other HotSpot seeds, and `interpret_scr17812` replays only with some of
them. The generators of the cases are in `qvto-testkit/tools/`: `gen_cases.py` for the Eclipse
inventory, `copy-corpus.sh`, `gen-spd-corpus.py`, `add-fuzz-case.py` and
`gen_external_cases.py`. Run them with `python3 -I` from the QVTo root.

Each crate's oracle has its own `tools/oracle` scripts, which build against the runner's jars:
the parser in `qvto-syntax`, the library table in `qvto-types`, the analyser in `qvto-sema`, the
Java primitives in `qvto-runtime`, and the EMF oracle in `crates/foundation/simoxide-emf/oracle`.

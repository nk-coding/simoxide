# Third-party test models (git submodules)

SimOxide's tests use example models from Palladio repositories that declare no licence
(`LICENSES` section 5). SimOxide does not distribute them: they are git submodules here, pinned to
the commits the former copies came from, and the test material only refers to them.

| Submodule | Repository | Commit |
|---|---|---|
| `palladio-example-models` | https://github.com/PalladioSimulator/Palladio-Example-Models | `4a8dc455216774435fefd42965b848851f7658ee` (2026-01-26) |
| `slingshot-e2e-tests` | https://github.com/PalladioSimulator/Palladio-Analyzer-Slingshot-E2E-Tests (archived) | `fd3e916d7cb522059efb29e1565068b99cb2bf64` (2026-01-26) |
| `slingshot-docs` | https://github.com/PalladioSimulator/Palladio-Documentation-Slingshot | `94448159e75f33edfb98e960ad244018b97e37d4` (2024-09-13), `examples/minimalexample` |

## Setup

```sh
git clone --recurse-submodules https://github.com/nk-coding/simoxide   # or, in a clone:
git submodule update --init
cargo run -q -p simoxide-thirdparty -- setup                           # optional, see below
```

`cargo build` does not need the submodules. Without them, the tests that read their models skip
and print `skipped (third-party): <dir> needs the submodule third_party/<name> (not checked
out): git submodule update --init` (cargo shows it with `-- --nocapture`; a skipped test passes).
With `SIMOXIDE_REQUIRE_THIRD_PARTY=1` they fail instead; set it
in CI and before regenerating expected outputs, so that a missing checkout cannot pass as green.

## How the files are referenced

- **Unmodified files** are relative symbolic links in place of the former copies, e.g.
  `corpus/x_pem_acquire/AcquireExample.repository -> ../../third_party/palladio-example-models/Acquire_Example/AcquireExample.repository`.
  Every consumer (Rust tests, CLI, refsim, the Java oracles and the Slingshot runner) reads them
  as before. SimOxide's own files stay next to them: `run.json`, `refsim.monitorrepository`,
  `refsim.measuringpoint`, `expected/`, `README.md`, `observed/`.
  - Linked directories: `corpus/x_pem_*`, `corpus/x_ss_*`,
    `reference/slingshot/upstream-report/reproductions/x_pem_acquire`,
    `crates/simoxide-sim/tests/models/u_{heuristics,linking}_own_monitors` (Palladio-Example-Models
    with their own monitor repositories), `reference/slingshot/models/spd_minimal` and
    `reference/slingshot/upstream-report/reproductions/spd_minimal` (Slingshot's minimal example).
  - The `corpus/x_pem_*`, `corpus/x_ss_*` and `u_*` copies had been written by `refsim import` (EMF load
    and save: line width 120, features in metamodel order, fragment paths replaced by ids,
    namespace prefixes renamed). The links point at the upstream files instead; both load into the
    same model (the canonical dumps of `reference/oracles/pcm-model` and every expected
    output are unchanged), so the re-save is not repeated. One exception: where the upstream file
    has elements without an `id`, EMF generated one at import and the outputs name it. Those
    files are derived (below) with exactly these ids inserted, nothing else:
    `corpus/x_pem_replicated_composite/default.{repository,resourceenvironment}` and
    `corpus/x_pem_subsystem{,_nested}/{default.repository,My.resourceenvironment}` (two ids each).
- **Modified files** (SimOxide changed an upstream file) are derived:
  `third_party/derived.txt` lists `destination<TAB>upstream source`, and
  `third_party/derived/<destination>.ed` holds the change as an `ed` script (`diff -e`: line
  numbers and SimOxide's new lines only). The destinations are git-ignored (block at the end of
  `.gitignore`). The test harnesses derive them on first use (`simoxide_thirdparty::check`);
  scripts and the Java runners need `cargo run -q -p simoxide-thirdparty -- setup` once after
  `git submodule update --init`. The derived files of the SPD example and the QVTo corpus are
  byte-identical to the former copies. In a directory with a derived file, every upstream file
  is derived (unchanged ones with an empty script): EMF on the JVM resolves a link to its target
  and would read the siblings of a linked file from the submodule, not the changed ones.
  - `reference/slingshot/models/spd_{same_instant,scale_in,service_group,time_policies,v4_cpu,v4_rt}`
    and `reference/slingshot/upstream-report/reproductions/spd_{and_trigger,or_trigger,scale_in}`:
    SimOxide's scaling policies in `default.spd`, and in `spd_scale_in` / `spd_v4_*` changed
    demands (`default.repository`) and arrival rates (`default.usagemodel`); see
    `reference/slingshot/models/README.md`.
  - `crates/qvto/qvto-testkit/corpus/spd/minimalexample`: a provenance comment after the XML
    declaration, and in `default.semanticspd` the enacted policy (`tools/copy-corpus.sh`).
  - `crates/qvto/qvto-testkit/corpus/spd/q1/min`, `q1/s_min_abs_3`, `q1/s_min_abs_3-noid`: the SPD
    adjustment corpus (`tools/gen-spd-corpus.py`): header comment, the configurations
    (`*.semanticspd`, variants of the example's), and the chained states (outputs of Slingshot's
    MainTransformation run by the Java runner, then ids assigned). `q1/*/default.spd` and the
    `q1/full` models are SimOxide's own and stay committed.

## Dumps

Outputs that reproduce one of these models in full are not committed either; the repository
keeps their SHA-256 and length, and the tests produce the dump from the submodules and compare
digests (`simoxide_thirdparty::digest`). Without the submodules these tests skip like the
others; small results (simulation outputs, metrics, journals, logs) stay committed.

| Directory | Committed | Full dump written by |
|---|---|---|
| `reference/oracles/pcm-model/golden/` | `third-party.digests` for `Palladio-Example-Models_*`, `Palladio-Analyzer-Slingshot-E2E-Tests_*`, `Palladio-Documentation-Slingshot_*`, `corpus_x_pem_*`, `corpus_x_ss_*` (the EMF dumps of the other models stay) | `run.sh` (Java, EMF 5.2.2 jars: `reference/README.md`) |
| `reference/oracles/pcm-model/golden-spd/` | `third-party.digests` for `slingshot_spd_*` (`spd_basic.jsonl` stays) | `run-spd.sh` (Java, plus the Slingshot SPD jars) |
| `crates/qvto/qvto-testkit/fixtures/java-{corpus,spd}{,-det}.txt` | `out.jsonl.digest` in place of `out.jsonl` for the cases with a third-party input (`spd_minimalexample_main`, `spd_min*`): digest of the dump with renumbered extent resources, and its resource lines | `qvto conform record` (Java runner; the full results stay in `target/qvto-testkit/java*`) |
| `crates/simoxide-reconf/tests/data/adapter-corpus/` | `spd_minimalexample_main.dump.digest` (journal, status and log stay) | `PARITY_OUT=DIR` with the parity test |

The full dumps are git-ignored where the oracles write them, and a full `golden*/<name>.jsonl`
takes precedence over the digest in the tests (to see the first difference). After running an
oracle, `cargo run -q -p simoxide-thirdparty -- digests reference/oracles/pcm-model/golden`
(`run.sh` and `run-spd.sh` do it) rewrites the digests; `qvto conform record` writes the
fixtures' digests itself. Which outputs are third-party is decided from the inputs
(`simoxide_thirdparty::uses_third_party`, `digest::THIRD_PARTY_DUMPS`); the tests
`third_party_dumps_are_digests` (simoxide-model) and
`fixtures_digest_exactly_the_dumps_of_third_party_cases` (qvto-testkit) check it.

## Changing a derived file

Edit or regenerate the destination, then record it again:

```sh
cargo run -q -p simoxide-thirdparty -- record <destination> <upstream source>
cargo run -q -p simoxide-thirdparty -- verify   # every derived file = source + script, no dangling link
```

`record` writes the script, the manifest line and the `.gitignore` entry. To update a submodule,
check out the new commit in it, run `verify`, and update the commit in this file, in
`crates/simoxide-thirdparty/src/lib.rs` and in `LICENSES` section 5.

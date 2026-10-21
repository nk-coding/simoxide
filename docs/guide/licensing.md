# Licensing and third-party material

SimOxide is licensed under the **Eclipse Public License 2.0** (`LICENSE`), like SimuLizar,
Slingshot, Eclipse QVTo and OCL, whose behaviour it reproduces. Two opt-in crates are kept apart,
and some test models are not part of the repository at all. `LICENSES` in the repository root is
the authoritative text; this page summarises it.

## What a build contains

| Crate | Licence | Pulled in by | Needed for |
|---|---|---|---|
| everything else | EPL-2.0 (with Apache-2.0 and BSD parts, `LICENSES` sections 1-3, 6, 8) | default | the fast mode, both Slingshot variants, SPD, HPA, QVTo's fast and ordered modes |
| `crates/foundation/simoxide-jcompat-exact` | **no licence granted** (`LICENSE-NOTICE`) | features `exact` (`simoxide-sim`, `-spd`, `-reconf`, `-testkit`, `-cli`), `jdk-order` (`simoxide-sim`), `compat` (QVTo crates, `simoxide-reconf`) | Java `HashMap` iteration order and HotSpot identity hashes: SimuLizar's exact mode, QVTo's compat mode |
| `crates/foundation/simoxide-hotspot-math` | **GPL-2.0-only** (a port of OpenJDK's Intel LIBM stubs) | feature `hotspot-math` (`simoxide-cli`, `-sim`, `-random`, `-stoex`) | `Math.log`/`Math.exp` bit-identical to HotSpot |

`simoxide-jcompat-exact` was written from documented behaviour and differential tests against
JDK 21, but only OpenJDK's GPL sources define the details it reproduces, while the crates that
use it are EPL-2.0. No licence covers that mix, so the crate grants none and builds that include
it are for local use only. The Slingshot-exact mode needs neither crate: the patched Slingshot
runner iterates in insertion order, and `java.util.Random` is specified by its javadoc.

So:

- **The default build may be distributed.** `cargo build --release` (also with
  `--features reconfiguration`) contains EPL-2.0 code and permissively licensed dependencies
  only.
- **The exact build must not be distributed.** `cargo build-exact`, or any build with `exact`,
  `compat`, `jdk-order` or `hotspot-math`, is for your own use: build it from source.
- **The release check.** With default features,

  ```sh
  cargo tree -e normal -p simoxide-cli
  cargo tree -e normal -p qvto-cli
  ```

  must list neither `simoxide-jcompat-exact` nor `simoxide-hotspot-math`.
- **No silent fallback.** A mode whose code is not compiled is an error (`ModeUnavailable`,
  `MODE_UNAVAILABLE` in QVTo), not a quiet switch to another mode.
- The optional feature `profile` of `simoxide-cli` pulls in `inferno` (CDDL-1.0) through `pprof`.

Without `hotspot-math` the SimuLizar-exact mode computes `log` and `exp` correctly rounded. HotSpot
differs from that on about 0.25 % of `exp` and 1e-5 of `log` arguments, so a run equals the
reference until the first draw that hits such an argument
([Deviations](../correctness/deviations.md#math-log-and-math-exp-without-hotspot-math)).

## Where SimOxide's code and data come from

| Source | Licence | Used for (`LICENSES` section) |
|---|---|---|
| Palladio 5.2.2: SimuLizar, SimuCom, AbstractSimEngine, Scheduler, Core-Commons, Core-PCM, QuAL | EPL-1.0 / EPL-2.0 (EPL-1.0 content redistributed under EPL-2.0) | ports of schedulers, StoEx, distributions; metamodel tables; standard models; SimuLizar's reconfiguration examples (§1) |
| Palladio 6.0 Core-Commons distributions | EPL-2.0 | the Slingshot-exact mode's distributions (§1) |
| Palladio-Analyzer-Slingshot | EPL-2.0 (LICENSE added upstream in October 2026) | the Slingshot-exact mode follows its handlers statement by statement, without copying source; a patched `ProcessorSharingResource` for the reference runner (§1) |
| Palladio-Addons-SPD-Metamodel | EPL-2.0 | the SPD metamodel tables; Slingshot's adjustment transformations, embedded, three of them patched (§6) |
| Apache Commons Math 2.1 and 3.6.1 | Apache-2.0 (MersenneTwister: BSD-3-Clause) | MT19937, special functions, inverse CDFs (§2) |
| JScience 4.3.1 | BSD-style | interval arithmetic of the sliding-window utilisation (§3) |
| OpenJDK HotSpot | GPL-2.0-only | `simoxide-hotspot-math` only (§4) |
| Eclipse QVTo 3.11, classic Eclipse OCL | EPL-2.0 | the QVTo engine's library semantics, messages, test data (§8; [QVTo licensing](../qvto/licensing.md)) |

The Rust crates SimOxide depends on through Cargo are not vendored; their licences come with
their packages.

## Third-party test models

Three Palladio repositories that SimOxide's tests use declare **no licence**. SimOxide does not
distribute their files. They are git submodules under `third_party/`, pinned to the commits the
test material was made from (`third_party/README.md`, `LICENSES` §5):

| Submodule | Repository | Used by |
|---|---|---|
| `third_party/palladio-example-models` | PalladioSimulator/Palladio-Example-Models | `corpus/x_pem_*`, some `crates/simoxide-sim/tests/models/u_*` |
| `third_party/slingshot-e2e-tests` | PalladioSimulator/Palladio-Analyzer-Slingshot-E2E-Tests (archived) | `corpus/x_ss_*` |
| `third_party/slingshot-docs` | PalladioSimulator/Palladio-Documentation-Slingshot | Slingshot's minimal example: `reference/slingshot/models/spd_*`, the QVTo SPD corpus |

The repository only refers to these files:

- **Unmodified files** are relative symbolic links into the submodules, e.g.
  `corpus/x_pem_acquire/AcquireExample.repository -> ../../third_party/palladio-example-models/Acquire_Example/AcquireExample.repository`.
  SimOxide's own files (`run.json`, monitor repositories, `expected/` outputs, READMEs) stay next
  to them.
- **Modified files** (SimOxide's scaling policies in Slingshot's example, inserted ids, changed
  demands) are *derived*: `third_party/derived.txt` lists destination and upstream source,
  `third_party/derived/<destination>.ed` holds only SimOxide's change as an `ed` script. The
  destinations are git-ignored and generated by `simoxide-thirdparty setup`.
- **Full dumps** of these models (canonical EMF dumps, the Java runner's outputs) are stored as
  SHA-256 digests; the tests rebuild the dump from the submodule and compare the digest.

### Setup

```sh
git clone --recurse-submodules https://github.com/nk-coding/simoxide
# or, in an existing clone:
git submodule update --init
cargo run -q -p simoxide-thirdparty -- setup     # derive the modified files
```

`cargo build` needs none of this. Without the submodules, the tests that read their models skip
and pass, printing `skipped (third-party): <dir> needs the submodule third_party/<name> (not
checked out): git submodule update --init` (visible with `cargo test -- --nocapture`). The Rust
test harnesses derive the modified files on first use; scripts and the Java reference runners
need `setup` once.

Set **`SIMOXIDE_REQUIRE_THIRD_PARTY=1`** in CI and before regenerating expected outputs: missing
material then fails the tests instead of skipping them, so a checkout without submodules cannot
pass as green.

### `simoxide-thirdparty`

| Command | Effect |
|---|---|
| `setup` | derive every modified file from its upstream source and script (after `git submodule update --init`) |
| `check [PATH..]` | exit 1 with a message if third-party material under `PATH` (default: `.`) is unavailable |
| `verify` | every derived file equals source plus script, and every link resolves |
| `record DEST SRC` | store `DEST`, a modified copy of `SRC`, as `SRC` plus an `ed` script; writes the manifest line and the `.gitignore` entry |
| `digests DIR..` | write `DIR/third-party.digests` from the third-party dumps `DIR/*.jsonl` |

Run it as `cargo run -q -p simoxide-thirdparty -- <command>`. To change a derived file, edit the
destination, then `record` it and `verify`. To move a submodule to another commit, check the
commit out in it, run `verify`, and update the commit in `third_party/README.md`,
`crates/simoxide-thirdparty/src/lib.rs` and `LICENSES` §5. Details, including which oracle writes
which dump: `third_party/README.md`.

Ask the authors before redistributing these models.

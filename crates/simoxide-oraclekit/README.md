# simoxide-oraclekit

What SimOxide's three Java reference harnesses share. The harnesses run a Java program as the
oracle and compare SimOxide's or the QVTo engine's results with it:

| Harness | Java side | Rust driver |
|---|---|---|
| refsim, patched SimuLizar 5.2.2 | `reference/refsim` (`run`, `batch`) | `simoxide-testkit`: `sim::RefSim`, `simoxide-fuzz` |
| Eclipse QVTo runner | `crates/qvto/reference/runner/qvto-run` (`--batch`, `--bench`) | `qvto-testkit`: `java::JavaRun`, `java::record`; `qvto conform record`, `qvto fuzz`, `qvto bench --java` |
| Slingshot reference runner | `reference/slingshot/run.sh` (Equinox), `corpus-ss.sh`, `verify-det.sh`, `smoke.sh` | none (shell only); SimOxide's tests read `corpus-ss/*/expected/` |

The crate is test infrastructure (`publish = false`, EPL-2.0, one dependency: `flate2`). It knows
none of the harnesses' formats (traces, fixture bundles, verdicts); those stay with their
harnesses.

## Rust side

| Module | Contents | Used by |
|---|---|---|
| `batch` | `run(cmd, Stderr, Deadlines, is_result)`: a warm-JVM batch process; standard output read on a thread, a deadline per result line plus a start-up allowance, the process killed when one passes (`Outcome::timed_out`), standard error to a file or captured | `RefSim` (deadlines; the hung model is the first without a result line), `JavaRun` (no deadline: the runner has its own per-case timeout) |
| `java` | `seed_agent(repo)` (the shared seed agent's jar, built on demand; `ORACLEKIT_SEED_AGENT` overrides), `agent_option(jar, args)`, `ORDERED_QVTO`; `ensure_built(build.sh, outputs)`; `stack_excerpt`, `jvm_trouble` (thread creation failed, out of memory) | `JavaRun --det`, `qvto bench --java`, `RefSim` |
| `results` | `ResultSet` (case → file → text), `read_dir(dir, marker, files)` for `<dir>/<case>/<file>` layouts, `read_maybe_gz`/`write_maybe_gz` for expected files stored as `.gz` above 256 KiB, `differing_files` and `differences` between two result sets (`CaseFiles` lets a harness's own result type take part) | the corpus harness and the fuzzer (expected files), `qvto-testkit` (result directories, stability reruns, `record --check`) |
| `scratch` | `Scratch`: a unique directory per batch, removed on drop unless kept | `RefSim`, `CmdSim` |

## Java and shell side: `reference/oraclekit/`

- `seed-agent/`: the seed agent (`-javaagent`; its Java package keeps the name
  `simoxide.slingshot.seed`, which the Slingshot runner reaches by reflection). It seeds
  `Math.random()` and UUIDs and, with `ordered`, rewrites hash-ordered collections to insertion
  order (`ordered,qvto`: also QVTo's and OCL's). Loaded by refsim (patch P10), the Slingshot
  runner's `det` mode and the QVTo runner's `--det` recordings.
- `seed-agent.sh [OUT_DIR [ASM_JAR]]`: builds it. Each harness keeps its own copy built with the
  ASM of the product it runs (refsim: Palladio 5.2.2's ASM 9.4, `--release 17`; Slingshot runner
  and QVTo: the Slingshot install's ASM 9.9), so the jars are the ones the harnesses had before.
- `lib.sh` (POSIX sh, sourced): `oraclekit_exec_java`/`oraclekit_java` (the JVM under
  `nice -n $ORACLE_NICE`, default 19), `oraclekit_limited SECONDS CMD` (time and file-size
  limits), `oraclekit_first_diff A B FILES` (the first differing file, `.gz` expected files
  transparent). refsim, `qvto-run`, `run.sh`, `corpus-ss.sh`, `verify-det.sh` and `smoke.sh` use
  it; the older variables `QVTO_NICE` and `NICE` still set the niceness.

## Not shared, on purpose

- **Batch protocols.** refsim's `batch` (a corpus directory, one status line per model, a digest
  per run, `--check` against `expected/`), the QVTo runner's `--batch` (a case file, a fresh
  thread per case, `results.tsv`) and the Slingshot runner's `-runs N` (warm repeats of one
  model, `run-<i>/`) answer different questions; their line formats are what the Rust drivers
  parse. One Java protocol would change three runners and their outputs for no gain.
- **Java helpers.** The runners' JSON writers and digests define their committed formats and run
  in different class loaders (a flat classpath, Equinox bundles); sharing them would risk byte
  changes.
- **Fixture bundles.** Only QVTo's fixtures are bundles (one text file per suite,
  `qvto_testkit::fixture`). refsim's and Slingshot's expectations are directories (one file per
  output, diffable, read directly by other tests), so the bundle format stays in `qvto-testkit`;
  what is shared is the result set and its comparison.
- **Caches keyed by input hashes.** None of the harnesses has one: the caches are keyed by suite
  and case (`target/qvto-testkit/java/<suite>`) or are scratch directories.

See the documentation site, [Testing](../../docs/correctness/testing.md#reference-harnesses).

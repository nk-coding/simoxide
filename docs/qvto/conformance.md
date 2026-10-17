# Conformance

The reference is Eclipse QVTo 3.11 on classic Eclipse OCL, run headless by a Java reference
runner. The engine is compared with it at every level: parser, analyser, runtime primitives,
EMF layer and whole runs. The comparisons are strict, so equal means equal text, not just
"passes the test".

## Oracles

| Oracle | What it compares | Where |
|---|---|---|
| Eclipse QVTo test suite | 523 inventory cases from the suite's drivers, 494 of them runnable outside Eclipse: the drivers' checks plus a strict comparison with the Java runner | `qvto conform --suite eclipse`; `qvto-testkit` |
| SimuLizar and SPD corpus | SimuLizar's reconfiguration rules (23 cases on 12 rule files, each with a measurement above and below its thresholds) and Slingshot's SPD `MainTransformation` (1 case) | `--suite corpus` |
| SPD adjustment corpus | 47 runs of `MainTransformation` that cover every path of its call graph, including chained states | `--suite spd` |
| fuzz regressions | 24 minimised findings of the differential fuzzing campaigns | `--suite fuzz` |
| `det` Java runner | the same 589 cases in the Java runner with the Slingshot reference runner's seed agent (QVTo's and OCL's `HashSet`/`HashMap` become their linked variants): the oracle of ordered mode | `qvto conform --engine qvto-ordered` |
| Slingshot reference runner | the adjustments Slingshot recorded in `det` mode, replayed on one in-memory copy of the models: outcome and model summary of each | `qvto spd-replay` |
| Eclipse's compiler | problems of every unit of the corpus (1 102 files: Eclipse suite 1 047, SimuLizar 37, SPD 18): severity, unit, line, offset, length, message | `qvto-sema`: `tests/oracle.rs`, `examples/oracle_report.rs` |
| Eclipse's parser | parse problems of the same files, round trips, 3 306 token mutants, 182 edge cases | `qvto-syntax` tests |
| JDK and classic OCL primitives | hash codes, `HashSet`/`BagImpl`/`Dict` orders through resizes, collisions and removals, `toString` of collections, `Double.toString`, number parsing, OCL arithmetic, `String` operations, `Character` data | `qvto-runtime/tests/differential.rs` (needs the JDK), `tests/golden.rs` (committed samples) |
| EMF 2.41 | loading, XMI save and canonical dumps of the suite's model files and of the PCM side, random edit scripts, the suite's comparer | `simoxide-emf` tests ([Foundation crates](../development/foundation.md#verification)) |
| external corpora | 2 402 `.qvto` files of 233 sources (Palladio repositories, Eclipse samples, GitHub): compile problems and runs against the Java runner | `qvto fuzz corpus`; `reference/external/` (not committed) |
| differential fuzzing | generated expressions and transformations over generated models, run in both | `qvto fuzz expr`, `qvto fuzz prog` |
| robustness fuzzing | no panic, hang or abort of parser, analyser, compiler and VM on mutated input | `qvto fuzz robust`, `qvto/tests/robust.rs` |

### The Java reference runner

`crates/qvto/reference/runner` is a headless Eclipse QVTo on a flat classpath, without OSGi,
built with `javac` against the QVTo 3.11.0 jars. It sets up what the Eclipse test environment
provides: EMF's `plugin.xml` registrations, the test bundle's Java classes (blackboxes, generated
packages, the suite's comparer), import resolution in the bundles' source containers, and the
drivers' resource set, parameter binding and log. One JVM runs a whole case file (about 8 s for
the Eclipse cases). Per case it writes the result, a canonical dump of the output extents, the
log, the stack trace, the compile problems and an identity-hash tape.

All 494 runnable Eclipse cases and all corpus cases pass Eclipse's own checks on the Java side.

### Verdicts

Each runnable case gets two verdicts:

- the **drivers' checks**: what the Eclipse test asserts (severity and code, expected models with
  the suite's comparer, log, stack trace, problem counts, trace records). A case that passes only
  these is `pass-lenient`;
- the **strict comparison** with the Java runner's fixture: load result, severity, code and
  exception; canonical dumps (order, values, reference targets); log; stack trace; problems by
  severity and line. A case that passes this is `pass`.

`java-fails-equal` means the Java runner fails the drivers' checks itself, and the engine's
outcome equals it strictly. Such cases are counted apart, not as passes.

### Identity hashes

Some outputs depend on Java identity hashes: the order of sets of objects, and `Class@hash` in
logs. `qvto conform record` therefore runs the Java side five times, with HotSpot's default
hashing, constant hashes and sequential hashes. Outputs that differ between these runs are marked
*unstable* and are not compared by default.

Each recording also writes a *tape*: the identity hash of every object of the inputs, outputs and
metamodels, keyed like the canonical dump, plus the case thread's first hashes. HotSpot's hashes
form a per-thread xor-shift sequence, so the thread's state can be recovered from them. Compat
mode then gives the objects that existed before the run their recorded hashes. Objects the run
creates take the following values of the sequence, in the order Java requests them, including the
hashes Java draws as side effects. With `--compare-unstable` the scoreboard also compares the
unstable outputs.

## Results

Current tree, `cargo conform --suite all` and its variants:

| Suite | Cases | Runnable | compat: strict | fast: drivers' checks / strict | ordered vs `det`: strict |
|---|---:|---:|---:|---:|---:|
| Eclipse test suite | 523 | 494 | 494 | 494 / 494 | 488 + 6 `java-fails-equal` |
| SimuLizar and SPD corpus | 24 | 24 | 24 | 24 / 24 | 24 |
| SPD adjustment corpus | 47 | 47 | 47 | 47 / 47 | 47 |
| fuzz regressions | 24 | 24 | 24 | 24 / 19 | 24 |
| **total** | **618** | **589** | **589** | **589 / 584** | **583** + 6 |

- **Compat mode** equals the Java runner on every runnable case. The SPD adjustment corpus also
  passes with `--compare-unstable` (47 of 47).
- **Fast mode** passes every driver's check. The 5 fuzz cases that are not strictly equal print
  hash-ordered collections in their logs (the fuzz suite runs with constant identity hashes on
  both sides, so Java's order there is the bucket order).
- **Ordered mode** equals the `det` runner on every case. In 6 `bug467600_*` cases the `det`
  rule turns OCL `Set`s into `LinkedHashSet`s, which classic OCL takes for OrderedSets. Those
  cases then fail their own assertions in Java, and ordered mode fails them identically.
- **SPD replay**: ordered mode equals the Slingshot reference runner on 13 of 13 recorded
  adjustments.
- **Analyser**: the problems of all 1 102 units equal Eclipse's exactly: 2 304 problems
  (945 errors, 1 359 warnings), none missing, none extra.
- **Parser**: 1 102 of 1 102 files give Eclipse's problems; the round trip holds on all
  error-free files; accept and reject agree on all 3 306 mutants.
- **External corpora** (recorded at Q7, 2026-10-17): of 1 516 compile cases, 1 263 give
  exactly Eclipse's problems. The others are cascades in files whose metamodels are missing, and
  LPG's repair messages. Of 410 run cases, 371 give equal problems. Of the 76 that compile
  without errors in Java, 71 run equal to the Java runner. The other 5 take QVTo's trace model
  or XML feature maps as input.
- **Fuzzing** (Q7): about 2.0 million generated expression statements in 51 000
  transformations and 56 700 generated transformations, in three bounded rounds. The findings
  went from 396 value and 40 model differences to none in the last round; each fixed class of
  difference is kept as a regression case. Robustness: 2.0 million mutated inputs in a
  30-minute campaign, with no open finding.

## Known differences from Eclipse

What the engine does not reproduce. Each needs a Java-specific emulation that real
transformations hardly hit, or is out of scope:

| Point | Eclipse | Here |
|---|---|---|
| set, bag and dict order in fast and ordered mode | Java's `HashMap` bucket order, identity hashes for objects | insertion order (by design: the mode's definition) |
| `Class@hash` in logs, without a tape | the JVM's identity hash | compat: HotSpot-like sequence, not equal to a particular JVM run; fast and ordered: derived from the object's key |
| an effectful `sortedBy` body | evaluated twice (a classic OCL quirk) | compat and ordered: twice; fast: once |
| `Double.toString` | depends on the JDK; JDK 19 changed it | JDK 19 and later only (the runner uses JDK 21) |
| problems after a syntax error | LPG's repair search | single-token repair, then resynchronisation: the first error is found, later errors and the first span can differ |
| strings with lone surrogates | Java keeps them | stored as UTF-8 with `?`, so two different lone surrogates compare equal |
| regular expressions on empty matches | Java splits surrogate pairs | matches on code points |
| generated metamodels of the Java runner (QVTo's trace model, XML feature maps of XSD-derived metamodels) | generated Java classes | dynamic EMF |
| Java blackboxes | run in the JVM | not supported; blackboxes are Rust ([Usage](./usage.md#blackboxes)) |
| QVTo 3.10.7, the version in SimuLizar 5.2.2 | a completed run reports `OK` also after a failed non-fatal `assert` | the engine follows 3.11, which raises the severity; SimOxide's host counts such a rule as applied, as 5.2.2 does ([MC-7.3](../spec/model-changes.md#mc-7-reconfiguration-rules-qvto)) |

The rationale for each of these is in the decision log of `crates/qvto/DESIGN.md` (§12, Q1b-4
and Q7-7). The semantics notes on Eclipse QVTo and OCL, with file and line references to the
sources, are in `crates/qvto/reference/notes/eclipse-semantics.md`.

## Running the checks

[Development](./development.md#test-gates) lists the commands and the reference material each
needs. The Java side's results are committed (`qvto-testkit/fixtures/`), so no Java is needed to
compare. The scoreboard does read the Eclipse checkout, where the suite's cases live, and the
PCM metamodels in `target/pcm` ([Reference material](./development.md#reference-material)).
Recording fixtures again also needs the Java runner.

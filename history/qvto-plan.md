# Plan (qvto-rs)

> History: the milestone log of the QVTo engine while it was the separate repository qvto-rs
> (formerly `crates/qvto/PLAN.md`), not maintained. Paths are qvto-rs's: `crates/qvto-x` is now
> `crates/qvto/qvto-x`, `qvto-model` and `qvto-emf` are `crates/foundation/simoxide-emf-core` and
> `simoxide-emf`, and the adapter `qvto-simoxide` was replaced by SimOxide's own graph (merge step
> M6). The current documentation is the QVTo section of the site (`docs/qvto/`).

Milestones with exit criteria. Details and rationale are in `DESIGN.md` (now `crates/qvto/DESIGN.md`), "Milestones".
Status values: done, next, open, parked.

| | Milestone | Status | Can run in parallel with |
|---|---|---|---|
| Q0 | Research, resources, design, skeleton | done | — |
| Q1a | Model API and own Ecore/XMI layer (`qvto-model`, `qvto-emf`) | done | Q1b, Q1c, Q1d |
| Q1b | Lexer and parser (`qvto-syntax`) | done | Q1a, Q1c, Q1d |
| Q1c | Runtime values and Java-compatible primitives (`qvto-runtime` core) | done | Q1a, Q1b, Q1d |
| Q1d | Reference harness: runner batch mode, canonical dump, conformance-runner skeleton (`qvto-testkit`) | done | Q1a, Q1b, Q1c |
| Q2 | Type system, standard library signatures, semantic analysis (`qvto-types`, `qvto-sema`) | done | Q3a |
| Q3a | Bytecode and VM core: expressions, imperative OCL, helpers, queries, mappings, objects, log, assert; the engine under the harness | done | Q2 (on hand-written IR) |
| Q3b | End-to-end pipeline; SimuLizar rule corpus | open | — |
| Q4 | Full QVTo: mappings with traces, resolve, late resolve, disjuncts, inherits, merges, intermediate data, deepclone, libraries, imports, extends, access, Dict/List, try/raise, configuration properties, `transform()`, blackboxes | done | Q5a |
| Q5a | Compat hardening: identity hashes, Java HashMap order, error and log texts, stack traces | done | Q4 (late part), Q5b |
| Q5b | Fast mode and performance: benchmarks, profiling, optimisations | done | Q5a |
| Q6 | SimOxide integration preparation: adapter over `raw::Graph`, change journal | done | Q5 |
| Q7 | Hardening: external corpora, differential and robustness fuzzing, resource limits, documentation | done | Q6 |
| SPD-Q1 | SPD adjustment corpus for every path of `MainTransformation` (DESIGN-SPD Q1) | done | SPD-Q2 |
| SPD-Q2 | Ordered mode: the Slingshot `det` runner's insertion order for `Set`s, EPL-only (DESIGN-SPD Q2) | done | SPD-Q1 |

## Exit criteria

- **Q1a**: every `.ecore` and model file of the Eclipse test suite and of the SimuLizar/SPD
  corpora loads; XMI save of a loaded model equals EMF's save byte for byte on the suite's
  models; the canonical dump equals the Java runner's dump; the model-API contract tests pass.
  Done 2026-10-16 (`crates/qvto-emf/README.md`, "Verification"; EMF 2.41 via
  `crates/qvto-emf/oracle`): 544 of the suite's 546 model files load with EMF's errors, save
  byte for byte and dump identically (2 AST models excluded, DESIGN Q1a-10); 640 PCM-side files
  (SimOxide corpus, default models, Slingshot/SPD examples, SPD and SimuLizar metamodels) the
  same; 300 random model-API scripts (15 321 operations, 1 200 saves of created models) give
  EMF's results and states; the lenient comparer gives the Java comparer's verdict on 1 662 of
  1 663 pairs (1 where Java throws); 20 contract checks pass on both metamodel paths. The Java
  dump (`CanonDump.java`) is ready for the runner's `--dump` (Q1d).
- **Q1b**: all 1 046 `.qvto` files of the suite and the 55 corpus files parse; files with
  expected syntax errors report errors; spans are exact; unparse/reparse round trip.
  Done 2026-10-16 (`crates/qvto-syntax/README.md`, "Verification"): on all 1 102 files the
  lexer and parser report exactly what Eclipse's parser reports (11 files with syntax errors,
  33 with parser warnings, same offsets, lengths and messages); round trip on all 1 091
  error-free files; same accept/reject verdict as Eclipse on 3 306 token mutants and 182
  edge cases; 121 MB/s (Eclipse warm: 11 MB/s). The 93 error cases of `TestQvtParser` contain
  10 with syntax errors; the other 83 are semantic and move to Q2.
- **Q1c**: values, the four OCL collection kinds, Java `HashMap`/`HashSet` iteration order (with
  copy-constructor capacities, removals, treeified bins noted), `String.hashCode`,
  `Double.toString`, `Integer` arithmetic and string operations agree with golden values dumped
  from Java.
  Done 2026-10-16 (`crates/qvto-runtime/README.md`, "Verification"): differential tests against
  JDK 21 and Eclipse OCL's own classes agree on every case: 1.5k–15k collection scripts per
  category (growth across resize thresholds, treeified bins with both removal kinds, all
  `CollectionUtil`/QVTo collection operations, `DictionaryImpl`), 20k hash codes, 210k
  `Double.toString` and 103k `Float.toString`, 60k number parses, 40k OCL arithmetic
  expressions through the classic evaluator, 48k string operations, all 65 536 code units and
  149k code points of character data; 12 993 samples are committed as goldens.
- **Q1d**: the Java runner runs a whole list of cases in one JVM, writes canonical dumps and logs;
  `qvto-testkit` reads `reference/inventory/eclipse-tests.tsv` and reports per-case status.
  Done 2026-10-16 (`crates/qvto-testkit/README.md`): the 523 inventory rows are cases
  (`cases/eclipse.cases`, generated from the drivers' sources): 494 runnable (all 309
  `TransformTests` cases, 154 parser cases, 31 of the other drivers), 29 classified (16 API, 4
  JDT, 3 Ant, 2 perf, 2 workspace, 1 OSGi, 1 PDE). The Java runner's batch mode runs them in one
  JVM in about 8 s, emulating the test bundle's environment; all 494 pass the drivers' checks on
  the Java side. 24 SimuLizar/SPD cases (12 SimuLizar rule files, the SPD `MainTransformation`)
  run and pass too. Results are committed fixtures (two 0.7 MB bundles); identity-hash tapes
  and XMI stay in a cache. `qvto conform` runs an engine (now a stub) and prints the scoreboard;
  replaying Java's own outputs through the Rust side reproduces Java's verdicts and dumps on all
  494 cases.
- **Q2**: the 49 `compile_ok` parser cases and all positive transformation cases type-check; the
  93 error cases report errors at the annotated spans.
  Done 2026-10-16 (`crates/qvto-sema/README.md`, "Verification"): against Eclipse's full-compile
  diagnostics of all 1 102 corpus files, 1 096 files get exactly Eclipse's problems (severity,
  unit, line, offset, length, message), all 37 SimuLizar and 18 SPD files included; all 2 304
  of Eclipse's problems are reproduced, with 8 extra; whether a file has errors agrees on all
  files (539 of the 541 problem-free files have no problems at all); 115 of the 117
  `TestQvtParser` count checks pass; the IR of all 969 error-free compilations passes the
  consistency checks; the whole corpus analyses in 0.40 s (Eclipse warm: 1.77 s).
- **Q3a**: the bytecode (register instructions of at most 8 bytes, inline iterators, constant
  pool, per-instruction instruction pointers for stack traces) and the VM (helpers, queries,
  constructors, contextual operations with virtual dispatch, mappings with Eclipse's protocol,
  traces and `resolve`, objects in extents, QVTo's assignment rules, `log`, `assert`, `raise`,
  `try`, the standard library over `qvto-runtime`, compat and fast mode) pass unit tests on
  hand-written IR; the conformance adapter (`qvto conform --engine qvto`) runs the whole
  pipeline; some Eclipse cases pass strictly; a first comparison with the warm Java runner.
  Done 2026-10-17 (`crates/qvto-vm/README.md`, "Verification"): 424 of the 494 runnable
  Eclipse cases equal the Java runner strictly (85.8 %; 425 pass Eclipse's checks, fast mode
  the same); `TestQvtInterpreter` 262 of 309 (85 %, the Q3b target is 50 %), `TestQvtParser` 137
  of 154. Of the 70 others: 39 do not compile, almost all for Q4 (Java blackbox libraries such
  as `Strings`, `org.bar.Foo`, `emf.tools`; the QVT and XMLType metamodels missing from the
  registry); 17 parser cases with other diagnostics (Q2); 12 wrong outcomes (blackboxes,
  `Model::copy`/`createEmptyModel`, `raise 'msg'` and `insertAt` overloads from the analyser,
  opposites of intermediate classes); 1 model (an implicit-iterator `iterate` the analyser
  rejects). Corpus: 22 of 24 strictly, 23 pass (SPD's main needs its blackbox library). Run
  times against the warm Java runner (median of repeated runs in one JVM): suite cases
  0.03–0.05 ms vs 2.6–9 ms, SimuLizar rules 0.04–0.07 ms vs 4.7–7.7 ms; compile 0.1–0.5 ms;
  micro benchmarks 2.2× (collections) to 7× (arithmetic loop) and 2 900× (`including` in a
  loop) faster.
- **Q3b**: SimuLizar's 37 `.qvto` files compile; the 22 rule files run on the corpus PCM models
  and match the Java runner; at least 50 % of the 309 transformation cases pass.
- **Q4**: Slingshot's SPD transformations run and match the Java runner; at least 85 % of the 309
  cases pass in compat mode.
  Q4a (blackboxes, metamodels, `Model` operations, EOperations) done 2026-10-17
  (`crates/qvto/README.md`, "Blackboxes"; `crates/qvto-vm/README.md`, "Verification"; DESIGN §12
  Q4a-1 to Q4a-8): native blackboxes (signatures as data, Rust implementations by key; library
  operations, `blackbox` operations declared in QVTo, blackbox mappings, constructors and
  transformations), QVTo's `Strings`/`emf.tools` and the test bundle's Java libraries ported,
  Palladio's signatures and `getSimulationTime` for hosts; the QVT and XMLType metamodels;
  `Model::copy`/`createEmptyModel`; Ecore's EOperations and host EOperations; EOperations in
  the CLI. The scoreboard went from 424 to 482 of the 494 runnable Eclipse cases strictly equal
  to the Java runner (483 pass Eclipse's checks; fast mode the same), `TestQvtInterpreter` from
  262 to 300 of 309 (97.1 %), `TestQvtParser` from 137 to 152 of 154; corpus 23 of 24 strictly,
  24 pass: **SPD's `MainTransformation` equals the Java runner**. The exit criteria are met.
  Q4b (analyser gaps, intermediate classes in the store, remaining VM semantics) done 2026-10-17
  (DESIGN §12 Q4b-1 to Q4b-11; `crates/qvto-vm/README.md`, "Verification"): the analyser fills
  model parameters (with a flag for generated legacy ones) and virtual tables, types
  `raise 'msg'`, accepts `iterate` with an implicit iterator, and reports exactly Eclipse's
  problems on all 1 102 corpus files (validator problems in object bodies, LPG's repairs, a
  Java unit loaded with the importing module's packages); intermediate class instances are store
  objects with EMF semantics, static features; OCL bodies and derivations of metamodels;
  incremental update with an input trace; per-instance properties of nested transformations;
  List mutators and two-argument OCL operations as Eclipse; `sortedBy` twice for effectful
  bodies; harness fixes (`extent:/N` numbering, runner thread stacks). **All 494 runnable
  Eclipse cases and all 24 corpus cases equal the Java runner strictly** (from 482 and 23; fast
  mode the same; `TestQvtInterpreter` 309 of 309, `TestQvtParser` 154 of 154), which also meets
  Q5a's case criterion.
- **Q5a**: at least 95 % of the runnable cases (excluding JDT/PDE, Ant, debugger cases) pass in
  compat mode, byte-identical output and log against the Java runner on all of them.
  Done with Q4b (2026-10-17): all 494 runnable Eclipse cases and all 24 corpus cases equal the
  Java runner strictly in compat mode; Q7's differential fuzzing (with identity hashes made
  constant on both sides) found and fixed the remaining value and order differences it could
  reach (DESIGN §12 Q7-6).
- **Q5b**: fast mode passes the same cases under the order-insensitive comparison; performance
  targets of `DESIGN.md` met.
  Done 2026-10-17 (`crates/qvto-cli/README.md`, "Benchmark suite" and "Results"; DESIGN §7.2 and
  §12 Q5b-1 to Q5b-11): `qvto bench` runs 50 workloads (SimuLizar rules on the corpus models
  ×1/×10/×100 and as series on a changing model, SPD, SimpleUML→RDB ×1/×10/×100, Ecore copies
  and queries up to 42 500 objects, micro benchmarks; the suite with `--suite`) in both modes and
  in the reference runner's new benchmark mode (warm: one JVM; cold: a JVM per workload), with
  sampling profiles (`--profile`, `pprof`). Optimisations: Ecore's derived lists cached and
  memoised, lists read in one call, O(1) feature slots, `+=` of objects and `sortedBy` without
  quadratic work, Fx hashing and dense tables in the VM, per-program tables, in-place loops,
  `LinkedMap` on a SwissTable, operation lookup by name first. Against main before Q5b:
  `deepclone` workloads 11–21×, `sortedBy` 200×, list building 60×, Ecore queries 2.4–2.6×,
  copies 1.4–2×, rule runs 1.1–1.4×. Against warm Eclipse: rule runs 4–11× (series 4–18×), SPD
  4.5–11×, SimpleUML→RDB 13×/9×/2.7×, Ecore copy 7×/2.7×/2.1×, micro 2.8–23×, suite cases median
  5.8×; against cold Eclipse 100–400× for rule-sized runs; compile 0.1–0.9 ms per rule file and
  7–8 ms for SPD. Fast mode is 1.0–1.4× compat (hash-heavy workloads) and passes all cases. The
  ≥ 10× target holds for small models only; realistic targets are in DESIGN §7.2. Conformance
  unchanged: 518 of 518 runnable cases equal the Java runner strictly in compat and fast mode;
  the analyser oracle 1 102 of 1 102.
- **Q6**: SimuLizar rules run on an in-memory SimOxide `raw::Graph` through the adapter, with the
  change journal; the SimOxide-side work list is written down.
  Done 2026-10-17 (`crates/qvto-simoxide/README.md`, `docs/simoxide-integration.md`, DESIGN §12
  Q6-1 to Q6-8): `qvto-simoxide` (a crate of its own, depending on SimOxide's branch
  `qvto-adapter` by path) has the registry of SimOxide's tables with SimOxide's ids (equal to the
  registry of the product's `.ecore` files in every property), a `ModelStore` over
  `raw::Graph` (in place, EMF's semantics ported from `qvto-emf`), objects of other metamodels
  and metamodel objects (`ResourceSet::materialize`) in a session, and the change journal. The
  contract suite passes with objects in both homes (40 tests); 300 random PCM scripts (23 914
  operations) give `qvto-emf`'s results and states; all 24 SimuLizar/SPD corpus cases on models
  loaded by SimOxide's loader equal the Java runner and the `qvto-emf` path; per rule execution
  on MediaStore 3-8 µs (6 containers) to 0.86 ms (1 000 containers), 0.9-1.1x the `qvto-emf`
  store with the journal on; the runtime flow (measurement, rule with `getSimulationTime`,
  journal, stub syncer, rebuilt SimOxide model) is `examples/runtime_flow.rs`. SimOxide's side
  (option D) is planned at 6-9 weeks. SimOxide changes on `qvto-adapter`: extension tables in
  `gen_meta.py`/`meta/generated_ext.rs`, `raw::Graph::add_object`.
- **Q7**: 24 h of parser fuzzing without a crash; step and memory limits enforced.
  Done 2026-10-17 (DESIGN §12 Q7-1 to Q7-7; `crates/qvto-testkit/README.md`, "Fuzzing"):
  - External corpora: 2 402 `.qvto` files of 233 sources (Palladio repositories, Eclipse
    samples, GitHub), 1 809 not in the suite, in `reference/external/` (not committed; sources,
    revisions and licences in its `SOURCES.tsv`). 1 516 compile cases (`cases/external.cases`):
    compile problems exactly equal to Eclipse's on 392 at first, 1 263 after the fixes (11 with
    another verdict; the rest are cascades in files with missing metamodels and LPG's repair
    messages); 410 run cases (`cases/external-run.cases`, inputs found in the sources): problems
    equal on 371, and of the 76 that compile without errors in Java 71 run equal to the Java
    runner (the others: QVTo's trace model as input, XML feature maps).
  - Differential fuzzing (`qvto fuzz expr|prog`, three bounded niced rounds): 2.0 M generated
    expression statements in 51 000 transformations and 56 700 generated transformations over
    generated models. Value/model findings per round: 396 / 40, then 12 / 7, then 1 / 0 (the
    last ones lone surrogates, documented); the remaining analyser findings are LPG's recovery
    after syntax errors. Classes fixed: type tests and casts, numeric promotion and conversion,
    strings and regular expressions, collection copies and aliasing, extents and roots,
    printing of objects, types and numbers, iteration scoping, inheritance, argument evaluation
    on undefined sources, common supertypes and problems on unknown types. Each minimised and
    kept: `cases/fuzz.cases` (24 cases, suite `fuzz` of `qvto conform`) and
    `qvto-sema/tests/data/q7` (23 files with Eclipse's problems).
  - Robustness fuzzing (`qvto fuzz robust`, own mutational fuzzer; `cargo fuzz` needs
    nightly): 2.0 M mutated inputs in a 30-minute campaign of 3 workers, besides the shorter
    ones during development; the panics (nameless nodes after syntax errors) and the one hang
    (quadratic string growth without step charging) are fixed and kept in
    `crates/qvto/tests/robust.rs`; limits on collection size, string length and native stack
    besides the step budget (Q7-4). The 24 h criterion was replaced by these bounded campaigns
    on the shared machine.
- **SPD-Q1** (DESIGN-SPD phase Q1): a corpus of SPD adjustments covering every path of
  Slingshot's `MainTransformation`; strict match with the Java runner.
  Done 2026-10-17 (DESIGN §12 SPD-Q1-1 to SPD-Q1-4; `qvto-testkit/README.md`, "SPD corpus";
  DESIGN-SPD §5.5 "Q1 as built"): 47 cases (suite `spd`), 73 of the 105 operations of the 17 units
  reached, every one in `MainTransformation`'s call graph; 47 of 47 strictly equal in compat mode,
  also on the outputs that vary between Java runs. Fixed: identity hashes Java draws as side
  effects (extent additions, `deepclone`), tapes keyed before the run, the batch's first seed,
  the sequence search depth, lazy proxies (`resolve-all`). Scoreboard: 589 of 589 (542 + 47).
- **SPD-Q2** (DESIGN-SPD phase Q2): an insertion-order policy for the Slingshot-exact mode, EPL
  only; SPD corpus equal to the `det` Java runner incl. container and link order.
  Done 2026-10-17 (DESIGN §6.4, §12 SPD-Q2-1, SPD-Q2-2): `Mode::Ordered`; against the Java runner
  with the `det` seed agent all 589 cases equal (6 where that runner fails the Eclipse checks
  itself); against the Slingshot `det` runs all 13 recorded adjustments (`qvto spd-replay`).

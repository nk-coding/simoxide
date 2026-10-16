# Plan

Milestones with exit criteria. Details and rationale are in `DESIGN.md`, "Milestones".
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
| Q4 | Full QVTo: mappings with traces, resolve, late resolve, disjuncts, inherits, merges, intermediate data, deepclone, libraries, imports, extends, access, Dict/List, try/raise, configuration properties, `transform()`, blackboxes | next (Q4a done) | Q5a |
| Q5a | Compat hardening: identity hashes, Java HashMap order, error and log texts, stack traces | open | Q4 (late part), Q5b |
| Q5b | Fast mode and performance: benchmarks, profiling, optimisations | open | Q5a |
| Q6 | SimOxide integration preparation: adapter over `raw::Graph`, change journal | open | Q5 |
| Q7 | Hardening: parser fuzzing, resource limits, documentation | open | Q6 |

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
  the CLI. The scoreboard went from 424 to 481 of the 494 runnable Eclipse cases strictly equal
  to the Java runner (482 pass Eclipse's checks; fast mode the same), `TestQvtInterpreter` from
  262 to 299 of 309 (96.8 %), `TestQvtParser` from 137 to 152 of 154; corpus 23 of 24 strictly,
  24 pass: **SPD's `MainTransformation` equals the Java runner**. The exit criteria are met; the
  Q4b part (analyser fixes, intermediate classes in the store, remaining VM semantics) is in
  progress. Open from Q4a: OCL bodies of EOperations and OCL derivations (`oclannotation`), a
  Java blackbox Eclipse only loads with the run's registry (`bug565747`), incremental update
  traces (`bug463572`).
- **Q5a**: at least 95 % of the runnable cases (excluding JDT/PDE, Ant, debugger cases) pass in
  compat mode, byte-identical output and log against the Java runner on all of them.
- **Q5b**: fast mode passes the same cases under the order-insensitive comparison; performance
  targets of `DESIGN.md` met.
- **Q6**: SimuLizar rules run on an in-memory SimOxide `raw::Graph` through the adapter, with the
  change journal; the SimOxide-side work list is written down.
- **Q7**: 24 h of parser fuzzing without a crash; step and memory limits enforced.

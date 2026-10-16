# Plan

Milestones with exit criteria. Details and rationale are in `DESIGN.md`, "Milestones".
Status values: done, next, open, parked.

| | Milestone | Status | Can run in parallel with |
|---|---|---|---|
| Q0 | Research, resources, design, skeleton | done | — |
| Q1a | Model API and own Ecore/XMI layer (`qvto-model`, `qvto-emf`) | done | Q1b, Q1c, Q1d |
| Q1b | Lexer and parser (`qvto-syntax`) | done | Q1a, Q1c, Q1d |
| Q1c | Runtime values and Java-compatible primitives (`qvto-runtime` core) | done | Q1a, Q1b, Q1d |
| Q1d | Reference harness: runner batch mode, canonical dump, conformance-runner skeleton (`qvto-testkit`) | next | Q1a, Q1b, Q1c |
| Q2 | Type system, standard library signatures, semantic analysis (`qvto-types`, `qvto-sema`) | open | Q3a |
| Q3a | Bytecode and VM core: expressions, imperative OCL, helpers, queries, objects, log, assert | open | Q2 (on hand-written IR) |
| Q3b | End-to-end pipeline; SimuLizar rule corpus | open | — |
| Q4 | Full QVTo: mappings with traces, resolve, late resolve, disjuncts, inherits, merges, intermediate data, deepclone, libraries, imports, extends, access, Dict/List, try/raise, configuration properties, `transform()`, blackboxes | open | Q5a |
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
- **Q2**: the 49 `compile_ok` parser cases and all positive transformation cases type-check; the
  93 error cases report errors at the annotated spans.
- **Q3b**: SimuLizar's 37 `.qvto` files compile; the 22 rule files run on the corpus PCM models
  and match the Java runner; at least 50 % of the 309 transformation cases pass.
- **Q4**: Slingshot's SPD transformations run and match the Java runner; at least 85 % of the 309
  cases pass in compat mode.
- **Q5a**: at least 95 % of the runnable cases (excluding JDT/PDE, Ant, debugger cases) pass in
  compat mode, byte-identical output and log against the Java runner on all of them.
- **Q5b**: fast mode passes the same cases under the order-insensitive comparison; performance
  targets of `DESIGN.md` met.
- **Q6**: SimuLizar rules run on an in-memory SimOxide `raw::Graph` through the adapter, with the
  change journal; the SimOxide-side work list is written down.
- **Q7**: 24 h of parser fuzzing without a crash; step and memory limits enforced.

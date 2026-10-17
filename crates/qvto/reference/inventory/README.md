# Eclipse QVTo test-suite inventory

Inventory of `eclipse-qvto/tests` at commit `8c01c4f` (2026-09-15). It is the conformance
oracle for qvto-rs.

- `eclipse-tests.tsv`: one row per test case (523 rows). Columns: `test_id`,
  `driving_test_class`, `qvto_path` (relative to the clone), `inputs`, `expected_outputs`,
  `expected_kind`, `needs_java_blackbox`, `metamodels`, `feature_tags`, `notes`.
- `build_inventory.py`: regenerates it: `python3 -I build_inventory.py <clone> <out.tsv>`.
  Feature tags and metamodels come from regexes over every `.qvto` in a case folder (comments
  and strings removed first), so they over-count. Rows for the API-level drivers (callapi, ant,
  perf, api) were entered by hand and are approximate.

`expected_kind`:

| Value | Meaning | Rows |
|---|---|---|
| `model` | output models compared with `expected.*` | 238 |
| `assert` | no expected model; the transformation checks itself with `assert fatal` | 78 |
| `none` | must run without error | 15 |
| `log` | text log compared exactly | 6 |
| `stack` | printed QVT stack trace compared exactly | 8 |
| `trace` | number of trace records checked | 2 |
| `error` | compile errors or warnings expected (count, and for 84 cases the exact span) | 115 |
| `compile_ok` | parser test that must compile with no errors and no warnings | 51 |
| `api`, `perf` | Java API tests | 10 |

## How the suite is organised

All drivers are in `tests/org.eclipse.m2m.tests.qvt.oml/src/org/eclipse/m2m/tests/qvt/oml/`.

- `transform/TransformTests.java#createTestData()` defines 309 transformation cases
  (`FileToFileData`, `FilesToFilesData`, project variants). Default input `in.ecore`, default
  expected output `expected.ecore`. Case folders: `parserTestData/models/<name>/`.
- The same 309 cases are run by several drivers: `transform/TestQvtInterpreter` (the main one,
  workspace-based, runs the compiled AST XMI), `callapi/TestQvtExecutor` (in-memory),
  `callapi/TestQvtStandaloneExecutor` (no workspace, `file:` URIs; the model for our harness),
  `transform/javaless/JavalessQvtTest` (about 255 cases on a dynamic copy of Ecore), the walker
  and the debugger drivers.
- Other execution drivers: `TransformManyModelTests` (7), `TestFailedTransformation` (5, log must
  equal `qvtTraceLog.stack`), `TestStackTrace` (6), `TestInvalidConfigProperty` (3),
  `TestBlackboxLibContext` (1), `traces/TestTraceFile` (2), `TestIncorrectTransformation` (2).
- Parser tests: `TestQvtParser` with `ParserTests.TestData(dir, errCount[, warnCount])`, 155
  entries plus 4 subclasses; sources in `parserTestData/sources/<dir>/`. 84 cases mark exact
  problem spans inline: `/*<error>*/ … /*</error>*/`, `/*<warn>*/ … /*</warn>*/`
  (`util/ProblemSourceAnnotationHelper`). Messages are not checked.
- `unparserTestData/` holds 386 copies of `.qvto` files for AST-to-text round trips.

### The comparer

`api/framework/comparator/TreeComparator.java` with `comparator/emf/*TreeNode.java`: a tree edit
distance that must be 0.

- Resource roots: matched by position.
- Containment lists: order does not matter (values sorted by EClass name, then attributes).
- Attributes: all of `eAllAttributes`, derived included; many-valued attributes order-sensitive;
  unset equals default; null differs from "".
- EClass and EReference matched by name; nsURI and XMI ids ignored.
- Non-containment references: counts equal, targets matched shallowly (EClass name and
  attribute values). A wrong target that looks the same passes.

Our harness reproduces this comparer as the "Eclipse-equivalent" check, and adds a strict
canonical comparison (order-sensitive, identity of reference targets) for differential tests
against the Java runner.

## Counts

- 1 046 `.qvto` files under `tests/`: 417 in `parserTestData/models`, 184 in
  `parserTestData/sources`, 386 unparser copies, the rest in `deployed`, `apiTestData`, ant,
  editor and builder data.
- Of the 309 transformation cases: 224 compare models, 76 self-check with asserts, 9 only run.
  286 use only Ecore; 23 use other metamodels (SimpleUML/RDB from the samples plugin, testqvt and
  generics from the test bundle, case-local `.ecore`, the QVTo AST metamodel). 20 need Java
  blackboxes from the test bundle; 13 more use only the built-in `Strings`/`emf.tools`
  libraries. 6 pass configuration properties.
- Parser cases: 159. 93 expect errors, 17 only warnings, 49 must compile cleanly.

## Feature coverage

Rows of the execution suite (343) / parser suite (159).

| Feature | Exec | Parse |
|---|---|---|
| mapping | 227 | 88 |
| init / population / end sections | 128 / 11 / 26 | 50 / 10 / 13 |
| when / where | 15 / 2 | 5 / 1 |
| disjuncts / inherits / merges / abstract | 7 / 4 / 4 / 5 | 7 / 4 / 2 / 4 |
| legacy `mapping main(...)` entry | 132 | 28 |
| object expression / `new` / constructor | 191 / 25 / 9 | 46 / 3 / 4 |
| resolve / resolveIn / invresolve / late resolve | 28 / 27 / 11 / 9 | 3 / 7 / 1 / 1 |
| deepclone / clone | 9 / 5 | 0 |
| intermediate class / property | 14 / 14 | 6 / 4 |
| library / import / access / extends | 26 / 91 / 15 / 28 | 27 / 35 / 7 / 6 |
| blackbox (declared / Java unit / built-in libs) | 10 / 19 / 16 | 9 / 17 / 2 |
| helper / query / contextual operation | 35 / 66 / 120 | 15 / 37 / 29 |
| while / forEach / compute / switch / break-continue | 14 / 8 / 3 / 13 / 3 | 4 / 1 / 3 / 3 / 0 |
| try-except / raise | 7 / 1 | 1 / 2 |
| assert / log | 140 / 18 | 7 / 4 |
| Dict / List / Tuple | 15 / 34 / 23 | 3 / 10 / 6 |
| configuration property / module property | 20 / 34 | 4 / 11 |
| string operations | 19 | 0 |
| collection operations / OCL iterators | 122 / 69 | 10 / 10 |
| xcollect, xselect, selectOne, collectselect | 14 | 1 |
| `[T]` / `[cond]` shorthand | 32 | 3 |
| `#T`, `##T`, `->[...]` | 0 | 0 |
| `->map` / `xmap` | 19 / 7 | 2 / 1 |
| inout model parameter / inout mapping parameter | 14 / 44 | 5 / 16 |
| objectsOfType / model operations / allInstances | 10 / 49 / 15 | 1 / 3 / 1 |
| transformation composition (`transform()`) | 13 | 0 |
| multiple models | 16 | 0 |
| trace persistence / incremental update | 2 / 1 | 0 |
| runtime failure (stack or log expected) | 13 | 0 |

## What a standalone harness must provide

- Imports resolve from the project root (`import models.<name>.Foo;`): the source root is the
  parent of `models/`.
- Metamodels: `testqvt`, `generics`, `bug577992` (generated in the test bundle; we load the
  `.ecore`), SimpleUML and RDB from `plugins/org.eclipse.m2m.qvt.oml.samples/models/`, the
  QVTo AST and trace metamodels, the dynamic packages `models/test_dynamicpkg.ecore` and
  `deployed/dynamicmodel/registered.ecore`, and case-local files from `includeMetamodelFile`.
- `platform:/plugin/...` and `platform:/resource/...` URI mappings.
- Java blackboxes from the test bundle (23 cases): ported to Rust blackboxes in `qvto-testkit`.
- Out of scope: workspace builder, JDT/PDE builds (bug577992, bug507955 and 6 parser cases),
  project references (bug433937, bug573659), Ant, debugger and launch configurations.

## Licence

`.qvto`, `.ecore` and `.xmi` files carry no licence header; the repository is EPL-2.0. Copies of
test data in qvto-rs keep EPL-2.0 and their origin.

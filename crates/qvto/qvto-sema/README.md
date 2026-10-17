# qvto-sema

Semantic analysis of QVT Operational Mappings as Eclipse QVTo 3.11 does it: a port of
`QVTOCompiler` (units, imports, cycles), `QvtOperationalVisitorCS` on classic OCL's
`AbstractOCLAnalyzer` (modules, model types, name and overload resolution, type checking,
Eclipse's desugarings), and the AST validation (`QvtOperationalValidationVisitor` with its OCL
`ValidationVisitor`), with Eclipse's diagnostics: same severity, offset, length and message. The
output is the typed IR (`src/ir.rs`) that `qvto-bytecode`/`qvto-vm` compile and run.

```rust
let comp = qvto_sema::compile_with_operations(main_unit, &resolver, registry, &eoperations);
for unit in &comp.units {
    for p in &unit.problems {
        let (offset, length) = p.utf16(&unit.source.text); // Eclipse's (UTF-16) positions
        println!("{:?} {} {offset} {length} {}", p.severity, p.line(&unit.lines), p.message);
    }
}
if !comp.has_errors() { /* comp.program: the IR */ }
```

## Inputs

- `UnitSource`: key, qualified name (`a.b.C`), file name, text; or a Java blackbox unit
  (`BlackboxUnit`: modules and operation signatures as Eclipse derives them from the Java class,
  `load_error` when Eclipse fails to load it).
- `UnitResolver`: `resolve(importer, "a.b.C")` (the analyser retries `<importer namespace>.C`, as
  Eclipse does) and `blackbox_units()`, the registered Java units searched for implementations of
  `blackbox` operations declared in QVTo.
- `MetaRegistry`: Eclipse's package registry, every metamodel the units name by nsURI (Ecore
  included). `scan_metamodel_uris` lists the URIs of a unit and its imports for hosts that load
  on demand. The analyser adds QVTo's library classes (`Stdlib`), the `_INTERMEDIATE` packages of
  intermediate classes, and a placeholder package for intermediate features of non-classifier
  types; ids of the given registry do not change.
- `MetaOperation`s: the metamodels' `EOperation`s (the registry has none), e.g. Ecore's
  `eGet`, `getEStructuralFeature`. `compile` passes none.

## Structure

| File | Eclipse | Contents |
|---|---|---|
| `units.rs` | `QVTOCompiler`, `AbstractQVTParser.setupTopLevel` | compilation of a unit and its imports (depth first, cached, cycles, "Imported unit has errors", duplicate imports), the distribution of top-level elements over modules, blackbox units |
| `module.rs` | `visitUnitCS` passes 1–6, `QvtOperationalModuleEnv` | module headers, model types (strict/effective, `where`), signatures, `access`/`extends` usages with signature checks, module loops, method headers (qualifiers, `main`, contextual operations, collisions and virtual tables), module, configuration and contextual properties, method bodies |
| `method.rs` | `visitMappingMethodCS`, `visitIntermediateClassesCS`, tags, `rename` | constructors, mappings (when/where, init/population/end, implicit `object` for the result, mapping extensions), helpers/queries/`main`; intermediate classes and exceptions; aliases |
| `expr.rs` | `oclExpressionCS`, `customSimpleNameCS`, `operationCallExpCS`, `genOperationCallExp` | literals, names and implicit sources, navigation and implicit `collect`, operation calls (implicit source, module-qualified, static, `->` with implicit `xcollect`), result types (operation codes, `GenericsResolver`), `let`, `if`, `validateOperationCall`, deprecation checks |
| `imp.rs` | the ImperativeOCL visitors | OCL iterators, imperative iterators and the `->name`, `[...]`, `->object`, `->switch` shorthands, `forEach`, blocks, `var`, assignment (`validateAssignment`, `validateVariableModification`), `while`, `compute`, `switch`, `return`, `log`, `assert`, `raise`, `try`, `new` with constructors, `object` (extents, `validateObjectExp`), mapping calls, the resolve family, late resolves |
| `lookup.rs` | `TypeCheckerImpl`, `QvtTypeResolverImpl`, `QVTUMLReflection` | operations and attributes of a type (OCL ops, metamodel ops, the library's and the modules' additional operations), overload resolution (`findMostSpecificOperationMatching`), properties with aliases |
| `names.rs`, `vars.rs` | `lookupClassifier`, `typeCS`; environments | type names (model types, packages, imported modules), variables per environment level, implicit sources (`this` before `self`, extended modules and libraries) |
| `ocltypes.rs` | `OCLStandardLibraryUtil.getResultTypeOf` | result types of classic OCL operations |
| `annot.rs` | `OCLAnnotationSupport` | OCL bodies of metamodel operations and OCL derivations |
| `validate.rs` | `QvtOperationalValidationVisitor` | the validation pass over the IR (operations, mapping extensions, `return`, `break`, instantiation, iterators, variable initialisers, null types) |
| `blackbox.rs` | `BlackboxUnitResolver` | modules of Java blackbox units |
| `messages.rs` | `NLS.bind`, `messages.properties` | the 444 messages, generated (`examples/gen_messages.rs`) |
| `irck.rs` | | consistency checks of the IR of error-free programs |

## Verification

`tests/oracle.rs` (about 4 s) compiles every file of the corpus — the 1 047 `.qvto` files of the
Eclipse suite and the 55 SimuLizar/SPD files, as `tests/data/compile.cases` — and compares with
Eclipse's full-compile diagnostics (`tests/fixtures/eclipse-compile-oracle.tsv`, produced by
`tools/oracle/make-compile-fixture.sh` on the reference runner's jars). The harness
(`tests/oracle_harness/`) reproduces Eclipse's setup per case: the registry of the runner's
bundles and the case's metamodel files, unit resolution through the bundles' source containers,
`imports root`, sibling folders and the classpath, and the blackbox units of
`crates/qvto-types/data/blackboxes.tsv`. The test gates on the number of exactly matching files
(`tests/fixtures/sema-oracle-baseline.txt`) and on `irck` for every error-free compilation.

`cargo run --release -p qvto-sema --example oracle_report` prints the full comparison and writes
every mismatch to `target/q2-oracle-report.tsv`. On 2026-10-17 (after Q4b; Q2's numbers in
brackets):

| | |
|---|---|
| files whose problems equal Eclipse's exactly (severity, unit, line, offset, length, message) | **1 102 of 1 102** (Eclipse suite 1 047, SimuLizar 37, SPD 18) [1 096] |
| files Eclipse compiles without problems | 541; we report none on 541 [539] |
| files with errors | 133; 133 match exactly [129]; errors or not agrees on all 1 102 files |
| problems | Eclipse 2 304, ours 2 304, all reproduced [8 extra] |
| `TestQvtParser` expected counts | 117 cases, ours pass 117 [115] |
| `TestQvtParser` inline annotations | 93 of 104 annotated files match exactly, as Eclipse's own problems [91] |
| IR checks | 969 error-free compilations, no violations |
| speed | 0.44 s for all 1 102 files (release, mean 0.40 ms, largest file 36 ms); Eclipse: 1.77 s warm |

Q4b closed the last six differences (DESIGN §12 Q4b-3): problems of OCL's validator inside
`object` bodies, which Eclipse drops (`UMLFoo`, `Ecore2UML`); LPG's repairs of a missing object
type and of `a-b` in qualified names (`missingObjectType`, `exec_erroneous`); a Java unit that
loads only with the importing module's packages (`bug565747`, alternative signatures in
`blackboxes.tsv`).

`tests/q7.rs` checks the analyser regressions of Q7's fuzz campaigns and external corpora
(`tests/data/q7/*.qvto` with Eclipse's problems in `.diag`, recorded by `tools/q7-record.sh`):
operations on unknown types, generic `CommonT` problems without position, unknown result and
context types.

`tools/probe/probe.sh` asks Eclipse about single files (problems, AST, the operation each call
refers to) for comparisons on small programs.

OCL annotations of metamodels (`annot.rs`): the `body` of operations and the `derive` of
features of dynamic classes are analysed after the units (`compile_with_metamodel`), in the main
module's environment; problems are not reported and a body with errors is `invalid`, as Eclipse
evaluates an unparseable annotation.

## IR changes

`CHANGES.md` lists every change of `ir.rs` after its first version.

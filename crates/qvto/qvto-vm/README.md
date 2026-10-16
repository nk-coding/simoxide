# qvto-vm

The bytecode interpreter of the engine (DESIGN §5): frames and registers, calls of helpers,
queries, constructors and mappings with Eclipse's protocol, virtual dispatch, objects in model
extents, QVTo's assignment semantics, traces and the `resolve` family, late resolve, `log`,
`assert`, `raise`/`try`, and the standard library over `qvto-runtime`. Generic over the model
store (`qvto_model::ModelStore` whose handle is a dense `u32`, `ObjHandle`; `qvto-emf`'s
`ObjId` with the default feature `emf`). Status: Q3a (see `PLAN.md`).

```rust
let program = qvto_bytecode::compile(ir)?;
let mut vm = qvto_vm::Vm::new(&program, &mut resource_set.graph, qvto_vm::RunOptions::default());
let outcome = vm.run_transformation(vec![input_roots, vec![]]);   // per model parameter
println!("{} {} {}", outcome.severity, outcome.code, outcome.message);
print!("{}", vm.log);                                              // WriterLog's text
```

## Layout

| File | Contents |
|---|---|
| `src/vm.rs` | `Vm`, `RunOptions`, frames, the instruction loop, `try` handlers, iteration slots |
| `src/calls.rs` | helper/query/constructor calls, mapping calls (`when`, trace lookup, disjuncts, `init`, implicit results and `out` parameters, the trace record before population, inherited and merged mappings, `end`, `where`), virtual dispatch (`VirtualTable`), overriding |
| `src/objects.rs` | navigation, `assign`/`callSetter`, creation into extents, engine-local objects, contextual intermediate properties, default values, `clone`/`deepclone` (`EcoreUtil.Copier`), Ecore's `EObject` operations |
| `src/extent.rs` | `ModelParameterExtent`: initial and created objects, in-memory `extent:/N` resources, the purge counter |
| `src/trace.rs` | trace records and indexes, `resolve`, `resolveIn`, `invresolve` |
| `src/stdlib.rs` | `Prim`: every `StdOp` of the generated table mapped to a run-time primitive; the primitives |
| `src/types.rs` | `oclIsKindOf`, `oclIsTypeOf`, `oclAsType`, QVTo's `isKindOf` |
| `src/log.rs` | `log` records (`WriterLog`), assertions, `String.valueOf` of values with EMF's `toString` |
| `src/run.rs` | `run_transformation`: extents, module initialisation, configuration properties, model type `where`, the entry operation (legacy `main(in x : T)` arguments), deferred late-resolve assignments, `RunOutcome` |
| `src/env.rs` | `RunEnv` (`HashEnv`: mode, memoised identity hashes), `TapeHook` (identity hashes from a Java run) |
| `src/error.rs` | `QvtError` (Eclipse's exception classes, messages, codes), QVT stack traces |
| `src/jregex.rs` | Java regular expressions on the `regex` crate |
| `tests/vm.rs`, `tests/common` | tests on hand-written typed IR (a small IR builder) |
| `examples/micro.rs` | micro benchmarks (same programs as `examples/micro.qvto`) |

## Execution model

- **Registers.** One `Vec<Value>` register stack; a call allocates a window of the callee's
  register count after the caller's. Arguments are temporaries of the caller and are moved into
  the callee's parameter registers. A second stack holds iteration slots.
- **Runtime errors** of Java code in Eclipse (casts, missing properties on null, index errors,
  model exceptions) do not unwind: the instruction yields `invalid` and a message is recorded
  (`Vm::errors`, Eclipse's `Evaluation failed at ...` on stderr). Only QVT exceptions unwind
  (`Unwind`): `raise` and fatal assertions (catchable), `QvtRuntimeException`, stack overflow,
  interruption.
- **Stack traces** use the frames' current instructions and the compiler's per-instruction
  instruction pointers: `Module::[Context::]op(file:line)`, as `QvtStackTraceBuilder`.
- **Extents.** Each model parameter of the main transformation has an extent; created objects
  go to the explicit extent, else the first non-`in` parameter whose model type's first package
  is the class's root package, else the first non-`in` parameter, else the unbound extent.
  Created objects without a resource become roots of an in-memory resource `extent:/N` of the
  store; at the end the out/inout extents' roots are detached from it (`getContents`).
- **Engine-local objects**: instances of QVTo's library classes and of intermediate classes live
  in the VM (`ObjRef::local`), with a slot per feature of the class.
- **Mode**: `RunOptions::mode` (compat: Java `HashMap` orders and identity hashes from a source;
  fast: insertion order). The VM is not monomorphised per mode (DESIGN §12, Q3a-3).

## Coverage (Q3a)

Implemented: all instructions of the bytecode; helpers, queries, constructors, contextual
operations with virtual dispatch (`null` dispatches as OclVoid); mappings with `when`, `where`,
`init`, population, `end`, traces, `inherits`, `merges`, `disjuncts`, `out`/`inout` parameters,
`map`/`xmap`, results as tuples; `object`, `new` (classes, data types, collections), extents
(with Eclipse's purge, the evaluation result of legacy entries, failed runs writing nothing
back, model parameters of other transformations bound as `ModelParameterHelper` does);
navigation, assignment (`:=`, `+=`, many-valued features with `ECollections.setEList`);
`resolve`, `resolveone`, `resolveIn`, `invresolve`, `late resolve` (property assignments,
conditions); `log`, `assert`, `raise`, `try`/`except`; module and configuration properties,
model type `where`; nested transformations (`new T(...)`, `transform()`, `Status`); the
standard library except the parts below; `clone`, `deepclone`; Java regular expressions that
the `regex` crate accepts.

Not yet (Q4): blackboxes (Java libraries; `No blackbox implementation found`), blackbox
transformations, `Model::copy`/`createEmptyModel`, `Collection::product`, metamodel operations
other than `eContainer`/`eContents`/`eAllContents`/`eIsProxy` (no OCL bodies of EOperations),
`unlink`, incremental update traces, per-instance properties of nested transformations.

## Verification

- `cargo test -p qvto-vm`: the primitive table covers every operation of the standard library
  table; programs on hand-written IR cover arithmetic and strings, null/invalid logic,
  iterators (including `closure`, `iterate`, `xselect`, `xcollect`, `sortedBy`), calls and stack
  overflow with Eclipse's stack trace text, `raise`/`try`, fatal assertions, mappings with
  traces and `resolveone`, `when` and `xmap`, out parameters, disjuncts, virtual dispatch,
  collections, `Dict` and `List`, objects in out extents.
- End to end (`qvto conform --engine qvto`, the analyser of branch `q2` in progress): 400 of
  the 494 runnable Eclipse cases equal the Java runner strictly (models, log, severity, code,
  stack, problems), 401 pass Eclipse's checks, fast mode the same; `TestQvtInterpreter` 247 of
  309. Of the failures, 49 are cases the analyser rejects and 22 parser cases with other
  diagnostics; most others trace back to the analyser's output (source ids in stack traces,
  overloads such as `List::insertAt`, QVTo's `Sequence::insertAt` shadowing OCL's) or Q4
  features (blackboxes, `Model::copy`). Corpus: 22 of 24 strictly.
- Performance (`qvto bench --java`, release build, median of 20 runs; Java: the reference
  runner, same cases repeated in one JVM, median of the second half):

  | Case | qvto-rs run | Java run (warm) |
  |---|---:|---:|
  | `interpret_operation_override` | 0.046 ms | 9.0 ms |
  | `interpret_overload_multipleParams` | 0.033 ms | 8.2 ms |
  | `interpret_resolve_resolveoneIn` | 0.035 ms | 2.6 ms |
  | `interpret_iteratetest` | 0.027 ms | 2.6 ms |
  | `interpret_bug233984` | 0.036 ms | 3.0 ms |
  | `simulizar_runconfigtest_scaleUp_high` | 0.052 ms | 7.7 ms |
  | `simulizar_runconfigtest_addNewServer_high` | 0.073 ms | 6.7 ms |
  | `simulizar_runconfigtest_addClonedServer_low` | 0.042 ms | 4.7 ms |

  Compiling (parse, analysis, bytecode) takes 0.1–0.5 ms per case. Micro benchmarks
  (`examples/micro.qvto` through `qvto run --repeat`, Java `qvto-run --repeat`, warm):
  1M-iteration arithmetic loop 15 ms vs 118 ms, `fib(25)` 12.8 ms vs 51 ms, `select`/`collect`/
  `sum` over 100 000 integers 5.4 ms vs 12 ms, 20 000 `including`s in a loop 1.7 ms vs 5.1 s.

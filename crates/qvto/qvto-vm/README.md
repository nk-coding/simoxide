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
| `src/objects.rs` | navigation, `assign`/`callSetter`, creation into extents, engine-local objects, contextual intermediate properties, default values, `clone`/`deepclone` (`EcoreUtil.Copier`) |
| `src/native.rs` | native blackboxes: `Natives` (implementations by key, transformations by name, EOperations), the call protocol (library operations, `blackbox` operations declared in QVTo, blackbox mappings and transformations, Java exceptions, cancellation), the `native_*` API for implementations |
| `src/meta.rs` | metamodel operations (`Callee::Meta`): Ecore's `EObject`, `EClass`, `EPackage`, `EModelElement`, `EClassifier`, `EEnum`, `EFactory`, `EStructuralFeature`, `EOperation` operations from EMF's semantics; host implementations of other metamodels' operations |
| `src/model_ops.rs` | `Model::copy()`, `Model::createEmptyModel()`, model types of extents; `BigInteger`/`BigDecimal` attribute values |
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
- **Engine-local objects**: instances of QVTo's library classes (`Exception`, `Status`) and of
  exception classes live in the VM (`ObjRef::local`), with a slot per feature of the class.
- **Intermediate classes** (`objects.rs`, `IntermInfo`, DESIGN §12 Q4b-4): when the store takes
  the program's registry (`ModelStore::extend_meta`), instances are store objects in an
  intermediate extent of their transformation: references with EMF's semantics (containment,
  opposites, container references), attributes and `static` features kept by the VM; else
  engine-local objects.
- **Dispatch**: every call of a contextual operation goes through its virtual table
  (`Operation::vtable`, the operation of the actual type or its closest supertype); module
  overriding (`extends`) applies to calls that are not module-qualified (DESIGN §12 Q4b-1).
  Operations of dynamic classes with an OCL body run that body (on the overriding operation of
  the instance's class, `meta.rs`); derived features with an OCL derivation evaluate it.
- **Nested transformations**: each `new T(...)` instance keeps its own model extents and
  property values, swapped in while it runs (`run.rs`).
- **Blackboxes** (`native.rs`, DESIGN §12 Q4a-1): an operation with `is_blackbox` runs the host's
  function registered under its `blackbox_impl` key (`Vm::set_natives`). Operations of Java
  (native) library modules are called like OCL operations (no stack frame; `invalid` on a
  `null`/`invalid` receiver; `map` on them skips the mapping protocol); `blackbox` operations
  declared in QVTo run on a stack frame of their own, a `blackbox mapping` with the protocol
  (`when`, trace lookup, a defined result, trace record, merged mappings, the result's extent);
  `invalid` arguments are passed as `null`; calls of library operations are never redirected to
  overriding operations (Eclipse calls them through their handler); implementation exceptions become
  `QvtRuntimeException`s with Eclipse's text, cancellation interrupts. A `blackbox
  transformation` is run by the implementation registered under its name with its model
  extents, its configuration properties as the context's.
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

Q4a added: native blackboxes (library operations, `blackbox` operations declared in QVTo,
blackbox mappings, constructors and transformations), `Model::copy`/`createEmptyModel`, Ecore's
EOperations and host EOperations, `BigInteger`/`BigDecimal` attributes as numbers, roots added
to an extent's resources by blackboxes (`ResourceModificationAdapter`), dictionary keys cloned by
`deepclone`.

Q4b added: virtual tables from the analyser (no tables of the VM's own), intermediate class
instances in the store (EMF references, static features), OCL bodies of EOperations and OCL
derivations (`OCLAnnotationSupport`), incremental update with an input trace
(`inctrace.rs`, `Vm::set_incremental_trace`), per-instance properties of nested transformations
(`t.p`), List mutators returning the list, classic OCL's invalid rule for two-argument
operations, `sortedBy` evaluating an effectful body twice in compat mode (once in fast mode).
`unlink` is not part of Eclipse's language (DESIGN §12 Q4b-10).

## Verification

- `cargo test -p qvto-vm`: the primitive table covers every operation of the standard library
  table; programs on hand-written IR cover arithmetic and strings, null/invalid logic,
  iterators (including `closure`, `iterate`, `xselect`, `xcollect`, `sortedBy`), calls and stack
  overflow with Eclipse's stack trace text, `raise`/`try`, fatal assertions, mappings with
  traces and `resolveone`, `when` and `xmap`, out parameters, disjuncts, virtual dispatch,
  collections, `Dict` and `List`, objects in out extents.
- End to end (`qvto conform --engine qvto`), after Q4b: **all 494 runnable Eclipse cases and
  all 24 corpus cases equal the Java runner strictly** (models, log, severity, code, stack,
  problems), fast mode the same. After Q4a: 482 and 23 (483 and 24 Eclipse checks); after Q3a:
  424 and 22.
- `cargo test -p qvto --test run`: end-to-end programs whose expected logs are Eclipse's
  (List mutators, two-argument OCL operations, intermediate classes, OCL annotations,
  incremental update, nested instances, `sortedBy`).
- Performance: `qvto bench` (`crates/qvto-cli/README.md`, "Results"; DESIGN §7.2). After Q5b,
  against the warm Java runner: SimuLizar rule runs 0.09–0.12 ms (4–11×; series on the same
  model 5–35 µs per run, 4–18×), SPD `MainTransformation` 0.23–0.86 ms (4.5–11×),
  SimpleUML→RDB 0.2/1.5/21 ms on 64/630/6 300 output objects (13×/9×/2.7×), Ecore copy by
  mappings 0.24 ms on `Ecore.ecore` (7×), micro benchmarks 2.8–23×; suite cases median 5.8×.
  Profiling: `cargo build --release -p qvto-cli --features profile` and `qvto bench --profile
  FILE.svg` (DESIGN §12 Q5b-2). What made the difference (§12 Q5b-3 to Q5b-8): cached Ecore
  derived lists and O(1) store paths, `+=` as `add`s, hashed `sortedBy` deduplication, Fx
  hashing, dense identity-hash tables, per-program tables instead of per-call scans (the
  overriding chain), a reused argument buffer for library calls, loops that read Sequences and
  OrderedSets in place.

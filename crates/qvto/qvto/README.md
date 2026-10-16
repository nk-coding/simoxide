# qvto

The engine's facade: `compile`/`compile_with_operations` (parse, analyse with `qvto-sema`,
compile to bytecode) and the VM (`Vm`, `RunOptions`, `RunOutcome` from `qvto-vm`);
`units::FsResolver` finds imported units on the file system (relative to the importing file, or
in the source containers of bundle directories, as the Eclipse test environment does);
`blackbox` provides native blackbox libraries. Status: Q4a (see `PLAN.md`).

```rust
let resolver = qvto::units::FsResolver::folder();
let main = resolver.unit_source(Path::new("t.qvto"))?;
resource_set.ecore_resource();                                   // EcorePackage's objects
let ops = qvto_emf::operations::meta_operations(&resource_set.graph);   // the EOperations
let compiled = qvto::compile_with_operations(main, &resolver, resource_set.graph.meta().clone(), &ops);
let program = compiled.program.expect("no errors");   // problems in compiled.units
let mut vm = qvto::Vm::new(&program, &mut resource_set.graph, qvto::RunOptions::default());
vm.set_natives(Rc::new(natives));                                // blackbox implementations
let outcome = vm.run_transformation(vec![input_roots, vec![]]);
```

The store must hold the metamodels the program was compiled against with the same ids (load
them in the same order; the program's registry only adds QVTo's library and intermediate
classes).

## Blackboxes

Eclipse runs Java blackbox libraries; here a blackbox library is native Rust in two halves
(DESIGN §12, Q4a-1):

- **Signatures** (`blackbox::Catalog`): units, modules, the metamodels they use and their
  operations (kind, context, parameters, result, an *implementation key*, flags), as rows of a
  tab-separated table — the format of `qvto-types/data/blackboxes.tsv`, which holds what
  Eclipse derives from the Java classes of the test suite and the corpus. A host writes the same
  rows for its own libraries. `blackbox::BlackboxResolver` puts the units behind a file resolver:
  registered units (Eclipse's `BlackboxRegistry`, also the candidates for `blackbox` operations
  declared in QVTo) and units found by class name.
- **Implementations** (`blackbox::Natives<S>`, from `qvto-vm`): closures
  `Fn(&mut Vm<S>, NativeCall) -> NativeResult` by implementation key —
  `natives.java("public java.lang.String pkg.Lib.echo(java.lang.String)", f)` derives the key
  `pkg.Lib#echo(java.lang.String)` from Java's `Method.toString()`, which is also the text of
  invocation errors; `natives.op(key, f)` for any other key; `natives.transformation(name, ...)`
  for `blackbox transformation`s; `natives.eoperation(nsURI, class, name, f)` for metamodel
  operations that generated Java code implements in Eclipse. A native function gets `self`
  (contextual receiver, a constructor's new object, a transformation instance) and the
  arguments, and uses the VM's `native_*` API: log, cancellation, configuration properties,
  objects by feature name, collections, `List`/`Dict`, model extents, per-run library state.
  `NativeError::Exception` becomes Eclipse's `QvtRuntimeException` (`Java method invocation
  error '<method>'` / `Caused by: <class>: <message>`).

Provided libraries:

| Module | Units | Contents |
|---|---|---|
| `blackbox::stdlib` | `Strings`, `emf.tools` | QVTo's deprecated legacy libraries: signatures (`data/stdlib-blackboxes.tsv`) and ports of `StringLibrary`/`EmfToolsLibrary` |
| `blackbox::palladio` | SimuLizar's `ProfilesLibrary`, `SimulationStateLibrary`; the architectural templates' `ProfilesLibrary` | signatures (`data/palladio-blackboxes.tsv`); `register_simulation_time(natives, clock)`; stereotype operations are the host's (keys in the table) |

The Eclipse test bundle's Java blackboxes (`TestBlackboxLibrary`, `org.bar.Foo`'s
`AnnotatedJavaLibrary`, `SimpleJavaLibrary`, `Bug289982_Library`, ...) are ported in the
conformance adapter (`qvto-cli/src/bbox.rs`). `tests/blackbox.rs` shows a host library end to
end (helpers, contextual queries, mappings, errors, cancellation, library state, a blackbox
transformation, a `blackbox` operation declared in QVTo).

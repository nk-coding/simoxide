# qvto

The engine's facade: `compile` (parse, analyse with `qvto-sema`, compile to bytecode) and the
VM (`Vm`, `RunOptions`, `RunOutcome` from `qvto-vm`); `units::FsResolver` finds imported units
on the file system (relative to the importing file, or in the source containers of bundle
directories, as the Eclipse test environment does). Status: Q3a (see `PLAN.md`).

```rust
let resolver = qvto::units::FsResolver::folder();
let main = resolver.unit_source(Path::new("t.qvto"))?;
let compiled = qvto::compile(main, &resolver, resource_set.graph.meta().clone());
let program = compiled.program.expect("no errors");   // problems in compiled.units
let mut vm = qvto::Vm::new(&program, &mut resource_set.graph, qvto::RunOptions::default());
let outcome = vm.run_transformation(vec![input_roots, vec![]]);
```

The store must hold the metamodels the program was compiled against with the same ids (load
them in the same order; the program's registry only adds QVTo's library and intermediate
classes).

# Changes to the typed IR (`src/ir.rs`)

The IR is the contract between `qvto-sema` and `qvto-bytecode`/`qvto-vm`. Every change after the
first version is listed here, newest first.

| Date | Change |
|---|---|
| 2026-10-17 | `Module::blackbox` is also set for the modules of Java (native) blackbox units; `Operation::blackbox_impl` is also set for a `blackbox` operation declared in QVTo whose Java implementation the analyser matched uniquely (Q4a, native blackboxes). `MetaOperation`/`MetaMultiplicity` moved to `qvto-model` (re-exported). |
| 2026-10-16 | `IntermClass::feature_types`: the declared (OCL) type of each own feature of an intermediate class; the registry's classifier is a placeholder for collection, tuple and other non-classifier types. |
| 2026-10-16 | `Assign::target_span`: the span of the assignment's left-hand side. |
| 2026-10-16 | `Operation::blackbox_impl`: the Java method of an operation of a Java blackbox library (`pkg.Class#name(java.lang.String, ...)`), for the VM's native bindings. |
| 2026-10-16 | `LValue::TupleProp(expr, part)`: `t.part += x` on a `List`/`Dict` part of a tuple (Eclipse allows only this tuple mutation). |
| 2026-10-16 | `while (x := e; c)` / `while (x : T := e; c)` is `ExprKind::Compute(x, e, While(c, body))` (Eclipse's `ComputeExp` around the `WhileExp`), not a block; doc fixed. |
| 2026-10-16 | Documented: `ModuleId(0)` is QVTo's library module `Stdlib` (no source, no operations in `Program::operations`; its operations are `Callee::Std`). `SourceId(0)` is a placeholder for it; unit sources start at `SourceId(1)`. |
| 2026-10-16 | First version. |

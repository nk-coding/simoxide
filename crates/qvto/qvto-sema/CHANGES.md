# Changes to the typed IR (`src/ir.rs`)

The IR is the contract between `qvto-sema` and `qvto-bytecode`/`qvto-vm`. Every change after the
first version is listed here, newest first.

| Date | Change |
|---|---|
| 2026-10-17 | `ExprKind::ModuleProp(inst, g)` / `LValue::ModuleProp(inst, g)`: a property read or written through a transformation instance other than `this` (`t.p` with `t := new T(...)`); `Global(g)` stays the property of the running instance (Q4b). |
| 2026-10-17 | `MetaOp::body` (`MetaBody`: a `CodeBody` with `self` and the parameters' variables): the analysed OCL body of an operation of a dynamic class (`http://www.eclipse.org/2007/OCL` annotation, `body`); `Program::derivations`: OCL derivations of features (`derive`). Hosts pass them with `compile_with_metamodel` (`MetaOperation::body`, `MetaDerivation` in `qvto-model`) (Q4b). |
| 2026-10-17 | `IntermClass::static_features`: the own features declared `static` (one value for all instances) (Q4b). |
| 2026-10-17 | `ExprKind::Raise(class, arg)`: `raise 'msg'` is `Raise(Some(Exception), Some('msg'))` as in Eclipse; `None` class only after an error (Q4b). |
| 2026-10-17 | `Operation::vtable` is filled by the analyser (Eclipse's `VirtualTable` contents from `doFindCollidingOperation`, in insertion order); the VM no longer computes tables of its own (Q4b). |
| 2026-10-17 | `ModelParam::generated`: a model parameter generated for a legacy `mapping main(in x : T) : U` of a transformation without parameters (`DeprecatedSignaturelessTransf`, named `$x`); `Module::model_params` is filled by the analyser (Q3a's temporary copy removed) (Q4b). |
| 2026-10-17 | `Module::blackbox` is also set for the modules of Java (native) blackbox units; `Operation::blackbox_impl` is also set for a `blackbox` operation declared in QVTo whose Java implementation the analyser matched uniquely (Q4a, native blackboxes). `MetaOperation`/`MetaMultiplicity` moved to `qvto-model` (re-exported). |
| 2026-10-16 | `IntermClass::feature_types`: the declared (OCL) type of each own feature of an intermediate class; the registry's classifier is a placeholder for collection, tuple and other non-classifier types. |
| 2026-10-16 | `Assign::target_span`: the span of the assignment's left-hand side. |
| 2026-10-16 | `Operation::blackbox_impl`: the Java method of an operation of a Java blackbox library (`pkg.Class#name(java.lang.String, ...)`), for the VM's native bindings. |
| 2026-10-16 | `LValue::TupleProp(expr, part)`: `t.part += x` on a `List`/`Dict` part of a tuple (Eclipse allows only this tuple mutation). |
| 2026-10-16 | `while (x := e; c)` / `while (x : T := e; c)` is `ExprKind::Compute(x, e, While(c, body))` (Eclipse's `ComputeExp` around the `WhileExp`), not a block; doc fixed. |
| 2026-10-16 | Documented: `ModuleId(0)` is QVTo's library module `Stdlib` (no source, no operations in `Program::operations`; its operations are `Callee::Std`). `SourceId(0)` is a placeholder for it; unit sources start at `SourceId(1)`. |
| 2026-10-16 | First version. |

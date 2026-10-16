# qvto-sema

Semantic analysis of QVT Operational Mappings as Eclipse QVTo 3.11 does it (the CST-to-AST
analyser `QvtOperationalVisitorCS` on classic OCL's `AbstractOCLAnalyzer`): units and imports,
modules, metamodel binding, name resolution, overload resolution, type checking, Eclipse's
desugarings, and Eclipse's diagnostics. Output: the typed IR.

Status: Q2 in progress (see `PLAN.md`). The IR (`src/ir.rs`) is fixed first so that the VM
(Q3a) can be built against it; changes are listed in `CHANGES.md`.

## The typed IR

`src/ir.rs` holds the whole definition with its documentation. In short:

| Item | What |
|---|---|
| `Program` | type table, the grown `MetaRegistry` (QVTo library classes, intermediate classes), sources with line indexes, modules, model types, operations, globals (module and configuration properties), contextual intermediate properties, intermediate classes with feature initialisers, metamodel operations |
| `Module` | transformation or library: model parameters, usages (`access`/`extends`), globals in initialisation order, operations, entry |
| `Operation` | helper, query, mapping, constructor, entry: context type, parameters with directions and extents, results, variable slots (`locals`), body statements or `MappingInfo` (when, where, init, population, end, inherits, merges, disjuncts), Eclipse's virtual table, override of an extended module's operation |
| `Expr` | typed expression tree: literals, variables (`Local`, `This`, `Global`, `ModelParam`), navigation (`Prop` by `FeatureId`, tuple parts, intermediate properties), calls (`Callee::Std(StdOp)`, user operations, mappings, metamodel operations), loops (OCL and imperative iterators), `let`, `if`, and the imperative expressions (block, var, assign, while, compute, switch, return, break, continue, log, assert, raise, try, object, new, resolve) |

Desugarings done by the analyser (as Eclipse's analyser does them) and the run-time protocol left
to the VM are listed in the module documentation of `ir.rs`.

# Architecture

## Pipeline

```text
 .qvto ──► qvto-syntax ──► AST ──► qvto-sema ──► typed IR ──► qvto-bytecode ──► Program
                                     ▲    ▲                                       │
   .ecore ──► simoxide-emf ──────────┘    └── qvto-types (types, library table)   ▼
                 │                                                             qvto-vm
 models (XMI, or SimOxide's model graph) ── ModelStore (simoxide-emf-core) ◄──────┤
                                                                                  │
                         qvto-runtime (values, collections, Java behaviour) ◄─────┘
```

A transformation is compiled once and run many times. Compiling means three steps: parse, analyse
against the metamodels in a `MetaRegistry`, and compile the typed IR to register bytecode. A
`Program` is immutable and `Send + Sync`. A run creates a VM over a model store, binds the model
parameters' root objects and interprets the program. The models are read and written in place,
never copied.

## Crates

The QVTo crates are in `crates/qvto/`. They keep their `qvto-*` names, so the engine stays usable
without the simulator.

| Crate | Role |
|---|---|
| `qvto-syntax` | lexer, recursive-descent parser with Pratt parsing for OCL, AST with spans, error recovery, unparser. The authority is Eclipse's LPG grammar, not the specification's EBNF; the keyword and operator tables are generated from the grammar files. No dependencies |
| `qvto-types` | interned OCL/QVTo types, QVTo's library classes, and the standard library table (`StdOp`). The table is generated from Eclipse's library and shared by the analyser and the VM. Blackbox signature tables (`data/*.tsv`) |
| `qvto-sema` | semantic analysis as Eclipse's compiler does it: units and imports, model types, name and overload resolution, type checking, Eclipse's desugarings and validation, with Eclipse's problems (severity, offset, length, message). Output: the typed IR (`src/ir.rs`, changes logged in `CHANGES.md`) |
| `qvto-bytecode` | the register instruction set (at most 8 bytes per instruction), the compiler from the IR, constant pools and side tables, and the disassembler. OCL and imperative iterators become inline loops |
| `qvto-runtime` | values (16 bytes; null and invalid; Java's number boxes), the four OCL collection kinds with the Java classes Eclipse uses for them, tuples, `Dict`, `List`, strings with UTF-16 semantics, identity-hash sources and the modes (`Mode`, `HashEnv`) |
| `qvto-vm` | the interpreter: frames and registers, calls with Eclipse's mapping protocol, virtual dispatch, extents, traces and the `resolve` family, late resolve, `log`, `assert`, exceptions and QVT stack traces, the standard library, native blackboxes, Ecore's operations, resource limits |
| `qvto` | the facade: `compile*`, `Vm`, `RunOptions`, `units::FsResolver`, the `blackbox` libraries ([Usage](./usage.md)) |
| `qvto-testkit` | the conformance harness: case files, the Java runner's fixtures, identity-hash tapes, the comparisons and the scoreboard, fuzz generators ([Conformance](./conformance.md)) |
| `qvto-cli` | the `qvto` command, the benchmark suite, and the conformance engine with the test bundle's Java blackboxes ported |
| `qvto-paths` | where tests, examples and tools find the QVTo root and the target directory ([Development](./development.md#paths)) |

Each crate's `README.md` describes its files and its own verification.

## On the foundation crates

The QVTo crates and SimOxide share the [foundation crates](../development/foundation.md) in
`crates/foundation/`:

| Foundation crate | What QVTo takes from it | Imported as |
|---|---|---|
| `simoxide-emf-core` | metamodel ids and the `MetaRegistry` (dynamic, or seeded from SimOxide's generated tables); `Datum`, the feature values with Java's box kinds; the `ModelStore` trait, through which the VM reads and writes models with EMF's semantics, and its contract tests; `MetaOperation` | `qvto_model` |
| `simoxide-emf` | the EMF graph (`Graph`, a `ModelStore`): `.ecore` registration, Ecore built in, XMI load and save as EMF 2.41, resource sets, URIs, proxies, the canonical dump, the Eclipse suite's comparer, the change journal and id policies | `qvto_emf` |
| `simoxide-jcompat` | Java behaviour that the Java documentation specifies: `String.hashCode` and the boxes' hash codes, `Double.toString` and `Float.toString`, Java's number parsers, `Character` data, `Math.round`/`max`/`min`, insertion-ordered maps | `simoxide_jcompat` |
| `simoxide-jcompat-exact` | compat mode only (feature `compat`): `java.util.HashMap` iteration order and HotSpot's identity-hash generator | `simoxide_jcompat_exact` |

The QVTo crates name the first two by their former names: Cargo's package rename
(`qvto-model = { package = "simoxide-emf-core", .. }`) keeps `qvto_model::` and `qvto_emf::`
in the QVTo sources. No QVTo crate depends on SimOxide's simulator crates. SimOxide reaches the
engine only through `simoxide-reconf` ([QVTo in SimOxide](./simoxide.md)).

What stays in the QVTo crates: QVTo's own Java behaviour on its string type (`java/string.rs` in
`qvto-runtime`), the collections that use Java's `HashMap` order in compat mode, and Java's
regular expressions on the `regex` crate (`qvto-vm/src/jregex.rs`).

## Runtime model

- **Values.** OCL collections are values; an operation returns a new collection, and in-place
  updates happen only when the collection is not shared. `Dict` and `List` are mutable, with
  identity semantics. Integers read from `ELong`, `EShort` and other attributes keep their Java
  box kind, because Java's `equals` tells `Long(1)` from `Integer(1)`.
- **Errors.** Java exceptions inside expressions do not unwind in Eclipse: the expression becomes
  `invalid` and a message goes to stderr. The VM does the same (`Vm::errors`). Only QVT
  exceptions unwind: `raise`, fatal assertions, `QvtRuntimeException`, stack overflow and
  interruption.
- **Extents.** Each model parameter has an extent. A created object goes to the extent that
  Eclipse's rules choose. Objects without a container are roots of an in-memory resource
  `extent:/N` of the store.
- **Engine-local objects.** Instances of QVTo's library classes (`Exception`, `Status`) live in
  the VM. Instances of intermediate classes are store objects with EMF's reference semantics
  when the store takes the program's registry (`ModelStore::extend_meta`).
- **Dispatch.** Contextual operations dispatch through virtual tables that the analyser builds,
  with Eclipse's overriding rules. Operations of dynamic classes with an OCL body run that body.
- **Modes** are a run option, not separate builds of the VM. Java-order code is confined to
  `qvto-runtime`'s collections and identity hashes, behind the feature `compat`
  ([Licensing](./licensing.md)).

The full design, with the reasons for each decision, is `crates/qvto/DESIGN.md` (a developer
document from qvto-rs; its decision log, §12, is cited by code comments).

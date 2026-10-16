# qvto-rs

A QVT Operational Mappings (QVTo) engine in Rust.

- **Complete.** Covers the QVTo language of the OMG QVT 1.3 specification and the OCL it embeds,
  as implemented by Eclipse QVTo, the reference implementation.
- **Two modes.** *Compat mode* behaves like Eclipse QVTo, including its quirks: collection
  iteration order, number and string formatting, null and invalid handling, trace and resolve
  semantics. *Fast mode* keeps the semantics but drops the Java emulation that costs time.
- **Bytecode.** Transformations compile to a register bytecode that a VM interprets.
- **Embeddable.** The engine reads and writes models through a trait, so it runs on its own
  Ecore/XMI layer and, later, directly on SimOxide's model graph.

Status: Q1a (model API, Ecore/XMI layer) and Q1b (parser) done; transformations do not run yet. See `PLAN.md`.

## Documents

| File | Contents |
|---|---|
| `DESIGN.md` | scope, architecture, model API, bytecode and VM, compat vs fast, performance, testing, milestones, licensing |
| `PLAN.md` | milestones and their status |
| `reference/README.md` | specifications, Eclipse sources, the Java reference runner |
| `reference/inventory/` | inventory of the Eclipse QVTo test suite |
| `reference/notes/` | notes on Eclipse QVTo and OCL semantics |

## Layout

| Crate | Contents |
|---|---|
| `qvto-syntax` | lexer, parser, AST, spans, error recovery |
| `qvto-model` | model API: metamodel registry, `ModelStore` trait, contract tests |
| `qvto-emf` | own Ecore/XMI store: EMF semantics, `.ecore` metamodels, XMI load/save as EMF, canonical dump, the suite's comparer |
| `qvto-types` | type system, standard library signatures |
| `qvto-sema` | semantic analysis and type checking |
| `qvto-bytecode` | instruction set, compiler, disassembler |
| `qvto-runtime` | values, collections, Java-compatible hashing and formatting, standard library |
| `qvto-vm` | the interpreter |
| `qvto` | engine facade |
| `qvto-testkit` | conformance runner, differential testing |
| `qvto-cli` | the `qvto` command |

## Building

This machine is shared. Build niced with few jobs:

```sh
CARGO_BUILD_JOBS=4 nice -n 19 cargo build
```

## Licence

Eclipse Public License 2.0, the licence of Eclipse QVTo and of SimOxide. See `DESIGN.md`,
"Licensing".

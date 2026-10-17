# qvto-rs

A QVT Operational Mappings (QVTo) engine in Rust.

- **Complete.** Covers the QVTo language of the OMG QVT 1.3 specification and the OCL it embeds,
  as implemented by Eclipse QVTo, the reference implementation.
- **Two modes.** *Compat mode* behaves like Eclipse QVTo, including its quirks: collection
  iteration order, number and string formatting, null and invalid handling, trace and resolve
  semantics. *Fast mode* keeps the semantics but drops the Java emulation that costs time.
- **Bytecode.** Transformations compile to a register bytecode that a VM interprets.
- **Embeddable.** The engine reads and writes models through a trait, so it runs on its own
  Ecore/XMI layer and directly on SimOxide's model graph (`crates/qvto-simoxide`).

Status: see `PLAN.md`.

## Documents

| File | Contents |
|---|---|
| `DESIGN.md` | scope, architecture, model API, bytecode and VM, compat vs fast, performance, testing, milestones, licensing |
| `PLAN.md` | milestones and their status |
| `docs/simoxide-integration.md` | running the engine inside SimOxide: what exists, SimOxide's work, plan |
| `crates/qvto-cli/README.md` | the `qvto` command; the benchmark suite (`qvto bench`) and its results against Eclipse QVTo |
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
| `qvto-simoxide` | the store over SimOxide's model graph, its metamodel registry, the change journal (a workspace of its own; needs a SimOxide checkout) |

## Building

This machine is shared. Build niced with few jobs:

```sh
CARGO_BUILD_JOBS=4 nice -n 19 cargo build
```

## Licence

Eclipse Public License 2.0, the licence of Eclipse QVTo and of SimOxide. See `DESIGN.md`,
"Licensing".

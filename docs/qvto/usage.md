# Usage

## The `qvto` command

`qvto-cli` builds the `qvto` command. Build it with the `release-unwind` profile:
`conform` and `fuzz` catch panics per case, and the workspace's `release` profile aborts on a
panic.

```sh
cargo build --profile release-unwind -p qvto-cli                    # EPL build: fast and ordered mode
cargo build --profile release-unwind -p qvto-cli --features compat  # with compat mode; do not distribute
cargo qvto run t.qvto in.xmi                                        # alias: compat build, then run
```

The command works inside the repository. It finds the QVTo root (`crates/qvto`), its suite
configuration (metamodels by namespace URI, URI maps) and the reference material from there.

| Command | What it does |
|---|---|
| `qvto run FILE.qvto [MODEL...] [--repeat N] [--fast \| --compat \| --ordered]` | compiles and runs a transformation; models bind to the `in`/`inout` parameters in order; prints the problems, the log, the outcome and the times |
| `qvto disasm FILE.qvto` | prints the problems, the modules, the operations and the bytecode |
| `qvto conform [--suite eclipse\|corpus\|fuzz\|spd\|all] [--engine qvto\|qvto-fast\|qvto-ordered\|...]` | the conformance scoreboard ([Conformance](./conformance.md)); `qvto` (compat) needs the feature `compat` |
| `qvto conform record [--suite ...] [--det] [--runs N]` | runs the Java reference runner N times (default 5) and writes the fixture bundle |
| `qvto bench [--group G] [--only ID,...] [--modes compat,fast,ordered] [--java] [--java-cold]` | the benchmark suite ([Performance](./performance.md)) |
| `qvto fuzz expr\|prog [--seed N] [--minutes M]` | differential fuzzing against the Java runner (generated expressions, generated transformations) |
| `qvto fuzz corpus --cases FILE` | compile problems (and runs) of a case file against the Java runner, for the external corpora |
| `qvto fuzz robust [--workers N] [--minutes M]` | robustness fuzzing: no panic, hang or abort on mutated input |
| `qvto spd-replay [MODELDIR SPD.jsonl] [--mode compat\|fast\|ordered]` | replays the SPD adjustments that the Slingshot reference runner recorded, and compares each adjustment's outcome and model summary |

`qvto` with no arguments prints the full usage, including the options not shown here.

## Library

The `qvto` crate is the facade: it compiles a transformation (parse, analyse, compile to
bytecode) and re-exports the VM.

```rust
use qvto::{Mode, RunOptions, Vm};
use qvto_emf::ResourceSet;                      // simoxide-emf, imported as qvto_emf
use std::{path::Path, rc::Rc};

let mut rs = ResourceSet::new();
rs.load_metamodel("platform:/plugin/my.bundle/model/my.ecore")?;
let input = rs.load_file(Path::new("in.xmi"))?;
rs.resolve_all();
rs.ecore_resource();                            // Ecore's own objects, for eClass() and its operations

let resolver = qvto::units::FsResolver::folder();          // imports relative to the file
let main = resolver.unit_source(Path::new("t.qvto"))?;
let ops = qvto_emf::operations::meta_operations(&rs.graph);
let compiled = qvto::compile_with_operations(main, &resolver, rs.graph.meta().clone(), &ops);
for unit in &compiled.units {
    for p in &unit.problems { eprintln!("{}: {}", unit.source.file_name, p.message); }
}
let program = compiled.program.expect("no errors");

let opts = RunOptions { mode: Mode::Ordered, ..RunOptions::default() };
let mut vm = Vm::new(&program, &mut rs.graph, opts);
vm.set_natives(Rc::new(natives));               // blackbox implementations, if any
let roots = rs.graph.roots(input).to_vec();
let outcome = vm.run_transformation(vec![roots, vec![]]);   // one entry per model parameter
println!("{} {} {}", outcome.severity, outcome.code, outcome.message);
print!("{}", vm.log);
```

- **Compiling.** `compile` takes the main unit, a `UnitResolver` for imports and the
  `MetaRegistry` of the metamodels. `compile_with_operations` adds the metamodels' `EOperation`s,
  and `compile_with_metamodel` adds the OCL derivations of their features
  (`qvto_emf::operations::meta_derivations`). `Compiled::units` holds every unit with its
  problems, which match Eclipse's in severity, position and text. `Compiled::program` is `None`
  when there are errors. A program is immutable and can be run many times.
- **Running.** `Vm::new` takes the program, a model store and `RunOptions`. The store must hold
  the metamodels the program was compiled against, with the same ids (load them in the same
  order). The VM is generic over `qvto_model::ModelStore`. The shared EMF graph
  (`simoxide_emf::Graph`) is the store used everywhere in the repository, including SimOxide's
  model graph. `run_transformation` takes the root objects of each model parameter.
  `RunOutcome` gives Eclipse's severity, code and message, and the number of trace records.
  `Vm::log` holds the log as Eclipse's `WriterLog` writes it; `Vm::errors` holds the evaluation
  errors that Eclipse prints to stderr.
- **Imports.** `units::FsResolver::folder()` resolves imports relative to the importing file.
  `FsResolver { bundles, .. }` resolves them in the source containers of bundle directories, as
  Eclipse's workspace does.

### `RunOptions`

| Field | Default | Meaning |
|---|---|---|
| `mode` | fast (compat in builds with `compat`) | [the mode](./index.md#modes) |
| `id_source` | none | identity hashes: a tape of a Java run, a constant, or another source |
| `max_stack_depth` | 1 000 | Eclipse's call depth limit (`EVALUATION_MAX_STACK_DEPTH`) |
| `max_native_stack` | 1 MB | native stack the run may use; a deeper call fails like the depth limit. A call level takes 2 to 4 KB in release builds, so Eclipse's depth of 1 000 needs a larger thread stack |
| `step_limit` | none | budget of backward jumps and calls; library operations that build collections or strings charge one step per 256 elements |
| `max_collection_size`, `max_string_length` | 2^27 | larger results interrupt the run ("resource limit exceeded") |
| `config` | empty | configuration properties by name, as strings |
| `readonly_guard` | false | Eclipse's read-only guard for `in` models |
| `line_numbers` | true | stack traces with units and lines |
| `coverage` | false | count the entries of each operation (`Vm::coverage`) |

For untrusted transformations, set `step_limit` and keep the size limits. Run the VM on a thread
with a known stack, and set `max_native_stack` to that stack size minus a margin.

## Blackboxes

Eclipse runs Java blackbox libraries. Here a blackbox library is native Rust, in two halves:

- **Signatures** for the analyser, as a `blackbox::Catalog`: units, their modules, the
  metamodels the modules use, and each operation's kind, context, parameters, result and
  *implementation key*. The catalog is data: `Catalog::parse` reads tab-separated rows, the format
  of `qvto-types/data/blackboxes.tsv` (which holds what Eclipse derives from the Java classes of
  the test suite and the corpus). A host writes the same rows for its own libraries.
  `blackbox::BlackboxResolver` puts the units behind a file resolver: first the files, then the
  registered units (Eclipse's `BlackboxRegistry`), then units found by class name.
- **Implementations** for the VM, in `blackbox::Natives`: closures by implementation key,
  `Fn(&mut Vm<S>, NativeCall) -> NativeResult`.
  - `natives.java("public java.lang.String pkg.Lib.echo(java.lang.String)", f)` derives the key
    `pkg.Lib#echo(java.lang.String)` from Java's `Method.toString()`. The same text appears in
    invocation errors.
  - `natives.op(key, f)` registers any other key.
  - `natives.transformation(name, f)` implements a `blackbox transformation`.
  - `natives.eoperation(nsURI, class, name, f)` implements a metamodel operation that generated
    Java code implements in Eclipse.

A native function gets `self` (the receiver, a constructor's new object or a transformation
instance) and the arguments. It uses the VM's `native_*` API: log, cancellation, configuration
properties, object creation, features by name, collections, `List` and `Dict`, model extents, and
per-run library state. `NativeError::exception(class, message)` becomes Eclipse's
`QvtRuntimeException`, with Eclipse's `Java method invocation error` text.

```rust
use qvto::blackbox::{BlackboxResolver, Catalog, Natives, palladio, stdlib};

let catalog = Catalog::parse(&std::fs::read_to_string("host-blackboxes.tsv")?)?;
let resolver = BlackboxResolver {
    inner: &qvto::units::FsResolver::folder(),
    registered: catalog.units("host").to_vec(),
    classpath: Vec::new(),
};
let mut natives = Natives::new();
stdlib::register(&mut natives);                                    // Strings, emf.tools
palladio::register_simulation_time(&mut natives, || clock.now());  // getSimulationTime()
natives.op("host#twice", |_vm, call| {
    let s = qvto::blackbox::java::string(&call.args[0]).unwrap_or_default();
    Ok(qvto_runtime::Value::str(&format!("{s}{s}")))
});
```

| Module | Units | Contents |
|---|---|---|
| `blackbox::stdlib` | `Strings`, `emf.tools` | QVTo's deprecated libraries: signatures and ports of `StringLibrary` and `EmfToolsLibrary` |
| `blackbox::palladio` | SimuLizar's `ProfilesLibrary` and `SimulationStateLibrary`, the architectural templates' `ProfilesLibrary` | signatures; `register_simulation_time`; the stereotype operations are left to the host |

The Java blackboxes of Eclipse's test bundle are ported in the conformance engine
(`qvto-cli/src/bbox.rs`). `qvto/tests/blackbox.rs` shows a host library end to end: helpers,
contextual queries, mappings, errors, cancellation, library state, a blackbox transformation,
and a `blackbox` operation declared in QVTo.

# QVTo engine

SimOxide contains a QVT Operational Mappings (QVTo) engine written in Rust. It runs
transformations as Eclipse QVTo 3.11 on classic Eclipse OCL does, the reference implementation
that SimuLizar and Slingshot use. SimOxide uses it for SimuLizar's reconfiguration rules and
Slingshot's SPD adjustments ([QVTo in SimOxide](./simoxide.md)). The engine also works on its own,
on any Ecore metamodel, as a library or with the `qvto` command line tool.

The engine was developed as the separate repository qvto-rs and imported with its history. Its
crates are in `crates/qvto/`. It shares Java compatibility and the EMF layer with SimOxide through
the [foundation crates](../development/foundation.md).

## What it covers

Everything of chapter 8 of the OMG QVT 1.3 specification that Eclipse implements, with Eclipse's
behaviour where the two differ:

- **Modules:** transformations with `in`, `out` and `inout` model parameters; `modeltype`
  (`strict`, `where`); libraries, `import`, `access`, `extends`; `blackbox` units; nested
  transformations and `transform()`; configuration and module properties; intermediate classes
  and properties; the legacy `mapping main(in x: T)` entry.
- **Operations:** mappings with `when`, `where`, `init`, population, `end`, `inherits`, `merges`,
  `disjuncts` and `abstract`; helpers, queries, constructors, contextual operations, overloading
  and virtual dispatch.
- **Objects and traces:** `object`, `new`, trace records, `map`/`xmap`, the `resolve` family,
  `late resolve`, `deepclone`, incremental update with an input trace.
- **ImperativeOCL:** blocks, variables, assignment, `if`, `while`, `forEach`, `switch`, `break`,
  `continue`, `return`, `raise`, `try`/`except`, `log`, `assert`, `xcollect` and the other
  imperative iterators, `Dict` and `List`, and Eclipse's syntax extensions.
- **Standard library:** QVT 1.3 §8.3 as Eclipse implements it, and the deprecated `Strings` and
  `emf.tools` libraries.
- **OCL:** the classic Eclipse OCL that QVTo embeds: collections and iterators, type operations,
  null and invalid, numbers, strings, tuples, enumerations, with classic OCL's quirks.
- **EMF:** dynamic EMF objects from `.ecore` metamodels, Ecore's own operations, XMI load and save
  byte for byte as EMF 2.41, OCL bodies of `EOperation`s and OCL derivations of features.

Not covered: QVT Relations and Core; Eclipse's editors, debugger, Ant task and workspace builder;
Java blackboxes (blackboxes are Rust functions, see [Usage](./usage.md#blackboxes)); executing
compiled `.qvtox` files; Pivot OCL and complete OCL documents; UML profiles. Syntax that the
specification defines but Eclipse does not accept (`#T`, `==`, `-=`, `default`, `refines`, ...)
is rejected, as in Eclipse.

## Modes

A run has one of three modes (`qvto::Mode`, `RunOptions::mode`). All three keep Eclipse's
semantics: numbers, strings, null and invalid, the mapping protocol, traces, extents, logs,
assertions, exceptions and stack traces. They differ in the order of hash-based collections and
in identity hashes.

| Mode | Set, Bag, Dict order | Identity hashes (`Class@hash` in logs) | Build | Use |
|---|---|---|---|---|
| **fast** | insertion order | derived from the object's key | every build (EPL) | default of the EPL build; throughput |
| **ordered** | insertion order; a `Set` is a `LinkedHashSet`, which classic OCL takes for an OrderedSet | from the key, or from a given source | every build (EPL) | SimOxide's rules and SPD adjustments; Slingshot's reference runner in its `det` mode |
| **compat** | Java's `HashMap` bucket order | HotSpot's generator, or a tape of a Java run | cargo feature `compat` only; such builds may not be distributed | Eclipse QVTo exactly, on a JVM with its default hashing |

- **Fast mode** drops the Java emulation that costs time. It also evaluates an effectful
  `sortedBy` body once, where classic OCL evaluates it twice.
- **Ordered mode** behaves like Eclipse QVTo in a JVM whose QVTo and OCL hash collections iterate
  in insertion order. This is what the Slingshot reference runner's seed agent does, and it is
  the rule of the patched SimuLizar reference (patch P10). So ordered mode reproduces both
  references without any Java-order code.
- **Compat mode** reproduces Java's `HashMap`/`HashSet` iteration order, and HotSpot's identity
  hashes for objects. A plain Eclipse run's object order depends on identity hashes that differ
  between JVM runs. The conformance harness therefore replays the hashes of a recorded Java run
  (a *tape*). Without a tape, compat mode uses a HotSpot-like sequence: deterministic, but equal
  to no particular JVM run.

The default mode is fast in the default build and compat in builds with the feature `compat`.
Asking for compat mode in a build without the feature is an error (`MODE_UNAVAILABLE`), not a
silent fallback. [Licensing](./licensing.md) explains the split.

## Status

All numbers are from the current tree ([Conformance](./conformance.md) describes the oracles):

| Check | Result |
|---|---|
| Eclipse QVTo test suite, SimuLizar and SPD corpus, fuzz regressions: compat mode against the Java runner | **589 of 589** runnable cases strictly equal (models, logs, status, stack traces, problems) |
| the same, fast mode | 589 pass Eclipse's checks; 584 strictly equal (5 fuzz cases log hash-ordered collections) |
| the same, ordered mode against the Java runner with insertion-ordered collections | 583 strictly equal; the other 6 fail Eclipse's checks in that Java runner too, identically |
| analyser against Eclipse's compiler (problems per unit: severity, position, message) | **1 102 of 1 102** units, 2 304 of 2 304 problems |
| ordered mode against the Slingshot reference runner's SPD adjustments | 13 of 13 |

Speed, against Eclipse QVTo in a warm JVM ([Performance](./performance.md)): a SimuLizar rule
runs in about 0.1 ms on a freshly loaded model, 4 to 11 times faster than Eclipse, and in
5 to 35 µs when it runs again on the same model. A rule compiles in under 1 ms, against 200 to
400 ms for the first compile in a fresh JVM.

## Pages

- [Usage](./usage.md): the `qvto` command and the library API.
- [Architecture](./architecture.md): the crates, the pipeline, and how they sit on the foundation
  crates.
- [Conformance](./conformance.md): the oracles, results and known differences from Eclipse.
- [Performance](./performance.md): the benchmark suite and its results.
- [QVTo in SimOxide](./simoxide.md): the reconfiguration host, SimuLizar's rules, SPD
  adjustments, the shared model graph.
- [Licensing](./licensing.md): which modes and builds are EPL-only.
- [Development](./development.md): building, test gates, reference material.

# qvto-simoxide

qvto-rs on SimOxide's model graph (milestone Q6): a `qvto_model::ModelStore` over
`simoxide_model::raw::Graph`, the metamodel registry of SimOxide's generated tables, metamodel
objects for `eClass()`, and a change journal for SimOxide's future syncers. The integration plan
is `docs/simoxide-integration.md`.

```rust
let mut model = simoxide_model::load_dir(dir)?;                     // SimOxide's loader
let mut session = qvto_simoxide::Session::new();                    // registry, Ecore, metamodel objects
session.load_metamodel_text(SRM_LOCATION, SRM_ECORE)?;              // other metamodels (PRM, SPD, ...)
let compiled = qvto::compile_with_metamodel(main, &resolver, session.meta().clone(),
    &session.operations(), &session.derivations());                 // once
session.start_journal();
let mut store = session.store(&mut model.graph);                    // a view, per rule run
let out = qvto::Vm::new(&program, &mut store, opts).run_transformation(args);
let journal = store.session().take_journal();                        // to the syncers
```

A member of the SimOxide workspace; it uses the in-repository `simoxide-model` (extension
tables, `raw::Graph::add_object`). Until M5/M6 of the merge (`docs/merge-into-simoxide.md`) it is
the bridge between the QVTo engine and SimOxide's graph.

```sh
cargo test -p qvto-simoxide
cargo run --release -p qvto-simoxide --example adapter-bench          # time per rule execution, adapter vs qvto-emf
cargo run -p qvto-simoxide --example runtime_flow               # measurement -> rule -> journal -> syncer
```

The corpus, registry and script tests need `target/pcm` (`qvto-emf/tools/pcm-setup.sh
target/pcm`) and skip otherwise; the corpus test also reads the Java run's identity-hash tapes
from `target/qvto-testkit/java` when cached.

## Layout

| File | Contents |
|---|---|
| `src/registry.rs` | `MetaRegistry` from `simoxide_model::meta`: packages, classes, features, enums with SimOxide's ids, data types, complete supertypes, multiplicities and flags from the extension tables, Ecore classifiers resolved after Ecore; SimOxide's `EOperation`s as `MetaOperation`s |
| `src/store.rs` | `Session` (registry, `qvto_emf` resource set for other metamodels and metamodel objects, resource table, mirrors, journal, id policy) and `SimStore` (the `ModelStore`: dispatch by home, objects of other metamodels, resources, `EcoreUtil.delete` over both homes, `extend_meta`, loading models of other metamodels and linking their references into the graph) |
| `src/raw_part.rs` | EMF's semantics on `raw::Graph` storage (the operations of `qvto_emf::Graph` ported: defaults, `isSet`, uniqueness, containment, opposites, container references, roots), driven by the registry alone; every change recorded |
| `src/journal.rs` | `Journal`, `Change`, `ChangeKind` (created, set, unset, added, removed, replaced, moved, container, root added/removed, deleted), `Summary` |
| `src/uuid.rs` | `IdPolicy`: no ids (dynamic EMF, as the corpus's Java runs) or EMF-style UUIDs from a seed (generated PCM code) |
| `tests/contract.rs` | `qvto-model`'s contract suite with objects in the graph and in the other home (40 tests) |
| `tests/registry.rs` | ids equal SimOxide's; the registry equals the one `qvto_emf` builds from the same `.ecore` files; metamodel objects and operations |
| `tests/scripts.rs` | 300 random scripts (about 24 000 operations) on PCM classes, adapter vs `qvto_emf`: every result and the final states equal |
| `tests/corpus.rs`, `tests/common/corpus.rs` | the 24 SimuLizar/SPD corpus cases on PCM models loaded by SimOxide's loader, against the Java runner's fixtures and the `qvto_emf` path |
| `tests/intermediate.rs` | intermediate-class instances in the store, referring to graph objects |
| `examples/adapter-bench.rs`, `examples/rules/scan.qvto` | time per rule execution on SimOxide's MediaStore model, grown to 100 and 1 000 containers |
| `examples/runtime_flow.rs`, `examples/rules/autoscale.qvto` | the runtime flow with `getSimulationTime` on a clock and a stub syncer |
| `data/runtimemeasurement.ecore` | SimuLizar's runtime-measurement metamodel (EPL-2.0, `Palladio-Analyzer-SimuLizar/bundles/org.palladiosimulator.simulizar.runtimemeasurement/model`), embedded by the example |

## Two homes

- **Graph objects**: instances of SimOxide's classes live in the `raw::Graph`; the handle is
  the `raw::ObjId`, metamodel ids are SimOxide's. Reads and writes go to `Obj::slots` in place
  (no copy). A slot is present exactly when EMF's `eIsSet` holds after a write, so SimOxide's
  own `Graph::is_set`/`attr` agree; values keep SimOxide's representation (`Value::Int` for all
  integral kinds, `Other` for lexical kinds).
- **Other objects** (the PRM model, SPD models, metamodel objects, intermediate-class
  instances): a `qvto_emf` graph in the session; handles `EXT_BASE + id`. A reference from
  such an object to a graph object goes through a *mirror* (a proxy object standing for it).
  Not supported (an `Unsupported` error, the VM's `invalid`): graph objects referring to other
  objects, containment or opposites between the homes, an object of one home as a root of a
  resource of the other.
- **Resources**: one table: the graph's resources first (`ResourceId(i)` = `ResId(i)` for the
  resources present when the session first sees the graph), the `qvto_emf` resources, and the
  resources the engine creates (`extent:/N`), whose roots may come from both homes.

## Verification

| Test | Result |
|---|---|
| contract suite, graph home and other home | 40 of 40 |
| registry vs the `.ecore` files (35 packages, 299 classes, 520 features: every property, supertypes, `eAllStructuralFeatures`, IDs, opposites, defaults) | equal; SimOxide's main tables miss the generic supertypes of four `probfunction` samples and `PCMBaseClass`'s explicit `EObject`, the extension tables have them |
| random PCM scripts vs `qvto_emf` | 300 scripts, 23 914 operations: every result and final state equal |
| corpus (12 SimuLizar rule files, 23 cases; SPD `MainTransformation`) on SimOxide-loaded models | 24 of 24: dumps, logs and status equal to the Java runner's and the `qvto_emf` path's |
| performance (release, MediaStore, median of 300 runs, journal on) | 0.9-1.1x the `qvto_emf` store: scaleUp 7.8 us (6 containers), 83 us (100), 0.86 ms (1 000); scan rule 87 us / 97 us / 198 us |

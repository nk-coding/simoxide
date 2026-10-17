# QVTo in SimOxide

SimOxide runs QVTo transformations on a running simulation's model. The changes they make reach
the simulation as SimuLizar's or Slingshot's runtime would carry them. Two kinds of
transformation do this: SimuLizar's reconfiguration rules, and Slingshot's SPD adjustment
(`MainTransformation.qvto`). Both run on the same engine, on the same model graph.

The user-facing pages are in the guide:

- [Reconfiguration rules (SimuLizar)](../guide/reconfiguration.md): `--rules`, the rules log,
  the examples, `RulesRun`;
- [Library API: QVTo transformations on a run](../guide/library.md#qvto-transformations-on-a-run):
  `QvtoCore`, `QvtoRun`, bindings, outcomes;
- [Scaling policies (SPD)](../guide/spd.md): the SPD interpreter and its adjustments;
- [Model changes](../spec/model-changes.md): what a change does to the simulation, and the rules
  of [MC-7](../spec/model-changes.md#mc-7-reconfiguration-rules-qvto) for reconfiguration rules.

This page shows how the pieces fit together.

## The host: `simoxide-reconf`

`simoxide-reconf` is the only SimOxide crate that depends on the engine. The simulator crates
do not, and `simoxide-cli` pulls it in only with its feature `reconfiguration`.

| Part | Role |
|---|---|
| `QvtoCore` | once per process, `Send + Sync`: the metamodel registry, the metamodel objects (for `eClass()` and Ecore's operations), and the Palladio metamodels SimuLizar and Slingshot register besides SimOxide's (runtime measurements, feature model and configuration, completions; embedded from `crates/simoxide-reconf/data`). Transformations are compiled once against it and shared between runs and threads |
| `QvtoRun` | one run's QVTo state, created on first use: the core's metamodel objects in the run's model graph, the id generator, models of other metamodels |
| `Invocation` | how a transformation's parameters bind: as SimuLizar does (`Invocation::simulizar()`: by metamodel), as Slingshot does (`Invocation::slingshot()`), or by name; configuration properties |
| `Outcome` | the transformation's result (success, log, journal, extents) and the `ChangeReport` of the applied changes |
| `RuleSet`, `RulesHost`, `RulesRun` | SimuLizar's reconfiguration process: a rule folder in file-name order, the runtime measurement model (PRM), the triggers, and a run that executes the process on the way ([Reconfiguration rules](../guide/reconfiguration.md#library)) |
| `QvtoAdjuster` | feature `spd`: an `Adjuster` for the SPD interpreter that runs `MainTransformation` with Slingshot's bindings for each adjustment request |
| `Edit` | direct edits through the same change path, without QVTo |

The path of a change is the same for every source:

1. The transformation (or an `Edit`) writes into the run's model graph through the
   `ModelStore` API. The graph records each effective change, side effects included, in its
   *journal*.
2. `change_set` turns the journal into the simulator's `ChangeSet`, keeping the changes of
   objects of SimOxide's metamodels.
3. `Simulation::apply_changes` rebuilds the typed model, recompiles it and merges the run state.
   The run continues as SimuLizar's syncers would let it ([Model changes](../spec/model-changes.md)).
   A change the simulation cannot take returns an error, and the run stays unchanged.

## One graph for SimOxide and QVTo

SimOxide's model graph (`simoxide_model::raw::Graph`) *is* the foundation's EMF graph
(`simoxide_emf::Graph`), and the graph is a `ModelStore`. QVTo therefore runs directly on the
models SimOxide loaded, with no copy and no adapter:

- **Metamodels.** The registry is seeded from SimOxide's generated PCM tables
  (`MetaRegistry::from_static`), so `simoxide_model::meta::ClassId` and the engine's class ids are
  the same numbers. Ecore and the other metamodels follow in the same registry.
- **Other models in the same graph.** The PRM, SPD models, a feature configuration and instances
  of intermediate classes are resources of the run's graph, and they may reference PCM objects.
  The simulator compiles only the objects of its own metamodels. When only other models change,
  `Simulation::set_model_graph` hands it the new graph without a recompilation.
- **Attaching.** The core holds a prototype graph with the metamodel objects. The first QVTo use
  in a run appends it to the run's graph (`QvtoCore::attach`) and sets the id policy.
- **Copy on write.** A run's graph sits behind an `Arc`. Its objects are stored in chunks whose
  frozen prefix is shared. A fork shares the graph, and the first write copies only the
  chunks it touches. A fork with its `QvtoRun` clone draws the same new ids as the original.
- **Ids.** Objects that rules create get ids as generated PCM code gives them, from a seeded
  UUID generator in the graph (`IdPolicy`). The n-th new object of a run gets the same id in
  every run and in the patched SimuLizar reference.
- **Snapshots.** A snapshot of a run with rules holds the QVTo state (where the prototype was
  attached, the objects of other metamodels, the id generator), and a restored run continues
  with it ([Snapshots](../guide/snapshots.md)).

## Modes in SimOxide

| Use | Default mode | Why |
|---|---|---|
| SimuLizar rules (`RulesHost`, `simoxide run --rules`) | ordered | the patched SimuLizar reference iterates QVTo's and OCL's sets in insertion order (patch P10), so ordered mode equals it byte for byte, in every build |
| `QvtoRun::invoke`, and SPD adjustments through `QvtoAdjuster` (`RunConfig::mode` = `QvtMode::Auto`) | compat in builds with `compat` or `exact`, else fast | Eclipse's models in an exact build |
| the same with `QvtMode::InsertionOrderSets` | ordered | the Slingshot reference runner's `det` rule: `qvto spd-replay` equals its recorded adjustments on 13 of 13 |

`simoxide run --qvto-mode auto|ordered|fast|compat` overrides the mode of the rules. Compat mode
needs the feature `compat` (or `exact`, which turns it on). Without it the mode is unavailable
(`ReconfError::ModeUnavailable`).

## How it is checked

- `simoxide-reconf/tests/rules_process.rs`: the 11 cases of `corpus-rules/` (SimuLizar's
  examples and SimOxide's own) give the patched reference's trace, random tape and measurements
  byte for byte. The PRM values are checked against the reference too.
- `tests/corpus.rs`: the 24 SimuLizar and SPD corpus cases of the conformance suite, on models
  run through `QvtoRun::invoke` in the middle of a simulation. Their models, logs and statuses
  equal the Java runner's, and their changes take the change path: applied, refused, or making
  the run fail where SimuLizar aborts.
- `tests/adapter_parity.rs`: the journals, dumps, logs and statuses of those 24 cases equal what
  the former adapter recorded before the engine moved onto SimOxide's graph.
- `tests/changes.rs`, `tests/host.rs`, `tests/snapshot.rs`: changes at pause points, the host's
  API, snapshots of runs with rules.

[Testing](../correctness/testing.md) describes SimOxide's other test layers.

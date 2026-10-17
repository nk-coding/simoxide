# Semantics of Slingshot

These pages specify what the Palladio simulator **Slingshot** does when it simulates a model,
precisely enough to implement SimOxide's planned **Slingshot flavour** in both of its modes
(design `DESIGN-SPD.md` on branch `spd`, decisions U4 and U5):

- **Slingshot-exact**: the same event projection, random draws and measurements, byte for byte, as
  the deterministic Slingshot of the reference runner (`reference/slingshot`, `det` mode,
  [Slingshot runner](../../reference-simulator/slingshot.md));
- **fast**: the same semantics, statistically equal results.

SimOxide does not implement the Slingshot flavour yet (phases S2, X2 of the design). These pages are
its specification, like the [SimuLizar pages](../index.md) are for the SimuLizar mode. The rules are
extracted from the Slingshot sources with file and line references, and wherever possible each rule
has a **probe**: a small model in `reference/slingshot/models/probes/` whose recorded `det` outputs
demonstrate it.

| Page | Rules | Content |
|---|---|---|
| [Event core](./core.md) | `SS-ENG-*`, `SS-BUS-*`, `SS-RUN-*`, `SS-ND-*` | SSJ calendar and double time, the event bus (subscribers, interceptors, result scheduling, disposed subscribers), start and stop, sources of nondeterminism and what the `det` patch fixes |
| [Usage model](./usage.md) | `SS-WL-*` | open and closed workloads, user life cycle, usage actions |
| [SEFF interpretation](./seff.md) | `SS-ACT-*` | entry calls, SEFF actions, composition and role lookup, external calls over the wire, allocation |
| [Resources](./resources.md) | `SS-RES-*` | processor sharing, FCFS, missing delay resources, passive resources, linking resources |
| [Monitoring](./monitoring.md) | `SS-MEAS-*` | monitors that are honoured, probes and calculators, `MeasurementMade`, output |
| [Random numbers](./random.md) | `SS-RNG-*` | the two random streams, draw sites and draw order |
| [Scaling policies (SPD)](./spd.md) | `SS-SPD-*` | the SPD interpreter, triggers, constraints, the QVTo adjustment and how the simulation applies it |
| [Quirks](./quirks.md) | `SS-Qn` | every known Slingshot bug or oddity: observable effect, compat flag, fixed behaviour |
| [Exact comparison](./exact.md) | `SS-X-*` | event projection, tape mapping, the `det` oracle and its verification, the probe set |

## Version and sources

The specified version is the Slingshot nightly `1.0.0.202609010045`, pinned by
`reference/slingshot/install.lock`, built from Slingshot commit `8bf9adc` (2026-08-21). All
sources are read-only under `/home/devbox/workspace/old/palladio-research/repos/`. Line numbers
refer to these commits.

| Tag | Path prefix (under `Palladio-Analyzer-Slingshot/bundles/org.palladiosimulator.analyzer.slingshot.`, except `[TR]`, `[SSJ]`, `[RUN]`) |
|---|---|
| `[CORE]` | `core/src/org/palladiosimulator/analyzer/slingshot/core/` |
| `[ED]` | `eventdriver/src/org/palladiosimulator/analyzer/slingshot/eventdriver/` |
| `[COM]` | `common/src/org/palladiosimulator/analyzer/slingshot/common/` |
| `[CU]` | `common.utils/src/org/palladiosimulator/analyzer/slingshot/common/utils/` |
| `[US]` | `behavior.usagesimulation/src/org/palladiosimulator/analyzer/slingshot/behavior/usagesimulation/` |
| `[USD]` | `behavior.usagesimulation.data/src/org/palladiosimulator/analyzer/slingshot/behavior/usagemodel/` |
| `[USM]` | `behavior.usagesimulation.monitor/src/org/palladiosimulator/analyzer/slingshot/behavior/usagesimulation/monitor/` |
| `[SY]` | `behavior.systemsimulation/src/org/palladiosimulator/analyzer/slingshot/behavior/systemsimulation/` |
| `[SYD]` | `behavior.systemsimulation.data/src/org/palladiosimulator/analyzer/slingshot/behavior/systemsimulation/` |
| `[SYM]` | `behavior.systemsimulation.monitor/src/org/palladiosimulator/analyzer/slingshot/behavior/systemsimulation/monitor/` |
| `[RS]` | `behavior.resourcesimulation/src/org/palladiosimulator/analyzer/slingshot/behavior/resourcesimulation/` |
| `[RSD]` | `behavior.resourcesimulation.data/src/org/palladiosimulator/analyzer/slingshot/behavior/resourcesimulation/` |
| `[RSM]` | `behavior.resourcesimulation.monitor/src/org/palladiosimulator/analyzer/slingshot/behavior/resourcesimulation/` |
| `[MON]` | `monitor/src/org/palladiosimulator/analyzer/slingshot/monitor/` |
| `[SPD]` | `behavior.spd/src/org/palladiosimulator/analyzer/slingshot/behavior/spd/` |
| `[ADJ]` | `behavior.spd.adjustment/src/org/palladiosimulator/analyzer/slingshot/behavior/spd/` |
| `[TR]` | `repos/Palladio-Addons-SPD-Metamodel/bundles/org.palladiosimulator.spd.semantic.transformations/transformations/` (commit `95cdb0b`) |
| `[SSJ]` | SSJ 3.3.1, `plugins/ca.umontreal.iro.simul.ssj_3.3.1.jar` of the install, decompiled with CFR 0.152: `umontreal.ssj.simevents.{Simulator, Event, eventlist.SplayTree}` |
| `[RUN]` | `reference/slingshot/src/` of this repository (the reference runner) |

SimuCom framework classes (`StackContext`, `SimulatedStackframe`, `SimuComConfig`, the
probability-function factory) are those of the Palladio 6.0.0 nightly of the same install and are
cited by class name. Where they match SimuLizar 5.2.2, the SimuLizar pages apply
([StoEx](../stoex.md), [random numbers](../random.md), ACT-5 frames in [Actions](../actions.md)).

## How to read the rules

- Rules are numbered `SS-<family>-<section>.<n>`. A rule states what Slingshot does, with its
  source; **SimuLizar:** notes say how the [SimuLizar rules](../index.md) differ, so that the
  flavour hooks of `DESIGN-SPD.md` §5.2 can be derived.
- **Probe** names a probe model (`reference/slingshot/models/probes/<name>/`, `probe.json` lists
  the rules it demonstrates; `expected/` holds its `det` outputs) or a corpus model run through the
  runner (`corpus/<name>`, results in `reference/slingshot/results/det-verify.csv`).
- Bugs and oddities carry a quirk id `SS-Qn` ([Quirks](./quirks.md)). Following decision U5, the
  Slingshot flavour **fixes** each bug by default and reproduces it under a compat flag; the
  Slingshot-exact mode turns all compat flags on. Behaviour that is Slingshot's design rather than
  a bug (double time, instantaneous utilisation, first allocation match, `Math.random()` branches,
  ...) is simply the flavour's semantics and has no flag.
- "Result order" below always means the order in the `det` oracle: insertion order of the handler's
  result set ([SS-ND-2](./core.md#ss-nd-nondeterminism-and-the-det-patch)). Stock Slingshot picks a
  run-dependent permutation.
- The scope is what the reference runner loads: PCM repository, system, resource environment,
  allocation, usage model, monitor repository, SPD and semantic SPD models. Usage evolution
  (DLIM) is not loaded yet (decision U9) and is only summarised.

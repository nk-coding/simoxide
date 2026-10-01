# Patches

[refsim](./refsim.md) runs the unmodified SimuLizar 5.2.2 product jars. The jars are referenced in place from
`palladio-research/work-simucom/palladio-5.2.2/plugins` and put on a flat classpath by
`tools/classpath.sh` (see "Classpath" below).

Changes enter in three ways:

1. **Shadowed classes** in `patches/src/`. These are copies of the `releases/5.2.2` sources with
   `// REFSIM` edits.
   - They are compiled into `build/classes-patches`, which comes first on the classpath.
   - Diffs against the tag sources: `patches/diffs/*.diff` (regenerate with `tools/mkdiffs.sh`).
2. **Runtime state fixes** in `src/refsim/{Bootstrap,Statics}.java`. No class is replaced.
3. **Extensions, not patches.**
   - The `refsim` recorder is registered through the standalone extension registry and replaces EDP2.
   - The DESMO-J engine component (see "Engine" below).

Tag sources used: `palladio-research/src-5.2.2/*` (shallow clones of `releases/5.2.2`).
`tools/verify-sources.sh` checks that they match the product bytecode. It compiles the pristine source
of every shadowed class with `javac -g`, decompiles both it and the jar class with CFR, and diffs the
normalised output. The only remaining differences are javac-vs-ECJ artefacts (`for` vs `while`,
`finally` vs catch-and-rethrow, string-concatenation style).

`REFSIM_UNPATCHED=1 ./refsim ...` runs the stock classes and skips the state fixes.
`REFSIM_EXCLUDE=Class1,Class2 ./refsim ...` drops individual shadowed classes. Both exist for the evidence
runs below.

## Classpath

OSGi gives each bundle its own class space; a flat classpath gives the first jar that holds a class.
`tools/classpath.sh` (via `tools/classpath.py gen`) writes `build/classpath.txt`, and `build.sh`,
`refsim` and all `oracles/*` use that one file.

**Rules.**
- One jar per bundle symbolic name, highest version. Antlr 3 and 4 are both kept (different packages).
- Nested `Bundle-ClassPath` jars are extracted to `build/lib/<bsn>/` and placed in their declared order.
- Bundles are in sorted symbolic-name order.
- No bundle-private copy may shadow an exported one. An entry that holds a class whose package its own
  bundle does not export, while another bundle does export it, moves to the end of the classpath.
- `tools/classpath.py check build/classpath.txt [-v]` lists every class found in more than one jar.
  It fails when:
  - the first copy is a private one while an exporter exists;
  - several bundles (or none) export the class and it is not in its reviewed list.
  `classpath.sh` runs the check on every regeneration; the report is in `build/classpath-check.txt`.

**Duplicate sets in 5.2.2, reviewed** (803 entries, 7 sets):

| classes | jars (first wins) | OSGi | why the order is right |
|---|---|---|---|
| `org.apache.commons.math{,.analysis,.distribution,.special,.util}` (55) | `org.apache.commons.math_2.1.0` then `desmoj-2.3.3-core-bin.jar` | `probfunction.math` and the other Palladio bundles `Require-Bundle` `org.apache.commons.math` 2.1.0; `de.desmoj` exports only `desmoj.*`, so its embedded old Commons Math is private | With desmoj first, `Norm`, `Lognorm`, `LognormMoments`, `Gamma` and `GammaMoments` would sample with the old library ([RND-9.2](../spec/random.md)). The DESMO-J classes that use Commons Math (`desmoj.core.dist.ContDistGamma`, `ContDistBeta`, `ContDistCustom`, `DiscreteDistBinomial`, `DiscreteDistHypergeo`, `Function`, `desmoj.core.statistic.ConfidenceCalculator`) would bind the private copy in OSGi, but refsim never loads them |
| `org.eclipse.jdt.core.compiler.*`, `org.eclipse.jdt.internal.compiler.*` (441 + 4) | `org.eclipse.jdt.core.compiler.batch` / `org.eclipse.jdt.core`, then `org.apache.jasper.glassfish` | jasper embeds a private JDT compiler; JDT bundles export theirs | exporter first; not loaded by refsim |
| `org.eclipse.jdt.core` (2) | `jdt.core.compiler.batch`, then `jasper.glassfish` | private in both | not loaded by refsim |
| `javax.el` (46) | `com.sun.el.javax.el_3.0.0`, then `javax.el-api_3.0.3` | two exporters, chosen per importer | JSP/help only; not loaded by refsim |
| `javax.servlet{,.annotation,.descriptor,.http}` (79) | `jakarta.servlet-api_4.0.0`, then `javax.servlet_3.1.0` | two exporters, chosen per importer | help/Jetty only; not loaded by refsim |
| `org.opengis.referencing.cs` (1) | `org.jscience_4.3.1.jar`, then its nested `lib/geoapi.jar` | same bundle, `Bundle-ClassPath: .,lib/javolution.jar,lib/geoapi.jar,…` | `.` comes first, as declared |

Checked with `-Xlog:class+load` over a full `refsim batch ../corpus`:
- no class from `javax.el`, `javax.servlet`, `org.eclipse.jdt`, `org.apache.jasper`, `org.opengis` or the
  DESMO-J classes listed above is loaded;
- all 39 `org.apache.commons.math` classes come from the 2.1.0 jar.

The desmoj jar also embeds `org.apache.commons.collections` (72 classes loaded from it). No other jar
has these classes, so OSGi and the flat classpath bind the same copy.

The order matters for 8 corpus models, whose sample values of these distributions differ between the
two libraries (draw and measurement counts do not): `h05_open_workload`, `h06_closed_think`,
`h21_stoex_distributions`, `x_pem_screencast_ms{,_enqueue,_instant}`,
`x_pem_simple_optimisation{,_composites}`.

## Engine

SimuLizar picks the engine from the Eclipse preference `simulationEngineId`. Its default is the **first
registered `de.uka.ipd.sdq.simulation.abstractsimengine.engine` extension**
(`SimulationPreferencesHelper.getDefaultEngineId`). In the 5.2.2 product exactly one bundle contributes
one: `de.uka.ipd.sdq.simulation.abstractsimengine.desmoj` (engine id `de.uka.ipd.sdq.simucomframework.desmoj.engine1`,
DESMO-J 2.3.3). The SSJ engine is not shipped. `Runner.java` therefore binds `DesmoJSimEngineFactory`
directly, as the standalone SimuLizar bootstrap in `palladio-research/work-simucom/standalone` does.

## Patch list

| # | Where | Change | Why | Semantics |
|---|---|---|---|---|
| P1 | `SimuComDefaultRandomNumberGenerator` | Draws synchronously: `random()` returns `rndNumberGenerator.nextDouble()` directly instead of taking it from a 1000-element queue filled by a producer thread. Calls `Trace.uniform(u)`. `refsimReset()` resets the static stream counter | Random tape. Removes the leaked producer thread per run | The queue is FIFO and filled by one thread from one generator, so the consumer sees the identical sequence. Seeding is unchanged: `useFixedSeed` + `fixedSeed0..5` → `MT19937.setSeed(int[6])` |
| P2 | `SimProcessorSharingResource` | `Hashtable<process,Double> running_processes` → `LinkedHashMap`. `getRemainingDemand`'s `running_processes.contains(p)` → `containsValue(p)`. The scheduler oracle uses the same patch (`reference/oracles/sched/patched/`) | `scheduleNextEvent()` finishes the *first* process with minimal remaining demand in iteration order. `Hashtable` order follows identity hash codes, so equal-demand ties were broken at random (spec ND-2) | Only tie order changes: the earliest inserted wins. Re-putting an existing key keeps its position, as in `Hashtable`. `Hashtable.contains` tests values, so the original bug (always false for a process) is kept. `toNow()` updates each entry independently, so its order has no effect (ND-3) |
| P3 | `ResourceTableManager` | `ConcurrentHashMap` → synchronized `LinkedHashMap`. `waitForProcesses()` iterates insertion-ordered snapshots and repeats until no unvisited process is left | At simulation end, `waitForProcesses()` re-activates all suspended processes so that they finish (spec SIM-6.5 / ND-4). The drain order was identity-hash order | The drain order becomes creation order of the resource-table entries. The drain itself stays: processes run to completion without waiting and can still record measurements at the final time. Stock CHM iteration would also visit processes added during the drain; the loop keeps that |
| P4 | `AbstractSimProcessDelegator` | `refsimReset()` sets the static `processIdGenerator` to 0. Constructor calls `Trace.bindControl` and `Trace.newProcess` | Batch runs equal fresh-JVM runs (ND-10). Trace process ids | Ids only appear in names and request-context strings |
| P5 | `SimuComSimProcess` | `refsimReset()` resets the static `sessionID`. Trace hooks: `hold`, process end, thread→process binding | Same as P4 | Session ids are only used for failure statistics |
| P6 | `Statics.java` (reflection) | Resets the private static `resourceId` counters of `ScheduledResource` and `SimulatedLinkingResource` to 1 before every run | ND-10 | These are scheduler resource names ("1", "2", …) |
| P7 | `Bootstrap.java` | `RepositoryComponentSwitch.SYSTEM_ASSEMBLY_CONTEXT.setId("_SYSTEM_ASSEMBLY_CONTEXT_")` | The static synthetic system assembly context gets a random UUID once per JVM (ND-9). It shows up in the interpreter's assembly-context stack and in the trace | The id is copied into the per-call system contexts. SimuLizar strips it from FQ component ids (`ComposedStructureInnerSwitch.getFQComponentID`), so it is used only as an identifier |
| P8 | trace/tape hooks: `EventDispatcher`, `RDSeffSwitch`, `RDSeffPerformanceSwitch`, `UsageScenarioSwitch`, `TransitionDeterminer`, `StackContext`, `AbstractScheduledResource`, `SimSimpleFairPassiveResource`, `ClosedWorkloadUser`, `OpenWorkload`, `AbstractExperiment` | Add calls to `refsim.trace.Trace`, guarded by `Trace.ON` / `Trace.TAPE`. Set and restore the tape origin around StoEx evaluations (try/finally) | Event trace and random tape ([Output formats](../guide/formats.md)) | Observation only: no model state is read that the original did not read, and nothing is written. Evaluations happen in the same order with the same arguments. A disabled trace costs one static boolean read |

Other determinism measures that are not patches:
- **Seed.** `--seed N` → `useFixedSeed=true`, `fixedSeed_i = N + i` (i = 0..5). The SimuLizar UI default
  0..5 therefore equals `--seed 0`.
- **One run at a time.** A batch runs simulations sequentially in one JVM. Each run gets its own RNG,
  `ProbabilityFunctionFactory` RNG and `StoExCache` (ND-6).
- **Deterministic registry.** Plugin jars are contributed to the extension registry in sorted file-name
  order (ND-16).
- **refsim recorder.** It replaces EDP2 (ND-14, ND-17 do not apply) and records every measurement in
  emission order.

Every source of nondeterminism of the [simulation spec](../spec/simulation.md) (ND-1 to ND-17) and how it is handled:

| ND | Status |
|---|---|
| ND-2 | P2 |
| ND-3 | no effect |
| ND-4 | P3 |
| ND-5 | fixed seed |
| ND-6 | sequential runs; P1 creates a fresh RNG per run |
| ND-7, ND-8 | not reachable: failures off, no exact schedulers |
| ND-9 | P7 |
| ND-10 | P4, P5, P6 |
| ND-11 | no effect: strict hand-off |
| ND-12, ND-13 | deterministic; SimOxide emulates the `java.util.HashMap<String>` iteration order (corpus `h27_collection_inner_multi`) |
| ND-14, ND-17 | EDP2 not used |
| ND-15 | PRM recorders, not used |
| ND-16 | sorted registry |
| ND-1 | Dagger `HashSet` of extension-contributed listeners. No effect observed: see the determinism results below, where 61 models are byte-identical across JVMs, including the trace. Watch it if extensions are added |

## Evidence

`tools/evidence.sh` and `verify-determinism.sh` check the patches on the corpus. The results below are
from the 61 models the corpus had when they were last run.

1. **Determinism, patched** (`verify-determinism.sh`).
   - Two fresh JVMs produce byte-identical trace, tape and measurements.
   - Within one JVM, every model run 3× is identical.
   - The results equal the stored `corpus/*/expected`.
   - A single `refsim run` in a fresh JVM, without warm-up, equals the batch output (checked for
     h13, h26, x_pem_screencast_ms).
2. **Stock 5.2.2 vs patched measurements** (`REFSIM_UNPATCHED=1`, two fresh JVMs `u1`, `u2`;
   measurements only, since the stock build has no trace hooks).
   - 56–57 of 61 models are identical to the patched run.
   - The rest differ: `x_pem_harddisk`, `x_pem_screencast_ms`, `…_enqueue`, `…_instant`, and in one of two
     evidence runs `x_ss_mediastore`.
   - Stock SimuLizar is nondeterministic on these models. `u1` and `u2` differ on the three screencast
     models, and in another evidence run also on `x_pem_harddisk`.
3. **Per-patch exclusion** (`REFSIM_EXCLUDE`, measurements compared with the fully patched run).
   - **Without P1+P4+P5+P8** (all trace hooks, synchronous RNG, id resets): identical on all 61 models.
     So these patches do not change simulation results.
   - **Without P2** (PS order): measurements identical on all 61 models. But traces are no longer
     reproducible within one JVM for `h02_ps_ties`, `h13_passive_contention`, `h25_deterministic_closed`,
     `x_espresso` and `x_sl_simulizar154` (repeat 3). So P2 changes only the order of equal-time
     completions, and that order was random before.
   - **Without P3** (drain order): exactly the models of item 2 differ, in both evidence runs. Every
     differing row has `time = t_end`, i.e. it is a post-stop measurement; for example, in
     `x_pem_harddisk` the `Resource Demand Tuple` rows at `t = 1677.328125`. This accounts for every
     stock-vs-patched difference.

## Known limitations of the reference (reproduced, not patched)

- **Double resume** (spec SIM-4.4a). A parent receives one resume per finished sync fork child. If two
  children finish before the parent runs, the second resume wakes the parent early from its next wait.
  In the trace, `demand_done` follows at the same `t`. See `corpus/h26_fork_double_resume`.
- **Post-stop drain** (SIM-6.5). Remaining processes run to completion after `stop`. This can emit
  measurements at `t_end`: for example `x_pem_harddisk` has many `Resource Demand Tuple` rows at the
  final time.
- **Recursive external calls.** They break SimuLizar's response-time calculator ("First measurement to
  the same context arrived …"). The affected Slingshot `minimalexample` models are imported without
  external-call monitors.
- **Linking resources.** `simulateLinkingResources=true` (middleware marshalling) needs `stream.BYTESIZE`,
  which only SimuCom's middleware completion provides. The corpus uses the UI defaults
  (`false` / `simulateThroughputOfLinkingResources=true`), except `h30_middleware_stream`, whose calls pass the
  stream explicitly. Containers on different nodes need a
  `LinkingResource` route even when throughput simulation is off.

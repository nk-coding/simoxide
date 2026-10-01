# Simulation core (SimuLizar 5.2.2)

This page specifies how SimuLizar 5.2.2 produces its event sequence: the engine, simulation time, the
event list, the process model, stop conditions, initialisation order and the sources of
nondeterminism. `crates/simoxide-sim` reproduces these semantics; the event sequence and the
measurements of a SimOxide run are identical to the reference's.

| Page | Content |
|---|---|
| Simulation core (this page) | engine, time, event list, process model, stop conditions, init order, nondeterminism (rules `SIM-*`) |
| [Workloads](./workloads.md) | usage model interpretation, open/closed workloads (rules `WL-*`) |
| [Actions](./actions.md) | RDSEFF interpretation, composition, parameters, passive resources, forks, network (rules `ACT-*`) |
| [Measurements](./measurements.md) | what is measured, when, in which order (rules `MEAS-*`) |
| [StoEx](./stoex.md), [random numbers](./random.md), [schedulers](./scheduler.md) | referenced, not repeated |

Rules are numbered `SIM-<section>.<n>`. The *supported scope* is SimuLizar's static performance
simulation: no failure or reliability simulation, no exact OS schedulers, no reconfiguration rules, no
event channels. Statements such as "never occurs" refer to this scope.

## Source abbreviations

All paths are in the 5.2.2 release tags (`releases/5.2.2`) of the Palladio repositories. DESMO-J has no
source in the repositories; it is decompiled (Vineflower 1.10.1) from the product jar
`plugins/de.desmoj_2.3.3.jar!/desmoj-2.3.3-core-bin.jar`, and DESMO-J citations name class and method.

| Tag | Path prefix |
|---|---|
| `[ASE]` | `Palladio-Simulation-AbstractSimEngine/bundles/de.uka.ipd.sdq.simulation.abstractsimengine/src/de/uka/ipd/sdq/simulation/abstractsimengine/` |
| `[ASE-DJ]` | `Palladio-Simulation-AbstractSimEngine/bundles/de.uka.ipd.sdq.simulation.abstractsimengine.desmoj/src/de/uka/ipd/sdq/simulation/abstractsimengine/desmoj/` |
| `[DJ]` | DESMO-J 2.3.3, package `desmoj.core.simulator` (decompiled) |
| `[SC]` | `Palladio-Analyzer-SimuCom/bundles/de.uka.ipd.sdq.simucomframework/src/de/uka/ipd/sdq/simucomframework/` |
| `[SCC]` | `Palladio-Analyzer-SimuCom/bundles/de.uka.ipd.sdq.simucomframework.core/src/de/uka/ipd/sdq/simucomframework/core/` |
| `[SCV]` | `Palladio-Analyzer-SimuCom/bundles/de.uka.ipd.sdq.simucomframework.variables/src/de/uka/ipd/sdq/simucomframework/variables/` |
| `[SIMC]` | `Palladio-Analyzer-SimuCom/bundles/de.uka.ipd.sdq.simulation.core/src/de/uka/ipd/sdq/simulation/core/` |
| `[SCH]` | `Palladio-Simulation-Scheduler/bundles/de.uka.ipd.sdq.scheduler/src/de/uka/ipd/sdq/scheduler/` |
| `[SL]` | `Palladio-Analyzer-SimuLizar/bundles/org.palladiosimulator.simulizar/src/org/palladiosimulator/simulizar/` |
| `[SLC]` | `Palladio-Analyzer-SimuLizar/bundles/org.palladiosimulator.simulizar.core/src/main/java/org/palladiosimulator/simulizar/core/` |

The product jars were spot-checked against the tag sources (DesmoJSimProcess, TransitionDeterminer,
RDSeffSwitch): identical.

## SIM-1 Engine selection

- **SIM-1.1** The engine is **DESMO-J 2.3.3** through the adapter `[ASE-DJ]`. SSJ is not used.
  - The engine comes from the Eclipse preference `SimulationPreferencesHelper.getPreferredSimulationEngine()`, default =
    the first registered `abstractsimengine.engine` extension
    (`Palladio-Analyzer-SimuCom/bundles/de.uka.ipd.sdq.simulation/src/.../preferences/SimulationPreferencesHelper.java:20-56`,
    `[SLC]runtimestate/SimulationPreferencesSimEngineFactoryProvider.java:20-27`).
  - The 5.2.2 product contains exactly one engine plugin, `de.uka.ipd.sdq.simulation.abstractsimengine.desmoj_5.2.2.jar`
    (engine id `de.uka.ipd.sdq.simucomframework.desmoj.engine1`). The SSJ bundle
    (`org.palladiosimulator.simulation.abstractsimengine.ssj`) exists in the repository but is not shipped.
  - The standalone runner (`work-simucom/standalone/src/standalone/StandaloneSimuLizar.java:223-229`), the
    SimuLizar test harness (`TestSimEngineComponent`) and the deterministic reference also hard-wire
    `DesmoJSimEngineFactory`.
- **SIM-1.2** One `DesmoJSimEngineFactory` per simulation. It creates one `DesmoJModel` and one DESMO-J
  `Experiment` (`[ASE-DJ]DesmoJSimEngineFactory.java:21-32`, `[ASE-DJ]DesmoJExperiment.java:23-37`).
  The event list implementation is `EventTreeList` (`[DJ]Experiment.setupExperiment`: `new EventTreeList()`).
  Real-time mode is off (`_executionSpeedRate == 0`), progress bar off, no output files.

## SIM-2 Simulation time

- **SIM-2.1** Time is an integer number of **nanoseconds** (`long`). `DesmoJExperiment` passes
  epsilon = `TimeUnit.NANOSECONDS`, reference unit = `TimeUnit.SECONDS` (`[ASE-DJ]DesmoJExperiment.java:27-34`;
  `[DJ]Experiment.setupExperiment` → `TimeOperations.setEpsilon/setReferenceUnit`, static globals).
- **SIM-2.2** Every delay `d` (a Java `double`, seconds) handed to the engine is converted with
  `span_ns = (long)(d * 1e9)` (`[DJ]TimeSpan(double,TimeUnit)`: `(long)(var1 * (double)epsilon.convert(1, SECONDS))`).
  - Java `(long)` truncates toward zero, maps NaN to 0 and saturates at ±2^63. Rust's `(d * 1e9) as i64`,
    which SimOxide uses (`simoxide_sched::time::span`), has identical semantics. The product `d * 1e9` is one
    IEEE multiply (1e9 is exact).
  - If `span_ns < 0` → `SimAbortedException` (simulation aborts, SIM-6.6). So delays in (-1e-9, 0) are accepted
    as 0; delays ≤ -1e-9 abort.
  - Confirmed with a DESMO-J probe program: 0.29999999999 s → 299 999 999 ns; 0.3000000009 s → 300 000 000 ns;
    (0.1+0.2) s → 300 000 000 ns.
- **SIM-2.3** Absolute event time = `now_ns + span_ns` (`[DJ]TimeOperations.add(TimeInstant,TimeSpan)`); if
  `now_ns > 0 && Long.MAX - span - now < 1` → abort.
- **SIM-2.4** The time seen by all model code is `getCurrentSimulationTime() = (double)now_ns / 1e9`
  (`[ASE-DJ]DesmoJExperiment.java:40-42` → `[DJ]TimeInstant.getTimeAsDouble(SECONDS)`:
  `(double)_timeInEpsilon / (double)epsilon.convert(1, SECONDS)`). It is a division by 1e9, not a
  multiplication by 1e-9; SimOxide divides as well (`simoxide_sched::time::seconds`).
- **SIM-2.5** Consequence: all timestamps (measurements, scheduler bookkeeping) are multiples of 1 ns expressed
  as `ns/1e9`; demands/delays smaller than 1 ns become zero-length events. Schedulers
  ([schedulers](./scheduler.md)) compute remaining demands from these quantized doubles.
- **SIM-2.6** The clock starts at 0 (`[DJ]Experiment.start()` → `start(new TimeInstant(0L))`) and only moves
  forward (`[DJ]Scheduler.processNextEventNote`: `advanceTime` only if next note is later; earlier → abort).

## SIM-3 Event list and ordering

- **SIM-3.1** The pending-event set is a list ordered by time; **ties are FIFO by insertion order**.
  `[DJ]EventTreeList.insert` binary-searches and inserts the new note **after the last note whose time is ≤ the
  new time**. SimOxide orders notes by `(time_ns, seq)` with `seq` = global insertion counter (incremented on
  every insert, including re-inserts).
- **SIM-3.2** No priorities exist. `insertAsFirst` (LIFO) is only used when a `TimeSpan` is *reference-equal* to
  the constant `TimeSpan.ZERO` (`[DJ]Scheduler.schedule(Entity,EventAbstract,TimeSpan)`: `if (var3 == TimeSpan.ZERO)`).
  All Palladio code passes `new TimeSpan(double)`, so a zero delay is an ordinary FIFO insert at `now`
  (probe program: an event scheduled at delay 0 from inside an event at time 0 runs after all other events
  already queued at time 0).
- **SIM-3.3** Cancelling an event (`removeEvent` → `[DJ]EventAbstract.cancel`) removes its note. Re-scheduling a
  cancelled event object is a fresh insert (new `seq`), i.e. it moves behind every note already queued at the
  same time. Schedulers do this on every arrival (`scheduleNextEvent`: `removeEvent(); schedule(...)`,
  `[SCH]resources/active/SimProcessorSharingResource.java:66-84`, `SimFCFSResource.java:50-57`). An event object
  that is still scheduled cannot be scheduled again (warning, ignored: `[DJ]Event.schedule`).
- **SIM-3.4** Main loop (`[DJ]Experiment.proceed`, status 2 loop):
  ```
  loop:
    if list empty: status := stopped; break                 # processNextEventNote returns false
    note := first(list); if note.t > now: now := note.t     # SIM-2.6
    remove note; run note.routine()                          # synchronous, see SIM-4
    for c in stopConditions: if c.check(): status := stopped; break   # SIM-6
  ```
  The event routine runs to completion, including all process execution it triggers (SIM-4.3), before the
  next note is taken. A superseded (cancelled) scheduler completion is not a processed note and triggers no
  stop check.
- **SIM-3.5** Event kinds that occur in the supported scope (all FIFO per SIM-3.1):

  | Kind | Scheduled by | Routine |
  |---|---|---|
  | `Resume(p)` | `p.scheduleAt(d)` (= `activate()` when d=0) | `if !p.terminated: resume p` (SIM-4.2) |
  | `Delay(p)` | `hold(d)` via the per-process `SimDelayResource` | SIM-4.6 |
  | `PS.Finished(p)`, `FCFS.Finished(p)` | schedulers | [schedulers](./scheduler.md) |
  | `Periodic(entity)` | sliding windows (TimeDriven utilisation) and TimeDrivenAggregation, scheduled at init | MEAS-6.3: routine, then re-insert at `now+span(inc)` |
  | `Resume(reconf)` | `Reconfigurator` on a runtime-measurement write (`triggersSelfAdaptations`) | MEAS-7.2 |
  | `PassiveResourceTimeout` | only if failures are simulated (off) | — |
  | `ResourceFailed/Repaired` | only if failures are simulated (off) | — |

## SIM-4 Process model (threads → state machines)

SimuLizar runs every simulated user, open-workload generator and forked behaviour as a Java thread that
executes as a coroutine: exactly one thread (the DESMO-J main thread or one process) runs at any time,
handed over with two semaphores (`[ASE]processes/AbstractSimProcessSemaphoreStrategy.java:9-30`). There is no
real concurrency, so the thread timing does not influence results. SimOxide replaces each thread with a
state machine plus an explicit continuation stack.

- **SIM-4.1 Creation.** `new SimuComSimProcess(...)` (`[SCC]SimuComSimProcess.java:63-85`) →
  `AbstractSimProcessDelegator` (`[ASE]AbstractSimProcessDelegator.java:26-30`) → `DesmoJSimProcess` ctor
  (`[ASE-DJ]DesmoJSimProcess.java:29-37`) starts the thread and blocks until the thread has parked itself in
  state `SUSPENDED` (`[ASE]processes/SimulatedProcess.java:60-72`). `lifeCycle()` has **not** run yet; creation
  schedules nothing. Each process also gets its own `SimDelayResource` (`<name>_thinktime`) used for `hold`.
  Process ids: `static AtomicLong` counter, global per JVM (`[ASE]AbstractSimProcessDelegator.java:15,42-44`).
- **SIM-4.2 States** `SUSPENDED | RUNNING | TERMINATED`. `resume()` requires `SUSPENDED` (else
  `IllegalStateException`), sets `RUNNING` and blocks the caller until the process suspends again or terminates
  (`SimulatedProcess.java:106-118`). `suspend()` requires `RUNNING`, sets `SUSPENDED`, notifies listeners
  (`SimuComSimProcess.notifySuspending/Resuming`: no effect without failure simulation), and parks
  (`SimulatedProcess.java:86-104`).
- **SIM-4.3 Activation** `p.scheduleAt(d)` (`[ASE-DJ]DesmoJSimProcess.java:39-61`):
  ```
  if p.terminated: return
  require p.state == SUSPENDED                      # else IllegalStateException
  if experiment is stopped (DESMO-J status 1): p.resume()   # synchronous "drain", SIM-6.5
  insert Resume(p) at now + span(d)                 # always, also in the stopped case
  ```
  `activate()` = `scheduleAt(0)` (`[SCC]SimuComSimProcess.java:88-90`). The caller keeps running; `p` runs only when
  its `Resume` note is reached. The `Resume` routine is `if (!isTerminated()) resume()`.
- **SIM-4.4 Passivation.** `p.passivate()` = `suspend()` without scheduling anything
  (`SimulatedProcess.java:80-82`); something else must `activate` p later. `passivate(double)` (hold via
  ExternalEvent, `DesmoJSimProcess.java:63-86`) is not used in the supported scope.
- **SIM-4.4a Resume tokens are not tied to a wait reason.** A `Resume(p)` resumes `p` from **whatever**
  suspension point it is currently in. If two `Resume(p)` notes are pending, the second one wakes `p` from its
  *next* wait prematurely (e.g. in the middle of a hold or a resource demand); the woken code simply continues
  after its `passivate()`. The known trigger is two synchronous fork children finishing before the parent's
  first `Resume` is processed (ACT-9.5; `corpus/h26_fork_double_resume`). SimOxide reproduces this: every
  `Resume` continues the process from its current wait point, and resumes are neither deduplicated nor
  validated. The consequences of a premature wake-up are reproduced too:
  - the resource the process was waiting at still holds its job. When a PS resource receives a second demand
    of a process it already serves, the remaining demand is overwritten in place (Java `put`);
  - an FCFS resource (processing or linking) that receives a second demand of the same process aborts the
    run with a `NullPointerException` in `SimFCFSResource.scheduleNextEvent`; SimOxide reports
    `NullPointerException: FCFS process queued twice` at the same event
    ([reference bugs](../correctness/reference-bugs.md), REF-2);
  - when the resource later activates the process while it is running a later wait, the reference aborts with
    `IllegalStateException: Tried to schedule thread which was not suspended`; SimOxide reports
    `IllegalStateException: activate() of a running process` (REF-2).
- **SIM-4.5 Termination.** After `lifeCycle()` returns: listeners notified, state `TERMINATED`, control returns
  to the resumer (`SimulatedProcess.java:66-71`). `SimuComSimProcess.lifeCycle` (`:185-215`) wraps
  `internalLifeCycle()`; any `Exception` → model status `ERROR` + `simulationControl.stop()` (SIM-6.6); then
  `fireTerminated()` notifies the resources registered as terminated-observers (resource table,
  [schedulers](./scheduler.md)).
- **SIM-4.6 hold(d)** (`[SCC]SimuComSimProcess.java:168-170` → `[SCH]resources/active/AbstractActiveResource.java:30-50`
  → `SimDelayResource.java:25-57`):
  ```
  if !simulationControl.isRunning(): return               # no event, no suspension
  last := resourceTable.last(p)
  if last != D_p: (last?.dequeue(p)); D_p.enqueue(p); resourceTable.set(p, D_p)
  if p.id not in D_p.running: D_p.enqueue(p)               # enqueue = put + fireStateChange(size,0)
  insert Delay(p) at now + span(d); p.passivate()
  Delay(p) routine: if p.id not in D_p.running: return
                    remove; fireStateChange(size,0); fireDemandCompleted(p); p.activate()   # → Resume(p) at now
  ```
  So **a hold of d costs two events**: `Delay(p)` at `now+span(d)` and then `Resume(p)` appended at that time
  behind everything already queued there. `hold(0)` yields twice. `dequeue` of PS/FCFS resources is a no-op
  (`SimProcessorSharingResource.java:108-110`, `SimFCFSResource.java:84-86`).
- **SIM-4.7 Resource demand wait.** `consumeResource` → scheduler `process()` → `doProcessing` → `p.passivate()`;
  the scheduler's completion event later inserts its next completion event first and then calls
  `p.activate()` → `Resume(p)` at the completion time, appended FIFO (SIM-12 E10/E12)
  (`SimProcessorSharingResource.java:40-52,113-127`, `SimFCFSResource.java:24-36,89-97`). Details:
  [schedulers](./scheduler.md). A demand ≤ 0 after `calculateDemand` returns immediately without any event
  (ACT-3.4).

## SIM-5 What happens at time 0

- **SIM-5.1** `ExperimentRunner.run` (`[SC]ExperimentRunner.java:35-47`) registers the stop conditions (SIM-6)
  and calls `AbstractExperiment.start()` (`[ASE]AbstractExperiment.java:36-54`): `isRunning := true`,
  `DesmoJModel.init()` (no-op), `DesmoJExperiment.startSimulator()` → `[DJ]Experiment.start()` → clock 0 →
  `DesmoJModel.doInitialSchedules()` → `SimuComModel.init()` (`[SCC]model/SimuComModel.java:204-210`):
  1. `ISimulationListener.simulationStart()` for each listener of `SimuComConfig.getListeners()` in list order
     (no listener acts on start; sliding windows register only a stop hook, MEAS-6.3);
  2. `IWorkloadDriver.run()` for each driver in insertion order = **usage scenario order in the usage model**
     (`[SL]modelobserver/UsageModelSyncer.java:36-43` iterates `getUsageScenario_UsageModel()`).
- **SIM-5.2** `OpenWorkload.run()` inserts `Resume(generator)` at 0; `ClosedWorkload.run()` creates users
  1..N and inserts `Resume(user_i)` at 0 in that order (WL-2, WL-3). No random number is drawn and no StoEx is
  evaluated before the first event (SIM-8.3).
- **SIM-5.3** Events inserted during initialisation, i.e. *before* `Experiment.start()`, precede all later inserts
  at the same time. These are only the first `Periodic` events of measurement windows (at `span(len)`,
  e.g. 10 s; MEAS-6.3); resource `start()` of PS/FCFS/Delay schedules nothing
  (`[SCH]resources/active/SimProcessorSharingResource.java:104-106`).

## SIM-6 Stop conditions and simulation end

- **SIM-6.1 Registration order** (`[SL]launcher/jobs/RunInterpreterJob.java:30`, `[SC]ExperimentRunner.java:59-75`):
  `monitor.isCanceled()`, then max-sim-time if `simuTime > 0`, then max-measurements if `maxMeasurementsCount > 0`,
  then confidence (off by default). At least one must be enabled or the run throws. `AbstractExperiment` wraps
  them into one DESMO-J `ModelCondition` (`[ASE-DJ]DesmoJExperiment.java:49-57`); any true → stop. Order is
  irrelevant for the result.
- **SIM-6.2 Check timing.** Conditions are checked **after every processed event note**
  (`[DJ]Experiment.proceed`), never before the first one and never in the middle of an event. The event whose
  processing makes a condition true is executed completely (including all process execution it resumes);
  no further event is processed, not even other notes at the same timestamp. So when a resource event falls
  on the stop instant, its state tuples are recorded before the run stops.
- **SIM-6.3 Max simulation time.** `simuTime` is a `long` (whole seconds, `Long.valueOf` of the config string,
  `[SIMC]AbstractSimulationConfig.java:106`). Condition: `getCurrentSimulationTime() >= simuTime`
  (`[ASE]ISimulationControl.java:41-43`), i.e. `(double)now_ns/1e9 >= (double)simuTime`. No stop event is
  scheduled at `simuTime`: the **first event with time ≥ simuTime is executed**, then the run stops. Its time
  may be > simuTime; measurements taken during it are kept.
- **SIM-6.4 Max measurement count.** Condition: `SimuComModel.mainMeasurementsCount >= max`
  (`[SC]stopcondition/MaxMeasurementsStopCondition.java:31-33`). The counter is incremented **only** in the
  workload users (`increaseMainMeasurementsCount`, `[SCC]model/SimuComModel.java:301-303`):
  - open: once per user, in `finally` right after the usage scenario completes (also on `FailureException`)
    (`[SC]usage/OpenWorkloadUser.java:40-73`);
  - closed: once per iteration, in `finally` **after the think-time hold** (`[SC]usage/ClosedWorkloadUser.java:67-109,118-126`).
    So a closed user's run counts only when its think time has elapsed; response times recorded before that
    are kept even if the count is reached later (explains n=1002 for maxMeasurements=1000 on espresso).
  - Counted over all usage scenarios together. Not related to what the monitor repository measures.
- **SIM-6.5 End of run.** When the DESMO-J loop ends (condition true or empty list), `DesmoJExperiment.startSimulator`
  calls `AbstractExperiment.stop()` (`[ASE]AbstractExperiment.java:57-80`): `isRunning := false`,
  `Experiment.stop()`, `SimuComModel.finalise()` (`[SCC]model/SimuComModel.java:342-361`):
  1. `simulationStop()` listeners;
  2. `deactivateAllActiveResources()` — for each active resource in `ResourceRegistry` hash order:
     `fireStateEvent(0, i)` per instance, which makes monitored state probes record the *current* queue length
     (`[SCC]resources/AbstractScheduledResource.java:212-227`, `ScheduledResource.java:148-158`), scheduler
     `stop()`;
  3. `schedulingFactory.cleanActiveResources()` → `ResourceTableManager.waitForProcesses()` activates every
     non-finished process known to the resource table (`[SCH]resources/active/ResourceTableManager.java:29-48`).
     Because the experiment is stopped, each activation resumes the process **synchronously** (SIM-4.3) and it
     runs to its end with `isRunning()==false`: demands/holds/acquires return immediately
     (`AbstractActiveResource.java:32-35`, `[SCC]resources/SimSimpleFairPassiveResource.java:113-117,178-182`),
     loops guarded by `isRunning()` exit.

  Later, `SimulatedBasicComponentInstance.cleanUp` activates processes still queued at passive resources
  (`[SL]runtimestate/SimulatedBasicComponentInstance.java:110-114`).

  SimOxide performs the same steps, including the drain: in resource-table insertion order (the order of the
  deterministic reference, patch P3 in [reference patches](../reference-simulator/patches.md); stock
  SimuLizar drains in identity-hash order, ND-4), then the processes waiting at passive resources.
  - **Measurement relevance.** SimuLizar's interpreter probes check `isRunning()` and record nothing during
    the drain (`[SL]interpreter/listener/AbstractProbeFrameworkListener.java:307-328,410-413`). In-flight
    requests therefore produce **no** end measurement; their start measurements are simply unmatched.
    Resource-demand tuples (MEAS-4.3) are not gated: a demand evaluated during the drain records its tuple at
    the stop time (e.g. `corpus/x_pem_harddisk`). The drain also draws random numbers and increments
    `mainMeasurementsCount`. Which state/utilisation values are emitted at `finalise` is specified in
    MEAS-10 (they **are** recorded).
- **SIM-6.6 Errors.** A Java exception inside a process (StoEx failure, negative delay, unsupported action, ...)
  is caught in `SimuComSimProcess.lifeCycle`, sets status `ERROR` and calls `stop()`; the loop ends after the
  current event and `RunInterpreterJob.cleanup` throws (`[SL]launcher/jobs/RunInterpreterJob.java:37-43`).
  SimOxide reports an error with the same trigger at the same event; its trace equals the reference's partial
  trace up to the abort (`crates/simoxide-sim/tests/bugs.rs`, repros in `corpus-fuzz/`). The reference aborts
  that SimOxide reproduces are listed in [reference bugs](../correctness/reference-bugs.md); the cases where
  SimOxide stops a run the reference cannot finish (livelock, resource limits) are in
  [deviations](../correctness/deviations.md).

## SIM-7 Per-simulation initialisation order

Relevant because it fixes object creation order; it does **not** draw random numbers (SIM-8.3).

1. `SimuComConfig` from the attribute map (`[SCC]SimuComConfig.java:103-155`, `[SIMC]AbstractSimulationConfig.java:100-160`).
2. `SimuLizarRootJob` (`[SL]launcher/jobs/SimuLizarRootJob.java:35-55`): prepare blackboard, load models,
   resolve partitions, OCL validation, model completions, then the runtime job.
3. Runtime component construction creates `SimuComModel` (`[SLC]runtimestate/SimuComModelFactory.java`):
   DESMO-J model/experiment, RNG (`config.getRandomGenerator()`, first call creates it) installed into the
   static `ProbabilityFunctionFactoryImpl` singleton and `StoExCache` (`[SCC]model/SimuComModel.java:92-126`).
4. `SimuLizarRuntimeJob.execute` (`[SL]launcher/jobs/SimuLizarRuntimeJob.java:61-83`):
   `pcmPartitionManager.initialize()` (creates an empty RuntimeMeasurementModel if a monitor repository exists,
   `[SLC]utils/PCMPartitionManager.java:143-159`), reconfiguration loaders, `RuntimeStateEntityManager.initialize`
   (Set), `RuntimeStateEntityObserver.initialize` (Set), `IModelObserver.initialize` (Set: allocation lookup,
   resource environment, usage model, usage evolution, reconfigurator), interpreter listeners added to the
   `EventDispatcher` and initialised (Set; the probe-framework listener builds its calculators here).
   These `Set`s are built by the generated Dagger component with Guava `ImmutableSet.builder`, i.e. in
   **insertion order** (core elements first, see MEAS-9.1). Only extension-contributed sub-sets come from Dagger
   `SetFactory`/`SetBuilder` (`java.util.HashSet`, identity-hash order, ND-1); they do not affect the
   event list or EDP2 values.
   - `ResourceEnvironmentSyncer.initialize` (`[SL]modelobserver/ResourceEnvironmentSyncer.java:87-103`): for the
     allocation's target resource environment: containers in model order, each container's processing
     resources in model order (`createSimulatedActiveResource` → `ScheduledResource`, `activateResource()`,
     monitors), then linking resources in model order (`SimulatedLinkingResource`, FCFS, no monitors).
   - `UsageModelSyncer.initialize`: one workload driver per usage scenario in model order
     (`[SL]usagemodel/SimulatedUsageModels.java:58-78`); an `OpenWorkload` driver is a process (SIM-4.1),
     a `ClosedWorkload` driver creates its users only in `run()`.
5. `RunInterpreterJob` → SIM-5.1.

## SIM-8 Random numbers and StoEx evaluation: when, not how

- **SIM-8.1** There is **one** random stream per simulation: `SimuComConfig.getRandomGenerator()`
  (`SimuComDefaultRandomNumberGenerator`, MT19937, pre-filled by a producer thread in order;
  [random numbers](./random.md)). StoEx sampling (via the static `ProbabilityFunctionFactoryImpl`) and the
  branch decisions (`TransitionDeterminer`, ACT-6) draw from it. Only draw *order* matters, and it equals
  program order.
- **SIM-8.2** Every StoEx evaluation goes through `StackContext.evaluateStatic(spec, [type], [frame])`
  (`[SCV]StackContext.java:106-311`): parsed once per distinct string (`StoExCache`), evaluated freshly on every
  call (a distribution literal draws on every evaluation; [StoEx](./stoex.md)). Expected-type conversions: only
  widening `Byte/Short/Char → Integer/Long/Float/Double`, `Integer/Long/Float → Double`; any other mismatch
  throws (e.g. a loop count evaluating to a `Double` aborts the run) (`StackContext.java:213-311`).
- **SIM-8.3** No draws happen during SIM-7. The first draw happens inside the first event (typically the first
  user's first stochastic delay/demand or the open generator's first inter-arrival time).
- **SIM-8.4** The complete list of evaluation points with their order is in [workloads](./workloads.md) (WL-*)
  and [actions](./actions.md) (ACT-*); each rule states when its StoEx is evaluated relative to events and other
  draws. Variable-id serialisation and distribution sampling are specified in [StoEx](./stoex.md) and
  [random numbers](./random.md).

## SIM-9 Sources of nondeterminism

Stock SimuLizar 5.2.2 is not fully deterministic. The deterministic reference removes each source listed here
with a patch, a fixed seed or sequential runs ([reference patches](../reference-simulator/patches.md));
SimOxide follows the patched order.

| ID | Where | What | Effect |
|---|---|---|---|
| ND-1 | Dagger 2.31 `SetFactory.get()`/`SetBuilder.build()` (`DaggerCollections.newHashSetWithExpectedSize`) for **extension-contributed** multibinding sub-sets (core sets use `ImmutableSet.builder`, insertion order, MEAS-9.1) | `HashSet` of objects without `hashCode()` → identity-hash order | order among extension observers/listeners/entity managers and RDSEFF switch factories; no effect on events or EDP2 values in default 5.2.2 (none observed across JVMs) |
| ND-2 | `[SCH]resources/active/SimProcessorSharingResource.java:28,66-84` | `Hashtable<ISchedulableProcess,Double>`, key = process (identity hash); `scheduleNextEvent` takes the *first* minimum in iteration order | **which of several processes with equal remaining demand finishes first** → changes event order and all later RNG draw order. Very common with deterministic demands. The reference is patched to an insertion-ordered map (P2): the earliest inserted job wins a tie; SimOxide does the same ([schedulers](./scheduler.md)) |
| ND-3 | `[SCH]resources/active/SimProcessorSharingResource.java:91-94` | same map iterated in `toNow` | floating-point update per entry is independent of order → no effect |
| ND-4 | `[SCH]resources/active/ResourceTableManager.java:9,33` | `ConcurrentHashMap<process,…>` iterated in `waitForProcesses` | post-stop drain order (SIM-6.5): order and, through the draw order, values of the resource-demand tuples recorded at the stop time. The reference drains in insertion order (P3), as SimOxide does |
| ND-5 | `[SCC]SimuComDefaultRandomNumberGenerator.java:102-112` | seed from `new Random()` when `useFixedSeed=false` (default) | everything; the reference and SimOxide use fixed seeds ([random numbers](./random.md)) |
| ND-6 | `[SCC]model/SimuComModel.java:109-115` | RNG installed into static singletons (`ProbabilityFunctionFactoryImpl`, `StoExCache`) | parallel simulations in one JVM share/overwrite streams; the reference runs one simulation at a time |
| ND-7 | `[SCC]resources/SimulatedLinkingResource.java:115-121` | `Math.random()` | only with failure simulation (off) |
| ND-8 | `[SCH]resources/active/special/SimProcessorSharingResourceLinuxO1.java:54,290`, `...Windows.java:53` | `new Random()` | exact OS schedulers only (not supported) |
| ND-9 | `[SL]interpreter/RepositoryComponentSwitch.java:55`; `IdentifierImpl` ctor (`EcoreUtil.generateUUID()`) | random UUID of the static `SYSTEM_ASSEMBLY_CONTEXT` and of any `Identifier` created at runtime | ids only; the system AC is excluded from FQ component ids; no value effect (the reference fixes the id, P7) |
| ND-10 | `[ASE]AbstractSimProcessDelegator.java:15`, `[SCC]SimuComSimProcess.java:30`, `[SCC]resources/ScheduledResource.java:20`, `SimulatedLinkingResource.java:25` | static counters (process ids, session ids, resource ids) shared across runs in one JVM | names/ids in logs and request contexts only; the reference resets them before every run (P4–P6) |
| ND-11 | thread hand-off (`[ASE]processes/*`) | platform threads + semaphores | none (strict coroutine hand-off) |
| ND-12 | `[SCV]stackframe/SimulatedStackframe.java:37,112-128` | `HashMap<String,…>` iteration | deterministic (String hash), but **defines draw order** for INNER evaluation and summation order for network payload (ACT-5.6, ACT-11.3); SimOxide emulates `java.util.HashMap` iteration (ACT-5.7) |
| ND-13 | `[SCC]ResourceRegistry.java:153-174`, `[SCC]resources/AbstractSimulatedResourceContainer.java:32` | `HashMap<String,…>` of containers/resources | deterministic String-hash order of `finalise` state events (SIM-6.5) |
| ND-14 | `[PF]calculator/RegisterCalculatorFactoryDecorator` | `HashSet<Calculator>` | recorder flush order at cleanup only (MEAS-10.3) |
| ND-15 | `[SL]interpreter/listener/DeferredMeasurementInitialization.java:45,107-111` | `HashSet` of suppliers (identity hash) | PRM-only recorders |
| ND-16 | Eclipse extension registry (`ExtensionHelper`) | installation-dependent order of probe-framework decorators | creation order of sliding windows → order of same-time `Periodic` events and final flushes (MEAS-9.2); the reference registers plugins in sorted file-name order |
| ND-17 | EDP2 recorder flush (`EDP2RawRecorder`) | wall-clock `Date` | metadata only |

Not found: `System.currentTimeMillis`/`nanoTime` in values (only for logging run time), `ThreadLocalRandom`,
`identityHashCode` in the supported code path.

## SIM-10 master (6.0-SNAPSHOT) vs 5.2.2

A diff of the tag exports against the master branch (`diff -r`, ignoring imports):
- AbstractSimEngine, Scheduler, SimuCom framework bundles: **no Java differences**.
- SimuLizar core bundle: only `javax.inject` → `jakarta.inject`, Dagger `@Assisted("…")` qualifiers
  (`LoopingUsageEvolver*`, `StretchedUsageEvolver*`), a Java-21 comparator fix in `ModelCompletionsJob` and an
  accessibility helper in `GenericExtensionComponent`. No interpreter semantics differ.

## SIM-11 Implementation notes for `simoxide-sim` (non-normative)

- Event list (`crates/simoxide-sim/src/events.rs`): notes are ordered by `(time_ns, seq)`, `seq` taken at
  insert time (SIM-3.1/3.3). Process, delay, window and reconfiguration notes live in a 4-ary min-heap; each
  active resource has one *timer slot* for its single pending completion, kept in a small indexed heap.
  Rescheduling a resource replaces its slot, like the reference's `removeEvent()` + `schedule()`.
- A process is a state machine whose suspension points are exactly: initial (SIM-4.1), hold (SIM-4.6),
  active-resource demand (SIM-4.7), passive acquire (ACT-8), fork join (ACT-9). `Resume` continues from the
  current point (SIM-4.4a).
- Every double operation listed in the ACT/WL rules keeps the Java expression order.

## SIM-12 Engine operations per interpreter step (integer-nanosecond contract)

Notation: `now` = current clock in ns; `span(d) = (long)(d*1e9)` (SIM-2.2); `ins(t, E)` = insert event `E`
at absolute time `t` with the next global `seq` (FIFO, SIM-3.1); `cancel(E)` = remove a pending note;
**wait** = the process suspends and control returns to the engine. Everything not listed creates no event.
Consistent with the event ordering contract in [schedulers](./scheduler.md).

| # | Step | Engine operations, in this order |
|---|---|---|
| E1 | init (before `Experiment.start`) | per measurement window / time-driven aggregation, in creation order: `ins(span(len), Periodic(w))` (MEAS-6.3) |
| E2 | `SimuComModel.init()` at t=0 | per usage scenario in model order: open → `ins(0, Resume(gen))`; closed → for i=1..N `ins(0, Resume(user_i))` |
| E3 | open generator iteration | create user `u`; `ins(now, Resume(u))`; evaluate inter-arrival `ia`; hold(ia) = `ins(now+span(ia), Delay(gen))`; wait |
| E4 | open user end | count += 1 (SIM-6.4); process terminates, nothing inserted |
| E5 | closed user end of scenario | evaluate think time `tt`; `ins(now+span(tt), Delay(u))`; wait. At its `Resume`: count += 1, next iteration starts in the same event |
| E6 | usage `Delay` | evaluate; `ins(now+span(d), Delay(p))`; wait |
| E7 | `Delay(p)` event | (state/demand-completed callbacks) `ins(now, Resume(p))` |
| E8 | resource demand, `d <= 0` after `calculateDemand` (ACT-3.4, ACT-11.4) | nothing, no wait |
| E9 | PS demand `d > 0` | `toNow`; add job (`max(d, 1e-9)`); state tuples; `cancel(PSDone)`; `ins(now+span(t), PSDone(shortest))` with `t = rem*f`, `t < 1e-9 → 0`; wait |
| E10 | `PSDone(q)` event | `toNow`; remove q; state tuples; demand completed; if jobs left `ins(now+span(t'), PSDone(next))`; **then** `ins(now, Resume(q))` |
| E11 | FCFS demand `d > 0` (also every network transmission) | `toNow`; enqueue; state tuple; `cancel(FCFSDone)`; `ins(now+span(rem(head)), FCFSDone(head))` (re-inserted on **every** arrival); wait |
| E12 | `FCFSDone(h)` event | `toNow`; dequeue h; state tuple; demand completed; if queue non-empty `ins(now+span(rem(head)), FCFSDone(head))`; **then** `ins(now, Resume(h))` |
| E13 | passive acquire | granted: nothing; blocked: enqueue, wait (no insert) |
| E14 | passive release | per granted waiter in FIFO order: `ins(now, Resume(w))`; releaser continues |
| E15 | fork | create children (async, then sync); for each in that order `ins(now, Resume(child))`; if any sync child alive: wait (no insert); re-check on every resume |
| E16 | sync fork child end | if parent not terminated and running: `ins(now, Resume(parent))` (may duplicate, SIM-4.4a) |
| E17 | `Periodic(w)` event | window flush (tuples); `ins(now+span(inc), Periodic(w))` |
| E18 | after **every** event | stop check (SIM-6.2) |

`Resume(p)` executes `p` synchronously until its next wait or its end; all inserts made meanwhile use the same
`now`. Branch, loop, start/stop actions, entry-level/external calls, SetVariable, parameter evaluation,
infrastructure calls and component-instance creation never insert events by themselves. With monitors that
have `triggersSelfAdaptations = true`, a runtime-measurement write can additionally insert `Resume(reconf)`
at `now` (MEAS-7.2).

## SIM-13 Verification

All rules are derived from the 5.2.2 tag sources (product jars spot-checked identical). DESMO-J's FIFO tie
order and ns truncation (SIM-2.2, SIM-3.1/3.2) are confirmed with a probe program against
`desmoj-2.3.3-core-bin.jar`; the JScience window arithmetic (MEAS-6.5) was executed on the JVM. SimOxide's
traces, random tapes and measurements are byte-identical to the deterministic reference on the corpus and on
generated models ([testing](../correctness/testing.md)). Rules that the corpus exercises specifically:

| Rule | Corpus model |
|---|---|
| SIM-4.4a / ACT-9.5 double resume | `corpus/h26_fork_double_resume`; the aborts it can cause: `corpus-fuzz/k_bug2_*` |
| ACT-5.7 Java `HashMap` order (INNER draws) | `corpus/h27_collection_inner_multi` |
| ACT-11.3 network flags (recorded in `run.json`, [formats](../guide/formats.md) §5) | `corpus/h19_linking_resource`, `corpus/h19b_linking_no_throughput`, `corpus/h30_middleware_stream` |
| MEAS-7.2 reconfiguration process (`triggersSelfAdaptations`) | `corpus/h28_triggers_default`, `corpus/h29_triggers_late` |
| ND-2 PS tie order | `corpus/h02_ps_ties` |

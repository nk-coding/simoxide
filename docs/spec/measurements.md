# Measurements (SimuLizar 5.2.2)

Rules `MEAS-*`. Engine/time rules (`SIM-*`): [simulation core](./simulation.md). SimOxide records the
measurements described here; the output format is in [formats](../guide/formats.md) §4. The JScience
arithmetic (MEAS-6.5) was executed on the JVM with the 5.2.2 jars.

Additional abbreviations (besides those in [simulation core](./simulation.md)):

| Tag | Path prefix / artefact |
|---|---|
| `[PF]` | `Palladio-QuAL-ProbeFramework/bundles/org.palladiosimulator.probeframework/src/org/palladiosimulator/probeframework/` |
| `[SLU]` | `Palladio-Analyzer-SimuLizar/bundles/org.palladiosimulator.simulizar.utilization/src/org/palladiosimulator/simulizar/utilization/` |
| `[SLW]` | `Palladio-Analyzer-SimuLizar/bundles/org.palladiosimulator.simulizar.slidingwindow/src/org/palladiosimulator/simulizar/slidingwindow/` |
| `[SLA]` | `Palladio-Analyzer-SimuLizar/bundles/org.palladiosimulator.simulizar.aggregation/src/org/palladiosimulizar/aggregation/` |
| `[SLF]` | `Palladio-Analyzer-SimuLizar/bundles/org.palladiosimulator.simulizar.monitorrepository.feedthrough/src/org/palladiosimulator/simulizar/monitorrepository/feedthrough/` |
| `[SLR]` | `Palladio-Analyzer-SimuLizar/bundles/org.palladiosimulator.simulizar.reliability/src/org/palladiosimulator/simulizar/reliability/` |
| `[EA]` | jar `org.palladiosimulator.experimentanalysis_5.2.2.jar`, package `org.palladiosimulator.experimentanalysis` (decompiled) |
| `[JS]` | jar `org.jscience_4.3.1.jar`, `org.jscience.physics.amount.Amount` (decompiled) |
| `[RC]` | jar `org.palladiosimulator.simulizar_5.2.2.jar`, generated `di/component/core/DaggerSimuLizarRuntimeComponent` (decompiled) |

Installed decorator bundles in the product: `simulizar.utilization`, `.slidingwindow`, `.aggregation`,
`.monitorrepository.feedthrough` (not `.monitorrepository.map`). No bundle contributes to
`org.palladiosimulator.probeframework.calculator.factories`.

## MEAS-1 Pipeline

- **MEAS-1.1** Calculator factory = `RegisterCalculatorFactoryDecorator(RecorderAttachingCalculatorFactoryDecorator(ExtensibleCalculatorFactoryDelegatingFactory))`
  (SimuLizar `EclipseQUALModule.java:23-28`; `[PF]ProbeFrameworkContext.java:47-50` adds no second decorator).
- **MEAS-1.2** Only two calculator kinds occur (`[PF]calculator/ExtensibleCalculatorFactoryDelegatingFactory.java:58-61`,
  `[PF]calculator/CalculatorProbeSetBasedInferingCalculatorFactory.java:35-50`): single probe → `IdentityCalculator`
  (tuple = the probe's measurement list verbatim); start+stop probes → `TimeSpanCalculator`.
- **MEAS-1.3** Every built calculator gets its own recorder / EDP2 series (`[SCC]calculator/RecorderAttachingCalculatorFactoryDecorator.java:39-57`).
  A calculator whose probes never fire yields an empty series. Registering a calculator equal to an existing one
  (same class, metric id, `EcoreUtil.equals` measuring point) throws → **duplicate monitors abort the run**
  (`[PF]calculator/RegisterCalculatorFactoryDecorator.java:114-142`).
- **MEAS-1.4** Observer lists are insertion-ordered `CopyOnWriteArrayList`; dispatch is synchronous
  (`Palladio-Core-Commons/.../designpatterns/AbstractObservable.java:40,96-98`). Calculator observers: EDP2 recorder
  first, then decorator recorders in attach order.
- **MEAS-1.5 Start/stop matching** (`[PF]calculator/Calculator.java:174-210`): a measurement from `probes[0]`
  with a `RequestContext` already in `memory` → `IllegalStateException` (abort); otherwise `memory[ctx] = [m]`.
  Every process has its own context, so this happens when one process re-enters a monitored external call or
  assembly operation by recursion ("First measurement to the same context arrived while previous series of the
  same context did not complete", right after the `begin` line; REF-9 in
  [reference bugs](../correctness/reference-bugs.md), `corpus-fuzz/l_ref_recursion_*`).
  Any other measurement: no entry → logged and dropped; else append; if from the last probe → compute, notify,
  remove. Single-probe calculators emit immediately. A start without stop stays in memory forever (nothing
  emitted, nothing counted).
- **MEAS-1.6** `TimeSpanCalculator` tuple = `(t_stop, t_stop - t_start)` seconds, plain f64 subtraction
  (`[PF]calculator/internal/TimeSpanCalculator.java:57-75`). Time probe value = `getCurrentSimulationTime()`
  (`[SCC]probes/TakeCurrentSimulationTimeProbe.java:36-39`) = `ns/1e9` (SIM-2.4).
- **MEAS-1.7** `RequestContext` equality = (id string, parent) recursively (`[PF]measurement/RequestContext.java:139-178`).
  Interpreter probes use `thread.getRequestContext()`: a process's context is `RequestContext(str(rawId), parent)`
  (`[SCC]SimuComSimProcess.java:75-76`); closed users append `"." + runCount` (WL-3.3), so each closed iteration
  has a fresh context; fork children use the parent's context as parent (ACT-9.2).
- **MEAS-1.8 Tuple layouts** (5.2.2 `commonMetrics.metricspec`; `TupleMeasurement` enforces order,
  `Palladio-QuAL-MeasurementFramework/.../TupleMeasurement.java:49-60`): response/waiting/holding time,
  state of active/passive resource, utilisation = `[PointInTime, value]`; **resource demand = `[ResourceDemand, PointInTime]`**;
  reconfiguration time = `[time, PointInTime]`; number of containers = `[count, PointInTime]`; execution result =
  `[PointInTime, type]`.
- **MEAS-1.9** Metric matching is by exact metric-description **id** (`MetricDescriptionUtility.metricDescriptionIdsEqual`);
  monitors must reference the base metrics named below (e.g. "Response Time", not "Response Time Tuple").

## MEAS-2 Which measurements exist

- **MEAS-2.1 Monitor repository.** If the PCM partition has none, `DefaultMonitorRepositoryCompletionContributor`
  generates one as the last model completion (`[SL]launcher/jobs/extensions/DefaultMonitorRepositoryCompletionContributor.java:29-46`).
  Measuring points come from `resourceSet.allContents` (resource order, then containment DFS), monitors in MP
  order, all `FeedThrough`, `triggersSelfAdaptations = false`, activated
  (`DefaultMeasuringPointRepositoryFactory.xtend:22-80`, `DefaultMonitorRepositoryFactory.xtend:97-175`):
  - every `UsageScenario`: Response Time;
  - every System × OperationProvidedRole × OperationSignature: Response Time (SystemOperationMeasuringPoint);
  - every `ExternalCallAction` of every loaded repository: Response Time;
  - every `ProcessingResourceSpecification` (replica 0): State of Active Resource + Resource Demand (multi-core
    → also the raw overall utilisation of MEAS-4.4);
  - every AssemblyContext of a BasicComponent × PassiveResource: Waiting Time, Holding Time, State of Passive Resource;
  - EntryLevelSystemCall MPs are created but get **no** monitor; no sliding-window utilisation.
  A user-supplied repository is used as is (nothing added).
- **MEAS-2.2 Catalogue** (details in the following sections):

  | Measurement | Registered by | Rule |
  |---|---|---|
  | Response time (scenario, ELSC, system op, assembly op, external call) | probe-framework interpreter listener | MEAS-3 |
  | State of active resource, resource demand, overall utilisation | `ResourceEnvironmentSyncer` | MEAS-4 |
  | Passive resource state / waiting / holding | `SimulatedBasicComponentInstance` (lazy) | MEAS-5 |
  | Utilisation (sliding window) | utilization decorator | MEAS-6 |
  | PRM-only aggregations (FeedThrough w/ trigger, fixed/variable/time-driven aggregation) | decorators | MEAS-7 |
  | Execution result type | reliability extension | MEAS-7.4 |
  | Linking-resource anything, throughput, arrival rate | **never produced** | MEAS-8 |

- **MEAS-2.3** SimuCom's own usage-scenario calculator (`[SC]usage/AbstractWorkloadUserFactory.java:51-67`) is
  never attached in SimuLizar (only `[SC]AbstractMain.java:270` calls it). The SimuCom start/stop probes in
  the users (WL-2.3, WL-3.2) fire into no observers: no output.

## MEAS-3 Response times

`[SL]interpreter/listener/AbstractProbeFrameworkListener.java:266-293,307-402`.
- **MEAS-3.1** For each **active** monitor (repository order) × measurement specification (list order) whose
  metric id is Response Time (processing type ignored): two fresh time probes stored in
  `currentTimeProbes[key(MP)]` (later specs with the same key overwrite the probes) and a `TimeSpanCalculator`
  at the monitor's MP. `key(MP)` (`[SL]utils/MonitorRepositoryUtil.java:81-118`): AssemblyOperationMP →
  `assemblyId::roleId::sigId`; SystemOperationMP → `systemId::roleId::sigId`; otherwise the id of the
  monitored element. Response-time monitors on other MP types give empty series. The probes are found by these
  id strings, so a measuring point may reference an equal-id element of another loaded model (the
  SimpleHeuristics example's monitor points to `quickbooking.usagemodel`,
  `crates/simoxide-sim/tests/models/u_heuristics_own_monitors`).
- **MEAS-3.2 Trigger points** (start = BEGIN, stop = END; each only if `isRunning()`, lines 307-328, 410-413):
  - UsageScenario: WL-4.1 (BEGIN first thing, END after the whole behaviour);
  - EntryLevelSystemCall: WL-4.2 around `doSwitch(action)`;
  - SystemOperation: WL-4.6 steps 2 and 7 — strictly nested inside the ELSC interval with no time passing
    in between, so both tuples are identical when both are monitored;
  - AssemblyOperation: ACT-2.1 (the system entry uses the random-id system AC → never matches);
  - ExternalCallAction: ACT-1.2 around ACT-4; includes both network transmissions (ACT-11) and the callee.
- **MEAS-3.3** Tuple `(t_stop, t_stop - t_start)` emitted synchronously at the stop.

## MEAS-4 Active resources: state, demand, overall utilisation

`[SL]modelobserver/ResourceEnvironmentSyncer.java:248-273,359-430`, `[SCC]resources/CalculatorHelper.java`.
Only processing resources (not linking resources); monitors are attached right after the resource is created
and activated (SIM-7 step 4). Uses the first MonitorRepository; specs of **active** monitors whose
`ActiveResourceMeasuringPoint` refers to the resource spec id (replica ignored for matching,
`[SL]utils/MonitorRepositoryUtil.java:224-242,316-338`).
- **MEAS-4.1 State of Active Resource**: processing type must be FeedThrough, else `IllegalArgumentException`.
  If `MP.replicaID == 0 && numberOfInstances > 1`, first create the overall-utilisation calculator (MEAS-4.4).
  Then the state calculator for instance `0` (DELAY/FCFS) or `MP.replicaID` (others).
- **MEAS-4.2 State calculator** (`CalculatorHelper.java:236-258`): probe list `[time, TakeScheduledResourceStateProbe(res, instance)]`,
  `IdentityCalculator`, state listener on `instance`, then **immediately one measurement** → initial tuple
  `(0.0, 0)` at init. The value is always `getQueueLength(instance)` **read when measuring**, not the value
  carried by the event (`[SCC]probes/TakeScheduledResourceStateProbe.java:42-45`,
  `[SCC]resources/AbstractScheduledResource.java:298-302,350-352`).
- **MEAS-4.3 Resource Demand** (`CalculatorHelper.java:205-212`): tuple `(demand, t)` from `fireDemand(d)` in
  ACT-3.4 — after `calculateDemand` and modifiers, **before** `process()` (i.e. before the state change), only if
  `d > 0`. The PS JIFFY clamp is not reflected. Processing type ignored.
- **MEAS-4.4 Overall utilisation** (`CalculatorHelper.java:280-298`, `ResourceEnvironmentSyncer.java:426-430`): MP =
  new ActiveResourceMP(resource, replicaID = numberOfInstances); value = (#cores with queue length > 0) /
  numberOfInstances as f64 (`[SCC]probes/TakeScheduledResourceUtilization.java:37-48`); no initial measurement;
  measured on every `fireOverallUtilization`.
- **MEAS-4.5** Waiting/holding time on active resources: ignored (FIXME in source). Other metrics: nothing here.
- **MEAS-4.6 When state probes fire.** `AbstractActiveResource.fireStateChange(state, core)` →
  `ScheduledResource.update`: state listeners of that core (state tuple), **then** `fireOverallUtilization`
  (overall tuple) (`[SCH]resources/active/AbstractActiveResource.java:74-78`, `[SCC]resources/ScheduledResource.java:216-220`).
  - PS (`[SCH]resources/active/SimProcessorSharingResource.java:113-172`): on arrival (after `put`) and on
    finish (after `remove`), `reportCoreUsage`: `n = #running`, `cap = #cores`; if `n < cap`: core i gets 1 for
    i < n else 0; else `floor(n/cap)` each and the first `n mod cap` cores +1; cores 0..cap-1 in order, firing
    **only for cores whose count changed**. `getQueueLength(core) = numberProcessesOnCore[core]`. The overall
    probe reads the partially updated array, so several overall tuples with intermediate values can occur at
    one instant (e.g. 0 → 0.5 → 1.0). On finish the order is: remove, state tuples, `fireDemandCompleted`,
    reschedule, activate.
  - FCFS (`SimFCFSResource.java:23-36,89-97`): fire after enqueue (always) and after removal on completion;
    queue length = `processQ.size()` (includes the job in service).
  - Delay (`SimDelayResource.java:38-73`): fire on enqueue (only if not yet present) and on dequeue; length =
    `running_processes.size()`. (Per-process think-time delay resources are never monitored.)

## MEAS-5 Passive resources

`[SL]runtimestate/SimulatedBasicComponentInstance.java:34-71`, `[SCC]resources/CalculatorHelper.java:77-173,313-391`.
- **MEAS-5.1** Calculators are created **lazily** when the component instance is created (first call, ACT-8.1),
  per passive resource, per MonitorRepository, checking in order STATE_OF_PASSIVE_RESOURCE, WAITING_TIME,
  HOLDING_TIME with `MonitorRepositoryUtil.isMonitored` = first spec over **all** monitors (activation and
  processing type **not** checked) whose MP conforms (AssemblyPassiveResourceMP: passive-resource id only,
  assembly ignored; MP types not handled conform to everything) (`MonitorRepositoryUtil.java:67-78,244-261,358-373`).
- **MEAS-5.2** The series' MP is a generated `ResourceURIMeasuringPoint` (URI of the PassiveResource,
  string = AssemblyPassiveResourceMP(instance's last AC, resource)), **not** the monitor's MP (`CalculatorHelper.java:380-391`).
- **MEAS-5.3 State**: tuple `(t, available)`; initial tuple at creation time (value = capacity); then after every
  grant (`available -= 1`) and every release (`available += 1`).
- **MEAS-5.4 Waiting time**: start at `fireRequest` (ctx = `RequestContext(process.getId())`, `getId = name + "_" + rawId`),
  stop at `fireAcquire` → `(t_acq, t_acq - t_req)`; immediate grants give 0.0.
- **MEAS-5.5 Holding time**: start at acquire, stop at release; ctx = `RequestContext("1")` if capacity == 1, else
  `RequestContext(process.getId())`. A process acquiring the same resource twice (capacity > 1) before releasing
  → `IllegalStateException` (MEAS-1.5, abort).
- **MEAS-5.6 Order** (sensor order = registration order [state, waiting, holding]; ACT-8.2/8.3): acquire granted
  immediately → state tuple, waiting tuple, holding start; release → state tuple, holding tuple, then for each
  granted waiter: state tuple, waiting tuple, holding start, `activate`.

## MEAS-6 Utilisation (sliding windows)

`[SLU]probeframework/UtilizationProbeFrameworkListenerDecorator.java:92-232`, `[SLW]impl/SimulizarSlidingWindow.java:120-225`,
`[EA]` classes as cited.
- **MEAS-6.1** Specs: first all active specs with metric UTILIZATION_OF_ACTIVE_RESOURCE_TUPLE, then those with
  UTILIZATION_OF_ACTIVE_RESOURCE. Processing type must be TimeDriven (or TimeDrivenAggregation), else
  `IllegalStateException`. It needs a registered **FeedThrough state calculator on the same MP** (string
  representation equal); missing (or a LinkingResourceMP) → `IllegalStateException` at init.
  Defaults `windowLength = windowIncrement = 10.0 s` (`TimeDrivenImpl` EDEFAULTs).
- **MEAS-6.2** A `SlidingWindowRecorder(SimulizarSlidingWindow(len, inc, accepted = STATE tuple,
  KeepLastElementPriorToLowerBoundStrategy), SlidingWindowUtilizationAggregator(→ recorder metric
  UTILIZATION_OF_ACTIVE_RESOURCE_TUPLE at the state MP))` is attached to the state calculator. If
  `MP.replica == 0`, the resource has > 1 replica and an overall calculator exists: a second window on the overall
  calculator (accepted = utilisation tuple), output at the overall MP (that MP then has two series with the same
  metric: raw and windowed). `triggersSelfAdaptations` (EMF default **true**) additionally adds a PRM recorder.
- **MEAS-6.3 Window timing (events!)**: each window creates at init (t = 0) a `PeriodicallyTriggeredSimulationEntity`
  (`[SL]simulationevents/PeriodicallyTriggeredSimulationEntity.java:15-41`, `[ASE]SimpleEventBasedSimEntity.java:34-57`)
  → event at `span(len)`; its routine runs `onWindowFullEvent()` and **then** re-schedules the same event at
  `now + span(inc)`. It also registers an `ISimulationListener` whose `simulationStop` emits a final partial window
  if its effective length ≠ 0 (runs in `finalise` **before** resource deactivation, SIM-6.5). These events count
  for SIM-6.2/6.3: a window event can be the first event ≥ `simuTime` and thus determine where the run stops.
- **MEAS-6.4 Window state** (`[EA]SlidingWindow.java:92-95,128-135,171-180`): `lowerBound = 0.0`; `addMeasurement`:
  if `lowerBound > t(m)` (Double.compare) clear data; append. Window full: notify `(data, lowerBound,
  effectiveLength)` then `lowerBound += inc` (f64), then the move-on strategy (`[EA]KeepLastElementPriorToLowerBoundStrategy.java:17-28`):
  if the first element's t < new lower bound, pop all leading elements with t < lower bound and push the last
  popped one back at the front. `upperBound = min(lowerBound + len, now)`, `effectiveLength = upperBound - lowerBound`.
- **MEAS-6.5 Aggregation with JScience `Amount` (reproduced literally)** (`[EA]windowaggregators/SlidingWindowUtilizationAggregator.java:52-100`):
  ```
  L = A(lowerBound); W = A(effectiveLength); R = W.plus(L); busy = exact(0)
  cur = first; curT = A(t(cur)); state = min(value(cur), 1.0)
  loop: if curT.isLessThan(L): curT = L
        nextT = has next ? A(t(next)) : R
        busy = busy.plus(nextT.minus(curT).times(state)); advance
  tuple = (R.estimated, busy.divide(W).estimated)      # empty window → 0.0
  ```
  `value` = queue length (state tuple) or fraction (overall tuple). `Amount` (`[JS]`): fields `exact: Option<i64>`,
  `min`, `max: f64`; `DEC = 1 - 2^-53`, `INC = 1 + 2^-53` (rounds to exactly 1.0);
  `adj(lo,hi) = (lo<0 ? lo*INC : lo*DEC, hi<0 ? hi*DEC : hi*INC)`; `A(v)` inexact with `min = v<0 ? v*INC : v*DEC`,
  `max = v<0 ? v*DEC : v*INC`; `exact(0)`: exact 0; `estimated = exact ? exact : (min+max)*0.5`; comparisons by
  `Double.compare` of estimates; `plus` = exact sum if both exact (no overflow) else `adj(a.min+b.min, a.max+b.max)`;
  `minus` = `adj(a.min-b.max, a.max-b.min)`; `times(f64 f)` = `f>0 ? (min*f, max*f) : (max*f, min*f)` then `adj`;
  `divide(W) = times(W.inverse())`, `inverse` = `(-inf,+inf)` if `min <= 0 <= max` else `adj(1/max, 1/min)`;
  `times(Amount)` = 9-case interval product then `adj` (`[JS]Amount` lines 70-72, 93-101, 149-151, 206-241,
  260-267, 270-318, 320-338, 369-376, 454-456, 490-495, 511-512, 640-656).
  Executed on the JVM: window [0,10] busy from 2 s → `(9.999999999999998, 0.7999999999999996)`; right bound of
  [10,20] = 19.999999999999996. Every element, including zero-length segments from several state tuples at one
  instant, enters the arithmetic, so the exact tuple sequence of MEAS-4.6 matters for the last bits.
- **MEAS-6.6** Window listeners are attached after the initial state tuple (MEAS-9), so the t=0 tuple is in the
  state series but in no window. The same-instant order of a window event and a state tuple at the window end
  does not change the value.

## MEAS-7 PRM-only outputs and other extras

- **MEAS-7.1** FeedThrough (only with `triggersSelfAdaptations`), Fixed/VariableSizeAggregation (only with trigger)
  and TimeDrivenAggregation write only the RuntimeMeasurementModel (PRM), never EDP2
  (`[SLF]FeedThroughDecorator.java:34-62`, `[SLA]probeframework/AggregatorsProbeFrameworkListenerDecorator.java:52-90`,
  `[SLW]probeframework/SlidingWindowProbeFrameWorkListenerDecorator.java:51-136`). They require an existing
  calculator (else `IllegalStateException`) and create none. **TimeDrivenAggregation windows are always created
  and schedule periodic events (MEAS-6.3)** even without recorders; SimOxide creates them too, because they can
  decide where a time-limited run stops (SIM-6.3).
- **MEAS-7.2 Reconfiguration side effect** (checked against the reference: `corpus/h28_triggers_default`,
  `corpus/h29_triggers_late`, `crates/simoxide-sim/tests/models/t_*`). A PRM model exists whenever a monitor
  repository exists (`[SLC]utils/PCMPartitionManager.java:143-159`). The `Reconfigurator` observes it
  (`[SL]reconfiguration/Reconfigurator.java:128-145`): every PRM change with a measuring point (an
  added `RuntimeMeasurement` or a `setMeasuringValue`) at `now > lastReconfigurationTime` (initially
  0) while the process is not scheduled creates the `ReconfigurationProcess` once
  (`SimuComSimProcess` constructor → a process id and a `spawn` line, `kind`
  `ReconfigurationProcess`, `name` `Reconfiguration Process`, `parent` = running process or 0) and
  `scheduleAt(0)`s it (`ReconfigurationProcess.java:161-205`). Without reconfiguration rules it only
  fires begin/end-reconfiguration events and passivates: no draws, no EDP2 values, no trace lines;
  its `Resume` notes never change the order of other notes or a stop decision (they are inserted at
  `now` during an event that already passed the stop check). **Observable effect: one `spawn` line,
  shifting all later process ids by one; measurements and tape are unchanged** (checked on the corpus
  models with every spec set to `true`) — **unless a `Reconfiguration Time` monitor exists**
  (MEAS-7.3): then every run of the process records a tuple, and SimOxide emulates the runs: a PRM
  change at `now > lastReconfigurationTime` while no `Resume` is pending inserts `Resume(reconf)` at
  `now` (FIFO position = the moment of the write, e.g. before the `Resume` of a job whose completion
  wrote a state tuple) and sets `lastReconfigurationTime = now` after scheduling. After the stop
  (`finalise`, drain) `scheduleAt` resumes the process synchronously and `lastReconfigurationTime` is
  set only when that run returns: if the run's own reconfiguration-time tuple writes the PRM (a
  triggering reconfiguration-time spec), the process schedules itself while running and the run
  aborts (`IllegalStateException: Tried to schedule thread which was not suspended [Reconfiguration
  Process_n]`, `pend` of the process; REF-11 in [reference bugs](../correctness/reference-bugs.md)). A pending
  `Resume` at the stop never runs. PRM writers (only for specs of **active** monitors with
  `triggersSelfAdaptations = true`, all created by probe-framework decorators):
  - `FeedThroughRecorder` on the calculator of the spec's MP and base metric (matched by MP string
    representation and metric subsumption, `DeferredMeasurementInitialization`): writes every tuple;
    the spec metric must be a `NumericalBaseMetricDescription`, else `IllegalStateException` at init;
  - `FixedSize`/`VariableSizeAggregation` (`[SLA]aggregators/*`): every `frequency`-th tuple, if the
    buffer holds `numberOfMeasurements` tuples / spans `retrospectionLength`
    (`!last.minus(R).isLessThan(first)`, JScience `Amount`); invalid parameters throw when the
    aggregator is created;
  - `SlidingWindowRuntimeMeasurementsRecorder` of `TimeDrivenAggregation` and of the utilisation
    windows (per replica and overall): every window output, **also for empty windows** and for the
    final partial window at the stop (MEAS-6.3).

  Each recorder's constructor adds a `RuntimeMeasurement` (a PRM change): at t = 0 for calculators
  created at init; for the lazily created passive-resource calculators (MEAS-5.1) at their
  registration, i.e. **before** the initial state tuple for the state calculator and after it for
  waiting/holding. Recorders on a calculator observe after the EDP2 recorder, so the `spawn` line
  follows the `meas` line of the triggering tuple. The initial state tuples of active resources
  (t = 0) precede all decorators and are not seen by aggregators. PRM writes in
  `SimuComModel.finalise` (window flush, final state tuples) still create the process (after the
  `stop` line).
- **MEAS-7.3** Reconfiguration time and number of resource containers (checked against the reference:
  `crates/simoxide-sim/tests/models/t_reconf_*` and `u_*`).
  - Reconfiguration time (`[SL]interpreter/listener/ProbeFrameworkListener.java:31-45`,
    `reconfiguration/probes/TakeReconfigurationDurationProbe.java`): a calculator per spec (active
    monitors) with metric `Reconfiguration Time` (`_VYg6MujFEeSB6OBq2SKZxQ`); tuple `(t, duration)` per
    **successful** reconfiguration. Without rules every run succeeds: the QVTo engine's
    `executeTransformations` returns `true` for an empty rule list (`QVTOExecutor.java:34-41`), so each run of
    the reconfiguration process (MEAS-7.2) records `(t, 0.0)` once (REF-12). Series key of a
    `ReconfigurationMeasuringPoint` (a `ResourceURIMeasuringPoint`):
    `ResourceURIMeasuringPoint[<fragment of resourceURI or null>|<measuringPoint>]`. Without `resourceURI` SimuLizar's measuring-point checks throw an
    NPE at init.
  - Number of resource containers (`[SL]reconfiguration/NumberOfResourceContainerTrackingListener.java:61-126`):
    first spec with metric `Number of Resource Containers` (`_e7x3gq-eEeSgL6DrxYuwZg`) of an active monitor whose
    MP conforms to the resource environment (a `ResourceEnvironmentMeasuringPoint`); initial tuple
    `(0, number of top-level containers)` when the `Reconfigurator` initialises, i.e. after the resource
    environment's initial state tuples and the open-workload generators' creation (UsageModelSyncer), before
    the probe-framework decorators; later tuples only when a reconfiguration changes the count (never without
    rules). Key `ResourceEnvironmentMeasuringPoint[<environment id or fragment, "/">]`.
- **MEAS-7.4** Execution result type (`[SLR]interpreter/listener/ReliabilityProbeFrameworkAdapter.java:49-86`, extension
  always installed): for specs with metric EXECUTION_RESULT_TYPE_TUPLE (all monitors, activation not checked):
  one tuple `(t, SUCCESS|FAILURE)` per usage-scenario run, right after the scenario END (WL-4.1
  `emitInterpretationFinished`), **not** gated by `isRunning()`. Only with an explicit monitor of the metric
  `Execution Result Type over Time` (`_-TkoURX7Eey-ibmvVnJ8rg`), never in a default configuration. The value is
  a textual identifier; the deterministic reference's recorder only takes numbers and aborts
  (`ClassCastException: IdentifierImpl cannot be cast to Number`), so there is no reference output to compare
  with. SimOxide warns and produces nothing.

## MEAS-8 Not produced

- **MEAS-8.1** Linking resources: no state, demand, utilisation or throughput series (ACT-11.5). A utilisation
  monitor on a LinkingResourceMP crashes initialisation (MEAS-6.1).
- **MEAS-8.2** Throughput / arrival rate: no calculator, probe or decorator in 5.2.2 (metrics exist in the metric
  spec only). SimOxide produces none either; throughput can be derived from response-time tuples.

## MEAS-9 Initialisation order of measurement objects

(`[SL]launcher/jobs/SimuLizarRuntimeJob.java:61-83`; SIM-7.)
- **MEAS-9.1** The runtime component builds its multibound sets with Guava `ImmutableSet.builder` in
  **insertion order** (`[RC]` `setOfIModelObserver`, `setOfIInterpreterListener`, `setOfRuntimeStateEntityManager`):
  - model observers: `[AllocationLookupSyncer, ResourceEnvironmentSyncer, UsageModelSyncer, UsageEvolutionSyncer]`,
    then extension observers (e.g. `ReliabilityProbeFrameworkAdapter`), then `Reconfigurator`;
  - interpreter listeners: extension listeners, then `ProbeFrameworkListener`;
  - entity managers: `[ComponentInstanceRegistry, AssemblyAllocationManager]`, the QUAL task (recorder configuration
    init), extension managers.
  Only the extension-contributed sub-sets come from Dagger `SetFactory`/`SetBuilder` = `HashSet` (identity-hash
  order); in default 5.2.2 they do not produce EDP2 measurements.
- **MEAS-9.2 Calculator creation order**: (1) `ResourceEnvironmentSyncer`: resource environment → containers →
  processing resources (model order) → monitors/specs (repository order): [overall], state (+ initial tuple at
  t=0), demand; (2) extension observers (execution-result calculators); (3) `Reconfigurator` (container count);
  (4) `ProbeFrameworkListener.initialize` (`AbstractProbeFrameworkListener.java:78-92`): response-time calculators,
  reconfiguration-time calculators, then decorators from extension point
  `org.palladiosimulator.simulizar.interpreter.listener.probeframework` in extension-registry order
  (installation dependent): utilisation, sliding window, aggregation, feed-through; (5) passive-resource
  calculators lazily during the run (MEAS-5.1).

## MEAS-10 End of run

- **MEAS-10.1** `SimuComModel.finalise` (SIM-6.5): (a) `simulationStop` listeners: final partial windows
  (MEAS-6.3); (b) `deactivateAllActiveResources` in `ResourceRegistry` HashMap order (String hash of container
  ids), per container in HashMap order of resource-type ids (`[SCC]ResourceRegistry.java:166-174`,
  `[SCC]resources/AbstractSimulatedResourceContainer.java:32`): for instance 0..N-1 `fireStateEvent(0, i)` →
  the state probe records the **current** queue length (not 0) at the stop time; then `fireOverallUtilization`
  once more (`AbstractScheduledResource.java:212-227`, `ScheduledResource.java:147-158`). **These tuples are
  recorded** (not gated by `isRunning()`); they arrive after the final window flush and are never aggregated.
- **MEAS-10.2** Response times of in-flight requests are not emitted (MEAS-1.5, SIM-6.5).
- **MEAS-10.3** Cleanup flushes recorders in calculator-registry `HashSet` order (metric-id String hash); this
  affects only write order, not series content (`[PF]calculator/RegisterCalculatorFactoryDecorator.java:98-103`).
  EDP2 metadata contains wall-clock dates.

## MEAS-11 Emission order

- **MEAS-11.1** All emission is synchronous in the running coroutine, in program order; e.g. nested response times:
  US start < ELSC start < SysOp start < AssemblyOp start < … < AssemblyOp stop < SysOp stop < ELSC stop < US stop
  < execution result. No hash iteration on the emission path.
- **MEAS-11.2** SimOxide records, per series `(measuring point, metric)`, the tuples in emission order. The
  trace's `meas` lines ([formats](../guide/formats.md) §2) show the cross-series emission order, which equals
  the reference's.
- **MEAS-11.3** Measurement-related nondeterminism (added to SIM-9): calculator-registry HashSet (flush order only);
  `DeferredMeasurementInitialization` HashSet of suppliers (PRM only); extension-registry order of decorators
  (creation order of windows → order of same-time window events and final flushes); PS tie-breaking (ND-2)
  changes the order and timing of everything. The deterministic reference fixes all of them
  ([reference patches](../reference-simulator/patches.md)).

## MEAS-12 Measurement count

See SIM-6.4: only scenario runs of workload users count, independent of monitors; closed users count after the
think time.

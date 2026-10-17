# Monitoring and measurements (Slingshot)

Rules `SS-MEAS-*`. Slingshot measures only what the **monitor repository** asks for, through the
Palladio probe framework (calculators on probes) with Slingshot's event-based probes. The reference
runner writes EDP2's raw series as `measurements.csv` in SimOxide's format
([formats](../../guide/formats.md) §4).

## SS-MEAS-1 The measurement path

- **SS-MEAS-1.1 Calculators.** At `PreSimulationConfigurationStarted` every activated monitor of the
  repository yields one `MonitorModelVisited(processing type)` and one
  `MeasurementSpecificationVisited` per measurement specification (`[MON]interpreter/MonitorModelVisitor.java:29-60`,
  `MonitorRepositoryInterpreterBehavior.java:33-41`; inactive monitors are skipped, a repository without
  monitors disables the interpreter). The behaviours of SS-MEAS-2 build a calculator for the
  specifications they understand; all others are ignored silently.
- **SS-MEAS-1.2 Observers.** The calculator factory chain (`[MON]calculator/CalculatorFactoryProvider.java:24-37`)
  attaches, in this order:
  1. the EDP2 recorder (`RecorderAttachingCalculatorFactoryDecorator.java:43-57`): the raw tuple is
     written **synchronously** when it is measured;
  2. the `MeasurementMade` observer (`MonitoringAttachingCalculatorFactoryDecorator.java:51-64`,
     `observer/EventBasedMeasurementObserver.java:31-36`): schedules a `MeasurementMade` at delay 0
     directly (SS-BUS-5). The measuring point is a copy of the monitor's
     (`MonitoringAttachingCalculatorFactoryDecorator.java:53-56`);
  3. later, processing-type listeners (FeedThrough, aggregators), which schedule `MeasurementUpdated`
     directly (`calculator/ProcessingTypeMeasurementSourceListener.java:68-80`). Their output goes
     to no recorder: processing types do not change `measurements.csv`.
- **SS-MEAS-1.3 A probe is taken inside the handler of the event it measures**, and its time is that
  event's time (`[MON]probes/EventBasedListProbe.java:91-114`, `EventBasedProbe.java:62-65`). So a row is
  recorded at the dispatch of `ActiveResourceStateUpdated`, `ResourceDemandCalculated`, a passed
  `Start`/`Stop`, … and its `MeasurementMade` is queued then, before the handler's results
  (`ProbeTaken`, no subscriber).
- **SS-MEAS-1.4** Row order within a series = probe order; `measurements.csv` sorts the series by
  key. Every recorded row has exactly one `MeasurementMade` (smoke corpus, probes:
  `rows == measurements_made`).

## SS-MEAS-2 Which monitors are honoured

| Measuring point | Metric | Behaviour | Probe(s) |
|---|---|---|---|
| `UsageScenarioMeasuringPoint` | Response Time | `[USM]UsageScenarioResponseTimeMonitoringBehavior` | start: passed `Start`, stop: passed `Stop` of the root behaviour |
| `AssemblyOperationMeasuringPoint` (operation provided role) | Response Time | `[SYM]OperationCallActionResponseTimeMonitoringBehavior` | passed `StartAction`/`StopAction` of a root SEFF |
| `ActiveResourceMeasuringPoint` | State of Active Resource, Utilization of Active Resource, Resource Demand | `[RSM]monitor/ActiveResourceMonitorBehavior` | `ActiveResourceStateUpdated`, `ResourceDemandCalculated` |
| `AssemblyPassiveResourceMeasuringPoint` | Waiting Time, Holding Time, State of Passive Resource | `[RSM]monitor/PassiveResourceMonitoringBehavior` | passive request, acquire, release, state events |
| resource-environment / service-group points | Number of Elements | `NumberOfElementsMonitorBehavior` (two) | `ModelAdjusted` only |

- **SS-MEAS-2.1 Usage scenario response time** (`UsageScenarioResponseTimeMonitoringBehavior.java:59-103`):
  start-stop calculator; the request context is the user id. Start = time of the passed `Start` of the
  root behaviour (the arrival, or the end of the think time), stop = time of the passed `Stop`
  (double subtraction `stop − start`). Not measured: aborted iterations, iterations open at
  `simTime`.
- **SS-MEAS-2.2 Not honoured** (no calculator, no row, no warning): `EntryLevelSystemCallMeasuringPoint`,
  `SystemOperationMeasuringPoint`, `ExternalCallActionMeasuringPoint`, `ResourceURIMeasuringPoint`
  (refsim's passive and reconfiguration points), any other metric on the points above. Quirk
  [SS-Q29](./quirks.md#ss-q29). Probe `ss_monitors` (only the scenario's rows); smoke `x_ss_minimal`.
- **SS-MEAS-2.3 Assembly operation: keyed by the provided role of the request**
  (`OperationCallActionResponseTimeMonitoringBehavior.java:80-128`, `AssemblyOperationCompoundKey.java:56-71`).
  A root SEFF's passed `Start` is matched by (assembly context, the *request's* provided role,
  signature):
  - for a SEFF reached through an assembly connector, the role is the connector's provided role:
    the natural measuring point matches;
  - for the SEFF of an **entry-level call**, the role is the **system's outer role**, so a measuring
    point with the component's role never matches; one with the system role and that assembly does.
    Quirk [SS-Q33](./quirks.md#ss-q33).

  Probe `ss_role_lookup`: `(FrontB, prov_front)` records the call through the connector,
  `(FrontA, sysprov2)` records the entry call, `(FrontA, prov_front)` records nothing.
  Request contexts are user id plus the caller context's identity hash
  (`:159-170`).
- **SS-MEAS-2.4 Infrastructure calls break the assembly-operation monitor.** The key is computed for
  every root SEFF start, before the monitor lookup; a SEFF reached through an infrastructure call has
  neither a user request nor a call-over-wire request, so `AssemblyOperationCompoundKey.of` throws
  (`AssemblyOperationCompoundKey.java:56-67`) and both handlers of the behaviour are disposed
  (SS-BUS-7). No assembly-operation response time is recorded after the first infrastructure call.
  Quirk [SS-Q31](./quirks.md#ss-q31). Corpus `h18_infrastructure_call` (two handler errors).

## SS-MEAS-3 Active resources

`[RSM]monitor/ActiveResourceMonitorBehavior.java:55-103`, `ActiveResourceProbeTable.java:33-63`:

- **SS-MEAS-3.1 Probes per specification, not per replica.** The probes are keyed by the
  `ProcessingResourceSpecification`; the measuring point's `replicaID` is ignored. A monitor for
  replica 1 receives the same series as one for replica 0 (probe `ss_ps_multicore`: two identical
  series). Quirk [SS-Q34](./quirks.md#ss-q34).
- **SS-MEAS-3.2 State of Active Resource** = `requestsAtResource` of the `ActiveResourceStateUpdated`:
  all jobs at the resource ([SS-RES-2.7](./resources.md#ss-res-2-processor-sharing),
  [SS-RES-3.5](./resources.md#ss-res-3-fcfs)), at every arrival, completion and stale completion.
  **SimuLizar:** per-core queue lengths, only on change for PS (Schedulers, `reportCoreUsage`).
- **SS-MEAS-3.3 Utilization of Active Resource** = busy cores / cores at the same instants: an
  **instantaneous sample, no sliding window**. Quirk-free Slingshot semantics. **SimuLizar:**
  time-driven sliding windows (MEAS-6).
- **SS-MEAS-3.4 Resource Demand** = `demand / rate` at the job's start (`ResourceDemandCalculated`),
  also 0 for zero demands. Linking jobs are not measured.
- **SS-MEAS-3.5 Order within one state event:** state probe, then utilisation probe
  (`ActiveResourceProbeTable.java:39-51`): `MeasurementMade(state)`, its `MeasurementUpdated`s,
  `MeasurementMade(utilisation)`, …, then two `ProbeTaken`.

## SS-MEAS-4 Passive resources

`[RSM]monitor/PassiveResourceMonitoringBehavior.java:46-112`, `PassiveResourceProbeTable.java`:

- **SS-MEAS-4.1** Probes are keyed by the passive resource **id only** (`PassiveResourceProbeTable.java:28-31`):
  the instances of one passive resource in several assembly contexts share one set of probes, and
  each monitor gets the merged series. Quirk [SS-Q34](./quirks.md#ss-q34).
- **SS-MEAS-4.2 Waiting Time** = acquire − request (start at `ResourceDemandRequested(PASSIVE)`, stop at
  `PassiveResourceAcquired`); **Holding Time** = release − acquire (stop at `PassiveResourceReleased`);
  **State of Passive Resource** = available units, sampled at `PassiveResourceStateUpdated`, i.e. after
  an immediate grant and after each release with its re-grants, not when a request queues
  ([SS-RES-4](./resources.md#ss-res-4-passive-resources)). Probe `ss_passive`.
- **SimuLizar:** state after every acquire and release (scheduler page, passive resources).

## SS-MEAS-5 Other measurements

- **SS-MEAS-5.1** Number of elements (resource containers of an elastic infrastructure, service-group
  assemblies) is measured only on `ModelAdjusted` (`NumberOfElementsMonitorBehavior.java:105-131`),
  not at t = 0.
- **SS-MEAS-5.2** No measurement is taken at `simTime` (SS-RUN-4).
- **SS-MEAS-5.3** Cloned monitors of SPD scale-outs produce series for the new containers from the
  `ModelAdjusted` on ([SS-SPD-5](./spd.md#ss-spd-5-applying-the-change)).

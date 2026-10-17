# SimOxide: runtime reconfiguration, a Slingshot mode, SPD and snapshots (design)

State: 2026-10-17. This is a design. Phases S0 (the Slingshot reference runner,
`reference/slingshot/`), S1 (the Slingshot semantics specification, `docs/spec/slingshot/`, rules
`SS-*`), F0, F1, F2, F3 (the QVTo host, `crates/simoxide-reconf`), P1 (the SPD interpreter,
`crates/simoxide-spd`), S2 (the Slingshot flavour's fast mode), Q1 (the SPD adjustment corpus), Q2
(QVTo's ordered mode), R1 (SimuLizar's reconfiguration rules), X1 (the Slingshot-exact corpus) and
X2 a-c, e (the Slingshot-exact mode, without SPD) are implemented; the rest is not.
User decisions of §8 are recorded. It is on branch `spd`
(forked from `qvto-adapter`). It is a plan, so it sits next to the code and not in the VitePress
site, which describes the current state (`docs/development/contributing.md`). The parts that get
built move into `docs/spec/` and `docs/guide/`.

## 0. Goal and summary

The user's goal:

- SimOxide gets a **Slingshot-compatible mode** next to its SimuLizar-compatible mode. The two
  share as much code as possible, and there is a performance comparison against Slingshot.
- **Both modes can be extended through QVTo** on qvto-rs:
  - the SimuLizar mode through SimuLizar's reconfiguration rules;
  - the Slingshot mode through SPD (Scaling Policy Definitions) with Slingshot's QVTo
    adjustment transformations.
- SPD follows Slingshot ("Slingshot-inspired") and runs Slingshot's own QVTo transformations.
- **Snapshot and restore** in both modes, for the MENTOR project: pause a run, change models or
  policies, continue. Slingshot already has this in its initialisation extension.

The design in one paragraph:

- One engine with two **flavours**, `SimuLizar` and `Slingshot`, chosen per run. Both share the
  model, the compiler, the behaviour interpreter, the schedulers, the measurement sink and the
  output.
- A shared **runtime-model-change foundation**:
  - a per-run mutable model graph with an overlay over the immutable `CompiledModel`;
  - a QVTo host. The QVTo engine moves into the SimOxide repository (`crates/qvto/*`) and runs
    directly on the shared EMF graph;
  - syncers driven by the change journal.
- On top of the foundation, SimuLizar's rule process and Slingshot's SPD interpreter. The SPD
  interpreter is Rust; the adjustment runs `MainTransformation.qvto` unchanged.
- The run state is a plain value, so a snapshot is a `clone()`. It can also be serialised.
- The **SimuLizar flavour stays byte-exact**.
- The **Slingshot flavour has two modes** (decision U4):
  - **Slingshot-exact**: the same events, random draws and measurements as a slightly patched,
    deterministic Slingshot (the S0 runner's `det` mode: seeded randomness, insertion-ordered
    same-time events, instrumentation);
  - **fast**: only the semantics count; statistically equal to Slingshot.
- Slingshot bugs are fixed by default. A compat flag per documented quirk (Q1-Q37) reproduces it,
  and the exact mode runs with all quirk flags on (U5). The fixed behaviour is SimuLizar's
  (SimOxide's) semantics; only genuine Slingshot design differences stay Slingshot-specific (U13).
- The exact mode targets **level E directly**: the full projected event trace, including the
  per-handler cascade and handler disposal (U14).
- SPD works in both flavours (U6).
- The oracle is the headless Slingshot reference runner `reference/slingshot/` (S0, §4.1).

Sources (all read-only):

| Tag | Path (under `/home/devbox/workspace/`) |
|---|---|
| `[SS]` | `old/palladio-research/repos/Palladio-Analyzer-Slingshot/bundles/` (commit `8bf9adc`, 2026-08-21) |
| `[SPD]` | `[SS]org.palladiosimulator.analyzer.slingshot.behavior.spd/src/org/palladiosimulator/analyzer/slingshot/behavior/spd/interpreter/` |
| `[ADJ]` | `[SS]org.palladiosimulator.analyzer.slingshot.behavior.spd.adjustment/src/org/palladiosimulator/analyzer/slingshot/behavior/spd/` |
| `[RS]` | `[SS]org.palladiosimulator.analyzer.slingshot.behavior.resourcesimulation/src/org/palladiosimulator/analyzer/slingshot/behavior/resourcesimulation/` |
| `[CORE]` | `[SS]org.palladiosimulator.analyzer.slingshot.core/src/org/palladiosimulator/analyzer/slingshot/core/` |
| `[MM]` | `old/palladio-research/repos/Palladio-Addons-SPD-Metamodel/bundles/` (commit `95cdb0b`): `org.palladiosimulator.spd/model/SPD.ecore`, `org.palladiosimulator.spd.semantic/model/semantic.ecore` |
| `[TR]` | `[MM]org.palladiosimulator.spd.semantic.transformations/transformations/` |
| `[INIT]` | `old/palladio-research/repos/Palladio-Analyzer-Slingshot-Extension-Initialisation/` (commit `1d8b41b`), bundles `…snapshot`, `…snapshot.data` |
| install | `old/palladio-research/work-slingshot/sl-install`: Slingshot nightly `1.0.0.202609010045`, Palladio 6.0.0 nightly, Eclipse 2025-12, QVTo 3.11.0 |
| plan | `history/qvto-simoxide-integration.md` (steps 1-7, D1-D6; formerly `qvto-rs/docs/simoxide-integration.md`); the adapter `qvto-simoxide` it describes was deleted in merge step M6 |

Abbreviated paths below drop the prefixes; line numbers refer to those commits.

## 1. Slingshot's SPD semantics

### 1.1 Metamodel

`SPD.ecore` (nsURI `http://palladiosimulator.org/ScalingPolicyDefinition/1.0`, with
sub-packages `…/Targets`, `…/Adjustments`, `…/Constraints{,/Policy,/Target}`,
`…/Triggers{,/Stimuli,/Expectations}`, all `/1.0`):

- **`SPD`** contains `scalingPolicies [1..*]` and `targetGroups [1..*]` (lines 22-35).
- **`ScalingPolicy`** (5-19) has:
  - `active`;
  - `adjustmentType` (contained);
  - `targetGroup` (a reference);
  - `policyConstraints [*]` (contained);
  - `scalingTrigger` (contained).
- **Target groups** (38-70):
  - `ElasticInfrastructure`: `unit`, a `ResourceContainer`;
  - `ServiceGroup`: `unitAssembly`, an `AssemblyContext`;
  - `CompetingConsumersGroup`: `unitAssembly`;
  - each group has `targetConstraints [*]`.
- **Adjustments** (77-117):
  - `StepAdjustment.stepValue`;
  - `RelativeAdjustment.percentageGrowthValue` (default 100) and `minAdjustmentValue`;
  - `AbsoluteAdjustment.goalValue`.
- **Constraints** (125-186):
  - policy constraints: `IntervalConstraint` (`offset`, `intervalDuration`, `repeat`) and
    `CooldownConstraint` (`cooldownTime`, `maxScalingOperations`, default 0);
  - target constraints: `TargetGroupSizeConstraint` (`minSize`, `maxSize`) and
    `ThrashingConstraint` (`minimumTimeNoThrashing`).
- **Triggers** (190-260):
  - `SimpleFireOnValue` has a stimulus, an expected value and a `RelationalOperator`
    (`LessThan`, `GreaterThan`, `EqualTo`, `LessThanOrEqualTo`, `GreaterThanOrEqualTo`);
  - `SimpleFireOnTrend`;
  - `ComposedTrigger` combines 2 or more triggers with `AND`/`OR`/`XOR`.
- **Stimuli** (267-358):
  - `SimulationTime`, `OperationResponseTime`, `NumberOfElements`;
  - `CPUUtilization`, `MemoryUtilization`, `HDDUtilization`, `NetworkUtilization`;
  - `TaskCount`, `QueueLength`;
  - managed-element stimuli carry `aggregationOverElements` (`AVERAGE`, `MAX`, `MIN`,
    `MEDIAN`, `SUM`).
- **Expectations** (365-384): `NoExpectation`, `ExpectedPercentage`, `ExpectedCount`,
  `ExpectedTime`, `ExpectedTrend`.

`semantic.ecore` (`…/ScalingPolicyDefinitionSemantic/1.0`) holds the semantic configuration.
The transformations read and write it:

- `Configuration` has:
  - `spd`, `enactedPolicy`;
  - `targetCfgs` (contained);
  - `allocation`, `system`, `resourceEnvironment`, `repository`.
- `ElasticInfrastructureCfg` has `unit`, `elements [1..*]` (the current containers),
  `linkingResource` and `resourceEnvironment`.
- `ServiceGroupCfg` has `unit`, `elements` (assemblies) and `loadBalancingAssembly`.
- `CompetingConsumersGroupCfg` has queues.

### 1.2 The control loop

**Set-up.**

- `SpdBehavior` (`[SPD]SpdBehavior.java:51-65`) interprets the SPD model once, on
  `PreSimulationConfigurationStarted` at t = 0.
- It registers one handler set per active policy and schedules the events that triggers need.
  Inactive policies are skipped (`SpdInterpreter.java:50`).
- Each target group gets one `TargetGroupState` that all its policies share (`:38`).
- Each policy gets an `SPDAdjustorContext` with a **filter chain**
  (`entities/SPDAdjustorContext.java:80-94`). The filters run in this order:
  1. `TargetGroupChecker`:
     - a `MeasurementMade` passes if its measuring point lies inside the target group;
     - a `SimulationTimeReached` passes if it is for this target group;
     - everything else is disregarded (`entity/targetgroup/TargetGroupChecker.java:25-41`).
  2. The trigger checker.
  3. The `ThrashingConstraint`s of the target group. **Only thrashing constraints** go into
     the chain (`SPDAdjustorContext.java:86`). `TargetGroupSizeConstraint` is enforced inside
     the QVTo adjustment (§1.3).
  4. The policy constraints, in model order (`:89-91`). Only Cooldown and Interval exist; any
     other kind throws (`entity/constraint/AbstractConstraintFilter.java:25-35`).
  5. The `Adjustor`.

**Running the chain.**

- Each event that reaches the policy's handler runs the chain from the start
  (`entities/FilterChain.java:100-123`).
- A filter returns *success* (pass on to the next filter) or *disregard* (stop). An exception
  in a filter counts as *disregard*.
- If the last filter succeeds, the handler returns its result event (`SPDAdjustorContext.java:136-148`).
- Quirk: `doOnDisregard` assigns `state = previousState`, but the chain keeps the state object it
  was built with (`:57`, `:109-112`). So disregarding has no effect on the state.

**Triggers.**

`ScalingTriggerInterpreter.java:29-129` and `StimuliInterpreter.java:40-108` support these
stimuli:

- **`SimulationTime`**:
  - schedules one `SimulationTimeReached(targetGroupId, expectedTime)` at set-up (`:41-50`);
  - fires if `t <op> expected` (`entity/trigger/SimulationTimeChecker.java:20-34`);
  - so it fires **once**, at the expected time. `GreaterThan` and `LessThan` never fire on the
    policy's own event.
  - Policies on the same target group also receive each other's `SimulationTimeReached`. With
    `>=`, a policy can fire on another policy's time event.
- **`OperationResponseTime`**:
  - every `MeasurementMade` at an `OperationReference` measuring point with the same
    signature id is compared directly, without aggregation
    (`entity/trigger/OperationResponseTimeTriggerChecker.java:28-57`);
  - expects an `ExpectedTime`.
- **`CPUUtilization`**:
  - takes `MeasurementMade` with metric set "utilisation of active resource tuple", value
    "utilisation of active resource" (`CPUUtilizationTriggerChecker.java:13-22`);
  - expects an `ExpectedPercentage`, compared as `value / 100` (`SimpleFireOnValueComparator.java:44-46`).
- **`TaskCount`**: the same, with the state-of-active-resource tuple and an `ExpectedCount`
  (`TaskCountTriggerChecker.java`).
- **`QueueLength`**: only for `CompetingConsumersGroup` (`StimuliInterpreter.java:88-96`).
- `SimpleFireOnTrend` falls back to the default (no checker). `NumberOfElements`, memory, HDD
  and network stimuli have no case: they are **not supported**.
- `NoExpectation` always fires. A wrong expectation type gives "wrong expected value", which is
  not a fire (`TriggerChecker.java:65-79`).
- The comparison is plain `double`: `==`, `>=`, `>`, `<=`, `<` (`SimpleFireOnValueComparator.java:23-30`).
- Composed triggers (`ScalingTriggerInterpreter.java:83-124`):
  - `AND` → `LogicalANDComboundFilter`;
  - `OR` → `LogicalORCompoundFilter`;
  - `XOR` → `LogicalXORCompoundFilter`.
- **The composed triggers are broken** (`entities/Logical*CompoundFilter.java`; `FilterChain.java:100-139`):
  - **AND**: `disregard` does not reset the iterator. The next event resumes after the child
    that failed and can succeed without evaluating any child.
  - **OR**: `next` never evaluates its children and returns `null`. The outer chain stays
    parked after the OR filter, so the next event skips the target-group check and the trigger.
    OR policies fire on roughly every second event, whatever the values.
  - **XOR**: with two children, the result is inverted after the first child.
  - Two stimulus kinds in one policy register two `MeasurementMade` subscribers, so each event
    runs the chain twice (`Subscriber.java:269-282`).

**Where the measurements come from.**

- Every calculator gets an `EventBasedMeasurementObserver`. It schedules a `MeasurementMade`
  (delay 0) for **every raw measurement**
  (`[SS]…monitor/…/recorder/decorator/MonitoringAttachingCalculatorFactoryDecorator.java:50-64`,
  `…/monitor/observer/EventBasedMeasurementObserver.java:30-36`).
- SPD sees raw calculator output, not the output of processing types: FeedThrough emits
  `MeasurementUpdated`.
- A measurement exists only if the monitor repository has a monitor for that metric at that
  measuring point.
- Every policy subscribes to the event *type*. Every `MeasurementMade` in the run enters every
  measurement-driven policy's chain; `TargetGroupChecker` filters it
  (`utils/MeasuringPointInsideTargetGroup.java:31-60`, `utils/TargetGroupUtils.java:44-56,242-248`).
- **The CPU value is instantaneous**, sampled on every `ActiveResourceStateUpdated`
  (`[RS]…resourcesimulation.monitor/…/ActiveResourceMonitorBehavior.java:95-103`):
  - PS: the number of busy cores divided by the number of cores
    (`[RS]resources/active/ProcessorSharingResource.java:277-285`);
  - FCFS: 0 or 1 (`FCFSResource.java:148-153`).
  - It is not a windowed utilisation.

**Aggregation of managed-element stimuli** (`AbstractManagedElementTriggerChecker.java:53-137`):

- Measurements whose metric-set id matches are aggregated at their point-in-time measure
  (`:119-123`). The check fires only when the aggregator can emit (`:103-113`).
- `getResult` runs for **every** event that reaches the checker, also for events of other
  metrics that were not aggregated (`:76-95`).
- All containers of the group feed **one merged stream**. There is no per-container
  aggregation: MAX is the maximum of the last 10 samples of any container.
- Aggregators consume samples and the EMA emit slot even when a later constraint rejects the
  event. Reading the value consumes the emit slot even when the comparison fails.
- No samples means no evaluation. An idle system does not decay the EMA.
- For `AVERAGE`: `SlidingTimeWindowAggregationBasedOnEMA(60, 10, 0.2)` (`:67-68`):
  - `v = 0.8·v + 0.2·x`, then `v *= exp(-(t - t_last)/60)`;
  - it can emit if `t_last > 60` and `t_last - t_emit > 10`;
  - reading the value sets `t_emit = t_last` (`entity/aggregator/SlidingTimeWindowAggregationBasedOnEMA.java:50-88`).
  - So an average-utilisation policy cannot fire before t = 60 s, and at most once per 10 s.
- For `MAX`, `MIN`, `MEDIAN`, `SUM`: `FixedLengthWindowSimpleAggregation` with **the last 10
  values** (count-based, not time-based). It can emit only when 10 values are present
  (`FixedLengthWindowSimpleAggregation.java:41-112`).
- `SlidingTimeWindowAggregation` (time-based, `:51-77`) exists but is **unused**.

**Constraints** (state in `SPDAdjustorState`, per policy, and `TargetGroupState`, per group):

- **Cooldown** (`CooldownConstraintFilter.java:28-41`):
  - passes if `t >= coolDownEnd`, or if `numberOfScalesInCooldown < maxScalingOperations`;
  - otherwise it disregards.
  - The `Adjustor` updates the state (`Adjustor.java:38-59`), only if `cooldownTime != 0`:
    - `t > coolDownEnd`: reset the count to 0 and set `coolDownEnd = t + cooldownTime`;
    - `t == coolDownEnd`: only move `coolDownEnd`;
    - otherwise increment the count.
- **Interval** (`IntervalConstraintFilter.java:13-51`), with `delta = offset + intervalDuration`:
  - It **disregards** inside the interval:
    - `repeat`: `t != 0 && (t % delta == 0 || t % delta >= offset)`;
    - not `repeat`: `offset <= t <= delta`.
  - Note the inverted reading: the interval is the *inactive* period.
- **Thrashing** (`ThrashingConstraintFilter.java:37-75`):
  - It passes if nothing was enacted yet in the group, or if either the current or the last
    enacted policy is Absolute.
  - Otherwise it disregards if the signs differ and `lastTime + minimumTimeNoThrashing >= t`.
  - Sign: step or relative value > 0 is positive.
- **Adjustor** (`Adjustor.java:28-61`):
  - records `latestAdjustmentAtSimulationTime`, `numberScales++`;
  - adds the enacted policy and time to the target group's state;
  - updates the cooldown and returns `ModelAdjustmentRequested(policy)`.
  - The constraint state changes **when the request is made**, even if the adjustment later
    fails or changes nothing.

**When evaluation happens.** Only on events. There is no periodic evaluation.

- `SimulationTimeReached` comes at its time.
- `MeasurementMade` comes whenever the monitoring produces a value for a monitor of the monitor
  repository.
- Triggers therefore need monitors on the right measuring points: utilisation needs a
  utilisation monitor of the container's resource.

### 1.3 The adjustment (QVTo)

`SpdAdjustmentBehavior.onModelAdjustmentRequested` (`[ADJ]adjustment/SpdAdjustmentBehavior.java:73-127`):

1. It sets `Configuration.enactedPolicy = event.policy` (`:82`). It copies the lists of
   containers and allocation contexts.
2. It runs `MainTransformation.qvto` through `QVToReconfigurator.execute` (`:86`;
   `SpdAdjustorModule.java:17,38`).
   - Models bind by nsURI from the model cache, as in SimuLizar.
   - Signature (`[TR]spd/MainTransformation.qvto:34-40`):
     `inout allocation, inout system, inout resourceEnvironment, inout repository, in spd,
     inout spdSemantic, inout usageModel`.
   - Configuration: `keepModeling=true` only (`AbstractQVToExecutor.java:205`).
     `max_assemblies_per_host` is declared and never set.
3. On success it builds `ModelAdjusted(true, changes)` (`:90-122`):
   - a `ResourceEnvironmentChange` with old, new and deleted containers;
   - an `AllocationChange` with the new allocation contexts;
   - one `MonitorChange` per monitor cloned for each new container. `ResourceContainerMonitorCloner`
     copies the monitors of the target group's `unit` container (`:141-156`).
4. On failure (`assert fatal`, compile error) it builds `ModelAdjusted(false, [])`. Nothing changes.

What `MainTransformation` does:

- It dispatches to bottom-up (target is an ElasticInfrastructure) or top-down (ServiceGroup,
  CompetingConsumersGroup) (`Main:65-77`, after fatal preconditions `:80-100`).
- **Size change** (`spd/common/AdjustmentCalculator.qvto`):
  - Step: `+stepValue`.
  - Relative: `x = cur·pct/100`; the change is `max(x, minAdj)` if `x > 0`, else `min(x, minAdj)`.
  - Absolute: `goal - cur`, with `goal > 0` asserted.
  - A `TargetGroupSizeConstraint` clamps the change to `[min - cur, max - cur]` (`:47-55`).
- **Scale-out**:
  - It appends `ceil(change)` copies of the unit container, named `<unit>_<n>`. Each copy gets
    the same processing resource specifications (`pcm/helpers/Constructors.qvto:77-111`).
  - **Every linking resource is connected to all containers**
    (`spd/common/ResourceEnvironmentTransformation.qvto:17-23`).
  - Service groups and competing-consumer groups on the unit follow
    (`spd/bottomup/BottomUpTransformationServices.qvto`):
    - each new container gets a new `AssemblyContext` and its `AllocationContext`;
    - the system's connectors are replicated;
    - the **load-balancer component's SEFF** gets one more required role and one more
      probabilistic branch transition. All branch probabilities become `1/n`
      (`spd/common/ServiceGroupRepositoryTransformation.qvto:33-65`).
- **Scale-in**:
  - It keeps `elements[1..max(1, size - round(|change|))]`, so **the newest elements are
    removed**. At least one element always stays (`BottomUpTransformationInfrastructure.qvto:84-107`).
  - Assemblies, allocations and connectors on removed containers are deleted. The
    load-balancer roles and branches shrink, and probabilities are reset to `1/n`.
  - Top-down scale-in removes only containers that are already empty. Containers that the
    current step empties go at a later enactment (`TopDownTransformationInfrastructure.qvto:58-63`).
- The `ProfilesLibrary` blackbox is imported by `Constructors`, `Wiring` and `Loadbalancing`,
  but the main path never calls it.

So in Slingshot, routing to replicas is **in the model**: a load-balancer component with
probabilistic branches. There is no simulator-level choice among allocations. Each assembly
context has one allocation context; the lookup takes the first match
(`[RS]repository/ResourceEnvironmentAccessor.java:42-47`).

### 1.4 How Slingshot's simulation applies the change

`ResourceSimulation.onModelAdjusted` (`[RS]ResourceSimulation.java:329-350`):

- **New containers**: active resources are created for every specification (`createActiveResourcesFromResourceContainer`).
- **New allocation contexts**: passive resources are built.
- **Deleted containers**: their active resources are **removed from the table**
  (`resources/active/ActiveResourceTable.java:80-85`). Jobs are not cleared.
  - A later `JobInitiated` or `JobProgressed` for such a resource becomes
    `JobAborted(…, "active resource is not found, may have been deleted")` (`:190-206`, `:253-269`).
  - Only the job that holds the scheduled `JobProgressed` gets one: the shortest job for PS
    (`ProcessorSharingResource.java:151-174`), the head job for FCFS (`FCFSResource.java:61-67`).
    It is aborted, then `ResourceDemandRequestAborted`, then `UserAborted`, and the usage
    simulation restarts the user (`SystemSimulationBehavior.java:284-290`).
  - **All other jobs on the removed resource are silently lost.** Closed-workload users
    disappear and their response times are never recorded.
  - **Confirmed by S0** (`reference/slingshot/models/spd_scale_in`). `AppServer_1` is removed at
    t = 80 while four jobs run on it:
    - the shortest job is aborted at its next `JobProgressed` (t = 87), and its open-workload user
      is dropped;
    - the other three jobs produce no further event, and their users never finish.
  - SimuLizar instead keeps simulating removed containers.
- **Deleted allocation contexts, system, repository and usage changes are not reported** in
  `ModelAdjusted`. Monitors and passive resources of removed elements are never removed.
- **Closed-workload population changes** that the competing-consumers transformation makes have
  no effect: the population is read only at the start
  (`[SS]…usagesimulation/…/UsageSimulationBehavior.java:247`).
- **Calls over the network** use the first linking resource that connects both containers. If
  none connects both, they use any link that touches one of them ("the other one got deleted
  during scaling in", `:351-388`).
- **System and SEFFs are read live** from the model by the interpreters. New assemblies,
  connectors and load-balancer branches therefore take effect for the next call.
- **Monitors**: `MonitorRepositoryInterpreterBehavior` and both `NumberOfElementsMonitorBehavior`s
  subscribe to `ModelAdjusted`. The cloned monitors start producing values for the new containers.
  - Observed in a reference run (§4.1): `CPU [0] in AppServer_1` reports utilisation after the
    scale-out at t = 50.

**Timing.**

- There is no reconfiguration delay. `ModelAdjustmentRequested` is a result event scheduled at
  `now`, behind events already queued for `now`.
- The adjustment runs when it is dispatched. `ModelAdjusted` is queued at `now` again.
- Reference run of the minimal example (§4.1):
  - `SimulationTimeReached` at 50.0;
  - then `JobProgressed`;
  - then `ModelAdjustmentRequested` (the QVTo run);
  - then `JobFinished`, `ActiveResourceStateUpdated`;
  - then `ModelAdjusted`, all at t = 50.0.
- There is no cost or duration of reconfiguration (no cold start). Containers are usable at once.

### 1.5 Quirks (fixed by default; a compat flag reproduces each one, U5)

Q1-Q16 are the SPD quirks; S1 added Q17-Q37 (`docs/spec/slingshot/quirks.md`, which also gives the
fixed behaviour of each, following U13).

| # | Quirk | Source |
|---|---|---|
| Q1 | `doOnDisregard` does not restore the state | `SPDAdjustorContext.java:57,109-112` |
| Q2 | Constraint state changes when the request is made, even if QVTo fails | `Adjustor.java:28-61` |
| Q3 | AVERAGE uses an EMA (60 s, 10 s, α = 0.2); the others use the last 10 values | `AbstractManagedElementTriggerChecker.java:67-71` |
| Q4 | Relative 0 % ignores `minAdjustmentValue` | `AdjustmentCalculator.qvto:59-74` |
| Q5 | Scale-in aborts the one job with a scheduled event on a removed container; the other jobs there are lost | `ResourceSimulation.java:190-206,253-269` |
| Q6 | All linking resources connect all containers after an adjustment | `ResourceEnvironmentTransformation.qvto:17-23` |
| Q7 | Top-down scale-in removes emptied containers one enactment late | `TopDownTransformationInfrastructure.qvto:58-63` |
| Q8 | Top-down placement uses `->any(true)` on a set: Java-hash-dependent | `TopDownTransformationInfrastructure.qvto:112-143` |
| Q9 | Only ThrashingConstraint as target constraint in the chain; group size only in QVTo | `SPDAdjustorContext.java:86` |
| Q10 | `TargetGroupUtils` keeps the allocation and configuration in static fields: runs of different SPD models in one JVM interfere | `[SPD]utils/TargetGroupUtils.java` |
| Q11 | Unsupported stimuli (NumberOfElements, memory, HDD, network) and `SimpleFireOnTrend` give a null checker: NPE at configuration | `StimuliInterpreter.java`, `ScalingTriggerInterpreter.java:44-47`, `SpdInterpreter.java:56` |
| Q12 | Composed triggers AND/OR/XOR misbehave (see §1.2) | `Logical*CompoundFilter.java`, `FilterChain.java:100-139` |
| Q13 | Instantaneous CPU samples, merged across containers; `getResult` also on unaggregated events | `AbstractManagedElementTriggerChecker.java:76-123` |
| Q14 | Deleted allocations and assemblies not reported; monitors of removed containers stay; CCG population change ineffective | `SpdAdjustmentBehavior.java:96-117`, `UsageSimulationBehavior.java:247` |
| Q15 | Policies of one group react to each other's `SimulationTimeReached` (confirmed in S0) | `TargetGroupChecker.java:36-40` |
| Q16 | After an adjustment, the order of the environment's containers and of each link's containers follows a QVTo `Set(ResourceContainer)`, which is Java identity-hash order (found in S0) | `[TR]spd/common/ResourceEnvironmentTransformation.qvto:13-18` |

## 2. Slingshot vs SimuLizar beyond SPD

Slingshot is a separate simulator. For the same PCM model its results differ from SimuLizar's in
distribution or in detail. The points that matter for a Slingshot flavour:

- **Event core.**
  - SimuLizar: DESMO-J. Time is integer nanoseconds. A FIFO event list. Threads become state
    machines (SimOxide spec SIM-3, SIM-4).
  - Slingshot: an SSJ `Simulator` with double time (`[CORE]engine/SimulationEngineSSJ.java:22-150`).
    Every model step is a bus event. Handlers return result events, which are scheduled at
    `now + delay`. That is about 20 times more events per request than SimOxide
    (`docs/performance/comparison.md`).
  - Events with `time() > 0` are inserted directly at their absolute time (`:44-62`).
  - An SSJ event's `actions()` posts the event synchronously to an RxJava `PublishSubject`
    (`:107-119`; `[SS]…eventdriver/internal/BusImplementation.java:146-152`).
    - Subscribers run in registration order. Each one runs pre-interceptors, the handler, then
      post-interceptors (`…/entity/Subscriber.java:71-91`).
    - `CoreBehavior`'s post-interceptor schedules every result event at `now + delay`
      (`[CORE]behavior/CoreBehavior.java:36-52`). Results therefore run after the events already
      queued for the same time (FIFO, which is SSJ's event-list behaviour).
    - A handler that throws is disposed for the rest of the run (RxJava `LambdaObserver`).
    - The order of registration comes from the extension and Guice set-up. S1: the extension order
      was the same in every run; `getDeclaredMethods()` order varies between JVMs, but only for
      reified subscribers of disjoint types, so it never changes a run (SS-BUS-2).
  - **Same-time order.**
    - SSJ's `SplayTree` keeps equal times FIFO by insertion, as SIM-3.1 does.
    - But the result events of one handler sit in a `HashSet` ordered by identity hash
      (`[SS]…eventdriver/returntypes/Result.java:25-33`). So the order of same-time events from one
      handler is **random per run**.
    - The PS job table is a `Hashtable` keyed by `Job.hashCode = hash(UUID)`, so ties between
      jobs with equal remaining demand resolve randomly (`[RS]…/Job.java:59-62`).
    - Method subscription order within a class follows `getDeclaredMethods()`, which the JVM
      does not specify (`BusImplementation.java:116`).
- **Randomness.**
  - StoEx sampling uses the JVM-global `ProbabilityFunctionFactoryImpl` generator: an unseeded
    `java.util.Random` (`DefaultRandomGenerator`, `GeneralSimulationConfigurationBehavior.java:32-35`).
    Slingshot never seeds it.
  - The S0 runner injects SimuCom's MT19937 seeded `{N, …, N+5}`, or `java.util.Random(N)` with
    `-stoex jur`.
  - With MT, the StoEx draws on `x_ss_minimal` coincide with SimuLizar's: the response times equal
    refsim's up to DESMO-J's nanosecond truncation.
  - Inter-arrival times, think times and linking latencies are StoEx, so they come from the same
    generator.
  - Branch decisions in SEFFs and usage models, and link failures, use **unseeded
    `Math.random()`** (`[SS]…common.utils/…/TransitionDeterminer.java:156-172`, "TODO: Use SimuCom
    Random instead"; `SimulatedLinkingResource.java:53`). The cumulative-sum algorithm is the
    same as RND-5; the stream is different.
  - Every event, user and job gets a `UUID.randomUUID()` id. `Job.hashCode` and `User.hashCode`
    hash it.
  - So **the nightly's runs are not reproducible**: not with branches, and not with same-time ties.
  - The S0 runner's `det` mode makes them reproducible (§4.1):
    - a Java agent seeds `Math.random()` (`java.util.Random(N + 1000003)`) and UUIDs (SplitMix64);
    - it rewrites Slingshot's, QVTo's and OCL's hash-ordered collections to insertion order;
    - one source patch fixes the PS job table.
  - On tie-heavy deterministic models (`x_espresso`), the chosen same-time order changes
    measurements: the state samples differ between the nightly's order and insertion order.
- **Start and stop.**
  - `PreSimulationConfigurationStarted` and `SimulationStarted` are scheduled at 0, and
    `SimulationFinished` at `simTime` (`[CORE]driver/SlingshotSimulationDriver.java:78-90`).
  - `SimulationFinished` was inserted first, so it runs **before** every other event at exactly
    `simTime`. SimuLizar runs the first event ≥ `simTime` and then stops (SIM-6.3).
  - The maximum measurement count is ignored.
  - `ResourceSimulation.onSimulationFinished` clears all job queues (`[RS]ResourceSimulation.java:433-438`).
    There is no drain, unlike SimuLizar's SIM-6.5. Unfinished requests produce no measurements.
- **Resources.**
  - Only FCFS and processor sharing exist. **Delay resources are commented out**
    (`[RS]resources/active/ActiveResourceTable.java:38-61`). A demand on any other kind aborts
    the user.
  - HDD read and write rates are ignored.
  - Resources exist only for containers that appear in an allocation context.
  - PS is a near-literal port of SimuCom's (exact, event-driven, same core distribution and
    epsilon quirks; `ProcessorSharingResource.java:151-260`). Differences:
    - zero demand becomes 1e-9 s;
    - stale completion events are ignored, not cancelled, but still emit state samples, so
      there are **extra utilisation and state samples**;
    - ties are random.
  - FCFS: a single server whatever `numberOfReplicas` says, like SimuLizar (REF-1).
  - Passive resources: FIFO grants; capacity evaluated once; the release amount re-evaluated
    (`SimplePassiveResource.java:82-108`).
  - Linking resources:
    - FCFS with demand `Σ *BYTESIZE / throughput` plus a latency draw;
    - failure retries;
    - no transfer within one container;
    - `findAny` over a `HashMap` when several links match (`[RS]ResourceSimulation.java:383-426`).
- **Usage model.**
  - Open workload: the first user starts at t = 0, then inter-arrivals add up, like WL-2. The
    inter-arrival time is drawn while the current user's `Start` is interpreted, so the draw
    order differs.
  - Closed workload: all users start at 0 without think time; the think time is drawn when the
    next start is scheduled.
  - An aborted closed user restarts; an aborted open user is dropped
    (`UsageSimulationBehavior.java:231-256,428-449`).
  - Usage evolution (DLIM) rewrites the open inter-arrival time every `evolutionStepWidth`. A
    closed-population evolution has no effect: the population is read only at the start
    (`UsageEvolutionBehavior.java:66-99`).
- **SEFF interpretation** (`[SS]…systemsimulation/…/interpreters/SeffInterpreter.java`):
  - An InternalAction with several demands runs them **concurrently**, not one after the other
    as in ACT-3.1. Each finished demand advances the shared cursor, which is a likely bug
    (`:335-373`, `SeffBehaviorWrapper.java:126-134`).
  - Only synchronous forks are handled, and all children share one stack. Asynchronous forks are
    ignored.
  - Guarded branches: the first true condition wins.
  - External calls go over the wire.
  - Component parameters are frames per call.
  - Only system-level connectors are searched (`findFirst`).
  - Composite components are only partly supported.
  - The allocation context is the **first** match in list order (`ResourceSimulation.java:155-157`);
    SimuLizar effectively takes the last (ACT-2.8).
- **Monitoring.**
  - Monitors are interpreted at `PreSimulationConfigurationStarted` and again on `ModelAdjusted`.
  - Calculators record raw values to EDP2. They also emit `MeasurementMade` at delay 0.
  - Processing types (FeedThrough, fixed and variable size aggregation) produce
    `MeasurementUpdated` events. They do not go to the recorder.
  - **Utilisation is an instantaneous sample** (busy cores / cores) at every state change. There
    is **no sliding window**, unlike MEAS-6 (`ActiveResourceProbeTable.java:39-51`).
  - "Number of elements" is measured only on `ModelAdjusted`.
  - **Response times only for usage scenarios and `AssemblyOperationMeasuringPoint`s**
    (`[SS]…systemsimulation.monitor/…/OperationCallActionResponseTimeMonitoringBehavior.java:85`).
    - Monitors on `EntryLevelSystemCallMeasuringPoint` and `SystemOperationMeasuringPoint` are
      silently ignored.
    - S0: `x_ss_minimal` with refsim's monitors gives no rows for them.
- **Comparison with SimOxide so far**:
  - Slingshot matches SimOxide **exactly on the deterministic Slingshot E2E models** (`x_ss_*`)
    and closely on the others;
  - it aborts on acquire/release without a `ParametricResourceDemand`;
  - it fails on synchronous forks (`h11_fork_sync`, `x_pem_fork`) (`docs/performance/comparison.md`).
- Phase S1 wrote all this down as rules `SS-*` (`docs/spec/slingshot/`), each with a source citation
  and, where possible, a probe run; findings in §2.1. Two statements above are refined there: composites
  reached through a connector are entered (but their demands abort, Q30), and the "exact match on
  `x_ss_*`" holds for the deterministic E2E models only.
- **QVTo in Slingshot.**
  - The only QVTo use is the SPD adjustment (`[ADJ]adjustment/qvto/*`), which is a port of
    SimuLizar's QVTo executor and model cache.
  - Slingshot's other extension mechanism is Java behaviour extensions (`@OnEvent`, Guice
    modules), not QVTo.
  - The SimuLizar flavour's reconfiguration rules are the other QVTo extension point (§5.5).

**Exact or semantics mode for Slingshot?** *Decided (U4): both.*

- A **Slingshot-exact mode** compares against the S0 runner's `det` mode, which is a Slingshot
  patched for determinism.
- A **fast mode** keeps only the semantics.

What the exact mode needs, and where S0 left it:

1. **A patched reference.** Done in S0:
   - seeded `Math.random()` and StoEx;
   - a draw tape (`tape.jsonl`, with the event index and, since S1, the draw site);
   - deterministic UUIDs.
2. **A deterministic order of everything that iterates hash-based collections.** Done in S0 for the
   smoke corpus:
   - the agent's ordering rule (insertion order for `HashSet`, `HashMap`, `Collectors.toSet`,
     `Set.of` in Slingshot, QVTo and OCL);
   - patch S0-P2 (PS job table).
   - Extension registration and `getDeclaredMethods` order were stable in all runs.
   - S1 extended the check to the probes and the corpus (SS-X-6); no further hash-dependent order was found.
3. **The same same-time order as Slingshot's bus cascade.** This is the main cost. The exact mode
   does not have to emit every bus event, but it must reproduce the order consequences:
   - result events are scheduled FIFO in insertion order after the events already queued;
   - stale PS events still produce state samples.
   - The comparison runs on a **projection** of the bus trace (user, job, measurement and SPD
     events), plus the tape and the measurements byte for byte. S1 defined the projection (SS-X-2).
4. **Java identity order in QVTo `Set`s (Q8, Q16).** In the `det` oracle these Sets iterate in
   insertion order. In the exact mode qvto-rs therefore needs an **insertion-order policy** for
   `Set`s (phase Q2).
5. **A pinned target.** Done: `reference/slingshot/install.lock` holds the SHA-256 of the 47 jars
   of nightly `1.0.0.202609010045`.

Effort: see §7 (X1, X2, Q2). Exactness on deterministic models (constant demands, no branches,
deterministic arrivals) remains the first milestone of the exact mode.

### 2.1 Findings of S1 (Slingshot semantics specification)

The specification is `docs/spec/slingshot/` (10 pages, rule families `SS-ENG`, `SS-BUS`, `SS-RUN`,
`SS-ND`, `SS-WL`, `SS-ACT`, `SS-RES`, `SS-MEAS`, `SS-RNG`, `SS-SPD`, `SS-X`, quirks `SS-Q1`–`SS-Q37`),
with 20 probe models in `reference/slingshot/models/probes/` (`probes.sh`, `expected/`). What changes
the plan:

- **A throwing handler is disposed and the run goes on** (SS-BUS-7, Q18). One exception (loop count 0,
  no true guard, two demands in one internal action, an acquire without demand, a collection
  iterator, a fork with two branches, an infrastructure call with assembly-operation monitors, an
  unconnected pair of containers) silently freezes that stage for every user; the run "succeeds".
  22 of the 88 verified models have such exceptions; of the 66 corpus models only 38 run without
  handler errors, aborts or empty output (`results/det-verify.csv`). The exact mode must
  emulate per-handler disposal; the fixed default aborts with an error.
- **Exact mode needs `f64` time** (SS-ENG-2): event times are `now + delay` double sums, response
  times their differences (e.g. `0.30000000000000004`). §5.2 corrected.
- **Role lookup by role id alone** (Q20): entry calls go to the first assembly of the component, not
  the delegation connector's; external calls take the first system connector with the required role,
  whatever the caller. Harmless for single assemblies and for SPD's load balancer (new roles per
  replica).
- **Several demands of one internal action** run concurrently and each completion advances the
  cursor (Q17): actions are skipped or overlap, the request can end before its demands, then a
  handler is disposed. Fixed default: sequential demands as SimuLizar (ACT-3.1; U13).
- New semantics to implement in S2 (not bugs): JIFFY jobs for zero PS demands with a demand tuple of
  0; FCFS serves each job for its whole demand from the head (no lost time, no re-scheduling); stale
  PS completions sample state and utilisation (Q26, kept in the exact mode); a link-failure
  `Math.random()` draw on every transfer (Q28); links always simulated with BYTESIZE payloads in both
  directions; probes per specification (replicaID ignored) and passive probes per passive resource;
  assembly-operation monitors keyed by the system role for entry calls (Q33); passive capacity
  without frame (Q27); infrastructure-call inputs ignored (Q36).
- **The event projection** for exact comparison (SS-X-2): level M (measurements, aborts, SPD events,
  stop) and level E (plus passed elements, calls, requests, jobs, grants, user ends), produced from
  the bus trace by `reference/slingshot/tools/project.py`; **draw sites are on the tape** (seed-agent
  hooks in `TransitionDeterminer` and `SimulatedLinkingResource`, stack frames for StoEx), mapped to
  SimOxide's origin purposes (SS-X-4).

## 3. Snapshots in Slingshot (initialisation extension)

`[INIT]README.md:9-52` and the `snapshot` bundles.

**What it is.**

- One "state" is a simulation run that starts from a snapshot and ends at the next snapshot.
  MENTOR explores a state graph.
- Each run starts at t = 0 again. Times in the snapshot are offsets relative to the snapshot
  time (`Camera.java:319-341`).
- Input:
  - the PCM models (copied, so the originals stay unchanged);
  - a `.snapshot` JSON: `{id, pointInTime, snapshot: {events, statevalues}}`;
  - a `.config` JSON: `incomingPolicies`, plus parameters per behaviour (`README.md:75-125`, `353-438`).
- Output:
  - a `.state` JSON: parent id, start time, duration, reasons to leave, utility, outgoing
    policies, measurement sets;
  - the next snapshot;
  - the changed PCM models.

**What a snapshot captures** (`snapshot.data/…/api/Camera.java:348-371`, `PlainSnapshotCamera.java:27-36`):

- All **scheduled events** of the engine (`getScheduledEvents`, a `HashSet`), minus aborts.
  The scheduled times are rebased:
  - think times of closed users are shortened to the remaining time;
  - inter-arrival delays likewise (`:293-317`).
- **Jobs at resources** as new `JobInitiated` events. They are recorded by
  `SnapshotRecordingBehavior` from `JobInitiated` pre/post interception
  (`SnapshotRecordingBehavior.java:141-167`):
  - FCFS: remaining demand from the scheduled `JobProgressed`/`JobFinished` time,
    de-normalised by the rate (`Camera.java:221-248`);
  - PS: requested demand × current/normalised demand (`:188-201`);
  - before the snapshot, a zero-demand "fake" job forces each PS resource to update its jobs'
    progress (`SnapshotRecordingBehavior.java:239-267`).
- **Open response-time calculators**: the start events of unfinished scenarios and SEFFs, with
  negative time offsets. Response times that span the snapshot are measured correctly
  (`InMemoryRecorder.java:48-105`, `SnapshotInitFromBehaviour.java:212-235`).
- **SPD adjustor state** for each policy:
  - cooldown end, number of scales in the cooldown, latest adjustment, number of scales;
  - the last enacted policy and time of the target group.
  - Times are shifted (`Camera.java:117-145`) and restored (`SnapshotInitFromBehaviour.java:129-154`).
  - This needs a MENTOR-only branch of the SPD interpreter (`SPDAdjustorStateRegistered`). It is
    not in Slingshot master (`README.md:618-655`).
- `ModelAdjustmentRequested` events pending at the snapshot time (`Camera.java:159-176`).

**What it does not capture:**

- RNG state. The resumed run reseeds.
- Monitor sliding windows and aggregation buffers.
- **SPD trigger aggregators** (the EMA, the 10-value windows). Every policy starts empty again:
  an AVERAGE policy cannot fire in the first 60 s of each state (§1.2).
- Passive-resource holders and wait queues. Only active and linking jobs are recorded. Not
  verified: this needs the MENTOR application with the initialisation extension (U7), which S0
  does not install.
- Event identity and ordering among same-time events. They come from a `HashSet`.

**When a snapshot is taken:**

- `SnapshotInitiated` with the maximum state duration is scheduled at the start
  (`SnapshotInitFromBehaviour.java:102-106`).
- `SnapshotTriggeringBehavior` takes one when a policy's trigger fires: it pre-intercepts
  `ModelAdjustmentRequested`.
  - The adjustment is not applied in this run. It is carried into the snapshot, so the next
    state starts with it.
  - Scale-ins on a minimal configuration can be dropped (`doDrop`).
- `SnapshotSLOTriggeringBehavior` takes one when a measurement comes close to an SLO threshold.
- `SnapshotSLOAbortionBehavior` and `SnapshotAbortionBehavior` abort the run.
- Taking the snapshot publishes `SimulationFinished` (`SnapshotStateUpdateBehaviour.java:130`).

**How a resumed run differs from an uninterrupted one:**

- The time base restarts at 0.
- Random streams differ.
- Windows and SPD aggregators restart.
- Same-time event order may change.
- At the start, "incoming policies" are **enforced**: their `ModelAdjustmentRequested` events are
  scheduled directly, past triggers and constraints (`SnapshotInitFromBehaviour.java:118-127`,
  `SnapshotInitialAdjustmentBehaviour.java`).

So in Slingshot, resume ≠ uninterrupted, even without changes.

The N3a spike corrected and extended this section from the code and real snapshot files (5.7,
"N3a spike": what is not written, what is restored late, what Slingshot cannot read back).

## 4. Oracles and validation

### 4.1 Slingshot reference runner (S0, implemented)

`reference/slingshot/`; documentation in `docs/reference-simulator/slingshot.md`.

**How it runs.**
- The installed nightly is used read-only: `old/palladio-research/work-slingshot/sl-install`,
  pinned by `install.lock`. `install.sh` makes a fresh install.
- Each run has a private Equinox configuration, generated by `setup.sh` from the install's
  `bundles.info`:
  - minus the old bench bundle;
  - plus the runner's bundles;
  - with patched jars in `det` mode.
- `run.sh` locks a configuration slot per JVM.
- Speed (niced, shared VM): a run takes 2.3-3.4 s in a fresh JVM (3.3-4.5 s for the whole process
  with Equinox start-up), and 0.17-0.45 s warm.

**Earlier attempts (design phase)** showed why it is set up this way:
- On a flat classpath, QVTo cannot resolve `MainTransformation`'s imports.
- In Equinox without a stub, `ProfilesLibrary` is missing. **The shipped nightly cannot run any
  SPD adjustment.**
- The stub bundle `org.palladiosimulator.architecturaltemplates.catalog` (`src/stub`) registers
  the blackbox; the main path never calls it.
- `default.slo` is skipped, because the SLO metamodel is not installed.

**Bundles and agent:**
- `src/runner`: the application. It handles seed, stop time, warm repeats, EDP2 → `measurements.csv`
  in SimOxide's format, and `-saveModels` for the adjusted models.
- `src/instrument`: a behaviour extension, active only when instrumentation is on. It writes:
  - `trace.jsonl`: every bus event in dispatch order, with stable keys (PCM ids; users and jobs
    numbered; aliases for ids that QVTo creates; no UUIDs);
  - `tape.jsonl`: every StoEx and `Math.random()` draw, with the event index;
  - `spd.jsonl`: per policy evaluation, each filter's result. The filter chain entries are replaced
    in place by recording proxies, through the bus's interceptors. Also the adjustments with model
    summaries before and after, and the QVTo log.
- `src/seed-agent` (since M8 `reference/oraclekit/seed-agent`, shared with refsim and the QVTo
  runner): a Java agent that does three things:
  - seeds `Math.random()` (a reseedable `java.util.Random` subclass);
  - seeds UUIDs (a seedable `SecureRandom` provider);
  - in `det` mode, rewrites hash-ordered collections to insertion order (ASM, at class-load time:
    Slingshot, plus QVTo and OCL).
- `patches/`: source patch S0-P2, the `ProcessorSharingResource` job table → `LinkedHashMap`.
  `setup.sh` checks the upstream copy against the installed class.

**Modes:**
- `det` (default): reproducible.
- `stock`: the nightly; `-seed` seeds only the random sources.
- Both are switchable per run.
- The `det` changes are minimal and documented: the ordering rule and one class. They are the
  basis of the Slingshot-exact oracle (U4).

**Smoke corpus** (`smoke.sh`, `results/smoke.csv`): `spd_minimal`, `spd_scale_in`, `x_ss_minimal`,
`x_ss_minimalexample`, `x_ss_minimalexample_open_ps`, `x_ss_delay`, `x_espresso`, `x_ss_mediastore`,
`x_sl_mediastore`.
- `det` is byte-identical in two fresh JVMs and in warm runs, for trace, tape, SPD log and
  measurements.
- The instrumentation does not change the measurements.
- In `stock`, the trace differs between runs.
- In `x_espresso`, the state samples differ between `stock` and `det` (the tie order).

**The SPD minimal example** (`models/spd_minimal`, from Palladio-Documentation-Slingshot):
- at t = 50, "RECONFIGURATION WAS true";
- `AppServer_1` is created, and with it `EspressoAccountingComponent_AssemblyContext_2` and a
  second load-balancer branch;
- `CPU [0] in AppServer_1` gets load;
- branch draws begin after the scale-out.
- Its `default.experiments` stops at 10 s; the runner takes `-simTime`.

**Open:**
- usage evolution (U9).
- Done in S1: the trace projection (`tools/project.py`, SS-X-2); draw sites on the tape (seed-agent
  hooks, no source patch needed; SS-X-4); `det` verification beyond the smoke corpus
  (`verify-det.sh`, `results/det-verify.csv`, SS-X-6); the subscriber order per event (`bus.jsonl`).

**X1 as built** (2026-10-17, branch `x2`; `docs/spec/slingshot/exact.md` SS-X-8):

- **Corpus `corpus-ss/`.** 88 cases, each `case.json` (model directory, seed, simulated time,
  group) and `expected/` with the `det` outputs: measurements, the level-E projection (level M is
  its subset and is not stored), the tape projection, the handler exceptions (`errors.txt`), the
  summary, the SPD log and the full bus trace (`trace.jsonl.gz`, debugging only). Groups: the 20
  probes, the SPD examples (`spd_minimal`, `spd_scale_in`, `spd_time_policies`), the 38 corpus
  models without failures, 25 failure models (all corpus models with handler errors, aborts or
  empty output: the 22 handler-error cases of the corpus and the probes are all in), and the two
  runs that end before t = 0 (`error.txt`). Seeds and simulated times from
  `results/det-verify.csv`; `x_pem_fork` (zero-time livelock) has no reference. About 14 MB of
  text. Tool: `reference/slingshot/corpus-ss.sh [--check]`.
- **Reproducibility.** `corpus-ss.sh --check` reproduced every case byte for byte in fresh JVMs.
  The only varying bytes were identity hashes inside three exception messages (Q31's
  `SEFFInterpretationContext@…`); `tools/errors.py` normalises them.
- **Subscriber audit.** `tools/audit_subscribers.py` parses the pinned Slingshot sources: 73
  `@Subscribe` methods in 25 classes; no class has two subscribers that can receive the same event
  (parameter types with a common subtype among the event classes and overlapping reified types);
  the three classes with overlapping parameter types reify to disjoint types. The bus itself
  refuses equal (type, reified) pairs in one class. So `getDeclaredMethods()` order cannot change
  a run (SS-BUS-2).
- **SPD corpus in `det`.** The three SPD example models run in `det` as part of `corpus-ss/` (1, 2
  and 8 adjustments, decision records in `spd.jsonl`); P2's `corpus-spd/` holds them and
  `spd_service_group` (`CORPUS=corpus-spd corpus-ss.sh`, 5.4 "P2 as built").

### 4.2 Validation levels

| Level | What | Oracle |
|---|---|---|
| V1 unit | filters, constraints, aggregators, comparator, adjustment arithmetic: edge cases from §1.2 (`==` at `coolDownEnd`, `t % delta == 0`, EMA emit rule, 10-value window) | hand-computed, from the Java code |
| V2 QVTo | every adjustment path (EI step ±, relative, absolute, size clamp, service group top-down and bottom-up, competing consumers, scale-in) on SimOxide-loaded models | qvto-rs Java runner, strict. *Built in Q1* (5.5, "Q1 as built"): suite `spd` of `qvto conform`, 47 cases on qvto-emf-loaded models (SimOxide-loaded with P2/F3) |
| V3 deterministic runs | SimulationTime triggers, constant demands, deterministic arrivals: same adjustment times, container and assembly names, model after each step, response times | Slingshot reference runner: exact on these values |
| V3x Slingshot-exact | any model the `det` runner reproduces: level-E trace projection, tape and measurements byte-equal, also on models with handler errors (U14) | S0 runner, `det` mode, same seed |
| V4 statistical runs | utilisation, response-time and task-count triggers on stochastic models: distribution of scale-event times, container-count trajectory, mean response time per window, over 30 or more seeds | Slingshot reference runner: confidence-interval overlap of per-run summaries and two-sample KS tests |
| V5 SimuLizar flavour | QVTo rule runs in SimuLizar mode | refsim with deterministic UUIDs: byte-exact (plan step 6) |
| V6 snapshots | resume equals uninterrupted run (§5.7) | SimOxide itself |

Slingshot's E2E tests contain **no SPD tests**. The SPD models there are empty or never loaded
(`Palladio-Analyzer-Slingshot-E2E-Tests`: `TestModelURIs.java:35-37,71`; `minimalexample/README.md:10-11`).
They only provide the non-SPD behaviours, already used as `x_ss_*`. The SPD corpus has to be
built: `corpus-spd/` (phase P2).

## 5. Architecture in SimOxide

### 5.1 Layers

The QVTo engine moves into this repository (user decision; merge design in
`crates/qvto/docs/merge-into-simoxide.md`, written separately). The design below assumes:

- QVTo crates under `crates/qvto/*`, as workspace crates;
- foundation crates shared by QVTo and SimOxide: Java compatibility (hashing, number formatting,
  `HashMap` order), and the EMF registry, object graph and XMI;
- QVTo running **directly on the shared graph**. The Q6 adapter (`qvto-simoxide`, two homes,
  mirrors) is only a bridge until then.

Names marked *foundation* follow the merge design.

**Licensing structure (decided):**
- Code for exact or compat mode that is derived from GPL sources lives in **separate opt-in crates
  or features**. Examples: qvto-runtime's port of OpenJDK's `HashMap`; SimOxide's existing
  `hotspot-math` feature.
- The default (fast) build stays EPL-only.
- The Slingshot-exact mode needs `java.util.Random`'s LCG for the `Math.random()` stream. Its
  algorithm and constants are given in the API specification, so it can be written from the
  specification, without GPL code.
- Embedded SPD `.qvto` and `.ecore` files carry their source repository and revision, and a
  "licence pending confirmation" note (U3).

```
foundation         (shared with QVTo) EMF registry + object graph + XMI, Java compat
simoxide-model     PCM loader on the foundation graph, typed PCM, + SPD/semanticspd packages (5.6)
simoxide-stoex, simoxide-random, simoxide-sched      shared as is
simoxide-sim
  ir.rs            CompiledModel (immutable, shared by runs)               shared
  code.rs, flat    behaviour instructions                                  shared
  sim/core         interpreter of SEFF/usage instructions, resources,      shared, generic over F: Flavour
                   frames, calls, measurement sink, limits
  sim/simulizar    DESMO-J process model, stop/drain, MEAS-6/7 windows,    SimuLizar flavour
                   PRM + reconfiguration process
  sim/slingshot    Slingshot timing rules (SS-*), its monitors and         Slingshot flavour
                   windows, stop without drain, abort-on-removed
  run.rs           RunState: everything mutable of one run (5.7)           shared
crates/qvto/*      the QVTo engine (in-repo workspace crates)              shared
simoxide-reconf    (new, feature `reconfiguration`) RunModel overlay,      shared
                   QVTo host on the shared graph, journal -> syncers
simoxide-spd       (new, feature `spd`) SPD model view, trigger/filter     shared by both flavours
                   interpreter, adjustment via MainTransformation
simoxide-snapshot  (new, feature `snapshot`) serde of RunState,            shared
                   Slingshot .snapshot import/export
```

`Simulation<'m, C: Compat, F: Flavour = SimuLizar>`:

- `C` stays what it is: the exact or fast RNG policy.
- `F` is a new trait whose hooks are monomorphized like `C`. The SimuLizar flavour's hooks are
  today's code; they cost nothing and must keep the golden hashes unchanged.

### 5.2 Where the flavours differ (the `Flavour` hooks)

| Concern | SimuLizar flavour | Slingshot flavour | Shape of the hook |
|---|---|---|---|
| time, event list | i64 ns, FIFO `(t, seq)` (SIM-2, SIM-3) | the same list; **`f64` time**, `t = now + delay` as SSJ (SS-ENG-2), in both modes (S2 as built: planned for the exact mode only, the fast mode needs it for exactness too) | `F::Clock` (`Nanos`, `F64Secs`) |
| process model | DESMO-J processes, `Resume` notes, elided hand-offs | the same machinery; Slingshot's same-instant order only where SS-* says it changes results | a few ordering hooks |
| branch draws | the reference stream (exact) or fast | exact: `java.util.Random(seed + 1000003)`, as the S0 runner seeds `Math.random()`; fast: any stream | `F::branch_rng` |
| StoEx draws | as now | exact: MT19937 `{seed..seed+5}` (S0 `-stoex mt`, the SimuLizar stream); fast: fast RNG | `C` |
| resources | SimuLizar schedulers (`simoxide-sched`) | PS as SimuCom's plus zero-demand jiffy; FCFS from the head without lost time; no HDD rates; delay resources as SimuLizar. Compat: stale-event samples (Q26), no delay resources (Q25) | scheduler variant enum |
| actions | ACT-* | first allocation match, links always simulated (design); otherwise ACT-* where Slingshot has a bug: sequential demands, async forks run, loop count 0 skips. Compat: concurrent demands with a cursor step per completion (Q17), async forks ignored (Q23), … | hooks in the action interpreter |
| usage | WL-* | Slingshot's draw order for inter-arrival and think times; aborted closed users restart; usage evolution later (decision U9) | hook per workload kind |
| monitoring | probe framework, MEAS-6 windows, PRM | instantaneous utilisation samples, no windows; processing types not recorded; `MeasurementMade` stream; ELSC, system-operation and external-call monitors honoured. Compat: ignored (Q29) | `F::Monitors` |
| stop | stop conditions, post-stop drain (SIM-6) | `SimulationFinished` at `simTime` before other events at that time; queues cleared; maximum measurement count honoured. Compat: ignored (Q32) | `F::stop` |
| removed container | keeps simulating (SimuLizar gap); "fixed" option drains | drain (default); compat: one job aborted, the rest lost (Q5) | syncer policy |
| reconfiguration trigger | PRM writes → reconfiguration process (MEAS-7.2) | SPD filter chain on measurements and time events | `F::Reconf` |
| exactness | byte-exact with `C = Exact` | exact mode: byte-exact against the S0 `det` runner (level-E trace projection, tape, measurements; U14); fast mode: statistical | `C` |
| handler errors | run error | fixed: run error at the event (Q18); exact mode: the throwing Slingshot handler is disposed, the run goes on | handler table with a disposed bit per handler (exact only) |

The fast mode does **not** emulate Slingshot's bus events. It emulates the timing and ordering rules
that change results (S1). The exact mode targets level E (U14), so it reproduces the order of every
projected step: it runs a FIFO queue of **handler steps**, one per Slingshot handler that can produce
a projected line, a draw or a measurement (SS-X-3.1). Internal events with nothing projected between
them may be merged. Handler disposal (Q18) needs these steps to be the same units as Slingshot's
handlers, with Slingshot's side effects up to the throwing statement (SS-X-3.6).

**S2 as built** (2026-10-17, branch `s2`; guide `docs/guide/slingshot.md`, evidence
`docs/correctness/slingshot.md`):

- **Shape.** `Simulation<'m, C: Compat, F: Flavour = SimuLizar>`, `RunState<C, F>`,
  `Detached<C, F>`. `Flavour` (`crates/simoxide-sim/src/flavour.rs`) has `KIND`, `SLINGSHOT` (a
  `const bool`) and `type Clock`; the hooks are `if F::SLINGSHOT` branches in the shared
  interpreter plus the Slingshot-only steps in `sim/slingshot.rs`. The SimuLizar instantiation
  contains none of them. The flavour is chosen at compile time of the model:
  `CompiledModel::compile_flavour(model, FlavourConfig)` (Slingshot tables in `cm.ss`, the
  flavour in `cm.flavour`); `simoxide_sim::run`/`run_batch` dispatch on mode x flavour (four
  monomorphizations). CLI `--flavour slingshot --quirks LIST`, also in `simoxide-bench suite/batch`.
- **Time (a design change: also the fast mode uses double time).** `simoxide_sched::Clock` with
  `Nanos` (SimuLizar) and `F64Secs` (Slingshot: the IEEE bit pattern of the seconds, which orders
  like the value, so the event list, the timer slots and every `i64` time field stay as they
  are). The schedulers are generic over the clock. With integer nanoseconds the fast mode could
  not be exact on any `x_ss` model (`1.7362678682860686` vs `1.736267868`); with `F64Secs` it
  costs nothing and X2 needs it anyway. APIs convert at the edges (`RunState::now_ns`,
  `SimError::at_ns`, `Pause`, `RunResult::end_ns`, `schedule_external`, `take_observed`).
- **S2a.** Trait, SimuLizar impl, clock, `FcfsHead` (Slingshot's FCFS: served for its demand
  from the head, no rescheduling). Golden identical; speed exact -0.5 %, fast +1.8 % (geomean).
- **S2b (usage, stop, RNG).** `SimulationFinished` at `simTime` before every event at `simTime`
  (checked before each pop), no drain, no final samples, end time `simTime`; the maximum
  measurement count stops the run (fixed Q32) or is ignored (Q32 sets it to -1; the livelock
  guard stays). Branch draws and link failure checks from `java.util.Random(seed + 1000003)`
  (`javarand.rs`, from the API specification, EPL) in the exact RNG mode, from the fast stream
  in the fast mode; tape records `{"k":"m",…}`. Think times and usage delays are plain events.
  Aborted users (no allocation, no resource of the type, Q25): closed ones are replaced by a new
  process that thinks first, open ones are dropped. Usage loops (Q19).
- **S2c (resources).** Resources for every allocated container in allocation order (nested ones
  too), a later specification of a type replaces the earlier one, HDD rates ignored, delay
  resources as SimuLizar's (fixed Q25) or none (Q25); first allocation match (Q30: inner
  assembly contexts unallocated); PS as SimuCom's on double time (zero demands become 1e-9 s
  jobs); stale PS completions as `Ev::Stale` notes at the superseded key (Q26,
  `EventList::set_timer_keep_stale`); links always simulated with the BYTESIZE payload, no
  transfer within a container or to an unallocated side, the failure draw (Q28: always and the
  stall), the route fallback (Q37); passive resources created before the start, capacities in
  the component frame (Q27: none), amounts of the action's first demand (Q24), the release
  re-evaluated.
- **S2d (monitors).** `SsTables`: active-resource probes per specification (state, utilisation,
  demand; per core or totals, Q34), passive probes per (passive resource, assembly context) or
  merged (Q34) with Slingshot's `AssemblyPassiveResourceMeasuringPoint[ac|pr]` keys; samples at
  every arrival, completion and stale completion; demand measured also when 0; no windows, PRM,
  initial or final tuples; Q29 drops entry-level-call, system-operation and external-call
  points; Q31, Q33. The SimuLizar compiler parts that build windows, PRM, reconfiguration-time
  and container-count series read an empty monitor list in the Slingshot flavour.
- **Actions.** SimuLizar's semantics (fixed) with the flags Q19-Q24, Q30, Q31, Q35-Q37 as listed in
  `docs/spec/slingshot/quirks.md#compat-flags`; Q17 and Q18 are refused (exact mode). A
  `Cont::Stall` continuation is a request that stops silently (Q35, Q30).
- **Measurement observer.** Merged with P1's hooks (`sim/hooks.rs`, made flavour-generic): the
  Slingshot flavour's tuples are Slingshot's `MeasurementMade` stream; `take_observed` reports
  nanoseconds.
- **Exit criterion, sharpened.** "Exact on deterministic models" is ill-defined for `x_ss_*`:
  none of them is free of random draws (DoublePDF think times, branches). It is read as: with the
  exact RNG mode (the runner's seeding) and the compat flags, byte-identical `measurements.csv`
  to `det` for the same seed. Met for all five non-espresso `x_ss` models with seeds 1-3 (the
  flags each needs: Q29; Q29; Q26+Q29; Q26+Q29+Q34; none), and beyond: 16 of 20 probes (the
  other four have handler errors), 38 of 45 corpus models (3 more equal to 1.5e-8, sampler
  numerics of Palladio 6.0; 2 differ in the draw order of several users at one instant; h21;
  `x_espresso`). "Statistically equal" is read as: no Holm-corrected rejection (α 0.01) of
  per-run counts, means and pooled values against stock Slingshot with 30 seeds, for series
  whose value does not depend on the order of simultaneous events. `x_ss_mediastore`: 0 of 26 in
  both RNG modes. `x_espresso` (30 users, every event a tie): response times, demands and the
  time-weighted state equal; the event-sampled state series differs (sample mean stock 22.7,
  `det` 21.1, fast mode 28.1) because the fast mode's processes return after two notes where
  Slingshot's cascade needs about 13 generations. `det` itself is outside the stock range, so the
  statistic measures the same-instant order, which is X2's job. The fixed behaviour differs as
  documented (mediastore: 41 of 55 tests, all Q26/Q29/Q34 effects).
- **Not done / deviations** (`docs/correctness/deviations.md#slingshot-flavour`): same-time and
  same-instant draw order; throws are run errors; link-failure retries keep the payload; model
  changes refused for Slingshot runs (`ChangeError::Unsupported`); SPD in the Slingshot flavour
  refused by the CLI (P3); several SEFFs per signature are an error; a negative FCFS demand is a
  run error (Slingshot throws; `gen_s6` hits it).
- **Gates** (after merging `spd` with P1). `golden.sh` identical (312 runs) at every step;
  `cargo test --release --workspace` 304 passed; `simoxide-fuzz corpus` 132/132, corpus-fuzz
  6/6; fmt and clippy `-D warnings` clean (workspace, and sim/sched/cli without default
  features). Speed of the SimuLizar flavour against `spd` 2386af5 (`simoxide-bench suite`, 20
  alternating rounds, pinned to one core, load about 5-7): geomean exact +0.2 % (models -2.9 % to
  +3.8 %), fast +2.7 % (-1.3 % to +6.5 %). Slingshot flavour (fixed behaviour) against the
  SimuLizar flavour, finished requests per second, geomean of 11 models: exact x1.03, fast x0.99;
  `x_ss_mediastore` 200 000 s in 0.25 ms, `x_espresso` 100 s in 0.31 ms (Slingshot warm: 418 ms
  and 271 ms, smoke run on a shared VM).
- **What X2 needs.** The handler-step queue on top of the same `F64Secs` clock and event list
  (`Ev` gets handler-step kinds; `Ev::Stale` already keeps the superseded key); the per-handler
  disposal for Q17/Q18 and the throwing quirks (their throw sites are the run errors of the fast
  mode, `slingshot.rs` constants); the draw order of SS-RNG-2.1 (rate one event after the
  demand, all demands of an action at its start, infrastructure counts up front, INNER once);
  Palladio 6.0's Gamma/Lognorm samplers (1e-8 differences); the level-E projection writer; the
  `math` stream records already carry the draw site.
- **What P3 needs.** `SpdRun` generic over `F` (today `Simulation<C>`): the Slingshot flavour's
  observed tuples are instantaneous utilisation samples at every resource event (no windows);
  passive and per-core series exist from the start (`cm.series`); one quirk set for both layers
  (`simoxide_sim::Quirks` has Q1-Q16 with the same numbers as `simoxide_spd::Quirks`); model
  changes for Slingshot runs (F4: `ss` tables, `res_mon` per new resource, first-match
  allocation in `change.rs`, removal policy Q5 = abort job / lose others).

**X2 as built** (2026-10-17, branch `x2`; guide `docs/guide/slingshot.md`, evidence
`docs/correctness/slingshot.md`, corpus `corpus-ss/`, SS-X-8):

- **X2a: the engine (handler steps, event order, time).** A separate engine
  `crates/simoxide-sim/src/ssx/` behind a third flavour kind, `FlavourKind::SlingshotExact`
  (`--flavour slingshot-exact`; `FlavourConfig::slingshot_exact()` = all 37 quirks;
  `simoxide_sim::run` dispatches to `ssx::run` in every build, the RNG mode is not read). The
  design choice: not hooks in the process interpreter but a literal emulation of Slingshot's bus,
  because level E needs the order of every projected step, and Slingshot's same-instant order is
  a breadth-first tree of handler results (one handler often returns two or three events whose
  chains interleave with every other user's), which a process model cannot express without
  replaying the same structure. So:
  - one `Ev` variant per Slingshot event that has a subscriber or a projected line (37);
    bookkeeping events with neither (`UserSlept`, `ProbeTaken`, `UsageScenarioStarted/Finished`,
    `MeasurementUpdated`, `CalculatorRegistered`, the monitor set-up events, and resource-state
    events of resources without a monitor) are not created, which changes no order;
  - an event list keyed by `(f64 time, insertion)` (SS-ENG-3), results queued at `now + delay`
    in the handler's insertion order after it returns, `MeasurementMade` queued directly when a
    probe fires (SS-BUS-4/5), `SimulationFinished` inserted third at t = 0 (SS-RUN-3);
  - one function per subscriber method (38, enum `H`), called in subscription order (SS-BUS-2:
    e.g. the passive monitor before `SeffSimulationBehavior` on `PassiveResourceAcquired`),
    written statement by statement after the Java code, with Slingshot's entities as
    reference-counted objects (`User` with its SimuCom stack, `UserInterpretationContext`,
    `SEFFInterpretationContext`, the behaviour-context holders with their cursors and the fork
    holder's rotating `ArrayList` iterator, `GeneralEntryRequest`, `CallOverWireRequest`,
    `ResourceDemandRequest`, jobs);
  - SimuCom stack frames with Java reference semantics (`ssx/frame.rs`: shared mutable frames;
    `copyFrame` snapshots for `INNER` proxies are the process interpreter's immutable frames, so
    proxy evaluation is shared code);
  - **handler disposal** (Q18): every handler returns `Result<results, Throw>`; side effects up to
    the throw (draws, pushed and popped frames, cursor moves, queue changes, measurements and
    their `MeasurementMade`) stay, the results are dropped, the handler's disposed bit is set and
    never cleared; the exceptions are listed in `RunResult::warnings`;
  - the two random streams: StoEx from SimuCom's MT19937 `{N..N+5}` (simoxide-random, EPL),
    `Math.random()` from `javarand.rs` (EPL); the tape projection records every uniform with its
    site purpose and the position `p` (SS-X-4) as the reference's `project.py` does;
  - the level-E projection writer (users and jobs numbered by first appearance) on
    `Outputs::trace`, the tape projection on `Outputs::tape`.
  Shared code: the compiled model (`CompiledModel::compile_flavour` builds the Slingshot tables
  for both Slingshot kinds; `ir.rs` changed in one line, `is_slingshot()`), the StoEx programs and
  evaluator, the frame bindings and proxies, `ProcessorSharing` and `FcfsHead` of
  `simoxide-sched` on the `F64Secs` clock (a stale PS completion is a wake-up of an older
  generation: Slingshot's `currentState` check), the measurement sink and CSV writer, the Java
  number formatting. Nothing of the SimuLizar flavour or of the fast Slingshot flavour changed
  (golden hashes identical).
  - First run against `corpus-ss/` (`crates/simoxide-sim/tests/slingshot_exact.rs`,
    `reference/slingshot/tools/compare_exact.sh`): **75 of 88 cases byte-equal** at level E
    (projection, tape projection with positions, measurements), among them all 20 probes, 33 of
    38 clean corpus models, 20 of the 25 failure models (18 of the 22 handler-error cases; the
    disposal and partial side effects of Q17, Q19, Q21-Q24, Q30, Q31 match) and both runs that end
    before t = 0. The other 13: Palladio 6.0's samplers (UniDouble, Gamma, Lognorm,
    LognormMoments, Norm: 9 cases, see X2b), the `INNER` evaluation order of a
    `java.util.HashMap` (h27, licence-bound, below), the three SPD examples (X2d).
  - Speed (indicative, shared VM): `x_ss_mediastore` 200 000 s (15 735 Slingshot events) in about
    0.9 ms against 0.16 ms for the fast mode and about 420 ms for Slingshot warm.
  - Gates: `golden.sh` identical (312 runs); `cargo test --release --workspace --features exact`
    548 passed (546 + 2); `simoxide-fuzz corpus` 132/132, corpus-fuzz 6/6; fmt, clippy
    `-D warnings` clean (default and `exact`); release check: no opt-in crate in `simoxide-cli`
    or `qvto-cli`.
  - Shared files touched: `flavour.rs` (the kind), `ir.rs` (one line; `metric_id`, `ss_mp_key`,
    `action_type`, `uaction_type` now `pub(crate)`), `lib.rs` (module, dispatch),
    `simoxide-cli/src/main.rs` (the flavour name, the SPD refusal for both Slingshot kinds).

- **X2b: resources, monitoring and the samplers.** The resources and monitors of X2a were already
  equal on every case without other causes (PS with stale-completion samples and the 1e-9 s jiffy,
  FCFS from the head, links with the failure draw, passive resources with re-evaluated releases,
  state/utilisation/demand probes per specification, passive probes per passive resource,
  usage-scenario and assembly-operation calculators with Slingshot's request contexts and the
  "last monitor of a key wins" maps). What X2b adds are **Palladio 6.0's samplers**, the gap S2
  had left open ("Gamma/Lognorm 1e-8 differences"): the nightly runs on Palladio 6.0, whose
  probability-function library wraps Commons Math 3.6.1 instead of SimuLizar 5.2.2's Commons Math
  2.1. New module `simoxide-random/src/p6/` (EPL Palladio wrappers + Apache-2.0 Commons Math 3,
  `LICENSES` §1, §2; ported from the 3.6.1 classes of the install with their operation order):
  `FastMath.log/log1p/exp` with their tables, `Gamma` (logGamma, logGamma1p, invGamma1pm1,
  regularized P/Q), `Erf.erfInv`, `ContinuedFraction`, `Precision.equals`, `BrentSolver` and the
  inverse CDFs of `AbstractRealDistribution` (Chebyshev bounds) and
  `AbstractIntegerDistribution` (bisection); the distribution functions `Exp` (FastMath.log),
  `Norm` (erfInv), `Lognorm`/`LognormMoments` (Palladio's own: `Math.exp` of the normal inverse,
  0 at u = 0), `Gamma`/`GammaMoments`, `UniDouble` (Palladio's CDF, generic inversion), `UniInt`
  (Palladio's CDF with its negative variance), `Pois` (no "−1" of 5.2.2). The exact mode's random
  source overrides these hooks of `UniformSource`; the SimuLizar flavour is untouched.
  Result: **86 of 90** cases equal in the test build (h04, h05, h06, h21, the three screencast
  models and both simple-optimisation models now equal), 83 of 90 in the default EPL build: there
  `java.lang.Math.exp`/`log` (Lognormal's `Math.exp`, StoEx `Log`) are correctly rounded instead
  of HotSpot's intrinsics, and one sample of h21 (of 406 draws) differs in the last bit
  (`hotspot-math` is GPL, so the EPL build stays correctly rounded: a documented, licence-bound
  deviation). Also two new cases for Q37 (`reference/slingshot/models/x2/ss_q37_*`, `det`
  reproduced): the "link touching either container" fallback and the `IllegalArgumentException`
  that disposes `onCallOverWireRequested` (no corpus model had them); both equal.

- **X2c: handler disposal and the throwing quirks.** The disposal mechanism is X2a's (a disposed
  bit per subscriber method, side effects up to the throw kept); X2c checks it quirk by quirk. Each
  throw site follows the Java statement order, e.g. Q19 (SEFF loop count 0: the body runs once,
  the wrapper's `next()` throws in `onSeffInterpretationProgressed`; usage loop: the builder throws
  after the count was drawn and, in `onUserStarted`, after the frame was pushed), Q21 (guards
  evaluated until none holds), Q22 (the `INNER` frame stays, `onUserRequestFinished` fails its
  two-frame precondition), Q23 (the fork holder's `ArrayList` iterator rotates on every
  `getCurrentProcessedBehavior()` call, also in `onActiveResourceFinished` and when a child
  holder is built, and throws `NoSuchElementException` when it runs dry), Q24, Q30 (the inner
  assembly's demand aborts, `findUserInterpretationContext` finds no user), Q31
  (`AssemblyOperationCompoundKey.of` throws for every root SEFF reached by an infrastructure call,
  monitored or not, so both monitor handlers go), Q17 (each completion of a concurrent demand
  advances the shared cursor; the completion after the `Stop` throws the
  `getPredecessor_AbstractAction()` NPE), Q37. New test `handler_errors_match_the_reference`: on
  every corpus case with handler errors (21 of the 22, h27 excepted, plus `ss_q37_none`) the
  exception classes and their order equal the reference's `errors.txt`. Docs:
  `docs/spec/slingshot/quirks.md#exact-mode`, SS-BUS-7.

- **X2e: level-E projection and corpus debugging.** The projection writer is X2a's (lines at
  dispatch, users and jobs numbered by first appearance in the projection, numbers in Java's
  `Double.toString`, keys in `project.py`'s order and only when present); `ssx::level_m` filters
  level M from it; `RunResult::main_count` counts the finished usage-scenario runs. Corpus
  debugging took few rounds because the handlers follow the Java code: the only engine bug found
  on the corpus was the job of a processor-sharing completion (the shortest job, not the arriving
  one); everything else were the samplers (X2b). **Final V3x status** (test build, `hotspot-math`
  on; default EPL build in brackets):

  | Group | Cases | Byte-equal (projection, tape with positions, measurements) |
  |---|---|---|
  | probes | 20 | 20 (20) |
  | clean corpus models | 38 | 38 (37: h21, one `Math.exp` sample) |
  | failure models (handler errors, aborts, no output) | 25 | 24 (24): all but h27 |
  | of these, and of the probes, with handler errors | 22 | 21 (21): all but h27 |
  | runs that fail before t = 0 | 2 | 2 (2) |
  | Q37 cases (X2) | 2 | 2 (2) |
  | SPD examples | 3 | 3 (3) after X2d (0 before) |
  | **total** | **90** | **89 (88)** after X2d (86 (85) before) |

  Remaining differences and their causes: h27 (`java.util.HashMap` iteration order of SimuCom's
  frames for two `INNER` keys, defined only by OpenJDK's sources; SS-X-9), h21 in the EPL build
  (HotSpot's `Math.exp` intrinsic, GPL; SS-X-9), the SPD examples (X2d).
  Speed (indicative, one core of the shared VM, `simoxide bench`, 20 runs): `x_ss_mediastore`
  200 000 s 0.83 ms per run (15 617 Slingshot events; fast mode 0.12 ms), `x_espresso` 100 s
  1.5 ms (29 151 events; fast 0.21 ms), `x_ss_minimalexample_open_ps` 1 000 s 3.5 ms,
  `h14_call_chain_3` 1 000 s 22 ms (387 328 events, about 17 million Slingshot events per
  second): 4 to 8 times the fast mode, two to three orders of magnitude faster than Slingshot. The
  SimuLizar flavour against the base 9e62855 (requests per second, 5 models, median of 3 x 10
  runs, pinned, with R1 merged): exact geomean +1.0 %, fast −2.8 % (per model −7.8 % to +8.8 %,
  noise on the shared VM); golden hashes identical.
- **What X2d needs (resume point).** SPD in the exact mode needs the adjustment to run and its
  model change to take effect inside the exact engine, i.e. P2 (`MainTransformation` via
  `simoxide-reconf` in the ordered mode, the `ModelAdjusted` change list) and F4's Slingshot part
  (new containers' resources, passive resources of new allocation contexts, cloned monitors'
  calculators, removed containers per Q5), none of which exists yet (`ChangeError::Unsupported`
  for Slingshot runs). The event side is small and fits the engine: a hook trait implemented by
  `simoxide-spd` (it depends on `simoxide-sim`, not the reverse) with `SpdBehavior`'s time events
  at `PreSimulationConfigurationStarted` (queued before everything else at t = 0), one subscriber
  per policy and stimulus after `CoreBehavior` on `MeasurementMade` and `SimulationTimeReached`
  (each with its own disposed bit; `Spd::on_measurement`/`on_timer`), `ModelAdjustmentRequested`
  running the adjustment inside its handler, and `ModelAdjusted` with its four subscribers in the
  observed order (SS-SPD-5.1); the projection lines `spd_time`, `spd_request`, `spd_adjusted`
  (names of new and deleted containers) and the runner's aliases for created ids
  (`~EClass:entityName`). The engine reads the model live through `&CompiledModel`; after a
  change it must continue on the recompiled model with stable ids (F2's `compile_seeded` /
  `build_from_graph_after` keep them), so the engine's model reference becomes an `Arc` swapped
  between events, like `RunModel`. Corpus: the three SPD cases of `corpus-ss/` (1, 2 and 8
  adjustments), then `corpus-spd/` (P2).

**X2d as built** (2026-10-17, branch `x2d`; spec `docs/spec/slingshot/spd.md`, evidence
`docs/correctness/slingshot.md`):

- **The hook trait** (`simoxide_sim::ssx::SpdHooks`, `ssx/spd.rs`; the engine knows no SPD
  model): `start(model)` = `SpdBehavior.onPreSimulationConfigurationStarted` (the time events
  with their delays and target-group ids, the policy subscribers in registration order, each
  `Stimulus::Time` or `Measurement`); `on_time(subscriber, event, now)` and
  `on_measurement(subscriber, SpdMeasurement, now)` (one call per subscriber, each returning at
  most one `SpdRequest`); `adjust(request, &mut dyn ChangeHost)` (the adjustment on the engine's
  model, returning `ModelAdjusted`'s objects). Entry point `ssx::run_with_spd(cm, cfg, out,
  hooks)`; `ssx::run` is unchanged. Implemented by `simoxide_spd::ssx::SsxSpd` (interpreter state,
  decision log, request table, a series → measuring-point cache over `point_of_key`) with an
  `SsxAdjuster`; the interpreter gained only `Spd::subscribers`, `on_timer_of` and
  `on_measurement_of` (one policy context; `on_timer`/`on_measurement` are unchanged).
- **`ChangeHost`** (`simoxide_sim::ChangeHost`, new): what a model-transformation host needs
  from a running model (time, base and current compiled model, generation, model graph,
  `set_model_graph`, `apply_changes`, `fail_run`). Implemented by `Simulation` (forwarding) and
  by the ssx engine. `simoxide-reconf`'s `QvtoRun` (`invoke`, `invoke_with`, `dry_run`,
  `with_store`, ...) and `Edit::new`/`apply` are generic over it (callers unchanged);
  `QvtoAdjuster::adjust_on` is the one adjustment for both, `Adjuster<C, F>` and
  `SsxAdjuster` forward to it. So the adjustment is P2's unchanged: `MainTransformation` in
  QVTo's ordered mode, seeded ids, `SlingshotHooks` (`Before`, `ModelAdjusted::compute`,
  summaries, membership), the monitor clones as a second change set.
- **Bus ordering** (what ssx emulates): `SimulationTimeReached` queued as `PreSim`'s results
  (before any other set-up result, SS-SPD-1.2; a negative delay throws in `SpdBehavior`, which
  is disposed); the policy subscribers after every behaviour on `SimulationTimeReached` and
  `MeasurementMade`, each with its own disposed bit; `ModelAdjustmentRequested` at delay 0 runs
  `SpdAdjustmentBehavior` (disposal bit `H::SpdAdjust`; an adjuster error ends the run); its
  result `ModelAdjusted` has the four subscribers of SS-SPD-5.1 (`H::NoeResource`,
  `H::RsModelAdjusted`, `H::NoeSystem`, `H::MonModelAdjusted`). Projection lines `spd_time`
  (`tg`), `spd_request` (`policy`), `spd_adjusted` (`ok`, `changes`, and with a resource-
  environment change `new`/`deleted` container names), counted for the tape positions.
- **The model change inside the handler.** `ChangeHost::apply_changes` of the engine runs F2's
  `change::recompile` (stable typed ids, resources and programs appended; the Slingshot
  flavour's change options) and swaps the engine's `cm`/`m` at once, so the second change set
  (monitor clones) and the next event see the changed model. Every compiled model of the run
  is kept (`OwnModels::models`, an `Arc` each) until the engine is dropped: a reference copied
  before the swap stays valid (the only `unsafe`, documented). Read live from the new model
  (SS-SPD-5.2): system, connectors, SEFFs, load-balancer branches, the first-match allocation
  (recomputed at the change), link connections. Kept from the start: the linking-resource table
  (`LinkingResourceTable` is never updated). Changed at `ModelAdjusted`: the active-resource
  table (`res_live`: new containers' resources join with fresh schedulers, deleted containers'
  resources leave it, so their pending completion becomes `JobAborted` and the other jobs are
  lost: Q5), passive resources of the new allocation contexts (capacities drawn in order), and
  the cloned monitors' calculators (`Monitors::add`, the start's code made incremental).
  `NumberOfElementsMonitorBehavior` has no probe in this engine (no corpus model has an
  elastic-infrastructure measuring point; such a monitor would be ignored).
- **Aliases.** After each change the engine maps the ids of elements not in the input model
  to the reference runner's `Keys.alias` (`~EClass:entityName`, else the container's key plus
  `/feature.index`); projection ids and the measuring-point keys of monitors created during
  the run use them. Series are kept per measuring point (raw ids): two created elements with
  equal aliases (`spd_time_policies` creates `AppServer_1` twice) are two series with the same
  key, written one after the other as the reference's EDP2 measurements (the first diff found).
- **Simplifications** (no corpus case can observe them): the cloned monitors' calculators are
  registered at `MonitorRepositoryInterpreterBehavior.onModelAdjusted` instead of a few
  same-instant generations later (a request reaching a new container's resource within the
  adjustment's instant would be measured one sample earlier); policy subscribers never throw
  (the interpreter turns filter exceptions into disregards, SS-SPD-1.4), so their disposed bits
  are never set in practice.
- **Result.** `corpus-ss/` **89 of 90** in the test build (88 in the EPL build: h21's `Math.exp`),
  the three SPD cases byte-equal in projection, tape and measurements on the first run after the
  series fix; their decision logs equal the reference's `spd.jsonl` with models
  (`project_with_models`); `corpus-spd/` 4 of 4 (with `spd_service_group`) at level E. Remaining:
  h27 (licence-bound). Tests: `slingshot_exact.rs` (KNOWN = h27 only;
  `spd_decision_logs_match_the_reference`, `corpus_spd_is_byte_equal_at_level_e`; `SSX_DUMP=DIR`
  writes the outputs). The test pulls `simoxide-spd` and `simoxide-reconf` in as
  dev-dependencies (a dev-dependency cycle; integration tests only).
- **Gates** (after merging `spd` 6ee7e47 with M8). `slingshot_exact` 89 of 90 (KNOWN: h27),
  plus the two new SPD tests; the CLI's EPL build (`--features reconfiguration`, no `exact`, no
  `hotspot-math`) on all of `corpus-ss/` with the `.spd`: the 3 SPD cases equal, 88 of 90 in all
  (h21, h27); `golden.sh` identical (312 runs, `cargo build-exact`); `cargo test --release
  --workspace --features exact` 601 passed, 0 failed, 39 ignored (599 + 2); EMF goldens 165/165;
  `simoxide-fuzz corpus` 132/132, corpus-fuzz 6/6; `test-fdn` 93; `test-qvto` 90; `conform`
  compat 589/589, fast 584, ordered 583 (unchanged), sema oracle unchanged; fmt and clippy
  `-D warnings` (default, `exact,hotspot-math`, `compat`; `simoxide-cli` with `reconfiguration`
  and `exact,reconfiguration`); release check 0 opt-in crates (`simoxide-cli`, `qvto-cli`).
  Speed (sanity only): `x_ss_mediastore` 200 000 s in 0.93 ms per run (X2e: 0.83 ms, other load),
  `spd_scale_in` through the CLI with both adjustments 14 ms wall including the QVTo set-up.
  Found and left alone: `simoxide-cli`'s `resume_with_a_change_file` fails with only
  `--features reconfiguration` (fast mode) on `spd` 6ee7e47 too ("assembly context
  MediaManagement is not allocated"); the gate builds do not run it.
- **Shared files touched**: `simoxide-sim` (`change.rs`: `ChangeHost`; `lib.rs`;
  `sim/runmodel.rs`: its `Simulation` impl; `ssx/mod.rs`, `ssx/monitor.rs`, `ssx/resource.rs`,
  new `ssx/spd.rs`; `Cargo.toml` dev-dependencies; `tests/slingshot_exact.rs`); `simoxide-spd`
  (`interp.rs`: three methods; new `ssx.rs`); `simoxide-reconf` (`host.rs`, `lib.rs`: generic
  over `ChangeHost`; `adjust.rs`: `adjust_on`, `SsxAdjuster`); `simoxide-cli` (`main.rs`:
  `--flavour slingshot-exact` with a `.spd` runs `spd::run_exact`, all SPD quirks, `--spd-log`;
  the adjustments with `reconfiguration`, decisions only without; `compare_exact.sh` still
  passes `--no-spd`, unchanged because M8 owns the runner scripts).
- **What ssx pause and snapshots need (next; done: 5.7, "ssx snapshots as built").** The engine is built and consumed inside
  `run_impl`; a pausable run needs an owned `SsxRun` (engine plus the event list) with
  `run_until`/`finish`, the hooks passed per call. A snapshot must serialise the event list
  with Slingshot's entities: they are `Rc` graphs shared between events (users with their
  SimuCom stacks of shared mutable frames, interpretation contexts, behaviour holders with their
  iterators, requests, jobs), so it needs an id-per-object writer that keeps the sharing; plus
  the schedulers (serde in `simoxide-sched`), both random streams with their positions, the
  projection's numbering maps and line count, the monitors' open start-stop calculators and key
  maps, the disposed bits (handlers and policy subscribers), passives and `res_live`, the
  linking-resource table; with SPD the hooks' state (`SpdState` and the request table, serde
  already) and the run's own model (the last `Arc`, its build order, the generation, the graph
  override with the QVTo state: P2's `qvto` section); aliases and input ids are recomputable. A
  fork needs a deep copy of the `Rc` graph (not `Send`), so detached runs too.

### 5.3 The runtime-model-change foundation (shared)

This follows plan D1-D4 (`history/qvto-simoxide-integration.md` §3), with these additions:

- **`RunModel`** = `Arc<CompiledModel>` plus a copy-on-write overlay:
  - its own `raw::Graph` (cloned lazily on the first change);
  - appended resources, containers, series and routes, with tombstones for removed ones.
  - Indexes held by in-flight requests stay valid.
  - Runs without changes never touch the overlay. Guard: golden hashes and benchmark.
- **Change sources**, all going through the same path:
  - QVTo runs (journal);
  - direct edits through a small Rust API (snapshot parameter changes, tests);
  - SPD adjustments (QVTo);
  - SimuLizar rules (QVTo).
- **Syncers** read the journal summary and update:
  - resources (new containers → active resources, `container_simulated`, `finalise_order`);
  - links and routes (rebuilt; containers change rarely);
  - allocation;
  - system, connectors and SEFFs (`prov_deleg`, `req_conn`, `seff_for`, `code`, `progs`);
  - monitors and series (cloned monitors → new series).
  - A first version rebuilds the typed model and recompiles the changed parts, then diffs
    (0.2-0.5 ms on MediaStore). Incremental syncers replace this where profiling says so.
- **Slingshot-specific needs** (beyond plan D4):
  - **SEFF changes must take effect**. The load balancer's SEFF gains branch transitions and
    required roles; new assemblies and connectors appear. SimOxide compiles SEFFs into flat
    code, so the syncer must recompile the changed component's behaviours and invalidate the
    per-site call caches (`call_cache`, `elsc_prov`, `prov_ids`).
  - **Monitor cloning** like `ResourceContainerMonitorCloner`: new monitor and measuring-point
    objects in the graph, then new series through the same code as at initialisation.
  - **Removal policy**: drain (fixed, the default in both flavours), keep simulating (SimuLizar's
    behaviour, an option), or Slingshot's (abort the job with a pending event, lose the others; the
    Q5 compat flag, on in the Slingshot-exact mode).
  - **Linking-resource rewiring** (Q6): the route table is rebuilt.

**F2 as built** (2026-10-17, branch `f2`; rules: `docs/spec/model-changes.md`, MC-*):

- **Where.** The run model and the first syncer live in `simoxide-sim`, because they index the
  engine's private state: `src/change.rs` (change sets, `recompile`) and `src/sim/runmodel.rs`
  (`RunModel`, `Simulation::apply_changes`). `simoxide-reconf` is the boundary to QVTo: the
  adapter's `Journal` → `simoxide_sim::ChangeSet` (`change_set`, `apply_journal`) and direct
  edits over the adapter's `SimStore` (`Edit`: `set_spec`/`set_rate`, `clone_container`,
  `connect`, `allocate`, `clone_assembly`, `set_ref`, `set_double`, `remove_from_environment`,
  `delete`, or any `ModelStore` operation through `Edit::store`). F2 runs no QVTo.
- **Adapter in the build (temporary until M1/M2; gone since M2/M6, record only).** `crates/qvto-simoxide-bridge` is a package
  named `qvto-simoxide` whose `[lib] path` is `../qvto-rs/crates/qvto-simoxide/src/lib.rs`,
  built against this workspace's `simoxide-model`, with `qvto-model` and `qvto-emf` by path. A
  plain path dependency on the adapter fails: its manifest points at the `qvto-adapter`
  checkout, and two `simoxide-model` packages collide in one lockfile. The bridge and
  `simoxide-reconf` are workspace members but not default members; `simoxide-cli`'s
  `reconfiguration` feature pulls `simoxide-reconf` in. The adapter compiled unchanged against
  the `spd` tables.
- **`RunModel`** (in `RunState`): `Option<Box<{Arc<CompiledModel>, build order, base shape,
  generation}>>`. Not a field-level overlay: after a change the run has its own complete
  `CompiledModel`, a superset of the old one in which every index the state holds keeps its
  meaning. `Simulation::cm` stays a plain `&'m CompiledModel` (the hot path does not change) and
  is pointed into the state's `Arc` when a change is applied (`RunModel::compiled_ref`, an
  `unsafe` lifetime extension; invariant: only `apply_changes` swaps it, between events). Forks
  share the `Arc`; `Simulation::resume(&base, state)` checks the base model's shape and continues
  on the state's own model. The run's graph is `cm.model.graph`; a change set starts from a
  copy (`model_graph().clone()`, 58 µs for MediaStore). No copy-on-write graph was needed: the
  graph is off the hot path.
- **Stable typed ids.** `simoxide_model::build_from_graph_after(graph, diags, prev_order)`: the
  builder's two passes run over the previous build order followed by the new objects, instead of
  the graph traversal (`build` is `build_in_order(build_order(g))`, unchanged). Old elements keep
  their typed ids, new ones are appended, elements no longer contained in a resource stay as
  tombstones (built from the graph as it is: `EcoreUtil.delete` keeps an object's own values).
  `CompiledModel::compile_seeded(model, Seed { keys, progs, series })` keeps key, program and
  series ids (the series seed is the run's list, including series created during the run).
- **Rebuild and diff** (`change::recompile`): validate (the graph is a changed copy; no new
  error diagnostics; it compiles), rebuild, recompile, then merge into the running model:
  - resources matched by `ResKind` (typed processing-resource or link id): index kept, rate and
    latency synced (MC-1.3), replicas/scheduling/type/HDD ignored (MC-1.4); new ones appended,
    the run state gets idle `ResRt`s; removed ones keep their entries and jobs (MC-1.5). The
    resource registry is kept in the compiled model (`registry`) and the final-deactivation order
    recomputed from it (MC-1.6);
  - routing: `links` is the router's order; the typed model's `connected` lists become the
    router's view (union semantics, MC-2.2); `route_res` recomputed;
  - allocation: the old lookup plus the entries of added allocation contexts (MC-3.1); changed
    contexts ignored (MC-3.2); removals, `UNSET` and `MOVE` in the allocation abort (MC-3.3, from
    the change set);
  - assembly-context path trie: old nodes and the run's dynamic paths keep their ids, new nodes
    are appended, every node's container is recomputed from the merged lookup;
  - system, repository and usage tables (`prov_deleg`, `req_conn`, `seff_for`, `act`, `uact`,
    `scenarios`, parameters, trace ids) from the fresh compilation: read live (MC-4, MC-5.1);
  - instructions: unchanged → kept; changed → the fresh streams are appended (`Flat::append`):
    new executions start there, executions in progress keep their positions (MC-4.3);
  - the run state: resolution caches (`prov_ids`, `call_cache`, `elsc_prov`) dropped, dynamic
    paths moved into the compiled model.
  Everything is computed before the state is touched: on any error the run is unchanged.
- **Change sets.** `simoxide_sim::ChangeSet` is a neutral copy of the journal's kinds (objects
  as `ObjId`). The syncer works from the changed graph; the change set triggers it (empty = no
  change) and carries what only notifications show: aborts (MC-2.3, MC-3.3), connections added
  and removed again (MC-2.2), containers or links added and removed again in one set (refused).
- **Errors.** `ChangeError::{State, Invalid, Unsupported, Abort}`. `Abort` = SimuLizar's syncer
  throws (allocation context removed or unset — also `EcoreUtil.delete` of an allocated
  container —, `move`/`set` of link lists); a reconfiguration process (R1) must turn it into a
  failed run. `Unsupported` (F4): new series, windows or PRM recorders (monitor changes);
  population and scenario changes; structural action changes; a new container with an already
  registered id.
- **In-flight semantics (SimuLizar compat).** Queued jobs keep their demand; rates apply to
  later demands; removed resources keep simulating and stay reachable through unchanged
  allocation lookups; requests in progress use new allocation entries from their next demand
  and new connectors from their next call; behaviours in progress finish their old
  instructions (deviation: SimuLizar follows a changed successor chain at once).
- **Tests.** `simoxide-sim/tests/model_change.rs`: on every model directory, a change set with
  an unchanged graph at four pause points (the full path: rebuild, recompile, merge, adoption)
  equals the uninterrupted run byte for byte, also its fork, a resume on the base model and a
  detached copy on another thread, exact and fast. `simoxide-reconf/tests/changes.rs` (16):
  rate, latency, SEFF demand and branch-probability changes before the start equal the model
  changed on disk byte for byte; added container + allocation, composite moved to a new
  container, rewired connector to a new assembly context, rate and demand changes mid-run, a
  missing then added route; removed container = uninterrupted run; aborts and unsupported
  changes leave the run unchanged; fork-with-change = change-then-continue; fork, resume and
  detach after two generations; determinism, also in fast mode.
- **Gates.** `golden.sh` identical (312 runs); `cargo test --release --workspace` 257 passed;
  `simoxide-fuzz corpus` 132/132 and corpus-fuzz 6/6; clippy clean with and without `fast`.
  Speed (`simoxide-bench suite`, pinned, 20 alternating rounds, load 2-2.4): geomean −1.0 %
  exact, −0.1 % fast; single models swing ±4 % between builds that differ only off the hot path
  (bisected: the hot path is byte-for-byte the old code; a build with only `ir.rs` changes shows
  the same per-model swings), so this is code-layout noise. Two hot-path additions measured as
  real regressions were removed (a fallback lookup in `dyn_path_child_slow`, a `route_links`
  branch in `transmit`).
- **Cost of a change** (`simoxide-bench change`, halfway through the long runs): MediaStore
  (1 220 objects) graph copy 58 µs, rebuild-recompile-merge 216 µs (rate change 218 µs); small
  models 21-40 µs; generated s10 models 79-94 µs.
- **What F3 needs.** A per-run `Session` (2.4 ms; lazily on the first reconfiguration) that
  forks can own: the session holds the journal and the UUID generator, so a fork must clone or
  recreate it (id determinism); the session sees successive copies of the graph with the same
  ids, which works. The reconfiguration process runs inside an event (`Ev::Reconf`): either
  apply there (the swap is safe when no `cm` copy is live, true for `dispatch`'s `Reconf` arm) or
  queue the change set to the end of the event. `ChangeError::Abort` → run failure.
- **What F4 needs.** Incremental syncers where profiling says so (the merge is per kind
  already); monitor syncing (extend `meas.m.series`/`rows`, `series_windows`, PRM recorders,
  window events; `Number of Resource Containers` tuples); removal policies (keep = now;
  Slingshot abort; drain); population and scenario changes (spawn/stop users); behaviour
  changes in progress (follow the changed chain; structural action changes); dropping appended
  instruction streams no execution uses; containers added and removed in one change set; the
  Slingshot flavour's rules. After M5, port `build_from_graph_after` to the shared graph (the
  order/tombstone scheme carries over) and drop the bridge.

**F4 as built** (2026-10-17, branch `f4`; spec `docs/spec/model-changes.md` MC-4.3, MC-5.2, MC-6,
MC-8, MC-9; guides `library.md`, `reconfiguration.md`, `snapshots.md` "Change files", `spd.md`,
`slingshot.md`; tests `simoxide-reconf/tests/f4.rs`):

- **Options** (`simoxide_sim::ChangeOptions`, `Simulation::{change_options, set_change_options}`,
  part of the run state and of snapshots): `removal` (`Keep` | `Drain` | `Abort` | `SlingshotQ5`),
  `in_progress` (`Follow` | `Finish`), `reallocation` (`Ignore` | `Apply`), `monitors`
  (`Reference` | `Full`). Defaults per flavour (`for_flavour`): drain (Q5 flag: `SlingshotQ5`),
  follow, SimuLizar ignores and Slingshot applies reallocations, the reference's monitors;
  `ChangeOptions::simulizar()` = SimuLizar 5.2.2's syncers. The public change API is unchanged
  (`apply_changes`, `ChangeSet`, `ChangeReport`, `Edit`); N2's change files gained `removal`,
  `in-progress`, `reallocation`, `monitors`, `clone-monitors`, `add-demand` lines.
- **Removal policies** (`sim/removal.rs`, `Life` per resource, checked where a demand is issued:
  one predictable branch). Drain: jobs present finish, requests in progress at the change are
  served (a request = a usage-scenario run, `Proc::epoch` = the model generation at its start,
  inherited by forks), later requests reaching it are aborted, the resource is gone once idle with
  no such request (no final deactivation tuples). Abort: jobs aborted at the change
  (`ActiveResource::job_list`). Q5: the pending completion aborts its job, the others are lost.
  Keep: SimuLizar, including its aborts (MC-2.3, MC-3.3); under the other policies a removed
  allocation context keeps its lookup entries for requests in progress and `MOVE`s change nothing
  (the router's fixed semantics F3 asked for). Aborted requests (MC-8.6) restart closed users
  after a think time, drop open users, end forked behaviours; they count as finished runs.
- **Root cause of the OOM** (the first draft took the VM down at 28 GB): under `Drain` every
  request still allocated to a removed server was aborted, and an aborted request did not count
  as a finished usage-scenario run, so a run that stops after N runs (h19: 300) never stopped
  while its trace and tape grew in memory. SimuCom counts failed runs too (`finally {
  increaseMainMeasurementsCount() }`); aborted requests now count (except Slingshot's Q5
  `UserAborted`). Regression tests: every policy stops such a run under an event limit; 300
  change sets on one run keep instructions, series, resources, programs and graph bounded.
- **Executions in progress** (`sim/runmodel.rs` `plan_code`): instruction streams are rebuilt at
  every behaviour change; `Follow` (SimuLizar: the successor is read after each action) maps each
  SEFF and usage-behaviour program counter to the same instruction in the changed behaviour, or
  to the same place of a structurally changed action (rank: begin < demands < infrastructure calls
  < resource calls < call/effect < return < end); executions whose action left the behaviour or
  whose call lost its return keep their old instructions (`Finish` keeps them for all). Kept
  segments are appended (`Flat::sseg`/`useg`), only those still used survive the next change (the
  GC: streams never grow with the number of changes). Kept instructions that no longer fit the
  changed action, and an infrastructure call or collection iterator in progress whose action
  lost that structure, are refused (`Unsupported`); structural changes are otherwise supported.
  Call caches and resolutions are dropped at every change (MC-4.4).
- **Workloads** (MC-5.2): population changes start users at once or let the oldest leave after
  their run (`STOP_REQUESTED`); new usage scenarios start their drivers, removed ones stop
  (`CScenario::removed`, indices stable); a changed workload kind is ignored (UMS).
- **Monitors** (MC-6): new series are accepted per mode (SimuLizar `Reference`: those of new
  active resources, `attachMonitors`; Slingshot and `Full`: all) and renumbered after the run's
  series; existing calculators are never removed; new actions use existing series by id (a
  `deepclone` copy keeps its original's id: `lb_clone_branch`). `Full` also merges sliding
  windows (first window one length after the change), PRM recorders (`Prm::attach_new`, values by
  `Prm::value_of`), reconfiguration-time and container-count series. Under `Reference` the
  recompilation builds windows, PRM and reconfiguration monitors from the start's monitors only
  (`Seed::start_monitors`, `CompiledModel::start_monitors`): a cloned utilisation monitor no longer
  makes the SimuLizar compile fail. Monitor cloning: `Edit::clone_monitors(template, container)`
  and `Edit::clone_monitor(monitor, target, id)` (Slingshot's `ResourceContainerMonitorCloner`);
  `QvtoAdjuster` creates `ModelAdjusted::monitor_clones` right after the transformation's change
  set (`clone_monitors`, default on). Container-count tuples at changes outside a reconfiguration
  process (MC-6.4).
- **Slingshot flavour** (MC-9; the refusal is gone): resources of allocated containers from the
  fresh compilation (`res_mon` probes, link failure), first-match allocation read live (removed
  contexts keep their entries except under Q5, `Reallocation`), live link connections, new links
  routed, passive resources of new allocation contexts at the change
  (`ss_create_passives`), passive monitors refreshed, Q14 ignores population changes, Q5 removal.
  The Slingshot-exact engine is untouched (X2d: `RunModel`'s `Arc` swap and `set_model_graph`).
- **Other kinds**: re-pointed allocation contexts (`Reallocation::Apply`); a container or link
  added and removed in one set (`Keep`: its id is registered, MC-8.5; the typed model never sees
  the object, its resources are unreachable and unmonitored).
- **Incremental syncers: not needed.** `simoxide-bench change` (fast, 20 reps, one core of the
  loaded VM): MediaStore graph copy 77 µs, rebuild-recompile-merge 298 µs, rate change 290 µs (F2:
  303/283), small models 27-46 µs, generated s10 112-132 µs; an SPD adjustment's QVTo run costs
  milliseconds, so the rebuild is not the bottleneck even for a scale-out every few seconds.
- **Tests.** `f4.rs` (15): the OOM case; bounded growth over 300 changes; population up and down,
  before the start equal to the changed model; scale-in under keep/drain/abort in both flavours and
  Q5 (with and without routing away); cloned monitors measuring a new container (both flavours);
  `Full` vs `Reference` for a new monitor of an existing element; executions in progress following
  an inserted action and a structural change (`Follow` vs `Finish`); a load-balancer branch added
  mid-run plus an action after the branch (routing changes, executions in progress run the new
  successor); add and remove in one set (container, link); removed usage scenarios; snapshots
  after each change kind (population, scenario removal, monitors in both flavours and `Full`,
  every removal policy in both flavours, follow/finish, add+remove) restore and continue byte for
  byte; N2's change files carry every F4 kind in both flavours. Adjusted: F2/F3 compat tests run
  with `ChangeOptions::simulizar()`; N2's and R1's refusal tests became applied-change tests; P2's
  waiting markers flipped (`spd_scale_in` equals Slingshot with its scale-in: 8 records, 2
  adjustments; the fixed behaviour of `spd_time_policies` scales in; the Slingshot flavour's
  decision log equals the reference and its new container is probed).
- **No F4-dependent SimuLizar example for `corpus-rules/`**: the media store's horizontal scaling
  and `migratecomponent.qvto` both need SimuLizar's action framework (not runnable headless, R1).
- **Gates.** `golden.sh` identical (312 runs); `cargo test --release --workspace --features exact`
  589 passed, 0 failed, 38 ignored (574 after the P2 merge, +15 `f4`); corpus 132/132, corpus-fuzz
  6/6; EMF goldens 165/165; `test-fdn` 93; `test-qvto` 90; `conform` compat 589/589, ordered 583
  (unchanged); fmt, clippy `-D warnings` (default, `exact,hotspot-math`, `compat`); release check
  0 opt-in crates. Every test and benchmark process ran under `prlimit --as=10 GB`. Runs without
  changes (`simoxide-bench suite`, 2 alternating rounds, one core, load about 6, against `spd`
  553ae1c): events/s geomean −1.4 % exact, −2.4 % fast (models −5.8 % to +1.4 %).
- **Shared files touched**: `simoxide-sched` (`resource.rs`, `fcfs_head.rs`: job lists);
  `simoxide-sim` (`change.rs`, `code.rs`, `ir.rs`, `lib.rs`, `sim.rs`, `windows.rs`, `sim/flat.rs`,
  `sim/reconf.rs`, `sim/runmodel.rs`, `sim/slingshot.rs`, `sim/snapshot.rs`, `sim/state.rs`, new
  `sim/removal.rs`); `simoxide-reconf` (`lib.rs`, `edits.rs`, `adjust.rs`; tests `changes.rs`,
  `host.rs`, `resume.rs`, `rules_process.rs`, `spd_adjust.rs`, new `f4.rs`);
  `corpus-spd/spd_scale_in/case.json`; docs.
- **What P2, X2d and P3 can rely on**: removal (drain default, Q5 under its flag) and
  allocation-context removal in both flavours; Slingshot-flavour changes (new containers'
  resources and probes, passive resources of new allocation contexts, first-match allocation);
  monitor cloning (`Edit::clone_monitor(s)`, the adjuster's `clone_monitors`); new series appended
  to `Measurements::series` at the change (an observer keyed by series must look at the tail after
  each adjustment, `SpdRun` maps series once at the start); container-count tuples; executions in
  progress following changed behaviours.

### 5.4 SPD layer (`simoxide-spd`)

- **Loading.**
  - SPD and semanticspd files are loaded like PCM files: `.spd` and `.semanticspd` are added to
    `MODEL_EXTENSIONS` once the packages are in the tables (5.6).
  - A typed view (`SpdModel`) is built at compile time. It is immutable apart from the
    semantic configuration, which QVTo changes in the graph.
- **Interpreter.**
  - One `PolicyCtx` per active policy, with the chain from §1.2. Filters are an enum, not trait
    objects; the order is as in Slingshot.
  - State: `SpdState { per_policy: Vec<AdjustorState>, per_group: Vec<GroupState>, aggregators }`.
    It is plain data and part of `RunState` (5.7).
  - Stimulus sources:
    - `SimulationTime`: an engine event at the expected time (in both flavours).
    - Measurements: a per-series observer list in the measurement sink. The observer fires for
      each tuple of a series whose measuring point is inside the target group and whose metric
      matches. It is empty and free when no SPD is loaded.
    - In the Slingshot flavour the tuple stream is Slingshot's `MeasurementMade`. In the
      SimuLizar flavour it is SimuLizar's tuples (documented as a deviation; no reference).
  - On success, the adjustment is queued as an engine event at `now` (FIFO), as Slingshot does.
    It is not run inline.
- **Adjustment.**
  - Set `enactedPolicy` in the configuration object of the graph.
  - Run the cached compiled `MainTransformation` on the run's graph (5.5). Bind by nsURI:
    allocation, system, resource environment, repository (inout), SPD (in), semanticspd
    (inout), usage model (inout). Set `keepModeling=true`.
  - Collect the journal. Compute `ModelAdjusted` as Slingshot does: diff of the container and
    allocation lists. Clone monitors. Run the syncers.
- **Transformations.**
  - `[TR]` (18 files, 3 534 lines; 15 reachable from Main, 2 506 lines) are embedded with
    `include_str!` and pinned to a commit.
  - Each embedded file is marked with its source repository, its revision and "licence pending
    confirmation" (U3).
  - The importer resolves `pcm.helpers.*` and `spd.*` against the embedded tree.
  - `ProfilesLibrary` is registered with stub implementations that report an error if called.
    The main path never calls them.
  - The user's own transformations can replace `MainTransformation` (an option).
  - The fixed behaviours of Q4, Q6 and Q7 (proposed, P2; U13) need a patched copy of a few
    transformation files. The unchanged set runs when those flags are on, so always in the exact
    mode.

**P1 as built** (2026-10-17, branch `p1`; guide: `docs/guide/spd.md`):

- **Crate `crates/simoxide-spd`** (EPL, default member; feature `sim`, default, for the
  simoxide-sim driver; without it the crate depends on `simoxide-model` only):
  - `model.rs`: `SpdModel::from_graph` (the first `SPD` and `Configuration` objects of the
    graph): policies, triggers (simple and composed, any depth), stimuli, expectations,
    adjustments, policy and target constraints, target groups. `Membership` (plain data): the
    semantic configuration's target-group configurations, the allocation, the environment's
    containers and the provided signatures of allocated assemblies, read with
    `Membership::from_graph`; the host refreshes it after an adjustment. `PointRef`: a measuring
    point resolved to what `MeasuringPointInsideTargetGroup` and the checkers read.
  - `interp.rs`: `Spd::new` interprets the model once (`SpdInterpreter`): one `PolicyCtx` per
    active policy with `chain: Vec<Filter>` (an enum, Slingshot's order) and the trigger tree
    `nodes: Vec<Node>`; the time events (one per `SimulationTime` leaf, policy order, then
    trigger order). `SpdState` (plain data, `Clone + PartialEq`): per policy the adjustor state,
    the parked position of the outer chain, per trigger node its windows or its compound
    iterator; per group the last enactment; the membership; counters. The chain runs like
    `FilterChain.next`: from the start unless a filter returned `null`, which parks it.
    The compat compound filters are ports of `LogicalANDComboundFilter`, `LogicalORCompoundFilter`
    and `LogicalXORCompoundFilter` with their iterator state (the AND recursion, the XOR's
    counters and `eventToProcess`), built as `ScalingTriggerInterpreter.addFrom` builds them (an
    AND joins its first child's AND filter, OR and XOR nest pairwise).
  - `aggregate.rs` (EMA, last-10 window, the functions with `Double.compare` order and the
    compensated mean), `adjust.rs` (`AdjustmentCalculator.qvto`: desired change, size clamp,
    effective change), `log.rs` (records, `VecLog`, `JsonlLog`, `project`), `jfmt.rs` (Java
    `%f`), `quirks.rs`.
  - `sim.rs`: `SpdRun` drives a `Simulation` (any `Compat`): time events and requests are
    external notes; measurements come from observed series (series → `PointRef` and metric from
    the measuring-point key); `Adjuster` executes requests (P1: `NoAdjustment`, `ok: false`);
    `run_until(before)` and `fork` pause and fork with the SPD state.
- **Host trait.** `SpdHost { now, schedule(at, timer), request(req) }`; the host calls
  `Spd::on_timer`, `Spd::on_measurement` and `Spd::adjusted(req, ok, …)`. The output is an
  `AdjustmentRequest { id, policy, ctx, group, adjustment, time }`. The unit tests use a fake host.
- **Touched in simoxide-sim** (generic, no SPD code; for the merge with `s2`):
  - `src/events.rs`: `Ev::External(u32)`;
  - `src/sim.rs`: `mod hooks`, `pub use hooks::ObservedTuple`, a field `hooks` in `Sink`, one
    dispatch arm (`Ev::External(tag) => hooks.due.push(tag)`), and in `emit` and `emit_out` one
    branch `if meas.hooks.observing { hooks::observe_tuple(…) }` (cold function);
  - `src/sim/hooks.rs` (new): `Simulation::{schedule_external, observe_series, take_external,
    take_observed}`, `RunState::hooks_pending`;
  - `src/sim/state.rs`: the field's initialisation;
  - `src/config.rs`: `SPD_EXTENSIONS`; `RunSpec::load_model` and `load_model_memory` load the
    directory's `.spd` and `.semanticspd` files after the entry files. (`simoxide_model`'s
    `MODEL_EXTENSIONS` stays the PCM set: its `load_dir` is compared with the PCM-only EMF
    oracle, `tests/golden.rs`.)
  - simoxide-cli: feature `spd` (default), `simoxide run --spd-log/--spd-log-all/--spd-quirks/--no-spd`.
- **Timing.** A time event is a note scheduled before the start, so it precedes the
  initialisation's notes at the same time (Slingshot: `SpdBehavior` before the monitors,
  SS-SPD-1.2) and runs FIFO among same-time notes. A request is a note at `now` behind the
  queued ones (`ModelAdjustmentRequested`, SS-SPD-5.3). Measurements are evaluated right after
  the event that emitted them, before the next note; Slingshot's `MeasurementMade` (delay 0)
  runs behind the notes already queued for `now` (a difference only for same-instant order, to
  be modelled by the exact mode, X2). A run that stops drops the requests of its last event
  (`SpdSummary::pending`). Time events and request notes count as events (`RunResult::events`).
- **Flags** (`Quirks`, all on = `Quirks::slingshot()`): Q1 (no observable effect), Q2, Q4 (the
  arithmetic, used by the Q9 check), Q9, Q11 (also wrong expectation types, CPU percentage
  outside 0-100, `QueueLength` outside competing-consumer groups, unsupported constraints: the
  whole SPD is dropped, `Spd::dropped`), Q12 (with the double evaluation of two measurement
  stimulus kinds), Q13, Q15. Q3 is kept (no flag). Provisional fixed behaviours: Q3 (kept),
  Q4, Q9 (disregard if the clamped change is 0 under a group-size constraint; the filter
  `TargetGroupSizeConstraintFilter` sits with the thrashing filters in target-constraint
  order), Q13 (one window per measuring point, the method over the ready windows, the EMA
  emit slot of the trigger consumed only when the policy fires). Fixed XOR = exactly one child.
- **Decision log** (`docs/guide/spd.md`): the reference runner's `spd.jsonl` records `policy`,
  `eval`, `adjust`, without the event index `e`, the `log` lines and the `adjust` model summaries;
  `log::project` reduces both logs to what they share (`adjust`: time, policy, name, `ok`).
- **Comparison** (`tests/reference.rs`, P1 exit criterion), all compat flags on, SimuLizar
  flavour, exact mode, seed 1, 100 s:

  | Model | Records equal |
  |---|---|
  | `models/spd_minimal` (reference: `upstream-report/reproductions/spd_minimal/observed`, same files) | 3 |
  | `models/spd_scale_in` (same) | 8 |
  | `models/spd_time_policies` (new, `expected/` from the det runner) | 64 |
  | `reproductions/spd_or_trigger` | 4 |
  | `reproductions/spd_and_trigger` | 4 |
  | the 20 probes (no SPD) | 0 (empty logs) |

  `spd_time_policies` covers `>=`/`<=`/`>` triggers firing on other policies' events (Q15),
  cooldown with `maxScalingOperations` (including `t == coolDownEnd`), a one-off and a repeated
  interval, thrashing within one event (a scale-in rejected right after a scale-out of the same
  instant), absolute and relative adjustments, an inactive policy, a time event at t = 0.
  **How the missing adjustment is handled:** the run feeds the reference's adjustment outcome
  into a test double (`ReferenceAdjuster`): for each request it reports the reference's `ok` for
  the same policy and time and sets the target group's element count to the reference's
  `after` summary. Comparing only up to the first adjustment was not needed: with the compat
  flags (Q2: state changes at the request; Q9: no size check) no decision of these models
  depends on the adjustment except through `ok` and the group size, which the double supplies.
  With the fixed behaviour the logs are checked against hand-derived expectations: `spd_minimal`
  equals the reference; `spd_scale_in` loses the two cross-policy evaluations (Q15);
  `spd_or_trigger` adjusts at 50 and 70; `spd_and_trigger` never; `spd_time_policies` fires
  Out20 at 20, InLe50 at 50, Abs60 at 60 and In77 at 77.
  The SimuLizar flavour rejects the Slingshot models' monitor repository (utilisation monitors
  without `TimeDriven` processing type), so the test loads these models without it (time
  triggers read no measurements).
- **Other tests.** `tests/unit.rs` (20, V1: every filter and quirk flag in both settings with a
  fake host, forks of the state), `tests/sim.rs` (observers deliver CPU, task-count and
  response-time tuples of a corpus model, outputs equal the plain run, a forked SPD run equals
  the uninterrupted one), unit tests of windows, functions, arithmetic and `%f`.
- **Slingshot behaviours found and kept** (not in Q1-Q16; candidates for the quirk list):
  - for an elastic infrastructure, an assembly or operation measuring point is "inside" the
    group if *any* allocated assembly of the environment matches, not only those on the group's
    containers (`TargetGroupUtils.isAssemblyInTargetGroup`, `getRelatedAssemblyContextFromInfrastructure`);
  - `CPUUtilizationTriggerChecker` takes the utilisation tuples of every active resource of the
    group (CPU, HDD, delay), not only CPUs;
  - an elastic infrastructure without a configuration entry for its unit throws in the
    target-group check (logged as `exception`);
  - a nested OR inside an AND makes the AND return `null` too: the outer chain parks (Q12,
    derived from the code, no reference run).
- **Gates.** `golden.sh` identical (312 runs); `cargo test --release --workspace` 292 passed, 0 failed (F2: 257; +35 in `simoxide-spd`);
  `simoxide-fuzz corpus` 132/132 and corpus-fuzz 6/6; fmt, clippy `-D warnings` clean (workspace, all targets;
  also `simoxide-spd` without `sim`, `simoxide-cli` without defaults with `fast` and with `spd`).
  Speed (`simoxide-bench suite --reps 3`, 20 alternating rounds base 7e343ce / p1, pinned to one
  core, shared machine at load 5-7): geomean of the long runs +0.7 % (counted) and +3.0 %
  (stored), short runs -1.0 %; single models -2 % to +8 %, except `nested subsystem` -20 % in
  that series, which did not reproduce (12 further runs on another core: equal, 24.2 vs 24.3
  Mev/s): within noise. The hot path gains one predictable branch per emitted tuple
  (`hooks.observing`) and a dispatch arm.
- **What P2 needs.** An `Adjuster` that runs `MainTransformation.qvto` on the run's graph
  (F3/F4): set `Configuration.enactedPolicy`, bind the models, apply the journal with
  `Simulation::apply_changes`, refresh `SpdState::membership` (`Membership::from_graph` on the
  changed graph), report `ok` and the sizes; the `adjust` record's model summaries (containers,
  links, allocation, target-group elements) for V3; monitor cloning so that new containers'
  series are observed (`SpdRun` maps series once at the start: extend it when series are
  appended); the patched transformations for Q4/Q6/Q7. For P3: the Slingshot flavour's
  `MeasurementMade` stream (instantaneous utilisation, S2) into `on_measurement`, observing the
  lazily created passive-resource series (`QueueLength`), and the exact mode's same-instant
  order and event index (X2).
- **From Q1/Q2** (5.5): run `MainTransformation` with `qvto::Mode::Ordered` for the
  Slingshot-exact mode (`Fast` otherwise; compat mode is never needed), `keepModeling=true`, the
  `ProfilesLibrary` signatures with stub natives; the transformation copies with licence headers
  are in `crates/qvto/qvto-testkit/corpus/spd/transformations`. The host must give objects that
  QVTo creates distinct PCM `id`s, as Slingshot's generated classes do (`Wiring`'s `.id`
  comparisons misbehave on `null` ids, corpus cases `min3_noid_*`), and resolve the models'
  proxies at load. `qvto spd-replay`'s model summary is the V3 comparison for the adjustment
  (container, link, allocation and target-group order equal the `det` runs); it also shows two
  candidates for the quirk list (the parentless `RequiredDelegationConnector`s, duplicate names
  across adjustments).

**P2 as built** (2026-10-17, branch `p2`; guide `docs/guide/spd.md`, "Adjustments"; tests
`simoxide-reconf/tests/spd_adjust.rs`, `simoxide-cli/tests/spd_cli.rs`). Done except what needs
F4 (listed below).

- **Transformations** (`simoxide-reconf/src/spd.rs`): the 17 units of Slingshot's bundle are
  embedded with `include_str!` from the licence-headed copies in
  `crates/qvto/qvto-testkit/corpus/spd/transformations` (U3: source, revision, "licence pending
  confirmation"); an in-memory `UnitResolver` resolves `pcm.helpers.*`/`spd.*`. Patched copies
  for the provisional fixed behaviours in `simoxide-reconf/transformations/spd-fixed` (headed
  "PATCHED by SimOxide", changes marked `SimOxide Qn`): Q4 `AdjustmentCalculator` (0 stays 0,
  otherwise at least the minimum in the percentage's direction, as P1's arithmetic), Q6
  `ResourceEnvironmentTransformation` (a link keeps its remaining containers in order; a link
  the unit is on gets the new elements appended), Q7 `TopDownTransformationInfrastructure`
  (allocations that `modifyAllocationOnScaleIn` removes in this enactment do not count, so
  emptied containers go now). `SpdVariant` (from `Quirks::{q4_relative_zero, q6_all_links,
  q7_late_removal}`, new flags Q6/Q7) picks per unit; `Quirks::slingshot()` runs the unchanged
  bundle. `main_transformation(core, variant)` compiles once per core (`QvtoCore::compiled`,
  a per-core memo).
- **Adjuster.** `QvtoAdjuster::spd(core, &SpdAdjust)`: QVTo `Ordered` by default (`Fast` an
  option; compat never needed), `keepModeling=true`, Slingshot's binding, seeded ids for every
  created PCM object (F3's `IdPolicy::Uuid`), `FailedRunChanges::Keep`. `SlingshotHooks`:
  `prepare` (new `AdjustHooks` method: the run's base graph, for the summaries' input ids),
  `before` sets `enactedPolicy` and captures the lists and the summary, `after` computes
  `ModelAdjusted`, refreshes `Membership`, reports `ok`, sizes, `changes`, summaries.
- **`simoxide-spd`.** `adjusted.rs`: `Before::capture`, `ModelAdjusted::compute` (old/new/deleted
  containers, new allocation contexts, `monitor_clones` as `ResourceContainerMonitorCloner`
  would create them: unit-container monitors, then same-type processing-resource monitors, ids
  compared; `changes()` = 2 + clones), `summary` (the reference runner's `SpdObserver.summary`,
  `~Class:name` keys for created elements). `AdjustOutcome` (now in `interp`, not `Copy`) carries
  `changes`, `before`, `after`, `adjusted`; `Spd::adjusted_with`; the log's `adjust` record writes
  them (`project_with_models` keeps them for comparisons). `SpdRun` is generic over the flavour
  (`SpdRun<'m, 's, C, F = SimuLizar>`), observes series a changed model appends (after each
  adjustment and `model_changed`), and restores its series map from the run's current model.
- **`enforce_policies`** (N2's hook): `Spd::enforce` builds a request past chain and constraints
  (`AdjustmentRequest::enforced`; an inactive policy has `ctx = usize::MAX`); the requests are
  queued at the current time; their outcome commits no state (Slingshot's adjustor never sees
  them); log records carry `"enforced":true`. Errors: `UnknownPolicy`, `NoTargetGroup`
  (nothing queued). CLI `resume --enforce`.
- **CLI.** With `spd` and `reconfiguration` the SPD run uses `QvtoAdjuster::spd` (`--qvto-mode`,
  `auto` = ordered); one QVTo state per SPD run (the snapshot's `qvto` section, also for
  `--change`/`--transform` at its resume); prints `adjusted=N`. Without `reconfiguration`:
  decisions only, as before. The Slingshot flavour still refuses SPD (F4/P3).
- **Snapshots.** An SPD run with adjustments (sections `engine`, `model`, `spd`, `qvto`) restores
  and continues byte for byte; found and fixed: a restored graph wrote its placeholder resources
  (the QVTo prototype's metamodel documents) under `simoxide:/snapshot-placeholder/{i}`, so a
  restored run did not write the bytes it was read from. Placeholders now keep the original URI
  (`simoxide:/snapshot-placeholder/{i}?{uri}`) and save as the foreign resource they stand for
  (`simoxide-model/src/persist.rs`).
- **SimuLizar flavour end to end.** The default path (ordered QVTo, fixed Q6) adds the replica
  without moving a container inside a link list, so SimuLizar's router accepts it (F3's MC-2.3
  abort was compat mode's HotSpot-hash order); also with all flags on (ordered and fast) the
  minimal example, the time policies and the service group run. The Slingshot models' utilisation
  monitors need a `TimeDriven` processing type there: the tests use copies with one.
- **V3** (corpus `corpus-spd/`, `det` outputs written by `CORPUS=corpus-spd
  reference/slingshot/corpus-ss.sh`, byte-equal to `corpus-ss/`'s for the three shared models;
  SimuLizar flavour, all compat flags on, ordered and fast mode): decision log equal to
  `spd.jsonl` including every `adjust` record's `changes`, `before` and `after` (scale times,
  container, link, allocation and target-group names and order):

  | Case | Records | Adjustments equal |
  |---|---|---|
  | `spd_minimal` | 3 | 1 of 1 |
  | `spd_time_policies` | 64 | 8 of 8 |
  | `spd_service_group` (new: top-down +1, +2, then bottom-up +1) | 11 | 3 of 3 |
  | `spd_scale_in` | 7 of 8 | 1 of 2: the scale-in at 80 s waits for F4 |

- **Other tests.** Fixed behaviour (minimal example's links per Q6); Q4/Q7 patches by dry runs
  on the Q1 corpus' states (Q4: -10 % with minimum 2 on three containers removes 0 vs 2; Q7: a
  top-down scale-in removes the emptied container at once); snapshots at eight times
  (`spd_time_policies` Slingshot, `spd_minimal` fixed) with fresh adjusters, re-written bytes
  equal; bounded growth: 40 enforced scale-outs add 12 objects each, journals stay flat; the
  Slingshot flavour's refusal; `enforce` in `simoxide-spd/tests/sim.rs`; the CLI in new
  processes.
- **Waited for F4; finished after F4 landed** (`spd` d57907b, merged): F4 flipped the markers
  (scale-in drains; `spd_scale_in` 8 records, 2 adjustments equal; the Slingshot flavour; the
  adjuster clones `ModelAdjusted::monitor_clones`). P2 then: `SpdRun` maps the run's
  `Measurements::series` (the compiled model's plus those a change appends), not the compiled
  model's, after every adjustment and at restore (test `measurement_triggers_see_new_containers`:
  a CPU policy's trigger gets a window for the new container's series in both flavours, SimuLizar
  with `MonitorSync::Full`); add and remove at one instant: `corpus-spd/spd_same_instant` (+2 and
  -1 at 50 s, with Q15 each twice: 4 adjustments equal); V3 in the Slingshot flavour (fast mode,
  Slingshot's fast-mode quirks): all five `corpus-spd/` cases' decision logs equal the reference.
- **Gates.** `golden.sh` identical (312 runs, `cargo build-exact`, against `spd` fb03e7d);
  `cargo test --release --workspace --features exact` 574 passed, 0 failed (566 before; +6
  `spd_adjust`, +2 `spd` unit tests; `simoxide-cli`'s `spd_cli` runs with `reconfiguration`);
  `simoxide-fuzz corpus` 132/132 and corpus-fuzz 6/6; `cargo conform --suite all` 589/589 (fast
  584, ordered 583, unchanged), sema oracle unchanged, `test-qvto` 90; fmt and clippy `-D
  warnings` clean (default, `exact,hotspot-math`, `compat`; `simoxide-cli` with
  `reconfiguration` and `exact,reconfiguration`; `simoxide-reconf` and `simoxide-spd` without
  default features); release check: 0 opt-in crates in `simoxide-cli` and `qvto-cli`. Timing
  deferred; sanity: the CLI runs the minimal example with its adjustment (core, compilation, run)
  in 13 ms wall.
- **For X2d and P3.** `simoxide_spd::adjusted` (`Before`, `ModelAdjusted` with new/deleted
  container names for `spd_adjusted`, `monitor_clones` for the `MonitorChange`s, `summary`),
  `QvtoAdjuster::spd` callable from any `Simulation<C, F>` (`Adjuster<C, F>`), `SpdRun` over any
  flavour, `Spd::enforce`.

**P3 as built** (2026-10-17, branch `p2`; evidence `docs/correctness/slingshot.md` "Scaling
policies"). Done (V3x with X2d).

- **Slingshot flavour end to end.** The CLI's refusal is gone: `simoxide run`/`resume` run
  `SpdRun<C, F>` for both flavours (and refuse `--flavour slingshot-exact` with SPD: X2d).
  Found and fixed: (1) an adjustment request at the current time failed in the Slingshot flavour
  ("external note ... before the current time"): its double clock rounds `now` to nanoseconds;
  `Simulation::schedule_external` takes the current time for the current nanosecond
  (`sim/hooks.rs`); (2) with the fixed behaviour (drain) a removed container's requests in progress
  call over a link the transformation took it off, a run error ("no linking resource connects");
  the fixed Q37 now uses a link touching the remaining side when one side was removed from the
  environment (`sim/slingshot.rs`, Slingshot's own fallback "the other one got deleted during
  scaling in"). Tests: `slingshot_flavour_drains_a_removed_container`,
  `slingshot_flavour_runs_a_stochastic_cpu_scenario` (both quirk settings, three seeds,
  deterministic), `adjustments_equal_slingshot_in_the_slingshot_flavour` (V3, five cases).
- **V4** (`reference/slingshot/spd-stat.sh OUT BIN [FROM TO]`, `CONTROL=1`; `tools/spd_stat.py`,
  `tools/compare_stat.py --created-from`): scenarios `spd_v4_cpu` (CPU AVERAGE > 80 % out, < 60 %
  in, cooldowns 30/60 s) and `spd_v4_rt` (operation response time > 20 s out), `Exp` arrivals and
  demands, 600 s, sizes 1..4; stock Slingshot (one JVM per seed, `MALLOC_ARENA_MAX=2` under
  `prlimit`) against the Slingshot flavour with Slingshot's quirks, seeds 1-30. Per-run SPD
  statistics (adjustments, scale-outs, scale-ins, first adjustment time, time-weighted and final
  size, mean response time in four windows, pooled adjustment times): 0 of 21 and 0 of 20 tests
  reject (Holm 0.01). All series: 1 of 42 and 2 of 42 reject, all pooled-values KS, which the
  control (stock seeds 1-30 against 31-60) also rejects (3 of 42, 1 of 42): not a valid test on
  autocorrelated values. New containers' series are pooled per class (the runner keys them
  `~ResourceContainer:AppServer_1/...`, SimOxide by seeded ids).
- **Gates** (after merging `spd` 4ba5244). `golden.sh` identical (312 runs); `cargo test --release
  --workspace --features exact` 605 passed, 0 failed (601 on `spd`; +4 `spd_adjust`); CLI tests
  with `reconfiguration` pass (the N2 test's F4-lifted refusals updated); corpus 132/132,
  corpus-fuzz 6/6; `conform` 589/589 (fast 584, ordered 583), sema unchanged, `test-qvto` 90; fmt
  and clippy `-D warnings` clean (default, `exact,hotspot-math`, `compat`; the CLI with
  `reconfiguration`, `exact,reconfiguration`, without default features); release check 0
  opt-in crates. Every test and harness process under `prlimit --as=10 GB`. Timing deferred;
  sanity: a 600 s V4 run takes 25 ms in SimOxide, 4.8 s in stock Slingshot (one JVM).
- **V3x (exact SPD runs): holds with X2d** (merged from `spd` 4ba5244: `SsxSpd`, `SsxAdjuster`,
  `QvtoAdjuster::adjust_on`): all five `corpus-spd/` cases (`spd_same_instant` added to
  `slingshot_exact.rs`'s `corpus_spd_is_byte_equal_at_level_e`) and the three SPD cases of
  `corpus-ss/` are byte-equal at level E (projection, tape, measurements).

### 5.5 QVTo host for both flavours (`simoxide-reconf`)

- The QVTo VM runs on the run's graph through the foundation's `ModelStore` implementation.
  There is no adapter and no second home.
  - PRM measurements, SPD models, semanticspd, metamodel objects and intermediate-class
    instances live in the same graph, in their own resources.
  - Until the merge lands, the Q6 adapter (`qvto-simoxide`) serves as a bridge; the API seen by
    `simoxide-reconf` stays the same.
- The per-run QVTo state (resources table, journal, id policy) is created on the **first**
  reconfiguration. Today's Q6 session costs 2.4 ms, too much for runs that never reconfigure.
- The parts that never change are shared through `Arc` across runs and batch threads:
  the registry, the metamodel objects and the compiled programs (compiled once per
  `CompiledModel`, about 1 ms per rule).
- Natives:
  - `getSimulationTime` bound to the run clock (`register_simulation_time`);
  - the Palladio `ProfilesLibrary` operations as stubs, or real ones if a stereotype model is
    ever needed (decision U8).
- **SimuLizar flavour**: plan D2/D3. PRM values are stored; rules are run by the
  reconfiguration process in file-name order; MEAS-7.2/7.3 timing; `assert fatal` means "not
  applied"; deterministic UUIDs for exactness against a patched refsim (D5).
- **Slingshot flavour**: SPD adjustment as above. The same host can also run SimuLizar-style
  rules in the Slingshot flavour if wanted (no reference; low cost).
- **Exact vs fast** (since merge step M3 the default build is EPL-only, `LICENSES` §7):
  - The default build has only qvto-rs's **fast mode**: Eclipse's semantics with hash-based
    collections in deterministic insertion order (`->any` and the iteration of sets of objects
    follow it). This is a documented deviation from Eclipse's Java `HashMap` and identity-hash
    orders; runs stay deterministic and seed-independent.
  - Builds with the feature `compat` (on through `exact`; no licence covers them) run QVTo's
    **compat mode** by default, so the models equal the Java runner's. Asking for the compat mode
    in a build without it is an error (`ModeUnavailable`), not a silent fallback.
  - Java identity-hash order (Q8, Q16): without a Java run there is no identity hash. The compat
    mode draws HotSpot's generator (seed-independent, not Java-equal); a Java run's hashes can be
    replayed for comparisons (`InvokeHooks::ids`).
  - In the Slingshot-exact mode, the oracle's QVTo and OCL collections iterate in insertion order
    (S0 ordering rule), so qvto-rs needs a matching **insertion-order policy** for `Set`s (Q2).
    The host has the mode parameter for it (`QvtMode::InsertionOrderSets`, mapped to the ordered
    mode below).
  - qvto-rs fast mode is an option for throughput.
  - *As built (Q2):* the policy is QVTo's **ordered mode** (`qvto_runtime::Mode::Ordered`, below);
    the Slingshot-exact mode uses it, and compat mode is not needed by either Slingshot mode (so
    neither needs the GPL-adjacent `simoxide-jcompat-exact`, decision U12).
  - *As built (R2):* SimuLizar's rules use the ordered mode too (`RulesHost` maps `Auto` to it):
    the reference runs QVTo with the same insertion-order rule (refsim patch P10), so rule exactness
    needs no compat mode either.


**F3 as built** (2026-10-17, branch `f3`; guide: `docs/guide/library.md`, "QVTo transformations
on a run"):

- **Crate `simoxide-reconf`** (EPL; feature `spd`, default: the SPD adjuster; `compat`: QVTo's
  compat mode; `exact` = `compat` + `simoxide-sim/exact`). Only `src/qvt.rs` uses the adapter's
  `Session`/`SimStore` for transformation runs (besides F2's `Edit`, whose `store()` is the
  adapter's store): the switch to the shared graph changes that module.
  - `QvtoCore` (`Arc`, `Send + Sync`): a template session (registry with SimOxide's tables and
    ids, metamodel objects, and the metamodels Palladio's analyzers register besides SimOxide's
    tables, embedded: feature model and feature configuration from the 5.2.2 product, which
    SimuLizar's rules declare as model types; the analyzer framework's completions, which the SPD
    transformations use; SimuLizar's runtime measurement model), behind a `Mutex` (the session
    holds a `RefCell` cache: `Send`, not `Sync`). `compile_file(path, Imports::{Folder, Roots})`
    and `compile(UnitSource, &dyn UnitResolver)` (embedded transformations) give a
    `Transformation` (`Arc`, `Send + Sync`) with its parameters (name, direction, nsURIs).
    Palladio's blackbox libraries are registered for every compilation. Not tied to a model:
    one core and one compilation serve all runs, models and threads.
  - `QvtoRun` (per run, `Clone + Send`): empty until the first use; then a clone of the template
    session (resource table, models of other metamodels such as the PRM, mirrors, journal, id
    generator) and a working copy of the run's model graph, recognised by the run model's
    generation and address and copied again when the simulation's model changed otherwise. Forks
    clone it copy-on-write (the session is shared until one side uses it). `RunConfig`: mode,
    `IdPolicy` (seeded UUIDs by default, none for dynamic-EMF comparisons), `AbortPolicy`
    (`FailRun` default, `Reject`), `FailedRunChanges` (`Keep` default: what a failing
    transformation changed stays, as in SimuLizar and Slingshot, which transform in place;
    `Discard`), call-depth limit.
  - `QvtoRun::invoke(sim, t, Invocation)` (generic over `Compat` and `Flavour`, between events):
    binds each model parameter (`Bind::ByNsUri(NsBinding::SimuLizar)`: the first root of each
    model of the parameter's metamodel, the PRM's measurements for the PRM model, as SimuLizar's
    `QVToModelCache`; `NsBinding::Slingshot`: every root, without `PrimitiveTypes.repository`;
    `Bind::Models`, `Bind::Empty`; order: resource order, the Java caches iterate a `HashSet`),
    sets configuration properties, registers `getSimulationTime()` on the run's clock and
    `ProfilesLibrary` stubs (both units; they throw `UnsupportedOperationException` "not supported
    by SimOxide"), runs with the journal on, converts the journal (`change_set`) and applies it
    (`Simulation::apply_changes`). `Outcome`: severity, code, message, `succeeded` (OK or INFO:
    SimuLizar's and Slingshot's "applied"; `assert fatal` makes it false), log, journal, the
    `ChangeReport`, extents, time. Errors: `Binding` (nothing ran, as the Java executors'
    `IllegalStateException`), `Rejected { outcome, error }` (`Invalid`, `Unsupported`, `State`,
    or `Abort` under `Reject`: the run is unchanged), `RunFailed` (`Abort` under `FailRun`: the
    run fails through the new `Simulation::fail_run`, every later `run_until`/`finish` returns
    the error), `ModeUnavailable`, `WrongCore`.
  - Hooks and helpers: `InvokeHooks { before, ids }` (edits before the run in its journal, e.g.
    SPD's `enactedPolicy`; replayed identity hashes), `load_model`/`create_model` (models of
    other metamodels, e.g. R1's PRM), `with_store` (any `ModelStore` operation; graph changes go
    through the change path), `with_scratch_store` (a scratch copy: dumps, summaries),
    `dry_run` (a transformation on a scratch copy, nothing reaches the run).
  - **R1**: `RuleSet::load_dir(core, dir)` (the `.qvto` files, file-name order, imports next to
    the rule), `QvtoRun::run_rules(sim, rules, inv)`: each rule in order, its changes applied
    before the next one runs (each sees the changes before it; SimuLizar's syncers react to each
    notification at once); a failing rule does not stop the others; `RulesOutcome::all_succeeded`
    is SimuLizar's `result &= ...`. The PRM storage, the reconfiguration process's timing and
    `Reconfiguration Time` tuples stay R1's; R1 calls the host at a pause point between events
    (an apply inside `Ev::Reconf` needs an engine hook, 5.3).
  - **P2**: `QvtoAdjuster` implements `simoxide_spd::Adjuster<C, F>` (the trait is generic over
    the flavour now; `SpdRun` still drives the SimuLizar flavour): Slingshot's bindings,
    `keepModeling = true`, `AdjustHooks { before, after }` (default: nothing before; `ok` = the
    transformation succeeded, membership refreshed from the changed graph, group sizes); a
    `RunFailed` ends the SPD run with the run's error. `fork` copies the QVTo state.
- **Costs** (release, shared machine, `tests/host.rs costs`): core 2.6 ms; compiling a rule
  0.4 ms; the per-run state on first use 0.8-1.4 ms (session clone, graph copy); a fork of it
  below 1 µs (copy on write); a rule run 25-100 µs; a change 0.12-0.16 ms (h19). 2 000
  invocations do not grow the session (`repeated_runs_do_not_grow_the_state`).
- **Tests.** `tests/corpus.rs` (exit criterion): the 24 SimuLizar/SPD corpus cases run on the
  `RunModel` of a simulation of the case's models at t = 100 s; a dry run's models (dump of the
  Java runner's resources), log and status equal the Java runner's (with the Java run's identity
  hashes, `target/qvto-testkit/java`; without them only the journals and the change path are
  checked, and the SPD case aborts, see below; the SPD dump is marked unstable in the fixture,
  as in the adapter's test);
  the run's journal equals the dry run's and the journal of the same transformation on the
  case's models loaded on their own (the adapter's set-up), object by object; the change path:
  10 cases applied, 12 change nothing, `addNewServer_high` and `addClonedServer_high` refused
  (`Invalid`: the rule adds an assembly context without allocating it, which SimOxide's model
  validation rejects; SimuLizar would only fail when a request reaches it), and every run then
  finishes. `tests/host.rs` (16): state created on first use, a plain run unchanged; a rule
  mid-run equals the same change as a direct edit, trace equal before the change; forks before
  and after a rule (equal continuations, the id sequence continues in both; a fresh state would
  repeat the first id); snapshot, detach to another thread and resume with the QVTo state:
  equal continuation and ids; an aborting rule (removing an allocation context, MC-3.3) fails
  the run, or under `Reject` leaves it byte-identical to the uninterrupted run; `assert fatal`
  "not applied" with the change kept, or dropped under `Discard` (then byte-identical); a rule
  folder in file-name order (the second rule sees the first's change, the third fails, the
  others count); `ProfilesLibrary` stub; binding and core checks; fast and compat give the same
  journal; the Slingshot flavour runs a read-only rule and refuses a change (`Unsupported`, S2);
  id policies; no growth over 2 000 runs; the SPD minimal example end to end (`SpdRun` with
  `QvtoAdjuster` and a hook setting `enactedPolicy`): `MainTransformation` adds `AppServer_1`
  at t = 50 s, the run model's generation is 1, the target group has 2 elements, as the
  reference runner's `adjust` record.
- **Gates.** `golden.sh` identical (312 runs, `cargo build-exact`, against b231a95);
  `cargo test --release --workspace --features exact` 543 passed, 0 failed (524 before; +19 in
  `simoxide-reconf`); `simoxide-fuzz corpus` 132/132 and corpus-fuzz 6/6; `cargo conform
  --suite all` 542/542 strictly equal (unchanged); fmt and clippy `-D warnings` clean (workspace,
  all targets, default and `--features exact`; `simoxide-reconf` without default features;
  `simoxide-cli --features reconfiguration`); release check: `cargo tree -e normal` of
  `simoxide-cli` (also with `reconfiguration`) and `qvto-cli` lists no opt-in crate. The hot
  path is unchanged (`simoxide-sim` gains only `fail_run`); a quick benchmark showed no
  slowdown (timing deferred).
- **Found for P2/F4.** In compat mode without Java's identity hashes (HotSpot's generator),
  `MainTransformation`'s wiring *moves* a container within
  `LinkingResource.connectedResourceContainers`; SimuLizar's router throws on that (MC-2.3), so
  the SimuLizar flavour's SPD run fails. With the Java run's hashes (corpus) and in fast mode it
  does not happen. The SimuLizar flavour's SPD needs the fixed router semantics there (U13: a
  move changes nothing for routing). The end-to-end test runs in fast mode.
- **Adapter API added** (`qvto-simoxide`): `#[derive(Clone)]` on `Session` (and its state).
- **What the switch to the shared graph (M4-M6) needs.** `qvt.rs` only: the template becomes
  the registry (`Arc<MetaRegistry>`) plus a graph prototype holding the embedded metamodels; the
  run session becomes the run graph's journal, id policy and resources (a resource for the PRM);
  `SimStore` becomes the graph (`ModelStore` on `simoxide_emf::Graph`); the working copy and its
  fingerprint stay (or become the run's copy-on-write graph, 5.9). `ModelRef::Loaded` indexes
  the store's resource table and becomes a graph `ResourceId`. The public API (`QvtoCore`,
  `QvtoRun`, `Invocation`, `Bind`, `Outcome`, `RuleSet`, `QvtoAdjuster`) does not change except
  the store type in the closures of `with_store`, `with_scratch_store`, `dry_run` and the hooks
  (the adapter's `SimStore` → the graph). `Edit` changes the same way.
- **Switched in merge step M6** (2026-10-17, branch `qvto-merge`): `qvt.rs` has a `Template`
  (a `ResourceSet` whose graph is the prototype: SimOxide's registry, Ecore, the metamodel objects
  and the embedded metamodels) and runs on `&mut simoxide_emf::Graph`; `Template::attach` appends
  the prototype to a run's graph once (`Graph::append_graph`) and sets its id policy, so the
  graph carries the QVTo state (journal, UUID generator, PRM and other models as graph
  resources). `PerRun` keeps the working graph behind an `Arc` (copy on write, shared with forks,
  fingerprint: generation, compiled model and model graph); when a run or `with_store` changes
  only objects of other metamodels, the working graph is handed to the simulation
  (`Simulation::set_model_graph`, no recompilation), so such models live with the run's model.
  API changes: closures take `&mut Graph` (and `IdSourceFn` `&Graph`); `Outcome::extents` and
  `Outcome::journal` use `ObjId`; `ModelRef::Loaded(ResId)`; `QvtoRun::load_model` and
  `create_model` take `&mut Simulation`; `Edit::new(&sim)` (no session; `Edit::store()` is the
  graph); `change_set(&graph, &journal)` (objects of other metamodels are left out by class);
  `QvtoCore::attach`, `load_model`, `operations`, `derivations` for use outside a simulation.

**Q1 as built** (2026-10-17, branch `q12`; `crates/qvto/qvto-testkit/README.md`, "SPD corpus"):

- **Corpus.** Suite `spd` of `qvto conform` (`cases/spd.cases`, 47 cases), models in
  `corpus/spd/q1`, generated by `qvto-testkit/tools/gen-spd-corpus.py`:
  - base `min`: the minimal example (an elastic infrastructure whose unit hosts a service group
    behind a load balancer) with an SPD of 17 policies; one semantic configuration per case
    (the enacted policy, or a broken configuration);
  - base `full` (built for the corpus): the unit of the elastic infrastructure hosts a
    service-group unit (load balancer whose branch call has input variable usages, an assembly
    connector to a database, a required delegation to the system, an infrastructure connector
    to a middleware, a required infrastructure delegation) and a competing-consumers unit (a
    queue, a consumption scenario with one branch, a closed workload of 4), a nested container,
    two linking resources, an elastic infrastructure without services, one without
    configuration; 17 policies;
  - six chained states: a case's outputs (Java runner with the `det` rule, so reproducible)
    become the inputs of later cases, as Slingshot's consecutive adjustments; objects the first
    run created get ids (`_q1_<state>_<n>`, as Slingshot's generated classes give them UUIDs),
    except in `min_abs_3-noid`; path references are rewritten to ids.
  - Paths: bottom-up scale-out and scale-in (step, relative incl. 0 % (Q4) and a minimum
    adjustment, absolute incl. the fatal goal 0, the size clamp at both bounds, the fatal
    constraints min 0 and max = min, scale-in at the minimum, by more than the size), service
    group and competing consumers bottom-up and top-down (scale-out, scale-in, consecutive),
    top-down placement among free containers (Q8), the late removal of emptied containers (Q7,
    a chain of two scale-ins), no change (no disjunct of `modifyAllocation` applies), an empty
    service group, the preconditions (no policy, no repository, no elastic-infrastructure
    configuration), an elastic infrastructure without configuration or without services.
- **Coverage** (`tools/spd-coverage.py`, from the VM's operation counts `RunOptions::coverage`):
  73 of the 105 operations of the 17 units are reached: every mapping, helper, query and
  constructor in `MainTransformation`'s call graph. The other 32 are not in it: 11 helpers of
  `Commons` nothing calls, the constructors only `BasicComponent`'s constructor and
  `LinkingResource` call (both never called), `connectLoadBalancerAndAssemblyContexts` and its
  `get*AssemblyContext` queries, the empty `createDelegationConnectorsForProvidedRoles`, the
  `InfrastructureInterface` overload of `createAssemblyConnectorsBetween…` (the call passes an
  `OperationInterface`).
- **Result.** Compat mode: 47 of 47 strictly equal to the Java runner, also on what varies
  between Java runs (`--compare-unstable`; the engine replays the recorded run's identity
  hashes). The gates: `cargo conform --suite all` 589 of 589 (542 + 47).
- **Discrepancies found and fixed** (DESIGN Q1-1 to Q1-4 in `crates/qvto/DESIGN.md`):
  1. *Engine:* Java hashes objects by identity as side effects qvto-rs did not reproduce:
     `ModelParameterExtent`'s `ResourceModificationAdapter` records every object added to an
     extent's in-memory resource in a `LinkedHashSet` (every created object, at creation), and
     EMF's `Copier` (`deepclone`) hashes the originals, every reference target, and the copies
     it adds to many-valued containments (`addAll` de-duplicates through a `LinkedHashSet`).
     With HotSpot's per-thread sequence one missed draw shifts the hash of every later object, so
     `Set` orders (containers, assemblies, allocations) differed in 36 of 47 cases. Now drawn at
     the same points (`RunEnv::touch`).
  2. *Harness:* the identity-hash tape keyed input objects by their path after the run, so an
     inout list the run reorders (the resource environment) gave moved objects the wrong hashes;
     the tape now has `pre` lines keyed before the run. The first case of a batch drew hashes
     before its seed (an unrecoverable seed), and the search for the run's hashes in the thread's
     sequence stopped at 20 000 draws (compiling the SPD transformations alone takes about
     10 000).
  3. *Harness:* EMF resolves cross-document path proxies lazily, when first used, which after an
     adjustment can be another object; qvto-rs and Palladio resolve at load. Case key
     `resolve-all` (the Java runner resolves the inputs before the run).
  4. Not a discrepancy: in dynamic EMF (the Java runner, qvto-emf) created PCM objects have no
     `id`; `Wiring`'s `.id` comparisons then treat all created objects as equal in a later
     adjustment (`min3_noid_*` cases). SimOxide must give them ids (P2).
- **SPD findings** (candidates for the quirk list): `RequiredDelegationConnector`'s constructor
  takes its parent from `resolveone(System)`, which is `null`: the connector of each replica is a
  root of the allocation model and points to a removed assembly after a scale-in; the name counter
  restarts with every adjustment, so consecutive scale-outs create containers of the same name
  (`AppServer_1` twice; also in the S0 run of `spd_time_policies`).

**Q2 as built** (2026-10-17, branch `q12`; `crates/qvto/DESIGN.md` §6.4):

- **Policy.** `qvto_runtime::Mode::Ordered`, next to `Compat` (Java `HashMap` order) and `Fast`:
  Eclipse's semantics, with QVTo's and OCL's `HashSet`/`HashMap` replaced by
  `LinkedHashSet`/`LinkedHashMap`, exactly as the S0 seed agent rewrites them (`det`): `Set`,
  `Bag` and `Dict` iterate in insertion order, `->any`, `asSequence`, `asSet` and every loop
  follow it. As in the `det` JVM, a `Set` *is* a `LinkedHashSet`, which classic OCL takes for an
  OrderedSet (`instanceof LinkedHashSet`: ordered equality, `asSet` copies, printing); the
  Eclipse suite under `det` showed it (6 `bug467600_*` cases whose own assertions fail under the
  ordering rule, e.g. `Set{1, "2"}->union(Set{"1", "2"}) = Set{1, "1", "2"}` is false). An
  effectful `sortedBy` body is evaluated twice as in compat mode. Identity hashes only print in
  `Class@hash` texts: derived from the key unless the host gives a source.
- **EPL only.** No Java hash order is involved: `Mode::Ordered.available()` in every build; the
  release check (`cargo tree`) is unchanged.
- **API** (`crates/qvto/README.md`, "Building and testing", modes): `qvto::RunOptions { mode:
  qvto::Mode::Ordered, .. }` (`qvto::Mode` is `qvto_runtime::Mode`; `Mode::from_name("ordered")`
  for configuration; `RunOptions::id_source` optional); `qvto_runtime::OrderedEnv` for
  runtime-level use; `qvto run --ordered`, `qvto conform --engine qvto-ordered`, `qvto bench
  --modes ordered`.
- **`det` Java runner.** `reference/oraclekit/seed-agent.sh` builds the seed agent alone;
  `qvto conform record --det` runs the QVTo Java runner with `-javaagent:seed-agent.jar=ordered,qvto`
  (fixtures `java-<suite>-det.txt`, cache `target/qvto-testkit/java-det`).
- **Exit met.** Ordered mode against the `det` runner: all 589 cases (Eclipse 494, corpus 24,
  fuzz 24, SPD 47) equal strictly, 6 of them where the `det` runner fails the Eclipse checks
  itself (verdict `java-fails-equal`); with `--compare-unstable` all but 2 (a timing log, a log of
  objects the tape does not have). The SPD corpus byte for byte, incl. container and link order
  (Q16). Against Slingshot itself: `qvto spd-replay` replays the 13 adjustments the S0 runs
  recorded in `det` (`spd_time_policies` 8, `spd_minimal` 1, `spd_scale_in` 2, `spd_or_trigger`
  1, `spd_and_trigger` 1) on one in-memory copy of the models; containers, links, allocation and
  target-group elements in their order equal all 13 (compat mode 6, fast mode 13).

**R1 as built** (2026-10-17, branch `r1`; spec: MEAS-7.2, 7.3, 7.5 in `docs/spec/measurements.md`,
MC-7 in `docs/spec/model-changes.md`; guide: `docs/guide/reconfiguration.md`):

- **Status: done**; the F4 remainder is listed below. Every runnable SimuLizar reconfiguration
  example runs and equals the patched SimuLizar reference byte for byte (trace, tape,
  measurements) in the exact build, and matches it statistically in the default (EPL, fast)
  build.
- **D2, PRM values (`simoxide-sim`, `src/sim/reconf.rs`, `src/stats.rs`).** Every PRM recorder
  owns a runtime measurement (`CompiledModel::prm_meas`: monitor MP and spec as graph objects,
  SimuLizar's order: fixed-size, variable-size, feed-through, time-driven aggregation, utilisation
  windows; lazily created passive-resource recorders append theirs) and writes its value into
  `Prm::values`, plain data in the run state's sink, so forks, snapshots and resumes carry it.
  Values: feed-through = the tuple; fixed and variable size = the statistical characterisation of
  the buffer (now buffered as `(t, v)`, fixed size in SimuLizar's array order); time-driven
  aggregation = the characterisation of a sliding window fed by the calculator of the spec's
  metric (before R1 these windows only produced their events); utilisation windows = the
  utilisation. The four characterisations (arithmetic, geometric, harmonic mean, median) were
  decompiled from the 5.2.2 monitor repository jar and ported with Java's arithmetic
  (`Collectors.averagingDouble` of JDK 21, JScience `Amount`, HotSpot `log`/`exp` with
  `hotspot-math`). The `Number of Resource Containers` calculator now takes PRM recorders too
  (the load balancer example aggregates it). Runs without rules: unchanged (golden hashes).
- **D3, the reconfiguration process.** *Decision: the rules run between events, not inside
  `Ev::Reconf`.* With a host attached (`Simulation::attach_reconfiguration`, run state), the
  `Ev::Reconf` event only marks the run of the process as due (`RunState::reconf_pending`); a
  driver that runs with `run_while(|s| !s.reconf_pending())` gets control right after that event,
  runs the rules (their changes go through `apply_changes`, safe between events) and reports the
  result (`Simulation::reconf_done`: the container-count tuple if the count changed, then the
  `Reconfiguration Time` tuples, both only on success). Nothing else happens in that event, so
  this equals SimuLizar's in-event run; no engine hook (with its borrow and fork problems) was
  needed. Runs after the stop, synchronous in SimuLizar (MEAS-7.2): `finalise` became a sequence of
  steps (each final window, each resource's deactivation, drain, passive cleanup) that pauses
  after the step whose PRM write triggered a run; the host runs the rules on the PRM values of
  that write; REF-11 behaves as before. Without a host every run succeeds without rules, as
  before.
- **Host (`simoxide-reconf`, `src/process.rs`; F3's public API only).** `RulesHost`: the run's
  `QvtoRun`, the `RuleSet`, a PRM mirror (a `RuntimeMeasurementModel` in the run's QVTo state,
  created on the first reconfiguration, one `RuntimeMeasurement` per run-state measurement
  referring to the graph's MP and spec objects; values written before every run, 15 µs), stats
  and optional records. `handle` = mirror, `QvtoRun::run_rules`, `reconf_done`. SimuLizar's result:
  a rule is applied when it *completed* (diagnostic code 0): QVTo 3.10.7, which SimuLizar 5.2.2
  ships, reports OK for a completed run even when a non-fatal `assert` failed; qvto-rs follows
  QVTo 3.11, which raises such a run to ERROR (found on the load balancer's `outsource`: 99
  successful reconfigurations in SimuLizar). Errors (a binding error, a change SimuLizar aborts on,
  a change SimOxide refuses: F4 kinds, invalid models) fail the run with the rule named.
  `RulesRun`: simulation + host; `run_until`, `finish`, `fork`, `into_parts`/`from_parts`.
- **Configuration.** `run.json` `reconfiguration_rules` (`RunSpec::reconfiguration_rules`);
  `simoxide run --rules DIR --rules-log FILE --qvto-mode auto|fast|compat` (feature
  `reconfiguration`).
- **Reference.** `refsim --rules DIR` (and the `run.json` key), `--seeds A:B`. Patch P9: the QVTo
  loader lists the folder itself in file-name order (stock: a `HashMap<URI, …>` whose order
  depends on the folder's location), the rule factory compiles a `DeployedQvtModule` (the stock
  resource asks for the Eclipse workspace), `Bootstrap` registers QVTo's `.qvto` factory. Only the
  QVTo reconfigurator is active in the 5.2.2 runtime (Henshin and Story Diagram engines use
  extension points its Dagger components no longer read).
- **Examples** (`corpus-rules/`, `reference/import-rules-examples.sh`, expected outputs of the
  patched reference): `lb_outsource` (the load balancer example's launch configuration: branch
  probabilities), `lb_rules_qvto` (its QVTo folder: branch probabilities and processing rates),
  `lb_scaleout` (SimOxide's `scaleOut.qvto`: a new container with resources joins the LAN, a new
  assembly context is allocated to it, the connector is rewired), `lb_add_branch` (SimOxide's
  `addBranch.qvto`: a third branch in the load balancer's SEFF that routes load), `power_rules_qvto`
  (the power-consumption variant with its PCM-only rules; its power monitors need the Palladio
  power extension, which the 5.2.2 product lacks, and its resource-URI measuring points refer to
  the Eclipse workspace: replaced by the PCM measuring points they denote), `sl_reconfiguration`
  (SimuLizar's `ReconfigurationSimulationTest`, response times 1..10). All six: byte-identical in
  the exact build (QVTo compat and fast mode alike); 13 349 reconfigurations in the power case.
- **Not runnable, and why** (also not headless in SimuLizar 5.2.2): rules that use SimuLizar's
  action framework (`AdaptationBehavior.execute`: the power example's `switchToStandby` and its
  `outsource`, which also needs the power model; the TransientEffects media store's
  `horizontalscaling`, which also needs usage evolution); PCM 5.0/5.1 namespaces (znn.com, trip
  planner, the old power-consumption example); Henshin and Story Diagram rules;
  `runconfigtest`'s `addNewServer`/`addClonedServer` add an unallocated assembly context (refused by
  SimOxide's validation; in simulation their thresholds are never reached anyway).
- **What needs F4** (per change kind; none of the runnable examples needs it): monitor changes
  (cloning a monitor or adding a PRM recorder: `ChangeError::Unsupported`, the run fails naming the
  rule, test `a_change_simoxide_does_not_support_yet_ends_the_run_naming_the_rule`); population and
  usage-scenario changes (MC-5.2); executions in progress inside a changed behaviour (MC-4.3: they
  finish their old instructions, SimuLizar follows the changed chain; for the load balancer's
  branch, the SEFF's first action, it makes no difference: `lb_add_branch` is byte-identical);
  structural changes of an action itself (its kind or its number of resource or infrastructure
  calls); removal policies beyond SimuLizar's "keep simulating" (MC-1.5, which R1 has). Of
  SimuLizar's examples, those that would need F4 also need the action framework: the media
  store's horizontal scaling (a new assembly context wired into the load balancer by adding a
  branch, `wirewithloadbalancer.qvto`; monitors of the new container), `migratecomponent.qvto`
  (moves an allocation context: ignored by SimuLizar's syncer, MC-3.2).
- **Statistics** (`reference/tools/rules-stat.sh`, 30 seeds per side, Welch and Mann-Whitney on
  per-run counts, means, time-weighted means, percentiles, KS on the pooled values, Holm 0.01):
  the exact build equals the reference on 30 of 30 seeds of every case (measurements byte for
  byte). The default build (fast simulator mode, QVTo fast mode) against the reference: no per-run
  statistic rejected on `lb_outsource`, `lb_rules_qvto`, `lb_add_branch`, `power_rules_qvto`,
  `sl_reconfiguration`; on `lb_scaleout` Server2's mean response time was rejected with seeds
  1-30 (3 tests; the scale-out time is heavy-tailed) but not with seeds 31-60, nor do the
  exact build's seeds 31-90 differ from the fast build's (median scale-out 280 vs 250 s). The
  pooled-values KS rejects on some series (11 of 164 tests), as it does in a control that compares
  the reference with itself (seeds 1-30 against 31-60: 10 of 164, all pooled KS): a run's values
  are autocorrelated, so the pooled KS is no valid test here.
- **Tests.** `simoxide-reconf/tests/rules_process.rs` (9): the PRM as the rules see it on the seven
  `t_reconf_*` models (fixed and variable size, time-driven aggregation, multi-core utilisation,
  lazily created passive recorders, a run after the stop, a late first run) against the
  reference's logs (`tests/prm/`, rule `dump.qvto`, `regen.sh`), outputs byte-identical; an empty
  rule set equals no rules (10 models), no QVTo state without reconfigurations; the six examples
  byte for byte; forks before and after the scale-out and while a reconfiguration waits; a
  detached snapshot with its host continuing on another thread; F4 refusal. `simoxide-sim`:
  `stats.rs` unit tests (JDK 21 averaging checked on the JVM).
- **Costs.** PRM mirror 15 µs per reconfiguration; a rule run about 0.1 ms in QVTo's fast mode and
  1 ms in compat mode (the compat mode's per-run set-up, qvto-internal; the power case: 4 s fast vs
  37 s compat for 40 000 rule runs). The fast build runs `lb_outsource` with rules in 10 ms.
- **Gates.** `golden.sh` identical (312 runs, `cargo build-exact`, against bae975d's binary);
  `cargo test --release --workspace --features exact` 555 passed, 0 failed (543 before: +3
  `stats`, +9 `rules_process`; QVTo's ignored reference material linked from the merge checkout
  for the run); `simoxide-fuzz corpus` 132/132 and corpus-fuzz 6/6; fmt and clippy `-D warnings`
  clean (workspace, all targets, default and `--features exact`; `simoxide-cli --features
  reconfiguration`, also with `exact`; `simoxide-reconf --no-default-features`); release check:
  `cargo tree -e normal` of `simoxide-cli` (also with `reconfiguration`) and `qvto-cli` lists no
  opt-in crate. Quick timing of runs without rules (exact mode, long runs, medians of 5, shared
  machine): within noise on models without PRM recorders; models with a time-driven aggregation
  monitor pay for its window now collecting tuples and computing its statistic
  (`t_reconf_tda` −8.7 %, `u_mediastore_own_monitors` −5.2 %, `h28_triggers_default` −2.8 %).
- **Changes outside `simoxide-reconf`** (for the merge agent): `simoxide-sim`: `src/sim/reconf.rs`
  (new: PRM state and recorders moved out of `sim.rs`, the host API), `src/stats.rs` (new),
  `sim.rs` (start: PRM attach and container count; dispatch of `Ev::Reconf`; `emit` passes the
  value; `window_full` computes TDA statistics and writes PRM values; `finalise` as steps with
  `deactivate` and `drain`; `advance` resumes a paused stop), `ir.rs` (`PrmMeas`, `prm_meas`,
  `PrmRec` with statistics, `prm_series` with the measurement index, `CWindow::{prm_meas, stat}`,
  TDA window inputs, PRM order), `change.rs` (keeps and compares `prm_meas`), `windows.rs`
  (`Amount::divide_long` public), `config.rs` (`RunSpec::reconfiguration_rules`), two RunSpec
  literals in tests. `qvt.rs`, `host.rs`, `rules.rs` and `simoxide-model` are unchanged.
- **R2 prerequisites (D5).** (1) Deterministic ids: SimuLizar's generated code gives every new
  `Identifier` an `EcoreUtil.generateUUID()`, also objects it creates outside rules (the PRM model
  and its measurements at initialisation and at passive-calculator registration, possibly per-run
  objects), so refsim needs a seeded generator that SimOxide reproduces draw for draw, or one that
  counts only draws inside rule executions (SimOxide's mirror then must not draw for PRM objects);
  the R1 examples that create objects set explicit ids. (2) Java identity hashes for QVTo's compat
  mode where a rule's result depends on the order of a `Set` of objects (`->any`, iteration): the
  reference with insertion-ordered sets (as Slingshot's `det` rule, Q2) or replayed hashes. (3) The
  order of passive-resource PRM measurements (SimuLizar: a `HashSet` of suppliers). (4) The
  cross-series order of `meas` lines of a run after the stop within its finalisation step. (5) New
  containers' ids decide the resource registry's `HashMap` order at the stop (follows from 1).

**R2 as built** (2026-10-17, branch `r2`; spec: MC-7.6 in `docs/spec/model-changes.md`, MEAS-7.5;
reference: patch P10 in `docs/reference-simulator/patches.md`; guide: `docs/guide/reconfiguration.md`):

- **Status: done.** All eleven `corpus-rules/` cases (R1's six and five new ones whose rules create
  objects without explicit ids, one of them depending on the order of a `Set` of model objects)
  equal the patched reference byte for byte (trace, tape, measurements) in the exact build, on 30 of
  30 seeds each; R1's six reference outputs did not change.
- **Id scheme (D5, prerequisites 1 and 5).** *Decision: one seeded generator per run that draws only
  while a rule runs.* An id is drawn for every new `Identifier` (generated PCM code's constructor:
  also for objects whose id the rule then sets, and for every `deepclone()` copy, which then takes
  the original's id); SplitMix64 seeded with `0x5EED0F51` (`RunConfig::default()`), two draws per id,
  in `EcoreUtil.generateUUID`'s encoding. This is F3's existing generator, unchanged; Java gets a
  30-line port (`reference/src/refsim/Ids.java`). Why not every draw: SimuLizar draws UUIDs for
  thousands of objects outside rules (model loading, the PRM at initialisation and at passive
  registration, a system assembly context per call), which SimOxide would have to replay draw for
  draw; counting only rule runs needs one window in the reference and nothing outside the QVTo host
  in SimOxide. The new containers' ids then give the stop's deactivation order (prerequisite 5,
  `lb_scaleout_noid`).
- **Set order (prerequisite 2).** *Decision: insertion order in the reference, QVTo's ordered mode in
  SimOxide.* refsim loads the Slingshot reference's seed agent with `ordered,qvto` (QVTo's and OCL's
  `HashSet`/`HashMap` become `Linked*`), the rule qvto-rs's `Mode::Ordered` reproduces (Q2). Replayed
  identity hashes would need a Java run per SimOxide run. `RulesHost` maps `QvtMode::Auto` to the
  ordered mode, so **every build, the default EPL build included, runs rules in the reference's
  order**; `--qvto-mode ordered|fast|compat` (CLI) selects otherwise. Compat mode (HotSpot's
  generator) differs on `lb_scaleout_each` (the servers move in the other order), fast mode does not.
- **Passive-resource PRM order (prerequisite 3).** The reference's `DeferredMeasurementInitialization`
  keeps its metric descriptions and suppliers in insertion order (P10). SimOxide's order (monitor
  order) equals it unless one lazily created calculator gets recorders of different metric
  descriptions deferred interleaved (the reference groups them by metric description); no case has
  it, and fixing it would change `PassivePrm`, part of N1's snapshot format: left as a deviation.
- **Post-stop tuple order (prerequisite 4).** Unchanged: a run after the stop still runs when the
  finalisation step that triggered it ends, so its tuples can come later in the trace than
  SimuLizar's within that step (MEAS-7.2). None of the eleven cases hits it; making the deactivation
  step pausable per tuple changes `FinStep` in the run state (N1's format). Left as a deviation.
- **Reference (patch P10).** Shadowed `IdentifierImpl` (Core Commons, unsigned: EMF's `EcoreUtil` is
  in a signed jar) calls `refsim.Ids.generate()`: seeded inside the window, EMF's UUID outside;
  shadowed `QVTOExecutor` opens the window around each rule; `Statics` resets the generator per run;
  `build.sh` builds the seed agent with the product's ASM, `refsim` loads it (`REFSIM_ORDERING=stock`
  without); shadowed `DeferredMeasurementInitialization`. Evidence: R1's six cases and the
  `tests/prm` dumps unchanged; the 66 corpus models unchanged (`regen-expected.sh --check`); without
  `IdentifierImpl` `lb_scaleout_noid`'s trace changes, without the agent `lb_scaleout_each`'s.
- **SimOxide changes** (for the merge agent; small and delimited): `simoxide-reconf/src/process.rs`:
  the PRM mirror creates its objects inside `without_ids` (the session's generator paused through
  N1's `Session::id_state`/`set_id_state`: no draw, no id), `RulesHost::new` maps `Auto` to the
  ordered mode; `simoxide-cli`: `--qvto-mode ordered`. No new run state: the id generator is N1's
  `SessionState::ids`, and the mirror no longer draws, so a restored host's rebuilt mirror does not
  disturb it. The adapter, `qvt.rs`, `host.rs` and `simoxide-model` are unchanged. On the shared
  graph (M6, merged into `qvto-merge`): `without_ids` pauses the run graph's id policy
  (`Graph::id_state`/`set_id_state`), and a restored host keeps the mirror's objects (the
  snapshot's QVTo state) instead of rebuilding them.
- **New cases** (`reference/rules-examples/`, `reference/import-rules-examples.sh`):
  `lb_scaleout_noid` and `lb_add_branch_noid` (R1's rules without ids), `lb_scaleout_each` (every
  server to a new node in the order of a `Set` of allocation contexts), `lb_wire_new_server`
  (SimuLizar's horizontal-scaling adaptation steps `instantiatecomponent.qvto` and
  `wirewithloadbalancer.qvto` as a rule: container, assembly and allocation context, a required
  role, a branch with three actions, a connector), `lb_clone_branch` (`deepclone()` of a branch
  behaviour: the copies draw ids, the new branch gets the next one).
- **Default EPL build** (fast simulator mode, ordered QVTo): the same rule decisions and ids for the
  same PRM values, statistically equal (`rules-stat.sh`, 30 seeds per side): on the new cases only
  the pooled-values KS rejects (invalid here, as the reference-against-itself control shows) and, in
  the two scale-out cases with seeds 1-30, Server2's mean response time, as R1's `lb_scaleout`; seeds
  31-60 reject nothing but the pooled KS.
- **Gates.** `golden.sh` identical (312 runs, `cargo build-exact`, against `spd` a3ffa38);
  `cargo test --release --workspace --features exact` 588 passed, 0 failed (586 after the N1 merge,
  +2 in `rules_process`; the snapshot test now continues all eleven cases from snapshots);
  `simoxide-fuzz corpus` 132/132 and corpus-fuzz 6/6; fmt and clippy `-D warnings` clean (workspace,
  all targets, default and `--features exact`; `simoxide-cli --features reconfiguration`, also with
  `exact`); release check: `cargo tree -e normal` of `simoxide-cli` (also with `reconfiguration`) and
  `qvto-cli` lists no opt-in crate. Timing (quick): rules run as fast as in R1's fast mode; the exact
  build's default is now the ordered mode instead of compat, so `power_rules_qvto` spends 1.4 s in
  rules instead of 42 s.

### 5.6 Metamodel tables and the generic-supertype bug (early fix)

- SimOxide's main tables miss two things:
  - the generic supertypes of `IntSample`, `BoolSample`, `DoubleSample` and `EnumSample`
    (`extends Sample<T>`). `meta/generated.rs:703-711` gives `IntSample` no supertypes and no
    features, so `value` and `probability` are unknown to the loader;
  - `PCMBaseClass`'s explicit `EObject` supertype.
- The extension tables of `qvto-adapter` have them. Fix in `tools/gen_meta.py`:
  - read `eGenericSuperTypes`;
  - regenerate; check that existing ids do not change;
  - add a loader test with an `IntSample` that has a value;
  - golden hashes unchanged.
- Add SPD (`SPD.ecore`, 9 packages), `semantic.ecore` and `spdmeasuringpoint.ecore` to the
  generated tables. Packages are **appended**, so existing ids stay.
- SPD objects then live in the shared graph (one graph, one journal).
- With the merge, the generated tables become the foundation registry. Packages from `.ecore`
  files loaded at run time (PRM, user metamodels) are appended to it.
- Fallback while the Q6 adapter is the bridge: load SPD into the adapter's other home, as the
  corpus test does today. That works, because SPD only refers to PCM objects, never the reverse.

### 5.7 Run state, snapshots and forks (shared)

**Principle.** All mutable state of a run is one value, `RunState`. The engine borrows it
together with the immutable `Arc<CompiledModel>`. Then:

- **fork** = `RunState::clone()`. It is in memory and cheap; large parts are shared through
  `Arc`/copy-on-write;
- **snapshot to disk** = `serde` of `RunState` plus the model overlay as XMI;
- **resume** = deserialize, apply changes, continue.

`RunState` holds, per area:

| Area | Contents | Clone cost |
|---|---|---|
| clock, event list | `now`, notes `(t, seq, kind)`, `seq` counter, `same_instant`, stop state | O(events) |
| processes | `procs` (frames as `Rc<Frame>` chains), free lists, join tables, pids | O(processes); frames must become `Arc` or be deep-copied (see below) |
| resources | `ResRt` per resource: PS/FCFS queues, remaining demands, last update times, passive tokens and waiters | O(jobs) |
| RNG | MT19937 state, or the fast generator state, per stream; tape position | small |
| measurements | series rows (or counts), sliding-window buffers and bounds, calculator start maps, PRM values | O(stored rows) unless rows are flushed first; option to fork without past rows |
| model | `RunModel` overlay: graph `Arc` (copy-on-write), appended or tombstoned entities, journal cursor | O(1) until written |
| reconfiguration | reconfiguration-process state (`last_ns`, scheduled, a waiting run, a paused stop), PRM values (R1) | small |
| SPD | `SpdState` (adjustor, group, aggregators, incl. EMA and windows) | small |
| limits | step and event counters | small |

**Exactness of resume.**

- **Definition**: snapshot at event boundary *k* at time *t*, resume without changes → the
  continued run's trace, tape and measurements equal those of the uninterrupted run, byte for
  byte, in **both flavours and both RNG modes**. This is stronger than Slingshot (§3).
- It holds because nothing is rebased. Times stay absolute, RNG state is captured, and
  aggregators and windows are captured.
- **Test**: for every corpus model and a set of *k* (random event indexes, plus instants with
  same-time events, during a reconfiguration, during a fork join), compare the trace hashes.
  Also check `clone → run both → equal`, and `serialize → deserialize → equal`.
- **With changes**:
  - between pause and resume, a change set is applied through the change path of 5.3:
    a QVTo transformation, a direct edit, enforced policies, or an SPD or rule-set change;
  - then the syncers run; then the run continues.
  - Result: "uninterrupted run of the changed model from *t* on, with the same random state".
- **Enforced policies** ("incomingPolicies"): adjustment events are queued at resume time and
  bypass triggers and constraints, as `SnapshotInitFromBehaviour.java:118-127` does. Optionally
  the constraint state is updated as if they had fired, which is Slingshot's behaviour (Q2).
- **Snapshot conditions**, as hooks:
  - maximum duration;
  - a policy trigger fired (snapshot *before* the adjustment, carrying the request, like
    `SnapshotTriggeringBehavior`);
  - a measurement close to a threshold;
  - an SLO violation (abort).
- **Slingshot `.snapshot` interoperability** (Slingshot flavour, optional, phase N3):
  - Export:
    - SimOxide's state is mapped to Slingshot's event list: users and their positions as
      `UsageModelPassedElement`/`SEFFModelPassedElement` start events with offsets; jobs as
      `JobInitiated` with remaining demand;
    - think and arrival timers as `ClosedWorkloadUserInitiated`/`InterArrivalUserInitiated`;
    - adjustor states with shifted times.
  - Import is the reverse.
  - It is **lossy by design**, like Slingshot's own (RNG, windows, aggregators, same-time
    order). It is useful for MENTOR tooling, not for exactness.
  - Feasibility risk: the gson format serialises Slingshot's internal entity classes
    (`[INIT]README.md:440-600`: dynamic types, reference ids, PCM element references). Mapping
    SimOxide's interpreter positions (instruction pointer, frame chain) to Slingshot's
    "model element passed" events must be checked for every action type. Do a spike first (N3a).
- **Implementation notes.**
  - `Rc<Frame>` and other `Rc`s in the run state become `Arc`, or are deep-copied on clone.
    Today's `Rc` is single-threaded and cheap; measure both.
  - The `PassiveInst`, `CompInst` and `ProvRes` tables are plain data already.
  - Snapshots are taken only **between events**. A "pause at t" request runs every event with
    time ≤ t (or < t, chosen by option) and then stops.
  - Serialization: `serde` with a versioned binary format (bincode or postcard) and a JSON
    debug form. The model overlay is written as XMI through `simoxide-model`'s writer. That
    writer comes from the foundation crates (qvto-emf's XMI writer, moved there by the merge).

**F1 as built** (2026-10-17, branch `f1`):

- **Layout.** `Simulation<'m, C>` = `{ cm: &CompiledModel, code (traced or plain variant),
  trace writer, tape writer, st: RunState<C> }`. `RunState` (`crates/simoxide-sim/src/sim/state.rs`,
  not a top-level `run.rs`: it holds the engine's private types) has: config, phase, failure,
  `traced`, model shape; clock, event count, livelock-guard counters, `main_count`; processes
  (continuation stacks, frames), free slots, table sequence, the shared empty frame, fork joins;
  `ResRt` per resource (schedulers, delay sets, state shadows), passive instances, component
  instances, dynamic assembly-context paths; the random source with its tape *buffer*; the
  measurement sink (rows, sliding windows, PRM recorders, reconfiguration process state, the
  event list with resource timers); lazily created series; resolution caches (`provs`,
  `call_cache`, `elsc_prov`); step/event counters of the `Limits` checks. Within-event counters
  (`nesting`, `direct`) are zero between events. The frame pool and the spare stacks are
  allocation reuse only; a clone starts with empty ones.
- **API.** `Simulation::run_until(Pause::{After(t), Before(t), Events(n), Never})`,
  `run_while(pred)` → `Status::{Paused, Finished}`; `finish()` (= `run()`); `state()`,
  `snapshot()` (clone, outputs flushed first), `fork(outputs)`, `into_state()`,
  `Simulation::resume(cm, state, outputs)`, `set_limits`; `RunState::detach()` → `Detached`
  (`Send`) → `into_state()`. Phases `NotStarted`/`Running`/`Finished`/`Failed` (a failed state
  returns its error again, also in forks).
- **Rc vs Arc: Rc stays.** `Arc` frames cost 7.3 % on the benchmark suite (geomean of the long
  runs), 13 % on MediaStore. A clone shares the frames copy on write (same thread); `detach`
  deep-copies the frame and proxy graph keeping its sharing (`FrameCopier`), and `Detached` is
  `Send` through an `unsafe impl` whose argument is that every reference count of the copy is
  owned by the copy.
- **Hidden state found.**
  - The tape writer (`Box<dyn Write>`) lived inside the RNG: moved out. The RNG keeps a
    buffer; the simulation drains it after events and every 4 096 steps (output unchanged).
  - Process program counters index into the traced *or* the plain instruction stream, and
    `ConsumeDone` continuations exist only with a trace: the state records `traced`. A traced
    state resumes with a trace (discarded without a writer); an untraced one cannot get one.
    The tape can be attached or dropped at any pause (the origin tag is `?` between events).
  - The frame pool must not be cloned shallowly (pooled frames must be unshared; a shared one
    would fail `frame_mut`), so clones get empty pools.
  - `Limits::cancel` is an `Arc` in the config: forks share the flag unless `set_limits`.
  - An elided process hand-off counts as an event inside another event, so `Pause::Events(n)`
    can stop at n + 1.
  - No static or thread-local state influences results (the `simoxide-random` memos are of
    pure functions).
- **Fork cost** (`simoxide-bench fork`, halfway through the long runs): 0.4-12 µs without
  stored rows (12 µs: PS model with 117 live processes), detach 0.5-30 µs; with stored rows
  the rows dominate (MediaStore, 1.1 M tuples: 530 µs).
- **Blockers for N1 (serde):** the `Rc` frame/proxy graph (serialise as an arena with indices,
  the `FrameCopier` traversal); `&'static str` in the state (`SeriesDef::metric`, the metric
  keys of lazily created series, `Sink::error`, `Reconf::error`, `ProvTarget::Fail`), which
  need an interning table or codes; `FastSource`'s `&'static Tables` (rebuild on load);
  `Limits::{deadline, cancel}` (run-time controls, skip); `RngMode::Replay(Arc<Tape>)`; serde
  for the MT19937 state (`simoxide-random`) and the schedulers (`simoxide-sched`, generic
  `J`); `Shape` is only a size check and should become a content hash of the `CompiledModel`.

**N1 as built** (2026-10-17, branch `n1`; guide `docs/guide/snapshots.md`, format
`docs/formats/snapshot.md`):

- **Format (version 1).** A magic line `SIMOXIDE-SNAPSHOT 1`, one JSON header line (format and
  SimOxide version, mode, literal engine, flavour and compat flags, the base model's fingerprint,
  the run model's generation and fingerprint, time, events, phase, traced, encoding, and the
  sections with length and FNV-1a-64 checksum), then the sections. Encodings: postcard (default,
  compact) or JSON (debugging; doubles bit for bit with serde_json's `float_roundtrip`,
  non-finite values do not survive). Sections: `engine` (the `RunState`), `model` (after model
  changes), `spd`, `rules`, `qvto` (layers above the engine add theirs; unknown ones are ignored).
  No timestamps; maps in key order: the same state gives the same bytes. A snapshot is read by the
  same SimOxide version only (the sections are internal state); version, model, mode and
  flavour mismatches are distinct errors (`SnapshotError`).
- **Model fingerprint** (`simoxide_model::persist::fingerprint`, `model_fingerprint`): two FNV
  lanes over the graph's content (classes, ids, values, references, resource names; not paths or
  lines) plus flavour and quirks. It replaces `Shape` for snapshots (`Shape` stays the in-memory
  check of `resume`).
- **`engine`.** serde derives behind the feature `snapshot` (`cfg_attr`, mechanical) on the
  engine's state types in `sim.rs`, `sim/state.rs`, `sim/reconf.rs`, `sim/hooks.rs`, `events.rs`,
  `meas.rs`, `windows.rs`, `javarand.rs`, `stats.rs`, `flavour.rs`, `code.rs`, `ir.rs`,
  `compat.rs`; feature `serde` in `simoxide-sched` (schedulers, generic `J` with explicit bounds)
  and in `simoxide-model` (typed ids only, `serde(transparent)`; the raw graph has none). The F1
  blockers: frames and proxies as an arena in dependency order (`sim/snapshot.rs`, sharing kept);
  `&'static str` fields are a type alias `ir::StaticStr` (so serde does not borrow) interned on
  read; the MT19937 and xoshiro states through accessors (`simoxide-random`: `state`/`from_state`,
  no serde there); `Limits::{deadline, cancel}` are not written; a replayed tape is part of the
  configuration and the RNG keeps only its position; `SimRng::{save, load}`. Within-event
  counters and pools are not written. A restored state is checked against the model's table sizes.
- **`model` (U10).** After `apply_changes` the run's compiled model is not a fresh compilation of
  its graph (the merge keeps old resources, appended instruction streams, the old allocation
  lookups, dynamic paths, ...), so `build_from_graph_after` + `compile_seeded` alone cannot rebuild
  it. The section holds: the generation; the build order; the graph as XMI documents with an object
  table (`simoxide_model::persist::GraphXmi`, new: an EMF-form XMI writer for the raw graph, one
  document per resource plus `detached-objects.xmi` for tombstones and unattached objects; the table
  maps document elements to object ids and carries homes, lines, unresolved proxies and values XMI
  cannot express); and the compiled tables by serde (StoEx programs by their text, keys by name,
  the typed model and the finalisation orders rebuilt). Restore: load the documents with the
  ordinary loader, renumber (the graph equals the saved one object for object, `same_graph`),
  `build_from_graph_after(graph, ∅, order)`, recompile the programs, check the run model's
  fingerprint. `write_xmi` is also the standalone XMI save of a resource. After M5 the documents
  stay; the table maps to the shared graph's ids (or the graph's own serde replaces it).
- **API.** `Simulation::{snapshot_to, snapshot_with, snapshot_value, restore_from,
  restore_snapshot}`, `RunState::{serialize, deserialize}`, `Simulation::base_model` (a new field
  `base` in `Simulation`: the fingerprint needs the base model after changes);
  `SpdRun::{snapshot_to, restore_from}` (section `spd`: policy, group and window states,
  membership, queued requests, counters, the interpreter's compat flags, checked on restore);
  `RulesRun::{snapshot_to, restore_from(r, base, new host)}` (section `rules`);
  `QvtoRun::{save_state, restore_state, snapshot_section, restore_section}` (section `qvto`).
  CLI: `simoxide run --pause-at T|--pause-before T|--pause-events N --snapshot FILE
  [--snapshot-format json]`, `simoxide resume FILE --model DIR [--until T --snapshot FILE2]`
  (mode, flavour and quirks from the header; SPD and rules runs recognised by their sections;
  the rules log and SPD log of each hop concatenate), `simoxide snapshot-info FILE`. The CLI always
  has the `snapshot` feature.
- **QVTo state.** The adapter's `Session` (a `qvto_emf` resource set) has no serialisation; the
  state is "the run's graph (engine XMI) plus the id generator": `qvt::SessionState { ids }`
  behind `RunSession::{save, restore}` (the one place M6 re-points), the journal is empty between
  events, the PRM mirror is rebuilt by `RulesHost` with the same measurements in the same order
  before the id generator is set. Adapter API added: `Session::{id_state, set_id_state}`. **What M6
  must provide:** serde (or a save/restore) of the graph-resident QVTo state: the id policy's
  generator state, the resources of other metamodels (PRM) with their handles, and the journal
  position, so that the run's session is restored as it was instead of rebuilt.
- **Slingshot-exact engine (`ssx`, merged from `spd` at 59484b7; superseded by 5.7, "ssx
  snapshots as built").** Not snapshotted: it has no
  pause and resume (only `ssx::run`); the CLI refuses `--pause-at`/`--snapshot` for
  `--flavour slingshot-exact` with an error. It needs: a pause API (the engine as a value with
  `run_until`); its bus FIFO of handler steps and events; the `Rc<RefCell<…>>` graph of users,
  behaviour contexts, SEFF contexts, holders, RPCs and requests and their Java-style mutable frames
  as an arena that keeps identity (mutation through shared references, unlike the copy-on-write
  frames of the other engine); the subscribers' disposed bits; its random streams (MT19937,
  `java.util.Random`, the Palladio 6 samplers) and monitors. About 1 w after X2d.
- **Tests (exit criterion).** `simoxide-sim/tests/snapshot.rs`: every model directory (104), each
  run written at several pause points (event counts incl. same-instant splits, times), restored
  into a fresh simulation each time, the last hop on another thread; the restored state must write
  the bytes it was read from; output byte-identical to the uninterrupted run: exact with trace and
  tape and without outputs (1 664 snapshots), fast (832), the Slingshot flavour in both modes on
  every model it runs (104 models, 1 664), the literal engine and tape replays, JSON, after
  model changes (three generations, a snapshot after the second and after a third change
  following a restore, exact and fast: 197 runs), finished and failed runs, determinism (two runs
  and a fork write equal bytes), refusals (model, mode, flavour, trace, format and SimOxide
  version, checksum). `simoxide-model/tests/persist.rs`: every model directory's graph restores
  object for object with the same typed model; changed graphs with detached subtrees, new and
  unattached objects, proxies, `null` and mistyped values, odd strings; fingerprint independent of
  the directory. `simoxide-spd`: snapshots at five points (postcard, JSON) equal the uninterrupted
  SPD run (decision log, state, measurements, events), the five SPD reference models with
  snapshots keep their decision logs, other interpreters are refused. `simoxide-reconf/tests/
  snapshot.rs`: direct edits (F2 `Edit`: a new container and allocation, a rate change, a removal
  with a tombstone; snapshots after each change and between, exact and fast); the QVTo state after
  a rule ran (the next rule run creates the ids of the uninterrupted run; without the state they
  repeat); the six `corpus-rules` examples with a fresh host at six pause points and while a
  reconfiguration waits, byte-identical to the patched reference (exact build) and to the
  uninterrupted fast run, rule records equal (72 snapshots). `simoxide-cli/tests/snapshot_cli.rs`:
  `run --pause-at --snapshot` and `resume` in new processes (MediaStore three hops, a fork model,
  the Slingshot flavour with quirks, an SPD model with its decision log, `lb_scaleout` with rules,
  JSON hops), another model and the Slingshot-exact engine refused.
- **Gates.** `golden.sh` identical (312 runs, exact build with `hotspot-math`, against `spd`
  59484b7's binary); `cargo test --release --workspace --features exact` 586 passed, 0 failed (563
  before: +4 `simoxide-model` persist, +9 `simoxide-sim` snapshot, +2 `simoxide-spd`, +3
  `simoxide-reconf`, +5 `simoxide-cli` (+1 with `reconfiguration`); QVTo's ignored reference material linked from the merge
  checkout for the run); `simoxide-fuzz corpus` 132/132 and corpus-fuzz 6/6; fmt and clippy
  `-D warnings` clean (workspace, all targets, default and `--features exact`; `simoxide-cli` with
  `reconfiguration`, with `exact,reconfiguration`, without default features; `simoxide-reconf` and
  `simoxide-spd` without default features; `snapshot` alone on sim, spd, reconf); release check:
  `cargo tree -e normal` of `simoxide-cli` (also with `reconfiguration`) and `qvto-cli` lists no
  opt-in crate (new dependencies serde, postcard, cobs, thiserror: MIT/Apache-2.0). Hot path: the
  derives add impls only; a quick run of MediaStore (exact) and h20 (fast) against the base binary
  is within noise (timing deferred). The CLI binary grows from 12.3 to 22.9 MB (the serde code of
  four mode-flavour instantiations, two encodings).
- **What N2 needs.** A change step between restore and continue (the CLI marks the place in
  `snap::resume`): change sets from a file (direct edits, a QVTo transformation, an SPD or rule-set
  change) applied with `apply_changes` on the restored run; enforced policies (adjustment events
  queued at resume time, bypassing triggers); snapshot conditions as `run_while` predicates (time,
  a policy's trigger, a threshold, an SLO) that write a snapshot; a fork-based exploration example
  (snapshot once, restore many times with different changes; `Snapshot` is plain bytes, so
  restores can run in parallel threads or processes).
- **What N3 needs.** The `engine` section is SimOxide's internal state; the Slingshot `.snapshot`
  export reads it through the `RunState` (processes with their continuation stacks and positions,
  jobs with remaining demands, timers, adjustor states with times), not through the file format.
  The header's mode, flavour and model fingerprint tell an importer what it has.

**N2 as built** (2026-10-17, branch `n2`; guide `docs/guide/snapshots.md`: pause conditions,
resume with changes, change files, exploration). Done except the P2 part (enforced policies).

- **Pause conditions** (`simoxide-sim/src/sim/watch.rs`, new): `Watch` = a set of `Condition`s
  (any one pauses): `At(Pause)` (time, events), `Elapsed(ns)`, `ElapsedEvents(n)` (relative to
  arming: the time of the last event before the pause, the event count), `Series { series:
  SeriesSel { metric, point }, stat: Value | Mean { window_s }, op, value, sustain_s }` (the
  trigger: a tuple, or the mean of the last `w` s per series, compared with a value, optionally
  held for `w` s on every tuple of that series; pauses before the first event at or after the
  start of the holding plus `w`), `Slo { upper, lower, sensitivity, delay_s }` (the threshold:
  Slingshot's `SnapshotSLOTriggeringBehavior` range narrowed by the sensitivity). Series are
  selected by metric (alias, name prefix or `*`) and point (key, part of a key, an element name
  resolved to its id and its children's ids, or `*`), also series created later. The watch reads
  the stored rows (`store_measurements` required) with a cursor per series and skips the series
  work when the tuple count did not change; it writes nothing into the run state, so a watched
  pause equals `Pause::Events(k)` at the same count, byte for byte (snapshot included).
  `Simulation::run_watch` checks time conditions with the engine's own `Pause` comparison (the
  flavour's clock); `Watch::keep_going(&RunState)` is the predicate for `run_while` and for the
  drivers: `SpdRun::run_while` and `RulesRun::run_while` (new; their `run_until` now delegate to
  them, behaviour unchanged). The general predicate stays `run_while`. `parse_condition` is the
  CLI grammar (`--pause-when`, repeatable, ORed with `--pause-at` and the like; the CLI prints
  `pause condition: <reason>`). Not built: "a policy's trigger fired" as a condition (it needs
  P2's adjustment requests to snapshot before; a decision-log condition can be added then).
- **Change files** (`simoxide-reconf/src/edits.rs`, new; format in the guide): one change per
  line, mapped onto F2's `Edit` (`set_rate`, `set_spec`, `set_double`, `clone_container` +
  `connect` to the template's links, `clone_assembly`, `allocate`, `remove_from_environment`,
  `delete`, the graph's `ModelStore::set`), all lines on one graph copy, applied as one change set
  (`Simulation::apply_changes`). Ops: `rate`, `latency`, `throughput`, `demand`, `probability`,
  `think-time`, `inter-arrival`, `arrival-rate` (`Exp(rate)`), `population`, `replicas`,
  `loop-count`, `set` (any single-valued attribute by EMF name, a random variable, a
  reference), `add-container`, `connect`, `add-assembly`, `allocate`, `remove-container`,
  `delete`. Elements by id, else by a name unique among the elements of the expected kind;
  `REF:TYPE` picks a container's resource or an action's demand by resource type. Errors carry
  the line; `ChangeFileError::Change` wraps the change path's refusal (run unchanged).
- **Resume change sets** (`simoxide-reconf/src/resume.rs`, new): `ResumeChanges` = ordered
  steps `File(ChangeFile)` | `Transform(Transformation, Invocation)` (run once through F3's
  `QvtoRun::invoke`; a transformation that does not complete is `ResumeError::NotApplied`);
  `apply(&mut sim, &mut QvtoRun)` → `StepReport`s. API pattern: `Simulation::restore_from` (or
  `restore_snapshot`) → `QvtoRun::restore_section` → `ResumeChanges::apply` → continue. Rule
  sets (R1): `RulesHost::replace_rules`, `RulesRun::restore_snapshot_replacing` (other rules than
  the snapshot's; the QVTo state, PRM mirror and statistics continue) and `snapshot_rules`;
  rules added to a run without rules = `RulesRun::new(restored, host)`. SPD: `SpdRun::
  model_changed` (membership from the changed model), `SpdRun::enforce_policies(names)`: the
  hook for Slingshot's incoming policies; it checks the names (`EnforceError::UnknownPolicy`) and
  returned `EnforceError::Unsupported` until P2; P2 queues a request per policy at `now`, past
  triggers and constraints, as `SnapshotInitFromBehaviour` does (5.4 "P2 as built").
- **CLI.** `simoxide resume FILE --model DIR [--change F.changes]... [--transform X.qvto]...
  [--rules DIR] [--enforce P,...] [--until T | --pause-when C ...] [--snapshot FILE2]`: the
  steps run in command-line order after the restore (plain, SPD and rules runs; transformations
  of a rules run use its host's QVTo state, of other runs a fresh one with the snapshot's `qvto`
  section, `FailedRunChanges::Discard`, and the next snapshot carries it); `--rules` with other
  rule names replaces them, on a snapshot without rules adds them; one `--qvto-mode` parser
  (`auto|ordered|fast|compat`, R2). `--change`/`--transform` need the `reconfiguration` feature
  (an error names it otherwise). `simoxide explore FILE --model DIR --variant NAME[=F.changes|
  =X.qvto]... [--until T] [--threads N] [--metrics rt,util,...]`.
- **Exploration.** `simoxide_reconf::explore::<C, F>(base, &snapshot, &core, &variants, until,
  threads)`: one restore per variant on a pool of scoped threads (the restores are independent;
  each thread builds its own simulation and QVTo state from the shared `Snapshot` bytes, the
  `Sync` compiled model and core), changes applied, run to `until` or the end, the
  continuation's measurements summarised; results in variant order. `compare` prints the table
  (means; time-weighted for state and utilisation series; end time, events, finished scenario
  runs). Example `crates/simoxide-reconf/examples/explore.rs` (MediaStore screencast model,
  snapshot at 2 000 s, five variants: base, faster disk, twice the load, load plus a faster CPU,
  the file storage moved to a new faster server; about 10 ms on 4 threads).
- **What needs F4 (MENTOR-relevant):** population changes of a closed workload (MENTOR's load
  variation); new or removed usage scenarios; monitors for new containers (a scaled-out
  container has no utilisation series, so SPD and pause conditions cannot see it; the
  `Number of Resource Containers` series likewise); the number of replicas (cores), scheduling
  policy and resource type of a processing resource (ignored as SimuLizar ignores them, MC-1.4:
  a fixed behaviour needs F4's syncer); scale-in removal policies (a removed container keeps
  simulating); structural behaviour changes (a branch added to a load balancer's SEFF takes
  effect for new executions only; an action's kind or demand count cannot change); containers
  added and removed in one change set; and **every model change in the Slingshot flavour**
  (`ChangeError::Unsupported` from `change::recompile`: Slingshot's syncers are F4/P3), which
  also refuses change files, transformations and rules at a resume. Enforced policies need P2.
- **Tests (exit criterion).** `simoxide-reconf/tests/resume.rs` (7): change files at a resume
  equal the direct F2 edits in the uninterrupted run byte for byte (result, measurements, trace,
  tape) on five models (h19 container + allocation + rate; h07 branch probabilities, a demand
  and a rate; h06 think time and a demand by name; h09 loop count and `set`; MediaStore
  arrival rate and an HDD rate), exact and fast, and each change has an effect; refused changes
  (population: `Unsupported`; a removed allocation context: `Abort`; an unknown element; a wrong
  workload kind) leave the restored run equal to the uninterrupted one; the Slingshot flavour
  refuses and continues unchanged; QVTo transformations at two resumes (fresh simulation and
  QVTo state with the section `qvto` each time) equal the in-process invocations, new ids
  included, exact and fast; a rule set replaced (`lb_scaleout`: `outsource` alone until 300 s,
  then both rules) and one added to a run without rules equal the in-process runs (R2's
  settings: QVTo `Auto` = ordered); `RulesRun::run_while` with a watch; exploration on 1 and 3
  threads and twice gives the same results, equal to a variant run by hand, the refused variant
  reports its error. `simoxide-sim/tests/watch.rs` (3): time and event conditions pause where
  `Pause` pauses (same snapshot bytes; SimuLizar exact and fast, Slingshot fast), `Elapsed` and
  `ElapsedEvents` after a pause; value, mean, held and SLO conditions (with a delay) pause at
  event k exactly as `Pause::Events(k)` and not at the check before (h06, `t_util`, the
  Slingshot flavour); a condition that never holds leaves the run unchanged; series conditions
  without stored measurements are an error. `simoxide-spd/tests/sim.rs` (+2): `SpdRun::
  run_while` with time and response-time conditions, continuation equal (decision log, state,
  measurements); `enforce_policies` refuses (unknown name, P2) and the run is unchanged.
  `simoxide-cli/tests/resume_cli.rs` (2 default, 5 with `reconfiguration`): `--pause-when`
  (held state, then `elapsed`) across three processes equals the uninterrupted run; change
  files deterministic and effective, refusals with their messages (wrong workload kind, abort,
  unknown element, closed population F4, Slingshot flavour, `--enforce` without SPD), two
  `--transform` hops, rules replaced and added, `explore` on 1 and 3 threads; without the
  feature `--change` names it.
- **Shared files touched** (small, delimited): `simoxide-sim/src/sim.rs` (`pub mod watch;`),
  `simoxide-sim/src/lib.rs` (re-exports `Condition`, `Watch`), `simoxide-spd/src/sim.rs`
  (`SpdRun::run_while`, `run_until` delegates), `simoxide-spd/src/lib.rs` (`mod resume`),
  `simoxide-reconf/src/process.rs` (`RulesRun::run_while`, `RulesHost::replace_rules`),
  `simoxide-reconf/src/snapshot.rs` (`restore_snapshot_replacing`, `snapshot_rules`),
  `simoxide-reconf/src/lib.rs` (modules, re-exports), `simoxide-cli/src/main.rs` (resume, explore,
  pause conditions; one `--qvto-mode` parser). New: `simoxide-sim/src/sim/watch.rs`,
  `simoxide-reconf/src/{edits,resume}.rs`, `simoxide-spd/src/resume.rs`, the tests and the example.
  Merged with `spd` at 8386d26 (R2) and 49dd1d8 (M4-M6, the shared graph): the change files work
  on the shared graph through the public `Edit` API (only `edits.rs` and the tests needed the new
  `Edit::new(&sim)` and graph accessors).
- **Gates.** After the merge with `spd` 49dd1d8: `golden.sh` identical (312 runs, exact
  build with `hotspot-math`, against the binary of a3ffa38); `cargo test --release --workspace
  --features exact` 566 passed, 0 failed (549 on `spd` 49dd1d8; +17: `simoxide-reconf` resume 7
  and the change-file parser 1, `simoxide-sim` watch 3 and the condition parser 1, `simoxide-spd`
  +2, `simoxide-cli` resume 2; with `reconfiguration` the CLI has 5 resume and 6 snapshot tests,
  all passing); the QVTo adapter-parity test needs the Java cache of the merge checkout
  (`target/qvto-testkit` from `simoxide-merge`, whose identity hashes the recorded data has);
  `simoxide-fuzz corpus` 132/132 and corpus-fuzz 6/6; fmt and clippy `-D warnings` clean (workspace,
  all targets, default, `exact,hotspot-math`, `compat`; `simoxide-cli` with `reconfiguration`,
  `exact,reconfiguration`, without default features, with only `reconfiguration`, with only
  `fast,spd`; `simoxide-reconf` and `simoxide-spd` without default features; `snapshot` alone on
  sim, spd, reconf); release check: no opt-in crate in `simoxide-cli` (also with
  `reconfiguration`) or `qvto-cli`. Runs without conditions or changes do not touch the new code
  (the watch is a separate `PauseWhen`; `run_until` of the drivers delegates to the same loop);
  a quick timing of the MediaStore screencast model (exact, 30 runs) is within noise (timing
  deferred).

**ssx snapshots as built** (2026-10-18, branch `ssx-snapshot`; guide `docs/guide/snapshots.md`
"The Slingshot-exact engine", `docs/guide/slingshot.md`; format `docs/formats/snapshot.md`,
sections `ssx`, `spd-ssx`). Pause, resume, fork and snapshots for the third engine; with it the
snapshot/restore requirement (MENTOR) holds in all three modes.

- **`SsxRun`** (`ssx/run.rs`, new): the engine with its event list as a value. `new` runs
  `ResourceSimulation`'s constructor and queues the first events (a set-up error is the run's
  error, outputs written); `run_until(Pause, hooks)`, `run_while(hooks, predicate on SsxView)`,
  `run_watch(&mut Watch, hooks)` and `finish(hooks)`, the SPD hooks passed per call;
  `fork(outputs)`, `detach() -> SsxState` (`Send`), `attach(base, state, outputs)`; accessors
  (time, events, measurements, warnings, generation, model graph). The loop is the old one
  split in two (`Engine::start`, `Engine::advance<P: SsxPause>`): `P::NEVER` compiles the check
  away, so `ssx::run` and `run_with_spd` are now `SsxRun::new(..)?.finish(hooks)` without a
  pause check per event. The pause is checked before each event (`Pause` on the double clock:
  `After(t)` pauses before the first event later than `t` ns as `F64Secs` converts it), the
  outputs (projection, tape) are flushed at each return; a traced state continues with a trace
  (or discards it), an untraced one cannot gain one, a tape can be attached at any pause. An
  error ends the run and is returned again. N2's `Watch` reads runs through a crate-private
  trait (`Watched`: time, events, measurements, stored rows), implemented by `RunState` and the
  engine; `Watch::arm` and `keep_going` are unchanged.
- **State** (`ssx/state.rs`, new): `SsxState` = the engine's image (`EngineImg`, plain data),
  the configuration, the run's own model (the last `Arc<CompiledModel>` and its build order) and
  the graph override. Slingshot's `Rc` objects become one **object arena** (`NodeImg`: immutable
  frames and proxies, SimuCom frames, users, usage behaviour contexts, user contexts and
  requests, holders, request processing contexts, SEFF contexts, entry requests, calls over the
  wire, resource demand requests, jobs, `ModelAdjusted` payloads), written depth first in post
  order without recursion (identity by kind and address), references are earlier node indices;
  the roots in a fixed order (events by insertion number, jobs at a resource by id, passive
  waiting queues), so the same state gives the same arena. The event list as `(time, insertion,
  event)` by insertion number. Rest as data: schedulers (serde of `simoxide-sched`), MT19937
  words and position, draws, `java.util.Random`, the tape position, disposed bits, handler
  exceptions, measurements, monitors (maps in key order), the projection's numbering maps and
  line count, `res_live`, passives, the linking-resource table, allocation, counters, the SPD
  side (time events, subscribers, their bits). Aliases and input ids are computed again. A fork
  and a detached run are a round trip through the image (no bytes): a deep copy that keeps the
  sharing, `Send` because the image holds no `Rc`.
- **Format** (v1, compatible; no version bump): header `flavour` = `slingshot-exact` (quirks:
  all 37; `mode` is the configuration's, not read), section `ssx` (configuration as N1's
  `ConfigImage` and the engine image) instead of `engine`, `model` and `graph` as N1 writes them
  (shared helpers `model_section`, `read_model_section`, `graph_section`, `read_graph_section`,
  factored out of `RunState::serialize`/`deserialize`, whose bytes are unchanged). The other
  engines refuse an `ssx` snapshot by flavour and vice versa. API: `SsxState::{serialize,
  deserialize}`, `SsxRun::{snapshot_value, snapshot_to, snapshot_with, restore_from,
  restore_snapshot}`.
- **SPD layer** (`simoxide-spd`): `SsxSpd::snapshot_sections` writes `spd` (the same image as
  `SpdRun`'s: interpreter state, request table by handle, counters; shared `spd_section` and
  `read_spd_section`) and `spd-ssx` (per series the resolved measuring point and metric: they
  are resolved on the model of a series' first measurement, so they are kept); `SsxSpd::restore`
  rebuilds the layer (subscribers from the interpreter); `SsxSpd::model_changed` refreshes the
  membership after a change the layer did not make. The adjuster's QVTo state is P2's `qvto`
  section; `QvtoRun::{restore_state, restore_section}` are generic over `ChangeHost` (callers
  unchanged).
- **Changes between events**: `SsxRun` is a `ChangeHost`; its `apply_changes` is the engine's
  (as inside an adjustment) plus what `ModelAdjusted`'s subscribers do for it (`Engine::
  sync_after_change`: resources of new containers join the table, deleted containers' leave,
  passive resources of new allocation contexts, calculators of new monitors), without the event.
  `ChangeFile::apply_to` and `ResumeChanges::apply_to` (`simoxide-reconf`) take any `ChangeHost`;
  the engine has no change options, so option lines are refused.
- **CLI**: `run --flavour slingshot-exact` with `--pause-at|--pause-before|--pause-events|
  --pause-when` and `--snapshot` (plain: `ssxrun::plain`; SPD: `spd::run_exact`, now started or
  restored), `resume` routes by the header's flavour, `--change`/`--transform` with
  `reconfiguration` (SPD runs in the adjuster's QVTo state), `--enforce` refused (no request
  queueing at a resume in this engine).
- **Tests (exit criterion).** `simoxide-sim/tests/ssx_snapshot.rs` (5): every case of
  `corpus-ss/` and `corpus-spd/` (95: 90 + 5, the two that fail before t = 0 compared by their
  error) paused at six points (events 1, n/4, time T/2, n/2, before 3T/4, n-1), a snapshot at
  each, each hop a fresh run restored from the last one's bytes (hop 3 as JSON, the last hop on
  another thread with its own interpreter and QVTo core), with SPD the layer and the adjuster's
  QVTo state restored from the sections: projection, tape with positions, measurements, handler
  exceptions, counts and decision log equal the uninterrupted run's (558 snapshots); after each
  snapshot a restore writes the same bytes. Forks: on 93 cases (those that pause at n/2) a fork
  in memory, a detached state on another thread and the original all continue to the
  uninterrupted result. Pause conditions (value, time, `before`, events; `elapsed` after a
  pause; a condition that never holds) pause where `Pause::Events` pauses, same bytes. A change
  file at a resume equals the change at the pause and has an effect; option lines are refused
  and leave the run unchanged. Same state, same bytes (two runs, a fork, a detached copy);
  `SsxState: Send`; an untraced state refuses a trace; another model is refused. A mutation
  check (a counter restored off by one) fails the suite. CLI: `snapshot_cli.rs` (+2, MediaStore
  three hops with a JSON hop and a pause within an instant; `spd_scale_in` with its decision log,
  adjustments before and at a pause), `resume_cli.rs` (+2: `--pause-when` across three
  processes; `--change` deterministic, effective, option lines refused).
- **Gates** (after merging `spd` a324c4e, P2/P3). `slingshot_exact` 89 of 90 (KNOWN: h27) and
  `corpus_spd_is_byte_equal_at_level_e` (5 of 5 with `spd_same_instant`); the new suite 5 of 5;
  `golden.sh` identical (312 runs, `cargo build-exact`, against 4ba5244's binary); `cargo test
  --release --workspace --features exact` 612 passed, 0 failed, 39 ignored (605 + 5 `ssx_snapshot`
  + 1 `snapshot_cli` (+2, the refusal test replaced) + 1 `resume_cli`); the CLI with
  `reconfiguration`: `resume_cli` 7/7, `snapshot_cli` 7/7; EMF goldens 165/165; `simoxide-fuzz
  corpus` 132/132, corpus-fuzz 6/6; `test-fdn` 93; `test-qvto` 90; `conform` compat 589/589,
  fast 584, ordered 583, sema oracle 0 extra (unchanged); fmt and clippy `-D warnings` (default,
  `exact,hotspot-math`, `compat`; `simoxide-cli` with `reconfiguration`, `exact,reconfiguration`,
  without default features); release check 0 opt-in crates (`simoxide-cli`, `qvto-cli`). Speed
  (sanity only, timing deferred): `x_ss_mediastore` 200 000 s, 20 runs, three alternating rounds
  on one core: 0.94-1.12 ms per run before, 0.97-1.14 ms after (noise); the run without pauses
  compiles the pause check away.
- **Side fixes.** `resume_with_a_change_file` (CLI, `--features reconfiguration`): the test was
  stale since F4 made `drain` the default removal policy (the deleted allocation context is then
  an invalid model, SimuLizar's abort needs `removal keep`) and F4 applied population changes
  and Slingshot-flavour changes it still expected to be refused; not mode-dependent. P3 fixed it
  on `spd` the same way, taken over by the merge. `reference/slingshot/tools/compare_exact.sh`
  runs SPD cases with their `.spd` (decision log `spd.jsonl`; adjustments need a binary with
  `reconfiguration`, the line says so otherwise) and takes `-c CORPUS`: with an
  `exact,hotspot-math,reconfiguration` CLI 87 of 90 (`corpus-ss`: the three SPD cases equal; h27
  differs, `x_pem_acquire` and `x_sl_runconfigtest` have no reference measurements) and 5 of 5
  (`corpus-spd`).
- **Shared files touched**: `simoxide-sim` (`sim.rs`: `pub(crate) mod snapshot`;
  `sim/snapshot.rs`: section helpers, `ConfigImage` crate-visible; `sim/watch.rs`: `Watched`,
  `arm_on`; `ssx/*`; `Cargo.toml` dev-dependency features); `simoxide-spd` (`interp.rs`: serde
  derive on `Metric`; `snapshot.rs`: section helpers, `spd-ssx`; `ssx.rs`); `simoxide-reconf`
  (`snapshot.rs`: `ChangeHost` generic; `edits.rs`: `apply_to`; `resume.rs`: `apply_to`);
  `simoxide-cli/src/main.rs`; the CLI tests; `compare_exact.sh`.
- **Limits.** A change at a resume makes no `ModelAdjusted` event and no projection line;
  `--enforce` is refused; JSON snapshots cannot hold non-finite doubles (N1); the SPD layer has
  no in-memory fork (a fork restores it from the sections); snapshots are taken between events
  only.

**N3a spike** (2026-10-17; Slingshot's `.snapshot`, `.config` and `.state` against SimOxide's
`RunState`). Sources: the extension `Palladio-Analyzer-Slingshot-Extension-Initialisation`
(`[INIT]`, bundles `snapshot.data`, `snapshot.serialization`, `snapshot`, `initialisedsimulation`),
Slingshot core and extensions, `reference/slingshot/`.

**Verdict: go, for the Slingshot fast flavour, export first; lossy by design; not for the
Slingshot-exact engine yet.** Export (SimOxide → `.snapshot`) is feasible and can be more correct
than Slingshot's own snapshots; import is feasible for the subset Slingshot can write and read;
exact SimOxide→SimOxide chains keep using the N1 snapshot (carried alongside, e.g. as an extra
top-level field, which Slingshot's reader ignores). Slingshot itself reads back only a subset of
what it writes, so "round trip with Slingshot" is limited to that subset.

- **Formats.**
  - `.snapshot` = `InitState {id, pointInTime, snapshot: {events, statevalues}}`. Events are the
    engine's scheduled events (`{"type": class, "event": gson reflection}`: `id`,
    `scheduledTime`, `delay`, `entity`, generic type token), minus a skip list (snapshot,
    measurement and `SimulationFinished` events, `ProcessorSharingJobProgressed`), plus recorded
    `JobInitiated` per job and open response-time calculators (`UsageModelPassedElement<Start>`,
    `SEFFModelPassedElement<StartAction>` with negative offsets). Entities are
    `{class, refId, obj}` on first use, then the bare refId (`hashCode$classHash`: collisions
    alias silently); a closed type list decides what can be read (`SlingshotTypeTokenSets`). PCM
    elements are `file#fragment` (XMI id or EMF path) or a full pathmap URI; SimOxide's loader
    resolves both. StoEx frames are plain gson maps (shared frames duplicated; numbers come
    back as `Double`, INNER proxies as maps). `statevalues` = SPD adjustor states with shifted
    times; they need MENTOR's SPD branch (another package than Slingshot master).
  - `.config` = `{incomingPolicies: [policy id], parameters: {Behaviour: {active, ...}}}`:
    `SnapshotTriggeringBehavior` (`dodrop` — the README's `doDrop` has no effect —,
    `dropInterval`), `SnapshotSLOTriggeringBehavior` (`sensitivity`, `activationDelay`), the two
    abortion behaviours (`active`); `minDuration` is read nowhere.
  - `.state` = `{parentId, id, startTime, duration, reasonsToLeave, utility, outgoingPolicyIds,
    measurementSets}` (EDP2 measurements with local times; utility from the SLO repository and a
    cost extension).
- **Corrections to §3** (from the code, partly verified by runs): pending
  `ModelAdjustmentRequested` events are not written to `.snapshot` (only to `.state` as outgoing
  policy ids); only closed-think, inter-arrival and calculator events are rebased: every other
  scheduled event is restored at its old local `scheduledTime + delay`, so stale FCFS
  `JobProgressed`, `UserWokeUp` and `SimulationTimeReached` events fire late or twice; FCFS vs PS
  is decided by the container's first processing resource, not the job's (an HDD head job is
  written with its full demand); one job record per user (a second concurrent job throws in the
  recorder); passive-resource holders and waiters are not captured (verified: the waiting user
  is lost; on restore the capacity inflates); the maximum state duration is the experiment's
  simulation time; SPD time triggers are rebased by editing the SPD model copy; **Slingshot cannot
  read its own snapshots that contain usage loops or branches, SEFF loops, forks,
  infrastructure calls or linking jobs** (verified: `Missing Type mapping
  …LoopScenarioBehaviorContext` on h09, `…LinkingJob` on h19).
- **Mapping** (fast flavour / Slingshot-exact engine):
  - *1:1*: closed users in think time, open arrivals, usage delays; usage and SEFF positions at
    the root and in branches (SimOxide's instructions carry their action ids, so a program
    counter maps to Slingshot's cursor, which points past the waiting action); nested external
    calls with call-over-wire requests; single-demand internal actions; PS remaining demands;
    the FCFS head's remaining demand and the queue (SimOxide writes a better one than
    Slingshot); scenario and operation response-time calculators; SPD adjustor counters, cooldown
    and last enactment; ids (fresh refIds).
  - *Lossy* (by design, as Slingshot's own): RNG state (reseeded), same-time order (a
    `HashSet`), sliding windows, monitor buffers and SPD trigger aggregators (reset), absolute
    time (rebased to 0; import needs the local snapshot time from the parent `.state`), frame
    identity and number types, FCFS queue order on import, the collection iterator (SimOxide's
    fixed behaviour pops frames per iteration), queued SPD requests (→ policy ids).
  - *Semantically 1:1, but unreadable by Slingshot*: usage loops and branches, SEFF loops,
    infrastructure calls, linking jobs (export only to SimOxide or a fixed reader).
  - *Impossible*: passive-resource holders and waiters in the native format (an untested
    workaround: synthetic SEFF-progress events at the acquire action); forks in the fast flavour
    (Slingshot's fork holder keeps a Java iterator; at ≥ 2 branches it throws, Q23); an internal
    action with several demands mid-action (concurrent in Slingshot, sequential in SimOxide's
    fixed behaviour).
  - *ssx* mirrors Slingshot's entity graph almost 1:1 (users, behaviour contexts, holders, RPCs,
    requests, jobs), so its mapping is structural, but it has no pause yet (N1 notes).
- **Effort for N3b** (fast flavour): gson-shaped serde model with envelopes, `Optional`, refId
  arena, PCM reference writer and resolver, type coercion 3-4 d; export (processes,
  continuations and frames → entity graph; jobs; timers; calculators; `statevalues`; rebasing; a
  strict "Slingshot-readable" mode that refuses or warns on loop, branch, fork, infrastructure and
  link positions) 5-7 d; import (rebuild processes, continuations, frames, jobs, timers,
  calculators and SPD state; handle Slingshot's artefacts: stale events, absolute times, fake
  jobs, lost waiters) 5-7 d; `.config` onto N2 (incoming policies = enforced policies, needs P2;
  SLO trigger = `Condition::Slo`; maximum duration; drop rules) 2 d; tests 3-4 d; a `.state`
  writer +2-3 d. **Both directions about 4-5 w; export plus `.config` and a lite `.state` about
  2.5-3 w** (fits the planned 2-3 w); import of Slingshot-written snapshots as a follow-up
  (+1-1.5 w); ssx: +1 w for its pause and resume, then +1 w. Risks: the format is Slingshot's
  private field layout through reflection (pin to the S0 nightly); Slingshot's snapshots are partly
  wrong (choose the intended semantics, document the artefacts); `statevalues` need MENTOR's SPD
  branch; frame layouts differ; refId collisions.
- **Round-trip test models** (`corpus-ss/`): `ss_closed_think` (closed, think time, FCFS),
  `h05_open_workload` (open, FCFS, usage delay), `h03_fcfs_hdd` (PS and FCFS in one container:
  shows the classification bug), `ss_fifo_ties`, `h01_ps_single`, `h20_ps_multicore` (PS),
  `ss_passive` (passive: the waiter is lost), `h14_call_chain_3` (nested calls over wires),
  `h08_guarded_branch_params` (SEFF branch with parameters: frames), `h22_usage_behaviour`;
  negative cases `h09_loop` and `h19_linking_resource` (Slingshot cannot read them back); SPD
  `spd_minimal`, `spd_scale_in`, `spd_time_policies` (`reference/slingshot/models`).
- **MENTOR application (U7): not available locally.** Only sources (`initialisedsimulation.
  application`, a headless launch configuration); no built jars, no `~/.m2`/`~/.p2`; the S0
  install lacks gson, jgrapht, the cost extension, ExperimentAutomation (only SimuLizar 5.2.2's
  jars exist), mdsdprofiles and the SLO metamodel. The branches exist remotely (SPD interpreter
  `stateexplorationRequirements`, ExperimentAutomation `slingshot-impl`), but building the stack
  needs a Tycho build and a port of the SPD branch to the nightly's layout (1-3 d). A round trip
  with the MENTOR app stays an open question for U7.
- **Evidence.** With the S0 runner and a probe bundle holding verbatim copies of the extension's
  `snapshot.data` and `snapshot.serialization` classes (SPD state stubbed: MENTOR branch missing),
  real `.snapshot` files were written and read back by the extension's own deserialiser for 10
  models (JVMs one at a time, `nice -n 19`, `-Xmx2g`; files in the spike's scratch directory, not
  committed): 8 read back (closed think, h03, h05, ss_passive, h14, h08, h22, spd_minimal with a
  `SimulationTimeReached`), 2 failed as above. The restore side was not run: it needs
  `InitWrapper`/`SnapshotConfiguration` of the `initialisedsimulation` bundle and so
  ExperimentAutomation (a probe that injects the events as `SnapshotInitFromBehaviour` does: about
  1-2 d).

### 5.8 Performance considerations

- Without SPD, rules or snapshots, nothing changes: the overlay is untouched, there are no
  observers, and the session is lazy. Guard: `bench/golden.sh` and the warm benchmark within
  noise.
- Adjustments:
  - QVTo time is µs to ms per run (power-consumption `scaleUp` 7.8 µs with 6 containers,
    0.86 ms with 1 000; plan §1);
  - the syncer rebuild is about 0.5 ms;
  - adjustments are rare compared with events.
- The SPD observer path is per tuple: a branch on an empty list when no policy listens; a few
  arithmetic operations otherwise.
- Fork cost: dominated by stored measurement rows. Forks for exploration should run with
  `store_measurements = false`, or flush rows to the parent first.

### 5.9 What the shared EMF graph must provide

The runtime-model-change foundation, both flavours and snapshots put these requirements on the
foundation graph. The merge design should treat them as constraints.

| Requirement | Why | Notes |
|---|---|---|
| **Mutable per run, cheap to share** | the `CompiledModel` stays immutable and shared; each reconfiguring run needs its own graph | copy-on-write (`Arc` of object chunks, or a persistent vector), so a run that never changes the model pays nothing and a fork shares unchanged objects |
| **Change journal** | syncers need every effective change, side effects included (opposites, containers, roots), from QVTo and from direct edits alike | the Q6 `Journal` semantics; recording switchable at no cost when off; a cursor per consumer |
| **Typed access by id** | the compiler and syncers map changes back to typed entities | every typed entity keeps its object id (as today) |
| **Stable, deterministic ids** | exactness of reconfiguring runs (SimuLizar flavour), resume equals uninterrupted, reproducible forks | object ids appended in creation order; never reused (tombstones); `xmi:id`/UUID policy seeded from the run (Q6 `IdPolicy::Uuid { seed }`); the id generator state is part of `RunState` |
| **Deterministic iteration** | Java `HashMap`/`HashSet` order emulation in QVTo compat mode and in SimuLizar's own orders | the Java-compat foundation crate; no dependence on Rust hash seeds or addresses |
| **Clone = value** | `RunState::clone()` for forks | no interior pointers or `Rc` cycles; references are ids |
| **Serialisable** | snapshots to disk | `serde` of the graph delta against the base model, plus an XMI writer for readable output |
| **Several metamodels in one graph** | PRM, SPD, semanticspd, user metamodels next to PCM, references across them in both directions | removes the Q6 two-homes limitation |
| **Resources** | QVTo extents, `qvto:/created`, per-file output | resource table part of the graph state |
| **Fast read path** | the simulator never reads the graph in the hot path (it reads compiled tables); QVTo reads it per adjustment | no locking; `&mut` per run |

## 6. Performance comparison plan

This extends `bench/compare/` with the same method: warm, cold, sustained, parallel; seeds
1…N; a quiet machine (memory: full runs only then).

- **Simulators**:
  - SimOxide SimuLizar flavour (exact and fast);
  - SimOxide Slingshot flavour;
  - refsim and stock SimuLizar (rules runs);
  - Slingshot via the S0 reference runner (Equinox, stub bundle): `stock` mode,
    `-instrument false -measurements false`. Warm runs use `-runs N` in one JVM.
    - `bench/compare/lib.sh`'s `slingshot-osgi` now uses it.
    - The flat-classpath driver cannot run QVTo (§4.1).
  - First times, from the S0 smoke run on a shared VM (indicative only):
    - 2.3-3.4 s for a cold run, 3.3-4.5 s for the whole process;
    - 0.17-0.45 s warm, for 2 500 to 46 000 bus events.
- **Workloads**:
  1. Non-SPD: `x_ss_minimal`, `x_espresso`, `x_ss_mediastore` in the Slingshot flavour vs
     Slingshot (like for like semantics), short and long.
  2. SPD: the minimal example (time trigger); time-triggered scale-out and scale-in; CPU
     utilisation (EMA) with an open workload; response-time trigger; ServiceGroup top-down;
     a long run with many adjustments (oscillation with cooldown and thrashing).
  3. SimuLizar rules: the load-balancer and power-consumption examples in the SimuLizar flavour
     vs refsim and SimuLizar.
  4. Snapshots:
     - SimOxide: fork cost (clone), serialize and deserialize time and size, resume time;
       state-space exploration throughput (k forks per state, n levels);
     - Slingshot initialisation extension: snapshot JSON size and time, run-from-snapshot
       time.
     - This needs the MENTOR application. It is not installed and needs branch-only
       ExperimentAutomation (decision U7). Without it, compare against Slingshot's documented
       mechanism only.
- **Metrics**:
  - wall time per run;
  - simulated s/s;
  - share of time in adjustments (QVTo, syncers);
  - peak RSS;
  - forks per second.
- **Fairness**:
  - Slingshot's SPD adjustment includes its EMF/QVTo cost, as SimOxide's includes qvto-rs.
  - State that Slingshot results are not reproducible (`Math.random`).
  - Same models, same simulated time.

## 7. Phased plan

Estimates are human-developer time for someone who knows SimOxide, as in the integration plan.
An "agent phase" is a unit one agent can finish and verify on its own. Larger phases are split
(a/b/c).

| Phase | Work | Depends on | Estimate | Exit criteria |
|---|---|---|---|---|
| **F0** | Branch hygiene: `reconfiguration` feature; **fix generic supertypes and `PCMBaseClass` in the main tables** (5.6); append SPD/semantic/measuring-point packages. QVTo is used through the Q6 adapter until the merge lands, then through the in-repo `crates/qvto/*` | — | 1-2 d | builds with and without features; golden hashes unchanged; `IntSample` test; SPD files load |
| **M** | *(merge, designed separately)* QVTo crates and foundation crates in the repo; QVTo on the shared graph; the graph meets 5.9. *M0-M3 done 2026-10-17 on branch `qvto-merge`* (QVTo in `crates/qvto/` with history, foundation Java-compat crates, EPL-only defaults with `exact`/`compat` opt-in; status in `crates/qvto/docs/merge-into-simoxide.md`); M4-M8 open | — | see the merge design | F3 switches from the adapter to the shared graph without API change |
| **F1** | *Done 2026-10-17* (findings: 5.7 "F1 as built"). `RunState` extraction: all mutable run state in one struct; `Rc` → `Arc` or deep-clone decision; `clone()` | F0 | 1 w | met: fork, pause/resume and resume of a deep copy on another thread equal the uninterrupted run on every model directory (exact own/replay, fast, literal engine); golden hashes, corpus, fuzz corpus unchanged; benchmark within noise |
| **F2** | *Done 2026-10-17* (findings: 5.3 "F2 as built"). `RunModel` overlay (D1) with rebuild-and-diff syncer; direct-edit API | F0 | 2-3 w (a: overlay; b: rebuild/diff; c: edit API) | met: a test edit adds a container mid-run and it receives load; changes before the start equal the changed model byte for byte; runs without changes unchanged (golden, corpus, fuzz corpus, fork tests); benchmark within noise |
| **F3** | *Done 2026-10-17 on the Q6 adapter* (findings: 5.5 "F3 as built"; the switch to the shared graph after M6 is open). QVTo host `simoxide-reconf`: shared immutable core (registry, metamodel objects, compiled programs), lazy per-run state, journal to syncers, natives; first on the Q6 adapter, then on the shared graph (after M) | F0 | 1 w (+2-3 d to switch after M) | met: the 24 corpus cases run on a `RunModel` mid-run; journals equal the standalone runs', models, logs and status the Java runner's |
| **F4** | *Done 2026-10-17* (findings: 5.3 "F4 as built"). Incremental syncers incl. SEFF recompilation, call-cache invalidation, monitor cloning, removal policies (keep / abort / drain) | F2, F3 | 2-3 w | met: each change kind has a test (`f4.rs`); a load-balancer branch added mid-run changes routing, executions in progress run the changed chain; scale-in in all three policies (and Q5) in both flavours; snapshots after each kind continue byte for byte; incremental syncers not needed (a change costs 0.3 ms on MediaStore) |
| **R1** | *Done 2026-10-17* (findings: 5.5 "R1 as built"; F4 remainder listed there: monitor, population and in-progress behaviour changes fail or deviate until F4). SimuLizar rules: PRM values (D2), reconfiguration process runs rules (D3), MEAS-7.3 series | F3, F4 | 1.5-2 w | met: the load balancer and power examples (and SimuLizar's reconfiguration test, a scale-out and an added SEFF branch) run, byte-identical to the patched reference in the exact build; statistical match of the default build (30 seeds) |
| **R2** | *Done 2026-10-17* (findings: 5.5 "R2 as built"). Exact rules: deterministic UUIDs in refsim (D5), byte-exact comparison | R1 | 1 w | met: byte-equal trace, tape and measurements on all 11 `corpus-rules/` cases (5 new: objects created without ids, a `Set`-order rule, SimuLizar's adaptation steps, `deepclone`) in the exact build, 30/30 seeds each; the EPL build makes the same rule decisions (ordered QVTo) and matches statistically |
| **S0** | *Done 2026-10-17.* Slingshot reference runner `reference/slingshot/`: stub bundle, scripts with private config areas, trace/tape/CSV/SPD-log extension, seed agent (StoEx, `Math.random`, UUIDs), `det` ordering rule and PS patch, smoke corpus | — | 1 w | met: SPD minimal example (and 8 more models) byte-reproducible; CSV in SimOxide's format |
| **S1** | *Done 2026-10-17.* Slingshot semantics spec `docs/spec/slingshot/` (10 pages, rules `SS-*`, quirks Q17–Q37 added to Q1–Q16): event core and bus, usage, SEFF, resources, monitoring, random streams, SPD, quirks, exact comparison (projection, tape mapping); 20 probe models with `det` outputs (`models/probes/`, `probes.sh`); draw sites on the tape; `det` verified on 88 models (`verify-det.sh`, `results/det-verify.csv`) | S0 helps | 1.5-2 w | met: every difference has a rule with a source citation; 20 probes and the corpus runs demonstrate them (§2.1) |
| **S2** | *Done 2026-10-17* (findings: 5.2 "S2 as built"). Slingshot flavour, fast mode: `Flavour` trait, hooks of 5.2, Slingshot monitors and windows, stop, branch stream, quirk compat flags (U5) | F1, S1 | 4-6 w (a: trait and SimuLizar impl, golden unchanged; b: usage, stop, RNG; c: resources; d: monitoring) | met as sharpened in 5.2 "S2 as built": `x_ss_*` byte-identical to `det` (exact RNG, compat flags; 5 models x 3 seeds), also 16/20 probes and 38/45 corpus models; `x_ss_mediastore` statistically equal to stock (30 seeds, 0/26 tests rejected, both RNG modes); `x_espresso` equal on every order-independent series, its event-sampled state series is order-dependent (Slingshot's own `det` order fails the same test) and left to X2 |
| **X1** | *Done 2026-10-17* (findings: 4.1 "X1 as built"). Slingshot-exact oracle hardening. Done in S1: trace projection (`tools/project.py`), draw sites on the tape (agent hooks, no patch), `det` verification on probes and corpus. Remaining: corpus `corpus-ss/` with `det` reference outputs (probes, SPD examples, the 38 corpus models without failures, chosen failure models), a static audit that no class subscribes twice to one event type without disjoint reification, SPD corpus runs in `det` (P2 models) | S0, S1 | 0.5-1 w | met: all 88 `corpus-ss/` cases byte-reproducible in fresh JVMs; projection documented (SS-X-2, SS-X-8); audit clean |
| **X2** | *done 2026-10-17* (findings: 5.2 "X2 as built", "X2d as built"). Slingshot-exact mode, **level E directly** (U14): the per-handler cascade as a FIFO queue of handler steps (insertion-ordered results, FIFO after queued events, direct `MeasurementMade`), double time, PS stale-event samples and zero-demand jiffy, `java.util.Random` branch stream, `SimulationFinished` order, all 37 quirk flags on, **handler disposal** (Q18: a disposed bit per handler, Slingshot's partial side effects up to each throw: Q17, Q19, Q21-Q24, Q30, Q31, Q37), level-E projection writer | S2, X1 | 7-10 w (a: handler-step queue, event order and time, 2-3 w; b: resources and monitoring, 1.5-2 w; c: handler disposal and the throwing quirks, 1.5-2.5 w; d: SPD events, 1 w; e: level-E projection and corpus debugging, 1-1.5 w) | V3x: level-E projection, tape (with positions) and measurements byte-equal with `det` on the Slingshot corpus, including the 22 models with handler errors. *Status:* 89 of 90 `corpus-ss/` cases (88 in the EPL build), 21 of the 22 handler-error cases, the 3 SPD cases (and `corpus-spd/` 5 of 5, with P2's `spd_same_instant`) with their decision logs; h27 (`HashMap` order) and h21 in the EPL build are licence-bound |
| **Q2** | *Done 2026-10-17* (findings: 5.5 "Q2 as built"). qvto-rs insertion-order policy for `Set`s (the `det` oracle's QVTo/OCL rule), opt-in for the Slingshot-exact mode: `Mode::Ordered` (EPL) | Q1 | 3-5 d | met: SPD corpus adjustments equal the `det` Java runner's byte for byte, including container and link order (Q16); also the whole suite (589/589) and the 13 adjustments of the S0 Slingshot `det` runs |
| **P1** | *Done 2026-10-17* (findings: 5.4 "P1 as built"). SPD interpreter: model view, filters, aggregators, state, stimulus observers, time events | F0 (F1 for state) | 1.5-2 w | met: V1 unit tests; decision log equals the reference runner's on the time-triggered models (5 SPD models, 83 records, compat flags on) |
| **P2** | *Done 2026-10-17* (findings: 5.4 "P2 as built"; the F4-dependent part finished after F4). SPD adjustment via `MainTransformation`; `ModelAdjusted` diff; corpus `corpus-spd/` with reference outputs | P1, F3, F4 | 2 w | met: minimal example and `spd_time_policies`, `spd_service_group`, `spd_scale_in`, `spd_same_instant`: same scale times, container and assembly names, models as Slingshot's (V3, model summaries of every adjustment equal), in both flavours |
| **P3** | *Done 2026-10-17* (findings: 5.4 "P3 as built"; V3x with X2d). SPD in the Slingshot flavour end to end; statistical scenarios; exact SPD runs (with X2, Q2) | P2, S2 | 1.5-2.5 w | met: V4 on a utilisation and a response-time scenario (0 of 41 SPD statistics rejected, 30 seeds); V3x: the five `corpus-spd/` cases byte-equal at level E |
| **Q1** | *Done 2026-10-17* (findings: 5.5 "Q1 as built"). qvto-rs: SPD adjustment corpus for every path (V2) incl. service group, competing consumers, scale-in: 47 cases, 73 of 105 operations (all of `MainTransformation`'s call graph) | — | 3-5 d | met: strict match with the Java runner (47/47, also on identity-hash-dependent outputs); two engine discrepancies and three harness ones fixed |
| **N1** | *Done 2026-10-17* (findings: 5.7 "N1 as built"). Snapshot serialisation: serde of `RunState`, XMI writer for the overlay, versioned format | F1, F2 | 1.5 w | met: serialize → deserialize → identical continuation, byte for byte, on every model directory in exact and fast mode and both flavours (Slingshot-exact engine: refused, no pause yet; done later, 5.7 "ssx snapshots as built"), after model changes, with SPD, with SimuLizar rules and a QVTo state; deterministic bytes |
| **N2** | *Done 2026-10-17* (findings: 5.7 "N2 as built"; enforced policies built in P2, 5.4 "P2 as built"). Pause/resume API with change sets, enforced policies, snapshot conditions (duration, trigger, threshold) | N1, P2 | 1 w | met: resume-with-change tests (change files, QVTo transformations, rule sets; byte-equal to the in-process change, exact and fast; the Slingshot flavour refuses until F4); pause-condition tests; fork-based exploration example (`explore.rs`, `simoxide explore`), deterministic |
| **N3** | Slingshot `.snapshot`/`.config`/`.state` interop: a: spike on the format mapping; b: export/import. *a done 2026-10-17: go for the fast flavour, export first, lossy* (5.7 "N3a spike"; real `.snapshot`s written and read back on 8 of 10 models; the MENTOR app is not available locally) | N2, S2 | a: 3 d; b: 2-3 w (export + `.config` + lite `.state`: 2.5-3 w; both directions 4-5 w) | a: met (verdict in 5.7); b: round trip with the MENTOR app on a model (if U7) |
| **B1** | Benchmarks of §6 (both Slingshot modes) and docs pages | S2, X2, P3, N2 | 1-1.5 w | tables in `docs/performance/` |
| **D** | Docs: guide pages (SPD, reconfiguration, snapshots, flavours), spec rules, deviations | continuous | +10 % | — |

**Parallelism** (2-3 agents at a time):

- Day 1: **F0**, **S0** and **Q1** are independent.
- After F0:
  - **F1** ∥ **F2** ∥ **F3**: they touch different files. F1 is `sim.rs` state, F2 is the
    overlay, F3 is the new crate.
  - **S1** runs alongside, as a reading and probing task.
  - **P1** can start on F0 alone (pure logic plus unit tests).
- Critical path: F0 → F2 → F4 → P2 → P3 → B1, with S2 in parallel from F1 and S1.
- The Slingshot-exact mode (X1 → X2, Q2) follows S2 and runs in parallel with P2, P3 and the
  snapshot phases. It is not on the path to the first usable autoscaling.
- R1/R2 (SimuLizar rules) and N1/N2 (snapshots) branch off after F4 and F1+F2 and can run in
  parallel with S2.

**Totals**:

- about 35-49 weeks of work: 26-36 before, plus about 9-13 for the Slingshot-exact mode (X1, X2,
  Q2 and the extra share of P3 and B1). X2 grew from 4-6 to 7-10 weeks with U14 (level E directly,
  handler disposal);
- with three agents in parallel, about 13-18 weeks of calendar time (the exact mode is off the
  critical path, so the calendar grows less than the work);
- SimuLizar rules (F0-F4, R1, R2) alone: 9-12 weeks of work, matching plan steps 1-6;
- SPD on the SimuLizar flavour without a Slingshot flavour (F0-F4, P1, P2): about 10-13 weeks;
  this is the earliest usable autoscaling.

**Risks**:

| Risk | Mitigation |
|---|---|
| Slingshot semantics differ in more places than expected (S1) | S1 before S2; the flavour hooks are added per finding; accept statistical equality |
| `Flavour` hooks slow the SimuLizar path | monomorphization; golden hashes plus warm benchmark per phase |
| SEFF and connector changes mid-run vs compiled flat code | recompile per changed component; per-site caches invalidated by a generation counter |
| Slingshot moves (nightly only) | pinned: `reference/slingshot/install.lock` (SHA-256 of 47 jars) and source commits; `setup.sh` warns on change |
| Slingshot-exact needs more ordering fixes than the smoke corpus shows | X1 audit; the ordering rule is generic (all Slingshot, QVTo, OCL classes); new models go through `smoke.sh` first |
| Exact emulation of the bus cascade costs speed | exact mode is opt-in; the fast mode stays the default and is benchmarked separately |
| Level E with handler disposal (U14) needs Slingshot's partial side effects at each throw | one throw site per quirk, each with a probe or corpus model (`results/det-verify.csv`, column `handler_errors`); level M stays available as a debugging aid |
| Reference SPD runs need the stub bundle; the real `ProfilesLibrary` might behave differently | the main path never calls it (checked in qvto-rs and by grep) |
| `Rc` → `Arc` cost in the hot path | measure; fall back to deep copy on fork |
| `.snapshot` mapping infeasible for some actions | N3a spike decides; interop can be export-only |
| Q8 Java-hash order in top-down placement | document; seed-independent but not Java-equal |
| Shared machine: Q5b/Q7 runs are timing-sensitive | niced builds, own target dirs, `CARGO_BUILD_JOBS=2`; benchmarks only on a quiet machine |

## 8. Decisions for the user

Decided on 2026-10-17 (U3-U6, U12-U14); the others are still open.

| # | Decision | Recommendation / decision |
|---|---|---|
| U1 | Merge `qvto-adapter` and later `spd` into SimOxide `main`; when | merge `qvto-adapter` with F0, `spd` per phase |
| U2 | *Decided*: QVTo moves into the SimOxide repository with shared foundation crates. Still open: whether `reconfiguration`/`spd`/`snapshot` stay optional cargo features (smaller default builds) or are always on | optional features, on in the CLI build |
| U3 | *Decided*: the user discusses licensing of Slingshot's transformations and the SPD `.ecore` files with the Palladio maintainers personally. Development proceeds | embedded `.qvto`/`.ecore` files (and the SPD example models in `reference/slingshot/models`) are marked with source repository, revision and "licence pending confirmation"; nothing is published until confirmed |
| U4 | *Decided*: **both** a Slingshot-exact mode and a fast mode | exact: the same events, random draws and measurements as the S0 runner's `det` Slingshot (seeded randomness, deterministic same-time order, instrumentation), like the SimuLizar-exact mode against refsim; fast: semantics only, statistically equal. The S0 patches are the basis: minimal, documented, switchable (`--stock`) |
| U5 | *Decided*: Slingshot bugs are **fixed by default**; a compat flag per documented quirk (Q1-Q37) reproduces it | the exact mode against the patched Slingshot runs with all quirk flags on; the SimuLizar flavour with SPD uses fixed semantics. What "fixed" means: U13 |
| U6 | *Decided*: SPD in **both flavours** (SimuLizar and Slingshot) | the SimuLizar-flavour SPD has no reference; it is the earliest usable autoscaling and shares all code |
| U7 | Install the MENTOR initialisation application and branch-only ExperimentAutomation for snapshot interop tests and benchmarks | only if `.snapshot` interop (N3) is wanted. N3a: not available locally; building it needs a Tycho build of the branch-only bundles and a port of MENTOR's SPD branch (1-3 d) |
| U8 | `ProfilesLibrary`: stubs only, or real stereotype support (EMF Profiles) for rules that use it | stubs (the S0 runner uses a stub bundle too) |
| U9 | Usage evolution (DLIM) in SimOxide: needed for realistic autoscaling load curves | yes, as a separate later phase (about 1-2 w); the S0 runner does not load usage evolution yet |
| U10 | Write changed PCM models as XMI (snapshot model state, MENTOR-like output) with the foundation's XMI writer | yes, in N1 |
| U11 | Scope of the Slingshot flavour's monitors and EDP2-like output | CSV as now; MENTOR's `.state` measurement sets only with N3 |
| U12 | *Decided*: licensing structure for GPL-derived exactness code | "exact-mode crates with the GPL+EPL constraints, fast mode without": GPL-derived code for exact or compat modes (qvto-runtime's OpenJDK-`HashMap` port, the existing `hotspot-math` feature, …) lives in separate opt-in crates or features; the default (fast) build stays EPL-only (§5.1) |
| U13 | *Decided*: the **fixed (default) behaviour principle** | wherever Slingshot's behaviour is a bug, the fixed behaviour is SimuLizar's (SimOxide's) semantics: asynchronous forks run (detached, ACT-9), delay resources work, no handler is disposed (a handler exception is a run error), several demands of one internal action are served one after the other (ACT-3.1), the response-time monitors are honoured, the maximum measurement count is honoured, … Only genuine Slingshot design differences stay Slingshot-specific (double time, FIFO cascade, stop at `simTime` without drain, instantaneous utilisation, first allocation match, `Math.random()` branches, links always simulated, …; `docs/spec/slingshot/quirks.md`). For SPD quirks without a SimuLizar counterpart the fixed behaviour is the evident intent of Slingshot's code or of the SPD metamodel. Every quirk Q1-Q37 keeps a compat flag that reproduces it; the exact mode turns all of them on. The fixed behaviours are listed in `quirks.md` |
| U14 | *Decided*: the Slingshot-exact mode targets **level E directly** | full projected event-trace equality (SS-X-2, level E: passed elements, calls, requests, jobs, grants, user ends, plus level M), the tape with positions, and the measurements, including the per-handler cascade and handler disposal (Q18). Level M is not a separate milestone; it stays as a subset for debugging. X2 re-estimated to 7-10 w (§7) |

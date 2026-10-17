# SimOxide: runtime reconfiguration, a Slingshot mode, SPD and snapshots (design)

State: 2026-10-17. This is a design; nothing in it is implemented yet. It is on branch `spd`
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
- The **Slingshot flavour is a semantics mode**: it is statistically equal to Slingshot. It is
  exact only on deterministic models.
- The oracle is a headless Slingshot reference runner. **It works on this machine** (§4.1).

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
| plan | `qvto-rs/docs/simoxide-integration.md` (steps 1-7, D1-D6); `qvto-rs/crates/qvto-simoxide/README.md` |

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
    disappear and their response times are never recorded. This is read from the code and not
    tested; S0 confirms it with a reference run.
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

### 1.5 Quirks to reproduce (Slingshot flavour) or to fix (option)

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
| Q15 | Policies of one group react to each other's `SimulationTimeReached` | `TargetGroupChecker.java:36-40` |

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
    - The order of registration comes from the extension and Guice set-up. S1 checks whether it
      is stable across runs.
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
    `java.util.Random` (`GeneralSimulationConfigurationBehavior.java:32-35`). Slingshot never
    seeds it. The reference runner injects a seeded generator
    (`work-slingshot/headless-app/src/slingshot/headless/SlingshotRunner.java:123-128`).
  - Inter-arrival times, think times and linking latencies are StoEx, so they come from the same
    generator.
  - Branch decisions in SEFFs and usage models, and link failures, use **unseeded
    `Math.random()`** (`[SS]…common.utils/…/TransitionDeterminer.java:156-172`, "TODO: Use SimuCom
    Random instead"; `SimulatedLinkingResource.java:53`). The cumulative-sum algorithm is the
    same as RND-5; the stream is different.
  - So **Slingshot runs are not reproducible**: not with branches, and not with same-time ties.
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
- **Comparison with SimOxide so far**:
  - Slingshot matches SimOxide **exactly on the deterministic Slingshot E2E models** (`x_ss_*`)
    and closely on the others;
  - it aborts on acquire/release without a `ParametricResourceDemand`;
  - it fails on synchronous forks (`h11_fork_sync`, `x_pem_fork`) (`docs/performance/comparison.md`).
- Phase S1 writes all this down as rules `SS-*`, like SimOxide's `SIM-*`/`MEAS-*`, each with a
  source citation and, where possible, a probe run.
- **QVTo in Slingshot.**
  - The only QVTo use is the SPD adjustment (`[ADJ]adjustment/qvto/*`), which is a port of
    SimuLizar's QVTo executor and model cache.
  - Slingshot's other extension mechanism is Java behaviour extensions (`@OnEvent`, Guice
    modules), not QVTo.
  - The SimuLizar flavour's reconfiguration rules are the other QVTo extension point (§5.5).

**Exact or semantics mode for Slingshot?** Recommendation: a **semantics mode**. It is
statistically equal and trace-equal on deterministic models.

A Slingshot-exact mode would need all of these:

1. A patched reference: seeded `Math.random()` and a recorded draw tape, as `refsim` does for
   SimuLizar.
2. A deterministic order of everything that iterates hash-based collections: the snapshot's
   event set, Guice/extension registration, and EMF `HashMap`s in the behaviours.
3. Emulating every bus event and its scheduling, the 20× event count included. That defeats
   the performance goal.
4. Java identity-hash order in QVTo `Set`s (Q8).
5. Following a nightly-only target. Slingshot has no releases, so the pin would be a nightly
   date.

Effort would be 2-3 times that of the semantics mode, with no user value beyond validation.
Exactness on deterministic models (constant demands, no branches, deterministic arrivals)
already gives event-level checks without any of this.

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
- Passive-resource holders and wait queues. Only active and linking jobs are recorded. **To
  verify in S0.**
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

## 4. Oracles and validation

### 4.1 Slingshot reference runner (works headless here)

Checked today, niced, `-Xmx2g`, about 2 s per run:

- **Flat classpath** (`slingshot.headless.PlainMain`): the simulation runs and the SPD trigger
  fires at t = 50. But QVTo cannot resolve the imports of `MainTransformation`
  ("Cannot find imported compilation unit 'pcm.helpers.Commons'"). The adjustment fails
  silently: "RECONFIGURATION WAS false".
- **Equinox** (`-application slingshot.headless.app`): the imports resolve. Compilation fails
  because `org.palladiosimulator.architecturaltemplates.catalog` (the `ProfilesLibrary`
  blackbox) is **not installed**. Again "RECONFIGURATION WAS false".
  - **The shipped nightly cannot run any SPD adjustment.**
- **Equinox plus a stub bundle** works:
  - The stub is qvto-rs's `reference/runner/src-stubs/…/ProfilesLibrary.java`, packaged as bundle
    `org.palladiosimulator.architecturaltemplates.catalog` with a `javaBlackboxUnits` extension.
  - It is added through a private `bundles.info`.
  - Result: "RECONFIGURATION WAS true". `AppServer_1` is created and gets load:
    `CPU [0] in AppServer_1` utilisation n = 12 over 50 s.
  - The stub's sources, manifest, `plugin.xml`, `config.ini` and the command line are in the
    session scratchpad. Phase S0 moves them into `reference/slingshot/`.
- Notes:
  - The scripts in `work-slingshot/` still point at the old path
    `/home/devbox/workspace/palladio-research`. Use private configuration areas; never write
    into `old/`.
  - `default.slo` must be left out: the SLO metamodel is not installed.
  - The minimal example's `default.experiments` stops at 10 s, so its t = 50 trigger never fires
    there.

**Instrumentation for comparisons** (phase S0), all outside Slingshot's jars:

- an extra behaviour extension (its own bundle) that writes:
  - an event trace: time, event type, entity key; no UUIDs;
  - the measurements as SimOxide's CSV format;
  - an SPD decision log: filter results, `ModelAdjustmentRequested`, the QVTo log, `ModelAdjusted`
    with the container and assembly lists;
- seeding: the existing injection into the StoEx factory, plus a patched `TransitionDeterminer`
  (a seeded `Random` instead of `Math.random()`), deployed as a fragment or an earlier bundle.
  It is used only for reproducible references, never to claim exactness.

### 4.2 Validation levels

| Level | What | Oracle |
|---|---|---|
| V1 unit | filters, constraints, aggregators, comparator, adjustment arithmetic: edge cases from §1.2 (`==` at `coolDownEnd`, `t % delta == 0`, EMA emit rule, 10-value window) | hand-computed, from the Java code |
| V2 QVTo | every adjustment path (EI step ±, relative, absolute, size clamp, service group top-down and bottom-up, competing consumers, scale-in) on SimOxide-loaded models | qvto-rs Java runner, strict (extends the one SPD corpus case, `qvto-testkit/cases/corpus.cases:347-366`) |
| V3 deterministic runs | SimulationTime triggers, constant demands, deterministic arrivals: same adjustment times, container and assembly names, model after each step, response times | Slingshot reference runner: exact on these values |
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
`qvto-rs/docs/merge-into-simoxide.md`, written separately). The design below assumes:

- QVTo crates under `crates/qvto/*`, as workspace crates;
- foundation crates shared by QVTo and SimOxide: Java compatibility (hashing, number formatting,
  `HashMap` order), and the EMF registry, object graph and XMI;
- QVTo running **directly on the shared graph**. The Q6 adapter (`qvto-simoxide`, two homes,
  mirrors) is only a bridge until then.

Names marked *foundation* follow the merge design.

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
| time, event list | i64 ns, FIFO `(t, seq)` (SIM-2, SIM-3) | the same list; double time converted at the edges | none: shared |
| process model | DESMO-J processes, `Resume` notes, elided hand-offs | the same machinery; Slingshot's same-instant order only where SS-* says it changes results | a few ordering hooks |
| branch draws | the reference stream (exact) or fast | a separate stream ("`Math.random`" stream), always fast-style | `F::branch_rng` |
| StoEx draws | as now | fast RNG; no tape | `C` = `Fast` only |
| resources | SimuLizar schedulers (`simoxide-sched`) | PS as SimuCom's plus zero-demand jiffy and the extra stale-event samples; no delay resources; no HDD rates | scheduler variant enum |
| actions | ACT-* | several demands of one InternalAction run concurrently; async forks ignored; first allocation match | hooks in the action interpreter |
| usage | WL-* | Slingshot's draw order for inter-arrival and think times; aborted closed users restart; usage evolution later (decision U9) | hook per workload kind |
| monitoring | probe framework, MEAS-6 windows, PRM | instantaneous utilisation samples, no windows; processing types not recorded; `MeasurementMade` stream | `F::Monitors` |
| stop | stop conditions, post-stop drain (SIM-6) | `SimulationFinished` at `simTime` before other events at that time; queues cleared; no measurement count | `F::stop` |
| removed container | keeps simulating (SimuLizar gap); "fixed" option drains | one job aborted, the rest lost (Q5); "fixed" option drains | syncer policy |
| reconfiguration trigger | PRM writes → reconfiguration process (MEAS-7.2) | SPD filter chain on measurements and time events | `F::Reconf` |
| exactness | byte-exact with `C = Exact` | statistical; exact on deterministic models | — |

The aim is that the Slingshot flavour does **not** emulate Slingshot's bus events. It emulates
the timing and ordering rules that change results. S1 decides which ones those are.

### 5.3 The runtime-model-change foundation (shared)

This follows plan D1-D4 (`qvto-rs/docs/simoxide-integration.md` §3), with these additions:

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
  - **Removal policy** per flavour: keep simulating (SimuLizar), Slingshot's (abort the job with a
    pending event, lose the others, Q5), or drain (fixed, the default for SPD in the SimuLizar flavour).
  - **Linking-resource rewiring** (Q6): the route table is rebuilt.

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
  - The importer resolves `pcm.helpers.*` and `spd.*` against the embedded tree.
  - `ProfilesLibrary` is registered with stub implementations that report an error if called.
    The main path never calls them.
  - The user's own transformations can replace `MainTransformation` (an option).

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
- **Exact vs fast.**
  - QVTo compat mode always for the adjustment, so the models equal the Java runner's.
  - The exception is Java identity-hash order (Q8): without a Java run there is no identity
    hash, so `->any` on sets of objects follows qvto-rs's deterministic order. This is
    documented as a deviation.
  - qvto-rs fast mode is an option for throughput.

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
| reconfiguration | reconfiguration-process state (`last_ns`, scheduled), rule order | small |
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
  - Slingshot via the Equinox reference runner with the stub bundle.
  - The flat-classpath driver cannot run QVTo (§4.1), so SPD runs use Equinox, warm via
    `-runs N` in one JVM.
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
| **M** | *(merge, designed separately)* QVTo crates and foundation crates in the repo; QVTo on the shared graph; the graph meets 5.9 | — | see the merge design | F3 switches from the adapter to the shared graph without API change |
| **F1** | `RunState` extraction: all mutable run state in one struct; `Rc` → `Arc` or deep-clone decision; `clone()` | F0 | 1 w | clone at event *k* and run both → equal traces (all corpus, both modes); benchmark within noise |
| **F2** | `RunModel` overlay (D1) with rebuild-and-diff syncer; direct-edit API | F0 | 2-3 w (a: overlay; b: rebuild/diff; c: edit API) | a test edit adds a container mid-run and it receives load; runs without changes unchanged (golden) |
| **F3** | QVTo host `simoxide-reconf`: shared immutable core (registry, metamodel objects, compiled programs), lazy per-run state, journal to syncers, natives; first on the Q6 adapter, then on the shared graph (after M) | F0 | 1 w (+2-3 d to switch after M) | corpus rules run on a `RunModel`; journals equal the corpus results |
| **F4** | Incremental syncers incl. SEFF recompilation, call-cache invalidation, monitor cloning, removal policies (keep / abort / drain) | F2, F3 | 2-3 w | each change kind has a test; load-balancer branch added mid-run changes routing; scale-in in all three policies |
| **R1** | SimuLizar rules: PRM values (D2), reconfiguration process runs rules (D3), MEAS-7.3 series | F3, F4 | 1.5-2 w | SimuLizar's load-balancer and power examples run; statistical match vs SimuLizar |
| **R2** | Exact rules: deterministic UUIDs in refsim (D5), byte-exact comparison | R1 | 1 w | byte-equal traces on the examples |
| **S0** | Slingshot reference runner `reference/slingshot/`: stub bundle, scripts with private config areas, trace/CSV/SPD-log extension, seeded branches | — | 1 w | SPD minimal example reproducible; CSV comparable with SimOxide's |
| **S1** | Slingshot semantics spec `docs/spec/slingshot/` (rules `SS-*`): event core, usage, SEFF, resources, monitoring, stop, model changes; differences to SIM/WL/ACT/MEAS rules | S0 helps | 1.5-2 w | every difference has a rule with a source citation and, where possible, a probe run |
| **S2** | Slingshot flavour: `Flavour` trait, hooks of 5.2, Slingshot monitors and windows, stop, branch stream | F1, S1 | 4-6 w (a: trait and SimuLizar impl, golden unchanged; b: usage, stop, RNG; c: resources; d: monitoring) | `x_ss_*` exact on deterministic models; `x_espresso`, `x_ss_mediastore` statistically equal (30 seeds) |
| **P1** | SPD interpreter: model view, filters, aggregators, state, stimulus observers, time events | F0 (F1 for state) | 1.5-2 w | V1 unit tests; decision log equals the reference runner's on time-triggered models |
| **P2** | SPD adjustment via `MainTransformation`; `ModelAdjusted` diff; corpus `corpus-spd/` with reference outputs | P1, F3, F4 | 2 w | minimal example: same scale time, container and assembly names, models as Slingshot's (V3) |
| **P3** | SPD in the Slingshot flavour end to end; statistical scenarios | P2, S2 | 1-2 w | V4 on utilisation and response-time scenarios |
| **Q1** | qvto-rs: SPD adjustment corpus for every path (V2) incl. service group, competing consumers, scale-in | — | 3-5 d | strict match with the Java runner |
| **N1** | Snapshot serialisation: serde of `RunState`, XMI writer for the overlay, versioned format | F1, F2 | 1.5 w | serialize → deserialize → identical continuation (both flavours) |
| **N2** | Pause/resume API with change sets, enforced policies, snapshot conditions (duration, trigger, threshold) | N1, P2 | 1 w | resume-with-change tests; fork-based exploration example |
| **N3** | Slingshot `.snapshot`/`.config`/`.state` interop: a: spike on the format mapping; b: export/import | N2, S2 | a: 3 d; b: 2-3 w | a: go/no-go report; b: round trip with the MENTOR app on a model (if U7) |
| **B1** | Benchmarks of §6 and docs pages | S2, P3, N2 | 1 w | tables in `docs/performance/` |
| **D** | Docs: guide pages (SPD, reconfiguration, snapshots, flavours), spec rules, deviations | continuous | +10 % | — |

**Parallelism** (2-3 agents at a time):

- Day 1: **F0**, **S0** and **Q1** are independent.
- After F0:
  - **F1** ∥ **F2** ∥ **F3**: they touch different files. F1 is `sim.rs` state, F2 is the
    overlay, F3 is the new crate.
  - **S1** runs alongside, as a reading and probing task.
  - **P1** can start on F0 alone (pure logic plus unit tests).
- Critical path: F0 → F2 → F4 → P2 → P3 → B1, with S2 in parallel from F1 and S1.
- R1/R2 (SimuLizar rules) and N1/N2 (snapshots) branch off after F4 and F1+F2 and can run in
  parallel with S2.

**Totals**:

- about 26-36 weeks of work;
- with three agents in parallel, about 10-14 weeks of calendar time;
- SimuLizar rules (F0-F4, R1, R2) alone: 9-12 weeks of work, matching plan steps 1-6;
- SPD on the SimuLizar flavour without a Slingshot flavour (F0-F4, P1, P2): about 10-13 weeks;
  this is the earliest usable autoscaling.

**Risks**:

| Risk | Mitigation |
|---|---|
| Slingshot semantics differ in more places than expected (S1) | S1 before S2; the flavour hooks are added per finding; accept statistical equality |
| `Flavour` hooks slow the SimuLizar path | monomorphization; golden hashes plus warm benchmark per phase |
| SEFF and connector changes mid-run vs compiled flat code | recompile per changed component; per-site caches invalidated by a generation counter |
| Slingshot moves (nightly only) | pin the nightly date and source commits; record the jar checksums in `reference/slingshot/` |
| Reference SPD runs need the stub bundle; the real `ProfilesLibrary` might behave differently | the main path never calls it (checked in qvto-rs and by grep) |
| `Rc` → `Arc` cost in the hot path | measure; fall back to deep copy on fork |
| `.snapshot` mapping infeasible for some actions | N3a spike decides; interop can be export-only |
| Q8 Java-hash order in top-down placement | document; seed-independent but not Java-equal |
| Shared machine: Q5b/Q7 runs are timing-sensitive | niced builds, own target dirs, `CARGO_BUILD_JOBS=2`; benchmarks only on a quiet machine |

## 8. Decisions for the user

| # | Decision | Recommendation |
|---|---|---|
| U1 | Merge `qvto-adapter` and later `spd` into SimOxide `main`; when | merge `qvto-adapter` with F0, `spd` per phase |
| U2 | *Decided*: QVTo moves into the SimOxide repository with shared foundation crates. Still open: whether `reconfiguration`/`spd`/`snapshot` stay optional cargo features (smaller default builds) or are always on | optional features, on in the CLI build |
| U3 | Licences: embedding Slingshot's transformations and the SPD `.ecore` files. The SPD repository has no `LICENSE` file; Palladio projects are EPL-2.0 by convention | confirm EPL-2.0 with the Palladio maintainers, or load the files from the user's install at run time |
| U4 | Slingshot flavour: semantics mode (statistical, exact on deterministic models) vs exact mode | semantics mode (§2) |
| U5 | Reproduce Slingshot quirks (Q1-Q11) by default, or fix them by default with a compat option | Slingshot flavour reproduces them, with a `fixed` option per quirk; SimuLizar flavour with SPD uses fixed semantics |
| U6 | Offer SPD in the SimuLizar flavour too (no reference) | yes: it is the earliest usable autoscaling and shares all code |
| U7 | Install the MENTOR initialisation application and branch-only ExperimentAutomation for snapshot interop tests and benchmarks | only if `.snapshot` interop (N3) is wanted |
| U8 | `ProfilesLibrary`: stubs only, or real stereotype support (EMF Profiles) for rules that use it | stubs |
| U9 | Usage evolution (DLIM) in SimOxide: needed for realistic autoscaling load curves | yes, as a separate later phase (about 1-2 w) |
| U10 | Write changed PCM models as XMI (snapshot model state, MENTOR-like output) with the foundation's XMI writer | yes, in N1 |
| U11 | Scope of the Slingshot flavour's monitors and EDP2-like output | CSV as now; MENTOR's `.state` measurement sets only with N3 |

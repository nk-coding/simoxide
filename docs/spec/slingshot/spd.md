# Scaling policies (SPD) in Slingshot

Rules `SS-SPD-*`. The derivation, with more source detail, is in `DESIGN-SPD.md` §1; this page states
the rules in specification form and adds the event-level facts the exact mode needs. Quirks
`SS-Q1` to `SS-Q16` are the SPD quirks Q1–Q16 of the design ([Quirks](./quirks.md#spd-quirks-q1-q16)).
Models: `reference/slingshot/models/spd_minimal`, `spd_scale_in`.

## SS-SPD-1 Set-up

- **SS-SPD-1.1** `SpdBehavior` is active when an SPD model is loaded
  (`[SPD]interpreter/SpdBehavior.java:46-49`). At `PreSimulationConfigurationStarted` it interprets the
  model once (`:51-64`): one `SPDAdjustorContext` per **active** policy (inactive ones are skipped,
  `SpdInterpreter.java:50`), one `TargetGroupState` per target group, shared by its policies.
- **SS-SPD-1.2 Order at t = 0.** `SpdBehavior` is subscribed before the monitor interpreter
  (observed `bus.jsonl`: `SpdBehavior.onPreSimulationConfigurationStarted`, then
  `MonitorRepositoryInterpreterBehavior`), so its results (the `SimulationTimeReached` events) are
  queued before the monitor events.
- **SS-SPD-1.3 Policy subscribers are registered during the run**, from the handler
  (`driver.registerEventHandler`, `SpdBehavior.java:59-62`): after all behaviours and `CoreBehavior`
  (SS-BUS-2). Each policy has one subscriber per stimulus kind of its trigger, on `MeasurementMade`
  or `SimulationTimeReached`, built with `Subscriber.builder` (no enclosing class; the time trigger's
  is named "something", `StimuliInterpreter.java:40-50`). Policies register in model order.
- **SS-SPD-1.4 The filter chain** of a policy (`entities/SPDAdjustorContext.java:80-94`), run from the start
  for each event (`entities/FilterChain.java:100-123`): target-group check; trigger checker; the
  target group's `ThrashingConstraint`s (only these; Q9); the policy constraints in model order
  (cooldown, interval; others throw); the `Adjustor`. A filter returns success or disregard; an
  exception counts as disregard. On success the handler returns `ModelAdjustmentRequested(policy)`
  (delay 0). The runner logs every evaluation in `spd.jsonl`.

## SS-SPD-2 Triggers

- **SS-SPD-2.1 Simulation time.** `SimulationTimeReached(targetGroup, delay = expected time)` is
  returned at t = 0, so it is dispatched at the expected time (FIFO behind events already queued for
  that instant). It fires if `t <op> expected` (`entity/trigger/SimulationTimeChecker.java:20-34`):
  once, with `==`, `>=`, `<=`; never with `<`, `>` on its own event. Every policy of the target group
  receives every `SimulationTimeReached` of the group (Q15).
- **SS-SPD-2.2 Measurement stimuli.** Every `MeasurementMade` of the run enters every
  measurement-driven policy's chain; the target-group check keeps those whose measuring point lies in
  the group (`utils/MeasuringPointInsideTargetGroup.java:31-60`). Supported:
  `OperationResponseTime` (each response time of the operation, unaggregated, `ExpectedTime`),
  `CPUUtilization` (utilisation tuples, `ExpectedPercentage` compared as `value / 100`), `TaskCount`
  (state tuples, `ExpectedCount`), `QueueLength` (competing-consumer groups only). Other stimuli and
  `SimpleFireOnTrend` give no checker: NPE at set-up (Q11).
- **SS-SPD-2.3 The values are instantaneous samples** ([SS-MEAS-3](./monitoring.md#ss-meas-3-active-resources)),
  only where a monitor exists, merged over all containers of the group (Q13).
- **SS-SPD-2.4 Aggregation** (`AbstractManagedElementTriggerChecker.java:53-137`): `AVERAGE` =
  `SlidingTimeWindowAggregationBasedOnEMA(60, 10, 0.2)`: `v := 0.8 v + 0.2 x; v *= exp(-(t - t_last)/60)`,
  can emit if `t_last > 60` and `t_last − t_emit > 10`, reading sets `t_emit` (so not before 60 s, at
  most every 10 s); `MAX`, `MIN`, `MEDIAN`, `SUM` = the last 10 values, emitting only with 10 values
  (Q3). Samples and the emit slot are consumed even when a later filter disregards (Q13).
- **SS-SPD-2.5 Comparison** is plain `double` (`==`, `>=`, `>`, `<=`, `<`); `NoExpectation` always
  fires; a wrong expectation type never fires (`TriggerChecker.java:65-79`).
- **SS-SPD-2.6 Composed triggers** AND/OR/XOR are broken (Q12): AND resumes after a failed child,
  OR never evaluates its children and parks the chain, XOR inverts after the first child.

## SS-SPD-3 Constraints and the adjustor

- **SS-SPD-3.1 Cooldown** (`CooldownConstraintFilter.java:28-41`): passes if `t >= coolDownEnd` or
  `numberOfScalesInCooldown < maxScalingOperations`. **Interval** (`IntervalConstraintFilter.java:13-51`):
  disregards inside the (inactive) interval: with `repeat`, `t != 0 && (t % (offset + duration) == 0 ||
  t % (offset + duration) >= offset)`; without, `offset <= t <= offset + duration`. **Thrashing**
  (`ThrashingConstraintFilter.java:37-75`): disregards opposite-sign adjustments within
  `minimumTimeNoThrashing` of the last one (absolute adjustments always pass).
- **SS-SPD-3.2 Adjustor** (`Adjustor.java:28-61`): records the adjustment time and count, the group's
  last enacted policy and time, updates the cooldown (only if `cooldownTime != 0`: `t > end` resets
  the count and moves the end, `t == end` only moves it, otherwise the count increments), and returns
  `ModelAdjustmentRequested`. The state changes when the request is made, even if the adjustment
  fails (Q2). `doOnDisregard` does not restore state (Q1).

## SS-SPD-4 The adjustment

- **SS-SPD-4.1** `ModelAdjustmentRequested` → `SpdAdjustmentBehavior.onModelAdjustmentRequested`
  (`[ADJ]adjustment/SpdAdjustmentBehavior.java:73-127`): set `enactedPolicy`, run
  `MainTransformation.qvto` (`keepModeling=true`) on allocation, system, resource environment,
  repository, SPD, semantic configuration and usage model, inside this handler. Result:
  `ModelAdjusted(true, changes)` with a resource-environment change (new and deleted containers), an
  allocation change (new allocation contexts) and one monitor change per monitor cloned from the unit
  container's monitors; on failure `ModelAdjusted(false, [])`.
- **SS-SPD-4.2** What the transformation does (step, relative, absolute size change; clamped by
  `TargetGroupSizeConstraint`; scale-out copies the unit container, connects every link to all
  containers, adds assemblies and load-balancer branches with probabilities `1/n`; scale-in removes the
  newest elements, keeps at least one): `DESIGN-SPD.md` §1.3, quirks Q4, Q6, Q7, Q8, Q16.

## SS-SPD-5 Applying the change

- **SS-SPD-5.1 Subscribers of `ModelAdjusted`**, in order (observed): `NumberOfElementsMonitorBehavior`
  (resource simulation), `ResourceSimulation.onModelAdjusted`, `NumberOfElementsMonitorBehavior`
  (system simulation), `MonitorRepositoryInterpreterBehavior.onModelAdjusted`. Resources are created
  and removed ([SS-RES-6](./resources.md#ss-res-6-removed-containers-and-the-end-of-the-run)), passive
  resources built for the new allocation contexts, the cloned monitors interpreted (new calculators,
  SS-MEAS-1.1), number-of-elements measured.
- **SS-SPD-5.2** System, connectors and SEFFs are read live, so new assemblies, connectors and
  load-balancer branches take effect for the next interpretation step. Removed allocations,
  assemblies, monitors and passive resources are not reported or removed (Q14).
- **SS-SPD-5.3 Timing.** No reconfiguration delay. At the trigger instant: the trigger event, events
  already queued for that instant, `ModelAdjustmentRequested` (the QVTo run), then the instant's
  further events, `ModelAdjusted`. Example `spd_minimal` at t = 50: `SimulationTimeReached`,
  `JobProgressed`, `ModelAdjustmentRequested`, `JobFinished`, `ActiveResourceStateUpdated`,
  `ModelAdjusted`.
- **SS-SPD-5.4 Removed containers** lose their jobs: the job with a pending event is aborted at that
  event, the others vanish (Q5; `spd_scale_in`).

## SS-SPD-6 In the projection

The exact comparison projects `SimulationTimeReached` (`spd_time`), `ModelAdjustmentRequested`
(`spd_request`) and `ModelAdjusted` (`spd_adjusted`, with the names of new and deleted containers)
into level M ([SS-X-2](./exact.md#ss-x-2-the-projection)), and compares `spd.jsonl`'s decision records
(`eval`, `adjust`) and the adjusted models (`-saveModels`, aliases for created ids) separately.

# Workloads and usage-model interpretation (SimuLizar 5.2.2)

Rules `WL-*`. Source abbreviations and the engine/process model (`SIM-*`) are defined in
[simulation core](./simulation.md). StoEx semantics: [StoEx](./stoex.md); sampling:
[random numbers](./random.md). SimOxide implements these rules as stated.

## WL-1 Workload drivers

- **WL-1.1** SimuLizar reuses SimuCom's drivers `de.uka.ipd.sdq.simucomframework.usage.OpenWorkload` /
  `ClosedWorkload` (`[SL]usagemodel/SimulatedUsageModels.java:58-135`). One driver per `UsageScenario`, created
  in usage-model order at init and run in that order at time 0 (SIM-5.1). Workload type by `eClass()` equality
  (`ClosedWorkload` / `OpenWorkload`), anything else throws.
- **WL-1.2** Per scenario run a fresh interpreter context is created
  (`[SL]interpreter/impl/SimulatedThreadComponentDelegatingScenarioRunner.java:42-46` →
  `[SL]di/modules/component/core/SimulatedThreadModule.java:26-30` →
  `[SL]interpreter/InterpreterDefaultContext.java:142-166`): stack = one new **empty** frame (the root context's
  stack is empty), empty assembly-context stack, empty result-frame stack, `thread` = the user process.

## WL-2 Open workload

`[SC]usage/OpenWorkload.java`, one generator process named `OpenWorkloadUserMaturationChamber`.

- **WL-2.1** `run()` (`:48-51`): `cancelled=false; scheduleAt(0)` → `Resume(gen)` at t=0.
- **WL-2.2** `internalLifeCycle()` (`:59-94`):
  ```
  while simulationControl.isRunning() && !cancelled:
      user := userFactory.createUser()      # new OpenWorkloadUser process (SIM-4.1), no draws
      user.startUserLife()                  # user.scheduleAt(0) → Resume(user) at now (FIFO)
      ia := evaluateStatic(interArrivalTime, Double.class)    # empty frame; draw(s) here
      hold(ia)                              # SIM-4.6: Delay(gen)@now+span(ia), then Resume(gen)
  ```
  - **The first user arrives at t = 0** (no initial inter-arrival wait). The k-th user arrives at
    `Σ_{i<k} span(ia_i)` ns (integer sum, SIM-2.2).
  - Draw order at an arrival instant: the generator draws `ia_k` **before** the new user executes anything
    (the user only runs when its `Resume` note is reached, after the generator has suspended in `hold`).
  - The generator keeps running until the run stops; after stop it exits its loop (SIM-6.5).
- **WL-2.3** `OpenWorkloadUser.internalLifeCycle()` (`[SC]usage/OpenWorkloadUser.java:40-73`):
  ```
  updateNewSessionID()                      # static counter, ids only
  simucomStartProbe.take()                  # SimuCom probe, no calculator attached in SimuLizar (MEAS-2.3)
  scenarioRunner(this)                      # interpret the UsageScenario (WL-4)
  simucomStopProbe.take()
  finally: model.increaseMainMeasurementsCount()   # SIM-6.4
  ```
  The user then terminates. No think time.

## WL-3 Closed workload

`[SC]usage/ClosedWorkload.java`, `ClosedWorkloadUser.java`.

- **WL-3.1** `run()` → `startUsers(population)` (`ClosedWorkload.java:44-46,73-79`): for i in 1..population:
  create user i (process), `startUserLife()` = `scheduleAt(0)` → `Resume(user_i)` at 0. So all users start at
  t=0 in index order, **without an initial think time**. `population` is the model's int (no StoEx).
- **WL-3.2** `ClosedWorkloadUser.internalLifeCycle()` (`ClosedWorkloadUser.java:67-109,118-126`):
  ```
  while !requestStop && simulationControl.isRunning():
      updateNewSessionID()
      try:
          simucomStartProbe.take(requestContext + "." + runCount)
          scenarioRunner.scenarioRunner(this)          # interpret the UsageScenario (WL-4)
          simucomStopProbe.take(...)
          tt := Context.evaluateStatic(thinkTime, Double.class, null)   # frame = null; draw(s)
          hold(tt)                                     # SIM-4.6
      finally:
          model.increaseMainMeasurementsCount()        # after the think time (SIM-6.4)
          runCount++
  ```
  - Think time is drawn **after** the scenario completes (the usage-scenario END measurement has already been
    taken, MEAS-3.2), then held. The next iteration starts in the same event as the end of the think time.
  - A think-time StoEx referencing variables fails (frame is `null`).
  - A think time of 0 with demands that all evaluate to 0 never advances time (zero-time livelock, REF-7 in
    [reference bugs](../correctness/reference-bugs.md)). With a time-only stop the reference loops forever;
    SimOxide stops such a run with a `livelock` error ([deviations](../correctness/deviations.md)).
- **WL-3.3** `getRequestContext()` = process request context + `"." + runCount` (`ClosedWorkloadUser.java:152-154`);
  used to pair start/stop probes (MEAS-1.7).

## WL-4 Usage scenario interpretation

`[SL]interpreter/UsageScenarioSwitch.java`. Every `firePassedEvent` notifies the interpreter listeners
synchronously (measurements, [measurements](./measurements.md)); it has no other effect.

- **WL-4.1 Scenario** (`caseUsageScenario`, `:233-248`): fire `UsageScenario BEGIN`; interpret the
  `ScenarioBehaviour`; check stack sizes; fire `UsageScenario END`; `emitInterpretationFinished` (no effect
  without failure simulation, except for execution-result monitors, MEAS-7.4).
- **WL-4.2 ScenarioBehaviour** (`caseScenarioBehaviour`, `:91-126`):
  - the **first** action in `actions_ScenarioBehaviour` list order whose `eClass()` is `Start` is the start;
    fire `BEGIN(start)`, `END(start)`; missing start → exception.
  - then follow `successor` links: for each action until one whose `eClass()` is `Stop`:
    `BEGIN(a)`; `doSwitch(a)`; `END(a)`. The `Stop` action fires no events.
- **WL-4.3 Delay** (`caseDelay`, `:145-160`): `d := StackContext.evaluateStatic(spec, Double.class)` with a
  **fresh empty frame** (variables are not visible); `hold(d)`.
- **WL-4.4 Loop** (`caseLoop`, `:206-218`): `n := evaluateStatic(spec, Integer.class)` (empty frame) **once per
  loop entry**; body interpreted `n` times (0 if n ≤ 0). A non-Integer result aborts (SIM-8.2).
- **WL-4.5 Branch** (`caseBranch`, `:132-139` → `[SL]utils/TransitionDeterminer.java:87-98,229-246`):
  exactly one `random()` draw per branch entry (even with a single transition), then
  ```
  cum[i] := ((p_0 + p_1) + ...) + p_i        # Java double adds in list order, from 0.0
  r := rng.random()
  i := first index with (cum[last] * r) < cum[i]
  ```
  (`createSummedProbabilityList` `:70-77`). Probabilities are the `double` attribute `branchProbability`
  (no StoEx) and are implicitly normalised by `cum[last]`. No match (only possible with NaN/negative
  probabilities) → index −1 → exception. The chosen transition's `ScenarioBehaviour` is interpreted via
  WL-4.2.
- **WL-4.6 EntryLevelSystemCall** (`caseEntryLevelSystemCall`, `:166-200`):
  1. create the provided-role switch (no effect);
  2. fire `SystemOperation BEGIN` (model element = the `System`, role, signature);
  3. push input frame: new frame **without parent**; for each `VariableUsage` in list order, for each
     characterisation in list order, evaluate against the user's current frame (ACT-5.2);
  4. push a new empty **result frame**;
  5. interpret the call through the system: ACT-2.1 (system provided role → provided delegation → ...);
  6. pop the input frame; pop the result frame and evaluate the call's **output** parameter usages against it,
     writing into the user's current frame (ACT-5.2);
  7. fire `SystemOperation END`.

  So a system-operation response time excludes nothing that happens inside the call and includes the
  (zero-time) parameter evaluation.
- **WL-4.7** Other `AbstractUserAction` types → `UnsupportedOperationException` (`:225-227`).

## WL-5 Summary of draws per usage-model element

| Element | Evaluation | Time of draw |
|---|---|---|
| Open workload | `interArrivalTime` (Double) | generator, right after creating each user |
| Closed workload | `thinkTime` (Double) | user, after each scenario run |
| Delay | time spec (Double) | on entering the delay |
| Loop | iteration count (Integer) | once on entering the loop |
| Branch | 1 × `random()` | on entering the branch |
| EntryLevelSystemCall | input characterisations (then output after return) | on call / on return |

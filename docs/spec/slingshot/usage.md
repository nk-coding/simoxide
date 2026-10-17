# Usage model (Slingshot)

Rules `SS-WL-*`. The usage model is interpreted by `UsageSimulationBehavior` (`[US]UsageSimulationBehavior.java`)
with a fresh `UsageScenarioInterpreter` per event (`[US]interpreters/UsageScenarioInterpreter.java`).
Each user is a `User` (a SimuCom `StackContext` with its own stack and a random UUID,
`[USD]entities/User.java`) inside an immutable `UserInterpretationContext` that names the current
action and the behaviour context (root, loop body, branch).

## SS-WL-1 Start of the usage simulation

- **SS-WL-1.1** At `SimulationStarted`, for each usage scenario in usage-model order
  (`UsageSimulationBehavior.java:140-179`), the first action that is a `Start` is interpreted:
  - **closed workload**: for `i = 1..population` (the `int` attribute, no StoEx), a new `User` and
    `doSwitch(Start)` (`:231-256`). Result per user: `UsageModelPassedElement(Start)`, `UserStarted`.
    All users start at t = 0 in index order, **without think time**.
  - **open workload**: one user, `doSwitch(Start)` (`:192-218`), which draws the first inter-arrival
    time (SS-WL-2.1). Result: `UsageModelPassedElement(Start)`, `UserStarted`, `InterArrivalUserInitiated`.

  The results of all scenarios form one set in this order.
- **SS-WL-1.2 Every interpretation step emits a passed-element event first.** `doSwitch(element)`
  returns `[UsageModelPassedElement(element), ...results of the case]` (`UsageScenarioInterpreter.java:347-363`):
  the case method runs first (draws included), the passed event is created afterwards but placed
  first. The response-time monitor reacts to the passed `Start` and `Stop` of the root behaviour
  ([SS-MEAS-2.1](./monitoring.md#ss-meas-2-which-monitors-are-honoured)).

## SS-WL-2 Open workload

- **SS-WL-2.1 The next arrival is drawn at the current arrival.** `caseStart` of an open-workload
  user in the root behaviour (`UsageScenarioInterpreter.java:195-243`):
  ```
  ia := evaluate(interArrivalTime, Double)              # empty frame; draw site "interarrival"
  next := new OpenWorkloadUserInterpretationContext(new User(), first Start of the scenario)
  result: UserStarted(this user, successor of Start), InterArrivalUserInitiated(next, delay = ia)
  ```
  So user 1 starts at t = 0 and user k+1 at `t_k + ia_k` (double sum). The draw for user k+1
  happens before user k interprets anything (probe `ss_open_draws`: tape `interarrival`, then
  `demand` of the user that just arrived).
- **SS-WL-2.2** `InterArrivalUserInitiated` → `doSwitch(Start)` of the new user
  (`UsageSimulationBehavior.java:293-309`), i.e. SS-WL-2.1 again.
- **SS-WL-2.3** The new `User` object (and its UUID) is created one arrival early, when the previous
  user's `Start` is interpreted.
- **SimuLizar:** the generator draws `ia` right after creating each user, before the user runs
  (WL-2.2): the same draw order for one scenario. Times are double sums instead of nanosecond sums.

## SS-WL-3 Closed workload

- **SS-WL-3.1 Think time is drawn when the scenario finishes.** `UserFinished` (`UsageSimulationBehavior.java:370-434`):
  pop the user's frame; result `ClosedWorkloadUserInitiated(context, think)` and
  `UsageScenarioFinished`. The think time is evaluated in the constructor of
  `ClosedWorkloadUserInitiated` (`[USD]events/ClosedWorkloadUserInitiated.java:26-27`), i.e. inside
  the `UserFinished` handler, after the usage-scenario response time was measured at the passed
  `Stop` (draw site `think`). Probe `ss_closed_think`.
- **SS-WL-3.2** `ClosedWorkloadUserInitiated` (delay = think time) → a new root behaviour context and
  `doSwitch(Start)` → `UsageModelPassedElement(Start)`, `UserStarted` (`:455-470`). The `User`
  (and its stack) is the same object across iterations.
- **SS-WL-3.3** The population is read once, at `SimulationStarted`. Later changes of the model
  (SPD competing-consumer adjustments, usage evolution) do not change it (DESIGN Q14).
- **SimuLizar:** the same draw point (WL-3.2: after the scenario, before the hold), no initial
  think time either.

## SS-WL-4 Usage actions

`UserStarted` (`UsageSimulationBehavior.java:263-283`) pushes a new empty frame on the user's stack,
interprets the current action (the successor of `Start`) and, for the root behaviour, adds
`UsageScenarioStarted` (no subscriber) after the action's results. All other progress events
interpret the context's current action: `UserWokeUp` (`:316-321`), `InnerScenarioBehaviorInitiated`
(`:472-480`), `UserInterpretationProgressed` (`:482-489`), `UserRequestFinished` (`:328-355`).

| Action | Case (`UsageScenarioInterpreter.java`) | Draws | Result after the passed element |
|---|---|---|---|
| `Start` in a nested behaviour | `:199-201` | – | `UserInterpretationProgressed(successor)` |
| `EntryLevelSystemCall` | `:119-142` | – (inputs are evaluated by the system, [SS-ACT-1](./seff.md#ss-act-1-entry-level-system-calls)) | `UserEntryRequested(request, context at successor, new result frame)` |
| `Delay` | `:330-336` | time spec, empty frame (`delay`) | `UserSlept(successor)` (delay 0), `UserWokeUp(successor, delay d)` |
| `Loop` | `:85-112` | iteration count, empty frame (`loop`) | `InnerScenarioBehaviorInitiated(first action of the body)` |
| `Branch` | `:255-285` | one `Math.random()` (`branch:<id>`) | `InnerScenarioBehaviorInitiated(Start of the chosen behaviour)` |
| `Stop`, loop body to repeat | `:151-161` | – | `UsageModelPassedElement(Start of the body)`, `UserInterpretationProgressed(successor of Start)` |
| `Stop`, other nested behaviour | `:162-174` | – | `UserInterpretationProgressed(parent behaviour, successor of the Loop or Branch)` |
| `Stop` of the root behaviour | `:177` | – | `UserFinished` |
| any other action | `:296-301` | – | none: the user stops silently |

- **SS-WL-4.1 Delay.** The delay is `evaluateStatic(spec, Double)` with no frame (variables are not
  visible). The user continues at `t + d` with `UserWokeUp`. Probe `ss_usage_delay`.
- **SS-WL-4.2 Branch.** `TransitionDeterminer.determineBranchTransition` (`[CU]TransitionDeterminer.java:76-82,156-172`):
  cumulative sums of `branchProbability` in list order, one `Math.random()` `r`, the first `i` with
  `sum_last * r < sum_i`; the same algorithm as WL-4.5, on the other stream
  ([SS-RNG-1](./random.md#ss-rng-1-two-streams)). An empty transition list returns −1 without a draw,
  then `get(-1)` throws. Probe `ss_branches`.
- **SS-WL-4.3 Loop.** The count is evaluated once when the loop is entered. The body runs `n` times:
  `startScenario()` increments the progression and `Stop` repeats while `progression < n`
  (`[USD]entities/scenariobehavior/LoopScenarioBehaviorContext.java:38-56`). Each repetition emits
  the body's passed `Start` again.
  - **A count ≤ 0 throws** in the builder (`initialLoopCount < maximalLoopCount`, `:39-41`), inside
    the handler that interprets the loop; that handler is disposed (SS-BUS-7). Quirk
    [SS-Q19](./quirks.md#ss-q19). Probe `ss_usage_loop_zero`: after the first user, `UserStarted`
    has no handler left and every later user stops after its passed `Start`. **SimuLizar:** a count
    ≤ 0 skips the body (WL-4.4).
- **SS-WL-4.4 Response of an entry call.** `UserRequestFinished` (`UsageSimulationBehavior.java:328-355`)
  first checks that the user's stack has exactly two frames (`Preconditions.checkArgument`, `:331-334`),
  pops the input frame, evaluates the call's output parameter usages from the result frame into the
  user's frame (draw site `return`) and interprets the successor. A SEFF that leaves an extra frame on
  the stack makes the check throw and disposes this handler (corpus `h10_collection_iterator`,
  [SS-ACT-6.3](./seff.md#ss-act-6-loops-and-collection-iterators)).

## SS-WL-5 Aborted users

- **SS-WL-5.1** `UserAborted` (`UsageSimulationBehavior.java:395-449`) comes from a resource demand
  that could not be served ([SS-RES-1](./resources.md#ss-res-1-which-resources-exist)), a removed
  resource (SPD scale-in, Q5) or a failed call ([SS-ACT-7](./seff.md#ss-act-7-calls-over-the-wire)).
  - **closed workload**: the user is replaced by a **new `User`** with an empty stack;
    `ClosedWorkloadUserInitiated(new user, think time drawn now)`. The interrupted request records no
    response time.
  - **open workload**: nothing; the user is dropped. Arrivals continue, they do not depend on it.
- **SS-WL-5.2** The stack frames, passive resources held and jobs still queued for the aborted user
  are not cleaned up.
- Probe `ss_delay_resource`: every request aborts at its first demand; the closed user restarts
  every think time as a new user (`u` numbers 1, 2, 3, … in the projection).
- **SimuLizar:** no aborts in the supported scope (failures are off).

## SS-WL-6 Not supported or not loaded

- **SS-WL-6.1** Usage evolution (DLIM) rewrites the open-workload inter-arrival time every
  `evolutionStepWidth` from `PreSimulationConfigurationStarted` on
  (`behavior.usageevolution/.../UsageEvolutionBehavior.java:66-108`). It is not loaded by the
  reference runner (decision U9) and not specified further here.
- **SS-WL-6.2** Usage-model actions other than those of SS-WL-4 end the user's interpretation
  without an event or an error.

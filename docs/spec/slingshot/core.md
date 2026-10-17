# Event core (Slingshot)

Rules `SS-ENG-*` (engine and time), `SS-BUS-*` (event bus), `SS-RUN-*` (start and stop) and
`SS-ND-*` (nondeterminism). Source tags: [overview](./index.md#version-and-sources).

Slingshot has no processes. Every step of the model is an **event** on a global **bus**; a handler
reacts to one event and returns the events that follow, and these are put on the SSJ event list at
`now + delay`. A request through a small model takes 20 to 40 events
([SS-BUS-8](#ss-bus-the-event-bus)), against one or two notes in SimuLizar (SIM-4).

## SS-ENG Engine and time

- **SS-ENG-1 Engine.** One SSJ 3.3.1 `Simulator` per run, with its default event list, a
  `SplayTree` (`[CORE]engine/SimulationEngineSSJ.java:21-27`; `[SSJ]Simulator()`). Each Slingshot
  event is wrapped in an `SSJEvent` (`:98-125`).
- **SS-ENG-2 Time is a `double` number of seconds.** The clock starts at 0.0 (`[SSJ]Simulator.init`).
  `scheduleEvent(e)` (`SimulationEngineSSJ.java:39-52`):
  ```
  if e.time() > 0: insert at e.time() + e.delay()            # scheduleEventAt, :55-63
  else:            insert at sim.time() + e.delay()          # [SSJ]Event.schedule: eventTime = time() + delay
  ```
  A new event has `time() == 0` (`[COM]events/AbstractSimulationEvent.java:35-46`), so in practice
  every event is scheduled at **`now + delay`, one IEEE double addition** at the moment it is
  scheduled, where `now` is the time of the event being dispatched. (Dispatch sets the event's time
  to `now`, `SimulationEngineSSJ.java:116`; an event object returned again would land at the same
  instant.)
  - A negative delay is rejected when the event is constructed:
    `Preconditions.checkArgument(delay >= 0)` (`AbstractSimulationEvent.java:26`). The exception is
    thrown inside the handler that creates the event (SS-BUS-7).
  - There is no quantisation. Times are sums of doubles: in `ss_role_lookup` the second request
    starts at `0.30000000000000004`.
  - **SimuLizar:** integer nanoseconds with `(long)(d * 1e9)` truncation (SIM-2.2). The exact mode
    must use `f64` time with Slingshot's additions; the fast mode may keep nanoseconds (a documented
    deviation of at most 1 ns per delay).
- **SS-ENG-3 Same-time order is FIFO.** The event list orders by `(time, priority)`; every Slingshot
  event keeps SSJ's default priority 1.0 (`[SSJ]Event(Simulator)`). Among equal keys, `SplayTree.add`
  places the new event **after** all equal ones (it marks it with `ra = 1`, which `myCompareTo` ranks
  last, `[SSJ]SplayTree.add`, `myCompareTo`). This is the same rule as SIM-3.1.
  Probe: `ss_fifo_ties`.
- **SS-ENG-4 Main loop** (`[SSJ]Simulator.start`, `SimulationEngineSSJ.java:107-119`):
  ```
  while (ev := removeFirst()) != null and not stopped:
      now := ev.time
      if stopped: skip                       # SSJEvent.actions
      event.setTime(now); bus.post(event)    # synchronous, SS-BUS
      consumedEvents++
  ```
  The whole handler cascade of one event (SS-BUS-3) runs inside `post` before the next event is taken.
- **SS-ENG-5 No cancellation.** Slingshot never removes a scheduled event. An obsolete event is
  dispatched like any other and its handler recognises it (PS completions,
  [SS-RES-2.6](./resources.md#ss-res-2-processor-sharing)). **SimuLizar:** schedulers cancel and
  re-insert (SIM-3.3).

## SS-BUS The event bus

- **SS-BUS-1 One synchronous bus.** `Bus.instance()` is one `BusImplementation` per JVM, built on
  an RxJava 3 `PublishSubject` (`[ED]internal/BusImplementation.java:38-67`). `post(event)` calls
  `onNext` synchronously (`:146-152`): every subscriber has run when `post` returns.
- **SS-BUS-2 Subscribers and their order.**
  - Every `@Subscribe` method of a registered behaviour becomes one `Subscriber`, subscribed as
    `bus.ofType(parameterType)` (`BusImplementation.java:83-98,154-186`). It receives every event
    that is an instance of its parameter type, subclasses included (a `JobProgressed` subscriber also
    gets `ProcessorSharingJobProgressed`).
  - A `@Subscribe(reified = X.class)` method additionally requires the generic type token of the
    event to be a subtype of `X` (`[ED]entity/Subscriber.java:78,174-186`). This is how
    `UsageModelPassedElement<Start>` and `<Stop>` reach different monitor methods.
  - Subscribers are called in **subscription order**. Registration order is:
    1. the behaviour extensions in extension-registry order, each class's methods in
       `getDeclaredMethods()` order (`[CORE]driver/SlingshotSimulationDriver.java:62-72`,
       `BusImplementation.java:116`); a behaviour whose `isActive()` is false is not registered
       (the monitor interpreter without monitors, the SPD behaviour without an SPD model);
    2. `CoreBehavior`, last of the extensions (`SlingshotSimulationDriver.java:74`);
    3. the SPD policy subscribers, registered while `PreSimulationConfigurationStarted` is handled
       (`[SPD]interpreter/SpdBehavior.java:51-64`), so after `CoreBehavior`.
  - The registry order is not specified but was the same in every run. **`getDeclaredMethods()`
    order varies between JVMs**: in one of the probe runs the two reified methods of
    `UsageScenarioResponseTimeMonitoringBehavior` (`<Start>`, `<Stop>`) were registered the other way
    round. Within every Slingshot class, methods that subscribe to the same event type are reified to
    disjoint types, so this never changes which handler runs or in which order handlers of one event
    run. The runner records the observed order per event type in `bus.jsonl` (in the probes'
    `expected/`, not compared by `probes.sh --check`). The disjointness is checked statically
    (X1, `reference/slingshot/tools/audit_subscribers.py`, on the pinned sources): of the 73
    `@Subscribe` methods in 25 classes, no two of one class can receive the same event; the only
    classes with two subscribers whose parameter types overlap are
    `UsageScenarioResponseTimeMonitoringBehavior` (`<Start>`/`<Stop>`),
    `OperationCallActionResponseTimeMonitoringBehavior` (`<StartAction>`/`<StopAction>`) and
    `MeasurementsAggregatorBehavior` (`<FixedSizeAggregation>`/`<VariableSizeAggregation>`), all
    reified to disjoint types. (The bus itself refuses two methods with the same parameter type and
    reified types in one class, `BusImplementation.java:172-175`.) Where more than one Slingshot subscriber
    handles an event, the order is:

    | Event | Subscribers, in order |
    |---|---|
    | `MeasurementSpecificationVisited` | `ActiveResourceMonitorBehavior`, `PassiveResourceMonitoringBehavior`, `OperationCallActionResponseTimeMonitoringBehavior`, `UsageScenarioResponseTimeMonitoringBehavior`, processing types (`MeasurementsAggregatorBehavior` ×2, `FeedThroughRecorderBehavior`) |
    | `MonitorModelVisited` | `MeasurementsAggregatorBehavior` (fixed size, variable size), `FeedThroughRecorderBehavior` |
    | `UsageModelPassedElement` | `UsageScenarioResponseTimeMonitoringBehavior.onUsageScenarioStarted<Start>`, `.onUsageScenarioFinished<Stop>` |
    | `SEFFModelPassedElement` | `OperationCallActionResponseTimeMonitoringBehavior.onOperationCallStarted<StartAction>`, `.onOperationCallStopped<StopAction>` |
    | `ResourceDemandRequested` | `PassiveResourceMonitoringBehavior.onResourceDemandRequest`, then `ResourceSimulation.onResourceDemandRequested` |
    | `SimulationFinished` | `ResourceSimulation.onSimulationFinished`, then `CoreBehavior.onSimulationFinished` |
    | `ModelAdjusted` | see [SS-SPD-5](./spd.md#ss-spd-5-applying-the-change) |
    | `MeasurementMade` | the SPD policy subscribers in policy order (none without SPD) |

    Every other event type has exactly one Slingshot subscriber or none (`ProbeTaken`,
    `UsageScenarioStarted`, `UsageScenarioFinished`, `UserSlept`, `MeasurementUpdated`,
    `CalculatorRegistered` in practice: none that changes the run).
- **SS-BUS-3 One subscriber call** (`Subscriber.java:71-91`):
  ```
  pre := composite pre-interceptor(info, event)      # every pre-interceptor registered for a supertype
  if not reified-match or not pre.successful: return  #   of the event, CompositeInterceptor.java:40-51
  result := handler(event)                            # void → Result.empty()
  composite post-interceptor(info, event, result)     # :27-38
  ```
  Interceptors run **once per subscriber**, not once per event. Slingshot registers no
  pre-interceptor and one relevant post-interceptor, `CoreBehavior.rescheduleNextEvents` for
  `DESEvent` (`[CORE]behavior/CoreBehavior.java:36-52`). (`EventContractChecker` also
  post-intercepts and never changes anything.) The interceptor map is a `HashMap<Class, List>`
  (`CompositeInterceptor.java:14-15`); its order matters only when several interceptors schedule.
- **SS-BUS-4 Results are scheduled right after their handler.** `rescheduleNextEvents` calls
  `driver.scheduleEvent(e)` for each event of the handler's `Result` in the set's iteration order
  (`CoreBehavior.java:40-50`), i.e. SS-ENG-2 for each. So the results of subscriber *k* are queued
  before subscriber *k+1* runs. `Result` copies the handler's collection into a `HashSet` and drops
  nulls (`[ED]returntypes/Result.java:25-33`): in stock Slingshot the order of one handler's results is
  their identity-hash order; in `det` it is insertion order ([SS-ND-2](#ss-nd-nondeterminism-and-the-det-patch)).
  The result order of each handler is given in the area pages.
- **SS-BUS-5 Directly scheduled events.** Two kinds of events are scheduled *during* a handler,
  not through its result:
  - `MeasurementMade`: every calculator measurement, through `EventBasedMeasurementObserver`
    (`[MON]observer/EventBasedMeasurementObserver.java:30-36`);
  - `MeasurementUpdated`: the output of processing types (FeedThrough, aggregators), through
    `ProcessingTypeMeasurementSourceListener` (`[MON]calculator/ProcessingTypeMeasurementSourceListener.java:72-80`).

  They are queued at the moment of the measurement, before the result events of the handler that
  took it ([SS-MEAS-1](./monitoring.md#ss-meas-1-the-measurement-path)).
- **SS-BUS-6 The same-time cascade.** From SS-ENG-3 and SS-BUS-4/5: all events at one instant are
  dispatched in the order they were queued; zero-delay results go to the end of the queue of that
  instant. The order is breadth-first by generation, not depth-first. Example (`ss_fifo_ties`, t = 3,
  three PS jobs with equal remaining demand): the completion of job 1 queues `JobFinished(1)`,
  `JobProgressed(2, delay 0)`, `ActiveResourceStateUpdated`; `JobFinished(1)` queues
  `ActiveResourceFinished`; `JobProgressed(2)` queues `JobFinished(2)`, …; only then is the state
  sample of job 1's completion recorded, so the projection reads `job_end 1`, `job_end 2`,
  `meas state 2`, … (`expected/projection.jsonl`).
- **SS-BUS-7 A throwing handler is disposed.** An exception in a handler is rethrown by
  `AnnotatedSubscriber.acceptEvent` (`[ED]entity/AnnotatedSubscriber.java:19-43`). RxJava's
  `LambdaObserver` then calls its error consumer, `BusImplementation.doError` (logs "An error
  occurred", calls the `@OnException` handlers, `:188-194`), and **disposes the subscription**.
  - That one handler method never runs again in this run; other subscribers and the simulation go
    on. The thrown invocation's results are lost; `MeasurementMade` events it already scheduled
    (SS-BUS-5) stay.
  - Typically one exception freezes a whole stage for every user: after
    `SeffSimulationBehavior.onSeffInterpretationProgressed` is gone, no SEFF advances; the run still
    reaches `simTime` and reports no error. Quirk [SS-Q18](./quirks.md#ss-q18).
  - Probes: `ss_loop_zero`, `ss_usage_loop_zero`, `ss_guard_none`, `ss_two_demands`; corpus `h10`,
    `h11`, `h12`, `h13`, `h18`, `h26`, `h27`, `h28` (`expected/errors.txt`,
    `results/det-verify.csv` column `handler_errors`). SimOxide's exact mode keeps a disposed bit
    per subscriber method and reproduces each of them ([Quirks](./quirks.md#exact-mode)).
  - A pre- or post-interceptor that throws returns `InterceptionResult.error` instead
    (`[ED]entity/interceptors/PreInterceptor.java:48-60`, `PostInterceptor.java:45-52`); a failed
    pre-interception skips the handler for that event only.
- **SS-BUS-8 The handler table.** What each event does, in one place; the semantics are on the
  area pages. *Result* lists the produced events in `det` order; *direct* marks SS-BUS-5 events;
  *draws* lists the random draws of the handler ([SS-RNG](./random.md)).

  | Event (delay) | Handler | Result (det order) |
  |---|---|---|
  | `PreSimulationConfigurationStarted` (0) | monitor interpreter; SPD set-up | `MonitorModelVisited`, `MeasurementSpecificationVisited` per monitor ([SS-MEAS-2](./monitoring.md#ss-meas-2-which-monitors-are-honoured)); `SimulationTimeReached` per time trigger ([SS-SPD-2](./spd.md#ss-spd-2-triggers)) |
  | `SimulationStarted` (0) | `UsageSimulationBehavior.onSimulationStart` | per scenario: closed: per user `UsageModelPassedElement(Start)`, `UserStarted`; open: `UsageModelPassedElement(Start)`, `UserStarted`, `InterArrivalUserInitiated` ([SS-WL-1](./usage.md#ss-wl-1-start-of-the-usage-simulation)) |
  | usage events | `UsageSimulationBehavior` | [SS-WL](./usage.md) |
  | `UserEntryRequested`, `SEFF*`, `CallOverWire*`, `ActiveResourceFinished`, `PassiveResourceAcquired`, `ResourceDemandRequestAborted` | `SystemSimulationBehavior`, `SeffSimulationBehavior` | [SS-ACT](./seff.md) |
  | `ResourceDemandRequested`, `Job*`, `PassiveResourceReleased`, `CallOverWireRequested` | `ResourceSimulation` | [SS-RES](./resources.md) |
  | `ActiveResourceStateUpdated`, `ResourceDemandCalculated`, passed elements, passive events | monitors | `ProbeTaken`; direct `MeasurementMade`, `MeasurementUpdated` ([SS-MEAS](./monitoring.md)) |
  | `MeasurementMade`, `SimulationTimeReached`, `ModelAdjustmentRequested`, `ModelAdjusted` | SPD | [SS-SPD](./spd.md) |
  | `SimulationFinished` (at `simTime`) | resources, core | none (SS-RUN-3) |
  | `ProbeTaken`, `MeasurementUpdated`, `UsageScenarioStarted`, `UsageScenarioFinished`, `UserSlept`, `CalculatorRegistered`, `ProcessingTypeRevealed` | (bookkeeping) | none |

## SS-RUN Start and stop

- **SS-RUN-1 Set-up.** `SlingshotSimulationDriver.init` builds a child Guice injector and
  instantiates every behaviour (`SlingshotSimulationDriver.java:52-77`). Constructors already do
  work: `ResourceSimulation` builds the active, passive and linking resource tables and evaluates
  passive-resource capacities ([SS-RES-4.1](./resources.md#ss-res-4-passive-resources)), the usage
  and system behaviours load their models. An exception here aborts the run before t = 0 (corpus
  `x_pem_acquire`: a capacity that reads a component parameter).
- **SS-RUN-2 Start** (`SlingshotSimulationDriver.java:79-91`):
  ```
  engine.init()                                   # time 0.0, empty list
  schedule PreSimulationConfigurationStarted      # delay 0
  schedule SimulationStarted                      # delay 0
  scheduleAt SimulationFinished, simTime          # absolute
  simulator.start()                               # SS-ENG-4, returns when stopped
  ```
  So at t = 0 the monitors are interpreted, and the SPD model, before the first user starts; their
  results (`MonitorModelVisited`, …) are queued before those of `SimulationStarted`, which therefore
  run after all calculators exist.
  - `simTime` is `SimuComConfig.getSimuTime()`, a `long`: whole seconds (the runner passes
    `(long) simTime`, `[RUN]runner/src/simoxide/slingshot/runner/Runner.java`).
- **SS-RUN-3 `SimulationFinished` comes first at `simTime`.** It was queued at t = 0, so by SS-ENG-3
  it is dispatched before every other event at exactly `simTime`; those never run. Its subscribers
  (SS-BUS-2):
  1. `ResourceSimulation.onSimulationFinished` clears the job queues of all active, passive and
     linking resources (`[RS]ResourceSimulation.java:433-438`);
  2. `CoreBehavior.onSimulationFinished` stops the driver, the engine and the simulator
     (`CoreBehavior.java:31-34`, `SimulationEngineSSJ.java:77-81`). Later results are dropped
     (`SlingshotSimulationDriver.java:109-124`) and the SSJ loop ends.

  Probe `ss_stop_time`: the arrival due at exactly t = 10 is not dispatched.
  **SimuLizar:** the first event at or after the stop time still runs, then post-stop processing
  (SIM-6.3, SIM-6.5).
- **SS-RUN-4 No drain and no final samples.** Requests in progress at `simTime` are dropped without
  measurements; no resource records a final state tuple (probe `ss_stop_time`: the last state sample
  is the one of the last completion). **SimuLizar:** drain (SIM-6.5) and final state tuples at
  `finalise` (MEAS-10.1).
- **SS-RUN-5 `simTime` is the only stop condition.** The maximum measurement count of the run
  configuration is never read. There is no livelock guard: a zero-time loop runs forever. Corpus
  `x_pem_fork`: one closed user with think time 0 whose every request aborts on a delay resource
  ([SS-RES-1](./resources.md#ss-res-1-which-resources-exist)) restarts at t = 0 without end (the
  verification stops it with a time limit). Quirk [SS-Q32](./quirks.md#ss-q32).

## SS-ND Nondeterminism and the det patch

- **SS-ND-1 Sources of nondeterminism in stock Slingshot.**
  1. The iteration order of each handler's `Result` (`HashSet` of objects with identity hashes,
     SS-BUS-4), and of the sets handlers build: `HashSet` in `UsageScenarioInterpreter.doSwitch`,
     `Set.of` (per-JVM salt) for nearly every multi-event result, `Collectors.toSet()`.
  2. The PS job table, a `Hashtable` keyed by `Job.hashCode = hash(UUID)`
     (`[RS]resources/active/ProcessorSharingResource.java:40,160-164`, `[RSD]entities/jobs/Job.java:60-61`):
     the choice among jobs with equal remaining demand.
  3. `Math.random()` (branches, link failures) and the StoEx generator, both unseeded
     ([SS-RNG-1](./random.md#ss-rng-1-two-streams)).
  4. `UUID.randomUUID()` for every event, user and job id.
  5. In SPD adjustments, QVTo/OCL `Set`s of model objects (identity hashes; Q8, Q16).
  6. Without effect on runs: `CompositeInterceptor`'s `HashMap<Class>` (one scheduling interceptor);
     `getDeclaredMethods()` order, which does vary between JVMs (SS-BUS-2) but only reorders
     subscribers of disjoint reified types.
- **SS-ND-2 What `det` changes** (reference runner, [Slingshot runner](../../reference-simulator/slingshot.md#what-det-changes)):
  - the ordering rule: every `HashSet`, `HashMap`, `Collectors.toSet()`, `Set.of`, `Set.copyOf` in
    Slingshot, QVTo and OCL classes iterates in insertion order;
  - patch S0-P2: the PS job table becomes a `LinkedHashMap` (ties go to the earliest job);
  - seeded streams: StoEx MT19937 `{N..N+5}`, `Math.random()` = `java.util.Random(N + 1000003)`,
    UUIDs from SplitMix64 seeded with N.
  - instrumentation hooks that change nothing: the trace, tape and SPD log, and the draw-site hooks
    of the seed agent ([SS-X-4](./exact.md#ss-x-4-tape)).

  Each `det` order is one of the orders stock Slingshot can produce: insertion order is one
  permutation of each set. **The Slingshot-exact mode implements the `det` order**: wherever these
  pages say "result order", they mean insertion order.
- **SS-ND-3 What is verified, what may remain.** `det` is byte-reproducible (trace, tape, SPD log,
  measurements) in fresh and warm JVMs on the 20 probes, the SPD examples and the corpus models
  ([SS-X-6](./exact.md#ss-x-6-det-verification)). Orders that could still depend on identity
  hashes are in classes outside the rewritten packages; none was observed to change a run:
  - SimuCom's `SimulatedStackframe` is a `HashMap<String, Object>`: deterministic (string hashes), the
    same order SimuLizar uses (ACT-5.6, ACT-5.7);
  - `OperationCallActionResponseTimeMonitoringBehavior` builds request contexts from
    `caller.hashCode()` (identity, `[SYM]OperationCallActionResponseTimeMonitoringBehavior.java:282-285`):
    only used for matching start and stop, so only a hash collision could matter;
  - EMF `EList`s keep model order.

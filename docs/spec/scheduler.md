# Resource models (schedulers) of SimuLizar 5.2.2

Implemented in `crates/simoxide-sched`. Checked against the golden traces of
`reference/oracles/sched` (the real 5.2.2 classes running on the real DESMO-J engine).

## Sources

Tag `releases/5.2.2` of each repo. For these files the Java sources are identical on master.
Decompiling the product jars (`de.uka.ipd.sdq.scheduler_5.2.2.jar`,
`de.uka.ipd.sdq.simucomframework.core_5.2.2.jar`) gives the same code.

| Abbreviation | Path |
|---|---|
| `PS` | Palladio-Simulation-Scheduler `bundles/de.uka.ipd.sdq.scheduler/src/de/uka/ipd/sdq/scheduler/resources/active/SimProcessorSharingResource.java` |
| `FCFS` | same directory, `SimFCFSResource.java` |
| `DELAY` | same directory, `SimDelayResource.java` |
| `AAR` | same directory, `AbstractActiveResource.java` |
| `ASR` | Palladio-Analyzer-SimuCom `bundles/de.uka.ipd.sdq.simucomframework.core/src/de/uka/ipd/sdq/simucomframework/core/resources/AbstractScheduledResource.java` |
| `SR` | same directory, `ScheduledResource.java` |
| `SLR` | same directory, `SimulatedLinkingResource.java` |
| `SSFPR` | same directory, `SimSimpleFairPassiveResource.java` |
| `CH` | same directory, `CalculatorHelper.java` |
| `MT` | `MathTools` in `de.uka.ipd.sdq.probfunction.math_5.2.2.jar` (decompiled) |
| `DJ` | Palladio-Simulation-AbstractSimEngine `bundles/de.uka.ipd.sdq.simulation.abstractsimengine.desmoj/...`, and DESMO-J 2.3.3 `TimeSpan`, `TimeInstant`, `EventTreeList` (decompiled) |

## Which classes simulate what

SimuLizar's `ResourceEnvironmentSyncer.createSimulatedActiveResource` builds a `ScheduledResource`
(SimuCom framework) for each `ProcessingResourceSpecification`. The strategy id is
`getSchedulingPolicy().getId()`. The ids in `Palladio.resourcetype` are `ProcessorSharing`,
`FCFS` and `Delay`. `ASR` constructor (l. 65-71) maps `ProcessorSharing` to `PROCESSOR_SHARING`
and `Delay` to `DELAY`, and keeps any other id as it is. `SR.getScheduledResource` (l. 86-117)
then selects the scheduler:

| Policy id | Scheduler | Cores |
|---|---|---|
| `ProcessorSharing` | `SimProcessorSharingResource` | `numberOfReplicas` |
| `FCFS` | `SimFCFSResource` | always 1 (`numberOfReplicas` is ignored, [REF-1](../correctness/reference-bugs.md)) |
| `Delay` | `SimDelayResource` | unbounded |
| other | a scheduler extension, e.g. the exact schedulers | out of scope |

Linking resources use `SimulatedLinkingResource`, which is always `SimFCFSResource` (`SLR`
l. 54, 82).

Passive resources use `SimSimpleFairPassiveResource`: one per (passive resource, assembly
context). SimuLizar's `SimulatedBasicComponentInstance` evaluates the capacity once, when the
component instance is created.

## Time: integer nanoseconds (DESMO-J)

SimuLizar 5.2.2 runs on DESMO-J (`DesmoJExperiment`). The epsilon is `NANOSECONDS` and the
reference unit is `SECONDS`.

- **Instants:** every instant is a `long` number of nanoseconds.
- **Delays:** a `double` delay `d` passed to `schedule(entity, d)` becomes
  `new TimeSpan(d) = (long)(d * 1e9)`. This **truncates** toward zero, so completions can come up
  to 1 ns earlier than the exact value.
- **Current time:** model code sees the current time as `getTimeAsDouble() = (double) nanos / 1e9`.
- **Invalid delays:** negative delays and delays of `Long.MAX_VALUE` abort the simulation.
- **Same-time events:** `EventTreeList.insert` puts a new note after every note with the same
  time, so same-time events run in FIFO order.

Rust: `simoxide_sched::time::{span, seconds, SimTime = i64}`. The event core of `simoxide-sim`
uses an `i64` nanosecond clock and runs events in `(time, insertion seq)` order, FIFO among
equal times (see [Simulation core](./simulation.md)).

## Demand conversion (before the scheduler)

`ASR.consumeResource` (l. 138-173):

1. `concrete = calculateDemand(demand)`.
   - Processing resources: `demand / processingRate` (`SR` l. 123). The rate StoEx is evaluated
     on every call.
   - Linking resources: `demand / throughput` (`SLR` l. 90). The throughput is evaluated on every
     call and must be > 0, otherwise `ThroughputZeroOrNegativeException`.
2. Demand-modifying behaviours: linking resources have one latency behaviour
   `("1.0", latency)`, so `concrete = concrete / 1.0; additive += latency; concrete += additive`.
3. `if (concrete <= 0) return;`. There is no measurement and no scheduling; the thread
   continues. This covers zero demands.
4. `fireDemand(concrete)` (the resource-demand measurement), then
   `AAR.process(thread, id, emptyMap, concrete)`.
5. `AAR.process` returns immediately if the simulation is no longer running (l. 32).

Rust: `simoxide_sched::demand::{processing_demand, linking_demand}`. StoEx evaluation and step 5 are
the caller's job.

## Processor sharing (`PS`)

State:

- `running_processes`: process to remaining demand, in seconds of service at full speed.
- `last_time`: a `double`.
- `numberProcessesOnCore[capacity]`.
- One `ProcessingFinishedEvent`.

The time scaling factor is `f = max(1.0, (double) n / (double) capacity)`, where `n` is the number
of jobs (l. 99-102).

`toNow()` (l. 86-97):

```java
now = currentTime (double); passed = now - last_time; processed = passed / f;
if (MathTools.less(0, passed))           // passed > 0 && |passed| >= 1e-5
    for each job: rem = rem - processed;
last_time = now;                         // always
```

**Lost time:** if less than 1e-5 s passed since the last update, nobody is served for that
interval (`MT.less`, `EPSILON_ERROR = 1e-5`). This quirk is kept.

`doProcessing(p, demand)` (l. 113-126):

1. `toNow()`.
2. `demand = max(demand, JIFFY = 1e-9)`.
3. `put(p, demand)`.
4. `reportCoreUsage()`.
5. `scheduleNextEvent()`.
6. `p.passivate()`.

`scheduleNextEvent()` (l. 66-84):

1. `shortest` is the first job in iteration order with the minimal remaining demand. The test is
   the strict `get(shortest) > get(p)`, so for equal values the earlier job wins.
2. `removeEvent()`.
3. `t = rem(shortest) * f`, and `t < JIFFY ? 0.0 : t`.
4. `schedule(shortest, t)`.

`ProcessingFinishedEvent.eventRoutine(p)` (l. 41-52):

1. `toNow()`.
2. `remove(p)`. The job is removed whatever its remaining demand is, which can be a few ulps off
   zero.
3. `reportCoreUsage()`.
4. `fireDemandCompleted(p)`.
5. `scheduleNextEvent()`.
6. `p.activate()`.

Jobs with equal remaining demand therefore finish as separate events at the same time: the delay
of the next one is below JIFFY and is set to 0.

`reportCoreUsage()` (l. 133-164):

- If `n < capacity`: core `i` is set to 1 if `i < n`, otherwise 0.
- Otherwise: every core gets `n / capacity` jobs, and the first `n % capacity` cores get one more.
- `fireStateChange(count, core)` fires only for cores whose count changed, in ascending core order.

**Hash-order ties.** `running_processes` is a `java.util.Hashtable` keyed by processes that have
no `hashCode` override. The iteration order, and with it the choice among exactly equal remaining
demands, therefore depends on identity hashes and is nondeterministic (ND-2 in
[Simulation core](./simulation.md)).

The oracle patch `reference/oracles/sched/patched/.../SimProcessorSharingResource.java` replaces
it with a `LinkedHashMap`, which keeps insertion order. `put` of a new key appends; `setValue`
does not reorder. `contains` becomes `containsValue`, which keeps the same meaning. The
deterministic reference simulator applies the identical patch (P2 in
[Patches](../reference-simulator/patches.md)). simoxide-sched implements this insertion order:
a `Vec` in insertion order, with order-preserving removal.

On the 29 oracle scripts without exact ties, the unpatched product class produces the same trace
as the patched one. It differs only on the 10 scripts with exact ties.

Other notes:

- `getRemainingDemand` calls `Hashtable.contains(process)`, which checks the *values*. It
  therefore always returns 0.0 (the same bug is in `FCFS`). It is only used by extensions, so it
  is not ported.
- `stop()` does nothing.

## FCFS (`FCFS`)

State:

- `processQ` (an `ArrayDeque`).
- `running_processes`: process to remaining demand. Only used for lookups; the iteration order
  does not matter.
- `last_time`.

`toNow()` (l. 59-79): if `MT.less(0, passed)`, the head's remaining demand becomes
`rem - passed`, and is snapped to `0.0` if `|rem - passed| < 1e-5`. Shorter intervals are lost,
as in PS. `last_time = now`.

`doProcessing` (l. 89-97):

1. `toNow`.
2. Append the job with its demand. There is no JIFFY clamp.
3. `fireStateChange(queue size, 0)`. This fires on every call, even if the value is unchanged.
4. `scheduleNextEvent`: `removeEvent()`, then `schedule(head, rem(head))`. The head is
   rescheduled on every arrival from its updated remaining demand, so the truncation applies anew.
5. `passivate`.

`eventRoutine` (l. 24-36):

1. `toNow`.
2. Remove the head.
3. `fireStateChange(size, 0)`.
4. `fireDemandCompleted`.
5. `scheduleNextEvent`.
6. `activate`.

`stop()` clears the queue but does not cancel the pending event. In simoxide-sched, `stop()` also
makes the pending wake-up stale. This only matters after the simulation has stopped (a
documented [deviation](../correctness/deviations.md)).

## Delay (`DELAY`)

`running_processes` is keyed by `process.getId()`.

`process()` fires `fireStateChange(size, 0)` exactly once per call, through `enqueue`. That call
comes from `AAR.process` if the last resource differs, otherwise from `doProcessing`. Then a new
`DelayEvent` is scheduled at `now + span(demand)`.

The event runs `dequeue(p)`:

1. If `p` is unknown (after `start()`/`stop()` cleared the map), do nothing.
2. Otherwise remove it.
3. `fireStateChange(size, 0)`.
4. `fireDemandCompleted`.
5. `activate`.

## Passive resources (`SSFPR`)

`acquire(p, num)` (l. 108-133):

1. If the simulation is not running, return true.
2. `fireRequest`.
3. If `canProceed` (the queue is empty or `p` is its head, and `num <= available`), grant:
   `available -= num`, then `fireAquire`. Return true.
4. Otherwise enqueue a `SimpleWaitingProcess`, `passivate`, and return false.

`release(p, num)` (l. 174-199):

1. If the simulation is not running, do nothing.
2. `available += num`. The value is not clamped.
3. `fireRelease`.
4. While the head of the queue fits: grant it (`fireAquire`), dequeue it, `activate` it.
   A head that does not fit blocks everyone behind it (strict FIFO).

Timeouts need failure simulation (out of scope).

The state measurement reads `getAvailable()` after each acquire or release (`CH` l. 313-352).
Waiting time runs from request to acquire (`CH` l. 77-111).

Rust: `PassiveResource`, `PassiveListener::{requested, acquired, released, wake}`.

## Event ordering contract (for `simoxide-sim`)

- `activate()` of a SimuLizar thread is `scheduleAt(0)`: a new event at `now + 0`, appended
  after all events already scheduled for `now`.
- In PS and FCFS completion routines, the resource's next completion is scheduled **before** the
  completed job's resumption. With ties this means that at time t the order is:
  `finish(A)`, `finish(B)`, `resume(A)`, ...
- In Rust, `on_wakeup` returns `Completion { job, next }`. The core inserts `next` first, then
  schedules the resumption of `job`.
- `process()` returns the new wake-up. The core inserts it immediately; it has no resumption,
  because the job is passivated.
- A previous wake-up of the same resource stays in the core's heap and is ignored as stale by
  generation, with no side effects. This matches `removeEvent()`, because a stale entry never
  runs and the FIFO rank of the live ones is unaffected.

## Measurements emitted by resources

These calculators exist only for monitors in the monitor repository
(`ResourceEnvironmentSyncer.attachMonitors`, `SimulatedBasicComponentInstance`).

- **`fireStateChange(state, core)`.** It goes through `ASR.update`, `IStateListener`, then
  `TakeScheduledResourceStateProbe`, which records `(now, getQueueLength(core))`.
  - The value is the PS per-core count, or the size for FCFS and Delay.
  - After each such update, `SR.update` also calls `fireOverallUtilization`. This triggers the
    overall-utilisation probe, if one is set up (replica 0 and `numberOfReplicas > 1`).
  - That probe records the fraction of `numberOfReplicas` cores whose queue length is > 0
    (`TakeScheduledResourceUtilization`). This fraction is taken between the per-core updates
    of one `reportCoreUsage`.
  - Rust: `ResourceListener::state_changed`, `ActiveResource::busy_fraction`.
- **Initial measurement.** The state calculator takes one measurement when it is set up.
- **Resource demand.** `fireDemand(concrete)` is emitted before `process`, and only for
  demands > 0.
- **At simulation end.** `SimuComModel.finalise` calls `notifyStopListeners`, then
  `deactivateAllActiveResources`.
  - `ASR.deactivateResource` fires `fireStateEvent(0, i)` for all instances. The probe reads the
    *current* queue length, not 0.
  - `SR` then fires the overall utilisation and `stop()`.
  - These late tuples are recorded: the probes are not gated by `isRunning()`. They arrive after
    the final window flush and are never aggregated (MEAS-10.1 in
    [Measurements](./measurements.md)). `simoxide-sim` emits them at the stop time.

## Rust API (crate `simoxide-sched`)

- **Active resources:** `ProcessorSharing<J>`, `VirtualTimeProcessorSharing<J>`, `Fcfs<J>` and
  `Delay`, plus the `ActiveResource<J>` enum built with
  `ActiveResource::new(SchedulingPolicy::from_pcm_id(id)?, replicas, PsAlgorithm)`.
  - `process(now, job, demand, &mut listener) -> Wakeup<J>`.
  - `on_wakeup(now, &wakeup, &mut listener) -> Option<Completion<J>>`.
  - `queue_length(core)`, `stop()`.
- **Passive resources:** `PassiveResource<J>` with `acquire(job, n, &mut l) -> bool` and
  `release(job, n, &mut l)`.
- **Design:** no allocation per job in steady state, and no global state.

## Exactness results

- **Golden traces.** `reference/oracles/sched` holds 39 scripts:
  - hand-written edge cases: ties, 1-4 cores, arrivals closer than 1e-5 s, zero, negative,
    sub-JIFFY and huge demands, closed loops with zero think time;
  - random open, closed, tie-heavy and stress workloads;
  - PS, FCFS, Delay and passive resources.

  The traces contain every state change, completion, activation, and the **bit pattern of every
  remaining demand** after each scheduler call. `cargo test -p simoxide-sched` reproduces all of them
  **bit-exactly** with the exact PS.
- **Virtual-time PS** (`PsAlgorithm::VirtualTime`, O(log n) per event, CLI
  `--ps-algorithm virtual-time`) is opt-in and not exact; it is a documented
  [deviation](../correctness/deviations.md) and not part of the
  [fast mode](../guide/fast-mode.md). It adds up the per-job service in one virtual clock
  instead of subtracting from each job. A job that demands again while it is still served keeps
  its position and gets the new demand, as in the exact algorithm.
  - On the PS oracle scripts, 125 of 4406 completion times differ; the largest difference is 10 ns.
  - A rounding difference can flip a 1e-5 s lost-time decision, so the bound in the property
    tests is 10 µs.
  - Speed: about 5x faster than exact at around 300 concurrent jobs (`perf_heavy_contention`).
- **Invariants (proptest).** The property tests check:
  - every job completes exactly once, with service time ≥ demand;
  - per-core counts are balanced;
  - FCFS keeps arrival order;
  - PS is fair;
  - PS completion times for a batch arriving together match the closed form;
  - delay adds exactly `span(demand)`;
  - passive resources conserve units and grant in FIFO order;
  - single-core PS and FCFS have the same makespan, up to the lost-time rule.
- **Analytical checks** (`--ignored`, release, 2e6 jobs): M/M/1-PS, M/M/1-FCFS, M/M/c-PS
  (c = 2, 4, against Erlang C) and M/M/∞ (delay). All fall within the batch-means confidence
  intervals.

**Lost time breaks work conservation.** PS and FCFS are not exactly work conserving in the
reference: an interval shorter than 1e-5 s between two resource events is not served.

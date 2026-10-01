# Reference bugs

SimuLizar 5.2.2 has bugs and anomalies that change results or abort runs. SimOxide reproduces all
of them, REF-14 excepted, so that its output stays identical to the reference. They are listed
here so that nobody "fixes" them. A fix belongs behind an option and on the
[Deviations](./deviations.md) page.

| | Anomaly | SimOxide |
|---|---|---|
| REF-1 | FCFS ignores `numberOfReplicas` | reproduced |
| REF-2 | double resumes abort | reproduced (same error, same time) |
| REF-3 | `UniDouble(a, a)` aborts | reproduced |
| REF-4 | `Pois(m)` is Poisson − 1 | reproduced |
| REF-5 | TimeDrivenAggregation plus FeedThrough on one measuring point aborts | reproduced |
| REF-6 | assembly-operation monitor on an infrastructure signature aborts | reproduced |
| REF-7 | zero-time livelock | stopped by the livelock guard |
| REF-8 | nested resource containers are not simulated | reproduced |
| REF-9 | recursion through a monitored call aborts | reproduced |
| REF-10 | middleware marshalling needs `stream.BYTESIZE` | reproduced |
| REF-11 | a triggering Reconfiguration Time monitor aborts after the stop | reproduced |
| REF-12 | empty reconfigurations succeed | reproduced |
| REF-13 | aggregating the passive-resource state fails | reproduced (error at the end of the event) |
| REF-14 | deep recursion deadlocks | not reproduced: runs, bounded by a limit |

## REF-1 FCFS ignores `numberOfReplicas`

`SimFCFSResource` serves only the head of one queue; `capacity` is unused. An FCFS CPU with 3
replicas is one server (checked statistically: `statistical.rs::fcfs_ignores_replicas`, M/M/1
response time within 0.3 %). Only replica 0 records state.

## REF-2 Double resumes abort

A synchronous fork whose children finish without waiting resumes its parent twice
([SIM-4.4a](../spec/simulation.md)). The second resume wakes the parent early from its next wait.

- If the parent then waits at a resource or delay, the run aborts with
  `IllegalStateException: Tried to schedule thread which was not suspended [ClosedUser_n]` (from
  `SimDelayResource.dequeue` → `activate`, or from a resource completion); also `[OpenUser_n]`.
  SimOxide reports `IllegalStateException: activate() of a running process`.
- If the parent demands the same FCFS resource again, it is queued twice. Java's
  `running_processes` `Hashtable` keeps one entry per process, so the second `put` overwrites the
  remaining demand of the first job; the run then aborts with a `NullPointerException` in
  `SimFCFSResource.scheduleNextEvent`. This happens on processing resources and on linking
  resources, which are FCFS too. SimOxide reports `NullPointerException: FCFS process queued
  twice` at the same event, after the same trace.

The model generator's `double_resume` feature provokes these on purpose.

## REF-3 `UniDouble(a, a)` aborts

`invalid bracketing parameters` from Commons Math's inverse CDF (degenerate interval). SimOxide
reports the same error.

## REF-4 `Pois(m)` is Poisson − 1 {#ref-4-pois-m-is-poisson-1}

Commons Math 2.1's integer inversion is off by one ([Random numbers](../spec/random.md)): loop
counts and demands can be −1. `UniInt` has a Palladio correction and is correct.

## REF-5 TimeDrivenAggregation plus FeedThrough on one measuring point aborts

`Calculator … already in calculator registry`. Only one processing type per metric and
measuring point is possible.

## REF-6 Assembly-operation monitor on an infrastructure signature aborts

`ClassCastException: … InfrastructureSignatureImpl must be of type OperationSignature`. The
model generator puts these monitors on operation signatures only.

## REF-7 Zero-time livelock

A closed workload with think time 0 whose demands all evaluate to 0 never advances time. With a
time-only stop the reference loops at t = 0 forever. This is a model property rather than a bug.
SimOxide stops such a run with a `livelock` error ([livelock guard](./deviations.md#livelock-guard),
test `api.rs::zero_time_livelock_is_stopped`). With a measurement stop both terminate, because
each think time of 0 ends a counted iteration.

## REF-8 Nested resource containers are not simulated

Only top-level containers get simulated resources. A component allocated to a nested container
aborts at its first resource demand (`NullPointerException`, `getSimulatedEntity(String) is
null`). Unused nested containers are ignored, including their monitors (`corpus/h31_nested_container`,
`corpus-fuzz/l_ref_nested_allocation`, [ACT-3.3](../spec/actions.md)).

## REF-9 Recursion through a monitored call aborts

A process that re-enters a monitored external call or assembly operation aborts with
`IllegalStateException: First measurement to the same context arrived while previous series of
the same context did not complete` ([MEAS-1.5](../spec/measurements.md);
`corpus-fuzz/l_ref_recursion_*`). Without such monitors recursion runs (`corpus/h32_recursion`).

## REF-10 Middleware marshalling needs `stream.BYTESIZE`

With `simulate_linking_resources = true`, SimuLizar expects the middleware completion (which it
does not execute) to put `stream.BYTESIZE` on every call's request and result frame. Without it
the first assembly-connector call aborts with "Stackframe is missing id stream.BYTESIZE", even
between components on one container ([ACT-11.3](../spec/actions.md),
`corpus-fuzz/l_ref_stream_bytesize_missing`; with the stream: `corpus/h30_middleware_stream`).

## REF-11 A triggering Reconfiguration Time monitor aborts after the stop

When a runtime-measurement write happens after the stop (final window flush, final state tuples,
drain), the reconfiguration process runs synchronously. If the `Reconfiguration Time` spec itself
has `triggersSelfAdaptations = true` (the EMF default), its tuple writes the measurement before
`lastReconfigurationTime` is updated, and the process schedules itself while running:
`IllegalStateException: Tried to schedule thread which was not suspended [Reconfiguration
Process_n]` ([MEAS-7.2](../spec/measurements.md),
`corpus-fuzz/l_ref_reconf_rescheduled_after_stop`). The model generator keeps that spec at
`false`.

## REF-12 Empty reconfigurations succeed

Without reconfiguration rules the QVTo engine reports success for every run of the
reconfiguration process, so a `Reconfiguration Time` monitor records `(t, 0.0)` at every run
([MEAS-7.3](../spec/measurements.md)).

## REF-13 Aggregating the passive-resource state fails

A `FixedSize` or `VariableSizeAggregation` with `triggersSelfAdaptations` on `State of Passive
Resource` aborts at its first aggregation (`ClassCastException: Long cannot be cast to Double`).
SimOxide reports the same error at the end of the event; the reference aborts inside it.

## REF-14 Deep recursion deadlocks the simulation

A deterministic call depth of 300 through two assembly contexts (`corpus/h32_recursion` with a
guarded `n.VALUE > 0` recursion and `n.VALUE - 1` passed on) never finishes. The main thread
parks in `AbstractSimProcessSemaphoreStrategy.resumeProcess`, and no simulated-process thread is
alive: the process thread died, most likely of a `StackOverflowError` on its 1 MB stack, without
the error being reported. Depth 100 runs.

SimOxide runs depth 100 000 and stops unbounded recursion with `Limits::max_stack_depth` (default
10 000 continuations, about 2 000 calls). It deliberately does not reproduce the hang
([Deviations](./deviations.md#deep-recursion)).

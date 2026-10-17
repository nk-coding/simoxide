# Resources (Slingshot)

Rules `SS-RES-*`. Resources are simulated by `ResourceSimulation` (`[RS]ResourceSimulation.java`)
with the classes under `[RS]resources/`. Compare the SimuLizar schedulers on
[Schedulers](../scheduler.md): Slingshot's processor sharing is a near-literal port of SimuCom's,
its FCFS is not.

## SS-RES-0 From request to job

- **SS-RES-0.1 Active demand** (`ResourceSimulation.java:115-173`). `ResourceDemandRequested(ACTIVE)`:
  ```
  d := evaluate(demand spec, Double, user's top frame)        # draw site "demand"
  ac := first allocation context of the assembly (SS-ACT-9)
  none: result JobAborted(job)                                # → SS-WL-5
  else: result JobInitiated(ActiveJob(d, new UUID, resource type, request, ac))
  ```
- **SS-RES-0.2 Job start** (`:191-231`, `[RS]resources/active/AbstractActiveResource.java:113-129`).
  `JobInitiated`: the resource is looked up by **(container id, resource type id)**
  (`ActiveResourceCompoundKey.java:33-34`); missing → `JobAborted("active resource is not found, may
  have been deleted")`. Otherwise:
  ```
  c := d / evaluate(processingRate)        # every job, empty frame; draw site "rate"
  job.demand := c
  next := process(JobInitiated)            # SS-RES-2.2 / SS-RES-3.2
  result: ActiveResourceStateUpdated(state after), ResourceDemandCalculated(c), next (if any)
  ```
  The resource demand measurement is `c`, also when it is 0 (probe `ss_zero_demand`).
  **SimuLizar:** a demand ≤ 0 is skipped without measurement (ACT-3.4); HDDs also divide by the
  read rate (ACT-3.4). Slingshot ignores HDD read and write rates: an HDD is an ordinary resource.
- **SS-RES-0.3 Every completion event samples the state.** `JobProgressed` → the resource's
  `process(JobProgressed)` and then, **always**, `ActiveResourceStateUpdated(state after)`, appended
  last (`AbstractActiveResource.java:132-141`). This includes the stale events of SS-RES-2.6.
- **SS-RES-0.4** `JobFinished` → `ActiveResourceFinished(request)` (`ResourceSimulation.java:297-311`)
  → the SEFF continues ([SS-ACT-3.2](./seff.md#ss-act-3-internalaction)). `JobAborted` →
  `ResourceDemandRequestAborted` (`:313-327`) → `UserAborted` (`[SY]SystemSimulationBehavior.java:284-290`).

## SS-RES-1 Which resources exist

- **SS-RES-1.1** At construction, for the container of every allocation context (allocation list
  order) and every processing resource specification of it, one resource keyed by (container,
  resource type); a later specification with the same type replaces an earlier one
  (`[RS]resources/active/ActiveResourceTable.java:29-78`):
  - scheduling policy `FCFS` → `FCFSResource` (SS-RES-3);
  - `ProcessorSharing` → `ProcessorSharingResource` with `numberOfReplicas` cores (SS-RES-2);
  - **any other policy, `Delay` included → no resource** (the delay case is commented out, `:49-52`).
- **SS-RES-1.2** A demand on a resource type without a resource aborts the job and the user
  ([SS-WL-5](./usage.md#ss-wl-5-aborted-users)). Quirk [SS-Q25](./quirks.md#ss-q25). Probe
  `ss_delay_resource`; corpus `h04_delay_resource`, `h25_deterministic_closed` (every request aborts
  at its delay demand), `x_pem_fork` (zero think time: livelock at t = 0, SS-RUN-5).
- **SS-RES-1.3** Containers are not looked up by nesting: every container that an allocation context
  references gets resources, nested or not. **SimuLizar:** delay resources exist; nested containers
  are not simulated (REF-8).

## SS-RES-2 Processor sharing

`[RS]resources/active/ProcessorSharingResource.java`. State: `runningJobs` (job → remaining demand
in seconds of full-speed service; a `Hashtable`, in `det` a `LinkedHashMap`, patch S0-P2), `internalTime`
(double), `currentState` (a UUID), `numberProcessesOnCore[cores]`. With `n` jobs and `cores` cores,
`f = max(1.0, (double) n / cores)` (`:211-214`).

- **SS-RES-2.1 Time update** (`:183-197`):
  ```
  passed := t - internalTime
  if MathTools.less(0, passed):          # passed > 0 and |passed| >= 1e-5 (MathTools.EPSILON_ERROR)
      for each job (map order): rem := rem - passed / f;  job.demand := rem
  internalTime := t                      # always: shorter intervals are lost
  ```
  `f` is evaluated with the job count before the change. Identical to SimuCom's `toNow`, including
  the lost time.
- **SS-RES-2.2 Arrival** (`:78-91`): time update; a demand `< JIFFY = 1e-9` becomes `JIFFY`; add the
  job (at the end of the map); `reportCoreUsage`; schedule the next completion (SS-RES-2.3).
  A zero demand is therefore a job of 1e-9 s with its own events and samples (probe `ss_zero_demand`:
  completion at `1.0E-9`). **SimuLizar:** zero demands never reach the scheduler (ACT-3.4).
- **SS-RES-2.3 Next completion** (`:151-174`): with no job, nothing. Otherwise a fresh
  `currentState := UUID.randomUUID()`; `shortest` = the first job in map order with the minimal
  remaining demand (strict `>` test, so ties go to the earliest job in `det`); `delay := rem(shortest)
  * f`, `0.0` if `< JIFFY`; result `ProcessorSharingJobProgressed(shortest, delay, currentState)`.
  The completion time is the double `now + delay` (no nanosecond truncation).
- **SS-RES-2.4 Completion** (`:103-125`): a `JobProgressed` whose expected state is not `currentState`
  is **stale**: nothing happens. Otherwise: time update, remove the event's job (whatever its
  remaining demand), `reportCoreUsage`, next completion; result `JobFinished(job)`, then the next
  `ProcessorSharingJobProgressed`, then (SS-RES-0.3) `ActiveResourceStateUpdated`. Jobs with equal
  remaining demand finish as separate events at the same instant (delay below JIFFY → 0).
  Probe `ss_fifo_ties`.
- **SS-RES-2.5 Cores** (`reportCoreUsage`, `:227-260`): as SimuCom: with `n < cores`, cores `0..n-1`
  have one job; otherwise every core `n / cores`, the first `n % cores` one more. The per-core counts
  feed only the utilisation (SS-RES-2.7).
- **SS-RES-2.6 Stale completions are dispatched and sample the state.** Slingshot never cancels the
  previous completion event (SS-ENG-5); every arrival and completion makes the pending one stale. Each
  stale event still yields an `ActiveResourceStateUpdated` (SS-RES-0.3): an extra state and
  utilisation tuple with unchanged values at the time the old completion was due. Quirk
  [SS-Q26](./quirks.md#ss-q26) (Slingshot semantics in the exact mode; the fixed behaviour drops
  them). Probes `ss_fifo_ties` (samples at t = 1 and 2), `ss_ps_multicore`.
- **SS-RES-2.7 State values** (`publishState`, `:277-285`): `requestsAtResource` = number of jobs at
  the resource (all cores together); `utilization` = cores with at least one job / cores.
- **SimuLizar** (the SimuCom scheduler, [Schedulers](../scheduler.md#processor-sharing-ps)): the same
  update, JIFFY, shortest-job and delay arithmetic; differences: the pending completion is cancelled
  (no stale samples), integer-nanosecond times, per-core state listeners.

## SS-RES-3 FCFS

`[RS]resources/active/FCFSResource.java`:

- **SS-RES-3.1 One server.** One `ArrayDeque`; `numberOfReplicas` is ignored (as REF-1). Probe
  `ss_fcfs_replicas`.
- **SS-RES-3.2 Arrival** (`:57-68`): append; only if the queue was empty, `JobProgressed(job, delay =
  job.demand)`.
- **SS-RES-3.3 Completion** (`:77-92`): remove the event's job; if a job is waiting,
  `JobProgressed(head, delay = head.demand)`; result `JobFinished(job)`, the next `JobProgressed`, and
  (SS-RES-0.3) `ActiveResourceStateUpdated`.
- **SS-RES-3.4 Each job is served for exactly its demand from the moment it reaches the head.** There
  is no remaining-demand bookkeeping (the timer method is unused, `:116-130`), no lost time and no
  re-scheduling on arrivals. A zero demand completes at the same instant.
- **SS-RES-3.5 State values** (`:148-153`): queue length including the job in service; utilisation 0
  or 1.
- **SimuLizar:** `toNow` with 1e-5 lost time and the head rescheduled from its remaining demand on
  every arrival, nanosecond truncation each time ([Schedulers](../scheduler.md#fcfs-fcfs)). With
  integral times the completion times coincide.

## SS-RES-4 Passive resources

`[RS]resources/passive/PassiveResourceTable.java`, `SimplePassiveResource.java`:

- **SS-RES-4.1 Instances.** One per (passive resource, assembly context) for every allocated assembly
  context of a basic component (`PassiveResourceTable.java:36-65`). The capacity is evaluated once, at
  construction, as `Long`, **without a frame**: component parameters are not visible, and a capacity
  that uses one aborts the run before t = 0 (corpus `x_pem_acquire`: "Stackframe is missing id
  lockCount.VALUE"). Quirk [SS-Q27](./quirks.md#ss-q27). **SimuLizar:** evaluated with the component
  instance's frame (ACT-8.1).
- **SS-RES-4.2 Acquire** (`ResourceSimulation.java:129-144`, `SimplePassiveResource.java:62-70,105-124`):
  `amount := evaluate(demand spec, Long, user's top frame)` (draw site `passive`). Granted when the
  queue is empty (or the job is its head) and `amount <= available`: `available -= amount`, result
  `PassiveResourceAcquired`, `PassiveResourceStateUpdated(available)`. Otherwise the job is queued and
  nothing is produced (no state event).
- **SS-RES-4.3 Release** (`ResourceSimulation.java:233-252`, `SimplePassiveResource.java:82-95`): the
  amount is **evaluated again** (draw site `passive`); `available += amount` (no clamp); then, strictly
  FIFO, the head is granted while it fits. Result: `PassiveResourceAcquired` per grant, in queue order,
  then `PassiveResourceStateUpdated(available)`. The releasing SEFF has already continued
  ([SS-ACT-8.3](./seff.md#ss-act-8-acquire-and-release)).
- **SS-RES-4.4** No instance (an assembly that is not allocated, a composite's inner assembly):
  acquire queues nothing and produces nothing, so the request waits forever; release logs an error.
- Probe `ss_passive`. **SimuLizar:** the same FIFO discipline, but always one unit, whatever the
  action's demand (ACT-8.2, ACT-8.3).

## SS-RES-5 Linking resources

`[RS]resources/linking/SimulatedLinkingResource.java`, `LinkingResourceTable.java`:

- **SS-RES-5.1** One `SimulatedLinkingResource` per linking resource of the environment, keyed by id
  (`LinkingResourceTable.java:19-26`). It is an FCFS resource with one server (`SimulatedLinkingResource.java:26-37`).
- **SS-RES-5.2 Transfer time.** For a `LinkingJob` with payload `b` (Σ BYTESIZE,
  [SS-ACT-7.1](./seff.md#ss-act-7-calls-over-the-wire)):
  `b / evaluate(throughput)` (draw site `rate`), then `+ evaluate(latency)` (draw site `latency`)
  (`AbstractActiveResource.java:117`, `SimulatedLinkingResource.java:42-49`), served FCFS
  (SS-RES-3). The latency occupies the link, in each direction. **SimuLizar:** the same demand and
  FCFS service (ACT-11.4).
- **SS-RES-5.3 Failure draw on every completion.** Each `JobProgressed` of a link first draws
  `Math.random()` (draw site `link:<id>`), **also when the failure probability is 0**
  (`SimulatedLinkingResource.java:52-66`). If `r < failureProbability` the job is removed and
  `JobAborted` produced ([SS-ACT-7.3](./seff.md#ss-act-7-calls-over-the-wire)); the link then
  schedules no completion for the next queued job, which waits forever. Quirk
  [SS-Q28](./quirks.md#ss-q28). **SimuLizar:** no failure draw without failure simulation.
- **SS-RES-5.4** Links have no state or demand measurements (the monitors handle only `ActiveJob`s).
- Probe `ss_link` (tape: one `link:` draw per transfer).

## SS-RES-6 Removed containers and the end of the run

- **SS-RES-6.1** On `ModelAdjusted`, resources of new containers are created and those of deleted
  containers are removed from the table; jobs are not moved. A later `JobInitiated` or `JobProgressed`
  for a removed resource becomes `JobAborted`; the other jobs on it are lost
  (`ResourceSimulation.java:329-350`; [SS-SPD-5](./spd.md#ss-spd-5-applying-the-change), DESIGN Q5;
  model `spd_scale_in`).
- **SS-RES-6.2** `SimulationFinished` clears every queue (`ResourceSimulation.java:433-438`);
  no final samples ([SS-RUN-3](./core.md#ss-run-start-and-stop)).

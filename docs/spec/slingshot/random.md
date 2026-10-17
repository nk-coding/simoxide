# Random numbers (Slingshot)

Rules `SS-RNG-*`. Sampling itself (distributions, PMF and PDF literals, `Pois`, `UniInt`, …) is the
SimuCom StoEx evaluator, the same as in SimuLizar ([StoEx](../stoex.md),
[Random numbers](../random.md)); this page covers which streams exist, where they are drawn and in
which order.

## SS-RNG-1 Two streams

- **SS-RNG-1.1 StoEx stream.** Every StoEx evaluation draws its uniforms from the generator of
  `ProbabilityFunctionFactoryImpl` (JVM-global). Slingshot never seeds it: by default it is a
  `java.util.Random()` (`DefaultRandomGenerator`), and `SimuComConfig`'s seed is not applied
  (`behavior.generalsimulationconfiguration/.../GeneralSimulationConfigurationBehavior.java`).
  The reference runner, with `-seed N` and `-stoex mt` (default), installs SimuCom's MT19937 seeded
  `{N, N+1, …, N+5}`, the stream refsim and SimOxide use for seed N (`[RUN]runner/src/simoxide/slingshot/runner/Runner.java`,
  `Seeding.apply`). With `-stoex jur` it is `java.util.Random(N)`.
- **SS-RNG-1.2 `Math.random()` stream.** Probabilistic branches (usage model and SEFF,
  `[CU]TransitionDeterminer.java:156-172`, "TODO: Use SimuCom Random instead") and the failure check
  of every linking-resource completion (`[RS]resources/linking/SimulatedLinkingResource.java:52-66`)
  call `Math.random()`. The runner's seed agent replaces the JVM's generator behind it by a
  `java.util.Random` subclass reseeded with `N + 1000003` at the start of every run (same LCG, same
  `nextDouble`). The exact mode implements `java.util.Random`'s LCG from its API specification (U12):
  `seed := (s ^ 0x5DEECE66D) & (2^48 − 1)`; `next(b)`: `seed := (seed * 0x5DEECE66D + 0xB) & (2^48 − 1)`,
  return `seed >>> (48 − b)`; `nextDouble() = ((next(26) << 27) + next(27)) * 2^-53`.
- **SS-RNG-1.3 UUIDs.** Every event, user, job and PS state gets `UUID.randomUUID()`. UUIDs never
  influence values or (in `det`) orders; the runner seeds them (SplitMix64) only for reproducible
  stock runs. The exact mode does not need them.
- **SimuLizar:** one MT stream for everything, branches included (RND-5). With the same seed the StoEx
  stream of Slingshot and SimuLizar is the **same sequence of uniforms**; whether a model's values
  coincide depends only on the draw order (SS-RNG-2).

## SS-RNG-2 Draw sites and order

Draws happen inside handlers, so their order is the dispatch order of the events
([SS-BUS-6](./core.md#ss-bus-the-event-bus)) and, within one handler, program order. The tape site
names (`o` of [SS-X-4](./exact.md#ss-x-4-tape)) are:

| Site | When | Source |
|---|---|---|
| `interarrival` | interpreting the `Start` of an open-workload user: the next user's inter-arrival time | `[US]interpreters/UsageScenarioInterpreter.java:206-207` |
| `think` | handling `UserFinished` (or `UserAborted`) of a closed-workload user | `[USD]events/ClosedWorkloadUserInitiated.java:26-27` |
| `delay`, `loop` | interpreting a usage `Delay` / `Loop` | `UsageScenarioInterpreter.java:87-88,332-333` |
| `branch:<id>` | usage `Branch`, SEFF probabilistic `BranchAction` (stream `math`) | `[CU]TransitionDeterminer.java:162` |
| `guard` | SEFF guarded branch conditions, in order until the first true | `TransitionDeterminer.java:44-46,90-115` |
| `param` | entry-call inputs (`UserEntryRequested`), external-call inputs (`SEFFExternalActionCalled`, again on a retry) | `[SY]SystemSimulationBehavior.java:115,184-185` |
| `component` | component and assembly parameter usages, at each SEFF entry | `[SY]interpreters/RepositoryInterpreter.java:142-148` |
| `return` | call outputs: `UserRequestFinished`, the reply's `CallOverWireSucceeded` | `[US]UsageSimulationBehavior.java:352`, `SystemSimulationBehavior.java:236-237` |
| `setvar`, `loop`, `collection` | SEFF `SetVariableAction`, `LoopAction`, `CollectionIteratorAction` | `[SY]interpreters/SeffInterpreter.java:153-154,270-288,306-307` |
| `infra` | all `numberOfCalls` of an internal action's infrastructure calls, when they start | `InfrastructureCallsContextHolder.java:32-50` |
| `demand` | handling `ResourceDemandRequested(ACTIVE)` | `[RS]ResourceSimulation.java:151-153` |
| `rate` | handling `JobInitiated` (processing rate, link throughput) | `[RS]resources/active/AbstractActiveResource.java:117` |
| `latency` | handling a link's `JobInitiated`, after `rate` | `[RS]resources/linking/SimulatedLinkingResource.java:47` |
| `link:<id>` | every `JobProgressed` of a link (stream `math`) | `SimulatedLinkingResource.java:53` |
| `passive` | passive acquire and release amounts | `ResourceSimulation.java:182-184` |
| `capacity` | passive capacities, at construction (before t = 0) | `[RS]resources/passive/PassiveResourceTable.java:59-60` |

- **SS-RNG-2.1 Differences in draw order from SimuLizar** (the uniforms are the same sequence):
  - an internal action's demands are all drawn at its start, one `ResourceDemandRequested` each,
    before any is served (SimuLizar: each when it is issued, after the previous completes);
  - the processing rate is drawn in the `JobInitiated` handler, an event later than the demand;
  - infrastructure-call counts are all drawn after the action's demands completed;
  - collection INNER values once per action, not per iteration;
  - branches draw from the other stream;
  - same-time events of different users are interleaved breadth-first (SS-BUS-6), so their draws
    interleave differently than SimuLizar's process steps.
  - Equal: open-workload inter-arrival before the arriving user's first draw (probe
    `ss_open_draws`), closed think time after the scenario (`ss_closed_think`), loop counts once per
    entry.
- **SS-RNG-2.2** Model `x_ss_minimal` (one closed user, one demand per request) draws in the same order
  as SimuLizar: its response times equal refsim's up to the nanosecond truncation (S0 finding).

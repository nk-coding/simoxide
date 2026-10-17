# Slingshot: findings from a headless, deterministic study

Date: 2026-10-17. For the Slingshot maintainers. Nothing here has been published.

## Summary

**Context.** SimOxide is a Rust port of SimuLizar 5.2.2. It reproduces SimuLizar's event trace,
random draws and measurements byte for byte against a patched reference. It is getting a
Slingshot-compatible mode. For that mode we wrote down Slingshot's semantics as rules with source
references, checked each rule with small probe models, and ran a model corpus through Slingshot.
This report lists the bugs and robustness problems we found on the way.

**What was examined.**

- Slingshot nightly `1.0.0.202609010045`, built from Palladio-Analyzer-Slingshot commit `8bf9adc`
  (2026-08-21).
- SPD `1.0.0.202609010039`, from Palladio-Addons-SPD-Metamodel commit `95cdb0b` (2026-08-18).
- Palladio 6.0.0 nightly (SimuCom framework `6.0.0.202609010041`, probfunction `6.0.0.202609010005`),
  QVTo 3.11.0, SSJ 3.3.1, Eclipse 2025-12 platform, OpenJDK 21.0.12.
- The install was made from the update sites on 2026-09-29. 47 jars are pinned by SHA-256 in
  [`install.lock`](./install.lock). Key jars:

  | Jar (`org.palladiosimulator.` omitted) | SHA-256 (first 16 hex digits) |
  |---|---|
  | `analyzer.slingshot.core_1.0.0.202609010045` | `44e6253e85057af6` |
  | `analyzer.slingshot.eventdriver_1.0.0.202609010045` | `cb838a8c893e1181` |
  | `analyzer.slingshot.behavior.systemsimulation_1.0.0.202609010045` | `2c4eb5b591b4923d` |
  | `analyzer.slingshot.behavior.resourcesimulation_1.0.0.202609010045` | `9ef42e9769c17d85` |
  | `analyzer.slingshot.behavior.spd_1.0.0.202609010045` | `78530e04f9e05dcd` |
  | `analyzer.slingshot.behavior.spd.adjustment_1.0.0.202609010045` | `34253de1fd5fedcf` |
  | `spd_1.0.0.202609010039` | `89106c7975bf2f8b` |

  Two required bundles are on no update site (I1). We built them from source at the commits above:
  `analyzer.slingshot.common.utils` and `spd.semantic.transformations`.

**How.**

- A headless Equinox application runs the install unchanged ("stock" mode). Simulation time is
  the only stop condition, as in Slingshot.
- An optional deterministic mode adds three things:
  - a Java agent seeds `Math.random()`, the StoEx generator (SimuCom's MT19937) and
    `UUID.randomUUID()`;
  - the same agent makes the hash-ordered collections of Slingshot, QVTo and OCL iterate in
    insertion order;
  - one source patch replaces the `Hashtable` job table of `ProcessorSharingResource` by a
    `LinkedHashMap`.
- A passive behaviour extension writes the bus trace, every random draw with its call site, and the
  SPD filter decisions. It changes no measurement (checked on every model).
- Every finding was traced to the source at the commits above. File references are given as
  `File.java:lines` with the bundle name (`org.palladiosimulator.analyzer.slingshot.` omitted).

**Models.** 88 in total: 20 probe models made for this study, 2 SPD examples (the
Palladio-Documentation-Slingshot minimal example and a scale-in variant), and 66 corpus models
(33 hand-made models with one feature each, 33 example models from Palladio-Example-Models, the
SimuLizar examples and tests, and the Slingshot E2E tests).

**Headline numbers.**

- 22 of the 88 models throw an exception in an event handler. The handler is disposed and the run
  ends normally, without an error (R1). 13 of the 88 record no measurement at all.
- Of the 66 corpus models, 38 run without handler errors, aborted users or empty output.
- In the deterministic mode, two fresh JVMs give byte-identical traces, draws and measurements on
  all 85 models that run. In stock mode the event order differs between runs with the same inputs;
  on 2 models the measurements differ between runs too, one of them without any random draw (D2).
- The SPD interpreter cannot run an adjustment in an install made from the update sites (I2).
  Scale-in loses the jobs of the removed container (S1). Composed triggers give wrong results (S2).

**Severity** is our assessment. *High*: silently wrong or missing results on common models, or a
feature that cannot be used. *Medium*: wrong results or a crash in specific configurations.
*Low*: small effect, or a robustness gap without known wrong results.

**Reproductions.** Every reproduction is a folder in [`reproductions/`](./reproductions/) with the
model files, a README (how to run it with stock Slingshot, what to look for, what we expected) and,
where the run terminates, the outputs of our deterministic run in `observed/`. Appendix A explains
how to run them.

## Issue index

| ID | Title | Severity | Reproduction |
|---|---|---|---|
| **Crash and robustness** | | | |
| R1 | A handler that throws is disposed; the run continues and reports success | High | `ss_loop_zero`, `ss_two_demands`, `h13_passive_contention`, … |
| R2 | No stop condition besides simulation time; zero-time loops never end | Medium | `ss_delay_livelock` |
| R3 | A call between unconnected containers disables all network transfers | Medium | `ss_link_unconnected` |
| R4 | Unsupported actions, ResourceCalls and missing connectors end requests silently | Medium | `h23_resource_call` |
| R5 | Passive resource capacity cannot use component parameters | Medium | `x_pem_acquire` |
| **Wrong results** | | | |
| W1 | Several resource demands of one InternalAction run concurrently and each advances the SEFF | High | `ss_two_demands` |
| W2 | Delay resources are not simulated | High | `ss_delay_resource` |
| W3 | ForkAction: asynchronous behaviours ignored; two or more synchronous behaviours fail | High | `ss_async_fork`, `h11_fork_sync`, `h12_fork_async` |
| W4 | CollectionIteratorAction: INNER evaluated once; frame never popped | High | `h10_collection_iterator` |
| W5 | A loop with iteration count 0 runs once and throws | Medium | `ss_loop_zero`, `ss_usage_loop_zero` |
| W6 | Acquire/Release without ParametricResourceDemand throw | High | `h13_passive_contention` |
| W7 | Composite components and subsystems are not simulated | High | `h15_composite` |
| W8 | Provided and required roles are resolved by role id only | Medium | `ss_role_lookup` |
| W9 | A guarded branch without a true guard disposes the SEFF handler | Low | `ss_guard_none` |
| W10 | Infrastructure-call input parameters are ignored | Low | `h18_infrastructure_call` |
| W11 | Outdated processor-sharing completion events record state samples | Low | `ss_fifo_ties`, `ss_ps_multicore` |
| W12 | Link failures: a draw per transfer at probability 0; a failure stalls the link | Medium | `ss_link`, `ss_link_failure` |
| **Monitoring** | | | |
| M1 | Response-time monitors on entry-level calls, system operations and external calls are ignored | Medium | `ss_monitors` |
| M2 | Entry-level assembly-operation response time is keyed by the system role | Low | `ss_role_lookup` |
| M3 | An infrastructure call disables the assembly-operation monitor | Medium | `h18_infrastructure_call` |
| M4 | Active-resource probes ignore `replicaID`; passive probes are shared between assemblies | Low | `ss_ps_multicore` |
| **SPD** | | | |
| S1 | Scale-in loses the jobs of the removed container | High | `spd_scale_in` |
| S2 | Composed triggers (AND, OR, XOR) give wrong results | High | `spd_or_trigger`, `spd_and_trigger` |
| S3 | Policies of one target group receive each other's time events | Medium | `spd_scale_in` |
| S4 | Unsupported stimuli give a NullPointerException at set-up | Medium | – (source) |
| S5 | Adjustor state: changed on request, not restored on disregard | Low | – (source) |
| S6 | `TargetGroupSizeConstraint` is not checked in the filter chain | Low | – (source) |
| S7 | Trigger aggregation: one merged stream, results on non-aggregated events | Medium | – (source) |
| S8 | Removed elements are not reported; population changes have no effect | Medium | – (source) |
| S9 | After an adjustment every linking resource connects all containers | Medium | – (source) |
| S10 | Top-down scale-in removes emptied containers one enactment late | Low | – (source) |
| S11 | Relative adjustment: `minAdjustmentValue` and a zero or negative percentage | Low | – (source) |
| S12 | Static fields in `TargetGroupUtils` leak between runs in one JVM | Medium | – (source) |
| **Determinism and reproducibility** | | | |
| D1 | No random source can be seeded | High | any model with branches or random demands |
| D2 | Same-time event order follows identity hash codes; measurements change between runs | High | `h24_two_scenarios`, `x_espresso` |
| D3 | Processor-sharing ties are broken by a random UUID hash | Medium | `ss_fifo_ties` |
| D4 | SPD transformations iterate `Set`s of model objects in identity-hash order | Medium | `spd_minimal` (container order) |
| D5 | Subscriber registration order depends on `getDeclaredMethods()` | Low | – (observed, no effect) |
| **Installation and runtime** | | | |
| I1 | Two required bundles are in no feature | High | fresh install from the update sites |
| I2 | The SPD transformations import `ProfilesLibrary` without a bundle dependency | High | `spd_minimal` |
| I3 | SPD adjustments need OSGi resolution of the transformations | Low | – |
| I4 | `generalsimulationconfiguration` has the version `1.0.0.quantifier` | Low | install `plugins/` |

## Crash and robustness

### R1 A handler that throws is disposed; the run continues and reports success

- **Severity:** High. One exception silently stops a whole stage (all SEFFs, all request ends, …)
  for every user, and the results look valid.
- **Where:** `eventdriver`: `BusImplementation.java:97` (`subscribe(subscriber, this::doError)`),
  `:188-194` (`doError` logs and calls the `@OnException` handlers); `AnnotatedSubscriber.java:19-43`
  rethrows the handler's exception. `core`: `CoreBehavior.java:54-57` only logs.
- **Observed:** RxJava's `LambdaObserver` calls the error consumer and disposes the subscription.
  That handler method never runs again in this run. The other handlers and the simulation continue
  until the simulation time; the launch ends without an error. The thrown invocation's result
  events are lost. Typical consequences:
  - `SeffSimulationBehavior.onSeffInterpretationProgressed` gone: no SEFF of any user advances;
  - `SystemSimulationBehavior.onActiveResourceFinished` gone: no demand completion advances a SEFF;
  - `UsageSimulationBehavior.onUserRequestFinished` gone: no user finishes a request.

  22 of 88 models are affected: 4 probes and 18 corpus models (`h10`, `h11`, `h12`, `h13`, `h18`,
  `h26`, `h27`, `h28`, `x_pem_harddisk`, `x_pem_hdd_infrastructure_calls`, `x_pem_infrastructure`,
  `x_pem_replicated_composite`, `x_pem_screencast_ms` with 2 variants,
  `x_pem_simple_optimisation_composites`, `x_sl_mediastore`, `x_sl_simulizar154`). The causes are
  W1, W3, W4, W5, W6, W7, W9 and M3; R3 adds another one.
- **Expected:** a handler exception stops the run with that error, and the launch reports failure.
  SimuLizar aborts the simulation on such errors.
- **Reproduction:** `ss_loop_zero`, `ss_usage_loop_zero`, `ss_guard_none`, `ss_two_demands` (each
  `observed/errors.txt` has one line per disposed handler); `h10_collection_iterator`,
  `h11_fork_sync`, `h13_passive_contention`.
- **Suggested fix:** catch the exception inside the subscriber so the subscription stays alive, and
  stop the simulation with a failure (for example `simulationDriver.stop()` plus an error status
  from the launcher, in `CoreBehavior.onGenericException` or in `doError`).

### R2 No stop condition besides simulation time; zero-time loops never end

- **Severity:** Medium. A model with a zero-time cycle hangs the launch; the maximum measurement
  count of the run configuration has no effect.
- **Where:** `core`: `SlingshotSimulationDriver.java:79-90` schedules only `SimulationFinished` at
  `config.getSimuTime()`. No Slingshot bundle reads the maximum measurement count.
- **Observed:** a closed user with think time 0 whose request aborts at t = 0 (W2) restarts at
  t = 0 forever. The simulation clock never advances. Corpus `x_pem_fork` (PCMFork_Example) does
  the same.
- **Expected:** SimuCom/SimuLizar also stop at the maximum measurement count. A guard against a
  zero-time livelock (for example a limit on events at one instant) would turn the hang into an
  error.
- **Reproduction:** `ss_delay_livelock`.
- **Suggested fix:** honour the maximum measurement count as a second stop condition; optionally a
  configurable limit of events per simulated instant.

### R3 A call between unconnected containers disables all network transfers

- **Severity:** Medium. One call on an unconnected pair stops every later remote call in the run.
- **Where:** `behavior.resourcesimulation`: `ResourceSimulation.java:352-407`, the throw at `:396`;
  `LinkingResourceTable.java:28-32,44-48`.
- **Observed:** `IllegalArgumentException: The resource containers of the assembly contexts are not
  directly connected by a linking resource`. Through R1, `onCallOverWireRequested` is disposed: no
  further transfer anywhere, also between connected containers. No measurement in the probe.
- **Expected:** a run error (SimuLizar fails the run for an unconnected pair), or a validation
  error before the start.
- **Reproduction:** `ss_link_unconnected`.
- **Suggested fix:** check at set-up that the containers of every assembly connector's ends are
  connected; at run time, fail the run (R1).

### R4 Unsupported actions, ResourceCalls and missing connectors end requests silently

- **Severity:** Medium. Requests vanish or take zero time, with no warning.
- **Where:** `behavior.systemsimulation`: `SeffInterpreter.java:375-386` (default case: empty
  result); `SystemModelRepositoryImpl.java:64-83,97-120` (no connector: empty result);
  `behavior.usagesimulation`: `UsageScenarioInterpreter.java:296-301`. Nothing reads
  `InternalAction.resourceCall__Action`.
- **Observed:**
  - an `EmitEventAction`, `RecoveryAction` or other unsupported action ends the request; corpus
    `x_pem_reliability` (RecoveryAction) completes no request and records nothing;
  - a ResourceCall is ignored: in `h23_resource_call` every response time is 0.0 and the CPU gets
    no job;
  - a missing delegation connector, assembly connector or SEFF ends the request.
- **Expected:** ResourceCalls put their demand on the resource (SimuLizar). Unsupported elements
  and incomplete models give an error or at least a warning.
- **Reproduction:** `h23_resource_call`.
- **Suggested fix:** implement ResourceCall; log an error (or fail the run) in the default cases.

### R5 Passive resource capacity cannot use component parameters

- **Severity:** Medium. Valid models that parameterise a capacity cannot be simulated.
- **Where:** `behavior.resourcesimulation`: `PassiveResourceTable.java:36-65` (evaluation at
  `:59-60`).
- **Observed:** the capacity is evaluated once, at construction, without a stack frame. A capacity
  `lockCount.VALUE` fails with `Architecture specification incomplete. Stackframe is missing id
  lockCount.VALUE`, inside the Guice construction of `ResourceSimulation`. The run ends before
  t = 0.
- **Expected:** the capacity is evaluated in the frame of the component instance (component and
  assembly-context parameters), as in SimuCom/SimuLizar.
- **Reproduction:** `x_pem_acquire` (Palladio-Example-Models `Acquire_Example`).
- **Suggested fix:** evaluate it with the component and assembly-context frames that
  `RepositoryInterpreter.caseBasicComponent` builds.

## Wrong results

### W1 Several resource demands of one InternalAction run concurrently and each advances the SEFF

- **Severity:** High. Response times are too short, actions are skipped or overlap, and the run
  then loses a handler (R1). Many models have a CPU and an HDD demand in one action. (The Slingshot
  usage tutorial already advises against several demands per action.)
- **Where:** `behavior.systemsimulation`: `SeffInterpreter.java:334-349` (one
  `ResourceDemandRequested` per demand, in one result); `SystemSimulationBehavior.java:343-372`
  (each `ActiveResourceFinished` advances the cursor); `SeffBehaviorWrapper.java:125-136`.
- **Observed:** all demands start at once. Each completion advances the shared cursor once. With
  CPU 1.0 and HDD 2.0 followed by two actions of 0.5 and 0.25 (rates 1.0), the request ends at 1.75
  while the HDD still serves. The HDD completion at 2.0 throws a `NullPointerException`
  (`SystemSimulationBehavior.java:349`), and `onActiveResourceFinished` is disposed.
- **Expected:** the demands are served one after the other, in list order (SimuCom/SimuLizar). The
  request takes 3.75 s.
- **Reproduction:** `ss_two_demands`; corpus `x_pem_screencast_ms*`.
- **Suggested fix:** request the next demand of the action when the previous one has finished;
  advance the cursor only after the last.

### W2 Delay resources are not simulated

- **Severity:** High. Every demand on a delay resource aborts the user; such models record no
  response time.
- **Where:** `behavior.resourcesimulation`: `ActiveResourceTable.java:49-52` (the `DELAY` case is
  commented out), `ResourceSimulation.java:190-206` (missing resource: `JobAborted`).
- **Observed:** `No such resource found! Job cannot be initiated, instead abort.` Then
  `JobAborted`, `ResourceDemandRequestAborted`, `UserAborted`. A closed user restarts as a new user.
  Corpus `h04_delay_resource`, `h25_deterministic_closed`, `x_sl_elsc_return` (416 aborts, no row).
- **Expected:** a delay resource serves each job for its demand, without queueing (SimuCom/SimuLizar).
- **Reproduction:** `ss_delay_resource`.
- **Suggested fix:** restore the delay resource.

### W3 ForkAction: asynchronous behaviours ignored; two or more synchronous behaviours fail

- **Severity:** High. ForkActions only work with exactly one synchronous behaviour.
- **Where:** `behavior.systemsimulation`: `SeffInterpreter.java:171-201`;
  `ForkBehaviorContextHolder.java:48-68` (`getCurrentProcessedBehavior` rotates on every call).
- **Observed:**
  - asynchronous forked behaviours are never started (`ss_async_fork`: the 5.0 s demand never
    appears);
  - a fork without a `SynchronisationPoint` throws `IllegalStateException: ForkAction must have a
    synchronisation point!` (`h12_fork_async`, `x_sl_simulizar154`);
  - all synchronous behaviours share one holder and the user's single stack. Each progress call
    moves the holder to the next unfinished behaviour, so one child's progress advances another
    child. With two behaviours this throws `NoSuchElementException` at
    `ForkBehaviorContextHolder.java:66` (`h11_fork_sync`, `h26_fork_double_resume`).
- **Expected:** PCM/SimuCom semantics: asynchronous behaviours run detached; each synchronous
  behaviour runs in parallel with its own cursor (and its own stack frames); the parent continues
  when all synchronous behaviours have finished. A fork without a SynchronisationPoint is valid.
- **Reproduction:** `ss_async_fork`, `h11_fork_sync`, `h12_fork_async`.
- **Suggested fix:** one context and cursor per forked behaviour, a join counter for the
  synchronous ones, no join for the asynchronous ones.

### W4 CollectionIteratorAction: INNER evaluated once; frame never popped

- **Severity:** High. After the first request with an iterator, no user finishes a request.
- **Where:** `behavior.systemsimulation`: `SeffInterpreter.java:264-298`; `behavior.usagesimulation`:
  `UsageSimulationBehavior.java:331-334` (precondition: exactly two frames).
- **Observed:** the frame with the INNER characterisations is pushed once per action and never
  popped. The INNER values are drawn once and reused for every iteration. At the end of the request
  the stack has one frame too many; `onUserRequestFinished` throws `IllegalArgumentException` and is
  disposed. In `h10_collection_iterator` only resource rows follow.
- **Expected:** one frame per iteration with freshly evaluated INNER values, popped after the
  iteration (SimuCom/SimuLizar).
- **Reproduction:** `h10_collection_iterator`; corpus `h27_collection_inner_multi`.
- **Suggested fix:** push and pop the iteration frame per iteration.

### W5 A loop with iteration count 0 runs once and throws

- **Severity:** Medium. Iteration counts with a zero probability (common in PMFs) break the run.
- **Where:** `behavior.systemsimulation.data`: `LoopBehaviorContextHolder.java:40,52-66`,
  `SeffBehaviorWrapper.java:125-130`; `behavior.usagesimulation.data`:
  `LoopScenarioBehaviorContext.java:38-56`.
- **Observed:** SEFF loop: the body runs once, then `NoSuchElementException: There is no action after
  StopAction`; `onSeffInterpretationProgressed` is disposed. Usage-model loop: `IllegalArgumentException:
  InitialLoopCount must be in [0,0] (maximalLoopCount) but is 0`; later users stop after their
  `Start`. A negative SEFF count fails a precondition.
- **Expected:** a count of 0 (or less) skips the body (SimuCom/SimuLizar).
- **Reproduction:** `ss_loop_zero` (count `IntPMF[(0;0.5)(2;0.5)]`), `ss_usage_loop_zero`.
- **Suggested fix:** if the count is ≤ 0, continue with the loop action's successor.

### W6 Acquire/Release without ParametricResourceDemand throw

- **Severity:** High. Most Palladio example models model acquire and release without a demand.
- **Where:** `behavior.systemsimulation`: `SeffInterpreter.java:230-262`.
- **Observed:** `NoSuchElementException: No parametric resource demand specified for AcquireAction!`;
  `onSeffInterpretationProgressed` is disposed. Corpus `h13`, `h28`, `x_pem_harddisk`,
  `x_sl_mediastore`.
- **Expected:** acquire and release one unit when no demand is given (SimuCom/SimuLizar always use
  one unit).
- **Reproduction:** `h13_passive_contention`.
- **Suggested fix:** default amount 1.

### W7 Composite components and subsystems are not simulated

- **Severity:** High. Models with composites record nothing, without an error.
- **Where:** `behavior.systemsimulation`: `RepositoryInterpreter.java:232-251`
  (`caseComposedProvidingRequiringEntity` only descends when the composite provides the given role);
  `SystemModelRepositoryImpl.java:122-169`; `behavior.resourcesimulation`: allocation lookup
  `ResourceEnvironmentAccessor.java:42-47`.
- **Observed:**
  - entered from the system, the role given is the system's outer role. The composite is not entered
    and the user stops silently: `h15_composite`, `x_pem_subsystem`, `x_pem_subsystem_nested` record
    no row;
  - entered through an assembly connector, the inner assembly is entered as a new root context. Its
    demands find no allocation context (only the composite's assembly is allocated), so every job
    aborts. The abort then fails (`User Interpretation Context not found!`) and disposes
    `onResourceDemandRequestAborted`: `x_pem_replicated_composite`,
    `x_pem_simple_optimisation_composites`.
- **Expected:** composites and subsystems are entered through their delegation connectors, and their
  inner assemblies run on the composite's allocation (SimuCom/SimuLizar).
- **Reproduction:** `h15_composite`.
- **Suggested fix:** follow the delegation connectors into the composite and keep the outer assembly
  context path for the allocation lookup.

### W8 Provided and required roles are resolved by role id only

- **Severity:** Medium. Wrong routing as soon as a component is assembled twice. (Not hit by the
  examples, nor by SPD's load balancer, which adds a new required role per replica.)
- **Where:** `behavior.systemsimulation`: `SystemModelRepositoryImpl.java:64-83` (external calls:
  first assembly connector with the required role id), `:122-169` (entry calls: first assembly
  whose component provides the delegated inner role).
- **Observed:** the entry call delegated to `FrontB` runs in `FrontA`. `FrontB`'s external call
  goes through `FrontA`'s connector to `Slow` instead of `Fast`.
- **Expected:** the delegation connector's assembly context serves the entry call; an external call
  uses the assembly connector of the calling assembly context (SimuCom/SimuLizar).
- **Reproduction:** `ss_role_lookup`.
- **Suggested fix:** use `ProvidedDelegationConnector.assemblyContext_ProvidedDelegationConnector`;
  filter assembly connectors by `requiringAssemblyContext_AssemblyConnector`.

### W9 A guarded branch without a true guard disposes the SEFF handler

- **Severity:** Low. The model is incomplete, but the effect is a silent freeze of all users (R1).
- **Where:** `behavior.systemsimulation`: `SeffInterpreter.java:107-117`.
- **Observed:** `IllegalStateException: No branch transition was active`; then no SEFF progresses;
  no row; the run "succeeds". An empty branch throws too.
- **Expected:** the run stops with this error (SimuLizar does).
- **Reproduction:** `ss_guard_none`.
- **Suggested fix:** covered by R1.

### W10 Infrastructure-call input parameters are ignored

- **Severity:** Low. Only matters for infrastructure SEFFs that use their inputs.
- **Where:** `behavior.systemsimulation`: `SystemSimulationBehavior.java:322-341`;
  `InfrastructureCallsContextHolder.java:32-50`.
- **Observed:** no input frame is built for the callee; its input variable usages are not evaluated.
- **Expected:** the input variable usages are evaluated per call, as for external calls (SimuLizar).
- **Reproduction:** `h18_infrastructure_call` (together with M3).
- **Suggested fix:** push an input frame as `onSEFFExternalActionCalled` does.

### W11 Outdated processor-sharing completion events record state samples

- **Severity:** Low. Extra state and utilisation tuples with unchanged values; time-weighted
  statistics are not affected, counts and unweighted means are.
- **Where:** `behavior.resourcesimulation`: `ProcessorSharingResource.java:103-125` (a stale event is
  recognised and does nothing), `AbstractActiveResource.java:132-141` (an
  `ActiveResourceStateUpdated` is appended after every `JobProgressed`).
- **Observed:** each arrival and each completion leaves the previous completion event in the
  calendar. When it comes due it is recognised as stale, but a state sample is still taken. In
  `ss_fifo_ties` the tuples at t = 1, 2, 4, 5 repeat the value 3.
- **Expected:** a sample only when the state changes (SimuCom removes the outdated event).
- **Reproduction:** `ss_fifo_ties`, `ss_ps_multicore`.
- **Suggested fix:** skip the state update for stale events.

### W12 Link failures: a draw per transfer at probability 0; a failure stalls the link

- **Severity:** Medium. With a failure probability above 0 the network stops after the first
  failure. At probability 0 the extra draws only shift the branch random stream.
- **Where:** `behavior.resourcesimulation`: `SimulatedLinkingResource.java:52-66`.
- **Observed:**
  - every completion of a link job calls `Math.random()`, also when the failure probability is 0;
  - after a failure the job is removed and `JobAborted` returned, but the link schedules no
    completion for the next queued job. In `ss_link_failure` (probability 0.5, two users) the first
    transfer fails at t = 0.71; the retry and the other user's transfer wait until the end of the run.
- **Expected:** no draw without failure simulation; after a failure the link continues with the next
  queued transfer.
- **Reproduction:** `ss_link`, `ss_link_failure`.
- **Suggested fix:** draw only if `failureRate > 0`; after `abortJob`, start the next job as on a
  normal completion.

## Monitoring

### M1 Response-time monitors on entry-level calls, system operations and external calls are ignored

- **Severity:** Medium. Monitor repositories made for SimuLizar silently lose most response times.
- **Where:** `behavior.systemsimulation.monitor`:
  `OperationCallActionResponseTimeMonitoringBehavior.java:85` (only
  `AssemblyOperationMeasuringPoint`); `behavior.usagesimulation.monitor`:
  `UsageScenarioResponseTimeMonitoringBehavior.java` (only `UsageScenarioMeasuringPoint`).
- **Observed:** monitors on `EntryLevelSystemCallMeasuringPoint`, `SystemOperationMeasuringPoint`,
  `ExternalCallActionMeasuringPoint` get no calculator. No row, no warning.
- **Expected:** these measuring points record response times (SimuLizar supports them), or a warning
  names the unsupported monitors.
- **Reproduction:** `ss_monitors` (four monitors, only the usage-scenario one records).
- **Suggested fix:** calculators for these points from the existing passed-element and call events;
  otherwise a warning at set-up.

### M2 Entry-level assembly-operation response time is keyed by the system role

- **Severity:** Low. A natural measuring point (assembly, component role) never matches an entry
  call.
- **Where:** `behavior.systemsimulation.monitor`:
  `OperationCallActionResponseTimeMonitoringBehavior.java:80-128`,
  `AssemblyOperationCompoundKey.java:56-71`.
- **Observed:** for the SEFF of an entry call, the key uses the request's provided role, which is the
  system's outer role. `(FrontA, prov_front)` records nothing; `(FrontA, sysprov2)` records the entry
  call.
- **Expected:** the key uses the component's provided role (the delegation connector's inner role).
- **Reproduction:** `ss_role_lookup`.
- **Suggested fix:** map the outer role to the inner role through the delegation connector.

### M3 An infrastructure call disables the assembly-operation monitor

- **Severity:** Medium. One infrastructure call ends all assembly-operation response times of the run.
- **Where:** `behavior.systemsimulation.monitor`: `AssemblyOperationCompoundKey.java:56-67`,
  called from `OperationCallActionResponseTimeMonitoringBehavior.java:119,138`.
- **Observed:** the key is computed for every root SEFF start. A SEFF reached by an infrastructure
  call has neither a user request nor a call-over-wire request, so `AssemblyOperationCompoundKey.of`
  throws `IllegalArgumentException: Cannot determine Signature of SEFFInterpretationContext`. Both
  handlers of the behaviour are disposed (R1).
- **Expected:** infrastructure SEFFs are skipped (or measured); the monitor keeps working.
- **Reproduction:** `h18_infrastructure_call`; corpus `x_pem_infrastructure`,
  `x_pem_hdd_infrastructure_calls`.
- **Suggested fix:** return early when the context has no request.

### M4 Active-resource probes ignore `replicaID`; passive probes are shared between assemblies

- **Severity:** Low. Per-core and per-instance series are not available; the monitors look valid.
- **Where:** `behavior.resourcesimulation.monitor`: `ActiveResourceProbeTable.java:33-63` (keyed by
  the processing resource specification), `PassiveResourceProbeTable.java:28-31` (keyed by the
  passive resource id).
- **Observed:** the monitors for replica 0 and replica 1 of a 2-core CPU give identical series. The
  instances of one passive resource in several assembly contexts feed one merged series.
- **Expected:** per-core series for `replicaID` (SimuCom/SimuLizar); passive series per
  (passive resource, assembly context).
- **Reproduction:** `ss_ps_multicore`.
- **Suggested fix:** include the replica and the assembly context in the probe keys.

## SPD

### S1 Scale-in loses the jobs of the removed container

- **Severity:** High. After a scale-in, requests disappear without an event; their users never
  finish and their response times are never recorded.
- **Where:** `behavior.resourcesimulation`: `ResourceSimulation.java:329-350` (`onModelAdjusted`
  removes the resources), `:190-206` and `:253-269` (an event for a removed resource becomes
  `JobAborted`); `ActiveResourceTable.java:80-85`.
- **Observed:** in `spd_scale_in`, `AppServer_1` is removed at t = 80 while four jobs run on its CPU.
  Only the job that holds the pending `JobProgressed` gets an event: it is aborted at t = 87
  (`JobAborted`, `ResourceDemandRequestAborted`, `UserAborted`), and its open-workload user is
  dropped. The other three jobs (started at t = 69, 72, 78) produce no further event.
- **Expected:** either the removed container finishes its jobs (drain), or every job on it is
  aborted with an event so that the user can be restarted.
- **Reproduction:** `spd_scale_in`.
- **Suggested fix:** keep removed resources until their queues are empty (drain), or abort all their
  jobs in `onModelAdjusted`.

### S2 Composed triggers (AND, OR, XOR) give wrong results

- **Severity:** High. Policies with composed triggers fire at wrong times or never.
- **Where:** `behavior.spd`: `LogicalANDComboundFilter.java:10-27`, `LogicalORCompoundFilter.java:12-44`,
  `LogicalXORCompoundFilter.java:15-50`, `FilterChain.java:100-139`.
- **Observed:**
  - OR: `next` never evaluates the children and the result stays `null`. The outer chain stays
    parked after the OR filter. The next event resumes the chain there, skipping the target-group
    check and the trigger. In `spd_or_trigger` (OR of t == 50 and t == 70): no adjustment at 50,
    then an adjustment at 70 that went straight to the `Adjustor`.
  - AND: `disregard` does not reset the children's iterator. The next event resumes after the child
    that failed. In `spd_and_trigger` (AND of t == 50 and t == 70, never true): an adjustment at 70.
  - XOR: with two children the result is inverted after the first child (from the source).
  - Two stimulus kinds in one policy register two `MeasurementMade` subscribers, so each measurement
    runs the chain twice (from the source).
- **Expected:** AND, OR, XOR of the children's results for each event; every event starts the
  chain from the target-group check.
- **Reproduction:** `spd_or_trigger`, `spd_and_trigger` (`observed/spd.jsonl`, records
  `"k":"eval"`).
- **Suggested fix:** evaluate all children per event with a fresh iterator and combine their
  results; do not leave the outer chain parked.

### S3 Policies of one target group receive each other's time events

- **Severity:** Medium. With `GreaterThanOrEqualTo` or `LessThanOrEqualTo` a policy can fire on
  another policy's time event.
- **Where:** `behavior.spd`: `TargetGroupChecker.java:36-40` (a `SimulationTimeReached` passes when
  it is for the same target group).
- **Observed:** in `spd_scale_in` the "Scale in at 80" policy evaluates the t = 50 event of the
  scale-out policy and is rejected only by its `SimulationTimeChecker`.
- **Expected:** each policy evaluates only the time events of its own trigger.
- **Reproduction:** `spd_scale_in` (`observed/spd.jsonl`).
- **Suggested fix:** let `SimulationTimeReached` carry the trigger (or policy) and check it.

### S4 Unsupported stimuli give a NullPointerException at set-up

- **Severity:** Medium. Valid SPD models fail with an unclear error.
- **Where:** `behavior.spd`: `StimuliInterpreter.java` (no case for `NumberOfElements`, memory, HDD,
  network utilisation), `ScalingTriggerInterpreter.java:44-47` (`SimpleFireOnTrend`: no checker),
  `SpdInterpreter.java:56`.
- **Observed:** a null checker; a NullPointerException while the SPD model is interpreted.
- **Expected:** support, or a configuration error that names the unsupported stimulus or trigger.
- **Reproduction:** none (source).
- **Suggested fix:** throw a descriptive exception in the default cases.

### S5 Adjustor state: changed on request, not restored on disregard

- **Severity:** Low. Cooldown and thrashing state can be wrong after a failed adjustment.
- **Where:** `behavior.spd`: `Adjustor.java:28-61`; `SPDAdjustorContext.java:57,109-112`.
- **Observed:** the adjustor updates the cooldown, the adjustment count and the group's last enacted
  policy when the request is made, also when the transformation later fails (`ModelAdjusted(false)`)
  or changes nothing. `doOnDisregard` assigns `state = previousState`, but the chain keeps the state
  object it was built with, so nothing is restored (no effect on results known).
- **Expected:** state changes only for a successful adjustment; a disregard restores the state.
- **Reproduction:** none (source).
- **Suggested fix:** commit the state on a successful `ModelAdjusted`; restore through the chain's
  reference.

### S6 `TargetGroupSizeConstraint` is not checked in the filter chain

- **Severity:** Low. A policy at the size limit still passes the chain, consumes its cooldown and
  starts a transformation that changes nothing.
- **Where:** `behavior.spd`: `SPDAdjustorContext.java:86` (only `ThrashingConstraint`s enter the
  chain); the size is clamped in `spd/common/AdjustmentCalculator.qvto:47-55`.
- **Expected:** a change clamped to zero disregards the event, before the adjustor's state changes.
- **Reproduction:** none (source).

### S7 Trigger aggregation: one merged stream, results on non-aggregated events

- **Severity:** Medium. Utilisation and task-count triggers depend on sample timing rather than on
  the group's state.
- **Where:** `behavior.spd`: `AbstractManagedElementTriggerChecker.java:53-137` (`:67-71`
  aggregator choice, `:76-123`).
- **Observed:**
  - the samples of all containers of the group feed one stream; there is no aggregation per element
    before `aggregationOverElements` is applied (MAX = maximum of the last 10 samples of any
    container);
  - `getResult` also runs for events of other metrics that were not aggregated;
  - samples and the EMA emit slot are consumed even when a later filter disregards the event;
  - AVERAGE is an EMA with fixed parameters (window 60 s, emit interval 10 s, α = 0.2), so an
    average-utilisation policy cannot fire before t = 60 s; MAX, MIN, MEDIAN, SUM use the last 10
    values (count-based). `SlidingTimeWindowAggregation` exists but is unused.
- **Expected:** a documented aggregation over the group's elements; the constants configurable.
- **Reproduction:** none (source).

### S8 Removed elements are not reported; population changes have no effect

- **Severity:** Medium. Monitors and passive resources of removed elements stay; competing-consumer
  adjustments of a closed population do nothing.
- **Where:** `behavior.spd.adjustment`: `SpdAdjustmentBehavior.java:96-117` (only new and deleted
  containers and new allocation contexts are reported); `behavior.usagesimulation`:
  `UsageSimulationBehavior.java:247` (population read once at start).
- **Expected:** deleted allocation contexts and assemblies reported; their monitors and passive
  resources removed; a population change applied.
- **Reproduction:** none (source).

### S9 After an adjustment every linking resource connects all containers

- **Severity:** Medium. The network topology changes with the first adjustment; calls may use a link
  that did not connect the two containers before.
- **Where:** `spd.semantic.transformations`: `spd/common/ResourceEnvironmentTransformation.qvto:11-23`.
- **Observed:** `modifyLinkingResource` sets `connectedResourceContainers_LinkingResource` of every
  link of the environment to the complete set of containers.
- **Expected:** a new container joins the links of the unit container; other links stay unchanged.
- **Reproduction:** none (source).

### S10 Top-down scale-in removes emptied containers one enactment late

- **Severity:** Low. Empty containers stay until the next adjustment.
- **Where:** `spd.semantic.transformations`: `spd/topdown/TopDownTransformationInfrastructure.qvto:58-63`
  (the code's own TODO).
- **Reproduction:** none (source).

### S11 Relative adjustment: `minAdjustmentValue` and a zero or negative percentage

- **Severity:** Low. The meaning of `minAdjustmentValue` for scale-in and for 0 % is unclear.
- **Where:** `spd.semantic.transformations`: `spd/common/AdjustmentCalculator.qvto:59-74`.
- **Observed:** with `x = size · percentage / 100`, the change is `max(x, min)` if `x > 0`, else
  `min(x, min)`. With 0 % and a positive `minAdjustmentValue` the change is 0; with a negative
  percentage and a positive minimum the minimum is ignored; with 0 % and a negative minimum the group
  shrinks by the minimum.
- **Expected:** documented semantics, probably a minimum magnitude in the direction of the
  percentage.
- **Reproduction:** none (source).

### S12 Static fields in `TargetGroupUtils` leak between runs in one JVM

- **Severity:** Medium. Several runs in one Eclipse session or in a batch (experiment automation)
  can evaluate target groups against the first run's models.
- **Where:** `behavior.spd`: `TargetGroupUtils.java:33-34` (`static final Allocation allocation`,
  `static final Configuration configuration`, initialised once from the injector).
- **Expected:** per-run instances.
- **Reproduction:** none (source). Our runner runs one model per JVM, so we did not observe it.
- **Suggested fix:** inject them per run.

## Determinism and reproducibility

### D1 No random source can be seeded

- **Severity:** High. Runs cannot be repeated, and common-random-number comparisons between designs
  are impossible.
- **Where:**
  - `behavior.generalsimulationconfiguration`: `GeneralSimulationConfigurationBehavior.java:32-35`
    initialises the StoEx cache with the JVM-global `ProbabilityFunctionFactoryImpl`, whose
    generator is an unseeded `java.util.Random` (`DefaultRandomGenerator`). The seed of
    `SimuComConfig` is not applied;
  - `common.utils`: `TransitionDeterminer.java:162` (`Math.random(); // TODO: Use SimuCom Random
    instead`) for usage-model and SEFF branches;
  - `behavior.resourcesimulation`: `SimulatedLinkingResource.java:53` (`Math.random()` for link
    failures);
  - every event, user and job gets `UUID.randomUUID()`; `Job.hashCode` and `User.hashCode` hash it
    (see D3).
- **Expected:** one seeded stream (SimuCom's generator with the configured seed) for StoEx and
  branches, as in SimuCom/SimuLizar.
- **Reproduction:** any model with branches or random demands (`ss_loop_zero`, `h24_two_scenarios`).
- **Suggested fix:** apply `SimuComConfig`'s seed to the factory's generator and draw branches from
  it.

### D2 Same-time event order follows identity hash codes; measurements change between runs

- **Severity:** High. Identical runs give different measurements, also for models without any random
  number.
- **Where:**
  - `eventdriver`: `Result.java:25-33` copies the handler's events into a `HashSet`;
    `core`: `CoreBehavior.java:36-52` schedules them in that order;
  - many handlers build their results in `HashSet`s, `Collectors.toSet()` or `Set.of` (for example
    `UsageScenarioInterpreter.java:353`);
  - `LinkingResourceTable.java:28-32`: link candidates in hash order of the link ids.
- **Observed:** with all random sources seeded (our agent), the order of same-time events still
  changes from run to run:
  - `h24_two_scenarios` (two scenarios on one FCFS CPU): three runs in one JVM give two different
    `measurements.csv`; scenario 2 mean response time 0.1065 vs 0.1156. The start order of the two
    scenarios' users decides which user gets which drawn demand;
  - `x_espresso` (30 closed users, constant demands, no random draw): three runs give three
    different files; mean `State of Active Resource` 22.74 and 22.73, and 21.12 in insertion order;
  - the event trace differed between two stock runs on 6 of 9 smoke models.
- **Expected:** a deterministic same-time order, for example the order in which a handler creates
  its events.
- **Reproduction:** `h24_two_scenarios`, `x_espresso` (run each 3 times; see their READMEs).
- **Suggested fix:** `LinkedHashSet` in `Result` and in the handlers (or lists). Our deterministic
  mode does exactly this at class-load time and was byte-reproducible on all models that run.

### D3 Processor-sharing ties are broken by a random UUID hash

- **Severity:** Medium. Which of several jobs with equal remaining demand completes first is random.
- **Where:** `behavior.resourcesimulation`: `ProcessorSharingResource.java:40,151-174` (`Hashtable<Job,
  Double>`, first minimum in table order); `behavior.resourcesimulation.data`: `Job.java:59-62`
  (`hashCode` = hash of a random UUID).
- **Expected:** a deterministic tie rule (for example arrival order).
- **Reproduction:** `ss_fifo_ties` (three jobs with equal demands).
- **Suggested fix:** `LinkedHashMap` (our one source patch).

### D4 SPD transformations iterate `Set`s of model objects in identity-hash order

- **Severity:** Medium. The container order of the resource environment and of each link after an
  adjustment, and some placements, differ between runs.
- **Where:** `spd.semantic.transformations`: `spd/common/ResourceEnvironmentTransformation.qvto:13-18`
  (`Set(ResourceContainer)` assigned to `resourceContainer_ResourceEnvironment` and to each link);
  `->any(true)` on sets in `pcm/helpers/Commons.qvto:240,391,404` and in the top-down placement.
  Elements created by the transformation get time-based ids (`EcoreUtil.generateUUID()`).
- **Observed:** without insertion order for QVTo's and OCL's collections, the order of the
  containers after the scale-out in `spd_minimal` follows identity hashes.
- **Expected:** a deterministic order (`OrderedSet`, or sorting).
- **Reproduction:** `spd_minimal` (save the adjusted models of two runs and compare the order of
  `resourceContainer_ResourceEnvironment`).
- **Suggested fix:** `OrderedSet` in these mappings.

### D5 Subscriber registration order depends on `getDeclaredMethods()`

- **Severity:** Low. No effect today; a risk for future behaviours.
- **Where:** `core`: `SlingshotSimulationDriver.java:62-72`; `eventdriver`: `BusImplementation.java:116`.
- **Observed:** in one of our runs the two reified handlers of
  `UsageScenarioResponseTimeMonitoringBehavior` (`<Start>`, `<Stop>`) were registered in the other
  order. Within every Slingshot class, methods that subscribe to one event type have disjoint reified
  types, so no run changed. `getDeclaredMethods()` has no specified order.
- **Suggested fix:** sort the methods (for example by name) before registering, or document that
  handlers of one event type in one class must not depend on their order.

## Installation and runtime

### I1 Two required bundles are in no feature

- **Severity:** High. An install of the features from the nightly update sites does not resolve.
- **Where:** `org.palladiosimulator.analyzer.slingshot.common.utils` is built
  (`bundles/pom.xml:34`) and required by six bundles (for example
  `behavior.resourcesimulation/META-INF/MANIFEST.MF:17`), but it is in no feature of
  Palladio-Analyzer-Slingshot. `org.palladiosimulator.spd.semantic.transformations` is built by
  Palladio-Addons-SPD-Metamodel (`bundles/pom.xml:23`) but is not in `org.palladiosimulator.spd.feature`.
  Only the Initialisation extension's feature lists both.
- **Observed:** neither bundle is on the Slingshot, SPD or Palladio nightly update sites; the
  bundles that require them do not resolve. We built the two bundles from source.
- **Expected:** both bundles in the features that need them.
- **Suggested fix:** add `common.utils` to `org.palladiosimulator.analyzer.slingshot.feature` and
  `spd.semantic.transformations` to the SPD feature (or to the SPD interpreter feature).

### I2 The SPD transformations import `ProfilesLibrary` without a bundle dependency

- **Severity:** High. No SPD adjustment can run in an install without
  `org.palladiosimulator.architecturaltemplates.catalog`.
- **Where:** `spd.semantic.transformations`: `pcm/helpers/Constructors.qvto:1`,
  `pcm/helpers/Wiring.qvto:1`, `pcm/helpers/Loadbalancing.qvto:1`
  (`import org.palladiosimulator.architecturaltemplates.catalog.black.ProfilesLibrary;`);
  `META-INF/MANIFEST.MF` requires only `org.eclipse.m2m.qvt.oml.runtime`.
- **Observed:** QVTo cannot resolve the blackbox import, so `MainTransformation` cannot be loaded
  and no adjustment is applied. No operation of `ProfilesLibrary` is called on the paths that
  `MainTransformation` takes. With a stub bundle that only registers the blackbox, every adjustment
  works.
- **Expected:** SPD adjustments work with the bundles of the SPD and Slingshot features.
- **Reproduction:** `spd_minimal` in an install made from the update sites.
- **Suggested fix:** remove the unused imports, or add the dependency (bundle and feature).

### I3 SPD adjustments need OSGi resolution of the transformations

- **Severity:** Low. Only matters for plain-Java use (tests, containers).
- **Where:** `behavior.spd.adjustment`: `SpdAdjustorModule.java:17`
  (`platform:/plugin/org.palladiosimulator.spd.semantic.transformations/transformations/spd/MainTransformation.qvto`);
  the imports `pcm.helpers.*` and `spd.*` resolve through the bundle's exported packages.
- **Observed:** on a flat class path (no Equinox) QVTo did not resolve `MainTransformation`'s
  imports. In Equinox they resolve.
- **Suggested fix:** none needed if OSGi is the only supported runtime; otherwise a class-path
  resolver for the imports.

### I4 `generalsimulationconfiguration` has the version `1.0.0.quantifier`

- **Severity:** Low. The jar is published with a constant version, so p2 never sees an update.
- **Where:** `behavior.generalsimulationconfiguration/META-INF/MANIFEST.MF:5`
  (`Bundle-Version: 1.0.0.quantifier`, probably meant as `.qualifier`). The installed jar is
  `org.palladiosimulator.analyzer.slingshot.behavior.generalsimulationconfiguration_1.0.0.quantifier.jar`.
- **Suggested fix:** `Bundle-Version: 1.0.0.qualifier`.

## Appendix A: running the reproductions

**With stock Slingshot (enough for every reproduction).**

1. Import the reproduction folder into an Eclipse workspace with the Slingshot nightly (and the SPD
   interpreter for the `spd_*` folders).
2. Create a *Slingshot Simulation* run configuration. Each README lists the allocation, usage model
   and monitor repository (and SPD and semantic SPD model) files and the maximum simulation time.
   Use EDP2 with an in-memory repository.
3. Set the console log level to INFO. Handler exceptions appear as `An error occurred: <exception>::
   <message>`.
4. Compare with the README's "What to look for". Our results are in `observed/`.

Models that draw random numbers give other times and values in an unseeded run; the effect does
not depend on them. The `spd_*` folders need the `ProfilesLibrary` blackbox (I2).

**With our headless runner (optional).** The runner is part of the SimOxide repository
(`reference/slingshot/`; documentation `docs/reference-simulator/slingshot.md`). It uses an
existing install read-only and needs Java 21, Python 3 and a shell.

```sh
cd reference/slingshot
./setup.sh                                     # once: bundles, agent, Equinox configurations
R=upstream-report/reproductions
./run.sh --stock -model $R/ss_two_demands -out /tmp/a -simTime 12           # unmodified nightly
./run.sh --stock -model $R/ss_two_demands -out /tmp/b -simTime 12 -seed 1   # + seeded random sources
./run.sh         -model $R/ss_two_demands -out /tmp/c -simTime 12 -seed 1   # deterministic mode
tools/project.py /tmp/c                        # projection.jsonl, projection-m.jsonl
```

- `--stock` runs the nightly unmodified. With `-seed N` a Java agent also seeds the StoEx generator
  (SimuCom's MT19937), `Math.random()` (`java.util.Random(N + 1000003)`) and UUIDs.
- The default (deterministic) mode adds insertion order for the hash-based collections of Slingshot,
  QVTo and OCL, and a `LinkedHashMap` job table in `ProcessorSharingResource`. Each of its orders is
  one of the orders stock Slingshot can produce.
- Outputs: `measurements.csv` (EDP2 raw rows), `trace.jsonl` (every dispatched event), `tape.jsonl`
  (every random draw with its call site), `spd.jsonl` (SPD filter decisions and adjustments),
  `summary.json`. Handler errors are in the console log.
- `ss_delay_livelock` does not terminate; run it with `-trace false` and stop it.
- The runner includes the `ProfilesLibrary` stub bundle, so I2 is only visible in a plain install.

## Appendix B: environment

- Linux x86-64 (Ubuntu 24.04), OpenJDK 21.0.12, `-Xmx2g`, one JVM at a time.
- Install: p2 director from the update sites `palladio-build-updatesite/nightly`,
  `palladio-addons-spd-metamodel/nightly`, `palladio-analyzer-slingshot/nightly`, the Eclipse
  2025-12 release and Orbit, on 2026-09-29. Installed: `org.palladiosimulator.analyzer.slingshot.feature`,
  the `extension.pcm.core`, `extension.monitoring` and `extension.spd.interpreter` features,
  `org.palladiosimulator.recorderframework.edp2`, `org.palladiosimulator.pcm.resources`.
- `common.utils` and `spd.semantic.transformations` built from source at `8bf9adc` and `95cdb0b`
  (I1).
- Pinned jars: [`install.lock`](./install.lock) (SHA-256 of 47 jars).
- Sources read at Palladio-Analyzer-Slingshot `8bf9adc0a85e` (2026-08-21) and
  Palladio-Addons-SPD-Metamodel `95cdb0b67727` (2026-08-18).

## Appendix C: models and licences

| Folder | Source | Licence |
|---|---|---|
| `ss_*` | generated for this study (SimOxide `tools/probegen.py`); `ss_delay_livelock`, `ss_link_failure`, `ss_link_unconnected` are hand-edited variants | EPL-2.0 (SimOxide) |
| `h*` | hand-made SimOxide corpus models | EPL-2.0 (SimOxide) |
| `x_espresso` | espresso example (Palladio-Documentation-Slingshot minimal example without SPD), SimOxide monitor repository added | licence pending confirmation |
| `x_pem_acquire` | Palladio-Example-Models `Acquire_Example` (`4a8dc45`), SimOxide monitor repository added | licence pending confirmation |
| `spd_*` | Palladio-Documentation-Slingshot `examples/minimalexample` (`9444815`); `spd_scale_in`, `spd_or_trigger`, `spd_and_trigger` are variants | licence pending confirmation |

The per-folder READMEs give the exact changes.

## Appendix D: cross-reference to SimOxide's quirk catalogue

For readers of SimOxide's specification (`docs/spec/slingshot/quirks.md`):

| Report | Quirk | Report | Quirk | Report | Quirk |
|---|---|---|---|---|---|
| R1 | Q18 | W5 | Q19 | S1 | Q5 |
| R2 | Q32 | W6 | Q24 | S2 | Q12 |
| R3 | Q37 | W7 | Q30 | S3 | Q15 |
| R4 | Q35 | W8 | Q20 | S4 | Q11 |
| R5 | Q27 | W9 | Q21 | S5 | Q1, Q2 |
| W1 | Q17 | W10 | Q36 | S6 | Q9 |
| W2 | Q25 | W11 | Q26 | S7 | Q3, Q13 |
| W3 | Q23 | W12 | Q28 | S8 | Q14 |
| W4 | Q22 | M1 | Q29 | S9 | Q6 |
| M2 | Q33 | M3 | Q31 | S10 | Q7 |
| M4 | Q34 | D4 | Q8, Q16 | S11 | Q4 |
| S12 | Q10 | D1-D3, D5, I1-I4 | – | | |

# SEFF interpretation (Slingshot)

Rules `SS-ACT-*`. System entry, composition and calls are handled by `SystemSimulationBehavior`
(`[SY]SystemSimulationBehavior.java`) with `RepositoryInterpreter` (`[SY]interpreters/RepositoryInterpreter.java`)
and `SystemModelRepositoryImpl` (`[SY]repository/SystemModelRepositoryImpl.java`); SEFF actions by
`SeffSimulationBehavior` (`[SY]SeffSimulationBehavior.java`) with a fresh `SeffInterpreter` per event
(`[SY]interpreters/SeffInterpreter.java`).

A SEFF in progress is a `SEFFInterpretationContext`: assembly context, behaviour context holder
(root, branch, loop, fork, infrastructure calls), parent, caller, call-over-wire request, result
frame (`[SYD]entities/seff/SEFFInterpretationContext.java`). The holder owns a cursor per behaviour,
`SeffBehaviorWrapper`, that starts at the `StartAction` and follows `successor` links; every
`getNextAction()` **advances** it (`[SYD]entities/seff/behaviorcontext/SeffBehaviorWrapper.java:125-136`,
`SeffBehaviorContextHolder.java:123-127`). Each `SEFFInterpretationProgressed` interprets
`getNextAction()` of its context (`SeffSimulationBehavior.java:47-62`).

The interpreters read the PCM objects live at each step: model changes (SPD adjustments) take effect
for the next step ([SS-SPD-5](./spd.md#ss-spd-5-applying-the-change)).

## SS-ACT-1 Entry-level system calls

`UserEntryRequested` → `onUserEntryRequested` (`SystemSimulationBehavior.java:89-137`):

- **SS-ACT-1.1 Delegation.** The `ProvidedDelegationConnector` of the system whose outer role id is the
  call's role id, provided that role is a provided role of the system
  (`SystemModelRepositoryImpl.java:97-120`). None: empty result, the user stops silently.
- **SS-ACT-1.2 The assembly is found by role, not by the connector.** The SEFF is the first SEFF for
  the signature id of the first assembly context (system list order) whose component is a basic
  component providing the connector's **inner role** (`:122-153,189-197`). The assembly context is
  the first one whose component provides that role (`:155-169`), **not** the connector's
  `assemblyContext_ProvidedDelegationConnector`. With one component assembled twice, every entry call
  reaches the first assembly. Quirk [SS-Q20](./quirks.md#ss-q20). Probe `ss_role_lookup` (call
  `direct`, delegated to `FrontB`, runs in `FrontA`).
- **SS-ACT-1.3 Frames.** The input frame (no parent) gets the call's input parameter usages, evaluated
  against the user's top frame (`SimulatedStackHelper.createAndPushNewStackFrame`,
  `[CU]SimulatedStackHelper.java:115-144`; draw site `param`). Then `caseBasicComponent`
  (`RepositoryInterpreter.java:136-175`) pushes the **component parameter frame** (parent: the input
  frame) with the component's parameter usages and the **assembly configuration frame** (parent: the
  component frame) with the assembly context's configuration parameter usages (draw site
  `component`), and starts every SEFF of the component whose signature id matches:
  `SEFFInterpretationProgressed` with a root holder, no caller, a new empty result frame.
- **SS-ACT-1.4 Return.** The root SEFF's `Stop` without a caller ends the request
  (`SeffSimulationBehavior.java:126-134`): pop two frames (`cleanUpComponentParameterStackFrames`,
  `:179-184`), `UserRequestFinished` with the result frame →
  [SS-WL-4.4](./usage.md#ss-wl-4-usage-actions).
- **SS-ACT-1.5 Composite components and subsystems.** `doSwitch` of a composed structure reaches
  `caseComposedProvidingRequiringEntity`, which only descends (through the composite's provided
  delegation connector, to the inner assembly) when the composite is the providing entity of the role
  it was given (`RepositoryInterpreter.java:232-251`).
  - **At entry** the role given is the system's outer role, so the result is empty and the user stops
    silently. Corpus `h15_composite`, `x_pem_subsystem`: no measurements at all.
  - **Through an assembly connector** the role is the connector's provided role, so the inner
    assembly is entered, as a new root context without caller. Its demands then find no allocation
    context (only the composite's assembly is allocated, SS-ACT-9), every job is aborted, and the
    abort cannot find the user context ("User Interpretation Context not found!"), which disposes
    `onResourceDemandRequestAborted`. Corpus `x_pem_replicated_composite`,
    `x_pem_simple_optimisation_composites`.

  Quirk [SS-Q30](./quirks.md#ss-q30).
- **SimuLizar:** the delegation connector's assembly context (ACT-2.1); composites are entered
  (ACT-2.2).

## SS-ACT-2 External calls and role lookup

- **SS-ACT-2.1** `ExternalCallAction` → `SEFFExternalActionCalled(request)` with the input and return
  variable usages, the required role, the signature and a copy of the calling context
  (`SeffInterpreter.java:211-228`).
- **SS-ACT-2.2 The provider is found by the required role alone.** The first `AssemblyConnector` of
  the **system** whose required role id equals the call's role id gives the providing assembly context
  and provided role (`SystemModelRepositoryImpl.java:64-83`), whatever the requiring assembly
  context. Two assemblies of one component therefore share the first one's wiring. Connectors inside
  composites and required delegation connectors are never followed. No connector: empty result, the
  request stops silently. Quirk [SS-Q20](./quirks.md#ss-q20). Probe `ss_role_lookup` (`FrontB`'s
  call reaches `Fast` through `FrontA`'s connector).
- **SS-ACT-2.3 Request.** Push the input frame (inputs evaluated against the caller's top frame, draw
  site `param`), then `CallOverWireRequested(from = caller's assembly, to = provider, signature, user,
  request, payload = input frame)` (`SystemSimulationBehavior.java:162-197`) →
  [SS-ACT-7](#ss-act-7-calls-over-the-wire). When it succeeds (`:221-264`): a `RepositoryInterpreter`
  for the provider with the caller context and the call-over-wire request → `caseBasicComponent`
  (two frames, SS-ACT-1.3) → `SEFFInterpretationProgressed`. A provider that no longer exists (SPD
  scale-in) gives `UserAborted` (`:259-263`).
- **SS-ACT-2.4 Return.** The callee's root `Stop` with a caller (`SeffSimulationBehavior.java:111-113,152-164`):
  pop two frames, then a **reply** `CallOverWireRequested` with the same `from`/`to` and the callee's
  result frame as payload (`[SYD]entities/resource/CallOverWireRequest.java:99-109`). When the reply
  succeeds (`SystemSimulationBehavior.java:225-240`): pop the input frame, evaluate the call's return
  variable usages from the reply's frame into the caller's top frame (draw site `return`),
  `SEFFInterpretationProgressed(caller)`.
- **SS-ACT-2.5 Infrastructure calls.** An `InternalAction`'s infrastructure calls run after its
  demands (`SystemSimulationBehavior.java:343-372`), or at once when it has none
  (`SeffInterpreter.java:351-365`):
  - all `numberOfCalls` are evaluated when the `InfrastructureCallsContextHolder` is built, in list
    order (draw site `infra`), and expanded into a flat list of calls
    (`InfrastructureCallsContextHolder.java:32-50`);
  - each call: `SEFFInfrastructureCalled` → the first `AssemblyInfrastructureConnector` with the
    required role id (`SystemModelRepositoryImpl.java:85-95`) → the provider's SEFF with the infra
    context as caller (`SystemSimulationBehavior.java:322-341`). The call's input variable usages are
    **ignored** (no input frame; quirk [SS-Q36](./quirks.md#ss-q36)). There is no link transfer. The return goes back to the caller
    directly (`SeffSimulationBehavior.java:156-163`) and the next call starts; after the last call
    the internal action's context continues (`:52-58`);
  - the provider SEFF's passed `StartAction` makes the assembly-operation monitor throw
    ([SS-MEAS-2.4](./monitoring.md#ss-meas-2-which-monitors-are-honoured)). Corpus `h18_infrastructure_call`.
  - **SimuLizar:** inputs are evaluated per call (ACT-3.5).

## SS-ACT-3 InternalAction

- **SS-ACT-3.1 All demands start at once.** One `ResourceDemandRequested(ACTIVE)` per
  `ParametricResourceDemand`, in list order, in the same result (`SeffInterpreter.java:334-349`); they
  run concurrently on their resources. The demand values are evaluated by the resource simulation
  ([SS-RES-0](./resources.md#ss-res-0-from-request-to-job)). No frame is pushed for the action.
- **SS-ACT-3.2 Every completion advances the cursor.** Each `ActiveResourceFinished` gives one
  `SEFFInterpretationProgressed` (or the infrastructure-call context, SS-ACT-2.5,
  `SystemSimulationBehavior.java:343-372`), which advances the shared cursor. With `k > 1` demands the
  cursor advances `k` times:
  - the first completion interprets the next action while the other demands still run; the j-th
    completion interprets the j-th action after the internal action, concurrently;
  - the `Stop` can be passed and the request answered while demands still run;
  - a completion after the cursor passed the `Stop` throws (`NullPointerException` at
    `SystemSimulationBehavior.java:349`, or `NoSuchElementException`), which disposes
    `onActiveResourceFinished` ([SS-BUS-7](./core.md#ss-bus-the-event-bus)): no SEFF of any user
    continues after a demand from then on.

  Quirk [SS-Q17](./quirks.md#ss-q17). Probe `ss_two_demands`: the CPU demand (1.0) finishes and the
  next two actions run while the HDD demand (2.0) is still served; the response time is 1.75; the HDD
  completion at 2.0 throws; the second request stops at its first completion.
  **SimuLizar:** demands are served one after the other (ACT-3.1).
- **SS-ACT-3.3** No demands and no infrastructure calls: `SEFFInterpretationProgressed(context)` at
  once (`SeffInterpreter.java:367-370`).
- **SS-ACT-3.4 ResourceCalls are ignored.** Nothing in Slingshot reads `resourceCall__Action`
  (quirk [SS-Q35](./quirks.md#ss-q35); corpus `h23_resource_call`).
  **SimuLizar:** ACT-3.6.

## SS-ACT-4 Start, Stop and the end of a behaviour

- **SS-ACT-4.1** `StartAction` → `SEFFInterpretationProgressed(context)`, `SEFFModelPassedElement(Start)`
  in this order (`SeffInterpreter.java:144-149`). `StopAction` → `SEFFInterpretationFinished(context)`,
  `SEFFModelPassedElement(Stop)` (`:96-101`). These two are the only passed-element events of a SEFF;
  the assembly-operation monitor uses those of root behaviours.
- **SS-ACT-4.2 `SEFFInterpretationFinished`** (`SeffSimulationBehavior.java:82-120`), in this order of tests:
  1. fork holder: continue in the parent once, when all forked behaviours have finished
     (SS-ACT-10);
  2. holder not finished (loop body to repeat): `SEFFInterpretationProgressed(context)`;
  3. child (branch, loop): `SEFFInterpretationProgressed(parent)`;
  4. caller present: return to the caller (SS-ACT-2.4);
  5. otherwise: end of the request (SS-ACT-1.4).
- **SS-ACT-4.3 Unsupported actions** (`EmitEventAction`, `RecoveryAction`, `ResourceCall`-only
  extensions, …) give an empty result: the request stops silently (`SeffInterpreter.java:375-386`).
  So do a missing delegation connector, connector or SEFF (SS-ACT-1.1, 2.2). Quirk
  [SS-Q35](./quirks.md#ss-q35).

## SS-ACT-5 Branches

- **SS-ACT-5.1** The first transition decides the kind (`[CU]TransitionDeterminer.java:141-154`).
  **Probabilistic:** cumulative sums of `branchProbability` in list order, one `Math.random()` (draw
  site `branch:<BranchAction id>`), the first `i` with `sum_last * r < sum_i` (`:124-132,156-172`);
  the algorithm of ACT-6.1, on the `Math.random()` stream. **Guarded:** the conditions are evaluated
  in list order against the user's top frame (`Boolean`, draw site `guard`); the first true wins,
  later conditions are not evaluated (`:90-115`).
- **SS-ACT-5.2** The chosen behaviour runs in a child context (`BranchBehaviorContextHolder`, no
  caller) → `SEFFChildInterpretationStarted` (`SeffInterpreter.java:103-136`); at its `Stop` the parent
  continues (SS-ACT-4.2).
- **SS-ACT-5.3 No true guard throws** `IllegalStateException("No branch transition was active")`
  (`SeffInterpreter.java:115-117`), which disposes `onSeffInterpretationProgressed`. Quirk
  [SS-Q21](./quirks.md#ss-q21). Probe `ss_guard_none`. An empty branch throws as well (`:107-109`).
  **SimuLizar:** the run aborts with an exception (ACT-6.2).
- Probe `ss_branches` (a probabilistic and a guarded `BranchAction`; tape sites `branch:<id>`).

## SS-ACT-6 Loops and collection iterators

- **SS-ACT-6.1 LoopAction.** The count is evaluated once, when the loop is entered (`Integer`, the
  user's top frame, draw site `loop`; `SeffInterpreter.java:151-169`). The body runs in a
  `LoopBehaviorContextHolder` whose progression starts at 1; the body is repeated while
  `progression < count`, and the holder is finished when the body has finished and
  `progression == count` (`[SYD]entities/seff/behaviorcontext/LoopBehaviorContextHolder.java:52-66`).
- **SS-ACT-6.2 A count of 0 runs the body once and then throws.** With `count = 0` the holder is never
  finished: after the body's `Stop` the repeat asks the cursor for another action and
  `SeffBehaviorWrapper.next` throws `NoSuchElementException` (`SeffBehaviorWrapper.java:125-130`),
  disposing `onSeffInterpretationProgressed`. A negative count fails the constructor's precondition (`LoopBehaviorContextHolder.java:40`).
  Quirk [SS-Q19](./quirks.md#ss-q19). Probe `ss_loop_zero` (count drawn from `IntPMF[(0;0.5)(2;0.5)]`).
  **SimuLizar:** a count ≤ 0 skips the body (ACT-7.1).
- **SS-ACT-6.3 CollectionIteratorAction** (`SeffInterpreter.java:264-298`):
  `n := evaluate("<param>.NUMBER_OF_ELEMENTS")` (draw site `collection`); a new frame (parent: the
  top frame) is pushed and the INNER characterisations are evaluated into it **once**
  (`evaluateInner`, draw site `collection`); the body runs `n` times with the same values. The frame
  is **never popped**, so at the end of the request the user's stack has one frame too many and
  `UserRequestFinished` fails its precondition ([SS-WL-4.4](./usage.md#ss-wl-4-usage-actions)), which
  disposes that handler: no user finishes a request after that. Quirk [SS-Q22](./quirks.md#ss-q22).
  Corpus `h10_collection_iterator`, `h27_collection_inner_multi`.
  **SimuLizar:** one frame per iteration, INNER drawn per iteration, popped (ACT-7.3).

## SS-ACT-7 Calls over the wire

`CallOverWireRequested` → `ResourceSimulation.onCallOverWireRequested` (`[RS]ResourceSimulation.java:352-407`):

- **SS-ACT-7.1 Route.** The containers of `from` and `to` are those of the first allocation context of
  each assembly ([SS-ACT-9](#ss-act-9-allocation-first-match)).
  - Either missing: `CallOverWireSucceeded` at once.
  - The same container id: `CallOverWireSucceeded` at once (no transfer).
  - Otherwise the linking resources that connect both containers, in linking-resource table order
    (`[RS]resources/linking/LinkingResourceTable.java:28-32`; in `det` the resource environment's
    list order, in stock Slingshot the hash order of the link ids); if there is none, those that touch
    either container (`:44-48`, meant for containers removed by a scale-in); `findAny()` on that list
    takes the first. None at all: `IllegalArgumentException`, which disposes
    `onCallOverWireRequested` (SS-BUS-7; quirk [SS-Q37](./quirks.md#ss-q37)).
  - The transfer is a `LinkingJob` with demand **Σ of the payload frame's entries whose key ends with
    `BYTESIZE`**, `NumberConverter.toDouble` each, in `SimulatedStackframe.getContents()` order
    (`ResourceSimulation.java:419-426`): the input frame for a request, the callee's result frame for
    a reply → [SS-RES-5](./resources.md#ss-res-5-linking-resources).
- **SS-ACT-7.2 Always simulated.** SimuCom's options "simulate linking resources" and "simulate
  throughput of linking resources" are not read: every call between two containers is a transfer in
  both directions, with the BYTESIZE demand. **SimuLizar:** ACT-11.3 (configuration-dependent;
  `stream.BYTESIZE` in middleware mode).
- **SS-ACT-7.3 Failure and retry.** A failed transfer (link failure probability,
  [SS-RES-5.3](./resources.md#ss-res-5-linking-resources)) gives `JobAborted` → `CallOverWireAborted`
  → pop the input frame and request again, re-evaluating the input usages (new draws)
  (`SystemSimulationBehavior.java:266-273`).
- Probe `ss_link`: request 500 B, reply 250 B over a link with latency 0.01 and throughput 1000;
  transfer times 0.51 and 0.26.

## SS-ACT-8 Acquire and Release

- **SS-ACT-8.1** `AcquireAction` and `ReleaseAction` need a `ParametricResourceDemand`: the first one
  of the action is the amount (`SeffInterpreter.java:230-262`). Without one they throw
  `NoSuchElementException`, which disposes `onSeffInterpretationProgressed`. Quirk
  [SS-Q24](./quirks.md#ss-q24). Corpus `h13_passive_contention`, `h28_triggers_default`.
- **SS-ACT-8.2** Acquire → `ResourceDemandRequested(PASSIVE)`; the SEFF continues when
  `PassiveResourceAcquired` is dispatched (`SeffSimulationBehavior.java:64-71`).
- **SS-ACT-8.3** Release → `PassiveResourceReleased(request)`, `SEFFInterpretationProgressed(context)`
  in this order: the SEFF continues without waiting for the release to be processed.
- Resource semantics: [SS-RES-4](./resources.md#ss-res-4-passive-resources). Probe `ss_passive`.

## SS-ACT-9 Allocation: first match

- **SS-ACT-9.1** The container of an assembly context is that of the **first** allocation context, in
  allocation list order, whose assembly context id matches (`[RS]ResourceSimulation.java:155-157`,
  `[RS]repository/ResourceEnvironmentAccessor.java:42-47`). Probe `ss_allocation_first` (one assembly
  allocated to `Fast` and then to `Slow`: every demand runs on `Fast`).
- **SS-ACT-9.2** No allocation context: the demand's job is aborted at once, the user is aborted
  ([SS-WL-5](./usage.md#ss-wl-5-aborted-users)).
- **SimuLizar:** the last matching allocation context wins (ACT-2.8); nested containers are not
  simulated (REF-8). Slingshot creates resources for every container referenced by an allocation
  context, nested or not (corpus `h31_nested_container` runs).

## SS-ACT-10 ForkAction

- **SS-ACT-10.1** Only **synchronous** forked behaviours are interpreted; asynchronous ones are
  ignored. A fork without a `SynchronisationPoint` throws `IllegalStateException` (`SeffInterpreter.java:171-201`).
  Probe `ss_async_fork` (the asynchronous 5.0 s demand never starts); corpus `h12_fork_async` (no
  synchronisation point: throws).
- **SS-ACT-10.2** Each synchronous behaviour starts as a child context (`SEFFChildInterpretationStarted`,
  in list order); all share **one** `ForkBehaviorContextHolder` and the user's single stack. The
  holder's `getCurrentProcessedBehavior()` moves to the next unfinished behaviour on **every** call
  (`ForkBehaviorContextHolder.java:48-68`), so a child's progress advances whichever behaviour is
  current. With two or more behaviours this misroutes and throws `NoSuchElementException` when the
  rotation finds no unfinished behaviour (corpus `h11_fork_sync`, `h26_fork_double_resume`,
  `x_pem_fork`). Quirk [SS-Q23](./quirks.md#ss-q23).
- **SS-ACT-10.3** When all behaviours have finished, the parent continues once
  (`SeffSimulationBehavior.java:91-104`). Result parameters of forked behaviours are not processed.

## SS-ACT-11 SetVariableAction and frames

- **SS-ACT-11.1** `SetVariableAction` evaluates its local variable usages against the top frame into
  the current **result frame** (draw site `setvar`) and continues (`SeffInterpreter.java:300-309`).
- **SS-ACT-11.2 Frames per user.** Each user has one SimuCom stack. Per call: input frame (SS-ACT-1.3,
  2.3), component frame, assembly frame; popped at the return (two) and at the response (the input
  frame). Branches and loops push no frames; internal actions push none
  (**SimuLizar:** one per internal action, ACT-3.1). Evaluation and frame lookup are SimuCom's
  (ACT-5.1 to ACT-5.7).

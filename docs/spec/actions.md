# RDSEFF, composition and parameter semantics (SimuLizar 5.2.2)

Rules `ACT-*`. Source abbreviations, time and process model (`SIM-*`): [simulation core](./simulation.md).
Usage model: [workloads](./workloads.md). SimOxide implements these rules as stated.

All steps run synchronously inside the current process unless they explicitly wait (hold, resource demand,
passive acquire, fork join). "Evaluate" = `StackContext.evaluateStatic` (SIM-8.2); "draw" = the evaluation may
consume random numbers ([StoEx](./stoex.md), [random numbers](./random.md)).

## ACT-1 Dispatch and behaviours

- **ACT-1.1** SEFF elements are interpreted by an `ExplicitDispatchComposedSwitch` built per call from the
  multibound set of switch factories (`[SL]interpreter/impl/ExtensibleComposedRDSeffSwitchFactory.java:25-35`):
  `RDSeffSwitch` (package `seff`), `RDSeffPerformanceSwitch` (`seff_performance`), `NOPReliabilityInterpreter`
  (`seff_reliability` and `reliability`, only when failures are off: ignores reliability elements, ACT-1.3).
  EMF's `ComposedSwitch` delegates by EPackage, and the packages are disjoint, so the HashSet order (ND-1)
  does not matter.
- **ACT-1.2 ResourceDemandingBehaviour** (SEFF, loop body, branch behaviour, forked behaviour;
  `[SL]interpreter/RDSeffSwitch.java:122-186`): identical to WL-4.2 with `StartAction`/`StopAction`:
  first `StartAction` in `steps_Behaviour` order → `BEGIN`,`END`; then along `successor_AbstractAction`
  until the `StopAction`: `BEGIN(a)`; `doSwitch(a)`; `END(a)`. `StopAction` fires nothing. Stack depth must be
  unchanged at the end.
- **ACT-1.3** Unsupported actions (`caseAbstractAction`, `:199-203`) throw → run aborts (SIM-6.6): e.g.
  `EmitEventAction`. A `RecoveryAction` is handled by `NOPReliabilityInterpreter`
  (`[SL]interpreter/legacy/NOPReliabilityInterpreter.java:57-66`): it interprets the action's primary behaviour
  (ACT-1.2) and ignores the alternatives, since without failure simulation the primary behaviour cannot fail.
  `DelegatingExternalCallAction` is supported (`:242-248`) but only occurs in SimuCom's middleware completions,
  which SimuLizar does not run.

## ACT-2 Composition: from a call to a SEFF

`[SL]interpreter/RepositoryComponentSwitch.java`, `[SL]interpreter/ComposedStructureInnerSwitch.java`.
The context keeps an **assembly-context stack**; index 0 is always the generated *system* AC.
FQ component id = ids of stack[1..] joined with `"::"` (`[SL]interpreter/InterpreterDefaultContext.java:112-123`,
`[SL]runtimestate/FQComponentID.java:128-131`).

- **ACT-2.1 Provided role** (`caseProvidedRole`, `:165-182`): push the AC (for the system entry: a generated AC
  with the id of the static `SYSTEM_ASSEMBLY_CONTEXT`); fire `AssemblyProvidedOperation BEGIN` (model element
  = the switch's AC); `doSwitch(providingEntity)`; pop; fire `... END`.
- **ACT-2.2 Composed structure** (System, CompositeComponent, SubSystem; `:134-159,261-266`): register a
  composite instance for the FQ id if new (no state); find the **first** `ProvidedDelegationConnector` in
  `connectors__ComposedStructure` order whose outer role equals the provided role (none → exception); recurse
  with ACT-2.1 on (inner AC, inner role). **Composite component parameters are ignored** (no frames pushed).
- **ACT-2.3 Basic component** (`caseBasicComponent`, `:93-131`):
  1. push frame `Fc` (parent = current frame) holding the component's default parameter usages
     (`componentParameterUsage_ImplementationComponentType`), evaluated against the current frame (ACT-5.2);
  2. push frame `Fa` (parent = `Fc`) holding the AC's `configParameterUsages__AssemblyContext`, evaluated
     against `Fc` (ACT-5.2);
  3. if no component instance exists for the FQ id: create it (passive resources, ACT-8.1);
  4. select the SEFFs whose `describedService__SEFF.id` equals the signature id; exactly one
     `ResourceDemandingSEFF` required, else exception; interpret it with a new composed switch (ACT-1.2);
  5. pop `Fa`, pop `Fc`.

  Lookup order inside the SEFF is therefore `Fa` → `Fc` → call input frame → (none): assembly config
  overrides component defaults, which override **same-named input parameters**. Component parameters are
  re-evaluated (and re-drawn) on **every** call.
- **ACT-2.4 Required role** (`ComposedStructureInnerSwitch.caseAssemblyContext`, `:184-187,196-266`): search the
  connectors of the AC's **parent structure** in list order for the first
  `AssemblyConnector(requiring AC == ctx && requiredRole == role)` or
  `RequiredDelegationConnector(AC == ctx && innerRole == role)` (or the infrastructure variants). No parent
  structure (the generated system AC) → "Required delegation of the system cannot be simulated" (abort): a
  system-level required role is not simulatable. No match → exception.
- **ACT-2.5 RequiredDelegationConnector** (`:156-167`): pop the enclosing AC from the stack, resolve the
  **outer** required role from that AC (ACT-2.4), push it back afterwards.
- **ACT-2.6 AssemblyConnector** (`:96-115`):
  ```
  src := allocation(FQ(stack[1..]) + requiringAC); dst := allocation(FQ(stack[1..]) + providingAC)
  transmit(src, dst, payload = current frame)              # ACT-11, request
  ProvidedRole switch on (providingAC, providedRole)        # ACT-2.1
  transmit(dst, src, payload = current result frame)        # ACT-11, response
  ```
  (The caller's AC has already been popped by ACT-4, so `stack[1..]` are the enclosing composites.)
- **ACT-2.7 AssemblyInfrastructureConnector** (`:145-153`): like ACT-2.6 but **without** transmissions.
- **ACT-2.8 Allocation lookup** (`[SL]modelobserver/AllocationLookupSyncer.java:119-160`,
  `[SL]runtimestate/AssemblyAllocationManager.java:33-39`): each `AllocationContext` (list order) maps the FQ id
  of its AC and, recursively, of every nested AC of a composite to its container; later entries overwrite
  earlier ones for the same FQ id. A missing allocation → NPE/exception.

## ACT-3 InternalAction and resource demands

- **ACT-3.1** `caseInternalAction` (`[SL]interpreter/RDSeffSwitch.java:209-224`): push an empty frame (parent =
  current), then in this order, each list in model order:
  1. `resourceDemand_Action` (ParametricResourceDemand, ACT-3.2), each one **completed (waited) before the next
     starts**;
  2. `infrastructureCall__Action` (ACT-3.5);
  3. `internalFailureOccurrenceDescriptions` (no-op without failures);
  4. `resourceCall__Action` (ACT-3.6);

  then pop the frame. The action is a sequence of waits; nothing runs in parallel.
- **ACT-3.2 ParametricResourceDemand** (`[SL]interpreter/RDSeffPerformanceSwitch.java:71-100`):
  ```
  value := evaluate(demandSpec, Double.class, currentFrame)          # draw(s)
  rc := allocation(FQ(current stack))                                 # container of the current component
  (pre-interpretation behaviours: none in the supported scope)
  rc.loadActiveResource(thread, requiredResourceType.id, value)       # ACT-3.3, resourceServiceId = 1
  ```
- **ACT-3.3 Container lookup** (`[SCC]resources/AbstractSimulatedResourceContainer.java:59-66,183-191`,
  `[SCC]resources/SimulatedResourceContainer.java:163-213`): the container holds **one resource per resource-type
  id** (`HashMap`; a second spec with the same type replaces the first, `SimulatedResourceContainer.java:130`).
  Missing type → retry in the parent container (nested containers), none → exception.
  **Nested containers are never simulated in SimuLizar 5.2.2** (checked with the reference): the
  `ResourceEnvironmentSyncer` creates simulated containers for `resourceContainer_ResourceEnvironment`
  (top level) only (`[SL]modelobserver/ResourceEnvironmentSyncer.java:95-97,203-218`), so nested
  containers, their resources and monitors do not exist, and a component allocated to one aborts at its first
  resource demand (`NullPointerException ... getSimulatedEntity(String) is null`; SimOxide: "resource
  container ... is not simulated"; [reference bugs](../correctness/reference-bugs.md), REF-8,
  `corpus/h31_nested_container`, `corpus-fuzz/l_ref_nested_allocation`). Links may still name nested containers
  (routing uses the ids).
- **ACT-3.4 consumeResource** (`[SCC]resources/AbstractScheduledResource.java:138-173`):
  ```
  (availability check: no-op without failures)
  scheduler.registerProcess(thread)                                    # no-op for PS/FCFS/Delay
  d := calculateDemand(value)          # processing resource: value / evaluate(processingRate, Double.class)
                                       #   (empty frame; evaluated on EVERY demand → draws; ScheduledResource.java:123-125)
  add := 0.0; for m in demandModifiers (list order): (d, a) := m(d); add += a      # none for CPUs/HDDs
  d := d + add
  if d <= 0: return                    # no event, no wait, no demand measurement
  fireDemand(d)                        # demand listeners (MEAS)
  scheduler.process(thread, serviceId, {}, d)   # SIM-4.7; returns immediately if !isRunning()
  ```
  Double order: `value / rate`, then `+ add`. Zero or negative demand is silently skipped.
  - HDD resources (`[SCC]resources/HDDResource.java:39-60`) first divide the demand by the read processing rate
    (resourceServiceId 1, which every parametric demand uses, ACT-3.2) or the write processing rate (id 2),
    each evaluated on every demand; any other id throws ("HDD Resource called without explicit read/write
    call"). The result then goes through `consumeResource` above, i.e. it is divided by the processing rate as
    well.
- **ACT-3.5 InfrastructureCall** (`RDSeffPerformanceSwitch.java:137-156`): `n := evaluate(numberOfCalls,
  Integer.class, currentFrame)` once; repeat n times: push input frame (ACT-5.2, no parent) from
  `inputVariableUsages__CallAction`; pop the current AC; resolve the infrastructure required role (ACT-2.4/2.7);
  push the AC back; pop the input frame. **No result frame is pushed and no output parameters are processed.**
- **ACT-3.6 ResourceCall** (`RDSeffPerformanceSwitch.java:103-135`): resource type = the **last** type in
  `availableResourceTypes_ResourceRepository` order that provides the called resource interface (the inner loop
  `break` only leaves the role loop); `demand := toDouble(evaluate(numberOfCalls, Double.class, currentFrame))`;
  `rc.loadActiveResource(thread, signature.resourceServiceId, type.id, demand)` → ACT-3.4.

## ACT-4 ExternalCallAction

`caseExternalCallAction` (`[SL]interpreter/RDSeffSwitch.java:238-269`). Sequence:

1. (`BEGIN(externalCall)` was fired by ACT-1.2 → external-call response time starts, MEAS)
2. push input frame `Fin` (**no parent**) with `inputVariableUsages__CallAction` evaluated against the caller's
   current frame (ACT-5.2; draws);
3. pop the caller's AC from the AC stack; push a new empty result frame;
4. resolve the required role from the caller's AC (ACT-2.4 → ACT-2.5/2.6 → ACT-2.1 ...);
5. push the caller's AC back; pop `Fin`;
6. pop the result frame and evaluate `returnVariableUsage__CallReturnAction` against it, writing into the
   caller's current frame (ACT-5.2; draws);
7. (`END(externalCall)` fired by ACT-1.2.)

The callee therefore sees only `Fin` + component frames (ACT-2.3), never the caller's variables.

## ACT-5 Parameters, variable characterisations, stack frames

- **ACT-5.1 Frames** (`[SCV]stackframe/SimulatedStackframe.java`): a frame is a `HashMap<String,Object>` plus an
  optional parent; lookup = own map, then parent chain (`getValue`, `:79-87`). The stack is a `java.util.Stack`
  of frames; evaluations use the **top** frame (`currentStackFrame`).
- **ACT-5.2 addParameterToStackFrame(contextFrame, usages, target)**
  (`[SL]utils/SimulatedStackHelper.java:44-82`): for each `VariableUsage` in list order, for each
  `VariableCharacterisation` in list order:
  ```
  id := serialise(usage.namedReference) + "." + characterisation.type.literal
        # literal ∈ VALUE | BYTESIZE | NUMBER_OF_ELEMENTS | TYPE | STRUCTURE; serialisation: stoex.md
  if the reference contains a component named "INNER":       # isInnerReference, :91-106
      target[id] := EvaluationProxy(spec, contextFrame.copyFrame())      # lazy, no draw now
  else:
      target[id] := evaluate(spec, contextFrame)              # raw result object (Integer/Double/Boolean/String/…), draws now
  ```
  `put` overwrites an existing id without changing its HashMap position.
- **ACT-5.3 createAndPushNewStackFrame(stack, usages[, parent])** (`SimulatedStackHelper.java:116-145`): new frame
  (with parent if given), fill it via ACT-5.2 with `contextFrame = stack.top` (or `null` if the stack is empty),
  then push. Used by ACT-2.3 (with parent), ACT-3.5, ACT-4, WL-4.6 (without parent).
- **ACT-5.4 EvaluationProxy access** (`[SCV]stoexvisitor/PCMStoExEvaluationVisitor.java:101-116`): reading a
  variable whose value is a proxy evaluates `proxy.spec` against `proxy.frame` **at every access** (fresh draws
  each time). `NumberConverter.toDouble(proxy)` does the same (`[SCV]converter/NumberConverter.java`).
- **ACT-5.5 copyFrame** (`SimulatedStackframe.java:94-103`): deep copy of the whole parent chain (values shared,
  maps new). Used for proxies (ACT-5.2), fork children and per-run contexts (ACT-9.2, WL-1.2).
- **ACT-5.6 getContents** (`SimulatedStackframe.java:112-128`): own entries in HashMap iteration order, then the
  parent's entries whose key was not seen yet, recursively. Its order defines draw order in ACT-7.3 and summation
  order in ACT-11.3.
- **ACT-5.7 Java HashMap iteration order (emulated where ACT-5.6 is used).** For `java.util.HashMap<String,_>`
  with default settings (JDK 17/21): `h = s.hashCode()` (UTF-16, `31*h + c`, wrapping i32);
  `spread = h ^ (h >>> 16)`; capacity `C` = 16 initially, doubled whenever an insert of a **new** key makes
  `size > 0.75*C`; iteration = buckets `spread & (C-1)` ascending, within a bucket in first-insertion order
  (resize splits keep relative order). Since frames never remove keys, iteration order is a pure function of
  the key set, its first-insertion order and the final size; `copyFrame` preserves it. SimOxide emulates this
  order (`crates/simoxide-sim/src/javahash.rs`; `corpus/h27_collection_inner_multi`). Treeified bins (more than
  8 keys in one bucket at capacity ≥ 64) are not emulated.
- **ACT-5.8 SetVariableAction** (`[SL]interpreter/RDSeffSwitch.java:379-390`): ACT-5.2 with
  `contextFrame = current frame`, `target = current result frame` (top of the context's result-frame stack,
  pushed by the caller: ACT-4 step 3 or WL-4.6 step 4). The value is **not** visible to later actions of the
  same SEFF; it only becomes the call's return data. In a forked behaviour there is no result frame (NPE → abort).

## ACT-6 BranchAction

`caseBranchAction` (`[SL]interpreter/RDSeffSwitch.java:275-309`), `[SL]utils/TransitionDeterminer.java:148-186`.
Empty branch → exception. The **first** transition's type decides the mode:
- **ACT-6.1 Probabilistic** (`ProbabilisticBranchTransition`): exactly WL-4.5 on `branchProbability` (double
  attribute): one `random()` draw, cumulative sums in list order, first `i` with `cum[last]*r < cum[i]`.
- **ACT-6.2 Guarded** (`GuardedBranchTransition`, `:108-138`): evaluate `branchCondition` (`Boolean.class`, current
  frame) in list order; the first `true` wins; later conditions are **not** evaluated (no draws). None true →
  exception (abort).
- **ACT-6.3** The chosen `branchBehaviour` is interpreted via ACT-1.2; no extra frame.

## ACT-7 LoopAction and CollectionIteratorAction

- **ACT-7.1 LoopAction** (`RDSeffSwitch.java:359-373,508-521`): `n := evaluate(iterationCount, Integer.class,
  current frame)` **once per loop entry**; body interpreted n times (none if n ≤ 0); no extra frame.
- **ACT-7.2 CollectionIteratorAction** (`:315-317,532-597`): `n := evaluate("<param>.NUMBER_OF_ELEMENTS",
  Integer.class, current frame)` where `<param>` = `parameter_CollectionIteratorAction.parameterName`.
- **ACT-7.3** Per iteration: push an empty frame `Fi` (parent = current); then `evaluateInner(Fi, "<param>.")`
  (`[SCV]StackContext.java:321-332`): iterate `Fi.getContents()` (ACT-5.6: `Fi` is empty, so the parent chain in
  HashMap order); for each entry whose key starts with `"<param>."` and whose value is an `EvaluationProxy`:
  `Fi[key] := evaluate(proxy.spec, proxy.frame)` (draws, in that order). Interpret the body; the top frame must
  be `Fi` again; pop `Fi`. So INNER characterisations are re-drawn per iteration; non-proxy values are visible
  unchanged through the parent chain.

## ACT-8 Passive resources (Acquire/Release)

- **ACT-8.1 Instances** (`[SL]runtimestate/SimulatedBasicComponentInstance.java:34-70`): one
  `SimSimpleFairPassiveResource` per (FQ component id, passive resource), created at the component's **first
  call** (ACT-2.3 step 3) in `passiveResource_BasicComponent` order. Capacity:
  `(long) evaluate(capacitySpec, Long.class, current frame)` where the current frame is `Fa` (component and
  AC parameters visible) — evaluated **once per simulation** (draws at that moment). Monitors attached here
  (MEAS).
- **ACT-8.2 Acquire** (`RDSeffSwitch.java:400-417` → `SimulatedBasicComponentInstance.java:81-86` →
  `[SCC]resources/SimSimpleFairPassiveResource.java:79-83,108-133`), always 1 unit:
  ```
  if !isRunning(): return
  fireRequest(p, 1)
  if (queue.empty or queue.head.process == p) and 1 <= available:
      available -= 1; fireAcquire(p, 1)              # no event, no yield
  else:
      queue.addLast(Waiting(p, 1)); (timeout: only with failures, ignored); p.passivate()
  ```
  The passive resource must belong to the current basic component instance, else exception.
- **ACT-8.3 Release** (`:174-199`):
  ```
  if !isRunning(): return
  available += 1; fireRelease(p, 1)
  while queue nonempty and canProceed(head):          # head always passes the head test
      available -= 1; fireAcquire(head.p, 1); queue.removeFirst(); head.p.activate()   # Resume at now, FIFO
  ```
  The releaser continues without yielding. The woken process continues after its `passivate()` in ACT-8.2 (it
  does not re-check). Strict FIFO; no capacity check on release (over-release grows `available`).
- **ACT-8.4** `AcquireAction.timeout/timeoutValue` are ignored when failures are off.

## ACT-9 ForkAction

`caseForkAction` (`[SL]interpreter/RDSeffSwitch.java:323-353,458-498`), `[SL]interpreter/impl/ForkedBehaviorProcessFactoryImpl.java:28-45`,
`[SC]fork/ForkedBehaviourProcess.java:30-99`, `[SC]fork/ForkExecutor.java:35-63`.

- **ACT-9.1** Create child processes (SIM-4.1), in this order: every `asynchronousForkedBehaviours_ForkAction`
  (list order, async), then, if a `SynchronisationPoint` exists, its `synchronousForkedBehaviours` (list order,
  sync). No child runs yet.
- **ACT-9.2** Each child gets its own interpreter context created at construction
  (`InterpreterDefaultContext.createChildContext(parentContext, child)`, `[SL]interpreter/InterpreterDefaultContext.java:142-166`):
  stack = one frame = `copyFrame()` of the parent's **current** frame (ACT-5.5); AC stack copied; **empty
  result-frame stack**; same evaluation mode. The child inherits the parent's request context (as parent id)
  and session id.
- **ACT-9.3** `ForkExecutor.run()`: `child.scheduleAt(0)` for all children in creation order → `Resume(child_i)` at
  now; then `while any sync child has !isTerminated(): parent.passivate()`. With no sync children the parent
  continues immediately (children start later, at their `Resume` notes).
- **ACT-9.4** Child `internalLifeCycle`: interpret the `ForkedBehaviour` (ACT-1.2); set its `isTerminated` flag;
  if sync, the parent is not terminated and `isRunning()`: `parent.scheduleAt(0)` → `Resume(parent)` at now.
  Async children are never joined.
- **ACT-9.5 Double resume (SIM-4.4a).** Every finishing sync child schedules a `Resume(parent)`.
  If two sync children finish before the parent's first `Resume` is processed (e.g. children without waits),
  the parent has two pending resumes: the first ends the join, the second wakes the parent from its next wait
  prematurely. SimOxide reproduces this (`corpus/h26_fork_double_resume`), including the aborts it can cause
  (SIM-4.4a; REF-2 in [reference bugs](../correctness/reference-bugs.md)).
- **ACT-9.6** SynchronisationPoint output parameter usages / fork result variables are **not** processed by
  SimuLizar 5.2.2 (nothing reads them).

## ACT-10 Summary: evaluation points inside a SEFF call

| Where | What | When |
|---|---|---|
| ExternalCall | input characterisations | before the call, list order |
| BasicComponent entry | component defaults, then AC config params | every call |
| first call of a component instance | passive resource capacities | once |
| ParametricResourceDemand | demand spec, then processing rate | per demand, before waiting |
| InfrastructureCall | number of calls, then per call its inputs | on entry |
| ResourceCall | number of calls, then processing rate | per call |
| Branch | 1 × `random()` or conditions until first true | on entry |
| Loop / CollectionIterator | count; per iteration INNER proxies (HashMap order) | on entry / per iteration |
| Assembly connector | payload proxies (ACT-11.3), throughput, latency | before request / after response |
| ExternalCall return | return characterisations against result frame | after the call |
| SetVariable | characterisations | at the action |

## ACT-11 Linking resources (network)

- **ACT-11.1 Simulated by default: yes** — every `AssemblyConnector` call whose two ACs are allocated to
  **different** containers performs a request and a response transmission (ACT-2.6) through
  `DefaultSimuLizarTransmissionInterpreter` (`[SL]interpreter/linking/impl/DefaultSimuLizarTransmissionInterpreter.java:66-91`).
  System entry calls, delegation hops and infrastructure connectors never transmit.
- **ACT-11.2 Route** (`[SL]interpreter/linking/impl/ResourceEnvironmentObservingLegacyRouter.java:80-94`): same
  container id → no transmission. Else the **first** linking resource (resource-environment list order) whose
  connected containers contain both → that one link. **No such link → RuntimeException (run aborts)**, for every
  payload mode. A model must therefore connect all container pairs that communicate.
- **ACT-11.3 Payload demand** (`[SL]di/modules/scoped/runtime/LinkingResourceSimulationModule.java:31-42`), chosen from
  the workflow configuration, which copies it from `SimuComConfig` (`SimuComWorkflowConfiguration.setSimuComConfiguration`,
  `Palladio-Analyzer-SimuCom/bundles/de.uka.ipd.sdq.codegen.simucontroller.core/src/.../runconfig/SimuComWorkflowConfiguration.java:42-49`):
  - `simulateLinkingResources = true` (UI default false): `MiddlewareCompletionAwareDemandCalculator`:
    `toDouble(evaluateStatic("stream.BYTESIZE", Double.class, payload))` — the SimuCom middleware completion
    would add `stream`; SimuLizar does not run it, so the model itself must pass `stream.BYTESIZE` (input
    variable usage of every assembly-connector call, and a SetVariable of the callee for the reply). Evaluated
    for **every** assembly-connector call, also within one container (before the route check); missing →
    "Architecture specification incomplete. Stackframe is missing id stream.BYTESIZE" (abort; REF-10 in
    [reference bugs](../correctness/reference-bugs.md); `corpus/h30_middleware_stream`,
    `corpus-fuzz/l_ref_stream_bytesize_missing`);
  - else `simulateThroughputOfLinkingResources = true` (**UI default true**, `Palladio-Analyzer-SimuCom/bundles/de.uka.ipd.sdq.codegen.simucontroller/src/de/uka/ipd/sdq/codegen/simucontroller/runconfig/FeatureOptionsTab.java:60-61`; also
    the standalone runner): `StackFrameBytesizeAccumulatingDemandCalculator` (`.../StackFrameBytesizeAccumulatingDemandCalculator.java:27-43`):
    `demand = 0.0; for (key, v) in payload.getContents() (ACT-5.6 order): if key.endsWith("BYTESIZE"): demand += toDouble(v)`
    (`toDouble` accepts Integer/Double/EvaluationProxy (evaluated, draws), else exception). Request payload =
    `Fin`, response payload = the result frame;
  - else `NoDemandCalculator`: 0.0.

  Both flags are part of the run configuration and recorded in `run.json` ([formats](../guide/formats.md) §5);
  the corpus sets them explicitly to the UI defaults unless a model tests another value.
- **ACT-11.4 Transmission on the link** (`SimulatedLinkingResourceContainerTransmissionStrategy.java:33-37` →
  `[SCC]resources/AbstractScheduledResource.java:138-173`, `[SCC]resources/SimulatedLinkingResource.java:40-105`,
  `[SCC]resources/DemandModifyingBehavior.java:41-55`), resourceServiceId 0, scheduler **FCFS**, 1 instance:
  ```
  tp := toDouble(evaluate(throughputSpec))            # empty frame, every transmission; tp <= 0 → exception
  d := payload / tp
  d := d / toDouble(evaluate("1.0"))                  # latency modifier: scaling "1.0" evaluated first
  add := 0.0 + toDouble(evaluate(latencySpec))        # then latency (draws)
  d := d + add
  if d <= 0: no transmission event
  else fireDemand(d); FCFS.process(thread, d)         # waits, SIM-4.7
  ```
  So even a zero payload costs `latency` on the FCFS link queue, in each direction.
- **ACT-11.5** Linking resources get no monitors from `ResourceEnvironmentSyncer` (`addActiveResourceWithoutCalculators`);
  see [measurements](./measurements.md) (MEAS-8.1) for what is measured on them.

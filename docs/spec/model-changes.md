# Model changes at run time

SimuLizar changes the models while a simulation runs: reconfiguration rules (QVTo) edit the PCM
models in place, and *syncers* observe the EMF notifications and update the simulation entities.
The interpreter reads the system, the repository and the allocation lookup at every call, so
those changes need no syncer. These rules say what each change does to a running simulation,
including the gaps of the syncers, which SimOxide reproduces.

Sources (SimuLizar `releases/5.2.2`, `bundles/org.palladiosimulator.simulizar/src/org/palladiosimulator/simulizar/`):
`modelobserver/ResourceEnvironmentSyncer.java` (RES), `modelobserver/AllocationLookupSyncer.java`
(ALS), `interpreter/linking/impl/ResourceEnvironmentObservingLegacyRouter.java` (ROUTER),
`modelobserver/UsageModelSyncer.java` (UMS), `modelobserver/AbstractModelObserver.java`,
`interpreter/RDSeffPerformanceSwitch.java`, `interpreter/ComposedStructureInnerSwitch.java`.

In SimOxide a change set is applied between two events through `Simulation::apply_changes`
(the [library guide](../guide/library.md#changing-the-model-of-a-run)); the syncer is *rebuild
and diff* (`simoxide-sim/src/change.rs`): the typed model is rebuilt from the changed graph with
every element keeping its typed id, recompiled from the old frame keys, StoEx programs and
measurement series, and merged into the running compiled model so that every index the run
state holds keeps its meaning.

## MC-1 Resource environment

- **MC-1.1** A container added to the resource environment gets its active resources
  (`RES.add` → `createSimulatedResourceContainer`, RES:109-127, 203-217), idle. A container whose id is
  registered already is not created (RES:213-215); SimOxide refuses such a change.
- **MC-1.2** A processing resource specification added to a registered container gets an active
  resource; if the container has one of its resource type already, that resource's processing
  rate is set from the new specification (`createSimulatedActiveResource`, RES:248-275).
- **MC-1.3** A changed processing rate (the specification of the `PCMRandomVariable`, or the
  variable replaced) is set on the resource (`syncProcessingResource`, RES:321-327, features
  RES:63-70). The rate is evaluated per demand, so it applies to demands issued after the
  change; jobs already queued keep their demand. Link throughput and latency likewise
  (`syncLinkingResource`, RES:283-314).
- **MC-1.4** Every other change of a specification is ignored: number of replicas, scheduling
  policy, resource type, HDD read and write rates (RES.set handles only MC-1.3).
- **MC-1.5** Removals are ignored (RES does not override `remove`): a container, processing
  resource or link that leaves the environment keeps its simulated resources; its jobs complete,
  and allocation lookups that lead to it still work. The resource registry keeps it, so it
  still takes part in the final deactivation (SIM-6.5).
- **MC-1.6** New resources join the resource registry in insertion order: new types at the end
  of their container's map, new containers and links at the end. The final deactivation follows
  the Java `HashMap` order of the whole registry.

## MC-2 Routing

- **MC-2.1** Links added to the environment are appended to the router's list; removed links
  leave it (ROUTER:118-140). Transmissions in progress on a removed link complete.
- **MC-2.2** Containers connected to a link are routed over it from then on (ROUTER:143-167, 180-183).
  Disconnecting is a no-op: `doRemoveConnectedResourceContainer` looks the link's set up by the
  container's id (ROUTER:185-188), so the connection stays, also for a container that was
  connected and disconnected in one change set.
- **MC-2.3** A `move` or `set` of the link list or of a link's connected containers throws
  `UnsupportedOperationException` (ROUTER:132, 159): the simulation aborts.
- **MC-2.4** The route between two containers is the first link in the router's order that
  connects both (ROUTER:81-94, as ACT-11).

## MC-3 Allocation

- **MC-3.1** An allocation context added to the allocation adds the lookup entries of its
  assembly context's path and of every path nested below it; later entries overwrite earlier
  ones (`doAddAllocationContext`, ALS:138-160, 263-269). An added context without assembly context adds nothing.
- **MC-3.2** Changes of an existing allocation context's container or assembly context are
  ignored: ALS reacts to `SET` only on `Allocation.allocationContexts`.
- **MC-3.3** Removing an allocation context aborts with a `NullPointerException`
  (`processNotification` passes the notification's *new* value, `null`, to
  `doAddAllocationContext`, ALS:244-258, 292-296). `UNSET`, `MOVE` and any other event type inside the
  allocation abort with `UnsupportedOperationException` (ALS:91-110). `EcoreUtil.delete` of a
  container or assembly context that an allocation context refers to unsets that reference, so
  it aborts too.
- **MC-3.4** The lookup happens at every demand (RDSeffPerformanceSwitch:80, 130) and at every
  call over an assembly connector (ComposedStructureInnerSwitch:117-121): requests in progress
  use the new entries from their next demand on.

## MC-4 System and repository

- **MC-4.1** The interpreter reads the system and the repository at every call: calls made after
  a change resolve over the new connectors and assembly contexts; a call in progress continues
  where it was resolved to.
- **MC-4.2** A changed action takes effect at its next execution (branch probabilities, guards,
  loop counts, demands, parameter characterisations).
- **MC-4.3** *SimOxide*: behaviours are compiled to instruction streams. After a change of a
  behaviour, new executions run its new instructions; an execution in progress finishes the
  instructions it started with. SimuLizar would follow a changed successor chain at once. A
  structural change of an action (its kind, or the number of infrastructure or resource calls of
  an internal action) is not supported yet.

## MC-5 Usage model

- **MC-5.1** A changed think time or inter-arrival time applies from the next draw
  (UMS:72-106, 113-139).
- **MC-5.2** *SimOxide*: population changes, new and removed usage scenarios (UMS:46-69, 108-111, 141-153)
  are not supported yet.

## MC-6 Monitors

- **MC-6.1** *SimOxide*: changes that create or remove measurement series, sliding windows or
  runtime-measurement recorders are not supported yet. New resources get no monitors, as in
  SimuLizar when no monitor of the monitor repository refers to their specification
  (`attachMonitors`, RES).

## MC-7 Reconfiguration rules (QVTo)

Sources: SimuLizar `releases/5.2.2`, bundle `org.palladiosimulator.simulizar.reconfiguration.qvto`
(`QVTOReconfigurator`, `QVTOExecutor`, `AbstractQVTOExecutor`, `QVTOReconfigurationLoader`,
`util/ModelTransformationCache`, `util/QVToModelCache`), `reconfiguration/ReconfigurationProcess.java`;
QVTo 3.10.7 as shipped with the 5.2.2 product. When the rules run: [measurements](./measurements.md)
MEAS-7.2; what they see in the runtime measurement model: MEAS-7.5.

- **MC-7.1 Rules.** Every `.qvto` file directly in the rules folder (`reconfigurationRulesFolder`).
  SimuLizar keeps them in a `HashMap<URI, …>` and runs them in its iteration order, which depends on
  the URIs, i.e. on where the folder lies. SimOxide, and the patched reference (patch P9), run them in
  file-name order. Henshin and Story Diagram rules are not run by the 5.2.2 runtime (their engines
  contribute to extension points its components no longer read), nor by SimOxide.
- **MC-7.2 Binding** (`AbstractQVTOExecutor.setupModelExtents`, `QVToModelCache`): each `in`/`inout`
  parameter gets the first root of every model of its metamodel; for the runtime measurement model,
  its measurements. A parameter without a model throws `IllegalStateException` in the
  reconfiguration process: the run aborts (SimOxide: the run fails with the error). `out`
  parameters get empty extents. Changes reach the models in place, whatever the direction.
- **MC-7.3 Result.** All rules run, one after the other (`result &= executeTransformation(...)`: no
  short circuit); each sees the changes of the rules before it. A rule is *applied* when QVTo reports
  `OK` or `INFO`. QVTo 3.10.7 reports `OK` for every run that completes, also when a non-fatal
  `assert` (`warning`, `error`) failed; a run that terminates (`assert fatal`, an exception) reports
  `ERROR`: "not applied". What it changed before it terminated stays (SimuLizar transforms the
  models in place). The reconfiguration succeeds when every rule applied (MEAS-7.3). *SimOxide*:
  qvto-rs follows QVTo 3.11, which raises a completed run's severity to that of its failed non-fatal
  assertions; SimOxide's host therefore counts a rule as applied when it completed (diagnostic code
  0), which is SimuLizar 5.2.2's result.
- **MC-7.4 Changes.** SimuLizar's syncers react to every notification while the rule runs (MC-1 to
  MC-6). *SimOxide* applies each rule's changes when the rule has finished, before the next rule
  runs (`Simulation::apply_changes`), with the same effect on the simulation for the change kinds
  of MC-1 to MC-5. A change that SimuLizar's syncers abort on (MC-2.3, MC-3.3) ends the run with an
  error, as in SimuLizar. A change SimOxide does not support yet ends the run with an error that
  names the rule (instead of continuing on a model that differs from SimuLizar's): monitor changes
  (MC-6.1), population and scenario changes (MC-5.2), structural behaviour changes such as a branch
  added to a SEFF (MC-4.3), an assembly context its validation rejects (e.g. one without an
  allocation context, which SimuLizar accepts until a request reaches it).
- **MC-7.5 Simulation time.** No simulation time passes while rules run; `getSimulationTime()` (the
  `SimulationStateLibrary` blackbox of `simulizar.action.repository`) returns the current time.
- **MC-7.6 New object ids and set order** (`DESIGN-SPD.md` R2). Generated PCM code gives every new
  `Identifier` an `EcoreUtil.generateUUID()` in its constructor (`IdentifierImpl`), also every copy
  that `deepclone()` makes (EMF's `Copier` constructs it, then copies the original's id over it), and
  objects SimuLizar creates outside rules (the runtime measurement model and its measurements, the
  per-call system assembly contexts). The UUIDs are time-based, so a rule that creates an object
  without setting its id gives it a new id in every run; ids reach the trace (assembly contexts,
  containers, actions, branch transitions), the stop's deactivation order (the resource registry's
  `HashMap` of container ids, MEAS-10.1) and string comparisons in rules. *SimOxide* and the
  patched reference (patch P10) draw the ids of objects created while a rule runs from one generator
  per run: SplitMix64 seeded with `0x5EED0F51` (`RunConfig::default()`), two draws per id, in
  `EcoreUtil.generateUUID`'s encoding (`_` and 22 characters); a copy draws too. Objects created
  outside rules draw nothing (SimOxide's PRM mirror has no ids). Iteration of a QVTo or OCL `Set`
  (`asSet`, `->any`, `allInstances`, …) of model objects follows Java identity hashes in SimuLizar
  (a different order in every run); the reference iterates QVTo's and OCL's hash collections in
  insertion order (P10, the ordering rule of the Slingshot reference's `det` mode), as QVTo's ordered
  mode does, which SimOxide uses for rules by default.
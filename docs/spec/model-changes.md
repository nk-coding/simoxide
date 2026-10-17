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

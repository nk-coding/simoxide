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
  and allocation lookups that lead to them still work. The resource registry keeps it, so it
  still takes part in the final deactivation (SIM-6.5). *SimOxide*: this is the removal policy
  `Removal::Keep`; the default policy drains (MC-8.2).
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
  `UnsupportedOperationException` (ROUTER:132, 159): the simulation aborts. *SimOxide*: under
  `Removal::Keep` (SimuLizar's syncers); under the other policies the notification changes
  nothing (a moved container stays routed).
- **MC-2.4** The route between two containers is the first link in the router's order that
  connects both (ROUTER:81-94, as ACT-11).

## MC-3 Allocation

- **MC-3.1** An allocation context added to the allocation adds the lookup entries of its
  assembly context's path and of every path nested below it; later entries overwrite earlier
  ones (`doAddAllocationContext`, ALS:138-160, 263-269). An added context without assembly context adds nothing.
- **MC-3.2** Changes of an existing allocation context's container or assembly context are
  ignored: ALS reacts to `SET` only on `Allocation.allocationContexts`. *SimOxide*: the default
  (`Reallocation::Ignore`); `Reallocation::Apply` recomputes the context's lookup entries (as
  MC-3.1), so requests use the new container from their next demand on (e.g. SimuLizar's
  `migratecomponent.qvto`).
- **MC-3.3** Removing an allocation context aborts with a `NullPointerException`
  (`processNotification` passes the notification's *new* value, `null`, to
  `doAddAllocationContext`, ALS:244-258, 292-296). `UNSET`, `MOVE` and any other event type inside the
  allocation abort with `UnsupportedOperationException` (ALS:91-110). `EcoreUtil.delete` of a
  container or assembly context that an allocation context refers to unsets that reference, so
  it aborts too. *SimOxide*: under `Removal::Keep`; under `Drain` and `Abort` a removed context's
  lookup entries stay for the requests in progress (an assembly context left unallocated is
  still a validation error), an `UNSET` is a reallocation (MC-3.2).
- **MC-3.4** The lookup happens at every demand (RDSeffPerformanceSwitch:80, 130) and at every
  call over an assembly connector (ComposedStructureInnerSwitch:117-121): requests in progress
  use the new entries from their next demand on.

## MC-4 System and repository

- **MC-4.1** The interpreter reads the system and the repository at every call: calls made after
  a change resolve over the new connectors and assembly contexts; a call in progress continues
  where it was resolved to.
- **MC-4.2** A changed action takes effect at its next execution (branch probabilities, guards,
  loop counts, demands, parameter characterisations).
- **MC-4.3** *SimOxide*: behaviours are compiled to instruction streams, rebuilt at every
  change of a behaviour. An execution in progress continues as `InProgress` says:
  - `Follow` (the default, SimuLizar's semantics: `RDSeffSwitch` and `UsageScenarioSwitch` read
    an action's successor after the action): it finishes the action it is in and continues with
    that action's successor in the changed behaviour. Inside a changed action it continues at
    the same place (after the same demand, infrastructure or resource call by index, or after
    the last instruction of the action with a lower place: `Begin` < demands < infrastructure
    calls < resource calls < call or effect < return < `End`). An execution whose action left
    the behaviour, or whose call in progress lost its return (an external call changed its
    kind), keeps its old instructions.
  - `Finish` (an option, F2's behaviour): it finishes the instructions it started with.
  Kept old instructions are appended to the new streams; at every change only those an
  execution still uses are kept (the streams do not grow with the number of changes). A change
  is refused (`Unsupported`, nothing changed) when kept instructions no longer fit their changed
  action (e.g. a resource call that no longer exists), or an infrastructure call or collection
  iterator in progress lost its action's structure. SimuLizar's own behaviour there (a Java
  iterator over a changed `EList`, possibly a `ConcurrentModificationException`) is not
  reproduced.
- **MC-4.4** *SimOxide*: the per-site call caches and provided-role resolutions are dropped at
  every change (calls resolve again on the changed system).

## MC-5 Usage model

- **MC-5.1** A changed think time or inter-arrival time applies from the next draw
  (UMS:72-106, 113-139).
- **MC-5.2** A changed population (`ClosedWorkload.setPopulation`, UMS:46-69) starts the
  missing users at once (each starts with its usage-scenario run) or asks the oldest users to
  stop: they leave after their current run and think time (`ClosedWorkloadUser.requestStop`). A
  new usage scenario's workload driver starts (UMS:141-146); a removed one's stops
  (`cancelAndUnregisterWorkloadDriver`, UMS:147-153: the open generator at its next arrival,
  closed users as above). A changed workload kind (a new workload object) is ignored (UMS logs
  "no resync strategy"). *Slingshot flavour*: the same (fixed behaviour of Q14; with the Q14
  flag population changes are ignored).

## MC-6 Monitors

- **MC-6.1** `MonitorSync::Reference` (the default). *SimuLizar*: the probe framework sets up
  sliding windows, PRM recorders, response-time, reconfiguration-time and container-count
  calculators at the start only; at run time `ResourceEnvironmentSyncer.attachMonitors` gives a
  *new* active resource the state (per replica), overall-utilisation and demand calculators of
  the monitors whose measuring point refers to its specification (their series appear from the
  change on, without initial tuples). Every other new monitor is ignored. New actions find the
  existing response-time calculators by id (a copy keeps its original's id). *Slingshot*: every
  new monitor of the monitor repository is interpreted
  (`MonitorRepositoryInterpreterBehavior.onModelAdjusted`): active-resource probes of new and
  existing resources, passive-resource probes of every instance.
- **MC-6.2** `MonitorSync::Full` (SimuLizar flavour, an extension): every new measurement
  specification gets what it would get at the start: series, sliding windows (their first
  window ends one length after the change), PRM recorders and runtime measurements (appended to
  the PRM), reconfiguration-time and container-count series. Removed monitors keep their
  calculators in both modes.
- **MC-6.3** Monitor cloning (Slingshot's `ResourceContainerMonitorCloner`, `Edit::clone_monitors`
  and `Edit::clone_monitor`; the SPD adjuster clones the monitors `ModelAdjusted` lists): copies of
  the template container's monitors (resource-container measuring points, then per processing
  resource the active-resource measuring points) with new measuring points for the new container
  and its processing resource of the same type.
- **MC-6.4** The `Number of Resource Containers` tuple: after a reconfiguration process as
  MEAS-7.3; after any other change (direct edits, SPD adjustments) when the number of top-level
  containers changed, at the change.

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
  of MC-1 to MC-6. A change that SimuLizar's syncers abort on (MC-2.3, MC-3.3, under
  `Removal::Keep`) ends the run with an error, as in SimuLizar. A change SimOxide refuses ends the
  run with an error that names the rule: an assembly context its validation rejects (e.g. one
  without an allocation context, which SimuLizar accepts until a request reaches it), kept
  instructions that no longer fit their action (MC-4.3). Two differences of applying a rule's
  changes at its end: `attachMonitors` sees the monitors the rule created after the container
  (SimuLizar: only those created before it), and the resources of a container added and removed
  in one rule (MC-8.5).
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

## MC-8 Removal policies and options (SimOxide)

`ChangeOptions` (`Simulation::set_change_options`; part of the run state: forks and snapshots
keep it; change files set it with `removal`, `in-progress`, `reallocation`, `monitors` lines).
Defaults (`ChangeOptions::for_flavour`): `Drain` (Slingshot with the Q5 flag: `SlingshotQ5`),
`Follow`, `Reallocation::Ignore` (Slingshot: `Apply`), `MonitorSync::Reference`.
`ChangeOptions::simulizar()` is SimuLizar 5.2.2's syncers in every respect.

- **MC-8.1 Keep** (SimuLizar, MC-1.5): resources of a container or processing resource that
  leaves the environment keep simulating and serving; MC-2.3 and MC-3.3 abort.
- **MC-8.2 Drain** (the default, both flavours): the resources finish the jobs they have and
  serve the requests that were in progress at the change, their later demands included (a
  request is a usage-scenario run; forked behaviours belong to it). Their later calls are
  included too: an assembly context that left its structure (with its connectors, as an SPD
  scale-in removes a pod's assemblies) calls over the connectors it had when it left; without
  such a connector the call fails as in SimuLizar ("Required delegation of the system"). A request that starts after
  the change and still reaches them is aborted (MC-8.6). Once idle with no such request left
  they are gone: no final deactivation tuples (SIM-6.5). Routing away is the model's part (a
  removed allocation context, a branch probability).
- **MC-8.3 Abort**: the jobs at the resources are aborted at the change (their requests,
  MC-8.6), every later demand there aborts its request; the resources are gone.
- **MC-8.4 SlingshotQ5** (Slingshot quirk Q5, SS-SPD-5.4): the job whose completion is pending is
  aborted at that completion (a delay resource: every job at its end), the other jobs are lost
  (their requests never continue); later demands abort their requests; removed allocation
  contexts' lookups end at once; aborted requests are not counted (Slingshot's `UserAborted`).
- **MC-8.5** A container or link added to the environment and removed again in one change set:
  `Keep`: SimuLizar created and registered its resources at the addition; they are unreachable
  and have no monitors, so its id is registered (a later container with that id is not created,
  MC-1.1); otherwise it never had resources.
- **MC-8.6 Aborted requests**: a closed-workload user is replaced by a new one that thinks
  first (SS-WL-5.1), an open-workload user is dropped, a forked behaviour ends (a synchronous
  fork's parent continues). The request records no response time; what it holds stays held.
  It counts as a finished usage-scenario run (SimuCom's users count failed runs too, so a run
  that stops after a number of runs stops).

## MC-9 The Slingshot flavour

The Slingshot flavour takes the same change sets (the fast mode; the Slingshot-exact engine
`ssx` has its own model reference, X2d). Rules SS-SPD-5 and SS-RES-6 with the fixed behaviours:

- **MC-9.1** Resources exist for allocated containers (SS-RES-1); a container that becomes
  allocated gets its resources with its monitors' probes (MC-6.1); rates follow the model.
- **MC-9.2** Allocation: the first allocation context of an assembly context wins, read live
  (SS-ACT-9.1): new contexts of an allocated assembly context change nothing, re-pointed ones
  apply (`Reallocation::Apply`), removed ones keep their entries for the requests in progress
  (all policies but `SlingshotQ5`).
- **MC-9.3** Passive resources of new allocation contexts are created at the change
  (`buildPassiveResources`); passive monitors of every instance follow the monitor repository.
- **MC-9.4** Routing: links' connected containers are read live; new links are routed (Slingshot
  reads its link table once: fixed behaviour); removed links leave the routing.


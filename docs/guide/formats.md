# Output formats

SimOxide and the [reference simulator](../reference-simulator/refsim.md) write the same three files
per run, in the format `palladio-trace/1`. In the reference they come from hooks in
`reference/patches/src` and the writer `reference/src/refsim/trace/Trace.java`; in SimOxide from
`simoxide-sim` (`trace.rs`, `rng.rs`, `meas.rs`). `simoxide-testkit` has parsers and writers for all three.

| File | Content |
|---|---|
| `trace.jsonl` | event trace, one JSON object per line, in execution order |
| `tape.jsonl` | random tape: every uniform draw, plus the derived value of each sampling evaluation |
| `measurements.csv` | all recorded measurements (raw series per measuring point and metric) |

Two runs agree if and only if the files are byte-identical. When they differ, the first differing trace line
localises the divergence.

## 1. Common rules

- **JSON lines.** UTF-8, `\n` terminated, no spaces, keys in the order listed below. Strings escape `"` `\`
  and control characters (`\n`, `\r`, `\t`, other `< 0x20` as `\u00XX`).
- **Numbers.** Doubles use **Java `Double.toString` (JDK ≥ 19)**. This is the shortest decimal that
  round-trips to the same double. Formatting:
  - if `1e-3 <= |x| < 1e7`: plain decimal with at least one fractional digit (`3.0`, `0.018428198`, `1493559.946308255`);
  - otherwise: `d.ddd` + `E` + exponent (`1.0E7`, `1.0E-4`, `7.212694342041994E-4`); the mantissa always has
    a fractional digit;
  - `0.0` and `-0.0` as written; NaN / infinities as JSON strings `"NaN"`, `"Infinity"`, `"-Infinity"`.

  SimOxide takes the shortest digits from `ryu` and applies these rules. Integers (ids, counts, `n`, `idx`) are
  plain JSON integers.
- **Time `t`.** The current simulation time in seconds. DESMO-J keeps time as an integer number of
  nanoseconds (`ns`). Every delay is truncated with `span(d) = (long)(d * 1e9)` ([SIM-2.2](../spec/simulation.md)). `t` is
  computed as `(double) ns / 1e9`. Both simulators print exactly this double. `ns` can be recovered as
  `round(t * 1e9)`, which is exact while `ns < 2^53` (about 104 days of simulated time).
- **Process ids `p`.** Every simulated process gets an id from 1 in **creation order** within the run. The
  id is assigned in the `AbstractSimProcessDelegator` constructor. Processes are users, the open-workload
  generator, and forked behaviours. `p` is 0 or absent for events outside a process, such as simulator
  events and measurements. SimOxide creates processes in the same order ([SIM-4.1](../spec/simulation.md)).
- **Model element ids.** These are PCM `id` attributes. For elements without an id, the EMF URI
  fragment is used.
  - SimuLizar's synthetic system assembly context has the fixed id `_SYSTEM_ASSEMBLY_CONTEXT_`. This
    replaces a per-JVM random UUID; see [patch P7](../reference-simulator/patches.md#patch-list).
  - Resource types are the ids from `Palladio.resourcetype`: CPU `_oro4gG3fEdy4YaaT-RYrLQ`,
    HDD `_BIjHoQ3KEdyouMqirZIhzQ`, DELAY `_nvHX4KkREdyEA_b89s7q9w`, LAN `_o3sScH2AEdyH8uerKnHYug`.

## 2. `trace.jsonl` events

`ev` comes first, then `t`, then the fields below, in this order. Optional fields are omitted when null.

| `ev` | fields (in order) | emitted when |
|---|---|---|
| `header` | `format`, `run`, `seed`, `max_sim_time`, `max_measurements` | first line (`t` = 0.0) |
| `spawn` | `p`, `kind`, `name`, `parent` | process constructor. `kind` is the Java class: `ClosedWorkloadUser`, `OpenWorkloadUser`, `OpenWorkload` (the generator, name `OpenWorkloadUserMaturationChamber`), `ForkedBehaviourProcess`, `ReconfigurationProcess` (created at the first runtime-measurement write after t = 0 when a monitor has `triggersSelfAdaptations = true`, [MEAS-7.2](../spec/measurements.md)), … `name` is the SimuCom process name (`ClosedUser`, `OpenUser`, `Forked Behaviour`, `Reconfiguration Process`). `parent` is the process running at creation (0 = engine/init) |
| `pend` | `p` | process life cycle finished |
| `begin` / `end` | `p`, `type`, `id`[, `ac`] | interpreter "passed" events, before the listeners (probes) run. `type` = EClass name. **User actions:** `UsageScenario`, `Start`, `Stop`, `EntryLevelSystemCall`, `Delay`, `Branch`, `Loop`. **SEFF actions:** `StartAction`, `StopAction`, `InternalAction`, `ExternalCallAction`, `BranchAction`, `LoopAction`, `CollectionIteratorAction`, `ForkAction`, `AcquireAction`, `ReleaseAction`, `SetVariableAction`, …, with `ac` = id of the assembly context on top of the interpreter's context stack |
| `begin` / `end` | `p`, `type`=`SystemOperation`, `role`, `sig` | entry level system call enters/leaves the system (provided role, signature) |
| `begin` / `end` | `p`, `type`=`AssemblyOperation`, `ac`, `role`, `sig` | a provided role of an assembly context is entered/left. The first one per call has `ac` = `_SYSTEM_ASSEMBLY_CONTEXT_` |
| `hold` | `p`, `d` | `SimuComSimProcess.hold(d)`: usage `Delay`, closed-workload think time, open-workload inter-arrival time |
| `branch` | `p`, `id`, `idx`, `sel` | branch decided. `id` = BranchAction / usage Branch; `idx` = 0-based index of the chosen transition; `sel` = transition id. For a usage branch `sel` is the EMF fragment, since `BranchTransition` has no id. Guarded branch with no true guard: `idx` -1 |
| `loop` | `p`, `id`, `n` | iteration count evaluated: `LoopAction`, `CollectionIteratorAction` (`NUMBER_OF_ELEMENTS`) and usage `Loop` |
| `infra` | `p`, `id`, `n` | number of calls of an `InfrastructureCall` evaluated |
| `fork` / `join` | `p`, `id`[, `async`, `sync`] | ForkAction: children created (counts) / `ForkExecutor.run()` returned |
| `demand` | `p`, `res`, `rc`, `spec`, `sched`, `d`, `st` | demand issued to an active resource (`AbstractScheduledResource.consumeResource`). `res` = resource type id. `rc` = container id; for linking resources, the linking resource id. `spec` = ProcessingResourceSpecification id; for linking resources, the CommunicationLinkResourceType name (`LAN`). `sched` = scheduling strategy (`PROCESSOR_SHARING`, `FCFS`, `DELAY`; linking resources are `FCFS`). `d` = abstract demand. `st` = service time: `d` / processing rate, or for links `d` / throughput + latency, after demand modifiers |
| `demand_done` | `p`, `res`, `rc` | the demand is complete and the process continues. If `st <= 0`, SimuLizar skips the resource and `demand_done` follows immediately |
| `acquire` | `p`, `res`, `ac`, `n`, `avail`, `queue` | passive resource acquire requested. `res` = PassiveResource id, `ac` = assembly context, `avail` and `queue` are taken before granting |
| `grant` | `p`, `res`, `ac`, `n`, `avail` | passive resource granted (immediately or later from the queue); `avail` after granting |
| `release` | `p`, `res`, `ac`, `n`, `avail` | passive resource released; `avail` after the release. Grants to waiting processes follow |
| `meas` | `mp`, `metric`, `v` | a measurement reached the recorder. `v` = `[time, value]` (see §4) |
| `stop` | – | `AbstractExperiment.stop()`: a stop condition held after an event, or the simulation was stopped |
| `finish` | `uniforms`, `measurements` | last line: total uniforms drawn, total measurement tuples |

Notes:
- **Events after `stop`.** When the simulation stops, SimuLizar resumes the remaining suspended processes
  so that they can finish ([SIM-6.5](../spec/simulation.md)). Resources no longer wait at this point.
  - The resource table drains processes in **insertion order** ([patch P3](../reference-simulator/patches.md#patch-list)).
  - The trace records these post-stop events, and measurements may still be recorded.
- **Double resume ([SIM-4.4a](../spec/simulation.md)).** A `demand_done` may follow its `demand` at the same `t` although
  `st > 0`. This happens when a pending resume wakes the process from the wait early. It is reference
  behaviour, and SimOxide reproduces it (corpus `h26_fork_double_resume`).
- **Equal-time order.** Events at the same `t` appear in execution order. This follows DESMO-J FIFO order
  for same-time notes and the order within a process step.
- A `begin`/`end` pair of the same element in one process always nests correctly. Lines of different
  processes interleave.

## 3. `tape.jsonl` (random tape)

All randomness comes from **one** stream per run. The stream is `SimuComDefaultRandomNumberGenerator` →
commons-math 2.1 `MersenneTwister`, seeded with `setSeed(int[6])` = `{seed, seed+1, …, seed+5}`, one
`nextDouble()` per draw (see [Random numbers](../spec/random.md)). Two record kinds, in consumption order:

```
{"k":"u","i":<n>,"u":<uniform>,"o":"<origin>"}
{"k":"s","n":<uniforms used>,"o":"<origin>","spec":"<stoex>","v":<value>}
```

- `u` records every uniform in (0,1). `i` counts from 0 per run.
- `s` records the result of an **outermost** StoEx evaluation that consumed at least one uniform. It is
  written right after that evaluation's `u` lines.
  - `spec` is truncated to 64 characters (61 + `...`).
  - `v` is the raw evaluation result before the caller converts it. Doubles use the number format above,
    integers and booleans are plain, anything else is a string.
- **Origin** `o` has the form `purpose[:elementId]`. It says why the draw happened:

| origin | element | draw site |
|---|---|---|
| `demand:<id>` | InternalAction | ParametricResourceDemand specification |
| `rescall:<id>` | ResourceCall | number of calls |
| `infra:<id>` | InfrastructureCall | number of calls |
| `param:<id>` | ExternalCallAction / EntryLevelSystemCall / InfrastructureCall | input variable usages |
| `return:<id>` | ExternalCallAction / EntryLevelSystemCall | return / output variable usages |
| `branch:<id>` | BranchAction / usage Branch | the single `random()` of a probabilistic branch |
| `guard:<id>` | GuardedBranchTransition | branch condition |
| `loop:<id>` | LoopAction / usage Loop | iteration count |
| `collection:<id>` | CollectionIteratorAction | `NUMBER_OF_ELEMENTS` |
| `inner:<id>` | CollectionIteratorAction | per-iteration INNER proxy evaluation (HashMap order, [ACT-7.3](../spec/actions.md)) |
| `setvar:<id>` | SetVariableAction | variable usages |
| `delay:<id>` | usage Delay | time specification |
| `rate:<containerId>` | ResourceContainer / LinkingResource | processing rate, link throughput and latency, evaluated in `consumeResource` |
| `think` | – | closed workload think time |
| `interarrival` | – | open workload inter-arrival time |
| `?` | – | anything else, e.g. lazily evaluated proxies read elsewhere, passive resource capacities |

**Replay mode.** A consumer can feed the `u` values back in order instead of generating them, and assert
the origin as a debugging aid. The `s` records let a test check derived samples without re-implementing
the distribution.

## 4. `measurements.csv`

```
measuring_point,metric,time,value
UsageScenarioMeasuringPoint[_LPnI8CHdEd6lJo4DCALHMw],Response Time Tuple,3.0,3.0
```

- **One row per recorded measurement tuple**, with `time` = the "Point in Time" component.
  - The other component is `value`. Tuples whose metric lists the value first, such as
    `Resource Demand Tuple`, are reordered.
  - Units are the metric default units: seconds for times and demands, and dimensionless counts for
    states.
- **Row order.**
  - Series are sorted by `(measuring_point, metric)` in Java `String.compareTo` order (UTF-16 code units,
    which equals byte order for ASCII).
  - Within a series, rows keep emission order.
- **Numbers.** Same format as in §1.
- **Fields.** Quoted only if they contain `,` `"` or a newline (RFC 4180). Keys are designed not to need it.
- **`metric`.** The MetricDescription name from `commonMetrics.metricspec`: `Response Time Tuple`,
  `State of Active Resource Tuple`, `Resource Demand Tuple`, `Waiting Time Tuple`, `Holding Time Tuple`,
  `State of Passive Resource Tuple`, `Utilization of Active Resource Tuple` (overall utilisation of
  multi-core resources), `Reconfiguration Time Tuple`, `Number of Resource Containers over Time`.
- **`measuring_point` key.** `EClassName[` + parts joined by `|` + `]`. The parts are the ids of the
  non-containment single references and the `name=value` of the attributes, in metamodel feature order.
  Concretely:

| measuring point | key |
|---|---|
| usage scenario | `UsageScenarioMeasuringPoint[<usageScenarioId>]` |
| entry level system call | `EntryLevelSystemCallMeasuringPoint[<elscId>]` |
| system operation | `SystemOperationMeasuringPoint[<providedRoleId>\|<signatureId>\|<systemId>]` |
| assembly operation | `AssemblyOperationMeasuringPoint[<roleId>\|<signatureId>\|<assemblyContextId>]` |
| external call | `ExternalCallActionMeasuringPoint[<externalCallActionId>]` |
| active resource (replica i) | `ActiveResourceMeasuringPoint[<processingResourceSpecId>\|replicaID=<i>]`. For a PS resource with n > 1 cores, SimuLizar also records `Utilization of Active Resource Tuple` at `replicaID=<n>` |
| passive resource | `ResourceURIMeasuringPoint[<passiveResourceId>\|Passive Resource: <assemblyContextName>.<passiveResourceName>]`. SimuLizar creates this point itself; the URI is reduced to its fragment |
| reconfiguration (`ReconfigurationMeasuringPoint`, any `ResourceURIMeasuringPoint`) | `ResourceURIMeasuringPoint[<fragment of resourceURI or null>\|<measuringPoint>]` |
| resource environment | `ResourceEnvironmentMeasuringPoint[<environment id or its fragment "/">]` |
| others (generic rule) | `SubSystemOperationMeasuringPoint[<subsystemId>\|<roleId>\|<signatureId>]`, `LinkingResourceMeasuringPoint[<linkingResourceId>]`, `ResourceContainerMeasuringPoint[<containerId>]`, `ResourceEnvironmentMeasuringPoint[<envId or fragment>]`, `AssemblyPassiveResourceMeasuringPoint[<assemblyContextId>\|<passiveResourceId>]` |

The trace `meas` event uses the same `mp` and `metric` strings and `v` = `[time, value]` in the same
normalised order, in emission order across all series.

## 5. Run configuration (`run.json`)

```json
{ "seed": 1, "max_measurements": 100, "max_sim_time": -1,
  "simulate_linking_resources": false, "simulate_throughput_of_linking_resources": true,
  "usagemodel": "x.usagemodel", "allocation": ["x.allocation"], "monitorrepository": "x.monitorrepository" }
```

- **Stop conditions.**
  - `max_sim_time` is an integer, because SimuLizar parses it as a `long`; -1 turns it off.
  - `max_measurements` counts finished usage-scenario runs, i.e. SimuCom's "main measurements"; -1 turns
    it off.
- **Network flags.** Both are SimuComConfig keys; the defaults shown are the SimuLizar UI defaults.
  `simulate_linking_resources` means middleware marshalling: the payload demand is `stream.BYTESIZE` of the
  request/result frame, which SimuLizar does not add itself ([ACT-11.3](../spec/actions.md)). The corpus keeps it `false`
  except `h30_middleware_stream`, whose calls pass the stream explicitly.
- **Model files.** The last three keys are optional. Without them, the directory is searched for exactly
  one `.usagemodel`, one or more `.allocation` files and at most one `.monitorrepository`.

# Runtime metadata

An external tool that reads SimOxide's measurements also needs to know what they refer to: which
containers, resources, assemblies and allocations the run has, how they are configured, how
scaling changed them and when, and which model element each measurement series measures.
`simoxide run` and `simoxide resume` export this in two documented JSON files, with stable ids
taken from the model:

| Flag | File | Written |
|---|---|---|
| `--model-metadata FILE` | the **model file** (`simoxide-model`): the model's definitions and its deployment, with a content hash | when asked; the same content for every invocation on the same model |
| `--runtime-metadata FILE` | the **runtime file** (`simoxide-runtime`): the invocation's deployment at its start, every change after it, the policy state, the retired elements and the metadata of every measurement series | per invocation |

```sh
# the root run: the model file once, the runtime file of the first interval
simoxide run --model M --flavour slingshot --spd-quirks fixed --max-sim-time 600 \
    --pause-at 300 --snapshot s1.snap --measurements-scope interval --measurements m0.csv \
    --model-metadata model.json --runtime-metadata r0.json
# each continuation: its own runtime file; it names the model file by its hash
simoxide resume s1.snap --model M --measurements m1.csv --runtime-metadata r1.json
```

Static definitions are in the model file, which a consumer reads once and caches under its
`hash`; a continuation does not need to write it again (its runtime file names the hash). The
runtime file is proportional to the deployment, the number of changes in the interval and the
number of series, not to the number of observations or the length of the history. Both files
are compact JSON, objects with sorted keys; arrays keep model order. Everything in them is a
function of the run state, so a paused and resumed run gives the uninterrupted run's metadata:
the same ids, the same changes at the same times, the same series.

## Identities

Every element is named by the model's id: the `id` of the PCM element (`ResourceContainer`,
`ProcessingResourceSpecification`, `LinkingResource`, `AssemblyContext`, `AllocationContext`,
...), the `.spd` id of a policy or target group. Elements that SPD adjustments, HPA rescales or
change files create get the ids those give them: Slingshot's transformation draws seeded UUIDs
from the run's id generator, which snapshots keep, so a resumed run gives the created elements
the uninterrupted run's ids; `add-container` lines name theirs. An id never changes and is
never given to another element during a run.

- `in_model`: the element (same id) is defined in the model the run started from, i.e. in the
  model file.
- `unit`: for a container or assembly that is an element of a target group, the group's unit:
  the element its scale-outs copy (for the unit itself, its own id). A pricing definition of the
  unit applies to its copies. `null` for an element of no group.

**Measurement series** are identified as in `measurements.csv`: by the pair
`(measuring_point, metric)`. A series exists from its first tuple's run on and is never
removed, also after the element it measures left the model.

## Model file

```json
{"format": "simoxide-model", "version": 1, "hash": "4c1f0e9a2b7d3e65",
 "resource_types": [...], "interfaces": [...], "components": [...], "systems": [...],
 "usage_scenarios": [...], "external_calls": [...], "monitors": [...],
 "deployment": {...}, "scaling": {...}}
```

| Member | Content |
|---|---|
| `format`, `version` | `simoxide-model`, `1` (a new version for incompatible changes; members may be added) |
| `hash` | FNV-1a 64 (16 hex digits) of the compact JSON of all other members: equal hashes, equal content |
| `resource_types` | `id`, `name` (`CPU`, `HDD`, `DELAY`, `LAN`, ...), `kind` (`processing`, `communication_link`) |
| `interfaces` | `id`, `name`, `kind`, `signatures` (`id`, `name`): the operations |
| `components` | `id`, `name`, `kind` (`basic`, `composite`, `subsystem`, ...), `repository`, `provided_roles` and `required_roles` (`id`, `name`, `interface`), `assemblies` (of a composite: `id`, `name`, `component`), `passive_resources` (`id`, `name`, `capacity`) |
| `systems` | `id`, `name`, `provided_roles`, `required_roles` |
| `usage_scenarios` | `id`, `name`, `workload` (`{"type": "closed", "population", "think_time"}` or `{"type": "open", "inter_arrival_time"}`), `calls`: its entry-level system calls (`id`, `name`, `role`, `signature`) |
| `external_calls` | the SEFFs' external call actions: `id`, `name`, `component`, `role`, `signature` |
| `monitors` | `id`, `name`, `activated`, `measuring_point` (`class`, `key` as in `measurements.csv` or `null` for a point SimOxide records no series at, and the referenced elements by id, as in a series' `point`), `specifications` (`id`, `metric` with `id` and `name`, `processing`: `type` and its window or aggregation parameters, `triggers_self_adaptations`) |
| `deployment` | the model's deployment, in the format of the runtime file ([Deployment](#deployment)) |
| `scaling` | `null` without scaling policies; else `spd` (its id), `target_groups` (`id`, `name`, `kind`, `constraints`: `TargetGroupSizeConstraint` with `min`, `max`; `ThrashingConstraint` with `minimum_time`), `policies` (`id`, `name`, `active`, `target_group`, `adjustment` as in the [decision file](./spd.md#decision-file), `constraints`: `CooldownConstraint` with `cooldown_time`, `max_scaling_operations`; `IntervalConstraint` with `offset`, `duration`, `repeat`), `autoscalers` (HPA: `name`, `target_group`, `policy`). Triggers are not described in version 1 |

A StoEx value (rates, latencies, capacities, think times) is
`{"expression": "<the specification>", "value": <number or null>}`: `value` is the constant the
expression folds to (`null` for a random or parametric expression).

## Runtime file

```json
{"format": "simoxide-runtime", "version": 1,
 "model": {"format": "simoxide-model", "hash": "4c1f0e9a2b7d3e65"},
 "run": {"parent": "fe43c5c8651b7945", "state": "92752f10c6460c33", "status": "paused"},
 "start": {"time": 300.0, "time_ns": 300000000000, "generation": 2,
           "deployment": {...}, "policies": {...}},
 "changes": [{"time": 362.0, "time_ns": 362000000000, "generation": 3,
              "cause": {"type": "policy", "policy": "_out", ...},
              "outcome": {"ok": true, "size_before": 2, "size_after": 3},
              "added": {"containers": [...], "allocations": [...]},
              "removed": {}, "changed": {"target_groups": [...]}}],
 "end": {"time": 600.0, "time_ns": 600000000000, "generation": 3, "policies": {...}},
 "retired": {"containers": [...], ...},
 "series": [...]}
```

| Member | Content |
|---|---|
| `model` | the model file it refers to, by `hash` |
| `run` | `parent`: the state id of the snapshot resumed (`null` for `run`); `state`: the state id of the snapshot written (`null` without one); `status`: `paused` or `finished` (the server also has `stopped` and `failed`). The ids are those of the snapshot headers ([Measurement intervals](./snapshots.md#measurement-intervals)), so the runtime files of a branch chain like its snapshots |
| `start` | the invocation's start: the time of the restore (0 for `run`), the model `generation` (the number of model changes the run has applied, 0: the model file's), the `deployment` (the initial projection) and the `policies` state ([Policy state](#policy-state); `null` without scaling policies) |
| `changes` | the configured-architecture changes of the invocation, in order ([Changes](#changes)) |
| `end` | the pause or the end of the run: time, generation and policy state |
| `retired` | elements the run's model still holds that left the deployment, as they were when they left ([Retired elements](#retired-elements)) |
| `series` | the metadata of every measurement series of the run ([Series](#series)) |

Times are simulation times, in seconds (`time`, a double) and nanoseconds (`time_ns`, an
integer), as everywhere in SimOxide.

**Replaying.** The deployment at any time of the interval is `start.deployment` with the
changes up to that time applied in order: add `added`, remove the ids of `removed`, replace the
elements of `changed` (by `id`). The deployment at the end, replayed, is the next invocation's
`start.deployment`; the changes of a path's runtime files, concatenated, are the uninterrupted
run's changes.

### Deployment

The deployment is what the model configures now, one member per kind; every element has an
`id`.

| Kind | Fields |
|---|---|
| `containers` | the containers of the resource environments (nested ones after their parent): `id`, `name`, `in_model`, `parent` (the enclosing container or `null`), `unit`, `resources`: the processing resources: `id`, `in_model`, `type` (name) and `type_id`, `scheduling` (`ProcessorSharing`, `FCFS`, `Delay`, ...), `rate` (StoEx value, work units per second: a demand of `d` takes `d / rate` seconds), `replicas` (`numberOfReplicas`, the cores), `mttf`, `mttr`, and for HDDs `hdd` (`read_rate`, `write_rate`) |
| `linking_resources` | `id`, `name`, `in_model`, `connected` (container ids), `type`, `type_id`, `latency` (StoEx value, seconds), `throughput` (StoEx value, payload units per second, the unit of the `BYTESIZE` characterisations), `failure_probability` |
| `assemblies` | the systems' assembly contexts: `id`, `name`, `in_model`, `component`, `system`, `unit` |
| `allocations` | the allocation contexts: `id`, `name`, `in_model`, `allocation`, `assembly`, `container` |
| `target_groups` | the SPD's target groups (empty without one): `id`, `name`, `kind` (`elastic_infrastructure`, `service_group`, `competing_consumers`), `unit`, `configured` (the semantic configuration has an entry for the group), `elements` (container or assembly ids), for competing consumers `broker` |

The deployment is the configured architecture: a container removed by a scale-in leaves it at
the change, although it may still finish the jobs it had (draining, [Model
changes](../spec/model-changes.md)); a container added by an HPA rescale is in it from the
rescale, before its pod is ready. Exact lifetimes of draining resources are not part of the
export.

### Changes

One record per change of the configured architecture, in the order they happened:

| Field | Meaning |
|---|---|
| `time`, `time_ns` | when the change happened |
| `generation` | the model generation after it |
| `cause` | what made it (below) |
| `outcome` | of an adjustment: `ok` and the target group's `size_before` and `size_after`; `null` for other causes |
| `added`, `removed`, `changed` | per kind, the elements added (as in the deployment), the ids removed, the elements changed (their new content); a kind without entries is left out. All three are empty for an adjustment that changed nothing (e.g. a scale-out at the group's maximum size) |

| `cause.type` | Meaning | Other fields |
|---|---|---|
| `policy` | a policy's recommendation, executed (a run without `--pause-on-decision`) | `policy`, `target_group`, `request` (the request's number in the run), `adjustment` |
| `decision` | an action `--decide` selected for decision `decision` | as `policy`, and `decision` |
| `enforce` | a policy enforced with `--enforce` | as `policy` |
| `hpa` | an HPA rescale | as `policy` (the autoscaler's policy), `autoscaler` (its index in `policies.autoscalers`) |
| `change` | a `--change FILE` step at the resume | `file` |
| `transform` | a `--transform FILE` step at the resume | `file` |
| `rules` | a reconfiguration of SimuLizar's rules that changed the model (`--rules`) | |

Recommendations and applied actions are kept apart: a change record is an action that ran. A
recommendation that is pending (`--pause-on-decision`) is in the policy state's
`pending_decision`, with nothing in `changes`; the decision file
([Decisions](./spd.md#decisions)) has the resolved decision's recommendations next to the
selected actions. HPA parameters (`--hpa-set`) change no element: the policy state at the end
shows them.

### Policy state

`null` without scaling policies; else:

| Field | Meaning |
|---|---|
| `policies` | per policy of the SPD: `id`, `active`, `state` (`null` for an inactive policy, which has no adjustor): `scales` (adjustments the policy made), `latest_adjustment` (its time, `null` before the first), `cooldown_end` and `scales_in_cooldown` (its cooldown constraint admits an adjustment at `t` while `t >= cooldown_end` or `scales_in_cooldown < max_scaling_operations`) |
| `target_groups` | per target group: `id`, `size` (its element count, `null` without a configuration), `enactments`, `last_enactment` (`time` and `policy` of the last adjustment of the group's policies, which the thrashing constraint reads; `null` before the first) |
| `decisions_made` | the number of scaling decisions the run has made ([Decisions](./spd.md#decisions)) |
| `pending_decision` | the decision the run paused at, as in the decision file (`decision`, `time`, `time_ns`, `recommended`), else `null` |
| `autoscalers` | per HPA autoscaler: `name`, `min_replicas`, `max_replicas`, `target_utilization` (as set now, with `--hpa-set`), `ready` and `starting` pods, `rescales` |

The state is the one the policies use, which they update when a recommendation is made, not
when it is applied ([Decisions](./spd.md#decisions)).

### Retired elements

A run keeps the elements that left its model (tombstones) so that requests in progress and
measurements that refer to them stay resolvable. `retired` lists them, in the format of the
deployment, as the model holds them now: `containers` (with their resources),
`processing_resources` (removed from a container that stayed; with `container`),
`linking_resources`, `assemblies`, `allocations`. An element keeps its own attributes and the
references it holds (a container its resources, an allocation context its assembly and
container); the references to it that its removal deleted are gone (`EcoreUtil.delete`), so a
retired assembly has no `system`, a retired allocation context no `allocation`, and a retired
container no `unit`. The deployment before the change that removed an element has it complete:
`start.deployment` replayed to that change. Retired elements accumulate over a run, as its
series do; the runtime file of every later invocation lists them, so a series that refers to a
removed element stays resolvable without the earlier files.

### Series

One object per measurement series of the run, in the order of `measurements.csv`, whether or
not the invocation recorded tuples for it:

```json
{"measuring_point": "ActiveResourceMeasuringPoint[_KFRwgCpXEeKNWc2UWjbmuw|replicaID=0]",
 "metric": "State of Active Resource Tuple", "unit": "1",
 "point": {"class": "ActiveResourceMeasuringPoint", "processing_resource": "_KFRwgCpXEeKNWc2UWjbmuw",
           "replica": 0, "container": "_GAeeUCpXEeKNWc2UWjbmuw", "resource_type": "_oro4gG3fEdy4YaaT-RYrLQ"},
 "monitors": [{"id": "_hVB9RtqvEe2AxOOsE9OYvg", "name": "ActiveResourceMonitor",
               "activated": true, "specification": "_hVB9R9qvEe2AxOOsE9OYvg"}],
 "unavailable": []}
```

| Field | Meaning |
|---|---|
| `measuring_point`, `metric` | the series' identity, as in `measurements.csv` |
| `unit` | of the values: `s` for `Response Time Tuple`, `Resource Demand Tuple`, `Waiting Time Tuple`, `Holding Time Tuple`, `Reconfiguration Time Tuple`; `1` for `State of Active Resource Tuple` (jobs at the resource), `Utilization of Active Resource Tuple` (busy fraction), `State of Passive Resource Tuple` (tokens), `Number of Resource Containers over Time`. The `time` column is always simulation time in seconds |
| `point` | the model elements the measuring point refers to, by id: `class` and, per class, `usage_scenario`; `entry_level_system_call` with its `usage_scenario`, `role`, `signature`; `role`, `signature`, `system` (system operation); `role`, `signature`, `assembly`, `component` (assembly operation); `subsystem`, `role`, `signature`; `external_call` with `role`, `signature`, `component`; `processing_resource`, `replica`, `container`, `resource_type` (active resource; `replica` = the core count `n` is SimuLizar's overall utilisation of `n` cores); `linking_resource`; `container`; `resource_environment`; `assembly`, `passive_resource`, `component` (passive resource); `resource_uri_fragment` and `text` (reconfiguration points) |
| `monitors` | the monitors of the run's model whose measuring point has this key (with the monitors Slingshot clones for new containers): `id`, `name`, `activated`, `specification` (the id of its measurement specification whose metric is the series' metric, else `null`) |
| `unavailable` | the associations neither the key nor the model gives: `monitor` (no monitor has the point: SimuLizar's passive-resource points, Slingshot's overall utilisation at `replicaID=n`), `assembly` (SimuLizar's passive-resource points name the assembly context by name only), and any derived association (`container`, `component`, ...) whose element the model does not hold |

The ids come from the key, whose parts are model ids by definition ([Formats](./formats.md#_4-measurements-csv)),
and from the model (with its retired elements); names are never used to find an element. The
container of a series of a removed container's resource is in `retired`; a removed container's
series keeps its metadata for the rest of the run.

## Library

`simoxide_sim::inspect` builds both files: `deployment` (a model's deployment), `Deployment::diff`,
`Recorder` (the start and the change records of an invocation; `Recorder::into_json` writes the
runtime file with `retired` and `series`), `definitions`, `model_file` and `seal` (the model
file). `simoxide_spd::inspect` adds the SPD: `deployment` and `add_target_groups` (target
groups and `unit`), `model_file` (with the scaling definitions), `policy_state`, and
`Recording`, an adjuster that records every adjustment of an `SpdRun` with its cause.

## Limitations

- The Slingshot-exact engine (`--flavour slingshot-exact`) does not support `--runtime-metadata`
  (`--model-metadata` works).
- The policies' triggers, the SEFFs and the usage model's behaviour are not described.
- Draining lifetimes, pod readiness and per-request routing are not part of the export.
- `explore` does not write runtime files.

## Server

A simulation request with `config.runtime_metadata: true` gives the job's results the model
file and the runtime file of its run ([Server](./server.md#runtime-metadata)).

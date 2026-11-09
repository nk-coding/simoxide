# Scaling policies (SPD)

SimOxide reads Slingshot's scaling policy definitions (`.spd`, with the semantic configuration
`.semanticspd`) and evaluates them during a run, as Slingshot's SPD interpreter does: per
policy a filter chain of target-group check, trigger, constraints and adjustor. The semantics
are on [Scaling policies (Slingshot)](../spec/slingshot/spd.md); the design is
`DESIGN-SPD.md` §5.4.

Kubernetes-style autoscaling (an HPA controller whose rescales are absolute adjustments of a
target group) is on [Autoscaling (HPA)](./hpa.md).

A policy that fires produces an *adjustment request* (policy, target group, adjustment); the
adjustment runs Slingshot's own transformation, `MainTransformation.qvto`, on the run's model
(below, [Adjustments](#adjustments)) and the run continues on the changed model.

::: tip Scale-in, monitors, flavours
Scale-in drains the removed containers by default: their jobs and the requests in progress
finish there, new requests go elsewhere ([Model changes](../spec/model-changes.md) MC-8; the Q5
flag reproduces Slingshot's removal). The adjuster creates the monitors Slingshot clones for new
containers, so their resources are measured from the adjustment on, and measurement triggers
see them. Both flavours take the adjustments: in the Slingshot flavour (`--flavour slingshot`)
the triggers read Slingshot's instantaneous utilisation samples, and with Slingshot's quirks
(`--quirks fast --spd-quirks slingshot`) the runs match stock Slingshot statistically; the
Slingshot-exact engine (`--flavour slingshot-exact`) reproduces the reference's SPD runs byte for
byte.
:::

## What is supported

| Part | Support |
|---|---|
| Policies | active policies in model order; inactive ones are skipped |
| Triggers | `SimpleFireOnValue` on `SimulationTime`, `OperationResponseTime`, `CPUUtilization`, `TaskCount`, `QueueLength`; `ComposedTrigger` (AND, OR, XOR) |
| Expectations | `ExpectedTime`, `ExpectedPercentage` (compared as `value / 100`), `ExpectedCount`, `NoExpectation` |
| Aggregation | `AVERAGE`: exponential moving average (60 s, emit every 10 s, α = 0.2, decayed over the time between samples that no sample covers); `MAX`, `MIN`, `MEDIAN`, `SUM`: the last 10 values |
| Policy constraints | `CooldownConstraint`, `IntervalConstraint` |
| Target constraints | `ThrashingConstraint`; `TargetGroupSizeConstraint` (the adjustment clamps the change onto `[minSize, maxSize]`, see Q9) |
| Target groups | `ElasticInfrastructure`, `ServiceGroup`, `CompetingConsumersGroup` |
| Not supported | `SimpleFireOnTrend`, `NumberOfElements`, memory, HDD and network stimuli (Slingshot has no checker for them either): a configuration error |

A model directory with a `.spd` file is loaded with it (the `.spd` and `.semanticspd` files are
read after the PCM entry files). Time triggers become events of the event list at their
expected time; measurement triggers read the tuples of the series of the monitor repository.

In the SimuLizar flavour the measurements are SimuLizar's: utilisation is the sliding-window
utilisation of the monitor, not Slingshot's instantaneous sample. There is no reference for
this combination. Slingshot's `AVERAGE` EMA multiplies its value by `exp(-Δt/60)` at every
sample, a correction for its own utilisation samples, which are taken only when a job arrives
or leaves: the time without a sample counts as idle. A sliding-window tuple already describes
its window, so for it only the time between samples that no window covers decays (none with
the usual `windowLength = windowIncrement`). Without this the EMA of a CPU busy all the time,
sampled every 10 s, settled at 52 % and an "above 80 %" policy could never fire (`spd_v4_cpu`
made no adjustment in 600 s; now 8 to 11, seeds 1 to 3). Slingshot's own monitor repositories often declare utilisation monitors
without a `TimeDriven` processing type, which the SimuLizar flavour rejects; the Slingshot
flavours (`--flavour slingshot`, `slingshot-exact`) take them, as Slingshot ignores processing
types ([Monitoring](../spec/slingshot/monitoring.md)).

## Command line

```sh
# Slingshot's minimal example with a time-triggered scale-out at t = 50 s
# (needs the submodules and a build with --features reconfiguration)
git submodule update --init && cargo run -q -p simoxide-thirdparty -- setup
cargo build --release -p simoxide-cli --features reconfiguration
./target/release/simoxide run --model reference/slingshot/models/spd_minimal \
    --flavour slingshot --max-sim-time 100 --max-measurements -1 --spd-log spd.jsonl
# the same in Slingshot's event cascade, byte-equal to the reference runner
./target/release/simoxide run --model reference/slingshot/models/spd_minimal \
    --flavour slingshot-exact --seed 1 --max-sim-time 100 --spd-log spd.jsonl
```

`corpus-spd/` lists more SPD models (`case.json` names the model directory, simulated time and
seed) with the reference runner's outputs.

| Flag | Meaning |
|---|---|
| `--spd-log FILE` | write the decision log (below) |
| `--spd-log-all` | also log the evaluations the target-group check rejected |
| `--spd-quirks fixed\|slingshot\|Q2,Q12,…` | Slingshot's quirks to reproduce (default `fixed`: none) |
| `--no-spd` | ignore the `.spd` file |

The flags need the `spd` feature of `simoxide-cli` (on by default). The adjustments run in a
build with the `reconfiguration` feature (`cargo build --release -p simoxide-cli --features
reconfiguration`); without it the run makes the decisions only and reports every adjustment
as not executed. After the run the CLI prints the number of policies, evaluations, requests
and executed adjustments (`adjusted=`).

| Flag | Meaning |
|---|---|
| `--qvto-mode auto\|ordered\|fast` | QVTo's mode for the adjustments: `auto` = `ordered`, the collection order of the Slingshot reference's `det` mode; `fast` is the throughput option (equal on every corpus case) |

At a resume, `simoxide resume FILE --model DIR --enforce POLICY,...` enforces policies (by
name or id): their adjustments are requested at the resume time, past triggers and
constraints, as Slingshot's initialisation extension enforces a state's incoming policies. They
run after the events already queued for that time and appear in the decision log with
`"enforced":true`; they change no cooldown, interval or thrashing state. `--change` and
`--transform` steps at the resume of an SPD run use the adjustments' QVTo state (one per run,
the snapshot's section `qvto`).

## Decisions

A run can stop before a policy's adjustment runs, so that the caller chooses what happens
instead: apply the recommendation, decline it, or run other actions. The snapshot holds the
recommendation un-applied; the next invocation resolves it with an explicit, ordered action list.

```sh
# pause at the first recommendation (here: a scale-out at t = 2 s)
simoxide run --model M --flavour slingshot --max-sim-time 600 \
    --pause-on-decision --snapshot d.snap --decision-file d.json
# pause condition: scaling decision 1 pending at t=2: recommended [_out2]

simoxide resume d.snap --model M --decide _out2          # apply it (= the run without decisions)
simoxide resume d.snap --model M --decide '[]'           # decline it
simoxide resume d.snap --model M --decide _scaleIn       # another action instead
simoxide resume d.snap --model M --decide '["_out2","_out2"]' --pause-on-decision \
    --snapshot d2.snap --decision-file d2.json           # twice, then pause at the next decision
```

| Flag | Meaning |
|---|---|
| `--pause-on-decision` | `run`, `resume`: pause when a policy's adjustment is about to run |
| `--decide ACTIONS` | `resume`: resolve the snapshot's pending decision with `ACTIONS` |
| `--decision-file FILE` | `run`, `resume`: write the decision the invocation resolved and the one it paused at ([below](#decision-file)) |

**When it pauses.** A request of a policy's chain (trigger, constraints and adjustor passed) is
queued for the current instant behind the events already queued for it, as Slingshot's
`ModelAdjustmentRequested`. With `--pause-on-decision` the run takes the request when it is
dispatched, before the adjustment runs (where Slingshot's MENTOR extension,
`SnapshotTriggeringBehavior`, intercepts `ModelAdjustmentRequested`), and pauses there. Every
other request queued at that moment (all of the same instant, e.g. other policies whose time
events or measurements came at the same time) joins it: one decision, its recommendations in
request order. Decisions are numbered from 1 per run (across resumes). The pause is a pause
between two events like `--pause-at` and works with them and with `--pause-when`; the flag holds
for one invocation (a resume pauses at the next decision only with the flag again). Requests of
`--enforce`, the HPA's rescales and the actions of `--decide` are not intercepted. A run that
ends at the instant of the request (its stop condition) executes it as without the flag.

**Resolving.** `--decide ACTIONS` names policies by id (the `.spd` id; or by name) as a JSON
array or a comma list; an action is the policy's adjustment of its target group. The list
replaces the recommendations: `[]` runs nothing, `[A]` runs A once, `[A, A]` twice, `[A, B]` A
then B. The actions run at the resume, at the decision's time, in place of the
recommendations, before anything else happens: after `--change` and `--transform`, before the
requests of `--enforce`. Each runs like an enforced policy, past its trigger and constraints
(inactive policies can be actions), with the size bound applied (Q9: a step beyond
`maxSize` becomes the bound, at the bound no change). The recommendations never run: a decision
is resolved once, and a snapshot taken later holds none of it. Policies can recommend again
later (a new decision). A snapshot with a pending decision does not resume without `--decide`
(an error naming the recommendation); `--decide` on a snapshot without one is an error, as is an
unknown policy (nothing runs then). The Slingshot-exact engine does not support decisions.

**Policy state.** The chain changes the policies' state when the request is made, as in every
run (Q2, decided on 4 November 2026): when the run pauses, the adjustor state (cooldown end,
scales in the cooldown, latest adjustment, scale count), the target group's last enactment (the
thrashing constraint's) and the triggers' aggregation slots (fixed Q13: consumed when the policy
fires) are already updated, whatever is decided. Neither the selection nor the application of
an action changes policy state: like Slingshot's incoming policies (`--enforce`) they bypass the
chain and only change the model, and with it the target groups' membership and size that later
evaluations see. A declined scale-out therefore still counts for its policy's cooldown, and an
alternative scale-in does not count for the thrashing constraint. With exactly the recommended
policies, in order, the continuation is the run without `--pause-on-decision`: the same
adjustments, measurements and decision log, except for the `decision` and `select` records and
the `decision` field.

**Measurements.** A decision's snapshot is an ordinary pause: its header has the measurement
cursor and the state id, and with `--measurements-scope interval` each `--decide` child of it
exports only its own tuples, from the parent's cursor, naming the parent's state
([Measurement intervals](./snapshots.md#measurement-intervals)). Tuples the actions record at
the resume are the child's. The parent's export followed by that of the child that decided the
recommendation is the uninterrupted run's `measurements.csv`.

**HPA.** The first version covers SPD policies only: the HPA controller's rescales
([Autoscaling (HPA)](./hpa.md)) run without pausing.

### Decision file

`--decision-file FILE` writes one JSON object per invocation, after the snapshot:

```json
{
  "format": "simoxide-spd-decisions",
  "version": 1,
  "resolved": {
    "decision": 1, "time": 2.0, "time_ns": 2000000000,
    "recommended": [{"policy": "_out2", "policy_name": "Scale out at 2", "target_group": "_DC8f...",
                     "group_size": 1, "adjustment": {"type": "StepAdjustment", "stepValue": 1}}],
    "selected": ["_out2", "_out2", "_out2"],
    "outcomes": [{"policy": "_out2", "policy_name": "Scale out at 2", "target_group": "_DC8f...",
                  "ok": true, "size_before": 1, "size_after": 2, "changes": 4},
                 {"policy": "_out2", ..., "size_before": 2, "size_after": 3, "changes": 4},
                 {"policy": "_out2", ..., "size_before": 3, "size_after": 3, "changes": 2}]
  },
  "pending": null
}
```

| Field | Meaning |
|---|---|
| `format`, `version` | `simoxide-spd-decisions`, `1` (a new version for incompatible changes; new fields may be added) |
| `resolved` | the decision `--decide` resolved in this invocation, else `null` |
| `pending` | the decision the invocation paused at (in its snapshot), else `null` |
| `decision` | the decision's number in the run (1, 2, ...) |
| `time`, `time_ns` | the simulation time of the decision, the requests' time: seconds (double) and nanoseconds (integer); the actions run at this time too |
| `recommended` | the recommendations in request order: `policy` (the `.spd` id), `policy_name`, `target_group` (id), `group_size` (the group's element count at the decision, `null` if unknown), `adjustment` (metamodel class as `type` and its attributes: `stepValue`, `percentageGrowthValue` and `minAdjustmentValue`, `goalValue`) |
| `selected` | the actions, policy ids in order (`[]`: declined) |
| `outcomes` | per action, in order: `policy`, `policy_name`, `target_group`, `ok` (the adjustment succeeded; `false` without the `reconfiguration` feature), `size_before`, `size_after` (the group's element count; equal when the size bound left nothing to do), `changes` (Slingshot's change count) |

The decision log (`--spd-log`) has the same in its `decision` and `select` records and the
`decision` field of the actions' `adjust` records ([Decision log](#decision-log)).

## Adjustments

Each executed request runs `MainTransformation.qvto` as Slingshot's
`SpdAdjustmentBehavior` does:

1. `Configuration.enactedPolicy` of the semantic configuration is set to the request's policy;
   the container and allocation lists are copied.
2. The transformation runs on the run's model graph with Slingshot's bindings (allocation,
   system, resource environment, repository, semantic configuration and usage model `inout`, the
   SPD `in`, every model of each metamodel) and `keepModeling = true`. Objects it creates get
   distinct, seeded ids (Slingshot's generated classes give each a UUID; the transformation
   compares ids).
3. Its changes go through the change path (as a reconfiguration rule's). On success the outcome
   is Slingshot's `ModelAdjusted`: the old, new and deleted containers, the new allocation
   contexts and the monitors Slingshot clones for the new containers (the change count of the
   log). The target groups' members are read from the changed model.

A transformation that fails (`assert fatal`) reports `ok: false`; what it changed before stays
(Slingshot transforms in place). The transformations are embedded in SimOxide (the bundle
`org.palladiosimulator.spd.semantic.transformations` of Palladio-Addons-SPD-Metamodel at a pinned
commit, EPL-2.0 like Slingshot itself) and compiled once per process.

**Scale-out** appends copies of the unit container (`<unit>_<n>`), each with a copy of the
unit's assemblies, their allocation, connectors and a load-balancer branch; the branch
probabilities become `1/n`. **Top-down** (a service group or competing consumers as the target)
adds assemblies first and containers as the placement needs them. **Scale-in** removes the
newest elements (drained, above).


## Fixed behaviour and compat flags

Where Slingshot's SPD interpreter is buggy, SimOxide fixes it by default; a flag per quirk
reproduces Slingshot (`slingshot` turns all of them on). Which behaviour is the semantics was
decided on 4 November 2026 ([Semantics decisions](../spec/slingshot/semantics-decisions.md)): rows
marked *kept* are Slingshot's behaviour, and their flag changes nothing.

| Quirk | Slingshot | Fixed (default) |
|---|---|---|
| Q1 | a disregard does not restore the adjustor state | not decided; no filter before the adjustor changes the state, so both behave the same |
| Q2 | constraint state (cooldown, last enactment) changes when the request is made | kept: a cooldown counts every adjustment made to the target, also one the size bound turns into no change, or one that fails |
| Q3 | AVERAGE is an EMA, the others use the last 10 values | kept; sliding-window samples (SimuLizar flavour) decay only over uncovered time |
| Q4 | a relative change below the minimum ignores `minAdjustmentValue` when it is not positive | 0 stays 0, otherwise at least `minAdjustmentValue` in the direction of the percentage (patched `AdjustmentCalculator.qvto`) |
| Q6 | after an adjustment every linking resource connects every container | a new container joins the links its unit is connected to, appended; other links keep their containers (patched `ResourceEnvironmentTransformation.qvto`) |
| Q7 | a top-down scale-in removes the containers it empties one enactment late | removed in the same enactment (patched `TopDownTransformationInfrastructure.qvto`) |
| Q9 | only thrashing constraints are in the chain; the adjustment clamps the change to the group size | kept: an out-of-range step is modified to the bound, not disregarded (size 2, maximum 3: step +2 becomes +1; at size 3 it becomes 0, an adjustment that still starts the cooldown, Q2) |
| Q11 | an unsupported stimulus or trigger throws at set-up: the whole SPD is dropped | a configuration error naming it |
| Q12 | composed triggers: AND resumes after a failed child, OR never evaluates its children and parks the chain, XOR inverts after the first child; two measurement stimulus kinds run the chain twice | AND / OR / XOR (exactly one) of the children's results, every child evaluated, one evaluation per event (not decided yet: every event or only the child's own) |
| Q13 | all elements of the group feed one window; any measurement reaching the checker reads it; reading consumes the EMA's emit slot | one window per measuring point, the aggregation over their values; a window starts with its element's first sample (an EMA is ready 60 s later) and is dropped when its container leaves the group; only the stimulus' metric is evaluated; the slot is consumed when the policy fires |
| Q15 | every policy of a target group receives every time event of the group | only its own |

The patched transformations are marked "PATCHED by SimOxide"; with the flag on, Slingshot's
unchanged file runs (`--spd-quirks slingshot` runs Slingshot's bundle as it is). Q8 and Q16
(the order of sets in the transformation) follow QVTo's mode: the ordered mode equals the
reference's `det` order. Q5 and Q14 (what a scale-in does to jobs; population changes) belong to the
change path (MC-8.4, MC-5.2).

## Decision log

One JSON object per line, in the format of the Slingshot reference runner's `spd.jsonl`
([Slingshot reference runner](../reference-simulator/slingshot.md)), without its event index
`e` and its `log` lines:

```json
{"k":"policy","id":"_7VIU0MAgEe6AcKIPVXJsSw","name":"Basic Point in Time Trigger","active":true,"target_group":"_DC8fM8pgEe6u3p81um7oyQ","adjustment":"StepAdjustment","trigger":"SimpleFireOnValue","chain":["TargetGroupChecker","SimulationTimeChecker","Adjustor"]}
{"k":"eval","t":50.0,"ev":"SimulationTimeReached","policy":"_7VIU0MAgEe6AcKIPVXJsSw","filters":[["TargetGroupChecker","ok"],["SimulationTimeChecker","ok"],["Adjustor","ok"]],"request":true}
{"k":"adjust","t":50.0,"policy":"_7VIU0MAgEe6AcKIPVXJsSw","policy_name":"Basic Point in Time Trigger","ok":true,"changes":4,"before":{"containers":[["AppServer","_GAeeUCpXEeKNWc2UWjbmuw"],["Loadbalancer_RC","_R8hGoLwqEe6VP9cIU1FUYw"]],"links":[["LinkingResource1","AppServer","Loadbalancer_RC"]],"allocation":[...],"target_groups":[...]},"after":{"containers":[["AppServer","_GAeeUCpXEeKNWc2UWjbmuw"],["AppServer_1","~ResourceContainer:AppServer_1"],["Loadbalancer_RC","_R8hGoLwqEe6VP9cIU1FUYw"]],...},"size_before":1,"size_after":2}
```

| `k` | Written | Fields |
|---|---|---|
| `policy` | when the policy's handler runs the first time | id, name, target group, adjustment and trigger class, the chain's filter classes |
| `eval` | per evaluation (not those the target-group check rejects, unless `--spd-log-all`) | time, event (`SimulationTimeReached`, `MeasurementMade`), the filters that ran with `ok`, `disregard` and Slingshot's reason, `null` or `exception`, and whether a request came out |
| `adjust` | when a request is executed | time, policy, success; Slingshot's change count (`changes`: the environment and allocation changes and one per cloned monitor) and the model summaries `before` and `after` (the containers with their ids, `~Class:name` for created ones; the links with their containers; the allocation contexts; the target-group configurations' elements; each in model order, as the reference runner's); the target group's size before and after; `enforced` for an enforced policy; `decision` (its number) for an action of `--decide`. Without the `reconfiguration` feature nothing is executed: `ok` is `false` |
| `decision` | when `--pause-on-decision` pauses ([Decisions](#decisions)) | time `t`, `decision` (its number), `recommended`: the policy ids in request order; they are not executed |
| `select` | when `--decide` resolves the decision | time `t`, `decision`, `selected`: the policy ids of the actions in order; their `adjust` records follow |

On the time-triggered models of the Slingshot reference runner (`corpus-spd/`), the decision
log with `--spd-quirks slingshot` equals the runner's `spd.jsonl`, the adjustments' change
counts and model summaries included, see [Testing](../correctness/testing.md).

## Library

`simoxide-spd` keeps the interpreter independent of the simulator:

- `SpdModel::from_graph(&graph)`: the typed view of the `.spd` model;
- `Spd::new(model, quirks)`: the interpreter (immutable, shared); `Spd::state(membership)` its
  plain-data `SpdState`, which forks and snapshots copy;
- the host implements `SpdHost` (current time, schedule a time event, queue a request) and
  calls `Spd::on_timer`, `Spd::on_measurement` and `Spd::adjusted`;
- `simoxide_spd::sim::SpdRun` is that host for `simoxide-sim`: it observes the series the
  triggers read, schedules the time events as external notes of the event list, queues requests
  at the current time and hands them to an `Adjuster` (`simoxide-reconf`'s
  `QvtoAdjuster::spd`: Slingshot's adjustment; `NoAdjustment`: decisions only). `run_until`
  and `fork` pause and fork the run with its SPD state; `enforce_policies` queues enforced
  requests; `snapshot_value` and `restore_snapshot` write and read it (with the adjuster's
  QVTo state: `QvtoRun::snapshot_section`, `restore_section`). It runs both flavours.
- `simoxide_spd::decision`: `SpdRun::set_pause_on_decision`, `pending_decision`
  (`PendingDecision` with its `Recommendation`s) and `decide` (a `Resolution` with the
  `ActionOutcome`s); `report` is the decision file, `parse_actions` reads `--decide`'s list.
- `simoxide_spd::adjusted`: Slingshot's `ModelAdjusted` and the model summary, for other hosts.

```rust
use simoxide_reconf::{QvtoAdjuster, QvtoCore, SpdAdjust};
use simoxide_spd::{Quirks, log::VecLog, sim::{SpdRun, spd_of}};
use simoxide_sim::{CompiledModel, Outputs, RunSpec, Simulation};
let spec = RunSpec::default();
let model = spec.load_model("model-dir".as_ref())?;
let spd = spd_of(&model, Quirks::fixed())?.expect("the model has an SPD");
let cm = CompiledModel::compile(model)?;
let mut cfg = spec.sim_config("run");
cfg.max_sim_time = 100;
let mut run = SpdRun::new(Simulation::new(&cm, cfg, Outputs::default())?, &spd)?;
// one core per process: it compiles the embedded transformations once
let core = QvtoCore::new()?;
let mut adjuster = QvtoAdjuster::spd(&core, &SpdAdjust::new(&spd.quirks()))?;
let mut log = VecLog::default();
run.run(&mut adjuster, &mut log)?;
print!("{}", log.to_jsonl());
```

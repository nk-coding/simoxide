# Scaling policies (SPD)

SimOxide reads Slingshot's scaling policy definitions (`.spd`, with the semantic configuration
`.semanticspd`) and evaluates them during a run, as Slingshot's SPD interpreter does: per
policy a filter chain of target-group check, trigger, constraints and adjustor. The semantics
are on [Scaling policies (Slingshot)](../spec/slingshot/spd.md); the design is
`DESIGN-SPD.md` §5.4.

::: warning Decisions only
This is phase P1. A policy that fires produces an *adjustment request* (policy, target group,
adjustment) and the decision log records it, but the adjustment itself (Slingshot's
`MainTransformation.qvto`, phase P2) is not executed yet: the model does not change. The
command line runs no adjustment; as a library, `simoxide-reconf`'s `QvtoAdjuster` runs a
transformation for each request on the run's model ([QVTo transformations on a
run](library.md#qvto-transformations-on-a-run)), with hooks for what P2 adds (the enacted
policy, Slingshot's model summaries).
:::

## What is supported

| Part | Support |
|---|---|
| Policies | active policies in model order; inactive ones are skipped |
| Triggers | `SimpleFireOnValue` on `SimulationTime`, `OperationResponseTime`, `CPUUtilization`, `TaskCount`, `QueueLength`; `ComposedTrigger` (AND, OR, XOR) |
| Expectations | `ExpectedTime`, `ExpectedPercentage` (compared as `value / 100`), `ExpectedCount`, `NoExpectation` |
| Aggregation | `AVERAGE`: exponential moving average (60 s, emit every 10 s, α = 0.2); `MAX`, `MIN`, `MEDIAN`, `SUM`: the last 10 values |
| Policy constraints | `CooldownConstraint`, `IntervalConstraint` |
| Target constraints | `ThrashingConstraint`; `TargetGroupSizeConstraint` (checked in the chain by default, see Q9) |
| Target groups | `ElasticInfrastructure`, `ServiceGroup`, `CompetingConsumersGroup` |
| Not supported | `SimpleFireOnTrend`, `NumberOfElements`, memory, HDD and network stimuli (Slingshot has no checker for them either): a configuration error |

A model directory with a `.spd` file is loaded with it (the `.spd` and `.semanticspd` files are
read after the PCM entry files). Time triggers become events of the event list at their
expected time; measurement triggers read the tuples of the series of the monitor repository.

In the SimuLizar flavour the measurements are SimuLizar's: utilisation is the sliding-window
utilisation of the monitor, not Slingshot's instantaneous sample. There is no reference for
this combination. Slingshot's own monitor repositories often declare utilisation monitors
without a `TimeDriven` processing type, which the SimuLizar flavour rejects; the Slingshot
flavour (phase S2) will take them.

## Command line

```text
simoxide run --model DIR --max-sim-time 100 --spd-log spd.jsonl
```

| Flag | Meaning |
|---|---|
| `--spd-log FILE` | write the decision log (below) |
| `--spd-log-all` | also log the evaluations the target-group check rejected |
| `--spd-quirks fixed\|slingshot\|Q2,Q12,…` | Slingshot's quirks to reproduce (default `fixed`: none) |
| `--no-spd` | ignore the `.spd` file |

The flags need the `spd` feature of `simoxide-cli` (on by default). After the run the CLI
prints the number of policies, evaluations and requests.

## Fixed behaviour and compat flags

Where Slingshot's SPD interpreter is buggy, SimOxide fixes it by default; a flag per quirk
reproduces Slingshot (`slingshot` turns all of them on). Rows marked *provisional* implement a
proposal that is not confirmed yet.

| Quirk | Slingshot | Fixed (default) |
|---|---|---|
| Q1 | a disregard does not restore the adjustor state | restored; no filter before the adjustor changes the state, so both behave the same |
| Q2 | constraint state (cooldown, last enactment) changes when the request is made | changes only when the adjustment succeeds |
| Q3 | AVERAGE is an EMA, the others use the last 10 values | kept (*provisional*) |
| Q4 | relative 0 % ignores `minAdjustmentValue` | 0 stays 0, otherwise at least `minAdjustmentValue` in the direction of the percentage (*provisional*; used by the Q9 check) |
| Q9 | only thrashing constraints are in the chain | group-size constraints too: a policy whose change would be clamped to zero is disregarded (*provisional*) |
| Q11 | an unsupported stimulus or trigger throws at set-up: the whole SPD is dropped | a configuration error naming it |
| Q12 | composed triggers: AND resumes after a failed child, OR never evaluates its children and parks the chain, XOR inverts after the first child; two measurement stimulus kinds run the chain twice | AND / OR / XOR (exactly one) of the children's results, every child evaluated, one evaluation per event |
| Q13 | all elements of the group feed one window; any measurement reaching the checker reads it; reading consumes the EMA's emit slot | one window per measuring point, the aggregation over their values; only the stimulus' metric is evaluated; the slot is consumed when the policy fires (*provisional*) |
| Q15 | every policy of a target group receives every time event of the group | only its own |

The quirks of the adjustment (Q5-Q8, Q14, Q16) belong to phase P2.

## Decision log

One JSON object per line, in the format of the Slingshot reference runner's `spd.jsonl`
([Slingshot reference runner](../reference-simulator/slingshot.md)), without its event index
`e` and its `log` lines:

```json
{"k":"policy","id":"_7VIU0MAgEe6AcKIPVXJsSw","name":"Basic Point in Time Trigger","active":true,"target_group":"_DC8fM8pgEe6u3p81um7oyQ","adjustment":"StepAdjustment","trigger":"SimpleFireOnValue","chain":["TargetGroupChecker","SimulationTimeChecker","Adjustor"]}
{"k":"eval","t":50.0,"ev":"SimulationTimeReached","policy":"_7VIU0MAgEe6AcKIPVXJsSw","filters":[["TargetGroupChecker","ok"],["SimulationTimeChecker","ok"],["Adjustor","ok"]],"request":true}
{"k":"adjust","t":50.0,"policy":"_7VIU0MAgEe6AcKIPVXJsSw","policy_name":"Basic Point in Time Trigger","ok":false,"size_before":1,"size_after":1}
```

| `k` | Written | Fields |
|---|---|---|
| `policy` | when the policy's handler runs the first time | id, name, target group, adjustment and trigger class, the chain's filter classes |
| `eval` | per evaluation (not those the target-group check rejects, unless `--spd-log-all`) | time, event (`SimulationTimeReached`, `MeasurementMade`), the filters that ran with `ok`, `disregard` and Slingshot's reason, `null` or `exception`, and whether a request came out |
| `adjust` | when a request is executed | time, policy, success, the target group's size before and after. In P1 nothing is executed: `ok` is `false` |

On the time-triggered models of the Slingshot reference runner, the decision log with
`--spd-quirks slingshot` equals the runner's `spd.jsonl` (projected), see
[Testing](../correctness/testing.md).

## Library

`simoxide-spd` keeps the interpreter independent of the simulator:

- `SpdModel::from_graph(&graph)`: the typed view of the `.spd` model;
- `Spd::new(model, quirks)`: the interpreter (immutable, shared); `Spd::state(membership)` its
  plain-data `SpdState`, which forks and snapshots copy;
- the host implements `SpdHost` (current time, schedule a time event, queue a request) and
  calls `Spd::on_timer`, `Spd::on_measurement` and `Spd::adjusted`;
- `simoxide_spd::sim::SpdRun` is that host for `simoxide-sim`: it observes the series the
  triggers read, schedules the time events as external notes of the event list, queues requests
  at the current time and hands them to an `Adjuster` (P1: `NoAdjustment`). `run_until` and
  `fork` pause and fork the run with its SPD state.

```rust
use simoxide_spd::{Quirks, log::VecLog, sim::{NoAdjustment, SpdRun, spd_of}};
use simoxide_sim::{CompiledModel, Outputs, RunSpec, Simulation};
let spec = RunSpec::default();
let model = spec.load_model("model-dir".as_ref())?;
let spd = spd_of(&model, Quirks::fixed())?.expect("the model has an SPD");
let cm = CompiledModel::compile(model)?;
let mut cfg = spec.sim_config("run");
cfg.max_sim_time = 100;
let mut run = SpdRun::new(Simulation::new(&cm, cfg, Outputs::default())?, &spd)?;
let mut log = VecLog::default();
run.run(&mut NoAdjustment, &mut log)?;
print!("{}", log.to_jsonl());
```

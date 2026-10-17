# Reconfiguration rules (SimuLizar)

SimuLizar adapts a running system with *reconfiguration rules*: QVTo transformations that run
whenever a runtime measurement changes and that edit the PCM models in place. SimOxide runs the
same rules, with SimuLizar's timing and result, on QVTo's Rust engine. The rules are specified in
[Model changes](../spec/model-changes.md#mc-7-reconfiguration-rules-qvto) (MC-7), the runtime
measurements they read and the reconfiguration process in
[Measurements](../spec/measurements.md) (MEAS-7.2, 7.3, 7.5); the design is `DESIGN-SPD.md` R1.

## Running rules

```text
simoxide run --model corpus-rules/lb_outsource --rules corpus-rules/lb_outsource/rules \
    --rules-log rules.jsonl --measurements m.csv
```

or, in the model's `run.json`, `"reconfiguration_rules": "rules"` (relative to the model
directory). Every `.qvto` file of the folder is a rule; they run in file-name order.

| Flag | Meaning |
|---|---|
| `--rules DIR` | the rules folder (overrides `run.json`) |
| `--rules-log FILE` | one JSON line per run of the reconfiguration process (below) |
| `--qvto-mode auto\|fast\|compat` | QVTo's mode (default `auto`: compat in builds with `exact` or `compat`, else fast) |

The flags need `simoxide-cli`'s cargo feature `reconfiguration`:

```sh
cargo build --release -p simoxide-cli --features reconfiguration          # EPL build, QVTo fast mode
cargo build --release -p simoxide-cli --features reconfiguration,exact,hotspot-math   # local exact build
```

After the run the CLI prints the number of rules, of reconfigurations, of successful ones, of
those that changed the model, and the time spent in the rules.

## What happens during a run

1. The monitors whose measurement specifications have `triggersSelfAdaptations` (the EMF default
   is `true`) write their values into the *runtime measurement model* (PRM): feed-through values,
   fixed- and variable-size aggregations, time-driven aggregations of a window and sliding-window
   utilisation, computed as SimuLizar computes them (MEAS-7.5).
2. A write at a later time than the last reconfiguration schedules the reconfiguration process at
   the current time; when it runs, every rule runs, each on the models as the rules before it
   left them. A rule's parameters are bound by metamodel: the PRM's measurements, the PCM models,
   the monitor repository.
3. The reconfiguration succeeds when every rule *applied*: a rule that completes applies (also when
   an `assert error` failed), one that ends in `assert fatal` or an exception does not; what it
   changed until then stays. A successful run records a `Reconfiguration Time` tuple (0 s) and, if
   the number of resource containers changed, a `Number of Resource Containers` tuple.
4. The changes reach the simulation as SimuLizar's syncers would carry them: processing rates, link
   latency and throughput, demands, branch probabilities and other StoEx, new resource containers
   with their resources, linking-resource connections, new allocation contexts, connectors and
   assembly contexts ([Model changes](../spec/model-changes.md)).

Changes that SimuLizar aborts on (removing an allocation context, `move` of a link list) end the
run with an error, as in SimuLizar. Changes SimOxide does not support yet end the run with an error
that names the rule: monitor changes, population and usage-scenario changes, structural changes of
a behaviour (an action or branch added to a SEFF), an assembly context without an allocation.

## Examples

`corpus-rules/` holds SimuLizar's reconfiguration examples (`reference/import-rules-examples.sh`)
with the outputs of the patched SimuLizar reference. In SimOxide's exact build all of them equal
the reference byte for byte (trace, random tape, measurements):

| Case | Model and rules | Changes |
|---|---|---|
| `lb_outsource` | the load balancer example as its launch configuration runs it | branch probabilities |
| `lb_rules_qvto` | the load balancer with `reconfigurations_qvto` (outsource, scaleDown, scaleIn, scaleUp) | branch probabilities, processing rates |
| `lb_scaleout` | the load balancer with outsource and SimOxide's `scaleOut.qvto` | a new resource container with resources, on the LAN; a new assembly context and allocation context; a rewired connector |
| `power_rules_qvto` | the power-consumption variant with its PCM-only rules (power monitors removed: the power extension is not part of the 5.2.2 product) | processing rates, branch probabilities |
| `sl_reconfiguration` | SimuLizar's `ReconfigurationSimulationTest`: the n-th user's response time is n | a resource demand |

Not runnable (also not in SimuLizar 5.2.2 headless): rules that use SimuLizar's action framework
(`AdaptationBehavior.execute`: the power example's `switchToStandby`, the media store's horizontal
scaling, the power example's `outsource`), examples with PCM 5.0 or 5.1 namespaces (`znn.com`,
trip planner, the old power-consumption example), Henshin and Story Diagram rules.

## The rules log

`--rules-log FILE` writes one line per run of the reconfiguration process:

```json
{"t":20,"post_stop":false,"succeeded":true,"rules":[{"rule":"outsource","applied":true,"changed":false,"log":"SimuLizar Outsource Rule Transformation started\n…"}]}
```

`t` is the simulation time (s), `post_stop` marks a run triggered after the stop (inside the
finalisation), `log` holds the rule's QVTo `log(...)` output.

## Library

```rust
use simoxide_reconf::{QvtoCore, RuleSet, RulesHost, RulesRun, RunConfig};

let core = QvtoCore::new()?;                                   // once per process
let rules = RuleSet::load_dir(&core, Path::new("rules"))?;     // compiled once, shareable
let host = RulesHost::new(&core, rules, RunConfig::default())?;
let sim = Simulation::new(&cm, cfg, Outputs::default())?;
let mut run = RulesRun::new(sim, host);                        // attaches the host
run.run_until(Pause::After(300_000_000_000))?;                 // rules run on the way
let fork = run.fork(Outputs::default())?;                      // PRM values and QVTo state included
let (result, host) = run.finish()?;
println!("{:?}", host.stats());
```

- `RulesRun` pauses, forks (`fork`), and splits into the simulation and its host (`into_parts`,
  `from_parts`): a snapshot is the run state plus a clone of the host, and a detached state
  continues on another thread with its host ([Pause, fork and resume](library.md#pause-fork-and-resume)).
- `RulesHost::keep_records` keeps a `ReconfRecord` per run (the rules log above).
- Underneath, the simulator pauses between events whenever its reconfiguration process runs
  (`Simulation::attach_reconfiguration`, `RunState::reconf_pending`, `Simulation::reconf_due`,
  `Simulation::prm_measurements`, `Simulation::reconf_done`); `RulesHost::handle` is one such run.

## Cost

A run without rules is unchanged (the host is attached only with rules). With rules, the PRM
mirror costs about 15 µs per reconfiguration; a rule run about 0.1 ms in QVTo's fast mode and about
1 ms in compat mode on the load balancer example (the compat mode's per-run set-up).

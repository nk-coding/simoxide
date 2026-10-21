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
| `--qvto-mode auto\|ordered\|fast\|compat` | QVTo's mode (default `auto`: ordered, below) |

The flags need `simoxide-cli`'s cargo feature `reconfiguration`:

```sh
cargo build --release -p simoxide-cli --features reconfiguration          # EPL build, QVTo fast mode
cargo build --release -p simoxide-cli --features reconfiguration,exact,hotspot-math   # local exact build
```

QVTo's modes for rules: `ordered` (the default) iterates QVTo's and OCL's sets in insertion order,
as the reference does (patch P10), and is part of every build; `fast` is the same with plain
insertion-ordered sets (classic OCL's OrderedSet quirks left out); `compat` iterates in Java
`HashMap` order with identity hashes from HotSpot's generator (not those of a Java run), needs the
`compat` or `exact` feature and differs from the reference on rules that iterate a `Set` of model
objects (`corpus-rules/lb_scaleout_each`).

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
4. Objects a rule creates get ids as generated PCM code gives them, from a seeded generator of the
   run (`RunConfig::ids`, the same sequence as the reference's): the n-th object created by rules in
   a run gets the same id in every run and in the reference, also when the rule sets the id itself
   or the object is a `deepclone()` copy (each copy draws an id, then takes the original's).
5. The changes reach the simulation as SimuLizar's syncers would carry them: processing rates, link
   latency and throughput, demands, branch probabilities and other StoEx, new resource containers
   with their resources, linking-resource connections, new allocation contexts, connectors and
   assembly contexts ([Model changes](../spec/model-changes.md)).

Every change kind is taken (population and usage scenarios, monitors as SimuLizar attaches them,
structural changes of behaviours with executions in progress following the changed chain). With
SimuLizar's removal (`ChangeOptions::simulizar()`, `removal keep`), changes that SimuLizar aborts on
(removing an allocation context, `move` of a link list) end the run with an error, as in
SimuLizar; by default removed containers drain. A change SimOxide refuses ends the run with an
error that names the rule: an assembly context without an allocation, kept instructions that no
longer fit a changed action ([Model changes](../spec/model-changes.md) MC-4.3, MC-8).

## Examples

`corpus-rules/` holds SimuLizar's reconfiguration examples and SimOxide's own
(`reference/import-rules-examples.sh`) with the outputs of the patched SimuLizar reference. In
SimOxide's exact build (`simoxide-sim`'s `exact` and `hotspot-math`; the QVTo side needs no opt-in
crate since R2) all of them equal the reference byte for byte (trace, random tape, measurements):

| Case | Model and rules | Changes |
|---|---|---|
| `lb_outsource` | the load balancer example as its launch configuration runs it | branch probabilities |
| `lb_rules_qvto` | the load balancer with `reconfigurations_qvto` (outsource, scaleDown, scaleIn, scaleUp) | branch probabilities, processing rates |
| `lb_scaleout` | the load balancer with outsource and SimOxide's `scaleOut.qvto` | a new resource container with resources, on the LAN; a new assembly context and allocation context; a rewired connector |
| `lb_add_branch` | the load balancer with SimOxide's `addBranch.qvto` | a branch added to the load balancer's SEFF: new requests take it |
| `power_rules_qvto` | the power-consumption variant with its PCM-only rules (power monitors removed: the power extension is not part of the 5.2.2 product) | processing rates, branch probabilities |
| `sl_reconfiguration` | SimuLizar's `ReconfigurationSimulationTest`: the n-th user's response time is n | a resource demand |
| `lb_scaleout_noid` | `lb_scaleout` without explicit ids | as `lb_scaleout`; the new objects' ids from the seeded generator |
| `lb_add_branch_noid` | `lb_add_branch` without explicit ids | as `lb_add_branch` |
| `lb_scaleout_each` | outsource and `scaleOutEach.qvto`: every server moves to a new node, in the order of a `Set` of allocation contexts | two containers, assembly and allocation contexts, rewired connectors; their ids and order follow the `Set` |
| `lb_wire_new_server` | SimuLizar's horizontal-scaling adaptation steps (`instantiatecomponent.qvto`, `wirewithloadbalancer.qvto` of its action repository) as one rule | a container, an assembly and allocation context, a required role, a branch with its actions, a connector; no explicit ids |
| `lb_clone_branch` | `cloneBranch.qvto`: a branch behaviour copied with `deepclone()` becomes a third branch | a branch whose copied actions keep their originals' ids |

The default (EPL) build runs the fast simulator mode and QVTo's ordered mode: its rules make the
same decisions and its new objects get the same ids as the reference's for the same runtime
measurements, but the fast simulator mode rounds differently, so its runs are not byte-equal. It
matches the reference statistically (`reference/tools/rules-stat.sh`, 30 seeds per side): no
per-run statistic differs, except Server2's mean response time in the scale-out cases
(`lb_scaleout`, `lb_scaleout_noid`, `lb_scaleout_each`) with seeds 1-30, which seeds 31-60 do not
reproduce (the scale-out time is heavy-tailed). The exact build equals the reference on all 30
seeds of every case.

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
mirror costs about 8 µs per reconfiguration and a rule evaluation about 80 µs, in QVTo's ordered
(default), fast and compat modes alike: the power example's 13 349 reconfigurations (2 000 s, seed
1) take 1.1 s of rules in ordered and in compat mode (measured 2026-10-21 on a quiet 25-core VM,
[Engine performance](../performance/engine.md#runtime-changes-spd-and-snapshots); the 42 s of compat
mode measured earlier no longer occur).

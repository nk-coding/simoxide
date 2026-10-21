# Snapshots

A snapshot is a paused run written to a file: pause a simulation between two events, write it,
and continue it later, in another process or on another machine, possibly after changing the
model. The continuation is exactly the uninterrupted run: the same trace lines, tape records,
measurements, event and draw counts, byte for byte. This holds in exact and fast mode, in the
SimuLizar and the Slingshot flavour, after model changes, with scaling policies (SPD) and with
SimuLizar's reconfiguration rules. The file layout is on [Snapshot format](../formats/snapshot.md).

Snapshots on disk extend [pause, fork and resume](./library.md#pause-fork-and-resume): a fork is
a copy in memory, a snapshot is the same state as bytes.

## Command line

```sh
# run to t = 120 s, write the snapshot, stop
simoxide run --model corpus/x_sl_mediastore --trace a.jsonl \
    --pause-at 120 --snapshot ms-120.snap

# continue it to the end (outputs get the continuation only)
simoxide resume ms-120.snap --model corpus/x_sl_mediastore \
    --trace b.jsonl --measurements m.csv

# or to the next pause, with a new snapshot
simoxide resume ms-120.snap --model corpus/x_sl_mediastore \
    --pause-at 300 --snapshot ms-300.snap

simoxide snapshot-info ms-300.snap   # the header (JSON)
```

- Pauses: `--pause-at T` (every event at or before `T` seconds has run), `--pause-before T`
  (every event before `T`), `--pause-events N` (at least `N` events). `simoxide resume` also
  takes `--until T`, the same as `--pause-at T`. A paused run reports `paused t=... events=...`
  and writes the measurements recorded so far to `--measurements`.
- `--snapshot-format json` writes the sections as JSON (for reading; see the format page).
- `resume` needs the model the run was created on (`--model`, `--run-json`): the snapshot names
  it by a fingerprint of its content and refuses another model. The mode (`exact`, `fast`), the
  flavour and its compat flags come from the snapshot; the seed, stop conditions and limits are
  part of the run state. `--timeout` applies to the continuation.
- Outputs: the trace, tape, decision log (`--spd-log`) and rules log (`--rules-log`) of the paused
  run and those of the resumed run concatenate to the outputs of the uninterrupted run. A run
  started with `--trace` must be resumed with one (or its lines are discarded); a tape can be
  added or dropped at any pause (exact mode).
- Scaling policies: a run with a `.spd` resumes with its interpreter state; the compat flags of
  `--spd-quirks` are kept in the snapshot and used again. HPA autoscalers (section `hpa`) need
  the same configuration at the resume and can be changed with `--hpa-set`
  ([Autoscaling (HPA)](./hpa.md#command-line)); a load profile and its position are part of the
  run state ([Load profiles](./load-profile.md)).
- Reconfiguration rules (`reconfiguration` feature): resume with the same rules (`--rules DIR` or
  the model's `run.json`); the rules' names are checked.
- The Slingshot-exact engine (`--flavour slingshot-exact`) pauses, snapshots and resumes the
  same way, with or without scaling policies (below, "The Slingshot-exact engine").

## Library

```rust
use simoxide_sim::{Outputs, Pause, Simulation};

let mut sim = Simulation::new(&cm, cfg, Outputs::default())?;
sim.run_until(Pause::After(120_000_000_000))?;
let mut file = std::fs::File::create("ms-120.snap")?;
let info = sim.snapshot_to(&mut file)?;            // postcard, between events
println!("{} bytes at t = {} ns", info.bytes, info.header.time_ns);

// later, in another process: the same compiled model
let f = std::fs::File::open("ms-120.snap")?;
let sim = Simulation::<simoxide_sim::Exact>::restore_from(f, &cm, Outputs::default())?;
let result = sim.finish()?;
```

- `Simulation::snapshot_to(w)` and `snapshot_with(w, &SnapshotOptions { encoding }, sections)`
  write the run (flushing its outputs first, as `Simulation::snapshot` does);
  `snapshot_value(enc, sections)` returns it as a `Snapshot` (header and sections) to add
  sections of one's own. `RunState::serialize(&base, enc)` and `RunState::deserialize(&s, &base)`
  work on a state alone.
- `Simulation::restore_from(r, &base, outputs)` and `restore_snapshot(&s, &base, outputs)` check
  the format version, the SimOxide version, the mode, the flavour and the model fingerprint
  (`SnapshotError::{Version, SimOxideVersion, Mismatch, Model}`; damage is
  `SnapshotError::Corrupt`) and continue on `base`, the compiled model the run was created on.
  The type parameters choose mode and flavour as in `Simulation::create`.
- After model changes (`Simulation::apply_changes`, `simoxide_reconf::Edit`, QVTo) the snapshot
  carries the run's own model: its graph as XMI, the build order and the compiled tables, so that
  every index of the run state keeps its meaning. `restore_from` still takes the base model; the
  restored run continues on its own model, and further changes apply to it.
- Snapshots are deterministic: the same state gives the same bytes, and a restored state writes
  the bytes it was read from.
- Not part of a snapshot: the output writers, the wall-clock deadline and the cancellation flag
  of `Limits` (set them again with `Simulation::set_limits`).

### Scaling policies and reconfiguration rules

```rust
// SPD: the interpreter's state (section `spd`); the interpreter is built from the model again
run.snapshot_to(&mut file)?;                       // SpdRun, after run_until
let run = SpdRun::<Exact>::restore_from(f, &cm, &spd, Outputs::default())?;

// SimuLizar's rules: the host's state (section `rules`); a new host with the same rules
rules_run.snapshot_to(&mut file)?;                 // RulesRun, after run_until
let host = RulesHost::new(&core, RuleSet::load_dir(&core, dir)?, cfg)?;
let rules_run = RulesRun::<Exact>::restore_from(f, &cm, host, Outputs::default())?;
```

- `SpdRun` (feature `snapshot` of `simoxide-spd`) keeps the policies' adjustor, trigger and
  aggregation states, the target groups, the membership, the queued adjustment requests and the
  counters. Restoring checks that the interpreter has the same policies and compat flags
  (`simoxide_spd::snapshot::quirks_of` reads the flags from a snapshot).
- `RulesRun` (feature `snapshot` of `simoxide-reconf`) keeps the host's QVTo state, the PRM mirror
  (rebuilt with the same objects), the statistics and kept records. The QVTo state of other hosts
  (e.g. an SPD run's `QvtoAdjuster.run`) goes into a snapshot with
  `QvtoRun::snapshot_section(enc)` and comes back with `QvtoRun::restore_section(&sim, &s)`: the
  id generator continues, so objects created after the restore get the ids they would have got.

## Pause conditions

A run can also pause when a condition holds, not only at a time: `--pause-when COND` (repeatable;
any condition that holds pauses, together with `--pause-at` and the like). The run stops between
two events, right after the event that made the condition hold; with `--snapshot FILE` the paused
run is written as above, and the CLI prints `pause condition: ...` with what held.

| Condition | Meaning |
|---|---|
| `time T`, `before T`, `events N` | as `--pause-at`, `--pause-before`, `--pause-events` |
| `elapsed T`, `elapsed-events N` | `T` seconds or `N` events after the start of this run or resume (counted from the time of the last event before the pause) |
| `METRIC[POINT] OP V` | a tuple of a selected series compares with `V` (`OP`: `>`, `>=`, `<`, `<=`) |
| `METRIC[POINT] mean W OP V` | the mean of the tuples of the last `W` seconds (per series) compares with `V` |
| `... for W` | the comparison held for `W` seconds on every tuple of that series (pauses before the first event at or after the time it started to hold plus `W`) |
| `slo METRIC[POINT] upper U [lower L] [sensitivity S] [delay D]` | closeness to a service-level objective, as Slingshot's `SnapshotSLOTriggeringBehavior`: with `m = (U - L) / 2` (`L` = 0 if absent) a value `>= U - S*m`, or with `lower` a value `<= L + S*m`, pauses; not before `D` seconds |

- `METRIC`: `rt` (response time), `util` (utilisation), `state` (state of an active resource: the
  queue length), `demand`, `waiting`, `holding`, `passive` (state of a passive resource),
  `containers`, a metric name of `measurements.csv` (or its beginning, case-insensitive), or `*`.
- `POINT`: a measuring-point key of `measurements.csv`, a part of it (an element's id), the name
  of a model element (it selects the keys with its id or the id of an element it contains, e.g. a
  container's processing resources), or `*`. A condition reads every series that matches, also
  series created later in the run.

```sh
simoxide run --model corpus/x_pem_screencast_ms --max-measurements -1 --max-sim-time 20000 \
    --pause-when 'rt[RealisticUsageScenario] > 5000' \
    --pause-when 'state[DatabaseServer] >= 3 for 60' --snapshot hot.snap
simoxide resume hot.snap --model corpus/x_pem_screencast_ms --pause-when 'elapsed 600' \
    --snapshot later.snap
```

Series conditions read the stored measurement rows (`store_measurements`, on by default). A
condition changes nothing in the run: a run paused by a condition is the run paused by
`--pause-events` at the same event count, and its snapshot has the same bytes.

## Resume with changes

`simoxide resume` can change the model between the restore and the continuation (needs the
`reconfiguration` feature of `simoxide-cli`):

```sh
simoxide resume ms-120.snap --model corpus/x_sl_mediastore \
    --change faster-db.changes --transform scale-out.qvto --until 600 --snapshot ms-600.snap
```

- `--change FILE`: a change file (below), applied as one change set.
- `--transform X.qvto`: a QVTo transformation, run once on the run's models (bound as SimuLizar
  binds rules: every model parameter by its metamodel). A transformation that does not complete
  (`assert fatal`, an exception) is an error and changes nothing. Its QVTo state (the id
  generator) goes into the next snapshot (section `qvto`), so a transformation at a later resume
  gives new objects the ids the uninterrupted run would give them.
- The steps run in the order given. Each prints what the change path did
  (`change file: generation 1, rate of ... -> ...`); changes SimuLizar's syncers ignore are
  warnings.
- `--rules DIR`: on a snapshot of a run with rules, other rules than the snapshot's replace them
  (`rules replaced: [...] -> [...]`; the QVTo state, the runtime measurement mirror and the
  statistics continue); on a snapshot without rules, the rules are added from the resume on.
- `--enforce POLICY,...`: enforce scaling policies at the resume, past their triggers and
  constraints (Slingshot's "incoming policies"), by name or id. Their adjustments run at the
  resume time, after the events already queued for it, and appear in the decision log with
  `"enforced":true` ([Scaling policies](./spd.md#command-line)). Not in the Slingshot-exact
  engine.
- The result is deterministic: the same snapshot and the same changes give the same
  continuation, byte for byte. It equals the uninterrupted run that applies the same changes at
  the pause (`Simulation::apply_changes`, `QvtoRun::invoke`, `RulesHost::replace_rules`).

Changes the run cannot take are errors that leave it unchanged:

- unsupported (`unsupported model change`): kept instructions of executions in progress that
  no longer fit their changed action (MC-4.3), a new container with a registered id;
- SimuLizar aborts on them (`SimuLizar aborts on this model change`), under `removal keep` only:
  removing an allocation context, `UNSET` or `MOVE` in the allocation or in the link lists;
- invalid (`invalid model change`): the changed model does not validate or compile (e.g. an
  assembly context left without allocation).

Changes SimuLizar's syncers ignore are applied to the model but do not change the simulation
(a warning): the number of replicas (cores), the scheduling policy and the type of a processing
resource, HDD read and write rates. A removed container drains by default (`removal keep`:
it keeps simulating as in SimuLizar; [Model changes](../spec/model-changes.md) MC-8).

### Change files

One change per line; `#` starts a comment line. An element (`REF`) is named by its id or, if no
element has that id, by its name (`entityName`), which must be unique among the elements of the
kind the change expects (assembly contexts for `allocate`, containers for `add-container`, ...);
a name with blanks is written in double quotes. `REF:TYPE` selects a container's processing
resource or an internal action's demand by resource type (`CPU`, `HDD`, `DELAY`, or the type's
id). The value of a value change (a StoEx or a number) is the rest of the line, as written. All
lines work on one copy of the run's model (later lines can name elements earlier lines created)
and are applied together; on any error nothing changes.

| Change | Effect |
|---|---|
| `rate REF[:TYPE] STOEX` | processing rate of a processing resource (or of a container's resource) |
| `latency LINK STOEX`, `throughput LINK STOEX` | of a linking resource |
| `demand ACTION[:TYPE] STOEX` | a resource demand of an internal action |
| `probability TRANSITION P` | probability of a branch transition (SEFF or usage model) |
| `think-time WORKLOAD STOEX` | think time of a closed workload (or of a usage scenario's) |
| `inter-arrival WORKLOAD STOEX` | inter-arrival time of an open workload (or of a usage scenario's) |
| `arrival-rate WORKLOAD RATE` | inter-arrival time `Exp(RATE)` |
| `population WORKLOAD N` | population of a closed workload: new users start at once, the oldest leave after their run (MC-5.2) |
| `replicas REF[:TYPE] N` | number of replicas of a processing resource (ignored by the syncers, MC-1.4) |
| `loop-count LOOP STOEX` | iteration count of a SEFF loop or a usage-model loop |
| `set REF FEATURE VALUE` | any single-valued attribute by its EMF name, a random variable (its specification), or a reference (`VALUE` is a `REF`) |
| `add-container TEMPLATE NEW_ID [NAME]` | a container with copies of the template's processing resources (ids `NEW_ID_0`, ...), connected to the template's linking resources |
| `connect LINK CONTAINER` | connects a container to a linking resource |
| `add-assembly TEMPLATE NEW_ID [NAME]` | an assembly context of the template's component |
| `allocate ASSEMBLY CONTAINER NEW_ID` | a new allocation context |
| `remove-container CONTAINER` | the container leaves the resource environment (drained by default, MC-8) |
| `delete REF` | `EcoreUtil.delete` |
| `clone-monitors TEMPLATE CONTAINER` | copies of the template container's monitors for the container (Slingshot's `ResourceContainerMonitorCloner`, MC-6.3) |
| `add-demand ACTION:TYPE STOEX` | a new resource demand of an internal action (a structural change, MC-4.3) |
| `removal keep\|drain\|abort\|slingshot-q5` | the run's removal policy from this change set on (MC-8) |
| `in-progress follow\|finish` | how executions in progress continue in changed behaviours (MC-4.3) |
| `reallocation ignore\|apply` | whether a re-pointed allocation context moves its lookups (MC-3.2) |
| `monitors reference\|full` | which new monitors get series (MC-6) |

```text
# faster database disk, twice the load, a second server for the file storage
rate          DatabaseServer:HDD  1200 * 1000 * 1000
arrival-rate  RealisticUsageScenario  0.08
add-container DatabaseServer _db2 "Database Server 2"
rate          _db2:HDD  2000 * 1000 * 1000
allocate      FileStorage _db2 _alloc_fs2
```

What the changes do to the running simulation (requests in progress, queued jobs, ...) is on
[Model changes](../spec/model-changes.md).

## Exploration

`simoxide explore` forks one snapshot into variants, each with its own changes, runs them to the
end (or `--until T`) on parallel threads and prints a table of their metrics (the mean of each
series of the continuation; for state and utilisation series the time-weighted mean; plus end
time, events and finished usage-scenario runs):

```sh
simoxide explore ms.snap --model corpus/x_pem_screencast_ms \
    --variant base --variant fast-disk=disk.changes --variant scale=scale-out.qvto \
    --metrics rt,state --threads 4
```

A variant is `NAME` (no change), `NAME=FILE.changes` or `NAME=X.qvto`. The table is the same
whatever the number of threads. A runnable library version is
`crates/simoxide-reconf/examples/explore.rs` (`cargo run --release -p simoxide-reconf --example
explore`).

## Library

```rust
use simoxide_reconf::{ChangeFile, Imports, Invocation, QvtoCore, QvtoRun, ResumeChanges, RunConfig};
use simoxide_sim::{Fast, Outputs, Simulation, Watch, sim::watch::parse_condition};

// restore, change, continue
let mut sim = Simulation::<Fast>::restore_from(file, &cm, Outputs::default())?;
let core = QvtoCore::new()?;
let mut qvto = QvtoRun::new(&core, RunConfig::default())?;
qvto.restore_section(&sim, &snapshot)?;              // the QVTo state, if the snapshot has one
let changes = ResumeChanges::new()
    .file(ChangeFile::load("faster-db.changes".as_ref())?)
    .transform(core.compile_file("scale.qvto".as_ref(), &Imports::Folder)?, Invocation::simulizar());
let steps = changes.apply(&mut sim, &mut qvto)?;     // a StepReport per step

// pause when a condition holds
let mut watch = Watch::new(vec![parse_condition("rt[RealisticUsageScenario] > 5000")?]);
sim.run_watch(&mut watch)?;                           // Status::Paused or Finished
if let Some(f) = watch.fired() {
    println!("{} at {} ns", f.reason, f.at_ns);
}
```

- `ChangeFile::{parse, load, apply}` applies a change file to any run (paused or restored);
  `ResumeChanges` chains change files and transformations (`StepReport`, `ResumeError`).
- `Watch` (`simoxide_sim::sim::watch`: `Condition`, `SeriesSel`, `Stat`, `Op`, `parse_condition`)
  is a set of conditions; `Simulation::run_watch` runs until one holds. `Watch::keep_going` is a
  predicate for `Simulation::run_while`, `SpdRun::run_while` and `RulesRun::run_while`, which
  also take any predicate of one's own on the `RunState`
  (`sim.run_while(|s| watch.keep_going(s) && my_predicate(s))`).
- Rules: `RulesHost::replace_rules`, `RulesRun::restore_snapshot_replacing` (other rules than the
  snapshot's), `RulesRun::new(restored_sim, host)` (rules added).
- SPD: `SpdRun::model_changed` (the target groups' membership after a change),
  `SpdRun::enforce_policies` (queues enforced requests; `EnforceError` for an unknown policy or
  one without a target group).
- `simoxide_reconf::explore` (restores in parallel threads, `Variant`, `VariantResult`) and
  `compare` (the table).

## The Slingshot-exact engine

`--flavour slingshot-exact` (Slingshot's event bus handler by handler, [Slingshot
flavour](./slingshot.md#the-exact-mode)) takes the same options: `--pause-at`, `--pause-before`,
`--pause-events`, `--pause-when`, `--snapshot`, `--snapshot-format json`, and `simoxide resume`
with `--until`, `--pause-when`, `--change` and `--transform` (with `reconfiguration`). The
snapshot's header names the flavour `slingshot-exact`; its run state is the section `ssx`
(format page), a run with scaling policies adds `spd`, `spd-ssx` and, after adjustments, `qvto`
and `model`. The continuation is exact: the level-E projection, the tape projection with its
positions, the measurements, the handler exceptions and the decision log of the paused run and
the resumed run concatenate to the uninterrupted run's.

- A change at a resume takes effect as an SPD adjustment's would (resources of new containers
  join the resource table, those of removed containers leave it, passive resources of new
  allocation contexts, calculators of new monitors), without a `ModelAdjusted` event. The
  engine has no change options: change files with `removal`, `in-progress`, `reallocation` or
  `monitors` lines are refused. `--enforce` is not supported.
- `--pause-events` and every pause condition work with scaling policies too (the engine pauses
  between two events of Slingshot's bus, after the SPD work of the last one).

```rust
use simoxide_sim::ssx::SsxRun;
let mut run = SsxRun::new(&cm, cfg, Outputs::default())?;    // cm compiled for slingshot-exact
run.run_until(Pause::After(120_000_000_000), Some(&mut hooks))?;   // hooks: SsxSpd, or None
let mut extra = hooks.snapshot_sections(Encoding::Postcard)?;      // SPD: `spd`, `spd-ssx`
drop(hooks);
extra.push(adjuster.run.snapshot_section(Encoding::Postcard)?);    // if adjuster.run.is_started()
run.snapshot_with(&mut file, &SnapshotOptions::default(), extra)?;

// later: the run, the adjuster's QVTo state, the SPD layer
let s = Snapshot::read(f)?;
let mut run = SsxRun::restore_snapshot(&s, &cm, Outputs::default())?;
adjuster.run.restore_section(&run, &s)?;
let mut hooks = SsxSpd::restore(&spd, &s, &mut adjuster, &mut log)?;
let r = run.finish(Some(&mut hooks))?;
```

`SsxRun::fork(outputs)` copies the run in memory (a deep copy of Slingshot's objects that keeps
their sharing), `SsxRun::detach()` gives the state as a `Send` value (`SsxState`) that
`SsxRun::attach` continues on any thread, `SsxState::{serialize, deserialize}` work on a state
alone. Same state, same bytes; a restored state writes the bytes it was read from.

## What a snapshot is not

- No interchange with Slingshot's `.snapshot` files. A spike found an export feasible but lossy
  (`DESIGN-SPD.md` 5.7, "N3a spike"); it is not built.
- A snapshot holds SimOxide's internal run state: it is read by the same SimOxide version only.

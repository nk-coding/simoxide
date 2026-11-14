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

# continue it to the end (the trace gets the continuation only, m.csv the whole run's rows)
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
  and writes its measurements to `--measurements`: the rows the run holds, from time 0 by
  default, or with `--measurements-scope interval` the tuples of this invocation only
  ([Measurement intervals](#measurement-intervals)).
- `--snapshot-format json` writes the sections as JSON (for reading; see the format page).
- `resume` needs the model the run was created on (`--model`, `--run-json`): the snapshot names
  it by a fingerprint of its content and refuses another model. The mode (`exact`, `fast`), the
  flavour and its compat flags come from the snapshot; the seed, stop conditions and limits are
  part of the run state. `--timeout` applies to the continuation.
- Outputs: the trace, tape, decision log (`--spd-log`) and rules log (`--rules-log`) of the paused
  run and those of the resumed run concatenate to the outputs of the uninterrupted run. A run
  started with `--trace` must be resumed with one (or its lines are discarded); a tape can be
  added or dropped at any pause (exact mode). Measurements are cumulative by default: the
  snapshot keeps the rows, and the resumed run's `--measurements` file is the uninterrupted
  run's. With `--measurements-scope interval` they concatenate like the trace (next section).
- Scaling policies: a run with a `.spd` resumes with its interpreter state; the compat flags of
  `--spd-quirks` are kept in the snapshot and used again. With `--pause-on-decision` a run pauses
  before a policy's adjustment runs; the snapshot holds the recommendation un-applied (section
  `spd-decision`, shown by `snapshot-info` as `spd_decision`) and resumes only with `--decide
  ACTIONS`, the actions that run in its place ([Decisions](./spd.md#decisions)). HPA
  autoscalers (section `hpa`) need the same configuration at the resume and can be changed with
  `--hpa-set` ([Autoscaling (HPA)](./hpa.md#command-line)); a load profile and its position are
  part of the run state ([Load profiles](./load-profile.md)).
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
- `Simulation::advance_clock(ns)` (and `SsxRun::advance_clock`) moves the clock of a paused run
  forward without an event, never past the next event or `max_sim_time`
  ([Actions at the boundary](#actions-at-the-boundary)); a run whose clock moved and that
  changes nothing continues as without the move.
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
- `Simulation::take_measurement_rows()` (and `SsxRun`'s) moves the stored rows out of a run
  between events; a snapshot written after it holds only the rows recorded since (interval
  scope, [Measurement intervals](#measurement-intervals)). It changes nothing else in the run.
  `Snapshot::write` fills in the header's `state`; `Header::parent` is the caller's to set.
- `Measurements::interval_rows(i)` gives series `i`'s tuples of the current interval (from a
  restore, or time 0) and its last tuple before them (`meas::Before`; the section `meas-before`
  carries it across snapshots whose rows were taken); `simoxide_sim::summary::document` builds
  the [interval summary](#interval-summaries) from them, `summary::SeriesAcc` accumulates one
  series tuple by tuple.

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
  counters; once the run made a scaling decision, also the decisions (section `spd-decision`:
  their count, the pending one; `simoxide_spd::snapshot::pending_decision_of` reads it).
  Restoring checks that the interpreter has the same policies and compat flags
  (`simoxide_spd::snapshot::quirks_of` reads the flags from a snapshot).
- `RulesRun` (feature `snapshot` of `simoxide-reconf`) keeps the host's QVTo state, the PRM mirror
  (rebuilt with the same objects), the statistics and kept records. The QVTo state of other hosts
  (e.g. an SPD run's `QvtoAdjuster.run`) goes into a snapshot with
  `QvtoRun::snapshot_section(&sim, enc)` and comes back with
  `QvtoRun::restore_section(&sim, &s)`: the id generator continues, so objects created after the
  restore get the ids they would have got. `sim` is the run the transformations change: its model
  may have changed since the last transformation (the monitors an SPD adjustment clones, a change
  file), and the generator continues from it then.

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

## Measurement intervals

A search that continues snapshots many times saves each continuation's new measurements once.
`--measurements-scope interval` (on `run` and `resume`) does that: `--measurements` gets only
the tuples recorded by this invocation, and the snapshot it writes holds no measurement rows,
so neither the export nor the snapshot grows with the observations of earlier intervals.

```sh
simoxide run --model M --flavour slingshot --pause-at 100 --snapshot s1.snap \
    --measurements m0.csv --measurements-scope interval          # tuples 0..n1
simoxide resume s1.snap --model M --pause-at 200 --snapshot s2a.snap \
    --measurements m1a.csv                                       # n1..n2a (the scope is s1's)
simoxide resume s1.snap --model M --change faster.changes --pause-at 200 \
    --snapshot s2b.snap --measurements m1b.csv                   # n1..n2b, another branch
```

**Tuple numbers (the cursor).** Every measurement tuple (a row of `measurements.csv`) is
numbered in recording order from 0 at time 0 of the run. The numbering is part of the run state
and continues across snapshots, so the `n`-th tuple of a paused and resumed run is the `n`-th
tuple of the uninterrupted run with the same seed. A snapshot's header carries the cursor,
`measurements.tuples`: the number of tuples recorded before the snapshot, which is the number
of the next one ([Snapshot format](../formats/snapshot.md#header)).

**Intervals.** One invocation simulates one interval: `run` from time 0, `resume` from the
snapshot it restores, to the pause (with or without a snapshot) or the end of the run. Its
tuples are those recorded during the invocation, numbers `from..to` (`to` excluded): `from` is
the restored snapshot's cursor (0 for `run`), `to` the cursor at the pause or the end. So:

- A tuple belongs to the interval in which it is recorded, and a tuple is recorded when its
  observation is complete. A response time is recorded when its request ends: a request that
  started before the snapshot and ends after the resume is the continuation's tuple (its
  `time` is the end; `time - value` is the start, before `from`'s time). Tuples recorded at the
  resume itself (by `--change`, `--transform`, `--enforce`, `--decide`, `--hpa-set`) are the
  continuation's.
- A pause falls between two events. With `--pause-at T` every event at or before `T` has run,
  so tuples with `time = T` belong to the interval before the pause; with `--pause-events N`
  or a pause condition, both intervals can have tuples at the same time. Their numbers tell
  them apart: membership is decided by the number, never by the time.
- The intervals of a path are adjacent and disjoint: the next interval's `from` is the previous
  one's `to`. Concatenating the interval exports of a path, per series in path order, gives
  the uninterrupted run's rows of that series, and the row counts add up to `to` of the last
  interval.

Within one export the rows are ordered as in every `measurements.csv` (by series, each in
recording order, [Formats](./formats.md#_4-measurements-csv)), so a row's tuple number is not
in the file: a row is identified by its interval and its series and position, and in a path's
history by its series and its position among that series' rows of the path.

**Branches.** `resume` never changes the snapshot it reads, so any number of continuations can
start from one snapshot. Each numbers its tuples from the snapshot's cursor on: the numbers of
two siblings overlap, and a tuple number names a tuple only together with its branch, the path
of snapshots from the root run. The headers link that path: `state` is a snapshot's state id (a
hash of its sections, the same for the same run state), `parent` the state id of the snapshot
the run was resumed from (absent for a run from time 0). The CLI prints the interval of every
`resume` and pause:

```text
measurements: scope=interval tuples=1000..1053 rows=53 parent=3f0c9a1d5e7b2468 state=9b1e04c27f3a8d55
```

`rows` is the number of rows written to `--measurements` (in interval scope `to - from`);
`state` is that of the snapshot written (absent without one).

**Scopes.**

| Scope | `--measurements` of a resume | the snapshot written |
|---|---|---|
| `cumulative` (default of `run`) | every row the run holds: the parent's and the new ones, the uninterrupted run's file | keeps every row (`rows_from` 0) |
| `interval` | the tuples `from..to` of this invocation | keeps no row (`rows_from` = `tuples`) |

`resume` takes the scope of the snapshot unless `--measurements-scope` is given: a snapshot
written in interval scope continues in interval scope, any other cumulatively. `interval` on
a cumulative snapshot drops the restored rows at the restore (the continuation and its
snapshots are in interval scope from there on); `cumulative` on an interval snapshot is
refused, since the rows of the earlier intervals are not in it. In interval scope an
invocation without `--measurements` writes its tuples nowhere. `simoxide explore` summarises
each variant's tuples from the fork on, the same interval membership.

**What a continuation needs instead of the rows.** Nothing in a run reads past measurement
rows: they are output only. What does depend on past tuples is kept in its own bounded state,
which is part of every snapshot in both scopes:

- the sliding windows of SimuLizar's calculators and runtime measurements (PRM), cut to their
  window length;
- the SPD triggers' aggregation: per trigger an exponential moving average (a sufficient
  statistic) or the last values of a fixed-size window, per element with the fixed `Q13`;
- the HPA controller's busy-time readings, recommendations and scale events of the
  stabilisation windows; the engine's busy-time integrals;
- the Slingshot-exact engine's open calculators (requests in progress), the count of finished
  usage-scenario runs (`--max-measurements`) and the tuple count itself;
- pause conditions (`--pause-when`) read only tuples recorded after the start of the
  invocation (their windows start empty there), so they never need earlier rows.

So dropping the rows changes nothing in the run: an interval-scope path gives the uninterrupted
run's tuples, events and decisions. The run state (section `engine` or `ssx`) of an
interval-scope snapshot is smaller than the cumulative one by exactly the rows: in `postcard`,
16 bytes per row, plus per series the bytes of its row count beyond the one byte of an empty
list. In their place the snapshot has the section `meas-before`: per series its last tuple (at
most 17 bytes per series in `postcard`), which is not run state either: it is what the
[interval summaries](#interval-summaries) of later intervals hold at their start.

## Interval summaries

A search that evaluates many short intervals need not read the raw tuples back to compute the
same statistics each time. `--summary FILE` (on `run` and `resume`) writes, per series,
statistics of the tuples of the invocation's interval as JSON, at the pause or the end:

```sh
simoxide resume s1.snap --model M --pause-at 200 --snapshot s2.snap --summary s2.json \
    --summary-series 'rt[*]' --summary-series 'containers[*]' --summary-stats mean,max,time_weighted
```

- `--summary-series METRIC[POINT]` (repeatable; default every series) selects series as in
  [pause conditions](#pause-conditions): aliases (`rt`, `util`, `state`, `containers`, ...) or
  a metric name (prefix), a measuring-point key, a part of it or an element name (looked up in
  the model of `--model`), `*`. A metric alone (`rt`) is `rt[*]`.
- `--summary-stats LIST` (comma list; default `all`) chooses statistics; `count` is always
  written.
- It needs no `--measurements`, works in both scopes and changes nothing in the run.

```json
{
  "format": "simoxide-interval-summary",
  "interval": {
    "time": {"from": 2.0, "to": 6.0},
    "time_ns": {"from": 2000000000, "to": 6000000000},
    "tuples": {"from": 4, "to": 12}
  },
  "parent": "7b1509dec8e81d77",
  "series": [
    {
      "count": 2, "sum": 1.0, "mean": 0.5, "min": 0.0, "max": 1.0,
      "first": {"time": 3.0, "value": 1.0}, "last": {"time": 5.0, "value": 0.0},
      "initial": {"status": "tuple", "time": 2.0, "value": 0.0},
      "measuring_point": "ActiveResourceMeasuringPoint[_KFRwgCpXEeKNWc2UWjbmuw|replicaID=0]",
      "metric": "State of Active Resource Tuple",
      "time_weighted": {"from": 2.0, "integral": 2.0, "mean": 0.5, "to": 6.0},
      "unit": "1"
    },
    {
      "count": 0, "sum": 0.0, "mean": null, "min": null, "max": null,
      "first": null, "last": null,
      "initial": {"status": "none"},
      "measuring_point": "ActiveResourceMeasuringPoint[_rSc8tILWZUP8i0M73aQGtQ|replicaID=0]",
      "metric": "State of Active Resource Tuple",
      "time_weighted": null,
      "unit": "1"
    }
  ],
  "state": "94beea1cd4b29cf0",
  "stats": ["count", "sum", "mean", "min", "max", "first", "last", "initial", "time_weighted"],
  "version": 1
}
```

(Slingshot's minimal example after a scale-out at 2 s, from 2 s to 6 s: the server's state, 0
from 2 to 3 s, 1 from 3 to 5 s, 0 from 5 to 6 s; and a resource of the container the
scale-out added, which got no job. Objects are written with their keys in alphabetical order.)

**Membership.** A summary covers exactly the interval's tuples, numbers `from..to`: the rows
the interval export writes (`--measurements-scope interval`), in both scopes. Tuples recorded at
the resume (change steps, `--enforce`, `--decide`, `--hpa-set`) belong to it; adjacent intervals
that share a time (`--pause-events`, pause conditions) are told apart by number, not time. So
the `count`s of a series over the intervals of a path add up to its rows in the uninterrupted
run, and `first` and `last` are the first and last row of the series in the interval's export.

**The interval.** `tuples`: the cursor, as on the interval line (`from` is `null` if the
snapshot resumed has no cursor). `time` is the interval's logical start and end, those of the
[result report](./result-report.md#interval-boundaries): `time.to` is the requested pause time
when the run paused there (`--pause-at 200` with the last event at 199.75 s ends the interval
at 200 s: the state of 199.75 s holds until then, the next event is later), else the time of
the last event processed; `time.from` is the end of the interval resumed (0 for `run`), so
each interval starts where the previous one ended. `time` is in seconds on the engine's clock,
the time the tuples' `time` is measured in (a double in the Slingshot flavours, whole
nanoseconds in SimuLizar); `time_ns` is the same in whole nanoseconds, truncated, as in
snapshot headers. `parent` and `state` are those of the interval line (`state` only with
`--snapshot`).

**Series.** One entry per selected series that the run has at the end of the interval, in
`measurements.csv` order, identified as there by `(measuring_point, metric)`. A series without
tuples in the interval is there with `count` 0. `unit` is the unit of the values:

| Metric | Value | `unit` |
|---|---|---|
| `Response Time Tuple` | the duration of a call (its `time` is the end) | `s` |
| `Resource Demand Tuple`, `Waiting Time Tuple`, `Holding Time Tuple`, `Reconfiguration Time Tuple` | a duration | `s` |
| `State of Active Resource Tuple` | jobs at the resource | `1` |
| `State of Passive Resource Tuple` | available units | `1` |
| `Utilization of Active Resource Tuple` | busy fraction, 0 to 1 | `1` |
| `Number of Resource Containers over Time` | containers | `1` |

(`null` for a metric this build does not know.) Times are simulation seconds. What a series
refers to (its model elements, monitors and specifications) is in the runtime file of
`--runtime-metadata` ([Runtime metadata](./runtime-metadata.md#series)), which lists the same
series, every series of the run at the end of the interval, with the same `unit`.

**Statistics.**

| Name | Value |
|---|---|
| `count` | tuples of the series in the interval |
| `sum` | the sum of their values, in recording order; `0` for none |
| `mean` | `sum / count`: the sample mean, every tuple weighs the same; `null` for none |
| `min`, `max` | `null` for none |
| `first`, `last` | `{"time", "value"}` of the first and the last tuple in recording order; `null` for none |
| `initial` | the series' last tuple before the interval: `{"status": "tuple", "time", "value"}`; `{"status": "none"}`: the series had none on this path (a run from time 0, a series created later); `{"status": "unavailable"}`: not known (a snapshot without `meas-before`, written by an older build) |
| `time_weighted` | `{"from", "to", "integral", "mean"}`, the value as a step function of time (below); `null` if the series holds no value in the interval |

Empty series are explicit: `count` 0, `sum` 0, `mean`, `min`, `max`, `first`, `last` `null`
(never 0 or NaN). Numbers are JSON numbers in shortest round-trip form; a non-finite value
would be `null`.

**Sample and time weighting.** `mean` weighs every tuple the same: right for per-request series
(response, waiting, holding times). State-like series (the state of a resource, utilisation,
the number of containers) are recorded when the state changes, so their sample mean weighs a
busy stretch with many changes more than a long quiet one. `time_weighted` treats the value as
held from its tuple's `time` until the next tuple of the series, the last one until the end of
the interval: `integral` is the sum of value times holding time (e.g. container-seconds),
`mean = integral / (to - from)` (`null` for `to = from`). At the start of the interval the value
held is `initial`'s, carried across the snapshot (from the rows in cumulative scope, from the
section `meas-before` in interval scope), so a series without tuples in the interval still
holds its value (the number of containers between two scalings). Without an initial tuple
(`none`, `unavailable`) the step function starts at the first tuple of the interval: `from` is
its time. A tuple whose time lies before the previous tuple's (or before the interval) holds
from that later time. There is no aggregation over fixed windows: the only window is the
interval, with the boundaries above.

**Summaries do not replace the raw observations.** They are computed from the same tuples as
the interval export and agree with it (`count`, `sum`, `min`, `max`, `first`, `last` exactly;
`mean = sum / count`). But a statistic of the tuples cannot be graded like the tuples: the
share of response times above a threshold, a percentile or a fuzzy utility of each tuple is not
a function of the mean, and grading the mean is not the mean of the grades. Export the tuples
(`--measurements`) for whatever needs them; the summary saves reading them for the statistics
above.

**Cost.** One pass over the interval's own tuples at the end of the invocation (they are in
memory until then, in both scopes): no rows of earlier intervals are read, in cumulative scope
the series' rows start at their length at the restore. The format is `format`
`simoxide-interval-summary`, `version` 1: fields may be added in version 1; a changed meaning
gets a new version.

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
- `--actions-at clock|boundary`: the resume time of these steps. `clock` (default): the
  snapshot's clock, the time of its last event; `boundary`: the logical start of the new
  interval, after a timed pause the time it asked for ([next
  section](#actions-at-the-boundary)).
- The result is deterministic: the same snapshot and the same changes give the same
  continuation, byte for byte. It equals the uninterrupted run that applies the same changes at
  the pause (`Simulation::apply_changes`, `QvtoRun::invoke`, `RulesHost::replace_rules`).

### Actions at the boundary

A snapshot written at a timed pause has two times ([Interval
boundaries](./result-report.md#interval-boundaries)): its clock, the time of its last event
(header `time_ns`), and the logical end of its interval, the time `--pause-at T` or
`--pause-before T` (also `--until T`, the conditions `time T`, `before T`, `elapsed D`) asked
for (header `interval_end`, when it is later than the clock). The continuation's interval
starts at the second. By default a resume applies its actions at the first: `--pause-at 50`
with the last event at 49.995 s scales out at 49.995 s, inside the interval that ended at 50
s, and the scale-out can schedule work before 50 s.

```sh
simoxide resume s50.snap --model M --enforce scaleOut --actions-at boundary --pause-at 51 ...
# actions at t=50 s: the interval boundary (the snapshot's clock: t=49.995113990540906 s)
```

With `--actions-at boundary` the clock moves to the boundary first, then the actions run:

- **Actions**: `--change`, `--transform` (and the `hpa` lines of change files), `--enforce`,
  `--hpa-set`, `--decide`. Rules given with `--rules` act on later events only and do not move
  the clock. The actions run in the usual order, and what they schedule for "now" (an
  enforced adjustment, the users a larger population adds) is queued at the boundary, behind
  the events already queued for it.
- **Nothing lost or reordered**: no event lies between the clock and the boundary (the next
  event is after `T` for `--pause-at T`, at or after `T` for `--pause-before T`), and the clock
  never moves past the next event or past `max_sim_time`. The clock only moves: resources,
  windows and the scaling policies' state account for the time that passed when they are next
  used, as after any gap between two events. Times the actions take (the runtime file's
  change times, the decision log's `t`, an adjustment's request time, the tuples it records,
  the HPA's view of the changed group and its replica accounting) are the boundary's; so
  everything measured from them (an HPA's stabilisation windows, the time to the next
  policy evaluation) starts there. (Enforced and decided adjustments start no policy
  cooldown, with or without the flag.)
- **No action, or no timed pause: nothing changes.** The clock moves only for actions: a
  resume without them is the uninterrupted run byte for byte, with or without the flag. After
  a pause that was not timed (`--pause-events`, a series or SLO condition, a scaling decision)
  or with a snapshot of an older build (no `interval_end`), the boundary is the clock: the
  actions apply there, and the CLI says so (`the snapshot's interval ends at its clock`).
- **Determinism**: the moved clock is run state. A snapshot written later continues like the
  run that wrote it, and the continuation equals the uninterrupted run that pauses at the same
  point, moves its clock to the boundary (`Simulation::advance_clock`,
  `SsxRun::advance_clock`) and applies the same actions.
- **Measurements**: tuples are still assigned to intervals by number; with the flag, the
  actions' tuples have times at or after the interval's start, so the [interval
  summary](#interval-summaries) and time-weighted values need no special case. An `elapsed D`
  pause condition counts from the clock the run continues from: the boundary when the clock
  moved.
- **Where**: every flavour and engine that takes these actions (SimuLizar, Slingshot,
  Slingshot-exact with change files and transformations), with scaling policies, HPA
  autoscalers and reconfiguration rules, in both modes. The [result
  report](./result-report.md) has the mode (`behaviour.actions_at`) and the time the actions
  applied (`interval.actions`). `simoxide explore` applies its variants at the clock.

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
  one without a target group), `SpdRun::decide` (resolves a pending decision,
  `simoxide_spd::decision`).
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
extra.push(adjuster.run.snapshot_section(&run, Encoding::Postcard)?); // if adjuster.run.is_started()
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

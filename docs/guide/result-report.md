# Result report

`--report FILE` (on `simoxide run` and `simoxide resume`) writes one small JSON document per
invocation that answers what an adapter needs after it: which interval was simulated, why the
run stopped, which scaling decisions occurred, and which files hold the results. It references
the other outputs instead of embedding them, and it is written also when the invocation fails.

```sh
simoxide run --model M --flavour slingshot --max-sim-time 3600 --pause-at 1000 \
    --snapshot s1.snap --measurements-scope interval --measurements m0.csv --report r0.json
simoxide resume s1.snap --model M --pause-at 2000 --snapshot s2.snap \
    --measurements m1.csv --summary s1.json --report r1.json
```

`r0.json` of a run whose last event before 1000 s is at 999.5 s (abridged; objects are written
with their keys in alphabetical order, reordered here for reading):

```json
{
  "format": "simoxide-run-report", "version": 1,
  "simulator": {"name": "simoxide", "version": "0.1.0", "features": ["fast", "spd", "reconfiguration"]},
  "command": "run",
  "status": "paused",
  "behaviour": {"mode": "fast", "flavour": "slingshot", "quirks": "none", "spd_quirks": "fixed",
                "measurements_scope": "interval", "seed": 0, "ps_algorithm": "exact",
                "link_throughput": true, "usage_streams": false, "fix_double_resume": false,
                "load_profile": false, "restored": []},
  "limits": {"max_sim_time": 3600, "max_measurements": -1,
             "pause": {"type": "pause_at", "time": 1000.0, "time_ns": 1000000000000},
             "pause_when": [], "pause_on_decision": false, "timeout": null,
             "max_events": 0, "max_steps": 0, "max_processes": 1000000,
             "max_stack_depth": 10000, "max_events_per_instant": 20000000},
  "interval": {"start": {"time": 0.0, "time_ns": 0},
               "end": {"time": 1000.0, "time_ns": 1000000000000},
               "last_event": {"time": 999.5, "time_ns": 999500000000},
               "tuples": {"from": 0, "to": 5321}},
  "stop": {"kind": "pause", "cause": "pause_at", "detail": "every event at or before 1000 s ran"},
  "endpoint_reached": true,
  "resumable": true,
  "decisions": {"resolved": null, "pending": null},
  "measurements": {"scope": "interval", "rows": 5321},
  "state": "92752f10c6460c33",
  "outputs": {"snapshot": {"path": "s1.snap", "bytes": 48213, "state": "92752f10c6460c33"},
              "measurements": {"path": "m0.csv", "bytes": 401230, "rows": 5321}},
  "counters": {"invocation": {"events": 21933, "tuples": 5321, "adjustments": 3,
                              "spd_evaluations": 2210, "spd_requests": 3, "wall_secs": 0.41},
               "cumulative": {"events": 21933, "tuples": 5321, "usage_scenario_runs": 1012,
                              "adjustments": 3, "spd_evaluations": 2210, "spd_requests": 3}}
}
```

`r1.json` has `"command": "resume"`, `"parent": "92752f10c6460c33"`, the interval from
`1000.0` s (not 999.5 s) and in `restored` the flags it took from the snapshot.

## When it is written

The report is written last: after the snapshot, the measurements, the summary, the decision
file and the logs are complete and flushed, and after the trace and tape are closed. It goes to
a temporary file in the same folder, which is then renamed, so a reader never sees a partial
report, and a report that exists names complete files. Without `--report` nothing changes; with
it, every other output has the same bytes.

If the invocation fails (an error of the model or the run, a resource limit, `--timeout`, a
bad argument, a file that cannot be written), it still writes a report, with `status` `failed`
and the failure as its `stop`, and exits with 1 as before. Its `outputs` name only the snapshot,
if one was written before the failure (`resumable` is then `true`); every other output may be
missing or partial. What is not known at the failure is `null` or absent.

## Fields

| Field | Content |
|---|---|
| `format`, `version` | `simoxide-run-report`, 1 |
| `simulator` | `name`, `version` (the crate version) and the cargo `features` of the build |
| `command` | `run` or `resume` |
| `status` | `paused` (between two events; the run can continue), `finished` (the run is over), `failed` |
| `behaviour` | the effective flags: `mode`, `flavour`, `quirks` (the Slingshot compat flags as `--quirks` takes them, `none` for none), `spd_quirks` (the SPD interpreter's: `fixed`, `slingshot` or a list; `null` without scaling policies), `measurements_scope`, and from the run's configuration `seed`, `ps_algorithm`, `link_throughput`, `usage_streams`, `fix_double_resume`, `load_profile` (a load profile is active). `restored`: the flags a `resume` took from the snapshot, not from its command line |
| `limits` | the run's stop conditions and limits: `max_sim_time` (s) and `max_measurements` (`-1` or `0` = off), `max_events`, `max_steps`, `max_processes`, `max_stack_depth`, `max_events_per_instant` (`0` = off); a resumed run's come from the snapshot. This invocation's: `pause` (`{"type": "pause_at" or "pause_before", "time", "time_ns"}`, `{"type": "pause_events", "events"}`, or `null`), `pause_when` (the conditions as given), `pause_on_decision`, `timeout` (s) |
| `interval` | `start`, `end`, `last_event` (each `{"time", "time_ns"}`) and the tuple cursor `tuples` `{"from", "to"}` ([Interval boundaries](#interval-boundaries)) |
| `stop` | why the run stopped: `kind`, `cause`, `detail` (in words), and `condition` or `decision` where they apply ([Stop reasons](#stop-reasons)) |
| `endpoint_reached` | the interval ended where the invocation asked it to end at the latest: at its requested pause (`stop.kind` `pause`), or at the run's own end (`completed`). `false` when something stopped it earlier: a series or SLO condition, a scaling decision, a limit, an error |
| `resumable` | a snapshot was written (`outputs.snapshot`) from which the run continues. A snapshot paused at a scaling decision resumes with `--decide` only |
| `decisions` | for runs with scaling policies (not in `slingshot-exact`): the [decision file](./spd.md#decision-file)'s `resolved` (the decision `--decide` resolved: `recommended`, the `selected` actions in order, their `outcomes` in order) and `pending` (the decision the run paused at, with its `recommended` actions); each `null` when there is none |
| `measurements` | `scope` (`cumulative`, `interval`) and `rows`: the rows `--measurements` gets |
| `parent`, `state` | the state ids of the snapshot resumed and of the snapshot written ([Measurement intervals](./snapshots.md#measurement-intervals)) |
| `outputs` | the files written, by kind: `snapshot` (with its `state`), `measurements` (with `rows`), `summary`, `trace`, `tape`, `spd_log`, `decision_file`, `rules_log`; each `{"path", "bytes"}`, the path as given. A file that was not asked for, or that this kind of run does not write (`spd_log` without scaling policies), is absent |
| `counters` | `invocation`: what this invocation did: `events`, `tuples` (`to - from`), `wall_secs`, and where they apply `adjustments` (executed SPD or HPA adjustments, `--decide` and `--enforce` actions included), `spd_evaluations`, `spd_requests`, `reconfigurations` and `reconfigurations_changed` (SimuLizar's rules), `change_steps`. `cumulative`: counted from time 0 along the run's path: `events`, `tuples`, `usage_scenario_runs` (finished usage-scenario runs, what `max_measurements` counts), and the SPD and rules counters |

Times are simulation seconds (`time`, the engine's clock: a double in the Slingshot flavours)
and the same in whole nanoseconds (`time_ns`, truncated, as in snapshot headers). Fields may be
added within version 1; a changed meaning gets a new version.

## Interval boundaries

An invocation simulates one interval of the run ([Measurement intervals](./snapshots.md#measurement-intervals)),
with three times:

- **`last_event`**: the simulation clock when the invocation stopped, the time of the last
  event processed; the `time_ns` of the snapshot written. In the Slingshot flavours the stop at
  `max_sim_time` is itself an event at that time (Slingshot's `SimulationFinished`). If the
  invocation processed no event, it is the clock it was resumed at, which can be before `start`.
- **`end`**: the interval's logical endpoint. If the run paused at a time it was asked for,
  `end` is that time: `--pause-at T` (also `--until T` and the condition `time T`): every event
  at or before `T` ran and the next one is later; `--pause-before T` (`before T`): every event
  before `T` ran and the next is at or after `T`; `elapsed D`: the clock at the start plus `D`.
  Otherwise `end` is `last_event`: a pause after a number of events, a series or SLO condition
  (the run pauses right after the event that recorded the tuple), a scaling decision (at the
  time of the recommendation), the end of the run (`max_sim_time`, `max_measurements`, no event
  left) and a failure (the time of the error). `end` is never before `start`.
- **`start`**: the logical end of the parent interval, the `end` of the invocation that wrote
  the snapshot (0 for `run`). A snapshot keeps it in its header as `interval_end` when it is
  not the clock ([Snapshot format](../formats/snapshot.md#header)); a snapshot without it (one
  that paused at its clock, or one from an older build) starts the next interval at its clock.

The issue's case: `--pause-at 1000` with the last event at 999.5 s reports `last_event`
999.5 s and `end` 1000 s, and the continuation's `start` is 1000 s. Nothing happens between
999.5 s and 1000 s: the run state at the pause is the state at every time in
`[last_event, end]`, since the next event is after `end`. So the intervals of a path tile its
timeline: each `start` is the previous `end`, without gaps or overlaps, and the durations
`end - start` add up to the time simulated. Use `end - start` for durations and costs, and
`last_event` to know how far the events ran. Endpoints are never taken from measurements.

The [interval summary](./snapshots.md#interval-summaries) (`--summary`) has the same
boundaries: its `interval.time` is `start` to `end`, and `time_weighted` holds the last value
until `end`, which is right because the state does not change between `last_event` and `end`.
The integrals of a path still add up to the uninterrupted run's.

At a boundary time: with `--pause-at T` the events at `T` belong to the interval that ends at
`T`, with `--pause-before T` to the next one. Tuples are assigned by number, never by time
(`interval.tuples` `from..to`, `to` excluded): the next interval's `from` is this one's `to`.

The anchoring does not move the clock: a continuation's first event is after the parent's
`end`, so the path is the uninterrupted run whatever `end` was, and a run with `--report` is
the run without it. But what a resume does before its first event (`--change`, `--transform`,
`--enforce`, `--decide`, `--hpa-set`) acts at the snapshot's clock, the parent's
`last_event`, which can be before `start`. Its tuples (and events it schedules before `start`)
belong to the continuation by number, with their own times; the
[interval summary](./snapshots.md#interval-summaries) counts such a value from `start` on.

## Stop reasons

| `stop.kind` | `stop.cause` | When |
|---|---|---|
| `pause` | `pause_at`, `pause_before`, `pause_events` | the requested pause: `--pause-at`, `--until`, `--pause-before`, `--pause-events`, or the conditions `time`, `before`, `events` |
| `pause` | `elapsed`, `elapsed_events` | the conditions `elapsed T`, `elapsed-events N` |
| `condition` | `series`, `slo` | a measurement or SLO condition of `--pause-when` |
| `decision` | `decision` | a scaling decision is pending (`--pause-on-decision`); `stop.decision` is its number |
| `completed` | `max_sim_time`, `max_measurements`, `no_events` | the run's own end: the simulation time, the number of finished usage-scenario runs, or no event left |
| `limit` | `timeout` | `--timeout` passed |
| `limit` | `max_events`, `max_steps`, `max_processes`, `max_stack_depth`, `livelock`, `limit` | a resource limit (`--max-events`, ...), the livelock guard (`--max-events-per-instant`), another bound of the engine |
| `error` | `simulation` | the model or the run failed (where the reference aborts too, or the model is outside what SimOxide supports) |
| `error` | `invocation` | the invocation failed outside the simulation: arguments, loading or compiling the model, a file |

For a `--pause-when` condition, `stop.condition` is `{"index", "text"}`, its position among the
`--pause-when` flags and the condition as given; `detail` says what held. The pause of
`--pause-at` given together with `--pause-when` is a `pause` without `condition`. If several
conditions hold after the same event, the first one listed is reported.

## Server

The server has no snapshots: a job is one interval from time 0. Its results reuse the
existing `summary` and `stop_reason` ([Server](./server.md#results)): `summary.interval` is the
`interval` above (`start` 0; `end` the `max_sim_time` termination criterion's time when it
ended the run, else the last event's; `tuples` `0..tuples`); `summary.events`, `tuples` and
`measurements` are the counters (a job's invocation counters are its cumulative ones); and
`stop_reason.cause` has the causes above:

| CLI `stop` | server `stop_reason` |
|---|---|
| `completed` (`max_sim_time`, `max_measurements`, `no_events`) | `simulator`, the same `cause` |
| `limit` (`max_events`, `max_steps`, ...), `error` `simulation` | `error` (the job fails), `cause` the limit or `simulation` |
| `decision` | `decision`, `cause` `decision` |
| `pause`, `condition` | `termination`: the request's termination conditions; `detail` names them |
| `limit` `timeout` | `termination` (`timeout_secs`) or `server_limit` (the server's run-time cap) |
| (none) | `server_limit`, `cancelled` |

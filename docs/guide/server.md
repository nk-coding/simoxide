# Server: HTTP and MCP

`simoxide-server` makes SimOxide callable from other programs and from AI agents. A client
sends a Palladio model with its measuring points and monitor repository, the server simulates
it in the background, and the client follows the run live, stops it by a confidence interval,
a timeout or a count, and reads the results at the end.

The server speaks two protocols over the same operations:

- **HTTP**: a REST API with long polling and a server-sent-events stream of progress and logs;
- **MCP** (Model Context Protocol): the operations as tools, over stdio (the client launches the
  server) or over HTTP at `/mcp`.

```text
            ┌──────────── simoxide-server ────────────┐
 REST/SSE ─►│ http ─┐                                 │
 MCP/HTTP ─►│ /mcp ─┼─► api::Service ─► JobManager ───┼─► one thread per simulation:
 MCP/stdio ►│ mcp ──┘                                 │   load, compile, run in steps,
            └─────────────────────────────────────────┘   monitor: progress, termination
```

## Build and start

```sh
cargo build --release -p simoxide-server                              # EPL build
cargo build --release -p simoxide-server --features reconfiguration   # + SPD/HPA adjustments

./target/release/simoxide-server --http 127.0.0.1:8080 --allow-dir corpus
./target/release/simoxide-server --mcp-stdio --allow-dir corpus      # for MCP clients
```

Without `--http` and `--mcp-stdio` the server serves HTTP on `127.0.0.1:8080`; with both it
serves both.

| Flag | Default | Meaning |
|---|---|---|
| `--http ADDR` | `127.0.0.1:8080` | serve HTTP (REST, SSE, MCP at `/mcp`) on `ADDR` |
| `--mcp-stdio` | off | serve MCP on stdin and stdout; the server ends when stdin closes |
| `--allow-dir DIR` | none | requests may name model directories below `DIR` (`model_dir`); repeatable |
| `--max-concurrent N` | CPUs | simulations that run at once; the others wait as `queued` |
| `--max-queued N` | 64 | simulations that wait for a free slot; beyond it a new one is refused (`429`) |
| `--max-jobs N` | 100 | simulations kept in memory; the oldest finished ones are dropped |
| `--max-run-secs S` | 3600 | wall-clock cap of one run (`0` = none) |
| `--max-tuples N` | 20 000 000 | cap of recorded tuples per run, 16 bytes each (`0` = none) |
| `--log-capacity N` | 5000 | log entries kept per simulation (ring buffer) |
| `--token TOKEN` | none | HTTP needs `Authorization: Bearer TOKEN` (also `SIMOXIDE_SERVER_TOKEN`) |
| `--max-body-mb N` | 64 | largest HTTP request body |
| `--allow-host NAME` | none | accept `Host` and `Origin` headers naming `NAME` besides `localhost`, `127.0.0.1` and `[::1]` (the server's name on a network); repeatable |
| `--allow-any-origin` | off | accept any `Host` and `Origin` header (turns the DNS-rebinding guard off) |

Cargo features of `simoxide-server`:

| Feature | Effect |
|---|---|
| `spd` (default) | the model's scaling policies (`.spd`) and HPA autoscalers (`*.hpa.json`) run; without `reconfiguration` their decisions are logged but not executed |
| `reconfiguration` | executes SPD and HPA adjustments with Slingshot's QVTo transformation (EPL) |
| `exact` | `"mode": "exact"` (SimuLizar-exact); not distributable ([Licensing](./licensing.md)) |
| `hotspot-math` | bit-exact HotSpot `log`/`exp` with `exact`; GPL-2.0-only, not distributable |

The release profile aborts on a panic. No panic is known on any input, but a server that must
survive one can be built with `--profile release-unwind`; a panic then fails only its
simulation.

## Connect an MCP client

Claude Code, with the server launched by the client (stdio):

```sh
claude mcp add simoxide -- /path/to/simoxide/target/release/simoxide-server \
    --mcp-stdio --allow-dir /path/to/models
```

Or against a running HTTP server:

```sh
claude mcp add --transport http simoxide http://127.0.0.1:8080/mcp
```

Other clients take the usual configuration:

```json
{
  "mcpServers": {
    "simoxide": {
      "command": "/path/to/simoxide/target/release/simoxide-server",
      "args": ["--mcp-stdio", "--allow-dir", "/path/to/models"]
    }
  }
}
```

The server's `initialize` answer carries instructions for the agent (the workflow below).

## Workflow

1. **Start** a simulation with the model and the termination conditions (`POST
   /v1/simulations`, tool `start_simulation`). The answer comes at once with the simulation's
   `id`; the run goes on in the background.
2. **Follow** it: its status carries the latest progress (`GET /v1/simulations/{id}`, tool
   `get_status`); long-poll for the next change (`?wait=30`, tool `wait_for_simulation`) or
   subscribe to the event stream (`GET /v1/simulations/{id}/events`). The log (`/logs`, tool
   `get_logs`) has model warnings, scaling-policy decisions and the end.
3. **Stop** it early if the numbers already answer the question (`POST .../cancel`, tool
   `cancel_simulation`); its results so far stay readable. A queued simulation ends at once; a
   running one stops at its next pause between two events, and only if it has not stopped
   10 s later is it aborted in the middle of an event.
4. **Read** the results once it has ended (`GET .../results`, `.../measurements.csv`, tool
   `get_results`).

A simulation's `status` is one of:

| Status | Meaning |
|---|---|
| `queued` | waiting for a free slot (`--max-concurrent`); at most `--max-queued` wait, and a submit beyond them is refused with `429` (try again once a simulation has ended) |
| `loading` | it has a slot: loading and compiling the model |
| `running` | simulating; progress is published |
| `completed` | a termination condition, the simulator's own stop condition, the end of the events, or a server cap ended it; results available |
| `cancelled` | a client stopped it; results so far available (none if it was still queued) |
| `failed` | the model did not load or compile, or the run aborted; results so far available if it had started |

Every change of a simulation (status, progress, log entry) increases its `version`. A client
that passes the last version it saw (`after_version`) is woken by the next change.

## The request

```json
{
  "name": "mediastore, 2 replicas",
  "files": [
    {"name": "ms.repository", "content": "<?xml version=\"1.0\" ...>"},
    {"name": "ms_base.system", "content": "..."},
    {"name": "ms.resourceenvironment", "content": "..."},
    {"name": "ms_base.allocation", "content": "..."},
    {"name": "ms_base_usage_realistic.usagemodel", "content": "..."},
    {"name": "refsim.measuringpoint", "content": "..."},
    {"name": "refsim.monitorrepository", "content": "..."},
    {"name": "run.json", "content": "{\"seed\": 1}"}
  ],
  "config": {"flavour": "simulizar", "seed": 7},
  "termination": {
    "timeout_secs": 120,
    "all": [
      {"confidence": {"relative_half_width": 0.02, "series": [{"metric": "Response Time"}]}},
      {"max_measurements": 2000}
    ]
  },
  "progress": {"interval_ms": 500, "warmup_time": 60}
}
```

Unknown fields are errors, so a misspelt option is reported instead of ignored. A request that
fails the checks that need no model is answered with `400`; a model that fails to load or
compile makes a simulation that ends as `failed` with the loader's message.

### The model: `files` or `model_dir`

`files` are the model's files, by name and content (`content` as text, or `content_base64`).
Names are relative paths as the files' `href`s refer to each other, usually bare file names;
a subdirectory is part of the name (`monitors/monitors.monitorrepository`). The files are
loaded from memory: the server never reads the file system for them, and `href`s to files
that were not sent fail to resolve (the bundled `pathmap://PCM_MODELS/...` resources are
available).

The entry files are chosen as for a model directory ([Getting started](./getting-started.md#run-a-model)):
the single `.usagemodel`, every `.allocation`, every `.monitorrepository` (or those `run.json`
or `config` names), then the `.spd` and `.semanticspd` files. Besides the PCM files, a model
may include:

| File | Effect |
|---|---|
| `run.json` | seed and the simulator's stop conditions ([Formats](./formats.md)); `config` overrides it |
| `*.spd`, `*.semanticspd` | scaling policies ([SPD](./spd.md)); off with `"config": {"spd": false}` |
| `*.hpa`, `*.hpa.json` | HPA autoscalers ([HPA](./hpa.md)); off with `"hpa": false` |
| `*.loadprofile.json` | a time-varying load ([Load profiles](./load-profile.md)); off with `"load_profile": false` |

`model_dir` names a model directory on the server instead, relative to one of the
`--allow-dir` directories (or absolute inside one). `GET /v1/models` (tool `list_models`)
lists the directories with a `.usagemodel` below them, each with its `root` (the allowed
directory it is below). When several allowed directories have a model of the same name, the
request must say which with `"root"` (as the listing reports it); otherwise it is refused, so
that a request never runs another model than the one listed. Without `--allow-dir`,
`model_dir` is refused.

### `config`

| Field | Default | Meaning |
|---|---|---|
| `seed` | run.json's, else 0 | seed of the random numbers |
| `flavour` | `simulizar` | `simulizar`, `slingshot` or `slingshot-exact` ([Slingshot-compatible mode](./slingshot.md)) |
| `mode` | `fast` | `fast` or `exact` (needs the feature `exact`) ([Fast mode](./fast-mode.md)) |
| `quirks` | `none` | Slingshot compat flags (`all`, `Q1,Q5,...`), as `--quirks` |
| `max_sim_time` | run.json's | the simulator's own stop: simulated seconds (`-1` = off); required by `slingshot-exact` |
| `max_measurements` | run.json's | the simulator's own stop: finished usage-scenario runs (`-1` = off) |
| `usagemodel`, `allocation`, `monitorrepository` | found by extension | entry files: names of model files, relative, without `..` |
| `simulate_linking_resources`, `simulate_throughput_of_linking_resources` | run.json's | as in `run.json` |
| `ps_algorithm` | `exact` | `exact` or `virtual-time` processor sharing |
| `usage_streams`, `fix_double_resume` | `false` | as `--usage-streams`, `--fix-double-resume` ([Command line](./cli.md)) |
| `spd`, `hpa`, `load_profile` | `true` | use the model's scaling policies, autoscalers, load profiles |
| `spd_quirks` | `fixed` (`slingshot` for `slingshot-exact`) | as `--spd-quirks` |
| `spd_log_all` | `false` | also log the SPD evaluations that requested nothing (one per watched measurement) |
| `spd_decisions` | absent (off) | scaling decisions ([below](#scaling-decisions)): a list of action lists, e.g. `[["_out"], []]` |
| `interval_summary` | absent (off) | `{"series": [{measuring_point, metric}], "stats": [...]}`: an interval summary in the results ([below](#interval-summaries)) |
| `runtime_metadata` | `false` | record the model file and the runtime file of the run ([below](#runtime-metadata)); not with `slingshot-exact` |
| `max_events_per_instant`, `max_stack_depth`, `max_processes`, `max_steps` | as `simoxide run` | resource limits ([Deviations](../correctness/deviations.md#resource-limits-for-untrusted-models)); `slingshot-exact`, which has no processes, bounds the frames of a user's stack, the live users and the handler events instead |

`config.max_sim_time` and `config.max_measurements` are the simulator's own stop conditions,
with SimuLizar's (or Slingshot's) semantics, including what happens at the stop. The
`termination` conditions below are the server's: checked between two events, they end the run
where it is.

### `termination`

A termination condition is a JSON object whose fields are criteria:

| Criterion | Holds when |
|---|---|
| `timeout_secs` | this many wall-clock seconds passed since the run started (after loading) |
| `max_measurements` | this many usage-scenario runs finished (SimuLizar's measurement count); not negative: unlike `config.max_measurements`, `-1` does not mean off here |
| `max_tuples` | this many measurement tuples were recorded, all series together |
| `max_events` | this many events were processed |
| `max_sim_time` | the next event is later than this simulated time (s): every event up to it has run |
| `confidence` | the confidence interval of the mean of every watched series is narrow enough (below) |
| `all` | every condition of the list holds |
| `any` | any condition of the list holds |

The criteria of one object combine as its `combine` field says: `any` (the default) or
`all`. `all` and `any` nest conditions, each counting as one criterion of its object; their
lists must not be empty. An empty object never holds. A `max_sim_time` that has been reached
stays reached, also inside `all` while the other criteria catch up.

```json
{"timeout_secs": 60, "max_measurements": 10000}
```
stops after a minute or 10 000 usage-scenario runs, whichever comes first.

```json
{"timeout_secs": 300,
 "all": [{"confidence": {"relative_half_width": 0.05}}, {"max_measurements": 1000}]}
```
stops when the response times' confidence intervals are within ±5 % and at least 1000 runs
finished, or after five minutes.

The counts are compared before each event, so a run stops at the first event boundary after
its count is reached: `max_measurements: 1000` ends with 1000 runs (1001 if one event finishes
two). `timeout_secs` is checked with the progress cadence (at least every `interval_ms`).

A run needs a stop that bounds it: `config.max_sim_time` or `config.max_measurements`, or a
termination condition with a count or a timeout. A request with only a confidence criterion
is accepted (it ends at the server's `--max-run-secs` cap at the latest, and its log says so);
on a server without that cap it fails with a message saying so. When the request gives the simulator no stop condition of
its own, the server sets its usage-scenario count where it never holds and the termination
conditions end the run (the livelock guard then also counts a finished usage-scenario run as
progress).

#### Confidence intervals

`confidence` watches series (by default every response-time series, metric `Response Time
Tuple`; `series` selects others by substrings of measuring point and metric) and holds when,
for every watched series with data,

- the series has at least `min_samples` values (default 100) after the warm-up, and
- the confidence interval of its mean at `level` (default 0.95) is narrower than the target:
  `relative_half_width` (half width relative to the mean, default 0.05 when no target is
  given) and/or `absolute_half_width` (in the series' unit, seconds for response times).

The interval comes from **batch means**, which tolerate the autocorrelation of simulation
output: the values are grouped into consecutive batches, the batch means are treated as
independent, and the interval is `mean ± t(level, k-1) · s / √k` for `k` batches with standard
deviation `s` (Student's t). Two batchings:

- default: a fixed number of batches whose size doubles as the series grows (between
  `min_batches` and `2·min_batches` batches, `min_batches` default 30), so the batches get
  longer, and the batch means less correlated, the longer the run;
- `batch_size: N`: batches of `N` values and at least `min_batches` of them (SimuCom's static
  batch-means algorithm).

Each interval reports the lag-1 autocorrelation of its batch means: near 0 means the batches are
long enough; well above 0 (say > 0.2) means the interval is optimistic, and a larger
`batch_size`, `min_samples` or warm-up helps. The intervals are recomputed at every progress
update and every 2 % of new tuples (at least every 500), so a run stops shortly after its
intervals are narrow enough.

The sample-mean interval fits values that are observations (response times, waiting times).
For state and utilisation series, whose values are a step function of time, the statistics
report the time-weighted mean, but an interval of the plain mean of their tuples means little.

### `progress`

| Field | Default | Meaning |
|---|---|---|
| `interval_ms` | 500 | wall-clock interval of progress updates (at least 20, at most 86 400 000: one day) |
| `warmup_time` | 0 | rows before this simulated time (s) count neither in the statistics nor in the confidence intervals |
| `warmup_samples` | 0 | nor the first rows of each series |

The warm-up applies to the progress, the final statistics and the confidence criteria; the raw
rows and `measurements.csv` keep every row.

## Live feedback

### Progress

The status of a running simulation carries its latest progress:

```json
{
  "id": "sim-3-1a2b3c", "status": "running", "version": 57, "last_log_seq": 12,
  "model": {"flavour": "simulizar", "mode": "fast", "seed": 7, "series": [...], ...},
  "progress": {
    "wall_secs": 2.5, "sim_time": 4512.8, "events": 1840211, "measurements": 4377,
    "tuples": 160233, "events_per_sec": 7.3e6, "sim_speed": 1805.1,
    "live_processes": 41,
    "termination": [
      "timeout_secs: 2.5/300 s",
      "all[0].confidence: 1/2 watched series within ±5 % at 95 %",
      "all[1].max_measurements: 4377/1000",
      "server cap: 2.5/3600.0 s"
    ],
    "series": [
      {"measuring_point": "UsageScenarioMeasuringPoint[...]", "metric": "Response Time Tuple",
       "count": 4377, "total_count": 4377, "mean": 1.92, "std_dev": 0.71, "min": 0.4,
       "max": 7.9, "time_weighted_mean": 1.95, "first_time": 0.8, "last_time": 4512.1,
       "confidence": {
         "interval": {"level": 0.95, "mean": 1.92, "half_width": 0.051,
                      "relative_half_width": 0.027, "batches": 34, "batch_size": 128,
                      "lag1_autocorrelation": 0.04},
         "target_relative_half_width": 0.05, "satisfied": true}}
    ]
  }
}
```

`termination` lists every criterion and how far the run is. `series` has every series with
data: `count` after the warm-up, `total_count` with it, `confidence` for series a confidence
criterion watches. Percentiles are left out of the live progress (they need a sort) and are
part of the results.

The statistics are incremental: each update folds in the rows recorded since the previous one,
so it costs time in proportion to the new rows. A run with live progress takes about 7 % longer
than `simoxide run` (MediaStore, 33 million tuples). The simulation itself is the same: same
seed, same events, same measurements.

### Long polling

`GET /v1/simulations/{id}?wait=30&after_version=57` answers as soon as the version exceeds 57
(new progress, a log entry, a status change), at the latest after 30 s; `until=done` waits for
the end instead. Without `after_version` it waits for the next change after the current one.
The tool `wait_for_simulation` does the same (`timeout_secs` at most 300).

### Server-sent events

`GET /v1/simulations/{id}/events` streams the simulation as server-sent events:

| Event | Data |
|---|---|
| `status` | the simulation's state without progress; first, and on every status change |
| `progress` | the progress object, at each update |
| `log` | one log entry |
| `done` | `id`, `status`, `stop_reason`, `error`, `results_available`; the last event |
| `error` | the simulation is gone (deleted) |

Query: `level=debug|info|warn|error` (default `info`), `logs=false` (no log events),
`since=SEQ` (log entries after `SEQ`; default the whole log). A comment line keeps the
connection alive every 15 s.

```sh
curl -N http://127.0.0.1:8080/v1/simulations/sim-3-1a2b3c/events
```

### Logs

A log entry:

```json
{"seq": 41, "wall_ms": 734, "sim_time": 577.27, "level": "info", "source": "spd",
 "message": "adjustment by scaleOutDataProcessing: ok=true size 1 -> 2",
 "data": {"k": "adjust", "t": 577.27, "policy": "_Ey7I...", "ok": true, ...}}
```

| Source | Entries |
|---|---|
| `server` | submission, cancellation, internal errors |
| `model` | files loaded, compile time, compile warnings (e.g. monitors that cannot be recorded), load errors |
| `spd` | scaling policies: each policy when first evaluated, every evaluation that requested an adjustment, every adjustment with its outcome and size change; `data` is the record of the SPD decision log ([Scaling policies](./spd.md)) |
| `simulator` | warnings of the run (e.g. Slingshot handler exceptions), errors |
| `termination` | how and after how long the run ended |

`GET /v1/simulations/{id}/logs?since=SEQ&limit=N&level=LEVEL` returns the entries after `SEQ`
with `last_seq` (pass it as the next `since`) and `dropped` (entries after `SEQ` that the ring
buffer had already evicted).

## Results

`GET /v1/simulations/{id}/results` (tool `get_results`) answers once the simulation has ended:

```json
{
  "id": "sim-3-1a2b3c", "name": "...", "status": "completed", "model": {...},
  "summary": {
    "stop_reason": {"kind": "termination",
                    "detail": "all(confidence: 2 series within ±5 % at 95 %; max_measurements: 1000 usage-scenario runs)"},
    "sim_time": 9120.4,
    "interval": {"start": {"time": 0.0, "time_ns": 0},
                 "end": {"time": 9120.4, "time_ns": 9120400000000},
                 "last_event": {"time": 9120.4, "time_ns": 9120400000000},
                 "tuples": {"from": 0, "to": 341007}},
    "events": 3901277, "measurements": 9310, "tuples": 341007,
    "wall_secs": 0.41, "warnings": [],
    "series": [{"measuring_point": "...", "metric": "Response Time Tuple", "count": 9310,
                "mean": 1.93, "std_dev": 0.72, "min": 0.4, "max": 9.1,
                "percentiles": {"p50": 1.8, "p90": 2.9, "p95": 3.4, "p99": 4.6},
                "time_weighted_mean": 1.94, "confidence": {...}, ...}],
    "spd": {"evaluations": 144640, "requests": 8, "adjustments_ok": 8, "pending": 0}
  }
}
```

`stop_reason.kind` is `termination` (a termination condition held; `detail` says which),
`simulator` (`config.max_sim_time`, `config.max_measurements`, or no event left),
`server_limit` (`--max-run-secs` or `--max-tuples`), `decision` (a scaling decision without an
action list in `config.spd_decisions`), `cancelled` or `error`. `stop_reason.cause` names the
cause in the words of the CLI's [result report](./result-report.md#stop-reasons): for
`simulator` `max_sim_time`, `max_measurements` or `no_events`; for `error` the limit
(`max_events`, `max_steps`, `max_processes`, `max_stack_depth`, `livelock`, `limit`) or
`simulation`; `decision` for `decision`; absent for the other kinds. `uniforms` (the random
numbers drawn) is reported for runs the simulator finished itself.

`summary.interval` is the run's interval as in the result report
([Interval boundaries](./result-report.md#interval-boundaries)): a job is one interval from
time 0. `last_event` is the time of the last event processed (`sim_time`); `end` is the time a
`termination.max_sim_time` criterion asked for when it ended the run (the run stops before the
first event later than that time, so the last event can be earlier), else the last event's;
`tuples` is `0..tuples`. The result report's mapping of stop reasons to the server's is on
that page ([Server](./result-report.md#server)).

### Scaling decisions

The server has no snapshots, so it runs `simoxide run --pause-on-decision` / `resume --decide`
([Decisions](./spd.md#decisions)) as one job with the actions given in advance:
`config.spd_decisions` is a list of action lists. With it, the run pauses before each scaling
policy's adjustment and resolves decision `i` (from 0) with the `i`-th list: the policies (ids or
names) whose adjustments run in its place, in order (`[]` declines, `["A","A"]` runs A twice).
The semantics are the command line's: the recommendations of one instant form one decision, the
policies' state changes on the recommendation, not on the selection. At the first decision
beyond the lists the job ends as `completed` with the stop reason `decision`, the decision
pending; a client that wants to continue it starts the job again with one more list (runs are
deterministic, so the same lists reach the same decision). `[]` (no lists) stops at the first
recommendation. `slingshot-exact` and `"spd": false` are refused (`400`); an action naming no
policy of the model, or a model without scaling policies, fails the job.

The results' `summary.spd` then has `decisions`, the resolved decisions (each with `decision`,
`time`, `time_ns`, `recommended`, `selected`, `outcomes`, as in the [decision
file](./spd.md#decision-file)), and `pending_decision` (the decision the run stopped at, or
`null`); the `spd` log has the `decision` and `select` records. The tool `start_simulation`
takes the same `config.spd_decisions`.

```json
{"model_dir": "rmuc_app_metrics",
 "config": {"flavour": "slingshot", "max_sim_time": 300, "spd_decisions": [[]]}}
```

Query parameters (tool arguments in parentheses):

| Parameter | Meaning |
|---|---|
| `measuring_point=`, `metric=` (`series: [{measuring_point, metric}]`) | only the series whose measuring point and metric contain these substrings |
| `rows=true` (`include_rows`) | add the raw `(time, value)` rows per series |
| `max_rows=N` (`max_rows_per_series`) | at most `N` rows per series, evenly spaced with the first and last (default 1000, `0` = all) |
| (`format: "csv"`, `max_csv_bytes`) | the tool returns `measurements.csv` instead, cut at whole lines after `max_csv_bytes` (default 1 MiB) |

`GET /v1/simulations/{id}/measurements.csv` returns every row as `measurements.csv`
([Formats](./formats.md)), optionally only the selected series. Before the end, results are
`409`; read the live progress instead, or cancel the run to get the results so far.

A simulation on the server is one run from time 0, without snapshots: its measurements are
a single interval in the sense of [Measurement intervals](./snapshots.md#measurement-intervals),
tuples `0..tuples` (`summary.tuples`, the number of rows of `measurements.csv` without a series
selection), so the cumulative and the interval scope coincide and there is nothing to choose.

### Interval summaries

With `config.interval_summary` the results' `summary.interval_summary` is the document
`simoxide run --summary` writes ([Interval summaries](./snapshots.md#interval-summaries)) for
the run's single interval: tuples `0..tuples`, time 0 to `summary.interval.end` (`sim_time` unless a
`termination.max_sim_time` criterion ended the run after an earlier last event), every tuple from time 0
(no warm-up), without `parent` and `state`. `series` selects series as everywhere in the server
(substrings of the measuring point and the metric; any selector matches; empty or absent: every
series), `stats` names the statistics (`count`, `sum`, `mean`, `min`, `max`, `first`, `last`,
`initial`, `time_weighted`, `all`; default all; `count` is always written). An unknown statistic
is refused (`400`). `initial` is `none` for every series (a run from time 0). The tool
`start_simulation` takes the same option; the raw rows stay available as above.

```json
{"model_dir": "rmuc_app_metrics",
 "config": {"interval_summary": {"series": [{"metric": "Response Time"}], "stats": ["mean", "max"]}}}
```

### Runtime metadata

With `config.runtime_metadata: true`, `GET /v1/simulations/{id}/runtime-metadata` (tool
`get_runtime_metadata`) returns, once the simulation has ended, `{"model": ..., "runtime":
...}`: the model file and the runtime file of `simoxide run --model-metadata
--runtime-metadata` ([Runtime metadata](./runtime-metadata.md)). The job is one interval from
time 0: `runtime.start` is the model's deployment, `runtime.changes` every adjustment of the run
(SPD policies, actions of `config.spd_decisions` with cause `decision`, HPA rescales), and
`runtime.series` describes every series of `measurements.csv`. `runtime.run` has no `parent` or
`state` (no snapshots); its `status` is `finished` when the simulator ended the run, `stopped`
when a termination condition, a decision without action list or a cancellation did, and
`failed`. Without the option the route answers `409`; `slingshot-exact` refuses it (`400`).

## HTTP API

| Method and path | Purpose |
|---|---|
| `GET /health` | `ok` (no authentication) |
| `GET /v1/info` | version, flavours, modes, features, limits |
| `GET /v1/models` | model directories below `--allow-dir` |
| `POST /v1/simulations` | start a simulation; `201` with its state and a `Location` header |
| `GET /v1/simulations` | every simulation, oldest first |
| `GET /v1/simulations/{id}` | state and progress; `wait`, `until`, `after_version`, `logs=N` (newest log entries, default 10), `series=false` (without per-series statistics) |
| `GET /v1/simulations/{id}/logs` | log entries: `since`, `limit` (default 1000), `level` |
| `GET /v1/simulations/{id}/runtime-metadata` | the model file and the runtime file (`config.runtime_metadata`, [above](#runtime-metadata)) |
| `GET /v1/simulations/{id}/events` | server-sent events |
| `GET /v1/simulations/{id}/results` | results: `rows`, `max_rows`, `measuring_point`, `metric` |
| `GET /v1/simulations/{id}/measurements.csv` | `measurements.csv`: `measuring_point`, `metric` |
| `POST /v1/simulations/{id}/cancel` | stop; the results so far stay readable |
| `DELETE /v1/simulations/{id}` | cancel and forget; `204` |
| `POST /mcp` | MCP over streamable HTTP |

Errors are `{"error": "..."}` with `400` (bad request), `401` (token), `403` (host or origin),
`404` (no such simulation or route), `409` (results not available yet), `415` (`POST /mcp`
without `Content-Type: application/json`) or `429` (as many simulations run and wait as the
server allows).

```sh
# a request with the files of a model directory
python3 - <<'EOF' > req.json
import json, pathlib
d = pathlib.Path("corpus/x_sl_mediastore")
files = [{"name": p.name, "content": p.read_text()} for p in sorted(d.iterdir()) if p.is_file()]
print(json.dumps({"files": files, "config": {"max_measurements": -1},
                  "termination": {"timeout_secs": 60, "confidence": {"relative_half_width": 0.02}}}))
EOF
curl -s -XPOST localhost:8080/v1/simulations -H 'content-type: application/json' -d @req.json
# {"id": "sim-1-4f2a91", "status": "loading", ...}

curl -s 'localhost:8080/v1/simulations/sim-1-4f2a91?wait=30&until=done&series=false'
curl -s 'localhost:8080/v1/simulations/sim-1-4f2a91/results?metric=Response'
curl -s 'localhost:8080/v1/simulations/sim-1-4f2a91/measurements.csv' > m.csv
```

## MCP

Tools (all results are JSON, as text and as `structuredContent`; a failure is a result with
`isError: true` and the message):

| Tool | Arguments | Returns |
|---|---|---|
| `server_info` | | version, flavours, modes, features, limits |
| `list_models` | | model directories below `--allow-dir` |
| `start_simulation` | the request: `files` or `model_dir`, `config`, `termination`, `progress`, `name` | the new simulation's state (`id`) |
| `get_status` | `id`, `recent_logs` (10), `include_series` (true) | state, progress, newest log entries |
| `wait_for_simulation` | `id`, `until` (`change`/`done`), `timeout_secs` (30, max 300), `after_version`, `recent_logs`, `include_series` | as `get_status`, after the change or the end |
| `get_logs` | `id`, `since`, `limit` (200), `min_level` (`info`) | log entries, `last_seq`, `dropped` |
| `get_results` | `id`, `series`, `include_rows`, `max_rows_per_series` (1000), `format` (`json`/`csv`), `max_csv_bytes` | results |
| `get_runtime_metadata` | `id` | the model file and the runtime file (`config.runtime_metadata`) |
| `cancel_simulation` | `id` | state |
| `list_simulations` | | every simulation |
| `delete_simulation` | `id` | `{"deleted": id}` |

Protocol: JSON-RPC 2.0, revisions `2025-06-18`, `2025-03-26` and `2024-11-05` (the client's,
if it is one of them). On stdio, messages are newline-delimited and each is handled on a thread
of its own, so a client can read the status while a `wait_for_simulation` blocks. Over HTTP,
`POST /mcp` takes one message or a batch and answers with `application/json` (`202` for
notifications); the server sends no server-initiated messages and keeps no sessions, so `GET
/mcp` is `405`.

## Limits and security

- **Binding.** The default address is `127.0.0.1`. To serve a network, bind to another address,
  set `--token` (constant-time comparison) and name the host clients use with `--allow-host`;
  put TLS in front with a reverse proxy.
- **Hosts and origins.** Every route but `/health` refuses (`403`) requests whose `Host` or
  `Origin` header names another host than `localhost`, `127.0.0.1`, `[::1]` or an
  `--allow-host` name, and the opaque origin `null` of sandboxed frames and `file:` pages,
  unless `--allow-any-origin`. `POST /mcp` takes only `application/json` bodies, which a page
  cannot send without a CORS preflight. So a web page the user visits cannot drive or read a
  local server, also not through DNS rebinding.
- **Files.** Models sent with the request are loaded from memory and never touch the file
  system. `model_dir` resolves (symbolic links included) inside the `--allow-dir` directories
  only.
- **Time and memory.** Each run has the `--max-run-secs` wall-clock cap (soft, at the next
  check, and hard 10 s later through the simulator's deadline) and the `--max-tuples` cap on
  stored tuples (16 bytes each); both end the run as `completed` with `stop_reason.kind:
  server_limit` and its results. The results of finished simulations stay in memory until
  `--max-jobs` pushes them out or a client deletes them. A queued simulation is not a thread,
  but it holds its request: at most `--max-queued` × `--max-body-mb` of waiting models. Loading
  and compiling a model have no deadline; cap the request size with `--max-body-mb`.
- **Untrusted models.** The simulator's resource limits apply (`config.max_stack_depth`,
  `max_processes`, the livelock guard; [Deviations](../correctness/deviations.md#resource-limits-for-untrusted-models)).
  For hard isolation run the server in a container with a memory limit.

## Embedding

The crate is a library as well; [`Service`](https://github.com/nk-coding/simoxide/blob/main/crates/simoxide-server/src/api.rs)
has every operation without HTTP. Its operations are synchronous, except `wait`, which is
async and needs a tokio runtime:

```rust
use simoxide_server::{JobManager, ServerConfig, Service, SimulationRequest};
use simoxide_server::api::Until;
use std::time::Duration;

let svc = Service {
    jobs: JobManager::new(ServerConfig {
        allow_dirs: vec!["corpus".into()],
        ..ServerConfig::default()
    }),
};
let req: SimulationRequest = serde_json::from_str(r#"{
    "model_dir": "x_sl_mediastore",
    "termination": {"timeout_secs": 10, "confidence": {"relative_half_width": 0.02}}
}"#)?;
let id = svc.start(req)?.id;
let done = svc.wait(&id, None, Until::Done, Duration::from_secs(60), 0).await?;
println!("{:?}", done.stop_reason);
let results = svc.results(&id, &Default::default())?;   // serde_json::Value
```

`simoxide_server::http::router(svc, HttpConfig)` is the axum router, to mount in another
server.

## What the server does not do

The server runs what `simoxide run` runs without its file outputs: no event trace, random tape,
snapshots, pauses with changes (so neither `resume --actions-at`: a job's scaling decisions
are not timed pauses, their actions apply at the decision's time), or exploration
([Command line](./cli.md)); SimuLizar's
reconfiguration rules (`--rules`) are not offered either. Use the command line or the
[library](./library.md) for those.

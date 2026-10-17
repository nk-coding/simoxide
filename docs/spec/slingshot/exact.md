# Exact comparison with Slingshot

Rules `SS-X-*`: what "the same as Slingshot" means for the Slingshot-exact mode (decision U4), how the
reference's bus trace is projected for the comparison, how the random tape maps, and how far the `det`
oracle itself is verified. Tools: `reference/slingshot/tools/project.py` (projection),
`verify-det.sh` (oracle verification), `probes.sh` (probe outputs).

## SS-X-1 The claim

For a model, a seed `N` and `simTime`, the Slingshot-exact mode gives byte-identical files to the
reference runner in `det` mode with `-seed N` (StoEx `mt`):

1. `measurements.csv` (the EDP2 rows, [SS-MEAS-1](./monitoring.md#ss-meas-1-the-measurement-path));
2. `projection.jsonl`, the level-E event projection (SS-X-2), and its subset `projection-m.jsonl`
   (level M);
3. `tape-projection.jsonl`, the draws with their sites and positions (SS-X-4);
4. for SPD runs, the decision records of `spd.jsonl` and the adjusted models (SS-SPD-6).

Target: **level E directly** (design decision U14), on every model the `det` runner reproduces,
including the models with handler errors (SS-X-3.6). Level M (measurements, aborts, SPD events, the
stop, and the tape without positions) is not a separate milestone; it is the subset to look at first
when a level-E comparison fails. The full bus trace (`trace.jsonl`) is not compared: SimOxide does not
emulate bookkeeping events (design §5.2).

## SS-X-2 The projection

`project.py RUN_DIR` reads `trace.jsonl` and writes one line per projected bus event, in dispatch
order. Keys in the order given; `t` is the event time as in the trace (Java `Double.toString`, also
for `d` and `v`); users `u` and jobs `job` are renumbered by first appearance **in the projection**.

| Bus event | Line | Keys | Level |
|---|---|---|---|
| `UsageModelPassedElement` | `pass` | `u`, `type`, `el` | E |
| `SEFFModelPassedElement` | `pass` | `u`, `ac`, `type`, `el` | E |
| `UserEntryRequested` | `call` | `u`, `role`, `sig` | E |
| `UserRequestFinished` | `call_end` | `u`, `role`, `sig` | E |
| `SEFFExternalActionCalled` | `ext` | `u`, `ac`, `role`, `sig` | E |
| `SEFFInfrastructureCalled` | `infra` | `u`, `ac`, `role`, `sig` | E |
| `ResourceDemandRequested` | `request` | `u`, `ac`, `kind`, `res`, `action`, `passive` | E |
| `JobInitiated` | `job` | `job`, `u`, `rc`, `res` (active) or `link`, `d` (the demand before the rate) | E |
| `JobFinished` | `job_end` | `job` | E |
| `JobAborted` | `job_abort` | `job` | M |
| `PassiveResourceAcquired` | `grant` | `u`, `ac`, `passive` | E |
| `PassiveResourceReleased` | `release` | `u`, `ac`, `passive` | E |
| `UserFinished` | `user_end` | `u` | E |
| `UserAborted` | `user_abort` | `u` | M |
| `MeasurementMade` | `meas` | `mp`, `metric`, `v` = `[time, value]` | M |
| `SimulationTimeReached` | `spd_time` | `tg` | M |
| `ModelAdjustmentRequested` | `spd_request` | `policy` | M |
| `ModelAdjusted` | `spd_adjusted` | `ok`, `changes`, `new`, `deleted` | M |
| `SimulationFinished` | `stop` | – | M |

Not projected (bookkeeping, or determined by the projected events): `PreSimulationConfigurationStarted`,
`SimulationStarted`, monitor set-up events, `ProbeTaken`, `MeasurementUpdated`, the cursor events
(`SEFFInterpretationProgressed`, `SEFFChildInterpretationStarted`, `SEFFInterpretationFinished`,
`UserStarted`, `UserInterpretationProgressed`, `InnerScenarioBehaviorInitiated`,
`UsageScenarioStarted`/`Finished`, `UserSlept`/`UserWokeUp`, `InterArrivalUserInitiated`,
`ClosedWorkloadUserInitiated`), the resource-internal events (`JobProgressed` and
`ProcessorSharingJobProgressed`, stale or not, `ActiveResourceStateUpdated`, `ResourceDemandCalculated`,
`ActiveResourceFinished`, `PassiveResourceStateUpdated`), `CallOverWire*`, `ResourceDemandRequestAborted`.

- **SS-X-2.1 Why these.** Every projected line marks a step whose position the exact mode needs anyway:
  the measurements depend on the order of same-time samples within a series, the tape on the order of
  handlers, and the projected lines are where the model-level state changes (user, call, demand,
  job, grant). Internal events are left out so that SimOxide may realise the cascade with fewer steps
  as long as the projected order is the same.
- **SS-X-2.2 Element keys** are PCM ids; ids created by SPD adjustments are aliased
  (`~EClass:entityName`, [Slingshot runner](../../reference-simulator/slingshot.md#outputs)).
- **SS-X-2.3 Example** (`ss_fifo_ties`, `expected/projection.jsonl`, t = 0): three `pass Start`, then
  per user `pass EntryLevelSystemCall`, `call`, then three `pass StartAction`, three `request`, three
  `job`, then the samples: the breadth-first cascade of SS-BUS-6.

## SS-X-3 What the exact mode must reproduce

From the rules of these pages, the exact mode needs, beyond the fast mode's semantics:

- **SS-X-3.1** `f64` time with Slingshot's additions (SS-ENG-2) and the FIFO cascade order
  (SS-ENG-3, SS-BUS-4 to SS-BUS-6): every Slingshot handler that produces a projected line, a draw or a
  measurement must happen at the same position. A practical implementation is a FIFO queue of
  handler steps, one per Slingshot event kind that matters (internal events may be merged where no
  projected line, draw or measurement can come between them).
- **SS-X-3.2** The result order of each handler in `det` (insertion order), as given on the area
  pages, and the direct scheduling of `MeasurementMade` before the handler's results (SS-BUS-5).
- **SS-X-3.3** The subscriber order where several handlers react to one event (SS-BUS-2 table).
- **SS-X-3.4** The PS algorithm with stale events (SS-RES-2), FCFS without bookkeeping (SS-RES-3), the
  JIFFY jobs, the lost time.
- **SS-X-3.5** Both random streams with the draw order of SS-RNG-2.
- **SS-X-3.6** All quirk flags on ([Quirks](./quirks.md)), including the disposed handlers (Q18):
  after an exception the exact mode keeps running with that handler gone.

## SS-X-4 Tape

`tape.jsonl` of the runner records every uniform: `{"k":"u","s":"stoex"|"math","i":n,"u":x,"e":event,"o":site}`
with the per-stream index `i` and the trace index `e` of the event being dispatched. The **site** `o`
(added by S1):

- for `math` draws, from the seed agent's hooks (`reference/oraclekit/seed-agent/simoxide/slingshot/seed/SiteTransformer.java`,
  `Sites.java`): `TransitionDeterminer.determineBranchTransition` and
  `determineProbabilisticBranchTransition` announce their transition list, `SimulatedLinkingResource.onJobProgressed`
  its link; the tape writes `branch:<id of the Branch or BranchAction>` or `link:<linking resource id>`.
  The hooks store a reference and change nothing else (verified: trace, measurements and draws equal
  to the runs without them on the corpus models);
- for `stoex` draws, the innermost Slingshot stack frame below the generic helpers, e.g.
  `ResourceSimulation.initiateActiveResource` (a `StackWalker` at the draw).

`project.py` writes `tape-projection.jsonl`: `{"s","i","u","o","p"}` where `o` is the SimOxide
**purpose** of the site and `p` the index of the last level-E projection line at or before the draw's
event (−1 before the first line). The purposes are those of [SS-RNG-2](./random.md#ss-rng-2-draw-sites-and-order):

| Slingshot site | `o` |
|---|---|
| `UsageScenarioInterpreter.caseStart` | `interarrival` |
| `ClosedWorkloadUserInitiated.<init>` | `think` |
| `UsageScenarioInterpreter.caseDelay` | `delay` |
| `UsageScenarioInterpreter.caseLoop`, `SeffInterpreter.caseLoopAction` | `loop` |
| `SeffInterpreter.caseCollectionIteratorAction` | `collection` |
| `SeffInterpreter.caseSetVariableAction` | `setvar` |
| `TransitionDeterminer.conditionHolds` | `guard` |
| `InfrastructureCallsContextHolder.<init>` | `infra` |
| `ResourceSimulation.initiateActiveResource` | `demand` |
| `ResourceSimulation.createWaitingJob` | `passive` |
| `PassiveResourceTable.createPassiveResources` | `capacity` |
| `AbstractActiveResource.onJobInitiated` | `rate` |
| `SimulatedLinkingResource.process` | `latency` |
| `SystemSimulationBehavior.onUserEntryRequested`, `.requestCallOverWire` | `param` |
| `RepositoryInterpreter.caseBasicComponent` | `component` |
| `UsageSimulationBehavior.onUserRequestFinished`, `SystemSimulationBehavior.onCallOverWireSucceeded` | `return` |
| `branch:<id>`, `link:<id>` (stream `math`) | unchanged |
| anything else | `?` |

- **SS-X-4.1 Mapping to SimOxide's tape** ([formats](../../guide/formats.md) §3): stream `stoex` is
  SimOxide's single stream of the SimuLizar mode (same MT seeding); stream `math` is new
  (`java.util.Random(N + 1000003)`, SS-RNG-1.2). The purposes are SimOxide's origin purposes without
  the element id, plus `latency` (SimOxide counts it under `rate`), `component`, `passive`, `capacity`.
- **SS-X-4.2** A level-M comparison compares `(s, i, u, o)`; level E adds `p`.

## SS-X-5 Running a comparison

```sh
reference/slingshot/run.sh -model DIR -out REF -simTime T -seed N        # det oracle
reference/slingshot/tools/project.py REF                                 # projection*.jsonl, tape-projection.jsonl
simoxide run --model DIR --flavour slingshot-exact --seed N --max-sim-time T \
    --trace OUT/projection.jsonl --tape OUT/tape-projection.jsonl --measurements OUT/measurements.csv
cmp REF/measurements.csv OUT/measurements.csv; cmp REF/projection.jsonl OUT/projection.jsonl; ...
reference/slingshot/tools/compare_exact.sh [NAME...]                      # the same for corpus-ss/
```

## SS-X-6 det verification

`verify-det.sh` runs each model in `det` mode twice in fresh JVMs and once warm (2 runs in one JVM),
and once in `stock` mode with the same seed, and compares all outputs
(`reference/slingshot/results/det-verify.csv`). Simulated time: the end time of refsim's run of the
corpus model (rounded up) or the probe's `probe.json`; seed: the corpus `run.json` seed.

Results (2026-10-17, nightly `1.0.0.202609010045`):

| Set | Models | `det` 2 fresh JVMs identical | warm = cold | stock measurements = `det` |
|---|---|---|---|---|
| probes (`models/probes/`) | 20 | 20 | 20 | 20 |
| SPD examples (`spd_minimal`, `spd_scale_in`) | 2 | 2 | 2 | 2 |
| corpus `h*` (hand-made, SimuLizar features) | 33 | 33 | 33 | 32 (`h24_two_scenarios` differs) |
| corpus `x_*` (Palladio, SimuLizar and Slingshot examples) | 33 | 30 run, all identical | 30 | 29 (`x_espresso` differs) |

- **`det` is byte-reproducible** (trace, tape, SPD log, measurements) on every model that runs: 85 of
  88. The other three never produce a run: `x_pem_acquire` and `x_sl_runconfigtest` abort before
  t = 0 (a passive capacity that reads a component parameter, SS-RES-4.1; a linking resource without a
  communication specification); `x_pem_fork` is a zero-time livelock (SS-RUN-5) and is stopped by the
  verification's time limit.
- **Stock Slingshot is not reproducible**: with the same seed its traces differ between runs
  (same-time order), and on `h24_two_scenarios` (two scenarios on one FCFS CPU) and `x_espresso` (30
  closed users, constant demands) even the measurements differ from `det`.
- **The instrumentation is neutral**: the draw-site hooks and the subscriber log leave trace,
  measurements and draws unchanged (checked on `h14`, `h22`; the smoke column
  `instrumentation_neutral`).
- **Slingshot fails on many corpus models**, without reporting an error: 22 of the 88 models have
  handler exceptions (SS-BUS-7), 13 record no measurement at all. Of the 66 corpus models, 38 run
  without handler errors, aborts or empty output. Column `handler_errors` names the first exception;
  the causes are the quirks Q17 (`x_pem_screencast_ms*`), Q19, Q21, Q22 (`h10`, `h27`), Q23 (`h11`,
  `h12`, `h26`, `x_sl_simulizar154`), Q24 (`h13`, `h28`, `x_pem_harddisk`, `x_sl_mediastore`), Q30
  (`h15`, `x_pem_subsystem*`, `x_pem_replicated_composite`, `x_pem_simple_optimisation_composites`),
  Q31 (`h18`, `x_pem_infrastructure`, `x_pem_hdd_infrastructure_calls`), Q25 (`h04`, `h25`,
  `x_sl_elsc_return`: every request aborts), Q35 (`x_pem_reliability`).
- Everything that `det` changes is one of stock Slingshot's possible orders (SS-ND-2), so the oracle
  is a valid Slingshot run.


## SS-X-7 Probes

`reference/slingshot/models/probes/<name>/` holds the model (generated by `tools/probegen.py`),
`probe.json` (simulated time, seed, rules, description) and `expected/` (the `det` outputs and their
projections, `errors.txt` with the handler exceptions, `bus.jsonl` with the observed subscriber
order). `probes.sh` regenerates `expected/`, `probes.sh --check` compares.

| Probe | Rules | What it shows |
|---|---|---|
| `ss_allocation_first` | SS-ACT-9.1 | assembly allocated twice: the first allocation (rate 2) serves |
| `ss_async_fork` | SS-ACT-10.1 | asynchronous behaviour ignored; one synchronous behaviour works |
| `ss_branches` | SS-ACT-5.1, SS-WL-4.2, SS-RNG-2 | usage and SEFF branch draws with sites; guarded branch takes the first true guard |
| `ss_closed_think` | SS-WL-3.1, SS-RNG-2 | users start at 0 without think time; think time drawn at `UserFinished` |
| `ss_delay_resource` | SS-RES-1.2, SS-WL-5.1, SS-Q25 | Delay resource missing: job and user aborted; closed user restarts as a new user |
| `ss_fcfs_replicas` | SS-RES-3.1, SS-RES-3.4 | two replicas, one server; full demand from the head |
| `ss_fifo_ties` | SS-ENG-3, SS-BUS-6, SS-RES-2.4, SS-RES-2.6, SS-MEAS-3.2 | FIFO cascade at ties; PS ties to the earliest job; stale samples at t = 1, 2 |
| `ss_guard_none` | SS-ACT-5.3, SS-Q21, SS-BUS-7 | no true guard: `IllegalStateException`, no SEFF progresses afterwards |
| `ss_link` | SS-ACT-7, SS-RES-5, SS-RNG-1.2, SS-Q28 | request and reply transfers BYTESIZE / throughput + latency; one `link:` draw per transfer |
| `ss_loop_zero` | SS-ACT-6.2, SS-Q19, SS-BUS-7 | count 0: body once, then `NoSuchElementException`; no SEFF progresses afterwards |
| `ss_monitors` | SS-MEAS-2.2, SS-Q29 | only the scenario monitor records; ELSC, system-operation and entry-level assembly-operation monitors do not |
| `ss_open_draws` | SS-WL-2.1, SS-RNG-2 | next inter-arrival drawn before the arriving user's demand |
| `ss_passive` | SS-RES-4, SS-MEAS-4.2 | FIFO grants; waiting, holding and state series |
| `ss_ps_multicore` | SS-RES-2.7, SS-MEAS-3.1, SS-MEAS-3.3, SS-Q34 | state = all jobs, utilisation = busy cores / cores; replicaID 1 = replicaID 0 |
| `ss_role_lookup` | SS-ACT-1.2, SS-ACT-2.2, SS-MEAS-2.3, SS-Q20, SS-Q33 | entry call to `FrontA` instead of `FrontB`; `FrontB` calls `Fast` via `FrontA`'s connector; assembly-operation keys |
| `ss_stop_time` | SS-RUN-3, SS-RUN-4 | arrival at exactly `simTime` = 10 not dispatched; no final sample |
| `ss_two_demands` | SS-ACT-3.2, SS-Q17, SS-BUS-7 | CPU and HDD demand concurrent; request ends at 1.75 before the HDD; NPE disposes `onActiveResourceFinished` |
| `ss_usage_delay` | SS-WL-4.1 | usage delay drawn at interpretation; `UserSlept`/`UserWokeUp` |
| `ss_usage_loop_zero` | SS-WL-4.3, SS-Q19, SS-BUS-7 | usage loop count 0: builder throws; later users stop after their `Start` |
| `ss_zero_demand` | SS-RES-2.2, SS-RES-3.4, SS-MEAS-3.4 | PS zero demand = 1e-9 s job; FCFS zero demand at once; demand tuple 0 |

## SS-X-8 The Slingshot-exact corpus

`corpus-ss/` (phase X1) is the corpus the Slingshot-exact mode is checked against (V3x). Each case
is a directory `corpus-ss/<name>/` with

- `case.json`: `model` (the model directory, relative to the repository root), `seed`,
  `sim_time` (whole seconds) and `group`;
- `expected/`: the `det` runner's outputs for that seed and simulated time: `measurements.csv`,
  `projection.jsonl` (level E; level M is the subset of the lines whose `ev` is `job_abort`,
  `user_abort`, `meas`, `spd_*` or `stop`), `tape-projection.jsonl`, `errors.txt` (the handler
  exceptions in order, `tools/errors.py`), `summary.json`, `spd.jsonl` (SPD runs),
  `trace.jsonl.gz` (the full bus trace, for debugging only) and, for runs that end before t = 0,
  `error.txt`.

`reference/slingshot/corpus-ss.sh` writes `expected/`; `corpus-ss.sh --check` runs every case
again in a fresh JVM and compares all files. Seeds and simulated times are those of
`results/det-verify.csv` (SS-X-6).

| Group | Cases | What |
|---|---|---|
| `probe` | 20 | the specification probes (SS-X-7); 4 with handler errors |
| `spd` | 3 | `spd_minimal`, `spd_scale_in`, `spd_time_policies` (`reference/slingshot/models/`) |
| `corpus` | 38 | the corpus models that run without handler errors, aborts or empty output |
| `failure` | 25 | the corpus models with handler errors (18), aborts (`h04`, `h25`, `x_sl_elsc_return`) or no output (`h15`, `x_pem_reliability`, `x_pem_subsystem*`) |
| `setup-error` | 2 | `x_pem_acquire`, `x_sl_runconfigtest`: the run fails before t = 0 (`error.txt`) |
| `x2` | 2 | `ss_q37_fallback` (a link touching only one of the containers is used), `ss_q37_none` (no link: `IllegalArgumentException` disposes `onCallOverWireRequested`), models in `reference/slingshot/models/x2/`, added in X2 because no corpus model has quirk Q37 |

Left out: `x_pem_fork`, a zero-time livelock without a reference result (SS-RUN-5).

**Reproducibility (X1, 2026-10-17; the two `x2` cases in X2).** `corpus-ss.sh --check` reproduced all 88 cases byte for byte
in fresh JVMs (projection, tape projection, measurements, SPD log, summary, full trace). The only
JVM-dependent bytes were identity hash codes inside three exception messages
(`SEFFInterpretationContext@1b2c3d`, Q31); `tools/errors.py` writes them as `@*`. The 22 cases
with handler errors are the 4 probes `ss_guard_none`, `ss_loop_zero`, `ss_two_demands`,
`ss_usage_loop_zero` and the corpus models `h10`, `h11`, `h12`, `h13`, `h18`, `h26`, `h27`, `h28`,
`x_pem_harddisk`, `x_pem_hdd_infrastructure_calls`, `x_pem_infrastructure`,
`x_pem_replicated_composite`, `x_pem_screencast_ms*` (3), `x_pem_simple_optimisation_composites`,
`x_sl_mediastore`, `x_sl_simulizar154`.

## SS-X-9 Exactness limits of the EPL build

Two Java behaviours that change values are defined only by OpenJDK's sources; the default (EPL)
build of the exact mode does not reproduce them:

- **`java.util.HashMap` iteration order** of SimuCom's stack frames (`getContents()`): the order
  in which a `CollectionIteratorAction` evaluates several `INNER` characterisations
  (SS-ACT-6.3) and in which BYTESIZE entries are summed (SS-ACT-7.1). The exact mode iterates in
  first-insertion order. Only models with two or more such entries in one frame are affected
  (corpus `h27_collection_inner_multi`).
- **HotSpot's `Math.log` and `Math.exp` intrinsics**, which Palladio's lognormal distributions
  (`Math.exp`, `Math.log` in `LognormMoments`) and the StoEx `Log` function call. The EPL build
  rounds correctly; the intrinsics differ in the last bit for rare arguments (one sample of
  `h21_stoex_distributions`). Builds with the opt-in `hotspot-math` feature reproduce them.

Everything else the reference computes, including Commons Math 3's `FastMath` (SS-RNG-1.1), is
pure Java and reproduced bit for bit.


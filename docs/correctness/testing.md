# Testing

SimOxide is tested in layers. The default suite runs everything that needs no Java; the heavy
tests, the fuzzing campaigns and the statistical campaigns run on demand. The APIs behind the
harness are described in [Test kit](../development/testkit.md).

## Layers

| Layer | Where | Oracle | Run |
|---|---|---|---|
| Unit tests | `crates/*/src` `#[cfg(test)]` | hand-written | `cargo test --release --workspace` |
| Golden values | `crates/simoxide-{stoex,random,model}/tests/golden*.rs`, `jmath_golden.rs` | values dumped from the Java libraries | same |
| Scheduler oracle and properties | `crates/simoxide-sched/tests/{oracle,props,ps_vt}.rs` | Java scheduler traces; invariants | same |
| Corpus, exact | `crates/simoxide-sim/tests/corpus.rs` | `corpus/*/expected` (reference), in tape-replay and own-RNG mode | same |
| Sliding windows, triggers, own monitors, fuzz regressions | `corpus.rs`: `tests/models/{w,t,u}_*`, `tests/models-replay/*`, `corpus-fuzz/*` | reference | same |
| Reference aborts | `bugs.rs`: `corpus-fuzz/{k_bug2,l_ref}_*` (error and trace prefix up to the abort) | reference partial trace | same |
| Metamorphic properties | `crates/simoxide-sim/tests/metamorphic.rs` | SimOxide itself | same |
| API properties | `crates/simoxide-sim/tests/api.rs`, `interp.rs` | SimOxide itself | same |
| Engine shortcuts | `crates/simoxide-sim/tests/literal_engine.rs` | DESMO-J's literal event sequence | same; more: `LITERAL_N=1000` |
| Pause, fork and resume | `crates/simoxide-sim/tests/fork.rs` | the uninterrupted run | same; `FORK_FILTER=a,b` |
| Model changes, no-op | `crates/simoxide-sim/tests/model_change.rs` | the uninterrupted run | same |
| Load profiles | `crates/simoxide-sim/tests/load_profile.rs` | the model with the scaled rate; arrival times derived by hand; the uninterrupted run | same |
| Model changes | `crates/simoxide-reconf/tests/changes.rs` | the changed model run from the start; SimuLizar's syncer rules | same |
| Robustness (untrusted input) | `crates/simoxide-sim/tests/robustness.rs` | no panic; errors of the right kind | same; more: `ROBUST_N=100000 ROBUST_SEED=n` |
| Model generator | `crates/simoxide-testkit/tests/modelgen.rs` | models load without errors; every feature occurs | same |
| Fast-mode samplers and generator | `crates/simoxide-random/tests/fast.rs`, `crates/simoxide-stoex/tests/fast.rs` | reference CDFs, reference errors | same |
| Fast-mode engine and API | `crates/simoxide-sim/tests/fast_mode.rs` | the exact engine with the same random numbers | same; more: `FAST_ENGINE_N=3000` |
| Statistical validation | `crates/simoxide-sim/tests/statistical.rs` (`#[ignore]`) | queueing theory, both modes | `crates/simoxide-testkit/scripts/heavy-tests.sh` |
| Differential fuzzing | `simoxide-fuzz fuzz` | reference: trace, tape and measurements | [below](#differential-fuzzing) |
| Long runs | `simoxide-fuzz long` | reference: measurements | [below](#long-runs) |
| Fast-mode equivalence | `simoxide-fuzz equiv` | the exact mode, statistically | [below](#fast-mode) |

`crates/simoxide-testkit/scripts/heavy-tests.sh [--fuzz]` runs everything that is `#[ignore]`d:
the statistical tests, the analytic scheduler tests, the 10^6-sample RNG and `Math.log` goldens,
the check that refsim reproduces the corpus, and optionally a 200-model fuzz campaign.

## Exact comparison with the reference

Every corpus model runs in two modes and is compared byte for byte with the reference's
`trace.jsonl`, `tape.jsonl` and `measurements.csv`:

- **Tape replay.** SimOxide takes the uniforms from the reference's tape. This isolates the
  engine: a divergence cannot come from the random number generator.
- **Own RNG.** SimOxide draws its own numbers. This checks the generator and every distribution
  as well.

The corpus is described in [Model corpus](../reference-simulator/corpus.md). The first
divergence is reported with context: the preceding trace lines, the differing fields, the
process, its spawn and its open `begin` elements.

## Metamorphic properties

Each property is checked on every corpus model and on 30 generated models. No reference is
needed, so these run in the normal suite (under 1 s in release).

| Test | Property |
|---|---|
| `deterministic_and_trace_independent` | The same seed twice gives the same trace, tape and measurements. A run without trace and tape gives the same measurements, draw count, event count and end time. |
| `replaying_own_tape_reproduces_the_run` | Replaying the run's own tape, with origin checks, reproduces the run. |
| `batch_equals_sequential` | `run_batch` with 3 seeds equals 3 sequential runs. |
| `longer_runs_have_the_same_past` | A run with doubled `max_measurements` or `max_sim_time` has the same measurements before the shorter run's stop time. |
| `scaling_demands_and_rates_by_two_changes_nothing` | Every demand, processing rate and resource-call count wrapped as `(…) * 2` changes nothing except the abstract demand and the `Resource Demand Tuple` series. |
| `entry_file_order_does_not_matter` | Reversing the order of the entry files changes nothing. |
| `unused_component_and_entity_names_change_nothing` | An unused component appended to the repository, and a prefix on every `entityName`, change nothing (except the passive-resource measuring-point string, which contains entity names as in the reference). |

`tests/interp.rs` also checks that runs with and without a trace agree in both modes.

## Engine shortcuts

The interpreter takes shortcuts that DESMO-J does not: a process woken by the end of a wait runs
inside the waking event when its `Resume` event would be the next one anyway, and `INNER`
characterisations are evaluated in place. `tests/literal_engine.rs` checks that these change
nothing observable. It runs every model directory, and 60 generated models (1 000 with
`LITERAL_N=1000`), once with the shortcuts and once on DESMO-J's literal event sequence
(`compat::Literal`), and compares traces, tapes, measurements, event counts and errors (message,
time, trace prefix), including aborting runs, replayed and truncated tapes, and tight livelock and
event limits.

## Pause, fork and resume

`tests/fork.rs` pauses runs between events, forks them, takes the state out and resumes it (also
as a deep copy on another thread), and compares the continuations with the uninterrupted run:
trace, tape, measurements, event, draw and request counts and errors, byte for byte. It runs
every model directory with its `run.json` and with a longer seed-7 configuration, at 14 pause
points each (event counts, among them random ones that split same-time instants, and times), in
exact mode with and without outputs, in tape replay, in fast mode and on the literal engine. It
also checks attaching a tape and dropping the trace at a pause, failed runs, the model check of
`resume` and per-fork limits.

## Model changes

`tests/model_change.rs` applies, on every model directory and at four pause points, a change set
whose graph is unchanged: the whole change path runs (typed rebuild with stable ids,
recompilation, merge, run-state adoption), and the continuation, a fork of it, a resumed state
and a deep copy on another thread must equal the uninterrupted run byte for byte, in exact and
fast mode. `simoxide-reconf/tests/changes.rs` edits models through the model graph's `ModelStore` API:
changes before the start (rates, latencies, demands, branch probabilities) give exactly the run
of the model changed on disk; changes at pause points (an added container with an allocation, a
composite moved to a new container, a rewired connector, rate and demand changes, a new route)
move the load as the [model change rules](../spec/model-changes.md) say; a removed container
keeps simulating (the run equals the uninterrupted one); changes SimuLizar aborts on, and
unsupported ones, leave the run unchanged; forks with changes equal their counterparts, also in
fast mode.

## Scaling policies

`simoxide-spd` ([Scaling policies](../guide/spd.md)) has three test files:

- `tests/unit.rs` (V1): the comparator, time triggers, cooldown (also `t == coolDownEnd`),
  interval (`t % delta == 0`, repeated and once), thrashing, the EMA emit rule, the 10-value
  window, the target-group check, response-time and task-count triggers, composed triggers, and
  every compat flag in both settings, against values derived by hand from Slingshot's code, with
  a fake host; forks of the SPD state continue like the original.
- `tests/reference.rs`: the decision log of the time-triggered models of the Slingshot
  reference runner (`spd_minimal`, `spd_scale_in`, `spd_time_policies`, the `spd_or_trigger`
  and `spd_and_trigger` reproductions, the 20 probes) equals the runner's `spd.jsonl` (det
  mode), projected, with all compat flags on: 83 records on the five SPD models. The adjustment
  is not executed in this crate (it is `simoxide-reconf`'s, below): a test double reports the reference's outcome of each adjustment
  (success, the target group's size after it). With the fixed behaviour the logs equal what
  the evident intent gives (derived by hand per model).
- `tests/sim.rs`: in a simoxide-sim run the observers deliver the measurements the triggers
  read, the outputs equal the plain run's (no adjuster changes the model), a paused and forked
  run continues like the uninterrupted one, and enforced policies queue their requests.

The adjustments (Slingshot's `MainTransformation`) are tested in `simoxide-reconf`'s
`tests/spd_adjust.rs` against the SPD corpus `corpus-spd/` (each case a model of
`reference/slingshot/models`, a seed and a simulated time; `expected/` holds the reference
runner's `det` outputs, `CORPUS=corpus-spd reference/slingshot/corpus-ss.sh [--check]`):

- with all compat flags on, the decision log equals the runner's `spd.jsonl`, the adjustments'
  change counts and model summaries (containers, links, allocation, target-group elements, in
  model order) included, in QVTo's ordered and fast mode (V3): `spd_minimal` (1 adjustment),
  `spd_time_policies` (8), `spd_service_group` (3, top-down); `spd_scale_in` up to its scale-in
  (F4);
- the default behaviour runs the patched transformations (Q6: the minimal example's links);
  dry runs on the QVTo engine's SPD corpus show the Q4 and Q7 fixes;
- snapshots with adjustments (sections `engine`, `model`, `spd`, `qvto`) restored into fresh
  runs continue like the uninterrupted run, and a restored run writes the bytes it was read
  from;
- repeated adjustments add the same number of objects each (bounded growth);
- the Slingshot flavour refuses the changes until F4.

`simoxide-cli`'s `tests/spd_cli.rs` (feature `reconfiguration`) runs the minimal example in new
processes: the scale-out, a resume after it, `--enforce`.

The HPA autoscaler ([Autoscaling (HPA)](../guide/hpa.md)) has no reference to compare with.
Its tests check Kubernetes' rules directly:

- `simoxide-spd`'s unit tests (`hpa::controller`, `hpa::config`) check the controller's
  arithmetic against hand-computed Kubernetes behaviour:
  - the proportional rule and the inclusive tolerance band, on the integer percentage;
  - unready and missing pods;
  - both stabilisation paths and both rate-limit paths (`Pods`/`Percent`, `Max`/`Min`/
    `Disabled`, periods);
  - the min/max bypass, starting pods, and the defaulting of `behavior`.
- `simoxide-reconf`'s `tests/hpa.rs` runs the V4 model with an `.hpa.json` and load steps from
  change files:
  - a step scales out within four syncs and in no earlier than the 300 s window allows, which
    every scale-down of the log is checked against, without flapping;
  - a ramp scales monotonically;
  - starting pods are unready until their delay;
  - scaling policies limit the rate;
  - the Slingshot flavour scales too;
  - forks and snapshots at nine pauses continue like the uninterrupted run;
  - the growth is bounded per created pod, with the `maxCreatedPods` guard;
  - configuration errors, and parameter changes at a pause.
- `simoxide-cli`'s `tests/hpa_cli.rs` checks run, pause and resume, `--hpa-set`, `hpa` change
  lines and the refusals.

## Differential fuzzing

`simoxide_testkit::modelgen` generates random, structurally valid PCM models: composite
components with delegation, call graphs, double assemblies with parameters; branches, loops,
collection iterators, forks, passive resources, set variables, infrastructure and resource calls;
CPUs, HDDs and delay resources with replicas, linking resources with partial routes; open and
closed workloads; the StoEx distributions, PMF and PDF literals and parametric dependencies.
`simoxide-fuzz fuzz` runs each model in the reference and in SimOxide, in both RNG modes, and
compares trace, tape and measurements exactly.

Optional features raise the probability of rare situations:

| Feature | Default | What it produces |
|---|---|---|
| `heavy_load` | 0.15 | utilisation 0.75 to 0.92: long queues, many requests in flight at the stop |
| `ties` | 0.15 | equal constant demands and times in multiples of 10 ms: simultaneous events, events exactly at `max_sim_time` |
| `deep_nesting` | 0.15 | nesting depth 5, forks in forks, larger SEFFs |
| `stoex_exotic` | 0.25 | PMFs that need normalising or sorting, `Pois`, integer overflow and division, `?:`, `^`, `BoolPMF` guards, sub-ns demands, degenerate branch probabilities, empty collections |
| `double_resume` | 0.1 | sync forks whose children never wait ([REF-2](./reference-bugs.md#ref-2-double-resumes-abort)) |
| `hdd_rw` | 0.5 | HDD read/write rates and read/write resource calls |
| `param_override` | 0.3 | component parameters with distributions and `BYTESIZE`, overridden on assembly contexts |
| `windows` | 0.15 | sliding-window utilisation monitors, TimeDrivenAggregation |
| `long_run` | 0 | 300 to 3 000 measurements |
| `nested_composite` | 0.4 | two levels of provided delegation |
| `extra_monitors` | 0.2 | response-time monitors on assembly operations |
| `triggers` | 0.3 | `triggersSelfAdaptations = true`, reconfiguration-time and container-count monitors |
| `prm_aggregation` | 0.3 | with `triggers`: Fixed/VariableSizeAggregation instead of FeedThrough |
| `nested_container` | 0.1 | nested resource containers, sometimes with an allocation inside ([REF-8](./reference-bugs.md#ref-8-nested-resource-containers-are-not-simulated)) |
| `middleware_stream` | 0.1 | middleware marshalling, sometimes without the stream ([REF-10](./reference-bugs.md#ref-10-middleware-marshalling-needs-stream-bytesize)) |

The generator avoids what the reference cannot run at all: think time 0 (livelock), monitors on
infrastructure signatures, time-only stops under heavy load, recursion through monitored calls,
guarded branches without a true guard, `SetVariable` without a result frame, missing links
between communicating containers, `Binom`, and `Min`/`Max` with mixed argument types.

### Results

About 12 000 generated models have been compared with the reference, most of them in both RNG
modes, with default features, with the optional features boosted, and with all 15 optional
features at probability 1:

| Campaign type | Models | Reference ran | Divergent |
|---|---|---|---|
| default features | about 6 000 | 98 % | 0 |
| heavy (load, ties, nesting, exotic StoEx, double resumes, windows) | about 3 400 | 96 % | 0 |
| triggers, nested containers, middleware, aggregation boosted | about 1 800 | 90 % | 0 |
| all 15 optional features at 1 | 700 | 62 % | 0 |

Where the reference aborts (a [reference bug](./reference-bugs.md) or a model it cannot run),
SimOxide aborts too, with the same error at the same time and an identical trace prefix
(`reference/reffail-cmp.sh`). Every divergence found during development was fixed, and the
model was saved as a regression case in `corpus-fuzz/`. No divergence is open.

Optimisations are accepted only if they keep every output byte-identical to the previous build:
the golden hashes of `crates/simoxide-cli/bench/golden.sh`, the corpus in both modes, and 1 400
generated models in three run configurations each (see
[Contributing](../development/contributing.md#guardrails)).

### Running a campaign

```sh
cargo build --release -p simoxide-cli -p simoxide-testkit
JVM_OPTS="-Xmx1g -XX:+UseSerialGC" ./target/release/simoxide-fuzz fuzz --n 400 --seed 600000 --sizes 1..10 \
    [--set heavy_load=0.5,ties=0.5,stoex_exotic=0.7] [--classic] \
    --coverage cov.txt --save-ref-failures --out corpus-fuzz \
    --sim "cmd:$PWD/target/release/simoxide run --model {dir} --run-json {run_json} --name {name} \
           --trace {trace} --tape {tape} --measurements {measurements} {replay:--replay-tape}"
```

- Each model runs in both modes: tape replay, then own RNG.
- A divergence is saved as a corpus entry in `corpus-fuzz/<name>/`, and a greedy minimiser saves
  the smallest diverging variant next to it.
- `--coverage` writes counts of trace event kinds, element types, StoEx functions and metrics.
- `--save-ref-failures` saves models whose reference run aborts. `reference/reffail-cmp.sh
  SIMOXIDE DIR...` checks that the candidate follows the reference's partial trace up to the
  abort.
- Put a per-file size limit on long campaigns (`ulimit -f`): a model that runs away writes
  gigabytes of trace.
- Never run the reference under `ulimit -v`. SimuLizar needs one thread per simulated process;
  when thread creation fails, it can end the simulation early and still report success. The
  harness rejects a batch whose stderr shows such failures. Cap the heap instead
  (`JVM_OPTS="-Xmx1g -XX:+UseSerialGC"`). Six workers at `-Xmx1g` fit in 34 GB.

## Long runs

`simoxide-fuzz long` runs models with trace and tape off, in the reference (warm JVM batches)
and in SimOxide, and compares the measurements exactly:

```sh
simoxide-fuzz long --corpus corpus --max-measurements 5000 --sim "cmd:.../simoxide run --model {dir} \
    --run-json {run_json} --name {name} --measurements {measurements}"
simoxide-fuzz long --gen 100 --seed 1 --sizes 1..10 --set heavy_load=0.5 --max-measurements 20000 --sim ...
```

All corpus models give identical measurements at 5 000 measurements, except
`x_pem_screencast_ms_instant`, which takes the reference longer than the 60 s timeout. Runs of
20 000 measurements (mediastore: 2.2 million tuples) are identical as well.

## Statistical validation

`statistical.rs` builds PCM models of classic queueing systems. Each run completes 1.47 million
users (`STAT_N` overrides) and is compared with closed-form results: the mean response time must
lie within a 99.9 % batch-means confidence interval (40 batches) plus 0.5 % for the transient;
utilisation and throughput within a relative tolerance. All checks run in exact and in fast mode.

| Test | Model | Expected | Measured (exact mode) |
|---|---|---|---|
| `mm1_processor_sharing` | M/M/1-PS, ρ = 0.7 | R = 3.3333 | 3.3164 ± 0.058 |
| `mm1_fcfs` | M/M/1-FCFS | 3.3333 | 3.3217 ± 0.057 |
| `md1_fcfs` | M/D/1-FCFS (Pollaczek-Khinchine) | 2.1667 | 2.1778 ± 0.019 |
| `mg1_ps_insensitivity` | M/G/1-PS with constant, `UniDouble`, `LognormMoments(1,2)` | 2.5 each | 2.497 / 2.503 / 2.489 |
| `mmc_ps` | 3-replica PS CPU (Erlang C) | 2.0787 | 2.0973 ± 0.038 |
| `fcfs_ignores_replicas` | 3-replica FCFS = M/M/1 ([REF-1](./reference-bugs.md#ref-1-fcfs-ignores-numberofreplicas)) | 3.3333 | 3.3233 ± 0.045 |
| `open_tandem_jackson` | PS CPU → FCFS HDD | 5.0 | 4.984 ± 0.108 |
| `closed_network_mva` | closed, N = 1, 5, 12 (exact MVA) | e.g. 1.9146 (N = 12) | 1.9140 ± 0.010 |
| `passive_resource_semaphore_is_mmc` | semaphore(3) around a delay = M/M/3 | 2.0787 | 2.1013 ± 0.054 |
| `fork_join` | E[max of 2 Exp(1)] on a delay; 2 × M/M/1 (Nelson-Tantawi) | 1.5; 2.875 | 1.4992; 2.8716 |

## Fast mode

The [fast mode](../guide/fast-mode.md) cannot be compared bit for bit. It is checked in three
layers.

**1. Components.** Each fast sampler is tested against the distribution it samples: a chi-square
test over 10^6 samples with cell probabilities from the reference CDFs, and mean and variance
within 6 standard errors, for `Norm`, `Exp`, `Gamma` (shapes 0.3 to 150), `Lognorm`,
`LognormMoments`, `GammaMoments`, `UniDouble`, `UniInt` and `Pois` (m = 0.3 to 1000, tested as
Poisson − 1). The generator is tested for uniformity, pairs, lag-1 correlation, bit balance and
seed independence. On every one of the 154 parameter sets of the Java oracle, the fast sampler
fails exactly when the reference fails, with the same error, and draws nothing on an error.

**2. The engine is the exact engine.** With the same seed, `Fast` and `Literal<Fast>` (the fast
random numbers on DESMO-J's literal event sequence) give byte-identical measurements, end time,
request, draw and event counts on every model directory of the repository (two seeds each) and
on generated models (60 in the normal suite, 1 000 with `FAST_ENGINE_N=1000`). Together with the
engine-shortcut check above, the only difference between the modes is the random numbers. The
corpus models without random draws give the exact mode's measurements byte for byte.

**3. Whole simulations, statistically** (`simoxide_testkit::equiv`, `simoxide-fuzz equiv`). Every
model runs with N seeds in each mode. Per measuring point and metric, the harness compares the
per-run tuple count, mean, p50, p90 and p99 (Welch t-test and Mann-Whitney U), the pooled values
(two-sample KS with a permutation test over whole runs, valid for autocorrelated tuples), and per
model the finished requests, the end time and the abort rate (Fisher). State-like series are
compared over time (time-weighted mean, quantiles and KS of the value at evenly spaced times).
Values within `1e-8 + 1e-9·|x|` count as ties. All p-values of a campaign form one Holm family
at alpha 0.01.

```sh
cargo build --release -p simoxide-testkit
./target/release/simoxide-fuzz equiv --corpus corpus,corpus-fuzz,crates/simoxide-sim/tests/models,crates/simoxide-cli/bench/models --seeds 100
./target/release/simoxide-fuzz equiv --gen 1000 --seed 1 --sizes 1..10 --seeds 40 --tests-csv tests.csv
./target/release/simoxide-fuzz equiv --gen 1000 --seed 1 --modes exact,exact --seed-offset-b 100000   # control
```

| Campaign | Models | Seeds per mode | Tests | Holm failures | raw p < 0.01 | raw p < 0.001 |
|---|---|---|---|---|---|---|
| all model directories | 96 | 100 | 4 931 | 0 | 0.75 % | 0 |
| the same, ≥ 2 000 measurements per run | 96 | 50 | 4 931 | 0 | 0.04 % | 0 |
| generated, default features | 1 000 | 40 | 77 422 | 0 | 0.36 % | 0.03 % |
| generated, all 15 optional features = 1 | 300 | 40 | 17 525 | 0 | 0.15 % | 0.01 % |
| control: exact vs exact (other seeds), generated | 1 000 | 40 | 77 428 | 0 | 0.35 % | 0.03 % |
| analytical checks, fast mode | 10 queueing models | 1.47 M users | – | 0 of 10 | – | – |

The tests are conservative (the fraction below q is under q), and exact vs fast looks like exact
vs exact. Every model with a small p-value was rerun with 300 to 3 000 fresh seeds, and none kept
a difference.

## Regression tests

`crates/simoxide-sim/tests/bugs.rs` guards against bugs that were found in SimOxide itself:

| Test | Guards |
|---|---|
| `bug1_*` | Java's `Double.toString` renders at least two significant digits. For a subnormal whose shortest decimal has one digit it picks the closest two-digit decimal (`4.9E-324`, not `5.0E-324`); checked against 24 859 Java 21 values and on a model that samples `GammaMoments(0.005, 3.0)` |
| `bug2_fcfs_double_queue_aborts` | a process queued twice at an FCFS resource aborts like the reference, after the same trace ([REF-2](./reference-bugs.md#ref-2-double-resumes-abort)) |
| `reference_aborts_are_reproduced` | the saved reference aborts of `corpus-fuzz/l_ref_*`: same error, same trace prefix |

`tests/double_resume.rs` checks the opt-in fix of REF-2 (`SimConfig::fix_double_resume`): by default
the double resume of corpus model h26 and of a variant that aborts are kept; with the fix both run like
their one-child variants, in both modes, also paused, forked and snapshotted; fork models without a
late resume run byte for byte as without it.

`tests/robustness.rs` covers pathological inputs: 600 mutated corpus models, 20 000 random StoEx
strings, deep StoEx on a 1 MiB stack, self-containing composites, zero-time recursion, a
2·10^9-iteration loop, zero inter-arrival time and cancellation. Each ends in an error of the
right kind, and the limits leave the results of other runs unchanged. 300 000 further mutants
(`ROBUST_N=100000`, three seeds) ran without a panic.

## Coverage

Line coverage of the library crates with `cargo llvm-cov`, measured on 2026-09-29 (the default
suite, and the suite plus 2 540 fuzz models):

| Crate | Suite | Suite + fuzz |
|---|---|---|
| simoxide-model | 86.6 % | 86.6 % |
| simoxide-random | 85.1 % | 85.1 % |
| simoxide-sched | 87.5 % | 87.5 % |
| simoxide-sim | 90.8 % | 91.2 % |
| simoxide-stoex | 92.3 % | 92.3 % |
| total | 88.8 % | 88.9 % |

The corpus and the metamorphic tests reach almost all simulation code. Not executed: error paths
for invalid models (unbound roles, unsupported connectors, missing start actions), dynamic
assembly-context paths (every path the corpus and the generator produce is precompiled), overflow
branches of the sliding-window arithmetic, and unused convenience APIs.

## Tools

| Tool | Purpose |
|---|---|
| `reference/import-case.sh SRC NAME [ROOT]` | copies a model to `corpus-fuzz/NAME` and writes `expected/` with the reference. If the reference aborts, it writes `REFERENCE-ERROR.txt` and the partial `reference-partial/{trace,tape}.jsonl` instead |
| `reference/reffail-cmp.sh SIMOXIDE DIR...` | for models whose reference run aborts, finds the first trace difference to the partial reference trace |
| `simoxide-fuzz diff a.jsonl b.jsonl` | first divergence of two traces |
| `crates/simoxide-testkit/scripts/heavy-tests.sh [--fuzz]` | all slow tests |

## Reference harnesses

Three Java programs serve as oracles: refsim (the patched SimuLizar,
[refsim](../reference-simulator/refsim.md)), the Eclipse QVTo runner
([Conformance](../qvto/conformance.md)) and the Slingshot reference runner
([Slingshot runner](../reference-simulator/slingshot.md)). What their drivers share is in one
place, `crates/simoxide-oraclekit` (Rust) and `reference/oraclekit/` (Java and shell; the crate's
`README.md` has the details):

| Piece | Rust (`simoxide-oraclekit`) | Java and shell (`reference/oraclekit/`) |
|---|---|---|
| warm-JVM batch process | `batch::run`: result lines read with a deadline each (refsim: 60 s, plus 120 s start-up), the process killed when one passes, standard error to a file or captured; `RefSim` and the QVTo `JavaRun` use it | refsim's `batch`, `qvto-run --batch` (their protocols stay their own) |
| JVM launch | `java::ensure_built` (a runner's `build.sh` on demand) | `lib.sh`: `oraclekit_exec_java`, the JVM under `nice -n $ORACLE_NICE` (default 19) for refsim, `qvto-run` and `run.sh` |
| seed and ordering agent | `java::seed_agent`, `agent_option`, `ORDERED_QVTO` | `seed-agent/` and `seed-agent.sh`: one source, built per harness with its product's ASM |
| results and expectations | `results`: `<dir>/<case>/<file>` result sets, `.gz` expected files, `differing_files`/`differences`; `scratch::Scratch` per batch | `lib.sh`: `oraclekit_first_diff` (the `--check` of `corpus-ss.sh`, `verify-det.sh`, `smoke.sh`), `oraclekit_limited` (time and file-size limits per JVM) |

Every harness has a check mode that runs the reference again and compares with what is committed,
writing nothing: `reference/regen-expected.sh --check` (the corpus' `expected/`),
`reference/slingshot/corpus-ss.sh --check` (`corpus-ss/*/expected/`) and
`cargo qvto conform record --check [--det]` (the fixture bundles; the cache keeps its tapes).

## QVTo engine and foundation crates

The [QVTo engine](../qvto/) and the [foundation crates](../development/foundation.md) have
their own test layers. They are described on [Conformance](../qvto/conformance.md) and
[Foundation crates](../development/foundation.md#verification):

| Layer | Oracle | Run |
|---|---|---|
| QVTo crates' unit and end-to-end tests | hand-written; Eclipse's logs | `cargo test-qvto` |
| parser, analyser | Eclipse QVTo 3.11's parser and compiler on 1 102 files | `cargo test-qvto`; `qvto-sema`'s `oracle_report` example |
| Java primitives | JDK 21 and classic OCL (committed samples; the differential test needs the JDK) | `cargo test-qvto` |
| conformance scoreboard | the Java reference runner on 589 runnable cases (Eclipse suite, SimuLizar and SPD corpus, SPD adjustments, fuzz regressions), in compat, fast and ordered mode | `cargo conform --suite all` |
| SPD adjustments | the Slingshot reference runner's recorded adjustments | `cargo qvto spd-replay` |
| differential and robustness fuzzing | the Java runner; no panics | `qvto fuzz` |
| EMF graph | EMF 2.41 on the suite's and the PCM side's model files, random edit scripts, the suite's comparer; the `ModelStore` contract | `cargo test-fdn` |
| QVTo on simulation models | the rules corpus byte for byte against the patched reference; the QVTo corpus on SimOxide-loaded models against the Java runner | `simoxide-reconf`'s tests (`cargo test-sim`) |

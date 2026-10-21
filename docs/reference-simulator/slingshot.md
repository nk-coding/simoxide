# Slingshot reference runner

`reference/slingshot/` runs the Slingshot nightly headless, with or without SPD (Scaling Policy
Definitions), and writes outputs that SimOxide can be compared with. It is the oracle of
SimOxide's Slingshot flavour ([Slingshot-compatible mode](../guide/slingshot.md); design:
`DESIGN-SPD.md`, phase S0). Like
[refsim](./refsim.md) it needs only `javac`, `jar`, Python 3 and a shell.

It has two modes:

| Mode | What runs | Use |
|---|---|---|
| `det` (default) | the nightly plus the patches below: seeded randomness, insertion-ordered collections, a deterministic PS job table | reproducible references; the basis of the Slingshot-exact comparison |
| `stock` (`--stock`) | the unmodified nightly; with `-seed` only the random sources are seeded | "what Slingshot does", statistics, benchmarks |

Both use the same install, the same stub bundle and the same instrumentation.

## Requirements

- Java 21.
- A headless Slingshot install (p2, about 130 MB). `env.sh` takes `SLINGSHOT_INSTALL`, else
  `reference/slingshot/install/sl-install`, else the install of 2026-09-29 in
  `old/palladio-research/work-slingshot/sl-install`. The install is only read.
- `install.lock` pins the install: SHA-256 of the 47 Slingshot, SPD, QVTo, SimuCom-framework,
  probfunction and SSJ jars (nightly `1.0.0.202609010045`, SPD `1.0.0.202609010039`, QVTo 3.11.0).
  `setup.sh` warns when the install differs. Slingshot has no releases; a fresh install from the
  update sites gets the current nightly.
- `install.sh` creates a fresh install (network; the p2 director of an Eclipse install such as the
  Palladio 5.2.2 product). It includes the two workarounds for the nightly's packaging bugs:
  `slingshot.common.utils` and `spd.semantic.transformations` are published nowhere and are built
  from source into a local p2 repository.

## Commands

```sh
reference/slingshot/setup.sh            # build/: bundles, seed agent, patched jars, Equinox configurations
reference/slingshot/setup.sh --lock     # rewrite install.lock after a deliberate install update

reference/slingshot/run.sh [--stock] [--slot N] -model DIR -out DIR -simTime T [-seed N] [options]
reference/slingshot/smoke.sh [OUT]      # smoke corpus: determinism and times -> results/smoke.csv
```

`run.sh` starts one JVM (`nice -n 19`, `-Xmx2g`; `NICE` or `ORACLE_NICE`, `JVM_OPTS`, `JAVA` override) with the
Equinox launcher of the install and a private configuration area `build/osgi/<mode>-<slot>`.
Slots are locked with `flock`, so concurrent runs never share an area.

| Runner option | Default | Meaning |
|---|---|---|
| `-model DIR` | | all `*.repository, system, resourceenvironment, allocation, usagemodel, monitorrepository, spd, semanticspd` files, in name order. `.slo` is skipped (the SLO metamodel is not installed); measuring points load through the monitor repository |
| `-out DIR` | | output directory; `run-N/` per run when `-runs > 1` |
| `-simTime T` | 1000 | simulated seconds. Slingshot's only stop condition; it ignores the maximum measurement count |
| `-seed N` | unseeded | seed all random sources (see below). Needs the seed agent, which `run.sh` adds |
| `-stoex mt\|jur` | `mt` | StoEx generator when seeded |
| `-runs N` | 1 | runs in one JVM (warm repetitions with the same seed) |
| `-instrument B` | true | `false`: the instrumentation extension is not registered at all |
| `-trace B`, `-tape B`, `-spd B` | true | individual outputs |
| `-spdLevel decisions\|all` | decisions | `all` also logs the evaluations that the target-group check rejects (every measurement × every policy) |
| `-measurements B` | true | `measurements.csv` from the EDP2 repository |
| `-saveModels B` | false | write the models after the run (with all SPD adjustments) to `models/` |
| `-linkingResources B` | `run.json`, else false | SimuCom's "simulate linking resources" |
| `-log LEVEL` | WARN | log4j root level |

## Seeding

Unpatched, Slingshot cannot be seeded: `SimuComConfig`'s seed is ignored, and there are three
JVM-global random sources. With `-seed N`:

| Source | Used for | Unpatched nightly | Seeded |
|---|---|---|---|
| StoEx (`ProbabilityFunctionFactoryImpl` generator) | demands, think and inter-arrival times, loop counts, latencies | `DefaultRandomGenerator`: `new java.util.Random()` | `mt`: SimuCom's MT19937 seeded `{N, …, N+5}`, the stream SimuLizar, refsim and SimOxide use for seed N. `jur`: `java.util.Random(N)` |
| `Math.random()` | probabilistic branches in SEFFs and usage models (`TransitionDeterminer`), link failures (`SimulatedLinkingResource`) | the JVM's unseeded `Math.random()` | `java.util.Random(N + 1000003)`: the agent replaces the `Random` behind `Math.random()` by a subclass with the same algorithm that can be reseeded and taped |
| `UUID.randomUUID()` | ids of every event, user, job, request; `Job.hashCode`, `User.hashCode` | `SecureRandom` | SplitMix64 seeded with N: the agent installs a seedable `SecureRandom` provider at position 1 |

All three are reseeded at the start of every run, so warm runs equal cold runs.

With `-stoex mt` the StoEx draws of Slingshot and SimuLizar coincide where they draw in the same
order: on `x_ss_minimal` Slingshot's usage-scenario response times equal refsim's up to DESMO-J's
nanosecond truncation (2.7362678682860686 vs 2.736267868).

## What `det` changes

`det` makes the same-time order of events deterministic. Unpatched, Slingshot schedules the result
events of a handler from a `HashSet` (`Result`), and many handlers build their results in
`HashSet`s, `Collectors.toSet()` or `Set.of`. The order follows identity hash codes or the per-JVM
`Set.of` salt, so it differs from run to run. Two changes fix it.

**1. Ordering rule (seed agent, `-javaagent:seed-agent.jar=ordered,qvto`).** At class-load time,
an ASM rewrite of every class under `org.palladiosimulator.analyzer.slingshot.*` and, with `qvto`,
`org.eclipse.ocl.*` and `org.eclipse.m2m.(internal.)qvt.oml.*` makes these changes:

- `new HashSet` / `HashSet::new` → `LinkedHashSet`;
- `new HashMap` / `HashMap::new` → `LinkedHashMap`;
- `Collectors.toSet()`, `toUnmodifiableSet()`, `Set.of(…)`, `Set.copyOf(…)` → insertion-ordered
  equivalents (`Ordered`). These keep the null and duplicate checks and the immutability.

Contents and lookups are unchanged, and in Slingshot's own code only the iteration order becomes
insertion order. In OCL the class change is visible too: classic OCL tells an OrderedSet by
`instanceof LinkedHashSet`, so under the rule every OCL `Set` behaves as an OrderedSet (ordered
`=`, `asSet` copies, printing). The SPD transformations do not depend on it; six cases of the
Eclipse QVTo suite do (their own assertions fail under the rule). The nightly loads 91 rewritten
classes with 227 sites. Without `qvto`, QVTo's `Set(ResourceContainer)` in
`ResourceEnvironmentTransformation.qvto:13-18` orders the resource environment's container list
and each link's connected containers by identity hash. `ORDERING=ordered ./run.sh` keeps QVTo
unrewritten.

qvto-rs reproduces the rule as its *ordered mode* (EPL, every build): `../oraclekit/seed-agent.sh` builds
the agent alone, and the QVTo Java runner with it is ordered mode's oracle on every QVTo test suite
(`crates/qvto/qvto-testkit/README.md`, "The `det` Java runner"); `qvto spd-replay` replays the
adjustments of this runner's `det` runs (`spd.jsonl`) in ordered mode and compares the model
summaries: 13 of 13 equal.

**2. Source patches (`patches/`).** `orig/` is the upstream source (Slingshot commit `8bf9adc`, the
commit the nightly was built from) and `src/` the patched copy. `setup.sh` compiles both, checks that
`orig/` has the member signatures of the installed class, and writes a copy of the installed jar
with the patched classes into `build/patched/`. The `det` configuration uses that copy.

| Patch | Class | Change | Effect |
|---|---|---|---|
| S0-P2 | `ProcessorSharingResource` (resourcesimulation) | `Hashtable<Job, Double>` → `LinkedHashMap` | ties of equal remaining demand go to the job that arrived first, instead of following `Job.hashCode` = hash of a random UUID |

The `Result` class needs no source patch: the ordering rule covers it.

Nothing else changes. Event times, handlers, the SSJ event list (FIFO for equal times) and all
Slingshot quirks stay as in the nightly. Semantically the `det` order is one of the orders the
nightly can produce: each `HashSet` order is some permutation, and `det` picks insertion order. The
`stock` mode with `-seed` seeds the random sources only. Its same-time order still varies, and
with it a measurement can occasionally differ.

## Outputs

Per run, in the output directory:

| File | Content |
|---|---|
| `measurements.csv` | EDP2's raw measurements in SimOxide's format ([formats](../guide/formats.md) §4): `measuring_point,metric,time,value`, series sorted by key, rows in EDP2 order |
| `trace.jsonl` | every dispatched DES event, in dispatch order (`slingshot-trace/1`, below) |
| `tape.jsonl` | every uniform draw: `{"k":"u","s":"stoex"\|"math","i":n,"u":x,"e":event,"o":site}`; `e` is the trace index of the event being dispatched; `o` the draw site (below). Only with `-seed`: the draws are taped by the seeded generators |
| `bus.jsonl` | for the first event of each type, its subscribers in call order (`Class.method<reified>`), from the bus's per-subscriber pre-interception |
| `spd.jsonl` | SPD decision log (below) |
| `summary.json` | event count, rows, draws, SPD counts, errors |
| `timings.csv` | per run: `load_ms` (blackboard, EDP2, seeding), `init_ms` (driver and Guice), `sim_ms`, `collect_ms`, `wall_ms`, events, rows; `jvm` = cold for the first run of the JVM |

**Trace.** One JSON line per event: `ev` (Slingshot's event name), `t` (Slingshot's double time),
then entity keys:

| Keys | Events |
|---|---|
| `u` | user number (first appearance in the run) |
| `type`, `el` | EClass and id of a model element (`UsageModelPassedElement`, `SEFFModelPassedElement`, `*Visited`, user events: the current action) |
| `ac` | assembly context of a SEFF interpretation or resource request |
| `job`, `d` | job number (first appearance), current demand; `rc`, `res` for active jobs; `link` for linking jobs |
| `kind`, `res`, `action`, `passive` | resource demand requests |
| `role`, `sig` | user requests and external calls |
| `mp`, `metric`, `v` | `MeasurementMade`: measuring point key, metric, `[time, value]` |
| `tg`, `policy`, `ok`, `new`, `deleted` | `SimulationTimeReached`, `ModelAdjustmentRequested`, `ModelAdjusted` |

There are no UUIDs and no identity hashes. Ids of elements that an SPD adjustment creates
(`EcoreUtil.generateUUID()`, time based) are replaced by aliases: `~EClass:entityName`
(`~ResourceContainer:AppServer_1`), or the parent's alias plus `/feature.index` for unnamed
elements. The same aliases are used in `measurements.csv` and `spd.jsonl`.

**Draw sites.** For `math` draws the seed agent hooks the two methods that call `Math.random()`
(`SiteTransformer`, ASM at class-load time, installed whenever the agent is loaded):
`TransitionDeterminer.determineBranchTransition` / `determineProbabilisticBranchTransition` announce
their transition list and `SimulatedLinkingResource.onJobProgressed` its link (`Sites.set`, a static
reference, nothing else); the tape writes `branch:<id of the Branch or BranchAction>` or
`link:<linking resource id>`. For `stoex` draws `o` is the innermost Slingshot frame on the stack
(`Class.method`, skipping `AbstractStoExProxy` and `SimulatedStackHelper`), e.g.
`ResourceSimulation.initiateActiveResource`. `tools/project.py` maps them to SimOxide's origin
purposes ([Slingshot semantics: exact comparison](../spec/slingshot/exact.md)). Trace, measurements
and draws are unchanged by the hooks.

**SPD log.** It is built without changing Slingshot's code. Each policy's subscriber is found
through the bus's interceptors, and the entries of its `FilterChain` are replaced in place by
recording proxies.

| `k` | Content |
|---|---|
| `policy` | id, name, target group, adjustment and trigger type, filter chain (`TargetGroupChecker`, trigger checker, thrashing and policy constraints, `Adjustor`) |
| `eval` | one handler invocation: time, event, policy, each filter run with `ok` / `disregard` + reason / `exception`, and whether a `ModelAdjustmentRequested` resulted |
| `adjust` | around `SpdAdjustmentBehavior`: model summary before and after the QVTo run (containers, links, allocation, semantic target-group elements), success, number of changes |
| `log` | log4j messages of the SPD adjustment (QVTo `log`, "RECONFIGURATION WAS …") |

The instrumentation is passive. It schedules nothing, never vetoes an interception and catches its
own errors. In `det` mode, runs with and without it give identical measurements (smoke column
`instrumentation_neutral`).

## Projection, probes and det verification

| Script | Purpose |
|---|---|
| `tools/project.py RUN_DIR` | event projection for the Slingshot-exact comparison: `projection.jsonl` (level E), `projection-m.jsonl` (level M), `tape-projection.jsonl` ([SS-X-2](../spec/slingshot/exact.md#ss-x-2-the-projection)) |
| `tools/probegen.py` | generates the probe models of the [Slingshot semantics](../spec/slingshot/index.md) into `models/probes/` |
| `probes.sh [--check]` | runs every probe in `det` mode and writes (or compares) `models/probes/<name>/expected/` |
| `verify-det.sh -o OUT DIR...` | `det` twice in fresh JVMs, once warm, once `stock`, per model; time and file-size limits per JVM; summary `OUT/verify.csv` (`results/det-verify.csv` for probes, SPD examples and corpus) |
| `corpus-ss.sh [--check] [NAME...]` | the Slingshot-exact corpus `corpus-ss/` (X1): runs each case (`case.json`: model, seed, simulated time) in `det` and writes (or compares) `corpus-ss/<name>/expected/` ([SS-X-8](../spec/slingshot/exact.md#ss-x-8-the-slingshot-exact-corpus)) |
| `tools/errors.py RUN_LOG` | the handler exceptions of a run (`errors.txt`; identity hashes in messages replaced by `@*`) |
| `tools/audit_subscribers.py` | static audit of the subscriber order: no Slingshot class subscribes twice to one event type without disjoint reified types ([SS-BUS-2](../spec/slingshot/core.md#ss-bus-the-event-bus)) |

## Smoke corpus

`smoke.sh` runs every Slingshot model of the corpus and the SPD examples through the runner.

| Model | Sim. time | Events | Rows | StoEx / `Math.random` draws | SPD requests | det: 2 fresh JVMs | det: warm = cold | stock: 2 runs | stock vs det measurements | cold run / JVM (ms) | warm run (ms) |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `spd_minimal` | 100 s | 2468 | 200 | 0 / 101 | 1 | yes | yes | differs (trace.jsonl) | same | 2648 / 3844 | 289 |
| `spd_scale_in` | 150 s | 2570 | 176 | 0 / 119 | 2 | yes | yes | differs (trace.jsonl) | same | 2415 / 3623 | 242 |
| `x_ss_minimal` | 1000 s | 17687 | 1812 | 453 / 0 | 0 | yes | yes | differs (trace.jsonl) | same | 2506 / 3569 | 272 |
| `x_ss_minimalexample` | 1000 s | 20538 | 1248 | 312 / 638 | 0 | yes | yes | yes | same | 2506 / 3630 | 380 |
| `x_ss_minimalexample_open_ps` | 1000 s | 45995 | 3495 | 0 / 1972 | 0 | yes | yes | differs (trace.jsonl) | same | 2874 / 3972 | 452 |
| `x_ss_delay` | 1000 s | 6348 | 453 | 453 / 0 | 0 | yes | yes | yes | same | 2316 / 3468 | 168 |
| `x_espresso` | 100 s | 45267 | 5026 | 0 / 0 | 0 | yes | yes | yes | differ | 2271 / 3298 | 271 |
| `x_ss_mediastore` | 200000 s | 23733 | 3902 | 468 / 0 | 0 | yes | yes | differs (trace.jsonl) | same | 3449 / 4492 | 418 |
| `x_sl_mediastore` | 100000 s | 2678 | 357 | 51 / 11 | 0 | yes | yes | differs (trace.jsonl) | same | 2715 / 3727 | 281 |

- **Determinism.** `det` gives byte-identical trace, tape, SPD log and measurements on all models,
  both in fresh JVMs and in warm runs.
- **`stock` with `-seed`.** The trace usually differs between two runs (same-time order). The
  measurements were the same in both runs.
- **`x_espresso` (stock vs det).** This model has 30 closed users without think time and constant
  demands, so it is full of ties. Its state samples differ between `stock` and `det`: 3018 vs 3016
  rows, mean 22.7 vs 21.1. The response times are equal. On such models, the order chosen for
  same-time events changes the measurements, so an exact comparison needs a fixed order.
- **Times** are on the shared development VM (other agents running, `nice -n 19`), without
  instrumentation and measurement output, with `-seed 1`:
  - "cold run" is the first run in a JVM (class loading, Guice, QVTo compilation);
  - "JVM" is the whole process including Equinox start-up;
  - "warm run" is the median wall time of runs 2 to 6 in one JVM.
  - They are indicative only. Benchmark B1 repeated them on a quiet 25-core VM (2026-10-21,
    stock mode, [details](../performance/comparison.md#slingshot-flavour-and-spd-against-slingshot)):
    first run in a JVM 1.9 s (`x_ss_minimal`), 2.1 s (`x_espresso`), 2.4 s (`spd_minimal`)
    and 3.1 s (`x_ss_mediastore`), 2.8 to 4.3 s for the JVM; warm runs 130 ms (`spd_minimal`), 123 ms
    (`x_ss_minimal`), 179 ms (`x_espresso`) and 249 ms (`x_ss_mediastore`), after 10 warm-up runs.
- `x_sl_mediastore` uses `bench/compare/models/x_sl_mediastore_ss` (explicit acquire demands).
  `x_pem_fork` and `h11_fork_sync` are not run: Slingshot cannot run synchronous forks.

## Findings

Findings from the reference runs, for the Slingshot semantics (S1) and the SPD design:

- **The shipped nightly cannot run any SPD adjustment.** `MainTransformation` imports the
  `ProfilesLibrary` blackbox of `org.palladiosimulator.architecturaltemplates.catalog`, which is
  not installed. The stub bundle (`src/stub`) registers the blackbox; it is never called.
- **Q5 confirmed (scale-in loses jobs).** `models/spd_scale_in` removes `AppServer_1` at t = 80
  while four jobs run on it. The job with the pending `JobProgressed` (the shortest) is aborted at
  its next event (t = 87): `JobAborted`, then `ResourceDemandRequestAborted`, then `UserAborted`,
  and the open-workload user is dropped. The other three jobs are lost without any event, and
  their users never finish.
- **Q15 confirmed.** Policies of one target group evaluate each other's `SimulationTimeReached`:
  the t = 80 policy evaluates the t = 50 event and is rejected only by `SimulationTimeChecker`.
- **The timing of §1.4 of the design is confirmed.** At t = 50 the order is:
  1. `SimulationTimeReached`;
  2. `JobProgressed`;
  3. `ModelAdjustmentRequested` (the QVTo run);
  4. `JobFinished`, `ActiveResourceStateUpdated`;
  5. `ModelAdjusted`.
- **QVTo `Set` order (new quirk).** After an adjustment, the order of the resource environment's
  containers and of each link's connected containers follows Java identity hashes
  (`ResourceEnvironmentTransformation.qvto:13-18`).
- **Measurements Slingshot does not produce.** Response times exist only for usage scenarios and
  `AssemblyOperationMeasuringPoint`s
  (`OperationCallActionResponseTimeMonitoringBehavior.java:85`). Monitors on
  `EntryLevelSystemCallMeasuringPoint` and `SystemOperationMeasuringPoint` are silently ignored:
  `x_ss_minimal` has no rows for them.
- **Created ids are not reproducible.** Elements that QVTo creates get time-based
  `EcoreUtil.generateUUID()` ids. The aliases above make outputs comparable.
- **EDP2 and `MeasurementMade` agree.** On the smoke models, every EDP2 row has a `MeasurementMade`
  event; `rows` equals `measurements_made`.

## Layout

| Path | Contents |
|---|---|
| `env.sh`, `setup.sh`, `run.sh`, `smoke.sh`, `install.sh` | locate, build, run, smoke corpus, fresh install |
| `install.lock` | SHA-256 pin of the install |
| `src/stub` | `ProfilesLibrary` stub bundle |
| `src/runner` | headless application, EDP2 → CSV, seeding |
| `src/instrument` | behaviour extension: trace, tape, SPD log, keys and aliases |
| `../oraclekit/seed-agent`, `seed-agent.sh` | Java agent (shared with refsim and the QVTo runner): seedable `Math.random()` and UUIDs, ordering rule; `seed-agent.sh` builds it into `build/` with the install's ASM 9 |
| `../oraclekit/lib.sh` | shared shell helpers: the niced JVM launch, time and file-size limits, the comparison of `corpus-ss.sh --check`, `verify-det.sh`, `smoke.sh` |
| `patches/<bundle>/{orig,src}` | upstream and patched sources of replaced classes |
| `tools/mkconfig.py` | private Equinox configuration (bundles.info from the install, minus the old bench bundle, plus ours, patched jars) |
| `models/` | SPD examples (provenance in `models/README.md`) |
| `corpus-ss.sh`, `../../corpus-ss/` | the Slingshot-exact corpus and its `det` reference outputs |
| `results/smoke.csv` | the last smoke run |
| `build/` | generated (gitignored) |

## Open issues

- Resolved in S1: the trace projection (`tools/project.py`), draw sites on the tape (seed-agent hooks),
  `det` verification beyond the smoke corpus (`verify-det.sh`, `results/det-verify.csv`).
- `getDeclaredMethods()` order varies between JVMs (observed in `bus.jsonl`); it only reorders
  subscribers of disjoint reified types, so no run changes, but `bus.jsonl` is not byte-stable.
  X1 checks this statically for every class (`tools/audit_subscribers.py`).
- Usage evolution (`.usageevolution`, DLIM) is not loaded (design decision U9).

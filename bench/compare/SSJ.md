# Palladio on the SSJ engine

SimOxide reproduces SimuLizar 5.2.2 on its default simulation engine, DESMO-J, and the comparison in
`docs/performance/comparison.md` measures SimuLizar and SimuCom on DESMO-J. Palladio has a second engine,
SSJ (Stochastic Simulation in Java), which is expected to be faster. This directory adds SimuLizar and
SimuCom on SSJ to the same benchmark, so that the speed-ups on the website are also stated against
Palladio's faster engine. The full run was made on 2026-10-21 on a 25-core, 34 GB VM (AMD Ryzen 9 9950X3D, the CPU of the
website's 22-core VM; all-core phases with `PAR_WORKERS=22`), 16:56–20:38: results in `ssj/results/`
(summaries and tables; raw logs git-ignored), on the website in `docs/performance/` ("Palladio on the
SSJ engine"). Outcome: SSJ is not faster than DESMO-J (warm SimuLizar runs 78 to 126 % of the
DESMO-J time, median 97 %), and both engines gave identical results per seed on every benchmark model.

## How Palladio selects the engine

Both simulators run on the abstract engine `de.uka.ipd.sdq.simulation.abstractsimengine`. Engines register
at the extension point `de.uka.ipd.sdq.simulation.abstractsimengine.engine`:

| Engine | Bundle (5.2.2) | Extension id | Factory |
|---|---|---|---|
| DESMO-J | `de.uka.ipd.sdq.simulation.abstractsimengine.desmoj` (+ `de.desmoj` 2.3.3), in the product | `de.uka.ipd.sdq.simucomframework.desmoj.engine1` | `DesmoJSimEngineFactory` |
| SSJ | `org.palladiosimulator.simulation.abstractsimengine.ssj` (+ `ca.umontreal.iro.simul.ssj` 3.3.1), **not** in the product | `org.palladiosimulator.simulation.abstractsimengine.ssj` | `SSJSimEngineFactory` |

The engine is chosen by the Eclipse preference **`de.uka.ipd.sdq.simulation/simulationEngineField`**
(preference page "Palladio Simulation"), not by a launch-configuration attribute. If it is unset, the
default is the first registered engine (`SimulationPreferenceInitialiser`), so with both bundles installed
the choice must be explicit. Sources (Palladio 5.2.2, `old/palladio-research/src-5.2.2`):

- `de.uka.ipd.sdq.simulation/.../preferences/SimulationPreferencesHelper.getPreferredSimulationEngine()`
  reads the preference and creates the factory through `AbstractSimEngineExtensionHelper.getEngineFactory(id)`
  (an unknown id gives `null`; there is no silent fallback).
- **SimuCom**: the generated `SimuComControl extends AbstractMain`; `AbstractMain.getSimulationEngine()` is
  abstract in 5.2.2 and implemented by `AbstractMainUi` (IDE) and by our headless patch
  (`simucom/bundle/.../SourcePatches.java`), both with `getPreferredSimulationEngine()`.
  `AbstractSimulationConfig.getEngine()` returns a hard-coded SSJ class name, but its only caller,
  `SimuComFactory.getSimuComModel`, is not on SimuCom's path (and compares it with extension ids, which
  never match).
- **SimuLizar 5.2.2 supports SSJ**: its Dagger component `SimEngineComponent` uses `EclipseSimEngineModule`
  → `SimulationPreferencesSimEngineFactoryProvider` → `getPreferredSimulationEngine()`. Nothing in
  SimuLizar's main bundles depends on DESMO-J (only the test commons hard-wire `DesmoJSimEngineFactory`).
  The only SSJ gaps (`SSJSimProcess.isScheduled/reschedule` throw `UnsupportedOperationException`) are not
  reached: SimuLizar's reconfiguration calls `isScheduled()` through the entity delegate, which SSJ supports.

How the benchmark sets it:

- OSGi (SimuCom, SimuLizar's `headless.bench` application): `eclipse.pluginCustomization=work/engine-<e>.ini`
  in the configuration's `config.ini`, with `de.uka.ipd.sdq.simulation/simulationEngineField=<id>`. Both
  engines are set explicitly, also DESMO-J.
- Flat classpath (the SimuLizar driver `java/simulizar/cmpsl/CompareSimuLizar`): no preferences without
  OSGi, so the driver injects the factory, as it always did for DESMO-J: new argument `engine=desmoj|ssj`.

Evidence of the engine in every log: the SimuLizar driver prints `# ENGINE <factory class> from <jar>`;
SimuCom's `PHASES` lines end with `engine=<factory class>`; `ssj/smoke.sh` also records the JVM's
class-loading log (`SSJExperiment` / `DesmoJExperiment` and the jar they come from).

## What was fetched

From the p2 repositories of the Palladio 5.2.2 release (the SHA-256 values match their `artifacts.xml`):

| File | URL | SHA-256 |
|---|---|---|
| `org.palladiosimulator.simulation.abstractsimengine.ssj_5.2.2.jar` (13 464 B) | `https://updatesite.palladio-simulator.com/palladio-simulation-abstractsimengine/releases/5.2.2/plugins/` | `7385ee9fd789885e10475b185987403157a9610c09a715ee1c8cb2ac4e664831` |
| `ca.umontreal.iro.simul.ssj_3.3.1.jar` (1 300 298 B) | `https://updatesite.palladio-simulator.com/palladio-thirdparty-library/releases/5.2.2/plugins/` | `f6d882c55101f6a18846451da4dca6f6c085dbd50a25327a4c264c93cd8fa5f3` |

Provenance check: the same repository's `de.uka.ipd.sdq.simulation.abstractsimengine_5.2.2.jar` and
`...desmoj_5.2.2.jar` are byte-identical to the jars of the installed product
(`work-simucom/palladio-5.2.2`), so the SSJ bundle is from the same build. The Slingshot install's
`ca.umontreal.iro.simul.ssj_3.3.1.jar` is a different (6.0) rebuild and is not used. The SSJ engine bundle
requires only `org.apache.log4j`, `de.uka.ipd.sdq.simulation.abstractsimengine` and the SSJ library, whose
Colt/JFreeChart imports are optional and unused by the event engine.

## What is set up

`ssj/setup.sh` (idempotent, about 10 s, writes only `ssj/work/`, git-ignored; never touches the Palladio
install): downloads and verifies the two jars; builds an **overlay install** `work/palladio-5.2.2-ssj`
(symlinks to all product plugins + the two SSJ bundles; the product's `configuration/` plus two
`bundles.info` lines); writes the engine preference files; builds SimuCom's OSGi configurations
`work/simucom-{desmoj,ssj}` (with `simucom/build.sh`), SimuLizar's OSGi configurations
`work/simulizar-osgi-{desmoj,ssj}` (as `work-simucom/make-headless-config.sh`, with the prebuilt
`headless.bench` jar) and the SimuLizar driver `work/build/simulizar`; and the SimuCom copy of the MiSArch
model. The existing configurations (`simucom/work/config`, `work-simucom/headless-config`) still point to
`/home/devbox/workspace/palladio-research`, which has moved to `old/`; the SSJ scripts find the install
themselves (`ssj/common.sh`, `PALLADIO_RESEARCH` overrides) and leave those files alone.

Simulators of the SSJ session (`ssj/ext.sh`, hooked into `lib.sh` by `CMP_EXT`):

| sim | what | how |
|---|---|---|
| `simoxide` | as in the main comparison | |
| `simulizar` / `simulizar-ssj` | stock SimuLizar, flat classpath | the same driver, `engine=desmoj` / `engine=ssj` (+ the two SSJ jars on the classpath) |
| `simulizar-osgi` / `simulizar-ssj-osgi` | stock SimuLizar in Equinox, cold runs only | `ssj/run-simulizar-osgi.sh`, engine by preference |
| `simucom` / `simucom-ssj` | SimuCom 5.2.2 (repaired) | `simucom/run-simucom.sh` on the overlay, engine by preference |

DESMO-J and SSJ run from the same overlay install in the same session, so the only difference between
each pair is the engine. The matrix is the website's (`docs/performance/comparison.md`), run by
`run-all.sh`'s own phases with unchanged parameters: cold one-shot (3 reps; OSGi variants for three
models, short), warm (same warm-up/measured run counts; SSJ SimuCom gets SimuCom's 5 + 10 for short
runs), parallel `x_espresso` short, sustained one core (`x_ss_mediastore` 2·10^6 s for 300 s, `x_espresso`
100 s for 120 s; stock SimuLizar on either engine without the RNG dispose), sustained all cores
(SimOxide threads/batch, SimuLizar threads/processes on both engines; SimuCom not, as on the website), the
very long run, plus the extra model `x_misarch` (cold and warm, 36 s and 361 s simulated).

The generated tables use the same `summarize.py` / `sustained.py` (the SSJ rows and columns appear only
when there is SSJ data; `CMP_COLUMNS=present` hides simulators not run in the session). Regenerating the
main tables from the existing logs gives byte-identical files.

**MiSArch model** (`ssj/models/x_misarch`, provenance in `SOURCE.txt`): written by
`msa-sim pcm misarch.model.json OUT_DIR`; open workloads, asynchronous forks, branches, PS CPUs and delay
resources. SimuLizar runs it unchanged on both engines. SimuCom cannot: its generated Java does not compile
because SimuCom uses entity names as Java identifiers (`JavaNamesExt.javaName()` only replaces
`" <>:.,+-()` and spaces, and signature names are used verbatim): `createUser@user`, `broker→address`,
`sidecar media`, the component `return`. `ssj/sanitize-names.py` makes a Java-safe copy (SimOxide gives the
same result on both copies), but SimuCom then fails with "product.impl cannot be resolved to a type" (a
generated package clash, not investigated further). `ssj/ext.sh` therefore leaves SimuCom out on
`x_misarch` (`supported`). Note: for models with several usage scenarios the drivers' `requests` and
`mean_rt` describe one scenario only (the last one they find), so the sanity table is not meaningful for
`x_misarch`; timing is unaffected.

## Running the full comparison (on a quiet machine)

```sh
bench/compare/ssj/run.sh            # build cold warm sus1 susN longrun par extra summary
```

- Output: `bench/compare/ssj/results/` (`summary.csv`, `tables.md`, `tables_sustained.md`, ..., raw logs in
  `raw/`, git-ignored), then `docs-numbers.md` and `docs-ssj.patch` (below). Every phase resumes like
  `run-all.sh`; single phases: `bench/compare/ssj/run.sh cold warm summary`.
- The run waits for `SSJ_LOCK` (default: the MiSArch `MEASURING.lock`) to disappear before each phase,
  and `run-all.sh` waits for a quiet machine (≤ 2 runnable tasks) before each benchmark process.
- **Duration**: about 3 hours (estimated from the DESMO-J logs, SSJ assumed similar): cold ≈ 30 min,
  warm ≈ 40 min, sustained one core ≈ 35 min, all cores ≈ 25 min, long run ≈ 15 min, parallel ≈ 15 min,
  MiSArch ≈ 20–40 min (unknown JVM speed on this model).
- **Resources**: single JVM runs `-Xmx4g`, up to 2–3 GB RSS; the all-core phases need what the website
  shows (22 SimuLizar processes reached 25 GB and were killed by the 8 GB watchdog floor; one JVM with 22
  threads 13 GB). The website's numbers come from a 22-core, 34 GB VM; **this VM has 8 cores and 15 GB**.
  Here, run the all-core phases with `PAR_WORKERS=8` (default: number of cores) and a lower
  `MEM_FLOOR_KB` (e.g. 2000000), and label them as 8-core results, or run the session on the 22-core
  machine. The single-core phases are comparable across the two VMs (same CPU model), but the SSJ
  session re-measures SimOxide and DESMO-J anyway, so ratios never mix sessions.
- Other simulators can join the session (`SIMS="... refsim slingshot ..."`) if their setups are fixed
  for the moved install (`env.sh`, `PALLADIO_RESEARCH`).

## Updating the website afterwards

1. `bench/compare/ssj/run.sh summary` (part of the full run) writes `ssj/results/tables*.md` and runs
   `ssj/docs-numbers.py`, which fills the placeholders of `ssj/docs-draft.patch` into
   `ssj/results/docs-ssj.patch` and lists every number with its source in `ssj/results/docs-numbers.md`.
2. The draft (not applied) adds: SSJ rows to "At a glance", an "On SSJ" note, a stability row and an
   exactness note in `docs/performance/index.md`; SSJ simulator rows, an "SSJ engine" paragraph, a section
   "Palladio on the SSJ engine" that includes `ssj/results/tables.md` and `tables_sustained.md`, an SSJ
   fairness caveat and the reproduce command in `comparison.md`; the SSJ range in the README and the home
   page; a tool row in `docs/development/benchmarking.md`.
3. Review `docs-ssj.patch` (wording depends on the outcome, e.g. if SSJ turns out slower; the generated
   `SSJ_STABILITY` / `SSJ_STOP_EFFECT` sentences are raw facts to be condensed), then `git apply`.

## Caveats

- **Not the same sample path.** The random generator belongs to `SimuComConfig`, so both engines draw the
  same streams for a seed. But SSJ keeps simulated time as a `double` in seconds (`Simulator.time()`),
  DESMO-J as configured by Palladio counts `long` nanoseconds (`TimeUnit.NANOSECONDS`), and the engines
  order simultaneous events by their own event lists (SSJ: `SplayTree`, then insertion order). Once one
  event differs, the draws are consumed in another order. Results on SSJ are therefore only guaranteed
  to be statistically equivalent to DESMO-J (and SimOxide); on the smoke-test models they were identical
  per seed. The sanity table of the session shows it for every model.
- **Stop semantics.** DESMO-J checks the stop conditions as a `ModelCondition` and afterwards always calls
  `AbstractExperiment.stop()`, which finalises the model (stop listeners, resource deactivation). SSJ checks
  the stop conditions at the start of each Palladio event (`SSJSimEvent.actions`) and implements the
  maximum simulated time as a separate event that calls only `Simulator.stop()`: a run ended by its
  simulated time is **not finalised** (`SimuComModel.finalise()`: stop listeners, deactivation of all
  resources), and `isRunning()` stays true. The process classes of both engines release a parked process
  only when it tries to wait again after the stop, so on SSJ processes parked at the end may stay
  parked. **Confirmed by the smoke test:** in one JVM, SimuLizar on SSJ grows by about one thread per
  simulated user per run (`x_espresso`, 30 users: 46 → 106 → 166 → 256 threads over consecutive runs),
  on DESMO-J it stays at 46; SimuCom on SSJ showed 51 to 124 threads against 41 to 64 on DESMO-J. The
  sustained runs (300 s of back-to-back runs) will show whether this costs memory or throughput, or hits
  a thread limit (`ulimit -u` here: 62 128); failed runs there are a result, not a setup error.
- **Threads.** On both engines, simulated processes (users, forked behaviours) are OS threads with
  semaphore hand-offs, which dominate SimuLizar's run time; SSJ does not change that. The 5.2.2 bundles
  differ in one detail: DESMO-J's `DesmoJSimProcess` takes threads from a cached pool
  (`SimProcessCachedThreadPoolStrategy`), SSJ's `SSJSimProcess` starts a new thread per process
  (`SimProcessThreadingStrategy`). Models with many short-lived processes (open workloads, forks) may
  therefore gain less from SSJ, or lose.
- **JIT warm-up** is handled exactly as in the main comparison: warm-up runs before the measured warm
  runs; cold runs include it; the sustained runs show the warm-up curve. Both engines get the same JVM,
  heap and GC settings.
- **Engine installation.** SSJ is an optional install; the product ships only DESMO-J. The overlay adds
  two bundles and nothing else. The OSGi SimuLizar runs draw a random seed per run (`headless.bench`),
  like `simulizar-osgi` on the website; they serve for cold start times only.
- **SimuCom and MiSArch**: SimuCom is left out on `x_misarch` (above).

## Smoke test

`ssj/smoke.sh` (refuses to run while the MiSArch lock exists or the 1-min load is ≥ 3; one core via
`taskset`, `-Xmx1g`): `x_espresso` short, seeds 1..10 for SimOxide and both SimuLizar engines, seeds 1..3
for both SimuCom engines, one cold OSGi run per engine; prints the engine evidence, thread/RSS maxima, and
mean response times with Welch's t. `ssj/smoke.sh misarch`: one short MiSArch run per simulator.

Results (2026-10-13, between and after the MiSArch measurement windows; the second and third pass at a
1-min load of 1–8 from other agents, which does not matter for these checks):

| check | result |
|---|---|
| engine actually used | every path loaded the expected engine: `SSJExperiment` from `org.palladiosimulator.simulation.abstractsimengine.ssj_5.2.2.jar` and `umontreal.ssj.simevents.Simulator` from `ca.umontreal.iro.simul.ssj_3.3.1.jar` for `simulizar-ssj`, `simucom-ssj` and `simulizar-ssj-osgi`; `DesmoJExperiment` + `desmoj.core.simulator.Experiment` for the DESMO-J twins. SimuCom's `PHASES` lines: `engine=...SSJSimEngineFactory` / `...DesmoJSimEngineFactory` |
| `x_espresso` short (deterministic demands), seeds 1–10 | 990 requests, mean RT 3.0 on every engine; the model has no randomness |
| `h13_passive_contention` short, seeds 1–10 (SimuCom 1–3) | SimuLizar on SSJ = SimuLizar on DESMO-J = SimOxide for every seed (e.g. mean RT 0.320115, 0.311485, 0.305162, ...; mean over seeds 0.318801, sd 0.0111); SimuCom on SSJ = SimuCom on DESMO-J = SimOxide per seed |
| `x_misarch` 36 s, seed 1 | SimuLizar DESMO-J = SSJ (same requests and mean RT of the reported scenario); SimOxide equal on the original and the renamed copy; SimuCom fails (above) |
| threads | SSJ leaks parked user threads per run (above) |
| timing (not a benchmark: one core, other load) | SimuLizar warm-ish median per `x_espresso` run 401 ms (SSJ) vs 735 ms (DESMO-J); `h13` 350 vs 493 ms; SimuCom 2.1 vs 2.8 s (`h13`) but 3.0 vs 2.8 s (`x_espresso`); OSGi cold start the same (≈ 21 s on one core) |

So on these models SSJ and DESMO-J give identical results per seed (same random draws in the same order);
the caveat above still applies to models with simultaneous events. SSJ looks clearly faster for SimuLizar
in this rough check, which is why the comparison matters.

Raw logs: `ssj/results/smoke/<model>/` (git-ignored).

## Rough single-threaded comparison (2026-10-13, shared machine)

A quick indicative answer before the full run: SimOxide and stock SimuLizar 5.2.2 on DESMO-J and on SSJ,
warm per-run wall time on the website's six models, everything pinned to one core. Not the website
benchmark: fewer repetitions, no wait for a quiet machine, and other agents' Gradle builds and
Testcontainers tests running on the other cores. SimuCom was left out (code generation and compilation per
process make it far from free).

**Setup.** `ssj/rough.sh` (new): 3 rounds; in each round, per model, the simulators run one after another
(interleaved, never concurrently): warm short, warm long, cold short. Warm processes: SimuLizar short 20
warm-up + 10 measured runs, long 1 + 3 (SimOxide 50 + 200, 3 + 20); a table cell is the median over the 3
processes' measured runs (30 or 9 SimuLizar runs). Every process is pinned with `taskset -c 4`; JVMs get
`-Xmx4g -XX:ActiveProcessorCount=1` (so the JVM's ergonomics pick SerialGC and size the JIT for one CPU;
the website uses G1 unpinned); SimOxide runs with `threads=1`. Seed 1 everywhere. SimOxide binaries
rebuilt from the current tree first, `ssj/setup.sh` re-run (no changes). 162 benchmark processes, none
failed or killed. Session 21:47–22:32 (45 min).

**Load.** 1-min load average at the start/end of the 324 process boundaries: min 1.9, median 2.5, mean
2.9, max 5.8 (8 cores); runnable tasks at those samples mean 2.4, max 7; MemAvailable 11–14 GB. Every
process boundary is in `load.log`.

Warm per-run wall time, median (ms); speed-up = slower / faster; "per round" = DESMO-J/SSJ ratio of
the medians of each round (each round's pair ran back to back):

| model | len | SimOxide | SimuLizar DESMO-J | SimuLizar SSJ | SimOxide vs DESMO-J | SimOxide vs SSJ | SSJ vs DESMO-J | per round | results (seed 1) |
|---|---|---:|---:|---:|---:|---:|---:|---|---|
| `x_ss_minimal` | short | 0.219 | 187 | 183 | 854× | 837× | 1.02× | 0.99 / 1.01 / 1.06 | identical: 453 req, mean RT 1 |
| `x_ss_minimal` | long | 9.80 | 2 953 | 2 661 | 301× | 271× | 1.11× | 1.11 / 1.10 / 1.15 | identical: 45 413 req, mean RT 1 |
| `x_espresso` | short | 0.387 | 277 | 280 | 716× | 724× | 0.99× | 1.13 / 0.90 / 0.97 | identical: 990 req, mean RT 3 |
| `x_espresso` | long | 5.22 | 2 996 | 2 514 | 574× | 482× | 1.19× | 1.30 / 1.18 / 1.17 | identical: 19 980 req, mean RT 3 |
| `x_ss_mediastore` | short | 1.22 | 541 | 518 | 443× | 425× | 1.04× | 1.09 / 0.82 / 1.06 | identical: 18 req, mean RT 20 801.1 |
| `x_ss_mediastore` | long | 8.31 | 5 029 | 4 440 | 605× | 535× | 1.13× | 1.11 / 0.79 / 1.22 | identical: 937 req, mean RT 21 303 |
| `x_sl_mediastore` | short | 1.25 | 566 | 568 | 452× | 454× | 1.00× | 1.33 / 0.99 / 0.84 | identical: 33 req, mean RT 23 574.7 |
| `x_sl_mediastore` | long | 5.11 | 4 080 | 4 041 | 798× | 790× | 1.01× | 0.97 / 1.02 / 1.00 | identical: 833 req, mean RT 23 550.5 |
| `h13_passive_contention` | short | 0.439 | 274 | 233 | 623× | 530× | 1.18× | 1.35 / 1.23 / 0.91 | identical: 468 req, mean RT 0.320115 |
| `h13_passive_contention` | long | 14.0 | 4 395 | 3 582 | 315× | 256× | 1.23× | 1.03 / 1.26 / 1.40 | identical: 22 749 req, mean RT 0.31877 |
| `x_pem_fork` | short | 0.259 | 192 | 288 | 743× | 1 115× | 0.67× | 0.67 / 0.65 / 0.69 | identical: 87 req, mean RT 1.13678 |
| `x_pem_fork` | long | 3.36 | 2 724 | 2 190 | 812× | 653× | 1.24× | 0.90 / 1.29 / 0.93 | identical: 4 396 req, mean RT 1.13722 |

SSJ vs DESMO-J over the 12 configurations: geometric mean **1.05×** (0.67× to 1.24×). Cold one-shot, short
(process start to results, median of 3, one core): SimuLizar 10.3–15.0 s on DESMO-J, 10.6–13.8 s on SSJ
(SSJ vs DESMO-J 0.95×–1.08×, i.e. the same: JVM start and OSGi-free model loading dominate); SimOxide CLI
4–7 ms.

**Reading.**

- SimOxide vs SimuLizar, one core, warm: about 300× to 850× faster than DESMO-J and 250× to 1 100× faster
  than SSJ; cold about 2 000× (ms against 10–15 s).
- SSJ vs DESMO-J: on the whole only about 5 % faster, within the noise of most configurations (the
  per-round ratios of one configuration spread by ±20–30 %). Consistent in all three rounds: SSJ 10–30 %
  faster on `x_ss_minimal` long and `x_espresso` long; SSJ about 1.5× **slower** on `x_pem_fork` short
  (0.65–0.69, plausibly SSJ's new thread per process against DESMO-J's thread pool, see Caveats; the long
  `x_pem_fork` runs are inconclusive). The clear SSJ advantage of the smoke test (401 vs 735 ms on
  `x_espresso`) does not hold for warm runs: the smoke runs had no warm-up and logged class loading.
- Results: all three simulators gave identical requests and mean response time in every configuration
  (and every run, same seed), so the timings compare the same simulation.

**Caveats.** Shared machine (load above); only the benchmark core was reserved by convention, not
isolated, and memory bandwidth/L3 are shared. 3 processes per configuration; SimuLizar's warm medians
moved by up to ±40 % between rounds (e.g. DESMO-J `x_pem_fork` long 1 960 / 2 800 / 2 752 ms), so
differences below about 25 % are not significant here. One CPU changes SimuLizar's thread hand-offs
(every simulated process is an OS thread): compared with the website's unpinned numbers, some models are
faster here (`h13` long 4.4 s vs 10.1 s) and others slower (`x_ss_mediastore` short 541 vs 297 ms), so
these speed-ups are not comparable with the website's. SerialGC (one-CPU ergonomics) instead of the
website's G1. Long runs have only 1 warm-up run per process. Cold runs: short only. SimuCom and the
OSGi variants not measured.

Commands (from `bench/compare`):

```sh
ssj/setup.sh                                                   # idempotent, nothing changed
( cd simoxide-driver && CARGO_TARGET_DIR=../../../target/compare cargo build --release )
( cd ../.. && cargo build --release -p simoxide-cli )
ROUNDS=2 CORE=4 ssj/rough.sh run       # rounds 1-2 (writes ssj/results/rough-<date>/)
ROUNDS=3 CORE=4 ssj/rough.sh run       # round 3 (resumes: keeps rounds 1-2)
OUT=ssj/results/rough-2026-10-13 ssj/rough.sh report   # rough.md (this table) + summarize.py's tables.md/summary.csv
```

Output: `ssj/results/rough-2026-10-13/` (`rough.md`, `summary.csv`, `tables.md`, `processes.csv`,
`load.log`; the raw logs in `raw/{warm,cold}/` and `run.log` are git-ignored via `ssj/.gitignore`).

## Open issues

- The all-core phases (`par`, `susN`) need the 22-core, 34 GB machine of the website, or must be labelled
  as 8-core results (`PAR_WORKERS=8`, lower `MEM_FLOOR_KB`). The single-core phases fit this VM if other
  agents leave about 8 GB free (the watchdog floor).
- SSJ's thread leak may make the SSJ sustained runs fail or slow down; decide how the website reports
  that (as measured, like stock SimuLizar's RNG leak).
- SimuCom on `x_misarch` is excluded; `x_misarch`'s `requests`/`mean_rt` cover one scenario only.
- The existing DESMO-J configurations outside `ssj/` (`simucom/work/config`, `work-simucom/headless-config`,
  `reference/build/classpath.txt`) still point to the old `palladio-research` path; `run-all.sh` without
  `ssj/` needs them regenerated (or `PALLADIO_RESEARCH` plus rebuilt configs).

## Changes to existing files (all backward compatible; default behaviour unchanged)

- `lib.sh`: sources `$CMP_EXT` at the end if set.
- `run-all.sh`: SSJ sims take their DESMO-J twin's branch (`simulizar|simulizar-ssj` in cold for the OSGi
  variant, `simucom*:short` warm counts, par threads/processes, `disposeRng=false` in sus1).
- `java/simulizar/cmpsl/CompareSimuLizar.java`: optional `engine=desmoj|ssj|<class>`; without it DESMO-J as
  before (no extra output).
- `simucom/build.sh`, `simucom/run-simucom.sh`: optional `SIMUCOM_WORK` (default `simucom/work`).
- `simucom/bundle` (`SimuComApp`, `PatchedSimuComJob`): optional `sim=` label (default `simucom`) and
  `engine=<factory>` at the end of `PHASES` lines (takes effect after `simucom/build.sh`).
- `summarize.py`: SSJ sims and `ssj/ext.sh` models only when present, `CMP_COLUMNS=present`;
  `sustained.py`: "all N cores" from `PAR_WORKERS` (default 22). Output on the existing logs is
  byte-identical.

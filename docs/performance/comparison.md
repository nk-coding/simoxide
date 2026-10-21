# Comparison in detail

How SimOxide was compared with the other Palladio simulators, and every result table. The
summary is on the [overview](./index.md).

The comparison measures:

- warm per-run time and cold one-shot time;
- parallel throughput with all cores;
- sustained behaviour over minutes: JIT warm-up, degradation, leaks, garbage-collection pauses,
  memory;
- one very long run per simulator.

## Simulators

| Name | What | Version | Runtime | Driver |
|---|---|---|---|---|
| **SimOxide** | this project, exact mode | repository state of 2026-09-30, release profile | native, rustc 1.98.1 | `bench/compare/simoxide-driver` (library API). Each run loads the XMI files from disk, compiles them, simulates with every measurement stored, and computes the summaries. Cold runs use the `simoxide run … --measurements out.csv` CLI |
| **refsim** | the patched, deterministic SimuLizar 5.2.2 of this project ([refsim](../reference-simulator/refsim.md)), trace and tape off | 5.2.2 product jars plus [patches](../reference-simulator/patches.md) | OpenJDK 21.0.12, G1, `-Xmx4g` | `java/refsim/CompareRefsim` (`refsim.Runner.run` in a loop, refsim's in-memory recorder) |
| **SimuLizar stock** | unpatched SimuLizar 5.2.2: product jars only, EDP2 `LocalMemoryRepository` recorder | Palladio 5.2.2 release | OpenJDK 21, `-Xmx4g` | `java/simulizar/CompareSimuLizar` on a flat classpath in OSGi order; cold runs also in the product's OSGi runtime |
| **SimuLizar + VT** | stock plus a one-class virtual-thread patch (`-Dpalladio.virtualThreads=true`) | | OpenJDK 21 | same driver |
| **Slingshot** | event-bus Palladio simulator (University of Stuttgart, actively developed) | nightly 2026-09-01, SSJ, EDP2 in memory | OpenJDK 21, `-Xmx4g` | `java/slingshot/CompareSlingshot` around `SlingshotRunner`, flat classpath; the seeded RNG is injected into the global StoEx factory. Cold runs also in Equinox |
| **SimuCom** | deprecated code-generating simulator | Palladio 5.2.2 with a repaired headless workflow (below) | Temurin 17 (5.2.2's JDT cannot use Java 21), `-Xmx4g` | `simucom/run-simucom.sh`, own OSGi application |
| **SimuLizar stock on SSJ** | stock SimuLizar 5.2.2 on the SSJ engine (below) | 5.2.2 product jars plus the release's SSJ engine bundle 5.2.2 and SSJ 3.3.1 | OpenJDK 21, `-Xmx4g` | the SimuLizar driver with the SSJ engine factory (`engine=ssj`); cold runs also in the product's OSGi runtime, engine chosen by preference |
| **SimuCom on SSJ** | SimuCom 5.2.2 on the SSJ engine | as SimuCom, plus the two SSJ bundles | Temurin 17, `-Xmx4g` | `simucom/run-simucom.sh`, engine chosen by preference |
| **EventSim** | archived (2022), unsupported; a historical data point | 5.1.0 on Palladio 5.1 | Temurin 11 | `eventsim.sh`, with an in-memory counting store instead of R |

Not included: the PCM2LQN/LQNS solver, which is analytical, and ProtoCom, which generates
prototypes; neither is a simulator.

**SimuCom repair.** In 5.2.2, SimuCom fails out of the box: the Xtend templates generate
references to classes that moved to `de.uka.ipd.sdq.simucomframework.core.*`.
`bench/compare/simucom/` fixes this with its own OSGi bundle, without changing any product jar:
it rewrites the moved packages in the generated code, adds headless versions of two methods that
5.2.2 made abstract, adds a missing bundle requirement and declares the missing project nature
again. SimuCom ignores the monitor repository and records its built-in sensors. With the same
seed it reproduces SimuLizar's request counts and mean response times exactly.

**SSJ engine.** SimuLizar and SimuCom do not schedule events themselves: they run on Palladio's
abstract simulation engine (`de.uka.ipd.sdq.simulation.abstractsimengine`), which has two
implementations. DESMO-J is the default and the only one shipped in the Palladio 5.2.2 product;
SimOxide reproduces it. SSJ (Stochastic Simulation in Java, Université de Montréal) is an
optional bundle of the same release (`org.palladiosimulator.simulation.abstractsimengine.ssj`
5.2.2 with the library `ca.umontreal.iro.simul.ssj` 3.3.1, from the 5.2.2 update site), with its
own event list (a splay tree) and floating-point clock; it was expected to be the faster engine.
Both simulators pick the engine from the
preference `de.uka.ipd.sdq.simulation/simulationEngineField` (Eclipse: Preferences → Palladio
Simulation); the benchmark sets it through the OSGi configuration, and the flat-classpath
SimuLizar driver injects the engine factory directly. Comparing SimOxide only with DESMO-J would
overstate its speed-up if SSJ were much faster, so SimuLizar and SimuCom were measured on both
engines ([Palladio on the SSJ engine](#palladio-on-the-ssj-engine)).

**Machine.** AMD Ryzen 9 9950X3D VM: 22 cores, 1 thread per core, 34 GB RAM, Linux 6.8. Each
benchmark process runs alone: the scripts wait until the machine has at most 2 runnable tasks.
A watchdog kills a benchmark when free memory drops below a floor (8 GB for the JVM runs). Repeated
warm measurements differ by −5 % to +8 %, so differences below about 10 % are noise.

## Workloads

Every simulator gets the same `corpus/` model files with the corpus monitor repository, which
records response times of the scenario, the calls and the operations, plus resource state,
utilisation and demand. The runs stop on simulated time only. The seed is 1; the sustained runs
use seeds 1, 2, 3, … in turn.

| Model | What | Short | Long |
|---|---|---|---|
| `x_ss_minimal` | Slingshot E2E MinimalModel: closed workload, 1 user, one FCFS CPU demand | 1 000 s | 100 000 s |
| `x_espresso` | espresso example: closed workload, 30 users, PS CPU | 100 s | 2 000 s |
| `x_ss_mediastore` | Slingshot E2E MediaStore: closed workload, branches, fork, external calls, linking resource, 2-core PS | 2·10^5 s | 10^7 s |
| `x_sl_mediastore` | SimuLizar MediaStore example: open workload, passive resources, fork, linking resource | 10^6 s | 2.5·10^7 s |
| `h13_passive_contention` | passive resource of capacity 2, 6 closed users | 100 s | 5 000 s |
| `x_pem_fork` | Palladio example PCMFork: closed workload, synchronous fork, PS and delay CPU, HDD | 100 s | 5 000 s |

**Model adjustments**, only where a simulator cannot read the original:

- **Slingshot and passive resources.** Slingshot aborts on an acquire or release action without a
  `ParametricResourceDemand`, from which it reads the number of tokens. For
  `h13_passive_contention` and `x_sl_mediastore` it gets a copy with demand `1` on those actions
  (`bench/compare/models/*_ss`). `run-all.sh check` verifies that SimOxide and refsim produce
  byte-identical measurements for the original and the copy. EventSim rejects such a demand, so
  it gets the originals.
- **Slingshot and `x_pem_fork`.** Slingshot cannot run this model: it does not finish 10 s of
  simulated time in 2 minutes and runs out of heap at 1 000 s. The synchronous fork of
  `h11_fork_sync` fails with a `NoSuchElementException` in `ForkBehaviorContextHolder`.

## Method

- **Warm per run.** One long-lived process per simulator, model and length, with warm-up runs
  first (JVM simulators: 20 short or 2 long; SimOxide: 50 or 3), then 20 or 10 measured runs
  (SimOxide: 200 or 20; SimuCom short: 5 and 10). The tables show the median. A run is
  everything a user's run needs: load the models, build the simulator, simulate, record every
  measurement, and read back the scenario response times. The JVM drivers reload the EMF models
  every run and SimOxide's driver re-parses and recompiles the XMI; SimuCom also generates,
  compiles and installs code every run.
- **Cold one-shot.** One process per run, from process start to results; median of 3. It
  includes JVM start, framework bootstrap (extension registry, EMF, OSGi), class loading and JIT
  warm-up. For SimOxide it is the `simoxide run` CLI, which also writes `measurements.csv`.
- **Rates.** Requests/s are completed usage-scenario runs per wall second; simulated s/s is
  simulated time per wall second. Events/s is shown only where the simulator counts events
  itself (SimOxide: DESMO-J-equivalent event notes; Slingshot: bus events, about 20 times more
  per request); the two are not comparable.
- **Peak RSS.** GNU time `%M`: the largest process of the command.
- **Parallel** (`x_espresso` short). Each tool uses all 22 cores in the ways it supports. After
  warm-up, the workers start together through a file barrier. SimOxide: 22 threads, reloading
  per run or using `run_batch`. SimuLizar and SimuLizar + VT: 22 threads in one JVM, with the
  known workarounds (a sequential warm-up run against an OCL/EMF lazy-initialisation race, one
  EDP2 repository per thread, a shared RNG, so runs are not reproducible). refsim, SimuLizar and
  Slingshot: 22 JVM processes; Slingshot also 22 isolated class loaders in one JVM; SimuCom: 16
  processes (each needs about 1 GB and its own OSGi dock). Worker JVMs run with `-Xmx1g
  -XX:+UseSerialGC`.
- **Sustained, one core.** One process per simulator runs the model back to back with a new seed
  per run for a fixed wall time: `x_ss_mediastore` at 2·10^6 simulated seconds per run for 300 s,
  `x_espresso` short for 120 s, SimOxide 60 s each. There is no warm-up: the JIT warm-up curve is
  part of the measurement. Reported: throughput per 30-s window; steady state (mean of the second
  half); warm-up time (start of the first run from which the mean of 5 consecutive runs stays
  within 10 % of the steady state); p50, p99 and maximum latency; GC pause share; RSS and thread
  count; failures; degradation (last third over middle third). Stock SimuLizar runs exactly as
  shipped, without disposing the RNG; the other SimuLizar drivers dispose it after each run.
- **Sustained, all cores** (`x_ss_mediastore` at 2·10^6 s). The same metrics over all workers:
  300 s for SimuLizar threads and processes and Slingshot, 180 s for refsim and SimuLizar + VT,
  60 s for SimOxide.
- **One very long run** (`x_ss_mediastore`). SimOxide simulates 100 and 500 times "long" (10^9
  and 5·10^9 s); the JVM simulators the largest multiple of "long" (at most 100) that should take
  about 4 minutes at their warm speed, capped at 300 s. Reported: simulated s/s of the whole run
  relative to the warm "long" run (1.0 = no slow-down with run length), and memory growth.

## Results

The tables below are generated by `bench/compare/summarize.py` and `sustained.py` from the raw
logs. "SimOxide speed-up" is the other simulator's time divided by SimOxide's.

<!--@include: ../../bench/compare/results/tables.md-->

<!--@include: ../../bench/compare/results/tables_sustained.md-->

## Observations

- **SimOxide** is flat from the first run: no warm-up and the same throughput in every window. Its
  RSS stays at 7 MB on one core and 64 MB with 22 threads. Its cold long runs are dominated by
  writing `measurements.csv` (50 MB for `h13_passive_contention` long; the simulation itself takes
  10 ms), so their wall time depends on the disk.
- **JIT warm-up.** The first JVM run takes 2 to 9 s; 5 to 15 s of runs pass before the steady
  state. With 22 concurrently starting JVMs this becomes 25 to 55 s.
- **Stock SimuLizar leaks about 0.9 threads per run**: the RNG producer thread is never disposed
  (115 → 433 threads after 369 espresso runs). On one core this barely costs throughput, but RSS
  grows from 0.8 to 2.1 GB on MediaStore. With the dispose workaround the thread count stays flat,
  but RSS still grows (0.8 → 2.0 GB), consistent with a per-run heap leak through the
  never-terminated reconfiguration process.
- **Stock SimuLizar with 22 threads in one JVM collapses.** It runs at 8 to 10 runs/s for about
  3.5 minutes; then the leaked heap fills the 12 GB and GC takes over: 37 % pause share, 0.2 runs/s
  in the last window, p99 latency 58 s, and 3 failed runs (EMF `IndexOutOfBounds`).
- **Stock SimuLizar with 22 processes** grows to 25.6 GB and is killed by the memory watchdog
  after 200 s. With 16 processes it runs at about 8.2 runs/s for 4 minutes, then drops to 2.2
  runs/s as each 1 GB heap fills up.
- **SimuLizar + VT in one JVM** stays at 9.4 to 10.8 runs/s, with 8 failed runs
  (`ConcurrentModificationException`, `IndexOutOfBounds`) from shared static state.
- **Slingshot leaks threads as well**, about 0.86 per run, but its throughput stays flat.
- **EventSim degrades steadily**: espresso from 4.9 to 1.0 runs/s within 2 minutes, RSS 1.3 →
  2.1 GB.
- **refsim**, whose patches reset the per-run static state, stays flat.
- **GC** takes 0.7 to 3.8 % on one core, so it is not what makes the JVM simulators slow;
  per-run interpretation overhead and thread hand-offs are.
- **Run length.** No simulator gets slower per simulated second as a run gets longer: the very
  long runs reach 0.9 to 1.1 of the warm "long" rate. Memory grows with the number of stored
  measurements: SimOxide needs 322 MB at 10^9 s and 1.6 GB at 5·10^9 s (80 million tuples at
  about 16 bytes each); the JVM simulators 1.1 to 3.1 GB at 2·10^8 to 9·10^8 s.
- **Time limit.** Simulated time is an `i64` in nanoseconds in SimOxide and in DESMO-J. A run of
  10^10 s crosses 2^63 ns (292 years), wraps around as in Java and aborts with the reference's
  error.

## Palladio on the SSJ engine

SimuLizar and SimuCom on SSJ were measured in a separate session (2026-10-21; AMD Ryzen 9 9950X3D
VM with 25 cores and 34 GB; the all-core runs with 22 workers, as above) together with
SimOxide and with SimuLizar and SimuCom on DESMO-J, with the same models, simulated times, phases
and drivers as above (`bench/compare/ssj/run.sh`). Within the session, every pair of engines ran
alternately per model and length on the same machine, so the SSJ to DESMO-J ratios do not depend
on the session. Additional model: `x_misarch`, a generated PCM model of the MiSArch microservice
system (34 containers, 1 146 actions, 38 open workloads, asynchronous forks), run for 36 s and
361 s of simulated time. SimuCom cannot generate compilable code for it (entity names such as
`createUser@user` or `return`; with Java-safe names, a package clash remains), so it is left out.

- **SSJ against DESMO-J**: SSJ is not faster. A warm SimuLizar run takes 78 to 126 % (median
  97 %) and a warm SimuCom run 85 to 132 % (median 99 %) of the time on DESMO-J; cold runs 92 to
  117 %. SSJ is slower on the fork-heavy `x_pem_fork` (127 % long) and on `x_misarch` (119 to
  124 %): it starts a new thread per simulated process where DESMO-J reuses pooled threads.
- **SimOxide speed-up** on SSJ: 238× to 3 271× (SimuLizar) and 208× to 2 866×
  (SimuCom) per warm run, against 253× to 2 588× and 241× to 2 169× on
  DESMO-J in the same session.
- **Results**: on every benchmark model and length (seed 1), SimuLizar on SSJ gave the same request counts and mean response times as on DESMO-J and as SimOxide, and SimuCom on SSJ the same request counts as on DESMO-J and mean response times equal to a relative 2·10^-9 (SSJ's floating-point clock). The sanity table's `x_misarch` rows are not comparable: the drivers report one of its 38 usage scenarios, not the same one.
- **Sustained runs**: on one core both engines are flat (MediaStore: 0.487 runs/s on SSJ, 0.48 on
  DESMO-J; no failed runs). On all cores, 22 SimuLizar processes on SSJ reach 13.6 runs/s before
  the memory watchdog kills them at 24 GB (DESMO-J: 8.4 runs/s, also killed); 22 threads in one
  JVM collapse on both engines (SSJ: 3.96 runs/s, 9 failed runs; DESMO-J: 4.92 runs/s, 13 failed).

<!--@include: ../../bench/compare/ssj/results/tables.md-->

<!--@include: ../../bench/compare/ssj/results/tables_sustained.md-->

## Slingshot flavour and SPD against Slingshot

Benchmark B1 of the reconfiguration design (`DESIGN-SPD.md` §6): SimOxide's
[Slingshot flavour](../guide/slingshot.md) (fast mode, fixed behaviour) and its Slingshot-exact
mode against stock Slingshot, on plain models and on Slingshot's scaling-policy (SPD) examples.
Measured on 2026-10-21 on the quiet 25-core, 34 GB VM (AMD Ryzen 9 9950X3D), each benchmark process
alone, 3 processes per configuration with seeds 1 to 3 (`bench/compare/spd/b1.sh`; every table and
the raw figures in `bench/compare/results/2026-10-21-b1/summary.md`).

- **Slingshot**: the nightly of 2026-09-01 through the S0 reference runner in Equinox
  (`reference/slingshot/run.sh --stock`, seeded, no instrumentation or measurement output),
  OpenJDK 21, `-Xmx4g`; 31 runs per JVM for short and 8 for long runs, the first third discarded as
  warm-up. Its run includes loading the models (about 10 % of a warm run).
- **SimOxide**: per run, load and compile the model and simulate (`simoxide-compare`, as above);
  SPD runs with `spd-bench` (`crates/simoxide-reconf/examples`), which creates the simulation with
  its scaling policies on a compiled model and times the adjustments; the QVTo engine is set up
  once per process (12.5 to 14 ms).

**Plain models, warm per run** (median):

| model | simulated s | Slingshot flavour | Slingshot-exact | Slingshot | Slingshot / Slingshot flavour | Slingshot / Slingshot-exact |
|---|---|---|---|---|---|---|
| `x_ss_minimal` | 1 000 | 0.27 ms | 0.61 ms | 134 ms | 490× | 219× |
| `x_ss_minimal` | 100 000 | 8.8 ms | 42.7 ms | 6.62 s | 755× | 155× |
| `x_espresso` | 100 | 0.39 ms | 1.53 ms | 182 ms | 464× | 119× |
| `x_espresso` | 2 000 | 4.7 ms | 26.5 ms | 2.85 s | 607× | 107× |
| `x_ss_mediastore` | 2·10^5 | 1.18 ms | 1.8 ms | 267 ms | 226× | 148× |
| `x_ss_mediastore` | 10^7 | 7.0 ms | 36.9 ms | 9.02 s | 1 285× | 244× |

Cold one-shot (process start to results): SimOxide 7 to 164 ms (`simoxide run`, writing
`measurements.csv`), Slingshot 3.3 to 13.9 s (JVM and Equinox start-up plus the first run). Peak
RSS: 6 to 12 MB against 0.8 to 1.3 GB. The Slingshot-exact mode replays Slingshot's whole event
cascade (14 times the events on `x_ss_mediastore`) and is 1.5 to 6 times slower than the Slingshot flavour.
Results: the Slingshot-exact mode and stock Slingshot (seed 1) finished the same requests with the
same mean response time on all six runs (e.g. 937 requests, 21 303 s on `x_ss_mediastore` long);
the Slingshot flavour draws its own random numbers and differs within noise (938, 21 288 s).

**SPD examples, warm per run** (median; `reference/slingshot/models`, 100 to 600 s simulated):

| model | adjustments | Slingshot flavour, fixed | Slingshot flavour, Slingshot's quirks | Slingshot | Slingshot / Slingshot's quirks |
|---|---|---|---|---|---|
| `spd_minimal` | 1 | 0.98 ms | 1.0 ms | 133 ms | 133× |
| `spd_scale_in` | 2 | 1.41 ms | 1.46 ms | 136 ms | 93× |
| `spd_service_group` | 3 | 2.61 ms | 2.61 ms | 140 ms | 54× |
| `spd_same_instant` | 2 (quirks: 4) | 1.49 ms | 2.65 ms | 138 ms | 52× |
| `spd_time_policies` | 4 (quirks: 8) | 2.67 ms | 4.12 ms | 146 ms | 35× |
| `spd_v4_cpu` (CPU utilisation) | 2 (quirks: 15 to 17) | 1.69 ms | 10.5 ms | 610 ms | 58× |
| `spd_v4_rt` (response time) | 0 (quirks: 3 to 5) | 0.21 ms | 3.58 ms | 528 ms | 147× |

- "Slingshot's quirks" runs the flavour with the compat flags stock Slingshot has
  (`--quirks fast --spd-quirks slingshot`): it makes Slingshot's decisions (16 adjustments in
  Slingshot's own `spd_v4_cpu` run, 15 to 17 here); the fixed default scales less (for example, a
  scaling policy no longer reacts to another policy's time event, Q15).
- An adjustment costs 0.5 to 0.9 ms (QVTo `MainTransformation` plus the change path) and takes 78
  to 98 % of a short SPD run. Slingshot's per-run time includes its adjustments; it is not
  instrumented to separate them.
- Cold one-shot: `simoxide run` with the SPD 20 to 53 ms (including the QVTo set-up), Slingshot 3.2
  to 4.2 s. Peak RSS 13 to 15 MB against 0.63 to 0.69 GB.
- The SimuLizar flavour runs the same models (with time-driven utilisation monitors, which
  SimuLizar needs) at the same speed. It does not scale `spd_v4_cpu` at all: with the fixed
  behaviour, the CPU trigger never sees the 80 % threshold (0 adjustments in 600 s, the Slingshot
  flavour 2). Noted, not investigated.
- Not measured: Slingshot's snapshots (its initialisation extension needs the MENTOR application,
  which is not installed, `DESIGN-SPD.md` U7). SimOxide's snapshot and fork costs are in
  [Engine performance](./engine.md#runtime-changes-spd-and-snapshots).

**SimuLizar's reconfiguration rules** (`corpus-rules/`; stock SimuLizar cannot run them on the
flat classpath, so refsim, the patched SimuLizar 5.2.2, stands in; warm runs after the first in one
JVM):

| case | simulated s | reconfigurations | SimOxide (run, exact) | of it in rules | refsim warm | refsim first run in JVM |
|---|---|---|---|---|---|---|
| `lb_outsource` | 1 000 | 99 | 17.7 ms | 8.4 ms | 1.94 s | 7.6 s |
| `lb_rules_qvto` | 1 000 | 99 | 107 ms | 95 ms | 2.05 s | 7.7 s |
| `power_rules_qvto` | 2 000 | about 13 000 | 1.11 s | 1.08 s | 5.72 s | 12.0 s |

The power example evaluates its rules about 13 000 times; at about 80 µs per evaluation SimOxide
spends almost all its time in the QVTo engine, so its lead shrinks to 5×.

## Fairness caveats

- **Same work only within the SimuLizar family.** SimOxide, refsim, stock SimuLizar, SimuLizar +
  VT and SimuCom (DESMO-J semantics, same seed) produce identical request counts and mean
  response times. Slingshot has its own semantics and RNG use and draws branch decisions from the
  unseedable `Math.random()`; it matches exactly on the Slingshot E2E models and closely
  elsewhere. EventSim differs more: on `x_sl_mediastore` it completes 527 instead of 833 requests.
  Compare those two on equal model and simulated time only.
- **Recorded measurements differ.** SimOxide, refsim, SimuLizar and Slingshot record every
  monitor of the corpus monitor repository (SimOxide keeps every tuple in memory, refsim uses its
  own recorder, SimuLizar and Slingshot EDP2 in memory). SimuCom records its default sensors into
  EDP2. EventSim only counts its default probes, which flatters it compared with a real setup that
  stores them in R. Storing measurements costs SimOxide up to 20 %.
- **Harness weight differs.** Slingshot's runner is a thin test harness without workflow jobs or
  OCL validation; the SimuLizar family goes through its full workflow.
- **SimuCom's cost is mostly code generation**: 0.3 to 1.5 s per run for generating, compiling
  and installing the simulation code. A SimuCom user pays this for every model change, but could
  skip it when only the seed changes.
- **Stock SimuLizar gets workarounds in some runs.** The warm, cold and process-parallel runs
  dispose the RNG after each run and use a fixed seed. Only the sustained one-core runs show stock
  SimuLizar exactly as shipped. In-JVM parallel runs need the workarounds and are then not
  reproducible.
- **JIT and GC.** The JVM numbers are for a warmed JIT on JDK 21 with G1 (single process) or
  Serial GC (22 workers). Other collectors or heap sizes change them by tens of percent, not by
  orders of magnitude. The all-core runs of stock SimuLizar are limited by memory, not CPU.
- **SimOxide's batch driver.** In the sustained all-core run, the `run_batch` driver computes the
  summaries of each batch on one thread while the other cores idle, so it reaches only about half
  the throughput of 22 threads that each reload the model. Its row is left out of the table (it
  is in `sus_summary.csv`). With short espresso runs, batch and reload are equal.
- **SSJ is not DESMO-J.** The two engines get the same random streams (the generator belongs to
  the simulation configuration, not to the engine), but SSJ keeps simulated time as a `double` in
  seconds where DESMO-J counts nanoseconds, and orders simultaneous events by its own event list.
  After a first difference the runs would follow different sample paths, so on SSJ the
  SimuLizar family is only guaranteed to match DESMO-J (and SimOxide) statistically; in this
  benchmark they matched: on every benchmark model and length (seed 1), SimuLizar on SSJ gave the same request counts and mean response times as on DESMO-J and as SimOxide, and SimuCom on SSJ the same request counts as on DESMO-J and mean response times equal to a relative 2·10^-9 (SSJ's floating-point clock). SSJ also ends a run at the maximum simulated time with a
  plain `Simulator.stop()`, without the engine-independent `stop()` that DESMO-J calls; the
  simulation model is then not finalised (no stop listeners, no resource deactivation), and the
  threads of simulated users parked at the end are never released: on `x_espresso` (30 users),
  SimuLizar on SSJ grows by about 30 threads per run in a long-lived JVM, on DESMO-J it stays flat.
  In the sustained one-core runs this shows as thread growth, not as lost throughput: on
  `x_espresso`, SimuLizar on SSJ grows from 1 255 to 10 551 threads in 344 runs and SimuCom on SSJ
  from 756 to 7 507 in 248 runs (DESMO-J: 110 → 400 and 152 → 157), at 1.7 and 2.8 GB RSS; no
  run failed and no thread limit was reached (`ulimit -u` 62 128).

## Reproducing

```sh
bench/compare/run-all.sh                       # build check cold warm sus1 susN longrun summary (about 3.5 h)
bench/compare/run-all.sh par                   # parallel throughput
bench/compare/run-all.sh summary               # only re-aggregate results/raw/
SIMS="simoxide slingshot" MODELS=x_espresso LENS=short bench/compare/run-all.sh cold warm summary
SIMS=simoxide MEM_FLOOR_KB=2000000 bench/compare/run-all.sh cold warm par sus1 susN longrun summary
bench/compare/ssj/run.sh                       # SimOxide, SimuLizar and SimuCom on DESMO-J and SSJ (bench/compare/SSJ.md)
```

Each phase resumes: a benchmark process whose log ends with `rc=0` is not run again. The raw logs
(`results/raw/`) are not in the repository; the CSV summaries and the Markdown tables are:

| File | Contents |
|---|---|
| `summary.csv` | per simulator, model and length |
| `processes.csv` | wall time and peak RSS of each benchmark process |
| `parallel.csv` | parallel throughput |
| `sus_windows.csv`, `sus_samples.csv`, `sus_summary.csv` | sustained runs: 30-s windows, RSS and threads every 2 s, per configuration |
| `longrun.csv`, `longrun_progress.csv` | the very long runs |
| `tables.md`, `tables_sustained.md` | the tables on this page |

The JVM simulators need the Palladio 5.2.2 product, the Slingshot nightly, Palladio 5.1 for
EventSim and the matching JDKs; `bench/compare/env.sh` holds their paths. Raw logs recorded
before the project was renamed use the label `pcmsim`; the summarizers map it to `simoxide`.

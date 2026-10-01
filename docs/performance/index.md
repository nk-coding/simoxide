# Performance overview

SimOxide produces the same results as SimuLizar 5.2.2 and is two to three orders of magnitude
faster than every other Palladio simulator, on one core and on all cores. It also has no
warm-up, does not leak and needs a few MB of memory instead of GBs.

All numbers on this page come from one machine (AMD Ryzen 9 9950X3D VM, 22 cores, 34 GB) and one
benchmark harness, `bench/compare/`. Every simulator runs the same model files for the same
simulated time. The method and every table are on [Comparison in detail](./comparison.md).

## At a glance

| Simulator | Status | Warm run, SimOxide is faster by | Cold start to results | One core, MediaStore | All 22 cores, MediaStore |
|---|---|---|---|---|---|
| **SimOxide** | this project | – | 4 to 10 ms (short runs) | **426 runs/s** | **5 396 runs/s** at 64 MB |
| SimuLizar 5.2.2 (stock) | supported | 256× to 2 420× | 5.4 to 17 s | 0.50 runs/s | 11.3 runs/s at 26 GB, then killed; 4.1 runs/s in one JVM (collapses) |
| SimuLizar 5.2.2 + virtual threads | patched | 222× to 1 270× | 5.5 to 17 s | 0.53 runs/s | 9.7 runs/s at 13 GB, 8 failed runs |
| refsim (deterministic SimuLizar) | this project's reference | 222× to 2 240× | 5.5 to 16 s | 0.57 runs/s | 17 runs/s at 19 GB |
| Slingshot (nightly 2026-09-01) | actively developed | 182× to 1 030× | 2.7 to 13 s | 0.61 runs/s | 9.4 runs/s at 15 GB |
| SimuCom 5.2.2 (repaired) | deprecated | 237× to 2 100× | 7.9 to 16 s | 0.45 runs/s | – |
| EventSim 5.1 | archived, unsupported | 76× to 340× | 3.8 to 7.5 s | 0.66 runs/s, degrading | – |

- **Warm run**: one simulation in a long-lived, warmed-up process, from the model files to the
  results in memory. The range covers six models, each at a short and a long simulated time.
- **Cold start**: one process per simulation, from process start to results.
- **One core**: sustained throughput of `x_ss_mediastore` runs of 2·10^6 simulated seconds
  (about 190 requests each), back to back with a new seed per run.
- **All cores**: the same, with the best configuration each simulator supports (threads,
  processes or isolated class loaders).

## What this means in practice

- **An evaluation that takes a second in a JVM simulator takes about a millisecond in
  SimOxide.** On MediaStore, a candidate evaluation of 2·10^6 simulated seconds takes 2.3 ms
  instead of 1.6 to 2.2 s. On all cores, SimOxide completes over 5 000 of them per second; the
  best JVM configuration completes 17.
- **A one-shot command line run starts in milliseconds**, instead of 3 to 17 s of JVM start,
  framework bootstrap and JIT warm-up. Short runs finish in 3 to 10 ms including loading and
  compiling the model. Long cold runs are dominated by writing `measurements.csv` (up to 50 MB).
- **Throughput is flat from the first run.** The JVM simulators need 5 to 15 s of runs to reach
  their steady state on one core, and 25 to 55 s when 22 JVMs start together.
- **Memory stays small.** SimOxide needs 4 to 17 MB per process, and 64 MB for 22 threads. The
  JVM simulators need 0.7 to 2.5 GB per process; 22 processes need 10 to 26 GB.
- **Long simulations do not slow down.** SimOxide simulates 5·10^9 s of MediaStore in 4.2 s
  (1.2·10^9 simulated seconds per second); the JVM simulators manage 1 to 4·10^6.

## Stability over time

| Simulator | Behaviour in sustained runs |
|---|---|
| SimOxide | flat throughput, no warm-up, constant memory |
| SimuLizar 5.2.2 (stock) | leaks a thread per run (the RNG producer thread is never disposed) and heap per run. In one JVM with 22 threads it collapses after about 3.5 minutes (37 % GC, 0.2 runs/s, failed runs); 22 processes exceed the memory limit |
| SimuLizar + virtual threads | flat throughput, but the heap still grows; in one JVM, 8 runs fail on shared static state |
| refsim | flat; its patches reset the per-run static state |
| Slingshot | flat throughput, but leaks about 0.86 threads per run |
| SimuCom | flat; code generation costs 0.3 to 1.5 s per model change |
| EventSim | degrades within minutes (espresso: 4.9 to 1.0 runs/s in 2 minutes) |

## Beyond speed

| | SimOxide | SimuLizar family (stock, VT, refsim, SimuCom) | Slingshot | EventSim |
|---|---|---|---|---|
| Same results as SimuLizar 5.2.2 | byte-identical | identical request counts and mean response times | exact on its own E2E models, close elsewhere | differs (e.g. 527 instead of 833 requests on the SimuLizar MediaStore) |
| Reproducible per seed | yes | refsim only | no: branch decisions use the unseedable `Math.random()` | yes |
| Runs every benchmark model | yes | yes | no: fails on synchronous forks, needs an extra demand on acquire and release | yes |
| Parallel runs in one process | yes, no shared state | only with workarounds, not reproducible | isolated class loaders | – |

## SimOxide on its own

For SimOxide's own benchmark suite (events per second, allocations, thread scaling, the fast
mode) and the design choices behind the numbers, see [Engine performance](./engine.md). In
short: 6 to 31 million events per second on one core, about 13x on 22 threads, a live heap of
6 to 130 KiB per simulation, and up to 3.4 times more on distribution-heavy models in
[fast mode](../guide/fast-mode.md).

## When these numbers were taken

SimOxide was measured on 2026-09-30 at the current state of the repository. The JVM simulators
were measured on 2026-09-29 on the same machine with the same harness; their versions have not
changed since. `bench/compare/run-all.sh` reproduces everything (about 3.5 hours;
`SIMS=simoxide` re-measures SimOxide alone in a few minutes).

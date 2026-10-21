# Engine performance

SimOxide's own numbers: how fast the engine runs the benchmark models, how it scales over
threads, what the fast mode adds, and which design choices the numbers rest on. For the
comparison with other simulators see the [overview](./index.md).

Measured on 2026-10-21: AMD Ryzen 9 9950X3D VM (25 cores, 34 GB, quiet), rustc 1.98.1, release
profile (fat LTO), the local exact build (`--features exact,hotspot-math,reconfiguration`; fast-mode
rows use its `--mode fast`), one thread pinned to one core unless stated otherwise; each suite
figure is the median of three `simoxide-bench` invocations (`bench/compare/engine.sh`, results in
`bench/compare/results/2026-10-21-engine/`). The earlier figures of 2026-09-30 came from a 22-core
VM with the same CPU; where they changed by more than the noise, the text says so. How to rerun everything is described in
[Benchmarking](../development/benchmarking.md).

## Summary

- **6 to 29 million events per second** on one core, in exact mode, with every measurement stored.
- **Short runs in 12 to 530 µs**, including setting up the simulation. Loading and compiling a
  model takes 0.1 to 0.9 ms.
- **No allocation per event** on most models: the steady state reuses its memory. The peak live
  heap of one simulation is 6 to 130 KiB, plus 16 bytes per stored measurement.
- **12.6x on 22 threads and 13.5x on 25** for independent runs of one compiled model.
- **The fast mode** adds up to 3.4x on models that sample distributions heavily, and little on
  models that do not.

## Benchmark suite

`simoxide-bench suite --reps 3`, exact mode. "Short" is the corpus `run.json` configuration;
"long" runs 20 000 measurements (5 000 for h21 and h27, 100 to 2 000 for the generated models).

| Benchmark | Load µs | Compile µs | Short run µs | Long run ms | M events/s | M requests/s | Allocations/event | Peak heap KiB |
|---|---|---|---|---|---|---|---|---|
| mediastore (SimuLizar example) | 749 | 193 | 39.5 | 61.2 | 11.1 | 0.327 | 0.000 | 23 |
| espresso | 114 | 6 | 21.3 | 3.8 | 20.6 | 5.33 | 0.004 | 24 |
| h13 passive resource | 146 | 9 | 33.8 | 7.0 | 17.2 | 2.88 | 0.047 | 10 |
| fork (x_pem_fork) | 151 | 10 | 31.1 | 10.8 | 28.6 | 1.86 | 0.000 | 8 |
| fork sync (h11) | 145 | 9 | 34.8 | 11.7 | 24.8 | 1.72 | 0.001 | 26 |
| call chain (h14) | 196 | 16 | 37.4 | 12.5 | 16.9 | 1.60 | 0.001 | 15 |
| nested subsystem | 172 | 11 | 12.1 | 3.3 | 23.4 | 6.13 | 0.001 | 6 |
| StoEx distributions (h21) | 159 | 27 | 137.6 | 24.2 | 6.1 | 0.206 | 0.001 | 11 |
| INNER collection (h27) | 157 | 16 | 29.1 | 4.6 | 8.4 | 1.08 | 0.377 | 11 |
| PS 4 cores, about 20 jobs | 138 | 8 | 33.8 | 6.1 | 15.7 | 3.27 | 0.020 | 129 |
| generated, size 10 (#5) | 459 | 65 | 33.8 | 3.1 | 11.5 | 0.651 | 0.114 | 13 |
| generated, size 10 (#6) | 505 | 87 | 532.6 | 2.3 | 6.4 | 0.045 | 0.153 | 76 |

Events are DESMO-J-equivalent event notes, so events/s compares models with each other. Against
2026-09-30 the simulation figures are unchanged within noise (±3 %); loading is 20 to 25 % slower
on the larger models (mediastore 609 → 749 µs) since the loader moved onto the graph shared with
the QVTo engine ([Loading](#loading)).
Requests are finished usage-scenario runs, which compares the two modes. Counting measurements
instead of storing them is up to 23 % faster (mediastore stores 2.2 million tuples per long run).

## Against the reference

The same runs in refsim (warm JVM, trace off) and in SimOxide, 20 000 measurements each; every
output file is byte-identical (2026-09-30, 22-core VM; not re-measured):

| Model | refsim | SimOxide | Speed-up |
|---|---|---|---|
| mediastore (2.2 M tuples) | 111.7 s (needs `-Xmx6g`) | 0.060 s | about 1 900x |
| fork sync (h11) | 25.0 s | 0.011 s | about 2 300x |
| StoEx distributions (h21) | 20.4 s | 0.098 s | about 210x |

h21 is the slow case: more than half of its time in exact mode goes into the reference's
numerical inverse CDFs, which SimOxide reproduces bit for bit (see [Fast mode](#fast-mode)).

## Thread scaling

`simoxide-bench batch --runs 176`: mediastore, 20 000 measurements per run, `run_batch` on one
compiled model, exact mode, 25-core VM.

| Threads | 1 | 2 | 4 | 8 | 16 | 22 | 25 |
|---|---|---|---|---|---|---|---|
| Runs/s | 16.1 | 31.2 | 59.3 | 113.4 | 187.5 | 203.0 | 217.2 |
| Speed-up | 1.00 | 1.94 | 3.68 | 7.04 | 11.7 | 12.6 | 13.5 |

The runs share nothing but the immutable compiled model. Efficiency drops above 8 threads on
this VM (the 22-core VM of 2026-09-30: 13.1x on 22 threads); the fast mode scales the same way
(25 threads: 222.6 runs/s). With 176 runs, the 22- and 25-thread rows are 7 to 8 runs per thread, so
they vary by about 10 % between invocations.

## Fast mode

`simoxide-bench suite --reps 3 --mode fast`, compared by finished requests per second (the modes
draw different random numbers, so their runs do slightly different work):

| Benchmark | Exact M requests/s | Fast M requests/s | Fast vs exact |
|---|---|---|---|
| mediastore | 0.327 | 0.327 | 1.00x |
| espresso | 5.33 | 5.29 | 0.99x |
| h13 passive resource | 2.88 | 2.83 | 0.99x |
| fork (x_pem_fork) | 1.86 | 1.85 | 1.00x |
| fork sync (h11) | 1.72 | 1.78 | 1.04x |
| call chain (h14) | 1.60 | 1.61 | 1.00x |
| nested subsystem | 6.13 | 6.28 | 1.02x |
| StoEx distributions (h21) | 0.206 | 0.698 | 3.39x |
| INNER collection (h27) | 1.08 | 1.39 | 1.29x |
| PS 4 cores, about 20 jobs | 3.27 | 3.73 | 1.14x |
| generated, size 10 (#5) | 0.651 | 0.814 | 1.25x |
| generated, size 10 (#6) | 0.045 | 0.101 | 2.24x |

The engine is the same in both modes, so the gain is exactly the cost of the reference's random
numbers: the MT19937 stream with its bookkeeping, and above all the inverse CDFs of `Norm`,
`Lognorm` and `Gamma` (bracketing plus Brent's method, 0.5 to 16 µs per sample) and the Poisson
bisection. Engine-bound models gain little; distribution-heavy ones gain 2 to 3.4x. The model
generator uses distributions far more than the hand-made models do.

## Loading

XMI loading and compilation happen once per model; for batch use they do not matter. For one-shot
CLI runs they are most of the cost of a short run.

| Model | Objects | From disk µs | From memory µs | With a warm `ParseCache` µs |
|---|---|---|---|---|
| mediastore (x_sl_mediastore) | 1 220 | 644 | 613 | 258 |
| screencast (x_pem_screencast_ms) | 623 | 308 | 288 | 143 |
| h01_ps_single | 158 | 76 | 59 | 53 |
| generated, size 10 (gen_s6) | 974 | 418 | 398 | 184 |

Best of 300, including typed-model build and validation (`cargo run --release -p simoxide-model
--example load-bench`), median of three invocations, one core of the 25-core VM (2026-10-21). On
2026-09-30 (22-core VM) the same table read 532 / 513 / 188 µs for mediastore, 268 / 245 / 94,
71 / 51 / 31 and 358 / 331 / 118: loading is 15 to 25 % slower from disk and memory and up to 70 %
slower with a warm `ParseCache` since the loader moved onto the graph shared with the QVTo engine
(merge step M5, whose gate was 5 %). Not investigated further here. XML tokenising takes about 45 µs of mediastore's time; creating objects,
setting values and resolving references most of the rest; typed build and validation 90 µs.

## Design

What the numbers rest on, roughly in order of effect:

- **Flat, pre-resolved code.** Every SEFF and usage behaviour is compiled once per model into a
  sequence of small `Copy` instructions with resolved operands (`simoxide_sim::code`). A process
  is a program counter into that code plus a stack of frames, not a thread or a coroutine. Two
  variants are compiled: one with trace instructions, and one without, which a run without trace
  never dispatches.
- **Calls resolved once.** A provided-role call resolves its target (a composite's delegation
  or a basic component's SEFF) once per run; external-call sites and entry-level calls cache
  their last resolution. Constant StoEx (processing rates, constant demands and delays) is
  stored as an `f64` and bypasses the evaluator.
- **Fewer dispatches.** A continuation that pushes a call, a SEFF or a child behaviour runs it
  directly (up to a nesting of 24), and a process woken by the end of a wait runs inside the
  waking event when its `Resume` event would be the next event anyway. On mediastore this cuts
  dispatches from about 5 to about 1.7 per event. The remaining cost is dominated by branch
  mispredictions on an event's resumption, not by cache misses.
- **No allocation in the steady state.** Parameter frames come from a pool of recycled frames
  (only unshared frames are recycled, so no reference can observe the reuse); process stacks are
  boxed and swapped by pointer; errors are boxed and cold.
- **A small event core.** A 4-ary heap on `(time, sequence)` compared as one `u128`, with
  per-resource timer slots. Processor sharing selects the shortest job without data-dependent
  branches.
- **Exact numerics, computed once.** Pure functions that the reference recomputes are memoized:
  `logGamma` during a Brent inversion, Poisson CDF values per mean, and the bracket values Brent
  would evaluate again. The results are bit-identical.
- **One monomorphized simulator per mode.** The exact/fast choice is a type parameter, chosen
  once per run.
- **Loading.** Attribute values are borrowed from the text; bundled models (`Palladio.resourcetype`,
  `commonMetrics.metricspec`, ...) are parsed once per process and copied in; `ParseCache`
  shares parsed files between loads and threads; internal maps use FxHash.

Every optimisation keeps the exact mode's outputs byte-identical; see
[Contributing](../development/contributing.md#guardrails) for the checks.

## Where the time goes

**mediastore** (exact, about 80 ns per event): the continuation handlers and their dispatch are
about half of the self time; StoEx evaluation and parameter passing about 20 %; the schedulers
10 to 13 %; the event list about 4 %; measurement emission about 3 %. Storing the 2.2 million tuples of a long run costs another
20 %.

**Distribution-heavy models** (exact): the reference-exact inverse-CDF sampling dominates, for
example about two thirds of h21's time. These are bit-exact reproductions of the reference's
algorithms and cannot change without changing the samples; the fast mode replaces them.

## Evaluated and not used

| Idea | Result |
|---|---|
| Galloping bracket search for inverse CDFs (feature `fast-bracket`) | returns the reference's bracket only if the computed CDF is monotone along all skipped points, which cannot be proven for the floating-point `erf` and `regularizedGammaP`; no gain on real models, whose scales are far below 1, so the reference's first step already brackets the root. Stays an opt-in feature for large-scale distributions |
| Merging hand-offs while other events are pending at the same instant | +10 to 30 % on tie-heavy models, but it reorders simultaneous events and changes results |
| Virtual-time processor sharing | +20 % at about 20 concurrent jobs, but it changes rounding; stays opt-in (`--ps-algorithm virtual-time`) |
| Same-time FIFO in front of the event heap | no gain: the heap is small |
| Pure binary search in PMF/PDF sampling | 4 % slower on mediastore, whose PDFs are front-loaded; a 16-entry linear prefix is used instead |
| Direct handler calls on every return path | −3 to +1 % |
| Insertion order instead of Java `HashMap` order for `INNER` | no measurable gain, and not exact |
| mimalloc | no difference (`--features mimalloc` switches it on) |

## Not done

- Merging hand-offs that happen inside a running process (passive-resource grants, fork
  children, joins). The woken process can only run after the current one waits, so this needs a
  queue of "run next" processes and a proof that no event can be scheduled in between.
- Building the typed model without the generic XMI graph, and interned strings in the model
  (both change public types).

## Runtime changes, SPD and snapshots

Costs of the reconfiguration features, quiet 25-core VM, 2026-10-21 (`bench/compare/engine.sh` and
benchmark B1, `bench/compare/spd/b1.sh`; results in `bench/compare/results/2026-10-21-{engine,b1}/`).

| What | Cost |
|---|---|
| Fork of a run halfway (`simoxide-bench fork`, `Simulation::snapshot`, measurements counted) | 0.3 to 3 µs (11 µs with 117 live processes); with stored rows the rows dominate: MediaStore, 1.1 M tuples, 0.6 ms |
| Fork of an SPD run with its QVTo state (`SpdRun::fork` + `QvtoAdjuster::fork_spd`) | 3 to 19 µs: 53 000 to 330 000 forks/s |
| Snapshot of an SPD run to bytes (postcard, with the QVTo section) / from bytes / restore | 84 to 780 µs for 2.7 to 137 KB / 6 to 111 µs / 74 to 1 010 µs; every restored run ends exactly as the uninterrupted one |
| Snapshot file of a long MediaStore run through the CLI (`--pause-at`, `--snapshot`) | 1.3 MB (`x_ss_mediastore`) and 0.75 MB (`x_sl_mediastore`), about 4 ms more than the uninterrupted run |
| Runtime model change (`simoxide-bench change`: graph copy, rebuild, recompile, merge) | MediaStore (1 220 objects) 0.32 ms; small models 28 to 57 µs; generated size 10 0.13 to 0.14 ms |
| SPD adjustment (Slingshot's `MainTransformation` plus the change path), SPD example models | 0.5 to 0.9 ms; the QVTo set-up (core and `MainTransformation`) once per process 12.5 to 14 ms |
| HPA scaling step at MiSArch's size (30 services, 2 279 objects; `tests/hpa.rs cost_per_scaling_step`, 3 runs) | 2.4 ms per adjustment (2.35 to 4.1), of which the change path 1.6 to 2.5 ms; 40 services 3.0 to 3.5 ms; the whole 900 s run with 114 adjustments 0.32 to 0.52 s |
| SimuLizar reconfiguration rules (power example, 2 000 s, 13 349 reconfigurations) | 1.1 s of rules (about 80 µs per reconfiguration, the PRM mirror 8 µs of it) in QVTo's ordered, fast and compat modes alike |

A short SPD run is mostly adjustments: on the SPD examples (100 to 600 s simulated, 1 to 17
adjustments) 78 to 98 % of a 1 to 10 ms run is spent in them.

## Slingshot flavour

The [Slingshot flavour](../guide/slingshot.md) runs on the same engine (`--flavour slingshot`,
fixed behaviour). Finished requests per second against the SimuLizar flavour, medians of three
`simoxide-bench suite --reps 3` invocations pinned to one core, quiet 25-core VM (2026-10-21):

| | geomean (11 models) | range |
|---|---|---|
| exact RNG mode | ×1.00 | ×0.86 (call chain h14, more link transfers) to ×1.08 |
| fast RNG mode | ×1.04 | ×0.91 to ×1.12 |

`generated s10 (#6)` stops with a run error in the Slingshot flavour (a negative FCFS demand,
which Slingshot rejects). A 200 000 s run of `x_ss_mediastore` simulates in 0.11 ms (1.2 ms
including loading and compiling the model), 100 s of `x_espresso` in 0.19 ms (0.39 ms); stock
Slingshot needs 267 ms and 182 ms for them in a warm JVM, including its model loading
([Slingshot flavour against Slingshot](./comparison.md#slingshot-flavour-and-spd-against-slingshot)).
The flavour hooks cost the SimuLizar flavour nothing measurable (geomean +0.2 % exact, +2.7 %
fast, within noise; `DESIGN-SPD.md`, S2 as built).

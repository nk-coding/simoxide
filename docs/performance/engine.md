# Engine performance

SimOxide's own numbers: how fast the engine runs the benchmark models, how it scales over
threads, what the fast mode adds, and which design choices the numbers rest on. For the
comparison with other simulators see the [overview](./index.md).

Measured on 2026-09-30: AMD Ryzen 9 9950X3D VM (22 cores), rustc 1.98.1, release profile (fat
LTO), one thread pinned to one core unless stated otherwise. How to rerun everything is described in
[Benchmarking](../development/benchmarking.md).

## Summary

- **6 to 31 million events per second** on one core, in exact mode, with every measurement stored.
- **Short runs in 12 to 530 µs**, including setting up the simulation. Loading and compiling a
  model takes 0.1 to 0.8 ms.
- **No allocation per event** on most models: the steady state reuses its memory. The peak live
  heap of one simulation is 6 to 130 KiB, plus 16 bytes per stored measurement.
- **13x on 22 threads** for independent runs of one compiled model.
- **The fast mode** adds up to 3.4x on models that sample distributions heavily, and little on
  models that do not.

## Benchmark suite

`simoxide-bench suite --reps 3`, exact mode. "Short" is the corpus `run.json` configuration;
"long" runs 20 000 measurements (5 000 for h21 and h27, 100 to 2 000 for the generated models).

| Benchmark | Load µs | Compile µs | Short run µs | Long run ms | M events/s | M requests/s | Allocations/event | Peak heap KiB |
|---|---|---|---|---|---|---|---|---|
| mediastore (SimuLizar example) | 609 | 172 | 40.0 | 62.4 | 10.7 | 0.321 | 0.000 | 23 |
| espresso | 99 | 5 | 21.0 | 3.7 | 20.3 | 5.36 | 0.004 | 24 |
| h13 passive resource | 139 | 10 | 33.8 | 7.0 | 16.7 | 2.84 | 0.047 | 10 |
| fork (x_pem_fork) | 125 | 8 | 29.7 | 10.2 | 28.7 | 1.97 | 0.000 | 8 |
| fork sync (h11) | 131 | 9 | 34.4 | 11.5 | 24.4 | 1.74 | 0.001 | 25 |
| call chain (h14) | 189 | 19 | 37.5 | 12.4 | 17.2 | 1.62 | 0.001 | 15 |
| nested subsystem | 133 | 8 | 12.1 | 3.2 | 23.8 | 6.21 | 0.001 | 6 |
| StoEx distributions (h21) | 127 | 25 | 135.5 | 23.8 | 6.2 | 0.210 | 0.001 | 11 |
| INNER collection (h27) | 138 | 17 | 29.4 | 4.8 | 8.3 | 1.05 | 0.377 | 11 |
| PS 4 cores, about 20 jobs | 126 | 9 | 33.1 | 6.1 | 15.8 | 3.28 | 0.020 | 127 |
| generated, size 10 (#5) | 376 | 61 | 34.0 | 3.1 | 11.4 | 0.654 | 0.114 | 13 |
| generated, size 10 (#6) | 437 | 73 | 527.3 | 2.2 | 6.3 | 0.046 | 0.153 | 76 |

Events are DESMO-J-equivalent event notes, so events/s compares models with each other.
Requests are finished usage-scenario runs, which compares the two modes. Counting measurements
instead of storing them is up to 23 % faster (mediastore stores 2.2 million tuples per long run).

## Against the reference

The same runs in refsim (warm JVM, trace off) and in SimOxide, 20 000 measurements each; every
output file is byte-identical:

| Model | refsim | SimOxide | Speed-up |
|---|---|---|---|
| mediastore (2.2 M tuples) | 111.7 s (needs `-Xmx6g`) | 0.060 s | about 1 900x |
| fork sync (h11) | 25.0 s | 0.011 s | about 2 300x |
| StoEx distributions (h21) | 20.4 s | 0.098 s | about 210x |

h21 is the slow case: more than half of its time in exact mode goes into the reference's
numerical inverse CDFs, which SimOxide reproduces bit for bit (see [Fast mode](#fast-mode)).

## Thread scaling

`simoxide-bench batch --runs 176`: mediastore, 20 000 measurements per run, `run_batch` on one
compiled model.

| Threads | 1 | 2 | 4 | 8 | 16 | 22 |
|---|---|---|---|---|---|---|
| Runs/s | 15.8 | 30.7 | 59.9 | 113.8 | 182.0 | 206.1 |
| Speed-up | 1.00 | 1.95 | 3.80 | 7.22 | 11.6 | 13.1 |

The runs share nothing but the immutable compiled model. Efficiency drops above 8 threads on
this VM; the fast mode scales the same way (22 threads: 211.7 runs/s).

## Fast mode

`simoxide-bench suite --reps 3 --mode fast`, compared by finished requests per second (the modes
draw different random numbers, so their runs do slightly different work):

| Benchmark | Exact M requests/s | Fast M requests/s | Fast vs exact |
|---|---|---|---|
| mediastore | 0.321 | 0.325 | 1.01x |
| espresso | 5.36 | 5.59 | 1.04x |
| h13 passive resource | 2.84 | 3.02 | 1.06x |
| fork (x_pem_fork) | 1.97 | 1.94 | 0.98x |
| fork sync (h11) | 1.74 | 1.86 | 1.07x |
| call chain (h14) | 1.62 | 1.65 | 1.02x |
| nested subsystem | 6.21 | 6.41 | 1.03x |
| StoEx distributions (h21) | 0.210 | 0.705 | 3.36x |
| INNER collection (h27) | 1.05 | 1.37 | 1.31x |
| PS 4 cores, about 20 jobs | 3.28 | 3.90 | 1.19x |
| generated, size 10 (#5) | 0.654 | 0.828 | 1.27x |
| generated, size 10 (#6) | 0.046 | 0.098 | 2.13x |

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
| mediastore (x_sl_mediastore) | 1 220 | 532 | 513 | 188 |
| screencast (x_pem_screencast_ms) | 623 | 268 | 245 | 94 |
| h01_ps_single | 158 | 71 | 51 | 31 |
| generated, size 10 (gen_s6) | 974 | 358 | 331 | 118 |

Best of 300, including typed-model build and validation (`cargo run --release -p simoxide-model
--example load-bench`). XML tokenising takes about 45 µs of mediastore's time; creating objects,
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

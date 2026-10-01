# SimOxide performance

> Historical log of the performance work, not maintained. Current numbers:
> `docs/performance/`. Paths to other documents refer to the layout of that time.

How SimOxide's speed is measured, where the time goes, what the performance pass of agent J
(2026-09-29) changed, and what was tried and rejected. Every change keeps the output bit-identical
to the reference (see "Guardrails"), except the opt-in fast mode ("Fast mode" at the end).

## Measuring

| Command | What it measures |
|---|---|
| `cargo run --release -p simoxide-cli --bin simoxide-bench -- suite [--filter a,b] [--reps N]` | Per benchmark model: XMI load and IR compile time, a short run (µs/run, allocations), a long run (M events/s with measurements counted only and stored, allocations per event, peak live heap of one simulation). Best of `--reps` repetitions. |
| `simoxide-bench batch --model DIR --threads 1,2,4,8 --runs N` | `run_batch` scaling over threads. |
| `simoxide bench --model DIR --max-measurements M --runs N [--threads T]` | Events/s of one model with any run flags. |
| `simoxide load-bench --model DIR --runs N` | XMI load plus IR compile only. |
| `--profile out.svg` or `--profile out.folded` on `bench` and `load-bench` | In-process sampling profiler (pprof, SIGPROF at 5 kHz). Needs `--features profile`. `.folded` writes one line per stack, frames as `name@file:line`, root first. |
| `--mode exact\|fast` on `simoxide run/bench` and `simoxide-bench suite/batch` | The simulator mode (default exact). `suite` and `batch` also print simulated requests/s, the fair comparison between modes. |

The benchmark models are listed in `crates/simoxide-cli/src/bin/simoxide-bench.rs`. Three of them
live in `crates/simoxide-cli/bench/models/` (`ps_many`, `gen_s5`, `gen_s6`), the rest in
`corpus/`. The allocator is the system allocator; `--features mimalloc` switches `simoxide` and
`simoxide-bench` to mimalloc.

For A/B comparisons, keep the old binary and run both alternately, several times, on a quiet
machine. Other agents' campaigns easily add 10 % noise, so check `uptime` and treat differences
under about 3 % as noise.

## Guardrails (run after every change)

1. `cargo test --release -p simoxide-sim` (includes `tests/bugs.rs` and the corpus comparisons).
2. `crates/simoxide-cli/bench/golden.sh`: prints a hash of the trace, tape and measurements of
   every corpus model under three run configurations (390 runs). The hashes must not change;
   compare against a list saved before the change.
3. `simoxide-fuzz corpus --corpus corpus --sim "cmd:…simoxide run …"` (both modes, reference expected
   outputs) and the same with `--corpus corpus-fuzz`.
4. For work on aborting runs, `reference/reffail-cmp.sh`.

## Results

AMD Ryzen 9 9950X3D, rustc 1.98.1, release profile, one thread.

### Before and after this pass

"Before" is the simoxide binary saved at 03:22 for a fuzz campaign, before this pass started;
"after" is the final state. Both use
`simoxide bench --runs 5 --max-measurements <long_meas of the suite>`, best of three invocations,
run alternately.

| benchmark | before (M events/s) | after (M events/s) | speed-up |
|---|---|---|---|
| mediastore | 6.07 | 9.96 | 1.64x |
| espresso | 11.69 | 17.01 | 1.46x |
| h13 passive | 9.70 | 13.45 | 1.39x |
| fork (x_pem_fork) | 15.06 | 24.93 | 1.66x |
| fork sync (h11) | 11.80 | 20.90 | 1.77x |
| call chain (h14) | 7.18 | 12.82 | 1.79x |
| nested subsystem | 12.93 | 17.60 | 1.36x |
| StoEx dists (h21) | 3.51 | 4.37 | 1.25x |
| INNER coll. (h27) | 4.56 | 5.43 | 1.19x |
| PS 4 cores, ~20 jobs | 7.53 | 12.45 | 1.65x |
| generated s10 (#5) | 6.71 | 8.97 | 1.34x |
| generated s10 (#6) | 3.83 | 4.72 | 1.23x |

### Suite (final state)

| benchmark | load µs | compile µs | short: events | short µs/run | short allocs | long: events | long ms/run | Mev/s (count) | Mev/s (store) | allocs/event | peak heap KiB |
|---|---|---|---|---|---|---|---|---|---|---|---|
| mediastore | 1617 | 200 | 409 | 50.9 | 768 | 819999 | 81.9 | 10.01 | 8.84 | 1.147 | 27 |
| espresso | 251 | 3 | 230 | 24.5 | 416 | 80030 | 4.3 | 18.61 | 18.02 | 0.503 | 28 |
| h13 passive | 299 | 6 | 508 | 41.3 | 356 | 125888 | 8.8 | 14.23 | 13.71 | 0.365 | 12 |
| fork (x_pem_fork) | 308 | 7 | 801 | 34.3 | 229 | 319995 | 12.2 | 26.23 | 25.44 | 0.125 | 9 |
| fork sync (h11) | 287 | 6 | 749 | 40.3 | 258 | 299999 | 13.9 | 21.61 | 20.64 | 0.134 | 31 |
| call chain (h14) | 384 | 14 | 559 | 48.0 | 557 | 219999 | 16.9 | 13.01 | 12.73 | 0.728 | 17 |
| nested subsystem | 328 | 7 | 201 | 15.0 | 175 | 80001 | 4.5 | 17.91 | 17.48 | 0.501 | 7 |
| StoEx dists (h21) | 318 | 26 | 932 | 184.7 | 144 | 150839 | 32.2 | 4.69 | 4.63 | 0.067 | 13 |
| INNER coll. (h27) | 316 | 15 | 237 | 40.8 | 906 | 40035 | 6.6 | 6.03 | 6.01 | 3.502 | 12 |
| PS 4 cores, ~20 jobs | 284 | 5 | 526 | 38.0 | 410 | 100014 | 8.1 | 12.41 | 11.84 | 0.413 | 160 |
| generated s10 (#5) | 840 | 73 | 343 | 41.0 | 675 | 36033 | 3.8 | 9.56 | 9.37 | 1.392 | 13 |
| generated s10 (#6) | 931 | 69 | 3084 | 697.8 | 4299 | 15026 | 2.9 | 5.09 | 4.92 | 1.018 | 84 |

### Thread scaling (`simoxide-bench batch`, mediastore, 64 runs, 20 000 measurements)

| threads | wall s | runs/s | Mev/s | speed-up | efficiency |
|---|---|---|---|---|---|
| 1 | 5.232 | 12.2 | 10.0 | 1.00 | 100% |
| 2 | 2.668 | 24.0 | 19.7 | 1.96 | 98% |
| 4 | 1.387 | 46.1 | 37.8 | 3.77 | 94% |
| 8 | 0.748 | 85.6 | 70.2 | 7.00 | 87% |

## Changes of this pass

All of them are output-neutral: the golden hashes and all corpus comparisons are unchanged.

- **Interpreter dispatch** (`sim.rs`). `step()` is a thin dispatcher; each large continuation
  kind has its own `#[inline(never)]` function (`step_sbeh`, `step_internal`, `step_prov_role`,
  `step_asm_conn`, `step_ext_call`, `step_elsc`, `step_ubeh`, …). This keeps the hot paths small
  and makes profiles attributable per continuation kind.
- **Fewer round trips through the dispatcher.** `step_sbeh`, `step_internal` and `step_asm_conn`
  loop over stages that neither wait nor push a child continuation. An SEFF action that finishes
  without a child (e.g. a `SetVariable`) is completed in the same step.
- **`ConsumeDone` only with a trace.** The continuation pushed while a process waits for a
  demand only writes the trace line of the completed demand, so it is skipped without a trace
  (`wait_consumed`).
- **Resolved provided-role calls** (`Simulation::provs`, `prov_id`, `resolve_prov`). A
  `(assembly context, role, signature)` call is resolved once per run: the provided delegation
  of a composite or the basic component's SEFF, and the assembly-operation series. `Cont::ProvRole`
  and `Cont::AsmConn` carry the call id. Before, each call did three hash lookups
  (`asmop_series`, `prov_deleg`, `seff_for`) and walked the model. Resolution failures are kept
  as `ProvTarget::Fail` and raised when the call happens, as before.
- **Small `Result`s.** The internal `R<T>` is `Result<T, Box<SimError>>`; `err()` and the
  evaluation-error constructor are `#[cold]`. The public API still returns `SimError`.
- **StoEx evaluation without bookkeeping.** With the own RNG and no tape output
  (`Rng::plain`), `eval` skips the origin tracking and sample recording that only the tape and
  replay need.
- **PMF/PDF sampling** (`simoxide_stoex::probfn::first_above`). The linear scan for the first cumulative
  probability above `u` covers the first 16 entries, and a binary search the rest. The cumulative
  sums of validated probabilities are non-decreasing (checked once, `monotone`), so the search
  finds the same index; NaN falls through to `None` in both. `binary_search_equals_linear_scan`
  tests it. The 22 large `DoublePDF`s of mediastore (up to 300 entries) were 3 % of its time.
- **Memoized `log_gamma`** (`simoxide_random::special::log_gamma_memo`). `regularized_gamma_p/q`
  recompute `logGamma(a)` on every CDF evaluation, but during a Brent inversion `a` is fixed
  (0.5 for `erf`, so Normal and Lognorm; `alpha` for Gamma). A one-entry per-thread memo of the
  pure function gives identical values. Samplers: Norm(0,1) 860 → 637 ns, Gamma(2,0.5)
  751 → 495 ns, Lognorm(1,0.5) 1003 → 723 ns (`cargo run --release -p simoxide-random --example
  bench`); h21 +13 %.
- **Tooling:** `simoxide-bench`, `simoxide load-bench`, `--profile` with folded output.

Two correctness fixes were made during the same pass (`docs/BUGS.md`): BUG-1, Java's two-digit
`Double.toString` of subnormals (`javafmt::two_digit_subnormal`), and BUG-2, the FCFS abort
after a double queue (`simoxide_sched::Fcfs`, now `Result`-returning). Neither costs measurable time.

## Where the time goes now

**mediastore** (profile of 20 runs, self time, approximate): the continuation steps and the
dispatcher about 30 %; StoEx evaluation (`eval_node`, PMF/PDF sampling) about 15 %; event list
`pop` about 6 %; malloc/free about 6 % (1.15 allocations per event, mostly parameter frames for
external calls). The rest is spread over frames, the schedulers (`consume`, `transmit`, PS
bookkeeping) and the measurement listeners.

**h21 (StoEx distributions):** more than half the time is the reference-exact inverse CDF
(Commons Math bracketing plus Brent), and 15 % is Poisson sampling. These are bit-exact
reproductions of the reference algorithms and cannot change without changing the samples. The
opt-in `simoxide-random/fast-bracket` feature (galloping bracket search) does not help here: the
distributions have small scales and short brackets. It helps only for large-scale
Normal/Gamma/Lognorm (Gamma(2,100): 7.8 → 1.8 µs before the memo).

**Load** (`load-bench`, mediastore: 2.2 ms load plus 0.2 ms compile): 83 % of the total is XMI
loading. `resolve_all`, which demand-loads the referenced resources, accounts for 54 % of the
total. Parsing is
allocation-heavy: attribute strings via `String::from_utf8_lossy` and `normalize_attr`,
malloc/free about 18 %, and per-line counting with `memchr_count`. For batch use the model is
loaded once, so this matters only for one-shot CLI runs.

## Tried and rejected

- **Same-time FIFO in the event list** (notes scheduled at the current time bypass the heap).
  No measurable gain on any benchmark, because the heap is small. It was removed. The fuzz cases
  `corpus-fuzz/pz1007_s1*` (a `stop` ordered before a same-time resource completion) were saved
  while that build was the current binary and pass without it.
- **Pure binary search in PMF/PDF sampling** (for more than 8 entries): mediastore about 4 %
  slower than the plain linear scan, because its PDFs are front-loaded. Hence the 16-entry scan
  prefix.
- **Measurement storage ablation:** skipping `emit` entirely gains < 1 % on mediastore; not
  worth pursuing.

## Ideas not done

- ~~Pool or reuse parameter frames~~ (done in round 3: `FramePool`).
- ~~XMI loading: borrow attribute values instead of allocating~~ (done by agent M, "Loading").
- ~~Poisson: a per-mean table of the CDF~~ (done in round 3: a per-mean memo of the CDF values,
  the bisection unchanged).
- See "Byte-identical optimizations (round 3)" for what is left.

## Loading (agent M, 2026-09-29)

XMI loading (`crates/simoxide-model`: parse, `resolve_all`, typed build, validation), made faster
without changing any result. Every model directory of the corpus and of the EMF oracle list
(231) loads to the same graph (`Debug` of all objects incl. source lines), canonical dump, typed
model and diagnostics, in strict, tolerant and entry-file mode (`load-bench snapshot`, compared
with a binary built before the change). The EMF golden test (163 dirs) is unchanged.

**Measuring.** `cargo run --release -p simoxide-model --example load-bench -- bench [--reps N]
[--phases] [--memory] [--cache|--cache-miss] [dirs]` loads like `simoxide-sim` does (entry files,
the rest demand-loaded) and prints µs (best of N) and allocations per load; `--memory` loads
from memory, `--cache` with a shared `ParseCache`, `--profile out.folded` with
`--features profile`. `load-bench snapshot DIR` writes the full loader output per model for
before/after `diff -r`.

**Results** (best of 300, µs per load incl. build and validation; "file" = from disk as before,
"memory" = `load_memory`, "cache" = in memory, every file already in a `ParseCache`):

| model | objects | before | file | memory | cache | allocations (file) |
|---|---|---|---|---|---|---|
| mediastore (x_sl_mediastore) | 1220 | 1257 | 532 | 513 | 188 | 5798 |
| screencast (x_pem_screencast_ms) | 623 | 664 | 268 | 245 | 94 | 3289 |
| h01_ps_single | 158 | 226 | 71 | 51 | 31 | 1379 |
| generated s10 (gen_s6) | 974 | 837 | 358 | 331 | 118 | 4276 |
| all 69 benchmark dirs (sum) | 19090 | 23600 | 8200 | 7000 | 3200 | |
| 8 generated s10 models (sum) | 4321 | 4550 | 1750 | 1570 | 616 | |

`simoxide load-bench` (load + compile, mean of 500): mediastore 1750 → 926 µs, h01 300 → 120 µs,
gen_s6 1171 → 615 µs (compile includes agent L's concurrent changes). The system allocator and
mimalloc give the same times now.

**Changes.**
- Attribute values and keys are borrowed from the text (`Cow`); they are copied only into the
  graph's values. A start tag's attributes are split by a small scanner (quick-xml's iterator
  as fallback for unusual syntax, tested against it on all corpus tags). Normalisation checks
  for control characters and `&` in one vectorised pass.
- Source lines: one `memchr` pass over the text at the end of a parse instead of counting per
  event (lines of diagnostics are computed on demand).
- Bundled models (`Palladio.resourcetype`, `commonMetrics.metricspec`, ...) are parsed once per
  process and copied in with shifted object ids: a parse only creates and links objects of its
  own resource, so the copy equals a fresh parse. This was about 40 % of a small model's load.
- `ParseCache` (opt-in, `Loader::set_cache`): the same mechanism for any resource with the same
  URI, display name and text, shared between loaders and threads (LRU, default 64 resources).
  A resource is cached on its second load (fingerprint of the text first, full comparison on a
  hit), so files that are loaded once cost < 3 %.
- ID index per resource without an allocation per ID (keys in one string); FxHash instead of
  SipHash for all internal maps; metamodel lookups by precomputed per-class tables
  (containment features, required references, typed-model kind, attribute defaults); `href`
  locations resolved once per file and without path normalisation when plain.
- `resolve_all` and tree traversals reuse buffers; a resolved proxy is found without
  allocating; the unresolved references are recorded by `resolve_all` instead of a second
  traversal of all objects in `finish`.
- Slot vectors are allocated with the usual size of their class (learned per process), the
  object vector per resource from the text size.

**In-memory API.** `load_memory(files)` / `load_memory_entries(files, entries)`,
`Loader::add_file`, `set_read_files(false)`, `load_memory_dir`: never touches the file system;
equal to directory loading (`tests/memory.rs`, all corpus, bench and edge-case dirs).

**Where the time goes now** (mediastore, from memory): XML tokenising (quick-xml) about 45 µs;
creating objects and setting values (hash lookups of features, value parsing, string copies)
most of the rest of the 420 µs parse; typed build and validation 90 µs. About 5 allocations per
object remain, from the public owned types (`Box<str>` values, a `Vec` of slots per object,
`Box<str>` in the typed model).

**Not done.** Building the typed model without the generic graph (`Model::graph` is public and
used by `simoxide-sim`); `Arc<str>` or interned strings in `raw::Value` and the typed model (would
change public types).

## Limits and reference comparison (agent N, 2026-09-29)

**Cost of `Limits`.** Adding the limit checks costs one counter per interpreter step, a
continuation-depth check per SEFF entry and a `Result` from `spawn`. A/B against a binary built
from the sources before the change (best of 7, alternating):
- mediastore: 10.0 vs 9.9 M events/s (unchanged);
- h11 (fork sync): 20.2 vs 20.8 M events/s (−2 to −3 %).

The golden hashes are unchanged.

**Against the reference** (warm `refsim batch --no-trace`, 20 000 measurements; outputs
byte-identical):

| Model | refsim | SimOxide | Speed-up |
|---|---|---|---|
| mediastore | 111.7 s (needs `-Xmx6g`: the recorder keeps 2.2 M tuples) | 0.107 s | ~1000x |
| h11 | 25.0 s | 0.017 s | ~1450x |
| h21 | 20.4 s | 0.140 s | ~145x |

**Thread scaling on the 22-core VM** (mediastore, 176 runs): 1 / 8 / 16 / 22 threads give
11.7 / 84 / 141 / 156 runs/s.

## Fast mode (agent Q, 2026-09-30)

Round 3 (see the next section) moved the engine shortcuts described here (merged hand-offs,
`INNER` without copies) into the exact mode, removed `Mode::FastRng` (now the test-only policy
`compat::Literal<Fast>`) and made `RunResult::events` count the merged notes in every mode. The
fast mode now differs from the exact mode only in its random numbers. The tables of this section
are the state before round 3.

`SimConfig::mode = Mode::Fast` (`simoxide run|bench --mode fast`, `simoxide-bench suite|batch
--mode fast`) removes what exists only to reproduce the reference's bits and keeps the model
semantics (`docs/deviations.md` "Fast mode" lists every difference; `docs/testing.md` §8 the
equivalence evidence).

**Design.** A compile-time policy, `simoxide_sim::compat::Compat`, with the associated random
source type (tape bookkeeping for `Exact`, xoshiro256++ plus fast samplers for `Fast`) and
constants for the engine switches. `Simulation<'m, C: Compat = Exact>` is monomorphized per
policy, so the mode costs nothing per event; `simoxide_sim::run` and `run_batch` choose the
monomorphization once per run from `SimConfig::mode`. The distribution functions are hooks of
`simoxide_random::UniformSource` whose defaults are the reference's algorithms, so the StoEx
evaluator needed no change beyond calling the hooks. Cargo: `simoxide-sim/fast` (and
`simoxide-random/fast`) is additive and off by default in the libraries; the `simoxide-cli`
binaries enable it by default (`--no-default-features` builds an exact-only CLI). Tests get it
through simoxide-testkit, so one `cargo test --workspace` covers both modes. `Mode::FastRng`
(fast random numbers on the exact engine) is a diagnostic mode.

Compiling three simulator instantiations into one binary made LLVM stop inlining a few hot
helpers that each had one call site before (event list `pop`, scheduler `process`/`on_wakeup`,
passive `acquire`/`release`, PMF/PDF `value_for`); the exact mode lost 8 to 18 %. They are
`#[inline(always)]` now, and the exact mode is within noise of the exact-only build before this
work (column "exact vs before" below, -3 to +3 %). Golden hashes unchanged.

**Single thread, long runs** (best of 3 × 5; "before" is the binary from before this work; the
split uses `Mode::FastRng`: "random numbers" = generator and samplers, "engine" = merged hand-offs
and `INNER` without copies):

| benchmark | long run: requests | before (exact-only build) ms | exact ms | fast-rng ms | fast ms | exact vs before | fast vs exact | random numbers (exact → fast-rng) | engine (fast-rng → fast) |
|---|---|---|---|---|---|---|---|---|---|
| mediastore | 19992 | 82.0 | 83.3 | 78.9 | 77.5 | 0.98x | 1.07x | 1.06x | 1.02x |
| espresso | 20138 | 4.5 | 4.5 | 4.5 | 4.6 | 1.00x | 0.98x | 1.00x | 0.98x |
| h13 passive | 20114 | 8.7 | 8.8 | 8.5 | 7.6 | 0.99x | 1.16x | 1.04x | 1.12x |
| fork (x_pem_fork) | 19994 | 12.6 | 12.8 | 12.1 | 12.3 | 0.98x | 1.04x | 1.06x | 0.98x |
| fork sync (h11) | 20046 | 14.7 | 14.5 | 14.0 | 12.8 | 1.01x | 1.13x | 1.04x | 1.09x |
| call chain (h14) | 19947 | 17.2 | 17.3 | 16.2 | 15.9 | 0.99x | 1.09x | 1.07x | 1.02x |
| nested subsystem | 19984 | 4.6 | 4.5 | 4.4 | 4.2 | 1.02x | 1.07x | 1.02x | 1.05x |
| StoEx dists (h21) | 5005 | 32.5 | 32.4 | 9.2 | 8.4 | 1.00x | 3.86x | 3.52x | 1.10x |
| INNER coll. (h27) | 4982 | 6.9 | 6.7 | 5.6 | 4.4 | 1.03x | 1.52x | 1.20x | 1.27x |
| PS 4 cores, ~20 jobs | 20057 | 8.2 | 8.1 | 7.1 | 6.7 | 1.01x | 1.21x | 1.14x | 1.06x |
| generated s10 (#5) | 2024 | 3.8 | 3.9 | 3.2 | 3.1 | 0.97x | 1.26x | 1.22x | 1.03x |
| generated s10 (#6) | 102 | 3.0 | 3.0 | 1.4 | 1.3 | 1.00x | 2.31x | 2.14x | 1.08x |

Runs/s, requests/s (finished usage-scenario runs, the fair comparison because the fast mode merges
events), events/s, the short `run.json` run and allocations:

| benchmark | runs/s exact → fast | M requests/s exact → fast | M events/s exact → fast | events per run exact → fast | short run µs exact → fast | allocations per short run exact → fast | allocations per event exact → fast |
|---|---|---|---|---|---|---|---|
| mediastore | 12 → 13 | 0.240 → 0.258 | 9.84 → 5.42 | 819999 → 420000 | 53.1 → 47.7 | 814 → 811 | 1.147 → 2.239 |
| espresso | 222 → 217 | 4.475 → 4.378 | 17.78 → 17.40 | 80030 → 80030 | 25.0 → 22.2 | 419 → 416 | 0.503 → 0.503 |
| h13 passive | 114 → 132 | 2.286 → 2.647 | 14.31 → 8.66 | 125888 → 65838 | 41.0 → 34.2 | 359 → 357 | 0.365 → 0.698 |
| fork (x_pem_fork) | 78 → 81 | 1.562 → 1.625 | 25.00 → 23.55 | 319995 → 289713 | 36.2 → 32.3 | 232 → 229 | 0.125 → 0.138 |
| fork sync (h11) | 69 → 78 | 1.382 → 1.566 | 20.69 → 15.63 | 299999 → 200002 | 41.6 → 34.9 | 261 → 271 | 0.134 → 0.201 |
| call chain (h14) | 58 → 63 | 1.153 → 1.255 | 12.72 → 7.56 | 219999 → 120170 | 49.8 → 40.9 | 569 → 550 | 0.728 → 1.332 |
| nested subsystem | 222 → 238 | 4.441 → 4.758 | 17.78 → 9.52 | 80001 → 40001 | 15.6 → 11.8 | 184 → 181 | 0.501 → 1.001 |
| StoEx dists (h21) | 31 → 119 | 0.154 → 0.596 | 4.66 → 9.28 | 150839 → 77964 | 185.5 → 50.0 | 147 → 163 | 0.067 → 0.130 |
| INNER coll. (h27) | 149 → 227 | 0.744 → 1.132 | 5.98 → 5.11 | 40035 → 22500 | 40.6 → 26.3 | 909 → 463 | 3.502 → 2.892 |
| PS 4 cores, ~20 jobs | 123 → 149 | 2.476 → 2.994 | 12.35 → 8.96 | 100014 → 60030 | 39.3 → 33.5 | 413 → 416 | 0.413 → 0.676 |
| generated s10 (#5) | 256 → 323 | 0.519 → 0.653 | 9.24 → 5.81 | 36033 → 18019 | 42.6 → 34.9 | 684 → 681 | 1.393 → 2.784 |
| generated s10 (#6) | 333 → 769 | 0.034 → 0.078 | 5.01 → 5.79 | 15026 → 7531 | 696.8 → 353.9 | 4327 → 2277 | 1.021 → 0.886 |

The fast mode processes 9 to 50 % fewer events (every hand-off whose `Resume` note would have
been next), so events/s is lower while requests/s is higher; espresso, whose closed users all
think and work in lockstep, has a simultaneous note at almost every hand-off and merges nothing.
Allocations per run are unchanged except `INNER` (h27: half). The peak heap is about the same.

**All cores** (`simoxide-bench batch --threads 1,22 --runs 176`, 22-core VM, runs/s):

| Model | exact 1 thread | fast 1 thread | exact 22 threads | fast 22 threads | fast vs exact, 22 threads |
|---|---|---|---|---|---|
| mediastore (20 000 meas.) | 11.7 | 12.4 | 155 | 155 | 1.00x |
| call chain h14 (20 000) | 55.5 | 60.3 | 724 | 818 | 1.13x |
| StoEx distributions h21 (5 000) | 29.9 | 113 | 431 | 1 524 | 3.53x |
| INNER h27 (5 000) | 139 | 213 | 1 811 | 2 950 | 1.63x |
| generated s10 #6 (100) | 305 | 567 | 4 053 | 7 890 | 1.95x |

Mediastore scales to about 13x on 22 threads in both modes (60 % efficiency); there its 6 %
single-thread gain disappears.

**Generated models.** 200 models of the default generator (sizes 1 to 10, 1 000 measurements,
20 runs each, `simoxide bench`): 18.4 s in exact mode, 9.0 s in fast mode (2.05x). Per model
(the 94 that take at least 20 ms): geometric mean 2.07x, median 2.12x, quartiles 1.71x and 2.50x,
range 0.96x to 5.3x. The generator uses distributions far more than the hand-made corpus models.

**Where the time goes in fast mode.** mediastore: as before, the interpreter (continuation
dispatch about 25 %, `step_*` functions), StoEx evaluation about 9 %, schedulers about 13 %,
frames and malloc/free; random numbers are negligible. h21: StoEx evaluation 31 % (of which the
distribution calls 10 %), schedulers 19 %, the rest the interpreter; sampling no longer dominates
(about two thirds of the time in exact mode, see above).

**Tried and rejected for the fast mode.**
- Merging every hand-off, also when other notes are pending at the same instant: +10 to 30 % on
  tie-heavy models (espresso 1.27x), but it reorders simultaneous events and changed results of
  tie-heavy generated models systematically (`docs/testing.md` §8).
- Virtual-time processor sharing: +20 % on `ps_many` (about 20 jobs on 4 cores), +5 % on
  espresso, nothing elsewhere; its rounding moved events across a time stop in deterministic
  models. It stays a separate opt-in (`--ps-algorithm virtual-time`), now with the exact
  algorithm's handling of a job that is queued again.
- Insertion order instead of Java `HashMap` order for `INNER`/`BYTESIZE`: no measurable gain.

## Round 3 combined (merge of A and B, 2026-09-30)

Round 3 made two sets of changes, and neither changes a result. Agent A moved the hand-off
elision and in-place `INNER` evaluation into exact mode, and added a Poisson memo, Brent reuse
and a frame pool. Agent B added flat instruction code, call caches and a cheaper engine. Their
details follow in the next two sections.

The merge `2c3be07` was checked against the base binary of `15dd467`, with these results:

- `golden.sh`: 312 of 312 hashes are identical.
- Corpus-style directories: 624 hashes are identical in both modes, with and without trace.
- Generated models: 1 000 default and 400 heavy models were run in 3 configurations each, in
  both modes. All output hashes are identical.
- Fast mode now reports a different `events=` count, because A deliberately counts the merged
  hand-off notes too. The outputs themselves are unchanged.
- `simoxide-fuzz corpus` passes 132 of 132, and corpus-fuzz passes 6 of 6.
- `cargo test --release --workspace` passes 208 tests, with `FAST_ENGINE_N=1000` and
  `LITERAL_N=1000`.

Speed was measured as single-thread runs/s, with base and merged runs alternating on core 17.
Each figure is the median of 5 runs, using B's `ab.sh` models:

| model | exact base → merged | speed-up | fast base → merged | speed-up |
|---|---|---|---|---|
| mediastore | 12.0 → 16.5 | 1.38x | 12.6 → 17.1 | 1.36x |
| h14 call chain | 58.4 → 81.0 | 1.39x | 62.1 → 85.5 | 1.38x |
| h11 fork sync | 66.6 → 91.1 | 1.37x | 73.8 → 93.4 | 1.27x |
| h13 passive | 110.5 → 149.1 | 1.35x | 126.9 → 154.9 | 1.22x |
| espresso | 216 → 268 | 1.24x | 218 → 270 | 1.24x |
| gen s10 #5 | 257 → 327 | 1.27x | 321 → 405 | 1.27x |
| gen s10 #6 | 311 → 416 | 1.34x | 596 → 776 | 1.30x |
| fork | 77.4 → 97.8 | 1.26x | 80.6 → 96.3 | 1.19x |
| nested subsystem | 208 → 311 | 1.49x | 234 → 322 | 1.37x |
| h21 StoEx distributions | 30.3 → 41.4 | 1.37x | 113 → 142 | 1.26x |
| h27 INNER | 141 → 213 | 1.51x | 224 → 269 | 1.20x |
| ps_many | 133 → 186 | 1.40x | 140 → 196 | 1.41x |
| **geomean** | | **1.36x** | | **1.29x** |

With 22 threads, mediastore exact goes from 158–163 to 222–224 runs/s (1.39x).

## Byte-identical optimizations (round 3, agent A, 2026-09-30)

Every change keeps all outputs of the exact mode byte-identical: golden hashes of
`bench/golden.sh` (312 runs: trace, tape, measurements, event counts) unchanged against a list
saved before the work, corpus 66/66 in both RNG modes (132/132 cases) and `corpus-fuzz` 6/6,
the random oracle goldens including the big dumps (`SCALE=big gen-golden.sh`: 20 000 parameter
sets per distribution), `cargo test --release --workspace`. The fast mode gives the same
results as before for the same random numbers (measurements of the old and the new binary
identical on 208 runs over all model directories; `FAST_ENGINE_N=1000`).

### What changed

**Task 1: the fast mode keeps only what changes values.** Every `Compat` switch was audited:
the random source (generator, samplers, tape bookkeeping) changes values; the two engine
switches do not, so they are now done in both modes and the trait has only `Rng`, `MODE` and a
hidden `LITERAL_ENGINE` flag for tests:

- *Hand-off elision in the exact mode.* A process woken by the end of a `hold`, a DELAY demand
  or a PS/FCFS demand runs inside the waking event when its `Resume` note would be the next
  note anyway (nothing else pending at `now`, no stop after the event, and, new, no pending
  measurement error or exhausted replay tape, which the main loop would raise between the two
  notes). The trace has no line for a `Resume` note (`docs/trace-format.md`), so traces are
  byte-identical with the elision active; no synthetic lines are needed and no dependence on
  whether a trace is written. The elided note is still counted: `count_event` does the main
  loop's per-event bookkeeping (livelock guard, `n_events`, `Limits` checks) for it, so
  `RunResult::events`, `events=` in the CLI and the errors of `--max-events`,
  `--max-events-per-instant` and replay are unchanged (the livelock counters moved into
  `Simulation` for that). The fast mode now reports these event counts too.
- *`INNER` in place in the exact mode*, with the `inner:<id>` tape origin that the fast-mode
  version had skipped (`eval_inner`).
- `Mode::FastRng` is gone; its role in the tests is taken by `compat::Literal<P>` (policy `P` on
  DESMO-J's literal event sequence), which is not a run-time mode, so the binaries compile two
  simulator instantiations instead of three. `tests/literal_engine.rs` checks `Exact` against
  `Literal<Exact>` on everything (trace, tape, measurements, events, error messages and trace
  prefixes of aborting runs, replay whole and truncated, tight livelock and event limits) over
  all model directories and 60 generated models (1 000 with `LITERAL_N=1000`);
  `tests/fast_mode.rs` checks `Fast` against `Literal<Fast>` (the former `FastRng`). Removing
  the `count_event` call or the `next_time` check makes `tests/literal_engine.rs` fail
  (checked).

**Task 2: remaining byte-identical ideas.**

- *Poisson* (`simoxide_random::dist::PoissonCdfMemo`): the reference's bisection over
  `[0, 2^31 - 1]` evaluates `regularizedGammaQ(x + 1, mean)` about 31 times per sample, mostly
  at the same points (the top of the bisection tree, where the CDF is exactly 1). A per-thread
  memo of the CDF values for the 4 most recent means (a direct table for `x < 256`, a small
  open-addressing table above, cleared when full) returns the value the bisection would
  compute: the CDF is a pure function of `(mean, x)`. The bisection, its comparisons and its
  evaluation order are untouched; errors and NaN are not stored. `Pois(2)`: 1 115 → 77 ns per
  sample. Tested against the unmemoized inversion (`poisson_memo_is_exact`, means from 1e-9 to
  1e6, alternating means, edge uniforms) and the big oracle dump.
- *Bracket values reused by Brent* (`special::bracket_values`, `brent_solve_values`): Commons
  Math's `BrentSolver.solve(f, a, b)` evaluates `f(a)` and `f(b)` again right after the bracket
  search computed them, and the linear bracket search re-evaluates an endpoint that is clamped
  at its bound. `f` is pure, so the known values are used (compared by bit pattern). `Norm(0,1)`
  642 → 559 ns, `Gamma(2,0.5)` 494 → 454, `Lognorm(1,0.5)` 742 → 654, small-scale corpus
  distributions 0-9 % (`cargo run --release -p simoxide-random --example bench`, pinned).
- *Frame pool* (`frames::FramePool`): input frames, component and assembly-context parameter
  frames, `INNER` frames, result frames and the scenario frame come from a pool of recycled
  frames; a frame whose last reference is popped from a process's stack (or dropped at process
  end) keeps its `Rc` box and entry vector. Only unshared frames are recycled (`Rc::get_mut`),
  so no reference can observe the reuse; `Rc::make_mut` became `FramePool::unshare`, which
  copies into a pooled frame. Allocations per event: mediastore 1.147 → 0.000, h14
  0.728 → 0.001, h27 3.502 → 0.378. Run time: +8 % on mediastore and the generated models,
  within ±3 % elsewhere (fork models slightly slower when the pool was also emptied inline at
  process end, hence the out-of-line `recycle_all`).

### Results

`simoxide-bench suite --reps 3`, the binary from before this work ("before") and after, run
alternately 7 times each, pinned to one core, medians; runs/s = long-run events/s divided by the
events per run. The fast mode's events/s now includes the merged `Resume` notes (before, it did
not count them), so compare modes and versions by runs/s.

| benchmark | exact runs/s before → after | exact M events/s | fast runs/s before → after | fast M events/s | allocs/event exact | allocs/event fast |
|---|---|---|---|---|---|---|
| mediastore | 11.7 → 13.0 (1.11x) | 9.63 → 10.69 | 12.4 → 13.4 (1.09x) | 5.19 → 11.01 | 1.147 → 0.000 | 2.239 → 0.000 |
| espresso | 218 → 220 (1.01x) | 17.47 → 17.63 | 218 → 222 (1.02x) | 17.45 → 17.79 | 0.503 → 0.004 | 0.503 → 0.004 |
| h13 passive | 113 → 117 (1.04x) | 14.24 → 14.74 | 129 → 126 (0.97x) | 8.52 → 15.87 | 0.365 → 0.047 | 0.698 → 0.047 |
| fork (x_pem_fork) | 77.6 → 77.4 (1.00x) | 24.82 → 24.78 | 78.7 → 80.4 (1.02x) | 22.80 → 25.73 | 0.125 → 0.000 | 0.138 → 0.000 |
| fork sync (h11) | 65.4 → 71.8 (1.10x) | 19.63 → 21.53 | 75.6 → 75.8 (1.00x) | 15.12 → 22.73 | 0.134 → 0.001 | 0.201 → 0.001 |
| call chain (h14) | 55.5 → 62.5 (1.13x) | 12.21 → 13.76 | 60.2 → 67.0 (1.11x) | 7.23 → 14.75 | 0.728 → 0.001 | 1.332 → 0.001 |
| nested subsystem | 211 → 226 (1.07x) | 16.87 → 18.08 | 232 → 235 (1.01x) | 9.29 → 18.83 | 0.501 → 0.001 | 1.001 → 0.001 |
| StoEx dists (h21) | 29.5 → 37.7 (1.28x) | 4.45 → 5.68 | 112 → 114 (1.02x) | 8.73 → 17.22 | 0.067 → 0.001 | 0.130 → 0.001 |
| INNER coll. (h27) | 143 → 200 (1.39x) | 5.73 → 7.99 | 220 → 255 (1.16x) | 4.94 → 10.20 | 3.502 → 0.378 | 2.892 → 0.378 |
| PS 4 cores, ~20 jobs | 123 → 123 (1.00x) | 12.29 → 12.31 | 146 → 147 (1.01x) | 8.75 → 14.67 | 0.413 → 0.020 | 0.676 → 0.012 |
| generated s10 (#5) | 254 → 285 (1.12x) | 9.14 → 10.26 | 317 → 356 (1.12x) | 5.72 → 12.84 | 1.393 → 0.115 | 2.784 → 0.115 |
| generated s10 (#6) | 321 → 384 (1.19x) | 4.83 → 5.77 | 760 → 812 (1.07x) | 5.72 → 10.90 | 1.021 → 0.155 | 0.886 → 0.147 |

Exact mode: geometric mean 1.11x (1.00x to 1.39x); fast mode: 1.05x (0.97x to 1.16x). Short
runs (corpus `run.json`): mediastore 53.9 → 50.2 µs and 814 → 369 allocations, h21 190 → 148 µs,
h27 41.5 → 31.5 µs and 909 → 183 allocations. The machine was shared with another agent's
benchmarks; single A/B pairs of `simoxide bench` varied by ±3 %.

Where the exact-mode gains come from: the hand-off elision (h13, h11, h14, nested, h27, the
generated models; no effect on espresso, whose hand-offs all have simultaneous notes), `INNER`
in place (h27), Poisson and Brent (h21: 1.28x, `Pois` was about 20 % of its time), the frame pool
(mediastore, generated models). The fast mode had the first two already; it gains from the pool,
and pays for counting the merged notes (h13 0.97x).

### Tried and rejected

- *Galloping bracket search for the inverse CDFs* (`fast-bracket`). It returns the reference's
  bracket only if the computed CDF is monotone along all skipped step points, and neither a
  runtime check of the examined points nor a cheap argument proves that for the floating-point
  `erf`/`regularizedGammaP` (series/continued-fraction switch, rounding near 0 and 1); a proof
  would need a rigorous error bound of those routines. It also cannot gain on the models we have:
  every distribution in the corpus and in the generator has a scale far below 1 (e.g.
  `Norm(0.02, 0.005)`, `Gamma(2, 0.01)`, `Lognorm(-4.5, 0.4)`), where the reference's first step
  of 1.0 already brackets the root (the near endpoint is clamped at the mean or at 0, the far one
  lies beyond the root), so the linear search does exactly one step. It stays an opt-in
  feature for large-scale distributions (Gamma(2,100): 4.3 → 1.1 µs), documented as not
  proven exact.
- *Recycling a terminated process's frames inline in `terminate`*: 2-5 % slower on the fork
  models (code layout of the hot `run_process`); done out of line instead.

### Left

- Merging hand-offs that happen inside a running process (passive-resource grants, fork
  children, joins): the woken process can only run after the current one waits, so this needs
  a queue of "run next" processes and a proof that no note can be scheduled in between.
- The interpreter itself (dispatch, `CAct` lookups: about 15 % self time on mediastore) is
  agent B's work in parallel.

## Leaner interpreter (round 3, agent B, 2026-09-30)

The interpreter now executes flat, pre-resolved code instead of walking action chains through
a stack of continuation kinds, and calls are resolved once per call site. No result changes:
the exact mode is byte-identical in trace, tape and measurements, and the fast mode gives the
same results for the same random numbers (evidence at the end of this section).

**Changes**, roughly in the order of their effect:

- **Flat instruction streams** (`simoxide_sim::code`, `sim/flat.rs`). Every SEFF behaviour and
  every usage-scenario behaviour is compiled once per model into a sequence of small `Copy`
  instructions with pre-resolved operands (`SOp`, 16 bytes; `UOp`, 12 bytes): the `BEGIN` and
  `END` trace lines, one instruction per resource demand, infrastructure call and resource call
  of an internal action, a call and a return instruction per external call and entry-level
  system call, and `Exec` for branches, loops, forks, acquire, release and set variable. A
  behaviour's continuation is `SBeh { pc, t0, caller }` (`UBeh { pc, t0, t1 }`): a demand that
  waits resumes at the next instruction. The `Internal`, `ExtCall`, `Elsc` and `BasicExit`
  continuations are gone (the SEFF exit is done by the end of the provided-role call).
- **Code without trace instructions.** Two variants are compiled; a run without a trace never
  dispatches a `BEGIN`, `END` or `Start` instruction (+3 to +10 %).
- **Call sites cache their resolution.** An external-call site keeps its last resolution
  (`call_required` from the same caller context, keyed by the caller's assembly context and the
  path below it: the continuation it pushed; calls through a required delegation are not
  cached), an entry-level system call its provided-role call. Before, each call did two hash
  lookups (`req_conn`, `prov_ids`), two trie and two container lookups (+3 to +10 %).
- **Constant StoEx as `f64`** (`Prog::konst_f64`): processing rates, constant demands and
  delays bypass the evaluator and the `Value` round trip.
- **Direct entry.** A continuation that pushes a call, a SEFF or a child behaviour runs it
  right away instead of returning to `run_process` (`Simulation::enter`, nesting bounded by
  `MAX_DIRECT` = 24, then the usual dispatch); a child behaviour or loop iteration of a SEFF or
  usage behaviour runs in the same loop. Nested subsystems +8 %, espresso +3 %, neutral
  elsewhere. Dispatches in `run_process` per event: mediastore 5.2 → 1.7, h14 4.4 → 1.6,
  espresso 5.1 → 2.0, gen_s5 3.4 → 1.3.
- **Boxed process stacks**: `run_process` swaps one pointer instead of moving four `Vec`s in
  and out (+1 to +4 %).
- **Branch-free comparisons**: the event heap compares `(t, seq)` as one `u128` (+1 to +3 %);
  processor sharing selects the shortest job and finds a job without data-dependent branches
  (`simoxide-sched`, `ps_many` +15 %).
- **Per-run setup** (short runs): series are looked up by measuring point in the compiled
  model (`CompiledModel::series_by_mp`) instead of a per-run map of all series, and no
  operation-key strings are built for models without assembly-operation monitors (short
  mediastore run +15 %).

`Limits::max_stack_depth` still counts the reference's interpreter levels that no longer have a
continuation (`ProcData::elided`), so the limit fires at exactly the same point
(`tests/interp.rs::stack_depth_limit_fires_where_it_did`). `Limits::max_steps` counts handler
invocations, fewer than before; its doc now says that the count can change between versions.

### Results

`simoxide-bench suite --reps 3`, before (`15dd467`, the starting point of this round) and after,
run alternately three times on one pinned core, medians. Runs/s = 1000 / long ms per run.

Exact mode:

| benchmark | long ms/run | runs/s | M events/s | M requests/s | short run µs | allocations per short run | allocations/event | speed-up |
|---|---|---|---|---|---|---|---|---|
| mediastore | 84.9 → 70.4 | 11.8 → 14.2 | 9.7 → 11.7 | 0.235 → 0.284 | 54.3 → 42.9 | 814 → 725 | 1.147 → 1.147 | 1.21x |
| espresso | 4.6 → 3.7 | 217 → 270 | 17.4 → 21.7 | 4.35 → 5.44 | 25.6 → 21.2 | 419 → 413 | 0.503 → 0.503 | 1.24x |
| h13 passive | 9.2 → 7.5 | 109 → 133 | 13.7 → 16.7 | 2.18 → 2.66 | 42.1 → 35.5 | 359 → 349 | 0.365 → 0.365 | 1.23x |
| fork (x_pem_fork) | 13.0 → 10.3 | 77 → 97 | 24.6 → 31.2 | 1.54 → 1.95 | 36.3 → 29.6 | 232 → 228 | 0.125 → 0.125 | 1.26x |
| fork sync (h11) | 14.9 → 11.9 | 67 → 84 | 20.1 → 25.1 | 1.34 → 1.68 | 43.0 → 35.5 | 261 → 255 | 0.134 → 0.134 | 1.25x |
| call chain (h14) | 17.6 → 13.8 | 57 → 72 | 12.5 → 15.9 | 1.14 → 1.45 | 50.5 → 39.9 | 569 → 553 | 0.728 → 0.728 | 1.28x |
| nested subsystem | 4.7 → 3.4 | 213 → 294 | 16.9 → 23.6 | 4.24 → 5.89 | 15.9 → 12.4 | 184 → 171 | 0.501 → 0.501 | 1.38x |
| StoEx dists (h21) | 34.0 → 32.3 | 29 → 31 | 4.4 → 4.7 | 0.147 → 0.155 | 188 → 177 | 147 → 142 | 0.067 → 0.067 | 1.05x |
| INNER coll. (h27) | 6.8 → 6.4 | 147 → 156 | 5.9 → 6.2 | 0.735 → 0.776 | 41.1 → 38.8 | 909 → 904 | 3.502 → 3.502 | 1.06x |
| PS 4 cores, ~20 jobs | 8.1 → 6.5 | 123 → 154 | 12.3 → 15.4 | 2.47 → 3.08 | 39.9 → 34.5 | 413 → 404 | 0.413 → 0.413 | 1.25x |
| generated s10 (#5) | 3.9 → 3.5 | 256 → 286 | 9.2 → 10.3 | 0.510 → 0.575 | 42.9 → 37.4 | 684 → 668 | 1.393 → 1.392 | 1.11x |
| generated s10 (#6) | 3.1 → 2.7 | 323 → 370 | 4.9 → 5.5 | 0.033 → 0.037 | 714 → 642 | 4327 → 4318 | 1.021 → 1.019 | 1.15x |

Fast mode:

| benchmark | long ms/run | runs/s | M events/s | M requests/s | short run µs | allocations per short run | allocations/event | speed-up |
|---|---|---|---|---|---|---|---|---|
| mediastore | 77.6 → 64.5 | 12.9 → 15.5 | 5.4 → 6.5 | 0.258 → 0.310 | 47.5 → 38.2 | 811 → 722 | 2.239 → 2.238 | 1.20x |
| espresso | 4.5 → 3.7 | 222 → 270 | 17.9 → 21.6 | 4.47 → 5.40 | 22.3 → 18.5 | 416 → 410 | 0.503 → 0.503 | 1.22x |
| h13 passive | 7.8 → 6.2 | 128 → 161 | 8.4 → 10.6 | 2.57 → 3.22 | 34.7 → 28.2 | 357 → 347 | 0.698 → 0.697 | 1.26x |
| fork (x_pem_fork) | 12.4 → 10.5 | 81 → 95 | 23.3 → 27.6 | 1.61 → 1.91 | 32.7 → 26.8 | 229 → 225 | 0.138 → 0.138 | 1.18x |
| fork sync (h11) | 13.0 → 10.4 | 77 → 96 | 15.4 → 19.3 | 1.54 → 1.93 | 35.0 → 29.4 | 271 → 266 | 0.201 → 0.201 | 1.25x |
| call chain (h14) | 15.9 → 13.1 | 63 → 76 | 7.5 → 9.2 | 1.26 → 1.53 | 41.7 → 33.8 | 550 → 534 | 1.332 → 1.332 | 1.21x |
| nested subsystem | 4.2 → 3.1 | 238 → 323 | 9.4 → 12.8 | 4.71 → 6.42 | 11.9 → 8.8 | 181 → 168 | 1.001 → 1.001 | 1.35x |
| StoEx dists (h21) | 8.6 → 7.0 | 116 → 143 | 9.0 → 11.1 | 0.578 → 0.709 | 50.4 → 40.5 | 163 → 158 | 0.130 → 0.130 | 1.23x |
| INNER coll. (h27) | 4.6 → 4.0 | 217 → 250 | 4.9 → 5.6 | 1.09 → 1.25 | 26.6 → 23.9 | 463 → 458 | 2.892 → 2.892 | 1.15x |
| PS 4 cores, ~20 jobs | 6.8 → 5.2 | 147 → 192 | 8.8 → 11.7 | 2.95 → 3.89 | 34.0 → 28.2 | 416 → 407 | 0.676 → 0.676 | 1.31x |
| generated s10 (#5) | 3.1 → 2.7 | 323 → 370 | 5.8 → 6.8 | 0.647 → 0.756 | 34.8 → 29.7 | 681 → 665 | 2.784 → 2.784 | 1.15x |
| generated s10 (#6) | 1.3 → 1.1 | 769 → 909 | 5.8 → 7.0 | 0.081 → 0.096 | 354 → 305 | 2277 → 2275 | 0.886 → 0.884 | 1.18x |

The events per run are unchanged in both modes (the interpreter creates the same events), so
events/s and requests/s improve by the same factor. Allocations per event are unchanged (they
are the frames, not touched in this round); the short runs allocate less (series setup). The
peak heap is 10 to 25 % lower (smaller continuation stacks).

**All cores** (`simoxide-bench batch --threads 1,22 --runs 176`, mediastore, 20 000
measurements, three alternating runs, medians): exact 1 thread 11.4 → 14.0 runs/s, 22 threads
153.5 → 186.2 runs/s (1.21x; 13.3x scaling, as before); fast 22 threads 162.7 → 193.8 runs/s
(1.19x).

### Where the time goes now

mediastore, exact, 85 ns per event (was 103 ns): the continuation handlers (`step_sbeh`,
`step_prov_role`, `step_asm_conn`, `step_ubeh`) and their dispatch about half of the self time;
StoEx evaluation and parameter passing (`eval`, `fill`, `input_frame`) about 20 % inclusive;
schedulers 10 to 13 %; malloc/free about 8 % (frames); event list about 4 %; measurement
emission about 3 %. The largest self-time spots are the first instructions after the
unpredictable indirect branches on an event's resumption (the `Ev` match, the continuation
match in `step`, the instruction match in `step_sbeh`): the samples land at the branch targets,
and they move when the code around them changes. Removing dispatches helped far more than the
handlers' own work suggested, while touching the resumed process's stacks early changed nothing
(below), so these are branch mispredictions rather than cache misses. On the generated models
the reference-exact inverse-CDF sampling (exact mode) and frame allocation remain the largest
single items.

### Tried and rejected

- Direct calls on every return path (after a SEFF, a provided-role call or a connector ends,
  call the caller's handler instead of returning to `run_process`): -2 to -3 % on h13 and
  gen_s5, +1 % elsewhere. Direct entry from `run_process`-level continuations (closed users,
  scenarios, forked behaviours, loop continuations in `step`): neutral to -3 %.
- Single-variable StoEx (`a.VALUE`) bypassing the evaluator: < 1 %, not kept.
- Processor sharing: skipping divisions by 1.0 and reusing the last time conversion at the same
  instant: no gain.
- Touching the resumed process's stacks at the start of `run_process` (to overlap cache misses):
  no gain.
- `u128` keys in the timer heap as well: slower. Pre-sized process stacks, pre-reserved series
  capacity, and a direct handler call after a cached external-call resolution: no gain.
- mimalloc instead of the system allocator: no difference.

### Evidence

Against a binary built from the starting commit and saved before the first change:
`golden.sh` (312 runs) identical; hashes of trace, tape and measurements of all corpus,
corpus-fuzz, test and bench models in both modes, with and without trace (the benchmarks run the
code without trace instructions; 624 runs), identical; `simoxide-fuzz corpus` 132/132
(66 models, tape replay and own RNG, byte-exact against `corpus/*/expected`) and corpus-fuzz
6/6; 1000 generated models (default features, sizes 1 to 10) and 400 with all optional
features at 0.2 to 0.5, three runs each (run.json with trace, 3000 measurements with trace, the
same without trace), in both modes: all identical, including the same errors for the runs that
abort. `cargo test --release --workspace`: 205 passed, including two new tests in
`tests/interp.rs` (runs with and without trace agree in both modes over all corpus and test
models; the stack-depth limit position); the ignored statistical and scheduler tests pass; fmt
and clippy `-D warnings` with and without `fast`.

# Deviations from the reference

SimOxide follows the patched reference, SimuLizar 5.2.2, including its
[bugs](./reference-bugs.md). This page lists every intentional difference, the reason for it and
the option that controls it. Anything not listed here and not reproduced is a bug.

## Always on

### Resource limits for untrusted models

SimOxide runs inside services on generated or untrusted models, so every input must end in a
result or an error, never in a crash, an endless run or unbounded memory. These checks only
reject inputs that the reference cannot run either: it overflows its stack, runs out of memory
or never finishes. They never change the result of a run that stays within them.

| Check | Where | Default | Reference behaviour |
|---|---|---|---|
| StoEx nesting (parentheses, calls, unary operators) > 200, syntax tree deeper than 1 000 | `simoxide_stoex::parser::{MAX_NESTING, MAX_DEPTH}`: parse error | always | ANTLR/EMF recursion, `StackOverflowError` at some depth |
| Composite component that contains itself | `CompiledModel::compile`: `CompileError` | always | endless recursion |
| More than 1 000 000 nested assembly-context paths | `CompileError` | always | enumerates them |
| `numberOfReplicas` > 100 000 | `CompileError` | always | allocates per replica |
| Continuation stack of one process > `Limits::max_stack_depth` (checked at each SEFF entry) | `SimError` kind `Limit` | 10 000 (about 2 000 nested calls) | deadlocks at a call depth of 300 ([REF-14](./reference-bugs.md#ref-14-deep-recursion-deadlocks-the-simulation)) |
| More than `Limits::max_processes` live processes | `SimError` kind `Limit` | 1 000 000 | one Java thread per process: `OutOfMemoryError: unable to create native thread` far earlier |
| Nested `INNER` proxy evaluations > 2 000 | `SimError` kind `Limit` | always | Java stack overflow |
| Processes run synchronously inside each other after the stop > 200 | `SimError` kind `Limit` | always | – |
| `Limits::max_events`, `max_steps`, `deadline`, `cancel` | `SimError` kind `Limit` / `Cancelled` | off | – |

CLI: `--max-steps`, `--max-events`, `--timeout SECONDS`, `--max-stack-depth`, `--max-processes`.
`Limits::unlimited()` switches off the two defaults of `Limits`.

`Limits::max_stack_depth` counts the reference interpreter's levels, so the limit fires at the
same point whatever the internal representation. `Limits::max_steps` counts handler invocations
of the interpreter; this count is an implementation detail and can change between versions.

### Livelock guard

`SimConfig::max_events_per_instant` (CLI `--max-events-per-instant N`, default 20 000 000, `0` =
off) aborts a run with `livelock: N events at simulation time t s without progress towards a stop
condition` when that many consecutive events happen at one simulated time and no stop condition
comes closer. With `max_measurements` enabled, a finished usage-scenario run counts as progress;
otherwise only advancing time does.

The reference loops forever in this situation, for example in a closed workload with think time
0 and zero demands under a time-only stop ([REF-7](./reference-bugs.md#ref-7-zero-time-livelock)).
A terminating model would need 20 million events at one instant without finishing a scenario
run, so terminating runs are unaffected. The guard stops a livelock after about 2 s and 0.5 GB.

### Deep recursion

SimuLizar deadlocks at a call depth of about 300
([REF-14](./reference-bugs.md#ref-14-deep-recursion-deadlocks-the-simulation)). SimOxide keeps its
call stack on the heap, runs such recursions to depth 100 000 and more, and stops unbounded
recursion with `Limits::max_stack_depth`.

### `Math.pow` rounding

StoEx `^` uses a correctly rounded `pow` (`simoxide_stoex::jmath::pow`). HotSpot's intrinsic is
not correctly rounded in about 0.04 % of random inputs, and there the results differ by 1 ulp.
StoEx `Log` is correctly rounded too and matched HotSpot on every probe. No corpus model uses `^`.
See [Stochastic expressions](../spec/stoex.md).

### `Math.log` and `Math.exp` without `hotspot-math`

The distributions and the special functions of Commons Math call `Math.log` and `Math.exp`.
SimOxide reproduces HotSpot's intrinsics bit for bit with the cargo feature `hotspot-math`
(default in `simoxide-cli` and `simoxide-testkit`, off in the libraries). The port is
GPL-2.0-only, so distributable binaries are built without it (see `LICENSES`). Without it,
SimOxide uses correctly rounded
`log` and `exp` (`simoxide_random::crmath`). HotSpot differs from the correctly rounded result by
one ulp on about 0.25 % of `exp` arguments and on about 1e-5 of `log` arguments (all near 1). A
draw that hits such an argument differs in its last bit, and the run may diverge from the
reference from then on. Statistically the two are equivalent.

### Unpaired UTF-16 surrogates in string literals

`"\uD800"` becomes U+FFFD in Rust strings; Java keeps the lone surrogate. This only affects string
comparisons involving such literals.

### `stop()` of an FCFS resource

The reference `SimFCFSResource.stop()` clears the queue but leaves the pending completion event
scheduled. SimOxide makes that wake-up stale. This only matters after the simulation has stopped
and has no visible effect.

## Opt-in

### Fast mode

`SimConfig::mode = Mode::Fast` (CLI `--mode fast`, cargo feature `fast`) runs the same model
semantics without the work that exists only to reproduce the reference's random numbers. See
[Fast mode](../guide/fast-mode.md) for usage. The policy is a compile-time type
(`simoxide_sim::compat::{Exact, Fast}`), so the choice costs nothing per event.

What changes:

| Mechanism | Exact mode (reference) | Fast mode | Why the results' quality is unaffected |
|---|---|---|---|
| Uniform stream | Commons Math MT19937, two 32-bit outputs per double; tape bookkeeping per draw | xoshiro256++ seeded by SplitMix64, 53-bit doubles; no origin tags, tape or `s` records (tape output and replay are errors) | a generator of at least the same quality (no BigCrush or PractRand failures reported by its authors; uniformity, bit balance and correlation are tested); no semantics depends on the sequence |
| `Norm`, `Lognorm`, `LognormMoments` | Commons Math 2.1 inverse CDF: bracketing in steps of 1.0, then Brent (absolute accuracy 1e-9), 0.5 to 16 µs per sample | ziggurat standard normal; `m + s·z`, `exp(mu + s·z)` | exact algorithms for the same distributions; the reference's result is only accurate to 1e-9 absolute |
| `Gamma`, `GammaMoments` | the same numerical inversion of `regularizedGammaP` | Marsaglia-Tsang (2000), `G(a+1)·U^(1/a)` for `a < 1` | exact algorithm |
| `Exp` | `-mean·Math.log(1-u)` with HotSpot's `log` | ziggurat exponential times the mean | exact algorithm |
| `Pois` | bisection over `[0, 2^31)` on `regularizedGammaQ(k+1, m)` | inversion by sequential search for `m < 10`, PTRS (Hörmann 1993) otherwise; **minus 1** as in the reference ([REF-4](./reference-bugs.md#ref-4-pois-m-is-poisson-1)) | exact algorithms for Poisson(m) − 1, the reference's distribution |
| `UniDouble` | Brent inversion, accuracy 1e-6 | `a + u·(b − a)` | exact; the reference's value is within 1e-6 of it |
| `UniInt` | bisection over the CDF | Lemire's unbiased multiply-shift | the same uniform distribution on `a..=b` |
| Parameter checks and errors | `checkParameters`, distribution constructors | the same code, no draw on an error; NaN or infinite parameters, `UniDouble(a, a)`, an overflowing `UniInt` range and `Pois` means above 1e9 fall back to the reference algorithm | identical error behaviour on all 154 parameter sets of the Java oracle |

Not changed, on purpose:

- all model semantics: workloads, actions, schedulers (PS, FCFS, delay, the 1e-5 s lost-time
  rule, the 1e-9 s demand floor, nanosecond truncation), stop conditions and the post-stop drain,
  measurement definitions, StoEx typing, and every reproduced reference bug;
- StoEx `Log`, `^` and the `LognormMoments` parameter computation keep the reference's
  `Math.log` and `pow`;
- the processor-sharing algorithm (the virtual-time variant below stays a separate opt-in).

Differences that remain, by construction:

- the random numbers themselves, so single runs differ. Statistically the modes are equivalent
  ([Testing](./testing.md#fast-mode));
- `RunResult::uniforms` counts samples (one per distribution call, PMF, PDF or branch), not
  uniforms;
- parameter sets on which the reference's numerical inversion itself fails or runs for hours
  (scales beyond 2^53, where bracketing in steps of 1.0 stops advancing; Brent failures at
  extreme tails) are sampled normally;
- distributions whose scale is below the reference's solver accuracy (for example
  `Norm(0, 1e-9)`, `UniDouble(1e-9, 2e-9)`) are sampled exactly instead of with an absolute
  error of up to 1e-9 (1e-6 for `UniDouble`);
- no random tape and no tape replay.

The engine is shared: with the same random numbers the fast mode gives byte-identical
measurements, end times and request, sample and event counts
(`crates/simoxide-sim/tests/fast_mode.rs`).

Not part of the fast mode because they change results: merging hand-offs while other events are
pending at the same instant (it reorders simultaneous events; in tie-heavy models it changed
passive-resource waiting times and reconfiguration counts systematically) and the virtual-time
processor sharing (below).

### Virtual-time processor sharing

`PsAlgorithm::VirtualTime` (CLI `--ps-algorithm virtual-time`, default `exact`) serves processor
sharing in O(log n) per event instead of O(n). It sums the per-job service in one virtual clock
instead of subtracting it from each job, which changes the floating-point rounding. On the
scheduler oracle scripts, completion times differ by at most 10 ns (125 of 4 406 completions
differ). A difference can flip the reference's 1e-5 s lost-time decision; the property-test
bound is 10 µs. It pays off at hundreds of concurrent jobs (about 5x faster than exact at 300).
See [Schedulers](../spec/scheduler.md).

A job that demands again while it is still served (an early resume,
[SIM-4.4a](../spec/simulation.md)) keeps its position and gets the new demand, as in the exact
algorithm.

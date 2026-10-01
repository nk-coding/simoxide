# Fast mode

The default **exact mode** reproduces the reference's random numbers and events bit for bit. The
opt-in **fast mode** keeps the model semantics and the statistics but draws its random numbers
differently: a faster generator and standard samplers instead of the reference's numerical
inverse CDFs. Its runs are reproducible per seed, but they are not the reference's runs.

```sh
./target/release/simoxide run --model corpus/x_sl_mediastore --mode fast --measurements m.csv
./target/release/simoxide bench --model DIR --runs 20 --mode fast
```

## What differs

Only the random numbers. The engine is the same in both modes: with the same random numbers the
fast mode produces byte-identical measurements, end times and event counts (checked on every
model directory of the repository and on 1 000 generated models).

| | Exact mode | Fast mode |
|---|---|---|
| Uniform generator | Commons Math MT19937, tape bookkeeping per draw | xoshiro256++ seeded by SplitMix64 |
| `Norm`, `Lognorm`, `LognormMoments` | inverse CDF by bracketing and Brent's method | ziggurat |
| `Gamma`, `GammaMoments` | inverse CDF by bracketing and Brent's method | Marsaglia-Tsang |
| `Exp` | `-mean·log(1-u)` with HotSpot's `log` | ziggurat |
| `Pois` | bisection over the CDF | sequential search or PTRS, minus 1 as in the reference |
| `UniDouble`, `UniInt` | inverse CDF | `a + u·(b − a)`, Lemire's method |
| Random tape | written and replayable | not available |

Everything else is unchanged: workloads, actions, schedulers and their rounding rules, stop
conditions, measurement definitions, StoEx typing, parameter checks and error messages, and every
reproduced [reference bug](../correctness/reference-bugs.md). The full list of differences is in
[Deviations](../correctness/deviations.md#fast-mode).

## Is it equivalent?

Statistically, yes. 1 396 models were run with 40 to 100 seeds in each mode, and about 105 000
tests compared counts, means, percentiles and distributions of every measured series. No
difference survived a Holm correction, and the p-values are distributed like those of a control
that compares the exact mode with itself. See [Testing](../correctness/testing.md#fast-mode).

## How much faster?

The gain depends on how much of a model's time goes into sampling distributions:

- models dominated by the engine (mediastore, espresso, forks): about as fast as exact mode;
- models that sample `Norm`, `Gamma`, `Lognorm` or `Pois` often: 2 to 3.4 times faster
  (StoEx distributions benchmark h21: 3.4x);
- generated models, which use distributions heavily: 1.3 to 2.1x on the two benchmark models.

See [Engine performance](../performance/engine.md#fast-mode) for the per-model numbers.

## Using it

- The CLI includes both modes (cargo feature `fast`, on by default for `simoxide-cli`;
  `--no-default-features` builds an exact-only binary).
- The libraries have the feature off by default; enable `simoxide-sim/fast`.
- Library: `SimConfig { mode: Mode::Fast, .. }` with `simoxide_sim::run`, `run_batch` or
  `simulate_memory_mode`, or the typed `Simulation::<simoxide_sim::Fast>::create`.
  `Simulation::new` is exact-only.
- The mode is a compile-time policy (`simoxide_sim::compat`): the simulator is monomorphized per
  mode, and the choice costs nothing per event.
- `RunResult::uniforms` counts samples in fast mode, not uniforms.

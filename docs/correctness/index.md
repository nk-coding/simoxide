# Exactness and evidence

## The claim

For a [supported model](../guide/scope.md), a seed and the stop conditions, SimOxide's exact mode
gives the same results as the patched reference SimuLizar 5.2.2 (`refsim`). "The same" means:

- every measurement tuple is bit-identical: `measurements.csv` is byte-identical;
- the event trace is byte-identical: the same events, in the same order, at the same times;
- the random tape is byte-identical: the same uniforms, drawn at the same places.

Where the reference aborts with an exception, SimOxide aborts with the same error at the same
simulated time, after the same trace.

The reference is SimuLizar 5.2.2 with patches that only remove nondeterminism: a single seeded
random stream, and insertion order where Java iterates in hash order. Each patch and the evidence
that it changes nothing else is on [Patches](../reference-simulator/patches.md). The patches
change no measurement except on models where stock SimuLizar is itself nondeterministic (the
order in which it drains processes after the stop).

## The evidence

| Check | Result |
|---|---|
| Corpus: 66 models (33 hand-made, one feature each; 33 example models from the Palladio repositories), in tape-replay and own-RNG mode, comparing trace, tape and measurements | 66/66 byte-identical in both modes |
| Saved fuzz and regression cases (`corpus-fuzz/`, `crates/simoxide-sim/tests/models*`) | all pass, including the cases where the reference aborts: same error, identical trace prefix |
| Differential fuzzing: about 12 000 generated models, with default features, boosted features and all 15 optional features at once | no open divergence; SimOxide also aborts wherever the reference does |
| Long runs (20 000 measurements, up to 2.2 million tuples; traces up to 1.2 GB in fuzzing) | identical |
| Component oracles: StoEx (8 834 cases), random numbers and distributions (10^6-sample goldens), `Math.log`/`exp` (1.1 million inputs), schedulers (39 traces), XMI loader against EMF (163 model directories) | exact |
| Statistical validation against queueing theory (M/M/1, M/D/1, M/G/1-PS, M/M/c, Jackson, MVA, fork/join) | 10/10 within 99.9 % confidence intervals, in both modes |
| Fast mode against exact mode: 1 396 models, 40 to 100 seeds per mode, about 105 000 tests | no difference after Holm correction |
| `cargo test --release --workspace` (2026-09-30) | 208 passed, 0 failed; the 25 ignored slow and reference tests pass as well |

How each layer works and how to rerun it: [Testing](./testing.md).

## Caveats

- **Bit-exact `Math.log`/`Math.exp` need the feature `hotspot-math`** (default in `simoxide-cli`
  and `simoxide-testkit`, off in the libraries). The port of HotSpot's intrinsics is
  GPL-2.0-only, so binaries built with it must not be distributed. Without it, `log` and `exp`
  are correctly rounded and differ from
  HotSpot by one ulp on rare inputs
  ([Deviations](./deviations.md#math-log-and-math-exp-without-hotspot-math)). The results on this
  page are with the feature.
- **`Math.log` depends on the CPU vendor.** HotSpot's `Math.log` uses the `rcpps` instruction,
  whose result differs between CPU vendors. SimOxide executes the same instruction, so it matches
  a JVM running on the same machine. Samples that go through `log` (`Exp`, `Norm`, `Lognorm`,
  `Gamma`, ...) can differ in the last bits between an Intel and an AMD machine, in Java as well.
  The cargo feature `simoxide-random/rcp-table` uses a recorded AMD Zen 5
  table instead of the instruction.
- **StoEx `^` is correctly rounded.** HotSpot's `pow` intrinsic differs by 1 ulp in about 0.04 %
  of inputs.
- **Intentional deviations** are listed on [Deviations](./deviations.md): resource limits for
  untrusted models, a livelock guard, no deadlock on deep recursion, and the opt-in fast mode and
  virtual-time processor sharing.
- **Reference bugs are reproduced**, on purpose: [Reference bugs](./reference-bugs.md).

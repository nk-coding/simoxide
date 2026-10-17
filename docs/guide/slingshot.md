# Slingshot flavour

By default SimOxide simulates like **SimuLizar 5.2.2**. The **Slingshot flavour** simulates like
the Palladio simulator **Slingshot** (nightly `1.0.0.202609010045`) instead: its event timing, its
resources, its monitors, its random streams. The rules it follows are the
[Slingshot semantics](../spec/slingshot/index.md) (`SS-*`).

```sh
./target/release/simoxide run --model corpus/x_ss_minimal --flavour slingshot \
    --max-sim-time 1000 --max-measurements -1 --measurements m.csv
# reproduce Slingshot's bugs as well (compat flags):
./target/release/simoxide run --model corpus/x_ss_minimal --flavour slingshot --quirks fast ...
./target/release/simoxide run --quirks list          # the flags
```

This is the flavour's **fast mode** (design phase S2): it reproduces the semantics that change
results, not Slingshot's event bus. The **Slingshot-exact mode** (`--flavour slingshot-exact`,
below) reproduces Slingshot's bus cascade and disposed handlers event for event. In the fast mode
both RNG modes work:
`--mode exact` draws StoEx values from the reference's MT19937 stream and branch decisions from
`java.util.Random(seed + 1000003)`, as the [Slingshot runner](../reference-simulator/slingshot.md)
seeds them; `--mode fast` draws everything from the fast generator ([Fast mode](./fast-mode.md)).

## The exact mode

```sh
./target/release/simoxide run --model corpus/x_ss_minimal --flavour slingshot-exact \
    --seed 1 --max-sim-time 110 --measurements m.csv \
    --trace projection.jsonl --tape tape-projection.jsonl
```

`--flavour slingshot-exact` runs Slingshot's event bus handler by handler (design phase X2):
the same events in the same order at the same `double` times as the
[reference runner](../reference-simulator/slingshot.md) in its `det` mode, with every quirk
(`--quirks` must be empty or `all`), including Slingshot's disposed handlers (Q18): a handler
that throws is logged (`warning: t=…: <exception> in <handler>: …`) and never runs again, the
run goes on. It needs `--max-sim-time` (Slingshot's `simTime`, its only stop condition) and
reads the seed; it ignores `--mode` (its random streams are always the reference runner's:
MT19937 for StoEx, `java.util.Random(seed + 1000003)` for branches and link failures) and
`--max-measurements` (Q32). `--trace` writes the level-E event projection and `--tape` the tape
projection of [exact comparison](../spec/slingshot/exact.md#ss-x-2-the-projection), so that both
can be compared byte for byte with the reference's `tools/project.py` output; the measurements
are the reference's `measurements.csv`. It works in the default (EPL) build. It is several times
slower than the fast mode (one step per Slingshot event) and still far faster than Slingshot.
How close it is: [Slingshot flavour correctness](../correctness/slingshot.md#the-exact-mode).
In the library: `CompiledModel::compile_flavour(model, FlavourConfig::slingshot_exact())` and
`simoxide_sim::run` (or `simoxide_sim::ssx::run`).

## What changes against SimuLizar

The genuine design differences of Slingshot, always on in the flavour:

| | SimuLizar flavour | Slingshot flavour |
|---|---|---|
| time | integer nanoseconds, delays truncated | `double` seconds, `t = now + delay` (SS-ENG-2) |
| stop | first event at or after the stop time, then a drain and final samples | `simTime` before any event at `simTime`, no drain, no final samples (SS-RUN-3/4); the maximum measurement count as SimuLizar (fixed Q32) |
| allocation | last matching allocation context | first (SS-ACT-9) |
| resources | SimuLizar's schedulers; nested containers not simulated | PS as SimuCom's with zero demands as 1e-9 s jobs, FCFS serving each job for its demand from the head (SS-RES-2/3); every allocated container, nested or not; HDD rates ignored |
| demands | skipped when 0, measured otherwise | measured also when 0; a demand without allocation or resource aborts the user (closed: restarts after a think time, open: dropped, SS-WL-5) |
| links | depending on the linking-resource options | always simulated, BYTESIZE payloads both ways (SS-ACT-7) |
| passive resources | one unit, created at first use | the action's amount, the release re-evaluated (SS-RES-4); instances created before the start |
| branches | the run's random stream | Slingshot's `Math.random()` stream (SS-RNG-1.2) |
| monitoring | probe framework, sliding windows, PRM, initial and final tuples | instantaneous state and utilisation samples at every resource event, the resource demand at every job start, Slingshot's passive-resource points; no windows, no processing types, no initial or final tuples (SS-MEAS) |

Model changes (`Simulation::apply_changes`) follow Slingshot's syncers with the fixed behaviours
([Model changes](../spec/model-changes.md) MC-9): resources for newly allocated containers, the
first allocation context of an assembly context read live, passive resources of new allocation
contexts, new monitors probed, removed containers drained (the Q5 flag: Slingshot's removal).

## Slingshot bugs: fixed by default, flags to reproduce them

Where Slingshot is buggy, the flavour does by default what SimuLizar does (design decision U13):
asynchronous forks run, delay resources work, the demands of one internal action are served one
after the other, loop count 0 skips the body, response-time monitors are honoured, a throwing
handler is a run error. Each documented quirk `Q1`–`Q37` has a compat flag that reproduces it
(decision U5); the list with the fixed behaviour of each is on [Quirks](../spec/slingshot/quirks.md).

| `--quirks` | Meaning |
|---|---|
| `none` (default) | the fixed behaviour of every quirk |
| `Q26,Q29` | these quirks |
| `fast` | every quirk the fast mode reproduces (all but Q17 and Q18) |
| `fast,-Q32` | `fast` without Q32 |
| `all` | all 37 (refused by the fast mode, which cannot reproduce Q17 and Q18; the exact mode, `--flavour slingshot-exact`, always runs all) |

How far the fast mode reproduces a quirk (`--quirks list` prints it):

- **Full**: as Slingshot.
- **UpToThrow**: the quirk's behaviour up to the exception Slingshot throws; there the fast mode
  stops with a run error. What Slingshot does next (it disposes the handler and runs on, Q18) is
  the exact mode's.
- **Spd**: an SPD quirk (Q1–Q16), read by the SPD interpreter (design phases P1–P3).
- **ExactOnly** (Q17, Q18): refused in the fast mode; the exact mode reproduces them.

## How close is it?

With `--quirks fast` and `--mode exact`, the measurements are byte-identical to the Slingshot
runner's deterministic (`det`) mode on 16 of the 20 [probe models](../spec/slingshot/index.md)
(the other four have Slingshot handler errors) and on the Slingshot corpus models `x_ss_minimal`,
`x_ss_delay`, `x_ss_minimalexample`, `x_ss_minimalexample_open_ps` and `x_ss_mediastore` (seeds 1
to 3). On `x_espresso`, a model full of same-time ties, the fast mode's same-time order differs from
Slingshot's event cascade; it is statistically equal to stock Slingshot. Details:
[Slingshot flavour correctness](../correctness/slingshot.md).

## Library

```rust
use simoxide_sim::{CompiledModel, FlavourConfig, Quirks, SimConfig, Outputs};
let model = simoxide_model::load_dir("corpus/x_ss_minimal")?;
let cm = CompiledModel::compile_flavour(model, FlavourConfig::slingshot(Quirks::all_fast()))?;
let cfg = SimConfig { seed: 1, max_sim_time: 1000, max_measurements: -1, ..Default::default() };
let r = simoxide_sim::run(&cm, cfg, Outputs::default())?;   // dispatches on mode and flavour
```

The typed form is `Simulation::<C, simoxide_sim::Slingshot>::create`; the flavour is a
compile-time parameter like the RNG policy `C`, so the SimuLizar flavour runs exactly the code it
ran before flavours existed. Pause, fork and resume work in both flavours.
`Simulation::observe_series` and `take_observed` give the tuples of chosen series as they are
recorded (Slingshot's `MeasurementMade` stream), the hook for the SPD interpreter.

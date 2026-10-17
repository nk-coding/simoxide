# spd_or_trigger

Report issues: S2.

**Source.** Palladio-Documentation-Slingshot `examples/minimalexample` (commit `9444815`, 2024-09-13): the files its `default.experiments` uses as initial model; `default.slo`, `representations.aird`, `default.dlim`, `default.usageevolution`, `default.experiments` not copied. **Licence pending confirmation** (Palladio example models, EPL-2.0 by convention). Variant made for this report.

**Changes.** `default.spd`: the policy's trigger replaced by a `ComposedTrigger` with `logicalOperator="OR"` of two `SimpleFireOnValue` triggers on `SimulationTime`: `EqualTo` 50.0 and `EqualTo` 70.0.

## Model

One step +1 policy that should fire at t = 50 and at t = 70.

## Run with stock Slingshot

Import the folder into an Eclipse workspace with the Slingshot nightly. Create a *Slingshot Simulation* run configuration:

- Allocation Model: `default.allocation`
- Usage Model: `default.usagemodel`
- Monitor Repository: `default.monitorrepository`
- Scaling Policy Definition: `default.spd`
- SPD Semantic Configuration: `default.semanticspd`
- Simulation tab: maximum simulation time **100 s**; EDP2, an in-memory repository.

The other model files are loaded through the references. Set the console log level to INFO to see the dispatched events and the handler errors (`An error occurred: …`).

The model draws random numbers. Stock Slingshot cannot be seeded, so times and values vary between runs; the effect described below does not depend on them.

## What to look for

- t = 50: the chain is `TargetGroupChecker ok`, `LogicalORCompoundFilter` returns null: no adjustment.
- t = 70: the chain resumes directly at the `Adjustor` (target-group check and trigger skipped): one adjustment.
- Result: one scale-out at t = 70 instead of two (t = 50 and t = 70). See `observed/spd.jsonl`, records `"k":"eval"`.

## Expected

OR fires when at least one child fires: adjustments at t = 50 and t = 70.

## Observed (reference runner)

`observed/` holds the outputs of the SimOxide reference runner, deterministic mode, `-seed 1 -simTime 100` (appendix of `../../REPORT.md`): `errors.txt`, `measurements.csv`, `projection-m.jsonl`, `spd.jsonl`, `summary.json`.
`errors.txt` lists the handler exceptions (`exception: message | first Slingshot frame`), one per disposed handler; `measurements.csv` the EDP2 rows (`measuring_point,metric,time,value`); `projection*.jsonl` the dispatched model-level events (users, calls, jobs, measurements, SPD events; `projection-m.jsonl` without passed elements and jobs).

Headless (optional, from `reference/slingshot/`):

```sh
./run.sh --stock -model upstream-report/reproductions/spd_or_trigger -out /tmp/spd_or_trigger -simTime 100 -seed 1
./run.sh         -model upstream-report/reproductions/spd_or_trigger -out /tmp/spd_or_trigger-det -simTime 100 -seed 1   # deterministic
```

# spd_minimal

Report issues: I2 (and the baseline for S1, S2).

**Source.** Palladio-Documentation-Slingshot `examples/minimalexample` (commit `9444815`, 2024-09-13): the files its `default.experiments` uses as initial model; `default.slo`, `representations.aird`, `default.dlim`, `default.usageevolution`, `default.experiments` not copied. **Licence pending confirmation** (Palladio example models, EPL-2.0 by convention).

## Model

The Slingshot SPD minimal example: one elastic infrastructure, a step +1 policy at simulation time 50 (`EqualTo`).

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

- Install Slingshot and the SPD interpreter from the nightly update sites only (no `org.palladiosimulator.architecturaltemplates.catalog`). The adjustment at t = 50 fails: QVTo cannot resolve the `ProfilesLibrary` blackbox imported by `pcm/helpers/Constructors.qvto`, `Wiring.qvto`, `Loadbalancing.qvto`; `ModelAdjusted(false)`.
- With a bundle that registers the blackbox (the runner's stub; presumably also a full Palladio product with Architectural Templates) the adjustment succeeds: `AppServer_1` appears at t = 50 (`observed/spd.jsonl`, `projection-m.jsonl`).

## Expected

SPD adjustments work in an install made from the Slingshot and SPD update sites.

## Observed (reference runner)

`observed/` holds the outputs of the SimOxide reference runner, deterministic mode, `-seed 1 -simTime 100` (appendix of `../../REPORT.md`): `errors.txt`, `measurements.csv`, `projection-m.jsonl`, `spd.jsonl`, `summary.json`.
`errors.txt` lists the handler exceptions (`exception: message | first Slingshot frame`), one per disposed handler; `measurements.csv` the EDP2 rows (`measuring_point,metric,time,value`); `projection*.jsonl` the dispatched model-level events (users, calls, jobs, measurements, SPD events; `projection-m.jsonl` without passed elements and jobs).

Headless (optional, from `reference/slingshot/`):

```sh
./run.sh --stock -model upstream-report/reproductions/spd_minimal -out /tmp/spd_minimal -simTime 100 -seed 1
./run.sh         -model upstream-report/reproductions/spd_minimal -out /tmp/spd_minimal-det -simTime 100 -seed 1   # deterministic
```

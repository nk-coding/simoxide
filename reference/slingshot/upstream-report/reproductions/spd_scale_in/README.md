# spd_scale_in

Report issues: S1, S3.

**Source.** Palladio-Documentation-Slingshot `examples/minimalexample` (commit `9444815`, 2024-09-13): the files its `default.experiments` uses as initial model; `default.slo`, `representations.aird`, `default.dlim`, `default.usageevolution`, `default.experiments` not copied. **Licence pending confirmation** (Palladio example models, EPL-2.0 by convention). Variant made by SimOxide (S0 probe).

**Changes.** `default.spd`: a second policy "Scale in at 80" (`StepAdjustment -1`, simulation time `EqualTo` 80); `default.repository`: the accounting demand raised from 2.0 to 9.0, so that several jobs are on `AppServer_1` when it is removed.

## Model

Scale-out at t = 50 (`AppServer_1` added), scale-in at t = 80 (`AppServer_1` removed) while four jobs run on it.

## Run with stock Slingshot

Import the folder into an Eclipse workspace with the Slingshot nightly. Create a *Slingshot Simulation* run configuration:

- Allocation Model: `default.allocation`
- Usage Model: `default.usagemodel`
- Monitor Repository: `default.monitorrepository`
- Scaling Policy Definition: `default.spd`
- SPD Semantic Configuration: `default.semanticspd`
- Simulation tab: maximum simulation time **150 s**; EDP2, an in-memory repository.

The other model files are loaded through the references. Set the console log level to INFO to see the dispatched events and the handler errors (`An error occurred: …`).

The model draws random numbers. Stock Slingshot cannot be seeded, so times and values vary between runs; the effect described below does not depend on them.

## What to look for

- At t = 80 `AppServer_1` is removed with four jobs on its CPU.
- The job that holds the pending `JobProgressed` is aborted when that event comes (t = 87): `JobAborted`, `ResourceDemandRequestAborted`, `UserAborted`; its open-workload user is dropped.
- The other three jobs (started at t = 69, 72, 78) never produce another event; their users never finish and record no response time.
- The t = 80 policy also evaluates the t = 50 `SimulationTimeReached` of the other policy and is rejected only by its `SimulationTimeChecker` (`observed/spd.jsonl`, `eval` records).

## Expected

Jobs on a removed container finish (drain) or are aborted with an event each; each policy evaluates only its own time event.

## Observed (reference runner)

`observed/` holds the outputs of the SimOxide reference runner, deterministic mode, `-seed 1 -simTime 150` (appendix of `../../REPORT.md`): `errors.txt`, `measurements.csv`, `projection-m.jsonl`, `spd.jsonl`, `summary.json`.
`errors.txt` lists the handler exceptions (`exception: message | first Slingshot frame`), one per disposed handler; `measurements.csv` the EDP2 rows (`measuring_point,metric,time,value`); `projection*.jsonl` the dispatched model-level events (users, calls, jobs, measurements, SPD events; `projection-m.jsonl` without passed elements and jobs).

Headless (optional, from `reference/slingshot/`):

```sh
./run.sh --stock -model upstream-report/reproductions/spd_scale_in -out /tmp/spd_scale_in -simTime 150 -seed 1
./run.sh         -model upstream-report/reproductions/spd_scale_in -out /tmp/spd_scale_in-det -simTime 150 -seed 1   # deterministic
```

# x_pem_acquire

Report issues: R5.

**Source.** Palladio-Example-Models `Acquire_Example` (commit `4a8dc45`, 2026-01-26); `refsim.monitorrepository` and `refsim.measuringpoint` added by SimOxide. No licence declared upstream. The model files are links into the git submodule `third_party/palladio-example-models` (no licence declared upstream, so not distributed here; `git submodule update --init`, `third_party/README.md`).

## Model

A passive resource whose capacity is the component parameter `lockCount.VALUE`.

## Run with stock Slingshot

Import the folder into an Eclipse workspace with the Slingshot nightly. Create a *Slingshot Simulation* run configuration:

- Allocation Model: `AllocationExample.allocation`
- Usage Model: `AquireExample.usagemodel`
- Monitor Repository: `refsim.monitorrepository`
- Simulation tab: maximum simulation time **500 s**; EDP2, an in-memory repository.

The other model files are loaded through the references. Set the console log level to INFO to see the dispatched events and the handler errors (`An error occurred: …`).

The model draws no random number.

## What to look for

- The run fails before t = 0: `RuntimeException: Architecture specification incomplete. Stackframe is missing id lockCount.VALUE` from `PassiveResourceTable` (`PassiveResourceTable.java:59`), inside the Guice construction of `ResourceSimulation`.
- With the reference runner the process exit status is still 0.

## Expected

The capacity is evaluated in the frame of the component instance (component and assembly parameters visible), as in SimuCom/SimuLizar.

## Observed (reference runner)

`observed/` holds the outputs of the SimOxide reference runner, deterministic mode, `-seed 1 -simTime 500` (appendix of `../../REPORT.md`): `errors.txt`.
`errors.txt` lists the handler exceptions (`exception: message | first Slingshot frame`), one per disposed handler; `measurements.csv` the EDP2 rows (`measuring_point,metric,time,value`); `projection*.jsonl` the dispatched model-level events (users, calls, jobs, measurements, SPD events; `projection-m.jsonl` without passed elements and jobs).

Headless (optional, from `reference/slingshot/`):

```sh
./run.sh --stock -model upstream-report/reproductions/x_pem_acquire -out /tmp/x_pem_acquire -simTime 500
./run.sh         -model upstream-report/reproductions/x_pem_acquire -out /tmp/x_pem_acquire-det -simTime 500 -seed 1   # deterministic
```

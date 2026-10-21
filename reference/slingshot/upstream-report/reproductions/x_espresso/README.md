# x_espresso

Report issues: D2.

**Source.** Espresso example model (the model of Palladio-Documentation-Slingshot `examples/minimalexample` without SPD), copied from the Palladio research work directory (`work-simucom/models/espresso`). `refsim.monitorrepository` and `refsim.measuringpoint` were added by SimOxide (default monitors). The model is the espresso example of Palladio-Addons-ExperimentAutomation (`bundles/org.palladiosimulator.experimentautomation.examples.espresso/model`, same ids), EPL-2.0.

## Model

30 closed users without think time, constant demands, one processor-sharing CPU. The model draws no random number.

## Run with stock Slingshot

Import the folder into an Eclipse workspace with the Slingshot nightly. Create a *Slingshot Simulation* run configuration:

- Allocation Model: `default.allocation`
- Usage Model: `default.usagemodel`
- Monitor Repository: `refsim.monitorrepository`
- Simulation tab: maximum simulation time **100 s**; EDP2, an in-memory repository.

The other model files are loaded through the references. Set the console log level to INFO to see the dispatched events and the handler errors (`An error occurred: …`).

The model draws no random number.

## What to look for

- Run it three times. The `State of Active Resource Tuple` series differs between runs although the model is fully deterministic.
- Observed: mean state 21.12 in the runner's insertion-ordered mode, 22.74 and 22.73 in two stock runs of one JVM (3016 rows each; three stock runs, three different files). Response times and demands are equal.

## Expected

Identical results for identical inputs.

## Observed (reference runner)

`observed/` holds the outputs of the SimOxide reference runner, deterministic mode, `-seed 1 -simTime 100` (appendix of `../../REPORT.md`): .
`errors.txt` lists the handler exceptions (`exception: message | first Slingshot frame`), one per disposed handler; `measurements.csv` the EDP2 rows (`measuring_point,metric,time,value`); `projection*.jsonl` the dispatched model-level events (users, calls, jobs, measurements, SPD events; `projection-m.jsonl` without passed elements and jobs).

Headless (optional, from `reference/slingshot/`):

```sh
./run.sh --stock -model upstream-report/reproductions/x_espresso -out /tmp/x_espresso -simTime 100
./run.sh         -model upstream-report/reproductions/x_espresso -out /tmp/x_espresso-det -simTime 100 -seed 1   # deterministic
```

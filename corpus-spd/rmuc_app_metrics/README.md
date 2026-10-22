# RMUC App-Metrics (ql5-rt500)

Origin: Remote Measuring Use Case (RMUC) mosaic demonstrator, Slingshot SPD
`decentralized-app-metrics-5-500` (DeviceCommunication response time 0.5 s,
DataProcessing queue length 5). Flattened so simoxide loads one `.spd` and one
`.semanticspd` from the model root (`run.json` points at `monitors/`).

Licence: Palladio model artefacts, distributed here under the Eclipse Public License 2.0
with the rest of this repository.

`--spd-quirks slingshot` reproduces Slingshot (Q2 commits cooldown when the request is
made; Q9 does not check group size), so a min-size scale-in can block the matching
scale-out. `--spd-quirks fixed` disregards that no-op and both groups scale out together
on the first spike.

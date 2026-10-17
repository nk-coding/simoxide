# corpus-rules: SimuLizar's reconfiguration examples

Models with SimuLizar reconfiguration rules (QVTo) and the outputs of the patched SimuLizar 5.2.2
reference (`reference/refsim`, patch P9) in `<case>/expected/`. Each case has its models, its rules
in `rules/` and a `run.json` with `"reconfiguration_rules": "rules"`. Regenerate with
`reference/import-rules-examples.sh` (models and rules) and `CORPUS=../corpus-rules
reference/regen-expected.sh` (expected outputs). Guide: `docs/guide/reconfiguration.md`.

Sources: SimuLizar `releases/5.2.2` (commit 23dc438, EPL-2.0): `bundles/ExampleModels` and the test
models of `org.palladiosimulator.simulizar.tests`. `scaleOut.qvto` (`lb_scaleout`) and `addBranch.qvto`
(`lb_add_branch`) are SimOxide's (`reference/rules-examples/`).

| Case | Source | Rules | Stop |
|---|---|---|---|
| `lb_outsource` | `examples.loadbalancer` (its launch configuration) | `reconfigurations_active/outsource.qvto` | 1000 s |
| `lb_rules_qvto` | `examples.loadbalancer` | `reconfigurations_qvto/*.qvto` | 1000 s |
| `lb_scaleout` | `examples.loadbalancer` | `outsource.qvto`, SimOxide's `scaleOut.qvto` | 1000 s |
| `lb_add_branch` | `examples.loadbalancer` | SimOxide's `addBranch.qvto` | 1000 s |
| `power_rules_qvto` | `TransientEffects/examples.loadbalancer.powerconsumption`, power monitors removed, resource-URI measuring points replaced by the PCM ones they denote | `reconf_qvto/{scaleDown,scaleIn,scaleUp}.qvto` | 2000 s |
| `sl_reconfiguration` | `testmodels/reconfiguration` (`ReconfigurationSimulationTest`) | `reconfigs/adapt.qvto` | 10 measurements |

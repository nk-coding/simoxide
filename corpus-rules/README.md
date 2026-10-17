# corpus-rules: SimuLizar's reconfiguration examples

Models with SimuLizar reconfiguration rules (QVTo) and the outputs of the patched SimuLizar 5.2.2
reference (`reference/refsim`, patches P9 and P10) in `<case>/expected/`. Each case has its models, its rules
in `rules/` and a `run.json` with `"reconfiguration_rules": "rules"`. Regenerate with
`reference/import-rules-examples.sh` (models and rules) and `CORPUS=../corpus-rules
reference/regen-expected.sh` (expected outputs). Guide: `docs/guide/reconfiguration.md`.

Sources: SimuLizar `releases/5.2.2` (commit 23dc438, EPL-2.0): `bundles/ExampleModels` and the test
models of `org.palladiosimulator.simulizar.tests`. The rules in `reference/rules-examples/` are
SimOxide's; `wireNewServer.qvto` adapts two adaptation steps of SimuLizar's action repository
(`instantiatecomponent.qvto`, `wirewithloadbalancer.qvto`) to a rule (its header lists the changes).

The R2 cases (DESIGN-SPD R2) create objects without explicit ids: their ids come from the seeded
generator that the reference (P10) and SimOxide share, which draws only while a rule runs.
`lb_scaleout_each` depends on the iteration order of a `Set` of model objects: insertion order in the
reference (P10) and in QVTo's ordered mode; QVTo's compat mode (HotSpot identity hashes) moves the
servers in the other order and differs. All eleven cases equal the reference byte for byte in the
exact build (`crates/simoxide-reconf/tests/rules_process.rs`, also continued from snapshots in
`tests/snapshot.rs`).

| Case | Source | Rules | Stop |
|---|---|---|---|
| `lb_outsource` | `examples.loadbalancer` (its launch configuration) | `reconfigurations_active/outsource.qvto` | 1000 s |
| `lb_rules_qvto` | `examples.loadbalancer` | `reconfigurations_qvto/*.qvto` | 1000 s |
| `lb_scaleout` | `examples.loadbalancer` | `outsource.qvto`, SimOxide's `scaleOut.qvto` | 1000 s |
| `lb_add_branch` | `examples.loadbalancer` | SimOxide's `addBranch.qvto` | 1000 s |
| `power_rules_qvto` | `TransientEffects/examples.loadbalancer.powerconsumption`, power monitors removed, resource-URI measuring points replaced by the PCM ones they denote | `reconf_qvto/{scaleDown,scaleIn,scaleUp}.qvto` | 2000 s |
| `lb_scaleout_noid` | `examples.loadbalancer` | `outsource.qvto`, SimOxide's `scaleOutNoId.qvto` (`scaleOut.qvto` without ids) | 1000 s |
| `lb_add_branch_noid` | `examples.loadbalancer` | SimOxide's `addBranchNoId.qvto` (`addBranch.qvto` without ids) | 1000 s |
| `lb_scaleout_each` | `examples.loadbalancer` | `outsource.qvto`, SimOxide's `scaleOutEach.qvto` (every server to a new node, in `Set` order) | 1000 s |
| `lb_wire_new_server` | `examples.loadbalancer` | `wireNewServer.qvto` (SimuLizar's `instantiatecomponent` and `wirewithloadbalancer` steps) | 1000 s |
| `lb_clone_branch` | `examples.loadbalancer` | SimOxide's `cloneBranch.qvto` (a branch behaviour copied with `deepclone()`) | 1000 s |
| `sl_reconfiguration` | `testmodels/reconfiguration` (`ReconfigurationSimulationTest`) | `reconfigs/adapt.qvto` | 10 measurements |

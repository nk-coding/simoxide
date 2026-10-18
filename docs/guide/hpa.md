# Autoscaling (HPA)

SimOxide can scale a model's services the way Kubernetes' horizontal pod autoscaler does. An
autoscaler watches the CPU utilisation of a target group's replicas and sets their number with
Kubernetes' proportional rule, tolerance, stabilisation windows, scaling policies and
`[minReplicas, maxReplicas]`. The replicas it adds or removes are real model changes:
Slingshot's SPD adjustment (`MainTransformation.qvto`) clones the unit container, its
assemblies, allocation, connectors, load-balancer branch and monitors, or removes the newest
ones, which drain. The semantics are on [HPA autoscaling](../spec/hpa.md).

## What the model needs

- An SPD with the **target group** to scale: an elastic infrastructure (`.spd`), and its
  semantic configuration (`.semanticspd`) with the group's elements, its linking resource and,
  for routing, the service group's load-balancing assembly. This is the structure Slingshot's
  adjustments need ([Scaling policies](./spd.md)). The SPD needs no policies.
- The scaled container's assemblies are reached through a **load balancer**: a component whose
  SEFF has a probabilistic branch with one external call per replica. The adjustment adds
  branches with probability `1/n`.
- An **`.hpa.json`** (or `.hpa`) file next to the model, or `--hpa FILE`.
- No monitor: the autoscaler reads the CPUs' busy times from the engine.

## The configuration file

The file holds Kubernetes `HorizontalPodAutoscaler` objects (`autoscaling/v2`), the controller's
settings (the `kube-controller-manager` flags, which Kubernetes sets cluster-wide) and SimOxide's
additions per autoscaler:

```json
{
  "controller": {
    "syncPeriod": "15s", "tolerance": 0.1, "downscaleStabilization": "5m",
    "cpuInitializationPeriod": "5m", "maxCreatedPods": 20000
  },
  "items": [{
    "apiVersion": "autoscaling/v2", "kind": "HorizontalPodAutoscaler",
    "metadata": {"name": "catalog"},
    "spec": {
      "scaleTargetRef": {"kind": "ElasticInfrastructure", "name": "catalog"},
      "minReplicas": 1, "maxReplicas": 10,
      "metrics": [{"type": "Resource", "resource": {"name": "cpu",
                   "target": {"type": "Utilization", "averageUtilization": 70}}}],
      "behavior": {
        "scaleDown": {"stabilizationWindowSeconds": 120,
                      "policies": [{"type": "Pods", "value": 1, "periodSeconds": 60}]}
      }
    },
    "simoxide": {"startupDelay": "20s"}
  }]
}
```

A file can also hold one `HorizontalPodAutoscaler` or a JSON array of them. All `*.hpa` and
`*.hpa.json` files of the model directory are read, in name order. Durations are seconds
(numbers) or Go duration strings (`"90s"`, `"1m30s"`).

| Field | Meaning (default) |
|---|---|
| `controller.syncPeriod` | how often each autoscaler is reconciled (15 s) |
| `controller.tolerance` | no rescale while `|ratio - 1|` is at most this (0.1) |
| `controller.downscaleStabilization` | the scale-down window without `behavior`, and its default with one (5 min) |
| `controller.cpuInitializationPeriod` | after a pod's start, metrics from before its readiness are set aside (5 min) |
| `controller.maxCreatedPods` | the run stops beyond this many created pods, all autoscalers together (20 000; [Bounds](../spec/hpa.md#hpa-8-bounds)) |
| `spec.scaleTargetRef.name` | the SPD target group's `entityName` or id; `kind` (optional): `ElasticInfrastructure` or `ServiceGroup` |
| `spec.minReplicas`, `spec.maxReplicas` | bounds (1, required) |
| `spec.metrics` | one CPU `Utilization` target (`averageUtilization` 80) |
| `spec.behavior` | `scaleUp`/`scaleDown`: `stabilizationWindowSeconds`, `selectPolicy` (`Max`, `Min`, `Disabled`), `policies` (`Pods`/`Percent`, `value`, `periodSeconds`), `tolerance`; missing fields take Kubernetes' defaults |
| `simoxide.startupDelay` | from a pod's creation until it is ready and takes requests (0) |
| `simoxide.metricWindow` | the CPU metric's window, ending at the sync (the sync period) |
| `simoxide.firstSync` | time of the first sync (one sync period) |
| `simoxide.cpuRequest` | the request per pod in cores (the pod's CPU cores) |

Unknown fields are errors. Without `behavior` the autoscaler follows the controller's legacy
path, as Kubernetes does when the HPA has none. That means a scale-up of at most
`max(2·current, 4)`, and the stabilisation window in both directions.

## Command line

```text
simoxide run --model DIR --max-sim-time 3600 --spd-log decisions.jsonl
```

The flags need the `spd` feature (default). The adjustments need the `reconfiguration` feature;
without it the decisions are made but nothing changes.

| Flag | Meaning |
|---|---|
| `--hpa FILE` | this configuration instead of the directory's files |
| `--no-hpa` | no autoscaler |
| `--spd-log FILE` | the decision log, with `hpa` and `hpa_ready` records (below) |
| `--flavour simulizar\|slingshot` | both run the HPA; `slingshot-exact` refuses it |
| `resume --hpa-set NAME.PARAMETER=VALUE` | change a parameter at the resume (also `hpa NAME PARAMETER VALUE` lines in a change file): `minReplicas`, `maxReplicas`, `averageUtilization`, `tolerance`, `startupDelay`, `scaleUpStabilization`, `scaleDownStabilization`, `downscaleStabilization` (durations in seconds) |

After the run the CLI prints one line per autoscaler:

```text
hpa catalog: syncs=240 rescales=7 adjusted=9 failed=0 created=12 replicas=2 starting=0 max=9 replica-seconds=14250.0 ready-replica-seconds=13980.0
```

`replica-seconds` counts ready and starting pods; `ready-replica-seconds` counts only the ready
ones.

Pauses, snapshots and `resume` work as for any SPD run ([Snapshots](./snapshots.md)): the snapshot
gets the section `hpa`, and the resume needs the same configuration. A time-varying load is a
[load profile](./load-profile.md) (`--load-profile`, or `SimConfig::load_profile` in the
library): the load steps run inside the simulation. Pauses with change files (`arrival-rate
WORKLOAD RATE`), or the library's `run_until` plus `ChangeFile::apply`, work too.

## Decision log

`hpa` records, one per sync:

```json
{"k":"hpa","t":315.0,"hpa":"app","metric":"cpu","current":2,"ready":2,"starting":0,"unready":0,"missing":0,"utilization":99,"target":50,"ratio":1.98,"recommended":4,"stabilized":4,"desired":4,"reason":"DesiredWithinRange","started":2,"cancelled":0,"goal":4}
```

| Field | Meaning |
|---|---|
| `current`, `ready`, `starting` | replicas: ready plus starting pods |
| `unready`, `missing` | pods the calculator set aside |
| `utilization`, `target`, `ratio` | the ready pods' utilisation (percent of their requests), the target, their ratio |
| `recommended` | the calculator's count (HPA-3) |
| `stabilized` | after the stabilisation window (HPA-5.1) |
| `desired`, `reason` | after the rate limits and bounds, with Kubernetes' condition reason (`DesiredWithinRange`, `ScaleUpLimit`, `TooManyReplicas`, `TooFewReplicas`, `ScaleDownLimit`, `AboveMax`, `BelowMin`, `FailedGetResourceMetric` with `error`) |
| `started`, `cancelled`, `goal` | what the rescale did: pods started, starting pods deleted, the group's new ready count if an adjustment runs now |

There is also one `hpa_ready` record when starting pods become ready (`ready`, `goal`). The
adjustments are `adjust` records of the autoscaler's policy `_simoxide_hpa_<name>`, with
Slingshot's change count and the model summaries.

## Library

```rust
use simoxide_reconf::{QvtoAdjuster, QvtoCore, SpdAdjust};
use simoxide_sim::{CompiledModel, Fast, Outputs, RunSpec, Simulation};
use simoxide_spd::hpa::{self, Hpa, HpaConfig};
use simoxide_spd::{Quirks, log::VecLog, sim::{SpdRun, spd_of}};
let spec = RunSpec::default();
let mut model = spec.load_model(dir)?;
let cfg = HpaConfig::load_dir(dir)?.expect("an .hpa.json");
hpa::attach(&mut model, &cfg)?;               // the autoscalers' policies, before compiling
let spd = spd_of(&model, Quirks::fixed())?.expect("an SPD");
let hpa = Hpa::new(&cfg, &spd)?;
let cm = CompiledModel::compile(model)?;
let sim = Simulation::<Fast>::create(&cm, spec.sim_config("run"), Outputs::default())?;
let mut run = SpdRun::new(sim, &spd)?.with_hpa(&hpa)?;
let core = QvtoCore::new()?;
let mut adjuster = QvtoAdjuster::spd(&core, &SpdAdjust::new(&Quirks::fixed()))?;
let mut log = VecLog::default();
run.run(&mut adjuster, &mut log)?;
for s in run.hpa_summary() { println!("{}", s.line()); }
```

- `HpaConfig::from_json` reads a configuration from text, for models built in memory
  (`RunSpec::load_model_memory`).
- `SpdRun::hpa_state` returns the controller's state.
- `SpdRun::hpa_set` changes a parameter.
- `SpdRun::restore_snapshot_hpa` restores a run with autoscalers.
- `simoxide_spd::hpa::controller` is the arithmetic alone (`reconcile`, `resource_replicas`).

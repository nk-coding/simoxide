# Load profiles

A load profile makes a run's load vary over time: per usage scenario, a step function of a
factor. From a point's time on, an open workload arrives `factor` times as often, and a closed
workload has `population · factor` users (rounded), until the next point. The semantics are on
[Workloads](../spec/workloads.md#lp-load-profiles) (rules `LP-*`).

Load profiles are SimOxide's own: SimuLizar and Slingshot have no time-varying load of this kind
(Palladio's usage evolution, DLIM, is not supported). A run without a profile is unchanged, and
a profile of factor 1 changes no output. Typical uses: a daily curve, a load step for an
autoscaler ([Autoscaling (HPA)](./hpa.md)), a ramp to find the saturation point.

## The file

```json
{
  "profiles": [
    {"points": [[0, 1.0], [600, 4.0], [1200, 1.0]]},
    {"scenarios": ["login", "_us7"], "points": [[0, 1.0]]}
  ]
}
```

| Field | Meaning |
|---|---|
| `profiles` | step functions in order; a later one overrides an earlier one for the scenarios it names |
| `scenarios` | the usage scenarios, by id or entity name: a list, one name, or `"*"` (the default: every scenario) |
| `points` | `[time, factor]` pairs: time in seconds (strictly increasing, not negative), factor finite and positive; before the first point the factor is 1 |

`{"points": [...]}` alone is one profile for every scenario. Unknown fields are errors, and so
is a name that matches no scenario (or several by name). The model directory's
`*.loadprofile.json` files are read with the model, in name order, as one profile.

## Command line

```text
simoxide run --model DIR --max-sim-time 1800 --load-profile day.loadprofile.json
```

| Flag | Meaning |
|---|---|
| `--load-profile FILE` | this profile instead of the directory's `*.loadprofile.json` files |
| `--no-load-profile` | no profile |

`--pause-at`, `--snapshot` and `resume` work as for any run: the profile and its state are part of
the snapshot ([Snapshot format](../formats/snapshot.md)), and the resumed run continues it. The
Slingshot-exact engine (`--flavour slingshot-exact`) refuses a profile; the SimuLizar and
Slingshot flavours run it, in both modes.

## Library

```rust
use simoxide_sim::{LoadProfile, SimConfig};
let mut cfg = spec.sim_config("run");
cfg.load_profile = Some(std::sync::Arc::new(LoadProfile::uniform(vec![
    (0.0, 1.0), (600.0, 4.0), (1200.0, 1.0),
])));
let r = simoxide_sim::run(&cm, cfg, Outputs::default())?;
```

- `LoadProfile::from_json`, `load_file` and `load_dir` read the format above; `uniform` builds
  one step function for every scenario.
- `Simulation::create` resolves the profile against the compiled model; an unknown scenario is a
  model error at t = 0.
- `RunState::load_factor(scenario)` is a scenario's factor now (`None` without a profile).
- With scaling policies or HPA autoscalers (`SpdRun`), the profile runs inside the simulation:
  no pauses or change files are needed for the load.

## How a step changes the load

- **Open workloads.** Each inter-arrival time the model draws is divided by the factor. At a
  step, the pending arrival moves so that the rest of its interval passes at the new rate: a
  remaining 50 s at factor 1 become 5 s at factor 10. The arrival process thus runs in
  operational time `∫ factor dt`, a time change of the model's renewal process, and needs no
  extra random number. A constant factor `f` on `Exp(λ)` is exactly `Exp(f·λ)`.
- **Closed workloads.** The population becomes `round(population · factor)`. New users start
  at the step (without a think time, as at the start); surplus users leave after their current
  run and think time, oldest first, as with a population change of the model.
- A model change at a pause (a change file's `arrival-rate`, `population`) changes the model's
  workload; the profile keeps scaling the changed one.

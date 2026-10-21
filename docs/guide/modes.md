# Modes and features

SimOxide simulates a Palladio (PCM) model the way one of two Java simulators does, and each of
them either exactly or fast. This page shows the combinations, which build has them, and which
features work in which mode.

## Two choices per run

**Whose semantics** (`--flavour`):

- **SimuLizar-compatible** (`simulizar`, the default): SimuLizar 5.2.2 with DESMO-J, its
  schedulers and its measurements ([Semantics](../spec/)).
- **Slingshot-compatible**: the event-driven simulator Slingshot (nightly
  `1.0.0.202609010045`), in two variants: `slingshot` reproduces Slingshot's semantics and fixes
  its bugs unless asked otherwise; `slingshot-exact` replays Slingshot's event bus handler by
  handler, bugs included ([Slingshot-compatible mode](./slingshot.md),
  [Semantics of Slingshot](../spec/slingshot/)).

**Which random numbers** (`--mode`, for `simulizar` and `slingshot`):

- **fast** (the default): the same semantics and statistics, a faster generator and samplers;
  reproducible per seed, but not the reference's draws ([Fast mode](./fast-mode.md)).
- **exact**: the reference's random streams, so that a run equals the reference's run byte for
  byte. Only in the exact build (below). `slingshot-exact` always uses the Slingshot reference
  runner's streams and ignores `--mode`.

| Mode | Command line | Equal to | Build |
|---|---|---|---|
| SimuLizar, exact | `--mode exact` | the patched, deterministic SimuLizar 5.2.2 (`refsim`): byte-identical trace, random tape and `measurements.csv` ([Exactness](../correctness/)) | exact build |
| SimuLizar, fast | (default) | SimuLizar 5.2.2 statistically; same events as exact mode given the same random numbers | every build |
| Slingshot | `--flavour slingshot` | stock Slingshot statistically; with `--mode exact --quirks fast`, the deterministic Slingshot runner byte for byte on most models ([Slingshot flavour](../correctness/slingshot.md)) | every build (`--mode exact`: exact build) |
| Slingshot-exact | `--flavour slingshot-exact` | the deterministic Slingshot runner (`det`) byte for byte: event projection, random tape, measurements, SPD decision log ([Slingshot flavour](../correctness/slingshot.md#the-exact-mode)) | every build |

The references are the Java simulators with patches that only make them deterministic
([refsim](../reference-simulator/refsim.md),
[Slingshot runner](../reference-simulator/slingshot.md)).

## Builds

| Build | Command | Adds | May be distributed |
|---|---|---|---|
| EPL (default) | `cargo build --release` | fast mode in both flavours, `slingshot-exact`, scaling policies and HPA (decisions only), load profiles, snapshots | yes |
| EPL with model changes | `cargo build --release -p simoxide-cli --features reconfiguration` | the QVTo engine: SimuLizar's reconfiguration rules, SPD and HPA adjustments, `resume --change/--transform`, `explore` | yes |
| exact | `cargo build-exact`, or `cargo build --release -p simoxide-cli --features exact,hotspot-math` (add `reconfiguration` for rules and adjustments) | `--mode exact`; QVTo's compat mode | **no**: for your own use ([Licensing](./licensing.md)) |

Without `reconfiguration`, a run with scaling policies or HPA autoscalers makes the decisions and
logs them, but executes no adjustment (`ok: false` in the decision log); `--rules`, `--change`,
`--transform` and `explore` are errors. A mode a build does not contain is an error, never a
silent fallback.

## Feature matrix

"exact" and "fast" are the SimuLizar flavour's modes; "Slingshot" is `--flavour slingshot` in
either mode.

| Feature | SimuLizar exact | SimuLizar fast | Slingshot | Slingshot-exact |
|---|---|---|---|---|
| [Supported models](./scope.md) | SimuLizar's static performance simulation | same | same, with Slingshot's differences (nested containers simulated, HDD rates ignored, ...) | same as Slingshot |
| Stop conditions | sim time, measurement count | same | sim time, measurement count (unless Q32) | sim time only (`--max-sim-time` required) |
| `--trace` | event trace | event trace | event trace | level-E event projection |
| `--tape`, `--replay-tape` | yes | no | `--mode exact` only | tape projection; no replay |
| Slingshot compat flags (`--quirks`) | – | – | opt-in, all but Q17, Q18 | always all |
| [Scaling policies (SPD)](./spd.md) | yes (no reference for this combination) | yes | yes (statistically equal to Slingshot with its quirks) | yes, byte-equal to the runner |
| [HPA autoscaling](./hpa.md) | yes | yes | yes | no |
| [Load profiles](./load-profile.md) | yes | yes | yes | no |
| Usage streams (`--usage-streams`) | no | yes | fast mode | no |
| [Reconfiguration rules](./reconfiguration.md) (SimuLizar QVTo) | yes, byte-equal | yes, statistically equal | no | no |
| [Pause, snapshot, resume](./snapshots.md) | yes | yes | yes | yes |
| Pause conditions (`--pause-when`) | yes | yes | yes | yes |
| Resume with changes (`--change`, `--transform`) | yes | yes | yes | yes (no option lines) |
| Enforced policies (`resume --enforce`) | yes | yes | yes | no |
| `simoxide explore` (forked variants) | yes | yes | yes | no |
| Library entry | `Simulation::<Exact>` | `Simulation::<Fast>` | `Simulation::<C, Slingshot>` | `simoxide_sim::ssx::SsxRun` |

Restrictions that hold in every mode:

- Reconfiguration rules and scaling policies cannot be combined in one CLI run (`--no-spd` drops
  the `.spd`).
- `explore` forks snapshots of runs without rules or scaling policies; use `resume` for those.
- A snapshot is resumed in the mode, flavour and compat flags it was taken in.

## Choosing

- **Validate against SimuLizar, or reproduce a published SimuLizar result exactly**: the exact
  build and `--mode exact`.
- **Design-space exploration, optimisation, services**: the default fast mode, in the flavour
  whose semantics you need. It is the fastest mode and the distributable build.
- **Autoscaling with Slingshot's SPD semantics**: `--flavour slingshot` with
  `--features reconfiguration`; `slingshot-exact` when the result must equal Slingshot's run event
  for event.
- **Kubernetes-style autoscaling, time-varying load**: HPA and load profiles, in `simulizar` or
  `slingshot`.
- **What-if at a point in a run**: snapshots, `resume --change`, `explore`.

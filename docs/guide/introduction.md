# Introduction

SimOxide is a performance simulator for **Palladio Component Models** (PCM), written in Rust. It
reads the same model files as Palladio's Java simulators (repository, system, resource
environment, allocation, usage model, monitor repository) and simulates them the way
**SimuLizar 5.2.2** or **Slingshot** does. It runs as a command line tool (`simoxide`) or as a
library inside another program.

The name: an oxidised (that is, Rust) Palladio simulator, in the naming tradition of SimuCom,
SimuLizar and EventSim.

## What it does

- **Simulates like SimuLizar or like Slingshot.** The SimuLizar-compatible mode reproduces the
  DESMO-J engine, the default schedulers and the default measurements of SimuLizar 5.2.2. The
  Slingshot-compatible mode reproduces Slingshot's event timing, resources, monitors and random
  streams, with its bugs fixed by default and compat flags that bring them back.
- **Exactly, or fast.** In the exact modes a run equals the reference simulator's run for the same
  seed byte for byte: the same events, random draws and `measurements.csv`, bugs included. The
  default fast mode keeps the semantics and statistics but draws random numbers faster.
- **Changes the model while it runs.** SimuLizar's reconfiguration rules and Slingshot's scaling
  policies (SPD) run on SimOxide's own QVTo engine; a Kubernetes-style horizontal pod autoscaler
  (HPA, `autoscaling/v2`) scales SPD target groups; load profiles vary the load over time.
- **Pauses, forks and resumes.** A run can stop between two events, be copied, written to a
  snapshot file, changed, and continued in another process. The continuation equals the
  uninterrupted run byte for byte. `simoxide explore` forks one snapshot into variants.
- **Embeds.** A library with no global state that loads models from memory, runs batches in
  parallel and bounds untrusted models with limits.

How the modes combine and which features work in which: [Modes and features](./modes.md).

## Exact

The references are the Java simulators with patches that only make them deterministic:
[refsim](../reference-simulator/refsim.md) (SimuLizar 5.2.2) and the
[Slingshot runner](../reference-simulator/slingshot.md) (Slingshot nightly). SimOxide reproduces
their quirks and bugs, such as FCFS resources that ignore their replicas or `Pois(m)` sampling
Poisson(m) − 1, and the exceptions they abort with.

The SimuLizar claim is checked on a corpus of 66 models, about 12 000 generated models by
differential fuzzing, component oracles that run the real Java classes, and queueing theory
([Exactness and evidence](../correctness/)); the Slingshot claim on a corpus of 90 cases with
Slingshot's event projection ([Slingshot flavour](../correctness/slingshot.md)).

## Fast

SimOxide is much faster than the Java simulators, in one-shot command-line runs and in warm,
parallel batches, at a small memory footprint. The measured numbers are on the
[Performance overview](../performance/).

## Scope

SimOxide covers static performance simulation (components and composition, open and closed
workloads, all SEFF actions, the full StoEx language, processor-sharing, FCFS and delay
resources, linking and passive resources, the measurements of a monitor repository) and runtime
model changes (reconfiguration rules, SPD, HPA, change files). It does not cover reliability, the
exact OS schedulers, reconfiguration engines other than QVTo, usage evolution (DLIM) or EDP2
output. See [Supported models](./scope.md).

## Parts

| Part | Where |
|---|---|
| the simulator: model loader, compiler, event core, both flavours, snapshots | `crates/simoxide-{model,stoex,random,sched,sim}` |
| scaling policies and HPA | `crates/simoxide-spd` |
| runtime model changes, the QVTo host | `crates/simoxide-reconf` |
| the QVTo engine (also usable on its own, with the `qvto` tool) | `crates/qvto` ([QVTo engine](../qvto/)) |
| command line tools `simoxide`, `simoxide-bench` | `crates/simoxide-cli` |
| the Java reference simulators and oracles | `reference/` |

## Licence

EPL-2.0. The SimuLizar-exact mode needs opt-in code that may not be distributed, so the
distributable default build has the fast mode and the Slingshot-exact mode only. Some test models
are git submodules. See [Licensing](./licensing.md).

## Next

- [Getting started](./getting-started.md): build, run a model in each mode.
- [Modes and features](./modes.md): the feature matrix.
- [Command line](./cli.md) and [Library API](./library.md).

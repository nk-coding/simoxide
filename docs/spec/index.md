# Semantics of SimuLizar 5.2.2

These pages specify what SimuLizar 5.2.2 does when it simulates a model, precisely enough to
reproduce its event sequence and measurements bit for bit. The rules are extracted from the
`releases/5.2.2` sources of SimuLizar, SimuCom, AbstractSimEngine, the Scheduler and Core-Commons,
and from the decompiled DESMO-J 2.3.3, with file and line references. SimOxide implements them;
code comments cite the rule identifiers.

| Page | Rules | Content |
|---|---|---|
| [Simulation core](./simulation.md) | `SIM-*`, `ND-*` | engine, nanosecond time, event list, process model, stop conditions, initialisation order, sources of nondeterminism |
| [Workloads](./workloads.md) | `WL-*` | usage model interpretation, open and closed workloads |
| [Actions](./actions.md) | `ACT-*` | RDSEFF actions, composition, parameters and stack frames, passive resources, forks, linking resources |
| [Measurements](./measurements.md) | `MEAS-*` | what is measured, when, and in which order |
| [Stochastic expressions](./stoex.md) | | parser, types, evaluation, Java arithmetic |
| [Random numbers](./random.md) | `RND-*` | the random stream, distribution sampling, PMF and PDF literals, branches |
| [Schedulers](./scheduler.md) | | processor sharing, FCFS, delay and passive resources |

Deliberate differences are on [Deviations](../correctness/deviations.md); SimuLizar's bugs, which
SimOxide reproduces, on [Reference bugs](../correctness/reference-bugs.md).

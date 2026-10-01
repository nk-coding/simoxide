# Introduction

SimOxide is a Rust port of the Palladio performance simulator **SimuLizar 5.2.2**: the DESMO-J
engine, the default schedulers and the default measurements. It takes the same PCM model files
(repository, system, resource environment, allocation, usage model, monitor repository) and
produces the same results.

The name: SimOxide is an oxidised (that is, Rust) Palladio simulator, in the naming tradition of
SimuCom, SimuLizar and EventSim.

## Exact

For a supported model and a seed, SimOxide reproduces the reference **byte for byte**: the same
event trace, the same random draws and the same `measurements.csv`. The reference is SimuLizar
5.2.2 with patches that only make it deterministic. This includes SimuLizar's quirks and bugs,
such as FCFS resources that ignore their replicas or `Pois(m)` sampling Poisson(m) − 1, and the
exceptions it aborts with.

The claim is checked on a corpus of 66 models, on about 12 000 generated models by differential
fuzzing, with component oracles that run the real Java classes, and against queueing theory. See
[Exactness and evidence](../correctness/).

## Fast

SimOxide is two to three orders of magnitude faster than every other Palladio simulator:

- a warm run is 250 to 2 400 times faster than stock SimuLizar 5.2.2, and 180 to 1 000 times
  faster than Slingshot;
- a one-shot command-line run takes milliseconds instead of 3 to 17 seconds;
- on all 22 cores of the benchmark machine it completes over 5 000 MediaStore evaluations per
  second, against 17 for the best JVM configuration, at 64 MB instead of 19 GB.

See the [Performance overview](../performance/). An opt-in [fast mode](./fast-mode.md) trades
the reference's exact random numbers for statistically equivalent, faster ones.

## Embeddable

SimOxide is a library with no global state and a small command line tool. It loads models from
memory, runs many simulations of one compiled model in parallel, bounds untrusted models with
limits, and never panics on any known input. See [Library API](./library.md).

## Scope

SimOxide covers static performance simulation: components and composition, open and closed
workloads, all SEFF actions, the full StoEx language, processor-sharing, FCFS and delay
resources, linking and passive resources, and the measurements of a monitor repository. It does
not cover reliability, the exact OS schedulers, reconfiguration rules or EDP2 output. See
[Supported models](./scope.md).

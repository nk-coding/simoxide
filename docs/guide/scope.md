# Supported models

SimOxide implements the static performance simulation of SimuLizar 5.2.2 with the DESMO-J
engine, the default schedulers and the default measurements.

## Supported

**Model elements.** Repository, system, resource environment, allocation and usage model.

- Basic, composite and subsystem components, nested composition, delegation, infrastructure
  calls and resource calls.
- Open and closed workloads; usage delay, branch, loop and entry-level system call.

**SEFF actions.** Internal action, external call, probabilistic and guarded branch, loop,
collection iterator (`INNER`), synchronous and asynchronous fork, acquire and release, set
variable. Parametric dependencies, component parameters, `BYTESIZE` payloads.

**Stochastic expressions (StoEx).** The full language, including PMFs, PDFs and every
distribution function.

**Resources.**

- Processor-sharing, FCFS and delay CPUs.
- HDDs with read and write rates.
- Linking resources (latency and throughput), optionally with middleware marshalling via
  `stream.BYTESIZE`.
- Passive resources.

**Measurements**, as SimuLizar records them for a monitor repository:

- response times of usage scenarios, system operations, entry-level calls, external calls and
  assembly operations;
- resource state, utilisation (sliding windows), resource demands, passive-resource state,
  waiting and holding times;
- FeedThrough, TimeDriven and Fixed/VariableSize aggregation;
- the side effects of `triggersSelfAdaptations` (the reconfiguration process with empty
  reconfigurations, reconfiguration time, number of resource containers).

**Stop conditions.** Maximum simulated time and maximum number of measurements (finished
usage-scenario runs).

**Reference aborts.** Where SimuLizar aborts with an exception, SimOxide reports the same error
at the same simulated time, after the same trace.

## Not supported

- Reliability and failure simulation.
- The exact OS schedulers (Windows, Linux).
- Reconfiguration rules (QVTo, SPD, Henshin). Only the empty reconfiguration is modelled.
- SimuCom code generation.
- EDP2 output. The output is a plain measurement CSV or the in-memory `Measurements`.
- `ExecutionResult` tuples. The reference runner cannot record them, so they cannot be verified;
  SimOxide warns when a monitor asks for them.
- Nested resource containers. They are ignored, as in SimuLizar, which aborts when a component
  is allocated to one ([REF-8](../correctness/reference-bugs.md#ref-8-nested-resource-containers-are-not-simulated)).

## Limitations

- **Stored measurements are unbounded.** Keeping every tuple is the one input-dependent memory
  cost that no default limit bounds. Bound runs with `Limits::max_events` or a deadline, or set
  `store_measurements = false`.
- **No deadline during loading and compilation.** Both are linear in the input; cap the request
  size at the service boundary.
- **Summaries are basic.** Mean, standard deviation, percentiles and the time-weighted mean;
  no confidence intervals (batch means exist in `simoxide_testkit::stats`).
- **Simulated time ends after about 292 years.** Time is an `i64` in nanoseconds, as in DESMO-J.
  A run past 2^63 ns wraps around as in Java and then aborts with the reference's error.
- **No FFI or JNI wrapper.** The library API is ready for one (see
  [Library API](./library.md#embedding-notes)), but there is no C-ABI or JNI crate.

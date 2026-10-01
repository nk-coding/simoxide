# SimOxide: plan and conventions

> The original plan the project started from, not maintained. The current state is documented in `docs/`;
> the conventions live in `docs/development/contributing.md`.

**Goal:** a Rust port of the Palladio performance simulator that is as fast as possible, heavily
tested, and produces results identical or nearly identical to the reference.

Every agent working here reads this file first. It then appends a short section to
`docs/STATUS.md`: what it did, what is verified, what is open.

## Reference

- **Semantic reference:** SimuLizar **5.2.2** from the prebuilt product at
  `/home/devbox/workspace/palladio-research/work-simucom/palladio-5.2.2/plugins`.
  - It already runs headless without OSGi via
    `/home/devbox/workspace/palladio-research/work-simucom/standalone` (a flat classpath and a
    hand-built Dagger component).
  - Background: `/home/devbox/workspace/palladio-research/notes/02-*.md` and `04-*.md`.
- **Source code:** shallow clones of the Palladio repos (master, 6.0-SNAPSHOT) are in
  `/home/devbox/workspace/palladio-research/repos/`.
  - Where semantics matter, check against the **5.2.2 sources**: fetch the matching tag, or
    decompile the product jars.
  - When master differs from 5.2.2, record it.
- **Where "correct" comes from:** always the behaviour of the patched, deterministic reference in
  `reference/`, never a paper or our own intuition.
  - If the reference has a bug, we reproduce it by default.
  - Deviations go behind an explicit option and are listed in `docs/deviations.md`.

## Scope (v1): static performance simulation

**Model:**
- PCM repository: basic and composite components, interfaces and signatures, parameters.
- System: assembly contexts, connectors, delegation.
- Resource environment: containers, processing resources, linking resources.
- Allocation.
- Usage model: open and closed workloads.
- Usage behaviours: delay, entry-level system call, branch, loop.

**SEFF actions:**
- internal action (resource demands), external call action
- branch (probabilistic and guarded), loop, collection iterator
- fork (synchronous and asynchronous)
- acquire / release of passive resources
- set variable, parametric dependencies, variable characterisations, component parameters
- infrastructure calls, resource calls
- network demand, if SimuLizar 5.2.2 simulates it by default

**StoEx:** the full language: parser, type handling, and evaluation including PMF/PDF arithmetic.

**Resources:**
- the schedulers SimuLizar uses by default: processor sharing, FCFS, delay;
- "exact" OS schedulers only later, if at all.

**Stop conditions:** maximum simulation time and maximum measurement count.

**Measurements** (as SimuLizar produces them by default for a monitor repository, or as agreed
in `docs/spec`):
- response times: usage scenario, calls, operations;
- resource utilisation and state;
- passive resource waiting;
- throughput.

**Out of scope for v1:** reconfiguration and SPD, reliability, SimuCom code generation, EDP2
output (a simple measurement sink replaces it), UI.

## Performance architecture (target)

- The loaded PCM is compiled to a flat, index-based IR:
  - arenas, no reference counting in the hot path;
  - StoEx compiled to bytecode or closures with variable slots resolved in advance.
- One single-threaded discrete-event core per simulation:
  - binary or 4-ary heap keyed by `(time, seq)`;
  - requests are explicit state machines with an explicit call stack, not threads or async
    tasks;
  - no allocation per event in steady state.
- No global state. A batch API runs many simulations in parallel (one per core).
- Keep the reference's float operation order wherever it affects results. Faster algorithms that
  change rounding go behind a flag and are covered by tolerance tests.

## Testing strategy: the core of this project

1. **Deterministic reference** (`reference/`): SimuLizar 5.2.2, patched so that
   - all randomness comes from one seeded stream;
   - hash-order ties become insertion order.

   Each patch is listed in `reference/PATCHES.md`, with evidence that it changes nothing else.
   The reference emits:
   - an **event trace** (`docs/trace-format.md`);
   - a **random tape** (every sample, tagged with its origin);
   - **measurements**.
2. **Component oracles:** small Java programs that call the real 5.2.2 classes and dump golden
   files. Covered components:
   - StoEx parse and evaluate;
   - random-number generators and distributions;
   - schedulers, fed fixed arrivals and demands.

   The Rust crates must reproduce these golden files exactly, or within a documented ulp or
   relative tolerance.
3. **Differential tests:** over the whole model corpus (`corpus/`), Rust vs reference.
   - **Tape replay:** Rust consumes the recorded samples. The event traces must match
     event for event (times within 1e-9 relative).
   - **Own RNG:** Rust uses its own port of the RNG and must match the reference's draws, so
     traces again match event for event.
   - **Statistical:** long runs where exact matching is impossible, tested with KS or confidence
     intervals.
4. **Generated models:** a random PCM model generator (structurally valid, covering all v1
   features) produces fuzzed differential tests against the reference.
5. **Analytical checks:** M/M/1, M/M/c, processor sharing, closed networks via MVA, within
   statistical bounds.
6. **Rust-internal tests:** unit tests, proptest, and snapshot tests of IR and traces.

## Layout

| Path | Contents |
|---|---|
| `docs/spec/` | Semantics extracted from the 5.2.2 source, with file and line references |
| `docs/trace-format.md` | The one event trace format, emitted by both the reference and Rust |
| `reference/` | Java: deterministic runner (CLI), component oracles, scripts; built with javac against the product jars (no Maven or Tycho) |
| `corpus/<model>/` | PCM model files, `run.json` (seed and stop conditions), `expected/` (reference outputs) |
| `crates/simoxide-model/` | XMI loader, typed PCM subset, resolution of cross-file references |
| `crates/simoxide-stoex/` | Parser, types, evaluator, PMF/PDF, compiler to fast IR |
| `crates/simoxide-random/` | Port of the reference RNG and distribution sampling |
| `crates/simoxide-sched/` | Scheduler and resource models |
| `crates/simoxide-sim/` | IR compiler, event core, interpreter, measurements, batch API |
| `crates/simoxide-cli/` | CLI: run a model, emit trace, tape and measurements, run benchmarks |
| `crates/simoxide-testkit/` | Differential harness: trace diff, tape replay, statistical tests, model generator |

## Conventions

- Rust stable (`~/.cargo/bin`), edition 2024, one cargo workspace in `simoxide/`.
  - `cargo fmt`, and `cargo clippy --all-targets -- -D warnings` must pass.
  - No `unsafe` without a comment explaining why and a test.
- Java: `/usr/bin/java` (21), with Temurin 17 at
  `/home/devbox/workspace/palladio-research/work-simucom/jre17` if needed.
  - Reference jars are referenced in place, never copied.
- **Disk is tight (about 6 GB free):**
  - one shared cargo target dir (default `simoxide/target`);
  - delete large temporary outputs;
  - check `df -h /home/devbox` before anything big.
- **No git commits.** The coordinator handles version control.
  - Don't edit files owned by another running agent, except to append to `docs/STATUS.md`.
- Tests must be quick by default. Anything long goes behind `--ignored` or a feature flag.
- Documentation is short and factual.

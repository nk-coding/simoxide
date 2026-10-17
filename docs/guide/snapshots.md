# Snapshots

A snapshot is a paused run written to a file: pause a simulation between two events, write it,
and continue it later, in another process or on another machine, possibly after changing the
model. The continuation is exactly the uninterrupted run: the same trace lines, tape records,
measurements, event and draw counts, byte for byte. This holds in exact and fast mode, in the
SimuLizar and the Slingshot flavour, after model changes, with scaling policies (SPD) and with
SimuLizar's reconfiguration rules. The file layout is on [Snapshot format](../formats/snapshot.md).

Snapshots on disk extend [pause, fork and resume](./library.md#pause-fork-and-resume): a fork is
a copy in memory, a snapshot is the same state as bytes.

## Command line

```sh
# run to t = 120 s, write the snapshot, stop
simoxide run --model corpus/x_sl_mediastore --trace a.jsonl \
    --pause-at 120 --snapshot ms-120.snap

# continue it to the end (outputs get the continuation only)
simoxide resume ms-120.snap --model corpus/x_sl_mediastore \
    --trace b.jsonl --measurements m.csv

# or to the next pause, with a new snapshot
simoxide resume ms-120.snap --model corpus/x_sl_mediastore \
    --pause-at 300 --snapshot ms-300.snap

simoxide snapshot-info ms-300.snap   # the header (JSON)
```

- Pauses: `--pause-at T` (every event at or before `T` seconds has run), `--pause-before T`
  (every event before `T`), `--pause-events N` (at least `N` events). `simoxide resume` also
  takes `--until T`, the same as `--pause-at T`. A paused run reports `paused t=... events=...`
  and writes the measurements recorded so far to `--measurements`.
- `--snapshot-format json` writes the sections as JSON (for reading; see the format page).
- `resume` needs the model the run was created on (`--model`, `--run-json`): the snapshot names
  it by a fingerprint of its content and refuses another model. The mode (`exact`, `fast`), the
  flavour and its compat flags come from the snapshot; the seed, stop conditions and limits are
  part of the run state. `--timeout` applies to the continuation.
- Outputs: the trace, tape, decision log (`--spd-log`) and rules log (`--rules-log`) of the paused
  run and those of the resumed run concatenate to the outputs of the uninterrupted run. A run
  started with `--trace` must be resumed with one (or its lines are discarded); a tape can be
  added or dropped at any pause (exact mode).
- Scaling policies: a run with a `.spd` resumes with its interpreter state; the compat flags of
  `--spd-quirks` are kept in the snapshot and used again.
- Reconfiguration rules (`reconfiguration` feature): resume with the same rules (`--rules DIR` or
  the model's `run.json`); the rules' names are checked.
- The Slingshot-exact engine (`--flavour slingshot-exact`) has no pause yet: `--pause-at` and
  `--snapshot` are refused there.

## Library

```rust
use simoxide_sim::{Outputs, Pause, Simulation};

let mut sim = Simulation::new(&cm, cfg, Outputs::default())?;
sim.run_until(Pause::After(120_000_000_000))?;
let mut file = std::fs::File::create("ms-120.snap")?;
let info = sim.snapshot_to(&mut file)?;            // postcard, between events
println!("{} bytes at t = {} ns", info.bytes, info.header.time_ns);

// later, in another process: the same compiled model
let f = std::fs::File::open("ms-120.snap")?;
let sim = Simulation::<simoxide_sim::Exact>::restore_from(f, &cm, Outputs::default())?;
let result = sim.finish()?;
```

- `Simulation::snapshot_to(w)` and `snapshot_with(w, &SnapshotOptions { encoding }, sections)`
  write the run (flushing its outputs first, as `Simulation::snapshot` does);
  `snapshot_value(enc, sections)` returns it as a `Snapshot` (header and sections) to add
  sections of one's own. `RunState::serialize(&base, enc)` and `RunState::deserialize(&s, &base)`
  work on a state alone.
- `Simulation::restore_from(r, &base, outputs)` and `restore_snapshot(&s, &base, outputs)` check
  the format version, the SimOxide version, the mode, the flavour and the model fingerprint
  (`SnapshotError::{Version, SimOxideVersion, Mismatch, Model}`; damage is
  `SnapshotError::Corrupt`) and continue on `base`, the compiled model the run was created on.
  The type parameters choose mode and flavour as in `Simulation::create`.
- After model changes (`Simulation::apply_changes`, `simoxide_reconf::Edit`, QVTo) the snapshot
  carries the run's own model: its graph as XMI, the build order and the compiled tables, so that
  every index of the run state keeps its meaning. `restore_from` still takes the base model; the
  restored run continues on its own model, and further changes apply to it.
- Snapshots are deterministic: the same state gives the same bytes, and a restored state writes
  the bytes it was read from.
- Not part of a snapshot: the output writers, the wall-clock deadline and the cancellation flag
  of `Limits` (set them again with `Simulation::set_limits`).

### Scaling policies and reconfiguration rules

```rust
// SPD: the interpreter's state (section `spd`); the interpreter is built from the model again
run.snapshot_to(&mut file)?;                       // SpdRun, after run_until
let run = SpdRun::<Exact>::restore_from(f, &cm, &spd, Outputs::default())?;

// SimuLizar's rules: the host's state (section `rules`); a new host with the same rules
rules_run.snapshot_to(&mut file)?;                 // RulesRun, after run_until
let host = RulesHost::new(&core, RuleSet::load_dir(&core, dir)?, cfg)?;
let rules_run = RulesRun::<Exact>::restore_from(f, &cm, host, Outputs::default())?;
```

- `SpdRun` (feature `snapshot` of `simoxide-spd`) keeps the policies' adjustor, trigger and
  aggregation states, the target groups, the membership, the queued adjustment requests and the
  counters. Restoring checks that the interpreter has the same policies and compat flags
  (`simoxide_spd::snapshot::quirks_of` reads the flags from a snapshot).
- `RulesRun` (feature `snapshot` of `simoxide-reconf`) keeps the host's QVTo state, the PRM mirror
  (rebuilt with the same objects), the statistics and kept records. The QVTo state of other hosts
  (e.g. an SPD run's `QvtoAdjuster.run`) goes into a snapshot with
  `QvtoRun::snapshot_section(enc)` and comes back with `QvtoRun::restore_section(&sim, &s)`: the
  id generator continues, so objects created after the restore get the ids they would have got.

## What a snapshot is not (yet)

- No change set is applied at resume time; that is phase N2 (resume with a change set, enforced
  policies, snapshot conditions). Today a restored run can be changed with `apply_changes` or an
  `Edit` before it continues.
- No interchange with Slingshot's `.snapshot` files (phase N3).
- A snapshot holds SimOxide's internal run state: it is read by the same SimOxide version only.

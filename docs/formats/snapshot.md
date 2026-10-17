# Snapshot format (version 1)

A SimOxide snapshot (`simoxide run --snapshot`, `Simulation::snapshot_to`; guide:
[Snapshots](../guide/snapshots.md)) is one file:

```text
SIMOXIDE-SNAPSHOT 1\n                       magic line: the format version
{"format":1,"simoxide":"0.1.0",...}\n       header: one line of JSON
<bytes of section 1><bytes of section 2>...  the sections, in header order
```

## Header

| Field | Meaning |
|---|---|
| `format` | format version (1); a reader refuses other versions |
| `simoxide` | the SimOxide version that wrote it; the sections hold internal run state, so a reader refuses other versions |
| `mode` | `exact` or `fast` |
| `literal` | the literal engine (tests only) |
| `flavour`, `quirks` | `simulizar` or `slingshot`; the Slingshot compat flags (`Q26,Q29`; empty: none) |
| `model` | fingerprint of the compiled model the run was created on (below) |
| `run_model` | after model changes: `{"generation": n, "fingerprint": ...}` of the run's own model; else `null` |
| `time_ns`, `events`, `phase` | simulation time (ns), events processed, `notstarted`/`running`/`finished`/`failed` |
| `traced` | the run writes a trace (a resumed run must keep one or discard its lines) |
| `encoding` | `postcard` or `json`: the encoding of every section |
| `sections` | `[{"name", "bytes", "checksum"}]`: the sections in file order, their length and FNV-1a-64 checksum (16 hex digits) |

The header has no timestamps: the same state gives the same file.

**Model fingerprint.** Two FNV-1a-64 hashes (32 hex digits) over the model graph: per resource
its name and roots, per object its class, home resource, container, `xmi:id`, proxy (file name and
fragment only) and every set feature's value or references, in object order; then `-simulizar`
or `-slingshot` and the compat flags. Absolute paths and source lines are not part of it, so the
same model files in another directory have the same fingerprint
(`simoxide_model::persist::fingerprint`, `simoxide_sim::snapshot::model_fingerprint`).

## Encodings

- `postcard` ([postcard 1.x](https://docs.rs/postcard), serde's compact binary format: varint
  integers, IEEE doubles as their bits, length-prefixed sequences and strings, enum variants by
  index). The default.
- `json` (serde's JSON form of the same values; doubles are printed shortest and read back bit
  for bit; non-finite doubles are written as `null` and cannot be read back). For reading and
  debugging.

Both are serde serialisations of SimOxide's run-state types. Maps are written as lists of pairs in
key order. The sections' layout follows the code (`crates/simoxide-sim/src/sim/snapshot.rs`
and the `serde` derives on the engine's types); it is versioned with SimOxide, not separately.

## Sections

| Name | Written by | Contents |
|---|---|---|
| `engine` | `simoxide-sim` | the run state ([`RunState`](../guide/library.md#pause-fork-and-resume)) |
| `model` | `simoxide-sim`, after model changes | the run's own model |
| `spd` | `simoxide-spd` (`SpdRun`) | the SPD interpreter's state |
| `rules` | `simoxide-reconf` (`RulesRun`) | the reconfiguration host's state |
| `qvto` | `simoxide-reconf` (`QvtoRun::snapshot_section`), for hosts of their own | a run's QVTo state |

A reader ignores sections it does not know; a run restores from the sections its layer needs.

### `engine`

Everything a run changes, as of a point between two events:

- configuration: run name, seed, stop conditions, RNG mode with the replayed tape (uniforms,
  origins, `s` records), processor-sharing algorithm, livelock guard, `Limits` without the
  wall-clock deadline and the cancellation flag, mode;
- phase, the error of a failed run, `traced`;
- clock and counters: current time (the flavour's encoding: integer ns, or the bits of the double
  seconds), events, livelock-guard counters, finished usage-scenario runs, interpreter steps and
  the next `Limits` checks;
- processes: per process slot its generation, pid, state, kind, continuation stack, frame and
  result stacks (indices into the frame arena), assembly-context stack, think/resource flags,
  response-time series; free slots, fork-join tables;
- the frame arena: frames (entries `(key, value or proxy)`, parent) and evaluation proxies
  (program, frame), each referring to earlier entries only, so sharing is kept;
- resources: per active resource its scheduler (queues, remaining demands, last update, pending
  completion, generation), delay set and state shadows; passive resources with their waiting
  queues and start times; component instances; assembly-context paths created during the run;
- random numbers: MT19937 words and position (exact), the replayed tape's position, or the
  xoshiro256++ state (fast), with the counters and tape bookkeeping; Slingshot's
  `java.util.Random` branch stream;
- measurements: series definitions, stored rows (or counts), sliding windows, PRM recorders and
  values, the reconfiguration process, the event list (the heap of notes and the resource timer
  slots, with the FIFO sequence counter), host hooks (external notes, observed tuples);
- resolution caches (provided-role calls, call sites, entry-level calls), warnings, the run
  model's generation.

What is zero or empty between events (nesting counters, allocation pools) is not written.

### `model`

Only after model changes (`Simulation::apply_changes`): the run's compiled model, which keeps
every index the run state holds.

- `generation`, and the **build order**: the graph object id of every typed-model element in
  typed-id order (`build_from_graph_after` rebuilds the typed model with the same ids, removed
  elements as tombstones).
- `graph`, the model graph as XMI (`simoxide_model::persist::GraphXmi`):
  - per resource: URI, name, path, load errors, `failed`, a document file name, and the
    resource's contents as an **XMI document** (EMF's form: the root element or an `xmi:XMI`
    wrapper, set persistent features, attributes as XML attributes, containment as nested
    elements with `xsi:type`, references in the document as IDREFs, references to other documents
    as `href` elements naming the other documents' file names; ids where unique, else paths);
  - one more document, `detached-objects.xmi`, with the objects no resource contains (removed by
    a change, or created and never attached);
  - the **object table**: per document the object id of each element in tree order, per object
    its home resource and source line, the containers of detached roots, the unresolved proxies
    (class, URI, container), and the values XMI does not carry (`null` attributes, empty but set
    lists, values of another type than the feature's, references to proxies).

  Restoring loads the documents with SimOxide's loader and renumbers the objects with the table:
  the restored graph equals the saved one object for object.
- `compiled`: the compiled tables (keys, StoEx programs by their text, behaviours, actions, usage,
  routing and allocation tables, resources, registry, series, windows, PRM, instruction streams
  including the streams appended by changes, Slingshot tables). The typed model, the compiled
  StoEx programs and the finalisation orders are rebuilt from them; the restored model's
  fingerprint must equal `run_model.fingerprint`.

### `spd`

The SPD interpreter's compat flags and shape (policy contexts, time events), and its state: per
policy the adjustor state, the parked chain position, per trigger node the aggregation windows
or the compound filter's iterator; per target group the last enactment; the membership (target
group configurations, allocation, environment, provided signatures; object ids); the queued
adjustment requests; counters.

### `rules`

The rules' names; the run's QVTo state (below); the number of runtime measurements of the PRM
mirror (rebuilt on restore with the same objects); the host's statistics and kept records.

### `qvto`

A run's QVTo state: whether it exists, the number of transformation runs, and the state of its id
generator (the seeded UUID generator of `IdPolicy::Uuid`). The run's model graph is the engine's
(`model`), the journal is empty between events, and models of other metamodels are rebuilt by
their owners. When QVTo runs on the shared EMF graph (merge step M6), the graph's own
serialisation replaces this.

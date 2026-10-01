# refsim

`reference/` holds `refsim`, a deterministic runner for SimuLizar 5.2.2. It is the oracle that
SimOxide is compared with. It runs the unmodified 5.2.2 product jars on a flat classpath, with
a few shadowed classes that remove nondeterminism and add the trace and tape hooks
([Patches](./patches.md)). It needs `javac` and a shell; there is no Maven or Tycho build.

## Requirements

- Java 17 or later (Java 21 is used).
- The Palladio 5.2.2 product plugins. Set `PALLADIO_PLUGINS` to their directory; the default is
  `/home/devbox/workspace/palladio-research/work-simucom/palladio-5.2.2/plugins`.

## Commands

```sh
reference/build.sh          # javac only: build/classpath.txt (product jars in place), build/classes-{base,patches}
reference/tools/classpath.sh  # build/classpath.txt, OSGi-ordered, with the duplicate-class check

reference/refsim run --model <dir | a.usagemodel,b.allocation[,c.monitorrepository]> --seed N \
      [--max-sim-time T] [--max-measurements M] [--trace t.jsonl] [--tape r.jsonl] [--measurements m.csv] \
      [--no-link-throughput]
reference/refsim run --run-json corpus/<model>/run.json --trace ... --tape ... --measurements ...
reference/refsim batch corpus [--only a,b] [--out DIR | --in-place] [--repeat N] [--check] [--no-trace]
reference/refsim gen corpus [--only h01_ps_single,...]      # (re)generate the hand-made models
reference/refsim import <dir | files> <destDir> [--skip-external-calls] [--keep-monitors] [--triggers] [--run-json '{..}']

reference/import-external.sh [corpusDir] [regex]    # re-import the models of external-models.txt
reference/regen-expected.sh [--only a,b] | --check  # corpus/*/expected and corpus/INDEX.md
reference/verify-determinism.sh                     # 2 fresh JVMs, 3x repeat in one JVM, equal to expected/
```

`import --triggers` gives the default monitors `triggersSelfAdaptations = true` (the EMF
default); without it they are `false`.

## Speed

A cold JVM needs about 5 s: 1.5 s bootstrap plus about 3.5 s for the first simulation (class
loading, OCL and EMF initialisation). A warm corpus model then takes 30 to 600 ms. `batch` runs
all models in one JVM after one warm-up run and resets the per-run global state (patches P1 and
P4 to P7), so every batch run equals a fresh-JVM run.

## Environment

| Variable | Effect |
|---|---|
| `REFSIM_UNPATCHED=1` | stock 5.2.2 classes, no state fixes |
| `REFSIM_EXCLUDE=Class,...` | drop individual shadowed classes |
| `REFSIM_VERBOSE=1` | show SimuLizar and DESMO-J output |
| `REFSIM_LOG=INFO` | log4j level |
| `JAVA`, `JVM_OPTS`, `PALLADIO_PLUGINS` | JVM and product location |

SimuLizar uses one Java thread per simulated user. Do not run refsim under `ulimit -v`: thread
creation then fails, and SimuLizar silently ends the run early. Cap the heap instead.

## Layout

| Path | Contents |
|---|---|
| `reference/src/refsim` | CLI, bootstrap (standalone Eclipse registry and EMF), runner, recorder (replaces EDP2), measurements CSV |
| `reference/src/refsim/trace` | trace and tape writer |
| `reference/src/refsim/corpus` | `PcmBuilder`, the hand-made models (`HandMade`), the default monitors (`Monitors`), the importer |
| `reference/patches/src` | shadowed classes; `patches/diffs` holds their diffs against the tag sources |
| `reference/oracles/` | component oracles: StoEx, random numbers, schedulers, PCM loading (EMF) |
| `reference/tools/` | classpath generation and check, patch evidence, source verification, corpus index |

## Sources

The semantics are checked against the `releases/5.2.2` tags of SimuLizar, SimuCom,
AbstractSimEngine, Scheduler, Core-Commons, Core-PCM, QuAL and Analyzer-Framework
(`palladio-research/src-5.2.2/`). `reference/tools/verify-sources.sh` checks that these sources
match the product bytecode. DESMO-J 2.3.3 has no published source; it was decompiled for the
specification.

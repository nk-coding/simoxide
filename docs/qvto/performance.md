# Performance

## Benchmark suite

`qvto bench` runs the workloads of `crates/qvto/qvto-cli/bench/workloads.bench` in the engine's
modes. With `--java` it runs the same workloads in the Java reference runner's benchmark mode in
one warm JVM; with `--java-cold` it starts a fresh JVM per workload.

| Group | Workloads |
|---|---|
| B1 | the conformance cases as workloads (`--suite eclipse\|corpus`) |
| B2 | SimuLizar's reconfiguration rules (load balancer and run-configuration examples) on the corpus PCM models, scaled to 10 and 100 times the containers and contexts; also as series of 50 runs on the same, changing model, as SimuLizar runs its rules at every trigger |
| B3 | Slingshot's SPD `MainTransformation` on the minimal example, scaled; and a series of 20 runs |
| B4 | the SimpleUML to RDB sample of Eclipse QVTo on its model and on models with N copies of its classes (64 to 6 300 output objects) |
| B5 | Ecore copies by mappings with `resolveone`, by `deepclone` with derived features, and queries, on `Ecore.ecore` and generated `.ecore` files of 425 to 42 500 objects |
| B6 | micro benchmarks: loops, recursion, `select`/`collect`, `including`, sets, hash sets, `Dict`, `sortedBy`, `closure`, strings, object creation |

Both sides follow the same protocol. A workload is compiled several times (default 10) and the
last program runs. In fresh mode, the inputs are loaded into a new resource set before each run
(not timed); in series mode, they are loaded once and the workload runs N times. A run is the
VM's creation, `run_transformation` and its drop (Java: `executor.execute`). The report gives
the median and the p10 to p90 range. It also compares the compat run's severity, output object
count and log line count with Java's.

```sh
cargo build --profile release-unwind -p qvto-cli --features compat
taskset -c 2,3 target/release-unwind/qvto bench --java --java-cold
target/release-unwind/qvto bench --group B2 --only b2_lb_scaleUp_x1 --runs 50 --before old/last.tsv
```

Results go to `target/qvto-bench/last.{md,tsv}`. Run pinned to one or two cores at normal
priority (the JVM inherits the affinity) and repeat; the report prints the load average.

## Results

Measured at milestone Q5b (2026-10-17) on a shared, loaded machine, pinned to two cores; expect
±20 % between repetitions. The table per workload is in `crates/qvto/qvto-cli/README.md`,
"Results".

| Workload | This engine | vs Eclipse, warm JVM | vs Eclipse, first run in a fresh JVM |
|---|---|---:|---:|
| B2 one rule run (corpus models ×1, ×10) | 0.09-0.12 ms | 4-11× | 180-400× |
| B2 with 100 times the containers | 0.3-0.45 ms | 2-7× | 80-160× |
| B2 series (per run, same model) | 5-35 µs | 4-18× | |
| B3 SPD `MainTransformation` | 0.23-0.86 ms | 4.5-11× | 125-210× |
| B4 SimpleUML to RDB (sample, ×10, ×100) | 0.2 / 1.5 / 21 ms | 13× / 9× / 2.7× | 30-250× |
| B5 Ecore copy by mappings (`Ecore.ecore`, 4 250, 42 500 objects) | 0.24 / 8.9 / 181 ms | 7× / 2.7× / 2.1× | 7-290× |
| B5 `deepclone`, queries | | 1.4-6× | 5-100× |
| B6 micro benchmarks | | 2.8-23×; sets built in loops 200-2 800× | |
| B1 suite cases | | median 5.8×, p90 14× | |
| compile | 0.1-0.9 ms per rule file; 7-8 ms for the 18 SPD units | 3-20× | 100-1 000× |

- Fresh runs of small rules are dominated by cold caches: the models were just loaded. A second
  run on the same model takes a third of the time, which is SimOxide's case (the series rows).
- On large models (B4 ×100, B5 ×10 and ×100) the time goes to model access and allocation.
  There the engine is 2 to 3 times faster than a warm JVM.
- Fast mode is 1.0 to 1.4 times faster than compat mode on hash-heavy workloads, and equal
  elsewhere. Ordered mode costs the same as fast mode.
- About 10 % of the suite cases run slower than in a warm JVM. They are transformations of a few
  microseconds, measured on cold, freshly loaded models.

These numbers were measured before the merge into SimOxide, on the engine's own EMF graph. The
graph is now SimOxide's model graph too ([Foundation crates](../development/foundation.md)).
In SimOxide, a reconfiguration rule run costs about 0.1 ms in ordered mode
([Reconfiguration rules](../guide/reconfiguration.md#cost)).

## What makes it fast

- Names are resolved at compile time: classes, features, operations and variables become ids and
  register slots.
- Register bytecode with instructions specialised by static type. OCL iterators compile to
  inline loops, so no evaluation environment is created per iteration (Eclipse's interpreter
  creates one).
- Copy-on-write collections: building a set in a loop is amortised O(1) per step. Java copies the
  set on every `including`.
- O(1) feature slots in the graph, cached Ecore derived lists, Fx hashing, and dense
  identity-hash tables.
- Tables the VM would otherwise compute per call (the overriding chain, library classes) are
  computed once per program.
- A many-valued feature read again while the store is unchanged (`ModelStore::version`, which
  the shared graph keeps) gives the collection of the earlier read, not a new copy; a typed
  `xselect` (`coll[T]`) filters a collection in one instruction and keeps its last results by
  source value. Neither draws an identity hash Java would not draw, so compat mode is unchanged.
  Loops over a model's lists (Slingshot's `MainTransformation` on a large system: half the time
  per adjustment, `DESIGN-SPD.md` §5.10) gain most.

Profiling: build with `--features profile` and pass `qvto bench --profile out.svg`.

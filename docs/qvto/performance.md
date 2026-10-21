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

Measured on 2026-10-21 in SimOxide (the engine on SimOxide's shared model graph) on a quiet
AMD Ryzen 9 9950X3D VM (25 cores, 34 GB), pinned to two cores; Eclipse QVTo 3.11 on OpenJDK 21
(`bench/compare/engine.sh`, report in
`bench/compare/results/2026-10-21-engine/qvto-bench.md`). Run times are medians of 30 runs.

| Workload | This engine (compat and fast mode) | vs Eclipse, warm JVM | vs Eclipse, first run in a fresh JVM |
|---|---|---:|---:|
| B2 one rule run (corpus models ×1, ×10) | 0.075-0.11 ms | 4-13× | 160-410× |
| B2 with 100 times the containers | 0.22-0.44 ms | 1.8-5.4× | 90-185× |
| B2 series (per run, same model) | 13-33 µs | 2.7-5.5× | |
| B3 SPD `MainTransformation` (×1 and ×10, ×100) | 0.17-0.26 / 0.6-0.72 ms | 5.5-15× | 100-290× |
| B4 SimpleUML to RDB (sample, ×10, ×100) | 0.21 / 1.6 / 17-19 ms | 9.6-21× / 7.5× / 2.7× | 27-230× |
| B5 Ecore copy by mappings (`Ecore.ecore`, 4 250, 42 500 objects) | 0.26 / 8.4 / 152-175 ms | 10× / 2.8× / 2.5× | 7-240× |
| B5 `deepclone`, queries | | 1.1-4.8× | 5-70× |
| B6 micro benchmarks | | 2.2-50×; sets built in loops 240-3 300× | |
| compile | 0.3-0.6 ms per rule file; 6.5-7.2 ms for the 18 SPD units | 3-20× | 300-1 000× |

- Against the measurement before the merge (Q5b, 2026-10-17, a shared and loaded machine) the
  engine is as fast or faster: rule runs 0.08-0.1 ms (then 0.09-0.12), the SPD
  `MainTransformation` 0.17-0.72 ms (then 0.23-0.86), the big Ecore copies 152-175 ms (then
  181 ms); the copy of `Ecore.ecore` takes 0.26 ms (then 0.24; measured in a separate run of the
  suite the same day, after its input path was fixed). The warm JVM ratios are partly smaller because Eclipse QVTo ran faster on the quiet
  machine.
- Fresh runs of small rules are dominated by cold caches: the models were just loaded. A second
  run on the same model takes a sixth to a third of the time, which is SimOxide's case (the
  series rows).
- On large models (B4 ×100, B5 ×10 and ×100) the time goes to model access and allocation.
  There the engine is 2.5 to 3 times faster than a warm JVM.
- Fast mode runs these workloads 0.9 to 1.2 times as fast as compat mode (ordered mode was not run).
- Not re-measured: the B1 suite cases (`--suite eclipse`; Q5b: median 5.8× a warm JVM, about 10 %
  of the cases slower, transformations of a few microseconds on cold, freshly loaded models).

In SimOxide, a reconfiguration rule run costs about 0.1 ms (80 µs per evaluation on the power
example, [Reconfiguration rules](../guide/reconfiguration.md#cost)) and an SPD adjustment 0.5 to
0.9 ms including the model change ([Engine performance](../performance/engine.md#runtime-changes-spd-and-snapshots)).

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

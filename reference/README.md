# reference/: deterministic SimuLizar 5.2.2 (`refsim`)

The oracle SimOxide is compared with: SimuLizar 5.2.2 from the unmodified product jars, with a few
shadowed classes that make it deterministic and emit the event trace and random tape.

Documentation: `docs/reference-simulator/refsim.md` (usage, environment, layout),
`docs/reference-simulator/patches.md` (every patch with its evidence, the classpath rules) and
`docs/guide/formats.md` (output formats).

```
./build.sh                                   # javac against the product jars (PALLADIO_PLUGINS)
./refsim run --run-json ../corpus/h01_ps_single/run.json --trace t.jsonl --tape u.jsonl --measurements m.csv
./refsim batch ../corpus --check             # whole corpus in one warm JVM, compared with expected/
./regen-expected.sh --check                  # corpus expected outputs are current
./verify-determinism.sh                      # 2 fresh JVMs + 3x repeat + expected/
```

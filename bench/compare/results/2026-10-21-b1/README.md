# Benchmark B1 (DESIGN-SPD.md §6), 2026-10-21

Machine: AMD Ryzen 9 9950X3D VM, 25 cores, 34 GB, Linux 6.8; OpenJDK 21.0.12 (Slingshot, refsim, `-Xmx4g`,
G1); SimOxide release builds (`simoxide-compare` driver, `simoxide` CLI with
`exact,hotspot-math,reconfiguration`, the `spd-bench` example of `simoxide-reconf`). Every benchmark process
ran alone (`wait_quiet`: at most 2 runnable tasks), unpinned, 3 processes per configuration with seeds 1 to 3.
Slingshot: the S0 reference runner (`reference/slingshot/run.sh --stock`, nightly of 2026-09-01, seeded,
no instrumentation or measurement output in the timed runs).

`bench/compare/spd/b1.sh` (about 45 minutes) wrote the raw logs to `bench/compare/spd/work/raw`
(git-ignored); `summary.md` and `b1.csv` are `bench/compare/spd/summarize-b1.py`'s output.

Not measured: stock SimuLizar with reconfiguration rules (the flat-classpath driver cannot run them;
refsim, the patched SimuLizar, stands in); Slingshot's snapshot side (needs the MENTOR application and
branch-only ExperimentAutomation, DESIGN-SPD.md U7, not installed); the time Slingshot spends in
adjustments (not instrumented; only its total time per run).

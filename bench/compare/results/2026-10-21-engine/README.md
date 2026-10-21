# SimOxide engine numbers, 2026-10-21

Machine: AMD Ryzen 9 9950X3D VM, 25 cores, 34 GB, Linux 6.8, rustc 1.98.1, OpenJDK 21.0.12 (QVTo's Java
side). Quiet machine (1-min load 1 to 2.5, the benchmark's own processes included); single-threaded runs
pinned to core 3, the QVTo benchmark to cores 2 and 3. Binaries: release profile (fat LTO), the CLI and
`simoxide-bench` with `--features exact,hotspot-math,reconfiguration` (fast-mode rows use `--mode fast`
of that build); `qvto` with `--profile release-unwind --features compat`.

`bench/compare/engine.sh` (all phases, about 10 minutes) wrote `raw/` (git-ignored); `summary.md` is
`summarize-engine.py`'s digest, `qvto-bench.md` the QVTo report (`qvto bench --java --java-cold`).
Not measured: `b5_copy_Ecore` (its input `crates/qvto/qvto-emf/models/Ecore.ecore` no longer exists
after the merge into SimOxide), the B1 suite cases (`--suite eclipse`).

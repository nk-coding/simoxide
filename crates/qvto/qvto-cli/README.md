# qvto-cli

The `qvto` command: `qvto run` (compile and run a transformation), `qvto disasm` (the compiled
program), `qvto conform` (the conformance scoreboard, `qvto-testkit/README.md`),
`qvto bench` (the benchmark suite) and `qvto spd-replay` (Slingshot's SPD adjustments against the
Slingshot reference runner's records). `src/engine.rs` is the engine under the harness,
`src/bbox.rs` the Eclipse test bundle's Java blackboxes ported, `src/bench.rs` the benchmarks,
`src/spdreplay.rs` the replay.

```sh
qvto run t.qvto in.xmi --repeat 10 [--fast | --ordered]
qvto spd-replay                              # the det runs of reference/slingshot, ordered mode: 13 of 13
qvto spd-replay MODELDIR SPD.jsonl --mode compat
qvto bench                                   # all workloads, compat and fast mode (--modes compat,fast,ordered)
taskset -c 2,3 qvto bench --java --java-cold # with the Java runner (warm: one JVM; cold: one JVM per workload)
qvto bench --group B2 --only b2_lb_scaleUp_x1,b3_spd_main_x1 --runs 50
qvto bench --suite corpus                    # the conformance cases as workloads (B1)
cargo build --profile release-unwind -p qvto-cli --features profile && qvto bench --only b6_fib --profile fib.svg
```

## Benchmark suite (`qvto bench`)

Workloads are cases of `bench/workloads.bench` (the case format of `qvto-testkit` plus
`bench <key> <value>` lines). Models that the workloads scale are generated into
`target/qvto-bench/gen` before a run (deterministic; `qvto bench gen` regenerates them).

| Group | Workloads | What |
|---|---|---|
| B2 | `b2_lb_{scaleUp,scaleIn,outsource}_x{1,10,100}`, `b2_rc_{addNewServer,addClonedServer}_x{1,10,100}` | SimuLizar's reconfiguration rules (the corpus' load balancer and run-configuration examples) with a measurement above the thresholds, on the corpus PCM models with (N−1)·k more resource containers, assembly and allocation contexts (k: the model's allocation contexts) |
| B2 series | `b2_*_series_x{1,10}` | a rule run 50 times on the same, changing model (as SimuLizar runs its rules at every trigger; `addNewServer` adds a server per run) |
| B3 | `b3_spd_main_x{1,10,100}`, `b3_spd_main_series_x1` | Slingshot's SPD `MainTransformation` on the minimal example (scaled resource environments), and 20 runs on the same models |
| B4 | `b4_uml2rdb_pim`, `b4_uml2rdb_x{1,10,100}` | the SimpleUML to RDB sample of `org.eclipse.m2m.qvt.oml.samples` on its `pim.simpleuml` and on models with N copies of its classes (64 to 6 300 output objects) |
| B5 | `b5_copy_Ecore`, `b5_copy_x{1,10,100}`, `b5_deepclone_x{1,10,100}`, `b5_query_x{10,100}` | Ecore copies by mappings with disjuncts and `resolveone` (`bench/ecore/EcoreCopy.qvto`), by `deepclone` plus derived features (`EcoreDeepclone.qvto`), and queries (`eAllSuperTypes`, `eAllReferences`, `closure`, `sortedBy`, `Dict`, strings) on `Ecore.ecore` and generated `.ecore` files of 5N packages × 10 classes (425 to 42 500 objects) |
| B6 | `b6_{loop,fib,selectcollect,including,sets,hashsets,dict,sortedby,closure,strings,objects}` | micro benchmarks (`bench/micro/*.qvto`) |

**Protocol** (the same on both sides; the Java side is `qvto-run --bench`,
`reference/runner/src/qvtorun/Bench.java`): a workload is compiled `--compiles` times (default
10); the last program runs. Fresh mode loads the inputs into a new resource set before each run
(not timed) and runs until `--runs` runs (default 30) or `--budget-ms` of run time (default
2000), at least 5. Series mode (`bench series N`) loads once and runs N times on the changing
models, in `bench rounds` rounds (default 2, fresh models each) of which the last counts. A run
is the VM's creation, `run_transformation` and its drop (Java: `executor.execute`). Reported:
median and p10–p90 of the second half of the runs (fresh) or of the round (series) and the
series total; compile: median (first). `--java` runs all workloads in one JVM (warm, after the
first half of the runs); `--java-cold` starts a JVM per workload and takes its first compile and
first run. The last column compares the severity, the number of output objects and of log lines
of the compat run with Java's. Results: `target/qvto-bench/last.{md,tsv}`, `java.tsv`,
`java-cold-all.tsv`; `--java-results`, `--java-cold-results` and `--before` (an earlier
`last.tsv`) put earlier results into the report.

Measurements are sensitive to load: run pinned to one or two cores at normal priority
(`taskset -c 2,3 qvto bench ...`; the JVM inherits the affinity, `QVTO_NICE=0`), repeat, and note
the load average, which the report prints.

## Results

Q5b, 2026-10-17, this shared machine (21 cores, other agents' builds and JVMs running), pinned to
two cores at normal priority; load average 5.7–7.3 during the Rust runs, 2.9–5.7 during the Java
runs (`target/qvto-bench`, `qvto bench --before ... --java-results ... --java-cold-results ...`).
*compat before*: the engine of main before Q5b (`87bada5` plus Q6, with this harness); times in
ms, median [p10–p90]; *Java cold*: first run in a fresh JVM; *compile*: median (first) of the
compiles, Java's warm median (first compile in a fresh JVM). Expect ±20 % between repetitions.

| Workload | compat before | compat run | speed-up | fast run | Java run (warm) | Java cold run | compat vs Java | fast vs compat | compile | Java compile (first) |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `b2_lb_scaleUp_x1` | 0.102 [0.095–0.134] | 0.086 [0.083–0.109] | 1.2× | 0.083 [0.075–0.097] | 0.811 [0.540–4.20] | 24.4 | 9.4× | 1.0× | 0.474 (1.19) | 8.91 (335) |
| `b2_lb_scaleIn_x1` | 0.105 [0.101–0.116] | 0.087 [0.080–0.109] | 1.2× | 0.095 [0.083–0.117] | 0.786 [0.504–2.01] | 30.3 | 9.0× | 0.9× | 0.442 (0.500) | 4.82 (364) |
| `b2_lb_outsource_x1` | 0.115 [0.103–0.167] | 0.092 [0.082–0.123] | 1.2× | 0.090 [0.079–0.119] | 1.02 [0.490–3.97] | 22.6 | 11× | 1.0× | 0.507 (0.563) | 7.95 (393) |
| `b2_rc_addNewServer_x1` | 0.111 [0.100–0.183] | 0.097 [0.088–0.133] | 1.1× | 0.098 [0.087–0.163] | 0.423 [0.401–0.601] | 19.5 | 4.4× | 1.0× | 0.497 (0.576) | 4.50 (378) |
| `b2_rc_addClonedServer_x1` | 0.119 [0.105–0.142] | 0.093 [0.077–0.105] | 1.3× | 0.095 [0.081–0.117] | 0.913 [0.495–1.83] | 31.6 | 9.8× | 1.0× | 0.494 (0.762) | 7.21 (334) |
| `b2_lb_scaleUp_x10` | 0.131 [0.125–0.142] | 0.108 [0.103–0.160] | 1.2× | 0.100 [0.093–0.113] | 0.655 [0.605–0.780] | 20.7 | 6.1× | 1.1× | 0.432 (0.418) | 2.05 (361) |
| `b2_lb_scaleIn_x10` | 0.117 [0.106–0.149] | 0.107 [0.088–0.144] | 1.1× | 0.113 [0.094–0.152] | 0.820 [0.560–1.35] | 18.6 | 7.6× | 1.0× | 0.427 (0.662) | 2.67 (329) |
| `b2_lb_outsource_x10` | 0.123 [0.104–0.146] | 0.115 [0.102–0.150] | 1.1× | 0.109 [0.101–0.117] | 0.446 [0.345–0.662] | 19.5 | 3.9× | 1.1× | 0.494 (0.669) | 4.70 (357) |
| `b2_rc_addNewServer_x10` | 0.141 [0.131–0.193] | 0.101 [0.098–0.121] | 1.4× | 0.107 [0.096–0.122] | 0.755 [0.499–2.01] | 21.7 | 7.4× | 0.9× | 0.493 (0.612) | 6.06 (387) |
| `b2_rc_addClonedServer_x10` | 0.121 [0.109–0.159] | 0.105 [0.097–0.137] | 1.2× | 0.116 [0.108–0.134] | 0.634 [0.416–1.79] | 20.8 | 6.0× | 0.9× | 0.482 (0.532) | 3.79 (329) |
| `b2_lb_scaleUp_x100` | 0.509 [0.443–0.728] | 0.453 [0.415–0.549] | 1.1× | 0.510 [0.408–0.766] | 3.31 [3.13–3.99] | 74.6 | 7.3× | 0.9× | 0.376 (0.657) | 2.75 (361) |
| `b2_lb_scaleIn_x100` | 0.393 [0.355–0.466] | 0.302 [0.257–0.352] | 1.3× | 0.275 [0.247–0.421] | 0.868 [0.738–1.22] | 25.3 | 2.9× | 1.1× | 0.509 (0.720) | 3.87 (306) |
| `b2_lb_outsource_x100` | 0.367 [0.323–0.435] | 0.295 [0.271–0.745] | 1.2× | 0.308 [0.239–0.596] | 0.718 [0.545–2.01] | 36.9 | 2.4× | 1.0× | 0.438 (0.645) | 3.51 (372) |
| `b2_rc_addNewServer_x100` | 0.351 [0.324–0.377] | 0.441 [0.305–0.876] | 0.8× | 0.259 [0.226–0.312] | 0.816 [0.740–0.981] | 39.0 | 1.9× | 1.7× | 0.523 (1.60) | 2.78 (361) |
| `b2_rc_addClonedServer_x100` | 0.381 [0.352–0.465] | 0.351 [0.316–0.510] | 1.1× | 0.427 [0.378–0.550] | 1.04 [0.647–2.08] | 42.1 | 3.0× | 0.8× | 0.505 (1.32) | 3.06 (349) |
| `b2_lb_scaleUp_series_x1` | 0.006 [0.006–0.006] Σ 0.394 | 0.005 [0.005–0.005] Σ 0.290 | 1.4× | 0.006 [0.006–0.006] Σ 0.389 | 0.041 [0.038–0.112] Σ 5.15 | 1.15 | 18× | 0.7× | 0.409 (0.755) | 3.15 (339) |
| `b2_lb_scaleIn_series_x1` | 0.008 [0.008–0.009] Σ 0.531 | 0.006 [0.006–0.008] Σ 0.406 | 1.3× | 0.006 [0.006–0.008] Σ 0.467 | 0.076 [0.067–0.135] Σ 5.49 | 0.868 | 14× | 0.9× | 0.479 (0.478) | 2.18 (370) |
| `b2_rc_addNewServer_series_x1` | 0.009 [0.009–0.012] Σ 0.711 | 0.008 [0.008–0.010] Σ 0.560 | 1.3× | 0.012 [0.011–0.019] Σ 0.798 | 0.049 [0.044–0.156] Σ 5.18 | 5.54 | 9.2× | 0.7× | 0.595 (0.599) | 3.84 (342) |
| `b2_lb_scaleUp_series_x10` | 0.026 [0.025–0.044] Σ 1.73 | 0.035 [0.034–0.043] Σ 1.94 | 0.9× | 0.021 [0.021–0.025] Σ 1.19 | 0.128 [0.106–0.190] Σ 17.5 | 1.17 | 9.1× | 1.6× | 0.400 (0.377) | 1.33 (388) |
| `b2_lb_scaleIn_series_x10` | 0.011 [0.011–0.015] Σ 0.725 | 0.008 [0.008–0.012] Σ 0.581 | 1.2× | 0.009 [0.008–0.009] Σ 0.504 | 0.032 [0.030–0.050] Σ 2.09 | 1.07 | 3.6× | 1.2× | 0.450 (0.465) | 1.69 (345) |
| `b2_rc_addNewServer_series_x10` | 0.017 [0.016–0.019] Σ 0.973 | 0.015 [0.014–0.016] Σ 0.852 | 1.1× | 0.015 [0.014–0.025] Σ 0.942 | 0.073 [0.056–0.181] Σ 4.73 | 1.64 | 5.6× | 0.9× | 0.811 (0.937) | 2.21 (413) |
| `b3_spd_main_x1` | 0.281 [0.251–0.322] | 0.230 [0.219–0.327] | 1.2× | 0.273 [0.239–0.507] | 2.53 [1.83–3.87] | 48.4 | 11× | 0.8× | 7.69 (8.70) | 75.1 (703) |
| `b3_spd_main_x10` | 0.354 [0.314–0.372] | 0.301 [0.272–0.412] | 1.2× | 0.263 [0.248–0.318] | 2.36 [1.39–3.15] | 47.3 | 7.8× | 1.1× | 7.75 (11.3) | 66.7 (749) |
| `b3_spd_main_x100` | 1.04 [0.980–1.29] | 0.861 [0.686–1.12] | 1.2× | 0.681 [0.580–0.904] | 3.88 [3.34–4.32] | 108 | 4.5× | 1.3× | 7.98 (7.54) | 55.5 (1499) |
| `b3_spd_main_series_x1` | 0.224 [0.122–0.345] Σ 4.79 | 0.158 [0.090–0.241] Σ 3.33 | 1.4× | 0.172 [0.113–0.255] Σ 3.58 | 1.04 [0.763–1.43] Σ 23.7 | 7.69 | 7.1× | 0.9× | 7.13 (7.13) | 45.6 (715) |
| `b4_uml2rdb_pim` | 0.315 [0.227–0.421] | 0.203 [0.181–0.230] | 1.6× | 0.204 [0.199–0.235] | 2.70 [2.46–3.47] | 50.2 | 13× | 1.0× | 0.586 (0.655) | 4.77 (253) |
| `b4_uml2rdb_x1` | 0.238 [0.213–0.286] | 0.300 [0.272–0.326] | 0.8× | 0.296 [0.269–0.348] | 1.75 [1.66–1.95] | 65.8 | 5.9× | 1.0× | 0.947 (1.00) | 4.45 (332) |
| `b4_uml2rdb_x10` | 1.98 [1.88–2.81] | 1.46 [1.42–2.27] | 1.4× | 1.48 [1.41–1.62] | 13.4 [10.7–16.3] | 167 | 9.1× | 1.0× | 0.870 (0.954) | 4.92 (295) |
| `b4_uml2rdb_x100` | 26.5 [22.9–34.2] | 20.6 [18.0–30.6] | 1.3× | 18.6 [17.7–29.6] | 55.2 [49.6–63.6] | 564 | 2.7× | 1.1× | 0.615 (1.04) | 4.60 (349) |
| `b5_copy_Ecore` | 0.329 [0.295–0.391] | 0.242 [0.222–0.271] | 1.4× | 0.222 [0.207–0.282] | 1.78 [1.49–3.31] | 69.4 | 7.4× | 1.1× | 0.291 (0.440) | 1.71 (253) |
| `b5_copy_x1` | 0.770 [0.705–0.823] | 0.534 [0.526–0.588] | 1.4× | 0.546 [0.527–0.639] | 3.61 [3.23–4.18] | 107 | 6.8× | 1.0× | 0.316 (0.675) | 1.79 (282) |
| `b5_copy_x10` | 15.3 [11.6–23.1] | 8.93 [8.46–10.2] | 1.7× | 10.3 [8.97–15.3] | 24.0 [23.2–30.0] | 416 | 2.7× | 0.9× | 0.297 (0.336) | 1.47 (260) |
| `b5_copy_x100` | 361 [209–406] | 181 [169–201] | 2.0× | 199 [168–284] | 380 [352–384] | 1242 | 2.1× | 0.9× | 0.290 (0.422) | 1.96 (275) |
| `b5_deepclone_x1` | 7.03 [6.81–7.45] | 0.340 [0.317–0.367] | 21× | 0.312 [0.305–0.377] | 1.85 [1.75–2.13] | 32.8 | 5.4× | 1.1× | 0.137 (0.204) | 0.659 (219) |
| `b5_deepclone_x10` | 50.8 [48.6–76.6] | 4.43 [4.01–5.55] | 11× | 4.20 [3.86–5.29] | 6.12 [5.70–7.05] | 96.6 | 1.4× | 1.1× | 0.131 (0.180) | 1.08 (211) |
| `b5_deepclone_x100` | 1005 [913–1130] | 77.5 [73.3–109] | 13× | 75.9 [73.0–102] | 121 [107–148] | 368 | 1.6× | 1.0× | 0.143 (0.219) | 0.793 (231) |
| `b5_query_x10` | 11.6 [11.1–13.1] | 4.52 [3.98–4.92] | 2.6× | 4.19 [3.93–4.71] | 26.7 [17.9–27.9] | 148 | 5.9× | 1.1× | 0.239 (0.349) | 1.22 (243) |
| `b5_query_x100` | 202 [189–385] | 84.6 [80.7–118] | 2.4× | 61.6 [55.9–74.4] | 233 [202–272] | 728 | 2.8× | 1.4× | 0.181 (0.262) | 1.23 (231) |
| `b6_loop` | 15.7 [15.6–15.9] | 16.6 [16.3–21.6] | 0.9× | 20.7 [16.5–27.6] | 381 [320–404] | 425 | 23× | 0.8× | 0.125 (0.819) | 0.435 (199) |
| `b6_fib` | 17.4 [14.3–23.0] | 14.8 [14.6–15.3] | 1.2× | 14.1 [14.1–15.7] | 123 [107–135] | 374 | 8.3× | 1.0× | 0.130 (0.232) | 0.325 (270) |
| `b6_selectcollect` | 5.20 [5.16–5.28] | 5.01 [4.89–5.17] | 1.0× | 7.30 [7.20–7.47] | 19.3 [13.9–31.5] | 143 | 3.9× | 0.7× | 0.109 (0.385) | 0.242 (246) |
| `b6_including` | 1.91 [1.86–1.93] | 1.84 [1.80–1.89] | 1.0× | 1.63 [1.60–2.04] | 5187 [4999–5280] | 5034 | 2822× | 1.1× | 0.126 (0.303) | 0.572 (285) |
| `b6_sets` | 14.5 [13.1–22.1] | 12.5 [11.9–13.3] | 1.2× | 11.2 [11.0–11.7] | 2655 [2548–2655] | 3399 | 212× | 1.1× | 0.176 (0.518) | 1.87 (267) |
| `b6_hashsets` | 16.0 [15.2–17.2] | 13.7 [13.0–17.0] | 1.2× | 12.7 [12.3–13.2] | 50.0 [41.1–82.8] | 249 | 3.7× | 1.1× | 0.186 (0.237) | 1.67 (258) |
| `b6_dict` | 16.1 [15.6–28.6] | 14.7 [14.1–15.8] | 1.1× | 14.9 [14.7–15.9] | 42.5 [34.7–69.1] | 313 | 2.9× | 1.0× | 0.161 (0.245) | 0.959 (256) |
| `b6_sortedby` | 2175 [2149–2474] | 10.7 [10.4–11.3] | 204× | 11.2 [10.9–11.9] | 35.0 [31.3–52.8] | 254 | 3.3× | 1.0× | 0.141 (0.234) | 0.555 (212) |
| `b6_closure` | 5.36 [5.10–5.97] | 4.86 [4.80–4.92] | 1.1× | 4.99 [4.93–5.12] | 23.1 [19.5–32.3] | 161 | 4.8× | 1.0× | 0.132 (0.199) | 0.454 (227) |
| `b6_strings` | 16.7 [16.1–18.0] | 15.1 [14.9–17.1] | 1.1× | 15.4 [15.1–15.5] | 42.4 [39.6–53.2] | 207 | 2.8× | 1.0× | 0.146 (0.195) | 0.878 (204) |
| `b6_objects` | 509 [507–534] | 8.42 [7.95–9.20] | 61× | 8.37 [7.80–8.63] | 537 [490–558] | 667 | 64× | 1.0× | 0.166 (0.259) | 0.900 (215) |

Summary:

- **SimuLizar rules (B2)**: one run on freshly loaded models 0.09–0.12 ms (Java warm 0.42–1.0 ms,
  4–11×; Java cold 18–42 ms, 180–400×); with 100× more containers 0.3–0.45 ms (2–7×). A series of
  runs on the same model (SimOxide's case): 5–35 µs per run (4–18× Java warm). Fresh runs of
  small rules are bound by cold caches (the models were just loaded): a second run on the same
  model takes a third of the time (DESIGN §12 Q5b-11).
- **SPD `MainTransformation` (B3)**: 0.23–0.86 ms per run (4.5–11× Java warm, 125–210× cold);
  compile of its 18 units 7–8 ms (Java warm 46–75 ms, cold 0.7–1.5 s).
- **SimpleUML→RDB (B4)**: 0.2 ms on the sample's model (13× Java warm), 1.5 ms ×10 (9×), 21 ms
  ×100 (2.7×).
- **Ecore copies (B5)**: mappings with `resolveone` 0.24 ms on `Ecore.ecore` (7×), 2–3× at 4 250
  and 42 500 objects; `deepclone` plus derived features 11–21× faster than before Q5b, 1.4–5×
  Java; queries 2.4–2.6× faster than before, 2.8–6× Java.
- **Micro (B6)**: 2.8–23× Java warm; building sets in loops 200–2 800× (Java copies the set
  per `including`).
- **Compile**: 0.1–0.9 ms per rule or benchmark file (Java warm 0.3–9 ms, cold 200–400 ms).
- **Fast mode**: 1.0–1.4× compat on hash-heavy workloads (`b6_hashsets`, `b6_sets`,
  `b6_including`, `b5_query_x100`), equal elsewhere (DESIGN §12 Q5b-10).
- **Suite cases (B1)**, `qvto bench --suite eclipse --java` (338 runnable cases with a run):
  median 5.8× Java warm, p90 14×, up to 270×; the 10 % below 1× are transformations of a few
  microseconds (ours 4–15 µs including the VM's creation on cold, freshly loaded models; Java
  warm 4–6 µs). Three cases fail on repeated execution with one Java executor in the benchmark
  mode (`interpret_bug329971`, `interpret_intermProperties`, `interpret_bug386115`).
- The *check* column of the report compares severity, output objects and log lines with Java:
  equal everywhere except `b2_rc_addClonedServer_x10`/`x100`, whose log depends on the iteration
  order of an unordered PCM feature, i.e. on identity hashes (DESIGN §6.1).

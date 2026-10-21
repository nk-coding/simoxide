### Plain models: SimOxide's flavours against Slingshot

Warm per-run wall time, median over all measured runs of the processes (p10–p90 in brackets); SimOxide: `simoxide-compare` (load, compile, run, summaries per run); Slingshot: the S0 runner (`--stock`, seeded, no instrumentation or measurement output), each JVM's runs after a warm-up of a third of them (31 short, 8 long runs per JVM). Speed-up = Slingshot / SimOxide.

| model | len | sim. s | SimOxide Slingshot flavour (fast) | SimOxide Slingshot-exact | SimOxide SimuLizar flavour (exact) | SimOxide SimuLizar flavour (fast) | Slingshot (stock nightly, S0 runner) | Slingshot / Slingshot flavour | Slingshot / Slingshot-exact |
|---|---|---|---|---|---|---|---|---|---|
| `x_espresso` | short | 100 | 392 µs (352 µs–476 µs) | 1.53 ms (1.47 ms–1.61 ms) | 392 µs (360 µs–473 µs) | 410 µs (365 µs–492 µs) | 182 ms (166 ms–288 ms) | 464× | 119× |
| `x_espresso` | long | 2000 | 4.69 ms (4.58 ms–5 ms) | 26.5 ms (25.7 ms–28.4 ms) | 4.84 ms (4.72 ms–5.29 ms) | 4.94 ms (4.7 ms–5.33 ms) | 2.85 s (2.68 s–2.99 s) | 607× | 107× |
| `x_ss_mediastore` | short | 200000 | 1.18 ms (1.12 ms–1.27 ms) | 1.8 ms (1.73 ms–1.89 ms) | 1.22 ms (1.16 ms–1.3 ms) | 1.22 ms (1.15 ms–1.31 ms) | 267 ms (234 ms–376 ms) | 226× | 148× |
| `x_ss_mediastore` | long | 1e+07 | 7.02 ms (6.83 ms–7.92 ms) | 36.9 ms (35.6 ms–41 ms) | 7.52 ms (7.28 ms–8.33 ms) | 7.54 ms (7.18 ms–8.21 ms) | 9.02 s (8.68 s–9.26 s) | 1 285× | 244× |
| `x_ss_minimal` | short | 1000 | 274 µs (214 µs–346 µs) | 611 µs (560 µs–676 µs) | 242 µs (218 µs–322 µs) | 242 µs (214 µs–329 µs) | 134 ms (106 ms–237 ms) | 490× | 219× |
| `x_ss_minimal` | long | 100000 | 8.76 ms (8.39 ms–10.3 ms) | 42.7 ms (41.6 ms–47.4 ms) | 9.07 ms (8.88 ms–9.66 ms) | 9.38 ms (8.98 ms–12 ms) | 6.62 s (6.41 s–7.06 s) | 755× | 155× |

**Cold one-shot** (process start to results, median of the processes; SimOxide: `simoxide run` with `--measurements`; Slingshot: the JVM's uptime when the S0 runner starts, i.e. JVM and Equinox start-up, plus its first run) and **first run in the JVM** (Slingshot, excluding JVM and Equinox start-up), peak RSS of the warm processes:

| model | len | SimOxide Slingshot flavour (fast) | SimOxide Slingshot-exact | SimOxide SimuLizar flavour (exact) | SimOxide SimuLizar flavour (fast) | Slingshot (stock nightly, S0 runner) | Slingshot first run in JVM | peak RSS SimOxide (Slingshot flavour) / Slingshot |
|---|---|---|---|---|---|---|---|---|
| `x_espresso` | short | 13.2 ms | 10.8 ms | 10.3 ms | 10.1 ms | 3.36 s | 2.37 s | 6 MB / 886 MB |
| `x_espresso` | long | 37.7 ms | 66 ms | 40.7 ms | 38.9 ms | 6.5 s | 5.52 s | 9 MB / 1268 MB |
| `x_ss_mediastore` | short | 10.3 ms | 11.6 ms | 12.1 ms | 10.8 ms | 4.31 s | 3.33 s | 8 MB / 832 MB |
| `x_ss_mediastore` | long | 66.7 ms | 164 ms | 83.7 ms | 98.4 ms | 13.9 s | 12.9 s | 12 MB / 1303 MB |
| `x_ss_minimal` | short | 7.76 ms | 9.12 ms | 6.94 ms | 7.93 ms | 3.25 s | 2.28 s | 6 MB / 777 MB |
| `x_ss_minimal` | long | 102 ms | 110 ms | 106 ms | 86.8 ms | 10.5 s | 9.55 s | 12 MB / 1333 MB |

**Simulation only** (SimOxide: the run without loading and compiling, median `sim_ms` of the warm runs) against Slingshot's warm run (which includes its model loading, about 10 % of it):

| model | len | SimOxide Slingshot flavour (fast) | SimOxide Slingshot-exact | SimOxide SimuLizar flavour (exact) | SimOxide SimuLizar flavour (fast) |
|---|---|---|---|---|---|
| `x_espresso` | short | 194 µs | 1.28 ms | 201 µs | 204 µs |
| `x_espresso` | long | 4.09 ms | 25.5 ms | 4.23 ms | 4.32 ms |
| `x_ss_mediastore` | short | 113 µs | 717 µs | 127 µs | 120 µs |
| `x_ss_mediastore` | long | 4.87 ms | 34.5 ms | 5.24 ms | 5.24 ms |
| `x_ss_minimal` | short | 70 µs | 414 µs | 78 µs | 75 µs |
| `x_ss_minimal` | long | 7.39 ms | 41.2 ms | 7.77 ms | 7.93 ms |

**Results** (seed 1; usage-scenario requests and mean response time): SimOxide's runs per flavour and mode, and one Slingshot run with measurements (stock Slingshot draws branch decisions from `Math.random()`, so it is not reproducible per seed):

| model | len | SimOxide Slingshot flavour (fast) | SimOxide Slingshot-exact | SimOxide SimuLizar flavour (exact) | SimOxide SimuLizar flavour (fast) | Slingshot (stock nightly, S0 runner) |
|---|---|---|---|---|---|---|
| `x_espresso` | short | 990 / 3 | 990 / 3 | 990 / 3 | 990 / 3 | 990 / 3 |
| `x_espresso` | long | 19980 / 3 | 19980 / 3 | 19980 / 3 | 19980 / 3 | 19980 / 3 |
| `x_ss_mediastore` | short | 17 / 23116.9 | 18 / 20801.1 | 18 / 20801.1 | 17 / 23116.9 | 18 / 20801.1 |
| `x_ss_mediastore` | long | 938 / 21287.8 | 937 / 21303 | 937 / 21303 | 938 / 21287.8 | 937 / 21303 |
| `x_ss_minimal` | short | 451 / 1 | 453 / 1 | 453 / 1 | 451 / 1 | 453 / 1 |
| `x_ss_minimal` | long | 45422 / 1 | 45413 / 1 | 45413 / 1 | 45422 / 1 | 45413 / 1 |

### SPD workloads

Warm per-run wall time (median; p10–p90), SimOxide: `spd-bench` (simulation created, run with its scaling policies and QVTo adjustments, result; the QVTo core and `MainTransformation` are set up once per process), Slingshot: S0 runner, each JVM's runs after a warm-up of a third of them. adj = adjustments per run (median), in adj. = share of the run's wall time spent in adjustments (QVTo plus the change path).

| model | sim. s | Slingshot flavour (fast) | adj | in adj. | SimuLizar flavour (fast) | adj | in adj. | SimuLizar flavour (exact) | Slingshot | Slingshot / Slingshot flavour | sim. s/s Slingshot flavour | sim. s/s Slingshot |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| `spd_minimal` | 100 | 979 µs (924 µs–1.07 ms) | 1 | 88 % | 994 µs (941 µs–1.07 ms) | 1 | 88 % | 1.02 ms (940 µs–1.08 ms) | 133 ms (110 ms–195 ms) | 136× | 102 145 | 753 |
| `spd_same_instant` | 100 | 1.49 ms (1.41 ms–1.64 ms) | 2 | 94 % | 1.49 ms (1.4 ms–1.58 ms) | 2 | 94 % | 1.55 ms (1.46 ms–1.73 ms) | 138 ms (109 ms–199 ms) | 93× | 67 114 | 725 |
| `spd_scale_in` | 150 | 1.41 ms (1.33 ms–1.51 ms) | 2 | 93 % | 1.48 ms (1.38 ms–1.6 ms) | 2 | 91 % | 1.5 ms (1.41 ms–1.63 ms) | 136 ms (108 ms–200 ms) | 97× | 106 686 | 1 105 |
| `spd_service_group` | 100 | 2.61 ms (2.53 ms–2.76 ms) | 3 | 97 % | 2.67 ms (2.54 ms–2.91 ms) | 3 | 97 % | 2.62 ms (2.53 ms–2.8 ms) | 140 ms (115 ms–208 ms) | 54× | 38 263 | 712 |
| `spd_time_policies` | 100 | 2.67 ms (2.54 ms–2.84 ms) | 4 | 97 % | 2.7 ms (2.56 ms–2.86 ms) | 4 | 97 % | 2.67 ms (2.53 ms–3.01 ms) | 146 ms (114 ms–204 ms) | 55× | 37 425 | 685 |
| `spd_v4_cpu` | 600 | 1.69 ms (1.62 ms–1.77 ms) | 2 | 78 % | 256 µs (230 µs–292 µs) | 0 | 0 % | 304 µs (254 µs–352 µs) | 610 ms (465 ms–710 ms) | 362× | 355 977 | 983 |
| `spd_v4_rt` | 600 | 214 µs (199 µs–262 µs) | 0 | 0 % | 265 µs (251 µs–307 µs) | 0 | 0 % | 298 µs (265 µs–334 µs) | 528 ms (433 ms–707 ms) | 2 473× | 2 810 304 | 1 137 |

Per adjustment (SimOxide, QVTo `MainTransformation` plus the change path, mean over all measured runs), QVTo set-up per process, cold one-shot `simoxide run` (incl. QVTo set-up), Slingshot cold (JVM with Equinox, first run) and peak RSS:

| model | ms per adjustment, Slingshot flavour | ms per adjustment, SimuLizar flavour | QVTo set-up ms | cold `simoxide run` Slingshot flavour / Slingshot-exact / SimuLizar exact | Slingshot cold process | Slingshot first run in JVM | peak RSS SimOxide / Slingshot |
|---|---|---|---|---|---|---|---|
| `spd_minimal` | 0.874 | 0.885 | 12.8 | 22.6 ms / 25.6 ms / 20.1 ms | 3.26 s | 2.27 s | 15 MB / 639 MB |
| `spd_same_instant` | 0.718 | 0.710 | 13.8 | 28.4 ms / 33.1 ms / 27.6 ms | 3.29 s | 2.3 s | 15 MB / 634 MB |
| `spd_scale_in` | 0.663 | 0.717 | 12.5 | 27.3 ms / 28 ms / 27 ms | 3.4 s | 2.29 s | 15 MB / 655 MB |
| `spd_service_group` | 0.853 | 0.878 | 14.0 | 32.2 ms / 33.7 ms / 30.5 ms | 3.23 s | 2.26 s | 15 MB / 635 MB |
| `spd_time_policies` | 0.652 | 0.657 | 13.0 | 28.7 ms / 32.5 ms / 28.6 ms | 3.39 s | 2.4 s | 15 MB / 669 MB |
| `spd_v4_cpu` | 0.664 | 0.000 | 13.1 | 29.2 ms / 52.9 ms / 21.2 ms | 4.21 s | 3.21 s | 15 MB / 686 MB |
| `spd_v4_rt` | 0.000 | 0.000 | 13.8 | 21.2 ms / 34.8 ms / 29.7 ms | 3.93 s | 2.99 s | 13 MB / 693 MB |

**With stock Slingshot's quirks** (Slingshot flavour, fast, `quirks=fast spdQuirks=slingshot` as in `reference/slingshot/spd-stat.sh`: the decisions stock Slingshot makes; the fixed default above scales `spd_v4_cpu` 2 times in 600 s, Slingshot about 16 times):

| model | warm per run | adjustments (min–max) | in adj. | ms per adjustment | requests (min–max) | Slingshot / this |
|---|---|---|---|---|---|---|
| `spd_minimal` | 998 µs | 1–1 | 88 % | 0.894 | 33–33 | 133× |
| `spd_same_instant` | 2.65 ms | 4–4 | 96 % | 0.644 | 33–33 | 52× |
| `spd_scale_in` | 1.46 ms | 2–2 | 92 % | 0.679 | 18–19 | 93× |
| `spd_service_group` | 2.61 ms | 3–3 | 97 % | 0.875 | 33–33 | 54× |
| `spd_time_policies` | 4.12 ms | 8–8 | 98 % | 0.516 | 33–33 | 35× |
| `spd_v4_cpu` | 10.5 ms | 15–17 | 94 % | 0.626 | 455–502 | 58× |
| `spd_v4_rt` | 3.58 ms | 3–5 | 88 % | 0.717 | 464–492 | 147× |

**Results** (seed 1): SimOxide's adjustments and finished requests per run (all seeds: min–max), and Slingshot's adjustments in its SPD log (one instrumented run, unseedable branch decisions):

| model | Slingshot flavour: adjustments, requests | SimuLizar flavour (fast) | Slingshot: adjustments (SPD log), requests |
|---|---|---|---|
| `spd_minimal` | 1–1, 33–33 | 1–1, 33–33 | 1, 33 |
| `spd_same_instant` | 2–2, 33–33 | 2–2, 33–33 | 4, 33 |
| `spd_scale_in` | 2–2, 20–22 | 2–2, 20–22 | 2, 19 |
| `spd_service_group` | 3–3, 33–33 | 3–3, 33–33 | 3, 33 |
| `spd_time_policies` | 4–4, 33–33 | 4–4, 33–33 | 8, 33 |
| `spd_v4_cpu` | 2–2, 288–307 | 0–0, 284–307 | 16, 533 |
| `spd_v4_rt` | 0–0, 284–307 | 0–0, 284–307 | 2, 534 |

### Snapshots of SPD runs (halfway)

Median of 10 runs (`spd-bench snapshotAt`): fork = the run and its QVTo state in memory (`SpdRun::fork`, `QvtoAdjuster::fork_spd`); serialize = snapshot value and postcard bytes incl. the QVTo section; deserialize = bytes to `Snapshot`; restore = run and QVTo state from it; continue = the restored run to its end. forks/s = 1 / fork time.

| model | flavour, mode | fork µs | forks/s | serialize µs | bytes | deserialize µs | restore µs | continue | restored run ends as uninterrupted |
|---|---|---|---|---|---|---|---|---|---|
| `spd_minimal` | simulizar-exact | 3.5 | 281 690 | 89 | 5 746 | 8 | 74 | 929 µs | yes |
| `spd_minimal` | slingshot-fast | 3.0 | 327 869 | 84 | 2 741 | 6 | 74 | 966 µs | yes |
| `spd_same_instant` | simulizar-exact | 3.5 | 285 714 | 95 | 5 782 | 8 | 81 | 1.48 ms | yes |
| `spd_same_instant` | slingshot-fast | 4.0 | 253 165 | 91 | 2 781 | 7 | 80 | 1.39 ms | yes |
| `spd_scale_in` | simulizar-exact | 8.8 | 113 636 | 733 | 114 825 | 92 | 978 | 445 µs | yes |
| `spd_scale_in` | slingshot-fast | 7.0 | 141 844 | 664 | 111 064 | 86 | 875 | 401 µs | yes |
| `spd_service_group` | simulizar-exact | 18.8 | 53 333 | 750 | 128 006 | 100 | 980 | 753 µs | yes |
| `spd_service_group` | slingshot-fast | 18.8 | 53 191 | 780 | 125 299 | 97 | 1013 | 791 µs | yes |
| `spd_time_policies` | simulizar-exact | 16.4 | 60 976 | 728 | 117 391 | 93 | 966 | 1.61 ms | yes |
| `spd_time_policies` | slingshot-fast | 15.3 | 65 147 | 702 | 113 627 | 89 | 904 | 1.56 ms | yes |
| `spd_v4_cpu` | simulizar-exact | 8.2 | 121 212 | 133 | 22 209 | 19 | 94 | 145 µs | yes |
| `spd_v4_cpu` | slingshot-fast | 12.7 | 78 740 | 782 | 136 799 | 111 | 986 | 198 µs | yes |
| `spd_v4_rt` | simulizar-exact | 8.3 | 120 482 | 133 | 22 058 | 19 | 94 | 154 µs | yes |
| `spd_v4_rt` | slingshot-fast | 10.1 | 98 522 | 154 | 29 363 | 28 | 106 | 130 µs | yes |

Snapshot files through the CLI (exact mode, long run, pause halfway; process wall, median of the processes):

| model | uninterrupted run | run to the pause + write snapshot | snapshot bytes | `simoxide resume` to the end |
|---|---|---|---|---|
| `x_sl_mediastore` | 13.7 ms | 18.6 ms | 750 551 | 15.8 ms |
| `x_ss_mediastore` | 18.6 ms | 22.5 ms | 1 335 542 | 24 ms |

`simoxide-bench fork --reps 5 --mode exact` (plain models, halfway through the long run):

```text
mode exact, flavour simulizar, quirks none
| benchmark | events at fork | live processes | snapshot µs | detach µs | into_state + resume µs | tuples stored | snapshot µs (stored) | detach µs (stored) |
|---|---|---|---|---|---|---|---|---|
| mediastore | 409999 | 1 | 1.6 | 1.9 | 0.1 | 1110010 | 620.7 | 631.9 |
| espresso | 40015 | 30 | 2.8 | 7.9 | 0.1 | 60038 | 19.5 | 21.3 |
| h13 passive | 62944 | 6 | 0.6 | 1.2 | 0.1 | 170210 | 41.6 | 39.8 |
| fork (x_pem_fork) | 159997 | 1 | 0.5 | 0.7 | 0.1 | 150000 | 34.6 | 34.9 |
| fork sync (h11) | 149999 | 1 | 0.7 | 0.8 | 0.1 | 230002 | 55.7 | 53.6 |
| call chain (h14) | 110000 | 3 | 0.8 | 1.4 | 0.1 | 169999 | 39.6 | 40.4 |
| nested subsystem | 40001 | 1 | 0.5 | 0.6 | 0.1 | 60004 | 13.3 | 13.7 |
| StoEx dists (h21) | 75419 | 4 | 0.6 | 1.1 | 0.1 | 109378 | 25.5 | 26.1 |
| INNER coll. (h27) | 20017 | 3 | 0.4 | 1.3 | 0.1 | 26267 | 4.0 | 4.7 |
| PS 4 cores, ~20 jobs | 50007 | 117 | 11.3 | 28.7 | 0.1 | 79808 | 30.7 | 49.6 |
| generated s10 (#5) | 18017 | 3 | 0.7 | 1.4 | 0.1 | 31037 | 6.2 | 6.7 |
| generated s10 (#6) | 7513 | 16 | 2.6 | 10.5 | 0.1 | 19477 | 5.7 | 13.7 |
```

`simoxide-bench fork --reps 5 --mode fast` (plain models, halfway through the long run):

```text
mode fast, flavour simulizar, quirks none
| benchmark | events at fork | live processes | snapshot µs | detach µs | into_state + resume µs | tuples stored | snapshot µs (stored) | detach µs (stored) |
|---|---|---|---|---|---|---|---|---|
| mediastore | 409999 | 1 | 1.5 | 1.8 | 0.1 | 1110010 | 649.3 | 460.9 |
| espresso | 40015 | 30 | 2.8 | 7.6 | 0.1 | 60038 | 16.8 | 20.7 |
| h13 passive | 62922 | 6 | 0.5 | 1.0 | 0.1 | 170128 | 34.3 | 34.4 |
| fork (x_pem_fork) | 160000 | 5 | 0.5 | 0.7 | 0.1 | 150003 | 34.5 | 30.3 |
| fork sync (h11) | 149999 | 7 | 1.1 | 1.8 | 0.1 | 229998 | 57.6 | 46.9 |
| call chain (h14) | 109999 | 1 | 0.5 | 0.4 | 0.1 | 170003 | 36.4 | 34.3 |
| nested subsystem | 40001 | 1 | 0.3 | 0.4 | 0.1 | 60004 | 12.6 | 13.6 |
| StoEx dists (h21) | 75463 | 2 | 0.4 | 0.5 | 0.1 | 109446 | 25.8 | 25.7 |
| INNER coll. (h27) | 20046 | 3 | 0.3 | 1.1 | 0.1 | 26321 | 4.8 | 5.7 |
| PS 4 cores, ~20 jobs | 50022 | 54 | 5.6 | 13.0 | 0.1 | 79944 | 21.9 | 31.0 |
| generated s10 (#5) | 18017 | 3 | 0.5 | 1.3 | 0.1 | 31035 | 6.8 | 8.0 |
| generated s10 (#6) | 6713 | 11 | 2.5 | 7.7 | 0.1 | 17224 | 5.3 | 10.9 |
```

### SimuLizar reconfiguration rules

SimOxide: `simoxide run` per process (QVTo ordered mode unless stated): process wall, the run's wall (after loading and compiling), the time in rules (QVTo runs and the PRM mirror) and the number of reconfigurations; refsim: `refsim run --seeds 1:11` in one JVM, the first run cold, the others warm (stock SimuLizar cannot run the rules on the flat classpath: they need refsim's patch P9).

| case | simulator | process wall | run wall | reconfigurations | in rules ms (PRM ms) | refsim first / warm run |
|---|---|---|---|---|---|---|
| `lb_outsource` | refsim | 29.6 s | – | nan | nan (nan) | 7.59 s / 1.94 s |
| `lb_outsource` | simoxide-exact | 30 ms | 17.7 ms | 99 | 8.4 (3.1) | – / – |
| `lb_outsource` | simoxide-exact-compat | 26.4 ms | 18.5 ms | 99 | 8.9 (3.0) | – / – |
| `lb_outsource` | simoxide-fast | 25.8 ms | 17.5 ms | 99 | 9.1 (3.0) | – / – |
| `lb_rules_qvto` | refsim | 30.9 s | – | nan | nan (nan) | 7.74 s / 2.05 s |
| `lb_rules_qvto` | simoxide-exact | 114 ms | 107 ms | 99 | 94.8 (2.7) | – / – |
| `lb_rules_qvto` | simoxide-exact-compat | 124 ms | 114 ms | 99 | 95.5 (3.8) | – / – |
| `lb_rules_qvto` | simoxide-fast | 115 ms | 106 ms | 99 | 91.4 (3.0) | – / – |
| `power_rules_qvto` | refsim | 69.7 s | – | nan | nan (nan) | 12 s / 5.72 s |
| `power_rules_qvto` | simoxide-exact | 1.12 s | 1.11 s | 12945 | 1083.8 (99.5) | – / – |
| `power_rules_qvto` | simoxide-exact-compat | 1.1 s | 1.1 s | 12945 | 1071.3 (96.5) | – / – |
| `power_rules_qvto` | simoxide-fast | 1.08 s | 1.08 s | 12859 | 1053.4 (92.7) | – / – |

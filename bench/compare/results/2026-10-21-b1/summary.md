### Plain models: SimOxide's flavours against Slingshot

Warm per-run wall time, median over all measured runs of the processes (p10–p90 in brackets); SimOxide: `simoxide-compare` (load, compile, run, summaries per run); Slingshot: the S0 runner (`--stock`, seeded, no instrumentation or measurement output), each JVM's runs after a warm-up of a third of them (31 short, 8 long runs per JVM). Speed-up = Slingshot / SimOxide.

| model | len | sim. s | SimOxide Slingshot flavour (fast) | SimOxide Slingshot-exact | SimOxide SimuLizar flavour (exact) | SimOxide SimuLizar flavour (fast) | Slingshot (stock nightly, S0 runner) | Slingshot / Slingshot flavour | Slingshot / Slingshot-exact |
|---|---|---|---|---|---|---|---|---|---|
| `x_espresso` | short | 100 | 406 µs (345 µs–492 µs) | 1.53 ms (1.48 ms–1.62 ms) | 422 µs (365 µs–497 µs) | 425 µs (356 µs–509 µs) | 179 ms (162 ms–201 ms) | 442× | 117× |
| `x_espresso` | long | 2000 | 4.62 ms (4.52 ms–5.15 ms) | 25.8 ms (25.2 ms–26.5 ms) | 4.81 ms (4.7 ms–5.1 ms) | 4.8 ms (4.66 ms–5.23 ms) | 2.39 s (2.33 s–2.5 s) | 518× | 93× |
| `x_ss_mediastore` | short | 200000 | 1.21 ms (1.15 ms–1.3 ms) | 1.84 ms (1.76 ms–1.96 ms) | 1.24 ms (1.18 ms–1.33 ms) | 1.2 ms (1.14 ms–1.27 ms) | 249 ms (233 ms–269 ms) | 206× | 135× |
| `x_ss_mediastore` | long | 1e+07 | 7.25 ms (6.97 ms–7.65 ms) | 38.5 ms (36.3 ms–40.1 ms) | 7.41 ms (7.25 ms–7.89 ms) | 7.35 ms (7.22 ms–7.72 ms) | 7.78 s (7.58 s–8.97 s) | 1 073× | 202× |
| `x_ss_minimal` | short | 1000 | 239 µs (203 µs–314 µs) | 622 µs (549 µs–713 µs) | 306 µs (248 µs–373 µs) | 258 µs (213 µs–337 µs) | 123 ms (103 ms–157 ms) | 515× | 198× |
| `x_ss_minimal` | long | 100000 | 8.56 ms (8.39 ms–9.52 ms) | 42.1 ms (41.3 ms–43.2 ms) | 8.92 ms (8.47 ms–9.63 ms) | 8.94 ms (8.69 ms–9.63 ms) | 5.06 s (5.01 s–5.32 s) | 591× | 120× |

**Cold one-shot** (process start to results, median of the processes; SimOxide: `simoxide run` with `--measurements`; Slingshot: the JVM's uptime when the S0 runner starts, i.e. JVM and Equinox start-up, plus its first run) and **first run in the JVM** (Slingshot, excluding JVM and Equinox start-up), peak RSS of the warm processes:

| model | len | SimOxide Slingshot flavour (fast) | SimOxide Slingshot-exact | SimOxide SimuLizar flavour (exact) | SimOxide SimuLizar flavour (fast) | Slingshot (stock nightly, S0 runner) | Slingshot first run in JVM | peak RSS SimOxide (Slingshot flavour) / Slingshot |
|---|---|---|---|---|---|---|---|---|
| `x_espresso` | short | 5.96 ms | 6.5 ms | 5.28 ms | 5.05 ms | 2.98 s | 2.11 s | 6 MB / 900 MB |
| `x_espresso` | long | 26.7 ms | 46.2 ms | 24.4 ms | 25.4 ms | 5.42 s | 4.57 s | 9 MB / 1268 MB |
| `x_ss_mediastore` | short | 5.91 ms | 6.31 ms | 5.48 ms | 5.39 ms | 4.01 s | 3.1 s | 8 MB / 837 MB |
| `x_ss_mediastore` | long | 45.4 ms | 73 ms | 48.1 ms | 52.5 ms | 12.5 s | 11.6 s | 13 MB / 1275 MB |
| `x_ss_minimal` | short | 4.75 ms | 4.9 ms | 4.37 ms | 4.52 ms | 2.8 s | 1.93 s | 6 MB / 756 MB |
| `x_ss_minimal` | long | 68.5 ms | 79 ms | 87.5 ms | 62.2 ms | 8.68 s | 7.81 s | 12 MB / 1413 MB |

**Simulation only** (SimOxide: the run without loading and compiling, median `sim_ms` of the warm runs) against Slingshot's warm run (which includes its model loading, about 10 % of it):

| model | len | SimOxide Slingshot flavour (fast) | SimOxide Slingshot-exact | SimOxide SimuLizar flavour (exact) | SimOxide SimuLizar flavour (fast) |
|---|---|---|---|---|---|
| `x_espresso` | short | 201 µs | 1.29 ms | 223 µs | 216 µs |
| `x_espresso` | long | 4.06 ms | 24.9 ms | 4.26 ms | 4.23 ms |
| `x_ss_mediastore` | short | 112 µs | 736 µs | 128 µs | 120 µs |
| `x_ss_mediastore` | long | 4.98 ms | 35.9 ms | 5.19 ms | 5.11 ms |
| `x_ss_minimal` | short | 70 µs | 424 µs | 88 µs | 81 µs |
| `x_ss_minimal` | long | 7.29 ms | 40.8 ms | 7.64 ms | 7.7 ms |

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
| `spd_minimal` | 100 | 914 µs (858 µs–981 µs) | 1 | 87 % | 956 µs (892 µs–1.06 ms) | 1 | 88 % | 952 µs (884 µs–1.01 ms) | 130 ms (111 ms–196 ms) | 143× | 109 409 | 766 |
| `spd_same_instant` | 100 | 1.4 ms (1.32 ms–1.52 ms) | 2 | 94 % | 1.42 ms (1.35 ms–1.52 ms) | 2 | 93 % | 1.41 ms (1.34 ms–1.49 ms) | 145 ms (114 ms–203 ms) | 103× | 71 378 | 691 |
| `spd_scale_in` | 150 | 1.33 ms (1.27 ms–1.41 ms) | 2 | 92 % | 1.38 ms (1.31 ms–1.49 ms) | 2 | 92 % | 1.36 ms (1.29 ms–1.47 ms) | 139 ms (113 ms–210 ms) | 105× | 113 080 | 1 078 |
| `spd_service_group` | 100 | 2.65 ms (2.42 ms–2.84 ms) | 3 | 97 % | 2.49 ms (2.42 ms–2.7 ms) | 3 | 97 % | 2.54 ms (2.43 ms–2.67 ms) | 147 ms (123 ms–206 ms) | 56× | 37 722 | 679 |
| `spd_time_policies` | 100 | 2.61 ms (2.43 ms–2.75 ms) | 4 | 97 % | 2.56 ms (2.43 ms–2.79 ms) | 4 | 97 % | 2.55 ms (2.42 ms–2.78 ms) | 151 ms (112 ms–201 ms) | 58× | 38 329 | 661 |
| `spd_v4_cpu` | 600 | 10.6 ms (10.3 ms–11.3 ms) | 16 | 93 % | 6.15 ms (5.92 ms–6.83 ms) | 9 | 94 % | 6.08 ms (5.17 ms–6.91 ms) | 601 ms (474 ms–698 ms) | 57× | 56 652 | 999 |
| `spd_v4_rt` | 600 | 220 µs (199 µs–268 µs) | 0 | 0 % | 263 µs (241 µs–316 µs) | 0 | 0 % | 295 µs (266 µs–331 µs) | 506 ms (449 ms–687 ms) | 2 298× | 2 727 273 | 1 187 |

Per adjustment (SimOxide, QVTo `MainTransformation` plus the change path, mean over all measured runs), QVTo set-up per process, cold one-shot `simoxide run` (incl. QVTo set-up), Slingshot cold (JVM with Equinox, first run) and peak RSS:

| model | ms per adjustment, Slingshot flavour | ms per adjustment, SimuLizar flavour | QVTo set-up ms | cold `simoxide run` Slingshot flavour / Slingshot-exact / SimuLizar exact | Slingshot cold process | Slingshot first run in JVM | peak RSS SimOxide / Slingshot |
|---|---|---|---|---|---|---|---|
| `spd_minimal` | 0.814 | 1.005 | 10.7 | 33 ms / 30.8 ms / 30.1 ms | 3.37 s | 2.36 s | 15 MB / 658 MB |
| `spd_same_instant` | 0.661 | 0.666 | 14.0 | 31.1 ms / 34.3 ms / 31.3 ms | 3.46 s | 2.38 s | 15 MB / 654 MB |
| `spd_scale_in` | 0.614 | 0.687 | 14.5 | 31.6 ms / 30.2 ms / 29.8 ms | 3.32 s | 2.33 s | 15 MB / 634 MB |
| `spd_service_group` | 0.884 | 0.824 | 12.8 | 30.5 ms / 31.6 ms / 29.6 ms | 3.44 s | 2.42 s | 15 MB / 630 MB |
| `spd_time_policies` | 0.635 | 0.651 | 15.4 | 32.9 ms / 36.8 ms / 32.9 ms | 3.34 s | 2.34 s | 15 MB / 686 MB |
| `spd_v4_cpu` | 0.638 | 0.641 | 15.2 | 42.6 ms / 54.8 ms / 29.3 ms | 4.34 s | 3.37 s | 16 MB / 699 MB |
| `spd_v4_rt` | 0.000 | 0.000 | 14.2 | 25 ms / 37 ms / 21.1 ms | 4 s | 2.95 s | 12 MB / 719 MB |

**With stock Slingshot's quirks** (Slingshot flavour, fast, `quirks=fast spdQuirks=slingshot` as in `reference/slingshot/spd-stat.sh`: the decisions stock Slingshot makes, Slingshot itself scales `spd_v4_cpu` about 16 times in 600 s):

| model | warm per run | adjustments (min–max) | in adj. | ms per adjustment | requests (min–max) | Slingshot / this |
|---|---|---|---|---|---|---|
| `spd_minimal` | 921 µs | 1–1 | 87 % | 0.813 | 33–33 | 142× |
| `spd_same_instant` | 2.67 ms | 4–4 | 96 % | 0.644 | 33–33 | 54× |
| `spd_scale_in` | 1.36 ms | 2–2 | 91 % | 0.618 | 18–19 | 103× |
| `spd_service_group` | 2.54 ms | 3–3 | 95 % | 0.839 | 33–33 | 58× |
| `spd_time_policies` | 3.88 ms | 8–8 | 97 % | 0.482 | 33–33 | 39× |
| `spd_v4_cpu` | 10.3 ms | 15–17 | 93 % | 0.619 | 455–502 | 59× |
| `spd_v4_rt` | 3.44 ms | 3–5 | 88 % | 0.659 | 464–492 | 147× |

**Results** (seed 1): SimOxide's adjustments and finished requests per run (all seeds: min–max), and Slingshot's adjustments in its SPD log (one instrumented run, unseedable branch decisions):

| model | Slingshot flavour: adjustments, requests | SimuLizar flavour (fast) | Slingshot: adjustments (SPD log), requests |
|---|---|---|---|
| `spd_minimal` | 1–1, 33–33 | 1–1, 33–33 | 1, 33 |
| `spd_same_instant` | 2–2, 33–33 | 2–2, 33–33 | 4, 33 |
| `spd_scale_in` | 2–2, 20–22 | 2–2, 20–22 | 2, 19 |
| `spd_service_group` | 3–3, 33–33 | 3–3, 33–33 | 3, 33 |
| `spd_time_policies` | 4–4, 33–33 | 4–4, 33–33 | 8, 33 |
| `spd_v4_cpu` | 15–16, 455–496 | 9–10, 455–498 | 16, 533 |
| `spd_v4_rt` | 0–0, 284–307 | 0–0, 284–307 | 2, 534 |

### Snapshots of SPD runs (halfway)

Median of 10 runs (`spd-bench snapshotAt`): fork = the run and its QVTo state in memory (`SpdRun::fork`, `QvtoAdjuster::fork_spd`); serialize = snapshot value and postcard bytes incl. the QVTo section; deserialize = bytes to `Snapshot`; restore = run and QVTo state from it; continue = the restored run to its end. forks/s = 1 / fork time.

| model | flavour, mode | fork µs | forks/s | serialize µs | bytes | deserialize µs | restore µs | continue | restored run ends as uninterrupted |
|---|---|---|---|---|---|---|---|---|---|
| `spd_minimal` | simulizar-exact | 3.9 | 256 410 | 93 | 5 746 | 9 | 78 | 904 µs | yes |
| `spd_minimal` | slingshot-fast | 2.9 | 344 828 | 82 | 2 741 | 6 | 72 | 885 µs | yes |
| `spd_same_instant` | simulizar-exact | 4.2 | 235 294 | 94 | 5 782 | 10 | 82 | 1.34 ms | yes |
| `spd_same_instant` | slingshot-fast | 3.1 | 322 581 | 86 | 2 781 | 6 | 74 | 1.32 ms | yes |
| `spd_scale_in` | simulizar-exact | 7.9 | 126 582 | 691 | 114 825 | 89 | 825 | 425 µs | yes |
| `spd_scale_in` | slingshot-fast | 8.6 | 115 607 | 699 | 111 064 | 89 | 861 | 452 µs | yes |
| `spd_service_group` | simulizar-exact | 18.4 | 54 201 | 760 | 128 006 | 99 | 908 | 760 µs | yes |
| `spd_service_group` | slingshot-fast | 19.9 | 50 251 | 809 | 125 299 | 100 | 982 | 814 µs | yes |
| `spd_time_policies` | simulizar-exact | 17.5 | 57 143 | 759 | 117 391 | 95 | 936 | 1.69 ms | yes |
| `spd_time_policies` | slingshot-fast | 15.3 | 65 359 | 733 | 113 627 | 89 | 866 | 1.6 ms | yes |
| `spd_v4_cpu` | simulizar-exact | 24.6 | 40 568 | 789 | 134 601 | 107 | 994 | 3.54 ms | yes |
| `spd_v4_cpu` | slingshot-fast | 8.6 | 115 607 | 838 | 151 636 | 141 | 1037 | 6.21 ms | yes |
| `spd_v4_rt` | simulizar-exact | 6.8 | 147 059 | 134 | 22 058 | 18 | 95 | 148 µs | yes |
| `spd_v4_rt` | slingshot-fast | 10.7 | 93 458 | 154 | 29 363 | 26 | 100 | 140 µs | yes |

Snapshot files through the CLI (exact mode, long run, pause halfway; process wall, median of the processes):

| model | uninterrupted run | run to the pause + write snapshot | snapshot bytes | `simoxide resume` to the end |
|---|---|---|---|---|
| `x_sl_mediastore` | 15.6 ms | 18.1 ms | 750 551 | 18.3 ms |
| `x_ss_mediastore` | 18.9 ms | 19.3 ms | 1 335 542 | 22.6 ms |

`simoxide-bench fork --reps 5 --mode exact` (plain models, halfway through the long run):

```text
mode exact, flavour simulizar, quirks none
| benchmark | events at fork | live processes | snapshot µs | detach µs | into_state + resume µs | tuples stored | snapshot µs (stored) | detach µs (stored) |
|---|---|---|---|---|---|---|---|---|
| mediastore | 409999 | 1 | 1.6 | 1.9 | 0.1 | 1110010 | 611.8 | 495.9 |
| espresso | 40015 | 30 | 2.9 | 7.7 | 0.1 | 60038 | 15.9 | 21.6 |
| h13 passive | 62944 | 6 | 0.6 | 1.3 | 0.1 | 170210 | 40.5 | 40.2 |
| fork (x_pem_fork) | 159997 | 1 | 0.5 | 0.7 | 0.1 | 150000 | 36.7 | 35.1 |
| fork sync (h11) | 149999 | 1 | 0.7 | 1.0 | 0.1 | 230002 | 54.0 | 54.4 |
| call chain (h14) | 110000 | 3 | 0.6 | 1.5 | 0.1 | 169999 | 39.5 | 40.9 |
| nested subsystem | 40001 | 1 | 0.5 | 0.7 | 0.1 | 60004 | 16.4 | 13.5 |
| StoEx dists (h21) | 75419 | 4 | 0.5 | 1.1 | 0.1 | 109378 | 26.1 | 26.2 |
| INNER coll. (h27) | 20017 | 3 | 0.4 | 1.4 | 0.1 | 26267 | 4.6 | 5.5 |
| PS 4 cores, ~20 jobs | 50007 | 117 | 11.4 | 30.5 | 0.1 | 79808 | 31.3 | 49.7 |
| generated s10 (#5) | 18017 | 3 | 0.7 | 1.4 | 0.1 | 31037 | 6.3 | 6.8 |
| generated s10 (#6) | 7513 | 16 | 2.6 | 10.4 | 0.1 | 19477 | 6.0 | 13.3 |
```

`simoxide-bench fork --reps 5 --mode fast` (plain models, halfway through the long run):

```text
mode fast, flavour simulizar, quirks none
| benchmark | events at fork | live processes | snapshot µs | detach µs | into_state + resume µs | tuples stored | snapshot µs (stored) | detach µs (stored) |
|---|---|---|---|---|---|---|---|---|
| mediastore | 409999 | 1 | 1.5 | 1.8 | 0.1 | 1110010 | 628.3 | 580.9 |
| espresso | 40015 | 30 | 2.5 | 7.4 | 0.1 | 60038 | 16.5 | 21.1 |
| h13 passive | 62922 | 6 | 0.5 | 1.1 | 0.1 | 170128 | 49.2 | 39.8 |
| fork (x_pem_fork) | 160000 | 5 | 0.5 | 0.7 | 0.1 | 150003 | 35.0 | 35.4 |
| fork sync (h11) | 149999 | 7 | 1.0 | 2.1 | 0.1 | 229998 | 55.2 | 55.8 |
| call chain (h14) | 109999 | 1 | 0.4 | 0.5 | 0.1 | 170003 | 39.0 | 38.7 |
| nested subsystem | 40001 | 1 | 0.3 | 0.4 | 0.1 | 60004 | 13.4 | 13.7 |
| StoEx dists (h21) | 75463 | 2 | 0.3 | 0.6 | 0.1 | 109446 | 26.2 | 25.9 |
| INNER coll. (h27) | 20046 | 3 | 0.3 | 1.1 | 0.1 | 26321 | 4.3 | 6.8 |
| PS 4 cores, ~20 jobs | 50022 | 54 | 4.7 | 13.3 | 0.1 | 79944 | 25.3 | 31.3 |
| generated s10 (#5) | 18017 | 3 | 0.5 | 1.3 | 0.1 | 31035 | 6.5 | 7.2 |
| generated s10 (#6) | 6713 | 11 | 2.5 | 7.4 | 0.1 | 17224 | 5.0 | 10.5 |
```

### SimuLizar reconfiguration rules

SimOxide: `simoxide run` per process (QVTo ordered mode unless stated): process wall, the run's wall (after loading and compiling), the time in rules (QVTo runs and the PRM mirror) and the number of reconfigurations; refsim: `refsim run --seeds 1:11` in one JVM, the first run cold, the others warm (stock SimuLizar cannot run the rules on the flat classpath: they need refsim's patch P9).

| case | simulator | process wall | run wall | reconfigurations | in rules ms (PRM ms) | refsim first / warm run |
|---|---|---|---|---|---|---|
| `lb_outsource` | refsim | 29.2 s | – | nan | nan (nan) | 7.35 s / 1.94 s |
| `lb_outsource` | simoxide-exact | 26 ms | 17.6 ms | 99 | 8.2 (2.6) | – / – |
| `lb_outsource` | simoxide-exact-compat | 27.7 ms | 18.6 ms | 99 | 8.7 (2.8) | – / – |
| `lb_outsource` | simoxide-fast | 29 ms | 19.2 ms | 99 | 8.6 (3.1) | – / – |
| `lb_rules_qvto` | refsim | 31.1 s | – | nan | nan (nan) | 7.87 s / 2.06 s |
| `lb_rules_qvto` | simoxide-exact | 112 ms | 102 ms | 99 | 88.4 (3.1) | – / – |
| `lb_rules_qvto` | simoxide-exact-compat | 115 ms | 105 ms | 99 | 90.0 (2.9) | – / – |
| `lb_rules_qvto` | simoxide-fast | 110 ms | 101 ms | 99 | 88.2 (2.8) | – / – |
| `power_rules_qvto` | refsim | 81.7 s | – | nan | nan (nan) | 13.1 s / 6.69 s |
| `power_rules_qvto` | simoxide-exact | 1.12 s | 1.11 s | 12945 | 1067.0 (98.9) | – / – |
| `power_rules_qvto` | simoxide-exact-compat | 1.18 s | 1.17 s | 12945 | 1132.6 (101.3) | – / – |
| `power_rules_qvto` | simoxide-fast | 1.17 s | 1.15 s | 12859 | 1108.6 (103.9) | – / – |

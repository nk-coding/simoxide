### `simoxide-bench suite --reps 3`, mode/flavour exact: median of 3 invocations

| benchmark | load µs | compile µs | short: events | short µs/run | short allocs | long: events | long ms/run | Mev/s (count) | Mev/s (store) | M requests/s | allocs/event | peak heap KiB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mediastore | 749 | 193 | 409 | 39.5 | 280 | 819999 | 61.2 | 13.4 | 11.08 | 0.327 | 0 | 23 |
| espresso | 114 | 6 | 230 | 21.3 | 318 | 80030 | 3.8 | 21.3 | 20.59 | 5.332 | 0.004 | 24 |
| h13 passive | 146 | 9 | 508 | 33.8 | 202 | 125888 | 7 | 18.1 | 17.22 | 2.877 | 0.047 | 10 |
| fork (x_pem_fork) | 151 | 10 | 801 | 31.1 | 130 | 319995 | 10.8 | 29.71 | 28.62 | 1.857 | 0 | 8 |
| fork sync (h11) | 145 | 9 | 749 | 34.8 | 165 | 299999 | 11.7 | 25.73 | 24.75 | 1.715 | 0.001 | 26 |
| call chain (h14) | 196 | 16 | 559 | 37.4 | 164 | 219999 | 12.5 | 17.66 | 16.91 | 1.605 | 0.001 | 15 |
| nested subsystem | 172 | 11 | 201 | 12.1 | 72 | 80001 | 3.3 | 24.53 | 23.36 | 6.134 | 0.001 | 6 |
| StoEx dists (h21) | 159 | 27 | 932 | 137.6 | 88 | 150839 | 24.2 | 6.22 | 6.09 | 0.206 | 0.001 | 11 |
| INNER coll. (h27) | 157 | 16 | 237 | 29.1 | 178 | 40035 | 4.6 | 8.62 | 8.44 | 1.076 | 0.377 | 11 |
| PS 4 cores, ~20 jobs | 138 | 8 | 526 | 33.8 | 220 | 100014 | 6.1 | 16.35 | 15.73 | 3.27 | 0.02 | 129 |
| generated s10 (#5) | 459 | 65 | 343 | 33.8 | 219 | 36033 | 3.1 | 11.71 | 11.46 | 0.651 | 0.114 | 13 |
| generated s10 (#6) | 505 | 87 | 3084 | 532.6 | 1139 | 15026 | 2.3 | 6.61 | 6.42 | 0.045 | 0.153 | 76 |

### `simoxide-bench suite --reps 3`, mode/flavour fast: median of 3 invocations

| benchmark | load µs | compile µs | short: events | short µs/run | short allocs | long: events | long ms/run | Mev/s (count) | Mev/s (store) | M requests/s | allocs/event | peak heap KiB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mediastore | 740 | 193 | 409 | 36.6 | 277 | 819999 | 61.1 | 13.42 | 11.25 | 0.327 | 0 | 20 |
| espresso | 116 | 6 | 230 | 19.5 | 315 | 80030 | 3.8 | 21.14 | 20.8 | 5.29 | 0.004 | 21 |
| h13 passive | 142 | 9 | 505 | 31.3 | 200 | 125842 | 7.1 | 17.83 | 16.91 | 2.834 | 0.047 | 8 |
| fork (x_pem_fork) | 151 | 10 | 801 | 28.9 | 127 | 320001 | 10.8 | 29.61 | 28.64 | 1.851 | 0 | 6 |
| fork sync (h11) | 144 | 9 | 749 | 31.8 | 179 | 299999 | 11.3 | 26.66 | 25.02 | 1.778 | 0.001 | 15 |
| call chain (h14) | 200 | 17 | 549 | 32.8 | 165 | 219999 | 12.4 | 17.7 | 16.89 | 1.609 | 0.001 | 12 |
| nested subsystem | 174 | 11 | 201 | 9.3 | 69 | 80001 | 3.2 | 25.1 | 23.13 | 6.275 | 0.001 | 3 |
| StoEx dists (h21) | 153 | 25 | 907 | 42.5 | 113 | 150927 | 7.2 | 21.08 | 20.06 | 0.698 | 0.001 | 10 |
| INNER coll. (h27) | 150 | 15 | 231 | 21.5 | 175 | 40091 | 3.6 | 11.13 | 10.93 | 1.388 | 0.377 | 10 |
| PS 4 cores, ~20 jobs | 144 | 9 | 523 | 30.2 | 227 | 100044 | 5.4 | 18.65 | 17.94 | 3.732 | 0.012 | 55 |
| generated s10 (#5) | 444 | 60 | 359 | 27.8 | 214 | 36035 | 2.5 | 14.64 | 14.2 | 0.814 | 0.114 | 10 |
| generated s10 (#6) | 503 | 90 | 3249 | 299.1 | 1140 | 13426 | 1 | 12.98 | 11.83 | 0.101 | 0.146 | 74 |

### `simoxide-bench suite --reps 3`, mode/flavour slingshot-exact: median of 3 invocations

| benchmark | load µs | compile µs | short: events | short µs/run | short allocs | long: events | long ms/run | Mev/s (count) | Mev/s (store) | M requests/s | allocs/event | peak heap KiB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mediastore | 721 | 176 | 409 | 37.4 | 250 | 819999 | 58.6 | 13.99 | 11.57 | 0.341 | 0 | 19 |
| espresso | 130 | 7 | 230 | 19.8 | 306 | 80030 | 3.5 | 22.64 | 22.05 | 5.658 | 0.003 | 23 |
| h13 passive | 141 | 8 | 508 | 32.6 | 182 | 125888 | 6.8 | 18.5 | 17.6 | 2.939 | 0.047 | 10 |
| fork (x_pem_fork) | 156 | 9 | 801 | 30.7 | 130 | 319995 | 10.6 | 30.28 | 28.67 | 1.893 | 0 | 8 |
| fork sync (h11) | 145 | 9 | 749 | 35.4 | 157 | 299999 | 11.8 | 25.36 | 24.32 | 1.691 | 0.001 | 25 |
| call chain (h14) | 208 | 19 | 963 | 41.9 | 164 | 379999 | 14.4 | 26.36 | 25.2 | 1.387 | 0 | 16 |
| nested subsystem | 167 | 9 | 201 | 11.5 | 69 | 80001 | 3.1 | 25.71 | 24.28 | 6.428 | 0.001 | 6 |
| StoEx dists (h21) | 175 | 29 | 954 | 137.6 | 87 | 154999 | 24.5 | 6.33 | 6.27 | 0.204 | 0.001 | 11 |
| INNER coll. (h27) | 152 | 15 | 237 | 30 | 177 | 40035 | 4.8 | 8.4 | 8.26 | 1.049 | 0.377 | 11 |
| PS 4 cores, ~20 jobs | 130 | 8 | 526 | 34.7 | 217 | 100014 | 6.3 | 15.8 | 14.69 | 3.159 | 0.02 | 128 |
| generated s10 (#5) | 460 | 68 | 343 | 30 | 217 | 36035 | 2.8 | 12.7 | 12.37 | 0.705 | 0.114 | 13 |

### `simoxide-bench suite --reps 3`, mode/flavour slingshot-fast: median of 3 invocations

| benchmark | load µs | compile µs | short: events | short µs/run | short allocs | long: events | long ms/run | Mev/s (count) | Mev/s (store) | M requests/s | allocs/event | peak heap KiB |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| mediastore | 773 | 188 | 409 | 33.9 | 247 | 819999 | 57.2 | 14.33 | 11.9 | 0.35 | 0 | 17 |
| espresso | 119 | 6 | 230 | 17 | 303 | 80030 | 3.4 | 23.7 | 22.42 | 5.924 | 0.003 | 20 |
| h13 passive | 145 | 9 | 505 | 27.8 | 180 | 125842 | 6.4 | 19.72 | 18.45 | 3.135 | 0.047 | 7 |
| fork (x_pem_fork) | 152 | 9 | 801 | 27.9 | 127 | 320001 | 10.8 | 29.75 | 28.75 | 1.859 | 0 | 6 |
| fork sync (h11) | 141 | 8 | 749 | 30.2 | 170 | 299999 | 10.9 | 27.56 | 25.98 | 1.837 | 0.001 | 14 |
| call chain (h14) | 200 | 17 | 949 | 35.8 | 165 | 379999 | 13.7 | 27.75 | 26.8 | 1.46 | 0 | 14 |
| nested subsystem | 168 | 8 | 201 | 8.5 | 66 | 80001 | 3 | 26.93 | 25.78 | 6.733 | 0 | 3 |
| StoEx dists (h21) | 156 | 26 | 929 | 40.7 | 112 | 154999 | 6.8 | 22.88 | 21.91 | 0.738 | 0.001 | 10 |
| INNER coll. (h27) | 160 | 15 | 231 | 21.4 | 174 | 40091 | 3.6 | 11.19 | 10.76 | 1.395 | 0.377 | 10 |
| PS 4 cores, ~20 jobs | 130 | 7 | 523 | 29.9 | 223 | 100044 | 5.4 | 18.69 | 17.37 | 3.736 | 0.012 | 55 |
| generated s10 (#5) | 465 | 66 | 359 | 24.3 | 215 | 36035 | 2.2 | 16.39 | 15.78 | 0.909 | 0.114 | 11 |

### Fast vs exact (requests/s, SimuLizar flavour)

| benchmark | exact M requests/s | fast M requests/s | fast vs exact |
|---|---|---|---|
| mediastore | 0.327 | 0.327 | 1.00x |
| espresso | 5.33 | 5.29 | 0.99x |
| h13 passive | 2.88 | 2.83 | 0.99x |
| fork (x_pem_fork) | 1.86 | 1.85 | 1.00x |
| fork sync (h11) | 1.72 | 1.78 | 1.04x |
| call chain (h14) | 1.6 | 1.61 | 1.00x |
| nested subsystem | 6.13 | 6.28 | 1.02x |
| StoEx dists (h21) | 0.206 | 0.698 | 3.39x |
| INNER coll. (h27) | 1.08 | 1.39 | 1.29x |
| PS 4 cores, ~20 jobs | 3.27 | 3.73 | 1.14x |
| generated s10 (#5) | 0.651 | 0.814 | 1.25x |
| generated s10 (#6) | 0.045 | 0.101 | 2.24x |

### Slingshot flavour vs SimuLizar flavour, exact RNG mode (requests/s)

| benchmark | SimuLizar | Slingshot | ratio |
|---|---|---|---|
| mediastore | 0.327 | 0.341 | 1.04 |
| espresso | 5.332 | 5.658 | 1.06 |
| h13 passive | 2.877 | 2.939 | 1.02 |
| fork (x_pem_fork) | 1.857 | 1.893 | 1.02 |
| fork sync (h11) | 1.715 | 1.691 | 0.99 |
| call chain (h14) | 1.605 | 1.387 | 0.86 |
| nested subsystem | 6.134 | 6.428 | 1.05 |
| StoEx dists (h21) | 0.206 | 0.204 | 0.99 |
| INNER coll. (h27) | 1.076 | 1.049 | 0.97 |
| PS 4 cores, ~20 jobs | 3.27 | 3.159 | 0.97 |
| generated s10 (#5) | 0.651 | 0.705 | 1.08 |

geomean 1.004, range 0.86 to 1.08 (11 models)

### Slingshot flavour vs SimuLizar flavour, fast RNG mode (requests/s)

| benchmark | SimuLizar | Slingshot | ratio |
|---|---|---|---|
| mediastore | 0.327 | 0.35 | 1.07 |
| espresso | 5.29 | 5.924 | 1.12 |
| h13 passive | 2.834 | 3.135 | 1.11 |
| fork (x_pem_fork) | 1.851 | 1.859 | 1.00 |
| fork sync (h11) | 1.778 | 1.837 | 1.03 |
| call chain (h14) | 1.609 | 1.46 | 0.91 |
| nested subsystem | 6.275 | 6.733 | 1.07 |
| StoEx dists (h21) | 0.698 | 0.738 | 1.06 |
| INNER coll. (h27) | 1.388 | 1.395 | 1.01 |
| PS 4 cores, ~20 jobs | 3.732 | 3.736 | 1.00 |
| generated s10 (#5) | 0.814 | 0.909 | 1.12 |

geomean 1.043, range 0.91 to 1.12 (11 models)

### batch-exact

```text
model corpus/x_sl_mediastore (176 runs, 20000 measurements, mode exact)
| threads | wall s | runs/s | Mev/s | M requests/s | speed-up | efficiency |
|---|---|---|---|---|---|---|
| 1 | 10.937 | 16.1 | 13.2 | 0.322 | 1.00 | 100% |
| 2 | 5.648 | 31.2 | 25.6 | 0.623 | 1.94 | 97% |
| 4 | 2.969 | 59.3 | 48.6 | 1.186 | 3.68 | 92% |
| 8 | 1.553 | 113.4 | 93.0 | 2.267 | 7.04 | 88% |
| 16 | 0.939 | 187.5 | 153.7 | 3.749 | 11.65 | 73% |
| 22 | 0.867 | 203.0 | 166.5 | 4.061 | 12.62 | 57% |
| 25 | 0.810 | 217.2 | 178.1 | 4.344 | 13.50 | 54% |
```

### batch-fast

```text
model corpus/x_sl_mediastore (176 runs, 20000 measurements, mode fast)
| threads | wall s | runs/s | Mev/s | M requests/s | speed-up | efficiency |
|---|---|---|---|---|---|---|
| 1 | 10.692 | 16.5 | 13.5 | 0.329 | 1.00 | 100% |
| 2 | 5.441 | 32.3 | 26.5 | 0.647 | 1.97 | 98% |
| 4 | 2.930 | 60.1 | 49.3 | 1.201 | 3.65 | 91% |
| 8 | 1.525 | 115.4 | 94.6 | 2.308 | 7.01 | 88% |
| 16 | 0.916 | 192.1 | 157.6 | 3.843 | 11.67 | 73% |
| 22 | 1.000 | 176.0 | 144.3 | 3.520 | 10.69 | 49% |
| 25 | 0.791 | 222.6 | 182.5 | 4.452 | 13.52 | 54% |
```

### fork-exact

```text
mode exact, flavour simulizar, quirks none
| benchmark | events at fork | live processes | snapshot µs | detach µs | into_state + resume µs | tuples stored | snapshot µs (stored) | detach µs (stored) |
|---|---|---|---|---|---|---|---|---|
| mediastore | 409999 | 1 | 1.7 | 1.6 | 0.1 | 1110010 | 667.4 | 627.3 |
| espresso | 40015 | 30 | 2.9 | 7.6 | 0.1 | 60038 | 16.2 | 21.1 |
| h13 passive | 62944 | 6 | 0.7 | 1.4 | 0.1 | 170210 | 47.8 | 40.2 |
| fork (x_pem_fork) | 159997 | 1 | 0.5 | 0.7 | 0.1 | 150000 | 38.9 | 35.3 |
| fork sync (h11) | 149999 | 1 | 0.8 | 0.9 | 0.1 | 230002 | 64.6 | 53.3 |
| call chain (h14) | 110000 | 3 | 0.6 | 1.4 | 0.1 | 169999 | 41.0 | 40.3 |
| nested subsystem | 40001 | 1 | 0.4 | 0.6 | 0.1 | 60004 | 13.5 | 13.7 |
| StoEx dists (h21) | 75419 | 4 | 0.5 | 1.1 | 0.1 | 109378 | 26.7 | 26.3 |
| INNER coll. (h27) | 20017 | 3 | 0.4 | 1.4 | 0.1 | 26267 | 4.4 | 5.4 |
| PS 4 cores, ~20 jobs | 50007 | 117 | 11.5 | 30.7 | 0.1 | 79808 | 30.4 | 50.0 |
| generated s10 (#5) | 18017 | 3 | 0.8 | 1.4 | 0.1 | 31037 | 7.0 | 7.5 |
| generated s10 (#6) | 7513 | 16 | 2.6 | 10.5 | 0.1 | 19477 | 6.1 | 13.3 |
```

### fork-fast

```text
mode fast, flavour simulizar, quirks none
| benchmark | events at fork | live processes | snapshot µs | detach µs | into_state + resume µs | tuples stored | snapshot µs (stored) | detach µs (stored) |
|---|---|---|---|---|---|---|---|---|
| mediastore | 409999 | 1 | 1.5 | 1.9 | 0.1 | 1110010 | 664.1 | 648.3 |
| espresso | 40015 | 30 | 2.6 | 7.5 | 0.1 | 60038 | 16.9 | 21.9 |
| h13 passive | 62922 | 6 | 0.5 | 1.1 | 0.1 | 170128 | 39.1 | 40.2 |
| fork (x_pem_fork) | 160000 | 5 | 0.5 | 0.7 | 0.1 | 150003 | 36.1 | 35.1 |
| fork sync (h11) | 149999 | 7 | 0.9 | 2.0 | 0.1 | 229998 | 54.8 | 54.8 |
| call chain (h14) | 109999 | 1 | 0.4 | 0.5 | 0.1 | 170003 | 33.7 | 33.8 |
| nested subsystem | 40001 | 1 | 0.3 | 0.4 | 0.1 | 60004 | 13.4 | 13.8 |
| StoEx dists (h21) | 75463 | 2 | 0.3 | 0.6 | 0.1 | 109446 | 25.7 | 26.0 |
| INNER coll. (h27) | 20046 | 3 | 0.3 | 1.1 | 0.1 | 26321 | 4.6 | 5.0 |
| PS 4 cores, ~20 jobs | 50022 | 54 | 4.8 | 12.9 | 0.1 | 79944 | 21.5 | 31.4 |
| generated s10 (#5) | 18017 | 3 | 0.5 | 1.3 | 0.1 | 31035 | 6.1 | 7.3 |
| generated s10 (#6) | 6713 | 11 | 2.4 | 7.2 | 0.1 | 17224 | 5.0 | 9.7 |
```

### change-exact

```text
mode exact, flavour simulizar, quirks none
| benchmark | graph objects | graph copy µs | unchanged-graph change set µs | rate change µs |
|---|---|---|---|---|
| mediastore | 1220 | 116.3 | 322.6 | 333.2 |
| espresso | 158 | 17.1 | 28.1 | 27.9 |
| h13 passive | 187 | 19.0 | 47.1 | 33.5 |
| fork (x_pem_fork) | 212 | 22.3 | 37.9 | 36.3 |
| fork sync (h11) | 185 | 19.6 | 36.6 | 33.5 |
| call chain (h14) | 273 | 26.1 | 56.3 | 47.5 |
| nested subsystem | 207 | 20.8 | 39.3 | 39.2 |
| StoEx dists (h21) | 210 | 28.1 | 57.2 | 50.6 |
| INNER coll. (h27) | 196 | 19.0 | 36.0 | 33.1 |
| PS 4 cores, ~20 jobs | 179 | 17.1 | 32.0 | 32.5 |
| generated s10 (#5) | 763 | 54.7 | 127.2 | 116.8 |
| generated s10 (#6) | 974 | 67.0 | 143.5 | 140.1 |
```

### change-fast

```text
mode fast, flavour simulizar, quirks none
| benchmark | graph objects | graph copy µs | unchanged-graph change set µs | rate change µs |
|---|---|---|---|---|
| mediastore | 1220 | 81.2 | 327.8 | 305.9 |
| espresso | 158 | 18.0 | 42.7 | 32.7 |
| h13 passive | 187 | 19.6 | 35.2 | 34.0 |
| fork (x_pem_fork) | 212 | 21.6 | 50.6 | 41.1 |
| fork sync (h11) | 185 | 20.8 | 36.2 | 36.1 |
| call chain (h14) | 273 | 27.2 | 58.0 | 46.6 |
| nested subsystem | 207 | 21.8 | 42.0 | 38.8 |
| StoEx dists (h21) | 210 | 19.9 | 36.3 | 35.5 |
| INNER coll. (h27) | 196 | 19.9 | 34.3 | 32.2 |
| PS 4 cores, ~20 jobs | 179 | 18.5 | 32.8 | 31.7 |
| generated s10 (#5) | 763 | 63.3 | 120.8 | 124.6 |
| generated s10 (#6) | 974 | 72.8 | 153.3 | 140.3 |
```

### load (`load-bench bench --reps 300`, best of 300; median of 3 invocations), µs

| model | µs |
|---|---|
| x_sl_mediastore | 647.0 |
| x_pem_screencast_ms | 308.8 |
| h01_ps_single | 75.6 |
| gen_s6 | 418.5 |

### load-memory (`load-bench bench --reps 300`, best of 300; median of 3 invocations), µs

| model | µs |
|---|---|
| x_sl_mediastore | 612.8 |
| x_pem_screencast_ms | 286.2 |
| h01_ps_single | 58.5 |
| gen_s6 | 398.3 |

### load-cache (`load-bench bench --reps 300`, best of 300; median of 3 invocations), µs

| model | µs |
|---|---|
| x_sl_mediastore | 258.4 |
| x_pem_screencast_ms | 144.0 |
| h01_ps_single | 53.4 |
| gen_s6 | 183.9 |

### host-costs (`tests/host.rs costs`, 3 runs)

```text
test costs ... core 5.194129ms, compile 1.241716ms, per-run state (session clone, graph copy) 2.060117ms (graph copy 8.11µs), fork of the state 69ns, rule run 358 µs, change 543 µs
test costs ... core 2.620385ms, compile 400.236µs, per-run state (session clone, graph copy) 859.822µs (graph copy 7.85µs), fork of the state 60ns, rule run 177 µs, change 353 µs
test costs ... core 2.842998ms, compile 364.525µs, per-run state (session clone, graph copy) 741.601µs (graph copy 6.74µs), fork of the state 60ns, rule run 146 µs, change 292 µs
```

### HPA scaling-step cost (`tests/hpa.rs cost_per_scaling_step`, fast mode, 3 runs)

```text
test cost_per_scaling_step ... QVTo set-up (core, MainTransformation): 14.5 ms
k=1: 104 objects (graph before QVTo), 3 adjustments (5 pods created), 2.64 ms per adjustment, change path alone 0.43 ms; run 0.01 s wall, events 12844
k=10: 779 objects (graph before QVTo), 36 adjustments (52 pods created), 1.31 ms per adjustment, change path alone 0.82 ms; run 0.06 s wall, events 127374
k=30: 2279 objects (graph before QVTo), 114 adjustments (158 pods created), 2.43 ms per adjustment, change path alone 1.57 ms; run 0.33 s wall, events 380453
k=40: 3029 objects (graph before QVTo), 145 adjustments (198 pods created), 3.03 ms per adjustment, change path alone 2.41 ms; run 0.51 s wall, events 508379
test cost_per_scaling_step ... QVTo set-up (core, MainTransformation): 10.4 ms
k=1: 104 objects (graph before QVTo), 3 adjustments (5 pods created), 0.99 ms per adjustment, change path alone 0.40 ms; run 0.00 s wall, events 12844
k=10: 779 objects (graph before QVTo), 36 adjustments (52 pods created), 1.15 ms per adjustment, change path alone 0.77 ms; run 0.05 s wall, events 127374
k=30: 2279 objects (graph before QVTo), 114 adjustments (158 pods created), 2.35 ms per adjustment, change path alone 2.14 ms; run 0.32 s wall, events 380453
k=40: 3029 objects (graph before QVTo), 145 adjustments (198 pods created), 3.06 ms per adjustment, change path alone 2.42 ms; run 0.52 s wall, events 508379
test cost_per_scaling_step ... QVTo set-up (core, MainTransformation): 13.7 ms
k=1: 104 objects (graph before QVTo), 3 adjustments (5 pods created), 2.10 ms per adjustment, change path alone 0.37 ms; run 0.01 s wall, events 12844
k=10: 779 objects (graph before QVTo), 36 adjustments (52 pods created), 1.20 ms per adjustment, change path alone 0.92 ms; run 0.06 s wall, events 127374
k=30: 2279 objects (graph before QVTo), 114 adjustments (158 pods created), 4.11 ms per adjustment, change path alone 2.49 ms; run 0.52 s wall, events 380453
k=40: 3029 objects (graph before QVTo), 145 adjustments (198 pods created), 3.52 ms per adjustment, change path alone 2.52 ms; run 0.59 s wall, events 508379
```


### Warm per-run wall time (median; in brackets: SimOxide speed-up)

| model | simulated s | SimOxide | SimuLizar 5.2.2 stock | SimuLizar 5.2.2 stock, SSJ |
|---|---|---|---|---|
| x_ss_minimal short | 1 000 | 0.219 ms | 187 ms (854x) | 183 ms (837x) |
| x_ss_minimal long | 100 000 | 9.8 ms | 2.95 s (301x) | 2.66 s (271x) |
| x_espresso short | 100 | 0.387 ms | 277 ms (716x) | 280 ms (724x) |
| x_espresso long | 2 000 | 5.22 ms | 3 s (574x) | 2.51 s (482x) |
| x_ss_mediastore short | 200 000 | 1.22 ms | 541 ms (443x) | 518 ms (425x) |
| x_ss_mediastore long | 10 M | 8.31 ms | 5.03 s (605x) | 4.44 s (535x) |
| x_sl_mediastore short | 1 M | 1.25 ms | 566 ms (452x) | 568 ms (454x) |
| x_sl_mediastore long | 25 M | 5.11 ms | 4.08 s (798x) | 4.04 s (790x) |
| h13_passive_contention short | 100 | 0.439 ms | 274 ms (623x) | 233 ms (530x) |
| h13_passive_contention long | 5 000 | 14 ms | 4.4 s (315x) | 3.58 s (256x) |
| x_pem_fork short | 100 | 0.259 ms | 192 ms (743x) | 288 ms (1 115x) |
| x_pem_fork long | 5 000 | 3.36 ms | 2.72 s (812x) | 2.19 s (653x) |

### Cold one-shot wall time, process start to results (median of 3; SimOxide speed-up)

| model | simulated s | SimOxide | SimuLizar 5.2.2 stock | SimuLizar 5.2.2 stock, SSJ |
|---|---|---|---|---|
| x_ss_minimal short | 1 000 | 5.78 ms | 12.8 s (2 207x) | 12.3 s (2 134x) |
| x_ss_minimal long | 100 000 | – | – | – |
| x_espresso short | 100 | 4.81 ms | 11.7 s (2 436x) | 11.4 s (2 367x) |
| x_espresso long | 2 000 | – | – | – |
| x_ss_mediastore short | 200 000 | 7 ms | 13 s (1 854x) | 13.5 s (1 933x) |
| x_ss_mediastore long | 10 M | – | – | – |
| x_sl_mediastore short | 1 M | 5.66 ms | 15 s (2 651x) | 13.8 s (2 445x) |
| x_sl_mediastore long | 25 M | – | – | – |
| h13_passive_contention short | 100 | 5.16 ms | 10.3 s (2 000x) | 10.6 s (2 058x) |
| h13_passive_contention long | 5 000 | – | – | – |
| x_pem_fork short | 100 | 4.09 ms | 10.8 s (2 644x) | 11.3 s (2 772x) |
| x_pem_fork long | 5 000 | – | – | – |

### Per-run times (warm = median in a long-lived process; cold = one process, start to results)

**x_ss_minimal, short: 1 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 0.219 ms | 453 | 2.07 M | 4.57 M | 8.27 M | 0.00578 s | 4.38 / 4.12 MB | – |
| SimuLizar 5.2.2 stock | 187 ms | 453 | 2 421 | 5 345 | – | 12.8 s | 382 / 341 MB | 854x / 2 207x |
| SimuLizar 5.2.2 stock, SSJ | 183 ms | 453 | 2 471 | 5 455 | – | 12.3 s | 405 / 343 MB | 837x / 2 134x |

**x_ss_minimal, long: 100 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 9.8 ms | 45 413 | 4.63 M | 10.2 M | 18.5 M | – s | 13.2 / – MB | – |
| SimuLizar 5.2.2 stock | 2 953 ms | 45 413 | 15 376 | 33 859 | – | – s | 510 / – MB | 301x / – |
| SimuLizar 5.2.2 stock, SSJ | 2 661 ms | 45 413 | 17 067 | 37 582 | – | – s | 455 / – MB | 271x / – |

**x_espresso, short: 100 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 0.387 ms | 990 | 2.56 M | 258 398 | 10.3 M | 0.00481 s | 4.5 / 4.25 MB | – |
| SimuLizar 5.2.2 stock | 277 ms | 990 | 3 574 | 361 | – | 11.7 s | 392 / 351 MB | 716x / 2 436x |
| SimuLizar 5.2.2 stock, SSJ | 280 ms | 990 | 3 534 | 357 | – | 11.4 s | 472 / 342 MB | 724x / 2 367x |

**x_espresso, long: 2 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 5.22 ms | 19 980 | 3.83 M | 383 105 | 15.3 M | – s | 7.09 / – MB | – |
| SimuLizar 5.2.2 stock | 2 996 ms | 19 980 | 6 668 | 668 | – | – s | 408 / – MB | 574x / – |
| SimuLizar 5.2.2 stock, SSJ | 2 514 ms | 19 980 | 7 948 | 796 | – | – s | 420 / – MB | 482x / – |

**x_ss_mediastore, short: 200 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 1.22 ms | 18 | 14 760 | 164 M | 953 670 | 0.007 s | 5.62 / 4.88 MB | – |
| SimuLizar 5.2.2 stock | 541 ms | 18 | 33.3 | 369 828 | – | 13 s | 472 / 371 MB | 443x / 1 854x |
| SimuLizar 5.2.2 stock, SSJ | 518 ms | 18 | 34.8 | 386 173 | – | 13.5 s | 500 / 370 MB | 425x / 1 933x |

**x_ss_mediastore, long: 10 M s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 8.31 ms | 937 | 112 803 | 1 204 M | 6.78 M | – s | 9.91 / – MB | – |
| SimuLizar 5.2.2 stock | 5 029 ms | 937 | 186 | 1.99 M | – | – s | 466 / – MB | 605x / – |
| SimuLizar 5.2.2 stock, SSJ | 4 440 ms | 937 | 211 | 2.25 M | – | – s | 452 / – MB | 535x / – |

**x_sl_mediastore, short: 1 M s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 1.25 ms | 33 | 26 368 | 799 M | 1.11 M | 0.00566 s | 5.75 / 4.88 MB | – |
| SimuLizar 5.2.2 stock | 566 ms | 33 | 58.3 | 1.77 M | – | 15 s | 476 / 369 MB | 452x / 2 651x |
| SimuLizar 5.2.2 stock, SSJ | 568 ms | 33 | 58.1 | 1.76 M | – | 13.8 s | 507 / 368 MB | 454x / 2 445x |

**x_sl_mediastore, long: 25 M s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 5.11 ms | 833 | 162 886 | 4 889 M | 6.69 M | – s | 8.49 / – MB | – |
| SimuLizar 5.2.2 stock | 4 080 ms | 833 | 204 | 6.13 M | – | – s | 426 / – MB | 798x / – |
| SimuLizar 5.2.2 stock, SSJ | 4 041 ms | 833 | 206 | 6.19 M | – | – s | 441 / – MB | 790x / – |

**h13_passive_contention, short: 100 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 0.439 ms | 468 | 1.07 M | 227 790 | 6.71 M | 0.00516 s | 4.62 / 4.25 MB | – |
| SimuLizar 5.2.2 stock | 274 ms | 468 | 1 711 | 366 | – | 10.3 s | 393 / 342 MB | 623x / 2 000x |
| SimuLizar 5.2.2 stock, SSJ | 233 ms | 468 | 2 011 | 430 | – | 10.6 s | 415 / 339 MB | 530x / 2 058x |

**h13_passive_contention, long: 5 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 14 ms | 22 749 | 1.63 M | 358 064 | 10.3 M | – s | 13.9 / – MB | – |
| SimuLizar 5.2.2 stock | 4 395 ms | 22 749 | 5 176 | 1 138 | – | – s | 549 / – MB | 315x / – |
| SimuLizar 5.2.2 stock, SSJ | 3 582 ms | 22 749 | 6 351 | 1 396 | – | – s | 542 / – MB | 256x / – |

**x_pem_fork, short: 100 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 0.259 ms | 87 | 336 557 | 386 847 | 5.42 M | 0.00409 s | 4.38 / 4.12 MB | – |
| SimuLizar 5.2.2 stock | 192 ms | 87 | 453 | 520 | – | 10.8 s | 380 / 345 MB | 743x / 2 644x |
| SimuLizar 5.2.2 stock, SSJ | 288 ms | 87 | 302 | 347 | – | 11.3 s | 381 / 352 MB | 1 115x / 2 772x |

**x_pem_fork, long: 5 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 3.36 ms | 4 396 | 1.31 M | 1.49 M | 21 M | – s | 7.3 / – MB | – |
| SimuLizar 5.2.2 stock | 2 724 ms | 4 396 | 1 614 | 1 836 | – | – s | 387 / – MB | 812x / – |
| SimuLizar 5.2.2 stock, SSJ | 2 190 ms | 4 396 | 2 007 | 2 283 | – | – s | 389 / – MB | 653x / – |

### Sanity check: usage-scenario response time (warm runs, seed 1)

| model | length | SimOxide | SimuLizar 5.2.2 stock | SimuLizar 5.2.2 stock, SSJ |
|---|---|---|---|---|
| x_ss_minimal | short | 1 (n=453) | 1 (n=453) | 1 (n=453) |
| x_ss_minimal | long | 1 (n=45 413) | 1 (n=45 413) | 1 (n=45 413) |
| x_espresso | short | 3 (n=990) | 3 (n=990) | 3 (n=990) |
| x_espresso | long | 3 (n=19 980) | 3 (n=19 980) | 3 (n=19 980) |
| x_ss_mediastore | short | 20801.12 (n=18) | 20801.12 (n=18) | 20801.12 (n=18) |
| x_ss_mediastore | long | 21302.96 (n=937) | 21302.96 (n=937) | 21302.96 (n=937) |
| x_sl_mediastore | short | 23574.74 (n=33) | 23574.74 (n=33) | 23574.74 (n=33) |
| x_sl_mediastore | long | 23550.52 (n=833) | 23550.52 (n=833) | 23550.52 (n=833) |
| h13_passive_contention | short | 0.320115 (n=468) | 0.320115 (n=468) | 0.320115 (n=468) |
| h13_passive_contention | long | 0.31877 (n=22 749) | 0.31877 (n=22 749) | 0.31877 (n=22 749) |
| x_pem_fork | short | 1.136782 (n=87) | 1.136782 (n=87) | 1.136782 (n=87) |
| x_pem_fork | long | 1.137216 (n=4 396) | 1.137216 (n=4 396) | 1.137216 (n=4 396) |

### Parallel throughput (all 22 cores)

| model | length | configuration | workers | runs | wall s | runs/s | requests/s | memory MB |
|---|---|---|---|---|---|---|---|---|

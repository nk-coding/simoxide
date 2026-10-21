### Sustained, one core

**x_espresso, 100 s simulated per run** (config: wall duration in brackets)

| config | runs (failed) | steady runs/s | steady requests/s | SimOxide speed-up | warm-up s | 30-s windows: first / peak / last runs/s | last/middle third | p50 / p99 / max ms | GC pause | RSS MB start / max / end | threads start / max / end |
|---|---|---|---|---|---|---|---|---|---|---|---|
| simoxide (60 s) | 145643 (0) | 2 424 | 2 399 694 | – | 0.00368 | 2 431 / 2 431 / 2 424 | 1 | 0.4 / 0.63 / 5.2 | – | 10 / 17 / 17 | 2 / 2 / 2 |
| simucom-ssj (120 s) | 248 (0) | 2.23 | 2 211 | 1 086x | 15 | 1.57 / 2.23 / 2.23 | 1.01 | 440 / 528 / 5 989 | 0.99 % | 1 877 / 2 816 / 2 816 | 756 / 7507 / 7507 |
| simucom (120 s) | 239 (0) | 2.17 | 2 145 | 1 119x | 14.5 | 1.53 / 2.17 / 2.17 | 1.05 | 456 / 529 / 5 757 | 1.61 % | 1 231 / 1 656 / 1 656 | 152 / 157 / 149 |
| simulizar-ssj (120 s) | 344 (0) | 3 | 2 970 | 808x | 8.44 | 2.47 / 3 / 3 | 1.01 | 328 / 406 / 4 489 | 1.11 % | 722 / 1 675 / 1 675 | 1255 / 10551 / 10551 |
| simulizar (120 s) | 333 (0) | 2.93 | 2 904 | 826x | 7.84 | 2.3 / 2.97 / 2.97 | 1.02 | 343 / 401 / 4 664 | 1.05 % | 714 / 1 176 / 1 176 | 110 / 400 / 400 |

**x_ss_mediastore, 2 000 000 s simulated per run** (config: wall duration in brackets)

| config | runs (failed) | steady runs/s | steady requests/s | SimOxide speed-up | warm-up s | 30-s windows: first / peak / last runs/s | last/middle third | p50 / p99 / max ms | GC pause | RSS MB start / max / end | threads start / max / end |
|---|---|---|---|---|---|---|---|---|---|---|---|
| simoxide (60 s) | 24355 (0) | 404 | 75 865 | – | 0.00709 | 407 / 407 / 404 | 1 | 2.38 / 3.44 / 219 | – | 8 / 9 / 9 | 2 / 2 / 2 |
| simucom-ssj (300 s) | 131 (0) | 0.447 | 83.8 | 905x | 11.7 | 0.3 / 0.467 / 0.433 | 1 | 2 232 / 2 409 / 8 777 | 0.8 % | 1 787 / 2 242 / 2 242 | 134 / 385 / 385 |
| simucom (300 s) | 125 (0) | 0.433 | 81.3 | 934x | 15.4 | 0.267 / 0.467 / 0.467 | 1.03 | 2 314 / 2 495 / 9 541 | 1.24 % | 1 656 / 1 941 / 1 941 | 128 / 139 / 123 |
| simulizar-ssj (300 s) | 141 (0) | 0.487 | 91.2 | 830x | 10.8 | 0.333 / 0.5 / 0.467 | 1.02 | 2 060 / 2 197 / 8 194 | 1.31 % | 801 / 2 054 / 2 054 | 64 / 465 / 465 |
| simulizar (300 s) | 142 (0) | 0.48 | 90 | 842x | 7.94 | 0.367 / 0.533 / 0.533 | 1.02 | 2 113 / 2 708 / 7 933 | 1.39 % | 819 / 2 026 / 2 026 | 50 / 186 / 186 |

### Sustained, all 22 cores

**x_ss_mediastore, 2 000 000 s simulated per run** (config: wall duration in brackets)

| config | runs (failed) | steady runs/s | steady requests/s | SimOxide speed-up | warm-up s | 30-s windows: first / peak / last runs/s | last/middle third | p50 / p99 / max ms | GC pause | RSS MB start / max / end | threads start / max / end |
|---|---|---|---|---|---|---|---|---|---|---|---|
| simoxide-threads-22 (60 s) | 309185 (0) | 5 146 | 965 594 | – | 0.0444 | 5 160 / 5 160 / 5 146 | 1 | 4.51 / 8.17 / 52.4 | – | 51 / 67 / 49 | 23 / 23 / 1 |
| simulizar-procs-22 (– s) | 1705 (0) | 8.37 | 1 566 | 615x | 40.6 | 0.067 / 9.67 / 8.63 | 0.906 | 2 605 / 3 823 / 34 596 | 6.42 % | 12 144 / 25 176 / 25 176 | 513 / 546 / 432 | KILLED
| simulizar-ssj-procs-22 (– s) | 1307 (0) | 13.6 | 2 554 | 378x | 45.3 | 0.533 / 13.6 / 13.6 | 1.4 | 1 663 / 2 985 / 31 970 | 6.8 % | 12 407 / 24 399 / 24 399 | 518 / 3010 / 3010 | KILLED
| simulizar-ssj-threads-22 (300 s) | 2154 (9) | 3.96 | 744 | 1 299x | 0.00349 | 10.9 / 11 / 0.133 | 0.108 | 2 591 / 49 951 / 51 057 | 34.8 % | 6 526 / 13 722 / 0 | 401 / 6512 / 0 |
| simulizar-threads-22 (300 s) | 2154 (13) | 4.92 | 923 | 1 046x | 0.00531 | 9.47 / 9.5 / 0.2 | 0.282 | 2 658 / 60 005 / 62 337 | 33 % | 6 183 / 13 338 / 13 322 | 201 / 2267 / 2245 |

### One very long run

| simulator | simulated s | finished | run wall s | requests | sim s / s | vs warm "long" | refsim in-run sim s/s (first → last minute) | RSS MB at 20 s → end → peak | GC pause |
|---|---|---|---|---|---|---|---|---|---|
| simoxide | 1 000 000 000 | yes | 0.888 | 94202 | 1 125 538 007 | 0.865 | – | – → – → 323 | – |
| simoxide | 5 000 000 000 | yes | 4.68 | 471282 | 1 068 804 504 | 0.821 | – | – → 826 → 1 591 | – |
| simucom-ssj | 300 000 000 | yes | 183 | 28256 | 1 642 549 | 1.1 | – | 1 186 → 2 849 → 2 918 | 2.26 % |
| simucom | 300 000 000 | yes | 176 | 28256 | 1 701 143 | 1.19 | – | 1 345 → 3 173 → 3 204 | 2.48 % |
| simulizar-ssj | 200 000 000 | yes | 172 | 18846 | 1 166 110 | 1.11 | – | 1 036 → 1 837 → 1 837 | 1.44 % |
| simulizar | 200 000 000 | yes | 211 | 18846 | 948 098 | 0.911 | – | 950 → 1 642 → 1 648 | 1.39 % |

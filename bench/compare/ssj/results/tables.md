### Warm per-run wall time (median; in brackets: SimOxide speed-up)

| model | simulated s | SimOxide | SimuLizar 5.2.2 stock | SimuLizar 5.2.2 stock, SSJ | SimuCom 5.2.2* | SimuCom 5.2.2*, SSJ |
|---|---|---|---|---|---|---|
| x_ss_minimal short | 1 000 | 0.231 ms | 83.9 ms (363x) | 83.3 ms (361x) | 357 ms (1 545x) | 365 ms (1 580x) |
| x_ss_minimal long | 100 000 | 9.4 ms | 3.27 s (348x) | 2.54 s (270x) | 2.27 s (241x) | 1.96 s (208x) |
| x_espresso short | 100 | 0.403 ms | 306 ms (761x) | 292 ms (727x) | 450 ms (1 117x) | 459 ms (1 140x) |
| x_espresso long | 2 000 | 4.88 ms | 5.09 s (1 043x) | 4.87 s (997x) | 3.9 s (799x) | 3.62 s (741x) |
| x_ss_mediastore short | 200 000 | 1.22 ms | 307 ms (253x) | 289 ms (238x) | 1.22 s (1 003x) | 1.26 s (1 034x) |
| x_ss_mediastore long | 10 M | 7.69 ms | 9.61 s (1 250x) | 9.5 s (1 236x) | 6.97 s (907x) | 6.73 s (875x) |
| x_sl_mediastore short | 1 M | 1.27 ms | 378 ms (298x) | 404 ms (319x) | 1.59 s (1 258x) | 1.51 s (1 194x) |
| x_sl_mediastore long | 25 M | 4.57 ms | 6.6 s (1 445x) | 6.75 s (1 477x) | 5.15 s (1 127x) | 5.59 s (1 224x) |
| h13_passive_contention short | 100 | 0.449 ms | 277 ms (618x) | 259 ms (577x) | 510 ms (1 137x) | 488 ms (1 086x) |
| h13_passive_contention long | 5 000 | 13.7 ms | 11.1 s (804x) | 10.1 s (731x) | 8.38 s (609x) | 7.13 s (519x) |
| x_pem_fork short | 100 | 0.248 ms | 227 ms (917x) | 258 ms (1 042x) | 496 ms (1 998x) | 545 ms (2 198x) |
| x_pem_fork long | 5 000 | 3.11 ms | 8.04 s (2 588x) | 10.2 s (3 271x) | 6.73 s (2 169x) | 8.9 s (2 866x) |
| x_misarch short | 36 | 11.9 ms | 11.7 s (985x) | 13.9 s (1 168x) | – | – |
| x_misarch long | 361 | 75.9 ms | 88.8 s (1 169x) | 110 s (1 446x) | – | – |

### Cold one-shot wall time, process start to results (median of 3; SimOxide speed-up)

| model | simulated s | SimOxide | SimuLizar 5.2.2 stock | SimuLizar 5.2.2 stock, SSJ | SimuCom 5.2.2* | SimuCom 5.2.2*, SSJ | SimuLizar stock, OSGi (cold only) | SimuLizar stock, SSJ, OSGi (cold only) |
|---|---|---|---|---|---|---|---|---|
| x_ss_minimal short | 1 000 | 6.85 ms | 5.99 s (874x) | 5.59 s (816x) | 8.33 s (1 215x) | 8.24 s (1 204x) | 6.22 s (908x) | 6.47 s (945x) |
| x_ss_minimal long | 100 000 | 172 ms | 9.81 s (57x) | 9.34 s (54x) | 11.3 s (66x) | 10.6 s (62x) | – | – |
| x_espresso short | 100 | 11 ms | 6.45 s (588x) | 6.29 s (574x) | 9.11 s (831x) | 8.86 s (809x) | 6.75 s (616x) | 6.69 s (611x) |
| x_espresso long | 2 000 | 50 ms | 12.5 s (251x) | 11.7 s (234x) | 12.6 s (253x) | 12.4 s (247x) | – | – |
| x_ss_mediastore short | 200 000 | 10.3 ms | 7.4 s (716x) | 7.38 s (715x) | 11.4 s (1 102x) | 11.2 s (1 084x) | 8.13 s (787x) | 8.03 s (778x) |
| x_ss_mediastore long | 10 M | 62.3 ms | 18.9 s (304x) | 17.8 s (286x) | 18.2 s (291x) | 17.2 s (276x) | – | – |
| x_sl_mediastore short | 1 M | 10.5 ms | 7.42 s (703x) | 7.18 s (681x) | 11.2 s (1 066x) | 11.2 s (1 066x) | – | – |
| x_sl_mediastore long | 25 M | 42.7 ms | 14.6 s (342x) | 14.4 s (339x) | 15.7 s (368x) | 15.9 s (373x) | – | – |
| h13_passive_contention short | 100 | 10.6 ms | 5.98 s (563x) | 5.91 s (557x) | 8.75 s (824x) | 8.73 s (822x) | – | – |
| h13_passive_contention long | 5 000 | 247 ms | 17.5 s (71x) | 16.3 s (66x) | 16.9 s (69x) | 15.6 s (63x) | – | – |
| x_pem_fork short | 100 | 8.75 ms | 5.85 s (668x) | 5.78 s (660x) | 8.71 s (995x) | 8.55 s (977x) | – | – |
| x_pem_fork long | 5 000 | 26.2 ms | 14.3 s (545x) | 16.4 s (625x) | 14.9 s (570x) | 17.4 s (666x) | – | – |
| x_misarch short | 36 | 28.5 ms | 19.8 s (693x) | 20.7 s (724x) | – | – | – | – |
| x_misarch long | 361 | 86.4 ms | 99.1 s (1 146x) | 114 s (1 318x) | – | – | – | – |

### Per-run times (warm = median in a long-lived process; cold = one process, start to results)

**x_ss_minimal, short: 1 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 0.231 ms | 453 | 1.96 M | 4.33 M | 7.84 M | 0.00685 s | 5.47 / 7.42 MB | – |
| SimuLizar 5.2.2 stock | 83.9 ms | 453 | 5 401 | 11 923 | – | 5.99 s | 700 / 507 MB | 363x / 874x |
| SimuLizar 5.2.2 stock, SSJ | 83.3 ms | 453 | 5 439 | 12 006 | – | 5.59 s | 724 / 512 MB | 361x / 816x |
| SimuLizar stock, OSGi (cold only) | – ms | – | – | – | – | 6.22 s | – / 460 MB | – / 908x |
| SimuLizar stock, SSJ, OSGi (cold only) | – ms | – | – | – | – | 6.47 s | – / 465 MB | – / 945x |
| SimuCom 5.2.2* | 357 ms | 453 | 1 269 | 2 801 | – | 8.33 s | 1 821 / 883 MB | 1 545x / 1 215x |
| SimuCom 5.2.2*, SSJ | 365 ms | 453 | 1 241 | 2 740 | – | 8.24 s | 1 818 / 916 MB | 1 580x / 1 204x |

**x_ss_minimal, long: 100 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 9.4 ms | 45 413 | 4.83 M | 10.6 M | 19.3 M | 0.172 s | 14.8 / 11.3 MB | – |
| SimuLizar 5.2.2 stock | 3 268 ms | 45 413 | 13 898 | 30 603 | – | 9.81 s | 1 413 / 859 MB | 348x / 57x |
| SimuLizar 5.2.2 stock, SSJ | 2 541 ms | 45 413 | 17 874 | 39 358 | – | 9.34 s | 1 256 / 774 MB | 270x / 54x |
| SimuCom 5.2.2* | 2 265 ms | 45 413 | 20 050 | 44 150 | – | 11.3 s | 1 897 / 1 118 MB | 241x / 66x |
| SimuCom 5.2.2*, SSJ | 1 959 ms | 45 413 | 23 182 | 51 046 | – | 10.6 s | 2 236 / 1 021 MB | 208x / 62x |

**x_espresso, short: 100 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 0.403 ms | 990 | 2.46 M | 248 447 | 9.92 M | 0.011 s | 6.05 / 7.42 MB | – |
| SimuLizar 5.2.2 stock | 306 ms | 990 | 3 233 | 327 | – | 6.45 s | 710 / 522 MB | 761x / 588x |
| SimuLizar 5.2.2 stock, SSJ | 292 ms | 990 | 3 385 | 342 | – | 6.29 s | 722 / 512 MB | 727x / 574x |
| SimuLizar stock, OSGi (cold only) | – ms | – | – | – | – | 6.75 s | – / 479 MB | – / 616x |
| SimuLizar stock, SSJ, OSGi (cold only) | – ms | – | – | – | – | 6.69 s | – / 460 MB | – / 611x |
| SimuCom 5.2.2* | 450 ms | 990 | 2 202 | 222 | – | 9.11 s | 1 522 / 961 MB | 1 117x / 831x |
| SimuCom 5.2.2*, SSJ | 459 ms | 990 | 2 157 | 218 | – | 8.86 s | 2 069 / 952 MB | 1 140x / 809x |

**x_espresso, long: 2 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 4.88 ms | 19 980 | 4.09 M | 409 836 | 16.4 M | 0.05 s | 8.69 / 9.18 MB | – |
| SimuLizar 5.2.2 stock | 5 090 ms | 19 980 | 3 925 | 393 | – | 12.5 s | 881 / 616 MB | 1 043x / 251x |
| SimuLizar 5.2.2 stock, SSJ | 4 867 ms | 19 980 | 4 105 | 411 | – | 11.7 s | 837 / 546 MB | 997x / 234x |
| SimuCom 5.2.2* | 3 900 ms | 19 980 | 5 124 | 513 | – | 12.6 s | 1 974 / 992 MB | 799x / 253x |
| SimuCom 5.2.2*, SSJ | 3 616 ms | 19 980 | 5 525 | 553 | – | 12.4 s | 1 958 / 835 MB | 741x / 247x |

**x_ss_mediastore, short: 200 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 1.22 ms | 18 | 14 815 | 165 M | 957 202 | 0.0103 s | 7.42 / 8.2 MB | – |
| SimuLizar 5.2.2 stock | 307 ms | 18 | 58.6 | 650 821 | – | 7.4 s | 817 / 636 MB | 253x / 716x |
| SimuLizar 5.2.2 stock, SSJ | 289 ms | 18 | 62.4 | 693 075 | – | 7.38 s | 843 / 577 MB | 238x / 715x |
| SimuLizar stock, OSGi (cold only) | – ms | – | – | – | – | 8.13 s | – / 610 MB | – / 787x |
| SimuLizar stock, SSJ, OSGi (cold only) | – ms | – | – | – | – | 8.03 s | – / 601 MB | – / 778x |
| SimuCom 5.2.2* | 1 219 ms | 18 | 14.8 | 164 069 | – | 11.4 s | 1 596 / 1 064 MB | 1 003x / 1 102x |
| SimuCom 5.2.2*, SSJ | 1 256 ms | 18 | 14.3 | 159 172 | – | 11.2 s | 2 139 / 1 037 MB | 1 034x / 1 084x |

**x_ss_mediastore, long: 10 M s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 7.69 ms | 937 | 121 910 | 1 301 M | 7.33 M | 0.0623 s | 11.6 / 10.9 MB | – |
| SimuLizar 5.2.2 stock | 9 608 ms | 937 | 97.5 | 1.04 M | – | 18.9 s | 1 193 / 798 MB | 1 250x / 304x |
| SimuLizar 5.2.2 stock, SSJ | 9 497 ms | 937 | 98.7 | 1.05 M | – | 17.8 s | 1 196 / 733 MB | 1 236x / 286x |
| SimuCom 5.2.2* | 6 968 ms | 937 | 134 | 1.44 M | – | 18.2 s | 1 717 / 1 240 MB | 907x / 291x |
| SimuCom 5.2.2*, SSJ | 6 726 ms | 937 | 139 | 1.49 M | – | 17.2 s | 2 252 / 1 222 MB | 875x / 276x |

**x_sl_mediastore, short: 1 M s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 1.27 ms | 33 | 26 056 | 790 M | 1.1 M | 0.0105 s | 7.62 / 8.2 MB | – |
| SimuLizar 5.2.2 stock | 378 ms | 33 | 87.3 | 2.65 M | – | 7.42 s | 887 / 649 MB | 298x / 703x |
| SimuLizar 5.2.2 stock, SSJ | 404 ms | 33 | 81.8 | 2.48 M | – | 7.18 s | 1 003 / 560 MB | 319x / 681x |
| SimuCom 5.2.2* | 1 594 ms | 33 | 20.7 | 627 549 | – | 11.2 s | 1 560 / 1 081 MB | 1 258x / 1 066x |
| SimuCom 5.2.2*, SSJ | 1 512 ms | 33 | 21.8 | 661 376 | – | 11.2 s | 1 656 / 987 MB | 1 194x / 1 066x |

**x_sl_mediastore, long: 25 M s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 4.57 ms | 833 | 182 196 | 5 468 M | 7.48 M | 0.0427 s | 10.3 / 9.77 MB | – |
| SimuLizar 5.2.2 stock | 6 604 ms | 833 | 126 | 3.79 M | – | 14.6 s | 953 / 735 MB | 1 445x / 342x |
| SimuLizar 5.2.2 stock, SSJ | 6 753 ms | 833 | 123 | 3.7 M | – | 14.4 s | 1 114 / 704 MB | 1 477x / 339x |
| SimuCom 5.2.2* | 5 154 ms | 833 | 162 | 4.85 M | – | 15.7 s | 1 774 / 1 091 MB | 1 127x / 368x |
| SimuCom 5.2.2*, SSJ | 5 594 ms | 833 | 149 | 4.47 M | – | 15.9 s | 1 732 / 1 145 MB | 1 224x / 373x |

**h13_passive_contention, short: 100 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 0.449 ms | 468 | 1.04 M | 222 717 | 6.56 M | 0.0106 s | 5.86 / 7.42 MB | – |
| SimuLizar 5.2.2 stock | 277 ms | 468 | 1 688 | 361 | – | 5.98 s | 692 / 520 MB | 618x / 563x |
| SimuLizar 5.2.2 stock, SSJ | 259 ms | 468 | 1 806 | 386 | – | 5.91 s | 677 / 515 MB | 577x / 557x |
| SimuCom 5.2.2* | 510 ms | 468 | 917 | 196 | – | 8.75 s | 1 650 / 905 MB | 1 137x / 824x |
| SimuCom 5.2.2*, SSJ | 488 ms | 468 | 960 | 205 | – | 8.73 s | 1 564 / 925 MB | 1 086x / 822x |

**h13_passive_contention, long: 5 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 13.7 ms | 22 749 | 1.65 M | 363 742 | 10.4 M | 0.247 s | 15.5 / 13.2 MB | – |
| SimuLizar 5.2.2 stock | 11 054 ms | 22 749 | 2 058 | 452 | – | 17.5 s | 1 267 / 723 MB | 804x / 71x |
| SimuLizar 5.2.2 stock, SSJ | 10 055 ms | 22 749 | 2 262 | 497 | – | 16.3 s | 1 275 / 603 MB | 731x / 66x |
| SimuCom 5.2.2* | 8 378 ms | 22 749 | 2 715 | 597 | – | 16.9 s | 2 307 / 1 010 MB | 609x / 69x |
| SimuCom 5.2.2*, SSJ | 7 133 ms | 22 749 | 3 189 | 701 | – | 15.6 s | 1 656 / 945 MB | 519x / 63x |

**x_pem_fork, short: 100 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 0.248 ms | 87 | 350 806 | 403 226 | 5.65 M | 0.00875 s | 5.47 / 7.23 MB | – |
| SimuLizar 5.2.2 stock | 227 ms | 87 | 382 | 440 | – | 5.85 s | 647 / 497 MB | 917x / 668x |
| SimuLizar 5.2.2 stock, SSJ | 258 ms | 87 | 337 | 387 | – | 5.78 s | 735 / 508 MB | 1 042x / 660x |
| SimuCom 5.2.2* | 496 ms | 87 | 176 | 202 | – | 8.71 s | 1 303 / 877 MB | 1 998x / 995x |
| SimuCom 5.2.2*, SSJ | 545 ms | 87 | 160 | 183 | – | 8.55 s | 1 633 / 939 MB | 2 198x / 977x |

**x_pem_fork, long: 5 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 3.11 ms | 4 396 | 1.42 M | 1.61 M | 22.7 M | 0.0262 s | 8.49 / 8.4 MB | – |
| SimuLizar 5.2.2 stock | 8 037 ms | 4 396 | 547 | 622 | – | 14.3 s | 888 / 653 MB | 2 588x / 545x |
| SimuLizar 5.2.2 stock, SSJ | 10 157 ms | 4 396 | 433 | 492 | – | 16.4 s | 1 045 / 620 MB | 3 271x / 625x |
| SimuCom 5.2.2* | 6 734 ms | 4 396 | 653 | 742 | – | 14.9 s | 1 371 / 914 MB | 2 169x / 570x |
| SimuCom 5.2.2*, SSJ | 8 900 ms | 4 396 | 494 | 562 | – | 17.4 s | 1 878 / 940 MB | 2 866x / 666x |

**x_misarch, short: 36 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 11.9 ms | 2 | 168 | 3 033 | 8.51 M | 0.0285 s | 14.2 / 11.5 MB | – |
| SimuLizar 5.2.2 stock | 11 688 ms | 2 | 0.171 | 3.08 | – | 19.8 s | 915 / 663 MB | 985x / 693x |
| SimuLizar 5.2.2 stock, SSJ | 13 868 ms | 2 | 0.144 | 2.6 | – | 20.7 s | 1 207 / 639 MB | 1 168x / 724x |

**x_misarch, long: 361 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 75.9 ms | 8 | 105 | 4 755 | 13.5 M | 0.0864 s | 14.3 / 11.9 MB | – |
| SimuLizar 5.2.2 stock | 88 756 ms | 18 | 0.203 | 4.07 | – | 99.1 s | 1 099 / 787 MB | 1 169x / 1 146x |
| SimuLizar 5.2.2 stock, SSJ | 109 783 ms | 21 | 0.191 | 3.29 | – | 114 s | 960 / 703 MB | 1 446x / 1 318x |

### Sanity check: usage-scenario response time (warm runs, seed 1)

| model | length | SimOxide | SimuLizar 5.2.2 stock | SimuLizar 5.2.2 stock, SSJ | SimuCom 5.2.2* | SimuCom 5.2.2*, SSJ |
|---|---|---|---|---|---|---|
| x_ss_minimal | short | 1 (n=453) | 1 (n=453) | 1 (n=453) | 1 (n=453) | 1 (n=453) |
| x_ss_minimal | long | 1 (n=45 413) | 1 (n=45 413) | 1 (n=45 413) | 1 (n=45 413) | 1 (n=45 413) |
| x_espresso | short | 3 (n=990) | 3 (n=990) | 3 (n=990) | 3 (n=990) | 3 (n=990) |
| x_espresso | long | 3 (n=19 980) | 3 (n=19 980) | 3 (n=19 980) | 3 (n=19 980) | 3 (n=19 980) |
| x_ss_mediastore | short | 20801.12 (n=18) | 20801.12 (n=18) | 20801.12 (n=18) | 20801.12 (n=18) | 20801.12 (n=18) |
| x_ss_mediastore | long | 21302.96 (n=937) | 21302.96 (n=937) | 21302.96 (n=937) | 21302.96 (n=937) | 21302.96 (n=937) |
| x_sl_mediastore | short | 23574.74 (n=33) | 23574.74 (n=33) | 23574.74 (n=33) | 23574.74 (n=33) | 23574.74 (n=33) |
| x_sl_mediastore | long | 23550.52 (n=833) | 23550.52 (n=833) | 23550.52 (n=833) | 23550.52 (n=833) | 23550.52 (n=833) |
| h13_passive_contention | short | 0.320115 (n=468) | 0.320115 (n=468) | 0.320115 (n=468) | 0.3201146 (n=468) | 0.3201146 (n=468) |
| h13_passive_contention | long | 0.31877 (n=22 749) | 0.31877 (n=22 749) | 0.31877 (n=22 749) | 0.3187697 (n=22 749) | 0.3187697 (n=22 749) |
| x_pem_fork | short | 1.136782 (n=87) | 1.136782 (n=87) | 1.136782 (n=87) | 1.136782 (n=87) | 1.136782 (n=87) |
| x_pem_fork | long | 1.137216 (n=4 396) | 1.137216 (n=4 396) | 1.137216 (n=4 396) | 1.137216 (n=4 396) | 1.137216 (n=4 396) |
| x_misarch | short | 0.024472 (n=2) | 0.040376 (n=2) | 0.040376 (n=2) | – | – |
| x_misarch | long | 0.013403 (n=8) | 0.010677 (n=18) | 0.009421 (n=21) | – | – |

### Parallel throughput (all 22 cores)

| model | length | configuration | workers | runs | wall s | runs/s | requests/s | memory MB |
|---|---|---|---|---|---|---|---|---|
| x_espresso | short | simoxide-reload-t22 (threads) | 22 | 22000 | 0.879 | 25 034 | 24.8 M | 17 |
| x_espresso | short | simoxide-batch-t22 (batch API threads) | 22 | 22000 | 0.888 | 24 769 | 24.5 M | 2 098 |
| x_espresso | short | simulizar-t22 (threads) | 22 | 440 | 7.27 | 60.5 | 59 883 | 5 913 |
| x_espresso | short | simulizar-ssj-t22 (threads) | 22 | 440 | 7.48 | 58.8 | 58 261 | 6 688 |
| x_espresso | short | simulizar-ssj-p22 (processes) | 22 | 440 | 8.07 | 54.5 | 53 964 | 15 023 |
| x_espresso | short | simulizar-p22 (processes) | 22 | 440 | 8.47 | 51.9 | 51 398 | 12 519 |
| x_espresso | short | simucom-p16 (processes) | 16 | 320 | 16.1 | 19.9 | 19 705 | 18 866 |
| x_espresso | short | simucom-ssj-p16 (processes) | 16 | 320 | 18.1 | 17.7 | 17 510 | 21 967 |

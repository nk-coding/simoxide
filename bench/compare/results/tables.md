### Warm per-run wall time (median; in brackets: SimOxide speed-up)

| model | simulated s | SimOxide | refsim (patched) | SimuLizar 5.2.2 stock | SimuLizar + VT patch | Slingshot | SimuCom 5.2.2* | EventSim 5.1 (archived) |
|---|---|---|---|---|---|---|---|---|
| x_ss_minimal short | 1 000 | 0.209 ms | 75.6 ms (361x) | 82.7 ms (395x) | 81.2 ms (388x) | 82.5 ms (394x) | 328 ms (1 568x) | 50 ms (239x) |
| x_ss_minimal long | 100 000 | 9.73 ms | 2.55 s (262x) | 2.97 s (306x) | 2.91 s (299x) | 5.72 s (588x) | 2.3 s (237x) | 1.36 s (139x) |
| x_espresso short | 100 | 0.38 ms | 311 ms (818x) | 319 ms (840x) | 113 ms (297x) | 151 ms (398x) | 512 ms (1 349x) | 76 ms (200x) |
| x_espresso long | 2 000 | 4.75 ms | 4.9 s (1 031x) | 5.06 s (1 065x) | 1.28 s (269x) | 2.37 s (498x) | 3.67 s (771x) | 520 ms (109x) |
| x_ss_mediastore short | 200 000 | 1.16 ms | 258 ms (222x) | 297 ms (256x) | 284 ms (245x) | 211 ms (182x) | 1.22 s (1 054x) | 121 ms (104x) |
| x_ss_mediastore long | 10 M | 7.64 ms | 7.56 s (989x) | 9.12 s (1 194x) | 8.72 s (1 141x) | 7.84 s (1 026x) | 6.5 s (851x) | 2.59 s (339x) |
| x_sl_mediastore short | 1 M | 1.18 ms | 298 ms (253x) | 330 ms (280x) | 366 ms (310x) | 241 ms (205x) | 1.4 s (1 190x) | 116 ms (98x) |
| x_sl_mediastore long | 25 M | 4.54 ms | 4.79 s (1 054x) | 5.54 s (1 219x) | 5.76 s (1 268x) | 4.36 s (960x) | 4.69 s (1 031x) | 1.34 s (295x) |
| h13_passive_contention short | 100 | 0.417 ms | 207 ms (497x) | 250 ms (599x) | 108 ms (259x) | 171 ms (411x) | 455 ms (1 091x) | 54.5 ms (131x) |
| h13_passive_contention long | 5 000 | 13 ms | 7.75 s (598x) | 10.1 s (777x) | 2.87 s (222x) | 7.38 s (569x) | 7.32 s (564x) | 985 ms (76x) |
| x_pem_fork short | 100 | 0.209 ms | 187 ms (893x) | 193 ms (923x) | 118 ms (566x) | fails | 438 ms (2 096x) | 34.5 ms (165x) |
| x_pem_fork long | 5 000 | 3.06 ms | 6.86 s (2 242x) | 7.42 s (2 423x) | 2.84 s (928x) | fails | 6.31 s (2 062x) | 373 ms (122x) |

### Cold one-shot wall time, process start to results (median of 3; SimOxide speed-up)

| model | simulated s | SimOxide | refsim (patched) | SimuLizar 5.2.2 stock | SimuLizar + VT patch | Slingshot | SimuCom 5.2.2* | EventSim 5.1 (archived) | SimuLizar stock, OSGi (cold only) | Slingshot, OSGi (cold only) |
|---|---|---|---|---|---|---|---|---|---|---|
| x_ss_minimal short | 1 000 | 4.88 ms | 5.49 s (1 125x) | 5.41 s (1 109x) | 5.45 s (1 118x) | 2.71 s (556x) | 7.88 s (1 615x) | 3.82 s (782x) | 5.94 s (1 218x) | 3.87 s (793x) |
| x_ss_minimal long | 100 000 | 90.1 ms | 8.64 s (96x) | 9.16 s (102x) | 9.47 s (105x) | 9.24 s (103x) | 10 s (111x) | 4.9 s (54x) | – | – |
| x_espresso short | 100 | 7.62 ms | 5.87 s (770x) | 5.77 s (757x) | 5.72 s (750x) | 2.87 s (376x) | 8 s (1 050x) | 3.79 s (498x) | 6.36 s (834x) | 3.84 s (504x) |
| x_espresso long | 2 000 | 65.2 ms | 11.4 s (176x) | 11.7 s (179x) | 7.71 s (118x) | 5.67 s (87x) | 11.9 s (183x) | 4.26 s (65x) | – | – |
| x_ss_mediastore short | 200 000 | 5.58 ms | 6.6 s (1 184x) | 6.66 s (1 195x) | 6.7 s (1 203x) | 3.8 s (682x) | 10.1 s (1 806x) | 4.46 s (800x) | 7.43 s (1 333x) | 4.84 s (868x) |
| x_ss_mediastore long | 10 M | 76 ms | 15.8 s (207x) | 17.1 s (225x) | 16.9 s (222x) | 12.6 s (166x) | 16.3 s (215x) | 7.52 s (99x) | – | – |
| x_sl_mediastore short | 1 M | 9.51 ms | 6.84 s (719x) | 6.96 s (732x) | 6.82 s (717x) | 4.02 s (423x) | 10.3 s (1 086x) | 4.42 s (465x) | – | – |
| x_sl_mediastore long | 25 M | 74 ms | 12.6 s (170x) | 13.7 s (185x) | 14.1 s (191x) | 9.33 s (126x) | 14.5 s (195x) | 6.21 s (84x) | – | – |
| h13_passive_contention short | 100 | 5.31 ms | 5.73 s (1 081x) | 5.76 s (1 085x) | 5.66 s (1 066x) | 3.04 s (573x) | 8.17 s (1 541x) | 3.95 s (744x) | – | – |
| h13_passive_contention long | 5 000 | 227 ms | 14.4 s (64x) | 16.5 s (73x) | 10 s (44x) | 11.6 s (51x) | 16.2 s (71x) | 4.9 s (22x) | – | – |
| x_pem_fork short | 100 | 4.18 ms | 5.56 s (1 329x) | 5.56 s (1 330x) | 5.56 s (1 329x) | fails | 8.14 s (1 947x) | 3.87 s (926x) | – | – |
| x_pem_fork long | 5 000 | 26.8 ms | 13.5 s (504x) | 13.8 s (517x) | 10 s (374x) | fails | 14.2 s (529x) | 4.28 s (160x) | – | – |

### Per-run times (warm = median in a long-lived process; cold = one process, start to results)

**x_ss_minimal, short: 1 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 0.209 ms | 453 | 2.16 M | 4.77 M | 8.65 M | 0.00488 s | 3.95 / 4.12 MB | – |
| refsim (patched) | 75.6 ms | 453 | 5 996 | 13 235 | – | 5.49 s | 733 / 496 MB | 361x / 1 125x |
| SimuLizar 5.2.2 stock | 82.7 ms | 453 | 5 476 | 12 089 | – | 5.41 s | 761 / 511 MB | 395x / 1 109x |
| SimuLizar stock, OSGi (cold only) | – ms | – | – | – | – | 5.94 s | – / 441 MB | – / 1 218x |
| SimuLizar + VT patch | 81.2 ms | 453 | 5 578 | 12 314 | – | 5.45 s | 742 / 512 MB | 388x / 1 118x |
| Slingshot | 82.5 ms | 453 | 5 492 | 12 123 | 214 416 | 2.71 s | 718 / 364 MB | 394x / 556x |
| Slingshot, OSGi (cold only) | – ms | – | – | – | – | 3.87 s | – / 431 MB | – / 793x |
| SimuCom 5.2.2* | 328 ms | 453 | 1 379 | 3 044 | – | 7.88 s | 1 521 / 910 MB | 1 568x / 1 615x |
| EventSim 5.1 (archived) | 50 ms | 454 | 9 080 | 20 000 | – | 3.82 s | 729 / 545 MB | 239x / 782x |

**x_ss_minimal, long: 100 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 9.73 ms | 45 413 | 4.67 M | 10.3 M | 18.7 M | 0.0901 s | 13.3 / 8.08 MB | – |
| refsim (patched) | 2 553 ms | 45 413 | 17 785 | 39 163 | – | 8.64 s | 912 / 854 MB | 262x / 96x |
| SimuLizar 5.2.2 stock | 2 974 ms | 45 413 | 15 269 | 33 623 | – | 9.16 s | 1 324 / 905 MB | 306x / 102x |
| SimuLizar + VT patch | 2 906 ms | 45 413 | 15 628 | 34 414 | – | 9.47 s | 1 408 / 889 MB | 299x / 105x |
| Slingshot | 5 721 ms | 45 413 | 7 938 | 17 480 | 309 598 | 9.24 s | 1 365 / 1 133 MB | 588x / 103x |
| SimuCom 5.2.2* | 2 302 ms | 45 413 | 19 728 | 43 440 | – | 10 s | 2 011 / 1 143 MB | 237x / 111x |
| EventSim 5.1 (archived) | 1 356 ms | 45 476 | 33 525 | 73 719 | – | 4.9 s | 1 541 / 719 MB | 139x / 54x |

**x_espresso, short: 100 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 0.38 ms | 990 | 2.61 M | 263 158 | 10.5 M | 0.00762 s | 4.3 / 4.12 MB | – |
| refsim (patched) | 311 ms | 990 | 3 186 | 322 | – | 5.87 s | 706 / 523 MB | 818x / 770x |
| SimuLizar 5.2.2 stock | 319 ms | 990 | 3 100 | 313 | – | 5.77 s | 687 / 524 MB | 840x / 757x |
| SimuLizar stock, OSGi (cold only) | – ms | – | – | – | – | 6.36 s | – / 492 MB | – / 834x |
| SimuLizar + VT patch | 113 ms | 990 | 8 781 | 887 | – | 5.72 s | 871 / 518 MB | 297x / 750x |
| Slingshot | 151 ms | 990 | 6 540 | 661 | 299 064 | 2.87 s | 871 / 395 MB | 398x / 376x |
| Slingshot, OSGi (cold only) | – ms | – | – | – | – | 3.84 s | – / 492 MB | – / 504x |
| SimuCom 5.2.2* | 512 ms | 990 | 1 932 | 195 | – | 8 s | 1 960 / 775 MB | 1 349x / 1 050x |
| EventSim 5.1 (archived) | 76 ms | 990 | 13 026 | 1 316 | – | 3.79 s | 732 / 555 MB | 200x / 498x |

**x_espresso, long: 2 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 4.75 ms | 19 980 | 4.2 M | 420 787 | 16.8 M | 0.0652 s | 7.1 / 6.02 MB | – |
| refsim (patched) | 4 898 ms | 19 980 | 4 079 | 408 | – | 11.4 s | 675 / 544 MB | 1 031x / 176x |
| SimuLizar 5.2.2 stock | 5 060 ms | 19 980 | 3 948 | 395 | – | 11.7 s | 796 / 630 MB | 1 065x / 179x |
| SimuLizar + VT patch | 1 277 ms | 19 980 | 15 652 | 1 567 | – | 7.71 s | 1 223 / 767 MB | 269x / 118x |
| Slingshot | 2 367 ms | 19 980 | 8 441 | 845 | 380 174 | 5.67 s | 1 258 / 880 MB | 498x / 87x |
| SimuCom 5.2.2* | 3 666 ms | 19 980 | 5 451 | 546 | – | 11.9 s | 1 838 / 940 MB | 771x / 183x |
| EventSim 5.1 (archived) | 520 ms | 19 980 | 38 423 | 3 846 | – | 4.26 s | 1 238 / 636 MB | 109x / 65x |

**x_ss_mediastore, short: 200 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 1.16 ms | 18 | 15 517 | 172 M | 1 M | 0.00558 s | 5.67 / 4.81 MB | – |
| refsim (patched) | 258 ms | 18 | 69.9 | 776 122 | – | 6.6 s | 849 / 545 MB | 222x / 1 184x |
| SimuLizar 5.2.2 stock | 297 ms | 18 | 60.6 | 673 853 | – | 6.66 s | 825 / 544 MB | 256x / 1 195x |
| SimuLizar stock, OSGi (cold only) | – ms | – | – | – | – | 7.43 s | – / 594 MB | – / 1 333x |
| SimuLizar + VT patch | 284 ms | 18 | 63.3 | 703 538 | – | 6.7 s | 826 / 547 MB | 245x / 1 203x |
| Slingshot | 211 ms | 18 | 85.3 | 947 277 | 112 409 | 3.8 s | 774 / 453 MB | 182x / 682x |
| Slingshot, OSGi (cold only) | – ms | – | – | – | – | 4.84 s | – / 565 MB | – / 868x |
| SimuCom 5.2.2* | 1 223 ms | 18 | 14.7 | 163 532 | – | 10.1 s | 1 805 / 825 MB | 1 054x / 1 806x |
| EventSim 5.1 (archived) | 121 ms | 18 | 149 | 1.65 M | – | 4.46 s | 846 / 613 MB | 104x / 800x |

**x_ss_mediastore, long: 10 M s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 7.64 ms | 937 | 122 620 | 1 309 M | 7.37 M | 0.076 s | 9.96 / 7.56 MB | – |
| refsim (patched) | 7 559 ms | 937 | 124 | 1.32 M | – | 15.8 s | 759 / 658 MB | 989x / 207x |
| SimuLizar 5.2.2 stock | 9 124 ms | 937 | 103 | 1.1 M | – | 17.1 s | 1 185 / 771 MB | 1 194x / 225x |
| SimuLizar + VT patch | 8 721 ms | 937 | 107 | 1.15 M | – | 16.9 s | 1 179 / 762 MB | 1 141x / 222x |
| Slingshot | 7 838 ms | 937 | 120 | 1.28 M | 147 130 | 12.6 s | 1 238 / 1 005 MB | 1 026x / 166x |
| SimuCom 5.2.2* | 6 500 ms | 937 | 144 | 1.54 M | – | 16.3 s | 1 660 / 1 037 MB | 851x / 215x |
| EventSim 5.1 (archived) | 2 589 ms | 944 | 364 | 3.86 M | – | 7.52 s | 935 / 719 MB | 339x / 99x |

**x_sl_mediastore, short: 1 M s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 1.18 ms | 33 | 27 978 | 848 M | 1.18 M | 0.00951 s | 5.84 / 4.98 MB | – |
| refsim (patched) | 298 ms | 33 | 111 | 3.35 M | – | 6.84 s | 810 / 549 MB | 253x / 719x |
| SimuLizar 5.2.2 stock | 330 ms | 33 | 99.9 | 3.03 M | – | 6.96 s | 932 / 664 MB | 280x / 732x |
| SimuLizar + VT patch | 366 ms | 33 | 90.2 | 2.73 M | – | 6.82 s | 922 / 566 MB | 310x / 717x |
| Slingshot | 241 ms | 33 | 137 | 4.14 M | 91 869 | 4.02 s | 829 / 480 MB | 205x / 423x |
| SimuCom 5.2.2* | 1 404 ms | 33 | 23.5 | 712 251 | – | 10.3 s | 1 679 / 862 MB | 1 190x / 1 086x |
| EventSim 5.1 (archived) | 116 ms | 16 | 138 | 8.62 M | – | 4.42 s | 866 / 610 MB | 98x / 465x |

**x_sl_mediastore, long: 25 M s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 4.54 ms | 833 | 183 298 | 5 501 M | 7.52 M | 0.074 s | 8.38 / 6.19 MB | – |
| refsim (patched) | 4 790 ms | 833 | 174 | 5.22 M | – | 12.6 s | 750 / 701 MB | 1 054x / 170x |
| SimuLizar 5.2.2 stock | 5 541 ms | 833 | 150 | 4.51 M | – | 13.7 s | 885 / 727 MB | 1 219x / 185x |
| SimuLizar + VT patch | 5 760 ms | 833 | 145 | 4.34 M | – | 14.1 s | 1 013 / 712 MB | 1 268x / 191x |
| Slingshot | 4 364 ms | 833 | 191 | 5.73 M | 124 228 | 9.33 s | 964 / 783 MB | 960x / 126x |
| SimuCom 5.2.2* | 4 686 ms | 833 | 178 | 5.33 M | – | 14.5 s | 2 497 / 1 091 MB | 1 031x / 195x |
| EventSim 5.1 (archived) | 1 340 ms | 527 | 393 | 18.7 M | – | 6.21 s | 833 / 730 MB | 295x / 84x |

**h13_passive_contention, short: 100 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 0.417 ms | 468 | 1.12 M | 239 808 | 7.07 M | 0.00531 s | 4.3 / 4.12 MB | – |
| refsim (patched) | 207 ms | 468 | 2 259 | 483 | – | 5.73 s | 725 / 513 MB | 497x / 1 081x |
| SimuLizar 5.2.2 stock | 250 ms | 468 | 1 872 | 400 | – | 5.76 s | 672 / 517 MB | 599x / 1 085x |
| SimuLizar + VT patch | 108 ms | 468 | 4 328 | 925 | – | 5.66 s | 765 / 513 MB | 259x / 1 066x |
| Slingshot | 171 ms | 471 | 2 750 | 584 | 263 564 | 3.04 s | 798 / 489 MB | 411x / 573x |
| SimuCom 5.2.2* | 455 ms | 468 | 1 029 | 220 | – | 8.17 s | 2 098 / 800 MB | 1 091x / 1 541x |
| EventSim 5.1 (archived) | 54.5 ms | 446 | 8 174 | 1 835 | – | 3.95 s | 726 / 549 MB | 131x / 744x |

**h13_passive_contention, long: 5 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 13 ms | 22 749 | 1.75 M | 385 342 | 11 M | 0.227 s | 13.8 / 9.86 MB | – |
| refsim (patched) | 7 754 ms | 22 749 | 2 934 | 645 | – | 14.4 s | 699 / 597 MB | 598x / 64x |
| SimuLizar 5.2.2 stock | 10 086 ms | 22 749 | 2 256 | 496 | – | 16.5 s | 1 230 / 731 MB | 777x / 73x |
| SimuLizar + VT patch | 2 874 ms | 22 749 | 7 915 | 1 740 | – | 10 s | 1 423 / 842 MB | 222x / 44x |
| Slingshot | 7 383 ms | 22 718 | 3 077 | 677 | 295 469 | 11.6 s | 1 380 / 1 093 MB | 569x / 51x |
| SimuCom 5.2.2* | 7 322 ms | 22 749 | 3 107 | 683 | – | 16.2 s | 2 046 / 982 MB | 564x / 71x |
| EventSim 5.1 (archived) | 985 ms | 22 795 | 23 142 | 5 076 | – | 4.9 s | 1 398 / 713 MB | 76x / 22x |

**x_pem_fork, short: 100 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 0.209 ms | 87 | 416 268 | 478 469 | 6.71 M | 0.00418 s | 3.95 / 4.12 MB | – |
| refsim (patched) | 187 ms | 87 | 466 | 536 | – | 5.56 s | 602 / 516 MB | 893x / 1 329x |
| SimuLizar 5.2.2 stock | 193 ms | 87 | 451 | 518 | – | 5.56 s | 634 / 508 MB | 923x / 1 330x |
| SimuLizar + VT patch | 118 ms | 87 | 736 | 846 | – | 5.56 s | 622 / 523 MB | 566x / 1 329x |
| SimuCom 5.2.2* | 438 ms | 87 | 199 | 228 | – | 8.14 s | 1 762 / 898 MB | 2 096x / 1 947x |
| EventSim 5.1 (archived) | 34.5 ms | 87 | 2 522 | 2 899 | – | 3.87 s | 705 / 541 MB | 165x / 926x |

**x_pem_fork, long: 5 000 s simulated**

| simulator | warm per run | requests | requests/s | sim s / s | events/s | cold one-shot | peak RSS warm / cold | SimOxide speed-up warm / cold |
|---|---|---|---|---|---|---|---|---|
| SimOxide | 3.06 ms | 4 396 | 1.44 M | 1.63 M | 23 M | 0.0268 s | 6.89 / 5.5 MB | – |
| refsim (patched) | 6 861 ms | 4 396 | 641 | 729 | – | 13.5 s | 879 / 575 MB | 2 242x / 504x |
| SimuLizar 5.2.2 stock | 7 417 ms | 4 396 | 593 | 674 | – | 13.8 s | 878 / 537 MB | 2 423x / 517x |
| SimuLizar + VT patch | 2 840 ms | 4 396 | 1 548 | 1 761 | – | 10 s | 767 / 608 MB | 928x / 374x |
| SimuCom 5.2.2* | 6 311 ms | 4 396 | 697 | 792 | – | 14.2 s | 1 430 / 858 MB | 2 062x / 529x |
| EventSim 5.1 (archived) | 373 ms | 4 392 | 11 773 | 13 405 | – | 4.28 s | 1 240 / 600 MB | 122x / 160x |

### Sanity check: usage-scenario response time (warm runs, seed 1)

| model | length | SimOxide | refsim (patched) | SimuLizar 5.2.2 stock | SimuLizar + VT patch | Slingshot | SimuCom 5.2.2* | EventSim 5.1 (archived) |
|---|---|---|---|---|---|---|---|---|
| x_ss_minimal | short | 1 (n=453) | 1 (n=453) | 1 (n=453) | 1 (n=453) | 1 (n=453) | 1 (n=453) | n/a (n=454) |
| x_ss_minimal | long | 1 (n=45 413) | 1 (n=45 413) | 1 (n=45 413) | 1 (n=45 413) | 1 (n=45 413) | 1 (n=45 413) | n/a (n=45 476) |
| x_espresso | short | 3 (n=990) | 3 (n=990) | 3 (n=990) | 3 (n=990) | 3 (n=990) | 3 (n=990) | n/a (n=990) |
| x_espresso | long | 3 (n=19 980) | 3 (n=19 980) | 3 (n=19 980) | 3 (n=19 980) | 3 (n=19 980) | 3 (n=19 980) | n/a (n=19 980) |
| x_ss_mediastore | short | 20801.12 (n=18) | 20801.12 (n=18) | 20801.12 (n=18) | 20801.12 (n=18) | 20801.12 (n=18) | 20801.12 (n=18) | n/a (n=18) |
| x_ss_mediastore | long | 21302.96 (n=937) | 21302.96 (n=937) | 21302.96 (n=937) | 21302.96 (n=937) | 21302.96 (n=937) | 21302.96 (n=937) | n/a (n=944) |
| x_sl_mediastore | short | 23574.74 (n=33) | 23574.74 (n=33) | 23574.74 (n=33) | 23574.74 (n=33) | 23677.54 (n=33) | 23574.74 (n=33) | n/a (n=16) |
| x_sl_mediastore | long | 23550.52 (n=833) | 23550.52 (n=833) | 23550.52 (n=833) | 23550.52 (n=833) | 23550.21 (n=833) | 23550.52 (n=833) | n/a (n=527) |
| h13_passive_contention | short | 0.320115 (n=468) | 0.320115 (n=468) | 0.320115 (n=468) | 0.320115 (n=468) | 0.3244 (n=471) | 0.3201146 (n=468) | n/a (n=446) |
| h13_passive_contention | long | 0.31877 (n=22 749) | 0.31877 (n=22 749) | 0.31877 (n=22 749) | 0.31877 (n=22 749) | 0.3168 (n=22 718) | 0.3187697 (n=22 749) | n/a (n=22 795) |
| x_pem_fork | short | 1.136782 (n=87) | 1.136782 (n=87) | 1.136782 (n=87) | 1.136782 (n=87) | – | 1.136782 (n=87) | n/a (n=87) |
| x_pem_fork | long | 1.137216 (n=4 396) | 1.137216 (n=4 396) | 1.137216 (n=4 396) | 1.137216 (n=4 396) | – | 1.137216 (n=4 396) | n/a (n=4 392) |

### Parallel throughput (all 22 cores)

| model | length | configuration | workers | runs | wall s | runs/s | requests/s | memory MB |
|---|---|---|---|---|---|---|---|---|
| x_espresso | short | simoxide-reload-t22 (threads) | 22 | 22000 | 0.806 | 27 282 | 27 M | 16 |
| x_espresso | short | simoxide-batch-t22 (batch API threads) | 22 | 22000 | 0.818 | 26 911 | 26.6 M | 2 096 |
| x_espresso | short | simulizar-vt-t22 (threads) | 22 | 440 | 5.93 | 74.2 | 73 457 | 5 797 |
| x_espresso | short | simulizar-t22 (threads) | 22 | 440 | 6.31 | 69.8 | 69 064 | 6 011 |
| x_espresso | short | refsim-p22 (processes) | 22 | 440 | 6.92 | 63.6 | 62 921 | 12 762 |
| x_espresso | short | simulizar-p22 (processes) | 22 | 440 | 7.3 | 60.3 | 59 679 | 12 641 |
| x_espresso | short | slingshot-p22 (processes) | 22 | 440 | 8 | 55 | 54 470 | 10 243 |
| x_espresso | short | slingshot-iso-k22 (isolated class loaders) | 22 | 440 | 21.5 | 20.5 | 20 261 | 4 422 |
| x_espresso | short | simucom-p16 (processes) | 16 | 320 | 17.9 | 17.9 | 17 734 | 18 854 |

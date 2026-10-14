| model | len | SimOxide ms | SimuLizar DESMO-J ms | SimuLizar SSJ ms | SimOxide vs DESMO-J | SimOxide vs SSJ | SSJ vs DESMO-J | per round | results |
|---|---|---:|---:|---:|---:|---:|---:|---|---|
| `x_ss_minimal` | short | 0.219 | 187 | 183 | 854× | 837× | 1.02× | 0.99 / 1.01 / 1.06 | identical (453 req, mean RT 1) |
| `x_ss_minimal` | long | 9.80 | 2953 | 2661 | 301× | 271× | 1.11× | 1.11 / 1.10 / 1.15 | identical (45413 req, mean RT 1) |
| `x_espresso` | short | 0.387 | 277 | 280 | 716× | 724× | 0.99× | 1.13 / 0.90 / 0.97 | identical (990 req, mean RT 3) |
| `x_espresso` | long | 5.22 | 2996 | 2514 | 574× | 482× | 1.19× | 1.30 / 1.18 / 1.17 | identical (19980 req, mean RT 3) |
| `x_ss_mediastore` | short | 1.22 | 541 | 518 | 443× | 425× | 1.04× | 1.09 / 0.82 / 1.06 | identical (18 req, mean RT 20801.1) |
| `x_ss_mediastore` | long | 8.31 | 5029 | 4440 | 605× | 535× | 1.13× | 1.11 / 0.79 / 1.22 | identical (937 req, mean RT 21303) |
| `x_sl_mediastore` | short | 1.25 | 566 | 568 | 452× | 454× | 1.00× | 1.33 / 0.99 / 0.84 | identical (33 req, mean RT 23574.7) |
| `x_sl_mediastore` | long | 5.11 | 4080 | 4041 | 798× | 790× | 1.01× | 0.97 / 1.02 / 1.00 | identical (833 req, mean RT 23550.5) |
| `h13_passive_contention` | short | 0.439 | 274 | 233 | 623× | 530× | 1.18× | 1.35 / 1.23 / 0.91 | identical (468 req, mean RT 0.320115) |
| `h13_passive_contention` | long | 14 | 4395 | 3582 | 315× | 256× | 1.23× | 1.03 / 1.26 / 1.40 | identical (22749 req, mean RT 0.31877) |
| `x_pem_fork` | short | 0.259 | 192 | 288 | 743× | 1,115× | 0.67× | 0.67 / 0.65 / 0.69 | identical (87 req, mean RT 1.13678) |
| `x_pem_fork` | long | 3.36 | 2724 | 2190 | 812× | 653× | 1.24× | 0.90 / 1.29 / 0.93 | identical (4396 req, mean RT 1.13722) |

SSJ vs DESMO-J (warm, geometric mean over 12 configurations): 1.05× (range 0.67× to 1.24×)

Cold one-shot, short (process start to results, median of the rounds, s):

| model | SimOxide CLI | SimuLizar DESMO-J | SimuLizar SSJ | SSJ vs DESMO-J |
|---|---:|---:|---:|---:|
| `x_ss_minimal` | 0.006 | 12.75 | 12.33 | 1.03× |
| `x_espresso` | 0.005 | 11.71 | 11.38 | 1.03× |
| `x_ss_mediastore` | 0.007 | 12.98 | 13.53 | 0.96× |
| `x_sl_mediastore` | 0.006 | 15.00 | 13.83 | 1.08× |
| `h13_passive_contention` | 0.005 | 10.33 | 10.62 | 0.97× |
| `x_pem_fork` | 0.004 | 10.82 | 11.34 | 0.95× |

Drift check, warm median per round (ms): simoxide h13_passive_contention long: 14/15/14; simoxide h13_passive_contention short: 0.444/0.447/0.429; simoxide x_espresso long: 5.12/4.93/8.09; simoxide x_espresso short: 0.372/0.387/0.403; simoxide x_pem_fork long: 3.22/3.31/3.76; simoxide x_pem_fork short: 0.363/0.223/0.254; simoxide x_sl_mediastore long: 4.72/5.46/5.67; simoxide x_sl_mediastore short: 1.25/1.25/1.27; simoxide x_ss_mediastore long: 8.31/7.99/8.48; simoxide x_ss_mediastore short: 1.27/1.22/1.18; simoxide x_ss_minimal long: 10/9.79/9.78; simoxide x_ss_minimal short: 0.233/0.212/0.214; simulizar h13_passive_contention long: 4281/4579/4395; simulizar h13_passive_contention short: 240/287/234; simulizar x_espresso long: 2996/2976/3001; simulizar x_espresso short: 292/218/297; simulizar x_pem_fork long: 1960/2800/2752; simulizar x_pem_fork short: 194/189/194; simulizar x_sl_mediastore long: 3604/4183/4059; simulizar x_sl_mediastore short: 656/570/491; simulizar x_ss_mediastore long: 4618/4113/5233; simulizar x_ss_mediastore short: 591/390/645; simulizar x_ss_minimal long: 2953/2519/3053; simulizar x_ss_minimal short: 186/163/211; simulizar-ssj h13_passive_contention long: 4145/3642/3146; simulizar-ssj h13_passive_contention short: 178/233/257; simulizar-ssj x_espresso long: 2300/2514/2558; simulizar-ssj x_espresso short: 259/243/307; simulizar-ssj x_pem_fork long: 2171/2175/2953; simulizar-ssj x_pem_fork short: 289/291/282; simulizar-ssj x_sl_mediastore long: 3704/4087/4041; simulizar-ssj x_sl_mediastore short: 495/577/584; simulizar-ssj x_ss_mediastore long: 4144/5238/4281; simulizar-ssj x_ss_mediastore short: 540/477/606; simulizar-ssj x_ss_minimal long: 2661/2287/2664; simulizar-ssj x_ss_minimal short: 188/162/199

Engine evidence (# ENGINE lines): simulizar: ['DesmoJSimEngineFactory'], simulizar-ssj: ['SSJSimEngineFactory']

Problems: none

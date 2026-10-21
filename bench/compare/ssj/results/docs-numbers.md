| placeholder | value | source |
|---|---|---|
| `SC_DESMOJ_WARM_SPEEDUP` | 241 to 2 169 | summary.csv warm_wall_ms simucom / simoxide, 6 models x 2 lengths |
| `SC_DESMOJ_WARM_SPEEDUP_X` | 241× to 2 169× | same as SC_DESMOJ_WARM_SPEEDUP |
| `SC_SSJ_COLD` | 8.24 to 17.4 s | summary.csv cold_wall_s of simucom-ssj |
| `SC_SSJ_ONECORE` | 0.447 runs/s | sus_summary.csv sus1 x_ss_mediastore steady_runs_per_s |
| `SC_SSJ_VS_DESMOJ` | 85.1 to 132 % (median 99.3 %) | summary.csv warm_wall_ms simucom-ssj / simucom |
| `SC_SSJ_WARM_SPEEDUP` | 208 to 2 866 | summary.csv warm_wall_ms simucom-ssj / simoxide, 6 models x 2 lengths |
| `SC_SSJ_WARM_SPEEDUP_X` | 208× to 2 866× | same as SC_SSJ_WARM_SPEEDUP |
| `SL_DESMOJ_WARM_SPEEDUP` | 253 to 2 588 | summary.csv warm_wall_ms simulizar / simoxide, 6 models x 2 lengths |
| `SL_DESMOJ_WARM_SPEEDUP_X` | 253× to 2 588× | same as SL_DESMOJ_WARM_SPEEDUP |
| `SL_HEADLINE_RANGE` | 238 to 3 271 | min and max of SL_DESMOJ_WARM_SPEEDUP and SL_SSJ_WARM_SPEEDUP |
| `SL_SSJ_ALLCORES` | 13.6 runs/s at 23.8 GB (22 processes), killed; 3.96 runs/s at 13.4 GB (in one JVM), 9 failed runs | sus_summary.csv susN simulizar-ssj-* |
| `SL_SSJ_COLD` | 5.59 to 17.8 s | summary.csv cold_wall_s of simulizar-ssj |
| `SL_SSJ_ONECORE` | 0.487 runs/s | sus_summary.csv sus1 x_ss_mediastore steady_runs_per_s |
| `SL_SSJ_VS_DESMOJ` | 77.8 to 126 % (median 97.2 %) | summary.csv warm_wall_ms simulizar-ssj / simulizar |
| `SL_SSJ_WARM_SPEEDUP` | 238 to 3 271 | summary.csv warm_wall_ms simulizar-ssj / simoxide, 6 models x 2 lengths |
| `SL_SSJ_WARM_SPEEDUP_X` | 238× to 3 271× | same as SL_SSJ_WARM_SPEEDUP |
| `SSJ_COLD_VS_DESMOJ` | SimuLizar 92.7 to 115 %, SimuCom 92 to 117 % | summary.csv cold_wall_s ssj / desmoj |
| `SSJ_CONSISTENCY` | SimuLizar on SSJ: mean response time within 0 % and request count within 0 % of DESMO-J (seed 1, every model and length); SimuCom on SSJ: mean response time within 2.33e-07 % and request count within 0 % of DESMO-J (seed 1, every model and length) | summary.csv mean_rt and requests, ssj vs desmoj |
| `SSJ_SESSION` | 2026-10-21, 25 cores, 34 GB | raw/*/*.log START_EPOCH_MS; cores and RAM of the machine running this script |
| `SSJ_STABILITY` | SimuLizar: 141 runs (0 failed), threads 64 → 465, RSS 801 → 2054 MB, last/middle third 1.02; SimuCom: 131 runs (0 failed), threads 134 → 385, RSS 1787 → 2242 MB, last/middle third 1 | sus_summary.csv sus1 x_ss_mediastore (review the wording) |
| `SSJ_STOP_EFFECT` | Sustained one-core runs: simucom-ssj on x_espresso: 756 → 7507 threads, RSS max 2816 MB in 248 runs; simucom-ssj on x_ss_mediastore: 134 → 385 threads, RSS max 2242 MB in 131 runs; simucom on x_espresso: 152 → 157 threads, RSS max 1656 MB in 239 runs; simucom on x_ss_mediastore: 128 → 139 threads, RSS max 1941 MB in 125 runs; simulizar-ssj on x_espresso: 1255 → 10551 threads, RSS max 1675 MB in 344 runs; simulizar-ssj on x_ss_mediastore: 64 → 465 threads, RSS max 2054 MB in 141 runs; simulizar on x_espresso: 110 → 400 threads, RSS max 1176 MB in 333 runs; simulizar on x_ss_mediastore: 50 → 186 threads, RSS max 2026 MB in 142 runs. | sus_summary.csv sus1 threads and RSS (review: is the SSJ growth the leak?) |

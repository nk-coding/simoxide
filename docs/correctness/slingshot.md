# Slingshot flavour

What the [Slingshot flavour](../guide/slingshot.md) (fast mode, design phase S2) is checked against,
and how close it is. The oracle is the [Slingshot reference runner](../reference-simulator/slingshot.md):
its `det` mode (seeded, insertion-ordered same-time events) for exact comparisons, its `stock` mode
(the unmodified nightly, same-time order random per run) for statistics.

## The claim

1. **Exact where the orders agree.** With the compat flags the fast mode supports (`--quirks fast`)
   and the exact RNG mode (`--mode exact`: MT19937 StoEx `{N..N+5}`, `java.util.Random(N + 1000003)`
   for branches and link failures), `measurements.csv` is byte-identical to `det` with the same seed
   and simulated time whenever the fast mode's same-time order and draw order agree with Slingshot's
   event cascade. This holds for every Slingshot corpus model `x_ss_*` except the tie-only
   `x_espresso`, for 16 of the 20 specification probes and for 38 of the 45 corpus models of the sweep below.
2. **Statistically equal otherwise**, for every series whose value does not depend on the order of
   simultaneous steps: response times, demands, counts of finished requests, time-weighted states.
3. **Fixed behaviour by default.** Without flags the flavour deviates from Slingshot exactly where a
   quirk is documented ([Quirks](../spec/slingshot/quirks.md)); the tests below show each one.

## Evidence

### Probes and `x_ss` models (`crates/simoxide-sim/tests/slingshot.rs`)

- **Probes** (`reference/slingshot/models/probes/*/expected`): 16 of 20 byte-identical with
  `--quirks fast`. The other four have Slingshot handler errors: `ss_guard_none`, `ss_loop_zero`
  and `ss_usage_loop_zero` stop with a run error where Slingshot's handler throws (Q19, Q21; the
  disposal, Q18, is the exact mode's), `ss_two_demands` needs Q17, which the fast mode refuses.
- **`x_ss_*`**, seeds 1, 2 and 3 (`reference/slingshot/results/s2/`): `x_ss_minimal`, `x_ss_delay`,
  `x_ss_minimalexample`, `x_ss_minimalexample_open_ps` (1 000 s) and `x_ss_mediastore` (200 000 s)
  are byte-identical, 15 of 15 runs. The quirks each needs (removing any one of them breaks the
  identity): `x_ss_minimal` and `x_ss_minimalexample` Q29; `x_ss_minimalexample_open_ps` Q26, Q29;
  `x_ss_mediastore` Q26, Q29, Q34; `x_ss_delay` none (the fixed behaviour is identical).
- **Fixed behaviour** (`fixed_behaviour_differs_where_documented`): Q19, Q25, Q26, Q29, Q32, Q34
  without and with their flags.
- **Pause and fork** of Slingshot runs equal the uninterrupted run in both RNG modes; the
  measurement observer sees every tuple.

### Corpus sweep

The corpus models that run in Slingshot without handler errors (`reference/slingshot/results/det-verify.csv`;
without the SPD examples and the livelocking `x_pem_fork`), in `det` with the seed and simulated time
of that table, against the fast mode with `--quirks fast` (S2, 2026-10-17):

| Result | Models |
|---|---|
| byte-identical (38) | h01–h04, h07, h08, h14–h17, h19, h19b, h20, h22, h23, h25, h29–h32, x_pem_hdd_resource_signatures, x_pem_linking_resource, x_pem_minimum, x_pem_minimum_network, x_pem_reliability, x_pem_setvariable, x_pem_simple_heuristics, x_pem_simple_heuristics_qb, x_pem_subsystem, x_pem_subsystem_nested, x_sl_elsc_return, x_sl_loadbalancer, x_sl_simulizar111, x_ss_* (5) |
| equal up to 1.5e-8 relative (3) | h05, h06, x_pem_simple_optimisation: `Gamma`, `Lognorm`, `LognormMoments` samples differ in the last digits; Slingshot runs on Palladio 6.0's probability-function library, SimOxide reproduces SimuLizar 5.2.2's numerical inversion |
| draw order within an instant (2) | h09 (two users draw their loop counts at t = 0 before either draws a demand), h24 (two scenarios) |
| same-time order (1) | x_espresso, see below |
| different row count (1) | h21: the distributions above, probably via the sampler differences |
| no reference (2) | x_pem_acquire (Slingshot aborts before t = 0, Q27; the fast mode with Q27 aborts with the same message), x_sl_runconfigtest (Slingshot fails at set-up) |

Models with silent stalls (h15, x_pem_subsystem, x_pem_reliability, x_sl_elsc_return: no rows in
Slingshot) are reproduced with Q30 and Q35.

### Statistics against stock Slingshot (30 seeds)

`reference/slingshot/tools/compare_stat.py`: per series and run the count, mean and time-weighted
mean; Welch and Mann-Whitney tests on the per-run values, a Kolmogorov-Smirnov test on the pooled
values, Holm's correction at α = 0.01; a deterministic side is checked against the other side's
range. A = stock Slingshot, seeds 1–30; B = the fast mode, seeds 1–30.

| Model | B | Tests | Rejected |
|---|---|---|---|
| `x_ss_mediastore` (200 000 s) | exact RNG, `--quirks fast` | 26 | 0 |
| | fast RNG, `--quirks fast` | 26 | 0 |
| | exact RNG, fixed | 55 | 41: the documented fixes (Q29: 29 extra response-time series; Q26: about a third fewer state samples; Q34: per-core state, 1.0 and 0.5 instead of the total) |
| `x_espresso` (100 s) | exact or fast RNG, `--quirks fast` | 10 | 3, all on the state series' sample count, sample mean and pooled values |
| | fixed | 12 | 5 (the same three, plus the Q29 series) |

`x_espresso` has 30 closed users without think time and a constant demand, so every arrival and
completion is a tie. Response times (990 per run, all 3.0 s in both), demands and the time-weighted state
(30) are equal; the **event-sampled** state series is not: Slingshot's breadth-first cascade lets
about 13 tied completions pass before a finished user arrives again, the fast mode's processes
return after two notes. Sample mean of the state: stock 22.72 (range 22.51–22.93), `det` 21.12, fast
mode 28.13; tuples per run: stock 3 016–3 017, `det` 3 016, fast mode 3 027 (one stale-completion
sample per arrival, Q26). Slingshot's own deterministic order (`det`) is outside the stock range as
well, so this statistic depends on the order of simultaneous events, not on the model: it is the
exact mode's domain (X2), not a fast-mode criterion (see `DESIGN-SPD.md`, S2 as built).

## Scaling policies

SPD runs in the Slingshot flavour (fast mode, Slingshot's quirks: `--flavour slingshot --quirks
fast --spd-quirks slingshot`, with the `reconfiguration` feature) are compared with stock
Slingshot on two stochastic scenarios (`reference/slingshot/models/spd_v4_cpu`, `spd_v4_rt`:
exponential arrivals and demands, 600 s, group size 1..4; CPU-utilisation policies out above
80 % and in below 60 %, or a response-time policy out above 20 s), 30 seeds per side
(`reference/slingshot/spd-stat.sh`, `CONTROL=1`). Per run: executed adjustments, scale-outs,
scale-ins, the first adjustment's time, the time-weighted and final group size, the mean
response time in four windows (`tools/spd_stat.py`: Welch and Mann-Whitney, KS on the pooled
adjustment times, Holm 0.01), and every measurement series (`tools/compare_stat.py
--created-from`: the series of new containers pooled, since both sides name them differently).

| Scenario | SPD statistics rejected | series tests rejected | control: stock seeds 1-30 vs 31-60 |
|---|---|---|---|
| `spd_v4_cpu` (15 adjustments per run: 8 out, 5.5 in) | 0 of 21 | 1 of 42 | SPD 0 of 21, series 3 of 42 |
| `spd_v4_rt` | 0 of 20 | 2 of 42 | SPD 0 of 20, series 1 of 42 |

Every rejected series test is a KS test on pooled values (the CPU's event-sampled state; on
`spd_v4_rt` also that of the new containers), which rejects stock Slingshot against itself as
well: a run's values are autocorrelated, so the pooled KS is no valid test here (as for the
reconfiguration rules, `DESIGN-SPD.md` R1). Exact SPD runs (level E against `det`, the three SPD
cases of `corpus-ss/` and `corpus-spd/`) wait for the exact engine's SPD events (X2d).

## The exact mode

The Slingshot-exact mode (`--flavour slingshot-exact`, design phase X2) is compared with the
Slingshot-exact corpus `corpus-ss/` ([SS-X-8](../spec/slingshot/exact.md#ss-x-8-the-slingshot-exact-corpus)):
90 cases (the 20 probes, the 3 SPD examples, the 38 clean corpus models, 25 failure models, 2
runs that end before t = 0, and 2 cases for quirk Q37), each with the `det` runner's outputs. A
case counts as equal when the level-E projection, the tape projection (every draw with stream,
index, value, site and position) and `measurements.csv` are byte-identical
(`crates/simoxide-sim/tests/slingshot_exact.rs`; `reference/slingshot/tools/compare_exact.sh`
from the shell).

| Result | Cases |
|---|---|
| byte-equal (89) | all 20 probes, all 38 clean corpus models, 24 of the 25 failure models (so 21 of the 22 handler-error cases, with their disposed handlers and partial side effects: Q17, Q19, Q21-Q24, Q30, Q31), x_pem_acquire and x_sl_runconfigtest (the run fails before t = 0, as in Slingshot), the two Q37 cases, and the 3 SPD examples (spd_minimal: 1 adjustment, spd_scale_in: 2, scale-out and scale-in with Q5; spd_time_policies: 8), with Slingshot's adjustment run by QVTo's ordered mode inside the engine (X2d) |
| `java.util.HashMap` order (1) | h27: two `INNER` characterisations are evaluated in the iteration order of SimuCom's `HashMap` frame, which only OpenJDK's sources define; the EPL build iterates in insertion order (a licence-bound deviation) |

These counts hold for builds with `hotspot-math` (the tests). The default EPL build computes
`java.lang.Math.log`/`exp` correctly rounded instead of with HotSpot's intrinsics (the
`hotspot-math` code is GPL); there `h21_stoex_distributions` differs in the last digit of one of
its 406 samples (Palladio's lognormal uses `Math.exp`), so 88 of 90. Palladio 6.0's own
`FastMath` (Commons Math 3) is ported exactly and needs no opt-in. The SPD cases need no opt-in
either: the adjustments run in QVTo's ordered mode, which every build has.

The SPD cases are also checked beyond level E: the decision log (`spd.jsonl`: evaluations,
requests, adjustments with their change counts and model summaries) equals the reference's, and
the four cases of the SPD corpus `corpus-spd/` (with `spd_service_group`: three service-group
adjustments) are byte-equal too (`spd_decision_logs_match_the_reference`,
`corpus_spd_is_byte_equal_at_level_e`). Ids of elements an adjustment creates appear under the
reference runner's aliases (`~EClass:entityName`, else the container's key plus
`/feature.index`); two created elements with equal aliases keep separate series, written one
after the other, as the reference's EDP2 measurements.

## Reproducing

```sh
cargo test --release -p simoxide-sim --test slingshot
reference/slingshot/run.sh --stock -model corpus/x_ss_mediastore -out A/seed1 -simTime 200000 -seed 1 -instrument false
simoxide run --model corpus/x_ss_mediastore --flavour slingshot --quirks fast --mode fast --seed 1 \
    --max-sim-time 200000 --max-measurements -1 --measurements B/seed1/measurements.csv
reference/slingshot/tools/compare_stat.py A/seed* -- B/seed*
```

# Slingshot reference outputs for the S2 fast-mode checks

`det`-mode measurements of the Slingshot corpus models, made with the reference runner
(`../../run.sh -model ../../../corpus/<model> -out DIR -simTime T -seed N -trace false -spd false`,
nightly `1.0.0.202609010045` pinned by `../../install.lock`), gzipped, with the runner's
`summary.json`:

| Model | simTime (s) | Seeds |
|---|---|---|
| `x_ss_minimal`, `x_ss_delay`, `x_ss_minimalexample`, `x_ss_minimalexample_open_ps` | 1000 | 1, 2, 3 |
| `x_ss_mediastore` | 200000 | 1, 2, 3 |
| `x_espresso` | 100 | 1, 2, 3 (no random draws: the three are equal) |

`crates/simoxide-sim/tests/slingshot.rs` runs SimOxide's Slingshot flavour (exact RNG mode,
`--quirks fast`) on the `x_ss_*` models and compares byte for byte. `x_espresso` is compared
statistically (`docs/correctness/slingshot.md`).

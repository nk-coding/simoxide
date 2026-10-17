# reference/slingshot: headless Slingshot reference runner

The oracle for SimOxide's Slingshot flavour and its SPD support: the Slingshot nightly (read-only
install, pinned by `install.lock`) in a private Equinox configuration, plus

- `src/stub`: the `ProfilesLibrary` blackbox stub, without which no SPD adjustment can run;
- `src/runner`: the headless application (`-application simoxide.slingshot.runner.app`);
- `src/instrument`: a Slingshot behaviour extension writing `trace.jsonl`, `tape.jsonl`, `spd.jsonl`;
- `src/seed-agent`: the Java agent that seeds `Math.random()` and UUIDs and (det mode) makes
  Slingshot's, QVTo's and OCL's hash-ordered collections iterate in insertion order;
- `patches/`: source patches of installed classes (`orig/` = upstream source, `src/` = patched).

Documentation: `docs/reference-simulator/slingshot.md`.

```
./setup.sh                                         # build/ (gitignored): bundles, agent, patched jars, configs
./run.sh -model models/spd_minimal -out /tmp/o -simTime 100 -seed 1      # det mode (default)
./run.sh --stock -model ... -seed 1                # unpatched nightly, seeded randomness only
./smoke.sh                                         # smoke corpus, determinism, times -> results/smoke.csv
./install.sh                                       # fresh install from the p2 update sites (network)
```

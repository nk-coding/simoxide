# Command line

The workspace builds these binaries:

| Binary | Crate | Purpose |
|---|---|---|
| `simoxide` | `simoxide-cli` | run a model, pause and resume it, explore variants, benchmark it |
| `simoxide-bench` | `simoxide-cli` | the benchmark suite |
| `simoxide-fuzz` | `simoxide-testkit` | differential testing against the reference |
| `qvto` | `qvto-cli` | the QVTo engine on its own ([QVTo usage](../qvto/usage.md)) |
| `simoxide-thirdparty` | `simoxide-thirdparty` | set up and check the third-party test models ([Licensing](./licensing.md#simoxide-thirdparty)) |

`simoxide --help` prints the full usage. Which flags work in which mode:
[Modes and features](./modes.md#feature-matrix).

## `simoxide run`

Runs one simulation. The flags mirror `reference/refsim run`.

```text
simoxide run --model <dir> [flags]
```

| Flag | Meaning |
|---|---|
| `--model DIR` | model directory |
| `--run-json FILE` | run configuration (default: `DIR/run.json` if present) |
| `--seed N` | seed |
| `--max-sim-time T` | stop at simulated time `T` seconds (`-1` = off) |
| `--max-measurements M` | stop after `M` finished usage-scenario runs (`-1` = off) |
| `--no-link-throughput` | do not simulate the throughput of linking resources |
| `--measurements FILE` | write `measurements.csv` |
| `--trace FILE` | write the event trace |
| `--tape FILE` | write the random tape (exact mode only) |
| `--replay-tape FILE` | take the uniforms from a recorded tape instead of the generator |
| `--check-origins` | with `--replay-tape`: fail at the first draw made at a different place |
| `--name NAME` | run name in the trace header |
| `--mode exact\|fast` | random-number mode, default `fast` ([Fast mode](./fast-mode.md)); `exact` needs the exact build ([Getting started](./getting-started.md#build)) |
| `--ps-algorithm exact\|virtual-time` | processor-sharing algorithm, default `exact` ([Deviations](../correctness/deviations.md#virtual-time-processor-sharing)) |
| `--flavour simulizar\|slingshot\|slingshot-exact` | whose semantics, default `simulizar` ([Slingshot-compatible mode](./slingshot.md)); `slingshot-exact` ignores `--mode`, needs `--max-sim-time`, and `--trace`/`--tape` write its event and tape projections |
| `--quirks LIST` | Slingshot compat flags: `Q1`..`Q37`, `fast`, `all`, `none`, `-Qn`; default `none` (the fixed behaviour); `--quirks list` prints them; `slingshot-exact` runs `all` ([Quirks](../spec/slingshot/quirks.md#compat-flags)) |
| `--usage-streams` | fast mode: the usage model draws from streams of its own (common random numbers across designs; [RND-11](../spec/random.md#rnd-11-usage-streams-opt-in-fast-mode-not-the-reference)) |
| `--spd-log FILE`, `--spd-log-all` | write the decision log of the model's scaling policies ([Scaling policies](./spd.md)) |
| `--spd-quirks fixed\|slingshot\|Q2,…` | Slingshot SPD quirks to reproduce, default `fixed` |
| `--no-spd` | ignore a `.spd` file of the model |
| `--hpa FILE`, `--no-hpa` | Kubernetes-style HPA autoscalers of SPD target groups: `FILE` instead of the model's `*.hpa` / `*.hpa.json` files, or none ([Autoscaling (HPA)](./hpa.md)) |
| `--load-profile FILE`, `--no-load-profile` | a time-varying load: `FILE` instead of the model's `*.loadprofile.json` files, or none ([Load profiles](./load-profile.md)) |
| `--rules DIR`, `--rules-log FILE` | run SimuLizar's reconfiguration rules of `DIR` (default: `run.json`'s `reconfiguration_rules`) and log each reconfiguration; feature `reconfiguration`, SimuLizar flavour, not together with an SPD ([Reconfiguration rules](./reconfiguration.md)) |
| `--qvto-mode auto\|ordered\|fast\|compat` | QVTo's mode for rules, SPD/HPA adjustments and `resume --transform`; `auto` (default) is `ordered` for rules and adjustments, which equals both references; `compat` needs a build with `exact` or `compat` ([QVTo modes](../qvto/index.md#modes)) |
| `--pause-at T`, `--pause-before T`, `--pause-events N` | stop between two events: after every event at or before `T` s, before the first event at or after `T` s, or after `N` events; prints `paused t=...` and writes the measurements so far ([Snapshots](./snapshots.md)) |
| `--pause-when COND` | pause when a condition holds (repeatable): `time T`, `before T`, `events N`, `elapsed T`, `elapsed-events N`, `METRIC[POINT] [mean W] OP V [for W]`, `slo METRIC[POINT] upper U [lower L] [sensitivity S] [delay D]` ([Pause conditions](./snapshots.md#pause-conditions)) |
| `--snapshot FILE`, `--snapshot-format postcard\|json` | write the paused run to `FILE` ([Snapshot format](../formats/snapshot.md)) |

Limits (see [Deviations](../correctness/deviations.md#resource-limits-for-untrusted-models)):

| Flag | Default |
|---|---|
| `--max-events-per-instant N` | 20 000 000 (livelock guard, `0` = off) |
| `--max-stack-depth N` | 10 000 continuations (about 2 000 nested calls) |
| `--max-processes N` | 1 000 000 |
| `--max-events N`, `--max-steps N` | off |
| `--timeout SECONDS` | off |

On success, `run` prints a summary line on stderr (`t_end`, `events`, `uniforms`,
`measurements`, `main_count`, wall time) and exits with 0. A load, compile or simulation error
exits with 1 (`error: ...`), a usage error with 2. Compile warnings, for example about monitors
that cannot be recorded, are printed as `warning: ...`.

## `simoxide resume`

```text
simoxide resume FILE --model DIR [--run-json F] [--trace F] [--tape F] [--measurements F]
                [--pause-at T ... --snapshot FILE2] [--until T] [--pause-when COND]...
                [--spd-log F] [--spd-quirks ...] [--rules DIR] [--rules-log F] [--qvto-mode M]
                [--timeout S] [--change FILE.changes]... [--transform X.qvto]...
                [--enforce POLICY,...] [--hpa-set NAME.PARAMETER=VALUE]...
simoxide snapshot-info FILE
```

Continues a snapshot on the model it was taken of (checked by fingerprint); mode, flavour and
compat flags come from the snapshot. The outputs receive the continuation only. `--until T` is
`--pause-at T`. `snapshot-info` prints the snapshot's header as JSON. See
[Snapshots](./snapshots.md).

| Flag | Meaning |
|---|---|
| `--change FILE` | apply a change file before the run continues ([Change files](./snapshots.md#change-files)); feature `reconfiguration` |
| `--transform X.qvto` | run a QVTo transformation once on the run's models; feature `reconfiguration` |
| `--rules DIR` | other rules than the snapshot's replace them; on a snapshot without rules, they are added |
| `--enforce POLICY,...` | adjust by these scaling policies at the resume, past their triggers and constraints ([Scaling policies](./spd.md#command-line)); not with `slingshot-exact` |
| `--hpa-set NAME.PARAMETER=VALUE` | change an HPA autoscaler's parameter ([Autoscaling (HPA)](./hpa.md#command-line)) |

The change steps run in the order given.

## `simoxide explore`

```text
simoxide explore FILE --model DIR --variant NAME[=FILE.changes|=X.qvto]... [--until T]
                 [--threads N] [--metrics rt,util,...]
```

Forks a snapshot into one run per variant, each with its changes, runs them to the end (or `T`)
in parallel and prints a table of their metrics. Feature `reconfiguration`; snapshots of
`simulizar` and `slingshot` runs without rules or scaling policies
([Exploration](./snapshots.md#exploration)).

## `simoxide bench`

```text
simoxide bench --model <dir> [--runs N] [--threads T] [run flags]
```

Runs the model `N` times (default 20) on `T` threads and prints events/s and runs/s. With the
`profile` cargo feature, `--profile out.svg` or `--profile out.folded` records a sampling profile.

## `simoxide load-bench`

```text
simoxide load-bench --model <dir> [--runs N]
```

Measures XMI loading plus compilation only.

## `simoxide-bench`

```text
simoxide-bench suite  [--filter a,b] [--reps N] [--short-secs S] [--mode exact|fast] [--flavour simulizar|slingshot] [--quirks LIST]
simoxide-bench batch  [--model DIR] [--max-measurements M] [--runs N] [--threads 1,2,4,8] [--mode exact|fast] [--flavour ...]
simoxide-bench fork   [--filter a,b] [--reps N] [--mode exact|fast]
simoxide-bench change [--filter a,b] [--reps N] [--mode exact|fast]
```

`suite` runs the benchmark models (load, compile, a short and a long run, allocations, peak heap);
`batch` measures thread scaling; `fork` the cost of a fork, a detached copy and a resume at half
time; `change` the cost of a runtime model change. See [Benchmarking](../development/benchmarking.md).

## `simoxide-fuzz`

| Command | Purpose |
|---|---|
| `fuzz` | generate models, run them in the reference and in a candidate, compare exactly |
| `validate` | reference pass rate and determinism on generated models |
| `corpus` | run a corpus directory against a candidate |
| `long` | long runs, measurements only |
| `equiv` | exact vs fast mode, statistically |
| `gen` | write one generated model |
| `diff` | first divergence of two traces |

See [Testing](../correctness/testing.md) and [Test kit](../development/testkit.md).

## Cargo features

| Crate | Feature | Effect |
|---|---|---|
| `simoxide-cli` | `spd` (default) | scaling policies and HPA (`.spd`, `.hpa.json`, `--spd-log`, `--hpa`); without `reconfiguration`, decisions only |
| `simoxide-cli` | `fast` (default) | kept for compatibility: the fast mode is compiled in every build |
| `simoxide-cli` (and `simoxide-sim`, `-spd`, `-reconf`, `-testkit`) | `exact` (off) | the SimuLizar-exact mode (`--mode exact`, Java's `HashMap` order through `simoxide-jcompat-exact`); no licence granted, binaries built with it must not be distributed (`LICENSES`, sections 4 and 7) |
| `simoxide-cli` (and `simoxide-random`, `-stoex`, `-sim`) | `hotspot-math` (off) | HotSpot's `Math.log`/`exp` bit for bit (`simoxide-hotspot-math`, GPL-2.0-only; not distributable); with `exact`, the local exact build (`cargo build-exact`) |
| `qvto-cli` (and `qvto`, `qvto-runtime`, `-vm`, `-testkit`) | `compat` (off) | QVTo's compat mode (Eclipse's collection orders and identity hashes; `simoxide-jcompat-exact`, not distributable); fast and ordered mode are in every build ([QVTo licensing](../qvto/licensing.md)) |
| `simoxide-cli` | `profile` | `--profile` on `bench` and `load-bench` |
| `simoxide-cli` | `mimalloc` | mimalloc as the global allocator |
| `simoxide-cli` | `reconfiguration` (off) | the QVTo engine through `simoxide-reconf`: `--rules`, SPD and HPA adjustments, `resume --change/--transform`, `explore` (EPL, distributable) |
| `simoxide-sim`, `simoxide-spd`, `simoxide-reconf` | `snapshot` (off; always on in `simoxide-cli`) | snapshots of runs (serde, postcard; MIT/Apache-2.0) |
| `simoxide-random` | `fast` (off; always on through `simoxide-sim`) | the fast-mode generator and samplers (`simoxide-sim`'s `fast` feature is kept for compatibility) |
| `simoxide-random` | `rcp-table` | a recorded AMD Zen 5 `rcpss` table instead of the instruction ([Exactness](../correctness/#caveats)) |
| `simoxide-random` | `fast-bracket` | galloping bracket search for inverse CDFs (not proven exact) |

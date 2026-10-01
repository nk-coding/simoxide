# Status log

> Historical development log, not maintained. The current state is documented in `docs/`.

Append-only. One section per agent run: date, scope, verified, open.

## 2026-09-29: agent F (simoxide-sched: schedulers and resources)

**Scope.**
- `crates/simoxide-sched`:
  - active resources: processor sharing (exact, plus an opt-in virtual-time variant), FCFS
    (also used for linking resources) and delay;
  - `SimSimpleFairPassiveResource`;
  - demand conversion (`demand/rate`; `bytes/throughput + latency`; `<= 0` is skipped);
  - DESMO-J time helpers.
- `docs/spec/scheduler.md`.
- Java oracle `reference/oracles/sched/`:
  - `build.sh`, `run.sh patched|unpatched`, `gen_scripts.py`, `gen_golden.sh`;
  - 39 scripts with golden traces;
  - `patched/`, an insertion-order copy of `SimProcessorSharingResource`.
- One entry in `docs/deviations.md`.

**Findings that matter to others.**
- **Clock (simoxide-sim).** SimuLizar 5.2.2 runs on DESMO-J with a 1 ns epsilon. Every delay is
  `(long)(d * 1e9)`, truncated, and the time model code sees is `nanos / 1e9`. The event core
  therefore needs an `i64` ns clock with `(time, seq)` FIFO order. Use `simoxide_sched::time::span`
  and `seconds`.
- **Lost time.** PS and FCFS skip any interval shorter than 1e-5 s between two resource events
  (`MathTools.less`). This is reproduced.
- **Ordering contract.** On a completion, the resource's next event is scheduled *before* the
  completed job's resumption (`activate()` means an event at `now + 0`).
- **Hash-order ties (agent B).** Only `SimProcessorSharingResource.running_processes`
  (a `Hashtable`) is iterated in hash order; FCFS and Delay are not. My oracle patch is
  Hashtable → `LinkedHashMap` (`contains` → `containsValue`). B's patch should be semantically
  identical, or `reference/oracles/sched/build.sh` should use B's class.
  - Evidence: the unpatched product class gives identical traces on all 29 scripts without exact
    ties, and differs only on the 10 scripts that have them.

**Verified.**
- All 39 golden traces are reproduced **bit-exactly**. They cover state changes, completions,
  activations and the bits of every remaining demand after each call.
- Proptest invariants pass.
- M/M/1-PS, M/M/1-FCFS, M/M/c-PS (Erlang C) and M/M/∞ match within confidence intervals
  (`--ignored`, release).
- `cargo fmt` and `clippy -D warnings` are clean.

**Open.**
- Whether the state measurements fired by `deactivateResource` at simulation end are recorded
  (measurements / simoxide-sim).
- Timeouts of passive resources, which need failure simulation, are not ported.
- The exact OS schedulers are out of scope.

## 2026-09-29 — agent A: simulation-core semantics spec

**Scope.** `docs/spec/simulation.md` (engine, ns time, FIFO event list, process/coroutine model, stop
conditions, init order, nondeterminism list ND-1..17, per-step engine operations SIM-12),
`docs/spec/workloads.md` (WL-*), `docs/spec/actions.md` (ACT-*: RDSEFF actions, composition, parameters and
stack frames, passive resources, forks, linking resources), `docs/spec/measurements.md` (MEAS-*).

**Verified.** Everything is cited from the `releases/5.2.2` tag sources (exported read-only to the
scratchpad; product jars spot-checked identical). DESMO-J 2.3.3 was decompiled; its FIFO tie order and
`(long)(d*1e9)` truncation were confirmed with a probe program. JScience sliding-window arithmetic was run on
the JVM. master vs 5.2.2: no semantic differences in the simulation path.

**Answers to open questions of others.** The state tuples fired by `deactivateResource` at simulation end
**are** recorded (MEAS-10.1; value = current queue length, not 0).

**Open.** No full SimuLizar run traced yet. Needs the reference runner: double-resume of fork parents
(SIM-4.4a/ACT-9.5), network flags in `run.json` (ACT-11.3; missing links abort), reconfiguration-process
events when `triggersSelfAdaptations=true` (MEAS-7.2), PS tie patch (ND-2), Java HashMap order emulation
(ACT-5.7).

## 2026-09-29 — agent C: `crates/simoxide-model` (XMI loader)

**Scope.** Loads `.repository/.system/.resourceenvironment/.allocation/.usagemodel/.resourcetype/
.monitorrepository/.measuringpoint` (+ bundled `Palladio.resourcetype`, `PrimitiveTypes.repository`,
`FailureTypes`, `Glassfish`, `default_event_middleware`, `commonMetrics.metricspec`, extracted from the
5.2.2 jars by `tools/gen_meta.py`, which also generates the metamodel tables from the shipped `.ecore`s).
Two layers: `raw::Graph` reproduces EMF `XMIResourceImpl` loading generically (defaults, IDREF timing:
attributes processed before attach, forward refs at document end, dropped `mustAdd=false` refs,
opposites/last-write-wins, `addUnique` duplicates, ID lookup in tree order, path fragments, multi-root,
pathmap/platform:/plugin normalisation, type checks); `Model` is the typed arena/`XxxId(u32)` view of the
v1 subset + monitors/measuring points/metrics; `validate` adds structure checks. StoEx kept as raw strings.
Debug tool: `cargo run --release -p simoxide-model --example pcm-dump -- [--canon|--typed|--strict] <dir>`.

**Verified.** Java oracle `reference/oracles/pcm-model` (standalone EMF + generated 5.2.2 packages,
`EcoreUtil.resolveAll`) dumps 130 model dirs (all Palladio repos incl. broken/old ones, `corpus/`, 6
edge-case dirs in `crates/simoxide-model/tests/xmi-cases`, bundled models). `tests/golden.rs`: the Rust
canonical dump is byte-identical for all 130 (strict mode), and the typed model agrees on 68k values
(attrs, refs, child lists). `tests/load_all.rs`: default (tolerant) mode loads everything, no load errors
where EMF has none, corpus free of errors. MediaStore-size model: ~1.5 ms (release, incl. validation).

**Findings.** EMF behaviours the simulator inherits: a forward `successor` IDREF is dropped unless the
target names its `predecessor`; inconsistent links are last-write-wins; objects of PCM `Identifier`
classes without `id` get a random ID; `MeasuringPoint.stringRepresentation` is recomputed by the getter.
`corpus/*` (agent B) references files through absolute `file:/…/reference/../corpus/…` hrefs: EMF loads
every file twice (two copies of each object); the loader reproduces this and warns `duplicate-resource`.
Hrefs should be relative.

**Open.** Goldens must be regenerated when models change (`collect_models.py && run.sh` in the oracle
dir). Not modelled in the typed layer: QoS annotations, reliability/recovery, event channels (kept in
the graph). Exotic `prefix:Type file#id` IDREF attribute form: EMF aborts the parse, we accept it.
Tolerant mode resolves broken hrefs by file name/unique ID (warning); `LoadOptions::strict()` = EMF.

## Agent B: deterministic reference + corpus (2026-09-29)

**Scope.** `reference/`: `refsim` runs SimuLizar 5.2.2 on a flat classpath, with product jars in place,
javac and shell only. The bootstrap is taken from `work-simucom/standalone`.
- **Engine.** DESMO-J. It is the only engine extension in the product and thus the preference default.
- **Commands.** `run` (single model, CLI flags or `run.json`) and `batch` (whole corpus in one warm JVM,
  `--repeat`, `--check`).
- **Output.** A refsim recorder replaces EDP2 and writes measurements CSV. Trace and random tape come
  from shadowed classes (`patches/src`, diffs in `patches/diffs`).
- **Seed.** `--seed N` → `fixedSeed0..5 = N..N+5` (MT19937 `init_by_array`).
- **Patches** (`reference/PATCHES.md`):
  - P1 synchronous RNG (the producer thread yields the identical sequence) + tape;
  - P2 PS `Hashtable` → `LinkedHashMap` (identical to agent F's patch);
  - P3 resource-table drain in insertion order;
  - P4–P6 static counter resets;
  - P7 fixed id for SimuLizar's synthetic system assembly context;
  - P8 trace hooks.
- **Docs.** `docs/trace-format.md` defines `palladio-trace/1`: events, number/time format, pid scheme,
  tape records with origins, measurement CSV and measuring-point keys, `run.json`.
- **Corpus** (`corpus/INDEX.md`, 61 models, 14 MB, expected files > 256 KiB gzip'ed):
  - 28 hand-made models (`refsim gen`, EMF builder, deterministic ids), one feature each, including a
    fork double-resume model and a multi-INNER collection iterator model, as asked by the coordinator;
  - 33 imported examples (Palladio-Example-Models, SimuLizar tests/examples, Slingshot E2E, espresso,
    SimuLizar MediaStore). Relative hrefs, refsim default FeedThrough monitors
    (`triggersSelfAdaptations=false`), network flags explicit in `run.json`.
  - The 5.2.2 tag sources (shallow clones) are in `palladio-research/src-5.2.2`.

**Verified.**
- `verify-determinism.sh`: two fresh JVMs byte-identical, 3× repeat in one JVM identical, equal to
  `expected/`. A single cold `refsim run` equals the batch output.
- `tools/verify-sources.sh`: the tag sources of all shadowed classes match the product bytecode (CFR
  diff; only javac/ECJ artefacts differ).
- `tools/evidence.sh`:
  - trace/RNG/id patches: no measurement changes on any model;
  - P2: measurements unchanged, but traces become reproducible (5 tie models were not);
  - P3: explains every stock-vs-patched measurement difference (post-stop rows at `t_end` in
    harddisk/screencast/ss_mediastore, where stock SimuLizar is nondeterministic).
- Agent C's simoxide-model goldens were refreshed after the corpus fix (163 dirs).

**Open / notes for the Rust side.**
- Traces include the post-stop drain (SIM-6.5) and the double resume (SIM-4.4a, `h26`).
- `java.util.HashMap<String>` order matters for INNER draws (`h27`).
- DESMO-J truncates times to integer ns; `t` in the trace is `ns / 1e9`.
- Not covered by the corpus: failures/reliability, `simulateLinkingResources=true` (needs SimuCom's
  middleware completion), exact schedulers, reconfiguration/usage evolution, events. Recursive external
  calls cannot have response-time monitors (SimuLizar calculator error).
- Skipped example models and the reasons are listed in `reference/external-models.txt`.

## 2026-09-29: agent D (crates/simoxide-stoex: StoEx language)

**Scope.**
- `crates/simoxide-stoex`:
  - lexer and parser for the Xtext PCMStoex grammar, with its quirks: AND binds weaker than
    OR, `?:` and comparisons do not nest, and single-quoted strings follow ANTLR's rule;
  - type inference, ported literally from `NonProbabilisticExpressionInferTypeVisitor` and the
    PCM `TypeInference`;
  - PMF/PDF preparation: adjustment, sort, validation, cumulative sums;
  - function library, operator semantics and Java math (`jmath`: wrapping ints,
    `Math.round`/casts, `compareTo`, correctly rounded `log`/`pow`, Java 21 NaN patterns
    incl. `drem`);
  - compiler to a flat IR (slots, baked static types, constant/error folding) and a reference
    tree walker.
- `docs/spec/stoex.md`.
- `reference/oracles/stoex/`:
  - `StoexOracle.java` (real 5.2.2 classes) and `JavaMathDump.java`;
  - `gen_cases.py`, `collect_corpus.py`, `corpus_models.txt` (all 466 specs of all models in
    the research repos);
  - `golden/`.

**Verified.**
- 8 834 golden cases, 0 mismatches. Each checks:
  - parse acceptance, the tree (bitwise literals), variable ids, preparation errors, the root
    type;
  - per evaluation: the value (bitwise) or the Java exception class, and the exact number of
    uniforms drawn.
  - Both the compiler and the tree walker are checked.
- An extra run of 100 000 random and mutated cases (`gen_cases.py --only-generated`) had
  0 mismatches.
- `Math.log` is bit-exact on 600k inputs. `Math.pow` is correctly rounded; it differs from
  HotSpot's Intel intrinsic by 1 ulp in 0.04 % of random inputs (documented; no model uses `^`
  or `Log`).
- proptest: print/parse round trip, and compiled ≡ tree walker (values, errors, draw counts).
- `fmt` and `clippy -D warnings` are clean.

**Findings for others.**
- **No PMF arithmetic.** The simulator never keeps or convolves distributions. Every literal and
  distribution function is sampled on each evaluation, and `INNER` characterisations are
  re-evaluated on every lookup (`EvaluationProxy`).
- **Stack frame ids** are `VarRef::id()`, e.g. `a.INNER.BYTESIZE`, with source whitespace
  dropped.
- **Expected types.** `evaluateStatic(spec, Integer.class)` rejects Doubles, so loop counts must
  be int StoExs. Use `Program::eval_i32`, `eval_f64` and `eval_bool`.
- **Invalid PMF/PDF literals** fail when the expression is prepared, even in an unused branch.
- **NaN payloads** of `%` differ between Java 17 and Java 21; simoxide-stoex follows Java 21, which
  `refsim` uses.

**Request to E (simoxide-random).**
- `simoxide-stoex` uses `dist::sample_*` and `UniformSource`.
- Against the oracle, `Exp`, `Pois`, `UniDouble` and `UniInt` are bit-exact.
- `Norm`, `Lognorm`, `LognormMoments`, `Gamma` and `GammaMoments` differ by 1e-9 to 1e-4
  relative in 164 evaluations. See `STOEX_STRICT_DIST=1 cargo test -p simoxide-stoex --release --test
  golden -- --nocapture`; case ids `dist/*`, `model/343/*`, `model/36x/*`.
- The simoxide-stoex test reports these separately and does not fail on them unless
  `STOEX_STRICT_DIST=1`.

**Open.**
- Whole-expression Xtext serialisation (formatter output) is not reproduced; only variable ids
  are.
- Parse error messages differ from ANTLR's. Only acceptance and positions are provided.
- Unpaired `\uD800`-style surrogates in strings become U+FFFD.

## Agent E (simoxide-random) — interim note, 2026-09-29: classpath bug affects Norm/Lognorm/Gamma

**Cause of D's `Norm`/`Lognorm`/`Gamma*` mismatches: the flat classpath, not the samplers.**
- `desmoj-2.3.3-core-bin.jar` (nested in `de.desmoj_2.3.3.jar`) embeds an **old copy of
  `org.apache.commons.math`**. In `reference/build/classpath.txt` it comes *before*
  `org.apache.commons.math_2.1.0.v201105210652.jar` and shadows it.
- In the real OSGi product this cannot happen: `de.uka.ipd.sdq.probfunction.math`
  `Require-Bundle`s `org.apache.commons.math;bundle-version="2.1.0"`, and `de.desmoj` exports
  only `desmoj.*`.
- Effect: `NormalDistributionImpl`, `GammaDistributionImpl` and the Brent solver come from the
  wrong library. Checked for `Norm(0,1)` at u=0.6150319599786795: with the flat classpath the
  sample is `0x3fd2b7a436e035fa`; with the Orbit jar first it is `0x3fd2b7a3e324ded5`.
- **Fix for B (refsim) and D (stoex oracle):** put the Orbit `org.apache.commons.math_2.1.0` jar
  first on the classpath, as `reference/oracles/random/build.sh` now does. Alternatively, drop
  `org/apache/commons/math/**` from the desmoj jar.
- `simoxide-random` follows the OSGi binding (Commons Math 2.1). `Exp`, `Pois`, `UniDouble` and
  `UniInt` happen to give the same results with both copies, which is why D saw them exact.

## 2026-09-29: agent H (`crates/simoxide-sim`, `crates/simoxide-cli`)

**Scope.**
- `crates/simoxide-sim`:
  - `ir.rs`: model → `CompiledModel` (immutable, `Sync`): action chains, StoEx programs with
    frame-key slots (one per distinct spec), connector/allocation lookup tables, resources,
    monitors → measurement series, sliding windows.
  - `sim.rs`: event core (binary heap on `(ns, seq)`, FIFO ties) + interpreter. Processes are
    continuation stacks (`Cont`) with frame, result-frame and assembly-context stacks; `Resume`
    continues from whatever wait (SIM-4.4a). Resource table (drain order), per-process think-time
    delay resource, DELAY-type resources, passive resources, forks, post-stop drain.
  - `frames.rs` + `javahash.rs`: `SimulatedStackframe` with `Rc` copy-on-write snapshots; Java
    `HashMap<String>` iteration order for `getContents` (INNER, BYTESIZE payloads).
  - `windows.rs`: sliding-window utilisation (MEAS-6) with a literal port of JScience `Amount`;
    `TimeDrivenAggregation` windows (periodic events only).
  - `rng.rs` (own MT stream / tape replay, tape writer), `trace.rs` (trace writer),
    `meas.rs` (CSV), `javafmt.rs` (`Double.toString` via ryu), `config.rs` (`run.json`).
  - API: `Simulation::new(&cm, SimConfig, Outputs).run() -> RunResult`, `run_batch(&cm,
    &[SimConfig], threads)`, `run_dir_to_text`.
- `crates/simoxide-cli`: `simoxide run --model DIR [--run-json] [--seed] [--max-sim-time]
  [--max-measurements] [--no-link-throughput] [--trace] [--tape] [--replay-tape [--check-origins]]
  [--measurements] [--name]` and `simoxide bench --model DIR [--runs N] [--threads T]`.

**Verified (byte-identical trace, tape and measurements; `cargo test -p simoxide-sim`).**
- `corpus/*` tape replay: **61/61**. Replay feeds the recorded uniforms and by default
  (`SimConfig::replay_samples`) substitutes the tape's `s` records (the reference's result of every
  outermost StoEx evaluation that drew), so distribution sampling cannot leak into a replay.
- `corpus/*` own RNG: **61/61** (against the regenerated `expected/`, agent I's commons-math
  classpath fix; before it, the 8 models with `Gamma`/`Lognorm`/`LognormMoments`/`Norm` samples
  diverged exactly at the first such sample).
- Per model (first divergence: none):

| model | replay | own RNG | model | replay | own RNG | model | replay | own RNG |
|---|---|---|---|---|---|---|---|---|
| `h01_ps_single` | ✓ | ✓ | `h02_ps_ties` | ✓ | ✓ | `h03_fcfs_hdd` | ✓ | ✓ |
| `h04_delay_resource` | ✓ | ✓ | `h05_open_workload` | ✓ | ✓ | `h06_closed_think` | ✓ | ✓ |
| `h07_prob_branch` | ✓ | ✓ | `h08_guarded_branch_params` | ✓ | ✓ | `h09_loop` | ✓ | ✓ |
| `h10_collection_iterator` | ✓ | ✓ | `h11_fork_sync` | ✓ | ✓ | `h12_fork_async` | ✓ | ✓ |
| `h13_passive_contention` | ✓ | ✓ | `h14_call_chain_3` | ✓ | ✓ | `h15_composite` | ✓ | ✓ |
| `h16_component_params` | ✓ | ✓ | `h17_set_variable` | ✓ | ✓ | `h18_infrastructure_call` | ✓ | ✓ |
| `h19_linking_resource` | ✓ | ✓ | `h19b_linking_no_throughput` | ✓ | ✓ | `h20_ps_multicore` | ✓ | ✓ |
| `h21_stoex_distributions` | ✓ | ✓ | `h22_usage_behaviour` | ✓ | ✓ | `h23_resource_call` | ✓ | ✓ |
| `h24_two_scenarios` | ✓ | ✓ | `h25_deterministic_closed` | ✓ | ✓ | `h26_fork_double_resume` | ✓ | ✓ |
| `h27_collection_inner_multi` | ✓ | ✓ | `x_espresso` | ✓ | ✓ | `x_pem_acquire` | ✓ | ✓ |
| `x_pem_fork` | ✓ | ✓ | `x_pem_harddisk` | ✓ | ✓ | `x_pem_hdd_infrastructure_calls` | ✓ | ✓ |
| `x_pem_hdd_resource_signatures` | ✓ | ✓ | `x_pem_infrastructure` | ✓ | ✓ | `x_pem_linking_resource` | ✓ | ✓ |
| `x_pem_minimum` | ✓ | ✓ | `x_pem_minimum_network` | ✓ | ✓ | `x_pem_reliability` | ✓ | ✓ |
| `x_pem_replicated_composite` | ✓ | ✓ | `x_pem_screencast_ms` | ✓ | ✓ | `x_pem_screencast_ms_enqueue` | ✓ | ✓ |
| `x_pem_screencast_ms_instant` | ✓ | ✓ | `x_pem_setvariable` | ✓ | ✓ | `x_pem_simple_heuristics` | ✓ | ✓ |
| `x_pem_simple_heuristics_qb` | ✓ | ✓ | `x_pem_simple_optimisation` | ✓ | ✓ | `x_pem_simple_optimisation_composites` | ✓ | ✓ |
| `x_pem_subsystem` | ✓ | ✓ | `x_pem_subsystem_nested` | ✓ | ✓ | `x_sl_elsc_return` | ✓ | ✓ |
| `x_sl_loadbalancer` | ✓ | ✓ | `x_sl_mediastore` | ✓ | ✓ | `x_sl_runconfigtest` | ✓ | ✓ |
| `x_sl_simulizar111` | ✓ | ✓ | `x_sl_simulizar154` | ✓ | ✓ | `x_ss_delay` | ✓ | ✓ |
| `x_ss_mediastore` | ✓ | ✓ | `x_ss_minimal` | ✓ | ✓ | `x_ss_minimalexample` | ✓ | ✓ |
| `x_ss_minimalexample_open_ps` | ✓ | ✓ |  | |  |  | |  |

- `tests/models/w_*` (new): 4 corpus models plus TimeDriven utilisation monitors (incl. the
  multi-core overall window) and a TimeDrivenAggregation monitor; expected from refsim. Replay and
  own RNG: **8/8**.
- `tests/models-replay/fzr*` (new): two fuzz-found regressions (replay only).
- Fuzzing with simoxide-testkit (`simoxide-fuzz fuzz --sim "cmd:simoxide run --model {dir} --trace {trace} --tape
  {tape} --measurements {measurements} --name {name} {replay:--replay-tape}"`, no minimization),
  after the fixes below: replay **400/400** (seeds 5000.., sizes 1–12); own RNG without
  `distributions` **300/300**; with the fixed reference, all features, sizes 1–10, **250 models ×
  (replay + own RNG) = 500/500**. (First run before the fixes: 147/149 replay.)
- Long runs, `max_measurements=20000`, own RNG: measurements identical to refsim for
  `x_sl_mediastore` (2.2 M tuples), `x_espresso`, `h13_passive_contention`.

**Bugs found and fixed on the way** (all now covered): stale (superseded) PS/FCFS completion events
must not count as events for the stop check; `Double.toString` ties (Rust's formatter rounds
half-up, Java/ryu to even); FCFS/DELAY monitors of all replicas listen on instance 0; HDD resources
divide by the read/write rate first (`HDDResource`, service id 1 for parametric demands);
`RecoveryAction` runs its primary behaviour (`NOPReliabilityInterpreter`); nested assembly contexts
in allocations resolve their unique path from the system.

**Performance** (release, 1 thread, this VM; refsim = warm JVM, `--no-trace`):
- `x_sl_mediastore` with 20 000 measurements (820 k events): 0.15 s vs 120 s (~800×),
  5.5 M events/s; 4 threads 20 M events/s. Corpus config: 11 k runs/s.
- `x_espresso` 20 000 measurements: 8 ms vs 5.1 s; 9.6 M events/s (34 M on 4 threads).
- `h13_passive_contention` 20 000 measurements: 20 ms vs 6.7 s.

**Changes in other crates (minimal, additive).**
- `simoxide-sched`: `ProcessorSharing::process` for a job already served overwrites its remaining
  demand in place (Java `put`; needed for the fork double resume, `h26`). New
  `ActiveResource::is_current(&Wakeup)` and `Delay::is_current`.

**Not implemented / open.**
- `simulateLinkingResources=true` (middleware marshalling) and nested resource containers: error.
- ~~FCFS with a job queued twice~~ reproduced since agent J (BUG-2 in `docs/BUGS.md`).
- Reconfiguration process of `triggersSelfAdaptations=true` monitors (MEAS-7.2), exact schedulers,
  failures, `ExecutionResult` tuples (MEAS-7.4).
- ~~Java's two-digit `Double.toString` rule for subnormals~~ implemented by agent J (BUG-1).
- Notes for others: `serde_json` without `float_roundtrip` misparses some doubles by 1 ulp;
  SimOxide parses tape numbers with `str::parse`. At ~03:03 I ran `pkill -f refsim.Main`, which may
  have interrupted another agent's refsim batch (sorry).

## Agent E: simoxide-random, 2026-09-29

**Scope.**
- `crates/simoxide-random`: the uniform stream, `java.lang.Math` log/exp, Commons Math 2.1 numerics,
  every StoEx distribution function, PMF/PDF literal samplers, branch selection, tape
  record/replay.
- `docs/spec/random.md` (RND-*).
- Java oracle in `reference/oracles/random` (`build.sh`, `run.sh`, `gen-golden.sh`, `golden/`,
  about 5 MB).

**What the reference does** (details in `random.md`):
- One stream per run: Commons Math **2.1** `MersenneTwister`, `setSeed(int[6])` from the
  `fixedSeed0..5` longs (each must fit in an int), `nextDouble()` with 52 bits.
- All StoEx distributions use inversion with **exactly one uniform per call**:
  - `Exp` is closed form with `Math.log`.
  - `Norm`, `Lognorm*`, `Gamma*` and `UniDouble` use a bracket in steps of 1.0, then Brent.
  - `Pois` and `UniInt` use integer bisection.
- `Binom` is not registered in 5.2.2.
- PMF/PDF literals and branches: see RND-4 and RND-5.
- `Math.log/exp` are the HotSpot Intel LIBM stubs, **not** fdlibm; they are ported bit-exactly
  in `jmath`.

**Verified.**
- Everything is bit-identical to the oracle, which runs the real 5.2.2 classes:
  - 100k uniforms (1M in an ignored test);
  - log/exp on 44k inputs (1.1M ignored);
  - 22.7k distribution calls, including errors and the number of uniforms consumed (356k
    ignored);
  - PMF/PDF literals.
- The same holds for dumps from JDK 21 and JDK 17.
- KS tests pass for all samplers.
- `cargo fmt` and `cargo clippy --all-targets --all-features -D warnings` are clean.
- Default tests take about 4 s in debug.

**Findings for others.**
- **B, D: classpath bug.** `desmoj-2.3.3-core-bin.jar` embeds an old `org.apache.commons.math`
  that shadows the Orbit 2.1 jar on the flat classpath. OSGi binds 2.1; see the interim note
  above and RND-9.2.
  - `reference/build/classpath.txt` has desmoj at line 13 and math 2.1 at line 120, so **refsim
    currently samples Norm/Lognorm/Gamma differently from the product**.
  - Checked: `Gamma(0.843…, 0.00667…)`: simoxide-random equals Java with the Orbit jar first. The
    flat classpath reproduces exactly the magnitude of D's 164 "mismatches".
  - After B and D put `org.apache.commons.math_2.1.0…jar` first and regenerate their goldens,
    `STOEX_STRICT_DIST` should pass and can become the default.
- **`Pois(m)` returns Poisson(m) − 1**, which can be −1. This is a Commons Math 2.1 integer
  inversion bug and is reproduced. `UniInt` has a Palladio +1 fix and is correct.
- **`UniDouble`** is a Brent approximation (±1e-6), not `a + u*(b-a)`.
- **PMF draw with `u >= cum[last]`** returns `Double 0.0` (`PmfSampler::sample_index` returns
  `None`).
- **Branch choice** draws no uniform when there are no transitions (`probfn::sample_branch`).
  `simoxide-sim` calls `branch_index(cum, rng.next_uniform())`, which draws first. That only matters
  for an empty list, which is an error anyway.
- **Speed.**
  - Per call: `Exp` 7 ns; `Norm(0,1)` 0.9 µs; `Gamma(2,100)` 8 µs; `Lognorm(5,1)` 16 µs.
  - The linear bracket of the reference dominates the slow cases.
    `dist::BracketSearch::Galloping` (feature `fast-bracket`, or the `sample_*_with` functions)
    gives the identical `(a, b)` for monotone CDFs. It matched on all 450k oracle draws and is
    4–6× faster on large scales.
  - The default stays `Linear`. The coordinator may switch the default.

**Open.**
- `rcpss` differs between CPU vendors. `jmath::log` uses the instruction on x86-64, so it
  matches a JVM on the same host. The recorded Zen 5 table (`rcp-table`) is exact only for
  AMD Zen 5.
- No oracle for `Binom`, which is unreachable in 5.2.2.

## 2026-09-29 — agent G: `crates/simoxide-testkit` (differential testing)

**Scope.** `crates/simoxide-testkit` (library + `simoxide-fuzz` binary). Overview and API table: `docs/testkit.md`.
- `javafmt`: Java 21 `Double.toString` (JDK 19+ shortest decimal). Fast path: ryu digits plus
  Java's layout. The one rule where Java differs, the 2-digit case when the shortest decimal has
  one digit (subnormals, e.g. `4.9E-324`, `9.9E-323`), goes to `javafmt::exact`, a literal big-int
  implementation of the javadoc spec. About 50 ns per double.
- `json` / `trace` / `tape` / `measurements`: parsers and byte-identical writers.
  - `TraceWriter` has one typed method per event and needs no allocation per event (~135 ns).
  - `TapeWriter` truncates `spec` like the reference.
  - `measurements::Recorder` sorts series in Java `String` order.
- `tape`: `Tape` (interned origins, samples with their first uniform index) and the
  `TapeReplay` cursor.
  - `next_checked(origin)` / `next_checked_parts(purpose, id)` report the first draw made in a
    different place (index, both origin tags).
  - `first_origin_mismatch` and `first_mismatch` compare two tapes.
- `diff`: first-divergence trace differ with context lines, field diffs, process, spawn info and
  open `begin` stack, and extra/missing classification by look-ahead. Modes: exact, tolerance,
  measurements-only, per-process.
- `meascmp` + `stats`: measurements comparison.
  - Modes: exact, tolerance, statistical. The statistical mode fails a series only if KS rejects
    and the batch-means CIs do not overlap; state and utilisation series use time-weighted means.
  - `stats` provides KS, t quantiles, batch means and percentiles.
- `sim`: the `Simulator` trait (`RunRequest{model_dir, RunConfig, RngMode::{OwnRng,
  TapeReplay(Arc<Tape>)}}` → `RunOutput{trace, tape, measurements}`). Implementations:
  - `RefSim`: `refsim batch` in one warm JVM. It stages models with symlinks and applies a
    per-model timeout that detects a hung model and reruns the rest. On a failure it keeps the
    stack excerpt with the `Caused by` lines.
  - `CmdSim`: any CLI, through placeholders.
  - `FnSim`: a closure, with panics caught.
  - `Unimplemented`.
- `corpus`: `run_corpus(&mut sim, &HarnessOptions::from_env())` runs every entry in replay and
  own-RNG mode and compares exactly. It reads `.gz` files, prints a summary table, and has
  `assert_all_pass()`. Environment: `TESTKIT_FILTER`, `TESTKIT_MODES`, `TESTKIT_DUMP`,
  `TESTKIT_STRICT`.
- `modelgen`: random PCM generator. `GenConfig{name, seed, size 1..=10, Features}` is a pure
  function of its inputs.
  - Writes all 7 XMI files, `run.json` and `FEATURES.txt`. The monitors mirror
    `Monitors.java`.
  - Estimates the expected load and sets arrival rates / think times for a utilisation of
    0.25–0.65.
  - Caps the expected executed actions per run.
- `fuzz` + `simoxide-fuzz`:
  - `validate`: pass rate, crash categories, determinism across 2 JVMs.
  - `fuzz`: reference vs candidate. A divergence is saved as a corpus-style entry in
    `corpus-fuzz/<name>.<mode>/`; greedy minimization over size and features goes to
    `minimized/`.
  - `gen`, `diff`, `corpus`.

**Generator coverage.** Composite components with provided and required delegation and an inner
assembly connector; call DAGs up to 6 units; double assemblies with assembly config parameters;
component parameters.

SEFF actions:
- probabilistic and guarded branches (2- and 3-way, always exhaustive);
- loops (constant, PMF, uniform, parametric);
- collection iterators (NUMBER_OF_ELEMENTS, INNER.VALUE, INNER.BYTESIZE);
- sync and async forks, including forks with external calls;
- acquire/release (capacity constant or IntPMF);
- set variables: RETURN plus a second output `aux` with VALUE and BYTESIZE, and return variable
  usages reading both;
- infrastructure calls, with or without parameters;
- resource calls.

Resources: CPU PS/FCFS (1–4 cores), HDD FCFS/PS (1–2), DELAY (1–2), StoEx processing rates;
1–2 linking resources, sometimes a partial first link to exercise routing, a StoEx latency, the
no-throughput flag.

Usage: open workloads (Exp, constant or uniform inter-arrival) and closed ones (1–4 users,
constant or Exp think time); 1–3 scenarios; usage delay, branch and loop; ELSC parameters;
2 system provided roles.

StoEx:
- distributions: Exp, UniDouble, Gamma(Moments), Lognorm(Moments), Norm (possibly negative
  demand), Max, Min, UniInt, Pois, Round, Trunc, Ceil, Sqrt, Log;
- literals: IntPMF, DoublePMF, DoublePDF;
- parametric dependencies on parameters, component parameters, return values and INNER.

Stop conditions: by measurements, by time, or both.

**Verified.**
- Every line of all 61 `corpus/*/expected` trace, tape and CSV files re-serializes byte for byte.
- Every number in them re-formats identically.
- Each corpus trace's `meas` events equal its CSV, and `finish` equals the tape and CSV counts.
- `javafmt` matches Java 21 on 24,859 golden values, including all subnormals below 5000 ulp and
  powers of 2 and 10 with their neighbours, and on 10^6 random doubles (live `--ignored` test).
- The fast formatter equals the exact one in proptests over random bits, subnormals and short
  decimals.
- Harness, differ and fuzz pipeline are tested with stand-in simulators (echo, perturbed,
  wrong-order replay).
- `RefSim` reproduces `expected/` exactly (`--ignored`).
- All generated models load in simoxide-model with 0 diagnostics (300 seeds, sizes 1–10).
- Reference validation (`simoxide-fuzz validate --n 300 --sizes 1..10`, seeds 1–300):
  - **300/300 run cleanly**;
  - **0 nondeterministic**: two separate JVMs gave byte-identical trace, tape and CSV;
  - run time p50 164 ms, p90 564 ms, max 1.28 s. These times were measured with other agents'
    JVMs running concurrently.
  - Before the fork fix below, 299/300 ran cleanly.
- cargo fmt and clippy `-D warnings` are clean for simoxide-testkit.

**Reference crash categories.** These are avoided by construction; each is cited from
`docs/spec` and the INDEX:
- recursive calls with response-time monitors ("First measurement to the same context");
- a guarded branch without a true guard (ACT-6.2);
- a SetVariable in a forked behaviour or infrastructure SEFF, which has no result frame
  (ACT-5.8);
- a missing link between communicating containers (ACT-11.2);
- `Binom` (unknown to FunctionLib);
- `Min`/`Max` with mixed Integer/Double arguments.

**New reference bug found by the validation.** It is avoided in the generator by giving every sync
fork child after the first an initial constant CPU demand:
- Two sync fork children finish without waiting (a loop count of 0 and a skipped negative `Norm`
  demand), so the parent gets two Resume notes (SIM-4.4a).
- The second note wakes the parent early from its next resource wait.
- When the resource later calls `activate()`, the run aborts with `IllegalStateException: Tried to
  schedule thread which was not suspended [ClosedUser_1]`.
- Case: `fz224_s10` (seed 224, size 10).
- `h26` survives this only because of its timing.

**Fuzzing SimOxide.** Setup: `simoxide` via `CmdSim`, 60 models (seeds 1000–1059, sizes 1–8).
- **First run** (simoxide build of 02:58): replay was exact on 47/60 models. The 13 divergences fell
  into two simoxide-sim issues:
  1. **FCFS resources with `numberOfReplicas > 1`.** The reference records `State of Active
     Resource Tuple` at `replicaID=0` only. SimOxide used the replica that serves the job.
  2. **Stop at `max_sim_time`.** When a resource event falls on the stop instant, the reference
     emits its state `meas` before `stop`. SimOxide stopped first.
- **Second run** (simoxide of 03:08; agent H had fixed both issues in the meantime):
  - **replay 60/60 exact**;
  - **own RNG 20/60 exact**. All 40 own-RNG divergences start at a `Gamma`, `GammaMoments`,
    `Lognorm`, `LognormMoments` or `Norm` sample: the known classpath issue, see agent E.
- Saved regression entries: `corpus-fuzz/pz1007_s1` and its minimized form
  `corpus-fuzz/pz1007_s1_m1_25` (issue 2). Replay now passes on both.
- Re-run:
  `simoxide-fuzz corpus --corpus corpus-fuzz --sim "cmd:$PWD/target/release/simoxide run --model {dir}
  --run-json {run_json} --name {name} --trace {trace} --tape {tape} --measurements {measurements}
  {replay:--replay-tape}"`

**For the core-sim author.**
- Harness: `crates/simoxide-sim/tests/corpus.rs` = `FnSim::new("simoxide-sim", |req| ...)` +
  `corpus::run_corpus(..).assert_all_pass()` (see the `corpus` module docs).
- Fuzzing: run `simoxide-fuzz fuzz --n 200 --modes replay,own --sim "cmd:<the simoxide command line
  above>"`. Divergences land in `corpus-fuzz/`, together with a minimized variant.
- simoxide-sim has its own `javafmt.rs` and `trace.rs`. Either use `simoxide_testkit::{javafmt, trace::TraceWriter,
  tape::TapeWriter, measurements::Recorder}` or check them against `tests/formats.rs`: the
  one-digit subnormal case and the non-finite quoting are easy to get wrong.
- Generated models exercise things the corpus does not:
  - FCFS, HDD and DELAY with several replicas;
  - StoEx processing rates and link latency (a `rate:` draw on every demand);
  - partial-link routing;
  - composites with required delegation;
  - infrastructure-call inputs;
  - the `aux` output variables and their BYTESIZE in response payloads.

**Open.** Minimization is at the generator level: it shrinks the parameters, not the model. The
statistical mode is not yet used by any test, because nothing produces long runs yet.

## 2026-09-29: agent I — reference classpath fixed; expected outputs regenerated; models changed: 8

**For H and G: `corpus/*/expected` changed for `h05_open_workload`, `h06_closed_think`,
`h21_stoex_distributions`, `x_pem_screencast_ms`, `x_pem_screencast_ms_enqueue`,
`x_pem_screencast_ms_instant`, `x_pem_simple_optimisation`, `x_pem_simple_optimisation_composites`.**
Only `Norm`/`Lognorm`/`LognormMoments`/`Gamma` sample values moved (first differing tape line, e.g.
`h05` `Gamma(2.0, 0.1)` 0.14595752134240408 → 0.14595756022536419); draw and measurement counts
are unchanged. The other 53 models are byte-identical. Any `DIST_INEXACT`-style allowance for these
models should now be unnecessary: refsim samples with Commons Math 2.1, like `simoxide-random`.

- **Cause** (E's RND-9.2): `desmoj-2.3.3-core-bin.jar` (private old Commons Math) came before
  `org.apache.commons.math_2.1.0` on the flat classpath.
- **Fix:** `reference/tools/classpath.py` (`gen` + `check`) and `tools/classpath.sh` build one
  OSGi-ordered `reference/build/classpath.txt`. A bundle-private copy of an exported class moves
  behind the exporter; `check` fails on any duplicate class whose first copy is not the OSGi
  binding or that is not reviewed. The 7 duplicate sets are reviewed in `reference/PATCHES.md`
  "Classpath", with `-Xlog:class+load` evidence.
- **Shared classpath.** `build.sh`, `refsim`, and the oracles `random`, `sched`, `stoex` and
  `pcm-model` all use it now. `stoex` previously used `work-simucom/standalone/classpath.txt`, which
  had the same bug. `simoxide-model` previously used its own copy.
- **Verified.**
  - With the old classpath, `refsim batch --check` passed (ALL OK) before the change.
  - After `regen-expected.sh`, `verify-determinism.sh` passes (two fresh JVMs identical, 3x
    repeat, and `expected/` identical).
  - Oracle goldens:
    - `stoex`: regenerated. 37 cases changed: distribution samples, Commons Math error messages,
      and the JIT-dependent `ArithmeticException` message "null" vs "/ by zero".
    - `simoxide-model`, `sched`, `random`: rebuilt and rerun, goldens byte-identical.
  - `cargo test -p simoxide-model -p simoxide-sched -p simoxide-random -p simoxide-stoex`: all pass.
  - `crates/simoxide-stoex/tests/golden.rs` is now strict by default:
    - distribution sample mismatches fail;
    - evaluations that panic (skipped) fail;
    - `STOEX_LAX_DIST=1` only reports mismatches;
    - result: 8834 cases, 0 failures, 0 distribution mismatches, 0 skipped.
- **Docs.** `docs/spec/stoex.md` (open item closed) and `docs/spec/random.md` RND-9.2 (one line
  on the fix) are updated.

## 2026-09-29: agent J (performance pass; BUG-1 and BUG-2)

- **Performance.** 1.2–1.8x more events/s over the benchmark suite (mediastore 6.1 → 10.0 M
  events/s, call chain h14 7.2 → 12.8), with all outputs bit-identical (golden hashes of 390
  runs, corpus both modes, `corpus-fuzz`). Details, measurements, profiles, tooling
  (`simoxide-bench`, `simoxide load-bench`, `--profile`) and rejected ideas: `docs/perf.md`.
- **BUG-1 fixed** (`simoxide_sim::javafmt`): Java's two-digit `Double.toString` of subnormals.
- **BUG-2 fixed** (`simoxide_sched::Fcfs`): a process queued twice at an FCFS resource aborts like
  the reference (NPE in `scheduleNextEvent`/`toNow`) at the same event. All 24 saved reference
  aborts of this kind match up to the abort (`reference/reffail-cmp.sh`).
- **API changes.** `Fcfs::process`/`on_wakeup` and `ActiveResource::process`/`on_wakeup` return
  `Result<_, SchedError>`. `simoxide_stoex::Pmf` and `BoxedPdf` have a new `monotone` field.
- Both bug tests in `crates/simoxide-sim/tests/bugs.rs` run un-ignored and pass.

## 2026-09-29: agent M (simoxide-model loading performance, in-memory loading)

- **Faster loading, same results.** mediastore 1257 → 532 µs from disk, 513 µs from memory,
  188 µs with a warm `ParseCache`; all 69 benchmark dirs 23.6 → 8.2 ms. Details and numbers:
  `docs/perf.md`, "Loading".
- **New API (additive).** `load_memory`, `load_memory_entries`, `Loader::add_file`,
  `Loader::set_read_files`, `Loader::load_memory_dir`, `load::MEMORY_DIR`; `ParseCache` +
  `Loader::set_cache` (share parsed files between loads/threads); `build_from_graph`;
  `ClassId::containment_features`.
- **Behaviour change.** `validate`: the order of "action not reachable from start" warnings is
  the step order (it was hash order, i.e. nondeterministic).
- **Verified.** `load-bench snapshot` (graph incl. lines, canonical dump, typed model,
  diagnostics; strict, tolerant, entry files) identical to the pre-change binary for 231 dirs;
  golden test 163 dirs identical to EMF; `tests/memory.rs` (memory = directory loading, cache =
  no cache, changed files not served from the cache); `cargo test --release` of all crates
  (simoxide-sim's `statistical` test did not compile at the time because of agent L's
  in-progress `GenModel` change); `golden.sh`: 258 of 267 hashes equal to the 04:37 binary; the
  9 others are agent L's new models h28–h30 (triggers, middleware), whose simoxide-model output is
  unchanged.
- **Tools.** `cargo run --release -p simoxide-model --example load-bench -- bench|snapshot`
  (`--features profile` for `--profile out.folded`).
- **Open.** simoxide-sim's `SimSpec::load_model` could offer a `ParseCache` and in-memory models to
  batch users.

## 2026-09-29: agent L (functional gaps: triggers, nested containers, middleware, aborts, livelock)

**Implemented in simoxide-sim (all checked against refsim, both RNG modes, byte-exact).**
- **`triggersSelfAdaptations = true`** (EMF default; MEAS-7.2/7.3 rewritten). The reference's
  only visible effects: the `ReconfigurationProcess` spawn (one `spawn` line, later pids + 1) at the
  first runtime-measurement (PRM) write after t = 0, and, with a `Reconfiguration Time` monitor,
  a `(t, 0.0)` tuple per run of that process (empty reconfigurations succeed). SimOxide models the
  PRM writers (FeedThrough, Fixed/VariableSizeAggregation incl. buffer/eviction, TimeDriven(Aggr.)
  windows, lazily registered passive calculators), the `Reconfigurator` scheduling (`Ev::Reconf`
  at the write's FIFO position, `lastReconfigurationTime`, synchronous runs after the stop) and the
  `Number of Resource Containers` initial tuple. Measurements and tape never change otherwise
  (all 61 corpus models re-run with every spec `true`).
- **Probes by id** (MEAS-3.1): response-time series are matched by id strings, not arena ids (a
  monitor may point to an equal-id element of another loaded model; fixed a real divergence in
  the SimpleHeuristics example with its own monitors).
- **Nested resource containers**: SimuLizar 5.2.2 simulates top-level containers only (ACT-3.3,
  REF-8); SimOxide already did the same, now with a matching NPE message at the first demand.
- **`simulate_linking_resources = true`**: payload = `stream.BYTESIZE` of request/result frame for
  every assembly-connector call (ACT-11.3, REF-10); runs when the model passes the stream.
- **Reference aborts reproduced**: recursion re-entering a monitored call/assembly operation
  (MEAS-1.5, REF-9), triggering reconfiguration-time spec after the stop (REF-11), aggregated
  passive state (REF-13, error at the end of the event).
- **REF-7 livelock guard** (`--max-events-per-instant`, default 2e7, deviation documented).
- `Execution Result Type` tuples (MEAS-7.4): only with an explicit monitor; refsim's recorder
  cannot record textual values (ClassCastException), so unverifiable: SimOxide warns.
- CLI prints compile warnings.

**Tests / corpus.** New corpus models (refsim gen): `h28_triggers_default`, `h29_triggers_late`,
`h30_middleware_stream`, `h31_nested_container`, `h32_recursion` (66 models). New
`crates/simoxide-sim/tests/models/`: `t_*` (9 trigger variants, 7 with reconfiguration monitors) and
`u_*` (mediastore, linking, heuristics, espresso imported with their own monitor repositories).
`corpus-fuzz/l_ref_*` (5 reference aborts) checked by `bugs.rs::reference_aborts_are_reproduced`
(error + trace prefix). `api.rs::zero_time_livelock_is_stopped`. 168 tests pass; fmt/clippy clean.
**refsim**: `Monitors.triggers` predicate, `HandMade` per-model `triggers`/`skipExternalCalls`,
`PcmBuilder.nestedContainer`, `refsim import --triggers`.

**Generator** (`simoxide_testkit::modelgen`): `triggers` (all/subset + reconfiguration-time and container
monitors), `prm_aggregation`, `nested_container` (+ `nested_allocation` aborts),
`middleware_stream` (+ `_missing` aborts). Final campaigns (both modes): 650 models (400 with
the new features boosted, 250 defaults), 608 run by the reference, 1216/1216 checks exact, all 42
reference aborts also abort in SimOxide; earlier campaigns in `docs/testing.md`.

**Guardrails.** Golden hashes of the 222 pre-existing runs unchanged; corpus 132/132 and
corpus-fuzz pass; mediastore ~10.0 M events/s (base 10.1, within noise).

**Notes for others.** My `cargo fmt --all` reformatted agent M's `crates/simoxide-model/tests/memory.rs`
(whitespace only, 05:28). `Sink` now owns the event list (PRM writes insert notes mid-emission).
Open: `Execution Result Type` needs a textual-value refsim recorder; exact reconfiguration
engines' set is refsim's (QVTo succeeds on empty rules).

## 2026-09-29: agent N (final review)

**Done so far.**
- **Robustness fixes** (BUG-3 in `docs/BUGS.md`; limits in `docs/deviations.md`):
  - StoEx parser limits: `MAX_NESTING` 200, `MAX_DEPTH` 1000;
  - `simoxide_sim::Limits` in `SimConfig::limits`: `max_stack_depth`, `max_processes`,
    `max_events`, `max_steps`, `deadline`, `cancel`;
  - `SimError::kind` (`Model` / `Limit` / `Cancelled`);
  - max proxy depth and max synchronous nesting;
  - compile errors for self-containing composites, more than 10^6 assembly-context paths, and
    replicas above 10^5;
  - the linking-resource route table is O(Σ|connected|²);
  - an `assert!` / `expect` in `component_instance` became errors.
- **Embedding API.**
  - `RunSpec::load_model_memory`;
  - `simulate_memory(files, spec, limits) -> Result<RunResult, RunError>`;
  - `Measurements::summaries()` (`SeriesSummary`);
  - example `crates/simoxide-sim/examples/embed.rs`;
  - CLI flags `--max-steps`, `--max-events`, `--timeout`, `--max-stack-depth`,
    `--max-processes`.
- **Tests.** `crates/simoxide-sim/tests/robustness.rs` (6 tests). 300 000 extra mutants
  (`ROBUST_N=100000`, seeds 1 to 3) caused no panic.
- **Guardrails.**
  - `golden.sh`: all 312 hashes equal to a binary built from the pre-change sources;
  - `cargo test --release --workspace`: 174 passed, 25 ignored;
  - fmt and clippy clean;
  - mediastore speed unchanged (10.0 vs 9.9 M events/s); h11 fork about 2 to 3 % slower
    (per-step limit counter).
- **Fuzz** (simoxide with these changes; seeds 1 100 000 to 1 101 199, defaults, both modes):
  - 1200 models, 1144 run by the reference, 2288/2288 checks exact, 0 divergent;
  - the 56 reference aborts are all known categories (REF-2, REF-8, REF-10, BUG-2 NPE,
    missing route), and SimOxide aborts on all 56 too.
  - All 15 optional features at 1 (seeds 1 200 000 to 1 200 699): 700 models, 434 run by the
    reference (3 of them after a rerun without the 60 s timeout, with traces up to 1.2 GB),
    868/868 checks exact, 0 divergent. SimOxide aborts on all 266 reference aborts.
- **Harness.** `ulimit -v` on refsim makes SimuLizar's thread creation fail, and SimuLizar then
  ends the run early while reporting success. That produced one false divergence (discarded).
  `simoxide_testkit::sim::RefSim` now distrusts a batch whose stderr shows thread or memory failures.

- **Heavy tests.** All 25 ignored tests pass: statistical 10, analytic schedulers 5, PS
  virtual-time performance 1, simoxide-random big goldens 4 (generated with
  `SCALE=big reference/oracles/random/gen-golden.sh`, `SIMOXIDE_RANDOM_GOLDEN`), Java 21 formats 1,
  refsim reproduces the corpus and the fuzz pipeline 2, generator on the reference 1.
- **Reference speed** (warm, 20 000 measurements, identical output): mediastore 111.7 s vs
  0.107 s; h11 25.0 s vs 0.017 s; h21 20.4 s vs 0.140 s (`docs/perf.md`).
- **New reference anomaly REF-14.** A deterministic recursion of depth 300 deadlocks SimuLizar:
  the process thread dies silently. SimOxide does not reproduce this.
- **README.md** written.
- **Final guardrails.**
  - `cargo test --release --workspace`: 174 passed, 0 failed;
  - golden hashes unchanged;
  - `simoxide-fuzz corpus`: 132/132 on the corpus and 6/6 on corpus-fuzz;
  - fmt and clippy clean.
- **Open.** See "Open items" in README.md: an FFI/JNI crate, a bound on stored measurements,
  and no deadline during loading and compilation.

## Agent O: performance comparison with the other Palladio simulators (2026-09-29)

- **Deliverables.**
  - `bench/compare/`: one entry script, `run-all.sh`, with the phases build, check, cold, warm, par,
    sus1, susN, longrun and summary. Every phase resumes.
  - Drivers: `simoxide-driver` (Rust), `java/{refsim,simulizar,slingshot}`, `simucom/` (the repaired
    SimuCom 5.2.2 headless workflow) and `eventsim.sh`; tools in `tools/`.
  - `results/`: CSVs and Markdown tables; raw logs in `results/raw/`.
  - The write-up is `docs/perf-comparison.md`.
- **Simulators.**
  - SimOxide; refsim with the trace off; unpatched SimuLizar 5.2.2 (EDP2 in memory, OSGi-correct
    classpath, plus an OSGi cold run); SimuLizar with the virtual-thread patch.
  - Slingshot nightly 2026-09-01 (flat classpath, OSGi, isolated class loaders, processes).
  - SimuCom 5.2.2: now runs, after repairing the codegen in a separate bundle.
  - EventSim 5.1: archived, included as a historical data point.
- **Main results.**
  - Warm per run: 250× to 2 100× faster than stock SimuLizar, 180× to 880× than Slingshot,
    190× to 2 060× than SimuCom.
  - Cold one-shot: 4 to 100 ms against 2.7 to 17 s.
  - Sustained on 22 cores, MediaStore: 4 937 runs/s against at most 17 (refsim, 22 processes).
  - Stock SimuLizar leaks threads and heap. With 22 threads in one JVM it collapses after about 3.5
    min (37 % GC); with 22 processes it exceeds the memory limit.
  - Slingshot leaks threads, but its throughput stays flat. EventSim degrades within minutes.
- **Sanity.** The SimuLizar family (SimOxide, refsim, stock, VT, SimuCom) produces identical request
  counts and mean response times. Slingshot is identical on its E2E models and close elsewhere.
- **Open.**
  - Slingshot cannot run `x_pem_fork` or `h11` (synchronous fork bug), and needs an explicit demand on
    Acquire and Release.
  - SimOxide wraps at 2^63 ns (292 simulated years), as the reference does; seen in a 10^10 s run.
  - The sustained durations were shortened (300 s / 120 s instead of 10 min / 3 min) to fit the time
    budget.

## Agent Q: fast mode (2026-09-30)

- **What.** An opt-in fast mode that keeps the model semantics and the statistics but drops the
  work that exists only to reproduce the reference's bits. Exact stays the default and is
  unchanged: golden hashes identical (312 runs), corpus 66/66 in both RNG modes, all previous
  tests pass unchanged.
- **Design.** Compile-time policy `simoxide_sim::compat::Compat` (`Exact`, `Fast`, diagnostic
  `FastRng`; round 3 replaced `FastRng` by the test-only `Literal<P>` and moved the engine
  shortcuts below into both modes, see `docs/perf.md`); `Simulation<'m, C = Exact>` is monomorphized; `SimConfig::mode` +
  `simoxide_sim::run` / `run_batch` / `simulate_memory_mode` dispatch once per run;
  `Simulation::new` stays exact. Distribution functions are hooks of
  `simoxide_random::UniformSource` (defaults = the reference's algorithms). Cargo feature `fast`
  (simoxide-random, simoxide-sim; on by default only in simoxide-cli). CLI `--mode
  exact|fast|fast-rng`, `--ps-algorithm exact|virtual-time`.
- **Fast-mode changes.** xoshiro256++ with ziggurat / Marsaglia-Tsang / PTRS / exact uniform
  samplers (reference parameter checks and errors, reference fallback for degenerate
  parameters); hand-off `Resume` notes merged only where the order of events cannot change;
  `INNER` evaluated without copying the frame contents; no tape. Left out because they changed
  results: unconditional hand-off merging, virtual-time PS, insertion-order frames.
- **Also.** Virtual-time PS now keeps a re-queued job's position (as the exact PS);
  `#[inline(always)]` on a few hot helpers that LLVM stopped inlining once three simulator
  instantiations shared them (exact mode had lost 8-18 %).
- **Tests.** `simoxide-random/tests/fast.rs` (chi-square over 10^6 samples per sampler, moments,
  generator checks, errors on all 154 oracle parameter sets), `simoxide-stoex/tests/fast.rs`,
  `simoxide-sim/tests/fast_mode.rs` (API; fast engine byte-identical to the exact engine with
  the same random numbers on 96 model directories and generated models; deterministic models
  byte-identical to exact; quick equivalence), `statistical.rs` in both modes,
  `simoxide_testkit::equiv` + `simoxide-fuzz equiv`. Campaigns: 1 396 models, about 105 000
  tests, 0 Holm failures, p-values like an exact-vs-exact control (`docs/testing.md` §8).
- **Performance.** Single thread 0.98x (espresso) to 3.9x (h21), generated models 2.05x (geomean
  2.07x); all cores h21 3.5x, generated s10 #6 1.95x, mediastore 1.0x (`docs/perf.md`
  "Fast mode"). Remaining cost is the interpreter, StoEx evaluation and the schedulers.
- **Guardrails.** `cargo test --release --workspace`: 203 passed; fmt and clippy `-D warnings`
  clean for the workspace, `simoxide-sim` with and without `fast`, `simoxide-random` with and
  without `fast`, `simoxide-cli --no-default-features` and `--features profile,mimalloc`.

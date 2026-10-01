# Random numbers and sampling (SimuLizar 5.2.2)

Rules `RND-*`. When draws happen: SIM-8 in [Simulation core](./simulation.md),
[Workloads](./workloads.md) and [Actions](./actions.md). StoEx semantics:
[StoEx](./stoex.md). Implementation: `crates/simoxide-random`. Oracle:
`reference/oracles/random`.

## Sources

| Abbrev. | Location |
|---|---|
| `[SCC]` | `Palladio-Analyzer-SimuCom/bundles/de.uka.ipd.sdq.simucomframework.core/src/de/uka/ipd/sdq/simucomframework/core/` |
| `[SCV]` | `Palladio-Analyzer-SimuCom/bundles/de.uka.ipd.sdq.simucomframework.variables/src/de/uka/ipd/sdq/simucomframework/variables/` |
| `[SIMC]` | `Palladio-Analyzer-SimuCom/bundles/de.uka.ipd.sdq.simulation.core/src/de/uka/ipd/sdq/simulation/core/` |
| `[PF]` | `Palladio-Core-Commons/bundles/de.uka.ipd.sdq.probfunction.math/src/de/uka/ipd/sdq/probfunction/math/` |
| `[PF522]` | the same bundle, decompiled from `de.uka.ipd.sdq.probfunction.math_5.2.2.jar` (**differs from master**, see RND-9) |
| `[SL]` | `Palladio-Analyzer-SimuLizar/bundles/org.palladiosimulator.simulizar/src/org/palladiosimulator/simulizar/` |
| `[CM2]` | Apache Commons Math **2.1** sources (Maven Central `commons-math-2.1-sources.jar`); the product ships Orbit `org.apache.commons.math_2.1.0.v201105210652.jar`, whose decompiled bytecode has identical logic |
| `[HS]` | OpenJDK jdk21u `src/hotspot/cpu/x86/stubGenerator_x86_64_{log,exp}.cpp` (Intel LIBM) |

Line numbers refer to master where master equals 5.2.2.

## RND-1 The stream

- **RND-1.1** One stream per run. `SimuComConfig.getRandomGenerator()` lazily creates a
  `SimuComDefaultRandomNumberGenerator(randomSeed)` (`[SCC]SimuComConfig.java:268-272`).
  `SimuComModel` installs it into the static `ProbabilityFunctionFactoryImpl` singleton and its
  `PDFFactory` (`[SCC]model/SimuComModel.java:109-115`). Therefore:
  - all StoEx distribution functions (RND-3),
  - PMF/PDF literals (RND-4),
  - probabilistic branch choice (RND-5)

  draw from this single stream.
- **RND-1.2** Generator: `MT19937RandomGenerator` = Commons Math 2.1 `MersenneTwister`
  (`[SCC]SimuComDefaultRandomNumberGenerator.java:51`, commented alternative: MRG32k3a, unused).
  - A producer thread pre-fills a `LinkedBlockingQueue` (capacity 1000) with `nextDouble()`
    values.
  - `random()` takes from the queue. With a single consumer, the sequence is exactly the MT
    sequence.
- **RND-1.3** Seeding (`[SCC]SimuComDefaultRandomNumberGenerator.java:88-116`,
  `[SIMC]AbstractSimulationConfig.java:148-154`):
  - `useFixedSeed=true`: six longs `fixedSeed0..5` → `ApacheMathRandomGenerator.setSeed(long[])`.
    - Each seed must lie in `[Integer.MIN_VALUE, Integer.MAX_VALUE]`, otherwise
      `IllegalArgumentException`.
    - The values are cast to `int[6]` → `MersenneTwister.setSeed(int[])`, i.e. MT19937
      `init_by_array` (`init_genrand(19650218)`, then the two mixing loops).
    - A seed array whose length is not 6 → `RuntimeException`.
  - `useFixedSeed=false` (default): six `new Random().nextInt()` → nondeterministic (ND-5 in
    [Simulation core](./simulation.md)).
- **RND-1.4** `nextDouble()` (`[CM2]random/BitsStreamGenerator.java:83-87`):
  `((long) next(26) << 26 | next(26)) * 0x1.0p-52`.
  - 52 random bits, two MT outputs per double.
  - Range `[0, 1)`; 0.0 is possible.
- **RND-1.5** No other source of randomness is in scope. Not relevant here:
  - `Math.random()` in `SimulatedLinkingResource` (failures only);
  - `new Random()` in the exact OS schedulers;
  - `DefaultRandomGenerator` (`java.util.Random`), which is only the unused default field of the
    discrete PDFs.

## RND-2 `java.lang.Math` on HotSpot x86-64

- **RND-2.1** Commons Math 2.1 uses `java.lang.Math.log/exp` (no `FastMath` in 2.1).
  - HotSpot (JDK 9+) implements both as the Intel LIBM stubs `[HS]`, in the interpreter, C1 and
    C2 alike.
  - They are neither fdlibm (`StrictMath`) nor correctly rounded.
  - Measured on 1.1M inputs, JDK 21 and 17 alike:
    - `Math.log` ≠ `StrictMath.log` in 2.5 % of cases;
    - `Math.log` ≠ glibc `log` in 0.04 %;
    - for `exp`, the rates are 5.7 % and 0.15 %.
- **RND-2.2** With the feature `hotspot-math` (default in `simoxide-cli` and `simoxide-testkit`,
  off in the libraries), `simoxide_random::jmath::{log, exp}` call
  the crate `simoxide-hotspot-math`, which ports the stubs instruction by instruction. SSE2 lane
  operations are written as scalar operations in the same order. They are bit-identical on all
  1.1M probes. The stubs are GPL-2.0-only, so that crate is too, and binaries built with the
  feature must not be distributed (see `LICENSES`).
  - Without the feature, `jmath::{log, exp}` are `simoxide_random::crmath::{log, exp}`:
    SimOxide's own correctly rounded implementations (table-driven reduction, a rounding test,
    double-double fallback), with HotSpot's special cases.
  - HotSpot's stubs are not correctly rounded on about 0.25 % of `exp` arguments, and, for `log`,
    on about 1e-5 of arguments, all near 1 (`log(1-u)` of `Exp` included). There the two differ
    by one ulp.
- **RND-2.3** `log` uses `rcpps`, an approximate reciprocal whose low bits are CPU-specific. Only
  its value rounded to 7 bits is used, which selects the table entry.
  - The port executes the same instruction on x86-64, so it matches a JVM on the same machine.
  - On other targets, or with the feature `rcp-table`, it uses a table recorded on AMD Zen 5
    (129 breakpoints, all 2^23 inputs).
- **RND-2.4** `Math.sqrt` is exact. `Math.max/min` have Java NaN/±0 semantics
  (`jmath::{max,min}`).

## RND-3 StoEx distribution functions (`[SCV]functions/FunctionLib.java:35-60`)

- **RND-3.1** Registered names: `Norm`, `Exp`, `Pois`, `UniDouble`, `UniInt`, `Lognorm`,
  `LognormMoments`, `Gamma`, `GammaMoments`. `Binom` exists (`BinomFunction`) but is **not
  registered**, so a call fails with `FunctionUnknownException`.
- **RND-3.2** Per evaluation, in this order:
  1. `checkParameters`; failure → `FunctionParametersNotAcceptedException`, **no draw**.
  2. A new distribution object is built via `PDFFactory`; a constructor exception → no draw.
  3. `inverseF(random())`: **exactly one uniform**, inversion.
  4. Exceptions from the inversion propagate after the draw.

  Every exception aborts the run.

| StoEx | Check (Java NaN semantics) | Object (`[PF522]`, `[CM2]`) | Inversion |
|---|---|---|---|
| `Exp(rate)` | `!(rate <= 0)` | `ExponentialDistributionImpl(mean = 1.0/rate)`, `mean <= 0` throws | `u == 1 ? +inf : -mean * Math.log(1.0 - u)` |
| `Norm(m, s)` | arity only | `NormalDistributionImpl(m, s)`, `s <= 0` throws | `u == 0 → -inf`, `u == 1 → +inf`, else RND-6 (acc. 1e-9) |
| `Lognorm(mu, s)` | `!(s <= 0)` | Palladio `LognormalDistributionImpl(mu, s)` extends `NormalDistributionImpl` | `u == 0 → 0`, `u == 1 → +inf`, else RND-6 with `F(x) = x == 0 ? 0 : Φ((Math.log(x) - mu)/s)`, and initial value and bounds `Math.exp(`normal ones`)` (acc. 1e-9) |
| `LognormMoments(mean, sd)` | `!(mean < 0) && !(sd < 0)` | `…FromMomentsImpl(mean, var = sd*sd)`: `mean <= 0` or `var <= 0` throws; `σ² = Math.log(var/(mean*mean) + 1.0)`, `mu = Math.log(mean) - σ²/2.0`, `σ = Math.sqrt(σ²)` | as `Lognorm` |
| `Gamma(α, θ)` | `!(θ <= 0) && !(α <= 0)` | `GammaDistributionImpl(α, β = θ)`: `α <= 0`, then `β <= 0` throws | `u == 0 → 0`, `u == 1 → +inf`, else RND-6 with `F = regularizedGammaP(α, x/β, 1e-14, MAX_INT)` (acc. 1e-9) |
| `GammaMoments(mean, cv)` | `!(mean < 0) && !(cv < 0)` | `var = cv*cv*mean*mean`, `θ = var/mean`, `α = mean/(var/mean)` | as `Gamma` |
| `Pois(m)` | `!(m < 0)` | `PoissonDistributionImpl(m)`, `m <= 0` throws (so `Pois(0)` fails) | RND-7 with `F(k) = regularizedGammaQ(k + 1.0, m, 1e-12, 10^7)` |
| `UniDouble(a, b)` | `!(a > b)` | Palladio `UniformDistributionImpl(a, b)`, `b < a` throws | RND-6 with `F = (x-a)/(b-a)`, initial `(a+b)/2.0`, bounds `[a, b]`, **acc. 1e-6**; `a == b` throws ([REF-3](../correctness/reference-bugs.md)) |
| `UniInt(a, b)` | both `Integer` | Palladio `UniformIntDistributionImpl`, `b < a` throws; `n = b - a + 1` (int wrap) | RND-7 with `F(k) = (double)(k-a+1)/(double)n`, then **`v = x0 + 1; if (v > b) v = b`** (Palladio fix, `[PF522]apache/impl/UniformIntDistribution.inverseF`) |

- **RND-3.3 `Pois` is shifted by −1** (reference bug [REF-4](../correctness/reference-bugs.md),
  reproduced).
  - Commons Math 2.1 integer inversion returns the largest `x` with `F(x) <= u` (RND-7).
  - So `Pois(m)` = Poisson(m) − 1, taking the value `-1` with probability `exp(-m)`.
  - Palladio corrected this for `UniInt` only (RND-3.2, last row). `Pois` is used as a loop count
    or demand exactly as it is.
- **RND-3.4 `UniDouble` is not `a + u*(b-a)`.**
  - It is Brent's approximation of the root with absolute accuracy 1e-6 and relative accuracy
    1e-14.
  - The result can differ from the exact value by up to about 1e-6.
- **RND-3.5 Cost of numerical inversion.**
  - `Norm`, `Lognorm*`, `Gamma*` and `UniDouble` bracket the root from the initial value in steps
    of **1.0** (RND-6.1).
  - The number of CDF evaluations therefore grows linearly with the distance from the initial
    value to the quantile, in absolute units. In simoxide-random, `Gamma(2, 100)` takes about
    4 µs per sample and `Lognorm(5, 1)` about 11 µs, against about 0.5 µs for `Norm(0, 1)`
    (`cargo run --release -p simoxide-random --example bench`).
  - Distributions with a scale far below 1 (every distribution in the corpus, e.g.
    `Norm(0.02, 0.005)`, `Gamma(2, 0.01)`, `Lognorm(-4.5, 0.4)`) are bracketed by the first
    step of 1.0 (each endpoint is either clamped at its bound or already beyond the root), so the
    bracket search does exactly one step.
  - In extreme tails the cost explodes: `Lognorm(0, 3)` at u = 1−1e−10 needs ~1e8 evaluations.
  - Beyond 2^53, `b + 1.0 == b`, and the reference loop runs until `numIterations` reaches
    `Integer.MAX_VALUE` (a hang).

## RND-4 Probability-function literals

- **RND-4.1 Adjustment** (`[SCV]cache/ProbFunctionCache.java:60-90,137-160`). This is done once
  per distinct StoEx string, in model order, and mutates the model.
  - `sum = Σ p_i`, summed left to right.
  - If `Math.abs(sum - 1) > 10e-10` (= 1e-9): `delta = (1 - sum) / count(p_i > 0)`, and
    `p_i += delta` for every `p_i > 0`.
  - `simoxide_random::probfn::adjust_probabilities`.
- **RND-4.2 PMF**: `transformToPMF` → `ProbabilityMassFunctionImpl.setSamples`
  (`[PF]impl/ProbabilityMassFunctionImpl.java:245-251`).
  - With more than one sample, the samples are sorted by value with `Comparable.compareTo`. This
    is a stable sort:
    - `Integer`/`Double`: `Double.compare` order (−0.0 < 0.0, NaN last);
    - `String`: UTF-16 code units;
    - `Boolean`: false < true.
  - `checkConstrains`: `|Σp - 1| < 1e-5` (`MathTools.equalsDouble`), each `p ∈ [0, 1]`, value
    not null. A failure → `RuntimeException` when the expression is prepared.
- **RND-4.3 PMF draw** (`[PF]impl/ProbabilityMassFunctionImpl.java:290-300`).
  - `cum` = running sums of the sorted probabilities (`prob = 0; prob += p`).
  - One uniform `u`; the result is the value of the **first** `j` with `u < cum[j]`.
  - If there is none (`u >= cum[last]`, possible when adjusted sums are < 1), the result is
    **`Double 0.0`**, whatever the PMF's value type.
  - `probfn::PmfSampler::sample_index` returns `None` in that case. Binary search is used only
    when `cum` is monotone, so it gives the same index.
- **RND-4.4 Boxed PDF** (`[PF]impl/BoxedPDFImpl.java:93-132`, `util/MathTools.java:191-222`,
  `util/Line.java`).
  - Construction:
    - duplicate values (`HashSet<Double>`) → `DoubleSampleException`;
    - stable sort by value;
    - `cum` as in RND-4.3;
    - lines: `(0,0)–(v0,cum0)` under key `cum0`, then for `i ≥ 1` with `cum[i-1] != cum[i]`,
      `(v[i-1],cum[i-1])–(v[i],cum[i])` under key `cum[i]`, in a `HashMap<Double, Line>`;
    - `Line` throws if `x2 - x1 == 0`, which includes a first value of 0;
    - `checkConstrains`: sum as RND-4.2, each `v ≥ 0`, `p ∈ [0,1]`, values non-decreasing.
  - Draw: one uniform; the first `j` with `u < cum[j]`; `x = (u - b)/a` with `a = (y2-y1)/(x2-x1)`
    and `b = y1 - a*x1`.
  - No `j` → `RuntimeException`.
  - Implementation: `probfn::BoxedPdfSampler`.

## RND-5 Probabilistic branches (`[SL]utils/TransitionDeterminer.java:70-80,229-247`)

- Running sums of the branch probabilities in model order (`currentSum += p`).
- One uniform `u` (none if there are no transitions).
- Result: the first `i` with `lastSum * u < sum[i]`. No match → index −1 →
  `IndexOutOfBoundsException`.
- Probabilities are **not** normalised or adjusted; scaling by `lastSum` covers sums ≠ 1.
- Guarded branches draw nothing.
- Implementation: `probfn::{summed_probabilities, branch_index, sample_branch}`.

## RND-6 Continuous inversion (`[CM2]distribution/AbstractContinuousDistribution.java`)

- **RND-6.1** `f(x) = F(x) - p`; a NaN value or a `MathException` → `FunctionEvaluationException`.
  Bracketing (`[CM2]analysis/UnivariateRealSolverUtils.bracket`):
  - `a = b = initial`;
  - repeat `a = Math.max(a - 1.0, lower)`, `b = Math.min(b + 1.0, upper)`, evaluate `f(a)`,
    `f(b)`;
  - while `f(a)*f(b) > 0`, `n < MAX_INT` and `(a > lower || b < upper)`;
  - still `f(a)*f(b) > 0` → `ConvergenceException`. It is caught: return `lower` if
    `|f(lower)| < acc`, `upper` if `|f(upper)| < acc`, else `MathException`.

  Bounds per distribution:

  | Distribution | `lower` | `upper` | `initial` |
  |---|---|---|---|
  | Normal | `p < .5 ? -MAX : mean` | `p < .5 ? mean : MAX` | `mean ∓ sd` (`mean` at .5) |
  | Gamma | `Double.MIN_VALUE` | `p < .5 ? αβ : MAX` | `p < .5 ? αβ*.5 : αβ` |

- **RND-6.2** Brent (`[CM2]analysis/solvers/BrentSolver.solve(f, min, max)`):
  - `min >= max` throws;
  - defaults: max 100 iterations, relative accuracy 1e-14, function-value accuracy 1e-15,
    absolute accuracy = the distribution's;
  - ported step by step in `special::brent_solve`.
  - `BrentSolver.solve(f, a, b)` evaluates `f(a)` and `f(b)` again right after the bracket search
    computed them, and the linear bracket search re-evaluates an endpoint that stays clamped at
    its bound. `f` is pure, so simoxide-random reuses the known values (`special::bracket_values`,
    `brent_solve_values`; endpoints compared by bit pattern). The results are bit-identical.
- **RND-6.3** Normal CDF: `0.5 * (1.0 + erf((x - mean) / (sd * Math.sqrt(2.0))))`. A
  `MaxIterationsExceededException` from `erf` yields 0 below `mean - 20sd`, 1 above
  `mean + 20sd`, and is rethrown otherwise.
  - `Erf.erf(x) = ±regularizedGammaP(0.5, x*x, 1e-15, 10000)`.
  - `regularizedGammaP/Q` (series for `x < a+1`, else the continued fraction with 2.1's scaling
    logic), `logGamma` (Lanczos, `g = 607/128`, `HALF_LOG_2_PI = 0.5*Math.log(2π)`, bits
    `0x3fed67f1c864beb4`) are ported in `special`.
  - `regularizedGammaP/Q` compute `logGamma(a)` on every CDF evaluation, while `a` is fixed
    during one inversion (0.5 for `erf`, so Normal and Lognormal; `α` for Gamma).
    simoxide-random keeps a one-entry per-thread memo of this pure function
    (`special::log_gamma_memo`), which returns bit-identical values.
- **RND-6.4 `BracketSearch::Galloping`** (not the reference loop). It uses the same endpoint
  sequence, still stepped by 1.0 in floating point, but finds the stopping step by exponential
  plus binary search, with O(log k) CDF evaluations.
  - It gives the same `(a, b)` as the reference loop whenever the computed CDF is monotone
    along all skipped endpoints. That is not proven for the floating-point `erf` and
    `regularizedGammaP` (series/continued-fraction switch, rounding near 0 and 1), so the
    galloping search is **not proven exact**.
  - It is identical on every oracle case (450k draws).
  - `BracketSearch::Linear` (the reference loop) is the default. `Galloping` is the default only
    with the opt-in cargo feature `fast-bracket`. The `dist::sample_*_with` functions take the
    search as a parameter.
  - Speed-up: about 4× for `Gamma(2,100)` and 7× for `Lognorm(5,1)`; none for small scales
    (RND-3.5), which the linear search already brackets in one step. See
    [Engine performance](../performance/engine.md).

## RND-7 Discrete inversion (`[CM2]distribution/AbstractIntegerDistribution.java`)

- `x0 = lower`, `x1 = upper`: `[0, MAX_INT]` for Poisson, `[a, b]` for UniInt.
- Bisection: `xm = x0 + (x1 - x0)/2` (int arithmetic); `F(xm) > p` → `x1 = (xm == x1) ? x1-1 : xm`,
  else `x0 = (xm == x0) ? x0+1 : xm`.
- Then `while (F(x0) > p) x0--`. Result: `x0`, the **largest `x` with `F(x) <= p`**.
- A NaN from `F` → `FunctionEvaluationException`.
- Poisson: `F(x<0) = 0`, `F(MAX_INT) = 1`.
- The Poisson bisection over `[0, 2^31 - 1]` evaluates `regularizedGammaQ(x + 1, m)` about 31
  times per sample, mostly at the same points (the top of the bisection tree, where `F` is
  exactly 1). `F` is a pure function of `(m, x)`, so simoxide-random memoizes its values per
  thread for the 4 most recent means (`dist::PoissonCdfMemo`). The bisection, its comparisons
  and its evaluation order are unchanged; errors and NaN are not stored. The memoized inversion
  is tested against the unmemoized one (`poisson_memo_is_exact`) and gives bit-identical
  results.

## RND-8 Verification (`crates/simoxide-random/tests`)

- **Golden** (`reference/oracles/random`, which calls the real 5.2.2 classes):
  - 100k uniforms (plus 1M in an ignored test);
  - `Math.log/exp` on 44k inputs (1.1M ignored);
  - 22.7k distribution calls: 77 parameter sets × (300 seeded draws + up to 20 fixed u), including
    error cases and the number of uniforms consumed (356k ignored);
  - 20 PMF/PDF literals × 500 draws (× 20k ignored).
  - All are **bit-identical**, with both bracket searches, on JDK 21 and JDK 17 dumps.
- **KS tests** (alpha 0.001) for every sampler; `Pois` is tested against Poisson − 1.

## RND-9 Pitfalls and differences

- **RND-9.1 Master ≠ 5.2.2.** Master's `probfunction.math` uses Commons Math **3** (`FastMath`,
  corrected integer inversion) and has no `UniformIntDistribution.inverseF` override. The 5.2.2
  jar uses Commons Math **2.1**. Only 5.2.2 matters.
- **RND-9.2 Flat classpaths.** `desmoj-2.3.3-core-bin.jar` embeds an older
  `org.apache.commons.math`.
  - In OSGi it is invisible: `probfunction.math` `Require-Bundle`s
    `org.apache.commons.math;bundle-version="2.1.0"`, and `de.desmoj` exports only `desmoj.*`.
  - On a flat classpath it shadows 2.1 if it comes first. `NormalDistributionImpl`,
    `GammaDistributionImpl` and the Brent solver then come from the old copy, and
    `Norm`/`Lognorm`/`Gamma` samples change by 1e-9 to 1e-4 relative. `Exp`, `Pois`,
    `UniDouble` and `UniInt` give the same results with both copies.
  - Outside OSGi, the reference must therefore be run with the Orbit Commons Math 2.1 jar before
    DESMO-J. `reference/tools/classpath.sh` builds this OSGi-ordered classpath for refsim and all
    oracles and checks every duplicate class
    ([Classpath](../reference-simulator/patches.md#classpath)). simoxide-random follows the OSGi
    binding (Commons Math 2.1).

## RND-10 Fast mode (not the reference)

The StoEx distribution functions are called through the sampling hooks of
`simoxide_random::UniformSource` (`sample_norm`, ...), whose default implementations are the
functions of RND-3 to RND-7. SimOxide's opt-in [fast mode](../guide/fast-mode.md) uses a source
with other algorithms for the same distributions (`simoxide_random::fast`, feature `fast`). It
is not bit-exact; see [Deviations](../correctness/deviations.md). Everything above describes the
exact mode.

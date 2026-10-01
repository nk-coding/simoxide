# StoEx: stochastic expressions as SimuLizar 5.2.2 evaluates them

Implemented in `crates/simoxide-stoex`. Verified against the real 5.2.2 classes by the oracle in
`reference/oracles/stoex/`. How the distribution functions and PMF/PDF literals draw their
uniforms is specified in [Random numbers and sampling](./random.md).

## 0. Sources

All paths are relative to the `releases/5.2.2` tags of the Palladio repositories. Decompiling
the product jars in `palladio-5.2.2/plugins` gives the same code.

- **V**: `Palladio-Analyzer-SimuCom/bundles/de.uka.ipd.sdq.simucomframework.variables/src/de/uka/ipd/sdq/simucomframework/variables/`
  - `StackContext.java`, `EvaluationProxy.java`
  - `cache/StoExCache.java`, `cache/StoExCacheEntry.java`, `cache/ProbFunctionCache.java`
  - `stoexvisitor/PCMStoExEvaluationVisitor.java` (PCMStoExEval), `stoexvisitor/PCMProbfunctionEvaluationVisitor.java`
  - `functions/*.java` (`FunctionLib`, one class per function), `converter/NumberConverter.java`
- **A**: `Palladio-Core-Commons/bundles/de.uka.ipd.sdq.stoex.analyser/src/.../visitors/`
  - `ExpressionInferTypeVisitor.java` (EITV)
  - `NonProbabilisticExpressionInferTypeVisitor.java` (NPEITV)
  - `../probfunction/ProbfunctionHelper.java`
- **G**: `Palladio-Core-Commons/bundles/org.palladiosimulator.commons.stoex/src/.../Stoex.xtext` and
  `Palladio-Core-PCM/bundles/org.palladiosimulator.pcm/src/.../stoex/PCMStoex.xtext`
  - parser wrapper: `api/impl/generic/GenericStoExParserImpl.java`
- **M**: `Palladio-Core-Commons/bundles/de.uka.ipd.sdq.probfunction.math/src/de/uka/ipd/sdq/probfunction/math/`
  - `impl/ProbabilityMassFunctionImpl.java` (PMFImpl), `impl/BoxedPDFImpl.java`,
    `impl/ProbabilityFunctionFactoryImpl.java`
  - `util/MathTools.java`, `util/Line.java`
- `de.uka.ipd.sdq.pcm.stochasticexpressions/TypeInference.java` (Palladio-Core-PCM): the type of
  a characterised variable.
- SimuLizar `utils/SimulatedStackHelper.java`: how variable usages are put into stack frames.

**master vs 5.2.2.** No semantic difference in V, A, G or the PCM StoEx API; only imports
changed (`javax.inject` became `jakarta.inject`). In M, master moved the named distributions to
Commons Math 3, which changes sampling numerics (RND-9.1 in [Random numbers and
sampling](./random.md)).

## 1. How the simulator uses StoEx

- **Evaluation entry point.** Every evaluation is `StackContext.evaluateStatic(spec, frame[,
  mode])` or `ctx.evaluate(spec)`.
  - It looks up a `StoExCacheEntry` by the exact spec string (V `StoExCache.getEntry`) and runs
    `PCMStoExEvaluationVisitor` on the cached tree.
  - Each call is a fresh evaluation. Every PMF/PDF literal and every distribution function is
    **sampled on every evaluation**.
  - Nothing keeps a distribution as a value. The simulator does **no PMF/PDF arithmetic**:
    - the evaluator uses `NonProbabilisticExpressionInferTypeVisitor`, which maps every `*_PMF`
      and `*_PDF` type to its scalar type;
    - `PCMProbfunctionEvaluationVisitor` draws a sample from each literal;
    - convolution etc. exists only in the analytical solver (`ExpressionSolveVisitor`), which is
      out of scope.
- **Expected types.** `evaluateStatic(spec, T.class, …)` converts the result:
  - `Integer` accepts only an Integer;
  - `Double` accepts Integer and Double;
  - `Long` accepts Integer;
  - `Boolean` accepts only a Boolean;
  - anything else is an `UnsupportedOperationException`. For example, loop counts must be int
    StoExs, so `"2.0"` fails.
  - API: `Value::convert`, `to_f64`, `to_i32`, `to_bool`; `Program::eval_f64`, `eval_i32`,
    `eval_bool`.
- **Stack frame ids.**
  - The id is the Xtext serialisation of the `CharacterisedVariable`: the reference names
    joined by `.`, then `.` and the characterisation literal (for example `a.INNER.BYTESIZE`).
    Whitespace in the source is not part of the id, so `a . b . VALUE` becomes `a.b.VALUE`. Rust:
    `VarRef::id()`.
  - Variable usages are stored under `serialise(namedReference) + "." + type` (Rust:
    `VarRef::reference_name()`).
    - If any reference name is `INNER`, SimuLizar stores an `EvaluationProxy(spec, copy of the
      caller frame)`. The proxy is evaluated again, with fresh draws, on **every** lookup, in
      `EXCEPTION_ON_NOT_FOUND` mode (PCMStoExEval `caseCharacterisedVariable`).
    - Otherwise the value is evaluated once, when the frame is filled.
    - In Rust, proxies are the environment's job: `Env::lookup` gets the RNG. `SimpleEnv`
      implements them as `Binding::Proxy`.
- **Missing variables** (`VariableMode`):
  - `EXCEPTION_ON_NOT_FOUND` (the default, and what every simulator call uses) raises
    `RuntimeException("Architecture specification incomplete. Stackframe is missing id …")`.
  - `RETURN_NULL` returns `null`.
  - `RETURN_DEFAULT` returns `0` for a static INT variable (`BYTESIZE`, `NUMBER_OF_ELEMENTS`)
    and raises the exception otherwise. The ANY variables are VALUE, TYPE and STRUCTURE.
- **RNG.** PMF/PDF literals and functions share one stream: the factory's `IRandomGenerator`,
  set by `SimuComModel` (`setRandomGenerator(config.getRandomGenerator())`).

## 2. Syntax (G)

`GenericStoExParserImpl.parse` rejects blank input and every input with a syntax error. Any error,
including a literal value-conversion failure, becomes a `ParseException`. The Rust parser
(`parser.rs`, a hand-written recursive descent parser) reproduces the **accepted language**. Its
error messages are its own, and they carry line and column.

```text
expression := ifelse EOF
ifelse     := boolAnd ('?' boolAnd ':' boolAnd)?        a ? b : c ? d : e is an error
boolAnd    := boolOr ('AND' boolOr)*                     AND binds WEAKER than OR/XOR
boolOr     := compare (('OR'|'XOR') compare)*
compare    := sum (cmpop sum)?                           1 < 2 < 3 is an error
sum        := prod (('+'|'-') prod)*
prod       := pow (('*'|'/'|'%') pow)*
pow        := unary ('^' unary)?                         2^3^2 is an error; -2^2 = (-2)^2
unary      := 'NOT' unary | '-' unary | atom             NOT 1 < 2 = (NOT 1) < 2
atom       := DECINT | DOUBLE | STRING | 'true' | 'false'
            | ID '(' (boolAnd (',' boolAnd)*)? ')'       arguments cannot be ?: without ( )
            | ID ('.' ID)* '.' (BYTESIZE|NUMBER_OF_ELEMENTS|STRUCTURE|TYPE|VALUE)
            | '(' ifelse ')'
            | IntPMF '[' ('(' SIGNED_INT ';' NUMBER ')')+ ']'
            | DoublePMF '[' ('(' SIGNED_NUMBER ';' NUMBER ')')+ ']'
            | EnumPMF ('(' 'ordered' ')')? '[' ('(' STRING ';' NUMBER ')')+ ']'
            | BoolPMF ('(' 'ordered' ')')? '[' ('(' BOOL ';' NUMBER ')')+ ']'
            | DoublePDF '[' ('(' SIGNED_NUMBER ';' NUMBER ')')+ ']'
```

**Lexer.**

- Hidden tokens are `' ' \t \r \n`, `/* */` and `//…`. Comments are allowed anywhere.
- **Keywords.** These cannot be identifiers:
  - `NOT AND OR XOR true false IntPMF DoublePMF EnumPMF BoolPMF DoublePDF ordered BYTESIZE
    NUMBER_OF_ELEMENTS STRUCTURE TYPE VALUE`
  - The match is exact: `ANDx` is an ID. A variable called `VALUE` cannot be referenced.
  - `INNER` is an ordinary ID.
- **`DECINT`** is `0 | [1-9][0-9]*`, so `01` and `00.5` are errors. Its value is
  `Integer.valueOf`, so `2147483648` is a syntax error. `-2147483648` in an expression is
  therefore an error, but it is valid as `SIGNED_INT` in an `IntPMF`.
- **`DOUBLE`** is `DECINT ('.' DIGIT* | ('.' DIGIT*)? [eE][+-]? DECINT)`.
  - Valid: `1.`, `1.e5`, `1e+5`.
  - Invalid: `.5`, `1e05` (the exponent is a DECINT), `2e` (the lexer commits to the exponent).
  - Overflow gives `Infinity`.
  - No literal is negative: `-5` is `NegativeExpression(5)`.
- **Signed literals in samples.** `SIGNED_INT` and `SIGNED_NUMBER` are datatype rules converted
  from their text, whitespace included, so `(- 5;1)` is an error. `-0` in a `DoublePMF` is
  `-0.0`. Probabilities (`NUMBER`) are unsigned.
- **`STRING`.**
  - Double or single quoted, with the escapes `\b \t \n \f \r \" \' \\ \uXXXX`. `\u` needs
    exactly 4 hex digits ("Invalid unicode"). Raw newlines are allowed.
  - Neither kind of string may contain a raw `"` or `\`.
  - **ANTLR quirk:** a single-quoted string ends at a `'` only if the next character is the end
    of input or `"`. So `'a' == 'b'` is the single string `a' == 'b`, and `EnumPMF[('a';1)]` is
    an error.
  - Rust `String` cannot hold unpaired surrogates from `\uD800`; they become U+FFFD (a
    documented [deviation](../correctness/deviations.md)).
- Every other character is `ANY_OTHER` and therefore an error. This includes `=`, `!`, `$` and
  non-ASCII letters outside strings.

## 3. Preparation: `new StoExCacheEntry(spec)` (V `StoExCacheEntry.java:39`)

Rust: `simoxide_stoex::prepare`. Errors are reported in this order:

1. Parse (see above).
2. Type inference over the whole tree (`typeInferer.doSwitch`, line 49). An unknown function name
   raises `UnsupportedOperationException` ("Function X not supported!").
   - Known names: `UniDouble Lognorm LognormMoments Norm Gamma GammaMoments Exp Pois UniInt Trunc
     Round Ceil Log Sqrt Min Max MinDeviation MaxDeviation Binom`.
   - `Binom` is known here but not to `FunctionLib`, so it fails at evaluation (§6).
3. `ProbFunctionCache` (line 54) prepares every PMF/PDF literal once (§7). An invalid literal
   raises "PMF not valid" or "PDF not valid", **even if the literal is in a branch that is never
   taken**.

## 4. Static types (A)

The evaluator asks `NPEITV.getType(node)`. It returns the raw EITV annotation mapped
`INT_PMF→INT`, `DOUBLE_PMF/DOUBLE_PDF→DOUBLE`, `ENUM_PMF→ENUM`, `BOOL_PMF→BOOL`, `ANY_PMF→ANY`.
`None` (Java `null`) means "no annotation". Rust: `types.rs`.

| Node | Raw type |
|---|---|
| int / double / string / bool literal | INT / DOUBLE / ENUM / BOOL |
| `IntPMF` / `DoublePMF` / `EnumPMF` / `BoolPMF` / `DoublePDF` | INT_PMF / DOUBLE_PMF / ENUM_PMF / BOOL_PMF / DOUBLE_PDF |
| `x.VALUE`, `.TYPE`, `.STRUCTURE` | ANY_PMF, so the evaluator resolves the type dynamically |
| `x.BYTESIZE`, `.NUMBER_OF_ELEMENTS` | INT_PMF, so it is **trusted to be an Integer** |
| `a ? b : c` | ANY |
| comparison | BOOL_PMF |
| AND / OR / XOR / NOT | BOOL |
| `(e)`, `-e` | the raw type of `e` (may be null) |
| `+ - * /` | `inferIntAndDouble(raw l, raw r)` (EITV 304). In order: INT,INT→INT; both in {INT, INT_PMF}→INT_PMF; both numeric→DOUBLE; both in {INT, DOUBLE, INT_PMF, DOUBLE_PMF}→DOUBLE_PMF; both in that set ∪ {ANY, ANY_PMF}→ANY_PMF; both in {INT, DOUBLE, INT_PMF, DOUBLE_PMF, DOUBLE_PDF}→DOUBLE_PDF; else null |
| `%` | always INT_PMF |
| `^` (NPEITV 44) | INT,INT→**INT**; numeric→DOUBLE; else ANY. The evaluated value is always a Double |
| `UniDouble Lognorm LognormMoments Norm Gamma GammaMoments Exp` | DOUBLE_PDF |
| `Pois UniInt Trunc Round Ceil Log Sqrt Binom` | INT_PMF, although `Log` and `Sqrt` return Doubles |
| `Min Max MinDeviation MaxDeviation` | raw type of the first argument (ANY without arguments) |

Consequences, all reproduced and golden-tested:

- `Log(2,8)+1`, `Sqrt(4)*2`, `2^3+1` and `(5.5%2)+1` raise `TypesIncompatibleIn*Exception`.
- `x.BYTESIZE+1` with a Double BYTESIZE raises `TypesIncompatibleInTermException`.
- `x.VALUE+1` uses int arithmetic if VALUE is an Integer.

## 5. Evaluation (V PCMStoExEval)

**Order of evaluation.**

- Children are evaluated left to right, then combined.
- Boolean operators evaluate **both** operands. There is no short circuit, and draws happen on
  both sides. The left operand is cast before the right one is evaluated.
- `?:` evaluates the condition and then **only** the chosen branch.
- Function arguments are all evaluated before the function is looked up.

**Binary operators.** Let `lt` and `rt` be the static types of the operands. For an ANY
operand, the type is taken from the value: Integer→INT, Double→DOUBLE, String→ENUM,
Boolean→BOOL, null→`RuntimeException`.

- **`+ -`** (line 280) and **`* / %`** (line 217):
  - If both types are INT, both values must be Integers, otherwise
    `TypesIncompatibleInTerm/ProductException`. Java `int` arithmetic applies: wrapping, `/`
    truncates, `/0` and `%0` raise `ArithmeticException`, and `MIN/-1` gives `MIN`.
  - Otherwise `getDouble` is applied to both (Integer or Double, else
    `UnsupportedOperationException`; null gives an NPE), and the operation runs on doubles.
- **`^`** (line 353): an INT operand is cast `(Integer)` and widened; every other operand is cast
  `(Double)` (`ClassCastException`). The result is `Math.pow`, always a Double.
- **Comparisons** (line 152):
  - If exactly one side is statically INT and the other statically DOUBLE, the INT side is
    widened, after an `(Integer)` cast.
  - The two values must then have the same class, otherwise
    `TypesIncompatibleInComparisionException`.
  - The comparison uses `compareTo`:
    - `Double.compareTo`: `-0.0 < 0.0`, `NaN == NaN`, NaN is the largest;
    - `String.compareTo`: UTF-16 code units;
    - `false < true`.
- **Unary operators and `?:`.**
  - `-`: Integer (wrapping) or Double, else `RuntimeException`.
  - `NOT`, the boolean operators and the `?:` condition cast to `(Boolean)`, which can raise
    `ClassCastException` or NPE.
- **Literals** evaluate to Integer, Double, String or Boolean.

## 6. Functions (V `functions/`, `FunctionLib.evaluate`)

- An unknown id raises `FunctionUnknownException` (only `Binom` gets that far).
- Next comes `checkParameters`, which raises `FunctionParametersNotAcceptedException`. Its
  `NumberConverter.toDouble` accepts Integer and Double and raises `RuntimeException` otherwise.
- Last comes `evaluate`.
- Distribution functions draw exactly one uniform after construction:
  `inverseF(random())` through `simoxide_random::dist`.

| Function | Accepted parameters | Result |
|---|---|---|
| `Trunc(x)` | 1 Integer or Double | Integer as is; else `(int)Math.round(Math.floor(x))`: long narrowed by **wrapping** (`Trunc(3e9)`) |
| `Round(x)` | same | `(int)Math.round(x)` (ties towards +∞, wrapping) |
| `Ceil(x)` | same | `(int)Math.ceil(x)`: saturating, NaN→0 |
| `Sqrt(x)` | 1 Integer or Double | Double |
| `Log(b, x)` | b ∉ (−∞,0] ∪ {1}, x > 0, each Integer or Double (NaN passes) | `Math.log(x)/Math.log(b)` (Double) |
| `Min`/`Max(a, b)` | 2 numbers of the **same class** (`Min(1, 2.5)` is rejected); a null first argument gives an NPE | `Math.min`/`max` in the class of the arguments |
| `MinDeviation`/`MaxDeviation(v, abs, rel)` | v a number or String; abs, rel Doubles | String → v. Otherwise `abs > v*rel ? floor(v-abs) : floor(v-v*rel)` (Min) or `ceil(v+abs)` / `ceil(v+v*rel)` (Max), then `(int)` if v is an Integer (saturating) |
| `Exp(rate)` | 1 argument, `!(rate <= 0)` | Double |
| `Norm(mean, sd)` | 2 arguments | Double |
| `Pois(mean)` | `!(mean < 0)` | Integer; Poisson(mean) − 1 (RND-3.3, [REF-4](../correctness/reference-bugs.md)) |
| `UniDouble(a, b)` | `!(a > b)` | Double; `a == b` aborts in the inversion (RND-3.2, [REF-3](../correctness/reference-bugs.md)) |
| `UniInt(a, b)` | 2 Integers | Integer |
| `Lognorm(mu, sigma)` | `!(sigma <= 0)` | Double |
| `LognormMoments(mean, sd)` | `!(mean < 0)`, `!(sd < 0)`; variance = sd·sd | Double |
| `Gamma(alpha, theta)` | `!(theta <= 0)`, `!(alpha <= 0)` | Double |
| `GammaMoments(mean, cv)` | `!(mean < 0)`, `!(cv < 0)` | Double |

Distribution constructor errors raise `ProbabilityFunctionException` or a Commons Math
exception, and consume no uniform.

## 7. PMF and PDF literals (V `ProbFunctionCache`; M)

**Preparation** happens once per cache entry.

1. **Adjustment** (`ProbFunctionCache` 60/137).
   - `sum` is added up in literal order.
   - If `|sum-1| > 10e-10` (= 1e-9), `delta = (1-sum)/count(p>0)` is added to every `p>0`.
     The EMF model is modified in place.
2. **Sort.** `transformToPMF` / `BoxedPDFImpl.setSamples` sort the samples stably by value
   (`compareTo`).
   - A PDF with duplicate values (`Double.equals`) raises `DoubleSampleException`.
   - The segment table is built (`MathTools.computeLines`). The line through `(0,0)` and the
     first sample throws if the first value is 0, so an implicit sample at 0 is assumed.
3. **`checkConstrains`.**
   - The sorted sum must satisfy `|sum-1| < 1e-5` (`MathTools.equalsDouble`).
   - Every probability must be in `[0,1]` (NaN passes that check but fails the sum).
   - PDF: no value may be `< 0`.
4. **Cumulative sums** are computed in sorted order.

**Sampling** draws one uniform `u` per evaluation.

- **PMF** (PMFImpl 290): the value of the first `i` with `u < cum[i]`. If there is none (a sum
  just below 1 after the adjustment), the result is **the Double 0.0**, even for an `IntPMF`.
  In an arithmetic context this then raises the static-INT errors.
- **Boxed PDF** (BoxedPDFImpl 126):
  - take the first `i` with `u < cum[i]` and return `(u-b)/a`;
  - `a = (cum[i]-cum[i-1])/(v[i]-v[i-1])` and `b = cum[i-1]-a·v[i-1]`, with the point `(0,0)`
    before the first sample (`Line.getX`);
  - if there is no `i`, `RuntimeException("No interval found…")`.

## 8. Floating point: which Java

The reference runs on HotSpot 21 on x86-64 (`reference/refsim` uses `java`). Java 17 gives the
same results except for NaN payloads of `%`.

- `+ - * / sqrt` are IEEE. Rust uses the same instructions. Subtraction is kept out of line
  (`jmath::dsub`), because LLVM may merge `a+b`/`a-b` into `a+(±b)`, which flips the sign of a
  NaN.
- `%` (`drem`) gives the exact fmod value. The NaN bit patterns of Java 21 are reproduced
  (`jmath::drem`): `NaN % y` → `0x7ff8…`; `±inf % NaN` → `0x7ff8…`; `finite % NaN` → that NaN,
  quieted; invalid → `0xfff8…`.
- `Math.pow` and `Math.log` are **Intel LIBM intrinsics**, not fdlibm (`StrictMath`) and not
  glibc. Measured on 300 000 inputs (`JavaMathDump`):
  - `Math.log` is correctly rounded in 100 % of the measured cases;
  - `Math.pow` is correctly rounded in 99.96 % of them;
  - glibc mismatches 0.16 % (pow) and 0.013 % (log); the `libm` crate (fdlibm) mismatches 6.7 %.
  - `jmath::log` and `jmath::pow` are therefore correctly rounded. They use double-double
    evaluation, exact integer powers, `pow(x, 0.5) = sqrt(x)`, and Java's special cases and NaN
    patterns.
  - Result: **log bit-exact on all 600 017 samples; pow differs by 1 ulp on 122 of 300 289 (0.04 %)**,
    in cases where Intel's pow is not correctly rounded. Subnormal pow results may be
    double-rounded. This is a documented [deviation](../correctness/deviations.md) without an
    option.
  - No model in the corpus uses `^` or `Log`.
- Java casts:
  - `(int)double` saturates and maps NaN→0;
  - `Math.round` gives a long, ties towards +∞;
  - `(int)long` wraps.

## 9. Rust API (`crates/simoxide-stoex`)

- `parse(&str) -> Result<Expr, ParseError>`. `ParseError` has line and column.
- `prepare(&str) -> Result<Prepared, PrepareError>` does §3.
- `Program::compile(&Prepared, resolver)` and `Program::from_str`:
  - the resolver maps each `VarRef` to a caller slot;
  - static types are baked into the nodes;
  - parentheses are dropped;
  - deterministic subtrees are folded to constants or to stored errors, which are raised at the
    same point of the evaluation order.
- `Program::eval(&env, &mut rng)`, `eval_mode(…)`, `eval_f64/i32/bool`. `Env::lookup(slot,
  rng)` is the stack frame. Evaluation does not allocate for numeric values.
  - Measured (`examples/bench.rs`): 2 ns for a constant, 5–13 ns for a variable, 13–45 ns for
    small arithmetic, 12–16 ns for a PMF/PDF sample.
- `interp::eval`: the reference tree walker, a literal transcription of the visitor. It is used
  to cross-check the compiler.
- `print::print`: a canonical, re-parsable printer. The Xtext formatter output for whole
  expressions is not reproduced: it is irregular (for example `NOT trueAND false`) and sometimes
  throws. Only variable ids matter to the simulator.

## 10. Verification

- **Golden tests.** `reference/oracles/stoex/golden/stoex_golden.jsonl` holds 8 834 cases,
  generated by `gen_cases.py` and run through the real 5.2.2 classes by
  `StoexOracle.java`:
  - all 466 StoEx strings of all PCM models in the research repos, with 3 variable bindings each;
  - about 400 hand-written edge cases;
  - variable modes;
  - distribution functions;
  - 4 000 random expressions and 3 000 mutations.
- **What each case checks.**
  - parse acceptance;
  - the tree, including literal bits;
  - variable ids;
  - preparation errors;
  - the root type;
  - per evaluation: the value (bitwise) or the Java exception class, and **the exact number of
    uniforms drawn**.
  - Both the compiler and the tree walker are checked.
  - An extra run of 100 000 random and mutated cases (`gen_cases.py --only-generated`) matched
    completely.
- **Other tests.**
  - `golden/javamath.txt` checks pow, log and `%`.
  - proptest: print→parse round trip of random grammatical trees, and compiled program ≡ tree
    walker (values, errors, draws).
- **Distribution samples** match bitwise. The oracle, like the reference simulator, runs on the
  OSGi-ordered classpath: Commons Math 2.1 comes before the older copy embedded in DESMO-J
  (RND-9.2 in [Random numbers and sampling](./random.md);
  [Classpath](../reference-simulator/patches.md#classpath)). A sample mismatch fails the test;
  `STOEX_LAX_DIST=1` only reports it.

# Eclipse QVTo + classic Eclipse OCL — implementation-specific semantics catalogue

Research input for the Rust QVTo engine ("compat mode" = reproduce Eclipse exactly, "fast mode" = sane/spec-ish).

* Sources: `eclipse-qvto` @ 8c01c4f6 (2026-09-15), `eclipse-ocl` @ 93585c74 (2026-09-15), specs QVT 1.3 / OCL 2.4 (text).
* All paths are relative to the respective `plugins/` directory. Abbreviations used in citations:

| Abbrev | Path |
|---|---|
| `QE/` | `org.eclipse.m2m.qvt.oml/src/org/eclipse/m2m/internal/qvt/oml/evaluator/` |
| `QA/` | `org.eclipse.m2m.qvt.oml/src/org/eclipse/m2m/internal/qvt/oml/ast/env/` |
| `QP/` | `org.eclipse.m2m.qvt.oml/src/org/eclipse/m2m/internal/qvt/oml/ast/parser/` |
| `QS/` | `org.eclipse.m2m.qvt.oml/src/org/eclipse/m2m/internal/qvt/oml/stdlib/` |
| `QL/` | `org.eclipse.m2m.qvt.oml/src/org/eclipse/m2m/internal/qvt/oml/library/` |
| `QU/` | `org.eclipse.m2m.qvt.oml/src/org/eclipse/m2m/qvt/oml/util/` |
| `QT/` | `org.eclipse.m2m.qvt.oml/_generated/org/eclipse/m2m/internal/qvt/oml/trace/impl/` |
| `QO/` | `org.eclipse.m2m.qvt.oml/src/org/eclipse/m2m/internal/qvt/oml/` (top level) |
| `O/`  | `org.eclipse.ocl/src/org/eclipse/ocl/` (classic OCL, eclipse-ocl checkout) |
| `OE/` | `org.eclipse.ocl.ecore/src/org/eclipse/ocl/ecore/` |

Determinism legend used throughout:

* **D** – deterministic, depends only on program + input.
* **D-J** – deterministic, but only if you emulate a specific Java algorithm (HashMap layout, `String.hashCode`, `Double.toString`, …).
* **N** – non-deterministic across JVM runs (depends on `Object.hashCode()` identity hashes or allocation). Cannot be reproduced bit-exactly; compat mode must pick a documented surrogate.
* **L** – depends on JVM default locale / line separator.

---------------------------------------------------------------------------------------------------

## 0. Executive summary — the compat points that matter most

(The full list with rationale is in the sections below; this is the condensed version.)

1. **Set = `java.util.HashSet`, Bag = `BagImpl` (backed by `HashMap<E,count>`), OrderedSet = `LinkedHashSet`, Sequence = `ArrayList`, QVTo `List` = `MutableListImpl extends ArrayList`, `Dict` = `DictionaryImpl` over `HashMap`.** Iteration order of Set/Bag/Dict is Java-HashMap bucket order. `O/util/CollectionUtil.java:1159-1210`, `O/util/BagImpl.java:30-45`, `QS/DictionaryImpl.java:27`, `QS/MutableListImpl.java:24`.
2. **Every EObject (and every enum literal, Tuple?, ModuleInstance, Status, Exception) hashes by identity** → order of any Set/Bag/Dict containing model objects is **N**. This affects: `Set{...}` of objects, `asSet()`, navigation of *unordered* multi-valued features (OCL type Set/Bag → `new HashSet(eList)`), `allInstances()`, `subobjects()`, `allSubobjects*()`, `Dict.keys()/values()` with object keys, `:=` to an unordered feature, `xcollect` over a Set (result Bag).
3. **Strings/Integers/Doubles/Booleans hash deterministically** (`String.hashCode`, `Integer.hashCode()==value`, `Double` bit-fold, `1231/1237`) → Set/Bag/Dict of primitives iterate in **D-J** order (bucket order of a power-of-two table with `h ^ (h>>>16)` spreading, capacity/resize history matters).
4. Runtime **Java class**, not static OCL type, drives later behaviour: e.g. `Model::objectsOfType()` / `rootObjects()` are statically `Set` but return `LinkedHashSet` (ordered) — subsequent `select` keeps order, but QVTo's `Set::including/excluding` and `Collection::flatten` explicitly convert a `LinkedHashSet` source to a `HashSet` first (`QS/SetTypeOperations.java:120-122,140-142`, `QS/CollectionTypeOperations.java:146-148`). Compat mode needs a "Java runtime collection class" tag on every collection value.
5. **`visitExpression` swallows every non-QVT `RuntimeException`**: logs `Evaluation failed at '<file>:<line>' with a reason: <msg>. Expression yields an 'invalid' value.` to the plugin log (stderr standalone) and returns `invalid` (`QE/QvtOperationalEvaluationVisitorImpl.java:415-432`). Operation calls swallow silently (`:750-762`). Only `QvtRuntimeException` (incl. `raise`, fatal assert, stack overflow, unknown property, read-only, interrupted) propagates.
6. **`invalid` is a singleton Java object, `null` is Java null**; `if` with null/invalid condition → `invalid` (not an assertion failure as QVT 1.3 §8.1.20 demands) (`:322-340`); `while`/`when`/`where`/`log when`/`assert` treat anything but `Boolean.TRUE` as false.
7. **`xcollect` does NOT flatten** (spec §8.2.2.7 says `BODY->flatten()`), drops `null`s, an `invalid` body makes the whole result `invalid`; result is `Bag` for Set/Bag sources, `Sequence` otherwise (also for `List`!) (`QE/QvtOperationalEvaluationVisitorImpl.java:1561-1565`, `QE/iterators/QvtIterationTemplateXCollect.java:38-43`). `coll->prop` on a collection compiles to xcollect, `coll.prop` to OCL `collect` (`QP/QvtOperationalVisitorCS.java:5676-5686`).
8. **Mapping execution order**: bind params → `when` (each must be TRUE; else result null, `xmap` raises `Precondition failed in mapping 'm'`) → trace lookup (cache hit returns old result, out params refreshed) → disjunct dispatch → `init` → implicit instantiation of null result/out params → **trace record added (before population!)** → inherited mappings → population → `end` → merged mappings → result added to extent if orphan → `where` (fatal `Postcondition failed in mapping 'm'`, evaluated even on cache hit). `QE/QvtOperationalEvaluationVisitorImpl.java:612-658,803-872`.
9. **Trace cache key** = `ArrayList[self, non-out params…]` (equals/hashCode semantics → Integer 1 ≠ Real 1.0; Strings by value; EObjects by identity) per mapping in a `HashMap`; later records overwrite earlier ones for the same key (`QE/TraceUtil.java:273-291`, `QT/TraceImpl.java:303-325`).
10. **resolve family**: source map is keyed by the *context object, or else the first IN/INOUT parameter whose type is not a predefined OCL type*; `invresolve` only returns the **context** (`self`) of records — non-contextual mappings are never found by `invresolve`; results are `Sequence` (ArrayList) in trace-record insertion order; `resolveone` returns first match; no match → `null` / empty Sequence (`QL/QvtResolveUtil.java`).
11. **late resolve**: only effective when the resolve is the direct RHS of an assignment to a *property* (optionally wrapped in one `->asSet/asBag/asSequence/asOrderedSet`); tasks run after `main` returns, FIFO; assignments to variables with late resolve are silently dropped; the LHS owner and resolve source are evaluated eagerly (`QL/AbstractLateResolveTask.java:35-72`, `QL/QvtResolveUtil.java:96-160`).
12. **Assignment semantics** (`QA/QvtOperationalEvaluationEnv.java:684-850`): `:=` on a collection variable/feature builds a *new* collection of the declared kind (Set ⇒ HashSet ⇒ hash order!) and adds RHS elements one level deep (no deep flatten); `x := null` for a collection variable yields an *empty collection*, not null; `+=` copies (or mutates in place for `List`/`Dict`) and appends; `+=` on single-valued feature acts like `:=` except null/invalid never unsets; many-valued feature update uses `ECollections.setEList` (minimal moves) with fallback `clear(); addAll()` and drops nulls for EObject lists.
13. **Uninitialised variables / properties get type defaults**, not null: String `""`, Boolean `false`, Integer `0`, Real `0.0`, UnlimitedNatural `0`, collections empty, tuples recursively defaulted (`QE/EvaluationUtil.java:278-317`; used by `var x : T;`, module properties, intermediate-class features, contextual intermediate properties, mapping result params of non-object type).
14. **Intermediate contextual property initialisers are ignored** (analyzer warns `IntermediatePropertiesInitNotSupported`; runtime uses the type default) (`QP/QvtOperationalVisitorCS.java:4835-4839`, `QE/IntermediatePropertyModelAdapter.java` `getInitialValue`).
15. **Object creation & extents**: new objects are appended to the extent's `myAdditionalEObjects` and to an in-memory resource; extent of an untagged `object`/result = first non-`in` model parameter whose model type's *first* metamodel package is the class's root package, else the first non-`in` model parameter regardless of metamodel, else the "unbound" extent (`QA/QvtOperationalModuleEnv.java:167-199`, `QA/QvtOperationalEvaluationEnv.java:615-636`). Output root order = creation order of still-uncontained objects; **contained objects are purged every max(150,size) additions** and are then lost as roots even if later detached (`QA/ModelParameterExtent.java:155-168,290-320`).
16. **`log` format** (WriterLog): `msg` / `msg, data: <String.valueOf(elem)>` / `Level <n> - msg` / `Level <n> - msg, data: <elem>`; level formatted by `MessageFormat` (**L**: `1,000`); null msg → `<null>`, invalid → `<Invalid>`; null element ⇒ data part omitted; EObject element prints `pkg.Class@hexIdentityHash (attr: val, …)` (**N**) (`QU/WriterLog.java:77-90`, `QE/QvtOperationalEvaluationVisitorImpl.java:1391-1451`).
17. **`assert`**: `ASSERT [<severity>] failed at (<file>:<line>)` then ` : <log msg>` if `with log`; fatal ⇒ `QvtAssertionFailed(<log msg without prefix> | "Fatal assertion failed")`; non-fatal ⇒ diagnostic (WARNING/ERROR, code 150) (`:1453-1485`).
18. **`try/except` only catches `raise`d QVT exceptions and fatal-assert `AssertionFailed`**; `return`/`break`/`continue` inside an `except` body are **ignored**; `try` always yields null (`:1670-1726`).
19. **String stdlib bugs**: null-argument guard `leftVal == null && leftVal == invalid` is always false ⇒ null args → NPE ⇒ silently `invalid`; `normalizeSpace` keeps the *first* whitespace char of each run instead of `' '`; `find/rfind` return 1-based index, 0 if not found (spec unclear), `format`/`Integer::range`/`List::deepclone`/`_localId`/`markedAs`/`stereotypedBy`/`parallelTransform`/`asTransformation` are *unsupported* (compile error); `matchBoolean/matchInteger/matchFloat` take an argument (spec: none); `asReal/matchReal` absent; string counters are global per execution context (`QS/StringOperations.java`).
20. **Dict**: HashMap order; `Dict{…}` literal skips pairs with invalid key or value; iterating a Dict iterates *values*; `Collection→Dict` coercion puts elements as *keys* with null values, while `Dict::add` (via `+=`) stores the element as value under key `null` (`QS/DictionaryImpl.java:100-106`, `QU/Utils.java:36-42`).
21. **Numbers from classic OCL** (§B): Integer = `Integer`, promoted to `Long` when it does not fit (QVTo turns on long literals), **64-bit wrap-around, no BigInteger**; `Int / Int` → Real, `/0` → invalid; `div` truncates, `mod` sign follows dividend, `x mod 0` → invalid; `floor()`/`round()` cast to **`int`** (saturate / wrap); `UnlimitedNatural *` is the Integer **−1** recognised only by static type; Real printed with `Double.toString` (`1.0`, `1.0E10`, JDK-version dependent for some values); `toInteger` = `Integer.valueOf` (no whitespace, 32-bit only), `toReal` = `Double.valueOf` (accepts whitespace, `NaN`, `1.5f`, hex).
22a. **Boolean logic maps null to invalid** (`true and null` → invalid, `not null` → invalid, `false or null` → invalid; short-circuits `false and X`, `true or X`, `X or true`, `X implies true` still win); `exists/forAll` return invalid as soon as a body is null/invalid, regardless of later deciding elements (order-dependent) (§D.2, §A.3).
22b. **Strings compare by UTF-16 code units** (`String.compareTo`), case mapping uses the JVM default locale (**L**), indices are UTF-16 based, `indexOf` 1-based with 0 = not found, `substring` needs `1 ≤ l ≤ u ≤ size` (else invalid), `+` only between Strings (§C).
22. **Stack depth limit 1000 nested evaluation environments** → `QvtStackOverFlowError("Stack depth: N")` (`QE/QVTEvaluationOptions.java:28`, `QE/EvaluationUtil.java:176-182`).
23. **`subobjects()` & friends return `HashSet<EObject>`** (N), `objects*()`/`rootObjects()` return `LinkedHashSet` in extent order (D), `allInstances()` returns a `HashSet` over all extents incl. unbound (N) (`QS/ElementOperations.java:399-430`, `QS/ModelOperations.java:86-99,173-188`, `QA/QVTExtentMap.java:72-96`).

---------------------------------------------------------------------------------------------------

## 1. Where the implementation lives (for citations)

### 1.1 QVTo interpreter (org.eclipse.m2m.qvt.oml)

| File | Lines | Role |
|---|---:|---|
| `QE/QvtOperationalEvaluationVisitorImpl.java` | 2654 | Main evaluator: all ImperativeOCL / QVT expressions, mapping protocol, module init, extent handling. Extends classic OCL `O/EvaluationVisitorImpl`. |
| `QE/TraceUtil.java` | 1035 | Trace record creation / lookup / (incremental) reuse. |
| `QE/QvtGenericEvaluationVisitor.java` | 527 | Generic decorator support. |
| `QE/EvaluationUtil.java` | 354 | Default values, collection kinds, list coercion, evaluation result. |
| `QE/OCLAnnotationSupport.java` | 273 | OCL bodies of dynamic Ecore operations/derived features. |
| `QE/ModelParameterHelper.java` | 218 | Binding model arguments to transformation parameters. |
| `QE/GraphWalker.java` | 202 | |
| `QE/QvtStackTraceBuilder.java` / `QVTStackTraceElement.java` | 177 / 119 | QVT stack traces (`Module::op(file:line)`). |
| `QE/ModuleInstanceImpl.java`, `ModuleInstanceFactory.java`, `TransformationInstanceImpl.java` | 160/151/120 | `this` objects. |
| `QE/IntermediatePropertyModelAdapter.java`, `RuntimePropertyShadowHelper.java` | 135/80 | Storage of contextual intermediate properties. |
| `QE/QVTEvaluationOptions.java` | 123 | Session options (max stack depth 1000, read-only guard, …). |
| `QE/NumberConversions.java` | 95 | Number coercion on assignment. |
| `QE/QvtRuntimeException.java`, `QvtException.java`, `QvtAssertionFailed.java`, `QvtStackOverFlowError.java`, `QvtInterruptedExecutionException.java` | 84/32/26/32/36 | Exceptions. |
| `QE/iterators/QvtIterationTemplate*.java` | 200 + 149 + 43 + 54 + 50 + 60 | forEach/forOne, xcollect, xselect, collectselect(+One). |
| `QE/messages.properties` | – | runtime message texts (quoted in this doc). |
| `QA/QvtOperationalEvaluationEnv.java` | 1227 | Variables, `assign`, `callSetter`, instantiation, extents, deferred tasks, `isKindOf`. |
| `QA/QvtOperationalEnv.java`, `QvtEnvironmentBase.java` | 972/867 | Compile-time env. |
| `QA/ModelParameterExtent.java` | 504 | Model extents (root objects, purge). |
| `QA/QvtOperationalStdLibrary.java` | 426 | QVT stdlib registration. |
| `QA/QVTExtentMap.java` | 97 | `allInstances` extent map. |
| `QA/VirtualTable.java` | 138 | Virtual dispatch of contextual operations. |
| `QL/QvtResolveUtil.java` | 448 | resolve/resolveIn/invresolve/late resolve. |
| `QL/AbstractLateResolveTask.java`, `LateResolveTask.java`, `LateResolveInTask.java` | 94/32/32 | Deferred assignment tasks. |
| `QL/Context.java` | 179 | IContext (config properties `HashMap`, log, session data). |
| `QO/InternalTransformationExecutor.java` | 411 | Top-level execution → `ExecutionDiagnostic`. |
| `QO/ExecutionDiagnosticImpl.java` | 129 | Diagnostic. |
| `QO/QvtPlugin.java` | 229 | Error log (Eclipse log or `System.err` `ERROR <code>: <msg>`). |
| `QP/QvtOperationalVisitorCS.java` | 5988 | CST→AST analyzer (implicit collect/xcollect, implicit object exp, …). |
| `QP/QvtOperationalValidationVisitor.java` | 1340 | AST validation. |
| `QP/IntermediateClassFactory.java` | 415 | Intermediate classes, exception instances. |
| `QT/TraceImpl.java` (generated + `@generated NOT`) | ~330 | Trace maps. |

### 1.2 QVTo standard library implementation (`QS/`)

| File | Lines | Defines |
|---|---:|---|
| `StringOperations.java` | 557 | length, substringBefore/After, firstToUpper, lastToUpper, normalizeSpace, replace, match, find, rfind, isQuoted, quotify, unquotify, matchBoolean/Integer/Float/Identifier, asBoolean/asInteger/asFloat, str counters, addSuffixNumber, `format` (UNSUPPORTED) |
| `QVTUMLReflection.java` | 479 | type reflection |
| `ElementOperations.java` | 431 | subobjects*, allSubobjects*, clone, deepclone, container, metaClassName; `_localId`, `_globalId`, markedAs, markValue, stereotypedBy, stereotypedStrictBy UNSUPPORTED |
| `ListOperations.java` | 340 | List(T) add/insertAt/remove/removeAll/removeAt/removeFirst/removeLast/joinfields/first/last/at/clone/selectByType/selectByKind/subSequence/flatten/including/excluding/union/reverse/append/prepend/insertAt/indexOf; deepclone UNSUPPORTED |
| `CollectionTypeOperations.java` | 246 | Collection::asSet/asOrderedSet/asSequence/asBag/asList/flatten/repr/dump/includes/excludes/count/includesAll/excludesAll/=/<> (override OCL's, bug 467600) |
| `AbstractContextualOperations.java` | 209 | registration infrastructure, `UNSUPPORTED_OPER` |
| `ModelOperations.java` | 190 | objects/objectsOfType/objectsOfKind/rootObjects/removeElement/copy/createEmptyModel/allInstances(deprecated); asTransformation UNSUPPORTED |
| `ConversionCollection.java`, `ConversionDictionary.java`, `ConversionUtils.java` | 169/166/101 | config-property string → value conversion |
| `SetTypeOperations.java` | 163 | Set::union/intersection/-/including/excluding/symmetricDifference |
| `OrderedSetTypeOperations.java` | 162 | append/prepend/insertAt/indexOf/reverse |
| `DictionaryImpl.java`, `DictionaryOperations.java` | 165/138 | Dict |
| `TransformationOperations.java` | 95 | transform(); parallelTransform, wait UNSUPPORTED |
| `StatusOperations.java`, `ExceptionOperations.java` | 73/48 | Status, Exception::getArgument |
| `MutableListImpl.java` | 68 | List runtime class |
| `BagTypeOperations.java`, `SequenceTypeOperations.java` | 61/62 | |
| `StdlibModuleOperations.java` | 60 | dump (deprecated) |
| `ObjectOperations.java`, `OclAnyOperations.java` | 52/42 | repr, OclAny::allInstances (deprecated → allSubobjectsOfKind!) |
| `IntegerOperations.java`, `RealOperations.java` | 36/26 | `Integer::range` UNSUPPORTED; Real: nothing |
| `model/StdlibFactory.java` | 155 | Status/Exception instances |

Registration order: `QA/QvtOperationalStdLibrary.java:139-158`.

### 1.3 Classic OCL evaluator files

| File | Lines |
|---|---:|
| `O/EvaluationVisitorImpl.java` | 2801 |
| `O/AbstractEvaluationVisitor.java` | 581 |
| `O/util/CollectionUtil.java` | 1286 |
| `O/util/BagImpl.java` | 211 |
| `O/internal/evaluation/IterationTemplate*.java` | 180 + 13 small templates (51-135 each) |
| `O/internal/evaluation/NumberUtil.java` | 201 |
| `OE/EcoreEvaluationEnvironment.java` | 531 |
| `OE/internal/OCLStandardLibraryImpl.java` | 589 |

### 1.4 Parser (details in §P)

QVTo ships its **own copies** of the EssentialOCL grammar in `org.eclipse.m2m.qvt.oml.cst.parser/cst/` (identical to `org.eclipse.ocl/src/org/eclipse/ocl/parser/EssentialOCL*.gi` except headers) — it does *not* reference the OCL plugin grammars at build time.

| File (`org.eclipse.m2m.qvt.oml.cst.parser/`) | lines | bytes |
|---|---:|---:|
| `cst/QVTOParser.gi` (→ imports ImperativeOCL.gi) | 2384 | 69319 |
| `cst/ImperativeOCL.gi` (→ imports EssentialOCL.gi) | 1983 | 57425 |
| `cst/EssentialOCL.gi` | 1484 | 58633 |
| `cst/QVTOLexer.gi` → `ImperativeOCLLexer.gi` → `EssentialOCLLexer.gi` → `lpg/LexerBasicMapF.gi` | 110 / 125 / 591 / 273 | 2002 / 2198 / 12710 / 14515 |
| `cst/QVTOKWLexer.gi` → `ImperativeOCLKWLexer.gi` → `EssentialOCLKWLexer.gi` → `lpg/KWLexerMapF.gi` | 537 / 222 / 302 / 79 | 7763 / 3387 / 4697 / 3647 |
| `lpg/btParserTemplateF.gi`, `LexerTemplateF.gi`, `KeywordTemplateF.gi` | 354 / 295 / 119 | |
| generated `src/.../cst/parser/QVTOParser.java`, `QVTOParserprs.java`, `QVTOParsersym.java` | 7078 / 5100 / 397 | 226373 / 244619 / 13156 |
| generated `QVTOLexer.java`, `QVTOLexerprs.java`, `QVTOKWLexer.java` | 896 / 582 / 1004 | |
| hand-written `AbstractQVTParser.java` | 1436 | 60210 |
| classic OCL counterpart (not used by QVTo): `org.eclipse.ocl/src/org/eclipse/ocl/parser/OCLParser.g` → `EssentialOCL.gi` | 616 / 1479 | 20662 / 58559 |
| OCL support code used at runtime: `O/parser/AbstractOCLParser.java` (CST factories, literal conversion), `O/lpg/AbstractParser.java` (escape decoding) | 783 / 749 | |

Backtracking LALR(2) (`lpg -backtrack`, 643 states, 837 rules, 162 terminals, 344 nonterminals).

---------------------------------------------------------------------------------------------------

## 2. Runtime value representation

| OCL/QVT value | Java runtime class | hashCode | equals | toString / `repr()` |
|---|---|---|---|---|
| `null` | `null` | – | – | `"null"` (`String.valueOf`) |
| `invalid` | singleton returned by `getInvalid()` (OCL std-lib invalid object, see §A) | identity | identity | see §A |
| Boolean | `java.lang.Boolean` | 1231 / 1237 | value | `true`/`false` |
| Integer | `java.lang.Integer` when it fits in 32 bits, else `java.lang.Long` (QVTo enables `USE_LONG_INTEGERS`; results normalised by `NumberUtil.coerceNumber`); EMF attributes may also deliver uncoerced `Long/Short/Byte/BigInteger` (§B.1) | value / long fold | value & class (Integer(1) ≠ Long(1) ≠ Double(1.0) in Java `equals`!) | `toString` |
| Real | `java.lang.Double` | `(int)(bits ^ (bits>>>32))` | `Double.equals` (NaN==NaN, 0.0≠-0.0) | `Double.toString` (e.g. `1.0`, `1.0E10`, `1.0E-4`) **D-J** |
| String | `java.lang.String` (UTF-16) | `s[0]*31^(n-1)+…` | value | itself |
| Enum literal | dynamic EMF: `EEnumLiteral` (an EObject); generated: Java `enum` | **identity (N)** | identity | literal name / `EEnumLiteralImpl.toString` |
| Model element | `EObject` (`DynamicEObjectImpl`/generated) | **identity (N)** | identity | `pkg.ClassImpl@<hex identityHash> (eClass: …)` / `(attr: v, …)` **N** |
| Set | `HashSet` | sum of element hashes | `AbstractSet.equals` | `[a, b]` in iteration order |
| OrderedSet | `LinkedHashSet` | sum | **Set equality (order-insensitive)** | `[a, b]` |
| Bag | `O/util/BagImpl` (HashMap<E,MutableInteger>) | see §A | see §A | see §A |
| Sequence | `ArrayList` | list hash | list equality | `[a, b]` |
| List (QVT) | `QS/MutableListImpl extends ArrayList` | list hash | **equals any `List` incl. Sequence** | `[a, b]` |
| Dict (QVT) | `QS/DictionaryImpl` (wraps `HashMap`) | map hash | only other `DictionaryImpl` with equal map | `Dict{k=v, …}` (HashMap order) |
| Tuple | OCL `TupleInstance` / EObject tuple (see §A) | see §A | see §A | see §A |
| Model param value | `ModelInstance` (EObject) | identity | identity | EObject default |
| Transformation / module `this` | `ModuleInstance` (EObject) | identity | identity | |
| Status / Exception | `StdlibFactory.Status` (`SUCCESS`/`FAILED raised: (…)`), `ExceptionClassInstance` (`<EClassName> : <arg>\n\tat …`) | identity | identity | `QS/model/StdlibFactory.java:96-104`, `QP/IntermediateClassFactory.java:349-360` |

Consequences:

* `Set{1, 1.0}` has two elements (HashSet uses Java `equals`; `Integer(1).equals(Double(1.0))` is false); `Set{1}->includes(1.0)` is false, while `1 = 1.0`, `count`, `indexOf` use `ObjectUtil.equal` with numeric promotion and say equal (§A.2, §B.2).
* Top-level `=` on collections goes through `ObjectUtil.equal` (QVTo's `Collection::=` handler too, `QS/CollectionTypeOperations.java:220-244`): OrderedSet vs OrderedSet compares **in order**, OrderedSet vs any other kind is false; everything else is Java `equals` (`OU:41-118`). But OrderedSets *nested inside* a HashSet/ArrayList compare with `LinkedHashSet.equals` = order-insensitive.
* `List{1,2} = Sequence{1,2}` → true via `ArrayList.equals`.

**Compat strategy:** a `Value` enum whose collection variant carries `JavaKind ∈ {HashSet, LinkedHashSet, BagImpl, ArrayList, MutableList, Dict}` plus an emulated Java hash table for the hash-based kinds (see §3). Equality and hashing functions must be literal ports of the Java `equals`/`hashCode` of the corresponding class. **Fast strategy:** `IndexSet`/`IndexMap` (insertion order) everywhere, OCL-spec equality.

---------------------------------------------------------------------------------------------------

## 3. Java hashing / HashMap emulation (needed for compat mode)

Everything order-relevant in Set/Bag/Dict comes from `java.util.HashMap` (HashSet and BagImpl and DictionaryImpl are all HashMap-backed). JDK 8+ algorithm (unchanged through JDK 21):

* `hash(key) = key == null ? 0 : (h = key.hashCode()) ^ (h >>> 16)`; bucket = `hash & (table.length-1)`.
* Default `new HashMap()` / `new HashSet()`: table allocated lazily with capacity 16, threshold 12 (load factor 0.75). Resize doubles capacity when `++size > threshold`; on resize each bucket list is split into lo/hi lists *preserving relative order*.
* `new HashSet(Collection c)`: `new HashMap(max((int)(c.size()/.75f)+1, 16))` → capacity = `tableSizeFor(that)` (next power of two), then `addAll` in `c`'s iteration order. `new HashMap(Map m)` / `putAll`: `tableSizeFor((int)((float)s/0.75f + 1.0f))` when larger than current threshold.
* Bucket chains are appended at the tail; iteration = bucket index ascending, chain order. Treeification (chain ≥ 8 and table ≥ 64) keeps the `next` order for iteration; removal unlinks in place.
* `HashMap.keySet()/values()/entrySet()` iterate in that same order; `new ArrayList(set)` / `new MutableListImpl(map.keySet())` snapshot it.
* `LinkedHashSet`: insertion order; re-adding an existing element does **not** move it.

Hash codes that must be reproduced:

| Type | Java hashCode |
|---|---|
| `String` | `h = 31*h + c` over UTF-16 code units, int overflow |
| `Integer` | the value |
| `Long` | `(int)(v ^ (v>>>32))` |
| `Double` | `bits = doubleToLongBits(v)` (canonical NaN), `(int)(bits ^ (bits>>>32))` |
| `Boolean` | 1231 (true) / 1237 (false) |
| `ArrayList`/`MutableListImpl` | `h = 31*h + (e==null?0:e.hashCode())`, start 1 |
| `HashSet`/`LinkedHashSet` | sum of element hashes (null → 0) |
| `HashMap` (Dict) | sum of `keyHash ^ valueHash` |
| `BagImpl` / Tuple | see §A |
| `EObject`, enum literals (dynamic `EEnumLiteral` *and* generated Java enums – `Enum.hashCode` is identity), `ModuleInstance`, `TraceRecord`, `Status`, exception instances | `System.identityHashCode` → **N** |

Identity hashes on HotSpot (default `-XX:hashCode=5`) are a thread-local Marsaglia xor-shift sequence seeded from `os::random()`; the value consumed by an object depends on every identity hash previously requested on that thread (including JVM/EMF-internal ones), so it is not reproducible from the QVTo program alone.

**Compat strategy:**
* Implement a `JavaHashMap<K,V>` clone (table, chain order, resize/split, `tableSizeFor`, constructors with initial capacity) and `JavaHashSet`, `BagImpl` on top of it; port `hashCode()` for primitives, strings (UTF-16!), lists, sets, maps.
* For identity-hashed values, supply a pluggable `IdentityHashSource`: default = deterministic pseudo-identity hash (e.g. emulate HotSpot xor-shift `(x,y,z,w) = (seed, 842502087, 0x8767, 273326509)`, one draw per object on first hash request, masked to 31 bits) so the *shape* of the behaviour (scrambled, capacity-dependent) is Java-like and reproducible run-to-run in Rust; document that it cannot equal any particular JVM run. Optionally a "creation order" source (`identityHash = allocation counter`) which makes HashSet iteration ≈ creation order for < capacity elements — handy for golden-file comparisons.
* Emit a compat-mode warning ("order depends on Java identity hash codes") whenever iteration order of a hash-based collection containing identity-hashed elements is observed by output (assignment to an ordered feature, string conversion, `first()`, `any`, `asSequence`…).

**Fast strategy:** insertion-ordered sets/maps (IndexMap) — deterministic and usually what users expect.

---------------------------------------------------------------------------------------------------

## 4. Collections

### 4.1 Factory / kinds

| Item | Spec | Eclipse | Source | Det. |
|---|---|---|---|---|
| Set/OrderedSet/Bag/Sequence classes | abstract | HashSet / LinkedHashSet / BagImpl / ArrayList | `O/util/CollectionUtil.java:1159-1210` | D-J / N |
| `createNewCollectionOfSameKind` | – | Bag→Bag, LinkedHashSet→OrderedSet, Set→Set, anything else (incl. EList, MutableList) → **Sequence** (OCL util) ; QVTo variant keeps MutableList/Dict | `O/util/CollectionUtil.java:1114-1128`, `QE/EvaluationUtil.java:238-250` | D |
| `List` literal `List{…}` | §8.2.2.29 | `MutableListImpl`; invalid element → whole literal `invalid`; ranges only for `Integer..Integer` (`int` loop, so `1..0` empty) | `QE/QvtOperationalEvaluationVisitorImpl.java:2151-2190` | D |
| `Dict` literal | §8.2.2.27 | `DictionaryImpl` (HashMap); parts with invalid key or value silently skipped; later duplicate key wins | `:209-219` | D-J/N |
| Other collection literals | OCL | classic OCL `visitCollectionLiteralExp` (§A) | | |
| Default value of a collection-typed var/property | spec: null | empty collection of the declared kind (`Collection(T)` declared → **null**, because `createNewCollection` returns null for COLLECTION_LITERAL kind) | `QE/EvaluationUtil.java:252-269,278-317` | D |
| Implicit List↔Sequence coercion | – | value passed where `List` expected and not a MutableList → copied into new MutableList; where `Sequence` expected and value is MutableList → copied into ArrayList (args, results, var init) | `QE/EvaluationUtil.java:321-335` | D |

### 4.2 QVTo overrides of OCL collection operations (`QS/*TypeOperations.java`)

QVTo re-registers several OCL collection operations with Java handlers (bug 467600, mostly for result typing). When the static type matches, these run *instead of* classic OCL:

| Op | Behaviour | Source |
|---|---|---|
| `Collection::asSet/asOrderedSet/asBag/asSequence` | `CollectionUtil.asSet/asOrderedSet/asBag/asSequence(source)` (§A: may return the *same* object if already of that kind); `asSequence` of a MutableList → new ArrayList copy | `QS/CollectionTypeOperations.java:88-129` |
| `Collection::asList` | MutableList → same object (aliasing!), else new MutableList copy in iteration order | `:131-139`, `QE/EvaluationUtil.java:271-276` |
| `Collection::flatten` | LinkedHashSet source first converted to **HashSet** (order lost), then `CollectionUtil.flatten`; List source → result re-wrapped as new List | `:141-159` |
| `includes/excludes/count` | invalid arg → invalid; else `CollectionUtil.includes/excludes/count` | `:161-198` |
| `=`/`<>` on collections | invalid arg → invalid; else `ObjectUtil.equal(source,arg)` | `:220-244` |
| `Set/Bag/Sequence/List::including` | invalid arg → invalid; **LinkedHashSet source → copied into HashSet first**; `CollectionUtil.including`; List source → re-wrapped List | `QS/SetTypeOperations.java:112-131` |
| `…::excluding` | same LinkedHashSet→HashSet conversion | `:133-151` |
| `Set/Bag/Sequence/List::union` | `CollectionUtil.union` (§A), List re-wrap | `:78-90` |
| `Set/Bag::intersection` | `CollectionUtil.intersection` | `:92-100` |
| `Set::-`, `symmetricDifference` | only if both are `java.util.Set` else invalid | `:102-110,153-161` |
| `OrderedSet/Sequence/List::append/prepend/insertAt/indexOf` | invalid arg → invalid; `indexOf` not found → invalid | `QS/OrderedSetTypeOperations.java:71-140` |
| `reverse` (QVT extension) | ArrayList copy reversed → List / OrderedSet (if LinkedHashSet) / Sequence | `:142-160` |
| `List::add`, `insertAt(obj, idx)`(void), `remove`, `removeAll`, `removeAt`, `removeFirst`, `removeLast` | **mutate in place**; invalid element ignored; `remove(x)` removes *all* occurrences; out-of-range → invalid; `insertAt(e, i)` → `ArrayList.add(i-1, e)` (IndexOutOfBounds → invalid) | `QS/ListOperations.java:117-222`, `QS/MutableListImpl.java:61-63` |
| `List::joinfields(sep,begin,end)` | `begin + String.valueOf(e1) + sep + … + end` (null → "null", EObject → identity toString **N**) | `QS/MutableListImpl.java:41-55` |
| `List::first/last/at` | empty / out of range → invalid | `QS/ListOperations.java:240-278` |
| `List::clone` | shallow `new MutableListImpl(source)` | `:280-288` |
| `List::selectByType/selectByKind` | new List, iteration order | `:290-324` |
| `List::subSequence` | `CollectionUtil.subSequence` (§A), re-wrapped | `:326-338` |

**Compat:** register these handlers so they shadow the OCL ops exactly where Eclipse does; reproduce the LinkedHashSet→HashSet conversions. **Fast:** keep insertion order, skip conversions.

### 4.3 Imperative iterators (QVT extensions)

| Construct | Spec | Eclipse | Source |
|---|---|---|---|
| `xcollect` / `->collect` shorthand `coll->f` | flatten body, drop nulls, Bag-or-Sequence | **no flatten**; null bodies dropped; `invalid` body ⇒ whole result invalid and loop stops; result `Bag` if source *static* type is Set/Bag, else `Sequence` (also for OrderedSet and List sources) | `QE/QvtOperationalEvaluationVisitorImpl.java:1561-1585`, `QE/iterators/QvtIterationTemplateXCollect.java:38-43`, `QE/iterators/QvtImperativeIteratorTemplate.java:85-100` |
| `xselect` / `coll[cond]` / `coll[Type]` | null elements removed; Bag/OS/Seq/Set kept | result is a **new collection of the same runtime kind** (HashSet stays HashSet, LinkedHashSet stays…); null elements skipped; condition: `Boolean` → used; `EClassifier` (typed shorthand) → `oclIsKindOf`; anything else (null/invalid/non-boolean) → loop stops and **whole result is invalid** | `:1559-1560`, `QE/iterators/QvtIterationTemplateXSelect.java:38-54`, `QvtImperativeIteratorTemplate.java:70-82` |
| `selectOne`, `collectOne`, `collectselect`, `collectselectOne` | §8.2.2.7 | `*One` variants stop at first accepted element and return it (not a collection); collectselect = collect body, drop null, filter by condition | `:1589-1601`, `QvtIterationTemplateCollectSelect.java` |
| undefined source of imperative iterate | – | `invalid` | `:1549-1551` |
| `forEach` / `forOne` | §8.2.2.6 | undefined source → **null**, no iteration; multiple iterators = cartesian product (nested Java iterators, first iterator varies fastest); condition must be `Boolean.TRUE`; `forOne` stops after first body execution; result value is always null unless a `return` happened | `:1512-1539`, `QE/iterators/QvtIterationTemplateForExp.java:40-60`, `QvtIterationTemplate.java:64-163` |
| `break` / `continue` / `return` in iterators | – | `break` ⇒ stop, keep partial result; `continue` ⇒ next element; `return` ⇒ propagate `OperationCallResult` out of the loop | `QvtIterationTemplate.java:82-121` |
| Iteration over a Set while the body mutates it | – | Java `ConcurrentModificationException` → swallowed → `invalid` | (Java semantics) |

Iteration order = Java iterator order of the runtime collection (see §3).

### 4.4 Model / element queries

| Op | Spec | Eclipse | Source | Det. |
|---|---|---|---|---|
| `Model::objects()` | Set(Element) | `LinkedHashSet` over `extent.getAllObjects()` = initial objects (in given order) each followed by its `eAllContents` (pre-order, proxies resolved), then created root objects + contents (after purge) | `QS/ModelOperations.java:75-84,173-188`, `QA/ModelParameterExtent.java:196-213` | D |
| `objectsOfType(T)` / `objectsOfKind(T)` | | same traversal, filtered by `isTypeOf`/`isKindOf` | `:101-123` | D |
| `rootObjects()` | | `LinkedHashSet` of `getRootObjects()` = initial objects whose container is unchanged + created objects with `eContainer()==null`, creation order | `:86-99`, `QA/ModelParameterExtent.java:174-194` | D |
| `Model::removeElement(e)` | | deletes `e` from its container and removes all references to its subtree from roots of the extent (`UsageCrossReferencer`) | `QA/ModelParameterExtent.java:242-253,322-359` | D |
| `Model::copy()` | | `EcoreUtil.copyAll` of root objects into new extent | `QS/ModelOperations.java:141-152`, `QA/ModelParameterExtent.java:57-61` | D |
| `Element::subobjects()` | Set | **`HashSet<EObject>`** of `eContents()` | `QS/ElementOperations.java:416-430` | **N** |
| `allSubobjects()` / `allSubobjectsOfType/Kind` | | **`HashSet<EObject>`** of `EcoreUtil.getAllContents(e,false)` (proxies *not* resolved) | `:399-414` | **N** |
| `subobjectsOfType/Kind` | | HashSet | `:329-349` | **N** |
| `Classifier::allInstances()` | §8.3.18.1: instances in all extents | `QVTExtentMap.get` → **HashSet** over all model-parameter extents + unbound extent (`getAllObjects`, incl. intermediate objects in the intermediate extent if it is a model parameter? – no: only model params + unbound) filtered by `isKindOf`; outside a transformation: `LazyExtentMap` from `self`'s resource | `QA/QVTExtentMap.java:53-96` | **N** |
| `OclAny::allInstances(T)` (deprecated QVT legacy op) | | mapped to `Element::allSubobjectsOfKind` (!) | `QS/OclAnyOperations.java:18-20` | N |
| `Model::allInstances(T)` (deprecated) | | = `objectsOfKind` | `QS/ModelOperations.java:67-69` | D |
| `container()` / `metaClassName()` | | `eContainer()` / `eClass().getName()`; non-EObject source → invalid | `:296-316` | D |

**Compat:** for the N cases use the identity-hash surrogate (§3). **Fast:** containment order (`eContents`/`eAllContents` order), extent order for allInstances.

### 4.5 Navigation of multi-valued features (classic OCL, but very visible)

`OE/EcoreEvaluationEnvironment.java:236-275,312-334`: `navigateProperty` returns `coerceValue(feature, eGet(feature), copy=true)`: the OCL collection kind comes from `ordered/unique` (`OE/internal/UMLReflectionImpl.java:629-655`):

| ordered | unique | OCL type | Java copy | Det. |
|---|---|---|---|---|
| true | true (EMF default for references) | OrderedSet | LinkedHashSet (EList order) | D |
| true | false | Sequence | ArrayList | D |
| false | true | **Set** | **HashSet** (identity order for objects) | **N** |
| false | false | Bag | BagImpl | **N** |

Single-valued feature whose value is a collection → first element or null. QVTo adds: navigating a *Tuple* looks up the part by name (multiple tuple-type instances), missing part → null; `IllegalArgumentException` from navigation → `QvtRuntimeException("Unknown property '<name>'")` (fatal, not invalid) (`QA/QvtOperationalEvaluationEnv.java:209-263`).

---------------------------------------------------------------------------------------------------

## A–G. Classic Eclipse OCL layer (what QVTo inherits)

Additional abbreviations for this part (all under `eclipse-ocl/plugins/`): **EVI** `O/EvaluationVisitorImpl.java`, **AEV** `O/AbstractEvaluationVisitor.java`, **AEE** `O/AbstractEvaluationEnvironment.java` (incl. `AbstractTuple`), **CU** `O/util/CollectionUtil.java`, **OU** `O/util/ObjectUtil.java`, **BAG** `O/util/BagImpl.java`, **NU** `O/internal/evaluation/NumberUtil.java`, **IT** `O/internal/evaluation/IterationTemplate*.java`, **EO** `O/options/EvaluationOptions.java`, **AOA** `O/parser/AbstractOCLAnalyzer.java`, **OSLU** `O/util/OCLStandardLibraryUtil.java`, **LEM** `O/LazyExtentMap.java`, **EEE** `OE/EcoreEvaluationEnvironment.java`, **OSLI** `OE/internal/OCLStandardLibraryImpl.java`, **UMLR** `OE/internal/UMLReflectionImpl.java`. Spec refs: OCL 2.4 § (printed page [file page marker]).

Remember: QVTo replaces some of these (§4.2 handlers, its own `visitExpression` catch-all, `if`, collection literals of List type, `allInstances`).

### A.0 Cross-cutting

**Evaluation options (defaults)**: `LAX_NULL_HANDLING=true` (EO:47-48), `DYNAMIC_DISPATCH=false` (EO:70-71), `ANY_LESS_IS_INVALID=false` (EO:88-89), `CLOSURE_INCLUDES_SOURCES=false` (EO:103-104); parsing option `USE_LONG_INTEGERS` false in OCL but **set true by QVTo** (`org.eclipse.m2m.qvt.oml/.../ast/env/QvtEnvironmentBase.java:131-137`).

**invalid**: singleton `OCLStandardLibraryImpl.INVALID`, an anonymous `EObjectImpl` whose `toString()` is `"invalid"` (OSLI:111-115, 154; AEV:175-177); identity equality/hash. `isUndefined(v) = v == null || v == INVALID` (AEV:311-314).

**Exception → invalid boundaries** (no precondition checks; Java exceptions are converted only at these points; `EvaluationHaltedException` always rethrown; `Error`s e.g. `StackOverflowError` never caught):
* `safeVisitExpression` (AEV:570-580): operation-call source (EVI:231), user/Java op arguments (EVI:251), `=`/`<>` argument (EVI:338,364), non-boolean binary arguments (EVI:784), variable/let/iterate-accumulator initialisers (EVI:2385).
* `visitExpression` (AEV:243-261, logs WARNING) — iterator bodies (IT:81), operation bodies (AEV:347-348), derived properties (AEV:420-421), top-level query. *(In QVTo, every `visitExpression` is the QVTo override with the error log, §6.)*
* explicit try/catch around `callOperation` (EVI:255-292) — `UnsupportedOperationException` there falls through to the built-in path; `AEE.callOperation` reflective invoke → invalid (AEE:205-217).
* **Not** boundaries: `if` parts, let body, property-call source, collection-literal items/ranges, tuple parts, boolean right operand (EVI:1157-1200), ternary args, type args, iterator source, sortedBy comparator. ⇒ `Tuple{a = seq->at(99)}` makes the *enclosing* expression invalid, `true and <throws>` makes the parent invalid.
* Compat: model built-ins as `Result<Value, JavaException>` and convert to `Invalid` exactly at those boundaries (plus QVTo's per-expression boundary). Fast: return `Invalid` at the failure point.

**Deterministic hashing** (complements §3): Tuple (`AbstractTuple`) hash = `37*identityHash(tupleType) + 17*parts.hashCode()` → **N** (AEE:458-465). **BagImpl hash = `37*(37*37 + coll.hashCode()) + size` where `coll` is a `HashMap<E, MutableInteger>` and `MutableInteger` does not override `hashCode` → N even for `Bag{1}`** (BAG:129-134,193-205): `Set{Bag{1}, Bag{1}}` has size 2. `IntegerRangeList` hashes like a List.

### A.1 Collection classes

| Kind | Java class | Source |
|---|---|---|
| Set | `HashSet` | CU:1162,1196 |
| OrderedSet | `LinkedHashSet` (kind test is `instanceof LinkedHashSet` everywhere: CU:296,562,625,771,1119,1229; OU:81-93) | CU:1166,1202 |
| Sequence | `ArrayList`, or lazy immutable `IntegerRangeList` for `Sequence{a..b}` | CU:1164,1198; EVI:2625-2714 |
| Bag | `BagImpl` = `HashMap<E,MutableInteger>` + size; iterates HashMap key order with each key repeated `count` times consecutively (`Bag{a,b,a}` → `a,a,b` or `b,a,a`); `toString` = HashMap's `{a=2, b=1}` | BAG:30-49,137-188,208-210 |

`BagImpl.EMPTY_BAG` is a shared *mutable* static returned by `emptyBag()` (intersection CU:367,382); `Collections.emptySet()` (immutable) returned by intersection CU:365,376 — mutating results of these (QVTo `+=` copies, so mostly safe) would be a bug source.

### A.2 Collection operations

"Kind-preserving copy" = `createNewCollection(kind, self)` (new instance → HashSet copy-constructor sizing, §3).

| Op | Eclipse | Source | Deviation |
|---|---|---|---|
| `union` | either side empty → copy of the other (Bag if either is Bag, else **same kind as the non-empty side**); else Bag if either Bag (counts add); else Sequence (concat) if either is a List; else **`new HashSet(self).addAll(arg)`** ⇒ `OrderedSet->union(OrderedSet)` is an unordered HashSet, but `OrderedSet{a}->union(OrderedSet{})` stays ordered | CU:421-453; null arg → invalid EVI:1343-1350 | inconsistent kinds |
| `intersection` | empty side → `emptySet()` if either Set else `EMPTY_BAG`; Set involved → HashSet else Bag; iterates the **smaller** collection (ties: self) adding `e` if other `contains(e)` ⇒ Bag multiplicity = that of the smaller bag, not `min(count)` (`Bag{a,a,b}∩Bag{a,b,c,d}` keeps 2×a) | CU:356-404 | bug |
| `-`, `symmetricDifference` | HashSet copy (OrderedSet order lost) | CU:541-545,593-607 | |
| `including` | kind-preserving copy + `add` (OrderedSet: existing element does not move; Bag count+1); `including(invalid)` → invalid; null added | CU:622-638; EVI:1290 | (QVTo overrides for Set/Bag/Seq/List, §4.2) |
| `excluding` | copy; Sequence removes all occurrences; Bag drops the key | CU:560-581 | |
| `flatten` | only the **first** element is inspected: not a collection / empty / null → `self` returned unchanged; else flatten one level and repeat; result kind Bag if Bag, **HashSet if any Set incl. OrderedSet**, else Sequence; mixed elements → CCE → invalid | CU:467-509 | order lost |
| `asSet` | HashSet (not LinkedHashSet) → **same instance**; else `new HashSet` | CU:652-657 | aliasing |
| `asBag` | Bag → same instance; else `new BagImpl(c)` | CU:671-676 | |
| `asSequence` | any `List` (incl. `IntegerRangeList`, `MutableList`) → same instance (QVTo handler copies MutableList); else `new ArrayList(c)` in iteration order | CU:690-695 | |
| `asOrderedSet` | LinkedHashSet → same instance; else `new LinkedHashSet(c)` | CU:709-715 | |
| `append` | OrderedSet: copy, `remove(o)`, `add(o)` → **existing element moves to end**; else new ArrayList + add | CU:768-780 | |
| `prepend` | new collection: `add(o)` then `addAll(self)` (OrderedSet: existing o ends up first) | CU:794-804 | |
| `insertAt(i,o)` | 1-based; `i<1 ∨ i>size+1` → IOOBE → invalid; OrderedSet with existing `o`: before `i` → no-op, after `i` → moves to `i`; null index → invalid | CU:821-852; EVI:1541 | |
| `at(i)` | index must be `java.lang.Integer` (a **Long index → invalid**); out of range → invalid; works on Sets/Bags at runtime (hash order) | CU:957-974; EVI:1397-1404 | |
| `first`/`last` | empty → invalid | EVI:642-654; CU:987-1014 | |
| `indexOf` | `OU.equal` linear scan (1 matches 1.0); not found → invalid | CU:1028-1039; EVI:1406-1414 | |
| `subSequence`/`subOrderedSet` | `lower<1`, `upper>size`, `upper<lower` → invalid; subOrderedSet on non-LinkedHashSet → Sequence | CU:867-941 | |
| `reverse` | **absent in classic OCL** (QVTo adds it, §4.2) | OSLU:2074-2230 | |
| `count` | `OU.equal` (numeric cross-type) | CU:83-91 | |
| `includes`/`excludes` | `Collection.contains` = **Java equals/hashCode** (`Set{1}->includes(1.0)` false; an uncoerced EMF `Long` not found in `Set{5}`) | CU:57-72 | inconsistent with `=` |
| `includesAll`/`excludesAll` | loop of `contains`; null arg → invalid | CU:103-130; EVI:1310-1326 | |
| `isEmpty`/`notEmpty` on `null` | parser wraps null in implicit set conversion, EVI skips a lone null item ⇒ `Set{}` ⇒ `null->isEmpty()` = true | EVI:2573-2579,2607-2622 | per spec |
| `sum` | empty: `0` (Integer elem type), `0.0` (Real), **null** otherwise; non-empty: iteration-order sum with promotion (Long/Double/BigInteger/BigDecimal), **long wraps**, then `coerceNumber`; non-number or null element → invalid | CU:238-272; EVI:603-620 | |
| `max`/`min` | empty → **null** (spec: invalid); once a Double is seen the accumulator is Double (`Sequence{3,1.0}->max() = 3.0`) | CU:154-216 | |
| `product` | HashSet of tuples (order **N**) | CU:729-754; EVI:1328-1342 | |
| `selectByKind`/`selectByType` | same kind as source; kind skips nulls | EVI:1416-1438 | |
| `=`/`<>` | `OU.equal`: null side → identity; OrderedSet vs non-OrderedSet → false; OrderedSet vs OrderedSet → ordered, recursive `OU.equal`; else Java `equals` (`Set{1} = Set{1.0}` false; `Sequence{1..3} = Sequence{1,2,3}` true; Set vs Bag false) | EVI:332-380; OU:41-118 | |

### A.3 Iterators (`IT` = `IterationTemplate.evaluate`, IT:62-110)

Empty source → initial accumulator; body via `visitExpression`; with several iterators **iterator #0 varies fastest** (IT:126-151); undefined source → invalid (EVI:1807-1809,1774-1776).

| Iterator | Eclipse | Source | Spec deviation |
|---|---|---|---|
| `select`/`reject` | result kind from the **static** source type (Set→HashSet, Bag→Bag, Sequence→ArrayList, else LinkedHashSet); null or invalid body → whole result invalid | EVI:1980-2068; IterationTemplateSelect:54-63 | |
| `collect` | Set/Bag source → Bag, else Sequence; invalid body → invalid; **null body added**; collection body → `addAll(CU.flatten(body))` (recursive) | EVI:1941-1978; IterationTemplateCollect:48-59; AOA:3626-3641 | |
| `collectNested` | same kinds, no flatten | EVI:1902-1939 | |
| `exists`/`forAll` | null or invalid body → **invalid immediately** (stops), even if a later element would decide; otherwise short-circuit | IterationTemplateExists:46-57; …ForAll:47-58 | spec: true/false dominates; **order-dependent** |
| `any` | first match in iteration order; null/invalid body → invalid; no match → **null** | IterationTemplateAny:50-60; EVI:2096-2121 | spec: invalid |
| `one` | null/invalid → invalid; second true → false and stop | IterationTemplateOne:44-70 | |
| `isUnique` | body values into a HashSet (Java equals: 1 ≠ 1.0); invalid treated as an ordinary value | IterationTemplateIsUnique:43-49 | spec: invalid |
| `sortedBy` | body evaluated **twice** per element (EVI:2145,2152); values in `HashMap<elem,Comparable>`; `Collections.sort` (stable TimSort) on `new ArrayList(source)` ⇒ ties keep source iteration order; comparator `compareTo` ⇒ Integer vs Double/Long → **CCE → invalid**, EObject body → invalid; Strings by UTF-16 `compareTo`; Booleans false<true; generated enums by ordinal; UnlimitedNatural body: `*` last (EVI:2178-2192); null/invalid body → invalid; result: Set/OrderedSet source → OrderedSet, Bag/Sequence → Sequence | EVI:2123-2204; IterationTemplateSortedBy:55-64; AOA:3648-3667 | spec: `<`, UCA |
| `closure` | result kind from the **body** type (OrderedSet if body type OrderedSet/Sequence, else HashSet); recursive DFS pre-order; sources included only if reached again (option off); no re-expansion; null body skipped; invalid → invalid; Java recursion (deep chains → uncaught StackOverflowError) | EVI:2232-2258; IterationTemplateClosure:57-134; AOA:3669-3690 | spec: sources included, kind from source |
| `iterate` | accumulator via `safeVisitExpression`; invalid accumulator does not stop; accumulator bound with `env.add` → name already bound → IllegalArgumentException | EVI:1760-1793; IT:169-172; AEE:117-126 | |

### A.4 Collection literals and ranges

* `Sequence{a..b}` alone (EVI:2534-2561): bounds cast to `Integer` (a Long bound → CCE propagates); **null bound → `Sequence{null}`**; `a > b` → empty; else lazy `IntegerRangeList`.
* Other literals (EVI:2564-2601): an invalid item → whole literal invalid; null items kept (except lone null in implicit set conversion); range part with null bound skipped; range values boxed `Integer`.
* QVTo `List{a..b}`: only `Integer..Integer` (`instanceof Integer`), otherwise the range part is silently skipped (`QE/QvtOperationalEvaluationVisitorImpl.java:2176-2184`).

### A.5 allInstances (classic OCL; QVTo replaces it inside transformations, §4.4)

`LazyExtentMap` (LEM:65-107): search root = resource contents of `EcoreUtil.getRootContainer(context)` only (or its containment tree); result **HashSet cached per class** (stale, same mutable instance each call); enum `allInstances` returns `EEnumLiteral`s while enum literal expressions evaluate to `Enumerator`s ⇒ `E.allInstances()->includes(E::a)` false; OclVoid → `Set{null}`; other datatypes → empty set.

### A.6 Navigation results — see §4.5 (`EEE:236-263,312-337`, `UMLR:628-650`); element values are **not** number-coerced.

### B. Numbers

**B.1 Representation**: Integer literal = `coerceNumber(getLongSymbol())` → `Integer` if it fits else `Long` (EVI:2445-2447); Real literal `Double.valueOf` (`O/parser/AbstractOCLParser.java:497-502`); binary arithmetic promotes both operands with `higherPrecisionNumber` (Integer→Long, Float/BigDecimal→Double; NU:170-200), branches only for Long/Long, Long/Double, Double/Long, Double/Double; a BigInteger beyond long only supports comparisons (others → invalid, EVI:1725); integer results `coerceNumber`ed back to Integer when they fit (NU:45-82). **Overflow wraps silently in 64 bits** (EVI:850-858). Spec §11.5 (unbounded) — deviation. Compat: `enum Num { Int(i32), Long(i64), Real(f64) }` + Java normalisation + wrapping ops; fast: i64 checked → BigInt.

**B.2 Operations**

| Op | Eclipse | Source |
|---|---|---|
| `Int / Int` | `(double)a/(double)b` → Real (`4/2 = 2.0`); `/0` → invalid | EVI:861-866 |
| Real `/` | zero denominator → invalid; overflow may still give ±Infinity/NaN | EVI:956-958,1043-1045,1106-1108 |
| `div` | `b==0` → invalid; Java truncation; `Long.MIN div -1` wraps | EVI:869-872 |
| `mod` | Java `%` (sign of dividend); `mod 0` → ArithmeticException → invalid | EVI:875-876 |
| `div`/`mod` on Real | no case → invalid | EVI:1134-1142 |
| unary `-` | `-Integer.MIN` → Long; `-*` → invalid | EVI:431-456 |
| `abs` | `Integer.MIN` → Long; `Long.MIN` stays negative; `*` → invalid | EVI:458-491 |
| `floor` | **`(int) Math.floor(d)`**: saturates (`1e10.floor() = 2147483647`), NaN → 0 | EVI:501-517 |
| `round` | **`(int) Math.round(d)`**: ties toward +∞ (`(-2.5).round() = -2`), long→int cast **wraps** (`3e9.round() = -1294967296`) | EVI:519-535 |
| `max`/`min` | mixed → `Math.max(long,double)` → Double (`1.max(2.0) = 2.0`) | EVI:878-884,960-966,1047-1053,1110-1116 |
| `< > <= >=` | numeric after promotion, Long vs Double as double | EVI:886-900,968-982,1055-1069,1118-1132 |
| `=`/`<>` | promotion then `OU.equal`: `1 = 1.0` true, `2^53+1 = 2^53.0` true, NaN ≠ NaN, `0.0 = -0.0` true | EVI:343-353; OU:49-72 |
| number op null | invalid | EVI:790-794 |
| `oclAsType(Integer)` on Real | **truncates** `intValue()` (`3.7 → 3`) | EVI:746-748 |
| `oclAsType(Real)`/`oclIsKindOf(Real)` on Integer | only for `java.lang.Integer` (→ Double); a `Long` → `oclIsKindOf(Real)` false, `oclAsType(Real)` invalid | EVI:752-765; EEE:424-426 |

**B.3 UnlimitedNatural**: `*` is the Integer **−1** (`UnlimitedNaturalLiteralExp.java:40`, EVI:112,2455-2458), recognised only via the static type `UnlimitedNatural`: `* = -1` is true; assigned to an Integer variable it is just −1; `toString()` gives `"*"` only with static type UnlimitedNatural, else `"-1"` (EVI:389-390). Comparisons: `* op *` → `<`,`>` false, `<=`,`>=` true, arithmetic invalid (EVI:818-831); `*` vs finite → `<`/`<=` give argUnlimited, `>`/`>=` sourceUnlimited (EVI:832-844); `*` vs Real: `<` false, `>`/`>=` true, **`<=` invalid**; Real vs `*`: `<` true, `>`/`>=` false, **`<=` invalid** (EVI:922-939,1006-1023). `toInteger()`/`oclAsType` of `*` → invalid; finite UnlimitedNatural arithmetic is plain Integer (`5-7 = -2`).

**B.4 Formatting / parsing**: `toString()` = Java `toString` (EVI:392-393): Double via **`Double.toString`** (`1.0`, `1.0E10` for |x| ≥ 1e7, `1.0E-4` for |x| < 1e-3, `NaN`, `Infinity`, `-0.0`); JDK ≤ 18 printed some values non-shortest (JDK-4511638, e.g. `2.0E23` → `1.9999999999999998E23`), JDK 19+ shortest. Compat: port Java's algorithm for a chosen JDK (default: JDK 17 behaviour if targeting current QVTo releases — confirm the reference JDK); fast: ryu + Java exponent formatting. `String::toInteger()` = `Integer.valueOf` (sign allowed, no whitespace, >int → invalid even with long literals) (EVI:566); `toReal()` = `Double.valueOf` (trims ≤U+0020, `NaN`, `Infinity`, `1e3`, `1.5f`, `0x1p3`) (EVI:575). QVTo `asInteger/asFloat` use the same Java parsers but return null on failure (§5).

### C. Strings (classic OCL)

All positions in UTF-16 code units. Spec §11.5.3 (p.160-161 [176-177]).

| Op | Eclipse | Source |
|---|---|---|
| `concat`, `+` | `String.concat`; null/invalid arg → invalid; **no implicit conversion** of non-strings (compile-time type error; no QVTo conversion found either) | EVI:1222-1230 |
| `size()` | UTF-16 length | EVI:551-554 |
| `substring(l,u)` | requires `1 ≤ l ≤ u ≤ size` else invalid (empty substring impossible) | EVI:1524-1532 |
| `at(i)` | Integer arg; out of range → invalid | EVI:1248-1252 |
| `characters()` | per UTF-16 unit (surrogates split), ArrayList | EVI:493-499 |
| `toUpper/toUpperCase/toLower/toLowerCase` | `String.toUpperCase()` with **JVM default Locale** (**L**; Turkish i; `ß`→`SS`) | EVI:577-585 |
| `< <= > >=` | `String.compareTo` — UTF-16 code-unit order, no UCA (Rust `str` order differs for U+E000.. vs supplementary chars → compare `encode_utf16()`) | EVI:1233-1246 |
| `indexOf`, `lastIndexOf` | `1 + Java index` ⇒ **0 = not found**; `indexOf('')` = 1 | EVI:1260-1264 |
| `startsWith`, `endsWith`, `equalsIgnoreCase` | Java | EVI:1254-1270 |
| `matches(re)` | Java `Pattern` full match (LRU cache) | EVI:1266-1267,2760-2800 |
| `replaceAll`, `replaceFirst` | Java regex + Java replacement syntax (`$1`, `\`) | EVI:1505-1509 |
| `substituteAll` / `substituteFirst` | literal replace; substituteFirst not found → invalid | EVI:1511-1522 |
| `tokenize(...)` | `StringTokenizer` (delimiter *set*, no empty tokens, default `" \t\n\r\f"`) | EVI:124-131,587-588,1272-1273,1534-1535 |
| `trim()` | Java trim (≤ U+0020 only) | EVI:590-591 |
| `toBoolean()` | `"true".equals(s)` | EVI:559-561 |

Spec QVT §8.3.16.9 `indexOf` (−1 if absent) is *not* what Eclipse does (0). Regex ops need a Java-compatible regex engine (backreferences, lookaround, possessive quantifiers) — e.g. `fancy-regex` plus a Java-syntax translation layer, or a port.

### D. null / invalid (classic OCL)

**D.1 Propagation**: OCL-bodied/Java ops: undefined source → invalid (EVI:259-262), args unchecked; `=`/`<>`: invalid either side → invalid, `null = null` true, `null = x` false (EVI:332-380); unary built-ins: undefined source → invalid except `oclIsUndefined`/`oclIsInvalid` (EVI:422-426) ⇒ **`not null` → invalid**; binary built-ins with undefined source → invalid except `oclIsTypeOf/KindOf/AsType`, `and/or/xor/implies` under lax handling (EVI:677-694,1606-1619); invalid argument → invalid (EVI:785-787,1290); null arg to arithmetic/string ops → invalid (EVI:790-794,1223-1225); collection ops accept null elements; ternary: undefined source / invalid arg → invalid (EVI:1476-1494); **navigation over null/invalid → invalid** (EVI:2307-2310,2360-2362); `if` undefined condition → invalid (EVI:2400-2409; QVTo override identical); `toString()`: `"null"` / `"invalid"` (EVI:382-388); `oclAsSet()`: null → `Set{}` (EVI:398-407).

**D.2 Boolean operators** (EVI:1144-1218, lax default; right operand evaluated *without* catch boundary). `n` = null, `⊥` = invalid:

| Expression | Eclipse | OCL 2.4 §11.5.4 |
|---|---|---|
| `false and X` / `X and false` | false (left-false short-circuits) | false |
| `true and n`, `n and true`, `n and n` | **⊥** | null |
| `true and ⊥`, `⊥ and true` | ⊥ | ⊥ |
| `true or X` / `X or true` | true | true |
| `false or n`, `n or false` | **⊥** | null |
| `false implies X` | true | true |
| `X implies true` | true (also for X = ⊥/n) | true |
| `true implies false` | false | false |
| `n implies false` | **⊥** | null |
| `xor` with undefined | ⊥ (no short-circuit) | ⊥/null |
| `not n` | **⊥** | null |

Strict mode (not default): any undefined left operand → ⊥.

**D.3 Type tests** (AEV:433-499, EVI:697-779), lax: `null.oclIsTypeOf(T)` = (T is OclVoid); `null.oclIsKindOf(T)` = (T ≠ OclInvalid); `invalid.oclIsTypeOf(T)` = (T is OclInvalid); **`invalid.oclIsKindOf(T)` = true**; `null.oclAsType(T)` = null; `invalid.oclAsType(T)` = invalid; failed `oclAsType` → invalid (spec agrees). QVTo's typed xselect uses its own `oclIsKindOf` (`QvtImperativeIteratorTemplate.java:106-149`): undefined values → exact type test (null only matches OclVoid).

**D.4**: null allowed in collections; invalid cannot enter via literals/`including`/`collect` (but can via `isUnique`'s internal set); variable initialiser exception binds invalid (EVI:2381-2388); `MessageExp`, `UnspecifiedValueExp` → `UnsupportedOperationException` (EVI:2426-2437); `oclIsNew`, `oclIsInState`, `@pre` not evaluable (`oclIsNew` → invalid EVI:1722).

### E. Tuples

`AbstractTuple` via `EEE.createTuple` (EEE:389-402; AEE:393-501): parts in `HashMap<String,Object>`; `getValue` `coerceNumber`s numbers (AEE:427-433); equals = `type.equals(other.type) && parts.equals(other.parts)` — TupleType is an EObject (identity) but deduplicated by `findTupleType` on part names + same types regardless of order (`O/AbstractTypeResolver.java:415-445`) ⇒ `Tuple{a=1,b='x'} = Tuple{b='x',a=1}` true, `Tuple{a=1} = Tuple{a=1.0}` false (different types); hash `37*identityHash(type)+17*parts.hashCode()` cached (stale if a part collection mutates) (AEE:458-465, **N**); field order = `oclProperties()` order of the first tuple type created with those parts (`OE/internal/OCLFactoryImpl.java:141-160`); `toString` = `Tuple{a = 1, b = 'x'}` (strings single-quoted, collections via `CU.toString` → `Set{1, 'a'}` style) (AEE:467-500). QVTo multi-result tuples are built with `evalEnv.createTuple` (`QE/…Impl.java:2321-2349`); tuple navigation by part *name* (§4.5).

### F. toString of values

OCL `toString()` / QVTo `repr()` / log data / `joinfields` all end in Java `toString`: Integer/Long decimal; Double `Double.toString`; Boolean; String unquoted; HashSet/LinkedHashSet/ArrayList/MutableList `[a, b]` (elements' own toString, strings unquoted); Bag `{a=2, b=1}`; Dict `Dict{k=v}`; Tuple `Tuple{a = 1, b = 'x'}`; generated enum → literal; `invalid` → `invalid` (but QVTo `log` prints `<Invalid>`); EObject → EMF `BasicEObjectImpl.toString`: `getClass().getName() + "@" + Integer.toHexString(hashCode())`, + ` (eProxyURI: …)` for proxies, generated classes append ` (attr: value, …)`, `DynamicEObjectImpl` appends ` (eClass: <EClass toString>)` — **N** (EMF source not in the reference tree; from EMF knowledge). Compat: same format with a surrogate hash; fast: readable stable repr.

### G. Classic OCL vs OCL 2.4 — deviation list

1. null in boolean logic → invalid (incl. `not`); exists/forAll stop at the first null/invalid (order-dependent).
2. Lax null handling: `invalid.oclIsKindOf(T) = true`, `null.oclIsKindOf/TypeOf` return Booleans.
3. `any` no match → null; empty `max/min` → null; empty UnlimitedNatural `sum` → null.
4. `closure` excludes sources; kind from body type.
5. Missing: `reverse`, `oclType()`, `oclLocale`; no evaluation of `oclIsNew`/`oclIsInState`/`@pre`/messages; `toString()` only on OclAny.
6. Extras: `lastIndexOf`, `matches`, `replaceAll/First`, `substituteAll/First`, `tokenize`, `trim`, `startsWith`, `endsWith`, `selectByKind/Type` (OSLU:1711-1795).
7. UnlimitedNatural = Integer −1 by static type; Real vs `*` with `<=` → invalid.
8. Integers 32/64-bit wrapping; `floor/round` cast to int; `oclAsType(Integer)` truncates Reals.
9. Membership/sets use Java equals (1 ≠ 1.0) while `=`/`count`/`indexOf` promote numbers.
10. Collection bugs: Bag intersection multiplicity; OrderedSet `union/flatten/-/symmetricDifference` → HashSet; `flatten` looks only at the first element; `sortedBy` CCE on mixed numbers and double body evaluation; `isUnique` treats invalid as a value.
11. Strings: UTF-16 code-unit comparison, default-locale case mapping.
12. `let` = plain env binding; name clash in the same env throws.
13. `iterate` continues after the accumulator becomes invalid.
14. `allInstances` (standalone OCL): one resource, cached/stale, unordered; enums return EEnumLiterals.
15. Irreproducible ordering: any Set/Bag of EObjects, enums, tuples, Bags; `product`; `allInstances` — and everything derived (`any`, `asSequence`, `sortedBy` ties, `first/at` on sets, float `sum` of a Real Set, `exists/forAll` vs invalid). Test oracles must compare those order-insensitively.

---------------------------------------------------------------------------------------------------

## 5. QVTo string library (§8.3.16) as implemented (`QS/StringOperations.java`)

General: all handlers are reached through `QvtOperationalEvaluationEnv.callOperation`, which returns `invalid` when the **source** is null or invalid (`QA/QvtOperationalEvaluationEnv.java:191-205`). Arguments are **not** checked: every handler's guard `if(leftVal == null && leftVal == invalid)` is always false (bug), so a null argument usually causes an NPE/ClassCastException which `doVisitOperationCallExp` turns into `invalid` silently (`QE/QvtOperationalEvaluationVisitorImpl.java:750-762`); an `invalid` argument hits a `ClassCastException` → `invalid`.

| Op | Spec | Eclipse | Line | Notes for compat |
|---|---|---|---|---|
| `format(value)` | %s/%d/%f, tuples, dict | **UNSUPPORTED** — compile error | 66 | reject at compile time |
| `length()` | = size(), deprecated | `String.length()` (UTF-16 units) | 95 | UTF-16 length |
| `substringBefore(m)` | | `indexOf(m)` <0 → **null**; else prefix | 103-120 | spec silent; null on no match |
| `substringAfter(m)` | | no match → **null** | 122-139 | |
| `firstToUpper()` / `lastToUpper()` | | `Character.toUpperCase(char)` on first/last UTF-16 unit; "" unchanged | 141-168 | per-UTF-16-unit, not locale aware, no surrogate handling |
| `normalizeSpace()` | trim + runs of whitespace → single space | `String.trim()` (removes chars ≤ U+0020) then keeps the **first** char of each `Character.isWhitespace` run (a tab stays a tab) | 170-194 | deviation |
| `replace(m1,m2)` | all occurrences | `String.replace(CharSequence,CharSequence)` (literal) | 196-213 | |
| `match(regex)` | Java regex | `Pattern.matches(regex, self)` – whole-string match | 215-227 | need Java-regex-compatible engine (lookbehind, possessive…) |
| `find(m)` / `rfind(m)` | "position" | `indexOf(m)+1` / `lastIndexOf(m)+1` → **1-based, 0 if absent** | 229-255 | |
| `isQuoted(s)` | starts & ends with s | `len >= 2*len(s) && startsWith && endsWith` | 257-269 | |
| `quotify(s)` | | `s + self + s` | 271-283 | |
| `unquotify(s)` | | if `isQuoted` strip both ends, else self | 285-300 | |
| `matchBoolean(b)` | `matchBoolean()` no arg: is "true/false/0/1" | **takes an argument**: `asBoolean(self) equals b` | 302-315 | signature deviation |
| `matchInteger(i)` | no arg | `Integer.valueOf(self) equals i` | 317-330 | |
| `matchFloat(r)` | no arg | `Double.valueOf(self) == r.doubleValue()` | 332-348 | `matchReal` absent |
| `matchIdentifier(s)` | alphanumeric word | ignores argument; true iff first char `isLetter` and all `isLetterOrDigit` (`_` → false; "" → true) | 350-370 | |
| `asBoolean()` | | `"1"` or equalsIgnoreCase `"true"` → true; `"0"`/`"false"` → false; else **null** | 372-385 | |
| `asInteger()` | | `Integer.valueOf(self)` (32-bit, accepts leading `+`/`-`, no whitespace) else null | 387-400 | |
| `asFloat()` | | `Double.valueOf(self)` (accepts `NaN`, `Infinity`, `1e3`, `0x1p3`, trailing `d`/`f`, surrounding whitespace!) else null | 402-415 | port Java `FloatingDecimal` grammar; `asReal` absent |
| `startStrCounter(s)`, `getStrCounter(s)`, `incrStrCounter(s)`, `restartAllStrCounter()` | | *static* operations, called as `startStrCounter('x')`; counters in a `HashMap` stored in the **execution context session data** (shared by nested transformations); `getStrCounter` of unknown → null; incr creates at 0 and returns 1 | 418-468, 506-557 | |
| `addSuffixNumber()` | | unknown counter: start it, return `self` (no suffix); else increment and return `self + <value before increment>` → `a`, `a1`, `a2`… | 470-487 | |
| `toUpper/toLower/size/substring/indexOf/startsWith/endsWith/trim/equalsIgnoreCase/+/concat/<` | §8.3.16 lists some | come from classic OCL (§A) | | |

Spec §8.3.16.9 `indexOf` returns −1 if not found; classic OCL's `indexOf` follows OCL 2.4 (see §A).

---------------------------------------------------------------------------------------------------

## 6. null / invalid in the QVTo layer

(OCL-level truth tables, navigation over null, `oclIsUndefined` etc. are in §A.)

| Situation | Spec (QVT 1.3) | Eclipse | Source |
|---|---|---|---|
| Representation | – | null = Java null; invalid = `getInvalid()` singleton; tests use `==` | `QP/QvtOperationalUtil.java:70-82` |
| Using invalid in an imperative expression | §8.1.20: assertion failure | silently propagates / ignored | throughout |
| Any Java `RuntimeException` during an expression | – | logged via `QvtPlugin.error` (`ERROR 0: Evaluation failed at '<file>:<line>' with a reason: <msg|ExceptionSimpleName>. Expression yields an 'invalid' value.` + Java stack trace on stderr when standalone) and **invalid** returned | `QE/QvtOperationalEvaluationVisitorImpl.java:415-432`, `QO/QvtPlugin.java:175-212`, `QE/messages.properties` |
| RuntimeException inside an OCL operation call | – | invalid, **no log** | `:750-762` |
| RuntimeException inside an imperative operation | – | wrapped: `QvtRuntimeException("Unexpected runtime exception caught during execution", e)` → fatal | `:2011-2016` |
| Imperative op called on invalid source | – | returns invalid (args already evaluated) | `:693-702` |
| Contextual imperative op on null source | – | invalid unless context type is `OclVoid` | `:719-723` |
| `if` condition null/invalid | – | **invalid** (then/else not evaluated); no `else` → null | `:322-340` |
| `switch`/`alt` | – | alternatives evaluated in order; first alt with undefined condition → whole switch **invalid**; first TRUE → its body value; no match → `else` or null | `:1323-1352` |
| `while` condition non-TRUE (incl. null/invalid) | | loop ends | `:1288-1316` |
| `when` / `where` / model-type `where` / `log ... when` / `assert` | | `Boolean.TRUE.equals(v)` required | `:577-596,1392,1456,2640` |
| Assign invalid to var | stores invalid | var := invalid (single) ; collection-typed var: `assign` skips collection logic for invalid → var := invalid | `QA/QvtOperationalEvaluationEnv.java:684-742` |
| Assign invalid/null to property | omit nulls | single-valued: `:=` → `eUnset`; `+=` → no-op. many-valued: `:=` → `clear()`; `+=` → no-op | `:745-850` |
| Dict/List literal with invalid | | Dict: pair skipped; List: whole literal invalid | `:209-219,2160-2162` |
| `List::add(invalid)`, `Dict::put(k, invalid)` | | ignored | `QS/ListOperations.java:124`, `QS/DictionaryOperations.java:92-96` |
| Navigation over null | §8.1.20: invalid | invalid (OCL) — no log, no exception | §A |
| Navigation of unknown property (IllegalArgumentException) | | fatal `QvtRuntimeException("Unknown property '<p>'")` | `QA/QvtOperationalEvaluationEnv.java:255-262` |
| Variable lookup | | flat `HashMap` per operation frame, **no lookup in caller frames**; unknown name → null | `:279-286` |
| `add` of existing name | | `IllegalArgumentException("The name: (x)  already has a binding: (v)")` → invalid via visitExpression | `:369-385` |

**Compat:** implement `invalid` as a distinguished value; implement the "catch-all → invalid + error log" boundary exactly at expression granularity (every `visitExpression` call), and the "silent invalid" boundary at OCL operation calls. **Fast:** same semantics but optional strict mode that raises on invalid use (spec).

---------------------------------------------------------------------------------------------------

## 7. Mappings, tracing, resolve

### 7.1 Data structures

* `Trace` (EMF model, `QT/TraceImpl.java`): `traceRecords` (EList, insertion order), `traceRecordMap: EMap<MappingOperation, EList<TraceRecord>>`, `sourceToTraceRecordMap: EMap<Object, EList<TraceRecord>>`, `targetToTraceRecordMap: EMap<Object, EList<TraceRecord>>` (EMF `EMap` = entry *list* + hash index → iteration in first-insertion order, **D**), plus non-EMF cache `mapping2Records: HashMap<MappingOperation, HashMap<Object key, TraceRecord>>` (`QT/TraceImpl.java:96-98,303-325`; lookup only).
* `TraceRecord` = mapping op, context `VarParameterValue` (`self`), parameters (all, incl. out), single result value (the result var, or `result` tuple when >1 result params) (`QE/TraceUtil.java:179-251`). Values are stored as given; IN `List`/`Dict` params are shallow-cloned (`:446-485`).
* "source" key of a record = context object if contextual, else the **first** parameter that is IN/INOUT and whose type is not a predefined OCL type (`:197-220`). "target" key = result value (tuple when multiple results).
* Nested transformations (`new T(...)`) share the **same** Trace object (bug 392153) (`QE/QvtOperationalEvaluationVisitorImpl.java:961-963`).

### 7.2 Mapping call protocol (`QE/QvtOperationalEvaluationVisitorImpl.java`)

1. `doVisitOperationCallExp` (688-763): evaluate source, apply List coercion, evaluate args **left-to-right** (`makeArgs` 2351-2362); invalid source → invalid; virtual dispatch on runtime type of the source for contextual ops (`findOperationByActualSourceType` 777-791 → `QA/VirtualTable.java` lookup: exact owner, then candidates owned by the actual type, then **depth-first over `getESuperTypes()` in declaration order**); `isVirtual` calls also consult module overriding (`extends`).
2. `executeImperativeOperation` (1961-2030): stack depth check (≥1000 → `QvtStackOverFlowError`), new frame, `this` bound to owning module instance.
3. `visitMappingOperation` (803-821): bind non-out params + `self`; **when** clauses in order, each must be `TRUE` → else `PRECOND_FAILED` (result null; `xmap` → `QvtAssertionFailed("Precondition failed in mapping '<name>'")` 736-741).
4. `executeMapping` (823-872): trace lookup `TraceUtil.getTraceRecord` (key `[self, non-out args…]` or `self` alone if no params; fallback linear scan comparing context/params with `isOclEqual` (identity, then both-undefined, then `equals`) and result) → hit: return stored result, out-args refreshed from record (`FETCHED_FROM_TRACE`). Then incremental-update trace (optional). Then **disjuncts** (2032-2074): candidates in declared order, skipping those whose context type does not match `isKindOf(self)` or whose arg count/types do not match (null args match anything); first non-failed result wins, a trace record for the *disjuncting* mapping is added; none → `NO_DISJUNCT_SELECTED` (null; `xmap` → precondition failure). Otherwise body.
5. `visitMappingBody` (612-658):
   1. `setupInitialResultValues`: result vars that are null get `createInitialValue` (String "", Integer 0, collections empty, objects stay null).
   2. **init** section expressions.
   3. `createOrGetResult` (1839-1862): result params still undefined and not Void → `createInstance` (into the param's `@extent` or inferred extent); >1 result → tuple `result` (creating missing objects) (2321-2349).
   4. `createOutParams` (1864-1873): undefined `out` params instantiated.
   5. **`TraceUtil.addTraceRecord`** — the mapping now "holds"; recursive calls with the same key during population/end return the partially populated result.
   6. **inherited** mappings, in `inherits` order, each executed as a "reusing" call (they see and write back the inheritor's result/out params; their own when-guard and trace logic applies; they add their own trace records).
   7. **population** (`visitOperationBody`); if the body has no explicit `population` keyword and the mapping has exactly one result (or is an `inout` contextual mapping with no result), the analyzer wraps all statements in an implicit `object result {…}` / `object self {…}` (`QP/QvtOperationalVisitorCS.java:4600-4636`). A `return` inside population stops population only; **end and merges still run, the mapping result is unaffected**.
   8. **end** section.
   9. **merged** mappings in `merges` order (reusing call semantics).
6. After body: each result object that has no container and no resource is put into its extent (858-869).
7. **where** clauses (`BlockExp` body) — each must be TRUE else `QvtAssertionFailed("Postcondition failed in mapping '<name>'")`; evaluated **even when the result came from the trace**.

Spec differences: spec §8.1.14 implicit disjunction of all same-name overloads with guard fallback is **not** implemented — only runtime-type virtual dispatch; spec says inherited mappings run "after its own initialization section" (Eclipse: after init + instantiation + trace record, matches), merged after end (matches).

### 7.3 resolve family (`QL/QvtResolveUtil.java`)

| Feature | Eclipse behaviour | Lines |
|---|---|---|
| `resolve(...)` with source | records from `sourceToTraceRecordMap.get(source)` (if declared source type is a collection and value is a collection: concatenation per element in collection iteration order); source evaluating to null → records whose source key was null | 312-341 |
| `resolve()` without source | all records of all keys, in EMap entry order | 314-321 |
| `invresolve` | uses `targetToTraceRecordMap`, yields the record's **context** value; records of non-contextual mappings are skipped | 342-379, 381-383 |
| `resolveIn(M)` without source | `traceRecordMap.get(M')` where M' = overriding mapping of M (if any), insertion order | 250-268 |
| `resolveIn(M)` with source | source records filtered by `runtimeMappingOperation.equals(M')` | 269-290 |
| type filter `resolve(T)` / `resolve(x : T | cond)` | `T.isInstance(candidate)` (EClass instance check; tuples never match); condition evaluated with the target variable `add`ed to the current frame (name clash → exception → invalid) ; must be TRUE | 385-401 |
| result | `resolveone`: first match (record order, then result order) or **null**; else `Sequence` (ArrayList) possibly empty | 342-379, 403-408 |
| deferred (`late`) | see 7.4 | |

### 7.4 Late resolve / deferred assignments

* Detection: `AssignExp` whose *first* RHS value is a `ResolveExp.isDeferred`, or `late resolve(...)->asSet()/asBag()/asSequence()/asOrderedSet()` with collection-typed resolve (`QL/QvtResolveUtil.java:84-160`).
* At assignment time (`QE/QvtOperationalEvaluationVisitorImpl.java:343-374`): the LHS owner expression is evaluated now and stored as "last assignment lvalue" (only for property LHS); the resolve source is evaluated now (`QL/AbstractLateResolveTask.java:35-46`); a task is appended to the root's deferred list; the assignment itself returns null and does nothing.
* Tasks run in creation order right after `main` returns (`runMainEntry` 2624-2630 → `RootInternal.processDeferredTasks`, `QA/QvtOperationalEvaluationEnv.java:1087-1102`), each in a cloned root environment; the resolve is evaluated against the final trace, coerced (`asSet`… → `CollectionUtil.asSet` etc.), then `callSetter(owner, feature, value, undefined?, isReset)` (`AbstractLateResolveTask.java:48-71`).
* During deferred execution, nested late resolves execute immediately.
* Late resolve not directly assigned to a property (e.g. to a variable, used in an expression) → evaluates to **null** now, the task later computes and discards the value. If the LHS is a variable, the stored "last lvalue" may be stale (from an earlier property assignment) but `getDeferredAssignmentFor` returns null so nothing is written.
* `+=` with late resolve appends at the end of the transformation, in task order.

### 7.5 Other mapping-related details

* `map` on a collection (`coll->map m()`) → implicit `xcollect` (null results dropped, no flatten) (`QP/QvtOperationalVisitorCS.java:1341-1352`).
* Entry operation `main`: `module.getEntry()` or first operation named `main` (`QP/QvtOperationalParserUtil.java:541-556`). Legacy `main(in x : MM)` signatures: each non-out param receives the **first initial object** of the corresponding model extent; empty extent → argument missing → `IllegalArgumentException("arguments mismatch…")` (`QE/QvtOperationalEvaluationVisitorImpl.java:2402-2450`).
* If no out/inout model params exist and `main` returns an EObject, it is placed into a synthetic extent ("compatibility") (1113-1123).
* Blackbox mapping returning null/invalid → `QvtRuntimeException("Blackbox operation '<q>' failed to assign mapping result")` (841-853).

---------------------------------------------------------------------------------------------------

## 8. Object creation, extents, assignment

### 8.1 Instantiation

| Item | Eclipse | Source |
|---|---|---|
| `object x : T {…}` | if variable `x` is bound to non-null: reuse it (type check failure → `RuntimeException("Cannot cast …")` → invalid); else `createInstance(T, @extent)` and bind; evaluate body expressions in order; re-read `x` afterwards (body may reassign); implicit temp var removed | `QE/QvtOperationalEvaluationVisitorImpl.java:1137-1158,2293-2311` |
| `new T(args)` (class) | `createInstance` then constructor (overridden constructor if module overrides it); no constructor + args on non-datatype → `QvtRuntimeException("Undefined constructor is called")`; args to collection type → `addAll` | `:932-1024` |
| `new T('lit')` for EDataType | `EFactory.createFromString`; failure → `QvtRuntimeException("Failed to instantiate datatype '<q>'")` | `:2553-2570` |
| abstract/interface/collection-without-kind | `QvtRuntimeException("Cannot instantiate type <q>")` | `QA/QvtOperationalEvaluationEnv.java:565-579`, `QE/…Impl.java:2540-2547` |
| new object placement | `ModelParameterExtent.addObject`: appended to `myAdditionalEObjects`; if it has no resource it is added to the extent's in-memory `ExtentResource` (URI `extent:/<n>`, non-unique contents list) | `QA/ModelParameterExtent.java:155-168,441-474` |
| default extent | intermediate class → intermediate extent; user model class → first non-`in` model param whose model type's **first** metamodel == root package of the class; else **first non-`in` model param whatever its metamodel**; else unbound extent (orphans; reported in `QvtEvaluationResult` but not written) | `QA/QvtOperationalEvaluationEnv.java:615-636`, `QA/QvtOperationalModuleEnv.java:167-199` |
| `in` extent write | only checked if `FLAG_READONLY_GUARD_ENABLED` session flag is TRUE (default **off**) → `QvtRuntimeException("Input model parameter '<p : T>' is read-only!")` | `QA/QvtOperationalEvaluationEnv.java:606-610,852-890` |
| intermediate class instance features | each feature: init expression or, if none / it yields null, the **type default** (`""`, 0, false, empty coll.); read-only features temporarily made changeable | `QP/IntermediateClassFactory.java:228-275` |
| contextual intermediate property `intermediate property C::p : T = e;` | stored per (module instance, property) in an `IdentityHashMap<owner, shadow EObject>`; first access creates shadow with type default; **init expression ignored** (warning at compile time) | `QE/IntermediatePropertyModelAdapter.java`, `QP/QvtOperationalVisitorCS.java:4835-4839` |
| `clone()` | `EcoreUtil.Copier` without containment (containment features empty), non-containment refs keep originals, intermediate contextual properties copied; result added to default extent of its class | `QS/ElementOperations.java:134-163` |
| `deepclone()` | full `EcoreUtil.Copier` (contained objects copied, internal refs redirected, external refs to originals; ID attributes copied by value; XMI ids not copied); only the top copy added to extent; intermediate properties cloned (collections/tuples/dicts deep-copied, objects mapped) | `:106-132,165-294` |
| `List::deepclone`, `Collection::clone/deepclone` (§8.3.11) | unsupported / not defined | `QS/ListOperations.java:83-85` |

### 8.2 Root objects of output extents

* `ModelParameterExtent.getContents()` (`QA/ModelParameterExtent.java:215-240`): `LinkedHashSet(initialObjects) ∪ additional objects (purged) ∪ objects added to the resource by others`, i.e. inout initial roots first (original order, even if they were re-contained? – purged only if container changed), then created roots in creation order.
* **Purge quirk** (`:162-166,290-320`): every `max(150, size-after-last-purge)` `addObject` calls, and at every `getAllObjects/getContents/removeElement`, contained objects are removed from `myAdditionalEObjects` permanently. An object that is contained at purge time and later detached will no longer be a root of the extent (it becomes an orphan without extent). Compat: replicate the counter exactly; fast: compute roots lazily from containment.
* Main executor: out/inout extents are written back to the caller's `ModelExtent`s in model-parameter order (`QO/InternalTransformationExecutor.java:197-224`).

### 8.3 Assignment (`QA/QvtOperationalEvaluationEnv.java:648-850`)

`assign(lhsType, old, rhs, isReset)`:

* Non-collection LHS type: result = rhs as is (for `+=` too!).
* Collection LHS type, rhs not invalid:
  * `:=` (reset): rhs is a `List`/`Dict` → **alias** the same object; otherwise new collection of the LHS *declared kind* (Set→HashSet, OrderedSet→LinkedHashSet, Bag→BagImpl, Sequence→ArrayList, List→MutableList, Dict→DictionaryImpl; `Collection(T)` → same kind as rhs or HashSet) filled with rhs elements (one level; a non-collection rhs becomes a singleton; null rhs → empty collection), numbers converted to the element instance class (`NumberConversions.convertNumber`).
  * `+=`: old collection (or new empty one if old is not a collection) is **copied** unless it is a List/Dict (then mutated in place → visible through aliases); if rhs is null or `isKindOf(rhs, elementType)` it is added as one element, else if rhs is a collection all its elements are added.
* Then `doImplicitListCoercion`.

`callSetter(owner, feature, value, undefined, isReset)`:

* Owner `invalid` → no-op; module instances → owning `this` instance; contextual intermediate properties → shadow object.
* `currentValue = owner.eGet(feature)`; `newValue = assign(oclType(feature), currentValue, value, isReset)` — for `:=` this creates a collection of the feature's OCL kind (**unordered unique feature → HashSet → N order**); for `+=` the EList is copied into an **ArrayList** (EList isn't a Set) and appended (order kept).
* Many-valued: undefined value → `clear()` if reset, else nothing. Otherwise nulls removed if the EList cannot contain null (EObject lists), then `ECollections.setEList(delegatingList, newList)` (in-place minimal moves/adds/removes; numbers converted); on any RuntimeException (e.g. uniqueness violations) logs a warning `Failed to merge new content to multivalued feature '<f>'. Whole re-assignment is used instead.` and does `clear(); addAll(newList)`.
* Single-valued: invalid or (null and primitive instance class) → `eUnset` if reset, else nothing; a collection value for a feature with upper bound `-2` (unspecified) → first non-null element; otherwise `eSet(convertNumber(value))` — a collection into a normal single-valued feature → `ClassCastException` → assignment expression yields invalid + error log.
* EMF itself enforces containment moves (assigning a contained object to a new container removes it from the old one and from the extent resource) and opposite updates; unique ELists reject duplicates.

Variable assignment: `visitAssignExp` (`QE/…Impl.java:376-387`) uses the same `assign` with the variable's declared type; result value of an `AssignExp` = rhs value (not the new LHS value); multiple RHS values (`x := a, b`) → only the **last** is used.

Spec §8.2.2.11 differences: no recursive flatten; `defaultValue` not implemented in evaluator; `+=` on single-valued is not invalid; null rhs to collection var yields empty collection.

---------------------------------------------------------------------------------------------------

## 9. Errors, logging, assertions, control flow

### 9.1 Exceptions

| Kind | Java class | Catchable by `try/except` | Result |
|---|---|---|---|
| `raise T('msg')` | `QvtException(msg, T)` with QVT stack trace | yes (catch by `EmfUtil.isAssignableFrom(catchType, T)`; `except()` without types catches all QVT exceptions); clauses checked in order | `QE/…Impl.java:1659-1668,1670-1696` |
| fatal assert, `xmap` precondition, `where` failure, model-type `where` failure | `QvtAssertionFailed` (QvtException with type `AssertionFailed`) | yes | |
| Stack overflow (depth ≥1000 or Java SOE) | `QvtStackOverFlowError` | no | |
| Unknown property, read-only extent, blackbox missing/ambiguous, bad config property, undefined constructor, unexpected RuntimeException in operation, … | `QvtRuntimeException` | no | |
| User interrupt | `QvtInterruptedExecutionException` | no | |
| NPE/CCE/etc. in expressions | swallowed → invalid (§6) | – | |

* **`raise 'msg'` (string form, `ImperativeOCL.gi:251-257`) produces a RaiseExp with *no exception type and no argument*** (the analyzer only processes the argument when a type is given, `QP/QvtOperationalVisitorCS.java:882-911`): at runtime `QvtException(null, null)`; only a catch-all `except()` clause reliably catches it (typed clauses call `EmfUtil.isAssignableFrom(T, null)`); uncaught it reports `QVT exception caught : 'org.eclipse.m2m.internal.qvt.oml.evaluator.QvtException'` with code 120. Compat: replicate; fast: treat as `raise StringException('msg')` per spec.
* Caught exception variable: stdlib exceptions → `StdlibFactory.createException(type, msg, stack)`; user exception classes → instance of the (intermediate) class with `argument` and stack set (`:1698-1726`). `Exception::getArgument()` returns the message.
* `raise` argument that is not a String → message null.
* **`return`/`break`/`continue` inside an `except` body are discarded** (result of the body is ignored); after a handled exception `try` evaluates to null. A `return` inside the `try` body propagates normally.
* `try` body variables declared at top level of a non-operation block are removed after the block.

### 9.2 Termination / diagnostics

* Uncaught `QvtRuntimeException` in `visitModule`: logs `Terminating execution...` then `<java.class.Name>: <message>` + `\n\tat <Module>::<op>(<file>:<line>)` lines (`printQvtStackTrace`) to the **transformation Log** (`QE/…Impl.java:878-900`, `QE/QvtRuntimeException.java:46-66`). Stack element: `Module::[Context::]op(file:line)`, unknown line → `(file:<Unknown>)`, no source → `(Unknown Source)`, module init frames use the module name as op name (`QE/QVTStackTraceElement.java`, `QE/QvtStackTraceBuilder.java`).
* `InternalTransformationExecutor.createExecutionFailure` (`QO/InternalTransformationExecutor.java:323-361`): severity ERROR (CANCEL for interrupt), code `FATAL_ASSERTION=100` if exception type is AssertionFailed, `USER_INTERRUPTED=110`, else `EXCEPTION_THROWN=120`; message = exception message, or `QVT Stack overflow occured during execution`, or `QVT exception caught : '<class>'` when null; data = exception (or its cause); context diagnostics appended.
* Success: returns the context's `ExecutionDiagnostic` (children = log records with severity OK and code = level (0 default), non-fatal assertion records with WARNING/ERROR and code `NON_FATAL_ASSERTION=150`). Overall severity = max of children (so an `assert error` makes the transformation result ERROR even though execution completed).
* Other codes: `VALIDATION=130`, `MODEL_PARAMETER_MISMATCH=140` (wrong number of model args), `TRANSFORMATION_LOAD_FAILED=200` (`QO/../../qvt/oml/ExecutionDiagnostic.java`).
* `ExecutionDiagnosticImpl.toString`: `Diagnostic <SEVERITY> source=org.eclipse.m2m.qvt.oml.execution code=<c>: <msg>[ data=[…]][ [children]]`.

### 9.3 `log`

`doVisitLogExp` (`QE/…Impl.java:1391-1451`), `WriterLog` (`QU/WriterLog.java:77-103`), default log = `Log.NULL_LOG` (`org.eclipse.m2m.qvt.oml/src/org/eclipse/m2m/qvt/oml/ExecutionContextImpl.java:52`):

* Evaluation order: condition (`when`), then **level** (3rd arg, converted with `strictConvertNumber` to Integer; non-number → treated as no level), then message, then element.
* message: null → `<null>`; invalid → `<Invalid>`; else `String.valueOf(message)`.
* element: invalid → `<Invalid>`; `ExceptionInstance` → its `toString`; else object itself (later `String.valueOf`).
* Output (WriterLog, each record + `System.getProperty("line.separator")` **L**, flushed):
  * no level, no element/element null: `<msg>`
  * no level, element: `<msg>, data: <String.valueOf(elem)>`
  * level: `Level <level> - <msg>` / `Level <level> - <msg>, data: <elem>` — `MessageFormat` formats the Integer level with the default locale's grouping (**L**, e.g. `Level 1,234`).
* A diagnostic `(OK, level|0, message)` is added for every executed `log` (not for assert's log).
* `String.valueOf(EObject)` → `EObjectImpl.toString()` = `<runtime class name>@<Integer.toHexString(identityHash)>` + ` (eClass: <EClass.toString()…>)` for dynamic objects or ` (attr: value, …)` for generated → **N**. Compat: emit `ClassName@<surrogate hash>` with the same suffix structure; fast: a stable readable repr.

### 9.4 `assert`

`visitAssertExp` (`:1453-1485`): if value ≠ TRUE: `msg = "ASSERT [" + severity + "] failed at (" + file + ":" + line + ")"` (severity literal `warning|error|fatal`; file = last URI segment or `UknownSource` [sic]); with `with log(...)`: the log record is `msg + " : " + logMessage` (+ level/data as for log); without: `msg` alone. Fatal → throw `QvtAssertionFailed(logMessage-without-prefix or "Fatal assertion failed")`. Otherwise add diagnostic `(WARNING|ERROR, 150, logMessage or "Non-fatal assertion failed")`. Returns null.

### 9.5 Control flow

| Construct | Eclipse | Source |
|---|---|---|
| `return e` | wraps value in `OperationCallResult`; operation body stops; for ops with `OclVoid` type result forced to null; multi-result ops build tuple if result isn't a tuple | `:1160-1205` |
| helper/query result without `return` | value of the **last expression** of the body (block → null, var-init → null unless `withResult`) | `:1175-1205,1233-1264` |
| `break`/`continue` | sentinel objects propagated through blocks, `if`, `switch`, `compute`; `while` handles both; iterators handle both | `:1631-1642,1288-1316` |
| `compute (v : T := init) body` | binds v (replace), evaluates body, returns final v (or propagates break/continue/return) | `:1266-1286` |
| block `do {…}` | value always null (unless broken); variables declared directly in a nested block are removed on exit (not in operation-body blocks) | `:1233-1264` |
| `var x : T;` | type default value (see §0.13); `var x := e` → value of e (List coercion) | `:1211-1227` |
| `while (cond) body` | see §6 | |
| `unlink`, `unpack`, `OrderedTuple{}` | **not implemented** (evaluate to null, no effect) | `:1649-1657,1728-1736` |

---------------------------------------------------------------------------------------------------

## 10. Modules, configuration, `this`, `transform()`

* Module instantiation (`QE/…Impl.java:1026-1045,1751-1833`): imported modules initialised **before** the importing module, depth-first in `moduleImport` order, each once. Per module: configuration properties first (declaration order), then other module-level properties in declaration order: init expression or type default (`""`,0,false, empty collection, null for objects). Contextual (intermediate) properties skipped.
* Configuration properties: value from `Context` (`HashMap`); not provided → **null** (spec says undefined); a String value for a non-String typed property is converted by `ConversionUtils.createFromString` (`Integer(new String)` 32-bit, `Double`, `Boolean.valueOf` (anything but "true" ignoring case → false), enums via factory, collections/dicts via `ConversionCollection/ConversionDictionary` string syntax); failure → fatal `QvtRuntimeException("Failed to get '<p>' configuration property value from '<raw>' string")` (`:461-490`, `QS/ConversionUtils.java:24-67`). Nested transformations get a copy of the parent's config properties.
* `this` resolves to the module instance of the *statically referred* module type (`getThisOfType`), not dynamically (`:222-250`).
* `self` bound only for contextual operations; `result` bound for mappings/constructors (constructor: `result := self`) (`:1613-1625`).
* Model-type `where` conditions on non-out model params are checked at `transform`/main start → `QvtAssertionFailed("WHERE condition failed for model parameter '<p>' of transformation '<t>'")` (`:2632-2652`).
* `new T(m1, m2)` with T a transformation: arguments must be `ModelInstance`s (else `QvtRuntimeException("Undefined model passed to transformation")`); creates nested root env with **shared trace**, own deferred task list, copy of config props; returns the transformation instance (`:932-978`).
* `t.transform()` (`QS/TransformationOperations.java:58-94`): runs main (or blackbox); returns `Status`: failure with exception instance for any `QvtRuntimeException` (incl. fatal asserts) except interrupts; failure with null exception if the nested diagnostic is ERROR; else success. No `main` → invalid. `parallelTransform`, `wait` unsupported.
* Blackbox (Java) operations: exactly one handler must exist, else fatal "No blackbox implementation found for '<q>'" / "Ambiguous blackbox implementation found for '<q>'".

---------------------------------------------------------------------------------------------------

## 11. Miscellaneous surprises

* `isKindOf(value, Collection(T))` checks only the **first element** of a non-empty collection; empty collections conform to any collection type; `OclAny` excludes collections (`QA/QvtOperationalEvaluationEnv.java:444-482`).
* `getType(null)` = OclVoid (`:485-490`).
* Dict iteration (`forEach`, `->select` etc.) iterates **values**; `Dict::keys()`/`values()` return *new* Lists in HashMap order; `size()` = map size; `Dict += x` stores x under key null (`QS/DictionaryImpl.java:73-110`).
* `repr()` = `String.valueOf(x)` (`QS/ObjectOperations.java:17-32`); `dump()` (deprecated) logs `String.valueOf`.
* `Status.toString`: `SUCCESS` / `FAILED` + ` raised: (<exception>)`; exception `toString`: `<EClassName>[ : <arg>\n]` + stack lines.
* Variable names generated for iterator results: `__qvtresult__<n>` — a JVM-global static counter (`QE/…Impl.java:1738-1740`); invisible unless printed.
* `QvtPlugin.error` standalone output format: `ERROR <code>: <message>` + Java stack trace to `System.err`; WARNING similarly; INFO/OK suppressed (`QO/QvtPlugin.java:175-212`).
* Evaluation of `->` on a single (non-collection) object: implicit `asSet` per OCL (classic OCL, §A); `.` on a collection: implicit OCL `collect` (List source → Sequence type) (`QP/QvtOperationalVisitorCS.java:1533-1544,5676-5686`); `->` with a non-collection-op name on a collection (`coll->prop`, `coll->helper()`, `coll->map m()`) → implicit **xcollect** (`:1341-1352`).
* Legacy `org.eclipse.m2m.qvt.oml.ocl` plugin contains deprecated native "libraries" (StringLibrary, EmfToolsLibrary) loaded only via the legacy library mechanism — out of scope unless a transformation imports them.

---------------------------------------------------------------------------------------------------

## P. Parser (LPG grammars), lexer, accepted syntax

`Q/` = `org.eclipse.m2m.qvt.oml.cst.parser/`. File list and sizes: §1.4.

### P.1 Build chain

```
Parser:   Q/cst/QVTOParser.gi --%Import--> ImperativeOCL.gi --%Import--> EssentialOCL.gi
          la=2, template btParserTemplateF.gi (backtracking), import_terminals=QVTOLexer.gi (QVTOParser.gi:21-31)
          built with: lpg -backtrack -list QVTOParser.gi
Lexer:    QVTOLexer.gi -> ImperativeOCLLexer.gi -> EssentialOCLLexer.gi -> lpg/LexerBasicMapF.gi (filter QVTOKWLexer.gi)
KW lexer: QVTOKWLexer.gi -> ImperativeOCLKWLexer.gi -> EssentialOCLKWLexer.gi -> lpg/KWLexerMapF.gi
Completion: cst/completion/LightweightParser.gi, LightweightTypeParser.gi import ../QVTOParser.gi
```

* `ImperativeOCL.gi` hard-wires `$super_lexer_class = QVTOLexer` (ImperativeOCL.gi:43) → not usable standalone.
* Classic OCL's own `OCLParser.g` (not used) additionally has `@pre` (OCLParser.g:192), `^`/`^^` messages (201-259), and `context/inv/pre/post/def/body/derive/init/package` declarations.
* Known grammar conflicts: 3 reduce/reduce + 5 shift/reduce around optional `endif` (ImperativeOCL.gi:1004-1010) resolved by LPG backtracking order → a Rust parser must reproduce the same choice (prefer the longest/`endif`-consuming alternative as LPG does) — hand-written recursive descent with explicit disambiguation recommended.

### P.2 Keywords (all lexically reserved: every KW-lexer hit becomes its own token, `EssentialOCLLexer.gi:243`, `LexerBasicMapF.gi:66-75`; `QvtKeywords.java:22-28`)

* **EssentialOCL** (EssentialOCLKWLexer.gi:73-110): reserved `and implies not or xor if then else endif let in false true null invalid self`; restricted (usable as name only after `::`, `unreservedSimpleNameCS`, EssentialOCL.gi:381-382): `Bag Collection OrderedSet Sequence Set Tuple Boolean Integer Real String UnlimitedNatural OclAny OclInvalid OclVoid`.
* **ImperativeOCL** (ImperativeOCLKWLexer.gi:54-76): `Dict List break switch case xselect xcollect selectOne collectOne collectselect collectselectOne forEach forOne compute return var while continue log assert with new`.
* **QVTo** (QVTOKWLexer.gi:53-118): `static init end out object transformation import library metamodel mapping query helper inout when configuration intermediate property opposites class population map xmap late resolve resolveone resolveIn resolveoneIn invresolve invresolveone invresolveIn invresolveoneIn modeltype uses where refines access extends blackbox abstract result main this rename disjuncts merges inherits composes constructor datatype default derived do elif enum except exception from literal ordered primitive raise readonly references tag try typedef unlimited`.
* Reserved but unused by any rule (still block identifiers): `metamodel datatype default enum from literal primitive typedef unlimited`.
* Not in spec keyword list (QVT-1.3.txt:7174-7191): `selectOne collectOne collectselect collectselectOne` (spec: `xselectOne xcollectOne xcollectselect xcollectselectOne`), `opposites rename result this self invalid`, OCL type names. Spec keywords **not** reserved by Eclipse: `any collect collectNested exists forAll isUnique iterate one reject select sortedBy package xselectOne xcollectOne xcollectselect xcollectselectOne`.
* Keywords usable as names: `this`, `result` (QVTOParser.gi:1965-1967), `self`, `main` (QVTOParser.gi:2306), restricted type words after `::`, anything via `_'kw'` where `simpleNameCS` is accepted (not in `var` declarators, `forEach` iterators, parameter names, modeltype names, scoped ids — those take raw `IDENTIFIER`).

### P.3 Precedence (low → high; all binary ops left-assoc; `let` only as right operand, body extends right)

| # | Construct | Rule |
|---|---|---|
| 0 | `lhs := e`, `lhs += e` (lhs must be `primaryExpCS`), `return [e]`, `var …`, `log(…) [when e]`, `assert [sev] (e) [with log(…)]` — only at `OclExpressionCS` level, so `a and x := b` does not parse | ImperativeOCL.gi:324-325,363-383,492-528,1424-1489 |
| 1 | `implies` | EssentialOCL.gi:1282-1291 |
| 2 | `xor` | :1294-1300 |
| 3 | `or` | :1303-1309 |
| 4 | `and` | :1312-1318 |
| 5 | `=`, `<>`, `!=` | :1321-1331; `!=` QVTOParser.gi:2020-2024 |
| 6 | `> < >= <=` | :1334-1352 |
| 7 | `+ -` | :1355-1365 |
| 8 | `* /` | :1368-1378 |
| 9 | unary `-`, `not`; `let … in` | :1399-1409,1458 |
| 10 | postfix: `.name`, `.name(args)`, `.p::q(args)`, `->name(args)`, `->iter(v\|e)`, `->forEach(..){..}`, `->xselect(..)`, `->name` (xcollect), `[e]`/`![e]` (xselect/selectOne), `.map/.xmap n(..)`, `.resolve*(..)`, `->switch(..){..}`, `->object(..) T {..}` | EssentialOCL.gi:843-1191; ImperativeOCL.gi:583,1274,1306,1522,1690,1744; QVTOParser.gi:1916-1951 |
| 11 | primaries: literals, `self`, names/paths, `(e)`, `if..endif`, `while`, `switch`, `do`, `compute`, `try`, `raise`, `new`, `break`, `continue`, `object`, `map/xmap`, `resolve*` | EssentialOCL.gi:1427-1435; ImperativeOCL.gi:162,239-240,677,1214,1227,1516,1768,1797-1799; QVTOParser.gi:1936,1961-1963 |

vs OCL 2.4 §7.4.8 (OCL-2.4.txt:889-906): same order (xor looser than or, relational tighter than equality) minus `@pre`, `^`. vs QVT 1.3 §8.4.7 (QVT-1.3.txt:7370-7432): spec has `or`/`xor` on one level, all comparisons incl. `==` on one level, `%` multiplicative, unary `- not # ## *`, `!->`, assignment ops `:= ::= += -=` with `default`, block as assignment RHS, assignment LHS `<unary_exp>` — **none of `== % # ## -= !-> default`, unary `*` exist in Eclipse**; `::=` only as `var` initialiser (ImperativeOCL.gi:486); `@` only in `typespec ::= typeCS '@' IDENTIFIER` (QVTOParser.gi:2252).

### P.4 Deprecated / legacy syntax still accepted

| Syntax | Rule | Diagnostic |
|---|---|---|
| `import library X;` | QVTOParser.gi:2048 | warning DeprecatedLibraryImportStatement (QvtOperationalVisitorCS.java:2956) |
| `transformation X;` (no signature) | QVTOParser.gi:2058 | none (handled by `QP/DeprecatedSignaturelessTransf.java`) |
| `rename T.prop = 'name';` | QVTOParser.gi:2073-2074 | warning DeprecatedRenameStatement (VisitorCS:3649) |
| switch alt `(cond) ? expr;`, `else ? expr;` | ImperativeOCL.gi:1932,1963 | warnings (AbstractQVTParser.java:832; QVTOParser.java:4104) |
| `import` after other elements; members before module header | QVTOParser.gi:187 | warnings (AbstractQVTParser.java:1407,1418-1430) |
| `main(in x : T)` params | | warning EntryOp_DepricatedParamDecl (QvtOperationalValidationVisitor.java:565) |
| helper/query named `main` | | warning (AbstractQVTParser.java:364) |
| implicit `this`/`self` source calls | | warning / error only if explicit-self enforced (DeprecatedImplicitSourceCallHelper.java:112-131) |
| deprecated stdlib ops `dump()`, `Model::allInstances`, `OclAny::allInstances(T)` | | DeprecatedElement(By) warnings (VisitorCS:1320-1330) |
| OCL "conceptual operation names" `a.and(b)` | EssentialOCL.gi:279-294,1027-1028 | ProblemOption |

### P.5 Accepted constructs (selected)

Bare line numbers below refer to `QVTOParser.gi` for module-level constructs (header, classifiers, properties, operations, object/map/resolve) and to `ImperativeOCL.gi` for expressions/statements (if, var, while, forEach, iterators, switch, try/raise, assert/log, new, literals).

* Header: `qualifier* transformation a.b.C(in m : MM, out …) [access|extends [transformation|library] ref(sig)?, …]* | refines ref` — **dotted names**, multiple access/extends (QVTOParser.gi:295,333,398-415,2350-2373). `library a.b.L [(sig)]`. `import a.b.c;` only (no `from … import`). `modeltype ID ["strict"] uses pkg | pkg('uri') | 'uri', … [where {..}];` (527-629); no `metamodel`/`package`/`enum` blocks.
* Classifiers: only `intermediate class X [extends …] {…}` and `exception X [extends …] {…}` (639,651); feature `[<<tags>>] (composes|references|readonly|derived|static)* name : T [mult] [ordered] [opposites [~] n [mult]] [= init]` (717; multiplicity `..` or `...`, `*`).
* Module properties: `configuration property x : T;`, `property x [: T] [= e];`, `intermediate property C::x : T [= e];` (928-992) (init of intermediate property ignored at runtime, §0.14).
* Operations: `(helper|query) [Ctx::]name(params) [: results] (; | = e; | {…})` (1006-1113; context may be primitive/collection/tuple/List/Dict type); `constructor Ctx::name(..)`; `main(..)`; `mapping [in|out|inout] [Ctx::]name(params) [: results] [inherits|merges|disjuncts a, b]* [when {..}] [where {..}] (; | { [init{..}] [population{..} | stmts] [end{..}] })` (1283-1600; no `refines`, when/where blocks only); qualifiers `blackbox abstract static` only; params `[in|out|inout] ID : typespec` or `[dir] typespec`, no defaults; `typespec = T [@ model]`; `tag "id" scoped_id [= e];`.
* `if`: OCL form `if c then e [elif c then e]* [else e] endif` (else optional; ImperativeOCL.gi:688-745) **and** statement-level C form `if (c) stmt [elif (c) stmt]* [else stmt] [endif]` (813-936; branch may not start with unary minus or `*`).
* `var x := e | = e | ::= e (with-result) | : T [op e]`, `var (x…)`; multiple declarators only at statement level (345-357, 365-486).
* `let x : T = e in …` (OCL) and all-untyped `let x = e, y = e in …` (QVTOParser.gi:1979-2007), no mixing.
* `while (cond) {..}`, `while (x : T := e; cond) {..}`, `while (x := e; cond)` (544-569).
* `src->forEach(i, j [| cond]) {..}`, `forOne` (616-647), untyped iterators.
* `->xselect|xcollect|selectOne|collectOne(v | e)`, `->collectselect(One)(v ; t = body | cond)`, `src[cond]`, `src[x | cond]`, `src![…]`, `src->xcollect(..)[..]` folded to collectselect, `src->prop` → xcollect, `->object(x) T {..}` / `->switch(x) {..}` → xcollect (1537-1744, 1274, 1306).
* `switch { case (c) stmt … [else stmt] }` (1229,1350-1416); `compute (decl) {..}`, `do {..}`, `break`, `continue`, `return [e]`.
* `try {..} except (T1, T2) {..} | except (e : T) {..} | except () {..}` (269-320); `raise path [( [e] )]` (one arg max) and `raise 'msg'` (242-262; see §9.1 quirk).
* `assert [sev] (cond) [with log(..)]` (1465-1489), `log([args]) [when e]` (empty `log()` allowed, 1449).
* `new typespec(args)` (parentheses required, 1770); `object [result|self|ID :] [typespec] {..}` (QVTOParser.gi:1618-1638).
* `src.map name(args)`, `src->map name(args)`, `src.map Ctx::name(args)`, `map path::name(args)` (1668-1729).
* `[late] resolve|resolveone|invresolve|invresolveone( [x :] T [| cond] )`, `[late] resolveIn…(Ctx::mapping [, [x:] T [| cond]])` with `.`/`->`/no source (1743-1851,1916-1936).
* Literals: `Set{..}` etc. with `a..b`; `List{..}`; `Dict{k = v}` (key must be a primitive literal incl. `*`, `null`, `invalid`; ImperativeOCL.gi:193-231); `Tuple{a = e, b : T = e}` (≥1 part); no `OrderedTuple`, no `Set(T){..}`; type expressions as values; `*` literal.

### P.6 Spec EBNF items missing in Eclipse / Eclipse extensions

Missing: `from U import`, `metamodel`/`package`/`enum`/`typedef`/`datatype`/`primitive`/plain `class`, classifier operations, top-level `access`, `derived`/`literal` property keys, config property initialiser, parameter defaults, mapping `refines`, operators `== % # ## -= !-> default` & unary `*`, block as assignment RHS, `unlimited` literal, `object(iter)` / `switch(iter)` prefix forms, `map(decl)`/`new(decl)` without parens, `forEach(...; decl)`, spec spellings `xselectOne…`, `raise X(a, b)`. (The spec's `<elif_part> ::= 'elif' <if_body>` has no condition — a spec bug; Eclipse uses `elif c then`.)

Extensions: `!=`, C-style `if`, optional `endif`, untyped `let`, renamed iterators, `->prop`/`->object`/`->switch` shorthands, `while (x := e; c)`, `var x ::= e`/`var x = e`, `except ()`, `log()`, `rename`, `import library`, signature-less `transformation X;`, `?` switch syntax, dotted module names, `..` in multiplicities, `intermediate property`.

### P.7 Lexer

* Characters: ASCII via table; **every char 0x80–0xFFFE is a Letter** (incl. surrogates, U+00A0); **U+00B4 (´) and U+FFFF are classified as EOF** → a `´` anywhere ends lexing (LexerBasicMapF.gi:105-247; QVTOLexer.java:434-444).
* Identifiers `Letter (Letter|Digit|'$')*`, Letter = `a-z A-Z _ ≥0x80` (EssentialOCLLexer.gi:507-529); quoted `_'…'` (escapes decoded; adjacent `_'a' 'b'` concatenate; EssentialOCL.gi:371).
* Strings `'…'` and `"…"` (EssentialOCLLexer.gi:262; ImperativeOCLLexer.gi:114); raw TAB/LF/CR allowed inside; backslash must precede a printable char. Escape decoding (QVTo enables `USE_BACKSLASH_ESCAPE_PROCESSING`, QvtEnvironmentBase.java:120,127; decoder `O/lpg/AbstractParser.java:451-576`): `\b \t \n \f \r \" \' \\`, octal `\0`–`\377`, `\xhh`, `\uhhhh`; unknown escape → error + raw literal kept; **bug: `\4`–`\7` followed by a non-octal char re-emits the digit** (`'\4a'` → `"\u0004" + "4a"`, AbstractParser.java:542-549).
* Adjacent string literals concatenate (EssentialOCL.gi:769) but QVTo errors if both are on the same line: "Multiline string literals must be located in different lines!" (AbstractQVTParser.java:1284-1295) ⇒ `'it''s'` is an error, not an escaped quote.
* Lone CR rewritten to LF before lexing (QvtOperationalParser.java:157-173) — matters for line numbers in log/assert/stack traces; tab width 4.
* Numbers: Integer `Digit+`; Real `Int.Int [Exp] | Int Exp`, Exp `[eE][+-]?Int` (EssentialOCLLexer.gi:471-499); `1.` = INTEGER DOT; `1..2` = INTEGER DOTDOT INTEGER; `.5` not a real; integers parsed as **Long** (`USE_LONG_INTEGERS`), overflow → "NumberFormatException: …" error and value 0 (AbstractQVTParser.java:1298-1313); reals `Double.valueOf`.
* Comments `--`, `//` (to EOL), `/* … */` (non-nesting). Whitespace SP LF CR HT FF (VT is an error).
* Multi-char tokens: `:= += ! ::= ?` (ImperativeOCLLexer.gi:76-100), `<< >> ... ~ != @` (QVTOLexer.gi:73-103) ⇒ `a>>b` / `x<<y` without spaces do not lex as comparisons. `#`, `%`, `^`, `&`, `` ` `` are lexer errors.

**Parser compat strategy:** hand-written lexer reproducing the char classes (incl. the ´/U+FFFF EOF quirk, `<<`/`>>` tokens, escape bug) + recursive-descent/Pratt parser over the cascade above, with LPG-equivalent resolution of the `endif` conflicts; diagnostics texts/positions copied from `ValidationMessages.properties`/`messages.properties`. Fast mode can accept spec extras (`==`, `%`, `#`) behind a flag.

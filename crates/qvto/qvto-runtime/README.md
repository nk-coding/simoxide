# qvto-runtime

Runtime values of the QVTo engine and the Java-compatible building blocks of compat mode:
OCL values with null and invalid, Java's number classes, strings, the four OCL collection kinds
with the Java classes Eclipse uses for them, tuples, QVTo's `Dict` and `List`, Java hashing and
`equals`, a port of `java.util.HashMap` iteration order, `Double.toString`, Java's number parsers,
`String` operations and character data. Status: Q1c (see `PLAN.md`); the standard library and the
VM come with Q2/Q3.

```rust
use qvto_runtime::{coll::ops, CompatEnv, Coll, CollKind, Value};
let env = CompatEnv::default();                     // identity hashes: HotSpot surrogate
let mut s = Coll::new_set(&env);                    // java.util.HashSet
for i in [17, 1, 33] { s.add(Value::int(i), &env); }
let v = ops::qvto_including(s.into(), Value::str("x"), &env);   // Set::including, QVTo's handler
assert_eq!(qvto_runtime::display::to_java_string(&v, &env), "[17, 1, 33, x]"); // bucket 1, then 8
```

## Layout

| File | Contents |
|---|---|
| `src/value.rs` | `Value` (16 bytes), `ObjRef`, `EnumLit`, `TypeRef`, `IntBox`, `JNum` (Java number classes) |
| `src/str.rs` | `Str`: thin reference-counted WTF-8 string with cached `String.hashCode` and UTF-16 length |
| `src/identity.rs` | `HashEnv` (mode, identity hashes, class names), `IdentityKey`, `IdentityHashSource` (`HotSpotXorShift`, `Tape`, `Sequential`), `CompatEnv`, `FastEnv` |
| `src/eq.rs` | Java `equals`/`hashCode` of values, OCL `=` (`ObjectUtil.equal`), `String.compareTo` |
| `src/coll/` | `Coll` (Set = `HashSet`, OrderedSet = `LinkedHashSet`, Bag = `BagImpl`, Sequence = `ArrayList`), `LinkedMap` (insertion order), `ops.rs`: `CollectionUtil` and QVTo's collection handlers |
| `src/tuple.rs`, `src/dict.rs` | tuples; `Dict` (`DictionaryImpl`) and `List` (`MutableListImpl`) |
| `src/num.rs` | OCL arithmetic and comparisons (promotion, 64-bit wrap-around, `Integer`/`Long` normalisation, UnlimitedNatural) |
| `src/strings.rs` | OCL and QVTo string operations on values |
| `src/display.rs` | `toString` of values (Java's, `CollectionUtil.toString`, tuples, OCL `toString()`) |
| `src/model.rs` | `Datum` ↔ `Value` |
| `src/java/hash.rs` | `hashCode` of `String`, boxes; `HashMap.hash` spreading; `Double.compare` |
| `src/java/hashmap.rs` | `JavaHashMap`: `java.util.HashMap` with Java's iteration order, including tree bins |
| `src/java/fmt.rs` | `Double.toString`, `Float.toString` (JDK 19+), integers |
| `src/java/parse.rs` | `Integer.parseInt`, `Long.parseLong`, `Double.valueOf` |
| `src/java/chars.rs`, `src/java/chars_data.rs` | `Character` data, **generated** from the JDK (`tools/oracle/gen-chars.sh`) |
| `src/java/string.rs` | `String` operations with UTF-16 indices, root-locale case mapping, `StringTokenizer` |
| `tools/oracle/` | the Java oracle (`Oracle.java`, `OclEval.java`, `run.sh`) and the character-table generator |

## Values

| OCL / QVTo | `Value` | Java class in Eclipse |
|---|---|---|
| null, invalid | `Null`, `Invalid` | `null`, `OCLStandardLibraryImpl.INVALID` |
| Boolean | `Bool` | `Boolean` |
| Integer | `Int(i64)`: `Integer` if it fits in 32 bits, else `Long` | `Integer` / `Long` (`coerceNumber`) |
| uncoerced EMF boxes | `Boxed(IntBox::{Long, Short, Byte, Char}, i32)` (a `Long` outside `int` range is `Int`) | `Long`, `Short`, `Byte`, `Character` |
| Real | `Real(f64)`; `Float(f32)` for `EFloat` values | `Double` / `Float` |
| String | `Str` | `String` |
| model object | `Obj(ObjRef)` (host or engine-local, top bit) | `EObject` |
| enum literal, type | `Enum(EnumLit)`, `Type(TypeRef)` | `EEnumLiteral`, `EClassifier` |
| collections | `Coll(Rc<Coll>)`, immutable, copy on write | `HashSet`, `LinkedHashSet`, `BagImpl`, `ArrayList` |
| tuple | `Tuple(Rc<Tuple>)` | `AbstractTuple` |
| Dict, List | `Dict(Rc<RefCell<Dict>>)`, `List(Rc<RefCell<List>>)` (mutable, shared) | `DictionaryImpl`, `MutableListImpl` |
| model, `this` | `Model(u32)`, `Instance(u32)` | `ModelInstance`, `ModuleInstance` |

The runtime kind of a collection is its Java class, not its static type: `Model::objectsOfType`
returns an OrderedSet-kind value with static type Set, QVTo's `Set::including` turns it into a
Set (`HashSet`), and `=` between a `LinkedHashSet` and a `HashSet` is false, as in Eclipse.

Copy on write follows Java: an operation reuses its operand's storage (in place when the
reference is unique) only when Java's copy constructor would rebuild an identical table (same
capacity, no tree bins); otherwise it rebuilds as Java does, so iteration order never depends on
whether the storage was shared.

## Compat and fast mode

The mode is a property of the run's `HashEnv` (`CompatEnv`, `FastEnv`, or the VM's own):

| | compat | fast |
|---|---|---|
| Set, Bag, Dict order | `java.util.HashMap` bucket order (port, `JavaHashMap`) | insertion order (`LinkedMap`) |
| OrderedSet, Sequence, List | insertion order | same |
| identity hashes (objects, enum literals, tuples' types, bag count cells) | memoised per key from an `IdentityHashSource` (HotSpot xor-shift surrogate, tape, sequential) | a fixed mix of the key |
| everything else (values, Java classes of collections, equality, arithmetic, strings, formatting) | Java's | same |

## Identity hashes and the model API

`IdentityKey::Obj(ObjRef)` keys an object's identity hash. A store handle maps to an `ObjRef`
through the caller (`model::datum_to_value(d, |o| ObjRef::host(o.0))` for `qvto_emf::ObjId`);
the VM implements `HashEnv` with the store (Java class names for tree-bin tie-breaks, a tape
keyed by stable object keys of the Java run for differential tests).

## Verification

`tests/differential.rs` (ignored by default, needs the JDK and the reference runner's jars)
generates cases deterministically, asks the Java oracle (`tools/oracle/run.sh`, JDK 21, Eclipse
OCL 3.22 and QVTo 3.11 classes for `CollectionUtil`, `BagImpl`, `DictionaryImpl` and the classic
OCL evaluator) and compares:

| Test | What | Agreement |
|---|---|---|
| `hash` | `hashCode` of strings (non-ASCII, surrogate pairs, lone surrogates) and boxes | 20 000 / 20 000 (200 000 at scale 10) |
| `coll-growth` | `HashSet`/`BagImpl`/`LinkedHashSet` built across resize thresholds with removals, iterator removal, copy constructors, `clear` | 1 500 / 1 500 scripts (15 000 at scale 10) |
| `coll-collisions` | equal-hash strings, integers, Integer/Long pairs, objects in one bucket: treeified bins, removal (movable and through the iterator), `removeAll` | 1 500 / 1 500 (15 000 at scale 10) |
| `coll-ops` | `CollectionUtil` and QVTo handlers on random kinds, nested collections, `at`, `indexOf`, `count`, `=`, `equals`, `sum`, `max`, `min` | 6 000 / 6 000 (60 000 at scale 10) |
| `coll-dict` | `DictionaryImpl` put, value removal, copy | 1 500 / 1 500 |
| `tostring` | `toString` of collections, bags, dictionaries with numbers, strings, booleans | 5 000 / 5 000 |
| `dtoa`, `ftoa` | `Double.toString` (powers of ten ±1 ulp, subnormals, random bits), `Float.toString` | 209 819 / 209 819, 103 000 / 103 000 (2 009 819 and 1 003 000 at scale 10) |
| `parse` | `Double.valueOf`, `Integer.valueOf`, `Long.parseLong` (decimal, hex floats, suffixes, whitespace, garbage) | 60 195 / 60 195 |
| `ocl-num` | classic OCL evaluator on literal expressions: `+ - * / div mod max min < > <= >= = <> - abs floor round toString` over Integer/Long/Real edge values (NaN compared by `doubleToLongBits`) | 40 000 / 40 000 |
| `string` | `String` `hashCode`, `length`, `toUpperCase`/`toLowerCase` (final sigma, `ß`, `İ`), `trim`, `compareTo`, `equalsIgnoreCase`, `indexOf`, `lastIndexOf`, `replace`, `startsWith`, `endsWith`, `substring`, `StringTokenizer` | 48 138 / 48 138 |
| `char`, `codepoint` | `Character` classification and case mappings of all 65 536 code units; full case mappings of 149 024 code points | all agree |

```sh
CARGO_TARGET_DIR=... nice -n 19 cargo test -p qvto-runtime --test differential -- --ignored --nocapture --test-threads=4
QVTO_WRITE_GOLDEN=1 ...   # also rewrites tests/golden/*.tsv (samples with Java's answers)
QVTO_DIFF_SCALE=10 ...    # ten times the cases
```

`tests/golden.rs` replays the committed samples (12 993 cases) without Java; `tests/fast_mode.rs` checks that
fast mode holds the same elements in insertion order.

## Not here yet

Regular expressions (`matches`, `replaceAll`, QVTo `match`; Java syntax needs its own engine),
QVTo's string counters (`addSuffixNumber`, run state), `List`/`Dict` mutators and the
remaining standard library (Q3a/Q4), `BigInteger`/`BigDecimal` attribute values (kept as
strings), `MessageFormat` of log levels.

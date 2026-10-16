# qvto-rs design

State: phase Q0, 2026-10-16. Sources are in `reference/` (see `reference/README.md`).
"Eclipse" means Eclipse QVTo 3.11 on classic Eclipse OCL, the reference implementation.

## 1. Goals

- Run QVTo transformations as Eclipse QVTo does, as completely as possible.
- Two modes: **compat** (Eclipse's observable behaviour, quirks included) and **fast** (same
  semantics, no Java emulation where it costs time).
- Compile to bytecode and interpret it. No JIT.
- Be fast from the start: compile once, run many times, no model copying.
- Run inside SimOxide later, on its model graph, with runtime-mutated PCM models.
- Stay out of the way of other work on this machine (niced builds, small disk footprint).

## 2. Scope

### 2.1 Language

Everything of QVT 1.3 chapter 8 that Eclipse implements:

- **Modules**: `transformation` with model parameters (`in`, `out`, `inout`), `modeltype` with
  `uses` (nsURI or file), `strict` and `where` conditions, `library`, `import`, `access`,
  `extends`, `blackbox` units, nested transformations and `transform()` invocation,
  `configuration property`, module `property`, `intermediate class` and `intermediate property`,
  `tag`, `rename`, the legacy `mapping main(in x: T): T` entry and the `main()` entry.
- **Operations**: `mapping` (with `when`, `where`, `init`, `population`, `end`, `inherits`,
  `merges`, `disjuncts`, `abstract`), `helper`, `query`, `constructor`, contextual operations,
  overloading and virtual dispatch on the context type, `result` and multiple results.
- **Object creation and tracing**: `object` expressions (implicit and explicit target), `new`,
  trace records, `map`/`xmap` (also on collections with `->map`), `resolve`, `resolveone`,
  `resolveIn`, `resolveoneIn`, `invresolve…`, `late resolve`, conditions in resolves.
- **ImperativeOCL**: blocks (`do`), `var`, `:=`, `+=`, `if`/`elif`/`else`, `while`, `forEach`,
  `forOne`, `compute`, `switch`/`case`/`else`, `break`, `continue`, `return`, `raise`,
  `try`/`except`, `log`, `assert` (warning/error/fatal) with `with log`, `xcollect`,
  `xselect`, `selectOne`, `collectselect`, `collectOne`, the `[T]` / `[cond]` shorthands and
  `->prop`, `Dict` and `List` types and literals, tuples, `unlink`, and Eclipse's extensions
  (`!=`, C-style `if`, optional `endif`, `var x ::= e`, `while (x := e; c)`, `except ()`).
- **Not in Eclipse, so not in compat mode**: the spec's `#T`, `##T`, `->[...]`, `==`, `%`,
  `-=`, `default`, mapping `refines`, parameter defaults and the other items of
  `reference/notes/eclipse-semantics.md` §P.6. They may come later behind a flag.
- **Standard library**: QVT 1.3 §8.3 as Eclipse implements it (operations on objects, elements,
  models, transformations, lists, dicts, strings, numbers; `Status`, `Exception`, `Model`,
  `Transformation` types; the deprecated `Strings` and `emf.tools` libraries the test suite uses).
- **OCL**: the classic Eclipse OCL subset that QVTo embeds: literals, `let`, `if`, navigation
  (`.` and `->` incl. implicit `asSet` and implicit collect), all OCL 2.x collection operations
  and iterators (`select`, `reject`, `collect`, `collectNested`, `forAll`, `exists`, `one`,
  `any`, `isUnique`, `sortedBy`, `closure`, `iterate`), type operations (`oclIsKindOf`,
  `oclIsTypeOf`, `oclAsType`, `oclType`, `allInstances`), `oclIsUndefined`, `oclIsInvalid`,
  null and invalid, Integer, Real, String, Boolean, UnlimitedNatural, tuples, enumerations,
  and the operations Eclipse's classic OCL adds or changes.

### 2.2 What "as much as possible" means

| Target | Measure |
|---|---|
| Eclipse test suite, compat mode | ≥ 95 % of the runnable cases (`reference/inventory/`: 309 transformation cases, plus the stack/log/trace drivers, minus JDT/PDE, Ant, debugger and project-reference cases) pass with the suite's comparer, and their output models and logs equal the Java runner's (strict canonical comparison) |
| Parser and compiler tests | the 159 parser cases: same errors and warnings at the same spans (messages best effort) |
| SimuLizar rules | all 37 `.qvto` files compile; the 22 rule files give the same models and logs as the Java runner on the corpus PCM models |
| Slingshot SPD transformations | all 18 files compile and give the same models and logs as the Java runner on the SPD examples |
| Fast mode | the same cases pass under the order-insensitive comparison |

### 2.3 Non-goals

- QVT Relations and QVT Core, the graphical syntax.
- Editors, outline, content assist, debugger, launch configurations, the Ant task, the
  Eclipse workspace builder, incremental compilation.
- Java blackboxes (no JVM in the engine). Blackboxes are Rust functions; the test-suite
  blackboxes and the Palladio ones (`getSimulationTime`, the stereotype library) are ported.
- Executing the compiled AST XMI (`.qvtox`) of Eclipse. We compile from source.
- Incremental update execution with an input trace (one test case uses it; maybe later).
- The Pivot-based OCL; complete OCL documents (`.ocl`).
- UML profiles and CMOF.

## 3. Architecture

```
          .qvto ──► qvto-syntax ──► AST ──► qvto-sema ──► typed IR ──► qvto-bytecode ──► Program
                                              ▲    ▲                                      │
                       .ecore ──► qvto-emf ───┘    └── qvto-types (types, library sigs)   ▼
  models (XMI or host graph) ──► qvto-model traits ◄──────────────────────────────── qvto-vm
                                                                                         │
                                       qvto-runtime (values, collections, stdlib) ◄──────┘
```

| Crate | Role | Depends on |
|---|---|---|
| `qvto-syntax` | lexer, parser, AST with spans, error recovery, unparser for tests | — |
| `qvto-model` | model API traits, metamodel registry, ids | — |
| `qvto-emf` | own Ecore and XMI: `.ecore` loading, resource sets, proxies, XMI load and save, canonical dump | `qvto-model` |
| `qvto-types` | OCL/QVTo type system, conformance, the standard library signature table, blackbox signatures | `qvto-model` |
| `qvto-sema` | units and imports, metamodel binding, name resolution, type checking, desugaring of shorthands; typed IR; diagnostics | `qvto-syntax`, `qvto-model`, `qvto-types` |
| `qvto-bytecode` | instruction set, compiler from the typed IR, disassembler, constant pools | `qvto-sema`, `qvto-types` |
| `qvto-runtime` | `Value`, collections, tuples, Dict/List, Java hashing and formatting, the standard library implementations, the blackbox trait | `qvto-model`, `qvto-types` |
| `qvto-vm` | interpreter, frames, mapping calls, traces, resolve, late resolve, extents, exceptions, log, assert | `qvto-bytecode`, `qvto-runtime`, `qvto-model` |
| `qvto` | facade: `Engine::compile`, `Program::run(models, config, mode)`, results | all of the above |
| `qvto-testkit` | conformance runner, differential tests against the Java runner, comparers | `qvto`, `qvto-emf` |
| `qvto-cli` | `qvto run | check | conform | bench | disasm` | `qvto`, `qvto-testkit` |

The standard library has a single source of truth: a declarative table in `qvto-types` (name,
receiver type, parameters, result type, flags such as "propagates invalid", an operation id).
`qvto-sema` uses it for typing and overload resolution; `qvto-runtime` implements each id; the
VM dispatches by id. A table-driven test checks that every id has an implementation.

### 3.1 Syntax

- The authority is Eclipse's LPG grammar (`QVTOParser.gi` 2 384 lines, `ImperativeOCL.gi` 1 983,
  `EssentialOCL.gi` 1 484, and the lexer and keyword lexer files), not the spec EBNF (QVT 1.3
  §8.4.7), which is incomplete and differs in places. Deviations go into a table in
  `qvto-syntax`'s docs (`crates/qvto-syntax/README.md`, "Differences from Eclipse").
- Hand-written lexer and recursive-descent parser with Pratt parsing for OCL operator
  precedence. LPG is a backtracking LALR generator; a hand-written parser gives better error
  recovery and spans. Keywords and operators are taken systematically from the grammar's keyword
  lexers (generated table plus a test that compares with the `.gi` files), not hand-maintained.
- Lossless tokens with byte spans; line/column maps for diagnostics; recovery at statement and
  declaration boundaries (`;`, `}`, top-level keywords).
- Deprecated syntax that Eclipse still accepts is accepted, with Eclipse's warnings: `import
  library`, `rename`, signature-less `transformation X;`, `?` switch alternatives.
- Facts to reproduce (from `reference/notes/eclipse-semantics.md` §P): QVTo has its own copies
  of the EssentialOCL grammars; LPG builds a backtracking LALR(2) parser (643 states, 837
  rules); 119 reserved keywords, including dead ones (`metamodel`, `datatype`, `enum`,
  `typedef`), while `select`, `collect` etc. are not reserved; `<<` and `>>` are single tokens;
  integer literals are parsed as Long; an octal-escape bug turns `'\4a'` into `"\u0004" + "4a"`;
  `´` ends lexing; adjacent string literals on one line are an error; a lone CR becomes LF
  (line numbers in logs and stack traces depend on it).

### 3.2 Semantic analysis

- Units resolve by qualified name against a source path list (like Eclipse's source container:
  `import models.foo.Bar;` resolves `models/foo/Bar.qvto`) and against registered blackbox units.
- `modeltype` binds by nsURI through the metamodel registry, or by `.ecore` path.
- Type checking follows classic OCL's rules (conformance, implicit collection conversions,
  `->` on single objects, implicit `collect` for `.` on collections, the QVTo `[T]` shorthands),
  and QVTo's (assignment conversions, `+=`, result types, `object` targets).
- Diagnostics: severity, span, a stable code, and Eclipse's message text where we have it (the
  `messages.properties` files of the compiler) so the compiler tests can compare more than spans.
- Output: a typed IR (expression trees with resolved operations, features, variables as slots).

## 4. Model API

The engine never touches model storage directly. `qvto-model` defines:

### 4.1 Metamodels

```rust
/// Dense ids, unique within one MetaRegistry.
pub struct ClassId(u32); pub struct FeatureId(u32); pub struct DataTypeId(u32);
pub struct EnumId(u32); pub struct PackageId(u32);

pub struct MetaRegistry { /* packages by nsURI, classes, features, enums, data types */ }

pub struct ClassInfo {
    pub name: Box<str>, pub package: PackageId, pub is_abstract: bool, pub is_interface: bool,
    pub supers: Box<[ClassId]>,
    pub all_supers: Box<[ClassId]>,          // EMF eAllSuperTypes order
    pub all_features: Box<[FeatureId]>,      // EMF eAllStructuralFeatures order
    pub id_attribute: Option<FeatureId>,
    pub subtype_bits: BitSet,               // O(1) conformance
}
pub struct FeatureInfo {
    pub name: Box<str>, pub owner: ClassId, pub ty: TypeRef,
    pub lower: i32, pub upper: i32,          // -1 = *
    pub ordered: bool, pub unique: bool, pub changeable: bool,
    pub derived: bool, pub transient: bool, pub volatile: bool, pub unsettable: bool,
    pub containment: bool, pub container: bool, pub opposite: Option<FeatureId>,
    pub resolve_proxies: bool, pub default: Option<Datum<'static>>,
}
```

Metamodels are small. They are converted once into a `MetaRegistry`: from `.ecore` files by
`qvto-emf`, from Ecore itself (built in), and from SimOxide's generated static tables by an
adapter. Converting metamodels is cheap; models are never copied. The compiler resolves every
class and feature name to ids at compile time, so the VM only sees ids.

Ecore is itself a metamodel here (the test suite mostly transforms `.ecore` files). Its
operations that tests call (`eAllStructuralFeatures`, `eContainer`, `eClass`, …) are derived
features or library operations.

### 4.2 Models

```rust
/// A store of model objects: one or more resources, all extents of one run.
pub trait ModelStore {
    type Obj: Copy + Eq + Hash + Ord;        // host object handle, e.g. u32

    fn meta(&self) -> &MetaRegistry;
    fn class_of(&self, o: Self::Obj) -> ClassId;
    fn is_proxy(&self, o: Self::Obj) -> bool;

    // Reading. Many-valued features are read by index to avoid allocation.
    fn get(&self, o: Self::Obj, f: FeatureId) -> Datum<'_, Self::Obj>;      // single-valued
    fn len(&self, o: Self::Obj, f: FeatureId) -> usize;                       // many-valued
    fn get_at(&self, o: Self::Obj, f: FeatureId, i: usize) -> Datum<'_, Self::Obj>;
    fn is_set(&self, o: Self::Obj, f: FeatureId) -> bool;
    fn container(&self, o: Self::Obj) -> Option<(Self::Obj, FeatureId)>;
    fn resource_of(&self, o: Self::Obj) -> Option<ResourceId>;
    fn roots(&self, r: ResourceId) -> &[Self::Obj];

    // Writing, with EMF list semantics (opposites, containment moves, uniqueness).
    fn create(&mut self, c: ClassId) -> Self::Obj;
    fn set(&mut self, o: Self::Obj, f: FeatureId, v: Datum<'_, Self::Obj>);
    fn unset(&mut self, o: Self::Obj, f: FeatureId);
    fn add(&mut self, o: Self::Obj, f: FeatureId, v: Datum<'_, Self::Obj>, at: Option<usize>) -> bool;
    fn remove(&mut self, o: Self::Obj, f: FeatureId, v: Datum<'_, Self::Obj>) -> bool;
    fn clear(&mut self, o: Self::Obj, f: FeatureId);
    fn move_to(&mut self, o: Self::Obj, f: FeatureId, to: usize, from: usize) -> bool;
    fn add_root(&mut self, r: ResourceId, o: Self::Obj, at: Option<usize>);
    fn remove_root(&mut self, r: ResourceId, o: Self::Obj);
    fn delete(&mut self, o: Self::Obj);      // EcoreUtil.delete: detach and clear inverse refs
}

/// A feature value as seen through the API.
pub enum Datum<'a, O> {
    Null, Bool(bool), Int(i64), Long(i64), Real(f64), Str(&'a str), Enum(EnumId, u32),
    Obj(O), Other(&'a str) /* lexical value of an uninterpreted data type */,
}
```

- **Contract.** The trait doc states EMF's semantics precisely: `add` to a unique list that
  already contains the value is a no-op that returns `false`; adding to a containment feature
  removes the object from its old container (or resource root list); setting one end of a
  bidirectional reference updates the other end (displacing a previous single value); `clear`
  and `unset` of containment detach the children; `delete` follows `EcoreUtil.delete`. A generic
  contract test suite in `qvto-model` runs against every implementation.
- **Monomorphisation.** The VM is generic over `S: ModelStore`. No dynamic dispatch on the hot
  path.
- **Engine-local objects.** Instances of intermediate classes, `Exception` and `Status` objects,
  and transformation instances live in a store owned by the VM. A VM object handle is
  `ObjRef(u32)` whose top bit selects host store or local store. Intermediate *properties* on host
  classes live in a side table keyed by `(object, property)`, as Eclipse keeps them in an
  adapter, so a host store never needs to grow new features.
- **Extents.** A model parameter's extent is the list of root objects bound to it. Input extents
  are the roots of the given resources. Output extents collect created objects that end up
  without a container, in creation order; Eclipse's exact rule is in the compat table.
- **Ordering.** `all_features` order defines the order of `eContents`/`eAllContents`, which
  `allSubobjects`, `objectsOfType`, `allInstances` and `deepclone` follow.
- **Proxies.** Cross-resource references are resolved by the store on read where EMF resolves
  them (resolving lists); unresolved proxies are objects with `is_proxy` true.
- **Change journal (optional).** A store may record a journal of changes (set, add, remove,
  create, delete) for its host. The engine does not depend on it.

### 4.3 qvto-emf

The engine's own store: dynamic metamodels from `.ecore`, `Graph` with per-object slot vectors
indexed by the class's feature index (not a feature-keyed list), XMI 2.0 load and save with
EMF's defaults (ids, `href`s, `xsi:type`, default value suppression, feature order), resource
sets and URI mapping (`platform:/plugin`, `platform:/resource`), and a canonical dump for
comparisons. SimOxide's XMI loader (`simoxide-model/src/xmi.rs`, `raw.rs`, `canon.rs`) is the
template: same author, same licence, same EMF-faithfulness checks.

### 4.4 Running inside SimOxide

SimOxide's `simoxide-model` already has an EMF-faithful generic graph (`raw::Graph`: objects with
class, container, proxy, slots; mutators that maintain opposites, `raw.rs` lines ~420–600) and
generated static metamodel tables (`meta/generated.rs`). The integration:

1. **Adapter** (in SimOxide, a feature-gated module or a small `simoxide-qvto` crate):
   - `MetaRegistry` built from `simoxide_model::meta` tables, with ids equal to SimOxide's
     `ClassId`/`FeatureId` (so no translation on access).
   - `impl ModelStore for GraphStore<'_>` wrapping `&mut raw::Graph`: reads map directly to
     `Graph::slot`; writes call the existing mutators (`set_ref`, `add_ref_unique`, `clear_refs`,
     `set_attr`, …), which become `pub` or get a public wrapper; `create` appends an object.
   - The journal records every change.
2. **SimOxide-side work** (this is option D of `SIMOXIDE-self-adaptation-options.md`):
   - keep the `raw::Graph` of the run alive and mutable (today the typed model and the
     `CompiledModel` are built once and shared immutably);
   - store runtime measurement values so rules can read them (the PRM model);
   - after each rule run, apply the journal: incremental updates of resources, allocation and
     route caches, connector maps, StoEx recompilation, plus the syncers' semantics and gaps
     (container removal not handled, etc.);
   - bind models by nsURI and `inout` like SimuLizar's `QVTOExecutor` (PCM models are `inout`,
     the PRM and monitor repository are `in`), configuration properties, the blackboxes
     `getSimulationTime` and the stereotype library;
   - rule loading and order: SimuLizar loads every `*.qvto` of the rules folder in file-system
     order; SimOxide fixes the order explicitly.
3. **Determinism.** New PCM objects need ids. Compat mode generates EMF-like UUIDs from a seeded
   generator so runs are reproducible; byte equality with SimuLizar is only possible against a
   patched reference (as the options doc notes).

The engine side needs no SimOxide knowledge. The cost per rule run is the VM run itself: the
program is compiled once, models are read in place.

## 5. Bytecode and VM

### 5.1 Values

```rust
pub enum Value {                // 16 bytes
    Null, Invalid,
    Bool(bool), Int(i64), Real(f64),  // Integer/Long by range; `*` is Int(-1) (compat row 8)
    Str(Str),                   // Rc<str> with cached Java hash, or interned id
    Obj(ObjRef),                // host or local object
    Enum(EnumId, u32),
    Type(TypeId),               // for oclAsType, allInstances, objectsOfType arguments
    Coll(Rc<Coll>),             // Set, OrderedSet, Bag, Sequence (immutable, copy-on-write)
    Tuple(Rc<Tuple>),
    Dict(Rc<RefCell<Dict>>),    // mutable, identity semantics
    List(Rc<RefCell<List>>),    // mutable, identity semantics
    Model(ExtentId), Exception(ObjRef), Status(..), Transformation(..),
}
```

- OCL collections are values: operations return new collections. `Rc::make_mut` makes
  `including`/`excluding` in place when the collection is not shared, which turns the common
  "build a set in a loop" into amortised O(1) per step.
- `Coll` stores its kind, its Java class tag (`HashSet`, `LinkedHashSet`, `BagImpl`,
  `ArrayList`; compat row 3), a `Vec<Value>` and, for Set/OrderedSet/Bag, a hash index. Compat
  mode keeps Set and Bag in a port of Java's `HashMap` (§6.1); fast mode uses an
  insertion-ordered index.
- Integers read from `ELong`/`EShort`/… attributes carry a box-kind tag, because Java's
  `equals` and `hashCode` tell `Long(1)` from `Integer(1)` (compat row 6).
- Strings are `Rc<str>` with a lazily cached Java hash; literals and feature names are interned.
- Null and invalid are distinct values with OCL's propagation rules (see the compat table for
  Eclipse's actual rules).

### 5.2 Instruction set

Register machine, like Lua's: each operation's frame has a fixed number of registers, computed by
the compiler. Fewer instructions than a stack machine and no stack shuffling for OCL's
expression trees. Instructions are a Rust `enum` of at most 8 bytes (opcode + three `u16`
operands, or a register and a 32-bit index), dispatched by `match`.

Groups:

| Group | Examples |
|---|---|
| moves and constants | `Move`, `LoadConst`, `LoadNull`, `LoadInvalid`, `LoadSelf`, `LoadThis` |
| arithmetic and logic, specialised by static type | `AddInt`, `AddReal`, `AddGeneric`, `Cmp*`, `Not`, three-valued `And`/`Or`/`Implies` as jumps |
| control | `Jump`, `JumpIf`, `JumpIfNot`, `JumpIfUndefined`, `Switch` |
| navigation | `GetAttr`, `GetRef`, `GetMany` (by `FeatureId`), `GetContainer`, `GetIntermProp`; collection navigation (implicit collect) as loops |
| mutation | `SetFeature`, `AddFeature`, `ClearFeature`, `SetIntermProp`, `Unlink` |
| collections | `NewColl`, `CollPush`, `IterInit`, `IterNext` (loops for iterators are compiled inline, no closures), `CollOp(op_id)` |
| calls | `CallStd(op_id)`, `CallHelper(fn)`, `CallVirtual(selector)` (contextual operations with overriding), `CallMapping(fn)`, `CallMappingColl`, `CallBlackbox(id)`, `CallTransformation` |
| objects and traces | `NewObject(class, extent)`, `ResultCheck`, `TraceLookup`, `TraceRecord`, `Resolve(kind, filter)`, `LateResolve` |
| imperative | `Log`, `Assert`, `Raise`, `Return`, handler ranges for `try` in a side table |

The compiler lowers OCL iterators (`select`, `collect`, `forAll`, `exists`, `sortedBy`,
`closure`, `iterate`, the `x…` imperative iterators) to loops over `IterInit`/`IterNext` with
the body inline. This is the main structural speed-up over Eclipse's visitor interpreter, which
creates an evaluation environment per iteration.

### 5.3 Calls, mappings, traces

- Frames live on one register stack (`Vec<Value>` windows) plus a call-frame stack. A depth
  limit of 1 000 reproduces Eclipse's `QvtStackOverFlowError` (test `bug370098`).
- Contextual operations dispatch on the dynamic type of `self` through a per-selector table
  built at load time (Eclipse's `OperationOverrideMap` rules).
- A mapping call follows Eclipse's protocol (compat row 20): bind parameters; `when` (not true:
  result null; `xmap` raises); trace lookup (a hit returns the old result); disjuncts in declared
  order; `init`; implicit instantiation of the result and `out` parameters; the trace record is
  added **before** population, so re-entrant calls see the partial result; inherited mappings;
  population; `end`; merged mappings; `where` (fatal on failure, checked on a trace hit too).
  `return` in population skips only the rest of population. Overloads dispatch on the runtime
  type, searching supertypes depth-first in declaration order.
- Traces: one append-only `Vec<TraceRecord>` (mapping, context, arguments, results, in creation
  order) plus indexes: per mapping by source key, per source object, per target object. `resolve`
  variants scan the per-object index in record order; `invresolve` uses the target index.
- Late resolve: the property assignment is queued with its owner and source, both evaluated
  eagerly, and resolved after the main entry returns, first in, first out (compat row 23).
- `deepclone` follows containment in `all_features` order and re-targets intra-copy references.

### 5.4 Exceptions, log, assert

- QVT exceptions (`raise`, fatal `assert`, stack overflow, read-only or unknown property,
  interruption) unwind through per-chunk handler tables; `try`/`except` matches by exception
  type. Other runtime errors (Java exceptions in Eclipse) do not unwind: the expression yields
  invalid and a message goes to the diagnostics sink (compat row 15). Uncaught errors end the run with a diagnostic and a QVT stack trace formatted
  as Eclipse prints it (`QvtStackTraceBuilder`; test cases compare it exactly).
- `log` and `assert` write to a `Log` sink with Eclipse's exact text format. `assert fatal`
  aborts the transformation with Eclipse's status.
- Interruption: a step budget and an optional cancel flag, checked at backward jumps and calls.

### 5.5 Determinism

The VM has no hidden nondeterminism: no hash-seeded maps, no addresses, no time. All iteration
orders are defined (compat: Eclipse's; fast: insertion order). Object ids for new objects come
from a seeded counter or generator.

## 6. Compat mode and fast mode

Mode is a compile-time policy (`trait Mode`, `Compat` and `Fast`); the VM and the runtime are
monomorphised per mode, as in SimOxide. Everything that is cheap to keep identical is identical
in both modes; fast mode differs only where emulating Java costs time or is not deterministic.

Sources for every row, with file and line: `reference/notes/eclipse-semantics.md`.

### 6.1 Order of hash-based collections

Eclipse's `Set` is a `java.util.HashSet`, `Bag` is classic OCL's `BagImpl` (a `HashMap` from
element to count), `Dict` is a `HashMap`. `OrderedSet` is a `LinkedHashSet`, `Sequence` and
`List` are `ArrayList`s. Iteration order of Set, Bag and Dict is Java's bucket order. It depends
on the Java hash codes of the elements and on the table's capacity history.

- **Value-hashed elements** (String, Integer, Long, Double, Boolean, lists, sets): the order is
  deterministic. Compat mode ports `java.util.HashMap`: `h ^ (h >>> 16)`, power-of-two tables,
  resize by splitting buckets in order, the copy constructor's initial capacity, removal, and the
  Java `hashCode` of each value kind. SimOxide's `javahash.rs` is the starting point; it handles
  insert-only maps and must be extended to a real table.
- **Identity-hashed elements** (EObjects, enum literals, tuples, Bags as elements, `Status`,
  exceptions): the order follows `System.identityHashCode`. On HotSpot this is a thread-local
  xor-shift sequence, consumed by every identity hash the thread requests, JVM-internal ones
  included. A small test shows equal values in two runs of the same program on this JDK, so a
  given Eclipse run is repeatable, but the values cannot be predicted from the transformation.
  Compat mode gets identity hashes from an `IdentityHashSource`:
  - **Tape** (for differential tests): the Java runner dumps the identity hash of every input
    and output object, keyed by a stable object key. Our run assigns the same values. This
    reproduces Eclipse's order exactly, like SimOxide's random tape.
  - **Surrogate** (default): HotSpot's xor-shift sequence, one draw per object on first use.
    Deterministic and Java-like, but not equal to any particular JVM run.
  - A lint reports when such an order reaches the output (assignment to an ordered feature,
    `asSequence`, `first`, string conversion, `log`).
- **Fast mode** uses insertion-ordered sets and maps for all elements.

### 6.2 Table

"Both" means both modes behave like Eclipse. Only the rows marked "differs" differ.

| # | Point | Eclipse | Compat | Fast |
|---|---|---|---|---|
| 1 | Set, Bag, Dict iteration with value-hashed elements | Java `HashMap` bucket order | port of `HashMap` | insertion order (differs) |
| 2 | … with identity-hashed elements | identity-hash order; repeatable per JVM run, not predictable | tape or surrogate identity hashes | insertion order (differs) |
| 3 | Java class of a collection value | matters: `objectsOfType`/`rootObjects` return a `LinkedHashSet`; QVTo's `including`, `excluding`, `flatten` turn it into a `HashSet`; `OrderedSet->union` returns a `HashSet`; `asSet` etc. return the same object if already that kind | a Java-class tag on every collection; operations follow Eclipse's class transitions | tag ignored (differs only in order) |
| 4 | Order-dependent results: `any`, `exists`/`forAll` returning invalid at the first undefined body, `sortedBy` ties, `first`, `at` on Sets | follow iteration order | follow compat order | follow insertion order (differs) |
| 5 | Integer | `Integer`, `Long` when it does not fit in 32 bits, 64-bit wrap-around; `Int / Int` is Real; `/ 0` and `mod 0` are invalid; `div` truncates; `floor`, `round` cast to `int` | both: `i64` with Java normalisation and wrapping | same |
| 6 | Integer-valued EMF attributes of type `ELong`, `EShort`, … | uncoerced Java boxes: `Long(1)` is not equal to `Integer(1)` in sets and trace keys | both: a box-kind tag on values read from such attributes | same |
| 7 | Equality | `1 = 1.0` is true, but `Set{1}->includes(1.0)` is false and `Set{1, 1.0}` has two elements (Java `equals`) | both | same |
| 8 | UnlimitedNatural `*` | the Integer -1, recognised only through the static type | both | same |
| 9 | Real to String | `Double.toString` (JDK ≥ 19: shortest round-trip digits; `1.0`, `1.0E10`) | both: `ryu` plus Java's layout (SimOxide `javafmt.rs`) | same |
| 10 | String to number | `toInteger` = `Integer.valueOf` (32-bit, no spaces), `toReal` = `Double.valueOf` (spaces, `NaN`, `1.5f`, hex) | both | same |
| 11 | Strings | UTF-16 indices and `compareTo`; `indexOf`, `find` 1-based with 0 for "absent" (spec: -1); `substring` needs `1 ≤ l ≤ u ≤ size`; `+` only between Strings | both; UTF-8 storage with an ASCII fast path and UTF-16 index mapping | same |
| 12 | Case mapping, `MessageFormat` in log levels | JVM default locale | both: fixed to `en_US` (the runner sets it) | same |
| 13 | String library bugs | null arguments become invalid; `normalizeSpace` keeps the first whitespace character; `matchBoolean` etc. take an argument; `format`, `Integer::range` missing | both | same |
| 14 | Regular expressions (`match`, `matches`, `replaceAll`, `replaceFirst`) | `java.util.regex` | both: a Java-compatible engine (`fancy-regex` plus a syntax translation layer, or a port) | same |
| 15 | Runtime errors inside expressions | any non-QVT exception becomes invalid; `Evaluation failed at '<file>:<line>' …` goes to stderr | both; the message goes to the diagnostics sink | same |
| 16 | null and invalid in conditions and logic | `if` with an undefined condition is invalid; `while`, `when`, `where`, `assert` treat non-true as false; `true and null` is invalid; short-circuit cases win | both | same |
| 17 | Uninitialised variables, properties, intermediate features | type defaults (`""`, `0`, `false`, `0.0`, empty collection), not null; intermediate contextual property initialisers ignored | both | same |
| 18 | `xcollect`, `->prop` on collections | no flattening, nulls dropped, any invalid makes all invalid; Bag for Set/Bag sources, else Sequence | both | same |
| 19 | Classic OCL bugs | Bag intersection multiplicity, `flatten` looks only at the first element, `sortedBy` evaluates its body twice and is invalid for mixed Integer/Real, `any`/`max`/`min` give null, `closure` excludes sources, `invalid.oclIsKindOf(T)` is true | both (`sortedBy` evaluates once if the body is pure) | same |
| 20 | Mapping protocol | parameters, `when`, trace lookup, disjuncts in order, `init`, implicit instantiation, **trace record added before population**, inherited mappings, population, `end`, merged mappings, `where` (also on a trace hit) | both | same |
| 21 | Trace key and lookup | key `[self, non-out args]` by Java `equals`; a later record overwrites an earlier one for the same key; nested transformations share the trace | both (key equality per row 7) | same |
| 22 | `resolve` family | records in creation order; source is `self` or the first in/inout argument of non-primitive type; `invresolve` finds only contextual mappings | both | same |
| 23 | `late resolve` | only as the direct right-hand side of a property assignment (optionally one `->asSet()`-style conversion); first in, first out after `main`; on a variable silently ignored | both | same |
| 24 | Assignment | `:=` to a collection builds a new collection of the declared kind; `coll := null` gives an empty collection; `+=` copies, except List and Dict, which are mutated; many-valued features via `ECollections.setEList`; nulls dropped from object lists | both | same (element order per row 1–2) |
| 25 | Extents | an object without explicit extent goes to the first non-`in` parameter whose first metamodel matches, else the first non-`in` parameter, else the unbound extent; roots in creation order; contained objects purged from the root list every max(150, size) additions and on queries, so a purged object that is later detached is no root | both, including the purge counter | same |
| 26 | `allInstances`, `subobjects`, `allSubobjects*`, navigation of unordered features | `HashSet` or `BagImpl` (identity order) | per 6.1 | insertion order (differs) |
| 27 | `log` text | `msg`, `msg, data: <elem>`, `Level N - msg…`; null message `<null>`; EObjects print as `pkg.Class@hexIdentityHash (attr: val, …)` | both; the hash from the identity-hash source | same format, surrogate hash (differs for objects) |
| 28 | `assert` | `ASSERT [sev] failed at (file:line)` + ` : msg`; fatal throws (code 100); error/warning add a diagnostic (code 150); `assert error` makes the result ERROR | both | same |
| 29 | Exceptions | only `raise`d QVT exceptions and fatal asserts can be caught; `return`/`break`/`continue` in `except` are dropped; `try` is null; `raise 'msg'` has no type; stack depth limit 1000 | both | same |
| 30 | Failure output | codes 120 (exception), 110 (interrupt), 140 (parameter mismatch); `Terminating execution...`, Java class and message, `\tat Module::op(file:line)` lines | both | same |
| 31 | Dict | HashMap order; iterating a Dict yields values; literals skip invalid pairs; collection-to-Dict coercion makes keys | both | order differs (row 1–2) |
| 32 | New object ids (`EcoreUtil.generateUUID`, e.g. PCM `Identifier.id` set in constructors) | time and random based | seeded generator in EMF's format; the reference runner gets the same patch for byte equality | same generator |
| 33 | EMF list semantics (uniqueness, opposites, containment moves, `setEList` moves) | EMF | both (model store contract) | same |

### 6.3 What must be identical across modes

- Programs that never observe the order of a Set, Bag or Dict: byte-identical models, logs and
  diagnostics.
- All programs: models equal under the order-insensitive comparison (multi-valued features
  matched as multisets, references by matched identity), the same diagnostics (severity, code,
  line) and the same log lines as a multiset, except `Class@hash` texts.
- The differential test runs every case in both modes and checks these rules.

## 7. Performance

### 7.1 Benchmarks

| Benchmark | What | Sizes |
|---|---|---|
| B1 suite | every passing test case, compile and run time | as in the suite |
| B2 SimuLizar rules | each rule run N times on the corpus PCM models (SimuLizar runs every rule at each trigger) | corpus models, scaled ×1, ×10, ×100 |
| B3 SPD | `MainTransformation` with each SPD example | corpus and generated PCM models |
| B4 SimpleUML→RDB | the classic example from `org.eclipse.m2m.qvt.oml.samples` | generated models with 10³, 10⁴, 10⁵, 10⁶ elements |
| B5 Ecore copy | deepclone, copy by mappings with resolve | `Ecore.ecore`, large generated `.ecore` |
| B6 micro | Set union and `including` in loops, `sortedBy`, `closure`, string building, Dict use, deep navigation | parameterised |

The Java side runs in the reference runner with `--repeat N` in one JVM (warm, after JIT
warm-up), `-Xmx2g`, niced. We report compile time and run time separately, cold and warm, and
peak memory.

### 7.2 Targets

- Compile: ≤ 10 ms for a typical rule file; Eclipse needs about 400 ms cold.
- Run, warm: ≥ 10× faster than warm Eclipse on B2, B4 and B5; never slower on any suite case.
- Memory: models in place, no copy; ≤ 2× the store's own size for traces on B4.
- Fast mode: ≥ 1.3× over compat on hash-heavy micro benchmarks; otherwise equal.

### 7.3 Key optimisations

- Names resolved at compile time: classes, features, operations, variables become ids and slots.
- Slot-indexed features in `qvto-emf` (`class.feature_index[f]`), no hash lookups on navigation.
- Specialised instructions by static type (int/real arithmetic, single vs many navigation).
- Iterators compiled inline; no closure or environment allocation per iteration.
- Copy-on-write collections; small-vector storage for short sequences.
- Interned strings with cached hashes.
- Trace indexes keyed by object handle; skip trace recording when the program has no
  resolve, re-invocation or trace output (whole-program analysis, same observable behaviour).
- Compile once, run many: a `Program` is immutable and `Send + Sync`; runs keep their state in a
  separate `Run`.

## 8. Testing

| Layer | Oracle | Where |
|---|---|---|
| unit tests | hand-written | each crate |
| model-API contract | EMF semantics as specified | `qvto-model` tests, run on every store |
| XMI round trip and canonical dump | Java dump (runner `--dump`) | `qvto-emf` tests |
| Java primitives | golden values dumped from Java (hash codes, `HashSet` orders, `Double.toString`, string ops) | `qvto-runtime/tests/golden*` |
| parser | the suite's 1 046 `.qvto` files, the parser cases' annotated spans | `qvto-syntax` tests |
| conformance | `reference/inventory/eclipse-tests.tsv`: expected models (suite comparer), asserts, logs, stack traces | `qvto-testkit`, `qvto conform` |
| differential | the Java runner on the same transformation and inputs: strict canonical model comparison, exact log, diagnostics (severity, line, code) | `qvto-testkit` |
| fast vs compat | order-insensitive comparison of both modes on every case | `qvto-testkit` |
| fuzzing (later) | parser: no panic, recovery; generated transformations and models: differential | `qvto-testkit` |

The differential harness:

1. `qvto-testkit` writes a job list (transformation, metamodels, model parameters, configuration
   properties, blackboxes).
2. The Java runner (batch mode, one JVM) runs each job and writes output models (XMI), a
   canonical dump, the log and the diagnostics.
3. `qvto` runs the same jobs in compat mode.
4. Comparison: canonical dumps equal (strict); logs equal; diagnostics equal by severity and
   line. On a mismatch the report shows the first differing object path and feature.

For deterministic Java output the runner needs patches for identity-hash-dependent orders (see
the compat table). Like SimOxide's `refsim`, it shadows a few classes on the flat classpath and
records them in `reference/runner/patches/`.

## 9. Milestones

See `PLAN.md` for the status. Each milestone ends with its exit criteria met and documented.

| | Milestone | Exit criteria | Parallel |
|---|---|---|---|
| Q1a | Model API, `qvto-emf` | all suite and corpus models load; XMI save byte-equal to EMF; canonical dump equal to Java's; contract tests | with Q1b, Q1c, Q1d |
| Q1b | Lexer and parser | all 1 046 suite files and 55 corpus files parse; expected syntax errors found; exact spans; unparse round trip | with Q1a, Q1c, Q1d |
| Q1c | Runtime core | values, collections, Java hashing and `HashSet` order, `Double.toString`, string and number ops match Java goldens | with Q1a, Q1b, Q1d |
| Q1d | Harness | Java runner batch mode with dumps; patches for determinism; testkit reads the inventory | with Q1a, Q1b, Q1c |
| Q2 | Types and sema | positive cases type-check; error cases report the annotated spans | with Q3a |
| Q3a | Bytecode and VM core | expressions, imperative OCL, helpers, queries, objects, log, assert on hand-written IR, then on sema output | with Q2 |
| Q3b | Pipeline | SimuLizar rules match the Java runner; ≥ 50 % of the 309 cases | — |
| Q4 | Full QVTo | SPD transformations match; ≥ 85 % of the cases | late part with Q5a |
| Q5a | Compat hardening | ≥ 95 % of runnable cases, strict equality with the Java runner | with Q5b |
| Q5b | Fast mode, performance | fast mode passes order-insensitively; targets of §7.2 | with Q5a |
| Q6 | SimOxide preparation | rules run on `raw::Graph` through the adapter, with journal; SimOxide work list | after Q5 |
| Q7 | Hardening | parser fuzzing, limits, docs | with Q6 |

Q1a–Q1d are independent and suit four parallel agents (two or three at a time on this machine).
Q2 and Q3a can run in parallel against an agreed typed-IR definition, written at the start of Q2.

## 10. Licensing

- Eclipse QVTo and classic Eclipse OCL are EPL-2.0. SimOxide is EPL-2.0 (with one
  GPL-2.0-only crate kept behind a feature). qvto-rs is EPL-2.0, so ported code and code shared
  with SimOxide (`javahash.rs`, `javafmt.rs`, the XMI loader) stay compatible, and the engine
  can be linked into SimOxide's EPL build.
- Where code is ported from Eclipse QVTo or OCL (library semantics, message texts, stack-trace
  format), the file says so and keeps the EPL-2.0 notice; a `LICENSES` file lists the sources, as
  SimOxide does.
- Test data copied from the Eclipse suite keeps EPL-2.0 and its origin path. The suite's
  `.qvto`, `.ecore` and `.xmi` files have no headers; the repository licence applies.
- OMG specifications are used to implement the standard; they are not redistributed.
- No GPL code. Java's `Double.toString` behaviour is reproduced from its specification (shortest
  round-trip digits, as `ryu` computes them), not from OpenJDK source.

## 11. Risks

| Risk | Mitigation |
|---|---|
| Identity-hash order cannot be predicted, so outputs that depend on it cannot match a plain Eclipse run | identity-hash tape from the Java runner for differential tests; lint for order-dependent output; the suite's own comparer ignores containment order |
| Java `HashMap` emulation gets complex with removals and treeified bins | port the algorithm, not a model of it; golden tests from Java for every collection operation |
| Java regex semantics differ from Rust's `regex` | a Java-compatible engine; golden tests; few transformations use regex |
| `Double.toString` changed in JDK 19; SimuLizar's product may run on an older JDK | target JDK ≥ 19 (the runner uses 21); a legacy formatter only if a case needs it (it must not be ported from GPL code) |
| Many tests depend on the Eclipse workspace, OSGi registrations and Java blackboxes | the harness emulates source paths, URI maps and metamodel registrations; Java blackboxes are ported; JDT/PDE cases are out of scope |
| The suite's comparer is lenient (containment order, shallow reference matching) | strict canonical comparison against the Java runner in addition |
| Hand-written parser drifts from the LPG grammar | keyword and operator tables generated from the `.gi` files; all 1 046 suite files and the unparser data as parser tests |
| SimOxide integration needs mutable models (option D, 4–8 weeks of SimOxide work) | keep the engine store-agnostic; the adapter is small; the SimOxide work is scheduled separately |
| Scope creep from rarely used features (incremental update, `.qvtox`, UML profiles) | listed as non-goals; revisit with data from the inventory |

## 12. Decisions

Decisions taken while implementing, where they refine or deviate from the sections above.

| # | Decision | Reason |
|---|---|---|
| Q1b-1 | `qvto-syntax`'s tables are generated by a Rust `.gi` reader (`crates/qvto-syntax/gen/lpg.rs`, run as `cargo run -p qvto-syntax --example gen_tables`) and the output `src/generated.rs` is committed; a test regenerates and compares. Not a `build.rs`. | The grammar sources live in the uncommitted `reference/` checkout; builds must work without it. |
| Q1b-2 | The generated tables cover more than keywords and operators: token kinds, punctuation spellings, ASCII character classes, keyword groups (single-token nonterminals such as `resolveOp`) and the binary precedence levels. The AST stores keyword choices as `TokenKind`. | No keyword or operator list is written by hand anywhere in the parser. |
| Q1b-3 | Eclipse's own parser, run parse-only (`crates/qvto-syntax/tools/oracle/ParseOracle.java` on the reference runner's jars), is the parser's test oracle; its results for the corpus, for 3 306 token mutants and for 182 edge cases are committed fixtures. | The suite's parser cases mostly test semantic errors; a parse-only oracle checks every file, every syntax diagnostic and rejection exactly. |
| Q1b-4 | Diagnostics reproduce LPG's message forms and Eclipse's texts; error recovery imitates LPG only for single-token repairs. Spans and messages agree with Eclipse on the whole corpus; on mutated input the first error agrees in about half the cases. | Exact LPG repair would mean reimplementing its backtracking repair search; only accept/reject and the positions in the suite matter for compat. |
| Q1b-5 | The unparser normalises (no comments or layout, strings re-quoted, integers printed as values); round-trip equality is equality of the span-free dump (`qvto_syntax::dump`). | Enough for round-trip tests and for generated code; a layout-preserving printer can come later from the lossless token spans. |
| Q1b-6 | Nesting is limited to 256 levels (about 1.5 MB of stack in debug builds). | Bounded stack on adversarial input (Q7 fuzzing); the corpus needs 14. |
| Q1b-7 | Corpus counts: SimuLizar has 37 `.qvto` files (a bundle directory named `*.qvto` was counted before), so the corpus has 55 files; `TestQvtParser` has 155 cases (81 annotated), the other 4 of the 159 are separate driver classes. | Counted with `tools/oracle/corpus-files.sh` and `tests/parser_cases.rs`. |

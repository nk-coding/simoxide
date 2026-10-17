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
  blackboxes are ported, the Palladio ones (`getSimulationTime`, the stereotype library) have
  their signatures in `qvto::blackbox::palladio` and the host's implementations (§12, Q4a-1).
- Executing the compiled AST XMI (`.qvtox`) of Eclipse. We compile from source.
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
- **Engine-local objects.** `Exception` and `Status` objects and transformation instances live
  in a store owned by the VM; instances of intermediate classes are store objects when the store
  can take the program's classes, else engine-local too (§12 Q4b-4). A VM object handle is
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

The trait as implemented is in `crates/qvto-model/src/store.rs`; §12 (Q1a-1 to Q1a-5) lists
where it differs from this sketch.

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

1. **Adapter** (done in Q6: `crates/qvto-simoxide`, §12 Q6-1 to Q6-8):
   - `MetaRegistry` built from `simoxide_model::meta` tables, with ids equal to SimOxide's
     `ClassId`/`FeatureId`/`EnumId`/`PackageId` (so no translation on access); the EMF facts
     the loader does not need come from extension tables SimOxide's generator now writes.
   - `impl ModelStore for SimStore<'_>` over `&mut raw::Graph`: reads and writes go to
     `Obj::slots` in place; EMF's semantics are the adapter's (ported from `qvto-emf`, not
     SimOxide's loader mutators); `create` appends an object (`raw::Graph::add_object`).
     Objects of other metamodels (PRM, SPD, metamodel objects) live in a `qvto-emf` graph of
     the session and refer to graph objects through mirrors.
   - The journal records every change, side effects included.
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
   The work list with estimates is `docs/simoxide-integration.md`.
3. **Determinism.** New PCM objects need ids. The adapter generates EMF-like UUIDs from a seeded
   generator (`IdPolicy::Uuid`) so runs are reproducible; byte equality with SimuLizar is only possible against a
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

As built in Q5b (§12 Q5b-1, `crates/qvto-cli/README.md`): `qvto bench` runs the workloads of
`crates/qvto-cli/bench/workloads.bench` (B2 on the corpus models ×1/×10/×100 and as series of 50
runs on the same model, B3 ×1/×10/×100 and a series, B4 on the sample's model and ×1/×10/×100,
B5 on `Ecore.ecore` and generated `.ecore` files of 425 to 42 500 objects, eleven B6 micro
benchmarks; B1 is `--suite eclipse|corpus`) in compat and fast mode, and with `--java`/
`--java-cold` the same workloads in the reference runner's benchmark mode, pinned to cores at
normal priority. Peak memory is not measured.

### 7.2 Targets

- Compile: ≤ 10 ms for a typical rule file; Eclipse needs about 400 ms cold.
- Run, warm: ≥ 10× faster than warm Eclipse on B2, B4 and B5; never slower on any suite case.
- Memory: models in place, no copy; ≤ 2× the store's own size for traces on B4.
- Fast mode: ≥ 1.3× over compat on hash-heavy micro benchmarks; otherwise equal.

Results (Q5b, `crates/qvto-cli/README.md`, "Results"; a loaded shared machine, ±20 %):

| | Measured | vs warm Eclipse | vs cold Eclipse |
|---|---|---:|---:|
| B2 one rule run (corpus models ×1/×10) | 0.09–0.12 ms | 4–11× | 180–400× |
| B2 ×100 containers | 0.3–0.45 ms | 2–7× | 80–160× |
| B2 series (per run, same model) | 5–35 µs | 4–18× | — |
| B3 SPD `MainTransformation` | 0.23–0.86 ms | 4.5–11× | 125–210× |
| B4 SimpleUML→RDB (sample, ×10, ×100) | 0.2 / 1.5 / 21 ms | 13× / 9× / 2.7× | 30–250× |
| B5 Ecore copy (`Ecore.ecore`, 4 250, 42 500 objects) | 0.24 / 8.9 / 181 ms | 7× / 2.7× / 2.1× | 7–290× |
| B5 deepclone, queries | | 1.4–6× | 5–100× |
| B6 micro | | 2.8–23× (sets built in loops 200–2 800×) | |
| B1 suite cases | median 5.8×, p90 14× | 0.3–270× | |
| compile | 0.1–0.9 ms per rule file, 7–8 ms SPD (18 units) | 3–20× | 100–1 000× |
| fast vs compat | 1.0–1.4× on hash-heavy workloads, else equal | | |

The 10× target holds for single runs of small rules and small models (B2 ×1, B3, B4 on the sample's
model, `Ecore.ecore`), and for cold starts by far; it does not hold for large models (B4 ×100, B5
×10/×100: 2–3×), where the time goes to model access and allocation, which a warm JVM with EMF's
dynamic objects does well too, nor for trivial suite cases (a few µs on both sides). Realistic
targets after Q5b: ≥ 5× warm Eclipse for rule-sized runs (B2, B3), ≥ 2× on large models, ≥ 100×
cold; compile ≤ 10 ms (met). The fast-mode target (1.3×) is met only on part of the hash-heavy
benchmarks (§12 Q5b-10).

### 7.3 Key optimisations

- Names resolved at compile time: classes, features, operations, variables become ids and slots.
- Slot-indexed features in `qvto-emf` (`class.feature_index[f]`), no hash lookups on navigation.
- Specialised instructions by static type (int/real arithmetic, single vs many navigation).
- Iterators compiled inline; no closure or environment allocation per iteration.
- Copy-on-write collections; small-vector storage for short sequences.
- Interned strings with cached hashes.
- Trace indexes keyed by object handle; skip trace recording when the program has no
  resolve, re-invocation or trace output (whole-program analysis, same observable behaviour).
  (Not done: trace records are kept always.)
- Q5b (§12 Q5b-3 to Q5b-8): Ecore's derived lists cached, lists read in one call, O(1) slots,
  `+=` and `sortedBy` without quadratic work, Fx hashing and dense tables in the VM, per-program
  tables, in-place loops, `LinkedMap` on a SwissTable, operation lookup by name first.
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
| differential fuzzing (Q7) | the Java runner on generated expressions and programs over generated models, minimised findings kept as regression cases | `qvto fuzz expr|prog`, `cases/fuzz.cases` |
| external corpora (Q7) | the Java runner on 1 516 real-world `.qvto` files (compile problems) and their runnable subset (outputs, logs) | `qvto fuzz corpus`, `cases/external*.cases` |
| robustness fuzzing (Q7) | none: no panic, hang or abort of parser, analyser, compiler and VM on mutated input | `qvto fuzz robust`, `crates/qvto/tests/robust.rs` |

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
| Q7 | Hardening | external corpora, differential and robustness fuzzing, limits, docs | with Q6 |

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
| Q1a-1 | `Datum` keeps Java's box classes apart: `Int(i32)`, `Long(i64)`, `Short`, `Byte`, `Char(u16)`, `Double(f64)`, `Float(f32)` (§4.2 had `Int(i64)`, `Long`, `Real`); `OwnedDatum` is its owned form. | Row 6 of §6.2: `Long(1)` and `Integer(1)` differ in Java sets and trace keys; floats print with `Float.toString`. |
| Q1a-2 | Writes return `ModelResult` (`TypeMismatch`, `IndexOutOfBounds`, `Duplicate`, `NullNotAllowed`, `Abstract`, `RecursiveContainment`, `NotChangeable`, ...) mirroring EMF's exceptions; the trait also has `remove_at`, `set_at`, `set_list` (`ECollections.setEList`), `direct_resource`, `proxy_uri`, `create_resource`, `resource_count`, `resource_uri`. `get` of a many-valued feature returns `Null`. | The VM turns Java exceptions into `invalid` plus a diagnostic (row 15); assignment to many-valued features uses `setEList` (row 24). |
| Q1a-3 | The store follows dynamic EMF (`DynamicEObjectImpl`): `unique` decides uniqueness of reference lists too, containments that resolve proxies (the default in `.ecore` files) keep a root in its resource, `set`/`unset` check changeability, the check order is index, uniqueness, then null and type. Ecore's own classes emulate their generated code (`eType`/`eGenericType`, `eSuperTypes`/`eGenericSuperTypes` views, `isSet` rules, derived features). | Eclipse runs transformations on dynamic packages for `.ecore` metamodels; verified against EMF by 300 random scripts. |
| Q1a-4 | Proxies are resolved eagerly (`ResourceSet::resolve_all` or `resolve`), not inside `get`. | `get` stays a pure read (`&self`); EMF's lazy resolution gives the same models when all proxies resolve. |
| Q1a-5 | A `Graph` owns its `MetaRegistry`, with Ecore registered first (ids `0..`); metamodels loaded from `.ecore` are linked both ways to their `EPackage`/`EClass`/feature objects (`MetaLinks`). The compiler's registry and the store's must be the same (clone the store from the compile registry; the registry only grows). | `eClass()` and reflective Ecore navigation need the objects; ids stay stable. |
| Q1a-6 | Ecore's entries come from the runtime `EcorePackage` (saved by the oracle into `crates/qvto-emf/models/Ecore.ecore`), not from the jar's `model/Ecore.ecore`. | The runtime package orders classifiers differently (classes first); canonical paths into Ecore must agree with EMF. |
| Q1a-7 | Canonical dump format: one JSON line per object in tree order, every persistent attribute (defaults included, `convertToString` text), persistent non-containment references as `<resource label>#/<root>/@f.i` paths, the set unsettable features (`crates/qvto-emf/README.md`). The Java implementation is `crates/qvto-emf/oracle/src/CanonDump.java`, for the runner's `--dump` (Q1d). | Strict, order-sensitive and ID-independent; equal dumps from both sides prove equal models. |
| Q1a-8 | The suite's lenient comparer is ported in `qvto-emf` (`compare::suite_equal`, a verdict), not in `qvto-testkit`; checked against the Java `TreeComparator` on 1 663 pairs. | It works on any `ModelStore`; the testkit uses it. |
| Q1a-9 | Metamodel resolution for test models uses a configuration shared with the Java oracle (`crates/qvto-emf/tests/data/suite.conf`): URI prefix maps, an index of `.ecore` files by nsURI (files under mapped directories by their `platform:/plugin` URIs; metamodels in the model's own directory first), aliases (`platform:/plugin/org.eclipse.emf.ecore/model/Ecore.ecore` = Ecore), and `xsi:schemaLocation`. | Replaces the bundle registrations of the Eclipse test environment, identically on both sides. |
| Q1a-10 | Two suite files (`Stdlib.xmi`, `bug449912/expected.qvtoperational`) are excluded from the EMF comparison: they are instances of the QVTo AST metamodel, whose dynamic classes extend Ecore's generated ones, which EMF itself handles wrongly. | In Eclipse these classes are generated; such models only matter for `.qvtox` (a non-goal). |
| Q1a-11 | Concurrent worktrees must not share one `CARGO_TARGET_DIR`: cargo hashes path dependencies relative to the workspace root, so the same crates of two worktrees collide. Q1a used `target/q1a` inside the shared target directory. | Observed: one worktree's build linked against the other's stale `qvto-model`. |
| Q1c-1 | `Value` is 16 bytes. `Int(i64)` is an OCL Integer, a Java `Integer` when it fits in 32 bits and a `Long` otherwise (`coerceNumber` normalisation is the representation); uncoerced EMF boxes are `Boxed(IntBox::{Long, Short, Byte, Char}, i32)` (a `Long` outside `int` range is `Int`); `EFloat` values are `Float(f32)`. Strings are `Str`, a thin (8-byte) reference-counted WTF-8 string with `String.hashCode` and the UTF-16 length computed at creation. | Java `equals` tells the boxes apart (row 6) while arithmetic needs only `i64`; WTF-8 keeps lone surrogates that `substring`, `at` and `characters` can produce. |
| Q1c-2 | The mode is a runtime property of the run's `HashEnv` (which also supplies identity hashes and Java class names), not a type parameter of the value layer. Collections choose their storage when created. | One `Value` type for both modes; the VM can still be monomorphised per mode around it. |
| Q1c-3 | The Java-class tag of a collection is its runtime `CollKind` (`Set` = `HashSet`, `OrderedSet` = `LinkedHashSet`, `Bag` = `BagImpl`, `Sequence` = `ArrayList`); QVTo `List`/`Dict` are their own values. Fast mode keeps the tags and the class transitions (row 3 says "tag ignored"); it differs only in order. | The tags also decide equality (`LinkedHashSet` vs `HashSet` is false) and result kinds, which are semantics, not order. |
| Q1c-4 | `JavaHashMap` reproduces JDK 21's `HashMap` including treeified bins (red-black trees, root moved to the front, insertion after the tree parent, movable and iterator removal, split/untreeify) and JDK 19+ sizing: `new HashSet(c)` is `HashMap.newHashMap(max(n, 12))`, `putAll`/`new HashMap(m)` pre-size with `ceil(n / 0.75)`. JDK 17's sizing (`n/.75f + 1`) is not implemented. | Tree bins are reachable with ordinary values (integers that are multiples of 1024, colliding strings); JDK 21 is the reference runner's JDK and the only one available to test against. |
| Q1c-5 | `Double.toString`/`Float.toString` follow JDK 19+ only (no legacy switch). | The pre-19 algorithm's only specification is OpenJDK's GPL source, and no older JDK is installed to test against (risk table). |
| Q1c-6 | Copy on write follows Java: an operation reuses (or mutates, when unique) its operand's storage only if Java's copy constructor would rebuild an identical table (same capacity, plain bins); otherwise it rebuilds as Java does. | Iteration order must not depend on sharing. |
| Q1c-7 | Identity hashes are memoised per `IdentityKey` (object, enum literal, type, tuple type, `invalid`, model, module instance, `BagImpl` count cell) and drawn on first use from an `IdentityHashSource`: `HotSpotXorShift` (default; HotSpot's `-XX:hashCode=5` generator), `Tape` (values from a Java run, fallback source for misses), `Sequential`. Fast mode derives them from the key. A store handle becomes an `ObjRef` (`ObjRef::host(id)` for `qvto_emf::ObjId`) through `model::datum_to_value`'s mapping; the VM implements `HashEnv` over its store. | DESIGN §6.1; keeps `qvto-runtime` independent of the store type. |
| Q1c-8 | Values Java hashes by their own identity although we have no object for them (boxes, strings, collections) order as `1` (and `null` as `0`) in tree-bin tie-breaks; `BagImpl.hashCode` includes identity hashes of its count cells; `Datum::Other` (BigInteger, BigDecimal, Date) becomes a `Str`. | Java consults these identities only for `>= 9` equal-hash non-comparable keys in one bin or against `null`; Bags inside hash collections and big numbers in attributes are not reproducible or not used by the corpora. |
| Q1c-9 | `Character` data (classification of code units, decimal digits, simple and full case mappings) is generated from JDK 21 into `src/java/chars_data.rs` (`tools/oracle/gen-chars.sh`), not taken from Rust's `char`. `toLowerCase` decides final sigma with Unicode's Final_Sigma context instead of Java's word boundaries (agrees on all tested strings). | Java's Unicode version and its own definitions (`isWhitespace`, `Character.digit` of non-ASCII digits in `Integer.parseInt`) differ from Rust's. |
| Q1c-10 | `intersection`'s `Collections.emptySet()` and shared `BagImpl.EMPTY_BAG` are fresh empty `HashSet`/`BagImpl` values; `Sequence{a..b}` (`IntegerRangeList`) is an `ArrayList`. | Values are immutable here, so the Java aliasing of these constants is not observable. |
| Q1c-11 | The Java oracle (`crates/qvto-runtime/tools/oracle`) runs JDK classes and Eclipse OCL's `CollectionUtil`, `BagImpl`, `ObjectUtil`, QVTo's `DictionaryImpl` and the classic OCL evaluator (long integers on) on generated requests; `tests/differential.rs` compares, `tests/golden/` keeps samples for JDK-less runs. QVTo's thin handler wrappers (LinkedHashSet to HashSet before `including`, ...) are replayed in the oracle, not called. | The handlers need a QVTo evaluation environment; their logic is three lines each. |
| Q1d-1 | The case file (`crates/qvto-testkit/cases/*.cases`, key/value blocks) is the job list both sides read; `tools/gen_cases.py` generates the Eclipse one from the drivers' Java sources and it is committed (§8 had the testkit write job lists per run). | One reviewed description per case; the Java runner needs no Rust and the Rust side no Java. |
| Q1d-2 | The Java runner emulates `TestQvtStandaloneExecutor` inside an OSGi-equivalent environment: EMF's standalone `ExtensionProcessor` over the test bundle's and the samples plugin's `plugin.xml`, imports resolved against the bundles' source containers (a `UnitResolverFactory` put first into QVTo's registry by reflection), the test bundle's Java classes compiled from the checkout, including the blackboxes the suite compiles in its workspace (bug577992, bug507955), which makes those cases runnable. | The suite runs under OSGi in CI; a plain standalone run fails about 40 cases on imports alone. |
| Q1d-3 | No runner patches for determinism (§8): fixtures keep HotSpot's default identity hashes (Eclipse-like orders); each case runs on a fresh thread; the tape records the hashes and the thread's seed, from which `idhash::XorShift` recovers the whole sequence; outputs that differ in reruns (with `-XX:hashCode=2`, `-XX:hashCode=3` and the default) are marked unstable and not compared strictly. | The tape makes the reference run replayable; `-XX:hashCode=3` would be reproducible but not Eclipse-like. |
| Q1d-4 | Canonical dumps of outputs: proxies resolved, each out/inout extent moved into a resource `extent<i>.xmi`; `dump models` adds the other model resources (sorted by file name) for rules that change models through `in` parameters. QVTo's internal extent resources (`extent:/N`, holding created objects without a container) show up as labels and must be reproduced by the engine. | Deterministic on both sides; observable EMF state as QVTo leaves it. |
| Q1d-5 | Fixtures: one committed bundle per suite of the small per-case files (result, dump, log, stack, problems; 0.7 MB each); tapes and XMI in `target/qvto-testkit/java` (regenerable with `qvto conform record`, about a minute; `qvto conform java --save-xmi` for the cache alone). | Fast tests without Java; the large parts are cheap to regenerate. |
| Q1d-6 | The `Engine` trait, its `Outcome` and the verdicts live in `qvto-testkit`; an engine plugs in through an adapter in the CLI. Two verdict levels: the drivers' checks (Eclipse's judgement, the Q3/Q4 targets) and strict equality with the Java result (Q5a). | The facade stays free of test concerns. |
| Q1d-7 | SimuLizar/SPD cases use PCM, SimuLizar and SPD metamodels as dynamic EMF from `.ecore` on both sides (SimuLizar runs generated code: new objects get no generated ids); runtime measurements are synthesized files with `RuntimeMeasurement` roots; the `ProfilesLibrary` blackbox the SPD helpers import is a stub; `pcm-setup.sh` also extracts the FeatureConfig, FeatureModel and completions metamodels the rules declare. | Same model semantics on both sides; the real blackboxes need EMF Profiles and the generated PCM. |
| Q2-1 | `qvto-sema` is a structural port of Eclipse's analyser: one function per visitor method of `QvtOperationalVisitorCS`, `AbstractOCLAnalyzer`, `QvtOperationalValidationVisitor`, the environments' lookups and `TypeCheckerImpl`, with their quirks (an assignment with an unresolvable left side is still built; a switch's type is the common supertype of the last two alternatives; an unparseable source takes the enclosing node's positions; infix operators have no accessor, so `coll = x` is no implicit collect). | The goal is identical diagnostics; a fresh design reproduced the messages but not when and where Eclipse reports them. |
| Q2-2 | Diagnostics have no codes (§3.2 planned stable codes): the 444 messages of QVTo's and OCL's `messages.properties` are generated into `messages_data.rs` and bound with `NLS.bind` semantics (`''`, quoted literals, an unmatched quote dropped). Positions are UTF-16 offsets/lengths on request (`Problem::utf16`). | Equality with Eclipse is checked on message text; codes would only duplicate it. |
| Q2-3 | Where the `qvto-syntax` AST's spans differ from Eclipse's CST nodes the analyser recomputes them from the source text instead of changing the parser: declarations start at `helper`/`query`/`constructor` (mappings: the direction keyword, else `mapping`) and end with the signature, module headers start at `transformation`/`library`, `else` parts at `else`, `src.map f()`/`src.resolve(...)` nodes after the source, empty mapping bodies get LPG's computed offsets. | The parser's spans are Q1b's verified contract; these are CST conventions only the analyser's messages expose. |
| Q2-4 | The host supplies what Eclipse takes from the platform: the package registry (`MetaRegistry`, with `scan_metamodel_uris` for on-demand loading), the metamodels' `EOperation`s (`MetaOperation`, `compile_with_operations`; the registry has no operations; the corpus harness reads them from the loaded `.ecore` objects), Java blackbox units as signature data (`BlackboxUnit`, `UnitResolver::blackbox_units`). | Keeps `qvto-sema` free of EMF loading and Java; the same data feeds tests and the CLI. |
| Q2-5 | The oracle is Eclipse's full compile of the whole corpus (`crates/qvto-sema/tools/oracle`, the reference runner's jars): every problem of every unit and its imports for all 1 102 files is a fixture; the test gates on the number of files that match exactly and on IR consistency (`irck`). The Java blackbox signatures are dumped likewise (`qvto-types/data/blackboxes.tsv`). | Covers far more than `TestQvtParser`'s counts and annotations, and the SimuLizar/SPD files. |
| Q2-6 | The registry has no `EAnnotation`s: `ExtendedMetaData` base types of nullable enum wrappers are recognised by EMF's naming convention (`<E>Object` data type in the enum's package). Intermediate class features of types the registry cannot represent get placeholder data types in a `_qvto_types` package; their declared types travel in `IntermClass::feature_types`. | Only these two uses of annotations/foreign types occur in the corpus. |
| Q2-7 | Tuple types are reused when part names and OCL part types agree (`Tuple(name : EString)` and `Tuple(name : String)` are the same, the first created wins), as Eclipse's type resolver cache does; all other types are interned structurally. | Eclipse's messages print the cached tuple type. |
| Q2-8 | `ModuleId(0)` is QVTo's library module `Stdlib`; its operations are `StdOp`s, not program operations (`ir.rs`, `CHANGES.md`). | One table (`StdOp`) for the analyser and the VM. |
| Q3a-1 | Instructions are a Rust `enum` of at most 8 bytes (opcode + three `u16`, or a register + `u32`); operands that do not fit (types, names, flags, feature ids beyond `u16`, jump targets of failure paths) are in a per-chunk site table. Constants are a program-wide pool of `Const`s turned into `Value`s at the start of a run, so a `Program` is `Send + Sync`. | §5.2's format without a side table for every wide operand; `Value` (with `Rc`) cannot be in a shared program. |
| Q3a-2 | A mapping's sections (`when`, `where`, `init`, population, `end`) are separate chunks on one frame. A `return` at the top level of an `init`/`end` statement ends that statement, in an `except` body it leaves the `try` (Eclipse drops the result in both places); in population it ends the population only. | Eclipse evaluates sections statement by statement and ignores their `OperationCallResult`s there. |
| Q3a-3 | The VM is not monomorphised per mode: the mode is a runtime property of its `HashEnv` (`RunEnv`), as Q1c-2 already made the value layer mode-agnostic. | One code path; the mode only matters inside collections and identity hashes, which take `&dyn HashEnv` anyway. Revisit in Q5b if profiles show the indirection. |
| Q3a-4 | Java runtime exceptions inside expressions do not unwind: the instruction yields `invalid` and the message goes to `Vm::errors` (Eclipse's `Evaluation failed at ...` on stderr). Eclipse turns the *enclosing* expression of the throwing node into `invalid`; for nested expressions the VM yields `invalid` at the innermost instruction instead, which propagates through operations the same way except where an enclosing construct would have caught it later. | Expression-granular catch boundaries would need a handler per IR node; the observable difference needs a Java exception under a construct that ignores `invalid` operands. |
| Q3a-5 | Instances of QVTo's library classes (`Exception` and subclasses, `Status`) and of intermediate classes are engine-local objects (`ObjRef::local`) with one slot per feature in the VM; the host store never sees them. Contextual intermediate properties are a side table keyed by `(object, property)`. | DESIGN §4.2; host stores with fixed metamodels (SimOxide) cannot hold them. |
| Q3a-6 | Eclipse's current instruction pointer (set on entering every AST node by the interruptible visitor, reset to the call after an operation call returns) is computed at compile time: each instruction records the start offset of the last node entered in evaluation order. Stack-trace lines and assertion locations come from it. | Stack traces are compared exactly (`TestStackTrace`, failure logs); no runtime cost. |
| Q3a-7 | `matches`, `replaceAll`, `replaceFirst` and QVTo's `match` use the `regex` crate with Java's `matches` anchoring and replacement syntax (`$n`, `\x`); patterns `regex` rejects (look-around, back references, possessive quantifiers) yield `invalid`. | Row 14's Java-compatible engine is later work; the suite's patterns are simple. |
| Q3a-8 | Fast paths that keep Eclipse's results: `+ - * < <= > >= = <> not` have instructions with Integer/Real fast paths; `x := x->op(...)` on a collection variable moves `x` into the call (`Take`), and `:=`/`+=` reuse an unshared collection when Java's copy would rebuild the same table (`ops::fresh_add_equivalent`, `copy_as`; the copy-equivalence checks are O(1) through a bucket-length bound in `JavaHashMap`). Deviation: elements of a value whose static element type equals the variable's are not re-converted on `:=` (Eclipse's `convertNumber` would truncate a Long element of a Set(Integer) to `int`). | Building collections in loops is O(n) instead of O(n²) (micro benchmark `set`: 1.8 ms vs 5.1 s in Java). |
| Q3a-9 | Identity hashes from the Java run's tape (the conformance adapter): input and metamodel objects by their keys, created objects by their keys in the outputs of a previous run (the case runs up to three times until the created objects stay the same), other draws from the Java thread's recovered xor-shift sequence. | Q1d's tape keys created objects by their final position, unknown when the hash is drawn. |
| Q3a-10 | `sortedBy` evaluates its body once per element (Eclipse twice); `collect` flattens collection bodies recursively (classic OCL's `CollectionUtil.flatten` decides by the first element only). | Identical for pure bodies and homogeneous nestings, which is what the corpora have.  (`sortedBy`: superseded by Q4b-8.) |
| Q3a-11 | Limits of the bytecode: registers, sites, loops and handlers per chunk and operation ids in `Call` are 16-bit (a compile error beyond). | Instruction size; real transformations are far below. |
| Q3a-12 | Not in Q3a (PLAN Q4): blackboxes, `Model::copy`/`createEmptyModel`, EOperations with OCL bodies, `unlink`, incremental update traces, `transform()` of blackbox transformations, per-instance properties of nested transformations (`new T(...)` instances share the module's properties). The VM raises a `QvtRuntimeException` naming the construct. Nested transformations (`new T(...)`, `transform()`, `Status`) are in. | PLAN's split; nested transformations were cheap once extents were bound per module.  (Done in Q4a/Q4b; `unlink`: see Q4b-10.) |
| Q3a-13 | Extents follow `InternalTransformationExecutor` and `createEvaluationResult`: every extent's contents are taken at the end (purge, detach from `extent:/N`); the outputs are the non-`in` parameters that are not generated for a legacy `main(in x : T)` signature (named `$x`; unnamed declared parameters are `$<index>`), filling the non-`in` parameters in order; none and an object result of the entry: that object alone (Eclipse's compatibility extent); a failed execution writes nothing back (the caller's extents keep what they were given). Other transformations' model parameters are bound as `ModelParameterHelper` does (a main parameter of compatible direction and metamodels, else of compatible direction, else an extent of their own); an explicit extent of an unbound parameter is the unbound extent. | Observed on the suite (legacy mains, imported transformations, failed runs). The IR has no flag for generated parameters yet (the `$` name is the convention). |
| Q3a-14 | Where the analyser's IR is incomplete, the VM computes Eclipse's virtual tables itself (contextual operations with the same name, kind and parameter types) when `Operation::vtable` is empty, and dispatches `null` as OclVoid. The branch carries two patches of `qvto-sema`: a temporary copy of the model parameters into the IR (`analyzer.rs`, marked `Q3a (temporary)`, to be dropped when the analyser fills them) and a fix of `SourceId`s (`module.rs`, `analyze_unit`: units are analysed imports first, so the source goes to slot `unit + 1` instead of being pushed). | Keeps the VM's dependence on the analyser at the IR contract; reported to Q2.  (Superseded by Q4b-1: the analyser fills the IR, the VM has no tables of its own.) |
| Q3a-15 | `raise 'msg'` is `raise Exception('msg')` in Eclipse (`AbstractQVTParser.createRaiseExpCS` adds the type), not a raise without type and argument as `reference/notes/eclipse-semantics.md` §9.1 and row 29 say; the IR currently drops type and argument (Q2). | Eclipse's source; test `bug419299`. |
| Q3a-16 | Values stored in module properties, configuration properties and intermediate class features go through `callSetter`'s rules (`assign` on the declared OCL type, then `eSet`/`eUnset`): a null collection becomes an empty one, an invalid value unsets (null) on `:=` and is ignored on `+=`; configuration values are parsed as `ConversionUtils.createFromString` (collections `[a, b]` and dictionaries `[k=v]` with escapes, nested). Intermediate features use `IntermClass::feature_types` (single-valued features of collection types, `createInitialValue`). Contextual intermediate properties can be owned by objects, module instances and model extents; `clone`/`deepclone` copy them (`IntermediatePropertyCopier`). | Eclipse's `doInitModule`, `IntermediateClassFactory.doInit`, `ElementOperations`; cases `bug267917`, `bug417996`, `numconversion`, `bug254962`, `fqnMainCalls_*`, `intermediateprop_import`. |
| Q3a-17 | Instances of intermediate classes are engine-local without EMF semantics: no opposites, no containment, no `eContainer` (case `intermProperties`). `qvto-emf` accepts enumeration literals in attributes of `Enumerator`/`Object` data types (EMF's nullable enumerations, `nullableEnum`). | Local objects were enough for Q3a's cases; a store-backed intermediate class (dynamic `EClass` in the store's registry) is the Q4 alternative.  (Superseded by Q4b-4.) |
| Q4a-1 | Blackboxes are native Rust in two halves. Signatures are data: rows of the `blackboxes.tsv` format (`qvto::blackbox::Catalog`), made from Eclipse for Java classes (`qvto-types/tools/oracle/BlackboxDump.java`) or written by the host; a `BlackboxResolver` serves them as units (registered, then by class name). Implementations are a `qvto_vm::Natives<S>` registry given to a run (`Vm::set_natives`): closures over `&mut Vm<S>` and the call (`self`, arguments) by implementation key (`Operation::blackbox_impl`; for Java the key `Class#method(params)` derived from `Method.toString()`, which is also the text of invocation errors), blackbox transformations by name, other metamodels' EOperations by `nsURI#Class.op`; per-run library state by Rust type (Eclipse: one Java instance per class and transformation). The VM follows Eclipse's protocol: library operations as OCL calls (no frame, `invalid` receiver gives `invalid`, `map` without the mapping protocol, never redirected to an overriding operation of an extending module), `blackbox` operations declared in QVTo on a frame with the mapping protocol for `blackbox mapping`, `invalid` arguments as `null`, exceptions as `QvtRuntimeException` (`Java method invocation error '<method>'`), cancellation as interruption. | DESIGN §2.3: no JVM; the host API must serve SimOxide (`getSimulationTime`, stereotypes) as well as the Eclipse ports, and signatures Eclipse derives by reflection cannot be derived from Rust closures. |
| Q4a-2 | IR: `Module::blackbox` is also set for modules of Java units; `Operation::blackbox_impl` is also set for a `blackbox` operation declared in QVTo when the analyser finds exactly one implementation (Eclipse binds at run time with the same registry; none or several give Eclipse's runtime error). | The VM needs the binding; the analyser already matched for its warnings. |
| Q4a-3 | Java values of blackboxes: `BigInteger` is an OCL Integer (`i64`), `BigDecimal` a Real, `Date`/`Time`/`XMLGregorianCalendar` their text; attributes of `EBigInteger`/`EBigDecimal` types read as Integer/Real and are written with `NumberConversions`' text. Data types whose instance class is `java.lang.Number`/`Object` accept boxes in the store. | Q1c-8 kept big numbers lexical, which made `numbers.bigInteger100 = createBigInteger('100')` false; no corpus value exceeds `i64`/`f64`. |
| Q4a-4 | Ecore's EOperations are implemented in the VM (`meta.rs`) from EMF's semantics, reading the metamodel objects through new optional `ModelStore` methods (`classifier_object`, `object_feature`, ...); the store's derived Ecore features do the work (`eAllStructuralFeatures`, ...). `eResource()` yields the resource URI (no resource values). Other metamodels' EOperations without OCL body are the host's (`Natives::eoperation`, what generated code does in Eclipse). OCL bodies and OCL derivations are not supported yet. | Generic over any store; only `oclannotation` needs OCL bodies. |
| Q4a-5 | `MetaOperation` moved to `qvto-model`; `qvto_emf::operations::meta_operations` extracts EOperations (from the test harness); hosts compile with `qvto::compile_with_operations` after `ResourceSet::ecore_resource()` (EcorePackage is always registered in Eclipse). | The CLI and the adapter need them; `qvto-emf` must not depend on the analyser. |
| Q4a-6 | Conformance adapter: blackbox units from `blackboxes.tsv` by the case's environments as the oracle harness picks them; inputs through `getResource` (a registered metamodel file given as input is the same resource, as in the Java runner); registry extras of the Eclipse environment (the QVT metamodel's upper-case key, the EMF jars' XMLType, XMLNamespace, Change from the analyser's fixtures); metamodels named by file location registered under that location. The Eclipse test bundle's blackboxes are ported in `qvto-cli/src/bbox.rs`, QVTo's legacy libraries in `qvto::blackbox::stdlib`, Palladio's signatures in `qvto::blackbox::palladio`. | Identical setup on both sides; the libraries are test code, the legacy libraries part of QVTo. |
| Q4a-7 | `Model::copy()` makes an extent of `EcoreUtil.copyAll` of the roots with the source's parameter (so an `in` copy is read-only, as in Eclipse) and model type; `createEmptyModel()` an extent without parameter that records its model type. `allInstances()` covers the current transformation's parameter extents and the unbound extent only (`QVTExtentMap`). | Eclipse's `ModelInstanceImpl.copy`, `ModelOperations`, `QVTExtentMap.getExtents`. |
| Q4a-8 | Extents record roots that blackboxes add to their resources (the in-memory `extent:/N` and the initial objects' resources; `ResourceModificationAdapter`) and include them in their contents. Only additions through the native API are seen (the store has no notifications); the VM's own additions go through the extents anyway. | UML stereotype applications (`uml2_stereotypeApplication`). |
| Q4b-1 | The analyser fills `Module::model_params` (with `ModelParam::generated` for the parameters of a legacy `main(in x : T)`; the VM's `$name` convention is gone) and `Operation::vtable` (Eclipse's `VirtualTable` from `doFindCollidingOperation`). The VM dispatches every call of a contextual operation through the table (`isOverloadableMapping`, also for `x.M::op()`), skips operations that another one of the table overrides (`filterOverriddenOperations`), and applies module overriding only to calls with `isVirtual` (not module-qualified). | Eclipse's evaluator (`findOperationByActualSourceType`, `getOverridingOperation`); the VM's own tables differed for unrelated context types and for access-imported modules. |
| Q4b-2 | `List{1}->insertAt(2, 2)` and `Sequence/OrderedSet::insertAt` resolve to the same operations as in Eclipse (checked with the probe of Q4b-11): the `bug467600_*` failures were run-time semantics. `ListOperations`' mutators (`add`, `insertAt(o, i)`, `remove`, `removeAll`), typed `OclVoid`, return the list; classic OCL's two-argument operations (`insertAt`, `subSequence`, `substring`, ...) are invalid for an undefined source or an invalid argument. | `ListOperations` handlers, `EvaluationVisitorImpl` "ternary operations". |
| Q4b-3 | Diagnostics now equal Eclipse's on all 1 102 corpus files: problems of OCL's `ValidationVisitor` (`validatorError`) on nodes inside `object` bodies are dropped, because Eclipse analyses object bodies in a child environment whose AST-to-CST map the validating module environment does not see and drops validator problems without a position; `object` without a type parses to an error node (LPG's repair leaves no object expression); a `.` substituted by repair ends the qualified name (`a-b` is `a`); a Java unit that fails with the case's registry is loaded again with the importing module's packages when an implementation of a `blackbox` operation is looked up (`JavaBlackboxProvider` infers types from the load context's registry keys and caches the first load; `blackboxes.tsv` rows `<unit>#with=<uris>`). | Found with the probe of Q4b-11 (`ValTrace`, `WalkTrace`); `isMDTOCLCompatibilityFalseProblem`, `BlackboxProvider.getBlackboxCallHandler`. |
| Q4b-4 | Instances of (non-exception) intermediate classes are objects of the model store when it accepts the program's registry (`ModelStore::extend_meta`, which `qvto-emf` implements; a store with fixed metamodels refuses and the VM keeps them locally as before): their references have EMF's semantics (containment, also of model objects, which then leave their extent; opposites set on one side as QVTo declares them, which `qvto-emf` now follows like EMF; container references). Their attributes (OCL-typed) and `static` features (`IntermClass::static_features`, one value per feature, initialised by the first instance) are kept by the VM; instances live in an intermediate extent per transformation (not searched by `allInstances`, which takes the current transformation's parameters and the unbound extent) and print as their class name. Initialisation follows `doInstancePropertyInit` (every feature in `eAllStructuralFeatures` order). | `IntermediateClassFactory`, `getDefaultInstantiationExtent`, `QVTExtentMap`; case `intermProperties`; EMF behaviour checked with `crates/qvto-emf/tests/data/OneSided.java`. |
| Q4b-5 | OCL bodies of operations and OCL derivations of features of dynamic classes (`http://www.eclipse.org/2007/OCL` annotations, `body`/`derive`) are read by the host (`qvto_emf::operations`), analysed once after the units in the main module's environment with `self` and the parameters as variables (implicit `self` allowed, problems not reported, a body with errors is `invalid`) and evaluated as `OCLAnnotationSupport` does: the operation that overrides the called one for the instance's class (`resolveRecursiveDepthLast`) runs if it has a body, with Eclipse's binding quirk (parameters named as the called operation's get the first argument). | `QvtOperationalEvaluationVisitorImpl.call`, `getOperationBody`, `getPropertyBody`; case `oclannotation`. |
| Q4b-6 | Incremental update: the host reads a `.qvtotrace` model (`qvto::trace::read_incremental_trace`) into `qvto_vm::IncrementalTrace`; a mapping call without a record in the run's trace takes the results and `out` values of the last input record with the same mapping (name and module), context and in-parameters if the recorded types match (`checkIncrementalResultMatch`), puts them into their extents and runs its body. Recorded output objects are detached from their resources at the start. | `TraceUtil.getIncrementalTraceRecord`, `fetchIncrementalResultFromTrace`, `resolveTraces`; case `bug463572`. |
| Q4b-7 | Each `new T(...)` instance keeps its own module property values (swapped in while it is current); `t.p` on a transformation instance is `ExprKind::ModuleProp` (reads the instance's value). The modules that a nested transformation imports still share one instance. | `ModuleInstance` per transformation instance (checked with the reference runner). |
| Q4b-8 | `sortedBy` evaluates its body a second time for every element in compat mode when the body has side effects (a call of a non-library operation, an imperative expression, a mutating library operation), and sorts by the second values, as classic OCL's `evaluateSortedByIterator`; fast mode evaluates once. | Observable through logs and property changes; a pure body gives the same result either way. |
| Q4b-9 | Harness: dumps compare `extent:/N` labels renumbered by first appearance (Eclipse's counter is JVM-wide); the reference runner runs cases on threads with a 512 MB stack, so QVTo's own depth limit (1000) is reached before a JVM `StackOverflowError` (a default stack overflows at about 740 calls), and `stacktrace_testStackOverFlow` was re-recorded; `qvto conform record --only/--prefix` merges into the fixture bundle and the cache. | Results of Eclipse that depend on the test JVM rather than on QVTo. |
| Q4b-10 | `unlink` needs nothing: Eclipse's grammar has no rule for it and `visitUnlinkExp` is an empty stub. | QVTo 3.11 sources. |
| Q4b-11 | `crates/qvto-sema/tools/probe/probe.sh` asks Eclipse about single files: compile problems, the AST of the operations, the referred operation of every call. | Faster than reading the analyser's sources when a behaviour is unclear. |
| Q6-1 | The adapter is a crate in qvto-rs, `crates/qvto-simoxide`, outside the qvto-rs workspace (`exclude`, a workspace of its own) and depending on SimOxide's `simoxide-model` by path; SimOxide is unchanged except for two additive commits on its branch `qvto-adapter`. | The adapter's tests need qvto-rs's corpus, testkit and Java fixtures; qvto-rs must build without a SimOxide checkout; SimOxide stays free of the engine until it decides to integrate (then it depends on this crate behind a feature, or moves it). |
| Q6-2 | SimOxide's generator writes extension tables (`meta/generated_ext.rs`: `upperBound`, ordered, unique, changeable, unsettable, interface, complete `eSuperTypes` incl. Ecore and generic supertypes, data types, classifier order, `EOperation` signatures, package locations) instead of changing its main tables; `generated.rs` regenerates byte for byte unchanged. | The registry needs these facts (312 PCM features are unordered, so navigation types depend on them); additive, no effect on SimOxide's loader. |
| Q6-3 | The registry puts SimOxide's packages first (ids equal) and Ecore after them: `qvto_emf::Graph::with_registry` (`EcoreIds::class_start`/`feature_start`); Ecore supertypes are patched after Ecore is registered and the classes finished again (`MetaRegistry::unfinish_class`). | DESIGN §4.4's "ids equal to SimOxide's"; `register_ecore` asserted an empty registry before. |
| Q6-4 | Programmatic registries get metamodel objects by `ResourceSet::materialize` (EPackage, EClass, EAttribute, EReference, EEnum, EDataType, EOperation objects from the registry, linked both ways, under the packages' `platform:/plugin` locations). | The Q1a open issue: `eClass()`, Ecore's reflective operations and `.ecore` files that refer to SimOxide's packages (SPD, the PRM metamodel) work without the `.ecore` files at run time. Checked: materialized, saved and registered again gives the original registry. |
| Q6-5 | Two homes in one store: instances of SimOxide's classes in the `raw::Graph` (handle = `ObjId`), all others in a `qvto-emf` graph of the session (handle = `2^30 + id`), with mirrors (proxy objects) for references from there to the graph. References from the graph to the other home and containment/opposites across homes are `Unsupported`. The engine's `extent:/N` resources are the adapter's own and may hold roots of both homes. | SimOxide's tables cannot describe other metamodels (`ClassId::def()` is static), and the PRM, SPD models and intermediate data must not appear in SimOxide's graph; no corpus metamodel needs the unsupported cases. |
| Q6-6 | The graph part implements EMF's semantics itself (a port of `qvto_emf::Graph`'s operations onto `Obj::slots`), driven by the registry only; SimOxide's loader mutators and `ClassId::def()` are not used. Slots are present exactly when EMF's `eIsSet` holds after a write; values keep SimOxide's representation; a string stays a `Value::Str` whatever the attribute's data kind (as `qvto-emf` keeps the given box). | The loader mutators lack uniqueness, roots, `isSet` and changeability; registry-driven code can run the contract suite on `raw::Graph` storage with the contract metamodel, and SimOxide's own reads stay consistent. |
| Q6-7 | The journal records effective changes for graph objects (both ends of opposites, old containers, roots) as EMF notifications would, and the operations as called for objects of other metamodels. | Syncers need every change of a PCM object; the other home holds no PCM objects. |
| Q6-8 | New objects get no `Identifier.id` by default (dynamic EMF, the corpus reference); `IdPolicy::Uuid { seed }` gives EMF-format UUIDs from a seeded generator (generated PCM code). `getSimulationTime`'s registration no longer requires a `'static` store (`SimStore` borrows the graph). | Equality with the Java fixtures and reproducible SimOxide runs; the store is a per-run view. |
| Q5b-1 | The benchmark suite is a workload file in the case format (`crates/qvto-cli/bench/workloads.bench`, `bench <key> <value>` lines: group, series, rounds, runs) with models generated deterministically into `target/qvto-bench/gen` (SimpleUML: the sample's classes ×N; Ecore: 5N packages of 10 classes; PCM: the corpus directories with (N−1)·k more resource containers, assembly and allocation contexts). The Java runner has a benchmark mode with the same protocol (`qvto-run --bench`, `Bench.java`): K compiles, then fresh runs (inputs loaded before each run, not timed; until N runs or B ms, at least 5) or series runs (one load, N runs on the changing models, the last of `rounds` rounds counts). Reported: median and p10–p90 of the second half (fresh) or of the round (series), the series total, compile medians; Java cold = a fresh JVM per workload. | Realistic workloads (SimuLizar rules during a simulation are series of runs on a mutating model), both sides measured identically, nothing large committed. §7.1's B1 is `--suite eclipse|corpus`. |
| Q5b-2 | Profiles come from `pprof` (SIGPROF sampling, feature `profile` of `qvto-cli`, `qvto bench --profile FILE.svg`: flame graph plus self/inclusive table of the samples inside the timed runs, `QVTO_PROFILE=compile` for compiles). | `perf` events are not available to users on this machine (`perf_event_paranoid` 4, no root). |
| Q5b-3 | Store: `ModelStore::get_list` reads a whole list in one call (default: `len` + `get_at`). `qvto-emf` caches Ecore's derived lists per (object, feature) with a generation counter that every write to an object of Ecore's classes bumps (`Graph::touch`), builds the `eAll*` lists from the supertypes' cached lists, reads `eContents` from the slots, answers `contains` of containment lists from the child's container (`EObjectContainmentEList.contains`). `MetaRegistry::slot` is O(1) for features whose slot is the same in every finished class (`fixed_slots`; single inheritance), recomputed by `finish_classes`, dropped by `unfinish_class`. | Q1a's note (derived lists recomputed per `get_at`) made `eAllStructuralFeatures` quadratic per class and uniqueness checks quadratic per list; EMF caches the same lists in `EClassImpl`. |
| Q5b-4 | `+=` of objects to a many-valued feature adds the values (`add` per value) instead of `ECollections.setEList` with a copy of the list plus the values. | Identical: with the current list as prefix, `setEList` performs exactly these `add`s; a failing `add` leaves the state Eclipse's clear-and-re-add reaches. Building a list in a loop was quadratic (`b6_objects` 497 → 9 ms). |
| Q5b-5 | `sortedBy`'s "last key of equal elements" (a `HashMap` keyed by element in Eclipse) is computed with hashes consistent with Java `equals` that draw no identity hashes (atoms; collections and tuples keep the pairwise scan); Set sources skip it. | It was a pairwise scan (`b6_sortedby` 1.9 s → 12 ms); drawing identity hashes there would change compat mode's draw order. |
| Q5b-6 | VM: `FxHashMap`/`FxHashSet` (`qvto_runtime::fxhash`) for internal maps; identity hashes of host objects in a paged table by handle; the primitive table built once per process; per-program tables in `Program` (final overriding operation, library classes) instead of per run or per call (the overriding chain was a scan of all operations per call); constants converted on first use; one reused argument buffer for library calls; loops over Sequences and OrderedSets read the value in place (`IterSlot::Coll`) instead of copying its elements; the copier's map (`clone`/`deepclone`) hashed. | Per-run and per-call overheads dominate small rule runs; all are semantics-free. |
| Q5b-7 | `LinkedMap` (Java's `LinkedHashMap`/`LinkedHashSet`: OrderedSet in both modes, every hash collection in fast mode) is a `hashbrown::HashTable` of indices into entries kept in insertion order. It still hashes keys with `java_hash` on every `put`, `get` and `remove`. | Java's bucket structure is not observable through a linked map; hashing as before keeps the identity-hash draw order (compat). The JDK differential tests of `qvto-runtime` pass unchanged. |
| Q5b-8 | The analyser's operation lookup (`findMostSpecificOperationMatching`) filters the candidate list by name before removing duplicates. | Same result (equal candidates have equal names, order kept); Eclipse's algorithm builds the list of all operations of the type for every call (SPD compile 9.3 → 7.5 ms). |
| Q5b-9 | The VM stays mode-agnostic at run time (Q3a-3 confirmed): the profiles show no cost in mode checks; fast mode's differences are inside the collections and identity hashes, behind `&dyn HashEnv`. | Monomorphising per mode would double the interpreter's code (`run` is 53 KB) for no measurable gain. |
| Q5b-10 | Fast mode differs from compat only in order (Set, Bag, Dict in insertion order; identity hashes from the key, so `Class@hash` texts differ) and evaluates an effectful `sortedBy` body once. Outputs that depend on the iteration order of a Set, Bag or Dict may differ: `any`, `first`/`at` and `asSequence` of sets, logs of sets, string conversions, ties in `sortedBy`, loops over unordered features (PCM has 312: `allocationContexts_Allocation`, ...) whose effects depend on the order (`b2_rc_addClonedServer_x10` creates a connector per allocation context after the client and the server were seen: 6 log lines in compat, 20 in fast). All are valid QVTo (the order of a Set is unspecified). It is 1.0–1.3× faster than compat; the rest of a run (interpretation, model access, allocation) is common to both modes. | DESIGN §6; measured with `qvto bench` (`b6_hashsets`, `b6_including`, `b6_sets`, `b5_query_*`). |
| Q5b-11 | Fresh-mode times of small runs are dominated by cold caches: the model is loaded just before the run (as in Java), and a second run on the same model takes a third of the time. Series mode is the realistic case for SimOxide. | Measured (`b2_lb_scaleUp_x1`: 0.06–0.07 ms first run, 0.02 ms second). |
| Q7-1 | Fuzz campaigns are a `qvto fuzz` command over generators in `qvto-testkit::fuzz` (expressions from typed productions with a tree for minimisation, transformations, models of `fuzz.ecore`): a batch of cases runs once in the Java runner (one JVM), then in the engine; findings are compared per statement label (expressions) or strictly (programs) and minimised by batched reductions (`E::reductions`, every candidate a case of one Java run) or line deletion (`pmin`). Both sides run with constant identity hashes (`-XX:hashCode=2` in Java, `identity::Constant(1)` in the engine; `--tape` replays a Java tape instead). | Tapes cover only the inputs, outputs and their metamodels; with constant hashes every identity-hash order (sets of objects, tuples, collisions in tree bins) is reproducible on both sides and `Class@hash` texts agree, so orders are compared too. |
| Q7-2 | Regressions: `cases/fuzz.cases` (suite `fuzz` of `qvto conform`, material in `crates/qvto-testkit/fuzz/`, added with `tools/add-fuzz-case.py`) runs with constant identity hashes on both sides and is recorded once (`qvto conform record --suite fuzz`); analyser regressions are `crates/qvto-sema/tests/data/q7/*.qvto` with Eclipse's problems (`tools/q7-record.sh`, compared exactly by `tests/q7.rs`, problems without position as Eclipse prints them: line -1, offset -1, length 1). | A deterministic suite and exact diagnostics for everything the campaigns found. |
| Q7-3 | Robustness fuzzing is a mutational fuzzer of our own (`qvto fuzz robust`): character-class token mutations of the corpus and of generated cases (deletion, duplication, swaps, keywords and punctuation from `qvto_syntax`'s generated tables, splices, numeric edge values, nesting bombs, odd characters, imports resolved to other mutated units), the whole pipeline per input (parse, print, analyse, compile, run with a step budget and limits), worker processes under `ulimit -v`, restarted on death or after 60 s without progress, inputs kept. | `cargo fuzz` needs a nightly toolchain, which the machine does not have; process isolation catches aborts (stack overflow, out of memory) that `catch_unwind` cannot. |
| Q7-4 | Resource limits of a run (`RunOptions`): `max_collection_size` (default 2^27 elements: ranges, `product`, any library result), `max_string_length` (2^27 UTF-16 units), `max_native_stack` (default 1 MB below the run's entry: a call beyond it fails like the depth limit with `QvtStackOverFlowError`; the conformance adapter allows 256 MB on its 512 MB thread, `qvto run` 6 MB), the step budget (unchanged, default none) also charged one step per 256 elements or characters a library operation builds. Exceeding a size limit interrupts the run (`QvtInterruptedExecutionException`, code 110, "resource limit exceeded: ..."). Eclipse has only its depth limit (1 000) and otherwise runs out of heap or stack. | Hosts (SimOxide) must not abort on hostile or runaway transformations; a QVTo call level takes 2–4 KB of native stack in release builds, so Eclipse's depth 1 000 needs a few MB. |
| Q7-5 | Case resource sets follow `TestQvtStandaloneExecutor`: only the first root package of each `mmfile` is registered under its namespace (the first file wins; subpackages and packages loaded on the way are not found by namespace), not when the global registry (the case configuration's index, the EMF jars) has it; unresolved proxies (types of features, operations and parameters) become placeholder classifiers named `null` (one per proxy URI) instead of failing the registration; metamodel files are loaded once per (normalised) URI; a case configuration (`conf`) lies on top of the suite configuration (the Java runner's registry is JVM-wide and cumulative). | The external corpora load metamodels from many repositories with missing pieces; Eclipse registers and resolves them in this way. |
| Q7-6 | Semantics fixed from the campaigns and the corpora (each with a regression case): `oclIsKindOf`/`oclIsTypeOf` by `EDataType.isInstance` of the Java class (a `Long` or `Float` is no Integer/Real; never `oclIsTypeOf(OclAny)`, enumerations, tuple types); `oclAsType` converts only for a primitive/collection/enumeration/data type static source; `invalid` tuple parts; Java's `find` loop and group references in `replaceAll`; lone surrogates encode as `?`; `=` promotes numbers for a numeric static source; `sortedBy` keys by Java class; numbers stored into properties and intermediate features converted to the declared type; collection-typed properties, intermediate features and contextual properties copied on every read (`coerceValue`); objects added to the in-memory extent resource remain root candidates (`ResourceModificationAdapter`); a type literal of a metamodel classifier is its `EClassifier` object; generated `toString` of Ecore objects and OCL primitive types; `iterate` adds its result variable before evaluating its source; inherited mappings run with the mapping body as current instruction; the executor's outcome for a transformation without entry; Long index arguments of `substring`/`at` are invalid; QVTo's string counters are effects for `sortedBy`. Analyser: operations on an unknown type are OclAny's, a `return` of unknown type is OclVoid, a null OCL result type falls back to the operation's, an unknown context type makes a module operation, `flatten` types through `List`/`Dict`, generic `CommonT` errors have no position, collections of unknown elements print as `Kind()`. | Found by the differential campaigns and the external corpora (PLAN Q7). |
| Q7-7 | Not reproduced (documented): LPG's repair messages after a syntax error (first message and later errors can differ, Q1b-4); regular expressions match code points where Java splits surrogate pairs on empty matches; generated metamodels of the Java runner (QVTo's trace model prints `...impl.EValueImpl@`, XML feature maps of XSD-derived metamodels) are dynamic here; assignments to properties typed by unresolved proxy classes; transformations that need Java blackboxes. | Each needs a Java-specific emulation that real transformations hardly hit, or is outside the scope (§2.3). |

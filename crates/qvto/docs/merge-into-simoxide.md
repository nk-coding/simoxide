# Merging qvto-rs into SimOxide (design)

State: 2026-10-17. Written as a plan at qvto-rs `9459e5e`; this copy lives in SimOxide
(`crates/qvto/docs/`) since the import.

**Status (2026-10-17, branch `qvto-merge` from `spd`, not merged into `main`):** M0-M6 done (M5 in
two steps), M7-M8 open.

| Step | As built |
|---|---|
| M0 | worktree `simoxide-merge`, branch `qvto-merge` from `spd` (explicit members, `release-unwind`) |
| M1 | qvto-rs `439d3ab` imported with its 194 commits by `git filter-repo` path renames (`crates/qvto/...`) and one `--allow-unrelated-histories` merge; `git log -- crates/qvto/qvto-vm` has its 69 commits |
| M2 | QVTo crates are workspace members; `qvto-paths` (one helper crate instead of a module of `qvto-testkit`: the lower crates' tests need it too) gives the QVTo root (`crates/qvto`) and keeps `crates/qvto/target` a link to the target directory, so case files, scripts and the Java runner keep their `target/...` and `reference/...` paths; data paths `crates/qvto-x/...` became `qvto-x/...`; adapter on in-repo `simoxide-model`; F2's `qvto-simoxide-bridge` replaced by it; aliases `test-sim`, `test-fdn`, `test-qvto`, `build-exact`, `conform`, `qvto`; QVTo crates rustfmt- and clippy-clean; debug-info profiles left as SimOxide's (D9 not adopted: disk is plentiful) |
| M3a | `crates/foundation/`: `simoxide-jcompat` (hash codes, `Double.toString` with an allocation-free byte buffer, `Integer`/`Double` parsing incl. `&str` entry points, `Character` data, `Math.round/max/min`, Fx hasher, `MapOrder`/`InsertionOrder`), `simoxide-jcompat-exact` (`JavaHashMap` generic over the hashing environment, `JdkHashOrder` closed form tested against it, one `HotSpotXorShift` for both former copies), `simoxide-hotspot-math` moved. Replaced: qvto-runtime `java/*`, `fxhash`, HotSpot generator, qvto-testkit `idhash`, qvto-emf parsers, SimOxide's `javahash.rs`, both `javafmt.rs`, `canon.rs`'s formatter (now with the subnormal rule; EMF goldens unchanged), `raw.rs` parsers, both `fxhash.rs` (the loader's byte hashing kept: the other variant cost 4 % in `load-bench`), `jmath` round/max/min. `java/string.rs` stays in qvto-runtime (it works on qvto's `Str`); `java.util.Random` is S2's `javarand.rs` in simoxide-sim (EPL), not yet in jcompat |
| M3b | features: simoxide-sim `exact` (= `jdk-order`), fast always compiled (`fast` kept as a no-op); `Compat::Order` (`JdkHashOrder` / `InsertionOrder`) for `INNER`, BYTESIZE sums and finalisation; `DefaultCompat`; `SimErrorKind::ModeUnavailable`; qvto `compat` in qvto-runtime/-vm/qvto/-testkit/-cli/-simoxide with an uninhabited `JavaHashMap` stand-in in EPL builds; `MODE_UNAVAILABLE` run outcome; `exact` in simoxide-testkit/-reconf/-spd/-cli. Defaults EPL-only, CLI default mode fast. Tests enable `exact`/`compat` through dev-dependencies (so no `test-sim-epl` alias); EPL check by builds, `cargo tree` and the EPL `conform --engine qvto-fast` scoreboard. `LICENSES` sections 7 (no licence for the exact crate) and 8 (Eclipse QVTo/OCL) |
| merge | `spd` merged twice (F2; then P1+S2 at `c8ee1d9`) |
| M4 | `crates/foundation/simoxide-emf-core` from `qvto-model` (`git mv`; the QVTo crates keep `qvto_model::` through `qvto-model = { .., package = "simoxide-emf-core" }`); `seed` module: `StaticPackages` (packages with classifier order and locations, classes with complete `super_types` incl. Ecore's and the derived `supers`/`all_supers`/`all_features`/`id_attribute`, features with EMF's flags and the static readers' `kind`, data types, operations); `MetaRegistry::from_static` (the adapter's `registry.rs` logic, ids asserted), `resolve_static_ecore` (called by `qvto_emf::Graph::with_registry` after Ecore), `static_operations`, `static_top_packages`; `gen_meta.py` writes one `generated.rs` with `SEED` (main and ext tables merged, `generated_ext.rs` gone; the `class::`/`feat::`/`enums::` constants are byte-identical); `simoxide_model::meta` ids are the core's ids, the static lookups are extension traits (`meta::prelude::*`: `PcmMeta` with `def()`/`name()`, `PcmClass`, `PcmFeature`, `PcmEnum`, `PcmPackage`); the table types keep SimOxide's names as aliases of the seed types; the adapter's registry is `from_static`. Compile-time `Send + Sync` (`CompiledModel`) and `Clone + Send + Sync` (`Model`, the graph) assertions in `simoxide-sim/src/ir.rs`. The adapter's results on the 24 corpus cases recorded for M6 (`simoxide-reconf/tests/data/adapter-corpus`: journal as text with objects by key, dump, status, log; `qvto-simoxide/tests/record.rs`). Gates: fmt; clippy `-D warnings` default, `exact,hotspot-math`, `compat`; `cargo test --release --workspace --features exact` 525 passed (524 + the recording test), 0 failed; `golden.sh` identical (312 runs); corpus 132/132, corpus-fuzz 6/6; EMF goldens 165/165 (81 335 typed values); `cargo test-qvto` 211 passed (adapter registry test: equal to the `.ecore`-built registry; corpus 24/24); `conform` compat 542/542 strict, fast 542 / 537 strict (unchanged); sema 1 102/1 102; release check 0 opt-in crates for `simoxide-cli` and `qvto-cli` |
| M5a | `crates/foundation/simoxide-emf` from `qvto-emf` (`git mv`; QVTo crates keep `qvto_emf::` through the package rename; paths of the oracle scripts, the reference runner, `SUITE_CONF` and the sema harness follow; no dependency on `qvto-runtime` any more: `Double`/`Float.toString` from `simoxide-jcompat`). Graph: registry behind `Arc` (`meta_mut` copies on write, `adopt_meta`, `meta_arc`, `empty_like` for cheap empty graphs), Ecore's hook table behind `Arc`, the derived-list cache in a `Mutex` (`Graph: Clone + Send + Sync`, tested), FxHash maps, `Index<ObjId>` with public `class`/`container`/`proxy`/`line`, resources with `name`/`path`/`failed`; the change journal (`journal.rs`: `Journal`, `Change`, `ChangeKind`, `Summary` with `ObjId`s) recorded inside the graph's EMF operations at the adapter's points (`Set` with effective values on both ends of opposites, `Added`/`Removed` at the list change, then the inverse updates, `Container`, root changes in `eSetResource`'s order, removals of `clear` from the end, `Deleted` last); the id policy (`ids.rs`: `IdPolicy`, the seeded UUID generator; the target class is PCM's `Identifier` or `set_id_target`); `load_ops.rs`: loader-level writes with the former `raw::Graph` semantics (`load_set_ref`, `load_add_ref`, ..., not recorded) and plain reads (`get_ref`, `get_refs`, `stored`, `is_stored`, `children`, `id`, `path_fragment`, `resource_contents_into`). `is_set` of an explicit null reference counts only for unsettable references (as the adapter). Gates: fmt, clippy (3 feature sets); `test-fdn` 91 passed (incl. the moved suite, 3 journal tests); `test-qvto` 138 passed (the 73 tests of `qvto-emf` moved to `test-fdn`; adapter corpus 24/24); `conform` 542/542 strict, fast 542/537; sema 1 102/1 102; release check 0/0. SimOxide is not touched by M5a |
| merge | `spd` merged a third time (F3, the QVTo host in `simoxide-reconf`, at `bae975d`); gates as M5a plus 548 workspace tests, reconf `corpus` 1 (24 cases) and `host` 16 |
| M5b | SimOxide on the shared graph: `simoxide_model::raw::Graph` *is* `simoxide_emf::Graph` (`raw::pcm_graph()`: an empty graph sharing one registry of the static tables plus Ecore). The PCM loader (`xmi.rs`, `load.rs`, EMF 2.33 rules, diagnostics) is kept and writes through the graph's `load_*` operations; values are stored as the adapter showed them to QVTo (`raw::to_stored`: the box of the registry's data kind), SimOxide's readers get `raw::Value` through the extension trait `raw::PcmGraph` (`attr`, `attrs`, `str_attr`, `fragment`, `describe`, ...; the static tables' defaults); `is_set` of SimOxide's readers is `is_stored`; the parse cache and bundled models copy `ObjectBlock`s (`block_of`, `splice_block`); proxies are resolved in feature order (dense slots have no first-set order; goldens unchanged); the builder, validation, `is_a` and `describe` ignore objects of other metamodels. The adapter is a thin view of the graph (one home, 400 lines instead of 2 200): its `Session` is a prototype graph with the metamodel objects, appended to a model graph by the first store (`Graph::append_graph`); `SimStore` delegates to the graph's `ModelStore` (handles = `ObjId.0`, `ids_as_u32`), carries the graph's journal into the session and the UUID generator state back; `load_model` loads next to the PCM documents in the same graph (`ResourceSet::rebind`, `resolve_resource`); `raw_part.rs`, `uuid.rs`, the mirrors and the resource table are gone. F3 compiles unchanged. Gates: fmt, clippy (3 sets); workspace `--features exact` 548 passed; `golden.sh` identical (312); corpus 132/132, corpus-fuzz 6/6; EMF goldens 165/165 (81 335 values); reconf `changes` 16, `corpus` (24 cases: dry runs equal Java, change path 10/12/2), `host` 16; adapter corpus 24/24 and the recorded journals, dumps, logs and statuses of the 24 cases byte-identical (`tests/record.rs` against `simoxide-reconf/tests/data/adapter-corpus`); `test-fdn` 92; `test-qvto` 137; `conform` 542/542 strict, fast 542/537; sema 1 102/1 102; release check 0/0. Speed (indicative, `taskset`, a few rounds, against the pre-M5b tree): `load-bench` 69 models geomean 112 → 125 µs from files (+9 to +12 %, 12 % more allocations), memory loading with a warm `ParseCache` 3.63 → 3.99 ms total (+10 %), after `Graph::default` stopped registering Ecore each time (the typed model's default graph had cost 140 µs per load); `simoxide-bench suite` (fast, 3 reps, 2 alternating rounds): load ratio 0.99 / 1.06, compile 0.93 / 1.02, events/s unchanged within noise. Loader output (`load-bench snapshot`, 231 directories: diagnostics, canonical dumps, typed models) identical apart from the graph's own debug format |
| merge | `spd` merged a fourth time (Q1/Q2: SPD corpus suite, `Mode::Ordered`, `RunEnv::touch`; at `9e62855`); paths of `spd-replay` to `simoxide-emf` fixed. Gates: workspace `--features exact` 551 passed; `golden.sh` identical; corpus 132/132, corpus-fuzz 6/6; EMF goldens 165/165; `test-fdn` 92; `test-qvto` 119; `conform` compat 589/589 strict, fast 589 / 584 strict, ordered 583 strict (plus the 6 `java-fails-equal`); sema 1 102/1 102; release check 0/0. Note: `interpret_scr17812` depends on the cached Java run's HotSpot seed (`target/qvto-testkit/java`): with 2 of 6 fresh recordings its identity-hash replay gives the other `union` order (also on `spd` itself); the cache here was re-recorded with a seed that replays |
| M6 | `crates/qvto/qvto-simoxide` deleted; QVTo runs directly on the graph. F3's switch (`simoxide-reconf`): `qvt.rs` `Template` = a `ResourceSet` over the prototype graph (registry with SimOxide's tables, Ecore, the metamodel objects, the embedded Palladio metamodels); `attach` appends it to a run's graph once and sets the id policy; `run`/`with_journal` on `&mut Graph`; `PerRun` = the working graph behind an `Arc` (copy on write, shared with forks) and its fingerprint (generation, compiled model, model graph); changes of other models only go to the simulation by `Simulation::set_model_graph` (new: the run's graph may differ from the compiled model's outside SimOxide's metamodels; `RunModel` holds it), PCM changes through the change path as before; `change_set(&graph, &journal)` keeps objects of SimOxide's classes. API: closures `&mut Graph`, `IdSourceFn` `&Graph`, `Outcome::{journal, extents}` with `ObjId`, `ModelRef::Loaded(ResId)`, `load_model`/`create_model` take `&mut Simulation`, `Edit::new(&sim)` with `store() -> &mut Graph`, new `QvtoCore::{attach, load_model, operations, derivations}`; `simoxide_reconf::{Journal, IdPolicy}` are `simoxide_emf`'s. Tests moved: the adapter's registry and intermediate-class tests (`simoxide-reconf/tests/graph_qvto.rs`), the comparison with the adapter's recorded results (`tests/adapter_parity.rs`: journal, dump, status, log of the 24 cases byte-identical; the SPD case's journal re-recorded after Q2 changed its identity-hash draw order, equal to the M5b adapter at `4ea8c75` on all 24); the adapter's contract and script tests are the graph's own (`simoxide-emf`); its examples dropped (`simoxide-reconf/examples/reconfigure.rs` shows the flow). Gates: fmt, clippy (3 sets); workspace `--features exact` 506 passed (the adapter's 50 tests gone, 5 added), 0 failed; `golden.sh` identical (312); corpus 132/132, corpus-fuzz 6/6; EMF goldens 165/165; reconf `changes` 16, `host` 16, `corpus` 24 (dry runs equal Java; change path 10 applied, 12 no-op, 2 `Invalid`); `test-fdn` 92; `test-qvto` 90; `conform` compat 589/589, fast 589/584, ordered 583 (+6 `java-fails-equal`); sema 1 102/1 102; release check 0/0 |
| merge | `spd` merged a fifth time (R1, SimuLizar's reconfiguration process, at `24c61f2`): `simoxide-reconf/src/process.rs`'s PRM mirror ported (objects as `ObjId`, `ModelRef::Loaded(ResId)`; the PRM is a resource of the run's model graph, written through `with_store` and handed to the simulation with `set_model_graph`). Gates: workspace `--features exact` 518 passed (incl. `rules_process` 9: PRM values against the patched reference, `corpus-rules` byte-identical); `golden.sh` identical; corpus 132/132, corpus-fuzz 6/6; EMF goldens 165/165; `test-fdn` 92; `test-qvto` 90; `conform` 589/589 compat, fast 589/584, ordered 583; sema 1 102/1 102; release check 0/0 |
| M6b | Speed follow-up after R1 (the reconfiguration process copied the run's graph, now with the metamodel objects, on every PRM write: `rules_process` 12.5 s against 5.9 s on `spd`): the graph's objects in a vector whose frozen prefix is shared, copy-on-write chunks (`chunks.rs`; `Graph::freeze`, called by `build_from_graph_after` and the QVTo attach), metamodel links behind `Arc`, the derived-list cache not copied, resource URIs parsed on first use, `Cow` version strings, the loader reserving objects. Indicative (`taskset`, a few rounds): `rules_process` 2.2 s (spd 4.8-5.8 s); `load-bench` geomean 111.6 → 120.8 µs (+8 %, allocations +6 %); `simoxide-bench suite` against `spd` `59484b7` (fast, 5 reps, 3 alternating rounds on the shared machine): load and compile ratios 0.99-1.19 with the base itself varying by 20 % (MediaStore compile 191/195/206 µs vs 191/200/205 µs, load 702-786 vs 766-819 µs), events/s 0.95-1.03: no regression beyond the noise except loading Gates: fmt, clippy (3 sets); workspace `--features exact` 519 passed; `golden.sh` identical (312); corpus 132/132, corpus-fuzz 6/6; EMF goldens 165/165; `test-fdn` 93; `test-qvto` 90; `conform` 589/589 compat, fast 589/584, ordered 583; sema 1 102/1 102; release check 0/0 |
| merge | `spd` merged a sixth time (X1/X2, the Slingshot-exact engine `simoxide-sim/src/ssx`, at `59484b7`; no conflicts, no port needed). Gates: workspace `--features exact` 524 passed (incl. `slingshot_exact`); `golden.sh` identical; corpus 132/132, corpus-fuzz 6/6; EMF goldens 165/165; `test-fdn` 93; `test-qvto` 90; `conform` 589/589, fast 589/584, ordered 583; sema 1 102/1 102; release check 0/0 |

**M7 resumes here:** the docs (`docs/qvto/`, section 4). Open follow-ups of M5/M6 (no gate
failed): loading about +8 % (dense slots, more allocations); a run's graph carries the metamodel
objects after its first QVTo use, so a model change rebuilds over a larger graph; one PCM loader (SimOxide's) and one
generic loader (`xmi_load`) write the same graph (unifying them is optional). For X2d (SPD events in the
Slingshot-exact engine, an `Arc` model reference swapped between events): the run's compiled
model already is an `Arc` swapped between events (`RunModel`, `compiled_ref`), and the run's
graph can be replaced without recompiling (`Simulation::set_model_graph`, `RunModel`'s second
field); a frozen graph copies cheaply.


User decision (2026-10-17): all QVTo crates move into the SimOxide repository, nested in their
own directory, with a QVTo section in SimOxide's VitePress docs ("the wiki"). A set of
foundation crates is shared by the QVTo crates and the Palladio crates.

Hard constraints:

- SimOxide's exact mode stays byte-identical to SimuLizar. Its guardrails pass unchanged:
  `cargo test --release` of its crates, `crates/simoxide-cli/bench/golden.sh` hashes, the
  corpus in both modes, the EMF canonical-dump goldens of `simoxide-model`.
- qvto stays at 518/518 strict conformance (compat and fast) and 1 102/1 102 for the analyser
  oracle. Q7 may change these counts (external corpora); the gate is "the counts at the time of
  the import, unchanged".
- SimOxide's hot paths do not regress: simulation (events/s), and loading (`load-bench`),
  which matters for the optimisation loops that load a model per candidate.
- Licensing (user decision, 2026-10-17): "exact mode crates with the GPL+EPL weirdnesses, fast
  mode without". Java-exactness code that is GPL-derived or GPL-adjacent lives in separate
  opt-in crates and cargo features, used only by the exact/compat modes. The default build of
  every crate and every fast mode is EPL-2.0 only. SimOxide will have two exact modes
  (SimuLizar-exact, Slingshot-exact) plus fast modes; QVTo has compat and fast. Section 2.3.

## 0. Summary

- **Duplication is real but concentrated.** Two places hold most of it: the EMF layer (metamodel
  tables vs registry, object graph, XMI loader, resource sets, canonical dump) and Java compat
  (`String.hashCode`, `HashMap` order, `Double.toString`, number parsing, `Math.round`/`max`/`min`).
  `qvto-emf`'s loader and `qvto-runtime`'s `Double.toString` were adapted from SimOxide's code,
  so the copies share an origin. SimOxide alone has three copies of `Double.toString` and two
  of FxHash.
- **Foundation crates**, in `crates/foundation/`:
  - `simoxide-jcompat` (EPL): Java behaviour that the javadoc specifies (hash codes,
    `Double.toString` of JDK ≥ 19, number parsing, `String`/`Character` semantics, `Math`
    basics, insertion-ordered maps, identity-hash interfaces, FxHash);
  - `simoxide-jcompat-exact` (opt-in, feature `jdk-order`, GPL-adjacent): what only OpenJDK's
    and HotSpot's sources define: `HashMap`/`HashSet` bucket order (SimOxide's `javahash.rs` and
    qvto's `hashmap.rs`) and HotSpot's identity-hash generator;
  - `simoxide-hotspot-math` (opt-in, feature `hotspot-math`, GPL-2.0-only; exists today, moves
    to `crates/foundation/`);
  - `simoxide-emf-core`: ids, `MetaRegistry`, `Datum`, the `ModelStore` trait and its contract
    tests (today `qvto-model`);
  - `simoxide-emf`: the one EMF graph, Ecore, `.ecore` registration, XMI load and save, URIs,
    resource sets, the change journal, id policies (today `qvto-emf` plus SimOxide's loader
    fast paths plus the Q6 journal).
  - Later, optionally `simoxide-oraclekit` (warm-JVM batch driver, fixture bundles), once the
    Slingshot reference runner is a third user.
- **SimOxide can sit on the shared graph.** The simulator never reads the graph at simulation
  time. Only loading, the typed-model builder, validation and the compiler (`ir.rs`, about ten
  call sites for ids and fragments) read it. Two things must change in the shared graph first:
  it must be `Sync` (today `qvto_emf::Graph` has a `RefCell` cache; `CompiledModel` holds the
  graph and must stay `Send + Sync`), and it must load as fast as SimOxide's loader.
- **The Q6 adapter then becomes unnecessary.** QVTo runs on the same graph type that SimOxide
  loads into. PRM, SPD and intermediate-class objects live in the same graph. The two-homes
  limits and the mirrors go away. The journal and the UUID policy move into `simoxide-emf`.
- **Exact modes become opt-in.** SimuLizar-exact, Slingshot-exact and QVTo compat need
  `jdk-order` (feature `exact`); bit-exact `Math.log`/`exp` also needs `hotspot-math`. Without
  them these modes are not compiled and asking for them is an error, not a silent fallback.
  Default builds run the fast modes. This changes SimOxide's defaults (today exact is the
  always-compiled default mode, and the CLI turns `hotspot-math` on by default).
- **Layout**: `crates/simoxide-*` stay where they are. `crates/qvto/` is the QVTo home: its
  crates (`crates/qvto/qvto-*`, names unchanged), `reference/`, design notes. User-facing docs go
  to `docs/qvto/`.
- **Import with history** by `git filter-repo` path renames, then one
  `--allow-unrelated-histories` merge. Exact commands in 5.3.
- **Plan**: M0-M8, about 5.5-8.5 weeks of developer time. M1-M4 (import, workspace, Java compat,
  registry) take about 1.5-2.5 weeks and keep the adapter as a bridge. M5 (one graph) is the
  large, risky step: 2-4 weeks.

## 1. Duplication inventory

"S" = SimOxide (`/home/devbox/workspace/simoxide`), "Q" = qvto-rs. Paths are relative to each
repository.

### 1.1 EMF layer

| Area | SimOxide | qvto-rs | Differences | More complete | Unification risk |
|---|---|---|---|---|---|
| Metamodel ids and tables | `crates/simoxide-model/src/meta/mod.rs` + `generated.rs` (15 k lines, from `tools/gen_meta.py`): static `&'static` tables, `ClassId::def()`, PCM 5.2.2 only (35 packages, 299 classes, 520 features). Branch `qvto-adapter` adds `generated_ext.rs` (multiplicities, flags, complete supertypes, data types, operations). | `crates/qvto-model/src/meta.rs`: dynamic `MetaRegistry` (add packages/classes at run time, `finish_classes`, `slot()` O(1) via `fixed_slots`, subtype bitsets). `qvto-emf` registers `.ecore` files; `materialize` makes metamodel objects. | Static vs dynamic. S's main tables miss generic supertypes (`IntSample extends Sample<T>`) and `PCMBaseClass`'s `EObject`; the ext tables have them. Q keeps EMF facts S never needed (ordered, unique, changeable, unsettable, interface, operations). | Q (superset; Q6 proved S's tables load into it with equal ids) | Low for the registry. Medium for S's code, which uses `def()` everywhere: keep `def()` through an extension trait (2.6). |
| Object graph | `src/raw.rs` `Graph`: `Vec<Obj>`, sparse slots `Vec<(FeatureId, Slot)>` in first-set order, `xmi_id` and source `line` in the object, `Value` with `Int(i64)` for all integral kinds. Mutators are loader-only (`pub(crate)`): opposites and containment, but no uniqueness, roots, `isSet` rules or changeability. | `crates/qvto-emf/src/graph.rs` `Graph`: dense slots per class feature (`Unset` state, "set" flag for unsettable lists), resources own `xmi:id` maps, values are `OwnedDatum` with Java box kinds (`Int(i32)`, `Long`, `Short`, ...), Ecore hooks, derived-list cache in a `RefCell`, owns its `MetaRegistry` by value. Full `ModelStore` with EMF semantics. | S is a load-time structure tuned for speed. Q is a full dynamic-EMF store. Q is `!Sync` (the `RefCell`). Q's box kinds matter for QVTo (`Long(1) != Integer(1)` in sets). | Q semantically; S in load speed | High. The central step (M5). |
| EMF write semantics | `raw.rs` lines ~420-600 (opposites, `eInverseAdd/Remove`, detach, `addUnique`, `move`). | `qvto-emf/src/graph.rs`, `ecore_hooks.rs`; `qvto-simoxide/src/raw_part.rs` ports the same semantics onto S's sparse slots. Checked by the contract suite and 300 random scripts against EMF. | Q is complete (uniqueness, roots, `isSet`, changeability, `EcoreUtil.delete`, `setEList`); S covers what loading needs. | Q | Low: S only needs a subset. |
| XMI load | `src/xmi.rs` (1 172 lines), EMF 2.33 (the SimuLizar product). Borrowed attribute values, `StrMap` ID index, tolerant namespaces (PCM 5.0/5.1 URIs), diagnostics with lines, generated-ID detection. | `src/xmi_load.rs` (994 lines), **adapted from S's `xmi.rs`**, generalised to the dynamic registry; EMF 2.41; `xsi:schemaLocation`, XMI 2.1, encoded attribute style. | Same core rules (IDREF forward refs, `mustAdd`, ID lookup order). S is faster and PCM-specific; Q is general. Q already loads 640 PCM-side files (S's corpus included) with dumps equal to EMF 2.41. | Q in coverage, S in speed | Medium: merge S's fast paths into Q's loader; gate on S's goldens and `load-bench`. |
| XMI save | none (U10 of `DESIGN-SPD.md` asks for one). `simoxide-testkit/src/modelgen/xmi.rs` writes PCM XMI text for generated models by hand (templates, not from a graph). | `src/xmi_save.rs` (851 lines): port of `XMLSaveImpl`/`XMISaveImpl`, byte-equal to EMF on 544 suite models. | — | Q | None: S gains it. The modelgen writer can stay (it writes the reference's `PcmBuilder` layout). |
| Resource sets, URIs, proxies | `src/load.rs` (993 lines): `Loader`, URI keys (`file:`, `pathmap://`), bundled default models parsed once per process, `ParseCache` (parsed templates shared between loads and threads), memory loading, demand loading, `resolveAll`. | `src/resource_set.rs`, `uri.rs` (port of EMF `URI`), `fragment.rs` (IDs, paths, `eKeys`, Ecore named segments), `config.rs` (URI maps shared with the Java oracle). | S has the PCM conveniences and caching; Q has EMF's full URI and fragment rules. S's proxies record the referencing object's resource; Q's record `eDirectResource`. | Both, different parts | Medium. |
| Data values | `raw.rs` `parse_value`, `parse_java_int`, `parse_java_double` (no hex). | `qvto-emf/src/datatypes.rs`: `createFromString`/`convertToString` per data kind (`EcoreFactoryImpl` vs `EFactoryImpl`), `Float.toString`, intrinsic defaults. | Q is complete. S accepts only ASCII digits for `EInt` (Java's `Integer.valueOf` accepts any Unicode digit). | Q | Low (S's goldens never use exotic literals). |
| Canonical dump and EMF oracle | `src/canon.rs` (1 757 lines): JSON lines, `{"path","id","type","attrs","refs"}`; oracle `reference/oracles/pcm-model` (EMF 2.33), goldens committed. Own `java_double_to_string` (Rust `{:e}`, no subnormal rule). | `qvto-emf/src/canon.rs`: different JSON-lines format (`nsURI#Class` types, `set` list, order-sensitive paths); oracle `crates/qvto-emf/oracle/CanonDump.java` (EMF 2.41). | Two formats with two committed oracles. Both test-only. | Equal | Keep both formats (low value in unifying). Port S's dump onto the shared graph unchanged. |
| Metamodel generator | `tools/gen_meta.py` reads the product's `.ecore` files, writes static Rust tables. | none at build time: `.ecore` files are loaded at run time; `materialize` builds metamodel objects from a registry. | — | — | Low: the generator keeps writing tables; they become a static seed of the registry. |

### 1.2 Java compatibility

| Area | SimOxide | qvto-rs | Differences | More complete | Unification risk |
|---|---|---|---|---|---|
| `String.hashCode`, `HashMap` spreading | `crates/simoxide-sim/src/javahash.rs` (84 lines): `string_hash`, `spread`, `capacity_for`, `iteration_order` (insert-only maps, no tree bins). Used by `ir.rs` and `frames.rs` (SEFF `INNER` characterisations, ND-12/13). | `crates/qvto-runtime/src/java/hash.rs` (ASCII fast path), `java/hashmap.rs` (1 341 lines): JDK 21 `HashMap` incl. resize split, tree bins, removal, constructor sizing. | Q is a full table; S is a closed-form order for insert-only maps (cheap, used at compile and in frame visits). | Q | Low if S's fast function is kept as a special case and tested against the full port. Switching S to the full table would also change behaviour if a bucket ever treeified (never seen; more correct). |
| `Double.toString` | three copies: `simoxide-sim/src/javafmt.rs` (`push_double` into `String`, trace writer), `simoxide-testkit/src/javafmt.rs` (fast path plus exact big-integer oracle), `simoxide-model/src/canon.rs` (`{:e}`, misses the two-digit subnormal rule). | `qvto-runtime/src/java/fmt.rs`: adapted from S's testkit version, adds `Float.toString` and integer formatting. | Same algorithm (ryu plus the subnormal rule) except S's `canon.rs` copy. JDK ≥ 19 only, everywhere. | Q | Low. Gate: golden hashes (trace numbers). Check that the trace writer stays allocation-free. |
| Number parsing | `raw.rs` (`Integer/Long/Double.valueOf`, partial); StoEx parser uses Rust `parse` after its own lexer. | `qvto-runtime/src/java/parse.rs`: Java's full syntax (signs, Unicode digits, hex floats, `NaN`, `Infinity`, `f`/`d` suffixes, whitespace). | Q complete. | Q | Low. |
| `String` semantics, `Character` | `simoxide-stoex/src/jmath.rs` `string_compare` (UTF-16 order). | `java/string.rs` (UTF-16 indices, `compareTo`, case mapping, `trim`, tokenizer), `java/chars.rs`, `chars_data.rs` (generated from JDK 21). | Q superset. | Q | None. |
| `Math` basics | `simoxide-stoex/src/jmath.rs` (`java_round`, `double_compare`, `max_f64`, `min_f64`, `drem`, `log`, `pow`), `simoxide-random/src/jmath.rs` (`max`, `min`, `log`/`exp` through `crmath` or GPL `hotspot-math`). | `qvto-runtime/src/num.rs` (`java_round`, `java_max`, `java_min`). | Three copies of round/max/min. `log`/`exp`/`pow` exist only in S. | — | None for round/max/min. `log`/`exp`/`pow` stay in S (QVTo does not need them; the GPL crate stays isolated). |
| Identity-hash generators | none in Rust (refsim is patched for determinism instead). | `qvto-runtime/src/identity.rs` (`HotSpotXorShift`, `Tape`, `Sequential`, `HashEnv`) and a second xor-shift in `qvto-testkit/src/idhash.rs` (sequence recovery from a seed). | Q-internal duplicate. | Q | None. The Slingshot flavour (SPD quirk Q8, `->any` on sets) will use it. |
| FxHash | `simoxide-model/src/fxhash.rs` (plus `StrMap`, identity hasher), `simoxide-sim/src/fxhash.rs`. | `qvto-runtime/src/fxhash.rs`. | Three copies of the same 40 lines. | equal | None. |

### 1.3 Test and tool infrastructure

| Area | SimOxide | qvto-rs | Differences | Unify? |
|---|---|---|---|---|
| Java oracles | `reference/oracles/{pcm-model,random,sched,stoex}` (`build.sh` against the SimuLizar 5.2.2 product jars, goldens committed). | crate-local: `qvto-emf/oracle`, `qvto-runtime/tools/oracle`, `qvto-syntax/tools/oracle`, `qvto-types/tools/oracle`, `qvto-sema/tools/oracle` (against the reference runner's QVTo 3.11 jars). | Same pattern (Java 21, `javac` against jars in place, dump goldens, committed fixtures). Different jar sets and EMF versions (2.33 vs 2.41). | No code to share; share conventions and a `reference/README.md` index. |
| Reference runner | `reference/refsim` (patched SimuLizar, `batch` mode in one warm JVM, trace and random tape). | `reference/runner` (`qvto-run`, `--batch`, `--bench`, identity-hash tape, canonical dumps). | Both: warm-JVM batch over a job list, results directory, tapes. | Later, a small `oraclekit` (warm-JVM batch process, result directories) when the Slingshot runner (`DESIGN-SPD.md` S0) is a third user. |
| Tapes | random tape (`tape.jsonl`, `simoxide-testkit/src/tape.rs`). | identity-hash tape (`idtape.tsv`, `qvto-testkit/src/fixture.rs`). | Different data; same idea (replay the reference's non-determinism). | No. |
| Corpus and cases | `corpus/*` directories with `expected/` (`simoxide-testkit/src/corpus.rs`), `corpus-fuzz/`. | case files (`qvto-testkit/cases/*.cases`), fixture bundles, inventory TSV, `corpus/{simulizar,spd}`. | Different domains. | No. |
| Differential helpers | `diff.rs` (first trace divergence, tolerance, per-process), `meascmp.rs`, `stats.rs`, `equiv.rs` (exact vs fast statistically). | `compare.rs` (drivers' checks, strict verdict), `report.rs` (scoreboard), `qvto-emf/src/compare.rs` (the suite's `TreeComparator`). | Different outputs compared. | No. |
| CLI and benchmarks | `simoxide-cli` (`simoxide`, `simoxide-bench`), `simoxide-fuzz`; hand-rolled argument parsing; `pprof` behind feature `profile`; medians. | `qvto-cli` (`run`, `check`, `conform`, `bench`, `disasm`, Q7: `fuzz`); hand-rolled parsing; `pprof` behind `profile`; median and p10-p90. | Same patterns, small code. | No (not worth a crate). |

### 1.4 Not duplicated

- SimOxide only: StoEx (`simoxide-stoex`), random numbers and distributions (`simoxide-random`,
  `simoxide-hotspot-math`), schedulers (`simoxide-sched`), the simulator (`simoxide-sim`), the
  typed PCM model (`model.rs`, `builder.rs`, `validate.rs`).
- qvto-rs only: syntax, types, sema, bytecode, runtime values and collections, VM, facade.

## 2. Foundation crates

### 2.1 What reads the graph in SimOxide

- **Simulation**: nothing. `sim.rs` and `sim/*` never touch `Model::graph`. They read the
  `CompiledModel` tables (and `cm.model.containers[..].id` at two places for output names).
- **Compile** (`ir.rs`): about ten calls (`graph.id`, `graph.fragment`, `graph.attr` for
  measuring points and trace ids). Once per model.
- **Build and validation** (`builder.rs`, `validate.rs`): helpers `r`, `int`, `dbl`, `s` over
  `attr`/`get_ref`/`get_refs`; about 40 call sites. Once per model.
- **Loading** (`xmi.rs`, `load.rs`): the graph's own mutators.
- **Canonical dump** (`canon.rs`): tests and `pcm-dump`.

So the graph is off the hot path. Its costs that matter are load time (mediastore: 513 µs from
memory, 188 µs with a warm `ParseCache`; best of 300) and the `Sync` bound of `CompiledModel`.

### 2.2 `simoxide-jcompat` and `simoxide-jcompat-exact` (Java compat)

`simoxide-jcompat` (EPL-2.0, always built), from `qvto-runtime/src/java/*` plus SimOxide's
copies. Only behaviour that the Java documentation specifies (2.3):

```rust
pub mod hash {     // String.hashCode (ASCII fast path), boxed hashCodes (javadoc formulas)
    pub fn str_hash(s: &str) -> i32; pub fn utf16_hash(u: impl IntoIterator<Item = u16>) -> i32;
    pub fn long_hash(v: i64) -> i32; pub fn double_hash(v: f64) -> i32; ...
}
pub mod order {    // what a mode needs to iterate a "Java HashMap"; the EPL side is insertion order
    pub trait JavaKey { fn java_hash(&self, env: &dyn HashEnv) -> i32; fn java_equals(..) -> bool; ... }
    pub trait MapOrder { fn order(&self, hashes: &[i32]) -> Vec<usize>; }   // SimOxide's call sites
    pub struct InsertionOrder;                                             // fast modes
    pub struct LinkedMap<K, V>;   // LinkedHashMap/LinkedHashSet (today qvto-runtime coll::LinkedMap)
}
pub mod fmt {      // Double.toString / Float.toString (JDK >= 19), integer formatting
    pub fn push_double(out: &mut String, x: f64); pub fn write_double(out: &mut Vec<u8>, x: f64);
    pub fn push_float(out: &mut String, x: f32); pub mod exact { /* big-integer oracle */ }
}
pub mod parse {    // Integer/Long.parseInt, Double.valueOf (full Java syntax)
    pub fn parse_int(u: &[u16]) -> Option<i32>; pub fn parse_long(..); pub fn parse_double(s: &str) -> Option<f64>;
}
pub mod string;    // UTF-16 indices, compareTo, case mapping, trim, tokenizer
pub mod chars;     // Character data generated from JDK 21 (chars_data.rs)
pub mod math {     // Math.round, Math.max/min (NaN, -0.0), Double.compare
    pub fn round(x: f64) -> i64; pub fn max(a: f64, b: f64) -> f64; pub fn min(..); pub fn double_compare(..);
}
pub mod identity { // interfaces and EPL sources only
    pub trait IdentityHashSource; pub trait HashEnv;
    pub struct Sequential; pub struct KeyDerived;   // fast modes
    pub struct Tape;                                // replays recorded values; fallback: any source
}
pub mod random;    // java.util.Random (javadoc-specified LCG), for Slingshot's Math.random stream
pub mod fx;        // FxHasher, FxHashMap, FxHashSet
```

`simoxide-jcompat-exact` (opt-in through feature `jdk-order`; licence: decision D5):

```rust
pub mod hashmap {  // JDK 21 HashMap/HashSet order (today qvto-runtime java/hashmap.rs)
    pub fn spread(h: i32) -> u32;                       // HashMap.hash
    pub struct JavaHashMap<K: JavaKey, V>;              // sizing, resize split, tree bins, removal
    pub struct JdkHashOrder;                            // impl MapOrder: SimOxide's javahash::iteration_order
    pub enum Jdk { V17, V21 }                           // constructor sizing differs (Q1c-4); for Slingshot-exact
}
pub mod hotspot {  // HotSpot identity hashes (-XX:hashCode=5)
    pub struct HotSpotXorShift;                         // today qvto-runtime identity.rs
    pub fn sequence_index(..);                          // today qvto-testkit idhash.rs
}
```

- `HashEnv` becomes generic: it hands out identity hashes for opaque `u64` keys and class ids.
  The engine's `IdentityKey`, `CompatEnv`, `FastEnv` stay in `qvto-runtime` and implement it;
  `CompatEnv` is compiled only with `jdk-order`.
- Dependencies: `simoxide-jcompat` on `ryu` only; the exact crate on `simoxide-jcompat`.
- `log`/`exp`/`pow` stay in `simoxide-random`/`simoxide-stoex` (correctly rounded, EPL).
  `simoxide-hotspot-math` (GPL-2.0-only) moves to `crates/foundation/` unchanged.
- SimOxide switches `javahash.rs` (to the exact crate), both `javafmt.rs`, `canon.rs`'s
  formatter, `raw.rs`'s parsers, `fxhash.rs` (keeping `StrMap` in the loader) and
  `java_round`/`max`/`min` (to the EPL crate).
- Exactness: same algorithms. `canon.rs` gains the subnormal rule (only test output changes, and
  only for subnormals). The golden hashes are the gate.

### 2.3 Licensing split: EPL core and exact crates

Rule: what the Java SE documentation (javadoc, JLS) or a public specification defines is
implemented in EPL crates. What only OpenJDK's or HotSpot's source defines is GPL-adjacent and
goes into opt-in crates. Code ported from GPL source is GPL (only `simoxide-hotspot-math`).

| Piece | Where today | Defined by | Side |
|---|---|---|---|
| `String.hashCode`, boxed `hashCode`s | S `javahash.rs`, Q `java/hash.rs` | javadoc formulas | EPL core |
| `Double.toString`/`Float.toString`, JDK ≥ 19 | S (3 copies), Q `java/fmt.rs` | javadoc (shortest-decimal spec) | EPL core |
| `Double.toString`, JDK ≤ 18 | not implemented | only OpenJDK source | exact crate, if a target JDK ever needs it |
| `Integer.parseInt`, `Long.parseLong`, `Double.valueOf` | S `raw.rs`, Q `java/parse.rs` | javadoc grammar | EPL core |
| `String` ops, `compareTo`, case mapping, `trim`, `StringTokenizer` | Q `java/string.rs`, S `stoex/jmath.rs` | javadoc, Unicode | EPL core |
| `Character` data | Q `chars_data.rs` | generated by running JDK 21 (Unicode data, Java's documented definitions) | EPL core; provenance in `LICENSES` |
| `Math.round`, `max`, `min`, `Double.compare` | S stoex/random `jmath.rs`, Q `num.rs` | javadoc | EPL core |
| `LinkedHashMap`/`LinkedHashSet` order | Q `coll::LinkedMap` | javadoc (insertion order) | EPL core |
| `java.util.Random` | none yet (Slingshot's `Math.random` stream) | javadoc (LCG constants) | EPL core |
| Java regex (`match`, `replaceAll`) | Q `qvto-vm/src/jregex.rs` (translation onto the `regex` crate) | javadoc of `Pattern`/`Matcher` | EPL |
| `HashMap.hash` spread, power-of-two tables, resize split, constructor sizing (JDK 17 vs 19+), tree bins, removal | Q `java/hashmap.rs`; S `javahash.rs` (insert-only closed form) | OpenJDK source only | **exact crate** |
| `HashSet`/`BagImpl`/`Dict` storage in Java order | Q `coll/*` on `JavaHashMap` | built on the above | qvto-runtime code behind `compat` |
| HotSpot identity hash (`-XX:hashCode=5`: xor-shift seeds, 31 bits, `0 → 0xBAD`), sequence recovery | Q `identity.rs`, `qvto-testkit/idhash.rs` | HotSpot source only | **exact crate** |
| Identity-hash tape replay, sequential, key-derived | Q `identity.rs` | own | EPL core |
| HotSpot `Math.log`/`exp` (Intel LIBM stubs) | S `simoxide-hotspot-math` | ported GPL-2.0-only code | GPL crate (as today) |
| correctly rounded `log`, `exp`, `pow` | S `crmath.rs`, `stoex/jmath.rs` | own | EPL |
| MT19937, inverse CDFs, `logGamma` | S `simoxide-random` | Apache Commons Math (Apache-2.0) | EPL build (compatible, as today) |
| EMF, OCL, QVTo semantics | Q, S | Eclipse code (EPL-2.0) | EPL |

The SimOxide `HashMap` rows are conservative: the closed form uses only `spread`, power-of-two
tables and the 0.75 threshold, but `spread` and the table layout are not in the javadoc.

**Features** (the same names in every crate that has them):

| Feature | Pulls in | Licence of what it adds | Enables |
|---|---|---|---|
| `jdk-order` | `simoxide-jcompat-exact` | D5 (recommended: GPL-2.0-only WITH Classpath-exception-2.0, the licence of the OpenJDK classes it follows) | Java `HashMap` order, HotSpot identity hashes |
| `hotspot-math` | `simoxide-hotspot-math` | GPL-2.0-only, no exception: binaries must not be distributed | bit-exact `Math.log`/`exp` |
| `exact` (in `simoxide-sim`, later `simoxide-reconf`, `simoxide-spd`) | `jdk-order`, `qvto/compat` | as above | SimuLizar-exact and Slingshot-exact modes |
| `compat` (in `qvto-runtime`, `qvto-vm`, `qvto`) | `jdk-order` | as above | QVTo compat mode |
| `fast` | nothing new | EPL | the fast modes (default on) |

Defaults: every crate's default features are EPL-only (`fast` on, `exact`/`compat`/
`hotspot-math` off), including the CLIs. Local developer builds and the guardrails turn them on
through cargo aliases (3). With the CPE licence for `jdk-order`, a build with `exact` but
without `hotspot-math` may be distributable; with `hotspot-math` it may not (as today).

**Modes without the features:**

| Mode | Built without its feature | Why |
|---|---|---|
| SimOxide SimuLizar-exact, Slingshot-exact | not compiled. `SimConfig { mode: Exact, .. }` returns `SimError::ModeUnavailable("build with feature exact")`; `simoxide run --mode exact` says the same. `Mode::default()` becomes `Fast` in such builds | an "exact" run with Rust's or insertion order would differ from the reference on most multi-container models and SEFFs with several `INNER` characterisations, while claiming exactness |
| SimOxide exact with `exact` but without `hotspot-math` | compiled; log/exp correctly rounded (1 ulp off HotSpot on about 0.25 % of `exp` and 1e-5 of `log` arguments) | today's documented library default; useful as a distributable build |
| SimOxide fast modes | always EPL. Where exact mode iterates in Java `HashMap` order (`frames.rs` `INNER` characterisations, `ir.rs` finalise order, `sim.rs` `getContents()`), fast mode uses `InsertionOrder` through a `Compat` hook (`C::map_order`) | removes the only exact-crate use from fast mode. Fast outputs change once; `Fast` vs `Literal<Fast>` and the statistical tests still hold |
| QVTo compat | not compiled. `Engine`/`Vm` creation with `Mode::Compat` returns `ModeUnavailable`; `qvto run --compat`, `qvto conform` (strict verdict) and recording tapes need the feature. Default mode: fast | silently running fast and calling it compat would give Eclipse-different orders under the compat label |
| QVTo fast | always EPL; `qvto conform --engine qvto-fast` (order-insensitive verdict) works in every build | fast already keeps all non-order semantics (Q5b-10) |

Consequences:

- In qvto, Java-order code is confined to `qvto-runtime` (`coll/*`, `dict.rs`, `java/hashmap.rs`,
  `identity.rs`). The VM (`trace.rs`, `stdlib.rs`) and the EMF crates only use `hashCode` and
  `equals`, which stay EPL. So `compat` gates a small, contained part.
- In SimOxide, Java-order code is `javahash.rs` and its three users (`frames.rs`, `ir.rs`
  finalise order, `sim.rs` `getContents()` order).
- `simoxide-sim`'s `compat::Exact` moves behind `exact`; `simoxide-random`'s inversion samplers
  and MT19937 stay EPL (they are not GPL-adjacent), so they remain usable by both.
- The guardrails (golden hashes, corpus exact, refsim comparisons) need a binary built with
  `exact,hotspot-math`; `golden.sh` gets the binary from the alias build.
- SimuLizar-exact with reconfiguration runs QVTo in compat mode (Set order of unordered PCM
  features), so `simoxide-sim/exact` turns on `qvto/compat` through `simoxide-reconf`.
- Slingshot-exact uses the same crates. `Jdk::V21` is assumed (Slingshot runs on Eclipse
  2025-12, which needs Java 21); if its reference ever runs on JDK 17, `Jdk::V17` sizing and the
  pre-19 `Double.toString` would be needed, both only in the exact crate.
- The EPL release check becomes: `cargo tree -p simoxide-cli -e normal` (default features) and
  `-p qvto-cli` list neither `simoxide-jcompat-exact` nor `simoxide-hotspot-math`.

### 2.4 `simoxide-emf-core` (today `qvto-model`)

- Ids (`PackageId`, `ClassId`, `FeatureId`, `EnumId`, `DataTypeId`, `ResourceId`), `Classifier`,
  `DataKind`, `MetaRegistry` (dynamic, append-only), `MetaOperation`, `Datum`/`OwnedDatum`,
  `ModelStore` + `ModelError`, the contract test suite (`contract.rs`).
- New: `MetaRegistry::from_static(&StaticPackages)`: ingests generated tables with their ids
  (Q6-3 logic, moved out of the adapter's `registry.rs`). Generated packages first, Ecore
  next, `.ecore` files loaded later after them.
- New: `serde` behind a feature (snapshots).
- Dependencies: `simoxide-jcompat` (Java `equals`/`hashCode` of data values).

### 2.5 `simoxide-emf` (today `qvto-emf`, plus SimOxide's loader, plus the Q6 journal)

```rust
pub struct Graph { /* objects, resources, Arc<MetaRegistry>, optional journal, id policy */ }
impl Graph {
    pub fn with_registry(meta: Arc<MetaRegistry>) -> Graph;
    // reads (SimOxide's builder and compiler use these)
    pub fn class_of(&self, o: ObjId) -> ClassId;
    pub fn attr(&self, o: ObjId, f: FeatureId) -> Datum<'_, ObjId>;     // effective value, default if unset
    pub fn get_ref(&self, o: ObjId, f: FeatureId) -> Option<ObjId>;
    pub fn get_refs(&self, o: ObjId, f: FeatureId) -> &[ObjId];
    pub fn children(&self, o: ObjId) -> impl Iterator<Item = (FeatureId, ObjId)>;
    pub fn all_contents(&self, r: ResourceId) -> Vec<ObjId>;
    pub fn id(&self, o: ObjId) -> Option<&str>; pub fn fragment(&self, o: ObjId) -> String;
    pub fn line(&self, o: ObjId) -> Option<u32>;                       // side table, loader option
    // journal (Q6 semantics; off = one branch per write)
    pub fn start_journal(&mut self); pub fn take_journal(&mut self) -> Journal;
    pub fn set_id_policy(&mut self, p: IdPolicy);                      // none | Uuid { seed }
}
impl ModelStore for Graph { /* the full EMF write semantics */ }
pub mod xmi_load; pub mod xmi_save; pub mod resource_set; pub mod uri; pub mod fragment;
pub mod ecore; pub mod materialize; pub mod register; pub mod datatypes;
pub mod canon;   // qvto's canonical dump; SimOxide keeps its own format in simoxide-model
pub mod compare; // the Eclipse suite's TreeComparator
pub mod journal; // Change, ChangeKind, Journal, Summary (from qvto-simoxide)
pub mod ids;     // IdPolicy, UUID generator (from qvto-simoxide)
pub struct LoadOptions { tolerant_namespaces, lines, cache: Option<Arc<ParseCache>>, .. }
pub struct ParseCache;  // from simoxide-model load.rs
```

Changes to `qvto_emf::Graph` that the merge needs:

| Change | Why |
|---|---|
| `MetaRegistry` behind `Arc` (copy on write when a metamodel is added) | the registry is shared by the compiled programs, all runs and forks; cloning a graph must not clone it |
| derived-list cache without `RefCell` (a `Mutex`, per-entry `OnceLock`, or computed eagerly once metamodels are finished) | `CompiledModel` holds the graph and is `Send + Sync`; `run_batch` shares it between threads |
| `xmi:id` in the object (or a flat string arena), reverse map built lazily | S's loader speed; Q keeps resource ID maps today |
| source line per object (side table, loader option) | S's diagnostics |
| loader fast paths from S: borrowed values, `StrMap`, `ParseCache` templates (parsed resources copied in by rebasing ids), bundled resources parsed once | `load-bench` must not regress |
| PCM loader options: tolerant namespaces, `pathmap://` keys, `has_generated_id` | S's behaviour on real-world models |
| journal and id policy as part of the graph | QVTo, direct edits and SPD go through one change path (`DESIGN-SPD.md` 5.3) |
| `MetaLinks` and other maps on FxHash, never iterated in hash order | determinism |
| `serde` behind a feature | snapshots (`DESIGN-SPD.md` 5.7) |

Storage layout: Q's dense slots stay the starting point (Q's semantics, Ecore hooks and
`fixed_slots` optimisation are written for them). If `load-bench` shows more than 5 % loss
that the fast paths above cannot recover, the slots switch to S's sparse layout inside the
crate. `qvto-simoxide/src/raw_part.rs` already proves the full semantics on sparse slots
(contract suite, 300 scripts), and QVTo ran 0.9-1.1x as fast there. Either way the API above
does not change.

### 2.6 SimOxide on the shared graph

- `simoxide-model` keeps: the generated PCM tables (now a `StaticPackages` seed plus typed
  constants `class::*`, `feat::*`), the PCM loader profile (bundled models,
  `MODEL_EXTENSIONS`, memory loading, `load_dir`), the typed model, the builder, validation and
  its own canonical dump (its EMF goldens stay valid).
- `meta::ClassId` etc. become the foundation's ids. SimOxide's static lookups (`c.def()`,
  `f.is_containment()`, ...) stay as an extension trait (`use simoxide_model::meta::PcmMeta;`),
  so call sites keep their syntax. Ids are equal because the generated packages are registered
  first.
- `raw.rs`, `xmi.rs`, most of `load.rs` and `fxhash.rs` go. `Model::graph` becomes
  `simoxide_emf::Graph`.
- Builder value reads: `Value::Int(i64)` becomes `Datum::Int(i32)`/`Long(i64)`; the helpers
  `int`/`dbl`/`s` hide it (`Datum::as_i64`, `as_f64`).
- The generic-supertype bug (`DESIGN-SPD.md` 5.6, phase F0) disappears: the seed uses the
  complete supertypes of the extension tables.

### 2.7 The Q6 adapter after the merge

It becomes unnecessary. QVTo's VM is generic over `S: ModelStore`, and the shared graph is a
`ModelStore`. What moves where:

| Adapter part | After the merge |
|---|---|
| `registry.rs` (S's tables as a registry with equal ids) | `MetaRegistry::from_static` in `simoxide-emf-core` |
| `raw_part.rs` (EMF semantics on S's slots) | deleted (the graph has the semantics); kept in history as the sparse-slot fallback |
| `store.rs` (`Session`, two homes, mirrors, resource table) | deleted; the graph holds all metamodels and resources (`extent:/N`, `qvto:/created`) |
| `journal.rs`, `uuid.rs` | `simoxide-emf::{journal, ids}` |
| tests: contract, registry, scripts, corpus (24 cases on S-loaded models), intermediate | `simoxide-emf` tests (contract with the PCM registry) and `qvto` tests (corpus on `simoxide_model`-loaded graphs) |
| examples `bench.rs`, `runtime_flow.rs` | `crates/qvto/qvto/examples/` until `simoxide-reconf` (`DESIGN-SPD.md` F3) takes over the flow |
| `data/runtimemeasurement.ecore` | `simoxide-reconf` (or `simoxide-model/models/`) |

Until M5 lands, the adapter is a normal workspace crate (`crates/qvto/qvto-simoxide`) and
`simoxide-reconf` can use it. Its API (compile against a registry, run on `&mut` graph, take
the journal) is what F3 codes against, so switching costs the 2-3 days `DESIGN-SPD.md` assumes.

### 2.8 Options considered for the graph

| Option | What | Verdict |
|---|---|---|
| G0 | keep two graphs; share only `jcompat` and the registry; keep the adapter | lowest risk; keeps the two-homes limits, the mirror code and two loaders; does not meet `DESIGN-SPD.md` 5.9 ("several metamodels in one graph") |
| **G1** | `qvto-emf`'s graph becomes the foundation graph; SimOxide's loader fast paths move into it; SimOxide's builder reads it | recommended: QVTo's 518 cases and the EMF oracles keep running on the code they run on today; SimOxide's side is load-time only and has strong goldens |
| G2 | SimOxide's `raw::Graph` becomes the foundation; `qvto-emf` (Ecore hooks, save, URIs) is rewritten onto it | more churn on the side with the larger test surface (518 cases, 544 suite models, Ecore hooks) |

### 2.9 Dependency graph

```
EPL, always built                                   opt-in (exact/compat modes only)
-----------------                                   --------------------------------
simoxide-jcompat ◄────────────────────────────────  simoxide-jcompat-exact   [feature jdk-order]
      ▲                                             simoxide-hotspot-math    [feature hotspot-math, GPL-2.0-only]
simoxide-emf-core ◄── simoxide-emf ◄── simoxide-model ◄── simoxide-sim ◄── simoxide-cli
      ▲                    ▲                             ▲    ▲   ▲
      │                    │              simoxide-stoex ┘    │   └ simoxide-jcompat-exact? (exact)
      │                    │              simoxide-random ────┘     (── simoxide-hotspot-math?)
   qvto-types, qvto-sema   │
      ▲                    │
   qvto-bytecode   qvto-runtime (── jcompat; jcompat-exact? with `compat`) ◄── qvto-vm ◄── qvto
                                                                                         ▲
                       qvto-testkit, qvto-cli          simoxide-reconf [reconfiguration] ┘  ◄── simoxide-spd [spd]
```

`?` marks optional dependencies behind the feature in brackets. No EPL crate depends on an
opt-in crate unconditionally.

- QVTo crates never depend on `simoxide-model`/`-sim`. SimOxide's libraries depend on QVTo only
  through `simoxide-reconf`, behind a feature, off in the libraries, on in the CLI build.
- `simoxide-sim` depends on `simoxide-jcompat` directly (trace numbers, `InsertionOrder`) and
  on `simoxide-jcompat-exact` only with `exact` (Java hash order).
- `qvto-model` and `qvto-emf` keep their names for the QVTo crates through cargo's package
  rename, so no QVTo source changes at first:
  `qvto-model = { path = "crates/foundation/simoxide-emf-core", package = "simoxide-emf-core" }`.
  A mechanical rename of the imports can follow later.

## 3. Target layout

```
simoxide/
  Cargo.toml                      workspace (members below), merged profiles
  .cargo/config.toml              aliases: test-sim, test-qvto, conform
  crates/
    foundation/
      simoxide-jcompat/           EPL
      simoxide-jcompat-exact/     opt-in, GPL-adjacent (licence: D5)
      simoxide-hotspot-math/      opt-in, GPL-2.0-only (moved from crates/)
      simoxide-emf-core/
      simoxide-emf/               incl. oracle/ (CanonDump, EmfOracle, ScriptRunner: today qvto-emf/oracle)
    simoxide-model/ simoxide-stoex/ simoxide-random/
    simoxide-sched/ simoxide-sim/ simoxide-cli/ simoxide-testkit/          (unchanged places)
    simoxide-reconf/ simoxide-spd/ simoxide-snapshot/                     (planned, DESIGN-SPD.md)
    qvto/                         the QVTo home
      README.md                   short: crates, where the docs are
      DESIGN.md                   engine design and decision log (developer document)
      .gitignore                  qvto-rs's, now relative to crates/qvto/
      reference/                  runner sources, inventory, notes (committed);
                                  specs/, eclipse-qvto/, eclipse-ocl/, runner/lib/, runner/build/ (ignored)
      qvto/ qvto-syntax/ qvto-types/ qvto-sema/ qvto-bytecode/ qvto-runtime/
      qvto-vm/ qvto-testkit/ qvto-cli/
      qvto-simoxide/              until M6
  reference/                      SimOxide's refsim, patches, oracles (unchanged)
  docs/                           VitePress, new docs/qvto/ section
  history/qvto-plan.md            qvto-rs PLAN.md (milestone log, not maintained)
```

Notes:

- `crates/qvto/qvto-model` and `qvto-emf` disappear in M4/M5 (their code moves to
  `crates/foundation/`).
- Crate oracles stay with their crates (`qvto-syntax/tools/oracle`, ...). The fixtures stay in
  the crates (`tests/fixtures`, `qvto-testkit/fixtures`, `qvto-testkit/corpus`).
- `docs/simoxide-integration.md` of qvto-rs is superseded by `DESIGN-SPD.md` and this document;
  it moves to `history/`.

**Workspace** (`Cargo.toml`):

```toml
[workspace]
resolver = "3"
members = ["crates/simoxide-*", "crates/foundation/*", "crates/qvto/qvto*"]

[workspace.package]
edition = "2024"
version = "0.1.0"
license = "EPL-2.0"
repository = "https://github.com/nk-coding/simoxide"
rust-version = "1.88"

[workspace.dependencies]
simoxide-jcompat = { path = "crates/foundation/simoxide-jcompat", version = "0.1.0" }
simoxide-emf-core = { path = "crates/foundation/simoxide-emf-core", version = "0.1.0" }
simoxide-emf = { path = "crates/foundation/simoxide-emf", version = "0.1.0" }
# ... existing simoxide-* entries ...
qvto-syntax = { path = "crates/qvto/qvto-syntax" }
qvto-model = { path = "crates/foundation/simoxide-emf-core", package = "simoxide-emf-core" }  # from M4
qvto-emf = { path = "crates/foundation/simoxide-emf", package = "simoxide-emf" }             # from M5
# ... other qvto-* entries ...

[profile.release]
lto = "fat"
codegen-units = 1
panic = "abort"
debug = "line-tables-only"

# qvto conform / fuzz catch panics per case; panic = "abort" would end the whole run.
[profile.release-unwind]
inherits = "release"
panic = "unwind"
```

- `members = ["crates/*"]` must go before the import: with `crates/qvto/` and
  `crates/foundation/` present, the glob names directories that are not packages.
- Profiles are workspace-wide. SimOxide's `panic = "abort"` breaks `qvto conform` and Q7's
  fuzzers (`catch_unwind` in `qvto-cli/src/engine.rs`, `fuzz.rs`, `robust.rs`); build `qvto-cli`
  with `--profile release-unwind`. Tests are not affected (cargo builds tests with unwind).
- qvto-rs's `[profile.dev]`/`[profile.test]` `debug = 0` (disk is tight here): adopt for the
  whole workspace or drop; a user choice, no effect on results.
- `Cargo.lock`: start from SimOxide's lock and let cargo add qvto's packages, so SimOxide's
  locked versions (`quick-xml`, `ryu`, `memchr`, ...) do not move. `hashbrown 0.17` and
  `proptest` come in new.
- Features: see 2.3 (`fast` default; `exact`, `compat`, `jdk-order`, `hotspot-math` opt-in).
  `simoxide-cli` also gets `reconfiguration` (and later `spd`, `snapshot`), on by default in the
  CLI, off in the libraries (as `DESIGN-SPD.md` U2 recommends); they are EPL in fast mode.
  `simoxide-cli`'s and `simoxide-testkit`'s default `hotspot-math` is dropped (default = EPL).
- EPL release check (`docs/development/contributing.md`): `cargo tree -e normal -p
  simoxide-cli` and `-p qvto-cli` with default features list neither `simoxide-jcompat-exact`
  nor `simoxide-hotspot-math`.
- Moving `simoxide-hotspot-math` to `crates/foundation/` changes its path only (the
  `workspace.dependencies` entry and `LICENSES`).

**Paths.** 16 Rust files and about 8 scripts of qvto-rs find the repository root as
`CARGO_MANIFEST_DIR/../..` or `$HERE/../../../..` (for `reference/`, `target/pcm`,
`target/qvto-testkit/java`, `target/qvto-bench`). After the move they need a QVTo root
(`crates/qvto`, one level up from a crate) and the workspace target directory (honour
`CARGO_TARGET_DIR`). One helper in `qvto-testkit` (`paths::qvto_root()`, `paths::target_dir()`)
plus the scripts. `qvto-simoxide`'s path dependency `../../../simoxide-qvto/...` becomes
`simoxide-model.workspace = true`.

**Test commands** (`.cargo/config.toml` aliases). The exactness gates need the opt-in
features, so the aliases turn them on; `cargo test --workspace` alone tests the EPL build:

| Alias | Command | Gate |
|---|---|---|
| `cargo test-sim` | `test --release --features exact,hotspot-math -p simoxide-model -p simoxide-stoex -p simoxide-random -p simoxide-sched -p simoxide-sim -p simoxide-testkit -p simoxide-cli` | all pass |
| `cargo test-sim-epl` | the same without features | all pass; exact-only tests skip with a message |
| `cargo test-fdn` | `test --release -p simoxide-jcompat -p simoxide-jcompat-exact -p simoxide-emf-core -p simoxide-emf` | all pass (EMF oracle fixtures, contract, scripts, JDK differential) |
| `cargo test-qvto` | `test --features compat -p 'qvto*'` (cargo ≥ 1.80 accepts globs with `--workspace`; else list them) | all pass |
| `cargo conform` | `run --profile release-unwind --features compat -p qvto-cli -- conform` and `-- conform --engine qvto-fast` | 518/518 strict (or the count at import) |
| — | `conform --engine qvto-fast` on an EPL build | fast's order-insensitive count unchanged |
| — | `cargo run --release -p qvto-sema --example oracle_report` | 1 102/1 102 |
| — | `crates/simoxide-cli/bench/golden.sh` with a `--features exact,hotspot-math` binary, before and after | identical hashes |
| — | `cargo run --release -p simoxide-model --example load-bench` | within 5 % |
| — | `qvto bench` (Q5b workloads, pinned) | within noise |

CI: SimOxide has only the docs workflow. Tests run locally by the guardrails. Optional: a
GitHub Actions job for `test-fdn` and `test-qvto` on committed fixtures (no Java needed).
`.github/workflows/docs.yml` gets new `paths` if docs include files from `crates/qvto/`
(see 4).

## 4. Docs ("the wiki")

A new top-level section **QVTo** in `docs/qvto/`, with its own sidebar. VitePress supports a
sidebar per path: `themeConfig.sidebar` becomes an object, `'/qvto/'` gets the QVTo sidebar and
`'/'` keeps today's.

| Page | Content | Source in qvto-rs |
|---|---|---|
| `/qvto/` Overview | what the engine is, status (conformance numbers), modes, where it is used in SimOxide | `README.md`, `PLAN.md` exit criteria |
| `/qvto/getting-started` | `qvto run`, `check`, `disasm`; building | `qvto-cli/README.md` |
| `/qvto/library` | `qvto::compile*`, `Vm`, `ModelStore`, running on `simoxide_emf::Graph`, journal, id policy | `qvto/README.md`, `qvto-vm/README.md`, `qvto-simoxide/README.md` |
| `/qvto/blackboxes` | host API: natives, signature tables, `getSimulationTime`, Palladio and Java blackboxes, `ProfilesLibrary` stub | `qvto/src/blackbox/*`, `qvto/data/*.tsv`, DESIGN §12 rows |
| `/qvto/modes` | compat vs fast: the 33-row table, what stays identical, identity hashes and tapes | DESIGN §6 |
| `/qvto/conformance` | Eclipse suite inventory, corpus, strict differential, parser and analyser oracles, fuzzing | `qvto-testkit/README.md`, `reference/inventory/README.md`, `qvto-syntax`/`qvto-sema` READMEs, Q7 results |
| `/qvto/deviations` | known differences from Eclipse (pre-JDK-19 `Double.toString`, LPG repair, `sortedBy` once, non-goals) | DESIGN §2.3, §12 |
| `/qvto/semantics` | Eclipse QVTo/OCL semantics notes | `reference/notes/eclipse-semantics.md` (882 lines; port as is) |
| `/qvto/performance` | benchmark suite, results vs Eclipse warm and cold | `qvto-cli/README.md` "Results" (move the table to a file that the page includes) |
| `/qvto/architecture` | pipeline, crates, typed IR, bytecode, values, VM, model API | DESIGN §3-5, crate READMEs |
| `/qvto/reference-runner` | `qvto-run`, batch and bench modes, tapes, fixtures, the oracles | `reference/README.md`, `qvto-testkit/README.md` |
| `/qvto/development` | layout, test commands, recording fixtures, decision log pointer | README "Building", DESIGN §8, §12 |
| `/development/foundation` | the foundation crates: `jcompat`, EMF core and graph, oracles, who uses what | this document §2 |
| `/guide/builds` (SimOxide guide) | build features and licences: EPL default (fast modes), `exact`/`compat`, `hotspot-math`; which binaries may be distributed | this document §2.3, `LICENSES` |

Also update:

- `index.md` (hero and features: "and a QVTo engine"), the site `description` in
  `config.mts`, nav: `{ text: 'QVTo', link: '/qvto/' }` between Semantics and More.
- `development/contributing.md`: repository layout table, guardrails for QVTo and foundation
  changes, the `release-unwind` profile.
- `correctness/testing.md`: foundation tests; `guide/library.md`: models are a
  `simoxide_emf::Graph`; later the reconfiguration and SPD guide pages (`DESIGN-SPD.md` D).
- `docs.yml` `paths`: add files the QVTo pages include (e.g. `crates/qvto/qvto-cli/bench/results.md`).

What stays out of the site: DESIGN.md's decision log (42 KB, developer-facing, stays in
`crates/qvto/DESIGN.md`), PLAN.md (history), crate READMEs (kept short, pointing to the site).

Navigation sketch:

```ts
nav: [ /* Guide, Performance, Correctness, Semantics */ { text: 'QVTo', link: '/qvto/' }, /* More */ ],
sidebar: {
  '/qvto/': [
    { text: 'QVTo engine', items: [
      { text: 'Overview', link: '/qvto/' }, { text: 'Getting started', link: '/qvto/getting-started' },
      { text: 'Library API', link: '/qvto/library' }, { text: 'Blackboxes and host API', link: '/qvto/blackboxes' },
      { text: 'Compat and fast mode', link: '/qvto/modes' } ] },
    { text: 'Correctness', items: [
      { text: 'Conformance', link: '/qvto/conformance' }, { text: 'Deviations', link: '/qvto/deviations' },
      { text: 'Eclipse semantics', link: '/qvto/semantics' } ] },
    { text: 'Internals', collapsed: true, items: [
      { text: 'Performance', link: '/qvto/performance' }, { text: 'Architecture', link: '/qvto/architecture' },
      { text: 'Reference runner', link: '/qvto/reference-runner' }, { text: 'Development', link: '/qvto/development' } ] },
  ],
  '/': [ /* today's sidebar array */ ],
},
```

## 5. Migration plan

### 5.1 Branch strategy

- Before starting: Q7 is merged into qvto-rs `main`; no agent commits to qvto-rs any more.
  qvto-rs is then archived (README points to SimOxide); its branches `q1b`..`q4a` are merged
  and need not be imported.
- `qvto-adapter` (2 commits, additive) is merged into SimOxide `main` first (also
  `DESIGN-SPD.md` U1).
- The merge runs on a branch `qvto-merge` from that `main`, in its own worktree with its own
  `CARGO_TARGET_DIR`. M0-M4 are additive and merge into `main` as soon as each is green.
- `spd` forks from `qvto-adapter`. Its F0 (supertype fix in `gen_meta.py`, SPD packages
  appended) should land **before** M4, which rebuilds the registry seed from the same
  generator. F1-F3 do not touch `simoxide-model` internals and can run in parallel with M5,
  as long as F2/F3 use the graph only through the adapter API (2.7). M5 must land before F4
  (syncers) depends on graph internals.
- Every step ends at a checkpoint where both suites are green (5.2 "Gate").

### 5.2 Steps

Estimates are developer time for someone who knows both code bases.

| Step | Work | Estimate | Gate (all previous gates too) |
|---|---|---|---|
| M0 | Q7 merged; `qvto-adapter` in SimOxide `main`; SimOxide `members = ["crates/simoxide-*"]`; inventory the Q7 counts (conformance, sema, fuzz regressions) | 0.5 d | SimOxide unchanged (golden hashes) |
| M1 | Import qvto-rs with history to `crates/qvto/` (5.3). Not yet workspace members. | 0.5 d | SimOxide builds and tests as before; `git log -- crates/qvto/qvto-vm` shows qvto-rs history |
| M2 | Workspace integration: members, workspace dependencies, profiles (`release-unwind`), lockfile, `paths` helper and scripts, adapter as member with `simoxide-model.workspace = true`, move the ignored `reference/` bulk (or re-download per `reference/README.md`), regenerate `target/pcm` | 1-2 d | `test-sim`, golden hashes unchanged; `test-qvto`, conformance and sema counts as in M0; adapter tests (contract 40/40, scripts, corpus 24/24) |
| M3a | `simoxide-jcompat` (EPL) and `simoxide-jcompat-exact` (opt-in): split `qvto-runtime/src/java/*` along 2.3; `fxhash.rs`; identity sources (EPL ones from `identity.rs`, HotSpot ones and `idhash.rs` into the exact crate); generic `HashEnv`; `JdkHashOrder` (SimOxide's closed form) with a test against `JavaHashMap`; move `simoxide-hotspot-math` to `crates/foundation/`; switch SimOxide's `javahash.rs`, both `javafmt.rs`, `canon.rs`'s formatter, `raw.rs` parsers, `fxhash.rs` copies, round/max/min. Builds with all features on, so nothing changes yet | 2-4 d | QVTo: `qvto-runtime` JDK differential and golden tests; SimOxide: golden hashes, corpus both modes, `jmath_golden`, trace writing benchmark (`simoxide-bench`) within noise |
| M3b | Feature gating (2.3): `exact` in `simoxide-sim` (`compat::Exact` and later the Slingshot-exact policy), `C::map_order` hook with `InsertionOrder` for fast; `compat` in `qvto-runtime`/`qvto-vm`/`qvto` (`CompatEnv`, Java-ordered collection storage); `ModeUnavailable` errors; EPL defaults in all crates and CLIs; aliases; `LICENSES` and the release check updated | 2-3 d | with features: all exact gates unchanged; without: `test-sim-epl`, QVTo fast tests, `cargo tree` release check clean; fast mode: `Fast` vs `Literal<Fast>` equal, `simoxide-fuzz equiv` passes; `Value` size and `qvto bench` fast within noise |
| M4 | `simoxide-emf-core` from `qvto-model` (package rename keeps `qvto_model::` imports); `MetaRegistry::from_static`; `gen_meta.py` writes the static seed (main + ext tables merged); `meta` ids become foundation ids with the `PcmMeta` extension trait; adapter's `registry.rs` uses `from_static` | 3-5 d | registry equals the `.ecore`-built one (adapter test); `generated.rs` ids unchanged; SimOxide canonical-dump goldens; golden hashes; QVTo counts |
| M5 | `simoxide-emf` from `qvto-emf`: `Arc` registry, `Sync` derived cache, ids in objects, line table, loader fast paths and PCM options, `ParseCache`, journal and id policy from the adapter, FxHash maps, `serde` feature. Then SimOxide on it: `Model::graph`, builder/validate/ir helpers, `canon.rs` over the new read API; delete `raw.rs`, `xmi.rs`, the loader part of `load.rs`. Split: M5a foundation graph changes (QVTo green), M5b SimOxide switch (SimOxide green), M5c delete old code | 2-4 w | `simoxide-model` goldens (EMF 2.33 dumps), `load_all`, `semantics`, `xmi-cases`, `memory`; corpus both modes; golden hashes; `load-bench` within 5 %; `CompiledModel: Send + Sync` (static assertion); qvto-emf suite (544 models, 640 PCM files, scripts vs EMF, contract); QVTo counts; `qvto bench` within noise |
| M6 | QVTo directly on the graph: corpus test on `simoxide_model`-loaded graphs; PRM and SPD metamodels registered into the same graph; delete `qvto-simoxide`; port its tests and examples (2.7) | 3-5 d | the 24 corpus cases on S-loaded models equal the Java runner; journal equals the adapter's on the same runs (record before deleting); runtime-flow example |
| M7 | Docs: `docs/qvto/*`, sidebar, index, contributing, testing, foundation page; `history/` moves | 3-5 d | `npm run build` clean (no dead links) |
| M8 | Optional, with `DESIGN-SPD.md` S0: `simoxide-oraclekit` (warm-JVM batch driver, result directories, fixture bundles) used by refsim, qvto-run and the Slingshot runner | 2-3 d | the three harnesses unchanged in results |

Total: about 5.5-8.5 weeks; without M8 and with M5 at its low end, about 5 weeks. M1-M4 give the
shared Java compat and registry in about 1.5-2.5 weeks with the adapter still in place.

### 5.3 Import commands (M1)

`git filter-repo` is recommended: it rewrites qvto-rs's history so that every commit already
has the new paths. `git log`, `git blame` and `git log --follow` then work on
`crates/qvto/...` without subtree tricks. `git subtree add` keeps the old paths in old commits
(history of `crates/qvto/x` stops at the merge). A squashed copy loses the history.
`git filter-repo` is not installed here; it is a single Python script.

```sh
WORK=/home/devbox/.vmdev/tmp/claude-1000/-home-devbox-workspace/<session>/scratchpad/qvto-import
mkdir -p "$WORK"
python3 -m venv "$WORK/venv" && "$WORK/venv/bin/pip" install git-filter-repo==2.47.0
FR="$WORK/venv/bin/git-filter-repo"

# 1. fresh clone of main only (filter-repo refuses to rewrite a non-fresh clone)
git clone --no-local --single-branch --branch main /home/devbox/workspace/qvto-rs "$WORK/qvto"
cd "$WORK/qvto"

# 2. rewrite paths in every commit (renames apply in order; prefixes match at the path start)
"$FR" --force \
  --path-rename crates/:crates/qvto/ \
  --path-rename reference/:crates/qvto/reference/ \
  --path-rename docs/:crates/qvto/docs/ \
  --path-rename DESIGN.md:crates/qvto/DESIGN.md \
  --path-rename PLAN.md:crates/qvto/PLAN.md \
  --path-rename README.md:crates/qvto/README.md \
  --path-rename LICENSE:crates/qvto/LICENSE \
  --path-rename .gitignore:crates/qvto/.gitignore \
  --path-rename Cargo.toml:crates/qvto/qvto-rs-workspace.toml \
  --path-rename Cargo.lock:crates/qvto/qvto-rs-workspace.lock
# Cargo.toml must not land at crates/qvto/Cargo.toml: a [workspace] there would capture the
# QVTo crates. The renamed files are deleted in M2.
git ls-files | grep -v '^crates/qvto/' && echo "unexpected paths"    # must print nothing

# 3. merge into SimOxide on the merge branch (M0 already narrowed the workspace members)
cd /home/devbox/workspace/simoxide-merge          # worktree of branch qvto-merge
git remote add qvto-import "$WORK/qvto"
git fetch qvto-import main
git merge --allow-unrelated-histories --no-ff qvto-import/main -m "Import qvto-rs with its history under crates/qvto/"
git remote remove qvto-import

# 4. the ignored bulk (not in git): move it, or fetch it again per crates/qvto/reference/README.md
for d in specs eclipse-qvto eclipse-ocl runner/lib runner/build; do
  mkdir -p "crates/qvto/reference/$(dirname $d)"
  cp -a "/home/devbox/workspace/qvto-rs/reference/$d" "crates/qvto/reference/$d"
done
```

Checks after the merge: `git log --oneline -- crates/qvto/qvto-vm | wc -l` matches the count
in qvto-rs for `crates/qvto-vm`; `git status` clean; `cargo test-sim` unchanged (the QVTo
crates are not members yet). The import adds about 440 files and 10 MB.

### 5.4 Risks

| Risk | Mitigation |
|---|---|
| The shared graph loads slower than SimOxide's loader (dense slots, registry lookups instead of static tables) | port S's fast paths first; gate at 5 % on `load-bench`; fallback: sparse slots inside `simoxide-emf` (proven by `raw_part.rs`) |
| `CompiledModel` stops being `Sync` (the `RefCell` cache) | replace the cache in M5a; a compile-time `Send + Sync` assertion in `simoxide-sim` |
| One loader must satisfy two EMF versions (S's goldens from EMF 2.33, Q's from 2.41) | both oracles stay as tests; Q's loader already gives 2.41's dumps on S's corpus; differences go into a table in the foundation docs |
| `Datum` box kinds vs `Int(i64)` in SimOxide's builder | helpers (`as_i64`, `as_f64`); builder tests and canonical-dump goldens |
| Switching SimOxide's Java compat changes an exact output | algorithms are the same; golden hashes and corpus after each switch; keep `insert_only_order` for S's call sites |
| `panic = "abort"` breaks QVTo tools that catch panics | `release-unwind` profile |
| Feature gating leaks: an EPL build pulls in the exact crate through a forgotten default, or `#[cfg]` splits make fast mode slower | `cargo tree` release check in the guardrails; `test-sim-epl` and an EPL `qvto` build in every step from M3b; benchmarks of the EPL build |
| Users get a non-exact default where they expected SimOxide's exact default | `ModeUnavailable` names the feature; README, getting-started and CLI help say how to build the exact modes |
| Relative paths in qvto-rs tests and scripts break after the move | one `paths` helper, done in M2, all qvto tests as the gate |
| The `spd` branch and M4/M5 both change `simoxide-model` | order: F0 before M4; F2/F3 against the adapter API; M5 before F4 |
| Workspace-wide `cargo test --release` gets slower (QVTo tests, fixtures) | aliases per area; guardrails name the relevant alias |
| Q7 still changes qvto-rs | import only after Q7 is merged; record the counts in M0 |
| Hidden behaviour differences between the adapter and the shared graph (proxies' resource, `xmi:id` of detached objects, journal side effects) | record journals and dumps of the 24 corpus cases with the adapter before M6; compare after |

### 5.5 Decisions for the user

| # | Decision | Recommendation |
|---|---|---|
| D1 | History import: `filter-repo` rewrite, `git subtree`, or one squashed commit | `filter-repo` (5.3) |
| D2 | Where the QVTo home is: `crates/qvto/` (crates, `reference/`, DESIGN) or a top-level `qvto/` | `crates/qvto/` (matches the request and `DESIGN-SPD.md`) |
| D3 | QVTo crate names: keep `qvto-*` | keep (the engine stays usable on its own; no source churn) |
| D4 | Foundation crate names: `simoxide-jcompat`, `simoxide-emf-core`, `simoxide-emf`, or neutral names (`jcompat`, `emf-core`, `emf`) | the `simoxide-` prefix (one project; avoids crates.io clashes) |
| D5 | Licence of `simoxide-jcompat-exact` (OpenJDK `HashMap` order incl. SimOxide's `javahash.rs`, HotSpot identity hashes). The code was written from behaviour and tests, not copied, but its details come only from OpenJDK/HotSpot sources. Options: (a) EPL-2.0 with the provenance in `LICENSES` (isolated anyway); (b) GPL-2.0-only WITH Classpath-exception-2.0, the licence of the OpenJDK classes it follows: exact binaries without `hotspot-math` stay distributable, the source is GPL; (c) GPL-2.0-only like `hotspot-math`: no exact binary may be distributed; (d) clean-room rewrite of the tree-bin part from black-box tests | (b): matches the user's rule (exact crates carry the GPL side) and keeps an exact build distributable. Tree bins matter only for ≥ 9 colliding keys at capacity ≥ 64 |
| D5a | How wide the GPL-adjacent side is: conservative (all `HashMap`-structure emulation incl. SimOxide's 84-line closed form, 2.3) or narrow (only tree bins, JDK-version sizing and HotSpot's identity-hash constants; basic bucket order stays EPL) | conservative: then no fast mode and no default build touches any `HashMap` emulation |
| D5b | Exact modes without their feature: unavailable with an error (recommended), or a labelled "approximate exact" fallback (insertion order) | unavailable; the one documented exception stays `exact` without `hotspot-math` (correctly rounded log/exp), as today |
| D5c | Defaults: all crates and CLIs EPL-only (fast default), with developer aliases for exact builds; this drops today's `hotspot-math` default of `simoxide-cli`/`simoxide-testkit` and makes fast SimOxide's default mode | yes (follows the licensing rule); alternatively keep exact defaults in the CLIs only, which would make the default CLI binary non-EPL |
| D6 | Replace SimOxide's own Java-compat code with `simoxide-jcompat` | yes, step by step, gated by the golden hashes |
| D7 | One graph (G1, M5) now, or keep two graphs and the adapter for a while (G0) | G1, after F0; M1-M4 first |
| D8 | Canonical dumps: keep both formats | keep both (both have committed EMF goldens) |
| D9 | `debug = 0` for dev/test builds workspace-wide | yes on this machine (disk) |
| D10 | `reconfiguration`/`spd`/`snapshot` as optional features | optional, on in the CLI (as `DESIGN-SPD.md` U2) |
| D11 | Where PLAN.md and `docs/simoxide-integration.md` go | `history/` (not maintained); DESIGN.md stays in `crates/qvto/` |
| D12 | A test CI job (none exists today) | optional: foundation and QVTo tests on committed fixtures |

## 6. Interactions with planned SimOxide work

`DESIGN-SPD.md` (branch `spd`) plans runtime model changes, a Slingshot flavour, SPD, and
snapshot/restore, and lists requirements for the shared graph (its 5.9). How the foundation
meets them:

| Requirement (`DESIGN-SPD.md` 5.9) | Foundation |
|---|---|
| Mutable per run, cheap to share | `Graph: Clone`; registry behind `Arc`. First version: plain clone on the first change (MediaStore: 1 220 objects, tens of µs). Later, if forks need it: objects in `Arc` chunks with copy on write, behind the same API |
| Change journal | in `simoxide-emf` (Q6 semantics: effective changes, side effects included); off = one branch per write; consumers keep a cursor (journal length); the loader bypasses it |
| Typed access by id | object ids are stable; SimOxide's typed entities keep their `obj` |
| Stable, deterministic ids | append-only arena; `EcoreUtil.delete` detaches, never frees; `IdPolicy::Uuid { seed }` with the generator state inside the graph (cloned and serialised with it) |
| Deterministic iteration | no hash-order iteration anywhere in the graph; Java orders through `simoxide-jcompat-exact` in the exact modes, insertion order in the fast modes |
| Clone = value | no `Rc`/`RefCell` in the graph (the derived cache changes in M5a) |
| Serialisable | `serde` feature on `emf-core` and `emf`; XMI save exists (`xmi_save.rs`), which answers `DESIGN-SPD.md` U10 |
| Several metamodels in one graph | the dynamic registry: PCM seed first, Ecore, then PRM, SPD, semanticspd, user metamodels; references in both directions; no mirrors |
| Resources | the graph's resource table holds `extent:/N`, `qvto:/created` and per-file outputs |
| Fast read path | `&mut Graph` per run, no locks; the simulator reads compiled tables, never the graph |

Further notes:

- **Registry growth.** Metamodels are added when a `CompiledModel` (or a QVTo host) is set up,
  not during runs, so runs share one frozen `Arc<MetaRegistry>`. QVTo's intermediate classes are
  added at compile time (Q4b-4), also before runs.
- **Slingshot flavour and PCM versions.** Slingshot runs Palladio 6.0. SimOxide maps older PCM
  namespaces to 5.2 today (`tolerant_namespaces`). If PCM 6.0 models ever differ in structure,
  the dynamic registry can load their `.ecore` files; the typed builder would need a mapping.
  Not needed for the current plans.
- **Identity-hash order (SPD quirk Q8).** `simoxide-jcompat-exact::hotspot` gives the HotSpot
  generator (Slingshot-exact builds only); Slingshot-exact uses that surrogate order and documents
  it as a deviation; fast modes use key-derived hashes.
- **Two exact modes, several fast modes.** SimuLizar-exact and Slingshot-exact are both behind
  `exact` and share `simoxide-jcompat-exact` (and optionally `hotspot-math`). Slingshot's
  `Math.random` stream is `java.util.Random`, which the javadoc specifies, so it sits in the EPL
  crate and fast Slingshot-style runs can use it too. Anything Slingshot-exact needs beyond that
  (e.g. `ConcurrentHashMap` or Guava iteration orders, a JDK 17 target) goes into the exact
  crate.
- **Exact reconfiguration (D5 of the integration plan).** Deterministic UUIDs come from the
  graph's id policy, so the same seed gives the same ids in Rust and in a patched refsim.
- **`simoxide-reconf`** (F3) is created against the adapter's API and switched to the graph
  after M6. The per-run QVTo state should be created on the first reconfiguration (the Q6
  session cost 2.4 ms, mostly for the second home and metamodel objects; with one graph only the
  metamodel objects for `eClass()` remain, and they can be built once per `CompiledModel`).
- **Snapshots** (N1). The graph delta against the base model plus the journal cursor is the
  model part of `RunState`. The XMI writer gives the readable form.

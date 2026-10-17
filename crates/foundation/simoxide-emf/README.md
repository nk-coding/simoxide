# simoxide-emf

The EMF graph shared by SimOxide and its QVTo engine (formerly qvto-rs's `qvto-emf`; the QVTo
crates and `simoxide-reconf` import it as `qvto_emf` through a package rename, and SimOxide's
`simoxide_model::raw::Graph` is this `Graph`). Overview: `docs/development/foundation.md` (the
documentation site's "Foundation crates" page).

A model store (`Graph`) with EMF's dynamic-object semantics,
Ecore built in (with the behaviour of its generated classes), `.ecore` metamodels registered
from loaded resources, XMI load and save byte for byte like EMF 2.41, resource sets with URI
maps and proxy resolution, the canonical dump (strict comparison) and the Eclipse suite's
lenient comparer; the change journal and id policies. Depends on `simoxide-emf-core`
(imported as `qvto_model`), `simoxide-jcompat`, `quick-xml`, `memchr`.

A host with fixed metamodel ids (SimOxide's generated tables) builds its registry first and
lets Ecore follow (`Graph::with_registry(meta)`, `ResourceSet::with_graph`); `materialize`
gives such packages their metamodel objects.

```rust
use qvto_emf::{ResourceSet, xmi_save::{save, SaveOptions}, canon::dump};
let mut rs = ResourceSet::new();
rs.map_uri("platform:/plugin/org.palladiosimulator.pcm/", "file:/.../org.palladiosimulator.pcm/");
rs.load_metamodel("platform:/plugin/org.palladiosimulator.pcm/model/pcm.ecore")?; // registers packages
let r = rs.load_file(Path::new("my.repository"))?;   // load errors: rs.graph.resource(r).errors
rs.resolve_all();                                      // EcoreUtil.resolveAll
let xmi = save(&rs.graph, r, &SaveOptions::for_resource(rs.graph.resource(r)))?;
let canonical = dump(&rs.graph, &[r]);                 // strict comparison text
```

## Layout

| File | Contents |
|---|---|
| `src/graph.rs` | `Graph`: objects with one slot per feature (`eAllStructuralFeatures` order), resources, `xmi:id` maps; the EMF internals (`eBasicSetContainer`, inverse add/remove, `eSetResource`, `EcoreUtil.delete`); `impl ModelStore` |
| `src/ecore.rs` | Ecore's registry entries, bootstrapped from `models/Ecore.ecore` (the runtime `EcorePackage` as EMF holds it, saved by the oracle); `EcoreIds` |
| `src/ecore_hooks.rs` | generated-code behaviour of Ecore's classes: `eType`/`eGenericType`, `eSuperTypes`/`eGenericSuperTypes`, `eExceptions` views; `isSet` of these and of `instanceClassName`/`instanceTypeName`; `EEnumLiteral.literal`; derived features (`eAllSuperTypes`, `eAllStructuralFeatures`, `eAllAttributes`, `eReferenceType`, `many`, `eIDAttribute`, ...) computed from the model objects; erasure of type parameters |
| `src/register.rs` | `EPackage` objects of loaded resources -> registry entries, linked both ways (`MetaLinks`: `EClass` object <-> `ClassId`, ...; also `ModelStore::classifier_object` etc.) |
| `src/materialize.rs` | `ResourceSet::materialize`: Ecore objects (`EPackage`, `EClass`, features, enums, data types, operations) for registry entries built in code, linked like registered `.ecore` files, under given locations; `describe_packages` (a name-based description of registry entries, for comparing registries) |
| `src/operations.rs` | `meta_operations`: the `EOperation`s of the registered classes read from their `.ecore` objects, for `qvto_sema::compile_with_operations` (load `ResourceSet::ecore_resource()` first for Ecore's own) |
| `src/datatypes.rs` | `createFromString` / `convertToString` per data kind (`EcoreFactoryImpl` vs reflective `EFactoryImpl`), Java `Double.toString`/`Float.toString`, intrinsic defaults |
| `src/uri.rs` | port of EMF's `URI` (parse, `resolve`, `deresolve`, file URIs) |
| `src/fragment.rs` | URI fragments: IDs, `/0/@f.1` paths, `eKeys` predicates, Ecore's named segments (`//Class/feature`, `%source%`) |
| `src/xmi_load.rs` | XMI reader (adapted from SimOxide's `simoxide-model/src/xmi.rs`): IDREF forward references, `mustAdd`, same-document proxies, `href` proxies, encoded attribute style, `xsi:schemaLocation`, `xmi:id`, XMI 2.1 |
| `src/xmi_save.rs` | XMI writer (port of `XMLSaveImpl`, `XMISaveImpl`, `XMLString`, `Escape`, parts of `XMLHelperImpl`): feature kinds, `xsi:type` rules, `href`/IDREF, namespace prefixes, line width and late namespace declarations, schema locations |
| `src/resource_set.rs` | `ResourceSet`: URI prefix maps, namespace locations, aliases, demand loading, proxy resolution, metamodel registration (with the metamodels it depends on) |
| `src/config.rs` | resource-set configuration file shared with the Java oracle (`map`, `mmroot`, `alias`) |
| `src/chunks.rs` | object storage in chunks with a shared frozen prefix (`Graph::freeze`; copy on write for forks) |
| `src/journal.rs` | the change journal (`Journal`, `Change`, `ChangeKind`, `Summary`), recorded inside the EMF operations |
| `src/ids.rs` | id policies (`IdPolicy`): EMF-like UUIDs from a seeded generator for new objects of a target class |
| `src/load_ops.rs` | loader-level writes (not recorded in the journal) and plain reads, used by SimOxide's PCM loader |
| `src/canon.rs` | canonical dump, first difference, FNV-64 |
| `src/compare.rs` | port of the suite's `TreeComparator` (verdict) |
| `oracle/` | the EMF oracle (Java, reference runner's jars): saves, dumps, scripts, pairs (`run.sh`) |
| `tools/pcm-setup.sh` | extracts the PCM/SPD/SimuLizar metamodels and lists the corpus models |
| `examples/emf-tool.rs` | load a model with a configuration and print errors, save or dump |

## Canonical dump

One JSON line per resource and per object, objects in containment-tree order:

```text
{"resource":"<label>","roots":N}
{"path":"/0/@f.1","type":"<nsURI>#<Class>","attrs":{...},"refs":{...},"set":[...]}
```

`attrs` holds every non-transient, non-derived attribute (defaults included) as
`convertToString` text, `refs` every non-empty, non-transient, non-derived, non-containment,
non-container reference (targets `<label>#<path>`, `!<path>` outside resources,
`?<last segment>#<fragment>` for proxies), `set` the unsettable features that are set. Paths always
use `@feature.index` and the root index, so the dump is order-sensitive and ID-independent. The
Java side is `oracle/src/CanonDump.java` (for the reference runner's `--dump`, Q1d).

## Verification

All against EMF 2.41 (standalone, the reference runner's jars) through `oracle/`:

| Test | What | Result |
|---|---|---|
| `tests/suite_models.rs` | the 546 model files of the Eclipse QVTo test suite (503 `.ecore`, SimpleUML, RDB, XMI, UML, `testqvt`, trace, AST): load errors, XMI save with the resource's default options, canonical dump after `resolveAll` | 544 equal to EMF (save byte for byte, dump identical, same error counts); 2 excluded (below) |
| same, `pcm_models` | 640 PCM-side files: SimOxide's corpus (66 directories, 7 models each), Palladio's default models, Slingshot/SPD examples and the SPD and SimuLizar metamodels | 640 equal (needs `tools/pcm-setup.sh target/pcm` and the oracle run; skipped otherwise) |
| `tests/scripts.rs` | 300 random scripts (15 321 operations: set, unset, add, insert, remove, move, set at index, clear, `setEList`, roots, delete, with wrong types, nulls and bad indexes) on dynamic EMF objects, then each resource saved with default options and with the runner's options (UTF-8, schema location) | every result, the final state and all 1 200 saves equal |
| `tests/compare.rs` | the suite's lenient comparer on 1 663 pairs: `in`/`expected` per test folder, each model with itself, each model with a perturbed copy (list reversed, attribute changed, object deleted, reference retargeted) | 1 662 verdicts as Java's `TreeComparator`, 1 skipped (Java's comparer throws); of 721 lenient-equal pairs, 174 differ strictly |
| `tests/contract.rs` | `simoxide-emf-core`'s contract suite, with the metamodel built in code and loaded from `.ecore` | 40 tests |
| `tests/materialize.rs` | the contract suite with Ecore registered after the contract package; materialized objects saved and registered again give the original registry (and that of `contract.ecore`); `.ecore` files referring to materialized packages register | pass |
| `tests/registry.rs` | registering `contract.ecore` gives the entries `build_metamodel` builds; built-in Ecore entries linked to Ecore's objects; derived `eAllStructuralFeatures` equals the registry | pass |

Excluded: `Stdlib.xmi` and `bug449912/expected.qvtoperational` are instances of the QVTo AST
metamodel, whose classes are dynamic subclasses of Ecore's generated classes (`Library extends
EClass`); EMF itself stores their features in the wrong slots, so its save and dump are not
meaningful (in Eclipse these classes are generated).

Regenerating the oracle data (from the QVTo root `crates/qvto`, where the reference runner's
jars and `target/` are; `O=target/emf-oracle`, `F=../foundation/simoxide-emf`):

```sh
$F/oracle/build.sh
$F/oracle/run.sh models $F/tests/data/suite.conf $F/tests/data/suite-models.txt $O/suite
cp $O/suite/results.tsv $F/tests/data/suite-oracle.tsv
QVTO_EMF_WRITE_SCRIPTS=$O/scripts-in cargo test -p simoxide-emf --test scripts   # writes the scripts
$F/oracle/run.sh scripts /dev/null ../foundation/simoxide-emf-core/src/contract.ecore $O/scripts-in $O/scripts
cp $O/scripts/results.tsv $F/tests/data/scripts-oracle.tsv
QVTO_EMF_WRITE_PAIRS=$O/pairs.txt cargo test -p simoxide-emf --test compare     # writes the pairs
$F/oracle/run.sh pairs $F/tests/data/suite.conf $O/pairs.txt $F/tests/data/compare-oracle.tsv
$F/tools/pcm-setup.sh target/pcm      # pcm.conf holds absolute paths: once per target directory
$F/oracle/run.sh models target/pcm/pcm.conf target/pcm/pcm-models.txt $O/pcm
```

With `QVTO_EMF_ORACLE`, `QVTO_EMF_ORACLE_PCM`, `QVTO_EMF_ORACLE_SCRIPTS` pointing at the oracle
directories, failing tests print the first differing line.

## Semantics and differences from EMF

- Every class behaves as a dynamic EMF class (unique reference lists only when `unique`,
  containments that resolve proxies keep a root in its resource, `set`/`unset` check
  changeability), except Ecore's own classes, which emulate their generated implementations
  (see `ecore_hooks.rs`). Difference: dynamic EMF reference lists accept any Java object (and
  fail later); here they accept objects only (`TypeMismatch`).
- Proxies are resolved eagerly (`ResourceSet::resolve_all`, `resolve`), not on read: `get` is a
  pure read. Unresolvable proxies stay proxies (`is_proxy`, `proxy_uri`).
- Metamodels are found by namespace URI in the resource set's locations (a configuration with the
  same rules as the oracle: metamodels in the document's directory first, then an index of
  `.ecore` files by nsURI, files under mapped directories known by their mapped
  `platform:/plugin` URIs), through `xsi:schemaLocation` (the package at the location serves the
  namespace whatever its nsURI), or registered explicitly. EMF's global registry of generated
  packages has only Ecore here (no XMLType, UML, ...).
- `platform:/plugin/org.eclipse.emf.ecore/model/Ecore.ecore` is the Ecore package only when
  configured as an alias (both configurations do).
- `xsi:` attributes are recognised by their literal prefix (EMF parses without namespace
  awareness).
- Values of data types without a loadable Java class become `null` on load, as in EMF; the store
  keeps `BigDecimal`, `Date` and other Java-class values in lexical form. Attributes of data
  types whose instance class is `java.lang.Number` (numbers) or `java.lang.Object` (numbers,
  booleans, characters) accept those boxes, as EMF's `isInstance` check does (e.g. a type
  parameter bounded by a `Number` data type).
- Ecore's derived lists (`eAllStructuralFeatures`, `eAllSuperTypes`, `eAttributes`, ...) are
  cached per object and feature, as `EClassImpl` caches them; any change of an object of Ecore's
  classes (`Graph::touch`, called by every slot, container and proxy write) invalidates the
  cache, and the `eAll*` lists are built from the supertypes' cached lists (DESIGN §12 Q5b-3).
  `eContents` reads the slots directly; the uniqueness check of a containment list asks the
  child's container (`EObjectContainmentEList.contains`), other lists scan the slot.
- The dynamic-subclass anomaly above is not reproduced.
- A reference whose `eOpposite` is set on one side only (as QVTo declares `opposites` of
  intermediate class features) behaves as in EMF: changing it updates the other end's value;
  a containment with such an opposite sets that value on the child instead of making it
  contained (`tests/one_sided_opposites.rs`, checked against EMF with
  `tests/data/OneSided.java`).
- `ModelStore::extend_meta` adopts a registry that extends the graph's (a program's
  intermediate classes); the new classes have no metamodel objects.
- `operations::meta_operations` also reads the OCL `body` of operations of dynamic classes and
  `operations::meta_derivations` the `derive` of their features (`http://www.eclipse.org/2007/OCL`
  annotations, Eclipse QVTo's `OCLAnnotationSupport`).

## Licence

EPL-2.0. Ports of EMF code and SimOxide's loader: see `LICENSES`.

# simoxide-emf-core

The EMF core shared by SimOxide and its QVTo engine (formerly `qvto-model`; the QVTo crates
import it as `qvto_model` through a package rename): metamodels resolved to dense ids, the
`ModelStore` trait through which the QVTo VM reads and writes models with EMF's semantics, and
the static seed of generated tables. No dependencies. Implementations of `ModelStore`:
`simoxide-emf` (the EMF graph shared by SimOxide and QVTo, imported by the QVTo crates as
`qvto_emf`). Overview: `docs/development/foundation.md` (the documentation site's "Foundation
crates" page).

SimOxide's generated PCM tables (`simoxide-model/src/meta/generated.rs`, written by
`simoxide-model/tools/gen_meta.py`) are a `seed::StaticPackages`. `MetaRegistry::from_static`
registers them first, so every table index is the registry id (`simoxide_model::meta::ClassId`
*is* `simoxide_emf_core::ClassId`); the graph crate registers Ecore after them and resolves the
tables' Ecore references (`MetaRegistry::resolve_static_ecore`). `static_operations` and
`static_top_packages` give the tables' `EOperation`s and package locations.

```rust
use qvto_model::{Datum, ModelStore};
let o = store.create(class)?;                       // ModelResult: EMF's exceptions as errors
store.set(o, name, Datum::Str("a"))?;               // single-valued
store.add(o, children, Datum::Obj(child), None)?;   // EList.add (Ok(false) on duplicates)
store.add(o, children, Datum::Obj(c2), Some(0))?;   // EList.add(index, v)
for i in 0..store.len(o, children) { let d = store.get_at(o, children, i); }
let mut all = Vec::new(); store.get_list(o, children, &mut all);   // the whole list in one call
store.set_list(o, tags, &[Datum::Str("x")])?;       // ECollections.setEList
```

## Layout

| File | Contents |
|---|---|
| `src/ids.rs` | `PackageId`, `ClassId`, `FeatureId`, `DataTypeId`, `EnumId`, `ResourceId` (dense `u32`) |
| `src/meta.rs` | `MetaRegistry`: packages, classes, features, data types, enums; derived class tables as EMF computes them (`eAllSuperTypes`, `eAllStructuralFeatures` = slot order, containments, cross references, ID attribute, supertype bit sets); `DataKind` (the Java class of values); `slot` is O(1) for features whose slot is the same in every class (single inheritance, `finish_classes` computes them), else a search of the class's blocks |
| `src/datum.rs` | `Datum` (borrowed value) and `OwnedDatum`; Java box classes kept apart (`Int`, `Long`, `Short`, `Byte`, `Char`, `Double`, `Float`) with Java `equals` |
| `src/store.rs` | the `ModelStore` trait with its contract, `ModelError`, provided `set_list`, `contents`, `all_contents`, `resource_contents`; optional metamodel objects (`classifier_object`, `object_classifier`, `feature_object`, `object_feature`, `package_object`, `object_package`: the `EClass`/`EStructuralFeature`/`EPackage` objects of registry entries, for `eClass()` and Ecore's EOperations; default none) |
| `src/seed.rs` | static tables (`StaticPackages`, `StaticClass`, `StaticFeature`, ...) and their ingestion (`MetaRegistry::from_static`, `resolve_static_ecore`, `static_operations`, `static_top_packages`) |
| `src/operation.rs` | `MetaOperation`, `MetaMultiplicity`: the `EOperation`s a host gives the analyser (the registry keeps none) |
| `src/contract.rs`, `src/contract.ecore` | the contract test suite (20 checks) and its metamodel; `contract_tests!` generates one `#[test]` per check for a store |

## The contract

The trait's doc comment states the rules; they are those of dynamic EMF objects
(`DynamicEObjectImpl`, the setting delegates of `EStructuralFeatureImpl`, `EcoreEList.Dynamic`):
defaults (literal, else the type's intrinsic default, first enum literal), `isSet` (non-unsettable
data: differs from the default; explicit null of a feature with a non-null default), uniqueness
(append returns `false`, insert and `set` at an index fail), the order of checks (index, then
uniqueness, then null and type), containment moves, opposites (one-to-one, one-to-many,
many-to-many, displaced single values), container references (`set`, `unset` detaches from any
container), roots and cross-resource containment (containments that resolve proxies keep the
child in its resource), `EcoreUtil.delete`, `ECollections.setEList`, changeability of `set` and
`unset`.

A store whose metamodel is fixed (generated tables) fills `contract::Roles` with features of its
own metamodel that have the properties of the contract metamodel's features, and runs the same
checks. The checks need all roles on one class with the contract's defaults, which no PCM class
has; SimOxide's PCM graph is the same `simoxide-emf` graph, whose code is registry-driven, so the
suite on that graph covers it.

## Verification

- `contract_tests!` runs on `simoxide-emf`'s `Graph` twice: with the contract metamodel built in code
  and loaded from `contract.ecore` (40 tests).
- The same semantics are checked against real EMF by `simoxide-emf`'s differential script test
  (300 random scripts, 15 321 operations, every result and the final state equal; see
  `../simoxide-emf/README.md`).

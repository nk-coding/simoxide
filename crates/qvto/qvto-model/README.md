# qvto-model

The model API of the engine: metamodels resolved to dense ids and the `ModelStore` trait through
which the VM reads and writes models with EMF's semantics. No dependencies. Implementations:
`qvto-emf` (own Ecore/XMI store); later an adapter over SimOxide's `raw::Graph`.

```rust
use qvto_model::{Datum, ModelStore};
let o = store.create(class)?;                       // ModelResult: EMF's exceptions as errors
store.set(o, name, Datum::Str("a"))?;               // single-valued
store.add(o, children, Datum::Obj(child), None)?;   // EList.add (Ok(false) on duplicates)
store.add(o, children, Datum::Obj(c2), Some(0))?;   // EList.add(index, v)
for i in 0..store.len(o, children) { let d = store.get_at(o, children, i); }
store.set_list(o, tags, &[Datum::Str("x")])?;       // ECollections.setEList
```

## Layout

| File | Contents |
|---|---|
| `src/ids.rs` | `PackageId`, `ClassId`, `FeatureId`, `DataTypeId`, `EnumId`, `ResourceId` (dense `u32`) |
| `src/meta.rs` | `MetaRegistry`: packages, classes, features, data types, enums; derived class tables as EMF computes them (`eAllSuperTypes`, `eAllStructuralFeatures` = slot order, containments, cross references, ID attribute, supertype bit sets); `DataKind` (the Java class of values) |
| `src/datum.rs` | `Datum` (borrowed value) and `OwnedDatum`; Java box classes kept apart (`Int`, `Long`, `Short`, `Byte`, `Char`, `Double`, `Float`) with Java `equals` |
| `src/store.rs` | the `ModelStore` trait with its contract, `ModelError`, provided `set_list`, `contents`, `all_contents`, `resource_contents` |
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
checks.

## Verification

- `contract_tests!` runs on `qvto-emf`'s `Graph` twice: with the contract metamodel built in code
  and loaded from `contract.ecore` (40 tests).
- The same semantics are checked against real EMF by `qvto-emf`'s differential script test (300
  random scripts, 15 321 operations, every result and the final state equal; see
  `crates/qvto-emf/README.md`).

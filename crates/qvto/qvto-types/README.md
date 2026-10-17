# qvto-types

The type system of the engine: interned OCL/QVTo types (`TypeTable`), QVTo's library classes
registered in the program's `MetaRegistry` (`StdClasses`), and the standard library table
(`StdOp`) shared by the analyser and the VM.

## Standard library table

Generated, not written by hand:

1. `tools/oracle/StdlibDump.java` loads Eclipse QVTo 3.11's library (classic OCL's predefined
   types and QVTo's `QvtOperationalStdLibrary`) and prints every operation: the predefined
   operations of each OCL type (`oclOperations()`, in order), the additional operations QVTo's
   library environment defines per owner type (in definition order), and the operations of the
   library classes (`Element`, `Model`, `Status`, `Exception`, `Transformation`) and module.
   Flags: static, deprecated (by), unsupported (with Eclipse's reason).
   `tools/oracle/run.sh StdlibDump > data/stdlib.tsv`
2. `examples/gen_stdlib.rs` turns `data/stdlib.tsv` into `src/stdlib_data.rs`: the `StdOp` enum
   (one variant per classic OCL signature, one per QVTo handler), the descriptor table and the
   per-owner lists in Eclipse's order. `cargo run -p qvto-types --example gen_stdlib [-- --check]`.

## Types and conformance

`TypeTable` interns types by structure and has fixed ids for the predefined ones
(`TypeId::OCL_ANY`, `INTEGER`, ..., `T`, `T2`, `KEY_T`, `COMMON_T`, `OCL_TYPE` = `TypeType(T)`,
`NULL` = "no type", Eclipse's `null` classifier). Collections, `List`, `Dict`, tuples (part
order kept), type types, modules and model types (both are `EClass`es in Eclipse), metamodel
classes, data types and enumerations are types; `List` and `Dict` are collection types of kind
`Collection`, as in Eclipse.

`check::Checker` is Eclipse's type checker over a table and a registry: `relationship`
(`TypeCheckerImpl.getRelationship` with QVTo's rules for `List`, `Dict`, tuples, modules and model
types, then classic OCL's), `compatible`/`exact`, `assignable`
(`QvtOperationalParserUtil.isAssignableToFrom`), `common_super_type` with OCL's errors, the OCL
type of an `EDataType` by instance class, and the names Eclipse prints (`name`,
`full_name` = `getTypeFullName` with the module prefix for collection and tuple types,
`qualified_name`).

## Blackbox signatures

`data/blackboxes.tsv` lists, per test environment, the Java blackbox units of the Eclipse suite
and the SimuLizar/SPD stubs with the module and operation signatures Eclipse derives from the
Java classes (`tools/oracle/BlackboxDump.java`); the semantic analysis tests build
`qvto_sema::BlackboxUnit`s from it.

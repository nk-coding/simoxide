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

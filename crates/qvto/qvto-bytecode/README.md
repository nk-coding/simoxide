# qvto-bytecode

The bytecode of the engine: a register instruction set (`Insn`, at most 8 bytes), the compiler
from `qvto-sema`'s typed IR, constant pools and side tables, and a disassembler. Status: Q3a
(see `PLAN.md`).

```rust
let program = qvto_bytecode::compile(ir_program)?;   // qvto_sema::ir::Program
print!("{}", qvto_bytecode::disasm::program(&program));
```

## Layout

| File | Contents |
|---|---|
| `src/insn.rs` | `Insn` (≤ 8 bytes, checked at compile time), `Reg`, collection kind codes |
| `src/program.rs` | `Program` (the IR's metadata + chunks), `Chunk`, `Site` (operands that do not fit an instruction), `OpCode` (an operation's chunks), `Const`, `LoopInfo`, `Handler` |
| `src/compile.rs` | the compiler |
| `src/disasm.rs` | listings |

## Format

- **Frames.** One frame per operation call: the operation's variables first (`LocalId` =
  register), then temporaries allocated as a stack. Mapping sections (`when`, `where`, `init`,
  population, `end`) are separate chunks on the same frame. Code outside operations
  (initialisers, model type `where`, OCL bodies of metamodel operations and OCL derivations:
  `Program::meta_bodies`, `Program::derivations`) gets chunks with frames of their own.
- **Instructions** have an opcode and up to three 16-bit operands, or a register and a 32-bit
  operand (jump targets, constants, globals, types). Everything else (feature ids beyond
  `u16`, names, flags, static types) is in the chunk's site table, indexed by a 16-bit operand.
- **Constants** are a program-wide pool (`Const`); runtime values are built from them at the
  start of a run, so a `Program` is `Send + Sync`.
- **Calls** put the receiver (or null) and the arguments into consecutive temporaries; the VM
  moves them into the callee's frame. Out arguments of mapping calls come back in the argument
  registers and are written to their variables or properties by the caller's code.
- **Iterators** (OCL and imperative) are inline loops: `IterInit`/`IterNext` over a slot of
  the frame's iteration stack; several iterator variables become nested loops with the first
  variable varying fastest (classic OCL's `IterationTemplate`); `closure` uses a depth-first
  work stack (`ClosureInit`/`ClosureNext`/`ClosurePush`); `sortedBy` collects elements and keys
  and sorts once (`SortBy`).
- **Control flow**: `and`/`or`/`implies` compile to jumps with classic OCL's lax null rules;
  `if`/`switch` jump to an `invalid` result on undefined conditions; `break`/`continue` are
  jumps; `return` is `Return` (inside an `except` body it leaves the `try`, inside a top-level
  `init`/`end` statement it ends that statement, as Eclipse drops the result there); `try`
  ranges and their `except` clauses are per-chunk handler tables.
- **Debug information**: per instruction the byte offset of Eclipse's *current instruction
  pointer* (the last AST node entered in evaluation order, reset to the call after a call
  returns), from which stack traces and assertion locations take their lines.

## Instruction groups

| Group | Instructions |
|---|---|
| moves, constants | `Move`, `LoadNull`, `LoadInvalid`, `LoadBool`, `LoadInt`, `LoadConst`, `LoadDefault`, `LoadThis`, `LoadModel`, `LoadGlobal`, `StoreGlobal`, `LoadInstGlobal`, `StoreInstGlobal` (a property of a transformation instance) |
| control | `Jump`, `JumpIfTrue`, `JumpIfNotTrue`, `JumpIfFalse`, `JumpIfUndef`, `JumpIfInvalid`, `JumpIfNull`, `JumpIfNotBool`, `JumpIfFastMode` (code only compat mode runs: `sortedBy`'s second pass), `Return` |
| calls | `CallStd`, `CallStdX`, `Call`, `CallVirtual`, `CallMapping`, `CallMeta`, `New` |
| navigation | `GetProp`, `GetPropW`, `GetTuple`, `GetInterm` |
| assignment | `Assign` (QVTo's `assign` on a variable), `Coerce` (List/Sequence coercion), `SetProp` (`callSetter`), `SetInterm` |
| collections | `NewColl`, `NewCollLike`, `LitAdd`, `LitRange`, `LitPut`, `NewTuple`, `Push`, `PushFlat`, `PushNew` |
| iteration | `IterInit`, `IterNext`, `ClosureInit`, `ClosureNext`, `ClosurePush`, `SortBy`, `SetNext` |
| types, objects | `KindTest`, `ObjectInit`, `Resolve`, `LateResolve` |
| imperative | `Log`, `AssertFail`, `Raise`, `Unsupported` |

## Limits

Registers, sites, loops and handlers per chunk are 16-bit; operation ids in `Call` are 16-bit
(compile error beyond); feature ids beyond `u16` use `GetPropW`.

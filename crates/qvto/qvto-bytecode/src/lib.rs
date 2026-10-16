//! The bytecode: a register-based instruction set ([`Insn`], at most 8 bytes), the compiler from
//! the typed IR ([`compile`]) and a disassembler ([`disasm`]). See `DESIGN.md`, "Bytecode and
//! VM", and the crate README.

pub mod compile;
pub mod disasm;
pub mod insn;
pub mod program;

pub use compile::{CompileError, compile};
pub use insn::{Insn, Reg};
pub use program::{
    BodyCode, CatchInfo, Chunk, ChunkId, Const, Handler, LoopInfo, OpCode, Program, Site,
};

//! The compiled program: chunks of instructions with their side tables, per-operation code,
//! the constant pool, and the typed IR's metadata (modules, operations, types, metamodels) that
//! the VM needs at run time.

use crate::insn::{Insn, Reg};
use qvto_model::{ClassId, EnumId, FeatureId};
use qvto_sema::ir::{
    self, AssertSeverity, GlobalId, IntermPropId, MetaOpId, ModelParamRef, ModuleId, OpId,
    ResolveKind, SourceId,
};
use qvto_types::{StdOp, TypeId};

/// Index of a chunk in [`Program::chunks`].
#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct ChunkId(pub u32);

/// A constant of the pool. Values are built from these at the start of a run (a `Program` is
/// `Send + Sync`; runtime values are not).
#[derive(Clone, Debug, PartialEq)]
pub enum Const {
    Int(i64),
    Real(f64),
    Str(Box<str>),
    Enum(EnumId, u32),
    Type(TypeId),
}

/// Operands that do not fit into an instruction.
#[derive(Clone, Debug)]
pub enum Site {
    Std {
        op: StdOp,
        /// The receiver's static type is UnlimitedNatural (`*` is -1).
        src_unlimited: bool,
        /// The argument's static type is UnlimitedNatural.
        arg_unlimited: bool,
        /// The receiver's static type (`sum` of an empty collection: `0`, `0.0` or null by
        /// element type).
        src_ty: TypeId,
        /// The call's static type (`product`: the tuple type of its elements).
        res_ty: TypeId,
    },
    Mapping {
        op: OpId,
        strict: bool,
        virtual_: bool,
    },
    Meta(MetaOpId),
    New {
        ty: TypeId,
        extent: Option<ModelParamRef>,
        constructor: Option<OpId>,
        nargs: u16,
    },
    NewTransformation {
        module: ModuleId,
        nargs: u16,
    },
    Feature(FeatureId),
    /// A tuple part name.
    Name(Box<str>),
    Interm(IntermPropId),
    /// Assignment to a variable of static type `ty`.
    Assign {
        ty: TypeId,
        add: bool,
        /// The value's element type is the variable's (no number conversion of elements).
        same_elem: bool,
        /// A module property: `callSetter`'s rules for undefined values (`+=` of an
        /// undefined value changes nothing, `:=` resets).
        property: bool,
    },
    SetProp {
        feature: FeatureId,
        add: bool,
    },
    SetInterm {
        prop: IntermPropId,
        add: bool,
    },
    /// A tuple literal of type `ty`; `order[i]` is the type's part index of the `i`-th written
    /// part.
    Tuple {
        ty: TypeId,
        order: Box<[u16]>,
    },
    Type(TypeId),
    Object {
        class: ClassId,
        extent: Option<ModelParamRef>,
        /// Where to continue when the variable holds an object of another type (the object
        /// expression yields invalid).
        fail: u32,
    },
    Resolve {
        kind: ResolveKind,
        /// `false`: resolve over all trace records.
        has_source: bool,
        /// The source's static type is a collection (resolve per element).
        source_is_coll: bool,
        mapping: Option<OpId>,
        target_ty: Option<TypeId>,
    },
    LateResolve {
        kind: ResolveKind,
        has_source: bool,
        source_is_coll: bool,
        mapping: Option<OpId>,
        target_ty: Option<TypeId>,
        /// The condition: its entry point in the same chunk (run on a fresh frame of the
        /// chunk's layout) and the register of the condition's variable.
        cond: Option<(u32, Reg)>,
        /// The assigned property and `+=`.
        feature: Option<FeatureId>,
        interm: Option<IntermPropId>,
        add: bool,
        /// One `->asSet()`-style conversion around the resolve.
        conversion: Option<StdOp>,
    },
    Log {
        has_level: bool,
        has_element: bool,
    },
    Assert {
        severity: AssertSeverity,
        /// `file:line` as Eclipse prints it.
        location: Box<str>,
        has_log: bool,
        has_level: bool,
        has_element: bool,
        /// The log's condition was false: nothing is logged.
        silent: bool,
    },
    Raise {
        class: Option<ClassId>,
        has_arg: bool,
    },
    Sort {
        /// The result is an OrderedSet (Set and OrderedSet sources), else a Sequence.
        ordered_set: bool,
        /// The body's static type is UnlimitedNatural (`*` sorts last).
        unlimited: bool,
    },
    Unsupported(Box<str>),
}

/// The variable a loop binds, and its iteration slot.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct LoopInfo {
    pub slot: u16,
    pub var: Reg,
    /// `closure`: the result collection.
    pub acc: Reg,
}

/// One `except` clause.
#[derive(Clone, Debug)]
pub struct CatchInfo {
    /// Empty: every QVT exception.
    pub types: Vec<ClassId>,
    pub var: Option<Reg>,
    pub target: u32,
}

/// A `try` block: QVT exceptions raised by instructions in `start..end` (and by calls made
/// from there) are matched against the clauses in order.
#[derive(Clone, Debug)]
pub struct Handler {
    pub start: u32,
    pub end: u32,
    pub catches: Vec<CatchInfo>,
}

/// A sequence of instructions run on one frame.
#[derive(Clone, Debug)]
pub struct Chunk {
    /// For disassembly and diagnostics (`Module::op/section`).
    pub name: String,
    pub source: SourceId,
    pub module: ModuleId,
    pub code: Vec<Insn>,
    /// Per instruction: the byte offset of Eclipse's current instruction pointer when the
    /// instruction runs (the start of the last AST node entered), `u32::MAX` if unknown.
    pub ips: Vec<u32>,
    pub sites: Vec<Site>,
    pub loops: Vec<LoopInfo>,
    /// Innermost first.
    pub handlers: Vec<Handler>,
    /// Registers of the frame (for chunks with a frame of their own).
    pub nregs: u32,
    /// Iteration slots used.
    pub n_iters: u32,
}

/// The chunks of one operation; all of them run on the operation's frame.
#[derive(Clone, Debug, Default)]
pub struct OpCode {
    pub nregs: u32,
    pub n_iters: u32,
    /// Helpers, queries, constructors, entries.
    pub body: Option<ChunkId>,
    /// Mappings: `when` (returns whether every clause is true), `where` (the same), the
    /// sections.
    pub when: Option<ChunkId>,
    pub where_: Option<ChunkId>,
    pub init: Option<ChunkId>,
    pub population: Option<ChunkId>,
    pub end: Option<ChunkId>,
}

/// A code body outside operations (initialisers, model type `where` conditions): a chunk with
/// its own frame and the register of `self`.
#[derive(Clone, Debug)]
pub struct BodyCode {
    pub chunk: ChunkId,
    pub self_reg: Option<Reg>,
}

/// The compiled program.
#[derive(Clone, Debug)]
pub struct Program {
    /// The analysed program: modules, operations (signatures, mapping relations, virtual
    /// tables), types, metamodels, sources. Bodies are compiled into [`Program::chunks`].
    pub ir: ir::Program,
    pub consts: Vec<Const>,
    pub chunks: Vec<Chunk>,
    /// Per operation of `ir.operations`.
    pub ops: Vec<OpCode>,
    /// Initialisers of `ir.globals`.
    pub globals: Vec<Option<BodyCode>>,
    /// Feature initialisers of `ir.interm_classes`.
    pub interm_inits: Vec<Vec<(FeatureId, BodyCode)>>,
    /// `where` conditions of `ir.model_types`.
    pub model_type_where: Vec<Option<BodyCode>>,
    /// OCL bodies of `ir.meta_ops` with the registers of their parameters.
    pub meta_bodies: Vec<Option<(BodyCode, Vec<Reg>)>>,
    /// OCL derivations of features (`ir.derivations`).
    pub derivations: std::collections::HashMap<FeatureId, BodyCode>,
    /// Per operation of `ir.operations`: the operation that overrides it in the end
    /// (`extends`), itself if none.
    pub overriding: Vec<OpId>,
}

impl Program {
    pub fn chunk(&self, c: ChunkId) -> &Chunk {
        &self.chunks[c.0 as usize]
    }
    pub fn op(&self, op: OpId) -> &ir::Operation {
        &self.ir.operations[op.index()]
    }
    pub fn code(&self, op: OpId) -> &OpCode {
        &self.ops[op.index()]
    }
    pub fn global(&self, g: GlobalId) -> &ir::Global {
        &self.ir.globals[g.index()]
    }
    /// The 1-based line of a byte offset in a source, if known.
    pub fn line(&self, source: SourceId, offset: u32) -> Option<u32> {
        if offset == u32::MAX {
            return None;
        }
        self.ir
            .sources
            .get(source.index())
            .map(|s| s.lines.line(offset))
    }
}

//! The instruction set: a register machine (DESIGN §5.2). Every instruction is at most 8 bytes:
//! an opcode and up to three 16-bit operands, or a register and a 32-bit operand. Operands
//! that do not fit (types, features beyond `u16`, names, flags) live in the chunk's site table
//! ([`crate::Site`]) and the instruction holds the site's index.
//!
//! Registers are frame slots: the operation's variables first (`LocalId` = register), then
//! temporaries. "Undefined" means `null` or `invalid`; "true" means `Boolean.TRUE` exactly.

/// A register of the current frame.
pub type Reg = u16;

/// One instruction. See the module docs for the conventions; the VM (`qvto-vm`) implements the
/// semantics, the compiler (`crate::compile`) documents which IR construct produces which
/// sequence.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum Insn {
    Nop,

    // ---- moves and constants --------------------------------------------------------------
    Move {
        dst: Reg,
        src: Reg,
    },
    /// Moves a value out of a register, leaving null (the variable is reassigned right after,
    /// so the value's storage can be reused in place).
    Take {
        dst: Reg,
        src: Reg,
    },
    LoadNull {
        dst: Reg,
    },
    LoadInvalid {
        dst: Reg,
    },
    LoadBool {
        dst: Reg,
        v: bool,
    },
    LoadInt {
        dst: Reg,
        v: i32,
    },
    /// A constant of the program's pool.
    LoadConst {
        dst: Reg,
        k: u32,
    },
    /// The initial value of a type (`EvaluationUtil.createInitialValue`): `""`, `0`, `false`,
    /// `0.0`, an empty collection of the declared kind, a defaulted tuple, else `null`.
    LoadDefault {
        dst: Reg,
        ty: u32,
    },
    /// `this` of a module.
    LoadThis {
        dst: Reg,
        module: u16,
    },
    /// A model parameter (`ModelInstance`) of a module.
    LoadModel {
        dst: Reg,
        module: u16,
        index: u16,
    },
    LoadGlobal {
        dst: Reg,
        g: u32,
    },
    StoreGlobal {
        src: Reg,
        g: u32,
    },
    /// A property of a transformation instance (`t.p`).
    LoadInstGlobal {
        dst: Reg,
        inst: Reg,
        g: u16,
    },
    StoreInstGlobal {
        src: Reg,
        inst: Reg,
        g: u16,
    },
    /// Fast mode: jump (skips what only compat mode evaluates).
    JumpIfFastMode {
        target: u32,
    },
    /// `coll[idx] := val; idx += 1` on a Sequence (`sortedBy`'s second pass).
    SetNext {
        coll: Reg,
        idx: Reg,
        val: Reg,
    },

    // ---- control --------------------------------------------------------------------------
    Jump {
        target: u32,
    },
    /// Jumps if the register holds `true`.
    JumpIfTrue {
        r: Reg,
        target: u32,
    },
    /// Jumps unless the register holds `true` (`false`, undefined, non-Booleans).
    JumpIfNotTrue {
        r: Reg,
        target: u32,
    },
    /// Jumps if the register holds `false`.
    JumpIfFalse {
        r: Reg,
        target: u32,
    },
    /// Jumps if the register holds `null` or `invalid`.
    JumpIfUndef {
        r: Reg,
        target: u32,
    },
    JumpIfInvalid {
        r: Reg,
        target: u32,
    },
    JumpIfNull {
        r: Reg,
        target: u32,
    },
    /// Jumps unless the register holds a Boolean.
    JumpIfNotBool {
        r: Reg,
        target: u32,
    },
    /// Ends the chunk with a value.
    Return {
        src: Reg,
    },

    // ---- arithmetic and comparison (OCL operators; fast paths for Integer and Real operands,
    // classic OCL's general rules otherwise) -----------------------------------------------
    Add {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Sub {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Mul {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Lt {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Le {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Gt {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Ge {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    /// OCL `=`.
    Eq {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    /// OCL `<>`.
    Ne {
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    /// Boolean `not`.
    Not {
        dst: Reg,
        src: Reg,
    },

    // ---- calls ----------------------------------------------------------------------------
    /// A standard library operation: receiver at `base` (`null` for static operations),
    /// arguments at `base + 1 ..`. `op` is a `qvto_types::StdOp`.
    CallStd {
        dst: Reg,
        base: Reg,
        op: u16,
    },
    /// A standard library operation with static information (UnlimitedNatural operands).
    CallStdX {
        dst: Reg,
        base: Reg,
        site: u16,
    },
    /// A helper, query or constructor: `self` (or `null`) at `base`, arguments after it.
    Call {
        dst: Reg,
        base: Reg,
        op: u16,
    },
    /// The same with virtual dispatch on the runtime type of `self`.
    CallVirtual {
        dst: Reg,
        base: Reg,
        op: u16,
    },
    /// A mapping call (`Site::Mapping`); after the call the registers of `out` arguments hold
    /// the callee's values.
    CallMapping {
        dst: Reg,
        base: Reg,
        site: u16,
    },
    /// A metamodel operation (`Site::Meta`).
    CallMeta {
        dst: Reg,
        base: Reg,
        site: u16,
    },
    /// `new T(args)` (`Site::New`), arguments at `base ..`.
    New {
        dst: Reg,
        base: Reg,
        site: u16,
    },

    /// `new T(m1, ...)` of a transformation (`Site::NewTransformation`), models at `base ..`.
    NewTransformation {
        dst: Reg,
        base: Reg,
        site: u16,
    },

    // ---- navigation -----------------------------------------------------------------------
    /// `obj.feature`; many-valued features give a copy of the kind of the feature.
    GetProp {
        dst: Reg,
        obj: Reg,
        feature: u16,
    },
    /// The same for a feature id beyond `u16` (`Site::Feature`).
    GetPropW {
        dst: Reg,
        obj: Reg,
        site: u16,
    },
    /// `tuple.part` (`Site::Name`).
    GetTuple {
        dst: Reg,
        src: Reg,
        site: u16,
    },
    /// A contextual intermediate property (`Site::Interm`).
    GetInterm {
        dst: Reg,
        obj: Reg,
        site: u16,
    },

    // ---- assignment -----------------------------------------------------------------------
    /// `dst := assign(type, dst, src, reset)` of a variable (`Site::Assign`).
    Assign {
        dst: Reg,
        src: Reg,
        site: u16,
    },
    /// Implicit `List`/`Sequence` coercion of a value bound to a variable of a type
    /// (`Site::Type`), in place.
    Coerce {
        r: Reg,
        site: u16,
    },
    /// `obj.feature := src` / `+=` (`Site::SetProp`): QVTo's `callSetter`.
    SetProp {
        obj: Reg,
        src: Reg,
        site: u16,
    },
    /// The same for a contextual intermediate property (`Site::SetInterm`).
    SetInterm {
        obj: Reg,
        src: Reg,
        site: u16,
    },

    // ---- collections ----------------------------------------------------------------------
    /// A new empty collection: 0 Set, 1 OrderedSet, 2 Bag, 3 Sequence, 4 List, 5 Dict.
    NewColl {
        dst: Reg,
        kind: u8,
    },
    /// A new empty collection of the runtime kind of `src`.
    NewCollLike {
        dst: Reg,
        src: Reg,
    },
    /// Collection or List literal item: an invalid item makes the literal invalid.
    LitAdd {
        coll: Reg,
        val: Reg,
    },
    /// Collection or List literal range `lo .. lo+1`.
    LitRange {
        coll: Reg,
        lo: Reg,
    },
    /// Dict literal part: key at `key`, value at `key + 1`; parts with an invalid key or value
    /// are skipped.
    LitPut {
        dict: Reg,
        key: Reg,
    },
    /// A tuple literal (`Site::Tuple`): part values at `base ..` in written order.
    NewTuple {
        dst: Reg,
        base: Reg,
        site: u16,
    },
    /// Java `add` of an element to a collection being built.
    Push {
        coll: Reg,
        val: Reg,
    },
    /// OCL `collect`: adds the element, or the flattened elements of a collection.
    PushFlat {
        coll: Reg,
        val: Reg,
    },
    /// `isUnique`: adds `val` to the `HashSet` in `coll`; `dst` is whether it was new.
    PushNew {
        dst: Reg,
        coll: Reg,
        val: Reg,
    },

    // ---- iteration ------------------------------------------------------------------------
    /// Starts iterating `src` (collections, `List`, `Dict` values) in slot `slot`.
    IterInit {
        slot: u16,
        src: Reg,
    },
    /// Next element of the loop's slot into the loop's variable (`Chunk::loops`), or a jump to
    /// `target` when exhausted.
    IterNext {
        lp: u16,
        target: u32,
    },
    /// `closure`: starts a depth-first traversal of `src` in slot `slot`.
    ClosureInit {
        slot: u16,
        src: Reg,
    },
    /// `closure`: the next element to evaluate the body on (elements below the first level
    /// that the result in `acc` already holds are skipped), or a jump when done. `lp` as for
    /// `IterNext`; the result collection is the register in `LoopInfo::acc`.
    ClosureNext {
        lp: u16,
        target: u32,
    },
    /// `closure`: descends into the body value of the current element.
    ClosurePush {
        slot: u16,
        val: Reg,
    },
    /// `sortedBy`: sorts the elements at `elems` by the keys at `elems + 1` (`Site::Sort`).
    SortBy {
        dst: Reg,
        elems: Reg,
        site: u16,
    },

    // ---- types ----------------------------------------------------------------------------
    /// QVTo's `isKindOf` of `src` against the type of `Site::Type` (typed `xselect`).
    KindTest {
        dst: Reg,
        src: Reg,
        site: u16,
    },

    // ---- objects, traces ------------------------------------------------------------------
    /// `object x : T @ m {...}`: keeps the variable's object when it is non-null (it must
    /// conform, else a jump to `Site::Object::fail`), else creates one and binds it.
    ObjectInit {
        var: Reg,
        site: u16,
    },
    /// The candidates of a `resolve` family expression (`Site::Resolve`): source at `base`.
    Resolve {
        dst: Reg,
        base: Reg,
        site: u16,
    },
    /// Registers a late resolve whose assignment runs after the entry operation
    /// (`Site::LateResolve`): owner at `base`, resolve source at `base + 1`.
    LateResolve {
        base: Reg,
        site: u16,
    },

    // ---- imperative -----------------------------------------------------------------------
    /// `log`: level, message and element at `base ..` as `Site::Log` says.
    Log {
        base: Reg,
        site: u16,
    },
    /// A failed `assert` (`Site::Assert`); log arguments at `base ..`.
    AssertFail {
        base: Reg,
        site: u16,
    },
    /// `raise` (`Site::Raise`), argument in `src`.
    Raise {
        src: Reg,
        site: u16,
    },
    /// Not supported by this engine version: a fatal runtime error (`Site::Unsupported`).
    Unsupported {
        site: u16,
    },
}

const _: () = assert!(std::mem::size_of::<Insn>() <= 8);

/// Collection kinds of [`Insn::NewColl`].
pub mod coll_kind {
    pub const SET: u8 = 0;
    pub const ORDERED_SET: u8 = 1;
    pub const BAG: u8 = 2;
    pub const SEQUENCE: u8 = 3;
    pub const LIST: u8 = 4;
    pub const DICT: u8 = 5;
}

impl Insn {
    /// The jump target of a jump instruction.
    pub fn target(&self) -> Option<u32> {
        match *self {
            Insn::Jump { target }
            | Insn::JumpIfTrue { target, .. }
            | Insn::JumpIfNotTrue { target, .. }
            | Insn::JumpIfFalse { target, .. }
            | Insn::JumpIfUndef { target, .. }
            | Insn::JumpIfInvalid { target, .. }
            | Insn::JumpIfNull { target, .. }
            | Insn::JumpIfNotBool { target, .. }
            | Insn::JumpIfFastMode { target }
            | Insn::IterNext { target, .. }
            | Insn::ClosureNext { target, .. } => Some(target),
            _ => None,
        }
    }

    /// Sets the jump target of a jump instruction.
    pub fn set_target(&mut self, t: u32) {
        match self {
            Insn::Jump { target }
            | Insn::JumpIfTrue { target, .. }
            | Insn::JumpIfNotTrue { target, .. }
            | Insn::JumpIfFalse { target, .. }
            | Insn::JumpIfUndef { target, .. }
            | Insn::JumpIfInvalid { target, .. }
            | Insn::JumpIfNull { target, .. }
            | Insn::JumpIfNotBool { target, .. }
            | Insn::JumpIfFastMode { target }
            | Insn::IterNext { target, .. }
            | Insn::ClosureNext { target, .. } => *target = t,
            _ => panic!("not a jump: {self:?}"),
        }
    }
}

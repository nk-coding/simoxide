//! The typed IR: the output of semantic analysis and the input of the bytecode compiler.
//!
//! A [`Program`] is everything a run needs, with every name resolved: operations are
//! [`OpId`]s, standard library operations are [`StdOp`] ids, features are `FeatureId`s of the
//! program's `MetaRegistry`, variables are [`LocalId`] slots of their operation, module
//! properties are [`GlobalId`]s, types are [`TypeId`]s of the program's [`TypeTable`].
//!
//! The IR is a tree of [`Expr`] nodes per operation body, close to Eclipse's QVTo AST
//! (ImperativeOCL + QVTOperational on classic OCL) after analysis, with Eclipse's desugarings
//! already applied by the analyser:
//!
//! - `.` on a collection is an OCL `collect` loop ([`LoopKind::Collect`]) whose body navigates the
//!   iterator; `coll->prop`, `coll->op()`, `coll->map m()` are `xcollect` loops
//!   ([`LoopKind::XCollect`]); `coll[cond]`/`coll![cond]` are `xselect`/`selectOne` loops, and
//!   `coll[T]` has [`Loop::filter_type`]; iterators without declared variables get implicit ones;
//!   `->` on a non-collection source wraps the source in `oclAsSet()`
//!   ([`StdOp`] `OclAny_oclAsSet`), as classic OCL does.
//! - A mapping body without an explicit `population` section and with exactly one result (or an
//!   `inout` context and no result) has its statements wrapped in an [`ExprKind::Object`]
//!   for `result` (resp. `self`), as Eclipse's `visitMappingBodyCS` does.
//! - `while (x := e; c)` and `while (x : T := e; c)` are a [`ExprKind::Compute`] of the
//!   variable around the loop, as Eclipse makes them.
//! - Calls that Eclipse resolves to the standard library are [`Callee::Std`], also operators
//!   (`a + b` is `Std(Integer_add)` with source `a` and argument `b`), `and`/`or`/`implies`
//!   included: the VM implements their short-circuit rules.
//!
//! What the analyser does *not* do (the VM's job, as in Eclipse's evaluator): implicit
//! instantiation of mapping results and `out` parameters, trace records, `when`/`where`
//! checks, disjunct dispatch, the default extent of created objects, number conversions on
//! assignment, default values of uninitialised variables, late-resolve detection on assignment.
//!
//! Spans are byte ranges in the operation's source file ([`Program::sources`]); line numbers for
//! logs, asserts and stack traces come from [`Source::lines`].
//!
//! Stability: this module is the contract with `qvto-bytecode`/`qvto-vm`. Changes are recorded
//! in `crates/qvto-sema/CHANGES.md`.

use qvto_model::{ClassId, EnumId, FeatureId, MetaRegistry, PackageId};
use qvto_syntax::{LineIndex, Span};
use qvto_types::{CollKind, StdClasses, StdOp, TypeId, TypeTable};

macro_rules! id {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
        pub struct $name(pub u32);
        impl $name {
            #[inline]
            pub fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}

id!(
    /// A module (transformation or library) of the program.
    ModuleId
);
id!(
    /// An imperative operation (helper, query, mapping, constructor, entry) of any module.
    OpId
);
id!(
    /// A module property or configuration property.
    GlobalId
);
id!(
    /// A variable slot of one operation or code body.
    LocalId
);
id!(
    /// A contextual intermediate property (`intermediate property C::p : T`).
    IntermPropId
);
id!(
    /// A `modeltype` declaration.
    ModelTypeId
);
id!(
    /// A source file.
    SourceId
);
id!(
    /// An operation of a metamodel (`EOperation`), e.g. Ecore's `eContainer()`.
    MetaOpId
);

/// A model parameter: the module that declares it and its index in the module's signature.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct ModelParamRef {
    pub module: ModuleId,
    pub index: u32,
}

/// A whole analysed program: the main unit, everything it imports, and the parts of the
/// standard library it refers to.
#[derive(Clone, Debug)]
pub struct Program {
    pub types: TypeTable,
    /// The metamodels the program was compiled against, grown by QVTo's library classes
    /// ([`Program::std_classes`]) and by the intermediate classes and exceptions of the modules
    /// (one package per module, named after it). Ids of the input registry are unchanged.
    pub meta: MetaRegistry,
    pub std_classes: StdClasses,
    /// Source files; `SourceId(0)` is an empty placeholder (the library), units start at 1.
    pub sources: Vec<Source>,
    /// All modules; imported modules come before the modules that import them. `ModuleId(0)`
    /// is QVTo's library module `Stdlib` (no operations of its own in
    /// [`Program::operations`]; calls to it are [`Callee::Std`]).
    pub modules: Vec<Module>,
    /// The module of the main unit that is executed (the first transformation of the main
    /// compilation unit, or its first library).
    pub main_module: ModuleId,
    pub model_types: Vec<ModelTypeDecl>,
    pub operations: Vec<Operation>,
    pub globals: Vec<Global>,
    pub interm_props: Vec<IntermProp>,
    /// Intermediate classes and exceptions declared by the modules, with their feature
    /// initialisers.
    pub interm_classes: Vec<IntermClass>,
    /// Metamodel operations the program calls.
    pub meta_ops: Vec<MetaOp>,
    /// The OCL derivations of features of dynamic classes (the `derive` detail of their
    /// `http://www.eclipse.org/2007/OCL` annotation): reading the feature evaluates them with
    /// `self` bound to the object (`OCLAnnotationSupport.getDerivedProperty`).
    pub derivations: Vec<(FeatureId, CodeBody)>,
}

/// A source file.
#[derive(Clone, Debug)]
pub struct Source {
    /// The file name as Eclipse prints it in logs and stack traces (last URI segment).
    pub name: String,
    /// The unit's qualified name (`a.b.C` for `a/b/C.qvto` below a source root).
    pub qualified_name: String,
    pub path: Option<std::path::PathBuf>,
    pub lines: LineIndex,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum ModuleKind {
    Transformation,
    Library,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum Direction {
    In,
    Out,
    InOut,
}

#[derive(Clone, Debug)]
pub struct Module {
    pub name: String,
    pub kind: ModuleKind,
    /// Implemented by the host: `blackbox transformation ...;` (no body), or a module of a Java
    /// (native) blackbox unit.
    pub blackbox: bool,
    pub source: SourceId,
    pub span: Span,
    /// The module as a type (`Type::Module`).
    pub ty: TypeId,
    pub model_params: Vec<ModelParam>,
    pub model_types: Vec<ModelTypeId>,
    /// `access`/`extends` usages and plain `import`s of library modules, in declaration order.
    pub usages: Vec<ModuleUsage>,
    /// Configuration properties then the other module properties, in declaration order (the
    /// initialisation order of Eclipse's module instantiation).
    pub globals: Vec<GlobalId>,
    /// All operations owned by this module, in declaration order.
    pub operations: Vec<OpId>,
    /// The entry operation: `main()` or a legacy `mapping main(...)`.
    pub entry: Option<OpId>,
    /// Intermediate classes and exceptions (indexes into [`Program::interm_classes`]).
    pub classes: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct ModelParam {
    pub name: String,
    pub direction: Direction,
    pub model_type: ModelTypeId,
    /// Type of the parameter as a value (`Type::ModelType`).
    pub ty: TypeId,
    pub span: Span,
    /// Generated for the parameters and results of a legacy entry `mapping main(in x : T) : U`
    /// of a transformation without model parameters (Eclipse's `DeprecatedSignaturelessTransf`,
    /// marked by the `QVT_AUTOGEN_MODELPARAM_EXPRESSION_URI` annotation; named `$x`). Their
    /// extents are not part of the evaluation result.
    pub generated: bool,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum UsageKind {
    /// `access` (and plain `import` of a library: Eclipse imports by access)
    Access,
    Extends,
}

#[derive(Clone, Debug)]
pub struct ModuleUsage {
    pub kind: UsageKind,
    pub module: ModuleId,
    pub span: Span,
}

/// `modeltype M ['strict'] uses p1('uri'), ... [where { ... }]`
#[derive(Clone, Debug)]
pub struct ModelTypeDecl {
    pub name: String,
    pub module: ModuleId,
    /// The metamodels; the first one decides default extents of created objects.
    pub packages: Vec<PackageId>,
    pub strict: bool,
    /// `where` conditions, evaluated with `self` bound to the model parameter.
    pub where_: Option<CodeBody>,
    pub ty: TypeId,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum GlobalKind {
    /// `configuration property x : T;`
    Config,
    /// `property x : T = e;`
    Property,
}

#[derive(Clone, Debug)]
pub struct Global {
    pub module: ModuleId,
    pub name: String,
    pub kind: GlobalKind,
    pub ty: TypeId,
    /// Initialiser; `None` means the type's default value (configuration properties: the
    /// configured value, else null).
    pub init: Option<CodeBody>,
    pub span: Span,
}

/// `intermediate property C::p : T [= e];`
#[derive(Clone, Debug)]
pub struct IntermProp {
    pub module: ModuleId,
    pub context: TypeId,
    pub name: String,
    pub ty: TypeId,
    /// Analysed but ignored at run time (Eclipse warns and uses the type default).
    pub init: Option<CodeBody>,
    pub span: Span,
}

/// An intermediate class or exception (a class of [`Program::meta`]).
#[derive(Clone, Debug)]
pub struct IntermClass {
    pub module: ModuleId,
    pub class: ClassId,
    pub exception: bool,
    /// Initialisers of the class's own features, in declaration order; features without one
    /// get the type default when an instance is created.
    pub inits: Vec<(FeatureId, CodeBody)>,
    /// The declared type of each own feature (`[*]` features: the collection type). The
    /// registry keeps a classifier per feature; for types it cannot represent (collections,
    /// `List`, `Dict`, tuples, `OclAny`, ...) that is a placeholder data type.
    pub feature_types: Vec<(FeatureId, TypeId)>,
    /// The own features declared `static`: one value shared by all instances
    /// (`IntermediateClassFactory.markFeatureAsStatic`), initialised by the first instance.
    pub static_features: Vec<FeatureId>,
    pub span: Span,
}

/// A metamodel operation called by the program.
#[derive(Clone, Debug)]
pub struct MetaOp {
    pub class: ClassId,
    pub name: String,
    pub params: Vec<TypeId>,
    pub result: TypeId,
    /// The OCL body of an operation of a dynamic class (the `body` detail of its
    /// `http://www.eclipse.org/2007/OCL` annotation, `OCLAnnotationSupport`), analysed in the
    /// class's context; a body that does not parse or analyse is `invalid` (Eclipse logs the
    /// parser exception and evaluates an `InvalidLiteralExp`).
    pub body: Option<MetaBody>,
}

/// The analysed OCL body of a metamodel operation: `self` and the parameters as variables.
#[derive(Clone, Debug)]
pub struct MetaBody {
    pub code: CodeBody,
    /// The parameters' variables, in parameter order.
    pub params: Vec<LocalId>,
}

/// Code outside operations (initialisers, `where` conditions): an expression with its own
/// variable slots.
#[derive(Clone, Debug)]
pub struct CodeBody {
    pub source: SourceId,
    pub locals: Vec<Local>,
    /// `self` of `where` conditions and of feature initialisers (the new object).
    pub self_local: Option<LocalId>,
    pub expr: Expr,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum OpKind {
    Helper,
    Query,
    Mapping,
    Constructor,
    /// `main(...)`
    Entry,
}

#[derive(Clone, Debug)]
pub struct Operation {
    pub module: ModuleId,
    pub kind: OpKind,
    pub name: String,
    /// The context type of a contextual operation (`Ctx::name`).
    pub context: Option<TypeId>,
    /// Direction of the context parameter of a mapping (`mapping inout C::m()`): `In` by default.
    pub context_direction: Direction,
    pub params: Vec<Param>,
    pub results: Vec<Param>,
    /// The operation's type: the single result's type, a tuple of the results, or `OclVoid`.
    pub result_ty: TypeId,
    pub is_abstract: bool,
    pub is_blackbox: bool,
    pub is_static: bool,
    /// All variable slots: parameters, `self`, `result`, local variables, iterators.
    pub locals: Vec<Local>,
    pub self_local: Option<LocalId>,
    /// `result` (the single result parameter, or the tuple of several results).
    pub result_local: Option<LocalId>,
    /// Statements of the body (helpers, queries, constructors, `main`); `None` for declarations
    /// without body (abstract, blackbox). Mappings use [`Operation::mapping`].
    pub body: Option<Vec<Expr>>,
    pub mapping: Option<MappingInfo>,
    /// Eclipse's virtual table of this operation: operations with the same name and signature
    /// on other context types, in insertion order. The VM picks by the runtime type of `self`
    /// (`VirtualTable.lookupActualOperation`).
    pub vtable: Vec<OpId>,
    /// The operation of an extended module that this one overrides (`extends`).
    pub overrides: Option<OpId>,
    pub source: SourceId,
    /// The declaration (Eclipse's `MappingDeclarationCS`).
    pub decl_span: Span,
    pub span: Span,
    /// The Java method implementing an operation of a Java blackbox library
    /// (`pkg.Class#name(java.lang.String, ...)`), also of a `blackbox` operation declared in
    /// QVTo whose implementation the analyser found (exactly one match).
    pub blackbox_impl: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub direction: Direction,
    pub ty: TypeId,
    /// `T @ m`: the extent of created objects.
    pub extent: Option<ModelParamRef>,
    pub local: LocalId,
    pub span: Span,
}

#[derive(Clone, Debug, Default)]
pub struct MappingInfo {
    pub when: Vec<Expr>,
    pub where_: Vec<Expr>,
    pub init: Vec<Expr>,
    /// Population statements, already wrapped in the implicit `object` expression where
    /// Eclipse does that.
    pub population: Vec<Expr>,
    pub end: Vec<Expr>,
    pub inherits: Vec<OpId>,
    pub merges: Vec<OpId>,
    pub disjuncts: Vec<OpId>,
    /// The mapping body (`{ ... }`, Eclipse's `MappingBody`): the current instruction of the
    /// mapping's frame while its inherited mappings run.
    pub body_span: Option<Span>,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum LocalKind {
    Param,
    SelfVar,
    Result,
    /// `var x`, `compute (x ...)`, `while (x : T := e; ...)`
    Var,
    Let,
    Iterator,
    Accumulator,
    /// `collectselect`'s target variable.
    Target,
    /// `except (e : T)`
    Catch,
    /// `object x : T {}` with a new name, and implicit object targets.
    Object,
    /// `resolve(x : T | cond)`
    ResolveVar,
    /// Analyser-made variables (implicit iterators, temporaries).
    Temp,
}

#[derive(Clone, Debug)]
pub struct Local {
    pub name: String,
    pub ty: TypeId,
    pub kind: LocalKind,
    pub span: Span,
}

/// An expression with its static type.
#[derive(Clone, Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub ty: TypeId,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub enum ExprKind {
    // ---- literals -----------------------------------------------------------------------
    Null,
    Invalid,
    Bool(bool),
    /// Integer literal (Java `long`; `Integer` when it fits in 32 bits).
    Int(i64),
    Real(f64),
    Str(Box<str>),
    /// `*` (the Integer -1 with static type UnlimitedNatural).
    Unlimited,
    EnumLit(EnumId, u32),
    /// A type used as a value (`TypeExp`): arguments of `oclIsKindOf`, `allInstances`, ...
    Type(TypeId),
    /// `Set{...}` etc.
    Coll(CollKind, Vec<CollPart>),
    /// `List{...}`
    List(Vec<CollPart>),
    /// `Dict{k = v, ...}`
    Dict(Vec<(Expr, Expr)>),
    /// `Tuple{a = e, ...}`, parts in written order.
    Tuple(Vec<(Box<str>, Expr)>),

    // ---- variables ----------------------------------------------------------------------
    Local(LocalId),
    /// `this`: the instance of the given module.
    This(ModuleId),
    /// A module or configuration property (of the running instance of its module).
    Global(GlobalId),
    /// `t.p`: a property of the transformation instance `t` (a value of a module type other
    /// than `this`; each `new T(...)` instance has its own values).
    ModuleProp(Box<Expr>, GlobalId),
    /// A model parameter as a value (type `Model`).
    ModelParam(ModelParamRef),

    // ---- navigation ---------------------------------------------------------------------
    /// `src.feature` on a single object (many-valued features have a collection type).
    Prop(Box<Expr>, FeatureId),
    /// `tuple.part`
    TupleProp(Box<Expr>, Box<str>),
    /// `src.p` of a contextual intermediate property.
    IntermProp(Box<Expr>, IntermPropId),

    // ---- calls --------------------------------------------------------------------------
    Call(Box<Call>),
    /// OCL and imperative iterators.
    Loop(Box<Loop>),
    /// `let x = e in body`
    Let(LocalId, Box<Expr>, Box<Expr>),
    /// `if c then a else b endif` and the statement form; no `else` gives null.
    If(Box<Expr>, Box<Expr>, Option<Box<Expr>>),

    // ---- imperative ---------------------------------------------------------------------
    /// `do { ... }` and statement blocks (value null).
    Block(Vec<Expr>),
    /// `var x : T [:= e]`; `None` gives the type's default value. `with_result` for `::=`.
    VarInit {
        local: LocalId,
        init: Option<Box<Expr>>,
        with_result: bool,
    },
    /// `lhs := rhs` (`add: false`) and `lhs += rhs`.
    Assign(Box<Assign>),
    While(Box<Expr>, Box<Expr>),
    /// `compute (x : T [:= e]) body`: the value is `x` after the body.
    Compute(LocalId, Option<Box<Expr>>, Box<Expr>),
    /// `switch { case (c) e ... else e }`
    Switch(Vec<(Expr, Expr)>, Option<Box<Expr>>),
    Return(Option<Box<Expr>>),
    Break,
    Continue,
    Log(Box<Log>),
    Assert(Box<Assert>),
    /// `raise T(arg)`; `raise 'msg'` is `raise Exception('msg')` (as in Eclipse). The class is
    /// `None` only when the analyser reported an error.
    Raise(Option<ClassId>, Option<Box<Expr>>),
    Try(Box<Try>),
    /// `object [x :] T [@ m] { ... }`
    Object(Box<ObjectExp>),
    /// `new T(args)` of a class or data type.
    New(Box<New>),
    /// `new T(m1, m2)` of a transformation.
    NewTransformation(ModuleId, Vec<Expr>),
    Resolve(Box<Resolve>),
}

#[derive(Clone, Debug)]
pub enum CollPart {
    Item(Expr),
    Range(Expr, Expr),
}

#[derive(Clone, Debug)]
pub enum Callee {
    /// A standard library operation (OCL or QVTo), including operators.
    Std(StdOp),
    /// A helper, query or constructor. `virtual_` calls dispatch on the runtime type of the
    /// source through [`Operation::vtable`].
    Op { op: OpId, virtual_: bool },
    /// `map`/`xmap`.
    Mapping {
        op: OpId,
        strict: bool,
        virtual_: bool,
    },
    /// An operation of a metamodel.
    Meta(MetaOpId),
}

#[derive(Clone, Debug)]
pub struct Call {
    pub callee: Callee,
    /// The receiver: `self` of contextual operations, the first operand of operators; `None`
    /// for non-contextual operations and static library calls.
    pub source: Option<Expr>,
    pub args: Vec<Expr>,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum LoopKind {
    // classic OCL
    Select,
    Reject,
    Collect,
    CollectNested,
    ForAll,
    Exists,
    One,
    Any,
    IsUnique,
    SortedBy,
    Closure,
    Iterate,
    // ImperativeOCL
    XSelect,
    XCollect,
    SelectOne,
    CollectOne,
    CollectSelect,
    CollectSelectOne,
    ForEach,
    ForOne,
}

#[derive(Clone, Debug)]
pub struct Loop {
    pub kind: LoopKind,
    pub source: Expr,
    /// Iterator variables (at least one; implicit ones are [`LocalKind::Temp`]... named like
    /// Eclipse's generated names).
    pub iterators: Vec<LocalId>,
    /// `iterate`'s accumulator and its initialiser.
    pub acc: Option<(LocalId, Expr)>,
    /// `collectselect`'s target variable (bound to the body value).
    pub target: Option<LocalId>,
    /// The body (OCL iterators, `xcollect`, `collectselect`, `forEach`).
    pub body: Option<Expr>,
    /// The condition (`xselect`, `selectOne`, `collectselect`, `forEach (... | c)`).
    pub cond: Option<Expr>,
    /// `coll[T]`: select by kind of `T` instead of a condition (QVTo's typed shorthand).
    pub filter_type: Option<TypeId>,
}

#[derive(Clone, Debug)]
pub enum LValue {
    Local(LocalId),
    Global(GlobalId),
    /// `t.p := x` on a transformation instance (see [`ExprKind::ModuleProp`]).
    ModuleProp(Expr, GlobalId),
    Prop(Expr, FeatureId),
    IntermProp(Expr, IntermPropId),
    /// `t.part += x` on a `List`/`Dict` tuple part (the only tuple mutation Eclipse allows).
    TupleProp(Expr, Box<str>),
}

#[derive(Clone, Debug)]
pub struct Assign {
    pub target: LValue,
    /// Static type of the target (decides collection kind and number conversion).
    pub target_ty: TypeId,
    pub value: Expr,
    /// `+=`
    pub add: bool,
    /// The left-hand side's span.
    pub target_span: Span,
}

#[derive(Clone, Debug)]
pub struct Log {
    pub message: Option<Expr>,
    pub element: Option<Expr>,
    pub level: Option<Expr>,
    pub when: Option<Expr>,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum AssertSeverity {
    Warning,
    Error,
    Fatal,
}

#[derive(Clone, Debug)]
pub struct Assert {
    pub severity: AssertSeverity,
    pub cond: Expr,
    pub log: Option<Log>,
}

#[derive(Clone, Debug)]
pub struct Try {
    pub body: Expr,
    pub catches: Vec<Catch>,
}

#[derive(Clone, Debug)]
pub struct Catch {
    /// Empty: catches every QVT exception.
    pub types: Vec<ClassId>,
    pub var: Option<LocalId>,
    pub body: Expr,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct ObjectExp {
    /// The variable the object is bound to: an existing variable (`result`, an `out`
    /// parameter, a `var`) that is reused when non-null, or a new one.
    pub var: LocalId,
    /// The instantiated class.
    pub class: ClassId,
    pub extent: Option<ModelParamRef>,
    pub body: Vec<Expr>,
}

#[derive(Clone, Debug)]
pub struct New {
    /// The instantiated type: a class, a data type (`new T('literal')`) or a collection type.
    pub ty: TypeId,
    pub extent: Option<ModelParamRef>,
    pub constructor: Option<OpId>,
    pub args: Vec<Expr>,
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum ResolveKind {
    Resolve,
    ResolveOne,
    InvResolve,
    InvResolveOne,
}

impl ResolveKind {
    pub fn is_one(self) -> bool {
        matches!(self, ResolveKind::ResolveOne | ResolveKind::InvResolveOne)
    }
    pub fn is_inverse(self) -> bool {
        matches!(self, ResolveKind::InvResolve | ResolveKind::InvResolveOne)
    }
}

#[derive(Clone, Debug)]
pub struct Resolve {
    pub kind: ResolveKind,
    pub late: bool,
    /// The source (`src.resolve(...)`); `None` resolves over all trace records.
    pub source: Option<Expr>,
    /// `resolveIn(M, ...)`: the mapping.
    pub mapping: Option<OpId>,
    /// `resolve(T)` / `resolve(x : T | cond)`: the filter type.
    pub target_ty: Option<TypeId>,
    pub cond_var: Option<LocalId>,
    pub cond: Option<Expr>,
}

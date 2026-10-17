//! The syntax tree. It mirrors Eclipse's CST (`org.eclipse.m2m.internal.qvt.oml.cst`) closely
//! enough for semantic analysis, keeps every construct's source span, and records the surface
//! form where two spellings mean the same thing only when the difference matters to Eclipse
//! (diagnostics, deprecation warnings); otherwise the printer normalises.
//!
//! Keyword-determined choices (primitive and collection type names, iterator kinds, resolve
//! operations, qualifiers, directions, feature keys) are stored as the keyword's [`TokenKind`],
//! so the set of keywords lives only in the generated tables.

use crate::generated::TokenKind;
use crate::span::Span;

/// A simple name.
#[derive(Clone, Debug, PartialEq)]
pub struct Name {
    /// The name, with quotes removed and escapes decoded for `_'…'` names.
    pub text: String,
    pub kind: NameKind,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameKind {
    /// `IDENTIFIER`
    Ident,
    /// `_'…'`, possibly followed by string literals that are appended.
    Quoted,
    /// A keyword used as a name: `this`, `result`, `self`, `main`, or a restricted keyword after
    /// `::` (`ecore::String`).
    Keyword(TokenKind),
}

/// `a::b::c`
#[derive(Clone, Debug, PartialEq)]
pub struct PathName {
    pub segments: Vec<Name>,
    pub span: Span,
}

/// A scoped identifier of an operation or property: `name`, `Ctx::name`, `a::Ctx::name`, or
/// `Set(T)::name` (context written as a type).
#[derive(Clone, Debug, PartialEq)]
pub struct ScopedName {
    /// A non-path context type (`typeCS2`: primitive, collection, tuple, List, Dict).
    pub context_type: Option<TypeExpr>,
    /// The path; the last segment is the element's own name.
    pub names: Vec<Name>,
    pub span: Span,
}

impl ScopedName {
    /// The element's own name (last segment).
    pub fn name(&self) -> Option<&Name> {
        self.names.last()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypeExpr {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TypeKind {
    /// `Boolean`, `Integer`, `Real`, `String`, `UnlimitedNatural`, `OclAny`, `OclInvalid`,
    /// `OclVoid` (the keyword).
    Primitive(TokenKind),
    /// A class, data type or enumeration by (qualified) name.
    Path(PathName),
    /// `Set(T)` etc.; the keyword gives the collection kind.
    Collection(TokenKind, Box<TypeExpr>),
    /// `Tuple(a : T, …)`
    Tuple(Vec<VarDecl>),
    /// `List(T)`
    List(Box<TypeExpr>),
    /// `Dict(K, V)`
    Dict(Box<TypeExpr>, Box<TypeExpr>),
    /// Recovery placeholder.
    Error,
}

/// `T` or `T @ extent`
#[derive(Clone, Debug, PartialEq)]
pub struct TypeSpec {
    pub ty: TypeExpr,
    pub extent: Option<Name>,
    pub span: Span,
}

/// A variable declaration: `x`, `x : T`, `x = e`, `x : T := e`, …
#[derive(Clone, Debug, PartialEq)]
pub struct VarDecl {
    pub name: Name,
    pub ty: Option<TypeExpr>,
    /// The initialisation operator (`=`, `:=` or `::=`) and expression.
    pub init: Option<(TokenKind, Box<Expr>)>,
    pub span: Span,
}

/// A string literal: one or more adjacent literals, concatenated.
#[derive(Clone, Debug, PartialEq)]
pub struct StringLit {
    /// The decoded, concatenated value.
    pub value: String,
    /// Spans of the literal tokens.
    pub parts: Vec<Span>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub stmts: Vec<Expr>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavOp {
    /// `.`
    Dot,
    /// `->`
    Arrow,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExprKind {
    /// Integer literal; `value` is 0 when the literal does not fit in a `long` (with an error).
    Int(i64),
    /// Real literal, as written (Java `Double.valueOf` syntax).
    Real(String),
    Str(StringLit),
    Bool(bool),
    Null,
    Invalid,
    /// `*`
    Unlimited,
    /// `self`
    SelfExp,
    /// A simple name (variable, property, `this`, `result`).
    Name(Name),
    /// `a::b::c` (enumeration literal, class, qualified property).
    Path(PathName),
    /// A type used as a value: `Integer`, `Set(T)`, `Tuple(a : T)`.
    TypeLit(TypeExpr),
    /// `Set{…}` etc.; the keyword gives the kind.
    CollectionLit(TokenKind, Vec<CollectionPart>),
    /// `List{…}`
    ListLit(Vec<CollectionPart>),
    /// `Dict{k = v, …}`
    DictLit(Vec<DictPart>),
    /// `Tuple{a = e, b : T = e}`
    TupleLit(Vec<VarDecl>),
    /// `(e)`
    Paren(Box<Expr>),
    /// `-e`, `not e`
    Unary(TokenKind, Box<Expr>),
    /// `a op b`; `op` is the operator token (`=`, `<>`, `!=`, `and`, …).
    Binary(TokenKind, Box<Expr>, Box<Expr>),
    /// `let x = e, … in body`
    Let(Vec<VarDecl>, Box<Expr>),
    If(Box<IfExp>),
    /// Operation call: `name(args)`, `p::name(args)`, `src.name(args)`, `src->name(args)`,
    /// `src.p::name(args)`. The name may be a conceptual operation name (`a.+(b)`).
    Call(Box<Call>),
    /// Property navigation `src.name`, `src.p::name`, or the `src->name` shorthand
    /// (`xcollect`).
    Nav(Box<Nav>),
    /// OCL iterator or `iterate`: `src->name(v, … | body)`, `src->name(v; acc : T = e | body)`.
    Iterate(Box<Iterate>),
    /// Imperative iterator: `src->xselect(…)`, `xcollect`, `selectOne`, `collectOne`,
    /// `collectselect`, `collectselectOne`.
    XIterate(Box<XIterate>),
    /// `src[cond]`, `src[x | cond]`, `src![…]` (select / selectOne shorthand).
    XSelect(Box<XSelect>),
    /// `src->forEach(i, … | cond) {…}`, `forOne`.
    ForEach(Box<ForEach>),
    /// `src->object(v) T {…}`
    ObjectIter(Box<ObjectIter>),
    /// `src->switch(v) {…}`
    SwitchIter(Box<SwitchIter>),
    /// `lhs := rhs`, `lhs += rhs`
    Assign(Box<Assign>),
    /// `var x := e`, `var x : T`, `var x := e, y := f`, `var (…)`.
    Var(VarExp),
    /// `return [e]`
    Return(Option<Box<Expr>>),
    Break,
    Continue,
    While(Box<While>),
    /// A body block `{ … }` (if branches, switch alternatives).
    Block(Block),
    /// `do { … }`
    Do(Block),
    /// `switch { case (c) s … else s }`
    Switch(Box<SwitchBody>),
    /// `log(args) [when c]`
    Log(Box<Log>),
    /// `assert [severity] (cond) [with log(…)]`
    Assert(Box<Assert>),
    /// `compute (decl) { … }`
    Compute(Box<Compute>),
    /// `new T(args)`
    New(Box<New>),
    /// `object [v :] [T] { … }`
    Object(Box<ObjectExp>),
    /// `[src.|src->]map|xmap name(args)`
    MapCall(Box<MapCall>),
    /// `[src.|src->][late] resolve…(…)`
    Resolve(Box<Resolve>),
    /// `raise T [(e)]`, `raise 'msg'`
    Raise(Box<Raise>),
    /// `try { … } except (…) { … } …`
    Try(Box<Try>),
    /// Recovery placeholder.
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CollectionPart {
    Item(Expr),
    Range(Expr, Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct DictPart {
    /// A primitive literal (`literalSimpleCS`).
    pub key: Expr,
    pub value: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IfExp {
    pub cond: Expr,
    pub then: Expr,
    pub elifs: Vec<(Expr, Expr)>,
    pub els: Option<Expr>,
    /// `if (c) s [elif (c) s] [else s] [endif]` (statement form) rather than
    /// `if c then e … endif`.
    pub c_style: bool,
    /// Whether `endif` is written (always for the OCL form).
    pub endif: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub source: Option<(Expr, NavOp)>,
    /// Qualifying path before the name (`p::q::name(…)`).
    pub qualifier: Vec<Name>,
    pub name: Name,
    pub args: Vec<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Nav {
    pub source: Expr,
    pub op: NavOp,
    pub qualifier: Vec<Name>,
    pub name: Name,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Iterate {
    pub source: Expr,
    pub name: Name,
    pub vars: Vec<VarDecl>,
    /// The accumulator of `iterate`.
    pub acc: Option<VarDecl>,
    pub body: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XIterate {
    pub source: Expr,
    /// `xselect`, `xcollect`, `selectOne`, `collectOne`, `collectselect`, `collectselectOne`.
    pub kind: TokenKind,
    /// Iterator variables; empty for the implicit form `->xselect(cond)`.
    pub vars: Vec<VarDecl>,
    /// The `t = body` target of `collectselect`.
    pub target: Option<VarDecl>,
    pub body: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct XSelect {
    pub source: Expr,
    /// `![…]` (selectOne)
    pub one: bool,
    pub var: Option<Name>,
    pub cond: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForEach {
    pub source: Expr,
    /// `forEach` or `forOne`
    pub kind: TokenKind,
    pub vars: Vec<Name>,
    pub cond: Option<Expr>,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectIter {
    pub source: Expr,
    pub var: VarDecl,
    pub decl: ObjectDecl,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SwitchIter {
    pub source: Expr,
    pub var: VarDecl,
    pub body: SwitchBody,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Assign {
    pub lhs: Expr,
    pub rhs: Expr,
    /// `+=` rather than `:=`.
    pub add: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VarExp {
    pub decls: Vec<VarDecl>,
    /// `var (…)`
    pub parens: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct While {
    pub init: Option<WhileInit>,
    pub cond: Expr,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WhileInit {
    /// `while (x : T := e; c)`
    Decl(VarDecl),
    /// `while (x := e; c)`
    Assign(Expr),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SwitchBody {
    pub alts: Vec<SwitchAlt>,
    pub els: Option<SwitchElse>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SwitchAlt {
    pub cond: Expr,
    /// An expression statement or a block.
    pub body: Expr,
    /// The deprecated `(cond) ? e;` form.
    pub deprecated: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SwitchElse {
    pub body: Expr,
    /// The deprecated `else ? e;` form.
    pub deprecated: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Log {
    pub args: Vec<Expr>,
    pub when: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Assert {
    pub severity: Option<Name>,
    pub cond: Expr,
    /// A `Log` expression.
    pub log: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Compute {
    pub decl: VarDecl,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub struct New {
    pub ty: TypeSpec,
    pub args: Vec<Expr>,
}

/// `object` target: `T`, `v : T`, `v :`, `result : T`, `self : T`.
#[derive(Clone, Debug, PartialEq)]
pub struct ObjectDecl {
    pub var: Option<Name>,
    pub ty: Option<TypeSpec>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObjectExp {
    pub decl: ObjectDecl,
    pub body: Block,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapCall {
    pub source: Option<(Expr, NavOp)>,
    pub xmap: bool,
    pub name: PathName,
    pub args: Vec<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Resolve {
    pub source: Option<(Expr, NavOp)>,
    pub late: bool,
    /// `resolve`, `resolveone`, `invresolve`, `invresolveone` and their `…In` forms.
    pub op: TokenKind,
    /// The mapping of the `…In` forms.
    pub mapping: Option<ScopedName>,
    pub target: Option<ResolveTarget>,
}

/// `[v :] T [| cond]`
#[derive(Clone, Debug, PartialEq)]
pub struct ResolveTarget {
    pub var: Option<Name>,
    pub ty: TypeExpr,
    pub cond: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Raise {
    /// A name or path (exception type), or a string literal.
    pub exception: Expr,
    /// `Some` when the argument list `(…)` is written; its content is optional.
    pub arg: Option<Option<Expr>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Try {
    pub body: Block,
    pub catches: Vec<Catch>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Catch {
    /// `except (e : T, …)`
    pub var: Option<Name>,
    pub types: Vec<TypeExpr>,
    pub body: Block,
    pub span: Span,
}

// ---------------------------------------------------------------------------------------------
// Units and module elements

/// A compilation unit (one `.qvto` file).
#[derive(Clone, Debug, PartialEq)]
pub struct Unit {
    pub elements: Vec<Element>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Element {
    Import(Import),
    Module(Module),
    ModelType(ModelType),
    Class(ClassDef),
    Property(Property),
    Helper(Helper),
    Constructor(Constructor),
    Entry(Entry),
    Mapping(Mapping),
    Tag(Tag),
    Rename(Rename),
    /// A region the parser skipped.
    Error(Span),
}

impl Element {
    pub fn span(&self) -> Span {
        match self {
            Element::Import(x) => x.span,
            Element::Module(x) => x.span,
            Element::ModelType(x) => x.span,
            Element::Class(x) => x.span,
            Element::Property(x) => x.span,
            Element::Helper(x) => x.span,
            Element::Constructor(x) => x.span,
            Element::Entry(x) => x.span,
            Element::Mapping(x) => x.span,
            Element::Tag(x) => x.span,
            Element::Rename(x) => x.span,
            Element::Error(s) => *s,
        }
    }
}

/// `import a.b.C;`, `import library a.b.C;`
#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    pub library: bool,
    pub unit: Vec<Name>,
    pub span: Span,
}

/// `transformation …` or `library …`, declaration (`;`) or definition (`{ … }`).
#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    /// `transformation` or `library`.
    pub kind: TokenKind,
    pub qualifiers: Vec<TokenKind>,
    /// Dotted name.
    pub name: Vec<Name>,
    /// Model parameters; `None` without a signature.
    pub params: Option<Vec<Param>>,
    pub usages: Vec<ModuleUsage>,
    pub refines: Option<ModuleRef>,
    /// Elements of the `{ … }` body; `None` for a declaration ending in `;`.
    pub body: Option<Vec<Element>>,
    /// Span of the header (up to and including the signature and usages).
    pub header_span: Span,
    pub span: Span,
}

/// `access|extends [transformation|library] ref, …`
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleUsage {
    /// `access` or `extends`.
    pub kind: TokenKind,
    pub module_kind: Option<TokenKind>,
    pub refs: Vec<ModuleRef>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModuleRef {
    pub path: PathName,
    pub params: Option<Vec<Param>>,
    pub span: Span,
}

/// A parameter: `[in|out|inout] [name :] T [@ extent]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub direction: Option<TokenKind>,
    pub name: Option<Name>,
    pub ty: TypeSpec,
    pub span: Span,
}

/// `modeltype M ['strict'] uses pkg('uri'), … [where { … }];`
#[derive(Clone, Debug, PartialEq)]
pub struct ModelType {
    pub name: Name,
    pub compliance: Option<StringLit>,
    pub packages: Vec<PackageRef>,
    pub where_: Option<Block>,
    pub span: Span,
}

/// `pkg`, `pkg('uri')` or `'uri'`.
#[derive(Clone, Debug, PartialEq)]
pub struct PackageRef {
    pub path: Option<PathName>,
    pub uri: Option<StringLit>,
    pub span: Span,
}

/// `intermediate class X [extends …] { … }` or `exception X [extends …] { … }`.
#[derive(Clone, Debug, PartialEq)]
pub struct ClassDef {
    pub exception: bool,
    pub name: Name,
    pub extends: Vec<TypeExpr>,
    pub features: Vec<ClassFeature>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClassFeature {
    Property(ClassProperty),
    Tag(Tag),
}

/// `[<<st, …>>] keys name : T [mult] [ordered] [opposites [~] n [mult]] [= e]`
#[derive(Clone, Debug, PartialEq)]
pub struct ClassProperty {
    pub stereotypes: Vec<Name>,
    /// `composes`, `references`, `readonly`, `derived`, `static`.
    pub keys: Vec<TokenKind>,
    pub name: Name,
    pub ty: TypeExpr,
    pub multiplicity: Option<Multiplicity>,
    pub ordered: bool,
    pub opposite: Option<Opposite>,
    pub init: Option<Expr>,
    pub span: Span,
}

/// `[n]`, `[*]`, `[l..u]`, `[l...u]`
#[derive(Clone, Debug, PartialEq)]
pub struct Multiplicity {
    pub lower: MultBound,
    pub upper: Option<MultBound>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MultBound {
    Int(i64),
    /// `*`
    Many,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Opposite {
    /// `~`
    pub navigable: bool,
    pub name: Name,
    pub multiplicity: Option<Multiplicity>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyKind {
    /// `configuration property x : T;`
    Configuration,
    /// `property x [: T] [= e];`
    Plain,
    /// `intermediate property Ctx::x : T [= e];`
    Intermediate,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Property {
    pub kind: PropertyKind,
    pub name: ScopedName,
    pub ty: Option<TypeExpr>,
    pub init: Option<Expr>,
    pub span: Span,
}

/// Body of a helper, query or constructor.
#[derive(Clone, Debug, PartialEq)]
pub enum OpBody {
    /// `;`
    None,
    /// `= e;`
    Expr(Expr),
    /// `{ … }`
    Block(Block),
}

/// `[qualifiers] helper|query [Ctx::]name(params) [: results] body`
#[derive(Clone, Debug, PartialEq)]
pub struct Helper {
    pub qualifiers: Vec<TokenKind>,
    /// `helper` or `query`.
    pub kind: TokenKind,
    pub name: ScopedName,
    pub params: Vec<Param>,
    pub results: Vec<Param>,
    pub body: OpBody,
    /// Span of the declaration (Eclipse's `MappingDeclarationCS`).
    pub decl_span: Span,
    pub span: Span,
}

/// `[qualifiers] constructor Ctx::name(params) body`
#[derive(Clone, Debug, PartialEq)]
pub struct Constructor {
    pub qualifiers: Vec<TokenKind>,
    pub name: ScopedName,
    pub params: Vec<Param>,
    pub body: Option<Block>,
    pub decl_span: Span,
    pub span: Span,
}

/// `main(params) body`
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub params: Vec<Param>,
    pub body: Option<Block>,
    pub decl_span: Span,
    pub span: Span,
}

/// `[qualifiers] mapping [dir] [Ctx::]name(params) [: results] [inherits …] [when {…}]
/// [where {…}] (; | { [init {…}] [population {…} | stmts] [end {…}] })`
#[derive(Clone, Debug, PartialEq)]
pub struct Mapping {
    pub qualifiers: Vec<TokenKind>,
    pub direction: Option<TokenKind>,
    pub name: ScopedName,
    pub params: Vec<Param>,
    pub results: Vec<Param>,
    pub extensions: Vec<MappingExtension>,
    pub when: Option<Block>,
    pub where_: Option<Block>,
    pub body: Option<MappingBody>,
    pub decl_span: Span,
    pub span: Span,
}

/// `inherits|merges|disjuncts a, b`
#[derive(Clone, Debug, PartialEq)]
pub struct MappingExtension {
    pub kind: TokenKind,
    pub names: Vec<ScopedName>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MappingBody {
    pub init: Option<Block>,
    pub population: Option<Population>,
    pub end: Option<Block>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Population {
    /// Written with the `population` keyword.
    pub explicit: bool,
    pub body: Block,
}

/// `tag "id" element [= e]`
#[derive(Clone, Debug, PartialEq)]
pub struct Tag {
    pub id: StringLit,
    pub element: ScopedName,
    pub value: Option<Expr>,
    pub span: Span,
}

/// `rename T.prop = 'name';` (deprecated)
#[derive(Clone, Debug, PartialEq)]
pub struct Rename {
    pub ty: TypeExpr,
    pub property: Name,
    pub new_name: StringLit,
    pub span: Span,
}

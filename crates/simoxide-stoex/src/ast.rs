//! Abstract syntax tree. It mirrors the EMF objects the Xtext parser creates (StoEx metamodel
//! 2.2 + PCM `CharacterisedVariable`), including [`ExprKind::Paren`] nodes, because
//! parentheses carry their own inferred type in the reference.

use crate::error::Span;

/// `TermOperations`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TermOp {
    Add,
    Sub,
}

/// `ProductOperations`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProdOp {
    Mult,
    Div,
    Mod,
}

/// `CompareOperations`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CmpOp {
    Greater,
    Less,
    Equals,
    NotEqual,
    GreaterEqual,
    LessEqual,
}

/// `BooleanOperations`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoolOp {
    And,
    Or,
    Xor,
}

/// PCM `VariableCharacterisationType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Characterisation {
    ByteSize,
    NumberOfElements,
    Structure,
    Type,
    Value,
}

impl Characterisation {
    /// The literal (`BYTESIZE`, ...), as used in stack frame ids.
    pub fn as_str(self) -> &'static str {
        match self {
            Characterisation::ByteSize => "BYTESIZE",
            Characterisation::NumberOfElements => "NUMBER_OF_ELEMENTS",
            Characterisation::Structure => "STRUCTURE",
            Characterisation::Type => "TYPE",
            Characterisation::Value => "VALUE",
        }
    }

    /// Parses a literal.
    pub fn from_literal(s: &str) -> Option<Self> {
        Some(match s {
            "BYTESIZE" => Characterisation::ByteSize,
            "NUMBER_OF_ELEMENTS" => Characterisation::NumberOfElements,
            "STRUCTURE" => Characterisation::Structure,
            "TYPE" => Characterisation::Type,
            "VALUE" => Characterisation::Value,
            _ => return None,
        })
    }
}

/// A `CharacterisedVariable`: a (namespace) reference like `a.INNER` plus a characterisation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VarRef {
    /// Reference names from the outermost namespace to the variable, e.g. `["a", "INNER"]`.
    pub path: Vec<String>,
    pub characterisation: Characterisation,
}

impl VarRef {
    /// The stack frame id the reference looks up: the Xtext serialisation of the
    /// `CharacterisedVariable`, e.g. `a.INNER.BYTESIZE` (whitespace in the source is dropped).
    pub fn id(&self) -> String {
        let mut s = self.reference_name();
        s.push('.');
        s.push_str(self.characterisation.as_str());
        s
    }

    /// The serialised `AbstractNamedReference` (`a.INNER`), as `StoExSerialiser.serialise`
    /// returns it for a variable usage.
    pub fn reference_name(&self) -> String {
        self.path.join(".")
    }

    /// True if one of the reference names is `INNER` (SimuLizar stores such characterisations
    /// as lazily re-evaluated proxies, see `SimulatedStackHelper.isInnerReference`).
    pub fn is_inner(&self) -> bool {
        self.path.iter().any(|p| p == "INNER")
    }
}

/// Probability function literals.
#[derive(Debug, Clone, PartialEq)]
pub enum ProbFnLit {
    /// `IntPMF[(v;p)...]`.
    IntPmf(Vec<(i32, f64)>),
    /// `DoublePMF[(v;p)...]`.
    DoublePmf(Vec<(f64, f64)>),
    /// `EnumPMF(ordered)?[("v";p)...]`.
    EnumPmf {
        ordered: bool,
        samples: Vec<(String, f64)>,
    },
    /// `BoolPMF(ordered)?[(true;p)...]`.
    BoolPmf {
        ordered: bool,
        samples: Vec<(bool, f64)>,
    },
    /// `DoublePDF[(v;p)...]` (a `BoxedPDF`).
    BoxedPdf(Vec<(f64, f64)>),
}

/// Expression node kinds.
#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    /// `cond ? a : b`.
    IfElse(Box<Expr>, Box<Expr>, Box<Expr>),
    BoolOp(BoolOp, Box<Expr>, Box<Expr>),
    Compare(CmpOp, Box<Expr>, Box<Expr>),
    Term(TermOp, Box<Expr>, Box<Expr>),
    Product(ProdOp, Box<Expr>, Box<Expr>),
    /// `base ^ exponent`.
    Power(Box<Expr>, Box<Expr>),
    /// `- inner`.
    Neg(Box<Expr>),
    /// `NOT inner`.
    Not(Box<Expr>),
    Int(i32),
    Double(f64),
    Str(String),
    Bool(bool),
    /// `Name(args)`.
    Func(String, Vec<Expr>),
    Var(VarRef),
    /// `( inner )`.
    Paren(Box<Expr>),
    ProbFn(ProbFnLit),
}

/// An expression node with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Expr { kind, span }
    }

    /// Node without a meaningful span (for constructed trees).
    pub fn synth(kind: ExprKind) -> Self {
        Expr {
            kind,
            span: Span::default(),
        }
    }

    /// Direct children in evaluation order.
    pub fn children(&self) -> Vec<&Expr> {
        match &self.kind {
            ExprKind::IfElse(c, a, b) => vec![c, a, b],
            ExprKind::BoolOp(_, l, r)
            | ExprKind::Compare(_, l, r)
            | ExprKind::Term(_, l, r)
            | ExprKind::Product(_, l, r)
            | ExprKind::Power(l, r) => vec![l, r],
            ExprKind::Neg(e) | ExprKind::Not(e) | ExprKind::Paren(e) => vec![e],
            ExprKind::Func(_, args) => args.iter().collect(),
            _ => vec![],
        }
    }

    /// Visits all nodes in pre-order (the order of `EcoreUtil.getAllContents`).
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Expr)) {
        f(self);
        for c in self.children() {
            c.walk(f);
        }
    }

    /// All variable references in pre-order.
    pub fn variables(&self) -> Vec<&VarRef> {
        let mut v = Vec::new();
        self.walk(&mut |e| {
            if let ExprKind::Var(r) = &e.kind {
                v.push(r);
            }
        });
        v
    }

    /// Canonical, re-parsable text (see [`crate::print`]).
    pub fn to_stoex(&self) -> String {
        crate::print::print(self)
    }
}

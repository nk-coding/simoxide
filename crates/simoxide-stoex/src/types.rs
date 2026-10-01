//! Static type inference exactly as the simulator runs it: `ExpressionInferTypeVisitor` with
//! the PCM `TypeInference` extension, seen through `NonProbabilisticExpressionInferTypeVisitor`
//! (stoex.analyser 5.2.2).
//!
//! The evaluator does not use these types to check anything; it uses them to *choose the
//! arithmetic* (int or double) and casts, which is why they must be reproduced including their
//! oddities:
//!
//! * `%` is always typed INT (so `(5.5 % 2) + 1` fails at run time);
//! * `^` of two INTs is typed INT although it evaluates to a Double;
//! * `Log`, `Sqrt` are typed INT although they return Doubles;
//! * `x.BYTESIZE`/`x.NUMBER_OF_ELEMENTS` are INT, `x.VALUE`/`TYPE`/`STRUCTURE` are ANY (dynamic);
//! * `?:` is ANY; comparisons and boolean operators are BOOL;
//! * incompatible operand types of `+ - * /` give *no* type (`None`, Java `null`).

use crate::ast::*;
use crate::error::PrepareError;

/// `TypeEnum` of the stoex analyser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeEnum {
    Int,
    Bool,
    Double,
    Enum,
    IntPmf,
    DoublePmf,
    EnumPmf,
    BoolPmf,
    DoublePdf,
    AnyPmf,
    Any,
}

impl TypeEnum {
    /// The Java enum literal.
    pub fn name(self) -> &'static str {
        match self {
            TypeEnum::Int => "INT",
            TypeEnum::Bool => "BOOL",
            TypeEnum::Double => "DOUBLE",
            TypeEnum::Enum => "ENUM",
            TypeEnum::IntPmf => "INT_PMF",
            TypeEnum::DoublePmf => "DOUBLE_PMF",
            TypeEnum::EnumPmf => "ENUM_PMF",
            TypeEnum::BoolPmf => "BOOL_PMF",
            TypeEnum::DoublePdf => "DOUBLE_PDF",
            TypeEnum::AnyPmf => "ANY_PMF",
            TypeEnum::Any => "ANY",
        }
    }

    /// `NonProbabilisticExpressionInferTypeVisitor.getType` mapping.
    pub fn non_probabilistic(self) -> TypeEnum {
        match self {
            TypeEnum::IntPmf => TypeEnum::Int,
            TypeEnum::DoublePmf | TypeEnum::DoublePdf => TypeEnum::Double,
            TypeEnum::EnumPmf => TypeEnum::Enum,
            TypeEnum::BoolPmf => TypeEnum::Bool,
            TypeEnum::AnyPmf => TypeEnum::Any,
            t => t,
        }
    }
}

/// Static type of a node as the evaluator sees it (`typeInferer.getType(node)`), `None` for
/// Java `null`.
pub type SType = Option<TypeEnum>;

/// Names the type inference knows (`ExpressionInferTypeVisitor.caseFunctionLiteral`).
pub fn function_raw_type(name: &str, first_arg: Option<SType>) -> Option<SType> {
    Some(match name {
        "UniDouble" | "Lognorm" | "LognormMoments" | "Norm" | "Gamma" | "GammaMoments" | "Exp" => {
            Some(TypeEnum::DoublePdf)
        }
        "Pois" | "UniInt" | "Trunc" | "Round" | "Ceil" | "Log" | "Sqrt" | "Binom" => {
            Some(TypeEnum::IntPmf)
        }
        "Min" | "Max" | "MinDeviation" | "MaxDeviation" => match first_arg {
            Some(t) => t,
            None => Some(TypeEnum::Any),
        },
        _ => return None,
    })
}

fn is_int_pmf(t: TypeEnum) -> bool {
    matches!(t, TypeEnum::Int | TypeEnum::IntPmf)
}

fn is_numeric(t: TypeEnum) -> bool {
    matches!(t, TypeEnum::Int | TypeEnum::Double)
}

fn is_double_int_pmf(t: TypeEnum) -> bool {
    matches!(
        t,
        TypeEnum::Double | TypeEnum::Int | TypeEnum::IntPmf | TypeEnum::DoublePmf
    )
}

fn is_double_int_any_pmf(t: TypeEnum) -> bool {
    matches!(
        t,
        TypeEnum::Double
            | TypeEnum::Int
            | TypeEnum::Any
            | TypeEnum::IntPmf
            | TypeEnum::DoublePmf
            | TypeEnum::AnyPmf
    )
}

fn is_double_int_pdf(t: TypeEnum) -> bool {
    matches!(
        t,
        TypeEnum::Double
            | TypeEnum::Int
            | TypeEnum::IntPmf
            | TypeEnum::DoublePmf
            | TypeEnum::DoublePdf
    )
}

/// `inferIntAndDouble` for `+ - * /`.
pub fn infer_int_and_double(l: SType, r: SType) -> SType {
    let (l, r) = (l?, r?);
    if l == TypeEnum::Int && r == TypeEnum::Int {
        Some(TypeEnum::Int)
    } else if is_int_pmf(l) && is_int_pmf(r) {
        Some(TypeEnum::IntPmf)
    } else if is_numeric(l) && is_numeric(r) {
        Some(TypeEnum::Double)
    } else if is_double_int_pmf(l) && is_double_int_pmf(r) {
        Some(TypeEnum::DoublePmf)
    } else if is_double_int_any_pmf(l) && is_double_int_any_pmf(r) {
        Some(TypeEnum::AnyPmf)
    } else if is_double_int_pdf(l) && is_double_int_pdf(r) {
        Some(TypeEnum::DoublePdf)
    } else {
        // Java logs "Type inference ... failed" and leaves the node untyped.
        None
    }
}

/// Raw type annotation (`typeAnnotation` map, before the non-probabilistic mapping) of a node,
/// computed bottom-up. Fails for function names the analyser does not know.
pub fn raw_type(e: &Expr) -> Result<SType, PrepareError> {
    Ok(match &e.kind {
        ExprKind::Compare(_, l, r) => {
            raw_type(l)?;
            raw_type(r)?;
            Some(TypeEnum::BoolPmf)
        }
        ExprKind::Product(op, l, r) => {
            let (lt, rt) = (raw_type(l)?, raw_type(r)?);
            match op {
                ProdOp::Mod => Some(TypeEnum::IntPmf),
                _ => infer_int_and_double(lt, rt),
            }
        }
        ExprKind::Term(_, l, r) => infer_int_and_double(raw_type(l)?, raw_type(r)?),
        ExprKind::Power(b, x) => {
            // NonProbabilisticExpressionInferTypeVisitor.casePowerExpression (uses getType).
            let bt = raw_type(b)?.map(TypeEnum::non_probabilistic);
            let xt = raw_type(x)?.map(TypeEnum::non_probabilistic);
            match (bt, xt) {
                (Some(TypeEnum::Int), Some(TypeEnum::Int)) => Some(TypeEnum::Int),
                (Some(b), Some(x)) if is_numeric(b) && is_numeric(x) => Some(TypeEnum::Double),
                _ => Some(TypeEnum::Any),
            }
        }
        ExprKind::Neg(i) | ExprKind::Paren(i) => raw_type(i)?,
        ExprKind::BoolOp(_, l, r) => {
            raw_type(l)?;
            raw_type(r)?;
            Some(TypeEnum::Bool)
        }
        ExprKind::Not(i) => {
            raw_type(i)?;
            Some(TypeEnum::Bool)
        }
        ExprKind::ProbFn(p) => Some(match p {
            ProbFnLit::IntPmf(_) => TypeEnum::IntPmf,
            ProbFnLit::DoublePmf(_) => TypeEnum::DoublePmf,
            ProbFnLit::EnumPmf { .. } => TypeEnum::EnumPmf,
            ProbFnLit::BoolPmf { .. } => TypeEnum::BoolPmf,
            ProbFnLit::BoxedPdf(_) => TypeEnum::DoublePdf,
        }),
        ExprKind::Int(_) => Some(TypeEnum::Int),
        ExprKind::Double(_) => Some(TypeEnum::Double),
        ExprKind::Str(_) => Some(TypeEnum::Enum),
        ExprKind::Bool(_) => Some(TypeEnum::Bool),
        ExprKind::Var(v) => Some(match v.characterisation {
            Characterisation::Value | Characterisation::Type | Characterisation::Structure => {
                TypeEnum::AnyPmf
            }
            Characterisation::NumberOfElements | Characterisation::ByteSize => TypeEnum::IntPmf,
        }),
        ExprKind::Func(name, args) => {
            let mut first = None;
            for (i, a) in args.iter().enumerate() {
                let t = raw_type(a)?;
                if i == 0 {
                    first = Some(t);
                }
            }
            match function_raw_type(name, first) {
                Some(t) => t,
                None => {
                    return Err(PrepareError::UnknownFunction {
                        name: name.clone(),
                        span: e.span,
                    });
                }
            }
        }
        ExprKind::IfElse(c, a, b) => {
            raw_type(c)?;
            raw_type(b)?;
            raw_type(a)?;
            Some(TypeEnum::Any)
        }
    })
}

/// The type the evaluator sees for a node (`NonProbabilisticExpressionInferTypeVisitor.getType`).
pub fn eval_type(e: &Expr) -> Result<SType, PrepareError> {
    Ok(raw_type(e)?.map(TypeEnum::non_probabilistic))
}

/// Checks all function names (the full `typeInferer.doSwitch(formula)` pass of
/// `StoExCacheEntry`).
pub fn check(e: &Expr) -> Result<(), PrepareError> {
    raw_type(e).map(|_| ())
}

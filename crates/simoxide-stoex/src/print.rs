//! Canonical printer. `parse(print(e))` reproduces `e` exactly (same tree, same literal bits)
//! for every tree the parser can produce.
//!
//! This is not the Xtext serialiser of the reference, whose formatter output for whole
//! expressions is irregular (and sometimes throws); only variable ids ([`VarRef::id`]) are
//! used by the simulator and those are reproduced exactly.

use crate::ast::*;
use std::fmt::Write;

/// Prints an expression.
pub fn print(e: &Expr) -> String {
    let mut s = String::new();
    write_expr(&mut s, e);
    s
}

fn write_expr(s: &mut String, e: &Expr) {
    match &e.kind {
        ExprKind::IfElse(c, a, b) => {
            write_expr(s, c);
            s.push_str(" ? ");
            write_expr(s, a);
            s.push_str(" : ");
            write_expr(s, b);
        }
        ExprKind::BoolOp(op, l, r) => {
            write_expr(s, l);
            s.push_str(match op {
                BoolOp::And => " AND ",
                BoolOp::Or => " OR ",
                BoolOp::Xor => " XOR ",
            });
            write_expr(s, r);
        }
        ExprKind::Compare(op, l, r) => {
            write_expr(s, l);
            s.push_str(match op {
                CmpOp::Greater => " > ",
                CmpOp::Less => " < ",
                CmpOp::Equals => " == ",
                CmpOp::NotEqual => " <> ",
                CmpOp::GreaterEqual => " >= ",
                CmpOp::LessEqual => " <= ",
            });
            write_expr(s, r);
        }
        ExprKind::Term(op, l, r) => {
            write_expr(s, l);
            s.push_str(match op {
                TermOp::Add => " + ",
                TermOp::Sub => " - ",
            });
            write_expr(s, r);
        }
        ExprKind::Product(op, l, r) => {
            write_expr(s, l);
            s.push_str(match op {
                ProdOp::Mult => " * ",
                ProdOp::Div => " / ",
                ProdOp::Mod => " % ",
            });
            write_expr(s, r);
        }
        ExprKind::Power(b, x) => {
            write_expr(s, b);
            s.push_str(" ^ ");
            write_expr(s, x);
        }
        ExprKind::Neg(i) => {
            s.push('-');
            write_expr(s, i);
        }
        ExprKind::Not(i) => {
            s.push_str("NOT ");
            write_expr(s, i);
        }
        ExprKind::Int(v) => {
            let _ = write!(s, "{v}");
        }
        ExprKind::Double(v) => s.push_str(&double_literal(*v)),
        ExprKind::Str(v) => write_string(s, v),
        ExprKind::Bool(b) => s.push_str(if *b { "true" } else { "false" }),
        ExprKind::Func(name, args) => {
            s.push_str(name);
            s.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    s.push_str(", ");
                }
                write_expr(s, a);
            }
            s.push(')');
        }
        ExprKind::Var(v) => s.push_str(&v.id()),
        ExprKind::Paren(i) => {
            s.push('(');
            write_expr(s, i);
            s.push(')');
        }
        ExprKind::ProbFn(p) => write_probfn(s, p),
    }
}

/// A `DOUBLE` token for a non-negative double (shortest round-trip digits, always with '.' or
/// an exponent). Infinity prints as `1e999`.
pub fn double_literal(v: f64) -> String {
    if v.is_infinite() {
        return if v > 0.0 {
            "1e999".into()
        } else {
            "-1e999".into()
        };
    }
    let mut t = format!("{v:?}");
    if !t.contains(['.', 'e', 'E']) {
        t.push_str(".0");
    }
    t
}

fn number(v: f64) -> String {
    double_literal(v)
}

fn write_string(s: &mut String, v: &str) {
    s.push('"');
    for c in v.chars() {
        match c {
            '"' => s.push_str("\\\""),
            '\\' => s.push_str("\\\\"),
            '\u{8}' => s.push_str("\\b"),
            '\u{c}' => s.push_str("\\f"),
            _ => s.push(c),
        }
    }
    s.push('"');
}

fn write_probfn(s: &mut String, p: &ProbFnLit) {
    match p {
        ProbFnLit::IntPmf(v) => {
            s.push_str("IntPMF[");
            for (x, p) in v {
                let _ = write!(s, "({x};{})", number(*p));
            }
        }
        ProbFnLit::DoublePmf(v) => {
            s.push_str("DoublePMF[");
            for (x, p) in v {
                let _ = write!(s, "({};{})", double_literal(*x), number(*p));
            }
        }
        ProbFnLit::BoxedPdf(v) => {
            s.push_str("DoublePDF[");
            for (x, p) in v {
                let _ = write!(s, "({};{})", double_literal(*x), number(*p));
            }
        }
        ProbFnLit::EnumPmf { ordered, samples } => {
            s.push_str(if *ordered {
                "EnumPMF(ordered)["
            } else {
                "EnumPMF["
            });
            for (x, p) in samples {
                s.push('(');
                write_string(s, x);
                let _ = write!(s, ";{})", number(*p));
            }
        }
        ProbFnLit::BoolPmf { ordered, samples } => {
            s.push_str(if *ordered {
                "BoolPMF(ordered)["
            } else {
                "BoolPMF["
            });
            for (x, p) in samples {
                let _ = write!(s, "({x};{})", number(*p));
            }
        }
    }
    s.push(']');
}

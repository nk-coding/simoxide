//! Property tests: printer/parser round trip on random grammatical trees, and the compiled
//! program (folding, specialisation) against the reference tree walker.

use proptest::prelude::*;
use simoxide_stoex::ast::*;
use simoxide_stoex::{Program, SimpleEnv, Value, VariableMode};
use std::sync::Arc;

const KEYWORDS: [&str; 17] = [
    "NOT",
    "AND",
    "OR",
    "XOR",
    "IntPMF",
    "DoublePMF",
    "EnumPMF",
    "BoolPMF",
    "DoublePDF",
    "ordered",
    "BYTESIZE",
    "NUMBER_OF_ELEMENTS",
    "STRUCTURE",
    "TYPE",
    "VALUE",
    "true",
    "false",
];

fn ident() -> impl Strategy<Value = String> {
    "[A-Za-z_][A-Za-z0-9_]{0,6}".prop_filter("keyword", |s| !KEYWORDS.contains(&s.as_str()))
}

fn characterisation() -> impl Strategy<Value = Characterisation> {
    prop_oneof![
        Just(Characterisation::Value),
        Just(Characterisation::ByteSize),
        Just(Characterisation::NumberOfElements),
        Just(Characterisation::Type),
        Just(Characterisation::Structure),
    ]
}

fn prob() -> impl Strategy<Value = f64> {
    prop_oneof![
        Just(0.0),
        Just(0.5),
        Just(0.25),
        Just(1.0),
        Just(0.1),
        (0.0f64..2.0),
        Just(f64::INFINITY)
    ]
}

fn any_double() -> impl Strategy<Value = f64> {
    prop_oneof![
        Just(0.0),
        Just(-0.0),
        Just(1.5),
        Just(1e300),
        Just(f64::INFINITY),
        (-1e6f64..1e6),
        any::<f64>().prop_filter("finite", |d| d.is_finite()),
    ]
}

fn probfn() -> impl Strategy<Value = ProbFnLit> {
    prop_oneof![
        prop::collection::vec((any::<i32>(), prob()), 1..4).prop_map(ProbFnLit::IntPmf),
        prop::collection::vec((any_double(), prob()), 1..4).prop_map(ProbFnLit::DoublePmf),
        prop::collection::vec((any_double().prop_map(f64::abs), prob()), 1..4)
            .prop_map(ProbFnLit::BoxedPdf),
        (
            any::<bool>(),
            prop::collection::vec(("[a-c\"'\\\\\n é]{0,3}", prob()), 1..4)
        )
            .prop_map(|(ordered, samples)| ProbFnLit::EnumPmf { ordered, samples }),
        (
            any::<bool>(),
            prop::collection::vec((any::<bool>(), prob()), 1..4)
        )
            .prop_map(|(ordered, samples)| ProbFnLit::BoolPmf { ordered, samples }),
    ]
}

fn leaf() -> impl Strategy<Value = Expr> {
    prop_oneof![
        (0i32..=i32::MAX).prop_map(ExprKind::Int),
        prop_oneof![Just(0i32), Just(1), Just(2), Just(7)].prop_map(ExprKind::Int),
        any_double().prop_map(|d| ExprKind::Double(d.abs())),
        "[a-z\"'\\\\\n\t é\u{1F600}]{0,4}".prop_map(ExprKind::Str),
        any::<bool>().prop_map(ExprKind::Bool),
        (prop::collection::vec(ident(), 1..3), characterisation()).prop_map(
            |(path, characterisation)| ExprKind::Var(VarRef {
                path,
                characterisation
            })
        ),
        prop_oneof![
            Just("x"),
            Just("y"),
            Just("b"),
            Just("s"),
            Just("n"),
            Just("m")
        ]
        .prop_map(|v| ExprKind::Var(VarRef {
            path: vec![v.to_string()],
            characterisation: if v == "n" {
                Characterisation::ByteSize
            } else {
                Characterisation::Value
            }
        })),
        probfn().prop_map(ExprKind::ProbFn),
    ]
    .prop_map(Expr::synth)
}

const FUNCS: [&str; 20] = [
    "Trunc",
    "Round",
    "Ceil",
    "Sqrt",
    "Log",
    "Min",
    "Max",
    "MinDeviation",
    "MaxDeviation",
    "Binom",
    "Exp",
    "Norm",
    "Pois",
    "UniDouble",
    "UniInt",
    "Gamma",
    "GammaMoments",
    "Lognorm",
    "LognormMoments",
    "Unknown",
];

fn expr() -> impl Strategy<Value = Expr> {
    expr_with(&FUNCS)
}

/// Functions whose sampling is cheap for any argument (the inverse CDFs of Norm, Gamma,
/// Lognorm and Pois use iterative solvers that are slow for extreme random arguments, as in
/// the reference; they are covered by the golden tests).
const CHEAP_FUNCS: [&str; 14] = [
    "Trunc",
    "Round",
    "Ceil",
    "Sqrt",
    "Log",
    "Min",
    "Max",
    "MinDeviation",
    "MaxDeviation",
    "Binom",
    "Exp",
    "UniDouble",
    "UniInt",
    "Unknown",
];

fn expr_with(funcs: &'static [&'static str]) -> impl Strategy<Value = Expr> {
    leaf()
        .prop_recursive(5, 48, 4, |inner| {
            let b = || inner.clone().prop_map(Box::new);
            prop_oneof![
                (b(), b(), b()).prop_map(|(c, x, y)| ExprKind::IfElse(c, x, y)),
                (
                    prop_oneof![Just(BoolOp::And), Just(BoolOp::Or), Just(BoolOp::Xor)],
                    b(),
                    b()
                )
                    .prop_map(|(o, l, r)| ExprKind::BoolOp(o, l, r)),
                (
                    prop_oneof![
                        Just(CmpOp::Equals),
                        Just(CmpOp::Less),
                        Just(CmpOp::Greater),
                        Just(CmpOp::NotEqual),
                        Just(CmpOp::LessEqual),
                        Just(CmpOp::GreaterEqual)
                    ],
                    b(),
                    b()
                )
                    .prop_map(|(o, l, r)| ExprKind::Compare(o, l, r)),
                (prop_oneof![Just(TermOp::Add), Just(TermOp::Sub)], b(), b())
                    .prop_map(|(o, l, r)| ExprKind::Term(o, l, r)),
                (
                    prop_oneof![Just(ProdOp::Mult), Just(ProdOp::Div), Just(ProdOp::Mod)],
                    b(),
                    b()
                )
                    .prop_map(|(o, l, r)| ExprKind::Product(o, l, r)),
                (b(), b()).prop_map(|(l, r)| ExprKind::Power(l, r)),
                b().prop_map(ExprKind::Neg),
                b().prop_map(ExprKind::Not),
                b().prop_map(ExprKind::Paren),
                (
                    prop::sample::select(funcs.to_vec()),
                    prop::collection::vec(inner.clone(), 0..4)
                )
                    .prop_map(|(f, a)| ExprKind::Func(f.to_string(), a)),
            ]
            .prop_map(Expr::synth)
        })
        .prop_map(|e| fix(e, 0))
}

/// Precedence level of a node (0 = ifelse ... 8 = atom).
fn level(e: &Expr) -> u8 {
    match &e.kind {
        ExprKind::IfElse(..) => 0,
        ExprKind::BoolOp(BoolOp::And, ..) => 1,
        ExprKind::BoolOp(..) => 2,
        ExprKind::Compare(..) => 3,
        ExprKind::Term(..) => 4,
        ExprKind::Product(..) => 5,
        ExprKind::Power(..) => 6,
        ExprKind::Neg(_) | ExprKind::Not(_) => 7,
        _ => 8,
    }
}

/// Makes a random tree grammatical by inserting parentheses where the grammar needs them.
fn fix(e: Expr, min: u8) -> Expr {
    let f = |c: Box<Expr>, m: u8| Box::new(fix(*c, m));
    let kind = match e.kind {
        ExprKind::IfElse(c, a, b) => ExprKind::IfElse(f(c, 1), f(a, 1), f(b, 1)),
        ExprKind::BoolOp(BoolOp::And, l, r) => ExprKind::BoolOp(BoolOp::And, f(l, 1), f(r, 2)),
        ExprKind::BoolOp(op, l, r) => ExprKind::BoolOp(op, f(l, 2), f(r, 3)),
        ExprKind::Compare(op, l, r) => ExprKind::Compare(op, f(l, 4), f(r, 4)),
        ExprKind::Term(op, l, r) => ExprKind::Term(op, f(l, 4), f(r, 5)),
        ExprKind::Product(op, l, r) => ExprKind::Product(op, f(l, 5), f(r, 6)),
        ExprKind::Power(l, r) => ExprKind::Power(f(l, 7), f(r, 7)),
        ExprKind::Neg(i) => ExprKind::Neg(f(i, 7)),
        ExprKind::Not(i) => ExprKind::Not(f(i, 7)),
        ExprKind::Paren(i) => ExprKind::Paren(f(i, 0)),
        ExprKind::Func(n, args) => ExprKind::Func(n, args.into_iter().map(|a| fix(a, 1)).collect()),
        k => k,
    };
    let e = Expr::synth(kind);
    if level(&e) < min {
        Expr::synth(ExprKind::Paren(Box::new(e)))
    } else {
        e
    }
}

fn strip(e: &Expr) -> Expr {
    let s = |b: &Expr| Box::new(strip(b));
    Expr::synth(match &e.kind {
        ExprKind::IfElse(c, a, b) => ExprKind::IfElse(s(c), s(a), s(b)),
        ExprKind::BoolOp(o, l, r) => ExprKind::BoolOp(*o, s(l), s(r)),
        ExprKind::Compare(o, l, r) => ExprKind::Compare(*o, s(l), s(r)),
        ExprKind::Term(o, l, r) => ExprKind::Term(*o, s(l), s(r)),
        ExprKind::Product(o, l, r) => ExprKind::Product(*o, s(l), s(r)),
        ExprKind::Power(l, r) => ExprKind::Power(s(l), s(r)),
        ExprKind::Neg(i) => ExprKind::Neg(s(i)),
        ExprKind::Not(i) => ExprKind::Not(s(i)),
        ExprKind::Paren(i) => ExprKind::Paren(s(i)),
        ExprKind::Func(n, a) => ExprKind::Func(n.clone(), a.iter().map(strip).collect()),
        k => k.clone(),
    })
}

/// Bitwise tree equality (doubles by bits).
fn same_tree(a: &Expr, b: &Expr) -> bool {
    format!("{:?}", strip(a)) == format!("{:?}", strip(b))
        && simoxide_stoex::print::print(a) == simoxide_stoex::print::print(b)
}

struct Tape {
    t: Vec<f64>,
    pos: usize,
}

impl simoxide_random::UniformSource for Tape {
    fn next_uniform(&mut self) -> f64 {
        let u = self.t[self.pos % self.t.len()];
        self.pos += 1;
        u
    }
}

fn env() -> SimpleEnv {
    let mut base = SimpleEnv::new();
    base.set("x.VALUE", Value::Int(7));
    base.set("y.VALUE", Value::Double(-2.5));
    base.set("b.VALUE", Value::Bool(true));
    base.set("s.VALUE", Value::from("abc"));
    base.set("n.BYTESIZE", Value::Int(100));
    let proxy =
        Program::from_str("IntPMF[(1;0.5)(2;0.5)] * x.VALUE", |v| base.slot(&v.id())).unwrap();
    let mut env = base.clone();
    env.set_proxy("m.VALUE", Arc::new(proxy), Arc::new(base));
    env
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 3000, .. ProptestConfig::default() })]

    #[test]
    fn print_parse_round_trip(e in expr()) {
        let text = simoxide_stoex::print::print(&e);
        let parsed = simoxide_stoex::parse(&text);
        prop_assert!(parsed.is_ok(), "{text}: {parsed:?}");
        let parsed = parsed.unwrap();
        prop_assert!(same_tree(&e, &parsed), "{text}\n{e:?}\n{parsed:?}");
    }

}

proptest! {
    // Distribution functions with extreme random arguments (e.g. Pois(2147483647)) are slow in
    // the reference algorithms too; keep the default run short.
    #![proptest_config(ProptestConfig { cases: 300, .. ProptestConfig::default() })]

    #[test]
    fn program_matches_interpreter(e in expr_with(&CHEAP_FUNCS), tape in prop::collection::vec(0.0f64..1.0, 1..8),
                                   mode in 0u8..3) {
        let text = simoxide_stoex::print::print(&e);
        let Ok(prepared) = simoxide_stoex::prepare(&text) else { return Ok(()); };
        let mode = [VariableMode::ExceptionOnNotFound, VariableMode::ReturnDefaultOnNotFound,
                    VariableMode::ReturnNullOnNotFound][mode as usize];
        let mut env = env();
        let slots: std::collections::HashMap<String, u32> =
            prepared.expr.variables().iter().map(|v| (v.id(), env.slot(&v.id()))).collect();
        let program = Program::compile(&prepared, |v| slots[&v.id()]);
        for _ in 0..2 {
            let mut r1 = Tape { t: tape.clone(), pos: 0 };
            let mut r2 = Tape { t: tape.clone(), pos: 0 };
            let a = program.eval_mode(&env, &mut r1, mode);
            let mut res = |v: &VarRef| slots[&v.id()];
            let b = simoxide_stoex::interp::eval(&prepared, &mut res, &env, &mut r2, mode);
            prop_assert_eq!(r1.pos, r2.pos, "draws for {}", text);
            match (&a, &b) {
                (Ok(x), Ok(y)) => prop_assert!(x == y, "{text}: {x:?} vs {y:?}"),
                (Err(x), Err(y)) => prop_assert_eq!(x.kind(), y.kind(), "{}", text),
                _ => prop_assert!(false, "{text}: {a:?} vs {b:?}"),
            }
        }
    }
}

//! Golden tests against the SimuLizar 5.2.2 StoEx oracle
//! (`reference/oracles/stoex`, regenerate with `run.sh`).
//!
//! For every case: parse acceptance, tree structure (literal bits included), variable ids,
//! preparation errors, static root type, and for every evaluation the result (bitwise for
//! doubles) or the Java exception class, plus the exact number of uniforms consumed. Each case
//! is evaluated with the compiled [`Program`] and with the reference tree walker.

use serde_json::Value as J;
use simoxide_stoex::ast::*;
use simoxide_stoex::{EvalError, Program, SimpleEnv, Value, VariableMode};
use std::collections::HashMap;
use std::sync::Arc;

const GOLDEN: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../reference/oracles/stoex/golden/"
);

/// Replays recorded uniforms; counts reads past the end.
struct Tape {
    t: Vec<f64>,
    pos: usize,
    overrun: usize,
}

impl simoxide_random::UniformSource for Tape {
    fn next_uniform(&mut self) -> f64 {
        if self.pos < self.t.len() {
            self.pos += 1;
            self.t[self.pos - 1]
        } else {
            self.overrun += 1;
            0.5
        }
    }
}

fn hex(s: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap())
}

fn num(v: &J) -> f64 {
    match v {
        J::String(s) if s.starts_with("0x") => hex(s),
        J::String(s) => s.parse().unwrap(),
        J::Number(n) => n.as_f64().unwrap(),
        _ => panic!("not a number: {v}"),
    }
}

fn q(s: &str) -> String {
    // Same escaping as the Java oracle's q()
    let mut b = String::from("\"");
    for c in s.encode_utf16() {
        match c {
            0x22 => b.push_str("\\\""),
            0x5c => b.push_str("\\\\"),
            0x0a => b.push_str("\\n"),
            0x0d => b.push_str("\\r"),
            0x09 => b.push_str("\\t"),
            c if !(0x20..=0x7e).contains(&c) => b.push_str(&format!("\\u{c:04x}")),
            c => b.push(c as u8 as char),
        }
    }
    b.push('"');
    b
}

fn h(d: f64) -> String {
    format!("{:016x}", d.to_bits())
}

/// S-expression in the format of `StoexOracle.tree`.
fn sexpr(e: &Expr) -> String {
    let bin = |head: &str, l: &Expr, r: &Expr| format!("({head} {} {})", sexpr(l), sexpr(r));
    match &e.kind {
        ExprKind::IfElse(c, a, b) => format!("(ifelse {} {} {})", sexpr(c), sexpr(a), sexpr(b)),
        ExprKind::BoolOp(op, l, r) => bin(
            match op {
                BoolOp::And => "bool AND",
                BoolOp::Or => "bool OR",
                BoolOp::Xor => "bool XOR",
            },
            l,
            r,
        ),
        ExprKind::Compare(op, l, r) => bin(
            match op {
                CmpOp::Greater => "cmp GREATER",
                CmpOp::Less => "cmp LESS",
                CmpOp::Equals => "cmp EQUALS",
                CmpOp::NotEqual => "cmp NOTEQUAL",
                CmpOp::GreaterEqual => "cmp GREATEREQUAL",
                CmpOp::LessEqual => "cmp LESSEQUAL",
            },
            l,
            r,
        ),
        ExprKind::Term(op, l, r) => bin(
            match op {
                TermOp::Add => "term ADD",
                TermOp::Sub => "term SUB",
            },
            l,
            r,
        ),
        ExprKind::Product(op, l, r) => bin(
            match op {
                ProdOp::Mult => "prod MULT",
                ProdOp::Div => "prod DIV",
                ProdOp::Mod => "prod MOD",
            },
            l,
            r,
        ),
        ExprKind::Power(b, x) => bin("pow", b, x),
        ExprKind::Neg(i) => format!("(neg {})", sexpr(i)),
        ExprKind::Not(i) => format!("(not {})", sexpr(i)),
        ExprKind::Int(v) => format!("(int {v})"),
        ExprKind::Double(v) => format!("(double {})", h(*v)),
        ExprKind::Str(s) => format!("(str {})", q(s)),
        ExprKind::Bool(b) => format!("(boolean {b})"),
        ExprKind::Func(name, args) => {
            let mut s = format!("(call {name}");
            for a in args {
                s.push(' ');
                s.push_str(&sexpr(a));
            }
            s + ")"
        }
        ExprKind::Var(v) => {
            let mut s = "(var".to_string();
            for p in &v.path {
                s.push(' ');
                s.push_str(&q(p));
            }
            format!("{s} {})", v.characterisation.as_str())
        }
        ExprKind::Paren(i) => format!("(paren {})", sexpr(i)),
        ExprKind::ProbFn(p) => {
            let mut s = String::new();
            match p {
                ProbFnLit::IntPmf(v) => {
                    s.push_str("(intpmf");
                    for (x, p) in v {
                        s.push_str(&format!(" ({x} {})", h(*p)));
                    }
                }
                ProbFnLit::DoublePmf(v) => {
                    s.push_str("(doublepmf");
                    for (x, p) in v {
                        s.push_str(&format!(" ({} {})", h(*x), h(*p)));
                    }
                }
                ProbFnLit::BoxedPdf(v) => {
                    s.push_str("(pdf");
                    for (x, p) in v {
                        s.push_str(&format!(" ({} {})", h(*x), h(*p)));
                    }
                }
                ProbFnLit::EnumPmf { ordered, samples } => {
                    s.push_str("(enumpmf");
                    if *ordered {
                        s.push_str(" ordered");
                    }
                    for (x, p) in samples {
                        s.push_str(&format!(" ({} {})", q(x), h(*p)));
                    }
                }
                ProbFnLit::BoolPmf { ordered, samples } => {
                    s.push_str("(boolpmf");
                    if *ordered {
                        s.push_str(" ordered");
                    }
                    for (x, p) in samples {
                        s.push_str(&format!(" ({x} {})", h(*p)));
                    }
                }
            }
            s + ")"
        }
    }
}

fn java_value(v: &J) -> Value {
    match v["t"].as_str().unwrap() {
        "Integer" => Value::Int(v["v"].as_i64().unwrap() as i32),
        "Double" => Value::Double(hex(v["v"].as_str().unwrap())),
        "Boolean" => Value::Bool(v["v"].as_bool().unwrap()),
        "String" => Value::from(v["v"].as_str().unwrap()),
        "null" => Value::Null,
        t => panic!("unexpected result type {t}"),
    }
}

/// Java exception class -> does the Rust error correspond?
fn same_error(java_class: &str, e: &EvalError) -> bool {
    let r = e.java_class();
    if r == java_class {
        return true;
    }
    matches!(e.kind(), simoxide_stoex::EvalErrorKind::Distribution)
        && (java_class.starts_with("org.apache.commons.math")
            || java_class.ends_with("ProbabilityFunctionException")
            || java_class == "java.lang.IllegalArgumentException")
}

fn build_env(vars: &[J]) -> SimpleEnv {
    let mut base = SimpleEnv::new();
    let mut proxies = Vec::new();
    for v in vars {
        let id = v["id"].as_str().unwrap();
        let val = &v["v"];
        match v["t"].as_str().unwrap() {
            "int" => base.set(id, Value::Int(val.as_i64().unwrap() as i32)),
            "double" => base.set(id, Value::Double(num(val))),
            "bool" => base.set(id, Value::Bool(val.as_bool().unwrap())),
            "string" => base.set(id, Value::from(val.as_str().unwrap())),
            "proxy" => proxies.push((id.to_string(), val.as_str().unwrap().to_string())),
            t => panic!("{t}"),
        }
    }
    let mut frame = base.clone();
    let base = Arc::new(base);
    for (id, src) in proxies {
        let mut b2 = (*base).clone();
        let prog = Program::from_str(&src, |r| b2.slot(&r.id())).expect("proxy compiles");
        frame.set_proxy(&id, Arc::new(prog), Arc::new(b2));
    }
    frame
}

#[derive(Default, Debug)]
struct Stats {
    cases: usize,
    parse_ok: usize,
    evals: usize,
    skipped_dist: usize,
    failures: Vec<String>,
    /// Value mismatches of expressions that sample a named distribution (simoxide-random).
    dist_mismatch: Vec<String>,
}

const DIST_FUNCS: [&str; 9] = [
    "Norm(",
    "Exp(",
    "Pois(",
    "UniDouble(",
    "UniInt(",
    "Lognorm(",
    "LognormMoments(",
    "Gamma(",
    "GammaMoments(",
];

fn check_case(line: &str, varsets: &HashMap<String, Vec<J>>, st: &mut Stats) {
    let g: J = serde_json::from_str(line).unwrap();
    let id = g["id"].as_str().unwrap().to_string();
    let expr = g["expr"].as_str().unwrap();
    st.cases += 1;
    let fail = |st: &mut Stats, msg: String| st.failures.push(format!("{id} [{expr}]: {msg}"));
    let parsed = simoxide_stoex::parse(expr);
    if g.get("parse_error").is_some() {
        if let Ok(e) = parsed {
            fail(
                st,
                format!(
                    "Java rejects ({}) but Rust parses {}",
                    g["parse_error"],
                    sexpr(&e)
                ),
            );
        }
        return;
    }
    let e = match parsed {
        Ok(e) => e,
        Err(err) => {
            fail(st, format!("Rust parse error {err}"));
            return;
        }
    };
    st.parse_ok += 1;
    if sexpr(&e) != g["tree"].as_str().unwrap() {
        fail(st, format!("tree {} != {}", sexpr(&e), g["tree"]));
        return;
    }
    // round trip through the canonical printer
    let printed = e.to_stoex();
    match simoxide_stoex::parse(&printed) {
        Ok(e2) if sexpr(&e2) == sexpr(&e) => {}
        other => fail(st, format!("print round trip: {printed} -> {other:?}")),
    }
    let mut ids: Vec<String> = e.variables().iter().map(|v| v.id()).collect();
    let mut jids: Vec<String> = g["var_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap().to_string())
        .collect();
    ids.sort();
    jids.sort();
    if ids != jids {
        fail(st, format!("var ids {ids:?} != {jids:?}"));
    }
    let prepared = simoxide_stoex::prepare(expr);
    if let Some(pe) = g.get("prepare_error") {
        match prepared {
            Ok(_) => fail(st, format!("Java prepare error {pe} but Rust prepared")),
            Err(re) => {
                let jc = pe["class"].as_str().unwrap();
                let ok = match &re {
                    simoxide_stoex::PrepareError::UnknownFunction { .. } => {
                        jc == "java.lang.UnsupportedOperationException"
                    }
                    simoxide_stoex::PrepareError::InvalidProbFunction { .. } => {
                        jc.starts_with("de.uka.ipd.sdq.probfunction.math.exception.")
                            || jc == "java.lang.RuntimeException"
                    }
                    simoxide_stoex::PrepareError::Parse(_) => false,
                };
                if !ok {
                    fail(st, format!("prepare error {re:?} vs Java {jc}"));
                }
            }
        }
        return;
    }
    let prepared = match prepared {
        Ok(p) => p,
        Err(err) => {
            fail(st, format!("Rust prepare error {err}"));
            return;
        }
    };
    let rt = prepared.root_type().map(|t| t.name());
    let jt = g["type"].as_str();
    if rt != jt {
        fail(st, format!("type {rt:?} != {jt:?}"));
    }
    let vars: Vec<J> = match g.get("varset") {
        Some(name) => varsets[name.as_str().unwrap()].clone(),
        None => g["vars"].as_array().cloned().unwrap_or_default(),
    };
    let mode = match g.get("mode").and_then(|m| m.as_str()) {
        Some("DEFAULT") => VariableMode::ReturnDefaultOnNotFound,
        Some("NULL") => VariableMode::ReturnNullOnNotFound,
        _ => VariableMode::ExceptionOnNotFound,
    };
    let mut env = build_env(&vars);
    let slots: HashMap<String, u32> = prepared
        .expr
        .variables()
        .iter()
        .map(|v| (v.id(), env.slot(&v.id())))
        .collect();
    let env = env;
    let program = Program::compile(&prepared, |v| slots[&v.id()]);
    let evals = g["evals"].as_array().unwrap();
    let tape: Vec<f64> = evals
        .iter()
        .flat_map(|ev| {
            ev["draws"]
                .as_array()
                .unwrap()
                .iter()
                .map(|d| hex(d.as_str().unwrap()))
        })
        .collect();
    for which in 0..2 {
        let mut rng = Tape {
            t: tape.clone(),
            pos: 0,
            overrun: 0,
        };
        for (k, ev) in evals.iter().enumerate() {
            let before = rng.pos;
            let got = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if which == 0 {
                    program.eval_mode(&env, &mut rng, mode)
                } else {
                    let mut res = |v: &VarRef| slots[&v.id()];
                    simoxide_stoex::interp::eval(&prepared, &mut res, &env, &mut rng, mode)
                }
            }));
            let got = match got {
                Ok(g) => g,
                Err(_) => {
                    // simoxide-random distribution not implemented yet (todo!)
                    st.skipped_dist += 1;
                    return;
                }
            };
            st.evals += 1;
            let what = if which == 0 { "program" } else { "interp" };
            let drawn = rng.pos - before + rng.overrun;
            let jdraws = ev["draws"].as_array().unwrap().len();
            if drawn != jdraws {
                fail(
                    st,
                    format!("{what} eval {k}: {drawn} draws, Java {jdraws} ({got:?})"),
                );
                return;
            }
            match (ev.get("result"), &got) {
                (Some(jr), Ok(v)) => {
                    let jv = java_value(jr);
                    if jv != *v {
                        let msg = format!("{id} [{expr}] {what} eval {k}: {v:?} != Java {jv:?}");
                        if DIST_FUNCS.iter().any(|f| expr.contains(f)) {
                            st.dist_mismatch.push(msg);
                        } else {
                            st.failures.push(msg);
                        }
                    }
                }
                (None, Err(e)) => {
                    let jc = ev["error"]["class"].as_str().unwrap();
                    if !same_error(jc, e) {
                        fail(
                            st,
                            format!(
                                "{what} eval {k}: error {e} vs Java {jc}: {}",
                                ev["error"]["message"]
                            ),
                        );
                    }
                }
                (Some(jr), Err(e)) => fail(st, format!("{what} eval {k}: error {e}, Java {jr}")),
                (None, Ok(v)) => fail(
                    st,
                    format!("{what} eval {k}: {v:?}, Java error {}", ev["error"]),
                ),
            }
        }
    }
}

/// `STOEX_GOLDEN_DIR=<dir>` checks another oracle output (`varsets.json` +
/// `stoex_golden.jsonl`), e.g. a large random run made with `gen_cases.py --only-generated`.
#[test]
fn golden_stoex() {
    let dir = std::env::var("STOEX_GOLDEN_DIR")
        .map(|d| format!("{d}/"))
        .unwrap_or_else(|_| GOLDEN.to_string());
    let varsets: HashMap<String, Vec<J>> = serde_json::from_str::<HashMap<String, J>>(
        &std::fs::read_to_string(format!("{dir}varsets.json")).unwrap(),
    )
    .unwrap()
    .into_iter()
    .map(|(k, v)| (k, v.as_array().unwrap().clone()))
    .collect();
    let text = std::fs::read_to_string(format!("{dir}stoex_golden.jsonl")).unwrap();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut st = Stats::default();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        check_case(line, &varsets, &mut st);
    }
    std::panic::set_hook(prev);
    eprintln!(
        "golden: {} cases, {} parsed, {} evaluations compared, {} cases skipped (simoxide-random todo), {} failures",
        st.cases,
        st.parse_ok,
        st.evals,
        st.skipped_dist,
        st.failures.len()
    );
    for f in st.failures.iter().take(60) {
        eprintln!("  {f}");
    }
    eprintln!(
        "{} value mismatches in distribution samples (simoxide-random inverse CDFs):",
        st.dist_mismatch.len()
    );
    for f in st.dist_mismatch.iter().take(200) {
        eprintln!("  {f}");
    }
    assert!(
        st.failures.is_empty(),
        "{} golden mismatches",
        st.failures.len()
    );
    // Strict by default since the oracle runs on the OSGi-ordered classpath (Commons Math 2.1,
    // docs/reference-simulator/patches.md "Classpath"); `STOEX_LAX_DIST=1` only reports sample mismatches.
    if !std::env::var("STOEX_LAX_DIST").is_ok_and(|v| v == "1") {
        assert!(
            st.dist_mismatch.is_empty(),
            "{} distribution sample mismatches",
            st.dist_mismatch.len()
        );
        assert_eq!(st.skipped_dist, 0, "evaluations panicked (skipped)");
    }
}

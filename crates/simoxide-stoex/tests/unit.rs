//! Focused tests of documented behaviours (the golden files cover the bulk).

use simoxide_stoex::{EvalErrorKind, Program, SimpleEnv, Value, VariableMode};
use std::sync::Arc;

fn eval_with(
    src: &str,
    env: &mut SimpleEnv,
    tape: &[f64],
) -> (Result<Value, simoxide_stoex::EvalError>, usize) {
    let p = Program::from_str(src, |v| env.slot(&v.id())).unwrap();
    let mut rng = simoxide_random::Recorder::new(simoxide_random::source::Cycle::new(tape));
    let r = p.eval(env, &mut rng);
    (r, rng.count())
}

fn eval(src: &str) -> Result<Value, simoxide_stoex::EvalError> {
    eval_with(src, &mut SimpleEnv::new(), &[0.5]).0
}

#[test]
fn parse_errors_have_positions() {
    let e = simoxide_stoex::parse("1 +\n  * 2").unwrap_err();
    assert_eq!((e.line, e.column), (2, 3), "{e}");
    let e = simoxide_stoex::parse("true ? 1 : 2 ? 3 : 4").unwrap_err();
    assert!(e.message.contains("end of input"), "{e}");
    assert!(
        simoxide_stoex::parse("   ")
            .unwrap_err()
            .message
            .contains("empty")
    );
    assert!(simoxide_stoex::parse("2147483648").is_err());
    assert!(simoxide_stoex::parse("IntPMF[(-2147483648;1)]").is_ok());
    assert!(simoxide_stoex::parse("IntPMF[(- 5;1)]").is_err());
    assert!(simoxide_stoex::parse("'a' == 'b'").is_ok()); // one string literal
    assert!(simoxide_stoex::parse("EnumPMF[('a';1)]").is_err());
}

#[test]
fn precedence_quirks() {
    // AND binds weaker than OR: (true OR false) AND false
    assert_eq!(eval("true OR false AND false").unwrap(), Value::Bool(false));
    // unary minus binds tighter than ^
    assert_eq!(eval("-2^2").unwrap(), Value::Double(4.0));
    assert!(simoxide_stoex::parse("2^3^2").is_err());
    assert!(simoxide_stoex::parse("1 < 2 < 3").is_err());
}

#[test]
fn variable_ids_are_normalised() {
    let e = simoxide_stoex::parse("a . INNER .  BYTESIZE + b.VALUE").unwrap();
    let ids: Vec<String> = e.variables().iter().map(|v| v.id()).collect();
    assert_eq!(ids, ["a.INNER.BYTESIZE", "b.VALUE"]);
    assert!(e.variables()[0].is_inner());
}

#[test]
fn static_types_choose_the_arithmetic() {
    let mut env = SimpleEnv::new();
    env.set("x.VALUE", Value::Int(7));
    env.set("d.BYTESIZE", Value::Double(2.5));
    assert_eq!(
        eval_with("x.VALUE / 2", &mut env, &[0.5]).0.unwrap(),
        Value::Int(3)
    );
    let err = eval_with("d.BYTESIZE + 1", &mut env, &[0.5]).0.unwrap_err();
    assert_eq!(err.kind(), EvalErrorKind::TypesIncompatibleInTerm);
    assert_eq!(
        eval("Log(2, 8) + 1").unwrap_err().kind(),
        EvalErrorKind::TypesIncompatibleInTerm
    );
    assert_eq!(eval("2^3").unwrap(), Value::Double(8.0));
    assert_eq!(
        eval("2^3 + 1").unwrap_err().kind(),
        EvalErrorKind::TypesIncompatibleInTerm
    );
    assert_eq!(
        eval("(5.5 % 2) + 1").unwrap_err().kind(),
        EvalErrorKind::TypesIncompatibleInTerm
    );
    assert_eq!(eval("2147483647 + 1").unwrap(), Value::Int(i32::MIN));
    assert_eq!(eval("1 / 0").unwrap_err().kind(), EvalErrorKind::Arithmetic);
    assert_eq!(eval("Round(3e9)").unwrap(), Value::Int(-1294967296)); // (int) of a long wraps
    assert_eq!(eval("Ceil(3e9)").unwrap(), Value::Int(i32::MAX)); // (int) of a double saturates
    assert_eq!(
        eval("Min(1, 2.5)").unwrap_err().kind(),
        EvalErrorKind::FunctionParametersNotAccepted
    );
    assert_eq!(
        eval("Binom(10, 0.5)").unwrap_err().kind(),
        EvalErrorKind::FunctionUnknown
    );
    assert!(simoxide_stoex::prepare("Foo(1)").is_err());
}

#[test]
fn draws_follow_evaluation_order() {
    let mut env = SimpleEnv::new();
    // both operands of AND are evaluated
    let (r, n) = eval_with(
        "IntPMF[(1;1.0)] == 2 AND IntPMF[(1;1.0)] == 1",
        &mut env,
        &[0.5],
    );
    assert_eq!((r.unwrap(), n), (Value::Bool(false), 2));
    // only the taken branch of ?:
    let (r, n) = eval_with(
        "true ? IntPMF[(1;1.0)] : DoublePMF[(2;1.0)]",
        &mut env,
        &[0.5],
    );
    assert_eq!((r.unwrap(), n), (Value::Int(1), 1));
    // arguments are evaluated before the unknown function fails
    let (r, n) = eval_with("Binom(IntPMF[(1;1.0)], 0.5)", &mut env, &[0.5]);
    assert!(r.is_err());
    assert_eq!(n, 1);
}

#[test]
fn pmf_sampling() {
    let mut env = SimpleEnv::new();
    let pmf = "IntPMF[(2;0.5)(1;0.5)]"; // sorted by value: 1, 2
    assert_eq!(eval_with(pmf, &mut env, &[0.49]).0.unwrap(), Value::Int(1));
    assert_eq!(eval_with(pmf, &mut env, &[0.5]).0.unwrap(), Value::Int(2));
    // sum 0.9999999999 is within 1e-9 of 1: not adjusted, so u above it falls through to 0.0
    let r = eval_with(
        "IntPMF[(1;0.5)(2;0.4999999999)]",
        &mut env,
        &[0.99999999995],
    );
    assert_eq!(r.0.unwrap(), Value::Double(0.0));
    // adjusted: 0.3 + 0.3 -> +0.2 each
    assert_eq!(
        eval_with("IntPMF[(1;0.3)(2;0.3)]", &mut env, &[0.49])
            .0
            .unwrap(),
        Value::Int(1)
    );
    // boxed PDF: linear interpolation from (0, 0)
    assert_eq!(
        eval_with("DoublePDF[(1;0.5)(2;0.5)]", &mut env, &[0.25])
            .0
            .unwrap(),
        Value::Double(0.5)
    );
    assert!(simoxide_stoex::prepare("DoublePDF[(0;0.5)(1;0.5)]").is_err());
    assert!(simoxide_stoex::prepare("false ? IntPMF[(1;2.0)] : 1").is_ok()); // adjusted to 1.0
    assert!(simoxide_stoex::prepare("false ? IntPMF[(1;0.0)] : 1").is_err());
}

#[test]
fn modes_and_conversions() {
    let env = SimpleEnv::new();
    let mut e2 = env.clone();
    let p = Program::from_str("m.BYTESIZE * 2", |v| e2.slot(&v.id())).unwrap();
    let mut rng = simoxide_random::MersenneTwister::from_int(1);
    assert!(p.eval(&e2, &mut rng).is_err());
    assert_eq!(
        p.eval_mode(&e2, &mut rng, VariableMode::ReturnDefaultOnNotFound)
            .unwrap(),
        Value::Int(0)
    );
    let p = Program::from_str("m.VALUE", |v| e2.slot(&v.id())).unwrap();
    assert_eq!(
        p.eval_mode(&e2, &mut rng, VariableMode::ReturnNullOnNotFound)
            .unwrap(),
        Value::Null
    );
    assert_eq!(Value::Int(3).to_f64().unwrap(), 3.0);
    assert!(Value::Double(3.0).to_i32().is_err());
}

#[test]
fn inner_proxies_are_resampled() {
    let base = SimpleEnv::new();
    let mut b2 = base.clone();
    let proxy = Program::from_str("IntPMF[(1;0.5)(2;0.5)]", |v| b2.slot(&v.id())).unwrap();
    let mut env = base.clone();
    env.set_proxy("a.INNER.VALUE", Arc::new(proxy), Arc::new(b2));
    let (r, n) = eval_with("a.INNER.VALUE * 10 + a.INNER.VALUE", &mut env, &[0.1, 0.9]);
    assert_eq!((r.unwrap(), n), (Value::Int(12), 2));
}

#[test]
fn folding() {
    let p = Program::from_str("(1 + 2) * 3.0", |_| 0).unwrap();
    assert_eq!(p.constant(), Some(&Value::Double(9.0)));
    let p = Program::from_str("1 / 0", |_| 0).unwrap();
    assert!(p.constant().is_none());
    let mut rng = simoxide_random::MersenneTwister::from_int(1);
    assert_eq!(
        p.eval(&simoxide_stoex::NoVars, &mut rng)
            .unwrap_err()
            .kind(),
        EvalErrorKind::Arithmetic
    );
}

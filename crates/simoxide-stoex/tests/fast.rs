//! StoEx evaluation with the fast mode's source (`simoxide_random::fast::FastSource`): the
//! distribution functions keep their result types (`Pois`/`UniInt` give `Int`, the others
//! `Double`), integer arithmetic on them, and the reference's parameter errors.

use simoxide_random::fast::FastSource;
use simoxide_random::{MersenneTwister, UniformSource};
use simoxide_stoex::{EvalErrorKind, Program, SimpleEnv, Value};

fn eval(src: &str, rng: &mut impl UniformSource) -> Result<Value, EvalErrorKind> {
    let mut env = SimpleEnv::new();
    let p = Program::from_str(src, |v| env.slot(&v.id())).unwrap();
    p.eval(&env, rng).map_err(|e| e.kind())
}

fn kind(v: &Result<Value, EvalErrorKind>) -> String {
    match v {
        Ok(Value::Int(_)) => "Int".into(),
        Ok(Value::Double(_)) => "Double".into(),
        Ok(other) => format!("{other:?}"),
        Err(e) => format!("error {e:?}"),
    }
}

#[test]
fn distribution_functions_keep_types_and_errors() {
    let exprs = [
        "Exp(2.0)",
        "Norm(1.0, 0.5)",
        "Lognorm(0.0, 1.0)",
        "LognormMoments(2.0, 1.0)",
        "Gamma(2.0, 0.5)",
        "GammaMoments(2.0, 0.5)",
        "Pois(3.0)",
        "Pois(3)",
        "Pois(3) + 1",
        "Pois(3) / 2",
        "UniDouble(1.0, 2.0)",
        "UniInt(1, 6)",
        "UniInt(1, 6) * 2",
        "UniInt(1.0, 6)",
        "Exp(0.0)",
        "Exp(-1.0)",
        "Norm(0.0, 0.0)",
        "Lognorm(0.0, 0.0)",
        "Gamma(0.0, 1.0)",
        "GammaMoments(1.0, 0.0)",
        "Pois(0.0)",
        "Pois(-1.0)",
        "UniDouble(2.0, 1.0)",
        "UniDouble(5.0, 5.0)",
        "UniInt(3, 2)",
        "Norm(1.0)",
        "Exp(1.0) + Pois(2.0) * UniInt(1, 3)",
    ];
    for e in exprs {
        let mut mt = MersenneTwister::from_seed(&[1, 2, 3, 4, 5, 6]).unwrap();
        let mut fast = FastSource::new(1);
        for _ in 0..50 {
            let (a, b) = (eval(e, &mut mt), eval(e, &mut fast));
            assert_eq!(kind(&a), kind(&b), "{e}: exact {a:?}, fast {b:?}");
        }
    }
}

#[test]
fn integer_samples_stay_in_range() {
    let mut fast = FastSource::new(2);
    for _ in 0..10_000 {
        match eval("UniInt(-2, 2)", &mut fast) {
            Ok(Value::Int(v)) => assert!((-2..=2).contains(&v)),
            other => panic!("{other:?}"),
        }
        match eval("Pois(0.5)", &mut fast) {
            Ok(Value::Int(v)) => assert!(v >= -1),
            other => panic!("{other:?}"),
        }
    }
}

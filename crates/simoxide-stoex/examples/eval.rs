//! Evaluates StoEx expressions given on the command line (variables: `id=value`, value an
//! integer, a double with '.', `true`/`false`, or `"string"`). Uniforms come from a fixed
//! Mersenne Twister stream.
//!
//! `cargo run -p simoxide-stoex --example eval -- 'n.VALUE * 2' n.VALUE=3`
fn main() {
    let mut env = simoxide_stoex::SimpleEnv::new();
    let mut exprs = Vec::new();
    for a in std::env::args().skip(1) {
        match a.split_once('=') {
            Some((id, v)) if id.contains('.') && !id.contains(' ') => {
                let val = if let Ok(i) = v.parse::<i32>() {
                    simoxide_stoex::Value::Int(i)
                } else if let Ok(d) = v.parse::<f64>() {
                    simoxide_stoex::Value::Double(d)
                } else if v == "true" || v == "false" {
                    simoxide_stoex::Value::Bool(v == "true")
                } else {
                    simoxide_stoex::Value::from(v.trim_matches('"'))
                };
                env.set(id, val);
            }
            _ => exprs.push(a),
        }
    }
    let mut rng = simoxide_random::MersenneTwister::from_int(42);
    for e in exprs {
        match simoxide_stoex::Program::from_str(&e, |v| env.slot(&v.id())) {
            Err(err) => println!("{e}: {err}"),
            Ok(p) => match p.eval(&env, &mut rng) {
                Ok(simoxide_stoex::Value::Double(d)) => {
                    println!("{e} = Double {d:?} ({:016x})", d.to_bits())
                }
                Ok(v) => println!("{e} = {v:?}"),
                Err(err) => println!("{e}: {err}"),
            },
        }
    }
}

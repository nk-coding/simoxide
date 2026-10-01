//! Throughput of compiled StoEx evaluation (run with `--release`).
//!
//! `cargo run -p simoxide-stoex --release --example bench`
use simoxide_stoex::{Program, SimpleEnv, Value};
use std::hint::black_box;
use std::time::Instant;

fn main() {
    let mut env = SimpleEnv::new();
    env.set("file.BYTESIZE", Value::Int(4096));
    env.set("n.VALUE", Value::Int(12));
    env.set("x.VALUE", Value::Double(3.25));
    let cases = [
        "0.04",
        "x.VALUE",
        "file.BYTESIZE",
        "x.VALUE",
        "x.VALUE * 2.0",
        "0.00511 * ( file.BYTESIZE )",
        "n.VALUE * 2 + 1",
        "x.VALUE * 1.5 + n.VALUE / 3",
        "n.VALUE > 10 ? x.VALUE * 2 : 1.0",
        "IntPMF[(1;0.3)(2;0.3)(3;0.4)]",
        "DoublePDF[(1.0;0.022)(2.0;0.047)(3.0;0.104)(4.0;0.161)(5.0;0.204)(6.0;0.188)(7.0;0.114)(8.0;0.069)(9.0;0.029)(10.0;0.017)(11.0;0.009)(12.0;0.009)(13.0;0.004)(14.0;0.005)(15.0;0.002)(16.0;0.002)(17.0;0.018)]",
        "Exp(2.0)",
        "Trunc(file.BYTESIZE / 1024) * 0.5 + DoublePMF[(0.1;0.5)(0.2;0.5)]",
    ];
    let mut rng = simoxide_random::MersenneTwister::from_int(1);
    for src in cases {
        let p = Program::from_str(src, |v| env.slot(&v.id())).unwrap();
        let n = 5_000_000u32;
        let mut acc = 0.0;
        let t = Instant::now();
        for _ in 0..n {
            acc += p.eval_f64(black_box(&env), &mut rng).unwrap();
        }
        let ns = t.elapsed().as_nanos() as f64 / f64::from(n);
        println!(
            "{ns:8.1} ns/eval  nodes={:3}  {src:.60}  (sum {acc:.3e})",
            p.node_count()
        );
    }
}

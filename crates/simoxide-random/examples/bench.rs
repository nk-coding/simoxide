//! Rough micro-benchmark of the samplers: `cargo run --release -p simoxide-random --example bench`.

use simoxide_random::dist::{self, BracketSearch};
use simoxide_random::probfn::PmfSampler;
use simoxide_random::{MersenneTwister, UniformSource};
use std::hint::black_box;
fn bench(name: &str, n: usize, mut f: impl FnMut() -> f64) {
    let t = std::time::Instant::now();
    let mut s = 0.0;
    for _ in 0..n {
        s += f();
    }
    black_box(s);
    println!(
        "{name:40} {:8.1} ns/op",
        t.elapsed().as_nanos() as f64 / n as f64
    );
}
fn main() {
    let mut r = MersenneTwister::from_seed(&[1, 2, 3, 4, 5, 6]).unwrap();
    bench("mt next_double", 10_000_000, || r.next_uniform());
    bench("jmath::log", 10_000_000, || {
        simoxide_random::jmath::log(black_box(1.2345))
    });
    bench("f64::ln", 10_000_000, || black_box(1.2345f64).ln());
    bench("jmath::exp", 10_000_000, || {
        simoxide_random::jmath::exp(black_box(1.2345))
    });
    bench("Exp(2)", 5_000_000, || {
        dist::sample_exp(2.0, &mut r).unwrap()
    });
    for s in [BracketSearch::Linear, BracketSearch::Galloping] {
        bench(&format!("Norm(0,1) {s:?}"), 200_000, || {
            dist::sample_norm_with(0.0, 1.0, s, &mut r).unwrap()
        });
        bench(&format!("Norm(100,30) {s:?}"), 50_000, || {
            dist::sample_norm_with(100.0, 30.0, s, &mut r).unwrap()
        });
        bench(&format!("Gamma(2,0.5) {s:?}"), 200_000, || {
            dist::sample_gamma_with(2.0, 0.5, s, &mut r).unwrap()
        });
        bench(&format!("Gamma(2,100) {s:?}"), 5_000, || {
            dist::sample_gamma_with(2.0, 100.0, s, &mut r).unwrap()
        });
        bench(&format!("Lognorm(1,0.5) {s:?}"), 200_000, || {
            dist::sample_lognorm_with(1.0, 0.5, s, &mut r).unwrap()
        });
        bench(&format!("Lognorm(5,1) {s:?}"), 5_000, || {
            dist::sample_lognorm_with(5.0, 1.0, s, &mut r).unwrap()
        });
        bench(&format!("UniDouble(0,10) {s:?}"), 500_000, || {
            dist::sample_unidouble_with(0.0, 10.0, s, &mut r).unwrap()
        });
    }
    // the scales of the corpus and generator models (brackets of one step)
    bench("Norm(0.01,0.003)", 200_000, || {
        dist::sample_norm(0.01, 0.003, &mut r).unwrap()
    });
    bench("Lognorm(-4.5,0.4)", 200_000, || {
        dist::sample_lognorm(-4.5, 0.4, &mut r).unwrap()
    });
    bench("Gamma(2,0.005)", 200_000, || {
        dist::sample_gamma(2.0, 0.005, &mut r).unwrap()
    });
    bench("LognormMoments(0.01,0.005)", 200_000, || {
        dist::sample_lognorm_moments(0.01, 0.005, &mut r).unwrap()
    });
    bench("Pois(2)", 200_000, || {
        dist::sample_pois(2.0, &mut r).unwrap() as f64
    });
    bench("Pois(4)", 200_000, || {
        dist::sample_pois(4.0, &mut r).unwrap() as f64
    });
    bench("UniInt(1,6)", 1_000_000, || {
        dist::sample_uniint(1, 6, &mut r).unwrap() as f64
    });
    let p3 = PmfSampler::new(&[0.2, 0.3, 0.5]);
    bench("PMF 3", 10_000_000, || p3.sample(&mut r).unwrap() as f64);
    let p100 = PmfSampler::new(&vec![0.01; 100]);
    bench("PMF 100", 10_000_000, || {
        p100.sample(&mut r).unwrap_or(0) as f64
    });
}

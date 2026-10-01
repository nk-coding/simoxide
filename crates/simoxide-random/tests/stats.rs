//! Statistical sanity tests (Kolmogorov–Smirnov) for every sampler, independent of the Java
//! oracle. Fixed seeds, so the tests are deterministic. Critical value: alpha = 0.001.

use simoxide_random::dist::{self, Exponential, Gamma, LogNormal, Normal, Poisson, UniformDouble};
use simoxide_random::probfn::{BoxedPdfSampler, PmfSampler};
use simoxide_random::special::{self, IntegerDistribution};
use simoxide_random::{MersenneTwister, UniformSource};

const N: usize = 4000;

fn rng(seed: i64) -> MersenneTwister {
    MersenneTwister::from_seed(&[seed, 17, 23, 5, 99, 1]).unwrap()
}

fn ks_critical(n: usize) -> f64 {
    1.95 / (n as f64).sqrt()
}

/// Two-sided KS statistic of a sample against a continuous CDF.
fn ks(mut xs: Vec<f64>, cdf: impl Fn(f64) -> f64) -> f64 {
    xs.sort_by(f64::total_cmp);
    let n = xs.len() as f64;
    xs.iter()
        .enumerate()
        .map(|(i, &x)| {
            let f = cdf(x);
            (f - i as f64 / n).abs().max(((i + 1) as f64 / n - f).abs())
        })
        .fold(0.0, f64::max)
}

/// Max deviation between empirical and model CDF of an integer sample.
fn ks_discrete(xs: &[i32], cdf: impl Fn(i32) -> f64) -> f64 {
    let (lo, hi) = (*xs.iter().min().unwrap(), *xs.iter().max().unwrap());
    let n = xs.len() as f64;
    let mut d: f64 = 0.0;
    for k in lo..=hi {
        let emp = xs.iter().filter(|&&x| x <= k).count() as f64 / n;
        d = d.max((emp - cdf(k)).abs());
    }
    d
}

fn assert_ks(name: &str, d: f64, n: usize) {
    let c = ks_critical(n);
    assert!(d < c, "{name}: KS statistic {d:.5} >= critical {c:.5}");
}

#[test]
fn uniform_stream_is_uniform() {
    let mut r = rng(1);
    let xs: Vec<f64> = (0..N).map(|_| r.next_uniform()).collect();
    assert!(xs.iter().all(|&u| (0.0..1.0).contains(&u)));
    assert_ks("uniform", ks(xs, |x| x), N);
}

#[test]
fn exponential() {
    for (seed, rate) in [(2, 1.0), (3, 0.01), (4, 250.0)] {
        let d = Exponential::new(rate).unwrap();
        let mut r = rng(seed);
        let xs: Vec<f64> = (0..N).map(|_| d.sample(&mut r).unwrap()).collect();
        assert_ks(
            &format!("Exp({rate})"),
            ks(xs, |x| 1.0 - (-rate * x).exp()),
            N,
        );
    }
}

#[test]
fn normal() {
    for (seed, m, s) in [(5, 0.0, 1.0), (6, 10.0, 2.0), (7, -3.0, 0.01)] {
        let d = Normal::new(m, s).unwrap();
        let mut r = rng(seed);
        let xs: Vec<f64> = (0..N).map(|_| d.sample(&mut r).unwrap()).collect();
        assert_ks(&format!("Norm({m},{s})"), ks(xs, |x| d.cdf(x).unwrap()), N);
    }
}

#[test]
fn lognormal() {
    for (seed, m, s) in [(8, 0.0, 1.0), (9, 1.0, 0.25)] {
        let d = LogNormal::new(m, s).unwrap();
        let mut r = rng(seed);
        let xs: Vec<f64> = (0..N).map(|_| d.sample(&mut r).unwrap()).collect();
        assert_ks(
            &format!("Lognorm({m},{s})"),
            ks(xs, |x| d.cdf(x).unwrap()),
            N,
        );
    }
    // From moments: mean and stdev of the sample.
    let d = LogNormal::from_moments(3.0, 0.5 * 0.5).unwrap();
    let mut r = rng(10);
    let xs: Vec<f64> = (0..N).map(|_| d.sample(&mut r).unwrap()).collect();
    let mean = xs.iter().sum::<f64>() / N as f64;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (N - 1) as f64;
    assert!((mean - 3.0).abs() < 0.05, "LognormMoments mean {mean}");
    assert!(
        (var.sqrt() - 0.5).abs() < 0.05,
        "LognormMoments sd {}",
        var.sqrt()
    );
    assert_ks("LognormMoments(3,0.5)", ks(xs, |x| d.cdf(x).unwrap()), N);
}

#[test]
fn gamma() {
    for (seed, a, t) in [
        (11, 1.0, 1.0),
        (12, 2.5, 0.4),
        (13, 0.5, 2.0),
        (14, 9.0, 0.3),
    ] {
        let d = Gamma::new(a, t).unwrap();
        let mut r = rng(seed);
        let xs: Vec<f64> = (0..N).map(|_| d.sample(&mut r).unwrap()).collect();
        assert_ks(&format!("Gamma({a},{t})"), ks(xs, |x| d.cdf(x).unwrap()), N);
    }
    // Gamma(1, theta) is Exp(1/theta).
    let d = Gamma::new(1.0, 2.0).unwrap();
    let mut r = rng(15);
    let xs: Vec<f64> = (0..N).map(|_| d.sample(&mut r).unwrap()).collect();
    assert_ks("Gamma(1,2) vs Exp", ks(xs, |x| 1.0 - (-x / 2.0).exp()), N);
    // From moments.
    let d = Gamma::from_moments(4.0, 0.5).unwrap();
    let (a, b) = d.params();
    assert!((a * b - 4.0).abs() < 1e-12 && (a.sqrt().recip() - 0.5).abs() < 1e-12);
}

#[test]
fn uniform_double() {
    let d = UniformDouble::new(-3.0, 7.0).unwrap();
    let mut r = rng(16);
    let xs: Vec<f64> = (0..N).map(|_| d.sample(&mut r).unwrap()).collect();
    assert!(xs.iter().all(|&x| (-3.0..=7.0).contains(&x)));
    assert_ks("UniDouble(-3,7)", ks(xs, |x| (x + 3.0) / 10.0), N);
}

#[test]
fn uniform_int() {
    let mut r = rng(17);
    let xs: Vec<i32> = (0..N)
        .map(|_| dist::sample_uniint(1, 6, &mut r).unwrap())
        .collect();
    assert!(xs.iter().all(|&x| (1..=6).contains(&x)));
    assert_ks(
        "UniInt(1,6)",
        ks_discrete(&xs, |k| (k as f64 / 6.0).clamp(0.0, 1.0)),
        N,
    );
}

/// Commons Math 2.1 `AbstractIntegerDistribution.inverseCumulativeProbability` returns the
/// largest `x` with `F(x) <= u`, so SimuCom's `Pois(m)` is a Poisson variate **minus one**
/// (it returns -1 with probability `exp(-m)`). The reference does this; so do we.
#[test]
fn poisson_is_shifted_by_minus_one() {
    for (seed, m) in [(18, 0.5), (19, 4.0), (20, 60.0)] {
        let d = Poisson::new(m).unwrap();
        let mut r = rng(seed);
        let xs: Vec<i32> = (0..N).map(|_| d.sample(&mut r).unwrap()).collect();
        let cdf = |k: i32| d.cdf_int(k + 1).unwrap();
        assert_ks(&format!("Pois({m}) - 1"), ks_discrete(&xs, cdf), N);
        let mean = xs.iter().map(|&x| x as f64).sum::<f64>() / N as f64;
        let sd = (m / N as f64).sqrt();
        assert!((mean - (m - 1.0)).abs() < 4.0 * sd, "Pois({m}) mean {mean}");
    }
}

#[test]
fn pmf_sampler() {
    let probs = [0.1, 0.0, 0.25, 0.4, 0.25];
    let s = PmfSampler::new(&probs);
    let mut r = rng(21);
    let mut counts = [0usize; 5];
    for _ in 0..N {
        counts[s.sample(&mut r).unwrap()] += 1;
    }
    assert_eq!(counts[1], 0);
    let xs: Vec<i32> = counts
        .iter()
        .enumerate()
        .flat_map(|(i, &c)| std::iter::repeat_n(i as i32, c))
        .collect();
    let cdf = |k: i32| probs[..=(k as usize)].iter().sum::<f64>();
    assert_ks("PMF", ks_discrete(&xs, cdf), N);
}

#[test]
fn boxed_pdf_sampler() {
    let s = BoxedPdfSampler::new(&[(4.0, 0.5), (1.0, 0.25), (2.0, 0.25)]).unwrap();
    // CDF: linear through (0,0), (1,.25), (2,.5), (4,1).
    let cdf = |x: f64| {
        if x <= 1.0 {
            0.25 * x
        } else if x <= 2.0 {
            0.25 + 0.25 * (x - 1.0)
        } else {
            0.5 + 0.25 * (x - 2.0)
        }
    };
    let mut r = rng(22);
    let xs: Vec<f64> = (0..N).map(|_| s.sample(&mut r).unwrap()).collect();
    assert_ks("BoxedPDF", ks(xs, cdf), N);
}

#[test]
fn inversion_is_monotone_in_u() {
    // Inversion samplers must be non-decreasing in u (sanity for the bracket search variants).
    let us: Vec<f64> = (1..400).map(|i| i as f64 / 400.0).collect();
    type Inv = Box<dyn Fn(f64, special::BracketSearch) -> f64>;
    let checks: [(&str, Inv); 3] = [
        (
            "Norm",
            Box::new(|u, s| {
                Normal::new(5.0, 3.0)
                    .unwrap()
                    .inverse_cdf_with(u, s)
                    .unwrap()
            }),
        ),
        (
            "Gamma",
            Box::new(|u, s| {
                Gamma::new(2.0, 3.0)
                    .unwrap()
                    .inverse_cdf_with(u, s)
                    .unwrap()
            }),
        ),
        (
            "Lognorm",
            Box::new(|u, s| {
                LogNormal::new(1.0, 1.0)
                    .unwrap()
                    .inverse_cdf_with(u, s)
                    .unwrap()
            }),
        ),
    ];
    for (name, f) in &checks {
        let mut prev = f64::NEG_INFINITY;
        for &u in &us {
            let x = f(u, special::BracketSearch::Linear);
            assert_eq!(
                x.to_bits(),
                f(u, special::BracketSearch::Galloping).to_bits(),
                "{name}({u})"
            );
            // Brent stops within 1e-9 of the root, so allow that much non-monotonicity.
            assert!(x >= prev - 2e-9, "{name}: F^-1({u}) = {x} < {prev}");
            prev = x;
        }
    }
}

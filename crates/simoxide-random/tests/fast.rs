//! The fast mode's generator and samplers (`simoxide_random::fast`, feature `fast`) against the
//! distributions they sample: chi-square tests over 10^6 samples with bin probabilities from the
//! reference CDFs, first moments, generator quality checks, and parameter/error behaviour equal
//! to the reference functions (every parameter set of the Java oracle's golden file).
#![cfg(feature = "fast")]

use simoxide_random::dist::{self, DistError, Gamma, LogNormal, Normal, Poisson};
use simoxide_random::fast::FastSource;
use simoxide_random::special::{self, IntegerDistribution};
use simoxide_random::{MersenneTwister, UniformSource};
use std::path::Path;

const N: usize = 1_000_000;
/// Significance level of each test (fixed seeds: the tests are deterministic).
const ALPHA: f64 = 1e-4;

fn chi2_sf(x: f64, dof: f64) -> f64 {
    special::regularized_gamma_q(dof / 2.0, x / 2.0, 1e-14, 1_000_000).unwrap()
}

/// Chi-square goodness of fit: `observed` counts against `expected` probabilities; cells with
/// an expected count below 20 are merged with their neighbour. Returns (statistic, dof, p).
fn chi2(observed: &[f64], probs: &[f64]) -> (f64, f64, f64) {
    let n: f64 = observed.iter().sum();
    let (mut cells, mut o, mut e) = (Vec::new(), 0.0, 0.0);
    for (&ob, &p) in observed.iter().zip(probs) {
        o += ob;
        e += p * n;
        if e >= 20.0 {
            cells.push((o, e));
            o = 0.0;
            e = 0.0;
        }
    }
    if let Some(last) = cells.last_mut() {
        last.0 += o;
        last.1 += e;
    }
    let stat: f64 = cells.iter().map(|(o, e)| (o - e) * (o - e) / e).sum();
    let dof = (cells.len() - 1) as f64;
    (stat, dof, chi2_sf(stat, dof))
}

/// Continuous sample against a CDF, cells between consecutive `edges` plus both tails.
fn chi2_continuous(xs: &[f64], cdf: impl Fn(f64) -> f64, edges: &[f64]) -> (f64, f64, f64) {
    let mut edges = edges.to_vec();
    edges.sort_by(f64::total_cmp);
    edges.dedup();
    let mut counts = vec![0.0; edges.len() + 1];
    for &x in xs {
        let i = edges.partition_point(|&e| e < x);
        counts[i] += 1.0;
    }
    let f: Vec<f64> = edges.iter().map(|&e| cdf(e)).collect();
    let mut probs = Vec::with_capacity(counts.len());
    probs.push(f[0]);
    for w in f.windows(2) {
        probs.push((w[1] - w[0]).max(0.0));
    }
    probs.push(1.0 - f[f.len() - 1]);
    chi2(&counts, &probs)
}

fn linspace(a: f64, b: f64, n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| a + (b - a) * i as f64 / (n - 1) as f64)
        .collect()
}

/// Mean and variance within 6 standard errors of the model's.
fn check_moments(name: &str, xs: &[f64], mean: f64, var: f64) {
    let n = xs.len() as f64;
    let m = xs.iter().sum::<f64>() / n;
    let v = xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (n - 1.0);
    let m4 = xs.iter().map(|x| (x - m).powi(4)).sum::<f64>() / n;
    assert!(
        (m - mean).abs() <= 6.0 * (var / n).sqrt(),
        "{name}: mean {m} vs {mean}"
    );
    let se_var = ((m4 - v * v).max(0.0) / n).sqrt();
    assert!(
        (v - var).abs() <= 6.0 * se_var + 1e-12 * var,
        "{name}: variance {v} vs {var} (se {se_var})"
    );
}

fn assert_fit(name: &str, r: (f64, f64, f64)) {
    eprintln!("{name}: chi2 {:.1} with {} dof, p = {:.3}", r.0, r.1, r.2);
    assert!(r.2 > ALPHA, "{name}: chi2 {} ({} dof), p {}", r.0, r.1, r.2);
}

fn samples(seed: u64, mut f: impl FnMut(&mut FastSource) -> f64) -> Vec<f64> {
    let mut s = FastSource::new(seed);
    (0..N).map(|_| f(&mut s)).collect()
}

// ------------------------------------------------------------------------------ generator

#[test]
fn uniforms_are_uniform_and_independent() {
    let mut s = FastSource::new(1);
    let us: Vec<f64> = (0..N).map(|_| s.next_uniform()).collect();
    assert!(us.iter().all(|&u| (0.0..1.0).contains(&u)));
    // 1000 equal cells
    let mut c = vec![0.0; 1000];
    for &u in &us {
        c[(u * 1000.0) as usize] += 1.0;
    }
    assert_fit("uniform 1000 cells", chi2(&c, &[1e-3; 1000]));
    // consecutive pairs in a 32 x 32 grid
    let mut c2 = vec![0.0; 1024];
    for w in us.as_chunks::<2>().0 {
        c2[(w[0] * 32.0) as usize * 32 + (w[1] * 32.0) as usize] += 1.0;
    }
    assert_fit("uniform pairs 32x32", chi2(&c2, &[1.0 / 1024.0; 1024]));
    // lag-1 serial correlation
    let r: f64 = us
        .windows(2)
        .map(|w| (w[0] - 0.5) * (w[1] - 0.5))
        .sum::<f64>()
        / ((N - 1) as f64 / 12.0);
    assert!(r.abs() < 5.0 / (N as f64).sqrt(), "lag-1 correlation {r}");
    check_moments("uniform", &us, 0.5, 1.0 / 12.0);
}

#[test]
fn every_output_bit_is_balanced() {
    let mut s = FastSource::new(7);
    let mut ones = [0u64; 64];
    for _ in 0..N {
        let x = s.next_u64();
        for (b, o) in ones.iter_mut().enumerate() {
            *o += (x >> b) & 1;
        }
    }
    let sd = (N as f64 / 4.0).sqrt();
    for (b, &o) in ones.iter().enumerate() {
        assert!(
            (o as f64 - N as f64 / 2.0).abs() < 5.0 * sd,
            "bit {b}: {o} ones"
        );
    }
}

#[test]
fn seeds_give_distinct_uncorrelated_streams() {
    let firsts: std::collections::HashSet<u64> =
        (0..10_000).map(|s| FastSource::new(s).next_u64()).collect();
    assert_eq!(firsts.len(), 10_000);
    let (mut a, mut b) = (FastSource::new(1000), FastSource::new(1001));
    let r: f64 = (0..N)
        .map(|_| (a.next_uniform() - 0.5) * (b.next_uniform() - 0.5))
        .sum::<f64>()
        / (N as f64 / 12.0);
    assert!(r.abs() < 5.0 / (N as f64).sqrt(), "correlation {r}");
}

// ------------------------------------------------------------------------------ samplers

#[test]
fn normal() {
    for (seed, m, sd) in [(1, 0.0, 1.0), (2, 10.0, 2.0), (3, -3.0, 0.01)] {
        let d = Normal::new(m, sd).unwrap();
        let xs = samples(seed, |s| s.sample_norm(m, sd).unwrap());
        let edges: Vec<f64> = linspace(-4.5, 4.5, 181)
            .into_iter()
            .map(|z| m + sd * z)
            .collect();
        assert_fit(
            &format!("Norm({m},{sd})"),
            chi2_continuous(&xs, |x| d.cdf(x).unwrap(), &edges),
        );
        check_moments(&format!("Norm({m},{sd})"), &xs, m, sd * sd);
    }
    // the tail beyond the ziggurat's base strip (|z| > 3.65) has the right mass
    let xs = samples(4, |s| s.std_normal());
    let tail = xs
        .iter()
        .filter(|x| x.abs() > 3.654_152_885_361_009)
        .count() as f64;
    let std = Normal::new(0.0, 1.0).unwrap();
    let expect = N as f64 * 2.0 * std.cdf(-3.654_152_885_361_009).unwrap();
    assert!(
        (tail - expect).abs() < 6.0 * expect.sqrt(),
        "tail {tail} vs {expect}"
    );
}

#[test]
fn exponential() {
    for (seed, rate) in [(5, 1.0), (6, 0.01), (7, 250.0)] {
        let xs = samples(seed, |s| s.sample_exp(rate).unwrap());
        assert!(xs.iter().all(|&x| x >= 0.0));
        let edges: Vec<f64> = linspace(0.0, 14.0, 281).iter().map(|t| t / rate).collect();
        assert_fit(
            &format!("Exp({rate})"),
            chi2_continuous(&xs, |x| 1.0 - (-rate * x).exp(), &edges),
        );
        check_moments(
            &format!("Exp({rate})"),
            &xs,
            1.0 / rate,
            1.0 / (rate * rate),
        );
    }
}

#[test]
fn gamma() {
    for (seed, a, t) in [
        (8, 0.3, 2.0),
        (9, 0.999, 1.0),
        (10, 1.0, 1.0),
        (11, 2.5, 0.4),
        (12, 9.0, 0.3),
        (13, 150.0, 0.01),
    ] {
        let d = Gamma::new(a, t).unwrap();
        let xs = samples(seed, |s| s.sample_gamma(a, t).unwrap());
        assert!(xs.iter().all(|&x| x >= 0.0));
        let hi = a + 12.0 * a.sqrt() + 12.0;
        let mut edges: Vec<f64> = linspace(0.0, hi, 300).iter().map(|x| x * t).collect();
        edges.extend((0..80).map(|i| t * 10f64.powf(-8.0 + i as f64 / 10.0)));
        assert_fit(
            &format!("Gamma({a},{t})"),
            chi2_continuous(&xs, |x| d.cdf(x).unwrap(), &edges),
        );
        check_moments(&format!("Gamma({a},{t})"), &xs, a * t, a * t * t);
    }
    // GammaMoments(mean, cv): alpha = 1/cv^2, theta = mean cv^2
    let xs = samples(14, |s| s.sample_gamma_moments(4.0, 0.5).unwrap());
    check_moments("GammaMoments(4,0.5)", &xs, 4.0, 4.0);
}

#[test]
fn lognormal() {
    for (seed, mu, s) in [(15, 0.0, 1.0), (16, 1.0, 0.25), (17, -4.5, 0.4)] {
        let d = LogNormal::new(mu, s).unwrap();
        let xs = samples(seed, |r| r.sample_lognorm(mu, s).unwrap());
        let edges: Vec<f64> = linspace(-4.5, 4.5, 181)
            .iter()
            .map(|z| (mu + s * z).exp())
            .collect();
        assert_fit(
            &format!("Lognorm({mu},{s})"),
            chi2_continuous(&xs, |x| d.cdf(x).unwrap(), &edges),
        );
        let m = (mu + s * s / 2.0).exp();
        check_moments(
            &format!("Lognorm({mu},{s})"),
            &xs,
            m,
            ((s * s).exp() - 1.0) * m * m,
        );
    }
    // LognormMoments(mean, stdev) has that mean and standard deviation
    let xs = samples(18, |r| r.sample_lognorm_moments(3.0, 0.5).unwrap());
    check_moments("LognormMoments(3,0.5)", &xs, 3.0, 0.25);
}

#[test]
fn unidouble() {
    let xs = samples(19, |s| s.sample_unidouble(-3.0, 7.0).unwrap());
    assert!(xs.iter().all(|&x| (-3.0..=7.0).contains(&x)));
    assert_fit(
        "UniDouble(-3,7)",
        chi2_continuous(
            &xs,
            |x| ((x + 3.0) / 10.0).clamp(0.0, 1.0),
            &linspace(-3.0, 7.0, 501),
        ),
    );
    check_moments("UniDouble(-3,7)", &xs, 2.0, 100.0 / 12.0);
    // tiny intervals, where the reference's Brent result (accuracy 1e-6) is not uniform
    let xs = samples(20, |s| s.sample_unidouble(1e-9, 2e-9).unwrap());
    assert!(xs.iter().all(|&x| (1e-9..=2e-9).contains(&x)));
    check_moments("UniDouble(1e-9,2e-9)", &xs, 1.5e-9, 1e-18 / 12.0);
}

/// Integer sample against a probability mass function on `lo..=hi` (tails merged).
fn chi2_int(xs: &[i32], lo: i32, hi: i32, pmf: impl Fn(i32) -> f64) -> (f64, f64, f64) {
    let mut c = vec![0.0; (hi - lo + 1) as usize];
    for &x in xs {
        assert!((lo..=hi).contains(&x), "{x} outside {lo}..={hi}");
        c[(x - lo) as usize] += 1.0;
    }
    let p: Vec<f64> = (lo..=hi).map(pmf).collect();
    chi2(&c, &p)
}

#[test]
fn uniint() {
    for (seed, a, b) in [(21, 1, 6), (22, -10, 10), (23, 0, 999), (24, 5, 5)] {
        let mut s = FastSource::new(seed);
        let xs: Vec<i32> = (0..N).map(|_| s.sample_uniint(a, b).unwrap()).collect();
        let n = f64::from(b - a + 1);
        if a == b {
            assert!(xs.iter().all(|&x| x == a));
            continue;
        }
        assert_fit(
            &format!("UniInt({a},{b})"),
            chi2_int(&xs, a, b, |_| 1.0 / n),
        );
    }
    // the full non-overflowing range is reachable at both ends
    let mut s = FastSource::new(25);
    let (lo, hi) = (-1_073_741_824, 1_073_741_822);
    assert!((0..1000).all(|_| (lo..=hi).contains(&s.sample_uniint(lo, hi).unwrap())));
}

#[test]
fn poisson_is_shifted_by_minus_one_like_the_reference() {
    for (seed, m) in [
        (26, 0.3),
        (27, 4.0),
        (28, 9.99),
        (29, 10.0),
        (30, 30.0),
        (31, 1000.0),
    ] {
        let d = Poisson::new(m).unwrap();
        let mut s = FastSource::new(seed);
        let xs: Vec<i32> = (0..N).map(|_| s.sample_pois(m).unwrap()).collect();
        // P(result = k) = P(X = k + 1) for X ~ Poisson(m): the reference returns X - 1 (REF-4)
        let cdf = |k: i32| d.cdf_int(k).unwrap();
        let (lo, hi) = (-1, (m + 12.0 * m.sqrt() + 20.0) as i32);
        assert_fit(
            &format!("Pois({m})"),
            chi2_int(&xs, lo, hi, |k| cdf(k + 1) - cdf(k)),
        );
        let ys: Vec<f64> = xs.iter().map(|&x| f64::from(x)).collect();
        check_moments(&format!("Pois({m})"), &ys, m - 1.0, m);
    }
}

// ------------------------------------------------------------------------------ parameters

#[derive(Debug, Clone, PartialEq)]
enum Class {
    Ok,
    NotAccepted,
    Invalid(String),
    Numerical(String),
}

fn class<T>(r: &Result<T, DistError>) -> Class {
    match r {
        Ok(_) => Class::Ok,
        Err(DistError::ParametersNotAccepted { .. }) => Class::NotAccepted,
        Err(DistError::InvalidParameter { dist, message, .. }) => {
            Class::Invalid(format!("{dist}: {message}"))
        }
        Err(DistError::Numerical { dist, .. }) => Class::Numerical(dist.to_string()),
    }
}

fn call(name: &str, p: &[f64], s: &mut impl UniformSource) -> Class {
    match name {
        "Exp" => class(&s.sample_exp(p[0])),
        "Norm" => class(&s.sample_norm(p[0], p[1])),
        "Lognorm" => class(&s.sample_lognorm(p[0], p[1])),
        "LognormMoments" => class(&s.sample_lognorm_moments(p[0], p[1])),
        "Gamma" => class(&s.sample_gamma(p[0], p[1])),
        "GammaMoments" => class(&s.sample_gamma_moments(p[0], p[1])),
        "Pois" => class(&s.sample_pois(p[0])),
        "UniDouble" => class(&s.sample_unidouble(p[0], p[1])),
        "UniInt" => class(&s.sample_uniint(p[0] as i32, p[1] as i32)),
        _ => panic!("unknown function {name}"),
    }
}

/// Every parameter set of the Java oracle (valid, invalid, degenerate, overflowing): the fast
/// sampler fails exactly when the reference fails, with the same error, and consumes no
/// uniform on a parameter or construction error.
#[test]
fn errors_equal_the_reference_for_every_oracle_parameter_set() {
    let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/oracles/random/golden/dists.txt");
    let text = std::fs::read_to_string(golden).unwrap();
    let mut cases = 0;
    for h in text.lines().filter(|l| l.starts_with("case ")) {
        let f: Vec<&str> = h.split(' ').collect();
        let name = f[2];
        let params: Vec<f64> = f[3..]
            .iter()
            .map(|p| match p.split_once(':').unwrap() {
                ("i", v) => v.parse::<i32>().unwrap() as f64,
                ("d", v) => f64::from_bits(u64::from_str_radix(v, 16).unwrap()),
                _ => panic!("{p}"),
            })
            .collect();
        let mut mt = MersenneTwister::from_seed(&[1, 2, 3, 4, 5, 6]).unwrap();
        let mut fast = FastSource::new(9);
        let mut want: Vec<Class> = (0..200).map(|_| call(name, &params, &mut mt)).collect();
        let mut got: Vec<Class> = (0..200).map(|_| call(name, &params, &mut fast)).collect();
        want.dedup();
        got.dedup();
        assert_eq!(got, want, "{h}");
        if matches!(want[..], [Class::NotAccepted] | [Class::Invalid(_)]) {
            let mut a = FastSource::new(3);
            let mut b = a.clone();
            call(name, &params, &mut a);
            assert_eq!(a.next_u64(), b.next_u64(), "{h}: a uniform was consumed");
        }
        cases += 1;
    }
    assert!(cases > 100, "{cases} cases");
}

#[test]
fn integer_results_stay_integers() {
    let mut s = FastSource::new(40);
    for _ in 0..10_000 {
        assert!(s.sample_pois(0.1).unwrap() >= -1);
        let v = s.sample_uniint(-3, 3).unwrap();
        assert!((-3..=3).contains(&v));
    }
    // Pois(small) is mostly -1, as in the reference
    let minus_one = (0..N)
        .filter(|_| s.sample_pois(1e-6).unwrap() == -1)
        .count();
    assert!(minus_one > N - 20);
    // the reference's degenerate cases still fail
    assert!(matches!(
        s.sample_unidouble(5.0, 5.0),
        Err(DistError::Numerical { .. })
    ));
    assert!(matches!(
        s.sample_pois(0.0),
        Err(DistError::InvalidParameter { .. })
    ));
    assert!(matches!(
        dist::sample_uniint(3, 2, &mut s),
        Err(DistError::InvalidParameter { .. })
    ));
}

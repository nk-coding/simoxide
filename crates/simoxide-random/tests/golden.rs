//! Golden-file tests against the Java oracle (`reference/oracles/random`), which calls the real
//! SimuLizar 5.2.2 / Commons Math 2.1 classes. The committed files are small; run
//! `SCALE=big OUT=<dir> reference/oracles/random/gen-golden.sh` and
//! `SIMOXIDE_RANDOM_GOLDEN=<dir> cargo test -p simoxide-random --release -- --ignored` for large dumps.

use std::path::{Path, PathBuf};

use simoxide_random::dist::{self, BracketSearch};
use simoxide_random::probfn::{self, BoxedPdfSampler, PmfSampler};
use simoxide_random::source::Cycle;
use simoxide_random::{MersenneTwister, Recorder, UniformSource};
use simoxide_random::{crmath, jmath};

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference/oracles/random/golden")
}

fn big_dir() -> Option<PathBuf> {
    std::env::var_os("SIMOXIDE_RANDOM_GOLDEN").map(PathBuf::from)
}

fn read(dir: &Path, name: &str) -> String {
    let p = dir.join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

fn hexf(s: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(s, 16).unwrap_or_else(|_| panic!("bad hex {s}")))
}

// ---------------------------------------------------------------------------------------------

fn check_uniforms(text: &str) {
    let mut lines = text.lines();
    let header = lines.next().unwrap();
    let seed: Vec<i64> = header
        .split("seed=")
        .nth(1)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .split(',')
        .map(|s| s.parse().unwrap())
        .collect();
    let mut mt = MersenneTwister::from_seed(&seed).unwrap();
    let mut n = 0;
    for (i, l) in lines.enumerate() {
        let u = mt.next_uniform();
        assert_eq!(
            u.to_bits(),
            hexf(l).to_bits(),
            "uniform #{i} (seed {seed:?})"
        );
        n += 1;
    }
    assert!(n > 0);
}

#[test]
fn uniforms_match_simucom_stream() {
    check_uniforms(&read(&golden_dir(), "uniforms.txt"));
    check_uniforms(&read(&golden_dir(), "uniforms-b.txt"));
}

#[test]
#[ignore]
fn uniforms_match_simucom_stream_big() {
    let d = big_dir().expect("set SIMOXIDE_RANDOM_GOLDEN");
    check_uniforms(&read(&d, "uniforms.txt"));
}

// ---------------------------------------------------------------------------------------------

/// Checks `Math.log` / `Math.exp` (`jmath`, HotSpot's with the `hotspot-math` feature, which the
/// tests enable) and the correctly rounded `crmath` versions, which must agree with Java except
/// for one-ulp differences. Returns (count, mismatches vs Rust std, vs StrictMath, vs crmath) for
/// the statistics printed by the test.
fn check_mathfns(text: &str) -> [(usize, usize, usize, usize); 2] {
    let mut stats = [(0, 0, 0, 0); 2];
    let mut bad = Vec::new();
    for l in text.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = l.split(' ').collect();
        let (x, java, strict) = (hexf(f[1]), hexf(f[2]), hexf(f[3]));
        let (ours, std, cr, k) = match f[0] {
            "log" => (jmath::log(x), x.ln(), crmath::log(x), 0),
            "exp" => (jmath::exp(x), x.exp(), crmath::exp(x), 1),
            _ => panic!("bad line {l}"),
        };
        let same = |a: f64, b: f64| a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan());
        stats[k].0 += 1;
        if !same(std, java) {
            stats[k].1 += 1;
        }
        if !same(strict, java) {
            stats[k].2 += 1;
        }
        if !same(cr, java) {
            stats[k].3 += 1;
            let ulps = (cr.to_bits() as i64 - java.to_bits() as i64).abs();
            if (ulps != 1 || !java.is_finite() || !cr.is_finite()) && bad.len() < 20 {
                bad.push(format!(
                    "crmath::{}({x:e} = {}) = {cr:e}, Java {java:e}",
                    f[0], f[1]
                ));
            }
        }
        if !same(ours, java) && bad.len() < 20 {
            bad.push(format!(
                "{}({x:e} = {}) = {ours:e}, Java {java:e}",
                f[0], f[1]
            ));
        }
    }
    assert!(bad.is_empty(), "mismatches:\n{}", bad.join("\n"));
    stats
}

fn print_mathfns_stats(s: [(usize, usize, usize, usize); 2]) {
    for (name, s) in ["log", "exp"].iter().zip(s) {
        eprintln!(
            "{name}: n={} differs from Rust std: {}, from StrictMath: {}, from crmath (1 ulp): {}",
            s.0, s.1, s.2, s.3
        );
    }
}

#[test]
fn math_log_exp_match_hotspot_intrinsics() {
    print_mathfns_stats(check_mathfns(&read(&golden_dir(), "mathfns.txt")));
}

#[test]
#[ignore]
fn math_log_exp_match_hotspot_intrinsics_big() {
    let d = big_dir().expect("set SIMOXIDE_RANDOM_GOLDEN");
    print_mathfns_stats(check_mathfns(&read(&d, "mathfns.txt")));
}

// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Param {
    I(i32),
    D(f64),
}

impl Param {
    fn d(self) -> f64 {
        match self {
            Param::I(i) => i as f64,
            Param::D(d) => d,
        }
    }
    fn i(self) -> i32 {
        match self {
            Param::I(i) => i,
            Param::D(_) => panic!("expected int"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    D(u64),
    I(i32),
    E(String),
}

fn java_outcome(s: &str) -> Outcome {
    let (k, v) = s.split_once(':').unwrap();
    match k {
        "D" => Outcome::D(u64::from_str_radix(v, 16).unwrap()),
        "I" => Outcome::I(v.parse().unwrap()),
        "E" => Outcome::E(v.to_string()),
        _ => panic!("bad outcome {s}"),
    }
}

fn run_fn(name: &str, p: &[Param], search: BracketSearch, rng: &mut impl UniformSource) -> Outcome {
    let r: Result<Outcome, dist::DistError> = match name {
        "Exp" => dist::sample_exp(p[0].d(), rng).map(|x| Outcome::D(x.to_bits())),
        "Norm" => {
            dist::sample_norm_with(p[0].d(), p[1].d(), search, rng).map(|x| Outcome::D(x.to_bits()))
        }
        "Lognorm" => dist::sample_lognorm_with(p[0].d(), p[1].d(), search, rng)
            .map(|x| Outcome::D(x.to_bits())),
        "LognormMoments" => dist::sample_lognorm_moments_with(p[0].d(), p[1].d(), search, rng)
            .map(|x| Outcome::D(x.to_bits())),
        "Gamma" => dist::sample_gamma_with(p[0].d(), p[1].d(), search, rng)
            .map(|x| Outcome::D(x.to_bits())),
        "GammaMoments" => dist::sample_gamma_moments_with(p[0].d(), p[1].d(), search, rng)
            .map(|x| Outcome::D(x.to_bits())),
        "Pois" => dist::sample_pois(p[0].d(), rng).map(Outcome::I),
        "UniDouble" => dist::sample_unidouble_with(p[0].d(), p[1].d(), search, rng)
            .map(|x| Outcome::D(x.to_bits())),
        "UniInt" => dist::sample_uniint(p[0].i(), p[1].i(), rng).map(Outcome::I),
        _ => panic!("unknown function {name}"),
    };
    r.unwrap_or_else(|e| Outcome::E(format!("{e:?}")))
}

/// Error kinds may be named differently; only "is an error" and "uniform consumed" must match.
fn same_outcome(ours: &Outcome, java: &Outcome) -> bool {
    match (ours, java) {
        (Outcome::E(_), Outcome::E(_)) => true,
        (Outcome::D(a), Outcome::D(b)) => {
            a == b || (f64::from_bits(*a).is_nan() && f64::from_bits(*b).is_nan())
        }
        _ => ours == java,
    }
}

struct DistStats {
    calls: usize,
    exact: usize,
    max_ulp: u64,
    failures: Vec<String>,
}

fn ulps(a: u64, b: u64) -> u64 {
    (a as i64).abs_diff(b as i64)
}

fn check_dists(text: &str, search: BracketSearch) -> DistStats {
    let mut st = DistStats {
        calls: 0,
        exact: 0,
        max_ulp: 0,
        failures: vec![],
    };
    let mut lines = text.lines().filter(|l| !l.starts_with('#')).peekable();
    while let Some(h) = lines.next() {
        let f: Vec<&str> = h.split(' ').collect();
        assert_eq!(f[0], "case", "{h}");
        let seeded = f[1]
            .strip_prefix("seed=")
            .map(|s| s.parse::<i64>().unwrap());
        let name = f[2];
        let params: Vec<Param> = f[3..]
            .iter()
            .map(|p| match p.split_once(':').unwrap() {
                ("i", v) => Param::I(v.parse().unwrap()),
                ("d", v) => Param::D(hexf(v)),
                _ => panic!("{p}"),
            })
            .collect();
        // Our own stream for seeded cases (must reproduce the recorded uniforms too).
        let mut mt = seeded
            .map(|s| MersenneTwister::from_seed(&[s, s + 1, s + 2, s + 3, s + 4, s + 5]).unwrap());
        while let Some(l) = lines.peek() {
            if l.starts_with("case ") {
                break;
            }
            let l = lines.next().unwrap();
            let (us, res) = l.split_once(' ').unwrap();
            let us: Vec<f64> = if us == "-" {
                vec![]
            } else {
                us.split(',').map(hexf).collect()
            };
            let java = java_outcome(res);
            // Replay the recorded uniforms (works for fixed and seeded cases).
            let tape = if us.is_empty() { vec![0.5] } else { us.clone() };
            let mut rec = Recorder::new(Cycle::new(&tape));
            let ours = run_fn(name, &params, search, &mut rec);
            st.calls += 1;
            let consumed = rec.count();
            let ok = same_outcome(&ours, &java) && consumed == us.len();
            if ok {
                st.exact += 1;
            } else {
                if let (Outcome::D(a), Outcome::D(b)) = (&ours, &java) {
                    st.max_ulp = st.max_ulp.max(ulps(*a, *b));
                }
                if st.failures.len() < 30 {
                    st.failures.push(format!(
                        "{name}{params:?} u={us:?}: ours {ours:?} (consumed {consumed}), Java {java:?}"
                    ));
                }
            }
            if let Some(mt) = mt.as_mut() {
                for &u in &us {
                    assert_eq!(
                        mt.next_uniform().to_bits(),
                        u.to_bits(),
                        "{name}{params:?} stream"
                    );
                }
            }
        }
    }
    st
}

#[test]
fn distributions_match_simucom_functions() {
    let st = check_dists(&read(&golden_dir(), "dists.txt"), BracketSearch::Linear);
    eprintln!("dists: {} calls, {} exact", st.calls, st.exact);
    assert!(
        st.failures.is_empty(),
        "max ulp {}:\n{}",
        st.max_ulp,
        st.failures.join("\n")
    );
}

#[test]
fn distributions_match_simucom_functions_galloping() {
    let st = check_dists(&read(&golden_dir(), "dists.txt"), BracketSearch::Galloping);
    eprintln!("dists: {} calls, {} exact", st.calls, st.exact);
    assert!(
        st.failures.is_empty(),
        "max ulp {}:\n{}",
        st.max_ulp,
        st.failures.join("\n")
    );
}

#[test]
#[ignore]
fn distributions_match_simucom_functions_big() {
    let d = big_dir().expect("set SIMOXIDE_RANDOM_GOLDEN");
    for search in [BracketSearch::Galloping, BracketSearch::Linear] {
        let st = check_dists(&read(&d, "dists.txt"), search);
        eprintln!("dists ({search:?}): {} calls, {} exact", st.calls, st.exact);
        assert!(
            st.failures.is_empty(),
            "max ulp {}:\n{}",
            st.max_ulp,
            st.failures.join("\n")
        );
    }
}

// ---------------------------------------------------------------------------------------------

fn check_probfn(text: &str) -> usize {
    let mut n = 0;
    let mut lines = text.lines().filter(|l| !l.starts_with('#')).peekable();
    while let Some(h) = lines.next() {
        let f: Vec<&str> = h.split(' ').collect();
        assert_eq!(f[0], "case");
        let kind = f[1];
        let seed: i64 = f[2].strip_prefix("seed=").unwrap().parse().unwrap();
        let slash = f.iter().position(|&x| x == "/").unwrap();
        let values: Vec<f64> = f[3..slash].iter().map(|s| hexf(s)).collect();
        let mut probs: Vec<f64> = f[slash + 1..].iter().map(|s| hexf(s)).collect();
        probfn::adjust_probabilities(&mut probs);
        let mut mt =
            MersenneTwister::from_seed(&[seed, seed + 1, seed + 2, seed + 3, seed + 4, seed + 5])
                .unwrap();
        // Build the sampler as the reference does.
        enum S {
            Pmf(PmfSampler, Vec<f64>),
            Pdf(BoxedPdfSampler),
            Err,
        }
        let sampler = if kind == "pmf" {
            let order = probfn::sort_order(&values, |a, b| probfn::java_double_compare(*a, *b));
            let sp: Vec<f64> = order.iter().map(|&i| probs[i]).collect();
            let sv: Vec<f64> = order.iter().map(|&i| values[i]).collect();
            match probfn::validate_pmf(&sp) {
                Ok(()) => S::Pmf(PmfSampler::new(&sp), sv),
                Err(_) => S::Err,
            }
        } else {
            let pairs: Vec<(f64, f64)> =
                values.iter().copied().zip(probs.iter().copied()).collect();
            match BoxedPdfSampler::new(&pairs) {
                Ok(s) => S::Pdf(s),
                Err(_) => S::Err,
            }
        };
        while let Some(l) = lines.peek() {
            if l.starts_with("case ") {
                break;
            }
            let l = lines.next().unwrap();
            let (us, res) = l.split_once(' ').unwrap();
            let java = java_outcome(res);
            if us == "-" {
                assert!(
                    matches!(sampler, S::Err),
                    "{h}: Java failed to build: {res}"
                );
                assert!(matches!(java, Outcome::E(_)));
                continue;
            }
            let u = hexf(us);
            assert_eq!(mt.next_uniform().to_bits(), u.to_bits(), "{h}: stream");
            let ours = match &sampler {
                S::Pmf(s, sv) => match s.sample_index(u) {
                    Some(i) => Outcome::D(sv[i].to_bits()),
                    None => Outcome::D(0f64.to_bits()), // Java returns Double 0.0
                },
                S::Pdf(s) => match s.inverse_cdf(u) {
                    Ok(x) => Outcome::D(x.to_bits()),
                    Err(e) => Outcome::E(format!("{e:?}")),
                },
                S::Err => panic!("{h}: we failed to build, Java did not"),
            };
            assert!(
                same_outcome(&ours, &java),
                "{h}: u={u}: ours {ours:?}, Java {java:?}"
            );
            n += 1;
        }
    }
    n
}

#[test]
fn probfn_literals_match_simucom() {
    let n = check_probfn(&read(&golden_dir(), "probfn.txt"));
    assert!(n > 0);
}

#[test]
#[ignore]
fn probfn_literals_match_simucom_big() {
    let d = big_dir().expect("set SIMOXIDE_RANDOM_GOLDEN");
    check_probfn(&read(&d, "probfn.txt"));
}

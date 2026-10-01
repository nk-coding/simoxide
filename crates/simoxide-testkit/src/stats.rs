//! Small statistics toolbox for the statistical comparisons: moments, quantiles, two-sample
//! Kolmogorov-Smirnov test, Student t quantiles, batch-means confidence intervals, time-weighted
//! means of step functions.

pub fn mean(x: &[f64]) -> f64 {
    if x.is_empty() {
        return f64::NAN;
    }
    x.iter().sum::<f64>() / x.len() as f64
}

/// Sample variance (n - 1).
pub fn variance(x: &[f64]) -> f64 {
    if x.len() < 2 {
        return f64::NAN;
    }
    let m = mean(x);
    x.iter().map(|v| (v - m) * (v - m)).sum::<f64>() / (x.len() - 1) as f64
}

/// Quantile with linear interpolation (R type 7 / numpy default) of an unsorted sample.
pub fn quantile(x: &[f64], p: f64) -> f64 {
    let mut s = x.to_vec();
    s.sort_by(f64::total_cmp);
    quantile_sorted(&s, p)
}

pub fn quantile_sorted(s: &[f64], p: f64) -> f64 {
    if s.is_empty() {
        return f64::NAN;
    }
    let h = (s.len() - 1) as f64 * p.clamp(0.0, 1.0);
    let lo = h.floor() as usize;
    let hi = h.ceil() as usize;
    s[lo] + (h - lo as f64) * (s[hi] - s[lo])
}

/// Two-sample Kolmogorov-Smirnov test: `(D, p-value)`. The p-value is the asymptotic Kolmogorov
/// distribution at `(sqrt(ne) + 0.12 + 0.11 / sqrt(ne)) * D` with `ne = nm / (n + m)` (Stephens'
/// small-sample correction, M. A. Stephens, JRSS B 32 (1970) 115-122).
pub fn ks_two_sample(a: &[f64], b: &[f64]) -> (f64, f64) {
    if a.is_empty() || b.is_empty() {
        return (f64::NAN, f64::NAN);
    }
    let mut x = a.to_vec();
    let mut y = b.to_vec();
    x.sort_by(f64::total_cmp);
    y.sort_by(f64::total_cmp);
    let (n, m) = (x.len() as f64, y.len() as f64);
    let (mut i, mut j) = (0usize, 0usize);
    let mut d: f64 = 0.0;
    while i < x.len() && j < y.len() {
        let v = x[i].min(y[j]);
        while i < x.len() && x[i] <= v {
            i += 1;
        }
        while j < y.len() && y[j] <= v {
            j += 1;
        }
        d = d.max((i as f64 / n - j as f64 / m).abs());
    }
    let en = (n * m / (n + m)).sqrt();
    (d, kolmogorov_q((en + 0.12 + 0.11 / en) * d))
}

/// Kolmogorov survival function `Q(lambda) = P(K > lambda) = 2 sum_k (-1)^(k-1) exp(-2 k^2 lambda^2)`.
/// The alternating series converges slowly for small `lambda`, so below 1 the equivalent theta
/// function form `1 - sqrt(2 pi) / lambda * sum_k exp(-(2k-1)^2 pi^2 / (8 lambda^2))` is used.
/// Both need at most a handful of terms for double precision.
pub fn kolmogorov_q(lambda: f64) -> f64 {
    if lambda.is_nan() {
        return f64::NAN;
    }
    if lambda <= 0.0 {
        return 1.0;
    }
    let q = if lambda < 1.0 {
        let c = -std::f64::consts::PI * std::f64::consts::PI / (8.0 * lambda * lambda);
        let mut sum = 0.0;
        for k in 0..10 {
            let odd = (2 * k + 1) as f64;
            let term = (c * odd * odd).exp();
            sum += term;
            if term <= f64::EPSILON * sum {
                break;
            }
        }
        1.0 - (2.0 * std::f64::consts::PI).sqrt() / lambda * sum
    } else {
        let c = -2.0 * lambda * lambda;
        let mut sum = 0.0;
        for k in 1..=10 {
            let kf = k as f64;
            let term = (c * kf * kf).exp();
            sum += if k % 2 == 1 { term } else { -term };
            if term <= f64::EPSILON * sum {
                break;
            }
        }
        2.0 * sum
    };
    q.clamp(0.0, 1.0)
}

/// ln Gamma (Lanczos, g = 7).
pub fn ln_gamma(x: f64) -> f64 {
    const G: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        return (std::f64::consts::PI / (std::f64::consts::PI * x).sin()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let mut a = G[0];
    let t = x + 7.5;
    for (i, g) in G.iter().enumerate().skip(1) {
        a += g / (x + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (x + 0.5) * t.ln() - t + a.ln()
}

/// Regularized incomplete beta function `I_x(a, b)`.
pub fn inc_beta(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    // The continued fraction converges fast for x < (a + 1) / (a + b + 2); otherwise use
    // I_x(a, b) = 1 - I_(1-x)(b, a).
    if x * (a + b + 2.0) < a + 1.0 {
        inc_beta_cf(a, b, x)
    } else {
        1.0 - inc_beta_cf(b, a, 1.0 - x)
    }
}

/// `I_x(a, b) = x^a (1-x)^b / (a B(a, b)) * 1 / (1 + d_1 / (1 + d_2 / (1 + ...)))` with
/// `d_(2m+1) = -(a+m)(a+b+m) x / ((a+2m)(a+2m+1))` and `d_(2m) = m(b-m) x / ((a+2m-1)(a+2m))`
/// (DLMF 8.17.22), the fraction evaluated forwards with the modified Lentz method
/// (Thompson & Barnett, J. Comput. Phys. 64 (1986) 490-509).
fn inc_beta_cf(a: f64, b: f64, x: f64) -> f64 {
    const TINY: f64 = 1e-300;
    let prefactor =
        (a * x.ln() + b * (1.0 - x).ln() + ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b)).exp() / a;
    // f = 1 / (1 + d_1 / (1 + d_2 / ...)): partial numerators 1, d_1, d_2, ..., denominators 1.
    let numerator = |j: u32| -> f64 {
        if j == 1 {
            return 1.0;
        }
        let n = j - 1;
        let m = f64::from(n / 2);
        if n % 2 == 1 {
            -(a + m) * (a + b + m) * x / ((a + 2.0 * m) * (a + 2.0 * m + 1.0))
        } else {
            m * (b - m) * x / ((a + 2.0 * m - 1.0) * (a + 2.0 * m))
        }
    };
    let nonzero = |v: f64| if v == 0.0 { TINY } else { v };
    let mut f = TINY;
    let (mut c, mut d) = (f, 0.0);
    for j in 1..1000 {
        let aj = numerator(j);
        d = 1.0 / nonzero(1.0 + aj * d);
        c = nonzero(1.0 + aj / c);
        let delta = c * d;
        f *= delta;
        if (delta - 1.0).abs() <= 1e-15 {
            break;
        }
    }
    prefactor * f
}

/// Student t CDF with `df` degrees of freedom.
pub fn t_cdf(t: f64, df: f64) -> f64 {
    let x = df / (df + t * t);
    let tail = 0.5 * inc_beta(df / 2.0, 0.5, x);
    if t >= 0.0 { 1.0 - tail } else { tail }
}

/// Student t quantile (bisection on [`t_cdf`]).
pub fn t_quantile(p: f64, df: f64) -> f64 {
    let (mut lo, mut hi) = (-1e3, 1e3);
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        if t_cdf(mid, df) < p {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Batch-means confidence interval `(mean, half width)` at level `conf` with `batches` equal batches
/// (tolerates autocorrelated simulation output). `None` if fewer than 2 observations per batch.
pub fn batch_means_ci(x: &[f64], batches: usize, conf: f64) -> Option<(f64, f64)> {
    let k = batches.max(2);
    let b = x.len() / k;
    if b < 2 {
        return None;
    }
    let means: Vec<f64> = (0..k).map(|i| mean(&x[i * b..(i + 1) * b])).collect();
    let m = mean(&means);
    let s = variance(&means).sqrt();
    let t = t_quantile(0.5 + conf / 2.0, (k - 1) as f64);
    Some((m, t * s / (k as f64).sqrt()))
}

/// Time-weighted mean of a step function given as `(time, value)` change points, up to `t_end` (the
/// last change point if `None`).
pub fn time_weighted_mean(times: &[f64], values: &[f64], t_end: Option<f64>) -> f64 {
    if times.is_empty() {
        return f64::NAN;
    }
    let end = t_end.unwrap_or(*times.last().unwrap());
    let start = times[0];
    if end <= start {
        return values[values.len() - 1];
    }
    let mut acc = 0.0;
    for i in 0..times.len() {
        let t1 = if i + 1 < times.len() {
            times[i + 1]
        } else {
            end
        };
        acc += values[i] * (t1.min(end) - times[i]).max(0.0);
    }
    acc / (end - start)
}

/// Complementary error function.
pub fn erfc(x: f64) -> f64 {
    libm::erfc(x)
}

/// Two-sided p-value of a standard normal statistic.
pub fn normal_two_sided_p(z: f64) -> f64 {
    if z.is_nan() {
        return f64::NAN;
    }
    erfc(z.abs() / std::f64::consts::SQRT_2).min(1.0)
}

/// Welch's unequal-variance t-test: two-sided p-value (`NaN` if a group has fewer than 2 values).
/// Two constant groups give 1 if equal and 0 otherwise.
pub fn welch_t_test(a: &[f64], b: &[f64]) -> f64 {
    if a.len() < 2 || b.len() < 2 {
        return f64::NAN;
    }
    let (ma, mb) = (mean(a), mean(b));
    let (va, vb) = (variance(a) / a.len() as f64, variance(b) / b.len() as f64);
    let se2 = va + vb;
    if se2 == 0.0 || !se2.is_finite() {
        return if ma == mb { 1.0 } else { 0.0 };
    }
    let t = (ma - mb) / se2.sqrt();
    let df = se2 * se2 / (va * va / (a.len() - 1) as f64 + vb * vb / (b.len() - 1) as f64);
    let tail = if t.abs() > 1e3 {
        0.0
    } else {
        0.5 * inc_beta(df / 2.0, 0.5, df / (df + t * t))
    };
    (2.0 * tail).min(1.0)
}

/// Mann-Whitney U test (normal approximation with tie correction and continuity correction):
/// two-sided p-value (`NaN` if a group is empty).
pub fn mann_whitney_u(a: &[f64], b: &[f64]) -> f64 {
    let (n1, n2) = (a.len(), b.len());
    if n1 == 0 || n2 == 0 {
        return f64::NAN;
    }
    let mut v: Vec<(f64, bool)> = a
        .iter()
        .map(|&x| (x, true))
        .chain(b.iter().map(|&x| (x, false)))
        .collect();
    v.sort_by(|x, y| x.0.total_cmp(&y.0));
    let n = v.len();
    let (mut r1, mut ties) = (0.0, 0.0);
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && v[j + 1].0 == v[i].0 {
            j += 1;
        }
        let rank = (i + j) as f64 / 2.0 + 1.0;
        let t = (j - i + 1) as f64;
        ties += t * t * t - t;
        for x in &v[i..=j] {
            if x.1 {
                r1 += rank;
            }
        }
        i = j + 1;
    }
    let (n1f, n2f, nf) = (n1 as f64, n2 as f64, n as f64);
    let u = r1 - n1f * (n1f + 1.0) / 2.0;
    let mu = n1f * n2f / 2.0;
    let var = n1f * n2f / 12.0 * ((nf + 1.0) - ties / (nf * (nf - 1.0)));
    if var <= 0.0 {
        return 1.0;
    }
    let d = (u - mu).abs() - 0.5;
    normal_two_sided_p(d.max(0.0) / var.sqrt())
}

/// Fisher's exact test of a 2x2 table `[[a, b], [c, d]]`: two-sided p-value (sum of the
/// probabilities of all tables with the same margins that are not more likely).
pub fn fisher_exact(a: u64, b: u64, c: u64, d: u64) -> f64 {
    let (r1, c1, n) = (a + b, a + c, a + b + c + d);
    let ln_fact = |k: u64| ln_gamma(k as f64 + 1.0);
    let ln_p = |x: u64| {
        ln_fact(r1) + ln_fact(n - r1) + ln_fact(c1) + ln_fact(n - c1)
            - ln_fact(n)
            - ln_fact(x)
            - ln_fact(r1 - x)
            - ln_fact(c1 - x)
            - ln_fact(n - r1 - c1 + x)
    };
    let lo = (r1 + c1).saturating_sub(n);
    let hi = r1.min(c1);
    let p0 = ln_p(a);
    let mut p = 0.0;
    for x in lo..=hi {
        let lp = ln_p(x);
        if lp <= p0 + 1e-7 {
            p += lp.exp();
        }
    }
    p.min(1.0)
}

/// Holm-Bonferroni adjusted p-values (same order as the input; NaN entries stay NaN and do not
/// count as tests).
pub fn holm(p: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..p.len()).filter(|&i| !p[i].is_nan()).collect();
    idx.sort_by(|&i, &j| p[i].total_cmp(&p[j]));
    let m = idx.len();
    let mut out = vec![f64::NAN; p.len()];
    let mut running: f64 = 0.0;
    for (k, &i) in idx.iter().enumerate() {
        running = running.max(((m - k) as f64 * p[i]).min(1.0));
        out[i] = running;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_and_moments() {
        let x = [3.0, 1.0, 2.0, 4.0];
        assert_eq!(mean(&x), 2.5);
        assert!((variance(&x) - 1.6666666666666667).abs() < 1e-12);
        assert_eq!(quantile(&x, 0.5), 2.5);
        assert_eq!(quantile(&x, 1.0), 4.0);
    }

    #[test]
    fn t_dist() {
        assert!((t_quantile(0.975, 10.0) - 2.228138851986).abs() < 1e-6);
        assert!((t_quantile(0.975, 1.0) - 12.7062047361747).abs() < 1e-5);
        assert!((t_quantile(0.995, 30.0) - 2.749995653567).abs() < 1e-6);
    }

    #[test]
    fn kolmogorov_distribution() {
        // reference values: the alternating series summed to convergence in 50-digit decimals
        for (lambda, q) in [
            (0.3, 0.999_990_694_198_665_5),
            (0.5, 0.963_945_243_664_875_1),
            (0.8, 0.544_142_411_574_198_2),
            (0.99, 0.280_873_839_225_548_9),
            (1.0, 0.269_999_671_677_354_5),
            (1.01, 0.259_434_169_093_597_5),
            (1.36, 0.049_485_876_755_377_91),
            (2.0, 6.709_252_557_796_953e-4),
            (3.0, 3.045_995_948_942_526e-8),
        ] {
            let r = kolmogorov_q(lambda);
            assert!(
                (r - q).abs() <= 1e-14 + 1e-12 * q,
                "Q({lambda}) = {r}, expected {q}"
            );
        }
        assert_eq!(kolmogorov_q(0.0), 1.0);
        assert_eq!(kolmogorov_q(1e-3), 1.0);
        assert_eq!(kolmogorov_q(40.0), 0.0);
    }

    #[test]
    fn incomplete_beta_and_erfc() {
        // I_x(a, a) = 1/2 at x = 1/2; I_x(1, b) = 1 - (1-x)^b; I_x(a, 1) = x^a
        for a in [0.5, 1.0, 3.0, 25.0, 400.0] {
            assert!((inc_beta(a, a, 0.5) - 0.5).abs() < 1e-13, "a = {a}");
        }
        for (b, x) in [(0.5f64, 0.3f64), (4.0, 0.1), (60.0, 0.02), (2.5, 0.9)] {
            let e = 1.0 - (1.0 - x).powf(b);
            assert!(
                (inc_beta(1.0, b, x) - e).abs() < 1e-13 * e.max(1e-3),
                "b = {b}, x = {x}"
            );
            let e = x.powf(b);
            assert!(
                (inc_beta(b, 1.0, x) - e).abs() < 1e-13 * e.max(1e-3),
                "a = {b}, x = {x}"
            );
        }
        assert!((erfc(1.0) - 0.157_299_207_050_285_13).abs() < 1e-16);
        assert!((erfc(-0.5) - 1.520_499_877_813_046_5).abs() < 1e-15);
    }

    #[test]
    fn ks() {
        let a: Vec<f64> = (0..200).map(|i| i as f64 / 200.0).collect();
        let b: Vec<f64> = (0..200).map(|i| (i as f64 + 0.5) / 200.0).collect();
        let (d, p) = ks_two_sample(&a, &b);
        assert!(d < 0.01 && p > 0.99);
        let c: Vec<f64> = a.iter().map(|x| x + 0.3).collect();
        let (d, p) = ks_two_sample(&a, &c);
        assert!((d - 0.3).abs() < 0.01 && p < 1e-6);
    }

    #[test]
    fn two_sample_tests() {
        // reference values computed independently (t density integrated numerically, exact
        // normal tail, hypergeometric sum)
        let a = [1.0, 2.0, 3.0, 4.0, 5.0];
        let b = [2.5, 3.5, 4.5, 5.5, 6.5, 7.5];
        assert!(
            (welch_t_test(&a, &b) - 0.086_880_708).abs() < 1e-8,
            "{}",
            welch_t_test(&a, &b)
        );
        assert!(
            (mann_whitney_u(&a, &b) - 0.120_690_80).abs() < 1e-6,
            "{}",
            mann_whitney_u(&a, &b)
        );
        assert!((fisher_exact(8, 2, 1, 5) - 0.034_965_03).abs() < 1e-6);
        assert_eq!(welch_t_test(&[1.0, 1.0], &[1.0, 1.0]), 1.0);
        assert_eq!(welch_t_test(&[1.0, 1.0], &[2.0, 2.0]), 0.0);
        assert!((normal_two_sided_p(1.959_963_985) - 0.05).abs() < 1e-6);
        let h = holm(&[0.01, 0.04, f64::NAN, 0.03]);
        assert!((h[0] - 0.03).abs() < 1e-12 && (h[3] - 0.06).abs() < 1e-12);
        assert!((h[1] - 0.06).abs() < 1e-12 && h[2].is_nan());
    }

    #[test]
    fn twm() {
        // 0 on [0,1), 2 on [1,3), end 4 with value 1 on [3,4)
        assert_eq!(
            time_weighted_mean(&[0.0, 1.0, 3.0], &[0.0, 2.0, 1.0], Some(4.0)),
            5.0 / 4.0
        );
    }
}

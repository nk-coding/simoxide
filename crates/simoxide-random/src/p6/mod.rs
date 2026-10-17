//! The StoEx distribution functions of **Palladio 6.0** (the probability-function library the
//! Slingshot nightly runs on), for the Slingshot-exact mode.
//!
//! SimuCom 5.2.2 (the [`crate::dist`] functions, SimuLizar's) inverts Commons Math 2.1
//! distributions; Palladio 6.0's `de.uka.ipd.sdq.probfunction.math.apache` wraps Commons Math 3.6.1
//! (`AbstractRealDistribution.inverseCumulativeProbability`: Chebyshev bounds and a Brent solver
//! with other accuracies; `NormalDistribution` through `Erf.erfInv`; `ExponentialDistribution`
//! through `FastMath.log`). The two libraries give results that differ in the last digits, and the
//! search can end at another iterate, so the exact mode needs these. Sources: Palladio's
//! wrappers (EPL-2.0, `Palladio-Core-Commons`, `de.uka.ipd.sdq.probfunction.math.apache`) and
//! Commons Math 3.6.1 (Apache License 2.0, `LICENSES`), with the Java operation order.
//!
//! Every function checks the parameters like the 6.0 `*DistFunction.checkParameters`, builds the
//! distribution (whose constructor may reject the parameters before anything is drawn), draws one
//! uniform and inverts it.

// Negated comparisons (`!(x > 0.0)`) are deliberate: they reproduce Java's NaN behaviour.
#![allow(clippy::neg_cmp_op_on_partial_ord)]

mod fastmath;
mod fastmath_tables;
mod special;

pub use special::MathError;

use crate::dist::DistError;
use crate::jmath;
use crate::source::UniformSource;

fn not_accepted(function: &'static str) -> DistError {
    DistError::ParametersNotAccepted { function }
}

fn invalid(dist: &'static str, message: &'static str, value: f64) -> DistError {
    DistError::InvalidParameter {
        dist,
        message,
        value,
    }
}

fn numerical(dist: &'static str, e: MathError) -> DistError {
    DistError::Numerical {
        dist,
        message: match e {
            MathError::NumberIsTooSmall => "NumberIsTooSmallException",
            MathError::NumberIsTooLarge => "NumberIsTooLargeException",
            MathError::MaxCountExceeded => "MaxCountExceededException",
            MathError::Convergence => "ConvergenceException",
            MathError::NoBracketing => "NoBracketingException",
            MathError::OutOfRange => "OutOfRangeException",
            MathError::NotStrictlyPositive => "NotStrictlyPositiveException",
        },
    }
}

/// `BrentSolver(absoluteAccuracy).solve(Integer.MAX_VALUE, f, min, max)` (relative accuracy
/// 1e-14, function value accuracy 1e-15, start in the middle).
fn brent_solve(
    f: &mut dyn FnMut(f64) -> Result<f64, MathError>,
    min: f64,
    max: f64,
    abs_acc: f64,
) -> Result<f64, MathError> {
    const REL_ACC: f64 = 1.0E-14;
    const FUN_ACC: f64 = 1.0E-15;
    let initial = min + 0.5 * (max - min);
    // verifySequence
    if !(min < initial && initial < max) {
        return Err(MathError::NumberIsTooLarge);
    }
    let y_initial = f(initial)?;
    if y_initial.abs() <= FUN_ACC {
        return Ok(initial);
    }
    let y_min = f(min)?;
    if y_min.abs() <= FUN_ACC {
        return Ok(min);
    }
    if y_initial * y_min < 0.0 {
        return brent(f, min, initial, y_min, y_initial, abs_acc, REL_ACC);
    }
    let y_max = f(max)?;
    if y_max.abs() <= FUN_ACC {
        return Ok(max);
    }
    if y_initial * y_max < 0.0 {
        return brent(f, initial, max, y_initial, y_max, abs_acc, REL_ACC);
    }
    Err(MathError::NoBracketing)
}

#[allow(clippy::too_many_arguments)]
fn brent(
    f: &mut dyn FnMut(f64) -> Result<f64, MathError>,
    lo: f64,
    hi: f64,
    f_lo: f64,
    f_hi: f64,
    t: f64,
    eps: f64,
) -> Result<f64, MathError> {
    let mut a = lo;
    let mut fa = f_lo;
    let mut b = hi;
    let mut fb = f_hi;
    let mut c = a;
    let mut fc = fa;
    let mut d = b - a;
    let mut e = d;
    loop {
        if fc.abs() < fb.abs() {
            a = b;
            b = c;
            c = a;
            fa = fb;
            fb = fc;
            fc = fa;
        }
        let tol = 2.0 * eps * b.abs() + t;
        let m = 0.5 * (c - b);
        if m.abs() <= tol || special::equals_zero(fb) {
            return Ok(b);
        }
        if e.abs() < tol || fa.abs() <= fb.abs() {
            // bisection
            d = m;
            e = d;
        } else {
            let mut s = fb / fa;
            let mut p;
            let mut q;
            if a == c {
                // linear interpolation
                p = 2.0 * m * s;
                q = 1.0 - s;
            } else {
                // inverse quadratic interpolation
                q = fa / fc;
                let r = fb / fc;
                p = s * (2.0 * m * q * (q - r) - (b - a) * (r - 1.0));
                q = (q - 1.0) * (r - 1.0) * (s - 1.0);
            }
            if p > 0.0 {
                q = -q;
            } else {
                p = -p;
            }
            s = e;
            e = d;
            if p >= 1.5 * m * q - (tol * q).abs() || p >= (0.5 * s * q).abs() {
                d = m;
                e = d;
            } else {
                d = p / q;
            }
        }
        a = b;
        fa = fb;
        if d.abs() > tol {
            b += d;
        } else if m > 0.0 {
            b += tol;
        } else {
            b -= tol;
        }
        fb = f(b)?;
        if (fb > 0.0 && fc > 0.0) || (fb <= 0.0 && fc <= 0.0) {
            c = a;
            fc = fa;
            d = b - a;
            e = d;
        }
    }
}

/// `AbstractRealDistribution.inverseCumulativeProbability(p)` of a distribution with a
/// connected support.
fn inverse_cumulative(
    p: f64,
    mut lower: f64,
    mut upper: f64,
    mean: f64,
    variance: f64,
    accuracy: f64,
    cdf: &dyn Fn(f64) -> Result<f64, MathError>,
) -> Result<f64, MathError> {
    if p < 0.0 || p > 1.0 {
        return Err(MathError::OutOfRange);
    }
    if p == 0.0 {
        return Ok(lower);
    }
    if p == 1.0 {
        return Ok(upper);
    }
    let mu = mean;
    let sig = variance.sqrt();
    let chebyshev = !(mu.is_infinite() || mu.is_nan() || sig.is_infinite() || sig.is_nan());
    if lower == f64::NEG_INFINITY {
        if chebyshev {
            lower = mu - sig * ((1.0 - p) / p).sqrt();
        } else {
            lower = -1.0;
            while cdf(lower)? >= p {
                lower *= 2.0;
            }
        }
    }
    if upper == f64::INFINITY {
        if chebyshev {
            upper = mu + sig * (p / (1.0 - p)).sqrt();
        } else {
            upper = 1.0;
            while cdf(upper)? < p {
                upper *= 2.0;
            }
        }
    }
    brent_solve(&mut |x| Ok(cdf(x)? - p), lower, upper, accuracy)
}

// ------------------------------------------------------------------------------------------
// the StoEx functions

/// `Exp(rate)`: `-mean * FastMath.log(1 - u)` (Commons Math 3 `ExponentialDistribution`).
pub fn sample_exp<R: UniformSource + ?Sized>(rate: f64, rng: &mut R) -> Result<f64, DistError> {
    if rate <= 0.0 {
        return Err(not_accepted("Exp"));
    }
    let mean = 1.0 / rate;
    if mean <= 0.0 {
        return Err(invalid("Exponential", "mean must be positive", mean));
    }
    let p = rng.next_uniform();
    if p == 1.0 {
        return Ok(f64::INFINITY);
    }
    Ok(-mean * fastmath::log(1.0 - p))
}

/// `NormalDistribution.inverseCumulativeProbability`: `mean + sd·√2·erfInv(2p − 1)`.
fn normal_inverse(mean: f64, sd: f64, p: f64) -> Result<f64, MathError> {
    if p < 0.0 || p > 1.0 {
        return Err(MathError::OutOfRange);
    }
    const SQRT2: f64 = std::f64::consts::SQRT_2;
    Ok(mean + sd * SQRT2 * special::erf_inv(2.0 * p - 1.0))
}

/// `Norm(mean, sd)`.
pub fn sample_norm<R: UniformSource + ?Sized>(
    mean: f64,
    sd: f64,
    rng: &mut R,
) -> Result<f64, DistError> {
    if sd <= 0.0 {
        return Err(invalid("Normal", "standard deviation must be positive", sd));
    }
    let p = rng.next_uniform();
    normal_inverse(mean, sd, p).map_err(|e| numerical("Normal", e))
}

/// Palladio's `LognormalDistribution.inverseF(u)`: 0 for `u == 0`, else
/// `Math.exp(Normal(mu, sigma).inverseCumulativeProbability(u))`.
fn lognormal_inverse(mu: f64, sigma: f64, u: f64) -> Result<f64, DistError> {
    if u == 0.0 {
        return Ok(0.0);
    }
    let x = normal_inverse(mu, sigma, u).map_err(|e| numerical("Lognormal", e))?;
    Ok(jmath::exp(x))
}

/// `Lognorm(mu, sigma)`.
pub fn sample_lognorm<R: UniformSource + ?Sized>(
    mu: f64,
    sigma: f64,
    rng: &mut R,
) -> Result<f64, DistError> {
    if sigma <= 0.0 {
        return Err(not_accepted("Lognorm"));
    }
    let u = rng.next_uniform();
    lognormal_inverse(mu, sigma, u)
}

/// `LognormMoments(mean, stdev)` (`LognormalDistributionFromMomentsImpl`).
pub fn sample_lognorm_moments<R: UniformSource + ?Sized>(
    mean: f64,
    stdev: f64,
    rng: &mut R,
) -> Result<f64, DistError> {
    if mean < 0.0 || stdev < 0.0 {
        return Err(not_accepted("LognormMoments"));
    }
    let variance = stdev * stdev;
    if mean <= 0.0 {
        return Err(invalid("Lognormal", "Mean has to be positive", mean));
    }
    if variance <= 0.0 {
        return Err(invalid(
            "Lognormal",
            "Variance has to be positive",
            variance,
        ));
    }
    let sigma2 = jmath::log(variance / (mean * mean) + 1.0);
    let mu = jmath::log(mean) - sigma2 / 2.0;
    let sigma = sigma2.sqrt();
    if sigma <= 0.0 {
        return Err(invalid(
            "Lognormal",
            "standard deviation must be positive",
            sigma,
        ));
    }
    let u = rng.next_uniform();
    lognormal_inverse(mu, sigma, u)
}

/// Commons Math 3 `GammaDistribution(shape, scale)` inverted at `u`.
fn gamma_inverse(shape: f64, scale: f64, u: f64) -> Result<f64, MathError> {
    let cdf = |x: f64| -> Result<f64, MathError> {
        if x <= 0.0 {
            Ok(0.0)
        } else {
            special::regularized_gamma_p(shape, x / scale)
        }
    };
    inverse_cumulative(
        u,
        0.0,
        f64::INFINITY,
        shape * scale,
        shape * scale * scale,
        1.0E-9,
        &cdf,
    )
}

/// `Gamma(alpha, theta)`.
pub fn sample_gamma<R: UniformSource + ?Sized>(
    alpha: f64,
    theta: f64,
    rng: &mut R,
) -> Result<f64, DistError> {
    if theta <= 0.0 || alpha <= 0.0 {
        return Err(not_accepted("Gamma"));
    }
    let u = rng.next_uniform();
    gamma_inverse(alpha, theta, u).map_err(|e| numerical("Gamma", e))
}

/// `GammaMoments(mean, coeffVar)` (`GammaDistributionFromMomentsImpl`).
pub fn sample_gamma_moments<R: UniformSource + ?Sized>(
    mean: f64,
    coeff_var: f64,
    rng: &mut R,
) -> Result<f64, DistError> {
    if mean < 0.0 || coeff_var < 0.0 {
        return Err(not_accepted("GammaMoments"));
    }
    let variance = coeff_var * coeff_var * mean * mean;
    let theta = variance / mean;
    let alpha = mean / (variance / mean);
    if !(alpha > 0.0) || !(theta > 0.0) {
        return Err(invalid("Gamma", "shape and scale must be positive", alpha));
    }
    let u = rng.next_uniform();
    gamma_inverse(alpha, theta, u).map_err(|e| numerical("Gamma", e))
}

/// `UniDouble(a, b)`: `UniformDistributionImpl(a, b)` (its CDF, no inverse of its own) inverted
/// by the generic solver.
pub fn sample_unidouble<R: UniformSource + ?Sized>(
    a: f64,
    b: f64,
    rng: &mut R,
) -> Result<f64, DistError> {
    if a > b {
        return Err(not_accepted("UniDouble"));
    }
    if b < a {
        return Err(invalid(
            "Uniform",
            "Second value has to be greater than first value of interval",
            b,
        ));
    }
    let u = rng.next_uniform();
    let cdf = |x: f64| -> Result<f64, MathError> {
        Ok(if x < a {
            0.0
        } else if x > b {
            1.0
        } else {
            (x - a) / (b - a)
        })
    };
    inverse_cumulative(
        u,
        a,
        b,
        (a + b) / 2.0,
        1.0 / 12.0 * (b - a) * (b - a),
        1.0E-6,
        &cdf,
    )
    .map_err(|e| numerical("Uniform", e))
}

/// `AbstractIntegerDistribution.inverseCumulativeProbability(p)` (connected support).
fn inverse_cumulative_int(
    p: f64,
    mut lower: i32,
    mut upper: i32,
    mean: f64,
    variance: f64,
    cdf: &dyn Fn(i32) -> Result<f64, MathError>,
) -> Result<i32, MathError> {
    if p < 0.0 || p > 1.0 {
        return Err(MathError::OutOfRange);
    }
    // checkedCumulativeProbability: NaN is a MathInternalError
    let checked = |x: i32| -> Result<f64, MathError> {
        let r = cdf(x)?;
        if r.is_nan() {
            return Err(MathError::Convergence);
        }
        Ok(r)
    };
    if p == 0.0 {
        return Ok(lower);
    }
    if lower == i32::MIN {
        if checked(lower)? >= p {
            return Ok(lower);
        }
    } else {
        lower -= 1;
    }
    if p == 1.0 {
        return Ok(upper);
    }
    let mu = mean;
    let sigma = variance.sqrt();
    let chebyshev =
        !(mu.is_infinite() || mu.is_nan() || sigma.is_infinite() || sigma.is_nan() || sigma == 0.0);
    if chebyshev {
        let mut k = ((1.0 - p) / p).sqrt();
        let mut tmp = mu - k * sigma;
        if tmp > f64::from(lower) {
            lower = (tmp.ceil() as i32) - 1;
        }
        k = 1.0 / k;
        tmp = mu + k * sigma;
        if tmp < f64::from(upper) {
            upper = (tmp.ceil() as i32) - 1;
        }
    }
    // solveInverseCumulativeProbability: bisection
    while lower + 1 < upper {
        let mut xm = (lower + upper) / 2;
        if xm < lower || xm > upper {
            xm = lower + (upper - lower) / 2;
        }
        if checked(xm)? >= p {
            upper = xm;
        } else {
            lower = xm;
        }
    }
    Ok(upper)
}

/// `Pois(mean)`: Commons Math 3 `PoissonDistribution(mean)` (CDF `Q(x + 1, mean)`, epsilon
/// 1e-12, at most 10^7 iterations).
pub fn sample_pois<R: UniformSource + ?Sized>(mean: f64, rng: &mut R) -> Result<i32, DistError> {
    if mean < 0.0 {
        return Err(not_accepted("Pois"));
    }
    if mean <= 0.0 {
        return Err(invalid("Poisson", "mean must be positive", mean));
    }
    let u = rng.next_uniform();
    let cdf = |x: i32| -> Result<f64, MathError> {
        if x < 0 {
            Ok(0.0)
        } else if x == i32::MAX {
            Ok(1.0)
        } else {
            special::regularized_gamma_q_with(f64::from(x) + 1.0, mean, 1.0E-12, 10_000_000)
        }
    };
    inverse_cumulative_int(u, 0, i32::MAX, mean, mean, &cdf).map_err(|e| numerical("Poisson", e))
}

/// `UniInt(a, b)`: Palladio's `UniformIntDistributionImpl` (CDF `(x - a + 1) / (b - a + 1)`,
/// a negative variance, so no Chebyshev bounds) inverted by bisection.
pub fn sample_uniint<R: UniformSource + ?Sized>(
    a: i32,
    b: i32,
    rng: &mut R,
) -> Result<i32, DistError> {
    if b < a {
        return Err(invalid(
            "UniformInt",
            "Second value has to be greater than first value of interval",
            f64::from(b),
        ));
    }
    let count = b.wrapping_sub(a).wrapping_add(1);
    let u = rng.next_uniform();
    let cdf = |x: i32| -> Result<f64, MathError> {
        Ok(if x < a {
            0.0
        } else if x > b {
            1.0
        } else {
            f64::from(x - a + 1) / f64::from(count)
        })
    };
    let mean = (f64::from(a) + f64::from(b)) / 2.0;
    let variance = (-f64::from(count) * f64::from(count)) / 12.0;
    inverse_cumulative_int(u, a, b, mean, variance, &cdf).map_err(|e| numerical("UniformInt", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct One(f64);
    impl UniformSource for One {
        fn next_uniform(&mut self) -> f64 {
            self.0
        }
    }

    #[test]
    fn samples_of_palladio_6() {
        // the byte-for-byte check is the Slingshot-exact corpus (simoxide-sim, tests/slingshot_exact.rs)
        let x = sample_unidouble(0.2, 0.8, &mut One(0.25)).unwrap();
        assert!((x - 0.35).abs() < 1e-6, "{x}");
        assert!(sample_exp(0.0, &mut One(0.5)).is_err());
        assert_eq!(sample_norm(1.0, 2.0, &mut One(0.5)).unwrap(), 1.0);
        assert!(sample_gamma(2.0, 3.0, &mut One(0.5)).unwrap() > 0.0);
        assert_eq!(sample_lognorm(0.0, 1.0, &mut One(0.0)).unwrap(), 0.0);
    }
}

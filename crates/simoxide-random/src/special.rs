//! Ports of the Commons Math 2.1 numerics used by the inverse CDFs: `special.Gamma`,
//! `special.Erf`, `util.ContinuedFraction`, `analysis.UnivariateRealSolverUtils.bracket`,
//! `analysis.solvers.BrentSolver` and the generic `inverseCumulativeProbability` of
//! `AbstractContinuousDistribution` / `AbstractIntegerDistribution`.
//!
//! Operation order follows the Java sources exactly; `Math.log`/`Math.exp` are [`crate::jmath`].

use crate::jmath;

/// Failure of a Commons Math routine (the Java exception it corresponds to).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathError {
    /// `MaxIterationsExceededException`.
    MaxIterations,
    /// `ConvergenceException` (continued fraction diverged, or bracketing failed).
    Convergence(&'static str),
    /// `FunctionEvaluationException`: the CDF returned NaN (or failed) during root finding.
    FunctionEvaluation,
    /// `IllegalArgumentException` (invalid bracket / interval / probability).
    IllegalArgument(&'static str),
}

impl MathError {
    /// A short message for error reports.
    pub fn message(&self) -> &'static str {
        match self {
            MathError::MaxIterations => "maximal number of iterations exceeded",
            MathError::Convergence(m) | MathError::IllegalArgument(m) => m,
            MathError::FunctionEvaluation => "cumulative probability function returned NaN",
        }
    }
}

type R<T> = Result<T, MathError>;

/// `Gamma.LANCZOS`, spelled as in the Java source.
#[allow(clippy::excessive_precision)]
const LANCZOS: [f64; 15] = [
    0.99999999999999709182,
    57.156235665862923517,
    -59.597960355475491248,
    14.136097974741747174,
    -0.49191381609762019978,
    0.33994649984811888699e-4,
    0.46523628927048575665e-4,
    -0.98374475304879564677e-4,
    0.15808870322491248884e-3,
    -0.21026444172410488319e-3,
    0.21743961811521264320e-3,
    -0.16431810653676389022e-3,
    0.84418223983852743293e-4,
    -0.26190838401581408670e-4,
    0.36899182659531622704e-5,
];

/// `Gamma.HALF_LOG_2_PI = 0.5 * Math.log(2.0 * Math.PI)` (checked in the tests).
const HALF_LOG_2_PI: f64 = 0.918_938_533_204_672_7; // bits 0x3fed67f1c864beb4 (from the JVM)

/// `Gamma.DEFAULT_EPSILON` (`10e-15`).
pub const GAMMA_DEFAULT_EPSILON: f64 = 10e-15;

/// `Gamma.logGamma(x)` (Lanczos approximation).
pub fn log_gamma(x: f64) -> f64 {
    if x.is_nan() || x <= 0.0 {
        return f64::NAN;
    }
    let g = 607.0 / 128.0;
    let mut sum = 0.0;
    let mut i = LANCZOS.len() - 1;
    while i > 0 {
        sum += LANCZOS[i] / (x + i as f64);
        i -= 1;
    }
    sum += LANCZOS[0];
    let tmp = x + g + 0.5;
    ((x + 0.5) * jmath::log(tmp)) - tmp + HALF_LOG_2_PI + jmath::log(sum / x)
}

/// [`log_gamma`] with a one-entry per-thread memo. A root search evaluates the regularized gamma
/// function many times with the same `a` (0.5 for `erf`, `alpha` for Gamma); `log_gamma` is a
/// pure function, so the memoized value is bit-identical.
#[inline]
fn log_gamma_memo(a: f64) -> f64 {
    use std::cell::Cell;
    thread_local! {
        static MEMO: Cell<(u64, f64)> = const { Cell::new((u64::MAX, f64::NAN)) };
    }
    let bits = a.to_bits();
    MEMO.with(|m| {
        let (k, v) = m.get();
        if k == bits {
            return v;
        }
        let v = log_gamma(a);
        m.set((bits, v));
        v
    })
}

/// `Gamma.regularizedGammaP(a, x, epsilon, maxIterations)`.
pub fn regularized_gamma_p(a: f64, x: f64, epsilon: f64, max_iterations: i32) -> R<f64> {
    if a.is_nan() || x.is_nan() || a <= 0.0 || x < 0.0 {
        Ok(f64::NAN)
    } else if x == 0.0 {
        Ok(0.0)
    } else if x >= a + 1.0 {
        Ok(1.0 - regularized_gamma_q(a, x, epsilon, max_iterations)?)
    } else {
        let max = max_iterations as f64;
        let mut n = 0.0;
        let mut an = 1.0 / a;
        let mut sum = an;
        while (an / sum).abs() > epsilon && n < max && sum < f64::INFINITY {
            n += 1.0;
            an *= x / (a + n);
            sum += an;
        }
        if n >= max {
            Err(MathError::MaxIterations)
        } else if sum.is_infinite() {
            Ok(1.0)
        } else {
            Ok(jmath::exp(-x + (a * jmath::log(x)) - log_gamma_memo(a)) * sum)
        }
    }
}

/// `Gamma.regularizedGammaQ(a, x, epsilon, maxIterations)`.
pub fn regularized_gamma_q(a: f64, x: f64, epsilon: f64, max_iterations: i32) -> R<f64> {
    if a.is_nan() || x.is_nan() || a <= 0.0 || x < 0.0 {
        Ok(f64::NAN)
    } else if x == 0.0 {
        Ok(1.0)
    } else if x < a + 1.0 {
        Ok(1.0 - regularized_gamma_p(a, x, epsilon, max_iterations)?)
    } else {
        let cf = continued_fraction(
            |n, x| ((2.0 * n as f64) + 1.0) - a + x,
            |n, _| n as f64 * (a - n as f64),
            x,
            epsilon,
            max_iterations,
        )?;
        let ret = 1.0 / cf;
        Ok(jmath::exp(-x + (a * jmath::log(x)) - log_gamma_memo(a)) * ret)
    }
}

/// `ContinuedFraction.evaluate(x, epsilon, maxIterations)` of Commons Math 2.1.
pub fn continued_fraction(
    get_a: impl Fn(i32, f64) -> f64,
    get_b: impl Fn(i32, f64) -> f64,
    x: f64,
    epsilon: f64,
    max_iterations: i32,
) -> R<f64> {
    let mut p0 = 1.0;
    let mut p1 = get_a(0, x);
    let mut q0 = 0.0;
    let mut q1 = 1.0;
    let mut c = p1 / q1;
    let mut n: i32 = 0;
    let mut relative_error = f64::MAX;
    while n < max_iterations && relative_error > epsilon {
        n += 1;
        let a = get_a(n, x);
        let b = get_b(n, x);
        let mut p2 = a * p1 + b * p0;
        let mut q2 = a * q1 + b * q0;
        let mut infinite = false;
        if p2.is_infinite() || q2.is_infinite() {
            let mut scale_factor = 1.0;
            let mut last_scale_factor;
            let max_power = 5;
            let scale = jmath::max(a, b);
            if scale <= 0.0 {
                return Err(MathError::Convergence(
                    "Continued fraction convergents diverged to +/- infinity",
                ));
            }
            infinite = true;
            for _ in 0..max_power {
                last_scale_factor = scale_factor;
                scale_factor *= scale;
                if a != 0.0 && a > b {
                    p2 = p1 / last_scale_factor + (b / scale_factor * p0);
                    q2 = q1 / last_scale_factor + (b / scale_factor * q0);
                } else if b != 0.0 {
                    p2 = (a / scale_factor * p1) + p0 / last_scale_factor;
                    q2 = (a / scale_factor * q1) + q0 / last_scale_factor;
                }
                infinite = p2.is_infinite() || q2.is_infinite();
                if !infinite {
                    break;
                }
            }
        }
        if infinite {
            return Err(MathError::Convergence(
                "Continued fraction convergents diverged to +/- infinity",
            ));
        }
        let r = p2 / q2;
        if r.is_nan() {
            return Err(MathError::Convergence("Continued fraction diverged to NaN"));
        }
        relative_error = (r / c - 1.0).abs();
        c = p2 / q2;
        p0 = p1;
        p1 = p2;
        q0 = q1;
        q1 = q2;
    }
    if n >= max_iterations {
        return Err(MathError::MaxIterations);
    }
    Ok(c)
}

/// `Erf.erf(x) = regularizedGammaP(0.5, x * x, 1.0e-15, 10000)` (negated for `x < 0`).
pub fn erf(x: f64) -> R<f64> {
    let mut ret = regularized_gamma_p(0.5, x * x, 1.0e-15, 10000)?;
    if x < 0.0 {
        ret = -ret;
    }
    Ok(ret)
}

// ---------------------------------------------------------------------------------------------
// Root finding.

/// How [`bracket_with`] searches for the first sign change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BracketSearch {
    /// Exactly the reference loop: widen by 1.0 per step, evaluate the CDF at every step.
    /// Costs two CDF evaluations per unit of distance between the initial point and the root.
    #[cfg_attr(not(feature = "fast-bracket"), default)]
    Linear,
    /// Same interval sequence (the endpoints are still stepped by 1.0 in floating point), but
    /// the stopping step is found by exponential + binary search: O(log k) CDF evaluations.
    /// Returns the same `(a, b)` as `Linear` whenever the computed CDF is monotone along the
    /// endpoint sequences (then "keep widening" is monotone in the step count). Differences:
    /// it may evaluate the CDF at a few points beyond the linear stopping point.
    #[cfg_attr(feature = "fast-bracket", default)]
    Galloping,
}

/// `UnivariateRealSolverUtils.bracket(f, initial, lower, upper)` (unbounded iterations).
/// `Err(Convergence)` corresponds to the `ConvergenceException` that callers may catch.
pub fn bracket(
    f: &mut impl FnMut(f64) -> R<f64>,
    initial: f64,
    lower: f64,
    upper: f64,
) -> R<(f64, f64)> {
    bracket_with(f, initial, lower, upper, BracketSearch::Linear)
}

/// [`bracket`] with a choice of search strategy.
pub fn bracket_with(
    f: &mut impl FnMut(f64) -> R<f64>,
    initial: f64,
    lower: f64,
    upper: f64,
    search: BracketSearch,
) -> R<(f64, f64)> {
    bracket_values(f, initial, lower, upper, search).map(|(a, b, _, _)| (a, b))
}

/// [`bracket_with`], also returning `f(a)` and `f(b)` of the bracket `(a, b)`.
///
/// `f` must be a pure function (the same value for the same argument): an endpoint that did
/// not move since the previous step (clamped at `lower` or `upper`) is not evaluated again.
fn bracket_values(
    f: &mut impl FnMut(f64) -> R<f64>,
    initial: f64,
    lower: f64,
    upper: f64,
    search: BracketSearch,
) -> R<(f64, f64, f64, f64)> {
    if initial < lower || initial > upper || lower >= upper {
        return Err(MathError::IllegalArgument("invalid bracketing parameters"));
    }
    // Java: `numIterations < maximumIterations` with maximumIterations = Integer.MAX_VALUE.
    const MAX_ITER: u64 = i32::MAX as u64;
    #[inline(always)]
    fn step(a: f64, b: f64, lower: f64, upper: f64) -> (f64, f64) {
        (jmath::max(a - 1.0, lower), jmath::min(b + 1.0, upper))
    }
    // Evaluates the loop condition after `k` steps with endpoints (a, b).
    let mut keep_going = |k: u64, a: f64, b: f64| -> R<(bool, f64, f64)> {
        let fa = f(a)?;
        let fb = f(b)?;
        let go = (fa * fb > 0.0) && (k < MAX_ITER) && ((a > lower) || (b < upper));
        Ok((go, fa, fb))
    };
    let (a, b, fa, fb) = match search {
        BracketSearch::Linear => {
            // `keep_going` inlined, with the previous endpoint values (none before step 1)
            let (mut a, mut b) = (initial, initial);
            let (mut prev_a, mut prev_b) = (None, None);
            let mut k = 0u64;
            loop {
                (a, b) = step(a, b, lower, upper);
                k += 1;
                let fa = match prev_a {
                    Some((x, v)) if f64::to_bits(x) == a.to_bits() => v,
                    _ => f(a)?,
                };
                let fb = match prev_b {
                    Some((x, v)) if f64::to_bits(x) == b.to_bits() => v,
                    _ => f(b)?,
                };
                let go = (fa * fb > 0.0) && (k < MAX_ITER) && ((a > lower) || (b < upper));
                if !go {
                    break (a, b, fa, fb);
                }
                (prev_a, prev_b) = (Some((a, fa)), Some((b, fb)));
            }
        }
        BracketSearch::Galloping => {
            // Phase 1: evaluate at k = 1, 2, 4, ...; remember the last "keep going" state.
            let (mut a, mut b) = (initial, initial);
            let mut k = 0u64;
            let mut lo = (0u64, initial, initial); // state known to keep going (k = 0: trivially)
            let mut next_probe = 1u64;
            let (mut hi_k, mut hi) = loop {
                (a, b) = step(a, b, lower, upper);
                k += 1;
                if k == next_probe || k == MAX_ITER {
                    let (go, fa, fb) = keep_going(k, a, b)?;
                    if !go {
                        break (k, (a, b, fa, fb));
                    }
                    lo = (k, a, b);
                    next_probe = next_probe.saturating_mul(2);
                }
            };
            // Phase 2: binary search in (lo.0, hi_k]; the state at lo is kept and advanced.
            while hi_k - lo.0 > 1 {
                let mid = lo.0 + (hi_k - lo.0) / 2;
                let (mut ma, mut mb) = (lo.1, lo.2);
                for _ in lo.0..mid {
                    (ma, mb) = step(ma, mb, lower, upper);
                }
                let (go, fa, fb) = keep_going(mid, ma, mb)?;
                if go {
                    lo = (mid, ma, mb);
                } else {
                    hi_k = mid;
                    hi = (ma, mb, fa, fb);
                }
            }
            hi
        }
    };
    if fa * fb > 0.0 {
        return Err(MathError::Convergence("bracketing failed"));
    }
    Ok((a, b, fa, fb))
}

/// Default settings of `BrentSolver` in Commons Math 2.1.
const BRENT_MAX_ITERATIONS: i32 = 100;
const BRENT_RELATIVE_ACCURACY: f64 = 1.0e-14;
const BRENT_FUNCTION_VALUE_ACCURACY: f64 = 1.0e-15;

/// `BrentSolver.solve(f, min, max)` with the given absolute accuracy (as used by
/// `UnivariateRealSolverUtils.solve(f, x0, x1, absoluteAccuracy)`).
pub fn brent_solve(
    f: &mut impl FnMut(f64) -> R<f64>,
    min: f64,
    max: f64,
    absolute_accuracy: f64,
) -> R<f64> {
    if min >= max {
        return Err(MathError::IllegalArgument(
            "endpoints do not specify an interval",
        ));
    }
    let y_min = f(min)?;
    let y_max = f(max)?;
    brent_solve_values(f, min, y_min, max, y_max, absolute_accuracy)
}

/// [`brent_solve`] after `f(min)` and `f(max)` (`min < max`).
fn brent_solve_values(
    f: &mut impl FnMut(f64) -> R<f64>,
    min: f64,
    y_min: f64,
    max: f64,
    y_max: f64,
    absolute_accuracy: f64,
) -> R<f64> {
    let sign = y_min * y_max;
    if sign > 0.0 {
        if y_min.abs() <= BRENT_FUNCTION_VALUE_ACCURACY {
            Ok(min)
        } else if y_max.abs() <= BRENT_FUNCTION_VALUE_ACCURACY {
            Ok(max)
        } else {
            Err(MathError::IllegalArgument(
                "function values at endpoints do not have different signs",
            ))
        }
    } else if sign < 0.0 {
        brent_iterate(f, min, y_min, max, y_max, min, y_min, absolute_accuracy)
    } else if y_min == 0.0 {
        Ok(min)
    } else {
        // Also reached for NaN products.
        Ok(max)
    }
}

#[allow(clippy::too_many_arguments)]
fn brent_iterate(
    f: &mut impl FnMut(f64) -> R<f64>,
    mut x0: f64,
    mut y0: f64,
    mut x1: f64,
    mut y1: f64,
    mut x2: f64,
    mut y2: f64,
    absolute_accuracy: f64,
) -> R<f64> {
    let mut delta = x1 - x0;
    let mut old_delta = delta;
    let mut i = 0;
    while i < BRENT_MAX_ITERATIONS {
        if y2.abs() < y1.abs() {
            x0 = x1;
            x1 = x2;
            x2 = x0;
            y0 = y1;
            y1 = y2;
            y2 = y0;
        }
        if y1.abs() <= BRENT_FUNCTION_VALUE_ACCURACY {
            return Ok(x1);
        }
        let dx = x2 - x1;
        let tolerance = jmath::max(BRENT_RELATIVE_ACCURACY * x1.abs(), absolute_accuracy);
        if dx.abs() <= tolerance {
            return Ok(x1);
        }
        if (old_delta.abs() < tolerance) || (y0.abs() <= y1.abs()) {
            delta = 0.5 * dx;
            old_delta = delta;
        } else {
            let r3 = y1 / y0;
            let mut p;
            let mut p1;
            if x0 == x2 {
                p = dx * r3;
                p1 = 1.0 - r3;
            } else {
                let r1 = y0 / y2;
                let r2 = y1 / y2;
                p = r3 * (dx * r1 * (r1 - r2) - (x1 - x0) * (r2 - 1.0));
                p1 = (r1 - 1.0) * (r2 - 1.0) * (r3 - 1.0);
            }
            if p > 0.0 {
                p1 = -p1;
            } else {
                p = -p;
            }
            if 2.0 * p >= 1.5 * dx * p1 - (tolerance * p1).abs()
                || p >= (0.5 * old_delta * p1).abs()
            {
                delta = 0.5 * dx;
                old_delta = delta;
            } else {
                old_delta = delta;
                delta = p / p1;
            }
        }
        x0 = x1;
        y0 = y1;
        if delta.abs() > tolerance {
            x1 += delta;
        } else if dx > 0.0 {
            x1 += 0.5 * tolerance;
        } else if dx <= 0.0 {
            x1 -= 0.5 * tolerance;
        }
        y1 = f(x1)?;
        if (y1 > 0.0) == (y2 > 0.0) {
            x2 = x0;
            y2 = y0;
            delta = x1 - x0;
            old_delta = delta;
        }
        i += 1;
    }
    Err(MathError::MaxIterations)
}

// ---------------------------------------------------------------------------------------------
// Generic inverse CDFs.

/// The virtual methods of a Commons Math 2.1 `AbstractContinuousDistribution` subclass.
pub trait ContinuousDistribution {
    /// `cumulativeProbability(x)`.
    fn cdf(&self, x: f64) -> R<f64>;
    /// `getInitialDomain(p)`.
    fn initial_domain(&self, p: f64) -> f64;
    /// `getDomainLowerBound(p)`.
    fn domain_lower_bound(&self, p: f64) -> f64;
    /// `getDomainUpperBound(p)`.
    fn domain_upper_bound(&self, p: f64) -> f64;
    /// `getSolverAbsoluteAccuracy()`.
    fn solver_absolute_accuracy(&self) -> f64;
}

/// `AbstractContinuousDistribution.inverseCumulativeProbability(p)`: bracket from the initial
/// domain in steps of 1.0, then Brent.
pub fn inverse_cumulative_continuous(d: &impl ContinuousDistribution, p: f64) -> R<f64> {
    inverse_cumulative_continuous_with(d, p, BracketSearch::Linear)
}

/// [`inverse_cumulative_continuous`] with a choice of bracket search.
pub fn inverse_cumulative_continuous_with(
    d: &impl ContinuousDistribution,
    p: f64,
    search: BracketSearch,
) -> R<f64> {
    if p < 0.0 || p > 1.0 {
        return Err(MathError::IllegalArgument(
            "probability out of [0, 1] range",
        ));
    }
    let mut f = |x: f64| -> R<f64> {
        let ret = d.cdf(x).map_err(|_| MathError::FunctionEvaluation)? - p;
        if ret.is_nan() {
            return Err(MathError::FunctionEvaluation);
        }
        Ok(ret)
    };
    let lower = d.domain_lower_bound(p);
    let upper = d.domain_upper_bound(p);
    let acc = d.solver_absolute_accuracy();
    let (a, b, fa, fb) = match bracket_values(&mut f, d.initial_domain(p), lower, upper, search) {
        Ok(ab) => ab,
        Err(MathError::Convergence(m)) => {
            if f(lower)?.abs() < acc {
                return Ok(lower);
            }
            if f(upper)?.abs() < acc {
                return Ok(upper);
            }
            return Err(MathError::Convergence(m));
        }
        Err(e) => return Err(e),
    };
    // `BrentSolver.solve(f, a, b)` evaluates `f(a)` and `f(b)` again: `f` is a pure function
    // of `x` (the CDF minus `p`), so the bracket's values are the values it would compute
    if a >= b {
        return brent_solve(&mut f, a, b, acc);
    }
    brent_solve_values(&mut f, a, fa, b, fb, acc)
}

/// The virtual methods of a Commons Math 2.1 `AbstractIntegerDistribution` subclass.
pub trait IntegerDistribution {
    /// `cumulativeProbability(int x)`.
    fn cdf_int(&self, x: i32) -> R<f64>;
    /// `getDomainLowerBound(p)`.
    fn domain_lower_bound(&self, p: f64) -> i32;
    /// `getDomainUpperBound(p)`.
    fn domain_upper_bound(&self, p: f64) -> i32;
}

/// `AbstractIntegerDistribution.inverseCumulativeProbability(p)`: the largest `x` with
/// `F(x) <= p` found by bisection (with Java `int` wrap-around semantics).
pub fn inverse_cumulative_int(d: &impl IntegerDistribution, p: f64) -> R<i32> {
    if p < 0.0 || p > 1.0 {
        return Err(MathError::IllegalArgument(
            "probability out of [0, 1] range",
        ));
    }
    let checked = |x: i32| -> R<f64> {
        let r = d.cdf_int(x).map_err(|_| MathError::FunctionEvaluation)?;
        if r.is_nan() {
            return Err(MathError::FunctionEvaluation);
        }
        Ok(r)
    };
    let mut x0 = d.domain_lower_bound(p);
    let mut x1 = d.domain_upper_bound(p);
    let mut pm;
    while x0 < x1 {
        let xm = x0.wrapping_add(x1.wrapping_sub(x0) / 2);
        pm = checked(xm)?;
        if pm > p {
            if xm == x1 {
                x1 = x1.wrapping_sub(1);
            } else {
                x1 = xm;
            }
        } else if xm == x0 {
            x0 = x0.wrapping_add(1);
        } else {
            x0 = xm;
        }
    }
    pm = checked(x0)?;
    while pm > p {
        x0 = x0.wrapping_sub(1);
        pm = checked(x0)?;
    }
    Ok(x0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_log_2_pi_constant() {
        assert_eq!(
            HALF_LOG_2_PI.to_bits(),
            (0.5 * jmath::log(2.0 * std::f64::consts::PI)).to_bits()
        );
    }

    #[test]
    fn lanczos_literals() {
        // The Java literals, parsed by Rust from their original spelling.
        let java: [f64; 15] = [
            "0.99999999999999709182",
            "57.156235665862923517",
            "-59.597960355475491248",
            "14.136097974741747174",
            "-0.49191381609762019978",
            ".33994649984811888699e-4",
            ".46523628927048575665e-4",
            "-.98374475304879564677e-4",
            ".15808870322491248884e-3",
            "-.21026444172410488319e-3",
            ".21743961811521264320e-3",
            "-.16431810653676389022e-3",
            ".84418223983852743293e-4",
            "-.26190838401581408670e-4",
            ".36899182659531622704e-5",
        ]
        .map(|s| s.parse::<f64>().unwrap());
        for (a, b) in LANCZOS.iter().zip(java) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }

    #[test]
    fn erf_values() {
        assert!((erf(0.5).unwrap() - 0.520_499_877_813_046_5).abs() < 1e-14);
        assert!((erf(-2.0).unwrap() + 0.995_322_265_018_952_7).abs() < 1e-14);
        assert!((log_gamma(10.0) - 12.801_827_480_081_469).abs() < 1e-12);
    }
}

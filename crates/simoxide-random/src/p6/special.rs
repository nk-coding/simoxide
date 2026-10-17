//! Special functions of Apache Commons Math 3.6.1 (`org.apache.commons.math3.special.Gamma`,
//! `Erf.erfInv`, `util.ContinuedFraction`; Apache License 2.0, `LICENSES`), with the operation
//! order of the Java code.

#![allow(
    clippy::excessive_precision,
    clippy::unreadable_literal,
    clippy::needless_late_init
)]

use super::fastmath;

const LANCZOS_G: f64 = 4.7421875;
const LANCZOS: [f64; 15] = [
    0.9999999999999971,
    57.15623566586292,
    -59.59796035547549,
    14.136097974741746,
    -0.4919138160976202,
    3.399464998481189E-5,
    4.652362892704858E-5,
    -9.837447530487956E-5,
    1.580887032249125E-4,
    -2.1026444172410488E-4,
    2.1743961811521265E-4,
    -1.643181065367639E-4,
    8.441822398385275E-5,
    -2.6190838401581408E-5,
    3.6899182659531625E-6,
];

/// Why a computation failed (a Commons Math exception).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathError {
    NumberIsTooSmall,
    NumberIsTooLarge,
    MaxCountExceeded,
    Convergence,
    NoBracketing,
    OutOfRange,
    NotStrictlyPositive,
}

fn half_log_2_pi() -> f64 {
    0.5 * fastmath::log(std::f64::consts::TAU)
}

/// `Gamma.lanczos(x)`.
pub fn lanczos(x: f64) -> f64 {
    let mut sum = 0.0;
    for i in (1..LANCZOS.len()).rev() {
        sum += LANCZOS[i] / (x + i as f64);
    }
    sum + LANCZOS[0]
}

/// `Gamma.invGamma1pm1(x)`: `1 / Γ(1 + x) - 1` for `-0.5 <= x <= 1.5`.
pub fn inv_gamma1pm1(x: f64) -> Result<f64, MathError> {
    if x < -0.5 {
        return Err(MathError::NumberIsTooSmall);
    }
    if x > 1.5 {
        return Err(MathError::NumberIsTooLarge);
    }
    let t = if x <= 0.5 { x } else { x - 0.5 - 0.5 };
    let ret;
    if t < 0.0 {
        let a = 6.116095104481416E-9 + t * 6.247308301164655E-9;
        let mut b = 1.9575583661463974E-10;
        b = -6.077618957228252E-8 + t * b;
        b = 9.926418406727737E-7 + t * b;
        b = -6.4304548177935305E-6 + t * b;
        b = -8.514194324403149E-6 + t * b;
        b = 4.939449793824468E-4 + t * b;
        b = 0.026620534842894922 + t * b;
        b = 0.203610414066807 + t * b;
        b = 1.0 + t * b;
        let mut c = -2.056338416977607E-7 + t * (a / b);
        c = 1.133027231981696E-6 + t * c;
        c = -1.2504934821426706E-6 + t * c;
        c = -2.013485478078824E-5 + t * c;
        c = 1.280502823881162E-4 + t * c;
        c = -2.1524167411495098E-4 + t * c;
        c = -0.0011651675918590652 + t * c;
        c = 0.0072189432466631 + t * c;
        c = -0.009621971527876973 + t * c;
        c = -0.04219773455554433 + t * c;
        c = 0.16653861138229148 + t * c;
        c = -0.04200263503409524 + t * c;
        c = -0.6558780715202539 + t * c;
        c = -0.42278433509846713 + t * c;
        ret = if x > 0.5 {
            t * c / x
        } else {
            x * ((c + 0.5) + 0.5)
        };
    } else {
        let mut p = 4.343529937408594E-15;
        p = -1.2494415722763663E-13 + t * p;
        p = 1.5728330277104463E-12 + t * p;
        p = 4.686843322948848E-11 + t * p;
        p = 6.820161668496171E-10 + t * p;
        p = 6.8716741130671986E-9 + t * p;
        p = 6.116095104481416E-9 + t * p;
        let mut q = 2.6923694661863613E-4;
        q = 0.004956830093825887 + t * q;
        q = 0.054642130860422966 + t * q;
        q = 0.3056961078365221 + t * q;
        q = 1.0 + t * q;
        let mut c = -2.056338416977607E-7 + (p / q) * t;
        c = 1.133027231981696E-6 + t * c;
        c = -1.2504934821426706E-6 + t * c;
        c = -2.013485478078824E-5 + t * c;
        c = 1.280502823881162E-4 + t * c;
        c = -2.1524167411495098E-4 + t * c;
        c = -0.0011651675918590652 + t * c;
        c = 0.0072189432466631 + t * c;
        c = -0.009621971527876973 + t * c;
        c = -0.04219773455554433 + t * c;
        c = 0.16653861138229148 + t * c;
        c = -0.04200263503409524 + t * c;
        c = -0.6558780715202539 + t * c;
        c = 0.5772156649015329 + t * c;
        ret = if x > 0.5 {
            (t / x) * ((c - 0.5) - 0.5)
        } else {
            x * c
        };
    }
    Ok(ret)
}

/// `Gamma.logGamma1p(x)` for `-0.5 <= x <= 1.5`.
pub fn log_gamma1p(x: f64) -> Result<f64, MathError> {
    if x < -0.5 {
        return Err(MathError::NumberIsTooSmall);
    }
    if x > 1.5 {
        return Err(MathError::NumberIsTooLarge);
    }
    Ok(-fastmath::log1p(inv_gamma1pm1(x)?))
}

/// `Gamma.logGamma(x)`.
pub fn log_gamma(x: f64) -> Result<f64, MathError> {
    if x.is_nan() || x <= 0.0 {
        return Ok(f64::NAN);
    }
    if x < 0.5 {
        return Ok(log_gamma1p(x)? - fastmath::log(x));
    }
    if x <= 2.5 {
        return log_gamma1p((x - 0.5) - 0.5);
    }
    if x <= 8.0 {
        let n = (x - 1.5).floor() as i32;
        let mut prod = 1.0;
        for i in 1..=n {
            prod *= x - f64::from(i);
        }
        return Ok(log_gamma1p(x - f64::from(n + 1))? + fastmath::log(prod));
    }
    let sum = lanczos(x);
    let tmp = x + LANCZOS_G + 0.5;
    Ok((x + 0.5) * fastmath::log(tmp) - tmp + half_log_2_pi() + fastmath::log(sum / x))
}

/// `Precision.equals(x, y, 1)`: equal within one ulp (and neither is NaN).
fn equals_ulp(x: f64, y: f64) -> bool {
    let xi = x.to_bits() as i64;
    let yi = y.to_bits() as i64;
    let eq = if ((xi ^ yi) & i64::MIN) == 0 {
        (xi - yi).abs() <= 1
    } else {
        let (dp, dm) = if xi < yi {
            (yi, xi.wrapping_sub(i64::MIN))
        } else {
            (xi, yi.wrapping_sub(i64::MIN))
        };
        dp <= 1 && dm <= 1 - dp
    };
    eq && !x.is_nan() && !y.is_nan()
}

/// `Precision.equals(x, 0)` (Brent's solver).
pub fn equals_zero(x: f64) -> bool {
    equals_ulp(x, 0.0)
}

/// `Precision.equals(x, y, eps)`.
fn equals_eps(x: f64, y: f64, eps: f64) -> bool {
    equals_ulp(x, y) || (y - x).abs() <= eps
}

/// `Gamma.regularizedGammaP(a, x)` (epsilon 1e-14, no iteration limit).
pub fn regularized_gamma_p(a: f64, x: f64) -> Result<f64, MathError> {
    regularized_gamma_p_with(a, x, 1.0E-14, i32::MAX)
}

/// `Gamma.regularizedGammaP(a, x, epsilon, maxIterations)`.
pub fn regularized_gamma_p_with(a: f64, x: f64, eps: f64, max: i32) -> Result<f64, MathError> {
    if a.is_nan() || x.is_nan() || a <= 0.0 || x < 0.0 {
        return Ok(f64::NAN);
    }
    if x == 0.0 {
        return Ok(0.0);
    }
    if x >= a + 1.0 {
        return Ok(1.0 - regularized_gamma_q_with(a, x, eps, max)?);
    }
    let mut n = 0.0;
    let mut an = 1.0 / a;
    let mut sum = an;
    while (an / sum).abs() > eps && n < f64::from(max) && sum < f64::INFINITY {
        n += 1.0;
        an *= x / (a + n);
        sum += an;
    }
    if n >= f64::from(max) {
        return Err(MathError::MaxCountExceeded);
    }
    if sum.is_infinite() {
        return Ok(1.0);
    }
    Ok(fastmath::exp(-x + a * fastmath::log(x) - log_gamma(a)?) * sum)
}

/// `Gamma.regularizedGammaQ(a, x, epsilon, maxIterations)`.
pub fn regularized_gamma_q_with(a: f64, x: f64, eps: f64, max: i32) -> Result<f64, MathError> {
    if a.is_nan() || x.is_nan() || a <= 0.0 || x < 0.0 {
        return Ok(f64::NAN);
    }
    if x == 0.0 {
        return Ok(1.0);
    }
    if x < a + 1.0 {
        return Ok(1.0 - regularized_gamma_p_with(a, x, eps, max)?);
    }
    // ContinuedFraction with a_n = (2n + 1) - a + x, b_n = n (a - n)
    let ret = 1.0 / continued_fraction(a, x, eps, max)?;
    Ok(fastmath::exp(-x + a * fastmath::log(x) - log_gamma(a)?) * ret)
}

/// `ContinuedFraction.evaluate(x, epsilon, maxIterations)` (modified Lentz) of the incomplete
/// gamma function's fraction.
fn continued_fraction(a: f64, x: f64, epsilon: f64, max: i32) -> Result<f64, MathError> {
    const SMALL: f64 = 1e-50;
    let get_a = |n: i32| (2.0 * f64::from(n) + 1.0) - a + x;
    let get_b = |n: i32| f64::from(n) * (a - f64::from(n));
    let mut h_prev = get_a(0);
    if equals_eps(h_prev, 0.0, SMALL) {
        h_prev = SMALL;
    }
    let mut n = 1;
    let mut d_prev = 0.0;
    let mut c_prev = h_prev;
    let mut hn = h_prev;
    while n < max {
        let an = get_a(n);
        let bn = get_b(n);
        let mut dn = an + bn * d_prev;
        if equals_eps(dn, 0.0, SMALL) {
            dn = SMALL;
        }
        let mut cn = an + bn / c_prev;
        if equals_eps(cn, 0.0, SMALL) {
            cn = SMALL;
        }
        dn = 1.0 / dn;
        let delta = cn * dn;
        hn = h_prev * delta;
        if hn.is_infinite() || hn.is_nan() {
            return Err(MathError::Convergence);
        }
        if (delta - 1.0).abs() < epsilon {
            break;
        }
        d_prev = dn;
        c_prev = cn;
        h_prev = hn;
        n += 1;
    }
    if n >= max {
        return Err(MathError::MaxCountExceeded);
    }
    Ok(hn)
}

/// `Erf.erfInv(x)`.
pub fn erf_inv(x: f64) -> f64 {
    let mut w = -fastmath::log((1.0 - x) * (1.0 + x));
    let mut p;
    if w < 6.25 {
        w -= 3.125;
        p = -3.64441206401782E-21;
        for c in [
            -1.6850591381820166E-19,
            1.28584807152564E-18,
            1.1157877678025181E-17,
            -1.333171662854621E-16,
            2.0972767875968562E-17,
            6.637638134358324E-15,
            -4.054566272975207E-14,
            -8.151934197605472E-14,
            2.6335093153082323E-12,
            -1.2975133253453532E-11,
            -5.415412054294628E-11,
            1.0512122733215323E-9,
            -4.112633980346984E-9,
            -2.9070369957882005E-8,
            4.2347877827932404E-7,
            -1.3654692000834679E-6,
            -1.3882523362786469E-5,
            1.8673420803405714E-4,
            -7.40702534166267E-4,
            -0.006033670871430149,
            0.24015818242558962,
            1.6536545626831027,
        ] {
            p = c + p * w;
        }
    } else if w < 16.0 {
        w = w.sqrt() - 3.25;
        p = 2.2137376921775787E-9;
        for c in [
            9.075656193888539E-8,
            -2.7517406297064545E-7,
            1.8239629214389228E-8,
            1.5027403968909828E-6,
            -4.013867526981546E-6,
            2.9234449089955446E-6,
            1.2475304481671779E-5,
            -4.7318229009055734E-5,
            6.828485145957318E-5,
            2.4031110387097894E-5,
            -3.550375203628475E-4,
            9.532893797373805E-4,
            -0.0016882755560235047,
            0.002491442096107851,
            -0.003751208507569241,
            0.005370914553590064,
            1.0052589676941592,
            3.0838856104922208,
        ] {
            p = c + p * w;
        }
    } else if !w.is_infinite() {
        w = w.sqrt() - 5.0;
        p = -2.7109920616438573E-11;
        for c in [
            -2.555641816996525E-10,
            1.5076572693500548E-9,
            -3.789465440126737E-9,
            7.61570120807834E-9,
            -1.496002662714924E-8,
            2.914795345090108E-8,
            -6.771199775845234E-8,
            2.2900482228026655E-7,
            -9.9298272942317E-7,
            4.526062597223154E-6,
            -1.968177810553167E-5,
            7.599527703001776E-5,
            -2.1503011930044477E-4,
            -1.3871931833623122E-4,
            1.0103004648645344,
            4.849906401408584,
        ] {
            p = c + p * w;
        }
    } else {
        p = f64::INFINITY;
    }
    p * x
}

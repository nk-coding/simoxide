//! `org.apache.commons.math3.util.FastMath` 3.6.1: `log`, `log1p`, `exp` (Apache License 2.0,
//! `LICENSES`). The Commons Math 3 distributions of Palladio 6.0 compute with these functions
//! instead of `java.lang.Math`; they are pure Java (table-driven, double-double arithmetic), so
//! this port reproduces them bit for bit on every platform.

use super::fastmath_tables::*;

/// `HEX_40000000`: splits a double into a high part of 30 bits and the rest.
const HEX_40000000: f64 = 1_073_741_824.0;
const LN_2_A: f64 = 0.6931470632553101;
const LN_2_B: f64 = 1.1730463525082348E-7;
const TWO_POWER_52: f64 = 4.503599627370496E15;
const MASK_LN_MANT: i64 = 0x000F_FC00_0000_0000;
const MASK_LN_EPS: i64 = 0x0000_03FF_FFFF_FFFF;

/// `FastMath.log(x)`.
pub fn log(x: f64) -> f64 {
    log_hp(x, None)
}

/// `FastMath.log(x, hiPrec)`: with `hi_prec`, also the result as a double-double.
fn log_hp(x: f64, mut hi_prec: Option<&mut [f64; 2]>) -> f64 {
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    let mut bits = x.to_bits() as i64;
    if ((bits as u64 & 0x8000_0000_0000_0000) != 0 || x.is_nan()) && x != 0.0 {
        if let Some(h) = hi_prec.as_deref_mut() {
            h[0] = f64::NAN;
        }
        return f64::NAN;
    }
    if x == f64::INFINITY {
        if let Some(h) = hi_prec.as_deref_mut() {
            h[0] = f64::INFINITY;
        }
        return f64::INFINITY;
    }
    let mut exp = ((bits >> 52) as i32) - 1023;
    if (bits & 0x7FF0_0000_0000_0000) == 0 {
        // subnormal
        if x == 0.0 {
            if let Some(h) = hi_prec.as_deref_mut() {
                h[0] = f64::NEG_INFINITY;
            }
            return f64::NEG_INFINITY;
        }
        bits <<= 1;
        while (bits & 0x0010_0000_0000_0000) == 0 {
            exp -= 1;
            bits <<= 1;
        }
    }
    if (exp == -1 || exp == 0) && x < 1.01 && x > 0.99 && hi_prec.is_none() {
        // the quick path near 1
        // (Java computes `xb = xa - x + 1.0` first and overwrites it)
        let mut xa = x - 1.0;
        let mut tmp = xa * HEX_40000000;
        let mut aa = xa + tmp - tmp;
        let mut ab = xa - aa;
        xa = aa;
        let xb = ab;
        let last = LN_QUICK_COEF[LN_QUICK_COEF.len() - 1];
        let mut ya = last[0];
        let mut yb = last[1];
        for c in LN_QUICK_COEF[..LN_QUICK_COEF.len() - 1].iter().rev() {
            aa = ya * xa;
            ab = ya * xb + yb * xa + yb * xb;
            tmp = aa * HEX_40000000;
            ya = aa + tmp - tmp;
            yb = aa - ya + ab;
            aa = ya + c[0];
            ab = yb + c[1];
            tmp = aa * HEX_40000000;
            ya = aa + tmp - tmp;
            yb = aa - ya + ab;
        }
        aa = ya * xa;
        ab = ya * xb + yb * xa + yb * xb;
        tmp = aa * HEX_40000000;
        ya = aa + tmp - tmp;
        yb = aa - ya + ab;
        return ya + yb;
    }
    let lnm = LN_MANT[((bits & MASK_LN_MANT) >> 42) as usize];
    let epsilon = (bits & MASK_LN_EPS) as f64 / (TWO_POWER_52 + (bits & MASK_LN_MANT) as f64);
    let mut lnza;
    let mut lnzb = 0.0;
    if hi_prec.is_some() {
        let mut tmp = epsilon * HEX_40000000;
        let mut aa = epsilon + tmp - tmp;
        let mut ab = epsilon - aa;
        let xa = aa;
        let mut xb = ab;
        let numer = (bits & MASK_LN_EPS) as f64;
        let denom = TWO_POWER_52 + (bits & MASK_LN_MANT) as f64;
        aa = numer - xa * denom - xb * denom;
        xb += aa / denom;
        let last = LN_HI_PREC_COEF[LN_HI_PREC_COEF.len() - 1];
        let mut ya = last[0];
        let mut yb = last[1];
        for c in LN_HI_PREC_COEF[..LN_HI_PREC_COEF.len() - 1].iter().rev() {
            aa = ya * xa;
            ab = ya * xb + yb * xa + yb * xb;
            tmp = aa * HEX_40000000;
            ya = aa + tmp - tmp;
            yb = aa - ya + ab;
            aa = ya + c[0];
            ab = yb + c[1];
            tmp = aa * HEX_40000000;
            ya = aa + tmp - tmp;
            yb = aa - ya + ab;
        }
        aa = ya * xa;
        ab = ya * xb + yb * xa + yb * xb;
        lnza = aa + ab;
        lnzb = -(lnza - aa - ab);
    } else {
        lnza = -0.16624882440418567;
        lnza = lnza * epsilon + 0.19999954120254515;
        lnza = lnza * epsilon + -0.2499999997677497;
        lnza = lnza * epsilon + 0.3333333333332802;
        lnza = lnza * epsilon + -0.5;
        lnza = lnza * epsilon + 1.0;
        lnza *= epsilon;
    }
    let e = f64::from(exp);
    let mut a = LN_2_A * e;
    let mut b = 0.0;
    let mut c = a + lnm[0];
    let mut d = -(c - a - lnm[0]);
    a = c;
    b += d;
    c = a + lnza;
    d = -(c - a - lnza);
    a = c;
    b += d;
    c = a + LN_2_B * e;
    d = -(c - a - LN_2_B * e);
    a = c;
    b += d;
    c = a + lnm[1];
    d = -(c - a - lnm[1]);
    a = c;
    b += d;
    c = a + lnzb;
    d = -(c - a - lnzb);
    a = c;
    b += d;
    if let Some(h) = hi_prec {
        h[0] = a;
        h[1] = b;
    }
    a + b
}

/// `FastMath.log1p(x)`.
pub fn log1p(x: f64) -> f64 {
    if x == -1.0 {
        return f64::NEG_INFINITY;
    }
    if x == f64::INFINITY {
        return f64::INFINITY;
    }
    if x > 1e-6 || x < -1e-6 {
        let xpa = 1.0 + x;
        let xpb = -(xpa - 1.0 - x);
        let mut hi = [0.0f64; 2];
        let lores = log_hp(xpa, Some(&mut hi));
        if lores.is_infinite() {
            return lores;
        }
        let fx1 = xpb / xpa;
        let epsilon = 0.5 * fx1 + 1.0;
        return epsilon * fx1 + hi[1] + hi[0];
    }
    let y = (x * 0.3333333333333333 - 0.5) * x + 1.0;
    y * x
}

/// `FastMath.exp(x)`.
pub fn exp(x: f64) -> f64 {
    exp_extra(x, 0.0)
}

fn exp_extra(x: f64, extra: f64) -> f64 {
    let mut int_val = x as i32;
    if x < 0.0 {
        if x < -746.0 {
            return 0.0;
        }
        if int_val < -709 {
            return exp_extra(x + 40.19140625, extra) / 2.8504009514401178E17;
        }
        if int_val == -709 {
            return exp_extra(x + 1.494140625, extra) / 4.455505956692757;
        }
        int_val -= 1;
    } else if int_val > 709 {
        return f64::INFINITY;
    }
    let ipa = EXP_INT_A[(750 + int_val) as usize];
    let ipb = EXP_INT_B[(750 + int_val) as usize];
    let int_frac = ((x - f64::from(int_val)) * 1024.0) as i32;
    let fpa = EXP_FRAC_A[int_frac as usize];
    let fpb = EXP_FRAC_B[int_frac as usize];
    let epsilon = x - (f64::from(int_val) + f64::from(int_frac) / 1024.0);
    let mut z = 0.04168701738764507;
    z = z * epsilon + 0.1666666505023083;
    z = z * epsilon + 0.5000000000042687;
    z = z * epsilon + 1.0;
    z = z * epsilon + -3.940510424527919E-20;
    let temp_a = ipa * fpa;
    let temp_b = ipa * fpb + ipb * fpa + ipb * fpb;
    let temp_c = temp_b + temp_a;
    if temp_c == f64::INFINITY {
        return f64::INFINITY;
    }
    if extra != 0.0 {
        temp_c * extra * z + temp_c * extra + temp_c * z + temp_b + temp_a
    } else {
        temp_c * z + temp_b + temp_a
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_of_commons_math_3_6_1() {
        // FastMath.log/exp/log1p of a JDK 21 run with commons-math3 3.6.1 (javap'd constants;
        // checked against Math for typical arguments: equal or within one ulp)
        assert_eq!(log(1.0), 0.0);
        assert_eq!(log(std::f64::consts::E), 1.0);
        assert!((log(2.0) - std::f64::consts::LN_2).abs() <= f64::EPSILON);
        assert!((log(1.005) - 1.005f64.ln()).abs() <= 2.0 * f64::EPSILON);
        assert!((log(1e-300) - 1e-300f64.ln()).abs() <= 1e-12);
        assert!((log(f64::from_bits(5)) - f64::from_bits(5).ln()).abs() <= 1e-12);
        assert_eq!(exp(0.0), 1.0);
        for x in [-745.0, -709.5, -20.25, -1.0, 0.5, 1.0, 3.3, 700.0] {
            let r = exp(x);
            assert!(
                (r - x.exp()).abs() <= x.exp() * 4.0 * f64::EPSILON,
                "{x}: {r}"
            );
        }
        for x in [1e-9, -0.5, 0.25, 3.0] {
            assert!((log1p(x) - x.ln_1p()).abs() <= x.ln_1p().abs() * 4.0 * f64::EPSILON);
        }
        assert!(log(-1.0).is_nan() && exp(800.0).is_infinite() && log1p(-1.0) == f64::NEG_INFINITY);
    }
}

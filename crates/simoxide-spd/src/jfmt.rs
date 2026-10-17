//! Java number formatting for Slingshot's messages and the decision log.

/// Java `String.format("%f", x)`: six fraction digits, rounded half-up from the shortest decimal
/// representation of `x` (as `java.util.Formatter` does), `NaN` and `Infinity` spelled out.
pub fn f6(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "Infinity" } else { "-Infinity" }.into();
    }
    // shortest digits and exponent: d.ddd e E
    let e = format!("{:e}", x.abs());
    let (mant, exp) = e.split_once('e').expect("exponent");
    let exp: i32 = exp.parse().expect("exponent");
    let digits: Vec<u8> = mant
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    // value = 0.d1 d2 ... * 10^(exp + 1); position of the decimal point in `digits`
    let point = exp + 1;
    // digits of the integer part and of the fraction, padded
    let mut int_part: Vec<u8> = Vec::new();
    let mut frac: Vec<u8> = Vec::new();
    for (i, &d) in digits.iter().enumerate() {
        if (i as i32) < point {
            int_part.push(d);
        } else {
            frac.push(d);
        }
    }
    if point > digits.len() as i32 {
        int_part.extend(std::iter::repeat_n(
            0,
            (point - digits.len() as i32) as usize,
        ));
    }
    if point < 0 {
        let mut f = vec![0u8; (-point) as usize];
        f.extend(frac);
        frac = f;
    }
    if int_part.is_empty() {
        int_part.push(0);
    }
    let round_up = frac.len() > 6 && frac[6] >= 5;
    frac.resize(6, 0);
    if round_up {
        let mut i = 6;
        loop {
            if i == 0 {
                // carry into the integer part
                let mut j = int_part.len();
                loop {
                    if j == 0 {
                        int_part.insert(0, 1);
                        break;
                    }
                    j -= 1;
                    if int_part[j] == 9 {
                        int_part[j] = 0;
                    } else {
                        int_part[j] += 1;
                        break;
                    }
                }
                break;
            }
            i -= 1;
            if frac[i] == 9 {
                frac[i] = 0;
            } else {
                frac[i] += 1;
                break;
            }
        }
    }
    let mut s = String::new();
    if x.is_sign_negative() {
        s.push('-');
    }
    s.extend(int_part.iter().map(|d| (b'0' + d) as char));
    s.push('.');
    s.extend(frac.iter().map(|d| (b'0' + d) as char));
    s
}

/// A double as JSON number in the style of Java's `Double.toString` for ordinary magnitudes
/// (`50.0`, `0.25`); shortest round-trip digits otherwise. Non-finite values become `null`.
pub fn json_double(x: f64) -> String {
    if !x.is_finite() {
        return "null".into();
    }
    if x == x.trunc() && x.abs() < 1e7 {
        return format!("{x:.1}");
    }
    let mut b = ryu::Buffer::new();
    b.format_finite(x).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_f() {
        assert_eq!(f6(50.0), "50.000000");
        assert_eq!(f6(80.0), "80.000000");
        assert_eq!(f6(0.0), "0.000000");
        assert_eq!(f6(-0.0), "-0.000000");
        assert_eq!(f6(1.5), "1.500000");
        assert_eq!(f6(0.0000005), "0.000001");
        assert_eq!(f6(0.00000049), "0.000000");
        assert_eq!(f6(123456.1234565), "123456.123457");
        assert_eq!(f6(9.9999995), "10.000000");
        assert_eq!(f6(1e20), "100000000000000000000.000000");
        assert_eq!(f6(-2.25), "-2.250000");
        assert_eq!(f6(f64::NAN), "NaN");
    }

    #[test]
    fn json() {
        assert_eq!(json_double(50.0), "50.0");
        assert_eq!(json_double(0.25), "0.25");
        assert_eq!(json_double(1e300), "1e300");
    }
}

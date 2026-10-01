//! Java number and string formatting used by the trace, tape and measurement files
//! (`docs/guide/formats.md` §1).

use std::fmt::Write as _;

/// Appends `Double.toString(x)` (JDK ≥ 19: shortest round-trip digits) to `out`.
///
/// `1e-3 <= |x| < 1e7` is written as a plain decimal with at least one fractional digit,
/// everything else as `d.ddd` + `E` + exponent. NaN and infinities are written as Java prints
/// them (`NaN`, `Infinity`); the JSON writers quote them.
pub fn push_double(out: &mut String, x: f64) {
    if x.is_nan() {
        out.push_str("NaN");
        return;
    }
    if x.is_infinite() {
        out.push_str(if x > 0.0 { "Infinity" } else { "-Infinity" });
        return;
    }
    if x == 0.0 {
        out.push_str(if x.is_sign_negative() { "-0.0" } else { "0.0" });
        return;
    }
    // Shortest round-trip digits (ties to even, like Java) and the decimal exponent from ryu.
    let mut rb = ryu::Buffer::new();
    let ax = x.abs();
    let (mut digits, mut exp) = parse_ryu(rb.format_finite(ax));
    if digits.len() == 1 && ax < f64::MIN_POSITIVE {
        // Java renders at least two digits: a one-digit shortest decimal (subnormals only) is
        // replaced by the two-digit one closest to x (`Double.MIN_VALUE` = `4.9E-324`).
        (digits, exp) = two_digit_subnormal(ax, exp);
    }
    if x < 0.0 {
        out.push('-');
    }
    if (1e-3..1e7).contains(&ax) {
        // position of the decimal point: number of digits before it
        let p = exp + 1;
        if p <= 0 {
            out.push_str("0.");
            for _ in 0..(-p) {
                out.push('0');
            }
            out.push_str(&digits);
        } else {
            let p = p as usize;
            if p >= digits.len() {
                out.push_str(&digits);
                for _ in 0..(p - digits.len()) {
                    out.push('0');
                }
                out.push_str(".0");
            } else {
                out.push_str(&digits[..p]);
                out.push('.');
                out.push_str(&digits[p..]);
            }
        }
    } else {
        out.push_str(&digits[..1]);
        out.push('.');
        if digits.len() > 1 {
            out.push_str(&digits[1..]);
        } else {
            out.push('0');
        }
        out.push('E');
        let _ = write!(out, "{exp}");
    }
}

/// The two-digit decimal `c * 10^(k-1)` (`10 <= c <= 100`, `10^k <= x < 10^(k+1)`) that Java's
/// `Double.toString` picks for a subnormal `x` whose shortest decimal `d * 10^k1` has one digit
/// (`k1` is `k` or `k + 1`, e.g. `1.0E-323` for `2 * MIN_VALUE` = 9.88e-324): of the two grid
/// neighbours of `x`, the closer one that still rounds to `x` (the javadoc rule; `simoxide_testkit::javafmt::
/// exact` is the reference). Exact ties are impossible: `x` is `m * 2^-1074` and a midpoint of
/// the grid has a factor `5^(1-k)` in its denominator. Returns significant digits and the
/// scientific exponent like [`parse_ryu`].
#[cold]
fn two_digit_subnormal(x: f64, k1: i32) -> (String, i32) {
    // In units of 2^-1075 (half an ulp): x = 2m, rounding interval [2m - 1, 2m + 1] (closed for
    // even m). A grid point c * 10^e (e = k - 1 < 0) is c * 2^1075 / 10^-e units.
    let m = (x.to_bits() & ((1u64 << 52) - 1)) as u128;
    // k: 10^k <= x, i.e. 2^1075 <= 2m * 10^-k
    let k = if big(1, 1075, 0).cmp_to(&big(2 * m, 0, (-k1) as u32)).is_le() {
        k1
    } else {
        k1 - 1
    };
    let e = k - 1;
    debug_assert!(e < 0);
    let p10 = (-e) as u32;
    // sign of (c * 2^1075) - (n * 10^p10)
    let cmp = |c: u128, n: u128| big(c, 1075, 0).cmp_to(&big(n, 0, p10));
    // largest c with c * 10^e <= x
    let mut c = 10u128;
    while cmp(c + 1, 2 * m) != std::cmp::Ordering::Greater {
        c += 1;
    }
    let in_r = |c: u128| {
        let lo = cmp(c, 2 * m - 1);
        let hi = cmp(c, 2 * m + 1);
        if m.is_multiple_of(2) {
            lo.is_ge() && hi.is_le()
        } else {
            lo.is_gt() && hi.is_lt()
        }
    };
    // closer neighbour: 2x vs (2c + 1) * 10^e, i.e. (2c + 1) * 2^1075 vs 4m * 10^p10
    let upper_closer = cmp(2 * c + 1, 4 * m).is_lt();
    let pick = match (in_r(c), in_r(c + 1)) {
        (true, true) => {
            if upper_closer {
                c + 1
            } else {
                c
            }
        }
        (true, false) => c,
        (false, true) => c + 1,
        (false, false) => unreachable!("the one-digit decimal is a grid point in the interval"),
    };
    let s = pick.to_string();
    let digits = s.trim_end_matches('0').to_string();
    (digits, e + s.len() as i32 - 1)
}

/// `small * 2^shl * 10^pow10` as little-endian `u32` limbs.
fn big(small: u128, shl: u32, pow10: u32) -> Big {
    let mut l: Vec<u32> = Vec::new();
    let mut v = small;
    while v > 0 {
        l.push(v as u32);
        v >>= 32;
    }
    let mul = |l: &mut Vec<u32>, f: u32| {
        let mut carry = 0u64;
        for x in l.iter_mut() {
            let p = u64::from(*x) * u64::from(f) + carry;
            *x = p as u32;
            carry = p >> 32;
        }
        if carry > 0 {
            l.push(carry as u32);
        }
    };
    let mut n = pow10;
    while n >= 9 {
        mul(&mut l, 1_000_000_000);
        n -= 9;
    }
    if n > 0 {
        mul(&mut l, 10u32.pow(n));
    }
    let mut b = shl;
    while b >= 31 {
        mul(&mut l, 1 << 31);
        b -= 31;
    }
    if b > 0 {
        mul(&mut l, 1 << b);
    }
    while l.last() == Some(&0) {
        l.pop();
    }
    Big(l)
}

struct Big(Vec<u32>);

impl Big {
    fn cmp_to(&self, o: &Big) -> std::cmp::Ordering {
        self.0
            .len()
            .cmp(&o.0.len())
            .then_with(|| self.0.iter().rev().cmp(o.0.iter().rev()))
    }
}

/// Parses ryu's output (`1.5e-7`, `1e16`, `0.001`, `123.0`) into significant digits (no leading
/// or trailing zeros) and the scientific exponent of the first digit.
fn parse_ryu(s: &str) -> (String, i32) {
    let (mant, exp) = match s.split_once('e') {
        Some((m, e)) => (m, e.parse::<i32>().expect("exponent")),
        None => (s, 0),
    };
    let (int, frac) = mant.split_once('.').unwrap_or((mant, ""));
    let mut all = String::with_capacity(int.len() + frac.len());
    all.push_str(int);
    all.push_str(frac);
    // value = 0.all * 10^(int.len() + exp)
    let lead = all.bytes().take_while(|&b| b == b'0').count();
    let digits = all[lead..].trim_end_matches('0').to_string();
    let sci = int.len() as i32 + exp - lead as i32 - 1;
    (digits, sci)
}

/// `Double.toString(x)` as a new string.
pub fn double(x: f64) -> String {
    let mut s = String::new();
    push_double(&mut s, x);
    s
}

/// Appends a JSON number in the trace format: `Double.toString`, NaN/infinities quoted.
pub fn push_json_double(out: &mut String, x: f64) {
    if x.is_finite() {
        push_double(out, x);
    } else {
        out.push('"');
        push_double(out, x);
        out.push('"');
    }
}

/// Appends a JSON string literal with the escapes of the reference writer.
pub fn push_json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::double;

    #[test]
    fn java_double_to_string() {
        assert_eq!(double(3.0), "3.0");
        assert_eq!(double(0.018428198), "0.018428198");
        assert_eq!(double(1493559.946308255), "1493559.946308255");
        assert_eq!(double(1.0e7), "1.0E7");
        assert_eq!(double(1.0e-4), "1.0E-4");
        assert_eq!(double(7.212694342041994E-4), "7.212694342041994E-4");
        assert_eq!(double(0.001), "0.001");
        assert_eq!(double(-0.0), "-0.0");
        assert_eq!(double(0.0), "0.0");
        assert_eq!(double(100.0), "100.0");
        assert_eq!(double(1e21), "1.0E21");
        assert_eq!(double(123456.7), "123456.7");
        assert_eq!(double(-2.5e-10), "-2.5E-10");
        assert_eq!(double(0.28699461400000004), "0.28699461400000004");
        assert_eq!(double(f64::NAN), "NaN");
        assert_eq!(double(9999999.0), "9999999.0");
        // one-digit subnormals get two digits (the closest to x)
        assert_eq!(double(f64::from_bits(1)), "4.9E-324");
        assert_eq!(double(-f64::from_bits(1)), "-4.9E-324");
        assert_eq!(double(f64::from_bits(2)), "9.9E-324");
        assert_eq!(double(f64::from_bits(20)), "9.9E-323");
        // exact ties between two shortest candidates go to the even digit (Java, ryu)
        assert_eq!(
            double(f64::from_bits(0x4000_27c4_0000_0000)),
            "2.0194168090820312"
        );
    }
}

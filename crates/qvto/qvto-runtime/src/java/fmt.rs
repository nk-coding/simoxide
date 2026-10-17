//! Java's `Double.toString`, `Float.toString` (JDK 19 and later: Raffaello Giulietti's
//! shortest-decimal specification) and integer formatting.
//!
//! Adapted from SimOxide's `simoxide-testkit/src/javafmt.rs` (EPL-2.0, same author). The fast
//! path takes the shortest round-tripping digits from `ryu`; Java's rule only differs from
//! "shortest, closest, ties to even" when the shortest decimal has one digit (Java then also
//! admits two-digit decimals, e.g. `Double.MIN_VALUE` is `4.9E-324`), which only happens for
//! subnormals and is delegated to [`exact`], a literal implementation of the javadoc with
//! big-integer arithmetic that also serves as the property-test oracle.
//!
//! JDK 18 and earlier printed some values with more digits than needed (JDK-4511638, e.g.
//! `2.0E23` as `1.9999999999999998E23`). That formatter is not reproduced (its only
//! specification is OpenJDK's GPL source); see `DESIGN.md` §12.

use std::cmp::Ordering;

/// Shortest decimal `digits * 10^(sci_exp - ndigits + 1)`, i.e. `d.ddd * 10^sci_exp`, `digits`
/// without trailing zeros.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decimal {
    pub digits: u64,
    pub ndigits: u32,
    pub sci_exp: i32,
}

/// `Double.toString(x)`.
pub fn double_to_string(x: f64) -> String {
    let mut s = String::with_capacity(24);
    push_double(&mut s, x);
    s
}

/// Appends `Double.toString(x)`.
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
    if x < 0.0 {
        out.push('-');
    }
    let ax = x.abs();
    let mut buf = ryu::Buffer::new();
    let mut d = parse_ryu(buf.format_finite(ax));
    if d.ndigits == 1 && ax < f64::MIN_POSITIVE {
        d = exact::shortest(ax);
    }
    format_decimal(out, d);
}

/// `Float.toString(x)`.
pub fn float_to_string(x: f32) -> String {
    let mut s = String::with_capacity(16);
    push_float(&mut s, x);
    s
}

/// Appends `Float.toString(x)`.
pub fn push_float(out: &mut String, x: f32) {
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
    if x < 0.0 {
        out.push('-');
    }
    let ax = x.abs();
    let mut buf = ryu::Buffer::new();
    let mut d = parse_ryu(buf.format_finite(ax));
    if d.ndigits == 1 && ax < f32::MIN_POSITIVE {
        d = exact::shortest_f32(ax);
    }
    format_decimal(out, d);
}

/// `Integer.toString` / `Long.toString`.
pub fn push_long(out: &mut String, v: i64) {
    use std::fmt::Write;
    let _ = write!(out, "{v}");
}

/// Parses ryu's output (`1.5e-7`, `1e16`, `0.001`, `123.0`) into a normalized [`Decimal`].
fn parse_ryu(s: &str) -> Decimal {
    let b = s.as_bytes();
    let mut digits: u64 = 0;
    let mut frac_digits: i32 = 0;
    let mut in_frac = false;
    let mut i = 0;
    let mut exp: i32 = 0;
    while i < b.len() {
        match b[i] {
            b'0'..=b'9' => {
                digits = digits * 10 + (b[i] - b'0') as u64;
                if in_frac {
                    frac_digits += 1;
                }
            }
            b'.' => in_frac = true,
            b'e' | b'E' => {
                let (neg, start) = if b.get(i + 1) == Some(&b'-') {
                    (true, i + 2)
                } else {
                    (false, i + 1)
                };
                let mut e: i32 = 0;
                for &c in &b[start..] {
                    e = e * 10 + (c - b'0') as i32;
                }
                exp = if neg { -e } else { e };
                break;
            }
            _ => {}
        }
        i += 1;
    }
    let mut e10 = exp - frac_digits;
    while digits.is_multiple_of(10) {
        digits /= 10;
        e10 += 1;
    }
    let n = ndigits(digits);
    Decimal {
        digits,
        ndigits: n,
        sci_exp: e10 + n as i32 - 1,
    }
}

fn ndigits(mut v: u64) -> u32 {
    let mut n = 1;
    while v >= 10 {
        v /= 10;
        n += 1;
    }
    n
}

/// Java's layout: plain for `1e-3 <= d < 1e7`, else computerized scientific notation.
pub fn format_decimal(out: &mut String, d: Decimal) {
    let mut tmp = [0u8; 20];
    let n = d.ndigits as usize;
    let mut v = d.digits;
    for i in (0..n).rev() {
        tmp[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    let s = std::str::from_utf8(&tmp[..n]).expect("digits");
    let k = d.sci_exp;
    if (-3..7).contains(&k) {
        if k >= 0 {
            let int_len = k as usize + 1;
            if n <= int_len {
                out.push_str(s);
                out.extend(std::iter::repeat_n('0', int_len - n));
                out.push_str(".0");
            } else {
                out.push_str(&s[..int_len]);
                out.push('.');
                out.push_str(&s[int_len..]);
            }
        } else {
            out.push_str("0.");
            out.extend(std::iter::repeat_n('0', (-k - 1) as usize));
            out.push_str(s);
        }
    } else {
        out.push_str(&s[..1]);
        out.push('.');
        if n > 1 {
            out.push_str(&s[1..]);
        } else {
            out.push('0');
        }
        out.push('E');
        push_long(out, k as i64);
    }
}

pub mod exact {
    use super::*;

    /// Minimal unsigned big integer (little-endian u32 limbs).
    #[derive(Clone, Debug)]
    struct Big(Vec<u32>);

    impl Big {
        fn from_u128(mut v: u128) -> Big {
            let mut l = Vec::new();
            while v > 0 {
                l.push(v as u32);
                v >>= 32;
            }
            Big(l)
        }
        fn mul_small(&mut self, m: u32) {
            let mut carry: u64 = 0;
            for x in self.0.iter_mut() {
                let p = *x as u64 * m as u64 + carry;
                *x = p as u32;
                carry = p >> 32;
            }
            if carry > 0 {
                self.0.push(carry as u32);
            }
        }
        fn shl(&mut self, bits: u32) {
            let words = (bits / 32) as usize;
            let b = bits % 32;
            if b > 0 {
                let mut carry = 0u32;
                for x in self.0.iter_mut() {
                    let nx = (*x << b) | carry;
                    carry = *x >> (32 - b);
                    *x = nx;
                }
                if carry > 0 {
                    self.0.push(carry);
                }
            }
            if words > 0 && !self.0.is_empty() {
                let mut v = vec![0u32; words];
                v.append(&mut self.0);
                self.0 = v;
            }
        }
        fn mul_pow10(&mut self, mut n: u32) {
            while n >= 9 {
                self.mul_small(1_000_000_000);
                n -= 9;
            }
            if n > 0 {
                self.mul_small(10u32.pow(n));
            }
        }
        fn trim(&mut self) {
            while self.0.last() == Some(&0) {
                self.0.pop();
            }
        }
        fn cmp(&self, o: &Big) -> Ordering {
            let (mut a, mut b) = (self.clone(), o.clone());
            a.trim();
            b.trim();
            if a.0.len() != b.0.len() {
                return a.0.len().cmp(&b.0.len());
            }
            for i in (0..a.0.len()).rev() {
                match a.0[i].cmp(&b.0[i]) {
                    Ordering::Equal => {}
                    o => return o,
                }
            }
            Ordering::Equal
        }
    }

    /// Compares `a * 2^a2 * 10^a10` with `b * 2^b2 * 10^b10`.
    fn cmp_scaled(a: u128, a2: i32, a10: i32, b: u128, b2: i32, b10: i32) -> Ordering {
        let m2 = a2.min(b2);
        let m10 = a10.min(b10);
        let mut x = Big::from_u128(a);
        x.shl((a2 - m2) as u32);
        x.mul_pow10((a10 - m10) as u32);
        let mut y = Big::from_u128(b);
        y.shl((b2 - m2) as u32);
        y.mul_pow10((b10 - m10) as u32);
        x.cmp(&y)
    }

    /// x = f * 2^q with the rounding interval [lo, hi] = [(4f - dl) * 2^(q-2), (4f + 2) * 2^(q-2)].
    struct Val {
        f: u128,
        q: i32,
        dl: u128,
        inclusive: bool,
    }

    impl Val {
        fn new_f32(x: f32) -> Val {
            let bits = x.to_bits();
            let be = ((bits >> 23) & 0xff) as i32;
            let m = (bits & ((1u32 << 23) - 1)) as u128;
            let (f, q) = if be == 0 {
                (m, -149)
            } else {
                (m | (1u128 << 23), be - 150)
            };
            let dl = if m == 0 && be > 1 { 1 } else { 2 };
            Val {
                f,
                q,
                dl,
                inclusive: f % 2 == 0,
            }
        }
        fn new(x: f64) -> Val {
            let bits = x.to_bits();
            let be = ((bits >> 52) & 0x7ff) as i32;
            let m = (bits & ((1u64 << 52) - 1)) as u128;
            let (f, q) = if be == 0 {
                (m, -1074)
            } else {
                (m | (1u128 << 52), be - 1075)
            };
            let dl = if m == 0 && be > 1 { 1 } else { 2 };
            Val {
                f,
                q,
                dl,
                inclusive: f % 2 == 0,
            }
        }
        /// c * 10^e compared with x.
        fn cmp_x(&self, c: u128, e: i32) -> Ordering {
            cmp_scaled(c, 0, e, self.f, self.q, 0)
        }
        fn in_r(&self, c: u128, e: i32) -> bool {
            let lo = cmp_scaled(c, 0, e, 4 * self.f - self.dl, self.q - 2, 0);
            let hi = cmp_scaled(c, 0, e, 4 * self.f + 2, self.q - 2, 0);
            if self.inclusive {
                lo != Ordering::Less && hi != Ordering::Greater
            } else {
                lo == Ordering::Greater && hi == Ordering::Less
            }
        }
        /// floor(x / 10^e)
        fn floor_div(&self, x: f64, e: i32) -> u128 {
            let est = if e >= 0 {
                x / 10f64.powi(e)
            } else if e >= -300 {
                x * 10f64.powi(-e)
            } else {
                x * 1e300 * 10f64.powi(-e - 300)
            }
            .floor();
            let mut c: u128 = if est.is_finite() && est > 0.0 {
                est as u128
            } else {
                0
            };
            while c > 0 && self.cmp_x(c, e) == Ordering::Greater {
                c -= 1;
            }
            while self.cmp_x(c + 1, e) != Ordering::Greater {
                c += 1;
            }
            c
        }
        /// Of the grid neighbours c, c+1 (both in R), the one closest to x; ties to even.
        fn closest(&self, c: u128, e: i32) -> u128 {
            // x - c*10^e vs (c+1)*10^e - x  <=>  2x vs (2c+1)*10^e
            match cmp_scaled(self.f, self.q + 1, 0, 2 * c + 1, 0, e) {
                Ordering::Less => c,
                Ordering::Greater => c + 1,
                Ordering::Equal => {
                    if c.is_multiple_of(2) {
                        c
                    } else {
                        c + 1
                    }
                }
            }
        }
        fn pick(&self, x: f64, e: i32) -> Option<u128> {
            let c = self.floor_div(x, e);
            let a = c > 0 && self.in_r(c, e);
            let b = self.in_r(c + 1, e);
            match (a, b) {
                (true, true) => Some(self.closest(c, e)),
                (true, false) => Some(c),
                (false, true) => Some(c + 1),
                (false, false) => None,
            }
        }
    }

    /// Java's decimal for a finite positive `x` (slow).
    pub fn shortest(x: f64) -> Decimal {
        assert!(x.is_finite() && x > 0.0);
        shortest_val(&Val::new(x), x)
    }

    /// Java's decimal for a finite positive `float` (slow).
    pub fn shortest_f32(x: f32) -> Decimal {
        assert!(x.is_finite() && x > 0.0);
        shortest_val(&Val::new_f32(x), x as f64)
    }

    fn shortest_val(v: &Val, x: f64) -> Decimal {
        // k with 10^k <= x < 10^(k+1)
        let mut k = x.log10().floor() as i32;
        while v.cmp_x(1, k) == Ordering::Greater {
            k -= 1;
        }
        while v.cmp_x(1, k + 1) != Ordering::Greater {
            k += 1;
        }
        for len in 1..=18 {
            let e = k - len + 1;
            if v.pick(x, e).is_some() {
                let (c, e) = if len == 1 {
                    let e2 = k - 1;
                    (
                        v.pick(x, e2)
                            .expect("1-digit in R implies a 2-digit grid point"),
                        e2,
                    )
                } else {
                    (v.pick(x, e).unwrap(), e)
                };
                return normalize(c, e);
            }
        }
        unreachable!("17 digits always suffice")
    }

    fn normalize(mut c: u128, mut e: i32) -> Decimal {
        while c.is_multiple_of(10) {
            c /= 10;
            e += 1;
        }
        let c = c as u64;
        let n = ndigits(c);
        Decimal {
            digits: c,
            ndigits: n,
            sci_exp: e + n as i32 - 1,
        }
    }

    /// `Double.toString` computed with [`shortest`] only.
    pub fn to_string(x: f64) -> String {
        if !x.is_finite() || x == 0.0 {
            return super::double_to_string(x);
        }
        let mut out = String::new();
        if x < 0.0 {
            out.push('-');
        }
        format_decimal(&mut out, shortest(x.abs()));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        let cases: &[(f64, &str)] = &[
            (0.0, "0.0"),
            (-0.0, "-0.0"),
            (1.0, "1.0"),
            (100.0, "100.0"),
            (0.001, "0.001"),
            (0.0009999999999999998, "9.999999999999998E-4"),
            (1.0e7, "1.0E7"),
            (9999999.999999998, "9999999.999999998"),
            (1.0e-4, "1.0E-4"),
            (f64::MIN_POSITIVE, "2.2250738585072014E-308"),
            (f64::MAX, "1.7976931348623157E308"),
            (5e-324, "4.9E-324"),
            (1e23, "1.0E23"),
            (2e23, "2.0E23"),
            (2e-323, "2.0E-323"),
            (f64::NAN, "NaN"),
            (f64::INFINITY, "Infinity"),
            (-1.5, "-1.5"),
            (123456789.0, "1.23456789E8"),
        ];
        for &(x, s) in cases {
            assert_eq!(double_to_string(x), s, "{x:e}");
            assert_eq!(exact::to_string(x), s, "exact {x:e}");
        }
        assert_eq!(float_to_string(1.0), "1.0");
        assert_eq!(float_to_string(0.1), "0.1");
        assert_eq!(float_to_string(f32::from_bits(1)), "1.4E-45");
        assert_eq!(float_to_string(1.0e10), "1.0E10");
    }
}

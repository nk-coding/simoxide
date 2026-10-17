//! Java's number parsers: `Integer.valueOf(String)` / `Integer.parseInt` (OCL `toInteger`,
//! QVTo `asInteger`), `Long.parseLong`, and `Double.valueOf(String)` (OCL `toReal`, QVTo
//! `asFloat`), with Java's accepted syntax.

use super::chars;

/// `Integer.parseInt(s, 10)`: optional `+`/`-`, then decimal digits (any Unicode decimal digit,
/// as `Character.digit` accepts them), no whitespace, 32-bit range. `None` where Java throws
/// `NumberFormatException`.
pub fn parse_int(units: &[u16]) -> Option<i32> {
    parse_integral(units, i32::MIN as i64, i32::MAX as i64).map(|v| v as i32)
}

/// `Long.parseLong(s, 10)`.
pub fn parse_long(units: &[u16]) -> Option<i64> {
    parse_integral(units, i64::MIN, i64::MAX)
}

fn parse_integral(units: &[u16], min: i64, max: i64) -> Option<i64> {
    let (neg, digits) = match units.first()? {
        0x2d => (true, &units[1..]),
        0x2b => (false, &units[1..]),
        _ => (false, units),
    };
    if digits.is_empty() {
        return None;
    }
    let mut acc: i128 = 0;
    for &u in digits {
        let d = chars::digit10(u)?;
        acc = acc * 10 + d as i128;
        if acc > (max as i128) + 1 {
            return None;
        }
    }
    let v = if neg { -acc } else { acc };
    if v < min as i128 || v > max as i128 {
        return None;
    }
    Some(v as i64)
}

/// `Double.valueOf(s)` / `Double.parseDouble(s)`: `None` where Java throws
/// `NumberFormatException`.
///
/// Syntax (`FloatingDecimal.readJavaFormatString`): the string is trimmed (characters
/// `<= U+0020` at both ends); optional sign; then `NaN`, `Infinity`, a hexadecimal
/// floating-point literal (`0x1.8p3`, binary exponent required) or a decimal number with
/// optional fraction and exponent; one optional trailing `f`, `F`, `d` or `D`. Decimal digits
/// are ASCII only. The result is correctly rounded.
pub fn parse_double(units: &[u16]) -> Option<f64> {
    // trim
    let mut a = 0;
    let mut b = units.len();
    while a < b && units[a] <= 0x20 {
        a += 1;
    }
    while b > a && units[b - 1] <= 0x20 {
        b -= 1;
    }
    let s = &units[a..b];
    if s.is_empty() {
        return None;
    }
    // only ASCII can be valid
    if s.iter().any(|&u| u > 0x7f) {
        return None;
    }
    let s: Vec<u8> = s.iter().map(|&u| u as u8).collect();
    let (neg, rest) = match s[0] {
        b'-' => (true, &s[1..]),
        b'+' => (false, &s[1..]),
        _ => (false, &s[..]),
    };
    if rest.is_empty() {
        return None;
    }
    let sign = |x: f64| if neg { -x } else { x };
    match rest[0] {
        b'N' => return if rest == b"NaN" { Some(f64::NAN) } else { None },
        b'I' => {
            return if rest == b"Infinity" {
                Some(sign(f64::INFINITY))
            } else {
                None
            };
        }
        b'0' if rest.len() > 1 && (rest[1] == b'x' || rest[1] == b'X') => {
            return parse_hex(&rest[2..]).map(sign);
        }
        _ => {}
    }
    // decimal
    let mut i = 0;
    let mut int_digits = Vec::new();
    let mut frac_digits = Vec::new();
    while i < rest.len() && rest[i].is_ascii_digit() {
        int_digits.push(rest[i]);
        i += 1;
    }
    if i < rest.len() && rest[i] == b'.' {
        i += 1;
        while i < rest.len() && rest[i].is_ascii_digit() {
            frac_digits.push(rest[i]);
            i += 1;
        }
    }
    if int_digits.is_empty() && frac_digits.is_empty() {
        return None;
    }
    let mut exp: i64 = 0;
    if i < rest.len() && (rest[i] == b'e' || rest[i] == b'E') {
        i += 1;
        let mut eneg = false;
        if i < rest.len() && (rest[i] == b'+' || rest[i] == b'-') {
            eneg = rest[i] == b'-';
            i += 1;
        }
        let start = i;
        while i < rest.len() && rest[i].is_ascii_digit() {
            exp = (exp * 10 + (rest[i] - b'0') as i64).min(1_000_000_000);
            i += 1;
        }
        if i == start {
            return None;
        }
        if eneg {
            exp = -exp;
        }
    }
    if i < rest.len() {
        if i == rest.len() - 1 && matches!(rest[i], b'f' | b'F' | b'd' | b'D') {
            // type suffix: ignored by Double.valueOf
        } else {
            return None;
        }
    }
    let mut t = String::with_capacity(int_digits.len() + frac_digits.len() + 24);
    if int_digits.is_empty() {
        t.push('0');
    } else {
        t.push_str(std::str::from_utf8(&int_digits).unwrap());
    }
    t.push('.');
    if frac_digits.is_empty() {
        t.push('0');
    } else {
        t.push_str(std::str::from_utf8(&frac_digits).unwrap());
    }
    t.push('e');
    t.push_str(&exp.to_string());
    let v: f64 = t.parse().ok()?;
    Some(sign(v))
}

/// The part after `0x`: `hex+ [. hex*] | hex* . hex+`, then `p`/`P`, optional sign, decimal
/// digits, optional type suffix.
fn parse_hex(s: &[u8]) -> Option<f64> {
    let mut i = 0;
    let mut mant: u64 = 0;
    let mut sticky = false;
    let mut shift: i64 = 0; // binary exponent adjustment
    let mut seen_digit = false;
    let mut nonzero_seen = false;
    let mut bits_used = 0u32;
    let mut push = |d: u8, frac: bool, shift: &mut i64| {
        let v = (d as char).to_digit(16).unwrap() as u64;
        if !nonzero_seen && v == 0 {
            if frac {
                *shift -= 4;
            }
            return;
        }
        nonzero_seen = true;
        if bits_used + 4 <= 64 {
            mant = (mant << 4) | v;
            bits_used += 4;
            if frac {
                *shift -= 4;
            }
        } else {
            if v != 0 {
                sticky = true;
            }
            if !frac {
                *shift += 4;
            }
        }
    };
    while i < s.len() && s[i].is_ascii_hexdigit() {
        push(s[i], false, &mut shift);
        seen_digit = true;
        i += 1;
    }
    if i < s.len() && s[i] == b'.' {
        i += 1;
        while i < s.len() && s[i].is_ascii_hexdigit() {
            push(s[i], true, &mut shift);
            seen_digit = true;
            i += 1;
        }
    }
    if !seen_digit {
        return None;
    }
    if i >= s.len() || (s[i] != b'p' && s[i] != b'P') {
        return None;
    }
    i += 1;
    let mut eneg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        eneg = s[i] == b'-';
        i += 1;
    }
    let start = i;
    let mut e: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        e = (e * 10 + (s[i] - b'0') as i64).min(1_000_000_000);
        i += 1;
    }
    if i == start {
        return None;
    }
    if i < s.len() && !(i == s.len() - 1 && matches!(s[i], b'f' | b'F' | b'd' | b'D')) {
        return None;
    }
    let e = if eneg { -e } else { e };
    Some(make_double(mant, sticky, e + shift))
}

/// The double nearest to `(m + sticky * epsilon) * 2^e2` (ties to even).
pub(crate) fn make_double(m: u64, sticky: bool, e2: i64) -> f64 {
    if m == 0 {
        return 0.0;
    }
    let lz = m.leading_zeros();
    let m = m << lz;
    let e2 = e2 - lz as i64;
    // value = 1.f * 2^big_e
    let mut big_e = e2 + 63;
    if big_e > 1023 {
        return f64::INFINITY;
    }
    let keep: i64 = if big_e >= -1022 {
        53
    } else {
        53 - (-1022 - big_e)
    };
    if keep < 0 {
        return 0.0;
    }
    let shift = (64 - keep) as u32; // 11..=64
    let mut kept: u64 = if shift >= 64 { 0 } else { m >> shift };
    let round = (m >> (shift - 1)) & 1 == 1;
    let rest = if shift - 1 == 0 {
        0
    } else {
        m & ((1u64 << (shift - 1)) - 1)
    };
    let st = rest != 0 || sticky;
    if round && (st || kept & 1 == 1) {
        kept += 1;
    }
    if big_e >= -1022 {
        if kept == 1u64 << 53 {
            kept >>= 1;
            big_e += 1;
            if big_e > 1023 {
                return f64::INFINITY;
            }
        }
        f64::from_bits((((big_e + 1023) as u64) << 52) | (kept & ((1u64 << 52) - 1)))
    } else {
        // subnormal (a carry into bit 52 gives the smallest normal, as it should)
        f64::from_bits(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn ints() {
        assert_eq!(parse_int(&u("123")), Some(123));
        assert_eq!(parse_int(&u("-2147483648")), Some(i32::MIN));
        assert_eq!(parse_int(&u("2147483648")), None);
        assert_eq!(parse_int(&u("+7")), Some(7));
        assert_eq!(parse_int(&u(" 7")), None);
        assert_eq!(parse_int(&u("-")), None);
        assert_eq!(parse_int(&u("")), None);
        assert_eq!(parse_int(&u("\u{0663}\u{0664}")), Some(34));
    }

    #[test]
    fn doubles() {
        let p = |s: &str| parse_double(&u(s));
        assert_eq!(p("1.5"), Some(1.5));
        assert_eq!(p(" 1e3 "), Some(1000.0));
        assert_eq!(p("1.5f"), Some(1.5));
        assert_eq!(p(".5"), Some(0.5));
        assert_eq!(p("5."), Some(5.0));
        assert_eq!(p("."), None);
        assert_eq!(p("1e"), None);
        assert!(p("NaN").unwrap().is_nan());
        assert_eq!(p("-Infinity"), Some(f64::NEG_INFINITY));
        assert_eq!(p("0x1.8p1"), Some(3.0));
        assert_eq!(p("0x1p-1074"), Some(f64::from_bits(1)));
        assert_eq!(p("0x1p-1075"), Some(0.0));
        assert_eq!(p("0x1.8p-1075"), Some(f64::from_bits(1)));
        assert_eq!(p("0x1.fffffffffffff8p1023"), Some(f64::INFINITY));
        assert_eq!(p("0x1"), None);
        assert_eq!(p("1ee"), None);
    }
}

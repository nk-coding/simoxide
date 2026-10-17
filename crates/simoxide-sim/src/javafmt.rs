//! Java number and string formatting used by the trace, tape and measurement files
//! (`docs/guide/formats.md` §1).

use std::fmt::Write as _;

/// Appends `Double.toString(x)` (JDK ≥ 19: shortest round-trip digits; `simoxide-jcompat`).
///
/// `1e-3 <= |x| < 1e7` is written as a plain decimal with at least one fractional digit,
/// everything else as `d.ddd` + `E` + exponent. NaN and infinities are written as Java prints
/// them (`NaN`, `Infinity`); the JSON writers quote them.
#[inline]
pub fn push_double(out: &mut String, x: f64) {
    simoxide_jcompat::fmt::push_double(out, x);
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

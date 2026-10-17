//! Java `hashCode` of the boxed primitive classes and `String` (the javadoc formulas), and
//! `Double.compare`/`Float.compare`. `HashMap`'s hash spreading is in `simoxide-jcompat-exact`.

/// `String.hashCode()` over UTF-16 code units: `s[0]*31^(n-1) + ... + s[n-1]`, wrapping.
#[inline]
pub fn utf16_hash(units: impl IntoIterator<Item = u16>) -> i32 {
    let mut h: i32 = 0;
    for u in units {
        h = h.wrapping_mul(31).wrapping_add(i32::from(u));
    }
    h
}

/// `String.hashCode()` of a Rust string (UTF-16 code units of its characters).
#[inline]
pub fn str_hash(s: &str) -> i32 {
    if s.is_ascii() {
        let mut h: i32 = 0;
        for &b in s.as_bytes() {
            h = h.wrapping_mul(31).wrapping_add(i32::from(b));
        }
        h
    } else {
        utf16_hash(s.encode_utf16())
    }
}

/// `Long.hashCode(v)`: `(int)(v ^ (v >>> 32))`.
#[inline]
pub fn long_hash(v: i64) -> i32 {
    (v ^ ((v as u64) >> 32) as i64) as i32
}

/// `Double.doubleToLongBits`: the raw bits with every NaN collapsed to `0x7ff8000000000000`.
#[inline]
pub fn double_to_long_bits(d: f64) -> i64 {
    if d.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        d.to_bits() as i64
    }
}

/// `Double.hashCode(d)`.
#[inline]
pub fn double_hash(d: f64) -> i32 {
    long_hash(double_to_long_bits(d))
}

/// `Float.floatToIntBits`: every NaN collapsed to `0x7fc00000`.
#[inline]
pub fn float_to_int_bits(f: f32) -> i32 {
    if f.is_nan() {
        0x7fc0_0000
    } else {
        f.to_bits() as i32
    }
}

/// `Float.hashCode(f)`.
#[inline]
pub fn float_hash(f: f32) -> i32 {
    float_to_int_bits(f)
}

/// `Boolean.hashCode(b)`.
#[inline]
pub fn bool_hash(b: bool) -> i32 {
    if b { 1231 } else { 1237 }
}

/// `Double.compare(a, b)`: numeric order, `-0.0 < 0.0`, NaN above everything and equal to itself.
pub fn double_compare(a: f64, b: f64) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    if a < b {
        Less
    } else if a > b {
        Greater
    } else {
        double_to_long_bits(a).cmp(&double_to_long_bits(b))
    }
}

/// `Float.compare(a, b)`.
pub fn float_compare(a: f32, b: f32) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    if a < b {
        Less
    } else if a > b {
        Greater
    } else {
        float_to_int_bits(a).cmp(&float_to_int_bits(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values() {
        assert_eq!(str_hash(""), 0);
        assert_eq!(str_hash("hello"), 99162322);
        assert_eq!(str_hash("polygenelubricants"), i32::MIN);
        assert_eq!(str_hash("Aa"), str_hash("BB"));
        // U+1F600 is the surrogate pair D83D DE00
        assert_eq!(str_hash("\u{1F600}"), 0xD83D * 31 + 0xDE00);
        assert_eq!(long_hash(-1), 0);
        assert_eq!(long_hash(1 << 32), 1);
        assert_eq!(double_hash(0.0), 0);
        assert_eq!(double_hash(-0.0), i32::MIN);
        assert_eq!(double_hash(1.0), 1072693248);
        assert_eq!(double_hash(f64::NAN), 2146959360);
        assert_eq!(bool_hash(true), 1231);
    }
}

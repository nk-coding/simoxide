//! `java.lang.Character` data of the reference JDK (21), as generated into [`chars_data`] by
//! `tools/oracle/GenChars.java`: classification of UTF-16 code units, decimal digit values,
//! simple and full case mappings.
//!
//! Generating the tables from the JDK, rather than using Rust's `char` methods, keeps the
//! Unicode version and Java's own definitions (e.g. `isWhitespace` excludes no-break spaces).

use super::chars_data as d;

fn in_ranges(r: &[(u32, u32)], c: u32) -> bool {
    r.binary_search_by(|&(s, e)| {
        if c < s {
            std::cmp::Ordering::Greater
        } else if c > e {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    })
    .is_ok()
}

fn map_pairs(r: &[(u32, u32)], c: u32) -> u32 {
    match r.binary_search_by_key(&c, |&(k, _)| k) {
        Ok(i) => r[i].1,
        Err(_) => c,
    }
}

/// `Character.isWhitespace(char)`.
pub fn is_whitespace(u: u16) -> bool {
    in_ranges(d::WHITESPACE, u as u32)
}

/// `Character.isLetter(char)`.
pub fn is_letter(u: u16) -> bool {
    in_ranges(d::LETTER, u as u32)
}

/// `Character.isLetterOrDigit(char)`.
pub fn is_letter_or_digit(u: u16) -> bool {
    in_ranges(d::LETTER_OR_DIGIT, u as u32)
}

/// `Character.digit(char, 10)`.
pub fn digit10(u: u16) -> Option<u32> {
    let c = u as u32;
    if (0x30..=0x39).contains(&c) {
        return Some(c - 0x30);
    }
    let i = d::DIGITS.partition_point(|&(s, _)| s <= c);
    if i == 0 {
        return None;
    }
    let (s, e) = d::DIGITS[i - 1];
    if c <= e { Some(c - s) } else { None }
}

/// `Character.toUpperCase(int)` (simple mapping).
pub fn to_upper_cp(c: u32) -> u32 {
    if c < 0x80 {
        return if (0x61..=0x7a).contains(&c) {
            c - 32
        } else {
            c
        };
    }
    map_pairs(d::UPPER, c)
}

/// `Character.toLowerCase(int)` (simple mapping).
pub fn to_lower_cp(c: u32) -> u32 {
    if c < 0x80 {
        return if (0x41..=0x5a).contains(&c) {
            c + 32
        } else {
            c
        };
    }
    map_pairs(d::LOWER, c)
}

/// `Character.toUpperCase(char)`.
pub fn to_upper_unit(u: u16) -> u16 {
    to_upper_cp(u as u32) as u16
}

/// `Character.toLowerCase(char)`.
pub fn to_lower_unit(u: u16) -> u16 {
    to_lower_cp(u as u32) as u16
}

/// The full upper-case mapping of a code point when it is not a single code point (`ß` to
/// `SS`), as `String.toUpperCase(Locale.ROOT)` applies it.
pub fn upper_special(c: u32) -> Option<&'static [u16]> {
    d::UPPER_SPECIAL
        .binary_search_by_key(&c, |&(k, _)| k)
        .ok()
        .map(|i| d::UPPER_SPECIAL[i].1)
}

/// The full lower-case mapping when it differs from the simple one (only `İ`).
pub fn lower_special(c: u32) -> Option<&'static [u16]> {
    d::LOWER_SPECIAL
        .binary_search_by_key(&c, |&(k, _)| k)
        .ok()
        .map(|i| d::LOWER_SPECIAL[i].1)
}

/// Lower-, upper- or title-case letter (`ConditionalSpecialCasing.isCased`, approximately).
pub fn is_cased(c: u32) -> bool {
    in_ranges(d::CASED, c)
}

/// Unicode's Case_Ignorable, approximated from Java's general categories.
pub fn is_case_ignorable(c: u32) -> bool {
    in_ranges(d::CASE_IGNORABLE, c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples() {
        assert!(is_whitespace(0x20) && is_whitespace(0x09) && !is_whitespace(0xa0));
        assert!(is_letter(0x41) && !is_letter(0x5f) && is_letter(0x3b1));
        assert!(is_letter_or_digit(0x39) && is_letter_or_digit(0x663));
        assert_eq!(digit10(0x663), Some(3));
        assert_eq!(digit10(0x41), None);
        assert_eq!(to_upper_cp('a' as u32), 'A' as u32);
        assert_eq!(to_upper_cp(0xe9), 0xc9);
        assert_eq!(to_upper_cp(0x101), 0x100);
        assert_eq!(to_upper_cp(0x100), 0x100);
        assert_eq!(to_lower_cp(0x100), 0x101);
        assert_eq!(to_upper_cp(0x10428), 0x10400);
        assert_eq!(upper_special(0xdf), Some(&[0x53u16, 0x53][..]));
        assert_eq!(lower_special(0x130), Some(&[0x69u16, 0x307][..]));
    }
}

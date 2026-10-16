//! Decoding of string literal and quoted name tokens, as Eclipse's parser does it
//! (`AbstractQVTParser.unSingleQuote`, classic OCL's `AbstractParser.decodeString` and
//! `decodeEscapeSequence`; EPL-2.0).

/// Message of Eclipse's `InvalidEscapeSequence_ERROR`.
pub const INVALID_ESCAPE: &str = "Invalid escape sequence (valid ones are \\b \\t \\n \\f \\r \\\" \\' \\\\)";

/// The content of a `'…'`, `"…"` or `_'…'` token without its quotes.
pub fn strip_quotes(tok: &str) -> &str {
    let t = tok.strip_prefix('_').unwrap_or(tok);
    if t.len() >= 2 {
        &t[1..t.len() - 1]
    } else {
        ""
    }
}

/// Decodes the escape sequences of a token's content. A lone CR becomes LF first (Eclipse
/// rewrites it before lexing). Returns `Err(raw)` with the undecoded content when an escape
/// sequence is invalid (Eclipse reports an error and keeps the raw text).
pub fn decode(content: &str) -> Result<String, String> {
    let fixed;
    let content = if content.contains('\r') {
        fixed = fix_cr(content);
        fixed.as_str()
    } else {
        content
    };
    if !content.contains('\\') {
        return Ok(content.to_string());
    }
    // Work on UTF-16 code units like Java does.
    let units: Vec<u16> = content.encode_utf16().collect();
    let mut out: Vec<u16> = Vec::with_capacity(units.len());
    let n = units.len();
    let mut i = 0;
    let oct = |u: u16| (b'0' as u16..=b'7' as u16).contains(&u);
    let hex = |u: Option<&u16>| -> Option<u16> {
        let c = char::from_u32(*u? as u32)?;
        c.to_digit(16).map(|d| d as u16)
    };
    while i < n {
        let c = units[i];
        if c != b'\\' as u16 {
            out.push(c);
            i += 1;
            continue;
        }
        let Some(&e) = units.get(i + 1) else { return Err(content.to_string()) };
        let simple = match e {
            0x62 => Some(0x08), // b
            0x66 => Some(0x0c), // f
            0x74 => Some(0x09), // t
            0x6e => Some(0x0a), // n
            0x72 => Some(0x0d), // r
            0x5c => Some(0x5c), // backslash
            0x27 => Some(0x27), // '
            0x22 => Some(0x22), // "
            _ => None,
        };
        if let Some(s) = simple {
            out.push(s);
            i += 2;
            continue;
        }
        match e {
            0x30..=0x33 => {
                // \0 .. \377
                let c1 = e - 0x30;
                let mut v = c1;
                let mut j = i + 2;
                if j < n && oct(units[j]) {
                    v = (v << 3) + (units[j] - 0x30);
                    j += 1;
                    if j < n && oct(units[j]) {
                        v = (v << 3) + (units[j] - 0x30);
                        j += 1;
                    }
                }
                out.push(v);
                i = j;
            }
            0x34..=0x37 => {
                // \4 .. \77, with Eclipse's bug: a following non-octal character makes the
                // iterator step back twice, so the digit is emitted again.
                let c1 = e - 0x30;
                let j = i + 2;
                if j < n && oct(units[j]) {
                    out.push((c1 << 3) + (units[j] - 0x30));
                    i = j + 1;
                } else if j < n {
                    out.push(c1);
                    i += 1;
                } else {
                    out.push(c1);
                    i = j;
                }
            }
            0x78 => {
                // \xhh
                match (hex(units.get(i + 2)), hex(units.get(i + 3))) {
                    (Some(a), Some(b)) => {
                        out.push((a << 4) + b);
                        i += 4;
                    }
                    _ => return Err(content.to_string()),
                }
            }
            0x75 => {
                // \uhhhh
                let d: Vec<Option<u16>> = (2..6).map(|k| hex(units.get(i + k))).collect();
                if d.iter().all(Option::is_some) {
                    let v = d.iter().fold(0u16, |acc, x| (acc << 4) + x.unwrap());
                    out.push(v);
                    i += 6;
                } else {
                    return Err(content.to_string());
                }
            }
            _ => return Err(content.to_string()),
        }
    }
    Ok(String::from_utf16_lossy(&out))
}

fn fix_cr(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    for (i, &c) in b.iter().enumerate() {
        if c == b'\r' && b.get(i + 1) != Some(&b'\n') {
            out.push_str(&s[last..i]);
            out.push('\n');
            last = i + 1;
        }
    }
    out.push_str(&s[last..]);
    out
}

/// Encodes a string as a single-quoted QVTo literal that decodes back to it.
pub fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for c in value.chars() {
        match c {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\u{:04x}", c as u32)),
            // Characters the LPG lexer treats as end of input.
            '\u{b4}' | '\u{ffff}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes() {
        assert_eq!(decode(r"a\tb\n").unwrap(), "a\tb\n");
        assert_eq!(decode(r"\123").unwrap(), "S");
        assert_eq!(decode(r"\0").unwrap(), "\0");
        assert_eq!(decode(r"\4a").unwrap(), "\u{4}4a");
        assert_eq!(decode(r"\4").unwrap(), "\u{4}");
        assert_eq!(decode(r"\47").unwrap(), "'");
        assert_eq!(decode(r"\x41B").unwrap(), "AB");
        assert_eq!(decode(r"😀").unwrap(), "\u{1f600}");
        assert!(decode(r"\q").is_err());
        assert!(decode(r"\x4").is_err());
        assert_eq!(decode("a\rb\r\nc").unwrap(), "a\nb\r\nc");
    }

    #[test]
    fn quote_round_trip() {
        for s in ["", "it's", "a\\b", "\n\t\u{1}x\u{b4}", "ü"] {
            let q = quote(s);
            assert_eq!(decode(strip_quotes(&q)).unwrap(), s);
        }
    }
}

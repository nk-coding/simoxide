//! The lexer. It follows Eclipse's LPG lexers (`QVTOLexer.gi` and its imports); the keyword,
//! punctuation and character-class tables are generated from them (`generated.rs`).
//!
//! Reproduced quirks (see `reference/notes/eclipse-semantics.md` §P.7):
//! - every character above ASCII is a letter, except U+00B4 (acute accent) and U+FFFF, which the
//!   LPG lexer classifies as end of input: lexing stops at the first of them;
//! - `<<`, `>>`, `...`, `!=`, `::=` and `+=` are single tokens (longest match);
//! - `1.` is INTEGER DOT, `1..2` is INTEGER DOTDOT INTEGER, `1...` is INTEGER DOTDOT DOT, `1e` and
//!   `1e+` are invalid tokens;
//! - strings may contain raw tab, LF and CR; a backslash must be followed by a printable
//!   character; a form feed or other control character ends the token as invalid;
//! - `--` and `//` line comments, non-nesting `/* */` comments;
//! - invalid characters and invalid tokens are reported and dropped, as LPG does.

use crate::diag::{Code, Diagnostic};
use crate::generated::{TokenKind, class, keyword, punctuation};
use crate::span::Span;

/// A token: kind and byte span.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// The lexer output.
#[derive(Clone, Debug, Default)]
pub struct Lexed {
    /// The tokens, ending with one `Eof` token.
    pub tokens: Vec<Token>,
    /// Comment spans, in order (single-line and multi-line).
    pub comments: Vec<Span>,
    pub diagnostics: Vec<Diagnostic>,
    /// End of the text the lexer saw: the source length, or the offset of the first character
    /// the LPG lexer treats as end of input.
    pub end: u32,
}

#[inline]
fn flags(b: u8) -> u8 {
    if b < 128 {
        class::ASCII[b as usize]
    } else if class::NON_ASCII_IS_LETTER {
        // Bytes of UTF-8 sequences: every non-ASCII character is a letter (the EOF characters
        // were cut off before lexing).
        class::IDENT_START | class::IDENT_PART
    } else {
        0
    }
}

/// Offset of the first character the LPG lexer maps to end of input.
fn effective_end(src: &str) -> usize {
    if src.is_ascii() {
        return src.len();
    }
    for (i, c) in src.char_indices() {
        if class::EOF_CHARS.contains(&c) {
            return i;
        }
    }
    src.len()
}

/// Lexes a whole source text.
pub fn lex(src: &str) -> Lexed {
    let end = effective_end(src);
    let b = &src.as_bytes()[..end];
    let mut out = Lexed {
        tokens: Vec::with_capacity(end / 5 + 1),
        end: end as u32,
        ..Default::default()
    };
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        let f = flags(c);
        if f & class::WHITESPACE != 0 {
            i += 1;
            continue;
        }
        let start = i;
        if f & class::IDENT_START != 0 {
            if c == b'_' && b.get(i + 1) == Some(&b'\'') {
                match scan_string(b, i + 1, b'\'') {
                    Ok(e) => {
                        out.tokens.push(tok(TokenKind::QuotedIdentifier, start, e));
                        i = e;
                    }
                    Err(e) => {
                        invalid_token(src, &mut out, start, e);
                        i = e;
                    }
                }
                continue;
            }
            i += 1;
            while i < b.len() && flags(b[i]) & class::IDENT_PART != 0 {
                i += 1;
            }
            let kind = keyword(&b[start..i]).unwrap_or(TokenKind::Identifier);
            out.tokens.push(tok(kind, start, i));
            continue;
        }
        if f & class::DIGIT != 0 {
            i = lex_number(src, b, i, &mut out);
            continue;
        }
        match c {
            b'\'' | b'"' => {
                match scan_string(b, i, c) {
                    Ok(e) => {
                        out.tokens.push(tok(TokenKind::StringLiteral, start, e));
                        i = e;
                    }
                    Err(e) => {
                        invalid_token(src, &mut out, start, e);
                        i = e;
                    }
                }
                continue;
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                i = line_comment(b, i);
                out.comments.push(Span::new(start as u32, i as u32));
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                i = line_comment(b, i);
                out.comments.push(Span::new(start as u32, i as u32));
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let mut j = i + 2;
                let mut closed = false;
                while j + 1 < b.len() {
                    if b[j] == b'*' && b[j + 1] == b'/' {
                        closed = true;
                        break;
                    }
                    j += 1;
                }
                if closed {
                    i = j + 2;
                    out.comments.push(Span::new(start as u32, i as u32));
                } else {
                    i = b.len();
                    invalid_token(src, &mut out, start, i);
                }
                continue;
            }
            b'`' => {
                // `BackQuote SLNotSQOpt Acute`: the acute accent is end of input, so this never
                // completes and runs to the end of input as an invalid token.
                i = b.len();
                invalid_token(src, &mut out, start, i);
                continue;
            }
            _ => {}
        }
        if let Some((kind, len)) = punctuation(&b[i..]) {
            out.tokens.push(tok(kind, i, i + len));
            i += len;
            continue;
        }
        // A character no token starts with.
        let ch_len = src[i..].chars().next().map_or(1, char::len_utf8);
        let pos = line_col(src, i);
        let text = if c < 32 || c == 127 {
            String::new()
        } else {
            src[i..i + ch_len].to_string()
        };
        out.diagnostics.push(Diagnostic::error(
            Code::InvalidCharacter,
            Span::new(i as u32, (i + ch_len) as u32),
            format!("{pos} \"{text}\" unexpected character ignored"),
        ));
        i += ch_len;
    }
    out.tokens.push(tok(TokenKind::Eof, end, end));
    out
}

#[inline]
fn tok(kind: TokenKind, start: usize, end: usize) -> Token {
    Token {
        kind,
        span: Span::new(start as u32, end as u32),
    }
}

fn line_comment(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i] != b'\n' && b[i] != b'\r' {
        i += 1;
    }
    i
}

/// Scans a string body starting at the opening quote `b[i]`. Returns the end offset after the
/// closing quote, or `Err(end)` with the end of the invalid token.
fn scan_string(b: &[u8], i: usize, quote: u8) -> Result<usize, usize> {
    let mut j = i + 1;
    while j < b.len() {
        let c = b[j];
        if c == quote {
            return Ok(j + 1);
        }
        if c == b'\\' {
            // BackslashEscapedSymbol: '\' followed by a printable character or space.
            match b.get(j + 1) {
                Some(&n) if n >= 0x20 && n != 0x7f => {
                    j += 2;
                    continue;
                }
                _ => return Err(j + 1),
            }
        }
        if c < 0x20 && !matches!(c, b'\t' | b'\n' | b'\r') || c == 0x7f {
            return Err(j);
        }
        j += 1;
    }
    Err(b.len())
}

fn lex_number(src: &str, b: &[u8], start: usize, out: &mut Lexed) -> usize {
    let digits = |mut j: usize| {
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
        j
    };
    let mut i = digits(start);
    let mut real = false;
    if b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i = digits(i + 1);
        real = true;
    }
    if matches!(b.get(i), Some(b'e' | b'E')) {
        // The LPG lexer commits to an exponent after an `e`.
        let mut j = i + 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if b.get(j).is_some_and(u8::is_ascii_digit) {
            i = digits(j);
            real = true;
        } else {
            invalid_token(src, out, start, j);
            return j;
        }
    }
    out.tokens.push(tok(
        if real {
            TokenKind::RealLiteral
        } else {
            TokenKind::IntegerLiteral
        },
        start,
        i,
    ));
    if !real && b[i..].starts_with(b"..") {
        // `Token ::= IntegerLiteral DotDotToken`: after an integer, `...` is `..` then `.`.
        out.tokens.push(tok(TokenKind::Dotdot, i, i + 2));
        return i + 2;
    }
    i
}

fn invalid_token(src: &str, out: &mut Lexed, start: usize, end: usize) {
    let pos = line_col(src, start);
    out.diagnostics.push(Diagnostic::error(
        Code::InvalidToken,
        Span::new(start as u32, end as u32),
        format!("{pos} token \"{}\" is invalid", lpg_text(&src[start..end])),
    ));
}

/// `line:col` as LPG prints it in lexer messages (1-based line, 1-based column, tab = 1).
fn line_col(src: &str, offset: usize) -> String {
    let before = &src[..offset];
    let line = 1
        + before.bytes().filter(|&c| c == b'\n').count()
        + before
            .as_bytes()
            .windows(2)
            .filter(|w| w[0] == b'\r' && w[1] != b'\n')
            .count()
        + usize::from(before.ends_with('\r'));
    let line_start = before.rfind(['\n', '\r']).map_or(0, |p| p + 1);
    let col = crate::span::utf16_len(&before[line_start..]) + 1;
    format!("{line}:{col}")
}

/// Token text as LPG's `computeInputString` shows it in messages: tabs become spaces, a line
/// break becomes a space unless it follows white space.
pub(crate) fn lpg_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    for c in s.chars() {
        match c {
            '\t' => out.push(' '),
            '\n' | '\r' | '\u{c}' => {
                if !prev_ws && !out.is_empty() {
                    out.push(' ');
                }
            }
            _ => out.push(c),
        }
        prev_ws = c.is_whitespace();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use TokenKind as K;

    fn kinds(s: &str) -> Vec<TokenKind> {
        lex(s).tokens.iter().map(|t| t.kind).collect()
    }

    #[test]
    fn numbers() {
        assert_eq!(
            kinds("1.e3"),
            [K::IntegerLiteral, K::Dot, K::Identifier, K::Eof]
        );
        assert_eq!(
            kinds("1..2"),
            [K::IntegerLiteral, K::Dotdot, K::IntegerLiteral, K::Eof]
        );
        assert_eq!(kinds("1...*"), [K::IntegerLiteral, K::Dotdot, K::Dot, K::Multiply, K::Eof]);
        assert_eq!(kinds("1 ...*"), [K::IntegerLiteral, K::MultiplicityRange, K::Multiply, K::Eof]);
        assert_eq!(
            kinds("1.5e-3 2E7"),
            [K::RealLiteral, K::RealLiteral, K::Eof]
        );
        assert_eq!(
            kinds("3.toString()"),
            [
                K::IntegerLiteral,
                K::Dot,
                K::Identifier,
                K::Lparen,
                K::Rparen,
                K::Eof
            ]
        );
        let l = lex("1e;");
        assert_eq!(
            l.tokens.iter().map(|t| t.kind).collect::<Vec<_>>(),
            [K::Semicolon, K::Eof]
        );
        assert_eq!(l.diagnostics[0].span, Span::new(0, 2));
    }

    #[test]
    fn punctuation_longest_match() {
        assert_eq!(
            kinds("a>>b<<c"),
            [
                K::Identifier,
                K::StereotypeQualifierClose,
                K::Identifier,
                K::StereotypeQualifierOpen,
                K::Identifier,
                K::Eof
            ]
        );
        assert_eq!(
            kinds("::= :: := ... != ! ->"),
            [
                K::Coloncolonequal,
                K::Coloncolon,
                K::ResetAssign,
                K::MultiplicityRange,
                K::NotEqualExeq,
                K::ExclamationMark,
                K::Arrow,
                K::Eof
            ]
        );
    }

    #[test]
    fn keywords_and_names() {
        assert_eq!(
            kinds("select xselect _'and' and résumé x$1"),
            [
                K::Identifier,
                K::KwXselect,
                K::QuotedIdentifier,
                K::KwAnd,
                K::Identifier,
                K::Identifier,
                K::Eof
            ]
        );
    }

    #[test]
    fn strings_and_comments() {
        let l = lex("'a\\'b' \"c\" -- x\n// y\r/* z */ 'm\nn'");
        assert_eq!(l.tokens.len(), 4);
        assert_eq!(l.comments.len(), 3);
        assert!(l.diagnostics.is_empty());
    }

    #[test]
    fn acute_ends_input() {
        let l = lex("a b\u{b4} c d");
        assert_eq!(l.tokens.len(), 3);
        assert_eq!(l.end, 3);
    }

    #[test]
    fn invalid_characters() {
        let l = lex("1 # 2");
        assert_eq!(l.tokens.len(), 3);
        assert_eq!(
            l.diagnostics[0].message,
            "1:3 \"#\" unexpected character ignored"
        );
        let l = lex("x := 'abc;\n}\n");
        assert_eq!(l.diagnostics[0].span, Span::new(5, 13));
        assert_eq!(
            l.diagnostics[0].message,
            "1:6 token \"'abc; } \" is invalid"
        );
    }
}

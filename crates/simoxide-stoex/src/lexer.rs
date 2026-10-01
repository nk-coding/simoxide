//! Lexer for the StoEx grammar of SimuLizar 5.2.2 (`Stoex.xtext` + `PCMStoex.xtext`).
//!
//! Reproduces the token set and the ANTLR 3 lexing decisions of the generated Xtext lexer:
//!
//! * hidden tokens: `WS` (space, tab, CR, LF only), `ML_COMMENT` (`/* ... */`),
//!   `SL_COMMENT` (`// ...` up to and including the line break);
//! * keywords win over `ID` only on an exact match (`ANDx` is an `ID`);
//! * `DECINT` is `'0' | [1-9][0-9]*`, so `01` is two tokens (and hence a syntax error);
//! * `DOUBLE` is `DECINT ('.' DIGIT* | ('.' DIGIT*)? [eE] [+-]? DECINT)`; once an `e`/`E` follows
//!   the mantissa the lexer commits to the exponent (`2e` is a lexer error, not `2` + `e`);
//! * `STRING`: double or single quoted with the escapes `\b \t \n \f \r \u \" \' \\`. Neither
//!   form may contain a raw `"` or `\`. A single-quoted string may contain `'`: the ANTLR
//!   decision closes it at a `'` only if the next character is the end of input or `"`
//!   (so `'a' == 'b'` is the one string `a' == 'b`).
//! * every other character is `ANY_OTHER`, which no parser rule accepts.

use crate::error::{ParseError, Span};

/// Keywords of the grammar (they cannot be used as identifiers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kw {
    Not,
    And,
    Or,
    Xor,
    IntPmf,
    DoublePmf,
    EnumPmf,
    BoolPmf,
    DoublePdf,
    Ordered,
    ByteSize,
    NumberOfElements,
    Structure,
    Type,
    Value,
}

impl Kw {
    fn from_word(w: &str) -> Option<Kw> {
        Some(match w {
            "NOT" => Kw::Not,
            "AND" => Kw::And,
            "OR" => Kw::Or,
            "XOR" => Kw::Xor,
            "IntPMF" => Kw::IntPmf,
            "DoublePMF" => Kw::DoublePmf,
            "EnumPMF" => Kw::EnumPmf,
            "BoolPMF" => Kw::BoolPmf,
            "DoublePDF" => Kw::DoublePdf,
            "ordered" => Kw::Ordered,
            "BYTESIZE" => Kw::ByteSize,
            "NUMBER_OF_ELEMENTS" => Kw::NumberOfElements,
            "STRUCTURE" => Kw::Structure,
            "TYPE" => Kw::Type,
            "VALUE" => Kw::Value,
            _ => return None,
        })
    }

    /// The keyword text.
    pub fn as_str(self) -> &'static str {
        match self {
            Kw::Not => "NOT",
            Kw::And => "AND",
            Kw::Or => "OR",
            Kw::Xor => "XOR",
            Kw::IntPmf => "IntPMF",
            Kw::DoublePmf => "DoublePMF",
            Kw::EnumPmf => "EnumPMF",
            Kw::BoolPmf => "BoolPMF",
            Kw::DoublePdf => "DoublePDF",
            Kw::Ordered => "ordered",
            Kw::ByteSize => "BYTESIZE",
            Kw::NumberOfElements => "NUMBER_OF_ELEMENTS",
            Kw::Structure => "STRUCTURE",
            Kw::Type => "TYPE",
            Kw::Value => "VALUE",
        }
    }
}

/// Punctuation / operator tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum P {
    Question,
    Colon,
    Gt,
    Lt,
    EqEq,
    NotEq,
    Ge,
    Le,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
    Comma,
    Dot,
    LBracket,
    RBracket,
    Semi,
}

impl P {
    /// The token text.
    pub fn as_str(self) -> &'static str {
        match self {
            P::Question => "?",
            P::Colon => ":",
            P::Gt => ">",
            P::Lt => "<",
            P::EqEq => "==",
            P::NotEq => "<>",
            P::Ge => ">=",
            P::Le => "<=",
            P::Plus => "+",
            P::Minus => "-",
            P::Star => "*",
            P::Slash => "/",
            P::Percent => "%",
            P::Caret => "^",
            P::LParen => "(",
            P::RParen => ")",
            P::Comma => ",",
            P::Dot => ".",
            P::LBracket => "[",
            P::RBracket => "]",
            P::Semi => ";",
        }
    }
}

/// Token kinds. Literal values are converted by the parser (value converter errors are syntax
/// errors in the reference, reported at the token).
#[derive(Debug, Clone, PartialEq)]
pub enum TokKind {
    /// `DECINT` (text in the source span).
    DecInt,
    /// `DOUBLE` (text in the source span).
    Double,
    /// `STRING`, already unescaped.
    Str(String),
    /// `BOOLEAN_KEYWORDS`.
    Bool(bool),
    /// `ID`.
    Id,
    Kw(Kw),
    P(P),
    /// A character no parser rule accepts (`ANY_OTHER`).
    Other,
    Eof,
}

/// A token with its source span (byte offsets).
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokKind,
    pub span: Span,
}

/// Tokenizes `src` completely. Hidden tokens are dropped. The first lexer error is returned.
pub fn tokenize(src: &str) -> Result<Vec<Token>, ParseError> {
    let b = src.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        let start = i;
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                // ML_COMMENT: '/*' -> '*/'
                let mut j = i + 2;
                loop {
                    if j + 1 >= b.len() {
                        return Err(ParseError::new(
                            src,
                            Span::new(start, b.len()),
                            "unterminated comment (expecting '*/')",
                        ));
                    }
                    if b[j] == b'*' && b[j + 1] == b'/' {
                        i = j + 2;
                        break;
                    }
                    j += 1;
                }
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                // SL_COMMENT: '//' !('\n'|'\r')* ('\r'? '\n')?
                let mut j = i + 2;
                while j < b.len() && b[j] != b'\n' && b[j] != b'\r' {
                    j += 1;
                }
                if j < b.len() && b[j] == b'\r' {
                    j += 1;
                }
                if j < b.len() && b[j] == b'\n' {
                    j += 1;
                }
                i = j;
                continue;
            }
            b'0'..=b'9' => {
                let (kind, end) = lex_number(src, i)?;
                toks.push(Token {
                    kind,
                    span: Span::new(start, end),
                });
                i = end;
                continue;
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                let w = &src[i..j];
                let kind = match w {
                    "true" => TokKind::Bool(true),
                    "false" => TokKind::Bool(false),
                    _ => match Kw::from_word(w) {
                        Some(k) => TokKind::Kw(k),
                        None => TokKind::Id,
                    },
                };
                toks.push(Token {
                    kind,
                    span: Span::new(start, j),
                });
                i = j;
                continue;
            }
            b'"' | b'\'' => {
                let (s, end) = lex_string(src, i)?;
                toks.push(Token {
                    kind: TokKind::Str(s),
                    span: Span::new(start, end),
                });
                i = end;
                continue;
            }
            _ => {}
        }
        let two = |a: u8, bb: u8| c == a && b.get(i + 1) == Some(&bb);
        let (p, len) = if two(b'=', b'=') {
            (Some(P::EqEq), 2)
        } else if two(b'<', b'>') {
            (Some(P::NotEq), 2)
        } else if two(b'>', b'=') {
            (Some(P::Ge), 2)
        } else if two(b'<', b'=') {
            (Some(P::Le), 2)
        } else {
            let p = match c {
                b'?' => Some(P::Question),
                b':' => Some(P::Colon),
                b'>' => Some(P::Gt),
                b'<' => Some(P::Lt),
                b'+' => Some(P::Plus),
                b'-' => Some(P::Minus),
                b'*' => Some(P::Star),
                b'/' => Some(P::Slash),
                b'%' => Some(P::Percent),
                b'^' => Some(P::Caret),
                b'(' => Some(P::LParen),
                b')' => Some(P::RParen),
                b',' => Some(P::Comma),
                b'.' => Some(P::Dot),
                b'[' => Some(P::LBracket),
                b']' => Some(P::RBracket),
                b';' => Some(P::Semi),
                _ => None,
            };
            (p, 1)
        };
        match p {
            Some(p) => {
                toks.push(Token {
                    kind: TokKind::P(p),
                    span: Span::new(start, start + len),
                });
                i += len;
            }
            None => {
                // ANY_OTHER: one (possibly multi-byte) character.
                let ch_len = src[i..].chars().next().map_or(1, char::len_utf8);
                toks.push(Token {
                    kind: TokKind::Other,
                    span: Span::new(start, start + ch_len),
                });
                i += ch_len;
            }
        }
    }
    toks.push(Token {
        kind: TokKind::Eof,
        span: Span::new(b.len(), b.len()),
    });
    Ok(toks)
}

fn lex_decint(b: &[u8], i: usize) -> Option<usize> {
    match b.get(i) {
        Some(b'0') => Some(i + 1),
        Some(b'1'..=b'9') => {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            Some(j)
        }
        _ => None,
    }
}

fn lex_number(src: &str, i: usize) -> Result<(TokKind, usize), ParseError> {
    let b = src.as_bytes();
    let mut j = lex_decint(b, i).expect("called on a digit");
    let mut is_double = false;
    if b.get(j) == Some(&b'.') {
        is_double = true;
        j += 1;
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
    }
    if matches!(b.get(j), Some(b'e' | b'E')) {
        is_double = true;
        j += 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        match lex_decint(b, j) {
            Some(k) => j = k,
            None => {
                return Err(ParseError::new(
                    src,
                    Span::new(i, (j + 1).min(b.len())),
                    "malformed number: exponent digits expected",
                ));
            }
        }
    }
    Ok((
        if is_double {
            TokKind::Double
        } else {
            TokKind::DecInt
        },
        j,
    ))
}

fn lex_string(src: &str, start: usize) -> Result<(String, usize), ParseError> {
    let b = src.as_bytes();
    let quote = b[start];
    let mut j = start + 1;
    // First pass: find the end according to the lexer rule; collect raw body.
    loop {
        let Some(&c) = b.get(j) else {
            return Err(ParseError::new(
                src,
                Span::new(start, b.len()),
                "unterminated string literal",
            ));
        };
        match c {
            b'\\' => {
                match b.get(j + 1) {
                    Some(b'b' | b't' | b'n' | b'f' | b'r' | b'u' | b'"' | b'\'' | b'\\') => {}
                    _ => {
                        return Err(ParseError::new(
                            src,
                            Span::new(j, (j + 2).min(b.len())),
                            "invalid escape sequence in string literal",
                        ));
                    }
                }
                j += 2;
            }
            b'"' => {
                if quote == b'"' {
                    j += 1;
                    break;
                }
                return Err(ParseError::new(
                    src,
                    Span::new(j, j + 1),
                    "'\"' not allowed in a single-quoted string",
                ));
            }
            b'\'' if quote == b'\'' => {
                // ANTLR decision: close only if followed by EOF or '"'.
                if matches!(b.get(j + 1), None | Some(b'"')) {
                    j += 1;
                    break;
                }
                j += 1;
            }
            _ => j += 1,
        }
    }
    let body = &src[start + 1..j - 1];
    let value = unescape(body).map_err(|off| {
        ParseError::new(
            src,
            Span::new(start + 1 + off, (start + 1 + off + 2).min(j)),
            "Invalid unicode",
        )
    })?;
    Ok((value, j))
}

/// Xtext `STRINGValueConverter`: Java escapes; `\u` needs exactly four hex digits.
/// Returns the byte offset of a bad `\u` escape on error. Unpaired surrogates become U+FFFD.
fn unescape(body: &str) -> Result<String, usize> {
    let mut out = String::with_capacity(body.len());
    let mut units: Vec<u16> = Vec::new();
    let flush = |units: &mut Vec<u16>, out: &mut String| {
        if !units.is_empty() {
            out.extend(char::decode_utf16(units.drain(..)).map(|r| r.unwrap_or('\u{fffd}')));
        }
    };
    let mut it = body.char_indices().peekable();
    while let Some((off, c)) = it.next() {
        if c != '\\' {
            flush(&mut units, &mut out);
            out.push(c);
            continue;
        }
        let (_, e) = it.next().ok_or(off)?;
        let r = match e {
            'b' => '\u{8}',
            't' => '\t',
            'n' => '\n',
            'f' => '\u{c}',
            'r' => '\r',
            '"' => '"',
            '\'' => '\'',
            '\\' => '\\',
            'u' => {
                let mut v: u32 = 0;
                for _ in 0..4 {
                    match it.next() {
                        Some((_, h)) if h.is_ascii_hexdigit() => {
                            v = v * 16 + h.to_digit(16).expect("hex digit");
                        }
                        _ => return Err(off),
                    }
                }
                units.push(v as u16);
                continue;
            }
            _ => return Err(off),
        };
        flush(&mut units, &mut out);
        out.push(r);
    }
    flush(&mut units, &mut out);
    Ok(out)
}

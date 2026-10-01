//! Flat JSON-lines model shared by `trace.jsonl` and `tape.jsonl`: one object per line, scalar values
//! and arrays of numbers only, keys in emission order. Parsing followed by [`Record::write`] reproduces
//! reference lines byte for byte (numbers via [`crate::javafmt`], strings escaped like
//! `refsim.trace.Trace.str`).

use std::borrow::Cow;
use std::fmt;

use crate::javafmt;

/// A key: the known trace/tape keys are static (no allocation when parsing).
pub type Key = Cow<'static, str>;

const KNOWN_KEYS: &[&str] = &[
    "ev",
    "t",
    "p",
    "type",
    "id",
    "ac",
    "role",
    "sig",
    "kind",
    "name",
    "parent",
    "d",
    "idx",
    "sel",
    "n",
    "async",
    "sync",
    "res",
    "rc",
    "spec",
    "sched",
    "st",
    "avail",
    "queue",
    "mp",
    "metric",
    "v",
    "format",
    "run",
    "seed",
    "max_sim_time",
    "max_measurements",
    "uniforms",
    "measurements",
    "k",
    "i",
    "u",
    "o",
];

fn intern(k: &str) -> Key {
    for &kk in KNOWN_KEYS {
        if kk == k {
            return Cow::Borrowed(kk);
        }
    }
    Cow::Owned(k.to_string())
}

/// A JSON value as it occurs in trace/tape lines. Doubles and integers are kept apart by their lexical
/// form (Java doubles always contain `.`), so serialization round-trips.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Num(f64),
    Str(String),
    Bool(bool),
    /// Array of doubles (the `meas` value `v`); non-finite elements are written as strings.
    Nums(Vec<f64>),
    Null,
}

impl Value {
    /// Numeric view: `Int`, `Num`, and the non-finite strings `"NaN"`, `"Infinity"`, `"-Infinity"`.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Num(x) => Some(*x),
            Value::Str(s) => nonfinite(s),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    /// Appends the canonical JSON form.
    pub fn write(&self, out: &mut Vec<u8>) {
        match self {
            Value::Int(i) => {
                let mut b = javafmt::itoa_buf();
                out.extend_from_slice(javafmt::fmt_i64(&mut b, *i));
            }
            Value::Num(x) => javafmt::write_json(out, *x),
            Value::Str(s) => write_str(out, s),
            Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
            Value::Nums(v) => {
                out.push(b'[');
                for (i, x) in v.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    javafmt::write_json(out, *x);
                }
                out.push(b']');
            }
            Value::Null => out.extend_from_slice(b"null"),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut b = Vec::new();
        self.write(&mut b);
        f.write_str(&String::from_utf8_lossy(&b))
    }
}

pub(crate) fn nonfinite(s: &str) -> Option<f64> {
    match s {
        "NaN" => Some(f64::NAN),
        "Infinity" => Some(f64::INFINITY),
        "-Infinity" => Some(f64::NEG_INFINITY),
        _ => None,
    }
}

/// One JSON line: ordered key/value pairs.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Record {
    pub fields: Vec<(Key, Value)>,
}

impl Record {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Value::as_i64)
    }
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(Value::as_f64)
    }
    /// Canonical serialization (without the trailing newline).
    pub fn write(&self, out: &mut Vec<u8>) {
        out.push(b'{');
        for (i, (k, v)) in self.fields.iter().enumerate() {
            if i > 0 {
                out.push(b',');
            }
            write_str(out, k);
            out.push(b':');
            v.write(out);
        }
        out.push(b'}');
    }
    pub fn to_line(&self) -> String {
        let mut b = Vec::with_capacity(128);
        self.write(&mut b);
        String::from_utf8(b).expect("utf8")
    }
}

impl fmt::Display for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_line())
    }
}

/// Appends a JSON string escaped like the reference writer (`"`, `\`, `\n`, `\r`, `\t`, other
/// controls as `\u00xx`; everything else raw UTF-8).
pub fn write_str(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    let b = s.as_bytes();
    let mut start = 0;
    for (i, &c) in b.iter().enumerate() {
        let esc: &[u8] = match c {
            b'"' => b"\\\"",
            b'\\' => b"\\\\",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            b'\t' => b"\\t",
            0..=0x1f => b"",
            _ => continue,
        };
        out.extend_from_slice(&b[start..i]);
        if esc.is_empty() {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            out.extend_from_slice(b"\\u00");
            out.push(HEX[(c >> 4) as usize]);
            out.push(HEX[(c & 15) as usize]);
        } else {
            out.extend_from_slice(esc);
        }
        start = i + 1;
    }
    out.extend_from_slice(&b[start..]);
    out.push(b'"');
}

/// Parse error with byte position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub col: usize,
    pub msg: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}:{}: {}", self.line, self.col, self.msg)
    }
}

impl std::error::Error for ParseError {}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> P<'a> {
    fn err(&self, msg: impl Into<String>) -> ParseError {
        ParseError {
            line: 0,
            col: self.i + 1,
            msg: msg.into(),
        }
    }
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\r') {
            self.i += 1;
        }
    }
    fn expect(&mut self, c: u8) -> Result<(), ParseError> {
        self.ws();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(self.err(format!("expected '{}'", c as char)))
        }
    }
    fn string(&mut self) -> Result<String, ParseError> {
        self.expect(b'"')?;
        let start = self.i;
        // fast path: no escapes
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'"' => {
                    let s = std::str::from_utf8(&self.b[start..self.i])
                        .map_err(|_| self.err("invalid utf-8"))?;
                    self.i += 1;
                    return Ok(s.to_string());
                }
                b'\\' => break,
                _ => self.i += 1,
            }
        }
        let mut out: Vec<u8> = self.b[start..self.i].to_vec();
        while self.i < self.b.len() {
            let c = self.b[self.i];
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).map_err(|_| self.err("invalid utf-8")),
                b'\\' => {
                    let e = *self.b.get(self.i).ok_or_else(|| self.err("bad escape"))?;
                    self.i += 1;
                    match e {
                        b'"' => out.push(b'"'),
                        b'\\' => out.push(b'\\'),
                        b'/' => out.push(b'/'),
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let mut cp = self.hex4()?;
                            if (0xD800..0xDC00).contains(&cp)
                                && self.b.get(self.i) == Some(&b'\\')
                                && self.b.get(self.i + 1) == Some(&b'u')
                            {
                                self.i += 2;
                                let lo = self.hex4()?;
                                cp = 0x10000
                                    + ((cp - 0xD800) << 10)
                                    + (lo.wrapping_sub(0xDC00) & 0x3ff);
                            }
                            let ch = char::from_u32(cp).unwrap_or('\u{fffd}');
                            let mut tmp = [0u8; 4];
                            out.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                _ => out.push(c),
            }
        }
        Err(self.err("unterminated string"))
    }
    fn hex4(&mut self) -> Result<u32, ParseError> {
        let s = self
            .b
            .get(self.i..self.i + 4)
            .ok_or_else(|| self.err("short \\u escape"))?;
        let s = std::str::from_utf8(s).map_err(|_| self.err("bad \\u escape"))?;
        let v = u32::from_str_radix(s, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(v)
    }
    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.i;
        let mut float = false;
        while self.i < self.b.len() {
            match self.b[self.i] {
                b'0'..=b'9' | b'-' | b'+' => {}
                b'.' | b'e' | b'E' => float = true,
                _ => break,
            }
            self.i += 1;
        }
        let s = std::str::from_utf8(&self.b[start..self.i]).unwrap();
        if float {
            s.parse::<f64>()
                .map(Value::Num)
                .map_err(|_| self.err(format!("bad number '{s}'")))
        } else {
            match s.parse::<i64>() {
                Ok(v) => Ok(Value::Int(v)),
                Err(_) => s
                    .parse::<f64>()
                    .map(Value::Num)
                    .map_err(|_| self.err(format!("bad number '{s}'"))),
            }
        }
    }
    fn value(&mut self) -> Result<Value, ParseError> {
        self.ws();
        match self.b.get(self.i) {
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b't') if self.b[self.i..].starts_with(b"true") => {
                self.i += 4;
                Ok(Value::Bool(true))
            }
            Some(b'f') if self.b[self.i..].starts_with(b"false") => {
                self.i += 5;
                Ok(Value::Bool(false))
            }
            Some(b'n') if self.b[self.i..].starts_with(b"null") => {
                self.i += 4;
                Ok(Value::Null)
            }
            Some(b'[') => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Value::Nums(v));
                }
                loop {
                    let x = self.value()?;
                    v.push(
                        x.as_f64()
                            .ok_or_else(|| self.err("only numeric arrays are supported"))?,
                    );
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Value::Nums(v));
                        }
                        _ => return Err(self.err("expected ',' or ']'")),
                    }
                }
            }
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.err("unexpected character")),
        }
    }
}

/// Parses one flat JSON object line.
pub fn parse_line(line: &str) -> Result<Record, ParseError> {
    let mut p = P {
        b: line.as_bytes(),
        i: 0,
    };
    p.expect(b'{')?;
    let mut fields = Vec::with_capacity(8);
    p.ws();
    if p.b.get(p.i) == Some(&b'}') {
        return Ok(Record { fields });
    }
    loop {
        let k = p.string()?;
        p.expect(b':')?;
        let v = p.value()?;
        fields.push((intern(&k), v));
        p.ws();
        match p.b.get(p.i) {
            Some(b',') => {
                p.i += 1;
                p.ws();
            }
            Some(b'}') => {
                p.i += 1;
                break;
            }
            _ => return Err(p.err("expected ',' or '}'")),
        }
    }
    p.ws();
    if p.i != p.b.len() {
        return Err(p.err("trailing characters"));
    }
    Ok(Record { fields })
}

/// Parses all non-empty lines; errors carry the 1-based line number.
pub fn parse_lines(text: &str) -> Result<Vec<Record>, ParseError> {
    let mut out = Vec::with_capacity(text.len() / 100);
    for (n, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        out.push(parse_line(line).map_err(|mut e| {
            e.line = n + 1;
            e
        })?);
    }
    Ok(out)
}

/// Low-level line builder writing canonical JSON straight into a byte buffer (no allocation).
pub struct LineBuilder<'a> {
    out: &'a mut Vec<u8>,
    first: bool,
}

impl<'a> LineBuilder<'a> {
    pub fn new(out: &'a mut Vec<u8>) -> Self {
        out.push(b'{');
        LineBuilder { out, first: true }
    }
    #[inline]
    fn key(&mut self, k: &str) {
        if !self.first {
            self.out.push(b',');
        }
        self.first = false;
        self.out.push(b'"');
        self.out.extend_from_slice(k.as_bytes());
        self.out.extend_from_slice(b"\":");
    }
    /// String field (key must not need escaping).
    #[inline]
    pub fn str(&mut self, k: &str, v: &str) -> &mut Self {
        self.key(k);
        write_str(self.out, v);
        self
    }
    #[inline]
    pub fn int(&mut self, k: &str, v: i64) -> &mut Self {
        self.key(k);
        let mut b = javafmt::itoa_buf();
        self.out.extend_from_slice(javafmt::fmt_i64(&mut b, v));
        self
    }
    /// Double field (Java `Double.toString`, non-finite quoted).
    #[inline]
    pub fn num(&mut self, k: &str, v: f64) -> &mut Self {
        self.key(k);
        javafmt::write_json(self.out, v);
        self
    }
    #[inline]
    pub fn bool(&mut self, k: &str, v: bool) -> &mut Self {
        self.key(k);
        self.out
            .extend_from_slice(if v { b"true" } else { b"false" });
        self
    }
    #[inline]
    pub fn nums(&mut self, k: &str, v: &[f64]) -> &mut Self {
        self.key(k);
        self.out.push(b'[');
        for (i, x) in v.iter().enumerate() {
            if i > 0 {
                self.out.push(b',');
            }
            javafmt::write_json(self.out, *x);
        }
        self.out.push(b']');
        self
    }
    /// Raw pre-serialized JSON value.
    #[inline]
    pub fn raw(&mut self, k: &str, json: &[u8]) -> &mut Self {
        self.key(k);
        self.out.extend_from_slice(json);
        self
    }
    /// Closes the object and appends `\n`.
    #[inline]
    pub fn end(&mut self) {
        self.out.extend_from_slice(b"}\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        for l in [
            r#"{"ev":"header","t":0.0,"format":"palladio-trace/1","run":"h11","seed":10,"max_sim_time":-1,"max_measurements":50}"#,
            r#"{"ev":"meas","t":0.01,"mp":"A[x|replicaID=0]","metric":"State","v":[0.01,"NaN"]}"#,
            r#"{"k":"s","n":1,"o":"interarrival","spec":"a\"b\\c\n\u0001é","v":true}"#,
            r#"{"ev":"fork","t":1.0E-4,"p":2,"id":"x","async":0,"sync":2}"#,
        ] {
            let r = parse_line(l).unwrap();
            assert_eq!(r.to_line(), l);
        }
        let r = parse_line(r#"{"a":"é😀"}"#).unwrap();
        assert_eq!(r.get_str("a"), Some("é😀"));
    }

    #[test]
    fn builder() {
        let mut b = Vec::new();
        LineBuilder::new(&mut b)
            .str("ev", "hold")
            .num("t", 0.5)
            .int("p", 3)
            .num("d", f64::INFINITY)
            .nums("v", &[1.0, 2.5])
            .end();
        assert_eq!(
            String::from_utf8(b).unwrap(),
            "{\"ev\":\"hold\",\"t\":0.5,\"p\":3,\"d\":\"Infinity\",\"v\":[1.0,2.5]}\n"
        );
    }
}

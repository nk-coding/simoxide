//! `tape.jsonl` (random tape, `docs/guide/formats.md` §3): parser, writer, replay cursor and
//! origin-order checks.
//!
//! Replay mode: the simulator draws uniforms from a [`TapeReplay`] instead of its RNG. With
//! [`TapeReplay::next_checked`] every draw also asserts the origin tag, so the first draw made in a
//! different order or place is reported with its index and both tags.

use std::fmt;
use std::io::{self, Write};

use crate::json::{self, LineBuilder, ParseError, Value};

/// Derived value of an outermost StoEx evaluation (`v` of an `s` record).
#[derive(Clone, Debug, PartialEq)]
pub enum SampleValue {
    Double(f64),
    Int(i64),
    Bool(bool),
    Str(String),
}

impl SampleValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            SampleValue::Double(x) => Some(*x),
            SampleValue::Int(i) => Some(*i as f64),
            SampleValue::Bool(_) => None,
            SampleValue::Str(s) => json::nonfinite(s),
        }
    }
    fn from_value(v: &Value) -> SampleValue {
        match v {
            Value::Int(i) => SampleValue::Int(*i),
            Value::Num(x) => SampleValue::Double(*x),
            Value::Bool(b) => SampleValue::Bool(*b),
            Value::Str(s) => match json::nonfinite(s) {
                Some(x) => SampleValue::Double(x),
                None => SampleValue::Str(s.clone()),
            },
            other => SampleValue::Str(other.to_string()),
        }
    }
    fn to_value(&self) -> Value {
        match self {
            SampleValue::Double(x) => Value::Num(*x),
            SampleValue::Int(i) => Value::Int(*i),
            SampleValue::Bool(b) => Value::Bool(*b),
            SampleValue::Str(s) => Value::Str(s.clone()),
        }
    }
}

/// An `s` record.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// Index of the first uniform consumed by this evaluation.
    pub first_uniform: u64,
    /// Number of uniforms consumed.
    pub n: u64,
    pub origin: u32,
    /// StoEx, truncated to 64 UTF-16 units (61 + `...`) like the reference.
    pub spec: String,
    pub v: SampleValue,
}

/// A parsed tape in replay-friendly form: uniforms with interned origin tags, and samples.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tape {
    pub uniforms: Vec<f64>,
    /// Origin of every uniform (index into `origin_table`).
    pub origins: Vec<u32>,
    pub origin_table: Vec<String>,
    pub samples: Vec<Sample>,
    /// Record order: `true` = uniform, `false` = sample (to re-serialize byte-identically).
    order: Vec<bool>,
}

impl Tape {
    pub fn parse(text: &str) -> Result<Tape, ParseError> {
        let mut t = Tape::default();
        let mut table: std::collections::HashMap<String, u32> = Default::default();
        for (n, line) in text.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let err = |msg: &str| ParseError {
                line: n + 1,
                col: 0,
                msg: msg.to_string(),
            };
            let r = json::parse_line(line).map_err(|mut e| {
                e.line = n + 1;
                e
            })?;
            let o = r.get_str("o").ok_or_else(|| err("missing o"))?;
            let oid = match table.get(o) {
                Some(&i) => i,
                None => {
                    let i = t.origin_table.len() as u32;
                    t.origin_table.push(o.to_string());
                    table.insert(o.to_string(), i);
                    i
                }
            };
            match r.get_str("k") {
                Some("u") => {
                    let i = r.get_i64("i").ok_or_else(|| err("missing i"))?;
                    if i as usize != t.uniforms.len() {
                        return Err(err("uniform index out of sequence"));
                    }
                    t.uniforms
                        .push(r.get_f64("u").ok_or_else(|| err("missing u"))?);
                    t.origins.push(oid);
                    t.order.push(true);
                }
                Some("s") => {
                    let cnt = r.get_i64("n").ok_or_else(|| err("missing n"))? as u64;
                    let first = (t.uniforms.len() as u64)
                        .checked_sub(cnt)
                        .ok_or_else(|| err("sample consumes more uniforms than drawn"))?;
                    t.samples.push(Sample {
                        first_uniform: first,
                        n: cnt,
                        origin: oid,
                        spec: r.get_str("spec").unwrap_or("null").to_string(),
                        v: SampleValue::from_value(r.get("v").ok_or_else(|| err("missing v"))?),
                    });
                    t.order.push(false);
                }
                _ => return Err(err("unknown record kind")),
            }
        }
        Ok(t)
    }

    pub fn len(&self) -> usize {
        self.uniforms.len()
    }
    pub fn is_empty(&self) -> bool {
        self.uniforms.is_empty()
    }
    pub fn origin(&self, i: usize) -> &str {
        &self.origin_table[self.origins[i] as usize]
    }
    pub fn intern(&self, origin: &str) -> Option<u32> {
        self.origin_table
            .iter()
            .position(|o| o == origin)
            .map(|i| i as u32)
    }

    /// Canonical text (equal to the parsed input for reference tapes).
    pub fn to_text(&self) -> String {
        let mut w = TapeWriter::new(Vec::new());
        let (mut ui, mut si) = (0usize, 0usize);
        for &is_u in &self.order {
            if is_u {
                w.uniform(self.uniforms[ui], self.origin(ui));
                ui += 1;
            } else {
                let s = &self.samples[si];
                w.sample_raw(s.n, &self.origin_table[s.origin as usize], &s.spec, &s.v);
                si += 1;
            }
        }
        String::from_utf8(w.finish().unwrap()).unwrap()
    }

    /// A cursor for replay mode.
    pub fn replay(&self) -> TapeReplay<'_> {
        TapeReplay { tape: self, pos: 0 }
    }
}

/// Replay error: the tape ran out, or a draw happened with a different origin tag.
#[derive(Clone, Debug, PartialEq)]
pub enum ReplayError {
    Exhausted {
        index: u64,
        origin: String,
    },
    OriginMismatch {
        index: u64,
        expected: String,
        actual: String,
    },
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplayError::Exhausted { index, origin } => {
                write!(
                    f,
                    "tape exhausted at uniform #{index} (draw origin {origin})"
                )
            }
            ReplayError::OriginMismatch {
                index,
                expected,
                actual,
            } => write!(
                f,
                "uniform #{index}: reference drew for '{expected}', port draws for '{actual}'"
            ),
        }
    }
}

impl std::error::Error for ReplayError {}

/// Sequential reader of a tape's uniforms.
#[derive(Clone, Debug)]
pub struct TapeReplay<'a> {
    tape: &'a Tape,
    pos: usize,
}

impl<'a> TapeReplay<'a> {
    /// Index of the next uniform.
    pub fn position(&self) -> u64 {
        self.pos as u64
    }
    pub fn remaining(&self) -> usize {
        self.tape.uniforms.len() - self.pos
    }
    /// Next uniform without an origin check.
    #[inline]
    pub fn next_uniform(&mut self) -> Option<f64> {
        let u = self.tape.uniforms.get(self.pos).copied();
        if u.is_some() {
            self.pos += 1;
        }
        u
    }
    /// Origin tag of the next uniform.
    pub fn peek_origin(&self) -> Option<&'a str> {
        (self.pos < self.tape.uniforms.len()).then(|| self.tape.origin(self.pos))
    }
    /// Next uniform, asserting its origin tag (`purpose` or `purpose:elementId`).
    #[inline]
    pub fn next_checked(&mut self, origin: &str) -> Result<f64, ReplayError> {
        self.next_checked_parts(origin, None)
    }
    /// Like [`next_checked`](Self::next_checked) with the origin given as `purpose` + optional element
    /// id, compared without allocating.
    #[inline]
    pub fn next_checked_parts(
        &mut self,
        purpose: &str,
        id: Option<&str>,
    ) -> Result<f64, ReplayError> {
        let mk = || match id {
            Some(id) => format!("{purpose}:{id}"),
            None => purpose.to_string(),
        };
        let Some(&u) = self.tape.uniforms.get(self.pos) else {
            return Err(ReplayError::Exhausted {
                index: self.pos as u64,
                origin: mk(),
            });
        };
        let exp = self.tape.origin(self.pos);
        let ok = match id {
            None => exp == purpose,
            Some(id) => {
                exp.len() == purpose.len() + 1 + id.len()
                    && exp.starts_with(purpose)
                    && exp.as_bytes()[purpose.len()] == b':'
                    && exp.ends_with(id)
            }
        };
        if !ok {
            return Err(ReplayError::OriginMismatch {
                index: self.pos as u64,
                expected: exp.to_string(),
                actual: mk(),
            });
        }
        self.pos += 1;
        Ok(u)
    }
    /// The sample record whose evaluation ends right before the current position, if any (to check a
    /// derived value right after an evaluation).
    pub fn last_sample(&self) -> Option<&'a Sample> {
        let pos = self.pos as u64;
        let i = self
            .tape
            .samples
            .partition_point(|s| s.first_uniform + s.n <= pos);
        i.checked_sub(1)
            .map(|i| &self.tape.samples[i])
            .filter(|s| s.first_uniform + s.n == pos)
    }
}

/// First difference between two tapes.
#[derive(Clone, Debug, PartialEq)]
pub enum TapeMismatch {
    /// The uniform at `index` was drawn for different origins.
    Origin {
        index: u64,
        expected: String,
        actual: String,
    },
    /// Same origin, different uniform value (different RNG state).
    Uniform {
        index: u64,
        origin: String,
        expected: f64,
        actual: f64,
    },
    /// One tape is a strict prefix of the other.
    Length { expected: usize, actual: usize },
    /// Derived sample value differs (sample index, origin, spec).
    Sample {
        sample: usize,
        first_uniform: u64,
        origin: String,
        spec: String,
        expected: SampleValue,
        actual: SampleValue,
    },
}

impl fmt::Display for TapeMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TapeMismatch::Origin {
                index,
                expected,
                actual,
            } => write!(
                f,
                "uniform #{index}: origin '{actual}', reference '{expected}' (draw order/place differs)"
            ),
            TapeMismatch::Uniform {
                index,
                origin,
                expected,
                actual,
            } => write!(
                f,
                "uniform #{index} ({origin}): {actual:?}, reference {expected:?} (RNG stream differs)"
            ),
            TapeMismatch::Length { expected, actual } => {
                write!(f, "{actual} uniforms, reference {expected}")
            }
            TapeMismatch::Sample {
                sample,
                first_uniform,
                origin,
                spec,
                expected,
                actual,
            } => write!(
                f,
                "sample #{sample} (from uniform #{first_uniform}, {origin}, {spec}): {actual:?}, reference {expected:?}"
            ),
        }
    }
}

/// First origin-tag difference only (ignores values): tells whether the port draws in the same order
/// and places as the reference.
pub fn first_origin_mismatch(expected: &Tape, actual: &Tape) -> Option<TapeMismatch> {
    let n = expected.len().min(actual.len());
    for i in 0..n {
        if expected.origin(i) != actual.origin(i) {
            return Some(TapeMismatch::Origin {
                index: i as u64,
                expected: expected.origin(i).to_string(),
                actual: actual.origin(i).to_string(),
            });
        }
    }
    (expected.len() != actual.len()).then_some(TapeMismatch::Length {
        expected: expected.len(),
        actual: actual.len(),
    })
}

/// First difference of any kind: origin, then uniform bits, then length, then derived samples
/// (compared with relative tolerance `rel` on numeric values; 0 = exact).
pub fn first_mismatch(expected: &Tape, actual: &Tape, rel: f64) -> Option<TapeMismatch> {
    let n = expected.len().min(actual.len());
    for i in 0..n {
        if expected.origin(i) != actual.origin(i) {
            return first_origin_mismatch(expected, actual);
        }
        if expected.uniforms[i].to_bits() != actual.uniforms[i].to_bits() {
            return Some(TapeMismatch::Uniform {
                index: i as u64,
                origin: expected.origin(i).to_string(),
                expected: expected.uniforms[i],
                actual: actual.uniforms[i],
            });
        }
    }
    if expected.len() != actual.len() {
        return Some(TapeMismatch::Length {
            expected: expected.len(),
            actual: actual.len(),
        });
    }
    for (k, (a, b)) in expected.samples.iter().zip(&actual.samples).enumerate() {
        let same = match (a.v.as_f64(), b.v.as_f64()) {
            (Some(x), Some(y)) => crate::diff::float_eq(x, y, rel, 0.0),
            _ => a.v == b.v,
        };
        if !same || a.first_uniform != b.first_uniform || a.n != b.n {
            return Some(TapeMismatch::Sample {
                sample: k,
                first_uniform: a.first_uniform,
                origin: expected.origin_table[a.origin as usize].clone(),
                spec: a.spec.clone(),
                expected: a.v.clone(),
                actual: b.v.clone(),
            });
        }
    }
    None
}

/// Truncates a StoEx like the reference (`spec.length() <= 64 ? spec : spec.substring(0, 61) + "..."`,
/// lengths in UTF-16 code units).
pub fn truncate_spec(spec: &str) -> std::borrow::Cow<'_, str> {
    let units: usize = spec.chars().map(char::len_utf16).sum();
    if units <= 64 {
        return spec.into();
    }
    let mut out = String::new();
    let mut n = 0;
    for c in spec.chars() {
        let l = c.len_utf16();
        if n + l > 61 {
            // a split surrogate pair: Java keeps the high surrogate; we cannot, so stop before it
            break;
        }
        n += l;
        out.push(c);
    }
    out.push_str("...");
    out.into()
}

/// Streaming tape writer (`u` and `s` records).
pub struct TapeWriter<W: Write> {
    out: W,
    buf: Vec<u8>,
    err: Option<io::Error>,
    next_index: u64,
}

impl<W: Write> TapeWriter<W> {
    pub fn new(out: W) -> Self {
        TapeWriter {
            out,
            buf: Vec::with_capacity(1 << 16),
            err: None,
            next_index: 0,
        }
    }
    /// Number of uniforms written.
    pub fn uniforms(&self) -> u64 {
        self.next_index
    }
    fn after(&mut self) {
        if self.buf.len() >= 1 << 16 {
            if self.err.is_none()
                && let Err(e) = self.out.write_all(&self.buf)
            {
                self.err = Some(e);
            }
            self.buf.clear();
        }
    }
    /// One uniform draw; the index is assigned automatically.
    pub fn uniform(&mut self, u: f64, origin: &str) {
        let i = self.next_index;
        self.next_index += 1;
        LineBuilder::new(&mut self.buf)
            .str("k", "u")
            .int("i", i as i64)
            .num("u", u)
            .str("o", origin)
            .end();
        self.after();
    }
    /// Sample record of an outermost evaluation that consumed `n` > 0 uniforms (`spec` is truncated
    /// here).
    pub fn sample(&mut self, n: u64, origin: &str, spec: &str, v: &SampleValue) {
        let spec = truncate_spec(spec);
        self.sample_raw(n, origin, &spec, v);
    }
    fn sample_raw(&mut self, n: u64, origin: &str, spec: &str, v: &SampleValue) {
        let mut vb = Vec::with_capacity(24);
        v.to_value().write(&mut vb);
        LineBuilder::new(&mut self.buf)
            .str("k", "s")
            .int("n", n as i64)
            .str("o", origin)
            .str("spec", spec)
            .raw("v", &vb)
            .end();
        self.after();
    }
    pub fn finish(mut self) -> io::Result<W> {
        if self.err.is_none() {
            self.out.write_all(&self.buf)?;
        }
        if let Some(e) = self.err.take() {
            return Err(e);
        }
        self.out.flush()?;
        Ok(self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = r#"{"k":"u","i":0,"u":0.30515901412890334,"o":"interarrival"}
{"k":"s","n":1,"o":"interarrival","spec":"Exp(2.0)","v":0.1820361284516083}
{"k":"u","i":1,"u":0.2466654361678413,"o":"demand:_act2"}
{"k":"u","i":2,"u":0.5,"o":"demand:_act2"}
{"k":"s","n":2,"o":"demand:_act2","spec":"IntPMF[(1;0.5)(2;0.5)]","v":2}
{"k":"u","i":3,"u":0.1,"o":"guard:_bt"}
{"k":"s","n":1,"o":"guard:_bt","spec":"x","v":true}
"#;

    #[test]
    fn parse_roundtrip_and_replay() {
        let t = Tape::parse(T).unwrap();
        assert_eq!(t.to_text(), T);
        assert_eq!(t.len(), 4);
        assert_eq!(t.samples[1].first_uniform, 1);
        let mut r = t.replay();
        assert_eq!(r.next_checked("interarrival").unwrap(), 0.30515901412890334);
        assert_eq!(r.last_sample().unwrap().spec, "Exp(2.0)");
        assert!(r.next_checked_parts("demand", Some("_act2")).is_ok());
        let e = r.next_checked_parts("demand", Some("_act3")).unwrap_err();
        assert!(matches!(e, ReplayError::OriginMismatch { index: 2, .. }));
        assert_eq!(r.next_uniform(), Some(0.5));
        assert_eq!(r.last_sample().unwrap().v, SampleValue::Int(2));
        r.next_uniform();
        assert!(matches!(
            r.next_checked("x"),
            Err(ReplayError::Exhausted { index: 4, .. })
        ));
    }

    #[test]
    fn mismatch() {
        let a = Tape::parse(T).unwrap();
        let b = Tape::parse(&T.replace(
            "\"i\":2,\"u\":0.5,\"o\":\"demand:_act2\"",
            "\"i\":2,\"u\":0.5,\"o\":\"loop:_l\"",
        ))
        .unwrap();
        assert!(matches!(
            first_origin_mismatch(&a, &b),
            Some(TapeMismatch::Origin { index: 2, .. })
        ));
        assert_eq!(first_mismatch(&a, &a, 0.0), None);
        let c = Tape::parse(&T.replace("0.1820361284516083", "0.18")).unwrap();
        assert!(matches!(
            first_mismatch(&a, &c, 0.0),
            Some(TapeMismatch::Sample { sample: 0, .. })
        ));
    }

    #[test]
    fn truncation() {
        let s = "a".repeat(70);
        assert_eq!(truncate_spec(&s), format!("{}...", "a".repeat(61)));
        assert_eq!(truncate_spec(&"b".repeat(64)), "b".repeat(64));
    }
}

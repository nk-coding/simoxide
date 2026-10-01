//! The run's single uniform stream: the reference generator or a replayed tape, plus the random
//! tape writer (`docs/guide/formats.md` §3).

use crate::javafmt::{push_json_double, push_json_str};
use simoxide_random::{SimuComStream, UniformSource};
use simoxide_stoex::Value;
use std::io::Write;
use std::sync::Arc;

/// Where the uniforms come from.
#[derive(Clone, Debug, Default)]
pub enum RngMode {
    /// The reference stream seeded like `refsim --seed N` (`fixedSeed_i = N + i`).
    #[default]
    Own,
    /// Replay the uniforms of a recorded tape, in order.
    Replay(Arc<Tape>),
}

/// A recorded random tape: uniforms with their origin tags.
#[derive(Clone, Debug, Default)]
pub struct Tape {
    pub uniforms: Vec<f64>,
    /// Origin tag of every uniform (checked in replay if `check_origins`).
    pub origins: Vec<Box<str>>,
    /// `s` records: index of the first uniform of the evaluation -> (uniforms used, value).
    pub samples: std::collections::HashMap<u64, (u64, Value)>,
}

/// Parses the `v` of an `s` record the way the reference printed it.
fn parse_sample_value(raw: &str) -> Option<Value> {
    if let Some(inner) = raw.strip_prefix('"').and_then(|x| x.strip_suffix('"')) {
        return Some(match inner {
            "NaN" => Value::Double(f64::NAN),
            "Infinity" => Value::Double(f64::INFINITY),
            "-Infinity" => Value::Double(f64::NEG_INFINITY),
            s => Value::Str(s.into()),
        });
    }
    match raw {
        "true" => return Some(Value::Bool(true)),
        "false" => return Some(Value::Bool(false)),
        _ => {}
    }
    if raw.contains(['.', 'E']) {
        raw.parse().ok().map(Value::Double)
    } else {
        raw.parse().ok().map(Value::Int)
    }
}

impl Tape {
    /// Parses the `u` records of a `tape.jsonl` file (other records are ignored).
    pub fn parse(text: &str) -> Result<Tape, String> {
        let mut t = Tape::default();
        for (ln, line) in text.lines().enumerate() {
            // {"k":"s","n":<n>,"o":"<origin>","spec":"<stoex>","v":<value>}
            if let Some(rest) = line.strip_prefix("{\"k\":\"s\",\"n\":") {
                let comma = rest.find(',').unwrap_or(0);
                let n: u64 = rest[..comma].parse().unwrap_or(0);
                if let Some(vpos) = rest.rfind(",\"v\":")
                    && let Some(raw) = rest[vpos + 5..].strip_suffix('}')
                    && let Some(v) = parse_sample_value(raw)
                    && n > 0
                {
                    let start = t.uniforms.len() as u64 - n;
                    t.samples.insert(start, (n, v));
                }
                continue;
            }
            // {"k":"u","i":<n>,"u":<uniform>,"o":"<origin>"}
            let Some(rest) = line.strip_prefix("{\"k\":\"u\",") else {
                continue;
            };
            let bad = || format!("tape line {}: malformed uniform record", ln + 1);
            let upos = rest.find(",\"u\":").ok_or_else(bad)?;
            let rest = &rest[upos + 5..];
            let comma = rest.find(',').ok_or_else(bad)?;
            // correctly rounded parse (the value is Java's shortest round-trip repr)
            let u: f64 = rest[..comma].parse().map_err(|_| bad())?;
            let o = rest[comma..]
                .strip_prefix(",\"o\":")
                .and_then(|x| x.strip_suffix('}'))
                .ok_or_else(bad)?;
            let o: String = serde_json::from_str(o).map_err(|_| bad())?;
            t.uniforms.push(u);
            t.origins.push(o.into());
        }
        Ok(t)
    }
}

enum Src {
    Mt(Box<SimuComStream>),
    Replay { tape: Arc<Tape>, pos: usize },
}

/// The exact mode's uniform source (the reference stream or a replayed tape, with the tape
/// bookkeeping); internal, public only as `compat::Exact::Rng`.
#[doc(hidden)]
pub struct Rng {
    src: Src,
    /// Uniforms drawn so far.
    pub count: u64,
    /// Current origin tag (only maintained when needed).
    origin: String,
    need_origin: bool,
    check_origins: bool,
    tape: Option<TapeOut>,
    /// First problem seen (tape exhausted, origin mismatch).
    pub problem: Option<String>,
    depth: u32,
    eval_start: u64,
    use_samples: bool,
    /// Own stream without tape output: evaluations need no bookkeeping (origins, `s` records,
    /// recorded samples).
    pub plain: bool,
}

pub(crate) struct TapeOut {
    buf: String,
    out: Box<dyn Write>,
}

impl TapeOut {
    fn flush_if_full(&mut self) {
        if self.buf.len() > (1 << 16) {
            let _ = self.out.write_all(self.buf.as_bytes());
            self.buf.clear();
        }
    }
    pub fn finish(&mut self) {
        let _ = self.out.write_all(self.buf.as_bytes());
        self.buf.clear();
        let _ = self.out.flush();
    }
}

/// Why a uniform is drawn (`purpose[:elementId]`).
#[doc(hidden)]
#[derive(Clone, Copy)]
pub enum Origin<'a> {
    Plain(&'a str),
    Elem(&'static str, &'a str),
}

impl Rng {
    pub fn new(
        mode: &RngMode,
        seed: i64,
        tape: Option<Box<dyn Write>>,
        check_origins: bool,
        use_samples: bool,
    ) -> Result<Rng, String> {
        let src = match mode {
            RngMode::Own => {
                let seeds: Vec<i64> = (0..6).map(|i| seed + i).collect();
                Src::Mt(Box::new(
                    SimuComStream::from_seed(&seeds).map_err(|e| e.to_string())?,
                ))
            }
            RngMode::Replay(t) => Src::Replay {
                tape: t.clone(),
                pos: 0,
            },
        };
        let check = check_origins && matches!(mode, RngMode::Replay(_));
        let plain = matches!(mode, RngMode::Own) && tape.is_none();
        Ok(Rng {
            src,
            count: 0,
            origin: "?".into(),
            need_origin: tape.is_some() || check,
            check_origins: check,
            tape: tape.map(|out| TapeOut {
                buf: String::with_capacity(1 << 17),
                out,
            }),
            problem: None,
            depth: 0,
            eval_start: 0,
            use_samples,
            plain,
        })
    }

    /// Sets the origin tag; returns the previous one (to restore).
    #[inline]
    pub fn set_origin(&mut self, o: Origin<'_>) -> Option<String> {
        if !self.need_origin {
            return None;
        }
        let mut s = String::new();
        match o {
            Origin::Plain(p) => s.push_str(p),
            Origin::Elem(p, id) => {
                s.push_str(p);
                s.push(':');
                s.push_str(id);
            }
        }
        Some(std::mem::replace(&mut self.origin, s))
    }

    #[inline]
    pub fn restore_origin(&mut self, prev: Option<String>) {
        if let Some(p) = prev {
            self.origin = p;
        }
    }

    /// Start of a `StackContext.evaluateStatic` (nesting allowed).
    #[inline]
    pub fn eval_begin(&mut self) {
        if self.depth == 0 {
            self.eval_start = self.count;
        }
        self.depth += 1;
    }

    /// In replay mode with recorded samples: the reference's result of the outermost evaluation
    /// that is about to end (replaces the locally computed value, so that inexact distribution
    /// sampling does not make a replayed run diverge).
    #[inline]
    pub fn recorded_sample(&self) -> Option<Value> {
        if !self.use_samples || self.depth != 1 || self.count == self.eval_start {
            return None;
        }
        let Src::Replay { tape, .. } = &self.src else {
            return None;
        };
        match tape.samples.get(&self.eval_start) {
            Some((n, v)) if *n == self.count - self.eval_start => Some(v.clone()),
            _ => None,
        }
    }

    /// End of an evaluation: writes the `s` record of an outermost evaluation that drew.
    #[inline]
    pub fn eval_end(&mut self, spec: &str, v: Option<&Value>) {
        self.depth = self.depth.saturating_sub(1);
        if self.depth == 0
            && self.count > self.eval_start
            && let (Some(t), Some(v)) = (self.tape.as_mut(), v)
        {
            let b = &mut t.buf;
            b.push_str("{\"k\":\"s\",\"n\":");
            b.push_str(&(self.count - self.eval_start).to_string());
            b.push_str(",\"o\":");
            push_json_str(b, &self.origin);
            b.push_str(",\"spec\":");
            let n = spec.encode_utf16().count();
            if n <= 64 {
                push_json_str(b, spec);
            } else {
                let units: Vec<u16> = spec.encode_utf16().take(61).collect();
                let mut s = String::from_utf16_lossy(&units);
                s.push_str("...");
                push_json_str(b, &s);
            }
            b.push_str(",\"v\":");
            match v {
                Value::Double(d) => push_json_double(b, *d),
                Value::Int(i) => b.push_str(&i.to_string()),
                Value::Bool(x) => b.push_str(if *x { "true" } else { "false" }),
                other => push_json_str(b, &other.to_string()),
            }
            b.push_str("}\n");
            t.flush_if_full();
        }
    }

    pub fn finish(&mut self) {
        if let Some(t) = self.tape.as_mut() {
            t.finish();
        }
    }
}

impl UniformSource for Rng {
    #[inline]
    fn next_uniform(&mut self) -> f64 {
        let i = self.count;
        self.count += 1;
        let u = match &mut self.src {
            Src::Mt(mt) => mt.next_uniform(),
            Src::Replay { tape, pos } => {
                let p = *pos;
                *pos += 1;
                match tape.uniforms.get(p) {
                    Some(&u) => {
                        if self.check_origins
                            && self.problem.is_none()
                            && *tape.origins[p] != *self.origin
                        {
                            self.problem = Some(format!(
                                "uniform {p}: origin '{}' but tape has '{}'",
                                self.origin, tape.origins[p]
                            ));
                        }
                        u
                    }
                    None => {
                        if self.problem.is_none() {
                            self.problem =
                                Some(format!("random tape exhausted after {p} uniforms"));
                        }
                        0.5
                    }
                }
            }
        };
        if let Some(t) = self.tape.as_mut() {
            let b = &mut t.buf;
            b.push_str("{\"k\":\"u\",\"i\":");
            b.push_str(&i.to_string());
            b.push_str(",\"u\":");
            push_json_double(b, u);
            b.push_str(",\"o\":");
            push_json_str(b, &self.origin);
            b.push_str("}\n");
            t.flush_if_full();
        }
        u
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAPE: &str = r#"{"k":"u","i":0,"u":0.4417607209716412,"o":"interarrival"}
{"k":"s","n":1,"o":"interarrival","spec":"Exp(2.0)","v":0.29148379652644635}
{"k":"u","i":1,"u":0.8745851024981088,"o":"demand:a1"}
{"k":"u","i":2,"u":0.25,"o":"demand:a1"}
{"k":"s","n":2,"o":"demand:a1","spec":"IntPMF[(1;0.5)(2;0.5)] + Exp(1.0)","v":3}
"#;

    #[test]
    fn parses_uniforms_origins_and_samples() {
        let t = Tape::parse(TAPE).unwrap();
        assert_eq!(
            t.uniforms,
            vec![0.4417607209716412, 0.8745851024981088, 0.25]
        );
        assert_eq!(&*t.origins[1], "demand:a1");
        assert!(matches!(t.samples.get(&0), Some((1, Value::Double(_)))));
        assert!(matches!(t.samples.get(&1), Some((2, Value::Int(3)))));
    }

    #[test]
    fn replay_reports_exhaustion_and_origin_mismatch() {
        let t = Arc::new(Tape::parse(TAPE).unwrap());
        let mut r = Rng::new(&RngMode::Replay(t), 0, None, true, false).unwrap();
        let prev = r.set_origin(Origin::Plain("interarrival"));
        assert_eq!(r.next_uniform(), 0.4417607209716412);
        r.restore_origin(prev);
        assert!(r.problem.is_none());
        r.next_uniform(); // origin "?" vs "demand:a1"
        assert!(r.problem.as_deref().unwrap().contains("origin"));
        r.next_uniform();
        r.problem = None;
        r.next_uniform();
        assert!(r.problem.as_deref().unwrap().contains("exhausted"));
    }

    #[test]
    fn own_stream_is_seeded_like_refsim() {
        // first uniform of h01_ps_single (seed 1)
        let mut r = Rng::new(&RngMode::Own, 1, None, false, false).unwrap();
        assert_eq!(r.next_uniform(), 0.4417607209716412);
        assert_eq!(r.count, 1);
    }
}

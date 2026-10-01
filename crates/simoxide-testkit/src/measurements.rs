//! `measurements.csv` (`docs/guide/formats.md` §4): parser, recorder/writer (series sorted in Java
//! `String.compareTo` order of `mp + "\0" + metric`, rows in emission order).

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

use crate::javafmt;

pub const CSV_HEADER: &str = "measuring_point,metric,time,value";

/// One measurement series (measuring point x metric).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Series {
    pub mp: String,
    pub metric: String,
    pub times: Vec<f64>,
    pub values: Vec<f64>,
}

impl Series {
    pub fn len(&self) -> usize {
        self.times.len()
    }
    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }
    pub fn key(&self) -> String {
        format!("{} / {}", self.mp, self.metric)
    }
}

/// All series of a run, in file order.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Measurements {
    pub series: Vec<Series>,
}

/// CSV parse error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CsvError {
    pub line: usize,
    pub msg: String,
}

impl fmt::Display for CsvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "measurements.csv line {}: {}", self.line, self.msg)
    }
}

impl std::error::Error for CsvError {}

/// Java `String.compareTo` (UTF-16 code unit order).
pub fn java_cmp(a: &str, b: &str) -> Ordering {
    if a.is_ascii() && b.is_ascii() {
        return a.cmp(b);
    }
    a.encode_utf16().cmp(b.encode_utf16())
}

fn split_csv(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::with_capacity(4);
    let b = line.as_bytes();
    let mut i = 0;
    loop {
        if b.get(i) == Some(&b'"') {
            let mut s = Vec::new();
            i += 1;
            loop {
                match b.get(i) {
                    None => return Err("unterminated quoted field".into()),
                    Some(b'"') if b.get(i + 1) == Some(&b'"') => {
                        s.push(b'"');
                        i += 2;
                    }
                    Some(b'"') => {
                        i += 1;
                        break;
                    }
                    Some(&c) => {
                        s.push(c);
                        i += 1;
                    }
                }
            }
            out.push(String::from_utf8(s).map_err(|_| "invalid utf-8")?);
        } else {
            let start = i;
            while i < b.len() && b[i] != b',' {
                i += 1;
            }
            out.push(line[start..i].to_string());
        }
        match b.get(i) {
            None => return Ok(out),
            Some(b',') => i += 1,
            Some(_) => return Err("garbage after quoted field".into()),
        }
    }
}

fn parse_num(s: &str) -> Option<f64> {
    crate::json::nonfinite(s).or_else(|| s.parse().ok())
}

/// Quotes a CSV field like the reference (`,` `"` or newline).
pub fn csv_field(out: &mut Vec<u8>, s: &str) {
    if s.contains([',', '"', '\n']) {
        out.push(b'"');
        out.extend_from_slice(s.replace('"', "\"\"").as_bytes());
        out.push(b'"');
    } else {
        out.extend_from_slice(s.as_bytes());
    }
}

impl Measurements {
    /// Parses the CSV (fields may contain quoted newlines only if the key does, which the reference
    /// never produces; lines are split on `\n`).
    pub fn parse(text: &str) -> Result<Measurements, CsvError> {
        let mut m = Measurements::default();
        let mut idx: HashMap<(String, String), usize> = HashMap::new();
        for (n, line) in text.lines().enumerate() {
            if n == 0 {
                if line != CSV_HEADER {
                    return Err(CsvError {
                        line: 1,
                        msg: format!("bad header '{line}'"),
                    });
                }
                continue;
            }
            if line.is_empty() {
                continue;
            }
            let err = |msg: String| CsvError { line: n + 1, msg };
            let f = split_csv(line).map_err(|e| err(e.to_string()))?;
            if f.len() != 4 {
                return Err(err(format!("{} fields", f.len())));
            }
            let t = parse_num(&f[2]).ok_or_else(|| err(format!("bad time '{}'", f[2])))?;
            let v = parse_num(&f[3]).ok_or_else(|| err(format!("bad value '{}'", f[3])))?;
            let mut it = f.into_iter();
            let mp = it.next().unwrap();
            let metric = it.next().unwrap();
            let k = (mp, metric);
            let i = match idx.get(&k) {
                Some(&i) => i,
                None => {
                    let i = m.series.len();
                    m.series.push(Series {
                        mp: k.0.clone(),
                        metric: k.1.clone(),
                        ..Default::default()
                    });
                    idx.insert(k, i);
                    i
                }
            };
            m.series[i].times.push(t);
            m.series[i].values.push(v);
        }
        Ok(m)
    }

    pub fn total(&self) -> usize {
        self.series.iter().map(Series::len).sum()
    }

    pub fn get(&self, mp: &str, metric: &str) -> Option<&Series> {
        self.series
            .iter()
            .find(|s| s.mp == mp && s.metric == metric)
    }

    /// Sorts series into file order.
    pub fn sort(&mut self) {
        self.series.sort_by(|a, b| {
            java_cmp(
                &format!("{}\0{}", a.mp, a.metric),
                &format!("{}\0{}", b.mp, b.metric),
            )
        });
    }

    /// Canonical CSV (series must be in file order; see [`sort`](Self::sort)).
    pub fn to_csv(&self) -> String {
        let mut out = Vec::with_capacity(64 + self.total() * 80);
        out.extend_from_slice(CSV_HEADER.as_bytes());
        out.push(b'\n');
        for s in &self.series {
            let mut prefix = Vec::new();
            csv_field(&mut prefix, &s.mp);
            prefix.push(b',');
            csv_field(&mut prefix, &s.metric);
            prefix.push(b',');
            for (t, v) in s.times.iter().zip(&s.values) {
                out.extend_from_slice(&prefix);
                javafmt::write_json(&mut out, *t);
                out.push(b',');
                javafmt::write_json(&mut out, *v);
                out.push(b'\n');
            }
        }
        String::from_utf8(out).expect("utf8")
    }
}

/// Handle of a series in a [`Recorder`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SeriesId(pub u32);

/// Measurement sink for simulators: register series once, then record tuples without allocation.
#[derive(Clone, Debug, Default)]
pub struct Recorder {
    series: Vec<Series>,
    index: HashMap<(String, String), SeriesId>,
    total: u64,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }
    /// Id of the series for (`mp`, `metric`), created on first use.
    pub fn series_id(&mut self, mp: &str, metric: &str) -> SeriesId {
        if let Some(&id) = self.index.get(&(mp.to_string(), metric.to_string())) {
            return id;
        }
        let id = SeriesId(self.series.len() as u32);
        self.series.push(Series {
            mp: mp.to_string(),
            metric: metric.to_string(),
            ..Default::default()
        });
        self.index.insert((mp.to_string(), metric.to_string()), id);
        id
    }
    #[inline]
    pub fn record(&mut self, id: SeriesId, time: f64, value: f64) {
        let s = &mut self.series[id.0 as usize];
        s.times.push(time);
        s.values.push(value);
        self.total += 1;
    }
    pub fn total(&self) -> u64 {
        self.total
    }
    /// Sorted measurements (series with no rows are dropped, like the reference CSV).
    pub fn into_measurements(self) -> Measurements {
        let mut m = Measurements {
            series: self.series.into_iter().filter(|s| !s.is_empty()).collect(),
        };
        m.sort();
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_order() {
        let mut r = Recorder::new();
        let b = r.series_id("B[x]", "Response Time Tuple");
        let a = r.series_id("A[y|replicaID=0]", "State of Active Resource Tuple");
        let q = r.series_id("A[y,z]", "M\"q");
        r.record(b, 1.0, 0.5);
        r.record(a, 0.0, 1.0);
        r.record(b, 2.0, f64::NAN);
        r.record(q, 1.0E7, 1.0E-4);
        let m = r.into_measurements();
        let csv = m.to_csv();
        assert_eq!(
            csv,
            "measuring_point,metric,time,value\n\"A[y,z]\",\"M\"\"q\",1.0E7,1.0E-4\n\
             A[y|replicaID=0],State of Active Resource Tuple,0.0,1.0\n\
             B[x],Response Time Tuple,1.0,0.5\nB[x],Response Time Tuple,2.0,\"NaN\"\n"
        );
        let p = Measurements::parse(&csv).unwrap();
        assert_eq!(p.to_csv(), csv);
        assert_eq!(p.total(), 4);
    }

    #[test]
    fn java_order() {
        // "A\0..." sorts before "AB": NUL separator
        assert_eq!(java_cmp("A\0z", "AB\0a"), Ordering::Less);
        // supplementary char (surrogates D800..) sorts before U+FFFD in UTF-16
        assert_eq!(java_cmp("\u{1F600}", "\u{FFFD}"), Ordering::Less);
    }
}

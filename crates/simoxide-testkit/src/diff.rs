//! Trace differ: aligns two traces line by line and reports the first divergence with context
//! (preceding lines, differing fields, process and its open model elements).
//!
//! Modes:
//! - [`DiffMode::Exact`]: byte-identical lines (the goal for tape replay and own-RNG runs);
//! - [`DiffMode::Tolerance`]: same events and fields, doubles (incl. `t`) within a relative/absolute
//!   tolerance;
//! - [`DiffMode::MeasurementsOnly`]: only `meas` events are compared (with tolerance);
//! - [`DiffMode::PerProcess`]: each process's event sequence is compared separately (tolerates a
//!   different interleaving of processes at equal times; tells "ordering bug" from "semantics bug").

use std::fmt;

use crate::json::{ParseError, Record, Value};
use crate::trace::{EventKind, Trace, describe_element, process_stack};

/// Relative and absolute tolerance for doubles; `Tolerance::EXACT` compares bits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tolerance {
    pub rel: f64,
    pub abs: f64,
}

impl Tolerance {
    pub const EXACT: Tolerance = Tolerance { rel: 0.0, abs: 0.0 };
    /// The PLAN's replay tolerance for times (1e-9 relative).
    pub const REPLAY: Tolerance = Tolerance {
        rel: 1e-9,
        abs: 1e-12,
    };
    pub fn eq(&self, a: f64, b: f64) -> bool {
        float_eq(a, b, self.rel, self.abs)
    }
}

/// `a == b` (NaN equals NaN), or within `abs`, or within `rel * max(|a|, |b|)`.
pub fn float_eq(a: f64, b: f64, rel: f64, abs: f64) -> bool {
    if a == b || (a.is_nan() && b.is_nan()) {
        return true;
    }
    let d = (a - b).abs();
    d <= abs || d <= rel * a.abs().max(b.abs())
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DiffMode {
    Exact,
    Tolerance(Tolerance),
    MeasurementsOnly(Tolerance),
    PerProcess(Tolerance),
}

#[derive(Clone, Debug)]
pub struct DiffOptions {
    pub mode: DiffMode,
    /// Lines of context before the divergence.
    pub context: usize,
    /// How far to look ahead for re-synchronisation (insertion/deletion classification).
    pub lookahead: usize,
}

impl Default for DiffOptions {
    fn default() -> Self {
        DiffOptions {
            mode: DiffMode::Exact,
            context: 8,
            lookahead: 50,
        }
    }
}

impl DiffOptions {
    pub fn exact() -> Self {
        Self::default()
    }
    pub fn tolerance(rel: f64, abs: f64) -> Self {
        DiffOptions {
            mode: DiffMode::Tolerance(Tolerance { rel, abs }),
            ..Self::default()
        }
    }
    pub fn measurements_only(rel: f64, abs: f64) -> Self {
        DiffOptions {
            mode: DiffMode::MeasurementsOnly(Tolerance { rel, abs }),
            ..Self::default()
        }
    }
    pub fn per_process(rel: f64, abs: f64) -> Self {
        DiffOptions {
            mode: DiffMode::PerProcess(Tolerance { rel, abs }),
            ..Self::default()
        }
    }
}

/// One differing field.
#[derive(Clone, Debug, PartialEq)]
pub struct FieldDiff {
    pub key: String,
    pub expected: Option<Value>,
    pub actual: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DivergenceKind {
    /// The lines differ (see `fields`).
    Changed,
    /// The actual trace has `n` extra lines here, then agrees again.
    Extra(usize),
    /// The actual trace lacks `n` expected lines here, then agrees again.
    Missing(usize),
    /// The actual trace ended early.
    ActualEnded,
    /// The actual trace continues after the expected one ended.
    ExpectedEnded,
}

/// The first divergence.
#[derive(Clone, Debug)]
pub struct Divergence {
    pub kind: DivergenceKind,
    /// 1-based line numbers in the expected / actual trace files.
    pub expected_line: usize,
    pub actual_line: usize,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub fields: Vec<FieldDiff>,
    /// Preceding expected lines `(line number, text)` (in the compared subsequence).
    pub context: Vec<(usize, String)>,
    pub time: Option<f64>,
    pub process: Option<i64>,
    /// `kind 'name' (parent p=..)` of the process.
    pub process_info: Option<String>,
    /// Open `begin` elements of the process (outermost first).
    pub process_stack: Vec<String>,
    /// The model element of the differing event.
    pub element: Option<String>,
}

/// Result of a trace comparison.
#[derive(Clone, Debug)]
pub struct DiffReport {
    pub mode: DiffMode,
    pub expected_len: usize,
    pub actual_len: usize,
    /// Number of equal lines before the divergence (all compared lines if equal).
    pub equal_prefix: usize,
    pub divergence: Option<Divergence>,
}

impl DiffReport {
    pub fn is_equal(&self) -> bool {
        self.divergence.is_none()
    }
}

fn cmp_values(k: &str, a: &Value, b: &Value, tol: Option<Tolerance>) -> bool {
    let _ = k;
    match (a, b, tol) {
        (_, _, None) => a == b,
        (Value::Num(x), Value::Num(y), Some(t)) => t.eq(*x, *y),
        (Value::Nums(x), Value::Nums(y), Some(t)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| t.eq(*p, *q))
        }
        _ => match (a.as_f64(), b.as_f64(), tol) {
            (Some(x), Some(y), Some(t)) if matches!(a, Value::Num(_) | Value::Str(_)) => t.eq(x, y),
            _ => a == b,
        },
    }
}

/// Field-level differences between two records (empty = equal under `tol`; `None` = exact).
pub fn record_diff(a: &Record, b: &Record, tol: Option<Tolerance>) -> Vec<FieldDiff> {
    let mut out = Vec::new();
    for (k, va) in &a.fields {
        match b.get(k) {
            Some(vb) if cmp_values(k, va, vb, tol) => {}
            vb => out.push(FieldDiff {
                key: k.to_string(),
                expected: Some(va.clone()),
                actual: vb.cloned(),
            }),
        }
    }
    for (k, vb) in &b.fields {
        if a.get(k).is_none() {
            out.push(FieldDiff {
                key: k.to_string(),
                expected: None,
                actual: Some(vb.clone()),
            });
        }
    }
    if out.is_empty() {
        // same key set: order must match too
        let ka: Vec<&str> = a.fields.iter().map(|(k, _)| k.as_ref()).collect();
        let kb: Vec<&str> = b.fields.iter().map(|(k, _)| k.as_ref()).collect();
        if ka != kb {
            out.push(FieldDiff {
                key: "<key order>".into(),
                expected: Some(Value::Str(ka.join(","))),
                actual: Some(Value::Str(kb.join(","))),
            });
        }
    }
    out
}

struct View<'a> {
    trace: &'a Trace,
    /// indices into trace.records
    idx: Vec<usize>,
}

impl<'a> View<'a> {
    fn all(trace: &'a Trace) -> Self {
        View {
            trace,
            idx: (0..trace.records.len()).collect(),
        }
    }
    fn filter(trace: &'a Trace, f: impl Fn(&Record) -> bool) -> Self {
        View {
            trace,
            idx: (0..trace.records.len())
                .filter(|&i| f(&trace.records[i]))
                .collect(),
        }
    }
    fn len(&self) -> usize {
        self.idx.len()
    }
    fn rec(&self, i: usize) -> &'a Record {
        &self.trace.records[self.idx[i]]
    }
    fn line(&self, i: usize) -> &'a str {
        &self.trace.lines[self.idx[i]]
    }
}

fn eq_at(e: &View, a: &View, i: usize, j: usize, tol: Option<Tolerance>) -> bool {
    match tol {
        None => e.line(i) == a.line(j),
        Some(_) => record_diff(e.rec(i), a.rec(j), tol).is_empty(),
    }
}

/// Compares two views in lockstep; returns the index pair of the first difference.
fn first_diff(e: &View, a: &View, tol: Option<Tolerance>) -> Option<(usize, usize)> {
    let n = e.len().min(a.len());
    for i in 0..n {
        if !eq_at(e, a, i, i, tol) {
            return Some((i, i));
        }
    }
    (e.len() != a.len()).then_some((n, n))
}

fn classify(
    e: &View,
    a: &View,
    i: usize,
    j: usize,
    look: usize,
    tol: Option<Tolerance>,
) -> DivergenceKind {
    if i >= e.len() {
        return DivergenceKind::ExpectedEnded;
    }
    if j >= a.len() {
        return DivergenceKind::ActualEnded;
    }
    const CONFIRM: usize = 3;
    let agrees = |ei: usize, aj: usize| {
        (0..CONFIRM).all(|k| {
            let (x, y) = (ei + k, aj + k);
            match (x < e.len(), y < a.len()) {
                (true, true) => eq_at(e, a, x, y, tol),
                (false, false) => true,
                _ => false,
            }
        })
    };
    for k in 1..=look {
        if j + k <= a.len() && agrees(i, j + k) {
            return DivergenceKind::Extra(k);
        }
        if i + k <= e.len() && agrees(i + k, j) {
            return DivergenceKind::Missing(k);
        }
    }
    DivergenceKind::Changed
}

fn build(
    e: &View,
    a: &View,
    i: usize,
    j: usize,
    opts: &DiffOptions,
    tol: Option<Tolerance>,
) -> Divergence {
    let kind = classify(e, a, i, j, opts.lookahead, tol);
    let er = (i < e.len()).then(|| e.rec(i));
    let ar = (j < a.len()).then(|| a.rec(j));
    let fields = match (er, ar) {
        (Some(x), Some(y)) => record_diff(x, y, tol),
        _ => Vec::new(),
    };
    let lo = i.saturating_sub(opts.context);
    let context = (lo..i.min(e.len()))
        .map(|k| (e.idx[k] + 1, e.line(k).to_string()))
        .collect();
    let focus = er.or(ar);
    let process = focus.and_then(|r| r.get_i64("p"));
    let (process_info, process_stack) = match (process, er) {
        (Some(p), Some(_)) => process_stack(&e.trace.records, e.idx[i], p),
        (Some(p), None) => process_stack(&a.trace.records, a.idx[j], p),
        _ => (None, Vec::new()),
    };
    Divergence {
        kind,
        expected_line: if i < e.len() {
            e.idx[i] + 1
        } else {
            e.trace.records.len() + 1
        },
        actual_line: if j < a.len() {
            a.idx[j] + 1
        } else {
            a.trace.records.len() + 1
        },
        expected: er.map(|_| e.line(i).to_string()),
        actual: ar.map(|_| a.line(j).to_string()),
        fields,
        context,
        time: focus.and_then(|r| r.get_f64("t")),
        process,
        process_info,
        process_stack,
        element: focus.and_then(describe_element),
    }
}

/// Compares two parsed traces.
pub fn diff_traces(expected: &Trace, actual: &Trace, opts: &DiffOptions) -> DiffReport {
    let (tol, e, a) = match opts.mode {
        DiffMode::Exact => (None, View::all(expected), View::all(actual)),
        DiffMode::Tolerance(t) => (Some(t), View::all(expected), View::all(actual)),
        DiffMode::MeasurementsOnly(t) => {
            let f = |r: &Record| EventKind::of_record(r) == EventKind::Meas;
            (Some(t), View::filter(expected, f), View::filter(actual, f))
        }
        DiffMode::PerProcess(t) => return diff_per_process(expected, actual, opts, t),
    };
    let d = first_diff(&e, &a, tol);
    DiffReport {
        mode: opts.mode,
        expected_len: e.len(),
        actual_len: a.len(),
        equal_prefix: d.map(|(i, _)| i).unwrap_or(e.len()),
        divergence: d.map(|(i, j)| build(&e, &a, i, j, opts, tol)),
    }
}

fn diff_per_process(
    expected: &Trace,
    actual: &Trace,
    opts: &DiffOptions,
    t: Tolerance,
) -> DiffReport {
    use std::collections::BTreeSet;
    let pids = |tr: &Trace| -> BTreeSet<i64> {
        tr.records
            .iter()
            .map(|r| r.get_i64("p").unwrap_or(-1))
            .collect()
    };
    let mut all = pids(expected);
    all.extend(pids(actual));
    let mut best: Option<Divergence> = None;
    let mut equal = 0;
    for p in all {
        let f = move |r: &Record| r.get_i64("p").unwrap_or(-1) == p;
        let (e, a) = (View::filter(expected, f), View::filter(actual, f));
        match first_diff(&e, &a, Some(t)) {
            None => equal += e.len(),
            Some((i, j)) => {
                equal += i;
                let d = build(&e, &a, i, j, opts, Some(t));
                if best
                    .as_ref()
                    .is_none_or(|b| d.expected_line < b.expected_line)
                {
                    best = Some(d);
                }
            }
        }
    }
    DiffReport {
        mode: opts.mode,
        expected_len: expected.len(),
        actual_len: actual.len(),
        equal_prefix: equal,
        divergence: best,
    }
}

/// Parses and compares two trace texts.
pub fn diff_trace_text(
    expected: &str,
    actual: &str,
    opts: &DiffOptions,
) -> Result<DiffReport, ParseError> {
    if matches!(opts.mode, DiffMode::Exact) && expected == actual {
        let n = expected.lines().filter(|l| !l.is_empty()).count();
        return Ok(DiffReport {
            mode: opts.mode,
            expected_len: n,
            actual_len: n,
            equal_prefix: n,
            divergence: None,
        });
    }
    Ok(diff_traces(
        &Trace::parse(expected)?,
        &Trace::parse(actual)?,
        opts,
    ))
}

impl fmt::Display for DiffReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Some(d) = &self.divergence else {
            return writeln!(
                f,
                "traces equal ({:?}, {} lines)",
                self.mode, self.expected_len
            );
        };
        writeln!(
            f,
            "first divergence ({:?}) at expected line {} / actual line {} after {} equal lines ({} vs {} lines compared): {:?}",
            self.mode,
            d.expected_line,
            d.actual_line,
            self.equal_prefix,
            self.expected_len,
            self.actual_len,
            d.kind
        )?;
        if let Some(t) = d.time {
            write!(f, "  t = {}", crate::javafmt::to_string(t))?;
        }
        if let Some(p) = d.process {
            write!(f, "  process p={p}")?;
            if let Some(i) = &d.process_info {
                write!(f, " {i}")?;
            }
        }
        writeln!(f)?;
        for (k, s) in d.process_stack.iter().enumerate() {
            writeln!(f, "  {:width$}in {s}", "", width = 2 * k)?;
        }
        if let Some(el) = &d.element {
            writeln!(f, "  element: {el}")?;
        }
        writeln!(f, "  context:")?;
        for (n, l) in &d.context {
            writeln!(f, "    {n:>7}  {l}")?;
        }
        writeln!(
            f,
            "  - expected: {}",
            d.expected.as_deref().unwrap_or("<end of trace>")
        )?;
        writeln!(
            f,
            "  + actual:   {}",
            d.actual.as_deref().unwrap_or("<end of trace>")
        )?;
        for fd in &d.fields {
            writeln!(
                f,
                "    field {}: expected {} actual {}",
                fd.key,
                fd.expected
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "<absent>".into()),
                fd.actual
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "<absent>".into())
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const E: &str = r#"{"ev":"header","t":0.0,"format":"palladio-trace/1","run":"x","seed":1,"max_sim_time":-1,"max_measurements":5}
{"ev":"spawn","t":0.0,"p":1,"kind":"ClosedWorkloadUser","name":"ClosedUser","parent":0}
{"ev":"begin","t":0.0,"p":1,"type":"UsageScenario","id":"_us"}
{"ev":"begin","t":0.0,"p":1,"type":"InternalAction","id":"_ia","ac":"_ac"}
{"ev":"demand","t":0.0,"p":1,"res":"_cpu","rc":"_rc","spec":"_prs","sched":"FCFS","d":0.5,"st":0.5}
{"ev":"meas","t":0.0,"mp":"A[_prs|replicaID=0]","metric":"Resource Demand Tuple","v":[0.0,0.5]}
{"ev":"demand_done","t":0.5,"p":1,"res":"_cpu","rc":"_rc"}
{"ev":"end","t":0.5,"p":1,"type":"InternalAction","id":"_ia","ac":"_ac"}
{"ev":"end","t":0.5,"p":1,"type":"UsageScenario","id":"_us"}
{"ev":"finish","t":0.5,"uniforms":0,"measurements":1}
"#;

    fn tr(s: &str) -> Trace {
        Trace::parse(s).unwrap()
    }

    #[test]
    fn equal_and_changed() {
        let r = diff_trace_text(E, E, &DiffOptions::exact()).unwrap();
        assert!(r.is_equal());
        let a = E.replace(
            "\"t\":0.5,\"p\":1,\"res\"",
            "\"t\":0.5000000001,\"p\":1,\"res\"",
        );
        let r = diff_traces(&tr(E), &tr(&a), &DiffOptions::exact());
        let d = r.divergence.as_ref().unwrap();
        assert_eq!(d.expected_line, 7);
        assert_eq!(d.kind, DivergenceKind::Changed);
        assert_eq!(d.fields.len(), 1);
        assert_eq!(d.fields[0].key, "t");
        assert_eq!(d.process, Some(1));
        assert_eq!(d.process_stack.len(), 2);
        assert!(r.to_string().contains("InternalAction _ia @_ac"));
        // within tolerance
        assert!(diff_traces(&tr(E), &tr(&a), &DiffOptions::tolerance(1e-9, 0.0)).is_equal());
    }

    #[test]
    fn extra_missing_ended() {
        let lines: Vec<&str> = E.lines().collect();
        let mut extra = lines.clone();
        extra.insert(4, r#"{"ev":"hold","t":0.0,"p":1,"d":1.0}"#);
        let r = diff_traces(&tr(E), &tr(&extra.join("\n")), &DiffOptions::exact());
        assert_eq!(r.divergence.unwrap().kind, DivergenceKind::Extra(1));
        let mut missing = lines.clone();
        missing.remove(4);
        let r = diff_traces(&tr(E), &tr(&missing.join("\n")), &DiffOptions::exact());
        assert_eq!(r.divergence.unwrap().kind, DivergenceKind::Missing(1));
        let r = diff_traces(&tr(E), &tr(&lines[..5].join("\n")), &DiffOptions::exact());
        assert_eq!(r.divergence.unwrap().kind, DivergenceKind::ActualEnded);
    }

    #[test]
    fn measurements_only_and_per_process() {
        let a = E.replace(
            r#"{"ev":"begin","t":0.0,"p":1,"type":"InternalAction","id":"_ia","ac":"_ac"}"#,
            r#"{"ev":"begin","t":0.0,"p":1,"type":"InternalAction","id":"_other","ac":"_ac"}"#,
        );
        assert!(diff_traces(&tr(E), &tr(&a), &DiffOptions::measurements_only(0.0, 0.0)).is_equal());
        let b = E.replace("\"v\":[0.0,0.5]", "\"v\":[0.0,0.6]");
        let r = diff_traces(&tr(E), &tr(&b), &DiffOptions::measurements_only(0.0, 0.0));
        assert_eq!(r.divergence.unwrap().expected_line, 6);
        // swapping lines of different "processes" (meas has none) is fine per process
        let lines: Vec<&str> = E.lines().collect();
        let mut sw = lines.clone();
        sw.swap(4, 5);
        let s = sw.join("\n");
        assert!(!diff_traces(&tr(E), &tr(&s), &DiffOptions::exact()).is_equal());
        assert!(diff_traces(&tr(E), &tr(&s), &DiffOptions::per_process(0.0, 0.0)).is_equal());
    }
}

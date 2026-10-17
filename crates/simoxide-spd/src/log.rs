//! The decision log: one JSON line per policy set-up, evaluation and adjustment, in the format
//! of the Slingshot reference runner's `spd.jsonl` (`reference/slingshot/src/instrument/…/
//! SpdObserver.java`, `docs/reference-simulator/slingshot.md`), and its projection for
//! comparisons ([`project`]).
//!
//! Records (`k`):
//!
//! * `policy`: written when a policy's handler is invoked the first time: `id`, `name`,
//!   `active`, `target_group`, `adjustment` and `trigger` (metamodel class names) and `chain`,
//!   the filter class names of the chain.
//! * `eval`: one evaluation of a policy's chain: time `t`, event `ev`
//!   (`SimulationTimeReached`, `MeasurementMade`), `policy`, `filters` (the outer chain's
//!   filters that ran: `[name, "ok"]`, `[name, "disregard", reason]`, `[name, "null"]`,
//!   `[name, "exception", message]`) and `request`. Evaluations that the target-group check
//!   rejects are left out unless [`VecLog::all`] (as the reference runner's default).
//! * `adjust`: an adjustment request reached its executor: `t`, `policy`, `policy_name`, `ok`;
//!   when the executor computes them, Slingshot's change count `changes` and the model
//!   summaries `before` and `after` (`crate::adjusted::summary`, as the reference runner's);
//!   the target-group size before and after when the host knows them; `enforced` for an
//!   enforced policy.
//! * `hpa`: one sync of an HPA autoscaler (`crate::hpa`, `docs/spec/hpa.md`): `t`, `hpa` (its
//!   name), `metric` (`cpu`), `current` (replicas: `ready` plus `starting`), `unready` and
//!   `missing` pods of the calculation, the ready pods' `utilization` (percent of their
//!   requests) against the `target`, the `ratio`, the calculator's `recommended` replicas, the
//!   `stabilized` recommendation, the bounded `desired` count with its `reason` (Kubernetes'
//!   condition reasons: `DesiredWithinRange`, `ScaleUpLimit`, `TooManyReplicas`,
//!   `TooFewReplicas`, `ScaleDownLimit`, …), or the calculator's `error`; what the rescale did:
//!   pods `started` and starting pods `cancelled`, and the `goal` of the group's absolute
//!   adjustment, if one runs now. Adjustments of the HPA are `adjust` records of its policy
//!   (`_simoxide_hpa_<name>`).
//! * `hpa_ready`: starting pods of an autoscaler became ready: `t`, `hpa`, `ready` (how many)
//!   and the adjustment's `goal`.
//!
//! The reference runner's records carry the bus event index `e`, which does not exist here.
//! [`project`] maps both logs onto what P1 compared (no model summaries);
//! [`project_with_models`] keeps the `adjust` records' change count and summaries.

use crate::jfmt::json_double;
use serde_json::Value;

/// One filter result of an evaluation.
#[derive(Clone, Debug, PartialEq)]
pub struct FilterRecord {
    pub name: &'static str,
    /// `ok`, `disregard`, `null` or `exception`.
    pub result: &'static str,
    pub reason: Option<String>,
}

/// A decision-log record.
#[derive(Clone, Debug, PartialEq)]
pub enum Record {
    Policy {
        id: Box<str>,
        name: Box<str>,
        active: bool,
        target_group: Option<Box<str>>,
        adjustment: Option<&'static str>,
        trigger: Option<&'static str>,
        chain: Vec<&'static str>,
    },
    Eval {
        t: f64,
        ev: &'static str,
        policy: Box<str>,
        filters: Vec<FilterRecord>,
        request: bool,
    },
    Adjust {
        t: f64,
        policy: Box<str>,
        policy_name: Box<str>,
        ok: bool,
        /// Entries of Slingshot's `ModelAdjusted` change list, if the executor computed it.
        changes: Option<usize>,
        /// Model summaries (JSON objects) before and after, if the executor recorded them.
        before: Option<Box<str>>,
        after: Option<Box<str>>,
        size_before: Option<usize>,
        size_after: Option<usize>,
        /// An enforced policy's request ([`crate::Spd::enforce`]).
        enforced: bool,
    },
    /// One sync of an HPA autoscaler.
    Hpa(Box<HpaRecord>),
    /// Starting pods of an autoscaler became ready.
    HpaReady {
        t: f64,
        hpa: Box<str>,
        ready: i32,
        goal: i32,
    },
}

/// The fields of an `hpa` record (see the module documentation).
#[derive(Clone, Debug, PartialEq)]
pub struct HpaRecord {
    pub t: f64,
    pub hpa: Box<str>,
    pub current: i32,
    pub ready: i32,
    pub starting: i32,
    pub unready: i32,
    pub missing: i32,
    pub utilization: Option<i32>,
    pub target: i32,
    pub ratio: Option<f64>,
    pub recommended: Option<i32>,
    pub stabilized: Option<i32>,
    pub desired: i32,
    pub reason: &'static str,
    pub error: Option<String>,
    pub started: i32,
    pub cancelled: i32,
    pub goal: Option<i32>,
}

fn q(s: &str) -> String {
    serde_json::to_string(s).expect("string")
}

impl Record {
    /// The record as one JSON line (without the newline), keys in the reference's order.
    pub fn to_json(&self) -> String {
        match self {
            Record::Policy {
                id,
                name,
                active,
                target_group,
                adjustment,
                trigger,
                chain,
            } => {
                let opt = |o: Option<&str>| o.map_or_else(|| "null".to_string(), q);
                let chain: Vec<String> = chain.iter().map(|c| q(c)).collect();
                format!(
                    "{{\"k\":\"policy\",\"id\":{},\"name\":{},\"active\":{},\"target_group\":{},\"adjustment\":{},\"trigger\":{},\"chain\":[{}]}}",
                    q(id),
                    q(name),
                    active,
                    opt(target_group.as_deref()),
                    opt(*adjustment),
                    opt(*trigger),
                    chain.join(",")
                )
            }
            Record::Eval {
                t,
                ev,
                policy,
                filters,
                request,
            } => {
                let fs: Vec<String> = filters
                    .iter()
                    .map(|f| match &f.reason {
                        Some(r) => format!("[{},{},{}]", q(f.name), q(f.result), q(r)),
                        None => format!("[{},{}]", q(f.name), q(f.result)),
                    })
                    .collect();
                format!(
                    "{{\"k\":\"eval\",\"t\":{},\"ev\":{},\"policy\":{},\"filters\":[{}],\"request\":{}}}",
                    json_double(*t),
                    q(ev),
                    q(policy),
                    fs.join(","),
                    request
                )
            }
            Record::Adjust {
                t,
                policy,
                policy_name,
                ok,
                changes,
                before,
                after,
                size_before,
                size_after,
                enforced,
            } => {
                let mut s = format!(
                    "{{\"k\":\"adjust\",\"t\":{},\"policy\":{},\"policy_name\":{},\"ok\":{}",
                    json_double(*t),
                    q(policy),
                    q(policy_name),
                    ok
                );
                if let Some(c) = changes {
                    s.push_str(&format!(",\"changes\":{c}"));
                }
                if let Some(b) = before {
                    s.push_str(",\"before\":");
                    s.push_str(b);
                }
                if let Some(a) = after {
                    s.push_str(",\"after\":");
                    s.push_str(a);
                }
                if *enforced {
                    s.push_str(",\"enforced\":true");
                }
                if let Some(b) = size_before {
                    s.push_str(&format!(",\"size_before\":{b}"));
                }
                if let Some(a) = size_after {
                    s.push_str(&format!(",\"size_after\":{a}"));
                }
                s.push('}');
                s
            }
            Record::Hpa(r) => {
                let opt_i = |o: Option<i32>| o.map_or_else(|| "null".into(), |v| v.to_string());
                let mut s = format!(
                    "{{\"k\":\"hpa\",\"t\":{},\"hpa\":{},\"metric\":\"cpu\",\"current\":{},\"ready\":{},\"starting\":{},\"unready\":{},\"missing\":{},\"utilization\":{},\"target\":{},\"ratio\":{},\"recommended\":{},\"stabilized\":{},\"desired\":{},\"reason\":{}",
                    json_double(r.t),
                    q(&r.hpa),
                    r.current,
                    r.ready,
                    r.starting,
                    r.unready,
                    r.missing,
                    opt_i(r.utilization),
                    r.target,
                    r.ratio.map_or_else(|| "null".into(), json_double),
                    opt_i(r.recommended),
                    opt_i(r.stabilized),
                    r.desired,
                    q(r.reason),
                );
                if let Some(e) = &r.error {
                    s.push_str(&format!(",\"error\":{}", q(e)));
                }
                s.push_str(&format!(
                    ",\"started\":{},\"cancelled\":{},\"goal\":{}}}",
                    r.started,
                    r.cancelled,
                    opt_i(r.goal)
                ));
                s
            }
            Record::HpaReady {
                t,
                hpa,
                ready,
                goal,
            } => format!(
                "{{\"k\":\"hpa_ready\",\"t\":{},\"hpa\":{},\"ready\":{ready},\"goal\":{goal}}}",
                json_double(*t),
                q(hpa)
            ),
        }
    }
}

/// Where the interpreter writes its decision log.
pub trait Log {
    /// Records are built only when this is true.
    fn enabled(&self) -> bool;
    /// Also log evaluations the target-group check rejected.
    fn all(&self) -> bool {
        false
    }
    fn record(&mut self, r: Record);
}

/// No log.
pub struct NoLog;

impl Log for NoLog {
    fn enabled(&self) -> bool {
        false
    }
    fn record(&mut self, _: Record) {}
}

/// A log kept in memory.
#[derive(Clone, Debug, Default)]
pub struct VecLog {
    pub records: Vec<Record>,
    /// Keep evaluations the target-group check rejected.
    pub all: bool,
}

impl Log for VecLog {
    fn enabled(&self) -> bool {
        true
    }
    fn all(&self) -> bool {
        self.all
    }
    fn record(&mut self, r: Record) {
        self.records.push(r);
    }
}

impl VecLog {
    /// The log as JSON lines.
    pub fn to_jsonl(&self) -> String {
        let mut s = String::new();
        for r in &self.records {
            s.push_str(&r.to_json());
            s.push('\n');
        }
        s
    }
}

/// The projection two decision logs are compared on: `policy` records as they are, `eval`
/// records without the event index `e`, `adjust` records reduced to `t`, `policy`,
/// `policy_name` and `ok`; `log` records dropped. Lines that are not JSON objects are errors.
pub fn project(jsonl: &str) -> Result<Vec<Value>, String> {
    project_keys(jsonl, &["k", "t", "policy", "policy_name", "ok"])
}

fn project_keys(jsonl: &str, adjust_keys: &[&str]) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for (i, line) in jsonl.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line).map_err(|e| format!("line {}: {e}", i + 1))?;
        let Value::Object(mut o) = v else {
            return Err(format!("line {}: not an object", i + 1));
        };
        match o.get("k").and_then(Value::as_str) {
            Some("policy") => out.push(Value::Object(o)),
            Some("eval") => {
                o.remove("e");
                out.push(Value::Object(o));
            }
            Some("adjust") => {
                let mut p = serde_json::Map::new();
                for &k in adjust_keys {
                    if let Some(x) = o.remove(k) {
                        p.insert(k.to_string(), x);
                    }
                }
                out.push(Value::Object(p));
            }
            _ => {}
        }
    }
    Ok(out)
}

/// [`project`], but `adjust` records keep `changes`, `before` and `after` (the comparison of the
/// adjustments themselves, V3).
pub fn project_with_models(jsonl: &str) -> Result<Vec<Value>, String> {
    project_keys(
        jsonl,
        &[
            "k",
            "t",
            "policy",
            "policy_name",
            "ok",
            "changes",
            "before",
            "after",
        ],
    )
}

/// Compares two projected logs: `None` if equal, else a description of the first difference.
pub fn first_difference(want: &[Value], got: &[Value]) -> Option<String> {
    for (i, (a, b)) in want.iter().zip(got).enumerate() {
        if a != b {
            return Some(format!("record {}:\n  want {a}\n  got  {b}", i + 1));
        }
    }
    if want.len() != got.len() {
        let (more, which) = if want.len() > got.len() {
            (&want[got.len()], "missing")
        } else {
            (&got[want.len()], "extra")
        };
        return Some(format!(
            "{} records wanted, {} got; first {which}: {more}",
            want.len(),
            got.len()
        ));
    }
    None
}

/// A log written as JSON lines to `W` (write errors are kept and reported by
/// [`JsonlLog::finish`]).
pub struct JsonlLog<W: std::io::Write> {
    out: W,
    /// Keep evaluations the target-group check rejected.
    pub all: bool,
    error: Option<std::io::Error>,
}

impl<W: std::io::Write> JsonlLog<W> {
    pub fn new(out: W) -> Self {
        JsonlLog {
            out,
            all: false,
            error: None,
        }
    }

    /// Flushes; the first write error, if any.
    pub fn finish(mut self) -> std::io::Result<W> {
        if let Some(e) = self.error.take() {
            return Err(e);
        }
        self.out.flush()?;
        Ok(self.out)
    }
}

impl<W: std::io::Write> Log for JsonlLog<W> {
    fn enabled(&self) -> bool {
        self.error.is_none()
    }
    fn all(&self) -> bool {
        self.all
    }
    fn record(&mut self, r: Record) {
        let mut line = r.to_json();
        line.push('\n');
        if let Err(e) = self.out.write_all(line.as_bytes()) {
            self.error = Some(e);
        }
    }
}

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
//! * `adjust`: an adjustment request reached its executor: `t`, `policy`, `policy_name`, `ok`,
//!   and the target-group size before and after when the host knows them.
//!
//! The reference runner's records carry the bus event index `e` and its `adjust` records model
//! summaries; neither exists here. [`project`] maps both logs onto what they share.

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
        size_before: Option<usize>,
        size_after: Option<usize>,
    },
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
                size_before,
                size_after,
            } => {
                let mut s = format!(
                    "{{\"k\":\"adjust\",\"t\":{},\"policy\":{},\"policy_name\":{},\"ok\":{}",
                    json_double(*t),
                    q(policy),
                    q(policy_name),
                    ok
                );
                if let Some(b) = size_before {
                    s.push_str(&format!(",\"size_before\":{b}"));
                }
                if let Some(a) = size_after {
                    s.push_str(&format!(",\"size_after\":{a}"));
                }
                s.push('}');
                s
            }
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
                for k in ["k", "t", "policy", "policy_name", "ok"] {
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

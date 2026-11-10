//! `--report FILE` (`docs/guide/result-report.md`): the result report of `run` and `resume`. One small
//! JSON document per invocation: what ran (build, behaviour flags, limits), the interval it
//! simulated, why it stopped, the decisions, the files it wrote (by reference) and its
//! counters. It is written last, atomically (a temporary file renamed), after every file it
//! names is complete; a failed invocation writes one too, with the failure as its stop.

use super::{Args, RunInfo, Why, interval_from, interval_start, later};
use serde_json::{Value, json};
use simoxide_sim::snapshot::SimTime;
use simoxide_sim::{Condition, Measurements, Pause, PsAlgorithm, SimConfig, SimError};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// The `format` of a report.
pub const FORMAT: &str = "simoxide-run-report";
/// The `version` of a report.
pub const VERSION: u32 = 1;

/// What a failed invocation still knows: kept as the invocation goes on.
#[derive(Default)]
struct Known {
    /// The invocation's arguments, resolved (with the snapshot's flags for `resume`).
    args: Option<Args>,
    /// The run's configuration (a resumed run's: the snapshot's), once known.
    cfg: Option<SimConfig>,
    /// The error that aborted the simulation.
    error: Option<SimError>,
    /// The snapshot written.
    snapshot: Option<(PathBuf, u64, String)>,
}

thread_local! {
    static KNOWN: RefCell<Known> = RefCell::new(Known::default());
}

/// The invocation's arguments, resolved.
pub fn invocation(a: &Args) {
    KNOWN.with(|k| k.borrow_mut().args = Some(a.clone()));
}

/// The run's configuration (at its creation or restore).
pub fn config(c: &SimConfig) {
    KNOWN.with(|k| k.borrow_mut().cfg = Some(c.clone()));
}

/// The snapshot written.
pub fn snapshot_written(s: &(PathBuf, u64, String)) {
    KNOWN.with(|k| k.borrow_mut().snapshot = Some(s.clone()));
}

/// `e` as the CLI's error message; a [`SimError`] is kept for the report of the failure.
pub fn sim_err<E: std::fmt::Display + 'static>(e: E) -> String {
    if let Some(s) = (&e as &dyn std::any::Any).downcast_ref::<SimError>() {
        KNOWN.with(|k| k.borrow_mut().error = Some(s.clone()));
    }
    e.to_string()
}

/// A time as the report writes it.
fn time(t: SimTime) -> Value {
    json!({"time": t.time, "time_ns": t.time_ns})
}

/// A time in nanoseconds as the report writes it (`requested` times).
fn time_ns(ns: i64) -> Value {
    time(SimTime {
        time: ns as f64 / 1e9,
        time_ns: ns,
    })
}

/// Why the run stopped.
struct Stop {
    kind: &'static str,
    cause: &'static str,
    detail: String,
    /// The `--pause-when` condition that held: its index and text.
    condition: Option<usize>,
    /// The scaling decision pending.
    decision: Option<u64>,
}

impl Stop {
    fn new(kind: &'static str, cause: &'static str, detail: String) -> Stop {
        Stop {
            kind,
            cause,
            detail,
            condition: None,
            decision: None,
        }
    }

    fn json(&self, a: &Args) -> Value {
        let mut v = json!({"kind": self.kind, "cause": self.cause, "detail": self.detail});
        if let Some(i) = self.condition {
            v["condition"] = json!({"index": i, "text": a.pause_when_text.get(i)});
        }
        if let Some(n) = self.decision {
            v["decision"] = json!(n);
        }
        v
    }
}

/// The cause of a pause `p` and its words.
fn pause_cause(p: Pause) -> (&'static str, String) {
    let s = |t: i64| t as f64 / 1e9;
    match p {
        Pause::After(t) => (
            "pause_at",
            format!("every event at or before {} s ran", s(t)),
        ),
        Pause::Before(t) => ("pause_before", format!("every event before {} s ran", s(t))),
        Pause::Events(n) => ("pause_events", format!("{n} events ran")),
        Pause::Never => ("pause", "paused".into()),
    }
}

/// Why a paused run paused.
fn paused_stop(a: &Args, why: &Why) -> Stop {
    if let Why::Decision(n, text) = why {
        let mut s = Stop::new("decision", "decision", text.clone());
        s.decision = Some(*n);
        return s;
    }
    let fired = match why {
        Why::Condition(f) => Some(f.reason.clone()),
        _ => None,
    };
    match why.condition(a) {
        Some(Err(p)) => {
            let (cause, detail) = pause_cause(p);
            Stop::new("pause", cause, detail)
        }
        Some(Ok(i)) => {
            let detail = fired.unwrap_or_default();
            let (kind, cause) = match &a.pause_when[i] {
                Condition::At(p) => ("pause", pause_cause(*p).0),
                Condition::Elapsed(_) => ("pause", "elapsed"),
                Condition::ElapsedEvents(_) => ("pause", "elapsed_events"),
                Condition::Series { .. } => ("condition", "series"),
                Condition::Slo { .. } => ("condition", "slo"),
            };
            let mut s = Stop::new(kind, cause, detail);
            s.condition = Some(i);
            s
        }
        None => Stop::new("pause", "pause", fired.unwrap_or_default()),
    }
}

/// Why a run that the simulator finished ended.
fn completed_stop(end_ns: i64, main_count: i64, cfg: &SimConfig) -> Stop {
    let c = simoxide_sim::stop::completion(end_ns, main_count, cfg);
    let detail = match c.cause {
        "max_measurements" => format!("max-measurements: {main_count} usage-scenario runs"),
        "max_sim_time" => format!("max-sim-time: {} s", cfg.max_sim_time),
        _ => "no event left".into(),
    };
    Stop::new(c.kind, c.cause, detail)
}

/// The build: version and cargo features.
fn simulator() -> Value {
    let features: Vec<&str> = [
        ("exact", cfg!(feature = "exact")),
        ("fast", cfg!(feature = "fast")),
        ("spd", cfg!(feature = "spd")),
        ("reconfiguration", cfg!(feature = "reconfiguration")),
        ("hotspot-math", cfg!(feature = "hotspot-math")),
    ]
    .into_iter()
    .filter(|f| f.1)
    .map(|f| f.0)
    .collect();
    json!({"name": "simoxide", "version": env!("CARGO_PKG_VERSION"), "features": features})
}

/// The effective behaviour flags.
fn behaviour(a: &Args, cfg: Option<&SimConfig>, spd: Option<&str>) -> Value {
    let mut v = json!({
        "mode": a.mode.name(),
        "flavour": a.flavour.name(),
        "quirks": a.quirks.to_string(),
        "measurements_scope": if a.interval { "interval" } else { "cumulative" },
        "spd_quirks": spd,
        "restored": a.restored,
    });
    if let Some(c) = cfg {
        v["seed"] = json!(c.seed);
        v["ps_algorithm"] = json!(match c.ps_algorithm {
            PsAlgorithm::Exact => "exact",
            PsAlgorithm::VirtualTime => "virtual-time",
        });
        v["link_throughput"] = json!(c.simulate_throughput_of_linking_resources);
        v["usage_streams"] = json!(c.usage_streams);
        v["fix_double_resume"] = json!(c.fix_double_resume);
        v["load_profile"] = json!(c.load_profile.is_some());
    }
    v
}

/// The limits: the run's (from its configuration) and this invocation's.
fn limits(a: &Args, cfg: Option<&SimConfig>) -> Value {
    let pause = a.pause.map(|p| match p {
        Pause::After(t) | Pause::Before(t) => {
            let mut v = time_ns(t);
            v["type"] = json!(pause_cause(p).0);
            v
        }
        Pause::Events(n) => json!({"type": "pause_events", "events": n}),
        Pause::Never => Value::Null,
    });
    let mut v = json!({
        "pause": pause,
        "pause_when": a.pause_when_text,
        "pause_on_decision": a.pause_on_decision,
        "timeout": a.timeout,
    });
    if let Some(c) = cfg {
        v["max_sim_time"] = json!(c.max_sim_time);
        v["max_measurements"] = json!(c.max_measurements);
        v["max_events"] = json!(c.limits.max_events);
        v["max_steps"] = json!(c.limits.max_steps);
        v["max_processes"] = json!(c.limits.max_processes);
        v["max_stack_depth"] = json!(c.limits.max_stack_depth);
        v["max_events_per_instant"] = json!(c.max_events_per_instant);
    }
    v
}

/// A file written: its path as given and its length now.
fn file(p: &Path) -> Value {
    let bytes = std::fs::metadata(p).map(|m| m.len()).ok();
    json!({"path": p.to_string_lossy(), "bytes": bytes})
}

/// The files the invocation wrote, by kind. `run`: the engine that ran (which logs it wrote).
fn outputs(
    a: &Args,
    snapshot: Option<&(PathBuf, u64, String)>,
    rows: u64,
    run: Option<&RunInfo>,
) -> Value {
    let mut o = snapshot_file(snapshot);
    if let Some(p) = &a.measurements {
        let mut v = file(p);
        v["rows"] = json!(rows);
        o.insert("measurements".into(), v);
    }
    let spd = run.is_some_and(|r| r.spd.is_some());
    let rules = run.is_some_and(|r| r.rules.is_some());
    for (kind, p, written) in [
        ("summary", &a.summary, true),
        ("trace", &a.trace, true),
        ("tape", &a.tape, true),
        ("spd_log", &a.spd_log, spd),
        ("decision_file", &a.decision_file, spd),
        ("rules_log", &a.rules_log, rules),
    ] {
        if let (Some(p), true) = (p, written) {
            o.insert(kind.into(), file(p));
        }
    }
    Value::Object(o)
}

/// The outputs with the snapshot written, if any.
fn snapshot_file(snapshot: Option<&(PathBuf, u64, String)>) -> serde_json::Map<String, Value> {
    let mut o = serde_json::Map::new();
    if let Some((p, _, state)) = snapshot {
        let mut v = file(p);
        v["state"] = json!(state);
        o.insert("snapshot".into(), v);
    }
    o
}

/// How a run that did not fail ended.
pub struct Ended<'a> {
    /// `Some`: paused (why); `None`: finished.
    pub why: Option<&'a Why>,
    pub measurements: &'a Measurements,
    /// The clock at the pause or the end: the last event's time.
    pub last: SimTime,
    /// Events processed and finished usage-scenario runs, from time 0.
    pub events: u64,
    pub main_count: i64,
    pub snapshot: Option<&'a (PathBuf, u64, String)>,
    pub info: &'a RunInfo,
}

/// The report of a run that paused or finished.
pub fn ended(a: &Args, e: &Ended, t0: Instant) -> Value {
    let m = e.measurements;
    let info = e.info;
    let start = interval_start(a, m);
    let end = super::interval_end(a, e.why, m, e.last);
    let stop = match e.why {
        Some(w) => paused_stop(a, w),
        None => completed_stop(e.last.time_ns, e.main_count, &info.cfg),
    };
    let from = interval_from(a, m);
    let spd = info.spd.as_ref();
    let delta = |i: usize| spd.map(|s| s.now[i] - s.start[i]);
    let mut doc = base(a, Some(&info.cfg), spd.map(|s| s.quirks.as_str()));
    let o = doc.as_object_mut().expect("an object");
    o.insert(
        "status".into(),
        json!(if e.why.is_some() {
            "paused"
        } else {
            "finished"
        }),
    );
    o.insert(
        "interval".into(),
        json!({
            "start": time(start),
            "end": time(end),
            "last_event": time(e.last),
            "tuples": {"from": from, "to": m.count},
        }),
    );
    o.insert(
        "endpoint_reached".into(),
        json!(matches!(stop.kind, "pause" | "completed")),
    );
    o.insert("stop".into(), stop.json(a));
    o.insert(
        "resumable".into(),
        json!(e.why.is_some() && e.snapshot.is_some()),
    );
    if let Some(d) = spd.and_then(|s| s.decisions.as_ref()) {
        o.insert(
            "decisions".into(),
            json!({"resolved": d["resolved"], "pending": d["pending"]}),
        );
    }
    o.insert(
        "measurements".into(),
        json!({"scope": if a.interval { "interval" } else { "cumulative" }, "rows": m.rows_held()}),
    );
    if let Some(s) = e.snapshot {
        o.insert("state".into(), json!(s.2));
    }
    o.insert(
        "outputs".into(),
        outputs(a, e.snapshot, m.rows_held(), Some(info)),
    );
    let mut inv = json!({
        "events": e.events.saturating_sub(a.start_events),
        "tuples": from.map(|f| m.count - f),
        "wall_secs": t0.elapsed().as_secs_f64(),
    });
    let mut cum = json!({
        "events": e.events,
        "tuples": m.count,
        "usage_scenario_runs": e.main_count,
    });
    if let Some(s) = spd {
        inv["spd_evaluations"] = json!(delta(0));
        inv["spd_requests"] = json!(delta(1));
        inv["adjustments"] = json!(delta(2));
        cum["spd_evaluations"] = json!(s.now[0]);
        cum["spd_requests"] = json!(s.now[1]);
        cum["adjustments"] = json!(s.now[2]);
    }
    if let Some((start, now)) = info.rules {
        inv["reconfigurations"] = json!(now[0] - start[0]);
        inv["reconfigurations_changed"] = json!(now[1] - start[1]);
        cum["reconfigurations"] = json!(now[0]);
        cum["reconfigurations_changed"] = json!(now[1]);
    }
    if !a.changes.is_empty() {
        inv["change_steps"] = json!(a.changes.len());
    }
    o.insert(
        "counters".into(),
        json!({"invocation": inv, "cumulative": cum}),
    );
    doc
}

/// The parts every report has: format, build, command, behaviour, limits, parent.
fn base(a: &Args, cfg: Option<&SimConfig>, spd: Option<&str>) -> Value {
    let mut v = json!({
        "format": FORMAT,
        "version": VERSION,
        "simulator": simulator(),
        "command": a.cmd,
        "behaviour": behaviour(a, cfg, spd),
        "limits": limits(a, cfg),
    });
    if let Some((p, _)) = &a.parent {
        v["parent"] = json!(p);
    }
    v
}

/// The report of a failed invocation (`given`: the arguments as parsed; `error`: the message).
pub fn failed(given: &Args, error: &str) -> Value {
    let k = KNOWN.with(|k| std::mem::take(&mut *k.borrow_mut()));
    let a = k.args.as_ref().unwrap_or(given);
    let mut doc = base(a, k.cfg.as_ref(), None);
    let stop = match &k.error {
        Some(e) => {
            let s = simoxide_sim::stop::error(e);
            Stop::new(s.kind, s.cause, error.to_string())
        }
        None => Stop::new("error", "invocation", error.to_string()),
    };
    let o = doc.as_object_mut().expect("an object");
    o.insert("status".into(), json!("failed"));
    // the interval as far as it is known: it ends at the error
    let start = (k.args.is_some()).then(|| {
        a.start_end.unwrap_or(SimTime {
            time: a.start_ns as f64 / 1e9,
            time_ns: a.start_ns,
        })
    });
    let last = k.error.as_ref().map(|e| SimTime {
        time: e.at_ns as f64 / 1e9,
        time_ns: e.at_ns,
    });
    let end = match (start, last) {
        (Some(s), Some(l)) => Some(later(s, l)),
        _ => None,
    };
    let from = a.parent.as_ref().map_or(Some(0), |p| p.1);
    o.insert(
        "interval".into(),
        json!({
            "start": start.map(time),
            "end": end.map(time),
            "last_event": last.map(time),
            "tuples": {"from": from, "to": null},
        }),
    );
    o.insert("endpoint_reached".into(), json!(false));
    o.insert("stop".into(), stop.json(a));
    o.insert("resumable".into(), json!(k.snapshot.is_some()));
    if let Some(s) = &k.snapshot {
        o.insert("state".into(), json!(s.2));
    }
    // only the snapshot is known to be complete
    o.insert(
        "outputs".into(),
        Value::Object(snapshot_file(k.snapshot.as_ref())),
    );
    doc
}

/// Writes `doc` to `p` atomically: a temporary file in the same folder, renamed.
pub fn write(p: &Path, doc: &Value) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(doc).map_err(|e| e.to_string())?;
    text.push('\n');
    let name = p
        .file_name()
        .ok_or_else(|| format!("--report {}: not a file name", p.display()))?;
    let tmp = p.with_file_name(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        std::process::id()
    ));
    let err = |e: std::io::Error| format!("--report {}: {e}", p.display());
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp).map_err(err)?;
        f.write_all(text.as_bytes()).map_err(err)?;
        f.sync_all().map_err(err)?;
    }
    std::fs::rename(&tmp, p).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        err(e)
    })
}

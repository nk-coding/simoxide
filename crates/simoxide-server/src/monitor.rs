//! The monitor of a running simulation: decides between two events whether the run pauses (to
//! publish progress, to check a confidence interval, because a termination condition holds or
//! because a client cancelled it), keeps the incremental series statistics and evaluates the
//! termination tree.
//!
//! The engines ask [`Monitor::keep_going`] before every event, so it has to be cheap: it
//! compares the run's counters with the next threshold of each kind (events, tuples, finished
//! usage-scenario runs, simulated time) and looks at the clock and the cancellation flag every
//! [`CLOCK_STRIDE`] events. Only when a threshold is crossed does it evaluate the termination
//! tree; the tree is monotone (counters only grow and there is no negation), so a crossed
//! threshold never needs to be looked at again. Confidence intervals change only at
//! checkpoints ([`Monitor::checkpoint`]), where the run is paused and the new rows of every
//! series are folded into the statistics.

use crate::report::{ConfidenceState, Percentiles, Progress, SeriesStats};
use crate::request::{Combine, Condition, Confidence};
use crate::stats::{BatchMeans, SeriesTracker, Warmup};
use simoxide_sim::Measurements;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The clock and the cancellation flag are read every this many events.
pub const CLOCK_STRIDE: u32 = 1024;
/// Nanoseconds per second.
pub(crate) const NANOS_PER_SEC: i64 = 1_000_000_000;
/// Longest duration the server computes with (s); it keeps huge values from overflowing
/// `Instant` arithmetic.
const MAX_DURATION_SECS: f64 = 1e9;
/// A confidence check is due after this share of the tuples recorded so far: 1/50, 2 % ...
const CONFIDENCE_CHECK_TUPLE_DIVISOR: u64 = 50;
/// ... but not before this many new tuples, so that short series do not check all the time.
const CONFIDENCE_CHECK_MIN_TUPLES: u64 = 500;
/// Lower bound of the elapsed time rates are computed over (s).
const MIN_RATE_INTERVAL_SECS: f64 = 1e-9;
/// Percent per unit.
const PERCENT: f64 = 100.0;

/// Nanoseconds of `secs` seconds, rounded.
pub(crate) fn secs_to_ns(secs: f64) -> i64 {
    (secs * NANOS_PER_SEC as f64).round() as i64
}

/// Seconds of `ns` nanoseconds.
pub(crate) fn ns_to_secs(ns: i64) -> f64 {
    ns as f64 / NANOS_PER_SEC as f64
}

/// A duration of `secs` seconds, capped at what the server computes with.
pub(crate) fn capped_duration(secs: f64) -> Duration {
    Duration::from_secs_f64(secs.min(MAX_DURATION_SECS))
}

/// The counters of a run between two events (what [`Monitor::keep_going`] sees).
#[derive(Clone, Copy, Debug, Default)]
pub struct Probe {
    /// Simulation time of the last event (ns).
    pub now_ns: i64,
    /// Time of the next event (ns), if any.
    pub next_ns: Option<i64>,
    /// Events processed.
    pub events: u64,
    /// Finished usage-scenario runs.
    pub main_count: i64,
    /// Recorded tuples, all series.
    pub tuples: u64,
    /// Live processes, if the engine counts them.
    pub live_processes: Option<usize>,
}

/// Why [`Monitor::keep_going`] asked for a pause.
#[derive(Clone, Debug, PartialEq)]
pub enum PauseWhy {
    /// Progress is due or a confidence interval is to be checked.
    Checkpoint,
    /// A termination condition holds (which criteria, in words).
    Terminate(String),
    /// The client cancelled the run.
    Cancel,
    /// A cap of the server holds (run time, recorded tuples).
    ServerLimit(String),
}

/// Thresholds of one counter kind, sorted, with the index of the first one not yet crossed.
#[derive(Clone, Debug, Default)]
struct Thresholds<T: Copy + PartialOrd> {
    values: Vec<T>,
    next: usize,
}

impl<T: Copy + PartialOrd> Thresholds<T> {
    fn new(mut values: Vec<T>) -> Self {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Thresholds { values, next: 0 }
    }
    /// Whether `v` reached the next threshold (`>=`).
    #[inline]
    fn reached(&self, v: T) -> bool {
        self.next < self.values.len() && v >= self.values[self.next]
    }
    /// Whether `v` passed the next threshold (`>`).
    #[inline]
    fn passed(&self, v: T) -> bool {
        self.next < self.values.len() && v > self.values[self.next]
    }
    fn skip_reached(&mut self, v: T) {
        while self.reached(v) {
            self.next += 1;
        }
    }
    fn skip_passed(&mut self, v: T) {
        while self.passed(v) {
            self.next += 1;
        }
    }
}

/// One confidence criterion of the termination tree and its state.
struct ConfState {
    /// The criterion.
    spec: Confidence,
    /// Per series: watched or not (series added by model changes are classified on arrival).
    selected: Vec<bool>,
    /// Per series: batch means of the watched ones.
    batches: Vec<Option<BatchMeans>>,
    /// Met at the last checkpoint.
    met: bool,
    /// Watched series with data, and how many of them met the targets (last checkpoint).
    watched: usize,
    satisfied: usize,
}

impl ConfState {
    /// Classifies the series recorded since the last call.
    fn sync(&mut self, m: &Measurements) {
        for i in self.selected.len()..m.series.len() {
            let def = &m.series[i];
            let sel = self.spec.selects(&def.mp, def.metric);
            self.selected.push(sel);
            self.batches
                .push(sel.then(|| BatchMeans::new(self.spec.batching())));
        }
    }

    /// The interval of series `i` against the targets; `None` if the series is not watched. `n`:
    /// the values of the series after the warm-up.
    fn state_of(&self, i: usize, n: u64) -> Option<ConfidenceState> {
        let b = self.batches.get(i)?.as_ref()?;
        let interval = b.interval(self.spec.level);
        let satisfied =
            n >= self.spec.min_samples && interval.as_ref().is_some_and(|ci| self.spec.met_by(ci));
        Some(ConfidenceState {
            interval,
            target_relative_half_width: self.spec.relative_target(),
            target_absolute_half_width: self.spec.absolute_half_width,
            satisfied,
        })
    }
}

/// See the module documentation.
pub struct Monitor {
    /// The termination tree.
    cond: Condition,
    warmup: Warmup,
    /// When the run started (the monitor was created).
    pub started: Instant,
    /// The progress interval.
    interval: Duration,
    /// The client's cancellation flag.
    cancel: Arc<AtomicBool>,
    /// Per series: the running statistics.
    trackers: Vec<SeriesTracker>,
    /// The confidence criteria, in the order of [`Condition::confidences`].
    confs: Vec<ConfState>,
    // per-event thresholds
    th_events: Thresholds<u64>,
    th_tuples: Thresholds<u64>,
    th_main: Thresholds<i64>,
    th_time: Thresholds<i64>,
    /// Wall-clock deadlines of `timeout_secs` criteria and of the server's cap.
    th_wall: Thresholds<Instant>,
    /// The server's run-time cap (a hard stop, whatever the termination tree says).
    cap: Option<Instant>,
    /// The server's cap on recorded tuples.
    tuple_cap: Option<u64>,
    /// Next checkpoint by the clock.
    next_checkpoint: Instant,
    /// Next confidence checkpoint by the tuple count (`u64::MAX`: no confidence criterion).
    next_ci_tuples: u64,
    /// Calls of [`Monitor::keep_going`] since the clock was read last.
    calls: u32,
    /// Why the last pause was asked for.
    pub why: Option<PauseWhy>,
    /// When the last progress was computed, with the events and simulation time (ns) then.
    last_progress: Option<(Instant, u64, i64)>,
}

impl Monitor {
    /// A monitor of the termination tree `cond`, the warm-up, the progress `interval`, the
    /// client's cancellation flag and the server's caps on run time and recorded tuples.
    pub fn new(
        cond: Condition,
        warmup: Warmup,
        interval: Duration,
        cancel: Arc<AtomicBool>,
        cap: Option<Duration>,
        tuple_cap: Option<u64>,
    ) -> Monitor {
        let started = Instant::now();
        let (mut ev, mut tu, mut ma, mut ti, mut wall) = (vec![], vec![], vec![], vec![], vec![]);
        cond.visit(&mut |c| {
            ev.extend(c.max_events);
            tu.extend(c.max_tuples);
            ma.extend(c.max_measurements);
            ti.extend(c.max_sim_time.map(secs_to_ns));
            wall.extend(c.timeout_secs.map(|s| started + capped_duration(s)));
        });
        let cap = cap.map(|d| started + d);
        wall.extend(cap);
        tu.extend(tuple_cap);
        let confs: Vec<ConfState> = cond
            .confidences()
            .into_iter()
            .map(|c| ConfState {
                spec: c.clone(),
                selected: Vec::new(),
                batches: Vec::new(),
                met: false,
                watched: 0,
                satisfied: 0,
            })
            .collect();
        let next_ci_tuples = if confs.is_empty() { u64::MAX } else { 0 };
        Monitor {
            cond,
            warmup,
            started,
            interval,
            cancel,
            trackers: Vec::new(),
            confs,
            th_events: Thresholds::new(ev),
            th_tuples: Thresholds::new(tu),
            th_main: Thresholds::new(ma),
            th_time: Thresholds::new(ti),
            th_wall: Thresholds::new(wall),
            cap,
            tuple_cap,
            next_checkpoint: started + interval,
            next_ci_tuples,
            calls: 0,
            why: None,
            last_progress: None,
        }
    }

    /// Whether [`Monitor::keep_going`] needs [`Probe::next_ns`] (a `max_sim_time` criterion not
    /// yet passed); finding the next event costs a little on every event otherwise.
    pub fn wants_next_time(&self) -> bool {
        self.th_time.next < self.th_time.values.len()
    }

    /// Asked before every event: `false` pauses the run ([`Monitor::why`] says why).
    #[inline]
    pub fn keep_going(&mut self, p: &Probe) -> bool {
        if self.th_events.reached(p.events)
            || self.th_tuples.reached(p.tuples)
            || self.th_main.reached(p.main_count)
            || p.next_ns.is_some_and(|t| self.th_time.passed(t))
        {
            self.th_events.skip_reached(p.events);
            self.th_tuples.skip_reached(p.tuples);
            self.th_main.skip_reached(p.main_count);
            if let Some(t) = p.next_ns {
                self.th_time.skip_passed(t);
            }
            if let Some(c) = self.tuple_cap
                && p.tuples >= c
            {
                self.why = Some(PauseWhy::ServerLimit(format!(
                    "the server's cap of {c} recorded tuples"
                )));
                return false;
            }
            if let Some(why) = self.evaluate(p, Instant::now()) {
                self.why = Some(PauseWhy::Terminate(why));
                return false;
            }
        }
        if p.tuples >= self.next_ci_tuples {
            self.why = Some(PauseWhy::Checkpoint);
            return false;
        }
        self.calls += 1;
        if self.calls >= CLOCK_STRIDE {
            self.calls = 0;
            if self.cancelled() {
                self.why = Some(PauseWhy::Cancel);
                return false;
            }
            let now = Instant::now();
            if now >= self.next_checkpoint || self.th_wall.reached(now) {
                self.why = Some(PauseWhy::Checkpoint);
                return false;
            }
        }
        true
    }

    /// The server's run-time cap, in words, if it has passed.
    pub fn cap_reason(&self, now: Instant) -> Option<String> {
        let c = self.cap?;
        (now >= c).then(|| {
            format!(
                "the server's run-time cap of {:.0} s",
                c.duration_since(self.started).as_secs_f64()
            )
        })
    }

    /// Whether a client cancelled the run.
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Folds the rows recorded since the last checkpoint into the statistics and re-checks the
    /// confidence criteria. Schedules the next checkpoint.
    pub fn checkpoint(&mut self, m: &Measurements, p: &Probe) {
        let now = Instant::now();
        while self.trackers.len() < m.series.len() {
            self.trackers.push(SeriesTracker::default());
        }
        for c in &mut self.confs {
            c.sync(m);
        }
        let warmup = self.warmup;
        for (i, t) in self.trackers.iter_mut().enumerate() {
            let rows = m.rows.get(i).map(Vec::as_slice).unwrap_or(&[]);
            let confs = &mut self.confs;
            t.update(rows, warmup, &mut |v| {
                for c in confs.iter_mut() {
                    if let Some(Some(b)) = c.batches.get_mut(i) {
                        b.push(v);
                    }
                }
            });
        }
        for c in &mut self.confs {
            let (mut watched, mut satisfied) = (0, 0);
            for (i, t) in self.trackers.iter().enumerate() {
                if t.n == 0 {
                    continue;
                }
                if let Some(s) = c.state_of(i, t.n) {
                    watched += 1;
                    satisfied += usize::from(s.satisfied);
                }
            }
            c.watched = watched;
            c.satisfied = satisfied;
            c.met = watched > 0 && satisfied == watched;
        }
        self.th_wall.skip_reached(now);
        while self.next_checkpoint <= now {
            self.next_checkpoint += self.interval;
        }
        if !self.confs.is_empty() {
            // check often while the series are short, then in proportion to their length
            self.next_ci_tuples = p.tuples
                + (p.tuples / CONFIDENCE_CHECK_TUPLE_DIVISOR).max(CONFIDENCE_CHECK_MIN_TUPLES);
        }
    }

    /// Whether the termination tree holds now (with the confidence results of the last
    /// checkpoint); the criteria that hold, in words.
    pub fn evaluate(&self, p: &Probe, now: Instant) -> Option<String> {
        if self.cap.is_some_and(|cap| now >= cap) || self.cond.is_empty() {
            return None;
        }
        let mut next_confidence = 0;
        self.criteria_that_hold(&self.cond, p, now, &mut next_confidence)
    }

    /// The criteria of the subtree `c` that make it hold, in words; `None` if it does not hold.
    /// `next_confidence`: index in `confs` of the next confidence criterion in tree order.
    fn criteria_that_hold(
        &self,
        c: &Condition,
        p: &Probe,
        now: Instant,
        next_confidence: &mut usize,
    ) -> Option<String> {
        let mut held: Vec<Option<String>> = Vec::new();
        let elapsed = now.duration_since(self.started).as_secs_f64();
        if let Some(t) = c.timeout_secs {
            held.push((elapsed >= t).then(|| format!("timeout_secs: {t} s elapsed")));
        }
        if let Some(m) = c.max_measurements {
            held.push(
                (p.main_count >= m)
                    .then(|| format!("max_measurements: {} usage-scenario runs", p.main_count)),
            );
        }
        if let Some(n) = c.max_tuples {
            held.push((p.tuples >= n).then(|| format!("max_tuples: {} tuples", p.tuples)));
        }
        if let Some(n) = c.max_events {
            held.push((p.events >= n).then(|| format!("max_events: {} events", p.events)));
        }
        if let Some(t) = c.max_sim_time {
            let lim = secs_to_ns(t);
            held.push(
                p.next_ns
                    .is_some_and(|n| n > lim)
                    .then(|| format!("max_sim_time: {t} s simulated")),
            );
        }
        if c.confidence.is_some() {
            let s = &self.confs[*next_confidence];
            *next_confidence += 1;
            held.push(s.met.then(|| {
                format!(
                    "confidence: {} series within {} at {} %",
                    s.watched,
                    confidence_target_text(&s.spec),
                    s.spec.level * PERCENT
                )
            }));
        }
        // the children are evaluated in full (no short cut), so that `next_confidence` stays in
        // step with `Condition::confidences`
        if let Some(l) = &c.all {
            let r: Vec<Option<String>> = l
                .iter()
                .map(|x| self.criteria_that_hold(x, p, now, next_confidence))
                .collect();
            let ok = !r.is_empty() && r.iter().all(Option::is_some);
            held.push(ok.then(|| {
                format!(
                    "all({})",
                    r.into_iter().flatten().collect::<Vec<_>>().join("; ")
                )
            }));
        }
        if let Some(l) = &c.any {
            let r: Vec<Option<String>> = l
                .iter()
                .map(|x| self.criteria_that_hold(x, p, now, next_confidence))
                .collect();
            held.push(r.into_iter().flatten().next());
        }
        if held.is_empty() {
            return None;
        }
        match c.combine {
            Combine::Any => held.into_iter().flatten().next(),
            Combine::All => {
                if held.iter().all(Option::is_some) {
                    Some(held.into_iter().flatten().collect::<Vec<_>>().join("; "))
                } else {
                    None
                }
            }
        }
    }

    /// Each criterion and how far the run is, e.g. `max_measurements: 412/1000`.
    pub fn termination_status(&self, p: &Probe) -> Vec<String> {
        let mut out = Vec::new();
        let mut next_confidence = 0;
        self.status_of(&self.cond, p, "", &mut next_confidence, &mut out);
        if let Some(c) = self.cap {
            out.push(format!(
                "server cap: {:.1}/{:.1} s",
                self.started.elapsed().as_secs_f64(),
                c.duration_since(self.started).as_secs_f64()
            ));
        }
        out
    }

    /// Appends the status of the subtree `c` at `path` (e.g. `all[0].`) to `out`.
    fn status_of(
        &self,
        c: &Condition,
        p: &Probe,
        path: &str,
        next_confidence: &mut usize,
        out: &mut Vec<String>,
    ) {
        let pre = if path.is_empty() {
            String::new()
        } else {
            format!("{path}.")
        };
        if c.combine == Combine::All && !c.is_empty() {
            out.push(format!("{pre}combine: all"));
        }
        if let Some(t) = c.timeout_secs {
            out.push(format!(
                "{pre}timeout_secs: {:.1}/{t} s",
                self.started.elapsed().as_secs_f64()
            ));
        }
        if let Some(m) = c.max_measurements {
            out.push(format!("{pre}max_measurements: {}/{m}", p.main_count));
        }
        if let Some(n) = c.max_tuples {
            out.push(format!("{pre}max_tuples: {}/{n}", p.tuples));
        }
        if let Some(n) = c.max_events {
            out.push(format!("{pre}max_events: {}/{n}", p.events));
        }
        if let Some(t) = c.max_sim_time {
            out.push(format!(
                "{pre}max_sim_time: {:.3}/{t} s",
                ns_to_secs(p.now_ns)
            ));
        }
        if c.confidence.is_some() {
            let s = &self.confs[*next_confidence];
            *next_confidence += 1;
            out.push(format!(
                "{pre}confidence: {}/{} watched series within {} at {} %{}",
                s.satisfied,
                s.watched,
                confidence_target_text(&s.spec),
                s.spec.level * PERCENT,
                if s.met { " (met)" } else { "" }
            ));
        }
        for (name, l) in [("all", &c.all), ("any", &c.any)] {
            for (i, x) in l.iter().flatten().enumerate() {
                self.status_of(x, p, &format!("{pre}{name}[{i}]"), next_confidence, out);
            }
        }
    }

    /// The live progress (call right after [`Monitor::checkpoint`]).
    pub fn progress(&mut self, m: &Measurements, p: &Probe) -> Progress {
        let now = Instant::now();
        let (events_per_sec, sim_speed) = match self.last_progress {
            Some((t, e, s)) if now > t => {
                let dt = now.duration_since(t).as_secs_f64();
                (
                    (p.events.saturating_sub(e)) as f64 / dt,
                    ns_to_secs(p.now_ns - s) / dt,
                )
            }
            _ => {
                let dt = now
                    .duration_since(self.started)
                    .as_secs_f64()
                    .max(MIN_RATE_INTERVAL_SECS);
                (p.events as f64 / dt, ns_to_secs(p.now_ns) / dt)
            }
        };
        self.last_progress = Some((now, p.events, p.now_ns));
        Progress {
            wall_secs: now.duration_since(self.started).as_secs_f64(),
            sim_time: ns_to_secs(p.now_ns),
            events: p.events,
            measurements: p.main_count,
            tuples: p.tuples,
            events_per_sec,
            sim_speed,
            live_processes: p.live_processes,
            termination: self.termination_status(p),
            series: self.series_stats(m, false),
        }
    }

    /// Statistics of every series with rows; with `percentiles`, the percentiles of the values
    /// after the warm-up as well (sorts a copy of each series: for final results).
    pub fn series_stats(&self, m: &Measurements, percentiles: bool) -> Vec<SeriesStats> {
        let mut out = Vec::new();
        for i in m.sorted_series() {
            let total = m.rows.get(i).map_or(0, Vec::len) as u64;
            if total == 0 {
                continue;
            }
            let def = &m.series[i];
            let fresh;
            let t = match self.trackers.get(i) {
                Some(t) => t,
                None => {
                    fresh = SeriesTracker::default();
                    &fresh
                }
            };
            let pct = percentiles
                .then(|| percentiles_of(&m.rows[i], self.warmup))
                .flatten();
            let confidence = self.confs.iter().find_map(|c| c.state_of(i, t.n));
            out.push(SeriesStats {
                measuring_point: def.mp.to_string(),
                metric: def.metric.to_string(),
                count: t.n,
                total_count: total,
                mean: t.mean(),
                std_dev: t.std_dev(),
                min: (t.n > 0).then_some(t.min),
                max: (t.n > 0).then_some(t.max),
                percentiles: pct,
                time_weighted_mean: t.time_weighted_mean(),
                first_time: t.first.map(|f| f.0),
                last_time: t.last.map(|l| l.0),
                confidence,
            });
        }
        out
    }
}

/// The targets of a confidence criterion, in words.
fn confidence_target_text(c: &Confidence) -> String {
    match (c.relative_target(), c.absolute_half_width) {
        (Some(r), Some(a)) => format!("±{} % and ±{a}", r * PERCENT),
        (Some(r), None) => format!("±{} %", r * PERCENT),
        (None, Some(a)) => format!("±{a}"),
        (None, None) => String::new(),
    }
}

/// Nearest-rank percentiles of the values after the warm-up.
fn percentiles_of(rows: &[(f64, f64)], w: Warmup) -> Option<Percentiles> {
    let mut v: Vec<f64> = rows
        .iter()
        .enumerate()
        .filter(|(i, (t, _))| (*i as u64) >= w.samples && *t >= w.time)
        .map(|(_, r)| r.1)
        .collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f64::total_cmp);
    let n = v.len() as f64;
    let pct = |p: f64| v[((p * n).ceil() as usize).clamp(1, v.len()) - 1];
    Some(Percentiles {
        p50: pct(0.5),
        p90: pct(0.9),
        p95: pct(0.95),
        p99: pct(0.99),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(json: &str) -> Monitor {
        let c: Condition = serde_json::from_str(json).unwrap();
        Monitor::new(
            c,
            Warmup::default(),
            Duration::from_secs(3600),
            Arc::new(AtomicBool::new(false)),
            None,
            None,
        )
    }

    #[test]
    fn counters_stop_at_their_threshold() {
        let mut m = monitor(r#"{"max_measurements": 10, "max_events": 1000}"#);
        let mut p = Probe::default();
        for i in 0..9 {
            p.main_count = i;
            assert!(m.keep_going(&p));
        }
        p.main_count = 10;
        assert!(!m.keep_going(&p));
        assert!(
            matches!(m.why, Some(PauseWhy::Terminate(ref s)) if s.contains("max_measurements"))
        );
    }

    #[test]
    fn all_needs_every_criterion() {
        let mut m = monitor(r#"{"combine": "all", "max_measurements": 10, "max_events": 100}"#);
        let mut p = Probe {
            main_count: 10,
            ..Probe::default()
        };
        assert!(m.keep_going(&p));
        p.events = 100;
        assert!(!m.keep_going(&p));
        let mut m = monitor(r#"{"all": [{"max_measurements": 10}, {"max_events": 100}]}"#);
        p.events = 0;
        assert!(m.keep_going(&p));
        p.events = 200;
        assert!(!m.keep_going(&p));
    }

    #[test]
    fn sim_time_holds_before_a_later_event() {
        let mut m = monitor(r#"{"max_sim_time": 2.0}"#);
        let mut p = Probe {
            next_ns: Some(2_000_000_000),
            ..Probe::default()
        };
        assert!(m.keep_going(&p));
        p.next_ns = Some(2_000_000_001);
        assert!(!m.keep_going(&p));
    }

    #[test]
    fn confidence_is_met_on_a_stable_series() {
        let mut m = monitor(
            r#"{"confidence": {"relative_half_width": 0.05, "series": [{"metric": "Response"}]}}"#,
        );
        let ms = Measurements {
            series: vec![simoxide_sim::ir::SeriesDef {
                mp: "op".into(),
                metric: "Response Time Tuple",
            }],
            rows: vec![
                (0..10_000)
                    .map(|i| (i as f64, 1.0 + (i % 5) as f64 * 0.1))
                    .collect(),
            ],
            count: 10_000,
        };
        let p = Probe {
            tuples: 10_000,
            ..Probe::default()
        };
        assert!(!m.keep_going(&p)); // a confidence checkpoint is due
        m.checkpoint(&ms, &p);
        let held = m.evaluate(&p, Instant::now()).unwrap();
        assert!(held.contains("confidence"), "{held}");
        let s = m.series_stats(&ms, true);
        assert_eq!(s.len(), 1);
        assert!(s[0].confidence.as_ref().unwrap().satisfied);
        assert!((s[0].mean.unwrap() - 1.2).abs() < 1e-9);
    }
}

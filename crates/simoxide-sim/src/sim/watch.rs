//! Pause conditions (`DESIGN-SPD.md` N2): a [`Watch`] pauses a run between two events when one
//! of its [`Condition`]s holds, e.g. to write a snapshot there.
//!
//! * **Duration**: an absolute [`Pause`] (time or event count), or a span of simulation time or
//!   a number of events after the watch was armed ([`Condition::Elapsed`],
//!   [`Condition::ElapsedEvents`]: "60 s after the resume").
//! * **Trigger**: a series condition ([`Condition::Series`]): a tuple's value, or the mean of the
//!   tuples of the last `w` seconds, compared with a value, optionally held for `w` seconds
//!   ("the response time of X exceeds 2 s", "the utilisation of R is above 0.8 for 30 s").
//! * **Threshold**: closeness to a service-level objective ([`Condition::Slo`]), as Slingshot's
//!   `SnapshotSLOTriggeringBehavior` computes it: the value range `[lower, upper]` narrowed by a
//!   sensitivity in `[0, 1]`; a value on or beyond a narrowed bound fires.
//!
//! The conditions are checked between events, before each event (as [`Simulation::run_while`]
//! asks its predicate): a series condition fires right after the event that emitted the tuple, a
//! time condition before the first event past its time. The watch reads the stored measurement
//! rows ([`RunState::measurements`]): series conditions need `SimConfig::store_measurements`.
//! It changes nothing in the run state, so a run paused by a watch is the run paused by
//! [`Pause::Events`] at the same event count, byte for byte (also its snapshot).
//!
//! Any other predicate is [`Simulation::run_while`]'s, also combined with a watch:
//! `sim.run_while(|s| watch.keep_going(s) && my_predicate(s))`.

use super::state::PauseWhen;
use super::*;
use simoxide_model::meta::feat as mf;
use std::collections::VecDeque;

/// A comparison of a [`Condition::Series`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Gt,
    Ge,
    Lt,
    Le,
}

impl Op {
    pub fn holds(self, a: f64, b: f64) -> bool {
        match self {
            Op::Gt => a > b,
            Op::Ge => a >= b,
            Op::Lt => a < b,
            Op::Le => a <= b,
        }
    }

    pub fn literal(self) -> &'static str {
        match self {
            Op::Gt => ">",
            Op::Ge => ">=",
            Op::Lt => "<",
            Op::Le => "<=",
        }
    }
}

/// The series a condition reads: every series whose metric matches `metric` and whose measuring
/// point matches `point`.
#[derive(Clone, Debug, PartialEq)]
pub struct SeriesSel {
    /// A metric name (`Response Time Tuple`, case-insensitive, also a prefix such as
    /// `response time`), an alias (`rt`, `util`, `state`, `demand`, `waiting`, `holding`,
    /// `passive`, `containers`), or `*` (any).
    pub metric: String,
    /// The measuring point: the key as in `measurements.csv`, a part of it (e.g. an element's
    /// id), or the name (`entityName`) of a model element, which matches the keys that contain
    /// its id or the id of an element it contains directly (a container's processing resources);
    /// `*` matches every measuring point.
    pub point: String,
}

/// Aggregation of a [`Condition::Series`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Stat {
    /// Each tuple's value.
    Value,
    /// The mean of the tuples whose point in time lies in the last `window_s` seconds (up to the
    /// newest tuple's), per series.
    Mean { window_s: f64 },
}

/// One pause condition of a [`Watch`].
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// An absolute time or event count ([`Pause`]).
    At(Pause),
    /// Simulation time (ns) after the watch was armed: every event up to then has run.
    Elapsed(i64),
    /// Events after the watch was armed.
    ElapsedEvents(u64),
    /// Trigger: `stat` of a selected series `op value`; with `sustain_s`, the comparison must hold
    /// (on every tuple of that series) for that many seconds: the run pauses before the first
    /// event at or after the time it started to hold plus `sustain_s`.
    Series {
        series: SeriesSel,
        stat: Stat,
        op: Op,
        value: f64,
        sustain_s: Option<f64>,
    },
    /// Threshold: closeness to an SLO (Slingshot's `SnapshotSLOTriggeringBehavior`): with
    /// `middle = (upper - lower) / 2` (`lower` 0 when absent), a value `>= upper - s * middle`,
    /// or with a lower bound `<= lower + s * middle`, fires; tuples emitted before `delay_s` are
    /// ignored.
    Slo {
        series: SeriesSel,
        upper: f64,
        lower: Option<f64>,
        sensitivity: f64,
        delay_s: f64,
    },
}

/// Why a [`Watch`] paused the run.
#[derive(Clone, Debug, PartialEq)]
pub struct Fired {
    /// Index of the condition in the watch.
    pub condition: usize,
    /// Simulation time (ns) and event count at the pause.
    pub at_ns: i64,
    pub events: u64,
    /// What held, e.g. `Response Time Tuple of EntryLevelSystemCallMeasuringPoint[_x] = 2.7 > 2.5`.
    pub reason: String,
}

/// Per condition: what it has seen.
#[derive(Clone, Debug, Default)]
struct Track {
    /// Series examined so far (series are created during a run, too).
    resolved: usize,
    /// Watched series: index, rows read, window of `(t, v)` and its sum, holding since (ns).
    series: Vec<SeriesTrack>,
    /// Ids that the point's name stands for.
    ids: Vec<String>,
    /// Armed at (ns, events).
    base_ns: i64,
    base_events: u64,
}

#[derive(Clone, Debug)]
struct SeriesTrack {
    s: usize,
    read: usize,
    window: VecDeque<(f64, f64)>,
    sum: f64,
    since_ns: Option<i64>,
}

/// A set of pause conditions (any one pauses the run). See the module documentation.
#[derive(Clone, Debug)]
pub struct Watch {
    conds: Vec<Condition>,
    track: Vec<Track>,
    armed: bool,
    /// Total tuples at the last check (nothing new: the series conditions are not re-read).
    count: u64,
    fired: Option<Fired>,
}

impl Watch {
    pub fn new(conds: Vec<Condition>) -> Watch {
        Watch {
            track: vec![Track::default(); conds.len()],
            conds,
            armed: false,
            count: 0,
            fired: None,
        }
    }

    pub fn conditions(&self) -> &[Condition] {
        &self.conds
    }

    /// The condition that paused the run, once one held (it stays held).
    pub fn fired(&self) -> Option<&Fired> {
        self.fired.as_ref()
    }

    /// Arms the watch on `sim`, between events: relative conditions count from here, series
    /// conditions read only tuples emitted from here on, element names are resolved in the
    /// run's model. Errors: series conditions without stored measurements, invalid parameters.
    /// [`Simulation::run_watch`] arms the watch on its first use; arming again re-bases it.
    pub fn arm<C: Compat, F: Flavour>(&mut self, sim: &Simulation<'_, C, F>) -> Result<(), String> {
        let st = sim.state();
        let g = sim.model_graph();
        for (c, t) in self.conds.iter().zip(&mut self.track) {
            *t = Track {
                base_ns: st.now_ns(),
                base_events: st.events(),
                ..Track::default()
            };
            let sel = match c {
                Condition::Series {
                    series,
                    stat,
                    sustain_s,
                    ..
                } => {
                    if let Stat::Mean { window_s } = stat
                        && !(*window_s > 0.0)
                    {
                        return Err(format!("mean window {window_s} s: expected > 0"));
                    }
                    if sustain_s.is_some_and(|w| !(w >= 0.0)) {
                        return Err("sustain time: expected >= 0 s".into());
                    }
                    series
                }
                Condition::Slo {
                    series,
                    sensitivity,
                    upper,
                    lower,
                    ..
                } => {
                    if !(0.0..=1.0).contains(sensitivity) {
                        return Err(format!("sensitivity {sensitivity}: expected 0 to 1"));
                    }
                    if lower.is_some_and(|l| l > *upper) {
                        return Err("SLO: lower bound above the upper one".into());
                    }
                    series
                }
                _ => continue,
            };
            if !st.config().store_measurements {
                return Err(
                    "series conditions read the stored measurements (store_measurements is off)"
                        .into(),
                );
            }
            let _ = metric_matcher(&sel.metric)?;
            // element names: the element's id and those of its direct children
            for o in 0..g.objs.len() as u32 {
                let o = simoxide_model::raw::ObjId(o);
                if g.str_attr(o, mf::NamedElement_entityName).as_deref() == Some(&*sel.point) {
                    t.ids.extend(g.id(o).map(str::to_string));
                    for (_, ch) in g.children(o) {
                        t.ids.extend(g.id(ch).map(str::to_string));
                    }
                }
            }
        }
        self.count = st.measurements().count;
        // series conditions start after the rows present now
        for (c, t) in self.conds.iter().zip(&mut self.track) {
            if matches!(c, Condition::Series { .. } | Condition::Slo { .. }) {
                resolve(c, t, st.measurements());
            }
        }
        self.armed = true;
        self.fired = None;
        Ok(())
    }

    /// Whether the run should go on (a predicate for [`Simulation::run_while`], also inside
    /// drivers such as `SpdRun`). Time conditions compare with [`RunState::next_event_ns`].
    /// The watch must be armed ([`Watch::arm`]).
    pub fn keep_going<C: Compat, F: Flavour>(&mut self, st: &RunState<C, F>) -> bool {
        let next = st.next_event_ns();
        !self.check(st, |p| reached_ns(p, st, next))
    }

    /// Whether some condition holds; `reached` answers time conditions.
    fn check<C: Compat, F: Flavour>(
        &mut self,
        st: &RunState<C, F>,
        reached: impl Fn(Pause) -> bool,
    ) -> bool {
        debug_assert!(self.armed, "Watch::arm first");
        if self.fired.is_some() {
            return true;
        }
        let now = st.now_ns();
        let m = st.measurements();
        let fresh = m.count != self.count || m.series.len() > self.series_seen();
        self.count = m.count;
        for (i, (c, t)) in self.conds.iter().zip(&mut self.track).enumerate() {
            let reason = match c {
                Condition::At(p) => reached(*p).then(|| format!("{p:?} reached")),
                Condition::Elapsed(d) => reached(Pause::After(t.base_ns.saturating_add(*d)))
                    .then(|| format!("{} s elapsed", *d as f64 / 1e9)),
                Condition::ElapsedEvents(n) => (st.events() >= t.base_events.saturating_add(*n))
                    .then(|| format!("{n} events elapsed")),
                Condition::Series {
                    stat,
                    op,
                    value,
                    sustain_s,
                    ..
                } => {
                    if fresh {
                        resolve(c, t, m);
                    }
                    let mut why = None;
                    for s in &mut t.series {
                        let rows = &m.rows[s.s];
                        for &(tt, v) in &rows[s.read.min(rows.len())..] {
                            let x = match stat {
                                Stat::Value => v,
                                Stat::Mean { window_s } => {
                                    s.window.push_back((tt, v));
                                    s.sum += v;
                                    while s.window.front().is_some_and(|f| f.0 <= tt - window_s) {
                                        s.sum -= s.window.pop_front().expect("front").1;
                                    }
                                    s.sum / s.window.len() as f64
                                }
                            };
                            if op.holds(x, *value) {
                                s.since_ns.get_or_insert(now);
                                if sustain_s.is_none() && why.is_none() {
                                    why = Some(format!(
                                        "{} of {} = {x} {} {value}",
                                        m.series[s.s].metric,
                                        m.series[s.s].mp,
                                        op.literal()
                                    ));
                                }
                            } else {
                                s.since_ns = None;
                            }
                        }
                        s.read = rows.len();
                        if why.is_none()
                            && let (Some(w), Some(since)) = (sustain_s, s.since_ns)
                            && reached(Pause::Before(since.saturating_add((w * 1e9) as i64)))
                        {
                            why = Some(format!(
                                "{} of {} {} {value} for {w} s (since {} s)",
                                m.series[s.s].metric,
                                m.series[s.s].mp,
                                op.literal(),
                                since as f64 / 1e9
                            ));
                        }
                    }
                    why
                }
                Condition::Slo {
                    upper,
                    lower,
                    sensitivity,
                    delay_s,
                    ..
                } => {
                    if fresh {
                        resolve(c, t, m);
                    }
                    let l = lower.unwrap_or(0.0);
                    let middle = (upper - l) / 2.0;
                    let (hi, lo) = (upper - sensitivity * middle, l + sensitivity * middle);
                    let active = now as f64 / 1e9 >= *delay_s;
                    let mut why = None;
                    for s in &mut t.series {
                        let rows = &m.rows[s.s];
                        if active && why.is_none() {
                            for &(_, v) in &rows[s.read.min(rows.len())..] {
                                let side = if v >= hi {
                                    Some(format!(">= {hi}"))
                                } else if lower.is_some() && v <= lo {
                                    Some(format!("<= {lo}"))
                                } else {
                                    None
                                };
                                if let Some(side) = side {
                                    why = Some(format!(
                                        "{} of {} = {v} {side} (SLO closeness)",
                                        m.series[s.s].metric, m.series[s.s].mp
                                    ));
                                    break;
                                }
                            }
                        }
                        s.read = rows.len();
                    }
                    why
                }
            };
            if let Some(reason) = reason {
                self.fired = Some(Fired {
                    condition: i,
                    at_ns: now,
                    events: st.events(),
                    reason,
                });
                return true;
            }
        }
        false
    }

    fn series_seen(&self) -> usize {
        self.track.iter().map(|t| t.resolved).max().unwrap_or(0)
    }
}

/// `p` reached before the next event (at `next` ns, if any; see [`Pause`]).
fn reached_ns<C: Compat, F: Flavour>(p: Pause, st: &RunState<C, F>, next: Option<i64>) -> bool {
    match p {
        Pause::Never => false,
        Pause::After(t) => next.is_some_and(|n| n > t),
        Pause::Before(t) => next.is_some_and(|n| n >= t),
        Pause::Events(n) => st.events() >= n,
    }
}

/// The metric names an alias or pattern stands for.
fn metric_matcher(p: &str) -> Result<Box<dyn Fn(&str) -> bool>, String> {
    use crate::ir::metric_names as n;
    let alias = match p.to_ascii_lowercase().as_str() {
        "*" | "any" => return Ok(Box::new(|_| true)),
        "rt" | "response-time" => Some(n::RESPONSE_TIME),
        "util" | "utilization" | "utilisation" => Some(n::UTILIZATION),
        "state" => Some(n::STATE_OF_ACTIVE_RESOURCE),
        "demand" => Some(n::RESOURCE_DEMAND),
        "waiting" | "wait" => Some(n::WAITING_TIME),
        "holding" | "hold" => Some(n::HOLDING_TIME),
        "passive" | "passive-state" => Some(n::STATE_OF_PASSIVE_RESOURCE),
        "containers" => Some(n::NUMBER_OF_RESOURCE_CONTAINERS),
        _ => None,
    };
    if let Some(a) = alias {
        return Ok(Box::new(move |m| m == a));
    }
    if p.is_empty() {
        return Err("empty metric".into());
    }
    let lower = p.to_ascii_lowercase();
    Ok(Box::new(move |m| {
        m.to_ascii_lowercase().starts_with(&lower)
    }))
}

/// Adds the series created since the last call that the condition selects.
fn resolve(c: &Condition, t: &mut Track, m: &Measurements) {
    let sel = match c {
        Condition::Series { series, .. } | Condition::Slo { series, .. } => series,
        _ => return,
    };
    let Ok(metric) = metric_matcher(&sel.metric) else {
        return;
    };
    for (i, def) in m.series.iter().enumerate().skip(t.resolved) {
        let point = sel.point == "*"
            || def.mp.contains(&*sel.point)
            || t.ids.iter().any(|id| def.mp.contains(id.as_str()));
        if point && metric(def.metric) {
            t.series.push(SeriesTrack {
                s: i,
                read: m.rows.get(i).map_or(0, Vec::len),
                window: VecDeque::new(),
                sum: 0.0,
                since_ns: None,
            });
        }
    }
    t.resolved = m.series.len();
}

/// The series a condition currently selects (`measurements.csv` keys and metrics), e.g. to warn
/// about a condition that selects none.
pub fn selected_series(sel: &SeriesSel, ids: &[String], m: &Measurements) -> Vec<String> {
    let Ok(metric) = metric_matcher(&sel.metric) else {
        return Vec::new();
    };
    m.series
        .iter()
        .filter(|d| {
            (sel.point == "*"
                || d.mp.contains(&*sel.point)
                || ids.iter().any(|id| d.mp.contains(id.as_str())))
                && metric(d.metric)
        })
        .map(|d| format!("{} {}", d.mp, d.metric))
        .collect()
}

impl Watch {
    /// Per condition, the series it selects now (empty for other conditions).
    pub fn selected<C: Compat, F: Flavour>(&self, st: &RunState<C, F>) -> Vec<Vec<String>> {
        self.conds
            .iter()
            .zip(&self.track)
            .map(|(c, t)| match c {
                Condition::Series { series, .. } | Condition::Slo { series, .. } => {
                    selected_series(series, &t.ids, st.measurements())
                }
                _ => Vec::new(),
            })
            .collect()
    }
}

/// [`PauseWhen`] of a watch: time conditions with the engine's own comparison ([`Pause`] in the
/// flavour's clock).
struct Watching<'w>(&'w mut Watch);

impl<C: Compat, F: Flavour> PauseWhen<C, F> for Watching<'_> {
    fn pause(&mut self, st: &RunState<C, F>, next: i64) -> bool {
        self.0
            .check(st, |mut p| PauseWhen::<C, F>::pause(&mut p, st, next))
    }
}

impl<C: Compat, F: Flavour> Simulation<'_, C, F> {
    /// Runs until a condition of `watch` holds (between events) or the run is over; arms the
    /// watch first if it is not armed ([`Watch::arm`]). [`Watch::fired`] says which condition
    /// held.
    pub fn run_watch(&mut self, watch: &mut Watch) -> Result<Status, SimError> {
        if !watch.armed {
            watch.arm(self).map_err(|message| SimError {
                message,
                at_ns: self.st.now_ns(),
                kind: SimErrorKind::Model,
            })?;
        }
        if watch.fired.is_some() {
            return Ok(match self.st.phase {
                Phase::Finished => Status::Finished,
                _ => Status::Paused,
            });
        }
        match self.advance(&mut Watching(watch)) {
            Ok(s) => Ok(s),
            Err(e) => {
                self.flush_outputs();
                Err(*e)
            }
        }
    }
}

/// Parses a condition (the CLI's `--pause-when`):
///
/// ```text
/// time T | before T | events N          absolute (seconds, events)
/// elapsed T | elapsed-events N           after the watch is armed (e.g. the resume)
/// METRIC[POINT] [mean W] OP VALUE [for W]           trigger (OP: > >= < <=)
/// slo METRIC[POINT] upper U [lower L] [sensitivity S] [delay D]   threshold
/// ```
///
/// e.g. `rt[_4P-X0HplEeS7kdTjUFmzYw] > 2.5`, `util[Server] mean 30 >= 0.8 for 60`,
/// `slo rt[UsageScenario] upper 3 sensitivity 0.2`.
pub fn parse_condition(s: &str) -> Result<Condition, String> {
    let s = s.trim();
    let err = |m: &str| format!("pause condition `{s}`: {m}");
    let num = |w: Option<&str>, what: &str| -> Result<f64, String> {
        let w = w.ok_or_else(|| err(&format!("{what} needs a value")))?;
        let x: f64 = w
            .parse()
            .map_err(|_| err(&format!("{what}: not a number: {w}")))?;
        if x.is_finite() {
            Ok(x)
        } else {
            Err(err(&format!("{what}: not finite")))
        }
    };
    let secs = |w: Option<&str>, what: &str| -> Result<i64, String> {
        let x = num(w, what)?;
        if x < 0.0 {
            return Err(err(&format!("{what}: negative")));
        }
        Ok((x * 1e9).round() as i64)
    };
    let count = |w: Option<&str>, what: &str| -> Result<u64, String> {
        w.ok_or_else(|| err(&format!("{what} needs a value")))?
            .parse()
            .map_err(|_| err(&format!("{what}: not a count")))
    };
    let (head, rest) = s.split_once(char::is_whitespace).unwrap_or((s, ""));
    let mut words = rest.split_whitespace();
    let simple = match head {
        "time" => Some(Condition::At(Pause::After(secs(words.next(), "time")?))),
        "before" => Some(Condition::At(Pause::Before(secs(words.next(), "before")?))),
        "events" => Some(Condition::At(Pause::Events(count(words.next(), "events")?))),
        "elapsed" => Some(Condition::Elapsed(secs(words.next(), "elapsed")?)),
        "elapsed-events" => Some(Condition::ElapsedEvents(count(
            words.next(),
            "elapsed-events",
        )?)),
        _ => None,
    };
    if let Some(c) = simple {
        if words.next().is_some() {
            return Err(err("unexpected words at the end"));
        }
        return Ok(c);
    }
    let (slo, body) = match head {
        "slo" => (true, rest.trim()),
        _ => (false, s),
    };
    // METRIC[POINT]: the point may contain brackets (a full key)
    let open = body
        .find('[')
        .ok_or_else(|| err("expected METRIC[POINT] (e.g. rt[_id])"))?;
    let mut depth = 0;
    let mut close = None;
    for (i, ch) in body.char_indices().skip(open) {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close.ok_or_else(|| err("unbalanced brackets"))?;
    let series = SeriesSel {
        metric: body[..open].trim().to_string(),
        point: body[open + 1..close].trim().to_string(),
    };
    if series.point.is_empty() {
        return Err(err("empty measuring point"));
    }
    let _ = metric_matcher(&series.metric).map_err(|e| err(&e))?;
    let mut words = body[close + 1..].split_whitespace().peekable();
    if slo {
        let (mut upper, mut lower, mut sensitivity, mut delay_s) = (None, None, 0.0, 0.0);
        while let Some(w) = words.next() {
            match w {
                "upper" => upper = Some(num(words.next(), "upper")?),
                "lower" => lower = Some(num(words.next(), "lower")?),
                "sensitivity" => sensitivity = num(words.next(), "sensitivity")?,
                "delay" => delay_s = num(words.next(), "delay")?,
                o => return Err(err(&format!("unexpected `{o}`"))),
            }
        }
        if !(0.0..=1.0).contains(&sensitivity) {
            return Err(err("sensitivity: expected 0 to 1"));
        }
        return Ok(Condition::Slo {
            series,
            upper: upper.ok_or_else(|| err("slo needs `upper U`"))?,
            lower,
            sensitivity,
            delay_s,
        });
    }
    let mut stat = Stat::Value;
    if words.peek() == Some(&"mean") {
        words.next();
        let w = num(words.next(), "mean")?;
        if w <= 0.0 {
            return Err(err("mean window: expected > 0"));
        }
        stat = Stat::Mean { window_s: w };
    }
    let op = match words.next() {
        Some(">") => Op::Gt,
        Some(">=") => Op::Ge,
        Some("<") => Op::Lt,
        Some("<=") => Op::Le,
        o => return Err(err(&format!("expected > >= < <=, got {o:?}"))),
    };
    let value = num(words.next(), "the value")?;
    let mut sustain_s = None;
    if let Some(w) = words.next() {
        if w != "for" {
            return Err(err(&format!("unexpected `{w}`")));
        }
        let d = num(words.next(), "for")?;
        if d < 0.0 {
            return Err(err("for: negative"));
        }
        sustain_s = Some(d);
    }
    if words.next().is_some() {
        return Err(err("unexpected words at the end"));
    }
    Ok(Condition::Series {
        series,
        stat,
        op,
        value,
        sustain_s,
    })
}

impl std::str::FromStr for Condition {
    type Err = String;
    fn from_str(s: &str) -> Result<Condition, String> {
        parse_condition(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses() {
        assert_eq!(
            parse_condition("time 12.5").unwrap(),
            Condition::At(Pause::After(12_500_000_000))
        );
        assert_eq!(
            parse_condition("elapsed-events 10").unwrap(),
            Condition::ElapsedEvents(10)
        );
        assert_eq!(
            parse_condition(
                "util[ActiveResourceMeasuringPoint[_a|replicaID=0]] mean 30 >= 0.8 for 60"
            )
            .unwrap(),
            Condition::Series {
                series: SeriesSel {
                    metric: "util".into(),
                    point: "ActiveResourceMeasuringPoint[_a|replicaID=0]".into()
                },
                stat: Stat::Mean { window_s: 30.0 },
                op: Op::Ge,
                value: 0.8,
                sustain_s: Some(60.0),
            }
        );
        assert_eq!(
            parse_condition("slo rt[My Scenario] upper 3 lower 1 sensitivity 0.5").unwrap(),
            Condition::Slo {
                series: SeriesSel {
                    metric: "rt".into(),
                    point: "My Scenario".into()
                },
                upper: 3.0,
                lower: Some(1.0),
                sensitivity: 0.5,
                delay_s: 0.0,
            }
        );
        for bad in [
            "rt > 3",
            "rt[x] = 3",
            "rt[x] > abc",
            "rt[x > 3",
            "slo rt[x] lower 1",
            "slo rt[x] upper 1 sensitivity 2",
            "time -1",
            "elapsed 1 2",
            "rt[] > 1",
        ] {
            assert!(parse_condition(bad).is_err(), "{bad}");
        }
    }
}

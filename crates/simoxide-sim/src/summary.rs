//! Interval summaries (`docs/guide/snapshots.md`, "Interval summaries"): per measurement series,
//! statistics of the tuples one invocation recorded, the interval `from..to` of
//! [`crate::Measurements`] ([`crate::Measurements::interval_rows`]). Membership is that of the
//! interval export: by tuple number, never by time.
//!
//! A [`SeriesAcc`] takes the tuples one by one in recording order (constant state per series),
//! so a summary costs one pass over the interval's own tuples and reads nothing of earlier
//! intervals. The document ([`document`]) is the JSON written by `simoxide run --summary`
//! (format [`FORMAT`], version [`VERSION`]).

use crate::Measurements;
use crate::ir::SeriesDef;
use crate::meas::Before;
use serde_json::{Value, json};

/// The `format` of a summary document.
pub const FORMAT: &str = "simoxide-interval-summary";
/// The `version` of a summary document.
pub const VERSION: u32 = 1;

/// A statistic of a summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stat {
    /// Tuples of the series in the interval (always written).
    Count,
    /// Sum of their values (0 for none).
    Sum,
    /// Arithmetic mean of their values (each tuple weighs the same; `null` for none).
    Mean,
    /// Smallest and largest value (`null` for none).
    Min,
    Max,
    /// The first and the last tuple in recording order (`null` for none).
    First,
    Last,
    /// The series' last tuple before the interval, with its status.
    Initial,
    /// The value as a step function of time over the interval: integral and mean.
    TimeWeighted,
}

impl Stat {
    /// Every statistic, in document order (the default selection).
    pub const ALL: [Stat; 9] = [
        Stat::Count,
        Stat::Sum,
        Stat::Mean,
        Stat::Min,
        Stat::Max,
        Stat::First,
        Stat::Last,
        Stat::Initial,
        Stat::TimeWeighted,
    ];

    /// The name in documents and lists.
    pub fn name(self) -> &'static str {
        match self {
            Stat::Count => "count",
            Stat::Sum => "sum",
            Stat::Mean => "mean",
            Stat::Min => "min",
            Stat::Max => "max",
            Stat::First => "first",
            Stat::Last => "last",
            Stat::Initial => "initial",
            Stat::TimeWeighted => "time_weighted",
        }
    }

    /// The statistics of a comma list of names (`all`: every one); `count` is always included.
    /// The result is in document order, without duplicates.
    pub fn parse_list(s: &str) -> Result<Vec<Stat>, String> {
        let mut on = [false; Stat::ALL.len()];
        on[0] = true;
        for w in s.split(',').map(str::trim).filter(|w| !w.is_empty()) {
            if w == "all" {
                on = [true; Stat::ALL.len()];
                continue;
            }
            let i = Stat::ALL
                .iter()
                .position(|x| x.name() == w)
                .ok_or_else(|| {
                    let names: Vec<&str> = Stat::ALL.iter().map(|x| x.name()).collect();
                    format!(
                        "unknown statistic `{w}` (expected {}, all)",
                        names.join(", ")
                    )
                })?;
            on[i] = true;
        }
        Ok(Stat::ALL
            .iter()
            .zip(on)
            .filter(|(_, on)| *on)
            .map(|(s, _)| *s)
            .collect())
    }
}

/// The unit of the values of a metric (`measurements.csv`'s `value` column): `s` for durations,
/// `1` for counts and ratios; `None` for a metric this build does not know.
pub fn metric_unit(metric: &str) -> Option<&'static str> {
    use crate::ir::metric_names as n;
    match metric {
        n::RESPONSE_TIME
        | n::RESOURCE_DEMAND
        | n::WAITING_TIME
        | n::HOLDING_TIME
        | n::RECONFIGURATION_TIME => Some("s"),
        n::STATE_OF_ACTIVE_RESOURCE
        | n::STATE_OF_PASSIVE_RESOURCE
        | n::UTILIZATION
        | n::NUMBER_OF_RESOURCE_CONTAINERS => Some("1"),
        _ => None,
    }
}

/// The time-weighted part of a [`SeriesAcc`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeWeighted {
    /// From when a value is held (s): the interval's start with an initial tuple, else the
    /// first tuple's time.
    pub from: f64,
    /// The interval's end (s).
    pub to: f64,
    /// Integral of the value over `[from, to]`.
    pub integral: f64,
    /// `integral / (to - from)`; `None` for an empty span.
    pub mean: Option<f64>,
}

/// The statistics of one series over an interval, tuple by tuple.
#[derive(Clone, Debug, PartialEq)]
pub struct SeriesAcc {
    pub count: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub first: Option<(f64, f64)>,
    pub last: Option<(f64, f64)>,
    /// The last tuple before the interval.
    pub initial: Before,
    /// The value held and since when (s).
    held: Option<(f64, f64)>,
    /// From when a value is held (s).
    held_from: Option<f64>,
    integral: f64,
    /// The interval's start (s).
    start: f64,
}

impl SeriesAcc {
    /// An empty interval starting at `start` (s), the series' last tuple before it `initial`
    /// (its value holds from `start` on).
    pub fn new(initial: Before, start: f64) -> SeriesAcc {
        let held = match initial {
            Before::Tuple(_, v) => Some((start, v)),
            Before::None | Before::Unknown => None,
        };
        SeriesAcc {
            count: 0,
            sum: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
            first: None,
            last: None,
            initial,
            held,
            held_from: held.map(|_| start),
            integral: 0.0,
            start,
        }
    }

    /// The next tuple of the interval (recording order): `(point in time, value)`. Its value
    /// holds from its time on (not before the interval's start or the previous tuple's time).
    pub fn add(&mut self, t: f64, v: f64) {
        self.count += 1;
        self.sum += v;
        self.min = self.min.min(v);
        self.max = self.max.max(v);
        self.first.get_or_insert((t, v));
        self.last = Some((t, v));
        let at = match self.held {
            Some((since, held)) => {
                let at = t.max(since);
                self.integral += held * (at - since);
                at
            }
            None => {
                let at = t.max(self.start);
                self.held_from = Some(at);
                at
            }
        };
        self.held = Some((at, v));
    }

    /// The arithmetic mean of the values (`None` for none).
    pub fn mean(&self) -> Option<f64> {
        (self.count > 0).then(|| self.sum / self.count as f64)
    }

    /// The value as a step function over the interval ending at `end` (s): each value holds
    /// from its tuple's time to the next tuple's or `end`; `None` if no value is held (no
    /// initial tuple and no tuple in the interval).
    pub fn time_weighted(&self, end: f64) -> Option<TimeWeighted> {
        let (since, held) = self.held?;
        let from = self.held_from?;
        let to = end.max(since);
        let integral = self.integral + held * (to - since);
        Some(TimeWeighted {
            from,
            to,
            integral,
            mean: (to > from).then(|| integral / (to - from)),
        })
    }

    /// The series' entry of a summary document with the statistics `stats`.
    pub fn to_json(&self, def: &SeriesDef, end: f64, stats: &[Stat]) -> Value {
        let tuple =
            |x: Option<(f64, f64)>| x.map_or(Value::Null, |(t, v)| json!({"time": t, "value": v}));
        let opt = |x: Option<f64>| x.map_or(Value::Null, Value::from);
        let mut o = serde_json::Map::new();
        o.insert("measuring_point".into(), def.mp.as_ref().into());
        o.insert("metric".into(), def.metric.into());
        o.insert("unit".into(), metric_unit(def.metric).into());
        let some = self.count > 0;
        for s in stats {
            let v = match s {
                Stat::Count => self.count.into(),
                Stat::Sum => Value::from(self.sum),
                Stat::Mean => opt(self.mean()),
                Stat::Min => opt(some.then_some(self.min)),
                Stat::Max => opt(some.then_some(self.max)),
                Stat::First => tuple(self.first),
                Stat::Last => tuple(self.last),
                Stat::Initial => match self.initial {
                    Before::Tuple(t, v) => json!({"status": "tuple", "time": t, "value": v}),
                    Before::None => json!({"status": "none"}),
                    Before::Unknown => json!({"status": "unavailable"}),
                },
                Stat::TimeWeighted => self.time_weighted(end).map_or(Value::Null, |w| {
                    json!({"from": w.from, "to": w.to, "integral": w.integral, "mean": opt(w.mean)})
                }),
            };
            o.insert(s.name().into(), v);
        }
        Value::Object(o)
    }
}

/// The interval of a summary.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    /// The tuple numbers `from..to` (`from`: `None` if unknown, a snapshot without a cursor).
    pub tuples_from: Option<u64>,
    pub tuples_to: u64,
    /// Simulation time at the start (the restore; 0 for a run) and at the end (the pause, or
    /// the end of the run): in seconds, the engine's clock as a double (the Slingshot flavours
    /// count in double seconds, SimuLizar in nanoseconds), and in whole nanoseconds (truncated,
    /// as in snapshot headers).
    pub from: f64,
    pub to: f64,
    pub from_ns: i64,
    pub to_ns: i64,
}

/// The summary document of the current interval of `m` (its rows from
/// [`Measurements::start`] on): the series that `select` accepts, in `measurements.csv` order,
/// each with `stats` ([`Stat::parse_list`]; `count` is always written).
pub fn document(
    m: &Measurements,
    iv: &Interval,
    select: &dyn Fn(&SeriesDef) -> bool,
    stats: &[Stat],
) -> Value {
    let (from, to) = (iv.from, iv.to);
    let mut stats: Vec<Stat> = stats
        .iter()
        .copied()
        .filter(|s| *s != Stat::Count)
        .collect();
    stats.insert(0, Stat::Count);
    let mut series = Vec::new();
    for i in m.sorted_series() {
        let def = &m.series[i];
        if !select(def) {
            continue;
        }
        let (before, rows) = m.interval_rows(i);
        let mut acc = SeriesAcc::new(before, from);
        for &(t, v) in rows {
            acc.add(t, v);
        }
        series.push(acc.to_json(def, to, &stats));
    }
    json!({
        "format": FORMAT,
        "version": VERSION,
        "interval": {
            "tuples": {"from": iv.tuples_from, "to": iv.tuples_to},
            "time": {"from": from, "to": to},
            "time_ns": {"from": iv.from_ns, "to": iv.to_ns},
        },
        "stats": stats.iter().map(|s| s.name()).collect::<Vec<_>>(),
        "series": series,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_lists() {
        assert_eq!(Stat::parse_list("").unwrap(), [Stat::Count]);
        assert_eq!(
            Stat::parse_list("max, mean,count,mean").unwrap(),
            [Stat::Count, Stat::Mean, Stat::Max]
        );
        assert_eq!(Stat::parse_list("all").unwrap(), Stat::ALL);
        assert!(Stat::parse_list("median").is_err());
    }

    /// Hand-calculated: the value 2 held from the start 10 s to 12 s, 4 from 12 to 15, 1 from
    /// 15 to the end 20: integral 2*2 + 4*3 + 1*5 = 21 over 10 s.
    #[test]
    fn time_weighting_holds_values() {
        let mut a = SeriesAcc::new(Before::Tuple(7.0, 2.0), 10.0);
        a.add(12.0, 4.0);
        a.add(15.0, 1.0);
        assert_eq!((a.count, a.sum, a.min, a.max), (2, 5.0, 1.0, 4.0));
        assert_eq!(a.mean(), Some(2.5));
        assert_eq!(
            a.time_weighted(20.0),
            Some(TimeWeighted {
                from: 10.0,
                to: 20.0,
                integral: 21.0,
                mean: Some(2.1)
            })
        );
        // no tuple before: from the first tuple on (4*3 + 1*5 = 17 over 8 s)
        let mut b = SeriesAcc::new(Before::None, 10.0);
        b.add(12.0, 4.0);
        b.add(15.0, 1.0);
        let w = b.time_weighted(20.0).unwrap();
        assert_eq!((w.from, w.integral, w.mean), (12.0, 17.0, Some(17.0 / 8.0)));
        // an empty interval: the initial value holds throughout; nothing held: none
        let c = SeriesAcc::new(Before::Tuple(3.0, 5.0), 10.0);
        assert_eq!(c.time_weighted(20.0).unwrap().mean, Some(5.0));
        assert_eq!(c.mean(), None);
        assert_eq!(
            SeriesAcc::new(Before::Unknown, 10.0).time_weighted(20.0),
            None
        );
        // a zero-length interval: integral 0, no mean
        let w = c.time_weighted(10.0).unwrap();
        assert_eq!((w.integral, w.mean), (0.0, None));
    }

    #[test]
    fn empty_series_are_null() {
        let def = SeriesDef {
            mp: "P".into(),
            metric: crate::ir::metric_names::STATE_OF_ACTIVE_RESOURCE,
        };
        let v = SeriesAcc::new(Before::Unknown, 0.0).to_json(&def, 1.0, &Stat::ALL);
        assert_eq!(
            v,
            json!({"measuring_point": "P", "metric": "State of Active Resource Tuple",
                   "unit": "1", "count": 0, "sum": 0.0, "mean": null, "min": null, "max": null,
                   "first": null, "last": null, "initial": {"status": "unavailable"},
                   "time_weighted": null})
        );
    }
}

//! Measurement results: tuples per series in emission order, and the `measurements.csv` writer
//! (`docs/guide/formats.md` §4).

use crate::ir::SeriesDef;
use crate::javafmt::push_double;
use std::io::Write;

/// Recorded measurements of one run.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub struct Measurements {
    /// Series definitions (measuring point key, tuple metric name).
    pub series: Vec<SeriesDef>,
    /// Rows `(point in time, value)` per series, in emission order (empty if not stored).
    pub rows: Vec<Vec<(f64, f64)>>,
    /// Total number of recorded tuples.
    pub count: u64,
    /// Per series, the last tuple before the rows held (`rows[i]`); series without an entry had
    /// none. Not part of the engine state: snapshots whose rows were taken carry it in a section
    /// of its own (`meas-before`), for the summaries of later intervals.
    #[cfg_attr(feature = "snapshot", serde(skip))]
    pub before: Vec<Before>,
    /// Per series, the number of rows held when the current interval started (a restore; series
    /// without an entry: 0). The rows `start[i]..` of series `i` are the interval's tuples
    /// (`docs/guide/snapshots.md`, "Interval summaries").
    #[cfg_attr(feature = "snapshot", serde(skip))]
    pub start: Vec<usize>,
    /// Simulation time (s, the engine's clock) at which the current interval started: the
    /// restore, 0 for a run from time 0.
    #[cfg_attr(feature = "snapshot", serde(skip))]
    pub start_time: f64,
}

/// The last tuple of a series before some point of a run ([`Measurements::before`]).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
#[cfg_attr(feature = "snapshot", derive(serde::Serialize, serde::Deserialize))]
pub enum Before {
    /// The series had no tuple before.
    #[default]
    None,
    /// Its last tuple: `(point in time, value)`.
    Tuple(f64, f64),
    /// Not known: the rows were taken out of a run whose snapshot did not record it.
    Unknown,
}

impl Measurements {
    /// Series indices in output order: `(measuring_point, metric)` in Java `String.compareTo`
    /// order.
    pub fn sorted_series(&self) -> Vec<usize> {
        let mut idx: Vec<usize> = (0..self.series.len()).collect();
        idx.sort_by(|&a, &b| {
            let (sa, sb) = (&self.series[a], &self.series[b]);
            java_cmp(&sa.mp, &sb.mp).then_with(|| java_cmp(sa.metric, sb.metric))
        });
        idx
    }

    /// The number of stored rows, all series together.
    pub fn rows_held(&self) -> u64 {
        self.rows.iter().map(|r| r.len() as u64).sum()
    }

    /// Takes the stored rows out (the series stay, each with no rows; `count` is unchanged) and
    /// returns them with the series and `count`. The rows recorded from then on are the tuples
    /// after `count` (`docs/guide/snapshots.md`, "Measurement intervals").
    ///
    /// The value returned keeps [`Measurements::before`] and [`Measurements::start`] as they
    /// were (its rows are the interval's from `start` on); here `before` becomes the last tuple
    /// of each series and `start` 0 (the rows recorded from now on are all the interval's).
    pub fn take_rows(&mut self) -> Measurements {
        let before = (0..self.series.len())
            .map(|i| match self.rows.get(i).and_then(|r| r.last()) {
                Some(&(t, v)) => Before::Tuple(t, v),
                None => self.before_of(i),
            })
            .collect();
        let rows = self.rows.iter_mut().map(std::mem::take).collect();
        Measurements {
            series: self.series.clone(),
            rows,
            count: self.count,
            before: std::mem::replace(&mut self.before, before),
            start: std::mem::take(&mut self.start),
            start_time: self.start_time,
        }
    }

    /// The last tuple of series `i` before its rows held ([`Measurements::before`]).
    pub fn before_of(&self, i: usize) -> Before {
        self.before.get(i).copied().unwrap_or_default()
    }

    /// Marks the start of an interval at simulation time `time` (s): the rows held now are
    /// before it ([`Measurements::start`]). Restoring a snapshot does this.
    pub fn start_interval(&mut self, time: f64) {
        self.start = self.rows.iter().map(Vec::len).collect();
        self.start_time = time;
    }

    /// The tuples of series `i` in the current interval (its rows from [`Measurements::start`]
    /// on) and the last tuple before them.
    pub fn interval_rows(&self, i: usize) -> (Before, &[(f64, f64)]) {
        let rows = self.rows.get(i).map_or(&[][..], Vec::as_slice);
        let k = self.start.get(i).copied().unwrap_or(0).min(rows.len());
        let before = match k {
            0 => self.before_of(i),
            _ => Before::Tuple(rows[k - 1].0, rows[k - 1].1),
        };
        (before, &rows[k..])
    }

    /// Writes `measurements.csv`.
    pub fn write_csv(&self, out: &mut dyn Write) -> std::io::Result<()> {
        let mut s = String::from("measuring_point,metric,time,value\n");
        for i in self.sorted_series() {
            let def = &self.series[i];
            let mp = csv_field(&def.mp);
            let metric = csv_field(def.metric);
            for &(t, v) in &self.rows[i] {
                s.push_str(&mp);
                s.push(',');
                s.push_str(&metric);
                s.push(',');
                push_double(&mut s, t);
                s.push(',');
                push_double(&mut s, v);
                s.push('\n');
                if s.len() > (1 << 16) {
                    out.write_all(s.as_bytes())?;
                    s.clear();
                }
            }
        }
        out.write_all(s.as_bytes())
    }

    /// The CSV as a string.
    pub fn to_csv(&self) -> String {
        let mut v = Vec::new();
        self.write_csv(&mut v).expect("writing to memory");
        String::from_utf8(v).expect("utf-8")
    }
}

/// Summary statistics of one series (see [`Measurements::summaries`]).
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesSummary {
    /// Measuring point key as in `measurements.csv`.
    pub measuring_point: String,
    /// Metric name as in `measurements.csv` (e.g. `Response Time Tuple`).
    pub metric: &'static str,
    pub count: u64,
    /// Arithmetic mean of the values (NaN if empty).
    pub mean: f64,
    /// Sample standard deviation (0 for fewer than 2 values).
    pub std_dev: f64,
    pub min: f64,
    pub max: f64,
    /// Percentiles (nearest rank) of the values.
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
    /// Time of the first and the last tuple (s).
    pub first_time: f64,
    pub last_time: f64,
    /// Mean of the value as a step function of time between the first and the last tuple
    /// (meaningful for state and utilisation series); `None` if they are at the same time.
    pub time_weighted_mean: Option<f64>,
}

impl Measurements {
    /// Summary statistics per series, in the order of [`Measurements::sorted_series`]. Needs
    /// stored rows (`SimConfig::store_measurements`); series without rows are left out.
    pub fn summaries(&self) -> Vec<SeriesSummary> {
        let mut out = Vec::new();
        for i in self.sorted_series() {
            let rows = &self.rows[i];
            if rows.is_empty() {
                continue;
            }
            let n = rows.len() as f64;
            let mean = rows.iter().map(|r| r.1).sum::<f64>() / n;
            let var = if rows.len() > 1 {
                rows.iter()
                    .map(|r| (r.1 - mean) * (r.1 - mean))
                    .sum::<f64>()
                    / (n - 1.0)
            } else {
                0.0
            };
            let mut v: Vec<f64> = rows.iter().map(|r| r.1).collect();
            v.sort_by(f64::total_cmp);
            let pct = |p: f64| v[((p * n).ceil() as usize).clamp(1, v.len()) - 1];
            let (t0, t1) = (rows[0].0, rows[rows.len() - 1].0);
            let time_weighted_mean = (t1 > t0).then(|| {
                rows.windows(2)
                    .map(|w| w[0].1 * (w[1].0 - w[0].0))
                    .sum::<f64>()
                    / (t1 - t0)
            });
            out.push(SeriesSummary {
                measuring_point: self.series[i].mp.to_string(),
                metric: self.series[i].metric,
                count: rows.len() as u64,
                mean,
                std_dev: var.sqrt(),
                min: v[0],
                max: v[v.len() - 1],
                p50: pct(0.5),
                p90: pct(0.9),
                p95: pct(0.95),
                p99: pct(0.99),
                first_time: t0,
                last_time: t1,
                time_weighted_mean,
            });
        }
        out
    }
}

/// `String.compareTo`: UTF-16 code unit order.
fn java_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

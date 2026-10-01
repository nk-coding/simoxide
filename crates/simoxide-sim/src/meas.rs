//! Measurement results: tuples per series in emission order, and the `measurements.csv` writer
//! (`docs/guide/formats.md` §4).

use crate::ir::SeriesDef;
use crate::javafmt::push_double;
use std::io::Write;

/// Recorded measurements of one run.
#[derive(Debug, Clone, Default)]
pub struct Measurements {
    /// Series definitions (measuring point key, tuple metric name).
    pub series: Vec<SeriesDef>,
    /// Rows `(point in time, value)` per series, in emission order (empty if not stored).
    pub rows: Vec<Vec<(f64, f64)>>,
    /// Total number of recorded tuples.
    pub count: u64,
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

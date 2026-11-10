//! What the server reports about a simulation: its status, live progress, log entries and final
//! result. All types serialise to the JSON documented in `docs/guide/server.md`.

use crate::stats::{Interval, finite_or_null, opt_finite_or_null};
use serde::Serialize;

/// Where a simulation is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Accepted, waiting for a free worker (`--max-concurrent`).
    Queued,
    /// Loading and compiling the model.
    Loading,
    /// Simulating; progress is published.
    Running,
    /// Ended by a termination condition, the simulator's own stop condition or the end of
    /// the events. Results are available.
    Completed,
    /// Stopped by a client. The results so far are available.
    Cancelled,
    /// The model did not load or compile, or the run aborted. Results so far may be available.
    Failed,
}

impl JobStatus {
    /// The simulation is over (results, if any, are final).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            JobStatus::Completed | JobStatus::Cancelled | JobStatus::Failed
        )
    }

    /// The status as in the JSON documents.
    pub fn as_str(self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Loading => "loading",
            JobStatus::Running => "running",
            JobStatus::Completed => "completed",
            JobStatus::Cancelled => "cancelled",
            JobStatus::Failed => "failed",
        }
    }
}

/// What ended a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopKind {
    /// A termination condition of the request held.
    Termination,
    /// The simulator's own stop condition (`config.max_sim_time` or `config.max_measurements`),
    /// or no event was left.
    Simulator,
    /// A cap of the server (run time, recorded tuples).
    ServerLimit,
    /// A client cancelled the run.
    Cancelled,
    /// The model did not load or compile, or the run aborted.
    Error,
    /// A scaling decision is pending and `config.spd_decisions` has no action list left for it.
    Decision,
}

impl StopKind {
    /// The kind as in the JSON documents.
    pub fn as_str(self) -> &'static str {
        match self {
            StopKind::Termination => "termination",
            StopKind::Simulator => "simulator",
            StopKind::ServerLimit => "server_limit",
            StopKind::Cancelled => "cancelled",
            StopKind::Error => "error",
            StopKind::Decision => "decision",
        }
    }
}

/// Why a run ended.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StopReason {
    /// What ended the run.
    pub kind: StopKind,
    /// Which criteria held, in words, e.g. `confidence: 2 series within ±5 % at 95 %`.
    pub detail: String,
}

/// Statistics of one series (after the warm-up).
#[derive(Clone, Debug, Serialize)]
pub struct SeriesStats {
    /// Measuring point as in `measurements.csv`.
    pub measuring_point: String,
    /// Metric as in `measurements.csv` (e.g. `Response Time Tuple`).
    pub metric: String,
    /// Values counted (after the warm-up).
    pub count: u64,
    /// All recorded values of the series, warm-up included.
    pub total_count: u64,
    /// Mean of the counted values.
    #[serde(serialize_with = "opt_finite_or_null")]
    pub mean: Option<f64>,
    /// Sample standard deviation of the counted values.
    #[serde(serialize_with = "opt_finite_or_null")]
    pub std_dev: Option<f64>,
    /// Smallest counted value.
    #[serde(serialize_with = "opt_finite_or_null")]
    pub min: Option<f64>,
    /// Largest counted value.
    #[serde(serialize_with = "opt_finite_or_null")]
    pub max: Option<f64>,
    /// Percentiles (nearest rank); only in final results.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percentiles: Option<Percentiles>,
    /// Mean of the value as a step function of time (for state and utilisation series).
    #[serde(serialize_with = "opt_finite_or_null")]
    pub time_weighted_mean: Option<f64>,
    /// Simulation time of the first counted value (s).
    #[serde(serialize_with = "opt_finite_or_null")]
    pub first_time: Option<f64>,
    /// Simulation time of the last counted value (s).
    #[serde(serialize_with = "opt_finite_or_null")]
    pub last_time: Option<f64>,
    /// Batch-means confidence interval of the mean, for series a confidence criterion watches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<ConfidenceState>,
}

/// Percentiles of a series' values.
#[derive(Clone, Debug, Serialize)]
pub struct Percentiles {
    /// Median.
    #[serde(serialize_with = "finite_or_null")]
    pub p50: f64,
    /// 90th percentile.
    #[serde(serialize_with = "finite_or_null")]
    pub p90: f64,
    /// 95th percentile.
    #[serde(serialize_with = "finite_or_null")]
    pub p95: f64,
    /// 99th percentile.
    #[serde(serialize_with = "finite_or_null")]
    pub p99: f64,
}

/// The confidence interval of a watched series against its criterion's targets.
#[derive(Clone, Debug, Serialize)]
pub struct ConfidenceState {
    /// The interval, once there are enough batches.
    pub interval: Option<Interval>,
    /// Target relative half width, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_relative_half_width: Option<f64>,
    /// Target absolute half width, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_absolute_half_width: Option<f64>,
    /// The interval meets the targets and the series has `min_samples` values.
    pub satisfied: bool,
}

/// A live view of a running simulation.
#[derive(Clone, Debug, Serialize)]
pub struct Progress {
    /// Wall-clock seconds since the run started (after loading).
    pub wall_secs: f64,
    /// Simulation time of the last event (s).
    pub sim_time: f64,
    /// Events processed.
    pub events: u64,
    /// Finished usage-scenario runs (SimuLizar's measurement count).
    pub measurements: i64,
    /// Recorded measurement tuples, all series.
    pub tuples: u64,
    /// Events per wall-clock second since the previous update.
    pub events_per_sec: f64,
    /// Simulated seconds per wall-clock second since the previous update.
    pub sim_speed: f64,
    /// Live processes (users, forked behaviours); not for slingshot-exact.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub live_processes: Option<usize>,
    /// Termination: the criteria and how far each is (e.g. `max_measurements: 412/1000`).
    pub termination: Vec<String>,
    /// Per-series statistics (after the warm-up).
    pub series: Vec<SeriesStats>,
}

/// Severity of a log entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Details only worth reading when asked for (e.g. SPD evaluations without a request).
    Debug,
    /// The normal course of a run.
    Info,
    /// Something the client should know; the run goes on.
    Warn,
    /// The run failed.
    Error,
}

impl Level {
    /// The minimum level a client asks for (`None`: info).
    pub fn from_query(value: Option<&str>) -> Result<Level, String> {
        match value {
            None | Some("info") => Ok(Level::Info),
            Some("debug") => Ok(Level::Debug),
            Some("warn") => Ok(Level::Warn),
            Some("error") => Ok(Level::Error),
            Some(other) => Err(format!(
                "expected debug, info, warn or error, got {other:?}"
            )),
        }
    }
}

/// Which part of the server wrote a log entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogSource {
    /// The job manager.
    Server,
    /// Loading and compiling the model.
    Model,
    /// The simulator's own warnings and errors.
    Simulator,
    /// Scaling-policy decisions.
    Spd,
    /// Why the run ended.
    Termination,
}

/// One entry of a simulation's log.
#[derive(Clone, Debug, Serialize)]
pub struct LogEntry {
    /// Sequence number, from 1, without gaps (entries evicted from the ring buffer are gone).
    pub seq: u64,
    /// Wall-clock milliseconds since the simulation was submitted.
    pub wall_ms: u64,
    /// Simulation time (s), for entries the run produced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sim_time: Option<f64>,
    /// Severity.
    pub level: Level,
    /// Which part of the server wrote it.
    pub source: LogSource,
    /// The entry in words.
    pub message: String,
    /// Structured details (e.g. an SPD decision record).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// The final result of a simulation.
#[derive(Clone, Debug, Serialize)]
pub struct ResultSummary {
    /// Why the run ended.
    pub stop_reason: StopReason,
    /// Simulation time at the end (s).
    pub sim_time: f64,
    /// Events processed.
    pub events: u64,
    /// Finished usage-scenario runs.
    pub measurements: i64,
    /// Recorded tuples, all series.
    pub tuples: u64,
    /// Wall-clock seconds of the run (after loading).
    pub wall_secs: f64,
    /// Random numbers drawn (only for runs the simulator finished itself).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uniforms: Option<u64>,
    /// Non-fatal problems the simulator reported.
    pub warnings: Vec<String>,
    /// Per-series statistics (after the warm-up), with percentiles and confidence intervals.
    pub series: Vec<SeriesStats>,
    /// Scaling policies: counters of the SPD interpreter, if the model has policies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spd: Option<serde_json::Value>,
    /// `config.interval_summary`: the summary document (format `simoxide-interval-summary`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval_summary: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `as_str` is what log messages and errors print: it must match the JSON documents.
    #[test]
    fn names_equal_the_json_documents() {
        for status in [
            JobStatus::Queued,
            JobStatus::Loading,
            JobStatus::Running,
            JobStatus::Completed,
            JobStatus::Cancelled,
            JobStatus::Failed,
        ] {
            assert_eq!(serde_json::json!(status), status.as_str());
        }
        for kind in [
            StopKind::Termination,
            StopKind::Simulator,
            StopKind::ServerLimit,
            StopKind::Cancelled,
            StopKind::Error,
            StopKind::Decision,
        ] {
            assert_eq!(serde_json::json!(kind), kind.as_str());
        }
    }
}

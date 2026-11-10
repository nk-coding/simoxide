//! The operations of the server, shared by the HTTP API ([`crate::http`]) and the MCP tools
//! ([`crate::mcp`]): start, list, inspect, wait for, cancel and delete simulations, read their
//! logs and results.
//!
//! Everything here is synchronous except [`Service::wait`], which waits asynchronously (on a
//! tokio runtime) so that a waiting client holds no thread. The HTTP layer runs the slower
//! synchronous operations (submitting, `measurements.csv`) on blocking threads.

use crate::job::{ApiError, JobManager, JobView, LogPage};
use crate::report::{JobStatus, Level, LogSource};
use crate::request::{RUN_JSON, SeriesSelector, SimulationRequest, USAGE_MODEL_SUFFIX};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use simoxide_sim::Measurements;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

/// The longest a client may block in [`Service::wait`].
pub const MAX_WAIT: Duration = Duration::from_secs(300);
/// Newest log entries in a status answer, unless the client asks otherwise.
pub const DEFAULT_RECENT_LOGS: usize = 10;
/// Newest log entries in the answer to starting or cancelling a simulation.
const RECENT_LOGS_AFTER_ACTION: usize = 5;
/// The largest page of log entries.
pub const MAX_LOG_PAGE: usize = 10_000;
/// Default of [`ResultsQuery::max_rows_per_series`].
pub const DEFAULT_MAX_ROWS_PER_SERIES: usize = 1000;
/// How many directory levels below an allowed directory [`Service::models`] looks.
pub const MODEL_SCAN_MAX_DEPTH: u32 = 2;

/// The server's operations (see the module documentation).
pub struct Service {
    /// The simulations.
    pub jobs: Arc<JobManager>,
}

/// A value as JSON (`null` in the unexpected case that it cannot be serialised).
pub(crate) fn to_json<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or_default()
}

/// A serialised [`JobView`] without the per-series statistics of its progress.
pub(crate) fn without_series(mut view: Value) -> Value {
    if let Some(progress) = view.get_mut("progress").and_then(Value::as_object_mut) {
        progress.remove("series");
    }
    view
}

/// What [`Service::wait`] waits for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Until {
    /// Any change after `after_version`: new progress, a log entry, a status change.
    #[default]
    Change,
    /// The end of the simulation.
    Done,
}

/// Options of [`Service::results`].
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResultsQuery {
    /// Include the raw `(time, value)` rows of the series.
    #[serde(default)]
    pub include_rows: bool,
    /// Only these series (in the statistics and the rows); default all.
    #[serde(default)]
    pub series: Vec<SeriesSelector>,
    /// At most this many rows per series (evenly spaced, the last row always included);
    /// default [`DEFAULT_MAX_ROWS_PER_SERIES`], 0 = all.
    #[serde(default)]
    pub max_rows_per_series: Option<usize>,
}

/// A simulation in a listing.
#[derive(Clone, Debug, Serialize)]
pub struct ListEntry {
    /// The simulation's id.
    pub id: String,
    /// Its label.
    pub name: String,
    /// Where it is.
    pub status: JobStatus,
    /// Submission time (Unix ms).
    pub submitted_at: u64,
    /// When it ended (Unix ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// Why it ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<crate::report::StopReason>,
    /// Simulation time of the latest progress.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sim_time: Option<f64>,
    /// Finished usage-scenario runs of the latest progress.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measurements: Option<i64>,
}

impl Service {
    /// The server's version, build features, limits and endpoints.
    pub fn info(&self) -> Value {
        let c = &self.jobs.config;
        json!({
            "name": "simoxide-server",
            "version": env!("CARGO_PKG_VERSION"),
            "simulator": "SimOxide: Palladio Component Model simulation with SimuLizar 5.2.2 and Slingshot semantics",
            "flavours": ["simulizar", "slingshot", "slingshot-exact"],
            "modes": {
                "fast": true,
                "exact": simoxide_sim::Mode::Exact.available(),
            },
            "features": {
                "spd": cfg!(feature = "spd"),
                "reconfiguration": cfg!(feature = "reconfiguration"),
                "exact": cfg!(feature = "exact"),
                "hotspot_math": cfg!(feature = "hotspot-math"),
            },
            "limits": {
                "max_concurrent": c.max_concurrent,
                "max_jobs": c.max_jobs,
                "max_queued": c.max_queued,
                "max_run_secs": c.max_run_secs,
                "max_tuples": c.max_tuples,
                "log_capacity": c.log_capacity,
            },
            "model_dir": !c.allow_dirs.is_empty(),
            "documentation": "docs/guide/server.md",
        })
    }

    /// Model directories under the allowed directories (directories with a usage model, up to
    /// [`MODEL_SCAN_MAX_DEPTH`] levels deep), as names a request can give as `model_dir`.
    pub fn models(&self) -> Result<Vec<Value>, ApiError> {
        let roots = &self.jobs.config.allow_dirs;
        if roots.is_empty() {
            return Err(ApiError {
                status: StatusCode::NOT_FOUND,
                message: "model_dir is disabled on this server (start it with --allow-dir DIR)"
                    .into(),
            });
        }
        let mut out = Vec::new();
        for root in roots {
            let Ok(root) = root.canonicalize() else {
                continue;
            };
            scan(&root, &root, 0, &mut out);
        }
        Ok(out)
    }

    /// Submits a simulation; returns its first state.
    pub fn start(&self, req: SimulationRequest) -> Result<JobView, ApiError> {
        let job = self.jobs.submit(req)?;
        Ok(job.view(RECENT_LOGS_AFTER_ACTION, true))
    }

    /// Every simulation the server keeps, oldest first.
    pub fn list(&self) -> Vec<ListEntry> {
        self.jobs
            .list()
            .iter()
            .map(|j| {
                let v = j.view(0, true);
                ListEntry {
                    id: v.id,
                    name: v.name,
                    status: v.status,
                    submitted_at: v.submitted_at,
                    finished_at: v.finished_at,
                    stop_reason: v.stop_reason,
                    sim_time: v.progress.as_ref().map(|p| p.sim_time),
                    measurements: v.progress.as_ref().map(|p| p.measurements),
                }
            })
            .collect()
    }

    /// The state of a simulation with its latest progress and the newest `recent_logs` log
    /// entries.
    pub fn status(&self, id: &str, recent_logs: usize) -> Result<JobView, ApiError> {
        Ok(self.jobs.get(id)?.view(recent_logs, true))
    }

    /// Log entries of a simulation (see [`crate::job::Job::logs`]); `limit` is clamped to a page.
    pub fn logs(
        &self,
        id: &str,
        since: u64,
        limit: usize,
        min_level: Level,
    ) -> Result<LogPage, ApiError> {
        Ok(self
            .jobs
            .get(id)?
            .logs(since, limit.clamp(1, MAX_LOG_PAGE), min_level))
    }

    /// Waits until the simulation changed after `after_version` (default: its version now) or
    /// ended (`until: done`), at most `timeout` (capped at [`MAX_WAIT`]); returns its state.
    pub async fn wait(
        &self,
        id: &str,
        after_version: Option<u64>,
        until: Until,
        timeout: Duration,
        recent_logs: usize,
    ) -> Result<JobView, ApiError> {
        let job = self.jobs.get(id)?;
        let after = after_version.unwrap_or_else(|| job.version());
        job.wait(after, until == Until::Done, timeout.min(MAX_WAIT))
            .await;
        Ok(job.view(recent_logs, true))
    }

    /// Asks a simulation to stop; it ends as `cancelled` with the results so far.
    pub fn cancel(&self, id: &str) -> Result<JobView, ApiError> {
        let job = self.jobs.get(id)?;
        if self.jobs.cancel(&job) {
            job.log(
                Level::Info,
                LogSource::Server,
                None,
                "cancellation requested",
                None,
            );
        }
        Ok(job.view(RECENT_LOGS_AFTER_ACTION, true))
    }

    /// The final results: the summary (stop reason, counters, per-series statistics with
    /// percentiles and confidence intervals) and, if asked, the raw rows.
    pub fn results(&self, id: &str, q: &ResultsQuery) -> Result<Value, ApiError> {
        let job = self.jobs.get(id)?;
        let v = job.view(0, false);
        let Some(o) = job.outcome() else {
            return Err(not_ready(&v));
        };
        let keep = |mp: &str, metric: &str| {
            q.series.is_empty() || q.series.iter().any(|s| s.matches(mp, metric))
        };
        let mut summary = to_json(&o.summary);
        if let Some(Value::Array(series)) = summary.get_mut("series") {
            series.retain(|s| {
                keep(
                    s["measuring_point"].as_str().unwrap_or(""),
                    s["metric"].as_str().unwrap_or(""),
                )
            });
        }
        let mut out = json!({
            "id": v.id,
            "name": v.name,
            "status": v.status,
            "model": v.model,
            "summary": summary,
        });
        if q.include_rows {
            let max = match q.max_rows_per_series {
                None => DEFAULT_MAX_ROWS_PER_SERIES,
                Some(0) => usize::MAX,
                Some(n) => n,
            };
            let m = &o.measurements;
            let rows: Vec<Value> = m
                .sorted_series()
                .into_iter()
                .filter(|&i| keep(&m.series[i].mp, m.series[i].metric))
                .map(|i| {
                    let r = &m.rows[i];
                    let picked = downsample(r, max);
                    json!({
                        "measuring_point": &*m.series[i].mp,
                        "metric": m.series[i].metric,
                        "rows_total": r.len(),
                        "rows_returned": picked.len(),
                        "downsampled": picked.len() < r.len(),
                        "columns": ["time", "value"],
                        "rows": picked.iter().map(|&(t, v)| json!([t, v])).collect::<Vec<_>>(),
                    })
                })
                .collect();
            out["rows"] = Value::Array(rows);
        }
        Ok(out)
    }

    /// The measurements as `measurements.csv` (the format of `simoxide run --measurements`),
    /// optionally only the selected series.
    pub fn csv(&self, id: &str, series: &[SeriesSelector]) -> Result<String, ApiError> {
        let job = self.jobs.get(id)?;
        let Some(o) = job.outcome() else {
            return Err(not_ready(&job.view(0, false)));
        };
        if series.is_empty() {
            return Ok(o.measurements.to_csv());
        }
        let m = &o.measurements;
        let mut f = Measurements::default();
        for (i, s) in m.series.iter().enumerate() {
            if series.iter().any(|x| x.matches(&s.mp, s.metric)) {
                f.series.push(s.clone());
                f.rows.push(m.rows[i].clone());
                f.count += m.rows[i].len() as u64;
            }
        }
        Ok(f.to_csv())
    }
}

impl Service {
    /// The runtime metadata of an ended simulation with `config.runtime_metadata`: `{"model":
    /// <model file>, "runtime": <runtime file>}` (`docs/guide/runtime-metadata.md`).
    pub fn runtime_metadata(&self, id: &str) -> Result<Value, ApiError> {
        let job = self.jobs.get(id)?;
        let Some(o) = job.outcome() else {
            return Err(not_ready(&job.view(0, false)));
        };
        o.runtime_metadata.clone().ok_or_else(|| {
            ApiError::conflict(format!(
                "simulation {id} ran without config.runtime_metadata: start it with \"runtime_metadata\": true in its config"
            ))
        })
    }
}

/// The error of asking for results a simulation does not have (yet).
fn not_ready(v: &JobView) -> ApiError {
    if v.status.is_terminal() {
        ApiError::conflict(format!(
            "simulation {} {} without results: {}",
            v.id,
            v.status.as_str(),
            v.error.clone().unwrap_or_default()
        ))
    } else {
        ApiError::conflict(format!(
            "simulation {} has not ended yet (status {}): read its live progress with the status call, wait for it, or cancel it to get the results so far",
            v.id,
            v.status.as_str()
        ))
    }
}

/// At most `max` rows, evenly spaced, the first and the last included.
pub fn downsample(rows: &[(f64, f64)], max: usize) -> Vec<(f64, f64)> {
    if rows.len() <= max {
        return rows.to_vec();
    }
    if max == 0 {
        return Vec::new();
    }
    if max == 1 {
        return vec![rows[rows.len() - 1]];
    }
    let n = rows.len() - 1;
    (0..max).map(|k| rows[k * n / (max - 1)]).collect()
}

/// Appends to `out` the directories with a usage model at or below `dir`; `depth`: how far
/// `dir` is below `root`.
fn scan(root: &Path, dir: &Path, depth: u32, out: &mut Vec<Value>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    entries.sort();
    let files: Vec<String> = entries
        .iter()
        .filter(|p| p.is_file())
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    if files.iter().any(|f| f.ends_with(USAGE_MODEL_SUFFIX)) {
        let rel = dir.strip_prefix(root).unwrap_or(dir);
        let name = if rel.as_os_str().is_empty() {
            ".".to_string()
        } else {
            rel.to_string_lossy().into_owned()
        };
        out.push(json!({
            "model_dir": name,
            "root": root.display().to_string(),
            "files": files,
            "has_run_json": files.iter().any(|f| f == RUN_JSON),
        }));
    }
    if depth >= MODEL_SCAN_MAX_DEPTH {
        return;
    }
    for p in entries.iter().filter(|p| p.is_dir()) {
        if p.file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        {
            continue;
        }
        scan(root, p, depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::downsample;

    #[test]
    fn downsample_keeps_ends() {
        let rows: Vec<(f64, f64)> = (0..100).map(|i| (i as f64, 0.0)).collect();
        let d = downsample(&rows, 10);
        assert_eq!(d.len(), 10);
        assert_eq!(d[0].0, 0.0);
        assert_eq!(d[9].0, 99.0);
        assert_eq!(downsample(&rows, 1000).len(), 100);
    }
}

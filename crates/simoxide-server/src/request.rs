//! What a client sends to start a simulation ([`SimulationRequest`]): the Palladio model, the
//! run configuration, the termination conditions and how often progress is published.
//!
//! The JSON form is documented in `docs/guide/server.md`; every struct here derives `serde`'s
//! `Deserialize` with `deny_unknown_fields`, so a misspelt field is an error, not a silent
//! default.

use crate::stats::MIN_BATCHES_FOR_INTERVAL;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// Name of the optional run configuration among the model files.
pub const RUN_JSON: &str = "run.json";
/// Suffix of a model's usage-model file.
pub const USAGE_MODEL_SUFFIX: &str = ".usagemodel";
/// Default of [`ProgressOptions::interval_ms`].
pub const DEFAULT_PROGRESS_INTERVAL_MS: u64 = 500;
/// The shortest [`ProgressOptions::interval_ms`] that is honoured.
pub const MIN_PROGRESS_INTERVAL_MS: u64 = 20;
/// The longest [`ProgressOptions::interval_ms`] a request may give (one day); longer intervals
/// would overflow the clock arithmetic.
pub const MAX_PROGRESS_INTERVAL_MS: u64 = 86_400_000;
/// Default of [`Confidence::level`].
pub const DEFAULT_CONFIDENCE_LEVEL: f64 = 0.95;
/// The target of [`Confidence::relative_target`] when a criterion gives no target.
pub const DEFAULT_RELATIVE_HALF_WIDTH: f64 = 0.05;
/// Default of [`Confidence::min_batches`].
pub const DEFAULT_MIN_BATCHES: usize = 30;
/// Default of [`Confidence::min_samples`].
pub const DEFAULT_MIN_SAMPLES: u64 = 100;
/// The metrics a [`Confidence`] without series selectors watches contain this.
const RESPONSE_TIME_METRIC: &str = "Response Time";

/// A request to run one simulation.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationRequest {
    /// Free-form label shown in listings (default: the model's usage model file name).
    #[serde(default)]
    pub name: Option<String>,
    /// The model files (XMI and the optional side files), sent with the request. Either `files`
    /// or `model_dir` is required.
    #[serde(default)]
    pub files: Vec<ModelFile>,
    /// A model directory on the server, relative to one of the directories the server was
    /// started with (`--allow-dir`). Not available without them.
    #[serde(default)]
    pub model_dir: Option<String>,
    /// The allowed directory `model_dir` is below, as the model listing reports it (its
    /// `root`). Needed when several allowed directories have a model of that name.
    #[serde(default)]
    pub root: Option<String>,
    /// Run configuration (seed, flavour, mode, the simulator's own stop conditions, ...).
    #[serde(default)]
    pub config: RunOptions,
    /// When the server stops the run (confidence interval, timeout, counts). Checked between
    /// two events, in addition to the simulator's own stop conditions in `config`.
    #[serde(default)]
    pub termination: Condition,
    /// Live progress options.
    #[serde(default)]
    pub progress: ProgressOptions,
}

/// One file of a model, by name (relative to the model directory, as `href`s in the other
/// files refer to it) and content.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFile {
    /// File name, e.g. `ms.repository`, `refsim.measuringpoint`, `run.json`. Relative, no `..`.
    pub name: String,
    /// The content as text (XMI, JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// The content base64-encoded (instead of `content`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_base64: Option<String>,
}

impl ModelFile {
    /// The file's bytes; errors on a bad name or a missing or doubled content.
    pub fn bytes(&self) -> Result<Vec<u8>, String> {
        check_name(&self.name)?;
        match (&self.content, &self.content_base64) {
            (Some(c), None) => Ok(c.as_bytes().to_vec()),
            (None, Some(b)) => base64::engine::general_purpose::STANDARD
                .decode(b.trim())
                .map_err(|e| format!("file {}: content_base64: {e}", self.name)),
            (Some(_), Some(_)) => Err(format!(
                "file {}: give either content or content_base64, not both",
                self.name
            )),
            (None, None) => Err(format!(
                "file {}: content or content_base64 is required",
                self.name
            )),
        }
    }
}

/// A relative file name without `..`, absolute or empty components.
pub fn check_name(name: &str) -> Result<(), String> {
    let bad = name.is_empty()
        || name.starts_with('/')
        || name.starts_with('\\')
        || name.contains(':')
        || name
            .split(['/', '\\'])
            .any(|c| c.is_empty() || c == "." || c == "..");
    if bad {
        return Err(format!(
            "file name {name:?}: expected a relative path without '.', '..' or empty components"
        ));
    }
    Ok(())
}

/// The run configuration: the fields of `run.json` and the `simoxide run` options that make
/// sense for a service. A `run.json` among the model files is read first; these fields override
/// it.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunOptions {
    /// Seed of the random numbers (default: run.json's, else 0).
    #[serde(default)]
    pub seed: Option<i64>,
    /// `simulizar` (default), `slingshot` or `slingshot-exact`.
    #[serde(default)]
    pub flavour: Option<String>,
    /// `fast` (default) or `exact` (needs a server built with the feature `exact`).
    #[serde(default)]
    pub mode: Option<String>,
    /// Slingshot compat flags (`--quirks`): `none` (default), `all`, `Q1,Q5,...`.
    #[serde(default)]
    pub quirks: Option<String>,
    /// The simulator's own stop: simulated time in whole seconds (`-1` = off), as `run.json`'s
    /// `max_sim_time`. Required by `slingshot-exact`.
    #[serde(default)]
    pub max_sim_time: Option<i64>,
    /// The simulator's own stop: finished usage-scenario runs (SimuLizar's "maximum
    /// measurements count"; `-1` = off), as `run.json`'s `max_measurements`.
    #[serde(default)]
    pub max_measurements: Option<i64>,
    /// Entry file of the usage model (default: the single `.usagemodel`).
    #[serde(default)]
    pub usagemodel: Option<String>,
    /// Entry files of the allocation (default: every `.allocation`).
    #[serde(default)]
    pub allocation: Option<Vec<String>>,
    /// Entry file of the monitors (default: every `.monitorrepository`).
    #[serde(default)]
    pub monitorrepository: Option<String>,
    /// Simulate linking resources (as `run.json`).
    #[serde(default)]
    pub simulate_linking_resources: Option<bool>,
    /// Simulate the throughput of linking resources (as `run.json`).
    #[serde(default)]
    pub simulate_throughput_of_linking_resources: Option<bool>,
    /// `exact` (default) or `virtual-time` processor sharing.
    #[serde(default)]
    pub ps_algorithm: Option<String>,
    /// Fast mode: common random numbers across designs (RND-11).
    #[serde(default)]
    pub usage_streams: bool,
    /// Fix SimuLizar's double resume (REF-2).
    #[serde(default)]
    pub fix_double_resume: bool,
    /// Run the model's scaling policies (`.spd`) if it has any (default true).
    #[serde(default)]
    pub spd: Option<bool>,
    /// SPD compat flags: `fixed` (default; `slingshot` for slingshot-exact), `slingshot`, `Q2,...`.
    #[serde(default)]
    pub spd_quirks: Option<String>,
    /// Also log the SPD evaluations that requested no adjustment (one per watched
    /// measurement: many), at level `debug`.
    #[serde(default)]
    pub spd_log_all: bool,
    /// Scaling decisions (`simoxide run --pause-on-decision`, `resume --decide`): the run stops
    /// before each scaling policy's adjustment, and decision `i` (from 0) is resolved with the
    /// `i`-th action list, the policies (ids or names) whose adjustments run in its place (`[]`
    /// declines). At the first decision beyond the lists the run ends with the stop reason
    /// `decision` and the decision pending. Off (absent) by default.
    #[serde(default)]
    pub spd_decisions: Option<Vec<Vec<String>>>,
    /// Run the HPA autoscalers of `*.hpa` / `*.hpa.json` files (default true).
    #[serde(default)]
    pub hpa: Option<bool>,
    /// Apply `*.loadprofile.json` files (default true).
    #[serde(default)]
    pub load_profile: Option<bool>,
    /// Livelock guard (default 20 000 000, 0 = off).
    #[serde(default)]
    pub max_events_per_instant: Option<u64>,
    /// Resource limit on the interpreter's stack depth (see `simoxide_sim::Limits`; 0 = off).
    #[serde(default)]
    pub max_stack_depth: Option<u32>,
    /// Resource limit on the live processes (see `simoxide_sim::Limits`; 0 = off).
    #[serde(default)]
    pub max_processes: Option<u32>,
    /// Resource limit on the interpreter's steps (see `simoxide_sim::Limits`; 0 = off).
    #[serde(default)]
    pub max_steps: Option<u64>,
}

/// Live progress options.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProgressOptions {
    /// Wall-clock interval between two progress updates in milliseconds (default
    /// [`DEFAULT_PROGRESS_INTERVAL_MS`], at least [`MIN_PROGRESS_INTERVAL_MS`]). Termination
    /// conditions are checked at least this often.
    pub interval_ms: u64,
    /// Rows to skip at the start of every series in the statistics and in the confidence
    /// condition: rows before this simulation time (s).
    pub warmup_time: f64,
    /// ... and the first rows of each series.
    pub warmup_samples: u64,
}

impl Default for ProgressOptions {
    fn default() -> Self {
        ProgressOptions {
            interval_ms: DEFAULT_PROGRESS_INTERVAL_MS,
            warmup_time: 0.0,
            warmup_samples: 0,
        }
    }
}

/// How the fields of a [`Condition`] combine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Combine {
    /// The condition holds when any of its fields holds (default).
    #[default]
    Any,
    /// The condition holds when all of its fields hold.
    All,
}

/// A termination condition, checked between two events. Each field is one criterion; the
/// criteria of one object combine as `combine` says (default: any). `all` and `any` nest
/// groups, each counting as one criterion. An object without criteria never holds.
///
/// ```json
/// {"timeout_secs": 60,
///  "all": [{"confidence": {"relative_half_width": 0.05}}, {"max_measurements": 1000}]}
/// ```
/// stops after one minute, or once the confidence interval is narrow enough and at least 1000
/// usage-scenario runs finished.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    /// Wall-clock time since the run started (s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<f64>,
    /// Finished usage-scenario runs (what SimuLizar calls measurements).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_measurements: Option<i64>,
    /// Recorded measurement tuples, all series together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tuples: Option<u64>,
    /// Processed events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_events: Option<u64>,
    /// Simulated time (s): holds before the first event later than this time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_sim_time: Option<f64>,
    /// The confidence interval of the mean of the selected series is narrow enough.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    /// Holds when every condition of the list holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<Vec<Condition>>,
    /// Holds when any condition of the list holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub any: Option<Vec<Condition>>,
    /// How this object's criteria combine (default `any`).
    #[serde(default)]
    pub combine: Combine,
}

impl Condition {
    /// Whether the object has no criterion at all.
    pub fn is_empty(&self) -> bool {
        self.timeout_secs.is_none()
            && self.max_measurements.is_none()
            && self.max_tuples.is_none()
            && self.max_events.is_none()
            && self.max_sim_time.is_none()
            && self.confidence.is_none()
            && self.all.is_none()
            && self.any.is_none()
    }

    /// Every confidence criterion of the tree.
    pub fn confidences(&self) -> Vec<&Confidence> {
        let mut v = Vec::new();
        self.visit(&mut |c| {
            if let Some(x) = &c.confidence {
                v.push(x);
            }
        });
        v
    }

    /// Visits every node of the tree: a node, then its `all` lists, then its `any` lists.
    pub(crate) fn visit<'a>(&'a self, f: &mut dyn FnMut(&'a Condition)) {
        f(self);
        for c in self.all.iter().chain(self.any.iter()).flatten() {
            c.visit(f);
        }
    }

    /// Whether the tree can ever hold: a criterion that bounds the run (anything but a
    /// confidence interval alone, which may never be reached).
    pub fn bounds_the_run(&self) -> bool {
        let own = self.timeout_secs.is_some()
            || self.max_measurements.is_some()
            || self.max_tuples.is_some()
            || self.max_events.is_some()
            || self.max_sim_time.is_some();
        let all_bounded = |l: &Option<Vec<Condition>>| {
            l.as_ref()
                .is_some_and(|l| !l.is_empty() && l.iter().all(Condition::bounds_the_run))
        };
        let any_bounded = |l: &Option<Vec<Condition>>| {
            l.as_ref()
                .is_some_and(|l| l.iter().any(Condition::bounds_the_run))
        };
        match self.combine {
            // any criterion that bounds the run bounds it
            Combine::Any => own || any_bounded(&self.any) || all_bounded(&self.all),
            // all must hold: bounded only if every criterion is (a confidence one is not)
            Combine::All => {
                !self.is_empty()
                    && self.confidence.is_none()
                    && (self.all.is_none() || all_bounded(&self.all))
                    && (self.any.is_none() || any_bounded(&self.any))
            }
        }
    }

    /// Checks the values (non-negative, levels in (0, 1), non-empty lists, ...).
    pub fn validate(&self) -> Result<(), String> {
        if let Some(m) = self.max_measurements
            && m < 0
        {
            return Err("termination.max_measurements: expected a non-negative count (unlike config.max_measurements and run.json, -1 does not mean off here: leave the criterion out)".into());
        }
        for (name, list) in [("all", &self.all), ("any", &self.any)] {
            if list.as_ref().is_some_and(Vec::is_empty) {
                return Err(format!(
                    "termination.{name}: expected at least one condition (an empty list never holds)"
                ));
            }
        }
        if let Some(t) = self.timeout_secs
            && !(t.is_finite() && t > 0.0)
        {
            return Err("termination.timeout_secs: expected a positive number".into());
        }
        if let Some(t) = self.max_sim_time
            && !(t.is_finite() && t >= 0.0)
        {
            return Err("termination.max_sim_time: expected a non-negative number".into());
        }
        if let Some(c) = &self.confidence {
            c.validate()?;
        }
        for c in self.all.iter().chain(self.any.iter()).flatten() {
            c.validate()?;
        }
        Ok(())
    }
}

/// A confidence-interval criterion: the batch-means confidence interval of the mean of every
/// selected series is narrower than the target.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Confidence {
    /// Confidence level (default [`DEFAULT_CONFIDENCE_LEVEL`]).
    pub level: f64,
    /// Target half width relative to the mean (e.g. 0.05 = ±5 %).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relative_half_width: Option<f64>,
    /// Target half width in the series' unit (e.g. seconds). With both targets, both must hold.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub absolute_half_width: Option<f64>,
    /// The series to watch (default: every response-time series). A selector matches a series
    /// when its fields are substrings of the series' measuring point and metric.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<SeriesSelector>,
    /// Values per batch (SimuCom's static batch means). Default: a fixed number of batches
    /// (`min_batches` to `2 * min_batches`) whose size doubles as the series grows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<u64>,
    /// Batches needed before the interval is computed (default [`DEFAULT_MIN_BATCHES`]).
    pub min_batches: usize,
    /// Values (after the warm-up) every selected series needs before the criterion can hold
    /// (default [`DEFAULT_MIN_SAMPLES`]).
    pub min_samples: u64,
}

impl Default for Confidence {
    fn default() -> Self {
        Confidence {
            level: DEFAULT_CONFIDENCE_LEVEL,
            relative_half_width: None,
            absolute_half_width: None,
            series: Vec::new(),
            batch_size: None,
            min_batches: DEFAULT_MIN_BATCHES,
            min_samples: DEFAULT_MIN_SAMPLES,
        }
    }
}

impl Confidence {
    /// Checks the values (level in (0, 1), positive targets, enough batches).
    pub fn validate(&self) -> Result<(), String> {
        if !(self.level > 0.0 && self.level < 1.0) {
            return Err("termination.confidence.level: expected a value in (0, 1)".into());
        }
        for (n, v) in [
            ("relative_half_width", self.relative_half_width),
            ("absolute_half_width", self.absolute_half_width),
        ] {
            if let Some(v) = v
                && !(v.is_finite() && v > 0.0)
            {
                return Err(format!(
                    "termination.confidence.{n}: expected a positive number"
                ));
            }
        }
        if self.min_batches < MIN_BATCHES_FOR_INTERVAL {
            return Err(format!(
                "termination.confidence.min_batches: at least {MIN_BATCHES_FOR_INTERVAL}"
            ));
        }
        if self.batch_size == Some(0) {
            return Err("termination.confidence.batch_size: at least 1".into());
        }
        Ok(())
    }

    /// The target relative half width: as given, or [`DEFAULT_RELATIVE_HALF_WIDTH`] when neither
    /// target is given.
    pub fn relative_target(&self) -> Option<f64> {
        match (self.relative_half_width, self.absolute_half_width) {
            (None, None) => Some(DEFAULT_RELATIVE_HALF_WIDTH),
            (r, _) => r,
        }
    }

    /// Whether an interval meets the targets.
    pub fn met_by(&self, ci: &crate::stats::Interval) -> bool {
        self.relative_target()
            .is_none_or(|r| ci.relative_half_width <= r)
            && self.absolute_half_width.is_none_or(|a| ci.half_width <= a)
    }

    /// Whether `(measuring_point, metric)` is watched.
    pub fn selects(&self, mp: &str, metric: &str) -> bool {
        if self.series.is_empty() {
            return metric.contains(RESPONSE_TIME_METRIC);
        }
        self.series.iter().any(|s| s.matches(mp, metric))
    }

    /// The batching of this criterion.
    pub fn batching(&self) -> crate::stats::Batching {
        match self.batch_size {
            Some(size) => crate::stats::Batching::Static {
                size,
                min_batches: self.min_batches,
            },
            None => crate::stats::Batching::Doubling {
                min: self.min_batches,
            },
        }
    }
}

/// Selects series by substrings of their measuring point and metric (`measurements.csv`'s
/// first two columns).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SeriesSelector {
    /// Substring of the measuring point (e.g. a usage scenario or operation name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measuring_point: Option<String>,
    /// Substring of the metric (e.g. `Response Time`, `Utilization`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
}

impl SeriesSelector {
    /// Whether the given fields are substrings of the measuring point and the metric.
    pub fn matches(&self, mp: &str, metric: &str) -> bool {
        self.measuring_point
            .as_deref()
            .is_none_or(|s| mp.contains(s))
            && self.metric.as_deref().is_none_or(|s| metric.contains(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(check_name("ms.repository").is_ok());
        assert!(check_name("sub/ms.repository").is_ok());
        for bad in ["", "/etc/passwd", "../x", "a/../b", "a//b", "C:x", "./a"] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn unknown_fields_are_errors() {
        let r: Result<SimulationRequest, _> =
            serde_json::from_str(r#"{"files": [], "termination": {"timeout": 3}}"#);
        assert!(r.is_err());
    }

    #[test]
    fn bounded() {
        let c: Condition = serde_json::from_str(r#"{"confidence": {}}"#).unwrap();
        assert!(!c.bounds_the_run());
        let c: Condition =
            serde_json::from_str(r#"{"confidence": {}, "timeout_secs": 3}"#).unwrap();
        assert!(c.bounds_the_run());
        let c: Condition =
            serde_json::from_str(r#"{"combine": "all", "confidence": {}, "max_measurements": 3}"#)
                .unwrap();
        assert!(!c.bounds_the_run());
        let c: Condition =
            serde_json::from_str(r#"{"all": [{"max_tuples": 4}, {"max_events": 9}]}"#).unwrap();
        assert!(c.bounds_the_run());
        // an empty list never holds, so it does not bound the run either
        let c: Condition =
            serde_json::from_str(r#"{"combine": "all", "max_events": 9, "all": []}"#).unwrap();
        assert!(!c.bounds_the_run());
    }

    #[test]
    fn validation() {
        for (bad, why) in [
            (r#"{"all": []}"#, "empty all"),
            (r#"{"any": [{"any": []}]}"#, "nested empty any"),
            (r#"{"max_measurements": -1}"#, "negative count"),
        ] {
            let c: Condition = serde_json::from_str(bad).unwrap();
            assert!(c.validate().is_err(), "{why}");
        }
        let c: Condition = serde_json::from_str(r#"{"max_measurements": 0}"#).unwrap();
        assert!(c.validate().is_ok());
    }
}

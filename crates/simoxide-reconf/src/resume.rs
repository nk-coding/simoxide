//! Change sets applied between a pause (or a restored snapshot) and the continuation
//! (`DESIGN-SPD.md` N2), and fork-based exploration.
//!
//! ```no_run
//! # use simoxide_reconf::*;
//! # use simoxide_sim::*;
//! # fn f(cm: &CompiledModel, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
//! let core = QvtoCore::new()?;
//! let mut sim = Simulation::<Fast>::restore_from(bytes, cm, Outputs::default())?;
//! let changes = ResumeChanges::new()
//!     .file(ChangeFile::load("faster-db.changes".as_ref())?)
//!     .transform(core.compile_file("scale.qvto".as_ref(), &Imports::Folder)?, Invocation::simulizar());
//! let mut qvto = QvtoRun::new(&core, RunConfig::default())?;
//! changes.apply(&mut sim, &mut qvto)?;
//! let result = sim.finish()?;
//! # Ok(()) }
//! ```
//!
//! A change applied after a restore equals the same change applied in the uninterrupted run at
//! the same point (the restore is exact, and the change path is deterministic), byte for byte.

use crate::edits::{ChangeFile, ChangeFileError};
use crate::host::{Invocation, QvtoRun, ReconfError, Transformation};
#[cfg(feature = "snapshot")]
use crate::host::{QvtoCore, RunConfig};
#[cfg(feature = "snapshot")]
use simoxide_sim::snapshot::Snapshot;
use simoxide_sim::{ChangeReport, Compat, Flavour, SeriesSummary, Simulation};
#[cfg(feature = "snapshot")]
use simoxide_sim::{CompiledModel, Outputs, Pause, Status};
use std::fmt;

/// One step of a [`ResumeChanges`].
#[derive(Clone, Debug)]
pub enum ChangeStep {
    /// A change file: direct edits, applied as one change set.
    File(ChangeFile),
    /// A QVTo transformation, run once on the run's model ([`QvtoRun::invoke`]).
    Transform(Transformation, Invocation),
}

/// Changes for a resumed run, applied in order.
#[derive(Clone, Debug, Default)]
pub struct ResumeChanges {
    pub steps: Vec<ChangeStep>,
}

/// What a step did.
#[derive(Clone, Debug)]
pub struct StepReport {
    /// `file` or the transformation's name.
    pub step: String,
    /// The change path's report (`None`: the step changed nothing).
    pub change: Option<ChangeReport>,
    /// The transformation's QVTo log.
    pub log: String,
}

/// Why a change step failed. The steps before it are applied; the failing one changed nothing
/// (a transformation that failed with [`crate::FailedRunChanges::Keep`] keeps its partial
/// changes, as SimuLizar does; the CLI uses `Discard`).
#[derive(Debug)]
pub enum ResumeError {
    File(ChangeFileError),
    Transform(ReconfError),
    /// The transformation did not complete (`assert fatal`, an exception): its name and log.
    NotApplied {
        transformation: String,
        log: String,
    },
}

impl fmt::Display for ResumeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResumeError::File(e) => write!(f, "change file: {e}"),
            ResumeError::Transform(e) => write!(f, "transformation: {e}"),
            ResumeError::NotApplied {
                transformation,
                log,
            } => write!(
                f,
                "transformation {transformation} did not complete (not applied){}{}",
                if log.is_empty() { "" } else { ":\n" },
                log.trim_end()
            ),
        }
    }
}

impl std::error::Error for ResumeError {}

impl ResumeChanges {
    pub fn new() -> ResumeChanges {
        ResumeChanges::default()
    }

    pub fn file(mut self, f: ChangeFile) -> Self {
        self.steps.push(ChangeStep::File(f));
        self
    }

    pub fn transform(mut self, t: Transformation, inv: Invocation) -> Self {
        self.steps.push(ChangeStep::Transform(t, inv));
        self
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Applies the steps to `sim` (paused, restored, or not started); transformations run in
    /// `qvto`, the run's QVTo state (for a run with rules, its host's: [`crate::RulesHost::qvto`]).
    pub fn apply<C: Compat, F: Flavour>(
        &self,
        sim: &mut Simulation<'_, C, F>,
        qvto: &mut QvtoRun,
    ) -> Result<Vec<StepReport>, ResumeError> {
        let mut out = Vec::new();
        for s in &self.steps {
            out.push(match s {
                ChangeStep::File(f) => {
                    let r = f.apply(sim).map_err(ResumeError::File)?;
                    StepReport {
                        step: "file".into(),
                        change: Some(r),
                        log: String::new(),
                    }
                }
                ChangeStep::Transform(t, inv) => {
                    let o = qvto.invoke(sim, t, inv).map_err(ResumeError::Transform)?;
                    if !o.succeeded {
                        return Err(ResumeError::NotApplied {
                            transformation: o.transformation,
                            log: o.log,
                        });
                    }
                    StepReport {
                        step: o.transformation,
                        change: o.change,
                        log: o.log,
                    }
                }
            });
        }
        Ok(out)
    }
}

/// One variant of an exploration: a name and the changes applied at the fork point.
#[derive(Clone, Debug)]
pub struct Variant {
    pub name: String,
    pub changes: ResumeChanges,
}

/// The end of one variant's run.
#[derive(Clone, Debug)]
pub struct VariantResult {
    pub name: String,
    /// `Err`: the changes or the run failed (message).
    pub outcome: Result<VariantRun, String>,
}

#[derive(Clone, Debug)]
pub struct VariantRun {
    pub end_ns: i64,
    pub events: u64,
    pub main_count: i64,
    /// Measurements of the continuation (from the fork point on), per series.
    pub summaries: Vec<SeriesSummary>,
    /// What the changes did.
    pub steps: Vec<StepReport>,
}

/// Fork-based exploration (feature `snapshot`): restores `snapshot` once per variant (on `threads` threads; the
/// restores are independent), applies the variant's changes, runs it to `until` (or the end) and
/// summarises its measurements. Deterministic: the same snapshot and variants give the same
/// results, whatever the number of threads. Plain runs (no SPD or rule sections) of the
/// SimuLizar or Slingshot flavour in mode `C`; transformations run in a fresh QVTo state per
/// variant (with the snapshot's `qvto` section, if any).
#[cfg(feature = "snapshot")]
pub fn explore<C: Compat, F: Flavour>(
    base: &CompiledModel,
    snapshot: &Snapshot,
    core: &QvtoCore,
    variants: &[Variant],
    until: Option<Pause>,
    threads: usize,
) -> Vec<VariantResult> {
    let one = |v: &Variant| -> Result<VariantRun, String> {
        let mut sim = Simulation::<C, F>::restore_snapshot(snapshot, base, Outputs::default())
            .map_err(|e| e.to_string())?;
        let mut qvto = QvtoRun::new(
            core,
            RunConfig {
                failed_runs: crate::FailedRunChanges::Discard,
                ..RunConfig::default()
            },
        )
        .map_err(|e| e.to_string())?;
        qvto.restore_section(&sim, snapshot)
            .map_err(|e| e.to_string())?;
        // only the continuation's measurements
        let skip: Vec<usize> = sim
            .state()
            .measurements()
            .rows
            .iter()
            .map(Vec::len)
            .collect();
        let steps = v
            .changes
            .apply(&mut sim, &mut qvto)
            .map_err(|e| e.to_string())?;
        let status = match until {
            Some(p) => sim.run_until(p).map_err(|e| e.to_string())?,
            None => Status::Finished,
        };
        let (mut m, end_ns, events, main_count) = if status == Status::Finished {
            let r = sim.finish().map_err(|e| e.to_string())?;
            (r.measurements, r.end_ns, r.events, r.main_count)
        } else {
            let st = sim.state();
            (
                st.measurements().clone(),
                st.now_ns(),
                st.events(),
                st.main_count(),
            )
        };
        for (rows, &k) in m.rows.iter_mut().zip(&skip) {
            rows.drain(..k.min(rows.len()));
        }
        Ok(VariantRun {
            end_ns,
            events,
            main_count,
            summaries: m.summaries(),
            steps,
        })
    };
    let threads = threads.clamp(1, variants.len().max(1));
    let mut results: Vec<Option<VariantResult>> = vec![None; variants.len()];
    let next = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        let handles: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(v) = variants.get(i) else { break };
                        done.push((
                            i,
                            VariantResult {
                                name: v.name.clone(),
                                outcome: one(v),
                            },
                        ));
                    }
                    done
                })
            })
            .collect();
        for h in handles {
            for (i, r) in h.join().expect("variant thread") {
                results[i] = Some(r);
            }
        }
    });
    results.into_iter().map(|r| r.expect("run")).collect()
}

/// A comparison table of explored variants: per series of the given metrics (all if empty),
/// the mean of each variant (`-` without tuples), plus end time, events and finished usage
/// scenario runs.
pub fn compare(results: &[VariantResult], metrics: &[&str]) -> String {
    use std::collections::BTreeMap;
    use std::fmt::Write;
    let mut rows: BTreeMap<(String, String), Vec<Option<f64>>> = BTreeMap::new();
    for (i, r) in results.iter().enumerate() {
        let Ok(run) = &r.outcome else { continue };
        for s in &run.summaries {
            if !metrics.is_empty() && !metrics.contains(&s.metric) {
                continue;
            }
            let row = rows
                .entry((s.metric.to_string(), s.measuring_point.clone()))
                .or_insert_with(|| vec![None; results.len()]);
            // utilisation and state series: the time-weighted mean
            row[i] = Some(
                s.time_weighted_mean
                    .filter(|_| is_state(s.metric))
                    .unwrap_or(s.mean),
            );
        }
    }
    let mut out = String::new();
    let w = results
        .iter()
        .map(|r| r.name.len().max(12))
        .collect::<Vec<_>>();
    let _ = write!(out, "{:<60}", "series (mean; state series: time-weighted)");
    for (r, w) in results.iter().zip(&w) {
        let _ = write!(out, " {:>w$}", r.name, w = w);
    }
    out.push('\n');
    let line = |out: &mut String, label: &str, f: &dyn Fn(&VariantRun) -> String| {
        let _ = write!(out, "{label:<60}");
        for (r, w) in results.iter().zip(&w) {
            let v = match &r.outcome {
                Ok(run) => f(run),
                Err(_) => "failed".into(),
            };
            let _ = write!(out, " {v:>w$}", w = w);
        }
        out.push('\n');
    };
    line(&mut out, "end time (s)", &|r| {
        format!("{:.3}", r.end_ns as f64 / 1e9)
    });
    line(&mut out, "events", &|r| r.events.to_string());
    line(&mut out, "finished scenario runs", &|r| {
        r.main_count.to_string()
    });
    for ((metric, mp), vals) in &rows {
        let mut label = format!("{} {}", short_metric(metric), mp);
        if label.len() > 59 {
            label.truncate(56);
            label.push_str("...");
        }
        let _ = write!(out, "{label:<60}");
        for (v, w) in vals.iter().zip(&w) {
            match v {
                Some(x) => {
                    let _ = write!(out, " {:>w$.6}", x, w = w);
                }
                None => {
                    let _ = write!(out, " {:>w$}", "-", w = w);
                }
            }
        }
        out.push('\n');
    }
    for r in results {
        if let Err(e) = &r.outcome {
            let _ = writeln!(out, "{}: {e}", r.name);
        }
    }
    out
}

fn is_state(metric: &str) -> bool {
    metric.starts_with("State of") || metric.starts_with("Utilization")
}

fn short_metric(m: &str) -> &str {
    use simoxide_sim::ir::metric_names as n;
    match m {
        n::RESPONSE_TIME => "rt",
        n::UTILIZATION => "util",
        n::STATE_OF_ACTIVE_RESOURCE => "state",
        n::RESOURCE_DEMAND => "demand",
        n::WAITING_TIME => "waiting",
        n::HOLDING_TIME => "holding",
        n::STATE_OF_PASSIVE_RESOURCE => "passive",
        o => o,
    }
}

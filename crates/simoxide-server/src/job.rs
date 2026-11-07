//! Simulation jobs and the [`JobManager`] that runs them.
//!
//! A job is one submitted simulation. It runs on a thread of its own (the simulator is
//! CPU-bound and its run state is not `Send`); the HTTP and MCP layers only read the job's
//! shared state ([`Job::view`], [`Job::logs`], [`Job::outcome`]) and wait for it to change
//! ([`Job::wait`], asynchronously, so that a waiting client holds no thread). Every change
//! bumps the job's `version`, so a client can long-poll for "any change after version N".
//!
//! The manager bounds how many jobs simulate at once (`max_concurrent`), how many wait for a
//! free slot (`max_queued`; a waiting job is data, not a thread, and a submit beyond the limit
//! is refused), how many it keeps (`max_jobs`; the oldest finished ones are dropped), and how
//! long a run may take (`max_run_secs`).

use crate::report::{
    JobStatus, Level, LogEntry, LogSource, Progress, ResultSummary, StopKind, StopReason,
};
use crate::request::{SimulationRequest, USAGE_MODEL_SUFFIX};
use axum::http::StatusCode;
use serde::Serialize;
use simoxide_sim::Measurements;
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

/// Bytes per mebibyte.
pub const MIB: usize = 1 << 20;
/// Default of [`ServerConfig::max_jobs`].
pub const DEFAULT_MAX_JOBS: usize = 100;
/// Default of [`ServerConfig::max_queued`] (with 64 MiB requests at most 4 GiB of waiting
/// models).
pub const DEFAULT_MAX_QUEUED: usize = 64;
/// Default of [`ServerConfig::max_run_secs`].
pub const DEFAULT_MAX_RUN_SECS: f64 = 3600.0;
/// Default of [`ServerConfig::max_tuples`] (16 bytes each: about 320 MB).
pub const DEFAULT_MAX_TUPLES: u64 = 20_000_000;
/// Default of [`ServerConfig::log_capacity`].
pub const DEFAULT_LOG_CAPACITY: usize = 5000;
/// Default of [`ServerConfig::stack_size`].
pub const DEFAULT_STACK_SIZE: usize = 64 * MIB;
/// A job's log keeps at least this many entries, whatever the configuration says.
const MIN_LOG_CAPACITY: usize = 10;
/// A cancelled run that has not stopped between two events after this long is aborted where it
/// is (the hard cancellation, which loses the counters of the event in progress). The server's
/// run-time cap gives the soft stop the same grace.
pub const HARD_STOP_GRACE: Duration = Duration::from_secs(10);
/// Stack size of the thread that escalates a cancellation (it only sleeps and sets a flag).
const ESCALATION_STACK_SIZE: usize = 64 * 1024;
/// Bits of the wall clock in a job id's suffix (it keeps ids of restarted servers apart).
const JOB_ID_SUFFIX_MASK: u64 = 0xff_ffff;
/// The job number is shifted by this before it is mixed into the suffix.
const JOB_ID_COUNTER_SHIFT: u32 = 20;

/// Locks `mutex`, also if a panic poisoned it: every update of the data it guards is one
/// assignment, so a panicking holder leaves consistent data.
pub(crate) fn lock_ignoring_poison<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Server-wide settings.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Directories whose subdirectories a request may name as `model_dir` (none: only models
    /// sent with the request).
    pub allow_dirs: Vec<PathBuf>,
    /// Simulations that run at the same time (default: the available parallelism).
    pub max_concurrent: usize,
    /// Jobs kept in memory; beyond it the oldest finished jobs are dropped.
    pub max_jobs: usize,
    /// Jobs that wait for a free slot; a submit beyond it is refused (`429`). A waiting job
    /// holds its request, with the model, in memory.
    pub max_queued: usize,
    /// Wall-clock cap of one run in seconds (`None`: none).
    pub max_run_secs: Option<f64>,
    /// Cap on the recorded tuples of one run (memory: 16 bytes each).
    pub max_tuples: Option<u64>,
    /// Log entries kept per job (a ring buffer).
    pub log_capacity: usize,
    /// Stack size of the simulation threads in bytes.
    pub stack_size: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            allow_dirs: Vec::new(),
            max_concurrent: std::thread::available_parallelism().map_or(1, |n| n.get()),
            max_jobs: DEFAULT_MAX_JOBS,
            max_queued: DEFAULT_MAX_QUEUED,
            max_run_secs: Some(DEFAULT_MAX_RUN_SECS),
            max_tuples: Some(DEFAULT_MAX_TUPLES),
            log_capacity: DEFAULT_LOG_CAPACITY,
            stack_size: DEFAULT_STACK_SIZE,
        }
    }
}

/// An error of the API: an HTTP status and a message.
#[derive(Clone, Debug, PartialEq)]
pub struct ApiError {
    /// The HTTP status of the error.
    pub status: StatusCode,
    /// What went wrong, in words.
    pub message: String,
}

impl ApiError {
    /// `400`: the request is malformed.
    pub fn bad_request(m: impl Into<String>) -> ApiError {
        ApiError {
            status: StatusCode::BAD_REQUEST,
            message: m.into(),
        }
    }
    /// `404`: no simulation has this id.
    pub fn not_found(id: &str) -> ApiError {
        ApiError {
            status: StatusCode::NOT_FOUND,
            message: format!("no simulation {id:?}"),
        }
    }
    /// `409`: the simulation is not in a state that allows this.
    pub fn conflict(m: impl Into<String>) -> ApiError {
        ApiError {
            status: StatusCode::CONFLICT,
            message: m.into(),
        }
    }
    /// `429`: the server runs and queues as many simulations as it may; try again later.
    pub fn too_many_requests(m: impl Into<String>) -> ApiError {
        ApiError {
            status: StatusCode::TOO_MANY_REQUESTS,
            message: m.into(),
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// What the run is: filled in once the model is loaded and compiled.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ModelInfo {
    /// `simulizar`, `slingshot` or `slingshot-exact`.
    pub flavour: String,
    /// `fast` or `exact`.
    pub mode: String,
    /// Seed of the random numbers.
    pub seed: i64,
    /// The simulator's own stop on simulated seconds (`-1` = off).
    pub max_sim_time: i64,
    /// The simulator's own stop on finished usage-scenario runs (`-1` = off).
    pub max_measurements: i64,
    /// The model files (names).
    pub files: Vec<String>,
    /// The measurement series the monitors define (measuring point, metric).
    pub series: Vec<SeriesName>,
    /// Scaling policies (`.spd`) that run with the simulation.
    pub scaling_policies: usize,
    /// HPA autoscalers that run with the simulation.
    pub autoscalers: usize,
    /// Whether SPD/HPA adjustments are executed (`reconfiguration` feature) or only decided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adjustments_executed: Option<bool>,
    /// A load profile is applied.
    pub load_profile: bool,
    /// Compile warnings (e.g. monitors that cannot be recorded).
    pub warnings: Vec<String>,
}

/// A measuring point and a metric.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct SeriesName {
    /// Measuring point as in `measurements.csv`.
    pub measuring_point: String,
    /// Metric as in `measurements.csv`.
    pub metric: String,
}

/// The final outcome of a job: the summary and the raw measurements.
#[derive(Debug)]
pub struct Outcome {
    /// Counters and per-series statistics.
    pub summary: ResultSummary,
    /// The recorded rows.
    pub measurements: Measurements,
}

/// The mutable part of a job.
struct JobState {
    status: JobStatus,
    /// Bumped by every change.
    version: u64,
    /// When the job started running (Unix ms).
    started: Option<u64>,
    /// When the job ended (Unix ms).
    finished: Option<u64>,
    error: Option<String>,
    model: Option<ModelInfo>,
    progress: Option<Arc<Progress>>,
    stop_reason: Option<StopReason>,
    outcome: Option<Arc<Outcome>>,
    /// The log's ring buffer.
    logs: VecDeque<LogEntry>,
    /// Sequence number of the next log entry.
    next_seq: u64,
}

/// What a waiting client watches: the job's version and whether the job has ended.
#[derive(Clone, Copy, Debug)]
struct Change {
    version: u64,
    ended: bool,
}

/// One simulation.
pub struct Job {
    /// The id clients address the job by.
    pub id: String,
    /// The label shown in listings.
    pub name: String,
    /// Submission time (Unix ms).
    pub submitted: u64,
    submitted_at: Instant,
    /// Soft cancellation: the run pauses at the next check and ends with the results so far.
    pub(crate) cancel: Arc<AtomicBool>,
    /// Hard cancellation (`Limits::cancel`): aborts a run that does not reach a pause. Set
    /// [`HARD_STOP_GRACE`] after the soft one.
    pub(crate) hard_cancel: Arc<AtomicBool>,
    log_capacity: usize,
    state: Mutex<JobState>,
    /// Wakes the waiting clients at every change.
    changes: watch::Sender<Change>,
}

/// A snapshot of a job for clients.
#[derive(Clone, Debug, Serialize)]
pub struct JobView {
    /// The job's id.
    pub id: String,
    /// The job's label.
    pub name: String,
    /// Where the job is.
    pub status: JobStatus,
    /// Bumped by every change (status, progress, log entry): wait for a version above it.
    pub version: u64,
    /// Submission time (Unix ms).
    pub submitted_at: u64,
    /// When the run started (Unix ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// When the job ended (Unix ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// Why the job failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Why the run ended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<StopReason>,
    /// Sequence number of the newest log entry (0: none).
    pub last_log_seq: u64,
    /// Results can be read (`completed`, `cancelled`, and `failed` runs that got far enough).
    pub results_available: bool,
    /// What the run is, once the model is compiled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelInfo>,
    /// The latest progress (statistics per series, termination status).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<Arc<Progress>>,
    /// The newest log entries (as many as asked for).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recent_logs: Vec<LogEntry>,
}

/// A page of log entries.
#[derive(Clone, Debug, Serialize)]
pub struct LogPage {
    /// The entries, oldest first.
    pub entries: Vec<LogEntry>,
    /// Sequence number of the newest entry (pass it as `since` to get only newer ones).
    pub last_seq: u64,
    /// Entries newer than `since` that were evicted from the ring buffer before this read.
    pub dropped: u64,
}

/// The wall clock in Unix milliseconds.
pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl Job {
    pub(crate) fn new(id: String, name: String, log_capacity: usize) -> Job {
        Job {
            id,
            name,
            submitted: unix_ms(),
            submitted_at: Instant::now(),
            cancel: Arc::new(AtomicBool::new(false)),
            hard_cancel: Arc::new(AtomicBool::new(false)),
            log_capacity: log_capacity.max(MIN_LOG_CAPACITY),
            state: Mutex::new(JobState {
                status: JobStatus::Queued,
                version: 1,
                started: None,
                finished: None,
                error: None,
                model: None,
                progress: None,
                stop_reason: None,
                outcome: None,
                logs: VecDeque::new(),
                next_seq: 1,
            }),
            changes: watch::Sender::new(Change {
                version: 1,
                ended: false,
            }),
        }
    }

    /// Marks a change: new version, waiting clients wake up.
    fn bump(&self, st: &mut JobState) {
        st.version += 1;
        self.changes.send_replace(Change {
            version: st.version,
            ended: st.status.is_terminal(),
        });
    }

    /// Appends a log entry.
    pub fn log(
        &self,
        level: Level,
        source: LogSource,
        sim_time: Option<f64>,
        message: impl Into<String>,
        data: Option<serde_json::Value>,
    ) {
        let mut st = lock_ignoring_poison(&self.state);
        self.log_locked(&mut st, level, source, sim_time, message.into(), data);
    }

    /// [`Job::log`] with the state locked.
    fn log_locked(
        &self,
        st: &mut JobState,
        level: Level,
        source: LogSource,
        sim_time: Option<f64>,
        message: String,
        data: Option<serde_json::Value>,
    ) {
        let e = LogEntry {
            seq: st.next_seq,
            wall_ms: self.submitted_at.elapsed().as_millis() as u64,
            sim_time,
            level,
            source,
            message,
            data,
        };
        st.next_seq += 1;
        if st.logs.len() >= self.log_capacity {
            st.logs.pop_front();
        }
        st.logs.push_back(e);
        self.bump(st);
    }

    /// Takes a queued job for its run (status `loading`); false if it was cancelled first.
    pub(crate) fn claim_for_run(&self) -> bool {
        let mut st = lock_ignoring_poison(&self.state);
        if st.status != JobStatus::Queued {
            return false;
        }
        st.status = JobStatus::Loading;
        self.bump(&mut st);
        true
    }

    /// A new status, unless the job has ended; the first `Running` stamps the start.
    pub(crate) fn set_status(&self, s: JobStatus) {
        let mut st = lock_ignoring_poison(&self.state);
        if st.status.is_terminal() {
            return;
        }
        st.status = s;
        if s == JobStatus::Running && st.started.is_none() {
            st.started = Some(unix_ms());
        }
        self.bump(&mut st);
    }

    /// Publishes what the run is.
    pub(crate) fn set_model(&self, m: ModelInfo) {
        let mut st = lock_ignoring_poison(&self.state);
        st.model = Some(m);
        self.bump(&mut st);
    }

    /// Publishes new progress.
    pub(crate) fn publish(&self, p: Progress) {
        let mut st = lock_ignoring_poison(&self.state);
        st.progress = Some(Arc::new(p));
        self.bump(&mut st);
    }

    /// Ends the job (the first call wins).
    pub(crate) fn end(
        &self,
        status: JobStatus,
        stop_reason: Option<StopReason>,
        error: Option<String>,
        outcome: Option<Outcome>,
    ) {
        let mut st = lock_ignoring_poison(&self.state);
        self.end_locked(&mut st, status, stop_reason, error, outcome);
    }

    /// [`Job::end`] with the state locked.
    fn end_locked(
        &self,
        st: &mut JobState,
        status: JobStatus,
        stop_reason: Option<StopReason>,
        error: Option<String>,
        outcome: Option<Outcome>,
    ) {
        if st.status.is_terminal() {
            return;
        }
        st.status = status;
        st.stop_reason = stop_reason;
        st.error = error;
        st.outcome = outcome.map(Arc::new);
        st.finished = Some(unix_ms());
        self.bump(st);
    }

    /// Asks the run to stop; it ends as `cancelled` with the results so far. A queued job ends
    /// at once. Returns false if the job had already ended.
    ///
    /// A running job stops at its next pause between two events (soft), which keeps the
    /// counters and warnings of a clean end; only if it has not stopped [`HARD_STOP_GRACE`]
    /// later is it aborted where it is (hard).
    pub fn request_cancel(&self) -> bool {
        let mut st = lock_ignoring_poison(&self.state);
        if st.status.is_terminal() {
            return false;
        }
        // `Release`: whoever sees a cancellation flag set sees this one set too
        let first = !self.cancel.swap(true, Ordering::AcqRel);
        if st.status == JobStatus::Queued {
            self.log_locked(
                &mut st,
                Level::Info,
                LogSource::Server,
                None,
                "cancelled while queued".into(),
                None,
            );
            let reason = StopReason {
                kind: StopKind::Cancelled,
                detail: "cancelled before it started".into(),
            };
            self.end_locked(&mut st, JobStatus::Cancelled, Some(reason), None, None);
            return true;
        }
        drop(st);
        if first {
            escalate_after_grace(self.hard_cancel.clone());
        }
        true
    }

    /// Whether a client asked the job to stop.
    pub fn cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    /// Where the job is now.
    pub fn status(&self) -> JobStatus {
        lock_ignoring_poison(&self.state).status
    }

    /// The job's version now (see [`JobView::version`]).
    pub fn version(&self) -> u64 {
        lock_ignoring_poison(&self.state).version
    }

    /// The current state, with the newest `recent_logs` log entries.
    pub fn view(&self, recent_logs: usize, with_progress: bool) -> JobView {
        let st = lock_ignoring_poison(&self.state);
        let skip = st.logs.len().saturating_sub(recent_logs);
        JobView {
            id: self.id.clone(),
            name: self.name.clone(),
            status: st.status,
            version: st.version,
            submitted_at: self.submitted,
            started_at: st.started,
            finished_at: st.finished,
            error: st.error.clone(),
            stop_reason: st.stop_reason.clone(),
            last_log_seq: st.next_seq - 1,
            results_available: st.outcome.is_some(),
            model: st.model.clone(),
            progress: if with_progress {
                st.progress.clone()
            } else {
                None
            },
            recent_logs: st.logs.iter().skip(skip).cloned().collect(),
        }
    }

    /// Log entries with `seq > since`, at most `limit`, at or above `min_level`.
    pub fn logs(&self, since: u64, limit: usize, min_level: Level) -> LogPage {
        let st = lock_ignoring_poison(&self.state);
        let first = st.logs.front().map_or(st.next_seq, |e| e.seq);
        let dropped = first.saturating_sub(since.saturating_add(1));
        let entries: Vec<LogEntry> = st
            .logs
            .iter()
            .filter(|e| e.seq > since && e.level >= min_level)
            .take(limit)
            .cloned()
            .collect();
        let last_seq = entries.last().map_or(st.next_seq - 1, |e| e.seq);
        LogPage {
            entries,
            last_seq,
            dropped,
        }
    }

    /// The final outcome, once there is one.
    pub fn outcome(&self) -> Option<Arc<Outcome>> {
        lock_ignoring_poison(&self.state).outcome.clone()
    }

    /// Waits until the job's version exceeds `after_version` (`until_done`: until the job has
    /// ended) or `timeout` passes; returns the version then. Needs a tokio runtime (its timer).
    pub async fn wait(&self, after_version: u64, until_done: bool, timeout: Duration) -> u64 {
        let mut changes = self.changes.subscribe();
        let wanted = |c: &Change| c.ended || (!until_done && c.version > after_version);
        // the end of the timeout is no error: the caller reads the state as it is then
        let _ = tokio::time::timeout(timeout, changes.wait_for(wanted)).await;
        self.version()
    }
}

/// Sets `hard_cancel` after [`HARD_STOP_GRACE`], on a thread of its own (at once if no thread
/// can be started). Setting it after the run has ended does nothing.
fn escalate_after_grace(hard_cancel: Arc<AtomicBool>) {
    let flag = hard_cancel.clone();
    let spawned = std::thread::Builder::new()
        .name("simoxide-cancel".into())
        .stack_size(ESCALATION_STACK_SIZE)
        .spawn(move || {
            std::thread::sleep(HARD_STOP_GRACE);
            flag.store(true, Ordering::Release);
        });
    if spawned.is_err() {
        hard_cancel.store(true, Ordering::Release);
    }
}

/// A submitted job that waits for a free slot, with its request.
struct Pending {
    job: Arc<Job>,
    request: SimulationRequest,
}

/// The jobs that wait for a slot, and how many run.
#[derive(Default)]
struct Queue {
    waiting: VecDeque<Pending>,
    running: usize,
}

/// Held by a job's thread while it runs: frees the slot on drop and starts the next waiting
/// job.
struct RunningSlot<'a>(&'a Arc<JobManager>);

impl Drop for RunningSlot<'_> {
    fn drop(&mut self) {
        let mut queue = lock_ignoring_poison(&self.0.queue);
        queue.running -= 1;
        self.0.start_waiting(&mut queue);
    }
}

/// Runs and keeps the simulation jobs (see the module documentation).
pub struct JobManager {
    /// The server-wide settings.
    pub config: ServerConfig,
    jobs: Mutex<BTreeMap<u64, Arc<Job>>>,
    counter: AtomicU64,
    /// Lock order: `queue` before `jobs` and before a job's state.
    queue: Mutex<Queue>,
}

impl JobManager {
    /// A manager without jobs.
    pub fn new(config: ServerConfig) -> Arc<JobManager> {
        Arc::new(JobManager {
            config,
            jobs: Mutex::new(BTreeMap::new()),
            counter: AtomicU64::new(0),
            queue: Mutex::new(Queue::default()),
        })
    }

    /// Simulations that may run at once.
    fn slots(&self) -> usize {
        self.config.max_concurrent.max(1)
    }

    /// Checks the request, registers a job and starts it, or queues it while every slot is
    /// busy; `429` when the queue is full as well.
    pub fn submit(self: &Arc<Self>, req: SimulationRequest) -> Result<Arc<Job>, ApiError> {
        crate::runner::precheck(&req, &self.config).map_err(ApiError::bad_request)?;
        let mut queue = lock_ignoring_poison(&self.queue);
        if queue.running >= self.slots() && queue.waiting.len() >= self.config.max_queued {
            return Err(ApiError::too_many_requests(format!(
                "{} simulations run and {} wait (limits {} and {}): try again when one has ended",
                queue.running,
                queue.waiting.len(),
                self.slots(),
                self.config.max_queued
            )));
        }
        let n = self.counter.fetch_add(1, Ordering::Relaxed) + 1;
        let suffix = (unix_ms() ^ (n << JOB_ID_COUNTER_SHIFT)) & JOB_ID_SUFFIX_MASK;
        let id = format!("sim-{n}-{suffix:06x}");
        let name = req.name.clone().unwrap_or_else(|| default_name(&req));
        let job = Arc::new(Job::new(id, name, self.config.log_capacity));
        {
            let mut jobs = lock_ignoring_poison(&self.jobs);
            jobs.insert(n, job.clone());
            self.evict(&mut jobs);
        }
        job.log(
            Level::Info,
            LogSource::Server,
            None,
            format!("simulation {} submitted", job.id),
            None,
        );
        queue.waiting.push_back(Pending {
            job: job.clone(),
            request: req,
        });
        self.start_waiting(&mut queue);
        Ok(job)
    }

    /// Starts waiting jobs on threads of their own while slots are free.
    fn start_waiting(self: &Arc<Self>, queue: &mut Queue) {
        while queue.running < self.slots() {
            let Some(Pending { job, request }) = queue.waiting.pop_front() else {
                return;
            };
            let (mgr, j) = (self.clone(), job.clone());
            let spawned = std::thread::Builder::new()
                .name(format!("simoxide-{}", job.id))
                .stack_size(self.config.stack_size)
                .spawn(move || mgr.execute(&j, request));
            match spawned {
                Ok(_) => queue.running += 1,
                Err(e) => job.end(
                    JobStatus::Failed,
                    None,
                    Some(format!("cannot start a thread: {e}")),
                    None,
                ),
            }
        }
    }

    /// The job thread: holds a slot while it runs the job. A panic of the run fails the job.
    fn execute(self: &Arc<Self>, job: &Arc<Job>, req: SimulationRequest) {
        let _slot = RunningSlot(self);
        if !job.claim_for_run() {
            return;
        }
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::runner::run(job, req, &self.config)
        }));
        if let Err(payload) = r {
            let message = format!("internal error: {}", panic_message(payload.as_ref()));
            job.log(Level::Error, LogSource::Server, None, message.clone(), None);
            job.end(
                JobStatus::Failed,
                Some(StopReason {
                    kind: StopKind::Error,
                    detail: "internal error".into(),
                }),
                Some(message),
                None,
            );
        }
    }

    /// Asks `job` to stop ([`Job::request_cancel`]); a queued job also leaves the queue, with
    /// its request. Returns false if the job had already ended.
    pub fn cancel(&self, job: &Arc<Job>) -> bool {
        let requested = job.request_cancel();
        lock_ignoring_poison(&self.queue)
            .waiting
            .retain(|p| !Arc::ptr_eq(&p.job, job));
        requested
    }

    /// Drops the oldest finished jobs beyond `max_jobs`.
    fn evict(&self, jobs: &mut BTreeMap<u64, Arc<Job>>) {
        let max = self.config.max_jobs.max(1);
        if jobs.len() <= max {
            return;
        }
        let excess = jobs.len() - max;
        let old: Vec<u64> = jobs
            .iter()
            .filter(|(_, j)| j.status().is_terminal())
            .map(|(k, _)| *k)
            .take(excess)
            .collect();
        for k in old {
            jobs.remove(&k);
        }
    }

    /// The job with this id.
    pub fn get(&self, id: &str) -> Result<Arc<Job>, ApiError> {
        lock_ignoring_poison(&self.jobs)
            .values()
            .find(|j| j.id == id)
            .cloned()
            .ok_or_else(|| ApiError::not_found(id))
    }

    /// Every job, oldest first.
    pub fn list(&self) -> Vec<Arc<Job>> {
        lock_ignoring_poison(&self.jobs).values().cloned().collect()
    }

    /// Cancels a job if it runs and forgets it.
    pub fn delete(&self, id: &str) -> Result<(), ApiError> {
        let removed = {
            let mut jobs = lock_ignoring_poison(&self.jobs);
            let key = jobs
                .iter()
                .find(|(_, j)| j.id == id)
                .map(|(k, _)| *k)
                .ok_or_else(|| ApiError::not_found(id))?;
            jobs.remove(&key)
        };
        if let Some(j) = removed {
            self.cancel(&j);
        }
        Ok(())
    }

    /// Cancels every queued and running job (server shutdown).
    pub fn cancel_all(&self) {
        for j in self.list() {
            self.cancel(&j);
        }
    }
}

/// A name for a request without one: the usage model's file name, or the model directory.
fn default_name(req: &SimulationRequest) -> String {
    if let Some(d) = &req.model_dir {
        return d.clone();
    }
    req.files
        .iter()
        .find(|f| f.name.ends_with(USAGE_MODEL_SUFFIX))
        .map(|f| f.name.trim_end_matches(USAGE_MODEL_SUFFIX).to_string())
        .unwrap_or_else(|| "simulation".into())
}

/// The text of a panic's payload.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "unknown panic".into())
}

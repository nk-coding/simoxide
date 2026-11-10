//! Runs one job: loads and compiles the model, chooses the engine, drives the run in steps
//! between which the [`Monitor`] publishes progress and checks the termination conditions, and
//! stores the outcome.
//!
//! The engines (all with the same driver, `Drive`):
//!
//! | flavour | scaling policies | engine |
//! |---|---|---|
//! | `simulizar`, `slingshot` | none | `Simulation::<C, F>` (fast or exact mode) |
//! | `simulizar`, `slingshot` | `.spd` (+ HPA) | `simoxide_spd::sim::SpdRun` |
//! | `slingshot-exact` | none or `.spd` | `simoxide_sim::ssx::SsxRun` (+ `SsxSpd` hooks) |
//!
//! This is the setup of `simoxide run` (crates/simoxide-cli) for the subset a service needs:
//! no trace, tape, snapshots or SimuLizar reconfiguration rules.

use crate::job::{HARD_STOP_GRACE, Job, ModelInfo, Outcome, SeriesName, ServerConfig};
use crate::monitor::{Monitor, PauseWhy, Probe, capped_duration, ns_to_secs, secs_to_ns};
use crate::report::{
    JobStatus, Level, LogSource, ResultSummary, RunInterval, StopKind, StopReason, TimePoint,
    TupleRange,
};
use crate::request::{
    MAX_PROGRESS_INTERVAL_MS, MIN_PROGRESS_INTERVAL_MS, RUN_JSON, RunOptions, SimulationRequest,
    check_name,
};
use crate::stats::Warmup;
use simoxide_sim::compat::Compat;
use simoxide_sim::ssx::{SpdHooks, SsxRun};
use simoxide_sim::{
    CompiledModel, Flavour, FlavourConfig, FlavourKind, Measurements, Mode, Outputs, Quirks,
    RunResult, RunSpec, RunState, SimConfig, SimError, SimErrorKind, SimuLizar, Simulation,
    Slingshot, Status,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The simulator's stop conditions have this value when they are off.
const SIM_STOP_OFF: i64 = -1;
/// Name of the exact processor-sharing algorithm in `config.ps_algorithm`.
const PS_EXACT: &str = "exact";
/// Name of the virtual-time processor-sharing algorithm in `config.ps_algorithm`.
const PS_VIRTUAL_TIME: &str = "virtual-time";
/// The stop reason's text of a cancellation.
const CANCELLED_BY_CLIENT: &str = "cancelled by a client";
/// Milliseconds per second, for log messages.
const MS_PER_SEC: f64 = 1e3;
/// Suffixes of the HPA autoscaler files.
const HPA_SUFFIXES: [&str; 2] = [".hpa", ".hpa.json"];

/// Where the model comes from.
enum Source {
    /// Files of the request: (name, bytes).
    Files(Vec<(String, Vec<u8>)>),
    /// A model directory on the server.
    Dir(PathBuf),
}

impl Source {
    /// The names of the model's files, in name order for a directory.
    fn names(&self) -> Vec<String> {
        match self {
            Source::Files(f) => f.iter().map(|(n, _)| n.clone()).collect(),
            Source::Dir(d) => {
                let mut v: Vec<String> = std::fs::read_dir(d)
                    .map(|rd| {
                        rd.filter_map(|e| e.ok())
                            .filter(|e| e.path().is_file())
                            .map(|e| e.file_name().to_string_lossy().into_owned())
                            .collect()
                    })
                    .unwrap_or_default();
                v.sort();
                v
            }
        }
    }

    /// The text of the files whose names end with `suffix`, in name order.
    fn texts_with_suffix(&self, suffix: &str) -> Result<Vec<(String, String)>, String> {
        let mut out = Vec::new();
        match self {
            Source::Files(f) => {
                for (n, b) in f {
                    if n.ends_with(suffix) {
                        let t = String::from_utf8(b.clone())
                            .map_err(|_| format!("{n}: not UTF-8 text"))?;
                        out.push((n.clone(), t));
                    }
                }
            }
            Source::Dir(d) => {
                for n in self.names() {
                    if n.ends_with(suffix) {
                        let p = d.join(&n);
                        let t = std::fs::read_to_string(&p)
                            .map_err(|e| format!("{}: {e}", p.display()))?;
                        out.push((n, t));
                    }
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

/// Resolves `model_dir` inside one of the allowed directories: inside `root` if given (an
/// allowed directory, as the model listing reports it), else inside the one allowed directory
/// that has it. A name that several allowed directories have needs `root`, so that a request
/// never runs another model than the listing showed.
pub fn resolve_dir(dir: &str, root: Option<&str>, allow: &[PathBuf]) -> Result<PathBuf, String> {
    if allow.is_empty() {
        return Err(
            "model_dir is disabled on this server (start it with --allow-dir DIR), send the files instead"
                .into(),
        );
    }
    let roots: Vec<PathBuf> = allow.iter().filter_map(|r| r.canonicalize().ok()).collect();
    let roots = match root {
        None => roots,
        Some(r) => {
            let wanted = Path::new(r).canonicalize().ok();
            let chosen: Vec<PathBuf> = roots
                .into_iter()
                .filter(|a| Some(a) == wanted.as_ref())
                .collect();
            if chosen.is_empty() {
                return Err(format!("root {r:?}: not one of the allowed directories"));
            }
            chosen
        }
    };
    let rel = Path::new(dir);
    let mut found: Vec<PathBuf> = Vec::new();
    for root in &roots {
        let cand = if rel.is_absolute() {
            rel.to_path_buf()
        } else {
            root.join(rel)
        };
        if let Ok(c) = cand.canonicalize()
            && c.starts_with(root)
            && c.is_dir()
            && !found.contains(&c)
        {
            found.push(c);
        }
    }
    match found.len() {
        0 => Err(format!(
            "model_dir {dir:?}: not a directory inside the allowed directories"
        )),
        1 => Ok(found.remove(0)),
        _ => Err(format!(
            "model_dir {dir:?} is a directory below several allowed directories: give root (see the model listing)"
        )),
    }
}

/// The request's `model_dir`, resolved (see [`resolve_dir`]).
fn request_dir(req: &SimulationRequest, dir: &str, cfg: &ServerConfig) -> Result<PathBuf, String> {
    resolve_dir(dir, req.root.as_deref(), &cfg.allow_dirs)
}

/// `config.flavour`, by default SimuLizar.
fn parse_flavour(o: &RunOptions) -> Result<FlavourKind, String> {
    match &o.flavour {
        None => Ok(FlavourKind::SimuLizar),
        Some(s) => FlavourKind::parse(s).ok_or_else(|| {
            format!("config.flavour: expected simulizar, slingshot or slingshot-exact, got {s:?}")
        }),
    }
}

/// `config.mode`, by default fast; an error if this build lacks the mode.
fn parse_mode(o: &RunOptions) -> Result<Mode, String> {
    let m = match &o.mode {
        None => Mode::Fast,
        Some(s) => Mode::parse(s)
            .ok_or_else(|| format!("config.mode: expected fast or exact, got {s:?}"))?,
    };
    if let Some(why) = m.unavailable_reason() {
        return Err(format!("config.mode {m}: {why}"));
    }
    Ok(m)
}

/// `config.quirks`, by default none.
fn parse_quirks(o: &RunOptions) -> Result<Quirks, String> {
    match &o.quirks {
        None => Ok(Quirks::default()),
        Some(s) => Quirks::parse(s).map_err(|e| format!("config.quirks: {e}")),
    }
}

/// The checks that need no model: run before a job is registered, so that a bad request is a
/// 400 and not a failed job.
pub fn precheck(req: &SimulationRequest, cfg: &ServerConfig) -> Result<(), String> {
    match (&req.model_dir, req.files.is_empty()) {
        (Some(_), false) => return Err("give either files or model_dir, not both".into()),
        (None, true) => {
            return Err("no model: give files (the model's XMI files) or model_dir".into());
        }
        (Some(d), true) => {
            request_dir(req, d, cfg)?;
        }
        (None, false) => {
            if req.root.is_some() {
                return Err(
                    "root selects the directory of a model_dir: give it only with model_dir".into(),
                );
            }
            let mut names = std::collections::HashSet::new();
            for f in &req.files {
                if !names.insert(&f.name) {
                    return Err(format!("file {} is given twice", f.name));
                }
                // the name, exactly one content, valid base64
                f.bytes()?;
            }
        }
    }
    check_entry_names(&req.config)?;
    let (flavour, _) = flavour_config(&req.config)?;
    parse_mode(&req.config)?;
    #[cfg(feature = "spd")]
    if req.config.spd != Some(false) {
        spd::quirks_of(&req.config, flavour)?;
    }
    if req.config.spd_decisions.is_some() {
        if flavour == FlavourKind::SlingshotExact {
            return Err("config.spd_decisions: scaling decisions run in the simulizar and slingshot flavours, not in slingshot-exact".into());
        }
        if req.config.spd == Some(false) || !cfg!(feature = "spd") {
            return Err(
                "config.spd_decisions: needs the model's scaling policies (config.spd)".into(),
            );
        }
    }
    if flavour == FlavourKind::SlingshotExact && req.config.runtime_metadata {
        return Err("config.runtime_metadata: the slingshot-exact flavour does not record runtime metadata: run simulizar or slingshot".into());
    }
    if flavour == FlavourKind::SlingshotExact && req.config.usage_streams {
        return Err("config.usage_streams: the slingshot-exact flavour has no usage streams (RND-11): run simulizar or slingshot in fast mode".into());
    }
    if flavour == FlavourKind::SlingshotExact && simulator_sim_time(req, cfg) <= 0 {
        return Err("slingshot-exact needs config.max_sim_time (or run.json's max_sim_time): Slingshot's simulation time is its only stop condition".into());
    }
    if let Some(p) = &req.config.ps_algorithm
        && p != PS_EXACT
        && p != PS_VIRTUAL_TIME
    {
        return Err(format!(
            "config.ps_algorithm: expected {PS_EXACT} or {PS_VIRTUAL_TIME}, got {p:?}"
        ));
    }
    if let Some(o) = &req.config.interval_summary {
        o.stats()?;
    }
    req.termination.validate()?;
    let w = &req.progress;
    if !(w.warmup_time.is_finite() && w.warmup_time >= 0.0) {
        return Err("progress.warmup_time: expected a non-negative number".into());
    }
    if w.interval_ms > MAX_PROGRESS_INTERVAL_MS {
        return Err(format!(
            "progress.interval_ms: at most {MAX_PROGRESS_INTERVAL_MS} (one day)"
        ));
    }
    Ok(())
}

/// The entry files of `config` name files of the model: relative, without `..`, so that they
/// stay inside the model directory (`usagemodel: "/etc/passwd"` would read any file).
fn check_entry_names(o: &RunOptions) -> Result<(), String> {
    let names = o
        .usagemodel
        .iter()
        .chain(o.allocation.iter().flatten())
        .chain(o.monitorrepository.iter());
    for name in names {
        check_name(name).map_err(|e| format!("config: entry {e}"))?;
    }
    Ok(())
}

/// The progress interval of a request, within the bounds the server honours.
fn progress_interval(req: &SimulationRequest) -> Duration {
    Duration::from_millis(
        req.progress
            .interval_ms
            .clamp(MIN_PROGRESS_INTERVAL_MS, MAX_PROGRESS_INTERVAL_MS),
    )
}

/// The simulator's `max_sim_time` of a request: `config.max_sim_time`, else its run.json's.
fn simulator_sim_time(req: &SimulationRequest, cfg: &ServerConfig) -> i64 {
    if let Some(t) = req.config.max_sim_time {
        return t;
    }
    let spec = match &req.model_dir {
        Some(d) => request_dir(req, d, cfg)
            .ok()
            .map(|d| d.join(RUN_JSON))
            .filter(|p| p.exists())
            .and_then(|p| RunSpec::load(&p).ok()),
        None => req
            .files
            .iter()
            .find(|f| f.name == RUN_JSON)
            .and_then(|f| f.bytes().ok())
            .and_then(|b| RunSpec::parse(&String::from_utf8_lossy(&b)).ok()),
    };
    spec.map_or(SIM_STOP_OFF, |s| s.max_sim_time)
}

/// A model ready to run.
struct Prepared {
    cm: CompiledModel,
    cfg: SimConfig,
    flavour: FlavourKind,
    info: ModelInfo,
    #[cfg(feature = "spd")]
    hpa: Option<simoxide_spd::hpa::HpaConfig>,
}

/// Loads the model, its run.json, HPA and load-profile files, compiles it and builds the
/// simulation configuration.
fn prepare(job: &Job, req: &SimulationRequest, scfg: &ServerConfig) -> Result<Prepared, String> {
    let o = &req.config;
    let src = model_source(req, scfg)?;
    let spec = run_spec(&src, o)?;
    let files = src.names();
    job.log(
        Level::Info,
        LogSource::Model,
        None,
        format!("loading {} files", files.len()),
        Some(serde_json::json!({ "files": files })),
    );
    #[allow(unused_mut)]
    let mut model = match &src {
        Source::Files(f) => spec.load_model_memory(f)?,
        Source::Dir(d) => spec.load_model(d)?,
    };
    let hpa_files = hpa_file_texts(&src)?;
    #[cfg(feature = "spd")]
    let hpa = merge_autoscalers(job, o, &hpa_files)?;
    // HPA autoscalers join the SPD before the model is compiled
    #[cfg(feature = "spd")]
    if let Some(config) = &hpa {
        simoxide_spd::hpa::attach(&mut model, config)?;
    }
    #[cfg(not(feature = "spd"))]
    if !hpa_files.is_empty() && o.hpa != Some(false) {
        job.log(
            Level::Warn,
            LogSource::Model,
            None,
            "HPA files ignored: this server is built without the `spd` feature",
            None,
        );
    }
    let (flavour, flavour_config) = flavour_config(o)?;
    let cm = CompiledModel::compile_flavour(model, flavour_config)
        .map_err(|e| format!("compile: {e}"))?;
    for w in &cm.warnings {
        job.log(Level::Warn, LogSource::Model, None, w.clone(), None);
    }
    let mut cfg = sim_config(job, req, &src, &spec, scfg)?;
    let simulator_stops = warn_if_unbounded(job, req, &cfg, scfg)?;
    let (max_sim_time, max_measurements) = (cfg.max_sim_time, cfg.max_measurements);
    // The process engine needs a stop condition of its own (SimuLizar refuses to start
    // without one). Without one in the request, the server's termination conditions end the
    // run, and the simulator's count is set where it never holds. (A finished usage-scenario
    // run then also counts as progress for the livelock guard, SimConfig::max_events_per_instant.)
    if !simulator_stops && flavour != FlavourKind::SlingshotExact {
        cfg.max_measurements = i64::MAX;
    }
    let series = initial_series(&cm, &cfg);
    let info = ModelInfo {
        flavour: flavour.name().into(),
        mode: cfg.mode.to_string(),
        seed: cfg.seed,
        max_sim_time,
        max_measurements,
        files,
        series,
        scaling_policies: 0,
        autoscalers: {
            #[cfg(feature = "spd")]
            {
                hpa.as_ref().map_or(0, |h| h.autoscalers.len())
            }
            #[cfg(not(feature = "spd"))]
            0
        },
        adjustments_executed: None,
        load_profile: cfg.load_profile.is_some(),
        warnings: cm.warnings.clone(),
    };
    Ok(Prepared {
        cm,
        cfg,
        flavour,
        info,
        #[cfg(feature = "spd")]
        hpa,
    })
}

/// Where the request's model is: its files, or the directory it names.
fn model_source(req: &SimulationRequest, scfg: &ServerConfig) -> Result<Source, String> {
    Ok(match &req.model_dir {
        Some(d) => Source::Dir(request_dir(req, d, scfg)?),
        None => Source::Files(
            req.files
                .iter()
                .map(|f| Ok((f.name.clone(), f.bytes()?)))
                .collect::<Result<_, String>>()?,
        ),
    })
}

/// The model's `run.json` (or the defaults), overridden by the request's run options.
fn run_spec(src: &Source, o: &RunOptions) -> Result<RunSpec, String> {
    let mut spec = match src {
        Source::Files(f) => match f.iter().find(|(n, _)| n == RUN_JSON) {
            Some((_, b)) => RunSpec::parse(&String::from_utf8_lossy(b))
                .map_err(|e| format!("{RUN_JSON}: {e}"))?,
            None => RunSpec::default(),
        },
        Source::Dir(d) => {
            let p = d.join(RUN_JSON);
            if p.exists() {
                RunSpec::load(&p)?
            } else {
                RunSpec::default()
            }
        }
    };
    if let Some(s) = o.seed {
        spec.seed = s;
    }
    if let Some(t) = o.max_sim_time {
        spec.max_sim_time = t;
    }
    if let Some(m) = o.max_measurements {
        spec.max_measurements = m;
    }
    if let Some(u) = &o.usagemodel {
        spec.usagemodel = Some(u.clone());
    }
    if let Some(a) = &o.allocation {
        spec.allocation = a.clone();
    }
    if let Some(m) = &o.monitorrepository {
        spec.monitorrepository = Some(m.clone());
    }
    if let Some(b) = o.simulate_linking_resources {
        spec.simulate_linking_resources = b;
    }
    if let Some(b) = o.simulate_throughput_of_linking_resources {
        spec.simulate_throughput_of_linking_resources = b;
    }
    Ok(spec)
}

/// The text of the model's HPA autoscaler files, in name order.
fn hpa_file_texts(src: &Source) -> Result<Vec<(String, String)>, String> {
    let mut files = Vec::new();
    for suffix in HPA_SUFFIXES {
        files.extend(src.texts_with_suffix(suffix)?);
    }
    files.sort();
    Ok(files)
}

/// Merges the autoscalers of the HPA files; `None` if they are turned off or there are none.
/// Autoscaler names must be unique across the files.
#[cfg(feature = "spd")]
fn merge_autoscalers(
    job: &Job,
    o: &RunOptions,
    hpa_files: &[(String, String)],
) -> Result<Option<simoxide_spd::hpa::HpaConfig>, String> {
    if o.hpa == Some(false) || hpa_files.is_empty() {
        return Ok(None);
    }
    if o.spd == Some(false) {
        job.log(
            Level::Warn,
            LogSource::Model,
            None,
            "HPA files ignored: the autoscalers scale SPD target groups and config.spd is false",
            None,
        );
        return Ok(None);
    }
    let mut merged: Option<simoxide_spd::hpa::HpaConfig> = None;
    for (name, text) in hpa_files {
        let config =
            simoxide_spd::hpa::HpaConfig::from_json(text).map_err(|e| format!("{name}: {e}"))?;
        let Some(all) = &mut merged else {
            merged = Some(config);
            continue;
        };
        for autoscaler in config.autoscalers {
            if all.autoscalers.iter().any(|a| a.name == autoscaler.name) {
                return Err(format!(
                    "{name}: two autoscalers named {:?}",
                    autoscaler.name
                ));
            }
            all.autoscalers.push(autoscaler);
        }
    }
    Ok(merged.filter(|config| !config.autoscalers.is_empty()))
}

/// The flavour the request asks for, with the compat flags it allows.
fn flavour_config(o: &RunOptions) -> Result<(FlavourKind, FlavourConfig), String> {
    let flavour = parse_flavour(o)?;
    let quirks = parse_quirks(o)?;
    let config = match flavour {
        FlavourKind::SimuLizar if !quirks.is_empty() => {
            return Err("config.quirks needs the slingshot flavour".into());
        }
        FlavourKind::SimuLizar => FlavourConfig::SIMULIZAR,
        FlavourKind::Slingshot => FlavourConfig::slingshot(quirks),
        FlavourKind::SlingshotExact if !quirks.is_empty() && quirks != Quirks::all() => {
            return Err("slingshot-exact runs with all quirks (config.quirks: all)".into());
        }
        FlavourKind::SlingshotExact => FlavourConfig::slingshot_exact(),
    };
    Ok((flavour, config))
}

/// The simulation configuration: the run spec's, with the request's options, the job's hard
/// cancellation and deadline, and the model's load profiles.
fn sim_config(
    job: &Job,
    req: &SimulationRequest,
    src: &Source,
    spec: &RunSpec,
    scfg: &ServerConfig,
) -> Result<SimConfig, String> {
    let o = &req.config;
    let name = req.name.clone().unwrap_or_else(|| job.name.clone());
    let mut cfg = spec.sim_config(&name);
    cfg.mode = parse_mode(o)?;
    if o.ps_algorithm.as_deref() == Some(PS_VIRTUAL_TIME) {
        cfg.ps_algorithm = simoxide_sim::PsAlgorithm::VirtualTime;
    }
    if let Some(n) = o.max_events_per_instant {
        cfg.max_events_per_instant = n;
    }
    if let Some(n) = o.max_stack_depth {
        cfg.limits.max_stack_depth = n;
    }
    if let Some(n) = o.max_processes {
        cfg.limits.max_processes = n;
    }
    if let Some(n) = o.max_steps {
        cfg.limits.max_steps = n;
    }
    cfg.limits.cancel = Some(job.hard_cancel.clone());
    if let Some(s) = scfg.max_run_secs {
        // the soft stop at the cap keeps the results so far: it gets time to act first
        cfg.limits.deadline = Some(Instant::now() + capped_duration(s) + HARD_STOP_GRACE);
    }
    cfg.usage_streams = o.usage_streams;
    cfg.fix_double_resume = o.fix_double_resume;
    if o.load_profile != Some(false) {
        let profiles = src.texts_with_suffix(simoxide_sim::load::LOAD_PROFILE_SUFFIX)?;
        if !profiles.is_empty() {
            let mut all = simoxide_sim::LoadProfile::default();
            for (name, text) in &profiles {
                all.entries.extend(
                    simoxide_sim::LoadProfile::from_json(text)
                        .map_err(|e| format!("{name}: {e}"))?
                        .entries,
                );
            }
            cfg.load_profile = Some(Arc::new(all));
        }
    }
    Ok(cfg)
}

/// A run that nothing stops would hold a worker forever: refuses it without a server cap, warns
/// with one. Returns whether the simulator has a stop condition of its own.
fn warn_if_unbounded(
    job: &Job,
    req: &SimulationRequest,
    cfg: &SimConfig,
    scfg: &ServerConfig,
) -> Result<bool, String> {
    let simulator_stops = cfg.max_sim_time > 0 || cfg.max_measurements > 0;
    if simulator_stops || req.termination.bounds_the_run() {
        return Ok(simulator_stops);
    }
    let Some(cap) = scfg.max_run_secs else {
        return Err("no stop condition: give config.max_sim_time, config.max_measurements, or a termination criterion that bounds the run (timeout_secs, max_measurements, max_tuples, max_events, max_sim_time)".into());
    };
    job.log(
        Level::Warn,
        LogSource::Termination,
        None,
        format!(
            "no stop condition bounds the run: it ends at the server's cap of {cap} s at the latest"
        ),
        None,
    );
    Ok(false)
}

/// The series the run records from the start (monitors added by model changes come later).
fn initial_series(cm: &CompiledModel, cfg: &SimConfig) -> Vec<SeriesName> {
    let mut c = cfg.clone();
    c.limits.cancel = None;
    let m = match cm.flavour.kind {
        FlavourKind::SlingshotExact => SsxRun::new(cm, c, Outputs::default())
            .ok()
            .map(|r| r.measurements().clone()),
        _ => None,
    };
    let series = match m {
        Some(m) => m.series,
        None => cm.series.clone(),
    };
    let mut v: Vec<SeriesName> = series
        .iter()
        .map(|s| SeriesName {
            measuring_point: s.mp.to_string(),
            metric: s.metric.to_string(),
        })
        .collect();
    v.sort_by(|a, b| (&a.measuring_point, &a.metric).cmp(&(&b.measuring_point, &b.metric)));
    v
}

/// One engine, as the driver sees it.
trait Drive {
    /// Runs until `keep` returns false between two events or the run is over; `next`: the
    /// probes need [`Probe::next_ns`].
    fn step(
        &mut self,
        next: bool,
        keep: &mut dyn FnMut(&Probe) -> bool,
    ) -> Result<Status, SimError>;
    fn probe(&self) -> Probe;
    fn measurements(&self) -> &Measurements;
    /// Ends the run where it is: the measurements so far (moved, not copied).
    fn into_measurements(self) -> Measurements;
    /// Runs to the end (after [`Status::Finished`]: collects the result); the SPD counters, if
    /// the engine has an SPD layer.
    fn finish(self) -> (Result<RunResult, SimError>, Option<serde_json::Value>);
    /// The SPD counters so far (for runs stopped early).
    fn spd_counters(&self) -> Option<serde_json::Value> {
        None
    }
    /// The run's warnings so far (handler exceptions, random-number problems), for runs that
    /// end before the simulator finishes them.
    fn warnings(&self) -> Vec<String>;
    /// A scaling decision the run paused at and cannot resolve (`config.spd_decisions`).
    fn pending_decision(&self) -> Option<String> {
        None
    }
    /// What the runtime file needs from the run now (`config.runtime_metadata`), before it
    /// ends.
    fn metadata_parts(&self) -> Option<MetaParts> {
        None
    }
}

/// `config.runtime_metadata`: the model the run starts from and its model file.
struct MetaCtx<'a> {
    base: &'a CompiledModel,
    model: serde_json::Value,
}

/// The deployment record of a run and what the runtime file needs from its state.
struct MetaParts {
    rec: simoxide_sim::inspect::Recorder,
    /// The run's own model after model changes.
    own: Option<Arc<CompiledModel>>,
    generation: u32,
    policies: serde_json::Value,
}

impl MetaParts {
    fn of<C: Compat, F: Flavour>(
        rec: &simoxide_sim::inspect::Recorder,
        s: &RunState<C, F>,
        policies: serde_json::Value,
    ) -> MetaParts {
        MetaParts {
            rec: rec.clone(),
            own: s.model().compiled().cloned(),
            generation: s.model().generation(),
            policies,
        }
    }

    /// `{"model": ..., "runtime": ...}` of a run that ended at `end_ns` (`status`: `finished`,
    /// `stopped` or `failed`) with the series `series`.
    fn into_json(
        self,
        ctx: &MetaCtx,
        series: &[simoxide_sim::ir::SeriesDef],
        end_ns: i64,
        status: &str,
    ) -> serde_json::Value {
        let run = serde_json::json!({"parent": null, "state": null, "status": status});
        let runtime = self.rec.into_json(
            ctx.model["hash"].as_str().unwrap_or_default(),
            run,
            self.own.as_deref().unwrap_or(ctx.base),
            ctx.base,
            series,
            end_ns,
            self.generation,
            self.policies,
        );
        serde_json::json!({"model": ctx.model, "runtime": runtime})
    }
}

/// The counters of a process-engine run; `next`: with the time of the next event.
fn probe_of<C: Compat, F: Flavour>(s: &RunState<C, F>, next: bool) -> Probe {
    Probe {
        now_ns: s.now_ns(),
        next_ns: if next { s.next_event_ns() } else { None },
        events: s.events(),
        main_count: s.main_count(),
        tuples: s.measurements().count,
        live_processes: Some(s.live_processes()),
    }
}

/// A plain process-engine run (with its deployment record for `config.runtime_metadata`).
struct Plain<'m, C: Compat, F: Flavour>(
    Simulation<'m, C, F>,
    Option<simoxide_sim::inspect::Recorder>,
);

impl<C: Compat, F: Flavour> Drive for Plain<'_, C, F> {
    fn step(
        &mut self,
        next: bool,
        keep: &mut dyn FnMut(&Probe) -> bool,
    ) -> Result<Status, SimError> {
        self.0.run_while(|s| keep(&probe_of(s, next)))
    }
    fn probe(&self) -> Probe {
        probe_of(self.0.state(), true)
    }
    fn measurements(&self) -> &Measurements {
        self.0.state().measurements()
    }
    fn into_measurements(self) -> Measurements {
        self.0.into_state().into_measurements()
    }
    fn finish(self) -> (Result<RunResult, SimError>, Option<serde_json::Value>) {
        (self.0.finish(), None)
    }
    fn warnings(&self) -> Vec<String> {
        self.0.state().warnings().to_vec()
    }
    fn metadata_parts(&self) -> Option<MetaParts> {
        let rec = self.1.as_ref()?;
        Some(MetaParts::of(rec, self.0.state(), serde_json::Value::Null))
    }
}

/// The Slingshot-exact engine, with or without its SPD hooks.
struct Ssx<'m, 'h> {
    run: SsxRun<'m>,
    hooks: Option<&'h mut dyn SpdHooks>,
}

impl Drive for Ssx<'_, '_> {
    fn step(
        &mut self,
        _next: bool,
        keep: &mut dyn FnMut(&Probe) -> bool,
    ) -> Result<Status, SimError> {
        let hooks = self.hooks.as_mut().map(|h| &mut **h as &mut dyn SpdHooks);
        self.run.run_while(hooks, |v| {
            keep(&Probe {
                now_ns: v.now_ns(),
                next_ns: v.next_event().map(secs_to_ns),
                events: v.events(),
                main_count: v.main_count(),
                tuples: v.measurements().count,
                live_processes: None,
            })
        })
    }
    fn probe(&self) -> Probe {
        Probe {
            now_ns: self.run.now_ns(),
            next_ns: None,
            events: self.run.events(),
            main_count: self.run.main_count(),
            tuples: self.run.measurements().count,
            live_processes: None,
        }
    }
    fn measurements(&self) -> &Measurements {
        self.run.measurements()
    }
    fn into_measurements(self) -> Measurements {
        self.run.into_measurements()
    }
    fn finish(mut self) -> (Result<RunResult, SimError>, Option<serde_json::Value>) {
        let hooks = self.hooks.take();
        (self.run.finish(hooks), None)
    }
    fn warnings(&self) -> Vec<String> {
        self.run.warnings().to_vec()
    }
}

/// How the driver loop ended.
enum End {
    /// The simulator's own stop condition held or no event was left.
    Finished,
    /// The monitor stopped the run.
    Stopped(JobStatus, StopReason),
    /// The run aborted.
    Failed(SimError),
}

/// Runs `d` in steps: checkpoint, publish progress, check termination, until the run ends.
fn drive<D: Drive>(job: &Job, d: &mut D, mon: &mut Monitor, interval: Duration) -> End {
    let mut last_publish = Instant::now();
    // the progress at t = 0, with the (empty) series
    let p0 = d.probe();
    mon.checkpoint(d.measurements(), &p0);
    job.publish(mon.progress(d.measurements(), &p0));
    loop {
        let next = mon.wants_next_time();
        let r = d.step(next, &mut |p| mon.keep_going(p));
        let p = d.probe();
        match r {
            Err(e) => return End::Failed(e),
            Ok(Status::Finished) => return End::Finished,
            Ok(Status::Paused) => {}
        }
        if let Some(s) = d.pending_decision() {
            return completed(StopKind::Decision, s);
        }
        match mon.why.take() {
            Some(PauseWhy::Terminate(s)) => return completed(StopKind::Termination, s),
            Some(PauseWhy::ServerLimit(s)) => return completed(StopKind::ServerLimit, s),
            Some(PauseWhy::Cancel) => return cancelled(),
            Some(PauseWhy::Checkpoint) | None => {}
        }
        mon.checkpoint(d.measurements(), &p);
        let now = Instant::now();
        if mon.cancelled() {
            return cancelled();
        }
        if let Some(s) = mon.cap_reason(now) {
            return completed(StopKind::ServerLimit, s);
        }
        if let Some(s) = mon.evaluate(&p, now) {
            return completed(StopKind::Termination, s);
        }
        if now.duration_since(last_publish) >= interval {
            last_publish = now;
            job.publish(mon.progress(d.measurements(), &p));
        }
    }
}

/// The monitor stopped the run regularly.
fn completed(kind: StopKind, detail: String) -> End {
    let cause = (kind == StopKind::Decision).then_some("decision");
    End::Stopped(
        JobStatus::Completed,
        StopReason {
            kind,
            detail,
            cause,
        },
    )
}

/// A client stopped the run.
fn cancelled() -> End {
    End::Stopped(
        JobStatus::Cancelled,
        StopReason {
            kind: StopKind::Cancelled,
            detail: CANCELLED_BY_CLIENT.into(),
            cause: None,
        },
    )
}

/// What a run left: the job's final state.
struct RunEnd {
    status: JobStatus,
    stop: StopReason,
    error: Option<String>,
    measurements: Option<Measurements>,
    probe: Probe,
    uniforms: Option<u64>,
    warnings: Vec<String>,
    spd: Option<serde_json::Value>,
    /// `config.runtime_metadata`: the model file and the runtime file.
    runtime_metadata: Option<serde_json::Value>,
}

/// Turns the end of the driver loop into the job's final state; `meta` with
/// `config.runtime_metadata`.
fn conclude<D: Drive>(
    job: &Job,
    d: D,
    end: End,
    cfg: &SimConfig,
    meta: Option<&MetaCtx>,
) -> RunEnd {
    let probe = d.probe();
    // the warnings so far, for the ends without a result of the simulator
    let warnings = d.warnings();
    let parts = meta.and(d.metadata_parts());
    let runtime = |parts: Option<MetaParts>, series: &[_], end_ns, status| {
        Some(parts?.into_json(meta?, series, end_ns, status))
    };
    match end {
        End::Finished => {
            let (r, spd) = d.finish();
            match r {
                Ok(r) => {
                    let runtime_metadata =
                        runtime(parts, &r.measurements.series, r.end_ns, "finished");
                    let (cause, detail) = simulator_stop_detail(&r, cfg);
                    RunEnd {
                        status: JobStatus::Completed,
                        stop: StopReason {
                            kind: StopKind::Simulator,
                            detail,
                            cause: Some(cause),
                        },
                        error: None,
                        probe: Probe {
                            now_ns: r.end_ns,
                            events: r.events,
                            main_count: r.main_count,
                            tuples: r.measurements.count,
                            ..probe
                        },
                        uniforms: Some(r.uniforms),
                        warnings: r.warnings,
                        measurements: Some(r.measurements),
                        spd,
                        runtime_metadata,
                    }
                }
                Err(e) => failed(job, e, None, probe, spd, warnings),
            }
        }
        End::Stopped(status, stop) => {
            let spd = d.spd_counters();
            let runtime_metadata = parts
                .and_then(|p| runtime(Some(p), &d.measurements().series, probe.now_ns, "stopped"));
            RunEnd {
                status,
                stop,
                error: None,
                measurements: Some(d.into_measurements()),
                probe,
                uniforms: None,
                warnings,
                spd,
                runtime_metadata,
            }
        }
        End::Failed(e) => {
            let spd = d.spd_counters();
            let runtime_metadata = parts
                .and_then(|p| runtime(Some(p), &d.measurements().series, probe.now_ns, "failed"));
            let mut r = failed(job, e, Some(d.into_measurements()), probe, spd, warnings);
            r.runtime_metadata = runtime_metadata;
            r
        }
    }
}

/// Which of the simulator's own stops ended a run that it finished itself: the cause
/// (`simoxide_sim::stop::completion`) and in words.
fn simulator_stop_detail(r: &RunResult, cfg: &SimConfig) -> (&'static str, String) {
    let cause = simoxide_sim::stop::completion(r.end_ns, r.main_count, cfg).cause;
    let detail = match cause {
        "max_measurements" => format!(
            "config.max_measurements: {} usage-scenario runs",
            r.main_count
        ),
        "max_sim_time" => format!("config.max_sim_time: {} s simulated", cfg.max_sim_time),
        _ => "no event left".into(),
    };
    (cause, detail)
}

/// An aborted run: a cancellation, the server's deadline or an error.
fn failed(
    job: &Job,
    e: SimError,
    m: Option<Measurements>,
    probe: Probe,
    spd: Option<serde_json::Value>,
    warnings: Vec<String>,
) -> RunEnd {
    let (status, stop, error) = match e.kind {
        // the simulator saw a cancellation flag or the deadline: a client's cancellation sets
        // the soft flag before the hard one
        SimErrorKind::Cancelled if job.cancel_requested() => (
            JobStatus::Cancelled,
            StopReason {
                kind: StopKind::Cancelled,
                detail: CANCELLED_BY_CLIENT.into(),
                cause: None,
            },
            None,
        ),
        SimErrorKind::Cancelled => (
            JobStatus::Completed,
            StopReason {
                kind: StopKind::ServerLimit,
                detail: format!("the server's run-time cap: {}", e.message),
                cause: None,
            },
            None,
        ),
        _ => (
            JobStatus::Failed,
            StopReason {
                kind: StopKind::Error,
                detail: e.to_string(),
                cause: Some(simoxide_sim::stop::error(&e).cause),
            },
            Some(e.to_string()),
        ),
    };
    RunEnd {
        status,
        stop,
        error,
        measurements: m,
        probe,
        uniforms: None,
        warnings,
        spd,
        runtime_metadata: None,
    }
}

/// Runs a job to its end (called on the job's thread, holding a simulation slot, with the job
/// in status `loading`).
pub fn run(job: &Arc<Job>, req: SimulationRequest, scfg: &ServerConfig) {
    let t0 = Instant::now();
    let p = match prepare(job, &req, scfg) {
        Ok(p) => p,
        Err(e) => return fail_without_results(job, LogSource::Model, e),
    };
    job.log(
        Level::Info,
        LogSource::Model,
        None,
        format!(
            "model compiled in {:.1} ms: flavour {}, mode {}, seed {}, {} series",
            t0.elapsed().as_secs_f64() * MS_PER_SEC,
            p.info.flavour,
            p.info.mode,
            p.info.seed,
            p.info.series.len()
        ),
        None,
    );
    let interval = progress_interval(&req);
    let mut mon = Monitor::new(
        req.termination.clone(),
        Warmup {
            time: req.progress.warmup_time,
            samples: req.progress.warmup_samples,
        },
        interval,
        job.cancel.clone(),
        scfg.max_run_secs.map(capped_duration),
        scfg.max_tuples,
    );
    let end = run_engine(job, &req, &p, &mut mon, interval);
    // the simulation's own wall time, without the final statistics
    let wall_secs = mon.started.elapsed().as_secs_f64();
    let end = match end {
        Ok(e) => e,
        Err(e) => return fail_without_results(job, LogSource::Simulator, e),
    };
    let RunEnd {
        status,
        stop,
        error,
        measurements,
        probe,
        uniforms,
        warnings,
        spd,
        runtime_metadata,
    } = end;
    for w in &warnings {
        job.log(Level::Warn, LogSource::Simulator, None, w.clone(), None);
    }
    let level = match status {
        JobStatus::Failed => Level::Error,
        _ => Level::Info,
    };
    job.log(
        level,
        LogSource::Termination,
        Some(ns_to_secs(probe.now_ns)),
        format!(
            "{} after {wall_secs:.3} s ({}): {}",
            status.as_str(),
            stop.kind.as_str(),
            stop.detail
        ),
        None,
    );
    // the logical end: a `max_sim_time` criterion's time if it ended the run
    let end_ns = match stop.kind {
        StopKind::Termination => mon.time_boundary(probe.now_ns).unwrap_or(probe.now_ns),
        _ => probe.now_ns,
    };
    let outcome = measurements.map(|m| {
        mon.checkpoint(&m, &probe);
        // the last progress, so that live views end with the final numbers
        job.publish(mon.progress(&m, &probe));
        Outcome {
            summary: ResultSummary {
                stop_reason: stop.clone(),
                sim_time: ns_to_secs(probe.now_ns),
                interval: RunInterval {
                    start: TimePoint::of_ns(0),
                    end: TimePoint::of_ns(end_ns),
                    last_event: TimePoint::of_ns(probe.now_ns),
                    tuples: TupleRange {
                        from: 0,
                        to: m.count,
                    },
                },
                events: probe.events,
                measurements: probe.main_count,
                tuples: m.count,
                wall_secs,
                uniforms,
                warnings,
                series: mon.series_stats(&m, true),
                spd,
                interval_summary: interval_summary(&req, &m, end_ns),
            },
            measurements: m,
            runtime_metadata,
        }
    });
    job.end(status, Some(stop), error, outcome);
}

/// `config.interval_summary`: the summary of the run, one interval from time 0 to its logical
/// end (`end_ns`, `summary.interval.end`).
fn interval_summary(
    req: &SimulationRequest,
    m: &simoxide_sim::Measurements,
    end_ns: i64,
) -> Option<serde_json::Value> {
    use simoxide_sim::summary::{Interval, document};
    let o = req.config.interval_summary.as_ref()?;
    // validated with the request
    let stats = o.stats().unwrap_or_default();
    let iv = Interval {
        tuples_from: Some(0),
        tuples_to: m.count,
        from: 0.0,
        to: ns_to_secs(end_ns),
        from_ns: 0,
        to_ns: end_ns,
    };
    let select = |d: &simoxide_sim::ir::SeriesDef| {
        o.series.is_empty() || o.series.iter().any(|s| s.matches(&d.mp, d.metric))
    };
    Some(document(m, &iv, &select, &stats))
}

/// Ends a job that failed before it recorded anything.
fn fail_without_results(job: &Job, source: LogSource, error: String) {
    job.log(Level::Error, source, None, error.clone(), None);
    job.end(
        JobStatus::Failed,
        Some(StopReason {
            kind: StopKind::Error,
            detail: error.clone(),
            cause: None,
        }),
        Some(error),
        None,
    );
}

/// Chooses the engine and runs it.
fn run_engine(
    job: &Arc<Job>,
    req: &SimulationRequest,
    p: &Prepared,
    mon: &mut Monitor,
    interval: Duration,
) -> Result<RunEnd, String> {
    #[cfg(feature = "spd")]
    {
        let spd = spd::of(job, &req.config, p)?;
        let mut info = p.info.clone();
        if let Some(s) = &spd {
            info.scaling_policies = s.contexts().len();
            info.adjustments_executed = Some(cfg!(feature = "reconfiguration"));
        }
        job.set_model(info);
        if let Some(s) = &spd {
            job.set_status(JobStatus::Running);
            return spd::run(job, req, p, s, mon, interval);
        }
    }
    #[cfg(not(feature = "spd"))]
    {
        job.set_model(p.info.clone());
    }
    if req.config.spd_decisions.is_some() {
        return Err(
            "config.spd_decisions: the model has no scaling policies (.spd) to decide".into(),
        );
    }
    job.set_status(JobStatus::Running);
    let cm = &p.cm;
    let cfg = p.cfg.clone();
    let err = |e: SimError| e.to_string();
    Ok(match (cfg.mode, p.flavour) {
        (_, FlavourKind::SlingshotExact) => {
            let run = SsxRun::new(cm, cfg.clone(), Outputs::default()).map_err(err)?;
            let mut d = Ssx { run, hooks: None };
            let end = drive(job, &mut d, mon, interval);
            conclude(
                job,
                d,
                end,
                &simoxide_sim::stop::effective_config(&cfg, &cm.flavour),
                None,
            )
        }
        #[cfg(feature = "exact")]
        (Mode::Exact, FlavourKind::SimuLizar) => run_plain::<simoxide_sim::Exact, SimuLizar>(
            job,
            cm,
            cfg,
            mon,
            interval,
            req.config.runtime_metadata,
        )?,
        #[cfg(feature = "exact")]
        (Mode::Exact, FlavourKind::Slingshot) => run_plain::<simoxide_sim::Exact, Slingshot>(
            job,
            cm,
            cfg,
            mon,
            interval,
            req.config.runtime_metadata,
        )?,
        (Mode::Fast, FlavourKind::SimuLizar) => run_plain::<simoxide_sim::Fast, SimuLizar>(
            job,
            cm,
            cfg,
            mon,
            interval,
            req.config.runtime_metadata,
        )?,
        (Mode::Fast, FlavourKind::Slingshot) => run_plain::<simoxide_sim::Fast, Slingshot>(
            job,
            cm,
            cfg,
            mon,
            interval,
            req.config.runtime_metadata,
        )?,
        #[cfg(not(feature = "exact"))]
        (m, _) => return Err(m.unavailable_reason().unwrap_or_default()),
    })
}

/// Runs a process engine without scaling policies.
fn run_plain<C: Compat, F: Flavour>(
    job: &Job,
    cm: &CompiledModel,
    cfg: SimConfig,
    mon: &mut Monitor,
    interval: Duration,
    runtime_metadata: bool,
) -> Result<RunEnd, String> {
    let sim = Simulation::<C, F>::create(cm, cfg.clone(), Outputs::default())
        .map_err(|e| e.to_string())?;
    let meta = runtime_metadata.then(|| {
        let d = simoxide_sim::inspect::deployment(cm, &cm.model);
        MetaCtx {
            base: cm,
            model: simoxide_sim::inspect::model_file(cm, &d, serde_json::Value::Null),
        }
    });
    let rec = runtime_metadata.then(|| {
        simoxide_sim::inspect::Recorder::start(
            simoxide_sim::inspect::deployment(cm, &cm.model),
            0,
            0,
            serde_json::Value::Null,
        )
    });
    let mut d = Plain(sim, rec);
    let end = drive(job, &mut d, mon, interval);
    Ok(conclude(
        job,
        d,
        end,
        &simoxide_sim::stop::effective_config(&cfg, &cm.flavour),
        meta.as_ref(),
    ))
}

/// Scaling policies (`.spd`) and HPA autoscalers.
#[cfg(feature = "spd")]
mod spd {
    use super::*;
    use simoxide_spd::Spd;
    use simoxide_spd::decision::DecisionError;
    use simoxide_spd::log::{Log, Record};
    use simoxide_spd::sim::{Adjuster, SpdRun, spd_of};
    use simoxide_spd::ssx::SsxSpd;

    /// The model's scaling policies, if it has any and the request does not turn them off.
    pub fn of(job: &Job, o: &RunOptions, p: &Prepared) -> Result<Option<Spd>, String> {
        if o.spd == Some(false) {
            return Ok(None);
        }
        let q = quirks_of(o, p.flavour)?;
        let s = spd_of(&p.cm.model, q).map_err(|e| e.to_string())?;
        if let Some(s) = &s {
            if let Some(d) = s.dropped() {
                job.log(
                    Level::Warn,
                    LogSource::Spd,
                    None,
                    format!("SPD dropped (Q11 compat, as Slingshot): {d}"),
                    None,
                );
            }
            job.log(
                Level::Info,
                LogSource::Spd,
                None,
                format!(
                    "{} scaling policies; adjustments {}",
                    s.contexts().len(),
                    if cfg!(feature = "reconfiguration") {
                        "executed (Slingshot's transformation)"
                    } else {
                        "decided only (server built without `reconfiguration`)"
                    }
                ),
                None,
            );
        }
        Ok(s)
    }

    /// `config.spd_quirks` for `flavour`: by default `fixed`, and `slingshot` (the only one it
    /// allows) for slingshot-exact.
    pub fn quirks_of(o: &RunOptions, flavour: FlavourKind) -> Result<simoxide_spd::Quirks, String> {
        let exact = flavour == FlavourKind::SlingshotExact;
        let q = match &o.spd_quirks {
            Some(s) => {
                simoxide_spd::Quirks::parse(s).map_err(|e| format!("config.spd_quirks: {e}"))?
            }
            None if exact => simoxide_spd::Quirks::slingshot(),
            None => simoxide_spd::Quirks::fixed(),
        };
        if exact && q != simoxide_spd::Quirks::slingshot() {
            return Err(
                "slingshot-exact runs the SPD with all quirks (config.spd_quirks: slingshot)"
                    .into(),
            );
        }
        Ok(q)
    }

    /// Forwards the SPD decision log into the job's log.
    pub struct JobLog<'a> {
        /// The job whose log it is.
        pub job: &'a Job,
        /// Also forward the evaluations that requested nothing (`config.spd_log_all`).
        pub all: bool,
    }

    impl Log for JobLog<'_> {
        fn enabled(&self) -> bool {
            true
        }
        fn all(&self) -> bool {
            self.all
        }
        fn record(&mut self, r: Record) {
            let line = r.to_json();
            let v: serde_json::Value =
                serde_json::from_str(&line).unwrap_or(serde_json::Value::String(line));
            // a field of the record as text
            let s = |k: &str| {
                v.get(k)
                    .map(|x| match x {
                        serde_json::Value::String(s) => s.clone(),
                        o => o.to_string(),
                    })
                    .unwrap_or_default()
            };
            let t = v.get("t").and_then(serde_json::Value::as_f64);
            let request = v.get("request").and_then(serde_json::Value::as_bool);
            // an evaluation that requests nothing happens on every measurement a policy
            // watches: logged only on request, so that it does not push the rest out of the
            // log's ring buffer
            if request == Some(false) && !self.all {
                return;
            }
            let (level, msg) = match s("k").as_str() {
                "policy" => (
                    Level::Info,
                    format!(
                        "policy {} ({}): active={} trigger={} adjustment={} target={}",
                        s("name"),
                        s("id"),
                        s("active"),
                        s("trigger"),
                        s("adjustment"),
                        s("target_group")
                    ),
                ),
                "eval" => (
                    if request == Some(true) {
                        Level::Info
                    } else {
                        Level::Debug
                    },
                    format!(
                        "policy {} evaluated on {}: request={}",
                        s("policy"),
                        s("ev"),
                        s("request")
                    ),
                ),
                "adjust" => (
                    Level::Info,
                    format!(
                        "adjustment by {}: ok={} size {} -> {}",
                        s("policy_name"),
                        s("ok"),
                        s("size_before"),
                        s("size_after")
                    ),
                ),
                "decision" => (
                    Level::Info,
                    format!(
                        "scaling decision {} at t={}: recommended {}",
                        s("decision"),
                        s("t"),
                        s("recommended")
                    ),
                ),
                "select" => (
                    Level::Info,
                    format!(
                        "scaling decision {} resolved: selected {}",
                        s("decision"),
                        s("selected")
                    ),
                ),
                k => (Level::Info, format!("{k} record")),
            };
            self.job.log(level, LogSource::Spd, t, msg, Some(v));
        }
    }

    #[cfg(feature = "reconfiguration")]
    type Adj = simoxide_reconf::QvtoAdjuster;
    #[cfg(not(feature = "reconfiguration"))]
    type Adj = simoxide_spd::sim::NoAdjustment;

    /// The adjuster: Slingshot's transformation with `reconfiguration`, decisions only without.
    fn adjuster(q: &simoxide_spd::Quirks) -> Result<Adj, String> {
        #[cfg(feature = "reconfiguration")]
        {
            let core = simoxide_reconf::QvtoCore::new().map_err(|e| e.to_string())?;
            let opts = simoxide_reconf::SpdAdjust::new(q);
            simoxide_reconf::QvtoAdjuster::spd(&core, &opts).map_err(|e| e.to_string())
        }
        #[cfg(not(feature = "reconfiguration"))]
        {
            let _ = q;
            Ok(simoxide_spd::sim::NoAdjustment)
        }
    }

    /// A process-engine run with an SPD interpreter.
    struct WithSpd<'m, 's, 'a, C: Compat, F: Flavour> {
        run: SpdRun<'m, 's, C, F>,
        adj: &'a mut dyn Adjuster<C, F>,
        log: &'a mut dyn Log,
        /// `config.spd_decisions`: the action lists not used yet, the resolved decisions.
        decisions: Option<Decisions>,
        /// `config.runtime_metadata`: the deployment record (every adjustment).
        rec: Option<simoxide_sim::inspect::Recorder>,
    }

    /// The scaling decisions of a run with `config.spd_decisions`.
    struct Decisions {
        plan: std::collections::VecDeque<Vec<String>>,
        resolved: Vec<serde_json::Value>,
    }

    impl<C: Compat, F: Flavour> WithSpd<'_, '_, '_, C, F> {
        /// The SPD counters, with the decisions in a run with `config.spd_decisions`.
        fn counters(&self, mut v: serde_json::Value) -> serde_json::Value {
            if let Some(d) = &self.decisions {
                v["decisions"] = serde_json::Value::Array(d.resolved.clone());
                v["pending_decision"] = self
                    .run
                    .pending_decision()
                    .map_or(serde_json::Value::Null, |p| p.to_json());
            }
            v
        }
    }

    impl<C: Compat, F: Flavour> Drive for WithSpd<'_, '_, '_, C, F> {
        fn step(
            &mut self,
            next: bool,
            keep: &mut dyn FnMut(&Probe) -> bool,
        ) -> Result<Status, SimError> {
            let mut recording;
            let adj: &mut dyn Adjuster<C, F> = match self.rec.as_mut() {
                Some(rec) => {
                    recording = simoxide_spd::inspect::Recording {
                        inner: &mut *self.adj,
                        rec,
                    };
                    &mut recording
                }
                None => &mut *self.adj,
            };
            loop {
                let st = self
                    .run
                    .run_while(&mut *adj, &mut *self.log, |s| keep(&probe_of(s, next)))?;
                let Some(d) = &mut self.decisions else {
                    return Ok(st);
                };
                if st != Status::Paused || self.run.pending_decision().is_none() {
                    return Ok(st);
                }
                let Some(actions) = d.plan.pop_front() else {
                    return Ok(st);
                };
                let names: Vec<&str> = actions.iter().map(String::as_str).collect();
                let n = d.resolved.len();
                let r =
                    self.run
                        .decide(&mut *adj, &mut *self.log, &names)
                        .map_err(|e| match e {
                            DecisionError::Sim(e) => e,
                            e => SimError {
                                message: format!("config.spd_decisions[{n}]: {e}"),
                                at_ns: self.run.sim.state().now_ns(),
                                kind: SimErrorKind::Model,
                            },
                        })?;
                d.resolved.push(r.to_json());
            }
        }
        fn pending_decision(&self) -> Option<String> {
            let d = self.run.pending_decision()?;
            Some(format!(
                "scaling decision {} pending at t={} s (recommended: {}): config.spd_decisions has no action list for it",
                d.decision,
                d.time_ns as f64 / 1e9,
                d.policies().join(", ")
            ))
        }
        fn probe(&self) -> Probe {
            probe_of(self.run.sim.state(), true)
        }
        fn measurements(&self) -> &Measurements {
            self.run.sim.state().measurements()
        }
        fn into_measurements(self) -> Measurements {
            self.run.sim.into_state().into_measurements()
        }
        fn finish(self) -> (Result<RunResult, SimError>, Option<serde_json::Value>) {
            let extra = self.counters(serde_json::json!({}));
            match self.run.finish() {
                Ok((r, _, s)) => {
                    let mut v = serde_json::json!({
                        "evaluations": s.evaluations,
                        "requests": s.requests,
                        "adjustments_ok": s.adjustments_ok,
                        "pending": s.pending,
                    });
                    if let (Some(v), serde_json::Value::Object(extra)) = (v.as_object_mut(), extra)
                    {
                        v.extend(extra);
                    }
                    (Ok(r), Some(v))
                }
                Err(e) => (Err(e), None),
            }
        }
        fn spd_counters(&self) -> Option<serde_json::Value> {
            Some(self.counters(serde_json::json!({
                "evaluations": self.run.state.evaluations,
                "requests": self.run.state.requests,
            })))
        }
        fn warnings(&self) -> Vec<String> {
            self.run.sim.state().warnings().to_vec()
        }
        fn metadata_parts(&self) -> Option<MetaParts> {
            let rec = self.rec.as_ref()?;
            Some(MetaParts::of(
                rec,
                self.run.sim.state(),
                simoxide_spd::inspect::policy_state(&self.run),
            ))
        }
    }

    /// Runs the model with its scaling policies, on the engine of its flavour and mode.
    pub fn run(
        job: &Arc<Job>,
        req: &SimulationRequest,
        p: &Prepared,
        spd: &Spd,
        mon: &mut Monitor,
        interval: Duration,
    ) -> Result<RunEnd, String> {
        let mut log = JobLog {
            job,
            all: req.config.spd_log_all,
        };
        let mut adj = adjuster(&spd.quirks())?;
        let cfg = p.cfg.clone();
        let cm = &p.cm;
        let decisions = match &req.config.spd_decisions {
            Some(plan) => {
                if p.flavour == FlavourKind::SlingshotExact {
                    return Err(
                        "config.spd_decisions: scaling decisions run in the simulizar and slingshot flavours, not in slingshot-exact".into(),
                    );
                }
                check_actions(spd, plan)?;
                Some(Decisions {
                    plan: plan.iter().cloned().collect(),
                    resolved: Vec::new(),
                })
            }
            None => None,
        };
        if p.flavour == FlavourKind::SlingshotExact {
            if p.hpa.is_some() {
                return Err("HPA autoscalers run in the simulizar and slingshot flavours, not in slingshot-exact".into());
            }
            let run =
                SsxRun::new(cm, cfg.clone(), Outputs::default()).map_err(|e| e.to_string())?;
            let mut hooks = SsxSpd::new(spd, &cm.model, &mut adj, &mut log);
            let mut d = Ssx {
                run,
                hooks: Some(&mut hooks),
            };
            let end = drive(job, &mut d, mon, interval);
            let mut r = conclude(
                job,
                d,
                end,
                &simoxide_sim::stop::effective_config(&cfg, &cm.flavour),
                None,
            );
            r.spd = Some(serde_json::json!({
                "evaluations": hooks.state.evaluations,
                "requests": hooks.state.requests,
                "adjustments_ok": hooks.adjusted_ok,
            }));
            return Ok(r);
        }
        let hpa = match &p.hpa {
            Some(c) => Some(simoxide_spd::hpa::Hpa::new(c, spd)?),
            None => None,
        };
        let meta = req.config.runtime_metadata.then(|| MetaCtx {
            base: cm,
            model: simoxide_spd::inspect::model_file(cm, Some(spd), hpa.as_ref()),
        });
        let meta = meta.as_ref();
        match (cfg.mode, p.flavour) {
            #[cfg(feature = "exact")]
            (Mode::Exact, FlavourKind::SimuLizar) => {
                run_process_engine::<simoxide_sim::Exact, SimuLizar>(
                    job,
                    cm,
                    cfg,
                    spd,
                    hpa.as_ref(),
                    &mut adj,
                    &mut log,
                    decisions,
                    mon,
                    interval,
                    meta,
                )
            }
            #[cfg(feature = "exact")]
            (Mode::Exact, FlavourKind::Slingshot) => {
                run_process_engine::<simoxide_sim::Exact, Slingshot>(
                    job,
                    cm,
                    cfg,
                    spd,
                    hpa.as_ref(),
                    &mut adj,
                    &mut log,
                    decisions,
                    mon,
                    interval,
                    meta,
                )
            }
            (Mode::Fast, FlavourKind::SimuLizar) => {
                run_process_engine::<simoxide_sim::Fast, SimuLizar>(
                    job,
                    cm,
                    cfg,
                    spd,
                    hpa.as_ref(),
                    &mut adj,
                    &mut log,
                    decisions,
                    mon,
                    interval,
                    meta,
                )
            }
            (Mode::Fast, FlavourKind::Slingshot) => {
                run_process_engine::<simoxide_sim::Fast, Slingshot>(
                    job,
                    cm,
                    cfg,
                    spd,
                    hpa.as_ref(),
                    &mut adj,
                    &mut log,
                    decisions,
                    mon,
                    interval,
                    meta,
                )
            }
            #[cfg(not(feature = "exact"))]
            (m, _) => Err(m.unavailable_reason().unwrap_or_default()),
            #[allow(unreachable_patterns)]
            _ => Err("unsupported flavour".into()),
        }
    }

    /// Every action of `config.spd_decisions` names a policy (id or name) with a target group.
    fn check_actions(spd: &Spd, plan: &[Vec<String>]) -> Result<(), String> {
        let policies = &spd.model().policies;
        for (i, actions) in plan.iter().enumerate() {
            for a in actions {
                match policies.iter().find(|p| &*p.id == a || &*p.name == a) {
                    None => {
                        return Err(format!(
                            "config.spd_decisions[{i}]: no scaling policy named {a}"
                        ));
                    }
                    Some(p) if p.group.is_none() => {
                        return Err(format!(
                            "config.spd_decisions[{i}]: scaling policy {a} has no target group to adjust"
                        ));
                    }
                    Some(_) => {}
                }
            }
        }
        Ok(())
    }

    /// Runs a process engine with an SPD interpreter and the optional HPA autoscalers.
    #[allow(clippy::too_many_arguments)]
    fn run_process_engine<C: Compat, F: Flavour>(
        job: &Job,
        cm: &CompiledModel,
        cfg: SimConfig,
        spd: &Spd,
        hpa: Option<&simoxide_spd::hpa::Hpa>,
        adj: &mut dyn Adjuster<C, F>,
        log: &mut dyn Log,
        decisions: Option<Decisions>,
        mon: &mut Monitor,
        interval: Duration,
        meta: Option<&MetaCtx>,
    ) -> Result<RunEnd, String> {
        let sim = Simulation::<C, F>::create(cm, cfg.clone(), Outputs::default())
            .map_err(|e| e.to_string())?;
        let mut run = SpdRun::new(sim, spd).map_err(|e| e.to_string())?;
        if let Some(h) = hpa {
            run = run.with_hpa(h).map_err(|e| e.to_string())?;
        }
        run.set_pause_on_decision(decisions.is_some());
        let rec = meta.map(|_| {
            simoxide_sim::inspect::Recorder::start(
                simoxide_spd::inspect::deployment(&run.sim, Some(spd.model())),
                0,
                0,
                simoxide_spd::inspect::policy_state(&run),
            )
        });
        let mut d = WithSpd {
            run,
            adj,
            log,
            decisions,
            rec,
        };
        let end = drive(job, &mut d, mon, interval);
        Ok(conclude(
            job,
            d,
            end,
            &simoxide_sim::stop::effective_config(&cfg, &cm.flavour),
            meta,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An engine that is paused for good, with warnings and a recorded series.
    struct Paused {
        warnings: Vec<String>,
    }

    impl Drive for Paused {
        fn step(
            &mut self,
            _next: bool,
            _keep: &mut dyn FnMut(&Probe) -> bool,
        ) -> Result<Status, SimError> {
            Ok(Status::Paused)
        }
        fn probe(&self) -> Probe {
            Probe::default()
        }
        fn measurements(&self) -> &Measurements {
            unreachable!("conclude reads the probe and the warnings only")
        }
        fn into_measurements(self) -> Measurements {
            Measurements::default()
        }
        fn finish(self) -> (Result<RunResult, SimError>, Option<serde_json::Value>) {
            unreachable!("a stopped or failed run is not finished")
        }
        fn warnings(&self) -> Vec<String> {
            self.warnings.clone()
        }
    }

    /// Runs that the monitor stops and runs that fail keep the simulator's warnings.
    #[test]
    fn stopped_and_failed_runs_keep_the_warnings() {
        let job = Job::new("sim-1".into(), "test".into(), 100);
        let warning = "t=1.0: NullPointerException in SeffProgressed: ...".to_string();
        let engine = || Paused {
            warnings: vec![warning.clone()],
        };
        let stopped = conclude(&job, engine(), cancelled(), &SimConfig::default(), None);
        assert_eq!(stopped.status, JobStatus::Cancelled);
        assert_eq!(stopped.warnings, std::slice::from_ref(&warning));
        let error = SimError {
            message: "limit exceeded".into(),
            at_ns: 0,
            kind: SimErrorKind::Limit,
        };
        let failed = conclude(
            &job,
            engine(),
            End::Failed(error),
            &SimConfig::default(),
            None,
        );
        assert_eq!(failed.status, JobStatus::Failed);
        assert_eq!(failed.warnings, [warning]);
    }
}

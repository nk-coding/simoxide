//! `simoxide`: run a PCM model with simoxide-sim (flags mirror `reference/refsim run`), or benchmark it.

use simoxide_sim::{
    CompiledModel, FlavourConfig, FlavourKind, Mode, Outputs, Quirks, RngMode, RunResult, RunSpec,
    SimConfig, Tape,
};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

mod meta;

const USAGE: &str = "usage:
  simoxide run --model <dir> [--run-json run.json] [--seed N] [--max-sim-time T]
             [--max-measurements M] [--no-link-throughput] [--trace out.jsonl] [--tape out.jsonl]
             [--replay-tape tape.jsonl] [--check-origins] [--measurements out.csv] [--name NAME]
             [--max-events-per-instant N]   (livelock guard, default 20000000, 0 = off)
             [--max-steps N] [--max-events N] [--timeout SECONDS] [--max-stack-depth N]
             [--max-processes N]   (resource limits, see simoxide_sim::Limits; 0 = off)
             [--mode exact|fast]   (default fast: same semantics and statistics as the
             reference, other random numbers, no tape; exact: byte-identical to the
             reference, needs a build with `--features exact`, see LICENSES)
             [--ps-algorithm exact|virtual-time]   (virtual-time: O(log n) processor sharing,
             other float rounding; opt-in in every mode, see docs/correctness/deviations.md)
             [--flavour simulizar|slingshot|slingshot-exact]   (default simulizar;
             slingshot: Slingshot's semantics, fast mode; slingshot-exact: Slingshot's event
             cascade with every quirk, byte-equal to the det reference runner; --trace writes
             its level-E projection, --tape its tape projection; docs/spec/slingshot)
             [--quirks LIST]   (Slingshot compat flags: Q1..Q37, all, fast, none, -Qn;
             default none = the fixed behaviour; --quirks list prints them)
             [--spd-log spd.jsonl] [--spd-log-all] [--spd-quirks fixed|slingshot|Q2,Q12,...]
             [--no-spd]   (scaling policies of a .spd next to the model; `spd` feature; the
             adjustments run Slingshot's MainTransformation with the `reconfiguration`
             feature, QVTo ordered by default (--qvto-mode), else decisions only)
             [--hpa FILE.hpa.json] [--no-hpa]   (Kubernetes-style HPA autoscalers of SPD target
             groups: the *.hpa / *.hpa.json files next to the model, or FILE; their decisions
             go to --spd-log; `spd` feature, adjustments with `reconfiguration`;
             docs/guide/hpa.md)
             [--load-profile FILE.loadprofile.json] [--no-load-profile]   (a time-varying load:
             per usage scenario, steps of a factor on the arrival rate or the population; the
             *.loadprofile.json files next to the model, or FILE; docs/guide/load-profile.md)
             [--usage-streams]   (fast mode: the usage model draws from streams of its own, per
             scenario and per user, so models with the same usage model and seed see the same
             users: common random numbers; docs/spec/random.md RND-11)
             [--fix-double-resume]   (a synchronous fork resumes its parent only while it waits
             at the join: fixes SimuLizar's double resume, REF-2, which wakes the parent early
             from its next wait or aborts the run; docs/correctness/deviations.md)
             [--rules DIR] [--rules-log rules.jsonl] [--qvto-mode auto|ordered|fast|compat]
             (SimuLizar's reconfiguration rules: the .qvto files of DIR in file-name order, or
             run.json's reconfiguration_rules; `reconfiguration` feature; QVTo's compat mode
             needs a build with `exact` or `compat`)
             [--pause-at T | --pause-before T | --pause-events N] [--snapshot FILE]
             [--snapshot-format postcard|json]   (pause between events and write a snapshot
             of the run: `snapshot` feature, docs/guide/snapshots.md)
             [--pause-when COND]...   (pause when a condition holds, any of several: time T,
             before T, events N, elapsed T, elapsed-events N, METRIC[POINT] [mean W] OP V
             [for W], slo METRIC[POINT] upper U [lower L] [sensitivity S] [delay D])
             [--measurements-scope cumulative|interval]   (cumulative, the default: the
             snapshot keeps the measurement rows and --measurements of a resume repeats them;
             interval: --measurements gets the tuples recorded by this invocation only, the
             snapshot keeps no rows; docs/guide/snapshots.md#measurement-intervals)
             [--pause-on-decision] [--decision-file FILE.json]   (pause before a scaling
             policy's adjustment runs, the recommendation pending in the snapshot; resume it
             with --decide; the file reports the decision; docs/guide/spd.md#decisions)
             [--summary FILE.json] [--summary-series METRIC[POINT]]... [--summary-stats LIST]
             (statistics per series of the tuples this invocation recorded, its interval:
             count, sum, mean, min, max, first, last, initial, time_weighted (default all);
             series as in --pause-when, default all; docs/guide/snapshots.md#interval-summaries)
             [--model-metadata FILE.json] [--runtime-metadata FILE.json]   (the model's
             definitions with a content hash; the run's deployment at the start, its changes
             with their causes, policy state, retired elements and the metadata of every
             measurement series; docs/guide/runtime-metadata.md)
  simoxide resume FILE --model <dir> [--run-json run.json] [--trace out.jsonl] [--tape out.jsonl]
             [--measurements out.csv] [--pause-at T ... --snapshot FILE2] [--until T]
             [--pause-when COND]... [--spd-log ...] [--spd-quirks ...] [--rules DIR]
             [--rules-log ...] [--qvto-mode ...] [--timeout SECONDS]   (continue a snapshot on
             the model it was taken of: mode, flavour and quirks come from the snapshot; the
             trace, tape and logs get the continuation only, --measurements as the
             measurement scope says; --until T = --pause-at T)
             [--measurements-scope cumulative|interval]   (default: the snapshot's; cumulative:
             the rows of the run from time 0, interval: the tuples recorded since the restore)
             [--summary FILE.json ...]   (as for run: the tuples recorded since the restore)
             [--change FILE.changes]... [--transform X.qvto]...   (change the model before it
             continues, in the given order: a change file, a QVTo transformation run once;
             `reconfiguration` feature; --rules DIR with other rules than the snapshot's replaces
             them, on a snapshot without rules adds them)
             [--enforce POLICY,...]   (adjust by these scaling policies at the resume, past
             their triggers and constraints: Slingshot's incoming policies)
             [--decide ACTIONS]   (resolve the snapshot's pending scaling decision: the
             policies, by id or name, whose adjustments run in its place, in order: a JSON
             array or a comma list; '[]' declines; needed to resume a pending decision)
             [--hpa-set NAME.PARAMETER=VALUE]...   (change an HPA autoscaler's parameter at the
             resume: minReplicas, maxReplicas, averageUtilization, tolerance, startupDelay,
             scaleUpStabilization, scaleDownStabilization, downscaleStabilization; also `hpa
             NAME PARAMETER VALUE` lines of change files)
             [--model-metadata FILE.json] [--runtime-metadata FILE.json]   (as for run: the
             deployment at the restore, the changes of this invocation from --change on)
  simoxide explore FILE --model <dir> --variant NAME=FILE.changes|NAME=X.qvto|NAME ...
             [--until T] [--threads N] [--metrics rt,util,...]   (fork the snapshot into one
             run per variant, each with its changes, to the end or T; compare their measurements;
             `reconfiguration` feature)
  simoxide snapshot-info FILE   (the snapshot's header)
  simoxide bench --model <dir> [--run-json run.json] [--runs N] [--threads T] [same run flags]
  simoxide load-bench --model <dir> [--run-json run.json] [--runs N]   (load + compile only)
  (with the `profile` feature, bench and load-bench take --profile out.svg|out.folded)";

#[derive(Clone)]
struct Args {
    cmd: String,
    model: Option<PathBuf>,
    run_json: Option<PathBuf>,
    seed: Option<i64>,
    max_sim_time: Option<i64>,
    max_meas: Option<i64>,
    no_link_throughput: bool,
    max_events_per_instant: Option<u64>,
    limits: simoxide_sim::Limits,
    timeout: Option<f64>,
    trace: Option<PathBuf>,
    tape: Option<PathBuf>,
    replay: Option<PathBuf>,
    check_origins: bool,
    measurements: Option<PathBuf>,
    name: Option<String>,
    runs: usize,
    threads: usize,
    threads_given: bool,
    profile: Option<PathBuf>,
    mode: Mode,
    ps_virtual_time: bool,
    flavour: FlavourKind,
    quirks: Quirks,
    spd_log: Option<PathBuf>,
    spd_log_all: bool,
    spd_quirks: Option<String>,
    no_spd: bool,
    /// `--hpa FILE`, `--no-hpa`, `--hpa-set NAME.KEY=VALUE`.
    hpa: Option<PathBuf>,
    no_hpa: bool,
    hpa_set: Vec<String>,
    /// `--load-profile FILE`, `--no-load-profile`.
    load_profile: Option<PathBuf>,
    no_load_profile: bool,
    usage_streams: bool,
    fix_double_resume: bool,
    rules: Option<PathBuf>,
    rules_log: Option<PathBuf>,
    qvto_mode: Option<String>,
    /// Where to pause (`--pause-at`, `--pause-before`, `--pause-events`, `--until`).
    pause: Option<simoxide_sim::Pause>,
    snapshot: Option<PathBuf>,
    snapshot_json: bool,
    /// The snapshot to continue (`resume FILE`, `snapshot-info FILE`).
    file: Option<PathBuf>,
    /// `--pause-when` conditions.
    pause_when: Vec<simoxide_sim::Condition>,
    /// `--change` and `--transform`, in order.
    changes: Vec<ChangeArg>,
    /// `--enforce` policies.
    enforce: Vec<String>,
    /// `--pause-on-decision`, `--decide ACTIONS`, `--decision-file FILE`.
    pause_on_decision: bool,
    decide: Option<Vec<String>>,
    decision_file: Option<PathBuf>,
    /// `explore`: `--variant NAME=FILE`, `--metrics`.
    variants: Vec<(String, Option<PathBuf>)>,
    metrics: Vec<String>,
    /// `--measurements-scope`: `Some(true)` interval, `Some(false)` cumulative.
    measurements_scope: Option<bool>,
    /// Resolved: this invocation's measurements are of its interval (docs/guide/snapshots.md,
    /// "Measurement intervals"): the restored rows are dropped, the snapshot holds none.
    interval: bool,
    /// `resume`: the state id and the measurement cursor of the snapshot resumed.
    parent: Option<(String, Option<u64>)>,
    /// `resume`: the simulation time of the snapshot resumed (ns; 0 for `run`).
    start_ns: i64,
    /// `--summary FILE`, `--summary-series SEL`, `--summary-stats LIST`.
    summary: Option<PathBuf>,
    summary_series: Vec<simoxide_sim::sim::watch::SeriesSel>,
    summary_stats: Option<Vec<simoxide_sim::summary::Stat>>,
    /// `--model-metadata FILE`, `--runtime-metadata FILE` (docs/guide/runtime-metadata.md).
    model_metadata: Option<PathBuf>,
    runtime_metadata: Option<PathBuf>,
    /// Resolved: the hash of the model file, for the runtime file.
    model_hash: Option<String>,
}

/// A change step of `resume` (`--change`, `--transform`).
#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "reconfiguration"), allow(dead_code))]
enum ChangeArg {
    File(PathBuf),
    Transform(PathBuf),
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let cmd = it.next().ok_or(USAGE)?;
    let mut a = Args {
        cmd,
        model: None,
        run_json: None,
        seed: None,
        max_sim_time: None,
        max_meas: None,
        no_link_throughput: false,
        max_events_per_instant: None,
        limits: simoxide_sim::Limits::default(),
        timeout: None,
        trace: None,
        tape: None,
        replay: None,
        check_origins: false,
        measurements: None,
        name: None,
        runs: 20,
        threads: 1,
        threads_given: false,
        profile: None,
        mode: Mode::Fast,
        ps_virtual_time: false,
        flavour: FlavourKind::SimuLizar,
        quirks: Quirks::NONE,
        spd_log: None,
        spd_log_all: false,
        spd_quirks: None,
        no_spd: false,
        hpa: None,
        no_hpa: false,
        hpa_set: Vec::new(),
        load_profile: None,
        no_load_profile: false,
        usage_streams: false,
        fix_double_resume: false,
        rules: None,
        rules_log: None,
        qvto_mode: None,
        pause: None,
        snapshot: None,
        snapshot_json: false,
        file: None,
        pause_when: Vec::new(),
        changes: Vec::new(),
        enforce: Vec::new(),
        pause_on_decision: false,
        decide: None,
        decision_file: None,
        variants: Vec::new(),
        metrics: Vec::new(),
        measurements_scope: None,
        interval: false,
        parent: None,
        start_ns: 0,
        summary: None,
        summary_series: Vec::new(),
        summary_stats: None,
        model_metadata: None,
        runtime_metadata: None,
        model_hash: None,
    };
    let secs = |f: &str, v: String| -> Result<i64, String> {
        let t: f64 = v.parse().map_err(|e| format!("{f}: {e}"))?;
        if !t.is_finite() || t < 0.0 {
            return Err(format!("{f}: expected a time in seconds, got {v}"));
        }
        Ok((t * 1e9).round() as i64)
    };
    while let Some(f) = it.next() {
        if a.file.is_none()
            && matches!(a.cmd.as_str(), "resume" | "snapshot-info" | "explore")
            && !f.starts_with('-')
        {
            a.file = Some(f.into());
            continue;
        }
        let mut val = || it.next().ok_or(format!("{f} needs a value"));
        match f.as_str() {
            "--model" => a.model = Some(val()?.into()),
            "--run-json" => a.run_json = Some(val()?.into()),
            "--seed" => a.seed = Some(val()?.parse().map_err(|e| format!("--seed: {e}"))?),
            "--max-sim-time" => {
                a.max_sim_time = Some(val()?.parse().map_err(|e| format!("--max-sim-time: {e}"))?)
            }
            "--max-measurements" => {
                a.max_meas = Some(
                    val()?
                        .parse()
                        .map_err(|e| format!("--max-measurements: {e}"))?,
                )
            }
            "--no-link-throughput" => a.no_link_throughput = true,
            "--max-events-per-instant" => {
                a.max_events_per_instant = Some(
                    val()?
                        .parse()
                        .map_err(|e| format!("--max-events-per-instant: {e}"))?,
                )
            }
            "--max-steps" => {
                a.limits.max_steps = val()?.parse().map_err(|e| format!("{f}: {e}"))?
            }
            "--max-events" => {
                a.limits.max_events = val()?.parse().map_err(|e| format!("{f}: {e}"))?
            }
            "--max-stack-depth" => {
                a.limits.max_stack_depth = val()?.parse().map_err(|e| format!("{f}: {e}"))?
            }
            "--max-processes" => {
                a.limits.max_processes = val()?.parse().map_err(|e| format!("{f}: {e}"))?
            }
            "--timeout" => a.timeout = Some(val()?.parse().map_err(|e| format!("{f}: {e}"))?),
            "--trace" => a.trace = Some(val()?.into()),
            "--tape" => a.tape = Some(val()?.into()),
            "--replay-tape" => {
                let v = val()?;
                a.replay = (!v.is_empty()).then(|| v.into());
            }
            "--check-origins" => a.check_origins = true,
            "--measurements" => a.measurements = Some(val()?.into()),
            "--name" => a.name = Some(val()?),
            "--runs" => a.runs = val()?.parse().map_err(|e| format!("--runs: {e}"))?,
            "--profile" => a.profile = Some(val()?.into()),
            "--threads" => {
                a.threads = val()?.parse().map_err(|e| format!("--threads: {e}"))?;
                a.threads_given = true;
            }
            "--ps-algorithm" => {
                a.ps_virtual_time = match val()?.as_str() {
                    "exact" => false,
                    "virtual-time" => true,
                    v => {
                        return Err(format!(
                            "--ps-algorithm: expected exact or virtual-time, got {v}"
                        ));
                    }
                }
            }
            "--mode" => {
                let v = val()?;
                a.mode =
                    Mode::parse(&v).ok_or(format!("--mode: expected exact or fast, got {v}"))?;
                if let Some(why) = a.mode.unavailable_reason() {
                    return Err(format!("--mode {v}: {why}"));
                }
            }
            "--flavour" => {
                let v = val()?;
                a.flavour = FlavourKind::parse(&v).ok_or(format!(
                    "--flavour: expected simulizar or slingshot, got {v}"
                ))?;
            }
            "--quirks" => {
                let v = val()?;
                if v == "list" {
                    let mut s = String::from("Slingshot quirks (docs/spec/slingshot/quirks.md):\n");
                    for q in simoxide_sim::Quirk::ALL {
                        s.push_str(&format!(
                            "  {q:<4} {:<10} {}\n",
                            format!("{:?}", q.support()),
                            q.summary()
                        ));
                    }
                    return Err(s);
                }
                a.quirks = Quirks::parse(&v).map_err(|e| format!("--quirks: {e}"))?;
            }
            "--spd-log" => a.spd_log = Some(val()?.into()),
            "--spd-log-all" => a.spd_log_all = true,
            "--spd-quirks" => a.spd_quirks = Some(val()?),
            "--no-spd" => a.no_spd = true,
            "--hpa" => a.hpa = Some(val()?.into()),
            "--no-hpa" => a.no_hpa = true,
            "--hpa-set" => a.hpa_set.push(val()?),
            "--load-profile" => a.load_profile = Some(val()?.into()),
            "--no-load-profile" => a.no_load_profile = true,
            "--usage-streams" => a.usage_streams = true,
            "--fix-double-resume" => a.fix_double_resume = true,
            "--rules" => a.rules = Some(val()?.into()),
            "--rules-log" => a.rules_log = Some(val()?.into()),
            "--qvto-mode" => a.qvto_mode = Some(val()?),
            "--pause-at" | "--until" => {
                a.pause = Some(simoxide_sim::Pause::After(secs(&f, val()?)?))
            }
            "--pause-before" => a.pause = Some(simoxide_sim::Pause::Before(secs(&f, val()?)?)),
            "--pause-events" => {
                a.pause = Some(simoxide_sim::Pause::Events(
                    val()?.parse().map_err(|e| format!("{f}: {e}"))?,
                ))
            }
            "--snapshot" => a.snapshot = Some(val()?.into()),
            "--pause-when" => a
                .pause_when
                .push(simoxide_sim::sim::watch::parse_condition(&val()?)?),
            "--change" => a.changes.push(ChangeArg::File(val()?.into())),
            "--transform" => a.changes.push(ChangeArg::Transform(val()?.into())),
            "--enforce" => a.enforce.extend(
                val()?
                    .split(',')
                    .map(str::trim)
                    .filter(|x| !x.is_empty())
                    .map(String::from),
            ),
            "--pause-on-decision" => a.pause_on_decision = true,
            "--decide" => a.decide = Some(decide_actions(&val()?)?),
            "--decision-file" => a.decision_file = Some(val()?.into()),
            "--model-metadata" => a.model_metadata = Some(val()?.into()),
            "--runtime-metadata" => a.runtime_metadata = Some(val()?.into()),
            "--variant" => {
                let v = val()?;
                a.variants.push(match v.split_once('=') {
                    Some((n, f)) => (n.to_string(), Some(f.into())),
                    None => (v, None),
                });
            }
            "--metrics" => a
                .metrics
                .extend(val()?.split(',').map(|m| m.trim().to_string())),
            "--measurements-scope" => {
                a.measurements_scope = match val()?.as_str() {
                    "cumulative" => Some(false),
                    "interval" => Some(true),
                    v => return Err(format!("{f}: expected cumulative or interval, got {v}")),
                }
            }
            "--summary" => a.summary = Some(val()?.into()),
            "--summary-series" => a
                .summary_series
                .push(simoxide_sim::sim::watch::parse_series_sel(&val()?)?),
            "--summary-stats" => {
                a.summary_stats = Some(
                    simoxide_sim::summary::Stat::parse_list(&val()?)
                        .map_err(|e| format!("{f}: {e}"))?,
                )
            }
            "--snapshot-format" => {
                a.snapshot_json = match val()?.as_str() {
                    "postcard" => false,
                    "json" => true,
                    v => return Err(format!("{f}: expected postcard or json, got {v}")),
                }
            }
            "-h" | "--help" => return Err(USAGE.into()),
            o => return Err(format!("unknown flag {o}\n{USAGE}")),
        }
    }
    if a.summary.is_none() && (!a.summary_series.is_empty() || a.summary_stats.is_some()) {
        return Err("--summary-series and --summary-stats need --summary FILE".into());
    }
    Ok(a)
}

/// The actions of `--decide`.
#[cfg(feature = "spd")]
fn decide_actions(v: &str) -> Result<Vec<String>, String> {
    simoxide_spd::decision::parse_actions(v).map_err(|e| format!("--decide: {e}"))
}

#[cfg(not(feature = "spd"))]
fn decide_actions(_: &str) -> Result<Vec<String>, String> {
    Err("--decide needs the `spd` feature".into())
}

/// The options of scaling decisions (`--pause-on-decision`, `--decide`, `--decision-file`)
/// that need a run with scaling policies.
fn decision_options(a: &Args) -> bool {
    a.pause_on_decision || a.decide.is_some() || a.decision_file.is_some()
}

fn setup(a: &Args) -> Result<(CompiledModel, SimConfig), String> {
    setup_with_rules(a).map(|(cm, cfg, _)| (cm, cfg))
}

/// The model directory (`--model`, else the folder of `--run-json`).
fn model_dir(a: &Args) -> Result<PathBuf, String> {
    let run_json = a.run_json.clone().or_else(|| {
        let p = a.model.as_ref()?.join("run.json");
        p.exists().then_some(p)
    });
    match (&a.model, &run_json) {
        (Some(m), _) => Ok(m.clone()),
        (None, Some(r)) => Ok(r.parent().unwrap_or(Path::new(".")).to_path_buf()),
        _ => Err(USAGE.into()),
    }
}

/// [`setup`] and the reconfiguration rules folder (`--rules`, else run.json's).
fn setup_with_rules(a: &Args) -> Result<(CompiledModel, SimConfig, Option<PathBuf>), String> {
    let run_json = a.run_json.clone().or_else(|| {
        let p = a.model.as_ref()?.join("run.json");
        p.exists().then_some(p)
    });
    let dir = model_dir(a)?;
    let spec = match &run_json {
        Some(p) => RunSpec::load(p)?,
        None => RunSpec::default(),
    };
    #[allow(unused_mut)]
    let mut model = spec.load_model(&dir)?;
    // the HPA autoscalers' policies join the SPD before the model is compiled
    #[cfg(feature = "spd")]
    if let Some(c) = spd::hpa_config(a, &dir)? {
        simoxide_spd::hpa::attach(&mut model, &c)?;
    }
    #[cfg(not(feature = "spd"))]
    if a.hpa.is_some() {
        return Err("--hpa needs the `spd` feature".into());
    }
    let rules = a
        .rules
        .clone()
        .or_else(|| spec.reconfiguration_rules.as_ref().map(|r| dir.join(r)));
    let flavour = match a.flavour {
        FlavourKind::SimuLizar if !a.quirks.is_empty() => {
            return Err("--quirks needs --flavour slingshot".into());
        }
        FlavourKind::SimuLizar => FlavourConfig::SIMULIZAR,
        FlavourKind::Slingshot => FlavourConfig::slingshot(a.quirks),
        // the exact mode runs every quirk (U5)
        FlavourKind::SlingshotExact if !a.quirks.is_empty() && a.quirks != Quirks::all() => {
            return Err("--flavour slingshot-exact runs with all quirks (--quirks all)".into());
        }
        FlavourKind::SlingshotExact => FlavourConfig::slingshot_exact(),
    };
    let cm = CompiledModel::compile_flavour(model, flavour).map_err(|e| e.to_string())?;
    let name = a.name.clone().unwrap_or_else(|| {
        std::fs::canonicalize(&dir)
            .ok()
            .and_then(|d| d.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default()
    });
    let mut cfg = spec.sim_config(&name);
    if let Some(s) = a.seed {
        cfg.seed = s;
    }
    if let Some(t) = a.max_sim_time {
        cfg.max_sim_time = t;
    }
    if let Some(m) = a.max_meas {
        cfg.max_measurements = m;
    }
    if a.no_link_throughput {
        cfg.simulate_throughput_of_linking_resources = false;
    }
    if let Some(n) = a.max_events_per_instant {
        cfg.max_events_per_instant = n;
    }
    cfg.limits = a.limits.clone();
    cfg.mode = a.mode;
    if a.ps_virtual_time {
        cfg.ps_algorithm = simoxide_sim::PsAlgorithm::VirtualTime;
    }
    if let Some(s) = a.timeout {
        let d = std::time::Duration::try_from_secs_f64(s).map_err(|e| format!("--timeout: {e}"))?;
        cfg.limits.deadline = Some(Instant::now() + d);
    }
    if let Some(r) = &a.replay {
        let text = std::fs::read_to_string(r).map_err(|e| format!("{}: {e}", r.display()))?;
        cfg.rng = RngMode::Replay(Arc::new(Tape::parse(&text)?));
        cfg.check_tape_origins = a.check_origins;
    }
    // the load profile: --load-profile FILE, else the directory's *.loadprofile.json files
    if !a.no_load_profile {
        let p = match &a.load_profile {
            Some(f) => Some(simoxide_sim::LoadProfile::load_file(f)?),
            None => simoxide_sim::LoadProfile::load_dir(&dir)?,
        };
        cfg.load_profile = p.map(Arc::new);
    }
    cfg.usage_streams = a.usage_streams;
    cfg.fix_double_resume = a.fix_double_resume;
    Ok((cm, cfg, rules))
}

fn create(p: &Path) -> Result<Box<dyn std::io::Write>, String> {
    let f = std::fs::File::create(p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(Box::new(BufWriter::new(f)))
}

fn run(a: &Args) -> Result<(), String> {
    if a.snapshot.is_some() && a.pause.is_none() && a.pause_when.is_empty() && !a.pause_on_decision
    {
        return Err(
            "--snapshot needs a pause (--pause-at, --pause-before, --pause-events, --pause-when, --pause-on-decision)"
                .into(),
        );
    }
    if !a.changes.is_empty() || !a.enforce.is_empty() || !a.hpa_set.is_empty() || a.decide.is_some()
    {
        return Err(
            "--change, --transform, --enforce, --decide and --hpa-set are options of `resume`"
                .into(),
        );
    }
    let mut a2 = a.clone();
    a2.interval = a.measurements_scope == Some(true);
    let (cm, cfg, rules_dir) = setup_with_rules(&a2)?;
    for w in &cm.warnings {
        eprintln!("warning: {w}");
    }
    let out = Outputs {
        trace: a2.trace.as_deref().map(create).transpose()?,
        tape: a2.tape.as_deref().map(create).transpose()?,
    };
    let t0 = Instant::now();
    let spd = spd::of(&a2, &cm)?;
    a2.model_hash = model_metadata(&a2, &cm, spd.as_ref())?;
    let a = &a2;
    if decision_options(a) && spd.is_none() {
        return Err(
            "--pause-on-decision and --decision-file need a model with scaling policies (.spd)"
                .into(),
        );
    }
    if let Some(dir) = rules_dir {
        if spd.is_some() {
            return Err("reconfiguration rules and scaling policies in one run are not supported (use --no-spd)".into());
        }
        if a.flavour.is_slingshot() {
            return Err("reconfiguration rules run in the SimuLizar flavour".into());
        }
        let t0 = Instant::now();
        let r = rules::run(a, &cm, cfg, out, &dir, None)?;
        return report_end(a, &cm, r, t0);
    }
    let r = match spd {
        Some(s) if a.flavour == simoxide_sim::FlavourKind::SlingshotExact => {
            spd::run_exact(a, &cm, Ok(cfg), out, &s)?
        }
        Some(s) => spd::run(a, &cm, cfg, out, &s, None)?,
        None if a.flavour == simoxide_sim::FlavourKind::SlingshotExact
            && (a.pause.is_some() || a.snapshot.is_some() || !a.pause_when.is_empty()) =>
        {
            ssxrun::plain(a, &cm, Ok(cfg), out)?
        }
        None if a.pause.is_some()
            || a.snapshot.is_some()
            || !a.pause_when.is_empty()
            || a.runtime_metadata.is_some() =>
        {
            snap::plain(a, &cm, Ok(cfg), out)?
        }
        None => End::Finished(simoxide_sim::run(&cm, cfg, out).map_err(|e| e.to_string())?),
    };
    report_end(a, &cm, r, t0)
}

/// Writes `--model-metadata` and returns the model file's hash for `--runtime-metadata` (with
/// the scaling definitions of `s` and its HPA autoscalers).
#[cfg(feature = "spd")]
fn model_metadata(
    a: &Args,
    cm: &CompiledModel,
    s: Option<&simoxide_spd::Spd>,
) -> Result<Option<String>, String> {
    if a.model_metadata.is_none() && a.runtime_metadata.is_none() {
        return Ok(None);
    }
    let hpa = match s {
        Some(s) => spd::hpa_of(a, s)?,
        None => None,
    };
    meta::write_model(a, || meta::model_file(cm, s, hpa.as_ref()))
}

#[cfg(not(feature = "spd"))]
fn model_metadata<T>(
    a: &Args,
    cm: &CompiledModel,
    _: Option<&T>,
) -> Result<Option<String>, String> {
    meta::write_model(a, || meta::model_file(cm))
}

/// `--runtime-metadata` is not supported by the Slingshot-exact engine.
fn no_runtime_metadata(a: &Args) -> Result<(), String> {
    match a.runtime_metadata {
        Some(_) => Err(
            "--runtime-metadata is not supported by the Slingshot-exact engine (use --flavour slingshot)"
                .into(),
        ),
        None => Ok(()),
    }
}

/// How a run (or a resumed one) ended: at its end, or paused with a snapshot written.
enum End {
    Finished(RunResult),
    Paused {
        now_ns: i64,
        /// The same in seconds (the engine's clock).
        now_s: f64,
        events: u64,
        measurements: simoxide_sim::Measurements,
        /// The snapshot written: path, bytes, state id.
        snapshot: Option<(PathBuf, u64, String)>,
        /// The pause condition that held (`--pause-when`).
        reason: Option<String>,
    },
}

/// The first tuple number of this invocation's interval (`None`: unknown).
fn interval_from(a: &Args, m: &simoxide_sim::Measurements) -> Option<u64> {
    match &a.parent {
        Some((_, Some(n))) => Some(*n),
        Some((_, None)) => a.interval.then(|| m.count - m.rows_held()),
        None => Some(0),
    }
}

/// Writes `--summary`: the interval summary of `m` (the run's measurements at the pause or the
/// end, `to_ns`), with the state id of the snapshot written, if any
/// (docs/guide/snapshots.md#interval-summaries).
fn write_summary(
    a: &Args,
    cm: &CompiledModel,
    m: &simoxide_sim::Measurements,
    (to, to_ns): (f64, i64),
    state: Option<&str>,
) -> Result<(), String> {
    use simoxide_sim::sim::watch::{ids_named, selects};
    use simoxide_sim::summary::{self, Interval, Stat};
    let Some(p) = &a.summary else {
        return Ok(());
    };
    // element names are looked up in the model of --model
    let sels: Vec<_> = a
        .summary_series
        .iter()
        .map(|s| (s, ids_named(&cm.model.graph, &s.point)))
        .collect();
    for (s, ids) in &sels {
        if !m.series.iter().any(|d| selects(s, ids, d)) {
            eprintln!(
                "warning: --summary-series {}[{}] selects no series",
                s.metric, s.point
            );
        }
    }
    let select = |d: &simoxide_sim::ir::SeriesDef| {
        sels.is_empty() || sels.iter().any(|(s, ids)| selects(s, ids, d))
    };
    let iv = Interval {
        tuples_from: interval_from(a, m),
        tuples_to: m.count,
        from: m.start_time,
        to,
        from_ns: a.start_ns,
        to_ns,
    };
    let stats = a.summary_stats.as_deref().unwrap_or(&Stat::ALL);
    let mut doc = summary::document(m, &iv, &select, stats);
    if let Some(o) = doc.as_object_mut() {
        if let Some((parent, _)) = &a.parent {
            o.insert("parent".into(), parent.as_str().into());
        }
        if let Some(s) = state {
            o.insert("state".into(), s.into());
        }
    }
    let mut text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    text.push('\n');
    std::fs::write(p, text).map_err(|e| format!("{}: {e}", p.display()))
}

/// Reports a run that ended or paused.
fn report_end(a: &Args, cm: &CompiledModel, r: End, t0: Instant) -> Result<(), String> {
    // the measurement interval of a pause, a resume or a run with --measurements-scope
    let interval = |m: &simoxide_sim::Measurements, paused: bool, state: Option<&str>| {
        if a.cmd != "resume" && a.measurements_scope.is_none() && !paused {
            return;
        }
        let held = m.rows_held();
        let from = interval_from(a, m);
        let mut line = format!(
            "measurements: scope={} tuples={}..{} rows={held}",
            if a.interval { "interval" } else { "cumulative" },
            from.map_or("?".to_string(), |n| n.to_string()),
            m.count
        );
        if let Some((p, _)) = &a.parent {
            line.push_str(&format!(" parent={p}"));
        }
        if let Some(s) = state {
            line.push_str(&format!(" state={s}"));
        }
        eprintln!("{line}");
    };
    match r {
        End::Finished(r) => {
            interval(&r.measurements, false, None);
            write_summary(a, cm, &r.measurements, (r.end_secs, r.end_ns), None)?;
            report(a, r, t0)
        }
        End::Paused {
            now_ns,
            now_s,
            events,
            measurements,
            snapshot,
            reason,
        } => {
            if let Some(r) = reason {
                eprintln!("pause condition: {r}");
            }
            if let Some(p) = &a.measurements {
                let mut f = create(p)?;
                measurements
                    .write_csv(&mut f)
                    .map_err(|e| format!("{}: {e}", p.display()))?;
            }
            let state = snapshot.as_ref().map(|s| s.2.as_str());
            interval(&measurements, true, state);
            write_summary(a, cm, &measurements, (now_s, now_ns), state)?;
            let snap = match snapshot {
                Some((p, n, _)) => format!(" snapshot={} ({n} bytes)", p.display()),
                None => String::new(),
            };
            eprintln!(
                "paused t={} events={} measurements={}{snap} wall={:.3}ms",
                now_ns as f64 / 1e9,
                events,
                measurements.count,
                t0.elapsed().as_secs_f64() * 1e3
            );
            Ok(())
        }
    }
}

/// Writes the measurements and the summary line of a run.
fn report(a: &Args, r: RunResult, t0: Instant) -> Result<(), String> {
    let dt = t0.elapsed();
    for w in &r.warnings {
        eprintln!("warning: {w}");
    }
    if let Some(p) = &a.measurements {
        let mut f = create(p)?;
        r.measurements
            .write_csv(&mut f)
            .map_err(|e| format!("{}: {e}", p.display()))?;
    }
    eprintln!(
        "t_end={} events={} uniforms={} measurements={} main_count={} wall={:.3}ms",
        r.end_ns as f64 / 1e9,
        r.events,
        r.uniforms,
        r.measurements.count,
        r.main_count,
        dt.as_secs_f64() * 1e3
    );
    Ok(())
}

/// Scaling policies (`spd` feature): the interpreter runs with the simulation when the model
/// directory has a `.spd` file.
#[cfg(feature = "spd")]
mod spd {
    use super::{Args, End, create};
    use simoxide_sim::compat::Compat;
    use simoxide_sim::snapshot::Snapshot;
    use simoxide_sim::{CompiledModel, Flavour, Mode, Outputs, SimConfig, Simulation, Status};
    use simoxide_spd::decision;
    use simoxide_spd::hpa::{Hpa, HpaConfig};
    use simoxide_spd::log::JsonlLog;
    #[cfg(not(feature = "reconfiguration"))]
    use simoxide_spd::sim::NoAdjustment;
    use simoxide_spd::sim::{SpdRun, spd_of};
    use simoxide_spd::{Quirks, Spd};

    pub fn of(a: &Args, cm: &CompiledModel) -> Result<Option<Spd>, String> {
        of_with(a, cm, None)
    }

    /// The HPA configuration of the run: `--hpa FILE`, else the model directory's `*.hpa` and
    /// `*.hpa.json` files (none with `--no-hpa`).
    pub fn hpa_config(a: &Args, dir: &std::path::Path) -> Result<Option<HpaConfig>, String> {
        if a.no_hpa {
            return Ok(None);
        }
        let c = match &a.hpa {
            Some(f) => Some(HpaConfig::load_file(f)?),
            None => HpaConfig::load_dir(dir)?,
        };
        match c {
            Some(c) if !c.autoscalers.is_empty() => {
                if a.no_spd {
                    return Err(
                        "the HPA autoscalers scale SPD target groups: --no-spd needs --no-hpa"
                            .into(),
                    );
                }
                Ok(Some(c))
            }
            _ => Ok(None),
        }
    }

    /// The run's autoscalers, resolved against its SPD interpreter.
    pub fn hpa_of(a: &Args, spd: &Spd) -> Result<Option<Hpa>, String> {
        let dir = super::model_dir(a)?;
        hpa_config(a, &dir)?.map(|c| Hpa::new(&c, spd)).transpose()
    }

    /// `--hpa-set NAME.KEY=VALUE`.
    fn hpa_sets(a: &Args) -> Result<Vec<(String, String, f64)>, String> {
        a.hpa_set
            .iter()
            .map(|s| {
                let bad = || format!("--hpa-set {s}: expected NAME.PARAMETER=VALUE");
                let (lhs, v) = s.split_once('=').ok_or_else(bad)?;
                let (n, k) = lhs.rsplit_once('.').ok_or_else(bad)?;
                let v: f64 = v.trim().parse().map_err(|_| bad())?;
                Ok((n.to_string(), k.to_string(), v))
            })
            .collect()
    }

    /// Prints the autoscalers' summaries.
    fn hpa_report<C: Compat, F: Flavour>(r: &SpdRun<'_, '_, C, F>) {
        for s in r.hpa_summary() {
            eprintln!("{}", s.line());
        }
    }

    /// The interpreter of a resumed run: built with the snapshot's compat flags (unless
    /// `--spd-quirks` says otherwise; restoring then checks they agree).
    pub fn of_snapshot(a: &Args, cm: &CompiledModel, s: &Snapshot) -> Result<Option<Spd>, String> {
        let q = simoxide_spd::snapshot::quirks_of(s).map_err(|e| e.to_string())?;
        of_with(a, cm, q)
    }

    fn of_with(a: &Args, cm: &CompiledModel, q: Option<Quirks>) -> Result<Option<Spd>, String> {
        if a.no_spd {
            return Ok(None);
        }
        let exact = a.flavour == simoxide_sim::FlavourKind::SlingshotExact;
        let q = match (&a.spd_quirks, q) {
            (Some(s), _) => Quirks::parse(s)?,
            (None, Some(q)) => q,
            // the Slingshot-exact mode is Slingshot with every quirk
            (None, None) if exact => Quirks::slingshot(),
            (None, None) => Quirks::fixed(),
        };
        if exact && q != Quirks::slingshot() {
            return Err(
                "--flavour slingshot-exact runs the SPD with all quirks (--spd-quirks slingshot)"
                    .into(),
            );
        }
        let s = spd_of(&cm.model, q).map_err(|e| e.to_string())?;
        if let Some(s) = &s
            && let Some(d) = s.dropped()
        {
            eprintln!("warning: SPD dropped (Q11 compat, as Slingshot): {d}");
        }
        Ok(s)
    }

    fn go<C: Compat, F: Flavour>(
        a: &Args,
        cm: &CompiledModel,
        cfg: SimConfig,
        out: Outputs,
        spd: &Spd,
        restore: Option<&Snapshot>,
    ) -> Result<End, String> {
        #[cfg(not(feature = "reconfiguration"))]
        let mut carry = super::changes::Carry::none();
        #[cfg(feature = "reconfiguration")]
        let mut adj = super::changes::spd_adjuster(a, &spd.quirks())?;
        #[cfg(not(feature = "reconfiguration"))]
        let mut adj = NoAdjustment;
        let hpa = hpa_of(a, spd)?;
        let mut sets = Vec::new();
        let groups = Some(spd.model());
        let mut rec;
        let mut r = match restore {
            Some(snap) => {
                let mut r = SpdRun::<C, F>::restore_snapshot_hpa(snap, cm, spd, hpa.as_ref(), out)
                    .map_err(|e| e.to_string())?;
                if a.interval {
                    r.sim.take_measurement_rows();
                }
                rec = super::meta::begin(a, &r.sim, groups, || policy_state(&r));
                match (r.pending_decision(), &a.decide) {
                    (Some(d), None) => {
                        let p = d.policies();
                        return Err(format!(
                            "the snapshot has a pending scaling decision (decision {} at t={} s, recommended: [{}]): resume it with --decide ACTIONS (the recommendation: --decide {}; decline it: --decide '[]')",
                            d.decision,
                            d.time_ns as f64 / 1e9,
                            p.join(", "),
                            p.join(",")
                        ));
                    }
                    (None, Some(_)) => {
                        return Err("--decide: the snapshot has no pending scaling decision".into());
                    }
                    _ => {}
                }
                #[cfg(feature = "reconfiguration")]
                sets.extend(super::changes::hpa_settings(a)?);
                sets.extend(hpa_sets(a)?);
                #[cfg(feature = "reconfiguration")]
                {
                    adj.run
                        .restore_section(&r.sim, snap)
                        .map_err(|e| e.to_string())?;
                    if super::changes::with_qvto(a, &mut r.sim, &mut adj.run, rec.as_mut(), groups)?
                    {
                        r.model_changed();
                    }
                }
                #[cfg(not(feature = "reconfiguration"))]
                {
                    carry = super::changes::plain(a, &mut r.sim, Some(snap), rec.as_mut())?;
                    if carry.changed {
                        r.model_changed();
                    }
                }
                if !a.enforce.is_empty() {
                    let names: Vec<&str> = a.enforce.iter().map(String::as_str).collect();
                    r.enforce_policies(&names).map_err(|e| e.to_string())?;
                }
                if !sets.is_empty() && hpa.is_none() {
                    return Err("HPA parameters (--hpa-set, `hpa` change lines): the run has no HPA autoscaler".into());
                }
                for (n, k, v) in &sets {
                    r.hpa_set(n, k, *v)
                        .map_err(|e| format!("HPA {n}.{k}: {e}"))?;
                    eprintln!("hpa {n}: {k} = {v}");
                }
                r
            }
            None => {
                let sim = Simulation::<C, F>::create(cm, cfg, out).map_err(|e| e.to_string())?;
                let r = SpdRun::new(sim, spd).map_err(|e| e.to_string())?;
                let r = match &hpa {
                    Some(h) => r.with_hpa(h).map_err(|e| e.to_string())?,
                    None => r,
                };
                rec = super::meta::begin(a, &r.sim, groups, || policy_state(&r));
                r
            }
        };
        // with --runtime-metadata every adjustment is recorded
        let mut recording;
        let adjuster: &mut dyn simoxide_spd::sim::Adjuster<C, F> = match rec.as_mut() {
            Some(rec) => {
                recording = simoxide_spd::inspect::Recording {
                    inner: &mut adj,
                    rec,
                };
                &mut recording
            }
            None => &mut adj,
        };
        let file = a.spd_log.as_deref().map(create).transpose()?;
        let sink: Box<dyn std::io::Write> = file.unwrap_or_else(|| Box::new(std::io::sink()));
        let mut log = JsonlLog::new(sink);
        log.all = a.spd_log_all;
        let resolved = match &a.decide {
            Some(actions) => {
                let names: Vec<&str> = actions.iter().map(String::as_str).collect();
                let res = r
                    .decide(adjuster, &mut log, &names)
                    .map_err(|e| format!("--decide: {e}"))?;
                report_resolution(&res);
                Some(res)
            }
            None => None,
        };
        r.set_pause_on_decision(a.pause_on_decision);
        let mut watch = super::snap::watch(a);
        let (status, before) = match &mut watch {
            Some(w) => {
                w.arm(&r.sim)?;
                let st = r
                    .run_while(adjuster, &mut log, |s| w.keep_going(s))
                    .map_err(|e| e.to_string())?;
                (st, None)
            }
            None => {
                let before = super::snap::before_ns(a)?;
                let st = r
                    .run_until(adjuster, &mut log, before)
                    .map_err(|e| e.to_string())?;
                (st, before)
            }
        };
        if a.spd_log.is_some() {
            log.finish().map_err(|e| format!("--spd-log: {e}"))?;
        }
        hpa_report(&r);
        // written after the snapshot, which it describes
        let decisions = a
            .decision_file
            .as_ref()
            .map(|p| (p, decision::report(resolved.as_ref(), r.pending_decision())));
        let decision = r.pending_decision().map(|d| {
            format!(
                "scaling decision {} pending at t={}: recommended [{}]",
                d.decision,
                d.time_ns as f64 / 1e9,
                d.policies().join(", ")
            )
        });
        if status == Status::Paused && (before.is_some() || watch.is_some() || decision.is_some()) {
            let measurements = match a.interval {
                true => r.sim.take_measurement_rows(),
                false => r.sim.state().measurements().clone(),
            };
            let snapshot = match &a.snapshot {
                Some(p) => {
                    let opts = super::snap::options(a);
                    let mut s = r.snapshot_value(opts.encoding).map_err(|e| e.to_string())?;
                    #[cfg(not(feature = "reconfiguration"))]
                    s.sections.extend(carry.sections(&r.sim, opts.encoding)?);
                    #[cfg(feature = "reconfiguration")]
                    if adj.run.is_started() {
                        s.sections.push(
                            adj.run
                                .snapshot_section(&r.sim, opts.encoding)
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    Some(super::snap::write(a, p, s)?)
                }
                None => None,
            };
            if let Some((p, v)) = &decisions {
                write_json(p, v)?;
            }
            super::meta::finish(
                a,
                rec,
                super::meta::End {
                    cm: r.sim.compiled(),
                    base: cm,
                    series: &measurements.series,
                    time_ns: r.sim.state().now_ns(),
                    generation: r.sim.state().model().generation(),
                    policies: policy_state(&r),
                    state: snapshot.as_ref().map(|s| s.2.as_str()),
                    paused: true,
                },
            )?;
            let st = r.sim.state();
            return Ok(End::Paused {
                now_ns: st.now_ns(),
                now_s: st.now_secs(),
                events: st.events(),
                measurements,
                snapshot,
                reason: decision
                    .or_else(|| watch.and_then(|w| w.fired().map(|f| f.reason.clone()))),
            });
        }
        let end_meta = rec.is_some().then(|| {
            let st = r.sim.state();
            (
                st.model().compiled().cloned(),
                st.model().generation(),
                policy_state(&r),
            )
        });
        let (res, _, s) = r.finish().map_err(|e| e.to_string())?;
        if let Some((p, v)) = &decisions {
            write_json(p, v)?;
        }
        if let Some((own, generation, policies)) = end_meta {
            super::meta::finish(
                a,
                rec,
                super::meta::End {
                    cm: own.as_deref().unwrap_or(cm),
                    base: cm,
                    series: &res.measurements.series,
                    time_ns: res.end_ns,
                    generation,
                    policies,
                    state: None,
                    paused: false,
                },
            )?;
        }
        let executed = if cfg!(feature = "reconfiguration") {
            format!("adjusted={}", s.adjustments_ok)
        } else {
            "(adjustments are not executed: they need the `reconfiguration` feature)".into()
        };
        eprintln!(
            "spd: policies={} evaluations={} requests={} {executed}",
            spd.contexts().len(),
            s.evaluations,
            s.requests
        );
        Ok(End::Finished(res))
    }

    /// The policy state of `--runtime-metadata`.
    fn policy_state<C: Compat, F: Flavour>(r: &SpdRun<'_, '_, C, F>) -> serde_json::Value {
        simoxide_spd::inspect::policy_state(r)
    }

    /// Prints what `--decide` did.
    fn report_resolution(r: &decision::Resolution) {
        eprintln!(
            "decision {} (t={}): recommended [{}], selected [{}]",
            r.decision.decision,
            r.decision.time_ns as f64 / 1e9,
            r.decision.policies().join(", "),
            r.selected.join(", ")
        );
        let n = |v: Option<usize>| v.map_or_else(|| "?".to_string(), |v| v.to_string());
        for o in &r.outcomes {
            eprintln!(
                "  {} ({}): ok={} size {} -> {}",
                o.policy,
                o.policy_name,
                o.ok,
                n(o.size_before),
                n(o.size_after)
            );
        }
    }

    /// Writes `v` to `p` (pretty JSON, a final newline).
    fn write_json(p: &std::path::Path, v: &serde_json::Value) -> Result<(), String> {
        let mut text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
        text.push('\n');
        std::fs::write(p, text).map_err(|e| format!("{}: {e}", p.display()))
    }

    /// The Slingshot-exact engine with the SPD layer (`DESIGN-SPD.md` X2d): Slingshot's
    /// adjustment in QVTo's ordered mode with `reconfiguration`, decisions only without;
    /// started (`Ok(cfg)`) or restored from a snapshot, to the pause or the end (5.7, "ssx
    /// snapshots as built").
    pub fn run_exact(
        a: &Args,
        cm: &CompiledModel,
        start: Result<SimConfig, &Snapshot>,
        out: Outputs,
        spd: &Spd,
    ) -> Result<End, String> {
        use simoxide_sim::ssx::{SpdHooks, SsxRun};
        use simoxide_spd::ssx::SsxSpd;
        if hpa_of(a, spd)?.is_some() || !a.hpa_set.is_empty() {
            return Err(
                "HPA autoscalers run in the SimuLizar and Slingshot flavours, not in the Slingshot-exact engine (--flavour slingshot, or --no-hpa)"
                    .into(),
            );
        }
        if !a.enforce.is_empty() {
            return Err(
                "--enforce is not supported by the Slingshot-exact engine (use --flavour slingshot)"
                    .into(),
            );
        }
        if super::decision_options(a) {
            return Err(
                "scaling decisions (--pause-on-decision, --decide, --decision-file) are not supported by the Slingshot-exact engine (use --flavour slingshot)"
                    .into(),
            );
        }
        super::no_runtime_metadata(a)?;
        #[cfg(feature = "reconfiguration")]
        let mut adj = super::changes::spd_adjuster(a, &spd.quirks())?;
        #[cfg(not(feature = "reconfiguration"))]
        let mut adj = NoAdjustment;
        let (mut run, snap) = match start {
            Ok(cfg) => (SsxRun::new(cm, cfg, out).map_err(|e| e.to_string())?, None),
            Err(s) => (
                SsxRun::restore_snapshot(s, cm, out).map_err(|e| e.to_string())?,
                Some(s),
            ),
        };
        if a.interval && snap.is_some() {
            run.take_measurement_rows();
        }
        #[cfg(feature = "reconfiguration")]
        let changed = {
            if let Some(s) = snap {
                adj.run
                    .restore_section(&run, s)
                    .map_err(|e| e.to_string())?;
            }
            super::changes::ssx_with_qvto(a, &mut run, &mut adj.run)?
        };
        #[cfg(not(feature = "reconfiguration"))]
        let carry = super::changes::ssx_plain_changes(a, &mut run, snap)?;
        let file = a.spd_log.as_deref().map(create).transpose()?;
        let sink: Box<dyn std::io::Write> = file.unwrap_or_else(|| Box::new(std::io::sink()));
        let mut log = JsonlLog::new(sink);
        log.all = a.spd_log_all;
        let mut hooks = match snap {
            Some(s) => SsxSpd::restore(spd, s, &mut adj, &mut log).map_err(|e| e.to_string())?,
            None => SsxSpd::new(spd, &cm.model, &mut adj, &mut log),
        };
        #[cfg(feature = "reconfiguration")]
        if changed {
            hooks.model_changed(&run.compiled().model.graph);
        }
        let (status, reason) =
            super::ssxrun::drive(a, &mut run, Some(&mut hooks as &mut dyn SpdHooks))?;
        let (end, ok, st) = if status == simoxide_sim::Status::Paused {
            let enc = super::snap::options(a).encoding;
            let mut extra = match &a.snapshot {
                Some(_) => hooks.snapshot_sections(enc).map_err(|e| e.to_string())?,
                None => Vec::new(),
            };
            let (ok, st) = (hooks.adjusted_ok, hooks.state.clone());
            drop(hooks);
            #[cfg(feature = "reconfiguration")]
            if a.snapshot.is_some() && adj.run.is_started() {
                extra.push(
                    adj.run
                        .snapshot_section(&run, enc)
                        .map_err(|e| e.to_string())?,
                );
            }
            #[cfg(not(feature = "reconfiguration"))]
            extra.extend(carry.sections(&run, enc)?);
            let measurements = match a.interval {
                true => run.take_measurement_rows(),
                false => run.measurements().clone(),
            };
            let snapshot = match &a.snapshot {
                Some(p) => {
                    let s = run.snapshot_value(enc, extra).map_err(|e| e.to_string())?;
                    Some(super::snap::write(a, p, s)?)
                }
                None => None,
            };
            let end = Ok(End::Paused {
                now_ns: run.now_ns(),
                now_s: run.now(),
                events: run.events(),
                measurements,
                snapshot,
                reason,
            });
            (end, ok, st)
        } else {
            let r = run.finish(Some(&mut hooks as &mut dyn SpdHooks));
            let (ok, st) = (hooks.adjusted_ok, hooks.state.clone());
            drop(hooks);
            (r.map(End::Finished).map_err(|e| e.to_string()), ok, st)
        };
        if a.spd_log.is_some() {
            log.finish().map_err(|e| format!("--spd-log: {e}"))?;
        }
        let end = end?;
        let executed = if cfg!(feature = "reconfiguration") {
            format!("adjusted={ok}")
        } else {
            "(adjustments are not executed: they need the `reconfiguration` feature)".into()
        };
        eprintln!(
            "spd: policies={} evaluations={} requests={} {executed}",
            spd.contexts().len(),
            st.evaluations,
            st.requests
        );
        Ok(end)
    }

    pub fn run(
        a: &Args,
        cm: &CompiledModel,
        cfg: SimConfig,
        out: Outputs,
        spd: &Spd,
        restore: Option<&Snapshot>,
    ) -> Result<End, String> {
        use simoxide_sim::{FlavourKind, SimuLizar, Slingshot};
        match (cfg.mode, cm.flavour.kind) {
            #[cfg(feature = "exact")]
            (Mode::Exact, FlavourKind::SimuLizar) => {
                go::<simoxide_sim::Exact, SimuLizar>(a, cm, cfg, out, spd, restore)
            }
            #[cfg(feature = "exact")]
            (Mode::Exact, FlavourKind::Slingshot) => {
                go::<simoxide_sim::Exact, Slingshot>(a, cm, cfg, out, spd, restore)
            }
            (Mode::Fast, FlavourKind::SimuLizar) => {
                go::<simoxide_sim::Fast, SimuLizar>(a, cm, cfg, out, spd, restore)
            }
            (Mode::Fast, FlavourKind::Slingshot) => {
                go::<simoxide_sim::Fast, Slingshot>(a, cm, cfg, out, spd, restore)
            }
            (_, FlavourKind::SlingshotExact) => Err(
                "the Slingshot-exact engine runs scaling policies without pauses and resumes \
                 (`run_exact`)"
                    .into(),
            ),
            #[cfg(not(feature = "exact"))]
            (m, _) => Err(m.unavailable_reason().unwrap_or_default()),
        }
    }
}

/// Without the `spd` feature, `.spd` files are ignored.
#[cfg(not(feature = "spd"))]
mod spd {
    use super::{Args, End};
    use simoxide_sim::snapshot::Snapshot;
    use simoxide_sim::{CompiledModel, Outputs, SimConfig};

    pub enum Never {}

    pub fn of_snapshot(_: &Args, _: &CompiledModel, _: &Snapshot) -> Result<Option<Never>, String> {
        Err("the snapshot is of a run with scaling policies: they need the `spd` feature".into())
    }

    pub fn of(a: &Args, _: &CompiledModel) -> Result<Option<Never>, String> {
        if a.spd_log.is_some() || a.spd_quirks.is_some() {
            return Err("--spd-log/--spd-quirks need the `spd` feature".into());
        }
        Ok(None)
    }

    pub fn run(
        _: &Args,
        _: &CompiledModel,
        _: SimConfig,
        _: Outputs,
        s: &Never,
        _: Option<&Snapshot>,
    ) -> Result<End, String> {
        match *s {}
    }

    pub fn run_exact(
        _: &Args,
        _: &CompiledModel,
        _: Result<SimConfig, &Snapshot>,
        _: Outputs,
        s: &Never,
    ) -> Result<End, String> {
        match *s {}
    }
}

/// SimuLizar's reconfiguration rules (`reconfiguration` feature): the run's reconfiguration
/// process runs the rules of the folder (DESIGN-SPD R1).
#[cfg(feature = "reconfiguration")]
mod rules {
    use super::{Args, End, create};
    use simoxide_reconf::{QvtoCore, RuleSet, RulesHost, RulesRun, RunConfig};
    use simoxide_sim::compat::Compat;
    use simoxide_sim::inspect::Recorder;
    use simoxide_sim::snapshot::Snapshot;
    use simoxide_sim::{
        CompiledModel, Mode, Outputs, Pause, RunState, SimConfig, SimError, SimuLizar, Simulation,
        Status,
    };
    use std::io::Write;
    use std::path::Path;

    fn go<C: Compat>(
        a: &Args,
        cm: &CompiledModel,
        cfg: SimConfig,
        out: Outputs,
        dir: &Path,
        restore: Option<&Snapshot>,
    ) -> Result<End, String> {
        let core = QvtoCore::new().map_err(|e| e.to_string())?;
        let rules = RuleSet::load_dir(&core, dir).map_err(|e| e.to_string())?;
        let mode = super::changes::mode(a)?;
        let cfg_q = RunConfig {
            mode,
            ..RunConfig::default()
        };
        let mut host = RulesHost::new(&core, rules, cfg_q).map_err(|e| e.to_string())?;
        if a.rules_log.is_some() {
            host = host.keep_records();
        }
        let mut run = match restore {
            Some(snap) if snap.section(simoxide_reconf::RULES).is_some() => {
                let old = RulesRun::<C>::snapshot_rules(snap).map_err(|e| e.to_string())?;
                let new: Vec<String> = host
                    .rules()
                    .rules
                    .iter()
                    .map(|r| r.name().to_string())
                    .collect();
                let mut run = if old == new {
                    RulesRun::<C>::restore_snapshot(snap, cm, host, out)
                } else {
                    eprintln!("rules replaced: {old:?} -> {new:?}");
                    RulesRun::<C>::restore_snapshot_replacing(snap, cm, host, out)
                }
                .map_err(|e| e.to_string())?;
                if a.interval {
                    run.sim_mut().take_measurement_rows();
                }
                run
            }
            Some(snap) => {
                // a snapshot without rules: they are added at the resume
                let mut sim =
                    Simulation::<C>::restore_snapshot(snap, cm, out).map_err(|e| e.to_string())?;
                if a.interval {
                    sim.take_measurement_rows();
                }
                eprintln!(
                    "rules added: {:?}",
                    host.rules()
                        .rules
                        .iter()
                        .map(|r| r.name().to_string())
                        .collect::<Vec<_>>()
                );
                RulesRun::new(sim, host)
            }
            None => {
                let sim = Simulation::<C>::create(cm, cfg, out).map_err(|e| e.to_string())?;
                RulesRun::new(sim, host)
            }
        };
        let mut rec = super::meta::begin(a, run.sim(), None, || serde_json::Value::Null);
        if let Some(snap) = restore {
            run = super::changes::with_rules(a, run, snap, rec.as_mut())?;
        }
        let mut watch = super::snap::watch(a);
        if let Some(w) = &mut watch {
            w.arm(run.sim())?;
        }
        let pause = a.pause.unwrap_or(Pause::Never);
        let mut keep = |s: &RunState<C, SimuLizar>| match &mut watch {
            Some(w) => w.keep_going(s),
            None => !reached(pause, s),
        };
        let status = match rec.as_mut() {
            None => run.run_while(&mut keep),
            Some(rec) => {
                let (r, st) = run_recorded(run, rec, &mut keep);
                run = r;
                st
            }
        }
        .map_err(|e| e.to_string())?;
        let paused = status == Status::Paused && (a.pause.is_some() || watch.is_some());
        // (time, generation, the run's own model) at the end, for --runtime-metadata
        let end_meta = {
            let st = run.sim().state();
            (
                st.now_ns(),
                st.model().generation(),
                st.model().compiled().cloned(),
            )
        };
        let (end, mut host) = if paused {
            // the records so far go to this run's log; the snapshot holds none
            let records = run.host_mut().take_records();
            let measurements = match a.interval {
                true => run.sim_mut().take_measurement_rows(),
                false => run.sim().state().measurements().clone(),
            };
            let snapshot = match &a.snapshot {
                Some(p) => {
                    let s = run
                        .snapshot_value(super::snap::options(a).encoding)
                        .map_err(|e| e.to_string())?;
                    Some(super::snap::write(a, p, s)?)
                }
                None => None,
            };
            super::meta::finish(
                a,
                rec,
                super::meta::End {
                    cm: run.sim().compiled(),
                    base: cm,
                    series: &measurements.series,
                    time_ns: end_meta.0,
                    generation: end_meta.1,
                    policies: serde_json::Value::Null,
                    state: snapshot.as_ref().map(|s| s.2.as_str()),
                    paused: true,
                },
            )?;
            let st = run.sim().state();
            let end = End::Paused {
                now_ns: st.now_ns(),
                now_s: st.now_secs(),
                events: st.events(),
                measurements,
                snapshot,
                reason: watch
                    .as_ref()
                    .and_then(|w| w.fired().map(|f| f.reason.clone())),
            };
            let (_, mut host) = run.into_parts();
            if let Some(r) = host_records(&mut host) {
                r.extend(records);
            }
            (end, host)
        } else {
            let (res, host) = run.finish().map_err(|e| e.to_string())?;
            super::meta::finish(
                a,
                rec,
                super::meta::End {
                    cm: end_meta.2.as_deref().unwrap_or(cm),
                    base: cm,
                    series: &res.measurements.series,
                    time_ns: res.end_ns,
                    generation: end_meta.1,
                    policies: serde_json::Value::Null,
                    state: None,
                    paused: false,
                },
            )?;
            (End::Finished(res), host)
        };
        if let Some(p) = &a.rules_log {
            let mut f = create(p)?;
            for r in host.take_records() {
                let rules: Vec<String> = r
                    .rules
                    .iter()
                    .map(|x| {
                        format!(
                            "{{\"rule\":{},\"applied\":{},\"changed\":{},\"log\":{}}}",
                            json_str(&x.name),
                            x.succeeded,
                            x.changed,
                            json_str(&x.log)
                        )
                    })
                    .collect();
                writeln!(
                    f,
                    "{{\"t\":{},\"post_stop\":{},\"succeeded\":{},\"rules\":[{}]}}",
                    r.at_ns as f64 / 1e9,
                    r.post_stop,
                    r.succeeded,
                    rules.join(",")
                )
                .map_err(|e| format!("{}: {e}", p.display()))?;
            }
        }
        let s = host.stats();
        eprintln!(
            "rules: {} rule(s), {} reconfiguration(s), {} succeeded, {} changed the model, {:.1} ms (PRM {:.1} ms)",
            host.rules().rules.len(),
            s.runs,
            s.succeeded,
            s.changed,
            s.micros as f64 / 1e3,
            s.prm_micros as f64 / 1e3
        );
        Ok(end)
    }

    /// `RulesRun::run_while`, recording every reconfiguration that changed the model in `rec`
    /// (cause `rules`).
    fn run_recorded<'m, C: Compat>(
        run: RulesRun<'m, C>,
        rec: &mut Recorder,
        keep: &mut dyn FnMut(&RunState<C, SimuLizar>) -> bool,
    ) -> (RulesRun<'m, C>, Result<Status, SimError>) {
        let (mut sim, mut host) = run.into_parts();
        let st = loop {
            let st = match sim.run_while(|s| !s.reconf_pending() && keep(s)) {
                Ok(st) => st,
                Err(e) => break Err(e),
            };
            let generation = sim.state().model().generation();
            match host.handle(&mut sim) {
                Ok(true) => {
                    if sim.state().model().generation() != generation {
                        super::meta::record_step(
                            Some(&mut *rec),
                            &sim,
                            None,
                            serde_json::json!({"type": "rules"}),
                        );
                    }
                }
                Ok(false) => break Ok(st),
                Err(e) => break Err(e),
            }
        };
        (RulesRun::from_parts(sim, host), st)
    }

    /// `RulesRun::run_until`'s pause test.
    fn reached<C: Compat>(p: Pause, s: &RunState<C, SimuLizar>) -> bool {
        match p {
            Pause::Never => false,
            Pause::After(t) => s.next_event_ns().is_some_and(|n| n > t),
            Pause::Before(t) => s.next_event_ns().is_some_and(|n| n >= t),
            Pause::Events(n) => s.events() >= n,
        }
    }

    /// Keeps the records taken before a snapshot for this run's log.
    fn host_records(host: &mut RulesHost) -> Option<&mut Vec<simoxide_reconf::ReconfRecord>> {
        host.records_mut()
    }

    fn json_str(s: &str) -> String {
        let mut o = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                '\n' => o.push_str("\\n"),
                c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                c => o.push(c),
            }
        }
        o.push('"');
        o
    }

    pub fn run(
        a: &Args,
        cm: &CompiledModel,
        cfg: SimConfig,
        out: Outputs,
        dir: &Path,
        restore: Option<&Snapshot>,
    ) -> Result<End, String> {
        match cfg.mode {
            #[cfg(feature = "exact")]
            Mode::Exact => go::<simoxide_sim::Exact>(a, cm, cfg, out, dir, restore),
            Mode::Fast => go::<simoxide_sim::Fast>(a, cm, cfg, out, dir, restore),
            #[cfg(not(feature = "exact"))]
            m => Err(m.unavailable_reason().unwrap_or_default()),
        }
    }
}

/// Without the `reconfiguration` feature, rules cannot run.
#[cfg(not(feature = "reconfiguration"))]
mod rules {
    use super::{Args, End};
    use simoxide_sim::snapshot::Snapshot;
    use simoxide_sim::{CompiledModel, Outputs, SimConfig};
    use std::path::Path;

    pub fn run(
        _: &Args,
        _: &CompiledModel,
        _: SimConfig,
        _: Outputs,
        dir: &Path,
        _: Option<&Snapshot>,
    ) -> Result<End, String> {
        Err(format!(
            "reconfiguration rules ({}) need a build with the `reconfiguration` feature",
            dir.display()
        ))
    }
}

/// Change sets at a resume (`--change`, `--transform`; `reconfiguration` feature;
/// DESIGN-SPD.md N2).
#[cfg(feature = "reconfiguration")]
mod changes {
    use super::meta::Groups;
    use super::{Args, ChangeArg};
    use simoxide_reconf::{
        ChangeFile, FailedRunChanges, Imports, QvtMode, QvtoCore, QvtoRun, RULES, ResumeChanges,
        RulesRun, RunConfig, StepReport,
    };
    use simoxide_sim::inspect::Recorder;
    use simoxide_sim::snapshot::{Encoding, Section, Snapshot};
    use simoxide_sim::{Compat, Flavour, Simulation};

    /// The QVTo state of a run without rules and SPD (transformations at a resume), carried
    /// into its next snapshot (section `qvto`).
    pub struct Carry {
        qvto: Option<QvtoRun>,
    }

    impl Carry {
        pub fn none() -> Carry {
            Carry { qvto: None }
        }

        /// The sections of the run `sim` (whose model may have changed since the last
        /// transformation).
        pub fn sections<H: simoxide_sim::ChangeHost + ?Sized>(
            &self,
            sim: &H,
            enc: Encoding,
        ) -> Result<Vec<Section>, String> {
            match &self.qvto {
                Some(q) if q.is_started() => Ok(vec![
                    q.snapshot_section(sim, enc).map_err(|e| e.to_string())?,
                ]),
                _ => Ok(Vec::new()),
            }
        }
    }

    pub fn mode(a: &Args) -> Result<QvtMode, String> {
        match a.qvto_mode.as_deref() {
            None | Some("auto") => Ok(QvtMode::Auto),
            Some("fast") => Ok(QvtMode::Fast),
            Some("compat") => Ok(QvtMode::Compat),
            Some("ordered") => Ok(QvtMode::InsertionOrderSets),
            Some(m) => Err(format!(
                "--qvto-mode: expected auto, ordered, fast or compat, got {m}"
            )),
        }
    }

    /// The QVTo configuration of a run's transformations at a resume: a transformation that
    /// does not complete changes nothing.
    pub fn config(a: &Args) -> Result<RunConfig, String> {
        Ok(RunConfig {
            mode: mode(a)?,
            failed_runs: FailedRunChanges::Discard,
            ..RunConfig::default()
        })
    }

    /// The change steps of `a`, transformations compiled with `core`.
    pub fn build(a: &Args, core: &QvtoCore) -> Result<ResumeChanges, String> {
        let mut c = ResumeChanges::new();
        for ch in &a.changes {
            c = add_step(c, ch, core)?;
        }
        Ok(c)
    }

    /// `c` with the change step `ch`.
    fn add_step(
        c: ResumeChanges,
        ch: &ChangeArg,
        core: &QvtoCore,
    ) -> Result<ResumeChanges, String> {
        Ok(match ch {
            ChangeArg::File(p) => c.file(ChangeFile::load(p).map_err(|e| match e {
                simoxide_reconf::ChangeFileError::Line(..) => {
                    format!("{}: {e}", p.display())
                }
                e => e.to_string(),
            })?),
            ChangeArg::Transform(p) => c.transform(
                core.compile_file(p, &Imports::Folder)
                    .map_err(|e| format!("{}: {e}", p.display()))?,
                simoxide_reconf::Invocation::simulizar(),
            ),
        })
    }

    /// Applies the change steps of `a` to `sim` in `q`, one after the other, each recorded in
    /// `rec` (`--runtime-metadata`) with its file as the cause.
    fn apply_steps<C: Compat, F: Flavour>(
        a: &Args,
        sim: &mut Simulation<'_, C, F>,
        q: &mut QvtoRun,
        mut rec: Option<&mut Recorder>,
        groups: Groups,
    ) -> Result<Vec<StepReport>, String> {
        let mut out = Vec::new();
        for ch in &a.changes {
            let c = add_step(ResumeChanges::new(), ch, q.core())?;
            let steps = c.apply(sim, q).map_err(|e| e.to_string())?;
            if steps.is_empty() {
                // a change file with HPA parameters only
                continue;
            }
            out.extend(steps);
            let cause = match ch {
                ChangeArg::File(p) => serde_json::json!({"type": "change", "file": p}),
                ChangeArg::Transform(p) => serde_json::json!({"type": "transform", "file": p}),
            };
            super::meta::record_step(rec.as_deref_mut(), &*sim, groups, cause);
        }
        Ok(out)
    }

    fn report(steps: &[StepReport]) {
        for s in steps {
            let Some(r) = &s.change else {
                eprintln!("change {}: no model change", s.step);
                continue;
            };
            eprintln!(
                "change {}: generation {}, {}",
                s.step,
                r.generation,
                if r.applied.is_empty() {
                    "nothing applied".to_string()
                } else {
                    r.applied.join("; ")
                }
            );
            for i in &r.ignored {
                eprintln!(
                    "warning: change {}: ignored (as SimuLizar's syncers): {i}",
                    s.step
                );
            }
        }
    }

    /// Applies the change steps to a restored run without rules; restores the snapshot's QVTo
    /// state first.
    pub fn plain<C: Compat, F: Flavour>(
        a: &Args,
        sim: &mut Simulation<'_, C, F>,
        snap: Option<&Snapshot>,
        rec: Option<&mut Recorder>,
    ) -> Result<Carry, String> {
        let has_qvto = snap.is_some_and(|s| s.section(simoxide_reconf::QVTO).is_some());
        if a.changes.is_empty() && !has_qvto {
            return Ok(Carry::none());
        }
        let core = QvtoCore::new().map_err(|e| e.to_string())?;
        let mut q = QvtoRun::new(&core, config(a)?).map_err(|e| e.to_string())?;
        if let Some(s) = snap {
            q.restore_section(sim, s).map_err(|e| e.to_string())?;
        }
        let steps = apply_steps(a, sim, &mut q, rec, None)?;
        report(&steps);
        Ok(Carry { qvto: Some(q) })
    }

    /// [`plain`] for a restored Slingshot-exact run: its change steps through the engine's
    /// `ChangeHost` (change files without option lines, transformations).
    pub fn ssx_plain_changes(
        a: &Args,
        run: &mut simoxide_sim::ssx::SsxRun<'_>,
        snap: Option<&Snapshot>,
    ) -> Result<Carry, String> {
        let has_qvto = snap.is_some_and(|s| s.section(simoxide_reconf::QVTO).is_some());
        if a.changes.is_empty() && !has_qvto {
            return Ok(Carry::none());
        }
        let core = QvtoCore::new().map_err(|e| e.to_string())?;
        let mut q = QvtoRun::new(&core, config(a)?).map_err(|e| e.to_string())?;
        if let Some(s) = snap {
            q.restore_section(&*run, s).map_err(|e| e.to_string())?;
        }
        let steps = build(a, &core)?
            .apply_to(run, &mut q)
            .map_err(|e| e.to_string())?;
        report(&steps);
        Ok(Carry { qvto: Some(q) })
    }

    /// [`with_qvto`] for a restored Slingshot-exact run with scaling policies.
    #[cfg(feature = "spd")]
    pub fn ssx_with_qvto(
        a: &Args,
        run: &mut simoxide_sim::ssx::SsxRun<'_>,
        q: &mut QvtoRun,
    ) -> Result<bool, String> {
        if a.changes.is_empty() {
            return Ok(false);
        }
        let steps = build(a, q.core())?
            .apply_to(run, q)
            .map_err(|e| e.to_string())?;
        report(&steps);
        Ok(!steps.is_empty())
    }

    /// The `hpa` lines of the change files (`--change`), in order.
    pub fn hpa_settings(a: &Args) -> Result<Vec<(String, String, f64)>, String> {
        let mut v = Vec::new();
        for ch in &a.changes {
            if let ChangeArg::File(p) = ch {
                v.extend(
                    ChangeFile::load(p)
                        .map_err(|e| format!("{}: {e}", p.display()))?
                        .hpa_settings(),
                );
            }
        }
        Ok(v)
    }

    /// The SPD adjuster of a run (Slingshot's `MainTransformation`, `simoxide-reconf`): QVTo's
    /// ordered mode unless `--qvto-mode` says otherwise, the transformations of the
    /// interpreter's compat flags (Q4, Q6, Q7).
    #[cfg(feature = "spd")]
    pub fn spd_adjuster(
        a: &Args,
        quirks: &simoxide_spd::Quirks,
    ) -> Result<simoxide_reconf::QvtoAdjuster, String> {
        let core = QvtoCore::new().map_err(|e| e.to_string())?;
        let mut opts = simoxide_reconf::SpdAdjust::new(quirks);
        opts.mode = match mode(a)? {
            QvtMode::Auto => QvtMode::InsertionOrderSets,
            m => m,
        };
        simoxide_reconf::QvtoAdjuster::spd(&core, &opts).map_err(|e| e.to_string())
    }

    /// Applies the change steps to a restored SPD run in its adjuster's QVTo state (one QVTo
    /// state per run: the next snapshot's section `qvto`); whether the model changed.
    #[cfg(feature = "spd")]
    pub fn with_qvto<C: Compat, F: Flavour>(
        a: &Args,
        sim: &mut Simulation<'_, C, F>,
        q: &mut QvtoRun,
        rec: Option<&mut Recorder>,
        groups: Groups,
    ) -> Result<bool, String> {
        if a.changes.is_empty() {
            return Ok(false);
        }
        let steps = apply_steps(a, sim, q, rec, groups)?;
        report(&steps);
        Ok(!steps.is_empty())
    }

    /// Applies the change steps to a restored run with rules, in its host's QVTo state.
    pub fn with_rules<'m, C: Compat, F: Flavour>(
        a: &Args,
        run: RulesRun<'m, C, F>,
        snap: &Snapshot,
        rec: Option<&mut Recorder>,
    ) -> Result<RulesRun<'m, C, F>, String> {
        let (mut sim, mut host) = run.into_parts();
        if snap.section(RULES).is_none() {
            // rules added to a run whose QVTo state (if any) is the section `qvto`
            host.qvto()
                .restore_section(&sim, snap)
                .map_err(|e| e.to_string())?;
        }
        if !a.changes.is_empty() {
            let steps = apply_steps(a, &mut sim, host.qvto(), rec, None)?;
            report(&steps);
        }
        Ok(RulesRun::from_parts(sim, host))
    }
}

/// Without the `reconfiguration` feature there are no change steps; a QVTo state in a snapshot
/// is carried as it is.
#[cfg(not(feature = "reconfiguration"))]
mod changes {
    use super::Args;
    use simoxide_sim::snapshot::{Encoding, Section, Snapshot};
    use simoxide_sim::{Compat, Flavour, Simulation};

    pub struct Carry {
        raw: Option<(Section, Encoding)>,
        /// Read by SPD runs (membership refresh).
        #[cfg_attr(not(feature = "spd"), allow(dead_code))]
        pub changed: bool,
    }

    impl Carry {
        pub fn none() -> Carry {
            Carry {
                raw: None,
                changed: false,
            }
        }

        pub fn sections<H: ?Sized>(&self, _sim: &H, enc: Encoding) -> Result<Vec<Section>, String> {
            match &self.raw {
                None => Ok(Vec::new()),
                Some((s, e)) if *e == enc => Ok(vec![s.clone()]),
                Some(_) => Err(
                    "the snapshot's QVTo state can be re-encoded only by a build with the \
                     `reconfiguration` feature (keep the snapshot format)"
                        .into(),
                ),
            }
        }
    }

    /// [`plain`] for a restored Slingshot-exact run.
    pub fn ssx_plain_changes(
        a: &Args,
        _: &mut simoxide_sim::ssx::SsxRun<'_>,
        snap: Option<&Snapshot>,
    ) -> Result<Carry, String> {
        if !a.changes.is_empty() {
            return Err(
                "--change and --transform need a build with the `reconfiguration` feature".into(),
            );
        }
        Ok(Carry {
            raw: snap.and_then(|s| Some((s.section("qvto")?.clone(), s.header.encoding))),
            changed: false,
        })
    }

    pub fn plain<C: Compat, F: Flavour>(
        a: &Args,
        _: &mut Simulation<'_, C, F>,
        snap: Option<&Snapshot>,
        _: Option<&mut simoxide_sim::inspect::Recorder>,
    ) -> Result<Carry, String> {
        if !a.changes.is_empty() {
            return Err(
                "--change and --transform need a build with the `reconfiguration` feature".into(),
            );
        }
        Ok(Carry {
            raw: snap.and_then(|s| Some((s.section("qvto")?.clone(), s.header.encoding))),
            changed: false,
        })
    }
}

/// `simoxide explore` (`reconfiguration` feature): fork a snapshot into variants.
#[cfg(feature = "reconfiguration")]
mod explore {
    use super::{Args, setup_with_rules};
    use simoxide_reconf::{ChangeFile, Imports, QvtoCore, ResumeChanges, Variant};
    use simoxide_sim::ir::metric_names as n;
    use simoxide_sim::{FlavourKind, Mode, SimuLizar, Slingshot};

    pub fn run(a: &Args) -> Result<(), String> {
        let s = super::snap::read(a)?;
        let a = super::snap::resume_args(a, &s)?;
        let a = &a;
        if s.section("rules").is_some() || s.section("spd").is_some() {
            return Err(
                "explore forks runs without scaling policies or rules (use resume for those)"
                    .into(),
            );
        }
        if a.variants.is_empty() {
            return Err(
                "explore needs at least one --variant NAME=FILE (or NAME alone: no change)".into(),
            );
        }
        let (cm, _, _) = setup_with_rules(a)?;
        let core = QvtoCore::new().map_err(|e| e.to_string())?;
        let mut variants = Vec::new();
        for (name, file) in &a.variants {
            let mut c = ResumeChanges::new();
            if let Some(p) = file {
                c = if p.extension().is_some_and(|e| e == "qvto") {
                    c.transform(
                        core.compile_file(p, &Imports::Folder)
                            .map_err(|e| format!("{}: {e}", p.display()))?,
                        simoxide_reconf::Invocation::simulizar(),
                    )
                } else {
                    c.file(ChangeFile::load(p).map_err(|e| format!("{}: {e}", p.display()))?)
                };
            }
            variants.push(Variant {
                name: name.clone(),
                changes: c,
            });
        }
        let threads = if a.threads_given {
            a.threads
        } else {
            std::thread::available_parallelism()
                .map_or(1, |n| n.get())
                .min(4)
        };
        let t0 = std::time::Instant::now();
        let results = match (a.mode, cm.flavour.kind) {
            #[cfg(feature = "exact")]
            (Mode::Exact, FlavourKind::SimuLizar) => {
                simoxide_reconf::explore::<simoxide_sim::Exact, SimuLizar>(
                    &cm, &s, &core, &variants, a.pause, threads,
                )
            }
            #[cfg(feature = "exact")]
            (Mode::Exact, FlavourKind::Slingshot) => {
                simoxide_reconf::explore::<simoxide_sim::Exact, Slingshot>(
                    &cm, &s, &core, &variants, a.pause, threads,
                )
            }
            (Mode::Fast, FlavourKind::SimuLizar) => {
                simoxide_reconf::explore::<simoxide_sim::Fast, SimuLizar>(
                    &cm, &s, &core, &variants, a.pause, threads,
                )
            }
            (Mode::Fast, FlavourKind::Slingshot) => {
                simoxide_reconf::explore::<simoxide_sim::Fast, Slingshot>(
                    &cm, &s, &core, &variants, a.pause, threads,
                )
            }
            (m, k) => {
                return Err(format!(
                    "explore: mode {m} with flavour {k:?} is not available"
                ));
            }
        };
        let metrics: Vec<&str> = if a.metrics.is_empty() {
            vec![n::RESPONSE_TIME, n::UTILIZATION]
        } else {
            a.metrics
                .iter()
                .map(|m| match m.as_str() {
                    "rt" => n::RESPONSE_TIME,
                    "util" => n::UTILIZATION,
                    "state" => n::STATE_OF_ACTIVE_RESOURCE,
                    "demand" => n::RESOURCE_DEMAND,
                    "waiting" => n::WAITING_TIME,
                    "holding" => n::HOLDING_TIME,
                    "passive" => n::STATE_OF_PASSIVE_RESOURCE,
                    o => o,
                })
                .collect()
        };
        print!("{}", simoxide_reconf::compare(&results, &metrics));
        eprintln!(
            "explore: {} variant(s) from t={} s on {threads} thread(s), wall={:.3}ms",
            results.len(),
            s.header.time_ns as f64 / 1e9,
            t0.elapsed().as_secs_f64() * 1e3
        );
        if results.iter().any(|r| r.outcome.is_err()) {
            return Err("some variants failed".into());
        }
        Ok(())
    }
}

#[cfg(not(feature = "reconfiguration"))]
mod explore {
    pub fn run(_: &super::Args) -> Result<(), String> {
        Err("explore needs a build with the `reconfiguration` feature".into())
    }
}

/// The Slingshot-exact engine with pauses and snapshots (`DESIGN-SPD.md` 5.7, "ssx snapshots
/// as built"): runs without scaling policies (with them: `spd::run_exact`).
mod ssxrun {
    use super::{Args, End};
    use simoxide_sim::snapshot::Snapshot;
    use simoxide_sim::ssx::{SpdHooks, SsxRun};
    use simoxide_sim::{CompiledModel, Outputs, SimConfig, Status};

    /// Runs `run` to the pause of `a` (`--pause-at` and the like, `--pause-when`) or the end;
    /// the status and the pause condition that held.
    pub fn drive(
        a: &Args,
        run: &mut SsxRun<'_>,
        hooks: Option<&mut dyn SpdHooks>,
    ) -> Result<(Status, Option<String>), String> {
        let mut watch = super::snap::watch(a);
        let status = match (&mut watch, a.pause) {
            (Some(w), _) => run.run_watch(w, hooks),
            (None, Some(p)) => run.run_until(p, hooks),
            (None, None) => run.run_until(simoxide_sim::Pause::Never, hooks),
        }
        .map_err(|e| e.to_string())?;
        let reason = watch.and_then(|w| w.fired().map(|f| f.reason.clone()));
        Ok((status, reason))
    }

    /// A run without scaling policies: started (`Ok(cfg)`) or restored (with `--change` and
    /// `--transform` steps), to the pause or the end.
    pub fn plain(
        a: &Args,
        cm: &CompiledModel,
        start: Result<SimConfig, &Snapshot>,
        out: Outputs,
    ) -> Result<End, String> {
        if !a.enforce.is_empty() {
            return Err("--enforce: the snapshot has no scaling policies".into());
        }
        super::no_runtime_metadata(a)?;
        let (mut run, snap) = match start {
            Ok(cfg) => (SsxRun::new(cm, cfg, out).map_err(|e| e.to_string())?, None),
            Err(s) => (
                SsxRun::restore_snapshot(s, cm, out).map_err(|e| e.to_string())?,
                Some(s),
            ),
        };
        if a.interval && snap.is_some() {
            run.take_measurement_rows();
        }
        let carry = super::changes::ssx_plain_changes(a, &mut run, snap)?;
        let (status, reason) = drive(a, &mut run, None)?;
        if status == Status::Finished {
            return run
                .finish(None)
                .map(End::Finished)
                .map_err(|e| e.to_string());
        }
        let measurements = match a.interval {
            true => run.take_measurement_rows(),
            false => run.measurements().clone(),
        };
        let snapshot = match &a.snapshot {
            Some(p) => {
                let enc = super::snap::options(a).encoding;
                let sections = carry.sections(&run, enc)?;
                let s = run
                    .snapshot_value(enc, sections)
                    .map_err(|e| e.to_string())?;
                Some(super::snap::write(a, p, s)?)
            }
            None => None,
        };
        Ok(End::Paused {
            now_ns: run.now_ns(),
            now_s: run.now(),
            events: run.events(),
            measurements,
            snapshot,
            reason,
        })
    }
}

/// Pausing, snapshots and `simoxide resume` (docs/guide/snapshots.md).
mod snap {
    use super::{Args, End, create, report_end, setup_with_rules};
    use simoxide_sim::compat::Compat;
    use simoxide_sim::snapshot::{Encoding, Snapshot, SnapshotOptions};
    use simoxide_sim::{
        CompiledModel, Condition, Flavour, FlavourKind, Mode, Outputs, Pause, Quirks, SimConfig,
        SimuLizar, Simulation, Slingshot, Status, Watch,
    };
    use std::path::{Path, PathBuf};
    use std::time::Instant;

    pub fn options(a: &Args) -> SnapshotOptions {
        SnapshotOptions {
            encoding: if a.snapshot_json {
                Encoding::Json
            } else {
                Encoding::Postcard
            },
        }
    }

    /// The pause as the SPD driver's bound: before the first event at or after it (ns).
    #[cfg_attr(not(feature = "spd"), allow(dead_code))]
    pub fn before_ns(a: &Args) -> Result<Option<i64>, String> {
        Ok(match a.pause {
            None => None,
            Some(Pause::After(t)) => Some(t.saturating_add(1)),
            Some(Pause::Before(t)) => Some(t),
            Some(Pause::Events(_)) => {
                return Err(
                    "--pause-events is not supported with scaling policies (use --pause-at)".into(),
                );
            }
            Some(Pause::Never) => None,
        })
    }

    /// The pause conditions of `--pause-when` (with `--pause-at` and the like as one more), if
    /// any.
    pub fn watch(a: &Args) -> Option<Watch> {
        if a.pause_when.is_empty() {
            return None;
        }
        let mut c = a.pause_when.clone();
        if let Some(p) = a.pause {
            c.push(Condition::At(p));
        }
        Some(Watch::new(c))
    }

    /// Writes `s` to `p`, with the state id of the snapshot resumed as its `parent`; the path,
    /// the number of bytes and the state id.
    pub fn write(a: &Args, p: &Path, mut s: Snapshot) -> Result<(PathBuf, u64, String), String> {
        s.header.parent = a.parent.as_ref().map(|(id, _)| id.clone());
        let mut bytes = Vec::new();
        let h = s.write(&mut bytes).map_err(|e| e.to_string())?;
        std::fs::write(p, &bytes).map_err(|e| format!("{}: {e}", p.display()))?;
        Ok((
            p.to_path_buf(),
            bytes.len() as u64,
            h.state.unwrap_or_default(),
        ))
    }

    fn go<C: Compat, F: Flavour>(
        a: &Args,
        cm: &CompiledModel,
        start: Result<SimConfig, &Snapshot>,
        out: Outputs,
    ) -> Result<End, String> {
        let mut carry = super::changes::Carry::none();
        let mut rec;
        let mut sim = match start {
            Ok(cfg) => {
                let sim = Simulation::<C, F>::create(cm, cfg, out).map_err(|e| e.to_string())?;
                rec = super::meta::begin(a, &sim, None, || serde_json::Value::Null);
                sim
            }
            Err(s) => {
                if !a.enforce.is_empty() {
                    return Err("--enforce: the snapshot has no scaling policies".into());
                }
                let mut sim =
                    Simulation::<C, F>::restore_snapshot(s, cm, out).map_err(|e| e.to_string())?;
                if a.interval {
                    sim.take_measurement_rows();
                }
                rec = super::meta::begin(a, &sim, None, || serde_json::Value::Null);
                carry = super::changes::plain(a, &mut sim, Some(s), rec.as_mut())?;
                sim
            }
        };
        let mut watch = watch(a);
        let status = match (&mut watch, a.pause) {
            (Some(w), _) => sim.run_watch(w).map_err(|e| e.to_string())?,
            (None, Some(p)) => sim.run_until(p).map_err(|e| e.to_string())?,
            (None, None) => Status::Finished,
        };
        if status == Status::Finished {
            let own = sim.state().model().compiled().cloned();
            let generation = sim.state().model().generation();
            let r = sim.finish().map_err(|e| e.to_string())?;
            super::meta::finish(
                a,
                rec,
                super::meta::End {
                    cm: own.as_deref().unwrap_or(cm),
                    base: cm,
                    series: &r.measurements.series,
                    time_ns: r.end_ns,
                    generation,
                    policies: serde_json::Value::Null,
                    state: None,
                    paused: false,
                },
            )?;
            return Ok(End::Finished(r));
        }
        let measurements = match a.interval {
            true => sim.take_measurement_rows(),
            false => sim.state().measurements().clone(),
        };
        let snapshot = match &a.snapshot {
            Some(p) => {
                let enc = options(a).encoding;
                let sections = carry.sections(&sim, enc)?;
                let s = sim
                    .snapshot_value(enc, sections)
                    .map_err(|e| e.to_string())?;
                Some(write(a, p, s)?)
            }
            None => None,
        };
        super::meta::finish(
            a,
            rec,
            super::meta::End {
                cm: sim.compiled(),
                base: cm,
                series: &measurements.series,
                time_ns: sim.state().now_ns(),
                generation: sim.state().model().generation(),
                policies: serde_json::Value::Null,
                state: snapshot.as_ref().map(|s| s.2.as_str()),
                paused: true,
            },
        )?;
        let st = sim.state();
        Ok(End::Paused {
            now_ns: st.now_ns(),
            now_s: st.now_secs(),
            events: st.events(),
            measurements,
            snapshot,
            reason: watch.and_then(|w| w.fired().map(|f| f.reason.clone())),
        })
    }

    /// A run without scaling policies or rules: started (`Ok(cfg)`) or restored, to the pause or
    /// the end.
    pub fn plain(
        a: &Args,
        cm: &CompiledModel,
        start: Result<SimConfig, &Snapshot>,
        out: Outputs,
    ) -> Result<End, String> {
        let mode = match &start {
            Ok(c) => c.mode,
            Err(s) => Mode::parse(&s.header.mode).ok_or("unknown mode in the snapshot")?,
        };
        match (mode, cm.flavour.kind) {
            #[cfg(feature = "exact")]
            (Mode::Exact, FlavourKind::SimuLizar) => {
                go::<simoxide_sim::Exact, SimuLizar>(a, cm, start, out)
            }
            #[cfg(feature = "exact")]
            (Mode::Exact, FlavourKind::Slingshot) => {
                go::<simoxide_sim::Exact, Slingshot>(a, cm, start, out)
            }
            (Mode::Fast, FlavourKind::SimuLizar) => {
                go::<simoxide_sim::Fast, SimuLizar>(a, cm, start, out)
            }
            (Mode::Fast, FlavourKind::Slingshot) => {
                go::<simoxide_sim::Fast, Slingshot>(a, cm, start, out)
            }
            (_, FlavourKind::SlingshotExact) => super::ssxrun::plain(a, cm, start, out),
            #[cfg(not(feature = "exact"))]
            (m, _) => Err(m.unavailable_reason().unwrap_or_default()),
        }
    }

    pub fn read(a: &Args) -> Result<Snapshot, String> {
        let p = a
            .file
            .as_ref()
            .ok_or("which snapshot? (resume FILE --model DIR)")?;
        let f = std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
        Snapshot::read(std::io::BufReader::new(f)).map_err(|e| format!("{}: {e}", p.display()))
    }

    /// `a` with the snapshot's mode, flavour and quirks.
    pub fn resume_args(a: &Args, s: &Snapshot) -> Result<Args, String> {
        let mut a2 = a.clone();
        a2.flavour =
            FlavourKind::parse(&s.header.flavour).ok_or("unknown flavour in the snapshot")?;
        a2.quirks = if s.header.quirks.is_empty() {
            Quirks::NONE
        } else {
            Quirks::parse(&s.header.quirks)?
        };
        a2.mode = Mode::parse(&s.header.mode).ok_or("unknown mode in the snapshot")?;
        if let Some(why) = a2.mode.unavailable_reason() {
            return Err(why);
        }
        // the measurement scope: the snapshot's unless given; a snapshot without the rows of
        // earlier intervals cannot continue cumulatively
        let m = s.header.measurements;
        let history = m.is_none_or(|m| m.rows_from == 0);
        a2.interval = match a.measurements_scope {
            Some(i) => i,
            None => !history,
        };
        if !a2.interval && !history {
            return Err(format!(
                "--measurements-scope cumulative: the snapshot holds no measurement rows before tuple {} (earlier intervals exported them); resume with --measurements-scope interval",
                m.map_or(0, |m| m.rows_from)
            ));
        }
        a2.parent = Some((s.header.state_id(), m.map(|m| m.tuples)));
        a2.start_ns = s.header.time_ns;
        Ok(a2)
    }

    /// `simoxide snapshot-info FILE`.
    pub fn info(a: &Args) -> Result<(), String> {
        let s = read(a)?;
        #[allow(unused_mut)]
        let mut out = simoxide_sim::snapshot::describe(&s);
        #[cfg(feature = "spd")]
        if let Some(d) = simoxide_spd::snapshot::decision_info(&s).map_err(|e| e.to_string())? {
            // one more member of the header object: the run's scaling decisions
            let body = serde_json::to_string_pretty(&d)
                .map_err(|e| e.to_string())?
                .replace('\n', "\n  ");
            let head = out.trim_end().strip_suffix('}').unwrap_or(&out).trim_end();
            out = format!("{head},\n  \"spd_decision\": {body}\n}}\n");
        }
        print!("{out}");
        Ok(())
    }

    /// `simoxide resume FILE --model DIR ...`: the model compiled as the snapshot says (flavour,
    /// quirks), the run restored and continued to the next pause or the end.
    pub fn resume(a: &Args) -> Result<(), String> {
        if a.snapshot.is_some()
            && a.pause.is_none()
            && a.pause_when.is_empty()
            && !a.pause_on_decision
        {
            return Err("--snapshot needs a pause (--pause-at, --until, --pause-when, ...)".into());
        }
        let s = read(a)?;
        let mut a2 = resume_args(a, &s)?;
        let (cm, _, rules_dir) = setup_with_rules(&a2)?;
        let has_rules = s.section("rules").is_some();
        let has_spd = s.section("spd").is_some();
        let spd = match has_spd && a2.rules.is_none() {
            true => Some(
                super::spd::of_snapshot(&a2, &cm, &s)?
                    .ok_or("the snapshot is of a run with scaling policies, the model has none")?,
            ),
            false => None,
        };
        a2.model_hash = super::model_metadata(&a2, &cm, spd.as_ref())?;
        let a = &a2;
        let out = Outputs {
            trace: a.trace.as_deref().map(create).transpose()?,
            tape: a.tape.as_deref().map(create).transpose()?,
        };
        let t0 = Instant::now();
        let cfg = SimConfig {
            mode: a.mode,
            ..SimConfig::default()
        };
        // the change set (--change, --transform, --rules, --enforce) is applied by the runs
        // below, between the restore and the continuation (DESIGN-SPD.md N2)
        #[cfg(feature = "reconfiguration")]
        let hpa_lines = !super::changes::hpa_settings(a)?.is_empty();
        #[cfg(not(feature = "reconfiguration"))]
        let hpa_lines = false;
        if !has_spd && super::decision_options(a) {
            return Err("scaling decisions (--pause-on-decision, --decide, --decision-file): the snapshot is of a run without scaling policies".into());
        }
        if !has_spd && (hpa_lines || !a.hpa_set.is_empty()) {
            return Err("HPA parameters (--hpa-set, `hpa` change lines): the snapshot is of a run without HPA autoscalers".into());
        }
        let r = if has_rules || (a.rules.is_some() && !has_spd) {
            let dir = rules_dir.ok_or("the snapshot is of a run with reconfiguration rules: give --rules DIR (or a run.json naming them)")?;
            if !a.enforce.is_empty() {
                return Err("--enforce: the snapshot has no scaling policies".into());
            }
            super::rules::run(a, &cm, cfg, out, &dir, Some(&s))?
        } else if has_spd {
            if a.rules.is_some() {
                return Err(
                    "reconfiguration rules and scaling policies in one run are not supported"
                        .into(),
                );
            }
            let spd = spd.expect("the snapshot's scaling policies");
            if a.flavour == FlavourKind::SlingshotExact {
                super::spd::run_exact(a, &cm, Err(&s), out, &spd)?
            } else {
                super::spd::run(a, &cm, cfg, out, &spd, Some(&s))?
            }
        } else {
            plain(a, &cm, Err(&s), out)?
        };
        report_end(a, &cm, r, t0)
    }
}

/// Starts the sampling profiler if `--profile` was given.
#[cfg(feature = "profile")]
fn profiler(a: &Args) -> Result<Option<pprof::ProfilerGuard<'static>>, String> {
    a.profile
        .as_ref()
        .map(|_| pprof::ProfilerGuard::new(4999).map_err(|e| e.to_string()))
        .transpose()
}

#[cfg(feature = "profile")]
fn write_profile(a: &Args, guard: Option<pprof::ProfilerGuard<'static>>) -> Result<(), String> {
    let (Some(g), Some(p)) = (guard, &a.profile) else {
        return Ok(());
    };
    let report = g.report().build().map_err(|e| e.to_string())?;
    let mut f = std::fs::File::create(p).map_err(|e| format!("{}: {e}", p.display()))?;
    if p.extension().is_some_and(|e| e == "svg") {
        return report.flamegraph(f).map_err(|e| e.to_string());
    }
    // folded stacks (root first), one line per distinct stack, frames as name@file:line
    use std::io::Write;
    for (frames, n) in &report.data {
        let mut names: Vec<String> = Vec::new();
        for fr in frames.frames.iter().rev() {
            for sym in fr.iter().rev() {
                let mut n = sym.name();
                if let (Some(file), Some(line)) = (&sym.filename, sym.lineno) {
                    let file = file.to_string_lossy();
                    let file = file.rsplit('/').next().unwrap_or_default().to_string();
                    n.push_str(&format!("@{file}:{line}"));
                }
                names.push(n);
            }
        }
        writeln!(f, "{} {n}", names.join(";")).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(not(feature = "profile"))]
fn profiler(a: &Args) -> Result<Option<()>, String> {
    match a.profile {
        Some(_) => Err("--profile needs the `profile` feature".into()),
        None => Ok(None),
    }
}

#[cfg(not(feature = "profile"))]
fn write_profile(_: &Args, _: Option<()>) -> Result<(), String> {
    Ok(())
}

fn load_bench(a: &Args) -> Result<(), String> {
    let guard = profiler(a)?;
    let t0 = Instant::now();
    let mut n = 0;
    for _ in 0..a.runs.max(1) {
        let (cm, _) = setup(a)?;
        n += cm.progs.len();
    }
    let dt = t0.elapsed().as_secs_f64();
    write_profile(a, guard)?;
    println!(
        "{} x load+compile in {:.3} s: {:.1} µs each ({} programs)",
        a.runs,
        dt,
        dt * 1e6 / a.runs.max(1) as f64,
        n / a.runs.max(1)
    );
    Ok(())
}

fn bench(a: &Args) -> Result<(), String> {
    if a.rules.is_some() || a.rules_log.is_some() || a.qvto_mode.is_some() {
        return Err("--rules, --rules-log and --qvto-mode are options of `run`".into());
    }
    let t0 = Instant::now();
    let (cm, cfg) = setup(a)?;
    let t_compile = t0.elapsed();
    let configs: Vec<SimConfig> = (0..a.runs)
        .map(|i| SimConfig {
            seed: cfg.seed + i as i64,
            store_measurements: false,
            ..cfg.clone()
        })
        .collect();
    let guard = profiler(a)?;
    let t1 = Instant::now();
    let res = simoxide_sim::run_batch(&cm, &configs, a.threads);
    let dt = t1.elapsed().as_secs_f64();
    write_profile(a, guard)?;
    let mut events = 0u64;
    let mut requests = 0i64;
    let mut ok = 0usize;
    for r in &res {
        match r {
            Ok(r) => {
                events += r.events;
                requests += r.main_count;
                ok += 1;
            }
            Err(e) => eprintln!("run failed: {e}"),
        }
    }
    println!(
        "mode {}: load+compile {:.2} ms; {} runs ({} ok) in {:.3} s on {} thread(s): {:.1} runs/s, {:.3e} events/s, {:.3e} requests/s, {:.0} events/run",
        a.mode,
        t_compile.as_secs_f64() * 1e3,
        a.runs,
        ok,
        dt,
        a.threads,
        a.runs as f64 / dt,
        events as f64 / dt,
        requests as f64 / dt,
        events as f64 / a.runs.max(1) as f64
    );
    Ok(())
}

fn main() -> ExitCode {
    let a = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let r = match a.cmd.as_str() {
        "run" => run(&a),
        "bench" => bench(&a),
        "load-bench" => load_bench(&a),
        "resume" => snap::resume(&a),
        "snapshot-info" => snap::info(&a),
        "explore" => explore::run(&a),
        _ => Err(USAGE.to_string()),
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

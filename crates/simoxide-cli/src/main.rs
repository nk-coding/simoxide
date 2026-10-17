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
  simoxide resume FILE --model <dir> [--run-json run.json] [--trace out.jsonl] [--tape out.jsonl]
             [--measurements out.csv] [--pause-at T ... --snapshot FILE2] [--until T]
             [--pause-when COND]... [--spd-log ...] [--spd-quirks ...] [--rules DIR]
             [--rules-log ...] [--qvto-mode ...] [--timeout SECONDS]   (continue a snapshot on
             the model it was taken of: mode, flavour and quirks come from the snapshot; outputs
             get the continuation only; --until T = --pause-at T)
             [--change FILE.changes]... [--transform X.qvto]...   (change the model before it
             continues, in the given order: a change file, a QVTo transformation run once;
             `reconfiguration` feature; --rules DIR with other rules than the snapshot's replaces
             them, on a snapshot without rules adds them)
             [--enforce POLICY,...]   (adjust by these scaling policies at the resume, past
             their triggers and constraints: Slingshot's incoming policies)
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
    /// `explore`: `--variant NAME=FILE`, `--metrics`.
    variants: Vec<(String, Option<PathBuf>)>,
    metrics: Vec<String>,
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
        variants: Vec::new(),
        metrics: Vec::new(),
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
    Ok(a)
}

fn setup(a: &Args) -> Result<(CompiledModel, SimConfig), String> {
    setup_with_rules(a).map(|(cm, cfg, _)| (cm, cfg))
}

/// [`setup`] and the reconfiguration rules folder (`--rules`, else run.json's).
fn setup_with_rules(a: &Args) -> Result<(CompiledModel, SimConfig, Option<PathBuf>), String> {
    let run_json = a.run_json.clone().or_else(|| {
        let p = a.model.as_ref()?.join("run.json");
        p.exists().then_some(p)
    });
    let dir: PathBuf = match (&a.model, &run_json) {
        (Some(m), _) => m.clone(),
        (None, Some(r)) => r.parent().unwrap_or(Path::new(".")).to_path_buf(),
        _ => return Err(USAGE.into()),
    };
    let spec = match &run_json {
        Some(p) => RunSpec::load(p)?,
        None => RunSpec::default(),
    };
    let model = spec.load_model(&dir)?;
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
    Ok((cm, cfg, rules))
}

fn create(p: &Path) -> Result<Box<dyn std::io::Write>, String> {
    let f = std::fs::File::create(p).map_err(|e| format!("{}: {e}", p.display()))?;
    Ok(Box::new(BufWriter::new(f)))
}

fn run(a: &Args) -> Result<(), String> {
    if a.snapshot.is_some() && a.pause.is_none() && a.pause_when.is_empty() {
        return Err(
            "--snapshot needs a pause (--pause-at, --pause-before, --pause-events, --pause-when)"
                .into(),
        );
    }
    if !a.changes.is_empty() || !a.enforce.is_empty() {
        return Err("--change, --transform and --enforce are options of `resume`".into());
    }
    let (cm, cfg, rules_dir) = setup_with_rules(a)?;
    for w in &cm.warnings {
        eprintln!("warning: {w}");
    }
    let out = Outputs {
        trace: a.trace.as_deref().map(create).transpose()?,
        tape: a.tape.as_deref().map(create).transpose()?,
    };
    let t0 = Instant::now();
    let spd = spd::of(a, &cm)?;
    if let Some(dir) = rules_dir {
        if spd.is_some() {
            return Err("reconfiguration rules and scaling policies in one run are not supported (use --no-spd)".into());
        }
        if a.flavour.is_slingshot() {
            return Err("reconfiguration rules run in the SimuLizar flavour".into());
        }
        let t0 = Instant::now();
        let r = rules::run(a, &cm, cfg, out, &dir, None)?;
        return report_end(a, r, t0);
    }
    let r = match spd {
        Some(_) if a.flavour.is_slingshot() => {
            return Err(
                "scaling policies in the Slingshot flavour are phase P3 (use --no-spd to ignore the .spd)"
                    .into(),
            );
        }
        Some(s) => spd::run(a, &cm, cfg, out, &s, None)?,
        None if a.pause.is_some() || a.snapshot.is_some() || !a.pause_when.is_empty() => {
            snap::plain(a, &cm, Ok(cfg), out)?
        }
        None => End::Finished(simoxide_sim::run(&cm, cfg, out).map_err(|e| e.to_string())?),
    };
    report_end(a, r, t0)
}

/// How a run (or a resumed one) ended: at its end, or paused with a snapshot written.
enum End {
    Finished(RunResult),
    Paused {
        now_ns: i64,
        events: u64,
        measurements: simoxide_sim::Measurements,
        snapshot: Option<(PathBuf, u64)>,
        /// The pause condition that held (`--pause-when`).
        reason: Option<String>,
    },
}

/// Reports a run that ended or paused.
fn report_end(a: &Args, r: End, t0: Instant) -> Result<(), String> {
    match r {
        End::Finished(r) => report(a, r, t0),
        End::Paused {
            now_ns,
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
            let snap = match snapshot {
                Some((p, n)) => format!(" snapshot={} ({n} bytes)", p.display()),
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
    use simoxide_sim::{CompiledModel, Mode, Outputs, SimConfig, Simulation, Status};
    use simoxide_spd::log::JsonlLog;
    #[cfg(not(feature = "reconfiguration"))]
    use simoxide_spd::sim::NoAdjustment;
    use simoxide_spd::sim::{SpdRun, spd_of};
    use simoxide_spd::{Quirks, Spd};

    pub fn of(a: &Args, cm: &CompiledModel) -> Result<Option<Spd>, String> {
        of_with(a, cm, None)
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
        let q = match (&a.spd_quirks, q) {
            (Some(s), _) => Quirks::parse(s)?,
            (None, Some(q)) => q,
            (None, None) => Quirks::fixed(),
        };
        let s = spd_of(&cm.model, q).map_err(|e| e.to_string())?;
        if let Some(s) = &s
            && let Some(d) = s.dropped()
        {
            eprintln!("warning: SPD dropped (Q11 compat, as Slingshot): {d}");
        }
        Ok(s)
    }

    fn go<C: Compat>(
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
        let mut r = match restore {
            Some(snap) => {
                let mut r =
                    SpdRun::<C>::restore_snapshot(snap, cm, spd, out).map_err(|e| e.to_string())?;
                #[cfg(feature = "reconfiguration")]
                {
                    adj.run
                        .restore_section(&r.sim, snap)
                        .map_err(|e| e.to_string())?;
                    if super::changes::with_qvto(a, &mut r.sim, &mut adj.run)? {
                        r.model_changed();
                    }
                }
                #[cfg(not(feature = "reconfiguration"))]
                {
                    carry = super::changes::plain(a, &mut r.sim, Some(snap))?;
                    if carry.changed {
                        r.model_changed();
                    }
                }
                if !a.enforce.is_empty() {
                    let names: Vec<&str> = a.enforce.iter().map(String::as_str).collect();
                    r.enforce_policies(&names).map_err(|e| e.to_string())?;
                }
                r
            }
            None => {
                let sim = Simulation::<C>::create(cm, cfg, out).map_err(|e| e.to_string())?;
                SpdRun::new(sim, spd).map_err(|e| e.to_string())?
            }
        };
        let file = a.spd_log.as_deref().map(create).transpose()?;
        let sink: Box<dyn std::io::Write> = file.unwrap_or_else(|| Box::new(std::io::sink()));
        let mut log = JsonlLog::new(sink);
        log.all = a.spd_log_all;
        let mut watch = super::snap::watch(a);
        let (status, before) = match &mut watch {
            Some(w) => {
                w.arm(&r.sim)?;
                let st = r
                    .run_while(&mut adj, &mut log, |s| w.keep_going(s))
                    .map_err(|e| e.to_string())?;
                (st, None)
            }
            None => {
                let before = super::snap::before_ns(a)?;
                let st = r
                    .run_until(&mut adj, &mut log, before)
                    .map_err(|e| e.to_string())?;
                (st, before)
            }
        };
        if a.spd_log.is_some() {
            log.finish().map_err(|e| format!("--spd-log: {e}"))?;
        }
        if status == Status::Paused && (before.is_some() || watch.is_some()) {
            let snapshot = match &a.snapshot {
                Some(p) => {
                    let opts = super::snap::options(a);
                    let mut s = r.snapshot_value(opts.encoding).map_err(|e| e.to_string())?;
                    #[cfg(not(feature = "reconfiguration"))]
                    s.sections.extend(carry.sections(opts.encoding)?);
                    #[cfg(feature = "reconfiguration")]
                    if adj.run.is_started() {
                        s.sections.push(
                            adj.run
                                .snapshot_section(opts.encoding)
                                .map_err(|e| e.to_string())?,
                        );
                    }
                    Some(super::snap::write(p, &s)?)
                }
                None => None,
            };
            let st = r.sim.state();
            return Ok(End::Paused {
                now_ns: st.now_ns(),
                events: st.events(),
                measurements: st.measurements().clone(),
                snapshot,
                reason: watch.and_then(|w| w.fired().map(|f| f.reason.clone())),
            });
        }
        let (res, _, s) = r.finish().map_err(|e| e.to_string())?;
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

    pub fn run(
        a: &Args,
        cm: &CompiledModel,
        cfg: SimConfig,
        out: Outputs,
        spd: &Spd,
        restore: Option<&Snapshot>,
    ) -> Result<End, String> {
        match cfg.mode {
            #[cfg(feature = "exact")]
            Mode::Exact => go::<simoxide_sim::Exact>(a, cm, cfg, out, spd, restore),
            Mode::Fast => go::<simoxide_sim::Fast>(a, cm, cfg, out, spd, restore),
            #[cfg(not(feature = "exact"))]
            m => Err(m.unavailable_reason().unwrap_or_default()),
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
}

/// SimuLizar's reconfiguration rules (`reconfiguration` feature): the run's reconfiguration
/// process runs the rules of the folder (DESIGN-SPD R1).
#[cfg(feature = "reconfiguration")]
mod rules {
    use super::{Args, End, create};
    use simoxide_reconf::{QvtoCore, RuleSet, RulesHost, RulesRun, RunConfig};
    use simoxide_sim::compat::Compat;
    use simoxide_sim::snapshot::Snapshot;
    use simoxide_sim::{CompiledModel, Mode, Outputs, Pause, SimConfig, Simulation, Status};
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
                if old == new {
                    RulesRun::<C>::restore_snapshot(snap, cm, host, out)
                } else {
                    eprintln!("rules replaced: {old:?} -> {new:?}");
                    RulesRun::<C>::restore_snapshot_replacing(snap, cm, host, out)
                }
                .map_err(|e| e.to_string())?
            }
            Some(snap) => {
                // a snapshot without rules: they are added at the resume
                let sim =
                    Simulation::<C>::restore_snapshot(snap, cm, out).map_err(|e| e.to_string())?;
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
        if let Some(snap) = restore {
            run = super::changes::with_rules(a, run, snap)?;
        }
        let mut watch = super::snap::watch(a);
        let status = match &mut watch {
            Some(w) => {
                w.arm(run.sim())?;
                run.run_while(|s| w.keep_going(s))
            }
            None => run.run_until(a.pause.unwrap_or(Pause::Never)),
        }
        .map_err(|e| e.to_string())?;
        let paused = status == Status::Paused && (a.pause.is_some() || watch.is_some());
        let (end, mut host) = if paused {
            // the records so far go to this run's log; the snapshot holds none
            let records = run.host_mut().take_records();
            let snapshot = match &a.snapshot {
                Some(p) => {
                    let f =
                        std::fs::File::create(p).map_err(|e| format!("{}: {e}", p.display()))?;
                    let info = run
                        .snapshot_with(std::io::BufWriter::new(f), &super::snap::options(a))
                        .map_err(|e| e.to_string())?;
                    Some((p.clone(), info.bytes))
                }
                None => None,
            };
            let st = run.sim().state();
            let end = End::Paused {
                now_ns: st.now_ns(),
                events: st.events(),
                measurements: st.measurements().clone(),
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
    use super::{Args, ChangeArg};
    use simoxide_reconf::{
        ChangeFile, FailedRunChanges, Imports, QvtMode, QvtoCore, QvtoRun, RULES, ResumeChanges,
        RulesRun, RunConfig, StepReport,
    };
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

        pub fn sections(&self, enc: Encoding) -> Result<Vec<Section>, String> {
            match &self.qvto {
                Some(q) if q.is_started() => {
                    Ok(vec![q.snapshot_section(enc).map_err(|e| e.to_string())?])
                }
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
            c = match ch {
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
            };
        }
        Ok(c)
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
        let steps = build(a, &core)?
            .apply(sim, &mut q)
            .map_err(|e| e.to_string())?;
        report(&steps);
        Ok(Carry { qvto: Some(q) })
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
    ) -> Result<bool, String> {
        if a.changes.is_empty() {
            return Ok(false);
        }
        let steps = build(a, q.core())?
            .apply(sim, q)
            .map_err(|e| e.to_string())?;
        report(&steps);
        Ok(!steps.is_empty())
    }

    /// Applies the change steps to a restored run with rules, in its host's QVTo state.
    pub fn with_rules<'m, C: Compat, F: Flavour>(
        a: &Args,
        run: RulesRun<'m, C, F>,
        snap: &Snapshot,
    ) -> Result<RulesRun<'m, C, F>, String> {
        let (mut sim, mut host) = run.into_parts();
        if snap.section(RULES).is_none() {
            // rules added to a run whose QVTo state (if any) is the section `qvto`
            host.qvto()
                .restore_section(&sim, snap)
                .map_err(|e| e.to_string())?;
        }
        if !a.changes.is_empty() {
            let c = build(a, host.qvto().core())?;
            let steps = c.apply(&mut sim, host.qvto()).map_err(|e| e.to_string())?;
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

        pub fn sections(&self, enc: Encoding) -> Result<Vec<Section>, String> {
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

    pub fn plain<C: Compat, F: Flavour>(
        a: &Args,
        _: &mut Simulation<'_, C, F>,
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

    /// Writes `s` to `p`; the path and the number of bytes.
    pub fn write(p: &Path, s: &Snapshot) -> Result<(PathBuf, u64), String> {
        let mut bytes = Vec::new();
        s.write(&mut bytes).map_err(|e| e.to_string())?;
        std::fs::write(p, &bytes).map_err(|e| format!("{}: {e}", p.display()))?;
        Ok((p.to_path_buf(), bytes.len() as u64))
    }

    fn go<C: Compat, F: Flavour>(
        a: &Args,
        cm: &CompiledModel,
        start: Result<SimConfig, &Snapshot>,
        out: Outputs,
    ) -> Result<End, String> {
        let mut carry = super::changes::Carry::none();
        let mut sim = match start {
            Ok(cfg) => Simulation::<C, F>::create(cm, cfg, out).map_err(|e| e.to_string())?,
            Err(s) => {
                if !a.enforce.is_empty() {
                    return Err("--enforce: the snapshot has no scaling policies".into());
                }
                let mut sim =
                    Simulation::<C, F>::restore_snapshot(s, cm, out).map_err(|e| e.to_string())?;
                carry = super::changes::plain(a, &mut sim, Some(s))?;
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
            return sim.finish().map(End::Finished).map_err(|e| e.to_string());
        }
        let snapshot = match &a.snapshot {
            Some(p) => {
                let enc = options(a).encoding;
                let s = sim
                    .snapshot_value(enc, carry.sections(enc)?)
                    .map_err(|e| e.to_string())?;
                Some(write(p, &s)?)
            }
            None => None,
        };
        let st = sim.state();
        Ok(End::Paused {
            now_ns: st.now_ns(),
            events: st.events(),
            measurements: st.measurements().clone(),
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
            (_, FlavourKind::SlingshotExact) => Err(
                "snapshots of the Slingshot-exact engine are not supported yet (it has no pause and \
                 resume; DESIGN-SPD.md 5.7, \"N1 as built\")"
                    .into(),
            ),
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
        Ok(a2)
    }

    /// `simoxide snapshot-info FILE`.
    pub fn info(a: &Args) -> Result<(), String> {
        let s = read(a)?;
        print!("{}", simoxide_sim::snapshot::describe(&s));
        Ok(())
    }

    /// `simoxide resume FILE --model DIR ...`: the model compiled as the snapshot says (flavour,
    /// quirks), the run restored and continued to the next pause or the end.
    pub fn resume(a: &Args) -> Result<(), String> {
        if a.snapshot.is_some() && a.pause.is_none() && a.pause_when.is_empty() {
            return Err("--snapshot needs a pause (--pause-at, --until, --pause-when, ...)".into());
        }
        let s = read(a)?;
        let a2 = resume_args(a, &s)?;
        let a = &a2;
        let (cm, _, rules_dir) = setup_with_rules(a)?;
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
        let has_rules = s.section("rules").is_some();
        let has_spd = s.section("spd").is_some();
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
            let spd = super::spd::of_snapshot(a, &cm, &s)?
                .ok_or("the snapshot is of a run with scaling policies, the model has none")?;
            super::spd::run(a, &cm, cfg, out, &spd, Some(&s))?
        } else {
            plain(a, &cm, Err(&s), out)?
        };
        report_end(a, r, t0)
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

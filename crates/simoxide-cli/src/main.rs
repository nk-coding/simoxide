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
             [--flavour simulizar|slingshot]   (default simulizar; slingshot: Slingshot's
             semantics, fast mode, docs/spec/slingshot)
             [--quirks LIST]   (Slingshot compat flags: Q1..Q37, all, fast, none, -Qn;
             default none = the fixed behaviour; --quirks list prints them)
             [--spd-log spd.jsonl] [--spd-log-all] [--spd-quirks fixed|slingshot|Q2,Q12,...]
             [--no-spd]   (scaling policies of a .spd next to the model; `spd` feature;
             P1: decisions only, adjustments are not executed)
             [--rules DIR] [--rules-log rules.jsonl] [--qvto-mode auto|fast|compat]
             (SimuLizar's reconfiguration rules: the .qvto files of DIR in file-name order, or
             run.json's reconfiguration_rules; `reconfiguration` feature; QVTo's compat mode
             needs a build with `exact` or `compat`)
             [--pause-at T | --pause-before T | --pause-events N] [--snapshot FILE]
             [--snapshot-format postcard|json]   (pause between events and write a snapshot
             of the run: `snapshot` feature, docs/guide/snapshots.md)
  simoxide resume FILE --model <dir> [--run-json run.json] [--trace out.jsonl] [--tape out.jsonl]
             [--measurements out.csv] [--pause-at T ... --snapshot FILE2] [--until T]
             [--spd-log ...] [--spd-quirks ...] [--rules DIR] [--rules-log ...] [--qvto-mode ...]
             [--timeout SECONDS]   (continue a snapshot on the model it was taken of: mode,
             flavour and quirks come from the snapshot; outputs get the continuation only;
             --until T = --pause-at T)
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
            && matches!(a.cmd.as_str(), "resume" | "snapshot-info")
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
            "--threads" => a.threads = val()?.parse().map_err(|e| format!("--threads: {e}"))?,
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
    if a.snapshot.is_some() && a.pause.is_none() {
        return Err("--snapshot needs a pause (--pause-at, --pause-before, --pause-events)".into());
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
        if a.flavour == FlavourKind::Slingshot {
            return Err("reconfiguration rules run in the SimuLizar flavour".into());
        }
        let t0 = Instant::now();
        let r = rules::run(a, &cm, cfg, out, &dir, None)?;
        return report_end(a, r, t0);
    }
    let r = match spd {
        Some(_) if a.flavour == FlavourKind::Slingshot => {
            return Err(
                "scaling policies in the Slingshot flavour are phase P3 (use --no-spd to ignore the .spd)"
                    .into(),
            );
        }
        Some(s) => spd::run(a, &cm, cfg, out, &s, None)?,
        None if a.pause.is_some() || a.snapshot.is_some() => snap::plain(a, &cm, Ok(cfg), out)?,
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
        } => {
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
    use simoxide_spd::sim::{NoAdjustment, SpdRun, spd_of};
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
        let mut r = match restore {
            Some(snap) => {
                SpdRun::<C>::restore_snapshot(snap, cm, spd, out).map_err(|e| e.to_string())?
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
        let before = super::snap::before_ns(a)?;
        let status = r
            .run_until(&mut NoAdjustment, &mut log, before)
            .map_err(|e| e.to_string())?;
        if a.spd_log.is_some() {
            log.finish().map_err(|e| format!("--spd-log: {e}"))?;
        }
        if status == Status::Paused && before.is_some() {
            let snapshot = match &a.snapshot {
                Some(p) => {
                    let f =
                        std::fs::File::create(p).map_err(|e| format!("{}: {e}", p.display()))?;
                    let info = r
                        .snapshot_with(std::io::BufWriter::new(f), &super::snap::options(a))
                        .map_err(|e| e.to_string())?;
                    Some((p.clone(), info.bytes))
                }
                None => None,
            };
            let st = r.sim.state();
            return Ok(End::Paused {
                now_ns: st.now_ns(),
                events: st.events(),
                measurements: st.measurements().clone(),
                snapshot,
            });
        }
        let (res, _, s) = r.finish().map_err(|e| e.to_string())?;
        eprintln!(
            "spd: policies={} evaluations={} requests={} (adjustments are not executed yet)",
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
    use simoxide_reconf::{QvtMode, QvtoCore, RuleSet, RulesHost, RulesRun, RunConfig};
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
        let mode = match a.qvto_mode.as_deref() {
            None | Some("auto") => QvtMode::Auto,
            Some("fast") => QvtMode::Fast,
            Some("compat") => QvtMode::Compat,
            Some(m) => {
                return Err(format!(
                    "--qvto-mode: expected auto, fast or compat, got {m}"
                ));
            }
        };
        let cfg_q = RunConfig {
            mode,
            ..RunConfig::default()
        };
        let mut host = RulesHost::new(&core, rules, cfg_q).map_err(|e| e.to_string())?;
        if a.rules_log.is_some() {
            host = host.keep_records();
        }
        let mut run = match restore {
            Some(snap) => {
                RulesRun::<C>::restore_snapshot(snap, cm, host, out).map_err(|e| e.to_string())?
            }
            None => {
                let sim = Simulation::<C>::create(cm, cfg, out).map_err(|e| e.to_string())?;
                RulesRun::new(sim, host)
            }
        };
        let pause = a.pause.unwrap_or(Pause::Never);
        let status = run.run_until(pause).map_err(|e| e.to_string())?;
        let paused = status == Status::Paused && a.pause.is_some();
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

/// Pausing, snapshots and `simoxide resume` (docs/guide/snapshots.md).
mod snap {
    use super::{Args, End, create, report_end, setup_with_rules};
    use simoxide_sim::compat::Compat;
    use simoxide_sim::snapshot::{Encoding, Snapshot, SnapshotOptions};
    use simoxide_sim::{
        CompiledModel, Flavour, FlavourKind, Mode, Outputs, Pause, Quirks, SimConfig, SimuLizar,
        Simulation, Slingshot, Status,
    };
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

    fn go<C: Compat, F: Flavour>(
        a: &Args,
        cm: &CompiledModel,
        start: Result<SimConfig, &Snapshot>,
        out: Outputs,
    ) -> Result<End, String> {
        let mut sim = match start {
            Ok(cfg) => Simulation::<C, F>::create(cm, cfg, out).map_err(|e| e.to_string())?,
            Err(s) => {
                Simulation::<C, F>::restore_snapshot(s, cm, out).map_err(|e| e.to_string())?
            }
        };
        let status = match a.pause {
            Some(p) => sim.run_until(p).map_err(|e| e.to_string())?,
            None => Status::Finished,
        };
        if status == Status::Finished {
            return sim.finish().map(End::Finished).map_err(|e| e.to_string());
        }
        let snapshot = match &a.snapshot {
            Some(p) => {
                let f = std::fs::File::create(p).map_err(|e| format!("{}: {e}", p.display()))?;
                let info = sim
                    .snapshot_with(std::io::BufWriter::new(f), &options(a), Vec::new())
                    .map_err(|e| e.to_string())?;
                Some((p.clone(), info.bytes))
            }
            None => None,
        };
        let st = sim.state();
        Ok(End::Paused {
            now_ns: st.now_ns(),
            events: st.events(),
            measurements: st.measurements().clone(),
            snapshot,
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
            #[cfg(not(feature = "exact"))]
            (m, _) => Err(m.unavailable_reason().unwrap_or_default()),
        }
    }

    fn read(a: &Args) -> Result<Snapshot, String> {
        let p = a
            .file
            .as_ref()
            .ok_or("which snapshot? (resume FILE --model DIR)")?;
        let f = std::fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()))?;
        Snapshot::read(std::io::BufReader::new(f)).map_err(|e| format!("{}: {e}", p.display()))
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
        if a.snapshot.is_some() && a.pause.is_none() {
            return Err("--snapshot needs a pause (--pause-at, --until, ...)".into());
        }
        let s = read(a)?;
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
        // N2: a change set applied here, between the restore and the continuation
        let r = if s.section("rules").is_some() {
            let dir = rules_dir.ok_or("the snapshot is of a run with reconfiguration rules: give --rules DIR (or a run.json naming them)")?;
            super::rules::run(a, &cm, cfg, out, &dir, Some(&s))?
        } else if s.section("spd").is_some() {
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

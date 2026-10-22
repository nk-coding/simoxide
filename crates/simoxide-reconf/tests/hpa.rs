//! The HPA controller end to end (`docs/spec/hpa.md`, `DESIGN-SPD.md` §5.10): Kubernetes-style
//! autoscaling of an SPD elastic infrastructure through Slingshot's `MainTransformation`
//! (absolute adjustments), in the SimuLizar and Slingshot flavours.
//!
//! The model: the Slingshot V4 scenario (`reference/slingshot/models/spd_v4_cpu`: an open
//! workload, `Exp(0.5)` s demands on a 1-core application server behind a load balancer), with
//! one target group and no policies in its SPD, and an `.hpa.json` next to it. Its utilisation
//! monitors get a `TimeDriven` processing type in the SimuLizar flavour (as `spd_adjust.rs`
//! does); the HPA itself reads the CPUs' busy times from the engine. Load steps and ramps are
//! change files (`arrival-rate`) applied at pauses.

use serde_json::Value;
use simoxide_reconf::{ChangeFile, QvtoAdjuster, QvtoCore, SpdAdjust};
use simoxide_sim::flavour::{Flavour, Quirks as SsQuirks};
use simoxide_sim::snapshot::{Encoding, Snapshot};
use simoxide_sim::{
    Compat, CompiledModel, Fast, FlavourConfig, Mode, Outputs, RunResult, RunSpec, Simulation,
    Slingshot,
};
use simoxide_spd::hpa::{self, Hpa, HpaConfig, HpaSummary};
use simoxide_spd::log::{NoLog, VecLog};
use simoxide_spd::sim::{SpdRun, spd_of};
use simoxide_spd::{Quirks, Spd};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const S: i64 = 1_000_000_000;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn core() -> &'static QvtoCore {
    static CORE: OnceLock<QvtoCore> = OnceLock::new();
    CORE.get_or_init(|| QvtoCore::new().expect("core"))
}

const SPD: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<spd:SPD xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:spd="http://palladiosimulator.org/ScalingPolicyDefinition/1.0" xmlns:targets="http://palladiosimulator.org/ScalingPolicyDefinition/Targets/1.0" id="_PKdecLwZEe6i2ICnm9lfkw">
  <targetGroups xsi:type="targets:ElasticInfrastructure" id="_DC8fM8pgEe6u3p81um7oyQ" entityName="app">
    <unit href="My.resourceenvironment#_GAeeUCpXEeKNWc2UWjbmuw"/>
  </targetGroups>
</spd:SPD>
"#;

/// An HPA on the group `app`: CPU 50 %, 1..10 replicas; `controller`, `spec` and `simoxide`
/// add fields.
fn hpa_json(controller: &str, spec: &str, simoxide: &str) -> String {
    format!(
        r#"{{"controller":{{"syncPeriod":"15s"{controller}}},
  "items":[{{"apiVersion":"autoscaling/v2","kind":"HorizontalPodAutoscaler",
    "metadata":{{"name":"app"}},
    "spec":{{"scaleTargetRef":{{"kind":"ElasticInfrastructure","name":"app"}},
      "minReplicas":1,"maxReplicas":10,
      "metrics":[{{"type":"Resource","resource":{{"name":"cpu","target":{{"type":"Utilization","averageUtilization":50}}}}}}]
      {spec}}},
    "simoxide":{{{simoxide}}}}}]}}"#
    )
}

/// The model directory: the V4 model with `rate` arrivals per second (`Exp(rate)`), the HPA
/// configuration `hpa`; `simulizar`: utilisation monitors with a `TimeDriven` processing type.
fn model_dir(rate: f64, hpa: &str, simulizar: bool) -> PathBuf {
    const UTIL: &str = "_QIb6cikUEeSuf8LV7cHLgA";
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let src = root().join("reference/slingshot/models/spd_v4_cpu");
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("hpa-{}-{n}", std::process::id()))
        .join("hpa_cpu");
    std::fs::create_dir_all(&out).unwrap();
    for e in std::fs::read_dir(&src).unwrap() {
        let p = e.unwrap().path();
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let mut text = std::fs::read_to_string(&p).unwrap();
        if name.ends_with(".usagemodel") {
            text = text.replace("Exp(0.8)", &format!("Exp({rate})"));
        }
        if name.ends_with(".spd") {
            text = SPD.into();
        }
        if simulizar && name.ends_with(".monitorrepository") {
            let mut util = false;
            let mut lines = Vec::new();
            for l in text.lines() {
                let mut l = l.to_string();
                if l.contains(UTIL) {
                    util = true;
                } else if util && l.contains("processingType") {
                    l = l.replace(
                        "xsi:type=\"monitorrepository:FeedThrough\"",
                        "xsi:type=\"monitorrepository:TimeDriven\" windowLength=\"10.0\" windowIncrement=\"10.0\"",
                    );
                    util = false;
                }
                lines.push(l);
            }
            text = lines.join("\n");
        }
        std::fs::write(out.join(&name), text).unwrap();
    }
    std::fs::write(out.join("default.hpa.json"), hpa).unwrap();
    out
}

struct Setup {
    cm: CompiledModel,
    spd: Spd,
    cfg: HpaConfig,
}

fn setup_flavour(dir: &Path, flavour: FlavourConfig) -> Setup {
    let mut model = RunSpec::default().load_model(dir).unwrap();
    let cfg = HpaConfig::load_dir(dir).unwrap().expect("an .hpa.json");
    hpa::attach(&mut model, &cfg).unwrap();
    let spd = spd_of(&model, Quirks::fixed()).unwrap().expect("an SPD");
    let cm = CompiledModel::compile_flavour(model, flavour).unwrap();
    Setup { cm, spd, cfg }
}

fn setup(dir: &Path) -> Setup {
    setup_flavour(dir, FlavourConfig::SIMULIZAR)
}

fn adjuster() -> QvtoAdjuster {
    QvtoAdjuster::spd(core(), &SpdAdjust::new(&Quirks::fixed())).unwrap()
}

fn sim_cfg(seed: i64, until: i64) -> simoxide_sim::SimConfig {
    let mut cfg = RunSpec::default().sim_config("hpa");
    cfg.seed = seed;
    cfg.max_sim_time = until;
    cfg.mode = Mode::Fast;
    cfg
}

/// Applies `arrival-rate` to the run.
fn set_rate<C: Compat, F: Flavour>(r: &mut SpdRun<'_, '_, C, F>, rate: f64) {
    ChangeFile::parse(&format!("arrival-rate ClosedWorkloadUsageScenario {rate}"))
        .unwrap()
        .apply(&mut r.sim)
        .unwrap();
    r.model_changed();
}

/// A run with load steps `(time s, arrival rate)` to the end: the decision log, the summary,
/// the result.
fn run_steps<F: Flavour>(
    s: &Setup,
    hpa: &Hpa,
    seed: i64,
    until: i64,
    steps: &[(i64, f64)],
) -> (VecLog, Vec<HpaSummary>, RunResult) {
    let sim =
        Simulation::<Fast, F>::create(&s.cm, sim_cfg(seed, until), Outputs::default()).unwrap();
    let mut r = SpdRun::new(sim, &s.spd).unwrap().with_hpa(hpa).unwrap();
    let mut adj = adjuster();
    let mut log = VecLog::default();
    for &(t, rate) in steps {
        r.run_until(&mut adj, &mut log, Some(t * S)).unwrap();
        set_rate(&mut r, rate);
    }
    r.run(&mut adj, &mut log).unwrap();
    let summary = r.hpa_summary();
    let (res, _, _) = r.finish().unwrap();
    (log, summary, res)
}

fn records(log: &VecLog, k: &str) -> Vec<Value> {
    log.to_jsonl()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .filter(|v| v["k"] == k)
        .collect()
}

fn t(v: &Value) -> f64 {
    v["t"].as_f64().unwrap()
}

fn int(v: &Value, k: &str) -> i64 {
    v[k].as_i64().unwrap()
}

/// The rescales of the `hpa` records: (time, current, desired).
fn rescales(log: &VecLog) -> Vec<(f64, i64, i64)> {
    records(log, "hpa")
        .iter()
        .filter(|r| int(r, "desired") != int(r, "current"))
        .map(|r| (t(r), int(r, "current"), int(r, "desired")))
        .collect()
}

/// Kubernetes' scale-down stabilisation (HPA-5.1) holds in the log: every scale-down goes to
/// at least the largest recommendation of the window `(t - window, t]`.
fn check_stabilisation(log: &VecLog, window: f64) {
    let recs = records(log, "hpa");
    for r in &recs {
        let (now, cur, des) = (t(r), int(r, "current"), int(r, "desired"));
        if des >= cur {
            continue;
        }
        let max = recs
            .iter()
            .filter(|x| t(x) > now - window && t(x) <= now)
            .filter_map(|x| x["recommended"].as_i64())
            .max()
            .unwrap();
        assert!(
            des >= max.max(1),
            "t = {now}: down to {des}, but {max} recommended in the window"
        );
    }
}

/// The directions of the rescales, with repeats removed (`+-` = up, then down).
fn directions(log: &VecLog) -> String {
    let mut s = String::new();
    for (_, cur, des) in rescales(log) {
        let c = if des > cur { '+' } else { '-' };
        if !s.ends_with(c) {
            s.push(c);
        }
    }
    s
}

/// A step from 0.4 to 4 offered cores at 300 s and back at 900 s (target 50 %: 8 replicas):
/// the HPA reaches 8 or more replicas within four syncs (Kubernetes' legacy rate limit at most
/// doubles per sync), holds them while the load stays (no rescale between 450 s and 900 s),
/// waits the 300 s stabilisation window before it scales in, then stays small.
#[test]
fn a_load_step_scales_out_within_syncs_and_in_after_the_window() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let dir = model_dir(0.2, &hpa_json("", "", ""), true);
    let s = setup(&dir);
    let hpa = Hpa::new(&s.cfg, &s.spd).unwrap();
    let (log, sum, res) =
        run_steps::<simoxide_sim::SimuLizar>(&s, &hpa, 1, 1800, &[(300, 2.0), (900, 0.2)]);
    let recs = records(&log, "hpa");
    assert_eq!(recs.len(), 120, "a sync every 15 s");
    let at = |time: f64| recs.iter().find(|r| t(r) == time).unwrap();
    // the first sync after the step scales out; 8 or more replicas at the fourth
    assert!(int(at(315.0), "desired") > int(at(315.0), "current"));
    assert!(int(at(360.0), "desired") >= 8, "{}", at(360.0));
    // while the load stays: no rescale
    let all = rescales(&log);
    assert!(
        !all.iter().any(|&(time, ..)| (450.0..900.0).contains(&time)),
        "{all:?}"
    );
    // scale-in only after the window: the last high recommendation is at 900 s
    check_stabilisation(&log, 300.0);
    let first_down = all.iter().find(|&&(_, c, d)| d < c).unwrap().0;
    assert!(first_down > 1000.0, "{first_down}");
    // ... and down to at most 3 by the end (0.4 offered cores at 50 % is 1)
    assert!(sum[0].ready <= 3, "{sum:?}");
    // up, then down: no flapping
    let dirs = directions(&log);
    assert!(matches!(dirs.as_str(), "+-" | "-+-"), "{dirs}");
    // every rescale ran its adjustment through MainTransformation
    let adjusts = records(&log, "adjust");
    assert_eq!(adjusts.len() as u64, sum[0].adjustments);
    assert!(
        adjusts
            .iter()
            .all(|a| a["ok"] == true && a["policy"] == "_simoxide_hpa_app")
    );
    assert_eq!(sum[0].failed, 0);
    assert!(sum[0].replica_seconds > 0.0 && sum[0].max_replicas >= 8);
    // the response times of the run are measured
    assert!(res.measurements.to_csv().contains("Response Time"));
}

/// The same step as a load profile of the run (`simoxide_sim::LoadProfile`, no pauses): the
/// arrival rate times 10 from 300 s to 900 s. The autoscaler behaves as with the change files.
#[test]
fn a_load_profile_drives_the_autoscaler() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let dir = model_dir(0.2, &hpa_json("", "", ""), true);
    let s = setup(&dir);
    let hpa = Hpa::new(&s.cfg, &s.spd).unwrap();
    let mut cfg = sim_cfg(1, 1800);
    cfg.load_profile = Some(std::sync::Arc::new(simoxide_sim::LoadProfile::uniform(
        vec![(300.0, 10.0), (900.0, 1.0)],
    )));
    let sim = Simulation::<Fast>::create(&s.cm, cfg, Outputs::default()).unwrap();
    let mut r = SpdRun::new(sim, &s.spd).unwrap().with_hpa(&hpa).unwrap();
    let mut adj = adjuster();
    let mut log = VecLog::default();
    r.run(&mut adj, &mut log).unwrap();
    assert_eq!(r.sim.state().load_factor(0), Some(1.0));
    let sum = r.hpa_summary();
    let recs = records(&log, "hpa");
    let at = |time: f64| recs.iter().find(|r| t(r) == time).unwrap();
    assert!(int(at(315.0), "desired") > int(at(315.0), "current"));
    assert!(int(at(360.0), "desired") >= 8, "{}", at(360.0));
    let all = rescales(&log);
    assert!(
        !all.iter().any(|&(time, ..)| (450.0..900.0).contains(&time)),
        "{all:?}"
    );
    check_stabilisation(&log, 300.0);
    let first_down = all.iter().find(|&&(_, c, d)| d < c).unwrap().0;
    assert!(first_down > 1000.0, "{first_down}");
    assert!(sum[0].ready <= 3, "{sum:?}");
    assert_eq!(sum[0].failed, 0);
}

/// A ramp from 0.5 to 4.5 offered cores in 60 s steps: the replica count follows the load up
/// without a scale-down, to about the load / 0.5.
#[test]
fn a_load_ramp_scales_out_monotonically() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let dir = model_dir(0.25, &hpa_json("", "", ""), true);
    let s = setup(&dir);
    let hpa = Hpa::new(&s.cfg, &s.spd).unwrap();
    let steps: Vec<(i64, f64)> = (1..=8).map(|k| (60 * k, 0.25 * (k + 1) as f64)).collect();
    let (log, sum, _) = run_steps::<simoxide_sim::SimuLizar>(&s, &hpa, 2, 720, &steps);
    let r = rescales(&log);
    assert!(r.iter().all(|&(_, c, d)| d > c), "{r:?}");
    assert!(r.len() >= 4, "{r:?}");
    assert!((8..=10).contains(&sum[0].ready), "{sum:?}");
}

/// A start-up delay (HPA-6): pods started at a scale-out count as replicas at once (`starting`),
/// are unready (0 % on a further scale-up), and join the group, by the adjustment, one delay
/// later.
#[test]
fn starting_pods_are_unready_until_their_delay() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let dir = model_dir(0.2, &hpa_json("", "", r#""startupDelay":"40s""#), true);
    let s = setup(&dir);
    let hpa = Hpa::new(&s.cfg, &s.spd).unwrap();
    let (log, sum, _) = run_steps::<simoxide_sim::SimuLizar>(&s, &hpa, 1, 600, &[(300, 2.0)]);
    let recs = records(&log, "hpa");
    let r315 = recs.iter().find(|r| t(r) == 315.0).unwrap();
    let started = int(r315, "started");
    assert!(started > 0 && r315["goal"].is_null(), "{r315}");
    // while they start, they are replicas and unready
    for time in [330.0, 345.0] {
        let r = recs.iter().find(|r| t(r) == time).unwrap();
        assert!(int(r, "starting") > 0, "{r}");
        assert!(int(r, "unready") >= int(r, "starting"), "{r}");
        assert_eq!(int(r, "current"), int(r, "ready") + int(r, "starting"));
    }
    // ready 40 s after the decision: the adjustment runs then
    let ready: Vec<_> = records(&log, "hpa_ready")
        .into_iter()
        .filter(|r| t(r) >= 300.0)
        .collect();
    assert_eq!(t(&ready[0]), 355.0);
    assert_eq!(int(&ready[0], "ready"), started);
    let adjusts: Vec<_> = records(&log, "adjust")
        .into_iter()
        .filter(|r| t(r) >= 300.0)
        .collect();
    assert_eq!(t(&adjusts[0]), 355.0, "nothing joins before");
    // a pod ready for less than one metric window is unready in the next sync
    let r360 = recs.iter().find(|r| t(r) == 360.0).unwrap();
    assert!(int(r360, "unready") >= started, "{r360}");
    assert!(sum[0].replica_seconds > sum[0].ready_replica_seconds);
}

/// Scaling policies (HPA-5.2): at most one pod per 15 s up.
#[test]
fn behavior_policies_limit_the_rate() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let spec =
        r#","behavior":{"scaleUp":{"policies":[{"type":"Pods","value":1,"periodSeconds":15}]}}"#;
    let dir = model_dir(0.2, &hpa_json("", spec, ""), true);
    let s = setup(&dir);
    let hpa = Hpa::new(&s.cfg, &s.spd).unwrap();
    let (log, _, _) = run_steps::<simoxide_sim::SimuLizar>(&s, &hpa, 1, 600, &[(300, 2.0)]);
    let ups: Vec<_> = rescales(&log).into_iter().filter(|r| r.2 > r.1).collect();
    assert!(ups.len() >= 5, "{ups:?}");
    assert!(ups.iter().all(|&(_, c, d)| d == c + 1), "{ups:?}");
    assert!(
        records(&log, "hpa")
            .iter()
            .any(|r| r["reason"] == "ScaleUpLimit"),
        "the policy limited a sync"
    );
}

/// The Slingshot flavour (fast mode): the same controller on Slingshot's engine, deterministic.
#[test]
fn the_slingshot_flavour_scales_too() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let dir = model_dir(0.2, &hpa_json("", "", r#""startupDelay":"20s""#), false);
    let s = setup_flavour(&dir, FlavourConfig::slingshot(SsQuirks::NONE));
    let hpa = Hpa::new(&s.cfg, &s.spd).unwrap();
    let run = || run_steps::<Slingshot>(&s, &hpa, 3, 1500, &[(300, 2.0), (700, 0.2)]);
    let (log, sum, _) = run();
    let (again, ..) = run();
    assert_eq!(log.to_jsonl(), again.to_jsonl());
    assert!(sum[0].max_replicas >= 8, "{sum:?}");
    assert!(sum[0].ready <= 3, "{sum:?}");
    check_stabilisation(&log, 300.0);
    assert_eq!(sum[0].failed, 0);
}

/// Two decision logs are equal (else the first different line).
fn same_log(got: &VecLog, want: &VecLog, what: &str) {
    let (g, w) = (got.to_jsonl(), want.to_jsonl());
    for (i, (a, b)) in g.lines().zip(w.lines()).enumerate() {
        assert!(a == b, "{what}: line {}:\n got  {a}\n want {b}", i + 1);
    }
    assert_eq!(g.lines().count(), w.lines().count(), "{what}");
}

/// The log without the adjustments' model summaries.
fn strip(log: &VecLog) -> VecLog {
    use simoxide_spd::log::Record;
    VecLog {
        records: log
            .records
            .iter()
            .map(|r| match r {
                Record::Adjust { .. } => {
                    let mut r = r.clone();
                    if let Record::Adjust { before, after, .. } = &mut r {
                        *before = None;
                        *after = None;
                    }
                    r
                }
                _ => r.clone(),
            })
            .collect(),
        all: log.all,
    }
}

/// Forks and snapshots (HPA-7): a fork and a restored snapshot continue like the uninterrupted
/// run (decision log, HPA state, measurements), also while pods start and right after rescales;
/// a restored run writes the bytes it was read from.
#[test]
fn forks_and_snapshots_continue_identically() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let dir = model_dir(0.2, &hpa_json("", "", r#""startupDelay":"25s""#), true);
    let s = setup(&dir);
    let hpa = Hpa::new(&s.cfg, &s.spd).unwrap();
    let steps = [(300, 2.0), (700, 0.2)];
    let (wlog, wsum, wres) = run_steps::<simoxide_sim::SimuLizar>(&s, &hpa, 5, 1300, &steps);
    let snap = |r: &mut SpdRun<'_, '_, Fast>, adj: &QvtoAdjuster| {
        let mut snap = r.snapshot_value(Encoding::Postcard).unwrap();
        if adj.run.is_started() {
            snap.sections
                .push(adj.run.snapshot_section(Encoding::Postcard).unwrap());
        }
        let mut bytes = Vec::new();
        snap.write(&mut bytes).unwrap();
        bytes
    };
    let sim = Simulation::<Fast>::create(&s.cm, sim_cfg(5, 1300), Outputs::default()).unwrap();
    let mut r = SpdRun::new(sim, &s.spd).unwrap().with_hpa(&hpa).unwrap();
    let mut adj = adjuster();
    let mut log = VecLog::default();
    let mut next_step = 0;
    for (k, pause) in [100, 300, 316, 330, 341, 700, 701, 1010, 1205]
        .into_iter()
        .enumerate()
    {
        r.run_until(&mut adj, &mut log, Some(pause * S)).unwrap();
        if next_step < steps.len() && steps[next_step].0 == pause {
            set_rate(&mut r, steps[next_step].1);
            next_step += 1;
        }
        // a fork continues like the run
        if k % 3 == 1 {
            let mut f = r.fork(Outputs::default()).unwrap();
            let mut fadj = adj.fork_spd(true);
            let mut flog = VecLog {
                records: log.records.clone(),
                all: false,
            };
            for &(t, rate) in &steps[next_step..] {
                f.run_until(&mut fadj, &mut flog, Some(t * S)).unwrap();
                set_rate(&mut f, rate);
            }
            f.run(&mut fadj, &mut flog).unwrap();
            // (a fork's adjustment summaries name the elements of the model it was forked on
            // as inputs: compared without them)
            same_log(&strip(&flog), &strip(&wlog), &format!("fork at {pause}"));
            assert_eq!(f.hpa_summary(), wsum, "fork at {pause}");
        }
        // a snapshot restores and writes the same bytes
        let bytes = snap(&mut r, &adj);
        drop(r);
        let read = Snapshot::read(&bytes[..]).unwrap();
        assert!(SpdRun::<Fast>::has_hpa(&read));
        assert!(
            SpdRun::<Fast>::restore_snapshot(&read, &s.cm, &s.spd, Outputs::default()).is_err(),
            "without the configuration, the restore refuses"
        );
        r = SpdRun::restore_snapshot_hpa(&read, &s.cm, &s.spd, Some(&hpa), Outputs::default())
            .unwrap();
        adj = adjuster();
        if read.section(simoxide_reconf::QVTO).is_some() {
            adj.run.restore_section(&r.sim, &read).unwrap();
        }
        assert!(snap(&mut r, &adj) == bytes, "pause {pause}");
    }
    r.run(&mut adj, &mut log).unwrap();
    assert_eq!(r.hpa_summary(), wsum);
    let (res, _, _) = r.finish().unwrap();
    same_log(&log, &wlog, "snapshots");
    assert_eq!(res.measurements.to_csv(), wres.measurements.to_csv());
    assert_eq!(res.events, wres.events);
}

/// Bounded growth (HPA-8): over 200 load oscillations (scale-down window 0, about 1 000
/// adjustments) the controller's state stays bounded (recommendations, scale events, starting
/// pods, readings); the model grows by the same amount per pod created (its tombstone: about
/// 22 graph objects with the cloned monitors, one resource, about 8 KB of snapshot), and a
/// steady load adds nothing.
/// `maxCreatedPods` stops a run that creates too many.
#[test]
fn growth_is_bounded_per_created_pod() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let spec = r#","behavior":{"scaleDown":{"stabilizationWindowSeconds":0}}"#;
    let dir = model_dir(0.2, &hpa_json("", spec, r#""startupDelay":"5s""#), true);
    let s = setup(&dir);
    let hpa = Hpa::new(&s.cfg, &s.spd).unwrap();
    // (measurements not stored: their rows grow with the run, not with the HPA)
    let mut cfg = sim_cfg(1, 1_000_000);
    cfg.store_measurements = false;
    let sim = Simulation::<Fast>::create(&s.cm, cfg, Outputs::default()).unwrap();
    let mut r = SpdRun::new(sim, &s.spd).unwrap().with_hpa(&hpa).unwrap();
    let mut adj = adjuster();
    let mut log = NoLog;
    let mut tnow = 0;
    // (created pods, graph objects, compiled resources, snapshot bytes)
    let mut samples = Vec::new();
    for cycle in 0..200 {
        for rate in [2.0, 0.1] {
            tnow += 60;
            r.run_until(&mut adj, &mut log, Some(tnow * S)).unwrap();
            set_rate(&mut r, rate);
        }
        let st = &r.hpa_state().autoscalers[0];
        assert!(st.memory.recommendations.len() <= 2, "{st:?}");
        assert!(st.memory.up_events.len() <= 3 && st.memory.down_events.len() <= 3);
        assert!(st.replicas.pending.len() <= 10 && st.pods.len() <= 10);
        assert!(st.readings.len() <= 10);
        if cycle % 50 == 49 {
            let created = r.hpa_summary()[0].created;
            let mut bytes = Vec::new();
            r.snapshot_value(Encoding::Postcard)
                .unwrap()
                .write(&mut bytes)
                .unwrap();
            let cm = r.sim.compiled();
            samples.push((
                created as f64,
                cm.model.graph.object_count() as f64,
                cm.resources.len() as f64,
                bytes.len() as f64,
            ));
        }
    }
    assert!(samples[3].0 > 500.0, "{samples:?}");
    // linear in the created pods: the same increment per pod in every segment
    type Sample = (f64, f64, f64, f64);
    let per = |k: usize, f: fn(&Sample) -> f64| {
        (f(&samples[k + 1]) - f(&samples[k])) / (samples[k + 1].0 - samples[k].0)
    };
    let (objects, bytes) = (per(0, |s| s.1), per(0, |s| s.3));
    for k in 1..samples.len() - 1 {
        assert!(
            (per(k, |s| s.1) - objects).abs() < 0.5,
            "graph objects per pod: {samples:?}"
        );
        assert!(
            (per(k, |s| s.2) - 1.0).abs() < 0.05,
            "resources per pod: {samples:?}"
        );
        assert!(
            (per(k, |s| s.3) / bytes - 1.0).abs() < 0.15,
            "snapshot bytes per pod: {samples:?}"
        );
    }
    // the container, its resource, allocation, assembly, connectors, load-balancer branch and
    // cloned monitors
    assert!(objects < 30.0 && bytes < 12_000.0, "{objects} {bytes}");
    // thousands of syncs without a rescale (min = max) add nothing
    r.hpa_set("app", "maxReplicas", 2.0).unwrap();
    r.hpa_set("app", "minReplicas", 2.0).unwrap();
    tnow += 60;
    r.run_until(&mut adj, &mut log, Some(tnow * S)).unwrap();
    let objects = r.sim.compiled().model.graph.object_count();
    let created = r.hpa_summary()[0].created;
    let mut before = Vec::new();
    r.snapshot_value(Encoding::Postcard)
        .unwrap()
        .write(&mut before)
        .unwrap();
    tnow += 30_000;
    r.run_until(&mut adj, &mut log, Some(tnow * S)).unwrap();
    let mut after = Vec::new();
    r.snapshot_value(Encoding::Postcard)
        .unwrap()
        .write(&mut after)
        .unwrap();
    assert_eq!(created, r.hpa_summary()[0].created, "no rescale");
    assert!(r.hpa_summary()[0].syncs > 2_000);
    assert_eq!(objects, r.sim.compiled().model.graph.object_count());
    assert!(
        after.len() <= before.len() + 64,
        "{} -> {}",
        before.len(),
        after.len()
    );

    // the guard
    let dir = model_dir(0.2, &hpa_json(r#","maxCreatedPods":20"#, spec, ""), true);
    let s = setup(&dir);
    let hpa = Hpa::new(&s.cfg, &s.spd).unwrap();
    let sim = Simulation::<Fast>::create(&s.cm, sim_cfg(1, 100_000), Outputs::default()).unwrap();
    let mut r = SpdRun::new(sim, &s.spd).unwrap().with_hpa(&hpa).unwrap();
    let mut adj = adjuster();
    let mut tnow = 0;
    let err = loop {
        tnow += 60;
        match r.run_until(&mut adj, &mut log, Some(tnow * S)) {
            Ok(_) => set_rate(&mut r, if tnow % 120 == 60 { 2.0 } else { 0.1 }),
            Err(e) => break e,
        }
        assert!(tnow < 20_000, "the guard did not stop the run");
    };
    assert!(err.message.contains("maxCreatedPods"), "{}", err.message);
}

/// The configuration is checked against the SPD.
#[test]
fn configuration_errors() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let dir = model_dir(0.2, &hpa_json("", "", ""), false);
    let mut model = RunSpec::default().load_model(&dir).unwrap();
    let bad = HpaConfig::from_json(
        &hpa_json("", "", "").replace(r#""name":"app"}"#, r#""name":"nope"}"#),
    )
    .unwrap();
    let e = hpa::attach(&mut model, &bad).unwrap_err();
    assert!(e.contains("no target group named \"nope\""), "{e}");
    // Hpa::new needs the attached policy
    let cfg = HpaConfig::load_dir(&dir).unwrap().unwrap();
    let spd = spd_of(&model, Quirks::fixed()).unwrap().unwrap();
    assert!(Hpa::new(&cfg, &spd).unwrap_err().contains("hpa::attach"));
    // attaching twice adds the policy once
    hpa::attach(&mut model, &cfg).unwrap();
    hpa::attach(&mut model, &cfg).unwrap();
    let spd = spd_of(&model, Quirks::fixed()).unwrap().unwrap();
    let n = spd
        .model()
        .policies
        .iter()
        .filter(|p| &*p.id == "_simoxide_hpa_app")
        .count();
    assert_eq!(n, 1);
    assert!(spd.contexts().is_empty(), "the HPA's policy is inactive");
}

/// Parameters change at a pause (`SpdRun::hpa_set`, as `simoxide resume --hpa-set` and change
/// files' `hpa` lines): a lower maximum scales in at the next sync.
#[test]
fn parameters_change_at_a_pause() {
    if !simoxide_thirdparty::skip_unless_all_available() {
        return;
    }
    let dir = model_dir(2.0, &hpa_json("", "", ""), true);
    let s = setup(&dir);
    let hpa = Hpa::new(&s.cfg, &s.spd).unwrap();
    let sim = Simulation::<Fast>::create(&s.cm, sim_cfg(1, 400), Outputs::default()).unwrap();
    let mut r = SpdRun::new(sim, &s.spd).unwrap().with_hpa(&hpa).unwrap();
    let mut adj = adjuster();
    let mut log = VecLog::default();
    r.run_until(&mut adj, &mut log, Some(200 * S)).unwrap();
    assert!(r.hpa_summary()[0].ready >= 7, "{:?}", r.hpa_summary());
    r.hpa_set("app", "maxReplicas", 4.0).unwrap();
    assert!(r.hpa_set("app", "maxReplicas", 0.0).is_err());
    assert!(r.hpa_set("app", "bogus", 1.0).is_err());
    assert!(r.hpa_set("other", "maxReplicas", 3.0).is_err());
    r.run(&mut adj, &mut log).unwrap();
    let recs = records(&log, "hpa");
    let r210 = recs.iter().find(|r| t(r) == 210.0).unwrap();
    assert_eq!(int(r210, "desired"), 4, "{r210}");
    assert_eq!(r210["reason"], "AboveMax");
    assert_eq!(r.hpa_summary()[0].ready, 4);
}

// ---------------------------------------------------------------------------------------------
// cost per scaling step on a model of MiSArch's size (a quick sanity measurement, ignored by
// default: `cargo test --release -p simoxide-reconf --test hpa cost -- --ignored --nocapture`)

/// A generated model of `k` services, each the V4 pattern (a 1-core application server behind
/// its own load balancer, an open workload of `Exp(rate)`, 0.5 s mean demand), each an elastic
/// infrastructure with an HPA (CPU 50 %, 1..8 replicas). `k = 40`: about 2 300 objects.
fn services_model(k: usize, rate: f64) -> PathBuf {
    services_model_with(k, rate, false)
}

/// [`services_model`]; with `backend`, every service calls a `Backend` component (its own
/// container, not scaled) after its work, so requests in progress on a pod make calls.
fn services_model_with(k: usize, rate: f64, backend: bool) -> PathBuf {
    const CPU: &str = "pathmap://PCM_MODELS/Palladio.resourcetype#_oro4gG3fEdy4YaaT-RYrLQ";
    const FCFS: &str = "pathmap://PCM_MODELS/Palladio.resourcetype#FCFS";
    const LAN: &str = "pathmap://PCM_MODELS/Palladio.resourcetype#_o3sScH2AEdyH8uerKnHYug";
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("hpa-services-{}-{n}", std::process::id()))
        .join("services");
    std::fs::create_dir_all(&out).unwrap();
    let xmi = r#"xmi:version="2.0" xmlns:xmi="http://www.omg.org/XMI" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance""#;
    let (mut repo, mut sys, mut env, mut alloc, mut usage, mut spd, mut sem, mut hpa) = (
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        String::new(),
        Vec::new(),
    );
    for i in 0..k {
        let s = format!("_s{i}");
        let (last, after, backend_call, backend_role) = if backend {
            (
                format!("{s}_bcall"),
                format!("{s}_bcall"),
                format!(
                    r#"<steps_Behaviour xsi:type="seff:ExternalCallAction" id="{s}_bcall" entityName="callBackend{i}" predecessor_AbstractAction="{s}_ia" successor_AbstractAction="{s}_stop" calledService_ExternalService="_backend_sig" role_ExternalService="{s}_breq"/>
      "#
                ),
                format!(
                    r#"<requiredRoles_InterfaceRequiringEntity xsi:type="repository:OperationRequiredRole" id="{s}_breq" entityName="Service{i}.Backend" requiredInterface__OperationRequiredRole="_backend_if"/>
    "#
                ),
            )
        } else {
            (
                format!("{s}_ia"),
                format!("{s}_stop"),
                String::new(),
                String::new(),
            )
        };
        repo += &format!(
            r#"  <components__Repository xsi:type="repository:BasicComponent" id="{s}_comp" entityName="Service{i}">
    <providedRoles_InterfaceProvidingEntity xsi:type="repository:OperationProvidedRole" id="{s}_prov" entityName="Service{i}.Provided" providedInterface__OperationProvidedRole="{s}_if"/>
    {backend_role}
    <serviceEffectSpecifications__BasicComponent xsi:type="seff:ResourceDemandingSEFF" id="{s}_seff" describedService__SEFF="{s}_sig">
      <steps_Behaviour xsi:type="seff:StartAction" id="{s}_start" successor_AbstractAction="{s}_ia"/>
      <steps_Behaviour xsi:type="seff:StopAction" id="{s}_stop" predecessor_AbstractAction="{last}"/>
      {backend_call}<steps_Behaviour xsi:type="seff:InternalAction" id="{s}_ia" entityName="work{i}" predecessor_AbstractAction="{s}_start" successor_AbstractAction="{after}">
        <resourceDemand_Action>
          <specification_ParametericResourceDemand specification="Exp(2.0)"/>
          <requiredResource_ParametricResourceDemand href="{CPU}"/>
        </resourceDemand_Action>
      </steps_Behaviour>
    </serviceEffectSpecifications__BasicComponent>
  </components__Repository>
  <components__Repository xsi:type="repository:BasicComponent" id="{s}_lb" entityName="LoadBalancer{i}">
    <providedRoles_InterfaceProvidingEntity xsi:type="repository:OperationProvidedRole" id="{s}_lbprov" entityName="LoadBalancer{i}.Provided" providedInterface__OperationProvidedRole="{s}_if"/>
    <requiredRoles_InterfaceRequiringEntity xsi:type="repository:OperationRequiredRole" id="{s}_lbreq" entityName="LoadBalancer{i}.Required" requiredInterface__OperationRequiredRole="{s}_if"/>
    <serviceEffectSpecifications__BasicComponent xsi:type="seff:ResourceDemandingSEFF" id="{s}_lbseff" describedService__SEFF="{s}_sig">
      <steps_Behaviour xsi:type="seff:StartAction" id="{s}_lbstart" successor_AbstractAction="{s}_br"/>
      <steps_Behaviour xsi:type="seff:StopAction" id="{s}_lbstop" predecessor_AbstractAction="{s}_br"/>
      <steps_Behaviour xsi:type="seff:BranchAction" id="{s}_br" entityName="Branch{i}" predecessor_AbstractAction="{s}_lbstart" successor_AbstractAction="{s}_lbstop">
        <branches_Branch xsi:type="seff:ProbabilisticBranchTransition" id="{s}_bt" entityName="Transition{i}" branchProbability="1.0">
          <branchBehaviour_BranchTransition id="{s}_bb">
            <steps_Behaviour xsi:type="seff:StartAction" id="{s}_bstart" successor_AbstractAction="{s}_call"/>
            <steps_Behaviour xsi:type="seff:StopAction" id="{s}_bstop" predecessor_AbstractAction="{s}_call"/>
            <steps_Behaviour xsi:type="seff:ExternalCallAction" id="{s}_call" entityName="Call{i}" predecessor_AbstractAction="{s}_bstart" successor_AbstractAction="{s}_bstop" calledService_ExternalService="{s}_sig" role_ExternalService="{s}_lbreq"/>
          </branchBehaviour_BranchTransition>
        </branches_Branch>
      </steps_Behaviour>
    </serviceEffectSpecifications__BasicComponent>
  </components__Repository>
  <interfaces__Repository xsi:type="repository:OperationInterface" id="{s}_if" entityName="Interface{i}">
    <signatures__OperationInterface id="{s}_sig" entityName="op{i}"/>
  </interfaces__Repository>
"#
        );
        sys += &format!(
            r#"  <assemblyContexts__ComposedStructure id="{s}_ac" entityName="Assembly_Service{i}">
    <encapsulatedComponent__AssemblyContext xsi:type="repository:BasicComponent" href="default.repository#{s}_comp"/>
  </assemblyContexts__ComposedStructure>
  <assemblyContexts__ComposedStructure id="{s}_lbac" entityName="Assembly_LoadBalancer{i}">
    <encapsulatedComponent__AssemblyContext xsi:type="repository:BasicComponent" href="default.repository#{s}_lb"/>
  </assemblyContexts__ComposedStructure>
  <connectors__ComposedStructure xsi:type="composition:ProvidedDelegationConnector" id="{s}_pdc" entityName="Delegation{i}" outerProvidedRole_ProvidedDelegationConnector="{s}_sysrole" assemblyContext_ProvidedDelegationConnector="{s}_lbac">
    <innerProvidedRole_ProvidedDelegationConnector href="default.repository#{s}_lbprov"/>
  </connectors__ComposedStructure>
  <connectors__ComposedStructure xsi:type="composition:AssemblyConnector" id="{s}_conn" entityName="Connector{i}" requiringAssemblyContext_AssemblyConnector="{s}_lbac" providingAssemblyContext_AssemblyConnector="{s}_ac">
    <providedRole_AssemblyConnector href="default.repository#{s}_prov"/>
    <requiredRole_AssemblyConnector href="default.repository#{s}_lbreq"/>
  </connectors__ComposedStructure>
  <providedRoles_InterfaceProvidingEntity xsi:type="repository:OperationProvidedRole" id="{s}_sysrole" entityName="Provided{i}">
    <providedInterface__OperationProvidedRole href="default.repository#{s}_if"/>
  </providedRoles_InterfaceProvidingEntity>
"#
        );
        if backend {
            sys += &format!(
                r#"  <connectors__ComposedStructure xsi:type="composition:AssemblyConnector" id="{s}_bconn" entityName="ToBackend{i}" requiringAssemblyContext_AssemblyConnector="{s}_ac" providingAssemblyContext_AssemblyConnector="_backend_ac">
    <providedRole_AssemblyConnector href="default.repository#_backend_prov"/>
    <requiredRole_AssemblyConnector href="default.repository#{s}_breq"/>
  </connectors__ComposedStructure>
"#
            );
        }
        let link_backend = if backend { " _backend_rc" } else { "" };
        env += &format!(
            r#"  <linkingResources__ResourceEnvironment id="{s}_link" entityName="Link{i}" connectedResourceContainers_LinkingResource="{s}_rc {s}_lbrc{link_backend}">
    <communicationLinkResourceSpecifications_LinkingResource id="{s}_linkspec">
      <communicationLinkResourceType_CommunicationLinkResourceSpecification href="{LAN}"/>
      <latency_CommunicationLinkResourceSpecification specification="0.0"/>
      <throughput_CommunicationLinkResourceSpecification specification="10000"/>
    </communicationLinkResourceSpecifications_LinkingResource>
  </linkingResources__ResourceEnvironment>
  <resourceContainer_ResourceEnvironment id="{s}_rc" entityName="Server{i}">
    <activeResourceSpecifications_ResourceContainer id="{s}_cpu">
      <schedulingPolicy href="{FCFS}"/>
      <activeResourceType_ActiveResourceSpecification href="{CPU}"/>
      <processingRate_ProcessingResourceSpecification specification="1.0"/>
    </activeResourceSpecifications_ResourceContainer>
  </resourceContainer_ResourceEnvironment>
  <resourceContainer_ResourceEnvironment id="{s}_lbrc" entityName="Balancer{i}">
    <activeResourceSpecifications_ResourceContainer id="{s}_lbcpu">
      <schedulingPolicy href="{FCFS}"/>
      <activeResourceType_ActiveResourceSpecification href="{CPU}"/>
      <processingRate_ProcessingResourceSpecification specification="100000"/>
    </activeResourceSpecifications_ResourceContainer>
  </resourceContainer_ResourceEnvironment>
"#
        );
        alloc += &format!(
            r#"  <allocationContexts_Allocation id="{s}_alloc" entityName="Allocation_Service{i}">
    <resourceContainer_AllocationContext href="My.resourceenvironment#{s}_rc"/>
    <assemblyContext_AllocationContext href="default.system#{s}_ac"/>
  </allocationContexts_Allocation>
  <allocationContexts_Allocation id="{s}_lballoc" entityName="Allocation_LoadBalancer{i}">
    <resourceContainer_AllocationContext href="My.resourceenvironment#{s}_lbrc"/>
    <assemblyContext_AllocationContext href="default.system#{s}_lbac"/>
  </allocationContexts_Allocation>
"#
        );
        usage += &format!(
            r#"  <usageScenario_UsageModel id="{s}_us" entityName="svc{i}">
    <scenarioBehaviour_UsageScenario id="{s}_usb" entityName="behaviour{i}">
      <actions_ScenarioBehaviour xsi:type="usagemodel:Start" id="{s}_ustart" successor="{s}_elsc"/>
      <actions_ScenarioBehaviour xsi:type="usagemodel:Stop" id="{s}_ustop" predecessor="{s}_elsc"/>
      <actions_ScenarioBehaviour xsi:type="usagemodel:EntryLevelSystemCall" id="{s}_elsc" successor="{s}_ustop" predecessor="{s}_ustart">
        <providedRole_EntryLevelSystemCall href="default.system#{s}_sysrole"/>
        <operationSignature__EntryLevelSystemCall href="default.repository#{s}_sig"/>
      </actions_ScenarioBehaviour>
    </scenarioBehaviour_UsageScenario>
    <workload_UsageScenario xsi:type="usagemodel:OpenWorkload">
      <interArrivalTime_OpenWorkload specification="Exp({rate})"/>
    </workload_UsageScenario>
  </usageScenario_UsageModel>
"#
        );
        spd += &format!(
            r#"  <targetGroups xsi:type="targets:ElasticInfrastructure" id="{s}_tg" entityName="svc{i}">
    <unit href="My.resourceenvironment#{s}_rc"/>
  </targetGroups>
"#
        );
        sem += &format!(
            r#"  <targetCfgs xsi:type="semanticspd:ElasticInfrastructureCfg">
    <unit href="My.resourceenvironment#{s}_rc"/>
    <elements href="My.resourceenvironment#{s}_rc"/>
    <linkingResource href="My.resourceenvironment#{s}_link"/>
    <resourceEnvironment href="My.resourceenvironment#/0"/>
  </targetCfgs>
  <targetCfgs xsi:type="semanticspd:ServiceGroupCfg">
    <unit href="default.system#{s}_ac"/>
    <elements href="default.system#{s}_ac"/>
    <loadBalancingAssembly href="default.system#{s}_lbac"/>
  </targetCfgs>
"#
        );
        hpa.push(format!(
            r#"{{"metadata":{{"name":"svc{i}"}},"spec":{{"scaleTargetRef":{{"name":"svc{i}"}},"minReplicas":1,"maxReplicas":8,
  "metrics":[{{"type":"Resource","resource":{{"name":"cpu","target":{{"type":"Utilization","averageUtilization":50}}}}}}]}}}}"#
        ));
    }
    if backend {
        repo += &format!(
            r#"  <components__Repository xsi:type="repository:BasicComponent" id="_backend_comp" entityName="Backend">
    <providedRoles_InterfaceProvidingEntity xsi:type="repository:OperationProvidedRole" id="_backend_prov" entityName="Backend.Provided" providedInterface__OperationProvidedRole="_backend_if"/>
    <serviceEffectSpecifications__BasicComponent xsi:type="seff:ResourceDemandingSEFF" id="_backend_seff" describedService__SEFF="_backend_sig">
      <steps_Behaviour xsi:type="seff:StartAction" id="_backend_start" successor_AbstractAction="_backend_ia"/>
      <steps_Behaviour xsi:type="seff:StopAction" id="_backend_stop" predecessor_AbstractAction="_backend_ia"/>
      <steps_Behaviour xsi:type="seff:InternalAction" id="_backend_ia" entityName="backendWork" predecessor_AbstractAction="_backend_start" successor_AbstractAction="_backend_stop">
        <resourceDemand_Action>
          <specification_ParametericResourceDemand specification="Exp(20.0)"/>
          <requiredResource_ParametricResourceDemand href="{CPU}"/>
        </resourceDemand_Action>
      </steps_Behaviour>
    </serviceEffectSpecifications__BasicComponent>
  </components__Repository>
  <interfaces__Repository xsi:type="repository:OperationInterface" id="_backend_if" entityName="BackendInterface">
    <signatures__OperationInterface id="_backend_sig" entityName="backend"/>
  </interfaces__Repository>
"#
        );
        sys += r#"  <assemblyContexts__ComposedStructure id="_backend_ac" entityName="Assembly_Backend">
    <encapsulatedComponent__AssemblyContext xsi:type="repository:BasicComponent" href="default.repository#_backend_comp"/>
  </assemblyContexts__ComposedStructure>
"#;
        env += &format!(
            r#"  <resourceContainer_ResourceEnvironment id="_backend_rc" entityName="BackendServer">
    <activeResourceSpecifications_ResourceContainer id="_backend_cpu" numberOfReplicas="8">
      <schedulingPolicy href="{FCFS}"/>
      <activeResourceType_ActiveResourceSpecification href="{CPU}"/>
      <processingRate_ProcessingResourceSpecification specification="100.0"/>
    </activeResourceSpecifications_ResourceContainer>
  </resourceContainer_ResourceEnvironment>
"#
        );
        alloc += r#"  <allocationContexts_Allocation id="_backend_alloc" entityName="Allocation_Backend">
    <resourceContainer_AllocationContext href="My.resourceenvironment#_backend_rc"/>
    <assemblyContext_AllocationContext href="default.system#_backend_ac"/>
  </allocationContexts_Allocation>
"#;
    }
    let files = [
        (
            "default.repository",
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<repository:Repository {xmi} xmlns:repository=\"http://palladiosimulator.org/PalladioComponentModel/Repository/5.2\" xmlns:seff=\"http://palladiosimulator.org/PalladioComponentModel/SEFF/5.2\" id=\"_repo\" entityName=\"repository\">\n{repo}</repository:Repository>\n"
            ),
        ),
        (
            "default.system",
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<system:System {xmi} xmlns:composition=\"http://palladiosimulator.org/PalladioComponentModel/Core/Composition/5.2\" xmlns:repository=\"http://palladiosimulator.org/PalladioComponentModel/Repository/5.2\" xmlns:system=\"http://palladiosimulator.org/PalladioComponentModel/System/5.2\" id=\"_sys\" entityName=\"services\">\n{sys}</system:System>\n"
            ),
        ),
        (
            "My.resourceenvironment",
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<resourceenvironment:ResourceEnvironment {xmi} xmlns:resourceenvironment=\"http://palladiosimulator.org/PalladioComponentModel/ResourceEnvironment/5.2\">\n{env}</resourceenvironment:ResourceEnvironment>\n"
            ),
        ),
        (
            "default.allocation",
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<allocation:Allocation {xmi} xmlns:allocation=\"http://palladiosimulator.org/PalladioComponentModel/Allocation/5.2\" id=\"_alloc\" entityName=\"allocation\">\n  <targetResourceEnvironment_Allocation href=\"My.resourceenvironment#/\"/>\n  <system_Allocation href=\"default.system#_sys\"/>\n{alloc}</allocation:Allocation>\n"
            ),
        ),
        (
            "default.usagemodel",
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<usagemodel:UsageModel {xmi} xmlns:usagemodel=\"http://palladiosimulator.org/PalladioComponentModel/UsageModel/5.2\">\n{usage}</usagemodel:UsageModel>\n"
            ),
        ),
        (
            "default.spd",
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<spd:SPD {xmi} xmlns:spd=\"http://palladiosimulator.org/ScalingPolicyDefinition/1.0\" xmlns:targets=\"http://palladiosimulator.org/ScalingPolicyDefinition/Targets/1.0\" id=\"_spd\">\n{spd}</spd:SPD>\n"
            ),
        ),
        (
            "default.semanticspd",
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<semanticspd:Configuration {xmi} xmlns:semanticspd=\"http://palladiosimulator.org/ScalingPolicyDefinitionSemantic/1.0\">\n  <spd href=\"default.spd#_spd\"/>\n{sem}  <allocation href=\"default.allocation#_alloc\"/>\n  <system href=\"default.system#_sys\"/>\n  <resourceEnvironment href=\"My.resourceenvironment#/0\"/>\n  <repository href=\"default.repository#_repo\"/>\n</semanticspd:Configuration>\n"
            ),
        ),
        (
            "services.hpa.json",
            format!("{{\"items\":[{}]}}", hpa.join(",\n")),
        ),
    ];
    for (name, text) in files {
        std::fs::write(out.join(name), text).unwrap();
    }
    out
}

/// Scale-in while requests are in progress on the removed pods, whose service then calls a
/// backend (MC-8.2: a removed assembly context's calls resolve over the connectors it had).
/// Each pod's work is a 0.5 s demand, then a call; a step from 2 to 0.2 offered cores leaves
/// requests on the pods the scale-in removes. The run completes, the removed pods' requests
/// finish (no request is lost: every arrival before the end completes or is still running),
/// and the scale-in happened.
#[test]
fn a_scale_in_drains_pods_whose_requests_make_calls() {
    let dir = services_model_with(1, 4.0, true);
    let hpa_cfg = r#"{"items":[{"metadata":{"name":"svc0"},"spec":{"scaleTargetRef":{"name":"svc0"},"minReplicas":1,"maxReplicas":8,
  "metrics":[{"type":"Resource","resource":{"name":"cpu","target":{"type":"Utilization","averageUtilization":50}}}],
  "behavior":{"scaleDown":{"stabilizationWindowSeconds":0}}}}]}"#;
    std::fs::write(dir.join("services.hpa.json"), hpa_cfg).unwrap();
    let s = setup(&dir);
    let hpa = Hpa::new(&s.cfg, &s.spd).unwrap();
    let mut cfg = sim_cfg(3, 900);
    cfg.load_profile = Some(std::sync::Arc::new(simoxide_sim::LoadProfile::uniform(
        vec![(300.0, 0.1), (600.0, 1.0)],
    )));
    let sim = Simulation::<Fast>::create(&s.cm, cfg, Outputs::default()).unwrap();
    let mut r = SpdRun::new(sim, &s.spd).unwrap().with_hpa(&hpa).unwrap();
    let mut adj = adjuster();
    let mut log = VecLog::default();
    r.run_until(&mut adj, &mut log, Some(650 * S)).unwrap();
    let before = r.sim.state().main_count();
    r.run(&mut adj, &mut log).unwrap();
    let downs: Vec<_> = rescales(&log)
        .into_iter()
        .filter(|&(_, c, d)| d < c)
        .collect();
    assert!(!downs.is_empty(), "no scale-in: {:?}", rescales(&log));
    assert!(
        downs.iter().any(|&(t, ..)| (300.0..600.0).contains(&t)),
        "{downs:?}"
    );
    let sum = r.hpa_summary();
    assert_eq!(sum[0].failed, 0);
    let (res, _, _) = r.finish().unwrap();
    // requests keep completing after the scale-ins (about 4 per second at the end)
    assert!(
        res.main_count > before + 500,
        "{} after {before}",
        res.main_count
    );
}

/// `SpdRun::container_units` maps every pod an elastic infrastructure had (also those removed
/// again) to its unit, so that what the run recorded per container (here the synchronous paths
/// of `SimConfig::sync_paths`, MEAS-13) can be read per scaled container.
#[test]
fn scaled_out_pods_map_to_their_unit() {
    let dir = services_model_with(1, 4.0, true);
    let hpa_cfg = r#"{"items":[{"metadata":{"name":"svc0"},"spec":{"scaleTargetRef":{"name":"svc0"},"minReplicas":1,"maxReplicas":8,
  "metrics":[{"type":"Resource","resource":{"name":"cpu","target":{"type":"Utilization","averageUtilization":50}}}],
  "behavior":{"scaleDown":{"stabilizationWindowSeconds":0}}}}]}"#;
    std::fs::write(dir.join("services.hpa.json"), hpa_cfg).unwrap();
    let s = setup(&dir);
    let hpa = Hpa::new(&s.cfg, &s.spd).unwrap();
    let mut cfg = sim_cfg(3, 900);
    cfg.sync_paths = true;
    cfg.load_profile = Some(std::sync::Arc::new(simoxide_sim::LoadProfile::uniform(
        vec![(300.0, 0.1), (600.0, 1.0)],
    )));
    let base: Vec<String> =
        s.cm.model
            .containers
            .iter()
            .map(|c| c.id.to_string())
            .collect();
    let sim = Simulation::<Fast>::create(&s.cm, cfg, Outputs::default()).unwrap();
    let mut r = SpdRun::new(sim, &s.spd).unwrap().with_hpa(&hpa).unwrap();
    let mut adj = adjuster();
    let mut log = VecLog::default();
    r.run(&mut adj, &mut log).unwrap();
    let cm = r.sim.compiled();
    let id = |c: simoxide_model::ContainerId| cm.model.containers[c].id.to_string();
    let units: Vec<(String, String)> = r
        .container_units()
        .iter()
        .map(|&(e, u)| (id(e), id(u)))
        .collect();
    let unit = &units[0].1;
    assert!(base.contains(unit), "{unit}");
    assert!(units.iter().all(|(_, u)| u == unit), "{units:?}");
    assert!(units.len() >= 3, "the group scaled out: {units:?}");
    assert!(
        units.len() > r.hpa_summary()[0].ready as usize,
        "pods removed again stay listed"
    );
    let paths = r.sim.state().sync_paths(cm).unwrap();
    let mut on_pods = 0;
    for p in &paths {
        for &c in &p.containers {
            let c = id(c);
            if !base.contains(&c) {
                assert!(units.iter().any(|(e, _)| *e == c), "{c} is a pod");
                on_pods += p.sessions;
            }
        }
    }
    assert!(on_pods > 100, "sessions served by created pods: {on_pods}");
}

/// Times the adjustments of an inner adjuster.
struct Timed {
    inner: QvtoAdjuster,
    total: std::time::Duration,
    n: u32,
}

impl<C: Compat, F: Flavour> simoxide_spd::sim::Adjuster<C, F> for Timed {
    fn adjust(
        &mut self,
        sim: &mut Simulation<'_, C, F>,
        spd: &Spd,
        state: &mut simoxide_spd::SpdState,
        req: &simoxide_spd::AdjustmentRequest,
    ) -> Result<simoxide_spd::AdjustOutcome, simoxide_sim::SimError> {
        let t0 = std::time::Instant::now();
        let r = self.inner.adjust_on(sim, spd, state, req);
        self.total += t0.elapsed();
        self.n += 1;
        r
    }
}

#[test]
#[ignore]
fn cost_per_scaling_step() {
    let t0 = std::time::Instant::now();
    let core = QvtoCore::new().unwrap();
    let _ = QvtoAdjuster::spd(&core, &SpdAdjust::new(&Quirks::fixed())).unwrap();
    println!(
        "QVTo set-up (core, MainTransformation): {:.1} ms",
        t0.elapsed().as_secs_f64() * 1e3
    );
    for k in [1, 10, 30, 40] {
        let dir = services_model(k, 0.4);
        let model = RunSpec::default().load_model(&dir).unwrap();
        let objects = model.graph.object_count();
        let s = setup(&dir);
        let hpa = Hpa::new(&s.cfg, &s.spd).unwrap();
        let sim = Simulation::<Fast>::create(&s.cm, sim_cfg(1, 900), Outputs::default()).unwrap();
        let mut r = SpdRun::new(sim, &s.spd).unwrap().with_hpa(&hpa).unwrap();
        let mut adj = Timed {
            inner: QvtoAdjuster::spd(&core, &SpdAdjust::new(&Quirks::fixed())).unwrap(),
            total: Default::default(),
            n: 0,
        };
        let mut log = NoLog;
        let wall = std::time::Instant::now();
        r.run_until(&mut adj, &mut log, Some(300 * S)).unwrap();
        // a step on every service: 0.2 -> 2 offered cores
        let text: String = (0..k)
            .map(|i| format!("arrival-rate svc{i} 4.0\n"))
            .collect();
        ChangeFile::parse(&text).unwrap().apply(&mut r.sim).unwrap();
        r.model_changed();
        r.run_until(&mut adj, &mut log, Some(890 * S)).unwrap();
        let c0 = std::time::Instant::now();
        ChangeFile::parse("rate Server0:CPU 1.0")
            .unwrap()
            .apply(&mut r.sim)
            .unwrap();
        let change = c0.elapsed().as_secs_f64() * 1e3;
        r.run(&mut adj, &mut log).unwrap();
        let wall = wall.elapsed().as_secs_f64();
        let created: u64 = r.hpa_summary().iter().map(|s| s.created).sum();
        println!(
            "k={k}: {objects} objects (graph before QVTo), {} adjustments ({created} pods created), {:.2} ms per adjustment, change path alone {change:.2} ms; run {wall:.2} s wall, events {}",
            adj.n,
            adj.total.as_secs_f64() * 1e3 / f64::from(adj.n.max(1)),
            r.sim.state().events()
        );
    }
}

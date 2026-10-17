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

/// A ramp from 0.5 to 4.5 offered cores in 60 s steps: the replica count follows the load up
/// without a scale-down, to about the load / 0.5.
#[test]
fn a_load_ramp_scales_out_monotonically() {
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

//! P2 (`DESIGN-SPD.md` §7): Slingshot's SPD adjustment through `MainTransformation` in a
//! running simulation (SimuLizar flavour), compared with the Slingshot reference runner's `det`
//! runs (V3): the cases of `corpus-spd/` (`case.json`: model directory, simulated time, seed,
//! the reference's `expected/spd.jsonl`). With all compat flags on (Slingshot's unchanged
//! transformations, ordered QVTo) the decision log equals the reference's, `adjust` records
//! with Slingshot's change count and the model summaries before and after (containers, links,
//! allocation, target-group elements, in model order) included.

use serde_json::Value;
use simoxide_reconf::{QvtMode, QvtoAdjuster, QvtoCore, SpdAdjust};
use simoxide_sim::{CompiledModel, Exact, Outputs, RunSpec, Simulation};
use simoxide_spd::log::{VecLog, first_difference, project_with_models};
use simoxide_spd::sim::{SpdRun, spd_of};
use simoxide_spd::{Quirks, Spd};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn core() -> &'static QvtoCore {
    static CORE: OnceLock<QvtoCore> = OnceLock::new();
    CORE.get_or_init(|| QvtoCore::new().expect("core"))
}

/// A case of `corpus-spd/`.
struct Case {
    name: String,
    model: PathBuf,
    sim_time: i64,
    seed: i64,
    expected: PathBuf,
    /// The phase the case waits for (`waits_for`, e.g. `F4`: scale-in needs the change path's
    /// removal of containers and allocation contexts). Until then the run ends with the change
    /// path's refusal at that adjustment and the records before it are compared.
    waits_for: Option<String>,
}

fn cases() -> Vec<Case> {
    let dir = root().join("corpus-spd");
    let mut v: Vec<Case> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("case.json").is_file())
        .map(|p| {
            let c: Value =
                serde_json::from_str(&std::fs::read_to_string(p.join("case.json")).unwrap())
                    .unwrap();
            Case {
                name: p.file_name().unwrap().to_string_lossy().into_owned(),
                model: root().join(c["model"].as_str().unwrap()),
                sim_time: c["sim_time"].as_i64().unwrap(),
                seed: c["seed"].as_i64().unwrap(),
                expected: p.join("expected/spd.jsonl"),
                waits_for: c["waits_for"].as_str().map(String::from),
            }
        })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// The case's model for the SimuLizar flavour: a copy of the directory (below the target
/// directory) whose utilisation monitors get a `TimeDriven` processing type (SimuLizar needs
/// one; Slingshot's feed-through samples do not exist there). The monitor repository stays, so
/// Slingshot's monitor clones (the change count) are as in the reference.
fn simulizar_copy(dir: &Path) -> PathBuf {
    const UTIL: &str = "_QIb6cikUEeSuf8LV7cHLgA";
    // one copy per call: the tests run in parallel
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("spd-simulizar-{}-{n}", std::process::id()))
        .join(dir.file_name().unwrap());
    std::fs::create_dir_all(&out).unwrap();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if !p.is_file() {
            continue;
        }
        let mut text = std::fs::read_to_string(&p).unwrap();
        if p.extension().is_some_and(|x| x == "monitorrepository") {
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
        std::fs::write(out.join(p.file_name().unwrap()), text).unwrap();
    }
    out
}

/// Runs a case with `quirks` and QVTo `mode`; the decision log, and the error that ended the
/// run early.
fn run(c: &Case, quirks: Quirks, mode: QvtMode) -> (String, Option<String>) {
    let model = RunSpec::default()
        .load_model(&simulizar_copy(&c.model))
        .unwrap();
    let spd: Spd = spd_of(&model, quirks).unwrap().expect("an SPD");
    let cm = CompiledModel::compile(model).unwrap();
    let mut cfg = RunSpec::default().sim_config("p2");
    cfg.seed = c.seed;
    cfg.max_sim_time = c.sim_time;
    let sim = Simulation::<Exact>::new(&cm, cfg, Outputs::default()).unwrap();
    let mut opts = SpdAdjust::new(&quirks);
    opts.mode = mode;
    let mut adj = QvtoAdjuster::spd(core(), &opts).unwrap();
    let mut r = SpdRun::new(sim, &spd).unwrap();
    let mut log = VecLog::default();
    if let Err(e) = r.run(&mut adj, &mut log) {
        return (log.to_jsonl(), Some(e.message));
    }
    let (_, _, summary) = r.finish().unwrap();
    assert_eq!(summary.pending, 0, "{}", c.name);
    (log.to_jsonl(), None)
}

fn compare(name: &str, want: &[Value], got: &str) {
    let got = project_with_models(got).unwrap();
    if let Some(d) = first_difference(want, &got) {
        panic!("{name}: {d}");
    }
}

/// V3: the decision log with Slingshot's adjustments equals the reference's, model summaries
/// included, in QVTo's ordered and fast modes.
#[test]
fn adjustments_equal_slingshot() {
    let cases = cases();
    assert!(cases.len() >= 3, "{} cases", cases.len());
    for c in &cases {
        let refl = std::fs::read_to_string(&c.expected).unwrap();
        let want = project_with_models(&refl).unwrap();
        let adjusts = want.iter().filter(|v| v["k"] == "adjust").count();
        assert!(adjusts > 0, "{}", c.name);
        for mode in [QvtMode::InsertionOrderSets, QvtMode::Fast] {
            let (got, err) = run(c, Quirks::slingshot(), mode);
            let name = format!("{} {mode:?}", c.name);
            match (&c.waits_for, err) {
                (None, None) => compare(&name, &want, &got),
                (Some(_), Some(e)) => {
                    assert!(e.contains("cannot remove"), "{name}: {e}");
                    let got = project_with_models(&got).unwrap();
                    let n = got.len();
                    assert!(n > 0 && n < want.len(), "{name}");
                    if let Some(d) = first_difference(&want[..n], &got) {
                        panic!("{name}: {d}");
                    }
                    eprintln!("{name}: {n} of {} records equal before {e}", want.len());
                }
                (w, e) => panic!("{name}: waits for {w:?}, error {e:?}"),
            }
        }
        if c.waits_for.is_none() {
            eprintln!(
                "{}: {} records, {adjusts} adjustments equal",
                c.name,
                want.len()
            );
        }
    }
}

fn case(name: &str) -> Case {
    cases().into_iter().find(|c| c.name == name).unwrap()
}

struct Setup {
    cm: CompiledModel,
    spd: Spd,
    cfg: simoxide_sim::SimConfig,
}

fn setup(c: &Case, quirks: Quirks) -> Setup {
    let model = RunSpec::default()
        .load_model(&simulizar_copy(&c.model))
        .unwrap();
    let spd = spd_of(&model, quirks).unwrap().expect("an SPD");
    let cm = CompiledModel::compile(model).unwrap();
    let mut cfg = RunSpec::default().sim_config("p2");
    cfg.seed = c.seed;
    cfg.max_sim_time = c.sim_time;
    Setup { cm, spd, cfg }
}

fn adjuster(quirks: Quirks) -> QvtoAdjuster {
    QvtoAdjuster::spd(core(), &SpdAdjust::new(&quirks)).unwrap()
}

fn adjust_records(log: &str) -> Vec<Value> {
    project_with_models(log)
        .unwrap()
        .into_iter()
        .filter(|v| v["k"] == "adjust")
        .collect()
}

/// The default (fixed) behaviour runs the patched transformations: in the minimal example the
/// new container joins the link of its unit at the end (fixed Q6: the link keeps its
/// containers in their order, so nothing moves and SimuLizar's router accepts the change,
/// MC-2.3), everything else as in Slingshot; the time-policy model runs to its end.
#[test]
fn fixed_behaviour_runs_the_patched_transformations() {
    let c = case("spd_minimal");
    let (got, err) = run(&c, Quirks::fixed(), QvtMode::InsertionOrderSets);
    assert_eq!(err, None);
    let a = adjust_records(&got);
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["ok"], true);
    assert_eq!(
        a[0]["after"]["links"],
        serde_json::json!([[
            "LinkingResource1",
            "AppServer",
            "Loadbalancer_RC",
            "AppServer_1"
        ]])
    );
    let refl = std::fs::read_to_string(&c.expected).unwrap();
    let r = adjust_records(&refl);
    for k in ["containers", "allocation", "target_groups"] {
        assert_eq!(a[0]["after"][k], r[0]["after"][k], "{k}");
    }
    // F4: the fixed behaviour of spd_time_policies removes an allocation context at its first
    // scale-in (the change path refuses it until F4); every adjustment before it succeeds
    for mode in [QvtMode::InsertionOrderSets, QvtMode::Fast] {
        let (got, err) = run(&case("spd_time_policies"), Quirks::fixed(), mode);
        assert!(err.is_some_and(|e| e.contains("cannot remove")), "{mode:?}");
        let a = adjust_records(&got);
        assert!(!a.is_empty() && a.iter().all(|a| a["ok"] == true));
    }
}

/// An SPD run with adjustments, written to snapshots (sections `engine`, `model`/`graph`,
/// `spd`, `qvto`) at several times and restored into a fresh run with a fresh adjuster each
/// time, continues like the uninterrupted run: decision log (model summaries included) and
/// measurements equal; a restored run writes the bytes it was read from.
#[test]
fn snapshots_with_adjustments_continue_identically() {
    use simoxide_sim::snapshot::{Encoding, Snapshot};
    // the fixed behaviour of spd_time_policies scales in (F4)
    for (name, q) in [
        ("spd_time_policies", Quirks::slingshot()),
        ("spd_minimal", Quirks::fixed()),
    ] {
        let c = case(name);
        let s = setup(&c, q);
        let sim = Simulation::<Exact>::new(&s.cm, s.cfg.clone(), Outputs::default()).unwrap();
        let mut whole = SpdRun::new(sim, &s.spd).unwrap();
        let mut wlog = VecLog::default();
        whole.run(&mut adjuster(q), &mut wlog).unwrap();
        let (wres, wst, _) = whole.finish().unwrap();

        let sim = Simulation::<Exact>::new(&s.cm, s.cfg.clone(), Outputs::default()).unwrap();
        let mut r = SpdRun::new(sim, &s.spd).unwrap();
        let mut adj = adjuster(q);
        let mut log = VecLog::default();
        let mut generations = Vec::new();
        for t in [5, 20, 33, 50, 51, 60, 77, 90] {
            r.run_until(&mut adj, &mut log, Some(t * 1_000_000_000))
                .unwrap();
            let snap = |r: &mut SpdRun<'_, '_, Exact>, adj: &QvtoAdjuster| {
                let mut snap = r.snapshot_value(Encoding::Postcard).unwrap();
                snap.sections
                    .push(adj.run.snapshot_section(Encoding::Postcard).unwrap());
                let mut bytes = Vec::new();
                snap.write(&mut bytes).unwrap();
                bytes
            };
            let bytes = snap(&mut r, &adj);
            generations.push(r.sim.state().model().generation());
            drop(r);
            let read = Snapshot::read(&bytes[..]).unwrap();
            r = SpdRun::restore_snapshot(&read, &s.cm, &s.spd, Outputs::default()).unwrap();
            adj = adjuster(q);
            adj.run.restore_section(&r.sim, &read).unwrap();
            let again = snap(&mut r, &adj);
            if again != bytes {
                let a = Snapshot::read(&again[..]).unwrap();
                for (x, y) in read.sections.iter().zip(&a.sections) {
                    assert!(x == y, "{q:?} t={t}: section {} differs", x.name);
                }
                panic!("{q:?} t={t}: header differs");
            }
        }
        r.run(&mut adj, &mut log).unwrap();
        let (res, st, _) = r.finish().unwrap();
        assert!(
            generations.windows(2).any(|w| w[0] < w[1]),
            "{generations:?}"
        );
        assert_eq!(log.to_jsonl(), wlog.to_jsonl(), "{q:?}");
        assert_eq!(st, wst);
        assert_eq!(res.measurements.to_csv(), wres.measurements.to_csv());
    }
}

/// Bounded growth: repeated adjustments (enforced scale-outs of the minimal example) add the
/// same number of graph objects each time (the containers, assemblies, allocations, roles and
/// branches of one replica; nothing per invocation in the QVTo state), and the journal of each
/// run is about the same size.
#[test]
fn repeated_adjustments_grow_the_model_by_one_replica_each() {
    let c = case("spd_minimal");
    let q = Quirks::fixed();
    let s = setup(&c, q);
    let mut cfg = s.cfg.clone();
    cfg.max_sim_time = 1000;
    let sim = Simulation::<Exact>::new(&s.cm, cfg, Outputs::default()).unwrap();
    let mut r = SpdRun::new(sim, &s.spd).unwrap();
    let mut adj = adjuster(q);
    let mut log = VecLog::default();
    let mut counts = Vec::new();
    let mut journals = Vec::new();
    let adjustments = |log: &VecLog| adjust_records(&log.to_jsonl()).len();
    for k in 1..=40 {
        r.run_until(&mut adj, &mut log, Some(k * 10_000_000_000))
            .unwrap();
        r.enforce_policies(&["Basic Point in Time Trigger"])
            .unwrap();
        r.run_until(&mut adj, &mut log, Some(k * 10_000_000_000 + 1))
            .unwrap();
        counts.push((r.sim.model_graph().object_count(), adjustments(&log)));
        journals.push(adj.last.as_ref().unwrap().journal.len());
    }
    // objects per adjustment (the model's own time policy adds one at t = 50)
    let d: Vec<usize> = counts
        .windows(2)
        .map(|w| (w[1].0 - w[0].0) / (w[1].1 - w[0].1))
        .collect();
    assert!(d.iter().all(|&x| x == d[0]), "{d:?}");
    assert!(d[0] > 0 && d[0] < 100, "{d:?}");
    let (lo, hi) = (journals[5], journals[39]);
    assert!(hi < lo * 2 + 200, "journal {lo} -> {hi}");
    assert!(
        adjust_records(&log.to_jsonl())
            .iter()
            .all(|a| a["ok"] == true)
    );
}

/// The SPD corpus of the QVTo engine (`crates/qvto/qvto-testkit/corpus/spd/q1`): the model
/// directory `state` with the semantic configuration `cfg`.
fn q1_model(state: &str, cfg: &str) -> simoxide_model::Model {
    let dir = qvto_paths::root()
        .join("qvto-testkit/corpus/spd/q1")
        .join(state);
    let mut files = vec![
        dir.join("default.usagemodel"),
        dir.join("default.allocation"),
        dir.join("default.spd"),
        dir.join(cfg),
    ];
    files.dedup();
    let m = simoxide_model::load_files(&files);
    for d in &m.diagnostics.0 {
        assert!(
            d.level != simoxide_model::Level::Error,
            "{state}/{cfg}: {d}"
        );
    }
    m
}

/// A dry run of `policy` (enacted, as `SpdAdjustmentBehavior` sets it) in `variant` on the
/// not-started simulation of `model`, after `edit`: the transformation's success and the
/// element counts of the target-group configurations and the number of containers after it.
fn dry(
    model: simoxide_model::Model,
    variant: simoxide_reconf::SpdVariant,
    policy: &str,
    edit: &dyn Fn(&mut simoxide_model::raw::Graph, &Spd),
) -> (bool, Vec<usize>, usize) {
    use qvto_model::{Datum, ModelStore};
    use simoxide_model::meta::feat as mf;
    use simoxide_reconf::{Invocation, InvokeHooks, QvtoRun, RunConfig};
    let spd = spd_of(&model, Quirks::fixed()).unwrap().expect("an SPD");
    let cm = CompiledModel::compile(model).unwrap();
    let sim =
        Simulation::<Exact>::new(&cm, RunSpec::default().sim_config("p2"), Outputs::default())
            .unwrap();
    let t = simoxide_reconf::main_transformation(core(), variant).unwrap();
    let mut run = QvtoRun::new(
        core(),
        RunConfig {
            mode: QvtMode::InsertionOrderSets,
            ..RunConfig::default()
        },
    )
    .unwrap();
    let p = spd
        .model()
        .policies
        .iter()
        .find(|p| &*p.name == policy)
        .unwrap()
        .obj;
    let cfg = spd.model().configuration.unwrap();
    let mut before = |g: &mut simoxide_model::raw::Graph| {
        g.set(cfg, mf::Configuration_enactedPolicy, Datum::Obj(p))
            .unwrap();
        edit(g, &spd);
    };
    let inv = Invocation::slingshot().config("keepModeling", "true");
    let hooks = InvokeHooks {
        before: Some(&mut before),
        ids: None,
    };
    let (out, (sizes, containers)) = run
        .dry_run(&sim, &t, &inv, hooks, |g, _| {
            let m = simoxide_spd::Membership::from_graph(g, spd.model());
            let sizes = m
                .cfgs
                .iter()
                .map(|c| match c {
                    simoxide_spd::model::GroupCfg::ElasticInfrastructure { elements, .. }
                    | simoxide_spd::model::GroupCfg::ServiceGroup { elements, .. }
                    | simoxide_spd::model::GroupCfg::CompetingConsumers { elements, .. } => {
                        elements.len()
                    }
                })
                .collect();
            (sizes, m.environment.len())
        })
        .unwrap();
    (out.succeeded, sizes, containers)
}

/// The patched transformations change exactly what their quirk is about (dry runs on the Q1
/// corpus' states, without the change path, so scale-ins are covered before F4):
/// - Q4: a relative -10 % with a minimum of 2 on three containers removes none in Slingshot
///   (`min(-0.3, 2)` rounds to 0) and two with the fix (at least the minimum);
/// - Q7: a top-down scale-in that empties a container removes it in the same enactment with the
///   fix, at the next one in Slingshot;
/// - everything else equal (a plain step).
#[test]
fn patched_transformations_fix_their_quirk() {
    use simoxide_model::meta::feat as mf;
    use simoxide_reconf::SpdVariant;
    let fixed = SpdVariant::fixed();
    let sling = SpdVariant::slingshot();
    let none = |_: &mut simoxide_model::raw::Graph, _: &Spd| {};
    // Q4
    let rel = |g: &mut simoxide_model::raw::Graph, spd: &Spd| {
        use qvto_model::{Datum, ModelStore};
        let p = spd
            .model()
            .policies
            .iter()
            .find(|p| &*p.name == "relative 50 %, min 2")
            .unwrap()
            .obj;
        let adj = g.get_ref(p, mf::ScalingPolicy_adjustmentType).unwrap();
        g.set(
            adj,
            mf::RelativeAdjustment_percentageGrowthValue,
            Datum::Int(-10),
        )
        .unwrap();
    };
    let m = || q1_model("s_min_abs_3", "state.semanticspd");
    let (ok_s, s, _) = dry(m(), sling, "relative 50 %, min 2", &rel);
    let (ok_f, f, _) = dry(m(), fixed, "relative 50 %, min 2", &rel);
    assert!(ok_s && ok_f);
    assert_eq!((s[0], f[0]), (3, 1), "{s:?} {f:?}");
    // equal where no quirk is involved
    assert_eq!(
        dry(m(), sling, "step +3", &none),
        dry(m(), fixed, "step +3", &none)
    );
    // Q7: top-down scale-ins of the service group
    let mut seen = false;
    for (state, cfg) in [
        ("s_full_sg4_step_minus2", "state.semanticspd"),
        ("s_full_sg4_step_minus2", "full_sg2_step_minus1.semanticspd"),
        ("s_full_sg4_step_minus2", "full_sg2_step_0.semanticspd"),
        ("s_full_sg_step_plus3", "state.semanticspd"),
    ] {
        let dir = qvto_paths::root()
            .join("qvto-testkit/corpus/spd/q1")
            .join(state);
        if !dir.join(cfg).is_file() {
            continue;
        }
        for policy in ["SG step -1", "SG step -2"] {
            let s = dry(q1_model(state, cfg), sling, policy, &none);
            let f = dry(q1_model(state, cfg), fixed, policy, &none);
            eprintln!("{state}/{cfg} {policy}: slingshot {s:?}, fixed {f:?}");
            assert!(f.2 <= s.2, "{state} {policy}");
            assert_eq!(s.1[1..], f.1[1..], "{state} {policy}: service groups");
            seen |= f.2 < s.2;
        }
    }
    assert!(seen, "no scale-in emptied a container");
}

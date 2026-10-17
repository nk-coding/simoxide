//! The P1 exit criterion (`DESIGN-SPD.md` §7): the decision log of the time-triggered SPD models
//! equals the Slingshot reference runner's `spd.jsonl` (det mode), projected
//! ([`simoxide_spd::log::project`]: no event index, no log lines, adjustments reduced to time,
//! policy and success), with all SPD compat flags on; and, with the fixed behaviour, the
//! decisions Slingshot's evident intent gives (hand-derived from the reference).
//!
//! The adjustment itself is P2. The runs feed the reference's adjustment outcome into a test
//! double ([`ReferenceAdjuster`]): success and the target group's element count after each
//! adjustment, so that the decisions after an adjustment see the same group as Slingshot's.
//! On these models no later decision depends on the adjustment otherwise (time triggers, and
//! in compat mode Slingshot's constraint state changes at the request, Q2).

use serde_json::Value;
use simoxide_model::raw::ObjId;
use simoxide_sim::compat::Compat;
use simoxide_sim::{CompiledModel, Exact, Outputs, RunSpec, SimError, Simulation};
use simoxide_spd::log::{VecLog, first_difference, project};
use simoxide_spd::model::GroupKind;
use simoxide_spd::sim::{AdjustOutcome, Adjuster, SpdRun, placeholder_elements, spd_of};
use simoxide_spd::{AdjustmentRequest, Quirks, Spd, SpdState};
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn reproductions() -> PathBuf {
    root().join("reference/slingshot/upstream-report/reproductions")
}

/// The reference's adjustments: (policy, time, ok, element count of each target-group
/// configuration type after).
struct RefAdjust {
    policy: String,
    t: f64,
    ok: bool,
    after: Vec<(String, usize)>,
}

fn reference_adjustments(spd_jsonl: &str) -> Vec<RefAdjust> {
    spd_jsonl
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["k"] == "adjust")
        .map(|v| RefAdjust {
            policy: v["policy"].as_str().unwrap().to_string(),
            t: v["t"].as_f64().unwrap(),
            ok: v["ok"].as_bool().unwrap(),
            after: v["after"]["target_groups"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|g| {
                            (
                                g["type"].as_str().unwrap().to_string(),
                                g["elements"].as_array().unwrap().len(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

/// Test double of the adjustment: reports the reference's outcome of the adjustment of the same
/// policy at the same time (success if the reference has none) and sets the target group's
/// element count to the reference's.
struct ReferenceAdjuster {
    adjustments: Vec<RefAdjust>,
}

impl<C: Compat> Adjuster<C> for ReferenceAdjuster {
    fn adjust(
        &mut self,
        sim: &mut Simulation<'_, C>,
        spd: &Spd,
        state: &mut SpdState,
        req: &AdjustmentRequest,
    ) -> Result<AdjustOutcome, SimError> {
        let now = sim.state().now_ns() as f64 / 1e9;
        let policy = &spd.model().policies[req.policy];
        let group = &spd.model().groups[req.group];
        let before = state.membership.group_size(group);
        let i = self
            .adjustments
            .iter()
            .position(|a| a.policy == &*policy.id && a.t == now);
        let Some(i) = i else {
            return Ok(AdjustOutcome {
                ok: true,
                size_before: before,
                size_after: before,
            });
        };
        let a = self.adjustments.remove(i);
        let cfg_type = match group.kind {
            GroupKind::ElasticInfrastructure { .. } => "ElasticInfrastructureCfg",
            GroupKind::ServiceGroup { .. } => "ServiceGroupCfg",
            GroupKind::CompetingConsumers { .. } => "CompetingConsumersGroupCfg",
        };
        let after = a.after.iter().find(|(t, _)| t == cfg_type).map(|x| x.1);
        if let Some(n) = after {
            let keep: Vec<ObjId> = match state.membership.cfg_of(group) {
                Some(
                    simoxide_spd::model::GroupCfg::ElasticInfrastructure { elements, .. }
                    | simoxide_spd::model::GroupCfg::ServiceGroup { elements, .. }
                    | simoxide_spd::model::GroupCfg::CompetingConsumers { elements, .. },
                ) => elements.clone(),
                None => Vec::new(),
            };
            state
                .membership
                .set_elements(group, placeholder_elements(n, &keep));
        }
        Ok(AdjustOutcome {
            ok: a.ok,
            size_before: before,
            size_after: after,
        })
    }
}

/// Loads `dir` like `RunSpec::load_model`, but without the monitor repository: Slingshot's
/// monitors (utilisation without a `TimeDriven` processing type) do not compile in the
/// SimuLizar flavour, and time triggers read no measurements.
fn load(dir: &Path) -> simoxide_model::Model {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.extension().is_some_and(|x| {
                ["usagemodel", "allocation", "spd", "semanticspd"].contains(&x.to_str().unwrap())
            })
        })
        .collect();
    files.sort_by_key(|p| {
        let e = p.extension().unwrap().to_str().unwrap().to_string();
        (
            ["usagemodel", "allocation", "spd", "semanticspd"]
                .iter()
                .position(|x| *x == e),
            p.clone(),
        )
    });
    let m = simoxide_model::load_files(&files);
    for d in &m.diagnostics.0 {
        assert!(
            d.level != simoxide_model::Level::Error,
            "{}: {d}",
            dir.display()
        );
    }
    m
}

/// Runs `dir` for `sim_time` s with `quirks`, feeding the adjustments of `reference`; returns the
/// decision log.
fn run(dir: &Path, sim_time: i64, quirks: Quirks, reference: &str) -> String {
    let spec = RunSpec::default();
    let model = load(dir);
    let spd = spd_of(&model, quirks).unwrap();
    let cm = CompiledModel::compile(model).unwrap();
    let mut cfg = spec.sim_config("p1");
    cfg.seed = 1;
    cfg.max_sim_time = sim_time;
    let sim = Simulation::<Exact>::new(&cm, cfg, Outputs::default()).unwrap();
    let mut log = VecLog::default();
    let Some(spd) = spd else {
        // no SPD: an empty decision log. (The probes are Slingshot-semantics models; some fail
        // in the SimuLizar flavour, e.g. `ss_guard_none`, which does not change the log.)
        let _ = sim.finish();
        return String::new();
    };
    let mut r = SpdRun::new(sim, &spd).unwrap();
    let mut adj = ReferenceAdjuster {
        adjustments: reference_adjustments(reference),
    };
    r.run(&mut adj, &mut log).unwrap();
    let (_, _, summary) = r.finish().unwrap();
    assert_eq!(summary.pending, 0, "{}", dir.display());
    log.to_jsonl()
}

fn compare(name: &str, want: &[Value], got: &str) {
    let got = project(got).unwrap();
    if let Some(d) = first_difference(want, &got) {
        panic!("{name}: {d}");
    }
}

/// (model directory, reference spd.jsonl, simulation time)
fn cases() -> Vec<(String, PathBuf, PathBuf, i64)> {
    let r = reproductions();
    let mut v = vec![
        (
            "spd_minimal".to_string(),
            root().join("reference/slingshot/models/spd_minimal"),
            r.join("spd_minimal/observed/spd.jsonl"),
            100,
        ),
        (
            "spd_scale_in".to_string(),
            root().join("reference/slingshot/models/spd_scale_in"),
            r.join("spd_scale_in/observed/spd.jsonl"),
            100,
        ),
        (
            "spd_or_trigger".to_string(),
            r.join("spd_or_trigger"),
            r.join("spd_or_trigger/observed/spd.jsonl"),
            100,
        ),
        (
            "spd_and_trigger".to_string(),
            r.join("spd_and_trigger"),
            r.join("spd_and_trigger/observed/spd.jsonl"),
            100,
        ),
    ];
    let probes = root().join("reference/slingshot/models/probes");
    let mut ps: Vec<PathBuf> = std::fs::read_dir(&probes)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("probe.json").exists())
        .collect();
    ps.sort();
    for p in ps {
        let probe: Value =
            serde_json::from_str(&std::fs::read_to_string(p.join("probe.json")).unwrap()).unwrap();
        let t = probe["sim_time"].as_f64().unwrap().ceil() as i64;
        v.push((
            p.file_name().unwrap().to_string_lossy().into_owned(),
            p.clone(),
            p.join("expected/spd.jsonl"),
            t.max(1),
        ));
    }
    v
}

#[test]
fn decision_log_equals_reference_with_compat_flags() {
    let cases = cases();
    assert!(cases.len() >= 24, "{} cases", cases.len());
    let mut with_spd = 0;
    for (name, dir, reference, t) in cases {
        let refl = std::fs::read_to_string(&reference).unwrap();
        let want = project(&refl).unwrap();
        let got = run(&dir, t, Quirks::slingshot(), &refl);
        compare(&name, &want, &got);
        if !want.is_empty() {
            with_spd += 1;
        }
        eprintln!("{name}: {} records equal", want.len());
    }
    assert_eq!(with_spd, 4);
}

fn lines(v: &[&str]) -> Vec<Value> {
    v.iter().map(|l| serde_json::from_str(l).unwrap()).collect()
}

const MIN_POLICY: &str = r#"{"k":"policy","id":"_7VIU0MAgEe6AcKIPVXJsSw","name":"Basic Point in Time Trigger","active":true,"target_group":"_DC8fM8pgEe6u3p81um7oyQ","adjustment":"StepAdjustment","trigger":"SimpleFireOnValue","chain":["TargetGroupChecker","SimulationTimeChecker","Adjustor"]}"#;

#[test]
fn fixed_behaviour_minimal_and_scale_in() {
    // spd_minimal: no quirk is involved, the fixed log equals the reference
    let r = reproductions();
    let refl = std::fs::read_to_string(r.join("spd_minimal/observed/spd.jsonl")).unwrap();
    let got = run(
        &root().join("reference/slingshot/models/spd_minimal"),
        100,
        Quirks::fixed(),
        &refl,
    );
    compare("spd_minimal (fixed)", &project(&refl).unwrap(), &got);

    // spd_scale_in: fixed Q15, each policy sees only its own time event; the other policy's
    // event is rejected by the target-group check and not logged
    let refl = std::fs::read_to_string(r.join("spd_scale_in/observed/spd.jsonl")).unwrap();
    let got = run(
        &root().join("reference/slingshot/models/spd_scale_in"),
        100,
        Quirks::fixed(),
        &refl,
    );
    let want = lines(&[
        MIN_POLICY,
        r#"{"k":"eval","t":50.0,"ev":"SimulationTimeReached","policy":"_7VIU0MAgEe6AcKIPVXJsSw","filters":[["TargetGroupChecker","ok"],["SimulationTimeChecker","ok"],["Adjustor","ok"]],"request":true}"#,
        r#"{"k":"policy","id":"_simoxideScaleInPolicy0001","name":"Scale in at 80","active":true,"target_group":"_DC8fM8pgEe6u3p81um7oyQ","adjustment":"StepAdjustment","trigger":"SimpleFireOnValue","chain":["TargetGroupChecker","SimulationTimeChecker","Adjustor"]}"#,
        r#"{"k":"adjust","t":50.0,"policy":"_7VIU0MAgEe6AcKIPVXJsSw","policy_name":"Basic Point in Time Trigger","ok":true}"#,
        r#"{"k":"eval","t":80.0,"ev":"SimulationTimeReached","policy":"_simoxideScaleInPolicy0001","filters":[["TargetGroupChecker","ok"],["SimulationTimeChecker","ok"],["Adjustor","ok"]],"request":true}"#,
        r#"{"k":"adjust","t":80.0,"policy":"_simoxideScaleInPolicy0001","policy_name":"Scale in at 80","ok":true}"#,
    ]);
    compare("spd_scale_in (fixed)", &want, &got);
}

#[test]
fn fixed_behaviour_composed_triggers() {
    let r = reproductions();
    // OR(t == 50, t == 70): two adjustments
    let refl = std::fs::read_to_string(r.join("spd_or_trigger/observed/spd.jsonl")).unwrap();
    let got = run(&r.join("spd_or_trigger"), 100, Quirks::fixed(), &refl);
    let p = "_7VIU0MAgEe6AcKIPVXJsSw";
    let pol = |name: &str, f: &str| {
        format!(
            r#"{{"k":"policy","id":"{p}","name":"{name}","active":true,"target_group":"_DC8fM8pgEe6u3p81um7oyQ","adjustment":"StepAdjustment","trigger":"ComposedTrigger","chain":["TargetGroupChecker","{f}","Adjustor"]}}"#
        )
    };
    let or_ok = |t: &str| {
        format!(
            r#"{{"k":"eval","t":{t},"ev":"SimulationTimeReached","policy":"{p}","filters":[["TargetGroupChecker","ok"],["LogicalORCompoundFilter","ok"],["Adjustor","ok"]],"request":true}}"#
        )
    };
    let adj = |t: &str, name: &str| {
        format!(r#"{{"k":"adjust","t":{t},"policy":"{p}","policy_name":"{name}","ok":true}}"#)
    };
    let n = "OR of t == 50 and t == 70";
    let want = lines(&[
        &pol(n, "LogicalORCompoundFilter"),
        &or_ok("50.0"),
        &adj("50.0", n),
        &or_ok("70.0"),
        &adj("70.0", n),
    ]);
    compare("spd_or_trigger (fixed)", &want, &got);

    // AND(t == 50, t == 70): never
    let refl = std::fs::read_to_string(r.join("spd_and_trigger/observed/spd.jsonl")).unwrap();
    let got = run(&r.join("spd_and_trigger"), 100, Quirks::fixed(), &refl);
    let n = "AND of t == 50 and t == 70";
    let and_no = |t: &str, reason: &str| {
        format!(
            r#"{{"k":"eval","t":{t},"ev":"SimulationTimeReached","policy":"{p}","filters":[["TargetGroupChecker","ok"],["LogicalANDComboundFilter","disregard","{reason}"]],"request":false}}"#
        )
    };
    let want = lines(&[
        &pol(n, "LogicalANDComboundFilter"),
        &and_no(
            "50.0",
            "Simulationtime of 50.000000 is not [EqualTo] expectedValue. [[DISREGARD]]",
        ),
        &and_no(
            "70.0",
            "Simulationtime of 70.000000 is not [EqualTo] expectedValue. [[DISREGARD]]",
        ),
    ]);
    compare("spd_and_trigger (fixed)", &want, &got);
}

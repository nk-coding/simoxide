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
    let out = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("spd-simulizar")
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

//! The Slingshot-exact mode (`DESIGN-SPD.md` X2, `docs/spec/slingshot/exact.md`) against the
//! Slingshot-exact corpus `corpus-ss/` (the `det` reference runner's outputs, SS-X-8): the
//! level-E projection, the tape projection (draws with their sites and positions) and the
//! measurements must be byte-equal (V3x).
//!
//! Every case is run; the ones listed in [`KNOWN`] are allowed to differ (with the cause), every
//! other case must be equal in all three files. A known case that becomes equal fails the test
//! too, so that the list stays accurate.

use simoxide_sim::{CompiledModel, FlavourConfig, Outputs, RunSpec, SimConfig};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// Cases that are not (yet) equal, with the cause.
const KNOWN: &[(&str, &str)] = &[(
    "h27_collection_inner_multi",
    "INNER evaluation order (java.util.HashMap)",
)];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(Clone, Default)]
struct Buf(Rc<RefCell<Vec<u8>>>);

impl std::io::Write for Buf {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Buf {
    fn text(&self) -> String {
        String::from_utf8(self.0.borrow().clone()).unwrap()
    }
}

struct Outcome {
    /// The SPD decision log (`spd.jsonl`), for the SPD cases.
    spd_log: Option<String>,
    projection: String,
    tape: String,
    csv: Option<String>,
    error: Option<String>,
    warnings: Vec<String>,
}

fn run_case(name: &str) -> (Outcome, PathBuf) {
    run_case_in("corpus-ss", name)
}

fn run_case_in(corpus: &str, name: &str) -> (Outcome, PathBuf) {
    let dir = root().join(corpus).join(name);
    let case: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("case.json")).unwrap()).unwrap();
    let model_dir = root().join(case["model"].as_str().unwrap());
    let model = RunSpec::default()
        .load_model(&model_dir)
        .expect("model loads");
    let cm = CompiledModel::compile_flavour(model, FlavourConfig::slingshot_exact()).unwrap();
    let cfg = SimConfig {
        seed: case["seed"].as_i64().unwrap(),
        max_sim_time: case["sim_time"].as_i64().unwrap(),
        max_measurements: -1,
        ..Default::default()
    };
    let (p, t) = (Buf::default(), Buf::default());
    let out = Outputs {
        trace: Some(Box::new(p.clone())),
        tape: Some(Box::new(t.clone())),
    };
    let spd = simoxide_spd::sim::spd_of(&cm.model, simoxide_spd::Quirks::slingshot()).unwrap();
    let mut log = simoxide_spd::log::VecLog::default();
    let r = match &spd {
        // the SPD cases: Slingshot's adjustment in QVTo's ordered mode (the default build's)
        Some(spd) => {
            let core = simoxide_reconf::QvtoCore::new().unwrap();
            let mut adj = simoxide_reconf::QvtoAdjuster::spd(
                &core,
                &simoxide_reconf::SpdAdjust::new(&spd.quirks()),
            )
            .unwrap();
            let mut hooks = simoxide_spd::ssx::SsxSpd::new(spd, &cm.model, &mut adj, &mut log);
            simoxide_sim::ssx::run_with_spd(&cm, cfg, out, &mut hooks)
        }
        None => simoxide_sim::run(&cm, cfg, out),
    };
    let (csv, error, warnings) = match r {
        Ok(r) => (Some(r.measurements.to_csv()), None, r.warnings),
        Err(e) => (None, Some(e.to_string()), Vec::new()),
    };
    let o = Outcome {
        spd_log: spd.is_some().then(|| log.to_jsonl()),
        projection: p.text(),
        tape: t.text(),
        csv,
        error,
        warnings,
    };
    // `SSX_DUMP=DIR`: the outputs in DIR/<case>/ (for debugging)
    if let Some(d) = std::env::var_os("SSX_DUMP") {
        let d = Path::new(&d).join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("projection.jsonl"), &o.projection).unwrap();
        std::fs::write(d.join("tape-projection.jsonl"), &o.tape).unwrap();
        if let Some(c) = &o.csv {
            std::fs::write(d.join("measurements.csv"), c).unwrap();
        }
        if let Some(l) = &o.spd_log {
            std::fs::write(d.join("spd.jsonl"), l).unwrap();
        }
    }
    (o, dir.join("expected"))
}

/// `E`, `T`, `C`: projection, tape projection, measurements equal.
fn compare(name: &str) -> (bool, bool, bool, String) {
    compare_in("corpus-ss", name)
}

fn compare_in(corpus: &str, name: &str) -> (bool, bool, bool, String) {
    let (o, exp) = run_case_in(corpus, name);
    let read = |f: &str| std::fs::read_to_string(exp.join(f)).ok();
    if exp.join("error.txt").exists() {
        // the run fails before t = 0, like the reference
        let ok = o.error.is_some();
        return (ok, ok, ok, o.error.unwrap_or_default());
    }
    let e = read("projection.jsonl").as_deref() == Some(o.projection.as_str());
    let t = read("tape-projection.jsonl").as_deref() == Some(o.tape.as_str());
    let c = read("measurements.csv") == o.csv;
    let first = read("projection.jsonl")
        .unwrap_or_default()
        .lines()
        .zip(o.projection.lines())
        .position(|(a, b)| a != b)
        .map(|i| format!("first projection difference at line {}", i + 1))
        .unwrap_or_else(|| o.error.unwrap_or_default());
    (e, t, c, first)
}

#[test]
fn corpus_ss_is_byte_equal_at_level_e() {
    let mut names: Vec<String> = std::fs::read_dir(root().join("corpus-ss"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("case.json").exists())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert!(names.len() >= 88, "corpus-ss has {} cases", names.len());
    let mut failures = Vec::new();
    let mut equal = 0;
    for n in &names {
        let (e, t, c, why) = compare(n);
        let all = e && t && c;
        equal += usize::from(all);
        let known = KNOWN.iter().find(|k| k.0 == n);
        eprintln!(
            "{n:40} E{} T{} C{} {}",
            if e { '=' } else { 'x' },
            if t { '=' } else { 'x' },
            if c { '=' } else { 'x' },
            if all { String::new() } else { why.clone() }
        );
        match (all, known) {
            (false, None) => failures.push(format!("{n}: differs ({why})")),
            (true, Some(k)) => {
                failures.push(format!("{n}: now equal, remove it from KNOWN ({})", k.1))
            }
            _ => {}
        }
    }
    eprintln!("{equal} of {} cases byte-equal", names.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The exact mode is deterministic (two runs, same bytes).
#[test]
fn exact_mode_is_deterministic() {
    for n in [
        "x_ss_mediastore",
        "ss_two_demands",
        "h18_infrastructure_call",
    ] {
        let (a, _) = run_case(n);
        let (b, _) = run_case(n);
        assert_eq!(a.projection, b.projection, "{n}");
        assert_eq!(a.tape, b.tape, "{n}");
        assert_eq!(a.csv, b.csv, "{n}");
    }
}

/// The disposed handlers (Q18): every case with handler errors throws the same exceptions in the
/// same order as the reference (`expected/errors.txt`: one line per "An error occurred").
#[test]
fn handler_errors_match_the_reference() {
    let mut checked = 0;
    for e in std::fs::read_dir(root().join("corpus-ss"))
        .unwrap()
        .filter_map(|e| e.ok())
    {
        let name = e.file_name().to_string_lossy().into_owned();
        let errors = e.path().join("expected/errors.txt");
        let Ok(text) = std::fs::read_to_string(&errors) else {
            continue;
        };
        if text.trim().is_empty() || KNOWN.iter().any(|k| k.0 == name) {
            continue;
        }
        let want: Vec<&str> = text.lines().map(|l| l.split(':').next().unwrap()).collect();
        let (o, _) = run_case(&name);
        // "t=<time>: <class> in <handler>: <message>"
        let got: Vec<String> = o
            .warnings
            .iter()
            .filter_map(|w| w.split(": ").nth(1))
            .map(|x| x.split(' ').next().unwrap().to_string())
            .collect();
        assert_eq!(got, want, "{name}: {:#?}", o.warnings);
        checked += 1;
    }
    assert!(checked >= 22, "{checked} cases with handler errors");
}

/// The SPD cases: the decision log (evaluations, requests, adjustments with their change counts
/// and model summaries) equals the reference's `spd.jsonl` (`simoxide_spd::log::project_with_models`).
#[test]
fn spd_decision_logs_match_the_reference() {
    let mut checked = 0;
    for (corpus, name) in [
        ("corpus-ss", "spd_minimal"),
        ("corpus-ss", "spd_scale_in"),
        ("corpus-ss", "spd_time_policies"),
        ("corpus-spd", "spd_service_group"),
    ] {
        let (o, exp) = run_case_in(corpus, name);
        let want = std::fs::read_to_string(exp.join("spd.jsonl")).unwrap();
        let want = simoxide_spd::log::project_with_models(&want).unwrap();
        let got = simoxide_spd::log::project_with_models(&o.spd_log.unwrap()).unwrap();
        assert_eq!(
            simoxide_spd::log::first_difference(&want, &got),
            None,
            "{name}"
        );
        checked += 1;
    }
    assert_eq!(checked, 4);
}

/// The SPD corpus (`corpus-spd/`, P2): every case byte-equal at level E (the case without a
/// tape reference compares projection and measurements).
#[test]
fn corpus_spd_is_byte_equal_at_level_e() {
    for name in [
        "spd_minimal",
        "spd_scale_in",
        "spd_service_group",
        "spd_time_policies",
    ] {
        let has_tape = root()
            .join("corpus-spd")
            .join(name)
            .join("expected/tape-projection.jsonl")
            .exists();
        let (e, t, c, why) = compare_in("corpus-spd", name);
        eprintln!("{name:40} E{} T{} C{} {why}", e as u8, t as u8, c as u8);
        assert!(e && (t || !has_tape) && c, "{name}: {why}");
    }
}

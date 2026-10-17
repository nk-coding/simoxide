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
const KNOWN: &[(&str, &str)] = &[
    (
        "h04_delay_resource",
        "UniDouble: Palladio 6.0 sampler (Commons Math 3 inversion)",
    ),
    ("h05_open_workload", "Gamma: Palladio 6.0 sampler"),
    ("h06_closed_think", "Lognorm: Palladio 6.0 sampler"),
    ("h21_stoex_distributions", "Palladio 6.0 samplers"),
    (
        "h27_collection_inner_multi",
        "INNER evaluation order (java.util.HashMap)",
    ),
    ("x_pem_screencast_ms", "Norm: Palladio 6.0 sampler"),
    ("x_pem_screencast_ms_enqueue", "Norm: Palladio 6.0 sampler"),
    ("x_pem_screencast_ms_instant", "Norm: Palladio 6.0 sampler"),
    (
        "x_pem_simple_optimisation",
        "LognormMoments: Palladio 6.0 sampler",
    ),
    (
        "x_pem_simple_optimisation_composites",
        "LognormMoments: Palladio 6.0 sampler",
    ),
    ("spd_minimal", "SPD events (X2d)"),
    ("spd_scale_in", "SPD events (X2d)"),
    ("spd_time_policies", "SPD events (X2d)"),
];

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
    projection: String,
    tape: String,
    csv: Option<String>,
    error: Option<String>,
}

fn run_case(name: &str) -> (Outcome, PathBuf) {
    let dir = root().join("corpus-ss").join(name);
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
    let r = simoxide_sim::run(&cm, cfg, out);
    let (csv, error) = match r {
        Ok(r) => (Some(r.measurements.to_csv()), None),
        Err(e) => (None, Some(e.to_string())),
    };
    (
        Outcome {
            projection: p.text(),
            tape: t.text(),
            csv,
            error,
        },
        dir.join("expected"),
    )
}

/// `E`, `T`, `C`: projection, tape projection, measurements equal.
fn compare(name: &str) -> (bool, bool, bool, String) {
    let (o, exp) = run_case(name);
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

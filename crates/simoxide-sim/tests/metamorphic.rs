//! Metamorphic and property tests of the whole simulator (no reference needed). Each property is
//! checked on every `corpus/*` model and on generated models (`simoxide_testkit::modelgen`):
//!
//! - same seed twice gives identical trace, tape and measurements;
//! - runs without trace/tape give the measurements, draw count, event count and end time of the
//!   traced run; `run_batch` equals sequential runs;
//! - replaying the run's own tape reproduces the run;
//! - the finish line and tape agree with the result counters;
//! - a longer run (2x max measurements, or a later max time) has the same past: every
//!   measurement strictly before the shorter run's stop time is identical;
//! - scaling every resource demand and every processing rate by 2 (exact in binary floating
//!   point) leaves every service time, time stamp and measurement (except the abstract demand)
//!   unchanged;
//! - the order of the entry files given to the loader does not matter;
//! - an extra, unused component (repository only) and renamed `entityName`s change nothing.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use simoxide_sim::{
    CompiledModel, Outputs, RngMode, RunSpec, SimConfig, Simulation, Tape, run_batch,
};
use simoxide_testkit::modelgen::{self, GenConfig};

#[derive(Clone, Default)]
struct Buf(std::rc::Rc<std::cell::RefCell<Vec<u8>>>);

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

#[derive(Debug, PartialEq)]
struct Out {
    trace: String,
    tape: String,
    csv: String,
    uniforms: u64,
    events: u64,
    end_ns: i64,
    main_count: i64,
}

fn compile(dir: &Path, spec: &RunSpec) -> CompiledModel {
    CompiledModel::compile(spec.load_model(dir).unwrap()).unwrap()
}

fn run_cfg(cm: &CompiledModel, cfg: SimConfig, traced: bool) -> Out {
    let (t, p) = (Buf::default(), Buf::default());
    let outputs = if traced {
        Outputs {
            trace: Some(Box::new(t.clone())),
            tape: Some(Box::new(p.clone())),
        }
    } else {
        Outputs::default()
    };
    let r = Simulation::new(cm, cfg, outputs).unwrap().run().unwrap();
    Out {
        trace: t.text(),
        tape: p.text(),
        csv: r.measurements.to_csv(),
        uniforms: r.uniforms,
        events: r.events,
        end_ns: r.end_ns,
        main_count: r.main_count,
    }
}

/// Model directories: the corpus plus generated models (written to a scratch directory).
fn models(tag: &str) -> Vec<(String, PathBuf, RunSpec)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let mut v = Vec::new();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.join("run.json").exists())
        .collect();
    dirs.sort();
    for d in dirs {
        let spec = RunSpec::load(&d.join("run.json")).unwrap();
        v.push((
            d.file_name().unwrap().to_string_lossy().into_owned(),
            d,
            spec,
        ));
    }
    let scratch = common::scratch(&format!("meta-{tag}"));
    for seed in 0..30u64 {
        let mut cfg = GenConfig::new(format!("mm{seed}"), 7000 + seed, 1 + (seed % 10) as u32);
        // double resumes can abort a run (as in the reference); every other feature is on
        cfg.features.double_resume = 0.0;
        let m = modelgen::generate(&cfg);
        let dir = scratch.join(&cfg.name);
        modelgen::xmi::write_model(&m, &dir).unwrap();
        let spec = RunSpec::load(&dir.join("run.json")).unwrap();
        let ok = Simulation::new(
            &compile(&dir, &spec),
            spec.sim_config(&cfg.name),
            Outputs::default(),
        )
        .and_then(|s| s.run())
        .is_ok();
        assert!(ok || seed != 0, "generated model {} fails", cfg.name);
        if ok {
            v.push((cfg.name, dir, spec));
        }
    }
    v
}

#[test]
fn deterministic_and_trace_independent() {
    for (name, dir, spec) in models("det") {
        let cm = compile(&dir, &spec);
        let a = run_cfg(&cm, spec.sim_config(&name), true);
        let b = run_cfg(&cm, spec.sim_config(&name), true);
        assert!(a == b, "{name}: two runs with the same seed differ");
        let c = run_cfg(&cm, spec.sim_config(&name), false);
        assert_eq!(
            (&a.csv, a.uniforms, a.events, a.end_ns, a.main_count),
            (&c.csv, c.uniforms, c.events, c.end_ns, c.main_count),
            "{name}: untraced run differs"
        );
        // counters agree with the files
        let draws = a
            .tape
            .lines()
            .filter(|l| l.starts_with("{\"k\":\"u\""))
            .count() as u64;
        assert_eq!(draws, a.uniforms, "{name}: tape draws vs counter");
        let last = a.trace.lines().last().unwrap_or("");
        assert!(
            last.starts_with("{\"ev\":\"finish\"")
                && last.contains(&format!("\"uniforms\":{}", a.uniforms)),
            "{name}: finish line {last}"
        );
        let rows = a.csv.lines().count().saturating_sub(1) as u64;
        assert!(
            last.contains(&format!("\"measurements\":{rows}")),
            "{name}: finish line {last} vs {rows} rows"
        );
    }
}

#[test]
fn replaying_own_tape_reproduces_the_run() {
    for (name, dir, spec) in models("replay") {
        let cm = compile(&dir, &spec);
        let a = run_cfg(&cm, spec.sim_config(&name), true);
        let tape = Tape::parse(&a.tape).unwrap();
        let cfg = SimConfig {
            rng: RngMode::Replay(Arc::new(tape)),
            check_tape_origins: true,
            ..spec.sim_config(&name)
        };
        let b = run_cfg(&cm, cfg, true);
        assert!(a == b, "{name}: replay of the own tape differs");
    }
}

#[test]
fn batch_equals_sequential() {
    for (name, dir, spec) in models("batch") {
        let cm = compile(&dir, &spec);
        let cfgs: Vec<SimConfig> = (0..3)
            .map(|i| SimConfig {
                seed: spec.seed + i,
                ..spec.sim_config(&name)
            })
            .collect();
        let batch = run_batch(&cm, &cfgs, 3);
        for (cfg, b) in cfgs.iter().zip(batch) {
            let b = b.unwrap();
            let s = run_cfg(&cm, cfg.clone(), false);
            assert_eq!(b.measurements.to_csv(), s.csv, "{name} seed {}", cfg.seed);
            assert_eq!(
                (b.uniforms, b.events, b.end_ns),
                (s.uniforms, s.events, s.end_ns)
            );
        }
    }
}

/// CSV rows (header dropped) with time strictly before `t`.
fn rows_before(csv: &str, t: f64) -> Vec<&str> {
    csv.lines()
        .skip(1)
        .filter(|l| {
            let f: Vec<&str> = l.rsplitn(3, ',').collect();
            f.get(1)
                .and_then(|x| x.parse::<f64>().ok())
                .is_some_and(|x| x < t)
        })
        .collect()
}

#[test]
fn longer_runs_have_the_same_past() {
    for (name, dir, spec) in models("past") {
        let cm = compile(&dir, &spec);
        let short = run_cfg(&cm, spec.sim_config(&name), false);
        let mut cfg = spec.sim_config(&name);
        if cfg.max_measurements > 0 {
            cfg.max_measurements *= 2;
        }
        if cfg.max_sim_time > 0 {
            cfg.max_sim_time *= 2;
        }
        let long = run_cfg(&cm, cfg, false);
        // stop-time outputs (state at finalise, the last utilisation window) are stamped at the
        // stop instant or a hair before it (window arithmetic), hence the 1 ns margin
        let t = short.end_ns as f64 / 1e9 - 1e-9;
        assert_eq!(
            rows_before(&short.csv, t),
            rows_before(&long.csv, t),
            "{name}: measurements before t={t} differ"
        );
        assert!(long.end_ns >= short.end_ns, "{name}");
    }
}

/// Wraps the value of every `tag specification="…"` attribute as `(…) * 2`.
fn scale_attr(text: &str, tag: &str) -> String {
    let pat = format!("<{tag} specification=\"");
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find(&pat) {
        let j = i + pat.len();
        let k = rest[j..].find('"').unwrap() + j;
        out.push_str(&rest[..j]);
        out.push('(');
        out.push_str(&rest[j..k]);
        out.push_str(") * 2");
        rest = &rest[k..];
    }
    out.push_str(rest);
    out
}

fn copy_model(from: &Path, to: &Path, edit: impl Fn(&str, &str) -> String) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let p = e.unwrap().path();
        if p.is_file() {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            match std::fs::read_to_string(&p) {
                Ok(t) => std::fs::write(to.join(&name), edit(&name, &t)).unwrap(),
                Err(_) => {
                    std::fs::copy(&p, to.join(&name)).unwrap();
                }
            }
        }
    }
}

/// The trace without the abstract demand field `"d":…` of `demand` events and without the
/// resource-demand measurements.
fn strip_demand(trace: &str) -> String {
    trace
        .lines()
        .filter(|l| !l.contains("\"metric\":\"Resource Demand Tuple\""))
        .map(|l| {
            if l.starts_with("{\"ev\":\"demand\"")
                && let Some(i) = l.find(",\"d\":")
            {
                let j = l[i + 1..].find(",\"st\":").unwrap() + i + 1;
                format!("{}{}", &l[..i], &l[j..])
            } else {
                l.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn scaling_demands_and_rates_by_two_changes_nothing() {
    let scratch = common::scratch("scale");
    for (name, dir, spec) in models("scale") {
        let scaled = scratch.join(&name);
        copy_model(&dir, &scaled, |_, t| {
            let t = scale_attr(t, "specification_ParametericResourceDemand");
            let t = scale_attr(&t, "processingRate_ProcessingResourceSpecification");
            scale_attr(&t, "numberOfCalls__ResourceCall")
        });
        let a = run_cfg(&compile(&dir, &spec), spec.sim_config(&name), true);
        let b = run_cfg(&compile(&scaled, &spec), spec.sim_config(&name), true);
        let u = |tape: &str| -> Vec<String> {
            tape.lines()
                .filter(|l| l.starts_with("{\"k\":\"u\""))
                .map(String::from)
                .collect()
        };
        assert_eq!(u(&a.tape), u(&b.tape), "{name}: uniform draws differ");
        let (ta, tb) = (strip_demand(&a.trace), strip_demand(&b.trace));
        if ta != tb {
            let (i, (la, lb)) = ta
                .lines()
                .zip(tb.lines())
                .enumerate()
                .find(|(_, (x, y))| x != y)
                .unwrap_or((0, ("", "")));
            panic!(
                "{name}: scaled trace differs at line {}:\n  {la}\n  {lb}",
                i + 1
            );
        }
        let drop_demand = |csv: &str| -> Vec<String> {
            csv.lines()
                .filter(|l| !l.contains(",Resource Demand Tuple,"))
                .map(String::from)
                .collect()
        };
        assert_eq!(drop_demand(&a.csv), drop_demand(&b.csv), "{name}");
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

#[test]
fn entry_file_order_does_not_matter() {
    for (name, dir, spec) in models("order") {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension().is_some_and(|x| {
                    x == "usagemodel" || x == "allocation" || x == "monitorrepository"
                })
            })
            .collect();
        if let Some(u) = &spec.usagemodel {
            files.retain(|p| p.extension().is_some_and(|x| x != "usagemodel") || p.ends_with(u));
        }
        if !spec.allocation.is_empty() {
            files.retain(|p| {
                p.extension().is_some_and(|x| x != "allocation")
                    || spec.allocation.iter().any(|a| p.ends_with(a))
            });
        }
        if let Some(m) = &spec.monitorrepository {
            files.retain(|p| {
                p.extension().is_some_and(|x| x != "monitorrepository") || p.ends_with(m)
            });
        }
        files.sort();
        let base = run_cfg(&compile(&dir, &spec), spec.sim_config(&name), true);
        files.reverse();
        let m = simoxide_model::load_files(&files);
        let cm = CompiledModel::compile(m).unwrap();
        let rev = run_cfg(&cm, spec.sim_config(&name), true);
        assert!(
            base == rev,
            "{name}: reversed entry-file order changes the run"
        );
    }
}

/// Appends a basic component that nothing uses to the repository,
/// and renames every `entityName` outside the monitor files. Entity names are visible in one
/// place only: the passive-resource measuring point string (`Passive Resource: <assembly>.<name>`,
/// as in the reference), so `renamed_` is removed from the edited run's output before comparing.
#[test]
fn unused_component_and_entity_names_change_nothing() {
    let scratch = common::scratch("unused");
    for (name, dir, spec) in models("unused") {
        let edited = scratch.join(&name);
        copy_model(&dir, &edited, |file, t| {
            let mut t = t.to_string();
            // appended last: positional EMF references (`//@components__Repository.0/...`)
            // keep their targets
            if file.ends_with(".repository")
                && let Some(i) = t.rfind("</repository:Repository>")
            {
                t.insert_str(
                    i,
                    "<components__Repository xsi:type=\"repository:BasicComponent\" \
                     id=\"_k_unused_component\" entityName=\"Unused\"/>\n  ",
                );
            }
            if !file.ends_with(".monitorrepository") && !file.ends_with(".measuringpoint") {
                t = t.replace("entityName=\"", "entityName=\"renamed_");
            }
            t
        });
        let a = run_cfg(&compile(&dir, &spec), spec.sim_config(&name), true);
        let mut b = run_cfg(&compile(&edited, &spec), spec.sim_config(&name), true);
        b.trace = b.trace.replace("renamed_", "");
        b.csv = b.csv.replace("renamed_", "");
        assert!(
            a == b,
            "{name}: unused component / renamed entities change the run"
        );
    }
    let _ = std::fs::remove_dir_all(&scratch);
}

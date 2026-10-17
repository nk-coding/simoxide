//! Time per rule execution on a realistic PCM model: SimuLizar's power-consumption rules and a
//! read-only scan rule on SimOxide's MediaStore model (`simoxide/corpus/x_sl_mediastore`),
//! also with the resource environment grown to more containers, on the adapter (SimOxide's
//! graph, journal on) and on `qvto_emf` (the engine's own store).
//!
//! `cargo run --release -p qvto-simoxide --example adapter-bench [-- <model dir> [iterations]]` (needs `target/pcm`).

use qvto_emf::ResourceSet;
use qvto_emf::config::Config;
use qvto_model::{ClassId, Datum, FeatureId, ModelStore, ResourceId};
use qvto_simoxide::{Session, SimStore};
use qvto_vm::ObjHandle;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const SRM: &str = "http://simulizar.palladiosimulator.org/RuntimeMeasurement/1.0";

fn root() -> PathBuf {
    qvto_paths::root()
}

struct Rule {
    name: &'static str,
    file: PathBuf,
    /// The measurement in the PRM (above or below the rules' threshold of 3.0).
    value: f64,
}

fn rules() -> Vec<Rule> {
    let pc = root().join("qvto-testkit/corpus/simulizar/powerconsumption");
    let scan = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/rules/scan.qvto");
    vec![
        Rule { name: "scaleUp (fires)", file: pc.join("scaleUp.qvto"), value: 5.0 },
        Rule { name: "scaleUp (no-op)", file: pc.join("scaleUp.qvto"), value: 1.0 },
        Rule { name: "scaleDown (fires)", file: pc.join("scaleDown.qvto"), value: 5.0 },
        Rule { name: "scan (read-only)", file: scan, value: 5.0 },
    ]
}

fn class<S: ModelStore>(s: &S, ns: &str, name: &str) -> ClassId {
    let m = s.meta();
    m.class_by_name(m.package_by_ns_uri(ns).unwrap(), name).unwrap()
}

fn feat<S: ModelStore>(s: &S, c: ClassId, name: &str) -> FeatureId {
    s.meta().feature_by_name(c, name).unwrap_or_else(|| panic!("{name}"))
}

const RE: &str = "http://palladiosimulator.org/PalladioComponentModel/ResourceEnvironment/5.2";
const CORE: &str = "http://palladiosimulator.org/PalladioComponentModel/Core/5.2";

/// The PRM: one runtime measurement with `value`, in a new resource.
fn prm<S: ModelStore>(s: &mut S, r: ResourceId, value: f64) {
    let c = class(s, SRM, "RuntimeMeasurement");
    let f = feat(s, c, "measuringValue");
    let m = s.create(c).unwrap();
    s.set(m, f, Datum::Double(value)).unwrap();
    s.add_root(r, m, None).unwrap();
}

/// Grows the resource environment to `n` containers: copies of the first container's
/// processing resources (type, scheduling, rate) in new containers.
fn grow<S: ModelStore>(s: &mut S, env: S::Obj, n: usize) {
    let rc = class(s, RE, "ResourceContainer");
    let ps = class(s, RE, "ProcessingResourceSpecification");
    let rv = class(s, CORE, "PCMRandomVariable");
    let re = s.class_of(env);
    let containers = feat(s, re, "resourceContainer_ResourceEnvironment");
    let specs = feat(s, rc, "activeResourceSpecifications_ResourceContainer");
    let name = feat(s, rc, "entityName");
    let rate = feat(s, ps, "processingRate_ProcessingResourceSpecification");
    let rtype = feat(s, ps, "activeResourceType_ActiveResourceSpecification");
    let sched = feat(s, ps, "schedulingPolicy");
    let repl = feat(s, ps, "numberOfReplicas");
    let spec = feat(s, rv, "specification");
    let Datum::Obj(first) = s.get_at(env, containers, 0) else { panic!("no container") };
    let Datum::Obj(first_spec) = s.get_at(first, specs, 0) else { panic!("no resource") };
    let (t, sp) = (s.get(first_spec, rtype).as_obj(), s.get(first_spec, sched).as_obj());
    let mut k = s.len(env, containers);
    while k < n {
        let c = s.create(rc).unwrap();
        s.set(c, name, Datum::Str(&format!("Server{k}"))).unwrap();
        s.add(env, containers, Datum::Obj(c), None).unwrap();
        let p = s.create(ps).unwrap();
        if let Some(t) = t {
            s.set(p, rtype, Datum::Obj(t)).unwrap();
        }
        if let Some(sp) = sp {
            s.set(p, sched, Datum::Obj(sp)).unwrap();
        }
        s.set(p, repl, Datum::Int(1)).unwrap();
        let v = s.create(rv).unwrap();
        s.set(v, spec, Datum::Str("1000.0")).unwrap();
        s.set(p, rate, Datum::Obj(v)).unwrap();
        s.add(c, specs, Datum::Obj(p), None).unwrap();
        k += 1;
    }
}

/// The model resources by extension (the first file resource with it).
fn by_ext<S: ModelStore>(s: &S, ext: &str) -> ResourceId {
    (0..s.resource_count() as u32)
        .map(ResourceId)
        .find(|r| {
            let u = s.resource_uri(*r);
            u.starts_with("file:") && u.ends_with(ext)
        })
        .unwrap_or_else(|| panic!("no {ext}"))
}

fn compile(file: &Path, meta: qvto_model::MetaRegistry, ops: &[qvto_model::MetaOperation]) -> qvto::Program {
    let resolver = qvto::units::FsResolver::folder();
    let main = resolver.unit_source(file).unwrap();
    let c = qvto::compile_with_operations(main, &resolver, meta, ops);
    if c.has_errors() {
        for u in &c.units {
            for p in &u.problems {
                eprintln!("{}: {}", u.source.file_name, p.message);
            }
        }
        panic!("compile errors");
    }
    c.program.unwrap()
}

/// Runs the rule `iters` times on the evolving model; the time of each run (VM creation,
/// binding, execution).
fn time_runs<S: ModelStore>(
    program: &qvto::Program,
    s: &mut S,
    inputs: &[ResourceId; 5],
    iters: usize,
    mut after: impl FnMut(&mut S),
) -> Vec<Duration>
where
    S::Obj: ObjHandle,
{
    let mut out = Vec::with_capacity(iters);
    for _ in 0..iters {
        let args: Vec<Vec<S::Obj>> = inputs.iter().map(|r| s.roots(*r).to_vec()).collect();
        let t = Instant::now();
        let mut vm = qvto::Vm::new(program, s, qvto::RunOptions::default());
        let o = vm.run_transformation(args);
        out.push(t.elapsed());

        assert_eq!(o.severity, 0, "{}", o.message);
        after(s);
    }
    out
}

/// The processing rates of all containers (both stores must agree).
fn last_rate<S: ModelStore>(s: &S, r: ResourceId) -> String {
    let env = s.roots(r)[0];
    let re = s.class_of(env);
    let containers = feat(s, re, "resourceContainer_ResourceEnvironment");
    let mut all = Vec::new();
    for i in 0..s.len(env, containers) {
    let Datum::Obj(c) = s.get_at(env, containers, i) else { panic!() };
    let rc = s.class_of(c);
    let specs = feat(s, rc, "activeResourceSpecifications_ResourceContainer");
    let Datum::Obj(p) = s.get_at(c, specs, 0) else { panic!() };
    let ps = s.class_of(p);
    let Datum::Obj(v) = s.get(p, feat(s, ps, "processingRate_ProcessingResourceSpecification")) else { panic!() };
    let rv = s.class_of(v);
    all.push(format!("{:?}", s.get(v, feat(s, rv, "specification"))));
    }
    all.join(",")
}

fn stats(mut v: Vec<Duration>) -> (Duration, Duration, Duration) {
    v.sort();
    let at = |q: f64| v[((v.len() - 1) as f64 * q) as usize];
    (at(0.5), at(0.1), at(0.9))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args
        .get(1)
        .filter(|a| !a.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| qvto_paths::workspace_root(&root()).join("corpus/x_sl_mediastore"))
        .canonicalize()
        .expect("model directory");
    let iters: usize = args.get(2).and_then(|x| x.parse().ok()).unwrap_or(300);
    let conf = Config::read(&root().join("target/pcm/pcm.conf"), &root()).expect("target/pcm");
    println!("model {} , {iters} runs per rule, median (p10..p90)", dir.display());
    println!("{:20} {:>11} {:>8} {:>30} {:>30} {:>6}", "rule", "containers", "changes", "adapter (journal on)", "qvto-emf", "ratio");
    for size in [0usize, 100, 1000] {
        for rule in rules() {
            // the adapter: SimOxide's loader and typed model build, its graph
            let mut model = simoxide_model::load_dir(&dir).unwrap();
            let mut session = Session::new();
            session.configure(&conf);
            session.load_metamodel(SRM).unwrap();
            let program = compile(&rule.file, session.meta().clone(), &session.operations());
            let mut store = SimStore::new(&mut model.graph, &mut session);
            let prm_r = store.create_model_resource("file:/bench/prm.xmi");
            prm(&mut store, prm_r, rule.value);
            let inputs = [prm_r, by_ext(&store, ".repository"), by_ext(&store, ".system"), by_ext(&store, ".allocation"), by_ext(&store, ".resourceenvironment")];
            let env = store.roots(inputs[4])[0];
            grow(&mut store, env, size);
            let containers = {
                let re = store.class_of(env);
                store.len(env, feat(&store, re, "resourceContainer_ResourceEnvironment"))
            };
            store.session().start_journal();
            let mut changes = 0;
            let a = time_runs(&program, &mut store, &inputs, iters, |s| {
                changes += s.session().take_journal().changes.len();
            });
            // qvto-emf
            let mut rs: ResourceSet = conf.resource_set_for(&dir.join("x"));
            let ns = conf.ns_index.get(SRM).unwrap().clone();
            rs.load_metamodel(&ns).unwrap();
            let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_file() && p.extension().is_some_and(|x| x != "json")).collect();
            files.sort();
            for f in files {
                rs.load_file(&f).unwrap();
            }
            rs.resolve_all();
            rs.ecore_resource();
            let program = compile(&rule.file, rs.graph.meta().clone(), &qvto_emf::operations::meta_operations(&rs.graph));
            let g = &mut rs.graph;
            let prm_e = g.create_resource("file:/bench/prm.xmi");
            prm(g, prm_e, rule.value);
            let inputs_e = [prm_e, by_ext(g, ".repository"), by_ext(g, ".system"), by_ext(g, ".allocation"), by_ext(g, ".resourceenvironment")];
            let env = g.roots(inputs_e[4])[0];
            grow(g, env, size);
            let e = time_runs(&program, g, &inputs_e, iters, |_| {});
            let check_a = last_rate(&store, inputs[4]);
            let check_e = last_rate(g, inputs_e[4]);
            assert_eq!(check_a, check_e, "the stores disagree");
            let (am, a10, a90) = stats(a);
            let (em, e10, e90) = stats(e);
            println!(
                "{:20} {:>11} {:>8} {:>12.1?} ({:.1?}..{:.1?}) {:>12.1?} ({:.1?}..{:.1?}) {:>6.2}",
                rule.name, containers, changes / iters, am, a10, a90, em, e10, e90,
                am.as_secs_f64() / em.as_secs_f64()
            );
        }
    }
}

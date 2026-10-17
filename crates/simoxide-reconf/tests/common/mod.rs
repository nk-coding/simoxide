//! Shared by the host tests: the SimuLizar/SPD corpus of the QVTo engine
//! (`crates/qvto/qvto-testkit/cases/corpus.cases`, the Java runner's results in
//! `qvto-testkit/fixtures/java-corpus.txt`) as simulation models, the Java run's identity
//! hashes, journals keyed by object paths, dumps in the Java runner's format.

#![allow(dead_code)]

use qvto_model::{ModelStore, OwnedDatum, ResourceId};
use qvto_runtime::IdentityKey;
use qvto_runtime::identity::IdentityHashSource;
use qvto_simoxide::{ChangeKind, EXT_BASE, Journal, SimStore};
use qvto_testkit::case::Case;
use qvto_testkit::fixture::{JavaResult, Tape, parse_tape};
use simoxide_reconf::{Bind, Imports, ModelRef, QvtoCore, QvtoRun, Transformation};
use simoxide_sim::{CompiledModel, SimConfig, Simulation};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// `crates/qvto`.
pub fn qroot() -> PathBuf {
    qvto_paths::root()
}

/// The repository root.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn cases() -> Vec<Case> {
    qvto_testkit::case::read_cases(&qroot().join("qvto-testkit/cases/corpus.cases"))
        .expect("corpus.cases")
}

pub fn case(id: &str) -> Case {
    cases().into_iter().find(|c| c.id == id).expect("case")
}

pub fn java() -> BTreeMap<String, JavaResult> {
    let text = std::fs::read_to_string(qroot().join(qvto_testkit::paths::CORPUS_FIXTURE))
        .expect("java fixture");
    qvto_testkit::fixture::read_bundle(&text).expect("bundle")
}

/// One core for all tests of a binary (the core is shared by runs and threads).
pub fn core() -> &'static QvtoCore {
    static CORE: OnceLock<QvtoCore> = OnceLock::new();
    CORE.get_or_init(|| QvtoCore::new().expect("core"))
}

/// The case's model directory.
pub fn case_dir(c: &Case) -> PathBuf {
    c.path(&qroot(), "").canonicalize().unwrap()
}

/// The case's transformation, compiled by the shared core.
pub fn transformation(c: &Case) -> Transformation {
    let path = c.path(&qroot(), c.qvto.as_deref().unwrap());
    let imports = match &c.import_root {
        Some(r) => Imports::Roots(vec![qroot().join(r)]),
        None => Imports::Folder,
    };
    core().compile_file(&path, &imports).expect("compiles")
}

fn files_with(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == ext))
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// The simulation model of a case's directory: its usage model, allocation, monitor
/// repositories (also in `monitors/`) and SPD files with everything they refer to. The SPD
/// example's monitor repository is left out: its utilisation monitor has no `TimeDriven`
/// processing type, which the SimuLizar flavour rejects (as in `simoxide-spd`'s tests).
pub fn sim_model(c: &Case) -> CompiledModel {
    CompiledModel::compile(sim_loaded(c)).expect("compiles")
}

/// The loaded model of [`sim_model`].
pub fn sim_loaded(c: &Case) -> simoxide_model::Model {
    let dir = case_dir(c);
    let mut files = Vec::new();
    files.extend(files_with(&dir, "usagemodel"));
    files.extend(files_with(&dir, "allocation"));
    if c.kind != "spd" {
        files.extend(files_with(&dir, "monitorrepository"));
        files.extend(files_with(&dir.join("monitors"), "monitorrepository"));
    }
    if c.kind == "spd" {
        // the case's inputs are below `minimalexample/`
        let ex = dir.join("minimalexample");
        files.clear();
        files.extend(files_with(&ex, "usagemodel"));
        files.extend(files_with(&ex, "allocation"));
        files.extend(files_with(&ex, "spd"));
        files.extend(files_with(&ex, "semanticspd"));
    }
    let model = simoxide_model::load_files(&files);
    let errors: Vec<String> = model
        .diagnostics
        .0
        .iter()
        .filter(|d| d.level == simoxide_model::Level::Error)
        .map(|d| d.to_string())
        .collect();
    assert!(errors.is_empty(), "{}: {errors:?}", c.id);
    model
}

pub fn sim_config() -> SimConfig {
    SimConfig {
        run_name: "reconf".into(),
        max_sim_time: 200,
        max_measurements: -1,
        ..Default::default()
    }
}

/// Whether a model document belongs in SimOxide's graph (all its namespaces are SimOxide's).
pub fn is_graph_document(text: &str) -> bool {
    let pre = qvto_emf::xmi_load::prescan(text);
    let mut any = false;
    for ns in &pre.ns_uris {
        if ns == "http://www.omg.org/XMI" || ns == "http://www.w3.org/2001/XMLSchema-instance" {
            continue;
        }
        any = true;
        if simoxide_model::meta::package_by_ns(ns).is_none() {
            return false;
        }
    }
    any
}

/// The case's inputs as models of the run: graph resources by file, the others (the PRM)
/// loaded into the run's QVTo state. Positional, like the corpus's Java driver.
pub fn inputs<C: simoxide_sim::Compat>(
    c: &Case,
    sim: &Simulation<'_, C>,
    run: &mut QvtoRun,
) -> Vec<ModelRef> {
    let mut out = Vec::new();
    for p in &c.inputs {
        let file = c.path(&qroot(), p).canonicalize().unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        if is_graph_document(&text) {
            let g = sim.model_graph();
            let r = (0..g.resources().len())
                .find(|&i| {
                    let u = &g.resources()[i].uri;
                    u.ends_with(&*file.to_string_lossy())
                })
                .unwrap_or_else(|| panic!("{}: {} not in the graph", c.id, file.display()));
            out.push(ModelRef::Graph(simoxide_model::raw::ResId(r as u32)));
        } else {
            out.push(run.load_model(sim, &file).expect("loads"));
        }
    }
    out
}

/// The bindings of the case's inputs to the in/inout parameters, in order.
pub fn binds(t: &Transformation, models: &[ModelRef]) -> Vec<(String, Bind)> {
    let mut it = models.iter();
    let mut v = Vec::new();
    for p in t.params() {
        if p.direction == simoxide_reconf::ParamDir::Out {
            continue;
        }
        let b = match it.next() {
            Some(m) => Bind::Models(vec![m.clone()]),
            None => Bind::Empty,
        };
        v.push((p.name.clone(), b));
    }
    v
}

/// Keys of the store's objects as the tape and the journals name them (`<label>#<path>`).
pub fn object_keys(s: &SimStore<'_>) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    for r in 0..s.resource_count() {
        let r = ResourceId(r as u32);
        let label = qvto_emf::canon::label(s.resource_uri(r));
        for o in s.resource_contents(r) {
            out.insert(o, format!("{label}#{}", qvto_emf::canon::path(s, o)));
        }
    }
    out
}

pub fn load_tape(c: &Case) -> Option<Tape> {
    let p = qroot()
        .join(qvto_testkit::paths::JAVA_CACHE)
        .join("corpus")
        .join(&c.id)
        .join("idtape.tsv");
    parse_tape(&std::fs::read_to_string(p).ok()?).ok()
}

struct Recovered(qvto_testkit::idhash::XorShift);

impl IdentityHashSource for Recovered {
    fn draw(&mut self, _key: IdentityKey) -> i32 {
        self.0.next()
    }
}

/// The Java run's identity hashes (as the conformance engine replays them): objects that exist
/// before the run by key, created objects in the Java thread's draw order.
pub fn id_source(s: &SimStore<'_>, tape: &Tape) -> Box<dyn IdentityHashSource> {
    let keys: HashMap<String, u32> = object_keys(s).into_iter().map(|(o, k)| (k, o)).collect();
    let graph_objs = s.graph().object_count() as u32;
    let existed = move |i: u32| i < graph_objs;
    let mut by_obj: HashMap<u32, i32> = HashMap::new();
    let mut others: Vec<i32> = Vec::new();
    for (k, h, assigned) in &tape.objects {
        match keys.get(k) {
            Some(&o) => {
                by_obj.insert(o, *h);
            }
            None => {
                if *assigned != Some(false) {
                    others.push(*h);
                }
            }
        }
    }
    let idx = qvto_testkit::idhash::sequence_index(tape.seed, &others, 20_000)
        .unwrap_or_else(|| vec![None; others.len()]);
    let mut ordered: Vec<(usize, i32)> = others
        .iter()
        .zip(idx)
        .map(|(h, i)| (i.unwrap_or(usize::MAX), *h))
        .collect();
    ordered.sort();
    let mut queue: std::collections::VecDeque<i32> = ordered.into_iter().map(|(_, h)| h).collect();
    let mut fallback = qvto_testkit::idhash::XorShift::recover(tape.seed.0, tape.seed.1);
    if let Some(x) = fallback.as_mut() {
        x.next();
        x.next();
    }
    let lookup = Box::new(move |k: IdentityKey| match k {
        IdentityKey::Obj(r) if !r.is_local() && existed(r.index()) => {
            by_obj.get(&r.index()).copied()
        }
        IdentityKey::Obj(_) => queue.pop_front(),
        _ => None,
    });
    let fb: Box<dyn IdentityHashSource> = match fallback {
        Some(x) => Box::new(Recovered(x)),
        None => Box::new(qvto_runtime::identity::HotSpotXorShift::default()),
    };
    Box::new(qvto_vm::TapeHook::new(lookup, fb))
}

/// A journal with objects named by their keys before the run (`new#<n>` for created objects,
/// in creation order), comparable between two stores of the same models.
pub fn keyed_journal(keys: &HashMap<u32, String>, j: &Journal) -> Vec<String> {
    let mut created: HashMap<u32, String> = HashMap::new();
    for c in &j.changes {
        if let ChangeKind::Created(_) = c.kind {
            let n = created.len();
            created.insert(c.obj, format!("new#{n}"));
        }
    }
    let name = |o: u32| -> String {
        created
            .get(&o)
            .or_else(|| keys.get(&o))
            .cloned()
            .unwrap_or_else(|| format!("?{}", if o >= EXT_BASE { "ext" } else { "graph" }))
    };
    let val = |d: &OwnedDatum<u32>| -> String {
        match d {
            OwnedDatum::Obj(o) => name(*o),
            d => format!("{d:?}"),
        }
    };
    j.changes
        .iter()
        .map(|c| {
            let k = match &c.kind {
                ChangeKind::Created(cl) => format!("created {}", cl.0),
                ChangeKind::Set { feature, old, new } => {
                    format!("set {} {} -> {}", feature.0, val(old), val(new))
                }
                ChangeKind::Unset { feature, old } => format!("unset {} {}", feature.0, val(old)),
                ChangeKind::Added {
                    feature,
                    index,
                    value,
                } => format!("added {} [{index}] {}", feature.0, val(value)),
                ChangeKind::Removed {
                    feature,
                    index,
                    value,
                } => format!("removed {} [{index}] {}", feature.0, val(value)),
                ChangeKind::Replaced {
                    feature,
                    index,
                    old,
                    new,
                } => format!(
                    "replaced {} [{index}] {} -> {}",
                    feature.0,
                    val(old),
                    val(new)
                ),
                ChangeKind::Moved { feature, from, to } => {
                    format!("moved {} {from} -> {to}", feature.0)
                }
                ChangeKind::Container { old, new } => format!(
                    "container {:?} -> {:?}",
                    old.map(|(o, f)| (name(o), f.0)),
                    new.map(|(o, f)| (name(o), f.0))
                ),
                ChangeKind::RootAdded { index, .. } => format!("root added [{index}]"),
                ChangeKind::RootRemoved { .. } => "root removed".to_string(),
                ChangeKind::Deleted => "deleted".to_string(),
            };
            format!("{} {k}", name(c.obj))
        })
        .collect()
}

/// The dump the Java runner writes (`dump models`): each out/inout extent into a resource
/// `extent<i>.xmi`, then the model resources by label; only the resources whose labels are in
/// `labels` (the Java dump's: the simulation's graph holds more models).
pub fn dump(s: &mut SimStore<'_>, extents: &[Option<Vec<u32>>], labels: &[String]) -> String {
    let before = s.resource_count();
    let mut dumped = Vec::new();
    for (i, e) in extents.iter().enumerate() {
        let Some(roots) = e else { continue };
        let r = s.create_resource(&format!("file:/qvto-rs/out/extent{i}.xmi"));
        for o in roots {
            let _ = s.add_root(r, *o, None);
        }
        dumped.push(r);
    }
    let mut v: Vec<(String, ResourceId)> = (0..before)
        .map(|r| ResourceId(r as u32))
        .filter_map(|r| {
            let uri = s.resource_uri(r);
            let label = qvto_emf::canon::label(uri);
            labels.contains(&label).then_some((label, r))
        })
        .collect();
    v.sort();
    v.dedup_by(|a, b| a.0 == b.0);
    dumped.extend(v.into_iter().map(|(_, r)| r));
    qvto_emf::canon::dump(&*s, &dumped)
}

/// The resource labels of a dump.
pub fn dump_labels(dump: &str) -> Vec<String> {
    dump.lines()
        .filter_map(|l| {
            let v: serde_json::Value = serde_json::from_str(l).ok()?;
            v.get("resource")?.as_str().map(String::from)
        })
        .collect()
}
